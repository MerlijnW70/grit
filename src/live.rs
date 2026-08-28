use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, Lanes, U32, Vector};

use crate::compact::{Over, Tier, owned};
use crate::layout::{AWAKE, BINDINGS, DENSE, GRAND, HOT, PARAMS, STATE};
use crate::sand::{self, Grid};

pub type Loop = (Vec<Vec<u32>>, Vec<Vec<crate::Groups>>);

pub fn tick(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, 1, BINDINGS))?;
    let slot = kernel.module().constant_u32(0)?;
    let one = kernel.module().constant_u32(1)?;
    let element = kernel.element();

    let now = kernel.load_at(PARAMS, slot)?;
    let next = kernel.module().i_add(element, now, one)?;
    kernel.store_at(PARAMS, slot, next)?;
    kernel.finish()
}

pub const COARSE_FROM: u32 = 2048;

#[must_use]
pub const fn coarse_pays(blocks: u32) -> bool {
    blocks >= COARSE_FROM
}

pub fn wired(
    subgroup: u32,
    grid: Grid,
    plan: &crate::compact::Plan,
    picture: Option<sand::View>,
) -> Result<Loop, LaneError> {
    arranged(subgroup, grid, plan, picture, coarse_pays(plan.blocks))
}

pub fn arranged(
    subgroup: u32,
    grid: Grid,
    plan: &crate::compact::Plan,
    picture: Option<sand::View>,
    coarse: bool,
) -> Result<Loop, LaneError> {
    use crate::Groups;
    use crate::layout::{HOT_BASES, HOT_COUNTS, HOT_DENSE, HOT_GRAND, HOT_TOTALS};
    use crate::sweep::COUNTS;

    let (listed, hot) = if coarse {
        (Over::Listed(HOT_DENSE), crate::compact::Plan::hot_listed())
    } else {
        (Over::Every, Groups::Fixed(plan.blocks))
    };
    let tiles = Groups::Fixed(plan.tiles);
    let one = Groups::Fixed(1);

    let looking = {
        let mut stage = Vec::new();
        if coarse {
            stage.push((mark(subgroup, Tier::COARSE, Over::Every)?, tiles));
        }
        if let Some(view) = picture {
            let over = u32::try_from(view.pixels())
                .unwrap_or(u32::MAX)
                .div_ceil(subgroup);
            stage.push((sand::paint(subgroup, grid, view)?, Groups::Fixed(over)));
        }
        stage
    };

    let hot_scan_counts = if coarse {
        vec![(
            crate::compact::scan(subgroup, HOT_COUNTS, HOT_BASES, HOT_TOTALS)?,
            Groups::Fixed(plan.hot_tiles),
        )]
    } else {
        Vec::new()
    };
    let hot_scan_totals = if coarse {
        vec![(
            crate::compact::scan(subgroup, HOT_TOTALS, HOT_TOTALS, HOT_GRAND)?,
            one,
        )]
    } else {
        Vec::new()
    };
    let hot_gather = if coarse {
        vec![
            (
                crate::compact::gather(subgroup, Tier::COARSE, Over::Every, plan.hot_dump())?,
                tiles,
            ),
            (
                crate::compact::prepare(subgroup, 1, Tier::COARSE)?,
                Groups::Counting(1),
            ),
        ]
    } else {
        Vec::new()
    };

    let stages = vec![
        vec![
            (tick(subgroup)?, one),
            (decay(subgroup, listed)?, hot),
            (wipe(subgroup)?, one),
            (wipe_counts(subgroup, COUNTS)?, tiles),
        ],
        vec![(
            sand::brush(subgroup, grid, true)?,
            Groups::Fixed(sand::BOX * sand::BOX / subgroup),
        )],
        vec![(settle(subgroup, grid)?, crate::compact::Plan::listed())],
        looking,
        hot_scan_counts,
        hot_scan_totals,
        hot_gather,
        vec![(mark(subgroup, Tier::FINE, listed)?, hot)],
        vec![(
            crate::compact::scan(
                subgroup,
                COUNTS,
                crate::layout::BASES,
                crate::layout::TOTALS,
            )?,
            tiles,
        )],
        vec![(
            crate::compact::scan(
                subgroup,
                crate::layout::TOTALS,
                crate::layout::TOTALS,
                crate::layout::GRAND,
            )?,
            one,
        )],
        vec![
            (
                crate::compact::gather(subgroup, Tier::FINE, listed, plan.dump())?,
                hot,
            ),
            (
                crate::compact::prepare(subgroup, subgroup, Tier::FINE)?,
                Groups::Counting(1),
            ),
        ],
    ];

    let mut passes = Vec::new();
    let mut groups = Vec::with_capacity(stages.len());
    for stage in stages {
        if stage.is_empty() {
            continue;
        }
        let mut counts = Vec::with_capacity(stage.len());
        for (module, over) in stage {
            passes.push(module);
            counts.push(over);
        }
        groups.push(counts);
    }

    Ok((passes, groups))
}

