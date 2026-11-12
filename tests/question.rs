#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;
use common::device;

use grit::bricks::Bricks;
use grit::compact::Plan;
use grit::layout::{Dimension, PARAMS};
use grit::ops::{Helpers, Ops};
use grit::question::cpu::{self, World as Cells};
use grit::question::{
    Answer, Asked, Layout, Map, Measure, Questions, Reduce, Refused, Region, Seen,
};
use grit::rule3::sand::Sand;
use grit::sand::Grid;
use grit::{Groups, Host, Resident, sweep};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Probe {
    Word,
    Kind,
    Sand,
    Placed,
    Spread,
}

impl Map for Probe {
    fn map<O: Ops>(&self, ops: &mut O, seen: Seen<'_, O::Word>) -> O::Word {
        let word = match seen.cell() {
            Some(word) => word,
            None => ops.constant(0),
        };
        let [x, y, z] = seen.at;
        match self {
            Self::Word => word,
            Self::Kind => ops.masked(word, 3),
            Self::Sand => {
                let kind = ops.masked(word, 3);
                ops.is(kind, 2).word()
            }
            Self::Placed => {
                let three = ops.constant(3);
                let lifted = ops.mul(y, three);
                let spot = ops.add(x, lifted);
                let spot = ops.xor(spot, z);
                let none = ops.constant(0);
                let set = ops.not_equal(word, none);
                ops.select(set, spot, none)
            }
            Self::Spread => {
                let stir = ops.constant(0x9E37_79B1);
                let one = ops.constant(1);
                let lifted = ops.add(word, one);
                let mixed = ops.mul(lifted, stir);
                let shifted = ops.shift_right(mixed, 7);
                ops.xor(mixed, shifted)
            }
        }
    }
}

fn shaped(size: [u32; 3]) -> Grid {
    Grid::solid(size[0], size[1], size[2]).unwrap()
}

struct World<'host> {
    live: Resident<'host>,
    stages: Vec<Vec<Groups>>,
    bindings: Vec<usize>,
    questions: Questions,
    asked: Vec<(Probe, Reduce, Asked)>,
    first_pass: usize,
    bricks: Option<Bricks>,
}

#[derive(Debug, Clone, Copy, Default)]
struct Made {
    sparse: bool,
    target: usize,
    counting: bool,
    split: Option<u32>,
}

impl Made {
    fn of(sparse: bool, target: usize) -> Self {
        Self {
            sparse,
            target,
            ..Self::default()
        }
    }
}

fn opened<'host>(
    host: &'host Host,
    size: [u32; 3],
    cells: &[u32],
    asks: &[(Probe, Reduce)],
    made: Made,
) -> World<'host> {
    let Made {
        sparse,
        target,
        counting,
        split,
    } = made;
    let width = host.width();
    let grid = shaped(size);
    let blocks = grid.blocks().div_ceil(sweep::cells(width));
    let (plan, bricks) = if sparse {
        let plan = Plan::bricked(width, blocks).unwrap();
        let bricks = match split {
            Some(bits) => split_pool(grid, &plan, bits),
            None => Bricks::new(grid, &plan, plan.blocks).unwrap(),
        };
        (plan, Some(bricks))
    } else {
        let plan = Plan::over_in(width, blocks, Dimension::Three).unwrap();
        let plan = match split {
            Some(bits) => split_plan(plan, bits),
            None => plan,
        };
        (plan, None)
    };
    let ((passes, stages), bindings) = match bricks.as_ref() {
        Some(bricks) => (
            grit::bricks::arranged(&Sand, bricks, &plan, true).unwrap(),
            bricks.sizes(&Sand, &plan),
        ),
        None => (
            grit::live::wired3_by(&Sand, width, grid, &plan).unwrap(),
            plan.windows().laid(&grit::live::sizes3(&Sand, &plan)),
        ),
    };
    let layout = match bricks {
        Some(bricks) => Layout::Sparse(bricks),
        None => Layout::Dense {
            grid,
            windows: plan.windows(),
        },
    };
    let questions = Questions::new(layout, width, bindings.len(), target).unwrap();
    let mut questions = if counting {
        counted(questions)
    } else {
        questions
    };
    let mut asked = Vec::new();
    for &(probe, reduce) in asks {
        asked.push((probe, reduce, questions.add(&probe, reduce).unwrap()));
    }
    let mut sizes = bindings.clone();
    sizes.extend(questions.sizes());
    let mut modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let first_pass = modules.len();
    modules.extend(questions.modules());
    let mut live = host.resident(&sizes, &modules).unwrap();
    for (binding, &room) in sizes.iter().enumerate() {
        live.write(binding, &vec![0; room]).unwrap();
    }
    match bricks.as_ref() {
        Some(bricks) => {
            live.write(PARAMS as usize, &[u32::MAX]).unwrap();
            let packed = bricks.pack(cells).unwrap();
            bricks.write_packed(&mut live, &plan, &packed).unwrap();
            bricks.seed(&mut live, &plan).unwrap();
        }
        None => {
            live.write(PARAMS as usize, &[u32::MAX, plan.dump()])
                .unwrap();
            grit::live::write_state(&mut live, plan.windows(), 0, cells).unwrap();
            grit::live::seed(&mut live, grid, &plan).unwrap();
        }
    }
    let mut stages = stages;
    stages.push(questions.resting());
    World {
        live,
        stages,
        bindings,
        questions,
        asked,
        first_pass,
        bricks,
    }
}

