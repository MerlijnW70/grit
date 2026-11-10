#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use grit::{Error, Groups, Host};
use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, U32};

const VAST: usize = 1024 * 1024 * 1023 / 4 * 4;

const OUT: usize = 64;

fn vast(host: &Host) -> usize {
    let words = host.limits().storage_range / size_of::<u32>() as u64;
    VAST.min(usize::try_from(words).unwrap_or(usize::MAX))
}

fn where_at(vast: usize) -> usize {
    (vast - 1).next_power_of_two() / 2
}

fn reaching(subgroup: u32, at: usize) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, 2))?;
    let uint = kernel.index_type();
    let lane = kernel.local_index();
    let base = kernel
        .module()
        .constant_u32(u32::try_from(at).unwrap_or(0))?;
    let far = kernel.module().i_add(uint, base, lane)?;
    let one = kernel.module().constant_u32(1)?;
    let value = kernel.module().i_add(uint, lane, one)?;
    kernel.store_at(1, far, value)?;
    let held = kernel.load_at(1, far)?;
    kernel.store_at(0, lane, held)?;
    kernel.finish()
}

fn peeking(subgroup: u32, at: usize) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, 2))?;
    let uint = kernel.index_type();
    let lane = kernel.local_index();
    let base = kernel
        .module()
        .constant_u32(u32::try_from(at).unwrap_or(0))?;
    let far = kernel.module().i_add(uint, base, lane)?;
    let held = kernel.load_at(1, far)?;
    kernel.store_at(0, lane, held)?;
    kernel.finish()
}

#[test]
fn a_window_of_a_buffer_larger_than_the_card() {
    let Some(host) = device("a_window_of_a_buffer_larger_than_the_card") else {
        return;
    };
    if !host.sparse() {
        common::skip(host.name());
        return;
    }
    let width = host.width();
    let vast = vast(&host);
    let where_at = where_at(vast);
    let looking = peeking(width, where_at).unwrap();
    let module = reaching(width, where_at).unwrap();
    let mut held = host
        .resident_vast(&[OUT, vast], &[&looking, &module], 1)
        .unwrap();

    let page = held.page(1).unwrap();
    eprintln!(
        "{} {width} {} {} {}",
        host.name(),
        vast / (1024 * 1024 / 4),
        host.heap() / (1 << 20),
        page / 256
    );
    assert!(held.page(0).is_none());
    assert_eq!(held.bound(1), 0);

    let limits = host.limits();
    let bytes = vast as u64 * size_of::<u32>() as u64;
    assert!(bytes <= limits.storage_range);
    if limits.storage_range > limits.allocation {
        assert!(bytes > limits.allocation, "{bytes} {}", limits.allocation);
    }
    assert!(where_at + page <= vast);

    if host.strict() {
        held.write(0, &[0xDEAD_BEEF; OUT]).unwrap();
        held.frame(&[Groups::Fixed(1), Groups::Fixed(0)]).unwrap();
        let empty = held.read(0, OUT).unwrap();
        for &one in empty.iter().take(width as usize) {
            assert_eq!(one, 0);
        }
    }

    held.bind(1, where_at, page).unwrap();
    assert_eq!(held.bound(1), page);
    eprintln!("{} {where_at} {}", page / 256, where_at / (1024 * 1024 / 4));

    held.write(0, &[0xDEAD_BEEF; OUT]).unwrap();
    held.frame(&[Groups::Fixed(0), Groups::Fixed(1)]).unwrap();
    let back = held.read(0, OUT).unwrap();
    for (lane, &one) in back.iter().take(width as usize).enumerate() {
        assert_eq!(one, lane as u32 + 1);
    }
    eprintln!("{width}");

    held.unbind(1, where_at, page).unwrap();
    assert_eq!(held.bound(1), 0);
}

#[test]
fn a_bind_that_does_not_fit_says_so() {
    let Some(host) = device("a_bind_that_does_not_fit_says_so") else {
        return;
    };
    if !host.sparse() {
        common::skip(host.name());
        return;
    }
    let width = host.width();
    let vast = vast(&host);
    let module = reaching(width, 0).unwrap();
    let mut held = host.resident_vast(&[OUT, vast], &[&module], 1).unwrap();
    let page = held.page(1).unwrap();

    assert!(matches!(held.bind(1, 1, page), Err(Error::Overrun { .. })));
    assert!(matches!(
        held.bind(1, 0, page / 2),
        Err(Error::Overrun { .. })
    ));
    assert!(matches!(
        held.bind(1, vast, page),
        Err(Error::Overrun { index: 1, needed, held: has })
            if needed == vast + page && has == vast
    ));
    assert!(matches!(
        held.bind(1, vast, page),
        Err(Error::Overrun { .. })
    ));
    assert!(matches!(held.bind(0, 0, page), Err(Error::NoSparse)));
    assert!(matches!(
        held.bind(9, 0, page),
        Err(Error::NoSuchBinding { .. })
    ));
    assert_eq!(held.bound(1), 0);
}

