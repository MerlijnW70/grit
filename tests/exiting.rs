#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use grit::{Error, Groups, Host, Resident};
use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, U32};

const WORDS: usize = 1 << 16;

const PER_LANE: u32 = 4;

const TRIES: usize = 8;

const NAME: &str = "a_process_whose_device_wedged_exits_cleanly_whenever_it_exits";

const LINGER: &str = "GRIT_EXITING_LINGER_NANOS";

const SALT: &str = "GRIT_EXITING_SALT";

const CHILDREN: u32 = 160;

/// The walk of `tests/wedged.rs`, with `salt` added to every sum, so each
/// process compiles a kernel no driver has seen before rather than finding
/// it in an on-disk shader cache filled by an earlier process.
fn walk(subgroup: u32, salt: u32) -> Result<Vec<u32>, LaneError> {
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
    let salted = kernel.module().constant_u32(salt)?;
    running = kernel.module().i_add(uint, running, salted)?;

    let zero = kernel.module().constant_u32(0)?;
    let _ = kernel.atomic_add_at(1, zero, running)?;
    kernel.finish()
}

fn walking(host: &Host, salt: u32) -> (Resident<'_>, [Groups; 1]) {
    let module = walk(host.width(), salt).unwrap();
    let every: Vec<&[u32]> = vec![&module];
    let mut held = host.resident(&[WORDS, 1], &every).unwrap();
    held.write(0, &vec![1; WORDS]).unwrap();
    let wide = u32::try_from(WORDS / (host.width() as usize * PER_LANE as usize)).unwrap();
    (held, [Groups::Fixed(wide)])
}

fn named(key: &str) -> Option<u64> {
    std::env::var(key)
        .ok()
        .and_then(|held| held.parse::<u64>().ok())
}

/// What each child process does: wedge a host on a frame it gives no time
/// at all, drop the resident and the host, wait a while, and leave.
fn wedge_and_leave(linger: Duration, salt: u32) {
    let Some(host) = device(NAME) else {
        return;
    };
    let (mut held, groups) = walking(&host, salt);
    held.set_patience(Duration::ZERO);
    for _ in 0..TRIES {
        match held.frame(&groups) {
            Err(Error::Wedged { .. }) => break,
            Err(other) => panic!("{other:?}"),
            Ok(_) => {}
        }
    }
    drop(held);
    drop(host);
    std::thread::sleep(linger);
}

/// A wedge is a verdict on time, not proof that the work stopped: a software
/// device goes on compiling and running the frame on its own threads. A
/// process that leaves while they still run tears down what they use, and
/// the moment it leaves decides whether that lands. The first frame of a
/// kernel new to the driver, compilation included, is timed here, and the
/// children leave at even steps across that time, so that some leave at
/// each moment the frame can be in. Every one must exit cleanly.
#[test]
fn a_process_whose_device_wedged_exits_cleanly_whenever_it_exits() {
    if let (Some(nanos), Some(salt)) = (named(LINGER), named(SALT)) {
        wedge_and_leave(
            Duration::from_nanos(nanos),
            u32::try_from(salt).unwrap_or_default(),
        );
        return;
    }
    let Some(host) = device(NAME) else {
        return;
    };
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    let salt = now.subsec_nanos() ^ u32::try_from(now.as_secs() & 0xffff).unwrap();
    let (mut held, groups) = walking(&host, salt);
    let first = held.frame(&groups).unwrap();
    drop(held);
    drop(host);

    eprintln!("{NAME}: the first frame took {first:?}, and {CHILDREN} children leave across it");
    let me = std::env::current_exe().unwrap();
    let mut failed = Vec::new();
    for child in 0..CHILDREN {
        let linger = first * child / CHILDREN;
        let out = Command::new(&me)
            .args(["--exact", NAME, "--nocapture", "--test-threads=1"])
            .env(LINGER, linger.as_nanos().to_string())
            .env(SALT, salt.wrapping_add(child + 1).to_string())
            .output()
            .unwrap();
        if !out.status.success() {
            let said = String::from_utf8_lossy(&out.stderr);
            let last = said.lines().last().unwrap_or_default().to_owned();
            failed.push(format!("{linger:?}: {} {last}", out.status));
        }
    }
    assert!(
        failed.is_empty(),
        "{} of {CHILDREN} processes did not exit cleanly: {failed:?}",
        failed.len()
    );
}
