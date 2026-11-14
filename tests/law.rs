#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use std::collections::BTreeSet;

use common::device;

use grit::compact::Plan;
use grit::law::{self, Code, Law, Refusal, limits};
use grit::layout::{Dimension, PARAMS, PIXELS, STATE};
use grit::live;
use grit::rule3::code::Code as CodeRule;
use grit::rule3::heat::{self, Heat, TUNED};
use grit::rule3::nature::Nature;
use grit::rule3::sand::Sand;
use grit::rule3::{CORNERS, Declared, FIRST_PARAM, Oct, OneStep, Rule};
use grit::sand::{self as sand2, Grid};
use grit::sand3::{self, Grid3, reference};
use grit::sweep;
use grit::wire::{self, Json};
use grit::{Groups, Host, Resident};

const SAND: &str = include_str!("law/sand.law.json");

const HEAT: &str = include_str!("law/heat.law.json");

const SAND_HASH: &str = "494773ea9940abbadfc496844a8e7cda99466041e7a02459a279dbc77415509a";

const HEAT_HASH: &str = "93086d203206c5fd3bb185f610335a7cf7787dd1b33c634121044fb8f03a7011";

const RANDOM_OCTS: u32 = 100_000;

const TRACED_OCTS: u32 = 200_000;

const WIDTHS: [u32; 5] = [4, 8, 16, 32, 64];

const HEAT_PARAMS: [[u32; 3]; 2] = [TUNED, [300, 16, 70_000]];

fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut mixed = *state;
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    mixed ^ (mixed >> 31)
}

fn word(state: &mut u64) -> u32 {
    (next(state) >> 16) as u32
}

fn sand_cell(state: &mut u64) -> u32 {
    let roll = next(state);
    let kind = match roll % 20 {
        0..8 => sand2::AIR,
        8..12 => sand2::WATER,
        12..17 => sand2::SAND,
        _ => sand2::WALL,
    };
    let bits = ((roll >> 8) % 128) as u32;
    kind + (bits << 2)
}

fn heat_cell(state: &mut u64) -> u32 {
    let roll = next(state);
    let kind = match roll % 32 {
        0..10 => heat::AIR,
        10..12 => heat::WALL,
        12..15 => heat::ICE,
        15..19 => heat::WATER,
        19..22 => heat::STEAM,
        22..26 => heat::FUEL,
        26..28 => heat::FIRE,
        28..31 => heat::ASH,
        _ => heat::KINDS + ((roll >> 40) % 8) as u32,
    };
    let temperature = ((roll >> 8) % 1200) as u32;
    let life = ((roll >> 20) % 16) as u32;
    let kept = ((roll >> 24) & 0xFF) as u32;
    heat::cell(kind, temperature) | (life << 4) | (kept << 8)
}

#[derive(Clone, Copy, Debug)]
struct Case {
    words: [u32; CORNERS],
    frame: u32,
    at: [u32; 3],
}

fn ruled<R: OneStep>(rule: &R, case: Case, params: &[u32]) -> (Vec<u32>, Vec<u32>) {
    let made = Rule::step(
        rule,
        Oct {
            words: case.words,
            frame: case.frame,
            at: case.at,
            params,
        },
    );
    (made.out.to_vec(), made.counts)
}

fn random_cases(cell: fn(&mut u64) -> u32, masks: &[u32], many: u32, seed: u64) -> Vec<Case> {
    let mut state = seed;
    (0..many)
        .map(|at| {
            let mask = masks[at as usize % masks.len()];
            let structured = next(&mut state).is_multiple_of(2);
            let words = core::array::from_fn(|_| {
                if structured {
                    cell(&mut state)
                } else {
                    word(&mut state) & mask
                }
            });
            Case {
                words,
                frame: word(&mut state),
                at: [word(&mut state), word(&mut state), word(&mut state)],
            }
        })
        .collect()
}

fn literals(json: &Json, out: &mut BTreeSet<u32>) {
    match json {
        Json::Int(held) => {
            if let Ok(held) = u32::try_from(*held) {
                out.insert(held);
            }
        }
        Json::List(items) => items.iter().for_each(|one| literals(one, out)),
        Json::Map(fields) => fields.iter().for_each(|(_, one)| literals(one, out)),
        _ => {}
    }
}

fn constants(text: &str, params: &[u32]) -> Vec<u32> {
    let json = wire::read(text).unwrap();
    let mut held = BTreeSet::new();
    literals(json.get("consts").unwrap(), &mut held);
    literals(json.get("defs").unwrap(), &mut held);
    literals(json.get("step").unwrap(), &mut held);
    held.extend(params.iter().copied());
    held.into_iter().collect()
}

fn boundary_cases(cell: fn(&mut u64) -> u32, constants: &[u32], seed: u64) -> Vec<Case> {
    let mut state = seed;
    let mut cases = Vec::new();
    for &constant in constants {
        for moved in [constant.wrapping_sub(1), constant, constant.wrapping_add(1)] {
            let width = 32 - moved.leading_zeros().min(31);
            for shift in 0..32_u32 {
                let field = if width >= 32 {
                    u32::MAX
                } else {
                    ((1_u32 << width) - 1).wrapping_shl(shift)
                };
                let value = moved.wrapping_shl(shift);
                for _ in 0..4 {
                    let words = core::array::from_fn(|_| {
                        let base = cell(&mut state);
                        if next(&mut state).is_multiple_of(2) {
                            (base & !field) | value
                        } else {
                            base
                        }
                    });
                    cases.push(Case {
                        words,
                        frame: word(&mut state),
                        at: [word(&mut state), word(&mut state), word(&mut state)],
                    });
                }
            }
        }
    }
    cases
}

fn sand_cases(many: u32) -> Vec<Case> {
    let mut cases = random_cases(
        sand_cell,
        &[0x1FF, 0x3, 0x1E3, 0x23, 0x3F, u32::MAX],
        many,
        0x5A4D,
    );
    cases.extend(boundary_cases(sand_cell, &constants(SAND, &[]), 0xB0B));
    cases
}

fn heat_cases(many: u32, params: &[u32]) -> Vec<Case> {
    let mut cases = random_cases(
        heat_cell,
        &[0xFFFF_F00F, 0x03FF_0007, 0x0280_00F7, u32::MAX, 0x0007_0005],
        many,
        0x4EA7,
    );
    cases.extend(boundary_cases(heat_cell, &constants(HEAT, params), 0xF1E));
    cases
}

fn differing<A: OneStep, B: OneStep>(one: &A, other: &B, params: &[u32], cases: &[Case]) -> usize {
    cases
        .iter()
        .filter(|&&case| ruled(one, case, params) != ruled(other, case, params))
        .count()
}

fn compiled(text: &str) -> Law {
    Law::compile(text).unwrap_or_else(|why| panic!("{why}"))
}

#[test]
fn sand_law_equals_the_rust_rule() {
    let law = compiled(SAND);
    let cases = sand_cases(RANDOM_OCTS);
    let differ = differing(&law, &Sand, &[], &cases);
    println!(
        "sand law: {} nodes, work {}, depth {}; {differ} of {} octs differ from Sand",
        law.nodes(),
        law.work(),
        law.depth(),
        cases.len()
    );
    assert_eq!(differ, 0);
    assert_eq!(law.params(), Sand.params());
    assert_eq!(law.tally(), Sand.tally());
}

