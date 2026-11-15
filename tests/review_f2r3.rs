#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

#[cfg(feature = "faults")]
mod common;

use std::time::Duration;

use grit::serve3::budget::{Rates, WORLD_LEAST, Work};

#[test]
fn r3a_larger_builds_raise_the_least_of_a_smaller_build_above_its_true_cost() {
    let cost = |bytes: u64| Duration::from_secs_f64(0.02 + 3e-9 * bytes as f64);
    let large: u64 = 400_000_000;
    let mut rates = Rates::default();
    rates.measure(Work::Build, WORLD_LEAST, cost(WORLD_LEAST));
    for _ in 0..3 {
        rates.measure(Work::Build, large, cost(large));
    }
    let least = rates.least(Work::Build, WORLD_LEAST);
    eprintln!(
        "small true {:?}, large true {:?}, least(small) {least:?}, estimate(small) {:?}",
        cost(WORLD_LEAST),
        cost(large),
        rates.estimate(Work::Build, WORLD_LEAST)
    );
    assert!(
        least <= cost(WORLD_LEAST) * 2,
        "least {least:?} for a build measured at {:?}",
        cost(WORLD_LEAST)
    );
}

#[test]
fn r3e_one_measured_build_is_floored_at_a_quarter_of_the_prior() {
    let large: u64 = 102_854_820;
    let spent = Duration::from_millis(27);
    let mut rates = Rates::default();
    rates.measure(Work::Build, large, spent);
    let least = rates.least(Work::Build, large);
    eprintln!("one 256^3 build measured at {spent:?}: least for the same size {least:?}");
    assert!(
        least <= spent * 2,
        "least {least:?} for a build the server measured at {spent:?}"
    );
}

#[cfg(feature = "faults")]
mod served {
    use std::time::{Duration, Instant};

    use grit::serve3::Server;
    use grit::wire::{Json, written};

    use super::common::device;

