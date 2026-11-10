#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use std::collections::BTreeMap;

use common::device;

use grit::compact::Plan;
use grit::geometry::Solid;
use grit::layout::{
    AWAKE, BINDINGS, DENSE, Dimension, GRAND, HOT, INDIRECT, PARAMS, PIXELS, STATE, TALLY,
};
use grit::live;
use grit::sand::{AIR, Grid, SAND, WALL, WATER};
use grit::sand3::{
    self, CORNERS, Grid3, hold_of, material_of, settle_oct, settle_reference3, x_first,
};
use grit::sweep;
use grit::{Groups, Host, Resident};
use simdr::lanes::LaneError;

const WORLDS: [(u32, u32, u32); 3] = [(16, 16, 16), (32, 32, 32), (64, 32, 16)];

const SEEDS: [u64; 4] = [1, 7, 0x5EED, 0xDEAD_BEEF];

const FRAMES: u32 = 64;

const WIDTHS: [u32; 5] = [4, 8, 16, 32, 64];

const TIERS: [bool; 2] = [false, true];

fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut mixed = *state;
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    mixed ^ (mixed >> 31)
}

fn word(state: &mut u64) -> u32 {
    let roll = next(state);
    let kind = match roll % 20 {
        0..8 => AIR,
        8..12 => WATER,
        12..17 => SAND,
        _ => WALL,
    };
    if kind == WALL {
        return WALL;
    }
    let bits = ((roll >> 8) % 128) as u32;
    kind + (bits << 2)
}

/// Every cell drawn at random, as the step-4 generator draws them.
fn dense(grid: Grid3, seed: u64) -> Vec<u32> {
    let mut state = seed;
    (0..grid.cells()).map(|_| word(&mut state)).collect()
}

/// One cell in 256 drawn at random, the rest air, so many octs sleep and
/// only the wake keeps the moving ones listed.
fn sparse(grid: Grid3, seed: u64) -> Vec<u32> {
    let mut state = seed;
    (0..grid.cells())
        .map(|_| {
            if next(&mut state).is_multiple_of(256) {
                word(&mut state)
            } else {
                AIR
            }
        })
        .collect()
}

#[derive(Clone, Copy)]
enum Seam {
    X,
    Y,
    Z,
}

/// Random cells within two cells of one seam of `seam`'s axis, on both
/// sides, and air everywhere else.
fn on_seam(grid: Grid3, seam: Seam, seed: u64) -> Vec<u32> {
    let mut state = seed;
    let mut cells = vec![AIR; grid.cells()];
    for z in 0..grid.tall {
        for y in 0..grid.deep {
            for x in 0..grid.wide {
                let (at, extent) = match seam {
                    Seam::X => (x, grid.wide),
                    Seam::Y => (y, grid.deep),
                    Seam::Z => (z, grid.tall),
                };
                let roll = word(&mut state);
                if at < 2 || at + 2 >= extent {
                    cells[grid.index(x, y, z)] = roll;
                }
            }
        }
    }
    cells
}

/// A box with a wall floor and walls on the low x and z sides, which the
/// torus puts next to the high ones, and loose cells in its upper half.
/// Grains come to rest on the floor and water runs out of patience, so the
/// world falls quiet.
fn room(grid: Grid3, seed: u64) -> Vec<u32> {
    let mut state = seed;
    let mut cells = vec![AIR; grid.cells()];
    for z in 0..grid.tall {
        for y in 0..grid.deep {
            for x in 0..grid.wide {
                let wall = y + 1 == grid.deep || x == 0 || z == 0;
                let roll = next(&mut state);
                cells[grid.index(x, y, z)] = if wall {
                    WALL
                } else if y < grid.deep / 2 && roll.is_multiple_of(4) {
                    if roll.is_multiple_of(8) { SAND } else { WATER }
                } else {
                    AIR
                };
            }
        }
    }
    cells
}

/// A floor, two walls that leave a channel one cell wide along z, and a
/// column of water standing in the channel near its low end: the water can
/// spread along z and nowhere else.
fn channel(grid: Grid3, _seed: u64) -> Vec<u32> {
    let mut cells = vec![AIR; grid.cells()];
    let middle = grid.wide / 2;
    for z in 0..grid.tall {
        for y in 0..grid.deep {
            for x in 0..grid.wide {
                let wall = y + 1 == grid.deep || x + 1 == middle || x == middle + 1;
                if wall {
                    cells[grid.index(x, y, z)] = WALL;
                }
            }
        }
    }
    for y in grid.deep / 2..grid.deep - 1 {
        cells[grid.index(middle, y, 3)] = WATER;
    }
    cells
}

