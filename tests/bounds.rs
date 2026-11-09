#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use grit::{Error, Groups, Host, Run};
use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, U32};

const LEN: usize = 16;

const LAID: u32 = 0xA5A5_A5A5;

const STALE: u32 = 0x0000_BEEF;

const FAR: usize = 1 << 62;

fn bump(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, 1, 1))?;
    let uint = kernel.index_type();
    let head = kernel.module().constant_u32(0)?;
    let one = kernel.module().constant_u32(1)?;
    let held = kernel.load_at(0, head)?;
    let next = kernel.module().i_add(uint, held, one)?;
    kernel.store_at(0, head, next)?;
    kernel.finish()
}

fn laid(host: &Host) -> grit::Resident<'_> {
    let mut held = host.resident(&[LEN, LEN], &[]).unwrap();
    held.write(0, &[LAID; LEN]).unwrap();
    held.write(1, &[STALE; LEN]).unwrap();
    held
}

#[test]
fn an_empty_write_leaves_the_binding_as_it_was() {
    let Some(host) = device("an_empty_write_leaves_the_binding_as_it_was") else {
        return;
    };
    let mut held = laid(&host);

    held.write_at(0, 7, &[]).unwrap();
    assert_eq!(held.read(0, LEN).unwrap(), vec![LAID; LEN]);

    held.write(1, &[STALE; LEN]).unwrap();
    held.write(0, &[]).unwrap();
    assert_eq!(held.read(0, LEN).unwrap(), vec![LAID; LEN]);

    held.write(1, &[STALE; LEN]).unwrap();
    held.write_at(0, LEN - 1, &[]).unwrap();
    assert_eq!(held.read(0, LEN).unwrap(), vec![LAID; LEN]);

    held.write(1, &[STALE; LEN]).unwrap();
    let nothing: [u32; 0] = [];
    let runs = [
        Run {
            index: 0,
            at: 3,
            words: &nothing,
        },
        Run {
            index: 0,
            at: 9,
            words: &nothing,
        },
    ];
    held.write_many(&runs).unwrap();
    assert_eq!(held.read(0, LEN).unwrap(), vec![LAID; LEN]);
}

#[test]
fn an_empty_run_beside_a_real_one_writes_only_the_real_one() {
    let Some(host) = device("an_empty_run_beside_a_real_one_writes_only_the_real_one") else {
        return;
    };
    let mut held = laid(&host);
    let nothing: [u32; 0] = [];
    let real = [1, 2];
    let runs = [
        Run {
            index: 0,
            at: 2,
            words: &nothing,
        },
        Run {
            index: 0,
            at: 5,
            words: &real,
        },
        Run {
            index: 0,
            at: 11,
            words: &nothing,
        },
    ];
    held.write_many(&runs).unwrap();
    let mut wanted = vec![LAID; LEN];
    wanted[5] = 1;
    wanted[6] = 2;
    assert_eq!(held.read(0, LEN).unwrap(), wanted);
}

#[test]
fn a_write_that_starts_past_the_end_is_an_overrun_even_when_empty() {
    let Some(host) = device("a_write_that_starts_past_the_end_is_an_overrun") else {
        return;
    };
    let mut held = laid(&host);

    assert!(matches!(
        held.write_at(0, LEN, &[]),
        Err(Error::Overrun {
            index: 0,
            needed: LEN,
            held: LEN
        })
    ));
    let nothing: [u32; 0] = [];
    let runs = [Run {
        index: 0,
        at: LEN,
        words: &nothing,
    }];
    assert!(matches!(
        held.write_many(&runs),
        Err(Error::Overrun { index: 0, .. })
    ));
    assert_eq!(held.read(0, LEN).unwrap(), vec![LAID; LEN]);
    assert_eq!(held.read(1, LEN).unwrap(), vec![STALE; LEN]);
}

#[test]
fn an_offset_that_wraps_is_an_overrun_and_writes_nothing() {
    let Some(host) = device("an_offset_that_wraps_is_an_overrun_and_writes_nothing") else {
        return;
    };
    let mut held = laid(&host);

    assert!(matches!(
        held.write_at(0, FAR, &[1]),
        Err(Error::Overrun { index: 0, .. })
    ));
    assert!(matches!(
        held.write_at(0, FAR - 1, &[1]),
        Err(Error::Overrun { index: 0, .. })
    ));
    assert!(matches!(
        held.write_at(0, usize::MAX, &[1]),
        Err(Error::Overrun { index: 0, .. })
    ));
    let one = [1];
    let runs = [Run {
        index: 0,
        at: FAR,
        words: &one,
    }];
    assert!(matches!(
        held.write_many(&runs),
        Err(Error::Overrun { index: 0, .. })
    ));
    assert_eq!(held.read(0, LEN).unwrap(), vec![LAID; LEN]);
}