#[cfg(feature = "faults")]
fn counted(questions: Questions) -> Questions {
    questions.counting()
}

#[cfg(not(feature = "faults"))]
fn counted(questions: Questions) -> Questions {
    questions
}

#[cfg(feature = "faults")]
fn split_pool(grid: Grid, plan: &Plan, bits: u32) -> Bricks {
    Bricks::split_at(grid, plan, plan.blocks, bits).unwrap()
}

#[cfg(not(feature = "faults"))]
fn split_pool(grid: Grid, plan: &Plan, _: u32) -> Bricks {
    Bricks::new(grid, plan, plan.blocks).unwrap()
}

#[cfg(feature = "faults")]
fn split_plan(plan: Plan, bits: u32) -> Plan {
    plan.split_at(bits)
}

#[cfg(not(feature = "faults"))]
fn split_plan(plan: Plan, _: u32) -> Plan {
    plan
}

impl World<'_> {
    fn step(&mut self, frames: u32) {
        let groups: Vec<&[Groups]> = self.stages.iter().map(Vec::as_slice).collect();
        for _ in 0..frames {
            self.live.staged(&groups).unwrap();
        }
    }

    fn hash(&mut self) -> u64 {
        let mut hash = 0xCBF2_9CE4_8422_2325_u64;
        for (binding, &words) in self.bindings.clone().iter().enumerate() {
            let read = self.live.read(binding, words).unwrap();
            for word in std::iter::once(words as u32).chain(read) {
                for byte in word.to_le_bytes() {
                    hash ^= u64::from(byte);
                    hash = hash.wrapping_mul(0x0100_0000_01B3);
                }
            }
        }
        hash
    }

    fn cells(&mut self) -> Vec<u32> {
        match self.bricks {
            Some(bricks) => bricks.read(&mut self.live).unwrap(),
            None => {
                let layout = self.questions.layout();
                grit::live::read_state(&mut self.live, layout.windows(), layout.grid().cells())
                    .unwrap()
            }
        }
    }

    fn ask(&mut self, which: usize, region: Region) -> Result<Answer, grit::question::Failure> {
        let before = self.hash();
        let (_, _, asked) = self.asked[which];
        let answer = self
            .questions
            .ask(&mut self.live, self.first_pass, asked, region);
        assert_eq!(before, self.hash(), "question {which} changed the world");
        answer
    }

    #[cfg(feature = "faults")]
    fn spread(
        &mut self,
        which: usize,
        region: Region,
        most: u32,
    ) -> (Answer, Option<grit::question::reduce::Reads>) {
        let before = self.hash();
        let (_, _, asked) = self.asked[which];
        let answer = self
            .questions
            .ask_spread(&mut self.live, self.first_pass, asked, region, most)
            .unwrap();
        assert_eq!(before, self.hash(), "question {which} changed the world");
        answer
    }

    fn aim(&mut self, words: &[u32]) {
        self.questions.aim(&mut self.live, words).unwrap();
    }
}

