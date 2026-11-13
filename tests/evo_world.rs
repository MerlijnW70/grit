#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use grit::bricks::{self, Bricks};
use grit::compact::Plan;
use grit::layout::{COARSE, Dimension, GRAND, PARAMS, PIXELS, STATE, TALLY};
use grit::live;
use grit::ops::{Helpers, Ops};
use grit::question::{Answer, Asked, Layout, Map, Questions, Reduce, Region, Seen};
use grit::rule3::evo::{self, Evo};
use grit::rule3::{Declared, FIRST_PARAM, Oct, Rule};
use grit::sand::Grid;
use grit::sand3::{self, Grid3, lattice};
use grit::{Groups, Host, Resident, sweep};

const SHAPE: (u32, u32, u32) = (32, 32, 32);

const FRAMES: u32 = 1024;

const WAKE: u64 = 27;

const SEEDED: usize = 2;

fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut mixed = *state;
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    mixed ^ (mixed >> 31)
}

fn grids() -> (Grid, Grid3) {
    let (wide, deep, tall) = SHAPE;
    (
        Grid::solid(wide, deep, tall).unwrap(),
        Grid3::new(wide, deep, tall).unwrap(),
    )
}

fn tally_words() -> usize {
    Evo.tally().words()
}

struct Frame {
    cells: Vec<u32>,
    tally: Vec<u32>,
    moved: u64,
}

fn cpu_frame(grid: Grid3, frame: u32, cells: &[u32], params: &[u32]) -> Frame {
    let mut out = cells.to_vec();
    let mut tally = vec![0_u32; tally_words()];
    let mut moved = 0;
    for oct in 0..grid.octs() {
        let places = grid.corners(frame, oct);
        let words = places.map(|place| cells[place]);
        let at = lattice(grid, oct);
        let ruled = Evo.step(Oct {
            words,
            frame,
            at,
            params,
        });
        for (place, value) in places.into_iter().zip(ruled.out) {
            out[place] = value;
        }
        for (total, &count) in tally.iter_mut().zip(&ruled.counts) {
            *total = total.wrapping_add(count);
        }
        let changed = ruled.counts[evo::CHANGED];
        for (axis, value) in at.into_iter().enumerate() {
            let slot = &mut tally[evo::COUNTS + axis];
            *slot = slot.wrapping_add((value >> COARSE).wrapping_mul(changed));
        }
        if changed > 0 {
            tally[evo::COUNTS + 3] += 1;
            moved += 1;
        }
    }
    Frame {
        cells: out,
        tally,
        moved,
    }
}

fn living(cells: &[u32]) -> u64 {
    cells.iter().filter(|&&one| evo::alive_of(one)).count() as u64
}

fn clear(held: &mut Resident<'_>, sizes: &[usize]) {
    for (binding, &words) in sizes.iter().enumerate() {
        if binding == PIXELS as usize {
            continue;
        }
        held.write(binding, &vec![0_u32; words]).unwrap();
    }
}

fn dense_plan(width: u32, grid: Grid) -> Plan {
    Plan::over_in(
        width,
        grid.blocks().div_ceil(sweep::cells(width)),
        Dimension::Three,
    )
    .unwrap()
}

struct Dense<'host> {
    live: Resident<'host>,
    full: Resident<'host>,
    stages: Vec<Vec<Groups>>,
    whole: [Groups; 2],
    plan: Plan,
    grid: Grid,
}

impl<'host> Dense<'host> {
    fn new(host: &'host Host, grid: Grid) -> Self {
        let width = host.width();
        let plan = dense_plan(width, grid);
        let (passes, stages) = live::arranged3_by(&Evo, width, grid, &plan, false).unwrap();
        let built = sand3::settle_by(&Evo, width, grid, host.limits().groups).unwrap();
        let tick = live::tick(width).unwrap();
        let sizes = live::sizes3(&Evo, &plan);
        let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
        let live = host.resident(&sizes, &modules).unwrap();
        let modules: Vec<&[u32]> = vec![&tick, &built.words];
        let full = host.resident(&sizes, &modules).unwrap();
        Self {
            live,
            full,
            stages,
            whole: [Groups::Fixed(1), built.spread.dispatch()],
            plan,
            grid,
        }
    }

