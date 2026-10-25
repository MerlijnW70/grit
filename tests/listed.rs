#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use grit::compact::Plan;
use grit::layout::{BINDINGS, BRUSH, GRAND, PARAMS, PIXELS, STATE};
use grit::sand::{self, AIR, Grid, SAND, WALL, WATER};
use grit::sweep;
use grit::{Groups, Host, Resident};

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

type Brushing = fn(u32, Grid) -> [u32; 4];
type Starting = fn(Grid) -> Vec<u32>;

#[derive(Clone, Copy)]
struct Setup {
    label: &'static str,
    grid: Grid,
    coarse: bool,
    frames: u32,
    start: Starting,
    brushing: Brushing,
    in_flight: bool,
}

/// The room of `tests/live.rs`: walls on three sides and a shelf.
fn room(grid: Grid) -> Vec<u32> {
    let mut cells = vec![AIR; grid.cells()];
    for row in 0..grid.deep {
        for column in 0..grid.wide {
            let edge = row + 1 >= grid.deep || column == 0 || column + 1 >= grid.wide;
            let shelf = (170..178).contains(&row) && (140..240).contains(&column);
            if let Some(slot) = cells.get_mut((row * grid.wide + column) as usize) {
                *slot = if edge || shelf { WALL } else { AIR };
            }
        }
    }
    cells
}

/// A walled room with a still layer of water on its floor, and one cell of
/// water standing on that layer with a wall to its right and air to its
/// left. None of the water is held yet.
///
/// In frame 1 the quad of phase 1 that holds that cell in its top row finds
/// water under it and sets its held bit; the wall beside it keeps it from
/// starting, so that bit is the only change the quad makes. The quad of
/// phase 0 over the same cell, with the air on its left, is then the one that
/// starts it running in frame 2, and only if something woke it.
fn pool(grid: Grid) -> Vec<u32> {
    let mut cells = vec![AIR; grid.cells()];
    let (wide, deep) = (grid.wide, grid.deep);
    let (column, row) = (wide / 2 + 1, deep - 9);
    for down in 0..deep {
        for across in 0..wide {
            let edge = down + 1 >= deep || across == 0 || across + 1 >= wide;
            let layer = down > row;
            let at = (down * wide + across) as usize;
            cells[at] = if edge {
                WALL
            } else if layer {
                WATER
            } else {
                AIR
            };
        }
    }
    cells[(row * wide + column) as usize] = WATER;
    cells[(row * wide + column + 1) as usize] = WALL;
    cells
}

/// The pour of `tests/live.rs`, started at frame 10 instead of frame 0, when
/// `seed` no longer keeps every quad awake.
fn late(frame: u32, _grid: Grid) -> [u32; 4] {
    let material = match frame {
        10..100 => SAND,
        100..190 => WATER,
        _ => AIR,
    };
    [190, 30, 36, material]
}

/// A brush that moves three cells to the right every frame, sand and water
/// in turns, and then stops.
fn moving(frame: u32, grid: Grid) -> [u32; 4] {
    let span = grid.wide - 80;
    let x = 40 + (3 * frame) % span;
    let material = match frame {
        0..400 if (frame / 40).is_multiple_of(2) => SAND,
        0..400 => WATER,
        _ => AIR,
    };
    [x, 30, 25, material]
}

fn still(_frame: u32, _grid: Grid) -> [u32; 4] {
    [0, 0, 0, AIR]
}

fn clear_all(held: &mut Resident<'_>, sizes: &[usize; BINDINGS as usize]) {
    for (binding, &words) in sizes.iter().enumerate() {
        if binding == PIXELS as usize {
            continue;
        }
        held.write(binding, &vec![0_u32; words]).unwrap();
    }
}

fn differ(frame: u32, grid: Grid, listed: &[u32], whole: &[u32]) {
    if let Some(at) = listed.iter().zip(whole).position(|(a, b)| a != b) {
        let (row, column) = (at as u32 / grid.wide, at as u32 % grid.wide);
        let count = listed.iter().zip(whole).filter(|(a, b)| a != b).count();
        panic!(
            "frame {frame}: cell ({column},{row}) listed {:#x} whole {:#x}, {count} cells differ",
            listed[at], whole[at]
        );
    }
}