#[test]
fn heat_law_equals_the_rust_rule() {
    let law = compiled(HEAT);
    for params in HEAT_PARAMS {
        let mut sets = vec![params];
        for which in 0..3 {
            for moved in [params[which].wrapping_sub(1), params[which].wrapping_add(1)] {
                let mut set = params;
                set[which] = moved;
                sets.push(set);
            }
        }
        for set in sets {
            let cases = heat_cases(RANDOM_OCTS / 4, &set);
            let differ = differing(&law, &Heat, &set, &cases);
            println!(
                "heat law {set:?}: {} nodes, work {}; {differ} of {} octs differ from Heat",
                law.nodes(),
                law.work(),
                cases.len()
            );
            assert_eq!(differ, 0, "params {set:?}");
        }
    }
    assert_eq!(law.params(), Heat.params());
    assert_eq!(law.tally(), Heat.tally());
}

fn sites(json: &Json, path: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
    match json {
        Json::Int(_) => out.push(path.clone()),
        Json::List(items) => {
            for (at, one) in items.iter().enumerate() {
                path.push(at);
                sites(one, path, out);
                path.pop();
            }
        }
        Json::Map(fields) => {
            for (at, (_, one)) in fields.iter().enumerate() {
                path.push(at);
                sites(one, path, out);
                path.pop();
            }
        }
        _ => {}
    }
}

fn site_mut<'a>(json: &'a mut Json, path: &[usize]) -> &'a mut Json {
    path.iter().fold(json, |here, &at| match here {
        Json::List(items) => &mut items[at],
        Json::Map(fields) => &mut fields[at].1,
        _ => panic!("no site"),
    })
}

fn site_name(json: &Json, path: &[usize]) -> String {
    let mut out = String::from("$");
    let mut here = json;
    for &at in path {
        here = match here {
            Json::List(items) => {
                out.push_str(&format!("[{at}]"));
                &items[at]
            }
            Json::Map(fields) => {
                out.push('.');
                out.push_str(&fields[at].0);
                &fields[at].1
            }
            _ => panic!("no site"),
        };
    }
    out
}

fn survivors<R: OneStep>(text: &str, rule: &R, sets: &[(Vec<u32>, Vec<Case>)]) -> Vec<String> {
    let json = wire::read(text).unwrap();
    let mut all = Vec::new();
    sites(&json, &mut Vec::new(), &mut all);
    let skipped = ["law", "params", "channels", "halo", "tally"];
    let wanted: Vec<Vec<_>> = sets
        .iter()
        .map(|(params, cases)| {
            cases
                .iter()
                .map(|&case| ruled(rule, case, params))
                .collect()
        })
        .collect();
    let mut left = Vec::new();
    let mut tried = 0;
    for site in all {
        let name = site_name(&json, &site);
        if skipped
            .iter()
            .any(|key| name == format!("$.{key}") || name.starts_with(&format!("$.{key}.")))
        {
            continue;
        }
        let Json::Int(held) = *site_mut(&mut json.clone(), &site) else {
            continue;
        };
        for delta in [-1_i64, 1] {
            let moved = held + delta;
            if u32::try_from(moved).is_err() {
                continue;
            }
            let mut mutant = json.clone();
            *site_mut(&mut mutant, &site) = Json::Int(moved);
            let Ok(law) = Law::from_json(&mutant) else {
                continue;
            };
            tried += 1;
            let caught = sets.iter().zip(&wanted).any(|((params, cases), wanted)| {
                cases
                    .iter()
                    .zip(wanted)
                    .any(|(&case, want)| &ruled(&law, case, params) != want)
            });
            if !caught {
                left.push(format!("{name} {held}->{moved}"));
            }
        }
    }
    println!(
        "{tried} compiling mutants, {} not caught: {left:#?}",
        left.len()
    );
    left
}

const SAND_EQUIVALENT: [&str; 1] = ["$.defs.x_first[1][1][2][1][5][4][2][1] 8->7"];

const HEAT_EQUIVALENT: [&str; 4] = [
    "$.consts.STILL 3->4",
    "$.defs.reacted[1][1][11][1][1][1][2] 240->241",
    "$.defs.diffused[1][1][1][1][2][2] 0->1",
    "$.defs.diffused[1][1][1][1][2][3] 4294967295->4294967294",
];

#[test]
fn every_constant_moved_by_one_in_the_sand_law_is_caught() {
    let sets = [(Vec::new(), sand_cases(RANDOM_OCTS / 5))];
    let left = survivors(SAND, &Sand, &sets);
    assert_eq!(left, SAND_EQUIVALENT);
}

#[test]
fn every_constant_moved_by_one_in_the_heat_law_is_caught() {
    let sets: Vec<(Vec<u32>, Vec<Case>)> = [TUNED, [600, 3, 4096], [300, 16, 70_000]]
        .into_iter()
        .map(|params| (params.to_vec(), heat_cases(RANDOM_OCTS / 10, &params)))
        .collect();
    let left = survivors(HEAT, &Heat, &sets);
    assert_eq!(left, HEAT_EQUIVALENT);
}

#[test]
fn the_canonical_form_and_hash_of_the_shipped_laws_are_pinned() {
    let sand = compiled(SAND);
    let heat = compiled(HEAT);
    assert_eq!(sand.hex(), SAND_HASH);
    assert_eq!(heat.hex(), HEAT_HASH);
    assert_eq!(
        law::hex(&grit::branch::sha256(sand.canonical().as_bytes())),
        SAND_HASH
    );
    assert!(!sand.canonical().contains(' '));
    assert!(!sand.canonical().contains('\n'));
    let mut json = wire::read(SAND).unwrap();
    if let Json::Map(fields) = &mut json {
        fields.reverse();
    }
    let spaced = wire::written(&json)
        .replace(',', " ,\n\t")
        .replace(':', " : ");
    assert_eq!(
        refusal(&spaced.replace("\"law\" : 1", "\"law\" : 0001")).code,
        Code::Json
    );
    assert_ne!(spaced, SAND);
    assert_eq!(compiled(&spaced).hex(), SAND_HASH);
    assert_eq!(compiled(&spaced).canonical(), sand.canonical());
}

fn traced_equals<R: OneStep>(name: &str, rule: &R, params: &[u32], masks: &[u32]) {
    let dag = law::trace(rule);
    let mut state = 0x1234_5678_9ABC_DEF1_u64;
    let mut differ = 0;
    for at in 0..TRACED_OCTS {
        let mask = masks[at as usize % masks.len()];
        let case = Case {
            words: core::array::from_fn(|_| word(&mut state) & mask),
            frame: word(&mut state),
            at: [word(&mut state), word(&mut state), word(&mut state)],
        };
        if ruled(rule, case, params) != ruled(&dag, case, params) {
            differ += 1;
        }
    }
    println!(
        "{name}: {} nodes; {differ} of {TRACED_OCTS} octs differ",
        dag.len()
    );
    assert_eq!(differ, 0, "{name}");
    let grid = Grid::solid(32, 32, 32).unwrap();
    for width in WIDTHS {
        let one = sand3::settle_by(rule, width, grid, u32::MAX).unwrap().words;
        let other = sand3::settle_by(&dag, width, grid, u32::MAX).unwrap().words;
        println!(
            "{name} width {width}: {} words, identical {}",
            one.len(),
            one == other
        );
        assert_eq!(one, other, "{name} width {width}");
    }
}

#[test]
fn the_trace_of_sand_replays_with_identical_kernels() {
    traced_equals("sand", &Sand, &[], &[0x1FF, 0x3, u32::MAX]);
}

#[test]
fn the_trace_of_heat_replays_with_identical_kernels() {
    traced_equals("heat", &Heat, &TUNED, &[0xFFFF_F00F, 0x0FFF_F007, u32::MAX]);
}

#[test]
fn the_trace_of_code_replays_with_identical_kernels() {
    traced_equals(
        "code",
        &CodeRule,
        &[],
        &[u32::MAX, 0x7F00_00FF, 0x0700_00FF],
    );
}

#[test]
fn the_trace_of_nature_replays_with_identical_kernels() {
    traced_equals(
        "nature",
        &Nature,
        &[3],
        &[0xFFFF_0FFF, 0x0FFF_0F0F, u32::MAX, 0x0000_000F],
    );
}

