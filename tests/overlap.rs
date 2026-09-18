#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use std::time::Duration;

use grit::resident::SLOTS;
use grit::{Error, Groups, Host};
use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, U32};

const WORDS: usize = 8 << 20;

const PASSES: usize = 16;

const PER_LANE: u32 = 64;

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

fn over(width: u32) -> u32 {
    u32::try_from(WORDS / (width as usize * PER_LANE as usize)).unwrap_or(1)
}

#[test]
fn a_frame_waited_for_later_is_the_frame_that_was_submitted() {
    let Some(host) = device("a_frame_waited_for_later_is_the_frame_that_was_submitted") else {
        return;
    };
    let module = walk(host.width()).unwrap();
    let every: Vec<&[u32]> = vec![&module; PASSES];
    let mut held = host.resident(&[WORDS, 1], &every).unwrap();
    held.write(0, &vec![3; WORDS]).unwrap();

    let groups = vec![Groups::Fixed(over(host.width())); PASSES];
    let stages: Vec<&[Groups]> = groups.iter().map(core::slice::from_ref).collect();

    held.write(1, &[0]).unwrap();
    held.frame(&groups).unwrap();
    let blocking = held.read(1, 1).unwrap();
    assert_eq!(blocking, vec![(3 * WORDS * PASSES) as u32]);

    held.write(1, &[0]).unwrap();
    let ticket = held.submit(&stages).unwrap();
    let mine: u64 = (0..1_000_000_u64).sum();
    assert_eq!(mine, 499_999_500_000);
    held.wait(ticket).unwrap();

    assert_eq!(held.read(1, 1).unwrap(), blocking);
}

#[test]
fn a_read_back_started_early_is_read_after_the_fence_and_not_before() {
    let Some(host) = device("a_read_back_started_early_is_read_after_the_fence") else {
        return;
    };
    let module = walk(host.width()).unwrap();
    let every: Vec<&[u32]> = vec![&module; PASSES];
    let mut held = host.resident(&[WORDS, 1], &every).unwrap();
    let groups = vec![Groups::Fixed(over(host.width())); PASSES];
    let stages: Vec<&[Groups]> = groups.iter().map(core::slice::from_ref).collect();

    for fill in [2_u32, 5] {
        held.write(0, &vec![fill; WORDS]).unwrap();
        held.write(1, &[0]).unwrap();

        let ticket = held.submit_reading(&stages, 1, 1).unwrap();
        let (_, words) = held.wait_reading(ticket).unwrap();
        assert_eq!(words, vec![(fill as usize * WORDS * PASSES) as u32]);
    }
}

#[test]
fn a_submission_past_the_slots_in_flight_is_refused() {
    let Some(host) = device("a_submission_past_the_slots_in_flight_is_refused") else {
        return;
    };
    let module = walk(host.width()).unwrap();
    let every: Vec<&[u32]> = vec![&module; PASSES];
    let mut held = host.resident(&[WORDS, 1], &every).unwrap();
    let groups = vec![Groups::Fixed(over(host.width())); PASSES];
    let stages: Vec<&[Groups]> = groups.iter().map(core::slice::from_ref).collect();

    let first = held.submit(&stages).unwrap();
    assert_eq!(held.in_flight(), 1);
    let second = held.submit(&stages).unwrap();
    assert_eq!(held.in_flight(), SLOTS);
    assert!(matches!(held.submit(&stages), Err(Error::StillInFlight)));
    assert!(matches!(held.frame(&groups), Err(Error::StillInFlight)));

    held.wait(first).unwrap();
    assert_eq!(held.in_flight(), SLOTS - 1);
    assert!(matches!(held.wait(first), Err(Error::NoSuchTicket { .. })));
    held.wait(second).unwrap();
    assert_eq!(held.in_flight(), 0);
    held.frame(&groups).unwrap();
}

#[test]
fn a_frame_submitted_while_another_flies_comes_back_on_its_own_ticket() {
    let Some(host) = device("a_frame_submitted_while_another_flies_comes_back_on_its_own_ticket")
    else {
        return;
    };
    let module = walk(host.width()).unwrap();
    let every: Vec<&[u32]> = vec![&module; PASSES];
    let mut held = host.resident(&[WORDS, 1], &every).unwrap();
    let groups = vec![Groups::Fixed(over(host.width())); PASSES];
    let stages: Vec<&[Groups]> = groups.iter().map(core::slice::from_ref).collect();

    held.write(0, &vec![1; WORDS]).unwrap();
    held.write(1, &[0]).unwrap();

    let mut waiting = held.submit_reading(&stages, 1, 1).unwrap();
    let mut seen = Vec::new();
    for _ in 0..4 {
        let next = held.submit_reading(&stages, 1, 1).unwrap();
        let (_, words) = held.wait_reading(waiting).unwrap();
        seen.push(words.first().copied().unwrap_or(0));
        waiting = next;
    }
    let (_, words) = held.wait_reading(waiting).unwrap();
    seen.push(words.first().copied().unwrap_or(0));

    let counted: Vec<u32> = (1..=seen.len() as u32)
        .map(|frame| frame * (WORDS * PASSES) as u32)
        .collect();
    assert_eq!(seen, counted);
}

const FIRST: u32 = 0x0A11_0000;
const SECOND: u32 = 0x0B22_0000;

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

#[test]
fn two_answers_come_back_from_one_submission_in_the_order_they_were_asked_for() {
    let Some(host) = device("two_answers_come_back_from_one_submission") else {
        return;
    };
    let module = both(host.width()).unwrap();
    let mut held = host.resident(&[2, 2], &[&module]).unwrap();
    held.write(0, &[FIRST, 0]).unwrap();
    held.write(1, &[SECOND, 0]).unwrap();

    let groups = [Groups::Fixed(1)];
    let stages: Vec<&[Groups]> = vec![&groups];

    let ticket = held.submit_reading_both(&stages, (0, 1), (1, 1)).unwrap();
    let (_, first, second) = held.wait_reading_both(ticket).unwrap();

    assert_eq!(
        (first.first().copied(), second.first().copied()),
        (Some(FIRST + 1), Some(SECOND + 1))
    );

    held.write(0, &[FIRST, 0]).unwrap();
    held.write(1, &[SECOND, 0]).unwrap();
    let (spent, first, second) = held.staged_reading_both(&stages, (0, 1), (1, 1)).unwrap();
    assert_eq!(
        (first.first().copied(), second.first().copied()),
        (Some(FIRST + 1), Some(SECOND + 1))
    );
    assert!(spent > Duration::ZERO);

    let ticket = held.submit_reading(&stages, 0, 1).unwrap();
    assert!(matches!(
        held.wait_reading_both(ticket),
        Err(Error::WrongOutputs {
            asked: 1,
            wanted: 2
        })
    ));

    assert!(matches!(
        held.wait(ticket),
        Err(Error::NoSuchTicket { held: None, .. })
    ));
}
