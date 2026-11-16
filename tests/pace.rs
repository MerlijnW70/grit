#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use std::time::Duration;

use grit::serve3::budget::{BATCH_MOST, FIXED_MOVES_AFTER, Pace, REFUSED_MOST};

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

const FAST: Device = Device::new(300, 20);

const SHORT: Duration = Duration::from_millis(40);

const LONG: Duration = Duration::from_secs(1);

fn called(pace: &mut Pace, device: Device, frames: u32, budget: Duration) -> (u32, Duration) {
    let mut left = budget;
    let mut done = 0;
    let mut spent = Duration::ZERO;
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
        left = left.saturating_sub(took);
        spent += took;
        done += step;
    }
    (done, spent)
}

fn warm() -> Pace {
    let mut pace = Pace::first(32 * 32 * 32);
    for _ in 0..4 {
        called(&mut pace, FAST, 1024, LONG);
    }
    pace
}

const SHAPES: [&[u32]; 6] = [&[65536], &[64], &[3], &[2, 7], &[5, 65536, 1, 64], &[1]];

#[test]
fn a_world_whose_one_frame_outlasts_a_short_budget_is_refused_up_front_however_a_long_client_batches()
 {
    let slow = Device::new(20_000, 30_000);
    assert!(slow.cost(1) > SHORT && slow.cost(3) / 3 < SHORT);
    for shape in SHAPES {
        let mut pace = warm();
        let mut late = Vec::new();
        for call_at in 0..24 {
            let frames = shape[call_at % shape.len()];
            called(&mut pace, slow, frames, LONG);
            let (ran, spent) = called(&mut pace, slow, 65536, SHORT);
            if ran > 0 {
                assert!(spent > SHORT, "{shape:?} call {call_at}: {pace:?}");
                late.push(call_at);
            }
        }
        assert!(
            late.iter().all(|call_at| *call_at < FIXED_MOVES_AFTER),
            "a world whose one frame costs {:?} against a {SHORT:?} budget, a long client stepping {shape:?}: short calls {late:?} of 24 ran late; {pace:?}",
            slow.cost(1)
        );
    }
}

#[test]
fn a_world_too_slow_for_a_short_budget_from_its_first_frame_runs_late_on_its_first_call_at_most() {
    let slow = Device::new(20_000, 30_000);
    for shape in SHAPES {
        let mut pace = Pace::first(32 * 32 * 32);
        let mut late = Vec::new();
        for call_at in 0..24 {
            let frames = shape[call_at % shape.len()];
            called(&mut pace, slow, frames, LONG);
            let (ran, _) = called(&mut pace, slow, 65536, SHORT);
            if ran > 0 {
                late.push(call_at);
            }
        }
        assert!(
            late.is_empty() || late == [0],
            "{shape:?}: short calls {late:?} ran late; {pace:?}"
        );
    }
}

#[test]
fn a_single_client_on_a_world_too_slow_for_it_measures_one_frame_every_third_call_once_settled() {
    let slow = Device::new(20_000, 30_000);
    let mut pace = warm();
    called(&mut pace, slow, 64, LONG);
    let mut probed = Vec::new();
    for call_at in 0..12 {
        let (ran, _) = called(&mut pace, slow, 65536, SHORT);
        if ran > 0 {
            assert_eq!(ran, 1, "call {call_at}: {pace:?}");
            probed.push(call_at);
        }
    }
    let every = usize::try_from(REFUSED_MOST).unwrap() + 1;
    let settled = probed
        .iter()
        .position(|at| *at >= FIXED_MOVES_AFTER)
        .unwrap();
    assert!(
        probed.first() == Some(&0)
            && probed
                .get(settled..)
                .unwrap()
                .windows(2)
                .all(|pair| pair[1] - pair[0] == every),
        "short calls {probed:?} of 12 measured a frame; {pace:?}"
    );
}

#[test]
fn a_world_whose_one_frame_fits_is_never_refused_however_large_the_long_clients_batches() {
    let heavy = Device::new(25_000, 20);
    assert!(heavy.cost(BATCH_MOST) > Duration::from_millis(25) && heavy.cost(1) < SHORT);
    for shape in SHAPES {
        let mut pace = warm();
        for call_at in 0..24 {
            let frames = shape[call_at % shape.len()];
            called(&mut pace, heavy, frames, LONG);
            let (ran, spent) = called(&mut pace, heavy, 65536, SHORT);
            assert!(
                ran > 0 && (spent <= SHORT || call_at < FIXED_MOVES_AFTER),
                "a world whose one frame costs {:?}, a long client stepping {shape:?}: short call {call_at} ran {ran} frames in {spent:?} of {SHORT:?}; {pace:?}",
                heavy.cost(1)
            );
        }
    }
}

