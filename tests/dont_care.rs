#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;
mod replay;

use common::device;
use replay::{ALU, adder, call, hand_chip, joined, majority};

use grit::Host;
use grit::credit::{self, Board, Complements, Marker, Twin, applied, paired, singles};
use grit::netlist::{Chip, Exam, Gate, Netlist, NotAnExam, drawn};
use grit::serve::{CROWD, Effort, Escape, handle, sought};
use grit::wire::{Json, read, written};

const GOLDEN: &str = include_str!("golden/dont_care_replay.txt");

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 16) as u32
    }

    fn below(&mut self, most: u32) -> u32 {
        self.next() % most.max(1)
    }
}

fn golden() -> Vec<String> {
    GOLDEN
        .replace("\r\n", "\n")
        .lines()
        .map(str::to_owned)
        .collect()
}

fn rows_json(rows: &[Option<u32>]) -> String {
    let all: Vec<String> = rows
        .iter()
        .map(|one| one.map_or_else(|| "null".to_owned(), |row| row.to_string()))
        .collect();
    all.join(",")
}

fn masked(table: &[u32], rng: &mut Rng, keep: u32) -> Vec<Option<u32>> {
    let mut rows: Vec<Option<u32>> = table
        .iter()
        .map(|row| (rng.below(100) < keep).then_some(*row))
        .collect();
    if rows.iter().all(Option::is_none) {
        let at = rng.below(rows.len() as u32) as usize;
        rows[at] = Some(table[at]);
    }
    if rows.iter().all(Option::is_some) {
        let at = rng.below(rows.len() as u32) as usize;
        rows[at] = None;
    }
    rows
}

fn cared_of(rows: &[Option<u32>]) -> u32 {
    rows.iter().filter(|one| one.is_some()).count() as u32
}

fn packed_of(answer: &Json) -> Vec<u32> {
    answer
        .get("gates")
        .unwrap()
        .list()
        .unwrap()
        .iter()
        .map(|one| {
            Gate {
                kind: one.counted("kind").unwrap(),
                left: one.counted("left").unwrap(),
                right: one.counted("right").unwrap(),
            }
            .pack()
        })
        .collect()
}

fn evaluated(shape: Netlist, circuit: &[u32]) -> Vec<u32> {
    let named = (shape.gates - shape.outputs..shape.gates).collect();
    Chip::of(shape, circuit.to_vec(), named).unwrap().table()
}

fn bits_on_cared(shape: Netlist, got: &[u32], rows: &[Option<u32>]) -> u32 {
    let mask = (1_u32 << shape.outputs) - 1;
    rows.iter()
        .zip(got)
        .filter_map(|(want, got)| want.map(|want| (!(want ^ got) & mask).count_ones()))
        .sum()
}

fn chip_json(circuit: &[u32]) -> String {
    let gates: Vec<String> = circuit
        .iter()
        .map(|one| {
            let gate = Gate::unpack(*one);
            format!(
                "{{\"kind\":{},\"left\":{},\"right\":{}}}",
                gate.kind, gate.left, gate.right
            )
        })
        .collect();
    format!("[{}]", gates.join(","))
}

fn refused(name: &str, arguments: &str) -> Json {
    let asked = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{{\"name\":\"{name}\",\"arguments\":{arguments}}}}}"
    );
    let answer = handle(None, &read(&asked).unwrap()).unwrap();
    let failed = answer.get("error").is_some()
        || answer
            .get("result")
            .and_then(|one| one.get("isError"))
            .is_some_and(|one| *one == Json::Bool(true));
    assert!(failed, "{}", written(&answer));
    answer
}

fn listed_tool(name: &str) -> Json {
    let asked = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}";
    let answer = handle(None, &read(asked).unwrap()).unwrap();
    answer
        .get("result")
        .and_then(|one| one.get("tools"))
        .and_then(Json::list)
        .unwrap()
        .iter()
        .find(|one| one.get("name").and_then(Json::text) == Some(name))
        .cloned()
        .unwrap()
}

#[test]
fn no_null_tables_replay_the_base_answers_of_every_search_tool_over_twenty_seeds() {
    let Some(host) = device("no_null_tables_replay_the_base_answers_of_every_search_tool") else {
        return;
    };
    let now = replay::replay(&host);
    let was = golden();
    let lines: Vec<&str> = now.lines().collect();
    assert_eq!(lines.len(), was.len());
    for (got, want) in lines.iter().zip(&was) {
        assert_eq!(*got, want.as_str());
    }
}