fn grids(wide: u32, deep: u32, tall: u32) -> (Grid, Grid3) {
    let grid = Grid::solid(wide, deep, tall).unwrap();
    let reference = Grid3::new(wide, deep, tall).unwrap();
    assert_eq!(
        (grid.wide, grid.deep, grid.tall),
        (reference.wide, reference.deep, reference.tall)
    );
    (grid, reference)
}

fn plan_of(width: u32, grid: Grid) -> Option<Plan> {
    match Plan::over_in(
        width,
        grid.blocks().div_ceil(sweep::cells(width)),
        Dimension::Three,
    ) {
        Ok(plan) => Some(plan),
        Err(too_big) => {
            common::skip(format_args!("width {width} {too_big}"));
            None
        }
    }
}

fn clear_all(held: &mut Resident<'_>, sizes: &[usize; BINDINGS as usize]) {
    for (binding, &words) in sizes.iter().enumerate() {
        if binding == PIXELS as usize {
            continue;
        }
        held.write(binding, &vec![0_u32; words]).unwrap();
    }
}

/// The listed engine and the whole-grid kernel on the same solid world,
/// stepped side by side and held to the CPU reference.
struct Engine<'host> {
    live: Resident<'host>,
    full: Resident<'host>,
    stages: Vec<Vec<Groups>>,
    whole: [Groups; 2],
    plan: Plan,
    grid: Grid,
    reference: Grid3,
    want: Vec<u32>,
    frame: u32,
    label: String,
}

impl<'host> Engine<'host> {
    fn new(host: &'host Host, (wide, deep, tall): (u32, u32, u32), coarse: bool) -> Option<Self> {
        let width = host.width();
        let (grid, reference) = grids(wide, deep, tall);
        let plan = plan_of(width, grid)?;
        let (passes, stages) = live::arranged3(width, grid, &plan, coarse).unwrap();
        let built = sand3::settle(width, grid, host.limits().groups).unwrap();
        let tick = live::tick(width).unwrap();
        let sizes = plan.sizes();
        let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
        let live = host.resident(&sizes, &modules).unwrap();
        let modules: Vec<&[u32]> = vec![&tick, &built.words];
        let full = host.resident(&sizes, &modules).unwrap();
        Some(Self {
            live,
            full,
            stages,
            whole: [Groups::Fixed(1), built.spread.dispatch()],
            plan,
            grid,
            reference,
            want: Vec::new(),
            frame: 0,
            label: String::new(),
        })
    }

    /// Puts `start` in both worlds and seeds the listed engine, so the next
    /// frame is frame 0 everywhere.
    fn restart(&mut self, start: &[u32], label: String) {
        let sizes = self.plan.sizes();
        for held in [&mut self.live, &mut self.full] {
            clear_all(held, &sizes);
            held.write(STATE as usize, start).unwrap();
            held.write(PARAMS as usize, &[u32::MAX, self.plan.dump()])
                .unwrap();
        }
        live::seed(&mut self.live, self.grid, &self.plan).unwrap();
        self.want = start.to_vec();
        self.frame = 0;
        self.label = label;
    }

    fn parted(&self, what: &str, got: &[u32]) -> Option<String> {
        let cells = self.reference.cells();
        let at = (0..cells).find(|&at| got[at] != self.want[at])?;
        let plane = (self.reference.wide * self.reference.deep) as usize;
        let (z, rest) = (at / plane, at % plane);
        let (y, x) = (
            rest / self.reference.wide as usize,
            rest % self.reference.wide as usize,
        );
        let count = (0..cells).filter(|&at| got[at] != self.want[at]).count();
        Some(format!(
            "{}: frame {} (phase {}, x first {}) cell ({x}, {y}, {z}): {what} {:#x}, reference {:#x}; {count} cells differ",
            self.label,
            self.frame,
            self.frame & 1,
            x_first(self.frame),
            got[at],
            self.want[at]
        ))
    }

