//! The fused pass: step every cell, and build the next frame's work list on the way past.
//!
//! # The offset, in three terms
//!
//! Where a live cell lands in the compacted work list is a sum over three tiers, and each tier is a
//! different thing in `simdr`:
//!
//! | term | what it counts | what computes it | cost |
//! | --- | --- | --- | --- |
//! | `serial` | earlier cells held by *this lane* | a Rust loop over registers | nothing |
//! | `before` | earlier lanes of *this subgroup* | `Lanes::prefix_sum_exclusive` | one instruction |
//! | `base` | earlier blocks | a later pass over [`COUNTS`] | one load |
//!
//! This file is the first two. The third needs every block's total, so it cannot exist until every
//! block has run — which is the whole reason a compaction is more than one dispatch.
//!
//! # Where the fusion is, which is not where it sounds like it is
//!
//! Not in merging kernels. In the **innermost loop**: the step and the running count share a
//! register, so the first tier of the prefix scan costs *no memory traffic at all*. The state is
//! read once, stepped, and written once, and the number that decides where it goes was added along
//! the way.
//!
//! Without it a compaction reads the state to step it, reads it again to test it, and reads the
//! flags again to scan them — three passes over sixteen million cells to answer a question that
//! fits in a register.
//!
//! # Why a workgroup is exactly one subgroup
//!
//! A fourth tier would sit between `before` and `base`: the subgroups of one workgroup, combined
//! through shared memory. It is not here, and that is a choice rather than an omission.
//!
//! Combining across subgroups means knowing **which** subgroup an invocation belongs to, and the
//! defined answer is SPIR-V's `SubgroupId`. `simdr`'s `src/spec/memory.rs` does not declare it —
//! `NumSubgroups` is there and `SubgroupId` is not — so the only spelling available is
//! `local_index / width`, which is the assumption `Lanes::lane_index`'s own documentation calls the
//! cheaper wrong one, one tier up.
//!
//! Making the workgroup the subgroup's own width removes the tier instead of guessing at it. The
//! *serial* tier absorbs the difference: a block is [`PER_LANE`] cells a lane whatever the device
//! is, so a 64-lane part has blocks twice as wide and half as many, and nothing here has to know.

use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, U32};

pub use crate::layout::{BINDINGS, BUCKETS, COUNTS, PARAMS, STATE};

/// How many cells one lane carries, serially, in registers.
///
/// **The tier that costs nothing.** Each is a load, a step, a store, and one add to a running count
/// — and that add is the prefix scan's first level. More of them makes the free tier bigger and the
/// block-sum array smaller; fewer makes the module smaller.
pub const PER_LANE: u32 = 8;

/// A cell joins next frame's work list when the top byte of its stepped state is below this.
///
/// About five per cent, which is the density this whole design is an argument about: the other
/// ninety-five is what compaction exists to stop paying for.
pub const AWAKE: u32 = 13;

/// The mixer's multipliers. Multiply, shift and add — `src/module/op.rs` declares no exclusive-or.
const GOLDEN: u32 = 0x9E37_79B1;
const STIR: u32 = 0x85EB_CA6B;

/// Cells in one block: one subgroup's worth of lanes, [`PER_LANE`] each.
#[must_use]
pub const fn cells(subgroup: u32) -> u32 {
    subgroup * PER_LANE
}

/// Words one block occupies in [`BUCKETS`]: its bucket, then its dump.
///
/// **The dump is interleaved rather than parked at the end**, so its address is a build-time stride
/// and the module never has to be told how many blocks the dispatch has. Block `b` owns
/// `b × stride`, its bucket is the first [`cells`] words of that, and the last `subgroup` words are
/// one dump slot per lane.
#[must_use]
pub const fn stride(subgroup: u32) -> u32 {
    cells(subgroup) + subgroup
}

/// Step every cell this block owns, and write the live ones into the block's own bucket.
///
/// # The dead cells still write, and that is deliberate
///
/// `decisions/DR-0003` refuses a per-lane branch, so a store cannot be conditional. Every cell
/// stores: a live one into its slot in the bucket, a dead one into **this lane's own dump slot**,
/// which nothing ever reads.
///
/// That is not the waste it looks like. All of a lane's dead cells write to the *same* address, so
/// they are one cache line touched once rather than eight words moved — a word per lane, not a word
/// per dead cell. Letting dead cells write into the bucket and be overwritten by a later live one
/// would be cheaper still and is wrong: the cells after a lane's last live one have no successor to
/// overwrite them, and would land on the next lane's first slot.
///
/// # Errors
///
/// [`LaneError::NoMapping`] for a width with no arm, otherwise as the module builder.
pub fn sweep(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    match subgroup {
        4 => built::<4>(subgroup),
        8 => built::<8>(subgroup),
        16 => built::<16>(subgroup),
        32 => built::<32>(subgroup),
        64 => built::<64>(subgroup),
        other => Err(LaneError::NoMapping {
            lanes: other,
            width: subgroup,
        }),
    }
}

