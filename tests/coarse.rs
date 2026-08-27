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
//! **So this does not assert anything at the crossing.** It asserts the order one octave either
//! side of it, where the gap is 23% and 33% of the frame rather than 2%, and it asserts that
//! `COARSE_FROM` falls between those two. A threshold that drifted to either would fail here.
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

/// How much of the frame the winner has to win by, either side.
///
/// Measured at 23% below and 33% above over four runs; a fifth of the frame leaves room for a busy
/// machine without letting a genuine reversal pass.
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

    let find = |want: u32| {
        seen.iter()
            .find(|(blocks, _, _)| *blocks == want)
            .copied()
            .expect("measured")
    };

    let (_, coarse_below, every_below) = find(BELOW);
    assert!(
        every_below.as_secs_f64() < coarse_below.as_secs_f64() * (1.0 - MARGIN),
        "at {BELOW} blocks every-block is {every_below:?} and the list is {coarse_below:?}: \
         the list is no longer the more expensive of the two, so COARSE_FROM is too high"
    );

    let (_, coarse_above, every_above) = find(ABOVE);
    assert!(
        coarse_above.as_secs_f64() < every_above.as_secs_f64() * (1.0 - MARGIN),
        "at {ABOVE} blocks the list is {coarse_above:?} and every-block is {every_above:?}: \
         the list is no longer the cheaper of the two, so COARSE_FROM is too low"
    );
}
