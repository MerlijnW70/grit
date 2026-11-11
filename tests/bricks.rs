#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use grit::bricks::{self, Bricks, ENTRY, PoolUse, UNIFORM};
use grit::compact::Plan;
use grit::geometry::Solid;
use grit::layout::{Dimension, PARAMS, PIXELS, STATE};
use grit::live;
use grit::ops::{Helpers, Ops};
use grit::rule3::code::{self, Code};
use grit::rule3::heat::{self, Heat};
use grit::rule3::nature::{self, Nature};
use grit::rule3::sand::Sand;
use grit::rule3::{Declared, FIRST_PARAM, Oct, OneStep, Rule, Ruled, Tally};
use grit::sand::{self as sand2, Grid};
use grit::sand3::{self, Grid3, reference};
use grit::sweep;
use grit::{Error, Groups, Host, Resident};
use simdr::lanes::LaneError;

const SEEDS: [u64; 2] = [3, 0x5EED];

const FRAMES: u32 = 48;

fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut mixed = *state;
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    mixed ^ (mixed >> 31)
}

struct Subject<R: Rule> {
    name: &'static str,
    rule: R,
    params: Vec<u32>,
    cell: fn(&mut u64) -> u32,
    calm: u32,
    ground: u32,
}

fn sand_cell(state: &mut u64) -> u32 {
    let roll = next(state);
    let kind = match roll % 20 {
        0..8 => sand2::AIR,
        8..12 => sand2::WATER,
        12..17 => sand2::SAND,
        _ => sand2::WALL,
    };
    if kind == sand2::WALL {
        return sand2::WALL;
    }
    kind + ((((roll >> 8) % 128) as u32) << 2)
}

fn heat_cell(state: &mut u64) -> u32 {
    let roll = next(state);
    let kind = match roll % 32 {
        0..10 => heat::AIR,
        10..12 => heat::WALL,
        12..15 => heat::ICE,
        15..19 => heat::WATER,
        19..22 => heat::STEAM,
        22..26 => heat::FUEL,
        26..28 => heat::FIRE,
        _ => heat::ASH,
    };
    let temperature = ((roll >> 8) % 1200) as u32;
    let life = ((roll >> 20) % 16) as u32;
    heat::cell(kind, temperature) | (life << 4)
}

fn code_cell(state: &mut u64) -> u32 {
    let roll = next(state);
    let instruction = match roll % 50 {
        0..20 => code::NOP,
        20..27 => code::DATA,
        27..36 => code::MOVE,
        36 => code::COPY,
        37..42 => code::ADD,
        42..45 => code::TURN,
        _ => code::HALT,
    };
    let way = ((roll >> 8) % 8) as u32;
    let payload = (roll >> 16) as u32;
    code::word(instruction, way, payload)
}

fn nature_cell(state: &mut u64) -> u32 {
    let roll = next(state);
    let kind = match roll % 38 {
        0..12 => nature::AIR,
        12..14 => nature::ROCK,
        14..16 => nature::EARTH,
        16..18 => nature::SAND,
        18..20 => nature::SNOW,
        20..21 => nature::ICE,
        21..25 => nature::WATER,
        25..27 => nature::WOOD,
        27..29 => nature::LEAVES,
        29..31 => nature::FIRE,
        31..33 => nature::ASH,
        33..35 => nature::SMOKE,
        35..37 => nature::STEAM,
        _ => nature::CHAR,
    };
    let temperature = ((roll >> 8) % 1200) as u32;
    nature::cell(kind, temperature)
}

fn sand() -> Subject<Sand> {
    Subject {
        name: "sand",
        rule: Sand,
        params: Vec::new(),
        cell: sand_cell,
        calm: sand2::AIR,
        ground: sand2::WALL,
    }
}

fn heated() -> Subject<Heat> {
    Subject {
        name: "heat",
        rule: Heat,
        params: heat::TUNED.to_vec(),
        cell: heat_cell,
        calm: heat::cell(heat::AIR, 300),
        ground: heat::cell(heat::WALL, 300),
    }
}

fn coded() -> Subject<Code> {
    Subject {
        name: "code",
        rule: Code,
        params: Vec::new(),
        cell: code_cell,
        calm: code::word(code::NOP, 0, 0),
        ground: code::word(code::HALT, 0, 0),
    }
}

