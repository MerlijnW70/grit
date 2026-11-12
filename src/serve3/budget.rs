use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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

#[derive(Debug, Clone, Default)]
pub struct Guard {
    pub deadline: Option<Instant>,
    pub cancel: Option<Arc<AtomicBool>>,
}

impl Guard {
    #[must_use]
    pub fn stop(&self) -> Option<Stop> {
        if self
            .cancel
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::SeqCst))
        {
            return Some(Stop::Cancelled);
        }
        self.deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
            .then_some(Stop::Budget)
    }

    #[must_use]
    pub fn left(&self) -> Duration {
        self.deadline.map_or(Duration::MAX, |deadline| {
            deadline.saturating_duration_since(Instant::now())
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cut(pub Stop);

impl std::fmt::Display for Cut {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(out, "the replay stopped on {}", self.0.name())
    }
}

impl std::error::Error for Cut {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Halted {
    pub stop: Stop,
    pub entries: usize,
    pub of: usize,
    pub frame: u32,
    pub goal: u32,
}

impl Halted {
    #[must_use]
    pub fn refusal(&self) -> Refusal {
        let field = match self.stop {
            Stop::Budget => "$.budget_ms",
            Stop::Cancelled => "$",
        };
        Refusal::new(
            self.stop.code(),
            field,
            format!(
                "the world's journal was replayed through {} of its {} entries, to frame {} of {}, before the call stopped on {}; nothing was opened",
                self.entries,
                self.of,
                self.frame,
                self.goal,
                self.stop.name()
            ),
        )
    }
}

impl std::fmt::Display for Halted {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(out, "{}", self.refusal().message)
    }
}

impl std::error::Error for Halted {}

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

pub const WORLD_LEAST: u64 = 1 << 20;

const FIXED_BYTES: f64 = 4_194_304.0;

const EASING: f64 = 0.75;

const RATIO_LEAST_RECORDED: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Work {
    Build,
    Fork,
    Save,
    File,
}

impl Work {
    const fn index(self) -> usize {
        match self {
            Self::Build => 0,
            Self::Fork => 1,
            Self::Save => 2,
            Self::File => 3,
        }
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Build => "building a world",
            Self::Fork => "copying a world",
            Self::Save => "writing a world to a file",
            Self::File => "reading a world file",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Rates {
    seconds_per_byte: [f64; 4],
    measured: [bool; 4],
    replay: f64,
    replay_measured: bool,
}

impl Default for Rates {
    fn default() -> Self {
        Self {
            seconds_per_byte: [2e-9, 2e-9, 4e-9, 4e-9],
            measured: [false; 4],
            replay: 2.0,
            replay_measured: false,
        }
    }
}

fn eased(old: f64, sample: f64) -> f64 {
    if !sample.is_finite() || sample <= 0.0 {
        return old;
    }
    sample.max(old * EASING)
}

impl Rates {
    #[must_use]
    pub fn estimate(&self, work: Work, bytes: u64) -> Duration {
        let rate = self
            .seconds_per_byte
            .get(work.index())
            .copied()
            .unwrap_or(1e-8);
        Duration::try_from_secs_f64((bytes as f64 + FIXED_BYTES) * rate).unwrap_or(Duration::MAX)
    }

    pub fn measure(&mut self, work: Work, bytes: u64, spent: Duration) {
        let index = work.index();
        let sample = spent.as_secs_f64() / (bytes as f64 + FIXED_BYTES);
        if let (Some(rate), Some(measured)) = (
            self.seconds_per_byte.get_mut(index),
            self.measured.get_mut(index),
        ) {
            *rate = if *measured {
                eased(*rate, sample)
            } else {
                sample
            };
            *measured = true;
        }
    }

    #[must_use]
    pub const fn replay(&self) -> f64 {
        self.replay
    }

    #[must_use]
    pub fn replaying(&self, recorded: Duration) -> Duration {
        recorded.mul_f64(self.replay.max(1.0))
    }

    pub fn measure_replay(&mut self, recorded: Duration, spent: Duration) {
        if recorded < RATIO_LEAST_RECORDED {
            return;
        }
        let sample = spent.as_secs_f64() / recorded.as_secs_f64();
        self.replay = if self.replay_measured {
            eased(self.replay, sample)
        } else {
            sample.max(1.0)
        };
        self.replay_measured = true;
    }

    #[must_use]
    pub fn shown(&self) -> Json {
        let one = |work: Work| {
            let index = work.index();
            let rate = self.seconds_per_byte.get(index).copied().unwrap_or(0.0);
            map(vec![
                (
                    "bytes_per_second",
                    counted64(if rate > 0.0 { (1.0 / rate) as u64 } else { 0 }),
                ),
                (
                    "measured",
                    Json::Bool(self.measured.get(index).copied().unwrap_or(false)),
                ),
            ])
        };
        map(vec![
            ("build", one(Work::Build)),
            ("fork", one(Work::Fork)),
            ("save", one(Work::Save)),
            ("file", one(Work::File)),
            (
                "replay_per_recorded_percent",
                counted64((self.replay * 100.0) as u64),
            ),
            ("replay_measured", Json::Bool(self.replay_measured)),
        ])
    }
}

#[must_use]
pub fn too_slow(needs: Duration, left: Duration, what: &str) -> Refusal {
    Refusal::new(
        Code::Budget,
        "$.budget_ms",
        format!(
            "{what} needs about {} ms on this device, by the rates the server has measured, and the call has {} ms left; nothing ran",
            needs.as_millis(),
            left.as_millis()
        ),
    )
    .limited(counted64(
        u64::try_from(needs.as_millis().saturating_add(1)).unwrap_or(u64::MAX),
    ))
}

#[must_use]
pub fn world_cost(sizes: &[usize]) -> u64 {
    bytes_of(sizes).max(WORLD_LEAST)
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
