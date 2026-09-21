#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use grit::Host;
use grit::serve::{handle, seeded};
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
fn the_server_names_every_tool_it_has() {
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
    assert_eq!(
        names,
        vec![
            "grit_machine",
            "grit_synthesise",
            "grit_least",
            "grit_slope",
            "grit_settle",
            "grit_pour",
            "grit_check"
        ]
    );
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

fn counting(steps: u32) -> String {
    let mut runs = Vec::new();
    for seed in 0..4_u32 {
        let mut state = seed.wrapping_mul(2_654_435_761) | 1;
        let mut count = 0_u32;
        let mut feed = Vec::new();
        let mut want = Vec::new();
        for _ in 0..steps {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            let on = state & 1;
            want.push(count);
            feed.push(on);
            if on == 1 {
                count = (count + 1) & 3;
            }
        }
        let feed: Vec<String> = feed.iter().map(u32::to_string).collect();
        let want: Vec<String> = want.iter().map(u32::to_string).collect();
        runs.push(format!(
            "{{\"feed\":[{}],\"want\":[{}]}}",
            feed.join(","),
            want.join(",")
        ));
    }
    runs.join(",")
}

#[test]
fn a_machine_that_counts_when_told_to_is_found_from_its_runs() {
    let Some(host) = device("a_machine_that_counts_when_told_to_is_found_from_its_runs") else {
        return;
    };
    let arguments = format!(
        "{{\"state\":2,\"inputs\":1,\"outputs\":2,\"gates\":12,\"traces\":[{}],\"crowd\":65536,\"rounds\":2000}}",
        counting(16)
    );
    let held = structured(&call(Some(&host), "grit_machine", &arguments));
    eprintln!(
        "solved {:?} marks {:?} of {:?} in {:?} rounds over {:?} runs",
        held.get("solved"),
        held.counted("marks"),
        held.counted("of"),
        held.counted("rounds"),
        held.counted("traces")
    );
    let said = held.get("said").and_then(Json::list).unwrap();
    eprintln!("the first run comes back as {}", written(&said[0]));
    assert_eq!(
        held.get("solved"),
        Some(&Json::Bool(true)),
        "{}",
        written(&held)
    );
    assert_eq!(held.counted("marks"), held.counted("of"));
    assert_eq!(held.get("wrong").and_then(Json::list).unwrap().len(), 0);
    assert_eq!(held.get("checked_on_the_host"), Some(&Json::Bool(true)));
    assert_eq!(said.len(), 4, "one answer a run");
}

#[test]
fn a_machine_given_its_gates_is_marked_rather_than_searched_and_needs_no_device() {
    let counter = "[{\"kind\":8,\"left\":0,\"right\":2},{\"kind\":6,\"left\":0,\"right\":2},{\"kind\":6,\"left\":1,\"right\":3},{\"kind\":8,\"left\":0,\"right\":0},{\"kind\":8,\"left\":1,\"right\":1}]";
    let arguments = format!(
        "{{\"state\":2,\"inputs\":1,\"outputs\":2,\"gates\":5,\"traces\":[{}],\"chip\":{counter}}}",
        counting(16)
    );
    let held = structured(&call(None, "grit_machine", &arguments));
    eprintln!("{}", written(&held));
    assert_eq!(
        held.get("solved"),
        Some(&Json::Bool(true)),
        "a counter written out by hand should count: {}",
        written(&held)
    );
    assert_eq!(held.counted("marks"), held.counted("of"));
    assert_eq!(held.counted("rounds"), Some(0), "nothing was searched");
    assert_eq!(held.get("wrong").and_then(Json::list).unwrap().len(), 0);

    let broken = counter.replace(
        "{\"kind\":6,\"left\":0,\"right\":2}",
        "{\"kind\":8,\"left\":0,\"right\":2}",
    );
    let arguments = format!(
        "{{\"state\":2,\"inputs\":1,\"outputs\":2,\"gates\":5,\"traces\":[{}],\"chip\":{broken}}}",
        counting(16)
    );
    let held = structured(&call(None, "grit_machine", &arguments));
    assert_eq!(
        held.get("solved"),
        Some(&Json::Bool(false)),
        "one gate turned from xor to and should stop it counting"
    );
    let wrong = held.get("wrong").and_then(Json::list).unwrap();
    eprintln!(
        "the broken one misses {} steps, first {}",
        wrong.len(),
        written(&wrong[0])
    );
    assert!(!wrong.is_empty(), "it should name the steps it missed");
}

#[test]
fn a_machine_with_no_room_to_remember_cannot_count_and_says_which_steps_it_missed() {
    let Some(host) =
        device("a_machine_with_no_room_to_remember_cannot_count_and_says_which_steps_it_missed")
    else {
        return;
    };
    let arguments = format!(
        "{{\"state\":0,\"inputs\":1,\"outputs\":2,\"gates\":8,\"traces\":[{}],\"crowd\":4096,\"rounds\":400}}",
        counting(16)
    );
    let held = structured(&call(Some(&host), "grit_machine", &arguments));
    eprintln!(
        "with nothing to remember: marks {:?} of {:?}, {} steps wrong",
        held.counted("marks"),
        held.counted("of"),
        held.get("wrong").and_then(Json::list).unwrap().len()
    );
    assert_eq!(held.get("solved"), Some(&Json::Bool(false)));
    assert!(!held.get("wrong").and_then(Json::list).unwrap().is_empty());
    assert_eq!(
        held.counted("crowd"),
        Some(65536),
        "a machine that cannot be found is pressed to the ceiling before it is called impossible: {}",
        written(&held)
    );
}

#[test]
fn a_machine_that_is_found_keeps_the_crowd_it_was_given() {
    let Some(host) = device("a_machine_that_is_found_keeps_the_crowd_it_was_given") else {
        return;
    };
    let arguments = "{\"state\":1,\"inputs\":1,\"outputs\":1,\"gates\":3,\"crowd\":4096,\"rounds\":600,\"traces\":[{\"feed\":[1,0,1,1,0],\"want\":[0,1,0,1,1]},{\"feed\":[0,0,1,0,1],\"want\":[0,0,0,1,0]}]}";
    let held = structured(&call(Some(&host), "grit_machine", arguments));
    eprintln!(
        "a one step delay: solved {:?}, crowd {:?}",
        held.get("solved"),
        held.counted("crowd")
    );
    assert_eq!(
        held.get("solved"),
        Some(&Json::Bool(true)),
        "one bit of memory is enough to say back what it was just told: {}",
        written(&held)
    );
    assert_eq!(
        held.counted("crowd"),
        Some(4096),
        "nothing to raise when it solved: {}",
        written(&held)
    );
}

#[test]
fn the_ladder_walks_the_budget_down_and_ends_on_two_failures_in_a_row() {
    let Some(host) = device("the_ladder_walks_the_budget_down_and_ends_on_two_failures_in_a_row")
    else {
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

    assert!(
        rungs.len() >= 3,
        "the ladder is too short to have been walked"
    );
    for rung in &rungs[rungs.len() - 2..] {
        assert_eq!(
            rung.get("solved"),
            Some(&Json::Bool(false)),
            "the ladder ends only after two rungs in a row fail"
        );
    }
    assert_eq!(
        rungs
            .iter()
            .rev()
            .take_while(|rung| matches!(rung.get("solved"), Some(Json::Bool(false))))
            .count(),
        2,
        "two failures in a row end it: not one, and not every rung down to the floor"
    );
    let standing: Vec<u32> = rungs
        .iter()
        .filter(|rung| matches!(rung.get("solved"), Some(Json::Bool(true))))
        .map(|rung| rung.counted("gates").unwrap())
        .collect();
    assert_eq!(
        smallest,
        standing.iter().copied().min().unwrap(),
        "the smallest is the smallest that solved anywhere on the ladder, hole or no hole"
    );
    assert!(
        held.get("best_attempt").is_none(),
        "a ladder that solved something answers with the circuit, not with what it reached"
    );
    assert!(
        held.get("nothing_solved").is_none(),
        "nothing to explain when a budget did solve"
    );
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
    assert_eq!(
        held.counted("crowd"),
        Some(65536),
        "evidence about a gate budget means the crowd was pressed to the ceiling, not that 4096 was asked for: {}",
        written(&held)
    );
    assert!(
        !held
            .get("wrong_rows")
            .and_then(Json::list)
            .unwrap()
            .is_empty()
    );
}

fn tool(named: &str) -> Json {
    let asked = read("{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}").unwrap();
    let answer = handle(None, &asked).unwrap();
    answer
        .get("result")
        .and_then(|one| one.get("tools"))
        .and_then(Json::list)
        .unwrap()
        .iter()
        .find(|one| one.get("name").and_then(Json::text) == Some(named))
        .cloned()
        .unwrap()
}

#[test]
fn the_machine_tool_says_it_can_mark_a_chip_it_is_handed() {
    let held = tool("grit_machine");
    let properties = held
        .get("inputSchema")
        .and_then(|one| one.get("properties"))
        .unwrap();
    assert!(
        properties.get("chip").is_some(),
        "a machine can be marked rather than searched, so the schema must offer it: {}",
        written(&held)
    );
    let said = held.get("description").and_then(Json::text).unwrap();
    assert!(said.contains("chip"), "{said}");
}

#[test]
fn a_budget_refused_before_any_search_says_what_is_wrong_in_words() {
    let answer = call(
        None,
        "grit_synthesise",
        "{\"inputs\":2,\"outputs\":1,\"gates\":0,\"table\":[0,1,1,0]}",
    );
    let result = answer.get("result").unwrap();
    assert_eq!(result.get("isError"), Some(&Json::Bool(true)));
    let said = result
        .get("content")
        .and_then(Json::list)
        .unwrap()
        .first()
        .and_then(|one| one.get("text"))
        .and_then(Json::text)
        .unwrap();
    eprintln!("{said}");
    assert!(said.contains("outputs"), "{said}");
    assert!(said.contains("gates"), "{said}");
    assert!(
        said.split_whitespace().count() > 4,
        "a caller cannot act on bare numbers: {said}"
    );
}

#[test]
fn a_ladder_that_solves_nothing_still_hands_back_what_it_reached() {
    let Some(host) = device("a_ladder_that_solves_nothing_still_hands_back_what_it_reached") else {
        return;
    };
    let arguments = format!(
        "{{\"inputs\":4,\"outputs\":3,\"gates\":3,\"table\":[{}],\"crowd\":64,\"rounds\":1}}",
        adder()
    );
    let held = structured(&call(Some(&host), "grit_least", &arguments));
    eprintln!("{}", written(&held));
    assert_eq!(held.get("smallest_that_solved"), Some(&Json::Null));
    assert_eq!(held.get("circuit"), Some(&Json::Null));
    let best = held.get("best_attempt").unwrap();
    assert_eq!(
        best.get("gates").and_then(Json::list).unwrap().len(),
        3,
        "the best it reached is still a circuit: {}",
        written(best)
    );
    assert!(best.counted("marks").unwrap() < best.counted("of").unwrap());
    assert_eq!(
        best.counted("crowd"),
        Some(65536),
        "a rung that ends the ladder is pressed to the ceiling before it gives up, whatever crowd the caller asked for: {}",
        written(best)
    );
    assert_eq!(best.counted("rounds"), Some(1), "{}", written(best));
    assert!(held.get("nothing_solved").and_then(Json::text).is_some());
}

#[test]
fn every_candidate_starts_from_its_own_odd_seed() {
    let seeds = seeded(64);
    assert_eq!(seeds.len(), 64);
    assert!(
        seeds.iter().all(|one| one & 1 == 1),
        "an even seed leaves the xorshift stuck at zero"
    );
    let apart: std::collections::BTreeSet<u32> = seeds.iter().copied().collect();
    assert_eq!(
        apart.len(),
        64,
        "two candidates on one seed search the same ground"
    );
}

#[test]
fn a_search_that_has_not_solved_raises_its_own_crowd_before_it_answers() {
    let Some(host) = device("a_search_that_has_not_solved_raises_its_own_crowd_before_it_answers")
    else {
        return;
    };
    let arguments = format!(
        "{{\"inputs\":4,\"outputs\":3,\"gates\":3,\"table\":[{}],\"crowd\":64,\"rounds\":1}}",
        adder()
    );
    let held = structured(&call(Some(&host), "grit_synthesise", &arguments));
    eprintln!("asked for 64, spent {:?}", held.counted("crowd"));
    assert_eq!(held.get("solved"), Some(&Json::Bool(false)));
    assert_eq!(
        held.counted("crowd"),
        Some(65536),
        "a caller who picks a small crowd must not be told a budget is impossible: {}",
        written(&held)
    );
}

#[test]
fn a_search_that_solves_stops_at_the_crowd_it_was_given() {
    let Some(host) = device("a_search_that_solves_stops_at_the_crowd_it_was_given") else {
        return;
    };
    let table: Vec<String> = (0..16_u32)
        .map(|row| (row.count_ones() % 2).to_string())
        .collect();
    let arguments = format!(
        "{{\"inputs\":4,\"outputs\":1,\"gates\":3,\"table\":[{}],\"crowd\":4096,\"rounds\":2000}}",
        table.join(",")
    );
    let held = structured(&call(Some(&host), "grit_synthesise", &arguments));
    eprintln!("{}", written(&held));
    assert_eq!(
        held.get("solved"),
        Some(&Json::Bool(true)),
        "exclusive or over four inputs is three gates: {}",
        written(&held)
    );
    assert_eq!(
        held.counted("crowd"),
        Some(4096),
        "nothing to raise when it solved: {}",
        written(&held)
    );
}

#[test]
fn a_crowd_too_small_for_the_optimum_no_longer_hides_it() {
    let Some(host) = device("a_crowd_too_small_for_the_optimum_no_longer_hides_it") else {
        return;
    };
    let table: Vec<String> = (0..16_u32)
        .map(|row| ((row & 3) * ((row >> 2) & 3)).to_string())
        .collect();
    let arguments = format!(
        "{{\"inputs\":4,\"outputs\":4,\"gates\":7,\"table\":[{}],\"crowd\":500,\"rounds\":800}}",
        table.join(",")
    );
    let held = structured(&call(Some(&host), "grit_synthesise", &arguments));
    eprintln!(
        "two by two at seven gates: solved {:?}, crowd {:?}, rounds {:?}",
        held.get("solved"),
        held.counted("crowd"),
        held.counted("rounds")
    );
    assert_eq!(
        held.get("solved"),
        Some(&Json::Bool(true)),
        "seven gates is the optimum for a two by two multiplier and a small crowd used to hide it: {}",
        written(&held)
    );
    assert!(
        held.counted("crowd").unwrap() > 500,
        "it got there by raising the crowd it was given"
    );
    assert_eq!(
        held.get("wrong_rows").and_then(Json::list).unwrap().len(),
        0
    );
}

#[test]
fn the_budget_that_fooled_a_reader_is_reached_by_raising_the_crowd() {
    let Some(host) = device("the_budget_that_fooled_a_reader_is_reached_by_raising_the_crowd")
    else {
        return;
    };
    let table: Vec<String> = (0..32_u32)
        .map(|row| u32::from(row.count_ones() >= 3).to_string())
        .collect();
    let arguments = format!(
        "{{\"inputs\":5,\"outputs\":1,\"gates\":12,\"table\":[{}],\"crowd\":8000,\"rounds\":4000}}",
        table.join(",")
    );
    let held = structured(&call(Some(&host), "grit_synthesise", &arguments));
    eprintln!(
        "majority of five at twelve gates: solved {:?}, crowd {:?}, rounds {:?}",
        held.get("solved"),
        held.counted("crowd"),
        held.counted("rounds")
    );
    assert_eq!(
        held.get("solved"),
        Some(&Json::Bool(true)),
        "twelve gates holds a majority of five, and a crowd of 8000 over 4000 rounds says it does not: {}",
        written(&held)
    );
    assert!(held.counted("crowd").unwrap() > 8000);
    assert_eq!(
        held.get("wrong_rows").and_then(Json::list).unwrap().len(),
        0
    );
}

#[test]
fn a_marked_machine_says_how_much_evidence_it_was_marked_on() {
    let counter = "[{\"kind\":8,\"left\":0,\"right\":2},{\"kind\":6,\"left\":0,\"right\":2},{\"kind\":6,\"left\":1,\"right\":3},{\"kind\":8,\"left\":0,\"right\":0},{\"kind\":8,\"left\":1,\"right\":1}]";
    let arguments = format!(
        "{{\"state\":2,\"inputs\":1,\"outputs\":2,\"gates\":5,\"traces\":[{}],\"chip\":{counter}}}",
        counting(16)
    );
    let held = structured(&call(None, "grit_machine", &arguments));
    assert_eq!(held.counted("traces"), Some(4));
    assert_eq!(
        held.counted("steps"),
        Some(16),
        "runs and their length are the two numbers that decide whether solved means anything: {}",
        written(&held)
    );
    let said = held.get("solved_means").and_then(Json::text).unwrap();
    assert!(said.contains("never seen"), "{said}");
    assert!(said.contains("chip"), "{said}");
}

#[test]
fn the_machine_tool_says_what_solved_does_not_cover() {
    let held = tool("grit_machine");
    let said = held.get("description").and_then(Json::text).unwrap();
    assert!(
        said.contains("never seen"),
        "a caller reading only the schema must learn that solved is scoped to the runs given: {said}"
    );
    let steps = held
        .get("inputSchema")
        .and_then(|one| one.get("properties"))
        .and_then(|one| one.get("steps"))
        .and_then(|one| one.get("description"))
        .and_then(Json::text)
        .unwrap();
    assert!(steps.contains("longer runs"), "{steps}");
}

#[test]
fn the_ladder_steps_over_a_budget_that_fails_to_reach_one_below_it_that_does_not() {
    let Some(host) =
        device("the_ladder_steps_over_a_budget_that_fails_to_reach_one_below_it_that_does_not")
    else {
        return;
    };
    let table: Vec<String> = (0..32_u32)
        .map(|row| row.count_ones().to_string())
        .collect();
    let arguments = format!(
        "{{\"inputs\":5,\"outputs\":3,\"gates\":14,\"table\":[{}],\"crowd\":65536,\"rounds\":4000}}",
        table.join(",")
    );
    let held = structured(&call(Some(&host), "grit_least", &arguments));
    for rung in held.get("ladder").and_then(Json::list).unwrap() {
        eprintln!(
            "{:>3} gates: solved {:?}, {:?} of {:?}",
            rung.counted("gates").unwrap(),
            matches!(rung.get("solved"), Some(Json::Bool(true))),
            rung.counted("marks").unwrap(),
            rung.counted("of").unwrap()
        );
    }
    assert_eq!(
        held.counted("smallest_that_solved"),
        Some(11),
        "popcount of five is not found at twelve gates and is found at eleven, so a ladder that stops at the first failure answers thirteen: {}",
        written(&held)
    );
    let twelve = held
        .get("ladder")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .find(|rung| rung.counted("gates") == Some(12))
        .unwrap();
    assert_eq!(
        twelve.get("solved"),
        Some(&Json::Bool(false)),
        "the hole is on the ladder for the caller to see"
    );
}

fn sloped(host: Option<&Host>, arguments: &str) -> Json {
    structured(&call(host, "grit_slope", arguments))
}

#[test]
fn the_slope_names_the_cells_the_weight_reached_and_counts_the_ones_it_does_not_name() {
    let Some(host) = device("slope") else {
        return;
    };
    let answer = sloped(
        Some(&host),
        "{\"wide\":32,\"deep\":32,\"kind\":\"living\",\"depth\":8,\"fill\":1000,\"cells\":[{\"at\":528,\"is\":60000}],\"most\":4}",
    );
    let named = answer.get("slope").and_then(Json::list).unwrap().len();
    let heard = answer.counted("heard").unwrap();
    assert_eq!(named, 4, "most is a cap on what is named");
    assert!(
        heard > named as u32,
        "heard counts every cell the weight reached, not only the four it named"
    );
    assert_eq!(answer.counted("at").unwrap(), 528);
    assert_eq!(
        answer.get("kind").and_then(Json::text).unwrap(),
        "living",
        "the rule it walked comes back with the answer"
    );
    assert_eq!(answer.get("checked_on_the_host"), Some(&Json::Bool(true)));
}

#[test]
fn the_weight_reaches_exactly_one_cell_a_frame() {
    let Some(host) = device("slope reach") else {
        return;
    };
    // A 128 wide grid, because reach cannot exceed half the grid and a 32 wide
    // one is already saturated after two frames.
    let asked = |depth: u32| {
        sloped(
            Some(&host),
            &format!(
                "{{\"wide\":128,\"deep\":128,\"depth\":{depth},\"fill\":1000,\"cells\":[{{\"at\":8256,\"is\":60000}}]}}"
            ),
        )
        .counted("reach")
        .unwrap()
    };
    // The walk back has a light cone: a cell's future is made of what is within
    // one cell a frame of it, and no more. That is a property of the rule rather
    // than of this tool, which makes it worth pinning - it would catch the grid
    // and the slope being answered as each other, since the grid is non-zero
    // everywhere and would read as a cone that swallowed the board in one frame.
    for depth in [1_u32, 2, 4, 12] {
        assert_eq!(
            asked(depth),
            depth,
            "the weight reaches exactly one cell a frame, so depth {depth} reaches {depth}"
        );
    }
}

#[test]
fn walking_forward_only_answers_without_a_slope() {
    let Some(host) = device("slope forward") else {
        return;
    };
    let answer = sloped(
        Some(&host),
        "{\"wide\":32,\"deep\":32,\"depth\":4,\"fill\":1000,\"carry\":false}",
    );
    assert_eq!(answer.get("slope"), None, "nothing was carried back");
    assert_eq!(answer.get("told"), None);
    assert!(answer.get("ended").is_some(), "the walk still happened");
}

#[test]
fn the_two_gradients_come_back_as_a_fraction_that_is_never_rounded() {
    let Some(host) = device("slope told") else {
        return;
    };
    let answer = sloped(
        Some(&host),
        "{\"wide\":32,\"deep\":32,\"depth\":8,\"fill\":1000,\"cells\":[{\"at\":528,\"is\":60000}]}",
    );
    let told = answer.get("told").unwrap();
    let over = told.get("over").and_then(Json::int).unwrap();
    assert!(over > 0, "a fraction needs a divisor");
    assert!(
        told.get("turning").and_then(Json::int).is_some(),
        "turning is an integer, so the caller does the division and nothing is lost here"
    );
}

#[test]
fn a_kind_that_is_not_in_the_kit_is_named_rather_than_guessed_at() {
    let answer = call(None, "grit_slope", "{\"kind\":\"treacle\"}");
    let said = written(&answer);
    assert!(
        said.contains("treacle") || said.contains("NoComputeDevice"),
        "either it names the kind it does not know, or it says it has no device: {said}"
    );
}

#[test]
fn a_cell_outside_the_grid_is_refused_rather_than_wrapped() {
    let Some(host) = device("slope outside") else {
        return;
    };
    let answer = call(
        Some(&host),
        "grit_slope",
        "{\"wide\":32,\"deep\":32,\"cells\":[{\"at\":99999,\"is\":1}]}",
    );
    let said = written(&answer);
    assert!(
        said.contains("outside"),
        "setting a cell off the grid has to be said, not folded back in: {said}"
    );
}

#[test]
fn settling_walks_back_to_the_parameters_the_target_was_made_with() {
    let Some(host) = device("settle round trip") else {
        return;
    };
    // Make a grid with the rule itself at a turning of eight, then hand that
    // grid back as the thing to land on and start somewhere else entirely.
    // Nothing here tells it eight; the only thing it is given is the grid.
    let start = "[{\"at\":528,\"is\":60000},{\"at\":529,\"is\":50000},{\"at\":560,\"is\":40000}]";
    let made = sloped(
        Some(&host),
        &format!(
            "{{\"wide\":32,\"deep\":32,\"depth\":6,\"turning\":8,\"fill\":0,\"carry\":false,\"most\":1024,\"cells\":{start}}}"
        ),
    );
    let want = written(made.get("ended").unwrap());
    let answer = structured(&call(
        Some(&host),
        "grit_settle",
        &format!(
            "{{\"wide\":32,\"deep\":32,\"depth\":6,\"turning\":60,\"rounds\":40,\"step\":16,\"fill\":0,\"cells\":{start},\"want\":{want}}}"
        ),
    ));
    assert_eq!(
        answer.get("apart").and_then(Json::int),
        Some(0),
        "the grid was made by this rule, so some parameters land on it exactly"
    );
    assert_eq!(
        answer.counted("turning"),
        Some(8),
        "and they are the ones it was made with"
    );
}

#[test]
fn the_distance_is_believed_over_the_slope_when_they_disagree() {
    let Some(host) = device("settle turns around") else {
        return;
    };
    // Between forty and fifty two the walk back points uphill while the
    // distance, which is counted exactly, falls the other way. Starting inside
    // that stretch is the case a tool that only followed the slope would lose.
    let start = "[{\"at\":528,\"is\":60000},{\"at\":529,\"is\":50000},{\"at\":560,\"is\":40000}]";
    let made = sloped(
        Some(&host),
        &format!(
            "{{\"wide\":32,\"deep\":32,\"depth\":6,\"turning\":8,\"fill\":0,\"carry\":false,\"most\":1024,\"cells\":{start}}}"
        ),
    );
    let want = written(made.get("ended").unwrap());
    let answer = structured(&call(
        Some(&host),
        "grit_settle",
        &format!(
            "{{\"wide\":32,\"deep\":32,\"depth\":6,\"turning\":44,\"rounds\":40,\"step\":16,\"fill\":0,\"cells\":{start},\"want\":{want}}}"
        ),
    ));
    let apart = answer.get("apart").and_then(Json::int).unwrap();
    let started = answer.get("started_apart").and_then(Json::int).unwrap();
    assert!(
        apart < started,
        "it began where the slope lies and still got nearer: {started} then {apart}"
    );
}

#[test]
fn a_target_it_is_already_standing_on_is_left_alone() {
    let Some(host) = device("settle already there") else {
        return;
    };
    let start = "[{\"at\":528,\"is\":60000}]";
    let made = sloped(
        Some(&host),
        &format!(
            "{{\"wide\":32,\"deep\":32,\"depth\":4,\"turning\":12,\"fill\":0,\"carry\":false,\"most\":1024,\"cells\":{start}}}"
        ),
    );
    let want = written(made.get("ended").unwrap());
    let answer = structured(&call(
        Some(&host),
        "grit_settle",
        &format!(
            "{{\"wide\":32,\"deep\":32,\"depth\":4,\"turning\":12,\"rounds\":6,\"step\":4,\"fill\":0,\"cells\":{start},\"want\":{want}}}"
        ),
    ));
    assert_eq!(answer.get("apart").and_then(Json::int), Some(0));
    assert_eq!(
        answer.counted("turning"),
        Some(12),
        "it started on the answer and had no reason to leave"
    );
}

#[test]
fn a_parameter_with_no_purchase_is_counted_rather_than_mistaken_for_an_answer() {
    let Some(host) = device("settle unmoved") else {
        return;
    };
    // At small cell values the distance does not move at all across a wide
    // stretch of turning: the parameter has no grip there. That reads exactly
    // like having arrived, so it is counted and said rather than left to look
    // like success. Measured: forty unit steps of turning, every one flat.
    let start = "[{\"at\":528,\"is\":600},{\"at\":529,\"is\":500}]";
    let answer = structured(&call(
        Some(&host),
        "grit_settle",
        &format!(
            "{{\"wide\":32,\"deep\":32,\"depth\":6,\"turning\":20,\"rounds\":8,\"step\":4,\"fill\":0,\"cells\":{start},\"want\":[{{\"at\":528,\"is\":1}}]}}"
        ),
    ));
    assert!(
        answer.counted("unmoved").unwrap_or(0) > 0,
        "moving the parameters changed nothing, and the answer has to say so"
    );
}

fn poured(host: Option<&Host>, arguments: &str) -> Json {
    structured(&call(host, "grit_pour", arguments))
}

#[test]
fn nothing_is_made_or_lost_while_it_pours() {
    let Some(host) = device("pour tally") else {
        return;
    };
    let answer = poured(
        Some(&host),
        "{\"wide\":64,\"deep\":64,\"frames\":120,\"brush\":{\"at\":544,\"is\":\"sand\",\"span\":25,\"frames\":80}}",
    );
    let tally = answer.get("tally").unwrap();
    let held: i64 = ["air", "water", "sand", "wall"]
        .iter()
        .filter_map(|one| tally.get(one).and_then(Json::int))
        .sum();
    let cells = answer
        .get("grid")
        .and_then(|one| one.counted("cells"))
        .unwrap();
    assert_eq!(
        held,
        i64::from(cells),
        "every cell holds exactly one of the four things, before and after"
    );
    assert!(
        tally.get("sand").and_then(Json::int).unwrap_or(0) > 0,
        "the brush poured, so there is sand to find"
    );
}

#[test]
fn the_bill_follows_what_is_moving_rather_than_how_big_the_grid_is() {
    let Some(host) = device("pour listed") else {
        return;
    };
    // The engine's claim, in the one number that can show it: the work list
    // names the quads worth stepping, and a room that is mostly still air is
    // mostly not stepped at all.
    let answer = poured(
        Some(&host),
        "{\"wide\":128,\"deep\":128,\"frames\":200,\"brush\":{\"at\":2112,\"is\":\"sand\",\"span\":25,\"frames\":120}}",
    );
    let listed = answer.get("listed").and_then(Json::int).unwrap();
    let quads = i64::from(answer.counted("of_quads").unwrap());
    assert!(listed > 0, "something was stepped");
    assert!(
        listed < quads,
        "a grid that is mostly still costs less than all of it: {listed} of {quads}"
    );
}

#[test]
fn sand_ends_up_lower_than_it_started() {
    let Some(host) = device("pour falls") else {
        return;
    };
    // Placed high, with a floor under it and no brush pouring, it has one way
    // to go. The rows say where it went.
    let answer = poured(
        Some(&host),
        "{\"wide\":64,\"deep\":64,\"frames\":200,\"cells\":[{\"at\":160,\"is\":\"sand\"},{\"at\":161,\"is\":\"sand\"},{\"at\":162,\"is\":\"sand\"}]}",
    );
    let rows = answer.get("rows").and_then(Json::list).unwrap();
    let lowest = rows
        .iter()
        .enumerate()
        .filter(|(_, one)| one.int().unwrap_or(0) > 0)
        .map(|(row, _)| row)
        .next_back()
        .unwrap_or(0);
    assert!(
        lowest > 2,
        "it started in the third row and had two hundred frames to fall: it ended in row {lowest}"
    );
}

#[test]
fn a_thing_that_is_not_in_the_room_is_named_rather_than_poured() {
    let Some(host) = device("pour stuff") else {
        return;
    };
    let answer = call(
        Some(&host),
        "grit_pour",
        "{\"wide\":64,\"deep\":64,\"frames\":2,\"cells\":[{\"at\":160,\"is\":\"treacle\"}]}",
    );
    let said = written(&answer);
    assert!(
        said.contains("treacle"),
        "it has four things in it and treacle is not one: {said}"
    );
}
