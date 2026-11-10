#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use grit::compact::Plan;
use grit::layout::{AWAKE, BINDINGS, BRUSH, DENSE, GRAND, PARAMS, PIXELS, STATE};
use grit::sand::{self, AIR, Grid, SAND, WALL, WATER};
use grit::sweep;
use grit::{Groups, Resident};

const ROOM: Grid = Grid::of(256, 128);
const JUNK: u32 = 0x5eed_0bad;
const QUIET: u32 = 8;

type Brushing = fn(u32) -> [u32; 4];

fn walls(grid: Grid) -> Vec<u32> {
    let mut cells = vec![AIR; grid.cells()];
    for row in 0..grid.deep {
        for column in 0..grid.wide {
            let edge = row + 1 >= grid.deep || column == 0 || column + 1 >= grid.wide;
            if edge {
                cells[(row * grid.wide + column) as usize] = WALL;
            }
        }
    }
    cells
}

fn plan_for(width: u32, grid: Grid) -> Plan {
    Plan::over(width, grid.quads().div_ceil(sweep::cells(width))).unwrap()
}

fn clear_all(held: &mut Resident<'_>, sizes: &[usize; BINDINGS as usize]) {
    for (binding, &words) in sizes.iter().enumerate() {
        if binding == PIXELS as usize {
            continue;
        }
        held.write(binding, &vec![0_u32; words]).unwrap();
    }
}

fn read(held: &mut Resident<'_>, binding: u32, from: usize, count: usize) -> Vec<u32> {
    held.read(binding as usize, from + count).unwrap()[from..].to_vec()
}

fn args_at(plan: &Plan) -> usize {
    plan.dump() as usize + 1
}

fn grids() -> Vec<Grid> {
    vec![
        Grid::of(4, 4),
        Grid::of(8, 8),
        Grid::of(16, 4),
        Grid::of(64, 64),
        Grid::of(256, 128),
        Grid::of(4096, 4),
        Grid::of(1024, 1024),
        Grid::of(4096, 4096),
    ]
}

