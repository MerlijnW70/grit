//! What happens when a submission does not come back.
//!
//! # Why the patience is the seam, and there is no test-only door
//!
//! Poisoning needs a fence that does not signal, and the honest way to get one is a dispatch that
//! hangs — which is a thing no test can ask a device for on purpose and then recover from. The
//! alternative usually reached for is a hidden method that sets the flag directly, and it tests the
//! wrong thing: it proves the *guards* work while stepping over the code that decides to raise
//! them.
//!
//! [`Resident::set_patience`] is already public and does the whole job — but the bound has to be
//! **zero**, and the first version of this file used a nanosecond and was wrong about why that was
//! safe. A Vulkan timeout is a floor and not a ceiling: the implementation may wait longer, and on
//! Windows the timer granularity is about a millisecond. So a nanosecond bound let a 253 µs frame
//! land and the wait came back `Ok`. Two of these four tests failed the first time they were run,
//! and that is what they were saying.
//!
//! Zero is the one value the specification pins down — it does not wait at all, and reports
//! `VK_TIMEOUT` if the fence is not already signalled. It reaches `await_fence`'s timeout branch by
//! exactly the return code a hung dispatch reaches it by.
//!
//! **This file used to add "asked immediately after `queue_submit` on a frame with real work in it,
//! that is not a race". It is a race.** Between `queue_submit` returning and `wait_for_fences` being
//! called there is a window, and a host thread can lose its timeslice inside it — on Windows for
//! milliseconds, which is far longer than a frame this size takes. Then the fence *is* already
//! signalled, the wait comes back `Ok`, and the test that demanded `Wedged` fails for a reason that
//! has nothing to do with the bound.
//!
//! Found where this file is vendored into `hyperia`, which runs it beside two hundred other tests
//! on one card: it failed about one run in eight there. It has not been seen to fail here, where
//! eleven test binaries make a quieter machine — which is the same reason `tests/waiting.rs` hid a
//! defect of its own for months, and not a reason to think this one is absent.
//!
//! [`wedge`] is the answer: an `Ok` frame poisons nothing, so it asks again. The claim is that the
//! timeout branch is *reachable*, and one reaching of it proves that.
//!
//! What that does **not** reach is a fence that never signals at all. Nothing in a test suite can
//! assert on that, because the failure is the absence of a return — a test that hangs reports
//! nothing, which is the whole reason the bound exists.
//!
//! It is worth saying plainly that no observed failure has needed this yet. It was written for the
//! sandboxes `hyperia`'s mutation gate was losing, and those turned out to be slow mutants escaping
//! a process-tree kill rather than anything waiting on a fence. See `Resident::await_fence`.
//!
//! # Why this is its own file
//!
//! Every test here ends with a device nobody destroys — that is what `Error::Wedged` promises: a
//! queue with work on it that will never finish is not something a teardown may wait for, so both of
//! them step over it and leak instead. Each test opens its own [`Host`], so what is poisoned is
//! contained to the test that poisoned it.

// A test may panic — that is how it reports.
#![allow(clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::time::Duration;

use grit::{Error, Groups, Host, Resident};
use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, U32};

/// How many words the kernel below walks.
///
/// Small on purpose. The frame here is not being measured, only made to exist, and a frame that is
/// over quickly is a frame whose leaked buffers are small.
const WORDS: usize = 1 << 16;

/// How many words each invocation adds up.
const PER_LANE: u32 = 4;

/// The bound the tests set when they want the wait to give up.
///
/// Zero rather than some small number: a bound under the platform's timer granularity is a bound
/// the driver is free to overshoot, and it did. See the note at the top of this file for the run
/// that established it.
const IMPATIENT: Duration = Duration::ZERO;

/// How many frames are asked for before the timeout branch is declared unreachable.
///
/// Each is a quarter of a megabyte walked once, so the loop is free; what it is buying is immunity
/// to the host thread losing its timeslice in a window a microsecond wide. See the note at the top.
const TRIES: usize = 8;

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

/// Run a frame at [`IMPATIENT`] until the fence does not beat the host to the wait.
///
/// Hands back the bound that was reported, or `None` if the fence won every time — which is a
/// failure worth having, because it would mean the timeout branch is not reachable at all.
///
/// See the note at the top of this file for why one call is not enough. An `Ok` frame poisons
/// nothing, so asking again costs a frame of a quarter of a megabyte and nothing else.
fn wedge(held: &mut Resident<'_>, groups: &[Groups]) -> Option<Duration> {
    for _ in 0..TRIES {
        match held.frame(groups) {
            Err(Error::Wedged { after }) => return Some(after),
            Err(other) => panic!("expected the device to be presumed lost, got {other:?}"),
            Ok(_) => {}
        }
    }
    None
}

