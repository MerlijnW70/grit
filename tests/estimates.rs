#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

#[cfg(feature = "faults")]
mod common;

use std::time::Duration;

use grit::serve3::budget::{
    BUDGET_MS, Basis, RECENT, REMEASURE_AFTER, Rates, WINDOW, WORLD_LEAST, Work,
};

const SIXTY_FOUR: u64 = 1_615_268;

const ONE_TWENTY_EIGHT: u64 = 12_864_164;

const TWO_FIFTY_SIX: u64 = 102_854_820;

const SPARSE_LARGE: u64 = 192_388_324;

fn budget() -> Duration {
    Duration::from_millis(BUDGET_MS)
}

#[test]
fn extreme_samples_never_panic_any_least_or_refusal() {
    let tiny = Duration::from_nanos(1);
    let mut rates = Rates::default();
    for work in Work::ALL {
        for (bytes, spent) in [
            (0, Duration::MAX),
            (u64::MAX, Duration::MAX),
            (1, tiny),
            (u64::MAX, Duration::ZERO),
            (0, Duration::ZERO),
        ] {
            rates.measure(work, bytes, spent);
        }
    }
    for (journal, recorded, spent) in [
        (0, Duration::ZERO, Duration::MAX),
        (u64::MAX, tiny, Duration::MAX),
        (1, Duration::MAX, Duration::MAX),
        (0, Duration::ZERO, tiny),
        (u64::MAX, Duration::MAX, tiny),
        (2, Duration::ZERO, Duration::ZERO),
    ] {
        rates.measure_journal(journal, spent);
        rates.measure_replay(recorded, spent);
    }
    for work in Work::ALL {
        for bytes in [0, 1, WORLD_LEAST, u64::MAX] {
            let (least, basis) = rates.least_of(work, bytes);
            assert!(least <= Duration::MAX);
            let _ = rates.estimate(work, bytes);
            let _ = (basis, rates.refuses(work, bytes));
            rates.admitted(work, bytes);
        }
    }
    for journal in [0, 1, 2, u64::MAX] {
        assert!(
            rates
                .journal_least(journal)
                .is_none_or(|least| least <= Duration::MAX)
        );
        let _ = rates.refuses_replay(journal);
        rates.admitted_replay(journal);
    }
    for recorded in [Duration::ZERO, tiny, Duration::from_secs(1), Duration::MAX] {
        let _ = rates.replaying(recorded);
    }
    let _ = rates.shown();
    let told = grit::serve3::budget::too_slow(Duration::MAX, Duration::ZERO, "work", Basis::Both);
    assert!(told.message.contains("needs at least"), "{}", told.message);
}

#[test]
fn one_held_up_build_never_refuses_a_larger_world_the_device_builds_in_time() {
    let held = Duration::from_millis(2200);
    let mut rates = Rates::default();
    rates.measure(Work::Build, SIXTY_FOUR, held);
    let scaled = rates.estimate(Work::Build, TWO_FIFTY_SIX);
    let least = rates.least(Work::Build, TWO_FIFTY_SIX);
    eprintln!("one build of 64^3 held up {held:?}: 256^3 scaled {scaled:?}, least {least:?}");
    assert!(
        scaled > budget(),
        "the sizing estimate scales a held-up build by bytes: {scaled:?}"
    );
    assert!(least >= held, "{least:?}");
    assert!(least < budget(), "{least:?}");
}

#[test]
fn held_up_builds_of_other_sizes_never_refuse_a_large_sparse_world_the_device_builds_in_time() {
    let mut rates = Rates::default();
    for (bytes, spent) in [
        (WORLD_LEAST, Duration::from_millis(10)),
        (ONE_TWENTY_EIGHT, Duration::from_millis(3550)),
        (WORLD_LEAST, Duration::from_millis(2000)),
    ] {
        rates.measure(Work::Build, bytes, spent);
    }
    let scaled = rates.estimate(Work::Build, SPARSE_LARGE);
    let least = rates.least(Work::Build, SPARSE_LARGE);
    eprintln!("large sparse world: scaled {scaled:?}, least {least:?}");
    assert!(scaled > budget(), "{scaled:?}");
    assert!(least < budget(), "{least:?}");
}

#[test]
fn a_held_up_sample_raises_the_least_only_when_the_last_three_of_its_size_are_held_up() {
    let size = 64 << 20;
    let quick = Duration::from_secs(1);
    let slow = Duration::from_secs(100);
    for work in Work::ALL {
        let mut rates = Rates::default();
        rates.measure(work, size, quick);
        for _ in 0..RECENT - 1 {
            rates.measure(work, size, slow);
            assert_eq!(rates.least(work, size), quick, "{work:?}");
        }
        rates.measure(work, size, slow);
        assert_eq!(rates.least(work, size), slow, "{work:?}");
        rates.measure(work, size, quick);
        assert_eq!(
            rates.least(work, size),
            quick,
            "{work:?}: a call of this size that just fit is never refused by evidence"
        );
    }
}

#[test]
fn one_fast_replay_never_hides_slow_ones_from_the_refusal() {
    let mut rates = Rates::default();
    rates.measure_replay(Duration::from_secs(1), Duration::from_secs(1));
    for _ in 0..WINDOW - 1 {
        rates.measure_replay(Duration::from_secs(1), Duration::from_secs(4));
    }
    let recorded = Duration::from_secs(10);
    assert!(rates.replaying(recorded) > budget());
    let journal = 7;
    rates.measure_journal(journal, recorded);
    for _ in 0..WINDOW - 1 {
        rates.measure_journal(journal, recorded * 4);
    }
    let least = rates.journal_least(journal).unwrap();
    assert!(least > budget(), "{least:?}");
}

