use super::{
    Axis, CORNERS, Corner, Declared, Oct, Octet, OneStep, Ruled, Tally, noise_by, param_by,
};
use crate::ops::{Cpu, Flag, Helpers, Ops};

pub const ALIVE: u32 = 1;

pub const ENERGY_SHIFT: u32 = 1;
pub const ENERGY_BITS: u32 = 8;
pub const AGE_SHIFT: u32 = 9;
pub const AGE_BITS: u32 = 6;
pub const GENOME_SHIFT: u32 = 16;
pub const GENOME_BITS: u32 = 16;

pub const FULL: u32 = (1 << ENERGY_BITS) - 1;
pub const OLDEST: u32 = (1 << AGE_BITS) - 1;
pub const GENOME_MASK: u32 = (1 << GENOME_BITS) - 1;
const ENERGY_MASK: u32 = FULL << ENERGY_SHIFT;
const AGE_MASK: u32 = OLDEST << AGE_SHIFT;

pub const THRESHOLD_GENE: u32 = 0;
pub const LIFESPAN_GENE: u32 = 2;
pub const GUARD_GENE: u32 = 4;
pub const MARKER_GENE: u32 = 6;
pub const GENE_BITS: u32 = 2;
pub const MARKER_BITS: u32 = GENOME_BITS - MARKER_GENE;

pub const THRESHOLD_LEAST: u32 = 64;
pub const THRESHOLD_STEP: u32 = 32;
pub const AGE_LEAST: u32 = 24;
pub const AGE_STEP: u32 = 12;

pub const FEED: usize = 0;
pub const UPKEEP: usize = 1;
pub const LIFE_COST: usize = 2;
pub const GUARD_COST: usize = 3;
pub const BIRTH_COST: usize = 4;
pub const HAZARD: usize = 5;
pub const MUTATION: usize = 6;
pub const SEED: usize = 7;

pub const PARAMS: usize = 8;

pub const DEFAULTS: [u32; PARAMS] = [2, 2, 1, 1, 8, 512, 256, 0];

pub const COST_MOST: u32 = 255;

pub const CHANCES: u32 = 1 << 16;

pub const CHANGED: usize = 0;
pub const LIVING: usize = 1;
pub const BIRTHS: usize = 2;
pub const DEATHS: usize = 3;
pub const MUTATIONS: usize = 4;

pub const COUNTS: usize = 5;

const ROLL_SALT: u32 = 0x632B_E5AB;
const CHILD_SALT: u32 = 0x85EB_CA77;
const BIT_SALT: u32 = 0xC2B2_AE3D;

const MIX_FIRST: u32 = 0x7FEB_352D;
const MIX_SECOND: u32 = 0x846C_A68B;

const ORDER: [Axis; 3] = [Axis::X, Axis::Y, Axis::Z];

#[must_use]
pub const fn kind_of(word: u32) -> u32 {
    word & ALIVE
}

#[must_use]
pub const fn alive_of(word: u32) -> bool {
    word & ALIVE != 0
}

#[must_use]
pub const fn energy_of(word: u32) -> u32 {
    (word >> ENERGY_SHIFT) & FULL
}

#[must_use]
pub const fn age_of(word: u32) -> u32 {
    (word >> AGE_SHIFT) & OLDEST
}

#[must_use]
pub const fn genome_of(word: u32) -> u32 {
    word >> GENOME_SHIFT
}

#[must_use]
pub const fn gene(genome: u32, at: u32) -> u32 {
    (genome >> at) & ((1 << GENE_BITS) - 1)
}

#[must_use]
pub const fn genome(threshold: u32, lifespan: u32, guard: u32, marker: u32) -> u32 {
    let mask = (1 << GENE_BITS) - 1;
    ((threshold & mask) << THRESHOLD_GENE)
        | ((lifespan & mask) << LIFESPAN_GENE)
        | ((guard & mask) << GUARD_GENE)
        | ((marker << MARKER_GENE) & GENOME_MASK)
}

#[must_use]
pub const fn cell(genome: u32, energy: u32, age: u32) -> u32 {
    ((genome & GENOME_MASK) << GENOME_SHIFT)
        | ((age & OLDEST) << AGE_SHIFT)
        | ((energy & FULL) << ENERGY_SHIFT)
        | ALIVE
}

#[must_use]
pub const fn threshold_of(genome: u32) -> u32 {
    THRESHOLD_LEAST + THRESHOLD_STEP * gene(genome, THRESHOLD_GENE)
}

#[must_use]
pub const fn lifespan_of(genome: u32) -> u32 {
    AGE_LEAST + AGE_STEP * gene(genome, LIFESPAN_GENE)
}

