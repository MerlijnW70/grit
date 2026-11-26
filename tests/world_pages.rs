#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;
#[path = "world_pages/pinned.rs"]
mod pinned;
#[path = "world_pages/served.rs"]
mod served;

use common::device;

use grit::serve3::Server;
use grit::wire::{Json, written};

use served::{
    Rng, cells_text, code_of, dense_box, encoded, int_at, no, ok, packed, quoted, read_page,
    words_hash,
};

const MAJORITY: &str = include_str!("world_write/majority.law.json");
const MIX: &str = include_str!("law/mix.law.json");
const LONG: u32 = 600_000;

#[derive(Clone, Copy)]
enum Made {
    Data(u32),
    Law(&'static str, &'static str),
}

struct Case {
    label: &'static str,
    made: Made,
    size: [u32; 3],
    sparse: bool,
    absorb: Option<[u32; 3]>,
    plane: u32,
    from: [u32; 3],
    to: [u32; 3],
    steps: u32,
}

fn create_args(case: &Case) -> String {
    let made = match case.made {
        Made::Data(planes) => format!("\"planes\":{planes}"),
        Made::Law(law, params) => format!("\"law\":{},\"params\":{params}", quoted(law)),
    };
    let absorb = case
        .absorb
        .map(|cells| format!(",\"absorb\":[{},{},{}]", cells[0], cells[1], cells[2]))
        .unwrap_or_default();
    format!(
        "{{\"wide\":{},\"deep\":{},\"tall\":{},{made},\"sparse\":{}{absorb},\"budget_ms\":{LONG}}}",
        case.size[0], case.size[1], case.size[2], case.sparse
    )
}

fn box_cells(from: [u32; 3], to: [u32; 3]) -> Vec<[u32; 3]> {
    let mut points = Vec::new();
    for z in from[2]..=to[2] {
        for y in from[1]..=to[1] {
            for x in from[0]..=to[0] {
                points.push([x, y, z]);
            }
        }
    }
    points
}

fn fixture(rng: &mut Rng, cells: usize, bits: bool) -> Vec<u32> {
    const EDGE: [u32; 5] = [0xFFFF_FFFF, 0x8000_0000, 0x7FFF_FFFF, 1, 0];
    (0..cells)
        .map(|at| {
            if bits {
                u32::from(rng.below(1000) < 400)
            } else if at % 97 == 3 {
                EDGE[at % EDGE.len()]
            } else if rng.below(4) == 0 {
                0
            } else {
                rng.word()
            }
        })
        .collect()
}

fn write_long(server: &mut Server<'_>, world: i64, plane: u32, cells: &[([u32; 3], u32)]) {
    for chunk in cells.chunks(4096) {
        let got = ok(
            server,
            "grit_world_write",
            &format!(
                "{{\"world\":{world},\"plane\":{plane},\"cells\":{},\"budget_ms\":{LONG}}}",
                cells_text(chunk)
            ),
        );
        assert_eq!(int_at(&got, "written"), chunk.len() as i64);
    }
}

fn write_page(
    server: &mut Server<'_>,
    world: i64,
    plane: u32,
    (from, to): ([u32; 3], [u32; 3]),
    format: &str,
    words: &[u32],
) -> Json {
    ok(
        server,
        "grit_world_write",
        &format!(
            "{{\"world\":{world},\"plane\":{plane},\"form\":\"page\",\"from\":[{},{},{}],\"to\":[{},{},{}],\"format\":\"{format}\",\"data\":\"{}\",\"budget_ms\":{LONG}}}",
            from[0],
            from[1],
            from[2],
            to[0],
            to[1],
            to[2],
            encoded(&packed(format, words))
        ),
    )
}

fn whole_long(server: &mut Server<'_>, world: i64, size: [u32; 3], planes: u32) -> Vec<u32> {
    let mut words = Vec::new();
    for plane in 0..planes {
        words.extend(dense_box(
            server,
            world,
            plane,
            [0, 0, 0],
            size.map(|side| side - 1),
        ));
    }
    words
}

