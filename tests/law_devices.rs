#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use std::collections::BTreeMap;

use common::device;

use grit::Host;
use grit::branch::{Made, Stepping, World};
use grit::law::Law;
use grit::law::quiet;
use grit::sand3::Grid3;
use grit::serve3::Server;
use grit::wire::{Json, written};

const SIDE: u32 = 8;

const FRAMES: u32 = 3;

const SEED: u64 = 0x6E31_C0DE_D1F1_5EED;

const LAWS: u64 = 96;

const EDGES: [u32; 16] = [
    0,
    1,
    2,
    3,
    7,
    31,
    32,
    33,
    255,
    65_535,
    0x7FFF_FFFF,
    0x8000_0000,
    0x8000_0001,
    0xFFFF_FFFE,
    0xFFFF_FFFF,
    0x0001_0000,
];

const SHIFTS: [u32; 10] = [0, 1, 2, 15, 16, 31, 32, 33, 63, 0xFFFF_FFFF];

const WORDS: [&str; 9] = [
    "add", "sub", "mul", "div", "and", "or", "xor", "dot_s", "dot_u",
];

const COMPARES: [&str; 6] = ["eq", "ne", "lt", "le", "gt", "ge"];

fn grid() -> Grid3 {
    Grid3::new(SIDE, SIDE, SIDE).unwrap()
}

fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut mixed = *state;
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    mixed ^ (mixed >> 31)
}

fn below(state: &mut u64, count: usize) -> usize {
    (next(state) % count as u64) as usize
}

fn edge_or_any(state: &mut u64) -> u32 {
    if below(state, 4) == 0 {
        next(state) as u32
    } else {
        EDGES[below(state, EDGES.len())]
    }
}

fn seed() -> u64 {
    std::env::var("GRIT_FUZZ_SEED").ok().map_or(SEED, |held| {
        let held = held.trim();
        held.strip_prefix("0x").map_or_else(
            || held.parse().unwrap(),
            |hex| u64::from_str_radix(hex, 16).unwrap(),
        )
    })
}

fn laws() -> u64 {
    std::env::var("GRIT_FUZZ_LAWS")
        .ok()
        .map_or(LAWS, |held| held.parse().unwrap())
}

struct Shape {
    channels: u32,
    params: u32,
    frozen: Option<u32>,
    reversible: bool,
}

struct Writer<'held> {
    state: &'held mut u64,
    shape: &'held Shape,
    inputs: Vec<String>,
    used: &'held mut BTreeMap<&'static str, u64>,
}

