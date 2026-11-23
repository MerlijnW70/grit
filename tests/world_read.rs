#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use grit::branch::sha256;
use grit::serve3::Server;
use grit::wire::{Json, written};

const LONG: u64 = 600_000;

const HEAT: &str = include_str!("law/heat.law.json");
const MIX: &str = include_str!("law/mix.law.json");
const KEEP: &str = r#"{"law":1,"name":"keep","params":0,"channels":1,"halo":0,"tally":{"counts":1,"weighted":false},"consts":{},"defs":{},"step":["let",[["out",["map","w","cells",["sel",["eq","w",0],0,["or","w",1]]]]],["vec","out",["vec",["count",["map","k",["range",8],["ne",["at","cells","k"],["at","out","k"]]]]]]]}"#;

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

    fn word(&mut self) -> u32 {
        u32::try_from(self.next() >> 32).unwrap()
    }
}

fn hex(sum: &[u8; 32]) -> String {
    sum.iter().map(|byte| format!("{byte:02x}")).collect()
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
    let shown = written(got);
    assert_eq!(
        held.get("isError"),
        Some(&Json::Bool(error)),
        "{}",
        &shown[..shown.len().min(4000)]
    );
    held.get("structuredContent").cloned().unwrap()
}

fn ok(server: &mut Server<'_>, name: &str, arguments: &str) -> Json {
    result(&server.heard(&line(name, arguments)).unwrap(), false)
}

