use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, Lanes, U32, Vector};

use crate::compact::{Over, Tier, owned};
use crate::geometry::{Emit, Placed, Plane, Space};
use crate::layout::{AWAKE, BINDINGS, DENSE, Dimension, GRAND, HOT, PARAMS, STATE};
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
    sand::flat("live::wired", grid)?;
    arranged(subgroup, grid, plan, picture, coarse_pays(plan.blocks))
}

pub type Stages = Vec<Vec<(Vec<u32>, crate::Groups)>>;

pub struct Keeping {
    pub before: Stages,
    pub after: Stages,
    pub over: crate::Groups,
    pub brush: crate::Groups,
}

pub fn keeping(
    subgroup: u32,
    plan: &crate::compact::Plan,
    coarse: bool,
) -> Result<Keeping, LaneError> {
    use crate::Groups;
    use crate::layout::{BASES, GRAND, HOT_BASES, HOT_COUNTS, HOT_DENSE, TOTALS};
    use crate::sweep::COUNTS;

    let (listed, hot) = if coarse {
        (Over::Listed(HOT_DENSE), crate::compact::Plan::hot_listed())
    } else {
        (Over::Every, Groups::Fixed(plan.blocks))
    };
    let tiles = Groups::Fixed(plan.tiles);
    let one = Groups::Fixed(1);

    let before = vec![vec![
        (decay(subgroup, listed)?, hot),
        (wipe(subgroup)?, one),
        (wipe_counts(subgroup, COUNTS)?, tiles),
        (unclaimed(subgroup, plan)?, one),
    ]];

    let mut after: Stages = Vec::new();
    if coarse {
        after.push(vec![(
            mark_below(subgroup, Tier::COARSE, Over::Every, plan.blocks)?,
            tiles,
        )]);
        after.push(vec![(
            crate::compact::whole(subgroup, HOT_COUNTS, HOT_BASES, Tier::COARSE)?,
            one,
        )]);
        after.push(vec![
            (
                crate::compact::gather(subgroup, Tier::COARSE, Over::Every, plan.hot_dump())?,
                tiles,
            ),
            (
                crate::compact::prepare(subgroup, 1, Tier::COARSE)?,
                Groups::Counting(1),
            ),
        ]);
    }
    after.push(vec![(
        mark_below(subgroup, Tier::FINE, listed, plan.dump())?,
        hot,
    )]);
    if plan.tiles == 1 {
        after.push(vec![(
            crate::compact::whole(subgroup, COUNTS, BASES, Tier::FINE)?,
            one,
        )]);
    } else {
        after.push(vec![(
            crate::compact::scan(subgroup, COUNTS, BASES, TOTALS)?,
            tiles,
        )]);
        after.push(vec![(
            crate::compact::scan(subgroup, TOTALS, TOTALS, GRAND)?,
            one,
        )]);
    }
    after.push(vec![
        (
            crate::compact::gather(subgroup, Tier::FINE, listed, plan.dump())?,
            hot,
        ),
        (
            crate::compact::prepare(subgroup, subgroup, Tier::FINE)?,
            Groups::Counting(1),
        ),
    ]);

    Ok(Keeping {
        before,
        after,
        over: crate::compact::Plan::listed(),
        brush: sand::seats(plan),
    })
}

#[must_use]
pub fn staged(stages: Stages) -> Loop {
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
    (passes, groups)
}

