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
//! that changed, the fire cells and the water cells after the step.

use simdr::lanes::{LaneError, Lanes, Predicate, U32};

use super::{CORNERS, Oct, Rule, Ruled, Word, noise, noise_emitted, param};

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

const fn with_kind(word: u32, kind: u32) -> u32 {
    (word & !KIND_MASK) | kind
}

const fn with_life(word: u32, life: u32) -> u32 {
    (word & !LIFE_MASK) | (life << LIFE_SHIFT)
}

const fn with_temperature(word: u32, temperature: u32) -> u32 {
    (word & KEPT_MASK) | (temperature << HEAT_SHIFT)
}

/// Where a kind sits in the density order, lightest first, or `None` for a
/// kind that does not move.
const fn density(kind: u32) -> Option<u32> {
    match kind {
        STEAM => Some(0),
        AIR => Some(1),
        WATER => Some(2),
        _ => None,
    }
}

/// The reaction of one cell, `nibble` its four bits of the noise.
#[must_use]
pub fn reacted(word: u32, nibble: u32, params: &[u32]) -> u32 {
    let (ignite, chance, flare) = (param(params, 0), param(params, 1), param(params, 2));
    let temperature = temperature_of(word);
    match kind_of(word) {
        ICE if temperature >= MELT => with_kind(word, WATER),
        WATER if temperature >= BOIL => with_kind(word, STEAM),
        STEAM if temperature < CONDENSE => with_kind(word, WATER),
        FUEL if temperature >= ignite => {
            let sure = temperature - ignite >= WINDOW || temperature == TOP;
            if sure || nibble < chance {
                with_life(with_kind(word, FIRE), LIFE)
            } else {
                with_temperature(word, temperature + 1)
            }
        }
        FIRE => {
            let hotter = (temperature + flare.min(TOP)).min(TOP);
            let warmed = with_temperature(word, hotter);
            match life_of(word) {
                0 => with_kind(warmed, ASH),
                life => with_life(warmed, life - 1),
            }
        }
        _ => word,
    }
}

/// The diffusion of one oct: the heat of its conducting cells shared out
/// evenly, the rest one degree each to the first of them in corner order.
#[must_use]
pub fn diffused(cells: [u32; CORNERS]) -> [u32; CORNERS] {
    let mut total = 0;
    let mut count = 0;
    for &word in &cells {
        if conducts(kind_of(word)) {
            total += temperature_of(word);
            count += 1;
        }
    }
    let count = u32::max(count, 1);
    let share = total / count;
    let rest = total - share * count;
    let mut out = cells;
    let mut rank = 0;
    for word in &mut out {
        if conducts(kind_of(*word)) {
            *word = with_temperature(*word, share + u32::from(rank < rest));
            rank += 1;
        }
    }
    out
}

/// The movement of one oct: each vertical pair trades a lighter cell under
/// a heavier one.
#[must_use]
pub fn moved(cells: [u32; CORNERS]) -> [u32; CORNERS] {
    let mut out = cells;
    for top in 0..CORNERS / 2 {
        let below = top + CORNERS / 2;
        let (Some(&upper), Some(&lower)) = (out.get(top), out.get(below)) else {
            continue;
        };
        if let (Some(over), Some(under)) = (density(kind_of(upper)), density(kind_of(lower)))
            && over > under
        {
            out.swap(top, below);
        }
    }
    out
}

/// One step of one oct on the CPU, without the tally: reactions, diffusion
/// and movement.
#[must_use]
pub fn stepped(words: [u32; CORNERS], noise: u32, params: &[u32]) -> [u32; CORNERS] {
    let mut cells = words;
    for (corner, word) in cells.iter_mut().enumerate() {
        let nibble = (noise >> (4 * corner)) & 0xF;
        *word = reacted(*word, nibble, params);
    }
    moved(diffused(cells))
}

/// Heat with phase changes, as the module describes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Heat;

impl Rule for Heat {
    fn params(&self) -> usize {
        PARAMS
    }

