#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use grit::{Error, Groups, Host};
use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, U32};

const VAST: usize = 1024 * 1024 * 1023 / 4 * 4;

const WHERE_AT: usize = 512 * 1024 * 1024;

const OUT: usize = 64;

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
        eprintln!("SKIPPED {}", host.name());
        return;
    }
    let width = host.width();
    let looking = peeking(width, WHERE_AT).unwrap();
    let module = reaching(width, WHERE_AT).unwrap();
    let mut held = host
        .resident_vast(&[OUT, VAST], &[&looking, &module], 1)
        .unwrap();

    let page = held.page(1).unwrap();
    eprintln!(
        "{} {width} {} {} {}",
        host.name(),
        VAST / (1024 * 1024 / 4),
        host.heap() / (1 << 20),
        page / 256
    );
    assert!(held.page(0).is_none());
    assert_eq!(held.bound(1), 0);

    held.write(0, &[0xDEAD_BEEF; OUT]).unwrap();
    held.frame(&[Groups::Fixed(1), Groups::Fixed(0)]).unwrap();
    let empty = held.read(0, OUT).unwrap();
    eprintln!("{:#010x}", empty[0]);

    held.bind(1, WHERE_AT, page).unwrap();
    assert_eq!(held.bound(1), page);
    eprintln!("{} {WHERE_AT} {}", page / 256, WHERE_AT / (1024 * 1024 / 4));

    held.write(0, &[0xDEAD_BEEF; OUT]).unwrap();
    held.frame(&[Groups::Fixed(0), Groups::Fixed(1)]).unwrap();
    let back = held.read(0, OUT).unwrap();
    for (lane, &one) in back.iter().take(width as usize).enumerate() {
        assert_eq!(one, lane as u32 + 1);
    }
    eprintln!("{width}");

    held.unbind(1, WHERE_AT, page).unwrap();
    assert_eq!(held.bound(1), 0);
}

#[test]
fn a_bind_that_does_not_fit_says_so() {
    let Some(host) = device("a_bind_that_does_not_fit_says_so") else {
        return;
    };
    if !host.sparse() {
        eprintln!("SKIPPED {}", host.name());
        return;
    }
    let width = host.width();
    let module = reaching(width, 0).unwrap();
    let mut held = host.resident_vast(&[OUT, VAST], &[&module], 1).unwrap();
    let page = held.page(1).unwrap();

    assert!(matches!(held.bind(1, 1, page), Err(Error::Overrun { .. })));
    assert!(matches!(
        held.bind(1, 0, page / 2),
        Err(Error::Overrun { .. })
    ));
    assert!(matches!(
        held.bind(1, VAST, page),
        Err(Error::Overrun { index: 1, needed, held: has })
            if needed == VAST + page && has == VAST
    ));
    assert!(matches!(
        held.bind(1, VAST, page),
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
        eprintln!("SKIPPED {}", host.name());
        return;
    }
    let width = host.width();
    let module = reaching(width, WHERE_AT).unwrap();
    let mut held = host.resident_vast(&[OUT, VAST], &[&module], 1).unwrap();
    let page = held.page(1).unwrap();

    for _ in 0..8 {
        held.bind(1, WHERE_AT, page).unwrap();
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
        eprintln!("SKIPPED {}", host.name());
        return;
    }
    let width = host.width();
    let looking = peeking(width, WHERE_AT).unwrap();
    let module = reaching(width, WHERE_AT).unwrap();
    let mut held = host
        .resident_vast(&[OUT, VAST], &[&looking, &module], 1)
        .unwrap();

    let page = held.page(1).unwrap();
    held.bind(1, WHERE_AT, page).unwrap();
    held.bind(1, WHERE_AT + page, page).unwrap();
    assert_eq!(held.bound(1), page * 2);

    held.unbind(1, WHERE_AT, page).unwrap();
    assert_eq!(held.bound(1), page);

    held.bind(1, WHERE_AT, page).unwrap();
    assert_eq!(held.bound(1), page * 2);

    held.unbind(1, WHERE_AT, page * 2).unwrap();
    assert_eq!(held.bound(1), 0);
}

#[test]
fn a_window_ending_on_the_last_word_fits_and_a_window_of_nothing_binds_nothing() {
    let Some(host) = device("a_window_ending_on_the_last_word_fits") else {
        return;
    };
    if !host.sparse() {
        eprintln!("SKIPPED {}", host.name());
        return;
    }
    let width = host.width();
    let looking = peeking(width, WHERE_AT).unwrap();
    let module = reaching(width, WHERE_AT).unwrap();
    let mut held = host
        .resident_vast(&[OUT, VAST], &[&looking, &module], 1)
        .unwrap();

    let page = held.page(1).unwrap();
    assert_eq!(held.bound(1), 0);

    held.bind(1, 0, 0).unwrap();
    assert_eq!(held.bound(1), 0);

    let last = (VAST / page) * page - page;
    held.bind(1, last, page).unwrap();
    assert_eq!(held.bound(1), page);
    held.unbind(1, last, page).unwrap();
    assert_eq!(held.bound(1), 0);
}