#[test]
fn the_law_kernels_are_emitted_at_every_width() {
    let sand = compiled(SAND);
    let heat = compiled(HEAT);
    let grid = Grid::solid(32, 32, 32).unwrap();
    for width in WIDTHS {
        for (name, built) in [
            ("sand", sand3::settle_by(&sand, width, grid, u32::MAX)),
            ("heat", sand3::settle_by(&heat, width, grid, u32::MAX)),
        ] {
            let built = built.unwrap_or_else(|why| panic!("{name} width {width}: {why:?}"));
            assert!(!built.words.is_empty());
        }
    }
}

fn template(step: &str) -> String {
    format!(
        "{{\"law\":1,\"name\":\"t\",\"params\":0,\"channels\":1,\"halo\":0,\"tally\":{{\"counts\":1,\"weighted\":false}},\"consts\":{{}},\"defs\":{{}},\"step\":{step}}}"
    )
}

fn edited(text: &str, edit: impl FnOnce(&mut Json)) -> String {
    let mut json = wire::read(text).unwrap();
    edit(&mut json);
    wire::written(&json)
}

fn set(json: &mut Json, key: &str, value: Json) {
    let Json::Map(fields) = json else {
        panic!("not an object");
    };
    match fields.iter_mut().find(|(name, _)| name == key) {
        Some((_, slot)) => *slot = value,
        None => fields.push((key.to_owned(), value)),
    }
}

fn field<'a>(json: &'a mut Json, key: &str) -> &'a mut Json {
    let Json::Map(fields) = json else {
        panic!("not an object");
    };
    &mut fields.iter_mut().find(|(name, _)| name == key).unwrap().1
}

fn refusal(text: &str) -> Refusal {
    match Law::compile(text) {
        Ok(law) => panic!("compiled {} into {} nodes", law.name(), law.nodes()),
        Err(why) => why,
    }
}

fn refused_at(text: &str, code: Code, path: &str) -> Refusal {
    let why = refusal(text);
    assert_eq!((why.code, why.path.as_str()), (code, path), "{why}");
    why
}

#[test]
fn a_refusal_names_the_path_the_code_what_was_expected_and_what_was_found() {
    refused_at("{\"law\":1,", Code::Json, "$");
    refused_at(&format!("{SAND} x"), Code::Json, "$");
    refused_at(&format!("{SAND}}}"), Code::Json, "$");
    refused_at("[1]", Code::Shape, "$");
    let missing = edited(SAND, |json| {
        if let Json::Map(fields) = json {
            fields.retain(|(name, _)| name != "halo");
        }
    });
    refused_at(&missing, Code::Missing, "$.halo");
    let unknown = edited(SAND, |json| set(json, "frames", Json::Int(100)));
    let why = refused_at(&unknown, Code::UnknownKey, "$.frames");
    assert_eq!(why.found, "the key \"frames\"");
    let duplicated = SAND.replacen("\"name\"", "\"name\": \"heat\", \"name\"", 1);
    let why = refused_at(&duplicated, Code::DuplicateKey, "$.name");
    assert_eq!(why.found, "the key \"name\" again");
    let version = edited(SAND, |json| set(json, "law", Json::Int(2)));
    refused_at(&version, Code::Version, "$.law");
    let negative = edited(SAND, |json| {
        set(field(json, "consts"), "AIR", Json::Int(-1))
    });
    let why = refused_at(&negative, Code::Literal, "$.consts.AIR");
    assert_eq!(why.found, "-1");
    let reserved = edited(SAND, |json| {
        set(field(json, "consts"), "frame", Json::Int(1))
    });
    refused_at(&reserved, Code::Reserved, "$.consts.frame");
    let op_name = edited(SAND, |json| {
        let body = field(field(json, "defs"), "mat").clone();
        set(field(json, "defs"), "add", body);
    });
    refused_at(&op_name, Code::Reserved, "$.defs.add");
    let mat = |body: &str| {
        edited(SAND, |json| {
            let value = wire::read(&format!("[[\"w\"],{body}]")).unwrap();
            set(field(json, "defs"), "mat", value);
        })
    };
    let why = refused_at(
        &mat("[\"and\",\"w\",\"WHAT\"]"),
        Code::UnknownName,
        "$.defs.mat[1][2]",
    );
    assert_eq!(why.found, "the name \"WHAT\"");
    refused_at(
        &mat("[\"andd\",\"w\",3]"),
        Code::UnknownOp,
        "$.defs.mat[1][0]",
    );
    let why = refused_at(&mat("[\"and\",\"w\"]"), Code::Arity, "$.defs.mat[1]");
    assert_eq!(why.expected, "and with 2 arguments");
    assert_eq!(why.found, "1 argument");
    let why = refused_at(&mat("[\"at\",\"w\",0]"), Code::Type, "$.defs.mat[1][1]");
    assert_eq!(why.expected, "a vec");
    assert_eq!(why.found, "a word");
    refused_at(
        &mat("[\"and\",\"w\",[\"eq\",\"w\",1]]"),
        Code::Type,
        "$.defs.mat[1][2]",
    );
    let why = refused_at(
        &mat("[\"and\",\"w\",[\"look\",0,-1,0,0,0]]"),
        Code::HaloDeclared,
        "$.halo",
    );
    assert_eq!(why.found, "halo 0, and a look at $.defs.mat[1][2]");
    refused_at(
        &mat("[\"and\",\"w\",null]"),
        Code::Shape,
        "$.defs.mat[1][2]",
    );
    refused_at(
        &mat("[\"and\",\"w\",4294967296]"),
        Code::Literal,
        "$.defs.mat[1][2]",
    );
    refused_at(
        &mat("[\"shl\",\"w\",\"w\"]"),
        Code::Type,
        "$.defs.mat[1][2]",
    );
    refused_at(&mat("[\"mat\",\"w\"]"), Code::Recursion, "$.defs.mat[1]");
    let why = refused_at(
        &template("[\"vec\",\"cells\",[\"vec\",[\"at\",\"cells\",8]]]"),
        Code::Index,
        "$.step[2][1][2]",
    );
    assert_eq!(why.limit, Some(8));
    assert_eq!(why.expected, "an index below 8");
    refused_at(
        &template("[\"vec\",\"cells\",[\"vec\",[\"param\",0]]]"),
        Code::Index,
        "$.step[2][1][1]",
    );
    refused_at(
        &template("[\"vec\",\"cells\",[\"vec\",0,1]]"),
        Code::Result,
        "$.step",
    );
    refused_at(
        &template("[\"vec\",[\"rev\",[\"range\",7]],[\"vec\",0]]"),
        Code::Result,
        "$.step",
    );
    refused_at(
        &template("[\"vec\",\"cells\",[\"vec\",[\"eq\",\"frame\",1]]]"),
        Code::Result,
        "$.step",
    );
    let looped = template("[\"vec\",\"cells\",[\"vec\",[\"a\",0]]]").replace(
        "\"defs\":{}",
        "\"defs\":{\"a\":[[\"x\"],[\"b\",\"x\"]],\"b\":[[\"y\"],[\"a\",\"y\"]]}",
    );
    let why = refused_at(&looped, Code::Recursion, "$.defs.b[1]");
    assert_eq!(why.found, "a -> b -> a");
    refused_at(
        &template("[\"vec\",\"cells\",[\"vec\",[]]]"),
        Code::Shape,
        "$.step[2][1]",
    );
    refused_at(
        &template("[\"vec\",\"cells\",[\"vec\",[1,2]]]"),
        Code::Shape,
        "$.step[2][1][0]",
    );
    let let_shape = template("[\"let\",[[\"a\"]],[\"vec\",\"cells\",[\"vec\",0]]]");
    refused_at(&let_shape, Code::Shape, "$.step[1][0]");
    let shapes = edited(SAND, |json| set(json, "defs", Json::List(Vec::new())));
    refused_at(&shapes, Code::Shape, "$.defs");
    let quoted = edited(SAND, |json| {
        set(field(json, "consts"), "a b", Json::Text("x".into()))
    });
    refused_at(&quoted, Code::Shape, "$.consts[\"a b\"]");
    for why in [refusal("{\"law\":1,"), refusal(&shapes), refusal(&looped)] {
        let shown = why.to_string();
        assert!(shown.starts_with(why.code.name()), "{shown}");
        assert!(shown.contains(&why.path), "{shown}");
        let json = wire::written(&why.json());
        assert!(
            json.contains(&format!("\"code\":\"{}\"", why.code.name())),
            "{json}"
        );
    }
}

