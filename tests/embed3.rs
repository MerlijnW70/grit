#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use grit::compact::Plan;
use grit::layout::{BINDINGS, Dimension, GRAND, PARAMS, PIXELS, STATE, TALLY};
use grit::live;
use grit::ops::Dots;
use grit::rule3::embed::{self, Embed};
use grit::rule3::{CORNERS, Declared, FIRST_PARAM, Oct, Rule};
use grit::sand::Grid;
use grit::sand3::{self, Grid3, reference};
use grit::sweep;
use grit::{Groups, Host, Resident};

const WORLDS: [(u32, u32, u32); 2] = [(16, 16, 16), (32, 16, 16)];

const SEEDS: [u64; 3] = [1, 0x5EED, 0xDEAD_BEEF];

const FRAMES: u32 = 48;

const PATIENCE: u32 = 4096;

const DOT_PRODUCT: u32 = 6019;

const PACKED: u32 = 6018;

const CAPABILITY: u32 = 17;

const TOWARD: [[i8; 4]; 3] = [[3, -2, 1, 0], [127, -128, 127, -128], [0, 0, 0, 1]];

fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut mixed = *state;
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    mixed ^ (mixed >> 31)
}

fn cell(state: &mut u64) -> u32 {
    let roll = next(state);
    match roll % 4 {
        0 => 0,
        1 => [0x8080_8080, 0x7F7F_7F7F, u32::MAX, 1][((roll >> 8) % 4) as usize],
        _ => (roll >> 32) as u32,
    }
}

fn world(grid: Grid3, seed: u64) -> Vec<u32> {
    let mut state = seed;
    (0..grid.cells()).map(|_| cell(&mut state)).collect()
}

fn param(toward: [i8; 4]) -> Vec<u32> {
    vec![embed::embedding(toward)]
}

fn parity(grid: Grid3, at: usize) -> u32 {
    let plane = (grid.wide * grid.deep) as usize;
    let (z, rest) = (at / plane, at % plane);
    let (y, x) = (rest / grid.wide as usize, rest % grid.wide as usize);
    u32::try_from((x & 1) + (y & 1) + (z & 1)).unwrap()
}

fn potential(grid: Grid3, cells: &[u32], toward: u32) -> i64 {
    cells
        .iter()
        .enumerate()
        .map(|(at, &word)| i64::from(embed::score(word, toward)) * i64::from(parity(grid, at)))
        .sum()
}

fn sorted(cells: &[u32]) -> Vec<u32> {
    let mut held = cells.to_vec();
    held.sort_unstable();
    held
}

#[test]
fn a_cell_trades_places_with_a_neighbour_of_larger_dot_on_the_odd_side() {
    let toward = embed::embedding([1, 0, 0, 0]);
    let big = embed::embedding([5, 0, 0, 0]);
    let small = embed::embedding([-5, 9, 9, 9]);
    let mut words = [0_u32; CORNERS];
    words[0] = big;
    let step = |words: [u32; CORNERS], frame: u32| {
        Embed::default()
            .step(Oct {
                words,
                frame,
                at: [0, 0, 0],
                params: &[toward],
            })
            .out
    };
    let mut want = [0_u32; CORNERS];
    want[7] = big;
    assert_eq!(step(words, 0), want);
    assert_eq!(step(words, 1), words);
    let mut low = [0_u32; CORNERS];
    low[7] = small;
    let mut lowered = [0_u32; CORNERS];
    lowered[0] = small;
    assert_eq!(step(low, 0), lowered);
    assert_eq!(step(lowered, 0), lowered);
    assert_eq!(step(want, 1), {
        let mut held = [0_u32; CORNERS];
        held[0] = big;
        held
    });
}

#[test]
fn the_tally_counts_the_cells_moved_and_the_cells_of_positive_dot() {
    let toward = embed::embedding([1, 0, 0, 0]);
    let mut words = [0_u32; CORNERS];
    words[0] = embed::embedding([5, 0, 0, 0]);
    words[3] = embed::embedding([-2, 0, 0, 0]);
    words[5] = embed::embedding([7, 0, 0, 0]);
    let ruled = Embed::default().step(Oct {
        words,
        frame: 0,
        at: [0, 0, 0],
        params: &[toward],
    });
    let moved = u32::try_from(
        ruled
            .out
            .iter()
            .zip(&words)
            .filter(|(one, other)| one != other)
            .count(),
    )
    .unwrap();
    assert!(moved > 0);
    assert_eq!(ruled.counts, vec![moved, 2]);
    assert_eq!(Embed::default().tally().counts, ruled.counts.len());
    assert!(Embed::default().tally().weighted);
}

