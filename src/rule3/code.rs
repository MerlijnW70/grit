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
//! The rule is one [`OneStep`] over [`Ops`]: the CPU functions below run
//! that step on `u32`. Its tally words are the cells that changed, the
//! [`MOVE`] cells and the [`DATA`] cells after the step, the first weighted.

use super::{Axis, CORNERS, Corner, Declared, Oct, Octet, OneStep, Ruled, Tally};
use crate::ops::{Cpu, Flag, Helpers, Ops};

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
const INSTRUCTION_BITS: u32 = 4;
const INSTRUCTION_MASK: u32 = 0xF << INSTRUCTION_SHIFT;
const WAY_SHIFT: u32 = 28;
const WAY_BITS: u32 = 3;
const WAY_MASK: u32 = 0x7 << WAY_SHIFT;

/// The axes, x, y and z, in the order a conflict is resolved: the claim of
/// the neighbour along the first of them wins.
pub const PRIORITY: [Axis; 3] = Axis::ALL;

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

/// The way along `axis` that a cell at `corner` faces a neighbour in the
/// oct.
const fn facing(corner: Corner, axis: Axis) -> u32 {
    2 * axis.index() as u32 + corner.side(axis)
}

fn instruction<O: Ops>(ops: &mut O, word: O::Word) -> O::Word {
    ops.field(word, INSTRUCTION_SHIFT, INSTRUCTION_BITS)
}

fn way<O: Ops>(ops: &mut O, word: O::Word) -> O::Word {
    ops.field(word, WAY_SHIFT, WAY_BITS)
}

/// The turn stage over [`Ops`]; `instructions` are those of `words`. The
/// turns are taken last axis first, so the first in [`PRIORITY`] stands.
fn turned_by<O: Ops>(
    ops: &mut O,
    words: Octet<O::Word>,
    instructions: Octet<O::Word>,
) -> Octet<O::Word> {
    let mut out = words;
    for corner in Corner::ALL {
        let cell = words[corner];
        let moving = ops.is(instructions[corner], MOVE);
        let kept = ops.masked(cell, !WAY_MASK);
        let mut held = cell;
        for axis in PRIORITY.into_iter().rev() {
            let near = corner.neighbour(axis);
            let turn = ops.is(instructions[near], TURN);
            let turns = ops.both(moving, turn);
            let way = ops.masked(words[near], WAY_MASK);
            let taken = ops.or(kept, way);
            held = ops.select(turns, taken, held);
        }
        out[corner] = held;
    }
    out
}

/// The turn stage: each [`MOVE`] cell takes the way of the first [`TURN`]
/// cell next to it in [`PRIORITY`].
#[must_use]
pub fn turned(words: [u32; CORNERS]) -> [u32; CORNERS] {
    let words = Octet(words);
    turned_by(&mut Cpu, words, words.map(instruction_of)).0
}

/// A cell of an oct after the turn stage, with what it is: flags of its
/// instruction, and whether it faces its neighbour along each axis.
#[derive(Clone, Copy)]
struct Cell<W> {
    word: W,
    moving: Flag<W>,
    copying: Flag<W>,
    adding: Flag<W>,
    empty: Flag<W>,
    data: Flag<W>,
    halting: Flag<W>,
    faces: [Flag<W>; 3],
}

/// `word`, of instruction `does`, at `corner`, as a [`Cell`].
fn cell_by<O: Ops>(ops: &mut O, word: O::Word, does: O::Word, corner: Corner) -> Cell<O::Word> {
    let pointed = way(ops, word);
    Cell {
        word,
        moving: ops.is(does, MOVE),
        copying: ops.is(does, COPY),
        adding: ops.is(does, ADD),
        empty: ops.is(does, NOP),
        data: ops.is(does, DATA),
        halting: ops.is(does, HALT),
        faces: PRIORITY.map(|axis| ops.is(pointed, facing(corner, axis))),
    }
}

/// The flag of `flags` along `axis`.
const fn along<T: Copy>(flags: [T; 3], axis: Axis) -> T {
    let [x, y, z] = flags;
    match axis {
        Axis::X => x,
        Axis::Y => y,
        Axis::Z => z,
    }
}

