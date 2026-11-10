//! Sand and water, a rule of the 3D engine.
//!
//! A cell word holds the material in bits 0 and 1, the running way in bits
//! 2 to 4, the held bit in bit 5 and the tired steps in bits 6 to 8. One
//! step is [`settle_oct`]; the frame sets its axis order through
//! [`x_first`].
//!
//! The rule is one [`OneStep`] over [`Ops`]. A frame that takes z first is
//! stepped as its mirror across the plane x = z: the words whose x and z
//! differ trade places, a running word trades its x and z ways, the x-first
//! rule runs, and both trade back. The rule treats x and z alike but for
//! their order, so the mirror of an x-first step is the z-first step, and
//! one x-first rule serves both orders on the CPU and on the device.

use super::{Axis, CORNERS, Corner, Declared, Oct, Octet, OneStep, Ruled, Tally};
use crate::ops::{Cpu, Flag, Helpers, Ops};
use crate::sand::{AIR, PATIENCE, SAND, WALL, WATER};

const MATERIAL_BITS: u32 = 2;
const RUNNING_BITS: u32 = 3;
const CARRIED_BITS: u32 = MATERIAL_BITS + RUNNING_BITS;
const TIRED_BITS: u32 = 3;

pub const HELD: u32 = 1 << CARRIED_BITS;
pub const TIRED: u32 = HELD << 1;

pub const MINUS_X: u32 = 1;
pub const PLUS_X: u32 = 2;
pub const MINUS_Z: u32 = 5;
pub const PLUS_Z: u32 = 6;

pub const RUNNING_MINUS_X: u32 = WATER + (MINUS_X << MATERIAL_BITS);
pub const RUNNING_PLUS_X: u32 = WATER + (PLUS_X << MATERIAL_BITS);
pub const RUNNING_MINUS_Z: u32 = WATER + (MINUS_Z << MATERIAL_BITS);
pub const RUNNING_PLUS_Z: u32 = WATER + (PLUS_Z << MATERIAL_BITS);

const WORN: u32 = ((1 << TIRED_BITS) - 1) * TIRED;

#[must_use]
pub const fn material_of(word: u32) -> u32 {
    word % (1 << MATERIAL_BITS)
}

#[must_use]
pub const fn running_of(word: u32) -> u32 {
    (word >> MATERIAL_BITS) % (1 << RUNNING_BITS)
}

#[must_use]
pub const fn held_of(word: u32) -> u32 {
    (word >> CARRIED_BITS) % 2
}

#[must_use]
pub const fn tired_of(word: u32) -> u32 {
    (word >> (CARRIED_BITS + 1)) % (1 << TIRED_BITS)
}

