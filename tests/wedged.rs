#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use std::time::Duration;

use grit::{Error, Groups, Host, Resident};
use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, U32};

const WORDS: usize = 1 << 16;

const PER_LANE: u32 = 4;

const IMPATIENT: Duration = Duration::ZERO;

const TRIES: usize = 8;

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

fn wedge(held: &mut Resident<'_>, groups: &[Groups]) -> Option<Duration> {
    for _ in 0..TRIES {
        match held.frame(groups) {
            Err(Error::Wedged { after }) => return Some(after),
            Err(other) => panic!("{other:?}"),
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
    let module = walk(host.width()).unwrap();
    let every: Vec<&[u32]> = vec![&module];
    let mut held = host.resident(&[WORDS, 1], &every).unwrap();
    held.write(0, &vec![1; WORDS]).unwrap();

    assert_eq!(held.patience(), Duration::from_secs(10));

    held.set_patience(IMPATIENT);
    let groups = [Groups::Fixed(over(host.width()))];

    let after = wedge(&mut held, &groups).unwrap();
    assert_eq!(after, IMPATIENT);
}

#[test]
fn a_wedged_device_refuses_everything_afterwards() {
    let Some(host) = device("a_wedged_device_refuses_everything_afterwards") else {
        return;
    };
    let module = walk(host.width()).unwrap();
    let every: Vec<&[u32]> = vec![&module];
    let mut held = host.resident(&[WORDS, 1], &every).unwrap();
    held.write(0, &vec![1; WORDS]).unwrap();

    held.set_patience(IMPATIENT);
    let groups = [Groups::Fixed(over(host.width()))];
    assert!(wedge(&mut held, &groups).is_some());

    assert!(matches!(held.frame(&groups), Err(Error::Wedged { .. })));
    assert!(matches!(held.write(0, &[7]), Err(Error::Wedged { .. })));
    assert!(matches!(
        held.write_at(0, 1, &[7]),
        Err(Error::Wedged { .. })
    ));
    assert!(matches!(held.read(1, 1), Err(Error::Wedged { .. })));
    assert!(matches!(held.submit(&[&groups]), Err(Error::Wedged { .. })));
}

#[test]
fn a_wedged_device_is_dropped_rather_than_waited_for() {
    let Some(host) = device("a_wedged_device_is_dropped_rather_than_waited_for") else {
        return;
    };
    let module = walk(host.width()).unwrap();
    let every: Vec<&[u32]> = vec![&module];
    let mut held = host.resident(&[WORDS, 1], &every).unwrap();
    held.write(0, &vec![1; WORDS]).unwrap();

    held.set_patience(IMPATIENT);
    let groups = [Groups::Fixed(over(host.width()))];
    assert!(wedge(&mut held, &groups).is_some());

    drop(held);
    drop(host);
}

#[test]
fn the_bound_does_not_get_in_the_way_of_ordinary_work() {
    let Some(host) = device("the_bound_does_not_get_in_the_way_of_ordinary_work") else {
        return;
    };
    let module = walk(host.width()).unwrap();
    let every: Vec<&[u32]> = vec![&module];
    let mut held = host.resident(&[WORDS, 1], &every).unwrap();
    held.write(0, &vec![1; WORDS]).unwrap();

    let groups = [Groups::Fixed(over(host.width()))];
    held.frame(&groups).unwrap();

    let summed = held.read(1, 1).unwrap();
    assert_eq!(summed.first().copied(), Some(u32::try_from(WORDS).unwrap()));
}
