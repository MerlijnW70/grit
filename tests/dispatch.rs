//! What a dispatch of no workgroups does, which is nothing.
//!
//! # Why this is worth a test
//!
//! Because it used to do something. `Groups::Fixed(count)` was dispatched as `count.max(1)`, so a
//! caller asking for no work got one workgroup of it. Nothing in this repository ever asked for
//! none, which is exactly why it went unnoticed — and a caller holding several passes and wanting
//! one of them is the obvious use, since a pass is recorded whether it is wanted or not.
//!
//! A rounded-up dispatch is not a slow dispatch. It runs a kernel over buffers the caller may have
//! had no reason to size for it.

// A test may panic — that is how it reports.
#![allow(clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::time::Duration;

use grit::{Error, Groups, Host, Run};
use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, U32};

/// What the kernel writes.
const MARK: u32 = 0x5EED_BEEF;

/// A kernel that writes one word to binding 0 and does nothing else.
fn mark(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, 1))?;
    let head = kernel.module().constant_u32(0)?;
    let value = kernel.module().constant_u32(MARK)?;
    kernel.store_at(0, head, value)?;
    kernel.finish()
}

/// A kernel that adds one to the word at binding 0 — it reads what a pass before it wrote.
fn bump(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, 1))?;
    let uint = kernel.index_type();
    let head = kernel.module().constant_u32(0)?;
    let one = kernel.module().constant_u32(1)?;
    let held = kernel.load_at(0, head)?;
    let next = kernel.module().i_add(uint, held, one)?;
    kernel.store_at(0, head, next)?;
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

#[test]
fn a_pass_dispatched_over_no_workgroups_does_not_run() {
    let Some(host) = device("a_pass_dispatched_over_no_workgroups_does_not_run") else {
        return;
    };
    let module = mark(host.width()).expect("a kernel");
    let mut held = host.resident(&[4], &[&module]).expect("a device buffer");
    held.write(0, &[0, 0, 0, 0]).expect("a clear");

    held.frame(&[Groups::Fixed(0)]).expect("a frame");
    assert_eq!(
        held.read(0, 1).expect("a read"),
        vec![0],
        "a pass told to run over nothing ran anyway"
    );

    // And one workgroup does what the kernel says, so the test is about the count and not about a
    // kernel that never worked.
    held.frame(&[Groups::Fixed(1)]).expect("a frame");
    assert_eq!(held.read(0, 1).expect("a read"), vec![MARK]);
}

/// A count of nought **in a buffer the device reads** stops the pass, and one stops it too.
///
/// # Why this is not the test above
///
/// `Groups::Fixed(0)` is not recorded at all — `Resident` drops it before the command buffer exists,
/// and the check above is that it does. `Groups::From` is the opposite case and the one this whole
/// repository turns on: the pass **is** recorded, because the host does not know what the count says,
/// and the device reads three words at dispatch time and decides. Those are different mechanisms and
/// only one of them was checked by what it left behind.
///
/// The other was checked by its cost. `tests/floor.rs` fits a line through a sweep of switched-off
/// passes and asserts that one costs less than a pass left on — which is true on an RTX 4080, 46 ns
/// against 120, and **false on an integrated Radeon**, 99 against 89 over three runs each. A
/// mechanism proved by a stopwatch is proved only on the hardware the stopwatch was held on. This
/// proves it by what is in the buffer afterwards, which every device has to agree about.
#[test]
fn a_count_of_nought_in_a_buffer_stops_the_pass_that_reads_it() {
    let Some(host) = device("a_count_of_nought_in_a_buffer_stops_the_pass_that_reads_it") else {
        return;
    };
    let module = mark(host.width()).expect("a kernel");
    // Binding 0 is what the kernel writes; binding 1 holds the x, y and z the dispatch reads.
    let mut held = host
        .resident(&[4, 3], &[&module])
        .expect("two device buffers");
    let over = [Groups::From { binding: 1, at: 0 }];

    held.write(0, &[0, 0, 0, 0]).expect("a clear");
    held.write(1, &[0, 1, 1]).expect("a count of nought");
    held.frame(&over).expect("a frame");
    assert_eq!(
        held.read(0, 1).expect("a read"),
        vec![0],
        "the count buffer said nought workgroups and the pass ran anyway"
    );

    // And the same recorded pass, over the same buffer, with the one word changed. **Nothing else
    // moves**: the command buffer `Resident` kept is the one that just did nothing.
    held.write(1, &[1, 1, 1]).expect("a count of one");
    held.frame(&over).expect("a frame");
    assert_eq!(
        held.read(0, 1).expect("a read"),
        vec![MARK],
        "the count buffer said one workgroup and the pass did not run"
    );
}

