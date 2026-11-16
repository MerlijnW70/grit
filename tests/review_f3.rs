#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use std::time::Duration;

use grit::serve3::budget::Pace;

fn affine(fixed_us: u64, each_us: u64) -> impl Fn(u32) -> Duration {
    move |frames| Duration::from_micros(fixed_us) + Duration::from_micros(each_us) * frames
}

fn called(
    pace: &mut Pace,
    frames: u32,
    budget: Duration,
    cost: &dyn Fn(u32, u32) -> Duration,
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
        let took = cost(step, u32::try_from(batches.len()).unwrap());
        pace.ran(step, took);
        batches.push(step);
        left = left.saturating_sub(took);
        spent += took;
        done += step;
    }
    (done, spent, batches)
}

#[test]
fn one_held_up_first_frame_of_a_new_world_refuses_short_calls_whose_frame_fits() {
    let device = affine(100, 500);
    let short = Duration::from_millis(2);
    assert!(device(1) < short);
    let mut pace = Pace::first(512 * 512 * 512);
    let held_first = |frames: u32, at: u32| {
        if at == 0 {
            Duration::from_millis(5)
        } else {
            device(frames)
        }
    };
    let (done, _, batches) = called(&mut pace, 32, Duration::from_secs(1), &held_first);
    assert_eq!(done, 32, "{batches:?}");
    let mut refused = Vec::new();
    for at in 0..3 {
        let (ran, _, _) = called(&mut pace, 1, short, &|frames, _| device(frames));
        if ran == 0 {
            refused.push(at);
        }
    }
    assert!(
        refused.is_empty(),
        "one frame costs {:?} against a {short:?} budget, but after one held-up first frame \
         (batches {batches:?}) short calls {refused:?} were refused; fixed {:?}; {pace:?}",
        device(1),
        pace.fixed()
    );
}

#[test]
fn the_second_batch_of_a_new_world_with_a_small_fixed_cost_overruns_by_far_more_than_a_frame() {
    for (fixed_us, each_us, budget_ms) in [(100_u64, 1_300_u64, 30_u64), (100, 800, 5)] {
        let device = affine(fixed_us, each_us);
        let budget = Duration::from_millis(budget_ms);
        let mut pace = Pace::first(32 * 32 * 32);
        let (_, spent, batches) = called(&mut pace, 65536, budget, &|frames, _| device(frames));
        assert!(
            spent <= budget + device(1),
            "frames of {:?} on a fixed cost of {fixed_us} us, {budget:?} budget: batches {batches:?} \
             ran {spent:?}, {:?} past the budget",
            device(1),
            spent.saturating_sub(budget)
        );
    }
}
