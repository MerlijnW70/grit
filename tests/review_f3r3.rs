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

#[test]
fn a_call_after_a_one_frame_batch_grows_straight_to_a_full_batch_and_runs_far_past_its_budget_when_frames_turn_slower()
 {
    let before = Device::new(500, 70);
    let mut late = Vec::new();
    for slower in [5_u64, 10, 30] {
        for budget_ms in [10_u64, 20, 50] {
            let after = Device::new(500, 70 * slower);
            let budget = Duration::from_millis(budget_ms);
            let mut pace = Pace::first(64 * 64 * 64);
            for frames in [64_u32, 3, 64, 7, 64, 1] {
                called(&mut pace, before, frames, Duration::from_secs(1));
            }
            let (_, spent, batches) = called(&mut pace, after, 65536, budget);
            if spent > budget + after.cost(1) {
                late.push(format!(
                    "frames {slower}x slower, {budget:?} budget: batches {batches:?} ran {spent:?}, a frame is {:?}",
                    after.cost(1)
                ));
            }
        }
    }
    assert!(
        late.is_empty(),
        "main grows a batch at most twofold, so its first batch after a one-frame batch is two frames; \
         here {} of 9 short calls ran more than a frame past budget: {late:#?}",
        late.len()
    );
}