struct Stir(u64);

impl Stir {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 16) as u32
    }
}

fn index(size: [u32; 3], [x, y, z]: [u32; 3]) -> usize {
    ((z as usize * size[1] as usize) + y as usize) * size[0] as usize + x as usize
}

fn sandy(size: [u32; 3], seed: u64) -> Vec<u32> {
    let mut stir = Stir(seed);
    let mut cells = vec![0; size.iter().map(|&side| side as usize).product()];
    for z in 0..size[2] {
        for y in 0..size[1] {
            for x in 0..size[0] {
                let roll = stir.next();
                let word = if y + 4 >= size[1] {
                    3
                } else if (x / 8 + z / 8) % 3 == 0 && roll % 5 < 2 {
                    2
                } else if y < size[1] / 3 && roll.is_multiple_of(7) {
                    1 | (roll & 0xFFFF_FFFC)
                } else {
                    0
                };
                cells[index(size, [x, y, z])] = word;
            }
        }
    }
    cells
}

fn sparse_size(host: &Host) -> [u32; 3] {
    sides_of(host.width()).map(|side| (side * 4).clamp(16, 64))
}

fn sides_of(width: u32) -> [u32; 3] {
    let k = (width * 8).trailing_zeros();
    [2 << k.div_ceil(3), 2 << ((k + 1) / 3), 2 << (k / 3)]
}

fn regions(size: [u32; 3]) -> Vec<Region> {
    let [wide, deep, tall] = size;
    vec![
        Region::whole(size),
        Region::new([3, 5, 7], [wide - 3, deep - 2, tall - 5]),
        Region::new([wide / 2, 0, tall / 4], [wide / 2 + 1, deep, tall / 4 + 1]),
        Region::new([1, 2, 3], [2, 3, 4]),
        Region::new([0, deep / 2, 0], [wide, deep / 2 + 3, tall]),
    ]
}

fn every_question() -> Vec<(Probe, Reduce)> {
    let mut asks = Vec::new();
    for reduce in [
        Reduce::Count,
        Reduce::Sum,
        Reduce::Min,
        Reduce::Max,
        Reduce::Histogram { bins: 4 },
        Reduce::Moments,
        Reduce::Differ(Measure::Count),
        Reduce::Differ(Measure::Sum),
    ] {
        asks.push((Probe::Word, reduce));
    }
    asks.push((Probe::Kind, Reduce::Histogram { bins: 3 }));
    asks.push((Probe::Kind, Reduce::Count));
    asks.push((Probe::Sand, Reduce::Moments));
    asks.push((Probe::Sand, Reduce::Count));
    for reduce in [
        Reduce::Count,
        Reduce::Sum,
        Reduce::Min,
        Reduce::Max,
        Reduce::Histogram { bins: 8 },
        Reduce::Moments,
    ] {
        asks.push((Probe::Placed, reduce));
    }
    for reduce in [Reduce::Sum, Reduce::Min, Reduce::Max] {
        asks.push((Probe::Spread, reduce));
    }
    asks
}

fn aimed(size: [u32; 3], region: Region, seed: u64) -> Vec<u32> {
    let mut stir = Stir(seed);
    let cells = sandy(size, seed ^ 0x55);
    let [wide, deep, tall] = region.sides();
    let mut target = Vec::new();
    for z in 0..tall {
        for y in 0..deep {
            for x in 0..wide {
                let at = [region.from[0] + x, region.from[1] + y, region.from[2] + z];
                let word = cells[index(size, at)];
                target.push(if stir.next().is_multiple_of(3) {
                    word
                } else {
                    stir.next()
                });
            }
        }
    }
    target
}

#[test]
fn every_primitive_answers_as_the_cpu_on_dense_and_sparse_worlds() {
    let Some(host) = device("question") else {
        return;
    };
    let size = sparse_size(&host);
    let sides = sides_of(host.width());
    for cells in [sandy(size, 0x1234_5678), mostly_uniform(size, sides)] {
        answered_alike(&host, size, &cells);
    }
}