fn case_planes(case: &Case) -> u32 {
    match case.made {
        Made::Data(planes) => planes,
        Made::Law(law, _) => {
            if law.contains("\"channels\":2") {
                2
            } else {
                1
            }
        }
    }
}

fn run_case(server: &mut Server<'_>, case: &Case, format: &str, seed: u64) {
    let label = format!("{} {format}", case.label);
    let args = create_args(case);
    let paged = int_at(&ok(server, "grit_world_create", &args), "world");
    let long = int_at(&ok(server, "grit_world_create", &args), "world");
    let points = box_cells(case.from, case.to);
    assert_ne!(
        points.len() % 32,
        0,
        "{label}: the box must end in a partial word"
    );
    let mut rng = Rng(seed);
    let words = fixture(&mut rng, points.len(), format == "bits");
    let told = write_page(
        server,
        paged,
        case.plane,
        (case.from, case.to),
        format,
        &words,
    );
    assert_eq!(int_at(&told, "written"), points.len() as i64, "{label}");
    let cells: Vec<([u32; 3], u32)> = points.iter().copied().zip(words.iter().copied()).collect();
    write_long(server, long, case.plane, &cells);
    let planes = case_planes(case);
    for (when, frames) in [("written", 0), ("stepped", case.steps)] {
        if frames > 0 {
            for world in [paged, long] {
                ok(
                    server,
                    "grit_world_step",
                    &format!("{{\"world\":{world},\"frames\":{frames},\"budget_ms\":{LONG}}}"),
                );
            }
        }
        let by_page = whole_long(server, paged, case.size, planes);
        let by_cells = whole_long(server, long, case.size, planes);
        assert_eq!(
            words_hash(&by_page),
            words_hash(&by_cells),
            "{label} {when}: the paged world differs from the one written cell by cell"
        );
        let page_box = read_page(server, paged, case.plane, (case.from, case.to), "words");
        let long_box = dense_box(server, long, case.plane, case.from, case.to);
        assert_eq!(page_box, long_box, "{label} {when}: page read of the box");
        if frames == 0 {
            assert_eq!(page_box, words, "{label}: the box reads back as written");
            let as_bits = read_page(server, long, case.plane, (case.from, case.to), format);
            assert_eq!(
                as_bits, words,
                "{label}: a {format} page read of the long way"
            );
        }
    }
    for world in [paged, long] {
        ok(
            server,
            "grit_world_close",
            &format!("{{\"world\":{world}}}"),
        );
    }
}

fn sparse_box(host: &grit::Host, size: [u32; 3]) -> ([u32; 3], [u32; 3]) {
    let law = grit::law::Law::compile(&grit::serve3::page::data_law(1)).unwrap();
    let world = grit::branch::World::create_law(
        host,
        &law,
        &grit::branch::Made {
            rule: law.name(),
            params: &[],
            size,
            sparse: true,
            slots: None,
        },
    )
    .unwrap();
    let sides = world.pool().unwrap().sides();
    let from = sides.map(|side| side - 3);
    let to = [from[0] + 36, from[1] + 30, from[2] + 4];
    for axis in 0..3 {
        assert!(
            to[axis] < size[axis] && to[axis] / sides[axis] > from[axis] / sides[axis],
            "the box must cross a brick edge on axis {axis}: sides {sides:?}"
        );
    }
    (from, to)
}

