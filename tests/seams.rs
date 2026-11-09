#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use grit::compact::Plan;
use grit::layout::{BINDINGS, BRUSH, DENSE, GRAND, PARAMS, PIXELS, STATE};
use grit::sand::{self, AIR, Grid, SAND, WALL, WATER};
use grit::sweep;
use grit::{Groups, Resident};

type Brushing = fn(u32, Grid) -> [u32; 4];
type Starting = fn(Grid) -> Vec<u32>;
type Crossed = fn(Grid, &[u32], &[u32]) -> bool;

#[derive(Clone, Copy)]
struct Setup {
    label: &'static str,
    coarse: bool,
    frames: u32,
    start: Starting,
    brushing: Brushing,
    crossed: Crossed,
}

const OPEN: Grid = Grid::of(512, 256);

/// A floor along the bottom row only: no side walls, so whatever spreads
/// sideways crosses the seam between the last column and the first.
fn floor_only(grid: Grid) -> Vec<u32> {
    let mut cells = vec![AIR; grid.cells()];
    for column in 0..grid.wide {
        cells[((grid.deep - 1) * grid.wide + column) as usize] = WALL;
    }
    cells
}

/// No wall at all: whatever falls through the bottom row comes back in at the
/// top row.
fn nothing(grid: Grid) -> Vec<u32> {
    vec![AIR; grid.cells()]
}

/// Sand and water standing on both sides of the side seam, on a floor.
fn blob_on_side_seam(grid: Grid) -> Vec<u32> {
    let mut cells = floor_only(grid);
    for row in 180..250 {
        for column in (grid.wide - 30)..(grid.wide + 30) {
            let column = column % grid.wide;
            cells[(row * grid.wide + column) as usize] = if row < 200 { SAND } else { WATER };
        }
    }
    cells
}

/// Sand and water straddling the bottom and top rows, with no floor.
fn blob_on_floor_seam(grid: Grid) -> Vec<u32> {
    let mut cells = nothing(grid);
    for row in (grid.deep - 20)..(grid.deep + 20) {
        let row = row % grid.deep;
        for column in 100..160 {
            cells[(row * grid.wide + column) as usize] = if column < 130 { SAND } else { WATER };
        }
    }
    cells
}

/// Water, then sand, poured just right of the left seam, so it spreads both
/// ways.
fn at_the_side_seam(frame: u32, _grid: Grid) -> [u32; 4] {
    let material = match frame {
        5..150 => WATER,
        150..200 => SAND,
        _ => AIR,
    };
    [3, 200, 36, material]
}

/// Sand, then water, poured near the bottom with no floor: it falls through
/// the bottom seam and keeps falling around the torus.
fn above_the_floor_seam(frame: u32, _grid: Grid) -> [u32; 4] {
    let material = match frame {
        5..60 => SAND,
        60..120 => WATER,
        _ => AIR,
    };
    [256, 240, 25, material]
}

/// Sand, then water, poured at the corner where both seams meet, with no wall
/// at all.
fn at_the_corner(frame: u32, _grid: Grid) -> [u32; 4] {
    let material = match frame {
        5..60 => SAND,
        60..120 => WATER,
        _ => AIR,
    };
    [2, 253, 36, material]
}

fn mix(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
}

/// Every fourth frame, in two of three, a small blob of sand or water at a
/// place chosen by `seed` within twelve cells of the corner where both seams
/// meet. The corner is where a quad that wakes its neighbours has to wrap in
/// both directions at once.
fn scattered_at_the_corner_of(seed: u32, frame: u32, grid: Grid) -> [u32; 4] {
    let draw = mix(seed.wrapping_mul(0x9e37_79b9) ^ (frame / 4));
    if !frame.is_multiple_of(4) || !(40..340).contains(&frame) || draw.is_multiple_of(3) {
        return [0, 0, 0, AIR];
    }
    let side = mix(draw ^ 1) % 24;
    let x = if side < 12 {
        side
    } else {
        grid.wide - (side - 11)
    };
    let y = (250 + mix(draw ^ 2) % 12) % grid.deep;
    let reach = 1 + mix(draw ^ 3) % 10;
    let material = if mix(draw ^ 4).is_multiple_of(2) {
        SAND
    } else {
        WATER
    };
    [x, y, reach, material]
}

