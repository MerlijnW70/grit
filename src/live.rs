//! The sand drives the compaction, and the compaction drives the sand.
//!
//! Everything before this was two systems: a settle over the whole grid, and a compaction of a
//! predicate that had nothing to do with it. This is the loop that makes them one.
//!
//! ```text
//!   decay    every quad     every wake counter, one closer to nothing
//!   brush    a small box    what the pointer holds, and the quads it touched, woken
//!   settle   the work list  one Margolus step over what moved last frame
//!   mark     every quad     the flags, scanned into buckets and counts
//!   scan ×2                 the blocks against each other
//!   gather   every block    the buckets, into one contiguous work list
//!   prepare  1              how many workgroups the *next* settle runs
//!   paint    every pixel    the cells the window is over, as colours
//! ```
//!
//! # Waking, and why nine writes race with nothing
//!
//! A quad that changed must be looked at again, and so must its neighbours: material crosses a
//! block boundary when the partition's origin shifts, so what moves in one quad this frame is in a
//! different quad next frame. Each moving quad therefore writes a **two** for itself and its eight
//! neighbours, and [`decay`] takes one off every frame — a two-frame window, for the reason that
//! function gives.
//!
//! Nine invocations may write the same flag in the same submission and that is not a race, because
//! **they all write the number one**. `Kernel::store_at` documents exactly this: identical writes to
//! one address need no ordering. A flag that counted rather than marked would need an atomic; a flag
//! that marks needs nothing.
//!
//! # Why one index space serves both phases
//!
//! The partition alternates between origin `(0,0)` and `(1,1)`, so a quad's four cells are not the
//! same four on consecutive frames. A flag indexed by quad would therefore name the wrong cells half
//! the time — except that the dilation already covers it: a phase-one quad `(x, y)` overlaps
//! phase-zero quads `(x, y)` through `(x+1, y+1)`, and a phase-zero quad overlaps phase-one quads
//! `(x-1, y-1)` through `(x, y)`. Both are inside the eight neighbours, so one index space and a
//! dilation of one is enough for either.
//!
//! # What proves it
//!
//! `tests/live.rs` runs the same pour twice — once over the work list and once over the whole grid —
//! and demands the two grids be **identical**, cell for cell, every frame. A quad that should have
//! woken and did not is a grain that stops, and the full-grid run is a different algorithm rather
//! than a second copy of the same mistake.

use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, Lanes, U32, Vector};

use crate::layout::{AWAKE, BINDINGS, DENSE, GRAND, HOT, PARAMS, STATE};
use crate::sand::{self, Grid};

/// Run `body` only in the workgroups whose block has something awake in it.
///
/// [`crate::gate`] carries the argument; this is the one line of it that names [`HOT`].
fn if_hot<const LANES: u32, F>(
    kernel: &mut Kernel<U32>,
    block: simdr::module::Id,
    body: F,
) -> Result<(), LaneError>
where
    F: FnOnce(&mut Kernel<U32>) -> Result<(), LaneError>,
{
    let held = kernel.load_at(HOT, block)?;
    crate::gate::if_any::<LANES, _>(kernel, held, body)
}

/// Every wake counter, one closer to nothing.
///
/// # Two, and why one is not enough
///
/// A quad that moves is set to two, and this takes one off every frame. That is a **two-frame**
/// window rather than one, and the second frame is the one that makes it sound.
///
/// The partition's origin alternates, so a quad has two different neighbourhoods on consecutive
/// frames. A grain that has just landed on the bottom row of its quad has nowhere to go *this*
/// phase — the cells below it are not in this quad — and somewhere to go the next. Waking only what
/// moved would drop it from the list in between, and it would hang there forever with nothing
/// nearby moving to wake it.
///
/// Two frames covers both phases with the same contents. If it is still static after that it cannot
/// move at all until something near it does, and that something will set it to two again.
///
/// The step down is `max(v, 1) − 1`, because there is no signed compare on the lane API and a
/// counter that wrapped past nothing would be awake for four billion frames.
///
/// One invocation a quad, its own slot, so nothing races. The dump past the counters is left alone —
/// nothing reads it, so nothing has to clear it.
///
/// # Errors
///
/// As [`sand::settle`].
pub fn decay(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    match subgroup {
        4 => decayed::<4>(subgroup),
        8 => decayed::<8>(subgroup),
        16 => decayed::<16>(subgroup),
        32 => decayed::<32>(subgroup),
        64 => decayed::<64>(subgroup),
        other => Err(LaneError::NoMapping {
            lanes: other,
            width: subgroup,
        }),
    }
}