fn sized(text: &str, bytes: usize) -> String {
    let mut out = text.trim_end().to_owned();
    while out.len() < bytes {
        out.push(' ');
    }
    out
}

#[test]
fn a_law_at_the_body_limit_compiles_and_one_byte_more_is_refused() {
    let at = sized(SAND, limits::BODY);
    assert_eq!(at.len(), limits::BODY);
    assert_eq!(compiled(&at).hex(), SAND_HASH);
    let past = sized(SAND, limits::BODY + 1);
    let why = refused_at(&past, Code::Size, "$");
    assert_eq!(why.limit, Some(limits::BODY as u64));
}

#[test]
fn params_counts_channels_and_halo_are_admitted_at_their_limit_and_refused_past_it() {
    let with = |key: &str, value: i64| edited(SAND, |json| set(json, key, Json::Int(value)));
    assert_eq!(compiled(&with("params", 32)).params(), 32);
    refused_at(&with("params", 33), Code::Params, "$.params");
    assert_eq!(compiled(&with("halo", 0)).halo(), 0);
    refused_at(&with("halo", 1), Code::HaloDeclared, "$.halo");
    refused_at(&with("halo", 2), Code::Halo, "$.halo");
    let why = refused_at(&with("channels", 5), Code::Channels, "$.channels");
    assert_eq!(why.limit, Some(4));
    refused_at(&with("channels", 0), Code::Channels, "$.channels");
    let planed = edited(&template("[\"vec\",\"cells\",[\"vec\",0]]"), |json| {
        set(json, "channels", Json::Int(4));
    });
    assert_eq!(compiled(&planed).channels(), 4);
    let counted = |counts: i64| {
        let step = format!("[\"vec\",\"cells\",[\"rev\",[\"range\",{counts}]]]");
        template(&step).replace("\"counts\":1", &format!("\"counts\":{counts}"))
    };
    assert_eq!(compiled(&counted(8)).tally().counts, 8);
    refused_at(&counted(9), Code::Counts, "$.tally.counts");
    refused_at(&counted(0), Code::Counts, "$.tally.counts");
}

fn flipped(times: usize) -> String {
    template(&format!(
        "[\"vec\",[\"put\",\"cells\",0,[\"fold\",\"a\",[\"at\",\"cells\",0],\"i\",[\"range\",{times}],[\"not\",\"a\"]]],[\"vec\",0]]"
    ))
}

#[test]
fn a_law_of_the_most_nodes_compiles_and_one_node_more_is_refused() {
    let base = compiled(&flipped(10)).nodes() - 10;
    let most = limits::NODES - base;
    assert_eq!(compiled(&flipped(most)).nodes(), limits::NODES);
    let why = refused_at(&flipped(most + 1), Code::Nodes, "$.step");
    assert_eq!(why.limit, Some(limits::NODES as u64));
}

fn worked(items: u64, pad: u64) -> String {
    let bindings: Vec<String> = (0..pad).map(|at| format!("[\"p{at}\",0]")).collect();
    template(&format!(
        "[\"let\",[{}],[\"vec\",\"cells\",[\"vec\",[\"fold\",\"a\",0,\"i\",[\"range\",{items}],\"a\"]]]]",
        bindings.join(",")
    ))
}

#[test]
fn a_law_of_the_most_work_compiles_and_one_visit_more_is_refused() {
    let fixed = compiled(&worked(0, 0)).work();
    let per = compiled(&worked(1, 0)).work() - fixed;
    assert_eq!(compiled(&worked(0, 1)).work(), fixed + 1);
    let items = (limits::WORK - fixed) / per;
    let pad = (limits::WORK - fixed) % per;
    assert_eq!(compiled(&worked(items, pad)).work(), limits::WORK);
    let why = refused_at(&worked(items, pad + 1), Code::Work, "$.step[2]");
    assert_eq!(why.limit, Some(limits::WORK));
    refused_at(
        &template("[\"vec\",\"cells\",[\"vec\",[\"count\",[\"range\",4294967295]]]]"),
        Code::Work,
        "$.step[2][1][1]",
    );
}

fn chained(defs: usize) -> String {
    let mut chain: Vec<String> = (0..defs)
        .map(|at| format!("\"d{at}\":[[\"x\"],[\"d{}\",\"x\"]]", at + 1))
        .collect();
    chain.push(format!("\"d{defs}\":[[\"x\"],\"x\"]"));
    template("[\"vec\",\"cells\",[\"vec\",[\"d0\",0]]]")
        .replace("\"defs\":{}", &format!("\"defs\":{{{}}}", chain.join(",")))
}

#[test]
fn a_law_nested_to_the_most_depth_compiles_and_one_level_more_is_refused() {
    let one = compiled(&chained(1)).depth();
    let per = compiled(&chained(2)).depth() - one;
    let defs = (limits::DEPTH - one) / per + 1;
    let at = std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(move || {
            let deepest = Law::compile(&chained(defs)).map(|law| law.depth());
            let past = Law::compile(&chained(defs + 1)).map(|law| law.depth());
            (deepest, past)
        })
        .unwrap()
        .join()
        .unwrap();
    let (deepest, past) = at;
    let deepest = deepest.unwrap();
    assert!(
        deepest <= limits::DEPTH && deepest + per > limits::DEPTH,
        "{deepest}"
    );
    let why = past.unwrap_err();
    assert_eq!(why.code, Code::Depth, "{why}");
    let mut nested = Json::Int(0);
    for _ in 0..=limits::DEPTH {
        nested = Json::List(vec![Json::Text("not".into()), nested]);
    }
    let mut json = wire::read(&template("0")).unwrap();
    set(&mut json, "step", nested);
    let why = Law::from_json(&json).unwrap_err();
    assert_eq!(why.code, Code::Depth, "{why}");
}

fn mutated(json: &mut Json, state: &mut u64, names: &[&str]) {
    let mut all = Vec::new();
    collect(json, &mut Vec::new(), &mut all);
    let site = all[(next(state) % all.len() as u64) as usize].clone();
    let current = walk(json, &site).clone();
    let pick = |state: &mut u64| names[(next(state) % names.len() as u64) as usize].to_owned();
    let replacement = match next(state) % 12 {
        0 => Json::Int(
            [
                0,
                1,
                -1,
                i64::from(u32::MAX),
                i64::from(u32::MAX) + 1,
                i64::MAX,
                i64::MIN,
                7,
                8,
                32,
            ][(next(state) % 10) as usize],
        ),
        1 => Json::Text(pick(state)),
        2 => Json::Null,
        3 => Json::Bool(next(state).is_multiple_of(2)),
        4 => Json::List(Vec::new()),
        5 => Json::Map(Vec::new()),
        6 => Json::Text("x".repeat((next(state) % 5000) as usize)),
        7 => match current {
            Json::List(mut items) if !items.is_empty() => {
                let at = (next(state) % items.len() as u64) as usize;
                items.remove(at);
                Json::List(items)
            }
            other => other,
        },
        8 => match current {
            Json::List(mut items) if !items.is_empty() => {
                let at = (next(state) % items.len() as u64) as usize;
                let copy = items[at].clone();
                items.insert(at, copy);
                Json::List(items)
            }
            other => other,
        },
        9 => Json::List(vec![
            Json::Text(pick(state)),
            current,
            Json::Int((next(state) % 40) as i64),
        ]),
        10 => Json::Int(i64::from(word(state))),
        _ => walk(json, &all[(next(state) % all.len() as u64) as usize]).clone(),
    };
    *walk_mut(json, &site) = replacement;
}