fn quoted(text: &str) -> String {
    written(&Json::Text(text.to_owned()))
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

fn life_law(server: &mut Server<'_>) -> String {
    let size = [16_u32, 16, 1];
    let mut rng = Rng(41);
    let mut examples = Vec::new();
    for permille in [250_u64, 350, 450, 600] {
        let input: Vec<u32> = (0..256)
            .map(|_| u32::from(rng.below(1000) < permille))
            .collect();
        let output: Vec<u32> = (0..256_usize)
            .map(|at| {
                let (x, y) = ((at % 16) as i64, (at / 16) as i64);
                let mut around = 0;
                for dy in -1..=1_i64 {
                    for dx in -1..=1_i64 {
                        if dx == 0 && dy == 0 {
                            continue;
                        }
                        let near =
                            ((y + dy).rem_euclid(16) * 16 + (x + dx).rem_euclid(16)) as usize;
                        around += input[near];
                    }
                }
                u32::from(around == 3 || (input[at] == 1 && around == 2))
            })
            .collect();
        let shown = |words: &[u32]| {
            words
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(",")
        };
        examples.push(format!(
            "{{\"size\":[{},{},{}],\"input\":[[{}]],\"output\":[[{}]]}}",
            size[0],
            size[1],
            size[2],
            shown(&input),
            shown(&output)
        ));
    }
    let got = ok(
        server,
        "grit_law_learn",
        &format!(
            "{{\"stencil\":\"3x3\",\"reads\":[{{\"plane\":0,\"bit\":0}}],\"writes\":[{{\"plane\":1,\"bit\":0}}],\"border\":\"wrapped\",\"rule\":\"outer\",\"examples\":[{}],\"budget_ms\":{LONG}}}",
            examples.join(",")
        ),
    );
    text_at(&got, "law")
}

struct Kind {
    name: &'static str,
    size: [u32; 3],
    planes: u32,
    mask: u32,
    sparse: bool,
    iterating: bool,
}

const KINDS: [Kind; 5] = [
    Kind {
        name: "sand-rule",
        size: [16, 16, 8],
        planes: 1,
        mask: 0xFF,
        sparse: false,
        iterating: false,
    },
    Kind {
        name: "heat-law",
        size: [16, 16, 16],
        planes: 1,
        mask: 0xFFFF_FFFF,
        sparse: false,
        iterating: false,
    },
    Kind {
        name: "mix-sparse",
        size: [32, 32, 32],
        planes: 1,
        mask: 0xFFFF_FFFF,
        sparse: true,
        iterating: false,
    },
    Kind {
        name: "life-dense-2d",
        size: [32, 32, 2],
        planes: 2,
        mask: 1,
        sparse: false,
        iterating: true,
    },
    Kind {
        name: "mix-dense-3d",
        size: [16, 16, 16],
        planes: 1,
        mask: 0xFFFF_FFFF,
        sparse: false,
        iterating: false,
    },
];

fn created(server: &mut Server<'_>, kind: &Kind) -> i64 {
    let [wide, deep, tall] = kind.size;
    let shape = format!(
        "\"wide\":{wide},\"deep\":{deep},\"tall\":{tall},\"sparse\":{},\"budget_ms\":{LONG}",
        kind.sparse
    );
    let arguments = match kind.name {
        "sand-rule" => format!("{{\"rule\":\"sand\",{shape}}}"),
        "heat-law" => format!(
            "{{\"law\":{},\"params\":[300,7,900],{shape}}}",
            quoted(HEAT)
        ),
        "keep-sparse" => format!("{{\"law\":{},\"params\":[],{shape}}}", quoted(KEEP)),
        "mix-sparse" | "mix-dense-3d" => {
            format!("{{\"law\":{},\"params\":[3000,1700],{shape}}}", quoted(MIX))
        }
        _ => {
            let law = life_law(server);
            format!("{{\"law\":{},\"alternate\":[0,1],{shape}}}", quoted(&law))
        }
    };
    int_at(&ok(server, "grit_world_create", &arguments), "world")
}

fn seeded(kind: &Kind, rng: &mut Rng, count: usize) -> Vec<([u32; 3], u32)> {
    (0..count)
        .map(|_| {
            let at =
                [0, 1, 2].map(|axis| u32::try_from(rng.below(u64::from(kind.size[axis]))).unwrap());
            (at, rng.word() & kind.mask)
        })
        .collect()
}

fn cells_text(cells: &[([u32; 3], u32)]) -> String {
    let shown: Vec<String> = cells
        .iter()
        .map(|(at, word)| format!("[{},{},{},{word}]", at[0], at[1], at[2]))
        .collect();
    format!("[{}]", shown.join(","))
}

fn write(server: &mut Server<'_>, world: i64, plane: &str, cells: &[([u32; 3], u32)]) {
    for piece in cells.chunks(4096) {
        ok(
            server,
            "grit_world_write",
            &format!(
                "{{\"world\":{world},{plane}\"cells\":{},\"budget_ms\":{LONG}}}",
                cells_text(piece)
            ),
        );
    }
}

fn step(server: &mut Server<'_>, world: i64, frames: u32) {
    ok(
        server,
        "grit_world_step",
        &format!("{{\"world\":{world},\"frames\":{frames},\"budget_ms\":{LONG}}}"),
    );
}

fn planes_of(kind: &Kind) -> Vec<String> {
    let mut planes: Vec<String> = (0..kind.planes)
        .map(|plane| format!("\"plane\":{plane},"))
        .collect();
    planes.push(String::new());
    if kind.iterating {
        planes.push("\"current\":true,".to_owned());
    }
    planes
}

fn existing_answers(server: &mut Server<'_>, kind: &Kind, seed: u64) -> Vec<String> {
    let world = created(server, kind);
    let mut rng = Rng(seed);
    let first = seeded(kind, &mut rng, 600);
    let plane = if kind.iterating {
        "\"current\":true,"
    } else {
        ""
    };
    write(server, world, plane, &first);
    step(server, world, 3);
    let second = seeded(kind, &mut rng, 200);
    write(server, world, plane, &second);
    let [wide, deep, tall] = kind.size;
    let boxes = [
        ([0, 0, 0], [7, 7, 1]),
        ([3, 5, 1], [3, 5, 1]),
        (
            [wide - 4, deep - 3, tall - 2],
            [wide - 1, deep - 1, tall - 1],
        ),
        (
            [0, 0, 0],
            [wide - 1, deep - 1, (4096 / (wide * deep)).min(tall) - 1],
        ),
    ];
    let mut answers = Vec::new();
    for plane in planes_of(kind) {
        for (from, to) in boxes {
            let got = ok(
                server,
                "grit_world_read",
                &format!(
                    "{{\"world\":{world},{plane}\"from\":[{},{},{}],\"to\":[{},{},{}]}}",
                    from[0], from[1], from[2], to[0], to[1], to[2]
                ),
            );
            answers.push(written(&got));
        }
        for (axis, at, shown, cursor) in [
            ("z", 0, "kinds", 0),
            ("x", 1, "words", 0),
            ("y", deep - 1, "kinds", 1),
            ("z", tall - 1, "words", 2),
        ] {
            let got = ok(
                server,
                "grit_world_slice",
                &format!(
                    "{{\"world\":{world},{plane}\"axis\":\"{axis}\",\"at\":{at},\"as\":\"{shown}\",\"cursor\":{cursor}}}"
                ),
            );
            answers.push(written(&got));
        }
    }
    step(server, world, 1);
    let got = ok(
        server,
        "grit_world_read",
        &format!("{{\"world\":{world},\"from\":[1,2,0],\"to\":[9,4,1]}}"),
    );
    answers.push(written(&got));
    answers
}

const RECORDED: [(&str, &str); 5] = [
    (
        "sand-rule",
        "41992e5ac0c989b48266726118df5b062a0a0609204791595c31a5a959e57c19",
    ),
    (
        "heat-law",
        "dc5eee6c2e1bbca8521f253b1d623e0ffdaa7c605b214dbeb53e8883e205f92c",
    ),
    (
        "mix-sparse",
        "4ad6eacc6183978f632154aee63f7ad82718454932e90b4a30cf95ce8da894a4",
    ),
    (
        "life-dense-2d",
        "637a715bdb97c589d926d420023725853cb6b374ee562d5d639a05ecc87b5272",
    ),
    (
        "mix-dense-3d",
        "ce1e61de111042605e374941ec8473fd4c79e87cfd4c72821902f198dcbe2a1f",
    ),
];

#[test]
fn r4_existing_reads_answer_as_on_the_recorded_base() {
    let Some(host) = device("world read recorded") else {
        return;
    };
    let mut wrong = Vec::new();
    for (kind, (name, want)) in KINDS.iter().zip(RECORDED) {
        assert_eq!(kind.name, name);
        let mut server = Server::new(Some(&host));
        let answers = existing_answers(&mut server, kind, 0x5EED ^ u64::from(kind.size[0]));
        let got = hex(&sha256(answers.join("\n").as_bytes()));
        println!("RECORD {name} {got}");
        if got != want {
            wrong.push(format!("{name}: {got} against {want}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

const EXACT: [Kind; 6] = [
    Kind {
        name: "sand-rule",
        size: [16, 16, 8],
        planes: 1,
        mask: 0xFF,
        sparse: false,
        iterating: false,
    },
    Kind {
        name: "heat-law",
        size: [16, 16, 16],
        planes: 1,
        mask: 0xFFFF_FFFF,
        sparse: false,
        iterating: false,
    },
    Kind {
        name: "mix-sparse",
        size: [32, 32, 32],
        planes: 1,
        mask: 0xFFFF_FFFF,
        sparse: true,
        iterating: false,
    },
    Kind {
        name: "keep-sparse",
        size: [32, 32, 16],
        planes: 1,
        mask: 1,
        sparse: true,
        iterating: false,
    },
    Kind {
        name: "life-dense-2d",
        size: [32, 32, 2],
        planes: 2,
        mask: 1,
        sparse: false,
        iterating: true,
    },
    Kind {
        name: "mix-dense-3d",
        size: [32, 16, 16],
        planes: 1,
        mask: 0xFFFF_FFFF,
        sparse: false,
        iterating: false,
    },
];

const SET_MOST: usize = 16_384;

type Cell = [u32; 4];

fn dense_plane(server: &mut Server<'_>, world: i64, size: [u32; 3], plane: &str) -> Vec<u32> {
    let per = (4096 / (size[0] * size[1])).max(1);
    let mut out = Vec::new();
    let mut z = 0;
    while z < size[2] {
        let top = (z + per).min(size[2]) - 1;
        let got = ok(
            server,
            "grit_world_read",
            &format!(
                "{{\"world\":{world},{plane}\"from\":[0,0,{z}],\"to\":[{},{},{top}]}}",
                size[0] - 1,
                size[1] - 1
            ),
        );
        for cell in got.get("cells").and_then(Json::list).unwrap() {
            out.push(u32::try_from(cell.int().unwrap()).unwrap());
        }
        z = top + 1;
    }
    out
}

fn boxed(size: [u32; 3], from: [u32; 3], to: [u32; 3]) -> Vec<(usize, [u32; 3])> {
    let mut out = Vec::new();
    for z in from[2]..=to[2] {
        for y in from[1]..=to[1] {
            for x in from[0]..=to[0] {
                let index = ((z * size[1] + y) * size[0] + x) as usize;
                out.push((index, [x, y, z]));
            }
        }
    }
    out
}

fn wanted_set(words: &[u32], size: [u32; 3], from: [u32; 3], to: [u32; 3]) -> Vec<Cell> {
    boxed(size, from, to)
        .into_iter()
        .filter(|(index, _)| words[*index] != 0)
        .map(|(index, at)| [at[0], at[1], at[2], words[index]])
        .collect()
}

fn wanted_changed(
    was: &[u32],
    now: &[u32],
    size: [u32; 3],
    from: [u32; 3],
    to: [u32; 3],
) -> Vec<Cell> {
    boxed(size, from, to)
        .into_iter()
        .filter(|(index, _)| was[*index] != now[*index])
        .map(|(index, at)| [at[0], at[1], at[2], now[index]])
        .collect()
}

fn listed_cells(got: &Json) -> Vec<Cell> {
    got.get("cells")
        .and_then(Json::list)
        .unwrap_or_else(|| panic!("no cells in {}", written(got)))
        .iter()
        .map(|cell| {
            let parts: Vec<u32> = cell
                .list()
                .unwrap()
                .iter()
                .map(|one| u32::try_from(one.int().unwrap()).unwrap())
                .collect();
            <[u32; 4]>::try_from(parts.as_slice()).unwrap()
        })
        .collect()
}

fn corners(from: [u32; 3], to: [u32; 3]) -> String {
    format!(
        "\"from\":[{},{},{}],\"to\":[{},{},{}]",
        from[0], from[1], from[2], to[0], to[1], to[2]
    )
}

fn asked(
    server: &mut Server<'_>,
    world: i64,
    plane: &str,
    (from, to): ([u32; 3], [u32; 3]),
    form: &str,
) -> Json {
    server
        .heard(&line(
            "grit_world_read",
            &format!("{{\"world\":{world},{plane}{},{form}}}", corners(from, to)),
        ))
        .unwrap()
}

#[derive(Default, Debug)]
struct Seen {
    listed: usize,
    changed: usize,
    refused: usize,
}

fn listed_or_refused(
    seen: &mut Seen,
    server: &mut Server<'_>,
    world: i64,
    plane: &str,
    (corner, form): (([u32; 3], [u32; 3]), &str),
    wanted: &[Cell],
    label: &str,
) -> Option<Json> {
    let got = asked(server, world, plane, corner, form);
    if wanted.len() > SET_MOST {
        let held = result(&got, true);
        assert_eq!(text_at(&held, "code"), "too_many", "{label}");
        assert_eq!(text_at(&held, "field"), "$.to", "{label}");
        assert_eq!(int_at(&held, "limit"), SET_MOST as i64, "{label}");
        assert_eq!(int_at(&held, "count"), wanted.len() as i64, "{label}");
        seen.refused += 1;
        return None;
    }
    let held = result(&got, false);
    let cells = listed_cells(&held);
    assert_eq!(int_at(&held, "count"), cells.len() as i64, "{label}");
    assert!(
        cells == wanted,
        "{label}: {} listed, {} wanted",
        cells.len(),
        wanted.len()
    );
    if form.contains("changed") {
        seen.changed += cells.len();
    } else {
        seen.listed += cells.len();
    }
    Some(held)
}

fn place_of(size: [u32; 3], index: usize) -> [u32; 3] {
    let wide = size[0] as usize;
    let deep = size[1] as usize;
    [
        (index % wide) as u32,
        ((index / wide) % deep) as u32,
        (index / (wide * deep)) as u32,
    ]
}

fn changes(kind: &Kind, rng: &mut Rng, now: &[u32]) -> Vec<([u32; 3], u32)> {
    let mut cells = seeded(kind, rng, 120);
    for cell in cells.iter_mut().take(40) {
        cell.1 = 0;
    }
    let set: Vec<usize> = (0..now.len()).filter(|&at| now[at] != 0).collect();
    for nth in 0..30 {
        if set.is_empty() {
            break;
        }
        let at = set[(rng.below(set.len() as u64)) as usize];
        let word = if nth % 2 == 0 { 0 } else { now[at] };
        cells.push((place_of(kind.size, at), word));
    }
    cells
}

fn references(kind: &Kind) -> Vec<String> {
    let mut out: Vec<String> = (0..kind.planes)
        .map(|plane| format!("\"plane\":{plane},"))
        .collect();
    if kind.iterating {
        out.push("\"current\":true,".to_owned());
    }
    out
}

fn exact_round(seen: &mut Seen, server: &mut Server<'_>, kind: &Kind, world: i64, rng: &mut Rng) {
    let size = kind.size;
    let [wide, deep, tall] = size;
    let boxes = [
        ([0, 0, 0], [wide - 1, deep - 1, tall - 1]),
        ([1, 2, 0], [wide - 2, deep / 2, tall - 1]),
        (
            [wide - 1, deep - 1, tall - 1],
            [wide - 1, deep - 1, tall - 1],
        ),
        ([0, 3, 1], [wide - 1, 3, 1]),
    ];
    for plane in references(kind) {
        let label = format!("{} {plane}", kind.name);
        let was = dense_plane(server, world, size, &plane);
        let mut marks = Vec::new();
        for corner in boxes {
            let wanted = wanted_set(&was, size, corner.0, corner.1);
            let got = listed_or_refused(
                seen,
                server,
                world,
                &plane,
                (corner, "\"form\":\"set\",\"mark\":true"),
                &wanted,
                &format!("{label} set {corner:?}"),
            );
            marks.push(got.map(|held| text_at(&held, "mark")));
        }
        step(server, world, 1);
        let stepped = dense_plane(server, world, size, &plane);
        let cells = changes(kind, rng, &stepped);
        write(server, world, &plane, &cells);
        let now = dense_plane(server, world, size, &plane);
        for (corner, mark) in boxes.iter().zip(&marks) {
            let wanted = wanted_set(&now, size, corner.0, corner.1);
            listed_or_refused(
                seen,
                server,
                world,
                &plane,
                (*corner, "\"form\":\"set\""),
                &wanted,
                &format!("{label} set after {corner:?}"),
            );
            let Some(mark) = mark else {
                continue;
            };
            let wanted = wanted_changed(&was, &now, size, corner.0, corner.1);
            let held = listed_or_refused(
                seen,
                server,
                world,
                &plane,
                (
                    *corner,
                    &format!("\"form\":\"changed\",\"since\":\"{mark}\""),
                ),
                &wanted,
                &format!("{label} changed {corner:?}"),
            );
            if let Some(held) = held {
                assert_eq!(&text_at(&held, "since"), mark);
            }
        }
    }
}

#[test]
fn r1_sparse_reads_equal_the_dense_read_filtered_on_the_host() {
    let Some(host) = device("world read exact") else {
        return;
    };
    for kind in &EXACT {
        let mut server = Server::new(Some(&host));
        let world = created(&mut server, kind);
        let mut rng = Rng(0xE1 ^ u64::from(kind.size[2]));
        let writer = if kind.iterating {
            "\"current\":true,"
        } else {
            ""
        };
        let soup = seeded(kind, &mut rng, 900);
        write(&mut server, world, writer, &soup);
        let mut seen = Seen::default();
        exact_round(&mut seen, &mut server, kind, world, &mut rng);
        step(&mut server, world, 5);
        exact_round(&mut seen, &mut server, kind, world, &mut rng);
        println!("R1 {} exact {seen:?}", kind.name);
        assert!(
            seen.listed > 0 && seen.changed > 0,
            "{}: {seen:?}",
            kind.name
        );
    }
}

fn no(server: &mut Server<'_>, name: &str, arguments: &str) -> Json {
    result(&server.heard(&line(name, arguments)).unwrap(), true)
}

fn refused_at(held: &Json, code: &str, field: &str, label: &str) {
    assert_eq!(text_at(held, "code"), code, "{label}: {}", written(held));
    assert_eq!(text_at(held, "field"), field, "{label}: {}", written(held));
}

struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "grit-world-read-{label}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn options(&self) -> grit::serve3::Options {
        grit::serve3::Options {
            dir: Some(self.0.join("worlds")),
            memory: None,
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const LIFE: Kind = Kind {
    name: "life-dense-2d",
    size: [32, 32, 2],
    planes: 2,
    mask: 1,
    sparse: false,
    iterating: true,
};

const WHOLE: ([u32; 3], [u32; 3]) = ([0, 0, 0], [31, 31, 1]);

const CURRENT: &str = "\"current\":true,";

fn set_mark(server: &mut Server<'_>, world: i64) -> (String, Vec<u32>) {
    let words = dense_plane(server, world, LIFE.size, CURRENT);
    let held = result(
        &asked(
            server,
            world,
            CURRENT,
            WHOLE,
            "\"form\":\"set\",\"mark\":true",
        ),
        false,
    );
    assert_eq!(
        listed_cells(&held),
        wanted_set(&words, LIFE.size, WHOLE.0, WHOLE.1)
    );
    (text_at(&held, "mark"), words)
}

fn changed_is(
    server: &mut Server<'_>,
    world: i64,
    mark: &str,
    was: &[u32],
    extra: &str,
    label: &str,
) -> (Json, Vec<u32>) {
    let now = dense_plane(server, world, LIFE.size, CURRENT);
    let held = result(
        &asked(
            server,
            world,
            CURRENT,
            WHOLE,
            &format!("\"form\":\"changed\",\"since\":\"{mark}\"{extra}"),
        ),
        false,
    );
    let wanted = wanted_changed(was, &now, LIFE.size, WHOLE.0, WHOLE.1);
    let got = listed_cells(&held);
    assert!(
        got == wanted,
        "{label}: {} listed, {} wanted",
        got.len(),
        wanted.len()
    );
    println!("R2 {label}: {} changed", got.len());
    (held, now)
}

fn unknown(server: &mut Server<'_>, world: i64, mark: &str, label: &str) {
    let held = result(
        &asked(
            server,
            world,
            CURRENT,
            WHOLE,
            &format!("\"form\":\"changed\",\"since\":\"{mark}\""),
        ),
        true,
    );
    refused_at(&held, "mark_unknown", "$.since", label);
}

#[test]
fn r2_changed_reads_hold_across_iteration_writes_fork_restore_save_and_restart() {
    let Some(host) = device("world read changed") else {
        return;
    };
    let scratch = Scratch::new("r2");
    let options = scratch.options();
    let mut rng = Rng(0x2C);
    let (world, mark, was) = {
        let mut server = Server::with(Some(&host), &options).unwrap();
        let world = created(&mut server, &LIFE);
        let soup = seeded(&LIFE, &mut rng, 400);
        write(&mut server, world, CURRENT, &soup);
        let (mark, was) = set_mark(&mut server, world);

        step(&mut server, world, 1);
        let (_, stepped) = changed_is(&mut server, world, &mark, &was, "", "iteration");
        assert_ne!(stepped, was);
        step(&mut server, world, 6);
        changed_is(&mut server, world, &mark, &was, "", "iteration 7");

        let cells = changes(&LIFE, &mut rng, &stepped);
        write(&mut server, world, CURRENT, &cells);
        let (held, now) = changed_is(&mut server, world, &mark, &was, ",\"mark\":true", "writes");
        let chained = text_at(&held, "mark");
        let (quiet, _) = changed_is(&mut server, world, &chained, &now, "", "chained mark");
        assert_eq!(int_at(&quiet, "count"), 0);
        let one = [([5_u32, 6, 0], 1_u32), ([7, 6, 0], 0)];
        write(&mut server, world, CURRENT, &one);
        changed_is(&mut server, world, &chained, &now, "", "one write");

        let forked = int_at(
            &ok(
                &mut server,
                "grit_world_fork",
                &format!("{{\"world\":{world}}}"),
            ),
            "world",
        );
        changed_is(&mut server, forked, &mark, &was, "", "fork");
        step(&mut server, forked, 3);
        changed_is(&mut server, forked, &mark, &was, "", "fork stepped");
        changed_is(&mut server, world, &mark, &was, "", "source after fork");

        step(&mut server, world, 2);
        ok(
            &mut server,
            "grit_world_restore",
            &format!("{{\"world\":{world},\"from\":{forked}}}"),
        );
        changed_is(&mut server, world, &mark, &was, "", "restore");

        ok(
            &mut server,
            "grit_world_save",
            &format!("{{\"world\":{world},\"name\":\"kept\"}}"),
        );
        let opened = int_at(
            &ok(&mut server, "grit_world_open", "{\"name\":\"kept\"}"),
            "world",
        );
        unknown(&mut server, opened, &mark, "opened file");
        let (again, words) = set_mark(&mut server, opened);
        step(&mut server, opened, 1);
        changed_is(&mut server, opened, &again, &words, "", "opened stepped");
        unknown(&mut server, forked, &again, "mark of another world");
        changed_is(&mut server, world, &mark, &was, "", "source after save");
        (world, mark, was)
    };
    let mut server = Server::with(Some(&host), &options).unwrap();
    let reopened = int_at(
        &ok(
            &mut server,
            "grit_world_open",
            &format!("{{\"name\":\"world-{world}\"}}"),
        ),
        "world",
    );
    unknown(&mut server, reopened, &mark, "restart");
    let (fresh, words) = set_mark(&mut server, reopened);
    step(&mut server, reopened, 2);
    changed_is(&mut server, reopened, &fresh, &words, "", "after restart");
    assert_ne!(words, was);
}

#[test]
fn r2_a_mark_is_dropped_past_the_marks_held_and_refused_by_name() {
    let Some(host) = device("world read marks held") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let world = created(&mut server, &LIFE);
    let mut rng = Rng(0x2D);
    let mut marks = Vec::new();
    for _ in 0..17 {
        let soup = seeded(&LIFE, &mut rng, 3);
        write(&mut server, world, CURRENT, &soup);
        marks.push(set_mark(&mut server, world));
    }
    unknown(&mut server, world, &marks[0].0, "oldest of 17");
    for (mark, words) in &marks[1..] {
        changed_is(&mut server, world, mark, words, "", "held");
    }
}

#[test]
fn r5_refusals_by_code_and_field() {
    let Some(host) = device("world read refusals") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let world = created(&mut server, &LIFE);
    let mut rng = Rng(0x55);
    let soup = seeded(&LIFE, &mut rng, 300);
    write(&mut server, world, CURRENT, &soup);
    let (mark, _) = set_mark(&mut server, world);
    let read = |server: &mut Server<'_>, arguments: &str| {
        no(
            server,
            "grit_world_read",
            &format!("{{\"world\":{world},{arguments}}}"),
        )
    };
    let whole = corners(WHOLE.0, WHOLE.1);
    let cases = [
        (
            format!("{whole},\"form\":\"changed\""),
            "read_form",
            "$.since",
        ),
        (
            format!("{whole},\"form\":\"set\",\"since\":\"{mark}\""),
            "read_form",
            "$.since",
        ),
        (
            format!("{whole},\"since\":\"{mark}\""),
            "read_form",
            "$.since",
        ),
        (format!("{whole},\"mark\":true"), "read_form", "$.mark"),
        (
            format!("{whole},\"form\":\"changed\",\"current\":true,\"since\":\"0123456789abcdef\""),
            "mark_unknown",
            "$.since",
        ),
        (
            format!(
                "\"from\":[1,0,0],\"to\":[31,31,1],\"form\":\"changed\",\"current\":true,\"since\":\"{mark}\""
            ),
            "mark_unlike",
            "$.from",
        ),
        (
            format!(
                "\"from\":[0,0,0],\"to\":[31,30,1],\"form\":\"changed\",\"current\":true,\"since\":\"{mark}\""
            ),
            "mark_unlike",
            "$.to",
        ),
        (
            format!("{whole},\"form\":\"changed\",\"plane\":0,\"since\":\"{mark}\""),
            "mark_unlike",
            "$.plane",
        ),
        (
            format!("{whole},\"form\":\"set\",\"plane\":2"),
            "outside",
            "$.plane",
        ),
        (
            "\"from\":[0,0,0],\"to\":[32,0,0],\"form\":\"set\"".to_owned(),
            "outside",
            "$.to",
        ),
        (
            "\"from\":[3,0,0],\"to\":[2,0,0],\"form\":\"set\"".to_owned(),
            "reversed",
            "$.to",
        ),
        (
            format!("{whole},\"form\":\"sparse\""),
            "malformed",
            "$.form",
        ),
        (
            format!("{whole},\"form\":\"set\",\"mark\":1"),
            "malformed",
            "$.mark",
        ),
        (
            format!("{whole},\"form\":\"changed\",\"since\":7"),
            "malformed",
            "$.since",
        ),
    ];
    for (arguments, code, field) in &cases {
        let held = read(&mut server, arguments);
        refused_at(&held, code, field, arguments);
    }
    let plain = int_at(
        &ok(
            &mut server,
            "grit_world_create",
            &format!(
                "{{\"law\":{},\"params\":[3000,1700],\"wide\":32,\"deep\":32,\"tall\":32,\"budget_ms\":{LONG}}}",
                quoted(MIX)
            ),
        ),
        "world",
    );
    let held = no(
        &mut server,
        "grit_world_read",
        &format!("{{\"world\":{plain},{whole},\"form\":\"set\",\"current\":true}}"),
    );
    refused_at(&held, "not_iterating", "$.current", "not iterating");
    let held = no(
        &mut server,
        "grit_world_read",
        &format!("{{\"world\":{plain},{whole},\"form\":\"set\",\"since\":\"{mark}\"}}"),
    );
    refused_at(&held, "read_form", "$.since", "since on set");
    let held = no(
        &mut server,
        "grit_world_read",
        &format!("{{\"world\":{plain},{whole},\"form\":\"changed\",\"since\":\"{mark}\"}}"),
    );
    refused_at(&held, "mark_unknown", "$.since", "mark of another world");
}

fn filled(server: &mut Server<'_>, world: i64, size: [u32; 3], count: usize) {
    let mut cells = Vec::with_capacity(count);
    for at in 0..count {
        cells.push((place_of(size, at * 2), (at as u32) | 1));
    }
    write(server, world, "", &cells);
}

#[test]
fn r5_the_limit_lists_up_to_set_most_and_refuses_past_it_with_the_count() {
    let Some(host) = device("world read limit") else {
        return;
    };
    for sparse in [false, true] {
        let mut server = Server::new(Some(&host));
        let make = format!(
            "{{\"law\":{},\"params\":[3000,1700],\"wide\":64,\"deep\":32,\"tall\":32,\"sparse\":{sparse},{}\"budget_ms\":{LONG}}}",
            quoted(MIX),
            if sparse { "\"slots\":4096," } else { "" }
        );
        let size = [64, 32, 32];
        let whole = ([0, 0, 0], [63, 31, 31]);
        let world = int_at(&ok(&mut server, "grit_world_create", &make), "world");
        filled(&mut server, world, size, SET_MOST);
        let held = result(
            &asked(
                &mut server,
                world,
                "",
                whole,
                "\"form\":\"set\",\"mark\":true",
            ),
            false,
        );
        assert_eq!(int_at(&held, "count"), SET_MOST as i64);
        let cells = listed_cells(&held);
        assert_eq!(cells.len(), SET_MOST);
        assert_eq!(
            cells.last().unwrap()[..3],
            place_of(size, (SET_MOST - 1) * 2)
        );
        let mark = text_at(&held, "mark");
        let extra = [(place_of(size, 1), 9_u32), (place_of(size, 3), 11)];
        write(&mut server, world, "", &extra);
        let held = result(
            &asked(&mut server, world, "", whole, "\"form\":\"set\""),
            true,
        );
        refused_at(&held, "too_many", "$.to", "set over");
        assert_eq!(int_at(&held, "limit"), SET_MOST as i64);
        assert_eq!(int_at(&held, "count"), SET_MOST as i64 + 2);
        let held = result(
            &asked(
                &mut server,
                world,
                "",
                whole,
                &format!("\"form\":\"changed\",\"since\":\"{mark}\",\"mark\":true"),
            ),
            true,
        );
        refused_at(&held, "too_many", "$.mark", "mark over");
        assert_eq!(int_at(&held, "limit"), SET_MOST as i64);
        assert_eq!(int_at(&held, "count"), SET_MOST as i64 + 2);
        let held = result(
            &asked(
                &mut server,
                world,
                "",
                whole,
                &format!("\"form\":\"changed\",\"since\":\"{mark}\""),
            ),
            false,
        );
        assert_eq!(listed_cells(&held), vec![[1, 0, 0, 9], [3, 0, 0, 11]]);
        let clear: Vec<([u32; 3], u32)> = (0..2 * SET_MOST)
            .map(|at| (place_of(size, at), 0))
            .collect();
        write(&mut server, world, "", &clear);
        let fresh = [
            (place_of(size, 5), 5_u32),
            (place_of(size, 7), 7),
            (place_of(size, 9), 9),
        ];
        write(&mut server, world, "", &fresh);
        let held = result(
            &asked(
                &mut server,
                world,
                "",
                whole,
                &format!("\"form\":\"changed\",\"since\":\"{mark}\""),
            ),
            true,
        );
        refused_at(&held, "too_many", "$.to", "changed over");
        assert_eq!(int_at(&held, "count"), SET_MOST as i64 + 3);
        let held = result(
            &asked(&mut server, world, "", whole, "\"form\":\"set\""),
            false,
        );
        assert_eq!(
            listed_cells(&held),
            vec![[5, 0, 0, 5], [7, 0, 0, 7], [9, 0, 0, 9]]
        );
    }
    let mut server = Server::new(Some(&host));
    let told = ok(&mut server, "grit_describe", "{}");
    let limits = told.get("limits").unwrap();
    assert_eq!(int_at(limits, "set_cells"), SET_MOST as i64);
    assert_eq!(int_at(limits, "marks_held"), 16);
    assert!(text_at(limits, "set_read").contains("12 bytes plus 8"));
}

#[cfg(feature = "faults")]
mod traffic {
    use super::*;

    fn moved(server: &mut Server<'_>, world: i64, arguments: &str) -> (Json, grit::Traffic) {
        let number = u32::try_from(world).unwrap();
        let before = server.traffic(number).unwrap();
        let held = result(
            &server
                .heard(&line(
                    "grit_world_read",
                    &format!("{{\"world\":{world},{arguments}}}"),
                ))
                .unwrap(),
            false,
        );
        (held, server.traffic(number).unwrap().since(before))
    }

    #[test]
    fn r3_a_sparse_read_moves_twelve_bytes_and_eight_a_listed_cell() {
        let Some(host) = device("world read traffic") else {
            return;
        };
        for sparse in [false, true] {
            let mut server = Server::new(Some(&host));
            let size = [64_u32, 64, 64];
            let world = int_at(
                &ok(
                    &mut server,
                    "grit_world_create",
                    &format!(
                        "{{\"law\":{},\"params\":[],\"wide\":64,\"deep\":64,\"tall\":64,\"sparse\":{sparse},\"budget_ms\":{LONG}}}",
                        quoted(KEEP)
                    ),
                ),
                "world",
            );
            let whole = corners([0, 0, 0], [63, 63, 63]);
            let mut rng = Rng(0x33 ^ u64::from(sparse));
            let mut seen = std::collections::BTreeSet::new();
            let mut cells = Vec::new();
            while cells.len() < 100 {
                let at =
                    [0, 1, 2].map(|axis| u32::try_from(rng.below(u64::from(size[axis]))).unwrap());
                if seen.insert(at) {
                    cells.push((at, (rng.word() | 1) & 0x7FFF_FFFF));
                }
            }
            let (held, empty) = moved(
                &mut server,
                world,
                &format!("{whole},\"form\":\"set\",\"mark\":true"),
            );
            assert_eq!(int_at(&held, "count"), 0);
            assert_eq!(empty.downloaded, 12, "empty {sparse}: {empty:?}");
            let mark = text_at(&held, "mark");
            write(&mut server, world, "", &cells);
            let (held, one) = moved(&mut server, world, &format!("{whole},\"form\":\"set\""));
            assert_eq!(int_at(&held, "count"), 100);
            assert_eq!(one.downloaded, 12 + 8 * 100, "set {sparse}: {one:?}");
            assert_eq!(one.submits, 3, "set {sparse}: {one:?}");
            let plane_bytes = 64 * 64 * 64 * 4_u64;
            assert!(
                one.downloaded * 50 < plane_bytes,
                "{} against {plane_bytes}",
                one.downloaded
            );
            let mut dense = 0_u64;
            for z in 0..64 {
                let (_, one) = moved(
                    &mut server,
                    world,
                    &format!("\"from\":[0,0,{z}],\"to\":[63,63,{z}]"),
                );
                dense += one.downloaded;
            }
            let (held, changed) = moved(
                &mut server,
                world,
                &format!("{whole},\"form\":\"changed\",\"since\":\"{mark}\",\"mark\":true"),
            );
            assert_eq!(int_at(&held, "count"), 100);
            assert_eq!(
                changed.downloaded,
                12 + 8 * 100,
                "changed {sparse}: {changed:?}"
            );
            let chained = text_at(&held, "mark");
            let few = &cells[..7];
            let cleared: Vec<([u32; 3], u32)> = few.iter().map(|&(at, _)| (at, 0)).collect();
            write(&mut server, world, "", &cleared);
            let (held, seven) = moved(
                &mut server,
                world,
                &format!("{whole},\"form\":\"changed\",\"since\":\"{chained}\""),
            );
            assert_eq!(int_at(&held, "count"), 7);
            assert_eq!(seven.downloaded, 12 + 8 * 7, "seven {sparse}: {seven:?}");
            println!(
                "R3 sparse {sparse}: dense plane {dense} bytes, set of 100 {} bytes, changed of 7 {} bytes",
                12 + 8 * 100,
                seven.downloaded
            );
        }
    }
}

#[test]
fn r1_a_plane_past_one_dispatch_is_scanned_to_its_last_cell() {
    let Some(host) = device("world read wide plane") else {
        return;
    };
    let size = [128_u32, 128, 128];
    for sparse in [false, true] {
        let mut server = Server::new(Some(&host));
        let world = int_at(
            &ok(
                &mut server,
                "grit_world_create",
                &format!(
                    "{{\"law\":{},\"params\":[],\"wide\":128,\"deep\":128,\"tall\":128,\"sparse\":{sparse},\"budget_ms\":{LONG}}}",
                    quoted(KEEP)
                ),
            ),
            "world",
        );
        let mut rng = Rng(0x128 ^ u64::from(sparse));
        let mut held = std::collections::BTreeMap::new();
        for at in [
            [0, 0, 0],
            [127, 127, 127],
            [127, 0, 0],
            [0, 127, 127],
            [64, 64, 64],
        ] {
            held.insert(at, rng.word() | 1);
        }
        while held.len() < 300 {
            let at = [0, 1, 2].map(|axis| u32::try_from(rng.below(u64::from(size[axis]))).unwrap());
            held.insert(at, rng.word() | 1);
        }
        let cells: Vec<([u32; 3], u32)> = held.iter().map(|(at, word)| (*at, *word)).collect();
        write(&mut server, world, "", &cells);
        let mut words = vec![0_u32; 128 * 128 * 128];
        for (at, word) in &cells {
            words[((at[2] * 128 + at[1]) * 128 + at[0]) as usize] = *word;
        }
        for corner in [
            ([0, 0, 0], [127, 127, 127]),
            ([1, 1, 1], [127, 127, 127]),
            ([0, 0, 64], [127, 127, 127]),
            ([127, 127, 127], [127, 127, 127]),
        ] {
            let got = result(
                &asked(&mut server, world, "", corner, "\"form\":\"set\""),
                false,
            );
            let wanted = wanted_set(&words, size, corner.0, corner.1);
            assert!(!wanted.is_empty());
            assert!(listed_cells(&got) == wanted, "{sparse} {corner:?}");
        }
    }
}

fn held_law(planes: u32) -> String {
    let cells: Vec<String> = (0..8).map(|k| format!("[\"at\",\"cells\",{k}]")).collect();
    format!(
        "{{\"law\":1,\"name\":\"held{planes}\",\"params\":0,\"channels\":{planes},\"halo\":0,\"tally\":{{\"counts\":1,\"weighted\":false}},\"consts\":{{}},\"defs\":{{}},\"step\":[\"vec\",[\"vec\",{}],[\"vec\",0]]}}",
        cells.join(",")
    )
}

fn block(from: [u32; 3], side: u32, word: u32) -> Vec<([u32; 3], u32)> {
    let mut cells = Vec::new();
    for z in 0..side {
        for y in 0..side {
            for x in 0..side {
                cells.push(([from[0] + x, from[1] + y, from[2] + z], word));
            }
        }
    }
    cells
}

fn pool_held(answer: &Json) -> i64 {
    int_at(answer.get("pool_use").unwrap(), "held")
}

fn stepped_held(server: &mut Server<'_>, world: i64, frames: u32) -> i64 {
    let got = ok(
        server,
        "grit_world_step",
        &format!("{{\"world\":{world},\"frames\":{frames},\"budget_ms\":{LONG}}}"),
    );
    pool_held(&got)
}

fn plane_checked(
    server: &mut Server<'_>,
    world: i64,
    size: [u32; 3],
    plane: &str,
    rng: &mut Rng,
    label: &str,
) -> usize {
    let whole = ([0, 0, 0], size.map(|side| side - 1));
    let inner = ([3, 5, 2], [12, 14, 9]);
    let was = dense_plane(server, world, size, plane);
    let mut marks = Vec::new();
    let mut listed = 0;
    for corner in [whole, inner] {
        let held = result(
            &asked(
                server,
                world,
                plane,
                corner,
                "\"form\":\"set\",\"mark\":true",
            ),
            false,
        );
        let wanted = wanted_set(&was, size, corner.0, corner.1);
        let cells = listed_cells(&held);
        assert!(
            cells == wanted,
            "{label} set {corner:?}: {} listed, {} wanted",
            cells.len(),
            wanted.len()
        );
        listed += cells.len();
        marks.push(text_at(&held, "mark"));
    }
    for (corner, mark) in [whole, inner].iter().zip(&marks) {
        let held = result(
            &asked(
                server,
                world,
                plane,
                *corner,
                &format!("\"form\":\"changed\",\"since\":\"{mark}\""),
            ),
            false,
        );
        assert_eq!(
            int_at(&held, "count"),
            0,
            "{label} changed {corner:?} with nothing written: {}",
            written(&held)
        );
    }
    let mut cells: Vec<([u32; 3], u32)> = (0..40)
        .map(|_| {
            let at = [0, 1, 2].map(|axis| u32::try_from(rng.below(u64::from(size[axis]))).unwrap());
            (at, if rng.below(3) == 0 { 0 } else { rng.word() | 1 })
        })
        .collect();
    cells.push(([4, 6, 3], 0));
    cells.push(([5, 6, 3], 11));
    write(server, world, plane, &cells);
    let now = dense_plane(server, world, size, plane);
    for (corner, mark) in [whole, inner].iter().zip(&marks) {
        let held = result(
            &asked(
                server,
                world,
                plane,
                *corner,
                &format!("\"form\":\"changed\",\"since\":\"{mark}\""),
            ),
            false,
        );
        let wanted = wanted_changed(&was, &now, size, corner.0, corner.1);
        assert!(!wanted.is_empty(), "{label} {corner:?}");
        assert!(
            listed_cells(&held) == wanted,
            "{label} changed after writes {corner:?}: {} listed, {} wanted",
            listed_cells(&held).len(),
            wanted.len()
        );
    }
    listed
}

#[test]
fn r6_uniform_bricks_of_a_sparse_world_read_their_word_on_every_plane() {
    let Some(host) = device("world read uniform bricks") else {
        return;
    };
    let size = [32_u32, 32, 32];
    let mut server = Server::new(Some(&host));
    let world = int_at(
        &ok(
            &mut server,
            "grit_world_create",
            &format!(
                "{{\"law\":{},\"params\":[],\"wide\":32,\"deep\":32,\"tall\":32,\"sparse\":true,\"budget_ms\":{LONG}}}",
                quoted(KEEP)
            ),
        ),
        "world",
    );
    write(&mut server, world, "", &block([0, 0, 0], 16, 7));
    write(
        &mut server,
        world,
        "",
        &block([16, 16, 16], 16, 0x8000_0001),
    );
    write(
        &mut server,
        world,
        "",
        &[([20, 3, 30], 5), ([31, 31, 0], 9)],
    );
    let busy = stepped_held(&mut server, world, 1);
    let quiet = stepped_held(&mut server, world, 4);
    println!("R6 keep: pool held {busy} after 1 frame, {quiet} after 5");
    assert!(
        quiet < busy,
        "the uniform blocks were never handed back to the table: {quiet} of {busy}"
    );
    let mut rng = Rng(0x66);
    let listed = plane_checked(&mut server, world, size, "", &mut rng, "keep");
    assert!(listed > 2 * 4096, "{listed}");

    let mut server = Server::new(Some(&host));
    let world = int_at(
        &ok(
            &mut server,
            "grit_world_create",
            &format!(
                "{{\"law\":{},\"params\":[],\"wide\":32,\"deep\":32,\"tall\":32,\"sparse\":true,\"budget_ms\":{LONG}}}",
                quoted(&held_law(3))
            ),
        ),
        "world",
    );
    for (plane, from, word) in [
        (0_u32, [0, 0, 0], 3_u32),
        (1, [0, 0, 0], 5),
        (2, [0, 0, 0], 9),
        (1, [16, 0, 16], 0xFFFF_FFFF),
        (2, [0, 16, 16], 6),
    ] {
        write(
            &mut server,
            world,
            &format!("\"plane\":{plane},"),
            &block(from, 16, word),
        );
    }
    for plane in 0..3_u32 {
        write(
            &mut server,
            world,
            &format!("\"plane\":{plane},"),
            &[([17 + plane, 2, 29], 40 + plane)],
        );
    }
    let busy = stepped_held(&mut server, world, 1);
    let quiet = stepped_held(&mut server, world, 4);
    println!("R6 held3: pool held {busy} after 1 frame, {quiet} after 5");
    assert!(quiet < busy, "{quiet} of {busy}");
    for plane in 0..3_u32 {
        let label = format!("held3 plane {plane}");
        let listed = plane_checked(
            &mut server,
            world,
            size,
            &format!("\"plane\":{plane},"),
            &mut rng,
            &label,
        );
        assert!(listed > 4096, "{label}: {listed}");
    }
}

#[test]
fn r8_mark_false_reads_as_no_mark_on_every_form() {
    let Some(host) = device("world read mark false") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let world = created(&mut server, &LIFE);
    let mut rng = Rng(0x88);
    let soup = seeded(&LIFE, &mut rng, 300);
    write(&mut server, world, CURRENT, &soup);
    let (mark, _) = set_mark(&mut server, world);
    let box_of = corners([2, 3, 0], [20, 9, 1]);
    for (form, plane) in [
        ("", CURRENT),
        ("\"form\":\"dense\",", CURRENT),
        ("\"form\":\"dense\",", "\"plane\":1,"),
        ("\"form\":\"set\",", CURRENT),
        ("\"form\":\"set\",", ""),
    ] {
        let plain = ok(
            &mut server,
            "grit_world_read",
            &format!("{{\"world\":{world},{plane}{form}{box_of}}}"),
        );
        let unmarked = ok(
            &mut server,
            "grit_world_read",
            &format!("{{\"world\":{world},{plane}{form}{box_of},\"mark\":false}}"),
        );
        assert_eq!(written(&unmarked), written(&plain), "{form} {plane}");
        assert!(unmarked.get("mark").is_none(), "{}", written(&unmarked));
    }
    step(&mut server, world, 1);
    let whole = corners(WHOLE.0, WHOLE.1);
    let changed = format!("\"form\":\"changed\",\"since\":\"{mark}\"");
    let plain = ok(
        &mut server,
        "grit_world_read",
        &format!("{{\"world\":{world},{CURRENT}{whole},{changed}}}"),
    );
    let unmarked = ok(
        &mut server,
        "grit_world_read",
        &format!("{{\"world\":{world},{CURRENT}{whole},{changed},\"mark\":false}}"),
    );
    assert_eq!(written(&unmarked), written(&plain));
    assert!(int_at(&plain, "count") > 0);
}

#[test]
fn r8_a_mark_of_another_plane_is_refused_at_the_field_the_read_names() {
    let Some(host) = device("world read mark field") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let world = created(&mut server, &LIFE);
    let mut rng = Rng(0x89);
    let soup = seeded(&LIFE, &mut rng, 300);
    write(&mut server, world, CURRENT, &soup);
    let whole = corners(WHOLE.0, WHOLE.1);
    let take = |server: &mut Server<'_>, plane: &str| {
        let held = ok(
            server,
            "grit_world_read",
            &format!("{{\"world\":{world},{plane}{whole},\"form\":\"set\",\"mark\":true}}"),
        );
        text_at(&held, "mark")
    };
    let on_current = take(&mut server, CURRENT);
    let on_zero = take(&mut server, "\"plane\":0,");
    let on_one = take(&mut server, "\"plane\":1,");
    let cases = [
        (&on_current, "\"plane\":0,", "$.plane"),
        (&on_current, "\"plane\":1,", "$.plane"),
        (&on_current, "", "$.plane"),
        (&on_current, "\"current\":false,", "$.plane"),
        (&on_zero, CURRENT, "$.current"),
        (&on_zero, "\"plane\":1,", "$.plane"),
        (&on_one, "", "$.plane"),
        (&on_one, CURRENT, "$.current"),
        (&on_one, "\"current\":true,\"plane\":0,", "$.current"),
    ];
    for (mark, plane, field) in cases {
        let held = no(
            &mut server,
            "grit_world_read",
            &format!(
                "{{\"world\":{world},{plane}{whole},\"form\":\"changed\",\"since\":\"{mark}\"}}"
            ),
        );
        refused_at(&held, "mark_unlike", field, &format!("{mark} {plane}"));
    }
    for (mark, plane) in [
        (&on_current, CURRENT),
        (&on_zero, "\"plane\":0,"),
        (&on_zero, ""),
        (&on_one, "\"plane\":1,"),
    ] {
        let held = ok(
            &mut server,
            "grit_world_read",
            &format!(
                "{{\"world\":{world},{plane}{whole},\"form\":\"changed\",\"since\":\"{mark}\"}}"
            ),
        );
        assert_eq!(int_at(&held, "count"), 0, "{mark} {plane}");
    }
}

#[test]
fn r9_a_set_read_past_its_limit_hints_the_dense_read() {
    let Some(host) = device("world read full box hint") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let size = [64, 32, 32];
    let world = int_at(
        &ok(
            &mut server,
            "grit_world_create",
            &format!(
                "{{\"law\":{},\"params\":[],\"wide\":64,\"deep\":32,\"tall\":32,\"budget_ms\":{LONG}}}",
                quoted(KEEP)
            ),
        ),
        "world",
    );
    filled(&mut server, world, size, SET_MOST);
    let whole = ([0, 0, 0], [63, 31, 31]);
    let held = result(
        &asked(
            &mut server,
            world,
            "",
            whole,
            "\"form\":\"set\",\"mark\":true",
        ),
        false,
    );
    let mark = text_at(&held, "mark");
    write(&mut server, world, "", &[(place_of(size, 1), 3)]);
    let told = ok(&mut server, "grit_describe", "{}");
    let generic = told
        .get("codes")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .find(|one| one.get("code").and_then(Json::text) == Some("too_many"))
        .map(|one| text_at(one, "hint"))
        .unwrap();
    for (form, field) in [
        ("\"form\":\"set\"".to_owned(), "$.to"),
        (
            format!("\"form\":\"changed\",\"since\":\"{mark}\",\"mark\":true"),
            "$.mark",
        ),
    ] {
        let held = result(&asked(&mut server, world, "", whole, &form), true);
        refused_at(&held, "too_many", field, &form);
        let hint = text_at(&held, "hint");
        assert!(hint.contains("form dense"), "{form}: {hint}");
        assert_ne!(hint, generic, "{form}");
    }
    let held = result(
        &asked(
            &mut server,
            world,
            "",
            ([0, 0, 0], [63, 31, 2]),
            "\"form\":\"dense\"",
        ),
        true,
    );
    refused_at(&held, "too_many", "$.to", "dense over");
    assert_eq!(text_at(&held, "hint"), generic);
}

#[cfg(feature = "faults")]
mod lost {
    use super::*;

    #[test]
    fn r7_a_mark_taken_before_a_lost_device_is_refused_by_name_after_the_rebuild() {
        let Some(host) = device("world read lost marks") else {
            return;
        };
        let mut server = Server::new(Some(&host));
        let world = created(&mut server, &LIFE);
        let mut rng = Rng(0x77);
        let soup = seeded(&LIFE, &mut rng, 400);
        write(&mut server, world, CURRENT, &soup);
        let (mark, was) = set_mark(&mut server, world);
        step(&mut server, world, 3);
        changed_is(&mut server, world, &mark, &was, "", "before the loss");
        let before = dense_plane(&mut server, world, LIFE.size, CURRENT);
        server.faults().lock().unwrap().wedge_before_call = true;
        let _ = server.heard(&line("grit_describe", "{}"));
        assert!(server.lost(), "setup: the device was not lost");
        let book = server.into_book();
        drop(host);
        let host = device("world read lost marks again").unwrap();
        let (mut server, _) = Server::resumed(Some(&host), book);
        let changed = format!("\"form\":\"changed\",\"since\":\"{mark}\"");
        let held = result(&asked(&mut server, world, CURRENT, WHOLE, &changed), true);
        refused_at(&held, "mark_unknown", "$.since", "after the rebuild");
        let hint = text_at(&held, "hint");
        assert!(hint.contains("lost device"), "{hint}");
        let again = asked(&mut server, world, CURRENT, WHOLE, &changed);
        assert!(
            written(&again).contains("a rebuild after a lost device"),
            "{}",
            written(&again)
        );
        let rebuilt = dense_plane(&mut server, world, LIFE.size, CURRENT);
        assert_eq!(rebuilt, before, "the rebuild replays the world as it was");
        let (fresh, words) = set_mark(&mut server, world);
        step(&mut server, world, 2);
        let (_, now) = changed_is(&mut server, world, &fresh, &words, "", "new mark");
        assert_ne!(now, words);
    }
}
