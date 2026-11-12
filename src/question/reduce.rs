use super::{MOST_BINS, Refused, Region};

pub const RESULT: usize = 24;

pub const COUNTER: usize = 4;

pub const COPIES: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Measure {
    Count,
    Sum,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reduce {
    Count,
    Sum,
    Min,
    Max,
    Histogram { bins: u32 },
    Moments,
    Differ(Measure),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Answer {
    Count(u64),
    Sum(u64),
    Min(u32),
    Max(u32),
    Histogram { bins: Vec<u64>, past: u64 },
    Moments { count: u64, sums: [u64; 3] },
    Differ(u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reads {
    pub pool: u64,
    pub table: u64,
}

impl Reads {
    #[must_use]
    pub fn of(words: &[u32]) -> Self {
        Self {
            pool: wide(words, 0),
            table: wide(words, 1),
        }
    }
}

fn wide(words: &[u32], pair: usize) -> u64 {
    let low = words.get(2 * pair).copied().unwrap_or(0);
    let high = words.get(2 * pair + 1).copied().unwrap_or(0);
    (u64::from(high) << 32) | u64::from(low)
}

impl Reduce {
    pub fn check(self) -> Result<(), Refused> {
        match self {
            Self::Histogram { bins } if bins == 0 || bins > MOST_BINS => {
                Err(Refused::Bins { asked: bins })
            }
            _ => Ok(()),
        }
    }

    #[must_use]
    pub const fn words(self) -> usize {
        match self {
            Self::Count | Self::Sum | Self::Differ(_) => 2,
            Self::Min | Self::Max => 1,
            Self::Histogram { bins } => 2 * (bins as usize + 1) * COPIES,
            Self::Moments => 8,
        }
    }

    #[must_use]
    pub fn initial(self) -> Vec<u32> {
        match self {
            Self::Min => vec![u32::MAX],
            other => vec![0; other.words()],
        }
    }

    #[must_use]
    pub fn most_cells(self, region: Region) -> (u128, &'static str) {
        let total = u128::from(u64::MAX);
        match self {
            Self::Count | Self::Histogram { .. } | Self::Differ(Measure::Count) => {
                (total, "a 64-bit count")
            }
            Self::Sum | Self::Differ(Measure::Sum) => {
                (total / u128::from(u32::MAX), "a 64-bit sum of 32-bit words")
            }
            Self::Min | Self::Max => (u128::MAX, "an extreme"),
            Self::Moments => {
                let far = region
                    .to
                    .iter()
                    .map(|&to| u128::from(to.saturating_sub(1)))
                    .max()
                    .unwrap_or(0)
                    .max(1);
                (total / far, "a 64-bit sum of points")
            }
        }
    }

    #[must_use]
    pub fn answer(self, result: &[u32]) -> Answer {
        match self {
            Self::Count => Answer::Count(wide(result, 0)),
            Self::Sum => Answer::Sum(wide(result, 0)),
            Self::Differ(_) => Answer::Differ(wide(result, 0)),
            Self::Min => Answer::Min(result.first().copied().unwrap_or(u32::MAX)),
            Self::Max => Answer::Max(result.first().copied().unwrap_or(0)),
            Self::Histogram { bins } => {
                let bins = bins as usize;
                let total = |bin: usize| {
                    (0..COPIES)
                        .map(|copy| wide(result, copy * (bins + 1) + bin))
                        .fold(0_u64, u64::wrapping_add)
                };
                Answer::Histogram {
                    bins: (0..bins).map(total).collect(),
                    past: total(bins),
                }
            }
            Self::Moments => Answer::Moments {
                count: wide(result, 0),
                sums: [wide(result, 1), wide(result, 2), wide(result, 3)],
            },
        }
    }
}
