//! Whether a device can be asked which arrangement it prefers, and what asking costs.
//!
//! `tests/coarse.rs` prices the two arrangements and compares the answer with
//! [`grit::live::COARSE_FROM`], a constant. The constant is right on one of the two cards this
//! machine holds and wrong on the other, and no rule derived from the subgroup width fixes that: at
//! the same 131 072 cells the sixty-four-lane part prefers the list and the thirty-two-lane part
//! prefers every block, so width names which card it is rather than why.
//!
//! What is left is to ask the device, which is what [`grit::live::both`] and
//! [`grit::live::cheaper`] are for. This is where their numbers come from: how few frames it takes
//! before the winner stops changing, what has to be paid before either arrangement can be timed at
//! all, and what the whole path costs against the constant over a long pour.
//!
//! One test here checks and the rest report. `decisions/DR-0019` is what they report into.

// A test may panic — that is how it reports.
#![allow(clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::time::{Duration, Instant};

use grit::compact::Plan;
use grit::layout::{BRUSH, GRAND, PARAMS, STATE};
use grit::sand::{AIR, Grid, SAND, WALL};
use grit::{Groups, Host, sweep};

/// Block counts to ask the question at, either side of both cards' crossings.
const SIZES: [u32; 7] = [64, 128, 256, 512, 1024, 2048, 4096];

/// How many frames a calibration is allowed, in the order they are tried.
const BUDGETS: [u32; 5] = [1, 5, 20, 60, 300];

/// How many times each budget is asked, to see whether it answers the same way twice.
const ROUNDS: usize = 5;

/// How many frames of each arrangement the end-to-end comparison spends on deciding.
///
/// **Twenty, and `how_few_frames_name_the_winner` is why**: one frame named the wrong arrangement
/// at two of the seven sizes, and five agreed with three hundred at every size on the Radeon. It is
/// also one submission at [`BATCH`].
const CALIBRATION: u32 = 20;

/// Frames a submission, as `tests/coarse.rs` uses — a frame's own submit and fence would otherwise
/// drown the difference being measured.
const BATCH: u32 = 20;

/// How many frames the same-world check pours before comparing.
const AGREEING: u32 = 60;

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
    let bits = cells.trailing_zeros();
    let wide = 1_u32 << (bits / 2 + bits % 2);
    let deep = u32::try_from(cells / u64::from(wide)).expect("deep");
    Grid::of(wide, deep)
}

/// A buffer as one number, so whole worlds can be compared without holding them all.
fn folded(words: &[u32]) -> u64 {
    let mut held = 0xcbf2_9ce4_8422_2325_u64;
    for one in words {
        held ^= u64::from(*one);
        held = held.wrapping_mul(0x100_0000_01b3);
    }
    held
}

/// Put a seeded world and a brush into a resident that has just been made.
fn seeded(resident: &mut grit::Resident<'_>, grid: Grid, plan: &Plan, sizes: &[usize]) {
    for (binding, held) in sizes.iter().enumerate() {
        resident.write(binding, &vec![0_u32; *held]).expect("wipe");
    }
    resident.write(STATE as usize, &room(grid)).expect("room");
    resident
        .write(PARAMS as usize, &[u32::MAX, plan.dump()])
        .expect("params");
    grit::live::seed(resident, grid, plan).expect("seed");
    let (x, y) = (grid.wide / 2, grid.deep / 8);
    resident
        .write_at(PARAMS as usize, BRUSH, &[x, y, 49, SAND])
        .expect("brush");
}

/// Pour `frames` frames, a [`BATCH`] a submission, and read nothing back.
///
/// **Nothing, because a read is not part of what is being timed.** Reading the world costs four to
/// sixteen megabytes over the bus, which on the 4080 is more than the two thousand frames it would
/// be attributed to — this file had it inside the timed path for one measurement and the numbers
/// doubled.
fn pouring(resident: &mut grit::Resident<'_>, groups: &[&[Groups]], frames: u32) {
    for _ in (0..frames).step_by(BATCH as usize) {
        resident.staged_frames(groups, BATCH).expect("frames");
    }
}

/// Pour `frames` frames and hand back the world and the work list.
fn poured(
    resident: &mut grit::Resident<'_>,
    grid: Grid,
    groups: &[&[Groups]],
    frames: u32,
) -> (u64, u32) {
    for _ in (0..frames).step_by(BATCH as usize) {
        resident.staged_frames(groups, BATCH).expect("frames");
    }
    let world = folded(&resident.read(STATE as usize, grid.cells()).expect("state"));
    let stepped = resident
        .read(GRAND as usize, 1)
        .expect("grand")
        .first()
        .copied()
        .unwrap_or(0);
    (world, stepped)
}

/// What one arrangement cost a frame, and what had to be paid before it could be timed.
struct Priced {
    /// Device time a frame, over the last submission.
    each: Duration,
    /// Host time to emit the arrangement's modules.
    built: Duration,
    /// Host time to make it resident — pipelines, buffers and descriptor set.
    made: Duration,
}

