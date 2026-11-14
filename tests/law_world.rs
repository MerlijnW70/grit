#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use grit::Host;
use grit::branch::{Made, Stepping, World};
use grit::law::Law;
use grit::law::quiet::{self, Class, Why};
use grit::rule3::{code, embed, evo, heat, nature};
use grit::sand3::Grid3;
use grit::serve3::{Options, Server};
use grit::wire::{Json, read, written};

const SIDE: u32 = 32;

const FRAMES: u32 = 256;

const BATCH: u32 = 32;

const SAND: &str = include_str!("law/sand.law.json");
const HEAT: &str = include_str!("law/heat.law.json");
const CODE: &str = include_str!("law/code.law.json");
const NATURE: &str = include_str!("law/nature.law.json");
const EVO: &str = include_str!("law/evo.law.json");
const EMBED: &str = include_str!("law/embed.law.json");

const BLINK: &str = r#"{"law":1,"name":"blink","params":0,"channels":1,"halo":0,"tally":{"counts":1,"weighted":false},"consts":{},"defs":{},
"step":["let",[["odd",["eq",["and","frame",1],1]],["out",["map","w","cells",["sel",["both","odd",["ne","w",0]],["add","w",1],"w"]]]],
["vec","out",["vec",["count",["map","k",["range",8],["ne",["at","cells","k"],["at","out","k"]]]]]]]}"#;

const PULSE: &str = r#"{"law":1,"name":"pulse","params":0,"channels":1,"halo":0,"tally":{"counts":1,"weighted":false},"consts":{},"defs":{},
"step":["let",[["due",["eq",["and","frame",7],5]],["out",["map","w","cells",["sel",["both","due",["ne","w",0]],["add","w",1],"w"]]]],
["vec","out",["vec",["count",["map","k",["range",8],["ne",["at","cells","k"],["at","out","k"]]]]]]]}"#;

const FILL: &str = r#"{"law":1,"name":"fill","params":0,"channels":1,"halo":0,"tally":{"counts":1,"weighted":false},"consts":{},"defs":{},
"step":["let",[["out",["map","w","cells",["or","w",1]]]],
["vec","out",["vec",["count",["map","k",["range",8],["ne",["at","cells","k"],["at","out","k"]]]]]]]}"#;

const KEEP: &str = r#"{"law":1,"name":"keep","params":0,"channels":1,"halo":0,"tally":{"counts":1,"weighted":false},"consts":{},"defs":{},
"step":["let",[["out",["map","w","cells",["sel",["eq","w",0],0,["or","w",1]]]]],
["vec","out",["vec",["count",["map","k",["range",8],["ne",["at","cells","k"],["at","out","k"]]]]]]]}"#;

const BIAS: &str = r#"{"law":1,"name":"bias","params":2,"channels":1,"halo":0,"tally":{"counts":1,"weighted":false},"consts":{"MARK":7},"defs":{},
"step":["let",[["out",["map","w","cells",["sel",["eq","w",0],["param",0],["sel",["eq","w","MARK"],["param",1],"w"]]]]],
["vec","out",["vec",["count",["map","k",["range",8],["ne",["at","cells","k"],["at","out","k"]]]]]]]}"#;

const DEAD: &str = r#"{"law":1,"name":"dead","params":0,"channels":1,"halo":0,"tally":{"counts":1,"weighted":false},"consts":{},"defs":{},
"step":["let",[["unused",["add","frame",["at","at",0]]],["out",["map","w","cells",["xor","w",0]]]],
["vec","out",["vec",["add","frame",["count",["map","k",["range",8],["ne",["at","cells","k"],["at","out","k"]]]]]]]]}"#;

const STILL_TWO: &str = r#"{"law":1,"name":"still2","params":0,"channels":2,"halo":0,"tally":{"counts":1,"weighted":false},"consts":{},"defs":{},
"step":["let",[["out",["map","c","cells",["vec",["at","c",0],["sel",["eq",["at","c",1],9],["sel",["eq",["at","c",0],0],5,9],["at","c",1]]]]]],
["vec","out",["vec",0]]]}"#;

fn tumble(name: &str, gated: bool) -> String {
    let result = if gated {
        r#"["map","k",["range",8],["sel",["eq",["and","frame",2],0],["at","out","k"],["at","cells","k"]]]"#
    } else {
        r#""out""#
    };
    format!(
        concat!(
            r#"{{"law":1,"name":"{name}","params":1,"channels":1,"halo":0,"tally":{{"counts":1,"weighted":false}},"consts":{{}},"#,
            r#""defs":{{"#,
            r#""empty":[["w"],["eq","w",0]],"#,
            r#""mobile":[["w"],["ge","w",["param",0]]],"#,
            r#""swap":[["c","a","b","go"],["put",["put","c","a",["sel","go",["at","c","b"],["at","c","a"]]],"b",["sel","go",["at","c","a"],["at","c","b"]]]],"#,
            r#""fall":[["c","t"],["swap","c","t",["xor","t",4],["both",["mobile",["at","c","t"]],["empty",["at","c",["xor","t",4]]]]]],"#,
            r#""slide":[["c","t","axis"],["swap","c","t",["xor",["xor","t","axis"],4],"#,
            r#"["both",["both",["mobile",["at","c","t"]],["denied",["empty",["at","c",["xor","t",4]]]]],"#,
            r#"["both",["empty",["at","c",["xor","t","axis"]]],["empty",["at","c",["xor",["xor","t","axis"],4]]]]]]]"#,
            r#"}},"#,
            r#""step":["let",[["a",["fold","c","cells","t",["range",4],["fall","c","t"]]],"#,
            r#"["b",["fold","c","a","t",["range",4],["slide","c","t","X"]]],"#,
            r#"["out",["fold","c","b","t",["range",4],["slide","c","t","Z"]]],"#,
            r#"["result",{result}]],"#,
            r#"["vec","result",["vec",["count",["map","k",["range",8],["ne",["at","cells","k"],["at","result","k"]]]]]]]}}"#
        ),
        name = name,
        result = result,
    )
}

fn compiled(text: &str) -> Law {
    Law::compile(text).unwrap_or_else(|why| panic!("{why}"))
}

fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut mixed = *state;
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    mixed ^ (mixed >> 31)
}

fn sand_cell(state: &mut u64) -> u32 {
    [grit::sand::SAND, grit::sand::WATER, grit::sand::WALL][(next(state) % 3) as usize]
}