#[test]
fn p1_a_page_equals_the_same_cells_written_and_read_the_long_way() {
    let Some(host) = device("world pages p1") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let (sparse_from, sparse_to) = sparse_box(&host, [64, 64, 64]);
    let cases = [
        Case {
            label: "data 2d dense",
            made: Made::Data(2),
            size: [64, 64, 2],
            sparse: false,
            absorb: None,
            plane: 1,
            from: [5, 9, 0],
            to: [51, 39, 1],
            steps: 0,
        },
        Case {
            label: "data 3d dense",
            made: Made::Data(1),
            size: [32, 32, 32],
            sparse: false,
            absorb: None,
            plane: 0,
            from: [3, 7, 1],
            to: [30, 24, 29],
            steps: 0,
        },
        Case {
            label: "data 3d sparse",
            made: Made::Data(2),
            size: [64, 64, 64],
            sparse: true,
            absorb: None,
            plane: 1,
            from: sparse_from,
            to: sparse_to,
            steps: 0,
        },
        Case {
            label: "law 2d dense wrapped",
            made: Made::Law(MAJORITY, "[]"),
            size: [64, 64, 2],
            sparse: false,
            absorb: None,
            plane: 0,
            from: [1, 2, 0],
            to: [62, 60, 0],
            steps: 3,
        },
        Case {
            label: "law 2d dense absorb",
            made: Made::Law(MAJORITY, "[]"),
            size: [64, 64, 2],
            sparse: false,
            absorb: Some([3, 3, 0]),
            plane: 0,
            from: [0, 1, 0],
            to: [62, 63, 1],
            steps: 3,
        },
        Case {
            label: "law 3d sparse",
            made: Made::Law(MIX, "[1,2]"),
            size: [64, 64, 64],
            sparse: true,
            absorb: None,
            plane: 0,
            from: sparse_from,
            to: sparse_to,
            steps: 2,
        },
        Case {
            label: "law 3d sparse absorb",
            made: Made::Law(MIX, "[1,2]"),
            size: [64, 64, 64],
            sparse: true,
            absorb: Some([2, 2, 2]),
            plane: 0,
            from: sparse_from,
            to: sparse_to,
            steps: 2,
        },
    ];
    for (at, case) in cases.iter().enumerate() {
        for (which, format) in ["bits", "words"].iter().enumerate() {
            run_case(
                &mut server,
                case,
                format,
                0x9A6E_0000 + (at * 2 + which) as u64,
            );
        }
    }
}

fn data_world(
    server: &mut Server<'_>,
    size: [u32; 3],
    planes: u32,
    pages: &[(u32, &str, &[u32])],
) -> i64 {
    let shown: Vec<String> = pages
        .iter()
        .map(|(plane, format, words)| {
            format!(
                "{{\"plane\":{plane},\"format\":\"{format}\",\"data\":\"{}\"}}",
                encoded(&packed(format, words))
            )
        })
        .collect();
    let pages = if shown.is_empty() {
        String::new()
    } else {
        format!(",\"pages\":[{}]", shown.join(","))
    };
    let got = ok(
        server,
        "grit_world_create",
        &format!(
            "{{\"wide\":{},\"deep\":{},\"tall\":{},\"planes\":{planes}{pages},\"budget_ms\":{LONG}}}",
            size[0], size[1], size[2]
        ),
    );
    assert_eq!(
        got.get("data"),
        Some(&Json::Bool(true)),
        "{}",
        written(&got)
    );
    int_at(&got, "world")
}