#[test]
fn one_pass_of_several_can_be_run_while_the_others_are_skipped() {
    // The use the clamp made impossible. Three passes over one buffer, each writing a different
    // word; only the middle one is dispatched.
    let Some(host) = device("one_pass_of_several_can_be_run_while_the_others_are_skipped") else {
        return;
    };
    let module = mark(host.width()).expect("a kernel");
    let mut held = host
        .resident(&[4], &[&module, &module, &module])
        .expect("a device buffer");
    held.write(0, &[0, 0, 0, 0]).expect("a clear");

    held.frame(&[Groups::Fixed(0), Groups::Fixed(1), Groups::Fixed(0)])
        .expect("a frame");
    assert_eq!(held.read(0, 1).expect("a read"), vec![MARK]);

    held.write(0, &[0, 0, 0, 0]).expect("a clear");
    held.frame(&[Groups::Fixed(0), Groups::Fixed(0), Groups::Fixed(0)])
        .expect("a frame");
    assert_eq!(
        held.read(0, 1).expect("a read"),
        vec![0],
        "no pass ran, so nothing was written"
    );
}

#[test]
fn the_allocation_ceiling_is_a_buffer_this_device_will_actually_hand_out() {
    // Not guessable, and not a round fraction of anything: on this machine's two parts it is
    // 3.997 GiB and 2.00 GiB. A caller with more to hold than this holds it in pieces, so the
    // number has to come from the device rather than from a constant somebody typed.
    let Some(host) = device("the_allocation_ceiling_is_a_buffer_this_device_will_hand_out") else {
        return;
    };
    let ceiling = host.ceiling();
    assert!(ceiling > 0, "a device that can allocate nothing");
    assert!(
        ceiling <= u64::from(u32::MAX),
        "a binding cannot address more than a u32 of bytes, and this says {ceiling}"
    );

    // And it is a buffer the device really will give: one word under it, asked for and freed.
    let words = usize::try_from(ceiling / 4).expect("a count");
    match host.resident(&[words], &[]) {
        Ok(held) => drop(held),
        Err(why) => panic!("the device refused its own ceiling of {ceiling}: {why}"),
    }
}

#[test]
fn a_skipped_pass_does_not_take_the_ordering_with_it() {
    // The failure a cheaper command buffer could introduce. A pass told to run nothing is not
    // recorded at all, so neither is the barrier that would have followed it — and if the barriers
    // were counted off the pass list rather than off what actually runs, the two passes either side
    // of a skipped one would go unordered. Both read the word the other wrote, so an unordered pair
    // reads a stale one and the answer comes back short.
    let Some(host) = device("a_skipped_pass_does_not_take_the_ordering_with_it") else {
        return;
    };
    let module = bump(host.width()).expect("a kernel");
    let mut held = host
        .resident(&[4], &[&module, &module, &module, &module, &module])
        .expect("a device buffer");

    // Every pass runs: five bumps of one workgroup, one lane of which stores, so the word is five.
    held.write(0, &[0, 0, 0, 0]).expect("a clear");
    held.frame(&[Groups::Fixed(1); 5]).expect("a frame");
    assert_eq!(held.read(0, 1).expect("a read"), vec![5]);

    // Now with three of them skipped, and the two that run are no longer neighbours in the list.
    held.write(0, &[0, 0, 0, 0]).expect("a clear");
    held.frame(&[
        Groups::Fixed(1),
        Groups::Fixed(0),
        Groups::Fixed(0),
        Groups::Fixed(0),
        Groups::Fixed(1),
    ])
    .expect("a frame");
    assert_eq!(
        held.read(0, 1).expect("a read"),
        vec![2],
        "the second pass did not see what the first wrote"
    );
}

