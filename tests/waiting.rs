#![allow(clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::time::{Duration, Instant};

use grit::{Groups, Host};
use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, U32};

const TRIES: usize = 12;

const WORDS: usize = 32 << 20;

const PASSES: usize = 16;

const PER_LANE: u32 = 64;

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
            eprintln!("SKIPPED {label}: no Vulkan device");
            None
        }
        Err(why) => {
            eprintln!("SKIPPED {label}: {why}");
            None
        }
    }
}

fn over(width: u32) -> u32 {
    u32::try_from(WORDS / (width as usize * PER_LANE as usize)).unwrap_or(1)
}

#[test]
fn the_wait_is_time_the_host_gets_back() {
    let Some(host) = device("the_wait_is_time_the_host_gets_back") else {
        return;
    };
    let module = walk(host.width()).expect("a kernel");
    let every: Vec<&[u32]> = vec![&module; PASSES];
    let mut held = host.resident(&[WORDS, 1], &every).expect("a device buffer");
    held.write(0, &vec![1; WORDS]).expect("a fill");

    let groups = vec![Groups::Fixed(over(host.width())); PASSES];
    let stages: Vec<&[Groups]> = groups.iter().map(core::slice::from_ref).collect();

    let elsewhere = || std::thread::sleep(std::time::Duration::from_millis(4));

    held.frame(&groups).expect("a frame");
    elsewhere();

    let mut showed = false;
    let mut best = None;

    for _ in 0..TRIES {
        let clock = Instant::now();
        held.frame(&groups).expect("a frame");
        let device_alone = clock.elapsed();

        let clock = Instant::now();
        elsewhere();
        let host_alone = clock.elapsed();

        let clock = Instant::now();
        let ticket = held.submit(&stages).expect("a submission");
        elsewhere();
        held.wait(ticket).expect("a landing");
        let overlapped = clock.elapsed();

        let sum = device_alone + host_alone;
        let longer = device_alone.max(host_alone);
        let halfway = longer + (sum - longer) / 2;
        showed |= overlapped < halfway;

        if best.is_none_or(|(_, _, held): (_, _, Duration)| overlapped < held) {
            best = Some((device_alone, host_alone, overlapped));
        }
    }

    let (recorded, frames) = held.recording();
    let per_frame = recorded / u32::try_from(frames.max(1)).unwrap_or(u32::MAX);
    let (device_alone, host_alone, overlapped) = best.unwrap_or_default();
    eprintln!(
        "  best of {TRIES}: device {device_alone:?} (of which {per_frame:?} is the host recording \
         it), host {host_alone:?}, overlapped {overlapped:?}"
    );

    assert!(
        showed,
        "in {TRIES} tries the wait was never given back: the best was device {device_alone:?} and \
         host {host_alone:?} overlapping to {overlapped:?}, which is nearer their sum than the \
         longer of them"
    );
}
