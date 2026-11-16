#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use std::time::Duration;

use grit::serve3::budget::Pace;

#[derive(Debug, Clone, Copy)]
struct Device {
    fixed: Duration,
    each: Duration,
}

impl Device {
    const fn new(fixed_us: u64, each_us: u64) -> Self {
        Self {
            fixed: Duration::from_micros(fixed_us),
            each: Duration::from_micros(each_us),
        }
    }

    fn cost(self, frames: u32) -> Duration {
        self.fixed + self.each * frames
    }
}

const LONG: Duration = Duration::from_secs(1);

fn called(
    pace: &mut Pace,
    device: Device,
    frames: u32,
    budget: Duration,
) -> (u32, Duration, Vec<u32>) {
    let mut left = budget;
    let mut done = 0;
    let mut spent = Duration::ZERO;
    let mut batches = Vec::new();
    while done < frames {
        let step = if done == 0 {
            pace.opening(frames - done, left)
        } else {
            pace.batch(frames - done, left)
        };
        if step == 0 {
            break;
        }
        let took = device.cost(step);
        pace.ran(step, took);
        batches.push(step);
        left = left.saturating_sub(took);
        spent += took;
        done += step;
    }
    (done, spent, batches)
}

fn turned_fast(slow: Device) -> Pace {
    let mut pace = Pace::first(32 * 32 * 32);
    for _ in 0..8 {
        called(&mut pace, slow, 4096, LONG);
    }
    pace
}

#[test]
fn a_world_whose_fixed_cost_drops_runs_a_short_call_a_whole_batch_past_its_budget() {
    let slow = Device::new(50_000, 1_000);
    let fast = Device::new(100, 1_000);
    let mut worst = Vec::new();
    for budget_ms in [5_u64, 10, 40] {
        let budget = Duration::from_millis(budget_ms);
        let mut pace = turned_fast(slow);
        for (at, long) in [64_u32, 3].into_iter().enumerate() {
            called(&mut pace, fast, long, LONG);
            let (_, spent, batches) = called(&mut pace, fast, 65536, budget);
            if spent > budget + fast.cost(1) {
                worst.push(format!(
                    "{budget:?} budget, call {at} after a long call of {long}: batches {batches:?} ran {spent:?}"
                ));
            }
        }
    }
    assert!(
        worst.is_empty(),
        "one frame costs {:?}, yet short calls ran past budget by more than a frame: {worst:#?}",
        fast.cost(1)
    );
}

#[test]
fn a_steady_world_whose_calls_start_with_a_slower_batch_runs_short_calls_a_whole_batch_past_their_budget()
 {
    let device = Device::new(2_000, 600);
    let start = Duration::from_millis(5);
    let budget = Duration::from_millis(10);
    let first =
        |frames: u32, at: usize| device.cost(frames) + if at == 0 { start } else { Duration::ZERO };
    assert!(first(1, 0) <= budget);
    let mut pace = Pace::first(32 * 32 * 32);
    let mut late = Vec::new();
    for call in 0..16 {
        for budget_of_call in [LONG, budget] {
            let frames = if budget_of_call == LONG { 64 } else { 65536 };
            let mut left = budget_of_call;
            let mut done = 0;
            let mut spent = Duration::ZERO;
            let mut batches = Vec::new();
            let before = pace;
            while done < frames {
                let step = if done == 0 {
                    pace.opening(frames - done, left)
                } else {
                    pace.batch(frames - done, left)
                };
                if step == 0 {
                    break;
                }
                let took = first(step, batches.len());
                pace.ran(step, took);
                batches.push(step);
                left = left.saturating_sub(took);
                spent += took;
                done += step;
            }
            if budget_of_call == budget && spent > budget + first(1, 0) {
                late.push(format!(
                    "call {call}: batches {batches:?} ran {spent:?}; before it {before:?}"
                ));
            }
        }
    }
    assert!(
        late.is_empty(),
        "a call's first frame costs at most {:?} on a {budget:?} budget, yet {} short calls ran past it by more than that frame: {late:#?}",
        first(1, 0),
        late.len()
    );
}

#[test]
fn a_world_whose_fixed_cost_drops_refuses_short_calls_whose_one_frame_fits() {
    let slow = Device::new(50_000, 1_000);
    let fast = Device::new(100, 1_000);
    let short = Duration::from_millis(10);
    assert!(fast.cost(1) < short);
    let mut pace = turned_fast(slow);
    let mut refused = Vec::new();
    for call in 0..30 {
        called(&mut pace, fast, 64, LONG);
        let (ran, _, _) = called(&mut pace, fast, 1, short);
        if ran == 0 {
            refused.push(call);
        }
    }
    assert!(
        refused.is_empty(),
        "one frame costs {:?} on a {short:?} budget, yet short calls {refused:?} were refused; {pace:?}",
        fast.cost(1)
    );
}

#[test]
fn one_held_up_first_frame_of_a_world_stepped_one_frame_at_a_time_refuses_short_calls_whose_frame_fits()
 {
    let device = Device::new(100, 500);
    let short = Duration::from_millis(2);
    let mut pace = Pace::first(32 * 32 * 32);
    assert_eq!(pace.opening(1, LONG), 1);
    pace.ran(1, Duration::from_millis(5));
    let mut refused = Vec::new();
    for call in 0..3 {
        let (ran, _, _) = called(&mut pace, device, 1, short);
        if ran == 0 {
            refused.push(call);
        }
    }
    assert!(
        refused.is_empty(),
        "one frame costs {:?} on a {short:?} budget; after one held-up first batch and no other, short calls {refused:?} were refused; {pace:?}",
        device.cost(1)
    );
}

#[test]
fn a_long_step_on_a_steady_world_keeps_full_batches_once_it_reaches_them() {
    let device = Device::new(800, 1);
    let mut pace = Pace::first(512 * 512 * 512);
    let (done, _, batches) = called(&mut pace, device, 65536, Duration::from_secs(600));
    assert_eq!(done, 65536);
    let full = batches
        .iter()
        .position(|&frames| frames == grit::serve3::budget::BATCH_MOST)
        .unwrap();
    let body = &batches[full..batches.len() - 1];
    let changed = body
        .iter()
        .filter(|&&frames| frames != grit::serve3::budget::BATCH_MOST)
        .count();
    assert_eq!(
        changed,
        0,
        "a steady world stepped 65536 frames in {} batches; {changed} batches after the first full one changed size \
         (each size change re-records the step on the GPU: a 65536-frame step on 512^3 took 966-984 ms at this head \
         against 823-828 ms at main and 819-827 ms with the cut disabled): {:?}",
        batches.len(),
        &body[..body.len().min(24)]
    );
}