#[test]
fn a_stage_is_ordered_against_the_next_and_not_against_itself() {
    // Six bumps of the same word. As six stages every one sees what the last wrote, so the answer
    // is six. As one stage nothing is ordered — and what that comes back as is the device's
    // business, not this crate's, so what is checked is the shape either side of it rather than a
    // number a racing kernel happened to produce.
    let Some(host) = device("a_stage_is_ordered_against_the_next_and_not_against_itself") else {
        return;
    };
    let module = bump(host.width()).expect("a kernel");
    let every: Vec<&[u32]> = vec![&module; 6];
    let mut held = host.resident(&[4], &every).expect("a device buffer");

    let all = [Groups::Fixed(1); 6];
    held.write(0, &[0, 0, 0, 0]).expect("a clear");
    held.frame(&all).expect("a frame");
    assert_eq!(
        held.read(0, 1).expect("a read"),
        vec![6],
        "six ordered bumps are six"
    );

    // The same six as three stages of two: each stage reads what the stage before it wrote, so the
    // three stages are ordered even though the pairs inside them are not.
    held.write(0, &[0, 0, 0, 0]).expect("a clear");
    let stages: Vec<&[Groups]> = all.chunks(2).collect();
    held.staged(&stages).expect("a staged frame");
    let paired = held.read(0, 1).expect("a read");
    assert!(
        paired[0] >= 3 && paired[0] <= 6,
        "three ordered stages of two unordered bumps came back as {}",
        paired[0]
    );

    // And the stages laid end to end still have to be the passes, or it is refused rather than
    // running some of them.
    let short: Vec<&[Groups]> = all[..4].chunks(2).collect();
    assert!(
        matches!(
            held.staged(&short),
            Err(Error::WrongPasses { given: 4, held: 6 })
        ),
        "four passes were accepted for a frame of six"
    );
}

#[test]
fn a_stage_with_nothing_in_it_orders_nothing() {
    // The card's case: a stage per step, and the steps a given call does not use are given no
    // workgroups. A skipped stage must not leave a barrier behind it, and must not break the
    // ordering of the stages either side.
    let Some(host) = device("a_stage_with_nothing_in_it_orders_nothing") else {
        return;
    };
    let module = bump(host.width()).expect("a kernel");
    let every: Vec<&[u32]> = vec![&module; 4];
    let mut held = host.resident(&[4], &every).expect("a device buffer");

    held.write(0, &[0, 0, 0, 0]).expect("a clear");
    let first = [Groups::Fixed(1)];
    let none = [Groups::Fixed(0), Groups::Fixed(0)];
    let last = [Groups::Fixed(1)];
    held.staged(&[&first, &none, &last])
        .expect("a staged frame");
    assert_eq!(
        held.read(0, 1).expect("a read"),
        vec![2],
        "the two stages that ran did not see each other"
    );
}

/// How many workgroups the pass below writes for the one after it.
///
/// Large enough that reading the seeded one instead is unmistakable, and not a round number so a
/// count that came from somewhere else would not look like this one.
const ASKED: u32 = 37;

/// A kernel that writes a workgroup count into binding 1, for a later pass to be dispatched over.
fn counting(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, 2))?;
    let head = kernel.module().constant_u32(0)?;
    let one = kernel.module().constant_u32(1)?;
    let two = kernel.module().constant_u32(2)?;
    let many = kernel.module().constant_u32(ASKED)?;
    kernel.store_at(1, head, many)?;
    kernel.store_at(1, one, one)?;
    kernel.store_at(1, two, one)?;
    kernel.finish()
}

