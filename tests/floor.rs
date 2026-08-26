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

/// How many indirect passes the switch sweep covers.
const SWITCHED: [usize; 6] = [1, 2, 4, 8, 16, 32];

/// The best of [`TRIES`] frames of `n` indirect passes in one stage, in device nanoseconds.
///
/// Every pass reads its own workgroup count out of binding 1, three words apart, exactly as
/// [`grit::compact::Plan::listed`] does. `live` is what the host puts there — but the recording
/// cannot tell, which is the point: the command buffer is the same either way.
fn switched(host: &Host, width: u32, n: usize, live: bool) -> f64 {
    let module = nothing(width).expect("a kernel");
    let modules: Vec<&[u32]> = vec![&module; n];
    let mut held = host
        .resident(&[4, 3 * n], &modules)
        .expect("two device buffers");
    let mut counts = vec![0_u32; 3 * n];
    for i in 0..n {
        counts[3 * i] = u32::from(live);
        counts[3 * i + 1] = 1;
        counts[3 * i + 2] = 1;
    }
    held.write(1, &counts).expect("the switches");

    let groups: Vec<Groups> = (0..n)
        .map(|i| Groups::From {
            binding: 1,
            at: 3 * i,
        })
        .collect();
    let stages: Vec<&[Groups]> = vec![&groups];

    let mut best = f64::MAX;
    for _ in 0..TRIES {
        held.staged_frames(&stages, 20).expect("frames");
        if let Some(card) = held.on_device() {
            best = best.min(card.as_nanos() as f64);
        }
    }
    best
}

/// The best of [`TRIES`] frames of `n` passes in `n` stages, in device nanoseconds.
///
/// One pass a stage, so what the slope holds is a barrier and a dispatch together. `writes` is
/// whether the pass is marked `Groups::Counting` — which is what makes `Resident` name
/// `DRAW_INDIRECT` in the barrier *after* it, and is the only difference between the two sweeps.
fn spread(host: &Host, width: u32, n: usize, writes: bool) -> f64 {
    let module = nothing(width).expect("a kernel");
    let modules: Vec<&[u32]> = vec![&module; n];
    let mut held = host
        .resident(&[4, 3 * n.max(1)], &modules)
        .expect("two device buffers");
    let mut counts = vec![0_u32; 3 * n];
    for i in 0..n {
        counts[3 * i] = 1;
        counts[3 * i + 1] = 1;
        counts[3 * i + 2] = 1;
    }
    held.write(1, &counts).expect("the switches");

    let apart: Vec<Vec<Groups>> = (0..n)
        .map(|_| {
            vec![if writes {
                Groups::Counting(1)
            } else {
                Groups::Fixed(1)
            }]
        })
        .collect();
    let stages: Vec<&[Groups]> = apart.iter().map(Vec::as_slice).collect();

    let mut best = f64::MAX;
    for _ in 0..TRIES {
        held.staged_frames(&stages, 20).expect("frames");
        if let Some(card) = held.on_device() {
            best = best.min(card.as_nanos() as f64);
        }
    }
    best
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

    // **The check that the device clock measures the device.** A submission holding nothing takes
    // 35-42 us of host time and does no work at all, so its timestamps have to say so; the same
    // submission with eight dispatches and seven barriers in it has to say more. If the two came
    // back alike, the query would be measuring the round trip -- which the host clock already did.
    held.staged_frames(&none, 20).expect("a warm");
    let idle_card = held.on_device();
    held.staged_frames(&apart, 20).expect("a warm");
    let busy_card = held.on_device();
    println!("  on the device: nothing {idle_card:?} a frame, eight in eight stages {busy_card:?}");
    if let (Some(idle), Some(busy)) = (idle_card, busy_card) {
        assert!(
            busy > idle,
            "the device clock reports {busy:?} for eight dispatches and {idle:?} for none, so it is \
             not measuring the device"
        );
    }

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

    // **What a pass costs when the device decided it should not run**, which is the one price this
    // file had never taken. `Groups::Fixed(0)` is not recorded at all — `Resident` drops it and
    // `tests/dispatch.rs` checks that it does. A `Groups::From` whose count the device wrote *is*
    // recorded, because the host does not know what the count says: the command buffer holds a
    // dispatch that turns out to launch nothing.
    //
    // It is the price of a branch not taken. The count buffer can be written by a kernel, so a
    // frame that records every path it could take and lets the device choose pays this once per
    // path per frame. Against a barrier's thousand nanoseconds that would be a few paths at most.
    //
    // **In one stage, not eight**, so what the slope holds is dispatches and not barriers.
    println!("\n  {:>7}  {:>12}  {:>12}", "passes", "off (ns)", "on (ns)");
    let mut off = Vec::new();
    let mut on = Vec::new();
    for count in SWITCHED {
        let dead = switched(&host, width, count, false);
        let alive = switched(&host, width, count, true);
        println!("  {count:>7}  {dead:>12.0}  {alive:>12.0}");
        off.push((count as f64, dead));
        on.push((count as f64, alive));
    }
    let (off_once, off_each) = line(&off);
    let (on_once, on_each) = line(&on);
    println!(
        "  a pass the device switched off is {off_each:.0} ns; one it left on {on_each:.0} ns \
         (intercepts {off_once:.0} and {on_once:.0})"
    );

    assert!(
        off_each > 0.0 && on_each > 0.0,
        "a pass costing nothing at all means the sweep measured something other than passes: \
         {off_each:.0} and {on_each:.0} ns"
    );
    assert!(
        off_each < on_each,
        "a pass the device switched off cost {off_each:.0} ns against {on_each:.0} for one it left \
         on, so what the count buffer holds is not reaching the dispatch"
    );
    assert!(
        off_each < (split_each - fused_each) / (PASSES - 1) as f64,
        "a pass the device switched off cost {off_each:.0} ns against {:.0} for a barrier, so \
         recording a path not taken is not the cheap half of the arrangement",
        (split_each - fused_each) / (PASSES - 1) as f64
    );

    // **And what it costs to let the device write that count**, which is not the dispatch but the
    // barrier behind it. `Resident` names `DRAW_INDIRECT` in the barrier after a stage holding a
    // `Groups::Counting`, because a count the device wrote has to be made visible to the commands
    // that read it. Both sweeps below run one workgroup a pass, one pass a stage; the only
    // difference is whether the pass says it wrote a count.
    //
    // It is the number that decides whether a frame can switch its own passes off. Doing so means
    // every gated count is written by a kernel, which puts one of these behind every stage that
    // writes one — so the saving of 73 ns a dispatch is set against whatever this is.
    let mut wide = Vec::new();
    let mut narrow = Vec::new();
    for count in [1_usize, 2, 4, 8, 16] {
        wide.push((count as f64, spread(&host, width, count, true)));
        narrow.push((count as f64, spread(&host, width, count, false)));
    }
    let (_, wide_each) = line(&wide);
    let (_, narrow_each) = line(&narrow);
    println!(
        "  a stage that wrote a count is {wide_each:.0} ns, one that did not {narrow_each:.0} ns"
    );

    assert!(
        wide_each > narrow_each,
        "a stage that wrote a count cost {wide_each:.0} ns against {narrow_each:.0} for one that \
         did not, so naming DRAW_INDIRECT is free and `Resident` narrows it for nothing"
    );
    assert!(
        wide_each - narrow_each > on_each - off_each,
        "widening a barrier costs {:.0} ns and switching a pass off saves {:.0}, so a frame could \
         gate its own passes and come out ahead — which `decisions/DR-0007` says it cannot",
        wide_each - narrow_each,
        on_each - off_each
    );
}