/// One arrangement alone, in a resident of its own, over one frame budget.
fn priced(host: &Host, width: u32, grid: Grid, coarse: bool, frames: u32) -> Priced {
    let plan = Plan::over(width, grid.quads().div_ceil(sweep::cells(width))).expect("plan");

    let clock = Instant::now();
    let (passes, stages) =
        grit::live::arranged(width, grid, &plan, None, coarse).expect("arranged");
    let built = clock.elapsed();

    let groups: Vec<&[Groups]> = stages.iter().map(Vec::as_slice).collect();
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let sizes = plan.sizes();

    let clock = Instant::now();
    let mut resident = host.resident(&sizes, &modules).expect("resident");
    let made = clock.elapsed();

    seeded(&mut resident, grid, &plan, &sizes);
    resident.staged(&groups).expect("warm");
    resident.staged_frames(&groups, frames).expect("frames");
    Priced {
        each: resident.on_device().unwrap_or_default(),
        built,
        made,
    }
}

/// **The check.** An arrangement silenced beside another computes what it computes alone.
///
/// [`grit::live::both`] puts both arrangements' passes in one module list and hands back a stage
/// list for each, the one that is not running given counts of nothing. That is a claim about
/// [`grit::Resident`] — that a pass told to run no workgroups is not recorded and takes no barrier
/// with it, and that the pass indices still line up — and if it were wrong the passes would be
/// dispatched over the wrong counts and nothing in either list would look wrong. So this pours the
/// same frames four ways and demands one world.
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
            eprintln!("SKIPPED crossing at width {width}: {why}");
            return;
        }
    };
    let sizes = plan.sizes();

    let both = grit::live::both(width, grid, &plan, None).expect("both");
    let together: Vec<&[u32]> = both.passes.iter().map(Vec::as_slice).collect();
    let listed: Vec<&[Groups]> = both.listed.iter().map(Vec::as_slice).collect();
    let every: Vec<&[Groups]> = both.every.iter().map(Vec::as_slice).collect();

    let mut seen = Vec::new();
    for (what, groups) in [("silenced list", &listed), ("silenced every", &every)] {
        let mut resident = host.resident(&sizes, &together).expect("resident");
        seeded(&mut resident, grid, &plan, &sizes);
        resident.staged(groups).expect("warm");
        seen.push((what, poured(&mut resident, grid, groups, AGREEING)));
    }
    for (what, coarse) in [("list alone", true), ("every alone", false)] {
        let (passes, stages) =
            grit::live::arranged(width, grid, &plan, None, coarse).expect("arranged");
        let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
        let groups: Vec<&[Groups]> = stages.iter().map(Vec::as_slice).collect();
        let mut resident = host.resident(&sizes, &modules).expect("resident");
        seeded(&mut resident, grid, &plan, &sizes);
        resident.staged(&groups).expect("warm");
        seen.push((what, poured(&mut resident, grid, &groups, AGREEING)));
    }

    eprintln!(
        "{} — subgroup {width}, {} blocks, {AGREEING} frames",
        host.name(),
        plan.blocks
    );
    for (what, (world, stepped)) in &seen {
        eprintln!("  {what:<16} world {world:#018x}  work list {stepped}");
    }
    let first = seen[0].1;
    for (what, held) in &seen {
        assert_eq!(
            *held, first,
            "{what} reached a different place from {}: the silencing is not the same program",
            seen[0].0
        );
    }
}

/// How few frames it takes to get the same answer as three hundred do.
#[test]
fn how_few_frames_name_the_winner() {
    let Some(host) = device("crossing") else {
        return;
    };
    let width = host.width();
    eprintln!("{} — subgroup {width}", host.name());
    eprintln!(
        "  {:>7}  {:>7}  {:>7}  which arrangement each round called cheaper",
        "blocks", "frames", "rounds"
    );

    for blocks in SIZES {
        let grid = sized(width, blocks);
        if let Err(why) = Plan::over(width, grid.quads().div_ceil(sweep::cells(width))) {
            eprintln!("  {blocks:>7}  no plan at subgroup {width}: {why}");
            continue;
        }
        for frames in BUDGETS {
            let mut called = Vec::with_capacity(ROUNDS);
            let mut gaps = Vec::with_capacity(ROUNDS);
            for _ in 0..ROUNDS {
                let with = priced(&host, width, grid, true, frames);
                let without = priced(&host, width, grid, false, frames);
                called.push(if with.each < without.each {
                    "list"
                } else {
                    "every"
                });
                let (a, b) = (with.each.as_secs_f64(), without.each.as_secs_f64());
                gaps.push((a - b) / a.max(b) * 100.0);
            }
            let agreed = called.windows(2).all(|two| two[0] == two[1]);
            eprintln!(
                "  {blocks:>7}  {frames:>7}  {ROUNDS:>7}  {}  {}  gap {:.0}..{:.0}%",
                called.join(" "),
                if agreed { "agreed" } else { "**split**" },
                gaps.iter().copied().fold(f64::MAX, f64::min),
                gaps.iter().copied().fold(f64::MIN, f64::max),
            );
        }
    }
}