#[test]
fn p3_each_limit_is_refused_by_name_and_nothing_is_written() {
    let Some(host) = device("world pages p3") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let world = data_world(&mut server, [64, 64, 32], 2, &[]);
    let page = |from: [u32; 3], to: [u32; 3], format: &str, text: &str| {
        format!(
            "{{\"world\":{world},\"form\":\"page\",\"from\":[{},{},{}],\"to\":[{},{},{}],\"format\":\"{format}\",\"data\":\"{text}\"}}",
            from[0], from[1], from[2], to[0], to[1], to[2]
        )
    };
    let box_from = [1, 2, 3];
    let box_to = [10, 4, 5];
    let cells = 10 * 3 * 3;
    let ones = vec![1_u32; cells];
    let mut short = packed("words", &ones);
    short.truncate(short.len() - 4);
    let mut long = packed("words", &ones);
    long.extend([0, 0, 0, 0]);
    let mut past = packed("bits", &ones);
    let last = past.len() - 1;
    past[last] |= 0x80;
    let refusals = [
        (
            "short",
            page(box_from, box_to, "words", &encoded(&short)),
            "page_length",
            "$.data",
        ),
        (
            "long",
            page(box_from, box_to, "words", &encoded(&long)),
            "page_length",
            "$.data",
        ),
        (
            "bits past the box",
            page(box_from, box_to, "bits", &encoded(&past)),
            "page_length",
            "$.data",
        ),
        (
            "bits as words",
            page(box_from, box_to, "words", &encoded(&packed("bits", &ones))),
            "page_length",
            "$.data",
        ),
        (
            "outside",
            page([0, 0, 0], [64, 3, 3], "words", &encoded(&short)),
            "outside",
            "$.to",
        ),
        (
            "too big",
            page([0, 0, 0], [63, 63, 31], "words", ""),
            "page_limit",
            "$.to",
        ),
        (
            "not base64",
            page(box_from, box_to, "words", "%%%%"),
            "malformed",
            "$.data",
        ),
    ];
    for (label, args, code, field) in &refusals {
        let got = no(&mut server, "grit_world_write", args);
        assert_eq!(code_of(&got), (*code, *field), "{label}: {}", written(&got));
    }
    let untouched = read_page(&mut server, world, 0, ([0, 0, 0], [63, 63, 31]), "bits");
    assert!(
        untouched.iter().all(|word| *word == 0),
        "a refused page wrote cells"
    );
    let too_big = no(
        &mut server,
        "grit_world_read",
        &format!(
            "{{\"world\":{world},\"form\":\"page\",\"from\":[0,0,0],\"to\":[63,63,31],\"format\":\"words\"}}"
        ),
    );
    assert_eq!(code_of(&too_big), ("page_limit", "$.to"));
    let mut rng = Rng(3);
    let wide: Vec<u32> = (0..cells).map(|_| rng.word() | 2).collect();
    write_page(&mut server, world, 1, (box_from, box_to), "words", &wide);
    let mismatch = no(
        &mut server,
        "grit_world_read",
        &format!(
            "{{\"world\":{world},\"plane\":1,\"form\":\"page\",\"from\":[1,2,3],\"to\":[10,4,5],\"format\":\"bits\"}}"
        ),
    );
    assert_eq!(code_of(&mismatch), ("page_format", "$.format"));
    let outside = no(
        &mut server,
        "grit_world_read",
        &format!("{{\"world\":{world},\"form\":\"page\",\"from\":[0,0,32],\"to\":[1,1,1]}}"),
    );
    assert_eq!(code_of(&outside).0, "outside");
    let wrong_size = no(
        &mut server,
        "grit_world_create",
        &format!(
            "{{\"wide\":8,\"deep\":8,\"tall\":2,\"planes\":1,\"pages\":[{{\"plane\":0,\"format\":\"bits\",\"data\":\"{}\"}}]}}",
            encoded(&packed("bits", &[1; 96]))
        ),
    );
    assert_eq!(code_of(&wrong_size), ("page_length", "$.pages[0].data"));
    let lawless = [
        (
            "grit_world_step",
            format!("{{\"world\":{world},\"frames\":1}}"),
        ),
        (
            "grit_world_back",
            format!("{{\"world\":{world},\"frames\":1}}"),
        ),
        (
            "grit_world_params",
            format!("{{\"world\":{world},\"params\":[]}}"),
        ),
        (
            "grit_law_score",
            format!(
                "{{\"world\":{world},\"frames\":1,\"aim\":{{\"from\":[0,0,0],\"to\":[1,1,1],\"target\":[0,0,0,0,0,0,0,0]}}}}"
            ),
        ),
        (
            "grit_law_gradient",
            format!(
                "{{\"world\":{world},\"frames\":1,\"aim\":{{\"from\":[0,0,0],\"to\":[1,1,1],\"target\":[0,0,0,0,0,0,0,0]}}}}"
            ),
        ),
    ];
    for (tool, args) in &lawless {
        let got = no(&mut server, tool, args);
        assert_eq!(
            code_of(&got),
            ("no_law", "$.world"),
            "{tool}: {}",
            written(&got)
        );
    }
    let both = no(
        &mut server,
        "grit_world_create",
        &format!(
            "{{\"wide\":8,\"deep\":8,\"tall\":2,\"planes\":1,\"law\":{}}}",
            quoted(MIX)
        ),
    );
    assert_eq!(code_of(&both), ("rule_or_law", "$.planes"));
}

