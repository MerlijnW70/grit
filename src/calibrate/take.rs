use std::collections::BTreeMap;
use std::time::Duration;

use crate::branch::{Failed, Made, World};
use crate::host::Host;
use crate::serve3::budget::world_cost;

use super::{Key, Meter, Rate, Sample, Table, device, faults, meter};

pub const RUNS: usize = 5;

pub const SMALL: [u32; 3] = [32, 32, 32];

pub const LARGE: [u32; 3] = [64, 64, 64];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plan {
    pub small: [u32; 3],
    pub large: [u32; 3],
    pub sparse: [u32; 3],
    pub frames: u32,
}

pub const PLAN: Plan = Plan {
    small: SMALL,
    large: LARGE,
    sparse: LARGE,
    frames: 8,
};

pub const COPY_LARGE: [u32; 3] = [96, 96, 96];

pub const WRITE_CHUNK: usize = 32;

pub const WRITE_CHUNKS: usize = 16;

const SPARSE_CELLS: [usize; 2] = [64, 1024];

const SPARSE_BRICKS: u32 = 2;

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

fn timed<T>(
    driver: Meter<'_>,
    run: impl FnOnce() -> Result<T, Failed>,
) -> Result<(T, Option<Duration>), Failed> {
    let from = driver.mark();
    faults::held();
    let out = run()?;
    let to = driver.closing();
    Ok((out, from.zip(to).map(|(from, to)| driver.between(from, to))))
}

fn least(held: &mut Option<Sample>, units: u64, submits: u32, time: Option<Duration>) {
    let Some(time) = time else {
        return;
    };
    let sample = Sample {
        units,
        submits,
        time,
    };
    if sample.valid() && held.is_none_or(|best| sample.time < best.time) {
        *held = Some(sample);
    }
}

