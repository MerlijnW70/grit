use crate::law::Law;
use crate::stencil::{self, Border, Sample, Stencil};
use crate::wire::{self, Json, counted, map, text};

use super::Failed;
use super::errors::{Code, Refusal};
use super::gradient::Told;

pub const FRAC: u32 = 16;

pub const FRAC_MOST: u32 = 30;

pub const POLY_MOST: u32 = 3;

pub const SCALE_MOST: u32 = 30;

pub const CONDITION_MOST: f64 = 1e10;

pub const SPREAD: f64 = 6.0;

pub const SWEEPS: u32 = 64;

pub const FORMAT: &str = "each read and written plane holds one signed fixed-point number a cell: the whole 32-bit word in two's complement with frac fractional bits, worth the word as a signed integer over 2^frac";

pub const ROUNDING: &str = "rnd_s(v) = floor(v / 2^s + 1/2) on a 32-bit word, round to nearest with ties toward +infinity, computed as ((v + 2^(s-1) + 2^31) >> s) - 2^(31-s) with a logical shift; a product term is rnd_frac of the 32-bit product, a cube rounds after each product; the output is y = ((D + sum_j C_j * T_j) >> k) - 2^(31-k) with D = B + 2^(k-1) + 2^31, every add and multiply wrapping on 32 bits, then clamped when clamp is given; inside range no step wraps";

pub const LINEAR: &str = "each write's word is the sum over the terms of a learned coefficient times the term, plus a learned bias, in fixed point: the terms are each read's word at every counted place, under poly 2 also the products of the centres of two reads and each centre times the sum of a read's other counted places, under poly 3 also the products of three centres; the coefficients are the exact least squares fit over every example cell, solved in f64 from integer normal equations, then rounded to k fractional bits";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Term {
    Place {
        read: usize,
        place: u32,
    },
    Pair {
        left: usize,
        right: usize,
    },
    Spread {
        centre: usize,
        around: usize,
    },
    Cube {
        left: usize,
        right: usize,
        third: usize,
    },
}

pub struct Plan<'held> {
    pub stencil: Stencil,
    pub reads: &'held [u32],
    pub writes: &'held [u32],
    pub mask: u32,
    pub poly: u32,
    pub frac: u32,
    pub clamp: Option<[i32; 2]>,
    pub range: Option<[i32; 2]>,
    pub border: Border,
    pub channels: u32,
    pub name: &'held str,
}

struct Square {
    side: usize,
    held: Vec<f64>,
}

impl Square {
    fn new(side: usize) -> Self {
        Self {
            side,
            held: vec![0.0; side * side],
        }
    }

    fn at(&self, row: usize, column: usize) -> f64 {
        self.held
            .get(row * self.side + column)
            .copied()
            .unwrap_or(0.0)
    }

    fn put(&mut self, row: usize, column: usize, value: f64) {
        if let Some(slot) = self.held.get_mut(row * self.side + column) {
            *slot = value;
        }
    }
}

fn places_of(plan: &Plan<'_>) -> Vec<u32> {
    (0..plan.stencil.positions())
        .filter(|place| (plan.mask >> place) & 1 == 1)
        .collect()
}

#[must_use]
pub fn terms(plan: &Plan<'_>) -> Vec<Term> {
    let places = places_of(plan);
    let reads = plan.reads.len();
    let mut out = Vec::new();
    for read in 0..reads {
        for place in &places {
            out.push(Term::Place {
                read,
                place: *place,
            });
        }
    }
    if plan.poly >= 2 {
        for left in 0..reads {
            for right in left..reads {
                out.push(Term::Pair { left, right });
            }
        }
        if places.len() > 1 {
            for centre in 0..reads {
                for around in 0..reads {
                    out.push(Term::Spread { centre, around });
                }
            }
        }
    }
    if plan.poly >= 3 {
        for left in 0..reads {
            for right in left..reads {
                for third in right..reads {
                    out.push(Term::Cube { left, right, third });
                }
            }
        }
    }
    out
}

#[must_use]
pub const fn rounded(frac: u32, value: u32) -> u32 {
    if frac == 0 {
        return value;
    }
    let lift = (1_u32 << (frac - 1)).wrapping_add(1 << 31);
    (value.wrapping_add(lift) >> frac).wrapping_sub(1 << (31 - frac))
}

struct Shape {
    places: Vec<u32>,
    centre: usize,
    terms: Vec<Term>,
}

impl Shape {
    fn of(plan: &Plan<'_>) -> Self {
        let places = places_of(plan);
        let centre_place = plan.stencil.place([0, 0, 0]);
        let centre = places
            .iter()
            .position(|one| *one == centre_place)
            .unwrap_or(0);
        Self {
            places,
            centre,
            terms: terms(plan),
        }
    }

    fn word(&self, words: &[u32], read: usize, at: usize) -> u32 {
        words
            .get(read * self.places.len() + at)
            .copied()
            .unwrap_or(0)
    }

    fn around(&self, words: &[u32], read: usize) -> u32 {
        (0..self.places.len())
            .filter(|at| *at != self.centre)
            .fold(0_u32, |sum, at| {
                sum.wrapping_add(self.word(words, read, at))
            })
    }

