#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use std::collections::BTreeMap;

use grit::Host;
use grit::serve3::Server;
use grit::wire::{Json, written};

const LONG: u64 = 600_000;

const BASE_SHA: [(&str, &str); 5] = [
    (
        "life",
        "2ac7a034eebcab8e480aa3c1f5cfc651899653213e2104e533eccf7b53152bad",
    ),
    (
        "majority",
        "7fa32a256d83d47dd154332af8c5a8d2a30c3f5278bcf4b16f611223757c50ed",
    ),
    (
        "vn",
        "70c3e9bcf5b3685ab497fb1b45e1454e5bcd8c2a4406abb19eba0e18f55f8c72",
    ),
    (
        "not_counting",
        "49bd524f5a68635ba38d99ce910761bea4b9cae99f1fcb360463add21b336164",
    ),
    (
        "conflict",
        "4b17c7eb8a865b5194816657a4683a49cf3b0c0a442fd0f9dc3e32864a086f90",
    ),
];

const MOORE: [u32; 9] = [0, 1, 2, 3, 4, 5, 6, 7, 8];

const VN: [u32; 5] = [1, 3, 4, 5, 7];

const SPACE: i64 = 1 << 18;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, most: u64) -> u64 {
        self.next() % most
    }
}

fn line(name: &str, arguments: &str) -> String {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{{\"name\":\"{name}\",\"arguments\":{arguments}}}}}"
    )
}

fn result(got: &Json, error: bool) -> Json {
    let held = got
        .get("result")
        .unwrap_or_else(|| panic!("{}", written(got)));
    assert_eq!(
        held.get("isError"),
        Some(&Json::Bool(error)),
        "{}",
        &written(got)[..written(got).len().min(4000)]
    );
    held.get("structuredContent").cloned().unwrap()
}

fn text_at(held: &Json, key: &str) -> String {
    held.get(key)
        .and_then(Json::text)
        .unwrap_or_else(|| panic!("no {key} in {}", written(held)))
        .to_owned()
}

fn int_at(held: &Json, key: &str) -> i64 {
    held.get(key)
        .and_then(Json::int)
        .unwrap_or_else(|| panic!("no {key} in {}", written(held)))
}

fn sha_of(held: &Json) -> String {
    grit::law::hex(&grit::branch::sha256(written(held).as_bytes()))
}

fn listed(held: &[u32]) -> String {
    format!(
        "[{}]",
        held.iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",")
    )
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Rule {
    Totalistic,
    Outer,
}

impl Rule {
    const fn name(self) -> &'static str {
        match self {
            Self::Totalistic => "totalistic",
            Self::Outer => "outer",
        }
    }
}

#[derive(Clone)]
struct Law {
    rule: Rule,
    places: Vec<u32>,
    code: u64,
}

impl Law {
    fn counts(&self) -> u32 {
        match self.rule {
            Rule::Totalistic => self.places.len() as u32 + 1,
            Rule::Outer => self.places.len() as u32,
        }
    }

    fn classes(&self) -> u32 {
        match self.rule {
            Rule::Totalistic => self.counts(),
            Rule::Outer => 2 * self.counts(),
        }
    }

    fn of(rule: Rule, places: &[u32], born: &[u32], kept: &[u32]) -> Self {
        let mut law = Self {
            rule,
            places: places.to_vec(),
            code: 0,
        };
        let counts = law.counts();
        law.code = born.iter().map(|n| 1_u64 << n).sum::<u64>()
            + kept.iter().map(|n| 1_u64 << (counts + n)).sum::<u64>();
        law
    }

    fn with(&self, code: u64) -> Self {
        Self {
            code,
            ..self.clone()
        }
    }

    fn shown(&self) -> String {
        let counts = self.counts();
        let row = |from: u32| -> String {
            let held: Vec<u32> = (from..from + counts)
                .map(|class| u32::from((self.code >> class) & 1 == 1))
                .collect();
            listed(&held)
        };
        match self.rule {
            Rule::Totalistic => format!("[{}]", row(0)),
            Rule::Outer => format!("[[{},{}]]", row(0), row(counts)),
        }
    }
}

#[derive(Clone, Copy)]
struct Grid {
    wide: u32,
    deep: u32,
}

impl Grid {
    const fn cells(self) -> usize {
        (self.wide * self.deep) as usize
    }