const RECORDED: [(&str, &str); 4] = [
    (
        "dense-session",
        "8df35f798ea208088b38f794c3511c74925385750948767774e8870c8d2c1e00",
    ),
    (
        "sparse-session",
        "c1c752613fd348d40f1843c581c7b986123507a141c7021dc4ed176a3c26b5bd",
    ),
    (
        "learn-session",
        "c8fd56520ae5187e59560317a0a117f974c4e27cdf1483bc960558d530f9314b",
    ),
    (
        "describe",
        "bcf10acbd0c0753b3904d535490ce7b5fc911c775583b2292a3939d505776c09",
    ),
];

#[test]
fn p4_answers_of_the_base_forms_keep_their_bytes() {
    let Some(host) = device("world pages p4") else {
        return;
    };
    let got = [
        (
            "dense-session",
            pinned::dense_session(&mut Server::new(Some(&host))),
        ),
        (
            "sparse-session",
            pinned::sparse_session(&mut Server::new(Some(&host))),
        ),
        (
            "learn-session",
            pinned::learn_session(&mut Server::new(Some(&host))),
        ),
        ("describe", pinned::described(&mut Server::new(None))),
    ];
    for (name, hash) in &got {
        println!("RECORD {name} {hash}");
    }
    for ((name, hash), (pinned_name, want)) in got.iter().zip(RECORDED) {
        assert_eq!(*name, pinned_name);
        assert_eq!(hash, want, "{name}: the answers differ from the base's");
    }
}

fn stacked(pairs: &pinned::Pairs) -> (Vec<u32>, Vec<u32>) {
    let tall = (pairs.inputs.len() as u32).next_power_of_two().max(2) as usize;
    let layer = pairs.inputs[0].len();
    let mut inputs = vec![0; layer * tall];
    let mut outputs = vec![0; layer * tall];
    for (at, (input, output)) in pairs.inputs.iter().zip(&pairs.outputs).enumerate() {
        inputs[at * layer..(at + 1) * layer].copy_from_slice(input);
        outputs[at * layer..(at + 1) * layer].copy_from_slice(output);
    }
    (inputs, outputs)
}

fn real_pairs(seed: u64, count: usize) -> pinned::Pairs {
    let size = [32, 32, 1];
    let mut rng = Rng(seed);
    let inputs: Vec<Vec<u32>> = (0..count)
        .map(|_| (0..1024).map(|_| (rng.below(1 << 16)) as u32).collect())
        .collect();
    let outputs = inputs
        .iter()
        .map(|input| {
            (0..1024_i64)
                .map(|at| {
                    let (x, y) = (at % 32, at / 32);
                    let cell = |dx: i64, dy: i64| -> i64 {
                        i64::from(
                            input[(((y + dy).rem_euclid(32)) * 32 + (x + dx).rem_euclid(32))
                                as usize],
                        )
                    };
                    let sum =
                        6 * cell(0, 0) + cell(1, 0) + cell(-1, 0) + cell(0, 1) + 2 * cell(0, -1);
                    let noise = (rng.below(9) as i64) - 4;
                    (sum / 10 + 300 + noise) as u32
                })
                .collect()
        })
        .collect();
    pinned::Pairs {
        size,
        inputs,
        outputs,
    }
}

