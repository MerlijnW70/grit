//! What a rule of the 3D engine is: one step of one oct of eight 32-bit
//! words, on the CPU and emitted through simdr, and the noise the engine
//! gives every rule.
//!
//! The engine places the octs, loads and stores their words, lists and wakes
//! them, and keeps the seams and the tally. It reads nothing in a word: a
//! rule owns every bit. An oct the step leaves unchanged is quiet; one it
//! changes wakes the 26 octs around it.
//!
//! The listed frame steps an oct only while it, or an oct around it, changed
//! in one of the two frames before. So the listed frame equals the whole grid
//! for a rule that keeps quiet octs quiet: a step that leaves eight words
//! unchanged at one frame leaves them unchanged two frames later, at the same
//! oct. Noise may decide a step only where the step changes the oct whatever
//! the noise says.
//!
//! A rule is best written once, as a [`OneStep`] over the word operations of
//! [`crate::ops`]: the same function is then its CPU reference and its
//! kernel. [`Corner`], [`Axis`] and [`Octet`] give it the oct's geometry.

use core::ops::{Index, IndexMut};

use simdr::lanes::{LaneError, Lanes};

use crate::layout::Dimension;
use crate::ops::{Cpu, Device, Ops};

pub use crate::ops::Word;

pub mod code;
pub mod heat;
pub mod sand;

/// The cells of an oct.
pub const CORNERS: usize = 8;

/// The first word of `PARAMS` a rule's parameters take. The words before it
/// are the engine's.
pub const FIRST_PARAM: usize = Dimension::Three.params_words();

/// An axis of the lattice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    /// The axes, x, y and z.
    pub const ALL: [Self; 3] = [Self::X, Self::Y, Self::Z];

    /// The bit of a corner's index that is its place along the axis: x is
    /// 1, z is 2 and y is 4, as [`crate::sand3::Grid3::corners`] numbers
    /// the corners.
    #[must_use]
    pub const fn bit(self) -> usize {
        match self {
            Self::X => 1,
            Self::Y => 4,
            Self::Z => 2,
        }
    }

    /// 0, 1 and 2 for x, y and z.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::X => 0,
            Self::Y => 1,
            Self::Z => 2,
        }
    }

    /// The axis of index 0, 1 or 2.
    #[must_use]
    pub const fn of(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::X),
            1 => Some(Self::Y),
            2 => Some(Self::Z),
            _ => None,
        }
    }
}

/// A cell of an oct, 0 to 7, in the order of
/// [`crate::sand3::Grid3::corners`]: its bit [`Axis::bit`] is its place
/// along each axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Corner(u8);

impl Corner {
    /// The corners in order, 0 to 7.
    pub const ALL: [Self; CORNERS] = [
        Self(0),
        Self(1),
        Self(2),
        Self(3),
        Self(4),
        Self(5),
        Self(6),
        Self(7),
    ];

    /// The corner of index `index`, if it is below [`CORNERS`].
    #[must_use]
    pub const fn new(index: usize) -> Option<Self> {
        match index {
            0..CORNERS => Some(Self((index & 7) as u8)),
            _ => None,
        }
    }

    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    /// The corner next to this one along `axis`, in the same oct.
    #[must_use]
    pub const fn neighbour(self, axis: Axis) -> Self {
        Self(self.0 ^ (axis.bit() as u8))
    }

    /// The side of the oct the corner is on along `axis`: 0 low, 1 high.
    #[must_use]
    pub const fn side(self, axis: Axis) -> u32 {
        if self.0 as usize & axis.bit() == 0 {
            0
        } else {
            1
        }
    }
}

/// One value for each corner of an oct, indexed by [`Corner`]; indexing
/// cannot fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Octet<T>(pub [T; CORNERS]);

impl<T> Octet<T> {
    #[must_use]
    pub fn from_fn(made: impl FnMut(Corner) -> T) -> Self {
        Self(Corner::ALL.map(made))
    }

    #[must_use]
    pub fn map<U>(self, made: impl FnMut(T) -> U) -> Octet<U> {
        Octet(self.0.map(made))
    }

