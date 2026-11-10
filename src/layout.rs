pub const PARAMS: u32 = 0;
pub const STATE: u32 = 1;
pub const BUCKETS: u32 = 2;
pub const COUNTS: u32 = 3;
pub const BASES: u32 = 4;
pub const TOTALS: u32 = 5;
pub const DENSE: u32 = 6;
pub const GRAND: u32 = 7;
pub const INDIRECT: u32 = 8;
pub const AWAKE: u32 = 9;
pub const PIXELS: u32 = 10;

pub const TALLY: u32 = 11;

pub const HOT: u32 = 12;

pub const HOT_BUCKETS: u32 = 13;
pub const HOT_COUNTS: u32 = 14;
pub const HOT_BASES: u32 = 15;
pub const HOT_TOTALS: u32 = 16;
pub const HOT_DENSE: u32 = 17;
pub const HOT_GRAND: u32 = 18;
pub const HOT_INDIRECT: u32 = 19;

pub const BINDINGS: u32 = 20;

pub const TALLY_WORDS: usize = Dimension::Two.tally_words();

pub const COARSE: u32 = 3;

pub const BRUSH: usize = 2;
pub const BRUSH_WORDS: usize = 4;

pub const LOOKING: usize = 6;

pub const BRUSH_Z: usize = 8;
pub const LOOKING_Z: usize = 9;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dimension {
    Two,
    Three,
}

impl Dimension {
    #[must_use]
    pub const fn corner_bits(self) -> u32 {
        match self {
            Self::Two => 2,
            Self::Three => 3,
        }
    }

    #[must_use]
    pub const fn corners(self) -> usize {
        1 << self.corner_bits()
    }

    #[must_use]
    pub const fn tally_words(self) -> usize {
        match self {
            Self::Two => 6,
            Self::Three => 7,
        }
    }

    #[must_use]
    pub const fn params_words(self) -> usize {
        match self {
            Self::Two => 8,
            Self::Three => 10,
        }
    }

    #[must_use]
    pub const fn seats(self) -> usize {
        (crate::sand::BOX as usize).pow(self.axes())
    }

    #[must_use]
    pub const fn axes(self) -> u32 {
        match self {
            Self::Two => 2,
            Self::Three => 3,
        }
    }
}
