#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use std::collections::BTreeSet;

use common::device;

use grit::bricks::{self, Bricks};
use grit::compact::Plan;
use grit::law::adjoint::{self, Aim, Fault, Lifting, Plain, Posed, Reversible};
use grit::law::{Code, Law, Refusal};
use grit::layout::{Dimension, PARAMS, PIXELS};
use grit::question::{Answer, Layout, Measure, Questions, Reduce, Region};
use grit::rule3::{CORNERS, Oct, Rule};
use grit::sand::Grid;
use grit::sand3::{self, Grid3, reference};
use grit::wire::{self, Json};
use grit::{Groups, Host, Resident, live, sweep};

const MIX: &str = include_str!("law/mix.law.json");

const STIR: &str = include_str!("law/stir.law.json");

const SAND: &str = include_str!("law/sand.law.json");

const HEAT: &str = include_str!("law/heat.law.json");

const CODE: &str = include_str!("law/code.law.json");

const NATURE: &str = include_str!("law/nature.law.json");

const EMBED: &str = include_str!("law/embed.law.json");

const MIX_PARAMS: [u32; 2] = [3_000, 1_700];

const STIR_PARAMS: [u32; 3] = [2_600, 900, 1_500];

const EXPECTED_FORM: &str = "a sequence of lifting steps over the cell's words, 8 a plane: b += f, b -= f or b ^= f, with f free of b";

const RANDOM_OCTS: u32 = 100_000;

const HOME_FRAMES: u32 = 512;

const SIDE: u32 = 32;

const SMALL: u32 = 16;

fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

fn word(state: &mut u64) -> u32 {
    (next(state) >> 16) as u32
}

fn compiled(text: &str) -> Law {
    Law::compile(text).unwrap_or_else(|why| panic!("{why}"))
}

fn reversible(text: &str) -> Reversible {
    Reversible::of(&compiled(text)).unwrap_or_else(|why| panic!("{why}"))
}

fn edited(text: &str, edit: impl FnOnce(&mut Vec<(String, Json)>)) -> String {
    let mut json = wire::read(text).unwrap();
    let Json::Map(fields) = &mut json else {
        panic!("a law is an object")
    };
    edit(fields);
    wire::written(&json)
}

fn marked(text: &str) -> String {
    edited(text, |fields| {
        fields.retain(|(name, _)| name != "reversible");
        fields.push((String::from("reversible"), Json::Bool(true)));
    })
}

fn unmarked(text: &str) -> String {
    edited(text, |fields| {
        fields.retain(|(name, _)| name != "reversible")
    })
}

fn template(step: &str) -> String {
    format!(
        "{{\"law\":1,\"name\":\"probe\",\"params\":1,\"channels\":1,\"halo\":0,\"reversible\":true,\"tally\":{{\"counts\":1,\"weighted\":false}},\"consts\":{{}},\"defs\":{{\"twice\":[[\"w\"],[\"mul\",\"w\",2]],\"chosen\":[[\"a\",\"b\"],[\"sel\",[\"gt\",\"a\",\"b\"],\"a\",\"b\"]]}},\"step\":{step}}}"
    )
}

fn out_of(words: [&str; 8]) -> String {
    format!("[\"vec\",[\"vec\",{}],[\"vec\",0]]", words.join(","))
}

fn cell(k: usize) -> String {
    format!("[\"at\",\"cells\",{k}]")
}

fn refused(text: &str) -> Refusal {
    match Law::compile(text) {
        Ok(_) => panic!("compiled: {text}"),
        Err(why) => why,
    }
}

#[test]
fn the_reversible_key_is_optional_true_only_and_part_of_the_hash() {
    let plain = compiled(&unmarked(MIX));
    assert!(!plain.reversible());
    let mix = compiled(MIX);
    assert!(mix.reversible());
    assert_ne!(plain.hex(), mix.hex());
    assert!(mix.canonical().contains("\"reversible\":true"));
    assert!(!plain.canonical().contains("reversible"));
    let moved = edited(MIX, |fields| fields.reverse());
    assert_eq!(compiled(&moved).hex(), mix.hex());
    assert_eq!(plain.dag(), mix.dag());
    for (value, shown) in [
        (Json::Bool(false), "false"),
        (Json::Int(1), "the number 1"),
        (Json::Null, "null"),
        (Json::Text(String::from("true")), "the text \"true\""),
    ] {
        let text = edited(MIX, |fields| {
            fields.retain(|(name, _)| name != "reversible");
            fields.push((String::from("reversible"), value));
        });
        let why = refused(&text);
        assert_eq!(why.code, Code::Shape, "{why}");
        assert_eq!(why.path, "$.reversible");
        assert_eq!(why.found, shown);
    }
    let unknown = edited(MIX, |fields| {
        fields.push((String::from("reversable"), Json::Bool(true)));
    });
    let why = refused(&unknown);
    assert_eq!(why.code, Code::UnknownKey);
    assert_eq!(why.path, "$.reversable");
    assert!(why.expected.contains("reversible"), "{why}");
    assert!(Code::ALL.contains(&Code::Lifting));
    assert_eq!(Code::Lifting.name(), "lifting");
}

#[test]
fn a_reversible_law_is_a_sequence_of_lifting_steps() {
    let mix = reversible(MIX);
    let words: Vec<(usize, Lifting)> = mix.steps().iter().map(|one| (one.word, one.how)).collect();
    assert_eq!(
        words,
        vec![
            (4, Lifting::Add),
            (5, Lifting::Add),
            (6, Lifting::Add),
            (7, Lifting::Add),
            (0, Lifting::Sub),
            (1, Lifting::Sub),
            (2, Lifting::Sub),
            (3, Lifting::Sub),
        ]
    );
    let stir = reversible(STIR);
    let words: Vec<(usize, Lifting)> = stir.steps().iter().map(|one| (one.word, one.how)).collect();
    assert_eq!(
        words,
        vec![
            (1, Lifting::Add),
            (3, Lifting::Add),
            (5, Lifting::Add),
            (7, Lifting::Add),
            (0, Lifting::Sub),
            (2, Lifting::Sub),
            (4, Lifting::Sub),
            (6, Lifting::Sub),
        ]
    );
    for (name, law) in [("mix", &mix), ("stir", &stir)] {
        println!(
            "{name}: {} steps, forward {} nodes, inverse {} nodes",
            law.steps().len(),
            law.forward().len(),
            law.inverse().len()
        );
        let again = Reversible::of_dag(law.inverse()).unwrap();
        assert_eq!(again.steps().len(), law.steps().len(), "{name} inverse");
    }
    let unmarked = compiled(&unmarked(MIX));
    let why = Reversible::of(&unmarked).unwrap_err();
    assert_eq!(why.code, Code::Missing);
    assert_eq!(why.path, "$.reversible");
}

fn broken_at(step: &str, path: &str, found: &str) -> Refusal {
    let text = template(step);
    let why = refused(&text);
    assert_eq!(why.code, Code::Lifting, "{step}: {why}");
    assert_eq!(why.path, path, "{step}: {why}");
    assert!(why.found.contains(found), "{step}: {why}");
    let law = compiled(&unmarked(&text));
    let again = Reversible::of(&law).unwrap_err();
    assert_eq!(again, why, "{step}: marked and unmarked refuse alike");
    why
}