fn natured() -> Subject<Nature> {
    Subject {
        name: "nature",
        rule: Nature,
        params: Vec::new(),
        cell: nature_cell,
        calm: nature::air(260, 0),
        ground: nature::cell(nature::ROCK, 260),
    }
}

const TICKS: u32 = 40;

const STILL: u32 = 0x100;

struct Ticking;

impl Declared for Ticking {
    fn params(&self) -> usize {
        0
    }

    fn tally(&self) -> Tally {
        Tally {
            counts: 1,
            weighted: false,
        }
    }
}

impl OneStep for Ticking {
    fn one_step<O: Ops>(&self, ops: &mut O, oct: Oct<'_, O::Word>) -> Ruled<O::Word> {
        let one = ops.constant(1);
        let last = ops.constant(TICKS);
        let mut ticked = Vec::new();
        let out = oct.words.map(|word| {
            let count = ops.masked(word, 0xFF);
            let short = ops.less(count, last);
            ticked.push(short);
            let up = ops.add(word, one);
            ops.select(short, up, word)
        });
        let counts = vec![ops.count(ticked)];
        Ruled { out, counts }
    }
}

fn ticking() -> Subject<Ticking> {
    Subject {
        name: "ticking",
        rule: Ticking,
        params: Vec::new(),
        cell: |state| (next(state) as u32 & 0x3) | STILL,
        calm: 0,
        ground: TICKS | STILL,
    }
}

fn clocks(grid: Grid3, side: [u32; 3], seed: u64) -> Vec<u32> {
    let mut cells = vec![TICKS | STILL; grid.cells()];
    for z in 0..grid.tall {
        for y in 0..grid.deep {
            for x in 0..grid.wide {
                let brick = (x / side[0]) + (y / side[1]) + (z / side[2]);
                if (brick + seed.count_ones()).is_multiple_of(2) {
                    cells[grid.index(x, y, z)] = 0;
                }
            }
        }
    }
    cells
}

fn grids(wide: u32, deep: u32, tall: u32) -> (Grid, Grid3) {
    (
        Grid::solid(wide, deep, tall).unwrap(),
        Grid3::new(wide, deep, tall).unwrap(),
    )
}

fn plan_of(width: u32, grid: Grid) -> Plan {
    Plan::over_in(
        width,
        grid.blocks().div_ceil(sweep::cells(width)),
        Dimension::Three,
    )
    .unwrap()
}

fn side_of(width: u32, grid: Grid) -> [u32; 3] {
    let plan = plan_of(width, grid);
    Bricks::new(grid, &plan, 1).unwrap().sides()
}

fn world_for(width: u32) -> (u32, u32, u32) {
    let k = sweep::cells(width).trailing_zeros();
    let sides = [2 << k.div_ceil(3), 2 << ((k + 1) / 3), 2 << (k / 3)];
    (sides[0] * 4, sides[1] * 4, sides[2] * 2)
}

fn scene<R: Rule>(subject: &Subject<R>, grid: Grid3, side: [u32; 3], seed: u64) -> Vec<u32> {
    let mut state = seed;
    let mut cells = vec![subject.calm; grid.cells()];
    let floor = grid.deep - side[1];
    let cloud = [
        (side[0] / 2, side[0] / 2 + side[0]),
        (side[1] / 2, side[1] / 2 + side[1]),
        (side[2] / 2, side[2] / 2 + side[2]),
    ];
    for z in 0..grid.tall {
        for y in 0..grid.deep {
            for x in 0..grid.wide {
                let at = grid.index(x, y, z);
                let roll = (subject.cell)(&mut state);
                if y >= floor {
                    cells[at] = subject.ground;
                } else if (cloud[0].0..cloud[0].1).contains(&x)
                    && (cloud[1].0..cloud[1].1).contains(&y)
                    && (cloud[2].0..cloud[2].1).contains(&z)
                {
                    cells[at] = roll;
                }
            }
        }
    }
    cells
}