/// The decay, at a width the caller has matched to the device.
fn decayed<const LANES: u32>(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    use crate::sweep::{self, PER_LANE};

    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.index_type();

    // **A workgroup a block, not a lane a quad**, so the coarse flag has something to gate. The
    // block is the same one `mark` sweeps, which is what keeps the two in step.
    let block = kernel.workgroup_index();
    let lane = kernel.local_index();
    let span = kernel.module().constant_u32(sweep::cells(subgroup))?;
    let start = kernel.module().i_mul(uint, block, span)?;
    let run = kernel.module().constant_u32(PER_LANE)?;
    let along = kernel.module().i_mul(uint, lane, run)?;
    let mine = kernel.module().i_add(uint, start, along)?;

    let step = |kernel: &mut Kernel<U32>, held: simdr::module::Id| {
        let mut lanes = kernel.lanes()?;
        let now = lanes.splat_id::<U32, 1>(held)?;
        let one = lanes.splat_bits::<U32, 1>(1)?;
        let minus = lanes.splat_bits::<U32, 1>(u32::MAX)?;
        let floored = lanes.max(now, one)?;
        let back = lanes.mul(one, minus)?;
        Ok::<_, LaneError>(lanes.add(floored, back)?.id())
    };

    if_hot::<LANES, _>(&mut kernel, block, |kernel| {
        for offset in 0..PER_LANE {
            let along = kernel.module().constant_u32(offset)?;
            let quad = kernel.module().i_add(uint, mine, along)?;
            let held = kernel.load_at(AWAKE, quad)?;
            let lower = step(kernel, held)?;
            kernel.store_at(AWAKE, quad, lower)?;
        }

        // And the block's own counter, written by every lane because every lane has the same
        // answer — the argument `layout::HOT` makes about identical writes, one level up.
        let held = kernel.load_at(HOT, block)?;
        let lower = step(kernel, held)?;
        kernel.store_at(HOT, block, lower)
    })?;
    kernel.finish()
}

/// The tile totals, back to nothing.
///
/// **The in-place scan leaves its answer where the next frame's input goes.** `scan_totals` reads
/// `TOTALS` and writes its exclusive prefix back over it, and the frame after that `scan_counts`
/// writes only the `tiles` entries it has — so the rest still hold last frame's prefix, and the
/// reduction adds them all again. It compounds: the grand total reached three billion by frame
/// sixty-one, the workgroup count with it, and the settle ran over a list that was not there.
///
/// One workgroup, because there are never more than `per_block` tiles — `Plan::over` refuses a
/// third level rather than pretend otherwise.
///
/// # Errors
///
/// As [`settle`].
pub fn wipe(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    use crate::layout::{TALLY, TALLY_WORDS, TOTALS};
    use crate::sweep::PER_LANE;

    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.index_type();
    let lane = kernel.local_index();
    let run = kernel.module().constant_u32(PER_LANE)?;
    let mine = kernel.module().i_mul(uint, lane, run)?;
    let nothing = kernel.module().constant_u32(0)?;

    for step in 0..PER_LANE {
        let along = kernel.module().constant_u32(step)?;
        let at = kernel.module().i_add(uint, mine, along)?;
        kernel.store_at(TOTALS, at, nothing)?;
    }

    // **The tally is accumulated with atomics, so it has to start at nothing every frame** — for
    // exactly the reason the tile totals do, and found the same way: a sum that is never cleared is
    // a sum of the whole run, and it looks perfectly plausible until you notice it only goes up.
    // Written by every lane rather than one, because they all write the same nought to the same
    // slot and a uniform store is cheaper than deciding which lane may do it.
    for word in 0..TALLY_WORDS {
        let at = kernel
            .module()
            .constant_u32(u32::try_from(word).unwrap_or(0))?;
        kernel.store_at(TALLY, at, nothing)?;
    }
    kernel.finish()
}

