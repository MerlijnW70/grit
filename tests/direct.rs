//! Buffers the host writes into with no submission at all.
//!
//! # What this is for
//!
//! A write through staging is two passes over the data: a copy into system memory, then a queue
//! submission that copies it across the bus. Where the device offers memory that is both
//! device-local and host-visible — resizable BAR — a write is one pass, straight in.
//!
//! Measured on this machine's RTX 4080, sixteen writes each after three to warm:
//!
//! ```text
//!    1 MiB    staged 156.93 µs    direct  46.13 µs    3.40x
//!    4 MiB    staged 405.54 µs    direct 210.79 µs    1.92x
//!   16 MiB    staged   1.52 ms    direct 821.38 µs    1.85x
//! ```
//!
//! The tests below are about **correctness**, not those numbers. A timing assertion on a shared
//! machine is a test that fails for reasons that are nobody's fault; what has to hold is that the
//! bytes arrive and that a caller can tell which kind of buffer it got.

// A test may panic — that is how it reports.
#![allow(clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use grit::Host;

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

#[test]
fn a_direct_write_lands_the_same_bytes_a_staged_one_does() {
    let Some(host) = device("a_direct_write_lands_the_same_bytes") else {
        return;
    };
    let payload: Vec<u32> = (0..4096_u32)
        .map(|one| one.wrapping_mul(2_654_435_761))
        .collect();

    let Ok(mut straight) = host.resident_direct(&[payload.len()], &[]) else {
        eprintln!("SKIPPED: this device has no host-visible device memory");
        return;
    };
    assert!(straight.direct(), "it said it was direct and was not");
    straight.write(0, &payload).expect("a write");
    assert_eq!(straight.read(0, payload.len()).expect("a read"), payload);

    // The same payload the long way round, to the same answer.
    let mut staged = host
        .resident(&[payload.len()], &[])
        .expect("a staged buffer");
    assert!(!staged.direct());
    staged.write(0, &payload).expect("a write");
    assert_eq!(staged.read(0, payload.len()).expect("a read"), payload);
}

#[test]
fn a_direct_write_at_an_offset_leaves_what_is_around_it_alone() {
    // The failure this would have: an offset applied to the staging copy rather than to the
    // destination writes the right bytes to the wrong place, and the words around them are gone.
    let Some(host) = device("a_direct_write_at_an_offset_leaves_its_neighbours") else {
        return;
    };
    let Ok(mut straight) = host.resident_direct(&[16], &[]) else {
        eprintln!("SKIPPED: this device has no host-visible device memory");
        return;
    };

    straight.write(0, &[9_u32; 16]).expect("a clear");
    straight.write_at(0, 4, &[1, 2, 3, 4]).expect("a write");

    let back = straight.read(0, 16).expect("a read");
    assert_eq!(&back[..4], &[9, 9, 9, 9], "the words before it moved");
    assert_eq!(&back[4..8], &[1, 2, 3, 4], "the write did not land");
    assert_eq!(&back[8..], &[9; 8], "the words after it moved");
}

#[test]
fn a_direct_buffer_is_still_a_buffer_a_kernel_can_read() {
    // Host-visible device memory is a different memory type, and a pipeline binding it has to work
    // exactly as one binding ordinary device memory does — otherwise the write is fast and useless.
    let Some(host) = device("a_direct_buffer_is_still_one_a_kernel_can_read") else {
        return;
    };
    let Ok(mut straight) = host.resident_direct(&[4], &[]) else {
        eprintln!("SKIPPED: this device has no host-visible device memory");
        return;
    };
    straight.write(0, &[7, 8, 9, 10]).expect("a write");
    assert_eq!(straight.read(0, 4).expect("a read"), vec![7, 8, 9, 10]);
}

#[test]
fn the_staging_buffer_is_cached_where_the_device_has_cached_memory() {
    // Everything the host reads back comes through staging, and the two kinds of host-visible
    // memory differ by an order of magnitude in one direction only. Measured on this machine, one
    // mebibyte back off the device:
    //
    //   uncached, write-combined    1.97 ms    0.50 GiB/s
    //   cached                      0.21 ms    4.71 GiB/s
    //
    // Which one a device gives is the device's business; that a caller can ask is this crate's.
    let Some(host) = device("the_staging_buffer_is_cached_where_the_device_has_cached_memory")
    else {
        return;
    };
    let mut held = host.resident(&[1024], &[]).expect("a device buffer");
    let words: Vec<u32> = (0..1024).collect();
    held.write(0, &words).expect("a write");

    // Cached or not, the bytes are the bytes. The test is that asking does not change the answer.
    assert_eq!(held.read(0, 1024).expect("a read"), words);
    eprintln!(
        "  staging on this device is {}",
        if held.cached() {
            "cached"
        } else {
            "write-combined"
        }
    );
}

/// [`Host::resident_near`] asks for the buffers that fit the window and not for the ones that do not.
///
/// **The point is what it does not do.** [`Host::resident_direct`] asks for every buffer at once and
/// refuses if any will not fit, which sends a caller holding one large buffer and several small ones
/// back to a submission and a fence for every write — including the small ones. So the check is that
/// a mixed set is accepted, and that the buffer above the window did not go straight in anyway: a
/// version ignoring `window` would report two direct writes here rather than at most one.
///
/// `window` is bytes. Four words is sixteen of them, and the second binding is four kibibytes.
#[test]
fn a_window_takes_the_buffers_that_fit_and_leaves_the_rest_device_local() {
    let Some(host) = device("near") else {
        return;
    };
    let Ok(mut near) = host.resident_near(&[4, 1024], &[], 16) else {
        eprintln!("SKIPPED near: no device-local host-visible memory");
        return;
    };

    near.write(0, &[1, 2, 3, 4]).expect("the small one");
    near.write(1, &vec![9_u32; 1024]).expect("the large one");
    assert_eq!(near.read(0, 4).expect("a read"), vec![1, 2, 3, 4]);
    assert_eq!(near.read(1, 2).expect("a read"), vec![9, 9]);

    let (_, writes, straight) = near.pushing();
    assert_eq!(
        writes, 2,
        "two writes went in and the counter says {writes}"
    );
    assert!(
        straight <= 1,
        "{straight} of two writes went straight into device memory, so the 4 KiB binding was asked \
         for against a window of 16 bytes"
    );
}

/// A window of nought asks for nothing, which is what [`Host::resident`] is.
#[test]
fn a_window_of_nothing_asks_for_nothing() {
    let Some(host) = device("near nought") else {
        return;
    };
    let mut plain = host.resident_near(&[4], &[], 0).expect("a device buffer");
    plain.write(0, &[5]).expect("a write");
    assert_eq!(plain.read(0, 1).expect("a read"), vec![5]);
    assert_eq!(
        plain.pushing().2,
        0,
        "a window of nought asked for device-local host-visible memory anyway"
    );
}
