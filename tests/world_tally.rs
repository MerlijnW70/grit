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

fn no(server: &mut Server<'_>, name: &str, arguments: &str) -> Json {
    result(&server.heard(&line(name, arguments)).unwrap(), true)
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
            "{{\"size\":[16,16,1],\"input\":[[{}]],\"output\":[[{}]]}}",
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
        "keep-sparse" | "keep-dense" => {
            format!("{{\"law\":{},\"params\":[],{shape}}}", quoted(KEEP))
        }
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

fn step(server: &mut Server<'_>, world: i64, frames: u32) -> Json {
    ok(
        server,
        "grit_world_step",
        &format!("{{\"world\":{world},\"frames\":{frames},\"budget_ms\":{LONG}}}"),
    )
}

fn corners(from: [u32; 3], to: [u32; 3]) -> String {
    format!(
        "\"from\":[{},{},{}],\"to\":[{},{},{}]",
        from[0], from[1], from[2], to[0], to[1], to[2]
    )
}

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
                "{{\"world\":{world},{plane}{}}}",
                corners([0, 0, z], [size[0] - 1, size[1] - 1, top])
            ),
        );
        for cell in got.get("cells").and_then(Json::list).unwrap() {
            out.push(u32::try_from(cell.int().unwrap()).unwrap());
        }
        z = top + 1;
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Record {
    bit: u32,
    count: u64,
    least: Option<[u32; 3]>,
    most: Option<[u32; 3]>,
}

fn host_tally(
    words: &[u32],
    size: [u32; 3],
    (from, to): ([u32; 3], [u32; 3]),
    bits: &[u32],
) -> Vec<Record> {
    let mut sorted = bits.to_vec();
    sorted.sort_unstable();
    sorted
        .iter()
        .map(|&bit| {
            let mut record = Record {
                bit,
                count: 0,
                least: None,
                most: None,
            };
            for z in from[2]..=to[2] {
                for y in from[1]..=to[1] {
                    for x in from[0]..=to[0] {
                        let word = words[((z * size[1] + y) * size[0] + x) as usize];
                        if word >> bit & 1 == 0 {
                            continue;
                        }
                        let at = [x, y, z];
                        record.count += 1;
                        let least = record.least.get_or_insert(at);
                        let most = record.most.get_or_insert(at);
                        for axis in 0..3 {
                            least[axis] = least[axis].min(at[axis]);
                            most[axis] = most[axis].max(at[axis]);
                        }
                    }
                }
            }
            record
        })
        .collect()
}

fn corner_of(held: &Json, key: &str) -> Option<[u32; 3]> {
    held.get(key).map(|corner| {
        let parts: Vec<u32> = corner
            .list()
            .unwrap()
            .iter()
            .map(|one| u32::try_from(one.int().unwrap()).unwrap())
            .collect();
        <[u32; 3]>::try_from(parts.as_slice()).unwrap()
    })
}

fn answered(held: &Json) -> Vec<Record> {
    assert_eq!(text_at(held, "form"), "tally", "{}", written(held));
    held.get("bits")
        .and_then(Json::list)
        .unwrap_or_else(|| panic!("no bits in {}", written(held)))
        .iter()
        .map(|one| Record {
            bit: u32::try_from(int_at(one, "bit")).unwrap(),
            count: u64::try_from(int_at(one, "count")).unwrap(),
            least: corner_of(one, "min"),
            most: corner_of(one, "max"),
        })
        .collect()
}

fn tally_text(bits: Option<&[u32]>) -> String {
    match bits {
        None => "\"form\":\"tally\"".to_owned(),
        Some(bits) => {
            let shown: Vec<String> = bits.iter().map(u32::to_string).collect();
            format!("\"form\":\"tally\",\"bits\":[{}]", shown.join(","))
        }
    }
}

fn tallied(
    server: &mut Server<'_>,
    world: i64,
    plane: &str,
    corner: ([u32; 3], [u32; 3]),
    bits: Option<&[u32]>,
) -> Json {
    ok(
        server,
        "grit_world_read",
        &format!(
            "{{\"world\":{world},{plane}{},{}}}",
            corners(corner.0, corner.1),
            tally_text(bits)
        ),
    )
}

const ALL: [u32; 32] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
    26, 27, 28, 29, 30, 31,
];