#[test]
fn every_batch_of_a_call_stays_inside_its_budget_once_both_costs_are_measured() {
    for device in [
        Device::new(300, 20),
        Device::new(4_000, 2_000),
        Device::new(25_000, 20),
        Device::new(20_000, 30_000),
        Device::new(0, 1_000),
        Device::new(10_000, 0),
    ] {
        let mut pace = Pace::first(32 * 32 * 32);
        called(&mut pace, device, 1024, Duration::from_secs(30));
        for budget_ms in [41_u64, 97, 250, 1000] {
            let budget = Duration::from_millis(budget_ms);
            for _ in 0..4 {
                let (_, spent) = called(&mut pace, device, 65536, budget);
                assert!(
                    spent <= budget || spent <= device.cost(1),
                    "{device:?}, {budget:?}: a call spent {spent:?}; {pace:?}"
                );
            }
        }
    }
}

#[test]
fn a_first_batch_runs_one_frame_whatever_the_guess() {
    let slow = Device::new(1_000, 20_000);
    for cells in [0_u64, 32 * 32 * 32, 512 * 512 * 512] {
        let mut pace = Pace::first(cells);
        assert_eq!(
            pace.opening(65536, Duration::from_millis(511)),
            1,
            "{pace:?}"
        );
        let (_, spent) = called(
            &mut Pace::first(cells),
            slow,
            65536,
            Duration::from_millis(511),
        );
        assert!(
            spent <= Duration::from_millis(511) + slow.cost(1),
            "{cells} cells: a 511 ms call on frames of {:?} ran {spent:?}",
            slow.cost(1)
        );
        pace.ran(1, slow.cost(1));
        let aim = Duration::from_millis(grit::serve3::budget::SUBMIT_MOST_MS / 2);
        let second = pace.batch(65536, Duration::from_secs(1));
        assert!(
            second > 1 && pace.cost(second) <= aim && pace.cost(second + 1) > aim,
            "the second batch is the most the measured frame predicts fits {aim:?}: {second}, {pace:?}"
        );
    }
}

#[test]
fn one_held_up_batch_never_raises_the_fixed_cost_past_a_frame_that_fits() {
    let device = Device::new(4_000, 1_500);
    assert!(device.cost(BATCH_MOST) > SHORT && device.cost(1) < SHORT);
    let held = device.cost(BATCH_MOST) - Duration::from_millis(1);
    for frames in [1_u32, 2, 4, 17, 31] {
        let mut pace = Pace::first(32 * 32 * 32);
        called(&mut pace, device, 4096, Duration::from_secs(30));
        pace.ran(frames, held);
        for _ in 0..2 {
            pace.ran(BATCH_MOST, device.cost(BATCH_MOST));
        }
        assert!(
            pace.opening(65536, SHORT) > 0,
            "{frames} frames held up to {held:?}, then batches of {:?}: {pace:?}",
            device.cost(BATCH_MOST)
        );
    }
}

#[test]
fn the_fixed_cost_is_split_only_from_batches_at_least_half_again_apart_in_size() {
    let device = Device::new(4_000, 2_000);
    let mut pace = Pace::first(0);
    pace.ran(40, device.cost(40));
    pace.ran(41, device.cost(41) - Duration::from_millis(1));
    pace.ran(39, device.cost(39));
    pace.ran(58, device.cost(58));
    assert_eq!(
        pace.fixed(),
        Pace::first(0).fixed(),
        "batches less than half again apart split no fixed cost: {pace:?}"
    );
    pace.ran(87, device.cost(87));
    assert_eq!(pace.fixed(), device.fixed, "{pace:?}");
    pace.ran(20, device.cost(20));
    assert_eq!(pace.fixed(), device.fixed, "{pace:?}");
    assert_eq!(pace.cost(1), device.cost(1), "{pace:?}");
    pace.ran(80, device.cost(10));
    pace.ran(160, device.cost(80));
    assert_eq!(
        pace.fixed(),
        device.fixed,
        "a larger batch that took less time, or a negative fixed cost, splits nothing: {pace:?}"
    );
}

