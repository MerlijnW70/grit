#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

#[cfg(feature = "faults")]
mod common;

use std::time::Duration;

use grit::serve3::budget::{BUDGET_MS, Rates, WINDOW, WORLD_LEAST, Work};

const SIXTY_FOUR: u64 = 1_615_268;

const ONE_TWENTY_EIGHT: u64 = 12_864_164;

const TWO_FIFTY_SIX: u64 = 102_854_820;

const SPARSE_LARGE: u64 = 192_388_324;

fn budget() -> Duration {
    Duration::from_millis(BUDGET_MS)
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
fn slow_samples_never_raise_the_least_estimate_while_a_third_of_the_kept_ones_are_faster() {
    let unrelated = 64 << 20;
    for work in Work::ALL {
        let prior = Rates::default().estimate(work, unrelated);
        let mut rates = Rates::default();
        for _ in 0..3 {
            rates.measure(work, unrelated, prior.mul_f64(0.5));
        }
        let settled = rates.least(work, unrelated);
        let close = |got: Duration, want: Duration| {
            got <= want.mul_f64(1.001) && got >= want.mul_f64(0.999)
        };
        assert!(close(settled, prior.mul_f64(0.5)), "{work:?}: {settled:?}");
        for _ in 0..WINDOW - 3 {
            rates.measure(work, unrelated, prior.mul_f64(500.0));
            let least = rates.least(work, unrelated);
            assert!(
                close(least, settled),
                "{work:?}: {least:?} against {settled:?}"
            );
        }
        assert!(
            rates.estimate(work, unrelated) >= prior.mul_f64(499.0),
            "{work:?}: the sizing estimate still errs slow"
        );
        rates.measure(work, unrelated, prior.mul_f64(500.0));
        let slowed = rates.least(work, unrelated);
        assert!(
            close(slowed, prior.mul_f64(500.0)),
            "{work:?}: once more than two thirds of the kept samples are slow the least estimate is slow: {slowed:?}"
        );
        let mut lone = Rates::default();
        lone.measure(work, unrelated, prior.mul_f64(0.5));
        for _ in 0..WINDOW - 1 {
            lone.measure(work, unrelated, prior.mul_f64(500.0));
        }
        let hidden = lone.least(work, unrelated);
        assert!(
            close(hidden, prior.mul_f64(500.0)),
            "{work:?}: one fast sample among slow ones decides nothing: {hidden:?}"
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
fn a_fast_large_build_sets_the_cost_per_byte_of_a_larger_world_below_the_prior() {
    let large = Duration::from_millis(278);
    let small = Duration::from_millis(50);
    let mut rates = Rates::default();
    rates.measure(Work::Build, TWO_FIFTY_SIX, large);
    rates.measure(Work::Build, WORLD_LEAST, small);
    let huge: u64 = 4 << 30;
    let per_byte = large.as_secs_f64() / TWO_FIFTY_SIX as f64;
    let want = large.as_secs_f64() + per_byte * (huge - TWO_FIFTY_SIX) as f64;
    let least = rates.least(Work::Build, huge).as_secs_f64();
    eprintln!(
        "4 GiB after 256^3 in {large:?} and 1 MiB in {small:?}: least {least} s, want {want} s"
    );
    assert!(per_byte < 1e-8, "{per_byte}");
    assert!(
        (least - want).abs() <= want * 0.002,
        "the cost per byte is the large build's time over its own bytes: {least} against {want}"
    );
}

#[test]
fn unlike_work_never_shares_a_least_estimate() {
    let bytes = TWO_FIFTY_SIX;
    let fast = Duration::from_millis(5);
    for (measured, asked) in [
        (Work::Snap, Work::Save),
        (Work::Save, Work::Snap),
        (Work::Build, Work::BuildSparse),
        (Work::BuildSparse, Work::Build),
    ] {
        let fresh = Rates::default().least(asked, bytes);
        let mut rates = Rates::default();
        for _ in 0..WINDOW {
            rates.measure(measured, bytes, fast);
        }
        assert_eq!(
            rates.least(asked, bytes),
            fresh,
            "{measured:?} moved {asked:?}"
        );
        assert!(rates.least(measured, bytes) < fresh, "{measured:?}");
    }
}

#[test]
fn a_build_every_recent_sample_shows_too_slow_is_still_refused() {
    for work in Work::ALL {
        let fresh = Rates::default();
        for bytes in [0, WORLD_LEAST, TWO_FIFTY_SIX] {
            assert_eq!(
                fresh.least(work, bytes),
                fresh.estimate(work, bytes),
                "{work:?} with nothing measured"
            );
        }
    }
    let short = Duration::from_millis(1000);
    let slow = Duration::from_millis(3000);
    let mut rates = Rates::default();
    rates.measure(Work::Build, SIXTY_FOUR, slow);
    for bytes in [WORLD_LEAST, SIXTY_FOUR, TWO_FIFTY_SIX] {
        let least = rates.least(Work::Build, bytes);
        assert!(least > short, "{bytes}: {least:?}");
        assert!(
            bytes < SIXTY_FOUR || least >= slow.mul_f64(0.999),
            "{bytes}: {least:?}"
        );
    }
    let mut fast = Rates::default();
    fast.measure(Work::Build, WORLD_LEAST, Duration::from_millis(10));
    let huge = 4 << 30;
    let least = fast.least(Work::Build, huge);
    assert!(
        least > budget(),
        "a world far past every measured size pays the fastest measured cost per byte: {least:?}"
    );
    assert!(least <= fast.estimate(Work::Build, huge));
}

#[test]
fn a_slow_replay_never_raises_the_least_replay_ratio() {
    let mut rates = Rates::default();
    assert!((rates.replay_least() - rates.replay()).abs() < 1e-9);
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
        let budget = Duration::from_millis(1500);
        grit::serve3::tools::faults::stall_every(Duration::from_millis(2));
        let mut cut = 0;
        let mut refused = None;
        for _ in 0..6 {
            let began = Instant::now();
            let told = result(&opened(&mut server, 2, budget), true);
            let took = began.elapsed();
            let message = told.get("message").and_then(Json::text).unwrap().to_owned();
            eprintln!("open with a {budget:?} budget answered in {took:?}: {message}");
            assert_eq!(told.get("code").and_then(Json::text), Some("budget"));
            assert!(took < budget + Duration::from_secs(1), "{took:?}");
            if message.contains("nothing was opened") {
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
        assert!(cut <= 3, "{cut} calls were cut before the refusal");
    }
}