const BIT_SETS: [Option<&[u32]>; 5] = [
    None,
    Some(&[0]),
    Some(&[31]),
    Some(&[0, 5, 31]),
    Some(&[7, 3]),
];

fn boxes(size: [u32; 3]) -> Vec<([u32; 3], [u32; 3])> {
    let last = size.map(|side| side - 1);
    vec![
        ([0, 0, 0], last),
        ([1, 2, 0], [size[0] - 2, size[1] - 3, size[2] - 1]),
        (last, last),
        (
            [0, size[1] / 2, size[2] / 2],
            [last[0], size[1] / 2, size[2] / 2],
        ),
        ([size[0] / 2, 0, 1], last),
    ]
}

#[derive(Default, Debug)]
struct Seen {
    asked: usize,
    counted: u64,
    top: u64,
}

fn plane_tallied(
    seen: &mut Seen,
    server: &mut Server<'_>,
    world: i64,
    size: [u32; 3],
    plane: &str,
    label: &str,
) {
    let words = dense_plane(server, world, size, plane);
    for corner in boxes(size) {
        for bits in BIT_SETS {
            let held = tallied(server, world, plane, corner, bits);
            let got = answered(&held);
            let wanted = host_tally(&words, size, corner, bits.unwrap_or(&ALL));
            assert!(
                got == wanted,
                "{label} {plane} {corner:?} {bits:?}:\n got {got:?}\nwant {wanted:?}"
            );
            assert_eq!(corner_of(&held, "from"), Some(corner.0), "{label}");
            assert_eq!(corner_of(&held, "to"), Some(corner.1), "{label}");
            assert_eq!(
                held.get("plane").is_some(),
                plane.contains("current"),
                "{label}"
            );
            seen.asked += 1;
            seen.counted += got.iter().map(|record| record.count).sum::<u64>();
            seen.top += got
                .iter()
                .filter(|record| record.bit == 31)
                .map(|record| record.count)
                .sum::<u64>();
        }
    }
}

fn references(kind: &Kind) -> Vec<String> {
    let mut out: Vec<String> = (0..kind.planes)
        .map(|plane| format!("\"plane\":{plane},"))
        .collect();
    out.push(String::new());
    if kind.iterating {
        out.push("\"current\":true,".to_owned());
    }
    out
}

const EXACT: [Kind; 7] = [
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
    Kind {
        name: "keep-dense",
        size: [64, 32, 16],
        planes: 1,
        mask: 0xFFFF_FFFF,
        sparse: false,
        iterating: false,
    },
];

