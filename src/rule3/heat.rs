//! Heat with phase changes, a rule of the 3D engine.
//!
//! A cell word holds:
//!
//! | bits | field |
//! |---|---|
//! | 0 to 3 | the kind: [`AIR`], [`WALL`], [`ICE`], [`WATER`], [`STEAM`], [`FUEL`], [`FIRE`], [`ASH`]; 8 to 15 are inert |
//! | 4 to 7 | the life of a fire, in steps |
//! | 8 to 15 | kept as they are |
//! | 16 to 31 | the temperature, from 0 to [`TOP`] |
//!
//! One step of an oct runs three stages in order:
//!
//! 1. **Reactions**, each cell on its own:
//!    - ice at [`MELT`] or warmer melts to water;
//!    - water at [`BOIL`] or warmer boils to steam;
//!    - steam colder than [`CONDENSE`] condenses to water;
//!    - fuel at the ignition temperature or warmer catches fire when its
//!      nibble of the noise is below the chance, or surely once it is
//!      [`WINDOW`] past the ignition temperature or at [`TOP`]; fuel that does
//!      not catch fire warms by one;
//!    - fire gains the flare, up to [`TOP`], and loses one step of life; fire
//!      with no life left turns to ash instead.
//! 2. **Diffusion** among the cells that conduct: every kind but [`WALL`] and
//!    the inert kinds. Their total heat is shared out evenly; what does not
//!    divide goes one degree each to the first conducting cells in corner
//!    order. The total is kept exactly.
//! 3. **Movement**: in each of the four vertical pairs, a lighter cell under
//!    a heavier one trades places with it, whole words, so heat moves with
//!    its cell. Steam is lighter than air and air than water; the other
//!    kinds do not move.
//!
//! So the block's heat changes only at the documented sources and sinks:
//! fuel warming by one, fire gaining its flare, and the flare cut at [`TOP`].
//! The step reads the frame only through the noise, and only for fuel that
//! changes whatever the noise says, so a quiet oct stays quiet.
//!
//! Its three parameters, from [`crate::rule3::FIRST_PARAM`] on, are the
//! ignition temperature, the chance (fuel catches fire when its nibble is
//! below it, so 16 is always) and the flare. Its tally words are the cells
//! that changed, the fire cells and the water cells after the step, the
//! first weighted.
//!
//! The rule is one [`OneStep`] over [`Ops`]: the CPU functions below run
//! that step on `u32`.

use super::{
    Axis, CORNERS, Corner, Declared, Oct, Octet, OneStep, Ruled, Tally, noise_by, param, param_by,
};
use crate::ops::{Cpu, Flag, Helpers, Ops};

pub const AIR: u32 = 0;
pub const WALL: u32 = 1;
pub const ICE: u32 = 2;
pub const WATER: u32 = 3;
pub const STEAM: u32 = 4;
pub const FUEL: u32 = 5;
pub const FIRE: u32 = 6;
pub const ASH: u32 = 7;

/// The kinds the rule knows; a kind field at or past it is inert.
pub const KINDS: u32 = 8;

const KIND_MASK: u32 = 0xF;
const LIFE_SHIFT: u32 = 4;
const LIFE_MASK: u32 = 0xF << LIFE_SHIFT;
const HEAT_SHIFT: u32 = 16;
const KEPT_MASK: u32 = (1 << HEAT_SHIFT) - 1;

/// The hottest a cell gets.
pub const TOP: u32 = 0xFFFF;

pub const MELT: u32 = 273;
pub const BOIL: u32 = 373;
pub const CONDENSE: u32 = 363;

/// How far past the ignition temperature fuel waits for its chance.
pub const WINDOW: u32 = 64;

/// The life of a fire that fuel starts.
pub const LIFE: u32 = 12;

/// The parameters [`Heat`] reads: the ignition temperature, the chance out
/// of 16, and the flare.
pub const PARAMS: usize = 3;

/// Parameters for the tests: fuel catches fire from 600 with a chance of 4
/// in 16, and a fire gains 4096 a step.
pub const TUNED: [u32; PARAMS] = [600, 4, 4096];

/// The place in the density order of a kind that does not move.
const STILL: u32 = 3;

#[must_use]
pub const fn kind_of(word: u32) -> u32 {
    word & KIND_MASK
}

#[must_use]
pub const fn life_of(word: u32) -> u32 {
    (word & LIFE_MASK) >> LIFE_SHIFT
}

#[must_use]
pub const fn temperature_of(word: u32) -> u32 {
    word >> HEAT_SHIFT
}

/// A cell of `kind` at `temperature`, cut to [`TOP`], with no life.
#[must_use]
pub const fn cell(kind: u32, temperature: u32) -> u32 {
    let temperature = if temperature > TOP { TOP } else { temperature };
    (kind & KIND_MASK) | (temperature << HEAT_SHIFT)
}