fn collect(json: &Json, path: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
    out.push(path.clone());
    match json {
        Json::List(items) => {
            for (at, one) in items.iter().enumerate() {
                path.push(at);
                collect(one, path, out);
                path.pop();
            }
        }
        Json::Map(fields) => {
            for (at, (_, one)) in fields.iter().enumerate() {
                path.push(at);
                collect(one, path, out);
                path.pop();
            }
        }
        _ => {}
    }
}

fn walk<'a>(json: &'a Json, path: &[usize]) -> &'a Json {
    path.iter().fold(json, |here, &at| match here {
        Json::List(items) => &items[at],
        Json::Map(fields) => &fields[at].1,
        _ => here,
    })
}

fn walk_mut<'a>(json: &'a mut Json, path: &[usize]) -> &'a mut Json {
    path.iter().fold(json, |here, &at| match here {
        Json::List(items) => &mut items[at],
        Json::Map(fields) => &mut fields[at].1,
        other => other,
    })
}

const FUZZ: u32 = 100_000;

#[test]
fn compile_is_total_on_a_seeded_fuzz_of_a_hundred_thousand_inputs() {
    let names = [
        "let", "if", "vec", "range", "at", "put", "map", "fold", "filter", "rev", "concat",
        "param", "noise", "look", "add", "sub", "mul", "div", "and", "or", "xor", "not", "shl",
        "shr", "dot_s", "dot_u", "eq", "ne", "lt", "le", "gt", "ge", "sel", "both", "either",
        "denied", "word", "count", "cells", "frame", "at", "X", "Y", "Z", "w", "c", "k", "fall",
        "x_first", "reacted", "WATER", "TOP", "nope",
    ];
    let seeds = [wire::read(SAND).unwrap(), wire::read(HEAT).unwrap()];
    let mut state = 0xF022_u64;
    let mut codes = std::collections::BTreeMap::new();
    let mut panicked = 0;
    let mut compiled_laws = 0;
    for case in 0..FUZZ {
        let text = match case % 10 {
            0 => {
                let bytes: Vec<u8> = (0..next(&mut state) % 300)
                    .map(|_| (next(&mut state) & 0xFF) as u8)
                    .collect();
                String::from_utf8_lossy(&bytes).into_owned()
            }
            1 => {
                let tokens = [
                    "{",
                    "}",
                    "[",
                    "]",
                    ",",
                    ":",
                    "\"law\"",
                    "1",
                    "-1",
                    "\"",
                    "null",
                    "1e9",
                    "99999999999999999999",
                    " ",
                    "\\u12",
                    "\"step\"",
                ];
                (0..next(&mut state) % 80)
                    .map(|_| tokens[(next(&mut state) % tokens.len() as u64) as usize])
                    .collect()
            }
            2 => {
                let deep = (next(&mut state) % 80) as usize;
                template(&format!(
                    "{}0{}",
                    "[\"not\",".repeat(deep),
                    "]".repeat(deep)
                ))
            }
            _ => {
                let mut json = seeds[(case % 2) as usize].clone();
                for _ in 0..1 + next(&mut state) % 3 {
                    mutated(&mut json, &mut state, &names);
                }
                wire::written(&json)
            }
        };
        let outcome = std::panic::catch_unwind(|| Law::compile(&text));
        match outcome {
            Err(_) => panicked += 1,
            Ok(Ok(law)) => {
                compiled_laws += 1;
                let made = law.step(Oct {
                    words: [case; CORNERS],
                    frame: case,
                    at: [1, 2, 3],
                    params: &[7; 4],
                });
                assert_eq!(made.counts.len(), law.tally().counts);
            }
            Ok(Err(why)) => {
                assert!(why.path.starts_with('$'), "{why}");
                assert!(Code::ALL.contains(&why.code));
                *codes.entry(why.code.name()).or_insert(0_u32) += 1;
            }
        }
    }
    println!("{FUZZ} inputs: {compiled_laws} laws, refusals {codes:?}, {panicked} panics");
    assert_eq!(panicked, 0);
    assert!(compiled_laws > 0);
    assert!(codes.len() >= 15, "{codes:?}");
}

fn grids(side: u32) -> (Grid, Grid3) {
    (
        Grid::solid(side, side, side).unwrap(),
        Grid3::new(side, side, side).unwrap(),
    )
}

fn whole_kernel<'host, R: Rule>(
    host: &'host Host,
    rule: &R,
    grid: Grid,
) -> (Resident<'host>, Plan, Groups) {
    let width = host.width();
    let plan = Plan::over_in(
        width,
        grid.blocks().div_ceil(sweep::cells(width)),
        Dimension::Three,
    )
    .unwrap();
    let built = sand3::settle_by(rule, width, grid, host.limits().groups).unwrap();
    let tick = live::tick(width).unwrap();
    let sizes = live::sizes3(rule, &plan);
    let modules: Vec<&[u32]> = vec![&tick, &built.words];
    let resident = host.resident(&sizes, &modules).unwrap();
    (resident, plan, built.spread.dispatch())
}

fn stepped_on_device<R: Rule>(
    held: &mut Resident<'_>,
    rule: &R,
    plan: &Plan,
    dispatch: Groups,
    start: &[u32],
    params: &[u32],
    frame: u32,
) -> Vec<u32> {
    let sizes = live::sizes3(rule, plan);
    for (binding, &words) in sizes.iter().enumerate() {
        if binding != PIXELS as usize {
            held.write(binding, &vec![0_u32; words]).unwrap();
        }
    }
    held.write(STATE as usize, start).unwrap();
    if !params.is_empty() {
        held.write_at(PARAMS as usize, FIRST_PARAM, params).unwrap();
    }
    held.write(PARAMS as usize, &[frame.wrapping_sub(1), plan.dump()])
        .unwrap();
    let whole = [Groups::Fixed(1), dispatch];
    let stages: Vec<&[Groups]> = whole.iter().map(core::slice::from_ref).collect();
    let (_, out) = held
        .staged_reading(&stages, STATE as usize, start.len())
        .unwrap();
    out
}

fn on_device<A: OneStep, B: OneStep>(
    host: &Host,
    name: &str,
    law: &A,
    rule: &B,
    params: &[u32],
    cell: fn(&mut u64) -> u32,
) {
    let (grid, reference_grid) = grids(32);
    let (mut held, plan, dispatch) = whole_kernel(host, law, grid);
    let mut state = 0xD1CE_u64;
    let mut octs = 0;
    let mut frame = 0_u32;
    while octs < RANDOM_OCTS {
        let start: Vec<u32> = (0..reference_grid.cells())
            .map(|_| cell(&mut state))
            .collect();
        let got = stepped_on_device(&mut held, law, &plan, dispatch, &start, params, frame);
        let replayed = reference(law, reference_grid, frame, &start, params);
        let wanted = reference(rule, reference_grid, frame, &start, params);
        assert_eq!(
            replayed, wanted,
            "{name} frame {frame}: CPU replay against the rule"
        );
        assert_ne!(replayed, start, "{name} frame {frame}: nothing moved");
        let differ = got
            .iter()
            .zip(&replayed)
            .filter(|(one, other)| one != other)
            .count();
        assert_eq!(
            differ,
            0,
            "{name} frame {frame}: {differ} cells differ on {}",
            host.name()
        );
        octs += reference_grid.octs();
        frame = frame.wrapping_add(1 + (next(&mut state) % 7) as u32);
    }
    println!(
        "{name} on {} width {}: {octs} octs, kernel equals the CPU replay and the rule",
        host.name(),
        host.width()
    );
}

