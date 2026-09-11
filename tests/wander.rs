#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use grit::compact::Plan;
use grit::layout::{AWAKE, BRUSH, GRAND, HOT, HOT_DENSE, HOT_GRAND, PARAMS, STATE};
use grit::sand::{AIR, Grid, SAND, WALL};
use grit::{Groups, Host, sweep};

const POURING: u32 = 200;

const BLOCKS: u32 = 1024;

const RUNS: usize = 8;

const FROM: u32 = 1;

const LIMIT: u32 = 150;

const EARLY: u32 = 6;

const BATCH_FRAMES: u32 = 150;

const START: u32 = 5;

const ROUNDS: usize = 400;

const BATCH_RUNS: usize = 400;

const EARLY_RUNS: usize = 128;

fn device(label: &str) -> Option<Host> {
    match Host::open() {
        Ok(Some(host)) => Some(host),
        Ok(None) => {
            eprintln!("SKIPPED {label}");
            None
        }
        Err(why) => {
            eprintln!("SKIPPED {label} {why}");
            None
        }
    }
}

fn room(grid: Grid) -> Vec<u32> {
    let mut cells = vec![AIR; grid.cells()];
    for row in 0..grid.deep {
        for column in 0..grid.wide {
            let edge = row + 1 >= grid.deep || column == 0 || column + 1 >= grid.wide;
            if let Some(slot) = cells.get_mut((row * grid.wide + column) as usize) {
                *slot = if edge { WALL } else { AIR };
            }
        }
    }
    cells
}

fn sized(width: u32, blocks: u32) -> Grid {
    let cells = u64::from(blocks) * u64::from(sweep::cells(width)) * 4;
    let (mut wide, mut deep) = (1_u32, 1_u32);
    while u64::from(wide) * u64::from(deep) < cells {
        if wide <= deep * 2 {
            wide *= 2;
        } else {
            deep *= 2;
        }
    }
    Grid::of(wide, deep)
}

struct Shot {
    grand: u32,
    hot_grand: u32,
    awake: Vec<u32>,
    hot: Vec<u32>,
    dense: Vec<u32>,
    world: u64,
}

#[derive(Clone, Copy)]
struct Running {
    apart: bool,
    batch: u32,
    frames: u32,
    coarse: bool,
}

fn ran(host: &Host, width: u32, grid: Grid, plan: &Plan, how: Running) -> Shot {
    let Running {
        apart,
        batch,
        frames,
        coarse,
    } = how;
    let (passes, wired) = grit::live::arranged(width, grid, plan, None, coarse).unwrap();
    let stages: Vec<Vec<Groups>> = if apart {
        wired.into_iter().flatten().map(|one| vec![one]).collect()
    } else {
        wired
    };
    let groups: Vec<&[Groups]> = stages.iter().map(Vec::as_slice).collect();
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let sizes = plan.sizes();

    let mut held = host.resident(&sizes, &modules).unwrap();
    for (binding, words) in sizes.iter().enumerate() {
        held.write(binding, &vec![0_u32; *words]).unwrap();
    }
    held.write(STATE as usize, &room(grid)).unwrap();
    held.write(PARAMS as usize, &[u32::MAX, plan.dump()])
        .unwrap();
    grit::live::seed(&mut held, grid, plan).unwrap();
    held.staged(&groups).unwrap();

    for frame in (0..frames).step_by(batch as usize) {
        let material = if frame < POURING { SAND } else { AIR };
        let (x, y) = (grid.wide / 2, grid.deep / 8);
        held.write_at(PARAMS as usize, BRUSH, &[x, y, 49, material])
            .unwrap();
        let left = frames - frame;
        held.staged_frames(&groups, batch.min(left)).unwrap();
    }

    let grand = held
        .read(GRAND as usize, 1)
        .unwrap()
        .first()
        .copied()
        .unwrap_or(0);
    let hot_grand = held
        .read(HOT_GRAND as usize, 1)
        .unwrap()
        .first()
        .copied()
        .unwrap_or(0);
    let dense = held
        .read(HOT_DENSE as usize, (hot_grand as usize).max(1))
        .unwrap();
    let hot = held.read(HOT as usize, plan.padded() as usize + 1).unwrap();
    let awake = held.read(AWAKE as usize, plan.quads).unwrap();
    let world = folded(&held.read(STATE as usize, grid.cells()).unwrap());
    Shot {
        grand,
        hot_grand,
        awake,
        hot,
        dense,
        world,
    }
}