#[test]
fn no_null_check_and_credit_answers_replay_the_base_bit_for_bit() {
    let now = replay::cpu_replay();
    let was = golden();
    let tail = &was[was.len() - now.len()..];
    assert_eq!(now.len(), 6);
    for (got, want) in now.iter().zip(tail) {
        assert_eq!(got, want);
    }
}

#[test]
fn a_partial_table_is_solved_on_its_cared_rows_and_the_circuit_matches_each_on_the_cpu() {
    let Some(host) = device("a_partial_table_is_solved_on_its_cared_rows") else {
        return;
    };
    let mut rng = Rng(0x00d1_0002);
    let shape = Netlist::over(4, 3, 9, 16).unwrap();
    let mut checked = 0;
    for seed in 1..=12_u32 {
        let keep = [20, 35, 50, 65, 80, 95][seed as usize % 6];
        let rows = masked(&adder(), &mut rng, keep);
        let cared = cared_of(&rows);
        let answer = call(
            Some(&host),
            "grit_synthesise",
            &format!(
                "{{\"inputs\":4,\"outputs\":3,\"gates\":9,\"table\":[{}],\"crowd\":256,\"rounds\":300,\"seed\":{seed}}}",
                rows_json(&rows)
            ),
        );
        assert_eq!(
            answer.get("solved"),
            Some(&Json::Bool(true)),
            "{}",
            written(&answer)
        );
        assert_eq!(answer.counted("cared"), Some(cared));
        assert_eq!(answer.counted("of"), Some(cared * 3));
        assert_eq!(answer.counted("marks"), Some(cared * 3));
        assert_eq!(answer.get("wrong_rows"), Some(&Json::List(Vec::new())));
        let got = evaluated(shape, &packed_of(&answer));
        for (at, row) in rows.iter().enumerate() {
            if let Some(want) = row {
                assert_eq!(got[at], *want, "seed {seed}, cared row {at}");
                checked += 1;
            }
        }
    }
    eprintln!("{checked} cared rows evaluated on the cpu");
}

#[test]
fn one_output_partial_tables_report_cared_rows_as_marks_and_of_alike() {
    let Some(host) = device("one_output_partial_tables_report_cared_rows") else {
        return;
    };
    let mut rng = Rng(0x00d1_0003);
    for seed in 1..=10_u32 {
        let rows = masked(&majority(), &mut rng, 60);
        let cared = cared_of(&rows);
        let answer = call(
            Some(&host),
            "grit_synthesise",
            &format!(
                "{{\"inputs\":3,\"outputs\":1,\"gates\":4,\"table\":[{}],\"crowd\":128,\"rounds\":200,\"seed\":{seed}}}",
                rows_json(&rows)
            ),
        );
        assert_eq!(answer.counted("marks"), Some(cared), "{}", written(&answer));
        assert_eq!(answer.counted("of"), Some(cared));
        assert_eq!(answer.counted("cared"), Some(cared));
    }
}

#[test]
fn a_full_table_answers_without_a_cared_field_and_a_partial_one_with_it() {
    let held = call(
        None,
        "grit_check",
        &format!(
            "{{\"inputs\":7,\"outputs\":4,\"gates\":22,\"table\":[{}],\"chip\":{}}}",
            joined(&ALU),
            hand_chip()
        ),
    );
    assert!(held.get("cared").is_none(), "{}", written(&held));
    let some = format!(
        "{{\"inputs\":3,\"outputs\":1,\"gates\":2,\"table\":[{}],\"chip\":[{{\"kind\":8,\"left\":0,\"right\":1}},{{\"kind\":6,\"left\":3,\"right\":2}}]}}",
        rows_json(&[Some(0), None, Some(1), None, Some(0), None, Some(1), None])
    );
    let held = call(None, "grit_check", &some);
    assert_eq!(held.counted("cared"), Some(4), "{}", written(&held));
}

fn mutant(shape: Netlist, base: &[u32], rng: &mut Rng) -> Vec<u32> {
    loop {
        let at = rng.below(shape.gates);
        let mut held = base.to_vec();
        held[at as usize] = drawn(&shape, at, rng.next()).pack();
        if held != base {
            return held;
        }
    }
}