fn scattered_at_the_corner<const SEED: u32>(frame: u32, grid: Grid) -> [u32; 4] {
    scattered_at_the_corner_of(SEED, frame, grid)
}

fn still(_frame: u32, _grid: Grid) -> [u32; 4] {
    [0, 0, 0, AIR]
}

fn matter(cell: u32) -> bool {
    cell == SAND || cell == WATER
}

fn some_matter(
    grid: Grid,
    world: &[u32],
    rows: core::ops::Range<u32>,
    columns: core::ops::Range<u32>,
) -> bool {
    rows.into_iter().any(|row| {
        columns.clone().any(|column| {
            world
                .get((row * grid.wide + column) as usize)
                .copied()
                .is_some_and(matter)
        })
    })
}

/// Something has crossed the side seam: sand or water in the last eight
/// columns, which the brush at column 3 cannot reach.
fn over_the_side_seam(grid: Grid, _start: &[u32], world: &[u32]) -> bool {
    some_matter(grid, world, 0..grid.deep, grid.wide - 8..grid.wide)
}

/// Something has crossed the bottom seam: sand or water in the first eight
/// rows, which the brush at row 240 cannot reach.
fn over_the_floor_seam(grid: Grid, _start: &[u32], world: &[u32]) -> bool {
    some_matter(grid, world, 0..8, 0..grid.wide)
}

/// Something has crossed both seams: sand or water in the last eight columns
/// of the first eight rows, which the brush at the corner cannot reach.
fn over_both_seams(grid: Grid, _start: &[u32], world: &[u32]) -> bool {
    some_matter(grid, world, 0..8, grid.wide - 8..grid.wide)
}

