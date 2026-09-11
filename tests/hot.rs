#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use grit::Groups;
use grit::Host;
use grit::compact::{self, Over, Plan, Tier};
use grit::layout::{HOT, HOT_BASES, HOT_COUNTS, HOT_DENSE, HOT_GRAND, HOT_INDIRECT, HOT_TOTALS};

const BLOCKS: u32 = 1000;

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
            eprintln!("SKIPPED {width} {too_big}");
            return;
        }
    };
    assert!(plan.tiles > 1);
    println!(
        "{} {width} {} {} {}",
        host.name(),
        plan.blocks,
        plan.tiles,
        plan.padded()
    );

    let passes = [
        grit::live::wipe(width).unwrap(),
        grit::live::mark(width, Tier::COARSE, Over::Every).unwrap(),
        compact::scan(width, HOT_COUNTS, HOT_BASES, HOT_TOTALS).unwrap(),
        compact::scan(width, HOT_TOTALS, HOT_TOTALS, HOT_GRAND).unwrap(),
        compact::gather(width, Tier::COARSE, Over::Every, plan.hot_dump()).unwrap(),
        compact::prepare(width, 1, Tier::COARSE).unwrap(),
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
    let mut resident = host.resident(&sizes, &modules).unwrap();
    for (binding, &words) in sizes.iter().enumerate() {
        resident.write(binding, &vec![0_u32; words]).unwrap();
    }

    let want = hot_blocks();
    let mut flags = vec![0_u32; sizes[HOT as usize]];
    for &block in &want {
        flags[block as usize] = 2;
    }
    resident.write(HOT as usize, &flags).unwrap();

    let untouched = u32::MAX;
    resident
        .write(
            HOT_DENSE as usize,
            &vec![untouched; sizes[HOT_DENSE as usize]],
        )
        .unwrap();

    println!(
        "{} {} {:.1}",
        want.len(),
        plan.blocks,
        want.len() as f64 * 100.0 / f64::from(plan.blocks)
    );

    for pass in 1..=2 {
        resident.frame(&groups).unwrap();

        let grand = resident
            .read(HOT_GRAND as usize, 1)
            .unwrap()
            .first()
            .copied()
            .unwrap_or(0);
        assert_eq!(grand as usize, want.len());

        let dense = resident
            .read(HOT_DENSE as usize, sizes[HOT_DENSE as usize])
            .unwrap();
        let listed = &dense[..grand as usize];

        if let Some(at) = listed
            .iter()
            .zip(&want)
            .position(|(there, here)| there != here)
        {
            panic!(
                "{pass} {at} {} {} {} {}",
                listed[at], want[at], plan.tiles, plan.per_block
            );
        }

        assert!(listed.iter().all(|&block| block < plan.blocks));
        assert!(
            dense[grand as usize..plan.hot_dump() as usize]
                .iter()
                .all(|&slot| slot == untouched)
        );

        let indirect = resident.read(HOT_INDIRECT as usize, 3).unwrap();
        assert_eq!(indirect, vec![grand, 1, 1]);
    }

    let saving = f64::from(plan.blocks) / (want.len().max(1) as f64);
    println!(
        "{} {} {saving:.1} {}",
        want.len(),
        plan.blocks,
        plan.tiles * 2 + plan.hot_tiles + 1
    );
}

#[test]
fn the_dump_slot_is_the_same_number_the_kernel_works_out() {
    for width in [4_u32, 8, 16, 32, 64] {
        let mut checked = 0;
        for blocks in [1_u32, 2, 7, 63, 64, 65, 255, 256, 257, 1000, 4096] {
            let Ok(plan) = Plan::over(width, blocks) else {
                continue;
            };
            let theirs = plan.blocks.div_ceil(plan.per_block) * plan.per_block;
            assert_eq!(plan.cold(), theirs);
            checked += 1;
        }
        assert!(checked > 0);
    }
}
