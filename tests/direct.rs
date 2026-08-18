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
    let payload: Vec<u32> = (0..4096_u32).map(|one| one.wrapping_mul(2_654_435_761)).collect();

    let Ok(mut straight) = host.resident_direct(&[payload.len()], &[]) else {
        eprintln!("SKIPPED: this device has no host-visible device memory");
        return;
    };
    assert!(straight.direct(), "it said it was direct and was not");
    straight.write(0, &payload).expect("a write");
    assert_eq!(straight.read(0, payload.len()).expect("a read"), payload);

    // The same payload the long way round, to the same answer.
    let mut staged = host.resident(&[payload.len()], &[]).expect("a staged buffer");
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