impl Writer<'_> {
    fn note(&mut self, what: &'static str) {
        *self.used.entry(what).or_default() += 1;
    }

    fn leaf(&mut self) -> String {
        match below(self.state, 9) {
            0..=2 => {
                self.note("input");
                self.inputs[below(self.state, self.inputs.len())].clone()
            }
            3 | 4 => {
                self.note("constant");
                edge_or_any(self.state).to_string()
            }
            5 if self.shape.params > 0 => {
                self.note("param");
                format!(
                    "[\"param\",{}]",
                    below(self.state, self.shape.params as usize)
                )
            }
            6 => {
                self.note("frame");
                String::from("\"frame\"")
            }
            7 => {
                self.note("at");
                format!("[\"at\",\"at\",{}]", below(self.state, 3))
            }
            8 if self.shape.frozen.is_some() => {
                self.note("look");
                let plane = self.shape.frozen.unwrap();
                let offset = |state: &mut u64| below(state, 3) as i64 - 1;
                format!(
                    "[\"look\",{plane},{},{},{},{}]",
                    offset(self.state),
                    offset(self.state),
                    offset(self.state),
                    below(self.state, 8)
                )
            }
            _ if below(self.state, 8) == 0 => {
                self.note("noise");
                String::from("[\"noise\"]")
            }
            _ => {
                self.note("constant");
                edge_or_any(self.state).to_string()
            }
        }
    }

    fn truth(&mut self, depth: u32) -> String {
        match below(self.state, 8) {
            0 if depth > 0 => {
                let op = ["both", "either"][below(self.state, 2)];
                self.note(op);
                let left = self.truth(depth - 1);
                let right = self.truth(depth - 1);
                format!("[\"{op}\",{left},{right}]")
            }
            1 if depth > 0 => {
                self.note("denied");
                let inner = self.truth(depth - 1);
                format!("[\"denied\",{inner}]")
            }
            _ => {
                let op = COMPARES[below(self.state, COMPARES.len())];
                self.note(op);
                let left = self.word(depth.saturating_sub(1));
                let right = self.word(depth.saturating_sub(1));
                format!("[\"{op}\",{left},{right}]")
            }
        }
    }

    fn word(&mut self, depth: u32) -> String {
        if depth == 0 {
            return self.leaf();
        }
        match below(self.state, 16) {
            0 | 1 => self.leaf(),
            2 => {
                self.note("not");
                let inner = self.word(depth - 1);
                format!("[\"not\",{inner}]")
            }
            3 | 4 => {
                let op = ["shl", "shr"][below(self.state, 2)];
                self.note(op);
                let inner = self.word(depth - 1);
                let by = SHIFTS[below(self.state, SHIFTS.len())];
                format!("[\"{op}\",{inner},{by}]")
            }
            5..=7 => {
                self.note("sel");
                let truth = self.truth(depth - 1);
                let then = self.word(depth - 1);
                let otherwise = self.word(depth - 1);
                format!("[\"sel\",{truth},{then},{otherwise}]")
            }
            8 => {
                self.note("word");
                let truth = self.truth(depth - 1);
                format!("[\"word\",{truth}]")
            }
            9 => {
                self.note("count");
                let one = self.truth(depth - 1);
                let other = self.truth(depth - 1);
                format!("[\"count\",[\"vec\",{one},{other}]]")
            }
            10 => {
                self.note("div");
                let dividend = edge_or_any(self.state);
                let divisor = self.word(depth - 1);
                format!("[\"div\",{dividend},{divisor}]")
            }
            _ => {
                let op = WORDS[below(self.state, WORDS.len())];
                self.note(op);
                let left = self.word(depth - 1);
                let right = self.word(depth - 1);
                format!("[\"{op}\",{left},{right}]")
            }
        }
    }
}

fn input_of(shape: &Shape, corner: u32, plane: u32) -> String {
    if shape.channels == 1 {
        format!("[\"at\",\"cells\",{corner}]")
    } else {
        format!("[\"at\",[\"at\",\"cells\",{corner}],{plane}]")
    }
}

fn header(name: &str, shape: &Shape, halo: u32) -> String {
    format!(
        "{{\"law\":1,\"name\":\"{name}\",\"params\":{},\"channels\":{},\"halo\":{halo},{}\"tally\":{{\"counts\":1,\"weighted\":false}},\"consts\":{{}},\"defs\":{{}},",
        shape.params,
        shape.channels,
        if shape.reversible {
            "\"reversible\":true,"
        } else {
            ""
        }
    )
}

fn plain_law(state: &mut u64, name: &str, used: &mut BTreeMap<&'static str, u64>) -> String {
    let channels = 1 + below(state, 3) as u32;
    let frozen = (channels > 1 && below(state, 2) == 0).then_some(channels - 1);
    let shape = Shape {
        channels,
        params: below(state, 3) as u32,
        frozen,
        reversible: false,
    };
    let mut bindings = Vec::new();
    let mut inputs = Vec::new();
    for corner in 0..8 {
        for plane in 0..channels {
            let named = format!("i{corner}p{plane}");
            bindings.push(format!("[\"{named}\",{}]", input_of(&shape, corner, plane)));
            inputs.push(format!("\"{named}\""));
        }
    }
    let depth = 2 + below(state, 3) as u32;
    let mut writer = Writer {
        state,
        shape: &shape,
        inputs,
        used,
    };
    let halo = u32::from(shape.frozen.is_some());
    if let Some(plane) = shape.frozen {
        bindings.push(format!("[\"edge\",[\"look\",{plane},1,1,1,7]]"));
    }
    let mut outs = Vec::new();
    for corner in 0..8 {
        let mut planes = Vec::new();
        for plane in 0..channels {
            if Some(plane) == shape.frozen {
                planes.push(format!("\"i{corner}p{plane}\""));
            } else if halo == 1 && corner == 0 && plane == 0 {
                let inner = writer.word(depth);
                planes.push(format!("[\"add\",{inner},\"edge\"]"));
            } else {
                planes.push(writer.word(depth));
            }
        }
        outs.push(if channels == 1 {
            planes.remove(0)
        } else {
            format!("[\"vec\",{}]", planes.join(","))
        });
    }
    let count = writer.word(depth);
    format!(
        "{}\"step\":[\"let\",[{}],[\"vec\",[\"vec\",{}],[\"vec\",{count}]]]}}",
        header(name, &shape, halo),
        bindings.join(","),
        outs.join(",")
    )
}

