#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use std::time::{Duration, Instant};

use grit::compact::Plan;
use grit::layout::{BRUSH, GRAND, PARAMS, STATE};
use grit::sand::{AIR, Grid, SAND, WALL};
use grit::{Groups, Host, sweep};

const SIZES: [u32; 7] = [64, 128, 256, 512, 1024, 2048, 4096];

const BUDGETS: [u32; 5] = [1, 5, 20, 60, 300];

const ROUNDS: usize = 5;

const CALIBRATION: u32 = 20;

const BATCH: u32 = 20;

const AGREEING: u32 = 60;

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

fn sized(width: u32, blocks: u32) -> Grid {
    let cells = u64::from(blocks) * u64::from(sweep::cells(width)) * 4;
    let bits = cells.trailing_zeros();
    let wide = 1_u32 << (bits / 2 + bits % 2);
    let deep = u32::try_from(cells / u64::from(wide)).unwrap();
    Grid::of(wide, deep)
}

fn folded(words: &[u32]) -> u64 {
    let mut held = 0xcbf2_9ce4_8422_2325_u64;
    for one in words {
        held ^= u64::from(*one);
        held = held.wrapping_mul(0x100_0000_01b3);
    }
    held
}

fn seeded(resident: &mut grit::Resident<'_>, grid: Grid, plan: &Plan, sizes: &[usize]) {
    for (binding, held) in sizes.iter().enumerate() {
        resident.write(binding, &vec![0_u32; *held]).unwrap();
    }
    resident.write(STATE as usize, &room(grid)).unwrap();
    resident
        .write(PARAMS as usize, &[u32::MAX, plan.dump()])
        .unwrap();
    grit::live::seed(resident, grid, plan).unwrap();
    let (x, y) = (grid.wide / 2, grid.deep / 8);
    resident
        .write_at(PARAMS as usize, BRUSH, &[x, y, 49, SAND])
        .unwrap();
}

fn pouring(resident: &mut grit::Resident<'_>, groups: &[&[Groups]], frames: u32) {
    for _ in (0..frames).step_by(BATCH as usize) {
        resident.staged_frames(groups, BATCH).unwrap();
    }
}

fn poured(
    resident: &mut grit::Resident<'_>,
    grid: Grid,
    groups: &[&[Groups]],
    frames: u32,
) -> (u64, u32) {
    for _ in (0..frames).step_by(BATCH as usize) {
        resident.staged_frames(groups, BATCH).unwrap();
    }
    let world = folded(&resident.read(STATE as usize, grid.cells()).unwrap());
    let stepped = resident
        .read(GRAND as usize, 1)
        .unwrap()
        .first()
        .copied()
        .unwrap_or(0);
    (world, stepped)
}

struct Priced {
    each: Duration,
    built: Duration,
    made: Duration,
}

fn priced(host: &Host, width: u32, grid: Grid, coarse: bool, frames: u32) -> Priced {
    let plan = Plan::over(width, grid.quads().div_ceil(sweep::cells(width))).unwrap();

    let clock = Instant::now();
    let (passes, stages) = grit::live::arranged(width, grid, &plan, None, coarse).unwrap();
    let built = clock.elapsed();

    let groups: Vec<&[Groups]> = stages.iter().map(Vec::as_slice).collect();
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let sizes = plan.sizes();

    let clock = Instant::now();
    let mut resident = host.resident(&sizes, &modules).unwrap();
    let made = clock.elapsed();

    seeded(&mut resident, grid, &plan, &sizes);
    resident.staged(&groups).unwrap();
    resident.staged_frames(&groups, frames).unwrap();
    Priced {
        each: resident.on_device().unwrap_or_default(),
        built,
        made,
    }
}

#[test]
fn a_silenced_arrangement_computes_the_same_world() {
    let Some(host) = device("crossing") else {
        return;
    };
    let width = host.width();
    let grid = sized(width, 256);
    let plan = match Plan::over(width, grid.quads().div_ceil(sweep::cells(width))) {
        Ok(plan) => plan,
        Err(why) => {
            eprintln!("SKIPPED {width} {why}");
            return;
        }
    };
    let sizes = plan.sizes();

    let both = grit::live::both(width, grid, &plan, None).unwrap();
    let together: Vec<&[u32]> = both.passes.iter().map(Vec::as_slice).collect();
    let listed: Vec<&[Groups]> = both.listed.iter().map(Vec::as_slice).collect();
    let every: Vec<&[Groups]> = both.every.iter().map(Vec::as_slice).collect();

    let mut seen = Vec::new();
    for (what, groups) in [(0_u8, &listed), (1, &every)] {
        let mut resident = host.resident(&sizes, &together).unwrap();
        seeded(&mut resident, grid, &plan, &sizes);
        resident.staged(groups).unwrap();
        seen.push((what, poured(&mut resident, grid, groups, AGREEING)));
    }
    for (what, coarse) in [(2_u8, true), (3, false)] {
        let (passes, stages) = grit::live::arranged(width, grid, &plan, None, coarse).unwrap();
        let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
        let groups: Vec<&[Groups]> = stages.iter().map(Vec::as_slice).collect();
        let mut resident = host.resident(&sizes, &modules).unwrap();
        seeded(&mut resident, grid, &plan, &sizes);
        resident.staged(&groups).unwrap();
        seen.push((what, poured(&mut resident, grid, &groups, AGREEING)));
    }

    eprintln!("{} {width} {} {AGREEING}", host.name(), plan.blocks);
    for (what, (world, stepped)) in &seen {
        eprintln!("{what} {world:#018x} {stepped}");
    }
    let first = seen[0].1;
    for (_, held) in &seen {
        assert_eq!(*held, first);
    }
}