/// One Margolus step over the quads the work list names, waking what moved.
///
/// The same rule as [`sand::settle`] — the four ordered swaps — over a different set of quads and
/// with the wake writes on the end. Kept apart from that one rather than folded into it, because
/// `tests/live.rs` runs both and compares, and a shared body would agree with itself.
///
/// # Errors
///
/// As [`sand::settle`].
pub fn settle(subgroup: u32, grid: Grid) -> Result<Vec<u32>, LaneError> {
    match subgroup {
        4 => listed::<4>(subgroup, grid),
        8 => listed::<8>(subgroup, grid),
        16 => listed::<16>(subgroup, grid),
        32 => listed::<32>(subgroup, grid),
        64 => listed::<64>(subgroup, grid),
        other => Err(LaneError::NoMapping {
            lanes: other,
            width: subgroup,
        }),
    }
}

/// The listed settle, at a width the caller has matched to the device.
fn listed<const LANES: u32>(subgroup: u32, grid: Grid) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.index_type();

    let group = kernel.workgroup_index();
    let lane = kernel.local_index();
    let width = kernel.module().constant_u32(subgroup)?;
    let base = kernel.module().i_mul(uint, group, width)?;
    let slot = kernel.module().i_add(uint, base, lane)?;

    let head = kernel.module().constant_u32(0)?;
    let frame = kernel.load_at(PARAMS, head)?;
    let grand = kernel.load_at(GRAND, head)?;
    let second = kernel.module().constant_u32(1)?;
    let dump_base = kernel.load_at(PARAMS, second)?;

    // **The last workgroup reads past the end of the list**, because a workgroup count is a ceiling,
    // and what it reads there is a stale entry the gather did not overwrite. That quad is *not*
    // harmless to step: it may be one another invocation is legitimately stepping this frame, and
    // then the same four cells are written twice with different answers. Both its state writes and
    // its wake writes are therefore sent to a dump instead.
    let inside = {
        let mut lanes = kernel.lanes()?;
        let here = lanes.splat_id::<U32, LANES>(slot)?;
        let total = lanes.splat_id::<U32, LANES>(grand)?;
        let one = lanes.splat_bits::<U32, LANES>(1)?;
        let none = lanes.splat_bits::<U32, LANES>(0)?;
        let listed = lanes.greater_than(total, here)?;
        lanes.select(listed, one, none)?
    };

    // Which quad, and where its four cells are. A slot past the end names whatever is there, and
    // `low_bits` keeps it inside the grid however wild it is.
    let quad = kernel.load_at(DENSE, slot)?;
    let (places, column, row) = corners::<LANES>(&mut kernel, grid, quad, frame)?;

    let held = [
        kernel.load_at(STATE, places[0].id())?,
        kernel.load_at(STATE, places[1].id())?,
        kernel.load_at(STATE, places[2].id())?,
        kernel.load_at(STATE, places[3].id())?,
    ];

    let (settled, counted) = {
        let mut lanes = kernel.lanes()?;
        let a = lanes.splat_id::<U32, LANES>(held[0])?;
        let b = lanes.splat_id::<U32, LANES>(held[1])?;
        let c = lanes.splat_id::<U32, LANES>(held[2])?;
        let d = lanes.splat_id::<U32, LANES>(held[3])?;

        // **The same seven rules as `sand::settle`, in the same order**, because the whole claim of
        // `tests/live.rs` is that the two runs differ only in *which* quads are stepped. A rule that
        // lived in one of them and not the other would make the comparison meaningless.
        let a = sand::hold::<LANES>(&mut lanes, a, c)?;
        let b = sand::hold::<LANES>(&mut lanes, b, d)?;
        let [sa, sb, sc, sd] = sand::start::<LANES>(&mut lanes, [a, b, c, d])?;
        let (sa, sb) = sand::course::<LANES>(&mut lanes, sa, sb)?;
        let (sc, sd) = sand::course::<LANES>(&mut lanes, sc, sd)?;

        let (na, nc) = sand::fall::<LANES>(&mut lanes, sa, sc)?;
        let (nb, nd) = sand::fall::<LANES>(&mut lanes, sb, sd)?;
        let (na, nd) = sand::fall::<LANES>(&mut lanes, na, nd)?;
        let (nb, nc) = sand::fall::<LANES>(&mut lanes, nb, nc)?;

        // **Does it hold anything that can move**, not *did anything move*. Those are different
        // questions and the second one is wrong: the partition's origin shifts every frame, so a
        // quad with nowhere to go this frame can have somewhere next frame with the same contents.
        // Waking on movement lets a falling stream outrun its own wake front, which is what
        // `tests/live.rs` found at frame 75 — a grain present in the whole-grid run and missing
        // from the listed one.
        //
        // Content is sufficient and provable: a move needs sand, a move happens inside one quad,
        // and next phase that sand lies inside a quad within one of this one. Four comparisons
        // summed, because a `Predicate` is not a value this API combines.
        let mut count = lanes.splat_bits::<U32, LANES>(0)?;
        for (before, after) in [(a, na), (b, nb), (c, nc), (d, nd)] {
            let one = lanes.splat_bits::<U32, LANES>(1)?;
            let none = lanes.splat_bits::<U32, LANES>(0)?;
            let same = lanes.equal(before, after)?;
            let differs = lanes.select(same, none, one)?;
            count = lanes.add(count, differs)?;
        }

        // What this quad holds, for the tally. Counted after the step rather than before, so the
        // numbers describe the world the viewer is about to be shown.
        let mut wet = lanes.splat_bits::<U32, LANES>(0)?;
        let mut dry = lanes.splat_bits::<U32, LANES>(0)?;
        for cell in [na, nb, nc, nd] {
            let one = lanes.splat_bits::<U32, LANES>(1)?;
            let none = lanes.splat_bits::<U32, LANES>(0)?;
            let water = lanes.splat_bits::<U32, LANES>(sand::WATER)?;
            let grain = lanes.splat_bits::<U32, LANES>(sand::SAND)?;
            let kind = sand::material::<LANES>(&mut lanes, cell)?;
            let is_wet = lanes.equal(kind, water)?;
            let is_dry = lanes.equal(kind, grain)?;
            let add_wet = lanes.select(is_wet, one, none)?;
            let add_dry = lanes.select(is_dry, one, none)?;
            wet = lanes.add(wet, add_wet)?;
            dry = lanes.add(dry, add_dry)?;
        }
        ([na, nb, nc, nd], [count, wet, dry])
    };

    // Where a slot past the end of the list puts its four cells: past the grid, four to an
    // invocation, and never read.
    let spare = {
        let mut lanes = kernel.lanes()?;
        let past =
            lanes.splat_bits::<U32, LANES>(u32::try_from(grid.cells()).unwrap_or(u32::MAX))?;
        let mine = lanes.splat_id::<U32, LANES>(slot)?;
        let four = lanes.splat_bits::<U32, LANES>(2)?;
        let along = lanes.shift_left(mine, four)?;
        lanes.add(past, along)?
    };

    for (corner, (place, value)) in places.iter().zip(&settled).enumerate() {
        let where_at = {
            let mut lanes = kernel.lanes()?;
            let none = lanes.splat_bits::<U32, LANES>(0)?;
            let step = lanes.splat_bits::<U32, LANES>(u32::try_from(corner).unwrap_or(0))?;
            let elsewhere = lanes.add(spare, step)?;
            let listed = lanes.greater_than(inside, none)?;
            lanes.select(listed, *place, elsewhere)?
        };
        kernel.store_at(STATE, where_at.id(), value.id())?;
    }

    let where_at = Waking {
        subgroup,
        grid,
        column,
        row,
    };
    let holding = *counted.first().ok_or(LaneError::NoMapping {
        lanes: LANES,
        width: subgroup,
    })?;
    wake::<LANES>(&mut kernel, where_at, holding, inside, dump_base)?;

    let what = Tallying {
        counted,
        column,
        row,
        inside,
    };
    tally::<LANES>(&mut kernel, what)?;
    kernel.finish()
}

