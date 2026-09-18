#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

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
            eprintln!("SKIPPED {label}");
            None
        }
        Err(why) => {
            eprintln!("SKIPPED {label} {why}");
            None
        }
    }
}

#[test]
fn a_pass_dispatched_over_no_workgroups_does_not_run() {
    let Some(host) = device("a_pass_dispatched_over_no_workgroups_does_not_run") else {
        return;
    };
    let module = mark(host.width()).unwrap();
    let mut held = host.resident(&[4], &[&module]).unwrap();
    held.write(0, &[0, 0, 0, 0]).unwrap();

    held.frame(&[Groups::Fixed(0)]).unwrap();
    assert_eq!(held.read(0, 1).unwrap(), vec![0]);

    held.frame(&[Groups::Fixed(1)]).unwrap();
    assert_eq!(held.read(0, 1).unwrap(), vec![MARK]);
}

#[test]
fn a_frame_can_be_taken_apart_stage_by_stage() {
    let Some(host) = device("a_frame_can_be_taken_apart_stage_by_stage") else {
        return;
    };
    let width = host.width();
    let one = mark(width).unwrap();
    let two = bump(width).unwrap();
    let modules: Vec<&[u32]> = vec![&one, &one, &one, &two];
    let mut held = host.resident(&[4], &modules).unwrap();
    held.write(0, &[0, 0, 0, 0]).unwrap();

    let first = [Groups::Fixed(1)];
    let second = [Groups::Fixed(1), Groups::Fixed(1)];
    let third = [Groups::Fixed(1)];
    held.staged(&[&first, &second, &third]).unwrap();

    let (whole, spans) = (held.on_device(), held.stage_times().to_vec());
    let Some(whole) = whole else {
        assert!(spans.is_empty());
        return;
    };
    eprintln!("{whole:.3?}");
    for (nth, span) in spans.iter().enumerate() {
        eprintln!("{nth} {span:.3?}");
    }

    assert_eq!(spans.len(), 3);
    let parts: Duration = spans.iter().sum();
    assert!(parts <= whole);
    assert!(spans.iter().all(|one| *one > Duration::ZERO));

    assert_eq!(held.read(0, 1).unwrap(), vec![MARK + 1]);
}

#[test]
fn a_frame_at_the_last_stage_that_profiles_is_timed_all_the_way_down() {
    let Some(host) = device("a_frame_at_the_last_stage_that_profiles_is_timed_all_the_way_down")
    else {
        return;
    };
    let width = host.width();
    let module = bump(width).unwrap();
    let many = grit::resident::STAGES;
    let modules: Vec<&[u32]> = vec![&module; many];
    let mut held = host.resident(&[4], &modules).unwrap();
    held.write(0, &[0, 0, 0, 0]).unwrap();

    let each = vec![vec![Groups::Fixed(1)]; many];
    let stages: Vec<&[Groups]> = each.iter().map(Vec::as_slice).collect();
    held.staged(&stages).unwrap();

    assert_eq!(held.read(0, 1).unwrap(), vec![many as u32]);
    if held.on_device().is_none() {
        assert!(held.stage_times().is_empty());
        return;
    }
    assert_eq!(held.stage_times().len(), many);
}

#[test]
fn a_frame_too_long_to_profile_is_not_half_profiled() {
    let Some(host) = device("a_frame_too_long_to_profile_is_not_half_profiled") else {
        return;
    };
    let width = host.width();
    let module = bump(width).unwrap();
    let many = grit::resident::STAGES + 1;
    let modules: Vec<&[u32]> = vec![&module; many];
    let mut held = host.resident(&[4], &modules).unwrap();
    held.write(0, &[0, 0, 0, 0]).unwrap();

    let each = vec![vec![Groups::Fixed(1)]; many];
    let stages: Vec<&[Groups]> = each.iter().map(Vec::as_slice).collect();
    held.staged(&stages).unwrap();

    assert_eq!(held.read(0, 1).unwrap(), vec![many as u32]);
    assert!(held.stage_times().is_empty());
}

