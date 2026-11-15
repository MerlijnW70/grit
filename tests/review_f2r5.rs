#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

#[cfg(feature = "faults")]
mod common;

#[cfg(feature = "faults")]
mod served {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
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
        (got.get("result").cloned().unwrap_or(got), began.elapsed())
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

    fn fine(result: &Json) -> bool {
        result.get("isError") == Some(&Json::Bool(false))
    }

    fn cancelled_open(server: &mut Server<'_>, number: u32) -> Duration {
        let faults = server.faults();
        {
            let mut armed = faults.lock().unwrap();
            armed.held_replays = 1;
            armed.held_by = Duration::from_secs(6);
        }
        let flag = Arc::new(AtomicBool::new(false));
        let setter = {
            let flag = Arc::clone(&flag);
            let faults = Arc::clone(&faults);
            std::thread::spawn(move || {
                while faults.lock().unwrap().held_replays > 0 {
                    std::thread::sleep(Duration::from_millis(5));
                }
                std::thread::sleep(Duration::from_secs(2));
                flag.store(true, Ordering::SeqCst);
            })
        };
        server.arriving(None, Some(Arc::clone(&flag)));
        let (cancelled, took) = open(server, number, 600_000);
        server.arriving(None, None);
        setter.join().unwrap();
        eprintln!(
            "cancelled open of world-{number} with 600 s: {took:?}: {}",
            written(&cancelled)
        );
        let message = written(&cancelled);
        assert!(
            message.contains("cancelled") && message.contains("to frame 0 of 400"),
            "setup: the open was not cancelled inside its step: {}",
            written(&cancelled)
        );
        took
    }

    fn server_with<'host>(host: &'host grit::Host, root: &std::path::Path) -> Server<'host> {
        let options = Options {
            dir: Some(root.join("worlds")),
            memory: None,
        };
        Server::with(Some(host), &options).unwrap()
    }

    #[test]
    fn r5a_one_cancelled_replay_is_recorded_as_needing_the_whole_budget() {
        let Some(host) = device("r5a") else {
            return;
        };
        let root = kept("r5a", &[1, 2]);
        let mut server = server_with(&host, &root);
        let took = cancelled_open(&mut server, 1);
        let (later, later_took) = open(&mut server, 2, 600_000);
        let _ = std::fs::remove_dir_all(&root);
        eprintln!(
            "next open of an identical world with 600 s: {later_took:?}: {}",
            written(&later)
        );
        assert!(
            fine(&later),
            "one replay cancelled after {took:?} made the server refuse a 600 s open of an identical world: {}",
            written(&later)
        );
    }

    #[test]
    fn r5c_one_replay_that_waited_in_the_queue_locks_out_every_later_open() {
        let Some(host) = device("r5c") else {
            return;
        };
        let root = kept("r5c", &[1, 2]);
        let mut server = server_with(&host, &root);
        {
            let faults = server.faults();
            let mut armed = faults.lock().unwrap();
            armed.held_replays = 1;
            armed.held_by = Duration::from_millis(1500);
        }
        let waited = Duration::from_millis(599_500);
        server.arriving(Some(Instant::now().checked_sub(waited).unwrap()), None);
        let (first, took) = open(&mut server, 1, 600_000);
        server.arriving(None, None);
        eprintln!(
            "open queued {waited:?} with 600 s, held up: {took:?}: {}",
            written(&first)
        );
        assert!(
            written(&first).contains("to frame 0 of 400"),
            "setup: the queued open was not cut inside its step: {}",
            written(&first)
        );
        let (later, later_took) = open(&mut server, 2, 600_000);
        let _ = std::fs::remove_dir_all(&root);
        eprintln!(
            "next open of an identical world with 600 s: {later_took:?}: {}",
            written(&later)
        );
        assert!(
            fine(&later),
            "one replay that had 0.5 s of its 600 s budget after queueing made the server refuse a 600 s open of an identical world: {}",
            written(&later)
        );
    }
}