/// What [`tally`] needs, gathered so the call is not eight arguments long.
struct Tallying<const LANES: u32> {
    /// Cells changed, water cells and sand cells, per quad.
    counted: [Vector<U32, LANES>; 3],
    /// The quad's own column and row.
    column: Vector<U32, LANES>,
    /// The quad's row.
    row: Vector<U32, LANES>,
    /// One where this slot is a real list entry, nought where it is past the end.
    inside: Vector<U32, LANES>,
}

/// Add this workgroup's share of the frame's numbers into [`TALLY`].
///
/// # The one reduction in a crate about scans
///
/// Every other pass here compacts: it wants to know *which* quads, in what order, and a prefix sum
/// is the only thing that answers that. This wants the opposite — not which, but how many, and one
/// answer for the whole device. That is [`simdr::lanes::Lanes::reduce_sum`], which folds the strips
/// inside a lane and then the lanes across the subgroup, and hands the same total back to every one
/// of them.
///
/// # And why the total is then multiplied by nought in all but one lane
///
/// Because every lane holds it, and every lane is about to execute the atomic. An add is not
/// idempotent, so `width` lanes adding the same total would count the workgroup `width` times.
/// `decisions/DR-0003` refuses the branch that would let only one lane run the instruction, so the
/// instruction runs everywhere and the *value* is what carries the decision: the total in the first
/// lane, nought in the rest. Adding nought is free and lands nowhere.
///
/// The cost is one subgroup reduction and one atomic a workgroup for each of six numbers, on data
/// that was already in registers because the settle had just computed it. **The telemetry is not a
/// pass.**
fn tally<const LANES: u32>(
    kernel: &mut Kernel<U32>,
    what: Tallying<LANES>,
) -> Result<(), LaneError> {
    use crate::layout::{COARSE, TALLY};

    let Tallying {
        counted,
        column,
        row,
        inside,
    } = what;
    let [moved, wet, dry] = counted;
    let first = kernel.local_index();

    let shares = {
        let mut lanes = kernel.lanes()?;
        let none = lanes.splat_bits::<U32, LANES>(0)?;
        let one = lanes.splat_bits::<U32, LANES>(1)?;
        let listed = lanes.greater_than(inside, none)?;
        let keep =
            |lanes: &mut Lanes<'_>, value: Vector<U32, LANES>| lanes.select(listed, value, none);

        // A slot past the end of the list stepped a spare quad, and its numbers are not the world's.
        let moved = keep(&mut lanes, moved)?;
        let wet = keep(&mut lanes, wet)?;
        let dry = keep(&mut lanes, dry)?;

        // Coarse, because the sum has to fit in a word — see `layout::COARSE`.
        let by = lanes.splat_bits::<U32, LANES>(COARSE)?;
        let across = lanes.shift_right_logical(column, by)?;
        let down = lanes.shift_right_logical(row, by)?;
        let across = lanes.mul(across, moved)?;
        let down = lanes.mul(down, moved)?;

        // And whether the quad moved at all, which is what the two sums are averaged over.
        let stirred = lanes.greater_than(moved, none)?;
        let stirred = lanes.select(stirred, one, none)?;

        [moved, wet, dry, across, down, stirred]
    };

    for (word, share) in shares.into_iter().enumerate() {
        let total = {
            let mut lanes = kernel.lanes()?;
            lanes.reduce_sum::<U32, LANES>(share)?
        };
        let mine = {
            let mut lanes = kernel.lanes()?;
            let none = lanes.splat_bits::<U32, LANES>(0)?;
            let seat = lanes.splat_id::<U32, LANES>(first)?;
            let sum = lanes.splat_id::<U32, LANES>(total)?;
            let leading = lanes.equal(seat, none)?;
            lanes.select(leading, sum, none)?
        };
        let at = kernel
            .module()
            .constant_u32(u32::try_from(word).unwrap_or(0))?;
        // The slot's old value is what makes this instruction an allocator elsewhere. Here it
        // is a counter and nobody wants the answer.
        let _ = kernel.atomic_add_at(TALLY, at, mine.id())?;
    }
    Ok(())
}

