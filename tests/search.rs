#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use std::time::Duration;

use common::device;

use grit::machine::Machine;
use grit::marking::{BINDINGS, HELD, SEED, TRIAL, turned_on, words_of};
use grit::netlist::{Exam, Gate, Netlist, drawn, turned};
use grit::serve::{
    ASIDE, Effort, Pace, Rolls, SEEDING, SUBMISSION, handle, machined, mixed, opening, pressed,
    shaken,
};
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

const fn unmixed(held: u32) -> u32 {
    let held = held ^ (held >> 16);
    let held = held.wrapping_mul(0x7ed1_b41d);
    let held = held ^ (held >> 13) ^ (held >> 26);
    let held = held.wrapping_mul(0xa5cb_9243);
    held ^ (held >> 16)
}

#[test]
fn seeds_whose_mix_differs_in_its_lowest_bit_open_their_own_crowds() {
    let shape = shape();
    for seed in [SEEDING, 1, 7, 12_345] {
        let twin = unmixed(mixed(seed) ^ 1);
        assert_eq!(mixed(twin), mixed(seed) ^ 1);
        assert_ne!(
            opening(shape, seed, 0, 256).0,
            opening(shape, twin, 0, 256).0,
            "seeds {seed} and {twin} open one crowd"
        );
    }
    let gates = shape.gates as usize;
    let empty = unmixed(mixed(SEEDING) ^ SEEDING ^ 1);
    let (crowd, _) = opening(shape, empty, 0, 64);
    assert!(
        crowd
            .chunks(gates)
            .skip(1)
            .any(|one| one != &crowd[..gates]),
        "the seed whose crowd state would be zero still draws a crowd"
    );
    let high = unmixed(mixed(SEEDING) ^ SEEDING ^ 1 ^ 0x8000_0000);
    assert_ne!(opening(shape, empty, 0, 64), opening(shape, high, 0, 64));
}

const K: u32 = 2_654_435_761;

fn rolled(state: u32) -> u32 {
    let state = state ^ (state << 13);
    let state = state ^ (state >> 17);
    state ^ (state << 5)
}

#[test]
fn the_roll_has_no_word_it_keeps_but_zero() {
    let mut rows: Vec<u32> = (0..32).map(|bit| rolled(1 << bit) ^ (1 << bit)).collect();
    let mut rank = 0;
    for bit in 0..32 {
        let Some(at) = (rank..rows.len()).find(|at| (rows[*at] >> bit) & 1 == 1) else {
            continue;
        };
        rows.swap(rank, at);
        let pivot = rows[rank];
        for (other, one) in rows.iter_mut().enumerate() {
            if other != rank && (*one >> bit) & 1 == 1 {
                *one ^= pivot;
            }
        }
        rank += 1;
    }
    assert_eq!(
        rank, 32,
        "the roll minus the identity must be invertible, so no word but zero rolls to itself"
    );
}

#[test]
fn the_stream_of_a_zero_start_is_no_other_start_stream() {
    let mut zero = Rolls::from(0);
    let (first, second) = (zero.rolled(), zero.rolled());
    let mut taken = Rolls::from(ASIDE);
    assert_ne!((first, second), (taken.rolled(), taken.rolled()));
    for start in [
        1_u32,
        2,
        ASIDE,
        rolled(ASIDE),
        first,
        second,
        u32::MAX,
        0x8000_0000,
    ] {
        let mut other = Rolls::from(start);
        let one = other.rolled();
        let two = other.rolled();
        assert_ne!(
            (first, second),
            (one, two),
            "the zero start rolls as the start {start}"
        );
        assert_eq!(
            one,
            rolled(start),
            "a start that is not zero rolls as it always did"
        );
    }
    let shape = shape();
    let crowd = opening(shape, SEEDING, 0, 64).0;
    assert_ne!(
        shaken(shape, &crowd, 6, 0),
        shaken(shape, &crowd, 6, ASIDE),
        "the shake whose seed is zero shakes as no other seed does"
    );
}

#[test]
fn twin_seeds_hand_the_device_no_seed_in_common() {
    let shape = shape();
    let many = 4096;
    for seed in [SEEDING, 1, 7, 12_345, 0, u32::MAX] {
        let twin = unmixed(mixed(seed) ^ 1);
        let one = opening(shape, seed, 0, many);
        let two = opening(shape, twin, 0, many);
        assert_ne!(one.0, two.0, "seeds {seed} and {twin} open one crowd");
        let shared = one.1.iter().zip(&two.1).filter(|(a, b)| a == b).count();
        assert_eq!(
            shared, 0,
            "seeds {seed} and {twin} hand {shared} candidates the same device seed"
        );
        assert!(one.1.iter().chain(&two.1).all(|held| *held != 0));
    }
}