#[test]
fn a_count_of_nought_in_a_buffer_stops_the_pass_that_reads_it() {
    let Some(host) = device("a_count_of_nought_in_a_buffer_stops_the_pass_that_reads_it") else {
        return;
    };
    let module = mark(host.width()).unwrap();
    let mut held = host.resident(&[4, 3], &[&module]).unwrap();
    let over = [Groups::From { binding: 1, at: 0 }];

    held.write(0, &[0, 0, 0, 0]).unwrap();
    held.write(1, &[0, 1, 1]).unwrap();
    held.frame(&over).unwrap();
    assert_eq!(held.read(0, 1).unwrap(), vec![0]);

    held.write(1, &[1, 1, 1]).unwrap();
    held.frame(&over).unwrap();
    assert_eq!(held.read(0, 1).unwrap(), vec![MARK]);
}

#[test]
fn one_pass_of_several_can_be_run_while_the_others_are_skipped() {
    let Some(host) = device("one_pass_of_several_can_be_run_while_the_others_are_skipped") else {
        return;
    };
    let module = mark(host.width()).unwrap();
    let mut held = host.resident(&[4], &[&module, &module, &module]).unwrap();
    held.write(0, &[0, 0, 0, 0]).unwrap();

    held.frame(&[Groups::Fixed(0), Groups::Fixed(1), Groups::Fixed(0)])
        .unwrap();
    assert_eq!(held.read(0, 1).unwrap(), vec![MARK]);

    held.write(0, &[0, 0, 0, 0]).unwrap();
    held.frame(&[Groups::Fixed(0), Groups::Fixed(0), Groups::Fixed(0)])
        .unwrap();
    assert_eq!(held.read(0, 1).unwrap(), vec![0]);
}

#[test]
fn the_allocation_ceiling_is_a_buffer_this_device_will_actually_hand_out() {
    let Some(host) = device("the_allocation_ceiling_is_a_buffer_this_device_will_hand_out") else {
        return;
    };
    let ceiling = host.ceiling();
    assert!(ceiling > 0);
    assert!(ceiling <= u64::from(u32::MAX));

    let words = usize::try_from(ceiling / 4).unwrap();
    match host.resident(&[words], &[]) {
        Ok(held) => drop(held),
        Err(why) => panic!("{ceiling} {why}"),
    }
}

#[test]
fn a_skipped_pass_does_not_take_the_ordering_with_it() {
    let Some(host) = device("a_skipped_pass_does_not_take_the_ordering_with_it") else {
        return;
    };
    let module = bump(host.width()).unwrap();
    let mut held = host
        .resident(&[4], &[&module, &module, &module, &module, &module])
        .unwrap();

    held.write(0, &[0, 0, 0, 0]).unwrap();
    held.frame(&[Groups::Fixed(1); 5]).unwrap();
    assert_eq!(held.read(0, 1).unwrap(), vec![5]);

    held.write(0, &[0, 0, 0, 0]).unwrap();
    held.frame(&[
        Groups::Fixed(1),
        Groups::Fixed(0),
        Groups::Fixed(0),
        Groups::Fixed(0),
        Groups::Fixed(1),
    ])
    .unwrap();
    assert_eq!(held.read(0, 1).unwrap(), vec![2]);
}

#[test]
fn a_stage_is_ordered_against_the_next_and_not_against_itself() {
    let Some(host) = device("a_stage_is_ordered_against_the_next_and_not_against_itself") else {
        return;
    };
    let module = bump(host.width()).unwrap();
    let every: Vec<&[u32]> = vec![&module; 6];
    let mut held = host.resident(&[4], &every).unwrap();

    let all = [Groups::Fixed(1); 6];
    held.write(0, &[0, 0, 0, 0]).unwrap();
    held.frame(&all).unwrap();
    assert_eq!(held.read(0, 1).unwrap(), vec![6]);

    held.write(0, &[0, 0, 0, 0]).unwrap();
    let stages: Vec<&[Groups]> = all.chunks(2).collect();
    held.staged(&stages).unwrap();
    let paired = held.read(0, 1).unwrap();
    assert!(paired[0] >= 3 && paired[0] <= 6);

    let short: Vec<&[Groups]> = all[..4].chunks(2).collect();
    assert!(matches!(
        held.staged(&short),
        Err(Error::WrongPasses { given: 4, held: 6 })
    ));
}

