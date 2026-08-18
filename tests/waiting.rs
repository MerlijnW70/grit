//! What the wait is worth, timed alone.
//!
//! # Why this is its own file
//!
//! Tests in one binary run at the same time. With this beside the three in `tests/overlap.rs` the
//! same frame measured 4.25 ms and 9.32 ms depending on what else had started, and the overlap it
//! is here to show disappeared into the noise. One test to a binary is the whole of the fix.
//!
//! # Why the host waits rather than works
//!
//! It sleeps, and that is not a way of making this easy: it is the shape the caller has. `hyperia`
//! overlaps a frame with a **disk read**, and a thread waiting on an NVMe read is blocked, not
//! spinning.
//!
//! A version that spun instead — forty million integer operations — overlapped **badly** and
//! repeatably, coming to very nearly the sum of the two rather than to the longer of them. Nothing
//! here has explained that. It is written down rather than tuned away, because a busy host thread
//! is a real thing for an engine to have and this says it is not free.

// A test may panic — that is how it reports.
#![allow(clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::time::Instant;

use grit::{Groups, Host};
use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, U32};

/// How many words the kernel below walks.
const WORDS: usize = 8 << 20;

/// How many times a frame walks them.
///
/// One pass is a tenth of a millisecond here, which is smaller than the noise in any host loop long
/// enough to be worth overlapping with. Sixteen makes the frame milliseconds, so that the question
/// "did the host get the wait back" has an answer larger than the measurement error.
const PASSES: usize = 16;

/// How many words each invocation adds up.
const PER_LANE: u32 = 64;

/// A kernel that sums a run of binding 0 into binding 1, so a frame has real work in it.
fn walk(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, 2))?;
    let uint = kernel.index_type();

    let block = kernel.workgroup_index();
    let lane = kernel.local_index();
    let wide = kernel.module().constant_u32(subgroup)?;
    let run = kernel.module().constant_u32(PER_LANE)?;

    let first = kernel.module().i_mul(uint, block, wide)?;
    let first = kernel.module().i_add(uint, first, lane)?;
    let first = kernel.module().i_mul(uint, first, run)?;

    let mut running = kernel.load_at(0, first)?;
    for step in 1..PER_LANE {
        let along = kernel.module().constant_u32(step)?;
        let at = kernel.module().i_add(uint, first, along)?;
        let value = kernel.load_at(0, at)?;
        running = kernel.module().i_add(uint, running, value)?;
    }

    let zero = kernel.module().constant_u32(0)?;
    let _ = kernel.atomic_add_at(1, zero, running)?;
    kernel.finish()
}

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

/// How many workgroups cover [`WORDS`] at this subgroup width.
fn over(width: u32) -> u32 {
    u32::try_from(WORDS / (width as usize * PER_LANE as usize)).unwrap_or(1)
}

#[test]
fn the_wait_is_time_the_host_gets_back() {
    // The reason the split exists. The same frame and the same host work, once one after the other
    // and once overlapping — and the second has to be near the longer of the two rather than near
    // their sum.
    //
    // Compared against itself on this device in this run, never against a number from another day.
    let Some(host) = device("the_wait_is_time_the_host_gets_back") else {
        return;
    };
    let module = walk(host.width()).expect("a kernel");
    let every: Vec<&[u32]> = vec![&module; PASSES];
    let mut held = host
        .resident(&[WORDS, 1], &every)
        .expect("a device buffer");
    held.write(0, &vec![1; WORDS]).expect("a fill");

    let groups = vec![Groups::Fixed(over(host.width())); PASSES];
    let stages: Vec<&[Groups]> = groups.iter().map(core::slice::from_ref).collect();

    // What a host does while a device works: waits for something else. See the note at the top of
    // this file for why this sleeps rather than spins.
    let elsewhere = || std::thread::sleep(std::time::Duration::from_millis(4));

    // Warm both, so neither pays for a first-touch page or a cold clock.
    held.frame(&groups).expect("a frame");
    elsewhere();

    // The two halves alone, so the claim below is about the overlap and not about their sizes.
    let clock = Instant::now();
    held.frame(&groups).expect("a frame");
    let device_alone = clock.elapsed();
    let clock = Instant::now();
    elsewhere();
    let host_alone = clock.elapsed();

    let clock = Instant::now();
    let ticket = held.submit(&stages).expect("a submission");
    elsewhere();
    held.wait(ticket).expect("a landing");
    let overlapped = clock.elapsed();

    eprintln!(
        "  device {device_alone:?}, host {host_alone:?}, overlapped {overlapped:?}",
    );

    // Nearer the longer of the two than their sum. Halfway is the line: anything below it is an
    // overlap that happened, and anything above it is one that did not.
    let sum = device_alone + host_alone;
    let longer = device_alone.max(host_alone);
    let halfway = longer + (sum - longer) / 2;
    assert!(
        overlapped < halfway,
        "device {device_alone:?} and host {host_alone:?} overlapped to {overlapped:?}, which is \
         nearer their sum than the longer of them"
    );
}
