use grit::serve3::budget::Rates;
use std::time::Duration;

#[test]
fn one_fast_replay_hides_slow_ones_from_the_refusal() {
    let mut rates = Rates::default();
    let recorded = Duration::from_secs(10);
    let journal = 1;
    rates.measure_replay(Duration::from_secs(1), Duration::from_millis(1000));
    rates.measure_journal(journal, recorded);
    for _ in 0..8 {
        rates.measure_replay(Duration::from_secs(1), Duration::from_secs(4));
        rates.measure_journal(journal, recorded * 4);
    }
    let least = rates.journal_least(journal).unwrap_or_default();
    eprintln!("replay {} least {least:?}", rates.replay());
    assert!(rates.replaying(recorded) > Duration::from_secs(30));
    assert!(least > Duration::from_secs(30), "least {least:?}");
}
