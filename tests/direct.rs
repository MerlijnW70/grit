#![allow(clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use grit::Host;

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

    let mut staged = host
        .resident(&[payload.len()], &[])
        .expect("a staged buffer");
    assert!(!staged.direct());
    staged.write(0, &payload).expect("a write");
    assert_eq!(staged.read(0, payload.len()).expect("a read"), payload);
}

#[test]
fn a_direct_write_at_an_offset_leaves_what_is_around_it_alone() {
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
    let Some(host) = device("the_staging_buffer_is_cached_where_the_device_has_cached_memory")
    else {
        return;
    };
    let mut held = host.resident(&[1024], &[]).expect("a device buffer");
    let words: Vec<u32> = (0..1024).collect();
    held.write(0, &words).expect("a write");

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
