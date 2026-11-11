//! The word operations a rule of the 3D engine is written in, once, for the
//! CPU and for the device.
//!
//! [`Ops`] is implemented for `u32` words on the CPU by [`Cpu`] and for the
//! emitted words of a subgroup by [`Device`]. A step written as one generic
//! function over [`Ops`] is then the CPU reference when run with [`Cpu`] and
//! the kernel's step when run with [`Device`], so the two cannot drift
//! apart: they differ only in the operations themselves, and those are held
//! equal one by one.
//!
//! Every operation means the same on both:
//!
//! - [`Ops::and`], [`Ops::or`], [`Ops::xor`] and [`Ops::not`] are bitwise.
//! - [`Ops::shift_left`] and [`Ops::shift_right`] shift by a constant taken
//!   modulo 32; the right shift is logical.
//! - [`Ops::add`], [`Ops::sub`] and [`Ops::mul`] wrap modulo 2 to the 32.
//! - [`Ops::div`] is the unsigned quotient, a divisor of 0 read as 1.
//! - The compares are unsigned and give a [`Flag`], a word of 1 where they
//!   hold and 0 where they do not.
//! - [`Ops::select`] gives its first word where the flag's word is not 0.
//!
//! A step branches only through [`Ops::select`]: both arms are computed and
//! the flag picks one, lane by lane. Constants come from [`Ops::constant`].
//! The [`Helpers`] every rule uses are composed of these operations alone,
//! so they too are the same on both.

use std::collections::BTreeMap;

use simdr::lanes::{LaneError, Lanes, Predicate, U32, Vector};
use simdr::module::Id;

/// One lane's word on the device.
pub type Word<const LANES: u32> = Vector<U32, LANES>;

/// A word that is 1 where something holds and 0 where it does not. Only a
/// compare or a [`Helpers`] function makes one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Flag<W>(W);

impl<W> Flag<W> {
    /// The flag as a word, 1 or 0, to count or to combine.
    pub fn word(self) -> W {
        self.0
    }
}

/// The operations a rule's step is written in. See the module for what each
/// one means; it means the same for every implementation.
pub trait Ops {
    type Word: Copy;

    fn constant(&mut self, value: u32) -> Self::Word;

    fn and(&mut self, left: Self::Word, right: Self::Word) -> Self::Word;
    fn or(&mut self, left: Self::Word, right: Self::Word) -> Self::Word;
    fn xor(&mut self, left: Self::Word, right: Self::Word) -> Self::Word;
    fn not(&mut self, value: Self::Word) -> Self::Word;

    fn shift_left(&mut self, value: Self::Word, by: u32) -> Self::Word;
    fn shift_right(&mut self, value: Self::Word, by: u32) -> Self::Word;

    fn add(&mut self, left: Self::Word, right: Self::Word) -> Self::Word;
    fn sub(&mut self, left: Self::Word, right: Self::Word) -> Self::Word;
    fn mul(&mut self, left: Self::Word, right: Self::Word) -> Self::Word;
    fn div(&mut self, left: Self::Word, right: Self::Word) -> Self::Word;

    fn dot_signed(&mut self, left: Self::Word, right: Self::Word) -> Self::Word;
    fn dot_unsigned(&mut self, left: Self::Word, right: Self::Word) -> Self::Word;

    fn equal(&mut self, left: Self::Word, right: Self::Word) -> Flag<Self::Word>;
    fn not_equal(&mut self, left: Self::Word, right: Self::Word) -> Flag<Self::Word>;
    fn less(&mut self, left: Self::Word, right: Self::Word) -> Flag<Self::Word>;
    fn less_equal(&mut self, left: Self::Word, right: Self::Word) -> Flag<Self::Word>;
    fn greater(&mut self, left: Self::Word, right: Self::Word) -> Flag<Self::Word>;
    fn greater_equal(&mut self, left: Self::Word, right: Self::Word) -> Flag<Self::Word>;

    fn select(
        &mut self,
        flag: Flag<Self::Word>,
        then: Self::Word,
        otherwise: Self::Word,
    ) -> Self::Word;
}

/// The flag and word helpers every rule uses, composed of [`Ops`] alone.
/// They have one definition for every implementation, and no
/// implementation can replace them.
pub trait Helpers: Ops {
    /// `holds` as a constant flag.
    fn truth(&mut self, holds: bool) -> Flag<Self::Word> {
        Flag(self.constant(u32::from(holds)))
    }

