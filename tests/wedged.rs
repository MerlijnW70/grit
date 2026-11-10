#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use std::time::Duration;

use grit::{Error, Groups, Resident};
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

    // What a caller gets who sets nothing. The environment may name it, and CI
    // does -- 300 seconds, because ten is not a bound a software implementation
    // under load can meet, and a bound crossed by work that was going to finish
    // reports a device lost that was never lost. So what is asserted here is the
    // wiring: a new resident takes the named bound rather than one of its own.
    // That ten seconds is the bound when nothing names one, and that nought and
    // nonsense do not become bounds, is pinned by the test below, which needs no
    // device to ask it.
    let named = std::env::var("GRIT_PATIENCE_SECS").ok();
    assert_eq!(held.patience(), Resident::patience_of(named.as_deref()));

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
    // The kernel adds into binding 1, so nought is where the sum has to start.
    // A fresh buffer is not promised to hold anything: it reads as nought on
    // this card and on lavapipe under Linux, and as whatever was there on
    // lavapipe under Windows, where this read 2180047112.
    held.write(1, &[0]).unwrap();

    let groups = [Groups::Fixed(over(host.width()))];
    held.frame(&groups).unwrap();

    let summed = held.read(1, 1).unwrap();
    assert_eq!(summed.first().copied(), Some(u32::try_from(WORDS).unwrap()));
}

#[test]
fn the_bound_is_settable_from_outside_and_ignores_what_is_not_a_bound() {
    // No device: this is what a new resident is given before it is asked
    // anything. The bound stops a hung dispatch parking a process in the
    // driver, so it cannot be turned off -- nought and nonsense both leave the
    // default standing rather than becoming a bound no work could meet.
    assert_eq!(
        Resident::patience_of(None),
        Resident::PATIENCE,
        "nothing named leaves the default"
    );
    assert_eq!(
        Resident::patience_of(Some("300")),
        Duration::from_secs(300),
        "a count of seconds is the bound"
    );
    assert_eq!(
        Resident::patience_of(Some("0")),
        Resident::PATIENCE,
        "a bound of nothing is not a bound"
    );
    assert_eq!(
        Resident::patience_of(Some("later")),
        Resident::PATIENCE,
        "and neither is a word"
    );
    assert_eq!(
        Resident::PATIENCE,
        Duration::from_secs(10),
        "the default a caller gets when the environment says nothing"
    );
}

#[cfg(feature = "faults")]
mod freed {
    use super::{IMPATIENT, TRIES, WORDS, device, over, walk, wedge};

    use grit::resident::faults::{self, Step};
    use grit::{Error, Groups};

    fn alive() -> Vec<String> {
        Step::ALL
            .iter()
            .map(|&step| (step, faults::count(step)))
            .filter(|(_, count)| count.made != count.gone || count.outstanding != 0)
            .map(|(step, count)| {
                format!(
                    "{step:?} made {} freed {}; the driver holds {} of its {} host allocations",
                    count.made, count.gone, count.outstanding, count.allocations
                )
            })
            .collect()
    }

    #[test]
    fn a_wedged_resident_whose_frame_finishes_late_is_freed_when_dropped() {
        let Some(host) =
            device("a_wedged_resident_whose_frame_finishes_late_is_freed_when_dropped")
        else {
            return;
        };
        faults::reset();
        let module = walk(host.width()).unwrap();
        let every: Vec<&[u32]> = vec![&module];
        let mut held = host.resident(&[WORDS, 1], &every).unwrap();
        held.write(0, &vec![1; WORDS]).unwrap();

        held.set_patience(IMPATIENT);
        let groups = [Groups::Fixed(over(host.width()))];
        assert!(wedge(&mut held, &groups).is_some());
        assert_eq!(held.in_flight(), 1, "the frame that ran out of time");

        drop(held);
        let left = alive();
        assert!(left.is_empty(), "{left:?}");
    }