fn folded(words: &[u32]) -> u64 {
    let mut held = 0xcbf2_9ce4_8422_2325_u64;
    for one in words {
        held ^= u64::from(*one);
        held = held.wrapping_mul(0x100_0000_01b3);
    }
    held
}

fn parted(left: &[u32], right: &[u32]) -> (usize, Vec<(usize, u32, u32)>) {
    let mut count = 0;
    let mut first = Vec::new();
    for (at, (a, b)) in left.iter().zip(right).enumerate() {
        if a != b {
            count += 1;
            if first.len() < 8 {
                first.push((at, *a, *b));
            }
        }
    }
    (count, first)
}

#[test]
fn which_block_goes_out_of_step() {
    let Some(host) = device("wander") else { return };
    let width = host.width();
    let grid = sized(width, BLOCKS);
    let plan = match Plan::over(width, grid.quads().div_ceil(sweep::cells(width))) {
        Ok(plan) => plan,
        Err(why) => {
            eprintln!("SKIPPED {width} {why}");
            return;
        }
    };
    let per = plan.per_block as usize;
    eprintln!(
        "{} {width} {} {} {} {RUNS}",
        host.name(),
        grid.wide,
        grid.deep,
        plan.blocks
    );

    for frames in FROM..=LIMIT {
        let shots: Vec<Shot> = (0..RUNS)
            .map(|_| {
                ran(
                    &host,
                    width,
                    grid,
                    &plan,
                    Running {
                        apart: false,
                        batch: 1,
                        frames,
                        coarse: true,
                    },
                )
            })
            .collect();
        let first = &shots[0];
        let world_same = shots.iter().all(|one| one.world == first.world);
        let mut blocks = Vec::new();
        for block in 0..plan.blocks as usize {
            let mine = &first.awake[block * per..(block + 1) * per];
            if shots
                .iter()
                .any(|one| one.awake[block * per..(block + 1) * per] != *mine)
            {
                blocks.push(block);
            }
        }
        for (nth, one) in shots.iter().enumerate() {
            let cold: Vec<usize> = (0..plan.blocks as usize)
                .filter(|block| {
                    let mine = &one.awake[block * per..(block + 1) * per];
                    one.hot[*block] < mine.iter().copied().max().unwrap_or(0)
                })
                .collect();
            if !cold.is_empty() {
                eprintln!(
                    "{frames:>3} {nth} {} {:?}",
                    cold.len(),
                    &cold[..cold.len().min(6)]
                );
            }
        }

        for (nth, one) in shots.iter().enumerate() {
            let expected: Vec<u32> = (0..plan.blocks)
                .filter(|&block| one.hot[block as usize] > 0)
                .collect();
            let listed = &one.dense[..one.hot_grand as usize];
            let mut sorted = listed.to_vec();
            sorted.sort_unstable();
            let mut distinct = sorted.clone();
            distinct.dedup();
            if sorted != expected {
                eprintln!(
                    "{frames:>3} {nth} {} {} {}",
                    listed.len(),
                    distinct.len(),
                    expected.len()
                );
            }
        }
        if blocks.is_empty() && world_same {
            continue;
        }
        eprintln!(
            "{frames:>3} {} {} {:?} {:?}",
            world_same,
            blocks.len(),
            shots.iter().map(|one| one.grand).collect::<Vec<_>>(),
            shots.iter().map(|one| one.hot_grand).collect::<Vec<_>>()
        );
        for block in blocks.iter().take(4) {
            for (nth, one) in shots.iter().enumerate() {
                let mine = &one.awake[block * per..(block + 1) * per];
                let (twos, ones, noughts) = (
                    mine.iter().filter(|&&v| v == 2).count(),
                    mine.iter().filter(|&&v| v == 1).count(),
                    mine.iter().filter(|&&v| v == 0).count(),
                );
                let listed = one.dense[..one.hot_grand as usize].contains(&(*block as u32));
                eprintln!(
                    "{block} {nth} {} {listed} {twos} {ones} {noughts}",
                    one.hot[*block]
                );
            }
        }
    }
}