    /// Steps the whole grid and the CPU one frame, and holds the listed
    /// world `listed` and the whole grid to the CPU: the first place where
    /// either differs.
    fn check(&mut self, listed: &[u32]) -> Option<String> {
        let cells = self.reference.cells();
        let stages: Vec<&[Groups]> = self.whole.iter().map(core::slice::from_ref).collect();
        let (_, whole) = self
            .full
            .staged_reading(&stages, STATE as usize, cells)
            .unwrap();
        self.want = settle_reference3(self.reference, self.frame, &self.want);
        let parted = self
            .parted("whole grid", &whole)
            .or_else(|| self.parted("listed", listed));
        self.frame += 1;
        parted
    }

    /// Runs `flying` listed frames, all submitted before any is read, and
    /// checks each. Returns the octs listed after each.
    fn frames(&mut self, flying: usize) -> Vec<u32> {
        let cells = self.reference.cells();
        let groups: Vec<&[Groups]> = self.stages.iter().map(Vec::as_slice).collect();
        let mut tickets = Vec::with_capacity(flying);
        for _ in 0..flying {
            tickets.push(
                self.live
                    .submit_reading_both(&groups, (STATE as usize, cells), (GRAND as usize, 1))
                    .unwrap(),
            );
        }
        let mut listed = Vec::with_capacity(flying);
        for ticket in tickets {
            let (_, world, grand) = self.live.wait_reading_both(ticket).unwrap();
            if let Some(parted) = self.check(&world) {
                panic!("{parted}");
            }
            listed.push(grand[0]);
        }
        listed
    }

    fn frame(&mut self) -> u32 {
        self.frames(1)[0]
    }

    /// One listed frame, and where it parts from the reference, if it does.
    fn tried(&mut self) -> Option<String> {
        let cells = self.reference.cells();
        let groups: Vec<&[Groups]> = self.stages.iter().map(Vec::as_slice).collect();
        let (_, world) = self
            .live
            .staged_reading(&groups, STATE as usize, cells)
            .unwrap();
        self.check(&world)
    }

    fn run(&mut self, frames: u32, flying: usize) -> Vec<u32> {
        let mut listed = Vec::new();
        while self.frame < frames {
            listed.extend(self.frames(flying));
        }
        listed
    }
}

fn label(name: &str, (wide, deep, tall): (u32, u32, u32), coarse: bool, seed: u64) -> String {
    let tier = if coarse { "coarse" } else { "fine" };
    format!("{name} {wide}x{deep}x{tall} {tier} seed {seed:#x}")
}

type Starting = fn(Grid3, u64) -> Vec<u32>;

/// Every world of `worlds` from every seed, `FRAMES` frames, on both tiers.
fn matrix(name: &str, worlds: &[(u32, u32, u32)], start: Starting, flying: usize) {
    let Some(host) = device(name) else {
        return;
    };
    for &shape in worlds {
        for coarse in TIERS {
            let Some(mut engine) = Engine::new(&host, shape, coarse) else {
                continue;
            };
            let mut fewest = u32::MAX;
            for seed in SEEDS {
                let first = start(engine.reference, seed);
                engine.restart(&first, label(name, shape, coarse, seed));
                let listed = engine.run(FRAMES, flying);
                assert_ne!(engine.want, first, "{}: nothing moved", engine.label);
                fewest = fewest.min(listed.iter().copied().min().unwrap_or(u32::MAX));
            }
            println!(
                "{name} {} width {} {}x{}x{} coarse {coarse} blocks {}: {} seeds x {FRAMES} frames equal, fewest listed {fewest} of {}",
                host.name(),
                host.width(),
                shape.0,
                shape.1,
                shape.2,
                engine.plan.blocks,
                SEEDS.len(),
                engine.grid.blocks()
            );
        }
    }
}

#[test]
fn the_listed_frame_equals_the_whole_grid_on_dense_random_worlds() {
    matrix("dense", &WORLDS, dense, 1);
}

#[test]
fn the_listed_frame_equals_the_whole_grid_on_sparse_random_worlds() {
    matrix("sparse", &WORLDS, sparse, 1);
}

#[test]
fn the_listed_frame_equals_the_whole_grid_across_the_x_seam() {
    matrix(
        "x seam",
        &WORLDS,
        |grid, seed| on_seam(grid, Seam::X, seed),
        1,
    );
}