fn answered_alike(host: &Host, size: [u32; 3], cells: &[u32]) {
    let asks = every_question();
    let whole = size.iter().map(|&side| side as usize).product();
    let mut dense = opened(host, size, cells, &asks, Made::of(false, whole));
    let mut sparse = opened(host, size, cells, &asks, Made::of(true, whole));
    for frames in [0, 3, 8] {
        dense.step(frames);
        sparse.step(frames);
        let now = dense.cells();
        assert_eq!(now, sparse.cells(), "the two tiers stepped apart");
        for (number, region) in regions(size).into_iter().enumerate() {
            let target = aimed(size, region, number as u64 + u64::from(frames));
            dense.aim(&target);
            sparse.aim(&target);
            for (which, &(probe, reduce)) in asks.iter().enumerate() {
                let world = Cells { size, cells: &now };
                let wanted = cpu::answer(&probe, reduce, world, region, &target);
                let from_dense = dense.ask(which, region).unwrap();
                let from_sparse = sparse.ask(which, region).unwrap();
                assert_eq!(
                    from_dense, wanted,
                    "dense {probe:?} {reduce:?} {region:?} after {frames}"
                );
                assert_eq!(
                    from_sparse, wanted,
                    "sparse {probe:?} {reduce:?} {region:?} after {frames}"
                );
            }
        }
    }
}

fn mostly_uniform(size: [u32; 3], sides: [u32; 3]) -> Vec<u32> {
    let mut cells = vec![0; size.iter().map(|&side| side as usize).product()];
    for z in 0..size[2] {
        for y in 0..size[1] {
            for x in 0..size[0] {
                let word = if y >= size[1] / 2 { 3 } else { 0 };
                cells[index(size, [x, y, z])] = word;
            }
        }
    }
    let spots = [
        [sides[0] + 1, 2, sides[2] + 3],
        [2 * sides[0] + 5, sides[1] / 2, 1],
    ];
    for spot in spots {
        cells[index(size, spot)] = 2;
    }
    cells
}

#[test]
fn a_uniform_brick_counts_every_cell() {
    let Some(host) = device("question") else {
        return;
    };
    let size = sparse_size(&host);
    let sides = sides_of(host.width());
    let cells = mostly_uniform(size, sides);
    let asks = vec![
        (Probe::Word, Reduce::Count),
        (Probe::Kind, Reduce::Sum),
        (Probe::Word, Reduce::Histogram { bins: 4 }),
        (Probe::Word, Reduce::Moments),
        (Probe::Word, Reduce::Min),
        (Probe::Word, Reduce::Max),
    ];
    let mut sparse = opened(&host, size, &cells, &asks, Made::of(true, 0));
    let world = Cells {
        size,
        cells: &cells,
    };
    let mut boxes = regions(size);
    boxes.push(Region::new(
        [0, size[1] / 2, 0],
        [sides[0], size[1], sides[2]],
    ));
    boxes.push(Region::new(
        [1, size[1] - 3, 2],
        [sides[0] + 3, size[1], 2 * sides[2] + 1],
    ));
    for region in boxes {
        for (which, &(probe, reduce)) in asks.iter().enumerate() {
            let wanted = cpu::answer(&probe, reduce, world, region, &[]);
            let found = sparse.ask(which, region).unwrap();
            assert_eq!(found, wanted, "{probe:?} {reduce:?} over {region:?}");
        }
    }
    let rock = Region::new([0, size[1] / 2, 0], size);
    let cells_in = rock.cells() as u64;
    assert_eq!(sparse.ask(0, rock).unwrap(), Answer::Count(cells_in));
    assert_eq!(sparse.ask(1, rock).unwrap(), Answer::Sum(3 * cells_in));
}

