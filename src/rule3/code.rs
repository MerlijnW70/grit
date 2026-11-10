//! Cellular code, a rule of the 3D engine: each cell holds an instruction
//! and a value, and acts on its neighbours in the oct.
//!
//! A cell word holds:
//!
//! | bits | field |
//! |---|---|
//! | 0 to 23 | the payload |
//! | 24 to 27 | the instruction: [`NOP`], [`DATA`], [`MOVE`], [`COPY`], [`ADD`], [`TURN`], [`HALT`]; 7 to 15 are inert |
//! | 28 to 30 | the way: [`PLUS_X`], [`MINUS_X`], [`PLUS_Y`], [`MINUS_Y`], [`PLUS_Z`], [`MINUS_Z`]; 6 and 7 are no way |
//! | 31 | kept as it is |
//!
//! Bits 24 to 31 are the opcode byte. A word with instruction [`NOP`] is an
//! empty cell, whatever its other bits.
//!
//! A cell faces the neighbour one cell along its way when that neighbour is
//! in the same oct. Corner `k` of an oct lies `k & 1` along x, `k >> 2`
//! along y and `(k >> 1) & 1` along z, so a cell faces along `+` from the
//! low side of an axis and along `-` from the high side. Since the octs
//! shift by one cell on every axis each frame, a cell that stays put faces
//! along `+` and `-` of its axis in turn.
//!
//! One step of an oct runs three stages in order:
//!
//! 1. **Turn.** A [`MOVE`] cell next to a [`TURN`] cell in the oct takes the
//!    turn's way. With more than one turn around it, the first in
//!    [`PRIORITY`] decides.
//! 2. **Claims**, each read from the words after the turn stage:
//!    - a [`MOVE`] cell claims the empty cell it faces for its own word;
//!    - a [`COPY`] cell claims every empty cell next to it in the oct, one
//!      on each axis, for its own word, whatever its way;
//!    - an [`ADD`] cell claims the [`DATA`] cell it faces for its own word
//!      with the payload of the data added in, modulo 2 to the 24.
//! 3. **Resolution.** A cell claimed by more than one neighbour takes the
//!    claim of the neighbour along the first axis of [`PRIORITY`] that
//!    claims it: x, then y, then z. A [`MOVE`] or [`ADD`] cell whose claim
//!    won leaves an empty cell, the word 0, behind; one whose claim lost
//!    stays as it is. A [`MOVE`] cell that faces a [`HALT`] cell becomes
//!    [`DATA`] with its payload and its other bits.
//!
//! So a [`MOVE`] packet carries its payload one cell a frame along its way
//! through empty cells, after at most one frame of waiting for an oct in
//! which it faces that way; an [`ADD`] cell walks along a row of [`DATA`]
//! and sums it; a [`COPY`] cell fills the empty cells it can reach through
//! faces; [`DATA`], [`HALT`], [`TURN`], [`NOP`] and the inert instructions
//! never act. Only empty cells and [`DATA`] cells are ever claimed, and only
//! [`MOVE`], [`COPY`] and [`ADD`] cells claim, so no cell is written twice.
//!
//! The step is a function of the eight words alone: it reads neither the
//! frame, nor the oct's point, nor the noise, nor any parameter. So an oct
//! the step leaves unchanged stays unchanged at every later frame, which is
//! the quiet contract of [`crate::rule3`]. A world of only [`DATA`] and
//! [`HALT`] cells has no cell that claims and is quiet from its first frame.
//!
//! Its tally words are the cells that changed, the [`MOVE`] cells and the
//! [`DATA`] cells after the step.

use simdr::lanes::{LaneError, Lanes, Predicate, U32};

use super::{CORNERS, Oct, Rule, Ruled, Word};

pub const NOP: u32 = 0;
pub const DATA: u32 = 1;
pub const MOVE: u32 = 2;
pub const COPY: u32 = 3;
pub const ADD: u32 = 4;
pub const TURN: u32 = 5;
pub const HALT: u32 = 6;

/// The instructions the rule knows; one at or past it is inert.
pub const INSTRUCTIONS: u32 = 7;

