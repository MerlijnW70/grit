use std::collections::{BTreeMap, VecDeque};
use std::time::Duration;

use crate::branch::{Failed, Made, World};
use crate::host::Host;

use super::{Key, Rate, Sample, Table, device, faults, times};

pub const RUNS: usize = 5;

pub const WINDOW: Duration = Duration::from_millis(600);

pub const WARM_MOST: Duration = Duration::from_millis(1500);

pub const WARM_STEPS: u32 = 4096;

pub const SMALL: [u32; 3] = [64, 64, 64];

pub const LARGE: [u32; 3] = [128, 128, 128];

pub const SPARSE: [u32; 3] = [64, 64, 64];

pub const FRAMES: u32 = 32;

const SPARSE_BRICKS: u32 = 2;

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

fn spread(world: &World<'_>, count: usize) -> Vec<([u32; 3], u32)> {
    let size = world.size();
    let sides = world
        .pool()
        .map_or([4, 4, 4], |pool| pool.sides())
        .map(|side| side.max(1));
    let lattice = [0, 1, 2].map(|axis| {
        size.get(axis)
            .copied()
            .unwrap_or(1)
            .div_ceil(sides.get(axis).copied().unwrap_or(1))
    });
    let mut cells = Vec::with_capacity(count);
    'filled: for z in 0..lattice[2] {
        for y in 0..lattice[1] {
            for x in 0..lattice[0] {
                if cells.len() == count {
                    break 'filled;
                }
                cells.push(([x * sides[0], y * sides[1], z * sides[2]], 1));
            }
        }
    }
    cells
}

#[must_use]
pub fn seeded(size: [u32; 3]) -> Vec<([u32; 3], u32)> {
    let [wide, deep, tall] = size.map(|side| side.max(1));
    let mut state = 0x9E37_79B9_u32;
    let count = wide.saturating_mul(deep).saturating_mul(tall) / 10;
    (0..count)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (
                [state % wide, (state >> 7) % deep, (state >> 14) % tall],
                1 + (state >> 21) % 3,
            )
        })
        .collect()
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

    fn push(&mut self, span: Option<(u64, u64)>) -> bool {
        let Some((first, last)) = span else {
            return false;
        };
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
        true
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

struct Warm<'host, 'world> {
    window: Window,
    filler: &'world mut World<'host>,
    since: Option<u64>,
    steps: u32,
}

impl Warm<'_, '_> {
    fn begin(&mut self) {
        self.since = None;
        self.steps = 0;
    }

    fn spent(&mut self, span: Option<(u64, u64)>) -> bool {
        let Some((first, last)) = span else {
            return false;
        };
        let since = *self.since.get_or_insert(first);
        self.steps = self.steps.saturating_add(1);
        last.saturating_sub(since) <= self.window.ticks(WARM_MOST) && self.steps <= WARM_STEPS
    }

    fn fill(&mut self) -> Result<bool, Failed> {
        self.filler.step(FRAMES)?;
        let span = self.filler.on_device_ticks();
        Ok(self.window.push(span) && self.spent(span))
    }

    fn hold(&mut self) -> Result<bool, Failed> {
        faults::waited();
        while !self.window.busy() {
            if !self.fill()? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn timed(&mut self, world: &mut World<'_>, frames: u32) -> Result<Option<Duration>, Failed> {
        self.begin();
        loop {
            if !self.hold()? || !self.fill()? {
                return Ok(None);
            }
            faults::held();
            world.step(frames)?;
            faults::timed();
            let span = world.on_device_ticks();
            if !self.window.push(span) || !self.spent(span) {
                return Ok(None);
            }
            if self.window.busy() {
                return Ok(world.on_device().map(|frame| frame * frames));
            }
        }
    }
}

#[derive(Debug)]
struct Lost;

impl std::fmt::Display for Lost {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.write_str("the device could not be kept busy")
    }
}

impl std::error::Error for Lost {}

fn point(warm: &mut Warm<'_, '_>, world: &mut World<'_>, frames: u32) -> Result<Sample, Failed> {
    world.step(1)?;
    let mut units = 0_u64;
    let mut least: Option<Duration> = None;
    for _ in 0..RUNS {
        let cells = frame_cells(world)?;
        let Some(time) = warm.timed(world, frames)? else {
            return Err(Box::new(Lost));
        };
        units = units.max(cells.saturating_mul(u64::from(frames)));
        least = Some(least.map_or(time, |held| held.min(time)));
    }
    Ok(Sample {
        units,
        submits: 1,
        time: least.unwrap_or_default(),
    })
}

struct Worlds<'host> {
    key: Key,
    small: World<'host>,
    large: Option<World<'host>>,
}

fn prepared(host: &Host) -> Result<Vec<Worlds<'_>>, Failed> {
    let mut prepared = Vec::new();
    for rule in crate::serve3::rules_on(Some(host)) {
        let params = vec![0_u32; rule.params()];
        let name = rule.name();
        if let (Ok(small), Ok(large)) = (
            World::create(host, &made(name, &params, SMALL, false)),
            World::create(host, &made(name, &params, LARGE, false)),
        ) {
            prepared.push(Worlds {
                key: Key::rule(name, false),
                small,
                large: Some(large),
            });
        }
        if let Ok(world) = World::create(host, &made(name, &params, SPARSE, true)) {
            prepared.push(Worlds {
                key: Key::rule(name, true),
                small: world,
                large: None,
            });
        }
    }
    for one in &mut prepared {
        one.small.step(1)?;
        if let Some(large) = one.large.as_mut() {
            large.step(1)?;
        }
    }
    Ok(prepared)
}

