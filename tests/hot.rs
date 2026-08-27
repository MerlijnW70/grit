//! The coarse hierarchy on its own: which blocks have anything in them, compacted.
//!
//! # What `tests/live.rs` cannot reach, and why it matters
//!
//! That test runs the whole loop and demands it agree with a whole-grid run cell for cell, which is
//! the strongest check in the repository — but it runs one world, and on a thirty-two-lane part that
//! world has **one tile**. The coarse scan's second and third tiers are then a base of nought added
//! to a base of nought, and a bug in either would go straight past it. The same world on an
//! eight-lane part has sixteen tiles, so the gap is a property of the device rather than of the
//! test, which is the worst kind: it passes here and fails on somebody else's card.
//!
//! So this holds the coarse tier to the same checks `tests/compact.rs` holds the fine one to, at a
//! shape with more than one tile **at every width this crate builds for** — and one that is not a
//! whole number of tiles, so the padding is exercised too.
//!
//! * the grand total is the number of hot blocks, on the nose;
//! * the list is those blocks, **in ascending order**, because the order is the scan — a check that
//!   sorted first would pass a hierarchy whose tiers were added in some other arrangement;
//! * nothing past the end of the list was written;
//! * no block in the padding is listed, because the padding is not a block;
//! * and the workgroup count the device wrote is the length of the list, not a ceiling of it —
//!   `prepare` divides by one here and a rounded count would dispatch over entries that are not
//!   there.
//!
//! # Two frames, not one
//!
//! Because the coarse scan writes its exclusive prefix back over its own input, exactly as the fine
//! one does. A frame that does not clear [`HOT_TOTALS`] first adds last frame's prefix to this
//! frame's counts, and the grand total climbs for ever — a failure that a single frame cannot see
//! and that cost this crate a work list once already. `live::wipe` is the first pass here for that
//! reason, and the second frame is what would catch it going missing.

// A test may panic — that is how it reports.
#![allow(clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use grit::Groups;
use grit::Host;
use grit::compact::{self, Over, Plan, Tier};
use grit::layout::{HOT, HOT_BASES, HOT_COUNTS, HOT_DENSE, HOT_GRAND, HOT_INDIRECT, HOT_TOTALS};

/// Blocks in the check.
///
/// **Not a multiple of any width's `per_block`**, so the array is padded and the padding has to stay
/// out of the list. A thousand and twenty-four would divide evenly at every width here and prove
/// less.
const BLOCKS: u32 = 1000;

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

/// Which blocks are hot, in a pattern that crosses tile boundaries rather than respecting them.
///
/// A run, a gap, a lone block, a stretch of nothing wide enough to be a whole tile at any width, and
/// a tail — so there is at least one tile that is entirely cold, one that is entirely hot, and
/// several that are neither. A pattern aligned to tiles would agree with a hierarchy that had
/// mistaken a tile for a block.
fn hot_blocks() -> Vec<u32> {
    (0..BLOCKS)
        .filter(|&block| match block {
            0..7 => true,
            33 => true,
            100..612 => block % 3 == 0,
            700..999 => block % 97 == 0,
            _ => false,
        })
        .collect()
}

