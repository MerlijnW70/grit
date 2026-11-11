//! Nature, a rule of the 3D engine that holds every material of a valley
//! in one world: rock, earth, sand, snow, ice, water, wood, leaves, fire,
//! ash, smoke and steam, in air, each cell with a temperature.
//!
//! A cell word holds:
//!
//! | bits | field |
//! |---|---|
//! | 0 to 3 | the kind: [`AIR`], [`ROCK`], [`EARTH`], [`SAND`], [`SNOW`], [`ICE`], [`WATER`], [`WOOD`], [`LEAVES`], [`FIRE`], [`ASH`], [`SMOKE`], [`STEAM`]; 13 to 15 are inert |
//! | 4 to 11 | the state, read by kind, see below |
//! | 12 to 15 | kept as they are |
//! | 16 to 31 | the temperature, from 0 to [`TOP`] |
//!
//! The kind is the low nibble, so a view maps a word to its kind with the
//! mask `0xF` and the shift 0. The state means:
//!
//! - water: the running way in bits 4 to 6 ([`MINUS_X`], [`PLUS_X`],
//!   [`MINUS_Z`], [`PLUS_Z`]; the other values name no way), the held bit
//!   [`HELD`] in bit 7 and the tired steps in bits 8 to 10, as the sand
//!   rule keeps them;
//! - fire and smoke: the life left, in steps, in bits 4 to 11;
//! - air: in bits 4 to 6, [`DEEPEST`] less the air cells under it, counted
//!   up to [`DEEPEST`], as it last saw them, so the word 0 is air that
//!   counts the most;
//! - every other kind: kept as it is.
//!
//! A cell that turns into another kind starts with a state of 0, but for a
//! fire or smoke, which start with their life. Its kept bits and its
//! temperature stay.
//!
//! The density order, lightest first, is smoke, steam, air, water, and
//! then the granular kinds, earth, sand, snow and ash, all alike. Rock,
//! ice, wood, leaves, fire and the inert kinds are still: they never move.
//!
//! One step of an oct runs these stages in order. Each corner has its
//! nibble of the engine's noise, its four bits from bit 4 times its index.
//!
//! 1. **Reactions**, each cell on its own:
//!    - snow and ice at [`MELT`] or warmer melt to water;
//!    - water at [`BOIL`] or warmer boils to steam;
//!    - steam colder than [`CONDENSE`] condenses to water;
//!    - wood at [`WOOD_IGNITES`] or warmer, and leaves at
//!      [`LEAVES_IGNITES`] or warmer, catch fire when their nibble is below
//!      [`CHANCE`], or surely once [`WINDOW`] past it or at [`TOP`]; fuel
//!      that does not catch fire warms by one. Fire from wood lives for
//!      [`WOOD_LIFE`] steps and fire from leaves for [`LEAVES_LIFE`];
//!    - fire loses one step of life, and fire with no life left burns out
//!      to ash;
//!    - smoke loses one step of life, and smoke with no life left fades to
//!      air.
//! 2. **Contact**, across the oct:
//!    - an oct that holds both fire and water puts the fire out: every fire
//!      turns to ash, and every water to steam at [`BOIL`], or at its own
//!      temperature if warmer;
//!    - in an oct that still holds fire, wood catches fire when its nibble
//!      is below [`WOOD_CATCHES`] and leaves when theirs is below
//!      [`LEAVES_CATCHES`], whatever their temperature, so fire spreads to
//!      the fuel next to it;
//!    - in an oct that holds fire, every cell that conducts warms by
//!      [`FLARE`], up to [`TOP`]: the heat the fire releases;
//!    - air right above a fire turns to smoke with [`SMOKE_LIFE`].
//! 3. **Diffusion** among the cells that conduct, every kind but rock,
//!    fire and the inert kinds, as in [`super::heat`], but only in an oct
//!    whose conducting cells differ by more than one degree: their total
//!    heat is shared out evenly, and what does not divide goes one degree
//!    each to the first conducting cells in corner order. The total is
//!    kept exactly. A share that acts lowers the sum of the squares of the
//!    temperatures, so without fire the heat comes to rest. A fire keeps
//!    its own temperature.
//! 4. **Movement**, in the frame's axis order, as the sand rule moves its
//!    cells:
//!    - water in an upper cell is held when water lies under it; held idle
//!      water that is not tired out starts running toward air beside it,
//!      along the first axis, else the second; runners trade places with
//!      the air they run into, along the first axis and then the second,
//!      and stop when they meet anything else;
//!    - in each of the four vertical pairs, a heavier cell over a lighter
//!      one trades places with it, so the granular kinds and water fall and
//!      smoke and steam rise;
//!    - then each upper cell of water or of a granular kind falls along an
//!      edge diagonal of the first axis, then of the second, then along the
//!      corner diagonal, into a lighter cell. Water and sand fall into any
//!      lighter cell. Ash, snow and earth fall only into air that counts at
//!      least its [`steepness`] less one air cells under it, so each keeps
//!      a steeper slope than the one before it.
//!
//!    The cell that falls keeps its kind, kept bits and temperature, and
//!    clears its state; the cell it passes keeps its word.
//! 5. **Wind.** Smoke trades places with air beside it along x toward plus
//!    x when its nibble is below [`WIND_WITH`], toward minus x when it is
//!    below [`WIND_AGAINST`], and along z either way when it is
//!    [`ACROSS`] or more; x pairs first, then z pairs.
//! 6. **Air counts.** Air in an upper cell takes its count from the cell
//!    under it: one more than that cell's count if it is air, up to
//!    [`DEEPEST`], else none.
//!
//! The frame's axis order is x first for two frames, one of each phase,
//! and then z first for two, as [`super::sand::x_first`] gives it. A z
//! first step is the x first step of the oct mirrored across the plane
//! x = z, with the ways of running water traded too.
//!
//! The quiet contract of [`crate::rule3`] holds. The noise decides only
//! for fuel at its ignition temperature or warmer, for fuel in an oct with
//! fire, and for smoke. Fuel is never made, fire is made only from fuel,
//! and smoke only where fire is. So a step in such an oct turns some fuel
//! into fire; or else takes a step of life from every fire, of which there
//! is no new one; or else, with no fire, takes a step of life from every
//! smoke; or else warms every hot fuel cell by one and so raises the
//! oct's total heat. An oct in which the noise decides always changes,
//! whatever the noise says. The other stages read only the eight words and
//! treat x and z alike, so an oct a step leaves unchanged is left unchanged
//! in either axis order. A landscape at rest, with no fire or smoke,
//! settled slopes, still water and one temperature below every threshold,
//! is quiet.
//!
//! The rule reads no parameters. Its tally words are the cells that
//! changed, and the fire, smoke and water cells after the step, the first
//! weighted.