#[test]
fn the_law_kernels_equal_their_cpu_replay_on_the_device() {
    let Some(host) = device("law kernels") else {
        return;
    };
    let sand = compiled(SAND);
    let heat = compiled(HEAT);
    on_device(&host, "sand law", &sand, &Sand, &[], sand_cell);
    for params in HEAT_PARAMS {
        on_device(&host, "heat law", &heat, &Heat, &params, heat_cell);
    }
}

const FOLDED: [u32; 16] = [
    0,
    1,
    2,
    3,
    7,
    31,
    32,
    33,
    0x7F,
    0x80,
    0xFF,
    0xFF00,
    0x0102_FF80,
    123_456_789,
    0x8000_0000,
    u32::MAX,
];

fn first_word(step: &str) -> u32 {
    let law = compiled(&template(&format!(
        "[\"vec\",[\"put\",\"cells\",0,{step}],[\"vec\",0]]"
    )));
    law.step(Oct {
        words: [0; CORNERS],
        frame: 0,
        at: [0; 3],
        params: &[],
    })
    .out[0]
}

fn known(value: u32) -> String {
    value.to_string()
}

fn unknown(value: u32) -> String {
    format!("[\"add\",[\"at\",\"cells\",1],{value}]")
}

fn flag(value: u32, runtime: bool) -> String {
    if runtime {
        format!("[\"ne\",{},0]", unknown(value))
    } else {
        format!("[\"ne\",{},0]", known(value))
    }
}

#[test]
fn a_folded_constant_means_what_the_runtime_operation_computes() {
    let binary = [
        "add", "sub", "mul", "div", "and", "or", "xor", "dot_s", "dot_u",
    ];
    let compares = ["eq", "ne", "lt", "le", "gt", "ge"];
    let mut checked = 0;
    for left in FOLDED {
        for right in FOLDED {
            for op in binary {
                let folded = first_word(&format!("[\"{op}\",{left},{right}]"));
                for (one, other) in [
                    (unknown(left), known(right)),
                    (known(left), unknown(right)),
                    (unknown(left), unknown(right)),
                ] {
                    let ran = first_word(&format!("[\"{op}\",{one},{other}]"));
                    assert_eq!(folded, ran, "{op} {left} {right}");
                    checked += 1;
                }
            }
            for op in compares {
                let folded = first_word(&format!("[\"word\",[\"{op}\",{left},{right}]]"));
                let ran = first_word(&format!("[\"word\",[\"{op}\",{},{right}]]", unknown(left)));
                assert_eq!(folded, ran, "{op} {left} {right}");
                checked += 1;
            }
            for op in ["both", "either"] {
                let folded = first_word(&format!(
                    "[\"word\",[\"{op}\",{},{}]]",
                    flag(left, false),
                    flag(right, false)
                ));
                for (one, other) in [(true, false), (false, true), (true, true)] {
                    let ran = first_word(&format!(
                        "[\"word\",[\"{op}\",{},{}]]",
                        flag(left, one),
                        flag(right, other)
                    ));
                    assert_eq!(folded, ran, "{op} {left} {right}");
                    checked += 1;
                }
            }
            let folded = first_word(&format!("[\"sel\",{},{right},{left}]", flag(left, false)));
            let ran = first_word(&format!("[\"sel\",{},{right},{left}]", flag(left, true)));
            assert_eq!(folded, ran, "sel {left} {right}");
            let folded = first_word(&format!(
                "[\"count\",[\"vec\",{},{},{}]]",
                flag(left, false),
                flag(right, false),
                flag(left, false)
            ));
            let ran = first_word(&format!(
                "[\"count\",[\"vec\",{},{},{}]]",
                flag(left, true),
                flag(right, false),
                flag(left, false)
            ));
            assert_eq!(folded, ran, "count {left} {right}");
            checked += 2;
        }
        for op in ["shl", "shr"] {
            for by in [0, 1, 4, 16, 31, 32, 33, 64] {
                let folded = first_word(&format!("[\"{op}\",{left},{by}]"));
                let ran = first_word(&format!("[\"{op}\",{},{by}]", unknown(left)));
                assert_eq!(folded, ran, "{op} {left} {by}");
                checked += 1;
            }
        }
        for (op, runtime) in [("not", unknown(left)), ("denied", flag(left, true))] {
            let wrap = |inner: String| {
                if op == "denied" {
                    format!("[\"word\",[\"denied\",{inner}]]")
                } else {
                    format!("[\"not\",{inner}]")
                }
            };
            let folded = if op == "denied" {
                flag(left, false)
            } else {
                known(left)
            };
            assert_eq!(
                first_word(&wrap(folded)),
                first_word(&wrap(runtime)),
                "{op} {left}"
            );
            checked += 1;
        }
        assert_eq!(
            first_word(&format!("[\"word\",{}]", flag(left, false))),
            first_word(&format!("[\"word\",{}]", flag(left, true))),
            "word {left}"
        );
        checked += 1;
    }
    println!("{checked} folded expressions equal their runtime twins");
}

#[test]
fn copying_a_vec_costs_work_for_every_item() {
    let put = template(
        "[\"vec\",\"cells\",[\"vec\",[\"at\",[\"fold\",\"v\",[\"range\",100000],\"i\",[\"range\",1000],[\"put\",\"v\",0,\"i\"]],0]]]",
    );
    let why = refused_at(&put, Code::Work, "$.step[2][1][1][5]");
    assert_eq!(why.limit, Some(limits::WORK));
    let doubled = template(
        "[\"vec\",\"cells\",[\"vec\",[\"at\",[\"fold\",\"v\",[\"vec\",0],\"i\",[\"range\",22],[\"concat\",\"v\",\"v\"]],0]]]",
    );
    refused_at(&doubled, Code::Work, "$.step[2][1][1][5]");
    for (name, body) in [
        ("map", "[\"map\",\"x\",\"v\",\"x\"]"),
        ("filter", "[\"filter\",\"x\",\"v\",1]"),
        ("rev", "[\"rev\",\"v\"]"),
    ] {
        let copied = template(&format!(
            "[\"vec\",\"cells\",[\"vec\",[\"at\",[\"fold\",\"v\",[\"range\",100000],\"i\",[\"range\",100],{body}],0]]]"
        ));
        let why = refusal(&copied);
        assert_eq!(why.code, Code::Work, "{name}: {why}");
    }
}

#[test]
fn a_name_is_found_without_scanning_every_binding_before_it() {
    let bindings: Vec<String> = (0..60_000).map(|at| format!("[\"b{at}\",0]")).collect();
    let text = template(&format!(
        "[\"let\",[{}],[\"vec\",\"cells\",[\"vec\",[\"fold\",\"a\",\"b0\",\"i\",[\"range\",450000],\"X\"]]]]",
        bindings.join(",")
    ));
    assert!(text.len() < limits::BODY);
    let start = std::time::Instant::now();
    let law = compiled(&text);
    let took = start.elapsed();
    println!(
        "{} bytes, work {}: compiled in {:.1} ms",
        text.len(),
        law.work(),
        took.as_secs_f64() * 1e3
    );
    assert!(took.as_secs() < 3, "{took:?}");
}

