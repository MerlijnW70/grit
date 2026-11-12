#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use grit::marking::{BINDINGS, HELD, SEED, TRIAL, turned_on, words_of};
use grit::netlist::{Gate, Netlist, drawn, turned};
use grit::serve::{SEEDING, handle, opening};
use grit::wire::{Json, read, written};
use grit::{Groups, Host};

const ALU: [u32; 128] = [
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 0, 15, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11,
    12, 13, 14, 15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0, 0, 2, 4, 6, 8, 10, 12, 14, 0,
    2, 4, 6, 8, 10, 12, 14, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 0, 15, 0, 1, 2, 3,
    4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0, 0,
    2, 4, 6, 8, 10, 12, 14, 0, 2, 4, 6, 8, 10, 12, 14,
];

const HAND: [[u32; 3]; 22] = [
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

fn rows() -> String {
    let all: Vec<String> = ALU.iter().map(u32::to_string).collect();
    all.join(",")
}

fn call(host: Option<&Host>, name: &str, arguments: &str) -> Json {
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

fn shape() -> Netlist {
    Netlist::over(7, 4, 22, 16).unwrap()
}

#[test]
fn the_fixture_is_the_four_bit_alu_with_its_third_opcode_bit_ignored() {
    for (word, wanted) in ALU.iter().enumerate() {
        let data = word as u32 & 15;
        let answer = match (word >> 4) & 3 {
            0 => (data + 1) & 15,
            1 => data.wrapping_sub(1) & 15,
            2 => !data & 15,
            _ => (data << 1) & 15,
        };
        assert_eq!(answer, *wanted, "row {word}");
    }
}

#[test]
fn the_hand_design_of_twenty_two_gates_checks_every_bit() {
    let chip: Vec<String> = HAND
        .iter()
        .map(|[kind, left, right]| format!("{{\"kind\":{kind},\"left\":{left},\"right\":{right}}}"))
        .collect();
    let held = call(
        None,
        "grit_check",
        &format!(
            "{{\"inputs\":7,\"outputs\":4,\"gates\":22,\"table\":[{}],\"chip\":[{}]}}",
            rows(),
            chip.join(",")
        ),
    );
    assert_eq!(held.counted("of"), Some(512));
    assert_eq!(held.counted("marks"), Some(512), "{}", written(&held));
    assert_eq!(held.get("solved"), Some(&Json::Bool(true)));
    assert_eq!(
        held.get("wrong_rows")
            .and_then(Json::list)
            .map(<[Json]>::len),
        Some(0)
    );
}

fn first_crowd(shape: Netlist, many: u32) -> (Vec<u32>, Vec<u32>) {
    let mut state = SEEDING | 1;
    let crowd = (0..many * shape.gates)
        .map(|at| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            drawn(&shape, at % shape.gates, state).pack()
        })
        .collect();
    let seeds = (0..many)
        .map(|who| who.wrapping_mul(2_654_435_761) | 1)
        .collect();
    (crowd, seeds)
}

#[test]
fn the_default_seed_starts_where_the_search_always_started() {
    let shape = shape();
    assert_eq!(
        opening(shape, SEEDING, 0, 512),
        first_crowd(shape, 512),
        "a call without a seed must keep answering byte for byte as before"
    );
}

#[test]
fn every_restart_starts_from_a_crowd_of_its_own() {
    let shape = shape();
    let many = 256;
    let crowds: Vec<(Vec<u32>, Vec<u32>)> = (0..6)
        .map(|restart| opening(shape, SEEDING, restart, many))
        .collect();
    for (one, first) in crowds.iter().enumerate() {
        for (other, second) in crowds.iter().enumerate().skip(one + 1) {
            assert_ne!(
                first.0, second.0,
                "restarts {one} and {other} drew one crowd"
            );
            assert_ne!(
                first.1, second.1,
                "restarts {one} and {other} drew one set of seeds"
            );
        }
    }
    let doubled = opening(shape, SEEDING, 1, many * 2);
    let before = opening(shape, SEEDING, 0, many);
    assert_ne!(
        &doubled.0[..(many * shape.gates) as usize],
        &before.0[..],
        "a doubled crowd that begins with the crowd before it searches that ground again"
    );
    let shared = doubled
        .0
        .chunks(shape.gates as usize)
        .filter(|one| before.0.chunks(shape.gates as usize).any(|two| two == *one))
        .count();
    assert_eq!(
        shared, 0,
        "no candidate of the restart repeats one of the first run"
    );
    assert_eq!(
        opening(shape, SEEDING, 3, many),
        crowds[3],
        "a restart is drawn from the seed and its number alone"
    );
}

#[test]
fn seeds_next_to_each_other_start_apart() {
    let shape = shape();
    for seed in [0_u32, 2, 4, 1 << 31] {
        let one = opening(shape, seed, 0, 64);
        let next = opening(shape, seed + 1, 0, 64);
        assert_ne!(
            one.0,
            next.0,
            "seeds {seed} and {} drew one crowd",
            seed + 1
        );
        assert_ne!(one.1, next.1, "seeds {seed} and {} drew one set", seed + 1);
    }
}

fn changed(shape: Netlist, at: u32, before: u32, after: u32, counts: &mut [u64; 4]) {
    let was = Gate::unpack(before);
    let now = Gate::unpack(after);
    assert!(
        shape.checked(at, now).is_ok(),
        "a turned gate left the field"
    );
    let fields = [
        was.kind != now.kind,
        was.left != now.left,
        was.right != now.right,
    ];
    assert!(
        fields.iter().filter(|one| **one).count() <= 1,
        "one turn changed two fields: {was:?} to {now:?}"
    );
    match fields.iter().position(|one| *one) {
        Some(field) => counts[field] += 1,
        None => counts[3] += 1,
    }
}

fn even(counts: [u64; 4], draws: u64) {
    let [kind, left, right, _] = counts;
    eprintln!(
        "{draws} turns: kind {kind}, left {left}, right {right}, unchanged {}",
        counts[3]
    );
    let apart = left.abs_diff(right);
    assert!(
        apart * 25 < left + right,
        "left and right must be chosen alike: left {left}, right {right}"
    );
    let third = draws / 3;
    for (name, one) in [("kind", kind), ("left", left), ("right", right)] {
        assert!(
            one * 10 > third * 8 && one * 10 < third * 11,
            "{name} turned {one} times of {draws}, not about a third"
        );
    }
}

#[test]
fn the_host_turns_kind_left_and_right_alike() {
    let shape = shape();
    let mut counts = [0_u64; 4];
    let mut state = 0x2545_f491_u32;
    let draws = 300_000_u64;
    for draw in 0..draws {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        let at = 12 + (draw % 10) as u32;
        let gate = drawn(&shape, at, state.rotate_left(7));
        changed(
            shape,
            at,
            gate.pack(),
            turned(&shape, at, gate, state).pack(),
            &mut counts,
        );
    }
    even(counts, draws);
}

#[test]
fn the_device_turns_kind_left_and_right_alike() {
    let Some(host) = device("the_device_turns_kind_left_and_right_alike") else {
        return;
    };
    let width = host.width();
    let shape = shape();
    let many = 65536_u32;
    let module = turned_on(width, shape).unwrap();
    let modules: Vec<&[u32]> = vec![&module];
    let sizes: [usize; BINDINGS as usize] = [
        (many * shape.gates) as usize,
        (many * shape.gates) as usize,
        (many * shape.sources() * words_of(shape)) as usize,
        (shape.outputs * words_of(shape)) as usize,
        many as usize,
        many as usize,
        many as usize,
        1,
    ];
    let mut resident = host.resident(&sizes, &modules).unwrap();
    let (crowd, seeds) = opening(shape, 0x00c0_ffee, 0, many);
    resident.write(HELD as usize, &crowd).unwrap();
    resident
        .write(TRIAL as usize, &vec![0_u32; sizes[TRIAL as usize]])
        .unwrap();
    resident.write(SEED as usize, &seeds).unwrap();
    let over = [Groups::Fixed(many.div_ceil(width))];
    let stages: Vec<&[Groups]> = vec![&over];
    resident.staged(&stages).unwrap();
    let trial = resident
        .read(TRIAL as usize, sizes[TRIAL as usize])
        .unwrap();
    let mut counts = [0_u64; 4];
    for (who, (before, after)) in crowd
        .chunks(shape.gates as usize)
        .zip(trial.chunks(shape.gates as usize))
        .enumerate()
    {
        let moved: Vec<usize> = (0..before.len())
            .filter(|at| before[*at] != after[*at])
            .collect();
        assert!(
            moved.len() <= 1,
            "candidate {who} turned {} gates",
            moved.len()
        );
        let at = moved.first().copied().unwrap_or(0);
        changed(shape, at as u32, before[at], after[at], &mut counts);
    }
    even(counts, u64::from(many));
}

fn popcount() -> String {
    let all: Vec<String> = (0..32_u32)
        .map(|row| row.count_ones().to_string())
        .collect();
    all.join(",")
}

#[test]
fn a_budget_that_held_the_search_one_mark_short_now_solves() {
    let Some(host) = device("a_budget_that_held_the_search_one_mark_short_now_solves") else {
        return;
    };
    let held = call(
        Some(&host),
        "grit_synthesise",
        &format!(
            "{{\"inputs\":5,\"outputs\":3,\"gates\":12,\"table\":[{}],\"seed\":{SEEDING}}}",
            popcount()
        ),
    );
    eprintln!(
        "popcount of five at 12 gates, seed {SEEDING}: {} of {} in {} rounds",
        held.counted("marks").unwrap(),
        held.counted("of").unwrap(),
        held.counted("rounds").unwrap()
    );
    assert_eq!(
        held.get("solved"),
        Some(&Json::Bool(true)),
        "{}",
        written(&held)
    );
    let chip = written(held.get("gates").unwrap());
    let again = call(
        None,
        "grit_check",
        &format!(
            "{{\"inputs\":5,\"outputs\":3,\"gates\":12,\"table\":[{}],\"chip\":{chip}}}",
            popcount()
        ),
    );
    assert_eq!(again.counted("marks"), Some(96), "{}", written(&again));
}

#[test]
fn a_seed_gives_the_same_answer_twice_and_another_seed_another() {
    let Some(host) = device("a_seed_gives_the_same_answer_twice_and_another_seed_another") else {
        return;
    };
    let adder: Vec<String> = (0..16_u32)
        .map(|row| ((row & 3) + ((row >> 2) & 3)).to_string())
        .collect();
    let asked = |seed: u32| {
        let held = call(
            Some(&host),
            "grit_synthesise",
            &format!(
                "{{\"inputs\":4,\"outputs\":3,\"gates\":4,\"table\":[{}],\"crowd\":64,\"rounds\":50,\"seed\":{seed}}}",
                adder.join(",")
            ),
        );
        written(held.get("gates").unwrap())
    };
    let one = asked(11);
    assert_eq!(one, asked(11), "a run is reproducible from its seed");
    assert_ne!(one, asked(12), "another seed searches other ground");
}