    /// Whether `value` is the constant `want`.
    fn is(&mut self, value: Self::Word, want: u32) -> Flag<Self::Word> {
        let want = self.constant(want);
        self.equal(value, want)
    }

    fn both(&mut self, left: Flag<Self::Word>, right: Flag<Self::Word>) -> Flag<Self::Word> {
        Flag(self.and(left.0, right.0))
    }

    fn either(&mut self, left: Flag<Self::Word>, right: Flag<Self::Word>) -> Flag<Self::Word> {
        Flag(self.or(left.0, right.0))
    }

    /// The flag that holds where `flag` does not.
    fn denied(&mut self, flag: Flag<Self::Word>) -> Flag<Self::Word> {
        let one = self.constant(1);
        Flag(self.xor(flag.0, one))
    }

    /// `value` and the constant `mask`.
    fn masked(&mut self, value: Self::Word, mask: u32) -> Self::Word {
        let mask = self.constant(mask);
        self.and(value, mask)
    }

    /// The `bits` bits of `value` from bit `from` on, `bits` from 1 to 31.
    fn field(&mut self, value: Self::Word, from: u32, bits: u32) -> Self::Word {
        let shifted = self.shift_right(value, from);
        self.masked(shifted, (1_u32 << (bits % 32)).wrapping_sub(1))
    }

    /// How many of `flags` hold.
    fn count<I: IntoIterator<Item = Flag<Self::Word>>>(&mut self, flags: I) -> Self::Word {
        let mut total = self.constant(0);
        for flag in flags {
            total = self.add(total, flag.0);
        }
        total
    }
}

impl<O: Ops + ?Sized> Helpers for O {}

/// The CPU: [`Ops`] on `u32`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cpu;

fn flagged(holds: bool) -> Flag<u32> {
    Flag(u32::from(holds))
}

fn signed(byte: u8) -> i32 {
    i32::from(i8::from_le_bytes([byte]))
}

#[must_use]
pub fn signed_lanes(word: u32) -> [i32; 4] {
    let [first, second, third, fourth] = word.to_le_bytes();
    [signed(first), signed(second), signed(third), signed(fourth)]
}

#[must_use]
pub fn unsigned_lanes(word: u32) -> [i32; 4] {
    word.to_le_bytes().map(i32::from)
}

fn summed(left: [i32; 4], right: [i32; 4]) -> u32 {
    left.into_iter()
        .zip(right)
        .fold(0_i32, |total, (one, other)| {
            total.wrapping_add(one.wrapping_mul(other))
        })
        .cast_unsigned()
}

impl Ops for Cpu {
    type Word = u32;

    #[inline]
    fn constant(&mut self, value: u32) -> u32 {
        value
    }

    #[inline]
    fn and(&mut self, left: u32, right: u32) -> u32 {
        left & right
    }

    #[inline]
    fn or(&mut self, left: u32, right: u32) -> u32 {
        left | right
    }

    #[inline]
    fn xor(&mut self, left: u32, right: u32) -> u32 {
        left ^ right
    }

    #[inline]
    fn not(&mut self, value: u32) -> u32 {
        !value
    }

    #[inline]
    fn shift_left(&mut self, value: u32, by: u32) -> u32 {
        value.wrapping_shl(by)
    }

    #[inline]
    fn shift_right(&mut self, value: u32, by: u32) -> u32 {
        value.wrapping_shr(by)
    }

    #[inline]
    fn add(&mut self, left: u32, right: u32) -> u32 {
        left.wrapping_add(right)
    }

    #[inline]
    fn sub(&mut self, left: u32, right: u32) -> u32 {
        left.wrapping_sub(right)
    }

    #[inline]
    fn mul(&mut self, left: u32, right: u32) -> u32 {
        left.wrapping_mul(right)
    }

    #[inline]
    fn div(&mut self, left: u32, right: u32) -> u32 {
        left / right.max(1)
    }

    #[inline]
    fn dot_signed(&mut self, left: u32, right: u32) -> u32 {
        summed(signed_lanes(left), signed_lanes(right))
    }

    #[inline]
    fn dot_unsigned(&mut self, left: u32, right: u32) -> u32 {
        summed(unsigned_lanes(left), unsigned_lanes(right))
    }

    #[inline]
    fn equal(&mut self, left: u32, right: u32) -> Flag<u32> {
        flagged(left == right)
    }

