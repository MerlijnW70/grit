#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use std::collections::BTreeSet;

use common::device;

use grit::bricks::Bricks;
use grit::compact::Plan;
use grit::law::Law;
use grit::layout::{Dimension, PARAMS, TALLY};
use grit::rule3::code::{self, Code};
use grit::rule3::embed::{self, Embed};
use grit::rule3::nature::{self, Nature};
use grit::rule3::{CORNERS, Declared, FIRST_PARAM, Oct, Rule};
use grit::sand::Grid;
use grit::wire::{self, Json};
use grit::{Groups, Host, Resident, sweep};

const NATURE: &str = include_str!("law/nature.law.json");

const CODE: &str = include_str!("law/code.law.json");

const EMBED: &str = include_str!("law/embed.law.json");

const NATURE_HASH: &str = "2eed8f63c4f202c06c152693c39798a5f332ed4c2f268db4ca5af64f52c9b33b";

const CODE_HASH: &str = "aedb8e70723d1a7be8acebca2d0458b52e2b44b74a2f3fc74a19ee58831b7f9d";

const EMBED_HASH: &str = "8966573f4ba850c92243ecb06dff789a415263bd2c1517c18423f65603d76005";

const RANDOM_OCTS: u32 = 100_000;

const FRAMES: u32 = 256;

const BATCH: u32 = 16;

const SIDE: u32 = 32;

const SUNS: [u32; 3] = [3, 1, 255];

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

fn nature_cell(state: &mut u64) -> u32 {
    let roll = next(state);
    let kind = match roll % 40 {
        0..9 => nature::AIR,
        9..11 => nature::ROCK,
        11..13 => nature::EARTH,
        13..15 => nature::SAND,
        15..17 => nature::SNOW,
        17..19 => nature::ICE,
        19..24 => nature::WATER,
        24..26 => nature::WOOD,
        26..28 => nature::LEAVES,
        28..31 => nature::FIRE,
        31..32 => nature::ASH,
        32..35 => nature::SMOKE,
        35..37 => nature::STEAM,
        37..39 => nature::CHAR,
        _ => nature::KINDS + ((roll >> 40) % 2) as u32,
    };
    let temperature = match (roll >> 44) % 8 {
        0 => nature::TOP - ((roll >> 48) % 3) as u32,
        1 => ((roll >> 48) & 0xFFFF) as u32,
        _ => ((roll >> 8) % 800) as u32,
    };
    let state_bits = ((roll >> 20) & 0xFF) as u32;
    let kept = ((roll >> 28) & 0xF) as u32;
    kind | (state_bits << 4) | (kept << 12) | (temperature << 16)
}

fn code_cell(state: &mut u64) -> u32 {
    let roll = next(state);
    let instruction = match roll % 24 {
        0..6 => code::NOP,
        6..10 => code::DATA,
        10..14 => code::MOVE,
        14..16 => code::COPY,
        16..19 => code::ADD,
        19..21 => code::TURN,
        21..23 => code::HALT,
        _ => code::INSTRUCTIONS + ((roll >> 40) % 9) as u32,
    };
    let way = ((roll >> 8) % 8) as u32;
    let payload = match (roll >> 12) % 4 {
        0 => code::PAYLOAD - ((roll >> 16) % 3) as u32,
        _ => ((roll >> 16) & u64::from(code::PAYLOAD)) as u32,
    };
    code::word(instruction, way, payload) | (((roll >> 50) & 1) as u32) << 31
}

fn embed_cell(state: &mut u64) -> u32 {
    let roll = next(state);
    match roll % 4 {
        0 => embed::embedding([(roll >> 8) as i8, (roll >> 16) as i8, 0, 0]),
        1 => ((roll >> 24) % 5) as u32,
        _ => (roll >> 16) as u32,
    }
}

const THRESHOLDS: [u32; 8] = [
    nature::MELT,
    nature::BOIL,
    nature::CONDENSE,
    nature::WOOD_IGNITES,
    nature::LEAVES_IGNITES,
    nature::TOP - 3,
    nature::BOIL - 3,
    0,
];

const STATES: [u32; 16] = [
    0, 1, 2, 7, 8, 9, 59, 60, 61, 191, 192, 193, 252, 253, 254, 255,
];