fn hand() -> (Netlist, Vec<u32>) {
    let shape = Netlist::over(7, 4, 22, 16).unwrap();
    let circuit = replay::HAND
        .iter()
        .map(|[kind, left, right]| {
            Gate {
                kind: *kind,
                left: *left,
                right: *right,
            }
            .pack()
        })
        .collect();
    (shape, circuit)
}

#[test]
fn a_circuit_that_differs_only_on_dont_care_rows_scores_full_and_one_that_misses_a_cared_row_does_not()
 {
    let (shape, base) = hand();
    let table = evaluated(shape, &base);
    assert_eq!(table, ALU.to_vec());
    let mut rng = Rng(0x00d1_0004);
    let mut full = 0;
    let mut short = 0;
    for _ in 0..40 {
        let other = mutant(shape, &base, &mut rng);
        let theirs = evaluated(shape, &other);
        let differing: Vec<usize> = (0..table.len())
            .filter(|at| table[*at] != theirs[*at])
            .collect();
        if differing.is_empty() {
            continue;
        }
        let nulled: Vec<Option<u32>> = table
            .iter()
            .enumerate()
            .map(|(at, row)| (!differing.contains(&at)).then_some(*row))
            .collect();
        if nulled.iter().all(Option::is_none) {
            continue;
        }
        let asked = |rows: &[Option<u32>], circuit: &[u32]| {
            call(
                None,
                "grit_check",
                &format!(
                    "{{\"inputs\":7,\"outputs\":4,\"gates\":22,\"table\":[{}],\"chip\":{}}}",
                    rows_json(rows),
                    chip_json(circuit)
                ),
            )
        };
        let held = asked(&nulled, &other);
        assert_eq!(
            held.get("solved"),
            Some(&Json::Bool(true)),
            "{}",
            written(&held)
        );
        assert_eq!(held.counted("marks"), held.counted("of"));
        assert_eq!(held.get("wrong_rows"), Some(&Json::List(Vec::new())));
        full += 1;
        let keep = differing[rng.below(differing.len() as u32) as usize];
        let mut cared = nulled.clone();
        cared[keep] = Some(table[keep]);
        let held = asked(&cared, &other);
        assert_eq!(
            held.get("solved"),
            Some(&Json::Bool(false)),
            "{}",
            written(&held)
        );
        let marks = held.counted("marks").unwrap();
        let of = held.counted("of").unwrap();
        assert!(marks < of, "{}", written(&held));
        assert_eq!(
            held.get("wrong_rows"),
            Some(&Json::List(vec![Json::Int(keep as i64)]))
        );
        let kept = (!(table[keep] ^ theirs[keep]) & 15).count_ones();
        assert_eq!(marks, of - 4 + kept);
        short += 1;
    }
    assert!(full >= 20 && short >= 20, "{full} {short}");
}

#[test]
fn the_wrong_rows_of_a_partial_table_name_cared_rows_only() {
    let (shape, base) = hand();
    let mut rng = Rng(0x00d1_0005);
    let other = mutant(shape, &base, &mut rng);
    let rows = masked(&ALU, &mut rng, 50);
    let held = call(
        None,
        "grit_check",
        &format!(
            "{{\"inputs\":7,\"outputs\":4,\"gates\":22,\"table\":[{}],\"chip\":{}}}",
            rows_json(&rows),
            chip_json(&other)
        ),
    );
    let theirs = evaluated(shape, &other);
    let expected: Vec<Json> = (0..ALU.len())
        .filter(|at| rows[*at].is_some() && theirs[*at] != ALU[*at])
        .map(|at| Json::Int(at as i64))
        .collect();
    assert_eq!(held.get("wrong_rows"), Some(&Json::List(expected)));
    assert_eq!(
        held.counted("marks"),
        Some(bits_on_cared(shape, &theirs, &rows))
    );
    assert_eq!(held.counted("of"), Some(cared_of(&rows) * 4));
}

fn random_partial(rng: &mut Rng) -> (Netlist, Vec<Option<u32>>) {
    let inputs = 2 + rng.below(3);
    let outputs = 1 + rng.below(2);
    let gates = outputs + 1 + rng.below(4);
    let shape = Netlist::over(inputs, outputs, gates, 16).unwrap();
    let table: Vec<u32> = (0..shape.words())
        .map(|_| rng.next() & ((1 << outputs) - 1))
        .collect();
    let keep = 25 + rng.below(60);
    (shape, masked(&table, rng, keep))
}

