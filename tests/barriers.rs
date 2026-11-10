#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use grit::compact::Plan;
use grit::layout::{BINDINGS, DENSE, HOT_INDIRECT, INDIRECT, PARAMS, PIXELS, STATE};
use grit::resident::indirect_barriers;
use grit::sand::{self, AIR, Grid, SAND, WALL, WATER};
use grit::sweep;
use grit::written::{Written, written};
use grit::{Groups, Host, Resident};
use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, U32};

const ROOM: Grid = Grid::of(256, 128);

fn plan_for(width: u32, grid: Grid) -> Plan {
    Plan::over(width, grid.quads().div_ceil(sweep::cells(width))).unwrap()
}

fn arrangements() -> Vec<(u32, Grid, Plan, bool)> {
    let mut every = Vec::new();
    for width in [4_u32, 8, 16, 32, 64] {
        for grid in [
            Grid::of(4, 4),
            Grid::of(16, 4),
            Grid::of(64, 64),
            ROOM,
            Grid::of(4096, 4),
            Grid::of(1024, 1024),
            Grid::of(4096, 4096),
        ] {
            let Ok(plan) = Plan::over(width, grid.quads().div_ceil(sweep::cells(width))) else {
                continue;
            };
            for coarse in [false, true] {
                every.push((width, grid, plan, coarse));
            }
        }
    }
    every
}

fn from(binding: u32) -> Groups {
    Groups::From {
        binding: binding as usize,
        at: 0,
    }
}

fn reads(stage: &[Groups], binding: u32) -> bool {
    stage.iter().any(
        |&groups| matches!(groups, Groups::From { binding: held, .. } if held == binding as usize),
    )
}

fn barriers_of(stages: &[Vec<Groups>], writes: &[Written], times: u32) -> Vec<(u32, usize)> {
    let refs: Vec<&[Groups]> = stages.iter().map(Vec::as_slice).collect();
    indirect_barriers(&refs, writes, times)
}

fn every_repeat(stages: &[usize], times: u32) -> Vec<(u32, usize)> {
    (0..times)
        .flat_map(|repeat| stages.iter().map(move |&at| (repeat, at)))
        .collect()
}

/// The coarse frame's indirect readers are the decay (stage 0, `HOT_INDIRECT`),
/// the brush (1, its count in `DENSE`), the settle (2, `INDIRECT`), the fine
/// mark and the fine gather (both `HOT_INDIRECT`). Only three need the bits:
/// the brush, whose count the first stage writes; the settle, whose count the
/// brush's claim writes; and the fine mark, whose arguments the coarse prepare
/// writes in the stage before it. The fine gather reads the same arguments,
/// which nothing writes after the fine mark's barrier, and the decay reads the
/// coarse prepare's arguments of the frame before, behind that same barrier
/// or the frame's first.
#[test]
fn the_coarse_frame_carries_the_indirect_bits_into_the_brush_the_settle_and_the_fine_mark() {
    let mut seen = 0;
    for (width, grid, plan, coarse) in arrangements() {
        if !coarse {
            continue;
        }
        let (passes, stages) = grit::live::arranged(width, grid, &plan, None, coarse).unwrap();
        let writes: Vec<Written> = passes.iter().map(|module| written(module)).collect();
        let label = format!("width {width} grid {}x{}", grid.wide, grid.deep);

        let last = stages.len() - 1;
        let fine_mark = (3..last)
            .find(|&at| reads(&stages[at], HOT_INDIRECT))
            .unwrap();
        assert!(reads(&stages[0], HOT_INDIRECT), "decay {label}");
        assert_eq!(stages[1], vec![sand::seats(&plan)], "brush {label}");
        assert_eq!(stages[2], vec![from(INDIRECT)], "settle {label}");
        assert!(reads(&stages[last], HOT_INDIRECT), "fine gather {label}");
        let readers = stages
            .iter()
            .filter(|stage| stage.iter().any(|g| matches!(g, Groups::From { .. })))
            .count();
        assert_eq!(readers, 5, "{label}");

        let (brush, settle) = (1, 2);
        for times in [1, 2, 3] {
            assert_eq!(
                barriers_of(&stages, &writes, times),
                every_repeat(&[brush, settle, fine_mark], times),
                "{label} times {times}"
            );
        }
        seen += 1;
    }
    assert!(seen >= 25, "{seen}");
}