fn nature_oct(state: &mut u64) -> [u32; CORNERS] {
    if next(state).is_multiple_of(3) {
        return core::array::from_fn(|_| nature_cell(state));
    }
    let kinds = [nature_cell(state), nature_cell(state), nature_cell(state)].map(nature::kind_of);
    let base = match next(state) % 4 {
        0 => (next(state) % 1200) as u32,
        _ => THRESHOLDS[(next(state) % 8) as usize].saturating_sub(1),
    };
    core::array::from_fn(|_| {
        let cell = nature_cell(state);
        let kind = kinds[(next(state) % 3) as usize];
        let temperature = (base + (next(state) % 4) as u32).min(nature::TOP);
        let cell = if next(state).is_multiple_of(2) {
            (cell & 0xF00F) | (STATES[(next(state) % 16) as usize] << 4)
        } else {
            cell
        };
        (cell & 0xFFF0) | kind | (temperature << 16)
    })
}

fn code_oct(state: &mut u64) -> [u32; CORNERS] {
    core::array::from_fn(|_| code_cell(state))
}

fn embed_oct(state: &mut u64) -> [u32; CORNERS] {
    core::array::from_fn(|_| embed_cell(state))
}

fn place(state: &mut u64) -> u32 {
    match next(state) % 8 {
        0 => 0,
        1 => 0x8000_0000,
        2 => 0x4000_0000,
        3 => 0xC000_0000,
        4 => 1,
        _ => word(state),
    }
}

#[derive(Clone, Copy, Debug)]
struct Case {
    words: [u32; CORNERS],
    frame: u32,
    at: [u32; 3],
}

fn ruled<R: Rule>(rule: &R, case: Case, params: &[u32]) -> (Vec<u32>, Vec<u32>) {
    let made = rule.step(Oct {
        words: case.words,
        frame: case.frame,
        at: case.at,
        params,
    });
    (made.out.to_vec(), made.counts)
}

fn placed(state: &mut u64, words: [u32; CORNERS]) -> Case {
    Case {
        words,
        frame: word(state),
        at: [place(state), place(state), place(state)],
    }
}

type Octs = fn(&mut u64) -> [u32; CORNERS];

fn random_cases(oct: Octs, masks: &[u32], many: u32, seed: u64) -> Vec<Case> {
    let mut state = seed;
    (0..many)
        .map(|at| {
            let mask = masks[at as usize % masks.len()];
            let words = if next(&mut state).is_multiple_of(4) {
                core::array::from_fn(|_| word(&mut state) & mask)
            } else {
                oct(&mut state)
            };
            placed(&mut state, words)
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

fn boundary_cases(oct: Octs, constants: &[u32], seed: u64) -> Vec<Case> {
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
                    let words = oct(&mut state).map(|base| {
                        if next(&mut state).is_multiple_of(2) {
                            (base & !field) | value
                        } else {
                            base
                        }
                    });
                    cases.push(placed(&mut state, words));
                }
            }
        }
    }
    cases
}

fn nature_cases(many: u32, params: &[u32]) -> Vec<Case> {
    let mut cases = random_cases(
        nature_oct,
        &[0xFFFF_0FFF, 0x0FFF_0F0F, u32::MAX, 0x0000_000F, 0x03FF_08FF],
        many,
        0x4A7E,
    );
    cases.extend(boundary_cases(
        nature_oct,
        &constants(NATURE, params),
        0x5E7,
    ));
    cases
}

fn code_cases(many: u32) -> Vec<Case> {
    let mut cases = random_cases(
        code_oct,
        &[u32::MAX, 0x7F00_00FF, 0x0700_00FF, 0x7700_0003],
        many,
        0xC0DE,
    );
    cases.extend(boundary_cases(code_oct, &constants(CODE, &[]), 0xC0D));
    cases
}

fn embed_cases(many: u32, params: &[u32]) -> Vec<Case> {
    let mut cases = random_cases(
        embed_oct,
        &[u32::MAX, 0x8181_8181, 0x0000_FFFF, 0x0303_0303],
        many,
        0xE3BE,
    );
    cases.extend(boundary_cases(embed_oct, &constants(EMBED, params), 0xE3B));
    cases
}