    #[inline]
    fn not_equal(&mut self, left: u32, right: u32) -> Flag<u32> {
        flagged(left != right)
    }

    #[inline]
    fn less(&mut self, left: u32, right: u32) -> Flag<u32> {
        flagged(left < right)
    }

    #[inline]
    fn less_equal(&mut self, left: u32, right: u32) -> Flag<u32> {
        flagged(left <= right)
    }

    #[inline]
    fn greater(&mut self, left: u32, right: u32) -> Flag<u32> {
        flagged(left > right)
    }

    #[inline]
    fn greater_equal(&mut self, left: u32, right: u32) -> Flag<u32> {
        flagged(left >= right)
    }

    #[inline]
    fn select(&mut self, flag: Flag<u32>, then: u32, otherwise: u32) -> u32 {
        if flag.0 == 0 { otherwise } else { then }
    }
}

/// The device: [`Ops`] emitted into a [`Lanes`].
///
/// An operation the emitter refuses does not stop the step: the first
/// refusal is kept, every later operation emits nothing and gives the word
/// 0, and [`Device::finish`] returns the refusal. So a step written over
/// [`Ops`] needs no error handling of its own, and a failed build is never
/// mistaken for a kernel.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Dots {
    Native,
    #[default]
    Exact,
}

pub struct Device<'held, 'module, const LANES: u32> {
    lanes: &'held mut Lanes<'module>,
    zero: Word<LANES>,
    one: Word<LANES>,
    failed: Option<LaneError>,
    truths: BTreeMap<Id, Predicate<LANES>>,
    dots: Dots,
}

type Emitted<const LANES: u32> = Result<Word<LANES>, LaneError>;

impl<'held, 'module, const LANES: u32> Device<'held, 'module, LANES> {
    pub fn new(lanes: &'held mut Lanes<'module>) -> Result<Self, LaneError> {
        Self::dotting(lanes, Dots::Exact)
    }

    pub fn dotting(lanes: &'held mut Lanes<'module>, dots: Dots) -> Result<Self, LaneError> {
        let zero = lanes.splat_bits::<U32, LANES>(0)?;
        let one = lanes.splat_bits::<U32, LANES>(1)?;
        Ok(Self {
            lanes,
            zero,
            one,
            failed: None,
            truths: BTreeMap::new(),
            dots,
        })
    }

    /// `value`, or the first refusal of any operation emitted so far.
    pub fn finish<T>(self, value: T) -> Result<T, LaneError> {
        match self.failed {
            Some(why) => Err(why),
            None => Ok(value),
        }
    }

