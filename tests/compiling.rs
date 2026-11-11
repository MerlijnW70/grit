#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use common::device;

use grit::compact::Plan;
use grit::layout::{BINDINGS, Dimension, GRAND};
use grit::live;
use grit::rule3::sand::Sand;
use grit::sand::Grid;
use grit::sweep;
use grit::{Groups, Host};

const OPENS: usize = 60;

const BUILDERS: usize = 3;

/// Builds the listed engine of an empty 16x16x16 world on `host`, seeds it
/// and steps it once, and returns how many octs it lists for the next frame
/// and how many the world has. Seeding wakes every oct, one frame keeps each
/// awake, so a frame whose kernels run right lists every oct.
fn listed_after_one_frame(host: &Host) -> (u32, u32) {
    let width = host.width();
    let grid = Grid::solid(16, 16, 16).unwrap();
    let plan = Plan::over_in(
        width,
        grid.blocks().div_ceil(sweep::cells(width)),
        Dimension::Three,
    )
    .unwrap();
    let (passes, stages) = live::arranged3_by(&Sand, width, grid, &plan, false).unwrap();
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let mut held = host
        .resident(&live::sizes3(&Sand, &plan), &modules)
        .unwrap();
    live::seed(&mut held, grid, &plan).unwrap();
    let groups: Vec<&[Groups]> = stages.iter().map(Vec::as_slice).collect();
    let (_, listed) = held.staged_reading(&groups, GRAND as usize, 1).unwrap();
    (listed[0], grid.blocks())
}

/// lavapipe keeps the vector width it builds kernels for in one variable of
/// the process, and every compile stores the driver's default there for a
/// moment. A host opened while other threads build and step engines must
/// still report one width and run its own engine right: after one frame from
/// the seed, every oct is listed.
#[test]
fn hosts_opened_while_other_threads_compile_run_their_engines_right() {
    let Some(first) = device("hosts_opened_while_other_threads_compile_run_their_engines_right")
    else {
        return;
    };
    let width = first.width();
    drop(first);

    let stop = Arc::new(AtomicBool::new(false));
    let built = Arc::new(AtomicU32::new(0));
    let builders: Vec<_> = (0..BUILDERS)
        .map(|_| {
            let stop = Arc::clone(&stop);
            let built = Arc::clone(&built);
            std::thread::spawn(move || {
                let host = Host::open().unwrap().unwrap();
                while !stop.load(Ordering::SeqCst) {
                    let _ = listed_after_one_frame(&host);
                    built.fetch_add(1, Ordering::SeqCst);
                }
            })
        })
        .collect();

    let mut odd = Vec::new();
    for at in 0..OPENS {
        let opened = Host::open().unwrap().unwrap();
        let (listed, octs) = listed_after_one_frame(&opened);
        if opened.width() != width || listed != octs {
            odd.push((at, opened.width(), listed, octs));
        }
    }
    stop.store(true, Ordering::SeqCst);
    for builder in builders {
        builder.join().unwrap();
    }

    println!(
        "{OPENS} hosts opened at width {width} while {BUILDERS} threads built {} engines",
        built.load(Ordering::SeqCst)
    );
    assert!(
        odd.is_empty(),
        "hosts that did not run right, as (open, width, octs listed, octs): {odd:?}"
    );
}

fn ticking(host: &Host) -> grit::Resident<'_> {
    let tick = live::tick(host.width()).unwrap();
    host.resident(&[4; BINDINGS as usize], &[&tick]).unwrap()
}

#[test]
fn a_resident_without_kernels_is_built_and_holds_its_words() {
    let Some(host) = device("a_resident_without_kernels_is_built_and_holds_its_words") else {
        return;
    };
    let mut held = host.resident(&[4, 2], &[]).unwrap();
    held.write(0, &[1, 2, 3, 4]).unwrap();
    held.write(1, &[5, 6]).unwrap();
    assert_eq!(held.read(0, 4).unwrap(), vec![1, 2, 3, 4]);
    assert_eq!(held.read(1, 2).unwrap(), vec![5, 6]);
}

#[test]
fn a_host_opens_while_a_frame_of_another_host_is_in_flight() {
    let Some(host) = device("a_host_opens_while_a_frame_of_another_host_is_in_flight") else {
        return;
    };
    let mut held = ticking(&host);
    let ticket = held.submit_reading(&[&[Groups::Fixed(1)]], 0, 1).unwrap();
    let opened = Host::open().unwrap().unwrap();
    assert_eq!(opened.width(), host.width());
    let (_, ticked) = held.wait_reading(ticket).unwrap();
    assert_eq!(ticked, vec![1]);
}

#[test]
fn a_resident_is_built_while_another_on_its_host_has_a_frame_in_flight() {
    let Some(host) = device("a_resident_is_built_while_another_on_its_host_has_a_frame_in_flight")
    else {
        return;
    };
    let mut first = ticking(&host);
    let ticket = first.submit_reading(&[&[Groups::Fixed(1)]], 0, 1).unwrap();
    let mut second = ticking(&host);
    let (_, ticked) = second.frame_reading(&[Groups::Fixed(1)], 0, 1).unwrap();
    assert_eq!(ticked, vec![1]);
    let (_, ticked) = first.wait_reading(ticket).unwrap();
    assert_eq!(ticked, vec![1]);
}

#[test]
fn hosts_opened_from_many_threads_at_once_all_open_at_one_width() {
    let Some(first) = device("hosts_opened_from_many_threads_at_once_all_open_at_one_width") else {
        return;
    };
    let width = first.width();
    let opening: Vec<_> = (0..4)
        .map(|_| {
            std::thread::spawn(|| {
                (0..5)
                    .map(|_| Host::open().unwrap().unwrap().width())
                    .collect::<Vec<u32>>()
            })
        })
        .collect();
    for thread in opening {
        assert_eq!(thread.join().unwrap(), vec![width; 5]);
    }
}

#[cfg(feature = "faults")]
#[test]
fn a_build_that_fails_leaves_hosts_free_to_open() {
    use grit::resident::faults::{self, Step};

    let Some(host) = device("a_build_that_fails_leaves_hosts_free_to_open") else {
        return;
    };
    faults::reset();
    let tick = live::tick(host.width()).unwrap();
    for step in [Step::Pipeline, Step::Fence] {
        faults::arm(step, 0);
        assert!(host.resident(&[4; BINDINGS as usize], &[&tick]).is_err());
        assert!(faults::fired());
        faults::disarm();
        let opened = Host::open().unwrap().unwrap();
        assert_eq!(opened.width(), host.width());
    }
    let mut held = ticking(&host);
    let (_, ticked) = held.frame_reading(&[Groups::Fixed(1)], 0, 1).unwrap();
    assert_eq!(ticked, vec![1]);
}