    fn at(self, x: i64, y: i64, wrapped: bool) -> Option<usize> {
        let (w, d) = (i64::from(self.wide), i64::from(self.deep));
        let (x, y) = if wrapped {
            (x.rem_euclid(w), y.rem_euclid(d))
        } else {
            (x, y)
        };
        ((0..w).contains(&x) && (0..d).contains(&y)).then(|| usize::try_from(y * w + x).unwrap())
    }
}

fn stepped(grid: Grid, cells: &[u32], wrapped: bool, law: &Law) -> Vec<u32> {
    let counts = law.counts();
    let mut out = vec![0; grid.cells()];
    for y in 0..i64::from(grid.deep) {
        for x in 0..i64::from(grid.wide) {
            let here = grid.at(x, y, wrapped).unwrap();
            let mut n = 0;
            for place in &law.places {
                if law.rule == Rule::Outer && *place == 4 {
                    continue;
                }
                let p = i64::from(*place);
                if let Some(at) = grid.at(x + p % 3 - 1, y + p / 3 - 1, wrapped) {
                    n += cells[at];
                }
            }
            let class = match law.rule {
                Rule::Totalistic => n,
                Rule::Outer => cells[here] * counts + n,
            };
            out[here] = u32::from((law.code >> class) & 1 == 1);
        }
    }
    out
}

fn applied(grid: Grid, cells: &[u32], wrapped: bool, law: &Law, steps: u32) -> Vec<u32> {
    let mut held = cells.to_vec();
    for _ in 0..steps {
        held = stepped(grid, &held, wrapped, law);
    }
    held
}

fn soup(rng: &mut Rng, grid: Grid, permille: u64) -> Vec<u32> {
    (0..grid.cells())
        .map(|_| u32::from(rng.below(1000) < permille))
        .collect()
}

struct Pair {
    grid: Grid,
    input: Vec<u32>,
    output: Vec<u32>,
}

#[derive(Clone, Copy)]
struct Making {
    warm_up: u32,
    apart: u32,
    permille: u64,
    warmed_from: usize,
}

const fn warmed(warm_up: u32, apart: u32) -> Making {
    Making {
        warm_up,
        apart,
        permille: 350,
        warmed_from: 0,
    }
}

const fn as_rt1(apart: u32, count: usize) -> Making {
    Making {
        warm_up: 10,
        apart,
        permille: 350,
        warmed_from: count / 2,
    }
}

fn pairs_by(
    seed: u64,
    count: usize,
    grid: Grid,
    making: Making,
    each: &dyn Fn(&[u32]) -> Vec<u32>,
) -> Vec<Pair> {
    let mut rng = Rng(seed);
    (0..count)
        .map(|at| {
            let mut input = soup(&mut rng, grid, making.permille);
            if at >= making.warmed_from {
                for _ in 0..making.warm_up {
                    input = each(&input);
                }
            }
            let apart = making.apart;
            let mut output = input.clone();
            for _ in 0..apart {
                output = each(&output);
            }
            Pair {
                grid,
                input,
                output,
            }
        })
        .collect()
}

fn pairs_of(
    seed: u64,
    count: usize,
    grid: Grid,
    making: Making,
    law: &Law,
    wrapped: bool,
) -> Vec<Pair> {
    pairs_by(seed, count, grid, making, &|cells| {
        stepped(grid, cells, wrapped, law)
    })
}

fn examples_of(pairs: &[Pair]) -> String {
    let shown: Vec<String> = pairs
        .iter()
        .map(|one| {
            format!(
                "{{\"size\":[{},{},1],\"input\":[{}],\"output\":[{}]}}",
                one.grid.wide,
                one.grid.deep,
                listed(&one.input),
                listed(&one.output)
            )
        })
        .collect();
    format!("[{}]", shown.join(","))
}

fn learn_args(rule: &str, border: &str, pairs: &[Pair], more: &str) -> String {
    format!(
        "{{\"stencil\":\"3x3\",\"reads\":[{{\"plane\":0,\"bit\":0}}],\"writes\":[{{\"plane\":1,\"bit\":0}}],\"border\":\"{border}\",\"rule\":\"{rule}\",\"examples\":{},\"budget_ms\":{LONG}{more}}}",
        examples_of(pairs)
    )
}

fn stepped_args(law: &Law, border: &str, pairs: &[Pair], steps: u32, more: &str) -> String {
    learn_args(
        law.rule.name(),
        border,
        pairs,
        &format!(",\"steps\":{steps}{more}"),
    )
}

