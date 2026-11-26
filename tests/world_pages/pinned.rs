#![allow(dead_code)]

use grit::serve3::Server;
use grit::wire::{Json, written};

use super::served::{Rng, bytes_hash, cells_text, int_at, ok, quoted, text_at};

pub const MAJORITY: &str = include_str!("../world_write/majority.law.json");
pub const MIX: &str = include_str!("../law/mix.law.json");
pub const CONTAINER: &str = "{\"law\":1,\"name\":\"container\",\"params\":0,\"channels\":2,\"halo\":0,\"tally\":{\"counts\":1,\"weighted\":false},\"consts\":{},\"defs\":{},\"step\":[\"vec\",[\"map\",\"c\",[\"range\",8],[\"vec\",0,0]],[\"vec\",0]]}";
const LONG: u32 = 600_000;

pub const NEW_CODES: [&str; 4] = ["page_limit", "page_format", "page_length", "no_law"];
pub const NEW_LIMITS: [&str; 2] = ["page", "data_world"];

const STEADY: [&str; 9] = [
    "world", "written", "frame", "frames", "tally", "cells", "count", "bits", "rule",
];

fn steady(answer: &Json) -> Json {
    match answer {
        Json::Map(pairs) => Json::Map(
            pairs
                .iter()
                .filter(|(key, _)| STEADY.contains(&key.as_str()))
                .cloned()
                .collect(),
        ),
        other => other.clone(),
    }
}

fn joined(answers: &[Json]) -> String {
    let shown: Vec<String> = answers.iter().map(written).collect();
    bytes_hash(shown.join("\n").as_bytes())
}

pub fn soup(rng: &mut Rng, size: [u32; 3], permille: u64) -> Vec<u32> {
    let cells = (size[0] * size[1] * size[2]) as usize;
    (0..cells)
        .map(|_| u32::from(rng.below(1000) < permille))
        .collect()
}

pub fn life(size: [u32; 3], cells: &[u32], wrapped: bool) -> Vec<u32> {
    let [wide, deep, tall] = size.map(|side| side as i64);
    let at = |x: i64, y: i64, z: i64| -> u32 {
        if wrapped {
            let x = x.rem_euclid(wide);
            let y = y.rem_euclid(deep);
            cells[((z * deep + y) * wide + x) as usize]
        } else if x < 0 || y < 0 || x >= wide || y >= deep {
            0
        } else {
            cells[((z * deep + y) * wide + x) as usize]
        }
    };
    let mut out = Vec::with_capacity(cells.len());
    for z in 0..tall {
        for y in 0..deep {
            for x in 0..wide {
                let mut n = 0;
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        if dx != 0 || dy != 0 {
                            n += at(x + dx, y + dy, z);
                        }
                    }
                }
                let alive = at(x, y, z) == 1;
                out.push(u32::from(n == 3 || (alive && n == 2)));
            }
        }
    }
    out
}

pub fn set_cells(size: [u32; 3], words: &[u32], z_at: u32) -> Vec<([u32; 3], u32)> {
    words
        .iter()
        .enumerate()
        .filter(|(_, word)| **word != 0)
        .map(|(at, word)| {
            let at = at as u32;
            (
                [
                    at % size[0],
                    at / size[0] % size[1],
                    at / (size[0] * size[1]) + z_at,
                ],
                *word,
            )
        })
        .collect()
}

pub fn write_cells(
    server: &mut Server<'_>,
    world: i64,
    plane: u32,
    cells: &[([u32; 3], u32)],
) -> Vec<Json> {
    cells
        .chunks(4096)
        .map(|chunk| {
            ok(
                server,
                "grit_world_write",
                &format!(
                    "{{\"world\":{world},\"plane\":{plane},\"cells\":{},\"budget_ms\":{LONG}}}",
                    cells_text(chunk)
                ),
            )
        })
        .collect()
}