/// A kernel that adds one to binding 0 from every invocation, so the total says how many ran.
fn counted(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, 2))?;
    let head = kernel.module().constant_u32(0)?;
    let one = kernel.module().constant_u32(1)?;
    let _ = kernel.atomic_add_at(0, head, one)?;
    kernel.finish()
}

/// A count written by one stage is the count the next stage is dispatched over.
///
/// # What this guards, and why nothing guarded it before
///
/// `Resident` records a barrier between stages, and a workgroup count is read at the
/// `DRAW_INDIRECT` stage rather than in a shader — so a barrier naming only `COMPUTE_SHADER` orders
/// the buffer's *contents* and not the command that reads them. That was covered by naming
/// `DRAW_INDIRECT` in every barrier, on the stated grounds that doing so cost nothing.
///
/// **It cost about a fifth of `grit`'s frame.** Six of its ten barriers do not precede an indirect
/// dispatch, and the mask is asked for per stage now. Which makes this the check that the asking is
/// right: the whole optimisation is one `matches!` over the stage about to run, and getting it
/// wrong is not a slow frame but a dispatch over a stale number.
///
/// So one stage writes a count and the next is dispatched over it, with a **different** count seeded
/// underneath. A barrier that failed to order the command would run the seeded one, and the tally
/// says which happened rather than merely whether something ran.
#[test]
fn a_workgroup_count_written_by_one_stage_is_read_by_the_next() {
    let Some(host) = device("a_workgroup_count_written_by_one_stage_is_read_by_the_next") else {
        return;
    };
    let width = host.width();
    let writes = counting(width).expect("a kernel");
    let adds = counted(width).expect("a kernel");
    let modules: Vec<&[u32]> = vec![&writes, &adds];
    let mut held = host.resident(&[1, 3], &modules).expect("a device buffer");

    // **Seeded with a different answer.** One workgroup rather than none, so that a barrier which
    // ordered nothing still runs the pass — and comes back with the wrong number instead of zero,
    // which is the failure worth telling apart.
    held.write(0, &[0]).expect("a tally");
    held.write(1, &[1, 1, 1]).expect("a seeded count");

    // **`Counting`, not `Fixed`, and that is the whole of what this test now says.** The barrier
    // naming `DRAW_INDIRECT` follows the stage that wrote the count, and `Resident` cannot tell
    // which stage that was — a kernel's writes are not in its stage list. Marked `Fixed(1)` here
    // instead, this comes back with 32 invocations against 1184: the dispatch reads the seeded 1
    // rather than the written 37, three runs out of three. Not a race — a wrong answer, every time.
    let first = [Groups::Counting(1)];
    let second = [Groups::From { binding: 1, at: 0 }];
    held.staged(&[&first, &second]).expect("two stages");

    let ran = held
        .read(0, 1)
        .expect("the tally")
        .first()
        .copied()
        .unwrap_or(0);
    let want = ASKED * width;
    assert_eq!(
        ran,
        want,
        "{ran} invocations ran and {want} were asked for — the dispatch read {} workgroups, not \
         {ASKED}",
        ran / width.max(1)
    );
}

