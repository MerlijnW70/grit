#![allow(clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

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
            eprintln!("SKIPPED {label}: no Vulkan device");
            None
        }
        Err(why) => {
            eprintln!("SKIPPED {label}: {why}");
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
    let (passes, wired) = grit::live::arranged(width, grid, plan, None, coarse).expect("arranged");
    let stages: Vec<Vec<Groups>> = if apart {
        wired.into_iter().flatten().map(|one| vec![one]).collect()
    } else {
        wired
    };
    let groups: Vec<&[Groups]> = stages.iter().map(Vec::as_slice).collect();
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let sizes = plan.sizes();

    let mut held = host.resident(&sizes, &modules).expect("resident");
    for (binding, words) in sizes.iter().enumerate() {
        held.write(binding, &vec![0_u32; *words]).expect("wipe");
    }
    held.write(STATE as usize, &room(grid)).expect("room");
    held.write(PARAMS as usize, &[u32::MAX, plan.dump()])
        .expect("params");
    grit::live::seed(&mut held, grid, plan).expect("seed");
    held.staged(&groups).expect("warm");

    for frame in (0..frames).step_by(batch as usize) {
        let material = if frame < POURING { SAND } else { AIR };
        let (x, y) = (grid.wide / 2, grid.deep / 8);
        held.write_at(PARAMS as usize, BRUSH, &[x, y, 49, material])
            .expect("brush");
        let left = frames - frame;
        held.staged_frames(&groups, batch.min(left))
            .expect("frames");
    }

    let grand = held
        .read(GRAND as usize, 1)
        .expect("grand")
        .first()
        .copied()
        .unwrap_or(0);
    let hot_grand = held
        .read(HOT_GRAND as usize, 1)
        .expect("hot grand")
        .first()
        .copied()
        .unwrap_or(0);
    let dense = held
        .read(HOT_DENSE as usize, (hot_grand as usize).max(1))
        .expect("hot dense");
    let hot = held
        .read(HOT as usize, plan.padded() as usize + 1)
        .expect("hot");
    let awake = held.read(AWAKE as usize, plan.quads).expect("awake");
    let world = folded(&held.read(STATE as usize, grid.cells()).expect("state"));
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
            eprintln!("SKIPPED wander at width {width}: {why}");
            return;
        }
    };
    let per = plan.per_block as usize;
    eprintln!(
        "{} — subgroup {width}, {}x{} cells in {} blocks, {RUNS} runs a frame count",
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
                    "  {frames:>3} frames run {nth}: flag below its counters in {} blocks {:?}",
                    cold.len(),
                    &cold[..cold.len().min(6)],
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
                    "  {frames:>3} frames run {nth}: the list is not the flags — {} listed, {} distinct, {} flagged",
                    listed.len(),
                    distinct.len(),
                    expected.len(),
                );
            }
        }
        if blocks.is_empty() && world_same {
            continue;
        }
        eprintln!(
            "  {frames:>3} frames  world {}  {} blocks out of step  fine {:?}  coarse {:?}",
            if world_same { "same" } else { "**differ**" },
            blocks.len(),
            shots.iter().map(|one| one.grand).collect::<Vec<_>>(),
            shots.iter().map(|one| one.hot_grand).collect::<Vec<_>>(),
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
                    "    block {block} run {nth}: HOT {} listed {listed}  AWAKE 2x{twos} 1x{ones} 0x{noughts}",
                    one.hot[*block],
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
            eprintln!("SKIPPED wander at width {width}: {why}");
            return;
        }
    };
    let per = plan.per_block as usize;
    eprintln!(
        "{} — subgroup {width}, {} blocks, {EARLY_RUNS} runs a frame count",
        host.name(),
        plan.blocks
    );

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
                    eprintln!(
                        "    {frames} frames: {} listed, {} distinct, {flagged} flagged",
                        listed.len(),
                        distinct.len(),
                    );
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
                    eprintln!(
                        "    {frames} frames: block {block} counters {low}..={high}, flag {flag}"
                    );
                }
            }
        }
        eprintln!(
            "  {frames:>3} frames  flag below its counters in {below} of {} blocks seen",
            plan.blocks as usize * EARLY_RUNS
        );
        for ((low, high, flag), count) in &classes {
            eprintln!("    counters {low}..={high}, flag {flag}: {count} blocks");
        }
    }
}

