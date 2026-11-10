#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use grit::compact::{Over, Plan};
use grit::layout::{AWAKE, HOT, HOT_DENSE, HOT_GRAND, HOT_INDIRECT, PARAMS};
use grit::sweep::{self, PER_LANE};
use grit::{Groups, live};
use simdr::decode;
use simdr::module::op;

const BLOCKS: u32 = 24;
const BINDING_DECORATION: u32 = 33;
const UNTOUCHED: u32 = 0xA5A5_A5A5;

fn tiers() -> [Over; 2] {
    [Over::Every, Over::Listed(HOT_DENSE)]
}

#[derive(Debug, PartialEq, Eq)]
enum Touch {
    Load,
    Store,
}

fn touches(words: &[u32], binding: u32) -> Vec<Touch> {
    let variable = decode::body(words)
        .find(|one| {
            one.opcode() == op::DECORATE
                && one.operands().get(1) == Some(&BINDING_DECORATION)
                && one.operands().get(2) == Some(&binding)
        })
        .and_then(|one| one.operands().first().copied())
        .unwrap();
    let mut chains = std::collections::BTreeSet::new();
    let mut found = Vec::new();
    for one in decode::body(words) {
        let operands = one.operands();
        match one.opcode() {
            op::ACCESS_CHAIN if operands.get(2) == Some(&variable) => {
                chains.insert(operands[1]);
            }
            op::LOAD if chains.contains(&operands[2]) => found.push(Touch::Load),
            op::STORE if chains.contains(&operands[0]) => found.push(Touch::Store),
            _ => {}
        }
    }
    found
}

#[test]
fn every_flag_is_loaded_before_the_first_flag_is_stored_at_every_width() {
    for width in [4, 8, 16, 32, 64] {
        for over in tiers() {
            let words = live::decay(width, over).unwrap();
            let seen = touches(&words, AWAKE);
            let loads = seen.iter().filter(|one| **one == Touch::Load).count();
            let stores = seen.len() - loads;
            assert_eq!(loads, PER_LANE as usize, "{width} {over:?}");
            assert_eq!(stores, PER_LANE as usize, "{width} {over:?}");
            let first_store = seen.iter().position(|one| *one == Touch::Store).unwrap();
            assert!(
                seen[first_store..].iter().all(|one| *one == Touch::Store),
                "{width} {over:?}: a load from AWAKE follows a store to it: {seen:?}"
            );
        }
    }
}

fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

fn flag(state: &mut u64) -> u32 {
    let roll = next(state);
    match roll % 8 {
        0 => 0,
        1 => 1,
        2 => 2,
        3 => u32::MAX,
        4 => (roll >> 8) as u32 % 4,
        _ => (roll >> 8) as u32,
    }
}

fn lowered(held: u32) -> u32 {
    held.max(1) - 1
}

fn run(over: Over, seed: u64) {
    let mut state = seed;
    let Some(host) = device("decaying") else {
        return;
    };
    let plan = Plan::over(host.width(), BLOCKS).unwrap();
    let before: Vec<u32> = (0..plan.quads).map(|_| flag(&mut state)).collect();
    let mut list: Vec<u32> = (0..plan.blocks).collect();
    for at in (1..list.len()).rev() {
        list.swap(at, (next(&mut state) % (at as u64 + 1)) as usize);
    }
    list.truncate(match over {
        Over::Every => list.len(),
        Over::Listed(_) => list.len() / 2 + 1,
    });
    check(&host, over, &before, &list);
}