/// Where a quad's four cells are, and which quad it was.
type Corners<const LANES: u32> = (
    [Vector<U32, LANES>; 4],
    Vector<U32, LANES>,
    Vector<U32, LANES>,
);

/// The four cell addresses of a quad, at this frame's partition origin.
///
/// Also hands back the quad's own column and row, which the wake writes need.
fn corners<const LANES: u32>(
    kernel: &mut Kernel<U32>,
    grid: Grid,
    quad: simdr::module::Id,
    frame: simdr::module::Id,
) -> Result<Corners<LANES>, LaneError> {
    let mut lanes = kernel.lanes()?;
    let which = lanes.splat_id::<U32, LANES>(quad)?;
    let now = lanes.splat_id::<U32, LANES>(frame)?;

    let phase = sand::low_bits::<LANES>(&mut lanes, now, 1)?;
    let by = lanes.splat_bits::<U32, LANES>(grid.across().trailing_zeros())?;
    let down = lanes.shift_right_logical(which, by)?;
    let along = sand::low_bits::<LANES>(&mut lanes, which, grid.across().trailing_zeros())?;

    let one = lanes.splat_bits::<U32, LANES>(1)?;
    let doubled_x = lanes.shift_left(along, one)?;
    let doubled_y = lanes.shift_left(down, one)?;
    let column = lanes.add(doubled_x, phase)?;
    let row = lanes.add(doubled_y, phase)?;
    let right = lanes.add(column, one)?;
    let under = lanes.add(row, one)?;

    let places = [
        sand::at::<LANES>(&mut lanes, grid, column, row)?,
        sand::at::<LANES>(&mut lanes, grid, right, row)?,
        sand::at::<LANES>(&mut lanes, grid, column, under)?,
        sand::at::<LANES>(&mut lanes, grid, right, under)?,
    ];
    Ok((places, along, down))
}

