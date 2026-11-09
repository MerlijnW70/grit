#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use grit::Groups;
use grit::compact::Plan;
use grit::layout::{BRUSH, GRAND, PARAMS, STATE};
use grit::sand::{self, AIR, Grid, SAND, WALL, WATER};
use grit::sweep;

type Brushing = fn(u32, Grid) -> [u32; 4];

/// A walled room with a shelf across its middle.
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

/// A brush that pours from frame 10, once `seed` no longer keeps every quad
/// awake: sand, then water, then nothing.
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

/// The frame a caller who brings its own middle builds from `keeping`:
/// the tick, the halves before, the waking brush, the listed settle, the
/// halves after. Nothing is added to what `keeping` hands over.
fn kept(width: u32, grid: Grid, plan: &Plan, coarse: bool) -> grit::live::Loop {
    let keeping = grit::live::keeping(width, plan, coarse).unwrap();
    let mut stages: grit::live::Stages =
        vec![vec![(grit::live::tick(width).unwrap(), Groups::Fixed(1))]];
    stages.extend(keeping.before);
    stages.push(vec![(
        sand::brush(width, grid, true).unwrap(),
        Groups::Counting(sand::BOX * sand::BOX / width),
    )]);
    stages.push(vec![(
        grit::live::settle(width, grid).unwrap(),
        keeping.over,
    )]);
    stages.extend(keeping.after);
    grit::live::staged(stages)
}

fn against_the_whole_grid(label: &str, brushing: Brushing, coarse: bool) {
    let Some(host) = device(label) else {
        return;
    };
    let width = host.width();
    let grid = Grid::of(512, 256);
    let frames = 600;
    let quads = grid.quads();
    let cells = grid.cells();
    let plan = Plan::over(width, quads.div_ceil(sweep::cells(width))).unwrap();

    let (wired, wired_stages) = kept(width, grid, &plan, coarse);
    let wired_groups: Vec<&[Groups]> = wired_stages.iter().map(Vec::as_slice).collect();

    let whole = [
        grit::live::tick(width).unwrap(),
        sand::brush(width, grid, false).unwrap(),
        sand::settle(width, grid).unwrap(),
    ];
    let whole_groups = [
        Groups::Fixed(1),
        Groups::Fixed(sand::BOX * sand::BOX / width),
        Groups::Fixed(quads.div_ceil(width)),
    ];
    let whole_stages: Vec<&[Groups]> = whole_groups.iter().map(core::slice::from_ref).collect();

    let sizes = plan.sizes();
    let modules: Vec<&[u32]> = wired.iter().map(Vec::as_slice).collect();
    let mut live = host.resident(&sizes, &modules).unwrap();
    let modules: Vec<&[u32]> = whole.iter().map(Vec::as_slice).collect();
    let mut full = host.resident(&sizes, &modules).unwrap();

    let start = room(grid);
    for held in [&mut live, &mut full] {
        for (binding, &words) in sizes.iter().enumerate() {
            held.write(binding, &vec![0_u32; words]).unwrap();
        }
        held.write(STATE as usize, &start).unwrap();
        held.write(PARAMS as usize, &[u32::MAX, plan.dump()])
            .unwrap();
    }
    grit::live::seed(&mut live, grid, &plan).unwrap();

    let mut listed_total = 0_u64;
    for frame in 0..frames {
        let brush = brushing(frame, grid);
        for held in [&mut live, &mut full] {
            held.write_at(PARAMS as usize, BRUSH, &brush).unwrap();
        }
        let (_, there, listed) = live
            .staged_reading_both(&wired_groups, (STATE as usize, cells), (GRAND as usize, 1))
            .unwrap();
        let (_, here) = full
            .staged_reading(&whole_stages, STATE as usize, cells)
            .unwrap();
        if let Some(at) = there.iter().zip(&here).position(|(a, b)| a != b) {
            let count = there.iter().zip(&here).filter(|(a, b)| a != b).count();
            panic!(
                "{label} frame {frame}: cell ({},{}) listed {:#x} whole {:#x}, {count} cells differ",
                at as u32 % grid.wide,
                at as u32 / grid.wide,
                there[at],
                here[at]
            );
        }
        listed_total += u64::from(listed.first().copied().unwrap_or(0));
    }

    let mean = listed_total / u64::from(frames);
    println!(
        "{label} {} sg={width} coarse={coarse} frames={frames}: AGREED, mean listed {mean} of {quads}",
        host.name()
    );
    assert!(mean < u64::from(quads));
}

#[test]
fn a_frame_built_from_keeping_follows_a_moving_brush() {
    against_the_whole_grid("keeping moving", moving, false);
}

#[test]
fn a_frame_built_from_keeping_follows_a_moving_brush_on_the_coarse_tier() {
    against_the_whole_grid("keeping moving coarse", moving, true);
}

#[test]
fn a_frame_built_from_keeping_follows_a_late_brush() {
    against_the_whole_grid("keeping late", late, false);
}

#[test]
fn a_frame_built_from_keeping_follows_a_late_brush_on_the_coarse_tier() {
    against_the_whole_grid("keeping late coarse", late, true);
}