#[test]
fn a_law_that_breaks_the_form_is_refused_by_name_with_its_first_step() {
    let mut words: [String; 8] = core::array::from_fn(cell);
    let fixed = |words: &[String; 8]| -> [String; 8] { words.clone() };
    let view = |words: &[String; 8]| out_of(core::array::from_fn(|k| words[k].as_str()));

    let mut swapped = fixed(&words);
    swapped.swap(0, 1);
    broken_at(
        &view(&swapped),
        "$.step",
        "word 1 is made by cell 0 unchanged",
    );

    let mut overwritten = fixed(&words);
    overwritten[2] = format!("[\"add\",{},{}]", cell(0), cell(1));
    broken_at(
        &view(&overwritten),
        "$.step[1][3]",
        "word 2 is made by add, and neither side carries the word",
    );

    let mut itself = fixed(&words);
    itself[3] = format!("[\"add\",{},[\"twice\",{}]]", cell(3), cell(3));
    broken_at(
        &view(&itself),
        "$.step[1][4]",
        "the step add on word 3 reads word 3",
    );

    let mut both = fixed(&words);
    both[3] = format!("[\"add\",{},{}]", cell(3), cell(3));
    broken_at(&view(&both), "$.step[1][4]", "reads word 3");

    let mut chosen = fixed(&words);
    chosen[5] = format!(
        "[\"add\",[\"chosen\",{},{}],[\"param\",0]]",
        cell(5),
        cell(6)
    );
    broken_at(
        &view(&chosen),
        "$.defs.chosen[1]",
        "word 5 is made by sel, which is not add",
    );

    let mut scaled = fixed(&words);
    scaled[6] = format!("[\"twice\",{}]", cell(6));
    broken_at(&view(&scaled), "$.defs.twice[1]", "word 6 is made by mul");

    let tangled = format!(
        "[\"let\",[[\"a\",[\"add\",{},{}]],[\"b\",[\"add\",{},{}]]],{}]",
        cell(0),
        cell(1),
        cell(1),
        cell(0),
        out_of([
            "\"a\"",
            "\"b\"",
            &cell(2),
            &cell(3),
            &cell(4),
            &cell(5),
            &cell(6),
            &cell(7)
        ])
    );
    broken_at(
        &tangled,
        "$.step[1][0][1]",
        "the step add on word 0 reads a word that is changed both before and after it",
    );

    let shared = format!(
        "[\"let\",[[\"a\",[\"add\",{},{}]]],{}]",
        cell(0),
        cell(1),
        out_of([
            "\"a\"",
            "\"a\"",
            &cell(2),
            &cell(3),
            &cell(4),
            &cell(5),
            &cell(6),
            &cell(7)
        ])
    );
    broken_at(
        &shared,
        "$.step[1][0][1]",
        "words 0 and 1 are both made by the one step add",
    );

    words[7] = format!("[\"xor\",{},[\"add\",{},[\"noise\"]]]", cell(7), cell(0));
    let fine = template(&view(&words));
    let law = compiled(&fine);
    let steps = Reversible::of(&law).unwrap();
    assert_eq!(steps.steps().len(), 1);
    assert_eq!(steps.steps()[0].how, Lifting::Xor);
    let ordered = format!(
        "[\"let\",[[\"b\",[\"add\",{},{}]],[\"a\",[\"sub\",{},\"b\"]]],{}]",
        cell(1),
        cell(0),
        cell(0),
        out_of([
            "\"a\"",
            "\"b\"",
            &cell(2),
            &cell(3),
            &cell(4),
            &cell(5),
            &cell(6),
            &cell(7)
        ])
    );
    let law = compiled(&template(&ordered));
    let steps = Reversible::of(&law).unwrap();
    let order: Vec<usize> = steps.steps().iter().map(|one| one.word).collect();
    assert_eq!(order, vec![1, 0]);
}

struct Rules {
    name: &'static str,
    text: &'static str,
    path: &'static str,
}

const RULES: [Rules; 5] = [
    Rules {
        name: "sand",
        text: SAND,
        path: "$.defs.traded[1]",
    },
    Rules {
        name: "heat",
        text: HEAT,
        path: "$.defs.moved[1][5][2][1][3]",
    },
    Rules {
        name: "code",
        text: CODE,
        path: "$.defs.settled[1][2][5][2]",
    },
    Rules {
        name: "nature",
        text: NATURE,
        path: "$.defs.sunned[1][2][5][1][2][1]",
    },
    Rules {
        name: "embed",
        text: EMBED,
        path: "$.defs.swapped[1][1][3]",
    },
];

#[test]
fn the_five_rules_are_refused_for_gradients_by_name_with_their_first_offending_step() {
    let refusals: Vec<Refusal> = RULES
        .iter()
        .map(|rule| {
            let law = compiled(rule.text);
            assert!(!law.reversible(), "{}", rule.name);
            let why = Reversible::of(&law).unwrap_err();
            println!(
                "{}: {} | {} | {}",
                rule.name,
                why.code.name(),
                why.path,
                why.found
            );
            why
        })
        .collect();
    for (rule, why) in RULES.iter().zip(refusals) {
        assert_eq!(why.code, Code::Lifting, "{}", rule.name);
        assert_eq!(why.path, rule.path, "{}", rule.name);
        assert_eq!(
            why.found, "word 0 is made by sel, which is not add, sub or xor of the word",
            "{}",
            rule.name
        );
        assert_eq!(why.expected, EXPECTED_FORM);
        let again = refused(&marked(rule.text));
        assert_eq!(again, why, "{} marked reversible", rule.name);
    }
}

#[derive(Clone)]
struct Case {
    words: [u32; CORNERS],
    frame: u32,
    at: [u32; 3],
    params: Vec<u32>,
}

fn literals(json: &Json, out: &mut BTreeSet<u32>) {
    match json {
        Json::Int(held) => {
            if let Ok(value) = u32::try_from(*held) {
                out.insert(value);
            }
        }
        Json::List(items) => items.iter().for_each(|item| literals(item, out)),
        Json::Map(fields) => fields.iter().for_each(|(_, item)| literals(item, out)),
        _ => {}
    }
}

fn edges(text: &str, params: &[u32]) -> Vec<u32> {
    let mut held = BTreeSet::new();
    literals(&wire::read(text).unwrap(), &mut held);
    held.extend(params);
    held.extend([0, 1 << 16, 1 << 20, 1 << 31, u32::MAX]);
    let mut out = BTreeSet::new();
    for value in held {
        out.extend([value.wrapping_sub(1), value, value.wrapping_add(1)]);
    }
    out.into_iter().collect()
}

fn cases(text: &str, params: &[u32], many: u32, seed: u64) -> Vec<Case> {
    let edges = edges(text, params);
    let mut state = seed;
    let mut out = Vec::new();
    for _ in 0..many {
        out.push(Case {
            words: core::array::from_fn(|_| word(&mut state)),
            frame: word(&mut state),
            at: core::array::from_fn(|_| word(&mut state)),
            params: params
                .iter()
                .map(|&one| {
                    if next(&mut state).is_multiple_of(2) {
                        one
                    } else {
                        word(&mut state)
                    }
                })
                .collect(),
        });
    }
    let pick = |state: &mut u64| edges[(next(state) % edges.len() as u64) as usize];
    for &edge in &edges {
        for slot in 0..CORNERS + params.len() {
            for _ in 0..2 {
                let mut words: [u32; CORNERS] = core::array::from_fn(|_| pick(&mut state));
                let mut held: Vec<u32> = params.iter().map(|_| pick(&mut state)).collect();
                match slot.checked_sub(CORNERS) {
                    None => words[slot] = edge,
                    Some(param) => held[param] = edge,
                }
                out.push(Case {
                    words,
                    frame: pick(&mut state),
                    at: core::array::from_fn(|_| pick(&mut state)),
                    params: held,
                });
            }
        }
    }
    out
}

fn stepped<R: Rule>(rule: &R, case: &Case, words: [u32; CORNERS]) -> [u32; CORNERS] {
    rule.step(Oct {
        words,
        frame: case.frame,
        at: case.at,
        params: &case.params,
    })
    .out
}

