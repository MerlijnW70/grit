#![allow(clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::time::Duration;

use grit::{Error, Groups, Host, Run};
use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, U32};

const MARK: u32 = 0x5EED_BEEF;

fn mark(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, 1))?;
    let head = kernel.module().constant_u32(0)?;
    let value = kernel.module().constant_u32(MARK)?;
    kernel.store_at(0, head, value)?;
    kernel.finish()
}

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

    held.frame(&[Groups::Fixed(1)]).expect("a frame");
    assert_eq!(held.read(0, 1).expect("a read"), vec![MARK]);
}

#[test]
fn a_frame_can_be_taken_apart_stage_by_stage() {
    let Some(host) = device("a_frame_can_be_taken_apart_stage_by_stage") else {
        return;
    };
    let width = host.width();
    let one = mark(width).expect("a kernel");
    let two = bump(width).expect("a kernel");
    let modules: Vec<&[u32]> = vec![&one, &one, &one, &two];
    let mut held = host.resident(&[4], &modules).expect("a device buffer");
    held.write(0, &[0, 0, 0, 0]).expect("a clear");

    let first = [Groups::Fixed(1)];
    let second = [Groups::Fixed(1), Groups::Fixed(1)];
    let third = [Groups::Fixed(1)];
    held.staged(&[&first, &second, &third])
        .expect("three stages");

    let (whole, spans) = (held.on_device(), held.stage_times().to_vec());
    let Some(whole) = whole else {
        eprintln!("  this queue counts no timestamps; nothing to take apart");
        assert!(
            spans.is_empty(),
            "no whole time but {} stage times",
            spans.len()
        );
        return;
    };
    eprintln!("  the whole submission: {whole:.3?}");
    for (nth, span) in spans.iter().enumerate() {
        eprintln!("    stage {nth}: {span:.3?}");
    }

    assert_eq!(
        spans.len(),
        3,
        "three stages ran and the profile has {}",
        spans.len()
    );
    let parts: Duration = spans.iter().sum();
    assert!(
        parts <= whole,
        "the stages come to {parts:.3?} and the whole submission was {whole:.3?}"
    );
    assert!(
        spans.iter().all(|one| *one > Duration::ZERO),
        "a stage that took no time at all did not run: {spans:.3?}"
    );

    assert_eq!(held.read(0, 1).expect("a read"), vec![MARK + 1]);
}

#[test]
fn a_frame_too_long_to_profile_is_not_half_profiled() {
    let Some(host) = device("a_frame_too_long_to_profile_is_not_half_profiled") else {
        return;
    };
    let width = host.width();
    let module = bump(width).expect("a kernel");
    let many = grit::resident::STAGES + 1;
    let modules: Vec<&[u32]> = vec![&module; many];
    let mut held = host.resident(&[4], &modules).expect("a device buffer");
    held.write(0, &[0, 0, 0, 0]).expect("a clear");

    let each = vec![vec![Groups::Fixed(1)]; many];
    let stages: Vec<&[Groups]> = each.iter().map(Vec::as_slice).collect();
    held.staged(&stages).expect("a long frame");

    assert_eq!(
        held.read(0, 1).expect("a read"),
        vec![many as u32],
        "the frame did not run all {many} of its passes"
    );
    assert!(
        held.stage_times().is_empty(),
        "{} stages were profiled and the pool holds {}",
        held.stage_times().len(),
        grit::resident::STAGES
    );
}

#[test]
fn a_count_of_nought_in_a_buffer_stops_the_pass_that_reads_it() {
    let Some(host) = device("a_count_of_nought_in_a_buffer_stops_the_pass_that_reads_it") else {
        return;
    };
    let module = mark(host.width()).expect("a kernel");
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
    let Some(host) = device("the_allocation_ceiling_is_a_buffer_this_device_will_hand_out") else {
        return;
    };
    let ceiling = host.ceiling();
    assert!(ceiling > 0, "a device that can allocate nothing");
    assert!(
        ceiling <= u64::from(u32::MAX),
        "a binding cannot address more than a u32 of bytes, and this says {ceiling}"
    );

    let words = usize::try_from(ceiling / 4).expect("a count");
    match host.resident(&[words], &[]) {
        Ok(held) => drop(held),
        Err(why) => panic!("the device refused its own ceiling of {ceiling}: {why}"),
    }
}

#[test]
fn a_skipped_pass_does_not_take_the_ordering_with_it() {
    let Some(host) = device("a_skipped_pass_does_not_take_the_ordering_with_it") else {
        return;
    };
    let module = bump(host.width()).expect("a kernel");
    let mut held = host
        .resident(&[4], &[&module, &module, &module, &module, &module])
        .expect("a device buffer");

    held.write(0, &[0, 0, 0, 0]).expect("a clear");
    held.frame(&[Groups::Fixed(1); 5]).expect("a frame");
    assert_eq!(held.read(0, 1).expect("a read"), vec![5]);

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

    held.write(0, &[0, 0, 0, 0]).expect("a clear");
    let stages: Vec<&[Groups]> = all.chunks(2).collect();
    held.staged(&stages).expect("a staged frame");
    let paired = held.read(0, 1).expect("a read");
    assert!(
        paired[0] >= 3 && paired[0] <= 6,
        "three ordered stages of two unordered bumps came back as {}",
        paired[0]
    );

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

const ASKED: u32 = 37;

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

fn counted(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, 2))?;
    let head = kernel.module().constant_u32(0)?;
    let one = kernel.module().constant_u32(1)?;
    let _ = kernel.atomic_add_at(0, head, one)?;
    kernel.finish()
}

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

    held.write(0, &[0]).expect("a tally");
    held.write(1, &[1, 1, 1]).expect("a seeded count");

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
