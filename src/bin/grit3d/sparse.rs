use std::time::{Duration, Instant};

use grit::bricks::{Bricks, Packed};
use grit::compact::Plan;
use grit::layout::{AWAKE, BUCKETS, DENSE, PARAMS, STATE};
use grit::rule3::Rule;
use grit::sand::Grid;
use grit::{Groups, Host, Resident, sweep};

use super::open::Land;
use super::session::Failed;

pub use super::session::OPEN;

pub enum World {
    Dense(Vec<u32>),
    Open(Land),
}

fn packed_for(bricks: &Bricks, world: &World) -> Result<Packed, Failed> {
    Ok(match world {
        World::Dense(cells) => bricks.pack(cells)?,
        World::Open(land) => bricks.compose(|brick| land.content(bricks, brick))?,
    })
}

fn milliseconds(times: &mut [Duration]) -> (f64, f64, f64) {
    times.sort_unstable();
    let at = |index: usize| times.get(index).copied().unwrap_or_default().as_secs_f64() * 1000.0;
    (
        at(0),
        at(times.len() / 2),
        at((times.len() * 99).div_ceil(100).saturating_sub(1)),
    )
}

pub struct Asked {
    pub slots: Option<u32>,
    pub skip: u32,
    pub warm: usize,
    pub timed: usize,
    pub params: Vec<u32>,
    pub name: &'static str,
}

pub fn engined<R: Rule>(
    host: &Host,
    rule: &R,
    grid: Grid,
    world: World,
    asked: &Asked,
) -> Result<String, Failed> {
    let width = host.width();
    let plan = Plan::bricked(width, grid.blocks().div_ceil(sweep::cells(width)))?;
    let clock = Instant::now();
    let most =
        u32::try_from(((1_u64 << 32) - 1) / (u64::from(plan.per_block) * 8)).unwrap_or(u32::MAX);
    let probe = Bricks::new(grid, &plan, most.min(plan.blocks))?;
    let packed = packed_for(&probe, &world)?;
    drop(world);
    let made = clock.elapsed();
    let held_at_start = packed.held;
    let capacity = asked
        .slots
        .unwrap_or_else(|| held_at_start + (held_at_start / 4).max(1024))
        .clamp(1, most.min(plan.blocks));
    let bricks = Bricks::new(grid, &plan, capacity)?;
    let packed = bricks.restacked(packed)?;

    let (passes, stages) = grit::bricks::arranged(rule, &bricks, &plan, true)?;
    let sizes = bricks.sizes(rule, &plan);
    host.limits().admits_on(&sizes, None, host.heap())?;
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let mut held = host.resident(&sizes, &modules)?;
    zeroed(&mut held, &sizes)?;
    if !asked.params.is_empty() {
        held.write_at(PARAMS as usize, grit::rule3::FIRST_PARAM, &asked.params)?;
    }
    held.write(PARAMS as usize, &[u32::MAX])?;
    bricks.write_packed(&mut held, &plan, &packed)?;
    drop(packed);
    bricks.seed(&mut held, &plan)?;

    let groups: Vec<&[Groups]> = stages.iter().map(Vec::as_slice).collect();
    for _ in 0..asked.skip {
        held.staged(&groups)?;
    }
    let mut times = Vec::with_capacity(asked.timed);
    let mut listed = Vec::with_capacity(asked.timed);
    for at in 0..asked.warm + asked.timed {
        held.staged(&groups)?;
        if at < asked.warm {
            continue;
        }
        times.push(held.on_device().ok_or("the device keeps no timestamps")?);
        let (octs, whole) = bricks.listed(&mut held)?;
        listed.push(u64::from(octs) + u64::from(whole) * u64::from(bricks.per_block()));
    }
    let used = bricks.check(&mut held)?;
    let (least, middle, tail) = milliseconds(&mut times);
    listed.sort_unstable();
    let mib = |words: usize| (words * 4) >> 20;
    let pool = sizes.get(STATE as usize).copied().unwrap_or(0);
    let windows = bricks.windows().count() as usize;
    let pool_windows: usize = (1..windows)
        .map(|window| {
            sizes
                .get(grit::layout::BINDINGS as usize + window - 1)
                .copied()
                .unwrap_or(0)
        })
        .sum();
    let per_slot: usize = [AWAKE, BUCKETS, DENSE]
        .iter()
        .map(|&binding| sizes.get(binding as usize).copied().unwrap_or(0))
        .sum();
    let total: usize = sizes.iter().sum();
    let per_brick = total - pool - pool_windows - per_slot;
    Ok(format!(
        "{} | {} sparse engine alone {}x{}x{} | made in {:.1} s | {} MiB on the device over {} bindings: pool {} MiB ({} slots of {} cells, {} held at the start, {} at the end), slot bookkeeping {} MiB, brick bookkeeping {} MiB for {} bricks | steps {}..{} | listed octs median {} | frame {least:.3}/{middle:.3}/{tail:.3} ms",
        host.name(),
        asked.name,
        grid.wide,
        grid.deep,
        grid.tall,
        made.as_secs_f64(),
        mib(total),
        sizes.len(),
        mib(pool + pool_windows),
        capacity,
        bricks.cells(),
        held_at_start,
        used.held,
        mib(per_slot),
        mib(per_brick),
        bricks.count(),
        asked.skip + asked.warm as u32,
        asked.skip + (asked.warm + asked.timed) as u32,
        listed.get(listed.len() / 2).copied().unwrap_or(0),
    ))
}

fn zeroed(held: &mut Resident<'_>, sizes: &[usize]) -> Result<(), Failed> {
    let zeros = vec![0_u32; 1 << 24];
    for (binding, &words) in sizes.iter().enumerate() {
        let mut at = 0;
        while at < words {
            let run = zeros.len().min(words - at);
            held.write_at(binding, at, zeros.get(..run).unwrap_or(&[]))?;
            at += run;
        }
    }
    Ok(())
}