pub fn arranged(
    subgroup: u32,
    grid: Grid,
    plan: &crate::compact::Plan,
    picture: Option<sand::View>,
    coarse: bool,
) -> Result<Loop, LaneError> {
    use crate::Groups;
    use crate::layout::{HOT_BASES, HOT_COUNTS, HOT_DENSE};
    use crate::sweep::COUNTS;

    sand::flat("live::arranged", grid)?;

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
            stage.push((
                mark_below(subgroup, Tier::COARSE, Over::Every, plan.blocks)?,
                tiles,
            ));
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
            crate::compact::whole(subgroup, HOT_COUNTS, HOT_BASES, Tier::COARSE)?,
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

    let fine_scan_counts = if plan.tiles == 1 {
        vec![(
            crate::compact::whole(subgroup, COUNTS, crate::layout::BASES, Tier::FINE)?,
            one,
        )]
    } else {
        vec![(
            crate::compact::scan(
                subgroup,
                COUNTS,
                crate::layout::BASES,
                crate::layout::TOTALS,
            )?,
            tiles,
        )]
    };
    let fine_scan_totals = if plan.tiles > 1 {
        vec![(
            crate::compact::scan(
                subgroup,
                crate::layout::TOTALS,
                crate::layout::TOTALS,
                crate::layout::GRAND,
            )?,
            one,
        )]
    } else {
        Vec::new()
    };

    let stages = vec![
        vec![
            (tick(subgroup)?, one),
            (decay(subgroup, listed)?, hot),
            (wipe(subgroup)?, one),
            (wipe_counts(subgroup, COUNTS)?, tiles),
            (unclaimed(subgroup, plan)?, one),
        ],
        vec![(sand::brush(subgroup, grid, true)?, sand::seats(plan))],
        vec![(settle(subgroup, grid)?, crate::compact::Plan::listed())],
        looking,
        hot_scan_counts,
        hot_gather,
        vec![(mark_below(subgroup, Tier::FINE, listed, grid.quads())?, hot)],
        fine_scan_counts,
        fine_scan_totals,
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

/// Why a 3D builder refuses a plan laid out for a flat grid.
pub const FLAT_PLAN: &str =
    "the plan is laid out for a flat grid, and a solid grid has eight cells to a block";

/// Why a 3D builder refuses a plan that holds fewer blocks than the grid.
pub const SHORT_PLAN: &str = "the plan holds fewer blocks than the grid has octs";

const fn solid(operation: &'static str, grid: Grid) -> Result<(), LaneError> {
    if grid.tall > 1 {
        Ok(())
    } else {
        Err(LaneError::NoSuchForm {
            operation,
            because: crate::sand3::FLAT,
        })
    }
}

fn fits(operation: &'static str, grid: Grid, plan: &crate::compact::Plan) -> Result<(), LaneError> {
    solid(operation, grid)?;
    if plan.dimension != Dimension::Three {
        return Err(LaneError::NoSuchForm {
            operation,
            because: FLAT_PLAN,
        });
    }
    if plan.quads < grid.blocks() as usize {
        return Err(LaneError::NoSuchForm {
            operation,
            because: SHORT_PLAN,
        });
    }
    Ok(())
}

/// [`arranged3`] on the tier the plan's size pays for, as [`wired`] picks it.
pub fn wired3(subgroup: u32, grid: Grid, plan: &crate::compact::Plan) -> Result<Loop, LaneError> {
    fits("live::wired3", grid, plan)?;
    arranged3(subgroup, grid, plan, coarse_pays(plan.blocks))
}

/// The frame of the listed engine on a solid grid, the 3D counterpart of
/// [`arranged`] without the brush and the picture: tick, decay and wipe,
/// [`settle3`] over the listed octs, then the marks, scans and gathers that
/// list the octs of the next frame. `plan` is laid out for
/// [`Dimension::Three`] and holds every oct of `grid`.
pub fn arranged3(
    subgroup: u32,
    grid: Grid,
    plan: &crate::compact::Plan,
    coarse: bool,
) -> Result<Loop, LaneError> {
    use crate::Groups;
    use crate::layout::{BASES, HOT_BASES, HOT_COUNTS, HOT_DENSE, TOTALS};
    use crate::sweep::COUNTS;

    fits("live::arranged3", grid, plan)?;

    let (listed, hot) = if coarse {
        (Over::Listed(HOT_DENSE), crate::compact::Plan::hot_listed())
    } else {
        (Over::Every, Groups::Fixed(plan.blocks))
    };
    let tiles = Groups::Fixed(plan.tiles);
    let one = Groups::Fixed(1);

    let mut stages: Stages = vec![
        vec![
            (tick(subgroup)?, one),
            (decay(subgroup, listed)?, hot),
            (wipe_in(subgroup, Dimension::Three)?, one),
            (wipe_counts(subgroup, COUNTS)?, tiles),
            (unclaimed(subgroup, plan)?, one),
        ],
        vec![(settle3(subgroup, grid)?, crate::compact::Plan::listed())],
    ];
    if coarse {
        stages.push(vec![(
            mark_below(subgroup, Tier::COARSE, Over::Every, plan.blocks)?,
            tiles,
        )]);
        stages.push(vec![(
            crate::compact::whole(subgroup, HOT_COUNTS, HOT_BASES, Tier::COARSE)?,
            one,
        )]);
        stages.push(vec![
            (
                crate::compact::gather(subgroup, Tier::COARSE, Over::Every, plan.hot_dump())?,
                tiles,
            ),
            (
                crate::compact::prepare(subgroup, 1, Tier::COARSE)?,
                Groups::Counting(1),
            ),
        ]);
    }
    stages.push(vec![(
        mark_below(subgroup, Tier::FINE, listed, grid.blocks())?,
        hot,
    )]);
    if plan.tiles == 1 {
        stages.push(vec![(
            crate::compact::whole(subgroup, COUNTS, BASES, Tier::FINE)?,
            one,
        )]);
    } else {
        stages.push(vec![(
            crate::compact::scan(subgroup, COUNTS, BASES, TOTALS)?,
            tiles,
        )]);
        stages.push(vec![(
            crate::compact::scan(subgroup, TOTALS, TOTALS, GRAND)?,
            one,
        )]);
    }
    stages.push(vec![
        (
            crate::compact::gather(subgroup, Tier::FINE, listed, plan.dump())?,
            hot,
        ),
        (
            crate::compact::prepare(subgroup, subgroup, Tier::FINE)?,
            Groups::Counting(1),
        ),
    ]);
    Ok(staged(stages))
}

/// Writes `cells` (each a cell index and its new word) into the world of a
/// listed engine on a solid grid between frames, and wakes what they touch:
/// the oct of either phase that holds each cell is marked awake and hot, as
/// [`seed`] marks every oct, and is put on the list the next frame steps,
/// once however many cells it holds. The list keeps the octs it held.
///
/// No frame may be in flight. A cell past the grid is refused with
/// [`Error::Overrun`](crate::Error::Overrun) before anything is written.
pub fn put3(
    held: &mut crate::Resident<'_>,
    grid: Grid,
    plan: &crate::compact::Plan,
    cells: &[(u32, u32)],
) -> Result<(), crate::Error> {
    use crate::geometry::Solid;
    use crate::layout::INDIRECT;

    fits("live::put3", grid, plan).map_err(crate::Error::Lane)?;
    let past = grid.cells();
    if let Some(&(cell, _)) = cells.iter().find(|(cell, _)| *cell as usize >= past) {
        return Err(crate::Error::Overrun {
            index: STATE as usize,
            needed: cell as usize + 1,
            held: past,
        });
    }

    let grand = held.read(GRAND as usize, 1)?.first().copied().unwrap_or(0);
    let mut octs: std::collections::BTreeSet<u32> = held
        .read(DENSE as usize, grand as usize)?
        .into_iter()
        .collect();
    let mut woken = std::collections::BTreeSet::new();
    for &(cell, word) in cells {
        held.write_at(STATE as usize, cell as usize, &[word])?;
        for phase in 0..2 {
            woken.insert(Solid::held(grid, cell, phase).index);
        }
    }

    let by = plan.per_block.trailing_zeros();
    for &oct in &woken {
        held.write_at(AWAKE as usize, oct as usize, &[2])?;
        held.write_at(HOT as usize, (oct >> by) as usize, &[2])?;
    }
    octs.extend(woken);
    let list: Vec<u32> = octs.into_iter().collect();
    let count = u32::try_from(list.len()).unwrap_or(u32::MAX);
    held.write(DENSE as usize, &list)?;
    held.write(GRAND as usize, &[count])?;
    held.write(INDIRECT as usize, &[count.div_ceil(plan.subgroup), 1, 1])
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
    sand::flat("live::both", grid)?;
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

/// Why [`seed`] refuses a plan laid out for another number of axes than the
/// grid has.
pub const UNLIKE: &str = "the plan is laid out for another number of axes than the grid has";

/// Sets up every word the listed engine reads before a frame writes it, so
/// the first frame after `seed` is right whatever the buffers held before.
///
/// The caller writes the world into `STATE` and the brush words of `PARAMS`;
/// the buffers hold at least `plan.sizes()`. After `seed`:
///
/// - `PARAMS` holds `u32::MAX` as the frame, so the first `tick` makes it
///   frame 0, and `plan.dump()` as the dump base. The other words of
///   `PARAMS` are the caller's and keep what they held.
/// - Every block of the grid, each quad of a flat grid and each oct of a
///   solid one, is awake and listed: `AWAKE`, `DENSE`, `GRAND` and
///   `INDIRECT`. The rest of `AWAKE` is zero: the plan's blocks past the
///   grid, and the scratch `wake` writes for lanes that wake nothing.
/// - Every block of the plan is hot and listed: `HOT`, `HOT_DENSE`,
///   `HOT_GRAND` and `HOT_INDIRECT`. The rest of `HOT` is zero: the padding
///   up to a whole tile, which the coarse mark reads, and the cold slot.
/// - `HOT_COUNTS`, which the coarse scan reads past the tiles the coarse
///   mark writes, `TALLY`, and the brush's claim count at `DENSE[dump]` are
///   zero.
///
/// A plan laid out for another number of axes than the grid has is refused
/// with [`UNLIKE`], before anything is written.
pub fn seed(
    held: &mut crate::Resident<'_>,
    grid: Grid,
    plan: &crate::compact::Plan,
) -> Result<(), crate::Error> {
    use crate::layout::{HOT_COUNTS, HOT_DENSE, HOT_GRAND, HOT_INDIRECT, INDIRECT, TALLY};

    let sizes = plan.sizes();
    let words = |binding: u32| sizes.get(binding as usize).copied().unwrap_or(0);
    let leading = |marked: usize, held: usize| {
        let mut words = vec![0_u32; held.max(marked)];
        for word in words.iter_mut().take(marked) {
            *word = 2;
        }
        words
    };

    if grid.axes() != plan.dimension.axes() {
        return Err(crate::Error::Lane(LaneError::NoSuchForm {
            operation: "live::seed",
            because: UNLIKE,
        }));
    }

    held.write(PARAMS as usize, &[u32::MAX, plan.dump()])?;

    let quads = grid.blocks();
    held.write(AWAKE as usize, &leading(quads as usize, words(AWAKE)))?;
    held.write(DENSE as usize, &(0..quads).collect::<Vec<u32>>())?;
    held.write_at(DENSE as usize, plan.dump() as usize, &[0])?;
    held.write(GRAND as usize, &[quads])?;
    held.write(INDIRECT as usize, &[quads.div_ceil(plan.subgroup), 1, 1])?;

    held.write(HOT as usize, &leading(plan.blocks as usize, words(HOT)))?;
    held.write(HOT_DENSE as usize, &(0..plan.blocks).collect::<Vec<u32>>())?;
    held.write(HOT_GRAND as usize, &[plan.blocks])?;
    held.write(HOT_INDIRECT as usize, &[plan.blocks, 1, 1])?;
    held.write(HOT_COUNTS as usize, &vec![0; words(HOT_COUNTS).max(1)])?;

    held.write(TALLY as usize, &vec![0; plan.dimension.tally_words()])
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

    let mut quads = Vec::with_capacity(PER_LANE as usize);
    let mut held = Vec::with_capacity(PER_LANE as usize);
    for offset in 0..PER_LANE {
        let along = kernel.module().constant_u32(offset)?;
        let quad = kernel.module().i_add(uint, mine, along)?;
        held.push(kernel.load_at(AWAKE, quad)?);
        quads.push(quad);
    }
    let mut lowered = Vec::with_capacity(PER_LANE as usize);
    for &was in &held {
        lowered.push(step(&mut kernel, was)?);
    }
    for (&quad, &lower) in quads.iter().zip(&lowered) {
        kernel.store_at(AWAKE, quad, lower)?;
    }
    let mut highest = kernel.module().constant_u32(0)?;
    for &lower in &lowered {
        let mut lanes = kernel.lanes()?;
        let so_far = lanes.splat_id::<U32, 1>(highest)?;
        let now = lanes.splat_id::<U32, 1>(lower)?;
        highest = lanes.max(so_far, now)?.id();
    }

    let flag = {
        let mut lanes = kernel.lanes()?;
        let mine = lanes.splat_id::<U32, LANES>(highest)?;
        lanes.reduce_max::<U32, LANES>(mine)?
    };
    kernel.store_at(HOT, block, flag)?;
    kernel.finish()
}

/// Zeroes the count of quads the brush adds to the list, which is kept in
/// `DENSE` at the dump base, past the end of any list, and writes the
/// brush's dispatch count for this frame right after it, at
/// [`sand::counted_at`]: no group while the brush paints nothing.
pub fn unclaimed(subgroup: u32, plan: &crate::compact::Plan) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, 1, BINDINGS))?;
    let second = kernel.module().constant_u32(1)?;
    let nothing = kernel.module().constant_u32(0)?;
    let at = kernel.load_at(PARAMS, second)?;
    kernel.store_at(DENSE, at, nothing)?;
    sand::reckon(&mut kernel, subgroup, sand::counted_at(plan))?;
    kernel.finish()
}

