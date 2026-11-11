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

/// A window of `STATE` holds `2^WINDOW_BITS` words, 2 GiB: two of them pass
/// the 4 GiB a binding may hold.
pub const WINDOW_BITS: u32 = 29;

/// How the words of `STATE` lie over bindings. A world of fewer than
/// `2^(bits + 1)` words keeps them in the one binding `STATE`. A larger one
/// is split by index: word `i` lies in window `i >> bits`, at `i` less the
/// window's start, each window `2^bits` words but the last. Window 0 is
/// `STATE`, and window `w` past it is binding `BINDINGS + w - 1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Windows {
    words: usize,
    bits: u32,
}

impl Windows {
    /// `words` laid out at [`WINDOW_BITS`].
    #[must_use]
    pub const fn of(words: usize) -> Self {
        Self::at(words, WINDOW_BITS)
    }

    pub(crate) const fn at(words: usize, bits: u32) -> Self {
        Self { words, bits }
    }

    /// `words` split into windows of `2^bits` words. Only tests build it,
    /// to split a small world.
    #[cfg(feature = "faults")]
    #[must_use]
    pub const fn split_at(words: usize, bits: u32) -> Self {
        Self::at(words, bits)
    }

    #[must_use]
    pub const fn words(self) -> usize {
        self.words
    }

    #[must_use]
    pub const fn bits(self) -> u32 {
        self.bits
    }

    #[must_use]
    pub const fn span(self) -> usize {
        1 << self.bits
    }

    #[must_use]
    pub const fn count(self) -> u32 {
        if self.words < 2 * self.span() {
            1
        } else {
            self.words.div_ceil(self.span()) as u32
        }
    }

    #[must_use]
    pub const fn is_split(self) -> bool {
        self.count() > 1
    }

    /// The bindings a kernel that reads `STATE` declares.
    #[must_use]
    pub const fn bindings(self) -> u32 {
        BINDINGS + self.count() - 1
    }

    #[must_use]
    pub const fn binding(window: u32) -> u32 {
        if window == 0 {
            STATE
        } else {
            BINDINGS + window - 1
        }
    }

    /// The words window `window` holds.
    #[must_use]
    pub const fn held(self, window: u32) -> usize {
        if !self.is_split() {
            return if window == 0 { self.words } else { 0 };
        }
        let start = window as usize * self.span();
        if start >= self.words {
            0
        } else if self.words - start < self.span() {
            self.words - start
        } else {
            self.span()
        }
    }

    /// Where word `index` of `STATE` lies: its binding and the word in it.
    #[must_use]
    pub const fn place(self, index: usize) -> (u32, usize) {
        if !self.is_split() {
            return (STATE, index);
        }
        let window = (index >> self.bits) as u32;
        (Self::binding(window), index & (self.span() - 1))
    }

    /// `sizes` with `STATE` in its first window and the other windows
    /// appended after the engine's bindings.
    #[must_use]
    pub fn laid(self, sizes: &[usize]) -> Vec<usize> {
        let mut bound = sizes.to_vec();
        if !self.is_split() {
            return bound;
        }
        if let Some(state) = bound.get_mut(STATE as usize) {
            *state = self.held(0);
        }
        if bound.len() < BINDINGS as usize {
            bound.resize(BINDINGS as usize, 1);
        }
        for window in 1..self.count() {
            bound.push(self.held(window));
        }
        bound
    }
}