#[test]
fn a_larger_sample_never_raises_the_least_of_a_smaller_request() {
    let cost = |bytes: u64| Duration::from_secs_f64(0.02 + 3e-9 * bytes as f64);
    let mut rates = Rates::default();
    rates.measure(Work::Build, WORLD_LEAST, cost(WORLD_LEAST));
    for _ in 0..3 {
        rates.measure(Work::Build, 400_000_000, Duration::from_secs(30));
    }
    assert_eq!(rates.least(Work::Build, WORLD_LEAST), cost(WORLD_LEAST));
    assert_eq!(
        rates.least(Work::Build, 2 * WORLD_LEAST),
        cost(WORLD_LEAST),
        "a size between two measured ones has the smaller one's evidence"
    );
}

#[test]
fn past_every_measured_size_the_least_is_the_larger_of_the_largest_smaller_sample_and_the_fastest_per_byte()
 {
    let measured = Duration::from_millis(20);
    let mut rates = Rates::default();
    rates.measure(Work::Build, WORLD_LEAST, measured);
    let huge: u64 = 4 << 30;
    let fastest = Work::Build.fastest().unwrap();
    let per_byte = Duration::from_secs_f64(fastest * huge as f64);
    assert!(per_byte > measured);
    assert_eq!(
        rates.least_of(Work::Build, huge),
        (per_byte, Basis::Fastest)
    );
    assert_eq!(
        rates.least_of(Work::Build, 2 * WORLD_LEAST),
        (measured, Basis::Measured)
    );
}

#[test]
fn a_device_slower_than_the_prior_is_refused_by_its_own_samples() {
    let per_byte = 5e-8;
    let cost = |bytes: u64| Duration::from_secs_f64(0.05 + per_byte * bytes as f64);
    let large: u64 = 800_000_000;
    let mut rates = Rates::default();
    rates.measure(Work::Build, WORLD_LEAST, cost(WORLD_LEAST));
    rates.measure(Work::Build, large, cost(large));
    assert!(cost(large) > budget());
    assert_eq!(rates.least(Work::Build, large), cost(large));
    assert!(rates.least(Work::Build, 2 * large) >= cost(large));
}

#[test]
fn unlike_work_never_shares_a_least_estimate() {
    let bytes = TWO_FIFTY_SIX;
    let slow = Duration::from_secs(100);
    for (measured, asked) in [
        (Work::Snap, Work::Save),
        (Work::Save, Work::Snap),
        (Work::Build, Work::BuildSparse),
        (Work::BuildSparse, Work::Build),
        (Work::Fork, Work::ForkSparse),
        (Work::ForkSparse, Work::Fork),
        (Work::File, Work::FileOpen),
        (Work::FileOpen, Work::File),
        (Work::Write, Work::WriteSparse),
        (Work::WriteSparse, Work::Write),
    ] {
        let fresh = Rates::default().least(asked, bytes);
        let mut rates = Rates::default();
        for _ in 0..WINDOW {
            rates.measure(measured, bytes, slow);
        }
        assert_eq!(
            rates.least(asked, bytes),
            fresh,
            "{measured:?} moved {asked:?}"
        );
        assert!(rates.least(measured, bytes) > budget(), "{measured:?}");
    }
}

#[test]
fn with_nothing_measured_only_the_fastest_per_byte_refuses() {
    for work in Work::ALL {
        let fresh = Rates::default();
        for bytes in [0, WORLD_LEAST, TWO_FIFTY_SIX] {
            let want = Duration::from_secs_f64(work.fastest().unwrap_or(0.0) * bytes as f64);
            assert_eq!(
                fresh.least_of(work, bytes),
                (want, Basis::Fastest),
                "{work:?} with nothing measured"
            );
        }
    }
    assert_eq!(Rates::default().journal_least(0), None);
    let slow = Duration::from_millis(3000);
    let mut rates = Rates::default();
    rates.measure(Work::Build, SIXTY_FOUR, slow);
    for bytes in [SIXTY_FOUR, TWO_FIFTY_SIX] {
        assert_eq!(rates.least(Work::Build, bytes), slow, "{bytes}");
    }
    assert!(rates.least(Work::Build, WORLD_LEAST) < Duration::from_millis(1));
}

#[test]
fn eight_refusals_in_a_row_let_the_next_call_run_whatever_decided_them() {
    let mut rates = Rates::default();
    for round in 0..3 {
        for refusal in 0..REMEASURE_AFTER {
            assert!(
                rates.refuses(Work::Build, TWO_FIFTY_SIX),
                "round {round}, refusal {refusal}"
            );
        }
        assert!(!rates.refuses(Work::Build, TWO_FIFTY_SIX));
    }
    rates.admitted(Work::Build, TWO_FIFTY_SIX);
    assert!(rates.refuses(Work::Fork, TWO_FIFTY_SIX));
}

#[test]
fn a_finished_sample_of_the_same_size_outranks_the_fastest_per_byte() {
    let quick = Duration::from_millis(5);
    let mut rates = Rates::default();
    rates.measure(Work::Build, TWO_FIFTY_SIX, quick);
    let fastest = Duration::from_secs_f64(Work::Build.fastest().unwrap() * TWO_FIFTY_SIX as f64);
    assert!(fastest > quick);
    assert_eq!(
        rates.least_of(Work::Build, TWO_FIFTY_SIX),
        (quick, Basis::Measured)
    );
    assert_eq!(
        rates.least_of(Work::Build, TWO_FIFTY_SIX + 1),
        (fastest, Basis::Fastest),
        "past every measured size the per-byte figure still applies"
    );
}

#[test]
fn a_journal_least_is_the_fastest_of_its_own_last_three_finished_replays() {
    let secs = Duration::from_secs;
    let mut rates = Rates::default();
    for spent in [3, 5, 4] {
        rates.measure_journal(1, secs(spent));
    }
    assert_eq!(rates.journal_least(1), Some(secs(3)));
    for _ in 0..2 {
        rates.measure_journal(1, secs(6));
    }
    assert_eq!(rates.journal_least(1), Some(secs(4)));
    rates.measure_journal(2, Duration::from_millis(1));
    assert_eq!(rates.journal_least(1), Some(secs(4)));
    assert_eq!(rates.journal_least(2), Some(Duration::from_millis(1)));
    assert_eq!(rates.journal_least(3), None);
}