fn heat_cell(state: &mut u64) -> u32 {
    let roll = next(state);
    let kind = [heat::WALL, heat::WATER, heat::FUEL, heat::FIRE, heat::ICE][(roll % 5) as usize];
    heat::cell(kind, ((roll >> 8) % 1200) as u32)
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

fn evo_cell(state: &mut u64) -> u32 {
    let roll = next(state);
    let genome = ((roll >> 24) & u64::from(evo::GENOME_MASK)) as u32;
    evo::cell(genome, 128 + ((roll >> 48) % 128) as u32, 0)
}

fn embed_cell(state: &mut u64) -> u32 {
    let roll = next(state);
    match roll % 4 {
        0 => embed::embedding([(roll >> 8) as i8, (roll >> 16) as i8, 0, 0]),
        1 => ((roll >> 24) % 5) as u32,
        _ => (roll >> 16) as u32,
    }
}

fn small_cell(state: &mut u64) -> u32 {
    2 + (next(state) % 9) as u32
}

fn grid() -> Grid3 {
    Grid3::new(SIDE, SIDE, SIDE).unwrap()
}

fn landscape(cell: fn(&mut u64) -> u32, seed: u64) -> Vec<u32> {
    let mut state = seed;
    let side = SIDE as usize;
    let mut cells = vec![0_u32; side * side * side];
    for (index, slot) in cells.iter_mut().enumerate() {
        let x = index % side;
        let y = (index / side) % side;
        let z = index / (side * side);
        let inside = (6..26).contains(&x) && (4..20).contains(&y) && (8..14).contains(&z);
        if (inside && next(&mut state).is_multiple_of(3)) || next(&mut state).is_multiple_of(97) {
            *slot = cell(&mut state);
        }
    }
    cells
}

fn replay(law: &Law, cells: &[u32], from: u32, frames: u32, params: &[u32]) -> Vec<u32> {
    let mut held = cells.to_vec();
    for frame in from..from + frames {
        held = grit::sand3::reference(law, grid(), frame, &held, params);
    }
    held
}

fn line(id: i64, name: &str, arguments: &str) -> String {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"tools/call\",\"params\":{{\"name\":\"{name}\",\"arguments\":{arguments}}}}}"
    )
}

fn call(server: &mut Server<'_>, name: &str, arguments: &str) -> Json {
    server.heard(&line(7, name, arguments)).unwrap()
}

fn answer(got: &Json) -> Json {
    let result = got.get("result").unwrap();
    assert_eq!(
        result.get("isError"),
        Some(&Json::Bool(false)),
        "{}",
        written(got)
    );
    result.get("structuredContent").cloned().unwrap()
}

fn refusal(got: &Json) -> Json {
    let result = got.get("result").unwrap();
    assert_eq!(
        result.get("isError"),
        Some(&Json::Bool(true)),
        "{}",
        written(got)
    );
    result.get("structuredContent").cloned().unwrap()
}

trait At {
    fn int_at(&self, key: &str) -> i64;
    fn text_at(&self, key: &str) -> String;
}

impl At for Json {
    fn int_at(&self, key: &str) -> i64 {
        self.get(key)
            .and_then(Json::int)
            .unwrap_or_else(|| panic!("no {key} in {}", written(self)))
    }

    fn text_at(&self, key: &str) -> String {
        self.get(key)
            .and_then(Json::text)
            .unwrap_or_else(|| panic!("no {key} in {}", written(self)))
            .to_owned()
    }
}

fn quoted(text: &str) -> String {
    written(&Json::Text(text.to_owned()))
}

fn making_law(law: &str, params: &[u32], sparse: bool) -> String {
    format!(
        "{{\"wide\":{SIDE},\"deep\":{SIDE},\"tall\":{SIDE},\"law\":{},\"params\":{params:?},\"sparse\":{sparse}}}",
        quoted(law)
    )
}

fn making_rule(rule: &str, params: &[u32], sparse: bool) -> String {
    format!(
        "{{\"wide\":{SIDE},\"deep\":{SIDE},\"tall\":{SIDE},\"rule\":\"{rule}\",\"params\":{params:?},\"sparse\":{sparse}}}"
    )
}

fn placed(cells: &[u32]) -> Vec<String> {
    let grid = grid();
    let mut out = Vec::new();
    for z in 0..SIDE {
        for y in 0..SIDE {
            for x in 0..SIDE {
                let word = cells[grid.index(x, y, z)];
                if word != 0 {
                    out.push(format!("[{x},{y},{z},{word}]"));
                }
            }
        }
    }
    out
}

fn writes(cells: &[u32]) -> Vec<String> {
    placed(cells)
        .chunks(4096)
        .map(|chunk| format!("[{}]", chunk.join(",")))
        .collect()
}

fn write_all(server: &mut Server<'_>, world: i64, cells: &[u32]) {
    for chunk in writes(cells) {
        answer(&call(
            server,
            "grit_world_write",
            &format!("{{\"world\":{world},\"cells\":{chunk},\"budget_ms\":600000}}"),
        ));
    }
}

fn slab(world: i64, z: u32) -> String {
    let last = SIDE - 1;
    format!(
        "{{\"world\":{world},\"from\":[0,0,{z}],\"to\":[{last},{last},{}]}}",
        z + 3
    )
}

fn numbers(held: &Json) -> Vec<u32> {
    held.get("cells")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .map(|one| u32::try_from(one.int().unwrap()).unwrap())
        .collect()
}

fn read_all(server: &mut Server<'_>, world: i64) -> Vec<u32> {
    let mut cells = Vec::new();
    for z in (0..SIDE).step_by(4) {
        cells.extend(numbers(&answer(&call(
            server,
            "grit_world_read",
            &slab(world, z),
        ))));
    }
    cells
}

fn stepped(server: &mut Server<'_>, world: i64, frames: u32) -> Json {
    answer(&call(
        server,
        "grit_world_step",
        &format!("{{\"world\":{world},\"frames\":{frames},\"budget_ms\":600000}}"),
    ))
}

fn state(server: &mut Server<'_>, world: i64, name: &str) -> String {
    answer(&call(
        server,
        "grit_world_save",
        &format!("{{\"world\":{world},\"name\":\"{name}\",\"replace\":true}}"),
    ))
    .text_at("state")
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "grit-lawworld-{label}-{}-{nanos}",
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

fn served<'host>(host: &'host Host, scratch: &Scratch) -> Server<'host> {
    Server::with(
        Some(host),
        &Options {
            dir: Some(scratch.dir()),
            memory: None,
        },
    )
    .unwrap()
}

struct Twin {
    name: &'static str,
    law: &'static str,
    params: Vec<u32>,
    cell: fn(&mut u64) -> u32,
}

fn twins() -> Vec<Twin> {
    let mut evo_params = evo::DEFAULTS.to_vec();
    evo_params[evo::SEED] = 0x5EED;
    vec![
        Twin {
            name: "sand",
            law: SAND,
            params: Vec::new(),
            cell: sand_cell,
        },
        Twin {
            name: "heat",
            law: HEAT,
            params: heat::TUNED.to_vec(),
            cell: heat_cell,
        },
        Twin {
            name: "code",
            law: CODE,
            params: Vec::new(),
            cell: code_cell,
        },
        Twin {
            name: "nature",
            law: NATURE,
            params: vec![3],
            cell: nature_cell,
        },
        Twin {
            name: "evo",
            law: EVO,
            params: evo_params,
            cell: evo_cell,
        },
        Twin {
            name: "embed",
            law: EMBED,
            params: vec![embed::embedding([1, -1, 2, -2])],
            cell: embed_cell,
        },
    ]
}