/// Which quad is doing the waking, and on what device.
pub(crate) struct Waking<const LANES: u32> {
    /// The subgroup's width, which is also the workgroup's.
    pub subgroup: u32,
    /// How big the world is.
    pub grid: Grid,
    /// The quad's column and row.
    pub column: Vector<U32, LANES>,
    pub row: Vector<U32, LANES>,
}

/// Mark this quad and its eight neighbours, if `holding` is anything and this slot was in the list.
///
/// Nine unconditional writes of the number one. Where the quad holds nothing, or the slot was past
/// the end of the list, every one of them goes to this invocation's dump slot instead — and they all
/// go to the *same* one, so it is a cache line touched once rather than nine words moved.
pub(crate) fn wake<const LANES: u32>(
    kernel: &mut Kernel<U32>,
    where_at: Waking<LANES>,
    holding: Vector<U32, LANES>,
    inside: Vector<U32, LANES>,
    dump_base: simdr::module::Id,
) -> Result<(), LaneError> {
    let Waking {
        subgroup,
        grid,
        column,
        row,
    } = where_at;
    let lane = kernel.local_index();
    let group = kernel.workgroup_index();

    for down in 0..3_u32 {
        for across in 0..3_u32 {
            let slot = {
                let mut lanes = kernel.lanes()?;
                let one = lanes.splat_bits::<U32, LANES>(1)?;
                let none = lanes.splat_bits::<U32, LANES>(0)?;
                let minus = lanes.splat_bits::<U32, LANES>(u32::MAX)?;

                // The neighbour, wrapped into the quad grid the way the cells wrap into theirs.
                let along = lanes.splat_bits::<U32, LANES>(across)?;
                let below = lanes.splat_bits::<U32, LANES>(down)?;
                let back = lanes.mul(one, minus)?;
                let near_x = lanes.add(column, along)?;
                let near_x = lanes.add(near_x, back)?;
                let near_y = lanes.add(row, below)?;
                let near_y = lanes.add(near_y, back)?;

                let x =
                    sand::low_bits::<LANES>(&mut lanes, near_x, grid.across().trailing_zeros())?;
                let y = sand::low_bits::<LANES>(&mut lanes, near_y, grid.down().trailing_zeros())?;
                let by = lanes.splat_bits::<U32, LANES>(grid.across().trailing_zeros())?;
                let stride = lanes.shift_left(y, by)?;
                let flag = lanes.add(stride, x)?;

                // Where this invocation's discarded writes go.
                let base = lanes.splat_id::<U32, LANES>(dump_base)?;
                let mine = lanes.splat_id::<U32, LANES>(group)?;
                let seat = lanes.splat_id::<U32, LANES>(lane)?;
                // The workgroup's own width, not the quad grid's — this is an invocation index.
                let width = lanes.splat_bits::<U32, LANES>(subgroup)?;
                let spread = lanes.mul(mine, width)?;
                let elsewhere = lanes.add(base, spread)?;
                let elsewhere = lanes.add(elsewhere, seat)?;

                let both = lanes.mul(holding, inside)?;
                let waking = lanes.greater_than(both, none)?;
                lanes.select(waking, flag, elsewhere)?
            };
            // Two, not one — see `clear`. One frame is a window that closes on the phase where the
            // move it was waiting for becomes available.
            let mark = kernel.module().constant_u32(2)?;
            kernel.store_at(AWAKE, slot.id(), mark)?;

            // **And the block that quad is in**, with the same number and by the same rule, so the
            // coarse flag can never be colder than the fine one it stands for. A quad index divided
            // by a block's worth of quads, which is a shift because both are powers of two.
            let coarse = {
                let mut lanes = kernel.lanes()?;
                let none = lanes.splat_bits::<U32, LANES>(0)?;
                let by = lanes
                    .splat_bits::<U32, LANES>(crate::sweep::cells(subgroup).trailing_zeros())?;
                let blocks = grid.quads().div_ceil(crate::sweep::cells(subgroup));
                let cold = lanes.splat_bits::<U32, LANES>(blocks)?;

                let which = lanes.shift_right_logical(slot, by)?;
                let both = lanes.mul(holding, inside)?;
                let waking = lanes.greater_than(both, none)?;
                lanes.select(waking, which, cold)?
            };
            kernel.store_at(HOT, coarse.id(), mark)?;
        }
    }
    Ok(())
}