/// A frame recorded once is submitted again, and a write in between does not undo that.
///
/// # The two halves, and why they are one test
///
/// `Resident` used to reset and re-record the command buffer on every call, though the comment
/// beside the counter said the quiet part out loud: *the structure does not change between calls;
/// only what the parameters buffer holds does.* At the seventeen passes `pour` runs, that recording
/// was about 7 µs of host time a frame against a frame of 35, measured before it was kept — so it is
/// kept now, keyed on the passes, their grouping and how many frames were asked for.
///
/// Keeping it is only worth anything if it survives what a caller actually does between frames,
/// which is **write a parameter**. A staged write is a recorded copy, and it used to be recorded
/// into the same command buffer — so `grit`'s own loop, which writes four brush words between
/// batches, recorded sixteen times and reused nought. Transfers have a command buffer of their own
/// now.
///
/// And keeping it is only *safe* if a second submission of the same commands sees the second frame's
/// data. It does, because none of what changes is in the recording — but that is an argument, and
/// this is the check. The write between the two frames puts a number there that the first frame
/// never saw, and the answer has to be built on it.
#[test]
fn a_frame_is_recorded_once_and_a_write_between_two_does_not_undo_it() {
    let Some(host) = device("a_frame_is_recorded_once_and_a_write_between_two_does_not_undo_it")
    else {
        return;
    };
    let module = bump(host.width()).expect("a kernel");
    let mut held = host.resident(&[1], &[&module]).expect("a device buffer");
    let groups = [Groups::Fixed(1)];

    held.write(0, &[0]).expect("a clear");
    held.frame(&groups).expect("the first frame");

    // A staged write — this resident is not direct, so it is a recorded copy and a submission.
    held.write(0, &[100]).expect("a parameter");
    held.frame(&groups).expect("the second frame");

    assert_eq!(
        held.read(0, 1).expect("a read"),
        vec![101],
        "the second frame did not see the write, so a kept recording is a stale answer"
    );
    assert_eq!(
        held.reused(),
        1,
        "the second frame was recorded again — a write between two frames is not supposed to cost \
         that, and {} recordings say it did",
        held.recording().1
    );
}

/// A pattern matching nothing names every device that is here.
///
/// # Why the pattern is an argument and not `GRIT_DEVICE`
///
/// Because that variable is process-wide and every test in this binary shares the process, so a test
/// that set it would be choosing a device for the seven beside it. `Host::matching` takes the
/// pattern, which is also what a caller holding two devices at once needs.
///
/// # What it is for
///
/// `Error::NoSuchDevice` carries what *is* present, and that list is how anybody finds out which
/// subgroup widths they can run this crate's tests at. Every kernel here has an arm for 4, 8, 16, 32
/// and 64 and refuses the rest by name; which of those a machine can exercise is a property of the
/// machine, and this is the one call that says so.
///
/// The machine this was written on answers `["NVIDIA GeForce RTX 4080", "AMD Radeon(TM) Graphics"]`
/// — subgroup 32 and 64, and the whole suite passes on both. That is not asserted: a device list is
/// not a thing to pin a test to.
#[test]
fn a_pattern_matching_nothing_names_the_devices_that_are_here() {
    let present = match Host::matching(Some("zzz-no-device-is-called-this")) {
        Err(Error::NoSuchDevice { wanted, present }) => {
            assert_eq!(
                wanted, "zzz-no-device-is-called-this",
                "the pattern is echoed back"
            );
            present
        }
        Ok(None) => {
            eprintln!("SKIPPED a_pattern_matching_nothing: no Vulkan loader");
            return;
        }
        Ok(Some(taken)) => panic!("a pattern matching nothing opened {:?}", taken.name()),
        Err(other) => panic!("a pattern matching nothing gave {other:?}"),
    };

    eprintln!("  devices here: {present:?}");
    assert!(
        !present.is_empty(),
        "the refusal named no devices, so the machine has none and `Ok(None)` was the answer"
    );

    // And a pattern that does match opens one, which is the other half of the same call.
    let first = present.first().cloned().unwrap_or_default();
    let taken = Host::matching(Some(&first)).expect("the device it just named");
    let Some(taken) = taken else {
        panic!("a device named in the refusal could not then be opened");
    };
    assert!(
        taken.name().to_lowercase().contains(&first.to_lowercase()),
        "asked for {first:?} and got {:?}",
        taken.name()
    );
    assert!(
        [4, 8, 16, 32, 64].contains(&taken.width()),
        "{} reports subgroup {}, which no kernel here has an arm for",
        taken.name(),
        taken.width()
    );
}