    fn emitted(&mut self, made: impl FnOnce(&mut Lanes<'module>) -> Emitted<LANES>) -> Word<LANES> {
        if self.failed.is_some() {
            return self.zero;
        }
        match made(self.lanes) {
            Ok(word) => word,
            Err(why) => {
                self.failed = Some(why);
                self.zero
            }
        }
    }

    fn compared(
        &mut self,
        compare: impl FnOnce(&mut Lanes<'module>) -> Result<Predicate<LANES>, LaneError>,
    ) -> Flag<Word<LANES>> {
        let (one, zero) = (self.one, self.zero);
        let mut held = None;
        let word = self.emitted(|lanes| {
            let truth = compare(lanes)?;
            held = Some(truth);
            lanes.select(truth, one, zero)
        });
        if let Some(truth) = held
            && self.failed.is_none()
        {
            self.truths.insert(word.id(), truth);
        }
        Flag(word)
    }

    fn constant_word(&mut self, value: u32) -> Word<LANES> {
        self.emitted(|lanes| lanes.splat_bits::<U32, LANES>(value))
    }

    fn component(&mut self, word: Word<LANES>, lane: u32, sign: bool) -> Word<LANES> {
        if sign {
            let raised = match 24 - 8 * lane {
                0 => word,
                up => self.shift_left(word, up),
            };
            let by = self.constant_word(24);
            self.emitted(|lanes| lanes.shift_right_arithmetic(raised, by))
        } else {
            let lowered = match lane {
                0 => word,
                _ => self.shift_right(word, 8 * lane),
            };
            match lane {
                3 => lowered,
                _ => self.masked(lowered, 0xFF),
            }
        }
    }

    fn exact_dot(&mut self, left: Word<LANES>, right: Word<LANES>, sign: bool) -> Word<LANES> {
        let mut total = None;
        for lane in 0..4 {
            let one = self.component(left, lane, sign);
            let other = self.component(right, lane, sign);
            let product = self.mul(one, other);
            total = Some(match total {
                None => product,
                Some(held) => self.add(held, product),
            });
        }
        total.unwrap_or(self.zero)
    }
}

impl<const LANES: u32> Ops for Device<'_, '_, LANES> {
    type Word = Word<LANES>;

    fn constant(&mut self, value: u32) -> Self::Word {
        self.constant_word(value)
    }

    fn and(&mut self, left: Self::Word, right: Self::Word) -> Self::Word {
        self.emitted(|lanes| lanes.and(left, right))
    }

    fn or(&mut self, left: Self::Word, right: Self::Word) -> Self::Word {
        self.emitted(|lanes| lanes.or(left, right))
    }

    fn xor(&mut self, left: Self::Word, right: Self::Word) -> Self::Word {
        self.emitted(|lanes| lanes.xor(left, right))
    }

    fn not(&mut self, value: Self::Word) -> Self::Word {
        self.emitted(|lanes| lanes.not(value))
    }

    fn shift_left(&mut self, value: Self::Word, by: u32) -> Self::Word {
        let by = self.constant_word(by % 32);
        self.emitted(|lanes| lanes.shift_left(value, by))
    }

    fn shift_right(&mut self, value: Self::Word, by: u32) -> Self::Word {
        let by = self.constant_word(by % 32);
        self.emitted(|lanes| lanes.shift_right_logical(value, by))
    }

    fn add(&mut self, left: Self::Word, right: Self::Word) -> Self::Word {
        self.emitted(|lanes| lanes.add(left, right))
    }

    fn sub(&mut self, left: Self::Word, right: Self::Word) -> Self::Word {
        self.emitted(|lanes| lanes.sub(left, right))
    }

    fn mul(&mut self, left: Self::Word, right: Self::Word) -> Self::Word {
        self.emitted(|lanes| lanes.mul(left, right))
    }

    fn div(&mut self, left: Self::Word, right: Self::Word) -> Self::Word {
        let one = self.one;
        self.emitted(|lanes| {
            let divisor = lanes.max(right, one)?;
            lanes.div(left, divisor)
        })
    }

    fn dot_signed(&mut self, left: Self::Word, right: Self::Word) -> Self::Word {
        match self.dots {
            Dots::Native => self.emitted(|lanes| {
                let total = lanes.dot_signed(left, right)?;
                lanes.reinterpret(total)
            }),
            Dots::Exact => self.exact_dot(left, right, true),
        }
    }

    fn dot_unsigned(&mut self, left: Self::Word, right: Self::Word) -> Self::Word {
        match self.dots {
            Dots::Native => self.emitted(|lanes| lanes.dot_unsigned(left, right)),
            Dots::Exact => self.exact_dot(left, right, false),
        }
    }

    fn equal(&mut self, left: Self::Word, right: Self::Word) -> Flag<Self::Word> {
        self.compared(|lanes| lanes.equal(left, right))
    }

    fn not_equal(&mut self, left: Self::Word, right: Self::Word) -> Flag<Self::Word> {
        self.compared(|lanes| lanes.not_equal(left, right))
    }

    fn less(&mut self, left: Self::Word, right: Self::Word) -> Flag<Self::Word> {
        self.compared(|lanes| lanes.less_than(left, right))
    }

    fn less_equal(&mut self, left: Self::Word, right: Self::Word) -> Flag<Self::Word> {
        self.compared(|lanes| lanes.less_equal(left, right))
    }

    fn greater(&mut self, left: Self::Word, right: Self::Word) -> Flag<Self::Word> {
        self.compared(|lanes| lanes.greater_than(left, right))
    }

    fn greater_equal(&mut self, left: Self::Word, right: Self::Word) -> Flag<Self::Word> {
        self.compared(|lanes| lanes.greater_equal(left, right))
    }

    fn select(
        &mut self,
        flag: Flag<Self::Word>,
        then: Self::Word,
        otherwise: Self::Word,
    ) -> Self::Word {
        let held = self.truths.get(&flag.0.id()).copied();
        let zero = self.zero;
        self.emitted(|lanes| {
            let truth = match held {
                Some(truth) => truth,
                None => lanes.not_equal(flag.0, zero)?,
            };
            lanes.select(truth, then, otherwise)
        })
    }
}