#[test]
fn a_law_world_made_through_the_tools_equals_its_named_twin() {
    let Some(host) = device("law world twins") else {
        return;
    };
    let scratch = Scratch::new("twins");
    let mut server = served(&host, &scratch);
    for twin in twins() {
        let law = compiled(twin.law);
        let class = quiet::class(&law, &twin.params, &[]);
        let start = landscape(twin.cell, 0x7A1 ^ twin.name.len() as u64);
        for sparse in [false, true] {
            let by_law = call(
                &mut server,
                "grit_world_create",
                &making_law(twin.law, &twin.params, sparse),
            );
            if sparse && !class.proved() {
                let told = refusal(&by_law);
                assert_eq!(told.text_at("code"), "quiet_refused", "{}", twin.name);
                continue;
            }
            let by_law = answer(&by_law);
            assert_eq!(by_law.text_at("law"), law.hex(), "{}", twin.name);
            assert_eq!(by_law.text_at("quiet"), class.name(), "{}", twin.name);
            assert_eq!(by_law.text_at("rule"), twin.name);
            let by_rule = answer(&call(
                &mut server,
                "grit_world_create",
                &making_rule(twin.name, &twin.params, sparse),
            ));
            let (law_world, rule_world) = (by_law.int_at("world"), by_rule.int_at("world"));
            write_all(&mut server, law_world, &start);
            write_all(&mut server, rule_world, &start);
            let mut done = 0;
            while done < FRAMES {
                let law_step = stepped(&mut server, law_world, BATCH);
                stepped(&mut server, rule_world, BATCH);
                done += BATCH;
                if !class.proved() {
                    assert_eq!(
                        law_step.int_at("listed"),
                        i64::from(grid().octs()),
                        "{}: a whole world lists every oct",
                        twin.name
                    );
                }
                let cells = read_all(&mut server, rule_world);
                assert_eq!(
                    read_all(&mut server, law_world),
                    cells,
                    "{} sparse {sparse} after {done} frames",
                    twin.name
                );
            }
            let law_state = state(&mut server, law_world, "twin-law");
            let rule_state = state(&mut server, rule_world, "twin-rule");
            assert_eq!(law_state, rule_state, "{} sparse {sparse}", twin.name);
            let moved = read_all(&mut server, rule_world)
                .iter()
                .zip(&start)
                .filter(|(a, b)| a != b)
                .count();
            println!(
                "{} sparse {sparse} quiet {}: law world equals rule world over {FRAMES} frames on {}, {moved} cells moved",
                twin.name,
                class.name(),
                host.name()
            );
            assert!(moved > 0, "{}: nothing moved", twin.name);
            for world in [law_world, rule_world] {
                answer(&call(
                    &mut server,
                    "grit_world_close",
                    &format!("{{\"world\":{world}}}"),
                ));
            }
        }
    }
}

#[test]
fn the_class_of_each_shipped_twin_is_what_its_dag_reads() {
    for twin in twins() {
        let law = compiled(twin.law);
        let class = quiet::class(&law, &twin.params, &[]);
        let reads = quiet::reads(law.dag());
        println!("{}: {} {:?}", twin.name, class.name(), class.why());
        assert_eq!(
            class.proved(),
            !reads.any() && twin.name == "code",
            "{}",
            twin.name
        );
    }
}

#[test]
fn the_class_reads_the_dag_and_not_the_text() {
    let dead = compiled(DEAD);
    assert_eq!(quiet::reads(dead.dag()), quiet::Reads::default());
    assert_eq!(quiet::class(&dead, &[], &[]), Class::Proved);
    let blink = compiled(BLINK);
    assert_eq!(
        quiet::class(&blink, &[], &[]),
        Class::Whole(Why::Reads(quiet::Reads {
            frame: true,
            at: false
        }))
    );
    let heat = compiled(HEAT);
    let reads = quiet::reads(heat.dag());
    assert!(
        reads.frame && reads.at,
        "noise reads frame and at: {reads:?}"
    );
    let fill = compiled(FILL);
    assert_eq!(
        quiet::class(&fill, &[], &[]),
        Class::Whole(Why::Moves {
            value: vec![0],
            to: vec![1; 8]
        })
    );
    assert_eq!(quiet::class(&compiled(KEEP), &[], &[]), Class::Proved);
    let bias = compiled(BIAS);
    assert!(quiet::class(&bias, &[0, 7], &[vec![7], vec![3]]).proved());
    assert!(!quiet::class(&bias, &[5, 7], &[]).proved());
    assert!(quiet::class(&bias, &[0, 9], &[]).proved());
    assert!(!quiet::class(&bias, &[0, 9], &[vec![7]]).proved());
    let two = compiled(STILL_TWO);
    assert!(quiet::class(&two, &[], &[]).proved());
    assert!(quiet::class(&two, &[], &[vec![4, 9]]).proved());
    assert_eq!(
        quiet::class(&two, &[], &[vec![0, 9]]),
        Class::Whole(Why::Moves {
            value: vec![0, 9],
            to: [vec![0; 8], vec![5; 8]].concat()
        })
    );
}

fn engine_world<'host>(
    host: &'host Host,
    law: &Law,
    params: &[u32],
    sparse: bool,
    stepping: Stepping,
) -> World<'host> {
    World::create_law_stepped(
        host,
        law,
        &Made {
            rule: law.name(),
            params,
            size: [SIDE, SIDE, SIDE],
            sparse,
            slots: None,
        },
        stepping,
    )
    .unwrap()
}

fn engine_write(world: &mut World<'_>, cells: &[u32]) {
    let grid = grid();
    let mut placed = Vec::new();
    for z in 0..SIDE {
        for y in 0..SIDE {
            for x in 0..SIDE {
                let word = cells[grid.index(x, y, z)];
                if word != 0 {
                    placed.push(([x, y, z], word));
                }
            }
        }
    }
    world.write(&placed).unwrap();
}

#[test]
fn a_law_that_wakes_on_the_frame_is_not_listed() {
    let Some(host) = device("frame law") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    for text in [BLINK, PULSE] {
        let law = compiled(text);
        let start = landscape(small_cell, 0xB11 ^ text.len() as u64);
        let told = refusal(&call(
            &mut server,
            "grit_world_create",
            &making_law(text, &[], true),
        ));
        assert_eq!(told.text_at("code"), "quiet_refused", "{}", law.name());
        assert_eq!(told.text_at("field"), "$.sparse");
        assert!(
            told.text_at("message").contains("frame"),
            "{}",
            written(&told)
        );
        assert!(told.text_at("hint").contains("dense"), "{}", written(&told));
        let made = answer(&call(
            &mut server,
            "grit_world_create",
            &making_law(text, &[], false),
        ));
        assert_eq!(made.text_at("quiet"), "whole");
        assert!(
            made.text_at("quiet_why").contains("frame"),
            "{}",
            written(&made)
        );
        let world = made.int_at("world");
        write_all(&mut server, world, &start);
        let mut engine = engine_world(&host, &law, &[], false, Stepping::Listed);
        engine_write(&mut engine, &start);
        let mut want = start.clone();
        let mut listed_differs = None;
        let mut done = 0;
        while done < FRAMES {
            let step = stepped(&mut server, world, BATCH);
            assert_eq!(step.int_at("listed"), i64::from(grid().octs()));
            engine.step(BATCH).unwrap();
            want = replay(&law, &want, done, BATCH, &[]);
            done += BATCH;
            assert_eq!(
                read_all(&mut server, world),
                want,
                "{} after {done} frames",
                law.name()
            );
            if listed_differs.is_none() && engine.cells().unwrap() != want {
                listed_differs = Some(done);
            }
        }
        assert_ne!(want, start, "{}: nothing changed", law.name());
        println!(
            "{}: whole dense equals the CPU replay over {FRAMES} frames; stepped by lists it first differs by frame {listed_differs:?}",
            law.name()
        );
        if text == PULSE {
            assert!(listed_differs.is_some(), "the listed pulse world kept up");
        }
        answer(&call(
            &mut server,
            "grit_world_close",
            &format!("{{\"world\":{world}}}"),
        ));
    }
}