pub fn dense_session(server: &mut Server<'_>) -> String {
    let mut answers = Vec::new();
    let made = ok(
        server,
        "grit_world_create",
        &format!(
            "{{\"wide\":64,\"deep\":64,\"tall\":2,\"law\":{},\"params\":[],\"budget_ms\":{LONG}}}",
            quoted(MAJORITY)
        ),
    );
    let world = int_at(&made, "world");
    answers.push(made);
    let mut rng = Rng(0x0D_E5E);
    let start = soup(&mut rng, [64, 64, 1], 450);
    answers.extend(write_cells(
        server,
        world,
        0,
        &set_cells([64, 64, 1], &start, 0),
    ));
    answers.push(ok(
        server,
        "grit_world_step",
        &format!("{{\"world\":{world},\"frames\":3,\"budget_ms\":{LONG}}}"),
    ));
    for args in [
        format!("{{\"world\":{world},\"from\":[3,5,0],\"to\":[60,40,0]}}"),
        format!("{{\"world\":{world},\"plane\":1,\"from\":[0,0,0],\"to\":[63,31,1]}}"),
        format!("{{\"world\":{world},\"form\":\"set\",\"from\":[0,0,0],\"to\":[63,63,1]}}"),
        format!(
            "{{\"world\":{world},\"form\":\"tally\",\"from\":[1,1,0],\"to\":[62,62,1],\"bits\":[0,1]}}"
        ),
    ] {
        answers.push(ok(server, "grit_world_read", &args));
    }
    answers.push(ok(
        server,
        "grit_world_slice",
        &format!("{{\"world\":{world},\"axis\":\"z\",\"at\":0}}"),
    ));
    answers.push(ok(
        server,
        "grit_world_write",
        &format!("{{\"world\":{world},\"form\":\"clear\",\"from\":[4,4,0],\"to\":[20,9,1]}}"),
    ));
    answers.push(ok(
        server,
        "grit_world_read",
        &format!("{{\"world\":{world},\"from\":[0,0,0],\"to\":[63,63,0]}}"),
    ));
    answers.push(ok(
        server,
        "grit_world_close",
        &format!("{{\"world\":{world}}}"),
    ));
    joined(&answers)
}

pub fn sparse_session(server: &mut Server<'_>) -> String {
    let mut answers = Vec::new();
    let made = ok(
        server,
        "grit_world_create",
        &format!(
            "{{\"wide\":64,\"deep\":64,\"tall\":32,\"law\":{},\"params\":[1,2],\"sparse\":true,\"budget_ms\":{LONG}}}",
            quoted(MIX)
        ),
    );
    let world = int_at(&made, "world");
    answers.push(steady(&made));
    let mut rng = Rng(0x5_9A55);
    let cells: Vec<([u32; 3], u32)> = (0..700)
        .map(|_| {
            (
                [
                    rng.below(64) as u32,
                    rng.below(64) as u32,
                    rng.below(32) as u32,
                ],
                rng.word() & 0x00FF_FFFF,
            )
        })
        .collect();
    for answer in write_cells(server, world, 0, &cells) {
        answers.push(steady(&answer));
    }
    answers.push(steady(&ok(
        server,
        "grit_world_step",
        &format!("{{\"world\":{world},\"frames\":2,\"budget_ms\":{LONG}}}"),
    )));
    answers.push(ok(
        server,
        "grit_world_read",
        &format!("{{\"world\":{world},\"form\":\"set\",\"from\":[0,0,0],\"to\":[63,63,31]}}"),
    ));
    answers.push(ok(
        server,
        "grit_world_read",
        &format!("{{\"world\":{world},\"from\":[10,10,10],\"to\":[25,25,25]}}"),
    ));
    answers.push(ok(
        server,
        "grit_world_close",
        &format!("{{\"world\":{world}}}"),
    ));
    joined(&answers)
}

pub struct Pairs {
    pub size: [u32; 3],
    pub inputs: Vec<Vec<u32>>,
    pub outputs: Vec<Vec<u32>>,
}