fn embed_towards() -> Vec<u32> {
    let mut state = 0x70_3A2D_u64;
    let mut held = vec![
        embed::embedding([1, -1, 2, -2]),
        embed::embedding([127, -128, 0, 64]),
        0,
        u32::MAX,
        0x8000_0000,
    ];
    held.extend((0..3).map(|_| word(&mut state)));
    held
}

fn moved_sets(base: &[u32]) -> Vec<Vec<u32>> {
    let mut sets = vec![base.to_vec()];
    for which in 0..base.len() {
        for moved in [base[which].wrapping_sub(1), base[which].wrapping_add(1)] {
            let mut set = base.to_vec();
            set[which] = moved;
            sets.push(set);
        }
    }
    sets
}

fn differing<A: Rule, B: Rule>(one: &A, other: &B, params: &[u32], cases: &[Case]) -> usize {
    cases
        .iter()
        .filter(|&&case| ruled(one, case, params) != ruled(other, case, params))
        .count()
}

fn compiled(text: &str) -> Law {
    Law::compile(text).unwrap_or_else(|why| panic!("{why}"))
}

#[test]
fn nature_law_equals_the_rust_rule() {
    let law = compiled(NATURE);
    let mut octs = 0;
    for sun in [0, SUNS[0], SUNS[2]] {
        for set in moved_sets(&[sun]) {
            let cases = nature_cases(RANDOM_OCTS / 7, &set);
            let differ = differing(&law, &Nature, &set, &cases);
            println!(
                "nature law {set:?}: {} nodes, work {}, depth {}; {differ} of {} octs differ from Nature",
                law.nodes(),
                law.work(),
                law.depth(),
                cases.len()
            );
            assert_eq!(differ, 0, "params {set:?}");
            octs += cases.len();
        }
    }
    println!("nature law: {octs} octs in all");
    assert_eq!(law.params(), Nature.params());
    assert_eq!(law.tally(), Nature.tally());
}

#[test]
fn code_law_equals_the_rust_rule() {
    let law = compiled(CODE);
    let cases = code_cases(RANDOM_OCTS);
    let differ = differing(&law, &Code, &[], &cases);
    println!(
        "code law: {} nodes, work {}, depth {}; {differ} of {} octs differ from Code",
        law.nodes(),
        law.work(),
        law.depth(),
        cases.len()
    );
    assert_eq!(differ, 0);
    assert_eq!(law.params(), Code.params());
    assert_eq!(law.tally(), Code.tally());
}