#[test]
fn a_law_that_moves_the_empty_oct_is_whole_and_would_lose_cells_sparse() {
    let Some(host) = device("empty oct law") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let fill = compiled(FILL);
    let told = refusal(&call(
        &mut server,
        "grit_world_create",
        &making_law(FILL, &[], true),
    ));
    assert_eq!(told.text_at("code"), "quiet_refused");
    assert!(
        told.text_at("message").contains("fixed point"),
        "{}",
        written(&told)
    );
    let made = answer(&call(
        &mut server,
        "grit_world_create",
        &making_law(FILL, &[], false),
    ));
    assert_eq!(made.text_at("quiet"), "whole");
    let keep = compiled(KEEP);
    let made_keep = answer(&call(
        &mut server,
        "grit_world_create",
        &making_law(KEEP, &[], true),
    ));
    assert_eq!(made_keep.text_at("quiet"), "proved");
    let size = [256, 256, 64];
    let grid = Grid3::new(size[0], size[1], size[2]).unwrap();
    let mut start = vec![0_u32; grid.cells()];
    let mut seed = 0xF111_u64;
    let mut placed = Vec::new();
    for _ in 0..400 {
        let roll = next(&mut seed);
        let at = [
            (roll % 256) as u32,
            ((roll >> 8) % 256) as u32,
            ((roll >> 16) % 64) as u32,
        ];
        let word = 2 + ((roll >> 32) % 50) as u32 * 2;
        start[grid.index(at[0], at[1], at[2])] = word;
        placed.push((at, word));
    }
    let frames = 4;
    let want_of = |law: &Law| {
        let mut want = start.clone();
        for frame in 0..frames {
            want = grit::sand3::reference(law, grid, frame, &want, &[]);
        }
        want
    };
    let sparse_world = |law: &Law, slots: Option<u32>| {
        World::create_law(
            &host,
            law,
            &Made {
                rule: law.name(),
                params: &[],
                size,
                sparse: true,
                slots,
            },
        )
        .unwrap()
    };

    let mut kept = sparse_world(&keep, None);
    let bricks = kept.pool().unwrap().count();
    let capacity = kept.pool_use().unwrap().unwrap().capacity;
    assert!(capacity < bricks, "{capacity} slots for {bricks} bricks");
    kept.write(&placed).unwrap();
    kept.step(frames).unwrap();
    assert_eq!(kept.cells().unwrap(), want_of(&keep));
    let used = kept.pool_use().unwrap().unwrap();
    assert!(
        !used.overflow && !used.lost && used.held < bricks,
        "{used:?}"
    );
    println!(
        "keep, proved, sparse on {size:?}: equals the CPU replay after {frames} frames holding {} of {bricks} bricks",
        used.held
    );

    let mut filled = sparse_world(&fill, None);
    filled.write(&placed).unwrap();
    filled.step(frames).unwrap();
    let used = filled.pool_use().unwrap().unwrap();
    assert!(used.overflow, "{used:?}");
    let why = filled
        .cells()
        .expect_err("an overflowed world still reads its cells");
    assert!(
        matches!(
            why.downcast_ref::<grit::host::Error>(),
            Some(grit::host::Error::PoolFull { .. })
        ),
        "{why}"
    );
    println!(
        "fill, whole, sparse on {size:?} with the default pool of {capacity} slots for {bricks} bricks: {used:?}; reading answered {why}"
    );

    let mut every = sparse_world(&fill, Some(bricks));
    every.write(&placed).unwrap();
    every.step(frames).unwrap();
    assert_eq!(every.cells().unwrap(), want_of(&fill));
    let used = every.pool_use().unwrap().unwrap();
    assert!(!used.overflow && !used.lost, "{used:?}");
    println!(
        "fill, whole, sparse with a slot for each of the {bricks} bricks: equals the CPU replay, {used:?}"
    );
}

#[test]
fn a_proved_law_steps_listed_as_whole_dense_and_sparse() {
    let Some(host) = device("proved law") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let drift = tumble("drift", false);
    for (text, params, cell) in [
        (drift.as_str(), vec![2], small_cell as fn(&mut u64) -> u32),
        (CODE, Vec::new(), code_cell),
    ] {
        let law = compiled(text);
        assert!(quiet::class(&law, &params, &[]).proved(), "{}", law.name());
        let start = landscape(cell, 0x9A0 ^ text.len() as u64);
        let mut worlds = Vec::new();
        for sparse in [false, true] {
            let made = answer(&call(
                &mut server,
                "grit_world_create",
                &making_law(text, &params, sparse),
            ));
            assert_eq!(made.text_at("quiet"), "proved", "{}", law.name());
            let world = made.int_at("world");
            write_all(&mut server, world, &start);
            worlds.push(world);
        }
        let mut whole = engine_world(&host, &law, &params, false, Stepping::Whole);
        engine_write(&mut whole, &start);
        let mut want = start.clone();
        let mut done = 0;
        let mut fewer = false;
        while done < FRAMES {
            for &world in &worlds {
                let step = stepped(&mut server, world, BATCH);
                fewer |= step.int_at("listed") < i64::from(grid().octs());
            }
            whole.step(BATCH).unwrap();
            want = replay(&law, &want, done, BATCH, &params);
            done += BATCH;
            assert_eq!(
                whole.cells().unwrap(),
                want,
                "{} whole after {done}",
                law.name()
            );
            for &world in &worlds {
                assert_eq!(
                    read_all(&mut server, world),
                    want,
                    "{} world {world} after {done}",
                    law.name()
                );
            }
        }
        assert!(
            fewer,
            "{}: the listed worlds never listed fewer octs than all",
            law.name()
        );
        assert_ne!(want, start, "{}: nothing moved", law.name());
        println!(
            "{}: listed dense and sparse equal whole and the CPU over {FRAMES} frames",
            law.name()
        );
        for world in worlds {
            answer(&call(
                &mut server,
                "grit_world_close",
                &format!("{{\"world\":{world}}}"),
            ));
        }
    }
}