/// Every plan the engine can be arranged over, at every subgroup width and
/// on both tiers, that `Plan::over` accepts.
fn arrangements() -> Vec<(u32, Grid, Plan, bool)> {
    let mut every = Vec::new();
    for width in [4_u32, 8, 16, 32, 64] {
        for grid in grids() {
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

#[test]
fn the_engine_dispatches_the_brush_only_through_the_count_its_first_stage_writes() {
    let every = arrangements();
    assert!(every.len() >= 60, "{}", every.len());
    for (width, grid, plan, coarse) in every {
        let (_, stages) = grit::live::arranged(width, grid, &plan, None, coarse).unwrap();
        let brush = &stages[1];
        assert_eq!(
            brush.as_slice(),
            &[Groups::From {
                binding: DENSE as usize,
                at: args_at(&plan),
            }],
            "width {width} grid {}x{} coarse {coarse}",
            grid.wide,
            grid.deep
        );
    }
}

#[test]
fn the_brush_count_lies_past_the_list_and_the_claim_count_and_inside_its_binding() {
    for (width, grid, plan, coarse) in arrangements() {
        let at = args_at(&plan);
        let sizes = plan.sizes();
        let label = format!(
            "width {width} grid {}x{} coarse {coarse}",
            grid.wide, grid.deep
        );
        assert!(at > plan.dump() as usize, "{label}");
        assert!(at > grid.quads() as usize, "{label}");
        assert!(at + 3 <= sizes[DENSE as usize], "{label}");
    }
}

#[test]
fn no_stage_but_the_brush_reads_the_brush_count() {
    for (width, grid, plan, coarse) in arrangements() {
        let (_, stages) = grit::live::arranged(width, grid, &plan, None, coarse).unwrap();
        let at = args_at(&plan);
        let readers: Vec<usize> = stages
            .iter()
            .enumerate()
            .filter(|(_, stage)| {
                stage.iter().any(|groups| {
                    matches!(groups, Groups::From { binding, at: from }
                        if *binding == DENSE as usize && from + 3 > at && *from < at + 3)
                })
            })
            .map(|(index, _)| index)
            .collect();
        assert_eq!(readers, vec![1], "width {width} coarse {coarse}");
    }
}

struct Engine<'host> {
    live: Resident<'host>,
    full: Resident<'host>,
    live_groups: Vec<Vec<Groups>>,
    whole_groups: Vec<Groups>,
    plan: Plan,
    grid: Grid,
}

impl Engine<'_> {
    fn brush(&mut self, words: [u32; 4]) {
        self.live.write_at(PARAMS as usize, BRUSH, &words).unwrap();
        self.full.write_at(PARAMS as usize, BRUSH, &words).unwrap();
    }

    fn frame(&mut self) -> (Vec<u32>, u32) {
        let cells = self.grid.cells();
        let groups = self.live_groups.clone();
        let stages: Vec<&[Groups]> = groups.iter().map(Vec::as_slice).collect();
        let whole_groups = self.whole_groups.clone();
        let whole: Vec<&[Groups]> = whole_groups.iter().map(core::slice::from_ref).collect();
        let (_, there, listed) = self
            .live
            .staged_reading_both(&stages, (STATE as usize, cells), (GRAND as usize, 1))
            .unwrap();
        let (_, here) = self
            .full
            .staged_reading(&whole, STATE as usize, cells)
            .unwrap();
        differ(&there, &here, self.grid);
        (there, listed[0])
    }

    fn frames_in_flight(&mut self) -> Vec<(Vec<u32>, u32)> {
        let cells = self.grid.cells();
        let groups = self.live_groups.clone();
        let stages: Vec<&[Groups]> = groups.iter().map(Vec::as_slice).collect();
        let whole_groups = self.whole_groups.clone();
        let whole: Vec<&[Groups]> = whole_groups.iter().map(core::slice::from_ref).collect();
        let tickets: Vec<_> = (0..2)
            .map(|_| {
                self.live
                    .submit_reading_both(&stages, (STATE as usize, cells), (GRAND as usize, 1))
                    .unwrap()
            })
            .collect();
        let mut seen = Vec::new();
        for ticket in tickets {
            let (_, there, listed) = self.live.wait_reading_both(ticket).unwrap();
            let (_, here) = self
                .full
                .staged_reading(&whole, STATE as usize, cells)
                .unwrap();
            differ(&there, &here, self.grid);
            seen.push((there, listed[0]));
        }
        seen
    }
}

fn differ(listed: &[u32], whole: &[u32], grid: Grid) {
    if let Some(at) = listed.iter().zip(whole).position(|(a, b)| a != b) {
        let count = listed.iter().zip(whole).filter(|(a, b)| a != b).count();
        panic!(
            "cell ({},{}) listed {:#x} whole {:#x}, {count} cells differ",
            at as u32 % grid.wide,
            at as u32 / grid.wide,
            listed[at],
            whole[at]
        );
    }
}

/// The frame a caller who brings its own middle builds from `keeping`, with
/// the brush dispatched the way `keeping` hands it over.
fn kept(width: u32, grid: Grid, plan: &Plan, coarse: bool) -> grit::live::Loop {
    let keeping = grit::live::keeping(width, plan, coarse).unwrap();
    let mut stages: grit::live::Stages =
        vec![vec![(grit::live::tick(width).unwrap(), Groups::Fixed(1))]];
    stages.extend(keeping.before);
    stages.push(vec![(
        sand::brush(width, grid, true).unwrap(),
        keeping.brush,
    )]);
    stages.push(vec![(
        grit::live::settle(width, grid).unwrap(),
        keeping.over,
    )]);
    stages.extend(keeping.after);
    grit::live::staged(stages)
}

fn arranged(width: u32, grid: Grid, plan: &Plan, coarse: bool) -> grit::live::Loop {
    grit::live::arranged(width, grid, plan, None, coarse).unwrap()
}

type Building = fn(u32, Grid, &Plan, bool) -> grit::live::Loop;

fn engine(host: &grit::Host, grid: Grid, coarse: bool) -> Engine<'_> {
    engine_of(host, grid, coarse, arranged)
}