#[test]
fn the_seed_whose_state_would_be_zero_shares_its_crowd_with_no_other() {
    let shape = shape();
    let empty = unmixed(mixed(SEEDING) ^ SEEDING ^ 1);
    let high = unmixed(mixed(SEEDING) ^ SEEDING ^ 1 ^ 0x8000_0000);
    let aside = unmixed(mixed(SEEDING) ^ SEEDING ^ 1 ^ ASIDE);
    let (crowd, _) = opening(shape, empty, 0, 64);
    for other in [high, aside, SEEDING] {
        assert_ne!(
            crowd,
            opening(shape, other, 0, 64).0,
            "seeds {empty} and {other} open one crowd"
        );
    }
}

#[test]
fn a_device_seed_that_would_be_zero_is_one_no_other_candidate_holds() {
    let shape = Netlist::over(3, 1, 4, 16).unwrap();
    let many = 1024;
    let plain = opening(shape, SEEDING, 0, many).1;
    for who in [0_u32, 1, 2, 77, 1023] {
        let salt = who.wrapping_mul(K) | 1;
        let seed = unmixed(salt ^ mixed(SEEDING));
        let seeds = opening(shape, seed, 0, many).1;
        assert!(
            seeds.iter().all(|held| *held != 0),
            "seed {seed} hands the device a zero"
        );
        let mut sorted = seeds.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            seeds.len(),
            "seed {seed} gives two candidates one seed"
        );
        let twin = opening(shape, unmixed(mixed(seed) ^ 1), 0, many).1;
        for (name, other) in [("the default seed", &plain), ("its twin", &twin)] {
            let shared = (0..many as usize)
                .filter(|at| seeds[*at] == other[*at])
                .count();
            assert_eq!(
                shared, 0,
                "seed {seed} and {name} share {shared} device seeds"
            );
        }
        let partner = unmixed(salt ^ (mixed(salt) | 1) ^ mixed(SEEDING));
        let theirs = opening(shape, partner, 0, many).1;
        let shared: Vec<usize> = (0..many as usize)
            .filter(|at| seeds[*at] == theirs[*at])
            .collect();
        assert_eq!(
            shared,
            vec![who as usize],
            "seed {seed} shares more than the one candidate a 32-bit seed must share with some other seed"
        );
    }
}

fn found_of(found: &grit::serve::Found) -> String {
    format!(
        "{} {} {} {} {:?} {:?}",
        found.marks, found.full, found.rounds, found.crowd, found.gates, found.table
    )
}

#[test]
fn a_search_answers_alike_however_its_rounds_are_split_into_submissions() {
    let Some(host) = device("a_search_answers_alike_however_its_rounds_are_split_into_submissions")
    else {
        return;
    };
    let table: Vec<u32> = (0..32_u32).map(u32::count_ones).collect();
    for (gates, crowd, rounds, stall) in [(9, 256, 750, 100), (12, 2048, 1000, 300)] {
        let shape = Netlist::over(5, 3, gates, 16).unwrap();
        let exam = Exam::of(shape, table.clone()).unwrap();
        let ran = |submitted: u32| {
            let effort = Effort {
                crowd,
                rounds,
                seed: 41,
                stall,
                shake: 6,
                submitted,
            };
            found_of(&pressed(&host, shape, &exam, effort).unwrap())
        };
        let whole = ran(u32::MAX);
        eprintln!("popcount of five at {gates} gates, crowd {crowd}: {whole}");
        for submitted in [1, 3, 7, 64, 100] {
            assert_eq!(
                ran(submitted),
                whole,
                "at most {submitted} rounds a submission answers otherwise"
            );
        }
    }
}

#[test]
fn a_machine_search_answers_alike_however_its_rounds_are_split_into_submissions() {
    let Some(host) =
        device("a_machine_search_answers_alike_however_its_rounds_are_split_into_submissions")
    else {
        return;
    };
    let held = Machine::over(2, 1, 1, 6, 16, 12).unwrap();
    let traces: Vec<(Vec<u32>, Vec<u32>)> = (0..40_u32)
        .map(|run| {
            let feed: Vec<u32> = (0..12).map(|step| (run >> (step % 6)) & 1).collect();
            let mut count = 0_u32;
            let want = feed
                .iter()
                .map(|one| {
                    count = (count + one) % 3;
                    u32::from(count == 0)
                })
                .collect();
            (feed, want)
        })
        .collect();
    let ran = |submitted: u32| {
        let effort = Effort {
            crowd: 512,
            rounds: 420,
            seed: 5,
            stall: 100,
            shake: 6,
            submitted,
        };
        let told = machined(&host, held, &traces, effort, 0).unwrap();
        format!(
            "{} {} {} {:?} {:?}",
            told.marks, told.full, told.rounds, told.gates, told.said
        )
    };
    let whole = ran(u32::MAX);
    eprintln!("count of ones by three: {whole}");
    for submitted in [1, 4, 13, 50] {
        assert_eq!(
            ran(submitted),
            whole,
            "at most {submitted} rounds a submission answers otherwise"
        );
    }
}