fn inverse_holds(name: &str, law: &Reversible, text: &str, params: &[u32]) -> usize {
    let all = cases(text, params, RANDOM_OCTS, 0xA1_0000 ^ name.len() as u64);
    let mut moved = 0;
    for case in &all {
        let out = stepped(law.forward(), case, case.words);
        let back = stepped(law.inverse(), case, out);
        assert_eq!(back, case.words, "{name}: inverse(step(oct))");
        let ahead = stepped(
            law.forward(),
            case,
            stepped(law.inverse(), case, case.words),
        );
        assert_eq!(ahead, case.words, "{name}: step(inverse(oct))");
        moved += usize::from(out != case.words);
    }
    println!(
        "{name}: inverse(step(oct)) == oct and step(inverse(oct)) == oct on {} octs ({} random), {moved} moved",
        all.len(),
        RANDOM_OCTS
    );
    assert!(moved * 2 > all.len(), "{name}: too few octs moved");
    all.len()
}

#[test]
fn the_inverse_undoes_every_step_on_the_cpu() {
    inverse_holds("mix", &reversible(MIX), MIX, &MIX_PARAMS);
    inverse_holds("stir", &reversible(STIR), STIR, &STIR_PARAMS);
}

fn grids(side: u32) -> (Grid, Grid3) {
    (
        Grid::solid(side, side, side).unwrap(),
        Grid3::new(side, side, side).unwrap(),
    )
}

struct Walker<'host> {
    live: Resident<'host>,
    ahead: Vec<Vec<Groups>>,
    back: Vec<Vec<Groups>>,
    grid: Grid,
    plan: Plan,
    bricks: Option<Bricks>,
    questions: Questions,
    first_pass: usize,
    differ: grit::question::Asked,
}

fn quiet(many: usize) -> Vec<Groups> {
    vec![Groups::Fixed(0); many]
}

impl<'host> Walker<'host> {
    fn open(host: &'host Host, law: &Reversible, side: u32, sparse: bool) -> Self {
        let width = host.width();
        let (grid, _) = grids(side);
        let blocks = grid.blocks().div_ceil(sweep::cells(width));
        let forward = law.forward();
        let inverse = law.inverse();
        let (plan, bricks, passes, ahead, back, bindings) = if sparse {
            let plan = Plan::bricked(width, blocks).unwrap();
            let bricks = Bricks::new(grid, &plan, plan.blocks).unwrap();
            let (mut passes, onward) = bricks::whole(forward, &bricks, &plan).unwrap();
            let (undo, backward) = bricks::whole(inverse, &bricks, &plan).unwrap();
            let mut ahead = onward.clone();
            ahead.push(quiet(undo.len()));
            let mut back = vec![quiet(passes.len())];
            back.extend(backward);
            passes.extend(undo);
            let sizes = bricks.sizes(forward, &plan);
            (plan, Some(bricks), passes, ahead, back, sizes)
        } else {
            let plan = Plan::over_in(width, blocks, Dimension::Three).unwrap();
            let groups = host.limits().groups;
            let onward = sand3::settle_by(forward, width, grid, groups).unwrap();
            let undo = sand3::settle_by(inverse, width, grid, groups).unwrap();
            let passes = vec![live::tick(width).unwrap(), onward.words, undo.words];
            let ahead = vec![
                vec![Groups::Fixed(1)],
                vec![onward.spread.dispatch(), Groups::Fixed(0)],
            ];
            let back = vec![
                vec![Groups::Fixed(1)],
                vec![Groups::Fixed(0), undo.spread.dispatch()],
            ];
            let sizes = live::sizes3(forward, &plan).to_vec();
            (plan, None, passes, ahead, back, sizes)
        };
        let layout = match bricks {
            Some(bricks) => Layout::Sparse(bricks),
            None => Layout::Dense {
                grid,
                windows: plan.windows(),
            },
        };
        let mut questions = Questions::new(layout, width, bindings.len(), grid.cells()).unwrap();
        let differ = questions.add(&Plain, Reduce::Differ(Measure::Sum)).unwrap();
        let mut sizes = bindings.clone();
        sizes.extend(questions.sizes());
        let mut modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
        let first_pass = modules.len();
        modules.extend(questions.modules());
        let live = host.resident(&sizes, &modules).unwrap();
        let mut ahead = ahead;
        ahead.push(questions.resting());
        let mut back = back;
        back.push(questions.resting());
        let mut walker = Walker {
            live,
            ahead,
            back,
            grid,
            plan,
            bricks,
            questions,
            first_pass,
            differ,
        };
        for (binding, &room) in sizes.iter().enumerate() {
            if binding != PIXELS as usize {
                walker.live.write(binding, &vec![0; room]).unwrap();
            }
        }
        walker
    }

    fn start(&mut self, cells: &[u32], params: &[u32], first: u32) {
        if !params.is_empty() {
            self.live
                .write_at(PARAMS as usize, grit::rule3::FIRST_PARAM, params)
                .unwrap();
        }
        match self.bricks {
            Some(bricks) => {
                self.live
                    .write(PARAMS as usize, &[first.wrapping_sub(1)])
                    .unwrap();
                let packed = bricks.pack(cells).unwrap();
                bricks
                    .write_packed(&mut self.live, &self.plan, &packed)
                    .unwrap();
                bricks.seed(&mut self.live, &self.plan).unwrap();
            }
            None => {
                self.live
                    .write(PARAMS as usize, &[first.wrapping_sub(1), self.plan.dump()])
                    .unwrap();
                live::write_state(&mut self.live, self.plan.windows(), 0, cells).unwrap();
            }
        }
        let whole = Region::whole([self.grid.wide, self.grid.deep, self.grid.tall]);
        self.questions.aim(&mut self.live, whole, cells).unwrap();
    }

    fn ahead(&mut self, frames: u32) {
        let groups: Vec<&[Groups]> = self.ahead.iter().map(Vec::as_slice).collect();
        self.live.staged_frames(&groups, frames).unwrap();
    }

    fn back_one(&mut self, frame: u32) {
        self.live
            .write_at(PARAMS as usize, 0, &[frame.wrapping_sub(1)])
            .unwrap();
        let groups: Vec<&[Groups]> = self.back.iter().map(Vec::as_slice).collect();
        self.live.staged(&groups).unwrap();
    }

    fn cells(&mut self) -> Vec<u32> {
        match self.bricks {
            Some(bricks) => bricks.read(&mut self.live).unwrap(),
            None => {
                live::read_state(&mut self.live, self.plan.windows(), self.grid.cells()).unwrap()
            }
        }
    }

    fn differ(&mut self) -> u64 {
        let whole = Region::whole([self.grid.wide, self.grid.deep, self.grid.tall]);
        match self
            .questions
            .ask(&mut self.live, self.first_pass, self.differ, whole)
            .unwrap()
        {
            Answer::Differ(total) => total,
            other => panic!("{other:?}"),
        }
    }
}

fn on_device(host: &Host, name: &str, law: &Reversible, text: &str, params: &[u32]) {
    let (_, reference_grid) = grids(SIDE);
    let mut walker = Walker::open(host, law, SIDE, false);
    let edges = edges(text, params);
    let mut state = 0xD1CE_u64 ^ name.len() as u64;
    let mut octs = 0;
    let mut round = 0_u32;
    while octs < 2 * RANDOM_OCTS {
        let edgy = round % 2 == 1;
        let pick = |state: &mut u64| {
            if edgy {
                edges[(next(state) % edges.len() as u64) as usize]
            } else {
                word(state)
            }
        };
        let start: Vec<u32> = (0..reference_grid.cells())
            .map(|_| pick(&mut state))
            .collect();
        let held: Vec<u32> = params.iter().map(|_| pick(&mut state)).collect();
        let frame = word(&mut state);
        walker.start(&start, &held, frame);
        walker.ahead(1);
        let got = walker.cells();
        let wanted = reference(law.forward(), reference_grid, frame, &start, &held);
        assert_eq!(got, wanted, "{name} round {round}: step on {}", host.name());
        assert_ne!(got, start, "{name} round {round}: nothing moved");
        walker.back_one(frame);
        let home = walker.cells();
        let differ = home.iter().zip(&start).filter(|(a, b)| a != b).count();
        assert_eq!(
            differ,
            0,
            "{name} round {round}: {differ} cells strayed on {}",
            host.name()
        );
        octs += reference_grid.octs();
        round += 1;
    }
    println!(
        "{name} on {} width {}: {octs} octs in {round} worlds, half of them of constants and params ±1: device step equals the CPU, inverse(step) == oct",
        host.name(),
        host.width()
    );
}

