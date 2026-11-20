use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::clock::Time;
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

pub const REFUSED_MOST: u32 = 2;

pub const FIXED_MOVES_AFTER: usize = 3;

pub const FIXED_KEPT: usize = 2 * FIXED_MOVES_AFTER - 1;

#[derive(Debug, Clone, Copy)]
struct Batch {
    frames: u32,
    spent: Duration,
}

#[derive(Debug, Clone, Copy)]
pub struct Pace {
    last: Batch,
    previous: Batch,
    fixed: [Option<Duration>; FIXED_KEPT],
    measured: u32,
    refused: u32,
    one_frame: [Option<Duration>; 2],
    wanted: bool,
    probing: bool,
    calls: u32,
}

impl Pace {
    #[must_use]
    pub fn first(cells: u64) -> Self {
        let guess = Batch {
            frames: 1,
            spent: WORST_PER_CELL
                .checked_mul(u32::try_from(cells).unwrap_or(u32::MAX))
                .unwrap_or(Duration::MAX)
                .saturating_add(WORST_PER_SUBMIT),
        };
        Self {
            last: guess,
            previous: guess,
            fixed: [None; FIXED_KEPT],
            measured: 0,
            refused: 0,
            one_frame: [None; 2],
            wanted: false,
            probing: false,
            calls: 0,
        }
    }

    fn known(&self) -> Option<Duration> {
        let mut kept = self.fixed;
        kept.sort_unstable();
        let filled = kept.iter().flatten().count();
        let at = FIXED_KEPT - filled + filled.saturating_sub(FIXED_MOVES_AFTER);
        kept.get(at).copied().flatten()
    }

    fn doubted(&self, time: Duration) -> bool {
        let fixed = self.known();
        fixed.is_some()
            && within(self.last, fixed, time) == 0
            && within(self.previous, fixed, time) == 0
    }

    #[must_use]
    pub fn fixed(&self) -> Duration {
        self.known().unwrap_or(WORST_PER_SUBMIT)
    }

    #[must_use]
    pub fn cost(&self, frames: u32) -> Duration {
        if frames > self.last.frames {
            return predicted(self.last, None, frames);
        }
        predicted(self.last, self.known(), frames)
    }

    #[must_use]
    pub fn batch(&self, left: u32, time: Duration) -> u32 {
        if time.is_zero() {
            return 0;
        }
        if self.measured == 0 {
            return left.min(1);
        }
        let fixed = self.known();
        let aimed = within(self.last, fixed, AIM).max(1);
        let fits = within(self.last, fixed, time);
        let mut step = aimed.min(fits).min(BATCH_MOST).min(left);
        if step > self.last.frames {
            let proven = within(self.last, None, time);
            let grown = within(self.last, None, AIM).max(self.last.frames);
            step = step.min(proven).min(grown);
        }
        step
    }

    pub fn opening(&mut self, left: u32, time: Duration) -> u32 {
        self.calls = self.calls.saturating_add(1);
        let step = self.batch(left, time);
        if left == 0 || time.is_zero() {
            return step;
        }
        if step > 0 {
            return if self.wanted { 1 } else { step };
        }
        let bound = Some(Duration::ZERO);
        let shown = within(self.last, bound, time) == 0 && within(self.previous, bound, time) == 0;
        let doubted = !shown && self.doubted(time);
        if doubted || shown {
            self.wanted = true;
        }
        let vouched = doubted
            && self.calls <= 1
            && self
                .one_frame
                .iter()
                .all(|one| one.is_some_and(|one| one > time));
        if self.measured >= 2 && (shown || vouched) && self.refused < REFUSED_MOST {
            self.refused += 1;
            return 0;
        }
        self.probing = doubted || shown;
        1
    }

    #[must_use]
    pub fn resumed(&self, left: u32, time: Duration) -> u32 {
        let step = self.batch(left, time);
        if step > 0 || left == 0 || time.is_zero() {
            return step;
        }
        1
    }