#[must_use]
pub fn upkeep_of(genome: u32, params: &[u32]) -> u32 {
    let held: Vec<u32> = (0..PARAMS)
        .map(|which| super::param(params, which))
        .collect();
    upkeep_by(&mut Cpu, genome, &held)
}

fn smaller<O: Ops>(ops: &mut O, value: O::Word, cap: u32) -> O::Word {
    let most = ops.constant(cap);
    let over = ops.greater(value, most);
    ops.select(over, most, value)
}

fn param_capped<O: Ops>(ops: &mut O, params: &[O::Word], which: usize) -> O::Word {
    let held = param_by(ops, params, which);
    smaller(ops, held, COST_MOST)
}

fn upkeep_by<O: Ops>(ops: &mut O, genome: O::Word, params: &[O::Word]) -> O::Word {
    let base = param_capped(ops, params, UPKEEP);
    let life = param_capped(ops, params, LIFE_COST);
    let guard = param_capped(ops, params, GUARD_COST);
    let lifespan = ops.field(genome, LIFESPAN_GENE, GENE_BITS);
    let guarded = ops.field(genome, GUARD_GENE, GENE_BITS);
    let living = ops.mul(lifespan, life);
    let guarding = ops.mul(guarded, guard);
    let total = ops.add(base, living);
    ops.add(total, guarding)
}

fn mixed<O: Ops>(ops: &mut O, value: O::Word) -> O::Word {
    let first = ops.constant(MIX_FIRST);
    let second = ops.constant(MIX_SECOND);
    let shifted = ops.shift_right(value, 16);
    let out = ops.xor(value, shifted);
    let out = ops.mul(out, first);
    let shifted = ops.shift_right(out, 15);
    let out = ops.xor(out, shifted);
    let out = ops.mul(out, second);
    let shifted = ops.shift_right(out, 16);
    ops.xor(out, shifted)
}

fn salted<O: Ops>(ops: &mut O, value: O::Word, salt: u32) -> O::Word {
    let salt = ops.constant(salt);
    let folded = ops.xor(value, salt);
    mixed(ops, folded)
}

#[must_use]
pub fn mix(value: u32) -> u32 {
    mixed(&mut Cpu, value)
}

#[must_use]
pub fn base_of(at: [u32; 3], frame: u32, seed: u32) -> u32 {
    let ops = &mut Cpu;
    let noise = noise_by(ops, at, frame);
    let folded = ops.xor(noise, seed);
    mixed(ops, folded)
}

#[must_use]
pub fn roll_of(base: u32, corner: usize) -> u32 {
    salted(&mut Cpu, base, ROLL_SALT.wrapping_add(corner as u32))
}

#[must_use]
pub fn flips_of(base: u32, corner: usize, genome: u32, rate: u32) -> u32 {
    let ops = &mut Cpu;
    let (mask, _) = flips_by(ops, base, corner as u32, genome, rate);
    mask
}

fn flips_by<O: Ops>(
    ops: &mut O,
    base: O::Word,
    corner: u32,
    genome: O::Word,
    rate: O::Word,
) -> (O::Word, Vec<Flag<O::Word>>) {
    let placed = ops.shift_left(genome, 3);
    let spot = ops.constant(corner);
    let placed = ops.or(placed, spot);
    let folded = ops.xor(base, placed);
    let child = salted(ops, folded, CHILD_SALT);
    let mut mask = ops.constant(0);
    let mut flips = Vec::with_capacity(GENOME_BITS as usize);
    for pair in 0..GENOME_BITS / 2 {
        let draw = salted(ops, child, BIT_SALT.wrapping_mul(pair + 1));
        let low = ops.masked(draw, CHANCES - 1);
        let high = ops.shift_right(draw, 16);
        for (bit, chance) in [(2 * pair, low), (2 * pair + 1, high)] {
            let flips_here = ops.less(chance, rate);
            let one = ops.shift_left(flips_here.word(), bit);
            mask = ops.or(mask, one);
            flips.push(flips_here);
        }
    }
    (mask, flips)
}

#[derive(Clone, Copy)]
struct Living<W> {
    word: W,
    lives: Flag<W>,
    died: Flag<W>,
    energy: W,
    fertile: Flag<W>,
}

fn struck_by<O: Ops>(ops: &mut O, roll: O::Word, hazard: O::Word, guard: O::Word) -> Flag<O::Word> {
    let chance = ops.masked(roll, CHANCES - 1);
    let mut limit = hazard;
    for level in 1..1_u32 << GENE_BITS {
        let shifted = ops.shift_right(hazard, level);
        let here = ops.is(guard, level);
        limit = ops.select(here, shifted, limit);
    }
    ops.less(chance, limit)
}

