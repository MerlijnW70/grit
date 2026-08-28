//! Which side of [`grit::live::COARSE_FROM`] a grid belongs on, measured rather than assumed.
//!
//! # What this guards
//!
//! `live::wired` picks between two arrangements that compute the same simulation: `decay`, `mark`
//! and `gather` dispatched over the coarse list, or over every block with the five passes that build
//! that list left out. Which is cheaper depends on how many blocks there are, and
//! [`grit::live::COARSE_FROM`] is where the crossing was measured — 2048 blocks on an RTX 4080 at
//! subgroup 32, where the two read 25.0 and 25.4 µs and the order changed between runs.
//!
//! # What it asserts, and what it only reports
//!
//! It used to demand that the every-block arrangement be the cheaper one an octave below the
//! constant and the dearer one an octave above. That is true on the card the constant was taken from
//! and false on the other one in this machine — at 1024 blocks the Radeon runs the list at 102 µs
//! against 306 for every block, and the constant sends it to every block — so the file was failing
//! because the *device* disagreed with a number about a different device. `decisions/DR-0019` has
//! the tables and what was done about it.
//!
//! So where the constant lands is **reported**: the row names every size at which this device wants
//! the other arrangement by more than [`MARGIN`]. Beside it goes a second report — whether
//! [`grit::live::cheaper`], which spends twenty frames an arrangement, reaches the same answer as
//! the three hundred here do, at every size where the three hundred are not close.
//!
//! Neither is asserted, and that is measured rather than cautious: held to the short-against-long
//! answer at a gap of a tenth this file passed alone and failed inside the whole suite, and at a
//! quarter one full-suite run in three still flipped a size. Twenty frames on a queue a dozen other
//! tests are also using prices the machine. What this file still **asserts** is the canary it always
//! had — that the two arrangements stepped the same number of quads, which is not a timing — and the
//! mechanism behind `cheaper` is gated by
//! `tests/crossing.rs::a_silenced_arrangement_computes_the_same_world`, which compares worlds.
//!
//! # Why a timing test at all
//!
//! Because the constant is a claim about the device, and nothing else in the repository can say
//! whether it is still true on a different card. `tests/live.rs` proves the two arrangements agree;
//! this one is the only thing that prices them.

// A test may panic — that is how it reports.
#![allow(clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::time::Duration;

use grit::compact::Plan;
use grit::layout::{BRUSH, GRAND, PARAMS, STATE};
use grit::live::{COARSE_FROM, coarse_pays};
use grit::sand::{AIR, Grid, SAND, WALL};
use grit::{Groups, Host, sweep};

/// Frames a measurement runs.
const FRAMES: u32 = 300;

/// How many of them go into one submission — a frame's own wall clock is 37 µs of submit and fence
/// whatever is inside it, which would drown a difference of five. See `tests/floor.rs`.
const BATCH: u32 = 20;

/// How long the pour lasts.
const POURING: u32 = 200;

/// Blocks below the crossing, where dispatching over every block is the cheaper arrangement.
const BELOW: u32 = 1024;

/// Blocks above it, where the coarse list is.
const ABOVE: u32 = 4096;

/// How far apart the two arrangements have to be before `grit::live::cheaper` is asked at all.
///
/// **Wider than [`MARGIN`], and `tests/crossing.rs` is why.** At 2048 blocks on the 4080 the two are
/// 10.0% apart and `how_few_frames_name_the_winner` watches a twenty-frame calibration split across
/// its five rounds there; at 1024 they are 14.9% apart and it agreed in all five. A quarter is past
/// every gap the short measurement was seen to fumble and short of every one it called five times
/// out of five, so below it the question is not worth putting.
const ASKED_ABOVE: f64 = 0.25;

/// How many frames `grit::live::cheaper` is given when this file asks it the same question.
///
/// `tests/crossing.rs` is where the number comes from: one frame named the wrong arrangement at two
/// of seven sizes, five agreed with three hundred at every size on the Radeon, and twenty is one
/// submission at the batch above.
const ASKING: u32 = 20;

/// How much of the frame the winner has to win by before the constant is called wrong about a size.
///
/// It was the margin the threshold assertions demanded either side of `COARSE_FROM`; it is now the
/// bar the report uses, so that a size where the two arrangements are within a tenth of the frame of
/// each other is not named. On the 4080 at 2048 blocks the gap is about a tenth, which is the
/// crossing this file has always declined to call.
const MARGIN: f64 = 0.10;
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