fn engine_of(host: &grit::Host, grid: Grid, coarse: bool, build: Building) -> Engine<'_> {
    let width = host.width();
    let plan = plan_for(width, grid);
    let (passes, stages) = build(width, grid, &plan, coarse);
    let whole = [
        grit::live::tick(width).unwrap(),
        sand::brush(width, grid, false).unwrap(),
        sand::settle(width, grid).unwrap(),
    ];
    let whole_groups = vec![
        Groups::Fixed(1),
        Groups::Fixed(sand::BOX * sand::BOX / width),
        Groups::Fixed(grid.quads().div_ceil(width)),
    ];
    let sizes = plan.sizes();
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let mut live = host.resident(&sizes, &modules).unwrap();
    let modules: Vec<&[u32]> = whole.iter().map(Vec::as_slice).collect();
    let mut full = host.resident(&sizes, &modules).unwrap();
    let start = walls(grid);
    for held in [&mut live, &mut full] {
        clear_all(held, &sizes);
        held.write(STATE as usize, &start).unwrap();
        held.write(PARAMS as usize, &[u32::MAX, plan.dump()])
            .unwrap();
    }
    grit::live::seed(&mut live, grid, &plan).unwrap();
    Engine {
        live,
        full,
        live_groups: stages,
        whole_groups,
        plan,
        grid,
    }
}

fn quiet(engine: &mut Engine<'_>, idle: [u32; 4]) {
    engine.brush(idle);
    let mut listed = u32::MAX;
    for _ in 0..QUIET {
        listed = engine.frame().1;
    }
    assert_eq!(listed, 0, "the walled room never fell quiet");
}

const IDLE: [(&str, [u32; 4]); 4] = [
    ("air", [128, 30, 36, AIR]),
    ("no reach", [128, 30, 0, SAND]),
    ("empty", [0, 0, 0, 0]),
    ("no reach, no material", [17, 90, 0, AIR]),
];

/// After a quiet frame with an idle brush, nothing a brush seat writes was
/// written: the brush's count reads zero groups, and none of the scratch
/// slots in `AWAKE` that every seat which wakes nothing marks holds a mark.
fn an_idle_brush_runs_no_seat(label: &str, coarse: bool, build: Building) {
    let Some(host) = device(label) else {
        return;
    };
    let width = host.width();
    for (name, idle) in IDLE {
        let mut engine = engine_of(&host, ROOM, coarse, build);
        quiet(&mut engine, idle);

        let plan = engine.plan;
        let sizes = plan.sizes();
        let scratch = plan.quads;
        let spare = sizes[AWAKE as usize] - scratch;
        assert!(spare >= sand::SEATS, "{label} {name}");
        engine
            .live
            .write_at(AWAKE as usize, scratch, &vec![0; spare])
            .unwrap();
        engine
            .live
            .write_at(DENSE as usize, args_at(&plan), &[JUNK; 3])
            .unwrap();
        let before = read(&mut engine.live, STATE, 0, ROOM.cells());

        let (after, listed) = engine.frame();
        assert_eq!(listed, 0, "{label} {name}");
        assert_eq!(after, before, "{label} {name}");

        let marks = read(&mut engine.live, AWAKE, scratch, spare);
        let marked = marks.iter().filter(|&&mark| mark != 0).count();
        assert_eq!(marked, 0, "{label} {name}: {marked} brush seats ran");
        let args = read(&mut engine.live, DENSE, args_at(&plan), 3);
        assert_eq!(args, [0, 1, 1], "{label} {name}: brush count");

        engine.brush([128, 30, 36, SAND]);
        engine.frame();
        let args = read(&mut engine.live, DENSE, args_at(&plan), 3);
        assert_eq!(
            args,
            [sand::BOX * sand::BOX / width, 1, 1],
            "{label} {name}: painting brush count"
        );
        println!(
            "{label} {name} {} sg={width}: idle brush ran no seat",
            host.name()
        );
    }
}

#[test]
fn an_idle_brush_runs_no_seat_on_the_fine_tier() {
    an_idle_brush_runs_no_seat("idle fine", false, arranged);
}