    fn restart(&mut self, start: &[u32], params: &[u32]) {
        let sizes = live::sizes3(&Evo, &self.plan);
        for held in [&mut self.live, &mut self.full] {
            clear(held, &sizes);
            held.write(STATE as usize, start).unwrap();
            held.write_at(PARAMS as usize, FIRST_PARAM, params).unwrap();
            held.write(PARAMS as usize, &[u32::MAX, self.plan.dump()])
                .unwrap();
        }
        live::seed(&mut self.live, self.grid, &self.plan).unwrap();
        for held in [&mut self.live, &mut self.full] {
            held.write(PARAMS as usize, &[u32::MAX]).unwrap();
        }
    }

    fn step(&mut self) -> (Vec<u32>, Vec<u32>, Vec<u32>, u64) {
        let cells = self.grid.cells();
        let groups: Vec<&[Groups]> = self.stages.iter().map(Vec::as_slice).collect();
        let (_, listed) = self
            .live
            .staged_reading(&groups, STATE as usize, cells)
            .unwrap();
        let tally = self.live.read(TALLY as usize, tally_words()).unwrap();
        let count = u64::from(self.live.read(GRAND as usize, 1).unwrap()[0]);
        let whole: Vec<&[Groups]> = self.whole.iter().map(core::slice::from_ref).collect();
        let (_, full) = self
            .full
            .staged_reading(&whole, STATE as usize, cells)
            .unwrap();
        (listed, full, tally, count)
    }
}

struct Sparse<'host> {
    held: Resident<'host>,
    stages: Vec<Vec<Groups>>,
    bricks: Bricks,
    plan: Plan,
    sizes: Vec<usize>,
    coarse: bool,
}

impl<'host> Sparse<'host> {
    fn new(host: &'host Host, grid: Grid, coarse: bool) -> Self {
        let width = host.width();
        let plan = Plan::bricked(width, grid.blocks().div_ceil(sweep::cells(width))).unwrap();
        let bricks = Bricks::new(grid, &plan, plan.blocks).unwrap();
        let (passes, stages) = bricks::arranged(&Evo, &bricks, &plan, coarse).unwrap();
        let sizes = bricks.sizes(&Evo, &plan);
        let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
        let held = host.resident(&sizes, &modules).unwrap();
        Self {
            held,
            stages,
            bricks,
            plan,
            sizes,
            coarse,
        }
    }

    fn restart(&mut self, start: &[u32], params: &[u32]) {
        clear(&mut self.held, &self.sizes);
        self.held
            .write(PARAMS as usize, &[u32::MAX, self.plan.dump()])
            .unwrap();
        self.held
            .write_at(PARAMS as usize, FIRST_PARAM, params)
            .unwrap();
        self.bricks
            .write(&mut self.held, &self.plan, start)
            .unwrap();
        self.bricks.seed(&mut self.held, &self.plan).unwrap();
    }

    fn step(&mut self) -> (Vec<u32>, Vec<u32>, u64) {
        let groups: Vec<&[Groups]> = self.stages.iter().map(Vec::as_slice).collect();
        self.held.staged(&groups).unwrap();
        let world = self.bricks.read(&mut self.held).unwrap();
        let tally = self.held.read(TALLY as usize, tally_words()).unwrap();
        let (octs, whole) = self.bricks.listed(&mut self.held).unwrap();
        let listed = u64::from(octs) + u64::from(whole) * u64::from(self.bricks.per_block());
        (world, tally, listed)
    }
}

fn parted(label: &str, grid: Grid3, frame: u32, got: &[u32], want: &[u32]) -> Option<String> {
    let at = (0..want.len()).find(|&at| got.get(at) != want.get(at))?;
    let plane = (grid.wide * grid.deep) as usize;
    let (z, rest) = (at / plane, at % plane);
    let (y, x) = (rest / grid.wide as usize, rest % grid.wide as usize);
    let count = (0..want.len())
        .filter(|&at| got.get(at) != want.get(at))
        .count();
    Some(format!(
        "{label}: frame {frame} cell ({x}, {y}, {z}): {:#x}, reference {:#x}; {count} cells differ",
        got.get(at).copied().unwrap_or_default(),
        want[at]
    ))
}