/// Whether a cell of `kind` takes part in diffusion.
#[must_use]
pub const fn conducts(kind: u32) -> bool {
    kind != WALL && kind < KINDS
}

fn with_kind<O: Ops>(ops: &mut O, word: O::Word, kind: u32) -> O::Word {
    let kept = ops.masked(word, !KIND_MASK);
    let kind = ops.constant(kind);
    ops.or(kept, kind)
}

fn with_temperature<O: Ops>(ops: &mut O, word: O::Word, temperature: O::Word) -> O::Word {
    let kept = ops.masked(word, KEPT_MASK);
    let placed = ops.shift_left(temperature, HEAT_SHIFT);
    ops.or(kept, placed)
}

/// `value`, cut to `cap`.
fn smaller<O: Ops>(ops: &mut O, value: O::Word, cap: u32) -> O::Word {
    let cap = ops.constant(cap);
    let over = ops.greater(value, cap);
    ops.select(over, cap, value)
}

/// The reaction of one cell over [`Ops`], `nibble` its four bits of the
/// noise.
fn reacted_by<O: Ops>(
    ops: &mut O,
    word: O::Word,
    nibble: O::Word,
    [ignite, chance, flare]: [O::Word; PARAMS],
) -> O::Word {
    let kind = ops.masked(word, KIND_MASK);
    let temperature = ops.shift_right(word, HEAT_SHIFT);
    let mut out = word;

    let ice = ops.is(kind, ICE);
    let melt = ops.constant(MELT);
    let warm = ops.greater_equal(temperature, melt);
    let melts = ops.both(ice, warm);
    let melted = with_kind(ops, word, WATER);
    out = ops.select(melts, melted, out);

    let water = ops.is(kind, WATER);
    let boil = ops.constant(BOIL);
    let hot = ops.greater_equal(temperature, boil);
    let boils = ops.both(water, hot);
    let boiled = with_kind(ops, word, STEAM);
    out = ops.select(boils, boiled, out);

    let steam = ops.is(kind, STEAM);
    let condense = ops.constant(CONDENSE);
    let cold = ops.less(temperature, condense);
    let condenses = ops.both(steam, cold);
    let condensed = with_kind(ops, word, WATER);
    out = ops.select(condenses, condensed, out);

    let fuel = ops.is(kind, FUEL);
    let warm = ops.greater_equal(temperature, ignite);
    let hot = ops.both(fuel, warm);
    let past = ops.sub(temperature, ignite);
    let window = ops.constant(WINDOW);
    let late = ops.greater_equal(past, window);
    let full = ops.is(temperature, TOP);
    let sure = ops.either(late, full);
    let lucky = ops.less(nibble, chance);
    let catches = ops.either(sure, lucky);
    let ignites = ops.both(hot, catches);
    let waits = ops.denied(catches);
    let warms = ops.both(hot, waits);
    let kept = ops.masked(word, !(KIND_MASK | LIFE_MASK));
    let lit = ops.constant(FIRE | (LIFE << LIFE_SHIFT));
    let lit = ops.or(kept, lit);
    out = ops.select(ignites, lit, out);
    let degree = ops.constant(1 << HEAT_SHIFT);
    let warmer = ops.add(word, degree);
    out = ops.select(warms, warmer, out);

    let fire = ops.is(kind, FIRE);
    let gain = smaller(ops, flare, TOP);
    let hotter = ops.add(temperature, gain);
    let hotter = smaller(ops, hotter, TOP);
    let warmed = with_temperature(ops, word, hotter);
    let life = ops.masked(word, LIFE_MASK);
    let spent = ops.is(life, 0);
    let ash = with_kind(ops, warmed, ASH);
    let step = ops.constant(1 << LIFE_SHIFT);
    let burning = ops.sub(warmed, step);
    let burnt = ops.select(spent, ash, burning);
    ops.select(fire, burnt, out)
}

/// The reaction of one cell, `nibble` its four bits of the noise.
#[must_use]
pub fn reacted(word: u32, nibble: u32, params: &[u32]) -> u32 {
    let params = [0, 1, 2].map(|which| param(params, which));
    reacted_by(&mut Cpu, word, nibble, params)
}

/// Whether `word` conducts, over [`Ops`].
fn conducting<O: Ops>(ops: &mut O, word: O::Word) -> Flag<O::Word> {
    let kind = ops.masked(word, KIND_MASK);
    let wall = ops.constant(WALL);
    let open = ops.not_equal(kind, wall);
    let kinds = ops.constant(KINDS);
    let known = ops.less(kind, kinds);
    ops.both(open, known)
}