    fn valued(&self, frac: u32, words: &[u32], out: &mut Vec<u32>) {
        out.clear();
        let centre = |read: usize| self.word(words, read, self.centre);
        for (at, term) in self.terms.iter().enumerate() {
            let value = match *term {
                Term::Place { .. } => words.get(at).copied().unwrap_or(0),
                Term::Pair { left, right } => {
                    rounded(frac, centre(left).wrapping_mul(centre(right)))
                }
                Term::Spread {
                    centre: read,
                    around,
                } => rounded(frac, centre(read).wrapping_mul(self.around(words, around))),
                Term::Cube { left, right, third } => rounded(
                    frac,
                    rounded(frac, centre(left).wrapping_mul(centre(right)))
                        .wrapping_mul(centre(third)),
                ),
            };
            out.push(value);
        }
    }
}

fn spot(size: [u32; 3], at: [i64; 3], border: Border) -> Option<usize> {
    let mut spot = [0_u32; 3];
    for ((slot, value), side) in spot.iter_mut().zip(at).zip(size) {
        let side = i64::from(side.max(1));
        let inside = if (0..side).contains(&value) {
            value
        } else if border == Border::Wrapped {
            value.rem_euclid(side)
        } else {
            return None;
        };
        *slot = u32::try_from(inside).ok()?;
    }
    Some(stencil::index(size, spot))
}

fn gathered(plan: &Plan<'_>, shape: &Shape, sample: &Sample<'_>, at: [u32; 3], out: &mut Vec<u32>) {
    out.clear();
    for read in 0..plan.reads.len() {
        for place in &shape.places {
            let offset = plan.stencil.offset(*place);
            let wanted = [
                i64::from(sample.from[0]) + i64::from(at[0]) + offset[0],
                i64::from(sample.from[1]) + i64::from(at[1]) + offset[1],
                i64::from(sample.from[2]) + i64::from(at[2]) + offset[2],
            ];
            let word = spot(sample.size, wanted, plan.border)
                .and_then(|index| sample.inputs.get(read).and_then(|one| one.get(index)))
                .copied()
                .unwrap_or(0);
            out.push(word);
        }
    }
}

struct Visited<'held> {
    words: &'held [u32],
    values: &'held [u32],
    wanted: &'held [i32],
}