#[test]
fn a_stage_with_nothing_in_it_orders_nothing() {
    let Some(host) = device("a_stage_with_nothing_in_it_orders_nothing") else {
        return;
    };
    let module = bump(host.width()).unwrap();
    let every: Vec<&[u32]> = vec![&module; 4];
    let mut held = host.resident(&[4], &every).unwrap();

    held.write(0, &[0, 0, 0, 0]).unwrap();
    let first = [Groups::Fixed(1)];
    let none = [Groups::Fixed(0), Groups::Fixed(0)];
    let last = [Groups::Fixed(1)];
    held.staged(&[&first, &none, &last]).unwrap();
    assert_eq!(held.read(0, 1).unwrap(), vec![2]);
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
    let writes = counting(width).unwrap();
    let adds = counted(width).unwrap();
    let modules: Vec<&[u32]> = vec![&writes, &adds];
    let mut held = host.resident(&[1, 3], &modules).unwrap();

    held.write(0, &[0]).unwrap();
    held.write(1, &[1, 1, 1]).unwrap();

    let first = [Groups::Counting(1)];
    let second = [Groups::From { binding: 1, at: 0 }];
    held.staged(&[&first, &second]).unwrap();

    let ran = held.read(0, 1).unwrap().first().copied().unwrap_or(0);
    let want = ASKED * width;
    assert_eq!(ran, want);
}

#[test]
fn a_frame_is_recorded_once_and_a_write_between_two_does_not_undo_it() {
    let Some(host) = device("a_frame_is_recorded_once_and_a_write_between_two_does_not_undo_it")
    else {
        return;
    };
    let module = bump(host.width()).unwrap();
    let mut held = host.resident(&[1], &[&module]).unwrap();
    let groups = [Groups::Fixed(1)];

    held.write(0, &[0]).unwrap();
    held.frame(&groups).unwrap();

    held.write(0, &[100]).unwrap();
    held.frame(&groups).unwrap();

    assert_eq!(held.read(0, 1).unwrap(), vec![101]);
    assert_eq!(held.reused(), 1);
}

#[test]
fn a_pattern_matching_nothing_names_the_devices_that_are_here() {
    let present = match Host::matching(Some("zzz-no-device-is-called-this")) {
        Err(Error::NoSuchDevice { wanted, present }) => {
            assert_eq!(wanted, "zzz-no-device-is-called-this");
            present
        }
        Ok(None) => {
            eprintln!("SKIPPED");
            return;
        }
        Ok(Some(taken)) => panic!("{:?}", taken.name()),
        Err(other) => panic!("{other:?}"),
    };

    eprintln!("{present:?}");
    assert!(!present.is_empty());

    let first = present.first().cloned().unwrap_or_default();
    let taken = Host::matching(Some(&first)).unwrap();
    let Some(taken) = taken else {
        panic!();
    };
    assert!(taken.name().to_lowercase().contains(&first.to_lowercase()));
    assert!([4, 8, 16, 32, 64].contains(&taken.width()));
}