/// Whether frame `frame` takes the x axis before the z axis. The order holds
/// for two frames, one of each phase, and then flips, so each phase sees both
/// orders and every oct of a frame runs the same rule.
#[must_use]
pub const fn x_first(frame: u32) -> bool {
    (frame >> 1) & 1 == 0
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

fn material<O: Ops>(ops: &mut O, word: O::Word) -> O::Word {
    ops.masked(word, (1 << MATERIAL_BITS) - 1)
}

fn is_material<O: Ops>(ops: &mut O, word: O::Word, want: u32) -> Flag<O::Word> {
    let kind = material(ops, word);
    ops.is(kind, want)
}

fn running<O: Ops>(ops: &mut O, word: O::Word) -> O::Word {
    ops.field(word, MATERIAL_BITS, RUNNING_BITS)
}

/// [`hold_of`] over [`Ops`].
fn hold_by<O: Ops>(ops: &mut O, word: O::Word, under: O::Word) -> O::Word {
    let kept = ops.masked(word, (HELD - 1) | WORN);
    let wet = is_material(ops, word, WATER);
    let standing = is_material(ops, under, WATER);
    let both = ops.both(wet, standing);
    let bit = ops.constant(HELD);
    let set = ops.add(kept, bit);
    ops.select(both, set, kept)
}

/// `word` with its held bit taken from the cell `under` it: set when both
/// are water, else cleared.
#[must_use]
pub fn hold_of(word: u32, under: u32) -> u32 {
    hold_by(&mut Cpu, word, under)
}

/// Whether `word` is ready to start running: idle held water with tired
/// steps to spare.
fn ready<O: Ops>(ops: &mut O, word: O::Word) -> Flag<O::Word> {
    let wet = is_material(ops, word, WATER);
    let way = running(ops, word);
    let still = ops.is(way, 0);
    let held = ops.field(word, CARRIED_BITS, 1);
    let held = ops.is(held, 1);
    let worn = ops.field(word, CARRIED_BITS + 1, TIRED_BITS);
    let patience = ops.constant(PATIENCE);
    let spare = ops.less(worn, patience);
    let all = ops.both(wet, still);
    let all = ops.both(all, held);
    ops.both(all, spare)
}

/// `word` started running along `way`: one more tired step, held.
fn started<O: Ops>(ops: &mut O, word: O::Word, way: u32) -> O::Word {
    let worn = ops.masked(word, WORN);
    let fixed = ops.constant(WATER + (way << MATERIAL_BITS) + HELD + TIRED);
    ops.add(worn, fixed)
}

/// `word` stopped: its material, held bit and tired steps.
fn halted<O: Ops>(ops: &mut O, word: O::Word) -> O::Word {
    ops.masked(word, ((1 << MATERIAL_BITS) - 1) | HELD | WORN)
}

/// One pair of cells along `axis`, the lower first.
fn along<O: Ops>(ops: &mut O, axis: Axis, [here, there]: [O::Word; 2]) -> [O::Word; 2] {
    let going_here = running(ops, here);
    let going_there = running(ops, there);
    let goes_up = ops.is(going_here, plus(axis));
    let goes_down = ops.is(going_there, minus(axis));
    let up_free = is_material(ops, there, AIR);
    let down_free = is_material(ops, here, AIR);
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

/// The cell `upper` falls into `lower` when its material is heavier and
/// not a wall: the upper cell takes the lower word, and the lower cell the
/// upper material.
fn fall<O: Ops>(ops: &mut O, upper: O::Word, lower: O::Word) -> [O::Word; 2] {
    let above = material(ops, upper);
    let below = material(ops, lower);
    let heavier = ops.greater(above, below);
    let wall = ops.constant(WALL);
    let loose = ops.less(above, wall);
    let moving = ops.both(heavier, loose);
    [
        ops.select(moving, lower, upper),
        ops.select(moving, above, lower),
    ]
}

/// The x-first rule over [`Ops`]: the words after the held step, and after
/// the whole rule.
///
/// In order: the four upper cells take their held bit from the cell under
/// them; held idle water starts to run toward an empty neighbour in its
/// layer along x, else z; runners swap with an empty neighbour along x and
/// then z; the four vertical pairs fall; then the four upper cells fall
/// along an edge diagonal of x, then of z, then along the corner diagonal.
fn x_first_by<O: Ops>(ops: &mut O, oct: Octet<O::Word>) -> (Octet<O::Word>, Octet<O::Word>) {
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
            let free = is_material(ops, before[at.neighbour(axis)], AIR);
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

    let straight = upper.map(|top| (top, top.neighbour(Axis::Y)));
    let edges = LEVEL.map(|axis| upper.map(|top| (top, top.neighbour(axis).neighbour(Axis::Y))));
    let corner = upper.map(|top| {
        let across = top.neighbour(Axis::X).neighbour(Axis::Z);
        (top, across.neighbour(Axis::Y))
    });
    let falls = straight
        .into_iter()
        .chain(edges.into_iter().flatten())
        .chain(corner);
    for (high, low) in falls {
        let [up, down] = fall(ops, cells[high], cells[low]);
        cells[high] = up;
        cells[low] = down;
    }

    (before, cells)
}

/// The corners the two axis orders trade: those whose x and z bits differ.
const TRADED: [(usize, usize); 2] = [(1, 2), (5, 6)];

/// `cells` with the corners of [`TRADED`] swapped where `mirrored` holds.
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

/// `word` with its x and z ways traded where `mirrored` holds. A running
/// field whose two low bits differ names a way, and its high bit picks z
/// over x; the other running fields name no way and stay.
fn traded<O: Ops>(ops: &mut O, word: O::Word, mirrored: Flag<O::Word>) -> O::Word {
    let first = ops.shift_right(word, MATERIAL_BITS);
    let second = ops.shift_right(word, MATERIAL_BITS + 1);
    let differ = ops.xor(first, second);
    let differ = ops.masked(differ, 1);
    let flip = ops.shift_left(differ, MATERIAL_BITS + 2);
    let flipped = ops.xor(word, flip);
    ops.select(mirrored, flipped, word)
}

/// One step of an oct: the words after the held step and after the whole
/// rule, both in the x-first mirror, and the words to store.
struct Settled<W> {
    held: Octet<W>,
    ruled: Octet<W>,
    out: Octet<W>,
}

/// The rule's step over [`Ops`], z first where `z_first` holds.
fn settled_by<O: Ops>(
    ops: &mut O,
    oct: Octet<O::Word>,
    z_first: Flag<O::Word>,
) -> Settled<O::Word> {
    let cells = traded_places(ops, oct, z_first);
    let cells = cells.map(|cell| traded(ops, cell, z_first));
    let (held, ruled) = x_first_by(ops, cells);
    let back = ruled.map(|cell| traded(ops, cell, z_first));
    let out = traded_places(ops, back, z_first);
    Settled { held, ruled, out }
}

/// One step of one oct, its corners laid out as
/// [`crate::sand3::Grid3::corners`] gives them.
///
/// In order: the four upper cells take their held bit from the cell under
/// them; held idle water starts to run toward an empty neighbour in its layer
/// along the first axis, else the second; runners swap with an empty
/// neighbour along the first axis and then the second; the four vertical
/// pairs fall; then the four upper cells fall along an edge diagonal of the
/// first axis, then of the second axis, then along the corner diagonal. Each
/// of these stages works on four disjoint pairs, so only the axis order can
/// change an outcome, and `x_first` sets it.
#[must_use]
pub fn settle_oct(oct: [u32; CORNERS], x_first: bool) -> [u32; CORNERS] {
    let ops = &mut Cpu;
    let z_first = ops.truth(!x_first);
    settled_by(ops, Octet(oct), z_first).out.0
}

/// Sand and water: [`settle_oct`] as a rule of the 3D engine. It reads no
/// parameters and no noise. Its tally words are the cells that moved past
/// the held step, the water cells and the sand cells after the step, the
/// first weighted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Sand;

impl Declared for Sand {
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

impl OneStep for Sand {
    fn one_step<O: Ops>(&self, ops: &mut O, oct: Oct<'_, O::Word>) -> Ruled<O::Word> {
        let order = ops.masked(oct.frame, 2);
        let none = ops.constant(0);
        let z_first = ops.not_equal(order, none);
        let Settled { held, ruled, out } = settled_by(ops, Octet(oct.words), z_first);
        let moved = Octet::from_fn(|corner| ops.not_equal(held[corner], ruled[corner]));
        let wet = out.map(|cell| is_material(ops, cell, WATER));
        let dry = out.map(|cell| is_material(ops, cell, SAND));
        let counts = vec![ops.count(moved.0), ops.count(wet.0), ops.count(dry.0)];
        Ruled { out: out.0, counts }
    }
}