#[test]
fn an_idle_brush_runs_no_seat_on_the_coarse_tier() {
    an_idle_brush_runs_no_seat("idle coarse", true, arranged);
}

/// The full count of seats once the brush paints, read back after the frame
/// in which it starts: the count is made in the frame it is used in.
fn a_painting_brush_gets_every_seat(label: &str, coarse: bool) {
    let Some(host) = device(label) else {
        return;
    };
    let width = host.width();
    let mut engine = engine(&host, ROOM, coarse);
    quiet(&mut engine, [0, 0, 0, AIR]);
    let at = args_at(&engine.plan);
    for (frame, words) in [
        [128, 30, 36, SAND],
        [128, 30, 0, SAND],
        [9, 9, 1, WATER],
        [0, 0, 0, 0],
        [250, 120, 4, WALL],
    ]
    .into_iter()
    .enumerate()
    {
        engine.brush(words);
        engine.frame();
        let args = read(&mut engine.live, DENSE, at, 3);
        let wanted = if words[2] == 0 || words[3] == AIR {
            0
        } else {
            sand::BOX * sand::BOX / width
        };
        assert_eq!(args, [wanted, 1, 1], "{label} frame {frame} {words:?}");
    }
}

#[test]
fn a_painting_brush_gets_every_seat_on_the_fine_tier() {
    a_painting_brush_gets_every_seat("seats fine", false);
}

#[test]
fn a_painting_brush_gets_every_seat_on_the_coarse_tier() {
    a_painting_brush_gets_every_seat("seats coarse", true);
}

fn sand_in(cells: &[u32]) -> usize {
    cells
        .iter()
        .filter(|&&cell| sand::material_of(cell) == SAND)
        .count()
}

/// A brush that was idle while the room fell quiet is turned on in frame
/// `QUIET`. The listed engine holds no quad at that point, so only the
/// brush's claim can put the painted quads in front of the settle; the frame
/// must still match the whole grid, which paints and settles every quad.
fn turned_on_in_frame_n(label: &str, coarse: bool, idle: [u32; 4], in_flight: bool) {
    let Some(host) = device(label) else {
        return;
    };
    let mut engine = engine(&host, ROOM, coarse);
    quiet(&mut engine, idle);

    let painting = [128, 30, 36, SAND];
    engine.brush(painting);
    let first = if in_flight {
        engine.frames_in_flight().remove(0)
    } else {
        engine.frame()
    };
    let (world, listed) = first;
    assert!(listed > 0, "{label}: nothing was listed after the paint");
    let poured = sand_in(&world);
    assert!(poured > 0, "{label}: the brush painted nothing");

    let unsettled = {
        let mut cells = walls(ROOM);
        for row in 0..ROOM.deep {
            for column in 0..ROOM.wide {
                let dx = column.wrapping_sub(painting[0]);
                let dy = row.wrapping_sub(painting[1]);
                let far = dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy));
                if far < painting[2] {
                    cells[(row * ROOM.wide + column) as usize] = SAND;
                }
            }
        }
        cells
    };
    assert_eq!(sand_in(&unsettled), poured, "{label}");
    assert_ne!(
        world, unsettled,
        "{label}: the painted sand was not stepped in the frame it was painted"
    );

    for _ in 0..24 {
        engine.frame();
    }
    engine.brush(idle);
    for _ in 0..24 {
        engine.frame();
    }
    println!("{label} {}: on in frame {QUIET}, AGREED", host.name());
}

#[test]
fn a_brush_turned_on_from_air_paints_and_settles_in_that_frame() {
    turned_on_in_frame_n("on from air", false, [128, 30, 36, AIR], false);
}

#[test]
fn a_brush_turned_on_from_air_paints_and_settles_in_that_frame_on_the_coarse_tier() {
    turned_on_in_frame_n("on from air coarse", true, [128, 30, 36, AIR], false);
}

#[test]
fn a_brush_given_reach_paints_and_settles_in_that_frame() {
    turned_on_in_frame_n("on from no reach", false, [128, 30, 0, SAND], false);
}