    fn line(name: &str, arguments: &str) -> String {
        format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{{\"name\":\"{name}\",\"arguments\":{arguments}}}}}"
        )
    }

    fn create(server: &mut Server<'_>, side: u32, budget: Option<u128>) -> (Json, Duration) {
        let budget = budget.map_or(String::new(), |ms| format!(",\"budget_ms\":{ms}"));
        let args = format!(
            "{{\"wide\":{side},\"deep\":{side},\"tall\":{side},\"rule\":\"sand\"{budget}}}"
        );
        let began = Instant::now();
        let got = server.heard(&line("grit_world_create", &args)).unwrap();
        let took = began.elapsed();
        let result = got.get("result").unwrap().clone();
        if result.get("isError") == Some(&Json::Bool(false)) {
            let world = result
                .get("structuredContent")
                .unwrap()
                .get("world")
                .and_then(Json::int)
                .unwrap();
            server
                .heard(&line("grit_world_close", &format!("{{\"world\":{world}}}")))
                .unwrap();
        }
        (result, took)
    }

    #[test]
    fn r3b_a_held_up_larger_build_refuses_a_small_build_that_fits() {
        let Some(host) = device("r3b") else {
            return;
        };
        let mut server = Server::new(Some(&host));
        let faults = server.faults();
        let (_, small) = create(&mut server, 16, None);
        let held = Duration::from_millis(1500);
        {
            let mut armed = faults.lock().unwrap();
            armed.slow_builds = 1;
            armed.slow_build = held;
        }
        let (_, large) = create(&mut server, 64, None);
        assert!(large >= held, "{large:?}");
        let (_, again) = create(&mut server, 64, None);
        let budget = 1000;
        let (told, took) = create(&mut server, 16, Some(budget));
        eprintln!(
            "16^3 first {small:?}; 64^3 held {large:?} then {again:?}; 16^3 with {budget} ms: {took:?} {}",
            written(&told)
        );
        assert_eq!(
            told.get("isError"),
            Some(&Json::Bool(false)),
            "a 16^3 build measured at {small:?} refused: {}",
            written(&told)
        );
    }

    #[test]
    fn r3c_larger_builds_refuse_a_small_build_the_server_just_ran() {
        let Some(host) = device("r3c") else {
            return;
        };
        let mut server = Server::new(Some(&host));
        let (_, a) = create(&mut server, 256, None);
        let (_, small) = create(&mut server, 16, None);
        let (_, b) = create(&mut server, 256, None);
        let (_, c) = create(&mut server, 256, None);
        let budget = (small.as_millis() * 2).max(20);
        let (told, took) = create(&mut server, 16, Some(budget));
        eprintln!(
            "256^3 {a:?} {b:?} {c:?}; 16^3 ran in {small:?}; 16^3 with {budget} ms: {took:?} {}",
            written(&told)
        );
        assert_eq!(
            told.get("isError"),
            Some(&Json::Bool(false)),
            "a 16^3 build measured at {small:?} refused: {}",
            written(&told)
        );
    }

    #[test]
    fn r3f_a_fresh_server_refuses_the_build_it_just_ran_by_the_prior_floor() {
        let Some(host) = device("r3f") else {
            return;
        };
        let mut server = Server::new(Some(&host));
        let (_, warm) = create(&mut server, 16, None);
        let (made, first) = create(&mut server, 256, None);
        let refused = made
            .get("structuredContent")
            .and_then(|told| told.get("code"))
            .and_then(Json::text);
        if refused == Some("too_big") {
            eprintln!(
                "this device cannot make a dense 256^3 world: {}",
                written(&made)
            );
            return;
        }
        let budget = (first.as_millis() * 4).max(100);
        let (told, took) = create(&mut server, 256, Some(budget));
        eprintln!(
            "16^3 {warm:?}; 256^3 ran in {first:?}; again with {budget} ms: {took:?} {}",
            written(&told)
        );
        assert_eq!(
            told.get("isError"),
            Some(&Json::Bool(false)),
            "a 256^3 build measured at {first:?} refused at {budget} ms: {}",
            written(&told)
        );
    }

    #[test]
    fn r3d_a_sparse_journal_of_writes_that_never_fits_burns_its_budget_every_call() {
        use grit::serve3::Options;
        use grit::serve3::journal::{Entry, Keep, Ledger, Made};
        let Some(host) = device("r3d") else {
            return;
        };
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("grit-r3d-{nanos}"));
        let dir = root.join("worlds");
        let keep = Keep::at(&dir).unwrap();
        let mut ledger = Ledger::created(Made {
            rule: "sand".to_owned(),
            params: Vec::new(),
            size: [32, 32, 32],
            sparse: true,
            slots: None,
        });
        for round in 0..4000_u32 {
            let cells: Vec<([u32; 3], u32)> = (0..64_u32)
                .map(|at| ([at % 32, (at / 32 + round) % 32, round % 32], 1 + round % 2))
                .collect();
            ledger.push(Entry::Write(cells), Duration::from_micros(10));
        }
        keep.begin(1, &ledger).unwrap();
        keep.set_next(2).unwrap();
        drop(keep);
        let options = Options {
            dir: Some(dir),
            memory: None,
        };
        let mut server = Server::with(Some(&host), &options).unwrap();
        for _ in 0..3 {
            let got = server
                .heard(&line(
                    "grit_world_create",
                    "{\"wide\":32,\"deep\":32,\"tall\":32,\"rule\":\"sand\",\"sparse\":true}",
                ))
                .unwrap();
            let world = got
                .get("result")
                .unwrap()
                .get("structuredContent")
                .unwrap()
                .get("world")
                .and_then(Json::int)
                .unwrap();
            server
                .heard(&line("grit_world_close", &format!("{{\"world\":{world}}}")))
                .unwrap();
        }
        let ask = |server: &mut Server<'_>, budget: u64| {
            let line = format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{{\"name\":\"grit_world_open\",\"arguments\":{{\"name\":\"world-1\",\"budget_ms\":{budget}}}}}}}"
            );
            let began = Instant::now();
            let got = server.heard(&line).unwrap();
            (got, began.elapsed())
        };
        let budget = 150_u64;
        let mut burned = 0;
        let mut late = 0;
        let mut refused = 0;
        let mut opened = false;
        for call in 0..8 {
            let (got, took) = ask(&mut server, budget);
            let text = written(&got);
            eprintln!("call {call}: {took:?}: {}", &text[..text.len().min(400)]);
            if got.get("result").and_then(|r| r.get("isError")) == Some(&Json::Bool(false)) {
                opened = true;
                break;
            }
            if took >= Duration::from_millis(budget) {
                burned += 1;
            }
            if took > Duration::from_millis(budget) + Duration::from_secs(1) {
                late += 1;
            }
            if text.contains("nothing ran") {
                refused += 1;
            }
        }
        let rates = written(&server.heard("{\"jsonrpc\":\"2.0\",\"id\":8,\"method\":\"tools/call\",\"params\":{\"name\":\"grit_describe\",\"arguments\":{}}}").unwrap());
        let at = rates.find("\"rates\"").unwrap_or(0);
        eprintln!("rates: {}", &rates[at..(at + 900).min(rates.len())]);
        let _ = std::fs::remove_dir_all(&root);
        if opened {
            eprintln!("opened within budget; scenario does not apply");
            return;
        }
        assert!(
            late == 0 && refused == 0,
            "a sparse replay that never fits {budget} ms ran its whole budget {burned} of 8 calls, {late} of them past it by more than 1 s, and was refused {refused} times without a finished replay"
        );
    }
}
