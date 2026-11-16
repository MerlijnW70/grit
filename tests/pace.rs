#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use std::time::Duration;

use grit::serve3::budget::{BATCH_MOST, BORNE_LEAST, FIXED_MOVES_AFTER, Pace, REFUSED_MOST};

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
fn a_world_too_slow_for_a_short_budget_from_its_first_frame_runs_late_only_on_its_first_calls() {
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
            late.iter().all(|call_at| *call_at < FIXED_MOVES_AFTER),
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
        late.iter().all(|call_at| *call_at < FIXED_MOVES_AFTER),
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
    let mut settling = 0;
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
                    if call_at < FIXED_MOVES_AFTER {
                        settling += 1;
                    } else {
                        late += 1;
                    }
                }
            }
        }
    }
    let settled = (seeds * u64::try_from(calls - FIXED_MOVES_AFTER).unwrap()).max(1);
    let first = seeds * u64::try_from(FIXED_MOVES_AFTER).unwrap();
    assert!(
        late * 50 <= settled && settling < first,
        "a frame of {:?} with up to 3% jitter per batch against a {budget:?} budget: {late} of {settled} settled short calls ran, and {settling} of the first {first}",
        device.cost(1)
    );
}

#[test]
fn a_call_is_refused_only_when_both_of_the_last_two_batches_outlasted_its_budget() {
    let mut state = 3_u64;
    let mut next = move |most: u64| {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (state >> 33) % most
    };
    let mut refusals = 0;
    for _ in 0..200 {
        let mut pace = Pace::first(next(1 << 24));
        let mut recent = [Duration::MAX; 2];
        let mut device = Device::new(next(30_000), next(30_000));
        for _ in 0..60 {
            if next(10) == 0 {
                device = Device::new(next(30_000), next(30_000));
            }
            let budget = Duration::from_micros(1 + next(200_000));
            let frames = 1 + u32::try_from(next(70_000)).unwrap();
            let mut left = budget;
            let mut done = 0;
            while done < frames {
                let step = if done == 0 {
                    pace.opening(frames - done, left)
                } else {
                    pace.batch(frames - done, left)
                };
                if step == 0 {
                    if done == 0 {
                        refusals += 1;
                        assert!(
                            recent.iter().all(|spent| *spent > left),
                            "refused with {left:?} left after batches of {recent:?}; {pace:?}"
                        );
                    }
                    break;
                }
                let held = if next(8) == 0 {
                    Duration::from_micros(next(300_000))
                } else {
                    Duration::ZERO
                };
                let took = device.cost(step) + held;
                pace.ran(step, took);
                recent = [took, recent[0]];
                left = left.saturating_sub(took);
                done += step;
            }
        }
    }
    assert!(refusals > 100, "{refusals}");
}

#[test]
fn a_world_that_turns_fast_refuses_a_short_budget_only_a_bounded_number_of_times() {
    let slow = Device::new(50_000, 1_000);
    let fast = Device::new(100, 1_000);
    assert!(fast.cost(BATCH_MOST) > SHORT && fast.cost(1) < SHORT);
    for shape in SHAPES {
        let mut pace = Pace::first(32 * 32 * 32);
        for _ in 0..8 {
            called(&mut pace, slow, 4096, LONG);
        }
        let mut refused = Vec::new();
        for call_at in 0..48 {
            let frames = shape[call_at % shape.len()];
            called(&mut pace, fast, frames, LONG);
            let (ran, _) = called(&mut pace, fast, 65536, SHORT);
            if ran == 0 {
                refused.push(call_at);
            }
        }
        let most = usize::try_from(REFUSED_MOST).unwrap() * FIXED_MOVES_AFTER;
        assert!(
            refused.len() <= most,
            "a world whose fixed cost fell from {:?} to {:?}, a long client stepping {shape:?}: short calls {refused:?} of 48 were refused; {pace:?}",
            slow.fixed,
            fast.fixed
        );
    }
}