pub struct Both {
    pub passes: Vec<Vec<u32>>,
    pub listed: Vec<Vec<crate::Groups>>,
    pub every: Vec<Vec<crate::Groups>>,
}

fn silenced(stages: &[Vec<crate::Groups>]) -> Vec<Vec<crate::Groups>> {
    stages
        .iter()
        .map(|stage| stage.iter().map(|_| crate::Groups::Fixed(0)).collect())
        .collect()
}

pub fn both(
    subgroup: u32,
    grid: Grid,
    plan: &crate::compact::Plan,
    picture: Option<sand::View>,
) -> Result<Both, LaneError> {
    let (with_passes, with_stages) = arranged(subgroup, grid, plan, picture, true)?;
    let (without_passes, without_stages) = arranged(subgroup, grid, plan, picture, false)?;

    let mut passes = with_passes;
    passes.extend(without_passes);

    let mut listed = with_stages.clone();
    listed.extend(silenced(&without_stages));
    let mut every = silenced(&with_stages);
    every.extend(without_stages);

    Ok(Both {
        passes,
        listed,
        every,
    })
}

pub struct Cheaper {
    pub coarse: bool,
    pub listed: std::time::Duration,
    pub every: std::time::Duration,
    pub on_device: bool,
}

pub fn cheaper(
    held: &mut crate::Resident<'_>,
    both: &Both,
    budget: u32,
) -> Result<Cheaper, crate::Error> {
    let listed: Vec<&[crate::Groups]> = both.listed.iter().map(Vec::as_slice).collect();
    let every: Vec<&[crate::Groups]> = both.every.iter().map(Vec::as_slice).collect();

    held.staged(&listed)?;
    held.staged(&every)?;

    let clock = std::time::Instant::now();
    held.staged_frames(&listed, budget)?;
    let (listed_wall, listed_card) = (clock.elapsed(), held.on_device());

    let clock = std::time::Instant::now();
    held.staged_frames(&every, budget)?;
    let (every_wall, every_card) = (clock.elapsed(), held.on_device());

    let (with, without, on_device) = match (listed_card, every_card) {
        (Some(with), Some(without)) => (with, without, true),
        _ => (listed_wall, every_wall, false),
    };
    Ok(Cheaper {
        coarse: with < without,
        listed: with,
        every: without,
        on_device,
    })
}

pub fn seed(
    held: &mut crate::Resident<'_>,
    grid: Grid,
    plan: &crate::compact::Plan,
) -> Result<(), crate::Error> {
    use crate::layout::{HOT_DENSE, HOT_GRAND, HOT_INDIRECT, INDIRECT};

    let quads = grid.quads();
    held.write(AWAKE as usize, &vec![2_u32; quads as usize])?;
    held.write(DENSE as usize, &(0..quads).collect::<Vec<u32>>())?;
    held.write(GRAND as usize, &[quads])?;
    held.write(INDIRECT as usize, &[quads.div_ceil(plan.subgroup), 1, 1])?;

    held.write(HOT as usize, &vec![2_u32; plan.blocks as usize])?;
    held.write(HOT_DENSE as usize, &(0..plan.blocks).collect::<Vec<u32>>())?;
    held.write(HOT_GRAND as usize, &[plan.blocks])?;
    held.write(HOT_INDIRECT as usize, &[plan.blocks, 1, 1])
}

pub fn decay(subgroup: u32, over: Over) -> Result<Vec<u32>, LaneError> {
    match subgroup {
        4 => decayed::<4>(subgroup, over),
        8 => decayed::<8>(subgroup, over),
        16 => decayed::<16>(subgroup, over),
        32 => decayed::<32>(subgroup, over),
        64 => decayed::<64>(subgroup, over),
        other => Err(LaneError::NoMapping {
            lanes: other,
            width: subgroup,
        }),
    }
}