    /// Each corner with its value, in corner order.
    pub fn iter(&self) -> impl Iterator<Item = (Corner, &T)> {
        Corner::ALL.into_iter().zip(&self.0)
    }
}

impl<T> From<[T; CORNERS]> for Octet<T> {
    fn from(values: [T; CORNERS]) -> Self {
        Self(values)
    }
}

impl<T> Index<Corner> for Octet<T> {
    type Output = T;

    fn index(&self, corner: Corner) -> &T {
        let [v0, v1, v2, v3, v4, v5, v6, v7] = &self.0;
        match corner.0 & 7 {
            0 => v0,
            1 => v1,
            2 => v2,
            3 => v3,
            4 => v4,
            5 => v5,
            6 => v6,
            _ => v7,
        }
    }
}

impl<T> IndexMut<Corner> for Octet<T> {
    fn index_mut(&mut self, corner: Corner) -> &mut T {
        let [v0, v1, v2, v3, v4, v5, v6, v7] = &mut self.0;
        match corner.0 & 7 {
            0 => v0,
            1 => v1,
            2 => v2,
            3 => v3,
            4 => v4,
            5 => v5,
            6 => v6,
            _ => v7,
        }
    }
}

/// One oct as a rule sees it, on the CPU (`W` is `u32`) or on the device
/// (`W` is a [`Word`]).
///
/// - `words`: the eight cells in the order of [`crate::sand3::Grid3::corners`]:
///   corner `k` lies `k & 1` along x, `(k >> 1) & 1` along z and `k >> 2`
///   along y, so corners 0 to 3 are the upper layer and 4 to 7 the lower.
/// - `frame`: the frame counter, `PARAMS[0]`; its bit 0 is the phase.
/// - `at`: the oct's point in the lattice of octs, x, y and z. With the
///   phase it names the eight cells.
/// - `params`: the rule's words of `PARAMS` from [`FIRST_PARAM`] on, as many
///   as [`Declared::params`] says.
#[derive(Debug, Clone, Copy)]
pub struct Oct<'held, W> {
    pub words: [W; CORNERS],
    pub frame: W,
    pub at: [W; 3],
    pub params: &'held [W],
}

/// The most tally counts a rule may declare.
pub const MOST_COUNTS: usize = 8;

/// The tally a rule leaves each frame, as [`Declared::tally`] declares it.
///
/// The frame's tally holds the sum over the stepped octs of each of the
/// rule's `counts`, in order. A `weighted` tally goes on with the first
/// count weighted by each axis of the oct's point shifted down by
/// [`crate::layout::COARSE`], x, y and z, and then the octs whose first
/// count is not zero; its first count is the cells the rule moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tally {
    pub counts: usize,
    pub weighted: bool,
}

impl Tally {
    /// The words the tally takes: the counts, and four more when weighted.
    #[must_use]
    pub const fn words(self) -> usize {
        if self.weighted {
            self.counts + 4
        } else {
            self.counts
        }
    }

    /// Whether the engine keeps such a tally: 1 to [`MOST_COUNTS`] counts.
    #[must_use]
    pub const fn fits(self) -> bool {
        self.counts >= 1 && self.counts <= MOST_COUNTS
    }
}

/// What one step of a rule gives: the eight words to store, in the order
/// they came, and the rule's tally counts, as many as [`Declared::tally`]
/// declares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ruled<W> {
    pub out: [W; CORNERS],
    pub counts: Vec<W>,
}

/// What a rule declares to the engine, however its step is written.
pub trait Declared {
    /// How many words of `PARAMS` the rule reads, from [`FIRST_PARAM`] on.
    fn params(&self) -> usize;

    /// The tally counts the rule gives, and whether the first is weighted.
    fn tally(&self) -> Tally;
}