pub fn life_pairs(seed: u64, count: usize, wrapped: bool) -> Pairs {
    let size = [32, 32, 1];
    let mut rng = Rng(seed);
    let inputs: Vec<Vec<u32>> = (0..count)
        .map(|at| soup(&mut rng, size, 250 + 70 * at as u64))
        .collect();
    let outputs = inputs.iter().map(|one| life(size, one, wrapped)).collect();
    Pairs {
        size,
        inputs,
        outputs,
    }
}

pub fn container_worlds(server: &mut Server<'_>, pairs: &Pairs) -> i64 {
    let tall = (pairs.inputs.len() as u32).next_power_of_two().max(2);
    let made = ok(
        server,
        "grit_world_create",
        &format!(
            "{{\"wide\":32,\"deep\":32,\"tall\":{tall},\"law\":{},\"params\":[],\"budget_ms\":{LONG}}}",
            quoted(CONTAINER)
        ),
    );
    let world = int_at(&made, "world");
    for (at, (input, output)) in pairs.inputs.iter().zip(&pairs.outputs).enumerate() {
        write_cells(server, world, 0, &set_cells(pairs.size, input, at as u32));
        write_cells(server, world, 1, &set_cells(pairs.size, output, at as u32));
    }
    world
}

pub fn learn_args(world: i64, count: usize, border: &str) -> String {
    let examples: Vec<String> = (0..count)
        .map(|at| format!("{{\"world\":{world},\"from\":[0,0,{at}],\"to\":[31,31,{at}]}}"))
        .collect();
    format!(
        "{{\"stencil\":\"3x3\",\"reads\":[{{\"plane\":0,\"bit\":0}}],\"writes\":[{{\"plane\":1,\"bit\":0}}],\"border\":\"{border}\",\"rule\":\"outer\",\"examples\":[{}],\"budget_ms\":{LONG}}}",
        examples.join(",")
    )
}

pub fn check_args(law: &str, world: i64, count: usize, border: &str) -> String {
    let starts: Vec<String> = (0..count)
        .map(|at| {
            let z = at - at % 2;
            format!(
                "{{\"world\":{world},\"from\":[0,0,{z}],\"plane\":0,\"targets\":[{{\"frame\":1,\"world\":{world},\"from\":[0,0,{z}],\"plane\":1}}]}}"
            )
        })
        .collect();
    format!(
        "{{\"law\":{},\"alternate\":[0,1],\"size\":[32,32,2],\"border\":\"{border}\",\"frames\":1,\"starts\":[{}],\"budget_ms\":{LONG}}}",
        quoted(law),
        starts.join(",")
    )
}

pub fn learn_session(server: &mut Server<'_>) -> String {
    let pairs = life_pairs(0x1EA2, 4, true);
    let world = container_worlds(server, &pairs);
    let learned = ok(server, "grit_law_learn", &learn_args(world, 4, "wrapped"));
    let law = text_at(&learned, "law");
    let checked = ok(
        server,
        "grit_law_check",
        &check_args(&law, world, 4, "wrapped"),
    );
    joined(&[learned, checked])
}

pub fn described(server: &mut Server<'_>) -> String {
    let got = ok(server, "grit_describe", "{}");
    let codes: Vec<Json> = got
        .get("codes")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .filter(|one| {
            one.get("code")
                .and_then(Json::text)
                .is_none_or(|code| !NEW_CODES.contains(&code))
        })
        .cloned()
        .collect();
    let limits = match got.get("limits").cloned().unwrap() {
        Json::Map(pairs) => Json::Map(
            pairs
                .into_iter()
                .filter(|(key, _)| !NEW_LIMITS.contains(&key.as_str()))
                .collect(),
        ),
        other => other,
    };
    let kept = Json::Map(vec![
        ("tools".to_owned(), got.get("tools").cloned().unwrap()),
        ("codes".to_owned(), Json::List(codes)),
        ("limits".to_owned(), limits),
        ("server".to_owned(), got.get("server").cloned().unwrap()),
    ]);
    bytes_hash(written(&kept).as_bytes())
}
