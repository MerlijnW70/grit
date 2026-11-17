#![allow(dead_code)]
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use grit::halo::BOX;
use grit::law::Law;
use grit::law::adjoint::{Aim, Fault, Reversible, Walked};
use grit::ops::{Cpu, Flag, Helpers, Ops};
use grit::question::Region;
use grit::rule3::{CORNERS, Oct};
use grit::sand3::{Grid3, reference};
use grit::serve3::gradient::cut;

pub const ONE: i128 = 1 << 64;

pub const SHIFTS: u32 = 64;

pub fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

pub fn below(state: &mut u64, most: u64) -> u64 {
    next(state) % most
}

fn pick<T: Copy>(state: &mut u64, from: &[T]) -> T {
    from[usize::try_from(below(state, from.len() as u64)).unwrap()]
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Dual {
    pub value: u32,
    pub slope: i128,
    pub undefined: bool,
}

#[derive(Debug, Default)]
pub struct Duals {
    pub inexact: bool,
    pub overflowed: bool,
}

impl Duals {
    fn held(&mut self, made: Option<i128>) -> i128 {
        made.unwrap_or_else(|| {
            self.overflowed = true;
            0
        })
    }

    fn carried(&mut self, value: u32, slope: Option<i128>, undefined: bool) -> Dual {
        let slope = self.held(slope);
        Dual {
            value,
            slope,
            undefined,
        }
    }

    fn halved(&mut self, slope: i128, by: i128) -> i128 {
        if by == 0 {
            self.overflowed = true;
            return 0;
        }
        if slope % by != 0 {
            self.inexact = true;
        }
        slope / by
    }

    const fn severed(value: u32, operands: &[Dual]) -> Dual {
        let mut undefined = false;
        let mut at = 0;
        while at < operands.len() {
            if operands[at].slope != 0 || operands[at].undefined {
                undefined = true;
            }
            at += 1;
        }
        Dual {
            value,
            slope: 0,
            undefined,
        }
    }
}

impl Ops for Duals {
    type Word = Dual;

    fn constant(&mut self, value: u32) -> Dual {
        Dual {
            value,
            slope: 0,
            undefined: false,
        }
    }

    fn and(&mut self, left: Dual, right: Dual) -> Dual {
        Self::severed(Cpu.and(left.value, right.value), &[left, right])
    }

    fn or(&mut self, left: Dual, right: Dual) -> Dual {
        Self::severed(Cpu.or(left.value, right.value), &[left, right])
    }

    fn xor(&mut self, left: Dual, right: Dual) -> Dual {
        Self::severed(Cpu.xor(left.value, right.value), &[left, right])
    }

    fn not(&mut self, value: Dual) -> Dual {
        Self::severed(Cpu.not(value.value), &[value])
    }

    fn shift_left(&mut self, value: Dual, by: u32) -> Dual {
        let slope = value.slope.checked_mul(1_i128 << (by % 32));
        self.carried(Cpu.shift_left(value.value, by), slope, value.undefined)
    }

    fn shift_right(&mut self, value: Dual, by: u32) -> Dual {
        let slope = self.halved(value.slope, 1_i128 << (by % 32));
        Dual {
            value: Cpu.shift_right(value.value, by),
            slope,
            undefined: value.undefined,
        }
    }

    fn add(&mut self, left: Dual, right: Dual) -> Dual {
        self.carried(
            Cpu.add(left.value, right.value),
            left.slope.checked_add(right.slope),
            left.undefined || right.undefined,
        )
    }

    fn sub(&mut self, left: Dual, right: Dual) -> Dual {
        self.carried(
            Cpu.sub(left.value, right.value),
            left.slope.checked_sub(right.slope),
            left.undefined || right.undefined,
        )
    }

    fn mul(&mut self, left: Dual, right: Dual) -> Dual {
        let slope = left
            .slope
            .checked_mul(i128::from(right.value))
            .and_then(|one| one.checked_add(i128::from(left.value).checked_mul(right.slope)?));
        self.carried(
            Cpu.mul(left.value, right.value),
            slope,
            left.undefined || right.undefined,
        )
    }

    fn div(&mut self, left: Dual, right: Dual) -> Dual {
        let value = Cpu.div(left.value, right.value);
        let undefined = left.undefined || right.undefined;
        if right.value == 0 {
            return Dual {
                value,
                slope: left.slope,
                undefined,
            };
        }
        let below = i128::from(right.value);
        let first = self.halved(left.slope, below);
        let against = i128::from(left.value).checked_mul(right.slope);
        let against = self.held(against);
        let second = self.halved(against, below * below);
        self.carried(value, first.checked_sub(second), undefined)
    }

    fn dot_signed(&mut self, left: Dual, right: Dual) -> Dual {
        Self::severed(Cpu.dot_signed(left.value, right.value), &[left, right])
    }

    fn dot_unsigned(&mut self, left: Dual, right: Dual) -> Dual {
        Self::severed(Cpu.dot_unsigned(left.value, right.value), &[left, right])
    }

    fn equal(&mut self, left: Dual, right: Dual) -> Flag<Dual> {
        self.truth(left.value == right.value)
    }

    fn not_equal(&mut self, left: Dual, right: Dual) -> Flag<Dual> {
        self.truth(left.value != right.value)
    }

    fn less(&mut self, left: Dual, right: Dual) -> Flag<Dual> {
        self.truth(left.value < right.value)
    }

    fn less_equal(&mut self, left: Dual, right: Dual) -> Flag<Dual> {
        self.truth(left.value <= right.value)
    }

    fn greater(&mut self, left: Dual, right: Dual) -> Flag<Dual> {
        self.truth(left.value > right.value)
    }

    fn greater_equal(&mut self, left: Dual, right: Dual) -> Flag<Dual> {
        self.truth(left.value >= right.value)
    }

    fn select(&mut self, flag: Flag<Dual>, then: Dual, otherwise: Dual) -> Dual {
        if flag.word().value != 0 {
            then
        } else {
            otherwise
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seed {
    Param(usize),
    Cell(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Derived {
    Exact(i128),
    Undefined,
    Overflowed,
}

pub struct Posing<'a> {
    pub grid: Grid3,
    pub planes: usize,
    pub start: &'a [u32],
    pub params: &'a [u32],
    pub first: u32,
    pub frames: u32,
    pub region: Region,
    pub target: &'a [u32],
}

pub struct Forward {
    pub ended: Vec<u32>,
    pub derived: Derived,
    pub inexact: bool,
}

fn wrapped(origin: u32, step: i64, side: u32) -> u32 {
    let at = i64::from(origin) + step;
    u32::try_from(at.rem_euclid(i64::from(side))).unwrap()
}

fn spot(grid: Grid3, origin: [u32; 3], step: [i64; 3]) -> usize {
    let x = wrapped(origin[0], step[0], grid.wide);
    let y = wrapped(origin[1], step[1], grid.deep);
    let z = wrapped(origin[2], step[2], grid.tall);
    usize::try_from((z * grid.deep + y) * grid.wide + x).unwrap()
}

pub const fn corner_step(corner: usize) -> [i64; 3] {
    [
        (corner & 1) as i64,
        (corner >> 2) as i64,
        ((corner >> 1) & 1) as i64,
    ]
}

pub const fn box_step(cell: usize) -> [i64; 3] {
    [
        (cell % 4) as i64 - 1,
        ((cell / 4) % 4) as i64 - 1,
        (cell / 16) as i64 - 1,
    ]
}

pub fn forward(law: &Law, posing: &Posing<'_>, seed: Option<Seed>) -> Forward {
    let dag = law.dag();
    let grid = posing.grid;
    let stride = grid.cells();
    let mut ops = Duals::default();
    let mut state: Vec<Dual> = posing
        .start
        .iter()
        .enumerate()
        .map(|(at, &value)| Dual {
            value,
            slope: if seed == Some(Seed::Cell(at)) { ONE } else { 0 },
            undefined: false,
        })
        .collect();
    let params: Vec<Dual> = posing
        .params
        .iter()
        .enumerate()
        .map(|(at, &value)| Dual {
            value,
            slope: if seed == Some(Seed::Param(at)) {
                ONE
            } else {
                0
            },
            undefined: false,
        })
        .collect();
    for done in 0..posing.frames {
        let frame = posing.first.wrapping_add(done);
        let phase = frame & 1;
        let before = state.clone();
        for lz in 0..grid.tall / 2 {
            for ly in 0..grid.deep / 2 {
                for lx in 0..grid.wide / 2 {
                    let origin = [2 * lx + phase, 2 * ly + phase, 2 * lz + phase];
                    let own: [usize; CORNERS] =
                        core::array::from_fn(|corner| spot(grid, origin, corner_step(corner)));
                    let words = own.map(|place| before[place]);
                    let upper: Vec<[Dual; CORNERS]> = (1..posing.planes)
                        .map(|plane| own.map(|place| before[plane * stride + place]))
                        .collect();
                    let boxes: Vec<[Dual; BOX]> = (0..posing.planes)
                        .map(|plane| {
                            core::array::from_fn(|cell| {
                                before[plane * stride + spot(grid, origin, box_step(cell))]
                            })
                        })
                        .collect();
                    let frame_word = ops.constant(frame);
                    let at = [lx, ly, lz].map(|axis| ops.constant(axis));
                    let planed = dag.replay_halo(
                        &mut ops,
                        Oct {
                            words,
                            frame: frame_word,
                            at,
                            params: &params,
                        },
                        &upper,
                        &boxes,
                    );
                    for (corner, &place) in own.iter().enumerate() {
                        state[place] = planed.ruled.out[corner];
                        for plane in 1..posing.planes {
                            state[plane * stride + place] = planed.planes[plane - 1][corner];
                        }
                    }
                }
            }
        }
    }
    let ended: Vec<u32> = state.iter().map(|one| one.value).collect();
    let mut total: Option<i128> = Some(0);
    let mut undefined = false;
    let mut aimed = posing.target.iter();
    let [from_x, from_y, from_z] = posing.region.from;
    let [to_x, to_y, to_z] = posing.region.to;
    for z in from_z..to_z {
        for y in from_y..to_y {
            for x in from_x..to_x {
                let wanted = *aimed.next().unwrap();
                let held = state[usize::try_from((z * grid.deep + y) * grid.wide + x).unwrap()];
                let sign: i128 = match held.value.cmp(&wanted) {
                    core::cmp::Ordering::Greater => 1,
                    core::cmp::Ordering::Less => -1,
                    core::cmp::Ordering::Equal => 0,
                };
                if sign == 0 {
                    continue;
                }
                undefined |= held.undefined;
                total = total.and_then(|sum| sum.checked_add(held.slope.checked_mul(sign)?));
            }
        }
    }
    let derived = match (ops.overflowed, total, undefined) {
        (true, _, _) | (_, None, _) => Derived::Overflowed,
        (false, Some(_), true) => Derived::Undefined,
        (false, Some(sum), false) => Derived::Exact(sum),
    };
    Forward {
        ended,
        derived,
        inexact: ops.inexact,
    }
}

pub fn ran(law: &Law, posing: &Posing<'_>, params: &[u32]) -> Vec<u32> {
    let mut cells = posing.start.to_vec();
    for done in 0..posing.frames {
        cells = reference(
            law.dag(),
            posing.grid,
            posing.first.wrapping_add(done),
            &cells,
            params,
        );
    }
    cells
}

pub fn boxed(grid: Grid3, region: Region, cells: &[u32]) -> Vec<u32> {
    let mut out = Vec::new();
    for z in region.from[2]..region.to[2] {
        for y in region.from[1]..region.to[1] {
            for x in region.from[0]..region.to[0] {
                out.push(cells[grid.index(x, y, z)]);
            }
        }
    }
    out
}

#[derive(Clone, Debug)]
pub struct Case {
    pub seed: u64,
    pub text: String,
    pub params: Vec<u32>,
    pub size: [u32; 3],
    pub planes: usize,
    pub frozen: Vec<bool>,
    pub frames: u32,
    pub first: u32,
    pub start: Vec<u32>,
    pub region: Region,
    pub target: Vec<u32>,
}

impl Case {
    pub fn grid(&self) -> Grid3 {
        Grid3::new(self.size[0], self.size[1], self.size[2]).unwrap()
    }

    pub fn posing(&self) -> Posing<'_> {
        Posing {
            grid: self.grid(),
            planes: self.planes,
            start: &self.start,
            params: &self.params,
            first: self.first,
            frames: self.frames,
            region: self.region,
            target: &self.target,
        }
    }

    pub fn aim(&self) -> Aim<'_> {
        Aim {
            region: self.region,
            target: &self.target,
        }
    }

    pub fn walked(&self, law: &Reversible) -> Result<Walked, Fault> {
        law.walk(
            self.grid(),
            &self.start,
            &self.params,
            self.first,
            self.frames,
            &self.aim(),
        )
    }
}

pub const SIZES: [[u32; 3]; 7] = [
    [2, 2, 2],
    [4, 4, 4],
    [8, 4, 2],
    [2, 4, 8],
    [4, 2, 4],
    [8, 8, 4],
    [16, 4, 4],
];

pub struct Builder {
    pub planes: usize,
    pub frozen: Vec<bool>,
    pub params: usize,
    pub divisors: Vec<usize>,
    pub budget: u32,
    pub outside: usize,
    versions: Vec<u32>,
    bindings: Vec<String>,
}

pub fn cell_of(planes: usize, word: usize) -> String {
    if planes == 1 {
        format!("[\"at\",\"cells\",{word}]")
    } else {
        format!("[\"at\",[\"at\",\"cells\",{}],{}]", word % 8, word / 8)
    }
}

impl Builder {
    pub fn new(
        planes: usize,
        frozen: Vec<bool>,
        params: usize,
        divisors: Vec<usize>,
        budget: u32,
    ) -> Self {
        Self {
            planes,
            frozen,
            params,
            divisors,
            budget,
            outside: 0,
            versions: vec![0; 8 * planes],
            bindings: (0..8 * planes)
                .map(|word| format!("[\"w{word}v0\",{}]", cell_of(planes, word)))
                .collect(),
        }
    }

    pub fn named(&self, word: usize) -> String {
        format!("\"w{word}v{}\"", self.versions[word])
    }

    pub fn evolving(&self) -> Vec<usize> {
        (0..8 * self.planes)
            .filter(|word| !self.frozen[word / 8])
            .collect()
    }

    pub fn lift(&mut self, target: usize, op: &str, by: &str) {
        let before = self.named(target);
        self.versions[target] += 1;
        let after = self.named(target);
        self.bindings
            .push(format!("[{after},[\"{op}\",{before},{by}]]"));
    }

    pub fn look(&mut self, state: &mut u64) -> String {
        let looked: Vec<usize> = (0..self.planes)
            .filter(|&plane| self.frozen[plane])
            .collect();
        let plane = pick(state, &looked);
        let step = [0, 1, 2].map(|_| i64::try_from(below(state, 3)).unwrap() - 1);
        let corner = usize::try_from(below(state, 8)).unwrap();
        let reached = corner_step(corner);
        if (0..3).any(|axis| !(0..=1).contains(&(reached[axis] + step[axis]))) {
            self.outside += 1;
        }
        format!(
            "[\"look\",{plane},{},{},{},{corner}]",
            step[0], step[1], step[2]
        )
    }

    pub fn outside_look(&mut self, state: &mut u64) -> String {
        let looked: Vec<usize> = (0..self.planes)
            .filter(|&plane| self.frozen[plane])
            .collect();
        let plane = pick(state, &looked);
        let corner = usize::try_from(below(state, 8)).unwrap();
        let reached = corner_step(corner);
        let step = reached.map(|one| if one == 0 { -1 } else { 1 });
        let kept = below(state, 7) + 1;
        let step: Vec<i64> = (0..3)
            .map(|axis| if kept >> axis & 1 == 1 { step[axis] } else { 0 })
            .collect();
        self.outside += 1;
        format!(
            "[\"look\",{plane},{},{},{},{corner}]",
            step[0], step[1], step[2]
        )
    }

    fn other(&self, state: &mut u64, target: usize) -> String {
        let words = 8 * self.planes;
        let pick = (target + 1 + usize::try_from(below(state, words as u64 - 1)).unwrap()) % words;
        self.named(pick)
    }

    fn still(&mut self, state: &mut u64) -> String {
        match below(state, 6) {
            0 | 1 => self.look(state),
            2 => "\"frame\"".to_owned(),
            3 => format!("[\"at\",\"at\",{}]", below(state, 3)),
            4 => "[\"noise\"]".to_owned(),
            _ => below(state, 1 << 12).to_string(),
        }
    }

    fn divisor(&mut self, state: &mut u64) -> String {
        match below(state, 3) {
            0 if !self.divisors.is_empty() => {
                format!("[\"param\",{}]", pick(state, &self.divisors))
            }
            1 => self.look(state),
            _ => (1_u32 << below(state, 4)).to_string(),
        }
    }

    fn leaf(&mut self, state: &mut u64, target: usize) -> String {
        match below(state, 8) {
            0..=2 => self.other(state, target),
            3 | 4 => self.look(state),
            5 if self.params > 0 => format!("[\"param\",{}]", below(state, self.params as u64)),
            6 => self.still(state),
            _ => below(state, 16).to_string(),
        }
    }

    pub fn expr(&mut self, state: &mut u64, target: usize, depth: u32) -> String {
        if depth == 0 || below(state, 4) == 0 {
            return self.leaf(state, target);
        }
        let less = depth - 1;
        match below(state, 12) {
            0 | 1 => {
                let op = pick(state, &["add", "sub"]);
                let left = self.expr(state, target, less);
                let right = self.expr(state, target, less);
                format!("[\"{op}\",{left},{right}]")
            }
            2 | 3 => {
                let left = self.expr(state, target, less);
                let right = self.expr(state, target, less);
                format!("[\"mul\",{left},{right}]")
            }
            4 => {
                let shifted = self.expr(state, target, less);
                format!("[\"shl\",{shifted},{}]", below(state, 3))
            }
            5 if self.budget > 0 => {
                let by = 1 + u32::try_from(below(state, u64::from(self.budget.min(6)))).unwrap();
                self.budget -= by;
                let shifted = self.expr(state, target, less);
                format!("[\"shr\",{shifted},{by}]")
            }
            6 if self.budget >= 6 => {
                self.budget -= 6;
                let left = self.expr(state, target, less);
                let below = self.divisor(state);
                format!("[\"div\",{left},{below}]")
            }
            7 | 8 => {
                let op = pick(state, &["lt", "le", "gt", "ge", "eq", "ne"]);
                let left = self.expr(state, target, less);
                let right = self.expr(state, target, less);
                let then = self.expr(state, target, less);
                let otherwise = self.expr(state, target, less);
                format!("[\"sel\",[\"{op}\",{left},{right}],{then},{otherwise}]")
            }
            9 => {
                let op = pick(state, &["xor", "and", "or", "dot_s", "dot_u"]);
                let left = self.still(state);
                let right = self.still(state);
                format!("[\"{op}\",{left},{right}]")
            }
            10 => format!("[\"not\",{}]", self.still(state)),
            _ => {
                let left = self.expr(state, target, less);
                let right = self.look(state);
                format!("[\"mul\",{left},{right}]")
            }
        }
    }

    pub fn text(&self, name: &str, count: &str) -> String {
        let out: Vec<String> = (0..8)
            .map(|corner| {
                if self.planes == 1 {
                    self.named(corner)
                } else {
                    let planes: Vec<String> = (0..self.planes)
                        .map(|plane| self.named(plane * 8 + corner))
                        .collect();
                    format!("[\"vec\",{}]", planes.join(","))
                }
            })
            .collect();
        format!(
            "{{\"law\":1,\"name\":\"{name}\",\"params\":{},\"channels\":{},\"halo\":1,\"reversible\":true,\"tally\":{{\"counts\":1,\"weighted\":false}},\"consts\":{{}},\"defs\":{{}},\"step\":[\"let\",[{}],[\"vec\",[\"vec\",{}],[\"vec\",{count}]]]}}",
            self.params,
            self.planes,
            self.bindings.join(","),
            out.join(",")
        )
    }
}

pub fn admitted(text: &str) -> Option<(Law, Reversible)> {
    let law = Law::compile(text).ok()?;
    let reversible = Reversible::of(&law).ok()?;
    cut(reversible.forward())
        .is_none()
        .then_some((law, reversible))
}

fn drafted(seed: u64) -> (Case, Vec<u32>) {
    let mut state = seed;
    let size = pick(&mut state, &SIZES);
    let planes = 1 + usize::try_from(below(&mut state, 3)).unwrap();
    let mut frozen = vec![true; planes];
    if planes > 1 {
        let evolving = 1 + usize::try_from(below(&mut state, planes as u64 - 1)).unwrap();
        let mut chosen = 0;
        while chosen < evolving {
            let plane = if below(&mut state, 4) == 0 {
                usize::try_from(below(&mut state, planes as u64)).unwrap()
            } else {
                chosen
            };
            if frozen[plane] {
                frozen[plane] = false;
                chosen += 1;
            }
        }
    }
    let frames = 1 + u32::try_from(below(&mut state, 8)).unwrap();
    let first = u32::try_from(below(&mut state, 5)).unwrap();
    let params = usize::try_from(below(&mut state, 4).max(below(&mut state, 4))).unwrap();
    let divisors: Vec<usize> = (0..params).filter(|_| below(&mut state, 3) == 0).collect();
    let mut builder = Builder::new(
        planes,
        frozen.clone(),
        params,
        divisors.clone(),
        SHIFTS / frames,
    );
    let evolving = builder.evolving();
    if !evolving.is_empty() {
        let lifts = 1 + below(&mut state, 5);
        let first: Vec<usize> = evolving.iter().copied().filter(|&word| word < 8).collect();
        for at in 0..lifts {
            let target = if !first.is_empty() && below(&mut state, 2) == 0 {
                pick(&mut state, &first)
            } else {
                pick(&mut state, &evolving)
            };
            let op = match below(&mut state, 10) {
                0 => "xor",
                1..=4 => "sub",
                _ => "add",
            };
            let mut by = builder.expr(&mut state, target, 3);
            if at == 0 || below(&mut state, 2) == 0 {
                let looked = builder.outside_look(&mut state);
                let other = builder.expr(&mut state, target, 1);
                let other = if params > 0 {
                    format!(
                        "[\"add\",[\"param\",{}],{other}]",
                        below(&mut state, params as u64)
                    )
                } else {
                    other
                };
                by = format!("[\"add\",{by},[\"mul\",{looked},{other}]]");
            }
            builder.lift(target, op, &by);
        }
    }
    let count = builder.outside_look(&mut state);
    let text = builder.text(&format!("looker{seed}"), &count);
    let values: Vec<u32> = (0..params)
        .map(|at| {
            if divisors.contains(&at) {
                1 << below(&mut state, 4)
            } else {
                u32::try_from(below(&mut state, 32)).unwrap()
            }
        })
        .collect();
    let grid = Grid3::new(size[0], size[1], size[2]).unwrap();
    let stride = grid.cells();
    let start: Vec<u32> = (0..stride * planes)
        .map(|at| {
            if frozen[at / stride] {
                1 << below(&mut state, 4)
            } else {
                u32::try_from(below(&mut state, 1 << 10)).unwrap()
            }
        })
        .collect();
    let mut from = [0; 3];
    let mut to = [0; 3];
    for axis in 0..3 {
        let one = u32::try_from(below(&mut state, u64::from(size[axis]))).unwrap();
        let other = u32::try_from(below(&mut state, u64::from(size[axis]))).unwrap();
        from[axis] = one.min(other);
        to[axis] = one.max(other) + 1;
    }
    let region = Region::new(from, to);
    let shifted: Vec<u32> = values
        .iter()
        .map(|&one| one.wrapping_add(u32::try_from(below(&mut state, 3)).unwrap()))
        .collect();
    (
        Case {
            seed,
            text,
            params: values,
            size,
            planes,
            frozen,
            frames,
            first,
            start,
            region,
            target: Vec::new(),
        },
        shifted,
    )
}

pub struct Drawn {
    pub case: Case,
    pub law: Law,
    pub reversible: Reversible,
    pub refused: u32,
}

pub fn case(seed: u64) -> Drawn {
    let mut refused = 0;
    for attempt in 0..64 {
        let (mut case, shifted) = drafted(seed.wrapping_mul(64).wrapping_add(attempt));
        let Some((law, reversible)) = admitted(&case.text) else {
            refused += 1;
            continue;
        };
        let ended = ran(&law, &case.posing(), &shifted);
        case.target = boxed(case.grid(), case.region, &ended);
        return Drawn {
            case,
            law,
            reversible,
            refused,
        };
    }
    panic!("no admitted law near seed {seed}");
}

#[derive(Debug, Default)]
pub struct Tally {
    pub laws: u32,
    pub refused: u32,
    pub overflowed: u32,
    pub told: u32,
    pub moved: u32,
    pub slopes: u32,
    pub looked: u32,
    pub undefined: u32,
    pub skipped: u32,
}

pub fn seeds(case: &Case, state: &mut u64) -> Vec<usize> {
    let cells = case.start.len();
    if cells <= 192 {
        return (0..cells).collect();
    }
    let stride = cells / case.planes;
    let mut picked: Vec<usize> = (0..96)
        .map(|at| {
            let plane = if at % 2 == 0 {
                let frozen: Vec<usize> = (0..case.planes).filter(|&one| case.frozen[one]).collect();
                pick(state, &frozen)
            } else {
                usize::try_from(below(state, case.planes as u64)).unwrap()
            };
            plane * stride + usize::try_from(below(state, stride as u64)).unwrap()
        })
        .collect();
    picked.sort_unstable();
    picked.dedup();
    picked
}

pub fn checked(drawn: &Drawn, walked: &Walked, tally: &mut Tally) {
    let case = &drawn.case;
    let posing = case.posing();
    let plain = forward(&drawn.law, &posing, None);
    assert_eq!(
        plain.ended,
        ran(&drawn.law, &posing, &case.params),
        "seed {}: the dual run is the law's run",
        case.seed
    );
    assert_eq!(plain.ended, walked.ended, "seed {}: ended", case.seed);
    for (index, &told) in walked.told.iter().enumerate() {
        let made = forward(&drawn.law, &posing, Some(Seed::Param(index)));
        assert!(!made.inexact, "seed {}: param {index} lost bits", case.seed);
        match made.derived {
            Derived::Exact(derivative) => {
                assert_eq!(
                    told, derivative,
                    "seed {} param {index}: adjoint {told}, dual {derivative}\n{}",
                    case.seed, case.text
                );
                tally.told += 1;
                if told != 0 {
                    tally.moved += 1;
                }
            }
            Derived::Undefined => panic!(
                "seed {} param {index}: an admitted law has no derivative\n{}",
                case.seed, case.text
            ),
            Derived::Overflowed => tally.skipped += 1,
        }
    }
    let stride = case.start.len() / case.planes;
    let mut state = case.seed ^ 0x5EED;
    for at in seeds(case, &mut state) {
        let made = forward(&drawn.law, &posing, Some(Seed::Cell(at)));
        assert!(!made.inexact, "seed {}: cell {at} lost bits", case.seed);
        match made.derived {
            Derived::Exact(derivative) => {
                assert_eq!(
                    walked.slope[at],
                    derivative,
                    "seed {} cell {at} (plane {}): adjoint {}, dual {derivative}\n{}",
                    case.seed,
                    at / stride,
                    walked.slope[at],
                    case.text
                );
                tally.slopes += 1;
                if case.frozen[at / stride] && at / stride > 0 && derivative != 0 {
                    tally.looked += 1;
                }
            }
            Derived::Undefined => tally.undefined += 1,
            Derived::Overflowed => tally.skipped += 1,
        }
    }
}

pub const SPREAD_TRUTH: [u32; 2] = [3_000, 1_700];

fn word_at(corner: usize, plane: usize) -> String {
    format!("[\"at\",[\"at\",\"cells\",{corner}],{plane}]")
}

fn share(word: &str, by: &str) -> String {
    format!("[\"shr\",[\"mul\",[\"shr\",{word},5],{by}],12]")
}

fn conducted(corner: usize) -> String {
    let look = |at: [i64; 3]| format!("[\"look\",1,{},{},{},{corner}]", at[0], at[1], at[2]);
    let [first, second] = match corner % 3 {
        0 => [[-1, 0, 0], [1, 0, 0]],
        1 => [[0, -1, 0], [0, 1, 0]],
        _ => [[0, 0, -1], [1, 1, 1]],
    };
    format!(
        "[\"shr\",[\"mul\",[\"param\",0],[\"add\",{},{}]],12]",
        look(first),
        look(second)
    )
}

pub fn spread_law() -> String {
    let mut bindings = Vec::new();
    for corner in 0..8 {
        bindings.push(format!("[\"a{corner}\",{}]", word_at(corner, 0)));
    }
    for corner in 0..4 {
        let lower = corner + 4;
        bindings.push(format!(
            "[\"b{lower}\",[\"add\",\"a{lower}\",{}]]",
            share(&format!("\"a{corner}\""), &conducted(corner))
        ));
    }
    for corner in 0..4 {
        let lower = corner + 4;
        bindings.push(format!(
            "[\"b{corner}\",[\"sub\",\"a{corner}\",{}]]",
            share(&format!("\"b{lower}\""), "[\"param\",1]")
        ));
    }
    let out: Vec<String> = (0..8)
        .map(|corner| format!("[\"vec\",\"b{corner}\",{}]", word_at(corner, 1)))
        .collect();
    format!(
        "{{\"law\":1,\"name\":\"spread\",\"params\":2,\"channels\":2,\"halo\":1,\"reversible\":true,\"tally\":{{\"counts\":1,\"weighted\":false}},\"consts\":{{}},\"defs\":{{}},\"step\":[\"let\",[{}],[\"vec\",[\"vec\",{}],[\"vec\",0]]]}}",
        bindings.join(","),
        out.join(",")
    )
}

pub fn spread_start(grid: Grid3, seed: u64) -> Vec<u32> {
    let mut state = seed;
    let cells = grid.cells();
    (0..2 * cells)
        .map(|at| {
            if at < cells {
                (1 << 21) + u32::try_from(below(&mut state, 1 << 22)).unwrap()
            } else {
                (1 << 10) + u32::try_from(below(&mut state, 3 << 10)).unwrap()
            }
        })
        .collect()
}
