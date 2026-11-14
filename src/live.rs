use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, Lanes, U32, Vector};

use crate::compact::{Over, Tier, owned};
use crate::geometry::{Emit, Placed, Plane, Space};
use crate::layout::{AWAKE, BINDINGS, DENSE, Dimension, GRAND, HOT, PARAMS, STATE};
use crate::rule3::Rule;
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

/// The scans of the fine tier, from the counts the mark wrote to the bases
/// and the totals the gather reads, one stage each: one pass over one tile,
/// two levels while the tiles fit a block's quads, and past that a third
/// level, which scans the tiles' totals in groups into `TOTALS` past them and
/// then scans those.
pub fn fine_scans(subgroup: u32, plan: &crate::compact::Plan) -> Result<Stages, LaneError> {
    use crate::Groups;
    use crate::compact::Along;
    use crate::layout::{BASES, COUNTS, TOTALS};

    let one = Groups::Fixed(1);
    let tiles = Groups::Fixed(plan.tiles);
    Ok(match plan.levels() {
        1 => vec![vec![(
            crate::compact::whole(subgroup, COUNTS, BASES, Tier::FINE)?,
            one,
        )]],
        2 => vec![
            vec![(
                crate::compact::scan(subgroup, COUNTS, BASES, TOTALS)?,
                tiles,
            )],
            vec![(crate::compact::scan(subgroup, TOTALS, TOTALS, GRAND)?, one)],
        ],
        _ => vec![
            vec![(
                crate::compact::scan(subgroup, COUNTS, BASES, TOTALS)?,
                tiles,
            )],
            vec![(
                crate::compact::scan_at(
                    subgroup,
                    TOTALS,
                    TOTALS,
                    TOTALS,
                    Along {
                        from: 0,
                        aggregate: plan.upper(),
                    },
                )?,
                Groups::Fixed(plan.supers()),
            )],
            vec![(
                crate::compact::scan_at(
                    subgroup,
                    TOTALS,
                    TOTALS,
                    GRAND,
                    Along {
                        from: plan.upper(),
                        aggregate: 0,
                    },
                )?,
                one,
            )],
        ],
    })
}

/// The gather of the fine tier over the blocks `listed` names, reading the
/// third level's totals where the plan's scan has one.
pub fn fine_gather(
    subgroup: u32,
    plan: &crate::compact::Plan,
    listed: Over,
) -> Result<Vec<u32>, LaneError> {
    let upper = (plan.levels() > 2).then(|| plan.upper());
    crate::compact::gather_over(subgroup, Tier::FINE, listed, plan.dump(), upper)
}

/// The scans of the coarse tier: one pass while the tiles fit a block's
/// quads, and two levels past that, as the fine tier scans its tiles.
pub fn coarse_scans(subgroup: u32, plan: &crate::compact::Plan) -> Result<Stages, LaneError> {
    use crate::Groups;
    use crate::layout::{HOT_BASES, HOT_COUNTS, HOT_GRAND, HOT_TOTALS};

    let one = Groups::Fixed(1);
    Ok(if plan.hot_tiles <= 1 {
        vec![vec![(
            crate::compact::whole(subgroup, HOT_COUNTS, HOT_BASES, Tier::COARSE)?,
            one,
        )]]
    } else {
        vec![
            vec![(
                crate::compact::scan(subgroup, HOT_COUNTS, HOT_BASES, HOT_TOTALS)?,
                Groups::Fixed(plan.hot_tiles),
            )],
            vec![(
                crate::compact::scan(subgroup, HOT_TOTALS, HOT_TOTALS, HOT_GRAND)?,
                one,
            )],
        ]
    })
}

/// Zeroes the whole of `TOTALS`, a block's quads of words a group: a scan
/// three levels deep reads the padding past the tiles and past the groups
/// of tiles, and writes its prefixes over it.
pub fn wipe_totals(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    wipe_counts(subgroup, crate::layout::TOTALS)
}

/// The pass that zeroes `TOTALS` for a scan three levels deep, and the
/// groups it runs over; none for a shallower scan, which [`wipe_in`] serves.
pub fn totals_wiped(
    subgroup: u32,
    plan: &crate::compact::Plan,
) -> Result<Option<(Vec<u32>, crate::Groups)>, LaneError> {
    if plan.levels() < 3 {
        return Ok(None);
    }
    Ok(Some((
        wipe_totals(subgroup)?,
        crate::Groups::Fixed(plan.supers() + 1),
    )))
}

