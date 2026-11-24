#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::device;

use grit::branch::{Made, Stepping, World};
use grit::bricks::{
    self, Bricks, ENTRY, HEAD, LOST, OVERFLOW, PoolUse, TILE_WORDS, TOP, TOUCHED, UNIFORM, VALUE,
    WANTED,
};
use grit::compact::Plan;
use grit::law::Law;
use grit::layout::{
    AWAKE, DENSE, GRAND, HOT, HOT_DENSE, HOT_GRAND, HOT_INDIRECT, INDIRECT, PARAMS, PIXELS,
};
use grit::live;
use grit::ops::{Helpers, Ops};
use grit::rule3::sand::Sand;
use grit::rule3::{Declared, Oct, OneStep, Rule, Ruled, Tally};
use grit::sand::{self as sand2, Grid};
use grit::sand3::{Grid3, reference};
use grit::serve3::Server;
use grit::wire::{Json, written};
use grit::{Groups, Host, Resident, sweep};

const LONG: u64 = 600_000;

const BAYS: Table = Table {
    birth: 1 << 6,
    survive: (1 << 5) | (1 << 6) | (1 << 7),
};

#[cfg(feature = "faults")]
const LIFE: Table = Table {
    birth: 1 << 3,
    survive: (1 << 2) | (1 << 3),
};

const BLUR: &str = include_str!("halo3/blur2.law.json");

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Table {
    birth: u32,
    survive: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(not(feature = "faults"), allow(dead_code))]
enum Shape {
    Square,
    Cube,
}

impl Shape {
    const fn name(self) -> &'static str {
        match self {
            Self::Square => "3x3",
            Self::Cube => "3x3x3",
        }
    }

    const fn around(self) -> u32 {
        match self {
            Self::Square => 8,
            Self::Cube => 26,
        }
    }

    const fn reach(self) -> i64 {
        match self {
            Self::Square => 0,
            Self::Cube => 1,
        }
    }
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, most: u64) -> u64 {
        self.next() % most
    }

    fn word(&mut self) -> u32 {
        u32::try_from(self.next() >> 32).unwrap()
    }
}

fn index(size: [u32; 3], at: [u32; 3]) -> usize {
    (at[2] as usize * size[1] as usize + at[1] as usize) * size[0] as usize + at[0] as usize
}

fn cells(size: [u32; 3]) -> usize {
    size.iter().map(|side| *side as usize).product()
}

fn point(size: [u32; 3], at: usize) -> [u32; 3] {
    let x = at % size[0] as usize;
    let y = at / size[0] as usize % size[1] as usize;
    let z = at / (size[0] as usize * size[1] as usize);
    [x, y, z].map(|one| u32::try_from(one).unwrap())
}

fn wrapped(size: [u32; 3], at: [i64; 3]) -> [u32; 3] {
    [0, 1, 2].map(|axis| u32::try_from(at[axis].rem_euclid(i64::from(size[axis]))).unwrap())
}

fn generation(shape: Shape, table: Table, size: [u32; 3], words: &[u32]) -> Vec<u32> {
    let reach = shape.reach();
    let mut out = vec![0_u32; words.len()];
    for z in 0..size[2] {
        for y in 0..size[1] {
            for x in 0..size[0] {
                let mut count = 0;
                for dz in -reach..=reach {
                    for dy in -1_i64..=1 {
                        for dx in -1_i64..=1 {
                            if dx == 0 && dy == 0 && dz == 0 {
                                continue;
                            }
                            let at = [i64::from(x) + dx, i64::from(y) + dy, i64::from(z) + dz];
                            count += words[index(size, wrapped(size, at))] & 1;
                        }
                    }
                }
                let here = index(size, [x, y, z]);
                let row = if words[here] & 1 == 1 {
                    table.survive
                } else {
                    table.birth
                };
                out[here] = (row >> count) & 1;
            }
        }
    }
    out
}

