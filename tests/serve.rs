#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use grit::Host;
use grit::serve::handle;
use grit::wire::{Json, read, written};

fn device(label: &str) -> Option<Host> {
    match Host::open() {
        Ok(Some(host)) => Some(host),
        Ok(None) => {
            eprintln!("SKIPPED {label}");
            None
        }
        Err(why) => {
            eprintln!("SKIPPED {label} {why}");
            None
        }
    }
}

fn adder() -> String {
    let rows: Vec<String> = (0..16_u32)
        .map(|row| ((row & 3) + ((row >> 2) & 3)).to_string())
        .collect();
    rows.join(",")
}

fn call(host: Option<&Host>, name: &str, arguments: &str) -> Json {
    let asked = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{{\"name\":\"{name}\",\"arguments\":{arguments}}}}}"
    );
    let request = read(&asked).unwrap();
    handle(host, &request).unwrap()
}

fn structured(answer: &Json) -> Json {
    answer
        .get("result")
        .and_then(|one| one.get("structuredContent"))
        .cloned()
        .unwrap_or(Json::Null)
}

#[test]
fn a_torn_line_is_read_back_as_it_was_written() {
    let held = "{\"a\":[1,-2,true,null,\"one\ntwo\"],\"b\":{\"c\":9}}";
    let one = read(held).unwrap();
    let again = read(&written(&one)).unwrap();
    assert_eq!(one, again);
    assert_eq!(one.get("b").and_then(|one| one.counted("c")), Some(9));
}

#[test]
fn the_server_names_the_three_tools_it_has() {
    let asked = read("{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}").unwrap();
    let answer = handle(None, &asked).unwrap();
    let tools = answer
        .get("result")
        .and_then(|one| one.get("tools"))
        .and_then(Json::list)
        .unwrap()
        .to_vec();
    let names: Vec<&str> = tools
        .iter()
        .filter_map(|one| one.get("name").and_then(Json::text))
        .collect();
    assert_eq!(names, vec!["grit_synthesise", "grit_least", "grit_check"]);
}

#[test]
fn a_method_it_does_not_know_is_refused_rather_than_guessed() {
    let asked = read("{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/dream\"}").unwrap();
    let answer = handle(None, &asked).unwrap();
    assert_eq!(
        answer.get("error").and_then(|one| one.counted("code")),
        None,
        "the code is negative and does not fit a u32"
    );
    assert!(answer.get("error").is_some(), "{}", written(&answer));
}

#[test]
fn a_notification_gets_no_answer() {
    let asked = read("{\"jsonrpc\":\"2.0\",\"method\":\"initialized\"}").unwrap();
    assert!(handle(None, &asked).is_none());
}

#[test]
fn synthesising_an_adder_returns_a_circuit_the_host_marked_itself() {
    let Some(host) = device("synthesising_an_adder_returns_a_circuit_the_host_marked_itself")
    else {
        return;
    };
    let arguments = format!(
        "{{\"inputs\":4,\"outputs\":3,\"gates\":10,\"table\":[{}],\"crowd\":16384,\"rounds\":2000}}",
        adder()
    );
    let answer = call(Some(&host), "grit_synthesise", &arguments);
    let held = structured(&answer);
    eprintln!("{}", written(&held));
    assert_eq!(
        held.get("solved"),
        Some(&Json::Bool(true)),
        "{}",
        written(&held)
    );
    assert_eq!(held.counted("marks"), held.counted("of"));
    assert_eq!(held.get("checked_on_the_host"), Some(&Json::Bool(true)));
    assert_eq!(
        held.get("wrong_rows").and_then(Json::list).unwrap().len(),
        0
    );

    let gates = written(held.get("gates").unwrap());
    let again = format!(
        "{{\"inputs\":4,\"outputs\":3,\"gates\":10,\"table\":[{}],\"chip\":{gates}}}",
        adder()
    );
    let marked = structured(&call(Some(&host), "grit_check", &again));
    assert_eq!(
        marked.get("solved"),
        Some(&Json::Bool(true)),
        "{}",
        written(&marked)
    );
    assert_eq!(marked.counted("marks"), held.counted("of"));
}

#[test]
fn the_ladder_walks_the_budget_down_and_stops_where_it_cannot() {
    let Some(host) = device("the_ladder_walks_the_budget_down_and_stops_where_it_cannot") else {
        return;
    };
    let arguments = format!(
        "{{\"inputs\":4,\"outputs\":3,\"gates\":10,\"table\":[{}],\"crowd\":16384,\"rounds\":1500}}",
        adder()
    );
    let held = structured(&call(Some(&host), "grit_least", &arguments));
    let rungs = held.get("ladder").and_then(Json::list).unwrap().to_vec();
    eprintln!(
        "{:>7} {:>8} {:>8} {:>8}",
        "gates", "solved", "marks", "rounds"
    );
    for rung in &rungs {
        eprintln!(
            "{:>7} {:>8} {:>8} {:>8}",
            rung.counted("gates").unwrap(),
            matches!(rung.get("solved"), Some(Json::Bool(true))),
            rung.counted("marks").unwrap(),
            rung.counted("rounds").unwrap()
        );
    }
    let smallest = held.counted("smallest_that_solved").unwrap();
    eprintln!("the smallest budget that solved it is {smallest} gates");

    assert!(rungs.len() >= 2, "the ladder has only one rung");
    let last = rungs.last().unwrap();
    assert_eq!(
        last.get("solved"),
        Some(&Json::Bool(false)),
        "the ladder should stop on a rung it could not solve"
    );
    for rung in &rungs[..rungs.len() - 1] {
        assert_eq!(rung.get("solved"), Some(&Json::Bool(true)));
    }
    assert_eq!(smallest, last.counted("gates").unwrap() + 1);
    let circuit = held.get("circuit").unwrap();
    assert_eq!(circuit.get("solved"), Some(&Json::Bool(true)));
    assert_eq!(
        circuit.get("gates").and_then(Json::list).unwrap().len(),
        smallest as usize
    );
    assert_eq!(circuit.get("checked_on_the_host"), Some(&Json::Bool(true)));
}

#[test]
fn a_budget_that_cannot_hold_it_comes_back_as_evidence_rather_than_a_guess() {
    let Some(host) =
        device("a_budget_that_cannot_hold_it_comes_back_as_evidence_rather_than_a_guess")
    else {
        return;
    };
    let arguments = format!(
        "{{\"inputs\":4,\"outputs\":3,\"gates\":4,\"table\":[{}],\"crowd\":4096,\"rounds\":600}}",
        adder()
    );
    let held = structured(&call(Some(&host), "grit_synthesise", &arguments));
    eprintln!("{}", written(&held));
    assert_eq!(held.get("solved"), Some(&Json::Bool(false)));
    assert!(held.counted("marks").unwrap() < held.counted("of").unwrap());
    assert_eq!(held.counted("rounds"), Some(600));
    assert_eq!(held.counted("crowd"), Some(4096));
    assert!(
        !held
            .get("wrong_rows")
            .and_then(Json::list)
            .unwrap()
            .is_empty()
    );
}
