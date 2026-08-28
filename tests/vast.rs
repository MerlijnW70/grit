#![allow(clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

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
            eprintln!("SKIPPED {label}: no Vulkan device");
            None
        }
        Err(why) => {
            eprintln!("SKIPPED {label}: {why}");
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
        eprintln!("SKIPPED: {} reports no sparse features", host.name());
        return;
    }
    let width = host.width();
    let looking = peeking(width, WHERE_AT).expect("a kernel");
    let module = reaching(width, WHERE_AT).expect("a kernel");
    let mut held = host
        .resident_vast(&[OUT, VAST], &[&looking, &module], 1)
        .expect("a vast binding");

    let page = held.page(1).expect("binding 1 is the vast one");
    eprintln!(
        "{} — subgroup {width}, {} MiB of address range, {} MiB of memory, a page is {} KiB",
        host.name(),
        VAST / (1024 * 1024 / 4),
        host.heap() / (1 << 20),
        page / 256,
    );
    assert!(
        held.page(0).is_none(),
        "binding 0 is not a vast one and says it is"
    );
    assert_eq!(
        held.bound(1),
        0,
        "nothing is bound yet and {} words are",
        held.bound(1)
    );

    held.write(0, &[0xDEAD_BEEF; OUT]).expect("a marker");
    held.frame(&[Groups::Fixed(1), Groups::Fixed(0)])
        .expect("a frame");
    let empty = held.read(0, OUT).expect("a read");
    eprintln!("  a load from an unbound page answers {:#010x}", empty[0]);

    held.bind(1, WHERE_AT, page).expect("one page");
    assert_eq!(
        held.bound(1),
        page,
        "one page is {page} words and {} are bound",
        held.bound(1)
    );
    eprintln!(
        "  bound {} KiB at word {WHERE_AT}, which is {} MiB into the range",
        page / 256,
        WHERE_AT / (1024 * 1024 / 4),
    );

    held.write(0, &[0xDEAD_BEEF; OUT]).expect("a marker");
    held.frame(&[Groups::Fixed(0), Groups::Fixed(1)])
        .expect("a frame");
    let back = held.read(0, OUT).expect("a read");
    for (lane, &one) in back.iter().take(width as usize).enumerate() {
        assert_eq!(
            one,
            lane as u32 + 1,
            "lane {lane} wrote {} and read back {one:#010x}",
            lane + 1
        );
    }
    eprintln!("  {width} lanes wrote two gigabytes in and read back what they wrote");

    held.unbind(1, WHERE_AT, page).expect("the page back");
    assert_eq!(
        held.bound(1),
        0,
        "the page was unbound and {} words remain",
        held.bound(1)
    );
}

#[test]
fn a_bind_that_does_not_fit_says_so() {
    let Some(host) = device("a_bind_that_does_not_fit_says_so") else {
        return;
    };
    if !host.sparse() {
        eprintln!("SKIPPED: {} reports no sparse features", host.name());
        return;
    }
    let width = host.width();
    let module = reaching(width, 0).expect("a kernel");
    let mut held = host
        .resident_vast(&[OUT, VAST], &[&module], 1)
        .expect("a vast binding");
    let page = held.page(1).expect("the vast one");

    assert!(
        matches!(held.bind(1, 1, page), Err(Error::Overrun { .. })),
        "an offset that is not a whole page was taken"
    );
    assert!(
        matches!(held.bind(1, 0, page / 2), Err(Error::Overrun { .. })),
        "a length that is not a whole page was taken"
    );
    assert!(
        matches!(held.bind(1, VAST, page), Err(Error::Overrun { .. })),
        "a range past the end was taken"
    );
    assert!(
        matches!(held.bind(0, 0, page), Err(Error::NoSparse)),
        "an ordinary binding was bound by hand"
    );
    assert!(
        matches!(held.bind(9, 0, page), Err(Error::NoSuchBinding { .. })),
        "a binding that does not exist was bound"
    );
    assert_eq!(held.bound(1), 0, "a refused bind left something behind");
}

#[test]
fn binding_the_same_window_twice_does_not_leak() {
    let Some(host) = device("binding_the_same_window_twice_does_not_leak") else {
        return;
    };
    if !host.sparse() {
        eprintln!("SKIPPED: {} reports no sparse features", host.name());
        return;
    }
    let width = host.width();
    let module = reaching(width, WHERE_AT).expect("a kernel");
    let mut held = host
        .resident_vast(&[OUT, VAST], &[&module], 1)
        .expect("a vast binding");
    let page = held.page(1).expect("the vast one");

    for _ in 0..8 {
        held.bind(1, WHERE_AT, page).expect("the same page again");
        assert_eq!(
            held.bound(1),
            page,
            "{} words are bound after one page",
            held.bound(1)
        );
    }
    held.write(0, &[0; OUT]).expect("a clear");
    held.frame(&[Groups::Fixed(1)]).expect("a frame");
    let back = held.read(0, OUT).expect("a read");
    assert_eq!(
        back.first().copied(),
        Some(1),
        "the page rebound is not the page read"
    );
}