#[test]
fn the_inverse_undoes_every_step_on_the_device() {
    let Some(host) = device("adjoint3 inverse") else {
        return;
    };
    on_device(&host, "mix", &reversible(MIX), MIX, &MIX_PARAMS);
    on_device(&host, "stir", &reversible(STIR), STIR, &STIR_PARAMS);
}

fn blob(grid: Grid3, seed: u64, low: u32, high: u32) -> Vec<u32> {
    let mut state = seed;
    let mut cells = vec![0_u32; grid.cells()];
    let wide = grid.wide;
    let quarter = wide / 4;
    for z in quarter..wide - quarter {
        for y in quarter..wide - quarter {
            for x in quarter..wide - quarter {
                cells[grid.index(x, y, z)] = low + word(&mut state) % (high - low);
            }
        }
    }
    for _ in 0..grid.cells() / 64 {
        let at = (next(&mut state) % grid.cells() as u64) as usize;
        cells[at] = low + word(&mut state) % (high - low);
    }
    cells
}

fn walked_home(name: &str, law: &Reversible, params: &[u32], seed: u64) -> Vec<u32> {
    let grid = Grid3::new(SIDE, SIDE, SIDE).unwrap();
    let start = blob(grid, seed, 1, 1 << 24);
    let aim = Aim {
        region: Region::whole([SIDE, SIDE, SIDE]),
        target: &start,
    };
    let walked = law
        .walk(grid, &start, params, 0, HOME_FRAMES, &aim)
        .unwrap_or_else(|why| panic!("{name}: {why}"));
    assert!(walked.objective > 0, "{name}: nothing moved");
    println!(
        "{name}: came home on the CPU after {HOME_FRAMES} frames out and back, {} cells moved on the way, objective {}",
        walked
            .ended
            .iter()
            .zip(&start)
            .filter(|(a, b)| a != b)
            .count(),
        walked.objective
    );
    start
}

fn device_home(host: &Host, name: &str, law: &Reversible, params: &[u32], start: &[u32]) {
    let (_, reference_grid) = grids(SIDE);
    let mut checkpoints = vec![start.to_vec()];
    let mut cells = start.to_vec();
    for frame in 0..HOME_FRAMES {
        cells = reference(law.forward(), reference_grid, frame, &cells, params);
        if (frame + 1) % 64 == 0 {
            checkpoints.push(cells.clone());
        }
    }
    let aim = Aim {
        region: Region::whole([SIDE, SIDE, SIDE]),
        target: start,
    };
    for sparse in [false, true] {
        let mut walker = Walker::open(host, law, SIDE, sparse);
        walker.start(start, params, 0);
        walker.ahead(HOME_FRAMES);
        let ended = walker.cells();
        assert_eq!(
            &ended,
            checkpoints.last().unwrap(),
            "{name} sparse {sparse}: forward"
        );
        let away = walker.differ();
        assert_eq!(
            away,
            aim.measured(reference_grid, &ended),
            "{name} sparse {sparse}: question"
        );
        assert!(away > 0);
        for frame in (0..HOME_FRAMES).rev() {
            walker.back_one(frame);
            if frame % 64 == 0 {
                let got = walker.cells();
                let wanted = &checkpoints[(frame / 64) as usize];
                let strayed = got.iter().zip(wanted).filter(|(a, b)| a != b).count();
                assert_eq!(
                    strayed,
                    0,
                    "{name} sparse {sparse}: {strayed} cells strayed at frame {frame} on {}",
                    host.name()
                );
            }
        }
        assert_eq!(walker.differ(), 0, "{name} sparse {sparse}: not home");
        println!(
            "{name} sparse {sparse} on {} width {}: {HOME_FRAMES} frames out and back, home; difference to the start {away} away and 0 home",
            host.name(),
            host.width()
        );
    }
}

#[test]
fn a_reversible_law_comes_home() {
    let mix = reversible(MIX);
    let stir = reversible(STIR);
    let mixed = walked_home("mix", &mix, &MIX_PARAMS, 0x3D_0001);
    let stirred = walked_home("stir", &stir, &STIR_PARAMS, 0x3D_0002);
    let Some(host) = device("adjoint3 home") else {
        return;
    };
    device_home(&host, "mix", &mix, &MIX_PARAMS, &mixed);
    device_home(&host, "stir", &stir, &STIR_PARAMS, &stirred);
}

#[test]
fn a_walk_refuses_a_world_params_or_box_that_do_not_fit() {
    let mix = reversible(MIX);
    let grid = Grid3::new(SMALL, SMALL, SMALL).unwrap();
    let start = blob(grid, 7, 1, 1 << 24);
    let aim = Aim {
        region: Region::whole([SMALL, SMALL, SMALL]),
        target: &start,
    };
    assert!(mix.walk(grid, &start, &MIX_PARAMS, 0, 4, &aim).is_ok());
    let short = &start[1..];
    assert_eq!(
        mix.walk(grid, short, &MIX_PARAMS, 0, 4, &aim),
        Err(Fault::Cells {
            given: grid.cells() - 1,
            wanted: grid.cells()
        })
    );
    assert_eq!(
        mix.walk(grid, &start, &MIX_PARAMS[..1], 0, 4, &aim),
        Err(Fault::Params {
            given: 1,
            wanted: 2
        })
    );
    let outside = Aim {
        region: Region::new([0, 0, 0], [SMALL + 1, 1, 1]),
        target: &start[..(SMALL as usize + 1)],
    };
    assert!(matches!(
        mix.walk(grid, &start, &MIX_PARAMS, 0, 4, &outside),
        Err(Fault::Region { .. })
    ));
    let short_target = Aim {
        region: Region::whole([SMALL, SMALL, SMALL]),
        target: &start[1..],
    };
    assert!(matches!(
        mix.walk(grid, &start, &MIX_PARAMS, 0, 4, &short_target),
        Err(Fault::Target { .. })
    ));
}

const CALM: u32 = 1 << 22;

fn calm(grid: Grid3, seed: u64) -> Vec<u32> {
    blob(grid, seed, CALM - (1 << 21), CALM + (1 << 21))
        .into_iter()
        .map(|one| if one == 0 { CALM } else { one })
        .collect()
}

fn ran(law: &Reversible, grid: Grid3, start: &[u32], params: &[u32], frames: u32) -> Vec<u32> {
    let mut cells = start.to_vec();
    for frame in 0..frames {
        cells = reference(law.forward(), grid, frame, &cells, params);
    }
    cells
}

const GRADIENT_FRAMES: u32 = 16;

const STEP: u32 = 4;

const TOLERANCE_PERCENT: f64 = 1.0;

fn objective(law: &Reversible, grid: Grid3, start: &[u32], params: &[u32], aim: &Aim<'_>) -> u64 {
    aim.measured(grid, &ran(law, grid, start, params, GRADIENT_FRAMES))
}