fn check(host: &grit::Host, over: Over, before: &[u32], list: &[u32]) {
    let width = host.width();
    let plan = Plan::over(width, BLOCKS).unwrap();
    let per = plan.per_block as usize;
    assert_eq!(sweep::cells(width) as usize, per);
    assert_eq!(before.len(), plan.quads);
    let sizes = plan.sizes();
    let words = live::decay(width, over).unwrap();
    let modules: Vec<&[u32]> = vec![&words];

    let mut held = host.resident(&sizes, &modules).unwrap();
    for (binding, words) in sizes.iter().enumerate() {
        held.write(binding, &vec![0_u32; *words]).unwrap();
    }
    held.write(PARAMS as usize, &[0, plan.dump()]).unwrap();
    held.write(AWAKE as usize, before).unwrap();
    held.write(HOT as usize, &vec![UNTOUCHED; plan.padded() as usize])
        .unwrap();
    let stage: Vec<Groups> = match over {
        Over::Every => vec![Groups::Fixed(plan.blocks)],
        Over::Listed(_) => {
            held.write(HOT_DENSE as usize, list).unwrap();
            held.write(HOT_GRAND as usize, &[list.len() as u32])
                .unwrap();
            held.write(HOT_INDIRECT as usize, &[list.len() as u32, 1, 1])
                .unwrap();
            vec![Plan::hot_listed()]
        }
    };
    held.staged(&[&stage]).unwrap();

    let awake = held.read(AWAKE as usize, plan.quads).unwrap();
    let hot = held.read(HOT as usize, plan.padded() as usize).unwrap();

    let covered: Vec<u32> = match over {
        Over::Every => (0..plan.blocks).collect(),
        Over::Listed(_) => list.to_vec(),
    };
    let mut want_awake = before.to_vec();
    let mut want_hot = vec![UNTOUCHED; plan.padded() as usize];
    for &block in &covered {
        let span = block as usize * per..(block as usize + 1) * per;
        for at in span.clone() {
            want_awake[at] = lowered(before[at]);
        }
        want_hot[block as usize] = want_awake[span].iter().copied().max().unwrap();
    }
    assert_eq!(awake, want_awake, "{width} {over:?} flags");
    assert_eq!(hot, want_hot, "{width} {over:?} hot");
}

fn on_both_tiers(before: impl Fn(usize) -> u32, listed: &[u32]) {
    let Some(host) = device("decaying") else {
        return;
    };
    let plan = Plan::over(host.width(), BLOCKS).unwrap();
    let flags: Vec<u32> = (0..plan.quads).map(before).collect();
    check(&host, Over::Every, &flags, &[]);
    check(&host, Over::Listed(HOT_DENSE), &flags, listed);
}

#[test]
fn flags_at_the_ends_of_their_range_lower_by_one_and_stop_at_zero() {
    for held in [0, 1, 2, 3, u32::MAX - 1, u32::MAX] {
        on_both_tiers(|_| held, &[0, BLOCKS - 1]);
    }
}

#[test]
fn a_lone_flag_in_each_position_of_a_lane_is_the_blocks_mark() {
    let Some(host) = device("decaying") else {
        return;
    };
    let width = host.width();
    for position in 0..(PER_LANE as usize) {
        for lane in [0, 1, width as usize - 1] {
            let at = lane * PER_LANE as usize + position;
            on_both_tiers(
                |quad| {
                    if quad % sweep::cells(width) as usize == at {
                        9
                    } else {
                        0
                    }
                },
                &[BLOCKS - 1, 0, 5],
            );
        }
    }
}

#[test]
fn every_quad_stores_to_its_own_address() {
    on_both_tiers(|quad| quad as u32 + 2, &[3, 1, 2, 0]);
}

#[test]
fn an_empty_list_leaves_every_flag_and_mark_alone() {
    on_both_tiers(|quad| quad as u32 + 2, &[]);
}

#[test]
fn a_list_of_the_last_block_alone_leaves_the_others_alone() {
    on_both_tiers(|quad| quad as u32 % 5, &[BLOCKS - 1]);
}

#[test]
fn a_list_in_descending_order_marks_each_block_by_its_own_flags() {
    let descending: Vec<u32> = (0..BLOCKS).rev().collect();
    on_both_tiers(|quad| (quad as u32 * 7) % 11, &descending);
}

#[test]
fn decay_matches_a_cpu_model_over_every_block() {
    for seed in [
        0x9E37_79B9_7F4A_7C15,
        0x1234_5678_9ABC_DEF1,
        0xD1B5_4A32_D192_ED03,
    ] {
        run(Over::Every, seed);
    }
}

#[test]
fn decay_matches_a_cpu_model_over_a_shuffled_list_and_leaves_the_rest_alone() {
    for seed in [
        0x9E37_79B9_7F4A_7C15,
        0x1234_5678_9ABC_DEF1,
        0xD1B5_4A32_D192_ED03,
    ] {
        run(Over::Listed(HOT_DENSE), seed);
    }
}
