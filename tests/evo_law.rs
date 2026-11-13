#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use std::collections::BTreeSet;

use common::device;

use grit::bricks::Bricks;
use grit::compact::Plan;
use grit::law::Law;
use grit::layout::{Dimension, PARAMS, TALLY};
use grit::rule3::evo::{self, Evo};
use grit::rule3::{CORNERS, Declared, FIRST_PARAM, Oct, Rule};
use grit::sand::Grid;
use grit::wire::{self, Json};
use grit::{Groups, Host, Resident, sweep};

const EVO: &str = include_str!("law/evo.law.json");

const EVO_HASH: &str = "1684eef78b7b08ed275d568366626b49fa37aab54c7c5f7cec06afe2e6f9a839";

const RANDOM_OCTS: u32 = 100_000;

const FRAMES: u32 = 256;

const BATCH: u32 = 16;

const SIDE: u32 = 32;

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

fn evo_cell(state: &mut u64) -> u32 {
    let roll = next(state);
    match roll % 10 {
        0..4 => {
            let genome = (roll >> 8) as u32 & evo::GENOME_MASK;
            let energy = match (roll >> 24) % 4 {
                0 => (roll >> 30) as u32 % 12,
                1 => evo::threshold_of(genome) - 2 + (roll >> 30) as u32 % 24,
                _ => (roll >> 30) as u32 & evo::FULL,
            };
            let age = match (roll >> 40) % 3 {
                0 => evo::lifespan_of(genome) - (roll >> 44) as u32 % 3,
                _ => (roll >> 44) as u32 & evo::OLDEST,
            };
            evo::cell(genome, energy, age) | (((roll >> 52) as u32 & 1) << 15)
        }
        4..7 => 0,
        7 => (roll >> 20) as u32 & !evo::ALIVE,
        _ => (roll >> 16) as u32,
    }
}

fn evo_oct(state: &mut u64) -> [u32; CORNERS] {
    let share = next(state) % 9;
    core::array::from_fn(|_| {
        let cell = evo_cell(state);
        if next(state) % 8 < share {
            cell | evo::ALIVE
        } else {
            cell
        }
    })
}