fn line(name: &str, arguments: &str) -> String {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{{\"name\":\"{name}\",\"arguments\":{arguments}}}}}"
    )
}

fn result(got: &Json, error: bool) -> Json {
    let held = got
        .get("result")
        .unwrap_or_else(|| panic!("{}", written(got)));
    let shown = written(got);
    assert_eq!(
        held.get("isError"),
        Some(&Json::Bool(error)),
        "{}",
        &shown[..shown.len().min(4000)]
    );
    held.get("structuredContent").cloned().unwrap()
}

fn ok(server: &mut Server<'_>, name: &str, arguments: &str) -> Json {
    result(&server.heard(&line(name, arguments)).unwrap(), false)
}

#[cfg(feature = "faults")]
fn int_at(held: &Json, key: &str) -> i64 {
    held.get(key)
        .and_then(Json::int)
        .unwrap_or_else(|| panic!("no {key} in {}", written(held)))
}

fn text_at(held: &Json, key: &str) -> String {
    held.get(key)
        .and_then(Json::text)
        .unwrap_or_else(|| panic!("no {key} in {}", written(held)))
        .to_owned()
}

fn shown_list(held: &[u32]) -> String {
    let shown: Vec<String> = held.iter().map(u32::to_string).collect();
    format!("[{}]", shown.join(","))
}

#[cfg(feature = "faults")]
fn quoted(text: &str) -> String {
    written(&Json::Text(text.to_owned()))
}

fn soup(rng: &mut Rng, size: [u32; 3], permille: u64) -> Vec<u32> {
    (0..cells(size))
        .map(|_| u32::from(rng.below(1000) < permille))
        .collect()
}

fn table_of(shape: Shape, table: Table) -> String {
    let row = |bits: u32| {
        let shown: Vec<String> = (0..=shape.around())
            .map(|count| ((bits >> count) & 1).to_string())
            .collect();
        format!("[{}]", shown.join(","))
    };
    format!("[[{},{}]]", row(table.birth), row(table.survive))
}

fn learned_on(server: &mut Server<'_>, shape: Shape, table: Table) -> String {
    let (size, densities): ([u32; 3], &[u64]) = match shape {
        Shape::Square => ([32, 32, 1], &[250, 300, 350, 400, 450, 500, 600, 700, 930]),
        Shape::Cube => ([16, 16, 16], &[120, 180, 220, 260, 320, 400, 930]),
    };
    let mut rng = Rng(u64::from(table.birth ^ table.survive.rotate_left(7)));
    let examples: Vec<String> = densities
        .iter()
        .map(|&permille| {
            let input = soup(&mut rng, size, permille);
            let output = generation(shape, table, size, &input);
            format!(
                "{{\"size\":[{},{},{}],\"input\":[{}],\"output\":[{}]}}",
                size[0],
                size[1],
                size[2],
                shown_list(&input),
                shown_list(&output)
            )
        })
        .collect();
    let got = ok(
        server,
        "grit_law_learn",
        &format!(
            "{{\"stencil\":\"{}\",\"reads\":[{{\"plane\":0,\"bit\":0}}],\"writes\":[{{\"plane\":1,\"bit\":0}}],\"border\":\"wrapped\",\"rule\":\"outer\",\"examples\":[{}],\"budget_ms\":{LONG}}}",
            shape.name(),
            examples.join(",")
        ),
    );
    assert_eq!(
        written(got.get("tables").unwrap()),
        table_of(shape, table),
        "{shape:?} {table:?}"
    );
    text_at(&got, "law")
}

fn iterating(text: &str) -> Law {
    let json = grit::law::parse::read(text).unwrap();
    let Json::Map(mut fields) = json else {
        panic!("a law is an object");
    };
    fields.push((
        "alternate".to_owned(),
        Json::List(vec![Json::Int(0), Json::Int(1)]),
    ));
    Law::from_json(&Json::Map(fields)).unwrap()
}

fn learned_law(host: &Host, shape: Shape, table: Table) -> Law {
    let mut server = Server::new(Some(host));
    iterating(&learned_on(&mut server, shape, table))
}

fn glider_cells(shape: Shape, at: [u32; 3], axes: [usize; 3], flip: [bool; 2]) -> Vec<[u32; 3]> {
    let flat = [[1_u32, 0], [2, 1], [0, 2], [1, 2], [2, 2]];
    let layers: &[u32] = match shape {
        Shape::Square => &[0],
        Shape::Cube => &[0, 1],
    };
    let mut out = Vec::new();
    for [a, b] in flat {
        let a = if flip[0] { 2 - a } else { a };
        let b = if flip[1] { 2 - b } else { b };
        for &c in layers {
            let mut cell = at;
            cell[axes[0]] += a;
            cell[axes[1]] += b;
            cell[axes[2]] += c;
            out.push(cell);
        }
    }
    out
}

type Planted = (u32, Vec<([u32; 3], u32)>);

struct Turn;

impl Declared for Turn {
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

impl OneStep for Turn {
    fn one_step<O: Ops>(&self, ops: &mut O, oct: Oct<'_, O::Word>) -> Ruled<O::Word> {
        let none = ops.constant(0);
        let words = oct.words;
        let out = std::array::from_fn(|corner| words[corner ^ 7]);
        let lit: Vec<_> = words
            .iter()
            .map(|&word| ops.not_equal(word, none))
            .collect();
        Ruled {
            out,
            counts: vec![ops.count(lit)],
        }
    }
}

struct Engine<'host> {
    held: Resident<'host>,
    stages: Vec<Vec<Groups>>,
    bricks: Bricks,
    plan: Plan,
    sizes: Vec<usize>,
}

#[derive(Debug, Clone, Default)]
struct Device {
    cells: Vec<u32>,
    table: Vec<u32>,
    hot: Vec<u32>,
    awake: Vec<u32>,
    header: Vec<u32>,
    fine: Vec<u32>,
    whole: Vec<u32>,
    hot_list: Vec<u32>,
    tiles: Vec<u32>,
    args: Vec<u32>,
    indirect: Vec<u32>,
    hot_indirect: Vec<u32>,
}

impl<'host> Engine<'host> {
    fn new<R: Rule>(host: &'host Host, rule: &R, size: [u32; 3], capacity: Option<u32>) -> Self {
        let width = host.width();
        let grid = Grid::solid(size[0], size[1], size[2]).unwrap();
        let plan = Plan::bricked(width, grid.blocks().div_ceil(sweep::cells(width))).unwrap();
        let bricks = Bricks::new(grid, &plan, capacity.unwrap_or(plan.blocks))
            .unwrap()
            .with_planes(u32::try_from(rule.planes()).unwrap())
            .unwrap();
        let (passes, stages) = bricks::arranged(rule, &bricks, &plan, true).unwrap();
        let sizes = bricks.sizes(rule, &plan);
        let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
        let held = host.resident(&sizes, &modules).unwrap();
        Self {
            held,
            stages,
            bricks,
            plan,
            sizes,
        }
    }

    fn restart(&mut self, start: &[u32], frame: u32) -> PoolUse {
        for (binding, &words) in self.sizes.iter().enumerate() {
            if binding == PIXELS as usize {
                continue;
            }
            self.held.write(binding, &vec![0_u32; words]).unwrap();
        }
        self.held
            .write(PARAMS as usize, &[frame.wrapping_sub(1), self.plan.dump()])
            .unwrap();
        let opened = self
            .bricks
            .write(&mut self.held, &self.plan, start)
            .unwrap();
        self.bricks.seed(&mut self.held, &self.plan).unwrap();
        opened
    }

    fn step(&mut self) {
        let groups: Vec<&[Groups]> = self.stages.iter().map(Vec::as_slice).collect();
        self.held.staged(&groups).unwrap();
    }

    fn state(&mut self) -> Device {
        let bricks = self.bricks;
        let counts = self
            .held
            .read_runs(&[
                (GRAND as usize, 0, 1),
                (bricks.whole_count() as usize, 0, 1),
                (HOT_GRAND as usize, 0, 1),
                (bricks.tile_args() as usize, 0, TILE_WORDS),
                (bricks.slots() as usize, 0, HEAD),
                (INDIRECT as usize, 0, 3),
                (HOT_INDIRECT as usize, 0, 3),
            ])
            .unwrap();
        let count = |at: usize| counts[at].first().copied().unwrap_or(0) as usize;
        let runs = [
            (DENSE as usize, 0, count(0)),
            (bricks.whole_list() as usize, 0, count(1)),
            (HOT_DENSE as usize, 0, count(2)),
            (bricks.tile_list() as usize, 0, count(3)),
            (bricks.table() as usize, 0, bricks.table_words()),
            (HOT as usize, 0, bricks.count() as usize),
            (AWAKE as usize, 0, bricks.pool_octs() as usize),
        ];
        let mut lists: Vec<Vec<u32>> = Vec::new();
        for run in runs {
            if run.2 == 0 {
                lists.push(Vec::new());
            } else {
                lists.push(self.held.read(run.0, run.2).unwrap());
            }
        }
        let pool = live::read_state(&mut self.held, bricks.windows(), bricks.scratch()).unwrap();
        let table = lists[4].clone();
        Device {
            cells: bricks.unpack(&table, &pool),
            table,
            hot: lists[5].clone(),
            awake: lists[6].clone(),
            header: counts[4].clone(),
            fine: lists[0].clone(),
            whole: lists[1].clone(),
            hot_list: lists[2].clone(),
            tiles: lists[3].clone(),
            args: counts[3].clone(),
            indirect: counts[5].clone(),
            hot_indirect: counts[6].clone(),
        }
    }
}

struct Model {
    bricks: Bricks,
    size: [u32; 3],
    planes: usize,
    per_tile: u32,
    held: Vec<bool>,
    values: Vec<Vec<u32>>,
    hot: Vec<u32>,
    awake: BTreeMap<u32, Vec<u32>>,
    touched: Vec<u32>,
    wanted: Vec<u32>,
    hot_list: Vec<u32>,
    fine: Vec<u32>,
    whole: Vec<u32>,
    tiles: Vec<u32>,
    lit_before: BTreeSet<u32>,
    capacity: u32,
}

#[derive(Debug, Default, Clone)]
struct Moves {
    released: BTreeSet<u32>,
    taken: BTreeSet<u32>,
    overflow: bool,
}

fn uniform_words(cells: &[u32], size: [u32; 3], bricks: &Bricks, brick: u32) -> Vec<Option<u32>> {
    let stride = cells.len() / bricks.planes();
    let per = u32::try_from(bricks.cells()).unwrap();
    (0..bricks.planes())
        .map(|plane| {
            let first = cells[plane * stride + index(size, bricks.point_of(brick, 0))];
            (0..per)
                .all(|offset| {
                    cells[plane * stride + index(size, bricks.point_of(brick, offset))] == first
                })
                .then_some(first)
        })
        .collect()
}

fn near_bricks(lattice: [u32; 3], brick: u32) -> BTreeSet<u32> {
    let at = [
        brick % lattice[0],
        brick / lattice[0] % lattice[1],
        brick / (lattice[0] * lattice[1]),
    ];
    let mut out = BTreeSet::new();
    for dz in 0..3 {
        for dy in 0..3 {
            for dx in 0..3 {
                let step = [dx, dy, dz];
                let near: [u32; 3] = std::array::from_fn(|axis| {
                    (at[axis] + lattice[axis] + step[axis] - 1) % lattice[axis]
                });
                out.insert((near[2] * lattice[1] + near[1]) * lattice[0] + near[0]);
            }
        }
    }
    out
}

impl Model {
    fn seeded(bricks: Bricks, size: [u32; 3], start: &[u32]) -> Self {
        let count = bricks.count();
        let planes = bricks.planes();
        let per_block = bricks.per_block() as usize;
        let mut held = vec![false; count as usize];
        let mut values = vec![vec![0; planes]; count as usize];
        let mut awake = BTreeMap::new();
        for brick in 0..count {
            let words = uniform_words(start, size, &bricks, brick);
            if words.iter().all(Option::is_some) {
                values[brick as usize] = words.into_iter().flatten().collect();
            } else {
                held[brick as usize] = true;
                awake.insert(brick, vec![bricks::AWAKE_LEVEL; per_block]);
            }
        }
        let per_tile = bricks.per_block();
        Self {
            bricks,
            size,
            planes,
            per_tile,
            held,
            values,
            hot: vec![bricks::AWAKE_LEVEL; count as usize],
            awake,
            touched: vec![0; count as usize],
            wanted: vec![0; count as usize],
            hot_list: (0..count).collect(),
            fine: Vec::new(),
            whole: (0..count).collect(),
            tiles: (0..count.div_ceil(per_tile)).collect(),
            lit_before: BTreeSet::new(),
            capacity: bricks.capacity(),
        }
    }

    fn word(&self, cells: &[u32], plane: usize, brick: u32, offset: u32) -> u32 {
        let stride = cells.len() / self.planes;
        cells[plane * stride + index(self.size, self.bricks.point_of(brick, offset))]
    }

    fn listed_octs(&self) -> Vec<u32> {
        let by = self.bricks.octs_bits();
        let mut out = self.fine.clone();
        for &brick in &self.whole {
            out.extend((0..self.bricks.per_block()).map(|local| (brick << by) | local));
        }
        out
    }

    fn frame(&mut self, now: u32, before: &[u32], after: &[u32]) -> Moves {
        let stamp = now.wrapping_add(1).max(1);
        let earlier = now.max(1);
        let by = self.bricks.octs_bits();
        let mut moves = Moves::default();
        for &brick in &self.hot_list.clone() {
            if self.held[brick as usize] {
                let octs = self.awake.get_mut(&brick).unwrap();
                for level in octs.iter_mut() {
                    *level = level.saturating_sub(1);
                }
                self.hot[brick as usize] = octs.iter().copied().max().unwrap_or(0);
            } else {
                self.hot[brick as usize] = self.hot[brick as usize].saturating_sub(1);
            }
        }
        let listed = self.listed_octs();
        for &oct in &listed {
            let placed = self.bricks.cells_of(oct, now);
            for cell in &placed.cells {
                self.touched[cell.brick as usize] = stamp;
            }
            if placed
                .cells
                .iter()
                .all(|cell| self.held[cell.brick as usize])
            {
                continue;
            }
            for cell in &placed.cells {
                if self.held[cell.brick as usize] {
                    continue;
                }
                let differs = (0..self.planes).any(|plane| {
                    self.word(after, plane, cell.brick, cell.offset)
                        != self.values[cell.brick as usize][plane]
                });
                if differs {
                    self.wanted[cell.brick as usize] = stamp;
                }
            }
        }
        for brick in 0..self.bricks.count() {
            let at = brick as usize;
            if self.held[at] && self.touched[at] == earlier && self.touched[at] != stamp {
                let words = uniform_words(before, self.size, &self.bricks, brick);
                if words.iter().all(Option::is_some) {
                    self.held[at] = false;
                    self.values[at] = words.into_iter().flatten().collect();
                    self.awake.remove(&brick);
                    moves.released.insert(brick);
                }
            }
        }
        let mut free =
            self.capacity - u32::try_from(self.held.iter().filter(|one| **one).count()).unwrap();
        for brick in 0..self.bricks.count() {
            let at = brick as usize;
            if !self.held[at] && self.wanted[at] == stamp {
                if free == 0 {
                    moves.overflow = true;
                    continue;
                }
                free -= 1;
                self.held[at] = true;
                self.awake
                    .insert(brick, vec![self.hot[at]; self.bricks.per_block() as usize]);
                moves.taken.insert(brick);
            }
        }
        if moves.overflow {
            return moves;
        }
        for &oct in &listed {
            let placed = self.bricks.cells_of(oct, now);
            let changed = placed.cells.iter().any(|cell| {
                (0..self.planes).any(|plane| {
                    self.word(before, plane, cell.brick, cell.offset)
                        != self.word(after, plane, cell.brick, cell.offset)
                })
            });
            for cell in &placed.cells {
                if self.held[cell.brick as usize] {
                    continue;
                }
                for plane in 0..self.planes {
                    assert_eq!(
                        self.word(after, plane, cell.brick, cell.offset),
                        self.values[cell.brick as usize][plane],
                        "oct {oct} writes a uniform brick {} at frame {now}",
                        cell.brick
                    );
                }
            }
            if !changed {
                continue;
            }
            for near in self.bricks.woken(oct) {
                let brick = near >> by;
                self.hot[brick as usize] = bricks::AWAKE_LEVEL;
                if let Some(octs) = self.awake.get_mut(&brick) {
                    octs[(near & (self.bricks.per_block() - 1)) as usize] = bricks::AWAKE_LEVEL;
                }
            }
        }
        self.hot_list = (0..self.bricks.count())
            .filter(|&brick| self.hot[brick as usize] > 0)
            .collect();
        self.fine = Vec::new();
        self.whole = Vec::new();
        for &brick in &self.hot_list {
            match self.awake.get(&brick) {
                Some(octs) => {
                    for (local, &level) in (0_u32..).zip(octs) {
                        if level > 0 {
                            self.fine.push((brick << by) | local);
                        }
                    }
                }
                None => self.whole.push(brick),
            }
        }
        let lattice = self.bricks.lattice();
        let lit: BTreeSet<u32> = self
            .hot_list
            .iter()
            .flat_map(|&brick| near_bricks(lattice, brick))
            .map(|brick| brick / self.per_tile)
            .collect();
        self.tiles = lit.union(&self.lit_before).copied().collect();
        self.lit_before = lit;
        moves
    }

    fn agrees(&self, got: &Device, label: &str) {
        let bricks = self.bricks;
        for brick in 0..bricks.count() {
            let at = brick as usize;
            let entry = &got.table[at * ENTRY..at * ENTRY + ENTRY];
            assert_eq!(
                entry[0] != UNIFORM,
                self.held[at],
                "{label}: brick {brick} held"
            );
            if !self.held[at] {
                for plane in 0..self.planes {
                    let value = if plane == 0 {
                        entry[VALUE]
                    } else {
                        got.table[bricks.value_at(plane, brick)]
                    };
                    assert_eq!(
                        value, self.values[at][plane],
                        "{label}: brick {brick} value of plane {plane}"
                    );
                }
            }
            assert_eq!(
                entry[TOUCHED], self.touched[at],
                "{label}: brick {brick} touched"
            );
            assert_eq!(
                entry[WANTED], self.wanted[at],
                "{label}: brick {brick} wanted"
            );
            assert_eq!(got.hot[at], self.hot[at], "{label}: brick {brick} hot");
            if let Some(octs) = self.awake.get(&brick) {
                let per = bricks.per_block() as usize;
                let from = entry[0] as usize * per;
                assert_eq!(
                    &got.awake[from..from + per],
                    octs.as_slice(),
                    "{label}: brick {brick} awake"
                );
            }
        }
        assert_eq!(got.hot_list, self.hot_list, "{label}: hot list");
        assert_eq!(got.fine, self.fine, "{label}: listed octs");
        assert_eq!(got.whole, self.whole, "{label}: whole bricks");
        assert_eq!(got.tiles, self.tiles, "{label}: active tiles");
        let lit = u32::try_from(self.tiles.len()).unwrap();
        let per = bricks.per_block();
        let subgroup = per / sweep::PER_LANE;
        assert_eq!(
            got.args,
            vec![lit, lit, 1, 1, lit * sweep::PER_LANE, 1, 1],
            "{label}: tile dispatch"
        );
        let octs = u32::try_from(self.fine.len()).unwrap();
        let whole = u32::try_from(self.whole.len()).unwrap();
        assert_eq!(
            got.indirect[0],
            octs.div_ceil(subgroup) + whole * (per / subgroup),
            "{label}: listed dispatch"
        );
        assert_eq!(
            got.hot_indirect[0],
            u32::try_from(self.hot_list.len()).unwrap(),
            "{label}: hot dispatch"
        );
        let held = u32::try_from(self.held.iter().filter(|one| **one).count()).unwrap();
        assert_eq!(got.header[TOP], self.capacity - held, "{label}: free slots");
        assert_eq!(got.header[OVERFLOW], 0, "{label}: overflow");
        assert_eq!(got.header[LOST], 0, "{label}: lost");
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct Seen {
    frames: u32,
    retaken: u32,
    tiles_both: u32,
    hot_both: u32,
    released: u32,
    taken: u32,
    overflow_at: Option<u32>,
}

fn modelled<R: Rule>(
    host: &Host,
    label: &str,
    rule: &R,
    (size, capacity): ([u32; 3], Option<u32>),
    start: &[u32],
    next: &mut dyn FnMut(&[u32], u32) -> Vec<u32>,
) -> Seen {
    let mut engine = Engine::new(host, rule, size, capacity);
    engine.restart(start, 0);
    let mut model = Model::seeded(engine.bricks, size, start);
    let mut seen = Seen::default();
    let mut cells_now = start.to_vec();
    let mut released_before = BTreeSet::new();
    for k in 1..=64_u32 {
        let now = k - 1;
        let after = next(&cells_now, now);
        engine.step();
        let tiles_before: BTreeSet<u32> = model.tiles.iter().copied().collect();
        let hot_before: BTreeSet<u32> = model.hot_list.iter().copied().collect();
        let moves = model.frame(now, &cells_now, &after);
        let got = engine.state();
        let at = format!("{label} k {k}");
        if moves.overflow {
            assert_eq!(
                got.header[OVERFLOW], 1,
                "{at}: the model overflows the pool here"
            );
            seen.overflow_at = Some(k);
            break;
        }
        assert!(got.cells == after, "{at}: cells differ from the reference");
        model.agrees(&got, &at);
        seen.frames = k;
        seen.retaken += u32::try_from(released_before.intersection(&moves.taken).count()).unwrap();
        seen.released += u32::try_from(moves.released.len()).unwrap();
        seen.taken += u32::try_from(moves.taken.len()).unwrap();
        let tiles_now: BTreeSet<u32> = model.tiles.iter().copied().collect();
        let hot_now: BTreeSet<u32> = model.hot_list.iter().copied().collect();
        if tiles_now.difference(&tiles_before).next().is_some()
            && tiles_before.difference(&tiles_now).next().is_some()
        {
            seen.tiles_both += 1;
        }
        if hot_now.difference(&hot_before).next().is_some()
            && hot_before.difference(&hot_now).next().is_some()
        {
            seen.hot_both += 1;
        }
        released_before = moves.released;
        cells_now = after;
    }
    println!(
        "s1 {label}: {} frames equal to the model, {} released, {} taken, {} taken the frame after their release, {} frames where tiles both join and leave, {} where hot bricks do, overflow at {:?}",
        seen.frames,
        seen.released,
        seen.taken,
        seen.retaken,
        seen.tiles_both,
        seen.hot_both,
        seen.overflow_at
    );
    seen
}

fn on_cpu<R: Rule>(rule: &R, size: [u32; 3]) -> impl FnMut(&[u32], u32) -> Vec<u32> + '_ {
    let grid = Grid3::new(size[0], size[1], size[2]).unwrap();
    move |cells: &[u32], frame: u32| reference(rule, grid, frame, cells, &[])
}

fn dense_of<'host>(
    host: &'host Host,
    law: &Law,
    size: [u32; 3],
    start: &[Planted],
) -> World<'host> {
    let mut world = World::create_law_stepped(
        host,
        law,
        &Made {
            rule: law.name(),
            params: &[],
            size,
            sparse: false,
            slots: None,
        },
        Stepping::Listed,
    )
    .unwrap();
    for (plane, cells) in start {
        world.write_plane(*plane, cells).unwrap();
    }
    world
}