fn rain(grid: Grid3, side: [u32; 3], seed: u64) -> Vec<u32> {
    let mut cells = vec![sand2::AIR; grid.cells()];
    let floor = grid.deep - side[1];
    for z in 0..grid.tall {
        for y in floor..grid.deep {
            for x in 0..grid.wide {
                cells[grid.index(x, y, z)] = sand2::WALL;
            }
        }
    }
    for x in 0..grid.wide {
        let column = u64::from(x / side[0]) + u64::from(seed.count_ones());
        let apart = side[1] + 1 + u32::try_from(column % 2).unwrap();
        for sheet in 0..3 {
            let y = sheet * apart;
            if y >= floor {
                continue;
            }
            for z in 0..grid.tall {
                cells[grid.index(x, y, z)] = sand2::SAND;
            }
        }
    }
    cells
}

fn pile(grid: Grid3, side: [u32; 3], seed: u64) -> Vec<u32> {
    let mut cells = vec![sand2::AIR; grid.cells()];
    let floor = grid.deep - side[1];
    for z in 0..grid.tall {
        for y in floor..grid.deep {
            for x in 0..grid.wide {
                cells[grid.index(x, y, z)] = sand2::WALL;
            }
        }
    }
    let shift = (seed.count_ones() % 2) * side[0] / 2;
    for z in 0..side[2] / 2 {
        for y in 0..side[1] / 2 {
            for x in shift + 1..shift + side[0] - 1 {
                cells[grid.index(x, y, z)] = sand2::SAND;
            }
        }
    }
    cells
}

fn clear(held: &mut Resident<'_>, sizes: &[usize]) {
    for (binding, &words) in sizes.iter().enumerate() {
        if binding == PIXELS as usize {
            continue;
        }
        held.write(binding, &vec![0_u32; words]).unwrap();
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Fine,
    Coarse,
    Whole,
}

const MODES: [Mode; 3] = [Mode::Fine, Mode::Coarse, Mode::Whole];

struct Sparse<'host> {
    held: Resident<'host>,
    stages: Vec<Vec<Groups>>,
    bricks: Bricks,
    plan: Plan,
    grid: Grid,
    sizes: Vec<usize>,
    mode: Mode,
}

impl<'host> Sparse<'host> {
    fn new<R: Rule>(
        host: &'host Host,
        rule: &R,
        grid: Grid,
        mode: Mode,
        capacity: Option<u32>,
    ) -> Self {
        let width = host.width();
        let plan = plan_of(width, grid);
        let bricks = Bricks::new(grid, &plan, capacity.unwrap_or(plan.blocks)).unwrap();
        Self::with(host, rule, bricks, plan, mode)
    }

    fn with<R: Rule>(host: &'host Host, rule: &R, bricks: Bricks, plan: Plan, mode: Mode) -> Self {
        let (passes, stages) = match mode {
            Mode::Fine => bricks::arranged(rule, &bricks, &plan, false).unwrap(),
            Mode::Coarse => bricks::arranged(rule, &bricks, &plan, true).unwrap(),
            Mode::Whole => bricks::whole(rule, &bricks, &plan).unwrap(),
        };
        let sizes = bricks.sizes(rule, &plan);
        let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
        let held = host.resident(&sizes, &modules).unwrap();
        Self {
            held,
            stages,
            bricks,
            plan,
            grid: bricks.grid(),
            sizes,
            mode,
        }
    }

    fn restart(&mut self, start: &[u32], params: &[u32], frame: u32) -> PoolUse {
        clear(&mut self.held, &self.sizes);
        self.held
            .write(PARAMS as usize, &[u32::MAX, self.plan.dump()])
            .unwrap();
        if !params.is_empty() {
            self.held
                .write_at(PARAMS as usize, FIRST_PARAM, params)
                .unwrap();
        }
        if self.mode != Mode::Whole {
            live::seed(&mut self.held, self.grid, &self.plan).unwrap();
        }
        self.held
            .write(PARAMS as usize, &[frame.wrapping_sub(1)])
            .unwrap();
        self.bricks
            .write(&mut self.held, &self.plan, start)
            .unwrap()
    }

    fn step(&mut self) {
        let groups: Vec<&[Groups]> = self.stages.iter().map(Vec::as_slice).collect();
        self.held.staged(&groups).unwrap();
    }

    fn world(&mut self) -> Result<Vec<u32>, Error> {
        self.bricks.read(&mut self.held)
    }

    fn raw(&mut self) -> Vec<u32> {
        let table = self
            .held
            .read(self.bricks.table() as usize, self.bricks.table_words())
            .unwrap();
        let pool =
            live::read_state(&mut self.held, self.bricks.windows(), self.bricks.scratch()).unwrap();
        self.bricks.unpack(&table, &pool)
    }