#[test]
fn t1_tallies_equal_the_host_count_of_the_dense_read() {
    let Some(host) = device("world tally exact") else {
        return;
    };
    for kind in &EXACT {
        let mut server = Server::new(Some(&host));
        let world = created(&mut server, kind);
        let mut rng = Rng(0x7A11 ^ u64::from(kind.size[0] * kind.size[2]));
        let plane = if kind.iterating {
            "\"current\":true,"
        } else {
            ""
        };
        let mut seen = Seen::default();
        let count = (kind.size.iter().product::<u32>() / 6) as usize;
        write(&mut server, world, plane, &seeded(kind, &mut rng, count));
        step(&mut server, world, 3);
        for reference in references(kind) {
            plane_tallied(
                &mut seen,
                &mut server,
                world,
                kind.size,
                &reference,
                kind.name,
            );
        }
        let last = kind.size.map(|side| side - 1);
        let mut more = seeded(kind, &mut rng, count / 4);
        for cell in more.iter_mut().take(count / 12) {
            cell.1 = 0;
        }
        more.push((last, kind.mask));
        more.push(([0, 0, 0], 0));
        write(&mut server, world, plane, &more);
        for reference in references(kind) {
            plane_tallied(
                &mut seen,
                &mut server,
                world,
                kind.size,
                &reference,
                kind.name,
            );
        }
        step(&mut server, world, 5);
        for reference in references(kind) {
            plane_tallied(
                &mut seen,
                &mut server,
                world,
                kind.size,
                &reference,
                kind.name,
            );
        }
        println!("T1 {}: {seen:?}", kind.name);
        assert!(seen.counted > 0, "{}", kind.name);
        if kind.mask >> 31 == 1 {
            assert!(seen.top > 0, "{}: no cell held bit 31", kind.name);
        }
    }
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

fn held_law(planes: u32) -> String {
    let cells: Vec<String> = (0..8).map(|k| format!("[\"at\",\"cells\",{k}]")).collect();
    format!(
        "{{\"law\":1,\"name\":\"held{planes}\",\"params\":0,\"channels\":{planes},\"halo\":0,\"tally\":{{\"counts\":1,\"weighted\":false}},\"consts\":{{}},\"defs\":{{}},\"step\":[\"vec\",[\"vec\",{}],[\"vec\",0]]}}",
        cells.join(",")
    )
}

fn sparse_world(server: &mut Server<'_>, law: &str) -> i64 {
    int_at(
        &ok(
            server,
            "grit_world_create",
            &format!(
                "{{\"law\":{},\"params\":[],\"wide\":32,\"deep\":32,\"tall\":32,\"sparse\":true,\"budget_ms\":{LONG}}}",
                quoted(law)
            ),
        ),
        "world",
    )
}

#[test]
fn t1_uniform_bricks_of_a_sparse_world_are_tallied_as_their_word_on_every_plane() {
    let Some(host) = device("world tally uniform bricks") else {
        return;
    };
    let size = [32_u32, 32, 32];
    let mut server = Server::new(Some(&host));
    let world = sparse_world(&mut server, KEEP);
    write(&mut server, world, "", &block([0, 0, 0], 16, 6));
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
    let busy = pool_held(&step(&mut server, world, 1));
    let quiet = pool_held(&step(&mut server, world, 4));
    println!("T1 keep uniform: pool held {busy} after 1 frame, {quiet} after 5");
    assert!(quiet < busy, "{quiet} of {busy}");
    let held = tallied(
        &mut server,
        world,
        "",
        ([0, 0, 0], [31, 31, 31]),
        Some(&[31, 2]),
    );
    let got = answered(&held);
    assert_eq!(
        got,
        vec![
            Record {
                bit: 2,
                count: 4096 + 1,
                least: Some([0, 0, 0]),
                most: Some([20, 15, 30]),
            },
            Record {
                bit: 31,
                count: 4096,
                least: Some([16, 16, 16]),
                most: Some([31, 31, 31]),
            },
        ],
        "{}",
        written(&held)
    );
    let mut seen = Seen::default();
    plane_tallied(&mut seen, &mut server, world, size, "", "keep uniform");

    let mut server = Server::new(Some(&host));
    let world = sparse_world(&mut server, &held_law(3));
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
    let busy = pool_held(&step(&mut server, world, 1));
    let quiet = pool_held(&step(&mut server, world, 4));
    println!("T1 held3 uniform: pool held {busy} after 1 frame, {quiet} after 5");
    assert!(quiet < busy, "{quiet} of {busy}");
    let held = tallied(
        &mut server,
        world,
        "\"plane\":1,",
        ([0, 0, 0], [31, 31, 31]),
        Some(&[31]),
    );
    assert_eq!(
        answered(&held),
        vec![Record {
            bit: 31,
            count: 4095,
            least: Some([16, 0, 16]),
            most: Some([31, 15, 31]),
        }],
        "{}",
        written(&held)
    );
    for plane in 0..3_u32 {
        plane_tallied(
            &mut seen,
            &mut server,
            world,
            size,
            &format!("\"plane\":{plane},"),
            "held3 uniform",
        );
    }
    println!("T1 uniform: {seen:?}");
}

#[test]
fn t1_a_plane_past_one_dispatch_is_tallied_to_its_last_cell() {
    let Some(host) = device("world tally wide plane") else {
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
        held.insert([127, 127, 127], 0xFFFF_FFFF);
        held.insert([0, 0, 0], 0x0000_0001);
        while held.len() < 400 {
            let at = [0, 1, 2].map(|axis| u32::try_from(rng.below(u64::from(size[axis]))).unwrap());
            held.insert(at, rng.word() & 0x7FFF_FFFE);
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
            for bits in [None, Some(&[31_u32][..])] {
                let got = answered(&tallied(&mut server, world, "", corner, bits));
                let wanted = host_tally(&words, size, corner, bits.unwrap_or(&ALL));
                assert!(got == wanted, "{sparse} {corner:?} {bits:?}");
                let top = got.iter().find(|record| record.bit == 31).unwrap();
                assert_eq!(top.count, 1, "{sparse} {corner:?}");
                assert_eq!(top.most, Some([127, 127, 127]), "{sparse} {corner:?}");
            }
        }
    }
}

#[test]
fn t1_an_iterating_world_is_tallied_after_each_call_of_frames() {
    let Some(host) = device("world tally iterating") else {
        return;
    };
    let kind = Kind {
        name: "life-dense-2d",
        size: [64, 64, 2],
        planes: 2,
        mask: 1,
        sparse: false,
        iterating: true,
    };
    let mut server = Server::new(Some(&host));
    let world = created(&mut server, &kind);
    let mut rng = Rng(0x1FE);
    let mut cells = Vec::new();
    for y in 20..44 {
        for x in 20..44 {
            if rng.below(100) < 35 {
                cells.push(([x, y, 0], 1));
            }
        }
    }
    write(&mut server, world, "\"current\":true,", &cells);
    let whole = ([0, 0, 0], [63, 63, 1]);
    let mut seen = Seen::default();
    for round in 0..6 {
        let answer = step(&mut server, world, 4);
        assert_eq!(int_at(&answer, "current"), 0, "round {round}");
        let words = dense_plane(&mut server, world, kind.size, "\"current\":true,");
        let held = tallied(&mut server, world, "\"current\":true,", whole, Some(&[0]));
        assert_eq!(int_at(&held, "plane"), 0);
        let got = answered(&held);
        assert_eq!(
            got,
            host_tally(&words, kind.size, whole, &[0]),
            "round {round}"
        );
        seen.counted += got[0].count;
        for reference in references(&kind) {
            plane_tallied(
                &mut seen,
                &mut server,
                world,
                kind.size,
                &reference,
                "life 64",
            );
        }
    }
    println!("T1 iterating: {seen:?}");
    assert!(seen.counted > 0);
}

fn refused_at(held: &Json, code: &str, field: &str, label: &str) {
    assert_eq!(text_at(held, "code"), code, "{label}: {}", written(held));
    assert_eq!(text_at(held, "field"), field, "{label}: {}", written(held));
}

#[test]
fn t4_refusals_by_code_and_field() {
    let Some(host) = device("world tally refusals") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let kind = &EXACT[5];
    let world = created(&mut server, kind);
    let whole = corners([0, 0, 0], [31, 15, 15]);
    let read = |arguments: &str| format!("{{\"world\":{world},{arguments}}}");
    for (arguments, code, field) in [
        (
            format!("{whole},\"form\":\"tally\",\"bits\":[32]"),
            "bit_past",
            "$.bits[0]",
        ),
        (
            format!("{whole},\"form\":\"tally\",\"bits\":[0,4294967295]"),
            "bit_past",
            "$.bits[1]",
        ),
        (
            format!("{whole},\"form\":\"tally\",\"bits\":[3,9,3]"),
            "malformed",
            "$.bits[2]",
        ),
        (
            format!("{whole},\"form\":\"tally\",\"bits\":[]"),
            "malformed",
            "$.bits",
        ),
        (
            format!("{whole},\"form\":\"tally\",\"bits\":[-1]"),
            "malformed",
            "$.bits[0]",
        ),
        (
            format!("{whole},\"form\":\"tally\",\"bits\":\"all\""),
            "malformed",
            "$.bits",
        ),
        (
            format!(
                "{whole},\"form\":\"tally\",\"bits\":[{}]",
                (0..33)
                    .map(|bit| (bit % 32).to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            "too_many",
            "$.bits",
        ),
        (
            format!("{whole},\"form\":\"set\",\"bits\":[0]"),
            "read_form",
            "$.bits",
        ),
        (format!("{whole},\"bits\":[0]"), "read_form", "$.bits"),
        (
            format!("{whole},\"form\":\"changed\",\"since\":\"00\",\"bits\":[0]"),
            "read_form",
            "$.bits",
        ),
        (
            format!("{whole},\"form\":\"tally\",\"since\":\"0123456789abcdef\""),
            "read_form",
            "$.since",
        ),
        (
            format!("{whole},\"form\":\"tally\",\"mark\":true"),
            "read_form",
            "$.mark",
        ),
        (
            format!("{},\"form\":\"tally\"", corners([0, 0, 0], [32, 15, 15])),
            "outside",
            "$.to",
        ),
        (
            format!("{},\"form\":\"tally\"", corners([0, 16, 0], [31, 15, 15])),
            "outside",
            "$.from",
        ),
        (
            format!("{},\"form\":\"tally\"", corners([5, 0, 0], [4, 15, 15])),
            "reversed",
            "$.to",
        ),
        (
            format!("{whole},\"form\":\"tally\",\"current\":true"),
            "not_iterating",
            "$.current",
        ),
        (
            format!("{whole},\"form\":\"tally\",\"plane\":1"),
            "outside",
            "$.plane",
        ),
        (
            format!("{whole},\"form\":\"tally\",\"bitz\":[0]"),
            "unknown_key",
            "$.bitz",
        ),
    ] {
        let held = no(&mut server, "grit_world_read", &read(&arguments));
        refused_at(&held, code, field, &arguments);
        if code == "read_form" {
            assert!(
                text_at(&held, "hint").contains("tally"),
                "{}",
                written(&held)
            );
        }
    }
    let held = ok(
        &mut server,
        "grit_world_read",
        &read(&format!(
            "{whole},\"form\":\"tally\",\"mark\":false,\"bits\":[31,0]"
        )),
    );
    assert_eq!(
        answered(&held)
            .iter()
            .map(|record| record.bit)
            .collect::<Vec<_>>(),
        vec![0, 31]
    );
    let life = &EXACT[4];
    let iterating = created(&mut server, life);
    let held = no(
        &mut server,
        "grit_world_read",
        &format!(
            "{{\"world\":{iterating},{},\"form\":\"tally\",\"current\":true,\"plane\":1}}",
            corners([0, 0, 0], [31, 31, 1])
        ),
    );
    refused_at(&held, "plane_or_current", "$.plane", "plane and current");
    let described = ok(&mut server, "grit_describe", "{}");
    let limits = described.get("limits").unwrap();
    assert!(text_at(limits, "tally_read").contains("960 bytes"));
}

fn existing_answers(server: &mut Server<'_>, kind: &Kind, seed: u64) -> Vec<String> {
    let world = created(server, kind);
    let mut rng = Rng(seed);
    let plane = if kind.iterating {
        "\"current\":true,"
    } else {
        ""
    };
    write(server, world, plane, &seeded(kind, &mut rng, 500));
    step(server, world, 2);
    let [wide, deep, tall] = kind.size;
    let last = [wide - 1, deep - 1, tall - 1];
    let mut answers = Vec::new();
    for reference in references(kind) {
        for (from, to) in [
            ([0, 0, 0], [7, 7, 1]),
            ([wide - 4, deep - 3, tall - 2], last),
        ] {
            let got = ok(
                server,
                "grit_world_read",
                &format!("{{\"world\":{world},{reference}{}}}", corners(from, to)),
            );
            answers.push(written(&got));
        }
        let got = ok(
            server,
            "grit_world_read",
            &format!(
                "{{\"world\":{world},{reference}{},\"form\":\"set\",\"mark\":true}}",
                corners([0, 0, 0], last)
            ),
        );
        answers.push(written(&got));
        let mark = text_at(&got, "mark");
        let got = ok(
            server,
            "grit_world_slice",
            &format!("{{\"world\":{world},{reference}\"axis\":\"y\",\"at\":1,\"as\":\"words\"}}"),
        );
        answers.push(written(&got));
        write(server, world, plane, &seeded(kind, &mut rng, 40));
        let got = ok(
            server,
            "grit_world_read",
            &format!(
                "{{\"world\":{world},{reference}{},\"form\":\"changed\",\"since\":\"{mark}\"}}",
                corners([0, 0, 0], last)
            ),
        );
        answers.push(written(&got));
    }
    let got = no(
        server,
        "grit_world_read",
        &format!(
            "{{\"world\":{world},{},\"form\":\"changed\"}}",
            corners([0, 0, 0], last)
        ),
    );
    answers.push(written(&got));
    answers
}

const PINNED: [Kind; 4] = [
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
        size: [32, 16, 16],
        planes: 1,
        mask: 0xFFFF_FFFF,
        sparse: false,
        iterating: false,
    },
    Kind {
        name: "sand-rule",
        size: [16, 16, 8],
        planes: 1,
        mask: 0xFF,
        sparse: false,
        iterating: false,
    },
];

const RECORDED: [(&str, &str); 4] = [
    (
        "mix-sparse",
        "c342e865ff9efd9bdd3032a1fc52cec8a8ad5b442780b1f8487edf9bbeae63af",
    ),
    (
        "life-dense-2d",
        "5af2777e17fbda168940942202469ad5ec7177d7abc7a3e825e8d03b4119212d",
    ),
    (
        "mix-dense-3d",
        "e9732a0a79b039ca95548e4b35922bc15f27f2824df91839c986f67cfc487f72",
    ),
    (
        "sand-rule",
        "8d9c30e0e796cc148b209cfe388a8a3e1ab8f138c5d247dd090ec95aa84083c8",
    ),
];

#[test]
fn t3_existing_reads_answer_as_on_the_recorded_base() {
    let Some(host) = device("world tally recorded") else {
        return;
    };
    let mut wrong = Vec::new();
    for (kind, (name, want)) in PINNED.iter().zip(RECORDED) {
        assert_eq!(kind.name, name);
        let mut server = Server::new(Some(&host));
        let answers = existing_answers(&mut server, kind, 0x7A1 ^ u64::from(kind.size[2]));
        let got = hex(&sha256(answers.join("\n").as_bytes()));
        println!("RECORD {name} {got}");
        if got != want {
            wrong.push(format!("{name}: {got} against {want}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[cfg(feature = "faults")]
mod traffic {
    use super::*;

    fn moved(server: &mut Server<'_>, world: i64, arguments: &str) -> (Json, grit::Traffic) {
        let number = u32::try_from(world).unwrap();
        let before = server.traffic(number).unwrap();
        let held = ok(
            server,
            "grit_world_read",
            &format!("{{\"world\":{world},{arguments}}}"),
        );
        (held, server.traffic(number).unwrap().since(before))
    }

    #[test]
    fn t2_a_tally_moves_the_same_960_bytes_whatever_the_box_or_bits() {
        let Some(host) = device("world tally traffic") else {
            return;
        };
        for sparse in [false, true] {
            let mut server = Server::new(Some(&host));
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
            let mut rng = Rng(0x960 ^ u64::from(sparse));
            let cells: Vec<([u32; 3], u32)> = (0..3000)
                .map(|_| {
                    let at = [0, 1, 2].map(|_| u32::try_from(rng.below(64)).unwrap());
                    (at, rng.word() | 0x8000_0001)
                })
                .collect();
            write(&mut server, world, "", &cells);
            let mut seen = Vec::new();
            for corner in [
                ([0, 0, 0], [63, 63, 63]),
                ([5, 5, 5], [5, 5, 5]),
                ([0, 0, 0], [63, 0, 0]),
                ([10, 20, 30], [50, 60, 63]),
            ] {
                for bits in [None, Some(&[31_u32][..]), Some(&[0, 1, 2][..])] {
                    let (held, moved) = moved(
                        &mut server,
                        world,
                        &format!("{},{}", corners(corner.0, corner.1), tally_text(bits)),
                    );
                    assert_eq!(
                        moved.downloaded, 960,
                        "{sparse} {corner:?} {bits:?}: {moved:?}"
                    );
                    assert!(moved.downloaded <= 32 * 32, "{moved:?}");
                    assert_eq!(moved.submits, 2, "{sparse} {corner:?} {bits:?}: {moved:?}");
                    seen.push(
                        answered(&held)
                            .iter()
                            .map(|record| record.count)
                            .sum::<u64>(),
                    );
                }
            }
            let mut dense = 0_u64;
            for z in 0..64 {
                let (_, one) = moved(&mut server, world, &corners([0, 0, z], [63, 63, z]));
                dense += one.downloaded;
            }
            println!(
                "T2 sparse {sparse}: tally 960 bytes a call, dense plane {dense} bytes, counts {seen:?}"
            );
            assert!(seen[0] > 3000, "{seen:?}");
            assert!(960 * 100 < dense, "{dense}");
        }
    }
}
