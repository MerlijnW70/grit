#![allow(clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

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
            eprintln!("SKIPPED {label}: no Vulkan device");
            None
        }
        Err(why) => {
            eprintln!("SKIPPED {label}: {why}");
            None
        }
    }
}

fn sized(width: u32, blocks: u32) -> Grid {
    let cells = u64::from(blocks) * u64::from(sweep::cells(width)) * 4;
    let bits = cells.trailing_zeros();
    let wide = 1_u32 << (bits / 2 + bits % 2);
    let deep = u32::try_from(cells / u64::from(wide)).expect("deep");
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
            eprintln!("SKIPPED counting at width {width}: {why}");
            return;
        }
    };
    let sizes = plan.sizes();
    let tick = grit::live::tick(width).expect("tick");
    let modules: Vec<&[u32]> = vec![&tick];
    let stage: Vec<Groups> = vec![Groups::Fixed(1)];

    eprintln!(
        "{} — subgroup {width}, {RUN} ticks a submission, {ROUNDS} submissions",
        host.name()
    );

    let mut held = host.resident(&sizes, &modules).expect("resident");
    for (binding, words) in sizes.iter().enumerate() {
        held.write(binding, &vec![0_u32; *words]).expect("wipe");
    }

    let mut seen = Vec::with_capacity(ROUNDS);
    for round in 0..ROUNDS {
        held.write(PARAMS as usize, &[0_u32; 8]).expect("params");
        held.staged_frames(&[&stage], RUN).expect("ticks");
        let now = held
            .read(PARAMS as usize, 1)
            .expect("params")
            .first()
            .copied()
            .unwrap_or(0);
        if now != RUN {
            eprintln!("  round {round}: {RUN} ticks left the counter at {now}");
        }
        seen.push(now);
    }

    let wrong: Vec<(usize, u32)> = seen
        .iter()
        .enumerate()
        .filter(|(_, now)| **now != RUN)
        .map(|(round, now)| (round, *now))
        .collect();
    eprintln!(
        "  {} of {ROUNDS} rounds counted {RUN}; the rest {wrong:?}",
        ROUNDS - wrong.len()
    );
    assert!(
        wrong.is_empty(),
        "{RUN} dispatches of live::tick, and the frame counter reached {wrong:?} instead — every \
         invocation of the workgroup reads that word, adds one and writes it back, which is the \
         shape decisions/DR-0018 took out of live::decay"
    );
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
            eprintln!("SKIPPED counting at width {width}: {why}");
            return;
        }
    };
    let sizes = plan.sizes();

    eprintln!(
        "{} — subgroup {width}, {} blocks, {POURED} frames a round, {POURED_ROUNDS} rounds each way",
        host.name(),
        plan.blocks
    );

    let mut wrong = Vec::new();
    for coarse in [true, false] {
        let (passes, stages) =
            grit::live::arranged(width, grid, &plan, None, coarse).expect("arranged");
        let groups: Vec<&[Groups]> = stages.iter().map(Vec::as_slice).collect();
        let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();

        for round in 0..POURED_ROUNDS {
            let mut held = host.resident(&sizes, &modules).expect("resident");
            for (binding, words) in sizes.iter().enumerate() {
                held.write(binding, &vec![0_u32; *words]).expect("wipe");
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
            held.write(STATE as usize, &cells).expect("room");
            held.write(PARAMS as usize, &[u32::MAX, plan.dump()])
                .expect("params");
            grit::live::seed(&mut held, grid, &plan).expect("seed");
            held.staged(&groups).expect("warm");
            let (x, y) = (grid.wide / 2, grid.deep / 8);
            held.write_at(PARAMS as usize, BRUSH, &[x, y, 49, SAND])
                .expect("brush");
            held.staged_frames(&groups, POURED).expect("frames");
            held.write_at(PARAMS as usize, BRUSH, &[x, y, 49, AIR])
                .expect("brush");

            let now = held
                .read(PARAMS as usize, 1)
                .expect("params")
                .first()
                .copied()
                .unwrap_or(0);
            if now != POURED {
                eprintln!(
                    "  {} list, round {round}: {POURED} frames left the counter at {now}",
                    if coarse { "with a" } else { "without" }
                );
                wrong.push((coarse, round, now));
            }
        }
    }
    eprintln!("  {} rounds counted wrong", wrong.len());
    assert!(
        wrong.is_empty(),
        "the frame counter did not count the frames: {wrong:?} — every invocation of tick's \
         workgroup reads that word, adds one and writes it back, and its low bit is the partition's \
         phase"
    );
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
            eprintln!("SKIPPED counting at width {width}: {why}");
            return;
        }
    };
    let sizes = plan.sizes();
    let tick = grit::live::tick(width).expect("tick");
    let modules: Vec<&[u32]> = vec![&tick];
    let stage: Vec<Groups> = vec![Groups::Fixed(1)];

    let mut held = host.resident(&sizes, &modules).expect("resident");
    for (binding, words) in sizes.iter().enumerate() {
        held.write(binding, &vec![0_u32; *words]).expect("wipe");
    }

    eprintln!(
        "{} — subgroup {width}, one tick and the barrier behind it",
        host.name()
    );
    for times in [1_u32, 10, 100, 1000] {
        let mut seen = Vec::new();
        for round in 0..6 {
            held.staged_frames(&[&stage], times).expect("ticks");
            if round > 0 {
                seen.push(held.on_device().unwrap_or_default().as_nanos());
            }
        }
        eprintln!(
            "  {times:>5} a submission: {}-{} ns a tick over {} submissions",
            seen.iter().copied().min().unwrap_or(0),
            seen.iter().copied().max().unwrap_or(0),
            seen.len(),
        );
    }
}