use super::{Axis, CORNERS, Corner, Declared, Oct, Octet, OneStep, Ruled, Tally, noise_by};
use crate::ops::{Cpu, Flag, Helpers, Ops};

pub const AIR: u32 = 0;
pub const ROCK: u32 = 1;
pub const EARTH: u32 = 2;
pub const SAND: u32 = 3;
pub const SNOW: u32 = 4;
pub const ICE: u32 = 5;
pub const WATER: u32 = 6;
pub const WOOD: u32 = 7;
pub const LEAVES: u32 = 8;
pub const FIRE: u32 = 9;
pub const ASH: u32 = 10;
pub const SMOKE: u32 = 11;
pub const STEAM: u32 = 12;

/// The kinds the rule knows; a kind field at or past it is inert.
pub const KINDS: u32 = 13;

/// The granular kinds, which fall and pile.
pub const GRANULAR: [u32; 4] = [EARTH, SAND, SNOW, ASH];

const KIND_MASK: u32 = 0xF;
const STATE_SHIFT: u32 = 4;
const STATE_MASK: u32 = 0xFF << STATE_SHIFT;
const RUNNING_BITS: u32 = 3;
const RUNNING_MASK: u32 = ((1 << RUNNING_BITS) - 1) << STATE_SHIFT;
const TIRED_BITS: u32 = 3;
const HEAT_SHIFT: u32 = 16;
const KEPT_MASK: u32 = (1 << HEAT_SHIFT) - 1;
const COUNT_BITS: u32 = 3;
const COUNT_MASK: u32 = ((1 << COUNT_BITS) - 1) << STATE_SHIFT;

/// The held bit of water.
pub const HELD: u32 = 1 << (STATE_SHIFT + RUNNING_BITS);

/// One tired step of water.
pub const TIRED: u32 = HELD << 1;

const WORN: u32 = ((1 << TIRED_BITS) - 1) * TIRED;

/// How many times water starts to run before it is tired out.
pub const PATIENCE: u32 = crate::sand::PATIENCE;

pub const MINUS_X: u32 = 1;
pub const PLUS_X: u32 = 2;
pub const MINUS_Z: u32 = 5;
pub const PLUS_Z: u32 = 6;

/// The most air cells under it that air counts.
pub const DEEPEST: u32 = (1 << COUNT_BITS) - 1;

/// The hottest a cell gets.
pub const TOP: u32 = 0xFFFF;

pub const MELT: u32 = 273;
pub const BOIL: u32 = 373;
pub const CONDENSE: u32 = 363;

pub const WOOD_IGNITES: u32 = 600;
pub const LEAVES_IGNITES: u32 = 450;

/// Fuel at its ignition temperature catches fire when its nibble of the
/// noise is below this.
pub const CHANCE: u32 = 4;

/// How far past its ignition temperature fuel waits for its chance.
pub const WINDOW: u32 = 64;

/// The heat each conducting cell of an oct with fire gains each step.
pub const FLARE: u32 = 32;

/// Wood in an oct with a fire catches fire when its nibble of the noise is
/// below this.
pub const WOOD_CATCHES: u32 = 2;

/// Leaves in an oct with a fire catch fire when their nibble of the noise
/// is below this.
pub const LEAVES_CATCHES: u32 = 6;

pub const WOOD_LIFE: u32 = 60;
pub const LEAVES_LIFE: u32 = 15;
pub const SMOKE_LIFE: u32 = 96;