fn random_chip(shape: Netlist, rng: &mut Rng) -> Vec<u32> {
    (0..shape.gates)
        .map(|at| drawn(&shape, at, rng.next()).pack())
        .collect()
}

fn marks_of(shape: Netlist, circuit: &[u32], rows: &[Option<u32>]) -> i64 {
    i64::from(bits_on_cared(shape, &evaluated(shape, circuit), rows))
}

struct Brute {
    improving_single: bool,
    best_pair: i64,
}

fn brute(shape: Netlist, circuit: &[u32], rows: &[Option<u32>]) -> Brute {
    let base = marks_of(shape, circuit, rows);
    let mut improving_single = false;
    let mut best_pair = i64::MIN;
    for first in singles(shape, circuit) {
        let mid = applied(circuit, first);
        improving_single |= marks_of(shape, &mid, rows) > base;
        for second in singles(shape, &mid) {
            let end = applied(&mid, second);
            best_pair = best_pair.max(marks_of(shape, &end, rows) - base);
        }
    }
    Brute {
        improving_single,
        best_pair,
    }
}

fn partial_exam(shape: Netlist, rows: &[Option<u32>]) -> Exam {
    Exam::partial(shape, rows.to_vec()).unwrap()
}

#[test]
fn credit_on_a_partial_table_agrees_with_a_brute_force_over_the_cared_rows() {
    let mut rng = Rng(0x00d1_0006);
    let mut singles_seen = 0;
    let mut certified_one = 0;
    let mut certified_two = 0;
    for _ in 0..60 {
        let (shape, rows) = random_partial(&mut rng);
        let circuit = random_chip(shape, &mut rng);
        let exam = partial_exam(shape, &rows);
        let board = Board::of(&exam);
        let tangent = board.tangent(&circuit);
        let base = marks_of(shape, &circuit, &rows);
        assert_eq!(i64::from(tangent.marks), base);
        assert_eq!(board.cared(), cared_of(&rows));
        for change in singles(shape, &circuit) {
            let real = marks_of(shape, &applied(&circuit, change), &rows) - base;
            assert_eq!(board.credit(&tangent, &circuit, change), real);
            singles_seen += 1;
        }
        let truth = brute(shape, &circuit, &rows);
        let answer = call(
            None,
            "grit_credit",
            &format!(
                "{{\"inputs\":{},\"outputs\":{},\"gates\":{},\"table\":[{}],\"chip\":{},\"depth\":2}}",
                shape.inputs,
                shape.outputs,
                shape.gates,
                rows_json(&rows),
                chip_json(&circuit)
            ),
        );
        assert_eq!(answer.counted("cared"), Some(cared_of(&rows)));
        assert_eq!(answer.counted("of"), Some(cared_of(&rows) * shape.outputs));
        assert_eq!(i64::from(answer.counted("marks").unwrap()), base);
        let certified = answer.counted("certified").unwrap();
        if truth.improving_single {
            assert_eq!(certified, 0, "{}", written(&answer));
        } else if truth.best_pair > 0 {
            assert_eq!(certified, 1, "{}", written(&answer));
            certified_one += 1;
        } else {
            assert_eq!(certified, 2, "{}", written(&answer));
            certified_two += 1;
        }
        let pairs = paired(&board, &mut Twin, &circuit, credit::PAIRS, 1).unwrap();
        assert!(pairs.complete());
        assert_eq!(pairs.best, truth.best_pair);
    }
    eprintln!("{singles_seen} singles; certified 1: {certified_one}, certified 2: {certified_two}");
    assert!(singles_seen > 1000);
    assert!(certified_one + certified_two > 0);
}

#[test]
fn the_certificate_of_a_partial_table_says_cared_rows() {
    let rows = [Some(0), Some(0), None, Some(1)];
    let held = call(
        None,
        "grit_credit",
        &format!(
            "{{\"inputs\":2,\"outputs\":1,\"gates\":1,\"table\":[{}],\"chip\":[{{\"kind\":8,\"left\":0,\"right\":1}}]}}",
            rows_json(&rows)
        ),
    );
    let said = held.get("certified_means").and_then(Json::text).unwrap();
    assert!(said.contains("cared rows"), "{said}");
    assert_eq!(held.counted("of"), Some(3));
}

