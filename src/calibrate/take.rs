use std::collections::{BTreeMap, VecDeque};
use std::time::Duration;

use crate::branch::{Failed, Made, World};
use crate::host::Host;

use super::{By, Key, Rate, Sample, Table, device, faults, times};

pub const RUNS: usize = 5;

pub const WINDOW: Duration = Duration::from_millis(600);

pub const WARM_MOST: Duration = Duration::from_millis(1500);

pub const WARM_STEPS: u32 = 4096;

pub const BUDGET: Duration = Duration::from_secs(10);

pub const FILLER: [u32; 3] = [128, 128, 128];

pub const WORLD: [u32; 3] = [64, 64, 64];

pub const SIDES: [u32; 3] = [16, 64, 256];

pub const GAP: Duration = Duration::from_micros(500);

pub const FRAMES: u32 = 32;

pub type Solved = (Rate, Vec<Sample>);

const fn made<'made>(
    rule: &'made str,
    params: &'made [u32],
    size: [u32; 3],
    sparse: bool,
) -> Made<'made> {
    Made {
        rule,
        params,
        size,
        sparse,
        slots: None,
    }
}

fn frame_cells(world: &mut World<'_>) -> Result<u64, Failed> {
    Ok(match world.pool() {
        Some(pool) => {
            let held = world.pool_use()?.map_or(0, |used| used.held);
            u64::from(held).saturating_mul(u64::try_from(pool.cells()).unwrap_or(u64::MAX))
        }
        None => world
            .size()
            .iter()
            .fold(1_u64, |all, side| all.saturating_mul(u64::from(*side))),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pair {
    pub short: Sample,
    pub long: Sample,
}

#[must_use]
pub fn agree(one: Duration, other: Duration) -> bool {
    let (low, high) = (one.min(other).as_nanos(), one.max(other).as_nanos());
    low > 0 && high.saturating_mul(4) <= low.saturating_mul(5)
}

const fn follows(before: (u64, u64), after: (u64, u64), gap: u64) -> bool {
    after.0 >= before.1 && after.0 - before.1 <= gap
}

#[must_use]
pub const fn primed(filler: (u64, u64), primer: (u64, u64), timed: (u64, u64), gap: u64) -> bool {
    follows(filler, primer, gap) && follows(primer, timed, gap)
}

impl Pair {
    #[must_use]
    pub fn agrees(&self, other: &Self) -> bool {
        agree(self.short.time, other.short.time) && agree(self.long.time, other.long.time)
    }

    fn least(self, other: Self) -> Self {
        let less = |one: Sample, two: Sample| Sample {
            units: one.units.max(two.units),
            time: one.time.min(two.time),
            ..one
        };
        Self {
            short: less(self.short, other.short),
            long: less(self.long, other.long),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pass {
    pub sizes: Vec<Pair>,
}

impl Pass {
    #[must_use]
    pub fn agrees(&self, other: &Self) -> bool {
        !self.sizes.is_empty()
            && self.sizes.len() == other.sizes.len()
            && self
                .sizes
                .iter()
                .zip(&other.sizes)
                .all(|(one, two)| one.agrees(two))
    }

    fn least(&self, other: &Self) -> Self {
        Self {
            sizes: self
                .sizes
                .iter()
                .zip(&other.sizes)
                .map(|(one, two)| one.least(*two))
                .collect(),
        }
    }
}

#[derive(Debug, Default)]
pub struct Passes {
    held: BTreeMap<Key, (Option<Pass>, Pass)>,
    first: BTreeMap<Key, Pass>,
}

impl Passes {
    pub fn add(&mut self, key: &Key, pass: Pass) {
        let before = self.held.remove(key).map(|(_, last)| last);
        self.first
            .entry(key.clone())
            .or_insert_with(|| pass.clone());
        self.held.insert(key.clone(), (before, pass));
    }

    #[must_use]
    pub fn agreed(&self, key: &Key) -> Option<Solved> {
        let (Some(before), last) = self.held.get(key)? else {
            return None;
        };
        if !before.agrees(last) {
            return None;
        }
        let least = before.least(last);
        let pairs: Vec<(Sample, Sample)> = least
            .sizes
            .iter()
            .map(|pair| (pair.short, pair.long))
            .collect();
        Rate::framed(&pairs).map(|rate| {
            (
                rate,
                least
                    .sizes
                    .iter()
                    .flat_map(|pair| [pair.short, pair.long])
                    .collect(),
            )
        })
    }

    #[must_use]
    pub fn settled(&self, keys: &[Key]) -> bool {
        keys.iter().all(|key| self.agreed(key).is_some())
    }

    #[must_use]
    pub fn solved(&self) -> BTreeMap<Key, Solved> {
        self.held
            .keys()
            .filter_map(|key| self.agreed(key).map(|solved| (key.clone(), solved)))
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    Agreed,
    Spent,
    Lost,
    Untimed,
}

#[derive(Debug)]
pub struct Measured {
    pub rates: BTreeMap<Key, Solved>,
    pub first: BTreeMap<Key, Pass>,
    pub ended: Ended,
    pub passes: u32,
    pub tried: u32,
    pub counted: u32,
    pub warm: Duration,
    pub made: Duration,
    pub timed: Duration,
}

impl Measured {
    const fn none(ended: Ended) -> Self {
        Self {
            rates: BTreeMap::new(),
            first: BTreeMap::new(),
            ended,
            passes: 0,
            tried: 0,
            counted: 0,
            warm: Duration::ZERO,
            made: Duration::ZERO,
            timed: Duration::ZERO,
        }
    }
}

enum Stop {
    Lost,
    Spent,
    Failed(Failed),
}

impl From<Failed> for Stop {
    fn from(why: Failed) -> Self {
        Self::Failed(why)
    }
}

struct Window {
    spans: VecDeque<(u64, u64)>,
    tick: f64,
}

impl Window {
    fn of(host: &Host) -> Self {
        Self {
            spans: VecDeque::new(),
            tick: f64::from(host.ticking().0).max(f64::MIN_POSITIVE),
        }
    }

    fn ticks(&self, time: Duration) -> u64 {
        (time.as_nanos() as f64 / self.tick) as u64
    }

    fn time(&self, ticks: u64) -> Duration {
        Duration::from_nanos((ticks as f64 * self.tick) as u64)
    }

    fn push(&mut self, (first, last): (u64, u64)) {
        if self.spans.back().is_some_and(|(_, end)| first < *end) {
            self.spans.clear();
        }
        self.spans.push_back((first, last));
        let reach = self.ticks(WINDOW).saturating_mul(2);
        while self
            .spans
            .front()
            .is_some_and(|(_, end)| last.saturating_sub(*end) > reach)
        {
            self.spans.pop_front();
        }
    }

    fn busy(&self) -> bool {
        let Some(&(_, end)) = self.spans.back() else {
            return false;
        };
        let width = self.ticks(WINDOW);
        let Some(from) = end.checked_sub(width) else {
            return false;
        };
        let held: u64 = self
            .spans
            .iter()
            .map(|(first, last)| last.saturating_sub((*first).max(from)))
            .sum();
        held.saturating_mul(2) >= width
    }
}

#[derive(Default)]
struct Clocked {
    started: bool,
    end: u64,
    used: u64,
    marked: bool,
    gap: u64,
}

impl Clocked {
    const fn add(&mut self, (first, last): (u64, u64)) {
        let from = if self.started && first >= self.end {
            self.end
        } else {
            first
        };
        if self.marked {
            self.gap = first.saturating_sub(from);
            self.marked = false;
        }
        self.used = self.used.saturating_add(last.saturating_sub(from));
        self.end = last;
        self.started = true;
    }
}

struct Warm<'host, 'world> {
    window: Window,
    clock: Clocked,
    filler: &'world mut World<'host>,
    since: Option<u64>,
    steps: u32,
    tried: u32,
    counted: u32,
}

impl Warm<'_, '_> {
    const fn begin(&mut self) {
        self.since = None;
        self.steps = 0;
    }

    fn note(&mut self, span: Option<(u64, u64)>) -> Result<(u64, u64), Stop> {
        let Some((first, last)) = span else {
            return Err(Stop::Lost);
        };
        self.window.push((first, last));
        self.clock.add((first, last));
        let since = *self.since.get_or_insert(first);
        self.steps = self.steps.saturating_add(1);
        if last.saturating_sub(since) > self.window.ticks(WARM_MOST) || self.steps > WARM_STEPS {
            return Err(Stop::Lost);
        }
        if self.clock.used > self.window.ticks(BUDGET) {
            return Err(Stop::Spent);
        }
        Ok((first, last))
    }

    fn fill(&mut self) -> Result<(u64, u64), Stop> {
        self.filler.step(FRAMES)?;
        let span = self.filler.on_device_ticks();
        self.note(span)
    }

    fn hold(&mut self) -> Result<(), Stop> {
        faults::waited();
        while !self.window.busy() {
            self.fill()?;
        }
        Ok(())
    }

    fn timed(&mut self, world: &mut World<'_>, frames: u32) -> Result<Duration, Stop> {
        self.begin();
        let gap = self.window.ticks(GAP);
        loop {
            self.hold()?;
            let filler = self.fill()?;
            let primer_from = world.frame();
            world.step(frames)?;
            let primer = self.note(world.on_device_ticks())?;
            faults::held();
            let timed_from = world.frame();
            world.step(frames)?;
            faults::timed();
            let timed = self.note(world.on_device_ticks())?;
            self.tried = self.tried.saturating_add(1);
            if primed(filler, primer, timed, gap) && self.window.busy() {
                self.counted = self.counted.saturating_add(1);
                faults::primed(
                    frames,
                    (primer_from, timed_from, world.frame()),
                    (
                        primer.0.saturating_sub(filler.1),
                        timed.0.saturating_sub(primer.1),
                    ),
                    gap,
                );
                return world
                    .on_device()
                    .map(|frame| frame * frames)
                    .ok_or(Stop::Lost);
            }
        }
    }
}

fn point(warm: &mut Warm<'_, '_>, world: &mut World<'_>, frames: u32) -> Result<Sample, Stop> {
    let units = frame_cells(world)?.saturating_mul(u64::from(frames));
    let mut least: Option<Duration> = None;
    for _ in 0..RUNS {
        let time = warm.timed(world, frames)?;
        least = Some(least.map_or(time, |held| held.min(time)));
    }
    Ok(Sample {
        units,
        frames,
        submits: 1,
        time: least.unwrap_or_default(),
    })
}

fn pass(warm: &mut Warm<'_, '_>, worlds: &mut [World<'_>]) -> Result<Pass, Stop> {
    let mut sizes = Vec::with_capacity(worlds.len());
    for world in worlds {
        sizes.push(Pair {
            short: point(warm, world, 1)?,
            long: point(warm, world, FRAMES)?,
        });
    }
    Ok(Pass { sizes })
}

fn prepared<'host>(
    host: &'host Host,
    clock: &mut Clocked,
    budget: u64,
) -> Result<Vec<(Key, Vec<World<'host>>)>, Stop> {
    let rules = crate::serve3::rules_on(Some(host));
    let mut prepared = Vec::new();
    for key in super::up_front(host) {
        let Key::Frame {
            sparse,
            by: By::Rule(name),
        } = &key
        else {
            continue;
        };
        let Some(rule) = rules.iter().find(|rule| rule.name() == name) else {
            continue;
        };
        let params = vec![0_u32; rule.params()];
        let mut worlds = Vec::new();
        for side in SIDES {
            if let Ok(mut world) = World::create(host, &made(name, &params, [side; 3], *sparse)) {
                world.step(1)?;
                clock.add(world.on_device_ticks().ok_or(Stop::Lost)?);
                if clock.used > budget {
                    return Err(Stop::Spent);
                }
                worlds.push(world);
            }
        }
        if !worlds.is_empty() {
            prepared.push((key, worlds));
        }
    }
    Ok(prepared)
}

pub fn calibrated(host: &Host) -> Result<Table, Failed> {
    let mut table = Table::new(device(host));
    for (key, (rate, _)) in measured(host)?.rates {
        table.set(key, rate);
    }
    Ok(table)
}

pub fn measured(host: &Host) -> Result<Measured, Failed> {
    if !times(host) {
        return Ok(Measured::none(Ended::Untimed));
    }
    let _sole = host.sole()?;
    let mut filler = World::create(host, &made("sand", &[], FILLER, false))?;
    filler.step(1)?;
    let origin = filler.on_device_ticks();
    let window = Window::of(host);
    let mut clock = Clocked::default();
    if let Some(span) = origin {
        clock.add(span);
    }
    let before = clock.used;
    let mut worlds = match prepared(host, &mut clock, window.ticks(BUDGET)) {
        Ok(worlds) => worlds,
        Err(Stop::Lost) => return Ok(Measured::none(Ended::Lost)),
        Err(Stop::Spent) => return Ok(Measured::none(Ended::Spent)),
        Err(Stop::Failed(why)) => return Err(why),
    };
    let created = clock.used;
    clock.marked = true;
    let mut warm = Warm {
        window,
        clock,
        filler: &mut filler,
        since: None,
        steps: 0,
        tried: 0,
        counted: 0,
    };
    warm.begin();
    match warm.hold() {
        Ok(()) => {}
        Err(Stop::Lost) => return Ok(Measured::none(Ended::Lost)),
        Err(Stop::Spent) => return Ok(Measured::none(Ended::Spent)),
        Err(Stop::Failed(why)) => return Err(why),
    }
    let warmed = warm.clock.used;
    let keys: Vec<Key> = worlds.iter().map(|(key, _)| key.clone()).collect();
    let mut passes = Passes::default();
    let mut count = 0_u32;
    let ended = 'passes: loop {
        for (key, world) in &mut worlds {
            match pass(&mut warm, world) {
                Ok(one) => passes.add(key, one),
                Err(Stop::Lost) => break 'passes Ended::Lost,
                Err(Stop::Spent) => break 'passes Ended::Spent,
                Err(Stop::Failed(why)) => return Err(why),
            }
        }
        count = count.saturating_add(1);
        if passes.settled(&keys) {
            break Ended::Agreed;
        }
    };
    let gap = warm.clock.gap;
    let used = warm.clock.used;
    Ok(Measured {
        rates: if ended == Ended::Lost {
            BTreeMap::new()
        } else {
            passes.solved()
        },
        first: passes.first,
        ended,
        passes: count,
        tried: warm.tried,
        counted: warm.counted,
        warm: warm
            .window
            .time(warmed.saturating_sub(created).saturating_sub(gap)),
        made: warm
            .window
            .time(created.saturating_sub(before).saturating_add(gap)),
        timed: warm.window.time(used.saturating_sub(warmed)),
    })
}