#[test]
fn a_read_too_long_to_count_is_refused_without_a_panic() {
    let Some(host) = device("a_read_too_long_to_count_is_refused_without_a_panic") else {
        return;
    };
    let mut held = laid(&host);

    let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        held.read(0, FAR + 1).map(|words| words.len())
    }));
    assert!(matches!(read, Ok(Err(Error::Overrun { index: 0, .. }))));

    let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        held.read(0, usize::MAX).map(|words| words.len())
    }));
    assert!(matches!(read, Ok(Err(Error::Overrun { index: 0, .. }))));

    assert_eq!(held.read(0, LEN).unwrap(), vec![LAID; LEN]);
}

#[test]
fn a_read_of_nothing_hands_back_nothing() {
    let Some(host) = device("a_read_of_nothing_hands_back_nothing") else {
        return;
    };
    let mut held = laid(&host);
    assert_eq!(held.read(0, 0).unwrap(), Vec::<u32>::new());
    assert_eq!(held.read(0, LEN).unwrap(), vec![LAID; LEN]);
    assert!(matches!(
        held.read(0, LEN + 1),
        Err(Error::Overrun {
            index: 0,
            needed: 17,
            held: LEN
        })
    ));
}

#[test]
fn a_read_back_too_long_to_count_is_refused_before_the_frame() {
    let Some(host) = device("a_read_back_too_long_to_count_is_refused") else {
        return;
    };
    let mut held = laid(&host);
    let stages: Vec<&[Groups]> = Vec::new();

    let asked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        held.submit_reading(&stages, 0, FAR + 1).map(|_| ())
    }));
    assert!(matches!(asked, Ok(Err(Error::Overrun { index: 0, .. }))));
    let asked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        held.submit_reading_both(&stages, (0, 1), (1, FAR))
            .map(|_| ())
    }));
    assert!(matches!(asked, Ok(Err(Error::Overrun { index: 1, .. }))));
    assert_eq!(held.in_flight(), 0);

    let ticket = held.submit_reading_both(&stages, (0, 0), (1, 2)).unwrap();
    let (_, first, second) = held.wait_reading_both(ticket).unwrap();
    assert_eq!(first, Vec::<u32>::new());
    assert_eq!(second, vec![STALE, STALE]);
}

#[test]
fn a_read_of_the_sparse_binding_is_bounded_by_the_staging_buffer() {
    let Some(host) = device("a_read_of_the_sparse_binding_is_bounded_by_staging") else {
        return;
    };
    eprintln!("device {}", host.name());
    assert!(host.sparse(), "{} has no sparse residency", host.name());
    let mut held = host.resident_vast(&[LEN, 1 << 24], &[], 1).unwrap();

    assert_eq!(held.read(1, LEN).unwrap().len(), LEN);
    assert!(matches!(
        held.read(1, LEN + 1),
        Err(Error::Staged {
            needed: 17,
            staging: LEN
        })
    ));
    assert!(matches!(
        held.read(1, 1 << 16),
        Err(Error::Staged {
            needed: 65_536,
            staging: LEN
        })
    ));
    assert!(matches!(
        held.read(1, (1 << 24) + 1),
        Err(Error::Overrun { index: 1, .. })
    ));

    let stages: Vec<&[Groups]> = Vec::new();
    assert!(matches!(
        held.submit_reading(&stages, 1, LEN + 1),
        Err(Error::Staged {
            needed: 17,
            staging: LEN
        })
    ));
    assert_eq!(held.in_flight(), 0);
    held.write(0, &[LAID; LEN]).unwrap();
    assert_eq!(held.read(0, LEN).unwrap(), vec![LAID; LEN]);
}

#[test]
fn a_window_that_wraps_neither_binds_nor_unbinds() {
    let Some(host) = device("a_window_that_wraps_neither_binds_nor_unbinds") else {
        return;
    };
    assert!(host.sparse(), "{} has no sparse residency", host.name());
    let mut held = host.resident_vast(&[LEN, 1 << 24], &[], 1).unwrap();
    let page = held.page(1).unwrap();

    assert!(matches!(
        held.bind(1, FAR, page),
        Err(Error::Overrun { index: 1, .. })
    ));
    assert_eq!(held.bound(1), 0);

    held.bind(1, 0, page).unwrap();
    assert_eq!(held.bound(1), page);
    assert!(matches!(
        held.unbind(1, FAR, page),
        Err(Error::Overrun { index: 1, .. })
    ));
    assert!(matches!(
        held.unbind(1, 0, FAR),
        Err(Error::Overrun { index: 1, .. })
    ));
    assert_eq!(held.bound(1), page);
    held.unbind(1, 0, page).unwrap();
    assert_eq!(held.bound(1), 0);
}