    fn step(&self, oct: Oct<'_, u32>) -> Ruled<u32> {
        let out = stepped(oct.words, noise(oct.at, oct.frame), oct.params);
        let mut counts = [0; 3];
        for (before, after) in oct.words.iter().zip(&out) {
            let found = [
                before != after,
                kind_of(*after) == FIRE,
                kind_of(*after) == WATER,
            ];
            for (count, add) in counts.iter_mut().zip(found) {
                *count += u32::from(add);
            }
        }
        Ruled { out, counts }
    }

    fn emit<const LANES: u32>(
        &self,
        lanes: &mut Lanes<'_>,
        oct: Oct<'_, Word<LANES>>,
    ) -> Result<Ruled<Word<LANES>>, LaneError> {
        let mut emit = Emitting::<LANES>::new(lanes)?;
        let none = emit.none;
        let params = [0, 1, 2].map(|which| oct.params.get(which).copied().unwrap_or(none));
        let noise = noise_emitted::<LANES>(emit.lanes, oct.at, oct.frame)?;

        let mut cells = oct.words;
        for (corner, word) in cells.iter_mut().enumerate() {
            let by = emit.bits(u32::try_from(4 * corner).unwrap_or(0))?;
            let shifted = emit.lanes.shift_right_logical(noise, by)?;
            let nibble = emit.masked(shifted, 0xF)?;
            *word = emit.reacted(*word, nibble, params)?;
        }
        let cells = emit.diffused(cells)?;
        let out = emit.moved(cells)?;

        let mut counts = [none; 3];
        for (before, after) in oct.words.into_iter().zip(out) {
            let same = emit.lanes.equal(before, after)?;
            let changed = emit.lanes.select(same, emit.none, emit.one)?;
            let kind = emit.masked(after, KIND_MASK)?;
            let fire = emit.is(kind, FIRE)?;
            let water = emit.is(kind, WATER)?;
            for (count, add) in counts.iter_mut().zip([changed, fire, water]) {
                *count = emit.lanes.add(*count, add)?;
            }
        }
        Ok(Ruled { out, counts })
    }
}

/// The heat rule's stages, emitted. Flags are words of 0 or 1.
struct Emitting<'held, 'module, const LANES: u32> {
    lanes: &'held mut Lanes<'module>,
    one: Word<LANES>,
    none: Word<LANES>,
}

impl<'held, 'module, const LANES: u32> Emitting<'held, 'module, LANES> {
    fn new(lanes: &'held mut Lanes<'module>) -> Result<Self, LaneError> {
        let one = lanes.splat_bits::<U32, LANES>(1)?;
        let none = lanes.splat_bits::<U32, LANES>(0)?;
        Ok(Self { lanes, one, none })
    }

    fn bits(&mut self, value: u32) -> Result<Word<LANES>, LaneError> {
        self.lanes.splat_bits::<U32, LANES>(value)
    }

    fn masked(&mut self, word: Word<LANES>, mask: u32) -> Result<Word<LANES>, LaneError> {
        let mask = self.bits(mask)?;
        self.lanes.and(word, mask)
    }

    fn flag(&mut self, truth: Predicate<LANES>) -> Result<Word<LANES>, LaneError> {
        self.lanes.select(truth, self.one, self.none)
    }

    fn truth(&mut self, flag: Word<LANES>) -> Result<Predicate<LANES>, LaneError> {
        self.lanes.not_equal(flag, self.none)
    }

    fn is(&mut self, value: Word<LANES>, want: u32) -> Result<Word<LANES>, LaneError> {
        let want = self.bits(want)?;
        let same = self.lanes.equal(value, want)?;
        self.flag(same)
    }

    fn at_least(
        &mut self,
        value: Word<LANES>,
        floor: Word<LANES>,
    ) -> Result<Word<LANES>, LaneError> {
        let above = self.lanes.greater_equal(value, floor)?;
        self.flag(above)
    }

    fn both(&mut self, left: Word<LANES>, right: Word<LANES>) -> Result<Word<LANES>, LaneError> {
        self.lanes.and(left, right)
    }