#[test]
fn what_a_flag_and_its_counters_say_about_each_other() {
    let Some(host) = device("wander") else { return };
    let width = host.width();
    let grid = sized(width, BLOCKS);
    let plan = match Plan::over(width, grid.quads().div_ceil(sweep::cells(width))) {
        Ok(plan) => plan,
        Err(why) => {
            eprintln!("SKIPPED {width} {why}");
            return;
        }
    };
    let per = plan.per_block as usize;
    eprintln!("{} {width} {} {EARLY_RUNS}", host.name(), plan.blocks);

    for frames in 1..=EARLY {
        let mut classes: std::collections::BTreeMap<(u32, u32, u32), usize> =
            std::collections::BTreeMap::new();
        let mut below = 0_usize;
        for _ in 0..EARLY_RUNS {
            let shot = ran(
                &host,
                width,
                grid,
                &plan,
                Running {
                    apart: false,
                    batch: 1,
                    frames,
                    coarse: true,
                },
            );
            {
                let listed = &shot.dense[..shot.hot_grand as usize];
                let mut sorted = listed.to_vec();
                sorted.sort_unstable();
                let mut distinct = sorted.clone();
                distinct.dedup();
                let flagged = (0..plan.blocks)
                    .filter(|&block| shot.hot[block as usize] > 0)
                    .count();
                if distinct.len() != listed.len() || sorted.len() != flagged {
                    eprintln!("{frames} {} {} {flagged}", listed.len(), distinct.len());
                }
            }
            for block in 0..plan.blocks as usize {
                let mine = &shot.awake[block * per..(block + 1) * per];
                let low = mine.iter().copied().min().unwrap_or(0);
                let high = mine.iter().copied().max().unwrap_or(0);
                let flag = shot.hot[block];
                *classes.entry((low, high, flag)).or_default() += 1;
                if flag < high {
                    below += 1;
                    eprintln!("{frames} {block} {low} {high} {flag}");
                }
            }
        }
        eprintln!("{frames:>3} {below} {}", plan.blocks as usize * EARLY_RUNS);
        for ((low, high, flag), count) in &classes {
            eprintln!("{low} {high} {flag} {count}");
        }
    }
}

#[test]
#[ignore]
fn how_few_frames_it_takes_to_part() {
    let Some(host) = device("wander") else { return };
    let width = host.width();
    let grid = sized(width, BLOCKS);
    let plan = match Plan::over(width, grid.quads().div_ceil(sweep::cells(width))) {
        Ok(plan) => plan,
        Err(why) => {
            eprintln!("SKIPPED {width} {why}");
            return;
        }
    };
    eprintln!(
        "{} {width} {} {} {} {RUNS}",
        host.name(),
        grid.wide,
        grid.deep,
        plan.blocks
    );

    for frames in 1..=LIMIT {
        let shots: Vec<Shot> = (0..RUNS)
            .map(|_| {
                ran(
                    &host,
                    width,
                    grid,
                    &plan,
                    Running {
                        apart: false,
                        batch: 1,
                        frames,
                        coarse: true,
                    },
                )
            })
            .collect();
        let first = &shots[0];
        let awake_same = shots.iter().all(|one| one.awake == first.awake);
        let hot_same = shots.iter().all(|one| one.hot == first.hot);
        let world_same = shots.iter().all(|one| one.world == first.world);
        let grands: Vec<u32> = shots.iter().map(|one| one.grand).collect();
        let hot_grands: Vec<u32> = shots.iter().map(|one| one.hot_grand).collect();
        let steady = awake_same && hot_same && world_same;
        if !steady || frames <= 3 || frames % 20 == 0 {
            eprintln!(
                "{frames:>3} {} {} {} {grands:?} {hot_grands:?}",
                awake_same, hot_same, world_same
            );
        }
        if !steady {
            for (nth, one) in shots.iter().enumerate().skip(1) {
                let (awake_count, awake_first) = parted(&first.awake, &one.awake);
                let (hot_count, hot_first) = parted(&first.hot, &one.hot);
                if awake_count == 0 && hot_count == 0 {
                    continue;
                }
                eprintln!("{nth} {awake_count} {awake_first:?} {hot_count} {hot_first:?}");
                for (at, _, _) in &awake_first {
                    let block = at / plan.per_block as usize;
                    let in_first =
                        first.dense[..first.hot_grand as usize].contains(&(block as u32));
                    let in_other = one.dense[..one.hot_grand as usize].contains(&(block as u32));
                    eprintln!(
                        "{at} {block} {in_first} {nth} {in_other} {} {}",
                        first.hot[block], one.hot[block]
                    );
                }
            }
            return;
        }
    }
    eprintln!("{LIMIT}");
}