#[test]
fn embed_law_equals_the_rust_rule() {
    let law = compiled(EMBED);
    let rules = [
        Embed::new(grit::ops::Dots::Exact),
        Embed::new(grit::ops::Dots::Native),
    ];
    let mut octs = 0;
    for toward in embed_towards() {
        for set in moved_sets(&[toward]) {
            let cases = embed_cases(RANDOM_OCTS.div_ceil(24), &set);
            for rule in &rules {
                let differ = differing(&law, rule, &set, &cases);
                println!(
                    "embed law {set:?} {:?}: {} nodes, work {}, depth {}; {differ} of {} octs differ from Embed",
                    rule.dots,
                    law.nodes(),
                    law.work(),
                    law.depth(),
                    cases.len()
                );
                assert_eq!(differ, 0, "params {set:?}");
            }
            octs += cases.len();
        }
    }
    println!("embed law: {octs} octs in all");
    assert_eq!(law.params(), Embed::default().params());
    assert_eq!(law.tally(), Embed::default().tally());
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

type Sets = Vec<(Vec<u32>, Vec<Case>)>;

fn survivors<R: Rule>(text: &str, rule: &R, sets: &Sets) -> Vec<String> {
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

const NATURE_EQUIVALENT: [&str; 13] = [
    "$.consts.RUNNING_MASK 112->113",
    "$.consts.WORN 1792->1793",
    "$.consts.WOODEN 2048->2049",
    "$.consts.COUNT_MASK 112->113",
    "$.consts.LEAVES_CATCHES_ABOVE 16->17",
    "$.consts.STILL 5->6",
    "$.defs.touched[1][1][3][1][4][2][1] 8->9",
    "$.defs.diffused[1][1][1][1][2][2] 0->1",
    "$.defs.diffused[1][1][1][1][2][3] 4294967295->4294967294",
    "$.defs.x_first[1][1][3][1][5][4][2][1] 8->7",
    "$.defs.moved[1][1][0][1][1][2][1] 8->9",
    "$.defs.blown[1][1][0][1][5][4][2][1] 8->7",
    "$.defs.blown[1][2][5][4][2][1] 8->7",
];

const CODE_EQUIVALENT: [&str; 0] = [];

const EMBED_EQUIVALENT: [&str; 1] = ["$.defs.sorted[1][5][4][2][1] 8->7"];

#[test]
fn every_constant_moved_by_one_in_the_nature_law_is_caught() {
    let sets: Sets = [0, SUNS[0], SUNS[1], SUNS[2]]
        .into_iter()
        .map(|sun| (vec![sun], nature_cases(RANDOM_OCTS / 10, &[sun])))
        .collect();
    let left = survivors(NATURE, &Nature, &sets);
    assert_eq!(left, NATURE_EQUIVALENT);
}

#[test]
fn every_constant_moved_by_one_in_the_code_law_is_caught() {
    let sets: Sets = vec![(Vec::new(), code_cases(RANDOM_OCTS / 5))];
    let left = survivors(CODE, &Code, &sets);
    assert_eq!(left, CODE_EQUIVALENT);
}

#[test]
fn every_constant_moved_by_one_in_the_embed_law_is_caught() {
    let sets: Sets = embed_towards()
        .into_iter()
        .map(|toward| (vec![toward], embed_cases(RANDOM_OCTS / 40, &[toward])))
        .collect();
    let left = survivors(EMBED, &Embed::default(), &sets);
    assert_eq!(left, EMBED_EQUIVALENT);
}

#[test]
fn the_canonical_form_and_hash_of_each_law_are_pinned() {
    for (name, text, hash) in [
        ("nature", NATURE, NATURE_HASH),
        ("code", CODE, CODE_HASH),
        ("embed", EMBED, EMBED_HASH),
    ] {
        let law = compiled(text);
        println!("{name}: {} nodes, hash {}", law.nodes(), law.hex());
        assert_eq!(law.name(), name);
        assert_eq!(law.hex(), hash, "{name}");
        assert_eq!(
            grit::law::hex(&grit::branch::sha256(law.canonical().as_bytes())),
            hash
        );
        let mut json = wire::read(text).unwrap();
        if let Json::Map(fields) = &mut json {
            fields.reverse();
        }
        let spaced = wire::written(&json)
            .replace(',', " ,\n\t")
            .replace(':', " : ");
        assert_ne!(spaced, text);
        assert_eq!(compiled(&spaced).hex(), hash, "{name}");
    }
}

struct World<'host> {
    live: Resident<'host>,
    stages: Vec<Vec<Groups>>,
    grid: Grid,
    plan: Plan,
    bricks: Option<Bricks>,
    tally: usize,
}

impl<'host> World<'host> {
    fn open<R: Rule>(
        host: &'host Host,
        rule: &R,
        params: &[u32],
        cells: &[u32],
        sparse: bool,
    ) -> Self {
        let width = host.width();
        let grid = Grid::solid(SIDE, SIDE, SIDE).unwrap();
        let blocks = grid.blocks().div_ceil(sweep::cells(width));
        let (plan, bricks) = if sparse {
            let plan = Plan::bricked(width, blocks).unwrap();
            let bricks = Bricks::new(grid, &plan, plan.blocks).unwrap();
            (plan, Some(bricks))
        } else {
            (
                Plan::over_in(width, blocks, Dimension::Three).unwrap(),
                None,
            )
        };
        let ((passes, stages), sizes) = match bricks.as_ref() {
            Some(bricks) => (
                grit::bricks::arranged(rule, bricks, &plan, true).unwrap(),
                bricks.sizes(rule, &plan),
            ),
            None => (
                grit::live::wired3_by(rule, width, grid, &plan).unwrap(),
                plan.windows().laid(&grit::live::sizes3(rule, &plan)),
            ),
        };
        let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
        let mut live = host.resident(&sizes, &modules).unwrap();
        for (binding, &room) in sizes.iter().enumerate() {
            live.write(binding, &vec![0; room]).unwrap();
        }
        if !params.is_empty() {
            live.write_at(PARAMS as usize, FIRST_PARAM, params).unwrap();
        }
        match bricks.as_ref() {
            Some(bricks) => {
                live.write(PARAMS as usize, &[u32::MAX]).unwrap();
                let packed = bricks.pack(cells).unwrap();
                bricks.write_packed(&mut live, &plan, &packed).unwrap();
                bricks.seed(&mut live, &plan).unwrap();
            }
            None => {
                live.write(PARAMS as usize, &[u32::MAX, plan.dump()])
                    .unwrap();
                grit::live::write_state(&mut live, plan.windows(), 0, cells).unwrap();
                grit::live::seed(&mut live, grid, &plan).unwrap();
            }
        }
        World {
            live,
            stages,
            grid,
            plan,
            bricks,
            tally: rule.tally().words(),
        }
    }

    fn step(&mut self, frames: u32) {
        let groups: Vec<&[Groups]> = self.stages.iter().map(Vec::as_slice).collect();
        self.live.staged_frames(&groups, frames).unwrap();
    }

    fn cells(&mut self) -> Vec<u32> {
        match self.bricks {
            Some(bricks) => bricks.read(&mut self.live).unwrap(),
            None => grit::live::read_state(&mut self.live, self.plan.windows(), self.grid.cells())
                .unwrap(),
        }
    }

    fn tally(&mut self) -> Vec<u32> {
        self.live.read(TALLY as usize, self.tally).unwrap()
    }
}

fn landscape(cell: fn(&mut u64) -> u32, seed: u64) -> Vec<u32> {
    let mut state = seed;
    let side = SIDE as usize;
    let mut cells = vec![0_u32; side * side * side];
    for (index, slot) in cells.iter_mut().enumerate() {
        let x = index % side;
        let z = (index / side) % side;
        let y = index / (side * side);
        let inside = (6..26).contains(&x) && (4..24).contains(&y) && (8..20).contains(&z);
        if inside || next(&mut state).is_multiple_of(97) {
            *slot = cell(&mut state);
        }
    }
    cells
}

fn worlds_agree<A: Rule, B: Rule>(
    host: &Host,
    name: &str,
    law: &A,
    rule: &B,
    params: &[u32],
    cell: fn(&mut u64) -> u32,
) {
    let start = landscape(cell, 0x3D_0000 ^ name.len() as u64);
    for sparse in [false, true] {
        let mut by_law = World::open(host, law, params, &start, sparse);
        let mut by_rule = World::open(host, rule, params, &start, sparse);
        let mut done = 0;
        while done < FRAMES {
            by_law.step(BATCH);
            by_rule.step(BATCH);
            done += BATCH;
            let cells = by_rule.cells();
            assert_eq!(
                by_law.cells(),
                cells,
                "{name} sparse {sparse} after {done} frames on {}",
                host.name()
            );
            assert_eq!(
                by_law.tally(),
                by_rule.tally(),
                "{name} sparse {sparse} tally after {done} frames"
            );
            if done == FRAMES {
                let moved = cells.iter().zip(&start).filter(|(a, b)| a != b).count();
                println!(
                    "{name} sparse {sparse} on {} width {}: {done} frames, law world equals rule world, {moved} cells moved",
                    host.name(),
                    host.width()
                );
                assert!(moved > 0, "{name} sparse {sparse}: nothing moved");
            }
        }
    }
}

#[test]
fn the_nature_law_world_equals_the_rule_world_dense_and_sparse() {
    let Some(host) = device("nature law world") else {
        return;
    };
    worlds_agree(
        &host,
        "nature",
        &compiled(NATURE),
        &Nature,
        &[SUNS[0]],
        nature_cell,
    );
}

#[test]
fn the_code_law_world_equals_the_rule_world_dense_and_sparse() {
    let Some(host) = device("code law world") else {
        return;
    };
    worlds_agree(&host, "code", &compiled(CODE), &Code, &[], code_cell);
}

#[test]
fn the_embed_law_world_equals_the_rule_world_dense_and_sparse() {
    let Some(host) = device("embed law world") else {
        return;
    };
    worlds_agree(
        &host,
        "embed",
        &compiled(EMBED),
        &Embed::on(&host),
        &[embed::embedding([1, -1, 2, -2])],
        embed_cell,
    );
}