#[derive(Debug, Default, Clone)]
struct Trace {
    living: Vec<u64>,
    moved: Vec<u64>,
    listed: Vec<u64>,
    listed_sparse: Vec<[u64; 2]>,
    tallies: Vec<Vec<u32>>,
}

struct Engines<'host> {
    dense: Dense<'host>,
    sparse: [Sparse<'host>; 2],
    reference: Grid3,
}

impl<'host> Engines<'host> {
    fn new(host: &'host Host) -> Self {
        let (grid, reference) = grids();
        Self {
            dense: Dense::new(host, grid),
            sparse: [
                Sparse::new(host, grid, false),
                Sparse::new(host, grid, true),
            ],
            reference,
        }
    }

    fn run(
        &mut self,
        label: &str,
        start: &[u32],
        params: &[u32],
        frames: u32,
    ) -> (Vec<u32>, Trace) {
        self.dense.restart(start, params);
        for sparse in &mut self.sparse {
            sparse.restart(start, params);
        }
        let mut want = start.to_vec();
        let mut trace = Trace::default();
        for frame in 0..frames {
            trace.living.push(living(&want));
            let made = cpu_frame(self.reference, frame, &want, params);
            want = made.cells;
            let (listed, full, tally, count) = self.dense.step();
            let failed = parted(
                &format!("{label} whole"),
                self.reference,
                frame,
                &full,
                &want,
            )
            .or_else(|| {
                parted(
                    &format!("{label} listed"),
                    self.reference,
                    frame,
                    &listed,
                    &want,
                )
            });
            if let Some(failed) = failed {
                panic!("{failed}");
            }
            assert_eq!(tally, made.tally, "{label} listed tally, frame {frame}");
            let mut sparse_listed = [0; 2];
            for (slot, sparse) in sparse_listed.iter_mut().zip(&mut self.sparse) {
                let tier = if sparse.coarse { "coarse" } else { "fine" };
                let (world, tally, listed) = sparse.step();
                if let Some(failed) = parted(
                    &format!("{label} sparse {tier}"),
                    self.reference,
                    frame,
                    &world,
                    &want,
                ) {
                    panic!("{failed}");
                }
                assert_eq!(
                    tally, made.tally,
                    "{label} sparse {tier} tally, frame {frame}"
                );
                *slot = listed;
            }
            trace.moved.push(made.moved);
            trace.listed.push(count);
            trace.listed_sparse.push(sparse_listed);
            trace.tallies.push(made.tally);
        }
        (want, trace)
    }
}

fn follows(label: &str, trace: &Trace) {
    let frames = trace.listed.len();
    let mut worst = 0.0_f64;
    for frame in 0..frames {
        let recent = trace.moved[frame] + frame.checked_sub(1).map_or(0, |one| trace.moved[one]);
        let alive = trace.living[frame] + frame.checked_sub(1).map_or(0, |one| trace.living[one]);
        assert!(
            frame < SEEDED || trace.listed[frame] <= WAKE * recent,
            "{label}: frame {frame}: {} octs listed, {recent} octs changed in this frame and the one before",
            trace.listed[frame]
        );
        assert!(
            recent <= alive,
            "{label}: frame {frame}: {recent} octs changed with {alive} living cells"
        );
        if alive > 0 && frame >= SEEDED {
            worst = worst.max(trace.listed[frame] as f64 / alive as f64);
        }
    }
    let listed: u64 = trace.listed.iter().sum();
    let sparse: [u64; 2] = [0, 1].map(|tier| trace.listed_sparse.iter().map(|one| one[tier]).sum());
    let alive: u64 = trace.living.iter().sum();
    let moved: u64 = trace.moved.iter().sum();
    println!(
        "{label}: over {frames} frames, {listed} octs listed (sparse fine {}, coarse {}), {moved} octs changed, {alive} living cell-frames; listed per living cell {:.3} on average, at most {worst:.3}; living at start {}, at the end {}",
        sparse[0],
        sparse[1],
        listed as f64 / alive.max(1) as f64,
        trace.living.first().copied().unwrap_or(0),
        trace.living.last().copied().unwrap_or(0)
    );
}