#[test]
fn a_sum_past_four_billion_is_exact() {
    let Some(host) = device("question") else {
        return;
    };
    let size = sparse_size(&host);
    let mut cells = vec![u32::MAX; size.iter().map(|&side| side as usize).product()];
    cells[index(size, [3, 4, 5])] = 7;
    let asks = vec![
        (Probe::Word, Reduce::Sum),
        (Probe::Word, Reduce::Count),
        (Probe::Word, Reduce::Moments),
        (Probe::Word, Reduce::Differ(Measure::Sum)),
    ];
    let whole = cells.len();
    for sparse in [false, true] {
        let mut world = opened(&host, size, &cells, &asks, Made::of(sparse, whole));
        world.aim(&vec![0; whole]);
        let region = Region::whole(size);
        let many = whole as u64;
        let wanted = (many - 1) * u64::from(u32::MAX) + 7;
        assert!(wanted > 1 << 32);
        assert_eq!(
            world.ask(0, region).unwrap(),
            Answer::Sum(wanted),
            "sparse {sparse}"
        );
        assert_eq!(world.ask(1, region).unwrap(), Answer::Count(many));
        assert_eq!(world.ask(3, region).unwrap(), Answer::Differ(wanted));
        let reference = cpu::answer(
            &Probe::Word,
            Reduce::Moments,
            Cells {
                size,
                cells: &cells,
            },
            region,
            &[],
        );
        assert_eq!(world.ask(2, region).unwrap(), reference);
    }
}

#[cfg(feature = "faults")]
#[test]
fn a_uniform_brick_is_answered_without_reading_the_pool() {
    let Some(host) = device("question") else {
        return;
    };
    let size = sparse_size(&host);
    let sides = sides_of(host.width());
    let cells = mostly_uniform(size, sides);
    let asks = vec![
        (Probe::Word, Reduce::Count),
        (Probe::Placed, Reduce::Sum),
        (Probe::Word, Reduce::Histogram { bins: 4 }),
    ];
    let mut sparse = opened(
        &host,
        size,
        &cells,
        &asks,
        Made {
            counting: true,
            ..Made::of(true, 0)
        },
    );
    let bricks = sparse.bricks.unwrap();
    let (table, _) = bricks.raw(&mut sparse.live).unwrap();
    let per = bricks.cells() as u64;
    for region in regions(size) {
        let mut held = 0_u64;
        let mut touched = 0_u64;
        for brick in 0..bricks.count() {
            let origin = bricks.point_of(brick, 0);
            let meets = (0..3).all(|axis| {
                origin[axis] < region.to[axis]
                    && origin[axis] + bricks.sides()[axis] > region.from[axis]
            });
            if meets {
                touched += 1;
                if table[brick as usize * grit::bricks::ENTRY] != grit::bricks::UNIFORM {
                    held += 1;
                }
            }
        }
        for (which, &(probe, reduce)) in asks.iter().enumerate() {
            let (answer, reads) = sparse.spread(which, region, 65535);
            let reads = reads.unwrap();
            let wanted = cpu::answer(
                &probe,
                reduce,
                Cells {
                    size,
                    cells: &cells,
                },
                region,
                &[],
            );
            assert_eq!(answer, wanted);
            eprintln!(
                "{probe:?} {reduce:?} over {region:?}: {touched} bricks met, {held} held, {} table entries read, {} pool words read ({} per held brick)",
                reads.table, reads.pool, per
            );
            assert_eq!(reads.table, touched);
            assert_eq!(reads.pool, held * per);
        }
    }
}

#[cfg(feature = "faults")]
#[test]
fn the_answer_does_not_depend_on_how_the_work_is_spread() {
    let Some(host) = device("question") else {
        return;
    };
    let size = sparse_size(&host);
    let cells = sandy(size, 0xFEED);
    let asks = every_question();
    let whole = cells.len();
    for sparse in [false, true] {
        let mut world = opened(&host, size, &cells, &asks, Made::of(sparse, whole));
        world.step(5);
        let region = Region::new([1, 1, 1], [size[0] - 1, size[1], size[2] - 2]);
        world.aim(&aimed(size, region, 9));
        for (which, ask) in asks.iter().enumerate() {
            let mut seen = Vec::new();
            for most in [1, 2, 7, 64, 65535] {
                let (answer, _) = world.spread(which, region, most);
                seen.push(answer);
            }
            assert!(
                seen.windows(2).all(|two| two[0] == two[1]),
                "{ask:?} on sparse {sparse}: {seen:?}"
            );
        }
    }
}