#[test]
fn a_fence_that_does_not_come_back_in_time_presumes_the_device_lost() {
    let Some(host) = device("a_fence_that_does_not_come_back_in_time_presumes_the_device_lost")
    else {
        return;
    };
    let module = walk(host.width()).expect("a kernel");
    let every: Vec<&[u32]> = vec![&module];
    let mut held = host.resident(&[WORDS, 1], &every).expect("a device buffer");
    held.write(0, &vec![1; WORDS]).expect("a fill");

    // The default is what a caller gets and it is not what is being tested here.
    assert_eq!(
        held.patience(),
        Duration::from_secs(10),
        "the default patience is ten seconds"
    );

    held.set_patience(IMPATIENT);
    let groups = [Groups::Fixed(over(host.width()))];

    // **The bound is reported, not the wait.** A caller who set a nanosecond is told a nanosecond,
    // so the error says which bound was crossed rather than how long the driver actually took.
    let after = wedge(&mut held, &groups).expect("the timeout branch to be reachable at all");
    assert_eq!(after, IMPATIENT, "the bound that was crossed");
}

#[test]
fn a_wedged_device_refuses_everything_afterwards() {
    let Some(host) = device("a_wedged_device_refuses_everything_afterwards") else {
        return;
    };
    let module = walk(host.width()).expect("a kernel");
    let every: Vec<&[u32]> = vec![&module];
    let mut held = host.resident(&[WORDS, 1], &every).expect("a device buffer");
    held.write(0, &vec![1; WORDS]).expect("a fill");

    held.set_patience(IMPATIENT);
    let groups = [Groups::Fixed(over(host.width()))];
    assert!(
        wedge(&mut held, &groups).is_some(),
        "the frame is what poisons it"
    );

    // **Every door, not the one that was walked through.** The submission still owns the command
    // buffer and every buffer bound to it, so a later call must not record into them, map them or
    // wait on them — and the patience is spent once, not again on each of these.
    assert!(
        matches!(held.frame(&groups), Err(Error::Wedged { .. })),
        "a second frame"
    );
    assert!(
        matches!(held.write(0, &[7]), Err(Error::Wedged { .. })),
        "a write"
    );
    assert!(
        matches!(held.write_at(0, 1, &[7]), Err(Error::Wedged { .. })),
        "a write at an offset"
    );
    assert!(
        matches!(held.read(1, 1), Err(Error::Wedged { .. })),
        "a read, which stages and waits"
    );
    assert!(
        matches!(held.submit(&[&groups]), Err(Error::Wedged { .. })),
        "a submission that would be waited for later"
    );
}

#[test]
fn a_wedged_device_is_dropped_rather_than_waited_for() {
    // The reason the bound exists at all, as near as a test can get to it. `device_wait_idle` on a
    // queue that never drains is the second place a process parks in the driver for good, and both
    // teardowns are supposed to step over it. If they do not, this test does not fail — it hangs,
    // which is the symptom being removed.
    let Some(host) = device("a_wedged_device_is_dropped_rather_than_waited_for") else {
        return;
    };
    let module = walk(host.width()).expect("a kernel");
    let every: Vec<&[u32]> = vec![&module];
    let mut held = host.resident(&[WORDS, 1], &every).expect("a device buffer");
    held.write(0, &vec![1; WORDS]).expect("a fill");

    held.set_patience(IMPATIENT);
    let groups = [Groups::Fixed(over(host.width()))];
    assert!(wedge(&mut held, &groups).is_some(), "poisoned");

    drop(held);
    drop(host);

    // There is no assertion after this on purpose: returning *is* the assertion, and the failure
    // this guards against cannot be caught by one — a teardown that waits on a wedged queue never
    // reaches a line that could fail.
}

#[test]
fn the_bound_does_not_get_in_the_way_of_ordinary_work() {
    // The other half, and the one that would catch a bound set too low: at the default patience a
    // frame lands and answers, exactly as it did when the wait had no bound at all.
    let Some(host) = device("the_bound_does_not_get_in_the_way_of_ordinary_work") else {
        return;
    };
    let module = walk(host.width()).expect("a kernel");
    let every: Vec<&[u32]> = vec![&module];
    let mut held = host.resident(&[WORDS, 1], &every).expect("a device buffer");
    held.write(0, &vec![1; WORDS]).expect("a fill");

    let groups = [Groups::Fixed(over(host.width()))];
    held.frame(&groups).expect("a frame at the default patience");

    let summed = held.read(1, 1).expect("the answer");
    assert_eq!(
        summed.first().copied(),
        Some(u32::try_from(WORDS).expect("a count that fits a word")),
        "every word was one and every one of them was added"
    );
}
