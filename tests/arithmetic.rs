#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use grit::compact::{Over, Plan, Tier, dense_reference, gather, prepare, scan};
use grit::sand::{Grid, settle};
use grit::sweep::{is_awake, step, sweep, sweep_reference};
use simdr::decode;
use simdr::module::op;

const SUBGROUP: u32 = 32;

fn plan(blocks: u32) -> Plan {
    Plan::over(SUBGROUP, blocks).unwrap()
}

#[test]
fn a_plan_covers_every_block_and_rounds_the_last_tile_up_rather_than_down() {
    let per_block = plan(1).per_block;

    for blocks in [1, per_block - 1, per_block, per_block + 1, per_block * 3] {
        let held = plan(blocks);
        assert_eq!(held.blocks, blocks);
        assert!(held.tiles * held.per_block >= blocks);
        assert!((held.tiles - 1) * held.per_block < blocks);
    }
}

#[test]
fn a_plans_padding_is_its_tiles_and_never_fewer_than_its_blocks() {
    for blocks in [1, 7, 33, 64, 100] {
        let held = plan(blocks);
        assert_eq!(held.padded(), held.tiles * held.per_block);
        assert!(held.padded() >= blocks);
    }
}

#[test]
fn a_plan_over_more_blocks_than_the_tiers_reach_is_refused_rather_than_truncated() {
    let per_block = plan(1).per_block;
    let limit = per_block * per_block;

    assert!(Plan::over(SUBGROUP, limit).is_ok());
    assert!(Plan::over(SUBGROUP, limit + 1).is_err());
}

#[test]
fn a_plans_grid_is_four_cells_to_a_quad_and_its_quads_are_its_blocks() {
    for blocks in [1, 5, 40] {
        let held = plan(blocks);
        assert_eq!(held.quads, held.per_block as usize * blocks as usize);
        assert_eq!(held.grid, held.quads * 4);
    }
}

#[test]
fn every_binding_a_plan_sizes_is_large_enough_to_be_written_to() {
    for blocks in [1, 9, 64] {
        let held = plan(blocks);
        for size in held.sizes() {
            assert!(size > 0);
        }
    }
}

#[test]
fn a_wider_plan_never_asks_for_less_room_than_a_narrower_one() {
    let small = plan(8).sizes();
    let large = plan(64).sizes();

    for (little, big) in small.iter().zip(large.iter()) {
        assert!(big >= little);
    }
}

#[test]
fn the_step_hash_moves_with_the_frame_and_with_the_cell() {
    assert_ne!(step(1, 0), step(1, 1));
    assert_ne!(step(1, 7), step(2, 7));
    assert_eq!(step(5, 3), step(5, 3));
}

#[test]
fn waking_is_a_property_of_the_top_byte_and_not_of_the_whole_word() {
    assert!(is_awake(0x0000_0000));
    assert!(!is_awake(0xFF00_0000));
    assert_eq!(is_awake(0x0000_FFFF), is_awake(0x0000_0000));
}

#[test]
fn the_reference_sweep_leaves_the_state_it_was_given_the_length_it_was_given() {
    let held = plan(4);
    let state = vec![0_u32; held.padded() as usize * 64];
    let buckets = vec![0_u32; held.sizes()[6]];

    let (stepped, written, counts) = sweep_reference(SUBGROUP, held.blocks, 0, &state, &buckets);

    assert_eq!(stepped.len(), state.len());
    assert_eq!(written.len(), buckets.len());
    assert_eq!(counts.len(), held.blocks as usize);
}

#[test]
fn the_reference_sweep_moves_with_the_frame_it_is_given() {
    let held = plan(2);
    let state: Vec<u32> = (0..held.padded() as usize * 64)
        .map(|one| one as u32)
        .collect();
    let buckets = vec![0_u32; held.sizes()[6]];

    let (first, _, _) = sweep_reference(SUBGROUP, held.blocks, 0, &state, &buckets);
    let (second, _, _) = sweep_reference(SUBGROUP, held.blocks, 1, &state, &buckets);

    assert_ne!(first, second);
}

#[test]
fn the_dense_reference_counts_what_it_was_handed_and_no_more() {
    let held = plan(4);
    let counts = vec![1_u32; held.blocks as usize];
    let buckets = vec![0_u32; held.blocks as usize * held.per_block as usize];

    let (dense, total) = dense_reference(&held, &counts, &buckets);

    assert_eq!(total, counts.iter().sum::<u32>());
    assert!(dense.len() >= total as usize);
}