/// The wake flags, scanned into buckets and counts — the compaction's first tier.
///
/// [`crate::sweep`] without the simulation in front of it: the predicate is already in [`AWAKE`],
/// so this reads it, scans it in three tiers, and writes the quad indices that survive. Everything
/// after it — the two scans, the gather, the count — is `crate::compact` unchanged.
///
/// # Errors
///
/// As [`settle`].
pub fn mark(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    match subgroup {
        4 => marked::<4>(subgroup),
        8 => marked::<8>(subgroup),
        16 => marked::<16>(subgroup),
        32 => marked::<32>(subgroup),
        64 => marked::<64>(subgroup),
        other => Err(LaneError::NoMapping {
            lanes: other,
            width: subgroup,
        }),
    }
}

/// The mark, at a width the caller has matched to the device.
fn marked<const LANES: u32>(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    use crate::layout::COUNTS;
    use crate::sweep::{self, PER_LANE};

    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.index_type();

    let block = kernel.workgroup_index();
    let lane = kernel.local_index();
    let span = kernel.module().constant_u32(sweep::cells(subgroup))?;
    let start = kernel.module().i_mul(uint, block, span)?;
    let run = kernel.module().constant_u32(PER_LANE)?;
    let along_lane = kernel.module().i_mul(uint, lane, run)?;
    let mine = kernel.module().i_add(uint, start, along_lane)?;

    let wide = kernel.module().constant_u32(sweep::stride(subgroup))?;
    let region = kernel.module().i_mul(uint, block, wide)?;
    let past = kernel.module().constant_u32(sweep::cells(subgroup))?;
    let dump_start = kernel.module().i_add(uint, region, past)?;
    let dump_mine = kernel.module().i_add(uint, dump_start, lane)?;

    // Blocked, not striped: the two scan tiers add up to a prefix only when (lane, then step) is
    // index order. `tests/compact.rs` found that out the hard way.
    // **Before the gate, so a block that is skipped still says nothing rather than last frame's
    // something.** `gather` reads this count and takes that many entries out of the bucket; a stale
    // one would gather quads that are no longer awake, from a region nobody has written.
    let nothing = kernel.module().constant_u32(0)?;
    kernel.store_at(COUNTS, block, nothing)?;

    if_hot::<LANES, _>(&mut kernel, block, |kernel| {
        swept::<LANES>(
            kernel,
            Sweeping {
                mine,
                region,
                dump_mine,
                block,
            },
        )
    })?;
    kernel.finish()
}