pub fn wipe(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    wipe_in(subgroup, Dimension::Two)
}

pub fn wipe_in(subgroup: u32, dimension: Dimension) -> Result<Vec<u32>, LaneError> {
    use crate::layout::{HOT_TOTALS, TALLY, TOTALS};
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

    for word in 0..dimension.tally_words() {
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
    sand::flat("live::settle", grid)?;
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
    let claimed = kernel.load_at(DENSE, dump_base)?;

    let (inside, counted_in) = {
        let mut lanes = kernel.lanes()?;
        let here = lanes.splat_id::<U32, LANES>(slot)?;
        let listed = lanes.splat_id::<U32, LANES>(grand)?;
        let added = lanes.splat_id::<U32, LANES>(claimed)?;
        let total = lanes.add(listed, added)?;
        let one = lanes.splat_bits::<U32, LANES>(1)?;
        let none = lanes.splat_bits::<U32, LANES>(0)?;
        let settled = lanes.greater_than(total, here)?;
        let counted = lanes.greater_than(listed, here)?;
        (
            lanes.select(settled, one, none)?,
            lanes.select(counted, one, none)?,
        )
    };

    let quad = kernel.load_at(DENSE, slot)?;
    let corners = corners::<LANES>(&mut kernel, grid, quad, frame)?;
    let (places, column, row) = (corners.places, corners.along, corners.down);

    let held = [
        kernel.load_at(STATE, places[0].id())?,
        kernel.load_at(STATE, places[1].id())?,
        kernel.load_at(STATE, places[2].id())?,
        kernel.load_at(STATE, places[3].id())?,
    ];

    let (settled, counted, touched) = {
        let mut lanes = kernel.lanes()?;
        let a = lanes.splat_id::<U32, LANES>(held[0])?;
        let b = lanes.splat_id::<U32, LANES>(held[1])?;
        let c = lanes.splat_id::<U32, LANES>(held[2])?;
        let d = lanes.splat_id::<U32, LANES>(held[3])?;
        let loaded = [a, b, c, d];

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

        let mut touched = lanes.splat_bits::<U32, LANES>(0)?;
        for (before, after) in loaded.into_iter().zip([na, nb, nc, nd]) {
            let one = lanes.splat_bits::<U32, LANES>(1)?;
            let none = lanes.splat_bits::<U32, LANES>(0)?;
            let same = lanes.equal(before, after)?;
            let differs = lanes.select(same, none, one)?;
            touched = lanes.add(touched, differs)?;
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
        ([na, nb, nc, nd], [count, wet, dry], touched)
    };

    let spare = {
        let mut lanes = kernel.lanes()?;
        let past =
            lanes.splat_bits::<U32, LANES>(u32::try_from(grid.cells()).unwrap_or(u32::MAX))?;
        let mine = lanes.splat_id::<U32, LANES>(slot)?;
        let shift = lanes.splat_bits::<U32, LANES>(Dimension::Two.corner_bits())?;
        let along = lanes.shift_left(mine, shift)?;
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
    wake::<LANES>(&mut kernel, where_at, touched, inside, dump_base)?;

    let what = Tallying {
        counted,
        at: [column, row],
        inside: counted_in,
    };
    tally::<LANES, 2>(&mut kernel, what)?;
    kernel.finish()
}

/// The listed settle of a solid grid: each listed entry is one oct, stepped
/// by [`crate::sand3::settle_oct`] with 8 loads and 8 stores of `STATE`
/// through [`crate::geometry::Solid`]. An oct that changes wakes itself and
/// the 26 octs around it ([`crate::geometry::Space::near`]), and the frame's
/// counts go to the [`Dimension::Three`] tally. Lanes past the list store
/// into the spare region of `STATE`, 8 words a slot from the grid's end.
pub fn settle3(subgroup: u32, grid: Grid) -> Result<Vec<u32>, LaneError> {
    solid("live::settle3", grid)?;
    match subgroup {
        4 => listed3::<4>(subgroup, grid),
        8 => listed3::<8>(subgroup, grid),
        16 => listed3::<16>(subgroup, grid),
        32 => listed3::<32>(subgroup, grid),
        64 => listed3::<64>(subgroup, grid),
        other => Err(LaneError::NoMapping {
            lanes: other,
            width: subgroup,
        }),
    }
}

fn listed3<const LANES: u32>(subgroup: u32, grid: Grid) -> Result<Vec<u32>, LaneError> {
    use crate::sand3::{CORNERS, placed, stepped};

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
    let claimed = kernel.load_at(DENSE, dump_base)?;

    let (inside, counted_in) = {
        let mut lanes = kernel.lanes()?;
        let here = lanes.splat_id::<U32, LANES>(slot)?;
        let listed = lanes.splat_id::<U32, LANES>(grand)?;
        let added = lanes.splat_id::<U32, LANES>(claimed)?;
        let total = lanes.add(listed, added)?;
        let one = lanes.splat_bits::<U32, LANES>(1)?;
        let none = lanes.splat_bits::<U32, LANES>(0)?;
        let settled = lanes.greater_than(total, here)?;
        let counted = lanes.greater_than(listed, here)?;
        (
            lanes.select(settled, one, none)?,
            lanes.select(counted, one, none)?,
        )
    };

    let oct = kernel.load_at(DENSE, slot)?;
    let placing = {
        let mut lanes = kernel.lanes()?;
        placed::<LANES>(&mut lanes, grid, oct, frame)?
    };

    let mut held = [frame; CORNERS];
    for (word, place) in held.iter_mut().zip(&placing.places) {
        *word = kernel.load_at(STATE, place.id())?;
    }

    let (settled, counted, touched) = {
        let mut lanes = kernel.lanes()?;
        let step = stepped::<LANES>(&mut lanes, frame, held)?;
        let one = lanes.splat_bits::<U32, LANES>(1)?;
        let none = lanes.splat_bits::<U32, LANES>(0)?;

        let mut count = none;
        for (before, after) in step.held.into_iter().zip(step.ruled) {
            let same = lanes.equal(before, after)?;
            let differs = lanes.select(same, none, one)?;
            count = lanes.add(count, differs)?;
        }

        let mut touched = none;
        for (before, after) in step.loaded.into_iter().zip(step.out) {
            let same = lanes.equal(before, after)?;
            let differs = lanes.select(same, none, one)?;
            touched = lanes.add(touched, differs)?;
        }

        let water = lanes.splat_bits::<U32, LANES>(sand::WATER)?;
        let grain = lanes.splat_bits::<U32, LANES>(sand::SAND)?;
        let mut wet = none;
        let mut dry = none;
        for cell in step.out {
            let kind = sand::material::<LANES>(&mut lanes, cell)?;
            let is_wet = lanes.equal(kind, water)?;
            let is_dry = lanes.equal(kind, grain)?;
            let add_wet = lanes.select(is_wet, one, none)?;
            let add_dry = lanes.select(is_dry, one, none)?;
            wet = lanes.add(wet, add_wet)?;
            dry = lanes.add(dry, add_dry)?;
        }
        (step.out, [count, wet, dry], touched)
    };

    let spare = {
        let mut lanes = kernel.lanes()?;
        let past =
            lanes.splat_bits::<U32, LANES>(u32::try_from(grid.cells()).unwrap_or(u32::MAX))?;
        let mine = lanes.splat_id::<U32, LANES>(slot)?;
        let shift = lanes.splat_bits::<U32, LANES>(Dimension::Three.corner_bits())?;
        let along = lanes.shift_left(mine, shift)?;
        lanes.add(past, along)?
    };

    for (corner, (place, value)) in placing.places.iter().zip(&settled).enumerate() {
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

    let block = Block::<LANES, 3> {
        subgroup,
        grid,
        at: placing.block,
    };
    woken::<LANES, 3, 8, 27>(&mut kernel, block, [touched, inside], dump_base)?;

    let what = Tallying {
        counted,
        at: placing.block,
        inside: counted_in,
    };
    tally::<LANES, 3>(&mut kernel, what)?;
    kernel.finish()
}

struct Tallying<const LANES: u32, const AXES: usize> {
    counted: [Vector<U32, LANES>; 3],
    at: [Vector<U32, LANES>; AXES],
    inside: Vector<U32, LANES>,
}

/// Adds a frame's counts to `TALLY`: the cells that moved, the wet and the
/// dry cells, the moved cells weighted by each axis of their block shifted
/// down by [`COARSE`](crate::layout::COARSE), and the blocks that moved at
/// all. That is `3 + AXES + 1` words, the dimension's tally.
fn tally<const LANES: u32, const AXES: usize>(
    kernel: &mut Kernel<U32>,
    what: Tallying<LANES, AXES>,
) -> Result<(), LaneError> {
    use crate::layout::{COARSE, TALLY};

    let Tallying {
        counted,
        at,
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
        let mut weighted = at;
        for value in &mut weighted {
            *value = lanes.shift_right_logical(*value, by)?;
        }
        for value in &mut weighted {
            *value = lanes.mul(*value, moved)?;
        }

        let stirred = lanes.greater_than(moved, none)?;
        let stirred = lanes.select(stirred, one, none)?;

        let mut shares = Vec::with_capacity(AXES + 4);
        shares.extend([moved, wet, dry]);
        shares.extend(weighted);
        shares.push(stirred);
        shares
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

pub struct Corners<const LANES: u32> {
    pub places: [Vector<U32, LANES>; 4],
    pub along: Vector<U32, LANES>,
    pub down: Vector<U32, LANES>,
}

pub fn corners<const LANES: u32>(
    kernel: &mut Kernel<U32>,
    grid: Grid,
    quad: simdr::module::Id,
    frame: simdr::module::Id,
) -> Result<Corners<LANES>, LaneError> {
    sand::flat("live::corners", grid)?;
    let mut lanes = kernel.lanes()?;
    let which = lanes.splat_id::<U32, LANES>(quad)?;
    let now = lanes.splat_id::<U32, LANES>(frame)?;
    let Placed {
        block: [along, down],
        places,
    } = Plane::block(&mut Emit::<LANES>(&mut lanes), grid, which, now)?;
    Ok(Corners {
        places,
        along,
        down,
    })
}

pub struct Waking<const LANES: u32> {
    pub subgroup: u32,
    pub grid: Grid,
    pub column: Vector<U32, LANES>,
    pub row: Vector<U32, LANES>,
}

pub fn wake<const LANES: u32>(
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
    sand::flat("live::wake", grid)?;
    let block = Block::<LANES, 2> {
        subgroup,
        grid,
        at: [column, row],
    };
    woken::<LANES, 2, 4, 9>(kernel, block, [holding, inside], dump_base)
}

struct Block<const LANES: u32, const AXES: usize> {
    subgroup: u32,
    grid: Grid,
    at: [Vector<U32, LANES>; AXES],
}

/// Wakes every block of [`Space::near`] around `block`, in a world of any
/// number of axes.
fn woken<const LANES: u32, const AXES: usize, const CORNERS: usize, const NEAR: usize>(
    kernel: &mut Kernel<U32>,
    block: Block<LANES, AXES>,
    [holding, inside]: [Vector<U32, LANES>; 2],
    dump_base: simdr::module::Id,
) -> Result<(), LaneError> {
    let Block { subgroup, grid, at } = block;
    let lane = kernel.local_index();
    let group = kernel.workgroup_index();

    for offset in Space::<AXES, CORNERS, NEAR>::near() {
        let slot = {
            let mut lanes = kernel.lanes()?;
            let none = lanes.splat_bits::<U32, LANES>(0)?;
            let flag = Space::<AXES, CORNERS, NEAR>::neighbour(
                &mut Emit::<LANES>(&mut lanes),
                grid,
                at,
                offset,
            )?;

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
            let blocks = grid.blocks().div_ceil(per_block);
            let cold = lanes.splat_bits::<U32, LANES>(blocks.div_ceil(per_block) * per_block)?;

            let which = lanes.shift_right_logical(slot, by)?;
            let both = lanes.mul(holding, inside)?;
            let waking = lanes.greater_than(both, none)?;
            lanes.select(waking, which, cold)?
        };
        kernel.store_at(HOT, coarse.id(), mark)?;
    }
    Ok(())
}

/// Lists every flagged entry of `tier` in the blocks `over` names, wherever
/// it lies. The engine's frames use [`mark_below`].
pub fn mark(subgroup: u32, tier: Tier, over: Over) -> Result<Vec<u32>, LaneError> {
    mark_below(subgroup, tier, over, u32::MAX)
}

/// Lists the flagged entries of `tier` in the blocks `over` names, and never
/// one at or past `below`, whatever its flag holds.
///
/// The coarse tier marks blocks, and its flags run on past the plan's blocks
/// to a whole tile. A block listed there would own quads in the scratch
/// `wake` writes, and `corners` wraps those onto real quads, which are then
/// stepped twice in one frame. The fine tier marks quads; `arranged` bounds
/// it by the quads of the grid, and `keeping`, which has no grid, by the
/// quads of the plan.
pub fn mark_below(
    subgroup: u32,
    tier: Tier,
    over: Over,
    below: u32,
) -> Result<Vec<u32>, LaneError> {
    match subgroup {
        4 => marked::<4>(subgroup, tier, over, below),
        8 => marked::<8>(subgroup, tier, over, below),
        16 => marked::<16>(subgroup, tier, over, below),
        32 => marked::<32>(subgroup, tier, over, below),
        64 => marked::<64>(subgroup, tier, over, below),
        other => Err(LaneError::NoMapping {
            lanes: other,
            width: subgroup,
        }),
    }
}

fn marked<const LANES: u32>(
    subgroup: u32,
    tier: Tier,
    over: Over,
    below: u32,
) -> Result<Vec<u32>, LaneError> {
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
            below,
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
    below: u32,
}

fn swept<const LANES: u32>(kernel: &mut Kernel<U32>, where_at: Sweeping) -> Result<(), LaneError> {
    use crate::sweep::PER_LANE;

    let Sweeping {
        mine,
        region,
        dump_mine,
        block,
        tier,
        below,
    } = where_at;
    let uint = kernel.index_type();

    let mut quads = Vec::with_capacity(PER_LANE as usize);
    let mut flags = Vec::with_capacity(PER_LANE as usize);
    for step in 0..PER_LANE {
        let along = kernel.module().constant_u32(step)?;
        let at = kernel.module().i_add(uint, mine, along)?;
        let held = kernel.load_at(tier.flags, at)?;
        let owned = {
            let mut lanes = kernel.lanes()?;
            let here = lanes.splat_id::<U32, LANES>(at)?;
            let bound = lanes.splat_bits::<U32, LANES>(below)?;
            let flag = lanes.splat_id::<U32, LANES>(held)?;
            let none = lanes.splat_bits::<U32, LANES>(0)?;
            let inside = lanes.greater_than(bound, here)?;
            lanes.select(inside, flag, none)?.id()
        };
        flags.push(owned);
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