/// One arrangement at one size, timed on the device's own clock.
fn cost(host: &Host, width: u32, grid: Grid, coarse: bool) -> (u32, u32, Duration) {
    let quads = grid.quads();
    let plan = Plan::over(width, quads.div_ceil(sweep::cells(width))).expect("plan");
    let (passes, stages) =
        grit::live::arranged(width, grid, &plan, None, coarse).expect("arranged");
    let groups: Vec<&[Groups]> = stages.iter().map(Vec::as_slice).collect();
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let sizes = plan.sizes();

    let mut resident = host.resident(&sizes, &modules).expect("resident");
    for (binding, held) in sizes.iter().enumerate() {
        resident.write(binding, &vec![0_u32; *held]).expect("wipe");
    }
    resident.write(STATE as usize, &room(grid)).expect("room");
    resident
        .write(PARAMS as usize, &[u32::MAX, plan.dump()])
        .expect("params");
    grit::live::seed(&mut resident, grid, &plan).expect("seed");
    resident.staged(&groups).expect("warm");

    for frame in (0..FRAMES).step_by(BATCH as usize) {
        let material = if frame < POURING { SAND } else { AIR };
        let (x, y) = (grid.wide / 2, grid.deep / 8);
        resident
            .write_at(PARAMS as usize, BRUSH, &[x, y, 49, material])
            .expect("brush");
        resident.staged_frames(&groups, BATCH).expect("frames");
    }
    let card = resident.on_device().unwrap_or_default();
    // **The canary.** An arrangement that computed something else would hand back a timing that
    // belongs to another program, and this is what says so: both sides of a row must have stepped
    // the same number of quads. `src/bin/scale.rs` carries the argument.
    let stepped = resident
        .read(GRAND as usize, 1)
        .expect("grand")
        .first()
        .copied()
        .unwrap_or(0);
    (plan.blocks, stepped, card)
}

/// A grid with this many blocks at this width, both sides a power of two.
///
/// **Derived rather than written down**, because `blocks` is `quads / sweep::cells(width)` and that
/// divisor is not the same on a sixty-four-lane part. A table of grid sizes would measure a
/// different block count there and say nothing about the constant.
fn sized(width: u32, blocks: u32) -> Grid {
    let cells = u64::from(blocks) * u64::from(sweep::cells(width)) * 4;
    assert!(
        cells.is_power_of_two(),
        "{cells} cells is not a power of two"
    );
    let bits = cells.trailing_zeros();
    let wide = 1_u32 << (bits / 2 + bits % 2);
    let deep = u32::try_from(cells / u64::from(wide)).expect("deep");
    Grid::of(wide, deep)
}

/// [`cost`] twice, keeping the smaller.
///
/// **Both arrangements get the same treatment**, so this cannot tilt the comparison the way a
/// best-of-N taken against a fixed threshold can: it removes one machine hiccup from each side.
fn best(host: &Host, width: u32, grid: Grid, coarse: bool) -> (u32, u32, Duration) {
    let first = cost(host, width, grid, coarse);
    let again = cost(host, width, grid, coarse);
    // **The same arrangement twice has to reach the same place.** This used to keep the faster of
    // the two and throw the other count away, which meant a run that was not reproducible looked
    // like one that was — the disagreement only showed later, against the other arrangement, where
    // it reads as the two programs differing rather than as one of them wandering.
    assert_eq!(
        first.1,
        again.1,
        "the {} arrangement stepped {} quads and then {} over the same {} blocks: the same program          twice did not reach the same place",
        if coarse { "coarse" } else { "every-block" },
        first.1,
        again.1,
        first.0,
    );
    if again.2 < first.2 { again } else { first }
}

/// The two sizes have to straddle the constant, or the assertions below measure nothing.
///
/// **A build error rather than a test failure**, because all three are known at compile time: a
/// `COARSE_FROM` moved past either size would otherwise produce a green test that proved nothing.
const _: () = assert!(
    BELOW < COARSE_FROM && COARSE_FROM <= ABOVE,
    "COARSE_FROM is not between the two sizes tests/coarse.rs measures"
);
const _: () = assert!(!coarse_pays(BELOW), "BELOW is not below COARSE_FROM");
const _: () = assert!(coarse_pays(ABOVE), "ABOVE is not above COARSE_FROM");