pub const PLUS_X: u32 = 0;
pub const MINUS_X: u32 = 1;
pub const PLUS_Y: u32 = 2;
pub const MINUS_Y: u32 = 3;
pub const PLUS_Z: u32 = 4;
pub const MINUS_Z: u32 = 5;

/// The ways that name an axis; a way at or past it is no way.
pub const WAYS: u32 = 6;

pub const PAYLOAD: u32 = (1 << 24) - 1;
const INSTRUCTION_SHIFT: u32 = 24;
const INSTRUCTION_MASK: u32 = 0xF << INSTRUCTION_SHIFT;
const WAY_SHIFT: u32 = 28;
const WAY_MASK: u32 = 0x7 << WAY_SHIFT;

/// The axes, x, y and z, in the order a conflict is resolved: the claim of
/// the neighbour along the first of them wins.
pub const PRIORITY: [usize; 3] = [0, 1, 2];

/// The bit of a corner index that is its place along each axis, x, y and z.
const AXIS_BIT: [usize; 3] = [1, 4, 2];

/// A cell of `instruction` facing `way`, holding `payload` cut to 24 bits.
#[must_use]
pub const fn word(instruction: u32, way: u32, payload: u32) -> u32 {
    ((instruction << INSTRUCTION_SHIFT) & INSTRUCTION_MASK)
        | ((way << WAY_SHIFT) & WAY_MASK)
        | (payload & PAYLOAD)
}

#[must_use]
pub const fn instruction_of(word: u32) -> u32 {
    (word & INSTRUCTION_MASK) >> INSTRUCTION_SHIFT
}

#[must_use]
pub const fn way_of(word: u32) -> u32 {
    (word & WAY_MASK) >> WAY_SHIFT
}

#[must_use]
pub const fn payload_of(word: u32) -> u32 {
    word & PAYLOAD
}

const fn with_instruction(word: u32, instruction: u32) -> u32 {
    (word & !INSTRUCTION_MASK) | (instruction << INSTRUCTION_SHIFT)
}

fn bit_of(axis: usize) -> usize {
    AXIS_BIT.get(axis).copied().unwrap_or(0)
}

/// The way along `axis` that a cell at `corner` faces a neighbour in the oct.
fn facing(corner: usize, axis: usize) -> u32 {
    let high = u32::from(corner & bit_of(axis) != 0);
    u32::try_from(2 * axis).unwrap_or(0) + high
}

fn at<T: Copy, const N: usize>(cells: &[T; N], which: usize, or: T) -> T {
    cells.get(which).copied().unwrap_or(or)
}

fn put<T>(cells: &mut [T; CORNERS], corner: usize, value: T) {
    if let Some(slot) = cells.get_mut(corner) {
        *slot = value;
    }
}

/// The turn stage: each [`MOVE`] cell takes the way of the first [`TURN`]
/// cell next to it in [`PRIORITY`].
#[must_use]
pub fn turned(words: [u32; CORNERS]) -> [u32; CORNERS] {
    let mut out = words;
    for (corner, &cell) in words.iter().enumerate() {
        if instruction_of(cell) != MOVE {
            continue;
        }
        for axis in PRIORITY {
            let next = at(&words, corner ^ bit_of(axis), 0);
            if instruction_of(next) == TURN {
                put(&mut out, corner, (cell & !WAY_MASK) | (next & WAY_MASK));
                break;
            }
        }
    }
    out
}

/// The word the cell `source`, at corner `from`, claims its neighbour
/// `target` along `axis` for, if it claims it.
#[must_use]
pub fn claim(source: u32, from: usize, target: u32, axis: usize) -> Option<u32> {
    let faces = way_of(source) == facing(from, axis);
    match (instruction_of(source), instruction_of(target)) {
        (MOVE, NOP) if faces => Some(source),
        (COPY, NOP) => Some(source),
        (ADD, DATA) if faces => Some((source & !PAYLOAD) | (source.wrapping_add(target) & PAYLOAD)),
        _ => None,
    }
}