/// Smoke beside air toward plus x moves there when its nibble is below
/// this.
pub const WIND_WITH: u32 = 6;

/// Smoke beside air toward minus x moves there when its nibble is below
/// this.
pub const WIND_AGAINST: u32 = 2;

/// Smoke beside air along z moves there when its nibble is this or more.
pub const ACROSS: u32 = 12;

/// The place in the density order of a kind that does not move.
const STILL: u32 = 5;

/// The place in the density order of water; water and every heavier kind
/// fall along diagonals.
const WET: u32 = 3;

#[must_use]
pub const fn kind_of(word: u32) -> u32 {
    word & KIND_MASK
}

/// The life of a fire or smoke cell.
#[must_use]
pub const fn life_of(word: u32) -> u32 {
    (word & STATE_MASK) >> STATE_SHIFT
}

#[must_use]
pub const fn temperature_of(word: u32) -> u32 {
    word >> HEAT_SHIFT
}

/// The running way of a water cell.
#[must_use]
pub const fn running_of(word: u32) -> u32 {
    (word & RUNNING_MASK) >> STATE_SHIFT
}

#[must_use]
pub const fn held_of(word: u32) -> u32 {
    (word & HELD) / HELD
}

#[must_use]
pub const fn tired_of(word: u32) -> u32 {
    (word & WORN) / TIRED
}

/// The air cells an air cell counts under it, up to [`DEEPEST`].
#[must_use]
pub const fn depth_of(word: u32) -> u32 {
    DEEPEST - ((word & COUNT_MASK) >> STATE_SHIFT)
}

/// A cell of `kind` at `temperature`, cut to [`TOP`], with a state of 0.
#[must_use]
pub const fn cell(kind: u32, temperature: u32) -> u32 {
    let temperature = if temperature > TOP { TOP } else { temperature };
    (kind & KIND_MASK) | (temperature << HEAT_SHIFT)
}

/// A fire or smoke cell of `kind` at `temperature` with `life` steps left.
#[must_use]
pub const fn alive(kind: u32, temperature: u32, life: u32) -> u32 {
    cell(kind, temperature) | ((life << STATE_SHIFT) & STATE_MASK)
}

/// An air cell at `temperature` that counts `depth` air cells under it.
#[must_use]
pub const fn air(temperature: u32, depth: u32) -> u32 {
    let depth = if depth > DEEPEST { DEEPEST } else { depth };
    cell(AIR, temperature) | ((DEEPEST - depth) << STATE_SHIFT)
}

/// Whether a cell of `kind` takes part in diffusion: every kind but rock,
/// fire and the inert kinds.
#[must_use]
pub const fn conducts(kind: u32) -> bool {
    kind != ROCK && kind != FIRE && kind < KINDS
}

/// The place of `kind` in the density order, lightest first: smoke 0,
/// steam 1, air 2, water 3, the granular kinds 4; none for a kind that does
/// not move.
#[must_use]
pub const fn density_of(kind: u32) -> Option<u32> {
    match kind {
        SMOKE => Some(0),
        STEAM => Some(1),
        AIR => Some(2),
        WATER => Some(WET),
        EARTH | SAND | SNOW | ASH => Some(4),
        _ => None,
    }
}

/// How steeply granular `kind` piles: a grain of it falls along a diagonal
/// only into air that counts this less one air cells under it. 1 for sand,
/// 2 for ash, 4 for snow and 6 for earth; none for a kind that is not
/// granular.
#[must_use]
pub const fn steepness(kind: u32) -> Option<u32> {
    match kind {
        SAND => Some(1),
        ASH => Some(2),
        SNOW => Some(4),
        EARTH => Some(6),
        _ => None,
    }
}

fn kind<O: Ops>(ops: &mut O, word: O::Word) -> O::Word {
    ops.masked(word, KIND_MASK)
}

fn is_kind<O: Ops>(ops: &mut O, word: O::Word, want: u32) -> Flag<O::Word> {
    let held = kind(ops, word);
    ops.is(held, want)
}

fn temperature<O: Ops>(ops: &mut O, word: O::Word) -> O::Word {
    ops.shift_right(word, HEAT_SHIFT)
}