#[test]
fn the_fixed_cost_follows_the_device_after_a_majority_of_the_splits_it_keeps() {
    let fast = Device::new(300, 20);
    let slow = Device::new(20_000, 30_000);
    let mut pace = Pace::first(0);
    called(&mut pace, fast, 4096, LONG);
    assert_eq!(pace.fixed(), fast.fixed, "{pace:?}");
    let mut sizes = [1_u32, 2].into_iter().cycle();
    for _ in 0..FIXED_MOVES_AFTER {
        assert!(pace.fixed() < slow.fixed, "{pace:?}");
        let frames = sizes.next().unwrap();
        pace.ran(frames, slow.cost(frames));
    }
    let frames = sizes.next().unwrap();
    pace.ran(frames, slow.cost(frames));
    assert_eq!(pace.fixed(), slow.fixed, "{pace:?}");
}

#[test]
fn a_fixed_cost_split_from_jittered_batches_keeps_a_world_too_slow_for_a_short_budget_refused() {
    let device = Device::new(4_600, 2_250);
    let budget = Duration::from_millis(5);
    assert!(device.cost(1) > budget + Duration::from_millis(1));
    let mut state = 7_u64;
    let mut jitter = move || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        Duration::from_micros((state >> 33) % 1_000)
    };
    let mut pace = Pace::first(128 * 128 * 128);
    let mut late = Vec::new();
    for call_at in 0..48 {
        for (frames, room) in [(65536, LONG), (65536, budget)] {
            let mut left = room;
            let mut done = 0;
            while done < frames {
                let step = if done == 0 {
                    pace.opening(frames - done, left)
                } else {
                    pace.batch(frames - done, left)
                };
                if step == 0 {
                    break;
                }
                let took = device.cost(step) + jitter();
                pace.ran(step, took);
                left = left.saturating_sub(took);
                done += step;
            }
            if room == budget && done > 0 {
                late.push(call_at);
            }
        }
    }
    assert!(
        late.is_empty(),
        "a frame of {:?} with up to 1 ms of jitter per batch against a {budget:?} budget: short calls {late:?} of 48 ran; {pace:?}",
        device.cost(1)
    );
}

#[test]
fn two_splits_far_off_never_move_the_fixed_cost() {
    let device = Device::new(4_600, 2_250);
    let off = Duration::from_millis(3);
    let mut pace = Pace::first(0);
    for frames in [1, 2, 4, 8, 16] {
        pace.ran(frames, device.cost(frames));
    }
    pace.ran(32, device.cost(32) + off);
    pace.ran(64, device.cost(64) - off);
    assert_eq!(pace.fixed(), device.fixed, "{pace:?}");
}

#[test]
fn a_fixed_cost_split_from_batches_with_proportional_jitter_runs_late_on_few_calls() {
    let device = Device::new(4_600, 2_250);
    let budget = Duration::from_millis(5);
    let seeds = 40_u64;
    let calls = 96;
    let mut late = 0;
    for seed in 1..=seeds {
        let mut state = seed;
        let mut jittered = move |cost: Duration| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let parts = (state >> 33) % 61;
            cost * u32::try_from(970 + parts).unwrap() / 1000
        };
        let mut pace = Pace::first(128 * 128 * 128);
        for call_at in 0..calls {
            let long = [96, 65536, 64, 3][call_at % 4];
            for (frames, room) in [(long, LONG), (65536, budget)] {
                let mut left = room;
                let mut done = 0;
                while done < frames {
                    let step = if done == 0 {
                        pace.opening(frames - done, left)
                    } else {
                        pace.batch(frames - done, left)
                    };
                    if step == 0 {
                        break;
                    }
                    let took = jittered(device.cost(step));
                    pace.ran(step, took);
                    left = left.saturating_sub(took);
                    done += step;
                }
                if room == budget && done > 0 {
                    late += 1;
                }
            }
        }
    }
    let all = seeds * u64::try_from(calls).unwrap();
    assert!(
        late * 50 <= all,
        "a frame of {:?} with up to 3% jitter per batch against a {budget:?} budget: {late} of {all} short calls ran",
        device.cost(1)
    );
}
