//! Whether the coarse arrangement needs a barrier it has not got.
//!
//! `tests/coarse.rs` says the coarse arrangement does not reach the same place twice on a
//! sixty-four-lane part. `tests/live.rs` says the **cells** agree for four hundred frames at the same
//! size. What settles the two is that the disagreement is in the work list and not in the world, and
//! that it comes and goes with what is read back between frames: reading one word after each frame it
//! appears at frame 1, and reading two megabytes it does not appear in three hundred and twenty.
//!
//! That is the shape of a race that a wait papers over, and this is the instrument that says whether
//! it is a missing barrier. `live::arranged` groups sixteen passes into eleven stages, and a stage is
//! a promise that the passes in it do not depend on each other. This runs the same passes twice: once
//! grouped as `arranged` grouped them, and once **one pass a stage**, which is every barrier the
//! ordering could possibly want. If the second is steady where the first is not, the promise is wrong
//! somewhere and the next question is which stage.
//!
//! It asserts nothing about which arrangement is right. It prints what each grouping did over several
//! runs, and leaves the reading to whoever is holding `decisions/DR-0013`.

// A test may panic — that is how it reports.
#![allow(clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use grit::compact::Plan;
use grit::layout::{AWAKE, BRUSH, GRAND, HOT, HOT_DENSE, HOT_GRAND, PARAMS, STATE};
use grit::sand::{AIR, Grid, SAND, WALL};
use grit::{Groups, Host, sweep};

/// How many frames a run walks.
const FRAMES: u32 = 300;

/// When the brush stops.
const POURING: u32 = 200;

/// How many blocks. The size at which `tests/coarse.rs` disagrees.
const BLOCKS: u32 = 1024;

/// How many times each grouping is run, to see whether it lands in the same place.
const RUNS: usize = 5;

/// Open a device, or say why not and hand back nothing.
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

/// A room with walls, and air inside it.
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

/// A grid with this many blocks at this width, both sides a power of two.
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

/// One whole run of one grouping, and the length of the work list it ends on.
fn ran(
    host: &Host,
    width: u32,
    grid: Grid,
    plan: &Plan,
    apart: bool,
    batch: u32,
) -> (u32, u64, u64, usize, usize, bool, u64) {
    let (passes, wired) = grit::live::arranged(width, grid, plan, None, true).expect("arranged");
    // **Every barrier the ordering could want, or the ones `wired` says it wants.** A stage is a
    // promise that what is in it does not depend on what else is in it; one pass a stage makes no
    // such promise anywhere.
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
    // **The warm-up `tests/coarse.rs` does**, which is a frame with the brush still at nothing.
    held.staged(&groups).expect("warm");

    for frame in (0..FRAMES).step_by(batch as usize) {
        let material = if frame < POURING { SAND } else { AIR };
        let (x, y) = (grid.wide / 2, grid.deep / 8);
        held.write_at(PARAMS as usize, BRUSH, &[x, y, 49, material])
            .expect("brush");
        held.staged_frames(&groups, batch).expect("frames");
    }
    // **Read once, at the end, and nowhere else.** Reading between frames changes the answer — one
    // word after each frame and the two arrangements part at frame 1, two megabytes and they never
    // part at all — so the instrument that looks every frame is not measuring the same program.
    let grand = held
        .read(GRAND as usize, 1)
        .expect("grand")
        .first()
        .copied()
        .unwrap_or(0);
    let world = folded(&held.read(STATE as usize, grid.cells()).expect("state"));
    // **The coarse list itself: is every block in it once?** A block listed twice is decayed twice,
    // which changes no cell — a settled cell stays settled either way — and drops the block off the
    // list a frame early, which is exactly a shorter list over an identical world.
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

/// A buffer as one number, so that whole buffers can be compared without holding them all.
fn folded(words: &[u32]) -> u64 {
    let mut held = 0xcbf2_9ce4_8422_2325_u64;
    for one in words {
        held ^= u64::from(*one);
        held = held.wrapping_mul(0x100_0000_01b3);
    }
    held
}

/// The same passes under two groupings, several runs each.
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
