#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

#[cfg(feature = "faults")]
mod common;

use std::time::Duration;

use grit::serve3::budget::Rates;

#[test]
fn r4a_a_replay_cut_before_its_first_frame_must_not_break_every_later_least() {
    let mut rates = Rates::default();
    let had = Duration::from_millis(500);
    let recorded_total = Duration::from_secs(1);
    let least = had.as_secs_f64() / recorded_total.as_secs_f64();
    rates.measure_cut_replay(Duration::ZERO, Duration::from_millis(600), least);
    eprintln!(
        "replay_least ratio after a zero-frame cut: {:e}",
        rates.replay_least()
    );
    let got = std::panic::catch_unwind(|| rates.replaying_least(recorded_total));
    assert!(
        got.is_ok(),
        "replaying_least panicked after one replay cut before any frame ran (ratio {:e})",
        rates.replay_least()
    );
}

#[cfg(feature = "faults")]
mod served {
    use std::time::{Duration, Instant};

    use grit::serve3::journal::{Entry, Keep, Ledger, Made};
    use grit::serve3::{Options, Server};
    use grit::wire::{Json, written};

    use super::common::device;

    fn open(server: &mut Server<'_>, number: u32, budget: u64) -> (Json, Duration) {
        let line = format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{{\"name\":\"grit_world_open\",\"arguments\":{{\"name\":\"world-{number}\",\"budget_ms\":{budget}}}}}}}"
        );
        let began = Instant::now();
        let got = server.heard(&line).unwrap();
        (got, began.elapsed())
    }

    fn kept(label: &str, numbers: &[u32]) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("grit-{label}-{nanos}"));
        let dir = root.join("worlds");
        let keep = Keep::at(&dir).unwrap();
        for &number in numbers {
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
        keep.set_next(numbers.iter().max().unwrap() + 1).unwrap();
        root
    }

    #[test]
    fn r4b_one_replay_held_up_before_its_first_frame_breaks_every_later_open() {
        let Some(host) = device("r4b") else {
            return;
        };
        let root = kept("r4b", &[1, 2, 3]);
        let options = Options {
            dir: Some(root.join("worlds")),
            memory: None,
        };
        let mut server = Server::with(Some(&host), &options).unwrap();
        {
            let faults = server.faults();
            let mut armed = faults.lock().unwrap();
            armed.held_replays = 1;
            armed.held_by = Duration::from_millis(1500);
        }
        let (first, took) = open(&mut server, 1, 500);
        eprintln!("held-up open: {took:?}: {}", written(&first));
        assert_eq!(server.faults().lock().unwrap().held_replays, 0);
        let mut faulted = 0;
        for _ in 0..10 {
            let (got, _) = open(&mut server, 2, 600_000);
            if got.get("error").is_some() {
                faulted += 1;
            }
        }
        eprintln!("ten later opens of world-2 with 600 s each: {faulted} hit a server fault");
        assert_eq!(faulted, 0, "{faulted} of ten opens hit a server fault");
        let second = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            open(&mut server, 3, 600_000)
        }));
        let _ = std::fs::remove_dir_all(&root);
        let (got, took) = second.unwrap_or_else(|_| {
            panic!("the open after one held-up replay panicked instead of running or refusing")
        });
        eprintln!("next open with 600 s: {took:?}: {}", written(&got));
        assert_eq!(
            got.get("result").and_then(|r| r.get("isError")),
            Some(&Json::Bool(false)),
            "{}",
            written(&got)
        );
    }

    fn call(server: &mut Server<'_>, name: &str, arguments: &str) -> (Json, Duration) {
        let line = format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{{\"name\":\"{name}\",\"arguments\":{arguments}}}}}"
        );
        let began = Instant::now();
        let got = server.heard(&line).unwrap();
        (got.get("result").unwrap().clone(), began.elapsed())
    }

    fn fine(result: &Json) -> bool {
        result.get("isError") == Some(&Json::Bool(false))
    }

    #[test]
    fn r4c_a_save_that_fits_on_this_device_is_never_refused_by_the_prior() {
        let Some(host) = device("r4c") else {
            return;
        };
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("grit-r4c-{nanos}"));
        let options = Options {
            dir: Some(root.join("worlds")),
            memory: None,
        };
        let mut server = Server::with(Some(&host), &options).unwrap();
        let (made, _) = call(
            &mut server,
            "grit_world_create",
            "{\"wide\":256,\"deep\":256,\"tall\":256,\"rule\":\"sand\"}",
        );
        if !fine(&made) {
            eprintln!("no dense 256^3 world here: {}", written(&made));
            let _ = std::fs::remove_dir_all(&root);
            return;
        }
        let world = made
            .get("structuredContent")
            .unwrap()
            .get("world")
            .and_then(Json::int)
            .unwrap();
        let save = format!("{{\"world\":{world},\"name\":\"r4c\",\"replace\":true}}");
        let mut fastest = Duration::MAX;
        for _ in 0..3 {
            let (got, took) = call(&mut server, "grit_world_save", &save);
            assert!(fine(&got), "{}", written(&got));
            fastest = fastest.min(took);
        }
        let budget = fastest.as_millis() * 2 + 5;
        let with = format!(
            "{{\"world\":{world},\"name\":\"r4c\",\"replace\":true,\"budget_ms\":{budget}}}"
        );
        let (got, took) = call(&mut server, "grit_world_save", &with);
        eprintln!(
            "three saves of 256^3, fastest {fastest:?}; with {budget} ms: {took:?} {}",
            written(&got)
        );
        let (open_got, open_took) = call(&mut server, "grit_world_open", "{\"name\":\"r4c\"}");
        eprintln!(
            "open {open_took:?}: {}",
            &written(&open_got)[..200.min(written(&open_got).len())]
        );
        let _ = std::fs::remove_dir_all(&root);
        assert!(
            fine(&got),
            "a save the server ran three times, fastest in {fastest:?}, was refused with {budget} ms: {}",
            written(&got)
        );
    }
}