#[test]
fn admitting_a_build_never_clears_the_count_of_a_replay_refusal() {
    let mut rates = Rates::default();
    for refusal in 0..REMEASURE_AFTER {
        assert!(rates.refuses_replay(9), "refusal {refusal}");
        rates.admitted(Work::Build, 9);
        rates.admitted(Work::Build, TWO_FIFTY_SIX);
    }
    assert!(!rates.refuses_replay(9));
    assert!(rates.refuses_replay(9));
    rates.admitted_replay(9);
    assert!(rates.refuses(Work::Build, 9));
}

#[test]
fn a_held_up_one_cell_write_never_refuses_a_rebuild_whose_writes_fit() {
    let journal_writes: u64 = 15_832;
    let mut rates = Rates::default();
    for spent in [1, 1, 34] {
        rates.measure(Work::Write, 16, Duration::from_micros(spent * 1000 + 300));
    }
    let scaled = rates.estimate(Work::Write, journal_writes);
    let least = rates.least(Work::Write, journal_writes);
    eprintln!("journal writes after a held-up one-cell write: scaled {scaled:?}, least {least:?}");
    assert!(scaled > budget(), "{scaled:?}");
    assert!(least < budget(), "{least:?}");
}

#[test]
fn the_prior_never_outvotes_one_measured_write_and_a_finished_slow_tail_is_refused() {
    let tail: u64 = 168_632;
    let chunk: u64 = 16 * 659;
    let per_byte = 2.8e-5;
    let mut rates = Rates::default();
    for _ in 0..3 {
        rates.measure(Work::Write, 16, Duration::from_millis(1));
    }
    rates.measure(
        Work::Write,
        chunk,
        Duration::from_secs_f64(per_byte * chunk as f64),
    );
    let scaled = rates.estimate(Work::Write, tail);
    let least = rates.least(Work::Write, tail);
    eprintln!(
        "a {tail} byte write tail after one similar chunk: scaled {scaled:?}, least {least:?}"
    );
    assert!(
        scaled >= Duration::from_secs_f64(2e-4 * tail as f64 * 0.999),
        "the sizing estimate is the padded prior: {scaled:?}"
    );
    assert!(scaled > budget(), "{scaled:?}");
    assert!(least < budget(), "{least:?}");
    rates.measure(
        Work::Write,
        tail,
        Duration::from_secs_f64(2e-4 * tail as f64),
    );
    assert!(
        rates.least(Work::Write, tail) > budget(),
        "a tail one finished replay measured as too slow is refused"
    );
}

#[test]
fn a_slow_replay_never_raises_the_least_replay_ratio_while_a_faster_one_is_recent() {
    let mut rates = Rates::default();
    let journal = 5;
    assert_eq!(rates.journal_least(journal), None);
    for _ in 0..2 {
        rates.measure_replay(Duration::from_secs(1), Duration::from_millis(1500));
        rates.measure_journal(journal, Duration::from_millis(1500));
    }
    assert_eq!(
        rates.journal_least(journal),
        Some(Duration::from_millis(1500))
    );
    rates.measure_replay(Duration::from_secs(1), Duration::from_secs(6));
    rates.measure_journal(journal, Duration::from_secs(6));
    assert!(rates.replay() >= 6.0 - 1e-9, "{}", rates.replay());
    assert_eq!(
        rates.journal_least(journal),
        Some(Duration::from_millis(1500))
    );
}

fn sparse_image(capacity: u32, upper: Option<u32>) -> grit::branch::Image {
    use grit::branch::{Image, Law, State};
    let mut table: Vec<u32> = (0..8).flat_map(|_| [u32::MAX, 0]).collect();
    if let Some(value) = upper {
        table.extend(std::iter::repeat_n(value, 8));
    }
    Image {
        law: Law::Rule {
            name: "sand".to_owned(),
        },
        params: Vec::new(),
        frame: 1,
        size: [4, 4, 4],
        state: State::Sparse {
            capacity,
            sides: [2, 2, 2],
            table,
            pool: Vec::new(),
        },
    }
}

fn snapped_in_memory(image: grit::branch::Image) -> grit::serve3::journal::Ledger {
    use grit::serve3::journal::{Ledger, Made, Snap, state_of};
    Ledger::snapped(std::sync::Arc::new(Snap {
        sum: state_of(&image),
        frame: image.frame,
        made: Made::of_image(&image),
        image: Some(image),
    }))
}

fn made(sparse: bool, slots: Option<u32>) -> grit::serve3::journal::Made {
    grit::serve3::journal::Made {
        rule: "sand".to_owned(),
        params: Vec::new(),
        size: [32, 32, 32],
        sparse,
        slots,
    }
}

