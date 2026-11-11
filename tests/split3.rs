#![cfg(feature = "faults")]
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use grit::compact::Plan;
use grit::layout::{BINDINGS, DENSE, Dimension, GRAND, PARAMS, PIXELS, STATE, TALLY, Windows};
use grit::live;
use grit::render3::{self, Build, Camera, Light, Material, Screen, Steps, Table, scene_words};
use grit::rule3::sand::Sand;
use grit::sand::{AIR, Grid, SAND, WALL, WATER};
use grit::sand3::{Grid3, settle_reference3};
use grit::view3::Kinds;
use grit::{Groups, Host, Resident, sweep};

const SIDE: u32 = 32;

const FRAMES: u32 = 12;

fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut mixed = *state;
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    mixed ^ (mixed >> 31)
}

fn word(state: &mut u64) -> u32 {
    let roll = next(state);
    match roll % 20 {
        0..10 => AIR,
        10..14 => WATER + ((((roll >> 8) % 128) as u32) << 2),
        14..18 => SAND + ((((roll >> 8) % 128) as u32) << 2),
        _ => WALL,
    }
}

fn world(cells: usize, seed: u64) -> Vec<u32> {
    let mut state = seed;
    (0..cells)
        .map(|_| {
            if next(&mut state).is_multiple_of(4) {
                word(&mut state)
            } else {
                AIR
            }
        })
        .collect()
}

fn plan_of(width: u32, grid: Grid) -> Plan {
    Plan::over_in(
        width,
        grid.blocks().div_ceil(sweep::cells(width)),
        Dimension::Three,
    )
    .unwrap()
}

fn table() -> Table {
    Table::empty()
        .with(SAND, Material::solid(0xC8_A060, 0.2))
        .with(WATER, Material::solid(0x30_60C0, 0.05).clear(0.8))
        .with(WALL, Material::solid(0x80_8080, 0.1))
}

/// What a frame leaves: the world, the whole of `PIXELS`, the tally and the
/// list of octs the next frame steps.
#[derive(Debug, PartialEq, Eq)]
struct Left {
    state: Vec<u32>,
    pixels: Vec<u32>,
    tally: Vec<u32>,
    listed: Vec<u32>,
}

/// The listed engine of `plan` with the whole picture after it, from
/// `start`, writing a few cells every third frame: what each frame leaves.
fn ran(host: &Host, width: u32, plan: Plan, coarse: bool, start: &[u32]) -> Vec<Left> {
    let grid = Grid::solid(SIDE, SIDE, SIDE).unwrap();
    let windows = plan.windows();
    let kinds = Kinds::named("sand").unwrap();
    let screen = Screen { wide: 40, high: 24 };
    let build = Build {
        grid,
        kinds,
        screen,
        subgroup: width,
        suns: 1,
        windows,
    };
    let layout = build.layout();
    let engine = live::arranged3(width, grid, &plan, coarse).unwrap();
    let (passes, mut stages) = render3::watching(engine, build.passes().unwrap()).unwrap();
    stages.extend(build.stages(Steps::ALL));
    let sizes = windows.laid(&layout.sized(live::sizes3(&Sand, &plan)));
    assert_eq!(sizes.len(), windows.bindings() as usize);
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let mut held = host.resident(&sizes, &modules).unwrap();
    for (binding, &words) in sizes.iter().enumerate() {
        held.write(binding, &vec![0; words]).unwrap();
    }
    live::write_state(&mut held, windows, 0, start).unwrap();
    held.write(PARAMS as usize, &[u32::MAX, plan.dump()])
        .unwrap();
    live::seed(&mut held, grid, &plan).unwrap();
    let table = table();
    held.write_at(PIXELS as usize, layout.table(), &table.words())
        .unwrap();
    let side = SIDE as f32;
    let rays = Camera::looking(
        [side * 1.4, -side * 0.6, side * 1.3],
        [side * 0.5, side * 0.6, side * 0.5],
        1.0,
    )
    .rays(grid, screen);
    let light = Light::evening(screen.high, 1.0);
    held.write_at(
        PIXELS as usize,
        layout.scene(),
        &scene_words(&rays, &table, &[[3, -9, 2]], &light),
    )
    .unwrap();

    let mut state = 0xF00D_u64;
    let mut left = Vec::new();
    for frame in 0..FRAMES {
        if frame % 3 == 1 {
            let mut writes: Vec<(u32, u32)> = (0..12)
                .map(|_| {
                    let cell = (next(&mut state) % grid.cells() as u64) as u32;
                    (cell, word(&mut state))
                })
                .collect();
            writes.push((grid.cells() as u32 - 1, SAND));
            writes.push((grid.cells() as u32 / 2, WATER));
            writes.push((grid.cells() as u32 / 2 - 1, SAND));
            writes.sort_unstable();
            writes.dedup_by_key(|write| write.0);
            live::put3(&mut held, grid, &plan, &writes).unwrap();
        }
        let groups: Vec<&[Groups]> = stages.iter().map(Vec::as_slice).collect();
        held.staged(&groups).unwrap();
        left.push(read(&mut held, windows, grid, &layout));
    }
    left
}

fn read(held: &mut Resident<'_>, windows: Windows, grid: Grid, layout: &render3::Layout) -> Left {
    let grand = held.read(GRAND as usize, 1).unwrap()[0];
    Left {
        state: live::read_state(held, windows, grid.cells()).unwrap(),
        pixels: held.read(PIXELS as usize, layout.words()).unwrap(),
        tally: held.read(TALLY as usize, 7).unwrap(),
        listed: held.read(DENSE as usize, grand as usize).unwrap(),
    }
}