fn colony(grid: Grid3, seed: u64, inside: impl Fn(u32, u32, u32) -> bool, share: u64) -> Vec<u32> {
    let mut state = seed;
    let mut cells = vec![0_u32; grid.cells()];
    for z in 0..grid.tall {
        for y in 0..grid.deep {
            for x in 0..grid.wide {
                let roll = next(&mut state);
                if inside(x, y, z) && roll % 100 < share {
                    let genome = evo::genome(
                        (roll >> 8) as u32 % 2,
                        (roll >> 10) as u32 % 2,
                        (roll >> 12) as u32 % 2,
                        (roll >> 14) as u32,
                    );
                    let energy = 40 + (roll >> 32) as u32 % 80;
                    let age = (roll >> 40) as u32 % 12;
                    cells[grid.index(x, y, z)] = evo::cell(genome, energy, age);
                } else if roll.is_multiple_of(7) {
                    cells[grid.index(x, y, z)] = ((roll >> 20) as u32) & !evo::ALIVE;
                }
            }
        }
    }
    cells
}

fn near_seam(at: u32, side: u32) -> bool {
    at < 2 || at + 2 >= side
}

fn agree(
    label: &str,
    start: impl Fn(Grid3) -> Vec<u32>,
    params: &[u32],
) -> Option<(Vec<u32>, Trace)> {
    let host = device(label)?;
    let mut engines = Engines::new(&host);
    let first = start(engines.reference);
    let name = format!("{label} on {} width {}", host.name(), host.width());
    let (last, trace) = engines.run(&name, &first, params, FRAMES);
    follows(&name, &trace);
    println!(
        "{name}: {FRAMES} frames, CPU, listed, whole, sparse fine and sparse coarse equal in every word and tally"
    );
    Some((last, trace))
}

#[test]
fn life_grown_from_the_corner_through_every_seam_is_the_same_listed_whole_sparse_and_on_the_cpu() {
    let start = |grid: Grid3| {
        colony(
            grid,
            0xC0_42E2,
            |x, y, z| near_seam(x, grid.wide) && near_seam(y, grid.deep) && near_seam(z, grid.tall),
            100,
        )
    };
    let Some((_, trace)) = agree("evo corner", start, &evo::DEFAULTS) else {
        return;
    };
    let first = trace.living[0];
    let most = trace.living.iter().copied().max().unwrap();
    assert!(first <= 64 && most > 50 * first, "{first} grew to {most}");
}

#[test]
fn life_on_the_three_seams_is_the_same_listed_whole_sparse_and_on_the_cpu() {
    let start = |grid: Grid3| {
        colony(
            grid,
            0x0005_EAA5,
            |x, y, z| near_seam(x, grid.wide) || near_seam(y, grid.deep) || near_seam(z, grid.tall),
            30,
        )
    };
    let mut params = evo::DEFAULTS;
    params[evo::MUTATION] = 2048;
    params[evo::SEED] = 0xDEAD_BEEF;
    let Some((_, trace)) = agree("evo seams", start, &params) else {
        return;
    };
    let births: u64 = trace
        .tallies
        .iter()
        .map(|one| u64::from(one[evo::BIRTHS]))
        .sum();
    let mutations: u64 = trace
        .tallies
        .iter()
        .map(|one| u64::from(one[evo::MUTATIONS]))
        .sum();
    assert!(births > 0 && mutations > 0);
}

#[test]
fn a_crowd_that_shrinks_is_the_same_listed_whole_sparse_and_on_the_cpu() {
    let start = |grid: Grid3| colony(grid, 0xC2_0D, |_, _, _| true, 95);
    let mut params = evo::DEFAULTS;
    params[evo::FEED] = 1;
    params[evo::HAZARD] = 2048;
    let Some((_, trace)) = agree("evo crowd", start, &params) else {
        return;
    };
    let first = trace.living[0];
    let last = *trace.living.last().unwrap();
    assert!(last > 0 && 2 * last < first, "{first} shrank to {last}");
}