#[test]
fn a_resident_says_what_it_holds_and_what_its_transfers_cost() {
    let Some(host) = device("counters") else {
        return;
    };
    let width = host.width();
    let module = bump(width).unwrap();
    let mut held = host.resident(&[4, 8], &[&module, &module]).unwrap();

    assert_eq!(held.bindings(), 2);
    assert_eq!(held.passes(), 2);
    assert_eq!(held.pushing(), (Duration::ZERO, 0, 0));
    assert_eq!(held.copying(), (Duration::ZERO, 0));

    held.write(0, &[7]).unwrap();
    let (spent, writes, straight) = held.pushing();
    assert_eq!((writes, straight), (1, 0));
    assert!(spent > Duration::ZERO);

    assert!(matches!(
        held.write(0, &[1, 2, 3, 4, 5]),
        Err(Error::Overrun { .. })
    ));
    let (_, writes, straight) = held.pushing();
    assert_eq!((writes, straight), (2, 0));

    assert_eq!(held.read(0, 1).unwrap(), vec![7]);
    let (spent, reads) = held.copying();
    assert_eq!(reads, 1);
    assert!(spent > Duration::ZERO);
}

#[test]
fn a_frame_asked_for_five_times_runs_five_times() {
    let Some(host) = device("frames") else {
        return;
    };
    let width = host.width();
    let module = bump(width).unwrap();
    let mut held = host.resident(&[4], &[&module]).unwrap();
    held.write(0, &[0]).unwrap();

    let each = held.frames(&[Groups::Fixed(1)], 5).unwrap();
    assert_eq!(held.read(0, 1).unwrap(), vec![5]);
    assert!(each > Duration::ZERO);
}

#[test]
fn many_runs_go_in_together_and_a_refused_call_puts_nothing_in() {
    let Some(host) = device("write_many") else {
        return;
    };
    let width = host.width();
    let module = bump(width).unwrap();
    let mut held = host.resident(&[4, 4], &[&module]).unwrap();

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
    .unwrap();
    assert_eq!(held.read(0, 1).unwrap(), vec![11]);
    assert_eq!(held.read(1, 2).unwrap(), vec![0, 22]);

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
            assert_eq!(index, 1);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(held.read(0, 1).unwrap(), vec![11]);
}

#[test]
fn a_buffer_small_enough_to_live_near_the_host_is_written_straight_into() {
    let Some(host) = device("a_buffer_small_enough_to_live_near_the_host") else {
        return;
    };
    let width = host.width();
    let module = bump(width).unwrap();
    let mut held = host
        .resident_near(&[4, 8], &[&module, &module], 64)
        .unwrap();

    held.write(0, &[1, 2, 3, 4]).unwrap();
    let (_, writes, straight) = held.pushing();
    eprintln!("{writes} {straight}");
    assert_eq!((writes, straight), (1, 1));
    assert_eq!(held.read(0, 4).unwrap(), vec![1, 2, 3, 4]);
}

#[test]
fn an_indirect_count_is_read_from_the_word_it_points_at() {
    let Some(host) = device("an_indirect_count_is_read_from_the_word_it_points_at") else {
        return;
    };
    let module = bump(host.width()).unwrap();
    let mut held = host.resident(&[4, 6], &[&module]).unwrap();

    held.write(0, &[0, 0, 0, 0]).unwrap();
    held.write(1, &[0, 1, 1, 1, 1, 1]).unwrap();

    held.frame(&[Groups::From { binding: 1, at: 0 }]).unwrap();
    assert_eq!(held.read(0, 1).unwrap(), vec![0]);

    held.frame(&[Groups::From { binding: 1, at: 3 }]).unwrap();
    assert_eq!(held.read(0, 1).unwrap(), vec![1]);
}

#[test]
fn a_stage_with_nothing_left_in_it_is_not_a_stage_that_is_timed() {
    let Some(host) = device("a_stage_with_nothing_left_in_it_is_not_timed") else {
        return;
    };
    let width = host.width();
    let module = bump(width).unwrap();
    let modules: Vec<&[u32]> = vec![&module, &module, &module];
    let mut held = host.resident(&[4], &modules).unwrap();
    held.write(0, &[0, 0, 0, 0]).unwrap();

    let first = [Groups::Fixed(1)];
    let middle = [Groups::Fixed(0)];
    let last = [Groups::Fixed(1)];
    held.staged(&[&first, &middle, &last]).unwrap();

    assert_eq!(held.read(0, 1).unwrap(), vec![2]);
    if held.on_device().is_none() {
        assert!(held.stage_times().is_empty());
        return;
    }
    assert_eq!(held.stage_times().len(), 2);
}

