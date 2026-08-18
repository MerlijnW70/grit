//! A submission the host does not wait for, and what it is worth.
//!
//! # Why this is a test and not only a benchmark
//!
//! Because the thing that can go wrong is not slowness. A submission split into a start and a wait
//! is a submission whose command buffer could be reset while it runs, whose read-back could be
//! mapped before the fence, and whose ticket could be waited on twice. Each of those is a wrong
//! answer or undefined behaviour rather than a slow one, and none of them shows up in a timing.
//!
//! `tests/waiting.rs` holds the timing, because a split that bought nothing would be worth taking
//! back out.
//!
//! # Why the timing is in its own file
//!
//! Because tests in one binary run at the same time, and three others hammering the same device is
//! not a background anything can be timed against. Measured with all four in one file, the same
//! frame came back at 4.25 ms and at 9.32 depending on what else had started.
//!
//! # Why the host waits rather than works
//!
//! That test spends its host time asleep. That is not a way of making the test easy: it is the
//! shape the caller has. `hyperia` overlaps a frame with a **disk read**, and a thread waiting on
//! an NVMe read is blocked, not spinning.
//!
//! A version of it that spun instead — forty million integer operations — overlapped **badly**, and
//! repeatably: 4.25 ms of device work and 3.34 ms of host work came to 6.95 ms rather than to the
//! 4.3 the sleeping version reaches. Nothing here has explained that, and it is written down rather
//! than tuned away, because a busy host thread is a real thing for an engine to have and this says
//! it is not free.

// A test may panic — that is how it reports.
#![allow(clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

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
fn a_frame_waited_for_later_is_the_frame_that_was_submitted() {
    // The first thing to get wrong: the answer. A frame started and collected later must produce
    // exactly what the same frame produces when it is waited on immediately — same kernel, same
    // buffers, same counts.
    let Some(host) = device("a_frame_waited_for_later_is_the_frame_that_was_submitted") else {
        return;
    };
    let module = walk(host.width()).expect("a kernel");
    let every: Vec<&[u32]> = vec![&module; PASSES];
    let mut held = host
        .resident(&[WORDS, 1], &every)
        .expect("a device buffer");
    held.write(0, &vec![3; WORDS]).expect("a fill");

    let groups = vec![Groups::Fixed(over(host.width())); PASSES];
    let stages: Vec<&[Groups]> = groups.iter().map(core::slice::from_ref).collect();

    held.write(1, &[0]).expect("a clear");
    held.frame(&groups).expect("a frame");
    let blocking = held.read(1, 1).expect("a read");
    assert_eq!(
        blocking,
        vec![(3 * WORDS * PASSES) as u32],
        "the kernel did not walk it"
    );

    held.write(1, &[0]).expect("a clear");
    let ticket = held.submit(&stages).expect("a submission");
    // Whatever the host likes, as long as it is not this frame's buffers.
    let mine: u64 = (0..1_000_000_u64).sum();
    assert_eq!(mine, 499_999_500_000);
    held.wait(ticket).expect("a landing");

    assert_eq!(held.read(1, 1).expect("a read"), blocking);
}

#[test]
fn a_read_back_started_early_is_read_after_the_fence_and_not_before() {
    // The second thing to get wrong: reading the staging buffer before the copy into it has
    // landed. `submit_reading` records the copy and hands back a ticket; only `wait_reading` may
    // map it, and what it gives back has to be the finished answer rather than the last frame's.
    let Some(host) = device("a_read_back_started_early_is_read_after_the_fence") else {
        return;
    };
    let module = walk(host.width()).expect("a kernel");
    let every: Vec<&[u32]> = vec![&module; PASSES];
    let mut held = host
        .resident(&[WORDS, 1], &every)
        .expect("a device buffer");
    let groups = vec![Groups::Fixed(over(host.width())); PASSES];
    let stages: Vec<&[Groups]> = groups.iter().map(core::slice::from_ref).collect();

    for fill in [2_u32, 5] {
        held.write(0, &vec![fill; WORDS]).expect("a fill");
        held.write(1, &[0]).expect("a clear");

        let ticket = held.submit_reading(&stages, 1, 1).expect("a submission");
        let (_, words) = held.wait_reading(ticket).expect("a landing");
        assert_eq!(
            words,
            vec![(fill as usize * WORDS * PASSES) as u32],
            "the read came back with the wrong frame's answer"
        );
    }
}

#[test]
fn a_second_submission_while_one_is_in_flight_is_refused() {
    // Not queued and not silently waited for. There is one command buffer; recording into it while
    // the device is reading it is undefined behaviour, and a caller who has lost track of a ticket
    // needs to be told rather than to have the overlap quietly taken away.
    let Some(host) = device("a_second_submission_while_one_is_in_flight_is_refused") else {
        return;
    };
    let module = walk(host.width()).expect("a kernel");
    let every: Vec<&[u32]> = vec![&module; PASSES];
    let mut held = host
        .resident(&[WORDS, 1], &every)
        .expect("a device buffer");
    let groups = vec![Groups::Fixed(over(host.width())); PASSES];
    let stages: Vec<&[Groups]> = groups.iter().map(core::slice::from_ref).collect();

    let ticket = held.submit(&stages).expect("a submission");
    assert!(
        held.submit(&stages).is_err(),
        "two submissions were accepted for one command buffer"
    );
    assert!(
        held.frame(&groups).is_err(),
        "a blocking frame was accepted over one in flight"
    );

    held.wait(ticket).expect("a landing");
    assert!(
        held.wait(ticket).is_err(),
        "the same ticket was waited on twice"
    );
    held.frame(&groups).expect("a frame, once nothing is in flight");
}