/// `word` turned into `kind`, its state set to `state`.
fn turned<O: Ops>(ops: &mut O, word: O::Word, kind: u32, state: u32) -> O::Word {
    let kept = ops.masked(word, !(KIND_MASK | STATE_MASK));
    let fresh = ops.constant(kind | ((state << STATE_SHIFT) & STATE_MASK));
    ops.or(kept, fresh)
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

/// `value`, raised to `floor`.
fn larger<O: Ops>(ops: &mut O, value: O::Word, floor: u32) -> O::Word {
    let floor = ops.constant(floor);
    let under = ops.less(value, floor);
    ops.select(under, floor, value)
}

/// Fuel of `fuel` in `word` catching fire or warming, over [`Ops`].
fn burned<O: Ops>(
    ops: &mut O,
    word: O::Word,
    out: O::Word,
    nibble: O::Word,
    (fuel, ignites, life): (u32, u32, u32),
) -> O::Word {
    let is_fuel = is_kind(ops, word, fuel);
    let warmth = temperature(ops, word);
    let ignites = ops.constant(ignites);
    let warm = ops.greater_equal(warmth, ignites);
    let hot = ops.both(is_fuel, warm);
    let past = ops.sub(warmth, ignites);
    let window = ops.constant(WINDOW);
    let late = ops.greater_equal(past, window);
    let full = ops.is(warmth, TOP);
    let sure = ops.either(late, full);
    let chance = ops.constant(CHANCE);
    let lucky = ops.less(nibble, chance);
    let catches = ops.either(sure, lucky);
    let lit = ops.both(hot, catches);
    let waits = ops.denied(catches);
    let warms = ops.both(hot, waits);
    let fire = turned(ops, word, FIRE, life);
    let degree = ops.constant(1 << HEAT_SHIFT);
    let warmer = ops.add(word, degree);
    let out = ops.select(lit, fire, out);
    ops.select(warms, warmer, out)
}

/// The reaction of one cell over [`Ops`], `nibble` its four bits of the
/// noise.
fn reacted_by<O: Ops>(ops: &mut O, word: O::Word, nibble: O::Word) -> O::Word {
    let held = kind(ops, word);
    let warmth = temperature(ops, word);
    let mut out = word;

    let snow = ops.is(held, SNOW);
    let ice = ops.is(held, ICE);
    let frozen = ops.either(snow, ice);
    let melt = ops.constant(MELT);
    let warm = ops.greater_equal(warmth, melt);
    let melts = ops.both(frozen, warm);
    let water = turned(ops, word, WATER, 0);
    out = ops.select(melts, water, out);

    let wet = ops.is(held, WATER);
    let boil = ops.constant(BOIL);
    let hot = ops.greater_equal(warmth, boil);
    let boils = ops.both(wet, hot);
    let steam = turned(ops, word, STEAM, 0);
    out = ops.select(boils, steam, out);

    let vapour = ops.is(held, STEAM);
    let condense = ops.constant(CONDENSE);
    let cold = ops.less(warmth, condense);
    let condenses = ops.both(vapour, cold);
    out = ops.select(condenses, water, out);

    out = burned(ops, word, out, nibble, (WOOD, WOOD_IGNITES, WOOD_LIFE));
    out = burned(
        ops,
        word,
        out,
        nibble,
        (LEAVES, LEAVES_IGNITES, LEAVES_LIFE),
    );

    let step = ops.constant(1 << STATE_SHIFT);
    let life = ops.masked(word, STATE_MASK);
    let spent = ops.is(life, 0);
    let older = ops.sub(word, step);

    let fire = ops.is(held, FIRE);
    let ash = turned(ops, word, ASH, 0);
    let burnt = ops.select(spent, ash, older);
    out = ops.select(fire, burnt, out);

    let smoke = ops.is(held, SMOKE);
    let faded = turned(ops, word, AIR, 0);
    let thinner = ops.select(spent, faded, older);
    ops.select(smoke, thinner, out)
}

/// The reaction of one cell, `nibble` its four bits of the noise.
#[must_use]
pub fn reacted(word: u32, nibble: u32) -> u32 {
    reacted_by(&mut Cpu, word, nibble)
}

/// The contact stage of one oct over [`Ops`].
fn touched_by<O: Ops>(ops: &mut O, cells: Octet<O::Word>, noise: O::Word) -> Octet<O::Word> {
    let mut fire = ops.truth(false);
    let mut water = ops.truth(false);
    for (_, &word) in cells.iter() {
        let burning = is_kind(ops, word, FIRE);
        fire = ops.either(fire, burning);
        let wet = is_kind(ops, word, WATER);
        water = ops.either(water, wet);
    }
    let quench = ops.both(fire, water);
    let mut out = cells;
    for corner in Corner::ALL {
        let word = cells[corner];
        let burning = is_kind(ops, word, FIRE);
        let put_out = ops.both(quench, burning);
        let ash = turned(ops, word, ASH, 0);
        let wet = is_kind(ops, word, WATER);
        let boiled = ops.both(quench, wet);
        let warmth = temperature(ops, word);
        let warmth = larger(ops, warmth, BOIL);
        let steam = turned(ops, word, STEAM, 0);
        let steam = with_temperature(ops, steam, warmth);
        let next = ops.select(put_out, ash, word);
        out[corner] = ops.select(boiled, steam, next);
    }
    let mut burning = ops.truth(false);
    for (_, &word) in out.iter() {
        let lit = is_kind(ops, word, FIRE);
        burning = ops.either(burning, lit);
    }
    for corner in Corner::ALL {
        let word = out[corner];
        let roll = nibble(ops, noise, corner);
        let mut next = word;
        for (fuel, catch, life) in [
            (WOOD, WOOD_CATCHES, WOOD_LIFE),
            (LEAVES, LEAVES_CATCHES, LEAVES_LIFE),
        ] {
            let found = is_kind(ops, word, fuel);
            let catch = ops.constant(catch);
            let lucky = ops.less(roll, catch);
            let caught = ops.both(found, lucky);
            let caught = ops.both(caught, burning);
            let lit = turned(ops, word, FIRE, life);
            next = ops.select(caught, lit, next);
        }
        out[corner] = next;
    }
    let flare = ops.constant(FLARE);
    for corner in Corner::ALL {
        let word = out[corner];
        let open = conducting(ops, word);
        let warms = ops.both(burning, open);
        let warmth = temperature(ops, word);
        let hotter = ops.add(warmth, flare);
        let hotter = smaller(ops, hotter, TOP);
        let warmed = with_temperature(ops, word, hotter);
        out[corner] = ops.select(warms, warmed, word);
    }
    for top in Corner::ALL {
        if top.side(Axis::Y) != 0 {
            continue;
        }
        let below = top.neighbour(Axis::Y);
        let burning = is_kind(ops, out[below], FIRE);
        let open = is_kind(ops, out[top], AIR);
        let smokes = ops.both(burning, open);
        let smoke = turned(ops, out[top], SMOKE, SMOKE_LIFE);
        out[top] = ops.select(smokes, smoke, out[top]);
    }
    out
}

/// The contact stage of one oct: fire and water in one oct put the fire
/// out, and air right above a fire turns to smoke.
#[must_use]
pub fn touched(cells: [u32; CORNERS], noise: u32) -> [u32; CORNERS] {
    touched_by(&mut Cpu, Octet(cells), noise).0
}

fn conducting<O: Ops>(ops: &mut O, word: O::Word) -> Flag<O::Word> {
    let held = kind(ops, word);
    let rock = ops.constant(ROCK);
    let open = ops.not_equal(held, rock);
    let fire = ops.constant(FIRE);
    let cold = ops.not_equal(held, fire);
    let kinds = ops.constant(KINDS);
    let known = ops.less(held, kinds);
    let open = ops.both(open, cold);
    ops.both(open, known)
}

/// The diffusion of one oct over [`Ops`].
fn diffused_by<O: Ops>(ops: &mut O, cells: Octet<O::Word>) -> Octet<O::Word> {
    let conducts = cells.map(|word| conducting(ops, word));
    let mut total = ops.constant(0);
    let mut warmest = ops.constant(0);
    let mut coldest = ops.constant(u32::MAX);
    for corner in Corner::ALL {
        let warmth = temperature(ops, cells[corner]);
        let held = ops.mul(warmth, conducts[corner].word());
        total = ops.add(total, held);
        let warmer = ops.greater(warmth, warmest);
        let warmer = ops.both(warmer, conducts[corner]);
        warmest = ops.select(warmer, warmth, warmest);
        let colder = ops.less(warmth, coldest);
        let colder = ops.both(colder, conducts[corner]);
        coldest = ops.select(colder, warmth, coldest);
    }
    let gap = ops.sub(warmest, coldest);
    let one = ops.constant(1);
    let uneven = ops.greater(gap, one);
    let count = ops.count(conducts.0);
    let share = ops.div(total, count);
    let spread = ops.mul(share, count);
    let rest = ops.sub(total, spread);

    let mut out = cells;
    let mut rank = ops.constant(0);
    for corner in Corner::ALL {
        let flag = conducts[corner];
        let extra = ops.greater(rest, rank);
        let warmth = ops.add(share, extra.word());
        let shared = with_temperature(ops, cells[corner], warmth);
        let flag = ops.both(flag, uneven);
        out[corner] = ops.select(flag, shared, cells[corner]);
        rank = ops.add(rank, conducts[corner].word());
    }
    out
}

/// The diffusion of one oct: the heat of its conducting cells shared out
/// evenly, the rest one degree each to the first of them in corner order.
#[must_use]
pub fn diffused(cells: [u32; CORNERS]) -> [u32; CORNERS] {
    diffused_by(&mut Cpu, Octet(cells)).0
}

/// Where `word`'s kind sits in the density order, with [`STILL`] for a
/// kind that does not move.
fn density<O: Ops>(ops: &mut O, word: O::Word) -> O::Word {
    let held = kind(ops, word);
    let mut out = ops.constant(STILL);
    for which in 0..KINDS {
        if let Some(place) = density_of(which) {
            let found = ops.is(held, which);
            let place = ops.constant(place);
            out = ops.select(found, place, out);
        }
    }
    out
}

/// The air cells under it that the cell `word` lets a grain fall past:
/// the count of air, none for any other kind.
fn depth<O: Ops>(ops: &mut O, word: O::Word) -> O::Word {
    let open = is_kind(ops, word, AIR);
    let count = ops.field(word, STATE_SHIFT, COUNT_BITS);
    let deepest = ops.constant(DEEPEST);
    let depth = ops.sub(deepest, count);
    let none = ops.constant(0);
    ops.select(open, depth, none)
}

/// The air a cell of `word`'s kind needs under the cell it falls into
/// along a diagonal: its steepness less one, none for water.
fn needs<O: Ops>(ops: &mut O, word: O::Word) -> O::Word {
    let held = kind(ops, word);
    let mut out = ops.constant(0);
    for which in GRANULAR {
        if let Some(steep) = steepness(which) {
            let found = ops.is(held, which);
            let need = ops.constant(steep - 1);
            out = ops.select(found, need, out);
        }
    }
    out
}

/// The cell `upper` falls into `lower` when it is heavier and moves; along
/// a diagonal only water and the granular kinds fall, and these only past
/// the air they need. The upper cell takes the lower word, and the lower
/// cell the upper word with its state cleared.
fn fall<O: Ops>(ops: &mut O, upper: O::Word, lower: O::Word, diagonal: bool) -> [O::Word; 2] {
    let above = density(ops, upper);
    let below = density(ops, lower);
    let heavier = ops.greater(above, below);
    let still = ops.constant(STILL);
    let loose = ops.less(above, still);
    let mut moving = ops.both(heavier, loose);
    if diagonal {
        let wet = ops.constant(WET);
        let falls = ops.greater_equal(above, wet);
        let room = depth(ops, lower);
        let need = needs(ops, upper);
        let enough = ops.greater_equal(room, need);
        moving = ops.both(moving, falls);
        moving = ops.both(moving, enough);
    }
    let landed = ops.masked(upper, !STATE_MASK);
    [
        ops.select(moving, lower, upper),
        ops.select(moving, landed, lower),
    ]
}

/// The axes the rule moves water along, in the x-first order.
const LEVEL: [Axis; 2] = [Axis::X, Axis::Z];

const fn plus(axis: Axis) -> u32 {
    match axis {
        Axis::Z => PLUS_Z,
        Axis::X | Axis::Y => PLUS_X,
    }
}

const fn minus(axis: Axis) -> u32 {
    match axis {
        Axis::Z => MINUS_Z,
        Axis::X | Axis::Y => MINUS_X,
    }
}

/// Whether `word` is water running along `way`.
fn runs<O: Ops>(ops: &mut O, word: O::Word, way: u32) -> Flag<O::Word> {
    let wet = is_kind(ops, word, WATER);
    let going = ops.field(word, STATE_SHIFT, RUNNING_BITS);
    let along = ops.is(going, way);
    ops.both(wet, along)
}

/// `word` with its held bit taken from the cell `under` it where it is
/// water: set when both are water, else cleared. Other kinds stay.
fn hold_by<O: Ops>(ops: &mut O, word: O::Word, under: O::Word) -> O::Word {
    let wet = is_kind(ops, word, WATER);
    let standing = is_kind(ops, under, WATER);
    let kept = ops.masked(word, !HELD);
    let bit = ops.constant(HELD);
    let set = ops.or(kept, bit);
    let both = ops.both(wet, standing);
    let water = ops.select(both, set, kept);
    ops.select(wet, water, word)
}

/// Whether `word` is ready to start running: idle held water with tired
/// steps to spare.
fn ready<O: Ops>(ops: &mut O, word: O::Word) -> Flag<O::Word> {
    let wet = is_kind(ops, word, WATER);
    let going = ops.field(word, STATE_SHIFT, RUNNING_BITS);
    let idle = ops.is(going, 0);
    let held = ops.masked(word, HELD);
    let held = ops.is(held, HELD);
    let worn = ops.masked(word, WORN);
    let patience = ops.constant(PATIENCE * TIRED);
    let spare = ops.less(worn, patience);
    let all = ops.both(wet, idle);
    let all = ops.both(all, held);
    ops.both(all, spare)
}

/// `word` started running along `way`: held, one more tired step.
fn started<O: Ops>(ops: &mut O, word: O::Word, way: u32) -> O::Word {
    let kept = ops.masked(word, !(RUNNING_MASK | HELD));
    let fixed = ops.constant((way << STATE_SHIFT) + HELD + TIRED);
    ops.add(kept, fixed)
}

/// `word` stopped running.
fn halted<O: Ops>(ops: &mut O, word: O::Word) -> O::Word {
    ops.masked(word, !RUNNING_MASK)
}

/// One pair of cells along `axis`, the lower first: runners trade places
/// with the air they run into, and stop at anything else.
fn along<O: Ops>(ops: &mut O, axis: Axis, [here, there]: [O::Word; 2]) -> [O::Word; 2] {
    let goes_up = runs(ops, here, plus(axis));
    let goes_down = runs(ops, there, minus(axis));
    let up_free = is_kind(ops, there, AIR);
    let down_free = is_kind(ops, here, AIR);
    let up_shut = ops.denied(up_free);
    let down_shut = ops.denied(down_free);

    let stops_here = ops.both(goes_up, up_shut);
    let stops_there = ops.both(goes_down, down_shut);
    let stilled_here = halted(ops, here);
    let stilled_there = halted(ops, there);
    let kept_here = ops.select(stops_here, stilled_here, here);
    let kept_there = ops.select(stops_there, stilled_there, there);

    let rises = ops.both(goes_up, up_free);
    let sinks = ops.both(goes_down, down_free);
    let swapping = ops.either(rises, sinks);
    [
        ops.select(swapping, kept_there, kept_here),
        ops.select(swapping, kept_here, kept_there),
    ]
}

/// The movement of an oct in the x-first order, over [`Ops`].
fn x_first_by<O: Ops>(ops: &mut O, oct: Octet<O::Word>) -> Octet<O::Word> {
    let [c0, c1, c2, c3, ..] = Corner::ALL;
    let upper = [c0, c1, c2, c3];
    let mut cells = oct;

    for top in upper {
        let under = top.neighbour(Axis::Y);
        cells[top] = hold_by(ops, cells[top], cells[under]);
    }

    let before = cells;
    for at in Corner::ALL {
        let word = before[at];
        let set = ready(ops, word);
        let open = LEVEL.map(|axis| {
            let free = is_kind(ops, before[at.neighbour(axis)], AIR);
            let way = if at.side(axis) == 0 {
                plus(axis)
            } else {
                minus(axis)
            };
            (free, started(ops, word, way))
        });
        let mut next = word;
        for (free, start) in open.into_iter().rev() {
            let goes = ops.both(set, free);
            next = ops.select(goes, start, next);
        }
        cells[at] = next;
    }

    for axis in LEVEL {
        for low in Corner::ALL {
            if low.side(axis) != 0 {
                continue;
            }
            let high = low.neighbour(axis);
            let [here, there] = along(ops, axis, [cells[low], cells[high]]);
            cells[low] = here;
            cells[high] = there;
        }
    }

    for top in upper {
        let bottom = top.neighbour(Axis::Y);
        let [up, down] = fall(ops, cells[top], cells[bottom], false);
        cells[top] = up;
        cells[bottom] = down;
    }
    let edges = LEVEL.map(|axis| upper.map(|top| (top, top.neighbour(axis).neighbour(Axis::Y))));
    let corner = upper.map(|top| {
        let across = top.neighbour(Axis::X).neighbour(Axis::Z);
        (top, across.neighbour(Axis::Y))
    });
    for (high, low) in edges.into_iter().flatten().chain(corner) {
        let [up, down] = fall(ops, cells[high], cells[low], true);
        cells[high] = up;
        cells[low] = down;
    }
    cells
}

/// The corners the two axis orders trade: those whose x and z bits differ.
const TRADED: [(usize, usize); 2] = [(1, 2), (5, 6)];

fn traded_places<O: Ops>(
    ops: &mut O,
    cells: Octet<O::Word>,
    mirrored: Flag<O::Word>,
) -> Octet<O::Word> {
    let mut out = cells;
    for (one, other) in TRADED {
        let (Some(one), Some(other)) = (Corner::new(one), Corner::new(other)) else {
            continue;
        };
        out[one] = ops.select(mirrored, cells[other], cells[one]);
        out[other] = ops.select(mirrored, cells[one], cells[other]);
    }
    out
}

/// `word` with its x and z ways traded where it is water and `mirrored`
/// holds. A running field whose two low bits differ names a way, and its
/// high bit picks z over x; the other running fields name no way and stay.
fn traded<O: Ops>(ops: &mut O, word: O::Word, mirrored: Flag<O::Word>) -> O::Word {
    let first = ops.shift_right(word, STATE_SHIFT);
    let second = ops.shift_right(word, STATE_SHIFT + 1);
    let differ = ops.xor(first, second);
    let differ = ops.masked(differ, 1);
    let flip = ops.shift_left(differ, STATE_SHIFT + 2);
    let flipped = ops.xor(word, flip);
    let wet = is_kind(ops, word, WATER);
    let trades = ops.both(mirrored, wet);
    ops.select(trades, flipped, word)
}

/// The movement of one oct over [`Ops`], z first where `z_first` holds.
fn moved_by<O: Ops>(ops: &mut O, oct: Octet<O::Word>, z_first: Flag<O::Word>) -> Octet<O::Word> {
    let cells = traded_places(ops, oct, z_first);
    let cells = cells.map(|cell| traded(ops, cell, z_first));
    let ruled = x_first_by(ops, cells);
    let back = ruled.map(|cell| traded(ops, cell, z_first));
    traded_places(ops, back, z_first)
}

/// The movement of one oct, in the x-first order or the z-first one.
#[must_use]
pub fn moved(cells: [u32; CORNERS], x_first: bool) -> [u32; CORNERS] {
    let ops = &mut Cpu;
    let z_first = ops.truth(!x_first);
    moved_by(ops, Octet(cells), z_first).0
}

/// The nibble of `noise` that belongs to `corner`.
fn nibble<O: Ops>(ops: &mut O, noise: O::Word, corner: Corner) -> O::Word {
    let by = u32::try_from(4 * corner.index()).unwrap_or(0);
    ops.field(noise, by, 4)
}

/// The wind of one oct over [`Ops`].
fn blown_by<O: Ops>(ops: &mut O, cells: Octet<O::Word>, noise: O::Word) -> Octet<O::Word> {
    let mut out = cells;
    for axis in LEVEL {
        for low in Corner::ALL {
            if low.side(axis) != 0 {
                continue;
            }
            let high = low.neighbour(axis);
            let (here, there) = (out[low], out[high]);
            let low_roll = nibble(ops, noise, low);
            let high_roll = nibble(ops, noise, high);
            let (up_wish, down_wish) = if axis == Axis::X {
                let with = ops.constant(WIND_WITH);
                let against = ops.constant(WIND_AGAINST);
                (ops.less(low_roll, with), ops.less(high_roll, against))
            } else {
                let across = ops.constant(ACROSS);
                (
                    ops.greater_equal(low_roll, across),
                    ops.greater_equal(high_roll, across),
                )
            };
            let smoke_here = is_kind(ops, here, SMOKE);
            let smoke_there = is_kind(ops, there, SMOKE);
            let air_here = is_kind(ops, here, AIR);
            let air_there = is_kind(ops, there, AIR);
            let up = ops.both(smoke_here, air_there);
            let up = ops.both(up, up_wish);
            let down = ops.both(smoke_there, air_here);
            let down = ops.both(down, down_wish);
            let trade = ops.either(up, down);
            out[low] = ops.select(trade, there, here);
            out[high] = ops.select(trade, here, there);
        }
    }
    out
}

/// The wind of one oct: smoke trades places with air beside it as its
/// nibble of `noise` says.
#[must_use]
pub fn blown(cells: [u32; CORNERS], noise: u32) -> [u32; CORNERS] {
    blown_by(&mut Cpu, Octet(cells), noise).0
}

/// The air counts of one oct over [`Ops`].
fn counted_by<O: Ops>(ops: &mut O, cells: Octet<O::Word>) -> Octet<O::Word> {
    let mut out = cells;
    for top in Corner::ALL {
        if top.side(Axis::Y) != 0 {
            continue;
        }
        let under = cells[top.neighbour(Axis::Y)];
        let open = is_kind(ops, under, AIR);
        let count = ops.field(under, STATE_SHIFT, COUNT_BITS);
        let one = ops.constant(1);
        let none = ops.constant(0);
        let empty = ops.is(count, 0);
        let less = ops.sub(count, one);
        let less = ops.select(empty, none, less);
        let shallow = ops.constant(DEEPEST);
        let count = ops.select(open, less, shallow);
        let placed = ops.shift_left(count, STATE_SHIFT);
        let kept = ops.masked(cells[top], !COUNT_MASK);
        let counted = ops.or(kept, placed);
        let air = is_kind(ops, cells[top], AIR);
        out[top] = ops.select(air, counted, cells[top]);
    }
    out
}

/// The air counts of one oct: air in an upper cell counts the air under
/// it.
#[must_use]
pub fn counted(cells: [u32; CORNERS]) -> [u32; CORNERS] {
    counted_by(&mut Cpu, Octet(cells)).0
}

/// One step of one oct over [`Ops`], without the tally.
fn stepped_by<O: Ops>(
    ops: &mut O,
    words: Octet<O::Word>,
    noise: O::Word,
    z_first: Flag<O::Word>,
) -> Octet<O::Word> {
    let mut cells = words;
    for corner in Corner::ALL {
        let roll = nibble(ops, noise, corner);
        cells[corner] = reacted_by(ops, cells[corner], roll);
    }
    let cells = touched_by(ops, cells, noise);
    let cells = diffused_by(ops, cells);
    let cells = moved_by(ops, cells, z_first);
    let cells = blown_by(ops, cells, noise);
    counted_by(ops, cells)
}

/// One step of one oct on the CPU, without the tally, given its noise and
/// its axis order.
#[must_use]
pub fn stepped(words: [u32; CORNERS], noise: u32, x_first: bool) -> [u32; CORNERS] {
    let ops = &mut Cpu;
    let z_first = ops.truth(!x_first);
    stepped_by(ops, Octet(words), noise, z_first).0
}

/// Nature, as the module describes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Nature;

impl Declared for Nature {
    fn params(&self) -> usize {
        0
    }

    fn tally(&self) -> Tally {
        Tally {
            counts: 4,
            weighted: true,
        }
    }
}

impl OneStep for Nature {
    fn one_step<O: Ops>(&self, ops: &mut O, oct: Oct<'_, O::Word>) -> Ruled<O::Word> {
        let noise = noise_by(ops, oct.at, oct.frame);
        let order = ops.masked(oct.frame, 2);
        let none = ops.constant(0);
        let z_first = ops.not_equal(order, none);
        let before = Octet(oct.words);
        let out = stepped_by(ops, before, noise, z_first);
        let changed = Octet::from_fn(|corner| ops.not_equal(before[corner], out[corner]));
        let kinds = out.map(|after| kind(ops, after));
        let fire = kinds.map(|held| ops.is(held, FIRE));
        let smoke = kinds.map(|held| ops.is(held, SMOKE));
        let water = kinds.map(|held| ops.is(held, WATER));
        let counts = vec![
            ops.count(changed.0),
            ops.count(fire.0),
            ops.count(smoke.0),
            ops.count(water.0),
        ];
        Ruled { out: out.0, counts }
    }
}