fn agrees(name: &str, law: &Reversible, truth: &[u32]) -> f64 {
    let grid = Grid3::new(SMALL, SMALL, SMALL).unwrap();
    let mut worst = 0.0_f64;
    for seed in [11_u64, 12] {
        let start = calm(grid, seed);
        let target = ran(law, grid, &start, truth, GRADIENT_FRAMES);
        let aim = Aim {
            region: Region::whole([SMALL, SMALL, SMALL]),
            target: &target,
        };
        for scale in [115_u32, 85] {
            let params: Vec<u32> = truth.iter().map(|one| one * scale / 100).collect();
            let walked = law
                .walk(grid, &start, &params, 0, GRADIENT_FRAMES, &aim)
                .unwrap();
            assert_eq!(
                walked.ended,
                ran(law, grid, &start, &params, GRADIENT_FRAMES)
            );
            assert_eq!(
                walked.objective,
                objective(law, grid, &start, &params, &aim)
            );
            assert!(walked.objective > 0);
            for (index, &told) in walked.told.iter().enumerate() {
                let mut up = params.clone();
                up[index] += STEP;
                let mut down = params.clone();
                down[index] -= STEP;
                let rise = i128::from(objective(law, grid, &start, &up, &aim))
                    - i128::from(objective(law, grid, &start, &down, &aim));
                let differenced = rise as f64 / f64::from(2 * STEP);
                let slope = told as f64 / 2_f64.powi(adjoint::FRACTION as i32);
                let off = 100.0 * (slope - differenced).abs() / differenced.abs();
                println!(
                    "{name} seed {seed} params {params:?} param {index}: adjoint {slope:.2}, finite difference {differenced:.2}, {off:.3} % apart"
                );
                assert_ne!(rise, 0, "{name} param {index}: flat");
                assert_eq!(
                    slope.is_sign_positive(),
                    differenced.is_sign_positive(),
                    "{name} seed {seed} param {index}: sign"
                );
                assert!(
                    off <= TOLERANCE_PERCENT,
                    "{name} seed {seed} param {index}: {off} % apart"
                );
                worst = worst.max(off);
            }
        }
    }
    worst
}

#[test]
fn the_adjoint_agrees_with_finite_differences_for_every_param() {
    let mix = agrees("mix", &reversible(MIX), &MIX_PARAMS);
    let stir = agrees("stir", &reversible(STIR), &STIR_PARAMS);
    println!(
        "{GRADIENT_FRAMES} frames, step {STEP}: the adjoint is at most {mix:.3} % (mix) and {stir:.3} % (stir) from the central finite difference"
    );
}

const DESCENT_SEEDS: u64 = 8;

const MOST_STEPS: u32 = 10;

fn descended(law: &Reversible, truth: &[u32], seed: u64) -> adjoint::Descent {
    let grid = Grid3::new(SMALL, SMALL, SMALL).unwrap();
    let start = calm(grid, 100 + seed);
    let target = ran(law, grid, &start, truth, GRADIENT_FRAMES);
    let posed = Posed {
        grid,
        start: &start,
        first: 0,
        frames: GRADIENT_FRAMES,
        aim: Aim {
            region: Region::whole([SMALL, SMALL, SMALL]),
            target: &target,
        },
    };
    let mut state = seed.wrapping_mul(7919);
    let from: Vec<u32> = truth
        .iter()
        .map(|&one| {
            let off = i64::from(word(&mut state) % 61) - 30;
            u32::try_from(i64::from(one) * (100 + off) / 100).unwrap()
        })
        .collect();
    law.descend(&posed, &from, MOST_STEPS).unwrap()
}

#[test]
fn gradient_steps_recover_the_params_that_made_a_target() {
    for (name, text, truth) in [
        ("mix", MIX, MIX_PARAMS.to_vec()),
        ("stir", STIR, STIR_PARAMS.to_vec()),
    ] {
        let law = reversible(text);
        let mut most = 0;
        for seed in 1..=DESCENT_SEEDS {
            let descent = descended(&law, &truth, seed);
            let (from, away) = descent.trail[0].clone();
            println!(
                "{name} seed {seed}: from {from:?} (objective {away}) to {:?} in {} steps, objective {}",
                descent.params, descent.steps, descent.objective
            );
            assert_eq!(
                descent.objective, 0,
                "{name} seed {seed}: {:?}",
                descent.trail
            );
            assert_eq!(descent.params, truth, "{name} seed {seed}");
            assert_ne!(from, truth, "{name} seed {seed}: started right");
            assert!(descent.steps <= MOST_STEPS);
            assert!(
                descent.trail.windows(2).all(|pair| pair[0].0 != pair[1].0),
                "{name} seed {seed}: a step that did not move"
            );
            assert_eq!(
                descended(&law, &truth, seed),
                descent,
                "{name} seed {seed}: replay"
            );
            most = most.max(descent.steps);
        }
        println!("{name}: {DESCENT_SEEDS} seeds, every one recovered in at most {most} steps");
    }
}

fn sites(json: &Json, path: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
    out.push(path.clone());
    match json {
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

fn site<'a>(json: &'a mut Json, path: &[usize]) -> &'a mut Json {
    path.iter().fold(json, |here, &at| match here {
        Json::List(items) => &mut items[at],
        Json::Map(fields) => &mut fields[at].1,
        other => other,
    })
}

const FUZZ: u32 = 20_000;

const SWAPS: [&str; 12] = [
    "add", "sub", "xor", "mul", "and", "or", "shr", "shl", "k", "d", "c", "cells",
];

fn mutated(text: &str, state: &mut u64) -> String {
    let mut json = wire::read(text).unwrap();
    let Json::Map(fields) = &json else {
        panic!("a law is an object")
    };
    let mut all = Vec::new();
    for (at, (name, held)) in fields.iter().enumerate() {
        if name == "defs" || name == "step" {
            sites(held, &mut vec![at], &mut all);
        }
    }
    let path = all[(next(state) % all.len() as u64) as usize].clone();
    let here = site(&mut json, &path);
    *here = match here.clone() {
        Json::Text(_) => Json::Text(SWAPS[(next(state) % SWAPS.len() as u64) as usize].to_owned()),
        Json::Int(_) => Json::Int((next(state) % 9) as i64),
        Json::List(mut items) if items.len() > 2 => {
            let from = 1 + (next(state) % (items.len() as u64 - 1)) as usize;
            let to = 1 + (next(state) % (items.len() as u64 - 1)) as usize;
            items.swap(from, to);
            Json::List(items)
        }
        other => other,
    };
    wire::written(&json)
}

fn fuzzed(text: &str, state: &mut u64) -> String {
    let mut made = text.to_owned();
    for _ in 0..1 + next(state) % 3 {
        made = mutated(&made, state);
    }
    made
}

#[test]
fn every_law_the_check_admits_is_undone_by_its_inverse_on_a_seeded_fuzz() {
    let mut state = 0xF0_22ED_u64;
    let mut admitted = 0;
    let mut codes = BTreeSet::new();
    for round in 0..FUZZ {
        let (base, params): (&str, &[u32]) = if round % 2 == 0 {
            (MIX, &MIX_PARAMS)
        } else {
            (STIR, &STIR_PARAMS)
        };
        let text = fuzzed(base, &mut state);
        let law = match Law::compile(&text) {
            Ok(law) => law,
            Err(why) => {
                assert!(Code::ALL.contains(&why.code));
                codes.insert(why.code.name());
                continue;
            }
        };
        let law = Reversible::of(&law).unwrap_or_else(|why| panic!("{why}: {text}"));
        admitted += 1;
        for case in cases(&text, params, 64, u64::from(round)).iter().take(64) {
            let out = stepped(law.forward(), case, case.words);
            assert_eq!(stepped(law.inverse(), case, out), case.words, "{text}");
        }
    }
    println!(
        "{FUZZ} mutated laws: {admitted} admitted and undone by their inverse, refusals {codes:?}"
    );
    assert!(admitted > 0);
    assert!(codes.contains("lifting"));
}

fn lifting_law(defs: &str, step: &str) -> String {
    format!(
        "{{\"law\":1,\"name\":\"probe\",\"params\":0,\"channels\":1,\"halo\":0,\"reversible\":true,\"tally\":{{\"counts\":1,\"weighted\":false}},\"consts\":{{}},\"defs\":{{{defs}}},\"step\":{step}}}"
    )
}

fn third(word: &str) -> String {
    let cells: Vec<String> = (0..8)
        .map(|k| if k == 3 { word.to_owned() } else { cell(k) })
        .collect();
    format!("[\"vec\",[\"vec\",{}],[\"vec\",0]]", cells.join(","))
}