#[test]
fn a_cold_world_runs_its_first_call_within_its_budget_and_one_frame() {
    for (fixed_us, each_us) in [
        (0_u64, 900_u64),
        (100, 800),
        (100, 1_300),
        (300, 20),
        (1_000, 0),
        (4_600, 2_250),
        (20_000, 30_000),
    ] {
        let device = Device::new(fixed_us, each_us);
        for budget_ms in [2_u64, 5, 30, 97, 511] {
            let budget = Duration::from_millis(budget_ms);
            let mut pace = Pace::first(32 * 32 * 32);
            let (_, spent) = called(&mut pace, device, 65536, budget);
            assert!(
                spent <= budget + device.cost(1),
                "{device:?}, {budget:?}: a cold world's first call spent {spent:?}; {pace:?}"
            );
        }
    }
}

#[test]
fn a_held_up_batch_in_every_split_so_far_never_refuses_a_frame_that_fits() {
    let device = Device::new(100, 500);
    let short = Duration::from_millis(2);
    let held = Duration::from_millis(5);
    assert!(device.cost(1) < short && held > short && device.cost(31) > short);
    for sizes in [&[1_u32, 31][..], &[31, 1, 31][..]] {
        let mut pace = Pace::first(512 * 512 * 512);
        for &frames in sizes {
            let took = if frames == 1 {
                held
            } else {
                device.cost(frames)
            };
            pace.ran(frames, took);
        }
        assert!(
            pace.fixed() > short,
            "the held-up frame is in every split: {pace:?}"
        );
        for call in 0..3 {
            let step = pace.opening(1, short);
            assert!(
                step > 0,
                "batches {sizes:?} with the one frame held up to {held:?}: short call {call} refused; {pace:?}"
            );
            pace.ran(step, device.cost(step));
        }
    }
}

#[test]
fn a_cold_replay_reaches_full_batches_after_two_whatever_its_first_frame_cost() {
    let warm = Device::new(100, 16);
    let frames = 400_u32;
    let full = frames.div_ceil(BATCH_MOST);
    for first_ms in [1_u64, 5, 30, 200] {
        let mut pace = Pace::first(0);
        let mut done = 0;
        let mut sizes = Vec::new();
        while done < frames {
            let step = if done == 0 {
                pace.resumed(frames - done, Duration::from_secs(600))
            } else {
                pace.batch(frames - done, Duration::from_secs(600))
            };
            let took = if sizes.is_empty() {
                Duration::from_millis(first_ms)
            } else {
                warm.cost(step)
            };
            pace.ran(step, took);
            sizes.push(step);
            done += step;
        }
        assert!(
            u32::try_from(sizes.len()).unwrap() <= full + 2,
            "a first frame of {first_ms} ms, then frames of {:?}: {} batches {sizes:?} for {frames} frames",
            warm.cost(1),
            sizes.len()
        );
    }
}

struct Rng(u64);

