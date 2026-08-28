//! When the coarse arrangement first parts, and in which word.
//!
//! `tests/hunt.rs` says the arrangement does not reach the same place twice at 1024 blocks on a
//! sixty-four-lane part, that the world is identical every time, and that what wanders is `AWAKE`,
//! `HOT` and the lists derived from them. It says nothing about **when** — it reads once at the end
//! of three hundred frames, because reading between frames changes the answer.
//!
//! Reading once at the end of a run of *`n`* frames does not. So this runs the same arrangement
//! several times at each of a rising number of frames and asks which `n` is the first where the runs
//! disagree, and then which words disagree at that `n`. Nothing here reads between frames either;
//! the shortening is the instrument.
//!
//! It asserts nothing. `decisions/DR-0013` is what it reports into.

// A test may panic — that is how it reports.
#![allow(clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use grit::compact::Plan;
use grit::layout::{AWAKE, BRUSH, GRAND, HOT, HOT_DENSE, HOT_GRAND, PARAMS, STATE};
use grit::sand::{AIR, Grid, SAND, WALL};
use grit::{Groups, Host, sweep};

/// When the brush stops. Past every frame count this walks, so the brush is always pouring.
const POURING: u32 = 200;

/// How many blocks. The size at which `tests/coarse.rs` disagrees.
const BLOCKS: u32 = 1024;

/// How many times each frame count is run.
const RUNS: usize = 8;

/// The lowest frame count the scan walks from.
const FROM: u32 = 1;

/// The highest frame count the scan walks to.
const LIMIT: u32 = 150;

/// How many frames the flag-against-counters classification walks.
const EARLY: u32 = 6;

/// How many frames the batch-size comparison walks, and how many runs it makes at each size.
const BATCH_FRAMES: u32 = 150;

/// What the isolated decay probe starts every counter and every flag at.
///
/// **Not two.** One decrement leaves one below it, a second leaves two below, and a read that saw
/// the nought the buffer held a moment before leaves nought — and at two the last two are the same
/// number. Anything above three tells them apart.
const START: u32 = 5;

/// How many rounds the isolated decay probe makes, half of each list.
const ROUNDS: usize = 400;

/// How many runs the batch-size comparison makes at each size.
const BATCH_RUNS: usize = 400;

/// How many runs it makes at each of those.
const EARLY_RUNS: usize = 128;

/// Open a device, or say why not and hand back nothing.
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

/// A room with walls, and air inside it.
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

/// A grid with this many blocks at this width, both sides a power of two.
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

/// What one run leaves behind, read once and only at the end.
struct Shot {
    /// How long the fine work list is.
    grand: u32,
    /// How long the coarse block list is.
    hot_grand: u32,
    /// One word a quad.
    awake: Vec<u32>,
    /// One word a block, padded, and the dump past it.
    hot: Vec<u32>,
    /// The coarse block list, as long as `hot_grand` says.
    dense: Vec<u32>,
    /// The world, folded.
    world: u64,
}

/// How one run is arranged: which grouping, how many frames a submission, how many frames, and
/// whether the coarse hierarchy is built at all.
#[derive(Clone, Copy)]
struct Running {
    /// One pass a stage, rather than the stages [`grit::live::arranged`] chose.
    apart: bool,
    /// How many frames go into one submission.
    batch: u32,
    /// How many frames the run walks.
    frames: u32,
    /// Whether the coarse list is built and the three passes dispatched over it.
    coarse: bool,
}

/// One whole run, and what it left behind.
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

/// A buffer as one number, so that whole buffers can be compared without holding them all.
fn folded(words: &[u32]) -> u64 {
    let mut held = 0xcbf2_9ce4_8422_2325_u64;
    for one in words {
        held ^= u64::from(*one);
        held = held.wrapping_mul(0x100_0000_01b3);
    }
    held
}

/// Where two buffers differ: how many words, and the first few as `(index, left, right)`.
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

/// Which blocks a run disagrees about, at each of a rising number of frames.
///
/// A run is read once at the end, so the frame count is the only clock this has. Every block whose
/// wake counters differ between runs is named, and beside it what the coarse level said about that
/// same block: its flag, whether the list held it, and the three counter values across its quads.
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
        // **The flag against the counters it stands for.** `live::wake` writes the same two to a
        // quad's counter and to its block's flag in the same invocation, and `decay` takes one off
        // both in the same invocation too — so a flag is at least the highest counter in its block,
        // always. A block where it is lower is one of the two decrements landing without the other,
        // and it needs no second run to see.
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

        // **The list against the flags it was compacted from.** `hot_grand` should be exactly how
        // many blocks hold a flag above nought, and the list should be exactly those blocks, once
        // each. This is the one check that does not need a second run to compare against.
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