/// The kernel, at a width the caller has already matched to the device.
fn built<const LANES: u32>(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    // A workgroup is a subgroup, which is what removes the fourth tier.
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.index_type();

    let block = kernel.workgroup_index();
    let lane = kernel.local_index();
    let head = kernel.module().constant_u32(0)?;
    let frame = kernel.load_at(PARAMS, head)?;

    // Where this block's cells begin in the state, and where its region begins in the buckets.
    let span = kernel.module().constant_u32(cells(subgroup))?;
    let cell_start = kernel.module().i_mul(uint, block, span)?;
    let run = kernel.module().constant_u32(PER_LANE)?;
    let cell_lane = kernel.module().i_mul(uint, lane, run)?;
    let cell_mine = kernel.module().i_add(uint, cell_start, cell_lane)?;

    let wide = kernel.module().constant_u32(stride(subgroup))?;
    let region = kernel.module().i_mul(uint, block, wide)?;
    let past = kernel.module().constant_u32(cells(subgroup))?;
    let dump_start = kernel.module().i_add(uint, region, past)?;
    let dump_mine = kernel.module().i_add(uint, dump_start, lane)?;

    // Every address this invocation touches, and the state at each.
    //
    // **Blocked and not striped**, which is a correctness matter and not a preference. A lane holds
    // cells `lane × PER_LANE` through `+ PER_LANE`, so the order (lane, then step) *is* index
    // order — and that is what makes the two scan tiers add up to a prefix. Striping the cells
    // across lanes reads more neatly and counts them in lane-major order, which is a permutation of
    // the prefix rather than the prefix: `tests/compact.rs` caught it by finding block 32's cells
    // at slot 12.
    //
    // It costs coalescing. One load instruction now reaches `PER_LANE` words apart across the
    // subgroup rather than a contiguous run, and that is the first thing to revisit if this pass
    // turns out to be bandwidth-bound.
    let mut addresses = Vec::with_capacity(PER_LANE as usize);
    let mut loaded = Vec::with_capacity(PER_LANE as usize);
    for step in 0..PER_LANE {
        let along = kernel.module().constant_u32(step)?;
        let at = kernel.module().i_add(uint, cell_mine, along)?;
        loaded.push(kernel.load_at(STATE, at)?);
        addresses.push(at);
    }

    // The whole of the arithmetic in one place: the step, the test, and the serial tier of the scan.
    let mut stepped = Vec::with_capacity(PER_LANE as usize);
    let mut awake = Vec::with_capacity(PER_LANE as usize);
    let mut serial = Vec::with_capacity(PER_LANE as usize);
    let mine = {
        let mut lanes = kernel.lanes()?;
        let now = lanes.splat_id::<U32, LANES>(frame)?;

        let golden = lanes.splat_bits::<U32, LANES>(GOLDEN)?;
        let stir = lanes.splat_bits::<U32, LANES>(STIR)?;
        let by16 = lanes.splat_bits::<U32, LANES>(16)?;
        let by24 = lanes.splat_bits::<U32, LANES>(24)?;
        let sleeping = lanes.splat_bits::<U32, LANES>(AWAKE)?;
        let one = lanes.splat_bits::<U32, LANES>(1)?;
        let none = lanes.splat_bits::<U32, LANES>(0)?;

        let mut running = none;
        for &value in &loaded {
            let held = lanes.splat_id::<U32, LANES>(value)?;

            // The step. Whatever a simulation turns out to be, it has this shape: the cell's own
            // state and the frame, mixed, and written back where it came from.
            let drifting = lanes.mul(now, golden)?;
            let seeded = lanes.add(held, drifting)?;
            let h = lanes.mul(seeded, stir)?;
            let folded = lanes.shift_right_logical(h, by16)?;
            let next = lanes.add(h, folded)?;

            // Awake or not: a comparison and a select, never a branch.
            let top = lanes.shift_right_logical(next, by24)?;
            let above = lanes.greater_than(top, sleeping)?;
            let live = lanes.select(above, none, one)?;

            // **The first tier, and it costs nothing.** `running` is already in a register because
            // the step above put it there.
            serial.push(running);
            running = lanes.add(running, live)?;

            stepped.push(next);
            awake.push(live);
        }
        running
    };

    // The stepped state goes back where it came from.
    for (at, value) in addresses.iter().zip(&stepped) {
        kernel.store_at(STATE, *at, value.id())?;
    }

    // The second tier: how many live cells every *earlier lane* of this subgroup holds, and how
    // many the block holds altogether. One subgroup instruction each, over the same vector.
    let (before, total) = {
        let mut lanes = kernel.lanes()?;
        let before = lanes.prefix_sum_exclusive::<U32, LANES>(mine)?;
        let total = lanes.reduce_sum::<U32, LANES>(mine)?;
        (before, total)
    };
    kernel.store_at(COUNTS, block, total)?;

    // And the live cells into the bucket, the dead into the dump.
    for (index, at) in addresses.iter().enumerate() {
        let Some(live) = awake.get(index).copied() else {
            continue;
        };
        let Some(earlier) = serial.get(index).copied() else {
            continue;
        };

        let slot = {
            let mut lanes = kernel.lanes()?;
            let here = lanes.splat_id::<U32, LANES>(region)?;
            let elsewhere = lanes.splat_id::<U32, LANES>(dump_mine)?;
            let none = lanes.splat_bits::<U32, LANES>(0)?;

            let within = lanes.add(before, earlier)?;
            let bucket = lanes.add(here, within)?;
            let alive = lanes.greater_than(live, none)?;
            lanes.select(alive, bucket, elsewhere)?
        };
        kernel.store_at(BUCKETS, slot.id(), *at)?;
    }

    kernel.finish()
}