#[test]
fn a_starved_world_dies_out_and_then_lists_nothing() {
    let start = |grid: Grid3| colony(grid, 0x57A_12E, |_, _, _| true, 40);
    let mut params = evo::DEFAULTS;
    params[evo::FEED] = 0;
    let Some((last, trace)) = agree("evo starved", start, &params) else {
        return;
    };
    assert_eq!(living(&last), 0);
    let dead = trace.living.iter().position(|&one| one == 0).unwrap();
    println!("evo starved: no living cell from frame {dead} on");
    assert!(dead < 128, "{dead}");
    for frame in dead + 2..trace.listed.len() {
        assert_eq!(trace.listed[frame], 0, "dense, frame {frame}");
        assert_eq!(trace.listed_sparse[frame], [0, 0], "sparse, frame {frame}");
        assert_eq!(
            trace.tallies[frame],
            vec![0; tally_words()],
            "frame {frame}"
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Probe {
    Alive,
    Genome(u32),
    Low,
    High,
}

impl Map for Probe {
    fn map<O: Ops>(&self, ops: &mut O, seen: Seen<'_, O::Word>) -> O::Word {
        let word = match seen.cell() {
            Some(word) => word,
            None => ops.constant(0),
        };
        let bit = ops.masked(word, evo::ALIVE);
        let alive = ops.is(bit, evo::ALIVE);
        let genome = ops.shift_right(word, evo::GENOME_SHIFT);
        let past = ops.constant(256);
        match *self {
            Self::Alive => alive.word(),
            Self::Genome(want) => {
                let same = ops.is(genome, want);
                ops.both(alive, same).word()
            }
            Self::Low => {
                let low = ops.masked(genome, 0xFF);
                ops.select(alive, low, past)
            }
            Self::High => {
                let high = ops.shift_right(genome, 8);
                ops.select(alive, high, past)
            }
        }
    }
}

struct Observed<'host> {
    live: Resident<'host>,
    stages: Vec<Vec<Groups>>,
    questions: Questions,
    asked: Vec<(Probe, Asked)>,
    first_pass: usize,
    plan: Plan,
    grid: Grid,
    sizes: Vec<usize>,
}

impl<'host> Observed<'host> {
    fn new(host: &'host Host, probes: &[(Probe, Reduce)]) -> Self {
        let width = host.width();
        let (grid, _) = grids();
        let plan = dense_plan(width, grid);
        let (passes, stages) = live::arranged3_by(&Evo, width, grid, &plan, false).unwrap();
        let bindings = plan.windows().laid(&live::sizes3(&Evo, &plan));
        let layout = Layout::Dense {
            grid,
            windows: plan.windows(),
        };
        let mut questions = Questions::new(layout, width, bindings.len(), 0).unwrap();
        let asked = probes
            .iter()
            .map(|&(probe, reduce)| (probe, questions.add(&probe, reduce).unwrap()))
            .collect();
        let mut sizes = bindings;
        sizes.extend(questions.sizes());
        let mut modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
        let first_pass = modules.len();
        modules.extend(questions.modules());
        let live = host.resident(&sizes, &modules).unwrap();
        let mut stages = stages;
        stages.push(questions.resting());
        Self {
            live,
            stages,
            questions,
            asked,
            first_pass,
            plan,
            grid,
            sizes,
        }
    }

    fn restart(&mut self, start: &[u32], params: &[u32]) {
        for (binding, &room) in self.sizes.iter().enumerate() {
            self.live.write(binding, &vec![0; room]).unwrap();
        }
        self.live
            .write(PARAMS as usize, &[u32::MAX, self.plan.dump()])
            .unwrap();
        self.live
            .write_at(PARAMS as usize, FIRST_PARAM, params)
            .unwrap();
        live::write_state(&mut self.live, self.plan.windows(), 0, start).unwrap();
        live::seed(&mut self.live, self.grid, &self.plan).unwrap();
    }

    fn params(&mut self, params: &[u32]) {
        self.live
            .write_at(PARAMS as usize, FIRST_PARAM, params)
            .unwrap();
    }

    fn step(&mut self, frames: u32) {
        let groups: Vec<&[Groups]> = self.stages.iter().map(Vec::as_slice).collect();
        self.live.staged_frames(&groups, frames).unwrap();
    }

    fn step_tally(&mut self) -> (Vec<u32>, u64) {
        let groups: Vec<&[Groups]> = self.stages.iter().map(Vec::as_slice).collect();
        let (_, tally) = self
            .live
            .staged_reading(&groups, TALLY as usize, tally_words())
            .unwrap();
        let listed = u64::from(self.live.read(GRAND as usize, 1).unwrap()[0]);
        (tally, listed)
    }

    fn ask(&mut self, probe: Probe) -> Answer {
        let (_, asked) = *self.asked.iter().find(|(one, _)| *one == probe).unwrap();
        let size = self.questions.layout().size();
        self.questions
            .ask(&mut self.live, self.first_pass, asked, Region::whole(size))
            .unwrap()
    }

    fn count(&mut self, probe: Probe) -> u64 {
        match self.ask(probe) {
            Answer::Count(count) => count,
            other => panic!("{probe:?}: {other:?}"),
        }
    }

    fn distinct(&mut self, probe: Probe) -> (usize, u64) {
        match self.ask(probe) {
            Answer::Histogram { bins, past } => (bins.iter().filter(|&&one| one > 0).count(), past),
            other => panic!("{probe:?}: {other:?}"),
        }
    }
}

fn diversity_probes() -> Vec<(Probe, Reduce)> {
    vec![
        (Probe::Alive, Reduce::Count),
        (Probe::Low, Reduce::Histogram { bins: 256 }),
        (Probe::High, Reduce::Histogram { bins: 256 }),
    ]
}

const TURNOVER_FRAMES: u32 = 4096;

const WARM_UP: u32 = 512;

const WINDOW: u32 = 256;

const FEWEST_SHARE: f64 = 0.6;

const MOST_SHARE: f64 = 0.8;

#[test]
fn a_fed_population_turns_over_within_bounds_and_a_starved_one_dies_out_and_costs_nothing() {
    let Some(host) = device("evo turnover") else {
        return;
    };
    let (_, reference) = grids();
    let cells = reference.cells() as f64;
    let mut world = Observed::new(&host, &diversity_probes());
    let start = colony(reference, 0x7_0E2, |_, _, _| true, 10);
    let mut params = evo::DEFAULTS;
    world.restart(&start, &params);
    let mut windows = Vec::new();
    let mut window = [0_u64; 4];
    let mut fewest = u64::MAX;
    let mut most = 0;
    for frame in 0..TURNOVER_FRAMES {
        let (tally, _) = world.step_tally();
        let alive = u64::from(tally[evo::LIVING]);
        window[0] += u64::from(tally[evo::BIRTHS]);
        window[1] += u64::from(tally[evo::DEATHS]);
        window[2] += u64::from(tally[evo::MUTATIONS]);
        window[3] = alive;
        if frame >= WARM_UP {
            fewest = fewest.min(alive);
            most = most.max(alive);
        }
        if (frame + 1) % WINDOW == 0 {
            let counted = world.count(Probe::Alive);
            assert_eq!(counted, alive, "frame {frame}: question and tally disagree");
            let (low, past_low) = world.distinct(Probe::Low);
            let (high, past_high) = world.distinct(Probe::High);
            assert_eq!(past_low, reference.cells() as u64 - counted);
            assert_eq!(past_high, past_low);
            println!(
                "frame {}: living {counted} ({:.1} %), births {} deaths {} mutations {} in the last {WINDOW} frames; {low} distinct low genome bytes, {high} distinct high bytes",
                frame + 1,
                100.0 * counted as f64 / cells,
                window[0],
                window[1],
                window[2]
            );
            if frame >= WARM_UP {
                windows.push(window);
            }
            window = [0; 4];
        }
    }
    assert_eq!(windows.len() as u32, (TURNOVER_FRAMES - WARM_UP) / WINDOW);
    for (at, one) in windows.iter().enumerate() {
        assert!(one[0] > 0 && one[1] > 0, "window {at}: {one:?}");
    }
    println!(
        "after {WARM_UP} frames of warm-up, living stays within {fewest}..{most} of {} cells ({:.1} % to {:.1} %); stated bounds {:.0} % to {:.0} %",
        reference.cells(),
        100.0 * fewest as f64 / cells,
        100.0 * most as f64 / cells,
        100.0 * FEWEST_SHARE,
        100.0 * MOST_SHARE
    );
    assert!(fewest as f64 >= FEWEST_SHARE * cells && most as f64 <= MOST_SHARE * cells);

    params[evo::FEED] = 0;
    world.params(&params);
    let mut frames = 0;
    loop {
        let (tally, listed) = world.step_tally();
        frames += 1;
        assert!(frames < 256, "still {} alive", tally[evo::LIVING]);
        if tally[evo::LIVING] == 0 {
            println!(
                "with no food every cell is dead after {frames} frames; {listed} octs listed then"
            );
            break;
        }
    }
    let mut after = Vec::new();
    for _ in 0..8 {
        after.push(world.step_tally().1);
    }
    println!("octs listed in the frames after: {after:?}");
    assert_eq!(after[2..], [0; 6]);
    assert_eq!(world.count(Probe::Alive), 0);
}

fn mutation_run(host: &Host, rate: u32) -> (u64, u64, usize, usize) {
    let mut world = Observed::new(host, &diversity_probes());
    let (_, reference) = grids();
    let mut state = 0x307_A7E ^ u64::from(rate);
    let founders = [evo::genome(0, 0, 0, 0x11), evo::genome(1, 1, 0, 0x2A2)];
    let start: Vec<u32> = (0..reference.cells())
        .map(|_| {
            let roll = next(&mut state);
            if roll.is_multiple_of(10) {
                evo::cell(founders[(roll >> 8) as usize % 2], 60, 0)
            } else {
                0
            }
        })
        .collect();
    let mut params = evo::DEFAULTS;
    params[evo::MUTATION] = rate;
    world.restart(&start, &params);
    let (low_start, _) = world.distinct(Probe::Low);
    let (high_start, _) = world.distinct(Probe::High);
    let mut births = 0;
    let mut mutations = 0;
    for _ in 0..FRAMES {
        let (tally, _) = world.step_tally();
        births += u64::from(tally[evo::BIRTHS]);
        mutations += u64::from(tally[evo::MUTATIONS]);
    }
    let (low, _) = world.distinct(Probe::Low);
    let (high, _) = world.distinct(Probe::High);
    assert_eq!((low_start, high_start), (2, 2));
    (births, mutations, low, high)
}

#[test]
fn with_mutation_rate_zero_no_new_genome_appears_on_the_device() {
    let Some(host) = device("evo rate zero") else {
        return;
    };
    let (births, mutations, low, high) = mutation_run(&host, 0);
    println!(
        "rate 0 on {} width {}: {births} births over {FRAMES} frames, {mutations} mutations, {low} low and {high} high genome bytes",
        host.name(),
        host.width()
    );
    assert!(births > 10_000);
    assert_eq!(mutations, 0);
    assert!(low <= 2 && high <= 2);
}

#[test]
fn the_measured_mutation_count_matches_the_rate_within_the_binomial_interval() {
    let Some(host) = device("evo rate") else {
        return;
    };
    for rate in [16_u32, 256, 4096] {
        let (births, mutations, low, high) = mutation_run(&host, rate);
        let p = f64::from(rate) / f64::from(evo::CHANCES);
        let trials = births as f64 * f64::from(evo::GENOME_BITS);
        let mean = trials * p;
        let sigma = (trials * p * (1.0 - p)).sqrt();
        let z = (mutations as f64 - mean) / sigma;
        println!(
            "rate {rate}/65536 on {} width {}: {births} births, {trials} bit draws, {mutations} flips; expected {mean:.1}, sigma {sigma:.1}, z {z:.2}; {low} low and {high} high genome bytes",
            host.name(),
            host.width()
        );
        assert!(z.abs() <= 4.0, "rate {rate}: z {z:.2} outside 4 sigma");
        assert!(low > 2 && high > 2);
    }
}

const SELECTION_SEEDS: u64 = 40;

const SELECTION_FRAMES: u32 = 128;

const Z_999: f64 = 3.29;

#[derive(Debug, Clone, Copy)]
struct Contest {
    name: &'static str,
    fitter: u32,
    other: u32,
    hazard: u32,
}

fn shares(host: &Host, contest: Contest) -> Vec<f64> {
    let probes = [
        (Probe::Genome(contest.fitter), Reduce::Count),
        (Probe::Genome(contest.other), Reduce::Count),
    ];
    let mut world = Observed::new(host, &probes);
    let (_, reference) = grids();
    let mut out = Vec::new();
    for seed in 0..SELECTION_SEEDS {
        let mut state = 0x5E1E_C700 + seed;
        let mut turn = 0;
        let start: Vec<u32> = (0..reference.cells())
            .map(|_| {
                let roll = next(&mut state);
                if roll.is_multiple_of(4) {
                    turn += 1;
                    let genome = if turn % 2 == 0 {
                        contest.fitter
                    } else {
                        contest.other
                    };
                    evo::cell(
                        genome,
                        60 + (roll >> 8) as u32 % 40,
                        (roll >> 16) as u32 % 16,
                    )
                } else {
                    0
                }
            })
            .collect();
        let mut params = evo::DEFAULTS;
        params[evo::MUTATION] = 0;
        params[evo::HAZARD] = contest.hazard;
        params[evo::SEED] = next(&mut state) as u32;
        world.restart(&start, &params);
        let first = world.count(Probe::Genome(contest.fitter));
        let rival = world.count(Probe::Genome(contest.other));
        assert!(first.abs_diff(rival) <= 1, "{first} against {rival}");
        world.step(SELECTION_FRAMES);
        let fit = world.count(Probe::Genome(contest.fitter)) as f64;
        let other = world.count(Probe::Genome(contest.other)) as f64;
        assert!(fit + other > 0.0, "{} seed {seed} died out", contest.name);
        out.push(fit / (fit + other));
    }
    out
}

fn interval(values: &[f64]) -> (f64, f64, f64) {
    let n = values.len() as f64;
    let mean = values.iter().sum::<f64>() / n;
    let var = values.iter().map(|one| (one - mean).powi(2)).sum::<f64>() / (n - 1.0);
    let half = Z_999 * (var / n).sqrt();
    (mean, mean - half, mean + half)
}

#[test]
fn the_fitter_genotype_rises_on_forty_seeds_and_a_neutral_pair_stays_level() {
    let Some(host) = device("evo selection") else {
        return;
    };
    let contests = [
        Contest {
            name: "no hazard: a guard is upkeep for nothing",
            fitter: evo::genome(1, 0, 0, 0x15),
            other: evo::genome(1, 0, 1, 0x2A),
            hazard: 0,
        },
        Contest {
            name: "hazard 8192: a guard pays for its upkeep",
            fitter: evo::genome(1, 0, 2, 0x15),
            other: evo::genome(1, 0, 0, 0x2A),
            hazard: 8192,
        },
        Contest {
            name: "control: two markers, no fitness difference",
            fitter: evo::genome(1, 0, 0, 0x15),
            other: evo::genome(1, 0, 0, 0x2A),
            hazard: 0,
        },
    ];
    for (at, contest) in contests.iter().enumerate() {
        let found = shares(&host, *contest);
        let rose = found.iter().filter(|&&one| one > 0.5).count();
        let (mean, low, high) = interval(&found);
        println!(
            "{} on {} width {}: share of the first genotype after {SELECTION_FRAMES} frames over {SELECTION_SEEDS} seeds: mean {mean:.4}, 99.9 % interval {low:.4}..{high:.4}, rose on {rose} of {SELECTION_SEEDS}; least {:.4}, most {:.4}",
            contest.name,
            host.name(),
            host.width(),
            found.iter().copied().fold(f64::MAX, f64::min),
            found.iter().copied().fold(f64::MIN, f64::max)
        );
        if at < 2 {
            assert!(low > 0.5, "{}: {low:.4}", contest.name);
            assert!(rose >= 35, "{}: {rose}", contest.name);
        } else {
            assert!(
                low < 0.5 && 0.5 < high,
                "{}: {low:.4}..{high:.4}",
                contest.name
            );
            assert!((10..=30).contains(&rose), "{}: {rose}", contest.name);
        }
    }
}