#[test]
fn ledgers_whose_replays_differ_in_cost_or_result_never_share_a_journal_key() {
    use grit::serve3::journal::{Entry, Ledger, state_of};
    let small = sparse_image(1, None);
    let large = sparse_image(8, None);
    assert_eq!(
        state_of(&small),
        state_of(&large),
        "setup: the state hash reads cells, not capacity"
    );
    let mut keys = vec![
        (
            "sparse snapshot, 1 slot",
            snapped_in_memory(small).journal(),
        ),
        (
            "sparse snapshot, 8 slots",
            snapped_in_memory(large).journal(),
        ),
        (
            "two-plane snapshot, plane 1 at 0",
            snapped_in_memory(sparse_image(8, Some(0))).journal(),
        ),
        (
            "two-plane snapshot, plane 1 at 5",
            snapped_in_memory(sparse_image(8, Some(5))).journal(),
        ),
        ("dense", Ledger::created(made(false, None)).journal()),
        ("sparse", Ledger::created(made(true, None)).journal()),
        (
            "sparse, 1 slot",
            Ledger::created(made(true, Some(1))).journal(),
        ),
        (
            "sparse, 2 slots",
            Ledger::created(made(true, Some(2))).journal(),
        ),
        (
            "law a",
            Ledger::created(made(false, None))
                .with_law(Some("a".into()))
                .journal(),
        ),
        (
            "law b",
            Ledger::created(made(false, None))
                .with_law(Some("b".into()))
                .journal(),
        ),
        (
            "rule wire",
            Ledger::created(grit::serve3::journal::Made {
                rule: "wire".to_owned(),
                ..made(false, None)
            })
            .journal(),
        ),
    ];
    let pushed = |entry: Entry, spent: Duration| {
        let mut ledger = Ledger::created(made(false, None));
        ledger.push(entry, spent);
        ledger.journal()
    };
    let millis = Duration::from_millis;
    keys.extend([
        ("step 1", pushed(Entry::Step(1), millis(5))),
        ("step 2", pushed(Entry::Step(2), millis(5))),
        ("step 1, recorded longer", pushed(Entry::Step(1), millis(6))),
        (
            "write 1",
            pushed(Entry::Write(vec![([0, 0, 0], 1)]), millis(5)),
        ),
        (
            "write 2",
            pushed(Entry::Write(vec![([0, 0, 0], 2)]), millis(5)),
        ),
        ("params", pushed(Entry::Params(vec![1]), millis(5))),
    ]);
    for (at, (one, key)) in keys.iter().enumerate() {
        for (other, held) in keys.iter().skip(at + 1) {
            assert_ne!(key, held, "{one} and {other} share a journal key");
        }
    }
}