/// What a resident holds, and what its transfers cost.
///
/// **A coverage run found these four reachable from outside and called by nothing**, which reading
/// the suite did not: `bindings`, `passes`, `pushing` and `copying` each return a field, and a
/// getter naming the wrong one is a plausible bug that no amount of frame timing would show.
#[test]
fn a_resident_says_what_it_holds_and_what_its_transfers_cost() {
    let Some(host) = device("counters") else {
        return;
    };
    let width = host.width();
    let module = bump(width).expect("a kernel");
    let mut held = host
        .resident(&[4, 8], &[&module, &module])
        .expect("a device buffer");

    assert_eq!(held.bindings(), 2, "two sizes were asked for");
    assert_eq!(held.passes(), 2, "two modules were asked for");
    assert_eq!(
        held.pushing(),
        (Duration::ZERO, 0, 0),
        "nothing has been written and the counter says otherwise"
    );
    assert_eq!(
        held.copying(),
        (Duration::ZERO, 0),
        "nothing has been read and the counter says otherwise"
    );

    held.write(0, &[7]).expect("a write");
    let (spent, writes, straight) = held.pushing();
    assert_eq!(writes, 1, "one write went in and the counter says {writes}");
    assert!(spent > Duration::ZERO, "a write that took no time at all");
    assert!(
        straight <= writes,
        "{straight} writes went straight into device memory out of {writes} that went in at all"
    );

    assert_eq!(held.read(0, 1).expect("a read"), vec![7]);
    let (spent, reads) = held.copying();
    assert_eq!(reads, 1, "one read came back and the counter says {reads}");
    assert!(spent > Duration::ZERO, "a read that took no time at all");
}

/// [`Resident::frames`] runs the passes that many times in one submission.
///
/// The kernel adds one to a word, so the word afterwards **is** the frame count. That is the whole
/// check: a submission that ran the passes once and returned a fifth of the time would pass a
/// timing assertion and fail this one.
#[test]
fn a_frame_asked_for_five_times_runs_five_times() {
    let Some(host) = device("frames") else {
        return;
    };
    let width = host.width();
    let module = bump(width).expect("a kernel");
    let mut held = host.resident(&[4], &[&module]).expect("a device buffer");
    held.write(0, &[0]).expect("a write");

    let each = held.frames(&[Groups::Fixed(1)], 5).expect("five frames");
    assert_eq!(
        held.read(0, 1).expect("a read"),
        vec![5],
        "five frames of a kernel that adds one did not add five"
    );
    assert!(each > Duration::ZERO, "five frames took no time at all");
}

/// [`Resident::write_many`] puts scattered runs in through one submission — or none of them.
///
/// **The second half is the point.** Every run is checked before anything is copied, so a call
/// holding one bad run leaves the staging buffer alone and the caller certain nothing landed. A
/// version that copied as it went would pass the first half of this test and fail the last line.
#[test]
fn many_runs_go_in_together_and_a_refused_call_puts_nothing_in() {
    let Some(host) = device("write_many") else {
        return;
    };
    let width = host.width();
    let module = bump(width).expect("a kernel");
    let mut held = host.resident(&[4, 4], &[&module]).expect("a device buffer");

    held.write_many(&[
        Run {
            index: 0,
            at: 0,
            words: &[11],
        },
        Run {
            index: 1,
            at: 1,
            words: &[22],
        },
    ])
    .expect("two runs");
    assert_eq!(held.read(0, 1).expect("a read"), vec![11]);
    assert_eq!(
        held.read(1, 2).expect("a read"),
        vec![0, 22],
        "the second run went in at the wrong offset"
    );

    // A run past the end of its binding, with a good one beside it.
    let refused = held.write_many(&[
        Run {
            index: 0,
            at: 0,
            words: &[99],
        },
        Run {
            index: 1,
            at: 100,
            words: &[99],
        },
    ]);
    match refused {
        Err(Error::Overrun { index, .. }) => {
            assert_eq!(index, 1, "the wrong binding was named as overrun");
        }
        other => panic!("a run past the end of a binding was not refused: {other:?}"),
    }
    assert_eq!(
        held.read(0, 1).expect("a read"),
        vec![11],
        "the good run in a refused call landed anyway, so the check is not before the copy"
    );
}