impl Rng {
    fn next(&mut self, most: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        if most == 0 { 0 } else { (self.0 >> 33) % most }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Turn {
    Faster,
    Slower,
}

#[derive(Debug, Default)]
struct Fuzzed {
    refused: u64,
    longest: u32,
    late: u64,
    over: u64,
}

fn fuzzed(turn: Turn, ramp_on: bool) -> Fuzzed {
    let mut rng = Rng(12_345);
    let mut seen = Fuzzed::default();
    for _ in 0..400 {
        let mut pace = Pace::first(rng.next(1 << 24));
        let mut device = Device::new(rng.next(30_000), 1 + rng.next(10_000));
        let ramp = if ramp_on {
            Duration::from_micros(rng.next(10_000))
        } else {
            Duration::ZERO
        };
        let short = Duration::from_micros(1 + rng.next(60_000));
        let mut run = 0;
        for _ in 0..80 {
            if rng.next(20) == 0 {
                let fixed = Duration::from_micros(rng.next(30_000));
                rng.next(10_000);
                let moved = match turn {
                    Turn::Faster => fixed < device.fixed,
                    Turn::Slower => fixed > device.fixed,
                };
                if moved {
                    device.fixed = fixed;
                }
            }
            let is_short = rng.next(2) == 0;
            let (frames, budget) = if is_short {
                (
                    [1_u32, 65536, 3][usize::try_from(rng.next(3)).unwrap()],
                    short,
                )
            } else {
                (
                    [65536_u32, 64, 3, 7, 200, 1][usize::try_from(rng.next(6)).unwrap()],
                    LONG,
                )
            };
            let (mut left, mut done, mut spent) = (budget, 0_u32, Duration::ZERO);
            while done < frames {
                let step = if done == 0 {
                    pace.opening(frames - done, left)
                } else {
                    pace.batch(frames - done, left)
                };
                if step == 0 {
                    break;
                }
                let took = device.cost(step) + if done == 0 { ramp } else { Duration::ZERO };
                pace.ran(step, took);
                left = left.saturating_sub(took);
                spent += took;
                done += step;
            }
            let one = device.cost(1) + ramp;
            if is_short && one <= budget {
                if done == 0 {
                    seen.refused += 1;
                    run += 1;
                    seen.longest = seen.longest.max(run);
                } else {
                    run = 0;
                }
            }
            if is_short && spent > budget {
                seen.over += 1;
            }
            if spent > budget + one {
                seen.late += 1;
            }
        }
    }
    seen
}

const MAIN_REFUSED: [u64; 2] = [25, 18];

const MAIN_OVER: [u64; 4] = [3056, 4170, 4860, 5587];

#[test]
fn a_world_whose_fixed_cost_turns_runs_no_call_late_and_misjudges_fewer_short_calls_than_per_frame_figures()
 {
    for (at, (turn, ramp)) in [
        (Turn::Faster, false),
        (Turn::Faster, true),
        (Turn::Slower, false),
        (Turn::Slower, true),
    ]
    .into_iter()
    .enumerate()
    {
        let seen = fuzzed(turn, ramp);
        let main_refused = if turn == Turn::Faster {
            MAIN_REFUSED[usize::from(ramp)]
        } else {
            0
        };
        assert!(
            seen.late == 0
                && seen.longest <= 2
                && seen.refused + seen.over <= main_refused + MAIN_OVER[at]
                && (ramp || seen.refused <= main_refused),
            "fixed costs {turn:?}, ramp {ramp}: {seen:?}; per-frame figures refused {main_refused} and ran {} over budget",
            MAIN_OVER[at]
        );
    }
}

#[test]
fn a_batch_before_any_split_never_outlasts_the_time_left() {
    let device = Device::new(20_000, 1_000);
    let mut pace = Pace::first(32 * 32 * 32);
    for _ in 0..2 {
        pace.ran(BATCH_MOST, device.cost(BATCH_MOST));
    }
    for budget_ms in [5_u64, 21, 30, 60, 84] {
        let budget = Duration::from_millis(budget_ms);
        let step = pace.batch(65536, budget);
        assert!(
            step == 0 || device.cost(step) <= budget,
            "batches of 64 at {:?}, no split yet: {step} frames on {budget:?} cost {:?}; {pace:?}",
            device.cost(BATCH_MOST),
            device.cost(step)
        );
    }
}

#[test]
fn a_batch_that_fit_predicts_that_it_fits_again_whatever_fixed_cost_the_splits_keep() {
    let slow = Device::new(20_000, 1_000);
    let fast = Device::new(4_000, 1_000);
    let mut pace = Pace::first(0);
    for _ in 0..3 {
        pace.ran(1, slow.cost(1));
        pace.ran(4, slow.cost(4));
    }
    pace.ran(2, fast.cost(2));
    assert!(
        pace.fixed() == slow.fixed && pace.fixed() > fast.cost(2),
        "the splits keep a fixed cost above the last batch's time: {pace:?}"
    );
    assert_eq!(pace.cost(2), fast.cost(2), "{pace:?}");
    assert!(pace.cost(1) <= fast.cost(2), "{pace:?}");
}

#[test]
fn a_call_with_more_time_than_the_last_batch_took_runs_whatever_fixed_cost_the_splits_keep() {
    let slow = Device::new(20_000, 1_000);
    let fast = Device::new(4_000, 1_000);
    let budget = Duration::from_millis(10);
    let mut pace = Pace::first(0);
    for _ in 0..3 {
        pace.ran(1, slow.cost(1));
        pace.ran(4, slow.cost(4));
    }
    pace.ran(2, fast.cost(2));
    assert!(
        pace.fixed() > budget && fast.cost(2) < budget,
        "the splits keep a fixed cost above the budget: {pace:?}"
    );
    let step = pace.batch(65536, budget);
    assert!(
        step > 0 && fast.cost(step) <= budget,
        "a batch of 2 took {:?}: {step} frames on {budget:?}; {pace:?}",
        fast.cost(2)
    );
    let opened = pace.opening(65536, budget);
    assert!(
        opened > 0 && fast.cost(opened) <= budget,
        "a batch of 2 took {:?}: the call opened with {opened} frames on {budget:?}; {pace:?}",
        fast.cost(2)
    );
}

#[test]
fn a_call_whose_frames_rose_up_to_eightfold_since_the_last_call_runs_within_its_budget_or_its_least_opening()
 {
    let before = Device::new(2_000, 1_000);
    let mut late = Vec::new();
    for shape in SHAPES {
        for rise in [1_u32, 2, 4, 8] {
            let after = Device {
                fixed: before.fixed,
                each: before.each * rise,
            };
            for budget_ms in [20_u64, 50, 100, 300] {
                let budget = Duration::from_millis(budget_ms);
                let mut pace = Pace::first(64 * 64 * 64);
                for call_at in 0..6 {
                    called(&mut pace, before, shape[call_at % shape.len()], LONG);
                }
                let (_, spent) = called(&mut pace, after, 65536, budget);
                if spent > budget.max(after.cost(BORNE_LEAST)) {
                    late.push(format!(
                        "{shape:?}, frames {rise}x slower, {budget:?}: {spent:?}"
                    ));
                }
            }
        }
    }
    assert!(late.is_empty(), "{late:#?}");
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Turns {
    Both,
    FixedFaster,
    FixedSlower,
    Steady,
    EachFaster,
    EachSlower,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Seen {
    late: u64,
    over_us: u64,
    refused: u64,
}

fn wide(turns: Turns, ramp_on: bool, seeds: &[u64]) -> Seen {
    let mut seen = Seen::default();
    for &seed in seeds {
        let mut rng = Rng(seed);
        for _ in 0..400 {
            let mut pace = Pace::first(rng.next(1 << 24));
            let mut device = Device::new(rng.next(30_000), 1 + rng.next(10_000));
            let ramp = if ramp_on {
                Duration::from_micros(rng.next(10_000))
            } else {
                Duration::ZERO
            };
            let short = Duration::from_micros(1 + rng.next(60_000));
            for _ in 0..80 {
                if rng.next(20) == 0 {
                    let other = Device::new(rng.next(30_000), 1 + rng.next(10_000));
                    match turns {
                        Turns::Both => device = other,
                        Turns::FixedFaster if other.fixed < device.fixed => {
                            device.fixed = other.fixed;
                        }
                        Turns::FixedSlower if other.fixed > device.fixed => {
                            device.fixed = other.fixed;
                        }
                        Turns::EachFaster if other.each < device.each => {
                            device.each = other.each;
                        }
                        Turns::EachSlower if other.each > device.each => {
                            device.each = other.each;
                        }
                        _ => {}
                    }
                }
                let is_short = rng.next(2) == 0;
                let (frames, budget) = if is_short {
                    (
                        [1_u32, 65536, 3][usize::try_from(rng.next(3)).unwrap()],
                        short,
                    )
                } else {
                    (
                        [65536_u32, 64, 3, 7, 200, 1][usize::try_from(rng.next(6)).unwrap()],
                        LONG,
                    )
                };
                let (mut left, mut done, mut spent) = (budget, 0_u32, Duration::ZERO);
                while done < frames {
                    let step = if done == 0 {
                        pace.opening(frames - done, left)
                    } else {
                        pace.batch(frames - done, left)
                    };
                    if step == 0 {
                        break;
                    }
                    let took = device.cost(step) + if done == 0 { ramp } else { Duration::ZERO };
                    pace.ran(step, took);
                    left = left.saturating_sub(took);
                    spent += took;
                    done += step;
                }
                let one = device.cost(1) + ramp;
                if is_short && one <= budget && done == 0 {
                    seen.refused += 1;
                }
                if spent > budget + one {
                    seen.late += 1;
                    seen.over_us += u64::try_from((spent - budget - one).as_micros()).unwrap();
                }
            }
        }
    }
    seen
}

struct Bound {
    turns: Turns,
    ramp: bool,
    before_f3: (u64, u64),
    f3: Seen,
}

const fn bound(turns: Turns, ramp: bool, before_f3: (u64, u64), f3: (u64, u64, u64)) -> Bound {
    Bound {
        turns,
        ramp,
        before_f3,
        f3: Seen {
            late: f3.0,
            over_us: f3.1,
            refused: f3.2,
        },
    }
}

fn held_to(bounds: &[Bound], seeds: &[u64]) {
    let mut worse = Vec::new();
    for one in bounds {
        let seen = wide(one.turns, one.ramp, seeds);
        println!(
            "{:?} ramp {}: late {} over {} us refused {}; before F3 {:?}, F3 {:?}",
            one.turns, one.ramp, seen.late, seen.over_us, seen.refused, one.before_f3, one.f3
        );
        if seen.late > one.before_f3.0.min(one.f3.late)
            || seen.over_us > one.before_f3.1.min(one.f3.over_us)
            || seen.refused > one.f3.refused
        {
            worse.push(format!(
                "costs turning {:?}, ramp {}: {seen:?}; before F3 late {} over {} us; F3 {:?}",
                one.turns, one.ramp, one.before_f3.0, one.before_f3.1, one.f3
            ));
        }
    }
    assert!(worse.is_empty(), "{worse:#?}");
}

#[test]
fn the_reviews_fuzz_runs_no_more_calls_late_than_before_f3_or_f3_and_refuses_no_more_than_f3() {
    held_to(
        &[
            bound(Turns::Both, false, (18, 814_197), (27, 1_803_413, 48)),
            bound(Turns::Both, true, (21, 1_473_526), (17, 1_933_968, 35)),
            bound(Turns::FixedFaster, false, (0, 0), (0, 0, 25)),
            bound(Turns::FixedFaster, true, (0, 0), (0, 0, 22)),
            bound(Turns::FixedSlower, false, (0, 0), (0, 0, 0)),
            bound(Turns::FixedSlower, true, (0, 0), (0, 0, 0)),
            bound(Turns::Steady, false, (0, 0), (0, 0, 0)),
            bound(Turns::Steady, true, (0, 0), (0, 0, 0)),
        ],
        &[12_345],
    );
}

const TEN: [u64; 10] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];

#[test]
fn a_wide_fuzz_whose_fixed_cost_turns_runs_no_more_calls_late_than_before_f3_or_f3_and_refuses_no_more_than_f3()
 {
    held_to(
        &[
            bound(Turns::Both, false, (219, 9_982_679), (276, 16_440_455, 460)),
            bound(Turns::Both, true, (183, 9_922_767), (175, 15_997_944, 383)),
            bound(Turns::FixedFaster, false, (2, 329_579), (0, 0, 206)),
            bound(Turns::FixedFaster, true, (0, 0), (0, 0, 156)),
            bound(Turns::FixedSlower, false, (2, 329_579), (0, 0, 0)),
            bound(Turns::FixedSlower, true, (0, 0), (0, 0, 0)),
            bound(Turns::Steady, false, (2, 329_579), (0, 0, 0)),
            bound(Turns::Steady, true, (0, 0), (0, 0, 0)),
        ],
        &TEN,
    );
}

#[test]
fn a_wide_fuzz_whose_cost_per_frame_turns_runs_no_more_calls_late_than_before_f3_or_f3_and_refuses_no_more_than_f3()
 {
    held_to(
        &[
            bound(Turns::EachFaster, false, (2, 329_579), (0, 0, 73)),
            bound(Turns::EachFaster, true, (0, 0), (0, 0, 70)),
            bound(
                Turns::EachSlower,
                false,
                (75, 3_048_943),
                (99, 4_813_713, 0),
            ),
            bound(Turns::EachSlower, true, (71, 2_604_976), (64, 5_231_353, 0)),
        ],
        &TEN,
    );
}