fn frame_key(warm: &mut Warm<'_, '_>, worlds: &mut Worlds<'_>) -> Result<Option<Solved>, Failed> {
    let small_world = &mut worlds.small;
    let (small, short, large) = match worlds.large.as_mut() {
        Some(large_world) => {
            let small = point(warm, small_world, FRAMES)?;
            let short = point(warm, small_world, 1)?;
            let large = point(warm, large_world, FRAMES)?;
            (small, short, large)
        }
        None => {
            let room = small_world.pool().map_or(0, |pool| pool.capacity());
            let few = spread(
                small_world,
                usize::try_from(SPARSE_BRICKS.min(room)).unwrap_or(0),
            );
            small_world.write(&few)?;
            let small = point(warm, small_world, FRAMES)?;
            let short = point(warm, small_world, 1)?;
            let many = spread(small_world, usize::try_from(room).unwrap_or(0));
            small_world.write(&many)?;
            let large = point(warm, small_world, FRAMES)?;
            (small, short, large)
        }
    };
    Ok(Rate::floor(small, large, short).map(|rate| (rate, vec![small, large, short])))
}

pub fn calibrated(host: &Host) -> Result<Table, Failed> {
    let mut table = Table::new(device(host));
    for (key, (rate, _)) in measured(host)? {
        table.set(key, rate);
    }
    Ok(table)
}

pub fn measured(host: &Host) -> Result<BTreeMap<Key, Solved>, Failed> {
    let mut found = BTreeMap::new();
    if !times(host) {
        return Ok(found);
    }
    let _sole = host.sole()?;
    let mut filler = World::create(host, &made("sand", &[], LARGE, false))?;
    filler.write(&seeded(LARGE))?;
    let mut warm = Warm {
        window: Window::of(host),
        filler: &mut filler,
        since: None,
        steps: 0,
    };
    warm.begin();
    if !warm.hold()? {
        return Ok(found);
    }
    let mut worlds = prepared(host)?;
    for one in &mut worlds {
        match frame_key(&mut warm, one) {
            Ok(Some(solved)) => {
                found.insert(one.key.clone(), solved);
            }
            Ok(None) => {}
            Err(why) if why.downcast_ref::<Lost>().is_some() => return Ok(BTreeMap::new()),
            Err(why) => return Err(why),
        }
    }
    Ok(found)
}