pub fn keeping(
    subgroup: u32,
    plan: &crate::compact::Plan,
    coarse: bool,
) -> Result<Keeping, LaneError> {
    use crate::Groups;
    use crate::layout::HOT_DENSE;
    use crate::sweep::COUNTS;

    let (listed, hot) = if coarse {
        (Over::Listed(HOT_DENSE), crate::compact::Plan::hot_listed())
    } else {
        (Over::Every, Groups::Fixed(plan.blocks))
    };
    let tiles = Groups::Fixed(plan.tiles);
    let one = Groups::Fixed(1);

    let mut first = vec![
        (decay(subgroup, listed)?, hot),
        (wipe_in(subgroup, plan.dimension)?, one),
        (wipe_counts(subgroup, COUNTS)?, tiles),
        (unclaimed(subgroup, plan)?, one),
    ];
    first.extend(totals_wiped(subgroup, plan)?);
    let before = vec![first];

    let mut after: Stages = Vec::new();
    if coarse {
        after.push(vec![(
            mark_below(subgroup, Tier::COARSE, Over::Every, plan.blocks)?,
            tiles,
        )]);
        after.extend(coarse_scans(subgroup, plan)?);
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
    after.extend(fine_scans(subgroup, plan)?);
    after.push(vec![
        (fine_gather(subgroup, plan, listed)?, hot),
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
    use crate::layout::HOT_DENSE;
    use crate::sweep::COUNTS;

    sand::flat("live::arranged", grid)?;
    if plan.windows().is_split() {
        return Err(LaneError::NoSuchForm {
            operation: "live::arranged",
            because: SPLIT_FLAT,
        });
    }

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

    let mut first = vec![
        (tick(subgroup)?, one),
        (decay(subgroup, listed)?, hot),
        (wipe(subgroup)?, one),
        (wipe_counts(subgroup, COUNTS)?, tiles),
        (unclaimed(subgroup, plan)?, one),
    ];
    first.extend(totals_wiped(subgroup, plan)?);
    let mut stages: Stages = vec![
        first,
        vec![(sand::brush(subgroup, grid, true)?, sand::seats(plan))],
        vec![(settle(subgroup, grid)?, crate::compact::Plan::listed())],
        looking,
    ];
    if coarse {
        stages.extend(coarse_scans(subgroup, plan)?);
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
        mark_below(subgroup, Tier::FINE, listed, grid.quads())?,
        hot,
    )]);
    stages.extend(fine_scans(subgroup, plan)?);
    stages.push(vec![
        (fine_gather(subgroup, plan, listed)?, hot),
        (
            crate::compact::prepare(subgroup, subgroup, Tier::FINE)?,
            Groups::Counting(1),
        ),
    ]);

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

/// Why a flat frame refuses a plan whose `STATE` is split: its brush and
/// its picture read `STATE` as one binding.
pub const SPLIT_FLAT: &str =
    "the plan splits STATE over bindings, and a flat frame's brush and picture read one";

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

pub const BRICKED_PLAN: &str =
    "the plan is laid out for a brick pool, whose STATE does not hold the grid";

pub(crate) fn fits(
    operation: &'static str,
    grid: Grid,
    plan: &crate::compact::Plan,
) -> Result<(), LaneError> {
    if plan.is_bricked() {
        return Err(LaneError::NoSuchForm {
            operation,
            because: BRICKED_PLAN,
        });
    }
    fits_pool(operation, grid, plan)
}

pub(crate) fn fits_pool(
    operation: &'static str,
    grid: Grid,
    plan: &crate::compact::Plan,
) -> Result<(), LaneError> {
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
    wired3_by(&crate::rule3::sand::Sand, subgroup, grid, plan)
}

/// [`arranged3_by`] on the tier the plan's size pays for, as [`wired`] picks
/// it.
pub fn wired3_by<R: Rule>(
    rule: &R,
    subgroup: u32,
    grid: Grid,
    plan: &crate::compact::Plan,
) -> Result<Loop, LaneError> {
    fits("live::wired3", grid, plan)?;
    arranged3_by(rule, subgroup, grid, plan, coarse_pays(plan.blocks))
}

/// The sizes of the bindings of a listed engine on `plan` that steps
/// `rule`: [`crate::compact::Plan::sizes`] with `PARAMS` grown by the
/// rule's parameters, which start at [`crate::rule3::FIRST_PARAM`], and
/// `TALLY` grown to the rule's [`crate::rule3::Tally::words`].
#[must_use]
pub fn sizes3<R: Rule>(rule: &R, plan: &crate::compact::Plan) -> [usize; BINDINGS as usize] {
    use crate::layout::TALLY;

    let mut sizes = plan.sizes();
    if let Some(state) = sizes.get_mut(STATE as usize) {
        *state = state3(rule, plan);
    }
    if let Some(params) = sizes.get_mut(PARAMS as usize) {
        *params = (*params).max(crate::rule3::FIRST_PARAM) + rule.params();
    }
    if let Some(tally) = sizes.get_mut(TALLY as usize) {
        *tally = (*tally).max(rule.tally().words() * TALLY_SPAN);
    }
    sizes
}

#[must_use]
pub fn state3<R: Rule>(rule: &R, plan: &crate::compact::Plan) -> usize {
    plan.state_words() * rule.planes().max(1)
}

#[must_use]
pub fn windows3<R: Rule>(rule: &R, plan: &crate::compact::Plan) -> crate::layout::Windows {
    if rule.planes() <= 1 {
        return plan.windows();
    }
    crate::layout::Windows::at(state3(rule, plan), plan.windows().bits())
}

/// [`arranged3_by`] under the sand and water rule.
pub fn arranged3(
    subgroup: u32,
    grid: Grid,
    plan: &crate::compact::Plan,
    coarse: bool,
) -> Result<Loop, LaneError> {
    arranged3_by(&crate::rule3::sand::Sand, subgroup, grid, plan, coarse)
}

/// The frame of the listed engine on a solid grid, the 3D counterpart of
/// [`arranged`] without the brush and the picture: tick, decay and wipe,
/// [`settle3_by`] under `rule` over the listed octs, then the marks, scans
/// and gathers that list the octs of the next frame. `plan` is laid out for
/// [`Dimension::Three`] and holds every oct of `grid`.
pub fn arranged3_by<R: Rule>(
    rule: &R,
    subgroup: u32,
    grid: Grid,
    plan: &crate::compact::Plan,
    coarse: bool,
) -> Result<Loop, LaneError> {
    fits("live::arranged3", grid, plan)?;
    let (listed, _) = tiers(plan, coarse);
    arranged3_with(
        rule,
        subgroup,
        grid,
        plan,
        (coarse, decay(subgroup, listed)?),
    )
}

pub fn wired3_switched_by<R: Rule>(
    rule: &R,
    subgroup: u32,
    grid: Grid,
    plan: &crate::compact::Plan,
    switch: u32,
) -> Result<Loop, LaneError> {
    fits("live::wired3", grid, plan)?;
    let coarse = coarse_pays(plan.blocks);
    let (listed, _) = tiers(plan, coarse);
    let kept = decay_switched(subgroup, listed, grid.blocks(), switch)?;
    arranged3_with(rule, subgroup, grid, plan, (coarse, kept))
}

fn arranged3_with<R: Rule>(
    rule: &R,
    subgroup: u32,
    grid: Grid,
    plan: &crate::compact::Plan,
    (coarse, decayed): (bool, Vec<u32>),
) -> Result<Loop, LaneError> {
    let mut stages: Stages = vec![
        opening3(rule, subgroup, plan, coarse, decayed)?,
        vec![(
            settle3_in(rule, subgroup, grid, windows3(rule, plan))?,
            crate::compact::Plan::listed(),
        )],
    ];
    stages.extend(closing3(subgroup, grid, plan, coarse)?);
    Ok(staged(stages))
}

pub(crate) fn tiers(plan: &crate::compact::Plan, coarse: bool) -> (Over, crate::Groups) {
    use crate::layout::HOT_DENSE;

    if coarse {
        (Over::Listed(HOT_DENSE), crate::compact::Plan::hot_listed())
    } else {
        (Over::Every, crate::Groups::Fixed(plan.blocks))
    }
}

pub(crate) fn opening3<R: Rule>(
    rule: &R,
    subgroup: u32,
    plan: &crate::compact::Plan,
    coarse: bool,
    decayed: Vec<u32>,
) -> Result<Vec<(Vec<u32>, crate::Groups)>, LaneError> {
    use crate::Groups;
    use crate::sweep::COUNTS;

    let (_, hot) = tiers(plan, coarse);
    let tiles = Groups::Fixed(plan.tiles);
    let one = Groups::Fixed(1);

    let mut first = vec![
        (tick(subgroup)?, one),
        (decayed, hot),
        (wipe3(subgroup, rule.tally().words())?, one),
        (wipe_counts(subgroup, COUNTS)?, tiles),
        (unclaimed(subgroup, plan)?, one),
    ];
    first.extend(totals_wiped(subgroup, plan)?);
    Ok(first)
}

pub(crate) fn closing3(
    subgroup: u32,
    grid: Grid,
    plan: &crate::compact::Plan,
    coarse: bool,
) -> Result<Stages, LaneError> {
    use crate::Groups;

    let (listed, hot) = tiers(plan, coarse);
    let tiles = Groups::Fixed(plan.tiles);
    let mut stages: Stages = Vec::new();
    if coarse {
        stages.push(vec![(
            mark_below(subgroup, Tier::COARSE, Over::Every, plan.blocks)?,
            tiles,
        )]);
        stages.extend(coarse_scans(subgroup, plan)?);
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
    stages.extend(fine_scans(subgroup, plan)?);
    stages.push(vec![
        (fine_gather(subgroup, plan, listed)?, hot),
        (
            crate::compact::prepare(subgroup, subgroup, Tier::FINE)?,
            Groups::Counting(1),
        ),
    ]);
    Ok(stages)
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
    put3_in(held, grid, plan, Placed3::FIRST, cells)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placed3 {
    pub windows: Option<crate::layout::Windows>,
    pub plane: u32,
}

impl Placed3 {
    pub const FIRST: Self = Self {
        windows: None,
        plane: 0,
    };
}

pub fn put3_in(
    held: &mut crate::Resident<'_>,
    grid: Grid,
    plan: &crate::compact::Plan,
    placed: Placed3,
    cells: &[(u32, u32)],
) -> Result<(), crate::Error> {
    use crate::geometry::Solid;
    use crate::layout::INDIRECT;

    fits("live::put3", grid, plan).map_err(crate::Error::Lane)?;
    let past = grid.cells();
    let windows = placed.windows.unwrap_or_else(|| plan.windows());
    let lifted = placed.plane as usize * past;
    if lifted + past > windows.words() {
        return Err(crate::Error::Overrun {
            index: STATE as usize,
            needed: lifted + past,
            held: windows.words(),
        });
    }
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
        write_state(held, windows, lifted + cell as usize, &[word])?;
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

/// Writes `words` into `STATE` laid out as `windows`, from word `at`: each
/// run within one window is one write of that window's binding.
pub fn write_state(
    held: &mut crate::Resident<'_>,
    windows: crate::layout::Windows,
    at: usize,
    words: &[u32],
) -> Result<(), crate::Error> {
    if !windows.is_split() {
        return held.write_at(STATE as usize, at, words);
    }
    let mut done = 0_usize;
    while done < words.len() {
        let index = at + done;
        let (binding, offset) = windows.place(index);
        let room = windows.span() - offset;
        let run = room.min(words.len() - done);
        let part = words.get(done..done + run).unwrap_or_default();
        held.write_at(binding as usize, offset, part)?;
        done += run;
    }
    Ok(())
}

/// The first `count` words of `STATE` laid out as `windows`, read window by
/// window.
pub fn read_state(
    held: &mut crate::Resident<'_>,
    windows: crate::layout::Windows,
    count: usize,
) -> Result<Vec<u32>, crate::Error> {
    if !windows.is_split() {
        return held.read(STATE as usize, count);
    }
    let mut out = Vec::with_capacity(count);
    for window in 0..windows.count() {
        if out.len() >= count {
            break;
        }
        let wanted = windows.held(window).min(count - out.len());
        let binding = crate::layout::Windows::binding(window);
        out.extend(held.read(binding as usize, wanted)?);
    }
    if out.len() < count {
        return Err(crate::Error::Overrun {
            index: STATE as usize,
            needed: count,
            held: windows.words(),
        });
    }
    Ok(out)
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

    held.write(
        TALLY as usize,
        &vec![0; plan.dimension.tally_words() * TALLY_SPAN],
    )
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

pub fn decay_switched(
    subgroup: u32,
    over: Over,
    quads: u32,
    switch: u32,
) -> Result<Vec<u32>, LaneError> {
    match subgroup {
        4 => decayed_switched::<4>(subgroup, over, quads, switch),
        8 => decayed_switched::<8>(subgroup, over, quads, switch),
        16 => decayed_switched::<16>(subgroup, over, quads, switch),
        32 => decayed_switched::<32>(subgroup, over, quads, switch),
        64 => decayed_switched::<64>(subgroup, over, quads, switch),
        other => Err(LaneError::NoMapping {
            lanes: other,
            width: subgroup,
        }),
    }
}

fn decayed_switched<const LANES: u32>(
    subgroup: u32,
    over: Over,
    quads: u32,
    switch: u32,
) -> Result<Vec<u32>, LaneError> {
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
    let switch_at = kernel.module().constant_u32(switch)?;
    let whole = kernel.load_at(PARAMS, switch_at)?;

    let kept = |kernel: &mut Kernel<U32>, held: simdr::module::Id, quad: simdr::module::Id| {
        let mut lanes = kernel.lanes()?;
        let on = lanes.splat_id::<U32, 1>(whole)?;
        let none = lanes.splat_bits::<U32, 1>(0)?;
        let now = lanes.splat_id::<U32, 1>(held)?;
        let at = lanes.splat_id::<U32, 1>(quad)?;
        let one = lanes.splat_bits::<U32, 1>(1)?;
        let minus = lanes.splat_bits::<U32, 1>(u32::MAX)?;
        let past = lanes.splat_bits::<U32, 1>(quads)?;
        let floored = lanes.max(now, one)?;
        let back = lanes.mul(one, minus)?;
        let lowered = lanes.add(floored, back)?;
        let inside = lanes.greater_than(past, at)?;
        let kept = lanes.select(inside, one, lowered)?;
        let on = lanes.greater_than(on, none)?;
        Ok::<_, LaneError>(lanes.select(on, kept, lowered)?.id())
    };

    let mut quad_ids = Vec::with_capacity(PER_LANE as usize);
    let mut held = Vec::with_capacity(PER_LANE as usize);
    for offset in 0..PER_LANE {
        let along = kernel.module().constant_u32(offset)?;
        let quad = kernel.module().i_add(uint, mine, along)?;
        held.push(kernel.load_at(AWAKE, quad)?);
        quad_ids.push(quad);
    }
    let mut lowered = Vec::with_capacity(PER_LANE as usize);
    for (&was, &quad) in held.iter().zip(&quad_ids) {
        lowered.push(kept(&mut kernel, was, quad)?);
    }
    for (&quad, &lower) in quad_ids.iter().zip(&lowered) {
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

    for word in 0..dimension.tally_words() * TALLY_SPAN {
        let at = kernel
            .module()
            .constant_u32(u32::try_from(word).unwrap_or(0))?;
        kernel.store_at(TALLY, at, nothing)?;
    }
    kernel.finish()
}

/// [`wipe_in`] for a solid grid whose rule's tally takes `words` words:
/// `TOTALS` and `HOT_TOTALS` as [`wipe_in`] zeroes them, and the first
/// `words` words of `TALLY`.
pub fn wipe3(subgroup: u32, words: usize) -> Result<Vec<u32>, LaneError> {
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

    for word in 0..words * TALLY_SPAN {
        let at = kernel
            .module()
            .constant_u32(u32::try_from(word).unwrap_or(0))?;
        kernel.store_at(TALLY, at, nothing)?;
    }
    kernel.finish()
}

pub fn wipe_counts(subgroup: u32, counts: u32) -> Result<Vec<u32>, LaneError> {
    use crate::sweep::{self, PER_LANE};

    let reach = crate::compact::reaching(&[counts]);
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, reach))?;
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

/// The listed settle of a solid grid under the sand and water rule:
/// [`settle3_by`] with [`crate::rule3::sand::Sand`].
pub fn settle3(subgroup: u32, grid: Grid) -> Result<Vec<u32>, LaneError> {
    settle3_by(&crate::rule3::sand::Sand, subgroup, grid)
}

/// The listed settle of a solid grid: each listed entry is one oct, stepped
/// by `rule` with 8 loads and 8 stores of `STATE` through
/// [`crate::geometry::Solid`]. An oct that changes wakes itself and the 26
/// octs around it ([`crate::geometry::Space::near`]), and the rule's counts
/// go to the [`Dimension::Three`] tally. Only the lanes inside the list
/// store, so `STATE` holds the grid and nothing past it.
pub fn settle3_by<R: Rule>(rule: &R, subgroup: u32, grid: Grid) -> Result<Vec<u32>, LaneError> {
    settle3_in(
        rule,
        subgroup,
        grid,
        crate::layout::Windows::of(grid.cells()),
    )
}

/// [`settle3_by`] on a `STATE` laid out as `windows`.
pub fn settle3_in<R: Rule>(
    rule: &R,
    subgroup: u32,
    grid: Grid,
    windows: crate::layout::Windows,
) -> Result<Vec<u32>, LaneError> {
    solid("live::settle3", grid)?;
    match subgroup {
        4 => listed3::<4, R>(rule, subgroup, grid, windows),
        8 => listed3::<8, R>(rule, subgroup, grid, windows),
        16 => listed3::<16, R>(rule, subgroup, grid, windows),
        32 => listed3::<32, R>(rule, subgroup, grid, windows),
        64 => listed3::<64, R>(rule, subgroup, grid, windows),
        other => Err(LaneError::NoMapping {
            lanes: other,
            width: subgroup,
        }),
    }
}

/// The window word `at` of `STATE` lies in, and where in it.
fn window_of(
    kernel: &mut Kernel<U32>,
    windows: crate::layout::Windows,
    at: simdr::module::Id,
) -> Result<(simdr::module::Id, simdr::module::Id), LaneError> {
    use simdr::module::op;

    let uint = kernel.index_type();
    let by = kernel.module().constant_u32(windows.bits())?;
    let mask = kernel
        .module()
        .constant_u32(u32::try_from(windows.span() - 1).unwrap_or(u32::MAX))?;
    let window = kernel
        .module()
        .binary(op::SHIFT_RIGHT_LOGICAL, uint, at, by)?;
    let offset = kernel.module().binary(op::BITWISE_AND, uint, at, mask)?;
    Ok((window, offset))
}

/// Runs `body` in the lanes where `when`, a bool, holds, and in no other.
pub fn lane_if<F>(
    kernel: &mut Kernel<U32>,
    when: simdr::module::Id,
    body: F,
) -> Result<(), LaneError>
where
    F: FnOnce(&mut Kernel<U32>) -> Result<(), LaneError>,
{
    use simdr::spec::SelectionControl;

    let taken = kernel.module().alloc_id()?;
    let merge = kernel.module().alloc_id()?;
    kernel
        .module()
        .selection_merge(merge, SelectionControl::None)?;
    kernel.module().branch_conditional(when, taken, merge)?;
    kernel.module().label_at(taken)?;
    body(kernel)?;
    kernel.module().branch(merge)?;
    kernel.module().label_at(merge)?;
    Ok(())
}

/// Whether `window` is `which`, as a bool.
fn in_window(
    kernel: &mut Kernel<U32>,
    window: simdr::module::Id,
    which: u32,
) -> Result<simdr::module::Id, LaneError> {
    use simdr::module::op;

    let truth = kernel.module().type_bool()?;
    let named = kernel.module().constant_u32(which)?;
    Ok(kernel.module().binary(op::I_EQUAL, truth, window, named)?)
}

/// Word `at` of `STATE` laid out as `windows`: one load of `STATE` when it is
/// one binding, and otherwise one load of the window that holds it.
pub fn state_load(
    kernel: &mut Kernel<U32>,
    windows: crate::layout::Windows,
    at: simdr::module::Id,
) -> Result<simdr::module::Id, LaneError> {
    use simdr::spec::SelectionControl;

    if !windows.is_split() {
        return kernel.load_at(STATE, at);
    }
    let uint = kernel.index_type();
    let (window, offset) = window_of(kernel, windows, at)?;
    let mut held = kernel.module().constant_u32(0)?;
    for which in 0..windows.count() {
        let here = in_window(kernel, window, which)?;
        let before = kernel
            .module()
            .current_block()
            .ok_or(LaneError::NoOpenBlock { arm: "before" })?;
        let taken = kernel.module().alloc_id()?;
        let merge = kernel.module().alloc_id()?;
        kernel
            .module()
            .selection_merge(merge, SelectionControl::None)?;
        kernel.module().branch_conditional(here, taken, merge)?;
        kernel.module().label_at(taken)?;
        let loaded = kernel.load_at(crate::layout::Windows::binding(which), offset)?;
        let finished = kernel
            .module()
            .current_block()
            .ok_or(LaneError::NoOpenBlock { arm: "then" })?;
        kernel.module().branch(merge)?;
        kernel.module().label_at(merge)?;
        held = kernel
            .module()
            .phi(uint, &[(loaded, finished), (held, before)])?;
    }
    Ok(held)
}

/// Stores `value` at word `at` of `STATE` laid out as `windows`, in the one
/// window that holds it.
pub fn state_store(
    kernel: &mut Kernel<U32>,
    windows: crate::layout::Windows,
    at: simdr::module::Id,
    value: simdr::module::Id,
) -> Result<(), LaneError> {
    if !windows.is_split() {
        return kernel.store_at(STATE, at, value);
    }
    let (window, offset) = window_of(kernel, windows, at)?;
    for which in 0..windows.count() {
        let here = in_window(kernel, window, which)?;
        lane_if(kernel, here, |kernel| {
            kernel.store_at(crate::layout::Windows::binding(which), offset, value)
        })?;
    }
    Ok(())
}

fn listed3<const LANES: u32, R: Rule>(
    rule: &R,
    subgroup: u32,
    grid: Grid,
    windows: crate::layout::Windows,
) -> Result<Vec<u32>, LaneError> {
    use crate::sand3::{CORNERS, Given, frame_and_params, placed, ruled};

    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, windows.bindings()))?;
    let uint = kernel.index_type();

    let group = kernel.workgroup_index();
    let lane = kernel.local_index();
    let width = kernel.module().constant_u32(subgroup)?;
    let base = kernel.module().i_mul(uint, group, width)?;
    let slot = kernel.module().i_add(uint, base, lane)?;

    let (frame, params) = frame_and_params(&mut kernel, rule)?;
    let head = kernel.module().constant_u32(0)?;
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

    let planes = rule.planes();
    let (places, settled, counted, touched) = if planes > 1 {
        let stride = crate::sand3::stride_of(rule, grid)?;
        let mut places = vec![placing.places];
        for plane in 1..planes {
            let mut lanes = kernel.lanes()?;
            let by = u32::try_from(plane).unwrap_or(0).wrapping_mul(stride);
            let by = lanes.splat_bits::<U32, LANES>(by)?;
            let mut lifted = placing.places;
            for place in &mut lifted {
                *place = lanes.add(*place, by)?;
            }
            places.push(lifted);
        }
        let mut loaded = Vec::with_capacity(planes);
        for plane in &places {
            let mut words = [frame; CORNERS];
            for (word, place) in words.iter_mut().zip(plane) {
                *word = state_load(&mut kernel, windows, place.id())?;
            }
            loaded.push(words);
        }
        let mut lanes = kernel.lanes()?;
        let given = Given {
            frame,
            params: &params,
        };
        let step =
            crate::sand3::ruled_planes::<LANES, R>(&mut lanes, rule, &placing, &loaded, given)?;
        let one = lanes.splat_bits::<U32, LANES>(1)?;
        let none = lanes.splat_bits::<U32, LANES>(0)?;
        let mut settled = vec![step.ruled.out];
        settled.extend(step.planes);
        let mut touched = none;
        for (before, after) in loaded.iter().flatten().zip(settled.iter().flatten()) {
            let before = lanes.splat_id::<U32, LANES>(*before)?;
            let same = lanes.equal(before, *after)?;
            let differs = lanes.select(same, none, one)?;
            touched = lanes.add(touched, differs)?;
        }
        (places, settled, step.ruled.counts, touched)
    } else {
        let mut loaded = [frame; CORNERS];
        for (word, place) in loaded.iter_mut().zip(&placing.places) {
            *word = state_load(&mut kernel, windows, place.id())?;
        }

        let mut lanes = kernel.lanes()?;
        let given = Given {
            frame,
            params: &params,
        };
        let step = ruled::<LANES, R>(&mut lanes, rule, &placing, loaded, given)?;
        let one = lanes.splat_bits::<U32, LANES>(1)?;
        let none = lanes.splat_bits::<U32, LANES>(0)?;

        let mut touched = none;
        for (before, after) in loaded.into_iter().zip(step.out) {
            let before = lanes.splat_id::<U32, LANES>(before)?;
            let same = lanes.equal(before, after)?;
            let differs = lanes.select(same, none, one)?;
            touched = lanes.add(touched, differs)?;
        }
        (vec![placing.places], vec![step.out], step.counts, touched)
    };

    let listed = {
        let mut lanes = kernel.lanes()?;
        let none = lanes.splat_bits::<U32, LANES>(0)?;
        let listed = lanes.greater_than(inside, none)?;
        listed
            .strips()
            .first()
            .copied()
            .ok_or(LaneError::NoOpenBlock { arm: "listed" })?
    };
    lane_if(&mut kernel, listed, |kernel| {
        for (plane, values) in places.iter().zip(&settled) {
            for (place, value) in plane.iter().zip(values) {
                state_store(kernel, windows, place.id(), value.id())?;
            }
        }
        Ok(())
    })?;

    let block = Block::<LANES, 3> {
        subgroup,
        grid,
        at: placing.block,
    };
    woken::<LANES, 3, 8, 27>(&mut kernel, block, [touched, inside], dump_base)?;

    let what = Tallied3 {
        counted: &counted,
        at: placing.block,
        inside: counted_in,
        weighted: rule.tally().weighted,
    };
    tally3::<LANES>(&mut kernel, what)?;
    kernel.finish()
}

/// What a 3D frame adds to `TALLY`, as [`tally3`] takes it.
pub(crate) struct Tallied3<'held, const LANES: u32> {
    pub counted: &'held [Vector<U32, LANES>],
    pub at: [Vector<U32, LANES>; 3],
    pub inside: Vector<U32, LANES>,
    pub weighted: bool,
}

/// Adds a 3D frame's counts to `TALLY`, as [`crate::rule3::Tally`] lays
/// them out: each of the rule's counts from the lanes inside the list, and
/// when weighted, the first count weighted by each axis of its oct shifted
/// down by [`COARSE`](crate::layout::COARSE), and the octs whose first
/// count is not zero.
pub(crate) fn tally3<const LANES: u32>(
    kernel: &mut Kernel<U32>,
    what: Tallied3<'_, LANES>,
) -> Result<(), LaneError> {
    use crate::layout::COARSE;

    let Tallied3 {
        counted,
        at,
        inside,
        weighted,
    } = what;

    let shares = {
        let mut lanes = kernel.lanes()?;
        let none = lanes.splat_bits::<U32, LANES>(0)?;
        let one = lanes.splat_bits::<U32, LANES>(1)?;
        let listed = lanes.greater_than(inside, none)?;

        let mut shares = Vec::with_capacity(counted.len() + 4);
        for &count in counted {
            shares.push((lanes.select(listed, count, none)?, None));
        }
        if weighted && let Some(&(moved, _)) = shares.first() {
            let by = lanes.splat_bits::<U32, LANES>(COARSE)?;
            let mut weighted = Vec::with_capacity(3);
            for value in at {
                let value = lanes.shift_right_logical(value, by)?;
                let (low, high) = wide_mul::<LANES>(&mut lanes, value, moved)?;
                weighted.push((low, Some(high)));
            }
            let stirred = lanes.greater_than(moved, none)?;
            let stirred = lanes.select(stirred, one, none)?;
            shares.extend(weighted);
            shares.push((stirred, None));
        }
        shares
    };
    tally_wide::<LANES>(kernel, &shares)
}

pub const TALLY_SPAN: usize = 3;

pub fn read_tally(held: &mut crate::Resident<'_>, words: usize) -> Result<Vec<u128>, crate::Error> {
    use crate::layout::TALLY;

    let raw = held.read(TALLY as usize, words * TALLY_SPAN)?;
    Ok((0..words)
        .map(|word| {
            let part = |at: usize| u128::from(raw.get(at).copied().unwrap_or(0));
            part(word) | part(words + word) << 32 | part(2 * words + word) << 64
        })
        .collect())
}

type Lane<const LANES: u32> = Vector<U32, LANES>;

fn wide_mul<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    left: Lane<LANES>,
    right: Lane<LANES>,
) -> Result<(Lane<LANES>, Lane<LANES>), LaneError> {
    let mask = lanes.splat_bits::<U32, LANES>(0xFFFF)?;
    let sixteen = lanes.splat_bits::<U32, LANES>(16)?;
    let one = lanes.splat_bits::<U32, LANES>(1)?;
    let none = lanes.splat_bits::<U32, LANES>(0)?;
    let left_low = lanes.and(left, mask)?;
    let left_high = lanes.shift_right_logical(left, sixteen)?;
    let right_low = lanes.and(right, mask)?;
    let right_high = lanes.shift_right_logical(right, sixteen)?;
    let lowest = lanes.mul(left_low, right_low)?;
    let across = lanes.mul(left_low, right_high)?;
    let down = lanes.mul(left_high, right_low)?;
    let highest = lanes.mul(left_high, right_high)?;
    let middle = lanes.add(across, down)?;
    let middle_over = lanes.less_than(middle, across)?;
    let middle_over = lanes.select(middle_over, one, none)?;
    let lifted = lanes.shift_left(middle, sixteen)?;
    let low = lanes.add(lowest, lifted)?;
    let low_over = lanes.less_than(low, lowest)?;
    let low_over = lanes.select(low_over, one, none)?;
    let dropped = lanes.shift_right_logical(middle, sixteen)?;
    let carried = lanes.shift_left(middle_over, sixteen)?;
    let high = lanes.add(highest, dropped)?;
    let high = lanes.add(high, carried)?;
    let high = lanes.add(high, low_over)?;
    Ok((low, high))
}

fn over<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    sum: Lane<LANES>,
    part: Lane<LANES>,
) -> Result<Lane<LANES>, LaneError> {
    let one = lanes.splat_bits::<U32, LANES>(1)?;
    let none = lanes.splat_bits::<U32, LANES>(0)?;
    let wrapped = lanes.less_than(sum, part)?;
    lanes.select(wrapped, one, none)
}

fn tally_wide<const LANES: u32>(
    kernel: &mut Kernel<U32>,
    shares: &[(Lane<LANES>, Option<Lane<LANES>>)],
) -> Result<(), LaneError> {
    let first = kernel.local_index();
    let (leading, small, large) = {
        let mut lanes = kernel.lanes()?;
        let none = lanes.splat_bits::<U32, LANES>(0)?;
        let seat = lanes.splat_id::<U32, LANES>(first)?;
        let leading = lanes.equal(seat, none)?;
        let leading = leading
            .strips()
            .first()
            .copied()
            .ok_or(LaneError::NoOpenBlock { arm: "tally" })?;
        let by = lanes.splat_bits::<U32, LANES>(26)?;
        let mut large = none;
        for &(low, high) in shares {
            let top = lanes.shift_right_logical(low, by)?;
            large = lanes.or(large, top)?;
            if let Some(high) = high {
                large = lanes.or(large, high)?;
            }
        }
        let any = lanes.reduce_or::<U32, LANES>(large)?;
        let any = lanes.splat_id::<U32, LANES>(any)?;
        let small = lanes.equal(any, none)?;
        let large = lanes.not_equal(any, none)?;
        let strip = |truth: simdr::lanes::Predicate<LANES>| {
            truth
                .strips()
                .first()
                .copied()
                .ok_or(LaneError::NoOpenBlock { arm: "tally" })
        };
        (leading, strip(small)?, strip(large)?)
    };
    lane_if(kernel, small, |kernel| {
        tally_small::<LANES>(kernel, shares, leading)
    })?;
    lane_if(kernel, large, |kernel| {
        tally_exact::<LANES>(kernel, shares, leading)
    })
}

fn tally_small<const LANES: u32>(
    kernel: &mut Kernel<U32>,
    shares: &[(Lane<LANES>, Option<Lane<LANES>>)],
    leading: simdr::module::Id,
) -> Result<(), LaneError> {
    use crate::layout::TALLY;

    let words = shares.len();
    let mut sums = Vec::with_capacity(words);
    for &(low, _) in shares {
        let mut lanes = kernel.lanes()?;
        let sum = lanes.reduce_sum::<U32, LANES>(low)?;
        sums.push(lanes.splat_id::<U32, LANES>(sum)?);
    }
    lane_if(kernel, leading, |kernel| {
        for (word, &sum) in sums.iter().enumerate() {
            let at = kernel
                .module()
                .constant_u32(u32::try_from(word).unwrap_or(0))?;
            let was = kernel.atomic_add_at(TALLY, at, sum.id())?;
            let carried = {
                let mut lanes = kernel.lanes()?;
                let was = lanes.splat_id::<U32, LANES>(was)?;
                let total = lanes.add(was, sum)?;
                let wrapped = lanes.less_than(total, was)?;
                wrapped
                    .strips()
                    .first()
                    .copied()
                    .ok_or(LaneError::NoOpenBlock { arm: "tally" })?
            };
            lane_if(kernel, carried, |kernel| {
                let at = kernel
                    .module()
                    .constant_u32(u32::try_from(words + word).unwrap_or(0))?;
                let one = kernel.module().constant_u32(1)?;
                let was = kernel.atomic_add_at(TALLY, at, one)?;
                let topped = {
                    let mut lanes = kernel.lanes()?;
                    let was = lanes.splat_id::<U32, LANES>(was)?;
                    let full = lanes.splat_bits::<U32, LANES>(u32::MAX)?;
                    let topped = lanes.equal(was, full)?;
                    topped
                        .strips()
                        .first()
                        .copied()
                        .ok_or(LaneError::NoOpenBlock { arm: "tally" })?
                };
                lane_if(kernel, topped, |kernel| {
                    let at = kernel
                        .module()
                        .constant_u32(u32::try_from(2 * words + word).unwrap_or(0))?;
                    let one = kernel.module().constant_u32(1)?;
                    let _ = kernel.atomic_add_at(TALLY, at, one)?;
                    Ok(())
                })
            })?;
        }
        Ok(())
    })
}

fn tally_exact<const LANES: u32>(
    kernel: &mut Kernel<U32>,
    shares: &[(Lane<LANES>, Option<Lane<LANES>>)],
    leading: simdr::module::Id,
) -> Result<(), LaneError> {
    use crate::layout::TALLY;

    let words = shares.len();
    let mut totals = Vec::with_capacity(words);
    for &(low, high) in shares {
        let halves = {
            let mut lanes = kernel.lanes()?;
            let mask = lanes.splat_bits::<U32, LANES>(0xFFFF)?;
            let sixteen = lanes.splat_bits::<U32, LANES>(16)?;
            let mut halves = vec![
                lanes.and(low, mask)?,
                lanes.shift_right_logical(low, sixteen)?,
            ];
            if let Some(high) = high {
                halves.push(lanes.and(high, mask)?);
                halves.push(lanes.shift_right_logical(high, sixteen)?);
            }
            halves
        };
        let mut sums = Vec::with_capacity(halves.len());
        for half in halves {
            let mut lanes = kernel.lanes()?;
            let sum = lanes.reduce_sum::<U32, LANES>(half)?;
            sums.push(lanes.splat_id::<U32, LANES>(sum)?);
        }
        totals.push(sums);
    }
    lane_if(kernel, leading, |kernel| {
        for (word, sums) in totals.iter().enumerate() {
            let [bottom, second, rest @ ..] = sums.as_slice() else {
                continue;
            };
            let (low, middle, top) = {
                let mut lanes = kernel.lanes()?;
                let none = lanes.splat_bits::<U32, LANES>(0)?;
                let sixteen = lanes.splat_bits::<U32, LANES>(16)?;
                let mask = lanes.splat_bits::<U32, LANES>(0xFFFF)?;
                let lifted = lanes.shift_left(*second, sixteen)?;
                let low = lanes.add(*bottom, lifted)?;
                let carry = over::<LANES>(&mut lanes, low, lifted)?;
                let middle = lanes.shift_right_logical(*second, sixteen)?;
                let (third, fourth) = match rest {
                    [third, fourth] => (*third, *fourth),
                    _ => (none, none),
                };
                let middle = lanes.add(middle, third)?;
                let fourth_low = lanes.and(fourth, mask)?;
                let fourth_low = lanes.shift_left(fourth_low, sixteen)?;
                let grown = lanes.add(middle, fourth_low)?;
                let first_over = over::<LANES>(&mut lanes, grown, fourth_low)?;
                let middle = lanes.add(grown, carry)?;
                let second_over = over::<LANES>(&mut lanes, middle, grown)?;
                let top = lanes.shift_right_logical(fourth, sixteen)?;
                let top = lanes.add(top, first_over)?;
                let top = lanes.add(top, second_over)?;
                (low, middle, top)
            };
            let at = |kernel: &mut Kernel<U32>, part: usize| {
                kernel
                    .module()
                    .constant_u32(u32::try_from(part * words + word).unwrap_or(0))
            };
            let low_at = at(kernel, 0)?;
            let was = kernel.atomic_add_at(TALLY, low_at, low.id())?;
            let middle = {
                let mut lanes = kernel.lanes()?;
                let was = lanes.splat_id::<U32, LANES>(was)?;
                let sum = lanes.add(was, low)?;
                let carry = over::<LANES>(&mut lanes, sum, was)?;
                let added = lanes.add(middle, carry)?;
                let passed = over::<LANES>(&mut lanes, added, middle)?;
                (added, passed)
            };
            let middle_at = at(kernel, 1)?;
            let was = kernel.atomic_add_at(TALLY, middle_at, middle.0.id())?;
            let top = {
                let mut lanes = kernel.lanes()?;
                let was = lanes.splat_id::<U32, LANES>(was)?;
                let sum = lanes.add(was, middle.0)?;
                let carry = over::<LANES>(&mut lanes, sum, was)?;
                let top = lanes.add(top, carry)?;
                lanes.add(top, middle.1)?
            };
            let top_at = at(kernel, 2)?;
            let _ = kernel.atomic_add_at(TALLY, top_at, top.id())?;
        }
        Ok(())
    })
}

struct Tallying<const LANES: u32, const AXES: usize> {
    counted: [Vector<U32, LANES>; 3],
    at: [Vector<U32, LANES>; AXES],
    inside: Vector<U32, LANES>,
}

/// Adds a 2D frame's counts to `TALLY`: three counts (the cells that
/// moved, the wet and the dry cells), the first count weighted by each axis
/// of its block shifted down by [`COARSE`](crate::layout::COARSE), and the
/// blocks whose first count is not zero. That is `3 + AXES + 1` words, the
/// dimension's tally.
fn tally<const LANES: u32, const AXES: usize>(
    kernel: &mut Kernel<U32>,
    what: Tallying<LANES, AXES>,
) -> Result<(), LaneError> {
    use crate::layout::COARSE;

    let Tallying {
        counted,
        at,
        inside,
    } = what;
    let [moved, wet, dry] = counted;

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
        let mut weighted = Vec::with_capacity(AXES);
        for value in at {
            let value = lanes.shift_right_logical(value, by)?;
            let (low, high) = wide_mul::<LANES>(&mut lanes, value, moved)?;
            weighted.push((low, Some(high)));
        }

        let stirred = lanes.greater_than(moved, none)?;
        let stirred = lanes.select(stirred, one, none)?;

        let mut shares = Vec::with_capacity(AXES + 4);
        shares.extend([(moved, None), (wet, None), (dry, None)]);
        shares.extend(weighted);
        shares.push((stirred, None));
        shares
    };
    tally_wide::<LANES>(kernel, &shares)
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

pub(crate) struct Block<const LANES: u32, const AXES: usize> {
    pub subgroup: u32,
    pub grid: Grid,
    pub at: [Vector<U32, LANES>; AXES],
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
    woken_by::<LANES, AXES, CORNERS, NEAR, _>(
        kernel,
        Block { subgroup, grid, at },
        [holding, inside],
        dump_base,
        |lanes, offset| {
            Space::<AXES, CORNERS, NEAR>::neighbour(&mut Emit::<LANES>(lanes), grid, at, offset)
        },
    )
}

pub(crate) fn woken_by<
    const LANES: u32,
    const AXES: usize,
    const CORNERS: usize,
    const NEAR: usize,
    F,
>(
    kernel: &mut Kernel<U32>,
    block: Block<LANES, AXES>,
    [holding, inside]: [Vector<U32, LANES>; 2],
    dump_base: simdr::module::Id,
    mut numbered: F,
) -> Result<(), LaneError>
where
    F: FnMut(&mut Lanes<'_>, [u32; AXES]) -> Result<Vector<U32, LANES>, LaneError>,
{
    let Block { subgroup, grid, .. } = block;
    let lane = kernel.local_index();
    let group = kernel.workgroup_index();

    for offset in Space::<AXES, CORNERS, NEAR>::near() {
        let slot = {
            let mut lanes = kernel.lanes()?;
            let none = lanes.splat_bits::<U32, LANES>(0)?;
            let flag = numbered(&mut lanes, offset)?;

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

    let reach = crate::compact::over_reach(tier, over);
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, reach))?;
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
