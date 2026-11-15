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
            rates.measure_cut(work, bytes, spent);
        }
    }
    for (recorded, spent, least) in [
        (Duration::ZERO, Duration::MAX, f64::MAX),
        (tiny, Duration::MAX, 0.0),
        (Duration::MAX, Duration::MAX, f64::INFINITY),
        (Duration::ZERO, tiny, f64::NAN),
        (Duration::MAX, tiny, 0.0),
        (Duration::ZERO, Duration::from_millis(600), 0.0),
    ] {
        rates.measure_cut_replay(recorded, spent, least);
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
    for recorded in [Duration::ZERO, tiny, Duration::from_secs(1), Duration::MAX] {
        let _ = rates.replaying_least(recorded);
        let _ = rates.replaying(recorded);
    }
    let _ = rates.shown();
    let told = grit::serve3::budget::too_slow(Duration::MAX, Duration::ZERO, "work", Basis::Both);
    assert!(told.message.contains("needs at least"), "{}", told.message);
}

#[test]
fn a_replay_cut_before_its_first_frame_records_a_finite_lower_bound() {
    let mut rates = Rates::default();
    let entry = Duration::from_secs(1);
    let spent = Duration::from_millis(1500);
    let bound = spent.as_secs_f64() / entry.as_secs_f64();
    rates.measure_cut_replay(Duration::ZERO, spent, bound);
    assert!(
        (rates.replay_least() - 1.5).abs() < 1e-9,
        "{}",
        rates.replay_least()
    );
    assert_eq!(rates.replaying_least(entry), spent);
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
    let least = rates.replaying_least(recorded);
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
    assert_eq!(Rates::default().replay_least(), 0.0);
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
fn cut_samples_alone_take_their_largest_and_one_finished_sample_takes_the_smallest() {
    let mut rates = Rates::default();
    for spent in [3, 5, 4] {
        rates.measure_cut(Work::Write, 4096, Duration::from_secs(spent));
    }
    assert_eq!(rates.least(Work::Write, 4096), Duration::from_secs(5));
    rates.measure(Work::Write, 4096, Duration::from_secs(1));
    assert_eq!(rates.least(Work::Write, 4096), Duration::from_secs(1));
    let mut ratios = Rates::default();
    for spent in [3, 5, 4] {
        ratios.measure_cut_replay(Duration::from_secs(1), Duration::from_secs(spent), 0.0);
    }
    assert!((ratios.replay_least() - 5.0).abs() < 1e-9);
    ratios.measure_replay(Duration::from_secs(1), Duration::from_secs(1));
    assert!((ratios.replay_least() - 1.0).abs() < 1e-9);
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
fn the_prior_never_outvotes_one_measured_write_and_a_cut_tail_is_refused() {
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
    rates.measure_cut(
        Work::Write,
        tail,
        Duration::from_secs_f64(2e-4 * tail as f64),
    );
    assert!(
        rates.least(Work::Write, tail) > budget(),
        "a tail one cut replay measured as too slow is refused"
    );
}

#[test]
fn a_cut_replay_that_ran_few_frames_is_still_measured() {
    let mut rates = Rates::default();
    rates.measure_replay(Duration::from_secs(1), Duration::from_secs(1));
    let recorded = Duration::from_millis(30);
    let spent = Duration::from_millis(1500);
    rates.measure_replay(recorded, spent);
    assert!(
        (rates.replay_least() - 1.0).abs() < 1e-9,
        "a finished replay below the floor is noise"
    );
    for _ in 0..RECENT {
        rates.measure_cut_replay(recorded, spent, 0.0);
    }
    assert!(
        rates.replay_least() >= 8.0 - 1e-9,
        "three cut replays that ran 30 ms of recorded steps in 1.5 s: {}",
        rates.replay_least()
    );
}

#[test]
fn a_slow_replay_never_raises_the_least_replay_ratio_while_a_faster_one_is_recent() {
    let mut rates = Rates::default();
    assert!(rates.replay_least().abs() < 1e-9);
    for _ in 0..2 {
        rates.measure_replay(Duration::from_secs(1), Duration::from_millis(1500));
    }
    assert!(
        (rates.replay_least() - 1.5).abs() < 1e-9,
        "{}",
        rates.replay_least()
    );
    rates.measure_replay(Duration::from_secs(1), Duration::from_secs(6));
    assert!(rates.replay() >= 6.0 - 1e-9, "{}", rates.replay());
    assert!(
        (rates.replay_least() - 1.5).abs() < 1e-9,
        "{}",
        rates.replay_least()
    );
    assert_eq!(
        rates.replaying_least(Duration::from_secs(2)),
        Duration::from_secs(3)
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
        let budget = held * 2;
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
            let began = Instant::now();
            let told = result(
                &call(&mut server, "grit_world_create", &cube(asked, Some(short))),
                true,
            );
            let took = began.elapsed();
            let message = told.get("message").and_then(Json::text).unwrap();
            eprintln!("{asked}^3 with a {short:?} budget refused in {took:?}: {message}");
            assert_eq!(told.get("code").and_then(Json::text), Some("budget"));
            assert_eq!(told.get("field").and_then(Json::text), Some("$.budget_ms"));
            assert!(message.contains("building the world"), "{message}");
            assert!(message.contains("nothing ran"), "{message}");
            assert!(
                u128::try_from(int_at(&told, "limit")).unwrap() > slow.as_millis(),
                "{}",
                written(&told)
            );
            assert!(took < short, "{took:?}");
        }
        assert_eq!(
            faults.lock().unwrap().slow_builds,
            u32::MAX - 1,
            "a refused create built nothing"
        );
        faults.lock().unwrap().slow_builds = 0;
    }

    #[test]
    fn after_eight_refusals_by_evidence_the_next_build_runs_and_is_measured_again() {
        let Some(host) = device("remeasure") else {
            return;
        };
        let mut server = Server::new(Some(&host));
        let faults = server.faults();
        let slow = Duration::from_millis(1500);
        {
            let mut armed = faults.lock().unwrap();
            armed.slow_builds = 1;
            armed.slow_build = slow;
        }
        let first = built(&mut server, 16, None);
        assert!(first >= slow, "{first:?}");
        let short = Duration::from_millis(1000);
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
        eprintln!("a held-up 16^3 build {first:?}, 8 refusals, then it ran in {again:?}");
        assert!(again < short, "{again:?}");
        let fits = built(&mut server, 16, Some(short));
        assert!(fits < short, "{fits:?}");
    }

    #[test]
    fn a_step_that_uses_up_the_budget_before_a_write_is_recorded_as_a_step() {
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
            ratios + 1,
            "the cut was not recorded as a step"
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

    #[test]
    fn a_step_cut_charges_only_its_own_time_never_the_writes_before_it() {
        let Some(host) = device("step cut floor") else {
            return;
        };
        let held = scratch("step-floor");
        let dir = held.0.join("worlds");
        let keep = Keep::at(&dir).unwrap();
        let cells: Vec<([u32; 3], u32)> = (0..2048_u32)
            .map(|at| ([at % 32, (at / 32) % 32, at / 1024], 1 + at % 2))
            .collect();
        for number in 1..=6 {
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
        keep.set_next(7).unwrap();
        drop(keep);
        let options = Options {
            dir: Some(dir),
            memory: None,
        };
        let mut server = Server::with(Some(&host), &options).unwrap();
        let began = Instant::now();
        result(&opened(&mut server, 1, Duration::from_secs(600)), false);
        let whole = began.elapsed();
        let cut = whole * 2;
        let enough = cut + whole / 2;
        for number in 2..=4 {
            let before = replays_measured(&mut server);
            {
                let faults = server.faults();
                let mut armed = faults.lock().unwrap();
                armed.held_replays = 1;
                armed.held_by = cut.saturating_sub(whole) + Duration::from_millis(30);
            }
            let told = result(&opened(&mut server, number, cut), true);
            let message = told.get("message").and_then(Json::text).unwrap();
            assert!(message.contains("stopped on budget"), "{message}");
            assert_eq!(replays_measured(&mut server), before + 1);
        }
        let probe = result(&opened(&mut server, 5, Duration::from_millis(1)), true);
        eprintln!(
            "an open took {whole:?}; after three cuts at {cut:?}: {}",
            probe.get("message").and_then(Json::text).unwrap()
        );
        let began = Instant::now();
        let got = call(
            &mut server,
            "grit_world_open",
            &format!(
                "{{\"name\":\"world-6\",\"budget_ms\":{}}}",
                enough.as_millis()
            ),
        );
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

    #[test]
    fn a_replay_too_slow_for_its_budget_is_refused_up_front_after_few_cut_calls() {
        let Some(host) = device("cut replays") else {
            return;
        };
        let held = scratch("cut-replay");
        let dir = held.0.join("worlds");
        let keep = Keep::at(&dir).unwrap();
        for number in [1, 2] {
            let mut ledger = Ledger::created(Made {
                rule: "sand".to_owned(),
                params: Vec::new(),
                size: [32, 32, 32],
                sparse: false,
                slots: None,
            });
            ledger.push(Entry::Step(4000), Duration::from_millis(400));
            keep.begin(number, &ledger).unwrap();
        }
        keep.set_next(3).unwrap();
        drop(keep);
        let options = Options {
            dir: Some(dir),
            memory: None,
        };
        let mut server = Server::with(Some(&host), &options).unwrap();
        result(&opened(&mut server, 1, Duration::from_secs(600)), false);
        let fast = replays_measured(&mut server);
        let bound = grit::serve3::budget::RECENT;
        let budget = Duration::from_millis(1500);
        grit::serve3::tools::faults::stall_every(Duration::from_millis(2));
        let mut cut = 0;
        let mut refused = None;
        for _ in 0..=bound + 1 {
            let before = replays_measured(&mut server);
            let began = Instant::now();
            let told = result(&opened(&mut server, 2, budget), true);
            let took = began.elapsed();
            let message = told.get("message").and_then(Json::text).unwrap().to_owned();
            eprintln!("open with a {budget:?} budget answered in {took:?}: {message}");
            assert_eq!(told.get("code").and_then(Json::text), Some("budget"));
            assert!(took < budget + Duration::from_secs(1), "{took:?}");
            if message.contains("nothing was opened") {
                assert_eq!(
                    replays_measured(&mut server),
                    before + 1,
                    "a cut replay was not measured"
                );
                cut += 1;
                continue;
            }
            assert!(message.contains("rebuilding world 2"), "{message}");
            assert!(message.contains("nothing ran"), "{message}");
            refused = Some(took);
            break;
        }
        grit::serve3::tools::faults::disarm();
        let refused = refused.expect("a replay that never fits was never refused up front");
        assert!(refused < Duration::from_millis(500), "{refused:?}");
        assert!(
            cut <= bound,
            "{cut} calls were cut before the refusal; with {fast} fast ratios kept, the fastest of the last {bound} ratios needs at most {bound} cut calls"
        );
    }
}