#[test]
fn a_question_leaves_the_world_stepping_as_before() {
    let Some(host) = device("question") else {
        return;
    };
    let size = sparse_size(&host);
    let cells = sandy(size, 0xABCD);
    let asks = every_question();
    let whole = cells.len();
    for sparse in [false, true] {
        let mut asking = opened(&host, size, &cells, &asks, Made::of(sparse, whole));
        let mut quiet = opened(&host, size, &cells, &asks, Made::of(sparse, whole));
        asking.aim(&vec![1; whole]);
        for round in 0..4 {
            for which in 0..asks.len() {
                asking.ask(which, Region::whole(size)).unwrap();
            }
            asking.step(2 + round);
            quiet.step(2 + round);
            assert_eq!(
                asking.hash(),
                quiet.hash(),
                "round {round}, sparse {sparse}"
            );
        }
    }
}

fn layout_of(size: [u32; 3], width: u32, sparse: bool) -> Layout {
    let grid = shaped(size);
    let blocks = grid.blocks().div_ceil(sweep::cells(width));
    if sparse {
        let plan = Plan::bricked(width, blocks).unwrap();
        Layout::Sparse(Bricks::new(grid, &plan, 1).unwrap())
    } else {
        let plan = Plan::over_in(width, blocks, Dimension::Three).unwrap();
        Layout::Dense {
            grid,
            windows: plan.windows(),
        }
    }
}

#[test]
fn every_question_kernel_builds_at_every_width() {
    for width in [4, 8, 16, 32, 64] {
        for sparse in [false, true] {
            let layout = layout_of([64, 64, 64], width, sparse);
            let mut questions = Questions::new(layout, width, 40, 64).unwrap();
            for (probe, reduce) in every_question() {
                questions.add(&probe, reduce).unwrap();
            }
            assert_eq!(questions.modules().len(), every_question().len());
            assert!(
                questions
                    .modules()
                    .iter()
                    .all(|words| words.first() == Some(&0x0723_0203))
            );
        }
    }
}

fn refusal(questions: &Questions, asked: Asked, region: Region) -> &'static str {
    questions.check(asked, region).unwrap_err().name()
}

#[test]
fn a_box_that_leaves_the_world_is_empty_or_too_large_is_refused_by_name() {
    let size = [64, 64, 64];
    let mut questions = Questions::new(layout_of(size, 32, true), 32, 40, 100).unwrap();
    let count = questions.add(&Probe::Word, Reduce::Count).unwrap();
    let differ = questions
        .add(&Probe::Word, Reduce::Differ(Measure::Count))
        .unwrap();
    assert!(questions.check(count, Region::whole(size)).is_ok());
    for (from, to) in [
        ([0, 0, 0], [65, 64, 64]),
        ([0, 0, 0], [64, 64, 65]),
        ([63, 0, 0], [70, 1, 1]),
        ([0, 0, 0], [u32::MAX, 1, 1]),
    ] {
        assert_eq!(refusal(&questions, count, Region::new(from, to)), "outside");
    }
    for (from, to) in [
        ([5, 5, 5], [5, 6, 6]),
        ([5, 5, 5], [6, 5, 6]),
        ([5, 5, 5], [6, 6, 4]),
        ([64, 0, 0], [64, 64, 64]),
    ] {
        assert_eq!(refusal(&questions, count, Region::new(from, to)), "empty");
    }
    assert_eq!(
        refusal(&questions, differ, Region::new([0, 0, 0], [11, 10, 1])),
        "target_short"
    );
    assert!(
        questions
            .check(differ, Region::new([0, 0, 0], [10, 10, 1]))
            .is_ok()
    );
    let mut more = questions.clone();
    more.add(&Probe::Word, Reduce::Max).unwrap();
    let beyond = more.add(&Probe::Word, Reduce::Min).unwrap();
    assert_eq!(refusal(&questions, beyond, Region::whole(size)), "unasked");

    let vast = [8192, 256, 4096];
    let mut questions = Questions::new(layout_of(vast, 32, true), 32, 40, 0).unwrap();
    let sum = questions.add(&Probe::Word, Reduce::Sum).unwrap();
    let count = questions.add(&Probe::Word, Reduce::Count).unwrap();
    let moments = questions.add(&Probe::Word, Reduce::Moments).unwrap();
    let least = questions.add(&Probe::Word, Reduce::Min).unwrap();
    let whole = Region::whole(vast);
    assert_eq!(refusal(&questions, sum, whole), "too_large");
    match questions.check(sum, whole).unwrap_err() {
        Refused::TooLarge { cells, most, .. } => {
            assert_eq!(cells, 1 << 33);
            assert_eq!(most, (1 << 32) + 1);
        }
        other => panic!("{other:?}"),
    }
    let fits = Region::new([0, 0, 0], [8192, 256, 2048]);
    assert!(questions.check(sum, fits).is_ok());
    let past = Region::new([0, 0, 0], [8192, 256, 2049]);
    assert_eq!(refusal(&questions, sum, past), "too_large");
    assert!(questions.check(count, whole).is_ok());
    assert!(questions.check(least, whole).is_ok());
    assert!(questions.check(moments, whole).is_ok());
}