fn life() -> Law {
    Law::of(Rule::Outer, &MOORE, &[3], &[2, 3])
}

fn high_life() -> Law {
    Law::of(Rule::Outer, &MOORE, &[3, 6], &[2, 3])
}

fn vn_rule() -> Law {
    Law::of(Rule::Outer, &VN, &[1], &[1, 2])
}

fn majority() -> Law {
    Law::of(Rule::Totalistic, &MOORE, &[5, 6, 7, 8, 9], &[])
}

const fn square() -> Grid {
    Grid { wide: 32, deep: 32 }
}

fn heard(server: &mut Server<'_>, arguments: &str) -> Json {
    server.heard(&line("grit_law_learn", arguments)).unwrap()
}

fn learned(server: &mut Server<'_>, arguments: &str) -> Json {
    result(&heard(server, arguments), false)
}

fn refused_as(server: &mut Server<'_>, arguments: &str, code: &str, field: &str) -> Json {
    let told = result(&heard(server, arguments), true);
    assert_eq!(
        (text_at(&told, "refused"), text_at(&told, "field")),
        (code.to_owned(), field.to_owned()),
        "{}",
        written(&told)
    );
    told
}

fn shown_of(held: &Json, key: &str) -> String {
    written(
        held.get(key)
            .unwrap_or_else(|| panic!("no {key} in {}", written(held))),
    )
}

fn code_of(tables: &Json, law: &Law) -> u64 {
    let write = &tables.list().unwrap()[0];
    let flat: Vec<i64> = match law.rule {
        Rule::Totalistic => write
            .list()
            .unwrap()
            .iter()
            .map(|bit| bit.int().unwrap())
            .collect(),
        Rule::Outer => write
            .list()
            .unwrap()
            .iter()
            .flat_map(|half| half.list().unwrap().iter().map(|bit| bit.int().unwrap()))
            .collect(),
    };
    assert_eq!(flat.len() as u32, law.classes());
    flat.iter()
        .enumerate()
        .map(|(class, bit)| u64::from(*bit == 1) << class)
        .sum()
}

fn codes_of(got: &Json, law: &Law) -> Vec<u64> {
    got.get("consistent_tables")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .map(|one| code_of(one, law))
        .collect()
}

fn wrong_of(law: &Law, pairs: &[Pair], wrapped: bool, steps: u32) -> u64 {
    pairs
        .iter()
        .map(|one| {
            applied(one.grid, &one.input, wrapped, law, steps)
                .iter()
                .zip(&one.output)
                .filter(|(a, b)| a != b)
                .count() as u64
        })
        .sum()
}

fn reproduces(law: &Law, pairs: &[Pair], wrapped: bool, steps: u32) -> bool {
    wrong_of(law, pairs, wrapped, steps) == 0
}

fn exhaustive(law: &Law, pairs: &[Pair], wrapped: bool, steps: u32) -> (u64, Vec<u64>, (u64, u64)) {
    let mut count = 0;
    let mut codes = Vec::new();
    let mut closest = (u64::MAX, 0);
    for code in 0..1_u64 << law.classes() {
        let wrong = wrong_of(&law.with(code), pairs, wrapped, steps);
        if wrong == 0 {
            count += 1;
            if codes.len() < 16 {
                codes.push(code);
            }
        } else if (wrong, code) < closest {
            closest = (wrong, code);
        }
    }
    (count, codes, closest)
}

fn under_determined(got: &Json, law: &Law, pairs: &[Pair], steps: u32) {
    assert_eq!(text_at(got, "stopped"), "complete");
    let consistent = int_at(got, "consistent");
    assert!(consistent > 1, "{}", written(got));
    let codes = codes_of(got, law);
    assert_eq!(codes.len(), usize::try_from(consistent.min(16)).unwrap());
    assert!(codes.windows(2).all(|two| two[0] < two[1]));
    assert!(codes.contains(&law.code));
    for code in &codes {
        assert!(
            reproduces(&law.with(*code), pairs, true, steps),
            "table {code}"
        );
    }
}

fn found_first(got: &Json, law: &Law, steps: u32) {
    assert_eq!(text_at(got, "stopped"), "complete", "{}", written(got));
    assert_eq!(got.get("complete"), Some(&Json::Bool(true)));
    assert_eq!(int_at(got, "steps"), i64::from(steps));
    assert_eq!(shown_of(got, "tables"), law.shown());
    assert_eq!(codes_of(got, law).first(), Some(&law.code));
    assert_eq!(int_at(got, "checked"), 1_i64 << law.classes());
}