#[test]
fn the_device_marks_a_partial_table_exactly_as_the_host_twin_does() {
    let Some(host) = device("the_device_marks_a_partial_table_exactly_as_the_host_twin_does")
    else {
        return;
    };
    let mut rng = Rng(0x00d1_0007);
    let mut compared = 0;
    for _ in 0..40 {
        let (shape, rows) = random_partial(&mut rng);
        let exam = partial_exam(shape, &rows);
        let board = Board::of(&exam);
        let circuits: Vec<Vec<u32>> = (0..20).map(|_| random_chip(shape, &mut rng)).collect();
        let room = credit::room(shape, CROWD);
        let mut resident = credit::opened(&host, &board, room).unwrap();
        let mut marker = Marker::within(&mut resident, 1, 0, host.width(), room, shape);
        let device = marker.tangents(&board, &circuits).unwrap();
        let twin = Twin.tangents(&board, &circuits).unwrap();
        for ((circuit, got), want) in circuits.iter().zip(&device).zip(&twin) {
            assert_eq!(got, want);
            assert_eq!(
                i64::from(got.marks),
                marks_of(shape, circuit, &rows),
                "device score of {circuit:?} on {rows:?}"
            );
            compared += 1;
        }
    }
    eprintln!("{compared} circuits, device equals twin and equals the cared-row count");
}

#[test]
fn the_credit_escape_certifies_against_the_cared_rows_only() {
    let mut rng = Rng(0x00d1_0008);
    let mut lifted = 0;
    for _ in 0..40 {
        let (shape, rows) = random_partial(&mut rng);
        let circuit = random_chip(shape, &mut rng);
        let exam = partial_exam(shape, &rows);
        let board = Board::of(&exam);
        let out = credit::escape(&board, &mut Twin, &circuit, 1 << 24).unwrap();
        assert_eq!(
            i64::from(out.marks),
            marks_of(shape, &out.circuit, &rows),
            "the escape's own count must be the cared-row count"
        );
        assert!(out.marks >= board.marks(&circuit));
        if out.marks > board.marks(&circuit) {
            lifted += 1;
        }
        if out.marks < board.full() {
            let truth = brute(shape, &out.circuit, &rows);
            assert!(
                !truth.improving_single,
                "the escape stopped on an improving single"
            );
        }
    }
    assert!(lifted > 0);
}

#[test]
fn a_search_with_the_credit_escape_scores_a_partial_table_on_its_cared_rows() {
    let Some(host) = device("a_search_with_the_credit_escape_scores_a_partial_table") else {
        return;
    };
    let shape = Netlist::over(7, 4, 70, 16).unwrap();
    let mut rng = Rng(0x00d1_0009);
    let table: Vec<u32> = (0..shape.words()).map(|_| rng.next() & 15).collect();
    let mut ran = 0;
    let mut lifted = 0;
    for keep in [30, 60, 90] {
        let rows = masked(&table, &mut rng, keep);
        let exam = partial_exam(shape, &rows);
        let effort = Effort {
            seed: 1,
            stall: 2,
            escape: Escape::Credit,
            ..Effort::of(64, 1500)
        };
        let found = sought(&host, shape, &exam, effort, 0).unwrap();
        ran += found.escapes.ran;
        lifted += found.escapes.lifted;
        let packed: Vec<u32> = found.gates.iter().map(|one| one.pack()).collect();
        let marks = bits_on_cared(shape, &evaluated(shape, &packed), &rows);
        assert_eq!(found.marks, marks);
        assert_eq!(found.full, cared_of(&rows) * 4);
        assert_eq!(found.cared, Some(cared_of(&rows)));
        let board = Board::of(&exam);
        assert_eq!(board.marks(&packed), marks);
    }
    assert!(ran > 0, "the escape never ran, so it was not exercised");
    eprintln!("escape ran {ran} times, lifted {lifted}");
}

