//! A buffer larger than the memory behind it, and a kernel that reaches into it.
//!
//! What `Host::resident_vast` gives is an address range: the buffer exists, a shader may index
//! anywhere in it, and nothing is behind any of it until something is bound. What it does **not**
//! give is a device that fetches what a shader touches — there is no page fault here that reaches
//! the host, and a read of an unbound range answers with whatever this file measures it answering
//! with. The loop that decides what to fetch stays the caller's.
//!
//! Every test here says so and passes where the device reports no sparse features.

// A test may panic — that is how it reports.
#![allow(clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use grit::{Error, Groups, Host};
use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, U32};

/// How large the address range is, in words. Four gigabytes less a megabyte.
///
/// **This is not a terabyte, and the reason is not sparse binding.** The range a *descriptor* may
/// reach is `maxStorageBufferRange`, which is 4095 MiB on both parts here, and a shader sees a
/// binding through a descriptor. Sparse binding lifts what has to be *behind* an address range and
/// not how much of one a shader can address.
const VAST: usize = 1024 * 1024 * 1023 / 4 * 4;

/// Where in that range the window goes, in words: two gigabytes in.
const WHERE_AT: usize = 512 * 1024 * 1024;

/// How many words the answer comes back in. A whole subgroup at every width `grit` builds for.
const OUT: usize = 64;

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

/// Write one word far into binding 1, read it straight back, and leave it in binding 0.
///
/// The read is what makes this a test of the memory rather than of the write: a store into an
/// address nothing is behind is discarded, and a load of one answers with something this measures.
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

/// Only read, never write — what an unbound page answers, with nothing to answer it from.
///
/// **The obvious kernel cannot ask this.** Storing a word and loading it straight back in the same
/// invocation is a store the compiler may hand to the load without either reaching memory: written
/// that way this reported `0x00000001` from a page nothing was bound to, which is what the lane had
/// just written and not what the device holds. So this one only loads.
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

/// Four gigabytes of address range, one page of memory, and a kernel that finds it.
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

    // **A read of nothing, before there is anything.** What comes back is the device's answer to a
    // load from an unbound page, which is a property of the part and is printed rather than
    // asserted.
    held.write(0, &[0xDEAD_BEEF; OUT]).expect("a marker");
    held.frame(&[Groups::Fixed(1), Groups::Fixed(0)])
        .expect("a frame");
    let empty = held.read(0, OUT).expect("a read");
    eprintln!("  a load from an unbound page answers {:#010x}", empty[0]);

    // And now one page of it, exactly where the kernel is looking.
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

    // And taken away again.
    held.unbind(1, WHERE_AT, page).expect("the page back");
    assert_eq!(
        held.bound(1),
        0,
        "the page was unbound and {} words remain",
        held.bound(1)
    );
}

/// What a range that is not a whole number of pages, or that runs past the end, is answered with.
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

/// Binding the same window twice replaces what was there rather than leaking it.
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
    // And what is there is what the last bind put there, not what the first did.
    held.write(0, &[0; OUT]).expect("a clear");
    held.frame(&[Groups::Fixed(1)]).expect("a frame");
    let back = held.read(0, OUT).expect("a read");
    assert_eq!(
        back.first().copied(),
        Some(1),
        "the page rebound is not the page read"
    );
}
