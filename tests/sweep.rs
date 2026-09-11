#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use grit::Host;
use grit::sweep::{self, PER_LANE};

const BLOCKS: u32 = 64;

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
    println!("{} {width} {} {PER_LANE}", host.name(), sweep::cells(width));

    let spirv = match sweep::sweep(width) {
        Ok(spirv) => spirv,
        Err(refused) => {
            eprintln!("SKIPPED {width} {refused}");
            return;
        }
    };

    let frame = 7_u32;
    let state = seeded(cells);
    let buckets = vec![u32::MAX; region];

    let mut resident = host
        .resident(&[1, cells, region, BLOCKS as usize], &[&spirv])
        .unwrap();
    resident.write(sweep::PARAMS as usize, &[frame]).unwrap();
    resident.write(sweep::STATE as usize, &state).unwrap();
    resident.write(sweep::BUCKETS as usize, &buckets).unwrap();
    resident
        .write(sweep::COUNTS as usize, &vec![0; BLOCKS as usize])
        .unwrap();

    resident.frame(&[grit::Groups::Fixed(BLOCKS)]).unwrap();

    let ran_state = resident.read(sweep::STATE as usize, cells).unwrap();
    let ran_buckets = resident.read(sweep::BUCKETS as usize, region).unwrap();
    let ran_counts = resident
        .read(sweep::COUNTS as usize, BLOCKS as usize)
        .unwrap();

    let (want_state, want_buckets, want_counts) =
        sweep::sweep_reference(width, BLOCKS, frame, &state, &buckets);

    if let Some(at) = ran_state
        .iter()
        .zip(&want_state)
        .position(|(there, here)| there != here)
    {
        panic!("{at} {:#010x} {:#010x}", ran_state[at], want_state[at]);
    }

    assert_eq!(ran_counts, want_counts);

    let wide = sweep::stride(width) as usize;
    for block in 0..BLOCKS as usize {
        let live = ran_counts.get(block).copied().unwrap_or(0) as usize;
        let from = block * wide;
        let there = &ran_buckets[from..from + live];
        let here = &want_buckets[from..from + live];
        assert_eq!(there, here);
    }

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

        assert_eq!(listed.len(), expected.len());

        let mut sorted = listed.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), listed.len());

        let mut wanted = expected.clone();
        wanted.sort_unstable();
        assert_eq!(sorted, wanted);
    }

    let total: u32 = ran_counts.iter().sum();
    let density = f64::from(total) * 100.0 / cells as f64;
    println!("{total} {cells} {density:.1}");
    assert!(total > 0 && (total as usize) < cells);
}