fn complete_and_exact(got: &Json, law: &Law, steps: u32) {
    assert_eq!(text_at(got, "stopped"), "complete", "{}", written(got));
    assert_eq!(got.get("complete"), Some(&Json::Bool(true)));
    assert_eq!(int_at(got, "steps"), i64::from(steps));
    assert_eq!(
        int_at(got, "consistent"),
        1,
        "{}",
        shown_of(got, "consistent_tables")
    );
    assert_eq!(shown_of(got, "tables"), law.shown());
    assert_eq!(codes_of(got, law), vec![law.code]);
    let space = 1_i64 << law.classes();
    assert_eq!(int_at(got, "checked"), space);
    assert_eq!(int_at(got, "space"), space);
    assert_eq!(
        shown_of(got, "iterate"),
        format!("{{\"alternate\":[0,1],\"frames\":{steps}}}")
    );
}

fn called(server: &mut Server<'_>, name: &str, arguments: &str) -> Json {
    result(&server.heard(&line(name, arguments)).unwrap(), false)
}

fn device_ran(server: &mut Server<'_>, got: &Json, pair: &Pair, steps: u32) -> Vec<u32> {
    let law = written(&Json::Text(text_at(got, "law")));
    let made = called(
        server,
        "grit_world_create",
        &format!(
            "{{\"wide\":{},\"deep\":{},\"tall\":2,\"law\":{law},\"alternate\":[0,1],\"budget_ms\":{LONG}}}",
            pair.grid.wide, pair.grid.deep
        ),
    );
    let world = int_at(&made, "world");
    let cells: Vec<String> = (0..pair.grid.cells())
        .filter(|at| pair.input[*at] != 0)
        .map(|at| {
            format!(
                "[{},{},0,{}]",
                at as u32 % pair.grid.wide,
                at as u32 / pair.grid.wide,
                pair.input[at]
            )
        })
        .collect();
    if !cells.is_empty() {
        called(
            server,
            "grit_world_write",
            &format!(
                "{{\"world\":{world},\"current\":true,\"cells\":[{}],\"budget_ms\":{LONG}}}",
                cells.join(",")
            ),
        );
    }
    called(
        server,
        "grit_world_step",
        &format!("{{\"world\":{world},\"frames\":{steps},\"budget_ms\":{LONG}}}"),
    );
    let read = called(
        server,
        "grit_world_read",
        &format!(
            "{{\"world\":{world},\"current\":true,\"from\":[0,0,0],\"to\":[{},{},0]}}",
            pair.grid.wide - 1,
            pair.grid.deep - 1
        ),
    );
    called(
        server,
        "grit_world_close",
        &format!("{{\"world\":{world}}}"),
    );
    read.get("cells")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .map(|one| u32::try_from(one.int().unwrap()).unwrap())
        .collect()
}

fn iterated_on_device(host: Option<&Host>, got: &Json, pairs: &[Pair], steps: u32) {
    let Some(host) = host else {
        return;
    };
    let mut server = Server::new(Some(host));
    for (at, pair) in pairs.iter().enumerate() {
        let cells = device_ran(&mut server, got, pair, steps);
        assert_eq!(
            cells, pair.output,
            "example {at} run {steps} frames on the device"
        );
    }
}

#[test]
fn k1_life_seen_every_second_and_fourth_frame_is_found_alone_and_iterates_on_the_device() {
    let host = device("law_steps k1");
    let law = life();
    for steps in [2, 4] {
        let pairs = pairs_of(
            20_261_123 + u64::from(steps),
            8,
            square(),
            as_rt1(steps, 8),
            &law,
            true,
        );
        assert!(reproduces(&law, &pairs, true, steps));
        let mut server = Server::new(None);
        let all_warmed = pairs_of(
            20_261_123 + u64::from(steps),
            4,
            square(),
            warmed(10, steps),
            &law,
            true,
        );
        let loose = learned(
            &mut server,
            &stepped_args(&law, "wrapped", &all_warmed, steps, ""),
        );
        under_determined(&loose, &law, &all_warmed, steps);
        for count in [1, 4, 8] {
            let arguments = stepped_args(&law, "wrapped", &pairs[..count], steps, "");
            let got = learned(&mut server, &arguments);
            if count == 8 {
                complete_and_exact(&got, &law, steps);
            } else {
                found_first(&got, &law, steps);
                for code in codes_of(&got, &law) {
                    assert!(reproduces(&law.with(code), &pairs[..count], true, steps));
                }
            }
            assert_eq!(
                int_at(got.get("examples").unwrap(), "cells"),
                1024 * count as i64
            );
            let again = learned(&mut server, &arguments);
            assert_eq!(written(&got), written(&again), "twice in one process");
            let mut restarted = Server::new(host.as_ref());
            let three = learned(&mut restarted, &arguments);
            assert_eq!(written(&got), written(&three), "after a restart");
            if count == 8 {
                iterated_on_device(host.as_ref(), &got, &pairs, steps);
            }
        }
    }
}