#[test]
fn a_name_of_the_most_bytes_compiles_and_one_byte_more_is_refused() {
    let named = |name: &str| edited(SAND, |json| set(field(json, "consts"), name, Json::Int(1)));
    let most = "N".repeat(limits::NAME);
    assert_eq!(compiled(&named(&most)).name(), "sand");
    let past = "N".repeat(limits::NAME + 1);
    let why = refused_at(
        &named(&past),
        Code::Name,
        &format!("$.consts[\"{}...\"]", "N".repeat(limits::SHOWN)),
    );
    assert_eq!(why.limit, Some(limits::NAME as u64));
    let long = "n".repeat(100_000);
    let read = template(&format!(
        "[\"let\",[[\"{long}\",0]],[\"vec\",\"cells\",[\"vec\",[\"fold\",\"a\",0,\"i\",[\"range\",450000],\"{long}\"]]]]"
    ));
    refused_at(&read, Code::Name, "$.step[1][0][0]");
    let called = template(&format!("[\"vec\",\"cells\",[\"vec\",[\"{long}\",0]]]"));
    refused_at(&called, Code::Name, "$.step[2][1][0]");
    let unknown = edited(SAND, |json| set(json, &long, Json::Int(1)));
    let why = refused_at(
        &unknown,
        Code::UnknownKey,
        &format!("$[\"{}...\"]", "n".repeat(limits::SHOWN)),
    );
    assert!(why.found.len() < 100, "{}", why.found.len());
}

struct EmbedExact;

impl grit::rule3::Declared for EmbedExact {
    fn params(&self) -> usize {
        grit::rule3::embed::PARAMS
    }

    fn tally(&self) -> grit::rule3::Tally {
        grit::rule3::embed::Embed::default().tally()
    }
}

impl OneStep for EmbedExact {
    fn one_step<O: grit::ops::Ops>(
        &self,
        ops: &mut O,
        oct: Oct<'_, O::Word>,
    ) -> grit::rule3::Ruled<O::Word> {
        grit::rule3::embed::stepped(ops, oct)
    }
}

#[test]
fn the_trace_of_embed_replays_with_identical_kernels() {
    let dag = law::trace(&EmbedExact);
    let embed = grit::rule3::embed::Embed::default();
    let mut state = 0x1234_5678_9ABC_DEF1_u64;
    let mut differ = 0;
    for _ in 0..TRACED_OCTS {
        let case = Case {
            words: core::array::from_fn(|_| word(&mut state)),
            frame: word(&mut state),
            at: [word(&mut state), word(&mut state), word(&mut state)],
        };
        let params = [word(&mut state)];
        let made = embed.step(Oct {
            words: case.words,
            frame: case.frame,
            at: case.at,
            params: &params,
        });
        if (made.out.to_vec(), made.counts) != ruled(&dag, case, &params) {
            differ += 1;
        }
    }
    println!(
        "embed: {} nodes; {differ} of {TRACED_OCTS} octs differ",
        dag.len()
    );
    assert_eq!(differ, 0);
    let grid = Grid::solid(32, 32, 32).unwrap();
    for width in WIDTHS {
        let one = sand3::settle_by(&embed, width, grid, u32::MAX)
            .unwrap()
            .words;
        let other = sand3::settle_by(&dag, width, grid, u32::MAX).unwrap().words;
        println!(
            "embed width {width}: {} words, identical {}",
            one.len(),
            one == other
        );
        assert_eq!(one, other, "embed width {width}");
    }
}

const SHIPPED: [(&str, &str); 7] = [
    ("sand", SAND),
    ("heat", HEAT),
    ("code", include_str!("law/code.law.json")),
    ("nature", include_str!("law/nature.law.json")),
    ("embed", include_str!("law/embed.law.json")),
    ("mix", include_str!("law/mix.law.json")),
    ("stir", include_str!("law/stir.law.json")),
];

fn sites_of(
    json: &Json,
    path: &mut Vec<usize>,
    names: &BTreeSet<String>,
    out: &mut Vec<Vec<usize>>,
) {
    match json {
        Json::Int(_) => out.push(path.clone()),
        Json::Text(name) if names.contains(name) => out.push(path.clone()),
        Json::List(items) => {
            let op = items.first().and_then(Json::text).unwrap_or("");
            let skipped: &[usize] = match op {
                "map" | "filter" => &[0, 1],
                "fold" => &[0, 1, 3],
                "let" => &[0, 1],
                _ => &[0],
            };
            for (at, one) in items.iter().enumerate() {
                if skipped.contains(&at) {
                    continue;
                }
                path.push(at);
                sites_of(one, path, names, out);
                path.pop();
            }
            if op == "let"
                && let Some(Json::List(bindings)) = items.get(1)
            {
                for (at, binding) in bindings.iter().enumerate() {
                    path.extend([1, at, 1]);
                    if let Some(value) = binding.list().and_then(|pair| pair.get(1)) {
                        sites_of(value, path, names, out);
                    }
                    path.truncate(path.len() - 3);
                }
            }
        }
        _ => {}
    }
}

fn constant_sites(json: &Json) -> Vec<Vec<usize>> {
    let Json::Map(fields) = json else {
        panic!("a law is an object");
    };
    let mut names: BTreeSet<String> = ["X", "Y", "Z"]
        .iter()
        .map(|one| (*one).to_owned())
        .collect();
    if let Some(Json::Map(consts)) = json.get("consts") {
        names.extend(consts.iter().map(|(name, _)| name.clone()));
    }
    let mut out = Vec::new();
    for (at, (key, value)) in fields.iter().enumerate() {
        if key == "step" {
            sites_of(value, &mut vec![at], &names, &mut out);
        }
        if let (true, Json::Map(defs)) = (key == "defs", value) {
            for (index, (_, def)) in defs.iter().enumerate() {
                if let Some(body) = def.list().and_then(|pair| pair.get(1)) {
                    sites_of(body, &mut vec![at, index, 1], &names, &mut out);
                }
            }
        }
    }
    out
}

fn known_only_when_compiled(why: &Refusal) -> bool {
    why.code == Code::Type && why.found.ends_with("known only when the law runs")
}

#[test]
fn the_type_verdict_of_a_shipped_law_does_not_change_when_a_constant_becomes_a_param() {
    let mut outcomes = std::collections::BTreeMap::new();
    for (name, text) in SHIPPED {
        let json = wire::read(text).unwrap();
        let params = json.counted("params").unwrap();
        let sites = constant_sites(&json);
        assert!(!sites.is_empty(), "{name}");
        for site in &sites {
            let mut moved = json.clone();
            set(&mut moved, "params", Json::Int(i64::from(params) + 1));
            *walk_mut(&mut moved, site) = Json::List(vec![
                Json::Text("param".into()),
                Json::Int(i64::from(params)),
            ]);
            let outcome = match Law::from_json(&moved) {
                Ok(_) => "compiles",
                Err(why) if known_only_when_compiled(&why) => "needs a known int there",
                Err(why) => {
                    assert_ne!(why.code, Code::Type, "{name} {site:?}: {why}");
                    why.code.name()
                }
            };
            *outcomes.entry(outcome).or_insert(0_u32) += 1;
        }
        let mut params_used = Vec::new();
        collect(&json, &mut Vec::new(), &mut params_used);
        let mut fixed = 0;
        for site in params_used {
            let held = walk(&json, &site);
            if held
                .list()
                .and_then(|items| items.first())
                .and_then(Json::text)
                != Some("param")
            {
                continue;
            }
            let mut moved = json.clone();
            *walk_mut(&mut moved, &site) = Json::Int(1);
            if let Err(why) = Law::from_json(&moved) {
                assert_ne!(why.code, Code::Type, "{name} {site:?} as a constant: {why}");
            }
            fixed += 1;
        }
        println!(
            "{name}: {} constants made params, {fixed} params made constants",
            sites.len()
        );
    }
    println!("{outcomes:?}");
    assert!(
        outcomes.get("compiles").copied().unwrap_or(0) > 250,
        "{outcomes:?}"
    );
    assert!(
        outcomes
            .get("needs a known int there")
            .copied()
            .unwrap_or(0)
            > 100,
        "{outcomes:?}"
    );
}

