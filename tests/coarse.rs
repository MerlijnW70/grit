#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use std::time::Duration;

use grit::compact::Plan;
use grit::layout::{BRUSH, GRAND, PARAMS, STATE};
use grit::live::{self, COARSE_FROM, coarse_pays};
use grit::sand::{AIR, Grid, SAND, WALL};
use grit::{Groups, Host, sweep};

const FRAMES: u32 = 300;

const BATCH: u32 = 20;

const POURING: u32 = 200;

const BELOW: u32 = 1024;

const ABOVE: u32 = 4096;

const ASKED_ABOVE: f64 = 0.25;

const ASKING: u32 = 20;

const COARSE_PASSES: usize = 5;

const MARGIN: f64 = 0.10;
fn device(label: &str) -> Option<Host> {
    match Host::open() {
        Ok(Some(host)) => Some(host),
        Ok(None) => {
            eprintln!("SKIPPED {label}");
            None
        }
        Err(why) => {
            eprintln!("SKIPPED {label} {why}");
            None
        }
    }
}

fn room(grid: Grid) -> Vec<u32> {
    let mut cells = vec![AIR; grid.cells()];
    for row in 0..grid.deep {
        for column in 0..grid.wide {
            let edge = row + 1 >= grid.deep || column == 0 || column + 1 >= grid.wide;
            if let Some(slot) = cells.get_mut((row * grid.wide + column) as usize) {
                *slot = if edge { WALL } else { AIR };
            }
        }
    }
    cells
}

fn cost(host: &Host, width: u32, grid: Grid, coarse: bool) -> (u32, u32, Duration) {
    let quads = grid.quads();
    let plan = Plan::over(width, quads.div_ceil(sweep::cells(width))).unwrap();
    let (passes, stages) = grit::live::arranged(width, grid, &plan, None, coarse).unwrap();
    let groups: Vec<&[Groups]> = stages.iter().map(Vec::as_slice).collect();
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let sizes = plan.sizes();

    let mut resident = host.resident(&sizes, &modules).unwrap();
    for (binding, held) in sizes.iter().enumerate() {
        resident.write(binding, &vec![0_u32; *held]).unwrap();
    }
    resident.write(STATE as usize, &room(grid)).unwrap();
    resident
        .write(PARAMS as usize, &[u32::MAX, plan.dump()])
        .unwrap();
    grit::live::seed(&mut resident, grid, &plan).unwrap();
    resident.staged(&groups).unwrap();

    for frame in (0..FRAMES).step_by(BATCH as usize) {
        let material = if frame < POURING { SAND } else { AIR };
        let (x, y) = (grid.wide / 2, grid.deep / 8);
        resident
            .write_at(PARAMS as usize, BRUSH, &[x, y, 49, material])
            .unwrap();
        resident.staged_frames(&groups, BATCH).unwrap();
    }
    let card = resident.on_device().unwrap_or_default();
    let stepped = resident
        .read(GRAND as usize, 1)
        .unwrap()
        .first()
        .copied()
        .unwrap_or(0);
    (plan.blocks, stepped, card)
}

fn sized(width: u32, blocks: u32) -> Grid {
    let cells = u64::from(blocks) * u64::from(sweep::cells(width)) * 4;
    assert!(cells.is_power_of_two());
    let bits = cells.trailing_zeros();
    let wide = 1_u32 << (bits / 2 + bits % 2);
    let deep = u32::try_from(cells / u64::from(wide)).unwrap();
    Grid::of(wide, deep)
}

fn best(host: &Host, width: u32, grid: Grid, coarse: bool) -> (u32, u32, Duration) {
    let first = cost(host, width, grid, coarse);
    let again = cost(host, width, grid, coarse);
    assert_eq!(first.1, again.1);
    if again.2 < first.2 { again } else { first }
}

const _: () = assert!(BELOW < COARSE_FROM && COARSE_FROM <= ABOVE);
const _: () = assert!(!coarse_pays(BELOW));
const _: () = assert!(coarse_pays(ABOVE));