fn each_cell(
    plan: &Plan<'_>,
    shape: &Shape,
    samples: &[Sample<'_>],
    visit: &mut dyn FnMut(&Visited<'_>),
) {
    let mut words = Vec::new();
    let mut values = Vec::new();
    let mut wanted = Vec::with_capacity(plan.writes.len());
    for sample in samples {
        let [wide, deep, tall] = sample.inner;
        for z in 0..tall {
            for y in 0..deep {
                for x in 0..wide {
                    gathered(plan, shape, sample, [x, y, z], &mut words);
                    shape.valued(plan.frac, &words, &mut values);
                    wanted.clear();
                    let index = stencil::index(sample.inner, [x, y, z]);
                    for write in 0..plan.writes.len() {
                        let word = sample
                            .outputs
                            .get(write)
                            .and_then(|one| one.get(index))
                            .copied()
                            .unwrap_or(0);
                        wanted.push(word.cast_signed());
                    }
                    visit(&Visited {
                        words: &words,
                        values: &values,
                        wanted: &wanted,
                    });
                }
            }
        }
    }
}

struct Sums {
    cells: i128,
    terms: Vec<i128>,
    wanted: Vec<i128>,
    cross: Vec<i128>,
    toward: Vec<i128>,
    least: Vec<i32>,
    most: Vec<i32>,
}

fn summed(plan: &Plan<'_>, shape: &Shape, samples: &[Sample<'_>]) -> Sums {
    let count = shape.terms.len();
    let writes = plan.writes.len();
    let reads = plan.reads.len();
    let mut sums = Sums {
        cells: 0,
        terms: vec![0; count],
        wanted: vec![0; writes],
        cross: vec![0; count * count],
        toward: vec![0; count * writes],
        least: vec![i32::MAX; reads],
        most: vec![i32::MIN; reads],
    };
    let width = shape.places.len();
    each_cell(plan, shape, samples, &mut |cell| {
        let Visited {
            words,
            values,
            wanted,
        } = *cell;
        sums.cells += 1;
        for (at, word) in words.iter().enumerate() {
            let read = at / width.max(1);
            let signed = word.cast_signed();
            if let Some(least) = sums.least.get_mut(read) {
                *least = (*least).min(signed);
            }
            if let Some(most) = sums.most.get_mut(read) {
                *most = (*most).max(signed);
            }
        }
        for (row, left) in values.iter().enumerate() {
            let left = i128::from(left.cast_signed());
            if let Some(slot) = sums.terms.get_mut(row) {
                *slot += left;
            }
            for (column, right) in values.iter().enumerate().skip(row) {
                if let Some(slot) = sums.cross.get_mut(row * count + column) {
                    *slot += left * i128::from(right.cast_signed());
                }
            }
            for (write, one) in wanted.iter().enumerate() {
                if let Some(slot) = sums.toward.get_mut(write * count + row) {
                    *slot += left * i128::from(*one);
                }
            }
        }
        for (write, one) in wanted.iter().enumerate() {
            if let Some(slot) = sums.wanted.get_mut(write) {
                *slot += i128::from(*one);
            }
        }
    });
    sums
}

fn jacobi(matrix: &Square) -> Vec<f64> {
    let side = matrix.side;
    let mut held = Square {
        side,
        held: matrix.held.clone(),
    };
    for _ in 0..SWEEPS {
        let off: f64 = (0..side)
            .flat_map(|row| {
                (0..side)
                    .filter(move |column| *column != row)
                    .map(move |column| (row, column))
            })
            .map(|(row, column)| held.at(row, column).powi(2))
            .sum();
        let whole: f64 = held.held.iter().map(|one| one.powi(2)).sum();
        if off <= whole * 1e-30 {
            break;
        }
        for row in 0..side {
            for column in (row + 1)..side {
                let apq = held.at(row, column);
                if apq == 0.0 {
                    continue;
                }
                let app = held.at(row, row);
                let aqq = held.at(column, column);
                let theta = (aqq - app) / (2.0 * apq);
                let tangent = theta.signum() / (theta.abs() + theta.mul_add(theta, 1.0).sqrt());
                let cosine = 1.0 / tangent.mul_add(tangent, 1.0).sqrt();
                let sine = tangent * cosine;
                for other in 0..side {
                    let kp = held.at(other, row);
                    let kq = held.at(other, column);
                    held.put(other, row, cosine * kp - sine * kq);
                    held.put(other, column, sine * kp + cosine * kq);
                }
                for other in 0..side {
                    let pk = held.at(row, other);
                    let qk = held.at(column, other);
                    held.put(row, other, cosine * pk - sine * qk);
                    held.put(column, other, sine * pk + cosine * qk);
                }
            }
        }
    }
    (0..side).map(|at| held.at(at, at)).collect()
}

fn cholesky(matrix: &Square) -> Option<Square> {
    let side = matrix.side;
    let mut lower = Square::new(side);
    for row in 0..side {
        for column in 0..=row {
            let mut sum = matrix.at(row, column);
            for inner in 0..column {
                sum -= lower.at(row, inner) * lower.at(column, inner);
            }
            if row == column {
                if sum <= 0.0 || !sum.is_finite() {
                    return None;
                }
                lower.put(row, row, sum.sqrt());
            } else {
                lower.put(row, column, sum / lower.at(column, column));
            }
        }
    }
    Some(lower)
}

fn substituted(lower: &Square, right: &[f64]) -> Vec<f64> {
    let side = lower.side;
    let mut middle = vec![0.0; side];
    for row in 0..side {
        let mut sum = right.get(row).copied().unwrap_or(0.0);
        for inner in 0..row {
            sum -= lower.at(row, inner) * middle.get(inner).copied().unwrap_or(0.0);
        }
        if let Some(slot) = middle.get_mut(row) {
            *slot = sum / lower.at(row, row);
        }
    }
    let mut out = vec![0.0; side];
    for row in (0..side).rev() {
        let mut sum = middle.get(row).copied().unwrap_or(0.0);
        for inner in (row + 1)..side {
            sum -= lower.at(inner, row) * out.get(inner).copied().unwrap_or(0.0);
        }
        if let Some(slot) = out.get_mut(row) {
            *slot = sum / lower.at(row, row);
        }
    }
    out
}

fn scientific(value: f64) -> Json {
    text(&format!("{value:.9e}"))
}

#[must_use]
pub fn decimal(value: i128, shift: u32) -> String {
    let negative = value < 0;
    let whole = value.unsigned_abs();
    let integer = whole >> shift;
    let mut rest = whole & ((1_u128 << shift) - 1);
    let mut digits = String::new();
    while rest != 0 {
        rest *= 10;
        let digit = rest >> shift;
        rest &= (1_u128 << shift) - 1;
        digits.push(char::from(b'0' + u8::try_from(digit).unwrap_or(0)));
    }
    let sign = if negative && whole != 0 { "-" } else { "" };
    if digits.is_empty() {
        format!("{sign}{integer}")
    } else {
        format!("{sign}{integer}.{digits}")
    }
}

#[derive(Clone, Debug)]
pub struct Fixed {
    pub scale: u32,
    pub coefficients: Vec<i32>,
    pub bias: i32,
}

#[must_use]
pub const fn offset(scale: u32) -> u32 {
    1 << (31 - scale)
}

#[must_use]
pub const fn constant(bias: i32, scale: u32) -> u32 {
    bias.cast_unsigned()
        .wrapping_add(1 << (scale - 1))
        .wrapping_add(1 << 31)
}

pub(super) fn clamp_bounds(clamp: [i32; 2], scale: u32) -> [u32; 2] {
    let top = (1_i64 << (32 - scale)) - 1;
    clamp.map(|one| {
        let moved = (i64::from(one) + i64::from(offset(scale))).clamp(0, top);
        u32::try_from(moved).unwrap_or(0)
    })
}

#[must_use]
pub fn law_word(fixed: &Fixed, clamp: Option<[i32; 2]>, values: &[u32]) -> u32 {
    let total = fixed
        .coefficients
        .iter()
        .zip(values)
        .fold(constant(fixed.bias, fixed.scale), |sum, (one, value)| {
            sum.wrapping_add(one.cast_unsigned().wrapping_mul(*value))
        });
    let mut moved = total >> fixed.scale;
    if let Some(bounds) = clamp {
        let [least, most] = clamp_bounds(bounds, fixed.scale);
        if moved < least {
            moved = least;
        }
        if moved > most {
            moved = most;
        }
    }
    moved.wrapping_sub(offset(fixed.scale))
}

fn term_bounds(shape: &Shape, frac: u32, reads: &[u64]) -> Result<Vec<u128>, usize> {
    let half = if frac == 0 { 0 } else { 1_u128 << (frac - 1) };
    let room = 1_u128 << 31;
    let product = |left: u128, right: u128, at: usize| -> Result<u128, usize> {
        let reach = left.saturating_mul(right);
        if reach.saturating_add(half) >= room {
            return Err(at);
        }
        Ok(if frac == 0 {
            reach
        } else {
            (reach >> frac) + 1
        })
    };
    let read = |at: usize| u128::from(reads.get(at).copied().unwrap_or(0));
    let others = u128::try_from(shape.places.len().saturating_sub(1)).unwrap_or(u128::MAX);
    shape
        .terms
        .iter()
        .enumerate()
        .map(|(at, term)| match *term {
            Term::Place { read: one, .. } => Ok(read(one)),
            Term::Pair { left, right } => product(read(left), read(right), at),
            Term::Spread { centre, around } => {
                let sum = others.saturating_mul(read(around));
                if sum >= room {
                    return Err(at);
                }
                product(read(centre), sum, at)
            }
            Term::Cube { left, right, third } => {
                let inner = product(read(left), read(right), at)?;
                product(inner, read(third), at)
            }
        })
        .collect()
}

fn quantised(value: f64, scale: u32) -> Option<i32> {
    let moved = (value * f64::from(1_u32 << scale)).round();
    if !moved.is_finite() || moved.abs() >= 2_147_483_648.0 {
        return None;
    }
    let whole = moved as i64;
    i32::try_from(whole).ok()
}

pub(super) fn scaled(bounds: &[u128], fit: &[f64], bias: f64) -> Option<Fixed> {
    for scale in (1..=SCALE_MOST).rev() {
        let Some(coefficients) = fit
            .iter()
            .map(|one| quantised(*one, scale))
            .collect::<Option<Vec<i32>>>()
        else {
            continue;
        };
        let Some(fixed_bias) = quantised(bias, scale) else {
            continue;
        };
        let reach = coefficients
            .iter()
            .zip(bounds)
            .fold(0_u128, |sum, (one, bound)| {
                sum.saturating_add(u128::from(one.unsigned_abs()).saturating_mul(*bound))
            })
            .saturating_add(u128::from(fixed_bias.unsigned_abs()))
            .saturating_add(1_u128 << (scale - 1));
        if reach < 1_u128 << 31 {
            return Some(Fixed {
                scale,
                coefficients,
                bias: fixed_bias,
            });
        }
    }
    None
}

fn term_shown(plan: &Plan<'_>, term: Term) -> Json {
    let read = |one: usize| counted(u32::try_from(one).unwrap_or(u32::MAX));
    match term {
        Term::Place { read: one, place } => {
            let [dx, dy, dz] = plan.stencil.offset(place);
            map(vec![
                ("term", text("place")),
                ("read", read(one)),
                ("place", counted(place)),
                (
                    "at",
                    Json::List(vec![Json::Int(dx), Json::Int(dy), Json::Int(dz)]),
                ),
            ])
        }
        Term::Pair { left, right } => map(vec![
            ("term", text("product")),
            ("reads", Json::List(vec![read(left), read(right)])),
        ]),
        Term::Spread { centre, around } => map(vec![
            ("term", text("centre_times_others")),
            ("reads", Json::List(vec![read(centre), read(around)])),
        ]),
        Term::Cube { left, right, third } => map(vec![
            ("term", text("cube")),
            (
                "reads",
                Json::List(vec![read(left), read(right), read(third)]),
            ),
        ]),
    }
}

fn term_name(term: Term) -> String {
    match term {
        Term::Place { read, place } => format!("read {read} at place {place}"),
        Term::Pair { left, right } => {
            format!("the product of the centres of reads {left} and {right}")
        }
        Term::Spread { centre, around } => {
            format!("the centre of read {centre} times the other places of read {around}")
        }
        Term::Cube { left, right, third } => {
            format!("the product of the centres of reads {left}, {right} and {third}")
        }
    }
}

fn op(name: &str, args: Vec<Json>) -> Json {
    let mut held = vec![text(name)];
    held.extend(args);
    Json::List(held)
}

fn named(name: &str) -> Json {
    text(name)
}

fn word(value: u32) -> Json {
    Json::Int(i64::from(value))
}

fn own(plane: u32, corner: &str) -> Json {
    op(
        "at",
        vec![op("at", vec![named("cells"), named(corner)]), word(plane)],
    )
}

fn rounding(frac: u32, value: Json) -> Json {
    if frac == 0 {
        return value;
    }
    let lift = (1_u32 << (frac - 1)).wrapping_add(1 << 31);
    op(
        "sub",
        vec![
            op("shr", vec![op("add", vec![value, word(lift)]), word(frac)]),
            word(1 << (31 - frac)),
        ],
    )
}

struct Emitted {
    bound: Vec<Json>,
}

impl Emitted {
    fn bind(&mut self, name: &str, value: Json) {
        self.bound.push(Json::List(vec![text(name), value]));
    }
}

fn corner(plan: &Plan<'_>, shape: &Shape, fixed: &[Fixed]) -> Json {
    let mut emitted = Emitted { bound: Vec::new() };
    let centre_place = plan.stencil.place([0, 0, 0]);
    for (read, plane) in plan.reads.iter().enumerate() {
        for place in &shape.places {
            let value = if *place == centre_place {
                own(*plane, "c")
            } else {
                let [dx, dy, dz] = plan.stencil.offset(*place);
                op(
                    "look",
                    vec![
                        word(*plane),
                        Json::Int(dx),
                        Json::Int(dy),
                        Json::Int(dz),
                        named("c"),
                    ],
                )
            };
            emitted.bind(&format!("x{read}_{place}"), value);
        }
    }
    let centre = |read: usize| named(&format!("x{read}_{centre_place}"));
    let frac = plan.frac;
    let spreads: Vec<usize> = shape
        .terms
        .iter()
        .filter_map(|term| match term {
            Term::Spread { around, .. } => Some(*around),
            _ => None,
        })
        .collect();
    let mut sums: Vec<(usize, String)> = Vec::new();
    for read in 0..plan.reads.len() {
        if !spreads.contains(&read) {
            continue;
        }
        let mut last: Option<String> = None;
        for place in shape.places.iter().filter(|one| **one != centre_place) {
            let one = named(&format!("x{read}_{place}"));
            let value = match &last {
                None => one,
                Some(before) => op("add", vec![named(before), one]),
            };
            let name = format!("n{read}_{place}");
            emitted.bind(&name, value);
            last = Some(name);
        }
        if let Some(name) = last {
            sums.push((read, name));
        }
    }
    let sum_of = |read: usize| {
        sums.iter()
            .find(|(one, _)| *one == read)
            .map_or_else(|| word(0), |(_, name)| named(name))
    };
    let mut names = Vec::with_capacity(shape.terms.len());
    for (at, term) in shape.terms.iter().enumerate() {
        let name = match *term {
            Term::Place { read, place } => format!("x{read}_{place}"),
            Term::Pair { left, right } => {
                let name = format!("p{left}_{right}");
                emitted.bind(
                    &name,
                    rounding(frac, op("mul", vec![centre(left), centre(right)])),
                );
                name
            }
            Term::Spread {
                centre: read,
                around,
            } => {
                let name = format!("t{at}");
                emitted.bind(
                    &name,
                    rounding(frac, op("mul", vec![centre(read), sum_of(around)])),
                );
                name
            }
            Term::Cube { left, right, third } => {
                let name = format!("t{at}");
                emitted.bind(
                    &name,
                    rounding(
                        frac,
                        op(
                            "mul",
                            vec![named(&format!("p{left}_{right}")), centre(third)],
                        ),
                    ),
                );
                name
            }
        };
        names.push(name);
    }
    for (write, one) in fixed.iter().enumerate() {
        let mut total = format!("a{write}");
        emitted.bind(&total, word(constant(one.bias, one.scale)));
        for (at, (coefficient, name)) in one.coefficients.iter().zip(&names).enumerate() {
            if *coefficient == 0 {
                continue;
            }
            let next = format!("a{write}_{at}");
            emitted.bind(
                &next,
                op(
                    "add",
                    vec![
                        named(&total),
                        op("mul", vec![word(coefficient.cast_unsigned()), named(name)]),
                    ],
                ),
            );
            total = next;
        }
        let mut moved = op("shr", vec![named(&total), word(one.scale)]);
        if let Some(bounds) = plan.clamp {
            let [least, most] = clamp_bounds(bounds, one.scale);
            emitted.bind(&format!("z{write}"), moved);
            let low = format!("l{write}");
            emitted.bind(
                &low,
                op(
                    "sel",
                    vec![
                        op("lt", vec![named(&format!("z{write}")), word(least)]),
                        word(least),
                        named(&format!("z{write}")),
                    ],
                ),
            );
            moved = op(
                "sel",
                vec![
                    op("gt", vec![named(&low), word(most)]),
                    word(most),
                    named(&low),
                ],
            );
        }
        emitted.bind(
            &format!("y{write}"),
            op("sub", vec![moved, word(offset(one.scale))]),
        );
    }
    let planes = (0..plan.channels)
        .map(
            |plane| match plan.writes.iter().position(|one| *one == plane) {
                Some(write) => named(&format!("y{write}")),
                None => own(plane, "c"),
            },
        )
        .collect();
    op("let", vec![Json::List(emitted.bound), op("vec", planes)])
}

fn law_text(plan: &Plan<'_>, shape: &Shape, fixed: &[Fixed]) -> String {
    let corners = op(
        "map",
        vec![
            named("c"),
            op("range", vec![word(8)]),
            corner(plan, shape, fixed),
        ],
    );
    let mut changed: Option<Json> = None;
    for plane in plan.writes {
        let one = op(
            "ne",
            vec![
                own(*plane, "k"),
                op(
                    "at",
                    vec![op("at", vec![named("out"), named("k")]), word(*plane)],
                ),
            ],
        );
        changed = Some(match changed {
            None => one,
            Some(before) => op("either", vec![before, one]),
        });
    }
    let count = op(
        "count",
        vec![op(
            "map",
            vec![
                named("k"),
                op("range", vec![word(8)]),
                changed.unwrap_or_else(|| op("ne", vec![word(0), word(0)])),
            ],
        )],
    );
    let step = op(
        "let",
        vec![
            Json::List(vec![Json::List(vec![text("out"), corners])]),
            op("vec", vec![named("out"), op("vec", vec![count])]),
        ],
    );
    let centre_place = plan.stencil.place([0, 0, 0]);
    let halo = u32::from(shape.places.iter().any(|one| *one != centre_place));
    wire::written(&map(vec![
        ("law", word(crate::law::limits::VERSION)),
        ("name", text(plan.name)),
        ("params", word(0)),
        ("channels", word(plan.channels)),
        ("halo", word(halo)),
        (
            "tally",
            map(vec![("counts", word(1)), ("weighted", Json::Bool(false))]),
        ),
        ("consts", Json::Map(Vec::new())),
        ("defs", Json::Map(Vec::new())),
        ("step", step),
    ]))
}

fn ill(condition: f64, why: String) -> Failed {
    let shown = if condition.is_finite() {
        format!("{condition:.3e}")
    } else {
        String::from("infinite")
    };
    Box::new(Told(
        Refusal::new(Code::IllConditioned, "$.examples", why)
            .limited(text(&format!("{CONDITION_MOST:.0e}"))),
        vec![("condition", text(&shown))],
    ))
}

fn range_refused(
    plan: &Plan<'_>,
    shape: &Shape,
    reads: &[u64],
    fit: &[Vec<f64>],
    biases: &[f64],
) -> Failed {
    let largest = (0..plan.frac).rev().find(|frac| {
        let down = plan.frac - frac;
        let reads: Vec<u64> = reads
            .iter()
            .map(|one| one.div_ceil(1_u64 << down))
            .collect();
        term_bounds(shape, *frac, &reads).is_ok_and(|bounds| {
            fit.iter()
                .zip(biases)
                .all(|(one, bias)| scaled(&bounds, one, bias / f64::from(1_u32 << down)).is_some())
        })
    });
    let message = format!(
        "at frac {} the terms over the examples' range need more than the 32 bits one word holds for the sum of coefficients times terms; lower frac, narrow range or drop poly",
        plan.frac
    );
    match largest {
        Some(frac) => {
            Box::new(Refusal::new(Code::FixedRange, "$.frac", message).limited(counted(frac)))
        }
        None => Box::new(Refusal::new(Code::FixedRange, "$.frac", message)),
    }
}

fn clamp_refused(plan: &Plan<'_>, fixed: &[Fixed]) -> Option<Failed> {
    let [least, most] = plan.clamp?;
    let scale = fixed.iter().map(|one| one.scale).max()?;
    let reach = i64::from(offset(scale));
    let (low, high) = (-reach, reach - 1);
    if i64::from(least) <= high && i64::from(most) >= low {
        return None;
    }
    Some(Box::new(
        Refusal::new(
            Code::ClampRange,
            "$.clamp",
            format!(
                "clamp [{least}, {most}] holds no word the law can write: at coefficient_frac {scale} every written word is in [{low}, {high}], so the clamp would saturate at its edge instead of holding; give a clamp that meets it, or a wider range"
            ),
        )
        .limited(Json::List(vec![Json::Int(low), Json::Int(high)])),
    ))
}

struct Fit {
    coefficients: Vec<f64>,
    bias: f64,
}

pub fn learned(
    plan: &Plan<'_>,
    samples: &[Sample<'_>],
) -> Result<Vec<(&'static str, Json)>, Failed> {
    let shape = Shape::of(plan);
    let count = shape.terms.len();
    let sums = summed(plan, &shape, samples);
    let cells = sums.cells;
    let cross = |row: usize, column: usize| {
        let (row, column) = if row <= column {
            (row, column)
        } else {
            (column, row)
        };
        sums.cross.get(row * count + column).copied().unwrap_or(0)
    };
    let term_sum = |at: usize| sums.terms.get(at).copied().unwrap_or(0);
    let centred =
        |row: usize, column: usize| cells * cross(row, column) - term_sum(row) * term_sum(column);
    for (at, term) in shape.terms.iter().enumerate() {
        if centred(at, at) <= 0 {
            return Err(ill(
                f64::INFINITY,
                format!(
                    "the term {} is the same on every example cell, so its coefficient and the bias are not told apart; give examples where it varies or leave it out",
                    term_name(*term)
                ),
            ));
        }
    }
    let scales: Vec<f64> = (0..count)
        .map(|at| (centred(at, at) as f64).sqrt())
        .collect();
    let scale = |at: usize| scales.get(at).copied().unwrap_or(1.0);
    let mut matrix = Square::new(count);
    for row in 0..count {
        for column in 0..count {
            matrix.put(
                row,
                column,
                centred(row, column) as f64 / (scale(row) * scale(column)),
            );
        }
    }
    let eigen = jacobi(&matrix);
    let least = eigen.iter().copied().fold(f64::INFINITY, f64::min);
    let most = eigen.iter().copied().fold(0.0_f64, f64::max);
    let condition = if least > 0.0 {
        most / least
    } else {
        f64::INFINITY
    };
    if condition.is_nan() || condition > CONDITION_MOST {
        return Err(ill(
            condition,
            format!(
                "the terms over the examples are close to dependent: the condition of their centred, scaled normal matrix is {}, past {CONDITION_MOST:.0e}, so the coefficients are not determined; give examples that vary more, or use fewer reads, places or poly terms",
                if condition.is_finite() {
                    format!("{condition:.3e}")
                } else {
                    String::from("infinite")
                }
            ),
        ));
    }
    let Some(lower) = cholesky(&matrix) else {
        return Err(ill(
            condition,
            "the centred, scaled normal matrix is not positive definite in f64".to_owned(),
        ));
    };
    let inverse: Vec<Vec<f64>> = (0..count)
        .map(|column| {
            let unit: Vec<f64> = (0..count)
                .map(|row| f64::from(u8::from(row == column)))
                .collect();
            substituted(&lower, &unit)
        })
        .collect();
    let inverse_at = |row: usize, column: usize| {
        inverse
            .get(column)
            .and_then(|one| one.get(row))
            .copied()
            .unwrap_or(0.0)
            / (scale(row) * scale(column))
    };
    let n = cells as f64;
    let mean = |at: usize| term_sum(at) as f64 / n;
    let fits: Vec<Fit> = (0..plan.writes.len())
        .map(|write| {
            let wanted = sums.wanted.get(write).copied().unwrap_or(0);
            let right: Vec<f64> = (0..count)
                .map(|at| {
                    let toward = sums.toward.get(write * count + at).copied().unwrap_or(0);
                    (cells * toward - term_sum(at) * wanted) as f64 / scale(at)
                })
                .collect();
            let coefficients: Vec<f64> = substituted(&lower, &right)
                .iter()
                .enumerate()
                .map(|(at, one)| one / scale(at))
                .collect();
            let bias = wanted as f64 / n
                - coefficients
                    .iter()
                    .enumerate()
                    .map(|(at, one)| one * mean(at))
                    .sum::<f64>();
            Fit { coefficients, bias }
        })
        .collect();
    if fits
        .iter()
        .any(|one| !one.bias.is_finite() || one.coefficients.iter().any(|value| !value.is_finite()))
    {
        return Err(ill(
            condition,
            "the f64 solve gave a value that is not finite".to_owned(),
        ));
    }
    let reads: Vec<u64> = match plan.range {
        Some([least, most]) => {
            vec![u64::from(least.unsigned_abs().max(most.unsigned_abs())); plan.reads.len()]
        }
        None => sums
            .least
            .iter()
            .zip(&sums.most)
            .map(|(least, most)| u64::from(least.unsigned_abs().max(most.unsigned_abs())))
            .collect(),
    };
    let coefficient_lists: Vec<Vec<f64>> =
        fits.iter().map(|one| one.coefficients.clone()).collect();
    let biases: Vec<f64> = fits.iter().map(|one| one.bias).collect();
    let Ok(bounds) = term_bounds(&shape, plan.frac, &reads) else {
        return Err(range_refused(
            plan,
            &shape,
            &reads,
            &coefficient_lists,
            &biases,
        ));
    };
    let Some(fixed) = fits
        .iter()
        .map(|one| scaled(&bounds, &one.coefficients, one.bias))
        .collect::<Option<Vec<Fixed>>>()
    else {
        return Err(range_refused(
            plan,
            &shape,
            &reads,
            &coefficient_lists,
            &biases,
        ));
    };
    if let Some(why) = clamp_refused(plan, &fixed) {
        return Err(why);
    }
    let writes = plan.writes.len();
    let mut squares = vec![0.0_f64; writes];
    let mut worst = vec![0_u64; writes];
    let mut errors = vec![0_u128; writes];
    each_cell(plan, &shape, samples, &mut |cell| {
        let Visited { values, wanted, .. } = *cell;
        for (write, one) in wanted.iter().enumerate() {
            if let (Some(fit), Some(fixed), Some(square), Some(far), Some(error)) = (
                fits.get(write),
                fixed.get(write),
                squares.get_mut(write),
                worst.get_mut(write),
                errors.get_mut(write),
            ) {
                let guess = fit
                    .coefficients
                    .iter()
                    .zip(values)
                    .map(|(coefficient, value)| coefficient * f64::from(value.cast_signed()))
                    .sum::<f64>()
                    + fit.bias;
                let missed = f64::from(*one) - guess;
                *square += missed * missed;
                let answered = law_word(fixed, plan.clamp, values).cast_signed();
                let off = (i64::from(answered) - i64::from(*one)).unsigned_abs();
                *far = (*far).max(off);
                *error += u128::from(off) * u128::from(off);
            }
        }
    });
    let freedom = (cells - i128::try_from(count).unwrap_or(0) - 1).max(1) as f64;
    let unit = f64::from(1_u32 << plan.frac);
    let mut coefficients_shown = Vec::with_capacity(writes);
    let mut biases_shown = Vec::with_capacity(writes);
    let mut residuals = Vec::with_capacity(writes);
    let mut symmetric_shown = Vec::with_capacity(writes);
    let mut invariant = true;
    let moves = plan.stencil.symmetries();
    let places = &shape.places;
    let closed = moves.iter().all(|moved| {
        places
            .iter()
            .all(|place| places.contains(moved.get(*place as usize).unwrap_or(&u32::MAX)))
    });
    for (write, (fit, one)) in fits.iter().zip(&fixed).enumerate() {
        let variance = squares.get(write).copied().unwrap_or(0.0) / freedom;
        let spread_of = |at: usize| (variance * n * inverse_at(at, at)).max(0.0).sqrt();
        coefficients_shown.push(Json::List(
            fit.coefficients
                .iter()
                .zip(&one.coefficients)
                .enumerate()
                .map(|(at, (value, constant))| {
                    map(vec![
                        ("value", text(&decimal(i128::from(*constant), one.scale))),
                        ("fixed", Json::Int(i64::from(*constant))),
                        ("fit", scientific(*value)),
                        ("stderr", scientific(spread_of(at))),
                    ])
                })
                .collect(),
        ));
        let means: Vec<f64> = (0..count).map(mean).collect();
        let bias_spread = {
            let quadratic: f64 = (0..count)
                .flat_map(|row| (0..count).map(move |column| (row, column)))
                .map(|(row, column)| {
                    means.get(row).copied().unwrap_or(0.0)
                        * means.get(column).copied().unwrap_or(0.0)
                        * inverse_at(row, column)
                })
                .sum();
            (variance / n + variance * n * quadratic).max(0.0).sqrt()
        };
        biases_shown.push(map(vec![
            (
                "value",
                text(&decimal(i128::from(one.bias), one.scale + plan.frac)),
            ),
            ("fixed", Json::Int(i64::from(one.bias))),
            ("fit", scientific(fit.bias / unit)),
            ("stderr", scientific(bias_spread / unit)),
        ]));
        let sum_squares = errors.get(write).copied().unwrap_or(0);
        let rms = (sum_squares as f64 / n).sqrt();
        let far = worst.get(write).copied().unwrap_or(0);
        residuals.push(map(vec![
            ("max", Json::Int(i64::try_from(far).unwrap_or(i64::MAX))),
            ("rms", scientific(rms)),
            ("sum_squares", text(&sum_squares.to_string())),
            ("max_real", scientific(far as f64 / unit)),
            ("rms_real", scientific(rms / unit)),
            (
                "fit_rms_real",
                scientific((squares.get(write).copied().unwrap_or(0.0) / n).sqrt() / unit),
            ),
        ]));
        let floor = 1.0 / f64::from(1_u32 << one.scale);
        let mut held: Option<(f64, f64, f64, usize, u32, u32)> = None;
        if closed {
            for read in 0..plan.reads.len() {
                for moved in &moves {
                    for (from, place) in places.iter().enumerate() {
                        let target = moved.get(*place as usize).copied().unwrap_or(*place);
                        let Some(to) = places.iter().position(|one| *one == target) else {
                            continue;
                        };
                        let left = read * places.len() + from;
                        let right = read * places.len() + to;
                        let gap = (fit.coefficients.get(left).copied().unwrap_or(0.0)
                            - fit.coefficients.get(right).copied().unwrap_or(0.0))
                        .abs();
                        let wide = variance
                            * n
                            * (inverse_at(left, left) + inverse_at(right, right)
                                - 2.0 * inverse_at(left, right));
                        let allowed = floor.max(SPREAD * wide.max(0.0).sqrt());
                        let ratio = gap / allowed;
                        if held.is_none_or(|one| ratio > one.0) {
                            held = Some((ratio, gap, allowed, read, *place, target));
                        }
                    }
                }
            }
        }
        let mut fields = Vec::new();
        match held {
            Some((ratio, gap, allowed, read, place, target)) => {
                let holds = ratio <= 1.0;
                invariant &= holds;
                fields.push(("invariant", Json::Bool(holds)));
                fields.push(("deviation", scientific(gap)));
                fields.push(("tolerance", scientific(allowed)));
                fields.push(("read", counted(u32::try_from(read).unwrap_or(u32::MAX))));
                fields.push(("places", Json::List(vec![counted(place), counted(target)])));
            }
            None => {
                invariant = false;
                fields.push(("invariant", Json::Bool(false)));
            }
        }
        symmetric_shown.push(map(fields));
    }
    let mut symmetry = vec![
        (
            "group",
            text(match plan.stencil {
                Stencil::Square => "D4",
                Stencil::Cube => "Oh",
            }),
        ),
        (
            "order",
            counted(u32::try_from(moves.len()).unwrap_or(u32::MAX)),
        ),
        ("invariant", Json::Bool(invariant && closed)),
        (
            "means",
            text(
                "the place coefficients of each read are invariant when every place and its image under every move of the group differ by at most max(2^-k, 6 standard errors of the difference)",
            ),
        ),
    ];
    if !closed {
        symmetry.push((
            "why",
            text("the counted places are not closed under the group"),
        ));
    }
    symmetry.push(("writes", Json::List(symmetric_shown)));
    let written = law_text(plan, &shape, &fixed);
    let law = Law::compile(&written).map_err(|why| {
        Box::new(Told(
            Refusal::new(
                Code::LawRefused,
                "$",
                format!("the law made from the fit does not compile: {why}"),
            ),
            vec![("law", why.json())],
        )) as Failed
    })?;
    let class = super::law::background_class(&law, &[]);
    let mut answer = vec![
        ("law", text(&written)),
        ("hash", text(&law.hex())),
        ("name", text(law.name())),
        ("channels", counted(law.channels())),
        ("halo", counted(law.halo())),
        ("quiet", text(class.name())),
    ];
    if let Some(why) = class.why() {
        answer.push(("quiet_why", text(&why)));
    }
    let range_shown = sums
        .least
        .iter()
        .zip(&sums.most)
        .map(|(least, most)| {
            let [least, most] = plan.range.unwrap_or([*least, *most]);
            Json::List(vec![
                Json::Int(i64::from(least)),
                Json::Int(i64::from(most)),
            ])
        })
        .collect();
    let mut fixed_shown = vec![
        ("frac", counted(plan.frac)),
        (
            "coefficient_frac",
            Json::List(fixed.iter().map(|one| counted(one.scale)).collect()),
        ),
        ("format", text(FORMAT)),
        ("rounding", text(ROUNDING)),
        ("range", Json::List(range_shown)),
    ];
    if let Some([least, most]) = plan.clamp {
        fixed_shown.push((
            "clamp",
            Json::List(vec![
                Json::Int(i64::from(least)),
                Json::Int(i64::from(most)),
            ]),
        ));
    }
    answer.extend([
        (
            "stepping",
            text(if class.proved() { "listed" } else { "whole" }),
        ),
        (
            "nodes",
            counted(u32::try_from(law.nodes()).unwrap_or(u32::MAX)),
        ),
        ("rule", text("linear")),
        ("means", text(LINEAR)),
        ("poly", counted(plan.poly)),
        ("fixed", map(fixed_shown)),
        (
            "terms",
            Json::List(
                shape
                    .terms
                    .iter()
                    .map(|one| term_shown(plan, *one))
                    .collect(),
            ),
        ),
        ("coefficients", Json::List(coefficients_shown)),
        ("bias", Json::List(biases_shown)),
        ("residual", Json::List(residuals)),
        ("condition", scientific(condition)),
        ("symmetry", map(symmetry)),
    ]);
    Ok(answer)
}
