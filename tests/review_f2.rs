use grit::serve3::budget::Rates;
use std::time::Duration;

#[test]
fn one_fast_replay_hides_slow_ones_from_the_refusal() {
    let mut rates = Rates::default();
    rates.measure_replay(Duration::from_secs(1), Duration::from_millis(1000));
    for _ in 0..8 {
        rates.measure_replay(Duration::from_secs(1), Duration::from_secs(4));
    }
    eprintln!("replay {} least {}", rates.replay(), rates.replay_least());
    let recorded = Duration::from_secs(10);
    assert!(rates.replaying(recorded) > Duration::from_secs(30));
    assert!(
        rates.replaying_least(recorded) > Duration::from_secs(30),
        "least {:?}",
        rates.replaying_least(recorded)
    );
}