#[test]
#[ignore = "the range scan above subsumes it; kept for the first-parting frame count alone"]
fn how_few_frames_it_takes_to_part() {
    let Some(host) = device("wander") else { return };
    let width = host.width();
    let grid = sized(width, BLOCKS);
    let plan = match Plan::over(width, grid.quads().div_ceil(sweep::cells(width))) {
        Ok(plan) => plan,
        Err(why) => {
            eprintln!("SKIPPED wander at width {width}: {why}");
            return;
        }
    };
    eprintln!(
        "{} — subgroup {width}, {}x{} cells in {} blocks, {RUNS} runs a frame count",
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
                "  {frames:>3} frames  AWAKE {}  HOT {}  world {}  fine {grands:?}  coarse {hot_grands:?}",
                if awake_same { "same" } else { "**differ**" },
                if hot_same { "same" } else { "**differ**" },
                if world_same { "same" } else { "**differ**" },
            );
        }
        if !steady {
            for (nth, one) in shots.iter().enumerate().skip(1) {
                let (awake_count, awake_first) = parted(&first.awake, &one.awake);
                let (hot_count, hot_first) = parted(&first.hot, &one.hot);
                if awake_count == 0 && hot_count == 0 {
                    continue;
                }
                eprintln!(
                    "    run 0 against run {nth}: AWAKE {awake_count} words {awake_first:?}, HOT {hot_count} words {hot_first:?}"
                );
                for (at, _, _) in &awake_first {
                    let block = at / plan.per_block as usize;
                    let in_first =
                        first.dense[..first.hot_grand as usize].contains(&(block as u32));
                    let in_other = one.dense[..one.hot_grand as usize].contains(&(block as u32));
                    eprintln!(
                        "      quad {at} is block {block}: listed in run 0 {in_first}, in run {nth} {in_other}, HOT {} against {}",
                        first.hot[block], one.hot[block]
                    );
                }
            }
            return;
        }
    }
    eprintln!("  no disagreement in {LIMIT} frames");
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
            eprintln!("SKIPPED twice at width {width}: {why}");
            return;
        }
    };
    let per = plan.per_block as usize;
    let sizes = plan.sizes();
    let decay = grit::live::decay(width, Over::Listed(HOT_DENSE)).expect("decay");
    let modules: Vec<&[u32]> = vec![&decay];

    let doubled = plan.blocks / 2;
    let twice: Vec<u32> = (0..doubled).flat_map(|block| [block, block]).collect();
    let once: Vec<u32> = (0..plan.blocks).collect();

    let mut totals: std::collections::BTreeMap<&str, (usize, usize, usize)> =
        std::collections::BTreeMap::new();
    eprintln!(
        "{} — subgroup {width}, one decay over {} entries naming {doubled} blocks twice, and over {} naming {} once",
        host.name(),
        twice.len(),
        once.len(),
        plan.blocks,
    );

    for (round, list) in
        (0..ROUNDS).map(|round| (round, if round % 2 == 0 { &twice } else { &once }))
    {
        let named = if round % 2 == 0 { "twice" } else { "once " };
        let covered = if round % 2 == 0 { doubled } else { plan.blocks };
        let mut held = host.resident(&sizes, &modules).expect("resident");
        for (binding, words) in sizes.iter().enumerate() {
            held.write(binding, &vec![0_u32; *words]).expect("wipe");
        }
        held.write(PARAMS as usize, &[0, plan.dump()])
            .expect("params");
        held.write(AWAKE as usize, &vec![START; plan.quads])
            .expect("awake");
        held.write(HOT as usize, &vec![START; plan.padded() as usize])
            .expect("hot");
        held.write(HOT_DENSE as usize, list).expect("list");
        held.write(HOT_GRAND as usize, &[list.len() as u32])
            .expect("grand");
        held.write(HOT_INDIRECT as usize, &[list.len() as u32, 1, 1])
            .expect("indirect");

        let stage: Vec<Groups> = vec![Groups::From {
            binding: HOT_INDIRECT as usize,
            at: 0,
        }];
        held.staged(&[&stage]).expect("decay");

        let hot = held
            .read(HOT as usize, plan.padded() as usize)
            .expect("hot");
        let awake = held.read(AWAKE as usize, plan.quads).expect("awake");

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
                "  round {round}, named {named}: counters {} flag {} in {both_once} blocks, flag one lower in {flag_twice}, anything else {other} {elsewhere:?}",
                START - 1,
                START - 1,
            );
        }
    }
    for (named, (both_once, flag_twice, other)) in &totals {
        eprintln!(
            "  named {named} over {ROUNDS} rounds: counters {} flag {} in {both_once} blocks, flag one lower in {flag_twice}, anything else {other}",
            START - 1,
            START - 1,
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
            eprintln!("SKIPPED batch at width {width}: {why}");
            return;
        }
    };
    let per = plan.per_block as usize;
    eprintln!(
        "{} — subgroup {width}, {} blocks, {BATCH_FRAMES} frames, {BATCH_RUNS} runs a batch size",
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
        eprintln!(
            "  {batch:>3} frames a submission: {runs_hit} of {BATCH_RUNS} runs hold a block whose flag is below its counters, {blocks_hit} blocks in all"
        );
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
            eprintln!("SKIPPED listless at width {width}: {why}");
            return;
        }
    };
    let per = plan.per_block as usize;
    eprintln!(
        "{} — subgroup {width}, {} blocks, {EARLY_RUNS} runs a frame count",
        host.name(),
        plan.blocks
    );

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
                                "      block {block}: flag {}, counters up to {}, 2x{twos} 1x{ones}",
                                shot.hot[*block],
                                mine.iter().copied().max().unwrap_or(0),
                            );
                        }
                    }
                }
                for block in cold {
                    *seen.entry(block).or_default() += 1;
                }
            }
            eprintln!(
                "  {} list, {}, {frames:>2} frames: {runs_hit} of {EARLY_RUNS} runs hold one, blocks {seen:?}",
                if coarse { "with a" } else { "without" },
                if apart {
                    "one pass a stage"
                } else {
                    "as arranged    "
                },
            );
            assert_eq!(
                runs_hit, 0,
                "a flag below the counters it stands for, in {runs_hit} of {EARLY_RUNS} runs: {seen:?}"
            );
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
            eprintln!("SKIPPED reach at width {width}: {why}");
            return;
        }
    };
    let per = plan.per_block as u32;
    let sizes = plan.sizes();
    let decay = grit::live::decay(width, Over::Listed(HOT_DENSE)).expect("decay");
    let modules: Vec<&[u32]> = vec![&decay];
    let list: Vec<u32> = (0..plan.blocks).collect();
    let counters: Vec<u32> = (0..plan.quads as u32).map(|quad| quad % per).collect();

    eprintln!(
        "{} — subgroup {width}, {per} quads a block over {width} lanes, 8 a lane; the highest counter is {} and lands in lane {}",
        host.name(),
        per - 1,
        width - 1,
    );

    let mut seen: std::collections::BTreeMap<u32, usize> = std::collections::BTreeMap::new();
    for _ in 0..8 {
        let mut held = host.resident(&sizes, &modules).expect("resident");
        for (binding, words) in sizes.iter().enumerate() {
            held.write(binding, &vec![0_u32; *words]).expect("wipe");
        }
        held.write(PARAMS as usize, &[0, plan.dump()])
            .expect("params");
        held.write(AWAKE as usize, &counters).expect("awake");
        held.write(HOT as usize, &vec![0_u32; plan.padded() as usize])
            .expect("hot");
        held.write(HOT_DENSE as usize, &list).expect("list");
        held.write(HOT_GRAND as usize, &[list.len() as u32])
            .expect("grand");
        held.write(HOT_INDIRECT as usize, &[list.len() as u32, 1, 1])
            .expect("indirect");
        let stage: Vec<Groups> = vec![Groups::From {
            binding: HOT_INDIRECT as usize,
            at: 0,
        }];
        held.staged(&[&stage]).expect("decay");
        let hot = held
            .read(HOT as usize, plan.padded() as usize)
            .expect("hot");
        for flag in hot.iter().take(plan.blocks as usize) {
            *seen.entry(*flag).or_default() += 1;
        }
    }
    eprintln!("  flags written, and how many blocks got each: {seen:?}");
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
            eprintln!("SKIPPED cost at width {width}: {why}");
            return;
        }
    };
    let sizes = plan.sizes();
    let decay = grit::live::decay(width, Over::Listed(HOT_DENSE)).expect("decay");
    let modules: Vec<&[u32]> = vec![&decay];
    let list: Vec<u32> = (0..plan.blocks).collect();

    let mut held = host.resident(&sizes, &modules).expect("resident");
    for (binding, words) in sizes.iter().enumerate() {
        held.write(binding, &vec![0_u32; *words]).expect("wipe");
    }
    held.write(PARAMS as usize, &[0, plan.dump()])
        .expect("params");
    held.write(AWAKE as usize, &vec![2_u32; plan.quads])
        .expect("awake");
    held.write(HOT as usize, &vec![2_u32; plan.padded() as usize])
        .expect("hot");
    held.write(HOT_DENSE as usize, &list).expect("list");
    held.write(HOT_GRAND as usize, &[list.len() as u32])
        .expect("grand");
    held.write(HOT_INDIRECT as usize, &[list.len() as u32, 1, 1])
        .expect("indirect");
    let stage: Vec<Groups> = vec![Groups::From {
        binding: HOT_INDIRECT as usize,
        at: 0,
    }];

    let mut seen = Vec::with_capacity(10);
    for round in 0..11 {
        held.staged_frames(&[&stage], 100).expect("decay");
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
        "{} — subgroup {width}, one decay over {} blocks: {low:.0}-{high:.0} ns a dispatch over {} submissions of a hundred",
        host.name(),
        plan.blocks,
        seen.len(),
    );
}
