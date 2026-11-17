#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

#[cfg(feature = "faults")]
mod common;

use std::time::Duration;

use grit::serve3::budget::{BUDGET_MS, Rates, WORLD_LEAST, Work};

#[test]
fn r1_build_that_never_fits_runs_every_call_when_small_builds_interleave() {
    let budget = Duration::from_millis(BUDGET_MS);
    let per_byte = 5e-8;
    let cost = |bytes: u64| Duration::from_secs_f64(0.05 + per_byte * bytes as f64);
    let large: u64 = 800_000_000;
    assert!(cost(large) > budget);
    let mut rates = Rates::default();
    rates.measure(Work::Build, large, cost(large));
    let mut overran = 0;
    for _ in 0..20 {
        rates.measure(Work::Build, WORLD_LEAST, cost(WORLD_LEAST));
        let least = rates.least(Work::Build, large);
        if least < budget {
            overran += 1;
            rates.measure(Work::Build, large, cost(large));
        }
    }
    eprintln!(
        "least now {:?}, true {:?}, overran {overran} of 20",
        rates.least(Work::Build, large),
        cost(large)
    );
    assert!(
        overran <= 3,
        "a build every sample says takes {:?} ran past the {budget:?} budget {overran} times",
        cost(large)
    );
}

#[test]
fn r2_fresh_server_refuses_journal_writes_by_the_prior_alone() {
    let budget = Duration::from_millis(BUDGET_MS);
    let least = Rates::default().least(Work::Write, 168_632);
    eprintln!("fresh least for 168632 write bytes: {least:?}");
    assert!(least < budget, "{least:?}");
}

#[cfg(feature = "faults")]
mod served {
    use std::time::{Duration, Instant};

    use grit::serve3::journal::{Entry, Keep, Ledger, Made};
    use grit::serve3::{Options, Server};
    use grit::wire::{Json, written};

    use super::common::device;

    #[test]
    fn r3_restarted_server_refuses_to_open_a_kept_world_whose_writes_fit() {
        let Some(host) = device("restart writes") else {
            return;
        };
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("grit-r2-{nanos}"));
        let dir = root.join("worlds");
        let keep = Keep::at(&dir).unwrap();
        let mut ledger = Ledger::created(Made {
            rule: "sand".to_owned(),
            params: Vec::new(),
            size: [32, 32, 32],
            sparse: false,
            slots: None,
        });
        let cells: Vec<([u32; 3], u32)> = (0..10_600_u32)
            .map(|at| ([at % 32, (at / 32) % 32, at / 1024], 1))
            .collect();
        ledger.push(Entry::Write(cells), Duration::from_millis(5));
        keep.begin(1, &ledger).unwrap();
        keep.set_next(2).unwrap();
        drop(keep);
        let options = Options {
            dir: Some(dir),
            memory: None,
        };
        let mut server = Server::with(Some(&host), &options).unwrap();
        let ask = |server: &mut Server<'_>, budget: u64| {
            let line = format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{{\"name\":\"grit_world_open\",\"arguments\":{{\"name\":\"world-1\",\"budget_ms\":{budget}}}}}}}"
            );
            let began = Instant::now();
            let got = server.heard(&line).unwrap();
            (got, began.elapsed())
        };
        let (first, took) = ask(&mut server, 30_000);
        eprintln!("30 s budget, {took:?}: {}", written(&first));
        let (second, took2) = ask(&mut server, 600_000);
        eprintln!("600 s budget, {took2:?}: {}", written(&second));
        let _ = std::fs::remove_dir_all(&root);
        let refused = first.get("result").and_then(|r| r.get("isError")) == Some(&Json::Bool(true));
        let said = written(&first);
        assert!(
            !refused
                || (said.contains("before the call stopped on budget")
                    && !said.contains("nothing ran")),
            "refused a replay before running it, which then ran in {took2:?}: {said}"
        );
    }
}