fn sand_rain(size: [u32; 3], side: [u32; 3]) -> Vec<u32> {
    let grid = Grid3::new(size[0], size[1], size[2]).unwrap();
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
        let apart = side[1] + 1 + (x / side[0]) % 2;
        for sheet in 0..3 {
            let y = sheet * apart;
            if y >= floor {
                continue;
            }
            for z in 0..grid.tall {
                if (z / side[2]) % 3 != 1 {
                    cells[grid.index(x, y, z)] = sand2::SAND;
                }
            }
        }
    }
    cells
}

fn sand_pile(size: [u32; 3], side: [u32; 3], at: [u32; 3]) -> Vec<u32> {
    let grid = Grid3::new(size[0], size[1], size[2]).unwrap();
    let mut cells = vec![sand2::AIR; grid.cells()];
    let floor = grid.deep - side[1];
    for z in 0..grid.tall {
        for y in floor..grid.deep {
            for x in 0..grid.wide {
                cells[grid.index(x, y, z)] = sand2::WALL;
            }
        }
    }
    for z in at[2]..at[2] + side[2] {
        for y in at[1]..at[1] + side[1] / 2 {
            for x in at[0] + 1..at[0] + side[0] - 1 {
                cells[grid.index(x, y, z)] = sand2::SAND;
            }
        }
    }
    cells
}