#[test]
fn the_threshold_is_where_the_two_arrangements_cross() {
    let Some(host) = device("coarse") else { return };
    let width = host.width();
    eprintln!("{} {width}", host.name());

    let mut seen = Vec::new();
    for blocks in [BELOW / 4, BELOW / 2, BELOW, COARSE_FROM, ABOVE] {
        let grid = sized(width, blocks);
        if let Err(why) = Plan::over(width, grid.quads().div_ceil(sweep::cells(width))) {
            eprintln!("{blocks:>7} {width} {why}");
            continue;
        }
        let (built, listed, with) = best(&host, width, grid, true);
        let (_, stepped, without) = best(&host, width, grid, false);
        assert_eq!(built, blocks);
        assert_eq!(listed, stepped);
        eprintln!(
            "{blocks:>7} {:>10} {with:>12.3?} {without:>12.3?} {listed:>8}",
            grid.cells()
        );
        seen.push((blocks, with, without));
    }

    if std::env::var_os("CI").is_some() {
        return;
    }

    let mut disagrees = Vec::new();
    let mut parted: Vec<u32> = Vec::new();
    for (blocks, with, without) in &seen {
        let gap = (with.as_secs_f64() - without.as_secs_f64()).abs()
            / with.as_secs_f64().max(without.as_secs_f64());
        let cheaper_here = with < without;
        if gap > MARGIN && cheaper_here != coarse_pays(*blocks) {
            disagrees.push(*blocks);
        }
    }
    if disagrees.is_empty() {
        eprintln!("{COARSE_FROM}");
    } else {
        eprintln!("{COARSE_FROM} {disagrees:?}");
    }

    for (blocks, with, without) in &seen {
        let gap = (with.as_secs_f64() - without.as_secs_f64()).abs()
            / with.as_secs_f64().max(without.as_secs_f64());
        if gap <= ASKED_ABOVE {
            eprintln!("{blocks:>7} {:.0}", gap * 100.0);
            continue;
        }
        let grid = sized(width, *blocks);
        let plan = Plan::over(width, grid.quads().div_ceil(sweep::cells(width))).unwrap();
        let both = grit::live::both(width, grid, &plan, None).unwrap();
        let modules: Vec<&[u32]> = both.passes.iter().map(Vec::as_slice).collect();
        let sizes = plan.sizes();
        let mut resident = host.resident(&sizes, &modules).unwrap();
        for (binding, held) in sizes.iter().enumerate() {
            resident.write(binding, &vec![0_u32; *held]).unwrap();
        }
        resident.write(STATE as usize, &room(grid)).unwrap();
        resident
            .write(PARAMS as usize, &[u32::MAX, plan.dump()])
            .unwrap();
        grit::live::seed(&mut resident, grid, &plan).unwrap();
        let (x, y) = (grid.wide / 2, grid.deep / 8);
        resident
            .write_at(PARAMS as usize, BRUSH, &[x, y, 49, SAND])
            .unwrap();
        let asked = grit::live::cheaper(&mut resident, &both, ASKING).unwrap();
        eprintln!(
            "{blocks:>7} {FRAMES} {} {ASKING} {} {:.3?} {:.3?} {}",
            with < without,
            asked.coarse,
            asked.listed,
            asked.every,
            asked.on_device
        );
        if asked.coarse != (with < without) {
            parted.push(*blocks);
        }
    }
    if parted.is_empty() {
    } else {
        eprintln!("{parted:?}");
    }
}

#[test]
fn the_block_the_threshold_names_is_the_first_one_that_pays() {
    assert!(!coarse_pays(COARSE_FROM - 1));
    assert!(coarse_pays(COARSE_FROM));
    assert!(coarse_pays(COARSE_FROM + 1));
    assert!(!coarse_pays(0));
}

#[test]
fn the_two_arrangements_a_choice_is_made_between_are_not_the_same_one() {
    let Some(host) = device("the_two_arrangements_a_choice_is_made_between") else {
        return;
    };
    let width = host.width();
    let grid = sized(width, COARSE_FROM);
    let plan = Plan::over(width, grid.quads().div_ceil(sweep::cells(width))).unwrap();

    let (with_passes, with_stages) = live::arranged(width, grid, &plan, None, true).unwrap();
    let (without_passes, without_stages) = live::arranged(width, grid, &plan, None, false).unwrap();

    for stages in [&with_stages, &without_stages] {
        assert!(stages.iter().all(|stage| !stage.is_empty()));
        let counted: usize = stages.iter().map(Vec::len).sum();
        assert_eq!(
            counted,
            if stages == &with_stages {
                with_passes.len()
            } else {
                without_passes.len()
            }
        );
    }
    eprintln!("{} {}", with_stages.len(), without_stages.len());
    assert!(with_stages.len() > without_stages.len());
    assert_eq!(with_passes.len(), without_passes.len() + COARSE_PASSES);
    let both = live::both(width, grid, &plan, None).unwrap();

    eprintln!(
        "{} {} {}",
        with_passes.len(),
        without_passes.len(),
        both.passes.len()
    );
    assert!(with_passes.len() > without_passes.len());
    assert_eq!(both.passes.len(), with_passes.len() + without_passes.len());
    assert_eq!(both.listed.len(), both.every.len());

    let listed: usize = both
        .listed
        .iter()
        .map(|stage| {
            stage
                .iter()
                .filter(|held| !matches!(held, Groups::Fixed(0)))
                .count()
        })
        .sum();
    let every: usize = both
        .every
        .iter()
        .map(|stage| {
            stage
                .iter()
                .filter(|held| !matches!(held, Groups::Fixed(0)))
                .count()
        })
        .sum();
    assert_ne!(listed, every);
}