#[test]
fn how_few_frames_name_the_winner() {
    let Some(host) = device("crossing") else {
        return;
    };
    let width = host.width();
    eprintln!("{} {width}", host.name());

    for blocks in SIZES {
        let grid = sized(width, blocks);
        if let Err(why) = Plan::over(width, grid.quads().div_ceil(sweep::cells(width))) {
            eprintln!("{blocks:>7} {width} {why}");
            continue;
        }
        for frames in BUDGETS {
            let mut called = Vec::with_capacity(ROUNDS);
            let mut gaps = Vec::with_capacity(ROUNDS);
            for _ in 0..ROUNDS {
                let with = priced(&host, width, grid, true, frames);
                let without = priced(&host, width, grid, false, frames);
                called.push(with.each < without.each);
                let (a, b) = (with.each.as_secs_f64(), without.each.as_secs_f64());
                gaps.push((a - b) / a.max(b) * 100.0);
            }
            let agreed = called.windows(2).all(|two| two[0] == two[1]);
            eprintln!(
                "{blocks:>7} {frames:>7} {ROUNDS:>7} {called:?} {agreed} {:.0} {:.0}",
                gaps.iter().copied().fold(f64::MAX, f64::min),
                gaps.iter().copied().fold(f64::MIN, f64::max)
            );
        }
    }
}

#[test]
fn what_asking_the_device_costs_before_it_answers() {
    let Some(host) = device("crossing") else {
        return;
    };
    let width = host.width();
    eprintln!("{} {width}", host.name());

    for blocks in SIZES {
        let grid = sized(width, blocks);
        if Plan::over(width, grid.quads().div_ceil(sweep::cells(width))).is_err() {
            continue;
        }
        let with = priced(&host, width, grid, true, 1);
        let without = priced(&host, width, grid, false, 1);
        eprintln!(
            "{blocks:>7} {:>10.3?} {:>10.3?} {:>10.3?} {:>10.3?}",
            with.built, with.made, without.built, without.made
        );
    }
}

struct Whole {
    wall: Duration,
    chose: bool,
}

fn by_constant(host: &Host, width: u32, grid: Grid, frames: u32) -> Whole {
    let clock = Instant::now();
    let plan = Plan::over(width, grid.quads().div_ceil(sweep::cells(width))).unwrap();
    let coarse = grit::live::coarse_pays(plan.blocks);
    let (passes, stages) = grit::live::arranged(width, grid, &plan, None, coarse).unwrap();
    let groups: Vec<&[Groups]> = stages.iter().map(Vec::as_slice).collect();
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let sizes = plan.sizes();
    let mut resident = host.resident(&sizes, &modules).unwrap();
    seeded(&mut resident, grid, &plan, &sizes);
    resident.staged(&groups).unwrap();
    pouring(&mut resident, &groups, frames);
    Whole {
        wall: clock.elapsed(),
        chose: coarse,
    }
}

fn by_measurement(host: &Host, width: u32, grid: Grid, frames: u32, budget: u32) -> Whole {
    let clock = Instant::now();
    let plan = Plan::over(width, grid.quads().div_ceil(sweep::cells(width))).unwrap();
    let both = grit::live::both(width, grid, &plan, None).unwrap();
    let modules: Vec<&[u32]> = both.passes.iter().map(Vec::as_slice).collect();
    let sizes = plan.sizes();
    let mut resident = host.resident(&sizes, &modules).unwrap();
    seeded(&mut resident, grid, &plan, &sizes);

    let asked = grit::live::cheaper(&mut resident, &both, budget).unwrap();
    let stages = if asked.coarse {
        &both.listed
    } else {
        &both.every
    };
    let groups: Vec<&[Groups]> = stages.iter().map(Vec::as_slice).collect();
    let left = frames.saturating_sub(budget * 2);
    pouring(&mut resident, &groups, left);
    Whole {
        wall: clock.elapsed(),
        chose: asked.coarse,
    }
}

#[test]
fn what_calibrating_costs_end_to_end() {
    let Some(host) = device("crossing") else {
        return;
    };
    let width = host.width();
    eprintln!("{} {width} {CALIBRATION}", host.name());

    for blocks in SIZES {
        let grid = sized(width, blocks);
        if Plan::over(width, grid.quads().div_ceil(sweep::cells(width))).is_err() {
            continue;
        }
        for frames in [BATCH, 100, 500, 2000] {
            let one = by_constant(&host, width, grid, frames);
            let two = by_measurement(&host, width, grid, frames, CALIBRATION);
            eprintln!(
                "{blocks:>7} {frames:>7} {:>12.3?} {:>7} {:>12.3?} {:>7}",
                one.wall, one.chose, two.wall, two.chose
            );
        }
    }
}