/// Where one workgroup's sweep reads and writes, gathered so the call is not five arguments long.
struct Sweeping {
    /// The first quad this lane owns.
    mine: simdr::module::Id,
    /// Where this block's bucket begins.
    region: simdr::module::Id,
    /// Where this lane's discarded bucket writes go.
    dump_mine: simdr::module::Id,
    /// Which block this is.
    block: simdr::module::Id,
}

/// The sweep itself, run only for blocks that have something awake in them.
fn swept<const LANES: u32>(kernel: &mut Kernel<U32>, where_at: Sweeping) -> Result<(), LaneError> {
    use crate::layout::{BUCKETS, COUNTS};
    use crate::sweep::PER_LANE;

    let Sweeping {
        mine,
        region,
        dump_mine,
        block,
    } = where_at;
    let uint = kernel.index_type();

    let mut quads = Vec::with_capacity(PER_LANE as usize);
    let mut flags = Vec::with_capacity(PER_LANE as usize);
    for step in 0..PER_LANE {
        let along = kernel.module().constant_u32(step)?;
        let at = kernel.module().i_add(uint, mine, along)?;
        flags.push(kernel.load_at(AWAKE, at)?);
        quads.push(at);
    }

    let mut serial = Vec::with_capacity(PER_LANE as usize);
    let carried = {
        let mut lanes = kernel.lanes()?;
        let mut running = lanes.splat_bits::<U32, LANES>(0)?;
        for &flag in &flags {
            // **One or nothing, not the countdown itself.** A scan of the counters would give
            // offsets two apart and a bucket full of holes.
            let held = lanes.splat_id::<U32, LANES>(flag)?;
            let none = lanes.splat_bits::<U32, LANES>(0)?;
            let one = lanes.splat_bits::<U32, LANES>(1)?;
            let listed = lanes.greater_than(held, none)?;
            let counted = lanes.select(listed, one, none)?;
            serial.push(running);
            running = lanes.add(running, counted)?;
        }
        running
    };

    let (before, total) = {
        let mut lanes = kernel.lanes()?;
        let before = lanes.prefix_sum_exclusive::<U32, LANES>(carried)?;
        let total = lanes.reduce_sum::<U32, LANES>(carried)?;
        (before, total)
    };
    kernel.store_at(COUNTS, block, total)?;

    for (index, quad) in quads.iter().enumerate() {
        let (Some(flag), Some(earlier)) = (flags.get(index), serial.get(index).copied()) else {
            continue;
        };
        let slot = {
            let mut lanes = kernel.lanes()?;
            let here = lanes.splat_id::<U32, LANES>(region)?;
            let elsewhere = lanes.splat_id::<U32, LANES>(dump_mine)?;
            let none = lanes.splat_bits::<U32, LANES>(0)?;
            // The counter is a countdown, so anything above nothing is awake.
            let awake = lanes.splat_id::<U32, LANES>(*flag)?;

            let within = lanes.add(before, earlier)?;
            let bucket = lanes.add(here, within)?;
            let listed = lanes.greater_than(awake, none)?;
            lanes.select(listed, bucket, elsewhere)?
        };
        kernel.store_at(BUCKETS, slot.id(), *quad)?;
    }

    Ok(())
}