fn lifting_law(state: &mut u64, name: &str, used: &mut BTreeMap<&'static str, u64>) -> String {
    let shape = Shape {
        channels: 1,
        params: below(state, 3) as u32,
        frozen: None,
        reversible: true,
    };
    let mut version = [0_u32; 8];
    let mut bindings: Vec<String> = (0..8)
        .map(|k| format!("[\"w{k}v0\",[\"at\",\"cells\",{k}]]"))
        .collect();
    let steps = 1 + below(state, 4);
    let depth = 1 + below(state, 3) as u32;
    for _ in 0..steps {
        let target = below(state, 8);
        let others: Vec<String> = (0..8)
            .filter(|&k| k != target)
            .map(|k| format!("\"w{k}v{}\"", version[k]))
            .collect();
        let mut writer = Writer {
            state,
            shape: &shape,
            inputs: others,
            used,
        };
        let value = writer.word(depth);
        let op = ["add", "sub", "xor"][below(state, 3)];
        used.entry("lifting").and_modify(|n| *n += 1).or_insert(1);
        let old = format!("w{target}v{}", version[target]);
        version[target] += 1;
        bindings.push(format!(
            "[\"w{target}v{}\",[\"{op}\",\"{old}\",{value}]]",
            version[target]
        ));
    }
    let outs: Vec<String> = (0..8).map(|k| format!("\"w{k}v{}\"", version[k])).collect();
    format!(
        "{}\"step\":[\"let\",[{}],[\"vec\",[\"vec\",{}],[\"vec\",0]]]}}",
        header(name, &shape, 0),
        bindings.join(","),
        outs.join(",")
    )
}

fn start(law: &Law, state: &mut u64) -> Vec<u32> {
    let planes = grit::rule3::Declared::planes(law);
    (0..grid().cells() * planes)
        .map(|_| edge_or_any(state))
        .collect()
}

fn replayed(law: &Law, cells: &[u32], frames: u32, params: &[u32]) -> Vec<Vec<u32>> {
    let mut held = cells.to_vec();
    let mut all = Vec::new();
    for frame in 0..frames {
        held = grit::sand3::reference(law, grid(), frame, &held, params);
        all.push(held.clone());
    }
    all
}

fn stepping(law: &Law, params: &[u32]) -> Stepping {
    if quiet::class(law, params, &[]).proved() {
        Stepping::Listed
    } else {
        Stepping::Whole
    }
}

fn written_world<'host>(
    host: &'host Host,
    law: &Law,
    params: &[u32],
    cells: &[u32],
) -> Result<World<'host>, String> {
    let mut world = World::create_law_stepped(
        host,
        law,
        &Made {
            rule: law.name(),
            params,
            size: [SIDE, SIDE, SIDE],
            sparse: false,
            slots: None,
        },
        stepping(law, params),
    )
    .map_err(|why| format!("create: {why}"))?;
    let g = grid();
    for plane in 0..world.planes() {
        let mut placed = Vec::with_capacity(g.cells());
        for z in 0..SIDE {
            for y in 0..SIDE {
                for x in 0..SIDE {
                    let at = plane as usize * g.cells() + g.index(x, y, z);
                    placed.push(([x, y, z], cells[at]));
                }
            }
        }
        world
            .write_plane(plane, &placed)
            .map_err(|why| format!("write: {why}"))?;
    }
    Ok(world)
}