    pub fn ran(&mut self, frames: u32, spent: Duration) {
        if frames == 0 {
            return;
        }
        let now = Batch { frames, spent };
        if self.measured > 0
            && let Some(fixed) = split(self.last, now)
        {
            self.fixed.rotate_right(1);
            if let Some(newest) = self.fixed.first_mut() {
                *newest = Some(fixed);
            }
        }
        if frames == 1 {
            self.one_frame = [Some(spent), self.one_frame[0]];
            self.wanted = self.wanted && self.probing;
            self.calls = 0;
        }
        self.previous = self.last;
        self.last = now;
        self.refused = 0;
        self.probing = false;
        self.measured = self.measured.saturating_add(1);
    }
}

fn apart(one: Batch, other: Batch) -> bool {
    let (small, large) = if one.frames <= other.frames {
        (one, other)
    } else {
        (other, one)
    };
    2 * u64::from(large.frames) >= 3 * u64::from(small.frames)
}

fn split(one: Batch, other: Batch) -> Option<Duration> {
    let (small, large) = if one.frames <= other.frames {
        (one, other)
    } else {
        (other, one)
    };
    if !apart(one, other) || large.spent < small.spent {
        return None;
    }
    let (n_small, n_large) = (u128::from(small.frames), u128::from(large.frames));
    let crossed = n_large.checked_mul(small.spent.as_nanos())?;
    let rising = n_small.checked_mul(large.spent.as_nanos())?;
    let fixed = crossed.checked_sub(rising)? / (n_large - n_small);
    u64::try_from(fixed).ok().map(Duration::from_nanos)
}

fn predicted(batch: Batch, fixed: Option<Duration>, frames: u32) -> Duration {
    let Some(fixed) = fixed else {
        if frames < batch.frames {
            return batch.spent;
        }
        return predicted(batch, Some(Duration::ZERO), frames);
    };
    let fixed = fixed.min(batch.spent);
    let each = batch.spent.saturating_sub(fixed).as_nanos();
    let more = each.saturating_mul(u128::from(frames)) / u128::from(batch.frames.max(1));
    fixed.saturating_add(Duration::from_nanos(
        u64::try_from(more).unwrap_or(u64::MAX),
    ))
}

fn within(batch: Batch, fixed: Option<Duration>, time: Duration) -> u32 {
    let Some(fixed) = fixed else {
        if batch.spent > time {
            return 0;
        }
        return within(batch, Some(Duration::ZERO), time);
    };
    let fixed = fixed.min(batch.spent);
    let Some(room) = time.checked_sub(fixed) else {
        return 0;
    };
    let each = batch.spent.saturating_sub(fixed).as_nanos();
    if each == 0 {
        return u32::MAX;
    }
    let frames = room.as_nanos().saturating_mul(u128::from(batch.frames)) / each;
    u32::try_from(frames).unwrap_or(u32::MAX)
}

#[derive(Debug, Clone)]
pub struct Clock {
    pub began: Instant,
    pub deadline: Instant,
    pub time: Arc<dyn Time>,
}

impl Clock {
    #[must_use]
    pub fn of(time: Arc<dyn Time>, arrived: Instant, budget_ms: u64) -> Self {
        let budget = Duration::from_millis(budget_ms.min(BUDGET_MOST_MS));
        Self {
            began: arrived,
            deadline: arrived + budget,
            time,
        }
    }

    #[must_use]
    pub fn left(&self) -> Duration {
        self.deadline.saturating_duration_since(self.time.now())
    }

    #[must_use]
    pub fn spent(&self) -> Duration {
        self.time.since(self.began)
    }
}

#[derive(Debug, Clone)]
pub struct Guard {
    pub deadline: Option<Instant>,
    pub cancel: Option<Arc<AtomicBool>>,
    pub time: Arc<dyn Time>,
}