fn lived_by<O: Ops>(
    ops: &mut O,
    word: O::Word,
    gain: O::Word,
    roll: O::Word,
    params: &[O::Word],
) -> Living<O::Word> {
    let bit = ops.masked(word, ALIVE);
    let alive = ops.is(bit, ALIVE);
    let energy = ops.field(word, ENERGY_SHIFT, ENERGY_BITS);
    let age = ops.field(word, AGE_SHIFT, AGE_BITS);
    let genome = ops.shift_right(word, GENOME_SHIFT);
    let cost = upkeep_by(ops, genome, params);
    let total = ops.add(energy, gain);
    let starved = ops.less_equal(total, cost);
    let one = ops.constant(1);
    let older = ops.add(age, one);
    let lifespan = ops.field(genome, LIFESPAN_GENE, GENE_BITS);
    let step = ops.constant(AGE_STEP);
    let span = ops.mul(lifespan, step);
    let least = ops.constant(AGE_LEAST);
    let span = ops.add(span, least);
    let aged = ops.greater(older, span);
    let hazard = param_by(ops, params, HAZARD);
    let guard = ops.field(genome, GUARD_GENE, GENE_BITS);
    let struck = struck_by(ops, roll, hazard, guard);
    let ends = ops.either(starved, aged);
    let ends = ops.either(ends, struck);
    let died = ops.both(alive, ends);
    let survives = ops.denied(ends);
    let lives = ops.both(alive, survives);
    let left = ops.sub(total, cost);
    let left = smaller(ops, left, FULL);
    let kept = ops.masked(word, !(ENERGY_MASK | AGE_MASK));
    let fuel = ops.shift_left(left, ENERGY_SHIFT);
    let years = ops.shift_left(older, AGE_SHIFT);
    let kept = ops.or(kept, fuel);
    let kept = ops.or(kept, years);
    let none = ops.constant(0);
    let gone = ops.select(alive, none, word);
    let out = ops.select(lives, kept, gone);
    let threshold = ops.field(genome, THRESHOLD_GENE, GENE_BITS);
    let rise = ops.constant(THRESHOLD_STEP);
    let threshold = ops.mul(threshold, rise);
    let floor = ops.constant(THRESHOLD_LEAST);
    let threshold = ops.add(threshold, floor);
    let ready = ops.greater_equal(left, threshold);
    let fertile = ops.both(lives, ready);
    Living {
        word: out,
        lives,
        died,
        energy: left,
        fertile,
    }
}

fn chosen_by<O: Ops>(
    ops: &mut O,
    free: [Flag<O::Word>; 3],
    turn: O::Word,
    fertile: Flag<O::Word>,
) -> [Flag<O::Word>; 3] {
    let never = ops.truth(false);
    let mut choice = [never; 3];
    for start in 0..3_usize {
        let mut taken = never;
        let mut picked = [never; 3];
        for place in 0..3 {
            let axis = (start + place) % 3;
            let open = free.get(axis).copied().unwrap_or(never);
            let left = ops.denied(taken);
            let picks = ops.both(open, left);
            if let Some(slot) = picked.get_mut(axis) {
                *slot = picks;
            }
            taken = ops.either(taken, open);
        }
        let here = ops.is(turn, start as u32);
        for (slot, pick) in choice.iter_mut().zip(picked) {
            let pick = ops.both(here, pick);
            *slot = ops.either(*slot, pick);
        }
    }
    choice.map(|pick| ops.both(pick, fertile))
}

#[must_use]
pub fn turn_of(roll: u32) -> u32 {
    ((roll >> 16) * 3) >> 16
}

fn turned_by<O: Ops>(ops: &mut O, roll: O::Word) -> O::Word {
    let high = ops.shift_right(roll, 16);
    let three = ops.constant(3);
    let spread = ops.mul(high, three);
    ops.shift_right(spread, 16)
}

fn along<T: Copy>(values: [T; 3], axis: Axis) -> T {
    let [x, y, z] = values;
    match axis {
        Axis::X => x,
        Axis::Y => y,
        Axis::Z => z,
    }
}