#[test]
fn the_hot_list_is_every_hot_block_once_and_in_order() {
    let Some(host) = device("hot") else {
        return;
    };
    let width = host.width();
    let plan = match Plan::over(width, BLOCKS) {
        Ok(plan) => plan,
        Err(too_big) => {
            eprintln!("SKIPPED hot at width {width}: {too_big}");
            return;
        }
    };
    assert!(
        plan.tiles > 1,
        "this check is about the tiers above one tile and there is only {} at width {width}",
        plan.tiles
    );
    println!(
        "{} — subgroup {width}, {} blocks in {} tiles, padded to {}",
        host.name(),
        plan.blocks,
        plan.tiles,
        plan.padded()
    );

    let passes = [
        grit::live::wipe(width).expect("wipe"),
        grit::live::mark(width, Tier::COARSE, Over::Every).expect("hot mark"),
        compact::scan(width, HOT_COUNTS, HOT_BASES, HOT_TOTALS).expect("hot counts"),
        compact::scan(width, HOT_TOTALS, HOT_TOTALS, HOT_GRAND).expect("hot totals"),
        compact::gather(width, Tier::COARSE, Over::Every, plan.hot_dump()).expect("hot gather"),
        compact::prepare(width, 1, Tier::COARSE).expect("hot prepare"),
    ];
    let groups = [
        Groups::Fixed(1),
        Groups::Fixed(plan.tiles),
        Groups::Fixed(plan.hot_tiles),
        Groups::Fixed(1),
        Groups::Fixed(plan.tiles),
        Groups::Fixed(1),
    ];
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();

    let sizes = plan.sizes();
    let mut resident = host.resident(&sizes, &modules).expect("allocated");
    for (binding, &words) in sizes.iter().enumerate() {
        resident
            .write(binding, &vec![0_u32; words])
            .expect("cleared");
    }

    // The countdown a block carries, not a flag — anything above nothing is hot, and two is what
    // `live::wake` writes. The padding past `BLOCKS` stays at nought.
    let want = hot_blocks();
    let mut flags = vec![0_u32; sizes[HOT as usize]];
    for &block in &want {
        flags[block as usize] = 2;
    }
    resident.write(HOT as usize, &flags).expect("hot");

    // Something recognisable in the list, so a slot nobody wrote is not mistaken for a block.
    let untouched = u32::MAX;
    resident
        .write(
            HOT_DENSE as usize,
            &vec![untouched; sizes[HOT_DENSE as usize]],
        )
        .expect("dense");

    println!(
        "  {} of {} blocks hot — {:.1}%",
        want.len(),
        plan.blocks,
        want.len() as f64 * 100.0 / f64::from(plan.blocks)
    );

    // Twice, because the scan writes over its own input and the second frame is where that shows.
    for pass in 1..=2 {
        resident.frame(&groups).expect("dispatched");

        let grand = resident
            .read(HOT_GRAND as usize, 1)
            .expect("grand")
            .first()
            .copied()
            .unwrap_or(0);
        assert_eq!(
            grand as usize,
            want.len(),
            "frame {pass}: the device says {grand} blocks are hot and {} are",
            want.len()
        );

        let dense = resident
            .read(HOT_DENSE as usize, sizes[HOT_DENSE as usize])
            .expect("dense");
        let listed = &dense[..grand as usize];

        if let Some(at) = listed
            .iter()
            .zip(&want)
            .position(|(there, here)| there != here)
        {
            panic!(
                "frame {pass}: slot {at} of the hot list holds block {} and should hold {} — the \
                 tiers are added differently somewhere ({} tiles of {} blocks)",
                listed[at], want[at], plan.tiles, plan.per_block
            );
        }

        assert!(
            listed.iter().all(|&block| block < plan.blocks),
            "frame {pass}: a block from the padding is in the list, so the dump is in the wrong place"
        );
        assert!(
            dense[grand as usize..plan.hot_dump() as usize]
                .iter()
                .all(|&slot| slot == untouched),
            "frame {pass}: something was written past the end of the hot list"
        );

        let indirect = resident.read(HOT_INDIRECT as usize, 3).expect("indirect");
        assert_eq!(
            indirect,
            vec![grand, 1, 1],
            "frame {pass}: the workgroup count is not the length of the list — this list is \
             dispatched a workgroup an entry, so a ceiling is a rounding up of nothing"
        );
    }

    let saving = f64::from(plan.blocks) / (want.len().max(1) as f64);
    println!(
        "  decay, mark and gather run {} workgroups instead of {} — {saving:.1}× fewer, and the \
         coarse hierarchy cost {} to say so",
        want.len(),
        plan.blocks,
        plan.tiles * 2 + plan.hot_tiles + 1
    );
}

/// [`Plan::cold`] is the number `live::mark` computes for itself, at every width.
///
/// # Why this is worth a test of its own
///
/// The kernel does not call it. `live.rs` works the dump slot out in lanes — `blocks.div_ceil(
/// per_block) * per_block` — and a comment there says `compact::Plan::cold` is "the same number,
/// arrived at from the other side". That was an assertion in prose and nothing checked it: `cold`
/// is `tiles × per_block`, so the two agree only while `tiles` is `blocks` rounded up to a whole
/// number of them. A `tiles` derived from quads instead would leave the host and the device naming
/// different slots, and the symptom would be one hot block listed every frame for ever.
///
/// **This is arithmetic on the host**, so it covers the widths this machine has no device to run —
/// four, eight, sixteen and sixty-four lanes are checked here and nowhere else.
#[test]
fn the_dump_slot_is_the_same_number_the_kernel_works_out() {
    for width in [4_u32, 8, 16, 32, 64] {
        let mut checked = 0;
        for blocks in [1_u32, 2, 7, 63, 64, 65, 255, 256, 257, 1000, 4096] {
            let Ok(plan) = Plan::over(width, blocks) else {
                continue;
            };
            let theirs = plan.blocks.div_ceil(plan.per_block) * plan.per_block;
            assert_eq!(
                plan.cold(),
                theirs,
                "at subgroup {width} with {blocks} blocks the host says {} and the kernel {theirs}",
                plan.cold()
            );
            checked += 1;
        }
        assert!(checked > 0, "no plan at subgroup {width} was checkable");
    }
}
