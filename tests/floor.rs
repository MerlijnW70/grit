#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use std::time::{Duration, Instant};

use grit::{Groups, Host};
use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, U32};

const PASSES: usize = 8;

const TIMES: [u32; 6] = [1, 2, 5, 10, 20, 40];

const TRIES: usize = 9;

fn nothing(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, 1))?;
    let head = kernel.module().constant_u32(0)?;
    kernel.store_at(0, head, head)?;
    kernel.finish()
}

fn device(label: &str) -> Option<Host> {
    if std::env::var_os("CI").is_some() {
        eprintln!("SKIPPED {label}");
        return None;
    }
    match Host::open() {
        Ok(Some(host)) => Some(host),
        Ok(None) => {
            eprintln!("SKIPPED {label}");
            None
        }
        Err(why) => {
            eprintln!("SKIPPED {label} {why}");
            None
        }
    }
}

fn whole(held: &mut grit::Resident<'_>, stages: &[&[Groups]], times: u32) -> f64 {
    let mut best = Duration::MAX;
    for _ in 0..TRIES {
        let clock = Instant::now();
        held.staged_frames(stages, times).unwrap();
        best = best.min(clock.elapsed());
    }
    best.as_nanos() as f64
}

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

const SWITCHED: [usize; 6] = [1, 2, 4, 8, 16, 32];

fn switched(host: &Host, width: u32, n: usize, live: bool) -> f64 {
    let module = nothing(width).unwrap();
    let modules: Vec<&[u32]> = vec![&module; n];
    let mut held = host.resident(&[4, 3 * n], &modules).unwrap();
    let mut counts = vec![0_u32; 3 * n];
    for i in 0..n {
        counts[3 * i] = u32::from(live);
        counts[3 * i + 1] = 1;
        counts[3 * i + 2] = 1;
    }
    held.write(1, &counts).unwrap();

    let groups: Vec<Groups> = (0..n)
        .map(|i| Groups::From {
            binding: 1,
            at: 3 * i,
        })
        .collect();
    let stages: Vec<&[Groups]> = vec![&groups];

    let mut best = f64::MAX;
    for _ in 0..TRIES {
        held.staged_frames(&stages, 20).unwrap();
        if let Some(card) = held.on_device() {
            best = best.min(card.as_nanos() as f64);
        }
    }
    best
}

fn spread(host: &Host, width: u32, n: usize, writes: bool) -> f64 {
    let module = nothing(width).unwrap();
    let modules: Vec<&[u32]> = vec![&module; n];
    let mut held = host.resident(&[4, 3 * n.max(1)], &modules).unwrap();
    let mut counts = vec![0_u32; 3 * n];
    for i in 0..n {
        counts[3 * i] = 1;
        counts[3 * i + 1] = 1;
        counts[3 * i + 2] = 1;
    }
    held.write(1, &counts).unwrap();

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
        held.staged_frames(&stages, 20).unwrap();
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
    let module = nothing(width).unwrap();
    let every: Vec<&[u32]> = vec![&module; PASSES];
    let mut held = host.resident(&[1], &every).unwrap();
    held.write(0, &[0]).unwrap();

    let idle = [Groups::Fixed(0); PASSES];
    let busy = [Groups::Fixed(1); PASSES];
    let together: [&[Groups]; 1] = [&busy];
    let apart: Vec<&[Groups]> = busy.iter().map(core::slice::from_ref).collect();
    let none: Vec<&[Groups]> = idle.iter().map(core::slice::from_ref).collect();

    println!("{} {width} {PASSES}", host.name());

    let mut empty = Vec::new();
    let mut fused = Vec::new();
    let mut split = Vec::new();
    for times in TIMES {
        held.staged_frames(&none, times).unwrap();
        held.staged_frames(&together, times).unwrap();
        held.staged_frames(&apart, times).unwrap();

        let (a, b, c) = (
            whole(&mut held, &none, times),
            whole(&mut held, &together, times),
            whole(&mut held, &apart, times),
        );
        println!(
            "{times:>6} {:>10.1} {:>10.1} {:>10.1}",
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

    println!("{:.1}", empty_once / 1000.0);
    println!(
        "{:.2} {:.2} {:.2}",
        empty_each / 1000.0,
        fused_each / 1000.0,
        split_each / 1000.0
    );
    println!(
        "{:.0} {:.0}",
        (fused_each - empty_each) / PASSES as f64,
        (split_each - fused_each) / (PASSES - 1) as f64
    );
    println!(
        "{:.1}",
        (fused_once.max(split_once).max(empty_once) - fused_once.min(split_once).min(empty_once))
            / 1000.0
    );

    let mut best = Duration::MAX;
    for _ in 0..TRIES {
        let clock = Instant::now();
        let word = held.read(0, 1).unwrap();
        best = best.min(clock.elapsed());
        assert_eq!(word.len(), 1);
    }
    let asking = best.as_nanos() as f64;
    println!("{:.1} {:.1}", asking / 1000.0, empty_once / 1000.0);

    held.staged_frames(&none, 20).unwrap();
    let idle_card = held.on_device();
    held.staged_frames(&apart, 20).unwrap();
    let busy_card = held.on_device();
    println!("{idle_card:?} {busy_card:?}");
    if let (Some(idle), Some(busy)) = (idle_card, busy_card) {
        assert!(busy > idle);
    }

    assert!(empty_once > 10_000.0);
    assert!(empty_once > empty_each);
    assert!(split_each > fused_each);
    assert!(fused_each > empty_each);

    let mut off = Vec::new();
    let mut on = Vec::new();
    for count in SWITCHED {
        let dead = switched(&host, width, count, false);
        let alive = switched(&host, width, count, true);
        println!("{count:>7} {dead:>12.0} {alive:>12.0}");
        off.push((count as f64, dead));
        on.push((count as f64, alive));
    }
    let (off_once, off_each) = line(&off);
    let (on_once, on_each) = line(&on);
    println!("{off_each:.0} {on_each:.0} {off_once:.0} {on_once:.0}");

    assert!(off_each > 0.0 && on_each > 0.0);
    if off_each < on_each {
        println!("{:.0}", on_each - off_each);
    } else {
        println!("{:.0}", off_each - on_each);
    }
    assert!(off_each < (split_each - fused_each) / (PASSES - 1) as f64);

    let mut wide = Vec::new();
    let mut narrow = Vec::new();
    for count in [1_usize, 2, 4, 8, 16] {
        wide.push((count as f64, spread(&host, width, count, true)));
        narrow.push((count as f64, spread(&host, width, count, false)));
    }
    let (_, wide_each) = line(&wide);
    let (_, narrow_each) = line(&narrow);
    println!("{wide_each:.0} {narrow_each:.0}");

    assert!(wide_each > narrow_each);
    assert!(wide_each - narrow_each > on_each - off_each);
}