fn padded(items: u64) -> String {
    format!(
        "[\"let\",[[\"junk\",[\"range\",{items}]]],{}]",
        third("[\"mul\",[\"at\",\"cells\",3],2]")
    )
}

fn chain(defs: usize) -> String {
    let mut all: Vec<String> = (0..defs)
        .map(|at| format!("\"d{at}\":[[\"v\"],[\"d{}\",\"v\"]]", at + 1))
        .collect();
    all.push(format!("\"d{defs}\":[[\"v\"],[\"mul\",\"v\",2]]"));
    all.join(",")
}

fn lifting_refused(text: &str) -> Refusal {
    let why = refused(text);
    assert_eq!(why.code, Code::Lifting, "{why}");
    assert_eq!(
        why.found,
        "word 3 is made by mul, which is not add, sub or xor of the word"
    );
    why
}

#[test]
fn a_lifting_refusal_names_its_step_exactly_at_the_work_and_depth_limits() {
    let plain = lifting_refused(&lifting_law("", &third("[\"mul\",[\"at\",\"cells\",3],2]")));
    assert_eq!(plain.path, "$.step[1][4]");
    let fixed = compiled(&unmarked(&lifting_law("", &padded(0)))).work();
    let most = grit::law::limits::WORK - fixed;
    assert_eq!(
        compiled(&unmarked(&lifting_law("", &padded(most)))).work(),
        grit::law::limits::WORK
    );
    let at_work = lifting_refused(&lifting_law("", &padded(most)));
    assert_eq!(at_work.path, "$.step[2][1][4]");
    let past = refused(&lifting_law("", &padded(most + 1)));
    assert_eq!(past.code, Code::Work, "{past}");
    let called = third("[\"d0\",[\"at\",\"cells\",3]]");
    let depth_of = |defs: usize| {
        let text = unmarked(&lifting_law(&chain(defs), &called));
        Law::compile(&text).map(|law| law.depth())
    };
    let mut defs = 1;
    while depth_of(defs + 1).is_ok() {
        defs += 1;
    }
    assert_eq!(depth_of(defs).unwrap(), grit::law::limits::DEPTH);
    let at_depth = lifting_refused(&lifting_law(&chain(defs), &called));
    assert_eq!(at_depth.path, format!("$.defs.d{defs}[1]"));
    let law = compiled(&unmarked(&lifting_law(&chain(defs), &called)));
    assert_eq!(Reversible::of(&law).unwrap_err(), at_depth);
    assert_eq!(depth_of(defs + 1).unwrap_err().code, Code::Depth);
}

fn located(json: &Json, path: &str) -> Option<Json> {
    let mut here = json.clone();
    let mut rest = path.strip_prefix('$')?;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('.') {
            let end = after.find(['.', '[']).unwrap_or(after.len());
            here = here.get(&after[..end])?.clone();
            rest = &after[end..];
        } else {
            let after = rest.strip_prefix('[')?;
            let end = after.find(']')?;
            let index: usize = after[..end].parse().ok()?;
            here = here.list()?.get(index)?.clone();
            rest = &after[end + 1..];
        }
    }
    Some(here)
}

#[test]
fn every_node_a_law_emits_knows_the_expression_that_emitted_it() {
    for (name, text) in [
        ("sand", SAND),
        ("heat", HEAT),
        ("code", CODE),
        ("nature", NATURE),
        ("embed", EMBED),
        ("mix", MIX),
        ("stir", STIR),
    ] {
        let law = compiled(text);
        let json = wire::read(text).unwrap();
        let first = law.dag().first_plane() as usize;
        let mut ops = BTreeSet::new();
        for (index, node) in law.dag().nodes().iter().enumerate() {
            let path = law.places().path(index as u32);
            if matches!(node, grit::law::Node::Input(_)) {
                assert_eq!(path, None, "{name} input {index}");
                assert!(
                    index < first + CORNERS * law.dag().planes().len(),
                    "{name} input {index}"
                );
                continue;
            }
            let path = path.unwrap_or_else(|| panic!("{name}: node {index} has no path"));
            let expression = located(&json, &path)
                .unwrap_or_else(|| panic!("{name}: node {index} at {path} is not in the law"));
            let Some(Json::Text(op)) = expression.list().and_then(|items| items.first()) else {
                panic!("{name}: node {index} at {path} is not an operation");
            };
            ops.insert(op.clone());
            let emits: &[&str] = match node {
                grit::law::Node::Input(_) => &[],
                grit::law::Node::Const(_) => &[],
                grit::law::Node::Select(..) => &["sel", "noise"],
                grit::law::Node::Eq(..) => &["eq"],
                grit::law::Node::Ne(..) => &["ne"],
                grit::law::Node::Lt(..) => &["lt"],
                grit::law::Node::Le(..) => &["le"],
                grit::law::Node::Gt(..) => &["gt"],
                grit::law::Node::Ge(..) => &["ge"],
                grit::law::Node::And(..) => &["and", "both", "noise"],
                grit::law::Node::Or(..) => &["or", "either", "noise"],
                grit::law::Node::Xor(..) => &["xor", "denied", "noise"],
                grit::law::Node::Add(..) => &["add", "count", "noise"],
                grit::law::Node::Not(..) => &["not", "noise"],
                grit::law::Node::Shl(..) => &["shl", "noise"],
                grit::law::Node::Shr(..) => &["shr", "noise"],
                grit::law::Node::Sub(..) => &["sub", "noise"],
                grit::law::Node::Mul(..) => &["mul", "noise"],
                grit::law::Node::Div(..) => &["div", "noise"],
                grit::law::Node::DotS(..) => &["dot_s"],
                grit::law::Node::DotU(..) => &["dot_u"],
            };
            if !emits.is_empty() {
                assert!(
                    emits.contains(&op.as_str()),
                    "{name}: node {index} {node:?} at {path} is a {op}"
                );
            }
        }
        println!("{name}: {} nodes, emitted by {ops:?}", law.nodes());
    }
}

fn shared(lifts: u32, chain: u32) -> String {
    let words: Vec<String> = (0..8)
        .map(|k| match k {
            0 => format!(
                "[\"fold\",\"w\",[\"at\",\"cells\",0],\"i\",[\"range\",{lifts}],[\"add\",\"w\",\"b\"]]"
            ),
            _ => cell(k),
        })
        .collect();
    lifting_law(
        "",
        &format!(
            "[\"let\",[[\"b\",[\"fold\",\"a\",[\"at\",\"cells\",1],\"i\",[\"range\",{chain}],[\"not\",\"a\"]]]],[\"vec\",[\"vec\",{}],[\"vec\",0]]]",
            words.join(",")
        ),
    )
}

#[test]
fn a_derived_inverse_is_no_larger_than_its_law() {
    for (lifts, chain) in [(500, 4_000), (2_000, 4_000), (8_000, 8_000)] {
        let text = shared(lifts, chain);
        let law = reversible(&text);
        println!(
            "{lifts} lifts reading a chain of {chain} on another word: forward {} nodes, inverse {} nodes",
            law.forward().len(),
            law.inverse().len()
        );
        assert!(
            law.inverse().len() <= law.forward().len() + law.steps().len(),
            "{lifts} lifts: inverse {} nodes for a forward of {}",
            law.inverse().len(),
            law.forward().len()
        );
        let mut state = 0xC1F_u64 + u64::from(lifts);
        for _ in 0..1_000 {
            let words: [u32; CORNERS] = core::array::from_fn(|_| word(&mut state));
            let oct = |words| Oct {
                words,
                frame: 3,
                at: [1, 2, 3],
                params: &[],
            };
            let out = law.forward().step(oct(words)).out;
            assert_eq!(law.inverse().step(oct(out)).out, words, "{lifts} lifts");
        }
    }
}