#[test]
fn the_listed_frame_equals_the_whole_grid_across_the_y_seam() {
    matrix(
        "y seam",
        &WORLDS,
        |grid, seed| on_seam(grid, Seam::Y, seed),
        1,
    );
}

#[test]
fn the_listed_frame_equals_the_whole_grid_across_the_z_seam() {
    matrix(
        "z seam",
        &WORLDS,
        |grid, seed| on_seam(grid, Seam::Z, seed),
        1,
    );
}

#[test]
fn the_listed_frame_follows_water_that_spreads_only_along_z() {
    matrix("channel", &[(16, 16, 32), (8, 8, 64)], channel, 1);
}

#[test]
fn two_frames_in_flight_equal_the_whole_grid_on_dense_worlds() {
    matrix("in flight dense", &WORLDS, dense, 2);
}

#[test]
fn two_frames_in_flight_equal_the_whole_grid_on_sparse_worlds() {
    matrix("in flight sparse", &WORLDS, sparse, 2);
}

const QUIET_WORLDS: [(u32, u32, u32); 2] = [(16, 16, 16), (32, 16, 16)];

const PATIENCE_FRAMES: u32 = 1024;

/// Runs until the list is empty, checking every frame, and returns the frame
/// it emptied in.
fn fall_quiet(engine: &mut Engine<'_>) -> u32 {
    while engine.frame < PATIENCE_FRAMES {
        if engine.frame() == 0 {
            let quiet = engine.frame;
            for _ in 0..4 {
                assert_eq!(engine.frame(), 0, "{}: the list filled again", engine.label);
            }
            return quiet;
        }
    }
    panic!(
        "{}: the list never emptied in {PATIENCE_FRAMES} frames",
        engine.label
    );
}

/// The host's write: sand high in the room and water next to it, which fall
/// and spread into octs that have been asleep for frames.
fn poured(grid: Grid3) -> Vec<(u32, u32)> {
    let mut cells = Vec::new();
    for (dx, dz, word) in [
        (0, 0, SAND),
        (1, 0, SAND),
        (0, 1, WATER),
        (1, 1, WATER),
        (2, 1, SAND),
    ] {
        let (x, z) = (grid.wide / 2 + dx, grid.tall / 2 + dz);
        let at = grid.index(x, 1, z);
        cells.push((u32::try_from(at).unwrap(), word));
    }
    cells
}

fn quiet_then_woken(name: &str, flying: usize) {
    let Some(host) = device(name) else {
        return;
    };
    for shape in QUIET_WORLDS {
        for coarse in TIERS {
            let Some(mut engine) = Engine::new(&host, shape, coarse) else {
                continue;
            };
            for seed in SEEDS {
                let start = room(engine.reference, seed);
                engine.restart(&start, label(name, shape, coarse, seed));
                let quiet = fall_quiet(&mut engine);

                let cells = poured(engine.reference);
                for &(cell, word) in &cells {
                    engine
                        .full
                        .write_at(STATE as usize, cell as usize, &[word])
                        .unwrap();
                    engine.want[cell as usize] = word;
                }
                live::put3(&mut engine.live, engine.grid, &engine.plan, &cells).unwrap();
                let woken = engine.frame;
                let listed = engine.run(woken + FRAMES, flying);
                assert!(
                    listed.iter().any(|&count| count > 0),
                    "{}: the write woke nothing",
                    engine.label
                );
                let again = fall_quiet(&mut engine);
                println!(
                    "{} {} width {}: quiet at frame {quiet}, written at {woken}, quiet again at {again}",
                    engine.label,
                    host.name(),
                    host.width()
                );
            }
        }
    }
}

#[test]
fn a_quiet_world_drains_the_list_and_a_host_write_wakes_it() {
    quiet_then_woken("quiet", 1);
}

#[test]
fn a_quiet_world_woken_by_a_host_write_agrees_with_two_frames_in_flight() {
    quiet_then_woken("quiet in flight", 2);
}

