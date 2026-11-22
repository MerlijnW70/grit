#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

#[cfg(feature = "faults")]
mod delayed;

use common::device;

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use grit::law::Law;
use grit::rule3::{Declared, heat};
use grit::sand3::{Grid3, reference};
use grit::serve3::Server;
use grit::serve3::learn::{
    CELLS_MOST, FRAMES_MOST, GENERATIONS_MOST, POPULATION_MOST, SEARCH, STALE, WORK_MOST,
};
use grit::wire::{Json, read, written};

const HEAT: &str = include_str!("law/heat.law.json");
const SAND: &str = include_str!("law/sand.law.json");
const MIX: &str = include_str!("law/mix.law.json");

const SHIPPED: [(&str, &str); 11] = [
    ("heat", HEAT),
    ("sand", SAND),
    ("nature", include_str!("law/nature.law.json")),
    ("evo", include_str!("law/evo.law.json")),
    ("code", include_str!("law/code.law.json")),
    ("embed", include_str!("law/embed.law.json")),
    ("mix", MIX),
    ("stir", include_str!("law/stir.law.json")),
    ("blur2", include_str!("halo3/blur2.law.json")),
    ("edge2d", include_str!("halo3/edge2d.law.json")),
    ("sobel3", include_str!("halo3/sobel3.law.json")),
];

const LONG: u64 = 600_000;

fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn below(state: &mut u64, most: u64) -> u64 {
    next(state) % most
}

fn small(state: &mut u64, most: u64) -> u32 {
    u32::try_from(below(state, most)).unwrap()
}

fn word(state: &mut u64) -> u32 {
    match below(state, 6) {
        0 => 0,
        1 => u32::MAX,
        2 => small(state, 16),
        _ => u32::try_from(next(state) >> 32).unwrap(),
    }
}

fn quoted(text: &str) -> String {
    written(&Json::Text(text.to_owned()))
}

fn line(id: i64, name: &str, arguments: &str) -> String {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"tools/call\",\"params\":{{\"name\":\"{name}\",\"arguments\":{arguments}}}}}"
    )
}

fn call(server: &mut Server<'_>, name: &str, arguments: &str) -> Json {
    server.heard(&line(7, name, arguments)).unwrap()
}

fn result(got: &Json, error: bool) -> Json {
    let held = got
        .get("result")
        .unwrap_or_else(|| panic!("{}", written(got)));
    assert_eq!(
        held.get("isError"),
        Some(&Json::Bool(error)),
        "{}",
        written(got)
    );
    held.get("structuredContent").cloned().unwrap()
}

fn answer(got: &Json) -> Json {
    result(got, false)
}

fn refusal(got: &Json) -> Json {
    result(got, true)
}

fn int_at(held: &Json, key: &str) -> i64 {
    held.get(key)
        .and_then(Json::int)
        .unwrap_or_else(|| panic!("no {key} in {}", written(held)))
}

fn text_at(held: &Json, key: &str) -> String {
    held.get(key)
        .and_then(Json::text)
        .unwrap_or_else(|| panic!("no {key} in {}", written(held)))
        .to_owned()
}

fn whole_of(held: &Json) -> u64 {
    match held {
        Json::Int(one) => u64::try_from(*one).unwrap(),
        Json::Text(one) => one.parse().unwrap(),
        other => panic!("not a whole number: {}", written(other)),
    }
}

fn words_of(held: &Json) -> Vec<u32> {
    held.list()
        .unwrap()
        .iter()
        .map(|one| u32::try_from(one.int().unwrap()).unwrap())
        .collect()
}