#[test]
fn the_threshold_is_where_the_two_arrangements_cross() {
    let Some(host) = device("coarse") else { return };
    let width = host.width();
    eprintln!("{} — subgroup {width}", host.name());
    eprintln!(
        "  {:>7}  {:>10}  {:>12}  {:>12}  {:>8}",
        "blocks", "cells", "coarse", "every", "stepped"
    );

    let mut seen = Vec::new();
    for blocks in [BELOW / 4, BELOW / 2, BELOW, COARSE_FROM, ABOVE] {
        let grid = sized(width, blocks);
        // **A narrow device cannot plan every size this asks for**, and that is a fact about the
        // hierarchy rather than about the test. Two levels of scan over four-wide groups reach 1024
        // blocks; `COARSE_FROM` is 2048. So on a four-wide device the constant that decides whether
        // the coarse list is worth building names a size at which it cannot be built — see
        // `decisions/DR-0015`. What this loop can do is measure the sizes that fit and say which
        // ones did not.
        if let Err(why) = Plan::over(width, grid.quads().div_ceil(sweep::cells(width))) {
            eprintln!("  {blocks:>7}  no plan at subgroup {width}: {why}");
            continue;
        }
        let (built, listed, with) = best(&host, width, grid, true);
        let (_, stepped, without) = best(&host, width, grid, false);
        assert_eq!(built, blocks, "asked for {blocks} blocks and got {built}");
        assert_eq!(
            listed, stepped,
            "at {blocks} blocks the list stepped {listed} quads and every-block stepped {stepped}: \
             the two are not the same program, so the timings are not comparable"
        );
        eprintln!(
            "  {blocks:>7}  {:>10}  {with:>12.3?}  {without:>12.3?}  {listed:>8}",
            grid.cells()
        );
        seen.push((blocks, with, without));
    }

    // **What follows is the measurement, and it does not travel.** Which of two arrangements is
    // faster is a property of the device: on `llvmpipe`, which runs on the processor, the list is
    // the cheaper of the two at 1024 blocks — 473 µs against 552 — where on every card this was
    // written for it is the dearer one. `COARSE_FROM` is a number about graphics hardware and a
    // check of it on a shared runner would be a check of the runner.
    //
    // The equality above is not a measurement and does run there. That is the half of this file that
    // found something.
    if std::env::var_os("CI").is_some() {
        eprintln!("  the threshold itself is a measurement and CI is set; it is not checked here");
        return;
    }

    // **Where the constant lands here, reported and not asserted.** This used to demand that the
    // every-block arrangement be the cheaper one an octave below `COARSE_FROM` and the dearer one an
    // octave above, which is true on the card the constant was taken from and false on the other one
    // in this machine: at 1024 blocks the Radeon runs the list at 102 µs against 306 for every
    // block, and the constant sends it to every block. There is no rule that fixes that — at the
    // same 131 072 cells the two cards give opposite answers — so `decisions/DR-0019` makes the
    // measured choice something a caller can ask for and leaves the constant as what it always was:
    // a number about one card. A file that failed on the second card was saying that the *device*
    // was wrong, and it is the constant that does not travel.
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
        eprintln!("  COARSE_FROM is {COARSE_FROM} and this device agrees at every size measured");
    } else {
        eprintln!(
            "  COARSE_FROM is {COARSE_FROM} and this device wants the other arrangement at {disagrees:?} blocks \
             — see decisions/DR-0019 and grit::live::cheaper"
        );
    }

    // **And the short question against the long answer, reported rather than asserted.**
    // `grit::live::cheaper` spends twenty frames an arrangement deciding what the three hundred
    // above took to decide, and `tests/crossing.rs` is where that budget was measured.
    //
    // **Held to it, this file was flaky, and that was measured.** At a gap of a tenth it passed
    // alone and failed inside the whole suite; at [`ASKED_ABOVE`], a quarter, one full-suite run in
    // three still flipped a size. Twenty frames on a queue a dozen other tests are also using is a
    // measurement of the machine, which is the argument this file has always made about timing
    // assertions and applies to its own. What gates the mechanism instead is
    // `tests/crossing.rs::a_silenced_arrangement_computes_the_same_world`, which compares worlds
    // rather than clocks and does not care what else is running.
    for (blocks, with, without) in &seen {
        let gap = (with.as_secs_f64() - without.as_secs_f64()).abs()
            / with.as_secs_f64().max(without.as_secs_f64());
        if gap <= ASKED_ABOVE {
            eprintln!(
                "  {blocks:>7}  the two are within {:.0}%; not asked",
                gap * 100.0
            );
            continue;
        }
        let grid = sized(width, *blocks);
        let plan = Plan::over(width, grid.quads().div_ceil(sweep::cells(width))).expect("plan");
        let both = grit::live::both(width, grid, &plan, None).expect("both");
        let modules: Vec<&[u32]> = both.passes.iter().map(Vec::as_slice).collect();
        let sizes = plan.sizes();
        let mut resident = host.resident(&sizes, &modules).expect("resident");
        for (binding, held) in sizes.iter().enumerate() {
            resident.write(binding, &vec![0_u32; *held]).expect("wipe");
        }
        resident.write(STATE as usize, &room(grid)).expect("room");
        resident
            .write(PARAMS as usize, &[u32::MAX, plan.dump()])
            .expect("params");
        grit::live::seed(&mut resident, grid, &plan).expect("seed");
        let (x, y) = (grid.wide / 2, grid.deep / 8);
        resident
            .write_at(PARAMS as usize, BRUSH, &[x, y, 49, SAND])
            .expect("brush");
        let asked = grit::live::cheaper(&mut resident, &both, ASKING).expect("cheaper");
        eprintln!(
            "  {blocks:>7}  {FRAMES} frames say {}, {ASKING} say {} ({:.3?} against {:.3?}{})",
            if with < without { "list" } else { "every" },
            if asked.coarse { "list" } else { "every" },
            asked.listed,
            asked.every,
            if asked.on_device {
                ""
            } else {
                ", on the host's clock"
            },
        );
        if asked.coarse != (with < without) {
            parted.push(*blocks);
        }
    }
    if parted.is_empty() {
        eprintln!("  the short question got the long answer at every size it was asked");
    } else {
        eprintln!(
            "  the short question got the other answer at {parted:?} blocks — a loaded machine is \
             what that looks like; see decisions/DR-0019"
        );
    }
}