fn x0_rows() -> Vec<Option<u32>> {
    let mut rng = Rng(0x00d1_0a0a);
    let mut order: Vec<u32> = (0..512).collect();
    for at in (1..order.len()).rev() {
        let pick = rng.below(at as u32 + 1) as usize;
        order.swap(at, pick);
    }
    let mut rows = vec![None; 512];
    for word in order.iter().take(144) {
        let bit = |at: u32| (word >> at) & 1;
        let centre = bit(4);
        let apart: u32 = [1, 3, 5, 7].iter().map(|at| bit(*at) ^ centre).sum();
        rows[*word as usize] = Some(u32::from(apart >= 2));
    }
    rows
}

fn machine_gates(host: &Host, rows: &[Option<u32>], gates: u32, more: &str) -> Json {
    let traces: Vec<String> = rows
        .iter()
        .enumerate()
        .filter_map(|(at, one)| one.map(|row| format!("{{\"feed\":[{at}],\"want\":[{row}]}}")))
        .collect();
    call(
        Some(host),
        "grit_machine",
        &format!(
            "{{\"state\":0,\"inputs\":9,\"outputs\":1,\"gates\":{gates},\"traces\":[{}]{more}}}",
            traces.join(",")
        ),
    )
}

#[test]
fn least_on_the_x0_shape_finds_no_larger_a_circuit_than_machine_state_zero_with_the_same_work() {
    let Some(host) = device("least_on_the_x0_shape") else {
        return;
    };
    let rows = x0_rows();
    assert_eq!(cared_of(&rows), 144);
    let effort = "\"crowd\":1024,\"rounds\":400,\"seed\":5";
    let mut machine_smallest = None;
    for gates in (8..=16).rev() {
        let held = machine_gates(&host, &rows, gates, &format!(",{effort}"));
        if held.get("solved") == Some(&Json::Bool(true)) {
            machine_smallest = Some(gates);
        } else if machine_smallest.is_some() {
            break;
        }
    }
    let machine_smallest = machine_smallest.expect("machine state 0 solved at no budget");
    let held = call(
        Some(&host),
        "grit_least",
        &format!(
            "{{\"inputs\":9,\"outputs\":1,\"gates\":24,\"table\":[{}],{effort}}}",
            rows_json(&rows)
        ),
    );
    assert_eq!(held.counted("cared"), Some(144), "{}", written(&held));
    let smallest = held
        .counted("smallest_that_solved")
        .expect("least solved nothing");
    eprintln!("machine state 0 smallest {machine_smallest}, least smallest {smallest}");
    assert!(
        smallest <= machine_smallest,
        "{smallest} > {machine_smallest}"
    );
    let ladder = held.get("ladder").unwrap().list().unwrap();
    let rung = ladder
        .iter()
        .filter(|rung| rung.get("solved") == Some(&Json::Bool(true)))
        .min_by_key(|rung| rung.counted("gates").unwrap())
        .unwrap();
    assert_eq!(rung.counted("gates"), Some(smallest));
    assert_eq!(rung.counted("marks"), rung.counted("of"));
    assert_eq!(rung.counted("of"), Some(144));
    let circuit = held.get("circuit").unwrap();
    assert_eq!(circuit.counted("of"), Some(144));
    let shape = Netlist::over(9, 1, smallest, 16).unwrap();
    let got = evaluated(shape, &packed_of(circuit));
    assert_eq!(bits_on_cared(shape, &got, &rows), 144);
}

#[test]
fn every_rung_of_a_partial_ladder_scores_the_cared_rows() {
    let Some(host) = device("every_rung_of_a_partial_ladder_scores_the_cared_rows") else {
        return;
    };
    let rows = [
        Some(0),
        Some(0),
        Some(0),
        Some(1),
        None,
        Some(1),
        Some(1),
        None,
    ];
    let held = call(
        Some(&host),
        "grit_least",
        &format!(
            "{{\"inputs\":3,\"outputs\":1,\"gates\":6,\"table\":[{}],\"crowd\":128,\"rounds\":200,\"seed\":3}}",
            rows_json(&rows)
        ),
    );
    assert_eq!(held.counted("cared"), Some(6));
    for rung in held.get("ladder").unwrap().list().unwrap() {
        assert_eq!(rung.counted("of"), Some(6), "{}", written(&held));
    }
    assert!(held.counted("smallest_that_solved").is_some());
}