fn decayed<const LANES: u32>(subgroup: u32, over: Over) -> Result<Vec<u32>, LaneError> {
    use crate::sweep::{self, PER_LANE};

    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.index_type();

    let block = owned(&mut kernel, over)?;
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

    let mut highest = kernel.module().constant_u32(0)?;
    for offset in 0..PER_LANE {
        let along = kernel.module().constant_u32(offset)?;
        let quad = kernel.module().i_add(uint, mine, along)?;
        let held = kernel.load_at(AWAKE, quad)?;
        let lower = step(&mut kernel, held)?;
        kernel.store_at(AWAKE, quad, lower)?;
        highest = {
            let mut lanes = kernel.lanes()?;
            let so_far = lanes.splat_id::<U32, 1>(highest)?;
            let now = lanes.splat_id::<U32, 1>(lower)?;
            lanes.max(so_far, now)?.id()
        };
    }

    let flag = {
        let mut lanes = kernel.lanes()?;
        let mine = lanes.splat_id::<U32, LANES>(highest)?;
        lanes.reduce_max::<U32, LANES>(mine)?
    };
    kernel.store_at(HOT, block, flag)?;
    kernel.finish()
}

pub fn wipe(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    use crate::layout::{HOT_TOTALS, TALLY, TALLY_WORDS, TOTALS};
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
        kernel.store_at(HOT_TOTALS, at, nothing)?;
    }

    for word in 0..TALLY_WORDS {
        let at = kernel
            .module()
            .constant_u32(u32::try_from(word).unwrap_or(0))?;
        kernel.store_at(TALLY, at, nothing)?;
    }
    kernel.finish()
}

pub fn wipe_counts(subgroup: u32, counts: u32) -> Result<Vec<u32>, LaneError> {
    use crate::sweep::{self, PER_LANE};

    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.index_type();

    let block = kernel.workgroup_index();
    let lane = kernel.local_index();
    let span = kernel.module().constant_u32(sweep::cells(subgroup))?;
    let start = kernel.module().i_mul(uint, block, span)?;
    let run = kernel.module().constant_u32(PER_LANE)?;
    let along = kernel.module().i_mul(uint, lane, run)?;
    let mine = kernel.module().i_add(uint, start, along)?;
    let nothing = kernel.module().constant_u32(0)?;

    for step in 0..PER_LANE {
        let at = kernel.module().constant_u32(step)?;
        let at = kernel.module().i_add(uint, mine, at)?;
        kernel.store_at(counts, at, nothing)?;
    }
    kernel.finish()
}

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

    let inside = {
        let mut lanes = kernel.lanes()?;
        let here = lanes.splat_id::<U32, LANES>(slot)?;
        let total = lanes.splat_id::<U32, LANES>(grand)?;
        let one = lanes.splat_bits::<U32, LANES>(1)?;
        let none = lanes.splat_bits::<U32, LANES>(0)?;
        let listed = lanes.greater_than(total, here)?;
        lanes.select(listed, one, none)?
    };

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

        let a = sand::hold::<LANES>(&mut lanes, a, c)?;
        let b = sand::hold::<LANES>(&mut lanes, b, d)?;
        let [sa, sb, sc, sd] = sand::start::<LANES>(&mut lanes, [a, b, c, d])?;
        let (sa, sb) = sand::course::<LANES>(&mut lanes, sa, sb)?;
        let (sc, sd) = sand::course::<LANES>(&mut lanes, sc, sd)?;

        let (na, nc) = sand::fall::<LANES>(&mut lanes, sa, sc)?;
        let (nb, nd) = sand::fall::<LANES>(&mut lanes, sb, sd)?;
        let (na, nd) = sand::fall::<LANES>(&mut lanes, na, nd)?;
        let (nb, nc) = sand::fall::<LANES>(&mut lanes, nb, nc)?;

        let mut count = lanes.splat_bits::<U32, LANES>(0)?;
        for (before, after) in [(a, na), (b, nb), (c, nc), (d, nd)] {
            let one = lanes.splat_bits::<U32, LANES>(1)?;
            let none = lanes.splat_bits::<U32, LANES>(0)?;
            let same = lanes.equal(before, after)?;
            let differs = lanes.select(same, none, one)?;
            count = lanes.add(count, differs)?;
        }

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

struct Tallying<const LANES: u32> {
    counted: [Vector<U32, LANES>; 3],
    column: Vector<U32, LANES>,
    row: Vector<U32, LANES>,
    inside: Vector<U32, LANES>,
}

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

        let moved = keep(&mut lanes, moved)?;
        let wet = keep(&mut lanes, wet)?;
        let dry = keep(&mut lanes, dry)?;

        let by = lanes.splat_bits::<U32, LANES>(COARSE)?;
        let across = lanes.shift_right_logical(column, by)?;
        let down = lanes.shift_right_logical(row, by)?;
        let across = lanes.mul(across, moved)?;
        let down = lanes.mul(down, moved)?;

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
        let _ = kernel.atomic_add_at(TALLY, at, mine.id())?;
    }
    Ok(())
}