fn stepped_like_its_twin(
    host: &Host,
    law: &Law,
    params: &[u32],
    cells: &[u32],
) -> Result<(), String> {
    let want = replayed(law, cells, FRAMES, params);
    let mut world = written_world(host, law, params, cells)?;
    for (frame, want) in want.iter().enumerate() {
        world
            .step(1)
            .map_err(|why| format!("step {frame}: {why}"))?;
        let got = world.cells().map_err(|why| format!("read: {why}"))?;
        if &got != want {
            let at = got.iter().zip(want).position(|(a, b)| a != b).unwrap();
            return Err(format!(
                "frame {frame}: cell {at} is {} on the device and {} on the CPU",
                got[at], want[at]
            ));
        }
    }
    Ok(())
}

fn params_of(law: &Law, state: &mut u64) -> Vec<u32> {
    (0..grit::rule3::Declared::params(law))
        .map(|_| edge_or_any(state))
        .collect()
}

#[derive(Default)]
struct Tally {
    made: u64,
    refused: u64,
    built: u64,
    inverses: u64,
    failed: Vec<String>,
}

fn fuzzed(host: &Host, seed: u64, count: u64, used: &mut BTreeMap<&'static str, u64>) -> Tally {
    let mut tally = Tally::default();
    let mut state = seed;
    for index in 0..count {
        let name = format!("fuzz{index}");
        let text = if below(&mut state, 5) == 0 {
            lifting_law(&mut state, &name, used)
        } else {
            plain_law(&mut state, &name, used)
        };
        tally.made += 1;
        let law = match Law::compile(&text) {
            Ok(law) => law,
            Err(why) => {
                tally.refused += 1;
                if why.code != grit::law::Code::Lifting {
                    tally
                        .failed
                        .push(format!("law {index} refused: {why}\n{text}"));
                }
                continue;
            }
        };
        let params = params_of(&law, &mut state);
        let cells = start(&law, &mut state);
        match stepped_like_its_twin(host, &law, &params, &cells) {
            Ok(()) => tally.built += 1,
            Err(why) => tally.failed.push(format!("law {index}: {why}\n{text}")),
        }
        if let Ok(Some(back)) = law.inverse() {
            match stepped_like_its_twin(host, back, &params, &cells) {
                Ok(()) => tally.inverses += 1,
                Err(why) => tally
                    .failed
                    .push(format!("law {index} inverse: {why}\n{text}")),
            }
        }
    }
    tally
}

#[test]
fn every_fuzzed_law_builds_and_steps_like_its_cpu_twin() {
    let Some(host) = device("every_fuzzed_law_builds_and_steps_like_its_cpu_twin") else {
        return;
    };
    let seed = seed();
    let count = laws();
    let mut used = BTreeMap::new();
    let tally = fuzzed(&host, seed, count, &mut used);
    println!(
        "{}: seed {seed:#x}, {} laws made, {} refused as not lifting, {} built and equal over {FRAMES} frames, {} inverses built and equal, {} failed",
        host.name(),
        tally.made,
        tally.refused,
        tally.built,
        tally.inverses,
        tally.failed.len()
    );
    println!("operations used: {used:?}");
    for one in &tally.failed {
        println!("FAILED {one}");
    }
    assert!(
        tally.failed.is_empty(),
        "{} laws failed",
        tally.failed.len()
    );
    assert!(tally.built * 4 >= tally.made * 3, "too few laws built");
    for op in [
        "add", "sub", "mul", "div", "and", "or", "xor", "dot_s", "dot_u", "not", "shl", "shr",
        "eq", "ne", "lt", "le", "gt", "ge", "sel", "both", "either", "denied", "word", "count",
        "param", "frame", "at", "look", "noise", "input", "constant", "lifting",
    ] {
        assert!(
            used.get(op).copied().unwrap_or(0) > 0,
            "the fuzz never used {op}"
        );
    }
}