#[test]
fn p5_learn_and_check_on_data_only_examples_equal_the_placeholder_law_way() {
    let Some(host) = device("world pages p5") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    for (border, wrapped) in [("wrapped", true), ("zero", false)] {
        let pairs = pinned::life_pairs(0x5EED ^ u64::from(wrapped), 4, wrapped);
        let placeholder = pinned::container_worlds(&mut server, &pairs);
        let (inputs, outputs) = stacked(&pairs);
        let data = data_world(
            &mut server,
            [32, 32, 4],
            2,
            &[(0, "bits", &inputs), (1, "bits", &outputs)],
        );
        let by_placeholder = ok(
            &mut server,
            "grit_law_learn",
            &pinned::learn_args(placeholder, 4, border),
        );
        let by_data = ok(
            &mut server,
            "grit_law_learn",
            &pinned::learn_args(data, 4, border),
        );
        assert_eq!(
            written(&by_data),
            written(&by_placeholder),
            "learn, border {border}"
        );
        let law = served::text_at(&by_data, "law");
        let checked_placeholder = ok(
            &mut server,
            "grit_law_check",
            &pinned::check_args(&law, placeholder, 4, border),
        );
        let checked_data = ok(
            &mut server,
            "grit_law_check",
            &pinned::check_args(&law, data, 4, border),
        );
        assert_eq!(
            written(&checked_data),
            written(&checked_placeholder),
            "check, border {border}"
        );
        let wrong = checked_data.get("totals").and_then(|one| one.get("wrong"));
        assert_eq!(
            wrong,
            Some(&Json::Int(0)),
            "check, border {border}: {}",
            written(&checked_data)
        );
    }
    let pairs = real_pairs(0x4EA1, 4);
    let placeholder = pinned::container_worlds(&mut server, &pairs);
    let (inputs, outputs) = stacked(&pairs);
    let data = data_world(
        &mut server,
        [32, 32, 4],
        2,
        &[(0, "words", &inputs), (1, "words", &outputs)],
    );
    let linear = |world: i64| {
        pinned::learn_args(world, 4, "wrapped").replace(
            "\"rule\":\"outer\"",
            "\"rule\":\"linear\",\"places\":\"von_neumann\",\"frac\":16",
        )
    };
    let by_placeholder = ok(&mut server, "grit_law_learn", &linear(placeholder));
    let by_data = ok(&mut server, "grit_law_learn", &linear(data));
    assert_eq!(written(&by_data), written(&by_placeholder), "linear learn");
}

struct Counted<'server, 'host> {
    server: &'server mut Server<'host>,
    calls: u32,
}

impl Counted<'_, '_> {
    fn loaded(&mut self, args: &str) -> i64 {
        self.calls += 1;
        let got = ok(self.server, "grit_world_create", args);
        if let Some(pages) = got.get("pages").and_then(Json::list) {
            for page in pages {
                assert!(page.get("stopped").is_none(), "{}", written(&got));
            }
        }
        int_at(&got, "world")
    }
}

fn pages_args(size: [u32; 3], made: &str, pages: &[(u32, &str, &[u32])]) -> String {
    let shown: Vec<String> = pages
        .iter()
        .map(|(plane, format, words)| {
            format!(
                "{{\"plane\":{plane},\"format\":\"{format}\",\"data\":\"{}\"}}",
                encoded(&packed(format, words))
            )
        })
        .collect();
    format!(
        "{{\"wide\":{},\"deep\":{},\"tall\":{},{made},\"pages\":[{}],\"budget_ms\":{LONG}}}",
        size[0],
        size[1],
        size[2],
        shown.join(",")
    )
}

struct Set {
    label: &'static str,
    shape: [u32; 3],
    pairs: usize,
    held: usize,
    format: &'static str,
}

const X1R_LOADING_CALLS: u32 = 40;

type Loaded = (String, i64, [u32; 3], u32, Vec<u32>);