/// The world has changed since the start in the first or last row or column.
fn moved_at_the_seam(grid: Grid, start: &[u32], world: &[u32]) -> bool {
    let edge = |column: u32, row: u32| {
        let at = (row * grid.wide + column) as usize;
        start.get(at) != world.get(at)
    };
    (0..grid.deep).any(|row| edge(0, row) || edge(grid.wide - 1, row))
        || (0..grid.wide).any(|column| edge(column, 0) || edge(column, grid.deep - 1))
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

fn kept_inside(frame: u32, quads: u32, list: &[u32]) {
    let mut seen = vec![false; quads as usize];
    for &quad in list {
        assert!(
            quad < quads,
            "frame {frame}: the list holds quad {quad}, outside a grid of {quads} quads"
        );
        let again = std::mem::replace(&mut seen[quad as usize], true);
        assert!(!again, "frame {frame}: the list holds quad {quad} twice");
    }
}

/// Runs the listed engine and the whole-grid step side by side, in the same
/// order (tick, brush, settle), compares the two worlds after every frame, and
/// checks that the list holds only quads of the grid, each once. The scenario
/// must also have carried something over the seam it is about.
fn against_the_whole_grid(setup: Setup) -> Option<u64> {
    let host = device(setup.label)?;
    let width = host.width();
    let grid = OPEN;
    let quads = grid.quads();
    let plan = match Plan::over(width, quads.div_ceil(sweep::cells(width))) {
        Ok(plan) => plan,
        Err(too_big) => {
            common::skip(format_args!("{} {width} {too_big}", setup.label));
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
    let mut listed_total = 0_u64;
    let mut crossed = false;
    for frame in 0..setup.frames {
        let brush = (setup.brushing)(frame, grid);
        for held in [&mut live, &mut full] {
            held.write_at(PARAMS as usize, BRUSH, &brush).unwrap();
        }
        let ticket = live
            .submit_reading_both(&wired_groups, state, grand)
            .unwrap();
        let (_, there, listed) = live.wait_reading_both(ticket).unwrap();
        let (_, here) = full
            .staged_reading(&whole_stages, STATE as usize, cells)
            .unwrap();
        differ(frame, grid, &there, &here);

        let count = listed.first().copied().unwrap_or(0);
        assert!(
            count <= quads,
            "frame {frame}: the list counts {count} quads in a grid of {quads}"
        );
        let list = live.read(DENSE as usize, count as usize).unwrap();
        kept_inside(frame, quads, &list);

        listed_total += u64::from(count);
        crossed = crossed || (setup.crossed)(grid, &start, &here);
    }
    assert!(
        crossed,
        "{}: nothing crossed the seam, so the scenario proves nothing",
        setup.label
    );

    let mean = listed_total / u64::from(setup.frames);
    println!(
        "{} {} sg={width} grid={}x{} coarse={} frames={}: AGREED, mean listed {mean} of {quads}",
        setup.label,
        host.name(),
        grid.wide,
        grid.deep,
        setup.coarse,
        setup.frames
    );
    assert!(mean < u64::from(quads));
    Some(mean)
}

fn open(
    label: &'static str,
    start: Starting,
    brushing: Brushing,
    crossed: Crossed,
    coarse: bool,
) -> Setup {
    Setup {
        label,
        coarse,
        frames: 400,
        start,
        brushing,
        crossed,
    }
}

#[test]
fn water_crossing_the_side_seam_is_the_world_the_whole_grid_makes() {
    let setup = open(
        "side seam",
        floor_only,
        at_the_side_seam,
        over_the_side_seam,
        false,
    );
    against_the_whole_grid(setup);
}

#[test]
fn water_crossing_the_side_seam_agrees_on_the_coarse_tier_too() {
    let setup = open(
        "side seam coarse",
        floor_only,
        at_the_side_seam,
        over_the_side_seam,
        true,
    );
    against_the_whole_grid(setup);
}

#[test]
fn sand_falling_through_the_floor_seam_is_the_world_the_whole_grid_makes() {
    let setup = open(
        "floor seam",
        nothing,
        above_the_floor_seam,
        over_the_floor_seam,
        false,
    );
    against_the_whole_grid(setup);
}

#[test]
fn sand_falling_through_the_floor_seam_agrees_on_the_coarse_tier_too() {
    let setup = open(
        "floor seam coarse",
        nothing,
        above_the_floor_seam,
        over_the_floor_seam,
        true,
    );
    against_the_whole_grid(setup);
}

#[test]
fn a_pile_standing_on_the_side_seam_is_the_world_the_whole_grid_makes() {
    let setup = open(
        "side pile",
        blob_on_side_seam,
        still,
        moved_at_the_seam,
        false,
    );
    against_the_whole_grid(setup);
}

#[test]
fn a_pile_standing_on_the_side_seam_agrees_on_the_coarse_tier_too() {
    let setup = open(
        "side pile coarse",
        blob_on_side_seam,
        still,
        moved_at_the_seam,
        true,
    );
    against_the_whole_grid(setup);
}

#[test]
fn a_pile_standing_on_the_floor_seam_is_the_world_the_whole_grid_makes() {
    let setup = open(
        "floor pile",
        blob_on_floor_seam,
        still,
        moved_at_the_seam,
        false,
    );
    against_the_whole_grid(setup);
}

#[test]
fn a_pile_standing_on_the_floor_seam_agrees_on_the_coarse_tier_too() {
    let setup = open(
        "floor pile coarse",
        blob_on_floor_seam,
        still,
        moved_at_the_seam,
        true,
    );
    against_the_whole_grid(setup);
}

#[test]
fn a_pour_at_the_corner_of_both_seams_is_the_world_the_whole_grid_makes() {
    let setup = open("corner", nothing, at_the_corner, over_both_seams, false);
    against_the_whole_grid(setup);
}

#[test]
fn a_pour_at_the_corner_of_both_seams_agrees_on_the_coarse_tier_too() {
    let setup = open(
        "corner coarse",
        nothing,
        at_the_corner,
        over_both_seams,
        true,
    );
    against_the_whole_grid(setup);
}

/// The disc a brush paints, by the rule of `sand::brush`: a cell is inside
/// when its offset from the centre, measured on the page and not around the
/// torus, squares to less than the reach.
fn the_cut_disc(grid: Grid, centre: (u32, u32), reach: u32) -> Vec<u32> {
    let mut cells = vec![AIR; grid.cells()];
    for row in 0..grid.deep {
        for column in 0..grid.wide {
            let across = i64::from(column) - i64::from(centre.0);
            let down = i64::from(row) - i64::from(centre.1);
            if across * across + down * down < i64::from(reach) {
                cells[(row * grid.wide + column) as usize] = SAND;
            }
        }
    }
    cells
}

/// Paints one brush stroke on an empty open grid with the whole-grid brush and
/// returns the world.
fn one_stroke(label: &str, centre: (u32, u32), reach: u32) -> Option<(Grid, Vec<u32>)> {
    let host = device(label)?;
    let width = host.width();
    let grid = OPEN;
    let plan = match Plan::over(width, grid.quads().div_ceil(sweep::cells(width))) {
        Ok(plan) => plan,
        Err(too_big) => {
            common::skip(format_args!("{label} {width} {too_big}"));
            return None;
        }
    };
    let modules = [sand::brush(width, grid, false).unwrap()];
    let modules: Vec<&[u32]> = modules.iter().map(Vec::as_slice).collect();
    let sizes = plan.sizes();
    let mut held = host.resident(&sizes, &modules).unwrap();
    clear_all(&mut held, &sizes);
    held.write(PARAMS as usize, &[u32::MAX, plan.dump()])
        .unwrap();
    held.write_at(PARAMS as usize, BRUSH, &[centre.0, centre.1, reach, SAND])
        .unwrap();
    let groups = [Groups::Fixed(sand::BOX * sand::BOX / width)];
    let stages: Vec<&[Groups]> = vec![&groups];
    let (_, world) = held
        .staged_reading(&stages, STATE as usize, grid.cells())
        .unwrap();
    Some((grid, world))
}

fn the_disc_is_cut(label: &str, centre: (u32, u32), reach: u32) {
    let Some((grid, world)) = one_stroke(label, centre, reach) else {
        return;
    };
    let wanted = the_cut_disc(grid, centre, reach);
    if let Some(at) = world.iter().zip(&wanted).position(|(a, b)| a != b) {
        let (row, column) = (at as u32 / grid.wide, at as u32 % grid.wide);
        let count = world.iter().zip(&wanted).filter(|(a, b)| a != b).count();
        panic!(
            "{label}: cell ({column},{row}) painted {:#x}, the cut disc says {:#x}, {count} cells differ",
            world[at], wanted[at]
        );
    }
    let painted = world.iter().filter(|&&cell| cell == SAND).count();
    assert!(painted > 0, "{label}: the brush painted nothing");
}

/// The disc of a brush near the seam is cut, not wrapped: the cells across the
/// seam, which a wrapped disc would paint, stay empty. The world around it
/// does wrap, so this pins the current behaviour and does not require it: a
/// change that wraps the disc has to change this test and say why.
#[test]
fn the_brush_disc_is_cut_at_the_side_seam() {
    the_disc_is_cut("disc side", (2, 100), 36);
}

#[test]
fn the_brush_disc_is_cut_at_the_floor_seam() {
    the_disc_is_cut("disc floor", (200, 253), 36);
}

#[test]
fn the_brush_disc_is_cut_at_the_corner_of_both_seams() {
    the_disc_is_cut("disc corner", (509, 2), 36);
}

#[test]
fn blobs_scattered_at_the_corner_of_both_seams_draw_0_are_the_world_the_whole_grid_makes() {
    let setup = open(
        "scattered corner 0",
        nothing,
        scattered_at_the_corner::<0>,
        over_both_seams,
        false,
    );
    against_the_whole_grid(setup);
}

#[test]
fn blobs_scattered_at_the_corner_of_both_seams_draw_0_are_the_world_the_whole_grid_makes_on_the_coarse_tier_too()
 {
    let setup = open(
        "scattered corner 0 coarse",
        nothing,
        scattered_at_the_corner::<0>,
        over_both_seams,
        true,
    );
    against_the_whole_grid(setup);
}

#[test]
fn blobs_scattered_at_the_corner_of_both_seams_draw_1_are_the_world_the_whole_grid_makes() {
    let setup = open(
        "scattered corner 1",
        nothing,
        scattered_at_the_corner::<1>,
        over_both_seams,
        false,
    );
    against_the_whole_grid(setup);
}

#[test]
fn blobs_scattered_at_the_corner_of_both_seams_draw_1_are_the_world_the_whole_grid_makes_on_the_coarse_tier_too()
 {
    let setup = open(
        "scattered corner 1 coarse",
        nothing,
        scattered_at_the_corner::<1>,
        over_both_seams,
        true,
    );
    against_the_whole_grid(setup);
}