const LAYERS: &str = r#"{"law":1,"name":"layers","params":2,"channels":2,"halo":0,"reversible":true,
"tally":{"counts":1,"weighted":false},
"consts":{"DROP":4,"POINT":12,"FINE":16,"BELOW":4},
"defs":{
"share":[["w","by"],["shr",["mul",["shr","w","DROP"],"by"],"POINT"]],
"nudge":[["w","by"],["shr",["mul",["shr","w","DROP"],"by"],"FINE"]],
"low":[["c","k"],["at",["at","c","k"],0]],
"high":[["c","k"],["at",["at","c","k"],1]],
"set":[["c","k","w","v"],["put","c","k",["vec","w","v"]]],
"poured":[["c"],["fold","d","c","k",["range",4],["set","d",["add","k","BELOW"],["add",["low","d",["add","k","BELOW"]],["share",["low","d","k"],["param",0]]],["high","d",["add","k","BELOW"]]]]],
"lifted":[["c"],["fold","d","c","k",["range",8],["set","d","k",["low","d","k"],["add",["high","d","k"],["share",["low","d","k"],["param",1]]]]]],
"settled":[["c"],["fold","d","c","k",["range",8],["set","d","k",["sub",["low","d","k"],["nudge",["high","d","k"],["param",0]]],["high","d","k"]]]]
},
"step":["let",[["out",["settled",["lifted",["poured","cells"]]]]],["vec","out",["vec",["count",["map","k",["range",8],["ne",["low","cells","k"],["low","out","k"]]]]]]]}"#;

const LAYERS_PARAMS: [u32; 2] = [2_600, 1_900];

fn planed(corner: impl Fn(usize) -> String) -> String {
    let corners: Vec<String> = (0..8).map(corner).collect();
    format!(
        "{{\"law\":1,\"name\":\"probe\",\"params\":0,\"channels\":2,\"halo\":0,\"reversible\":true,\"tally\":{{\"counts\":1,\"weighted\":false}},\"consts\":{{}},\"defs\":{{}},\"step\":[\"vec\",[\"vec\",{}],[\"vec\",0]]}}",
        corners.join(",")
    )
}

fn plane_of(k: usize, plane: usize) -> String {
    format!("[\"at\",[\"at\",\"cells\",{k}],{plane}]")
}

#[test]
fn every_word_of_every_plane_is_checked_for_lifting() {
    let zeroed = planed(|k| format!("[\"vec\",{},0]", plane_of(k, 0)));
    let why = refused(&zeroed);
    assert_eq!(why.code, Code::Lifting, "{why}");
    assert_eq!(
        why.found,
        "word 0 of plane 1 is made by the constant 0, which is not add, sub or xor of the word"
    );
    assert_eq!(why.path, "$.step");
    let scaled = planed(|k| {
        if k == 3 {
            format!(
                "[\"vec\",{},[\"mul\",{},3]]",
                plane_of(k, 0),
                plane_of(k, 1)
            )
        } else {
            format!("[\"vec\",{},{}]", plane_of(k, 0), plane_of(k, 1))
        }
    });
    let why = refused(&scaled);
    assert_eq!(why.code, Code::Lifting, "{why}");
    assert_eq!(why.path, "$.step[1][4][2]");
    assert_eq!(
        why.found,
        "word 3 of plane 1 is made by mul, which is not add, sub or xor of the word"
    );
    let reading = planed(|k| {
        if k == 5 {
            format!(
                "[\"vec\",{},[\"add\",{},[\"mul\",{},7]]]",
                plane_of(k, 0),
                plane_of(k, 1),
                plane_of(k, 1)
            )
        } else {
            format!("[\"vec\",{},{}]", plane_of(k, 0), plane_of(k, 1))
        }
    });
    let why = refused(&reading);
    assert_eq!(why.code, Code::Lifting, "{why}");
    assert!(
        why.found
            .contains("word 5 of plane 1 reads word 5 of plane 1"),
        "{why}"
    );
    let still = planed(|k| format!("[\"vec\",{},{}]", plane_of(k, 0), plane_of(k, 1)));
    assert!(reversible(&still).steps().is_empty());
    let layers = reversible(LAYERS);
    assert_eq!(layers.steps().len(), 20);
    assert_eq!(layers.forward().planes().len(), 1);
    assert_eq!(layers.inverse().planes().len(), 1);
    let planes: BTreeSet<usize> = layers
        .steps()
        .iter()
        .map(|one| one.word / CORNERS)
        .collect();
    assert_eq!(planes, BTreeSet::from([0, 1]));
}

#[test]
fn a_law_of_planes_is_undone_by_its_inverse_on_the_cpu() {
    let law = reversible(LAYERS);
    let mut state = 0x291A_u64;
    let mut moved = 0;
    for round in 0..RANDOM_OCTS {
        let words: [u32; CORNERS] = core::array::from_fn(|_| word(&mut state));
        let upper: [u32; CORNERS] = core::array::from_fn(|_| word(&mut state));
        let params = if round % 2 == 0 {
            LAYERS_PARAMS.to_vec()
        } else {
            vec![word(&mut state), word(&mut state)]
        };
        let oct = |words| Oct {
            words,
            frame: round,
            at: [1, 2, 3],
            params: &params,
        };
        let there = law
            .forward()
            .replay_planes(&mut grit::ops::Cpu, oct(words), &[upper]);
        let back =
            law.inverse()
                .replay_planes(&mut grit::ops::Cpu, oct(there.ruled.out), &there.planes);
        assert_eq!(back.ruled.out, words, "round {round}: plane 0");
        assert_eq!(back.planes, vec![upper], "round {round}: plane 1");
        moved += usize::from(there.ruled.out != words || there.planes != vec![upper]);
    }
    assert!(moved * 2 > RANDOM_OCTS as usize, "{moved}");
}

fn layered(grid: Grid3, seed: u64) -> Vec<u32> {
    let mut start = calm(grid, seed);
    start.extend(calm(grid, seed + 100));
    start
}

fn layered_objective(
    law: &Reversible,
    grid: Grid3,
    start: &[u32],
    params: &[u32],
    aim: &Aim<'_>,
) -> u64 {
    aim.measured(grid, &ran(law, grid, start, params, GRADIENT_FRAMES))
}

#[test]
fn a_law_of_planes_comes_home_and_its_adjoint_agrees_with_finite_differences() {
    let law = reversible(LAYERS);
    let grid = Grid3::new(SMALL, SMALL, SMALL).unwrap();
    let start = layered(grid, 21);
    let first_plane = &start[..grid.cells()];
    let aim = Aim {
        region: Region::whole([SMALL, SMALL, SMALL]),
        target: first_plane,
    };
    let walked = law
        .walk(grid, &start, &LAYERS_PARAMS, 0, HOME_FRAMES, &aim)
        .unwrap();
    assert!(walked.objective > 0);
    assert_eq!(walked.ended.len(), 2 * grid.cells());
    assert_eq!(
        law.walk(grid, first_plane, &LAYERS_PARAMS, 0, 1, &aim),
        Err(Fault::Cells {
            given: grid.cells(),
            wanted: 2 * grid.cells(),
        })
    );
    let mut worst = 0.0_f64;
    for seed in [31_u64, 32] {
        let start = layered(grid, seed);
        let ended = ran(&law, grid, &start, &LAYERS_PARAMS, GRADIENT_FRAMES);
        let target = ended[..grid.cells()].to_vec();
        let aim = Aim {
            region: Region::whole([SMALL, SMALL, SMALL]),
            target: &target,
        };
        for scale in [115_u32, 85] {
            let params: Vec<u32> = LAYERS_PARAMS.iter().map(|one| one * scale / 100).collect();
            let walked = law
                .walk(grid, &start, &params, 0, GRADIENT_FRAMES, &aim)
                .unwrap();
            assert_eq!(
                walked.ended,
                ran(&law, grid, &start, &params, GRADIENT_FRAMES)
            );
            assert!(walked.objective > 0);
            for (index, &told) in walked.told.iter().enumerate() {
                let mut up = params.clone();
                up[index] += STEP;
                let mut down = params.clone();
                down[index] -= STEP;
                let rise = i128::from(layered_objective(&law, grid, &start, &up, &aim))
                    - i128::from(layered_objective(&law, grid, &start, &down, &aim));
                let differenced = rise as f64 / f64::from(2 * STEP);
                let slope = told as f64 / 2_f64.powi(adjoint::FRACTION as i32);
                let off = 100.0 * (slope - differenced).abs() / differenced.abs();
                println!(
                    "layers seed {seed} params {params:?} param {index}: adjoint {slope:.2}, finite difference {differenced:.2}, {off:.3} % apart"
                );
                assert_ne!(rise, 0, "param {index}: flat");
                assert_eq!(slope.is_sign_positive(), differenced.is_sign_positive());
                assert!(
                    off <= TOLERANCE_PERCENT,
                    "seed {seed} param {index}: {off} % apart"
                );
                worst = worst.max(off);
            }
        }
    }
    println!(
        "layers: came home after {HOME_FRAMES} frames; the adjoint is at most {worst:.3} % from the finite difference"
    );
}