/// The fine frame reads arguments in the brush and the settle only, and both
/// are written in the frame before them, so both carry the bits.
#[test]
fn the_fine_frame_carries_the_indirect_bits_into_the_brush_and_the_settle() {
    let mut seen = 0;
    for (width, grid, plan, coarse) in arrangements() {
        if coarse {
            continue;
        }
        let (passes, stages) = grit::live::arranged(width, grid, &plan, None, coarse).unwrap();
        let writes: Vec<Written> = passes.iter().map(|module| written(module)).collect();
        let label = format!("width {width} grid {}x{}", grid.wide, grid.deep);
        let readers: Vec<usize> = (0..stages.len())
            .filter(|&at| stages[at].iter().any(|g| matches!(g, Groups::From { .. })))
            .collect();
        assert_eq!(readers, vec![1, 2], "{label}");
        for times in [1, 2, 3] {
            assert_eq!(
                barriers_of(&stages, &writes, times),
                every_repeat(&[1, 2], times),
                "{label} times {times}"
            );
        }
        seen += 1;
    }
    assert!(seen >= 25, "{seen}");
}

#[test]
fn a_frame_whose_writes_are_unknown_carries_the_bits_into_every_reader() {
    for (width, grid, plan, coarse) in arrangements() {
        let (passes, stages) = grit::live::arranged(width, grid, &plan, None, coarse).unwrap();
        let writes = vec![Written::Every; passes.len()];
        let readers: Vec<usize> = (0..stages.len())
            .filter(|&at| stages[at].iter().any(|g| matches!(g, Groups::From { .. })))
            .collect();
        let later: Vec<usize> = readers.iter().copied().filter(|&at| at > 0).collect();
        assert_eq!(barriers_of(&stages, &writes, 1), every_repeat(&later, 1));
        let mut twice = every_repeat(&later, 1);
        twice.extend(readers.iter().map(|&at| (1, at)));
        assert_eq!(barriers_of(&stages, &writes, 2), twice, "width {width}");
    }
}

fn only(bindings: &[usize]) -> Written {
    Written::Only(bindings.to_vec())
}

#[test]
fn a_reader_right_after_its_writer_gets_the_bits() {
    let stages = vec![vec![Groups::Fixed(1)], vec![from(0)]];
    assert_eq!(
        barriers_of(&stages, &[only(&[0]), only(&[])], 1),
        vec![(0, 1)]
    );
}

#[test]
fn a_reader_of_arguments_nothing_wrote_since_the_last_carrying_barrier_does_not() {
    let stages = vec![
        vec![Groups::Fixed(1)],
        vec![from(0)],
        vec![Groups::Fixed(4)],
        vec![from(0)],
    ];
    let writes = [only(&[0]), only(&[1]), only(&[2, 3]), only(&[1])];
    assert_eq!(barriers_of(&stages, &writes, 1), vec![(0, 1)]);
}

#[test]
fn a_writer_between_two_readers_keeps_the_bits_for_the_second() {
    let stages = vec![
        vec![Groups::Fixed(1)],
        vec![from(0)],
        vec![Groups::Fixed(1)],
        vec![from(0)],
    ];
    let writes = [only(&[0]), only(&[1]), only(&[0]), only(&[1])];
    assert_eq!(barriers_of(&stages, &writes, 1), vec![(0, 1), (0, 3)]);
}

#[test]
fn a_writer_two_stages_back_still_gives_the_reader_the_bits() {
    let stages = vec![
        vec![Groups::Fixed(1)],
        vec![Groups::Fixed(2)],
        vec![from(0)],
    ];
    let writes = [only(&[0]), only(&[1]), only(&[])];
    assert_eq!(barriers_of(&stages, &writes, 1), vec![(0, 2)]);
}

#[test]
fn a_pass_dispatched_over_no_groups_writes_nothing() {
    for zero in [Groups::Fixed(0), Groups::Counting(0)] {
        let stages = vec![vec![zero], vec![from(0)]];
        assert_eq!(barriers_of(&stages, &[only(&[0]), only(&[])], 1), vec![]);
    }
}

#[test]
fn a_writer_the_analysis_cannot_follow_gives_every_reader_the_bits() {
    let stages = vec![vec![Groups::Fixed(1)], vec![from(5)], vec![from(5)]];
    let writes = [Written::Every, only(&[]), only(&[])];
    assert_eq!(barriers_of(&stages, &writes, 1), vec![(0, 1)]);
    let writes = [only(&[]), Written::Every, only(&[])];
    assert_eq!(barriers_of(&stages, &writes, 1), vec![(0, 2)]);
}

#[test]
fn the_first_stage_of_a_repeat_gets_the_bits_from_the_last_stage_before() {
    let stages = vec![vec![from(0)], vec![Groups::Fixed(1)]];
    let writes = [only(&[1]), only(&[0])];
    assert_eq!(barriers_of(&stages, &writes, 1), vec![]);
    assert_eq!(barriers_of(&stages, &writes, 3), vec![(1, 0), (2, 0)]);
}