    #[test]
    fn a_wedged_resident_whose_transfer_finishes_late_is_freed_when_dropped() {
        let Some(host) =
            device("a_wedged_resident_whose_transfer_finishes_late_is_freed_when_dropped")
        else {
            return;
        };
        faults::reset();
        let module = walk(host.width()).unwrap();
        let every: Vec<&[u32]> = vec![&module];
        let mut held = host.resident(&[WORDS, 1], &every).unwrap();

        held.set_patience(IMPATIENT);
        let mut wedged = false;
        for _ in 0..TRIES {
            match held.read(0, WORDS) {
                Err(Error::Wedged { .. }) => {
                    wedged = true;
                    break;
                }
                Err(other) => panic!("{other:?}"),
                Ok(_) => {}
            }
        }
        assert!(
            wedged,
            "no transfer of {WORDS} words ran out of no time at all"
        );
        assert_eq!(held.in_flight(), 0, "only the transfer was sent");

        drop(held);
        let left = alive();
        assert!(left.is_empty(), "{left:?}");
    }

    #[test]
    fn a_resident_that_never_ran_is_freed_on_a_host_another_resident_wedged() {
        let Some(host) =
            device("a_resident_that_never_ran_is_freed_on_a_host_another_resident_wedged")
        else {
            return;
        };
        faults::reset();
        let module = walk(host.width()).unwrap();
        let every: Vec<&[u32]> = vec![&module];
        let mut wedging = host.resident(&[WORDS, 1], &every).unwrap();
        let idle = host.resident(&[WORDS, 1], &every).unwrap();
        wedging.write(0, &vec![1; WORDS]).unwrap();

        wedging.set_patience(IMPATIENT);
        let groups = [Groups::Fixed(over(host.width()))];
        assert!(wedge(&mut wedging, &groups).is_some());

        drop(idle);
        drop(wedging);
        let left = alive();
        assert!(left.is_empty(), "{left:?}");
    }

    #[test]
    fn a_wedged_resident_whose_sparse_bind_finishes_late_is_freed_when_dropped() {
        let Some(host) =
            device("a_wedged_resident_whose_sparse_bind_finishes_late_is_freed_when_dropped")
        else {
            return;
        };
        if !host.sparse() {
            crate::common::skip(host.name());
            return;
        }
        faults::reset();
        let module = walk(host.width()).unwrap();
        let every: Vec<&[u32]> = vec![&module];
        let mut held = host.resident_vast(&[WORDS, 1 << 24], &every, 1).unwrap();
        let page = held.page(1).unwrap();

        held.set_patience(IMPATIENT);
        let mut wedged = false;
        for at in 0..TRIES {
            match held.bind(1, at * page, page) {
                Err(Error::Wedged { .. }) => {
                    wedged = true;
                    break;
                }
                Err(other) => panic!("{other:?}"),
                Ok(()) => {}
            }
        }
        assert!(wedged, "no sparse bind ran out of no time at all");

        drop(held);
        let left = alive();
        assert!(left.is_empty(), "{left:?}");
    }

    #[test]
    fn a_wedged_host_whose_residents_all_finished_destroys_its_device() {
        faults::reset();
        let Some(host) = device("a_wedged_host_whose_residents_all_finished_destroys_its_device")
        else {
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
        let made = faults::count(Step::Device);
        assert_eq!(
            (made.made, made.gone, made.outstanding),
            (1, 1, 0),
            "{made:?}"
        );
    }

    #[test]
    fn a_wedged_host_keeps_its_device_while_a_resident_still_owes_it() {
        faults::reset();
        let Some(host) = device("a_wedged_host_keeps_its_device_while_a_resident_still_owes_it")
        else {
            return;
        };
        let module = walk(host.width()).unwrap();
        let every: Vec<&[u32]> = vec![&module];
        let mut held = host.resident(&[WORDS, 1], &every).unwrap();
        held.write(0, &vec![1; WORDS]).unwrap();

        held.set_patience(IMPATIENT);
        let groups = [Groups::Fixed(over(host.width()))];
        assert!(wedge(&mut held, &groups).is_some());

        std::mem::forget(held);
        drop(host);
        let made = faults::count(Step::Device);
        assert_eq!((made.made, made.gone), (1, 0), "{made:?}");
    }
}
