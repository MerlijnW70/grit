//! What Vulkan charges before any arithmetic happens.
//!
//! Every timing in this repository is a host clock around a submit and a fence, divided by however
//! many frames were in the submission. That is the right number for a caller — it is what a caller
//! waits — but it is a *sum* of two very different things, and nothing here has ever taken them
//! apart:
//!
//! * what a submission costs once, however much is in it;
//! * what one more frame inside it costs.
//!
//! So this varies the only thing that separates them, which is how many frames go into one
//! submission, and reads the two off the line. The intercept is the round trip; the slope is a
//! frame. Everything is a kernel that writes one word, so what is left is Vulkan.

// A test may panic — that is how it reports.
#![allow(clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::time::{Duration, Instant};

use grit::{Groups, Host};
use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, U32};

/// How many passes the probe holds.
const PASSES: usize = 8;

/// How many frames go into one submission, across the sweep.
const TIMES: [u32; 6] = [1, 2, 5, 10, 20, 40];

/// How many times each point is measured, of which the best is kept.
///
/// Contention only ever adds, so the smallest reading is the one least contaminated. The same
/// argument `tests/waiting.rs` makes at more length.
const TRIES: usize = 9;

/// A kernel that writes one word. As close to nothing as a dispatch can be and still be one.
fn nothing(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, 1))?;
    let head = kernel.module().constant_u32(0)?;
    kernel.store_at(0, head, head)?;
    kernel.finish()
}

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

/// The best of [`TRIES`] whole submissions of `times` frames, in nanoseconds.
fn whole(held: &mut grit::Resident<'_>, stages: &[&[Groups]], times: u32) -> f64 {
    let mut best = Duration::MAX;
    for _ in 0..TRIES {
        let clock = Instant::now();
        held.staged_frames(stages, times).expect("a submission");
        best = best.min(clock.elapsed());
    }
    best.as_nanos() as f64
}

/// A least-squares line through the points, as (intercept, slope) in nanoseconds.
fn line(points: &[(f64, f64)]) -> (f64, f64) {
    let n = points.len() as f64;
    let mean_x = points.iter().map(|&(x, _)| x).sum::<f64>() / n;
    let mean_y = points.iter().map(|&(_, y)| y).sum::<f64>() / n;
    let top: f64 = points
        .iter()
        .map(|&(x, y)| (x - mean_x) * (y - mean_y))
        .sum();
    let bottom: f64 = points.iter().map(|&(x, _)| (x - mean_x).powi(2)).sum();
    let slope = if bottom == 0.0 { 0.0 } else { top / bottom };
    (mean_y - slope * mean_x, slope)
}

#[test]
fn what_a_submission_costs_before_it_holds_anything() {
    let Some(host) = device("floor") else {
        return;
    };
    let width = host.width();
    let module = nothing(width).expect("a kernel");
    let every: Vec<&[u32]> = vec![&module; PASSES];
    let mut held = host.resident(&[1], &every).expect("a device buffer");
    held.write(0, &[0]).expect("a word");

    // Three shapes over the same eight passes: none of them running, all of them in one stage, and
    // all of them in a stage each. The differences are a dispatch and a barrier.
    let idle = [Groups::Fixed(0); PASSES];
    let busy = [Groups::Fixed(1); PASSES];
    let together: [&[Groups]; 1] = [&busy];
    let apart: Vec<&[Groups]> = busy.iter().map(core::slice::from_ref).collect();
    let none: Vec<&[Groups]> = idle.iter().map(core::slice::from_ref).collect();

    println!("{} — subgroup {width}, {PASSES} passes", host.name());
    println!(
        "  {:>6}  {:>12}  {:>12}  {:>12}",
        "frames", "no passes", "one stage", "eight stages"
    );

    let mut empty = Vec::new();
    let mut fused = Vec::new();
    let mut split = Vec::new();
    for times in TIMES {
        // Warm each shape, so the first point does not pay for a pipeline the driver has not seen.
        held.staged_frames(&none, times).expect("a warm");
        held.staged_frames(&together, times).expect("a warm");
        held.staged_frames(&apart, times).expect("a warm");

        let (a, b, c) = (
            whole(&mut held, &none, times),
            whole(&mut held, &together, times),
            whole(&mut held, &apart, times),
        );
        println!(
            "  {times:>6}  {:>10.1} us  {:>10.1} us  {:>10.1} us",
            a / 1000.0,
            b / 1000.0,
            c / 1000.0
        );
        empty.push((f64::from(times), a));
        fused.push((f64::from(times), b));
        split.push((f64::from(times), c));
    }

    let (empty_once, empty_each) = line(&empty);
    let (fused_once, fused_each) = line(&fused);
    let (split_once, split_each) = line(&split);

    println!(
        "\n  a submission costs {:.1} us before it holds anything",
        empty_once / 1000.0
    );
    println!(
        "  a frame of nothing adds {:.2} us, of eight dispatches {:.2} us, of eight and \
         seven barriers {:.2} us",
        empty_each / 1000.0,
        fused_each / 1000.0,
        split_each / 1000.0
    );
    println!(
        "  so a dispatch is about {:.0} ns and a barrier about {:.0} ns",
        (fused_each - empty_each) / PASSES as f64,
        (split_each - fused_each) / (PASSES - 1) as f64
    );
    println!(
        "  (the intercepts agree to within {:.1} us, which is how much to trust the fit)",
        (fused_once.max(split_once).max(empty_once) - fused_once.min(split_once).min(empty_once))
            / 1000.0
    );

    // **And what it costs to ask the device for one number**, which is the thing this whole crate
    // is arranged to avoid: `compact::prepare` exists so that a workgroup count never comes back to
    // the host. A read is a recorded copy, a submission, a fence and a map — the submission above
    // plus whatever four bytes cost — and the argument for `prepare` has always been priced at "a
    // round trip, about a hundred microseconds". That number is a round trip *that moves data*.
    let mut best = Duration::MAX;
    for _ in 0..TRIES {
        let clock = Instant::now();
        let word = held.read(0, 1).expect("one word");
        best = best.min(clock.elapsed());
        assert_eq!(word.len(), 1, "a read of one word came back with {}", word.len());
    }
    let asking = best.as_nanos() as f64;
    println!(
        "  asking the device for one word costs {:.1} us, against {:.1} us for a submission that \
         holds nothing",
        asking / 1000.0,
        empty_once / 1000.0
    );

    // **The claims, and they are shapes rather than numbers.** A number from this machine on this
    // day is not a thing to assert on; that a submission costs more than a frame inside it, and
    // that a barrier costs more than a dispatch, are.
    assert!(
        empty_once > 10_000.0,
        "a submission cost {empty_once} ns before holding anything, which is too little to be a \
         round trip — the fit has gone wrong rather than the driver"
    );
    assert!(
        empty_once > empty_each,
        "a submission costs {empty_once} ns and a frame in it {empty_each} ns, so batching frames \
         into one submission would buy nothing — which is what `Resident::frames` is for"
    );
    assert!(
        split_each > fused_each,
        "eight passes in eight stages cost {split_each} ns and in one stage {fused_each} ns, so a \
         barrier is free and `live::wired`'s grouping is pointless"
    );
    assert!(
        fused_each > empty_each,
        "eight dispatches cost {fused_each} ns and none cost {empty_each} ns, so a dispatch is free"
    );
}