#[test]
fn a_param_write_that_breaks_a_fixed_point_on_a_sparse_world_is_refused_and_changes_nothing() {
    let Some(host) = device("param write") else {
        return;
    };
    let scratch = Scratch::new("params");
    let mut server = served(&host, &scratch);
    let bias = compiled(BIAS);
    let start: Vec<u32> = landscape(small_cell, 0xB1A5)
        .into_iter()
        .map(|word| if word == 4 { 7 } else { word })
        .collect();
    let made = answer(&call(
        &mut server,
        "grit_world_create",
        &making_law(BIAS, &[0, 7], true),
    ));
    assert_eq!(made.text_at("quiet"), "proved");
    let world = made.int_at("world");
    write_all(&mut server, world, &start);
    stepped(&mut server, world, 9);
    let cells = read_all(&mut server, world);
    let before = state(&mut server, world, "before");
    let told = refusal(&call(
        &mut server,
        "grit_world_params",
        &format!("{{\"world\":{world},\"params\":[5,7]}}"),
    ));
    assert_eq!(told.text_at("code"), "quiet_refused");
    assert_eq!(told.text_at("field"), "$.params");
    assert!(
        told.text_at("message").contains("[0]"),
        "{}",
        written(&told)
    );
    assert_eq!(read_all(&mut server, world), cells);
    assert_eq!(state(&mut server, world, "after"), before);
    let listed = answer(&call(&mut server, "grit_world_list", "{}"));
    let item = listed
        .get("items")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .find(|one| one.get("world").and_then(Json::int) == Some(world))
        .cloned()
        .unwrap();
    assert_eq!(item.text_at("quiet"), "proved");
    assert_eq!(item.text_at("law"), bias.hex());
    let moved = answer(&call(
        &mut server,
        "grit_world_params",
        &format!("{{\"world\":{world},\"params\":[0,9]}}"),
    ));
    assert_eq!(moved.text_at("quiet"), "proved");
    stepped(&mut server, world, 2);
    let want = replay(&bias, &cells, 9, 2, &[0, 9]);
    assert!(want.contains(&9) && !want.contains(&7));
    assert_eq!(read_all(&mut server, world), want);

    let filled = answer(&call(
        &mut server,
        "grit_world_create",
        &making_law(BIAS, &[0, 7], true),
    ));
    let full = filled.int_at("world");
    write_all(&mut server, full, &vec![7_u32; grid().cells()]);
    let settled = stepped(&mut server, full, 8);
    let used = settled.get("pool_use").unwrap();
    assert_eq!(used.int_at("held"), 0, "{}", written(&settled));
    let before = state(&mut server, full, "full-before");
    let told = refusal(&call(
        &mut server,
        "grit_world_params",
        &format!("{{\"world\":{full},\"params\":[0,9]}}"),
    ));
    assert_eq!(told.text_at("code"), "quiet_refused");
    assert!(
        told.text_at("message").contains("[7]"),
        "{}",
        written(&told)
    );
    assert_eq!(state(&mut server, full, "full-after"), before);
    assert_eq!(read_all(&mut server, full), vec![7_u32; grid().cells()]);

    let held = answer(&call(
        &mut server,
        "grit_world_create",
        &making_law(BIAS, &[0, 9], true),
    ))
    .int_at("world");
    let sevens = vec![7_u32; grid().cells()];
    write_all(&mut server, held, &sevens);
    let saved = state(&mut server, held, "held-sevens");
    let opened = answer(&call(
        &mut server,
        "grit_world_open",
        "{\"name\":\"held-sevens\"}",
    ));
    assert_eq!(opened.text_at("quiet"), "proved", "{}", written(&opened));
    let reopened = opened.int_at("world");
    assert_eq!(state(&mut server, reopened, "held-sevens-again"), saved);
    for world in [held, reopened] {
        stepped(&mut server, world, 2);
        assert_eq!(
            read_all(&mut server, world),
            replay(&bias, &sevens, 0, 2, &[0, 9]),
            "world {world}"
        );
    }
}

#[test]
fn a_param_write_that_makes_a_dense_law_whole_steps_it_whole() {
    let Some(host) = device("dense param write") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let bias = compiled(BIAS);
    let start = landscape(small_cell, 0xDE5);
    let made = answer(&call(
        &mut server,
        "grit_world_create",
        &making_law(BIAS, &[0, 7], false),
    ));
    let world = made.int_at("world");
    write_all(&mut server, world, &start);
    let step = stepped(&mut server, world, 4);
    assert!(step.int_at("listed") < i64::from(grid().octs()));
    let mut want = replay(&bias, &start, 0, 4, &[0, 7]);
    assert_eq!(read_all(&mut server, world), want);
    let told = answer(&call(
        &mut server,
        "grit_world_params",
        &format!("{{\"world\":{world},\"params\":[5,7]}}"),
    ));
    assert_eq!(told.text_at("quiet"), "whole");
    assert_eq!(told.int_at("frame"), 4);
    for round in 0..4 {
        let step = stepped(&mut server, world, 8);
        assert_eq!(
            step.int_at("listed"),
            i64::from(grid().octs()),
            "round {round}"
        );
        want = replay(&bias, &want, 4 + round * 8, 8, &[5, 7]);
        assert_eq!(read_all(&mut server, world), want, "round {round}");
    }
    let back = answer(&call(
        &mut server,
        "grit_world_params",
        &format!("{{\"world\":{world},\"params\":[0,7]}}"),
    ));
    assert_eq!(back.text_at("quiet"), "proved");
    stepped(&mut server, world, 6);
    want = replay(&bias, &want, 36, 6, &[0, 7]);
    assert_eq!(read_all(&mut server, world), want);
}