#[test]
fn a_binding_too_large_to_count_is_refused() {
    let Some(host) = device("a_binding_too_large_to_count_is_refused") else {
        return;
    };
    assert!(matches!(
        host.resident(&[FAR], &[]).map(|held| held.bindings()),
        Err(Error::NoMemory { .. })
    ));
    assert!(matches!(
        host.resident(&[LEN, usize::MAX], &[])
            .map(|held| held.bindings()),
        Err(Error::NoMemory { .. })
    ));
}

#[test]
fn a_write_to_a_near_binding_is_refused_while_a_frame_is_in_flight() {
    let Some(host) = device("a_write_to_a_near_binding_is_refused_in_flight") else {
        return;
    };
    let mut held = host.resident_near(&[LEN, 1 << 20], &[], 4096).unwrap();
    held.write(0, &[LAID; LEN]).unwrap();
    assert_eq!(held.pushing().2, 1, "{} kept nothing near", host.name());

    let stages: Vec<&[Groups]> = Vec::new();
    let ticket = held.submit(&stages).unwrap();
    assert_eq!(held.in_flight(), 1);
    assert!(matches!(
        held.write(0, &[1; LEN]),
        Err(Error::StillInFlight)
    ));
    assert!(matches!(
        held.write_at(0, 3, &[1]),
        Err(Error::StillInFlight)
    ));
    assert!(matches!(
        held.write(1, &[1; LEN]),
        Err(Error::StillInFlight)
    ));
    held.wait(ticket).unwrap();

    assert_eq!(held.read(0, LEN).unwrap(), vec![LAID; LEN]);
    held.write_at(0, 3, &[1]).unwrap();
    let mut wanted = vec![LAID; LEN];
    wanted[3] = 1;
    assert_eq!(held.read(0, LEN).unwrap(), wanted);
}

#[test]
fn an_indirect_count_past_its_binding_is_refused() {
    let Some(host) = device("an_indirect_count_past_its_binding_is_refused") else {
        return;
    };
    let module = bump(host.width()).unwrap();
    let mut held = host.resident(&[4, 6], &[&module]).unwrap();
    held.write(0, &[0; 4]).unwrap();
    held.write(1, &[0, 0, 0, 1, 1, 1]).unwrap();

    for at in [4, 5, 6, FAR, usize::MAX] {
        assert!(
            matches!(
                held.frame(&[Groups::From { binding: 1, at }]),
                Err(Error::Overrun { index: 1, .. })
            ),
            "at {at}"
        );
    }
    assert!(matches!(
        held.frame(&[Groups::From { binding: 2, at: 0 }]),
        Err(Error::NoSuchBinding { index: 2, held: 2 })
    ));
    assert_eq!(held.in_flight(), 0);
    assert_eq!(held.read(0, 1).unwrap(), vec![0]);

    held.frame(&[Groups::From { binding: 1, at: 3 }]).unwrap();
    assert_eq!(held.read(0, 1).unwrap(), vec![1]);
}

#[test]
fn a_write_to_a_direct_binding_is_refused_while_a_frame_is_in_flight() {
    let Some(host) = device("a_write_to_a_direct_binding_is_refused_in_flight") else {
        return;
    };
    let mut held = host.resident_direct(&[LEN], &[]).unwrap();
    held.write(0, &[LAID; LEN]).unwrap();
    assert_eq!(held.pushing().2, 1, "{} wrote through staging", host.name());

    let stages: Vec<&[Groups]> = Vec::new();
    let ticket = held.submit(&stages).unwrap();
    assert!(matches!(
        held.write_at(0, LEN - 1, &[1]),
        Err(Error::StillInFlight)
    ));
    held.write_at(0, LEN - 1, &[]).unwrap();
    held.wait(ticket).unwrap();

    held.write_at(0, LEN - 1, &[1]).unwrap();
    let mut wanted = vec![LAID; LEN];
    wanted[LEN - 1] = 1;
    assert_eq!(held.read(0, LEN).unwrap(), wanted);
}