#[test]
fn k2_high_life_and_a_von_neumann_rule_are_found_exactly() {
    let host = device("law_steps k2");
    let mut server = Server::new(None);
    let high = high_life();
    let pairs = pairs_of(7_002, 4, square(), as_rt1(2, 4), &high, true);
    let got = learned(&mut server, &stepped_args(&high, "wrapped", &pairs, 2, ""));
    complete_and_exact(&got, &high, 2);
    iterated_on_device(host.as_ref(), &got, &pairs, 2);
    let vn = vn_rule();
    let pairs = pairs_of(7_003, 4, square(), warmed(4, 3), &vn, true);
    assert!(pairs.iter().all(|one| one.output.contains(&1)));
    let preset = learned(
        &mut server,
        &stepped_args(&vn, "wrapped", &pairs, 3, ",\"places\":\"von_neumann\""),
    );
    complete_and_exact(&preset, &vn, 3);
    assert_eq!(
        shown_of(&preset, "places"),
        "{\"list\":[1,3,4,5,7],\"preset\":\"von_neumann\"}"
    );
    let named = learned(
        &mut server,
        &stepped_args(&vn, "wrapped", &pairs, 3, ",\"place_list\":[7,5,4,3,1]"),
    );
    complete_and_exact(&named, &vn, 3);
    assert_eq!(text_at(&preset, "law"), text_at(&named, "law"));
    iterated_on_device(host.as_ref(), &named, &pairs, 3);
}

#[test]
fn k2_every_step_reads_zero_past_the_edge_of_a_zero_border_example() {
    let mut server = Server::new(None);
    let law = life();
    let pairs = pairs_of(7_004, 4, square(), as_rt1(2, 4), &law, false);
    let wrapped_differs = pairs
        .iter()
        .any(|one| applied(one.grid, &one.input, true, &law, 2) != one.output);
    assert!(wrapped_differs, "the zero border changes these examples");
    let got = learned(&mut server, &stepped_args(&law, "zero", &pairs, 2, ""));
    complete_and_exact(&got, &law, 2);
    assert_eq!(text_at(&got, "border"), "zero");
}

#[test]
fn k2_a_world_example_reads_k_cells_around_its_box_from_its_world() {
    let host = device("law_steps k2 world");
    let Some(host) = host.as_ref() else {
        return;
    };
    let mut server = Server::new(Some(host));
    let law = life();
    let grid = square();
    let mut rng = Rng(7_005);
    let input = soup(&mut rng, grid, 350);
    let output = applied(grid, &input, true, &law, 3);
    let carrier = learned(
        &mut server,
        &stepped_args(
            &law,
            "wrapped",
            &[Pair {
                grid,
                input: input.clone(),
                output: output.clone(),
            }],
            3,
            "",
        ),
    );
    let made = called(
        &mut server,
        "grit_world_create",
        &format!(
            "{{\"wide\":32,\"deep\":32,\"tall\":2,\"law\":{},\"budget_ms\":{LONG}}}",
            written(&Json::Text(text_at(&carrier, "law")))
        ),
    );
    let world = int_at(&made, "world");
    for (plane, cells) in [(0, &input), (1, &output)] {
        let shown: Vec<String> = (0..grid.cells())
            .filter(|at| cells[*at] != 0)
            .map(|at| format!("[{},{},0,1]", at % 32, at / 32))
            .collect();
        called(
            &mut server,
            "grit_world_write",
            &format!(
                "{{\"world\":{world},\"plane\":{plane},\"cells\":[{}],\"budget_ms\":{LONG}}}",
                shown.join(",")
            ),
        );
    }
    let arguments = format!(
        "{{\"stencil\":\"3x3\",\"reads\":[{{\"plane\":0,\"bit\":0}}],\"writes\":[{{\"plane\":1,\"bit\":0}}],\"border\":\"wrapped\",\"rule\":\"outer\",\"examples\":[{{\"world\":{world},\"from\":[5,6,0],\"to\":[26,24,0]}}],\"steps\":3,\"budget_ms\":{LONG}}}"
    );
    let got = learned(&mut server, &arguments);
    found_first(&got, &law, 3);
    assert_eq!(int_at(got.get("examples").unwrap(), "cells"), 22 * 19);
}