/// One `u32` stepped, on the host.
///
/// **Written from the arithmetic rather than from the kernel**, and in the same order — the order is
/// what is being checked.
#[must_use]
pub fn step(held: u32, frame: u32) -> u32 {
    let drifting = frame.wrapping_mul(GOLDEN);
    let seeded = held.wrapping_add(drifting);
    let h = seeded.wrapping_mul(STIR);
    h.wrapping_add(h >> 16)
}

/// Whether a stepped cell joins the work list.
#[must_use]
pub fn is_awake(stepped: u32) -> bool {
    (stepped >> 24) <= AWAKE
}

/// What the device should leave behind: the stepped state, the buckets, and the counts.
///
/// **The layout is the reference's business too.** Which slot a live cell lands in is the sum of
/// three tiers, and getting that wrong on the host would be a reference that agreed with a kernel
/// about the wrong arrangement — so it is spelled out here rather than derived from the kernel.
///
/// The dump region of each block is left as it was: nothing reads it, so nothing may check it.
#[must_use]
pub fn sweep_reference(
    subgroup: u32,
    blocks: u32,
    frame: u32,
    state: &[u32],
    buckets: &[u32],
) -> (Vec<u32>, Vec<u32>, Vec<u32>) {
    let per_block = cells(subgroup) as usize;
    let wide = stride(subgroup) as usize;

    let mut stepped = state.to_vec();
    let mut written = buckets.to_vec();
    let mut counts = vec![0_u32; blocks as usize];

    for block in 0..blocks as usize {
        // The serial tier, per lane, and what each lane contributes.
        let mut serial = vec![vec![0_u32; PER_LANE as usize]; subgroup as usize];
        let mut mine = vec![0_u32; subgroup as usize];

        for lane in 0..subgroup as usize {
            let mut running = 0_u32;
            for step_index in 0..PER_LANE as usize {
                let at = block * per_block + lane * PER_LANE as usize + step_index;
                let Some(slot) = stepped.get_mut(at) else {
                    continue;
                };
                *slot = step(*slot, frame);
                let live = u32::from(is_awake(*slot));

                if let Some(entry) = serial.get_mut(lane).and_then(|l| l.get_mut(step_index)) {
                    *entry = running;
                }
                running += live;
            }
            if let Some(slot) = mine.get_mut(lane) {
                *slot = running;
            }
        }

        // The second tier: an exclusive prefix sum over the lanes, and the block's total.
        let mut before = vec![0_u32; subgroup as usize];
        let mut running = 0_u32;
        for lane in 0..subgroup as usize {
            if let Some(slot) = before.get_mut(lane) {
                *slot = running;
            }
            running += mine.get(lane).copied().unwrap_or(0);
        }
        if let Some(slot) = counts.get_mut(block) {
            *slot = running;
        }

        // And the live cells into the bucket. The dead ones go to a dump nothing reads.
        for lane in 0..subgroup as usize {
            for step_index in 0..PER_LANE as usize {
                let at = block * per_block + lane * PER_LANE as usize + step_index;
                let Some(&value) = stepped.get(at) else {
                    continue;
                };
                if !is_awake(value) {
                    continue;
                }
                let within = before.get(lane).copied().unwrap_or(0) as usize
                    + serial
                        .get(lane)
                        .and_then(|l| l.get(step_index))
                        .copied()
                        .unwrap_or(0) as usize;
                if let Some(slot) = written.get_mut(block * wide + within) {
                    *slot = u32::try_from(at).unwrap_or(u32::MAX);
                }
            }
        }
    }

    (stepped, written, counts)
}
