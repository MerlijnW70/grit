use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, U32};

pub use crate::layout::{BINDINGS, BUCKETS, COUNTS, PARAMS, STATE};

pub const PER_LANE: u32 = 8;

pub const AWAKE: u32 = 13;

const GOLDEN: u32 = 0x9E37_79B1;
const STIR: u32 = 0x85EB_CA6B;

#[must_use]
pub const fn cells(subgroup: u32) -> u32 {
    subgroup * PER_LANE
}

#[must_use]
pub const fn stride(subgroup: u32) -> u32 {
    cells(subgroup) + subgroup
}

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

fn built<const LANES: u32>(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.index_type();

    let block = kernel.workgroup_index();
    let lane = kernel.local_index();
    let head = kernel.module().constant_u32(0)?;
    let frame = kernel.load_at(PARAMS, head)?;

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

    let mut addresses = Vec::with_capacity(PER_LANE as usize);
    let mut loaded = Vec::with_capacity(PER_LANE as usize);
    for step in 0..PER_LANE {
        let along = kernel.module().constant_u32(step)?;
        let at = kernel.module().i_add(uint, cell_mine, along)?;
        loaded.push(kernel.load_at(STATE, at)?);
        addresses.push(at);
    }

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

            let drifting = lanes.mul(now, golden)?;
            let seeded = lanes.add(held, drifting)?;
            let h = lanes.mul(seeded, stir)?;
            let folded = lanes.shift_right_logical(h, by16)?;
            let next = lanes.add(h, folded)?;

            let top = lanes.shift_right_logical(next, by24)?;
            let above = lanes.greater_than(top, sleeping)?;
            let live = lanes.select(above, none, one)?;

            serial.push(running);
            running = lanes.add(running, live)?;

            stepped.push(next);
            awake.push(live);
        }
        running
    };

    for (at, value) in addresses.iter().zip(&stepped) {
        kernel.store_at(STATE, *at, value.id())?;
    }

    let (before, total) = {
        let mut lanes = kernel.lanes()?;
        let before = lanes.prefix_sum_exclusive::<U32, LANES>(mine)?;
        let total = lanes.reduce_sum::<U32, LANES>(mine)?;
        (before, total)
    };
    kernel.store_at(COUNTS, block, total)?;

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

#[must_use]
pub fn step(held: u32, frame: u32) -> u32 {
    let drifting = frame.wrapping_mul(GOLDEN);
    let seeded = held.wrapping_add(drifting);
    let h = seeded.wrapping_mul(STIR);
    h.wrapping_add(h >> 16)
}

#[must_use]
pub fn is_awake(stepped: u32) -> bool {
    (stepped >> 24) <= AWAKE
}

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