#[test]
fn a_malformed_law_names_its_code_and_path_and_one_of_rule_or_law_is_given() {
    let mut server = Server::new(None);
    for (text, code, path) in [
        (BLINK.replace("\"law\":1", "\"law\":2"), "version", "$.law"),
        (r#"{"law":1,"#.to_owned(), "json", "$"),
        (BLINK.replace("\"sel\"", "\"pick\""), "unknown_op", ""),
    ] {
        let told = refusal(&call(
            &mut server,
            "grit_world_create",
            &format!(
                "{{\"wide\":8,\"deep\":8,\"tall\":8,\"law\":{}}}",
                quoted(&text)
            ),
        ));
        assert_eq!(told.text_at("code"), "law_refused", "{}", written(&told));
        assert_eq!(told.text_at("field"), "$.law");
        let inner = told.get("law").unwrap();
        assert_eq!(inner.text_at("code"), code, "{}", written(&told));
        if path.is_empty() {
            assert!(
                inner.text_at("path").starts_with("$.step"),
                "{}",
                written(&told)
            );
        } else {
            assert_eq!(inner.text_at("path"), path, "{}", written(&told));
        }
        assert!(told.text_at("hint").contains("path"));
    }
    for arguments in [
        format!(
            "{{\"wide\":8,\"deep\":8,\"tall\":8,\"rule\":\"sand\",\"law\":{}}}",
            quoted(BLINK)
        ),
        "{\"wide\":8,\"deep\":8,\"tall\":8}".to_owned(),
    ] {
        let told = refusal(&call(&mut server, "grit_world_create", &arguments));
        assert_eq!(told.text_at("code"), "rule_or_law", "{}", written(&told));
        assert!(told.text_at("hint").contains("exactly one"));
    }
    let told = refusal(&call(
        &mut server,
        "grit_world_create",
        &format!(
            "{{\"wide\":8,\"deep\":8,\"tall\":8,\"law\":{},\"params\":[1]}}",
            quoted(BLINK)
        ),
    ));
    assert_eq!(told.text_at("code"), "params");
    assert!(
        told.text_at("message").contains("law blink reads 0"),
        "{}",
        written(&told)
    );
    let described = answer(&call(&mut server, "grit_describe", "{}"));
    let codes: Vec<String> = described
        .get("codes")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .map(|one| one.text_at("code"))
        .collect();
    for code in ["law_refused", "quiet_refused", "rule_or_law"] {
        assert!(codes.iter().any(|one| one == code), "{code}");
    }
    let tools = described.get("tools").and_then(Json::list).unwrap();
    assert!(tools.contains(&Json::Text("grit_world_params".to_owned())));
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

    fn kill(mut self) {
        self.child.kill().unwrap();
        self.child.wait().unwrap();
    }
}

trait Asking {
    fn asked(&mut self, name: &str, arguments: &str) -> Json;
}

impl Asking for Process {
    fn asked(&mut self, name: &str, arguments: &str) -> Json {
        self.ask(name, arguments)
    }
}

fn read_by(asking: &mut impl Asking, world: i64) -> Vec<u32> {
    let mut cells = Vec::new();
    for z in (0..SIDE).step_by(4) {
        cells.extend(numbers(&answer(
            &asking.asked("grit_world_read", &slab(world, z)),
        )));
    }
    cells
}

fn write_by(asking: &mut impl Asking, world: i64, cells: &[u32]) {
    for chunk in writes(cells) {
        answer(&asking.asked(
            "grit_world_write",
            &format!("{{\"world\":{world},\"cells\":{chunk},\"budget_ms\":600000}}"),
        ));
    }
}

fn step_by(asking: &mut impl Asking, world: i64, frames: u32) -> Json {
    answer(&asking.asked(
        "grit_world_step",
        &format!("{{\"world\":{world},\"frames\":{frames},\"budget_ms\":600000}}"),
    ))
}

struct Model {
    law: Law,
    params: Vec<u32>,
    cells: Vec<u32>,
    frame: u32,
}

impl Model {
    fn new(text: &str, params: &[u32], cells: &[u32]) -> Self {
        Self {
            law: compiled(text),
            params: params.to_vec(),
            cells: cells.to_vec(),
            frame: 0,
        }
    }

    fn step(&mut self, frames: u32) {
        self.cells = replay(&self.law, &self.cells, self.frame, frames, &self.params);
        self.frame += frames;
    }
}

fn params_by(asking: &mut impl Asking, world: i64, model: &mut Model, params: &[u32]) -> Json {
    model.params = params.to_vec();
    answer(&asking.asked(
        "grit_world_params",
        &format!("{{\"world\":{world},\"params\":{params:?}}}"),
    ))
}

#[test]
fn a_law_world_survives_save_open_fork_restore_and_a_restart() {
    let Some(host) = device("law world restart") else {
        return;
    };
    drop(host);
    let scratch = Scratch::new("restart");
    let drift = tumble("drift", false);
    let gated = tumble("gated", true);
    let start = landscape(small_cell, 0x5A7E);
    let mut first = Process::start(&scratch.dir());
    let mut worlds: Vec<(i64, Model)> = Vec::new();
    for (text, sparse, quiet) in [
        (drift.as_str(), false, "proved"),
        (drift.as_str(), true, "proved"),
        (gated.as_str(), false, "whole"),
    ] {
        let made = answer(&first.ask("grit_world_create", &making_law(text, &[2], sparse)));
        assert_eq!(made.text_at("quiet"), quiet);
        assert_eq!(made.text_at("law"), compiled(text).hex());
        let world = made.int_at("world");
        write_by(&mut first, world, &start);
        let mut model = Model::new(text, &[2], &start);
        step_by(&mut first, world, 40);
        model.step(40);
        let told = params_by(&mut first, world, &mut model, &[3]);
        assert_eq!(told.text_at("quiet"), quiet);
        step_by(&mut first, world, 30);
        model.step(30);
        assert_eq!(read_by(&mut first, world), model.cells, "world {world}");
        worlds.push((world, model));
    }
    let source = worlds[0].0;
    let forked = answer(&first.ask("grit_world_fork", &format!("{{\"world\":{source}}}")));
    assert_eq!(forked.text_at("law"), worlds[0].1.law.hex());
    assert_eq!(forked.text_at("quiet"), "proved");
    let fork = forked.int_at("world");
    let mut fork_model = Model {
        law: compiled(&drift),
        params: worlds[0].1.params.clone(),
        cells: worlds[0].1.cells.clone(),
        frame: worlds[0].1.frame,
    };
    step_by(&mut first, fork, 11);
    fork_model.step(11);
    params_by(&mut first, fork, &mut fork_model, &[2]);
    step_by(&mut first, fork, 5);
    fork_model.step(5);
    step_by(&mut first, source, 3);
    worlds[0].1.step(3);
    let restored = answer(&first.ask(
        "grit_world_restore",
        &format!("{{\"world\":{source},\"from\":{fork}}}"),
    ));
    assert_eq!(restored.text_at("quiet"), "proved");
    worlds[0].1.cells.clone_from(&fork_model.cells);
    worlds[0].1.frame = fork_model.frame;
    worlds[0].1.params.clone_from(&fork_model.params);
    step_by(&mut first, source, 4);
    worlds[0].1.step(4);
    assert_eq!(read_by(&mut first, source), worlds[0].1.cells);
    let other = answer(&first.ask(
        "grit_world_create",
        &making_law(&tumble("drift", true), &[2], false),
    ))
    .int_at("world");
    for from in [worlds[2].0, other] {
        let told = refusal(&first.ask(
            "grit_world_restore",
            &format!("{{\"world\":{source},\"from\":{from}}}"),
        ));
        assert_eq!(told.text_at("code"), "unlike", "{}", written(&told));
    }
    answer(&first.ask("grit_world_close", &format!("{{\"world\":{other}}}")));
    let mut saved = Vec::new();
    for (index, (world, model)) in worlds.iter().enumerate() {
        let name = format!("law-{index}");
        let got = answer(&first.ask(
            "grit_world_save",
            &format!("{{\"world\":{world},\"name\":\"{name}\"}}"),
        ));
        saved.push((
            name,
            got.text_at("state"),
            model.cells.clone(),
            model.frame,
            model.params.clone(),
        ));
    }
    for (world, model) in &mut worlds {
        step_by(&mut first, *world, 7);
        model.step(7);
    }
    worlds.push((fork, fork_model));
    let listed = answer(&first.ask("grit_world_list", "{}"));
    for (world, model) in &worlds {
        let item = listed
            .get("items")
            .and_then(Json::list)
            .unwrap()
            .iter()
            .find(|one| one.get("world").and_then(Json::int) == Some(*world))
            .cloned()
            .unwrap();
        assert_eq!(item.text_at("law"), model.law.hex(), "{}", written(&item));
        assert!(item.get("quiet").is_some(), "{}", written(&item));
    }
    let described = answer(&first.ask("grit_describe", "{}"));
    let laws = described
        .get("worlds")
        .and_then(|one| one.get("laws"))
        .and_then(Json::list)
        .unwrap();
    assert_eq!(laws.len(), worlds.len(), "{}", written(&described));
    first.kill();

    let mut second = Process::start(&scratch.dir());
    let listed = answer(&second.ask("grit_world_list", "{}"));
    for (world, model) in &worlds {
        let item = listed
            .get("items")
            .and_then(Json::list)
            .unwrap()
            .iter()
            .find(|one| one.get("world").and_then(Json::int) == Some(*world))
            .cloned()
            .unwrap();
        assert_eq!(item.text_at("kind"), "kept");
        assert_eq!(item.text_at("law"), model.law.hex());
        assert_eq!(item.int_at("frame"), i64::from(model.frame));
    }
    for (world, model) in &mut worlds {
        let opened = answer(&second.ask(
            "grit_world_open",
            &format!("{{\"name\":\"world-{world}\"}}"),
        ));
        assert_eq!(opened.text_at("law"), model.law.hex());
        assert_eq!(opened.int_at("frame"), i64::from(model.frame));
        assert_eq!(
            read_by(&mut second, *world),
            model.cells,
            "kept world {world}"
        );
        step_by(&mut second, *world, 9);
        model.step(9);
        assert_eq!(
            read_by(&mut second, *world),
            model.cells,
            "kept world {world} stepped"
        );
    }
    for (index, (name, sum, cells, frame, params)) in saved.into_iter().enumerate() {
        let opened = answer(&second.ask("grit_world_open", &format!("{{\"name\":\"{name}\"}}")));
        let world = opened.int_at("world");
        let text = if index == 2 {
            gated.as_str()
        } else {
            drift.as_str()
        };
        assert_eq!(opened.text_at("law"), compiled(text).hex());
        assert_eq!(opened.int_at("frame"), i64::from(frame));
        assert_eq!(read_by(&mut second, world), cells, "saved {name}");
        let again = answer(&second.ask(
            "grit_world_save",
            &format!("{{\"world\":{world},\"name\":\"{name}-again\"}}"),
        ));
        assert_eq!(again.text_at("state"), sum, "saved {name}");
        let mut model = Model {
            law: compiled(text),
            params,
            cells,
            frame,
        };
        step_by(&mut second, world, 13);
        model.step(13);
        assert_eq!(
            read_by(&mut second, world),
            model.cells,
            "saved {name} stepped"
        );
    }
    second.kill();
}

const ONES: &str = r#"{"law":1,"name":"ones","params":0,"channels":1,"halo":0,"tally":{"counts":1,"weighted":false},"consts":{},"defs":{},
"step":["vec","cells",["vec",1]]}"#;