#[test]
fn one_ledger_keeps_its_journal_key_through_its_kept_journal_and_a_copy() {
    use grit::serve3::journal::{Entry, Keep, Ledger, Snap};
    struct Gone(std::path::PathBuf);
    impl Drop for Gone {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let held =
        Gone(std::env::temp_dir().join(format!("grit-f2b-key-{}-{nanos}", std::process::id())));
    let keep = Keep::at(&held.0).unwrap();
    let image = sparse_image(1, None);
    let snapped = Ledger::snapped(std::sync::Arc::new(Snap {
        sum: [7; 32],
        frame: image.frame,
        made: grit::serve3::journal::Made::of_image(&image),
        image: None,
    }));
    let mut ledgers = vec![
        Ledger::created(made(true, Some(3))).with_law(Some("law".into())),
        snapped,
    ];
    for ledger in &mut ledgers {
        ledger.push(
            Entry::Write(vec![([1, 2, 3], 4)]),
            Duration::new(0, 1_234_567),
        );
        ledger.push(Entry::Step(9), Duration::new(1, 999_999_999));
        ledger.push(Entry::Params(vec![2]), Duration::from_secs(86_400));
    }
    for (number, ledger) in (1..).zip(&ledgers) {
        keep.begin(number, ledger).unwrap();
    }
    let (scanned, ignored) = keep.scan();
    assert!(ignored.is_empty(), "{ignored:?}");
    assert_eq!(scanned.len(), ledgers.len());
    for ((number, read), ledger) in scanned.iter().zip(&ledgers) {
        assert_eq!(
            read.journal(),
            ledger.journal(),
            "world {number}: its kept journal reads back under another key"
        );
        assert_eq!(ledger.clone().journal(), ledger.journal());
    }
}

#[test]
fn the_refusal_counts_stay_bounded_under_a_stream_of_distinct_refused_journals() {
    use grit::serve3::budget::REFUSALS_KEPT;
    let mut rates = Rates::default();
    for journal in 0..(REFUSALS_KEPT as u64) * 16 {
        assert!(rates.refuses_replay(journal));
        assert!(rates.refuses(Work::Build, journal));
        assert!(rates.refusals_kept() <= REFUSALS_KEPT);
    }
    assert_eq!(rates.refusals_kept(), REFUSALS_KEPT);
}

#[test]
fn a_refusal_count_touched_while_others_stream_past_still_reaches_its_remeasure() {
    use grit::serve3::budget::REFUSALS_KEPT;
    let mut rates = Rates::default();
    let mut stream = 1_000_u64;
    for refusal in 0..REMEASURE_AFTER {
        assert!(rates.refuses_replay(1), "refusal {refusal}");
        for _ in 0..REFUSALS_KEPT - 1 {
            assert!(rates.refuses_replay(stream));
            stream += 1;
        }
    }
    assert!(
        !rates.refuses_replay(1),
        "a journal refused {REMEASURE_AFTER} times in a row lost its count to journals refused between"
    );
}

#[cfg(feature = "faults")]
mod served {
    use std::time::{Duration, Instant};

    use grit::Host;
    use grit::serve3::journal::{Entry, Keep, Ledger, Made};
    use grit::serve3::{Options, Server, rules};
    use grit::wire::{Json, written};

    use super::common::device;

    fn int_at(held: &Json, key: &str) -> i64 {
        held.get(key)
            .and_then(Json::int)
            .unwrap_or_else(|| panic!("no {key} in {}", written(held)))
    }

    fn line(name: &str, arguments: &str) -> String {
        format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{{\"name\":\"{name}\",\"arguments\":{arguments}}}}}"
        )
    }

    fn call(server: &mut Server<'_>, name: &str, arguments: &str) -> Json {
        server.heard(&line(name, arguments)).unwrap()
    }

    fn result(got: &Json, error: bool) -> Json {
        let result = got.get("result").unwrap();
        assert_eq!(
            result.get("isError"),
            Some(&Json::Bool(error)),
            "{}",
            written(got)
        );
        result.get("structuredContent").cloned().unwrap()
    }

    fn dense_side(host: &Host) -> u32 {
        let sand = rules()
            .into_iter()
            .find(|one| one.name() == "sand")
            .unwrap();
        for side in [256, 128, 64] {
            let made = Made {
                rule: "sand".to_owned(),
                params: Vec::new(),
                size: [side, side, side],
                sparse: false,
                slots: None,
            };
            if let Ok((_, sizes)) = grit::serve3::shaped(host, sand.as_ref(), &made)
                && host.limits().admits(&sizes, None).is_ok()
            {
                return side;
            }
        }
        32
    }

    fn cube(side: u32, budget: Option<Duration>) -> String {
        let budget = budget.map_or(String::new(), |held| {
            format!(",\"budget_ms\":{}", held.as_millis())
        });
        format!("{{\"wide\":{side},\"deep\":{side},\"tall\":{side},\"rule\":\"sand\"{budget}}}")
    }

    fn built(server: &mut Server<'_>, side: u32, budget: Option<Duration>) -> Duration {
        let began = Instant::now();
        let made = result(
            &call(server, "grit_world_create", &cube(side, budget)),
            false,
        );
        let took = began.elapsed();
        result(
            &call(
                server,
                "grit_world_close",
                &format!("{{\"world\":{}}}", int_at(&made, "world")),
            ),
            false,
        );
        took
    }

    #[test]
    fn a_build_held_up_once_never_refuses_a_larger_world_that_fits() {
        let Some(host) = device("held up build") else {
            return;
        };
        let side = dense_side(&host);
        let mut server = Server::new(Some(&host));
        let faults = server.faults();
        let held = Duration::from_millis(2000);
        {
            let mut armed = faults.lock().unwrap();
            armed.slow_builds = 1;
            armed.slow_build = held;
        }
        let small = built(&mut server, 16, None);
        assert!(small >= held, "{small:?}");
        let budget = small + held;
        let large = built(&mut server, side, Some(budget));
        eprintln!(
            "a 16^3 build held up to {small:?}, then a {side}^3 build with a {budget:?} budget ran in {large:?}"
        );
    }

    #[test]
    fn a_build_slower_than_its_budget_in_every_recent_sample_is_refused_before_it_starts() {
        let Some(host) = device("slow builds") else {
            return;
        };
        let side = dense_side(&host);
        let mut server = Server::new(Some(&host));
        let faults = server.faults();
        let slow = Duration::from_millis(1500);
        {
            let mut armed = faults.lock().unwrap();
            armed.slow_builds = u32::MAX;
            armed.slow_build = slow;
        }
        let first = built(&mut server, 16, None);
        assert!(first >= slow, "{first:?}");
        let short = Duration::from_millis(1000);
        for asked in [16, side] {
            let told = result(
                &call(&mut server, "grit_world_create", &cube(asked, Some(short))),
                true,
            );
            let message = told.get("message").and_then(Json::text).unwrap();
            eprintln!("{asked}^3 with a {short:?} budget refused: {message}");
            assert_eq!(told.get("code").and_then(Json::text), Some("budget"));
            assert_eq!(told.get("field").and_then(Json::text), Some("$.budget_ms"));
            assert!(message.contains("building the world"), "{message}");
            assert!(message.contains("nothing ran"), "{message}");
            assert!(
                u128::try_from(int_at(&told, "limit")).unwrap() > slow.as_millis(),
                "{}",
                written(&told)
            );
            assert_eq!(
                faults.lock().unwrap().slow_builds,
                u32::MAX - 1,
                "the refused create of {asked}^3 built"
            );
        }
        faults.lock().unwrap().slow_builds = 0;
    }

    #[test]
    fn after_eight_refusals_by_evidence_the_next_build_runs_and_is_measured_again() {
        let Some(host) = device("remeasure") else {
            return;
        };
        let mut server = Server::new(Some(&host));
        let faults = server.faults();
        let larger = (0..grit::serve3::budget::RECENT)
            .map(|_| built(&mut server, 64, None))
            .max()
            .unwrap();
        let short = (larger * 3).max(Duration::from_millis(1000));
        let slow = short + Duration::from_secs(2);
        {
            let mut armed = faults.lock().unwrap();
            armed.slow_builds = 1;
            armed.slow_build = slow;
        }
        let first = built(&mut server, 16, None);
        assert!(first >= slow, "{first:?}");
        assert!(short < first, "setup: {first:?} {short:?}");
        for refusal in 0..grit::serve3::budget::REMEASURE_AFTER {
            let told = result(
                &call(&mut server, "grit_world_create", &cube(16, Some(short))),
                true,
            );
            let message = told.get("message").and_then(Json::text).unwrap();
            assert!(
                message.contains("by the fastest the server measured for work of this size"),
                "refusal {refusal}: {message}"
            );
        }
        let again = built(&mut server, 16, Some(short));
        eprintln!(
            "64^3 builds at most {larger:?}; a held-up 16^3 build {first:?}, 8 refusals at {short:?}, then it ran in {again:?}"
        );
        assert!(again < short, "{again:?}");
        let fits = built(&mut server, 16, Some(short));
        assert!(fits < short, "{fits:?}");
    }

    #[test]
    fn a_cut_replay_records_only_the_write_entries_that_ran_whole() {
        let Some(host) = device("cut attribution") else {
            return;
        };
        let held = scratch("cut-attribution");
        let dir = held.0.join("worlds");
        let keep = Keep::at(&dir).unwrap();
        let cells: Vec<([u32; 3], u32)> =
            (0..64_u32).map(|at| ([at % 32, at / 32, 0], 1)).collect();
        let mut ledger = Ledger::created(Made {
            rule: "sand".to_owned(),
            params: Vec::new(),
            size: [32, 32, 32],
            sparse: false,
            slots: None,
        });
        ledger.push(Entry::Write(cells.clone()), Duration::from_millis(1));
        ledger.push(Entry::Step(8), Duration::from_millis(100));
        ledger.push(Entry::Write(cells), Duration::from_millis(1));
        keep.begin(1, &ledger).unwrap();
        keep.begin(2, &ledger).unwrap();
        keep.set_next(3).unwrap();
        drop(keep);
        let options = Options {
            dir: Some(dir),
            memory: None,
        };
        let mut server = Server::with(Some(&host), &options).unwrap();
        let writes = |server: &mut Server<'_>| {
            let told = result(&call(server, "grit_describe", "{}"), false);
            int_at(
                told.get("submissions")
                    .and_then(|one| one.get("rates"))
                    .and_then(|one| one.get("write"))
                    .unwrap(),
                "observations",
            )
        };
        let began = Instant::now();
        result(&opened(&mut server, 2, Duration::from_secs(600)), false);
        let budget = began.elapsed() * 2 + Duration::from_millis(300);
        let ratios = replays_measured(&mut server);
        let before = writes(&mut server);
        grit::serve3::tools::faults::stall_batch(0, budget + Duration::from_millis(500));
        let told = result(&opened(&mut server, 1, budget), true);
        grit::serve3::tools::faults::disarm();
        let message = told.get("message").and_then(Json::text).unwrap().to_owned();
        eprintln!("a [write, step, write] journal with its step held up: {message}");
        assert!(message.contains("nothing was opened"), "{message}");
        assert_eq!(
            replays_measured(&mut server),
            ratios,
            "a cut replay taught a replay sample"
        );
        assert_eq!(
            writes(&mut server),
            before + 1,
            "only the first write entry, which ran whole, is a write sample"
        );
    }

    fn cancelled_open(server: &mut Server<'_>, number: u32) {
        let faults = server.faults();
        {
            let mut armed = faults.lock().unwrap();
            armed.held_replays = 1;
            armed.held_by = Duration::from_secs(3);
        }
        let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let setter = {
            let flag = std::sync::Arc::clone(&flag);
            let faults = std::sync::Arc::clone(&faults);
            std::thread::spawn(move || {
                while faults.lock().unwrap().held_replays > 0 {
                    std::thread::sleep(Duration::from_millis(5));
                }
                std::thread::sleep(Duration::from_secs(1));
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
            })
        };
        server.arriving(None, Some(std::sync::Arc::clone(&flag)));
        let told = result(&opened(server, number, Duration::from_secs(600)), true);
        server.arriving(None, None);
        setter.join().unwrap();
        let message = told.get("message").and_then(Json::text).unwrap();
        assert!(message.contains("cancelled"), "{message}");
    }

    #[test]
    fn cancelled_replays_teach_no_ratio_and_never_refuse_an_open_that_fits() {
        let Some(host) = device("cancelled replays") else {
            return;
        };
        let held = scratch("cancelled");
        let dir = held.0.join("worlds");
        let keep = Keep::at(&dir).unwrap();
        for number in 1..=5 {
            let mut ledger = Ledger::created(Made {
                rule: "sand".to_owned(),
                params: Vec::new(),
                size: [32, 32, 32],
                sparse: false,
                slots: None,
            });
            ledger.push(Entry::Step(400), Duration::from_secs(1));
            keep.begin(number, &ledger).unwrap();
        }
        keep.set_next(6).unwrap();
        drop(keep);
        let options = Options {
            dir: Some(dir),
            memory: None,
        };
        let mut server = Server::with(Some(&host), &options).unwrap();
        result(&opened(&mut server, 1, Duration::from_secs(600)), false);
        let ratios = replays_measured(&mut server);
        for number in 2..=4 {
            cancelled_open(&mut server, number);
            assert_eq!(
                replays_measured(&mut server),
                ratios,
                "a cancelled replay taught a ratio"
            );
        }
        let probe = result(&opened(&mut server, 5, Duration::from_millis(1)), true);
        eprintln!("after one open and three cancels: {}", written(&probe));
        result(&opened(&mut server, 5, Duration::from_secs(600)), false);
    }

    const OPEN_STALL: Duration = Duration::from_millis(50);

    #[test]
    fn cut_replays_teach_nothing_and_never_refuse_an_identical_open_that_fits() {
        let Some(host) = device("cuts teach nothing") else {
            return;
        };
        let held = scratch("cuts-teach-nothing");
        let dir = held.0.join("worlds");
        let keep = Keep::at(&dir).unwrap();
        let chunk = u32::try_from(grit::serve3::journal::WRITE_CHUNK).unwrap();
        let cells: Vec<([u32; 3], u32)> = (0..chunk).map(|at| ([at, 0, 0], 1 + at % 2)).collect();
        for number in 1..=7 {
            let mut ledger = Ledger::created(Made {
                rule: "sand".to_owned(),
                params: Vec::new(),
                size: [32, 32, 32],
                sparse: false,
                slots: None,
            });
            ledger.push(Entry::Write(cells.clone()), Duration::from_millis(1));
            ledger.push(Entry::Step(40), Duration::from_secs(1));
            keep.begin(number, &ledger).unwrap();
        }
        keep.set_next(8).unwrap();
        drop(keep);
        let options = Options {
            dir: Some(dir),
            memory: None,
        };
        let mut server = Server::with(Some(&host), &options).unwrap();
        grit::serve3::tools::faults::stall_every(OPEN_STALL);
        let mut whole = Duration::MAX;
        for number in [1, 6] {
            let began = Instant::now();
            result(
                &opened(&mut server, number, Duration::from_secs(600)),
                false,
            );
            whole = whole.min(began.elapsed());
        }
        let measured = replays_measured(&mut server);
        let cut = whole * 2;
        for number in 2..=4 {
            {
                let faults = server.faults();
                let mut armed = faults.lock().unwrap();
                armed.held_replays = 1;
                armed.held_by = cut;
            }
            let told = result(&opened(&mut server, number, cut), true);
            let message = told.get("message").and_then(Json::text).unwrap();
            assert!(message.contains("stopped on budget"), "{message}");
            assert_eq!(
                replays_measured(&mut server),
                measured,
                "a cut replay taught a sample"
            );
        }
        let probe = result(&opened(&mut server, 5, Duration::from_millis(1)), true);
        let message = probe.get("message").and_then(Json::text).unwrap();
        eprintln!("an open took {whole:?}; after three cuts at {cut:?}: {message}");
        let enough = cut - whole / 2;
        let began = Instant::now();
        let got = opened(&mut server, 7, enough);
        grit::serve3::tools::faults::disarm();
        eprintln!("open with {enough:?}: {:?}", began.elapsed());
        result(&got, false);
    }

    struct Scratch(std::path::PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn scratch(label: &str) -> Scratch {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("grit-f2-{label}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Scratch(path)
    }

    fn replays_measured(server: &mut Server<'_>) -> usize {
        let told = result(&call(server, "grit_describe", "{}"), false);
        let observed = told
            .get("submissions")
            .and_then(|one| one.get("rates"))
            .and_then(|one| one.get("replay_observations"))
            .and_then(Json::int)
            .unwrap();
        usize::try_from(observed).unwrap()
    }

    fn opened(server: &mut Server<'_>, number: u32, budget: Duration) -> Json {
        call(
            server,
            "grit_world_open",
            &format!(
                "{{\"name\":\"world-{number}\",\"budget_ms\":{}}}",
                budget.as_millis()
            ),
        )
    }

    const LATENESS: Duration = Duration::from_secs(1);

    fn reached(message: &str) -> u64 {
        let at = message
            .find("to frame ")
            .unwrap_or_else(|| panic!("no frame in {message}"));
        message[at + 9..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>()
            .parse()
            .unwrap()
    }

    #[test]
    fn a_replay_that_never_finished_is_never_refused_and_each_cut_ends_within_its_budget() {
        let Some(host) = device("cut replays") else {
            return;
        };
        let held = scratch("cut-replay");
        let dir = held.0.join("worlds");
        let keep = Keep::at(&dir).unwrap();
        let mut ledger = Ledger::created(Made {
            rule: "sand".to_owned(),
            params: Vec::new(),
            size: [32, 32, 32],
            sparse: false,
            slots: None,
        });
        ledger.push(Entry::Step(4000), Duration::from_millis(400));
        keep.begin(1, &ledger).unwrap();
        keep.set_next(2).unwrap();
        drop(keep);
        let options = Options {
            dir: Some(dir),
            memory: None,
        };
        let mut server = Server::with(Some(&host), &options).unwrap();
        let builds: Vec<Duration> = (0..grit::serve3::budget::RECENT)
            .map(|_| built(&mut server, 32, None))
            .collect();
        let slowest = builds.iter().copied().max().unwrap();
        let budget = (slowest * 2 + Duration::from_millis(300)).max(Duration::from_millis(600));
        let stall = LATENESS / (4 * grit::serve3::budget::BATCH_MOST);
        eprintln!("32^3 builds {builds:?}; budget {budget:?}; {stall:?} a frame");
        grit::serve3::tools::faults::stall_every(stall);
        let most = u64::try_from(budget.as_nanos() / stall.as_nanos()).unwrap()
            + u64::from(grit::serve3::budget::BATCH_MOST);
        for call in 0..=grit::serve3::budget::RECENT {
            let before = replays_measured(&mut server);
            let began = Instant::now();
            let told = result(&opened(&mut server, 1, budget), true);
            let took = began.elapsed();
            let message = told.get("message").and_then(Json::text).unwrap().to_owned();
            let frame = reached(&message);
            eprintln!("call {call} with {budget:?}: frame {frame} of at most {most} in {took:?}");
            assert!(
                message.contains("nothing was opened"),
                "call {call}: {message}"
            );
            assert!(
                frame <= most,
                "call {call}: frames held {stall:?} each ran past the budget and one batch: {message}"
            );
            assert_eq!(replays_measured(&mut server), before, "call {call}");
        }
        grit::serve3::tools::faults::disarm();
    }

    fn lost(mut server: Server<'_>) -> grit::serve3::Book {
        server.faults().lock().unwrap().wedge_before_call = true;
        let _ = call(&mut server, "grit_describe", "{}");
        assert!(server.lost(), "setup: the device was not lost");
        server.into_book()
    }

    fn read(server: &mut Server<'_>, world: i64, budget: Duration) -> Json {
        call(
            server,
            "grit_world_read",
            &format!(
                "{{\"world\":{world},\"from\":[0,0,0],\"to\":[0,0,0],\"budget_ms\":{}}}",
                budget.as_millis()
            ),
        )
    }

    fn refused_by_its_journal(told: &Json, what: &str) {
        let message = told.get("message").and_then(Json::text).unwrap();
        assert!(
            message.contains("nothing ran") && !message.contains("whose build alone"),
            "{what}: {message}"
        );
    }

    #[test]
    fn a_fork_and_a_lost_device_keep_the_journal_key_their_rebuilds_are_decided_by() {
        let Some(host) = device("journal key kept") else {
            return;
        };
        let mut server = Server::new(Some(&host));
        let made = result(
            &call(&mut server, "grit_world_create", &cube(32, None)),
            false,
        );
        let world = int_at(&made, "world");
        grit::serve3::tools::faults::stall_every(Duration::from_millis(20));
        result(
            &call(
                &mut server,
                "grit_world_step",
                &format!("{{\"world\":{world},\"frames\":40,\"budget_ms\":600000}}"),
            ),
            false,
        );
        let forked = result(
            &call(
                &mut server,
                "grit_world_fork",
                &format!("{{\"world\":{world}}}"),
            ),
            false,
        );
        let fork = int_at(&forked, "world");
        let book = lost(server);
        drop(host);
        let host = device("journal key kept, again").unwrap();
        let (mut server, _) = Server::resumed(Some(&host), book);
        let began = Instant::now();
        result(&read(&mut server, world, Duration::from_secs(600)), false);
        let whole = began.elapsed();
        let budget = whole / 2;
        let told = result(&read(&mut server, fork, budget), true);
        refused_by_its_journal(
            &told,
            &format!(
                "the fork's rebuild given {budget:?}, after its source's rebuild took {whole:?}"
            ),
        );
        let book = lost(server);
        drop(host);
        let host = device("journal key kept, third").unwrap();
        let (mut server, _) = Server::resumed(Some(&host), book);
        let told = result(&read(&mut server, world, budget), true);
        refused_by_its_journal(
            &told,
            &format!("the second rebuild given {budget:?}, after the first took {whole:?}"),
        );
        grit::serve3::tools::faults::disarm();
        result(&read(&mut server, fork, Duration::from_secs(600)), false);
    }

    #[test]
    fn a_held_up_create_never_refuses_the_rebuild_of_its_world_from_a_snapshot() {
        let Some(host) = device("create apart from image build") else {
            return;
        };
        let mut server = Server::new(Some(&host));
        let held = Duration::from_secs(3);
        {
            let faults = server.faults();
            let mut armed = faults.lock().unwrap();
            armed.slow_builds = 1;
            armed.slow_build = held;
        }
        let began = Instant::now();
        let made = result(
            &call(&mut server, "grit_world_create", &cube(32, None)),
            false,
        );
        assert!(began.elapsed() >= held, "setup: the create was not held");
        let world = int_at(&made, "world");
        grit::serve3::tools::faults::stall_batch(0, held);
        result(
            &call(
                &mut server,
                "grit_world_step",
                &format!("{{\"world\":{world},\"frames\":1,\"budget_ms\":600000}}"),
            ),
            false,
        );
        grit::serve3::tools::faults::disarm();
        let book = lost(server);
        drop(host);
        let host = device("create apart from image build, again").unwrap();
        let (mut server, _) = Server::resumed(Some(&host), book);
        let budget = held / 3;
        let began = Instant::now();
        let got = read(&mut server, world, budget);
        let took = began.elapsed();
        let restored = result(&got, false);
        eprintln!("rebuilt from its snapshot with {budget:?} in {took:?}");
        let note = restored
            .get("restored")
            .unwrap_or_else(|| panic!("{}", written(&restored)));
        assert_eq!(
            int_at(note, "from_frame"),
            1,
            "setup: the world was not rebuilt from its snapshot: {}",
            written(note)
        );
    }

    const SLOW_FRAMES: u32 = 400;

    const FRAME_STALL: Duration = Duration::from_millis(5);

    fn replay_floor() -> Duration {
        FRAME_STALL * SLOW_FRAMES
    }

    fn two_journals(label: &str) -> (Scratch, Options) {
        let held = scratch(label);
        let dir = held.0.join("worlds");
        let keep = Keep::at(&dir).unwrap();
        for (number, frames) in [
            (1, SLOW_FRAMES),
            (2, SLOW_FRAMES),
            (3, 40),
            (4, 40),
            (5, 40),
        ] {
            let mut ledger = Ledger::created(Made {
                rule: "sand".to_owned(),
                params: Vec::new(),
                size: [32, 32, 32],
                sparse: false,
                slots: None,
            });
            ledger.push(Entry::Step(frames), Duration::from_millis(100));
            keep.begin(number, &ledger).unwrap();
        }
        keep.set_next(6).unwrap();
        drop(keep);
        let options = Options {
            dir: Some(dir),
            memory: None,
        };
        (held, options)
    }

    #[test]
    fn a_journal_that_finished_too_slow_is_refused_whatever_other_journals_finished() {
        let Some(host) = device("journal refusal") else {
            return;
        };
        let (_held, options) = two_journals("journal-refusal");
        let mut server = Server::with(Some(&host), &options).unwrap();
        grit::serve3::tools::faults::stall_every(FRAME_STALL);
        let began = Instant::now();
        result(&opened(&mut server, 1, Duration::from_secs(600)), false);
        let slow = began.elapsed();
        let mut fast = Duration::ZERO;
        for number in 3..=5 {
            let began = Instant::now();
            result(
                &opened(&mut server, number, Duration::from_secs(600)),
                false,
            );
            fast = fast.max(began.elapsed());
        }
        let budget = ((slow + fast) / 2).min(replay_floor() * 9 / 10);
        eprintln!(
            "400 frames finished in {slow:?}, 40 frames in at most {fast:?}; budget {budget:?}"
        );
        assert!(
            fast + Duration::from_millis(20) < budget,
            "setup: {fast:?} {budget:?}"
        );
        let began = Instant::now();
        let told = result(&opened(&mut server, 2, budget), true);
        let took = began.elapsed();
        grit::serve3::tools::faults::disarm();
        let message = told.get("message").and_then(Json::text).unwrap();
        assert!(
            message.contains("nothing ran"),
            "an identical journal finished whole in {slow:?}, so {budget:?} cannot fit it; three other journals finished in at most {fast:?}; the open was run instead of refused (answered in {took:?}): {message}"
        );
    }

    #[test]
    fn a_replay_refused_by_its_journal_runs_by_the_remeasure_whatever_builds_ran_between() {
        let Some(host) = device("replay remeasure") else {
            return;
        };
        let (_held, options) = two_journals("replay-remeasure");
        let mut server = Server::with(Some(&host), &options).unwrap();
        grit::serve3::tools::faults::stall_every(FRAME_STALL);
        let began = Instant::now();
        result(&opened(&mut server, 1, Duration::from_secs(600)), false);
        let slow = began.elapsed();
        let budget = (slow / 2).min(replay_floor() / 2);
        let mut refused = 0;
        let mut answer = None;
        for _ in 0..=grit::serve3::budget::REMEASURE_AFTER {
            let told = result(&opened(&mut server, 2, budget), true);
            let message = told.get("message").and_then(Json::text).unwrap().to_owned();
            if !message.contains("nothing ran") {
                answer = Some(message);
                break;
            }
            refused += 1;
            let made = result(
                &call(&mut server, "grit_world_create", &cube(32, None)),
                false,
            );
            result(
                &call(
                    &mut server,
                    "grit_world_close",
                    &format!("{{\"world\":{}}}", int_at(&made, "world")),
                ),
                false,
            );
        }
        grit::serve3::tools::faults::disarm();
        let answer = answer.unwrap_or_else(|| {
            panic!("refused {refused} times in a row with a same-size create between each")
        });
        assert_eq!(refused, grit::serve3::budget::REMEASURE_AFTER, "{answer}");
        assert!(answer.contains("stopped on budget"), "{answer}");
    }
}