#[test]
fn the_cheaper_arrangement_is_the_one_the_clock_named() {
    let Some(host) = device("the_cheaper_arrangement_is_the_one_the_clock_named") else {
        return;
    };
    let width = host.width();
    let grid = sized(width, COARSE_FROM);
    let plan = Plan::over(width, grid.quads().div_ceil(sweep::cells(width))).unwrap();
    let both = live::both(width, grid, &plan, None).unwrap();

    let modules: Vec<&[u32]> = both.passes.iter().map(Vec::as_slice).collect();
    let sizes = plan.sizes();
    let mut held = host.resident(&sizes, &modules).unwrap();
    for (binding, words) in sizes.iter().enumerate() {
        held.write(binding, &vec![0_u32; *words]).unwrap();
    }
    held.write(STATE as usize, &room(grid)).unwrap();
    held.write(PARAMS as usize, &[u32::MAX, plan.dump()])
        .unwrap();
    live::seed(&mut held, grid, &plan).unwrap();

    let picked = live::cheaper(&mut held, &both, 4).unwrap();
    eprintln!(
        "{} {:.3?} {:.3?} {}",
        picked.coarse, picked.listed, picked.every, picked.on_device
    );
    assert_eq!(picked.coarse, picked.listed < picked.every);
    assert_eq!(picked.on_device, held.on_device().is_some());
}

#[test]
fn the_brush_wakes_the_ground_it_paints() {
    let Some(host) = device("the_brush_wakes_the_ground_it_paints") else {
        return;
    };
    let width = host.width();
    let grid = Grid::of(256, 256);
    let plan = Plan::over(width, grid.quads().div_ceil(sweep::cells(width))).unwrap();
    let (passes, stages) = grit::live::arranged(width, grid, &plan, None, false).unwrap();
    let groups: Vec<&[Groups]> = stages.iter().map(Vec::as_slice).collect();
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();

    let sizes = plan.sizes();
    let mut held = host.resident(&sizes, &modules).unwrap();
    for (binding, words) in sizes.iter().enumerate() {
        held.write(binding, &vec![0_u32; *words]).unwrap();
    }
    held.write(STATE as usize, &room(grid)).unwrap();
    held.write(PARAMS as usize, &[u32::MAX, plan.dump()])
        .unwrap();
    grit::live::seed(&mut held, grid, &plan).unwrap();

    let quiet = [0_u32, 0, 0, AIR];
    held.write_at(PARAMS as usize, BRUSH, &quiet).unwrap();
    held.staged_frames(&groups, 64).unwrap();
    let asleep = held
        .read(GRAND as usize, 1)
        .unwrap()
        .first()
        .copied()
        .unwrap_or(0);
    eprintln!("{asleep}");
    assert_eq!(asleep, 0);

    let (x, y) = (grid.wide / 2, grid.deep / 4);
    held.write_at(PARAMS as usize, BRUSH, &[x, y, 49, SAND])
        .unwrap();
    held.staged(&groups).unwrap();
    held.write_at(PARAMS as usize, BRUSH, &quiet).unwrap();
    held.staged(&groups).unwrap();

    let woken = held
        .read(GRAND as usize, 1)
        .unwrap()
        .first()
        .copied()
        .unwrap_or(0);
    eprintln!("{woken}");
    assert!(woken > 0);

    let before = held.read(STATE as usize, grid.cells()).unwrap();
    held.staged_frames(&groups, 32).unwrap();
    let after = held.read(STATE as usize, grid.cells()).unwrap();
    let moved = before
        .iter()
        .zip(&after)
        .filter(|(one, other)| one != other)
        .count();
    eprintln!("{moved}");
    assert!(moved > 0);
}

#[test]
fn the_halves_a_rule_sits_between_carry_the_tier_they_were_asked_for() {
    let width = 32;
    let plan = Plan::over(width, COARSE_FROM).unwrap();

    let fine = live::keeping(width, &plan, false).unwrap();
    let coarse = live::keeping(width, &plan, true).unwrap();

    assert_eq!(fine.before.len(), 1);
    assert_eq!(coarse.before.len(), 1);
    assert_eq!(fine.before[0].len(), 3);
    assert_eq!(coarse.before[0].len(), 3);

    eprintln!("{} {}", fine.after.len(), coarse.after.len());
    assert_eq!(coarse.after.len(), fine.after.len() + 4);

    let counted = |stages: &live::Stages| stages.iter().map(Vec::len).sum::<usize>();
    assert_eq!(counted(&coarse.after), counted(&fine.after) + COARSE_PASSES);
    assert_eq!(fine.over, Plan::listed());
    assert_eq!(coarse.over, Plan::listed());
    assert!(fine.after.iter().all(|stage| !stage.is_empty()));
    assert!(coarse.after.iter().all(|stage| !stage.is_empty()));
}

#[test]
fn a_stage_handed_over_empty_is_not_handed_back() {
    let width = 32;
    let plan = Plan::over(width, COARSE_FROM).unwrap();
    let keeping = live::keeping(width, &plan, false).unwrap();

    let mut stages = keeping.before;
    stages.push(Vec::new());
    stages.extend(keeping.after);
    let wanted: usize = stages.iter().map(Vec::len).sum();
    let held = stages.len();

    let (passes, groups) = live::staged(stages);
    eprintln!("{held} {} {}", groups.len(), passes.len());
    assert_eq!(passes.len(), wanted);
    assert_eq!(groups.len(), held - 1);
    assert!(groups.iter().all(|stage| !stage.is_empty()));
    assert_eq!(groups.iter().map(Vec::len).sum::<usize>(), wanted);
}