/// One claim over [`Ops`]: whether `source` claims its neighbour `target`
/// along `axis`, and the word it claims it for.
fn claimed_by<O: Ops>(
    ops: &mut O,
    source: &Cell<O::Word>,
    target: &Cell<O::Word>,
    axis: Axis,
) -> (Flag<O::Word>, O::Word) {
    let faces = along(source.faces, axis);
    let walks = ops.both(source.moving, faces);
    let spreads = ops.either(walks, source.copying);
    let into_empty = ops.both(spreads, target.empty);
    let sums = ops.both(source.adding, faces);
    let into_data = ops.both(sums, target.data);
    let claims = ops.either(into_empty, into_data);
    let total = ops.add(source.word, target.word);
    let total = ops.masked(total, PAYLOAD);
    let kept = ops.masked(source.word, !PAYLOAD);
    let added = ops.or(kept, total);
    let word = ops.select(sums, added, source.word);
    (claims, word)
}

/// The word the cell `source`, at corner `from`, claims its neighbour
/// `target` along `axis` for, if it claims it.
#[must_use]
pub fn claim(source: u32, from: usize, target: u32, axis: usize) -> Option<u32> {
    let from = Corner::new(from)?;
    let axis = Axis::of(axis)?;
    let ops = &mut Cpu;
    let source = cell_by(ops, source, instruction_of(source), from);
    let target = cell_by(ops, target, instruction_of(target), from.neighbour(axis));
    let (claims, word) = claimed_by(ops, &source, &target, axis);
    (claims.word() != 0).then_some(word)
}

/// One step of one oct over [`Ops`], without the tally.
fn stepped_by<O: Ops>(ops: &mut O, words: Octet<O::Word>) -> Octet<O::Word> {
    let instructions = words.map(|word| instruction(ops, word));
    let turned = turned_by(ops, words, instructions);
    let cells = Octet::from_fn(|corner| cell_by(ops, turned[corner], instructions[corner], corner));
    let none = ops.truth(false);

    let mut out = turned;
    let mut won = Octet([[none; 3]; CORNERS]);
    for target in Corner::ALL {
        let mut taken = none;
        for axis in PRIORITY {
            let from = target.neighbour(axis);
            let (claims, claimed) = claimed_by(ops, &cells[from], &cells[target], axis);
            let free = ops.denied(taken);
            let wins = ops.both(claims, free);
            taken = ops.either(taken, claims);
            out[target] = ops.select(wins, claimed, out[target]);
            won[target] = winning(won[target], axis, wins);
        }
    }

    let gone = ops.constant(word(NOP, 0, 0));
    let stopped = ops.constant(DATA << INSTRUCTION_SHIFT);
    for corner in Corner::ALL {
        let cell = cells[corner];
        let walker = ops.either(cell.moving, cell.adding);
        let kept = ops.masked(cell.word, !INSTRUCTION_MASK);
        let halted = ops.or(kept, stopped);
        let mut now = out[corner];
        for axis in PRIORITY {
            let near = corner.neighbour(axis);
            let left = ops.both(along(won[near], axis), walker);
            now = ops.select(left, gone, now);
            let blocked = ops.both(cell.moving, along(cell.faces, axis));
            let blocked = ops.both(blocked, cells[near].halting);
            now = ops.select(blocked, halted, now);
        }
        out[corner] = now;
    }
    out
}

/// `won` with the claim along `axis` set to `wins`.
const fn winning<T: Copy>(won: [T; 3], axis: Axis, wins: T) -> [T; 3] {
    let [x, y, z] = won;
    match axis {
        Axis::X => [wins, y, z],
        Axis::Y => [x, wins, z],
        Axis::Z => [x, y, wins],
    }
}

/// One step of one oct on the CPU, without the tally: turn, claims and
/// resolution.
#[must_use]
pub fn stepped(words: [u32; CORNERS]) -> [u32; CORNERS] {
    stepped_by(&mut Cpu, Octet(words)).0
}

/// Cellular code, as the module describes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Code;

impl Declared for Code {
    fn params(&self) -> usize {
        0
    }

    fn tally(&self) -> Tally {
        Tally {
            counts: 3,
            weighted: true,
        }
    }
}

impl OneStep for Code {
    fn one_step<O: Ops>(&self, ops: &mut O, oct: Oct<'_, O::Word>) -> Ruled<O::Word> {
        let before = Octet(oct.words);
        let out = stepped_by(ops, before);
        let changed = Octet::from_fn(|corner| ops.not_equal(before[corner], out[corner]));
        let does = out.map(|after| instruction(ops, after));
        let moving = does.map(|does| ops.is(does, MOVE));
        let data = does.map(|does| ops.is(does, DATA));
        let counts = vec![ops.count(changed.0), ops.count(moving.0), ops.count(data.0)];
        Ruled { out: out.0, counts }
    }
}
