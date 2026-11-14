#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use grit::Host;
use grit::serve::handle;
use grit::wire::{Json, read, written};

pub const ALU: [u32; 128] = [
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 0, 15, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11,
    12, 13, 14, 15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0, 0, 2, 4, 6, 8, 10, 12, 14, 0,
    2, 4, 6, 8, 10, 12, 14, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 0, 15, 0, 1, 2, 3,
    4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0, 0,
    2, 4, 6, 8, 10, 12, 14, 0, 2, 4, 6, 8, 10, 12, 14,
];

pub const HAND: [[u32; 3]; 22] = [
    [8, 4, 5],
    [14, 0, 5],
    [6, 8, 4],
    [14, 1, 5],
    [6, 10, 4],
    [8, 9, 11],
    [14, 2, 5],
    [6, 13, 4],
    [8, 12, 14],
    [6, 1, 0],
    [8, 7, 16],
    [14, 9, 17],
    [6, 2, 1],
    [8, 7, 19],
    [14, 12, 20],
    [6, 3, 2],
    [8, 7, 22],
    [14, 15, 23],
    [1, 0, 7],
    [6, 1, 18],
    [6, 2, 21],
    [6, 3, 24],
];

pub fn call(host: Option<&Host>, name: &str, arguments: &str) -> Json {
    let asked = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{{\"name\":\"{name}\",\"arguments\":{arguments}}}}}"
    );
    let answer = handle(host, &read(&asked).unwrap()).unwrap();
    answer
        .get("result")
        .and_then(|one| one.get("structuredContent"))
        .cloned()
        .unwrap_or_else(|| panic!("{}", written(&answer)))
}

pub fn joined(rows: &[u32]) -> String {
    let all: Vec<String> = rows.iter().map(u32::to_string).collect();
    all.join(",")
}

pub fn adder() -> Vec<u32> {
    (0..16_u32)
        .map(|row| (row & 3) + ((row >> 2) & 3))
        .collect()
}

pub fn majority() -> Vec<u32> {
    (0..8_u32)
        .map(|row| u32::from(row.count_ones() >= 2))
        .collect()
}

pub fn hand_chip() -> String {
    let gates: Vec<String> = HAND
        .iter()
        .map(|[kind, left, right]| format!("{{\"kind\":{kind},\"left\":{left},\"right\":{right}}}"))
        .collect();
    format!("[{}]", gates.join(","))
}

fn solved(held: &Json, label: &str) -> String {
    assert_eq!(
        held.get("solved"),
        Some(&Json::Bool(true)),
        "{label} did not solve, so it cannot be replayed: {}",
        written(held)
    );
    format!("{label} {}", written(held))
}

pub fn replay(host: &Host) -> String {
    let mut lines = Vec::new();
    for seed in 1..=20_u32 {
        let held = call(
            Some(host),
            "grit_synthesise",
            &format!(
                "{{\"inputs\":4,\"outputs\":3,\"gates\":9,\"table\":[{}],\"crowd\":256,\"rounds\":300,\"seed\":{seed}}}",
                joined(&adder())
            ),
        );
        lines.push(solved(&held, &format!("adder-shake-{seed}")));
        let held = call(
            Some(host),
            "grit_synthesise",
            &format!(
                "{{\"inputs\":4,\"outputs\":3,\"gates\":9,\"table\":[{}],\"crowd\":256,\"rounds\":300,\"seed\":{seed},\"escape\":\"credit\"}}",
                joined(&adder())
            ),
        );
        lines.push(solved(&held, &format!("adder-credit-{seed}")));
        let held = call(
            Some(host),
            "grit_synthesise",
            &format!(
                "{{\"inputs\":3,\"outputs\":1,\"gates\":4,\"table\":[{}],\"crowd\":128,\"rounds\":200,\"seed\":{seed}}}",
                joined(&majority())
            ),
        );
        lines.push(solved(&held, &format!("majority-{seed}")));
    }
    for seed in [3_u32, 11, 17] {
        let held = call(
            Some(host),
            "grit_least",
            &format!(
                "{{\"inputs\":3,\"outputs\":1,\"gates\":6,\"table\":[{}],\"crowd\":128,\"rounds\":200,\"seed\":{seed}}}",
                joined(&majority())
            ),
        );
        let ladder: Vec<String> = held
            .get("ladder")
            .unwrap()
            .list()
            .unwrap()
            .iter()
            .map(|rung| {
                let solved = rung.get("solved") == Some(&Json::Bool(true));
                if solved {
                    written(rung)
                } else {
                    format!("{} unsolved", rung.counted("gates").unwrap())
                }
            })
            .collect();
        lines.push(format!(
            "least-{seed} {} {} {}",
            written(held.get("smallest_that_solved").unwrap()),
            ladder.join(";"),
            written(held.get("circuit").unwrap())
        ));
    }
    lines.extend(cpu_replay());
    lines.join("\n") + "\n"
}

pub fn cpu_replay() -> Vec<String> {
    let mut lines = Vec::new();
    let chip = hand_chip();
    let alu = joined(&ALU);
    let held = call(
        None,
        "grit_check",
        &format!("{{\"inputs\":7,\"outputs\":4,\"gates\":22,\"table\":[{alu}],\"chip\":{chip}}}"),
    );
    lines.push(format!("check-hand {}", written(&held)));
    let broken = chip.replacen("\"kind\":6", "\"kind\":9", 3);
    let held = call(
        None,
        "grit_check",
        &format!("{{\"inputs\":7,\"outputs\":4,\"gates\":22,\"table\":[{alu}],\"chip\":{broken}}}"),
    );
    lines.push(format!("check-broken {}", written(&held)));
    for depth in [1, 2] {
        let held = call(
            None,
            "grit_credit",
            &format!(
                "{{\"inputs\":7,\"outputs\":4,\"gates\":22,\"table\":[{alu}],\"chip\":{chip},\"depth\":{depth}}}"
            ),
        );
        lines.push(format!("credit-hand-{depth} {}", written(&held)));
        let held = call(
            None,
            "grit_credit",
            &format!(
                "{{\"inputs\":7,\"outputs\":4,\"gates\":22,\"table\":[{alu}],\"chip\":{broken},\"depth\":{depth}}}"
            ),
        );
        lines.push(format!("credit-broken-{depth} {}", written(&held)));
    }
    lines
}
