#![allow(clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use grit::compact::Plan;
use grit::layout::{AWAKE, BRUSH, GRAND, HOT, HOT_DENSE, HOT_GRAND, PARAMS, STATE};
use grit::sand::{AIR, Grid, SAND, WALL};
use grit::{Groups, Host, sweep};

const FRAMES: u32 = 300;

const POURING: u32 = 200;

const BLOCKS: u32 = 1024;

const RUNS: usize = 5;

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
    let (mut wide, mut deep) = (1_u32, 1_u32);
    while u64::from(wide) * u64::from(deep) < cells {
        if wide <= deep * 2 {
            wide *= 2;
        } else {
            deep *= 2;
        }
    }
    Grid::of(wide, deep)
}

fn ran(
    host: &Host,
    width: u32,
    grid: Grid,
    plan: &Plan,
    apart: bool,
    batch: u32,
) -> (u32, u64, u64, usize, usize, bool, u64) {
    let (passes, wired) = grit::live::arranged(width, grid, plan, None, true).expect("arranged");
    let stages: Vec<Vec<Groups>> = if apart {
        wired.into_iter().flatten().map(|one| vec![one]).collect()
    } else {
        wired
    };
    let groups: Vec<&[Groups]> = stages.iter().map(Vec::as_slice).collect();
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let sizes = plan.sizes();

    let mut held = host.resident(&sizes, &modules).expect("resident");
    for (binding, words) in sizes.iter().enumerate() {
        held.write(binding, &vec![0_u32; *words]).expect("wipe");
    }
    held.write(STATE as usize, &room(grid)).expect("room");
    held.write(PARAMS as usize, &[u32::MAX, plan.dump()])
        .expect("params");
    grit::live::seed(&mut held, grid, plan).expect("seed");
    held.staged(&groups).expect("warm");

    for frame in (0..FRAMES).step_by(batch as usize) {
        let material = if frame < POURING { SAND } else { AIR };
        let (x, y) = (grid.wide / 2, grid.deep / 8);
        held.write_at(PARAMS as usize, BRUSH, &[x, y, 49, material])
            .expect("brush");
        held.staged_frames(&groups, batch).expect("frames");
    }
    let grand = held
        .read(GRAND as usize, 1)
        .expect("grand")
        .first()
        .copied()
        .unwrap_or(0);
    let world = folded(&held.read(STATE as usize, grid.cells()).expect("state"));
    let long = held
        .read(HOT_GRAND as usize, 1)
        .expect("hot grand")
        .first()
        .copied()
        .unwrap_or(0) as usize;
    let list = held
        .read(HOT_DENSE as usize, long.max(1))
        .expect("hot dense");
    let mut sorted = list.clone();
    sorted.sort_unstable();
    let distinct = {
        let mut held = sorted.clone();
        held.dedup();
        held.len()
    };
    let ordered = list == sorted;
    let hot = folded(
        &held
            .read(HOT as usize, plan.padded() as usize)
            .expect("hot"),
    );
    let woke = folded(
        &held
            .read(
                AWAKE as usize,
                plan.blocks as usize * plan.per_block as usize,
            )
            .expect("awake"),
    );
    (grand, world, woke, long, distinct, ordered, hot)
}

fn folded(words: &[u32]) -> u64 {
    let mut held = 0xcbf2_9ce4_8422_2325_u64;
    for one in words {
        held ^= u64::from(*one);
        held = held.wrapping_mul(0x100_0000_01b3);
    }
    held
}

#[test]
fn whether_a_barrier_is_what_is_missing() {
    let Some(host) = device("hunt") else { return };
    let width = host.width();
    let grid = sized(width, BLOCKS);
    let plan = match Plan::over(width, grid.quads().div_ceil(sweep::cells(width))) {
        Ok(plan) => plan,
        Err(why) => {
            eprintln!("SKIPPED hunt at width {width}: {why}");
            return;
        }
    };
    eprintln!(
        "{} — subgroup {width}, {}×{} cells in {} blocks, {FRAMES} frames",
        host.name(),
        grid.wide,
        grid.deep,
        plan.blocks
    );

    for (apart, batch, what) in [
        (false, 1, "grouped, a frame a submission"),
        (true, 1, "one pass a stage, a frame a submission"),
        (false, 20, "grouped, twenty frames a submission"),
        (true, 20, "one pass a stage, twenty a submission"),
    ] {
        let mut seen = Vec::with_capacity(RUNS);
        for _ in 0..RUNS {
            seen.push(ran(&host, width, grid, &plan, apart, batch));
        }
        let lists: Vec<u32> = seen.iter().map(|one| one.0).collect();
        let worlds = seen.windows(2).all(|two| two[0].1 == two[1].1);
        let wokes = seen.windows(2).all(|two| two[0].2 == two[1].2);
        let hots = seen.windows(2).all(|two| two[0].6 == two[1].6);
        let coarse: Vec<String> = seen
            .iter()
            .map(|one| format!("{}{}", one.3, if one.3 == one.4 { "" } else { "!" }))
            .collect();
        let all_ordered = seen.iter().all(|one| one.5);
        eprintln!(
            "  {what:<40} lists {lists:?}  world {}  AWAKE {}  HOT {}  coarse list {}  {}",
            if worlds { "same" } else { "**differ**" },
            if wokes { "same" } else { "**differ**" },
            if hots { "same" } else { "**differ**" },
            coarse.join("/"),
            if all_ordered {
                "ascending"
            } else {
                "**not ascending**"
            },
        );
    }
}
