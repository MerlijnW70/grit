#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use grit::Host;
use grit::compact::{self, Over, Plan, Tier};
use grit::sweep;

const BLOCKS: u32 = 1024;

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

#[test]
fn the_work_list_is_every_awake_cell_once_and_in_block_order() {
    let Some(host) = device("compact") else {
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
    println!(
        "{} {width} {} {} {}",
        host.name(),
        plan.quads,
        plan.blocks,
        plan.tiles
    );

    let passes = [
        sweep::sweep(width).unwrap(),
        compact::scan(width, sweep::COUNTS, compact::BASES, compact::TOTALS).unwrap(),
        compact::scan(width, compact::TOTALS, compact::TOTALS, compact::GRAND).unwrap(),
        compact::gather(width, Tier::FINE, Over::Every, plan.dump()).unwrap(),
        compact::prepare(width, sweep::cells(width), Tier::FINE).unwrap(),
    ];
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let words: usize = passes.iter().map(Vec::len).sum();
    println!("{words} {}", passes.len());

    let sizes = plan.sizes();
    let mut resident = host.resident(&sizes, &modules).unwrap();

    let frame = 11_u32;
    let state: Vec<u32> = (0..plan.quads)
        .map(|at| (at as u32).wrapping_mul(0x9E37_79B1))
        .collect();
    let buckets = vec![u32::MAX; sizes[sweep::BUCKETS as usize]];

    resident
        .write(sweep::PARAMS as usize, &[frame, plan.dump()])
        .unwrap();
    resident.write(sweep::STATE as usize, &state).unwrap();
    resident.write(sweep::BUCKETS as usize, &buckets).unwrap();
    for binding in [
        sweep::COUNTS as usize,
        compact::BASES as usize,
        compact::TOTALS as usize,
        compact::GRAND as usize,
        compact::INDIRECT as usize,
    ] {
        resident
            .write(binding, &vec![0_u32; sizes[binding]])
            .unwrap();
    }
    resident
        .write(
            compact::DENSE as usize,
            &vec![u32::MAX; sizes[compact::DENSE as usize]],
        )
        .unwrap();

    let spent = resident.frame(&plan.groups()).unwrap();

    let ran_state = resident.read(sweep::STATE as usize, plan.quads).unwrap();
    let ran_counts = resident
        .read(sweep::COUNTS as usize, plan.blocks as usize)
        .unwrap();
    let ran_buckets = resident
        .read(sweep::BUCKETS as usize, sizes[sweep::BUCKETS as usize])
        .unwrap();
    let grand = resident
        .read(compact::GRAND as usize, 1)
        .unwrap()
        .first()
        .copied()
        .unwrap_or(0);
    let dense = resident.read(compact::DENSE as usize, plan.quads).unwrap();

    let (want_dense, want_grand) = compact::dense_reference(&plan, &ran_counts, &ran_buckets);

    assert_eq!(grand, want_grand);
    assert_eq!(grand as usize, want_dense.len());

    let listed = &dense[..grand as usize];
    if let Some(at) = listed
        .iter()
        .zip(&want_dense)
        .position(|(there, here)| there != here)
    {
        let wide = sweep::stride(width) as usize;
        panic!("{at} {} {} {wide}", listed[at], want_dense[at]);
    }

    let awake: Vec<u32> = (0..plan.quads)
        .filter(|&at| ran_state.get(at).copied().is_some_and(sweep::is_awake))
        .map(|at| u32::try_from(at).unwrap_or(u32::MAX))
        .collect();

    assert_eq!(listed.len(), awake.len());

    let mut sorted = listed.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), listed.len());
    assert_eq!(sorted, awake);

    assert!(dense[grand as usize..].iter().all(|&slot| slot == u32::MAX));

    let indirect = resident.read(compact::INDIRECT as usize, 3).unwrap();
    let due = grand.div_ceil(plan.per_block);
    assert_eq!(indirect, vec![due, 1, 1]);

    let saving = plan.quads as f64 / f64::from(grand.max(1));
    println!("{due} {} {saving:.0}", plan.blocks);

    let density = f64::from(grand) * 100.0 / plan.quads as f64;
    println!("{grand} {} {density:.1} {spent:.3?}", plan.quads);
    assert!(grand > 0 && (grand as usize) < plan.quads);
}

#[test]
fn the_reach_of_a_plan_is_the_last_grid_it_will_take() {
    // `reach` says in one place what `over` refuses in another, so the two are
    // asked the same question here rather than trusted to agree. No device:
    // both are arithmetic about the shape of the scan.
    for subgroup in [4_u32, 8, 16, 32, 64] {
        let per_block = u64::from(sweep::cells(subgroup));
        assert_eq!(
            Plan::reach(subgroup),
            per_block * per_block * per_block,
            "a block of {per_block} quads, and {per_block} squared of them"
        );

        let blocks = u32::try_from(Plan::reach(subgroup) / per_block).unwrap();
        assert!(
            Plan::over(subgroup, blocks).is_ok(),
            "the last grid that fits is one a plan takes"
        );
        assert!(
            Plan::over(subgroup, blocks + 1).is_err(),
            "and one block more is refused"
        );
    }

    assert_eq!(Plan::reach(32), 16_777_216, "a subgroup of 32");
    assert_eq!(Plan::reach(4), 32_768, "a subgroup of 4");
}