/// The control for [`live::put3`]: the same cells written into `STATE` alone
/// wake nothing, the empty list steps nothing, and the listed world parts
/// from the whole grid within the two frames it takes both phases to reach
/// the written cells.
#[test]
fn a_host_write_that_wakes_nothing_is_missed() {
    let Some(host) = device("unwoken") else {
        return;
    };
    for coarse in TIERS {
        let Some(mut engine) = Engine::new(&host, (16, 16, 16), coarse) else {
            continue;
        };
        let start = room(engine.reference, SEEDS[0]);
        engine.restart(&start, label("unwoken", (16, 16, 16), coarse, SEEDS[0]));
        fall_quiet(&mut engine);
        let cells = poured(engine.reference);
        for &(cell, word) in &cells {
            for held in [&mut engine.live, &mut engine.full] {
                held.write_at(STATE as usize, cell as usize, &[word])
                    .unwrap();
            }
            engine.want[cell as usize] = word;
        }
        let parted = (0..4).find_map(|_| engine.tried());
        let Some(parted) = parted else {
            panic!(
                "{}: a write the list never saw still moved in the listed world",
                engine.label
            );
        };
        assert!(parted.contains("listed"), "{parted}");
        println!("unwoken {} width {}: {parted}", host.name(), host.width());
    }
}

#[test]
fn put3_lists_each_written_oct_of_either_phase_once_and_keeps_the_list() {
    let Some(host) = device("put3") else {
        return;
    };
    let width = host.width();
    let (grid, reference) = grids(16, 16, 16);
    let plan = plan_of(width, grid).unwrap();
    let (passes, _) = live::arranged3(width, grid, &plan, false).unwrap();
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let sizes = plan.sizes();
    let mut held = host.resident(&sizes, &modules).unwrap();
    clear_all(&mut held, &sizes);
    held.write(DENSE as usize, &[40, 7]).unwrap();
    held.write(GRAND as usize, &[2]).unwrap();

    let corner = u32::try_from(reference.index(0, 0, 0)).unwrap();
    let inner = u32::try_from(reference.index(5, 6, 7)).unwrap();
    let cells = [(corner, SAND), (inner, WATER), (inner + 1, WATER)];
    live::put3(&mut held, grid, &plan, &cells).unwrap();

    let mut wanted = std::collections::BTreeSet::from([40, 7]);
    for (cell, _) in cells {
        for phase in 0..2 {
            wanted.insert(Solid::held(grid, cell, phase).index);
        }
    }
    let wanted: Vec<u32> = wanted.into_iter().collect();
    let count = u32::try_from(wanted.len()).unwrap();
    assert_eq!(held.read(GRAND as usize, 1).unwrap(), [count]);
    assert_eq!(held.read(DENSE as usize, wanted.len()).unwrap(), wanted);
    assert_eq!(
        held.read(INDIRECT as usize, 3).unwrap(),
        [count.div_ceil(width), 1, 1]
    );
    let state = held.read(STATE as usize, grid.cells()).unwrap();
    for (cell, word) in cells {
        assert_eq!(state[cell as usize], word);
    }
    let awake = held.read(AWAKE as usize, grid.blocks() as usize).unwrap();
    let hot = held.read(HOT as usize, plan.blocks as usize).unwrap();
    for &oct in &wanted {
        let marked = awake[oct as usize];
        assert_eq!(
            marked,
            if oct == 40 || oct == 7 { 0 } else { 2 },
            "oct {oct}"
        );
    }
    for oct in [corner, inner].map(|cell| Solid::held(grid, cell, 0).index) {
        assert_eq!(hot[(oct >> plan.per_block.trailing_zeros()) as usize], 2);
    }
    let corner_octs: Vec<u32> = (0..2)
        .map(|phase| Solid::held(grid, corner, phase).index)
        .collect();
    assert_eq!(corner_octs, [0, grid.blocks() - 1]);

    let past = u32::try_from(grid.cells()).unwrap();
    assert!(matches!(
        live::put3(&mut held, grid, &plan, &[(past, SAND)]),
        Err(grit::Error::Overrun { .. })
    ));
}