fn lifting_text(name: &str, bindings: &[&str]) -> String {
    let out: Vec<String> = (0..8)
        .map(|k| {
            let last = bindings
                .iter()
                .rev()
                .find_map(|one| {
                    let head = format!("[\"w{k}v");
                    one.starts_with(&head)
                        .then(|| one[2..].split('"').next().unwrap().to_owned())
                })
                .unwrap_or(format!("w{k}v0"));
            format!("\"{last}\"")
        })
        .collect();
    let mut all: Vec<String> = (0..8)
        .map(|k| format!("[\"w{k}v0\",[\"at\",\"cells\",{k}]]"))
        .collect();
    all.extend(bindings.iter().map(|one| (*one).to_owned()));
    format!(
        "{{\"law\":1,\"name\":\"{name}\",\"params\":1,\"channels\":1,\"halo\":0,\"reversible\":true,\"tally\":{{\"counts\":1,\"weighted\":false}},\"consts\":{{}},\"defs\":{{}},\"step\":[\"let\",[{}],[\"vec\",[\"vec\",{}],[\"vec\",0]]]}}",
        all.join(","),
        out.join(",")
    )
}

const DIVIDER: &str =
    "[\"w7v1\",[\"sub\",\"w7v0\",[\"sel\",[\"lt\",[\"div\",4294967295,\"w3v0\"],\"frame\"],1,0]]]";