fn movers(size: [u32; 3], side: [u32; 3]) -> Vec<u32> {
    let mut out = vec![0_u32; cells(size)];
    let lanes = (size[1] / side[1]).min(12);
    for lane in 0..lanes {
        let behind = 2 * (1 + lane % 6);
        let base = [
            2 * (lane * 5 % (size[0] / 2)),
            lane * side[1] + side[1] / 2 + 1,
            (lane * 37) % size[2],
        ];
        let heading: [i64; 3] = [1, -1, if base[2].is_multiple_of(2) { 1 } else { -1 }];
        for mover in 0..3_i64 {
            let back = mover * i64::from(behind);
            let at: [i64; 3] =
                std::array::from_fn(|axis| i64::from(base[axis]) - back * heading[axis]);
            out[index(size, wrapped(size, at))] = 1 + lane;
        }
    }
    out
}

fn pair(size: [u32; 3]) -> Vec<u32> {
    let mut out = vec![0_u32; cells(size)];
    out[index(size, [2, 9, 0])] = 1;
    for (lane, z) in [33_u32, 67, 101, 87].into_iter().enumerate() {
        let lane = u32::try_from(lane).unwrap();
        out[index(size, [40 + 48 * lane, 41 + 48 * lane, z])] = 2 + lane;
    }
    out
}

fn law_start<'host>(
    host: &'host Host,
    law: &Law,
    size: [u32; 3],
    start: &[Planted],
) -> (World<'host>, Vec<u32>) {
    let mut world = dense_of(host, law, size, start);
    let cells = world.cells().unwrap();
    (world, cells)
}

fn sides_of(host: &Host, size: [u32; 3]) -> [u32; 3] {
    let width = host.width();
    let grid = Grid::solid(size[0], size[1], size[2]).unwrap();
    let plan = Plan::bricked(width, grid.blocks().div_ceil(sweep::cells(width))).unwrap();
    Bricks::new(grid, &plan, 1).unwrap().sides()
}