    fn pick(
        &mut self,
        flag: Word<LANES>,
        then: Word<LANES>,
        otherwise: Word<LANES>,
    ) -> Result<Word<LANES>, LaneError> {
        let truth = self.truth(flag)?;
        self.lanes.select(truth, then, otherwise)
    }

    fn with_kind(&mut self, word: Word<LANES>, kind: u32) -> Result<Word<LANES>, LaneError> {
        let kept = self.masked(word, !KIND_MASK)?;
        let kind = self.bits(kind)?;
        self.lanes.or(kept, kind)
    }

    fn with_temperature(
        &mut self,
        word: Word<LANES>,
        temperature: Word<LANES>,
    ) -> Result<Word<LANES>, LaneError> {
        let kept = self.masked(word, KEPT_MASK)?;
        let by = self.bits(HEAT_SHIFT)?;
        let placed = self.lanes.shift_left(temperature, by)?;
        self.lanes.or(kept, placed)
    }

    fn smaller(&mut self, value: Word<LANES>, cap: u32) -> Result<Word<LANES>, LaneError> {
        let cap = self.bits(cap)?;
        let over = self.lanes.greater_than(value, cap)?;
        self.lanes.select(over, cap, value)
    }

    /// [`reacted`].
    fn reacted(
        &mut self,
        word: Word<LANES>,
        nibble: Word<LANES>,
        [ignite, chance, flare]: [Word<LANES>; PARAMS],
    ) -> Result<Word<LANES>, LaneError> {
        let kind = self.masked(word, KIND_MASK)?;
        let by = self.bits(HEAT_SHIFT)?;
        let temperature = self.lanes.shift_right_logical(word, by)?;
        let mut out = word;

        let ice = self.is(kind, ICE)?;
        let melt = self.bits(MELT)?;
        let warm = self.at_least(temperature, melt)?;
        let melts = self.both(ice, warm)?;
        let melted = self.with_kind(word, WATER)?;
        out = self.pick(melts, melted, out)?;

        let water = self.is(kind, WATER)?;
        let boil = self.bits(BOIL)?;
        let hot = self.at_least(temperature, boil)?;
        let boils = self.both(water, hot)?;
        let boiled = self.with_kind(word, STEAM)?;
        out = self.pick(boils, boiled, out)?;

        let steam = self.is(kind, STEAM)?;
        let condense = self.bits(CONDENSE)?;
        let warm = self.at_least(temperature, condense)?;
        let cold = self.lanes.xor(warm, self.one)?;
        let condenses = self.both(steam, cold)?;
        let condensed = self.with_kind(word, WATER)?;
        out = self.pick(condenses, condensed, out)?;

        let fuel = self.is(kind, FUEL)?;
        let warm = self.at_least(temperature, ignite)?;
        let hot = self.both(fuel, warm)?;
        let past = self.lanes.sub(temperature, ignite)?;
        let window = self.bits(WINDOW)?;
        let late = self.at_least(past, window)?;
        let full = self.is(temperature, TOP)?;
        let sure = self.lanes.or(late, full)?;
        let lucky = self.lanes.greater_than(chance, nibble)?;
        let lucky = self.flag(lucky)?;
        let catches = self.lanes.or(sure, lucky)?;
        let ignites = self.both(hot, catches)?;
        let waits = self.lanes.xor(catches, self.one)?;
        let warms = self.both(hot, waits)?;
        let kept = self.masked(word, !(KIND_MASK | LIFE_MASK))?;
        let lit = self.bits(FIRE | (LIFE << LIFE_SHIFT))?;
        let lit = self.lanes.or(kept, lit)?;
        out = self.pick(ignites, lit, out)?;
        let degree = self.bits(1 << HEAT_SHIFT)?;
        let warmer = self.lanes.add(word, degree)?;
        out = self.pick(warms, warmer, out)?;

        let fire = self.is(kind, FIRE)?;
        let gain = self.smaller(flare, TOP)?;
        let hotter = self.lanes.add(temperature, gain)?;
        let hotter = self.smaller(hotter, TOP)?;
        let warmed = self.with_temperature(word, hotter)?;
        let life = self.masked(word, LIFE_MASK)?;
        let spent = self.is(life, 0)?;
        let ash = self.with_kind(warmed, ASH)?;
        let step = self.bits(1 << LIFE_SHIFT)?;
        let burning = self.lanes.sub(warmed, step)?;
        let burnt = self.pick(spent, ash, burning)?;
        self.pick(fire, burnt, out)
    }