#[test]
fn a_block_named_twice_in_one_list() {
    use grit::compact::Over;
    use grit::layout::{HOT_INDIRECT, PARAMS};

    let Some(host) = device("twice") else { return };
    let width = host.width();
    let grid = sized(width, BLOCKS);
    let plan = match Plan::over(width, grid.quads().div_ceil(sweep::cells(width))) {
        Ok(plan) => plan,
        Err(why) => {
            eprintln!("SKIPPED {width} {why}");
            return;
        }
    };
    let per = plan.per_block as usize;
    let sizes = plan.sizes();
    let decay = grit::live::decay(width, Over::Listed(HOT_DENSE)).unwrap();
    let modules: Vec<&[u32]> = vec![&decay];

    let doubled = plan.blocks / 2;
    let twice: Vec<u32> = (0..doubled).flat_map(|block| [block, block]).collect();
    let once: Vec<u32> = (0..plan.blocks).collect();

    let mut totals: std::collections::BTreeMap<bool, (usize, usize, usize)> =
        std::collections::BTreeMap::new();
    eprintln!(
        "{} {width} {} {doubled} {} {}",
        host.name(),
        twice.len(),
        once.len(),
        plan.blocks
    );

    for (round, list) in
        (0..ROUNDS).map(|round| (round, if round % 2 == 0 { &twice } else { &once }))
    {
        let named = round % 2 == 0;
        let covered = if round % 2 == 0 { doubled } else { plan.blocks };
        let mut held = host.resident(&sizes, &modules).unwrap();
        for (binding, words) in sizes.iter().enumerate() {
            held.write(binding, &vec![0_u32; *words]).unwrap();
        }
        held.write(PARAMS as usize, &[0, plan.dump()]).unwrap();
        held.write(AWAKE as usize, &vec![START; plan.quads])
            .unwrap();
        held.write(HOT as usize, &vec![START; plan.padded() as usize])
            .unwrap();
        held.write(HOT_DENSE as usize, list).unwrap();
        held.write(HOT_GRAND as usize, &[list.len() as u32])
            .unwrap();
        held.write(HOT_INDIRECT as usize, &[list.len() as u32, 1, 1])
            .unwrap();

        let stage: Vec<Groups> = vec![Groups::From {
            binding: HOT_INDIRECT as usize,
            at: 0,
        }];
        held.staged(&[&stage]).unwrap();

        let hot = held.read(HOT as usize, plan.padded() as usize).unwrap();
        let awake = held.read(AWAKE as usize, plan.quads).unwrap();

        let mut both_once = 0;
        let mut flag_twice = 0;
        let mut other = 0;
        let mut elsewhere: std::collections::BTreeMap<(u32, u32, u32), usize> =
            std::collections::BTreeMap::new();
        for block in 0..covered as usize {
            let mine = &awake[block * per..(block + 1) * per];
            let high = mine.iter().copied().max().unwrap_or(0);
            let low = mine.iter().copied().min().unwrap_or(0);
            match (low, high, hot[block]) {
                (a, b, c) if a == START - 1 && b == START - 1 && c == START - 1 => both_once += 1,
                (a, b, c) if a == START - 1 && b == START - 1 && c + 2 == START => flag_twice += 1,
                what => {
                    other += 1;
                    *elsewhere.entry(what).or_insert(0_usize) += 1;
                }
            }
        }
        totals.entry(named).or_insert((0_usize, 0_usize, 0_usize)).0 += both_once;
        totals.entry(named).or_insert((0_usize, 0_usize, 0_usize)).1 += flag_twice;
        totals.entry(named).or_insert((0_usize, 0_usize, 0_usize)).2 += other;
        if ROUNDS <= 16 {
            eprintln!(
                "{round} {named} {} {} {both_once} {flag_twice} {other} {elsewhere:?}",
                START - 1,
                START - 1
            );
        }
    }
    for (named, (both_once, flag_twice, other)) in &totals {
        eprintln!(
            "{named} {ROUNDS} {} {} {both_once} {flag_twice} {other}",
            START - 1,
            START - 1
        );
    }
}