const ONES_FRAMED: &str = r#"{"law":1,"name":"onesf","params":0,"channels":1,"halo":0,"tally":{"counts":1,"weighted":false},"consts":{},"defs":{},
"step":["vec",["map","w","cells",["add","w",["and","frame",0]]],["vec",1]]}"#;

fn held_bytes(server: &mut Server<'_>) -> i64 {
    answer(&call(server, "grit_describe", "{}"))
        .get("memory")
        .unwrap()
        .int_at("held_bytes")
}

#[test]
fn a_class_changing_param_write_holds_no_second_copy_of_the_world() {
    let Some(host) = device("param write memory") else {
        return;
    };
    let mut probe = Server::new(Some(&host));
    let made = answer(&call(
        &mut probe,
        "grit_world_create",
        &making_law(BIAS, &[0, 7], false),
    ));
    let bytes = held_bytes(&mut probe);
    drop(made);
    drop(probe);
    let room = u64::try_from(bytes + bytes / 2).unwrap();
    let mut server = Server::with(
        Some(&host),
        &Options {
            dir: None,
            memory: Some(room),
        },
    )
    .unwrap();
    let bias = compiled(BIAS);
    let start = landscape(small_cell, 0x3E31);
    let world = answer(&call(
        &mut server,
        "grit_world_create",
        &making_law(BIAS, &[0, 7], false),
    ))
    .int_at("world");
    write_all(&mut server, world, &start);
    stepped(&mut server, world, 3);
    let told = refusal(&call(
        &mut server,
        "grit_world_fork",
        &format!("{{\"world\":{world}}}"),
    ));
    assert_eq!(told.text_at("code"), "memory", "{}", written(&told));
    let held = held_bytes(&mut server);
    assert_eq!(held, bytes);
    let changed = answer(&call(
        &mut server,
        "grit_world_params",
        &format!("{{\"world\":{world},\"params\":[5,7]}}"),
    ));
    assert_eq!(changed.text_at("quiet"), "whole");
    assert_eq!(held_bytes(&mut server), held);
    let mut want = replay(&bias, &start, 0, 3, &[0, 7]);
    let step = stepped(&mut server, world, 5);
    assert_eq!(step.int_at("listed"), i64::from(grid().octs()));
    want = replay(&bias, &want, 3, 5, &[5, 7]);
    assert_eq!(read_all(&mut server, world), want);
    assert_eq!(held_bytes(&mut server), held);
    println!(
        "a world of {bytes} bytes in a server of {room}: the fork is refused, the params write that makes it whole holds {} bytes",
        held_bytes(&mut server)
    );
}

#[test]
fn a_restore_takes_the_class_and_stepping_of_a_fork_whose_params_changed_it() {
    let Some(host) = device("restore across classes") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let bias = compiled(BIAS);
    let start = landscape(small_cell, 0x5E57);
    let first = answer(&call(
        &mut server,
        "grit_world_create",
        &making_law(BIAS, &[0, 7], false),
    ))
    .int_at("world");
    write_all(&mut server, first, &start);
    stepped(&mut server, first, 4);
    let mut want = replay(&bias, &start, 0, 4, &[0, 7]);
    let fork = answer(&call(
        &mut server,
        "grit_world_fork",
        &format!("{{\"world\":{first}}}"),
    ))
    .int_at("world");
    let changed = answer(&call(
        &mut server,
        "grit_world_params",
        &format!("{{\"world\":{fork},\"params\":[5,7]}}"),
    ));
    assert_eq!(changed.text_at("quiet"), "whole");
    stepped(&mut server, fork, 3);
    want = replay(&bias, &want, 4, 3, &[5, 7]);
    let restored = answer(&call(
        &mut server,
        "grit_world_restore",
        &format!("{{\"world\":{first},\"from\":{fork}}}"),
    ));
    assert_eq!(restored.text_at("quiet"), "whole", "{}", written(&restored));
    assert_eq!(restored.int_at("frame"), 7);
    assert_eq!(read_all(&mut server, first), want);
    for _ in 0..3 {
        let step = stepped(&mut server, first, 4);
        assert_eq!(step.int_at("listed"), i64::from(grid().octs()));
    }
    want = replay(&bias, &want, 7, 12, &[5, 7]);
    assert_eq!(read_all(&mut server, first), want);
    let back = answer(&call(
        &mut server,
        "grit_world_fork",
        &format!("{{\"world\":{first}}}"),
    ))
    .int_at("world");
    answer(&call(
        &mut server,
        "grit_world_params",
        &format!("{{\"world\":{back},\"params\":[0,7]}}"),
    ));
    let restored = answer(&call(
        &mut server,
        "grit_world_restore",
        &format!("{{\"world\":{first},\"from\":{back}}}"),
    ));
    assert_eq!(
        restored.text_at("quiet"),
        "proved",
        "{}",
        written(&restored)
    );
    let mut listed = Vec::new();
    for _ in 0..4 {
        listed.push(stepped(&mut server, first, 2).int_at("listed"));
    }
    want = replay(&bias, &want, 19, 8, &[0, 7]);
    assert_eq!(read_all(&mut server, first), want);
    assert!(
        listed.last().copied().unwrap() < i64::from(grid().octs()),
        "{listed:?}"
    );
}

#[test]
fn a_law_world_names_what_its_tally_counts() {
    let Some(host) = device("law tally") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let described = answer(&call(&mut server, "grit_describe", "{}"));
    let text = described.get("limits").unwrap().text_at("law_tally");
    assert!(
        text.contains("listed octs") && text.contains("every oct"),
        "{text}"
    );
    let octs = i64::from(grid().octs());
    for (law, tallied, quiet) in [
        (ONES, "listed octs", "proved"),
        (ONES_FRAMED, "every oct", "whole"),
    ] {
        let made = answer(&call(
            &mut server,
            "grit_world_create",
            &making_law(law, &[], false),
        ));
        assert_eq!(made.text_at("quiet"), quiet);
        assert_eq!(made.text_at("tallied"), tallied);
        let world = made.int_at("world");
        answer(&call(
            &mut server,
            "grit_world_write",
            &format!("{{\"world\":{world},\"cells\":[[3,3,3,7]]}}"),
        ));
        let mut tallies = Vec::new();
        for _ in 0..6 {
            let step = stepped(&mut server, world, 1);
            assert_eq!(step.text_at("tallied"), tallied);
            tallies.push(
                step.get("tally")
                    .and_then(Json::list)
                    .unwrap()
                    .first()
                    .and_then(Json::int)
                    .unwrap(),
            );
        }
        println!(
            "{} ({tallied}): tallies {tallies:?} of {octs} octs a phase",
            compiled(law).name()
        );
        if quiet == "proved" {
            assert_eq!(tallies.first().copied(), Some(octs));
            assert_eq!(tallies.last().copied(), Some(0));
        } else {
            assert!(tallies.iter().all(|&one| one == octs), "{tallies:?}");
        }
    }
}

#[cfg(feature = "faults")]
mod faulted {
    use super::*;