fn cells_of(size: [u32; 3]) -> u64 {
    size.iter()
        .fold(1_u64, |all, side| all.saturating_mul(u64::from(*side)))
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

fn ran(world: &mut World<'_>, meter: Meter<'_>, size: [u32; 3]) -> Result<Option<Sample>, Failed> {
    let frames = PLAN.frames;
    world.step(1)?;
    let mut best = None;
    for _ in 0..RUNS {
        let cells = match world.pool() {
            Some(pool) => {
                let held = world.pool_use()?.map_or(0, |used| used.held);
                u64::from(held).saturating_mul(u64::try_from(pool.cells()).unwrap_or(u64::MAX))
            }
            None => cells_of(size),
        };
        let ((), time) = timed(meter, || world.step(frames))?;
        least(&mut best, cells.saturating_mul(u64::from(frames)), 1, time);
    }
    Ok(best)
}

fn frames(
    host: &Host,
    meter: Meter<'_>,
    (rule, params): (&str, &[u32]),
    sparse: bool,
) -> Result<Option<Solved>, Failed> {
    let mut points = Vec::new();
    let Plan {
        small,
        large,
        sparse: spread_over,
        ..
    } = PLAN;
    if sparse {
        let Ok(mut world) = World::create(host, &made(rule, params, spread_over, true)) else {
            return Ok(None);
        };
        let room = world.pool().map_or(0, |pool| pool.capacity());
        for bricks in [SPARSE_BRICKS.min(room), room] {
            let cells = spread(&world, usize::try_from(bricks).unwrap_or(0));
            world.write(&cells)?;
            points.push(ran(&mut world, meter, spread_over)?);
        }
    } else {
        for size in [small, large] {
            let Ok(mut world) = World::create(host, &made(rule, params, size, false)) else {
                return Ok(None);
            };
            points.push(ran(&mut world, meter, size)?);
        }
    }
    Ok(pair(&points))
}

fn built(
    host: &Host,
    meter: Meter<'_>,
    (size, repeats): ([u32; 3], u32),
    sparse: bool,
) -> Result<Option<Sample>, Failed> {
    let mut best = None;
    for _ in 0..RUNS {
        let (worlds, time) = timed(meter, || {
            (0..repeats)
                .map(|_| World::create(host, &made("sand", &[], size, sparse)))
                .collect::<Result<Vec<_>, _>>()
        })?;
        let units = worlds.first().map_or(0, |world| world_cost(world.sizes()));
        least(
            &mut best,
            units.saturating_mul(u64::from(repeats)),
            repeats,
            time,
        );
        drop(worlds);
    }
    Ok(best)
}

fn written(host: &Host, meter: Meter<'_>, sparse: bool) -> Result<Option<Solved>, Failed> {
    let mut world = World::create(host, &made("sand", &[], LARGE, sparse))?;
    let mut points = Vec::new();
    let plans: [(usize, usize, usize); 2] = if sparse {
        [(SPARSE_CELLS[0], 1, 8), (SPARSE_CELLS[1], 1, 1)]
    } else {
        [(1, 1, 16), (WRITE_CHUNK * WRITE_CHUNKS, WRITE_CHUNKS, 1)]
    };
    for (count, calls, repeats) in plans {
        let side = LARGE[0];
        let cells: Vec<([u32; 3], u32)> = (0..count)
            .map(|at| {
                let at = u32::try_from(at).unwrap_or(0);
                ([at % 16, (at / 16) % 16, at / 256 % side], 1)
            })
            .collect();
        let chunk = cells.len().div_ceil(calls).max(1);
        let mut best = None;
        for _ in 0..RUNS {
            let ((), time) = timed(meter, || {
                for _ in 0..repeats {
                    for piece in cells.chunks(chunk) {
                        world.write(piece)?;
                    }
                }
                Ok(())
            })?;
            least(
                &mut best,
                u64::try_from(count.saturating_mul(repeats)).unwrap_or(u64::MAX),
                u32::try_from(calls.saturating_mul(repeats)).unwrap_or(u32::MAX),
                time,
            );
        }
        points.push(best);
    }
    Ok(pair(&points))
}

#[derive(Clone, Copy)]
enum Copying {
    Fork,
    Restore,
}

fn copied(
    host: &Host,
    meter: Meter<'_>,
    (size, repeats): ([u32; 3], u32),
    how: Copying,
) -> Result<Option<Sample>, Failed> {
    let mut source = World::create(host, &made("sand", &[], size, false))?;
    let mut target = match how {
        Copying::Restore => Some(World::create(host, &made("sand", &[], size, false))?),
        Copying::Fork => None,
    };
    let units = world_cost(source.sizes());
    let mut best = None;
    for _ in 0..RUNS {
        let time = match (how, target.as_mut()) {
            (Copying::Fork, _) => {
                let (forks, time) = timed(meter, || {
                    (0..repeats)
                        .map(|_| source.fork())
                        .collect::<Result<Vec<_>, _>>()
                })?;
                drop(forks);
                time
            }
            (Copying::Restore, Some(target)) => {
                timed(meter, || {
                    for _ in 0..repeats {
                        target.restore(&mut source)?;
                    }
                    Ok(())
                })?
                .1
            }
            (Copying::Restore, None) => None,
        };
        least(
            &mut best,
            units.saturating_mul(u64::from(repeats)),
            repeats,
            time,
        );
    }
    Ok(best)
}

pub type Solved = (Rate, [Sample; 2]);

fn pair(points: &[Option<Sample>]) -> Option<Solved> {
    match points {
        [Some(small), Some(large)] => {
            Rate::solved(*small, *large).map(|rate| (rate, [*small, *large]))
        }
        _ => None,
    }
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
    let Some(meter) = meter(host) else {
        return Ok(found);
    };
    let _sole = host.sole()?;
    for rule in crate::serve3::rules_on(Some(host)) {
        let params = vec![0_u32; rule.params()];
        for sparse in [false, true] {
            if let Some(solved) = frames(host, meter, (rule.name(), &params), sparse)? {
                found.insert(Key::rule(rule.name(), sparse), solved);
            }
        }
    }
    for sparse in [false, true] {
        let built = pair(&[
            built(host, meter, (SMALL, 2), sparse)?,
            built(host, meter, (COPY_LARGE, 1), sparse)?,
        ]);
        if let Some(solved) = built {
            found.insert(Key::Build { sparse }, solved);
        }
        if let Some(solved) = written(host, meter, sparse)? {
            found.insert(Key::Write { sparse }, solved);
        }
    }
    for (key, how, repeats) in [
        (Key::Fork, Copying::Fork, [4, 1]),
        (Key::Restore, Copying::Restore, [4, 1]),
    ] {
        let copies = pair(&[
            copied(host, meter, (SMALL, repeats[0]), how)?,
            copied(host, meter, (COPY_LARGE, repeats[1]), how)?,
        ]);
        if let Some(solved) = copies {
            found.insert(key, solved);
        }
    }
    Ok(found)
}