#[test]
fn k3_a_tiny_example_leaves_many_tables_listed_in_canonical_order() {
    let mut server = Server::new(None);
    let grid = Grid { wide: 4, deep: 4 };
    let input = vec![0, 1, 0, 0, 0, 0, 1, 0, 1, 1, 1, 0, 0, 0, 0, 0];
    for law in [life(), Law::of(Rule::Totalistic, &MOORE, &[3, 4], &[])] {
        let output = applied(grid, &input, true, &law, 2);
        let pairs = [Pair {
            grid,
            input: input.clone(),
            output,
        }];
        let got = learned(&mut server, &stepped_args(&law, "wrapped", &pairs, 2, ""));
        let (count, codes, _) = exhaustive(&law, &pairs, true, 2);
        assert!(count > 1);
        assert_eq!(int_at(&got, "consistent"), i64::try_from(count).unwrap());
        let shown = codes_of(&got, &law);
        assert_eq!(shown, codes, "the first 16 in ascending number");
        assert_eq!(shown.len(), 16.min(usize::try_from(count).unwrap()));
        assert!(shown.windows(2).all(|two| two[0] < two[1]));
        assert_eq!(shown_of(&got, "tables"), law.with(shown[0]).shown());
        for code in &shown {
            assert!(
                reproduces(&law.with(*code), &pairs, true, 2),
                "table {code}"
            );
        }
        assert_eq!(text_at(&got, "stopped"), "complete");
        assert_eq!(int_at(&got, "checked"), 1 << law.classes());
    }
}

fn edge(grid: Grid, cells: &[u32]) -> Vec<u32> {
    let mut out = vec![0; grid.cells()];
    for y in 0..i64::from(grid.deep) {
        for x in 0..i64::from(grid.wide) {
            let here = grid.at(x, y, true).unwrap();
            let all = [(0, -1), (-1, 0), (1, 0), (0, 1)]
                .iter()
                .all(|(dx, dy)| cells[grid.at(x + dx, y + dy, true).unwrap()] == 1);
            out[here] = u32::from(cells[here] == 1 && !all);
        }
    }
    out
}

#[test]
fn k4_data_from_a_rule_that_does_not_count_is_refused_with_the_closest_table() {
    let mut server = Server::new(None);
    for (law, grid) in [
        (life(), square()),
        (life(), Grid { wide: 8, deep: 8 }),
        (Law::of(Rule::Totalistic, &MOORE, &[], &[]), square()),
    ] {
        let making = Making {
            permille: 500,
            ..warmed(0, 2)
        };
        let pairs = pairs_by(9_001, 2, grid, making, &|cells| edge(grid, cells));
        let told = refused_as(
            &mut server,
            &stepped_args(&law, "wrapped", &pairs, 2, ""),
            "no_rule_k",
            "$.examples",
        );
        let closest = told.get("closest").unwrap();
        let wrong = u64::try_from(int_at(closest, "wrong")).unwrap();
        assert!(wrong > 0);
        assert_eq!(int_at(&told, "cells"), (grid.cells() * 2) as i64);
        assert_eq!(int_at(&told, "checked"), 1 << law.classes());
        assert_eq!(int_at(&told, "steps"), 2);
        let code = code_of(closest.get("tables").unwrap(), &law);
        assert_eq!(
            wrong_of(&law.with(code), &pairs, true, 2),
            wrong,
            "the closest table's wrong cells"
        );
        if law.rule == Rule::Totalistic || grid.wide == 8 {
            let (count, _, nearest) = exhaustive(&law, &pairs, true, 2);
            assert_eq!(count, 0);
            assert_eq!(nearest, (wrong, code), "the first of the fewest wrong");
        }
    }
}

