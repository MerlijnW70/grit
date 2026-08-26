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

use std::time::Duration;

use grit::{Error, Groups, Host};
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
    // **By name, not by `is_err`.** A refusal for some other reason is not this refusal, and a test
    // that takes any error takes the wrong one just as happily.
    assert!(
        matches!(held.submit(&stages), Err(Error::StillInFlight)),
        "two submissions were accepted for one command buffer"
    );
    assert!(
        matches!(held.frame(&groups), Err(Error::StillInFlight)),
        "a blocking frame was accepted over one in flight"
    );

    held.wait(ticket).expect("a landing");
    assert!(
        matches!(held.wait(ticket), Err(Error::NoSuchTicket { .. })),
        "the same ticket was waited on twice"
    );
    held.frame(&groups).expect("a frame, once nothing is in flight");
}


/// What each of the two bindings is filled with, so a swap between them is visible.
const FIRST: u32 = 0x0A11_0000;
const SECOND: u32 = 0x0B22_0000;

/// A kernel that adds one to word nought of binding 0 and of binding 1.
fn both(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, 2))?;
    let uint = kernel.index_type();
    let head = kernel.module().constant_u32(0)?;
    let one = kernel.module().constant_u32(1)?;
    for binding in 0..2 {
        let held = kernel.load_at(binding, head)?;
        let next = kernel.module().i_add(uint, held, one)?;
        kernel.store_at(binding, head, next)?;
    }
    kernel.finish()
}

/// Two answers out of one submission, in the order they were asked for.
///
/// # Why this had no test until now
///
/// Because the file it lives in is covered. `Resident::submit_reading_both` and
/// `Resident::wait_reading_both` were added so a frame producing two answers costs one round trip
/// rather than two — `tests/floor.rs` prices the second at 41.1–43.9 µs — and the caller they were added
/// for is `hyperia`, which has since vendored the file and tests its own copy. That left three
/// public entry points and one error variant reachable only from here, and reached by nothing:
/// `staged_reading_both`, `submit_reading_both`, `wait_reading_both`, and `Error::WrongOutputs`.
///
/// # What it has to tell apart
///
/// **That the two come back in the order they were asked for, and not merely that two came back.**
/// They land in one staging buffer one after the other, so a copy that overwrote the first with the
/// second, or handed them over swapped, would produce two plausible vectors. The two bindings are
/// therefore filled with different words before the frame, and the frame adds one to each — so the
/// answer names which binding it came from.
///
/// And that asking a frame for two answers when it copied one is refused rather than answered with
/// an empty second, which is a wrong answer that reads like a right one.
#[test]
fn two_answers_come_back_from_one_submission_in_the_order_they_were_asked_for() {
    let Some(host) = device("two_answers_come_back_from_one_submission") else {
        return;
    };
    let module = both(host.width()).expect("a kernel");
    let mut held = host
        .resident(&[2, 2], &[&module])
        .expect("two device buffers");
    // **Two words each, not one.** The staging buffer is sized to the largest binding, so two
    // answers of one word apiece do not fit a resident whose largest binding is one word -- which is
    // `Error::Overrun`, and is what this test hit first.
    held.write(0, &[FIRST, 0]).expect("the first");
    held.write(1, &[SECOND, 0]).expect("the second");

    let groups = [Groups::Fixed(1)];
    let stages: Vec<&[Groups]> = vec![&groups];

    let ticket = held
        .submit_reading_both(&stages, (0, 1), (1, 1))
        .expect("a submission asking for both");
    let (_, first, second) = held.wait_reading_both(ticket).expect("both answers");

    assert_eq!(
        (first.first().copied(), second.first().copied()),
        (Some(FIRST + 1), Some(SECOND + 1)),
        "the two answers came back as {first:?} and {second:?}, which is not binding 0 then binding 1"
    );

    // **And the blocking form of the same question**, which this file's own doc claimed to cover
    // and did not: `staged_reading_both` is `submit_reading_both` and `wait_reading_both` in one
    // call, and until a coverage run said so, nothing had ever entered it. The buffers are put back
    // first, because the kernel above has already added one to each.
    held.write(0, &[FIRST, 0]).expect("the first, again");
    held.write(1, &[SECOND, 0]).expect("the second, again");
    let (spent, first, second) = held
        .staged_reading_both(&stages, (0, 1), (1, 1))
        .expect("both answers in one call");
    assert_eq!(
        (first.first().copied(), second.first().copied()),
        (Some(FIRST + 1), Some(SECOND + 1)),
        "the blocking form answered {first:?} and {second:?}, which the submitting form did not"
    );
    assert!(spent > Duration::ZERO, "a submission that took no time at all");

    // And one answer asked for as two is refused. The frame below copies one buffer out; asking it
    // for two would otherwise hand back an empty second.
    let ticket = held
        .submit_reading(&stages, 0, 1)
        .expect("a submission asking for one");
    assert!(
        matches!(
            held.wait_reading_both(ticket),
            Err(Error::WrongOutputs {
                asked: 1,
                wanted: 2
            })
        ),
        "a frame that copied one buffer answered a question about two"
    );

    // **And the ticket is spent by that refusal.** The fence is waited on and the flight cleared
    // before the shape is checked, so the frame has landed and its one answer is gone. Written down
    // here because the doc said `NoSuchTicket` where the code says `WrongOutputs`, and running it is
    // what told them apart.
    assert!(
        matches!(held.wait(ticket), Err(Error::NoSuchTicket { held: None, .. })),
        "the refusal left the ticket outstanding"
    );
}