#[test]
fn a_brush_given_reach_paints_and_settles_in_that_frame_on_the_coarse_tier() {
    turned_on_in_frame_n("on from no reach coarse", true, [128, 30, 0, SAND], false);
}

#[test]
fn a_brush_turned_on_with_two_frames_in_flight_paints_and_settles_in_that_frame() {
    turned_on_in_frame_n("on in flight", false, [0, 0, 0, 0], true);
}

#[test]
fn a_brush_turned_on_with_two_frames_in_flight_agrees_on_the_coarse_tier() {
    turned_on_in_frame_n("on in flight coarse", true, [0, 0, 0, 0], true);
}

fn flicker(frame: u32) -> [u32; 4] {
    let x = 20 + (7 * frame) % 216;
    match frame % 5 {
        0 => [x, 24, 36, SAND],
        1 => [x, 24, 0, SAND],
        2 => [x, 24, 25, WATER],
        3 => [x, 24, 25, AIR],
        _ => [0, 0, 0, 0],
    }
}

fn seam(frame: u32) -> [u32; 4] {
    let material = if frame.is_multiple_of(3) { AIR } else { SAND };
    [ROOM.wide - 1, (frame * 5) % ROOM.deep, 49, material]
}

/// A brush that switches between painting and idle every frame, in every
/// way it can be idle, agrees with the whole grid frame by frame.
fn switching(label: &str, coarse: bool, brushing: Brushing, in_flight: bool) {
    switching_in(label, coarse, brushing, in_flight, arranged);
}

fn switching_in(label: &str, coarse: bool, brushing: Brushing, in_flight: bool, build: Building) {
    let Some(host) = device(label) else {
        return;
    };
    let mut engine = engine_of(&host, ROOM, coarse, build);
    let mut frame = 0;
    while frame < 120 {
        engine.brush(brushing(frame));
        if in_flight {
            engine.frames_in_flight();
            frame += 2;
        } else {
            engine.frame();
            frame += 1;
        }
    }
    println!("{label} {}: {frame} frames AGREED", host.name());
}

#[test]
fn a_brush_that_flickers_between_idle_and_painting_agrees_with_the_whole_grid() {
    switching("flicker", false, flicker, false);
}

#[test]
fn a_flickering_brush_agrees_on_the_coarse_tier_too() {
    switching("flicker coarse", true, flicker, false);
}

#[test]
fn a_flickering_brush_agrees_with_two_frames_in_flight() {
    switching("flicker in flight", false, flicker, true);
}

#[test]
fn a_flickering_brush_at_the_seam_agrees_on_both_tiers() {
    switching("seam fine", false, seam, false);
    switching("seam coarse", true, seam, true);
}

#[test]
fn keeping_hands_over_the_brush_count_and_the_pass_that_writes_it() {
    for (width, grid, plan, coarse) in arrangements() {
        let keeping = grit::live::keeping(width, &plan, coarse).unwrap();
        let label = format!(
            "width {width} grid {}x{} coarse {coarse}",
            grid.wide, grid.deep
        );
        assert_eq!(
            keeping.brush,
            Groups::From {
                binding: DENSE as usize,
                at: args_at(&plan),
            },
            "{label}"
        );
        let writer = grit::live::unclaimed(width, &plan).unwrap();
        let held = keeping
            .before
            .iter()
            .flatten()
            .filter(|(module, _)| *module == writer)
            .count();
        assert_eq!(held, 1, "{label}");
        let after_reads = keeping.after.iter().flatten().any(|(_, groups)| {
            matches!(groups, Groups::From { binding, at }
                if *binding == DENSE as usize && *at == args_at(&plan))
        });
        assert!(!after_reads, "{label}");
    }
}

#[test]
fn an_idle_brush_in_a_kept_frame_runs_no_seat() {
    an_idle_brush_runs_no_seat("kept idle fine", false, kept);
    an_idle_brush_runs_no_seat("kept idle coarse", true, kept);
}

#[test]
fn a_flickering_brush_in_a_kept_frame_agrees_with_the_whole_grid() {
    switching_in("kept flicker fine", false, flicker, false, kept);
    switching_in("kept flicker coarse", true, flicker, true, kept);
}