fn dense_frame(
    host: &Host,
    dag: &grit::law::Dag,
    grid: Grid,
    cells: &[u32],
    params: &[u32],
) -> Vec<u32> {
    let width = host.width();
    let built = sand3::settle_by(dag, width, grid, host.limits().groups).unwrap();
    let tick = live::tick(width).unwrap();
    let modules: Vec<&[u32]> = vec![&tick, &built.words];
    let mut sizes = [1_usize; grit::layout::BINDINGS as usize];
    sizes[PARAMS as usize] = grit::rule3::FIRST_PARAM + params.len();
    sizes[grit::layout::STATE as usize] = cells.len();
    let mut held = host.resident(&sizes, &modules).unwrap();
    held.write(PARAMS as usize, &[u32::MAX]).unwrap();
    held.write_at(PARAMS as usize, grit::rule3::FIRST_PARAM, params)
        .unwrap();
    held.write(grit::layout::STATE as usize, cells).unwrap();
    held.frame(&[Groups::Fixed(1), built.spread.dispatch()])
        .unwrap();
    held.read(grit::layout::STATE as usize, cells.len())
        .unwrap()
}

#[test]
fn a_law_of_planes_is_undone_by_its_inverse_on_the_device() {
    let Some(host) = device("adjoint3 planes") else {
        return;
    };
    let law = reversible(LAYERS);
    let (grid, reference_grid) = grids(SMALL);
    for seed in [41_u64, 42, 43] {
        let mut state = seed;
        let start: Vec<u32> = (0..2 * reference_grid.cells())
            .map(|_| word(&mut state))
            .collect();
        let there = dense_frame(&host, law.forward(), grid, &start, &LAYERS_PARAMS);
        let wanted = reference(law.forward(), reference_grid, 0, &start, &LAYERS_PARAMS);
        assert_eq!(there, wanted, "seed {seed}: step on {}", host.name());
        assert_ne!(
            there[reference_grid.cells()..],
            start[reference_grid.cells()..]
        );
        let home = dense_frame(&host, law.inverse(), grid, &there, &LAYERS_PARAMS);
        assert_eq!(home, start, "seed {seed}: home on {}", host.name());
    }
    println!(
        "layers on {} width {}: the device step of both planes equals the CPU and its inverse brings both home",
        host.name(),
        host.width()
    );
}

fn paired(one: &str, zero: &str) -> String {
    let rest: Vec<String> = (2..8).map(cell).collect();
    lifting_law(
        "",
        &format!(
            "[\"let\",[[\"y1\",{one}],[\"y0\",{zero}]],[\"vec\",[\"vec\",\"y0\",\"y1\",{}],[\"vec\",0]]]",
            rest.join(",")
        ),
    )
}

#[test]
fn a_step_may_read_an_earlier_version_of_its_word_through_another_word() {
    let lifted = format!("[\"add\",{},{}]", cell(1), cell(0));
    for zero in [
        format!("[\"add\",{},\"y1\"]", cell(0)),
        format!("[\"xor\",{},\"y1\"]", cell(0)),
        format!("[\"add\",{},[\"mul\",\"y1\",3]]", cell(0)),
        format!("[\"sub\",{},\"y1\"]", cell(0)),
        format!("[\"add\",\"y1\",{}]", cell(0)),
    ] {
        let text = paired(&lifted, &zero);
        let law = reversible(&text);
        assert_eq!(law.steps().len(), 2, "{zero}");
        assert_eq!(law.steps()[0].word, 1, "{zero}");
        let mut state = 0xBA_u64;
        for _ in 0..1_000 {
            let words: [u32; CORNERS] = core::array::from_fn(|_| word(&mut state));
            let oct = |words| Oct {
                words,
                frame: 0,
                at: [0; 3],
                params: &[],
            };
            let out = law.forward().step(oct(words)).out;
            assert_eq!(law.inverse().step(oct(out)).out, words, "{zero}");
        }
    }
    for (one, zero, word) in [
        (
            lifted.clone(),
            format!("[\"add\",{},{}]", cell(0), cell(0)),
            0,
        ),
        (
            lifted.clone(),
            format!("[\"add\",{},[\"mul\",{},\"y1\"]]", cell(0), cell(0)),
            0,
        ),
        (
            format!("[\"add\",{},{}]", cell(1), cell(1)),
            format!("[\"add\",{},\"y1\"]", cell(0)),
            1,
        ),
    ] {
        let why = refused(&paired(&one, &zero));
        assert_eq!(why.code, Code::Lifting, "{zero}: {why}");
        assert!(
            why.found
                .contains(&format!("reads word {word} in what it adds")),
            "{zero}: {why}"
        );
    }
}

fn sequential(state: &mut u64) -> String {
    let mut current: Vec<String> = (0..8).map(cell).collect();
    let mut binds = Vec::new();
    for step in 0..(2 + next(state) % 24) {
        let word = (next(state) % 8) as usize;
        let mut terms = Vec::new();
        for _ in 0..1 + next(state) % 3 {
            let mut other = (next(state) % 8) as usize;
            if other == word {
                other = (other + 1 + (next(state) % 7) as usize) % 8;
            }
            terms.push(current[other].clone());
        }
        let ops = ["add", "mul", "xor", "sub", "and"];
        let mut f = terms[0].clone();
        for term in &terms[1..] {
            f = format!("[\"{}\",{f},{term}]", ops[(next(state) % 5) as usize]);
        }
        let how = ["add", "sub", "xor"][(next(state) % 3) as usize];
        let name = format!("w{step}");
        let made = if how != "sub" && next(state).is_multiple_of(2) {
            format!("[\"{how}\",{f},{}]", current[word])
        } else {
            format!("[\"{how}\",{},{f}]", current[word])
        };
        binds.push(format!("[\"{name}\",{made}]"));
        current[word] = format!("\"{name}\"");
    }
    format!(
        "[\"let\",[{}],[\"vec\",[\"vec\",{}],[\"vec\",0]]]",
        binds.join(","),
        current.join(",")
    )
}

#[test]
fn every_sequential_lifting_program_is_admitted_and_undone() {
    let mut state = 0x5E9_u64;
    let mut steps = 0;
    for program in 0..3_000 {
        let text = lifting_law("", &sequential(&mut state));
        let made = Law::compile(&text).unwrap_or_else(|why| panic!("{program}: {why} {text}"));
        let law = Reversible::of(&made).unwrap_or_else(|why| panic!("{program}: {why}\n{text}"));
        steps += law.steps().len();
        for _ in 0..200 {
            let words: [u32; CORNERS] = core::array::from_fn(|_| word(&mut state));
            let oct = |words| Oct {
                words,
                frame: 0,
                at: [0; 3],
                params: &[],
            };
            let out = law.forward().step(oct(words)).out;
            assert_eq!(law.inverse().step(oct(out)).out, words, "{program}\n{text}");
        }
    }
    println!(
        "3000 sequential lifting programs of {steps} steps: all admitted, each undone on 200 octs"
    );
}