#[test]
fn every_emitter_here_builds_a_module_for_arguments_it_accepts() {
    let grid = Grid::of(64, 64);

    let emitted: [Vec<u32>; 4] = [
        settle(SUBGROUP, grid).unwrap(),
        sweep(SUBGROUP).unwrap(),
        scan(SUBGROUP, 2, 3, 4).unwrap(),
        gather(SUBGROUP, Tier::FINE, Over::Every, 0).unwrap(),
    ];

    for words in emitted {
        assert_eq!(words[0], 0x0723_0203);
        assert!(words.len() > 32);
    }
}

#[test]
fn a_gather_over_a_list_is_not_the_gather_over_everything() {
    let first = gather(SUBGROUP, Tier::FINE, Over::Every, 0).unwrap();
    let second = gather(SUBGROUP, Tier::FINE, Over::Listed(9), 0).unwrap();

    assert_ne!(first, second);
}

#[test]
fn a_plan_sizes_every_binding_to_the_number_its_arithmetic_produces() {
    let held = plan(8);

    assert_eq!(held.per_block, 256);
    assert_eq!(held.tiles, 1);
    assert_eq!(held.hot_tiles, 1);
    assert_eq!(held.quads, 2048);
    assert_eq!(held.grid, 8192);
    assert_eq!(
        held.sizes(),
        [
            8, 16384, 2304, 256, 256, 256, 2304, 1, 3, 4096, 1, 6, 257, 288, 256, 256, 256, 288, 1,
            3
        ]
    );
}

#[test]
fn the_step_hash_is_the_hash_it_is_and_not_a_neighbouring_one() {
    assert_eq!(step(1, 0), 2_246_856_790);
    assert_eq!(step(1, 1), 396_126_466);
    assert_eq!(step(7, 3), 1_585_740_898);
}

#[test]
fn waking_stops_at_the_boundary_and_includes_it() {
    let last_awake = (0_u32..=255)
        .take_while(|top| is_awake(top << 24))
        .last()
        .unwrap();

    assert_eq!(last_awake, 13);
    assert!(is_awake(last_awake << 24));
    assert!(!is_awake((last_awake + 1) << 24));
}

#[test]
fn the_reference_sweep_writes_the_values_it_writes_and_not_merely_the_right_number_of_them() {
    let held = plan(8);
    let state: Vec<u32> = (0..held.padded() as usize * 64)
        .map(|one| one as u32)
        .collect();
    let buckets = vec![0_u32; held.sizes()[6]];

    let (stepped, written, counts) = sweep_reference(SUBGROUP, held.blocks, 0, &state, &buckets);

    assert_eq!(
        &stepped[..8],
        [
            0,
            2_246_856_790,
            198_680_749,
            2_445_537_540,
            397_361_499,
            2_644_218_289,
            596_042_248,
            2_842_899_039
        ]
    );
    assert_eq!(&counts[..8], [15, 14, 14, 14, 15, 13, 13, 14]);
    assert_eq!(written.iter().filter(|one| **one != 0).count(), 111);
}

#[test]
fn the_dense_reference_places_each_block_where_its_stride_puts_it() {
    let held = plan(8);
    let counts = vec![1_u32; held.blocks as usize];
    let buckets: Vec<u32> = (0..held.blocks as usize * held.per_block as usize)
        .map(|one| one as u32)
        .collect();

    let (dense, total) = dense_reference(&held, &counts, &buckets);

    assert_eq!(total, 8);
    assert_eq!(&dense[..8], [0, 288, 576, 864, 1152, 1440, 1728, 2016]);
}

#[test]
fn the_prepare_pass_bakes_the_mask_its_arithmetic_produces_and_not_its_neighbour() {
    let words = prepare(SUBGROUP, 256, Tier::FINE).unwrap();
    let mut held: Vec<u32> = decode::body(&words)
        .filter(|one| one.opcode() == op::CONSTANT)
        .filter_map(|one| one.operands().get(2).copied())
        .collect();
    held.sort_unstable();
    held.dedup();

    assert_eq!(held, [0, 1, 2, 3, 8, 255]);
}