#[test]
fn s1_the_brick_lists_follow_a_host_model_every_frame() {
    let Some(host) = device("sparse_lists s1") else {
        return;
    };
    let mut all: Vec<(String, Seen)> = Vec::new();

    let size = [128, 128, 128];
    let side = sides_of(&host, size);
    let start = sand_rain(size, side);
    let seen = modelled(
        &host,
        "sand rain",
        &Sand,
        (size, None),
        &start,
        &mut on_cpu(&Sand, size),
    );
    all.push(("sand rain".to_owned(), seen));

    let tall = [128, 128, 256];
    let start = movers(tall, sides_of(&host, tall));
    let seen = modelled(
        &host,
        "turn movers",
        &Turn,
        (tall, None),
        &start,
        &mut on_cpu(&Turn, tall),
    );
    all.push(("turn movers".to_owned(), seen));

    let wide = [256, 256, 128];
    let start = pair(wide);
    let seen = modelled(
        &host,
        "turn pair across tiles",
        &Turn,
        (wide, Some(256)),
        &start,
        &mut on_cpu(&Turn, wide),
    );
    all.push(("turn pair across tiles".to_owned(), seen));

    let start = sand_pile(size, side, [side[0] * 2, 0, side[2] * 3]);
    let probe = Engine::new(&host, &Sand, size, None);
    let held = probe.bricks.pack(&start).unwrap().held;
    drop(probe);
    let seen = modelled(
        &host,
        "sand pile in a full pool",
        &Sand,
        (size, Some(held + 1)),
        &start,
        &mut on_cpu(&Sand, size),
    );
    assert!(
        seen.overflow_at.is_some(),
        "the small pool never overflowed"
    );
    all.push(("sand pile in a full pool".to_owned(), seen));

    let law = learned_law(&host, Shape::Cube, BAYS);
    let size = [128, 128, 64];
    let mut rng = Rng(0x51);
    let mut placed = Vec::new();
    for z in 0..6 {
        for y in 0..10 {
            for x in 0..10 {
                if rng.below(1000) < 230 {
                    placed.push((wrapped(size, [x - 5, y - 5, z - 3]), 1));
                }
            }
        }
    }
    for cell in glider_cells(Shape::Cube, [40, 40, 20], [0, 2, 1], [true, true]) {
        placed.push((cell, 1));
    }
    for cell in glider_cells(Shape::Cube, [90, 20, 40], [0, 1, 2], [false, true]) {
        placed.push((cell, 1));
    }
    let begun = vec![(0, placed)];
    let (mut dense, start) = law_start(&host, &law, size, &begun);
    let mut next = |_: &[u32], _: u32| {
        dense.step(1).unwrap();
        dense.cells().unwrap()
    };
    let seen = modelled(
        &host,
        "Bays 5766 law",
        &law,
        (size, None),
        &start,
        &mut next,
    );
    all.push(("Bays 5766 law".to_owned(), seen));

    let law = Law::compile(BLUR).unwrap();
    let mut low = Vec::new();
    for z in 0..32 {
        for y in 16..80 {
            for x in 16..80 {
                low.push(([x, y, z], 7));
            }
        }
    }
    for _ in 0..40 {
        let at = [0, 1, 2].map(|axis| u32::try_from(rng.below(u64::from(size[axis]))).unwrap());
        low.push((at, rng.word()));
    }
    let mut high = Vec::new();
    for z in 20..64 {
        for y in 0..96 {
            for x in 64..128 {
                high.push(([x, y, z], 0x0001_0000));
            }
        }
    }
    let begun = vec![(0, low), (1, high)];
    let (mut dense, start) = law_start(&host, &law, size, &begun);
    let mut next = |_: &[u32], _: u32| {
        dense.step(1).unwrap();
        dense.cells().unwrap()
    };
    let seen = modelled(&host, "blur2 law", &law, (size, None), &start, &mut next);
    all.push(("blur2 law".to_owned(), seen));

    let retaken: u32 = all.iter().map(|(_, one)| one.retaken).sum();
    let tiles_both: u32 = all.iter().map(|(_, one)| one.tiles_both).sum();
    let hot_both: u32 = all.iter().map(|(_, one)| one.hot_both).sum();
    assert!(
        retaken > 0,
        "no brick was taken again the frame after its release: {all:?}"
    );
    assert!(
        tiles_both > 0,
        "no frame where tiles both joined and left the list: {all:?}"
    );
    assert!(
        hot_both > 0,
        "no frame where hot bricks both joined and left: {all:?}"
    );
    for (label, one) in &all {
        if one.overflow_at.is_none() {
            assert_eq!(one.frames, 64, "{label}");
        }
    }
}

type Live = BTreeSet<[u32; 3]>;

fn stepped_set(table: Table, size: [u32; 3], live: &Live) -> Live {
    let mut counts: BTreeMap<[u32; 3], u32> = BTreeMap::new();
    for cell in live {
        counts.entry(*cell).or_insert(0);
        for dz in -1_i64..=1 {
            for dy in -1_i64..=1 {
                for dx in -1_i64..=1 {
                    if dx == 0 && dy == 0 && dz == 0 {
                        continue;
                    }
                    let at = [
                        i64::from(cell[0]) + dx,
                        i64::from(cell[1]) + dy,
                        i64::from(cell[2]) + dz,
                    ];
                    *counts.entry(wrapped(size, at)).or_insert(0) += 1;
                }
            }
        }
    }
    counts
        .into_iter()
        .filter(|(cell, count)| {
            let row = if live.contains(cell) {
                table.survive
            } else {
                table.birth
            };
            (row >> count) & 1 == 1
        })
        .map(|(cell, _)| cell)
        .collect()
}

fn read_live(world: &mut World<'_>, size: [u32; 3]) -> Live {
    let plane = world.current().unwrap();
    let to = size.map(|side| side - 1);
    let got = world.scan(plane, [0, 0, 0], to, &[], u32::MAX).unwrap();
    got.cells
        .unwrap()
        .into_iter()
        .map(|(at, word)| {
            assert_eq!(word, 1);
            point(size, at as usize)
        })
        .collect()
}

fn span_bricks(size: [u32; 3], sides: [u32; 3], recent: &[Live], margin: i64) -> u64 {
    let mut boxes: BTreeSet<[u32; 3]> = BTreeSet::new();
    for cell in recent.iter().flatten() {
        let low: [i64; 3] = std::array::from_fn(|axis| i64::from(cell[axis]) - margin);
        let high: [i64; 3] = std::array::from_fn(|axis| i64::from(cell[axis]) + margin);
        for z in low[2]..=high[2] {
            for y in low[1]..=high[1] {
                for x in low[0]..=high[0] {
                    let at = wrapped(size, [x, y, z]);
                    boxes.insert(std::array::from_fn(|axis| at[axis] / sides[axis]));
                }
            }
        }
    }
    u64::try_from(boxes.len()).unwrap()
}

struct Flown {
    most_held: u32,
    most_bound: u64,
    released: u32,
    crossed: [bool; 3],
    tiles_seen: usize,
}

fn flown(
    host: &Host,
    law: &Law,
    size: [u32; 3],
    start: &[[u32; 3]],
    later: Option<(u32, Vec<[u32; 3]>)>,
) -> Flown {
    let mut world = World::create_law_stepped(
        host,
        law,
        &Made {
            rule: law.name(),
            params: &[],
            size,
            sparse: true,
            slots: Some(256),
        },
        Stepping::Listed,
    )
    .unwrap();
    let pool = world.pool().unwrap();
    let sides = pool.sides();
    let per_tile = pool.per_block();
    let lattice = pool.lattice();
    let cells: Vec<([u32; 3], u32)> = start.iter().map(|at| (*at, 1)).collect();
    world.write_plane(0, &cells).unwrap();
    let mut live: Live = start.iter().copied().collect();
    let mut recent = vec![live.clone()];
    let mut seen_at: Vec<[u32; 3]> = vec![*live.iter().next().unwrap()];
    let mut tiles_seen = BTreeSet::new();
    let mut held_before = world.pool_use().unwrap().unwrap().held;
    let mut out = Flown {
        most_held: 0,
        most_bound: 0,
        released: 0,
        crossed: [false; 3],
        tiles_seen: 0,
    };
    for frame in 1..=520_u32 {
        if let Some((at, cells)) = &later
            && *at == frame - 1
        {
            let plane = world.current().unwrap();
            let written: Vec<([u32; 3], u32)> = cells.iter().map(|cell| (*cell, 1)).collect();
            world.write_plane(plane, &written).unwrap();
            live.extend(cells.iter().copied());
        }
        world.step(1).unwrap();
        live = stepped_set(BAYS, size, &live);
        let got = read_live(&mut world, size);
        assert!(
            got == live,
            "glider at frame {frame}: device {got:?}, host {live:?}"
        );
        recent.push(live.clone());
        if recent.len() > 5 {
            recent.remove(0);
        }
        let used = world.pool_use().unwrap().unwrap();
        assert!(!used.overflow && !used.lost, "frame {frame}: {used:?}");
        let most = span_bricks(size, sides, &recent, 4);
        assert!(
            u64::from(used.held) <= most,
            "frame {frame}: {} bricks held, the glider's last generations widened by 4 cells touch {most}",
            used.held
        );
        for cell in &live {
            let brick = (cell[2] / sides[2] * lattice[1] + cell[1] / sides[1]) * lattice[0]
                + cell[0] / sides[0];
            tiles_seen.insert(brick / per_tile);
        }
        let low = *live.iter().next().unwrap();
        let last = *seen_at.last().unwrap();
        for axis in 0..3 {
            let jump = i64::from(low[axis]) - i64::from(last[axis]);
            if jump.abs() > i64::from(size[axis] / 2) {
                out.crossed[axis] = true;
            }
        }
        seen_at.push(low);
        out.most_held = out.most_held.max(used.held);
        out.most_bound = out.most_bound.max(most);
        out.released += held_before.saturating_sub(used.held);
        held_before = used.held;
    }
    out.tiles_seen = tiles_seen.len();
    out
}

