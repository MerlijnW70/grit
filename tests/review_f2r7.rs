#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

#[cfg(feature = "faults")]
mod common;

#[cfg(feature = "faults")]
mod served {
    use std::time::{Duration, Instant};

    use grit::serve3::journal::{Entry, Keep, Ledger, Made};
    use grit::serve3::{Options, Server};
    use grit::wire::{Json, written};

    use super::common::device;

    fn call(server: &mut Server<'_>, tool: &str, arguments: &str) -> Json {
        let line = format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{{\"name\":\"{tool}\",\"arguments\":{arguments}}}}}"
        );
        let got = server.heard(&line).unwrap();
        got.get("result").cloned().unwrap_or(got)
    }

    fn open(server: &mut Server<'_>, number: u32, budget: Duration) -> (Json, Duration) {
        let began = Instant::now();
        let got = call(
            server,
            "grit_world_open",
            &format!(
                "{{\"name\":\"world-{number}\",\"budget_ms\":{}}}",
                budget.as_millis()
            ),
        );
        (got, began.elapsed())
    }

    fn fine(result: &Json) -> bool {
        result.get("isError") == Some(&Json::Bool(false))
    }

    const SIDE: u32 = 64;

    const FRAMES: u32 = 400;

    const STALL: Duration = Duration::from_millis(1);

    fn kept(label: &str, numbers: &[u32]) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("grit-{label}-{}-{nanos}", std::process::id()));
        let keep = Keep::at(&root.join("worlds")).unwrap();
        for &number in numbers {
            let mut ledger = Ledger::created(Made {
                rule: "sand".to_owned(),
                params: Vec::new(),
                size: [SIDE, SIDE, SIDE],
                sparse: false,
                slots: None,
            });
            ledger.push(Entry::Step(FRAMES), Duration::from_secs(1));
            keep.begin(number, &ledger).unwrap();
        }
        keep.set_next(numbers.iter().max().unwrap() + 1).unwrap();
        root
    }

    fn hold(server: &Server<'_>, by: Duration) {
        let faults = server.faults();
        let mut armed = faults.lock().unwrap();
        armed.held_replays = 1;
        armed.held_by = by;
    }

    fn setup(label: &str) -> Option<(grit::Host, std::path::PathBuf)> {
        let host = device(label)?;
        let numbers: Vec<u32> = (1..=40).collect();
        Some((host, kept(label, &numbers)))
    }

    fn cuts(server: &mut Server<'_>, fit: Duration) {
        for number in 3..=4 {
            let given = fit + Duration::from_millis(30);
            hold(server, given + Duration::from_millis(200));
            let (cut, _) = open(server, number, given);
            assert!(
                written(&cut).contains("stopped on budget"),
                "setup: world-{number} not cut: {}",
                written(&cut)
            );
        }
        let generous = Duration::from_millis(3000);
        hold(server, generous.saturating_sub(fit / 2));
        let (cut, took) = open(server, 5, generous);
        eprintln!("held-up open of world-5 with {generous:?}: {took:?}");
        assert!(
            written(&cut).contains("stopped on budget"),
            "setup: world-5 not cut: {}",
            written(&cut)
        );
        server.faults().lock().unwrap().held_replays = 0;
    }

    fn fitted(server: &mut Server<'_>) -> Duration {
        let (first, one) = open(server, 1, Duration::from_secs(60));
        assert!(fine(&first), "setup: {}", written(&first));
        let (second, two) = open(server, 2, Duration::from_secs(60));
        assert!(fine(&second), "setup: {}", written(&second));
        eprintln!("whole opens of world-1 and world-2: {one:?}, {two:?}");
        one.max(two)
    }

    #[test]
    fn r7a_one_held_up_open_with_a_large_budget_refuses_identical_opens_that_fit() {
        let Some((host, root)) = setup("r7a") else {
            return;
        };
        let options = Options {
            dir: Some(root.join("worlds")),
            memory: None,
        };
        let mut server = Server::with(Some(&host), &options).unwrap();
        grit::serve3::tools::faults::stall_every(STALL);
        let fit = fitted(&mut server);
        cuts(&mut server, fit);
        let given = fit * 2 + Duration::from_millis(50);
        let mut refused = 0;
        let mut first = None;
        let mut ran = None;
        for number in 10..20 {
            let (later, took) = open(&mut server, number, given);
            if fine(&later) {
                ran = Some(took);
                break;
            }
            refused += 1;
            first.get_or_insert(later);
        }
        grit::serve3::tools::faults::disarm();
        drop(server);
        let _ = std::fs::remove_dir_all(&root);
        if let Some(first) = first {
            panic!(
                "identical opens finished whole in at most {fit:?}; two opens were cut with {:?} and one open given 3 s was held up in its step; then an identical open given {given:?} was refused {refused} times in a row (then ran in {ran:?}), saying: {}",
                fit + Duration::from_millis(30),
                written(&first)
            );
        }
    }

    #[test]
    fn r7b_same_size_creates_reset_the_remeasure_so_the_held_up_refusal_never_expires() {
        let Some((host, root)) = setup("r7b") else {
            return;
        };
        let options = Options {
            dir: Some(root.join("worlds")),
            memory: None,
        };
        let mut server = Server::with(Some(&host), &options).unwrap();
        grit::serve3::tools::faults::stall_every(STALL);
        let fit = fitted(&mut server);
        cuts(&mut server, fit);
        let given = fit * 2 + Duration::from_millis(50);
        let mut refused = 0;
        let mut first = None;
        let mut ran = None;
        for number in 10..30 {
            let (later, took) = open(&mut server, number, given);
            if fine(&later) {
                ran = Some(took);
                break;
            }
            refused += 1;
            first.get_or_insert(later);
            let made = call(
                &mut server,
                "grit_world_create",
                &format!(
                    "{{\"wide\":{SIDE},\"deep\":{SIDE},\"tall\":{SIDE},\"rule\":\"sand\",\"budget_ms\":60000}}"
                ),
            );
            assert!(fine(&made), "setup: {}", written(&made));
            let world = made
                .get("structuredContent")
                .and_then(|told| told.get("world"))
                .cloned()
                .unwrap();
            let closed = call(
                &mut server,
                "grit_world_close",
                &format!("{{\"world\":{}}}", grit::wire::written(&world)),
            );
            assert!(fine(&closed), "setup: {}", written(&closed));
        }
        grit::serve3::tools::faults::disarm();
        drop(server);
        let _ = std::fs::remove_dir_all(&root);
        if let Some(first) = first {
            panic!(
                "identical opens finished whole in at most {fit:?}; after one held-up open among three cuts, an identical open given {given:?} was refused {refused} times in a row with a same-size create between each (then ran in {ran:?}); the 8-refusal remeasure never came: {}",
                written(&first)
            );
        }
    }

    fn reached(text: &str) -> u64 {
        let at = text
            .find("to frame ")
            .unwrap_or_else(|| panic!("no frame in {text}"));
        text[at + 9..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>()
            .parse()
            .unwrap()
    }

    fn shaped(label: &str, worlds: &[(u32, u32, Duration)]) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("grit-{label}-{}-{nanos}", std::process::id()));
        let keep = Keep::at(&root.join("worlds")).unwrap();
        for &(number, frames, recorded) in worlds {
            let mut ledger = Ledger::created(Made {
                rule: "sand".to_owned(),
                params: Vec::new(),
                size: [SIDE, SIDE, SIDE],
                sparse: false,
                slots: None,
            });
            ledger.push(Entry::Step(frames), recorded);
            keep.begin(number, &ledger).unwrap();
        }
        keep.set_next(worlds.iter().map(|one| one.0).max().unwrap() + 1)
            .unwrap();
        root
    }

    #[test]
    fn r7c_finished_replays_of_another_journal_keep_work_that_cannot_fit_from_ever_being_refused() {
        let Some(host) = device("r7c") else {
            return;
        };
        let mut worlds = Vec::new();
        for round in 0..12 {
            worlds.push((10 + round, 40, Duration::from_secs(1)));
            worlds.push((40 + round, FRAMES, Duration::from_millis(100)));
        }
        let root = shaped("r7c", &worlds);
        let options = Options {
            dir: Some(root.join("worlds")),
            memory: None,
        };
        let mut server = Server::with(Some(&host), &options).unwrap();
        grit::serve3::tools::faults::stall_every(STALL);
        let short = Duration::from_millis(200);
        let most = u64::try_from(short.as_nanos() / STALL.as_nanos()).unwrap()
            + u64::from(grit::serve3::budget::BATCH_MOST);
        let mut cut = 0;
        let mut refused = None;
        let mut late = None;
        let mut last = None;
        for round in 0..12 {
            let (other, _) = open(&mut server, 10 + round, Duration::from_secs(60));
            assert!(fine(&other), "setup: {}", written(&other));
            let (slow, took) = open(&mut server, 40 + round, short);
            let text = written(&slow);
            assert!(
                !fine(&slow),
                "setup: {FRAMES} frames under a {STALL:?} stall each ran in {took:?}"
            );
            if text.contains("whose build alone") {
                continue;
            }
            if text.contains("nothing ran") {
                refused = Some(round);
                break;
            }
            assert!(text.contains("stopped on budget"), "setup: {text}");
            if reached(&text) > most {
                late.get_or_insert(took);
            }
            cut += 1;
            last = Some(text);
        }
        grit::serve3::tools::faults::disarm();
        drop(server);
        let _ = std::fs::remove_dir_all(&root);
        assert!(
            refused.is_none() && late.is_none(),
            "a replay of {FRAMES} frames, each held {STALL:?}, can never fit {short:?} and never finished, so it must be cut by its budget and one batch ({most} frames) every time and never refused, whatever another journal finished: cut {cut} times, refused at round {refused:?}, first late cut after {late:?}; last: {}",
            last.unwrap_or_default()
        );
    }
}