#[test]
fn p2_the_x1r_data_loads_in_one_call_per_example_set() {
    let Some(host) = device("world pages p2") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let sets = [
        Set {
            label: "p1",
            shape: [64, 64, 1],
            pairs: 12,
            held: 4,
            format: "bits",
        },
        Set {
            label: "p2",
            shape: [32, 32, 32],
            pairs: 10,
            held: 4,
            format: "bits",
        },
        Set {
            label: "p3",
            shape: [64, 64, 1],
            pairs: 8,
            held: 4,
            format: "words",
        },
    ];
    let law = format!("\"law\":{},\"params\":[]", quoted(MAJORITY));
    let mut counted = Counted {
        server: &mut server,
        calls: 0,
    };
    let mut checks: Vec<Loaded> = Vec::new();
    for (index, set) in sets.iter().enumerate() {
        let mut rng = Rng(0x0D1A_0000 + index as u64);
        let cells = (set.shape[0] * set.shape[1] * set.shape[2]) as usize;
        let field = |rng: &mut Rng| -> Vec<u32> {
            (0..cells)
                .map(|_| {
                    if set.format == "bits" {
                        u32::from(rng.below(1000) < 300)
                    } else {
                        rng.word() >> 8
                    }
                })
                .collect()
        };
        let flat = set.shape[2] == 1;
        let train: Vec<(Vec<u32>, Vec<u32>)> = (0..set.pairs)
            .map(|_| (field(&mut rng), field(&mut rng)))
            .collect();
        let held: Vec<Vec<u32>> = (0..set.held).map(|_| field(&mut rng)).collect();
        if flat {
            let tall = (set.pairs as u32).next_power_of_two();
            let size = [set.shape[0], set.shape[1], tall];
            let mut inputs = vec![0; cells * tall as usize];
            let mut outputs = inputs.clone();
            for (at, (input, output)) in train.iter().enumerate() {
                inputs[at * cells..(at + 1) * cells].copy_from_slice(input);
                outputs[at * cells..(at + 1) * cells].copy_from_slice(output);
            }
            let world = counted.loaded(&pages_args(
                size,
                "\"planes\":2",
                &[(0, set.format, &inputs), (1, set.format, &outputs)],
            ));
            checks.push((format!("{} train in", set.label), world, size, 0, inputs));
            checks.push((format!("{} train out", set.label), world, size, 1, outputs));
            let size = [set.shape[0], set.shape[1], set.held as u32];
            let starts: Vec<u32> = held.concat();
            let world = counted.loaded(&pages_args(size, &law, &[(0, set.format, &starts)]));
            checks.push((format!("{} held", set.label), world, size, 0, starts));
        } else {
            for (at, (input, output)) in train.iter().enumerate() {
                let world = counted.loaded(&pages_args(
                    set.shape,
                    "\"planes\":2",
                    &[(0, set.format, input), (1, set.format, output)],
                ));
                checks.push((
                    format!("{} train {at} in", set.label),
                    world,
                    set.shape,
                    0,
                    input.clone(),
                ));
                checks.push((
                    format!("{} train {at} out", set.label),
                    world,
                    set.shape,
                    1,
                    output.clone(),
                ));
            }
            for (at, start) in held.iter().enumerate() {
                let world = counted.loaded(&pages_args(set.shape, &law, &[(0, set.format, start)]));
                checks.push((
                    format!("{} held {at}", set.label),
                    world,
                    set.shape,
                    0,
                    start.clone(),
                ));
            }
        }
    }
    let calls = counted.calls;
    println!("REPLAY loading calls {calls}, against {X1R_LOADING_CALLS} in X1r");
    for (label, world, size, plane, want) in &checks {
        let format = if want.iter().all(|word| *word <= 1) {
            "bits"
        } else {
            "words"
        };
        let got = read_page(
            &mut server,
            *world,
            *plane,
            ([0, 0, 0], size.map(|side| side - 1)),
            format,
        );
        assert_eq!(words_hash(&got), words_hash(want), "{label}");
    }
    assert_eq!(
        calls,
        1 + 1 + 10 + 4 + 1 + 1,
        "one call per example set or held world"
    );
    assert!(calls < X1R_LOADING_CALLS);
}
