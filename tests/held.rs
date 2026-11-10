#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use grit::compact::Plan;
use grit::layout::{BINDINGS, BRUSH, GRAND, PARAMS, PIXELS, STATE};
use grit::sand::{self, AIR, Grid, WALL, WATER};
use grit::sweep;
use grit::{Groups, Resident};

const POOL: Grid = Grid::of(512, 256);

const WIDTHS: [u32; 5] = [4, 8, 16, 32, 64];

const FRAMES: u32 = 200;

#[derive(Clone, Copy)]
enum Side {
    Right,
    Left,
}

#[derive(Clone, Copy)]
struct Setup {
    label: &'static str,
    side: Side,
    coarse: bool,
    in_flight: bool,
}

/// The cell of standing water and the wall beside it. The wall shares the
/// top row of the cell's phase-1 quad with it, and the air on the other side
/// shares the bottom row of its phase-0 quad.
fn held_at(grid: Grid, side: Side) -> ((u32, u32), (u32, u32)) {
    let row = grid.deep - 9;
    match side {
        Side::Right => ((grid.wide / 2 + 1, row), (grid.wide / 2 + 2, row)),
        Side::Left => ((grid.wide / 2 - 2, row), (grid.wide / 2 - 3, row)),
    }
}

/// The pool of `tests/listed.rs` at a size every subgroup width can plan:
/// walls on three sides, still water on the floor, and one cell of unheld
/// water standing on it between a wall and air.
fn pool(grid: Grid, side: Side) -> Vec<u32> {
    let (wide, deep) = (grid.wide, grid.deep);
    let ((column, row), (wall, _)) = held_at(grid, side);
    let mut cells = vec![AIR; grid.cells()];
    for down in 0..deep {
        for across in 0..wide {
            let edge = down + 1 >= deep || across == 0 || across + 1 >= wide;
            cells[(down * wide + across) as usize] = if edge {
                WALL
            } else if down > row {
                WATER
            } else {
                AIR
            };
        }
    }
    cells[(row * wide + column) as usize] = WATER;
    cells[(row * wide + wall) as usize] = WALL;
    cells
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

fn plan(width: u32) -> Plan {
    Plan::over(width, POOL.quads().div_ceil(sweep::cells(width))).unwrap()
}

/// Steps the listed engine and the whole-grid step side by side over the
/// pool, tick then settle with the brush idle, and compares the two worlds
/// after every frame.
fn against_the_whole_grid(setup: Setup) {
    let Some(host) = device(setup.label) else {
        return;
    };
    let width = host.width();
    let grid = POOL;
    let plan = plan(width);
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

    let start = pool(grid, setup.side);
    for held in [&mut live, &mut full] {
        clear_all(held, &sizes);
        held.write(STATE as usize, &start).unwrap();
        held.write(PARAMS as usize, &[u32::MAX, plan.dump()])
            .unwrap();
        held.write_at(PARAMS as usize, BRUSH, &[0, 0, 0, AIR])
            .unwrap();
    }
    grit::live::seed(&mut live, grid, &plan).unwrap();

    let state = (STATE as usize, cells);
    let grand = (GRAND as usize, 1);
    let flying = if setup.in_flight { 2 } else { 1 };
    let mut frame = 0;
    while frame < FRAMES {
        let mut tickets = Vec::with_capacity(flying);
        for _ in 0..flying {
            tickets.push(
                live.submit_reading_both(&wired_groups, state, grand)
                    .unwrap(),
            );
        }
        for ticket in tickets {
            let (_, there, _) = live.wait_reading_both(ticket).unwrap();
            let (_, here) = full
                .staged_reading(&whole_stages, STATE as usize, cells)
                .unwrap();
            differ(frame, grid, &there, &here);
            frame += 1;
        }
    }
    println!(
        "{} {} sg={width} grid={}x{} coarse={} frames={frame}: AGREED",
        setup.label,
        host.name(),
        grid.wide,
        grid.deep,
        setup.coarse
    );
}

#[test]
fn the_pool_fits_a_plan_at_every_subgroup_width() {
    for width in WIDTHS {
        let blocks = POOL.quads().div_ceil(sweep::cells(width));
        assert!(
            Plan::over(width, blocks).is_ok(),
            "{}x{} does not fit at width {width}",
            POOL.wide,
            POOL.deep
        );
    }
}

#[test]
fn the_held_cell_stands_between_a_wall_and_air_on_still_water() {
    for side in [Side::Right, Side::Left] {
        let cells = pool(POOL, side);
        let ((column, row), (wall, _)) = held_at(POOL, side);
        let at = |x: u32, y: u32| cells[(y * POOL.wide + x) as usize];
        let open = 2 * column - wall;
        assert_eq!(at(column, row), WATER);
        assert_eq!(at(wall, row), WALL);
        assert_eq!(at(open, row), AIR);
        assert_eq!(at(column, row + 1), WATER);
        assert_eq!(at(column, row - 1), AIR);
        assert_eq!(row % 2, 1);
        assert_eq!(column.min(wall) % 2, 1);
        assert!(column.max(wall) + 1 < POOL.wide && column.min(wall) > 0);
    }
}

#[test]
fn water_held_beside_a_sleeping_quad_is_woken_at_every_width() {
    against_the_whole_grid(Setup {
        label: "held small",
        side: Side::Right,
        coarse: false,
        in_flight: false,
    });
}

#[test]
fn water_held_beside_a_sleeping_quad_is_woken_at_every_width_on_the_coarse_tier() {
    against_the_whole_grid(Setup {
        label: "held small coarse",
        side: Side::Right,
        coarse: true,
        in_flight: false,
    });
}

#[test]
fn water_held_against_a_wall_on_its_left_is_woken_at_every_width() {
    against_the_whole_grid(Setup {
        label: "held small left",
        side: Side::Left,
        coarse: false,
        in_flight: false,
    });
}

#[test]
fn water_held_against_a_wall_on_its_left_is_woken_on_the_coarse_tier() {
    against_the_whole_grid(Setup {
        label: "held small left coarse",
        side: Side::Left,
        coarse: true,
        in_flight: false,
    });
}

#[test]
fn water_held_with_two_frames_in_flight_is_woken_at_every_width() {
    against_the_whole_grid(Setup {
        label: "held small in flight",
        side: Side::Right,
        coarse: false,
        in_flight: true,
    });
}

#[test]
fn water_held_with_two_frames_in_flight_is_woken_on_the_coarse_tier() {
    against_the_whole_grid(Setup {
        label: "held small in flight coarse",
        side: Side::Right,
        coarse: true,
        in_flight: true,
    });
}