#[test]
fn k5_each_wrong_use_of_steps_is_refused_by_its_code_and_field() {
    let mut server = Server::new(None);
    let law = life();
    let pairs = pairs_of(5, 1, Grid { wide: 8, deep: 8 }, warmed(2, 2), &law, true);
    refused_as(
        &mut server,
        &stepped_args(&law, "wrapped", &pairs, 0, ""),
        "malformed",
        "$.steps",
    );
    let told = refused_as(
        &mut server,
        &stepped_args(&law, "wrapped", &pairs, 9, ""),
        "steps_limit",
        "$.steps",
    );
    assert_eq!(int_at(&told, "limit"), 8);
    for rule in ["minimal", "symmetric"] {
        refused_as(
            &mut server,
            &learn_args(rule, "wrapped", &pairs, ",\"steps\":2"),
            "steps_rule",
            "$.steps",
        );
    }
    refused_as(
        &mut server,
        &stepped_args(&law, "wrapped", &pairs, 2, ",\"noise\":\"vote\""),
        "steps_noise",
        "$.noise",
    );
    let one_write = "\"writes\":[{\"plane\":1,\"bit\":0}]";
    let two_writes = stepped_args(&law, "wrapped", &pairs, 2, "").replace(
        one_write,
        "\"writes\":[{\"plane\":1,\"bit\":0},{\"plane\":1,\"bit\":1}]",
    );
    refused_as(&mut server, &two_writes, "steps_writes", "$.writes");
    let other_bit = stepped_args(&law, "wrapped", &pairs, 2, "")
        .replace(one_write, "\"writes\":[{\"plane\":1,\"bit\":3}]");
    refused_as(&mut server, &other_bit, "steps_writes", "$.writes");
    let cells: Vec<u32> = (0..64).map(|at| u32::from(at % 3 == 0)).collect();
    let cube = |rule: &str, more: &str| {
        format!(
            "{{\"stencil\":\"3x3x3\",\"reads\":[{{\"plane\":0,\"bit\":0}}],\"writes\":[{{\"plane\":1,\"bit\":0}}],\"border\":\"wrapped\",\"rule\":\"{rule}\",\"examples\":[{{\"size\":[4,4,4],\"input\":[{}],\"output\":[{}]}}],\"steps\":2,\"budget_ms\":{LONG}{more}}}",
            listed(&cells),
            listed(&cells)
        )
    };
    for (rule, space, classes) in [("outer", 1_i64 << 54, 54), ("totalistic", 1 << 28, 28)] {
        let told = refused_as(&mut server, &cube(rule, ""), "steps_space", "$.steps");
        assert_eq!(int_at(&told, "space"), space);
        assert_eq!(int_at(&told, "classes"), classes);
        assert_eq!(int_at(&told, "limit"), SPACE);
    }
    let fits = learned(&mut server, &cube("outer", ",\"places\":\"von_neumann\""));
    assert_eq!(int_at(&fits, "space"), 1 << 14);
    assert_eq!(text_at(&fits, "stopped"), "complete");
    assert!(int_at(&fits, "consistent") >= 1);
}

struct Base {
    name: &'static str,
    arguments: String,
    error: bool,
}

fn base_requests() -> Vec<Base> {
    let life_one = pairs_of(31, 4, square(), warmed(10, 1), &life(), true);
    let life_two = pairs_of(32, 4, square(), warmed(10, 2), &life(), true);
    let major = pairs_of(33, 4, square(), warmed(0, 1), &majority(), false);
    let vn = pairs_of(34, 4, square(), warmed(3, 1), &vn_rule(), true);
    vec![
        Base {
            name: "life",
            arguments: learn_args("outer", "wrapped", &life_one, ""),
            error: false,
        },
        Base {
            name: "majority",
            arguments: learn_args("totalistic", "zero", &major, ""),
            error: false,
        },
        Base {
            name: "vn",
            arguments: learn_args("outer", "wrapped", &vn, ",\"places\":\"von_neumann\""),
            error: false,
        },
        Base {
            name: "not_counting",
            arguments: learn_args("totalistic", "wrapped", &life_one, ""),
            error: true,
        },
        Base {
            name: "conflict",
            arguments: learn_args("outer", "wrapped", &life_two, ""),
            error: true,
        },
    ]
}