/// The tally of one frame on the CPU, for every oct: the cells that moved
/// past the held step, each weighted by its oct's point shifted down by
/// `COARSE`, and the octs that moved at all. The wet and dry words are the
/// cells of the whole world after the frame.
fn tally_of(grid: Grid3, frame: u32, cells: &[u32]) -> [u32; 7] {
    let (across, down) = (grid.wide / 2, grid.deep / 2);
    let mut out = [0_u32; 7];
    for oct in 0..grid.octs() {
        let places = grid.corners(frame, oct);
        let block: [u32; CORNERS] = places.map(|at| cells[at]);
        let mut held = block;
        for top in 0..4 {
            held[top] = hold_of(block[top], block[top + 4]);
        }
        let after = settle_oct(block, x_first(frame));
        let moved = u32::try_from(held.iter().zip(&after).filter(|(a, b)| a != b).count()).unwrap();
        let point = [oct % across, (oct / across) % down, oct / across / down];
        out[0] += moved;
        for (word, value) in out[3..6].iter_mut().zip(point) {
            *word = word.wrapping_add((value >> grit::layout::COARSE) * moved);
        }
        out[6] += u32::from(moved > 0);
    }
    let next = settle_reference3(grid, frame, cells);
    out[1] = u32::try_from(
        next.iter()
            .filter(|&&cell| material_of(cell) == WATER)
            .count(),
    )
    .unwrap();
    out[2] = u32::try_from(
        next.iter()
            .filter(|&&cell| material_of(cell) == SAND)
            .count(),
    )
    .unwrap();
    out
}

#[test]
fn the_seven_word_tally_counts_what_the_cpu_counts() {
    let Some(host) = device("tally3") else {
        return;
    };
    for shape in [(16, 16, 16), (64, 32, 16)] {
        for coarse in TIERS {
            let Some(mut engine) = Engine::new(&host, shape, coarse) else {
                continue;
            };
            for (seed, start) in [(SEEDS[0], dense as Starting), (SEEDS[1], sparse)] {
                let first = start(engine.reference, seed);
                engine.restart(&first, label("tally", shape, coarse, seed));
                for _ in 0..16 {
                    let before = engine.want.clone();
                    let frame = engine.frame;
                    engine.frame();
                    let got: [u32; 7] = engine
                        .live
                        .read(TALLY as usize, 7)
                        .unwrap()
                        .try_into()
                        .unwrap();
                    let wanted = tally_of(engine.reference, frame, &before);
                    let pick = |words: [u32; 7]| [words[0], words[3], words[4], words[5], words[6]];
                    assert_eq!(pick(got), pick(wanted), "{} frame {frame}", engine.label);
                    if frame == 0 {
                        assert_eq!(got, wanted, "{} frame 0, every oct listed", engine.label);
                    } else {
                        assert!(got[1] <= wanted[1] && got[2] <= wanted[2]);
                    }
                }
                assert!(engine.frame > 0);
            }
        }
    }
}

/// The loads and stores of each binding in `words`, counted in the SPIR-V
/// as written: a load or store through an access chain into the variable
/// decorated with that binding.
fn accesses(words: &[u32]) -> BTreeMap<u32, (usize, usize)> {
    use std::collections::HashMap;
    let mut instructions = Vec::new();
    let mut at = 5;
    while at < words.len() {
        let count = (words[at] >> 16) as usize;
        assert!(count > 0, "a zero-length instruction at word {at}");
        instructions.push((words[at] & 0xFFFF, &words[at + 1..at + count]));
        at += count;
    }
    let mut bound = HashMap::new();
    for (code, operands) in &instructions {
        if *code == 71 && operands[1] == 33 {
            bound.insert(operands[0], operands[2]);
        }
    }
    let mut chains = HashMap::new();
    for (code, operands) in &instructions {
        if *code == 65
            && let Some(&binding) = bound.get(&operands[2])
        {
            chains.insert(operands[1], binding);
        }
    }
    let mut counts = BTreeMap::new();
    for (code, operands) in &instructions {
        let pointer = match code {
            61 => operands[2],
            62 => operands[0],
            _ => continue,
        };
        let Some(&binding) = chains.get(&pointer) else {
            continue;
        };
        let entry: &mut (usize, usize) = counts.entry(binding).or_default();
        if *code == 61 {
            entry.0 += 1;
        } else {
            entry.1 += 1;
        }
    }
    counts
}

/// An oct is 8 loads and 8 stores of `STATE`; its wake is 27 marks in
/// `AWAKE` and 27 in `HOT`, against 9 and 9 for a quad.
#[test]
fn a_listed_oct_is_eight_loads_eight_stores_and_a_wake_of_twenty_seven() {
    for (wide, deep, tall) in WORLDS {
        let (grid, _) = grids(wide, deep, tall);
        for width in WIDTHS {
            let counts = accesses(&live::settle3(width, grid).unwrap());
            assert_eq!(
                counts.get(&STATE),
                Some(&(8, 8)),
                "{wide}x{deep}x{tall} {width}"
            );
            assert_eq!(counts.get(&AWAKE), Some(&(0, 27)));
            assert_eq!(counts.get(&HOT), Some(&(0, 27)));
            assert_eq!(counts.get(&DENSE), Some(&(2, 0)));
        }
    }
    let flat = accesses(&live::settle(16, Grid::of(64, 64)).unwrap());
    assert_eq!(flat.get(&STATE), Some(&(4, 4)));
    assert_eq!(flat.get(&AWAKE), Some(&(0, 9)));
    assert_eq!(flat.get(&HOT), Some(&(0, 9)));
}

