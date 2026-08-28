#![allow(clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::time::Duration;

use grit::compact::Plan;
use grit::layout::{BRUSH, GRAND, PARAMS, STATE};
use grit::live::{COARSE_FROM, coarse_pays};
use grit::sand::{AIR, Grid, SAND, WALL};
use grit::{Groups, Host, sweep};

const FRAMES: u32 = 300;

const BATCH: u32 = 20;

const POURING: u32 = 200;

const BELOW: u32 = 1024;

const ABOVE: u32 = 4096;

const ASKED_ABOVE: f64 = 0.25;

const ASKING: u32 = 20;

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
    let stepped = resident
        .read(GRAND as usize, 1)
        .expect("grand")
        .first()
        .copied()
        .unwrap_or(0);
    (plan.blocks, stepped, card)
}

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

fn best(host: &Host, width: u32, grid: Grid, coarse: bool) -> (u32, u32, Duration) {
    let first = cost(host, width, grid, coarse);
    let again = cost(host, width, grid, coarse);
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

    if std::env::var_os("CI").is_some() {
        eprintln!("  the threshold itself is a measurement and CI is set; it is not checked here");
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
        eprintln!("  COARSE_FROM is {COARSE_FROM} and this device agrees at every size measured");
    } else {
        eprintln!(
            "  COARSE_FROM is {COARSE_FROM} and this device wants the other arrangement at {disagrees:?} blocks \
             — see decisions/DR-0019 and grit::live::cheaper"
        );
    }

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