#[test]
fn every_frame_that_changes_the_world_raises_its_potential_and_keeps_its_cells() {
    for toward in TOWARD {
        let toward = embed::embedding(toward);
        let grid = Grid3::new(16, 8, 8).unwrap();
        let start = world(grid, 0xE3BED);
        let mut cells = start.clone();
        let mut frame = 0;
        let mut still = 0;
        while still < 2 {
            assert!(
                frame < PATIENCE,
                "{toward:#010x}: no rest in {PATIENCE} frames"
            );
            let after = reference(&Embed::default(), grid, frame, &cells, &[toward]);
            if after == cells {
                still += 1;
            } else {
                still = 0;
                assert!(
                    potential(grid, &after, toward) > potential(grid, &cells, toward),
                    "{toward:#010x} frame {frame}"
                );
            }
            cells = after;
            frame += 1;
        }
        assert_eq!(sorted(&cells), sorted(&start));
        assert_ne!(cells, start);
        println!("{toward:#010x}: at rest after {frame} frames");
    }
}

#[test]
fn a_world_of_equal_words_never_moves() {
    let grid = Grid3::new(8, 8, 8).unwrap();
    for word in [0, 0x8080_8080, 0x1234_5678] {
        let cells = vec![word; grid.cells()];
        for frame in 0..4 {
            assert_eq!(
                reference(&Embed::default(), grid, frame, &cells, &[0x7F81_7F81]),
                cells
            );
        }
    }
}