#[test]
fn k6_requests_without_steps_or_with_one_step_answer_the_base_bytes() {
    let pinned: BTreeMap<&str, &str> = BASE_SHA.into_iter().collect();
    let mut server = Server::new(None);
    for one in base_requests() {
        let one_step = one.arguments.replace(
            &format!("\"budget_ms\":{LONG}"),
            &format!("\"budget_ms\":{LONG},\"steps\":1"),
        );
        assert_ne!(one_step, one.arguments);
        for arguments in [&one.arguments, &one_step] {
            let got = heard(&mut server, arguments);
            let told = result(&got, one.error);
            if one.error {
                assert_eq!(text_at(&told, "refused"), one.name);
            }
            assert_eq!(
                sha_of(&got),
                pinned[one.name],
                "{} as on the base",
                one.name
            );
        }
    }
}

#[cfg(feature = "faults")]
mod braked {
    use super::*;

    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;

    use grit::clock::{Fake, Time};
    use grit::serve3::Options;
    use grit::serve3::law_learn::Point;
    use grit::serve3::law_learn::faults::{cancel_after, deadline_passed_at, disarm, reached};

    const LIFE_BATCH: u64 = ((1 << 3) | (1 << 11) | (1 << 12)) / 64;

    fn timed() -> (Server<'static>, Arc<Fake>) {
        let fake = Arc::new(Fake::new());
        let time: Arc<dyn Time> = fake.clone();
        (
            Server::timed(None, &Options::default(), time).unwrap(),
            fake,
        )
    }

    fn arguments() -> String {
        let law = life();
        let pairs = pairs_of(20_261_125, 4, square(), as_rt1(2, 4), &law, true);
        stepped_args(&law, "wrapped", &pairs, 2, "")
    }

    fn whole() -> Json {
        let (mut server, fake) = timed();
        disarm();
        let got = learned(&mut server, &arguments());
        let passed = reached(Point::Search);
        disarm();
        assert_eq!(fake.elapsed(), Duration::ZERO, "the search read no clock");
        assert_eq!(passed, 4096, "one brake before each 64 tables");
        got
    }

    fn stopped_at(ordinal: u64) -> Json {
        let (mut server, fake) = timed();
        disarm();
        deadline_passed_at(Point::Search, ordinal, Arc::clone(&fake), LONG + 1);
        let got = heard(&mut server, &arguments());
        disarm();
        assert!(fake.elapsed() > Duration::from_millis(LONG));
        got
    }

    #[test]
    fn k6_a_budget_stop_answers_what_it_found_and_refuses_when_it_found_none() {
        let all = whole();
        complete_and_exact(&all, &life(), 2);
        let again = whole();
        assert_eq!(
            written(&all),
            written(&again),
            "a complete search is the same bytes"
        );
        let late = result(&stopped_at(LIFE_BATCH + 1), false);
        assert_eq!(text_at(&late, "stopped"), "budget");
        assert_eq!(late.get("complete"), Some(&Json::Bool(false)));
        assert_eq!(int_at(&late, "consistent"), 1);
        assert_eq!(
            int_at(&late, "checked"),
            i64::try_from((LIFE_BATCH + 1) * 64).unwrap()
        );
        assert_eq!(shown_of(&late, "tables"), life().shown());
        assert_eq!(text_at(&late, "law"), text_at(&all, "law"));
        assert!(late.get("hint").is_some());
        let early = result(&stopped_at(LIFE_BATCH), true);
        assert_eq!(
            (text_at(&early, "refused"), text_at(&early, "field")),
            ("budget".to_owned(), "$.budget_ms".to_owned())
        );
        assert_eq!(
            int_at(&early, "checked"),
            i64::try_from(LIFE_BATCH * 64).unwrap()
        );
        let none = result(&stopped_at(0), true);
        assert_eq!(int_at(&none, "checked"), 0);
    }

    #[test]
    fn k6_a_cancelled_search_answers_cancelled_with_what_it_found() {
        let (mut server, _) = timed();
        let flag = Arc::new(AtomicBool::new(false));
        server.arriving(None, Some(Arc::clone(&flag)));
        disarm();
        cancel_after(LIFE_BATCH + 3);
        let got = heard(&mut server, &arguments());
        disarm();
        let held = result(&got, false);
        assert_eq!(text_at(&held, "stopped"), "cancelled");
        assert_eq!(held.get("complete"), Some(&Json::Bool(false)));
        assert_eq!(int_at(&held, "consistent"), 1);
        assert_eq!(
            int_at(&held, "checked"),
            i64::try_from((LIFE_BATCH + 3) * 64).unwrap()
        );
    }
}