#[test]
fn a_folded_compare_is_a_flag_and_sel_picks_one_type_whether_or_not_its_flag_is_known() {
    let pairs = [
        (
            "[\"vec\",\"cells\",[\"vec\",[\"add\",[\"eq\",1,1],1]]]",
            "[\"vec\",\"cells\",[\"vec\",[\"add\",[\"eq\",[\"param\",0],1],1]]]",
            "$.step[2][1][1]",
        ),
        (
            "[\"vec\",[\"sel\",[\"eq\",1,1],\"cells\",0],[\"vec\",0]]",
            "[\"vec\",[\"sel\",[\"eq\",[\"param\",0],1],\"cells\",0],[\"vec\",0]]",
            "$.step[1][2]",
        ),
        (
            "[\"vec\",\"cells\",[\"vec\",[\"sel\",[\"eq\",1,1],0,[\"eq\",1,0]]]]",
            "[\"vec\",\"cells\",[\"vec\",[\"sel\",[\"eq\",[\"param\",0],1],0,[\"eq\",1,0]]]]",
            "$.step[2][1][3]",
        ),
        (
            "[\"vec\",\"cells\",[\"vec\",[\"word\",[\"both\",1,[\"eq\",1,1]]]]]",
            "[\"vec\",\"cells\",[\"vec\",[\"word\",[\"both\",[\"param\",0],[\"eq\",1,1]]]]]",
            "$.step[2][1][1][1]",
        ),
        (
            "[\"vec\",\"cells\",[\"vec\",[\"count\",[\"vec\",[\"eq\",1,1],1]]]]",
            "[\"vec\",\"cells\",[\"vec\",[\"count\",[\"vec\",[\"eq\",1,1],[\"param\",0]]]]]",
            "$.step[2][1][1]",
        ),
        (
            "[\"vec\",\"cells\",[\"vec\",[\"eq\",1,1]]]",
            "[\"vec\",\"cells\",[\"vec\",[\"eq\",[\"param\",0],1]]]",
            "$.step",
        ),
    ];
    for (folded, live, path) in pairs {
        let with = |step: &str| edited(&template(step), |json| set(json, "params", Json::Int(1)));
        let one = refusal(&with(folded));
        let other = refusal(&with(live));
        assert_eq!(
            (one.code, one.path.as_str()),
            (other.code, other.path.as_str()),
            "{one} | {other}"
        );
        assert_eq!(one.path, path, "{one}");
    }
    let picked = edited(
        &template("[\"vec\",\"cells\",[\"vec\",[\"sel\",[\"eq\",1,1],[\"param\",0],7]]]"),
        |json| set(json, "params", Json::Int(1)),
    );
    assert_eq!(
        compiled(&picked)
            .step(Oct {
                words: [0; CORNERS],
                frame: 0,
                at: [0; 3],
                params: &[5],
            })
            .counts,
        vec![5]
    );
    let why = refusal(&template(
        "[\"vec\",\"cells\",[\"vec\",[\"if\",[\"ne\",\"frame\",0],1,2]]]",
    ));
    assert_eq!(
        (why.code, why.path.as_str()),
        (Code::Type, "$.step[2][1][1]"),
        "{why}"
    );
    assert_eq!(why.found, "a flag, known only when the law runs");
}

#[test]
fn a_repeated_key_is_refused_at_its_json_path() {
    assert_eq!(
        wire::read("{\"a\":1,\"a\":2}"),
        Err(wire::Torn::Twice { at: 7 })
    );
    assert_eq!(wire::Torn::Twice { at: 7 }.to_string(), "7 '\"'");
    let cases = [
        (
            SAND.replacen("\"counts\": 3", "\"counts\": 3, \"counts\": 4", 1),
            "$.tally.counts",
            "counts",
        ),
        (
            SAND.replacen("\"AIR\": 0", "\"AIR\": 0, \"AIR\": 1", 1),
            "$.consts.AIR",
            "AIR",
        ),
        (
            SAND.replacen("\"AIR\": 0", "\"A\\u0042\": 0, \"AB\": 1", 1),
            "$.consts.AB",
            "AB",
        ),
        (
            SAND.replacen(
                "\"mat\": [[\"w\"], [\"and\", \"w\", 3]],",
                "\"mat\": [[\"w\"], [\"and\", \"w\", 3]], \"mat\": [[\"w\"], 0],",
                1,
            ),
            "$.defs.mat",
            "mat",
        ),
        (
            template("[\"vec\",\"cells\",[\"vec\",{\"k\":1,\"k\":2}]]"),
            "$.step[2][1].k",
            "k",
        ),
        (
            template(
                "[\"vec\",\"cells\",[\"vec\",[\"a,]}\\\"[{\",{\"b\":[1,{\"c\":2}],\"b\":3}]]]",
            ),
            "$.step[2][1][1].b",
            "b",
        ),
        (
            SAND.replacen("\"name\": \"sand\"", "\"name\": \"s\\\"],{\"", 1)
                .replacen(
                    "\"weighted\": true",
                    "\"weighted\": true, \"weighted\": false",
                    1,
                ),
            "$.tally.weighted",
            "weighted",
        ),
        (
            SAND.replacen(
                "\"AIR\": 0",
                &format!("\"{0}\": 0, \"{0}\": 1", "N".repeat(100)),
                1,
            ),
            &format!("$.consts[\"{}...\"]", "N".repeat(limits::SHOWN)),
            &format!("{}...", "N".repeat(limits::SHOWN)),
        ),
        (
            SAND.replacen("\"AIR\": 0", "\"é\": 0, \"é\": 1", 1),
            "$.consts[\"é\"]",
            "é",
        ),
    ];
    for (text, path, key) in &cases {
        let why = refused_at(text, Code::DuplicateKey, path);
        assert_eq!(why.found, format!("the key \"{key}\" again"), "{why}");
        assert_eq!(why.expected, "each key once");
    }
}

#[test]
fn a_raw_control_byte_inside_a_string_is_refused_at_its_byte() {
    for (byte, text) in [
        (0x0a_u8, "[\"a\nb\"]"),
        (0x09, "[\"a\tb\"]"),
        (0x00, "[\"a\u{0}b\"]"),
        (0x1f, "[\"a\u{1f}b\"]"),
    ] {
        assert_eq!(
            wire::read(text),
            Err(wire::Torn::Control { at: 3, byte }),
            "{byte:#04x}"
        );
    }
    assert_eq!(
        wire::Torn::Control { at: 3, byte: 0x0a }.to_string(),
        "3 control byte 0x0a"
    );
    assert_eq!(
        wire::read("[\"a\\nb\",\"\u{7f}\",\"\u{e9}\"]"),
        Ok(Json::List(vec![
            Json::Text("a\nb".into()),
            Json::Text("\u{7f}".into()),
            Json::Text("\u{e9}".into()),
        ]))
    );
    assert_eq!(
        wire::read("[1,\n2]"),
        Ok(Json::List(vec![Json::Int(1), Json::Int(2)]))
    );
    let raw = template("[\"let\",[[\"a\nb\",0]],[\"vec\",\"cells\",[\"vec\",0]]]");
    let why = refused_at(&raw, Code::Json, "$");
    let at = raw.find('\n').unwrap();
    assert_eq!(
        why.found,
        format!(
            "the control byte 0x0a raw inside a string at byte {at}, which JSON writes escaped"
        )
    );
    let escaped = template("[\"let\",[[\"a\\nb\",0]],[\"vec\",\"cells\",[\"vec\",0]]]");
    assert!(Law::compile(&escaped).is_ok());
}