fn place(state: &mut u64) -> u32 {
    match next(state) % 8 {
        0 => 0,
        1 => 0x8000_0000,
        2 => 0x4000_0000,
        3 => u32::MAX,
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

fn random_cases(many: u32, seed: u64) -> Vec<Case> {
    let mut state = seed;
    (0..many)
        .map(|_| {
            let words = if next(&mut state).is_multiple_of(5) {
                core::array::from_fn(|_| word(&mut state))
            } else {
                evo_oct(&mut state)
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

fn constants(params: &[u32]) -> Vec<u32> {
    let json = wire::read(EVO).unwrap();
    let mut held = BTreeSet::new();
    literals(json.get("consts").unwrap(), &mut held);
    literals(json.get("defs").unwrap(), &mut held);
    literals(json.get("step").unwrap(), &mut held);
    held.extend(params.iter().copied());
    held.into_iter().collect()
}

fn boundary_cases(constants: &[u32], seed: u64) -> Vec<Case> {
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
                let words = evo_oct(&mut state).map(|base| {
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
    cases
}

fn edge_cases(params: &[u32]) -> Vec<Case> {
    let mut cases = Vec::new();
    let full = evo::cell(evo::genome(3, 3, 0, 0x155), 200, 1);
    let mut at = 0_u32;
    while cases.len() < 4 {
        let base = evo::base_of([at, 7, 9], 3, params[evo::SEED]);
        if (0..8).any(|corner| evo::roll_of(base, corner) & 0xFFFF == 0xFFFF) {
            cases.push(Case {
                words: [full; CORNERS],
                frame: 3,
                at: [at, 7, 9],
            });
        }
        at += 1;
    }
    cases
}

fn evo_cases(many: u32, params: &[u32]) -> Vec<Case> {
    let mut cases = random_cases(many, 0xE70 ^ u64::from(params[evo::SEED]));
    cases.extend(boundary_cases(&constants(params), 0xE7B));
    cases.extend(edge_cases(params));
    cases
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

fn param_sets() -> Vec<Vec<u32>> {
    let mut sets = moved_sets(&evo::DEFAULTS);
    let mut edges = evo::DEFAULTS;
    edges[evo::HAZARD] = evo::CHANCES;
    edges[evo::MUTATION] = evo::CHANCES;
    sets.push(edges.to_vec());
    sets.push(vec![0; evo::PARAMS]);
    sets.push(vec![u32::MAX; evo::PARAMS]);
    sets.push(vec![255, 256, 254, 255, 256, 65_535, 65_537, 0x8000_0000]);
    sets.push(vec![7, 0, 0, 0, 0, 0, 0, 1]);
    sets.push(vec![2, 2, 1, 1, 8, 65_535, 65_535, 0]);
    sets.push(vec![1, 1, 1, 1, 64, 0, 0, 3]);
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
fn the_evo_law_equals_the_rust_rule() {
    let law = compiled(EVO);
    let sets = param_sets();
    let each = RANDOM_OCTS.div_ceil(sets.len() as u32);
    let mut octs = 0;
    let mut random = 0;
    for set in &sets {
        let cases = evo_cases(each, set);
        let differ = differing(&law, &Evo, set, &cases);
        println!(
            "evo law {set:?}: {} nodes, work {}, depth {}; {differ} of {} octs differ from Evo",
            law.nodes(),
            law.work(),
            law.depth(),
            cases.len()
        );
        assert_eq!(differ, 0, "params {set:?}");
        octs += cases.len();
        random += each;
    }
    println!(
        "evo law: {octs} octs in all, {random} of them random, over {} param sets",
        sets.len()
    );
    assert!(random >= RANDOM_OCTS);
    assert_eq!(law.params(), Evo.params());
    assert_eq!(law.tally(), Evo.tally());
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

const EVO_EQUIVALENT: [&str; 4] = [
    "$.defs.chosen_by[1][2][1] 3->4",
    "$.defs.flips_by[1][1][1][1][2][1] 8->9",
    "$.defs.resolved[1][2][2] 0->1",
    "$.step[1][3][1][2][1] 8->9",
];

#[test]
fn every_constant_moved_by_one_in_the_evo_law_is_caught() {
    let sets: Vec<(Vec<u32>, Vec<Case>)> = param_sets()
        .into_iter()
        .map(|set| {
            let cases = evo_cases(RANDOM_OCTS / 20, &set);
            (set, cases)
        })
        .collect();
    let json = wire::read(EVO).unwrap();
    let mut all = Vec::new();
    sites(&json, &mut Vec::new(), &mut all);
    let skipped = ["law", "params", "channels", "halo", "tally"];
    let wanted: Vec<Vec<_>> = sets
        .iter()
        .map(|(params, cases)| {
            cases
                .iter()
                .map(|&case| ruled(&Evo, case, params))
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
    assert_eq!(left, EVO_EQUIVALENT);
}

#[test]
fn the_canonical_form_and_hash_of_the_evo_law_are_pinned() {
    let law = compiled(EVO);
    println!("evo: {} nodes, hash {}", law.nodes(), law.hex());
    assert_eq!(law.name(), "evo");
    assert_eq!(law.hex(), EVO_HASH);
    assert_eq!(
        grit::law::hex(&grit::branch::sha256(law.canonical().as_bytes())),
        EVO_HASH
    );
    let mut json = wire::read(EVO).unwrap();
    if let Json::Map(fields) = &mut json {
        fields.reverse();
    }
    let spaced = wire::written(&json)
        .replace(',', " ,\n\t")
        .replace(':', " : ");
    assert_ne!(spaced, EVO);
    assert_eq!(compiled(&spaced).hex(), EVO_HASH);
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
        live.write_at(PARAMS as usize, FIRST_PARAM, params).unwrap();
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

fn landscape(seed: u64) -> Vec<u32> {
    let mut state = seed;
    let side = SIDE as usize;
    let mut cells = vec![0_u32; side * side * side];
    for (index, slot) in cells.iter_mut().enumerate() {
        let x = index % side;
        let z = (index / side) % side;
        let y = index / (side * side);
        let inside = (6..26).contains(&x) && (4..24).contains(&y) && (8..20).contains(&z);
        let roll = next(&mut state);
        if (inside && roll.is_multiple_of(3)) || roll.is_multiple_of(97) {
            let genome = (roll >> 8) as u32 & evo::GENOME_MASK;
            *slot = evo::cell(
                genome,
                40 + (roll >> 32) as u32 % 100,
                (roll >> 40) as u32 % 20,
            );
        }
    }
    cells
}

#[test]
fn the_evo_law_world_equals_the_rule_world_dense_and_sparse() {
    let Some(host) = device("evo law world") else {
        return;
    };
    let law = compiled(EVO);
    let start = landscape(0x3D_0E70);
    let mut params = evo::DEFAULTS;
    params[evo::MUTATION] = 1024;
    params[evo::SEED] = 0x5EED;
    for sparse in [false, true] {
        let mut by_law = World::open(&host, &law, &params, &start, sparse);
        let mut by_rule = World::open(&host, &Evo, &params, &start, sparse);
        let mut done = 0;
        while done < FRAMES {
            by_law.step(BATCH);
            by_rule.step(BATCH);
            done += BATCH;
            let cells = by_rule.cells();
            assert_eq!(
                by_law.cells(),
                cells,
                "evo sparse {sparse} after {done} frames on {}",
                host.name()
            );
            let tally = by_rule.tally();
            assert_eq!(
                by_law.tally(),
                tally,
                "evo sparse {sparse} tally after {done} frames"
            );
            if done == FRAMES {
                let moved = cells.iter().zip(&start).filter(|(a, b)| a != b).count();
                let alive = cells.iter().filter(|&&one| evo::alive_of(one)).count();
                println!(
                    "evo sparse {sparse} on {} width {}: {done} frames, law world equals rule world, {moved} cells moved, {alive} living, last tally {tally:?}",
                    host.name(),
                    host.width()
                );
                assert!(
                    moved > 0 && alive > 0 && tally[evo::BIRTHS] > 0 && tally[evo::MUTATIONS] > 0
                );
            }
        }
    }
}