/// One step of one oct on the CPU, without the tally: turn, claims and
/// resolution.
#[must_use]
pub fn stepped(words: [u32; CORNERS]) -> [u32; CORNERS] {
    let cells = turned(words);
    let mut out = cells;
    for (target, &held) in cells.iter().enumerate() {
        for axis in PRIORITY {
            let from = target ^ bit_of(axis);
            let source = at(&cells, from, 0);
            if let Some(claimed) = claim(source, from, held, axis) {
                put(&mut out, target, claimed);
                if matches!(instruction_of(source), MOVE | ADD) {
                    put(&mut out, from, word(NOP, 0, 0));
                }
                break;
            }
        }
    }
    for (corner, &cell) in cells.iter().enumerate() {
        if instruction_of(cell) != MOVE {
            continue;
        }
        for axis in PRIORITY {
            let next = at(&cells, corner ^ bit_of(axis), 0);
            if way_of(cell) == facing(corner, axis) && instruction_of(next) == HALT {
                put(&mut out, corner, with_instruction(cell, DATA));
            }
        }
    }
    out
}

/// Cellular code, as the module describes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Code;

impl Rule for Code {
    fn params(&self) -> usize {
        0
    }

    fn step(&self, oct: Oct<'_, u32>) -> Ruled<u32> {
        let out = stepped(oct.words);
        let mut counts = [0; 3];
        for (before, after) in oct.words.iter().zip(&out) {
            let found = [
                before != after,
                instruction_of(*after) == MOVE,
                instruction_of(*after) == DATA,
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
        let out = emit.stepped(oct.words)?;
        let none = emit.none;
        let mut counts = [none; 3];
        for (before, after) in oct.words.into_iter().zip(out) {
            let same = emit.lanes.equal(before, after)?;
            let changed = emit.lanes.select(same, emit.none, emit.one)?;
            let instruction = emit.instruction(after)?;
            let moving = emit.is(instruction, MOVE)?;
            let data = emit.is(instruction, DATA)?;
            for (count, add) in counts.iter_mut().zip([changed, moving, data]) {
                *count = emit.lanes.add(*count, add)?;
            }
        }
        Ok(Ruled { out, counts })
    }
}

/// The code rule's stages, emitted. Flags are words of 0 or 1.
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

    fn is(&mut self, value: Word<LANES>, want: u32) -> Result<Word<LANES>, LaneError> {
        let want = self.bits(want)?;
        let same = self.lanes.equal(value, want)?;
        self.flag(same)
    }

    fn pick(
        &mut self,
        flag: Word<LANES>,
        then: Word<LANES>,
        otherwise: Word<LANES>,
    ) -> Result<Word<LANES>, LaneError> {
        let truth = self.lanes.not_equal(flag, self.none)?;
        self.lanes.select(truth, then, otherwise)
    }

    fn instruction(&mut self, word: Word<LANES>) -> Result<Word<LANES>, LaneError> {
        let by = self.bits(INSTRUCTION_SHIFT)?;
        let shifted = self.lanes.shift_right_logical(word, by)?;
        self.masked(shifted, 0xF)
    }

    fn way(&mut self, word: Word<LANES>) -> Result<Word<LANES>, LaneError> {
        let by = self.bits(WAY_SHIFT)?;
        let shifted = self.lanes.shift_right_logical(word, by)?;
        self.masked(shifted, 0x7)
    }

    /// [`turned`].
    fn turned(
        &mut self,
        words: [Word<LANES>; CORNERS],
        instructions: [Word<LANES>; CORNERS],
    ) -> Result<[Word<LANES>; CORNERS], LaneError> {
        let none = self.none;
        let mut out = words;
        for corner in 0..CORNERS {
            let cell = at(&words, corner, none);
            let moving = self.is(at(&instructions, corner, none), MOVE)?;
            let kept = self.masked(cell, !WAY_MASK)?;
            let mut held = cell;
            for axis in PRIORITY.into_iter().rev() {
                let near = corner ^ bit_of(axis);
                let turn = self.is(at(&instructions, near, none), TURN)?;
                let turns = self.lanes.and(moving, turn)?;
                let way = self.masked(at(&words, near, none), WAY_MASK)?;
                let taken = self.lanes.or(kept, way)?;
                held = self.pick(turns, taken, held)?;
            }
            put(&mut out, corner, held);
        }
        Ok(out)
    }

    /// [`stepped`].
    fn stepped(
        &mut self,
        words: [Word<LANES>; CORNERS],
    ) -> Result<[Word<LANES>; CORNERS], LaneError> {
        let none = self.none;
        let mut instructions = [none; CORNERS];
        for (instruction, &word) in instructions.iter_mut().zip(&words) {
            *instruction = self.instruction(word)?;
        }
        let cells = self.turned(words, instructions)?;
        let mut ways = [none; CORNERS];
        for (way, &cell) in ways.iter_mut().zip(&cells) {
            *way = self.way(cell)?;
        }
        let mut faces = [[none; 3]; CORNERS];
        for (corner, facing_axes) in faces.iter_mut().enumerate() {
            for (axis, flag) in facing_axes.iter_mut().enumerate() {
                *flag = self.is(at(&ways, corner, none), facing(corner, axis))?;
            }
        }
        let mut moving = [none; CORNERS];
        let mut copying = [none; CORNERS];
        let mut adding = [none; CORNERS];
        let mut empty = [none; CORNERS];
        let mut data = [none; CORNERS];
        let mut halting = [none; CORNERS];
        for corner in 0..CORNERS {
            let instruction = at(&instructions, corner, none);
            put(&mut moving, corner, self.is(instruction, MOVE)?);
            put(&mut copying, corner, self.is(instruction, COPY)?);
            put(&mut adding, corner, self.is(instruction, ADD)?);
            put(&mut empty, corner, self.is(instruction, NOP)?);
            put(&mut data, corner, self.is(instruction, DATA)?);
            put(&mut halting, corner, self.is(instruction, HALT)?);
        }

        let mut out = cells;
        let mut won = [[none; 3]; CORNERS];
        for target in 0..CORNERS {
            let held = at(&cells, target, none);
            let mut taken = none;
            for axis in PRIORITY {
                let from = target ^ bit_of(axis);
                let source = at(&cells, from, none);
                let faced = at(&faces, from, [none; 3]);
                let faced = at(&faced, axis, none);
                let walks = self.lanes.and(at(&moving, from, none), faced)?;
                let spreads = self.lanes.or(walks, at(&copying, from, none))?;
                let into_empty = self.lanes.and(spreads, at(&empty, target, none))?;
                let sums = self.lanes.and(at(&adding, from, none), faced)?;
                let into_data = self.lanes.and(sums, at(&data, target, none))?;
                let claims = self.lanes.or(into_empty, into_data)?;
                let free = self.lanes.xor(taken, self.one)?;
                let wins = self.lanes.and(claims, free)?;
                taken = self.lanes.or(taken, claims)?;
                let total = self.lanes.add(source, held)?;
                let total = self.masked(total, PAYLOAD)?;
                let kept = self.masked(source, !PAYLOAD)?;
                let added = self.lanes.or(kept, total)?;
                let claimed = self.pick(sums, added, source)?;
                let now = self.pick(wins, claimed, at(&out, target, none))?;
                put(&mut out, target, now);
                if let Some(row) = won.get_mut(target)
                    && let Some(slot) = row.get_mut(axis)
                {
                    *slot = wins;
                }
            }
        }

        let gone_word = self.bits(word(NOP, 0, 0))?;
        let stopped = self.bits(DATA << INSTRUCTION_SHIFT)?;
        for corner in 0..CORNERS {
            let cell = at(&cells, corner, none);
            let walker = self
                .lanes
                .or(at(&moving, corner, none), at(&adding, corner, none))?;
            let faced = at(&faces, corner, [none; 3]);
            let mut now = at(&out, corner, none);
            for axis in PRIORITY {
                let near = corner ^ bit_of(axis);
                let wins = at(&at(&won, near, [none; 3]), axis, none);
                let gone = self.lanes.and(wins, walker)?;
                now = self.pick(gone, gone_word, now)?;
                let blocked = self
                    .lanes
                    .and(at(&moving, corner, none), at(&faced, axis, none))?;
                let blocked = self.lanes.and(blocked, at(&halting, near, none))?;
                let kept = self.masked(cell, !INSTRUCTION_MASK)?;
                let halted = self.lanes.or(kept, stopped)?;
                now = self.pick(blocked, halted, now)?;
            }
            put(&mut out, corner, now);
        }
        Ok(out)
    }
}