fn call(server: &mut Server<'_>, name: &str, arguments: &str) -> Json {
    let line = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{{\"name\":\"{name}\",\"arguments\":{arguments}}}}}"
    );
    server.heard(&line).unwrap()
}

fn content(got: &Json) -> (bool, Json) {
    let held = got.get("result").unwrap();
    (
        held.get("isError") == Some(&Json::Bool(true)),
        held.get("structuredContent").cloned().unwrap(),
    )
}

#[test]
fn the_a1b_lifting_law_builds_and_steps_like_its_cpu_twin() {
    let Some(host) = device("the_a1b_lifting_law_builds_and_steps_like_its_cpu_twin") else {
        return;
    };
    let text = lifting_text("divider", &[DIVIDER]);
    let law = Law::compile(&text).unwrap();
    let mut server = Server::new(Some(&host));
    let got = call(
        &mut server,
        "grit_world_create",
        &format!(
            "{{\"wide\":{SIDE},\"deep\":{SIDE},\"tall\":{SIDE},\"law\":{},\"params\":[0]}}",
            written(&Json::Text(text.clone()))
        ),
    );
    let (error, held) = content(&got);
    assert!(!error, "{} on {}", written(&held), host.name());
    let mut state = SEED;
    let cells: Vec<u32> = (0..grid().cells())
        .map(|at| match at % 3 {
            0 => at as u32,
            1 => edge_or_any(&mut state),
            _ => 0,
        })
        .collect();
    for params in [[0_u32], [7], [u32::MAX]] {
        stepped_like_its_twin(&host, &law, &params, &cells).unwrap();
        let back = law.inverse().unwrap().unwrap();
        stepped_like_its_twin(&host, back, &params, &cells).unwrap();
    }
    let want = replayed(&law, &cells, FRAMES, &[0]);
    assert_ne!(want[FRAMES as usize - 1], cells, "the divider never fired");
}

#[cfg(feature = "faults")]
mod pipeline {
    use super::*;

    use ash::vk;
    use grit::resident::faults::{self, Step};

    fn answered(server: &mut Server<'_>, name: &str, arguments: &str) -> Json {
        let (error, held) = content(&call(server, name, arguments));
        assert!(!error, "{name}: {}", written(&held));
        held
    }

    fn field(held: &Json, key: &str) -> Json {
        held.get(key)
            .cloned()
            .unwrap_or_else(|| panic!("no {key} in {}", written(held)))
    }

    fn read_world(server: &mut Server<'_>, world: i64) -> Json {
        let last = SIDE - 1;
        let mut cells = Vec::new();
        for z in (0..SIDE).step_by(4) {
            let got = answered(
                server,
                "grit_world_read",
                &format!(
                    "{{\"world\":{world},\"from\":[0,0,{z}],\"to\":[{last},{last},{}]}}",
                    z + 3
                ),
            );
            cells.push(field(&got, "cells"));
        }
        Json::List(cells)
    }

    fn seen(server: &mut Server<'_>, world: i64) -> Vec<Json> {
        let described = answered(server, "grit_describe", "{}");
        vec![
            field(&described, "memory"),
            field(&described, "worlds"),
            answered(server, "grit_world_list", "{}"),
            read_world(server, world),
        ]
    }

    fn making(text: &str) -> String {
        format!(
            "{{\"wide\":{SIDE},\"deep\":{SIDE},\"tall\":{SIDE},\"law\":{},\"params\":[0]}}",
            written(&Json::Text(text.to_owned()))
        )
    }

    #[test]
    fn a_kernel_the_driver_cannot_compile_is_a_named_refusal_at_the_law_and_changes_nothing() {
        let Some(host) = device(
            "a_kernel_the_driver_cannot_compile_is_a_named_refusal_at_the_law_and_changes_nothing",
        ) else {
            return;
        };
        let text = lifting_text("divider", &[DIVIDER]);
        let mut server = Server::new(Some(&host));
        let first = answered(&mut server, "grit_world_create", &making(&text));
        let world = first.get("world").and_then(Json::int).unwrap();
        let mut placed = Vec::new();
        for at in 0..64_u32 {
            placed.push(format!(
                "[{},{},{},{}]",
                at % SIDE,
                at / SIDE,
                at % 3,
                at * 997 + 5
            ));
        }
        answered(
            &mut server,
            "grit_world_write",
            &format!("{{\"world\":{world},\"cells\":[{}]}}", placed.join(",")),
        );
        answered(
            &mut server,
            "grit_world_step",
            &format!("{{\"world\":{world},\"frames\":2}}"),
        );
        let before = seen(&mut server, world);
        let failures = [
            (Step::Pipeline, 0, vk::Result::ERROR_UNKNOWN),
            (Step::Pipeline, 1, vk::Result::ERROR_UNKNOWN),
            (Step::Pipeline, 0, vk::Result::ERROR_INITIALIZATION_FAILED),
            (Step::Pipeline, 0, vk::Result::ERROR_INVALID_SHADER_NV),
            (Step::Shader, 0, vk::Result::ERROR_UNKNOWN),
            (Step::Shader, 2, vk::Result::ERROR_INVALID_SHADER_NV),
        ];
        for (step, nth, code) in failures {
            faults::reset();
            faults::arm_with(step, nth, code);
            let (error, told) = content(&call(&mut server, "grit_world_create", &making(&text)));
            faults::disarm();
            let label = format!("{step:?} {nth} {code:?}");
            assert!(faults::fired(), "{label}: the fault never fired");
            assert!(error, "{label}: {}", written(&told));
            assert_eq!(
                field(&told, "code"),
                Json::Text("law_not_built".into()),
                "{label}: {}",
                written(&told)
            );
            assert_eq!(field(&told, "field"), Json::Text("$.law".into()), "{label}");
            assert_eq!(
                field(&told, "device"),
                Json::Text(host.name().into()),
                "{label}"
            );
            assert_eq!(
                field(&told, "vulkan"),
                Json::Text(format!("{code:?}")),
                "{label}"
            );
            let message = field(&told, "message");
            let message = message.text().unwrap();
            assert!(message.contains(host.name()), "{label}: {message}");
            assert!(message.contains(&format!("{code:?}")), "{label}: {message}");
            assert!(
                field(&told, "hint").text().unwrap().contains("law"),
                "{label}: {}",
                written(&told)
            );
            for kind in Step::ALL {
                let count = faults::count(kind);
                assert_eq!(count.made, count.gone, "{label}: {kind:?} leaked {count:?}");
            }
            assert_eq!(
                seen(&mut server, world),
                before,
                "{label}: the server changed"
            );
        }
        faults::reset();
        faults::arm(Step::Pipeline, 0);
        let (error, told) = content(&call(&mut server, "grit_world_create", &making(&text)));
        faults::disarm();
        assert!(faults::fired() && error, "{}", written(&told));
        assert_ne!(
            field(&told, "code"),
            Json::Text("law_not_built".into()),
            "{}",
            written(&told)
        );
        assert_eq!(
            seen(&mut server, world),
            before,
            "out of memory changed the server"
        );
        let next = answered(&mut server, "grit_world_create", &making(&text));
        assert_eq!(next.get("world").and_then(Json::int), Some(world + 1));
        let law = Law::compile(&text).unwrap();
        let cells: Vec<u32> = (0..grid().cells()).map(|at| at as u32 * 997).collect();
        stepped_like_its_twin(&host, &law, &[0], &cells).unwrap();
    }
}
