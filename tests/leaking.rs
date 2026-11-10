#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod budget;
mod common;

use common::device;

const QUARTER: usize = 16 << 20;

const SIZES: [usize; 4] = [QUARTER; 4];

const BYTES: u64 = (QUARTER * 4 * 4) as u64;

/// A SPIR-V header that declares no instruction: no entry point can be found
/// in it, so the pipeline made from it fails on every driver measured.
const HEADER_ONLY: [u32; 5] = [0x0723_0203, 0x0001_0300, 0, 8, 0];

#[test]
fn a_module_whose_pipeline_fails_three_times_leaves_the_heap_where_it_was() {
    let _alone = budget::alone();
    let Some(host) = device("leaking") else {
        return;
    };
    let tick = grit::live::tick(host.width()).unwrap();
    budget::three_failures(
        &host,
        BYTES,
        || host.resident(&SIZES, &[&tick]),
        || host.resident(&SIZES, &[&tick, &HEADER_ONLY]),
    );
}

#[test]
fn the_same_holds_when_the_bad_module_is_the_only_one() {
    let _alone = budget::alone();
    let Some(host) = device("leaking") else {
        return;
    };
    budget::three_failures(
        &host,
        BYTES,
        || host.resident(&SIZES, &[]),
        || host.resident(&SIZES, &[&HEADER_ONLY]),
    );
}