#[test]
fn a_submission_is_sized_to_the_measured_cost_of_its_work() {
    let mut pace = Pace::over(1_000_000, u32::MAX);
    assert_eq!(
        pace.next(100),
        1,
        "before anything is measured one round goes alone"
    );
    pace.learn(1, Duration::from_millis(30), None);
    let planned = pace.next(100);
    assert_eq!(planned, (SUBMISSION.as_secs_f64() / 0.030).floor() as u32);
    pace.learn(1, Duration::from_millis(2), Some(Duration::from_millis(40)));
    assert_eq!(
        pace.next(100),
        (SUBMISSION.as_secs_f64() / 0.040).floor() as u32,
        "the slower of the wall and the device clock sizes the next"
    );
    pace.learn(5, Duration::from_micros(50), None);
    assert_eq!(pace.next(100), 100, "a cheap round fills the look");
    assert_eq!(pace.next(7), 7, "never past the look");
    pace.learn(1, SUBMISSION * 3, None);
    assert_eq!(pace.next(100), 1, "a round over the span goes alone");
    let mut capped = Pace::over(10, 4);
    capped.learn(10, Duration::from_nanos(10), None);
    assert_eq!(capped.next(100), 4, "never past what the caller allows");
    let mut light = Pace::over(1, u32::MAX);
    light.learn(10, Duration::from_micros(10), None);
    let each = light.measured().unwrap();
    assert!(
        (each - 1e-6).abs() < 1e-12,
        "a unit of work costs the time over the work"
    );
    for (work, nanos) in [
        (1_u64, 1_u64),
        (1 << 20, 3),
        (1 << 26, 40),
        (12_345, 99_999),
    ] {
        let mut pace = Pace::over(work, u32::MAX);
        pace.learn(1, Duration::from_nanos(nanos * work), None);
        let rounds = pace.next(u32::MAX);
        let planned = Duration::from_nanos(nanos * work * u64::from(rounds));
        assert!(
            rounds == 1 || planned <= SUBMISSION,
            "work {work} at {nanos} ns a unit plans {rounds} rounds, {planned:?}"
        );
    }
}

const ALU_SEARCH: &str = r#"472 512 300 65536 [{"kind":2,"left":5,"right":4},{"kind":9,"left":4,"right":1},{"kind":5,"left":2,"right":5},{"kind":6,"left":3,"right":7},{"kind":1,"left":0,"right":7},{"kind":4,"left":0,"right":11},{"kind":5,"left":7,"right":10},{"kind":9,"left":10,"right":9},{"kind":7,"left":8,"right":5},{"kind":8,"left":4,"right":5},{"kind":9,"left":11,"right":0},{"kind":14,"left":16,"right":8},{"kind":9,"left":3,"right":3},{"kind":7,"left":16,"right":14},{"kind":6,"left":0,"right":18},{"kind":11,"left":20,"right":4},{"kind":7,"left":22,"right":14},{"kind":3,"left":13,"right":0},{"kind":4,"left":16,"right":24},{"kind":9,"left":17,"right":21},{"kind":9,"left":5,"right":9},{"kind":9,"left":23,"right":2}]"#;

#[test]
fn an_alu_search_over_the_whole_crowd_completes_and_answers_alike_on_every_device() {
    let Some(host) =
        device("an_alu_search_over_the_whole_crowd_completes_and_answers_alike_on_every_device")
    else {
        return;
    };
    let held = call(
        Some(&host),
        "grit_synthesise",
        &format!(
            "{{\"inputs\":7,\"outputs\":4,\"gates\":22,\"table\":[{}],\"crowd\":65536,\"rounds\":300,\"stall\":100}}",
            rows()
        ),
    );
    let answer = format!(
        "{} {} {} {} {}",
        held.counted("marks").unwrap(),
        held.counted("of").unwrap(),
        held.counted("rounds").unwrap(),
        held.counted("crowd").unwrap(),
        written(held.get("gates").unwrap())
    );
    eprintln!("ALU at 22 gates on {}: {answer}", host.name());
    assert_eq!(
        answer,
        ALU_SEARCH,
        "the same seed answers otherwise on {}",
        host.name()
    );
}