    fn sound(&mut self) -> Result<(), String> {
        let capacity = self.bricks.capacity();
        let table = self
            .held
            .read(self.bricks.table() as usize, self.bricks.table_words())
            .unwrap();
        let header = self
            .held
            .read(self.bricks.slots() as usize, bricks::HEAD)
            .unwrap();
        let top = header[bricks::TOP];
        if top > capacity {
            return Err(format!("{top} free slots in a pool of {capacity}"));
        }
        let stack = self
            .held
            .read(self.bricks.slots() as usize, bricks::HEAD + top as usize)
            .unwrap();
        let mut owner = vec![None; capacity as usize];
        for brick in 0..self.bricks.count() {
            let slot = table[brick as usize * ENTRY];
            if slot == UNIFORM {
                continue;
            }
            let Some(seat) = owner.get_mut(slot as usize) else {
                return Err(format!("brick {brick} holds slot {slot} past the pool"));
            };
            if let Some(other) = seat.replace(brick) {
                return Err(format!("bricks {other} and {brick} share slot {slot}"));
            }
        }
        for &slot in &stack[bricks::HEAD..] {
            let Some(seat) = owner.get_mut(slot as usize) else {
                return Err(format!("free slot {slot} past the pool"));
            };
            if let Some(brick) = seat.replace(u32::MAX) {
                return Err(format!("slot {slot} is free and held by {brick}"));
            }
        }
        if let Some(lost) = owner.iter().position(Option::is_none) {
            return Err(format!("slot {lost} is neither held nor free"));
        }
        Ok(())
    }
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
    fn new<R: Rule>(host: &'host Host, rule: &R, grid: Grid) -> Self {
        let width = host.width();
        let plan = plan_of(width, grid);
        let (passes, stages) = live::arranged3_by(rule, width, grid, &plan, true).unwrap();
        let built = sand3::settle_by(rule, width, grid, host.limits().groups).unwrap();
        let tick = live::tick(width).unwrap();
        let sizes = live::sizes3(rule, &plan);
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

    fn restart(&mut self, start: &[u32], params: &[u32], frame: u32) {
        let sizes = self.plan.sizes();
        for held in [&mut self.live, &mut self.full] {
            clear(held, &sizes);
            held.write(STATE as usize, start).unwrap();
            if !params.is_empty() {
                held.write_at(PARAMS as usize, FIRST_PARAM, params).unwrap();
            }
            held.write(PARAMS as usize, &[u32::MAX, self.plan.dump()])
                .unwrap();
        }
        live::seed(&mut self.live, self.grid, &self.plan).unwrap();
        for held in [&mut self.live, &mut self.full] {
            held.write(PARAMS as usize, &[frame.wrapping_sub(1)])
                .unwrap();
        }
    }

    fn step(&mut self) -> (Vec<u32>, Vec<u32>) {
        let cells = self.grid.cells();
        let groups: Vec<&[Groups]> = self.stages.iter().map(Vec::as_slice).collect();
        let (_, listed) = self
            .live
            .staged_reading(&groups, STATE as usize, cells)
            .unwrap();
        let whole: Vec<&[Groups]> = self.whole.iter().map(core::slice::from_ref).collect();
        let (_, full) = self
            .full
            .staged_reading(&whole, STATE as usize, cells)
            .unwrap();
        (listed, full)
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

struct Run {
    mode: Mode,
    lowest: u32,
    highest: u32,
    last: u32,
}

type Pooling<'held> = dyn Fn(Grid, &Plan) -> Bricks + 'held;

fn agree<R: Rule>(
    host: &Host,
    subject: &Subject<R>,
    shape: (u32, u32, u32),
    start: &dyn Fn(Grid3, [u32; 3], u64) -> Vec<u32>,
    frames: u32,
) -> Vec<Run> {
    let pooled = |grid: Grid, plan: &Plan| Bricks::new(grid, plan, plan.blocks).unwrap();
    agree_in(host, subject, shape, start, frames, &pooled)
}

fn agree_in<R: Rule>(
    host: &Host,
    subject: &Subject<R>,
    shape: (u32, u32, u32),
    start: &dyn Fn(Grid3, [u32; 3], u64) -> Vec<u32>,
    frames: u32,
    pooled: &Pooling<'_>,
) -> Vec<Run> {
    let width = host.width();
    let (grid, reference3) = grids(shape.0, shape.1, shape.2);
    let side = side_of(width, grid);
    let mut dense = Dense::new(host, &subject.rule, grid);
    let mut runs = Vec::new();
    for mode in MODES {
        let plan = plan_of(width, grid);
        let pool = pooled(grid, &plan);
        let mut sparse = Sparse::with(host, &subject.rule, pool, plan, mode);
        for seed in SEEDS {
            let label = format!(
                "{} {} width {width} {}x{}x{} {mode:?} windows {} seed {seed:#x}",
                subject.name,
                host.name(),
                shape.0,
                shape.1,
                shape.2,
                sparse.bricks.windows().count()
            );
            let first = start(reference3, side, seed);
            let opened = sparse.restart(&first, &subject.params, 0);
            dense.restart(&first, &subject.params, 0);
            let mut want = first.clone();
            let mut run = Run {
                mode,
                lowest: opened.held,
                highest: opened.held,
                last: opened.held,
            };
            for frame in 0..frames {
                sparse.step();
                let (listed, full) = dense.step();
                want = reference(&subject.rule, reference3, frame, &want, &subject.params);
                let world = sparse.world().unwrap_or_else(|why| {
                    let raw = sparse.raw();
                    let differs = parted("held as they stand", reference3, frame, &raw, &want)
                        .unwrap_or_default();
                    panic!("{label}: frame {frame}: {why}; {differs}")
                });
                if let Err(why) = sparse.sound() {
                    panic!("{label}: frame {frame}: the pool is not sound: {why}");
                }
                let failed = parted(
                    &format!("{label} dense whole"),
                    reference3,
                    frame,
                    &full,
                    &want,
                )
                .or_else(|| {
                    parted(
                        &format!("{label} dense listed"),
                        reference3,
                        frame,
                        &listed,
                        &want,
                    )
                })
                .or_else(|| parted(&format!("{label} sparse"), reference3, frame, &world, &want));
                if let Some(failed) = failed {
                    panic!("{failed}");
                }
                let used = sparse.bricks.check(&mut sparse.held).unwrap();
                run.lowest = run.lowest.min(used.held);
                run.highest = run.highest.max(used.held);
                run.last = used.held;
            }
            assert_ne!(want, first, "{label}: nothing moved");
            println!(
                "{label}: {frames} frames equal; {} bricks, {} held at start, {}..{} held, {} at the end",
                sparse.bricks.count(),
                opened.held,
                run.lowest,
                run.highest,
                run.last
            );
            runs.push(run);
        }
    }
    runs
}

fn matrix<R: Rule>(subject: &Subject<R>) {
    let name = format!("{} bricks", subject.name);
    let Some(host) = device(&name) else {
        return;
    };
    let shape = world_for(host.width());
    let start = |grid: Grid3, side: [u32; 3], seed: u64| scene(subject, grid, side, seed);
    let runs = agree(&host, subject, shape, &start, FRAMES);
    assert!(
        runs.iter().any(|run| run.highest > run.lowest),
        "{name}: no brick was ever taken"
    );
}

#[test]
fn sparse_sand_steps_to_the_dense_words_listed_and_whole() {
    matrix(&sand());
}

#[test]
fn sparse_heat_steps_to_the_dense_words_listed_and_whole() {
    matrix(&heated());
}

#[test]
fn sparse_code_steps_to_the_dense_words_listed_and_whole() {
    matrix(&coded());
}

#[test]
fn sparse_nature_steps_to_the_dense_words_listed_and_whole() {
    matrix(&natured());
}

#[test]
fn a_pile_falls_through_an_air_brick_that_is_taken_and_freed_again() {
    let name = "bricks pile";
    let Some(host) = device(name) else {
        return;
    };
    let shape = world_for(host.width());
    let start = |grid: Grid3, side: [u32; 3], seed: u64| pile(grid, side, seed);
    let runs = agree(&host, &sand(), shape, &start, 96);
    for run in &runs {
        assert!(
            run.highest > run.lowest,
            "{name}: the falling pile took no brick"
        );
        if run.mode == Mode::Whole {
            continue;
        }
        assert!(
            run.last < run.highest,
            "{name} {:?}: no brick the pile passed was freed: {} held at most, {} at the end",
            run.mode,
            run.highest,
            run.last
        );
    }
}

#[test]
fn sheets_that_leave_a_brick_empty_for_one_frame_step_to_the_dense_words() {
    let name = "bricks rain";
    let Some(host) = device(name) else {
        return;
    };
    let shape = world_for(host.width());
    let start = |grid: Grid3, side: [u32; 3], seed: u64| rain(grid, side, seed);
    let runs = agree(&host, &sand(), shape, &start, 64);
    for run in &runs {
        if run.mode == Mode::Whole {
            continue;
        }
        assert!(
            run.last < run.highest,
            "{name} {:?}: no brick the sheets passed was freed",
            run.mode
        );
    }
}

#[test]
fn a_uniform_brick_that_changes_every_frame_is_held_until_it_is_quiet() {
    let name = "bricks ticking";
    let Some(host) = device(name) else {
        return;
    };
    let shape = world_for(host.width());
    let start = |grid: Grid3, side: [u32; 3], seed: u64| clocks(grid, side, seed);
    let runs = agree(&host, &ticking(), shape, &start, TICKS + 8);
    for run in &runs {
        assert_eq!(
            run.lowest, 0,
            "{name} {:?}: the clocks start uniform",
            run.mode
        );
        assert!(
            run.highest > 0,
            "{name} {:?}: no clock took a brick",
            run.mode
        );
        if run.mode != Mode::Whole {
            assert_eq!(
                run.last, 0,
                "{name} {:?}: the stopped clocks were not all freed",
                run.mode
            );
        }
    }
}

#[cfg(feature = "faults")]
#[test]
fn a_pool_split_over_windows_steps_to_the_dense_words() {
    let name = "bricks split pool";
    let Some(host) = device(name) else {
        return;
    };
    let shape = world_for(host.width());
    for more in [3, 4] {
        let pooled = |grid: Grid, plan: &Plan| {
            let probe = Bricks::new(grid, plan, plan.blocks).unwrap();
            let bits = probe.cell_bits() + more;
            let split = Bricks::split_at(grid, plan, plan.blocks, bits).unwrap();
            assert!(split.windows().is_split(), "{name}: {bits} bits");
            split
        };
        let start = |grid: Grid3, side: [u32; 3], seed: u64| scene(&natured(), grid, side, seed);
        agree_in(&host, &natured(), shape, &start, FRAMES, &pooled);
        let start = |grid: Grid3, side: [u32; 3], seed: u64| rain(grid, side, seed);
        agree_in(&host, &sand(), shape, &start, 64, &pooled);
    }
}

#[test]
fn a_full_pool_stops_the_world_at_the_frame_before_with_a_named_error() {
    let name = "bricks full pool";
    let Some(host) = device(name) else {
        return;
    };
    let width = host.width();
    let shape = world_for(width);
    let (grid, reference3) = grids(shape.0, shape.1, shape.2);
    let side = side_of(width, grid);
    let start = pile(reference3, side, 0);
    let plan = plan_of(width, grid);
    let probe = Bricks::new(grid, &plan, plan.blocks).unwrap();
    let held = probe.pack(&start).unwrap().held;
    for mode in MODES {
        let mut sparse = Sparse::new(&host, &Sand, grid, mode, Some(held));
        let opened = sparse.restart(&start, &[], 0);
        assert_eq!(
            opened.free, 0,
            "{name} {mode:?}: the pool should start full"
        );
        let mut want = start.clone();
        let mut stopped = None;
        for frame in 0..64 {
            sparse.step();
            match sparse.world() {
                Ok(world) => {
                    want = reference(&Sand, reference3, frame, &want, &[]);
                    if let Some(failed) = parted(name, reference3, frame, &world, &want) {
                        panic!("{failed}");
                    }
                }
                Err(Error::PoolFull { slots }) => {
                    assert_eq!(slots, held, "{name} {mode:?}");
                    stopped = Some(frame);
                    break;
                }
                Err(other) => panic!("{name} {mode:?}: {other}"),
            }
        }
        let Some(stopped) = stopped else {
            panic!("{name} {mode:?}: the pile fell 64 frames in a full pool");
        };
        for _ in 0..3 {
            sparse.step();
        }
        let table = sparse
            .held
            .read(sparse.bricks.table() as usize, sparse.bricks.table_words())
            .unwrap();
        let pool = live::read_state(
            &mut sparse.held,
            sparse.bricks.windows(),
            sparse.bricks.scratch(),
        )
        .unwrap();
        let world = sparse.bricks.unpack(&table, &pool);
        if let Some(failed) = parted(name, reference3, stopped, &world, &want) {
            panic!("{mode:?} after the pool filled: {failed}");
        }
        assert!(matches!(
            sparse.bricks.check(&mut sparse.held),
            Err(Error::PoolFull { .. })
        ));
        println!(
            "{name} {} width {width} {mode:?}: {held} slots, full at frame {stopped}, the world kept frame {}",
            host.name(),
            stopped.wrapping_sub(1)
        );
    }
}

#[test]
fn a_host_write_takes_a_brick_and_wakes_a_quiet_world() {
    let name = "bricks host write";
    let Some(host) = device(name) else {
        return;
    };
    let width = host.width();
    let shape = world_for(width);
    let (grid, reference3) = grids(shape.0, shape.1, shape.2);
    let side = side_of(width, grid);
    let start = pile(reference3, side, 0);
    for mode in [Mode::Fine, Mode::Coarse] {
        let mut sparse = Sparse::new(&host, &Sand, grid, mode, None);
        sparse.restart(&start, &[], 0);
        let mut want = start.clone();
        let mut frame = 0;
        let mut quiet = None;
        while frame < 2048 {
            sparse.step();
            want = reference(&Sand, reference3, frame, &want, &[]);
            frame += 1;
            let grand = sparse.held.read(grit::layout::GRAND as usize, 1).unwrap()[0];
            if grand == 0 {
                quiet = Some(frame);
                break;
            }
        }
        assert!(
            quiet.is_some(),
            "{name} {mode:?}: the pile never fell quiet"
        );
        let world = sparse.world().unwrap();
        assert_eq!(world, want, "{name} {mode:?}: the quiet world differs");
        let before = sparse.bricks.check(&mut sparse.held).unwrap();

        let x = grid.wide - side[0] / 2;
        let z = grid.tall - side[2] / 2;
        let cells: Vec<(u32, u32)> = (0..3)
            .map(|dy| {
                let at = reference3.index(x, dy, z);
                (u32::try_from(at).unwrap(), sand2::SAND)
            })
            .collect();
        for &(cell, word) in &cells {
            want[cell as usize] = word;
        }
        sparse
            .bricks
            .put(&mut sparse.held, &sparse.plan, &cells)
            .unwrap();
        let written = sparse.bricks.check(&mut sparse.held).unwrap();
        assert_eq!(
            written.held,
            before.held + 1,
            "{name} {mode:?}: the write into a uniform brick took no slot"
        );
        assert_eq!(sparse.world().unwrap(), want, "{name} {mode:?}: the write");
        let mut listed = 0;
        for _ in 0..FRAMES {
            sparse.step();
            want = reference(&Sand, reference3, frame, &want, &[]);
            frame += 1;
            listed += sparse.held.read(grit::layout::GRAND as usize, 1).unwrap()[0];
            let world = sparse.world().unwrap();
            if let Some(failed) = parted(name, reference3, frame - 1, &world, &want) {
                panic!("{mode:?} after the write: {failed}");
            }
        }
        assert!(listed > 0, "{name} {mode:?}: the write woke nothing");
        println!(
            "{name} {} width {width} {mode:?}: quiet at {}, {} held, {} after the write",
            host.name(),
            quiet.unwrap_or_default(),
            before.held,
            written.held
        );
    }
}

#[test]
fn octs_are_numbered_brick_by_brick_and_a_block_is_a_brick() {
    for width in [4, 8, 16, 32, 64] {
        let (wide, deep, tall) = world_for(width);
        let (grid, reference3) = grids(wide * 2, deep, tall);
        let plan = plan_of(width, grid);
        let bricks = Bricks::new(grid, &plan, 1).unwrap();
        let per_block = plan.per_block;
        let sides = bricks.sides();
        assert_eq!(
            sides[0] * sides[1] * sides[2],
            per_block * 8,
            "width {width}"
        );
        let octs = grid.blocks();
        let mut seen = vec![false; octs as usize];
        for oct in 0..octs {
            let point = bricks.oct_at(oct);
            assert_eq!(bricks.oct_of(point), oct, "width {width} oct {oct}");
            let row = point[0]
                + point[1] * (grid.wide / 2)
                + point[2] * (grid.wide / 2) * (grid.deep / 2);
            assert!(!seen[row as usize]);
            seen[row as usize] = true;
            for frame in [0, 1] {
                let placed = bricks.cells_of(oct, frame);
                let corners = reference3.corners(frame, row);
                for (cell, corner) in placed.cells.iter().zip(corners) {
                    let at = bricks.point_of(cell.brick, cell.offset);
                    assert_eq!(
                        Solid::cell(grid, at) as usize,
                        corner,
                        "width {width} oct {oct} frame {frame}"
                    );
                    if frame == 0 {
                        assert_eq!(cell.brick, oct / per_block, "width {width} oct {oct}");
                    }
                }
            }
            let woken = bricks.woken(oct);
            let rows = Solid::woken(grid, row);
            for (near, by_row) in woken.iter().zip(rows) {
                let back = bricks.oct_at(*near);
                let again = back[0]
                    + back[1] * (grid.wide / 2)
                    + back[2] * (grid.wide / 2) * (grid.deep / 2);
                assert_eq!(again, by_row, "width {width} oct {oct}");
            }
        }
        assert!(seen.iter().all(|&held| held));
    }
}

#[test]
fn packing_keeps_every_word_and_holds_only_mixed_bricks() {
    for width in [4, 8, 16, 32, 64] {
        let shape = world_for(width);
        let (grid, reference3) = grids(shape.0, shape.1, shape.2);
        let plan = plan_of(width, grid);
        let bricks = Bricks::new(grid, &plan, plan.blocks).unwrap();
        let side = bricks.sides();
        for subject_cells in [
            scene(&sand(), reference3, side, 7),
            pile(reference3, side, 1),
            vec![0xDEAD_BEEF; reference3.cells()],
        ] {
            let packed = bricks.pack(&subject_cells).unwrap();
            let mut mixed = 0;
            for brick in 0..bricks.count() {
                let mut words = Vec::new();
                for offset in 0..bricks.cells() {
                    let point = bricks.point_of(brick, u32::try_from(offset).unwrap());
                    words.push(subject_cells[Solid::cell(grid, point) as usize]);
                }
                let entry = packed.table[brick as usize * ENTRY];
                if words.iter().all(|&word| word == words[0]) {
                    assert_eq!(entry, UNIFORM, "width {width} brick {brick}");
                    assert_eq!(packed.table[brick as usize * ENTRY + 1], words[0]);
                } else {
                    assert_eq!(entry, mixed, "width {width} brick {brick}");
                    mixed += 1;
                }
            }
            assert_eq!(packed.held, mixed);
            assert_eq!(packed.slots[0], bricks.capacity() - mixed);
            assert_eq!(bricks.unpack(&packed.table, &packed.pool), subject_cells);
        }
        let full = Bricks::new(grid, &plan, 1).unwrap();
        let start = scene(&sand(), reference3, full.sides(), 7);
        assert!(
            matches!(full.pack(&start), Err(Error::PoolFull { slots: 1 })),
            "width {width}"
        );
    }
}

#[test]
fn a_pool_is_refused_by_name_for_a_flat_plan_a_small_grid_or_no_slot() {
    let named = |built: Result<Bricks, LaneError>, why: &str| match built {
        Err(LaneError::NoSuchForm { because, .. }) => assert_eq!(because, why),
        other => panic!("expected {why:?}, got {other:?}"),
    };
    let width = 32;
    let (grid, _) = grids(64, 64, 32);
    let plan = plan_of(width, grid);
    named(Bricks::new(grid, &plan, 0), bricks::NO_SLOTS);
    let flat = Plan::over(width, 4).unwrap();
    named(Bricks::new(grid, &flat, 4), bricks::FLAT);
    let (small, _) = grids(8, 64, 64);
    let small_plan = plan_of(width, small);
    named(Bricks::new(small, &small_plan, 4), bricks::SMALL);
    let other = Plan::over_in(width, plan.blocks * 2, Dimension::Three).unwrap();
    named(Bricks::new(grid, &other, 4), bricks::UNPLANNED);
    named(Bricks::new(grid, &plan, u32::MAX), bricks::TOO_MANY_SLOTS);
}
