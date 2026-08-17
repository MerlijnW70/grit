//! The whole compaction, four passes in one submission, held to the host.
//!
//! # What this checks that `tests/sweep.rs` cannot
//!
//! That file holds one block to its own bucket. This holds every block against every *other* one —
//! which is where a compaction actually goes wrong. The tiers are added in three places and a base
//! that is off by one block is a work list that looks entirely reasonable: the right length, full of
//! plausible indices, with one block's worth written over another's.
//!
//! So the checks are:
//!
//! * the grand total, which `scan_totals` reduces and nothing else may write;
//! * the list itself, word for word, against a host that concatenates the buckets in block order —
//!   **order included**, because a check that sorted first would pass a scan whose tiers were added
//!   in the wrong arrangement;
//! * and that every awake cell appears exactly once and no sleeping one appears at all, which holds
//!   whatever the arithmetic turned out to be.

// A test may panic — that is how it reports.
#![allow(clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use grit::Host;
use grit::compact::{self, Plan};
use grit::sweep;

/// Blocks in the check. Enough that there is more than one tile at every width here.
const BLOCKS: u32 = 1024;

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

#[test]
fn the_work_list_is_every_awake_cell_once_and_in_block_order() {
    let Some(host) = device("compact") else {
        return;
    };
    let width = host.width();
    let plan = match Plan::over(width, BLOCKS) {
        Ok(plan) => plan,
        Err(too_big) => {
            eprintln!("SKIPPED compact at width {width}: {too_big}");
            return;
        }
    };
    println!(
        "{} — subgroup {width}, {} cells in {} blocks, {} tiles",
        host.name(),
        plan.quads,
        plan.blocks,
        plan.tiles
    );

    let passes = [
        sweep::sweep(width).expect("sweep"),
        compact::scan(width, sweep::COUNTS, compact::BASES, compact::TOTALS).expect("counts"),
        compact::scan(width, compact::TOTALS, compact::TOTALS, compact::GRAND).expect("totals"),
        compact::gather(width).expect("gather"),
        compact::prepare(width, sweep::cells(width)).expect("prepare"),
    ];
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let words: usize = passes.iter().map(Vec::len).sum();
    println!("  {words} words of SPIR-V across {} passes", passes.len());

    let sizes = plan.sizes();
    let mut resident = host.resident(&sizes, &modules).expect("allocated");

    let frame = 11_u32;
    let state: Vec<u32> = (0..plan.quads)
        .map(|at| (at as u32).wrapping_mul(0x9E37_79B1))
        .collect();
    let buckets = vec![u32::MAX; sizes[sweep::BUCKETS as usize]];

    resident
        .write(sweep::PARAMS as usize, &[frame, plan.dump()])
        .expect("params");
    resident
        .write(sweep::STATE as usize, &state)
        .expect("state");
    resident
        .write(sweep::BUCKETS as usize, &buckets)
        .expect("buckets");
    for binding in [
        sweep::COUNTS as usize,
        compact::BASES as usize,
        compact::TOTALS as usize,
        compact::GRAND as usize,
        compact::INDIRECT as usize,
    ] {
        resident
            .write(binding, &vec![0_u32; sizes[binding]])
            .expect("cleared");
    }
    resident
        .write(
            compact::DENSE as usize,
            &vec![u32::MAX; sizes[compact::DENSE as usize]],
        )
        .expect("dense");

    // Six passes, one submission, one fence — and the last one's size came from the device.
    let spent = resident.frame(&plan.groups()).expect("dispatched");

    let ran_state = resident
        .read(sweep::STATE as usize, plan.quads)
        .expect("state");
    let ran_counts = resident
        .read(sweep::COUNTS as usize, plan.blocks as usize)
        .expect("counts");
    let ran_buckets = resident
        .read(sweep::BUCKETS as usize, sizes[sweep::BUCKETS as usize])
        .expect("buckets");
    let grand = resident
        .read(compact::GRAND as usize, 1)
        .expect("grand")
        .first()
        .copied()
        .unwrap_or(0);
    let dense = resident
        .read(compact::DENSE as usize, plan.quads)
        .expect("dense");

    // What the host says, from the counts and buckets the device produced — so this is a check of
    // the *scan*, with the sweep's own answer taken as given. `tests/sweep.rs` checks that half.
    let (want_dense, want_grand) = compact::dense_reference(&plan, &ran_counts, &ran_buckets);

    assert_eq!(
        grand, want_grand,
        "the grand total disagrees with the counts it was reduced from"
    );
    assert_eq!(
        grand as usize,
        want_dense.len(),
        "the total and the list are different lengths"
    );

    let listed = &dense[..grand as usize];
    if let Some(at) = listed
        .iter()
        .zip(&want_dense)
        .position(|(there, here)| there != here)
    {
        let wide = sweep::stride(width) as usize;
        panic!(
            "slot {at} of the work list holds cell {} and the host says {} — the tiers are added \
             differently somewhere (block width {wide})",
            listed[at], want_dense[at]
        );
    }

    // The properties, which hold whatever the arithmetic is.
    let awake: Vec<u32> = (0..plan.quads)
        .filter(|&at| ran_state.get(at).copied().is_some_and(sweep::is_awake))
        .map(|at| u32::try_from(at).unwrap_or(u32::MAX))
        .collect();

    assert_eq!(
        listed.len(),
        awake.len(),
        "{} cells are listed and {} are awake",
        listed.len(),
        awake.len()
    );

    let mut sorted = listed.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        listed.len(),
        "a cell is listed twice, which is two blocks writing at the same base"
    );
    assert_eq!(
        sorted, awake,
        "the list and the awake cells are different sets"
    );

    // And nothing was written past the end of it.
    assert!(
        dense[grand as usize..].iter().all(|&slot| slot == u32::MAX),
        "something was written past the end of the work list"
    );

    // And the pass that ran over the list rather than the grid reached exactly those cells.
    let indirect = resident
        .read(compact::INDIRECT as usize, 3)
        .expect("indirect");
    let due = grand.div_ceil(plan.per_block);
    assert_eq!(
        indirect,
        vec![due, 1, 1],
        "the workgroup count the device wrote is not the one the list needs"
    );

    let saving = plan.quads as f64 / f64::from(grand.max(1));
    println!(
        "  a pass over the list would run {due} workgroups, not {} — {saving:.0}× fewer",
        plan.blocks
    );

    let density = f64::from(grand) * 100.0 / plan.quads as f64;
    println!(
        "  {grand} of {} cells awake — {density:.1}% — five passes in {spent:.3?}",
        plan.quads
    );
    assert!(
        grand > 0 && (grand as usize) < plan.quads,
        "every cell is awake or none is, so the predicate says nothing"
    );
}
