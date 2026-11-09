#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use grit::compact::{Over, Plan, Tier};
use grit::layout::{AWAKE, HOT};
use grit::sweep;
use grit::{Groups, Host};

/// Runs one mark over `groups` blocks of `tier` with `flags` in its flag
/// binding and every other binding zero, and returns what it listed, block
/// by block in order.
fn listed(
    host: &Host,
    plan: &Plan,
    module: &[u32],
    tier: Tier,
    groups: u32,
    flags: &[u32],
) -> Vec<u32> {
    let sizes = plan.sizes();
    let mut held = host.resident(&sizes, &[module]).unwrap();
    for (binding, &words) in sizes.iter().enumerate() {
        held.write(binding, &vec![0_u32; words]).unwrap();
    }
    held.write(tier.flags as usize, flags).unwrap();
    held.frame(&[Groups::Fixed(groups)]).unwrap();

    let stride = sweep::stride(plan.subgroup) as usize;
    let counts = held.read(tier.counts as usize, groups as usize).unwrap();
    let buckets = held
        .read(tier.buckets as usize, groups as usize * stride)
        .unwrap();
    counts
        .iter()
        .enumerate()
        .flat_map(|(group, &count)| {
            let from = group * stride;
            buckets[from..from + count as usize].to_vec()
        })
        .collect()
}

#[test]
fn the_coarse_mark_lists_no_block_at_or_past_the_bound() {
    let Some(host) = device("coarse bound") else {
        return;
    };
    let width = host.width();
    let per_block = sweep::cells(width);
    let plan = Plan::over(width, per_block + per_block / 2 + 3).unwrap();
    assert_eq!(plan.tiles, 2);
    let (blocks, padded) = (plan.blocks, plan.padded());
    assert!(padded > blocks);

    let real = [0, 5, per_block, blocks - 1];
    let mut flags = vec![0_u32; plan.sizes()[HOT as usize]];
    for &block in &real {
        flags[block as usize] = 2;
    }
    for flag in &mut flags[blocks as usize..] {
        *flag = 2;
    }

    let bounded = |below: u32| {
        let module = grit::live::mark_below(width, Tier::COARSE, Over::Every, below).unwrap();
        listed(&host, &plan, &module, Tier::COARSE, plan.tiles, &flags)
    };

    assert_eq!(bounded(blocks), real);
    assert_eq!(bounded(blocks - 1), real[..3]);
    assert_eq!(bounded(0), Vec::<u32>::new());

    let open = grit::live::mark(width, Tier::COARSE, Over::Every).unwrap();
    let every = listed(&host, &plan, &open, Tier::COARSE, plan.tiles, &flags);
    let mut want = real.to_vec();
    want.extend(blocks..padded);
    assert_eq!(every, want);
    println!(
        "{} sg={width} blocks={blocks} padded={padded}: bounded {} listed, open {}",
        host.name(),
        real.len(),
        every.len()
    );
}

#[test]
fn the_fine_mark_lists_no_quad_at_or_past_the_bound() {
    let Some(host) = device("fine bound") else {
        return;
    };
    let width = host.width();
    let per_block = sweep::cells(width);
    let plan = Plan::over(width, 2).unwrap();
    let flags = vec![2_u32; plan.sizes()[AWAKE as usize]];

    let bounded = |below: u32| {
        let module = grit::live::mark_below(width, Tier::FINE, Over::Every, below).unwrap();
        listed(&host, &plan, &module, Tier::FINE, plan.blocks, &flags)
    };

    let all: Vec<u32> = (0..2 * per_block).collect();
    assert_eq!(bounded(u32::MAX), all);
    assert_eq!(bounded(2 * per_block), all);
    assert_eq!(bounded(per_block + 1), all[..=per_block as usize]);
    assert_eq!(bounded(1), [0]);
    assert_eq!(bounded(0), Vec::<u32>::new());
    println!("{} sg={width} fine bound held", host.name());
}
