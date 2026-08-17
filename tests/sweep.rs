//! The fused pass, held to the host, bit for bit.
//!
//! # What a wrong answer would look like, and why only a comparison finds it
//!
//! A compaction that gets its offsets wrong does not crash and does not look wrong. It writes some
//! live cells over each other and leaves gaps where others should be, and the work list it produces
//! is still a list of plausible indices — shorter than it should be, in an order nobody expected.
//! The next frame then simulates it happily.
//!
//! So the checks here are the ones a wrong offset cannot survive:
//!
//! * every live cell appears **exactly once** in its block's bucket, and no dead one appears at all;
//! * the count is the number of live cells, on the nose;
//! * and the whole thing equals what the host computes, word for word — including which slot each
//!   index landed in, because the slot *is* the scan.
//!
//! The first two are properties: they hold whatever the arithmetic is. The third is the reference,
//! and it is the one that catches an offset that is consistently wrong.

// A test may panic — that is how it reports.
#![allow(clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use grit::Host;
use grit::sweep::{self, PER_LANE};

/// Blocks in the check. Enough that the block tier is exercised and the run stays quick.
const BLOCKS: u32 = 64;

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

/// A state buffer worth stepping: every cell different, so a lane mapping that pairs the wrong
/// cells is visible rather than lucky.
fn seeded(count: usize) -> Vec<u32> {
    (0..count)
        .map(|at| (at as u32).wrapping_mul(0x9E37_79B1))
        .collect()
}

#[test]
fn the_work_list_is_the_work_list_the_host_computes() {
    let Some(host) = device("sweep") else {
        return;
    };
    let width = host.width();
    let cells = sweep::cells(width) as usize * BLOCKS as usize;
    let region = sweep::stride(width) as usize * BLOCKS as usize;
    println!(
        "{} — subgroup {width}, {} cells a block, {PER_LANE} a lane",
        host.name(),
        sweep::cells(width)
    );

    let spirv = match sweep::sweep(width) {
        Ok(spirv) => spirv,
        Err(refused) => {
            eprintln!("SKIPPED sweep at width {width}: {refused}");
            return;
        }
    };

    let frame = 7_u32;
    let state = seeded(cells);
    let buckets = vec![u32::MAX; region];

    let mut resident = host
        .resident(&[1, cells, region, BLOCKS as usize], &[&spirv])
        .expect("allocated");
    resident
        .write(sweep::PARAMS as usize, &[frame])
        .expect("params");
    resident
        .write(sweep::STATE as usize, &state)
        .expect("state");
    resident
        .write(sweep::BUCKETS as usize, &buckets)
        .expect("buckets");
    resident
        .write(sweep::COUNTS as usize, &vec![0; BLOCKS as usize])
        .expect("counts");

    resident
        .frame(&[grit::Groups::Fixed(BLOCKS)])
        .expect("dispatched");

    let ran_state = resident.read(sweep::STATE as usize, cells).expect("state");
    let ran_buckets = resident
        .read(sweep::BUCKETS as usize, region)
        .expect("buckets");
    let ran_counts = resident
        .read(sweep::COUNTS as usize, BLOCKS as usize)
        .expect("counts");

    let (want_state, want_buckets, want_counts) =
        sweep::sweep_reference(width, BLOCKS, frame, &state, &buckets);

    // The state, first: if the step disagrees nothing below means anything.
    if let Some(at) = ran_state
        .iter()
        .zip(&want_state)
        .position(|(there, here)| there != here)
    {
        panic!(
            "cell {at} stepped to {:#010x} and the host says {:#010x}",
            ran_state[at], want_state[at]
        );
    }

    // The counts.
    assert_eq!(ran_counts, want_counts, "the block totals disagree");

    // And the buckets, but only as far as each block's count reaches — the dump past it is never
    // read, so nothing here may check it.
    let wide = sweep::stride(width) as usize;
    for block in 0..BLOCKS as usize {
        let live = ran_counts.get(block).copied().unwrap_or(0) as usize;
        let from = block * wide;
        let there = &ran_buckets[from..from + live];
        let here = &want_buckets[from..from + live];
        assert_eq!(there, here, "block {block}'s bucket disagrees");
    }

    // The properties, which hold whatever the arithmetic turned out to be.
    let per_block = sweep::cells(width) as usize;
    for block in 0..BLOCKS as usize {
        let live = ran_counts.get(block).copied().unwrap_or(0) as usize;
        let from = block * wide;
        let listed: Vec<u32> = ran_buckets[from..from + live].to_vec();

        let expected: Vec<u32> = (0..per_block)
            .map(|at| block * per_block + at)
            .filter(|&at| ran_state.get(at).copied().is_some_and(sweep::is_awake))
            .map(|at| u32::try_from(at).unwrap_or(u32::MAX))
            .collect();

        assert_eq!(
            listed.len(),
            expected.len(),
            "block {block} listed {} cells and {} are awake",
            listed.len(),
            expected.len()
        );

        let mut sorted = listed.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            listed.len(),
            "block {block} listed a cell twice, which is an offset collision"
        );

        let mut wanted = expected.clone();
        wanted.sort_unstable();
        assert_eq!(
            sorted, wanted,
            "block {block} listed a different set of cells than the ones that are awake"
        );
    }

    let total: u32 = ran_counts.iter().sum();
    let density = f64::from(total) * 100.0 / cells as f64;
    println!("  {total} of {cells} cells awake — {density:.1}%");
    assert!(
        total > 0 && (total as usize) < cells,
        "every cell is awake or none is, so the predicate says nothing"
    );
}
