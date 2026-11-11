use simdr::lanes::{LaneError, Lanes};

use super::{Axis, Corner, Declared, Oct, Octet, Rule, Ruled, Tally, Word, param_by};
use crate::host::Host;
use crate::ops::{Cpu, Device, Dots, Helpers, Ops};

pub const PARAMS: usize = 1;

pub const COUNTS: usize = 2;

const BIAS: u32 = 0x8000_0000;

const ORDER: [Axis; 3] = [Axis::X, Axis::Z, Axis::Y];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Embed {
    pub dots: Dots,
}

impl Embed {
    #[must_use]
    pub const fn new(dots: Dots) -> Self {
        Self { dots }
    }

    #[must_use]
    pub const fn on(host: &Host) -> Self {
        Self::new(host.dots())
    }
}

#[must_use]
pub const fn kind_of(word: u32) -> u32 {
    (word >> 4) & 0xF
}

#[must_use]
pub const fn embedding(components: [i8; 4]) -> u32 {
    u32::from_le_bytes([
        components[0].cast_unsigned(),
        components[1].cast_unsigned(),
        components[2].cast_unsigned(),
        components[3].cast_unsigned(),
    ])
}

#[must_use]
pub fn score(word: u32, toward: u32) -> i32 {
    Cpu.dot_signed(word, toward).cast_signed()
}

fn stepped<O: Ops>(ops: &mut O, oct: Oct<'_, O::Word>) -> Ruled<O::Word> {
    let toward = param_by(ops, oct.params, 0);
    let bias = ops.constant(BIAS);
    let start = Octet(oct.words);
    let mut words = start;
    let mut keys = start.map(|word| {
        let dot = ops.dot_signed(word, toward);
        ops.xor(dot, bias)
    });
    let phase = ops.masked(oct.frame, 1);
    let high_odd = ops.is(phase, 0);
    let low_odd = ops.denied(high_odd);
    for axis in ORDER {
        for low in Corner::ALL {
            if low.side(axis) != 0 {
                continue;
            }
            let high = low.neighbour(axis);
            let (low_key, high_key) = (keys[low], keys[high]);
            let rises = ops.less(high_key, low_key);
            let falls = ops.less(low_key, high_key);
            let up = ops.both(high_odd, rises);
            let down = ops.both(low_odd, falls);
            let swaps = ops.either(up, down);
            let (low_word, high_word) = (words[low], words[high]);
            words[low] = ops.select(swaps, high_word, low_word);
            words[high] = ops.select(swaps, low_word, high_word);
            keys[low] = ops.select(swaps, high_key, low_key);
            keys[high] = ops.select(swaps, low_key, high_key);
        }
    }
    let changed: Vec<_> = Corner::ALL
        .into_iter()
        .map(|corner| ops.not_equal(words[corner], start[corner]))
        .collect();
    let moved = ops.count(changed);
    let positive: Vec<_> = Corner::ALL
        .into_iter()
        .map(|corner| ops.greater(keys[corner], bias))
        .collect();
    let aligned = ops.count(positive);
    Ruled {
        out: words.0,
        counts: vec![moved, aligned],
    }
}

impl Declared for Embed {
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

impl Rule for Embed {
    fn step(&self, oct: Oct<'_, u32>) -> Ruled<u32> {
        stepped(&mut Cpu, oct)
    }

    fn emit<const LANES: u32>(
        &self,
        lanes: &mut Lanes<'_>,
        oct: Oct<'_, Word<LANES>>,
    ) -> Result<Ruled<Word<LANES>>, LaneError> {
        let mut device = Device::<LANES>::dotting(lanes, self.dots)?;
        let ruled = stepped(&mut device, oct);
        device.finish(ruled)
    }
}