    use std::collections::BTreeMap;
    use std::sync::mpsc::Receiver;
    use std::time::Duration;

    use grit::serve3::exec::Exec;

    struct Run {
        exec: Exec,
        answers: Receiver<Json>,
        id: i64,
        early: BTreeMap<i64, Json>,
    }

    impl Run {
        fn new(options: &Options) -> Self {
            let pattern = std::env::var("GRIT_DEVICE")
                .ok()
                .filter(|one| !one.is_empty());
            let (exec, answers) = Exec::spawn(
                Box::new(move || Host::matching(pattern.as_deref())),
                options,
            )
            .unwrap();
            Self {
                exec,
                answers,
                id: 100,
                early: BTreeMap::new(),
            }
        }

        fn ask(&mut self, name: &str, arguments: &str) -> Json {
            self.id += 1;
            let id = self.id;
            self.exec.line(&line(id, name, arguments));
            if let Some(got) = self.early.remove(&id) {
                return got;
            }
            loop {
                let got = self.answers.recv_timeout(Duration::from_secs(300)).unwrap();
                let came = got.get("id").and_then(Json::int).unwrap();
                if came == id {
                    return got;
                }
                self.early.insert(came, got);
            }
        }
    }

    impl Asking for Run {
        fn asked(&mut self, name: &str, arguments: &str) -> Json {
            self.ask(name, arguments)
        }
    }

    #[test]
    fn after_a_lost_device_a_law_world_is_back_with_its_law_and_cells() {
        let Some(host) = device("law world lost device") else {
            return;
        };
        drop(host);
        let scratch = Scratch::new("lost");
        let mut run = Run::new(&Options {
            dir: Some(scratch.dir()),
            memory: None,
        });
        let drift = tumble("drift", false);
        let gated = tumble("gated", true);
        let start = landscape(small_cell, 0x1057);
        let mut worlds: Vec<(i64, Model)> = Vec::new();
        for (text, sparse) in [
            (drift.as_str(), true),
            (gated.as_str(), false),
            (drift.as_str(), false),
        ] {
            let made = answer(&run.ask("grit_world_create", &making_law(text, &[2], sparse)));
            let world = made.int_at("world");
            write_by(&mut run, world, &start);
            let mut model = Model::new(text, &[2], &start);
            step_by(&mut run, world, 20);
            model.step(20);
            params_by(&mut run, world, &mut model, &[3]);
            step_by(&mut run, world, 10);
            model.step(10);
            worlds.push((world, model));
        }
        let first = worlds[0].0;
        let mut seed = 0x5_u64;
        let mut writes = 0;
        let kept = |prefix: &str| {
            std::fs::read_dir(scratch.dir().join("kept"))
                .unwrap()
                .flatten()
                .any(|one| one.file_name().to_string_lossy().starts_with(prefix))
        };
        while !kept("snap-") && !kept("pending-") {
            assert!(writes < 64, "no snapshot after {writes} writes");
            let cells: Vec<u32> = worlds[0]
                .1
                .cells
                .iter()
                .map(|&word| {
                    if word == 0 && next(&mut seed).is_multiple_of(9) {
                        1
                    } else {
                        word
                    }
                })
                .collect();
            write_by(&mut run, first, &cells);
            worlds[0].1.cells = cells;
            writes += 1;
        }
        let landing = std::time::Instant::now();
        while kept("pending-") || !kept("snap-") {
            assert!(
                landing.elapsed() < Duration::from_secs(120),
                "the snapshot begun after {writes} writes never landed"
            );
            answer(&run.ask("grit_world_rules", "{}"));
            std::thread::sleep(Duration::from_millis(20));
        }
        step_by(&mut run, first, 6);
        worlds[0].1.step(6);
        params_by(&mut run, first, &mut worlds[0].1, &[2]);
        step_by(&mut run, first, 5);
        worlds[0].1.step(5);
        {
            let faults = run.exec.faults();
            faults.lock().unwrap().wedge_after_batch = Some(1);
        }
        let lost = run.ask(
            "grit_world_step",
            &format!("{{\"world\":{first},\"frames\":200,\"budget_ms\":600000}}"),
        );
        let told = refusal(&lost);
        assert_eq!(told.text_at("code"), "device_lost", "{}", written(&told));
        let message = told.text_at("message");
        let ran: u32 = message
            .split(" of the 200 frames ran")
            .next()
            .and_then(|head| head.rsplit(' ').next())
            .and_then(|count| count.parse().ok())
            .unwrap_or_else(|| panic!("{message}"));
        worlds[0].1.step(ran);
        let restored = told.get("restored").and_then(Json::list).unwrap();
        assert_eq!(restored.len(), worlds.len(), "{}", written(&told));
        for note in restored {
            assert!(note.get("failed").is_none(), "{}", written(note));
        }
        for (world, model) in &mut worlds {
            assert_eq!(
                read_by(&mut run, *world),
                model.cells,
                "world {world} after the lost device"
            );
            step_by(&mut run, *world, 12);
            model.step(12);
            assert_eq!(
                read_by(&mut run, *world),
                model.cells,
                "world {world} stepped after"
            );
        }
        let listed = answer(&run.ask("grit_world_list", "{}"));
        for (world, model) in &worlds {
            let item = listed
                .get("items")
                .and_then(Json::list)
                .unwrap()
                .iter()
                .find(|one| one.get("world").and_then(Json::int) == Some(*world))
                .cloned()
                .unwrap();
            assert_eq!(item.text_at("law"), model.law.hex());
        }
        run.exec.finish();
    }

    #[test]
    fn without_a_directory_a_law_world_is_rebuilt_from_its_snapshot_in_memory() {
        let Some(host) = device("law world snapshot in memory") else {
            return;
        };
        drop(host);
        let mut run = Run::new(&Options {
            dir: None,
            memory: None,
        });
        let drift = tumble("drift", false);
        let start = landscape(small_cell, 0x3E3);
        let made = answer(&run.ask("grit_world_create", &making_law(&drift, &[2], false)));
        let world = made.int_at("world");
        let mut model = Model::new(&drift, &[2], &start);
        write_by(&mut run, world, &start);
        step_by(&mut run, world, 10);
        model.step(10);
        params_by(&mut run, world, &mut model, &[3]);
        let mut seed = 0x3E4_u64;
        for _ in 0..12 {
            let cells: Vec<u32> = model
                .cells
                .iter()
                .map(|&word| {
                    if next(&mut seed).is_multiple_of(5) {
                        small_cell(&mut seed)
                    } else {
                        word
                    }
                })
                .collect();
            write_by(&mut run, world, &cells);
            model.cells = cells;
            step_by(&mut run, world, 1);
            model.step(1);
        }
        step_by(&mut run, world, 3);
        model.step(3);
        {
            let faults = run.exec.faults();
            faults.lock().unwrap().wedge_before_call = true;
        }
        let told = refusal(&run.ask("grit_world_step", &format!("{{\"world\":{world}}}")));
        assert_eq!(told.text_at("code"), "device_lost", "{}", written(&told));
        let got = answer(&run.ask("grit_world_read", &slab(world, 0)));
        let note = got.get("restored").unwrap();
        assert!(note.get("failed").is_none(), "{}", written(note));
        assert!(
            note.int_at("from_frame") > 0,
            "no snapshot in memory: {}",
            written(note)
        );
        assert_eq!(read_by(&mut run, world), model.cells);
        step_by(&mut run, world, 15);
        model.step(15);
        assert_eq!(read_by(&mut run, world), model.cells);
        run.exec.finish();
    }
}