#[test]
fn a_question_that_cannot_be_built_is_refused_by_name() {
    let layout = layout_of([64, 64, 64], 32, false);
    let mut questions = Questions::new(layout, 32, 40, 0).unwrap();
    for bins in [0, 257, u32::MAX] {
        let refused = questions
            .add(&Probe::Word, Reduce::Histogram { bins })
            .unwrap_err();
        assert_eq!(refused.refused().map(Refused::name), Some("bins"));
    }
    assert!(
        questions
            .add(&Probe::Word, Reduce::Histogram { bins: 256 })
            .is_ok()
    );
    assert!(
        questions
            .add(&Probe::Word, Reduce::Histogram { bins: 1 })
            .is_ok()
    );
    let refused = questions
        .add(&Probe::Word, Reduce::Differ(Measure::Sum))
        .unwrap_err();
    assert_eq!(refused.refused().map(Refused::name), Some("no_target"));
    assert_eq!(
        Questions::new(layout, 12, 40, 0).unwrap_err().name(),
        "width"
    );
    let flat = Layout::Dense {
        grid: Grid::solid(64, 64, 1).unwrap(),
        windows: grit::layout::Windows::of(4096),
    };
    assert_eq!(Questions::new(flat, 32, 40, 0).unwrap_err().name(), "flat");
}

#[test]
fn a_map_reads_the_point_only_when_it_does() {
    assert!(!grit::question::reads_point(&Probe::Word));
    assert!(!grit::question::reads_point(&Probe::Kind));
    assert!(!grit::question::reads_point(&Probe::Sand));
    assert!(!grit::question::reads_point(&Probe::Spread));
    assert!(grit::question::reads_point(&Probe::Placed));
}

#[cfg(feature = "faults")]
#[test]
fn a_world_split_over_windows_answers_as_one_binding_does() {
    let Some(host) = device("question") else {
        return;
    };
    let size = sparse_size(&host);
    let cells = sandy(size, 0x5917);
    let asks = every_question();
    let whole = cells.len();
    for sparse in [false, true] {
        let mut plain = opened(&host, size, &cells, &asks, Made::of(sparse, whole));
        let bits = match plain.bricks {
            Some(bricks) => bricks.cell_bits() + 2,
            None => whole.trailing_zeros() - 2,
        };
        let made = Made {
            split: Some(bits),
            ..Made::of(sparse, whole)
        };
        let mut split = opened(&host, size, &cells, &asks, made);
        let windows = split.questions.layout().windows();
        assert!(windows.is_split(), "sparse {sparse}: {bits} bits");
        plain.step(3);
        split.step(3);
        let now = plain.cells();
        assert_eq!(
            now,
            split.cells(),
            "sparse {sparse}: the split world stepped apart"
        );
        for (number, region) in regions(size).into_iter().enumerate() {
            let target = aimed(size, region, number as u64);
            plain.aim(&target);
            split.aim(&target);
            for (which, &(probe, reduce)) in asks.iter().enumerate() {
                let world = Cells { size, cells: &now };
                let wanted = cpu::answer(&probe, reduce, world, region, &target);
                assert_eq!(plain.ask(which, region).unwrap(), wanted);
                assert_eq!(
                    split.ask(which, region).unwrap(),
                    wanted,
                    "{} windows, sparse {sparse}: {probe:?} {reduce:?} {region:?}",
                    windows.count()
                );
            }
        }
    }
}