#[test]
fn a_table_with_no_cared_row_is_refused_by_every_tool_that_takes_a_table() {
    let none = "[null,null,null,null]";
    let chip = "[{\"kind\":8,\"left\":0,\"right\":1}]";
    for name in ["grit_synthesise", "grit_least", "grit_check", "grit_credit"] {
        let more = if name == "grit_check" || name == "grit_credit" {
            format!(",\"chip\":{chip}")
        } else {
            String::new()
        };
        let answer = refused(
            name,
            &format!("{{\"inputs\":2,\"outputs\":1,\"gates\":1,\"table\":{none}{more}}}"),
        );
        let said = written(&answer);
        assert!(said.contains("no row is cared about"), "{name}: {said}");
        assert!(said.contains("null"), "{name}: {said}");
    }
}

#[test]
fn a_table_of_the_wrong_length_is_refused_and_the_hint_names_null() {
    let answer = refused(
        "grit_check",
        "{\"inputs\":2,\"outputs\":1,\"gates\":1,\"table\":[0,1,null],\"chip\":[{\"kind\":8,\"left\":0,\"right\":1}]}",
    );
    let said = written(&answer);
    assert!(said.contains("exactly 4"), "{said}");
    assert!(said.contains("null"), "{said}");
}

#[test]
fn an_entry_that_is_neither_null_nor_a_whole_number_in_range_is_refused_never_read_as_zero() {
    let chip = "[{\"kind\":8,\"left\":0,\"right\":1}]";
    for entry in ["-1", "\"x\"", "true", "2", "[]", "{}"] {
        refused(
            "grit_check",
            &format!(
                "{{\"inputs\":2,\"outputs\":1,\"gates\":1,\"table\":[0,{entry},null,1],\"chip\":{chip}}}"
            ),
        );
    }
}

#[test]
fn the_machine_tool_does_not_take_null_in_its_traces() {
    for want in ["[0,null]", "[null,null]"] {
        refused(
            "grit_machine",
            &format!(
                "{{\"state\":0,\"inputs\":1,\"outputs\":1,\"gates\":2,\"traces\":[{{\"feed\":[0,1],\"want\":{want}}}]}}"
            ),
        );
    }
}

#[test]
fn the_table_schema_of_every_table_tool_says_null_means_dont_care() {
    for name in ["grit_synthesise", "grit_least", "grit_check", "grit_credit"] {
        let tool = listed_tool(name);
        let table = tool
            .get("inputSchema")
            .and_then(|one| one.get("properties"))
            .and_then(|one| one.get("table"))
            .unwrap();
        let said = table.get("description").and_then(Json::text).unwrap();
        assert!(said.contains("null"), "{name}: {said}");
        let items = table.get("items").unwrap();
        let kinds = written(items.get("type").unwrap());
        assert!(
            kinds.contains("null") && kinds.contains("integer"),
            "{name}: {kinds}"
        );
    }
}

#[test]
fn an_exam_scores_only_its_cared_rows_and_refuses_a_table_with_none() {
    let shape = Netlist::over(2, 1, 1, 16).unwrap();
    let exam = Exam::partial(shape, vec![Some(0), None, Some(1), None]).unwrap();
    assert!(exam.is_partial());
    assert_eq!(exam.cared(), 2);
    assert_eq!(exam.marks(), 2);
    assert_eq!(exam.scored(&[0, 1, 1, 0]), 2);
    assert_eq!(exam.scored(&[1, 1, 1, 1]), 1);
    assert_eq!(exam.scored(&[1, 0, 0, 0]), 0);
    let full = Exam::partial(shape, vec![Some(0), Some(1), Some(1), Some(0)]).unwrap();
    assert_eq!(full, Exam::of(shape, vec![0, 1, 1, 0]).unwrap());
    assert_eq!(
        Exam::partial(shape, vec![None; 4]),
        Err(NotAnExam::Nothing { rows: 4 })
    );
    assert!(matches!(
        Exam::partial(shape, vec![Some(0), None]),
        Err(NotAnExam::Rows { .. })
    ));
}

#[test]
fn a_cared_row_with_a_bit_past_the_outputs_is_refused_even_among_null_rows() {
    let shape = Netlist::over(2, 1, 1, 16).unwrap();
    assert!(matches!(
        Exam::partial(shape, vec![None, Some(2), None, None]),
        Err(NotAnExam::Stray { row: 1, .. })
    ));
}