    /// [`diffused`].
    fn diffused(
        &mut self,
        cells: [Word<LANES>; CORNERS],
    ) -> Result<[Word<LANES>; CORNERS], LaneError> {
        let by = self.bits(HEAT_SHIFT)?;
        let wall = self.bits(WALL)?;
        let kinds = self.bits(KINDS)?;
        let mut conducting = [self.none; CORNERS];
        let mut total = self.none;
        let mut count = self.none;
        for (word, flag) in cells.iter().zip(&mut conducting) {
            let kind = self.masked(*word, KIND_MASK)?;
            let open = self.lanes.not_equal(kind, wall)?;
            let open = self.flag(open)?;
            let known = self.lanes.greater_than(kinds, kind)?;
            let known = self.flag(known)?;
            *flag = self.both(open, known)?;
            let warmth = self.lanes.shift_right_logical(*word, by)?;
            let held = self.lanes.mul(warmth, *flag)?;
            total = self.lanes.add(total, held)?;
            count = self.lanes.add(count, *flag)?;
        }
        let empty = self.is(count, 0)?;
        let count = self.pick(empty, self.one, count)?;
        let share = self.lanes.div(total, count)?;
        let spread = self.lanes.mul(share, count)?;
        let rest = self.lanes.sub(total, spread)?;

        let mut out = cells;
        let mut rank = self.none;
        for (word, flag) in out.iter_mut().zip(conducting) {
            let extra = self.lanes.greater_than(rest, rank)?;
            let extra = self.flag(extra)?;
            let temperature = self.lanes.add(share, extra)?;
            let shared = self.with_temperature(*word, temperature)?;
            *word = self.pick(flag, shared, *word)?;
            rank = self.lanes.add(rank, flag)?;
        }
        Ok(out)
    }

    /// Where `kind` sits in the density order, with 3 for a kind that does
    /// not move.
    fn density(&mut self, kind: Word<LANES>) -> Result<Word<LANES>, LaneError> {
        let mut out = self.bits(3)?;
        for (which, place) in [(STEAM, 0), (AIR, 1), (WATER, 2)] {
            let found = self.is(kind, which)?;
            let place = self.bits(place)?;
            out = self.pick(found, place, out)?;
        }
        Ok(out)
    }

    /// [`moved`].
    fn moved(
        &mut self,
        cells: [Word<LANES>; CORNERS],
    ) -> Result<[Word<LANES>; CORNERS], LaneError> {
        let still = self.bits(3)?;
        let mut out = cells;
        for top in 0..CORNERS / 2 {
            let below = top + CORNERS / 2;
            let (Some(&upper), Some(&lower)) = (out.get(top), out.get(below)) else {
                continue;
            };
            let over = self.masked(upper, KIND_MASK)?;
            let over = self.density(over)?;
            let under = self.masked(lower, KIND_MASK)?;
            let under = self.density(under)?;
            let moves = self.lanes.greater_than(still, over)?;
            let moves = self.flag(moves)?;
            let can = self.lanes.greater_than(still, under)?;
            let can = self.flag(can)?;
            let heavier = self.lanes.greater_than(over, under)?;
            let heavier = self.flag(heavier)?;
            let trade = self.both(moves, can)?;
            let trade = self.both(trade, heavier)?;
            let up = self.pick(trade, lower, upper)?;
            let down = self.pick(trade, upper, lower)?;
            if let Some(slot) = out.get_mut(top) {
                *slot = up;
            }
            if let Some(slot) = out.get_mut(below) {
                *slot = down;
            }
        }
        Ok(out)
    }
}