#[test]
fn any_reader_of_a_written_binding_in_the_next_stage_gives_it_the_bits() {
    let stages = vec![vec![Groups::Fixed(1)], vec![from(0), from(1)]];
    let writes = [only(&[1]), only(&[]), only(&[])];
    assert_eq!(barriers_of(&stages, &writes, 1), vec![(0, 1)]);
    let writes = [only(&[2]), only(&[]), only(&[])];
    assert_eq!(barriers_of(&stages, &writes, 1), vec![]);
}

const ARGS: usize = 0;
const FIRST: usize = 1;
const SECOND: usize = 2;
const SIDE: usize = 3;
const SIZES: [usize; 4] = [4, 1, 1, 1];
const SEVEN: u32 = 7;
const ROUNDS: usize = 4;

fn store(width: u32, binding: u32, words: &[(u32, u32)]) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(width, 1, SIZES.len() as u32))?;
    for &(at, value) in words {
        let slot = kernel.module().constant_u32(at)?;
        let word = kernel.module().constant_u32(value)?;
        kernel.store_at(binding, slot, word)?;
    }
    kernel.finish()
}

fn args(width: u32, x: u32) -> Vec<u32> {
    store(width, ARGS as u32, &[(0, x), (1, 1), (2, 1)]).unwrap()
}

fn marking(width: u32, binding: usize) -> Vec<u32> {
    store(width, binding as u32, &[(0, SEVEN)]).unwrap()
}

/// Writes `[1, 1, 1]` into the arguments, reads them indirectly to mark
/// `FIRST`, runs `between`, and reads them indirectly again to mark `SECOND`.
fn twice_read(host: &Host, between: &[u32], times: u32) -> (Vec<(u32, usize)>, u32, u32) {
    let width = host.width();
    let modules = [
        args(width, 1),
        marking(width, FIRST),
        between.to_vec(),
        marking(width, SECOND),
    ];
    let refs: Vec<&[u32]> = modules.iter().map(Vec::as_slice).collect();
    let mut held = host.resident(&SIZES, &refs).unwrap();
    let read = Groups::From {
        binding: ARGS,
        at: 0,
    };
    let stages: [&[Groups]; 4] = [&[Groups::Fixed(1)], &[read], &[Groups::Fixed(1)], &[read]];
    let carried = indirect_barriers(&stages, held.writes(), times);
    held.write(ARGS, &[0, 1, 1, 0]).unwrap();
    held.write(FIRST, &[0]).unwrap();
    held.write(SECOND, &[0]).unwrap();
    held.staged_frames(&stages, times).unwrap();
    let first = held.read(FIRST, 1).unwrap()[0];
    let second = held.read(SECOND, 1).unwrap()[0];
    (carried, first, second)
}

#[test]
fn a_reader_whose_arguments_nothing_rewrote_skips_the_bits_and_still_sees_them() {
    let Some(host) = device("a_reader_whose_arguments_nothing_rewrote_skips_the_bits") else {
        return;
    };
    let side = store(host.width(), SIDE as u32, &[(0, 1)]).unwrap();
    for times in [1, 2] {
        for round in 0..ROUNDS {
            let (carried, first, second) = twice_read(&host, &side, times);
            let label = format!("{} times {times} round {round}", host.name());
            assert_eq!((first, second), (SEVEN, SEVEN), "{label}");
            assert_eq!(carried, every_repeat(&[1], times), "{label}");
        }
    }
}

#[test]
fn a_writer_between_two_readers_keeps_the_bits_and_the_second_sees_its_value() {
    let Some(host) = device("a_writer_between_two_readers_keeps_the_bits") else {
        return;
    };
    let zero = args(host.width(), 0);
    for round in 0..ROUNDS {
        let (carried, first, second) = twice_read(&host, &zero, 1);
        let label = format!("{} round {round}", host.name());
        assert_eq!((first, second), (SEVEN, 0), "{label}");
        assert_eq!(carried, vec![(0, 1), (0, 3)], "{label}");
    }
}

fn walls(grid: Grid) -> Vec<u32> {
    let mut cells = vec![AIR; grid.cells()];
    for row in 0..grid.deep {
        for column in 0..grid.wide {
            if row + 1 >= grid.deep || column == 0 || column + 1 >= grid.wide {
                cells[(row * grid.wide + column) as usize] = WALL;
            }
        }
    }
    cells
}

fn brushing(frame: u32) -> [u32; 4] {
    let x = 20 + (11 * frame) % 216;
    match frame % 4 {
        0 => [x, 24, 36, SAND],
        1 => [x, 40, 25, WATER],
        2 => [x, 24, 0, SAND],
        _ => [0, 0, 0, 0],
    }
}

