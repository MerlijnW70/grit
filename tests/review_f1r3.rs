use std::time::Duration;

use grit::serve3::budget::{BATCH_MOST, Pace};

const SHORT: Duration = Duration::from_millis(40);

fn cost(frames: u32) -> Duration {
    Duration::from_millis(20) + Duration::from_millis(30) * frames
}

fn long_call(pace: &mut Pace) {
    let mut left = Duration::from_secs(1);
    let mut done = 0;
    let mut first = true;
    while done < 64 {
        let step = if first {
            pace.opening(64 - done, left)
        } else {
            pace.batch(64 - done, left)
        };
        first = false;
        if step == 0 {
            break;
        }
        let took = cost(step);
        pace.ran(step, took);
        left = left.saturating_sub(took);
        done += step;
    }
}

#[test]
fn a_long_client_batching_frames_keeps_short_calls_running_late_on_a_world_too_slow_for_them() {
    let mut pace = Pace::first(32 * 32 * 32);
    pace.ran(BATCH_MOST, Duration::from_micros(20) * BATCH_MOST);
    assert!(cost(1) > SHORT);
    let mut late = Vec::new();
    for call_at in 0..12 {
        long_call(&mut pace);
        let step = pace.opening(65536, SHORT);
        if step > 0 {
            let took = cost(step);
            if took > SHORT {
                late.push(call_at);
            }
            pace.ran(step, took);
        }
    }
    assert!(
        late.len() <= 5,
        "one frame costs {:?} against a {SHORT:?} budget, yet short calls {late:?} of 12 ran late; {pace:?}",
        cost(1)
    );
}