#[test]
fn what_the_batch_size_does_to_it() {
    let Some(host) = device("batch") else { return };
    let width = host.width();
    let grid = sized(width, BLOCKS);
    let plan = match Plan::over(width, grid.quads().div_ceil(sweep::cells(width))) {
        Ok(plan) => plan,
        Err(why) => {
            eprintln!("SKIPPED {width} {why}");
            return;
        }
    };
    let per = plan.per_block as usize;
    eprintln!(
        "{} {width} {} {BATCH_FRAMES} {BATCH_RUNS}",
        host.name(),
        plan.blocks
    );

    for batch in [1_u32, 20, BATCH_FRAMES] {
        let mut runs_hit = 0;
        let mut blocks_hit = 0;
        for _ in 0..BATCH_RUNS {
            let shot = ran(
                &host,
                width,
                grid,
                &plan,
                Running {
                    apart: false,
                    batch,
                    frames: BATCH_FRAMES,
                    coarse: true,
                },
            );
            let cold = (0..plan.blocks as usize)
                .filter(|block| {
                    let mine = &shot.awake[block * per..(block + 1) * per];
                    shot.hot[*block] < mine.iter().copied().max().unwrap_or(0)
                })
                .count();
            if cold > 0 {
                runs_hit += 1;
            }
            blocks_hit += cold;
        }
        eprintln!("{batch:>3} {runs_hit} {BATCH_RUNS} {blocks_hit}");
    }
}

#[test]
fn the_flag_against_its_counters_without_a_list() {
    let Some(host) = device("listless") else {
        return;
    };
    let width = host.width();
    let grid = sized(width, BLOCKS);
    let plan = match Plan::over(width, grid.quads().div_ceil(sweep::cells(width))) {
        Ok(plan) => plan,
        Err(why) => {
            eprintln!("SKIPPED {width} {why}");
            return;
        }
    };
    let per = plan.per_block as usize;
    eprintln!("{} {width} {} {EARLY_RUNS}", host.name(), plan.blocks);

    for (coarse, apart) in [(false, false), (false, true), (true, false), (true, true)] {
        for frames in 2..=2 {
            let mut runs_hit = 0;
            let mut seen: std::collections::BTreeMap<usize, usize> =
                std::collections::BTreeMap::new();
            for _ in 0..EARLY_RUNS {
                let shot = ran(
                    &host,
                    width,
                    grid,
                    &plan,
                    Running {
                        apart,
                        batch: 1,
                        frames,
                        coarse,
                    },
                );
                let cold: Vec<usize> = (0..plan.blocks as usize)
                    .filter(|block| {
                        let mine = &shot.awake[block * per..(block + 1) * per];
                        shot.hot[*block] < mine.iter().copied().max().unwrap_or(0)
                    })
                    .collect();
                if !cold.is_empty() {
                    runs_hit += 1;
                    if runs_hit == 1 {
                        for block in cold.iter().take(3) {
                            let mine = &shot.awake[block * per..(block + 1) * per];
                            let twos = mine.iter().filter(|&&v| v == 2).count();
                            let ones = mine.iter().filter(|&&v| v == 1).count();
                            eprintln!(
                                "{block} {} {} {twos} {ones}",
                                shot.hot[*block],
                                mine.iter().copied().max().unwrap_or(0)
                            );
                        }
                    }
                }
                for block in cold {
                    *seen.entry(block).or_default() += 1;
                }
            }
            eprintln!("{coarse} {apart} {frames:>2} {runs_hit} {EARLY_RUNS} {seen:?}");
            assert_eq!(runs_hit, 0);
        }
    }
}