fn grids(wide: u32, deep: u32, tall: u32) -> (Grid, Grid3) {
    (
        Grid::solid(wide, deep, tall).unwrap(),
        Grid3::new(wide, deep, tall).unwrap(),
    )
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

struct Engine<'host> {
    rule: Embed,
    params: Vec<u32>,
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
    fn new(host: &'host Host, rule: Embed, shape: (u32, u32, u32), coarse: bool) -> Option<Self> {
        let width = host.width();
        let (grid, reference) = grids(shape.0, shape.1, shape.2);
        let plan = plan_of(width, grid)?;
        let (passes, stages) = live::arranged3_by(&rule, width, grid, &plan, coarse).unwrap();
        let built = sand3::settle_by(&rule, width, grid, host.limits().groups).unwrap();
        let tick = live::tick(width).unwrap();
        let sizes = live::sizes3(&rule, &plan);
        let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
        let live = host.resident(&sizes, &modules).unwrap();
        let modules: Vec<&[u32]> = vec![&tick, &built.words];
        let full = host.resident(&sizes, &modules).unwrap();
        Some(Self {
            rule,
            params: Vec::new(),
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

    fn restart(&mut self, start: &[u32], params: Vec<u32>, label: String) {
        let sizes = live::sizes3(&self.rule, &self.plan);
        for held in [&mut self.live, &mut self.full] {
            clear_all(held, &sizes);
            held.write(STATE as usize, start).unwrap();
            held.write_at(PARAMS as usize, FIRST_PARAM, &params)
                .unwrap();
            held.write(PARAMS as usize, &[u32::MAX, self.plan.dump()])
                .unwrap();
        }
        live::seed(&mut self.live, self.grid, &self.plan).unwrap();
        for held in [&mut self.live, &mut self.full] {
            held.write(PARAMS as usize, &[u32::MAX]).unwrap();
        }
        self.params = params;
        self.want = start.to_vec();
        self.frame = 0;
        self.label = label;
    }

    fn parted(&self, what: &str, got: &[u32]) -> Option<String> {
        let cells = self.reference.cells();
        let at = (0..cells).find(|&at| got[at] != self.want[at])?;
        let count = (0..cells).filter(|&at| got[at] != self.want[at]).count();
        Some(format!(
            "{}: frame {} cell {at}: {what} {:#x}, reference {:#x}; {count} cells differ",
            self.label, self.frame, got[at], self.want[at]
        ))
    }

    fn frame(&mut self) -> (u32, Vec<u32>) {
        let cells = self.reference.cells();
        let groups: Vec<&[Groups]> = self.stages.iter().map(Vec::as_slice).collect();
        let ticket = self
            .live
            .submit_reading_both(&groups, (STATE as usize, cells), (GRAND as usize, 1))
            .unwrap();
        let (_, listed, grand) = self.live.wait_reading_both(ticket).unwrap();
        let tally = self.live.read(TALLY as usize, 2).unwrap();
        let stages: Vec<&[Groups]> = self.whole.iter().map(core::slice::from_ref).collect();
        let (_, whole) = self
            .full
            .staged_reading(&stages, STATE as usize, cells)
            .unwrap();
        self.want = reference(
            &self.rule,
            self.reference,
            self.frame,
            &self.want,
            &self.params,
        );
        if let Some(parted) = self.parted("whole grid", &whole) {
            panic!("{parted}");
        }
        if let Some(parted) = self.parted("listed", &listed) {
            panic!("{parted}");
        }
        self.frame += 1;
        (grand[0], tally)
    }
}

fn paths(host: &Host) -> Vec<Dots> {
    let mut paths = vec![Dots::Exact];
    if host.dots() == Dots::Native {
        paths.push(Dots::Native);
    }
    paths
}

#[test]
fn listed_and_whole_equal_the_reference_on_both_paths() {
    let Some(host) = device("embed listed") else {
        return;
    };
    for dots in paths(&host) {
        for shape in WORLDS {
            for coarse in [false, true] {
                let Some(mut engine) = Engine::new(&host, Embed::new(dots), shape, coarse) else {
                    continue;
                };
                for (seed, toward) in SEEDS.into_iter().zip(TOWARD) {
                    let start = world(engine.reference, seed);
                    engine.restart(
                        &start,
                        param(toward),
                        format!("{dots:?} {shape:?} coarse {coarse} seed {seed:#x}"),
                    );
                    for _ in 0..FRAMES {
                        engine.frame();
                    }
                    assert_ne!(engine.want, start, "{}: nothing moved", engine.label);
                }
                println!(
                    "embed {} width {} {dots:?} {shape:?} coarse {coarse}: {} seeds x {FRAMES} frames equal",
                    host.name(),
                    host.width(),
                    SEEDS.len()
                );
            }
        }
    }
}

#[test]
fn the_listed_engine_falls_quiet_and_its_tally_counts_the_moves() {
    let Some(host) = device("embed quiet") else {
        return;
    };
    for dots in paths(&host) {
        let Some(mut engine) = Engine::new(&host, Embed::new(dots), (16, 8, 8), false) else {
            continue;
        };
        let start = world(engine.reference, 0xE3BED);
        let toward = param(TOWARD[0]);
        engine.restart(&start, toward.clone(), format!("{dots:?} quiet"));
        let mut before = start.clone();
        loop {
            assert!(engine.frame < PATIENCE, "{}: never quiet", engine.label);
            let frame = engine.frame;
            let (listed, tally) = engine.frame();
            let moved = engine
                .want
                .iter()
                .zip(&before)
                .filter(|(one, other)| one != other)
                .count();
            assert_eq!(tally[0] as usize, moved, "{} frame {frame}", engine.label);
            let positive = engine
                .want
                .iter()
                .filter(|&&word| embed::score(word, toward[0]) > 0)
                .count();
            let stepped = engine.reference.octs() as usize * CORNERS;
            assert!(tally[1] as usize <= positive.min(stepped));
            before.clone_from(&engine.want);
            if listed == 0 {
                break;
            }
        }
        for _ in 0..4 {
            assert_eq!(engine.frame().0, 0, "{}", engine.label);
        }
        println!(
            "embed {} width {} {dots:?}: quiet at frame {}",
            host.name(),
            host.width(),
            engine.frame
        );
    }
}

fn declared(module: &[u32]) -> Vec<u32> {
    let mut found = Vec::new();
    let mut at = 5;
    while at < module.len() {
        let word = module[at];
        let length = (word >> 16) as usize;
        if word & 0xFFFF == CAPABILITY {
            found.push(module[at + 1]);
        }
        at += length.max(1);
    }
    found
}

fn dotting(module: &[u32]) -> bool {
    let found = declared(module);
    let both = found.contains(&DOT_PRODUCT) && found.contains(&PACKED);
    let either = found.contains(&DOT_PRODUCT) || found.contains(&PACKED);
    assert_eq!(both, either, "{found:?}");
    both
}

#[test]
fn the_world_tools_build_the_embed_rule_on_the_path_the_host_offers() {
    let Some(host) = device("dot world") else {
        return;
    };
    let width = host.width();
    let grid = grit::sand::Grid::solid(16, 16, 16).unwrap();
    let plan = grit::compact::Plan::over_in(
        width,
        grid.blocks().div_ceil(grit::sweep::cells(width)),
        grit::layout::Dimension::Three,
    )
    .unwrap();
    for (rules, want) in [
        (
            grit::serve3::rules_on(Some(&host)),
            host.dots() == Dots::Native,
        ),
        (grit::serve3::rules(), false),
    ] {
        let embed = rules.into_iter().find(|one| one.name() == "embed").unwrap();
        let (passes, _) = embed.wired(width, grid, &plan).unwrap();
        assert_eq!(passes.iter().any(|pass| dotting(pass)), want);
        let other = grit::serve3::rules()
            .into_iter()
            .find(|one| one.name() == "nature")
            .unwrap();
        let (passes, _) = other.wired(width, grid, &plan).unwrap();
        assert!(!passes.iter().any(|pass| dotting(pass)));
    }
    println!("{} width {width}: {:?}", host.name(), host.dots());
}