pub fn stepped<O: Ops>(ops: &mut O, oct: Oct<'_, O::Word>) -> Ruled<O::Word> {
    let params = oct.params;
    let seed = param_by(ops, params, SEED);
    let noise = noise_by(ops, oct.at, oct.frame);
    let folded = ops.xor(noise, seed);
    let base = mixed(ops, folded);
    let before = Octet(oct.words);
    let alive_before = before.map(|word| {
        let bit = ops.masked(word, ALIVE);
        ops.is(bit, ALIVE)
    });
    let living_before = ops.count(alive_before.0);
    let all = ops.constant(CORNERS as u32);
    let empties = ops.sub(all, living_before);
    let feed = param_capped(ops, params, FEED);
    let gain = ops.mul(feed, empties);
    let rolls =
        Octet::from_fn(|corner| salted(ops, base, ROLL_SALT.wrapping_add(corner.index() as u32)));
    let lived = Octet::from_fn(|corner| lived_by(ops, before[corner], gain, rolls[corner], params));
    let free = lived.map(|one| ops.denied(one.lives));
    let choices = Octet::from_fn(|corner| {
        let open = ORDER.map(|axis| free[corner.neighbour(axis)]);
        let turn = turned_by(ops, rolls[corner]);
        chosen_by(ops, open, turn, lived[corner].fertile)
    });
    let never = ops.truth(false);
    let rate = param_by(ops, params, MUTATION);
    let mut won = Octet([[never; 3]; CORNERS]);
    let mut out = lived.map(|one| one.word);
    let mut born = Octet([never; CORNERS]);
    let mut flipped = Vec::new();
    for target in Corner::ALL {
        let mut taken = never;
        let mut parent = ops.constant(0);
        for axis in ORDER {
            let from = target.neighbour(axis);
            let claims = along(choices[from], axis);
            let free_here = ops.denied(taken);
            let wins = ops.both(claims, free_here);
            taken = ops.either(taken, claims);
            parent = ops.select(wins, lived[from].word, parent);
            won[from] = winning(won[from], axis, wins);
        }
        let energy = ops.field(parent, ENERGY_SHIFT, ENERGY_BITS);
        let half = ops.shift_right(energy, 1);
        let lineage = ops.shift_right(parent, GENOME_SHIFT);
        let (mask, flips) = flips_by(ops, base, target.index() as u32, lineage, rate);
        let heir = ops.xor(lineage, mask);
        let heir = ops.shift_left(heir, GENOME_SHIFT);
        let fuel = ops.shift_left(half, ENERGY_SHIFT);
        let alive = ops.constant(ALIVE);
        let child = ops.or(heir, fuel);
        let child = ops.or(child, alive);
        out[target] = ops.select(taken, child, out[target]);
        born[target] = taken;
        for flip in flips {
            flipped.push(ops.both(flip, taken));
        }
    }
    let birth = param_capped(ops, params, BIRTH_COST);
    let mut paid_off = Octet([never; CORNERS]);
    for corner in Corner::ALL {
        let one = lived[corner];
        let [x, y, z] = won[corner];
        let gave = ops.either(x, y);
        let gave = ops.either(gave, z);
        let half = ops.shift_right(one.energy, 1);
        let kept = ops.sub(one.energy, half);
        let spent = ops.less_equal(kept, birth);
        let ends = ops.both(gave, spent);
        let rest = ops.sub(kept, birth);
        let fuel = ops.shift_left(rest, ENERGY_SHIFT);
        let cleared = ops.masked(out[corner], !ENERGY_MASK);
        let paid = ops.or(cleared, fuel);
        let none = ops.constant(0);
        let after = ops.select(spent, none, paid);
        out[corner] = ops.select(gave, after, out[corner]);
        paid_off[corner] = ends;
    }
    let changed = Octet::from_fn(|corner| ops.not_equal(before[corner], out[corner]));
    let alive_after = out.map(|word| {
        let bit = ops.masked(word, ALIVE);
        ops.is(bit, ALIVE)
    });
    let died = lived.map(|one| one.died);
    let changed = ops.count(changed.0);
    let living = ops.count(alive_after.0);
    let births = ops.count(born.0);
    let starved = ops.count(died.0);
    let spent = ops.count(paid_off.0);
    let deaths = ops.add(starved, spent);
    let mutations = ops.count(flipped);
    Ruled {
        out: out.0,
        counts: vec![changed, living, births, deaths, mutations],
    }
}

const fn winning<T: Copy>(won: [T; 3], axis: Axis, wins: T) -> [T; 3] {
    let [x, y, z] = won;
    match axis {
        Axis::X => [wins, y, z],
        Axis::Y => [x, wins, z],
        Axis::Z => [x, y, wins],
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Evo;

impl Declared for Evo {
    fn params(&self) -> usize {
        PARAMS
    }

    fn tally(&self) -> Tally {
        Tally {
            counts: COUNTS,
            weighted: true,
        }
    }
}

impl OneStep for Evo {
    fn one_step<O: Ops>(&self, ops: &mut O, oct: Oct<'_, O::Word>) -> Ruled<O::Word> {
        stepped(ops, oct)
    }
}