/// What has to be paid on the host before either arrangement can be timed.
#[test]
fn what_asking_the_device_costs_before_it_answers() {
    let Some(host) = device("crossing") else {
        return;
    };
    let width = host.width();
    eprintln!("{} — subgroup {width}", host.name());
    eprintln!(
        "  {:>7}  {:>10}  {:>10}  {:>10}  {:>10}",
        "blocks", "built list", "made list", "built every", "made every"
    );

    for blocks in SIZES {
        let grid = sized(width, blocks);
        if Plan::over(width, grid.quads().div_ceil(sweep::cells(width))).is_err() {
            continue;
        }
        let with = priced(&host, width, grid, true, 1);
        let without = priced(&host, width, grid, false, 1);
        eprintln!(
            "  {blocks:>7}  {:>10.3?}  {:>10.3?}  {:>10.3?}  {:>10.3?}",
            with.built, with.made, without.built, without.made
        );
    }
}

/// What a run costs the host from nothing to `frames` frames poured, and which arrangement it ran.
struct Whole {
    /// Wall clock for the whole path: emitting modules, making it resident, and the frames.
    wall: Duration,
    /// Which arrangement the path ran, where it had a choice.
    chose: &'static str,
}

/// The path the constant names: build the one arrangement it picks, make it resident, pour.
fn by_constant(host: &Host, width: u32, grid: Grid, frames: u32) -> Whole {
    let clock = Instant::now();
    let plan = Plan::over(width, grid.quads().div_ceil(sweep::cells(width))).expect("plan");
    let coarse = grit::live::coarse_pays(plan.blocks);
    let (passes, stages) =
        grit::live::arranged(width, grid, &plan, None, coarse).expect("arranged");
    let groups: Vec<&[Groups]> = stages.iter().map(Vec::as_slice).collect();
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let sizes = plan.sizes();
    let mut resident = host.resident(&sizes, &modules).expect("resident");
    seeded(&mut resident, grid, &plan, &sizes);
    resident.staged(&groups).expect("warm");
    pouring(&mut resident, &groups, frames);
    Whole {
        wall: clock.elapsed(),
        chose: if coarse { "list" } else { "every" },
    }
}

/// The path that asks the device, spending the first two submissions of the run on the question.
fn by_measurement(host: &Host, width: u32, grid: Grid, frames: u32, budget: u32) -> Whole {
    let clock = Instant::now();
    let plan = Plan::over(width, grid.quads().div_ceil(sweep::cells(width))).expect("plan");
    let both = grit::live::both(width, grid, &plan, None).expect("both");
    let modules: Vec<&[u32]> = both.passes.iter().map(Vec::as_slice).collect();
    let sizes = plan.sizes();
    let mut resident = host.resident(&sizes, &modules).expect("resident");
    seeded(&mut resident, grid, &plan, &sizes);

    let asked = grit::live::cheaper(&mut resident, &both, budget).expect("cheaper");
    let stages = if asked.coarse {
        &both.listed
    } else {
        &both.every
    };
    let groups: Vec<&[Groups]> = stages.iter().map(Vec::as_slice).collect();
    // **The calibration's frames are frames of the run.** Both paths pour the same number into the
    // same world; this one spends its first two submissions finding out which arrangement to spend
    // the rest on.
    let left = frames.saturating_sub(budget * 2);
    pouring(&mut resident, &groups, left);
    Whole {
        wall: clock.elapsed(),
        chose: if asked.coarse { "list" } else { "every" },
    }
}

/// What asking the device is worth over a long pour, against reading the constant.
#[test]
fn what_calibrating_costs_end_to_end() {
    let Some(host) = device("crossing") else {
        return;
    };
    let width = host.width();
    eprintln!(
        "{} — subgroup {width}, a calibration of {CALIBRATION} frames an arrangement",
        host.name()
    );
    eprintln!(
        "  {:>7}  {:>7}  {:>12}  {:>7}  {:>12}  {:>7}",
        "blocks", "frames", "by constant", "chose", "by measure", "chose"
    );

    for blocks in SIZES {
        let grid = sized(width, blocks);
        if Plan::over(width, grid.quads().div_ceil(sweep::cells(width))).is_err() {
            continue;
        }
        for frames in [BATCH, 100, 500, 2000] {
            let one = by_constant(&host, width, grid, frames);
            let two = by_measurement(&host, width, grid, frames, CALIBRATION);
            eprintln!(
                "  {blocks:>7}  {frames:>7}  {:>12.3?}  {:>7}  {:>12.3?}  {:>7}",
                one.wall, one.chose, two.wall, two.chose
            );
        }
    }
}