/// The diffusion of one oct over [`Ops`].
fn diffused_by<O: Ops>(ops: &mut O, cells: Octet<O::Word>) -> Octet<O::Word> {
    let conducts = cells.map(|word| conducting(ops, word));
    let mut total = ops.constant(0);
    for corner in Corner::ALL {
        let warmth = ops.shift_right(cells[corner], HEAT_SHIFT);
        let held = ops.mul(warmth, conducts[corner].word());
        total = ops.add(total, held);
    }
    let count = ops.count(conducts.0);
    let share = ops.div(total, count);
    let spread = ops.mul(share, count);
    let rest = ops.sub(total, spread);

    let mut out = cells;
    let mut rank = ops.constant(0);
    for corner in Corner::ALL {
        let flag = conducts[corner];
        let extra = ops.greater(rest, rank);
        let temperature = ops.add(share, extra.word());
        let shared = with_temperature(ops, cells[corner], temperature);
        out[corner] = ops.select(flag, shared, cells[corner]);
        rank = ops.add(rank, flag.word());
    }
    out
}

/// The diffusion of one oct: the heat of its conducting cells shared out
/// evenly, the rest one degree each to the first of them in corner order.
#[must_use]
pub fn diffused(cells: [u32; CORNERS]) -> [u32; CORNERS] {
    diffused_by(&mut Cpu, Octet(cells)).0
}

/// Where `word`'s kind sits in the density order, lightest first, with
/// [`STILL`] for a kind that does not move.
fn density<O: Ops>(ops: &mut O, word: O::Word) -> O::Word {
    let kind = ops.masked(word, KIND_MASK);
    let mut out = ops.constant(STILL);
    for (which, place) in [(STEAM, 0), (AIR, 1), (WATER, 2)] {
        let found = ops.is(kind, which);
        let place = ops.constant(place);
        out = ops.select(found, place, out);
    }
    out
}

/// The movement of one oct over [`Ops`].
fn moved_by<O: Ops>(ops: &mut O, cells: Octet<O::Word>) -> Octet<O::Word> {
    let still = ops.constant(STILL);
    let mut out = cells;
    for top in Corner::ALL {
        if top.side(Axis::Y) != 0 {
            continue;
        }
        let below = top.neighbour(Axis::Y);
        let (upper, lower) = (out[top], out[below]);
        let over = density(ops, upper);
        let under = density(ops, lower);
        let moves = ops.less(over, still);
        let can = ops.less(under, still);
        let heavier = ops.greater(over, under);
        let trade = ops.both(moves, can);
        let trade = ops.both(trade, heavier);
        out[top] = ops.select(trade, lower, upper);
        out[below] = ops.select(trade, upper, lower);
    }
    out
}

/// The movement of one oct: each vertical pair trades a lighter cell under
/// a heavier one.
#[must_use]
pub fn moved(cells: [u32; CORNERS]) -> [u32; CORNERS] {
    moved_by(&mut Cpu, Octet(cells)).0
}

/// One step of one oct over [`Ops`], without the tally.
fn stepped_by<O: Ops>(
    ops: &mut O,
    words: Octet<O::Word>,
    noise: O::Word,
    params: [O::Word; PARAMS],
) -> Octet<O::Word> {
    let mut cells = words;
    for corner in Corner::ALL {
        let by = u32::try_from(4 * corner.index()).unwrap_or(0);
        let shifted = ops.shift_right(noise, by);
        let nibble = ops.masked(shifted, 0xF);
        cells[corner] = reacted_by(ops, cells[corner], nibble, params);
    }
    let cells = diffused_by(ops, cells);
    moved_by(ops, cells)
}

/// One step of one oct on the CPU, without the tally: reactions, diffusion
/// and movement.
#[must_use]
pub fn stepped(words: [u32; CORNERS], noise: u32, params: &[u32]) -> [u32; CORNERS] {
    let params = [0, 1, 2].map(|which| param(params, which));
    stepped_by(&mut Cpu, Octet(words), noise, params).0
}

/// Heat with phase changes, as the module describes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Heat;

impl Declared for Heat {
    fn params(&self) -> usize {
        PARAMS
    }

    fn tally(&self) -> Tally {
        Tally {
            counts: 3,
            weighted: true,
        }
    }
}

impl OneStep for Heat {
    fn one_step<O: Ops>(&self, ops: &mut O, oct: Oct<'_, O::Word>) -> Ruled<O::Word> {
        let params = [0, 1, 2].map(|which| param_by(ops, oct.params, which));
        let noise = noise_by(ops, oct.at, oct.frame);
        let before = Octet(oct.words);
        let out = stepped_by(ops, before, noise, params);
        let changed = Octet::from_fn(|corner| ops.not_equal(before[corner], out[corner]));
        let kinds = out.map(|after| ops.masked(after, KIND_MASK));
        let fire = kinds.map(|kind| ops.is(kind, FIRE));
        let water = kinds.map(|kind| ops.is(kind, WATER));
        let counts = vec![ops.count(changed.0), ops.count(fire.0), ops.count(water.0)];
        Ruled { out: out.0, counts }
    }
}