#[test]
fn a_buffer_exactly_the_size_of_the_window_still_lives_near_the_host() {
    let Some(host) = device("a_buffer_exactly_the_size_of_the_window_lives_near") else {
        return;
    };
    let width = host.width();
    let module = bump(width).unwrap();
    let window = (4 * size_of::<u32>()) as u64;
    let mut held = host
        .resident_near(&[4, 4], &[&module, &module], window)
        .unwrap();

    held.write(0, &[1, 2, 3, 4]).unwrap();
    let (_, writes, straight) = held.pushing();
    eprintln!("{writes} {straight}");
    assert_eq!((writes, straight), (1, 1));
    assert_eq!(held.read(0, 4).unwrap(), vec![1, 2, 3, 4]);
}

#[test]
fn a_write_into_a_near_buffer_does_not_wait_for_the_frame_in_flight() {
    let Some(host) = device("a_write_into_a_near_buffer_does_not_wait") else {
        return;
    };
    let width = host.width();
    let module = bump(width).unwrap();
    let window = 64_u64;
    let mut held = host.resident_near(&[4, 4], &[&module], window).unwrap();
    held.write(0, &[0, 0, 0, 0]).unwrap();
    held.write(1, &[0, 0, 0, 0]).unwrap();
    let (_, _, straight) = held.pushing();
    if straight == 0 {
        eprintln!("SKIPPED {}", host.name());
        return;
    }

    let groups = [Groups::Fixed(1)];
    let stages: Vec<&[Groups]> = vec![&groups];
    let ticket = held.submit(&stages).unwrap();
    held.write_at(1, 2, &[MARK]).unwrap();
    held.wait(ticket).unwrap();

    assert_eq!(held.read(1, 4).unwrap(), vec![0, 0, MARK, 0]);
    assert_eq!(held.read(0, 1).unwrap(), vec![1]);
}

#[test]
fn runs_that_fill_the_staging_buffer_to_the_word_are_taken() {
    let Some(host) = device("runs_that_fill_the_staging_buffer_to_the_word") else {
        return;
    };
    let width = host.width();
    let module = bump(width).unwrap();
    let mut held = host.resident(&[4, 4], &[&module]).unwrap();
    held.write(0, &[0, 0, 0, 0]).unwrap();
    held.write(1, &[0, 0, 0, 0]).unwrap();

    let first = [MARK, MARK];
    let second = [7, 9];
    let runs = [
        Run {
            index: 0,
            at: 0,
            words: &first,
        },
        Run {
            index: 1,
            at: 2,
            words: &second,
        },
    ];
    held.write_many(&runs).unwrap();
    assert_eq!(held.read(0, 4).unwrap(), vec![MARK, MARK, 0, 0]);
    assert_eq!(held.read(1, 4).unwrap(), vec![0, 0, 7, 9]);
}

#[test]
fn the_time_a_repeated_frame_reports_is_the_time_of_one_of_them() {
    let Some(host) = device("the_time_a_repeated_frame_reports_is_the_time_of_one") else {
        return;
    };
    let width = host.width();
    let module = bump(width).unwrap();
    let mut held = host.resident(&[4], &[&module]).unwrap();
    held.write(0, &[0, 0, 0, 0]).unwrap();

    let groups = [Groups::Fixed(1)];
    let once = held.frames(&groups, 1).unwrap();
    let many = held.frames(&groups, 16).unwrap();
    eprintln!("{once:?} {many:?}");

    assert_eq!(held.read(0, 1).unwrap(), vec![17]);
    assert!(many < once * 8);
}