impl Default for Guard {
    fn default() -> Self {
        Self {
            deadline: None,
            cancel: None,
            time: crate::clock::real(),
        }
    }
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
            .is_some_and(|deadline| self.time.now() >= deadline)
            .then_some(Stop::Budget)
    }

    #[must_use]
    pub fn left(&self) -> Duration {
        self.deadline.map_or(Duration::MAX, |deadline| {
            deadline.saturating_duration_since(self.time.now())
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

pub const WINDOW: usize = 9;

pub const STEP_MOST_FACTOR: f64 = 4.0;

const SIMILAR: f64 = 16.0;

const RATIO_LEAST_RECORDED: Duration = Duration::from_millis(50);

const RATIO_LEAST: f64 = 1.0;

const RATIO_MOST: f64 = 8.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Work {
    Build,
    BuildSparse,
    BuildImage,
    BuildImageSparse,
    Fork,
    ForkSparse,
    Save,
    Snap,
    File,
    FileOpen,
    Write,
    WriteSparse,
    Walk,
    Run,
}

impl Work {
    pub const ALL: [Self; 14] = [
        Self::Build,
        Self::BuildSparse,
        Self::BuildImage,
        Self::BuildImageSparse,
        Self::Fork,
        Self::ForkSparse,
        Self::Save,
        Self::Snap,
        Self::File,
        Self::FileOpen,
        Self::Write,
        Self::WriteSparse,
        Self::Walk,
        Self::Run,
    ];

    #[must_use]
    pub const fn build(sparse: bool) -> Self {
        if sparse {
            Self::BuildSparse
        } else {
            Self::Build
        }
    }

    #[must_use]
    pub const fn build_image(sparse: bool) -> Self {
        if sparse {
            Self::BuildImageSparse
        } else {
            Self::BuildImage
        }
    }

    #[must_use]
    pub const fn write(sparse: bool) -> Self {
        if sparse {
            Self::WriteSparse
        } else {
            Self::Write
        }
    }

    #[must_use]
    pub const fn fork(sparse: bool) -> Self {
        if sparse { Self::ForkSparse } else { Self::Fork }
    }

    const fn index(self) -> usize {
        match self {
            Self::Build => 0,
            Self::BuildSparse => 1,
            Self::BuildImage => 2,
            Self::BuildImageSparse => 3,
            Self::Fork => 4,
            Self::ForkSparse => 5,
            Self::Save => 6,
            Self::Snap => 7,
            Self::File => 8,
            Self::FileOpen => 9,
            Self::Write => 10,
            Self::WriteSparse => 11,
            Self::Walk => 12,
            Self::Run => 13,
        }
    }

    const fn prior(self) -> f64 {
        match self {
            Self::Build
            | Self::BuildSparse
            | Self::BuildImage
            | Self::BuildImageSparse
            | Self::Fork
            | Self::ForkSparse => 1e-8,
            Self::Save | Self::Snap => 5e-8,
            Self::File | Self::FileOpen => 2e-8,
            Self::Write | Self::WriteSparse => 2e-4,
            Self::Walk | Self::Run => 1e-8,
        }
    }

    #[must_use]
    pub const fn fastest(self) -> Option<f64> {
        match self {
            Self::Build | Self::BuildImage => Some(FASTEST_BUILD),
            Self::BuildSparse | Self::BuildImageSparse => Some(FASTEST_BUILD_SPARSE),
            Self::Fork | Self::ForkSparse => Some(FASTEST_FORK),
            Self::Save => Some(FASTEST_SAVE),
            Self::Snap => Some(FASTEST_SNAP),
            Self::File | Self::FileOpen => Some(FASTEST_FILE),
            Self::Write | Self::WriteSparse | Self::Walk | Self::Run => None,
        }
    }

    const fn fixed(self) -> f64 {
        match self {
            Self::Write | Self::WriteSparse | Self::Walk | Self::Run => 0.0,
            _ => FIXED_BYTES,
        }
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::BuildSparse => "build_sparse",
            Self::BuildImage => "build_from_image",
            Self::BuildImageSparse => "build_from_image_sparse",
            Self::Fork => "fork",
            Self::ForkSparse => "fork_sparse",
            Self::Save => "save",
            Self::Snap => "snapshot_in_memory",
            Self::File => "file",
            Self::FileOpen => "file_open",
            Self::Write => "write",
            Self::WriteSparse => "write_sparse",
            Self::Walk => "walk",
            Self::Run => "run",
        }
    }
}

pub const FASTEST_BUILD: f64 = 2.0e-10;

pub const FASTEST_BUILD_SPARSE: f64 = 1.5e-10;

pub const FASTEST_FORK: f64 = 1.5e-10;

pub const FASTEST_SAVE: f64 = 1.5e-9;

pub const FASTEST_SNAP: f64 = 2.0e-11;

pub const FASTEST_FILE: f64 = 2.0e-9;

pub const SAMPLES_KEPT: usize = 32;

pub const REMEASURE_AFTER: u32 = 8;

pub const JOURNALS_KEPT: usize = 256;

pub const REFUSALS_KEPT: usize = JOURNALS_KEPT;

const REPLAY_KEY: usize = Work::ALL.len();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Basis {
    Measured,
    Fastest,
    Both,
}

impl Basis {
    #[must_use]
    pub const fn with(self, other: Self) -> Self {
        match (self, other) {
            (Self::Measured, Self::Measured) => Self::Measured,
            (Self::Fastest, Self::Fastest) => Self::Fastest,
            _ => Self::Both,
        }
    }

    #[must_use]
    pub const fn told(self) -> &'static str {
        match self {
            Self::Measured => "by the fastest the server measured for work of this size",
            Self::Fastest => "by the fastest any tested device has shown per byte",
            Self::Both => {
                "by the fastest the server measured for work of this size and, where it measured none, the fastest any tested device has shown per byte"
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
struct Samples {
    seen: std::collections::VecDeque<(u64, f64)>,
}

fn recent(held: impl Iterator<Item = f64>) -> Option<f64> {
    held.take(RECENT).reduce(f64::min)
}

impl Samples {
    fn push(&mut self, bytes: u64, spent: f64) {
        if self.seen.len() == SAMPLES_KEPT {
            self.seen.pop_front();
        }
        self.seen.push_back((bytes, spent));
    }

    fn evidence(&self, bytes: u64) -> Option<(f64, u64)> {
        let class = self
            .seen
            .iter()
            .map(|(size, _)| *size)
            .filter(|size| *size <= bytes)
            .max()?;
        recent(
            self.seen
                .iter()
                .rev()
                .filter(|(size, _)| *size == class)
                .map(|(_, spent)| *spent),
        )
        .map(|spent| (spent, class))
    }
}

#[derive(Debug, Clone, Default)]
struct Kept {
    seen: std::collections::VecDeque<(f64, f64)>,
}

pub const PADDED: usize = 3;

fn median(mut held: Vec<f64>, prior: f64) -> f64 {
    while held.len() < PADDED {
        held.push(prior);
    }
    held.sort_by(f64::total_cmp);
    held.get((held.len() - 1) / 2).copied().unwrap_or(prior)
}

impl Kept {
    fn rate(&self, prior: f64, bytes: f64) -> f64 {
        let chosen = self.chosen(bytes);
        let recent = chosen
            .iter()
            .rev()
            .take(RECENT)
            .copied()
            .fold(0.0_f64, f64::max);
        median(chosen, prior).max(recent)
    }

    fn middle(&self, prior: f64, bytes: f64) -> f64 {
        median(self.chosen(bytes), prior)
    }

    fn chosen(&self, bytes: f64) -> Vec<f64> {
        let similar: Vec<f64> = self
            .seen
            .iter()
            .filter(|(size, _)| *size * SIMILAR >= bytes && *size <= bytes * SIMILAR)
            .map(|(_, rate)| *rate)
            .collect();
        if similar.is_empty() {
            self.seen.iter().map(|(_, rate)| *rate).collect()
        } else {
            similar
        }
    }

    fn push(&mut self, bytes: f64, rate: f64) {
        if self.seen.len() == WINDOW {
            self.seen.pop_front();
        }
        self.seen.push_back((bytes, rate));
    }
}

#[derive(Debug, Clone, Default)]
pub struct Rates {
    kinds: [Kept; Work::ALL.len()],
    replay: Kept,
    samples: [Samples; Work::ALL.len()],
    journals: BTreeMap<u64, std::collections::VecDeque<f64>>,
    journal_order: std::collections::VecDeque<u64>,
    replays_seen: u64,
    refused: BTreeMap<(usize, u64), (u32, u64)>,
    refusal_tick: u64,
}

pub const RECENT: usize = 3;

fn bounded(sample: f64, around: f64, least: f64, most: f64) -> f64 {
    sample.max(around / STEP_MOST_FACTOR).clamp(least, most)
}

impl Rates {
    fn kept(&self, work: Work) -> Option<&Kept> {
        self.kinds.get(work.index())
    }

    #[must_use]
    pub fn rate(&self, work: Work, bytes: u64) -> f64 {
        let size = bytes as f64 + work.fixed();
        self.kept(work)
            .map_or(work.prior(), |kept| kept.rate(work.prior(), size))
    }

    #[must_use]
    pub fn observed(&self, work: Work) -> usize {
        self.kept(work).map_or(0, |kept| kept.seen.len())
    }

    #[must_use]
    pub fn estimate(&self, work: Work, bytes: u64) -> Duration {
        let size = bytes as f64 + work.fixed();
        Duration::try_from_secs_f64(size * self.rate(work, bytes)).unwrap_or(Duration::MAX)
    }

    #[must_use]
    pub fn least(&self, work: Work, bytes: u64) -> Duration {
        self.least_of(work, bytes).0
    }

    #[must_use]
    pub fn least_of(&self, work: Work, bytes: u64) -> (Duration, Basis) {
        let fastest = work.fastest().unwrap_or(0.0) * bytes as f64;
        let evidence = self
            .samples
            .get(work.index())
            .and_then(|samples| samples.evidence(bytes));
        let measured = evidence.map_or(0.0, |(spent, _)| spent);
        let same_size = evidence.is_some_and(|(_, class)| class == bytes);
        let (least, basis) = if same_size || (measured > 0.0 && measured >= fastest) {
            (measured, Basis::Measured)
        } else {
            (fastest, Basis::Fastest)
        };
        (
            Duration::try_from_secs_f64(least).unwrap_or(Duration::MAX),
            basis,
        )
    }

    pub fn refuses(&mut self, work: Work, bytes: u64) -> bool {
        self.refused_at((work.index(), bytes))
    }

    #[must_use]
    pub fn refusals_kept(&self) -> usize {
        self.refused.len()
    }

    pub fn admitted(&mut self, work: Work, bytes: u64) {
        self.refused.remove(&(work.index(), bytes));
    }

    pub fn refuses_replay(&mut self, journal: u64) -> bool {
        self.refused_at((REPLAY_KEY, journal))
    }

    pub fn admitted_replay(&mut self, journal: u64) {
        self.refused.remove(&(REPLAY_KEY, journal));
    }

    fn refused_at(&mut self, key: (usize, u64)) -> bool {
        self.refusal_tick = self.refusal_tick.wrapping_add(1);
        if !self.refused.contains_key(&key)
            && self.refused.len() >= REFUSALS_KEPT
            && let Some(stalest) = self
                .refused
                .iter()
                .min_by_key(|(_, (_, touched))| *touched)
                .map(|(stalest, _)| *stalest)
        {
            self.refused.remove(&stalest);
        }
        let tick = self.refusal_tick;
        let (refused, touched) = self.refused.entry(key).or_insert((0, tick));
        *touched = tick;
        if *refused >= REMEASURE_AFTER {
            *refused = 0;
            return false;
        }
        *refused = refused.saturating_add(1);
        true
    }

    fn sampled(&mut self, work: Work, bytes: u64, spent: Duration) {
        if let Some(samples) = self.samples.get_mut(work.index())
            && !spent.is_zero()
        {
            samples.push(bytes, spent.as_secs_f64());
        }
    }

    pub fn measure(&mut self, work: Work, bytes: u64, spent: Duration) {
        let size = bytes as f64 + work.fixed();
        let sample = spent.as_secs_f64() / size;
        if !sample.is_finite() || sample <= 0.0 {
            return;
        }
        let around = self
            .kept(work)
            .map_or(work.prior(), |kept| kept.middle(work.prior(), size));
        let taken = bounded(sample, around, 0.0, f64::MAX);
        if let Some(kept) = self.kinds.get_mut(work.index()) {
            kept.push(size, taken);
        }
        self.sampled(work, bytes, spent);
    }

    #[must_use]
    pub fn replay(&self) -> f64 {
        self.replay.rate(2.0, 1.0).clamp(RATIO_LEAST, RATIO_MOST)
    }

    #[must_use]
    pub fn replaying(&self, recorded: Duration) -> Duration {
        scaled(recorded, self.replay())
    }

    #[must_use]
    pub fn journal_least(&self, journal: u64) -> Option<Duration> {
        let held = self.journals.get(&journal)?;
        recent(held.iter().rev().copied())
            .map(|spent| Duration::try_from_secs_f64(spent).unwrap_or(Duration::MAX))
    }

    pub fn measure_journal(&mut self, journal: u64, spent: Duration) {
        if spent.is_zero() {
            return;
        }
        if !self.journals.contains_key(&journal) {
            if self.journal_order.len() == JOURNALS_KEPT
                && let Some(oldest) = self.journal_order.pop_front()
            {
                self.journals.remove(&oldest);
            }
            self.journal_order.push_back(journal);
        }
        let held = self.journals.entry(journal).or_default();
        if held.len() == RECENT {
            held.pop_front();
        }
        held.push_back(spent.as_secs_f64());
        self.replays_seen = self.replays_seen.saturating_add(1);
    }

    pub fn measure_replay(&mut self, recorded: Duration, spent: Duration) {
        if recorded < RATIO_LEAST_RECORDED {
            return;
        }
        let sample = spent.as_secs_f64() / recorded.as_secs_f64();
        if !sample.is_finite() || sample <= 0.0 {
            return;
        }
        let around = self.replay.middle(2.0, 1.0).clamp(RATIO_LEAST, RATIO_MOST);
        let taken = bounded(sample, around, RATIO_LEAST, RATIO_MOST);
        self.replay.push(1.0, taken);
    }

    #[must_use]
    pub fn shown(&self) -> Json {
        let mut shown: Vec<(&'static str, Json)> = Work::ALL
            .iter()
            .map(|work| {
                let rate = self.rate(*work, 0);
                let seen = self.kept(*work).map_or(0, |kept| kept.seen.len());
                (
                    work.name(),
                    map(vec![
                        (
                            "bytes_per_second",
                            counted64(if rate > 0.0 { (1.0 / rate) as u64 } else { 0 }),
                        ),
                        ("observations", counted64(seen as u64)),
                    ]),
                )
            })
            .collect();
        shown.push((
            "replay_per_recorded_percent",
            counted64((self.replay() * 100.0) as u64),
        ));
        shown.push(("replay_observations", counted64(self.replays_seen)));
        map(shown)
    }
}

#[must_use]
pub fn scaled(time: Duration, by: f64) -> Duration {
    if by.is_nan() || by <= 0.0 {
        return Duration::ZERO;
    }
    Duration::try_from_secs_f64(time.as_secs_f64() * by).unwrap_or(Duration::MAX)
}

#[must_use]
pub fn too_slow(needs: Duration, left: Duration, what: &str, basis: Basis) -> Refusal {
    Refusal::new(
        Code::Budget,
        "$.budget_ms",
        format!(
            "{what} needs at least {} ms on this device, {}, and the call has {} ms left; nothing ran",
            needs.as_millis(),
            basis.told(),
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