#[test]
fn s3_gliders_cross_the_torus_seam_on_every_axis_with_a_bounded_pool() {
    let Some(host) = device("sparse_lists s3") else {
        return;
    };
    let law = learned_law(&host, Shape::Cube, BAYS);
    let size = [256, 256, 256];
    let flights = [
        (
            "in x and y, falling",
            glider_cells(Shape::Cube, [10, 12, 100], [0, 1, 2], [true, true]),
            [true, true, false],
            None,
        ),
        (
            "in x and z, rising",
            glider_cells(Shape::Cube, [226, 60, 228], [0, 2, 1], [false, false]),
            [true, false, true],
            Some((
                200,
                glider_cells(Shape::Cube, [120, 180, 60], [1, 2, 0], [false, true]),
            )),
        ),
        (
            "in y and z, falling",
            glider_cells(Shape::Cube, [100, 14, 10], [1, 2, 0], [true, true]),
            [false, true, true],
            None,
        ),
    ];
    for (label, start, axes, later) in flights {
        let got = flown(&host, &law, size, &start, later);
        println!(
            "s3 Bays glider {label}: 520 frames equal to the host, crossed the seam on x y z {:?}, at most {} bricks held (bound at most {}), {} released, {} tiles visited",
            got.crossed, got.most_held, got.most_bound, got.released, got.tiles_seen
        );
        for (axis, (&asked, &crossed)) in axes.iter().zip(&got.crossed).enumerate() {
            assert!(
                !asked || crossed,
                "{label}: did not cross the seam on axis {axis}"
            );
        }
        assert!(got.released > 0, "{label}: no brick returned");
        if axes[2] {
            assert!(
                got.tiles_seen > 2,
                "{label}: stayed in {} tiles",
                got.tiles_seen
            );
        }
    }
}

impl Engine<'_> {
    fn restart_empty(&mut self, frame: u32) {
        for (binding, &words) in self.sizes.iter().enumerate() {
            if binding == PIXELS as usize {
                continue;
            }
            let mut at = 0;
            while at < words {
                let run = (words - at).min(1 << 20);
                self.held.write_at(binding, at, &vec![0_u32; run]).unwrap();
                at += run;
            }
        }
        self.held
            .write(PARAMS as usize, &[frame.wrapping_sub(1), self.plan.dump()])
            .unwrap();
        let packed = self
            .bricks
            .compose(|_| bricks::Content::Uniform(0))
            .unwrap();
        self.bricks
            .write_packed(&mut self.held, &self.plan, &packed)
            .unwrap();
        self.bricks.seed(&mut self.held, &self.plan).unwrap();
    }

    fn dispatched(&mut self) -> Vec<(String, u32)> {
        let bricks = self.bricks;
        let tier = bricks.tier();
        let read = self
            .held
            .read_runs(&[
                (INDIRECT as usize, 0, 3),
                (HOT_INDIRECT as usize, 0, 3),
                (tier.indirect as usize, 0, 3),
                (bricks.tile_args() as usize, 0, TILE_WORDS),
                (GRAND as usize, 0, 1),
                (bricks.whole_count() as usize, 0, 1),
                (HOT_GRAND as usize, 0, 1),
            ])
            .unwrap();
        vec![
            ("listed octs and whole bricks".to_owned(), read[0][0]),
            ("hot bricks".to_owned(), read[1][0]),
            ("pool list".to_owned(), read[2][0]),
            ("tiles".to_owned(), read[3][bricks::TILE_GROUPS]),
            ("brick lanes".to_owned(), read[3][bricks::LANE_GROUPS]),
            ("octs listed".to_owned(), read[4][0]),
            ("whole bricks listed".to_owned(), read[5][0]),
            ("hot bricks listed".to_owned(), read[6][0]),
        ]
    }
}

#[test]
fn s4_an_empty_world_lists_nothing_and_dispatches_nothing_per_brick() {
    let Some(host) = device("sparse_lists s4") else {
        return;
    };
    let width = host.width();
    let side = [1024_u32, 512, 256, 128]
        .into_iter()
        .find(|&side| {
            let grid = Grid::solid(side, side, side).unwrap();
            Plan::bricked(width, grid.blocks().div_ceil(sweep::cells(width))).is_ok()
        })
        .unwrap();
    let size = [side, side, side];
    let mut engine = Engine::new(&host, &Turn, size, Some(64));
    let count = engine.bricks.count();
    let per = engine.bricks.per_block();
    let tiles = engine.bricks.tile_count();
    let most_fixed = count.div_ceil(per * per) + 1;
    let mut fixed = Vec::new();
    for stage in &engine.stages {
        for groups in stage {
            match *groups {
                Groups::Fixed(groups) | Groups::Counting(groups) => fixed.push(groups),
                Groups::From { binding, at } => {
                    let known = [
                        (INDIRECT as usize, 0),
                        (HOT_INDIRECT as usize, 0),
                        (engine.bricks.tier().indirect as usize, 0),
                        (engine.bricks.tile_args() as usize, bricks::TILE_GROUPS),
                        (engine.bricks.tile_args() as usize, bricks::LANE_GROUPS),
                    ];
                    assert!(
                        known.contains(&(binding, at)),
                        "an indirect dispatch from binding {binding} at {at} is not read back"
                    );
                }
            }
        }
    }
    let widest = fixed.iter().copied().max().unwrap_or(0);
    println!(
        "s4 empty {}^3 world: {count} bricks in {tiles} tiles; the widest fixed dispatch is {widest} groups (bound {most_fixed}), {} fixed dispatches",
        size[0],
        fixed.len()
    );
    assert!(
        widest <= most_fixed,
        "a fixed dispatch of {widest} groups scales with the {count} bricks"
    );
    engine.restart_empty(0);
    for frame in 1..=8_u32 {
        engine.step();
        let got = engine.dispatched();
        let used = engine.bricks.pool_use(&mut engine.held).unwrap();
        assert_eq!(used.held, 0, "frame {frame}: {used:?}");
        if frame <= 2 {
            continue;
        }
        println!("s4 frame {frame}: {got:?}");
        for (name, groups) in &got {
            assert_eq!(
                *groups, 0,
                "frame {frame}: {name} dispatches {groups} groups"
            );
        }
    }
}

#[cfg(feature = "faults")]
mod pinned {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use grit::Host;
    use grit::clock::{Fake, Time};
    use grit::serve3::{Options, Server};
    use grit::wire::{Json, written};

    use super::{BAYS, BLUR, LIFE, LONG, Rng, Shape, device, int_at, learned_on, line, ok, quoted};

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "grit-sparse-lists-{label}-{}-{nanos}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn hex(sum: &[u8; 32]) -> String {
        sum.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    const STEADY: [&str; 7] = [
        "world", "from", "name", "written", "frame", "frames", "tally",
    ];

    #[derive(Clone, Copy)]
    enum Made {
        Learned(Shape),
        Blur,
        Turn,
        Rule(&'static str),
    }

    struct Case {
        name: &'static str,
        made: Made,
        size: [u32; 3],
        sparse: bool,
        planes: u32,
        mask: u32,
        seed: u64,
        slots: Option<u32>,
    }

    const CASES: [Case; 8] = [
        Case {
            name: "dense-life-iterating",
            made: Made::Learned(Shape::Square),
            size: [64, 64, 16],
            sparse: false,
            planes: 2,
            mask: 1,
            seed: 0x52F1,
            slots: None,
        },
        Case {
            name: "dense-blur",
            made: Made::Blur,
            size: [64, 64, 16],
            sparse: false,
            planes: 3,
            mask: 0xFFFF,
            seed: 0x52F2,
            slots: None,
        },
        Case {
            name: "sparse-life-iterating",
            made: Made::Learned(Shape::Square),
            size: [256, 256, 16],
            sparse: true,
            planes: 2,
            mask: 1,
            seed: 0x52F3,
            slots: None,
        },
        Case {
            name: "sparse-bays-iterating",
            made: Made::Learned(Shape::Cube),
            size: [128, 128, 64],
            sparse: true,
            planes: 2,
            mask: 1,
            seed: 0x52F4,
            slots: None,
        },
        Case {
            name: "sparse-blur",
            made: Made::Blur,
            size: [128, 128, 64],
            sparse: true,
            planes: 3,
            mask: 0xFFFF,
            seed: 0x52F5,
            slots: None,
        },
        Case {
            name: "sparse-turn-iterating",
            made: Made::Turn,
            size: [128, 128, 64],
            sparse: true,
            planes: 2,
            mask: u32::MAX,
            seed: 0x52F6,
            slots: None,
        },
        Case {
            name: "sparse-sand",
            made: Made::Rule("sand"),
            size: [128, 128, 64],
            sparse: true,
            planes: 1,
            mask: 3,
            seed: 0x52F7,
            slots: None,
        },
        Case {
            name: "sparse-sand-small-pool",
            made: Made::Rule("sand"),
            size: [128, 128, 64],
            sparse: true,
            planes: 1,
            mask: 3,
            seed: 0x52F8,
            slots: Some(48),
        },
    ];