fn compiled(text: &str) -> Law {
    Law::compile(text).unwrap_or_else(|why| panic!("{why}\n{text}"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Boxed {
    from: [u32; 3],
    to: [u32; 3],
}

impl Boxed {
    const fn whole(size: [u32; 3]) -> Self {
        Self {
            from: [0, 0, 0],
            to: [size[0] - 1, size[1] - 1, size[2] - 1],
        }
    }

    fn points(&self) -> Vec<[u32; 3]> {
        let mut points = Vec::new();
        for z in self.from[2]..=self.to[2] {
            for y in self.from[1]..=self.to[1] {
                for x in self.from[0]..=self.to[0] {
                    points.push([x, y, z]);
                }
            }
        }
        points
    }

    fn shown(&self) -> String {
        format!("\"from\":{:?},\"to\":{:?}", self.from, self.to)
    }
}

fn grid_of(size: [u32; 3]) -> Grid3 {
    Grid3::new(size[0], size[1], size[2]).unwrap()
}

fn forward(
    law: &Law,
    size: [u32; 3],
    start: &[u32],
    params: &[u32],
    first: u32,
    frames: u32,
) -> Vec<u32> {
    let mut cells = start.to_vec();
    for done in 0..frames {
        cells = reference(law, grid_of(size), first.wrapping_add(done), &cells, params);
    }
    cells
}

fn measured(size: [u32; 3], cells: &[u32], aim: Boxed, target: &[u32]) -> u64 {
    let grid = grid_of(size);
    aim.points()
        .iter()
        .zip(target)
        .map(|(&[x, y, z], &wanted)| u64::from(cells[grid.index(x, y, z)].abs_diff(wanted)))
        .sum()
}

fn boxed(size: [u32; 3], cells: &[u32], aim: Boxed) -> Vec<u32> {
    let grid = grid_of(size);
    aim.points()
        .iter()
        .map(|&[x, y, z]| cells[grid.index(x, y, z)])
        .collect()
}

#[derive(Clone)]
struct Case {
    text: String,
    size: [u32; 3],
    cells: Vec<u32>,
    first: u32,
    params: Vec<u32>,
    frames: u32,
    aim: Boxed,
    target: Vec<u32>,
}

impl Case {
    fn law(&self) -> Law {
        compiled(&self.text)
    }

    fn expected(&self, params: &[u32]) -> u64 {
        let ended = forward(
            &self.law(),
            self.size,
            &self.cells,
            params,
            self.first,
            self.frames,
        );
        measured(self.size, &ended, self.aim, &self.target)
    }

    fn arguments(&self, more: &str) -> String {
        format!(
            "{{\"law\":{},\"size\":{:?},\"cells\":{:?},\"first\":{},\"params\":{:?},\"frames\":{},\"aim\":{{{},\"target\":{:?}}}{more},\"budget_ms\":{LONG}}}",
            quoted(&self.text),
            self.size,
            self.cells,
            self.first,
            self.params,
            self.frames,
            self.aim.shown(),
            self.target
        )
    }
}

fn searching(bounds: &[[u32; 2]], seed: u32, population: u32, generations: u32) -> String {
    format!(
        ",\"bounds\":{bounds:?},\"seed\":{seed},\"population\":{population},\"generations\":{generations}"
    )
}

fn law_of(name: &str, params: u32, planes: u32, halo: u32, outs: &[String]) -> String {
    format!(
        "{{\"law\":1,\"name\":\"{name}\",\"params\":{params},\"channels\":{planes},\"halo\":{halo},\"tally\":{{\"counts\":1,\"weighted\":false}},\"consts\":{{}},\"defs\":{{}},\"step\":[\"vec\",[\"vec\",{}],[\"vec\",0]]}}",
        outs.join(",")
    )
}

fn cell_at(k: u32, plane: u32, planes: u32) -> String {
    if planes == 1 {
        format!("[\"at\",\"cells\",{k}]")
    } else {
        format!("[\"at\",[\"at\",\"cells\",{k}],{plane}]")
    }
}

struct Shape {
    params: u32,
    planes: u32,
    halo: bool,
    divides: bool,
}

fn leaf(state: &mut u64, shape: &Shape) -> String {
    match below(state, 7) {
        0 | 1 => cell_at(
            small(state, 8),
            small(state, u64::from(shape.planes)),
            shape.planes,
        ),
        2 if shape.params > 0 => format!("[\"param\",{}]", small(state, u64::from(shape.params))),
        3 => match below(state, 3) {
            0 => "\"frame\"".to_owned(),
            1 => format!("[\"at\",\"at\",{}]", below(state, 3)),
            _ => "[\"noise\"]".to_owned(),
        },
        4 if shape.halo => format!(
            "[\"look\",1,{},{},{},{}]",
            i64::from(small(state, 3)) - 1,
            i64::from(small(state, 3)) - 1,
            i64::from(small(state, 3)) - 1,
            small(state, 8)
        ),
        _ => word(state).to_string(),
    }
}

fn expr(state: &mut u64, shape: &Shape, depth: u32) -> String {
    if depth == 0 || below(state, 4) == 0 {
        return leaf(state, shape);
    }
    let less = depth - 1;
    let compares = ["lt", "le", "gt", "ge", "eq", "ne"];
    match below(state, 8) {
        0..=2 => {
            let ops = [
                "add", "sub", "mul", "xor", "and", "or", "dot_s", "dot_u", "div",
            ];
            let kinds = if shape.divides { 9 } else { 8 };
            let op = ops[usize::try_from(below(state, kinds)).unwrap()];
            let left = expr(state, shape, less);
            let right = expr(state, shape, less);
            format!("[\"{op}\",{left},{right}]")
        }
        3 => {
            let op = ["shl", "shr"][usize::try_from(below(state, 2)).unwrap()];
            let shifted = expr(state, shape, less);
            format!("[\"{op}\",{shifted},{}]", below(state, 32))
        }
        4 | 5 => {
            let compare = compares[usize::try_from(below(state, 6)).unwrap()];
            let left = expr(state, shape, less);
            let right = expr(state, shape, less);
            let then = expr(state, shape, less);
            let otherwise = expr(state, shape, less);
            format!("[\"sel\",[\"{compare}\",{left},{right}],{then},{otherwise}]")
        }
        6 => {
            let compare = compares[usize::try_from(below(state, 6)).unwrap()];
            let left = expr(state, shape, less);
            let right = expr(state, shape, less);
            format!("[\"word\",[\"{compare}\",{left},{right}]]")
        }
        _ => format!("[\"not\",{}]", expr(state, shape, less)),
    }
}

fn random_law(seed: u64, divides: bool) -> (String, Vec<u32>) {
    let mut state = seed;
    let halo = below(&mut state, 3) == 0;
    let shape = Shape {
        params: small(&mut state, 4),
        planes: if halo { 2 } else { 1 + small(&mut state, 2) },
        halo,
        divides,
    };
    let outs: Vec<String> = (0..8)
        .map(|k| {
            let mut made = expr(&mut state, &shape, 3);
            if shape.halo && k == 0 {
                made = format!(
                    "[\"add\",{made},[\"look\",1,1,0,0,{}]]",
                    small(&mut state, 8)
                );
            }
            if shape.planes == 1 {
                made
            } else if shape.halo {
                format!("[\"vec\",{made},{}]", cell_at(k, 1, 2))
            } else {
                format!("[\"vec\",{made},{}]", expr(&mut state, &shape, 2))
            }
        })
        .collect();
    let params = (0..shape.params).map(|_| word(&mut state)).collect();
    (
        law_of(
            &format!("random{seed}"),
            shape.params,
            shape.planes,
            u32::from(shape.halo),
            &outs,
        ),
        params,
    )
}

fn random_case(state: &mut u64, text: &str, params: Vec<u32>) -> Case {
    let law = compiled(text);
    let side = |state: &mut u64| 2 << small(state, 3);
    let size = [side(state), side(state), side(state)];
    let planes = u32::try_from(Declared::planes(&law)).unwrap();
    let cells: Vec<u32> = (0..size.iter().product::<u32>() * planes)
        .map(|_| word(state))
        .collect();
    let first = if below(state, 4) == 0 {
        u32::MAX - small(state, 3)
    } else {
        small(state, 100)
    };
    let frames = 1 + small(state, 6);
    let mut from = [0; 3];
    let mut to = [0; 3];
    for axis in 0..3 {
        let one = small(state, u64::from(size[axis]));
        let other = small(state, u64::from(size[axis]));
        from[axis] = one.min(other);
        to[axis] = one.max(other);
    }
    let aim = Boxed { from, to };
    let ended = forward(&law, size, &cells, &params, first, frames);
    let mut target = boxed(size, &ended, aim);
    match below(state, 3) {
        0 => {
            for one in &mut target {
                *one = word(state);
            }
        }
        1 => {
            for _ in 0..=below(state, 3) {
                let at = usize::try_from(below(state, target.len() as u64)).unwrap();
                target[at] = target[at].wrapping_add(1 + small(state, 1000));
            }
        }
        _ => {}
    }
    Case {
        text: text.to_owned(),
        size,
        cells,
        first,
        params,
        frames,
        aim,
        target,
    }
}

fn shipped_params(state: &mut u64, law: &Law) -> Vec<u32> {
    (0..law.params()).map(|_| small(state, 5000)).collect()
}

const RANDOM_LAWS: u64 = 40;

#[test]
fn s1_the_score_is_the_cpu_reference_objective_on_random_laws_params_and_aims() {
    let mut server = Server::new(None);
    let mut state = 0x5C0E_u64;
    let mut cases = Vec::new();
    for (_, text) in SHIPPED {
        for _ in 0..2 {
            let params = shipped_params(&mut state, &compiled(text));
            cases.push(random_case(&mut state, text, params));
        }
    }
    let mut halos = 0;
    let mut planed = 0;
    for seed in 0..RANDOM_LAWS {
        let (text, params) = random_law(seed, true);
        let law = compiled(&text);
        halos += usize::from(law.halo() == 1);
        planed += usize::from(Declared::planes(&law) > 1);
        for _ in 0..2 {
            cases.push(random_case(&mut state, &text, params.clone()));
        }
    }
    assert!(
        halos >= 8 && planed >= 16,
        "{halos} looking, {planed} planed"
    );
    let mut zero = 0;
    for (index, case) in cases.iter().enumerate() {
        let got = answer(&call(&mut server, "grit_law_score", &case.arguments("")));
        let wanted = case.expected(&case.params);
        zero += usize::from(wanted == 0);
        assert_eq!(
            whole_of(got.get("objective").unwrap()),
            wanted,
            "case {index}: {}",
            case.text
        );
        assert_eq!(words_of(got.get("params").unwrap()), case.params);
        assert_eq!(int_at(&got, "first"), i64::from(case.first));
        assert_eq!(int_at(&got, "frames"), i64::from(case.frames));
    }
    assert!(
        zero > 0 && zero < cases.len(),
        "{zero} of {} at 0",
        cases.len()
    );
}

trait Ask {
    fn ask(&mut self, name: &str, arguments: &str) -> Json;
}

impl Ask for Server<'_> {
    fn ask(&mut self, name: &str, arguments: &str) -> Json {
        call(self, name, arguments)
    }
}

fn created(asker: &mut dyn Ask, text: &str, params: &[u32], size: [u32; 3]) -> i64 {
    let made = answer(&asker.ask(
        "grit_world_create",
        &format!(
            "{{\"wide\":{},\"deep\":{},\"tall\":{},\"law\":{},\"params\":{params:?}}}",
            size[0],
            size[1],
            size[2],
            quoted(text)
        ),
    ));
    int_at(&made, "world")
}

fn write_all(asker: &mut dyn Ask, world: i64, size: [u32; 3], cells: &[u32]) {
    let grid = grid_of(size);
    let placed: Vec<String> = Boxed::whole(size)
        .points()
        .iter()
        .map(|&[x, y, z]| format!("[{x},{y},{z},{}]", cells[grid.index(x, y, z)]))
        .collect();
    for chunk in placed.chunks(4096) {
        answer(&asker.ask(
            "grit_world_write",
            &format!(
                "{{\"world\":{world},\"cells\":[{}],\"budget_ms\":{LONG}}}",
                chunk.join(",")
            ),
        ));
    }
}

fn step(asker: &mut dyn Ask, world: i64, frames: u32) {
    answer(&asker.ask(
        "grit_world_step",
        &format!("{{\"world\":{world},\"frames\":{frames},\"budget_ms\":{LONG}}}"),
    ));
}

fn state_of(asker: &mut dyn Ask, world: i64, size: [u32; 3], planes: u32) -> (i64, Vec<u32>) {
    let grid = grid_of(size);
    let mut cells = vec![0; grid.cells() * planes as usize];
    let mut frame = 0;
    for plane in 0..planes {
        for z in 0..size[2] {
            let got = answer(&asker.ask(
                "grit_world_read",
                &format!(
                    "{{\"world\":{world},\"from\":[0,0,{z}],\"to\":[{},{},{z}],\"plane\":{plane}}}",
                    size[0] - 1,
                    size[1] - 1
                ),
            ));
            frame = int_at(&got, "frame");
            let slab = Boxed {
                from: [0, 0, z],
                to: [size[0] - 1, size[1] - 1, z],
            };
            for (&[x, y, z], one) in slab
                .points()
                .iter()
                .zip(words_of(got.get("cells").unwrap()))
            {
                cells[plane as usize * grid.cells() + grid.index(x, y, z)] = one;
            }
        }
    }
    (frame, cells)
}

fn landscape(state: &mut u64, size: [u32; 3], cell: fn(&mut u64) -> u32) -> Vec<u32> {
    (0..size.iter().product::<u32>())
        .map(|_| if below(state, 3) == 0 { cell(state) } else { 0 })
        .collect()
}

fn heat_cell(state: &mut u64) -> u32 {
    let roll = next(state);
    let kind = [heat::WALL, heat::WATER, heat::FUEL, heat::FUEL, heat::ICE]
        [usize::try_from(roll % 5).unwrap()];
    heat::cell(kind, u32::try_from((roll >> 8) % 1200).unwrap())
}

fn sand_cell(state: &mut u64) -> u32 {
    [grit::sand::SAND, grit::sand::WATER, grit::sand::WALL]
        [usize::try_from(below(state, 3)).unwrap()]
}

type Laid = (String, Vec<u32>, fn(&mut u64) -> u32);

#[test]
fn s1_a_world_start_scores_as_the_device_steps_it() {
    let Some(host) = device("learn_search world start") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let mut state = 0xD5C0_u64;
    let mut laws: Vec<Laid> = vec![
        (HEAT.to_owned(), vec![250, 7, 900], heat_cell),
        (SAND.to_owned(), vec![], sand_cell),
        (
            include_str!("halo3/edge2d.law.json").to_owned(),
            vec![300],
            word,
        ),
    ];
    for seed in 100..104 {
        let (text, params) = random_law(seed, false);
        laws.push((text, params, word));
    }
    let size = [8, 8, 8];
    for (index, (text, params, cell)) in laws.iter().enumerate() {
        let law = compiled(text);
        let planes = u32::try_from(Declared::planes(&law)).unwrap();
        let world = created(&mut server, text, params, size);
        write_all(
            &mut server,
            world,
            size,
            &landscape(&mut state, size, *cell),
        );
        step(&mut server, world, 3 + small(&mut state, 5));
        let (frame, start) = state_of(&mut server, world, size, planes);
        let frames = 1 + small(&mut state, 6);
        let fork = int_at(
            &answer(&server.ask("grit_world_fork", &format!("{{\"world\":{world}}}"))),
            "world",
        );
        step(&mut server, fork, frames);
        let (_, ended) = state_of(&mut server, fork, size, planes);
        let first = u32::try_from(frame).unwrap();
        assert_eq!(
            forward(&law, size, &start, params, first, frames),
            ended,
            "law {index}: the device and the CPU reference agree"
        );
        for _ in 0..3 {
            let one = [
                small(&mut state, 8),
                small(&mut state, 8),
                small(&mut state, 8),
            ];
            let other = [
                small(&mut state, 8),
                small(&mut state, 8),
                small(&mut state, 8),
            ];
            let aim = Boxed {
                from: [0, 1, 2].map(|axis| one[axis].min(other[axis])),
                to: [0, 1, 2].map(|axis| one[axis].max(other[axis])),
            };
            let target: Vec<u32> = aim.points().iter().map(|_| word(&mut state)).collect();
            let got = answer(&server.ask(
                "grit_law_score",
                &format!(
                    "{{\"world\":{world},\"frames\":{frames},\"aim\":{{{},\"target\":{target:?}}}}}",
                    aim.shown()
                ),
            ));
            assert_eq!(
                whole_of(got.get("objective").unwrap()),
                measured(size, &ended, aim, &target),
                "law {index}: the score is the objective of the device's own frames"
            );
            assert_eq!(int_at(&got, "first"), frame);
            let by_world = answer(&server.ask(
                "grit_law_score",
                &format!(
                    "{{\"world\":{world},\"frames\":{frames},\"aim\":{{{},\"world\":{fork}}}}}",
                    aim.shown()
                ),
            ));
            assert_eq!(
                whole_of(by_world.get("objective").unwrap()),
                0,
                "law {index}: the fork stepped by the device is its own target"
            );
        }
        assert_eq!(
            state_of(&mut server, world, size, planes),
            (frame, start),
            "law {index}: scoring left the world as it was"
        );
    }
}

fn heat_case(side: u32, seed: u64, truth: &[u32], guess: &[u32], frames: u32) -> Case {
    let size = [side, side, side];
    let mut state = seed;
    let cells = landscape(&mut state, size, heat_cell);
    let law = compiled(HEAT);
    let ended = forward(&law, size, &cells, truth, 0, frames);
    let aim = Boxed::whole(size);
    Case {
        text: HEAT.to_owned(),
        size,
        target: boxed(size, &ended, aim),
        cells,
        first: 0,
        params: guess.to_vec(),
        frames,
        aim,
    }
}

const HEAT_TRUTH: [u32; 3] = [300, 7, 900];

const HEAT_GUESS: [u32; 3] = [700, 12, 3000];

const HEAT_BOUNDS: [[u32; 2]; 3] = [[0, 1023], [0, 15], [0, 4095]];

const HEAT_FRAMES: u32 = 8;

const HEAT_SEED: u32 = 7;

fn trail_of(got: &Json) -> Vec<(Vec<u32>, u64)> {
    got.get("trail")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .map(|one| {
            (
                words_of(one.get("params").unwrap()),
                whole_of(one.get("objective").unwrap()),
            )
        })
        .collect()
}

fn earliest_lowest(trail: &[(Vec<u32>, u64)]) -> (Vec<u32>, u64) {
    let lowest = trail.iter().map(|(_, one)| *one).min().unwrap();
    trail
        .iter()
        .find(|(_, one)| *one == lowest)
        .cloned()
        .unwrap()
}

fn checked_trail(case: &Case, got: &Json, bounds: &[[u32; 2]]) {
    let trail = trail_of(got);
    assert_eq!(trail[0].0, case.params, "generation 0 is the first parent");
    for (generation, (params, objective)) in trail.iter().enumerate() {
        assert_eq!(
            *objective,
            case.expected(params),
            "generation {generation}: {params:?} is scored with its own params"
        );
        for (param, bound) in params.iter().zip(bounds) {
            assert!(
                (bound[0]..=bound[1]).contains(param),
                "{params:?} {bounds:?}"
            );
        }
    }
    let (best, lowest) = earliest_lowest(&trail);
    assert_eq!(whole_of(got.get("objective").unwrap()), lowest);
    if text_at(got, "stopped") != "budget" && text_at(got, "stopped") != "cancelled" {
        assert_eq!(words_of(got.get("params").unwrap()), best);
    }
    assert_eq!(
        int_at(got, "generations"),
        i64::try_from(trail.len() - 1).unwrap()
    );
}

fn heat_search(case: &Case, seed: u32, generations: u32) -> String {
    case.arguments(&searching(&HEAT_BOUNDS, seed, 8, generations))
}

#[test]
fn s5_a_search_through_the_mcp_recovers_hidden_heat_params_inline() {
    let case = heat_case(8, 0x4EA7, &HEAT_TRUTH, &HEAT_GUESS, HEAT_FRAMES);
    assert!(case.expected(&case.params) > 0);
    let mut server = Server::new(None);
    let got = answer(&call(
        &mut server,
        "grit_learn_params",
        &heat_search(&case, HEAT_SEED, 200),
    ));
    println!("{}", written(&got));
    assert_eq!(text_at(&got, "stopped"), "found", "{}", written(&got));
    assert_eq!(whole_of(got.get("objective").unwrap()), 0);
    let found = words_of(got.get("params").unwrap());
    assert_eq!(case.expected(&found), 0);
    checked_trail(&case, &got, &HEAT_BOUNDS);
}

const MIX_TRUTH: [u32; 2] = [3_000, 1_700];

const MIX_GUESS: [u32; 2] = [2_100, 2_250];

const MIX_BOUNDS: [[u32; 2]; 2] = [[0, 8191], [0, 8191]];

fn mix_case() -> Case {
    let size = [16, 16, 16];
    let mut state = 4_u64;
    let cells: Vec<u32> = (0..4096)
        .map(|_| (1 << 22) - (1 << 21) + small(&mut state, 1 << 22))
        .collect();
    let ended = forward(&compiled(MIX), size, &cells, &MIX_TRUTH, 0, 8);
    let aim = Boxed::whole(size);
    Case {
        text: MIX.to_owned(),
        size,
        target: boxed(size, &ended, aim),
        cells,
        first: 0,
        params: MIX_GUESS.to_vec(),
        frames: 8,
        aim,
    }
}

#[test]
fn s5_on_a_reversible_law_the_search_and_the_descent_agree_on_the_optimum() {
    let case = mix_case();
    let mut server = Server::new(None);
    let descended = answer(&call(
        &mut server,
        "grit_law_descend",
        &case.arguments(",\"steps\":16"),
    ));
    let searched = answer(&call(
        &mut server,
        "grit_learn_params",
        &case.arguments(&searching(&MIX_BOUNDS, 3, 8, 200)),
    ));
    for got in [&descended, &searched] {
        assert_eq!(
            words_of(got.get("params").unwrap()),
            MIX_TRUTH,
            "{}",
            written(got)
        );
        assert_eq!(whole_of(got.get("objective").unwrap()), 0);
    }
    assert_eq!(text_at(&searched, "stopped"), "found");
    checked_trail(&case, &searched, &MIX_BOUNDS);
    let around: Vec<u64> = [[-1_i64, 0], [1, 0], [0, -1], [0, 1]]
        .iter()
        .map(|delta| {
            let near: Vec<u32> = MIX_TRUTH
                .iter()
                .zip(delta)
                .map(|(one, by)| u32::try_from(i64::from(*one) + by).unwrap())
                .collect();
            case.expected(&near)
        })
        .collect();
    assert!(
        around.iter().all(|one| *one > 0),
        "the optimum is strict: {around:?}"
    );
}

#[test]
fn s2_the_same_seed_gives_the_same_answer_and_a_shorter_search_a_prefix() {
    let case = heat_case(8, 0x4EA7, &HEAT_TRUTH, &HEAT_GUESS, HEAT_FRAMES);
    let mut server = Server::new(None);
    let whole = written(&answer(&call(
        &mut server,
        "grit_learn_params",
        &heat_search(&case, 5, 12),
    )));
    let again = written(&answer(&call(
        &mut server,
        "grit_learn_params",
        &heat_search(&case, 5, 12),
    )));
    let mut fresh = Server::new(None);
    let fresh_answer = written(&answer(&call(
        &mut fresh,
        "grit_learn_params",
        &heat_search(&case, 5, 12),
    )));
    assert_eq!(whole, again, "the same server, the same answer");
    assert_eq!(whole, fresh_answer, "a fresh server, the same answer");
    let whole_trail = trail_of(&read(&whole).unwrap());
    assert_eq!(whole_trail.len(), 13);
    for generations in [1, 5, 11] {
        let shorter = answer(&call(
            &mut server,
            "grit_learn_params",
            &heat_search(&case, 5, generations),
        ));
        let trail = trail_of(&shorter);
        assert_eq!(text_at(&shorter, "stopped"), "generations");
        assert_eq!(trail[..], whole_trail[..trail.len()], "{generations}");
        assert_eq!(trail.len(), generations as usize + 1);
    }
    let other = answer(&call(
        &mut server,
        "grit_learn_params",
        &heat_search(&case, 6, 12),
    ));
    assert_ne!(trail_of(&other), whole_trail, "the seed draws the children");
    let pinned = grit::law::hex(&grit::branch::sha256(whole.as_bytes()));
    println!("answer sha256 {pinned}");
    assert_eq!(
        pinned, PINNED,
        "the answer on this machine, any device, any OS"
    );
}

const PINNED: &str = "f03d7835a59c1d6f82b361a01c59171d564a916b216dbd58d36d30152ef16eab";

fn without_world(got: &Json) -> String {
    match got {
        Json::Map(fields) => written(&Json::Map(
            fields
                .iter()
                .filter(|(key, _)| key != "world" && key != "restored" && key != "restored_total")
                .cloned()
                .collect(),
        )),
        other => written(other),
    }
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "grit-learnsearch-{label}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn dir(&self) -> PathBuf {
        self.0.join("worlds")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Process {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    id: i64,
}

impl Process {
    fn start(dir: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_grit-mcp"))
            .arg("--dir")
            .arg(dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            input,
            output,
            id: 0,
        }
    }

    fn kill(mut self) {
        self.child.kill().unwrap();
        self.child.wait().unwrap();
    }
}

impl Ask for Process {
    fn ask(&mut self, name: &str, arguments: &str) -> Json {
        self.id += 1;
        writeln!(self.input, "{}", line(self.id, name, arguments)).unwrap();
        self.input.flush().unwrap();
        loop {
            let mut held = String::new();
            assert!(
                self.output.read_line(&mut held).unwrap() > 0,
                "the server ended"
            );
            let got = read(held.trim_end()).unwrap();
            if got.get("id").and_then(Json::int) == Some(self.id) {
                return got;
            }
        }
    }
}

fn world_search(world: i64, target: i64, case: &Case, seed: u32, generations: u32) -> String {
    format!(
        "{{\"world\":{world},\"frames\":{},\"aim\":{{{},\"world\":{target}}}{},\"budget_ms\":{LONG}}}",
        case.frames,
        case.aim.shown(),
        searching(&HEAT_BOUNDS, seed, 8, generations)
    )
}

fn opened_heat(asker: &mut dyn Ask, case: &Case) -> (i64, i64) {
    let learner = created(asker, HEAT, &case.params, case.size);
    write_all(asker, learner, case.size, &case.cells);
    let hidden = created(asker, HEAT, &HEAT_TRUTH, case.size);
    write_all(asker, hidden, case.size, &case.cells);
    step(asker, hidden, case.frames);
    (learner, hidden)
}

#[test]
fn s2_a_world_search_answers_as_inline_on_this_device_and_across_a_restart() {
    let Some(host) = device("learn_search restart") else {
        return;
    };
    drop(host);
    let case = heat_case(8, 0x4EA7, &HEAT_TRUTH, &HEAT_GUESS, HEAT_FRAMES);
    let inline = written(&answer(&call(
        &mut Server::new(None),
        "grit_learn_params",
        &heat_search(&case, 5, 12),
    )));
    let scratch = Scratch::new("restart");
    let mut first = Process::start(&scratch.dir());
    let (learner, hidden) = opened_heat(&mut first, &case);
    let before = answer(&first.ask(
        "grit_learn_params",
        &world_search(learner, hidden, &case, 5, 12),
    ));
    assert_eq!(int_at(&before, "world"), learner);
    assert_eq!(without_world(&before), inline, "a world start is its cells");
    first.kill();
    let mut second = Process::start(&scratch.dir());
    for world in [learner, hidden] {
        answer(&second.ask(
            "grit_world_open",
            &format!("{{\"name\":\"world-{world}\"}}"),
        ));
    }
    let after = answer(&second.ask(
        "grit_learn_params",
        &world_search(learner, hidden, &case, 5, 12),
    ));
    assert_eq!(written(&after), written(&before), "across a restart");
    second.kill();
}

#[cfg(feature = "faults")]
fn runs() -> Option<u64> {
    Some(grit::serve3::learn::faults::runs())
}

#[cfg(not(feature = "faults"))]
const fn runs() -> Option<u64> {
    None
}

struct Refused {
    tool: &'static str,
    arguments: String,
    code: &'static str,
    field: &'static str,
}

fn both(arguments: &str, search: &str, code: &'static str, field: &'static str) -> [Refused; 2] {
    [
        Refused {
            tool: "grit_law_score",
            arguments: arguments.replace('@', ""),
            code,
            field,
        },
        Refused {
            tool: "grit_learn_params",
            arguments: arguments.replace('@', search),
            code,
            field,
        },
    ]
}

fn inline_with(
    text: &str,
    size: &str,
    cells: &str,
    params: &str,
    frames: &str,
    aim: &str,
) -> String {
    format!(
        "{{\"law\":{},\"size\":{size},\"cells\":{cells},\"params\":{params},\"frames\":{frames},\"aim\":{aim}@,\"budget_ms\":{LONG}}}",
        quoted(text)
    )
}

fn checked_refusals(server: &mut Server<'_>, cases: &[Refused]) -> Vec<Json> {
    let mut told_all = Vec::new();
    for one in cases {
        let ran = runs();
        let told = refusal(&call(server, one.tool, &one.arguments));
        assert_eq!(
            (text_at(&told, "code"), text_at(&told, "field")),
            (one.code.to_owned(), one.field.to_owned()),
            "{}: {}\n{}",
            one.tool,
            written(&told),
            one.arguments
        );
        assert!(told.get("hint").is_some());
        assert_eq!(runs(), ran, "{} ran before {}", one.tool, one.code);
        told_all.push(told);
    }
    told_all
}

#[test]
fn s3_every_refusal_names_its_code_and_path_before_any_run() {
    let case = heat_case(8, 0x4EA7, &HEAT_TRUTH, &HEAT_GUESS, HEAT_FRAMES);
    let cells = format!("{:?}", case.cells);
    let size = "[8,8,8]";
    let params = "[700,12,3000]";
    let aim = format!(
        "{{\"from\":[0,0,0],\"to\":[7,7,7],\"target\":{:?}}}",
        case.target
    );
    let search = searching(&HEAT_BOUNDS, 1, 8, 4);
    let at = |size: &str, cells: &str, params: &str, frames: &str, aim: &str| {
        inline_with(HEAT, size, cells, params, frames, aim)
    };
    let inline = at(size, &cells, params, "8", &aim);
    let wide = |from: &str, to: &str, more: &str| {
        at(
            size,
            &cells,
            params,
            "8",
            &format!("{{\"from\":{from},\"to\":{to}{more}}}"),
        )
    };
    let mut cases = Vec::new();
    for (arguments, code, field) in [
        (inline.replacen('{', "{\"world\":1,", 1), "start", "$"),
        (format!("{{\"frames\":8,\"aim\":{aim}@}}"), "start", "$"),
        (inline.replace("\"size\":[8,8,8],", ""), "start", "$.size"),
        (at("[8,8,6]", &cells, params, "8", &aim), "start", "$.size"),
        (at(size, "[1,2,3]", params, "8", &aim), "start", "$.cells"),
        (
            inline.replace(&quoted(HEAT), &quoted("{\"law\":1}")),
            "law_refused",
            "$.law",
        ),
        (at(size, &cells, "[1,2]", "8", &aim), "params", "$.params"),
        (
            inline.replace(",\"params\":[700,12,3000]", ""),
            "params",
            "$.params",
        ),
        (
            wide("[0,0,0]", "[8,0,0]", ",\"target\":[1]"),
            "outside",
            "$.aim.to",
        ),
        (
            wide("[0,9,0]", "[0,0,0]", ",\"target\":[1]"),
            "outside",
            "$.aim.from",
        ),
        (
            wide("[1,0,0]", "[0,0,0]", ",\"target\":[1]"),
            "reversed",
            "$.aim.to",
        ),
        (
            wide("[0,0,0]", "[1,0,0]", ",\"target\":[1]"),
            "target_size",
            "$.aim.target",
        ),
        (wide("[0,0,0]", "[0,0,0]", ""), "aim", "$.aim"),
        (
            wide("[0,0,0]", "[0,0,0]", ",\"target\":[1],\"world\":1"),
            "aim",
            "$.aim",
        ),
        (
            at(size, &cells, params, "4096", &aim),
            "run_limit",
            "$.frames",
        ),
        (
            at(size, &cells, params, &(FRAMES_MOST + 1).to_string(), &aim),
            "frames",
            "$.frames",
        ),
        (
            format!("{{\"world\":4000,\"frames\":8,\"aim\":{aim}@}}"),
            "no_world",
            "$.world",
        ),
    ] {
        cases.extend(both(&arguments, &search, code, field));
    }
    for (more, code, field) in [
        (searching(&HEAT_BOUNDS[..2], 1, 8, 4), "params", "$.bounds"),
        (
            searching(&[[0, 1023], [9, 3], [0, 4095]], 1, 8, 4),
            "reversed",
            "$.bounds[1]",
        ),
        (
            searching(&[[0, 1023], [0, 11], [0, 4095]], 1, 8, 4),
            "bounds",
            "$.params[1]",
        ),
        (
            searching(&[[700, 700], [12, 12], [3000, 3000]], 1, 8, 4),
            "bounds",
            "$.bounds",
        ),
        (
            searching(&HEAT_BOUNDS, 1, POPULATION_MOST + 1, 4),
            "too_many",
            "$.population",
        ),
        (searching(&HEAT_BOUNDS, 1, 0, 4), "too_many", "$.population"),
        (
            searching(&HEAT_BOUNDS, 1, 8, GENERATIONS_MOST + 1),
            "too_many",
            "$.generations",
        ),
        (
            searching(&HEAT_BOUNDS, 1, 8, 0),
            "too_many",
            "$.generations",
        ),
        (
            searching(&HEAT_BOUNDS, 1, 8, 4).replace("\"seed\":1", "\"seed\":4294967296"),
            "malformed",
            "$.seed",
        ),
        (
            searching(&HEAT_BOUNDS, 1, 8, 4)
                .replace(",\"bounds\":[[0, 1023], [0, 15], [0, 4095]]", ""),
            "malformed",
            "$.bounds",
        ),
    ] {
        cases.push(Refused {
            tool: "grit_learn_params",
            arguments: inline.replace('@', &more),
            code,
            field,
        });
    }
    let mut server = Server::new(None);
    let told = checked_refusals(&mut server, &cases);
    let law = compiled(HEAT);
    let frame_units = 64 * (u64::try_from(law.nodes()).unwrap() + 128);
    let limit = told
        .iter()
        .find(|one| text_at(one, "code") == "run_limit")
        .and_then(|one| one.get("limit"))
        .and_then(Json::int)
        .unwrap();
    assert_eq!(
        u64::try_from(limit).unwrap(),
        WORK_MOST / frame_units,
        "the frames that fit"
    );
    let fits = answer(&call(
        &mut server,
        "grit_law_score",
        &at(size, &cells, params, &limit.to_string(), &aim).replace('@', ""),
    ));
    assert_eq!(
        int_at(&fits, "frames"),
        limit,
        "the limit itself is admitted"
    );
    println!("{} refusals, none ran", cases.len());
}

#[test]
fn s3_a_world_refusal_runs_nothing_and_leaves_every_world_as_it_was() {
    let Some(host) = device("learn_search world refusals") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let case = heat_case(8, 0x4EA7, &HEAT_TRUTH, &HEAT_GUESS, HEAT_FRAMES);
    let (learner, hidden) = opened_heat(&mut server, &case);
    let rule = int_at(
        &answer(&server.ask(
            "grit_world_create",
            "{\"wide\":8,\"deep\":8,\"tall\":8,\"rule\":\"sand\"}",
        )),
        "world",
    );
    let big_size = [32, 32, 64];
    assert!(u64::from(big_size.iter().product::<u32>()) > CELLS_MOST);
    let big = created(&mut server, HEAT, &HEAT_GUESS, big_size);
    let flat_size = [8, 8, 2];
    let flat = created(&mut server, HEAT, &HEAT_GUESS, flat_size);
    let aim = format!("{{{},\"world\":{hidden}}}", case.aim.shown());
    let search = searching(&HEAT_BOUNDS, 1, 8, 4);
    let mut cases = Vec::new();
    for (arguments, code, field) in [
        (
            format!("{{\"world\":{rule},\"frames\":8,\"aim\":{aim}@}}"),
            "start",
            "$.world",
        ),
        (
            format!("{{\"world\":{big},\"frames\":1,\"aim\":{aim}@}}"),
            "run_limit",
            "$.world",
        ),
        (
            format!("{{\"world\":{learner},\"size\":[8,8,8],\"frames\":8,\"aim\":{aim}@}}"),
            "start",
            "$.size",
        ),
        (
            format!("{{\"world\":{learner},\"first\":0,\"frames\":8,\"aim\":{aim}@}}"),
            "start",
            "$.first",
        ),
        (
            format!(
                "{{\"world\":{learner},\"frames\":8,\"aim\":{{{},\"world\":{flat}}}@}}",
                case.aim.shown()
            ),
            "outside",
            "$.aim.world",
        ),
        (
            format!("{{\"world\":{learner},\"params\":[1],\"frames\":8,\"aim\":{aim}@}}"),
            "params",
            "$.params",
        ),
        (
            format!(
                "{{\"world\":{learner},\"frames\":8,\"aim\":{{{},\"world\":4000}}@}}",
                case.aim.shown()
            ),
            "no_world",
            "$.aim.world",
        ),
    ] {
        cases.extend(both(&arguments, &search, code, field));
    }
    let worlds = [
        (learner, case.size),
        (hidden, case.size),
        (big, big_size),
        (flat, flat_size),
    ];
    let before: Vec<(i64, Vec<u32>)> = worlds
        .iter()
        .map(|&(world, size)| state_of(&mut server, world, size, 1))
        .collect();
    checked_refusals(&mut server, &cases);
    let after: Vec<(i64, Vec<u32>)> = worlds
        .iter()
        .map(|&(world, size)| state_of(&mut server, world, size, 1))
        .collect();
    assert_eq!(before, after, "no world moved");
}

const LOOP_GENERATIONS: u32 = 200;

const LOOP_BUDGET_MS: u64 = 120_000;

const LOOP_RUNS: u64 = 533;

fn client(asker: &mut dyn Ask) -> Vec<String> {
    let case = heat_case(8, 0x4EA7, &HEAT_TRUTH, &HEAT_GUESS, HEAT_FRAMES);
    let (learner, hidden) = opened_heat(asker, &case);
    let found = answer(&asker.ask(
        "grit_learn_params",
        &world_search(learner, hidden, &case, HEAT_SEED, LOOP_GENERATIONS).replace(
            &format!("\"budget_ms\":{LONG}"),
            &format!("\"budget_ms\":{LOOP_BUDGET_MS}"),
        ),
    ));
    assert_eq!(text_at(&found, "stopped"), "found", "{}", written(&found));
    assert_eq!(whole_of(found.get("objective").unwrap()), 0);
    assert_eq!(
        whole_of(found.get("scored").unwrap()),
        LOOP_RUNS,
        "as docs/law-worlds.md says"
    );
    let params = words_of(found.get("params").unwrap());
    let set = answer(&asker.ask(
        "grit_world_params",
        &format!("{{\"world\":{learner},\"params\":{params:?}}}"),
    ));
    step(asker, learner, case.frames);
    let (frame, learned) = state_of(asker, learner, case.size, 1);
    let (hidden_frame, wanted) = state_of(asker, hidden, case.size, 1);
    assert_eq!(
        (frame, &learned),
        (hidden_frame, &wanted),
        "the learner steps onto the hidden world"
    );
    vec![written(&found), written(&set)]
}

#[test]
fn s5_a_client_that_knows_only_the_mcp_recovers_hidden_heat_params_on_the_device() {
    let Some(host) = device("learn_search loop") else {
        return;
    };
    let mut runs_of = Vec::new();
    for _ in 0..2 {
        let mut server = Server::new(Some(&host));
        let began = std::time::Instant::now();
        runs_of.push(client(&mut server));
        println!("the loop took {:?}", began.elapsed());
    }
    assert_eq!(runs_of[0], runs_of[1], "the loop is deterministic");
}

#[test]
fn the_tools_are_listed_described_and_closed() {
    let mut server = Server::new(None);
    let listed = server
        .heard("{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}")
        .unwrap();
    let tools = listed
        .get("result")
        .and_then(|one| one.get("tools"))
        .and_then(Json::list)
        .unwrap()
        .to_vec();
    let names: Vec<&str> = tools
        .iter()
        .filter_map(|one| one.get("name").and_then(Json::text))
        .collect();
    let at = names.len() - 2;
    assert_eq!(names[at..], ["grit_law_score", "grit_learn_params"]);
    assert_eq!(
        grit::serve3::WORLD_TOOLS[grit::serve3::WORLD_TOOLS.len() - 2..],
        ["grit_law_score", "grit_learn_params"]
    );
    for tool in &tools[at..] {
        let schema = tool.get("inputSchema").unwrap();
        assert_eq!(schema.get("additionalProperties"), Some(&Json::Bool(false)));
        let properties = schema.get("properties").unwrap();
        let frames = properties.get("frames").unwrap();
        assert_eq!(
            frames.get("maximum").and_then(Json::int),
            Some(i64::from(FRAMES_MOST))
        );
        let description = text_at(tool, "description");
        assert!(description.len() < 200, "{description}");
    }
    let search = tools[at + 1]
        .get("inputSchema")
        .unwrap()
        .get("properties")
        .unwrap();
    assert_eq!(
        search
            .get("population")
            .and_then(|one| one.get("maximum"))
            .and_then(Json::int),
        Some(i64::from(POPULATION_MOST))
    );
    assert_eq!(
        search
            .get("generations")
            .and_then(|one| one.get("maximum"))
            .and_then(Json::int),
        Some(i64::from(GENERATIONS_MOST))
    );
    let described = answer(&call(&mut server, "grit_describe", "{}"));
    let limits = described.get("limits").unwrap();
    assert_eq!(text_at(limits, "law_search"), SEARCH);
    for (key, value) in [
        ("run_cells", CELLS_MOST),
        ("run_work", WORK_MOST),
        ("run_frames", u64::from(FRAMES_MOST)),
        ("search_population", u64::from(POPULATION_MOST)),
        ("search_generations", u64::from(GENERATIONS_MOST)),
        ("search_stale", u64::from(STALE)),
    ] {
        assert_eq!(
            limits.get(key).and_then(Json::int),
            Some(i64::try_from(value).unwrap()),
            "{key}"
        );
    }
    let codes: Vec<String> = described
        .get("codes")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .map(|one| text_at(one, "code"))
        .collect();
    for code in ["bounds", "run_limit"] {
        assert!(codes.iter().any(|one| one == code), "{code}");
    }
}

#[cfg(feature = "faults")]
mod faulted {
    use super::*;

    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::Receiver;
    use std::time::{Duration, Instant};

    use grit::Host;
    use grit::clock::{Fake, Time};
    use grit::serve3::Options;
    use grit::serve3::exec::Exec;
    use grit::serve3::learn::faults::{cancel_after, disarm};
    use grit::serve3::tools::faults::{disarm as unstall, stall_every};

    use super::delayed::Waiting;

    const RUN_FLOOR: Duration = Duration::from_millis(500);

    const WAITED: Duration = Duration::from_millis(4);

    const FLOOR: Duration = Duration::from_millis(20);

    const POPULATION: u32 = 4;

    fn args(case: &Case, budget: u64) -> String {
        case.arguments(&searching(&HEAT_BOUNDS, 2, POPULATION, 40))
            .replace(
                &format!("\"budget_ms\":{LONG}"),
                &format!("\"budget_ms\":{budget}"),
            )
    }

    fn climbed(trail: &[(Vec<u32>, u64)]) -> usize {
        (1..trail.len())
            .find(|&at| trail[at].1 > trail[..at].iter().map(|(_, one)| *one).min().unwrap())
            .unwrap_or_else(|| panic!("no generation is worse than the best before it: {trail:?}"))
    }

    fn stopped_at(got: &Json, whole: &[(Vec<u32>, u64)], generation: usize, why: &str) {
        let trail = trail_of(got);
        assert_eq!(text_at(got, "stopped"), why, "{}", written(got));
        assert_eq!(
            trail[..],
            whole[..=generation],
            "a prefix of the whole search"
        );
        let runs = 1 + u64::from(POPULATION) * u64::try_from(generation).unwrap();
        assert_eq!(whole_of(got.get("scored").unwrap()), runs);
        let (best, lowest) = earliest_lowest(&whole[..=generation]);
        assert_ne!(
            trail[generation].0, best,
            "the last generation's best is not the best"
        );
        assert_eq!(
            words_of(got.get("params").unwrap()),
            best,
            "the best so far"
        );
        assert_eq!(whole_of(got.get("objective").unwrap()), lowest);
        assert!(got.get("hint").is_some());
    }

    fn case() -> Case {
        heat_case(8, 0x4EA7, &HEAT_TRUTH, &HEAT_GUESS, HEAT_FRAMES)
    }

    fn stopped_inside(case: &Case, got: &Json, whole: &[(Vec<u32>, u64)], generation: usize) {
        let trail = trail_of(got);
        assert_eq!(text_at(got, "stopped"), "budget", "{}", written(got));
        assert_eq!(
            trail[..],
            whole[..generation],
            "a prefix of the whole search"
        );
        assert_eq!(
            int_at(got, "generations"),
            i64::try_from(generation - 1).unwrap()
        );
        let (best, lowest) = earliest_lowest(&whole[..generation]);
        assert!(
            whole[generation].1 > lowest,
            "every child of generation {generation} is worse than the best before it"
        );
        assert_eq!(
            words_of(got.get("params").unwrap()),
            best,
            "the best so far"
        );
        assert_eq!(whole_of(got.get("objective").unwrap()), lowest);
        assert_eq!(case.expected(&best), lowest);
        assert!(got.get("hint").is_some());
    }

    #[derive(Debug, PartialEq, Eq)]
    struct Told {
        answers: Vec<String>,
        runs: Vec<u64>,
        elapsed: Vec<Duration>,
        slept: Vec<Duration>,
        steps: u64,
    }

    struct Session {
        got: Vec<Json>,
        took: Vec<Duration>,
        told: Told,
    }

    fn session(time: Arc<dyn Time>, fake: &Fake, case: &Case, budget: u64, stall: bool) -> Session {
        let mut server = Server::timed(None, &Options::default(), time).unwrap();
        let mut got = Vec::new();
        let mut took = Vec::new();
        let mut told = Told {
            answers: Vec::new(),
            runs: Vec::new(),
            elapsed: Vec::new(),
            slept: Vec::new(),
            steps: 0,
        };
        if stall {
            stall_every(RUN_FLOOR);
        }
        for _ in 0..2 {
            let (at, slept, ran) = (fake.elapsed(), fake.slept(), runs().unwrap());
            let began = Instant::now();
            let one = answer(&call(&mut server, "grit_learn_params", &args(case, budget)));
            took.push(began.elapsed());
            told.elapsed.push(fake.elapsed() - at);
            told.slept.push(fake.slept() - slept);
            told.runs.push(runs().unwrap() - ran);
            assert_eq!(
                whole_of(one.get("scored").unwrap()),
                *told.runs.last().unwrap(),
                "scored counts the runs made"
            );
            told.answers.push(written(&one));
            got.push(one);
        }
        unstall();
        told.steps = fake.steps();
        Session { got, took, told }
    }

    fn faked(case: &Case, budget: u64, stall: bool) -> Session {
        let fake = Arc::new(Fake::new());
        session(
            Arc::clone(&fake) as Arc<dyn Time>,
            &fake,
            case,
            budget,
            stall,
        )
    }

    fn twice(case: &Case, budget: u64, stall: bool) -> Session {
        let first = faked(case, budget, stall);
        let second = faked(case, budget, stall);
        assert_eq!(
            first.told, second.told,
            "budget {budget}: one session twice in one process"
        );
        first
    }

    fn searched_whole(case: &Case) -> Vec<(Vec<u32>, u64)> {
        let fake = Arc::new(Fake::new());
        let mut server = Server::timed(
            None,
            &Options::default(),
            Arc::clone(&fake) as Arc<dyn Time>,
        )
        .unwrap();
        let whole = trail_of(&answer(&call(
            &mut server,
            "grit_learn_params",
            &args(case, LONG),
        )));
        assert_eq!(fake.elapsed(), Duration::ZERO);
        whole
    }

    fn ms(time: Duration) -> u64 {
        u64::try_from(time.as_millis()).unwrap()
    }

    #[test]
    fn s4_a_search_stopped_by_its_budget_answers_the_best_so_far_and_the_same_again() {
        let case = case();
        let whole = searched_whole(&case);
        let generation = climbed(&whole);
        assert!(generation <= 6, "{generation}");
        let runs = 1 + POPULATION * u32::try_from(generation).unwrap();
        for (budget, made) in [
            (RUN_FLOOR * runs + RUN_FLOOR / 2, runs),
            (RUN_FLOOR * runs, runs),
            (RUN_FLOOR * runs - Duration::from_millis(1), runs - 1),
        ] {
            let held = twice(&case, ms(budget), true);
            println!(
                "a budget of {budget:?} made {:?} runs in {:?} of fake time",
                held.told.runs, held.told.elapsed
            );
            for got in &held.got {
                if made == runs {
                    stopped_at(got, &whole, generation, "budget");
                } else {
                    stopped_inside(&case, got, &whole, generation);
                }
            }
            assert_eq!(
                held.told.runs,
                [u64::from(made); 2],
                "a budget of {budget:?} makes floor(budget / run) runs of {RUN_FLOOR:?}"
            );
            assert_eq!(held.told.elapsed, [RUN_FLOOR * made; 2], "{budget:?}");
            assert_eq!(held.told.slept, [RUN_FLOOR * made; 2], "{budget:?}");
            assert_eq!(held.told.steps, 0);
            assert_eq!(held.told.answers[0], held.told.answers[1]);
        }
    }

    #[test]
    fn s4_a_search_on_a_frozen_clock_answers_alike_at_any_budget_and_stops_on_none() {
        let case = case();
        let whole = searched_whole(&case);
        let held = [1, LONG].map(|budget| twice(&case, budget, false));
        for (budget, one) in [1, LONG].iter().zip(&held) {
            for got in &one.got {
                assert_ne!(text_at(got, "stopped"), "budget", "budget {budget}");
                assert_eq!(trail_of(got), whole, "budget {budget}: the whole search");
            }
            assert_eq!(one.told.elapsed, [Duration::ZERO; 2], "budget {budget}");
            assert_eq!(one.told.slept, [Duration::ZERO; 2], "budget {budget}");
            assert_eq!(one.told.answers[0], one.told.answers[1]);
        }
        assert_eq!(
            held[0].told, held[1].told,
            "a frozen clock answered differently at budgets of 1 ms and {LONG} ms"
        );
    }

    #[test]
    fn s4_a_search_under_a_waiting_clock_answers_as_on_the_fake_clock() {
        let case = case();
        let whole = searched_whole(&case);
        let generation = climbed(&whole);
        let runs = 1 + POPULATION * u32::try_from(generation).unwrap();
        for (budget, stall) in [
            (1, false),
            (LONG, false),
            (ms(RUN_FLOOR * runs + RUN_FLOOR / 2), true),
        ] {
            let plain = faked(&case, budget, stall);
            let waiting = Arc::new(Waiting::new(WAITED, WAITED));
            let delayed = session(
                Arc::clone(&waiting) as Arc<dyn Time>,
                waiting.fake(),
                &case,
                budget,
                stall,
            );
            let (waits, waited) = waiting.waited();
            println!(
                "budget {budget}: {waits} waits, {waited:?} in all, the calls took {:?}",
                delayed.took
            );
            assert_eq!(
                plain.told, delayed.told,
                "budget {budget}: real time spent at the clock moved the search"
            );
            if budget == 1 {
                for took in &delayed.took {
                    assert!(*took >= FLOOR, "budget {budget}: the call took {took:?}");
                }
            }
        }
    }

    #[test]
    fn s4_a_search_cancelled_between_runs_answers_the_best_so_far_and_the_same_again() {
        let case = case();
        let mut server = Server::new(None);
        let whole = trail_of(&answer(&call(
            &mut server,
            "grit_learn_params",
            &args(&case, LONG),
        )));
        let generation = climbed(&whole);
        let runs = 1 + u64::from(POPULATION) * u64::try_from(generation).unwrap();
        let mut answers = Vec::new();
        for _ in 0..2 {
            let flag = Arc::new(AtomicBool::new(false));
            server.arriving(None, Some(Arc::clone(&flag)));
            cancel_after(runs);
            let got = answer(&call(&mut server, "grit_learn_params", &args(&case, LONG)));
            assert!(flag.load(Ordering::SeqCst));
            server.arriving(None, None);
            stopped_at(&got, &whole, generation, "cancelled");
            answers.push(written(&got));
        }
        disarm();
        assert_eq!(answers[0], answers[1]);
        let flag = Arc::new(AtomicBool::new(false));
        server.arriving(None, Some(Arc::clone(&flag)));
        cancel_after(1);
        let scored = answer(&call(&mut server, "grit_law_score", &case.arguments("")));
        server.arriving(None, None);
        disarm();
        assert_eq!(
            whole_of(scored.get("objective").unwrap()),
            case.expected(&case.params),
            "a score is one run and answers whole"
        );
    }

    #[test]
    fn s3_a_run_the_budget_cannot_fit_is_refused_by_evidence_before_it_runs() {
        let case = case();
        let mut server = Server::new(None);
        stall_every(RUN_FLOOR * 2);
        let began = Instant::now();
        answer(&call(&mut server, "grit_law_score", &case.arguments("")));
        assert!(began.elapsed() >= RUN_FLOOR * 2);
        let short = u64::try_from(RUN_FLOOR.as_millis()).unwrap();
        let ran = runs();
        for (tool, more) in [
            ("grit_law_score", String::new()),
            ("grit_learn_params", searching(&HEAT_BOUNDS, 1, 8, 4)),
        ] {
            let arguments = case.arguments(&more).replace(
                &format!("\"budget_ms\":{LONG}"),
                &format!("\"budget_ms\":{short}"),
            );
            let told = refusal(&call(&mut server, tool, &arguments));
            assert_eq!(
                (text_at(&told, "code"), text_at(&told, "field")),
                ("budget".to_owned(), "$.budget_ms".to_owned()),
                "{tool}: {}",
                written(&told)
            );
            assert!(text_at(&told, "message").contains("nothing ran"));
        }
        assert_eq!(runs(), ran, "a budget refusal ran a run");
        unstall();
    }

    struct Run {
        exec: Exec,
        answers: Receiver<Json>,
        id: i64,
    }

    impl Run {
        fn new() -> Self {
            let pattern = std::env::var("GRIT_DEVICE")
                .ok()
                .filter(|one| !one.is_empty());
            let (exec, answers) = Exec::spawn(
                Box::new(move || Host::matching(pattern.as_deref())),
                &Options::default(),
            )
            .unwrap();
            Self {
                exec,
                answers,
                id: 100,
            }
        }
    }

    impl Ask for Run {
        fn ask(&mut self, name: &str, arguments: &str) -> Json {
            self.id += 1;
            self.exec.line(&line(self.id, name, arguments));
            loop {
                let got = self.answers.recv_timeout(Duration::from_secs(300)).unwrap();
                if got.get("id").and_then(Json::int) == Some(self.id) {
                    return got;
                }
            }
        }
    }

    #[test]
    fn s4_a_device_lost_during_a_search_leaves_every_world_and_the_next_answer_as_before() {
        let Some(host) = device("learn_search lost device") else {
            return;
        };
        drop(host);
        let case = case();
        let mut run = Run::new();
        let (learner, hidden) = opened_heat(&mut run, &case);
        let kept = |run: &mut Run| {
            (
                state_of(run, learner, case.size, 1),
                state_of(run, hidden, case.size, 1),
            )
        };
        let before = kept(&mut run);
        let arguments = world_search(learner, hidden, &case, 5, 12);
        let clean = answer(&run.ask("grit_learn_params", &arguments));
        assert!(whole_of(clean.get("scored").unwrap()) > 20);
        run.exec.faults().lock().unwrap().wedge_after_runs = Some(20);
        let during = answer(&run.ask("grit_learn_params", &arguments));
        let restored = during.get("restored").and_then(Json::list).unwrap();
        assert!(restored.len() >= 2, "{}", written(&during));
        assert_eq!(
            without_world(&during),
            without_world(&clean),
            "the runs are on the CPU: the answer stands"
        );
        assert_eq!(int_at(&during, "world"), learner);
        assert_eq!(kept(&mut run), before, "rebuilt as before");
        run.exec.faults().lock().unwrap().wedge_before_call = true;
        let told = refusal(&run.ask("grit_learn_params", &arguments));
        assert_eq!(text_at(&told, "code"), "device_lost", "{}", written(&told));
        assert_eq!(kept(&mut run), before, "rebuilt as before");
        let again = answer(&run.ask("grit_learn_params", &arguments));
        assert_eq!(written(&again), written(&clean));
        run.exec.finish();
    }
}