/// A rule of the 3D engine.
///
/// [`Rule::emit`] must equal [`Rule::step`] bit for bit on every input, at
/// every subgroup width. Both are pure: the step of an oct depends only on
/// its [`Oct`]. A [`OneStep`] rule has both from its one step; a rule may
/// also write the two by hand.
pub trait Rule: Declared {
    /// One step of one oct on the CPU.
    fn step(&self, oct: Oct<'_, u32>) -> Ruled<u32>;

    /// One step of one oct, emitted into `lanes`.
    fn emit<const LANES: u32>(
        &self,
        lanes: &mut Lanes<'_>,
        oct: Oct<'_, Word<LANES>>,
    ) -> Result<Ruled<Word<LANES>>, LaneError>;
}

/// A rule written once, over [`Ops`]: [`OneStep::one_step`] with [`Cpu`]
/// is its [`Rule::step`], and with [`Device`] its [`Rule::emit`].
pub trait OneStep: Declared {
    /// One step of one oct.
    fn one_step<O: Ops>(&self, ops: &mut O, oct: Oct<'_, O::Word>) -> Ruled<O::Word>;
}

impl<S: OneStep> Rule for S {
    fn step(&self, oct: Oct<'_, u32>) -> Ruled<u32> {
        self.one_step(&mut Cpu, oct)
    }

    fn emit<const LANES: u32>(
        &self,
        lanes: &mut Lanes<'_>,
        oct: Oct<'_, Word<LANES>>,
    ) -> Result<Ruled<Word<LANES>>, LaneError> {
        let mut device = Device::<LANES>::new(lanes)?;
        let ruled = self.one_step(&mut device, oct);
        device.finish(ruled)
    }
}

const STIR: u32 = 0x9E37_79B9;

const fn mixed(value: u32) -> u32 {
    let mut out = value;
    out ^= out >> 16;
    out = out.wrapping_mul(0x7FEB_352D);
    out ^= out >> 15;
    out = out.wrapping_mul(0x846C_A68B);
    out ^ (out >> 16)
}

/// The engine's noise for the oct at lattice point `at` in frame `frame`: a
/// 32-bit hash, the same on the CPU and on the device. Each input word is
/// folded in with an exclusive or and then mixed by a bijection of `u32`.
#[must_use]
pub const fn noise(at: [u32; 3], frame: u32) -> u32 {
    let mut out = mixed(frame ^ STIR);
    out = mixed(out ^ at[0]);
    out = mixed(out ^ at[1]);
    mixed(out ^ at[2])
}

fn mixed_by<O: Ops>(ops: &mut O, value: O::Word) -> O::Word {
    let first = ops.constant(0x7FEB_352D);
    let second = ops.constant(0x846C_A68B);
    let shifted = ops.shift_right(value, 16);
    let out = ops.xor(value, shifted);
    let out = ops.mul(out, first);
    let shifted = ops.shift_right(out, 15);
    let out = ops.xor(out, shifted);
    let out = ops.mul(out, second);
    let shifted = ops.shift_right(out, 16);
    ops.xor(out, shifted)
}

/// [`noise`] over [`Ops`].
pub fn noise_by<O: Ops>(ops: &mut O, at: [O::Word; 3], frame: O::Word) -> O::Word {
    let stir = ops.constant(STIR);
    let start = ops.xor(frame, stir);
    let mut out = mixed_by(ops, start);
    for value in at {
        let folded = ops.xor(out, value);
        out = mixed_by(ops, folded);
    }
    out
}

/// [`noise`], emitted into `lanes`.
pub fn noise_emitted<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    at: [Word<LANES>; 3],
    frame: Word<LANES>,
) -> Result<Word<LANES>, LaneError> {
    let mut device = Device::<LANES>::new(lanes)?;
    let made = noise_by(&mut device, at, frame);
    device.finish(made)
}

/// The rule's parameter `which`, or 0 where `params` holds fewer words.
#[must_use]
pub fn param(params: &[u32], which: usize) -> u32 {
    params.get(which).copied().unwrap_or(0)
}

/// [`param`] over [`Ops`]: the constant 0 where `params` holds fewer words.
pub fn param_by<O: Ops>(ops: &mut O, params: &[O::Word], which: usize) -> O::Word {
    match params.get(which) {
        Some(&value) => value,
        None => ops.constant(0),
    }
}
