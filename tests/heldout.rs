#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use grit::Host;
use grit::serve::handle;
use grit::wire::{Json, read, written};

fn call(host: Option<&Host>, name: &str, arguments: &str) -> Json {
    let asked = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{{\"name\":\"{name}\",\"arguments\":{arguments}}}}}"
    );
    handle(host, &read(&asked).unwrap()).unwrap()
}

fn structured(answer: &Json) -> Json {
    answer
        .get("result")
        .and_then(|one| one.get("structuredContent"))
        .cloned()
        .unwrap_or(Json::Null)
}

fn detector(seed: u32, steps: u32) -> String {
    let mut state = seed.wrapping_mul(2_654_435_761) | 1;
    let mut feed = Vec::new();
    let mut want = Vec::new();
    let mut last = [0_u32; 3];
    for _ in 0..steps {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        let on = state & 1;
        last = [last[1], last[2], on];
        feed.push(on.to_string());
        want.push(u32::from(last == [1, 0, 1]).to_string());
    }
    format!(
        "{{\"feed\":[{}],\"want\":[{}]}}",
        feed.join(","),
        want.join(",")
    )
}

fn runs(from: u32, many: u32, steps: u32) -> String {
    (from..from + many)
        .map(|seed| detector(seed, steps))
        .collect::<Vec<String>>()
        .join(",")
}

fn tried(host: Option<&Host>, many: u32, steps: u32) -> (bool, u32, u32) {
    let taught = structured(&call(
        host,
        "grit_machine",
        &format!(
            "{{\"state\":2,\"inputs\":1,\"outputs\":1,\"gates\":6,\"crowd\":4096,\"rounds\":2000,\"traces\":[{}]}}",
            runs(0, many, steps)
        ),
    ));
    let fit = taught.get("solved") == Some(&Json::Bool(true));
    let gates = written(taught.get("gates").unwrap());
    let held = structured(&call(
        None,
        "grit_machine",
        &format!(
            "{{\"state\":2,\"inputs\":1,\"outputs\":1,\"gates\":6,\"chip\":{gates},\"traces\":[{}]}}",
            runs(500, 8, 32)
        ),
    ));
    (
        fit,
        held.counted("marks").unwrap(),
        held.counted("of").unwrap(),
    )
}

#[test]
fn a_machine_that_answers_every_run_it_was_taught_can_still_miss_runs_it_never_saw() {
    let Some(host) = Host::open().unwrap() else {
        eprintln!("SKIPPED no device");
        return;
    };
    let (fit, marks, of) = tried(Some(&host), 3, 8);
    eprintln!("taught on three runs of eight steps: fit {fit}, unseen {marks} of {of}");
    assert!(fit, "it answers every run it was given");
    assert!(
        marks < of,
        "solved is a word about the runs it was handed, not about the rule behind them"
    );
}

#[test]
fn longer_runs_pin_the_same_machine_down_on_runs_it_never_saw() {
    let Some(host) = Host::open().unwrap() else {
        eprintln!("SKIPPED no device");
        return;
    };
    let (fit, marks, of) = tried(Some(&host), 3, 32);
    eprintln!("taught on three runs of thirty two steps: fit {fit}, unseen {marks} of {of}");
    assert!(fit);
    assert_eq!(
        marks, of,
        "the same three runs, four times as long, hold on everything it never saw"
    );
}