#[test]
fn a_world_split_over_windows_steps_draws_and_takes_writes_as_one_binding_does() {
    let Some(host) =
        device("a_world_split_over_windows_steps_draws_and_takes_writes_as_one_binding_does")
    else {
        return;
    };
    let width = host.width();
    let grid = Grid::solid(SIDE, SIDE, SIDE).unwrap();
    let whole = plan_of(width, grid);
    assert!(!whole.windows().is_split());
    for coarse in [false, true] {
        let start = world(grid.cells(), 0x5EED);
        let one = ran(&host, width, whole, coarse, &start);
        for (bits, count) in [(14, 2), (13, 4)] {
            let split = whole.split_at(bits);
            assert_eq!(split.windows().count(), count);
            assert_eq!(split.windows().bindings(), BINDINGS + count - 1);
            let parts = ran(&host, width, split, coarse, &start);
            for (frame, (got, want)) in parts.iter().zip(&one).enumerate() {
                assert_eq!(
                    got.state, want.state,
                    "{count} windows, coarse {coarse}: frame {frame}: the world"
                );
                assert_eq!(
                    got.listed, want.listed,
                    "{count} windows, coarse {coarse}: frame {frame}: the list"
                );
                assert_eq!(
                    got.tally, want.tally,
                    "{count} windows, coarse {coarse}: frame {frame}: the tally"
                );
                let differ = got
                    .pixels
                    .iter()
                    .zip(&want.pixels)
                    .filter(|(a, b)| a != b)
                    .count();
                assert_eq!(
                    differ, 0,
                    "{count} windows, coarse {coarse}: frame {frame}: {differ} words of the picture"
                );
            }
            let moved = parts.first().unwrap().state != start;
            assert!(moved, "nothing moved");
            println!(
                "{} width {width} {SIDE}^3 coarse {coarse}: {count} windows of {} words equal one binding over {FRAMES} frames",
                host.name(),
                split.windows().span()
            );
        }
    }
}

#[test]
fn a_split_world_steps_as_the_cpu_does() {
    let Some(host) = device("a_split_world_steps_as_the_cpu_does") else {
        return;
    };
    let width = host.width();
    let grid = Grid::solid(SIDE, SIDE, SIDE).unwrap();
    let plan = plan_of(width, grid).split_at(12);
    let windows = plan.windows();
    assert_eq!(windows.count(), 8);
    let (passes, stages) = live::arranged3(width, grid, &plan, false).unwrap();
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let bound = plan.bound();
    let mut held = host.resident(&bound, &modules).unwrap();
    for (binding, &words) in bound.iter().enumerate() {
        if binding != PIXELS as usize {
            held.write(binding, &vec![0; words]).unwrap();
        }
    }
    let start = world(grid.cells(), 7);
    live::write_state(&mut held, windows, 0, &start).unwrap();
    for window in 0..windows.count() {
        let binding = Windows::binding(window) as usize;
        let span = windows.span();
        let back = held.read(binding, span).unwrap();
        let from = window as usize * span;
        assert_eq!(back, start[from..from + span], "window {window}");
    }
    held.write(PARAMS as usize, &[u32::MAX, plan.dump()])
        .unwrap();
    live::seed(&mut held, grid, &plan).unwrap();
    let reference = Grid3::new(SIDE, SIDE, SIDE).unwrap();
    let mut want = start;
    for frame in 0..FRAMES {
        let groups: Vec<&[Groups]> = stages.iter().map(Vec::as_slice).collect();
        held.staged(&groups).unwrap();
        want = settle_reference3(reference, frame, &want);
        let got = live::read_state(&mut held, windows, grid.cells()).unwrap();
        let differ = (0..grid.cells()).filter(|&at| got[at] != want[at]).count();
        assert_eq!(differ, 0, "frame {frame}: {differ} cells differ");
    }
}

#[test]
fn windows_split_by_index_and_lay_the_last_one_short() {
    let windows = Windows::split_at(10_000, 12);
    assert_eq!(windows.count(), 3);
    assert_eq!(
        (0..3).map(|at| windows.held(at)).collect::<Vec<_>>(),
        [4096, 4096, 1808]
    );
    assert_eq!(windows.held(3), 0);
    assert_eq!(windows.place(0), (STATE, 0));
    assert_eq!(windows.place(4095), (STATE, 4095));
    assert_eq!(windows.place(4096), (BINDINGS, 0));
    assert_eq!(windows.place(9999), (BINDINGS + 1, 9999 - 8192));
    let laid = windows.laid(&[3; BINDINGS as usize]);
    assert_eq!(laid.len(), BINDINGS as usize + 2);
    assert_eq!(laid[STATE as usize], 4096);
    assert_eq!(laid[BINDINGS as usize..], [4096, 1808]);

    let under = Windows::split_at(8191, 12);
    assert!(!under.is_split());
    assert_eq!(under.place(8000), (STATE, 8000));
    assert_eq!(under.laid(&[3; BINDINGS as usize]).len(), BINDINGS as usize);
    assert_eq!(Windows::split_at(8192, 12).count(), 2);
}

#[test]
fn a_flat_frame_refuses_a_split_plan() {
    let grid = Grid::new(64, 64).unwrap();
    let width = 8;
    let plan = Plan::over(width, grid.quads().div_ceil(sweep::cells(width))).unwrap();
    assert!(live::arranged(width, grid, &plan, None, false).is_ok());
    let split = plan.split_at(8);
    assert!(split.windows().is_split());
    assert!(matches!(
        live::arranged(width, grid, &split, None, false),
        Err(simdr::lanes::LaneError::NoSuchForm { because, .. }) if because == live::SPLIT_FLAT
    ));
}