#[test]
fn binding_the_same_window_twice_does_not_leak() {
    let Some(host) = device("binding_the_same_window_twice_does_not_leak") else {
        return;
    };
    if !host.sparse() {
        common::skip(host.name());
        return;
    }
    let width = host.width();
    let vast = vast(&host);
    let where_at = where_at(vast);
    let module = reaching(width, where_at).unwrap();
    let mut held = host.resident_vast(&[OUT, vast], &[&module], 1).unwrap();
    let page = held.page(1).unwrap();

    for _ in 0..8 {
        held.bind(1, where_at, page).unwrap();
        assert_eq!(held.bound(1), page);
    }
    held.write(0, &[0; OUT]).unwrap();
    held.frame(&[Groups::Fixed(1)]).unwrap();
    let back = held.read(0, OUT).unwrap();
    assert_eq!(back.first().copied(), Some(1));
}

#[test]
fn unbinding_one_window_leaves_its_neighbour_where_it_was() {
    let Some(host) = device("unbinding_one_window_leaves_its_neighbour_where_it_was") else {
        return;
    };
    if !host.sparse() {
        common::skip(host.name());
        return;
    }
    let width = host.width();
    let vast = vast(&host);
    let where_at = where_at(vast);
    let looking = peeking(width, where_at).unwrap();
    let module = reaching(width, where_at).unwrap();
    let mut held = host
        .resident_vast(&[OUT, vast], &[&looking, &module], 1)
        .unwrap();

    let page = held.page(1).unwrap();
    held.bind(1, where_at, page).unwrap();
    held.bind(1, where_at + page, page).unwrap();
    assert_eq!(held.bound(1), page * 2);

    held.unbind(1, where_at, page).unwrap();
    assert_eq!(held.bound(1), page);

    held.bind(1, where_at, page).unwrap();
    assert_eq!(held.bound(1), page * 2);

    held.unbind(1, where_at, page * 2).unwrap();
    assert_eq!(held.bound(1), 0);
}

#[test]
fn a_window_ending_on_the_last_word_fits_and_a_window_of_nothing_binds_nothing() {
    let Some(host) = device("a_window_ending_on_the_last_word_fits") else {
        return;
    };
    if !host.sparse() {
        common::skip(host.name());
        return;
    }
    let width = host.width();
    let vast = vast(&host);
    let where_at = where_at(vast);
    let looking = peeking(width, where_at).unwrap();
    let module = reaching(width, where_at).unwrap();
    let mut held = host
        .resident_vast(&[OUT, vast], &[&looking, &module], 1)
        .unwrap();

    let page = held.page(1).unwrap();
    assert_eq!(held.bound(1), 0);

    held.bind(1, 0, 0).unwrap();
    assert_eq!(held.bound(1), 0);

    let last = (vast / page) * page - page;
    held.bind(1, last, page).unwrap();
    assert_eq!(held.bound(1), page);
    held.unbind(1, last, page).unwrap();
    assert_eq!(held.bound(1), 0);
}

#[test]
fn a_window_is_not_moved_under_a_frame_still_running() {
    let Some(host) = device("a_window_is_not_moved_under_a_frame_still_running") else {
        return;
    };
    if !host.sparse() {
        common::skip(host.name());
        return;
    }
    let width = host.width();
    let vast = vast(&host);
    let where_at = where_at(vast);
    let module = reaching(width, where_at).unwrap();
    let mut held = host.resident_vast(&[OUT, vast], &[&module], 1).unwrap();
    let page = held.page(1).unwrap();
    held.bind(1, where_at, page).unwrap();
    held.write(0, &[0; OUT]).unwrap();

    let running = held.submit(&[&[Groups::Fixed(1)]]).unwrap();
    assert!(matches!(
        held.bind(1, where_at + page, page),
        Err(Error::StillInFlight)
    ));
    assert!(matches!(
        held.unbind(1, where_at, page),
        Err(Error::StillInFlight)
    ));
    held.wait(running).unwrap();

    held.unbind(1, where_at, page).unwrap();
    assert_eq!(held.bound(1), 0);
}