struct Engine<'host> {
    live: Resident<'host>,
    full: Resident<'host>,
    stages: Vec<Vec<Groups>>,
    whole: Vec<Groups>,
}

impl Engine<'_> {
    fn brush(&mut self, words: [u32; 4]) {
        let at = grit::layout::BRUSH;
        self.live.write_at(PARAMS as usize, at, &words).unwrap();
        self.full.write_at(PARAMS as usize, at, &words).unwrap();
    }

    fn frames(&mut self, times: u32) {
        let stages: Vec<&[Groups]> = self.stages.iter().map(Vec::as_slice).collect();
        let whole: Vec<&[Groups]> = self.whole.iter().map(core::slice::from_ref).collect();
        self.live.staged_frames(&stages, times).unwrap();
        self.full.staged_frames(&whole, times).unwrap();
    }

    fn agree(&mut self, label: &str) {
        let cells = ROOM.cells();
        let there = self.live.read(STATE as usize, cells).unwrap();
        let here = self.full.read(STATE as usize, cells).unwrap();
        if let Some(at) = there.iter().zip(&here).position(|(a, b)| a != b) {
            let count = there.iter().zip(&here).filter(|(a, b)| a != b).count();
            panic!(
                "{label}: cell ({},{}) listed {:#x} whole {:#x}, {count} cells differ",
                at as u32 % ROOM.wide,
                at as u32 / ROOM.wide,
                there[at],
                here[at]
            );
        }
    }
}

fn engine(host: &Host, coarse: bool) -> (Engine<'_>, Vec<(u32, usize)>) {
    let width = host.width();
    let plan = plan_for(width, ROOM);
    let (passes, stages) = grit::live::arranged(width, ROOM, &plan, None, coarse).unwrap();
    let whole = [
        grit::live::tick(width).unwrap(),
        sand::brush(width, ROOM, false).unwrap(),
        sand::settle(width, ROOM).unwrap(),
    ];
    let sizes = plan.sizes();
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let mut live = host.resident(&sizes, &modules).unwrap();
    let modules: Vec<&[u32]> = whole.iter().map(Vec::as_slice).collect();
    let mut full = host.resident(&sizes, &modules).unwrap();
    let start = walls(ROOM);
    for held in [&mut live, &mut full] {
        for (binding, &words) in sizes.iter().enumerate() {
            if binding != PIXELS as usize {
                held.write(binding, &vec![0_u32; words]).unwrap();
            }
        }
        held.write(STATE as usize, &start).unwrap();
        held.write(PARAMS as usize, &[u32::MAX, plan.dump()])
            .unwrap();
    }
    grit::live::seed(&mut live, ROOM, &plan).unwrap();
    let carried = barriers_of(&stages, live.writes(), 1);
    let engine = Engine {
        live,
        full,
        stages,
        whole: vec![
            Groups::Fixed(1),
            Groups::Fixed(sand::BOX * sand::BOX / width),
            Groups::Fixed(ROOM.quads().div_ceil(width)),
        ],
    };
    assert_eq!(BINDINGS as usize, sizes.len());
    (engine, carried)
}

fn skipping_agrees(label: &str, coarse: bool) {
    let Some(host) = device(label) else {
        return;
    };
    let (mut engine, carried) = engine(&host, coarse);
    let last = engine.stages.len() - 1;
    if coarse {
        assert!(reads(&engine.stages[last], HOT_INDIRECT), "{label}");
        assert!(!carried.contains(&(0, last)), "{label}: {carried:?}");
        assert!(reads(&engine.stages[0], HOT_INDIRECT), "{label}");
    }
    assert!(carried.contains(&(0, 1)), "{label}: {carried:?}");
    assert!(carried.contains(&(0, 2)), "{label}: {carried:?}");
    assert!(reads(&engine.stages[1], DENSE), "{label}");
    let mut frame = 0;
    for round in 0..24 {
        let times = 1 + round % 3;
        engine.brush(brushing(frame));
        engine.frames(times);
        engine.agree(&format!("{label} round {round} times {times}"));
        frame += 1;
    }
    println!(
        "{label} {}: {frame} rounds AGREED, {carried:?}",
        host.name()
    );
}

#[test]
fn the_coarse_frame_without_the_gather_bits_agrees_with_the_whole_grid() {
    skipping_agrees("coarse skipping", true);
}

#[test]
fn the_fine_frame_agrees_with_the_whole_grid_over_repeats() {
    skipping_agrees("fine skipping", false);
}
