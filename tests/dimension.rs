#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use ash::vk;
use common::device;

use grit::compact::Plan;
use grit::host::{Limit, Limits};
use grit::layout::{
    AWAKE, BRUSH_Z, BUCKETS, DENSE, Dimension, LOOKING, LOOKING_Z, PARAMS, STATE, TALLY,
    TALLY_WORDS,
};
use grit::{Error, sand, sweep};

const SUBGROUP: u32 = 32;

const RANGE: u64 = 4_294_967_295;

const CELLS_512: usize = 512 * 512 * 512;

const REPORT_512: u64 = 1_360_334_948;

const PARAMS_GROWTH: u64 = 2 * 4;

const SPARE_512: u64 = 536_870_912;

fn octs_in(cells: usize) -> u32 {
    u32::try_from(cells / 8).unwrap()
}

fn world(subgroup: u32, cells: usize) -> Plan {
    let blocks = octs_in(cells).div_ceil(sweep::cells(subgroup));
    Plan::over_in(subgroup, blocks, Dimension::Three).unwrap()
}

fn bytes(sizes: &[usize]) -> u64 {
    sizes.iter().map(|&words| words as u64 * 4).sum()
}

fn limits(range: u64) -> Limits {
    Limits {
        storage_range: range,
        allocation: range,
        groups: u32::MAX,
        width: SUBGROUP,
        stages: vk::ShaderStageFlags::COMPUTE,
        operations: vk::SubgroupFeatureFlags::empty(),
        api: vk::API_VERSION_1_1,
    }
}

#[test]
fn the_two_dimensional_layout_keeps_the_values_it_always_had() {
    assert_eq!(Dimension::Two.corners(), 4);
    assert_eq!(Dimension::Two.corner_bits(), 2);
    assert_eq!(Dimension::Two.tally_words(), 6);
    assert_eq!(Dimension::Two.params_words(), 8);
    assert_eq!(Dimension::Two.seats(), sand::SEATS);
    assert_eq!(TALLY_WORDS, 6);

    let by_default = Plan::over(SUBGROUP, 40).unwrap();
    let named = Plan::over_in(SUBGROUP, 40, Dimension::Two).unwrap();
    assert_eq!(by_default.dimension, Dimension::Two);
    assert_eq!(by_default.sizes(), named.sizes());
    assert_eq!(by_default.grid, by_default.quads * 4);
    assert_eq!(by_default.sizes()[PARAMS as usize], 8);
    assert_eq!(by_default.sizes()[TALLY as usize], 6);
    assert_eq!(
        by_default.sizes()[STATE as usize],
        by_default.grid + by_default.quads * 4
    );
}

#[test]
fn the_three_dimensional_layout_has_eight_corners_seven_tally_words_and_ten_params() {
    assert_eq!(Dimension::Three.corners(), 8);
    assert_eq!(Dimension::Three.corner_bits(), 3);
    assert_eq!(Dimension::Three.tally_words(), 7);
    assert_eq!(Dimension::Three.params_words(), 10);
    assert_eq!(Dimension::Three.seats(), (sand::BOX as usize).pow(3));
    let used = [
        PARAMS as usize,
        1,
        2,
        3,
        4,
        5,
        LOOKING,
        LOOKING + 1,
        BRUSH_Z,
        LOOKING_Z,
    ];
    let mut words = used.to_vec();
    words.sort_unstable();
    words.dedup();
    assert_eq!(words.len(), used.len());
    assert_eq!(
        words.last().copied(),
        Some(Dimension::Three.params_words() - 1)
    );

    let held = Plan::over_in(SUBGROUP, 40, Dimension::Three).unwrap();
    assert_eq!(held.dimension, Dimension::Three);
    assert_eq!(held.sizes()[PARAMS as usize], 10);
    assert_eq!(held.sizes()[TALLY as usize], 7);
}

#[test]
fn a_three_dimensional_plan_holds_eight_cells_to_an_oct_and_no_spare() {
    for blocks in [1, 5, 40] {
        let held = Plan::over_in(SUBGROUP, blocks, Dimension::Three).unwrap();
        assert_eq!(held.quads, held.per_block as usize * blocks as usize);
        assert_eq!(held.grid, held.quads * 8);
        assert_eq!(held.sizes()[STATE as usize], held.quads * 8);
    }
}