type Corners<const LANES: u32> = (
    [Vector<U32, LANES>; 4],
    Vector<U32, LANES>,
    Vector<U32, LANES>,
);

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

pub(crate) struct Waking<const LANES: u32> {
    pub subgroup: u32,
    pub grid: Grid,
    pub column: Vector<U32, LANES>,
    pub row: Vector<U32, LANES>,
}

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

                let base = lanes.splat_id::<U32, LANES>(dump_base)?;
                let mine = lanes.splat_id::<U32, LANES>(group)?;
                let seat = lanes.splat_id::<U32, LANES>(lane)?;
                let width = lanes.splat_bits::<U32, LANES>(subgroup)?;
                let spread = lanes.mul(mine, width)?;
                let elsewhere = lanes.add(base, spread)?;
                let elsewhere = lanes.add(elsewhere, seat)?;

                let both = lanes.mul(holding, inside)?;
                let waking = lanes.greater_than(both, none)?;
                lanes.select(waking, flag, elsewhere)?
            };
            let mark = kernel.module().constant_u32(2)?;
            kernel.store_at(AWAKE, slot.id(), mark)?;

            let coarse = {
                let mut lanes = kernel.lanes()?;
                let none = lanes.splat_bits::<U32, LANES>(0)?;
                let per_block = crate::sweep::cells(subgroup);
                let by = lanes.splat_bits::<U32, LANES>(per_block.trailing_zeros())?;
                let blocks = grid.quads().div_ceil(per_block);
                let cold =
                    lanes.splat_bits::<U32, LANES>(blocks.div_ceil(per_block) * per_block)?;

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

pub fn mark(subgroup: u32, tier: Tier, over: Over) -> Result<Vec<u32>, LaneError> {
    match subgroup {
        4 => marked::<4>(subgroup, tier, over),
        8 => marked::<8>(subgroup, tier, over),
        16 => marked::<16>(subgroup, tier, over),
        32 => marked::<32>(subgroup, tier, over),
        64 => marked::<64>(subgroup, tier, over),
        other => Err(LaneError::NoMapping {
            lanes: other,
            width: subgroup,
        }),
    }
}

fn marked<const LANES: u32>(subgroup: u32, tier: Tier, over: Over) -> Result<Vec<u32>, LaneError> {
    use crate::sweep::{self, PER_LANE};

    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.index_type();

    let block = owned(&mut kernel, over)?;
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

    swept::<LANES>(
        &mut kernel,
        Sweeping {
            mine,
            region,
            dump_mine,
            block,
            tier,
        },
    )?;
    kernel.finish()
}

struct Sweeping {
    mine: simdr::module::Id,
    region: simdr::module::Id,
    dump_mine: simdr::module::Id,
    block: simdr::module::Id,
    tier: Tier,
}

fn swept<const LANES: u32>(kernel: &mut Kernel<U32>, where_at: Sweeping) -> Result<(), LaneError> {
    use crate::sweep::PER_LANE;

    let Sweeping {
        mine,
        region,
        dump_mine,
        block,
        tier,
    } = where_at;
    let uint = kernel.index_type();

    let mut quads = Vec::with_capacity(PER_LANE as usize);
    let mut flags = Vec::with_capacity(PER_LANE as usize);
    for step in 0..PER_LANE {
        let along = kernel.module().constant_u32(step)?;
        let at = kernel.module().i_add(uint, mine, along)?;
        flags.push(kernel.load_at(tier.flags, at)?);
        quads.push(at);
    }

    let mut serial = Vec::with_capacity(PER_LANE as usize);
    let carried = {
        let mut lanes = kernel.lanes()?;
        let mut running = lanes.splat_bits::<U32, LANES>(0)?;
        for &flag in &flags {
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
    kernel.store_at(tier.counts, block, total)?;

    for (index, quad) in quads.iter().enumerate() {
        let (Some(flag), Some(earlier)) = (flags.get(index), serial.get(index).copied()) else {
            continue;
        };
        let slot = {
            let mut lanes = kernel.lanes()?;
            let here = lanes.splat_id::<U32, LANES>(region)?;
            let elsewhere = lanes.splat_id::<U32, LANES>(dump_mine)?;
            let none = lanes.splat_bits::<U32, LANES>(0)?;
            let awake = lanes.splat_id::<U32, LANES>(*flag)?;

            let within = lanes.add(before, earlier)?;
            let bucket = lanes.add(here, within)?;
            let listed = lanes.greater_than(awake, none)?;
            lanes.select(listed, bucket, elsewhere)?
        };
        kernel.store_at(tier.buckets, slot.id(), *quad)?;
    }

    Ok(())
}