    fn turn_law() -> String {
        let corner = |k: u32| {
            format!(
                "[\"vec\",[\"at\",[\"at\",\"cells\",{k}],0],[\"at\",[\"at\",\"cells\",{}],0]]",
                k ^ 7
            )
        };
        let out: Vec<String> = (0..8).map(corner).collect();
        format!(
            "{{\"law\":1,\"name\":\"turn\",\"params\":0,\"channels\":2,\"halo\":0,\"tally\":{{\"counts\":1,\"weighted\":false}},\"consts\":{{}},\"defs\":{{}},\"step\":[\"vec\",[\"vec\",{}],[\"vec\",0]]}}",
            out.join(",")
        )
    }

    fn timed<'host>(host: &'host Host, dir: &Path, fake: &Arc<Fake>) -> Server<'host> {
        let time: Arc<dyn Time> = Arc::clone(fake) as Arc<dyn Time>;
        let options = Options {
            dir: Some(dir.to_path_buf()),
            memory: None,
        };
        Server::timed(Some(host), &options, time).unwrap()
    }

    fn steady(answer: &Json) -> String {
        match answer {
            Json::Map(pairs) => written(&Json::Map(
                pairs
                    .iter()
                    .filter(|(key, _)| STEADY.contains(&key.as_str()))
                    .cloned()
                    .collect(),
            )),
            other => written(other),
        }
    }

    struct Kept {
        full: Vec<String>,
        steady: Vec<String>,
    }

    impl Kept {
        fn push(&mut self, answer: &Json) {
            self.full.push(written(answer));
            self.steady.push(steady(answer));
        }

        fn push_both(&mut self, text: String) {
            self.full.push(text.clone());
            self.steady.push(text);
        }
    }

    fn asked(server: &mut Server<'_>, name: &str, arguments: &str) -> Json {
        let got = server.heard(&line(name, arguments)).unwrap();
        let held = got.get("result").cloned().unwrap_or(got);
        match held.get("structuredContent") {
            Some(content) => content.clone(),
            None => held,
        }
    }

    fn scattered(seed: u64, size: [u32; 3], count: usize, mask: u32) -> Vec<String> {
        let mut rng = Rng(seed);
        (0..count)
            .map(|_| {
                let x = u32::try_from(rng.below(u64::from(size[0]))).unwrap();
                let y = u32::try_from(rng.below(u64::from(size[1]))).unwrap();
                let z = u32::try_from(rng.below(u64::from(size[2]))).unwrap();
                format!("[{x},{y},{z},{}]", rng.word() & mask)
            })
            .collect()
    }

    fn write(server: &mut Server<'_>, world: i64, plane: &str, cells: &[String]) -> Json {
        asked(
            server,
            "grit_world_write",
            &format!(
                "{{\"world\":{world},{plane},\"cells\":[{}],\"budget_ms\":{LONG}}}",
                cells.join(",")
            ),
        )
    }

    fn step(server: &mut Server<'_>, world: i64, frames: u32) -> Json {
        asked(
            server,
            "grit_world_step",
            &format!("{{\"world\":{world},\"frames\":{frames},\"budget_ms\":{LONG}}}"),
        )
    }

    fn read_plane(server: &mut Server<'_>, world: i64, size: [u32; 3], plane: u32) -> String {
        let per = (16384 / (size[0] * size[1])).max(1);
        let mut parts = Vec::new();
        let mut z = 0;
        while z < size[2] {
            let top = (z + per).min(size[2]) - 1;
            let got = asked(
                server,
                "grit_world_read",
                &format!(
                    "{{\"world\":{world},\"plane\":{plane},\"form\":\"set\",\"from\":[0,0,{z}],\"to\":[{},{},{top}]}}",
                    size[0] - 1,
                    size[1] - 1
                ),
            );
            parts.push(written(got.get("cells").unwrap_or(&got)));
            z = top + 1;
        }
        parts.join(";")
    }

    fn read_all(server: &mut Server<'_>, world: i64, case: &Case) -> String {
        let mut parts = Vec::new();
        for plane in 0..case.planes {
            parts.push(read_plane(server, world, case.size, plane));
        }
        hex(&grit::branch::sha256(parts.join("|").as_bytes()))
    }

    fn sand_pile(seed: u64, size: [u32; 3]) -> Vec<String> {
        let mut rng = Rng(seed);
        let mut out = Vec::new();
        for z in 8..40 {
            for y in 0..24 {
                for x in 30..70 {
                    if rng.below(10) < 6 {
                        out.push(format!("[{x},{y},{z},1]"));
                    }
                }
            }
        }
        for z in 0..size[2] {
            for x in 0..size[0] {
                out.push(format!("[{x},{},{z},2]", size[1] - 1));
            }
        }
        out
    }

    fn played(server: &mut Server<'_>, case: &Case) -> (i64, Kept) {
        let mut kept = Kept {
            full: Vec::new(),
            steady: Vec::new(),
        };
        let (made, iterating) = match case.made {
            Made::Learned(shape) => {
                let table = if shape == Shape::Square { LIFE } else { BAYS };
                let text = learned_on(server, shape, table);
                kept.push_both(text.clone());
                (
                    format!("\"law\":{},\"alternate\":[0,1]", quoted(&text)),
                    true,
                )
            }
            Made::Blur => (format!("\"law\":{}", quoted(BLUR)), false),
            Made::Turn => (
                format!("\"law\":{},\"alternate\":[0,1]", quoted(&turn_law())),
                true,
            ),
            Made::Rule(name) => (format!("\"rule\":\"{name}\""), false),
        };
        let plane = if iterating {
            "\"current\":true".to_owned()
        } else {
            "\"plane\":0".to_owned()
        };
        let slots = case
            .slots
            .map_or_else(String::new, |slots| format!(",\"slots\":{slots}"));
        let created = ok(
            server,
            "grit_world_create",
            &format!(
                "{{\"wide\":{},\"deep\":{},\"tall\":{},{made},\"sparse\":{}{slots},\"budget_ms\":{LONG}}}",
                case.size[0], case.size[1], case.size[2], case.sparse
            ),
        );
        let world = int_at(&created, "world");
        kept.push(&created);
        let size = case.size;
        if case.slots.is_some() {
            let pile = sand_pile(case.seed, size);
            for piece in pile.chunks(4096) {
                kept.push(&write(server, world, &plane, piece));
            }
            for _ in 0..6 {
                kept.push(&step(server, world, 8));
                kept.push_both(read_all(server, world, case));
            }
            kept.push(&asked(
                server,
                "grit_world_save",
                &format!("{{\"world\":{world},\"name\":\"kept\"}}"),
            ));
            return (world, kept);
        }
        let first = scattered(case.seed, size, 2000, case.mask);
        kept.push(&write(server, world, &plane, &first));
        kept.push(&step(server, world, 3));
        let fork = ok(server, "grit_world_fork", &format!("{{\"world\":{world}}}"));
        let forked = int_at(&fork, "world");
        kept.push(&fork);
        kept.push(&step(server, world, 2));
        let more = scattered(case.seed ^ 1, size, 600, case.mask);
        kept.push(&write(server, world, &plane, &more));
        if case.planes > 2 {
            let upper = scattered(case.seed ^ 2, size, 400, case.mask);
            kept.push(&write(server, world, "\"plane\":1", &upper));
        }
        kept.push(&step(server, world, 5));
        kept.push_both(read_all(server, world, case));
        kept.push(&asked(
            server,
            "grit_world_restore",
            &format!("{{\"world\":{forked},\"from\":{world}}}"),
        ));
        kept.push(&step(server, forked, 3));
        kept.push_both(read_all(server, forked, case));
        kept.push(&asked(
            server,
            "grit_world_save",
            &format!("{{\"world\":{forked},\"name\":\"kept\"}}"),
        ));
        let opened = ok(server, "grit_world_open", "{\"name\":\"kept\"}");
        let reopened = int_at(&opened, "world");
        kept.push(&opened);
        kept.push(&step(server, reopened, 1));
        kept.push_both(read_all(server, reopened, case));
        kept.push(&step(server, world, 7));
        kept.push_both(read_all(server, world, case));
        (world, kept)
    }

    fn files(dir: &Path, into: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                files(&path, into);
            } else if path.file_name().and_then(|name| name.to_str()) != Some("lock") {
                into.push(path);
            }
        }
    }

    fn journal_hash(dir: &Path) -> String {
        let mut names = Vec::new();
        files(dir, &mut names);
        names.sort();
        let mut joined = Vec::new();
        for path in names {
            let name = path
                .strip_prefix(dir)
                .unwrap()
                .to_str()
                .unwrap()
                .replace('\\', "/");
            joined.extend_from_slice(name.as_bytes());
            joined.push(0);
            joined.extend_from_slice(&std::fs::read(&path).unwrap());
            joined.push(0);
        }
        hex(&grit::branch::sha256(&joined))
    }

    struct Traced {
        full: String,
        steady: String,
        journal: String,
    }

    fn trace(host: &Host, case: &Case) -> Traced {
        let scratch = Scratch::new(case.name);
        let dir = scratch.0.join("worlds");
        let fake = Arc::new(Fake::new());
        let (world, mut kept) = {
            let mut server = timed(host, &dir, &fake);
            played(&mut server, case)
        };
        let journal = journal_hash(&dir);
        let fake = Arc::new(Fake::new());
        let mut again = timed(host, &dir, &fake);
        let opened = asked(
            &mut again,
            "grit_world_open",
            &format!("{{\"name\":\"world-{world}\",\"budget_ms\":{LONG}}}"),
        );
        kept.push(&opened);
        if let Some(reopened) = opened.get("world").and_then(Json::int) {
            let replayed = read_all(&mut again, reopened, case);
            kept.push_both(replayed);
        }
        Traced {
            full: hex(&grit::branch::sha256(kept.full.join("\n").as_bytes())),
            steady: hex(&grit::branch::sha256(kept.steady.join("\n").as_bytes())),
            journal,
        }
    }

    const RECORDED: [(&str, u32, &str, &str); 8] = [
        (
            "dense-life-iterating",
            32,
            "a8b9ff1725ab46b22f8395c045ec8804c32bcfc773848a00d9653206abef5ee6",
            "46aa896f272de8c93e74bceb7fc0cfb872890ec86e7a92e85e8f915bcf1e1e5b",
        ),
        (
            "dense-blur",
            32,
            "eddce49b9a990767ad53d3c44beaa4b56e30d52bb5212a01495fc461355d33e9",
            "ae7ec45a9a895982626481853886462e976e15fa93a0bef0d8ca1cf52ada3551",
        ),
        (
            "sparse-life-iterating",
            32,
            "3ad0dba3c42aeeda0b2ab9c96f3da2dc4e653be98e3914f4a602a567740c764c",
            "3b7c414e85ebba3fcbc08fcc041863ed0cdf5bd513589d48d1c8949a279fccc8",
        ),
        (
            "sparse-bays-iterating",
            32,
            "c4b826cf02b1663b39879d4970477cc5b409d7de93429644ba9aa4845b056bf0",
            "dcc6e2e4344e51c0f3fdf39a8e6dafa510a3451240c7202f2e81c8ce226ac4ce",
        ),
        (
            "sparse-blur",
            32,
            "922b0860c4c76dec39008ca8d48bd81ee86247bcf0cd5932ee2634e87ece0f5e",
            "31966d3a49c81a445d223c9092ec9c776cf55c5784f8c95d9463a8ff3bbcd565",
        ),
        (
            "sparse-turn-iterating",
            32,
            "a0caae9066a69f8cde90bbbc7000e75b1bc9c3bd6311ddb9a3538a11888bdf6c",
            "8febd70c3208b76c452584f762e718c366eaf784ff4f279d0178d5617378bf92",
        ),
        (
            "sparse-sand",
            32,
            "bcc223742a96af5c66ad432d7bee74f049b0b0ff315bf06674fb6081c019bd21",
            "1580f641ac2e54174ae52f371bb930e7fafc6b67e14ccacf0febffd33e63e7d9",
        ),
        (
            "sparse-sand-small-pool",
            32,
            "8c8f49416b1c9a66a1d5b8d5f874f4cf33d9862af630111599f278f079242ce8",
            "b872d24721f814c06763a2fc15aac85802740f6dc5bbc1a2ca6c31d098b0b0c3",
        ),
    ];

    const STEADY_RECORDED: [(&str, &str); 8] = [
        (
            "dense-life-iterating",
            "8bb6746a2d07760209c4c3ddcecb90e572903706f4e16fb035fd2f83fd9248b6",
        ),
        (
            "dense-blur",
            "d07358e11594976c5dfb0e5c2ba1d72b190fb78ca8f3dd952feed08ed9944a09",
        ),
        (
            "sparse-life-iterating",
            "853e112e1ce51247c0f7cefe7db723658586271641a7edfedfe7287f24432dea",
        ),
        (
            "sparse-bays-iterating",
            "e1d65288c4170867982222446351ccfa80a5d958bbbfde7fc0e5a3bde081651d",
        ),
        (
            "sparse-blur",
            "70c720d133efd9b32e430194ead62612c9e9dc0293643e7ea4bc0e552442df6b",
        ),
        (
            "sparse-turn-iterating",
            "1f70e5115087b7765de6515ea31b18e5c3b21a0e24f49c1b4dd99e7cb64d3977",
        ),
        (
            "sparse-sand",
            "3d636eeb7c71cc4bb7887172128c0ebe4ccf73f358282e2e04fbe48b6b9cf6f4",
        ),
        (
            "sparse-sand-small-pool",
            "ea0e5e847f611f05739d78e51fefc6a383d69ddfb26048c06545ea65a30c05cd",
        ),
    ];

    #[test]
    fn s2_sparse_and_dense_worlds_answer_read_and_journal_as_on_base() {
        let Some(host) = device("sparse_lists s2") else {
            return;
        };
        let width = host.width();
        let got: Vec<Traced> = CASES.iter().map(|case| trace(&host, case)).collect();
        for (case, traced) in CASES.iter().zip(&got) {
            println!(
                "RECORD {} {width} {} {} {}",
                case.name, traced.full, traced.journal, traced.steady
            );
        }
        let mut checked = 0;
        for (case, traced) in CASES.iter().zip(&got) {
            let (_, _, want_full, want_journal) = RECORDED
                .iter()
                .find(|(name, at, _, _)| *name == case.name && (*at == width || !case.sparse))
                .copied()
                .unwrap_or(("", 0, "", ""));
            if case.slots.is_some() && want_full.is_empty() {
                println!(
                    "s2 {}: its pool overflows by brick size; no record at subgroup width {width}",
                    case.name
                );
                continue;
            }
            let (_, want_steady) = STEADY_RECORDED
                .iter()
                .find(|(name, _)| *name == case.name)
                .copied()
                .unwrap_or_else(|| panic!("{}: no steady record", case.name));
            assert_eq!(
                traced.steady, want_steady,
                "{}: steady answers and reads",
                case.name
            );
            if want_full.is_empty() {
                println!(
                    "s2 {}: no record at subgroup width {width}; steady answers and reads checked",
                    case.name
                );
                continue;
            }
            assert_eq!(
                traced.full, want_full,
                "{}: answers and reads at width {width}",
                case.name
            );
            assert_eq!(
                traced.journal, want_journal,
                "{}: files at width {width}",
                case.name
            );
            checked += 1;
        }
        if width == 32 {
            assert_eq!(
                checked,
                CASES.len(),
                "the 4080's width checks every case in full"
            );
        }
    }
}
