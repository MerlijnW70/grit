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

use simdr::lanes::{LaneError, Lanes, U32, Vector};

use crate::layout::Dimension;

pub mod heat;
pub mod sand;

/// The cells of an oct.
pub const CORNERS: usize = 8;

/// The first word of `PARAMS` a rule's parameters take. The words before it
/// are the engine's.
pub const FIRST_PARAM: usize = Dimension::Three.params_words();

/// One lane's word on the device.
pub type Word<const LANES: u32> = Vector<U32, LANES>;

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
///   as [`Rule::params`] says.
#[derive(Debug, Clone, Copy)]
pub struct Oct<'held, W> {
    pub words: [W; CORNERS],
    pub frame: W,
    pub at: [W; 3],
    pub params: &'held [W],
}

/// What one step of a rule gives: the eight words to store, in the order
/// they came, and the rule's three tally words. The first counts the cells
/// the rule moved, and the engine weights it by the oct's point; the other
/// two are the rule's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ruled<W> {
    pub out: [W; CORNERS],
    pub counts: [W; 3],
}

/// A rule of the 3D engine.
///
/// [`Rule::emit`] must equal [`Rule::step`] bit for bit on every input, at
/// every subgroup width. Both are pure: the step of an oct depends only on
/// its [`Oct`].
pub trait Rule {
    /// How many words of `PARAMS` the rule reads, from [`FIRST_PARAM`] on.
    fn params(&self) -> usize;

    /// One step of one oct on the CPU.
    fn step(&self, oct: Oct<'_, u32>) -> Ruled<u32>;

    /// One step of one oct, emitted into `lanes`.
    fn emit<const LANES: u32>(
        &self,
        lanes: &mut Lanes<'_>,
        oct: Oct<'_, Word<LANES>>,
    ) -> Result<Ruled<Word<LANES>>, LaneError>;
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

fn mixed_emitted<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    value: Word<LANES>,
) -> Result<Word<LANES>, LaneError> {
    let sixteen = lanes.splat_bits::<U32, LANES>(16)?;
    let fifteen = lanes.splat_bits::<U32, LANES>(15)?;
    let first = lanes.splat_bits::<U32, LANES>(0x7FEB_352D)?;
    let second = lanes.splat_bits::<U32, LANES>(0x846C_A68B)?;
    let shifted = lanes.shift_right_logical(value, sixteen)?;
    let out = lanes.xor(value, shifted)?;
    let out = lanes.mul(out, first)?;
    let shifted = lanes.shift_right_logical(out, fifteen)?;
    let out = lanes.xor(out, shifted)?;
    let out = lanes.mul(out, second)?;
    let shifted = lanes.shift_right_logical(out, sixteen)?;
    lanes.xor(out, shifted)
}

/// [`noise`], emitted into `lanes`.
pub fn noise_emitted<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    at: [Word<LANES>; 3],
    frame: Word<LANES>,
) -> Result<Word<LANES>, LaneError> {
    let stir = lanes.splat_bits::<U32, LANES>(STIR)?;
    let start = lanes.xor(frame, stir)?;
    let mut out = mixed_emitted::<LANES>(lanes, start)?;
    for value in at {
        let folded = lanes.xor(out, value)?;
        out = mixed_emitted::<LANES>(lanes, folded)?;
    }
    Ok(out)
}

/// The rule's parameter `which`, or 0 where `params` holds fewer words.
#[must_use]
pub fn param(params: &[u32], which: usize) -> u32 {
    params.get(which).copied().unwrap_or(0)
}