#[test]
fn the_sizes_of_a_three_dimensional_world_follow_the_reports_words_per_oct() {
    for side in [64_usize, 128, 256, 512] {
        let cells = side * side * side;
        let held = world(SUBGROUP, cells);
        let octs = cells / 8;
        let sizes = held.sizes();
        assert_eq!(held.quads, octs, "{side}");
        assert_eq!(sizes[STATE as usize], 8 * octs, "{side}");
        assert_eq!(sizes[AWAKE as usize], 2 * octs, "{side}");
        assert_eq!(sizes[BUCKETS as usize], octs + octs / 8, "{side}");
        assert_eq!(sizes[DENSE as usize], octs + octs / 8, "{side}");

        let slack = if side >= 256 { 0.001 } else { 0.05 };
        let per_cell = bytes(&sizes) as f64 / cells as f64;
        assert!(
            (per_cell - 6.1353).abs() < slack,
            "{side}: {per_cell} bytes a cell, the report's figure without the spare"
        );
    }
}

#[test]
fn the_world_of_512_cubed_sums_to_the_bytes_the_report_computed() {
    let held = world(SUBGROUP, CELLS_512);
    let sizes = held.sizes();
    assert_eq!(held.quads, 16_777_216);
    assert_eq!(held.blocks, 65_536);
    assert_eq!(bytes(&sizes), REPORT_512 - SPARE_512 + PARAMS_GROWTH);
    assert_eq!(sizes[STATE as usize] as u64 * 4, 536_870_912);
    assert_eq!(Plan::reach(SUBGROUP) * 8, CELLS_512 as u64);
}

#[test]
fn the_brush_box_sets_the_awake_floor_of_a_small_three_dimensional_world() {
    let held = Plan::over_in(SUBGROUP, 1, Dimension::Three).unwrap();
    let seats = (sand::BOX as usize).pow(3);
    assert!(held.quads < seats);
    assert_eq!(held.sizes()[AWAKE as usize], held.quads + seats);
}

#[test]
fn a_world_past_512_cubed_takes_a_third_level_of_scan_at_a_subgroup_of_32() {
    let per_block = sweep::cells(SUBGROUP);
    let limit = per_block * per_block;
    let at = |blocks| Plan::over_in(SUBGROUP, blocks, Dimension::Three);
    assert_eq!(at(limit).unwrap().levels(), 2);
    assert_eq!(at(limit + 1).unwrap().levels(), 3);
    let indexed = u32::try_from(Plan::MOST_WORDS / 8 / u64::from(per_block)).unwrap();
    assert_eq!(at(indexed).unwrap().levels(), 3);
    assert!(at(indexed + 1).is_err());
    assert!(at(limit * per_block).is_err());
}

#[test]
fn a_binding_range_of_four_gibibytes_admits_512_cubed_and_refuses_1024_cubed_by_its_state() {
    let admits = limits(RANGE);

    let held = world(SUBGROUP, CELLS_512);
    assert!(admits.admits(&held.sizes(), None).is_ok());

    let wide = world(64, 1024 * 1024 * 1024);
    let sizes = wide.sizes();
    assert_eq!(sizes[STATE as usize] as u64 * 4, 4_294_967_296);
    let refused = admits.admits(&sizes, None);
    assert!(
        matches!(
            refused,
            Err(Error::TooLarge { binding, limit: Limit::StorageBufferRange, allowed, .. })
                if binding == STATE as usize && allowed == RANGE
        ),
        "{refused:?}"
    );
}

#[test]
fn the_device_admits_512_cubed_exactly_where_its_limits_allow_without_allocating() {
    let Some(host) =
        device("the_device_admits_512_cubed_exactly_where_its_limits_allow_without_allocating")
    else {
        return;
    };
    let held = world(SUBGROUP, CELLS_512);
    let sizes = held.sizes();
    let widest = sizes.iter().map(|&words| words as u64 * 4).max().unwrap();
    let reported = host.limits();
    eprintln!(
        "{}: widest binding {widest} bytes, storage range {}, allocation {}",
        host.name(),
        reported.storage_range,
        reported.allocation
    );

    let admitted = reported.admits(&sizes, None);
    if widest <= reported.ceiling() {
        assert!(admitted.is_ok(), "{}: {admitted:?}", host.name());
    } else {
        assert!(
            matches!(admitted, Err(Error::TooLarge { binding, .. }) if binding == STATE as usize),
            "{}: {admitted:?}",
            host.name()
        );
    }
    if host.name().contains("4080") {
        assert!(admitted.is_ok(), "{admitted:?}");
    }
}
