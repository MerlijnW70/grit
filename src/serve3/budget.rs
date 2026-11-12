use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use crate::wire::{Json, map};

use super::errors::{Code, Refusal};

pub const BUDGET_MS: u64 = 30_000;

pub const BUDGET_MOST_MS: u64 = 600_000;

pub const SUBMIT_MOST_MS: u64 = 250;

pub const BATCH_MOST: u32 = 64;

const AIM: Duration = Duration::from_millis(SUBMIT_MOST_MS / 2);

const WORST_PER_CELL: Duration = Duration::from_nanos(100);

const WORST_PER_SUBMIT: Duration = Duration::from_millis(1);

pub const SHARE_NUMERATOR: u64 = 3;

pub const SHARE_DENOMINATOR: u64 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    Budget,
    Cancelled,
}

impl Stop {
    #[must_use]
    pub const fn code(self) -> Code {
        match self {
            Self::Budget => Code::Budget,
            Self::Cancelled => Code::Cancelled,
        }
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        self.code().name()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Pace {
    per_frame: Duration,
    measured: bool,
    last: u32,
}

impl Pace {
    #[must_use]
    pub fn first(cells: u64) -> Self {
        let guess = WORST_PER_CELL
            .checked_mul(u32::try_from(cells).unwrap_or(u32::MAX))
            .unwrap_or(Duration::MAX)
            .saturating_add(WORST_PER_SUBMIT);
        Self {
            per_frame: guess,
            measured: false,
            last: BATCH_MOST,
        }
    }

    #[must_use]
    pub fn batch(&self, left: u32, time: Duration) -> u32 {
        if time.is_zero() {
            return 0;
        }
        let aimed = frames_in(AIM, self.per_frame).max(1);
        let grown = self.last.saturating_mul(2).max(1);
        let fits = frames_in(time, self.per_frame);
        let fits = if self.measured { fits } else { fits.max(1) };
        aimed.min(grown).min(BATCH_MOST).min(left).min(fits)
    }

    pub fn ran(&mut self, frames: u32, spent: Duration) {
        if frames == 0 {
            return;
        }
        self.per_frame = spent / frames;
        self.measured = true;
        self.last = frames;
    }

    #[must_use]
    pub const fn per_frame(&self) -> Duration {
        self.per_frame
    }
}

fn frames_in(time: Duration, per_frame: Duration) -> u32 {
    if per_frame.is_zero() {
        return u32::MAX;
    }
    u32::try_from(time.as_nanos() / per_frame.as_nanos()).unwrap_or(u32::MAX)
}

#[derive(Debug, Clone)]
pub struct Clock {
    pub began: Instant,
    pub deadline: Instant,
}

impl Clock {
    #[must_use]
    pub fn of(arrived: Instant, budget_ms: u64) -> Self {
        let budget = Duration::from_millis(budget_ms.min(BUDGET_MOST_MS));
        Self {
            began: arrived,
            deadline: arrived + budget,
        }
    }

    #[must_use]
    pub fn left(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }

    #[must_use]
    pub fn spent(&self) -> Duration {
        self.began.elapsed()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Short {
    pub asked: u64,
    pub held: u64,
    pub allowed: u64,
    pub what: String,
}

impl Short {
    #[must_use]
    pub fn refusal(&self) -> Refusal {
        Refusal::new(
            Code::Memory,
            "$",
            format!(
                "{} needs {} bytes on the device, {} are held by open worlds, and the server allows {} in all",
                self.what, self.asked, self.held, self.allowed
            ),
        )
        .limited(counted64(self.allowed))
    }

    #[must_use]
    pub fn told(&self) -> Vec<(&'static str, Json)> {
        vec![
            ("asked", counted64(self.asked)),
            ("held", counted64(self.held)),
            ("allowed", counted64(self.allowed)),
        ]
    }
}

impl std::fmt::Display for Short {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(out, "{}", self.refusal().message)
    }
}

impl std::error::Error for Short {}

#[must_use]
pub fn counted64(held: u64) -> Json {
    i64::try_from(held).map_or(Json::Null, Json::Int)
}

#[derive(Debug, Clone, Default)]
pub struct Memory {
    allowed: u64,
    held: BTreeMap<u32, u64>,
}

impl Memory {
    #[must_use]
    pub const fn new(allowed: u64) -> Self {
        Self {
            allowed,
            held: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn share_of(heap: u64) -> u64 {
        u64::try_from(
            u128::from(heap) * u128::from(SHARE_NUMERATOR) / u128::from(SHARE_DENOMINATOR),
        )
        .unwrap_or(u64::MAX)
    }

    #[must_use]
    pub const fn allowed(&self) -> u64 {
        self.allowed
    }

    pub const fn set_allowed(&mut self, allowed: u64) {
        self.allowed = allowed;
    }

    #[must_use]
    pub fn held(&self) -> u64 {
        self.held
            .values()
            .fold(0_u64, |all, one| all.saturating_add(*one))
    }

    #[must_use]
    pub fn of(&self, world: u32) -> u64 {
        self.held.get(&world).copied().unwrap_or(0)
    }

    pub fn admits(&self, asked: u64, what: impl Into<String>) -> Result<(), Short> {
        let held = self.held();
        if held
            .checked_add(asked)
            .is_some_and(|all| all <= self.allowed)
        {
            return Ok(());
        }
        Err(Short {
            asked,
            held,
            allowed: self.allowed,
            what: what.into(),
        })
    }

    pub fn hold(&mut self, world: u32, bytes: u64) {
        self.held.insert(world, bytes);
    }

    pub fn free(&mut self, world: u32) {
        self.held.remove(&world);
    }

    #[must_use]
    pub fn shown(&self) -> Json {
        map(vec![
            ("allowed_bytes", counted64(self.allowed)),
            ("held_bytes", counted64(self.held())),
        ])
    }
}

#[must_use]
pub fn bytes_of(sizes: &[usize]) -> u64 {
    sizes.iter().fold(0_u64, |all, &words| {
        let one = u64::try_from(words.max(1))
            .unwrap_or(u64::MAX)
            .saturating_mul(4);
        all.saturating_add(one)
    })
}
