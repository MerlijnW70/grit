#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use grit::compact::Plan;
use grit::layout::PARAMS;
use grit::sand::Grid;
use grit::{Groups, Host, sweep};

const BLOCKS: u32 = 16;

const RUN: u32 = 10_000;

const ROUNDS: usize = 50;

const POURED_BLOCKS: u32 = 256;

const POURED: u32 = 2000;

const POURED_ROUNDS: usize = 4;

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

fn sized(width: u32, blocks: u32) -> Grid {
    let cells = u64::from(blocks) * u64::from(sweep::cells(width)) * 4;
    let bits = cells.trailing_zeros();
    let wide = 1_u32 << (bits / 2 + bits % 2);
    let deep = u32::try_from(cells / u64::from(wide)).unwrap();
    Grid::of(wide, deep)
}

#[test]
fn the_frame_counter_counts_frames() {
    let Some(host) = device("counting") else {
        return;
    };
    let width = host.width();
    let grid = sized(width, BLOCKS);
    let plan = match Plan::over(width, grid.quads().div_ceil(sweep::cells(width))) {
        Ok(plan) => plan,
        Err(why) => {
            eprintln!("SKIPPED {width} {why}");
            return;
        }
    };
    let sizes = plan.sizes();
    let tick = grit::live::tick(width).unwrap();
    let modules: Vec<&[u32]> = vec![&tick];
    let stage: Vec<Groups> = vec![Groups::Fixed(1)];

    eprintln!("{} {width} {RUN} {ROUNDS}", host.name());

    let mut held = host.resident(&sizes, &modules).unwrap();
    for (binding, words) in sizes.iter().enumerate() {
        held.write(binding, &vec![0_u32; *words]).unwrap();
    }

    let mut seen = Vec::with_capacity(ROUNDS);
    for round in 0..ROUNDS {
        held.write(PARAMS as usize, &[0_u32; 8]).unwrap();
        held.staged_frames(&[&stage], RUN).unwrap();
        let now = held
            .read(PARAMS as usize, 1)
            .unwrap()
            .first()
            .copied()
            .unwrap_or(0);
        if now != RUN {
            eprintln!("{round} {RUN} {now}");
        }
        seen.push(now);
    }

    let wrong: Vec<(usize, u32)> = seen
        .iter()
        .enumerate()
        .filter(|(_, now)| **now != RUN)
        .map(|(round, now)| (round, *now))
        .collect();
    eprintln!("{} {ROUNDS} {RUN} {wrong:?}", ROUNDS - wrong.len());
    assert!(wrong.is_empty());
}

#[test]
fn the_counter_after_a_pour_is_the_frames_poured() {
    use grit::layout::{BRUSH, STATE};
    use grit::sand::{AIR, SAND, WALL};

    let Some(host) = device("counting") else {
        return;
    };
    let width = host.width();
    let grid = sized(width, POURED_BLOCKS);
    let plan = match Plan::over(width, grid.quads().div_ceil(sweep::cells(width))) {
        Ok(plan) => plan,
        Err(why) => {
            eprintln!("SKIPPED {width} {why}");
            return;
        }
    };
    let sizes = plan.sizes();

    eprintln!(
        "{} {width} {} {POURED} {POURED_ROUNDS}",
        host.name(),
        plan.blocks
    );

    let mut wrong = Vec::new();
    for coarse in [true, false] {
        let (passes, stages) = grit::live::arranged(width, grid, &plan, None, coarse).unwrap();
        let groups: Vec<&[Groups]> = stages.iter().map(Vec::as_slice).collect();
        let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();

        for round in 0..POURED_ROUNDS {
            let mut held = host.resident(&sizes, &modules).unwrap();
            for (binding, words) in sizes.iter().enumerate() {
                held.write(binding, &vec![0_u32; *words]).unwrap();
            }
            let mut cells = vec![AIR; grid.cells()];
            for row in 0..grid.deep {
                for column in 0..grid.wide {
                    let edge = row + 1 >= grid.deep || column == 0 || column + 1 >= grid.wide;
                    if let Some(slot) = cells.get_mut((row * grid.wide + column) as usize) {
                        *slot = if edge { WALL } else { AIR };
                    }
                }
            }
            held.write(STATE as usize, &cells).unwrap();
            held.write(PARAMS as usize, &[u32::MAX, plan.dump()])
                .unwrap();
            grit::live::seed(&mut held, grid, &plan).unwrap();
            held.staged(&groups).unwrap();
            let (x, y) = (grid.wide / 2, grid.deep / 8);
            held.write_at(PARAMS as usize, BRUSH, &[x, y, 49, SAND])
                .unwrap();
            held.staged_frames(&groups, POURED).unwrap();
            held.write_at(PARAMS as usize, BRUSH, &[x, y, 49, AIR])
                .unwrap();

            let now = held
                .read(PARAMS as usize, 1)
                .unwrap()
                .first()
                .copied()
                .unwrap_or(0);
            if now != POURED {
                eprintln!("{} {round} {POURED} {now}", coarse);
                wrong.push((coarse, round, now));
            }
        }
    }
    eprintln!("{}", wrong.len());
    assert!(wrong.is_empty());
}

#[test]
fn what_a_tick_costs() {
    let Some(host) = device("counting") else {
        return;
    };
    let width = host.width();
    let grid = sized(width, BLOCKS);
    let plan = match Plan::over(width, grid.quads().div_ceil(sweep::cells(width))) {
        Ok(plan) => plan,
        Err(why) => {
            eprintln!("SKIPPED {width} {why}");
            return;
        }
    };
    let sizes = plan.sizes();
    let tick = grit::live::tick(width).unwrap();
    let modules: Vec<&[u32]> = vec![&tick];
    let stage: Vec<Groups> = vec![Groups::Fixed(1)];

    let mut held = host.resident(&sizes, &modules).unwrap();
    for (binding, words) in sizes.iter().enumerate() {
        held.write(binding, &vec![0_u32; *words]).unwrap();
    }

    eprintln!("{} {width}", host.name());
    for times in [1_u32, 10, 100, 1000] {
        let mut seen = Vec::new();
        for round in 0..6 {
            held.staged_frames(&[&stage], times).unwrap();
            if round > 0 {
                seen.push(held.on_device().unwrap_or_default().as_nanos());
            }
        }
        eprintln!(
            "{times:>5} {} {} {}",
            seen.iter().copied().min().unwrap_or(0),
            seen.iter().copied().max().unwrap_or(0),
            seen.len()
        );
    }
}