#[test]
fn how_far_a_reduction_in_decay_reaches() {
    use grit::compact::Over;
    use grit::layout::{HOT_INDIRECT, PARAMS};

    let Some(host) = device("reach") else { return };
    let width = host.width();
    let grid = sized(width, BLOCKS);
    let plan = match Plan::over(width, grid.quads().div_ceil(sweep::cells(width))) {
        Ok(plan) => plan,
        Err(why) => {
            eprintln!("SKIPPED {width} {why}");
            return;
        }
    };
    let per = plan.per_block as u32;
    let sizes = plan.sizes();
    let decay = grit::live::decay(width, Over::Listed(HOT_DENSE)).unwrap();
    let modules: Vec<&[u32]> = vec![&decay];
    let list: Vec<u32> = (0..plan.blocks).collect();
    let counters: Vec<u32> = (0..plan.quads as u32).map(|quad| quad % per).collect();

    eprintln!(
        "{} {width} {per} {width} {} {}",
        host.name(),
        per - 1,
        width - 1
    );

    let mut seen: std::collections::BTreeMap<u32, usize> = std::collections::BTreeMap::new();
    for _ in 0..8 {
        let mut held = host.resident(&sizes, &modules).unwrap();
        for (binding, words) in sizes.iter().enumerate() {
            held.write(binding, &vec![0_u32; *words]).unwrap();
        }
        held.write(PARAMS as usize, &[0, plan.dump()]).unwrap();
        held.write(AWAKE as usize, &counters).unwrap();
        held.write(HOT as usize, &vec![0_u32; plan.padded() as usize])
            .unwrap();
        held.write(HOT_DENSE as usize, &list).unwrap();
        held.write(HOT_GRAND as usize, &[list.len() as u32])
            .unwrap();
        held.write(HOT_INDIRECT as usize, &[list.len() as u32, 1, 1])
            .unwrap();
        let stage: Vec<Groups> = vec![Groups::From {
            binding: HOT_INDIRECT as usize,
            at: 0,
        }];
        held.staged(&[&stage]).unwrap();
        let hot = held.read(HOT as usize, plan.padded() as usize).unwrap();
        for flag in hot.iter().take(plan.blocks as usize) {
            *seen.entry(*flag).or_default() += 1;
        }
    }
    eprintln!("{seen:?}");
}

#[test]
fn what_the_flag_costs() {
    use grit::compact::Over;
    use grit::layout::{HOT_INDIRECT, PARAMS};

    let Some(host) = device("cost") else { return };
    let width = host.width();
    let grid = sized(width, BLOCKS);
    let plan = match Plan::over(width, grid.quads().div_ceil(sweep::cells(width))) {
        Ok(plan) => plan,
        Err(why) => {
            eprintln!("SKIPPED {width} {why}");
            return;
        }
    };
    let sizes = plan.sizes();
    let decay = grit::live::decay(width, Over::Listed(HOT_DENSE)).unwrap();
    let modules: Vec<&[u32]> = vec![&decay];
    let list: Vec<u32> = (0..plan.blocks).collect();

    let mut held = host.resident(&sizes, &modules).unwrap();
    for (binding, words) in sizes.iter().enumerate() {
        held.write(binding, &vec![0_u32; *words]).unwrap();
    }
    held.write(PARAMS as usize, &[0, plan.dump()]).unwrap();
    held.write(AWAKE as usize, &vec![2_u32; plan.quads])
        .unwrap();
    held.write(HOT as usize, &vec![2_u32; plan.padded() as usize])
        .unwrap();
    held.write(HOT_DENSE as usize, &list).unwrap();
    held.write(HOT_GRAND as usize, &[list.len() as u32])
        .unwrap();
    held.write(HOT_INDIRECT as usize, &[list.len() as u32, 1, 1])
        .unwrap();
    let stage: Vec<Groups> = vec![Groups::From {
        binding: HOT_INDIRECT as usize,
        at: 0,
    }];

    let mut seen = Vec::with_capacity(10);
    for round in 0..11 {
        held.staged_frames(&[&stage], 100).unwrap();
        let each = held
            .on_device()
            .map_or(0.0, |each| each.as_secs_f64() * 1e9);
        if round > 0 {
            seen.push(each);
        }
    }
    let low = seen.iter().copied().fold(f64::MAX, f64::min);
    let high = seen.iter().copied().fold(0.0_f64, f64::max);
    eprintln!(
        "{} {width} {} {low:.0} {high:.0} {}",
        host.name(),
        plan.blocks,
        seen.len()
    );
}