/// What a block's wake counters and its flag say about each other, over the first few frames.
///
/// A block nothing has touched since [`grit::live::seed`] is in a state this can be written down:
/// the seed puts two in every quad's counter and two in the block's flag, and `decay` takes one off
/// both **in the same invocation**, so after one frame it is one and one, after two it is nought and
/// nought, and the block leaves the list. The flag can be *higher* than a quad's counter — anything
/// waking any quad in the block raises it — and it can never be lower.
///
/// So this classifies every block by what its five hundred and twelve counters hold and what its
/// flag holds, and counts the classes. A class where the flag is below every counter in the block is
/// a decrement one of the two took and the other did not.
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

/// The shortest run that does not reach the same place twice.
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

/// What one `decay` dispatch does when the list names the same block twice.
///
/// # Why this is worth asking in isolation
///
/// The counters and the flag are read-modify-written in the same invocation, so nothing that
/// dispatches the pass **once** a block can part them. Twice a block can: the two workgroups race,
/// and the two halves of the pass do not race the same way. A quad's counter is read and written by
/// one lane in each workgroup, so both nearly always read the old value and one decrement is lost.
/// The block's flag is read and written by *every* lane of both — a hundred and twenty-eight
/// read-modify-writes on one word — so one of them nearly always reads what another has already
/// written, and the decrement is not lost.
///
/// Which is exactly the state `which_block_goes_out_of_step` finds: counters one above the flag,
/// never below. This runs the pass alone, over a list that names every block twice, and counts how
/// the two come out. It asserts nothing.
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

    // Half the blocks, each named twice, so the dispatch is as wide as the seeded one -- and the
    // same dispatch over a list that names each block once, as the control.
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
        // **Five, not two.** One decrement leaves four; a second leaves three; a read that saw the
        // nought this buffer held a moment ago leaves nought. Two is the value the loop uses and it
        // cannot tell those apart -- both the second decrement and the stale read leave nought.
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

/// Whether the frame boundary is where the flag and its counters part.
///
/// [`grit::Resident::staged_frames`] puts a barrier between the last stage of one frame and the
/// first of the next — but only **inside** one call. The call's last stage has nothing after it, so
/// a run of `n` frames taken a frame a submission has `n` boundaries with no barrier on them and one
/// with a barrier inside every batch; taken all in one submission it has one boundary and `n − 1`
/// barriers. If that boundary is where `decay` reaches over the previous frame's `settle`, the count
/// of blocks whose flag is below their counters moves with the batch size. It asserts nothing.
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

/// The same invariant under the arrangement that has no coarse list at all.
///
/// `decay`, `mark` and `gather` dispatched over every block rather than over a list is the other
/// half of [`grit::live::arranged`], and `tests/live.rs` runs it. It cannot go out of step the way
/// the coarse one does — a block is visited whether its flag says anything or not — so if the flag
/// still falls below the counters it stands for here, the list is not where the two part. It asserts
/// nothing.
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
            // **The one line in this file that is a check and not a reading.** Before
            // `decisions/DR-0018` this was 42 of 128 runs as arranged without a list, 33 with one
            // pass a stage and 22 with a list — so a flag that steps down beside its counters again,
            // rather than being computed from them, fails here, and does not have to be caught by
            // two runs disagreeing about something further downstream.
            assert_eq!(
                runs_hit, 0,
                "a flag below the counters it stands for, in {runs_hit} of {EARLY_RUNS} runs: {seen:?}"
            );
        }
    }
}

/// How far across a workgroup a reduction in `decay` actually reaches.
///
/// The pass is built as [`grit::Shape`]`::new(subgroup, subgroup, ..)` — one workgroup of exactly a
/// subgroup's invocations — and everything in it that folds across lanes assumes those two numbers
/// are the same partition. If a workgroup is more than one subgroup on this part, a reduction sees
/// only its own share and the flag it writes is whichever share stored last.
///
/// So this gives every block counters that rise across its lanes: quad `q` holds `q mod per_block`,
/// so the highest counter in the block belongs to the **last** lane. One `decay` steps them all and
/// writes the block's flag. A flag of `per_block − 2` is a reduction that reached the last lane;
/// anything lower names the lane it stopped at. It asserts nothing.
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

/// What the flag's reduction costs, against the read-modify-write it replaced.
///
/// One `decay` and nothing else, dispatched over a list naming every block once, timed on the
/// device rather than on the host — the same clock [`grit::Resident::on_device`] hands the frame
/// loop. Ten submissions of a hundred repetitions each, and the whole spread is printed rather than
/// an average, because a number without its spread is a claim about one run. It asserts nothing.
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
        // **Not divided.** `Resident::on_device` is already per frame — `timed` divides the two
        // stamps by how many frames the submission held — and dividing again is how the first
        // version of this file reported a number a hundred times too small.
        let each = held
            .on_device()
            .map_or(0.0, |each| each.as_secs_f64() * 1e9);
        // The first submission carries the pipeline's first use with it.
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