/// Runs the listed engine and the whole-grid step side by side, in the same
/// order (tick, brush, settle), and compares the two worlds after every
/// frame. With `in_flight`, the listed engine is driven two frames at a time,
/// both submitted before either is read. Returns the mean listed quads per
/// frame.
fn against_the_whole_grid(setup: Setup) -> Option<u64> {
    let host = device(setup.label)?;
    let width = host.width();
    let grid = setup.grid;
    let quads = grid.quads();
    let plan = match Plan::over(width, quads.div_ceil(sweep::cells(width))) {
        Ok(plan) => plan,
        Err(too_big) => {
            eprintln!("SKIPPED {} {width} {too_big}", setup.label);
            return None;
        }
    };
    let cells = grid.cells();

    let (wired, wired_stages) =
        grit::live::arranged(width, grid, &plan, None, setup.coarse).unwrap();
    let wired_groups: Vec<&[Groups]> = wired_stages.iter().map(Vec::as_slice).collect();

    let whole = [
        grit::live::tick(width).unwrap(),
        sand::brush(width, grid, false).unwrap(),
        sand::settle(width, grid).unwrap(),
    ];
    let whole_groups = [
        Groups::Fixed(1),
        Groups::Fixed(sand::BOX * sand::BOX / width),
        Groups::Fixed(grid.quads().div_ceil(width)),
    ];
    let whole_stages: Vec<&[Groups]> = whole_groups.iter().map(core::slice::from_ref).collect();

    let sizes = plan.sizes();
    let modules: Vec<&[u32]> = wired.iter().map(Vec::as_slice).collect();
    let mut live = host.resident(&sizes, &modules).unwrap();
    let modules: Vec<&[u32]> = whole.iter().map(Vec::as_slice).collect();
    let mut full = host.resident(&sizes, &modules).unwrap();

    let start = (setup.start)(grid);
    for held in [&mut live, &mut full] {
        clear_all(held, &sizes);
        held.write(STATE as usize, &start).unwrap();
        held.write(PARAMS as usize, &[u32::MAX, plan.dump()])
            .unwrap();
    }
    grit::live::seed(&mut live, grid, &plan).unwrap();

    let state = (STATE as usize, cells);
    let grand = (GRAND as usize, 1);
    let flying = if setup.in_flight { 2 } else { 1 };
    let mut listed_total = 0_u64;
    let mut frame = 0;
    while frame < setup.frames {
        let brush = (setup.brushing)(frame, grid);
        for held in [&mut live, &mut full] {
            held.write_at(PARAMS as usize, BRUSH, &brush).unwrap();
        }
        let mut tickets = Vec::with_capacity(flying);
        for _ in 0..flying {
            tickets.push(
                live.submit_reading_both(&wired_groups, state, grand)
                    .unwrap(),
            );
        }
        for ticket in tickets {
            let (_, there, listed) = live.wait_reading_both(ticket).unwrap();
            let (_, here) = full
                .staged_reading(&whole_stages, STATE as usize, cells)
                .unwrap();
            differ(frame, grid, &there, &here);
            listed_total += u64::from(listed.first().copied().unwrap_or(0));
            frame += 1;
        }
    }

    let mean = listed_total / u64::from(frame);
    println!(
        "{} {} sg={width} grid={}x{} coarse={} frames={frame}: AGREED, mean listed {mean} of {quads}",
        setup.label,
        host.name(),
        grid.wide,
        grid.deep,
        setup.coarse
    );
    assert!(mean < u64::from(quads));
    Some(mean)
}

const ROOM: Grid = Grid::of(512, 256);
const BIG: Grid = Grid::of(2048, 1024);

fn setup(label: &'static str, brushing: Brushing, coarse: bool) -> Setup {
    Setup {
        label,
        grid: ROOM,
        coarse,
        frames: 600,
        start: room,
        brushing,
        in_flight: false,
    }
}

fn held(label: &'static str, coarse: bool) -> Setup {
    Setup {
        grid: BIG,
        frames: 200,
        start: pool,
        ..setup(label, still, coarse)
    }
}

#[test]
fn a_brush_that_starts_late_paints_the_world_the_whole_grid_paints() {
    against_the_whole_grid(setup("late", late, false));
}

#[test]
fn a_brush_that_starts_late_agrees_on_the_coarse_tier_too() {
    against_the_whole_grid(setup("late coarse", late, true));
}

#[test]
fn a_brush_that_moves_every_frame_paints_the_world_the_whole_grid_paints() {
    against_the_whole_grid(setup("moving", moving, false));
}

#[test]
fn a_brush_that_moves_every_frame_agrees_on_the_coarse_tier_too() {
    against_the_whole_grid(setup("moving coarse", moving, true));
}

#[test]
fn water_held_beside_a_sleeping_quad_is_woken() {
    against_the_whole_grid(held("held", false));
}

#[test]
fn water_held_beside_a_sleeping_quad_is_woken_on_the_coarse_tier_too() {
    against_the_whole_grid(held("held coarse", true));
}

#[test]
fn two_frames_in_flight_give_the_world_of_the_whole_grid() {
    against_the_whole_grid(Setup {
        in_flight: true,
        ..setup("in flight", moving, false)
    });
}

#[test]
fn two_frames_in_flight_agree_on_the_coarse_tier_too() {
    against_the_whole_grid(Setup {
        in_flight: true,
        ..setup("in flight coarse", moving, true)
    });
}