#[test]
fn the_wake_reaches_every_oct_that_can_share_a_cell_next_frame() {
    for (wide, deep, tall) in [(16, 16, 16), (2, 4, 8), (64, 32, 16)] {
        let (grid, reference) = grids(wide, deep, tall);
        for frame in 0..2 {
            for oct in 0..grid.blocks() {
                let woken = Solid::woken(grid, oct);
                for cell in Solid::cells_of(grid, oct, frame) {
                    for later in 1..3 {
                        let holder = Solid::held(grid, cell, frame + later).index;
                        assert!(
                            woken.contains(&holder),
                            "{wide}x{deep}x{tall} frame {frame} oct {oct} cell {cell}: oct {holder} of frame {} not woken",
                            frame + later
                        );
                    }
                }
                let cells = reference.corners(frame, oct);
                let mine: Vec<u32> = Solid::cells_of(grid, oct, frame).to_vec();
                let mut sorted = cells.map(|at| u32::try_from(at).unwrap()).to_vec();
                sorted.sort_unstable();
                let mut theirs = mine.clone();
                theirs.sort_unstable();
                assert_eq!(sorted, theirs);
            }
        }
    }
}

#[test]
fn wired3_picks_the_tier_the_plan_pays_for() {
    for (wide, deep, tall) in [(16, 16, 16), (256, 256, 256)] {
        let (grid, _) = grids(wide, deep, tall);
        for width in WIDTHS {
            let Ok(plan) = Plan::over_in(
                width,
                grid.blocks().div_ceil(sweep::cells(width)),
                Dimension::Three,
            ) else {
                continue;
            };
            let coarse = live::coarse_pays(plan.blocks);
            let wired = live::wired3(width, grid, &plan).unwrap();
            let arranged = live::arranged3(width, grid, &plan, coarse).unwrap();
            assert_eq!(wired.0, arranged.0);
            assert_eq!(wired.1.len(), arranged.1.len());
        }
    }
}

fn refused<T: std::fmt::Debug>(built: Result<T, LaneError>, named: &str, why: &str) {
    match built {
        Err(LaneError::NoSuchForm { operation, because }) => {
            assert_eq!(operation, named);
            assert_eq!(because, why);
        }
        other => panic!("{named}: wanted {why:?}, got {other:?}"),
    }
}

#[test]
fn the_3d_builders_refuse_a_flat_grid_a_flat_plan_and_a_short_plan_by_name() {
    let (grid, _) = grids(16, 16, 16);
    let flat = Grid::of(64, 64);
    for width in WIDTHS {
        let plan = plan_of(width, grid).unwrap();
        refused(live::settle3(width, flat), "live::settle3", sand3::FLAT);
        refused(
            live::arranged3(width, flat, &plan, false),
            "live::arranged3",
            sand3::FLAT,
        );
        refused(
            live::wired3(width, flat, &plan),
            "live::wired3",
            sand3::FLAT,
        );

        let two = Plan::over(width, grid.blocks().div_ceil(sweep::cells(width))).unwrap();
        refused(
            live::arranged3(width, grid, &two, true),
            "live::arranged3",
            live::FLAT_PLAN,
        );
        let short = Plan::over_in(width, 1, Dimension::Three).unwrap();
        if short.quads < grid.blocks() as usize {
            refused(
                live::arranged3(width, grid, &short, false),
                "live::arranged3",
                live::SHORT_PLAN,
            );
            refused(
                live::wired3(width, grid, &short),
                "live::wired3",
                live::SHORT_PLAN,
            );
        }
        assert!(live::arranged3(width, grid, &plan, false).is_ok());
        assert!(matches!(
            live::settle3(12, grid),
            Err(LaneError::NoMapping { lanes: 12, .. })
        ));
    }
}
