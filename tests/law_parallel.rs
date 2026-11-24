#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;
#[path = "law_parallel/pinned.rs"]
mod pinned;

use common::device;

use grit::Host;
use grit::branch::{Made, Stepping, World};
use grit::halo::{self, BOX};
use grit::law::Law;
use grit::law::chip::{
    Circuit, Copies, Fault, KEEPS, READS, Read, Rest, Spot, WRITES, Wired, Wiring,
};
use grit::netlist::Gate;
use grit::rule3::{CORNERS, Oct, Rule};
use grit::serve3::Server;
use grit::wire::{Json, written};

use pinned::{
    Counted, LONG, Rng, bays, cells, cube_faces, examples, index, inline, life, majority,
    minimal_request, moore, more_requests, request, soup, spots,
};

const KINDS: u32 = 16;

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

fn quoted(text: &str) -> String {
    written(&Json::Text(text.to_owned()))
}

fn hex(sum: &[u8; 32]) -> String {
    sum.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn parallel(arguments: &str) -> String {
    format!("{},\"parallel\":true}}", &arguments[..arguments.len() - 1])
}

const FORM: [&str; 5] = ["law", "hash", "nodes", "rest", "parallel"];

fn learning(answer: &Json) -> String {
    let Json::Map(pairs) = answer else {
        panic!("an answer is an object");
    };
    written(&Json::Map(
        pairs
            .iter()
            .filter(|(key, _)| !FORM.contains(&key.as_str()))
            .cloned()
            .collect(),
    ))
}

struct Pair {
    single: Json,
    wide: Json,
}

impl Pair {
    fn learned(server: &mut Server<'_>, arguments: &str) -> Self {
        let single = ok(server, "grit_law_learn", arguments);
        let wide = ok(server, "grit_law_learn", &parallel(arguments));
        assert_eq!(
            learning(&single),
            learning(&wide),
            "parallel changes only the law's form, not what is learned"
        );
        assert!(single.get("parallel").is_none());
        assert!(wide.get("rest").is_none());
        let told = wide.get("parallel").unwrap();
        assert_eq!(int_at(told, "copies"), 32);
        assert_eq!(
            int_at(told, "bytes"),
            i64::try_from(text_at(&wide, "law").len()).unwrap()
        );
        assert_eq!(int_at(told, "nodes"), int_at(&wide, "nodes"));
        let one = told.get("single").unwrap();
        assert_eq!(
            int_at(one, "bytes"),
            i64::try_from(text_at(&single, "law").len()).unwrap()
        );
        assert_eq!(int_at(one, "nodes"), int_at(&single, "nodes"));
        assert_ne!(text_at(&single, "law"), text_at(&wide, "law"));
        Self { single, wide }
    }

    fn sizes(&self) -> String {
        let told = self.wide.get("parallel").unwrap();
        let one = told.get("single").unwrap();
        format!(
            "gates {}, bytes {} vs {} single, nodes {} vs {} single",
            int_at(&self.wide, "gates"),
            int_at(told, "bytes"),
            int_at(one, "bytes"),
            int_at(told, "nodes"),
            int_at(one, "nodes")
        )
    }
}

fn iterating(text: &str) -> Law {
    let json = grit::law::parse::read(text).unwrap();
    let Json::Map(mut fields) = json else {
        panic!("a law is an object");
    };
    fields.push((
        "alternate".to_owned(),
        Json::List(vec![Json::Int(0), Json::Int(1)]),
    ));
    Law::from_json(&Json::Map(fields)).unwrap()
}

fn law_of(text: &str, alternate: bool) -> Law {
    if alternate {
        iterating(text)
    } else {
        Law::compile(text).unwrap()
    }
}

fn world<'host>(host: &'host Host, law: &Law, size: [u32; 3], stepping: Stepping) -> World<'host> {
    World::create_law_stepped(
        host,
        law,
        &Made {
            rule: law.name(),
            params: &[],
            size,
            sparse: false,
            slots: None,
        },
        stepping,
    )
    .unwrap()
}

fn whole_plane(world: &mut World<'_>, plane: u32, size: [u32; 3], words: &[u32]) {
    let put: Vec<([u32; 3], u32)> = spots(size)
        .into_iter()
        .map(|at| (at, words[index(size, at)]))
        .collect();
    world.write_plane(plane, &put).unwrap();
}

fn plane_of(world: &mut World<'_>, plane: u32, size: [u32; 3]) -> Vec<u32> {
    let all = world.cells().unwrap();
    let count = cells(size);
    all[plane as usize * count..(plane as usize + 1) * count].to_vec()
}

fn written_plane(world: &mut World<'_>, alternate: bool) -> u32 {
    if alternate {
        let plane = world.current().unwrap();
        assert_eq!(plane, world.frame() & 1);
        plane
    } else {
        1
    }
}

fn packed(rng: &mut Rng, size: [u32; 3], permille: u64) -> Vec<u32> {
    (0..cells(size))
        .map(|_| {
            (0..32).fold(0_u32, |word, bit| {
                word | (u32::from(rng.below(1000) < permille) << bit)
            })
        })
        .collect()
}

fn copy(words: &[u32], bit: u32) -> Vec<u32> {
    words.iter().map(|word| (word >> bit) & 1).collect()
}

fn at_of(size: [u32; 3], at: usize) -> [usize; 3] {
    [
        at % size[0] as usize,
        at / size[0] as usize % size[1] as usize,
        at / (size[0] as usize * size[1] as usize),
    ]
}

struct Compared {
    copies: u32,
    cells: usize,
    set: usize,
}

fn copies_compared(
    host: &Host,
    texts: (&str, &str),
    size: [u32; 3],
    frames: u32,
    start: &[u32],
    label: &str,
) -> Compared {
    let alternate = frames > 1;
    let (single, wide) = texts;
    let mut many = world(host, &law_of(wide, alternate), size, Stepping::Listed);
    whole_plane(&mut many, 0, size, start);
    many.step(frames).unwrap();
    let plane = written_plane(&mut many, alternate);
    let got = plane_of(&mut many, plane, size);
    let mut one = world(host, &law_of(single, alternate), size, Stepping::Whole);
    let zeros = vec![0_u32; cells(size)];
    let mut set = 0;
    for bit in 0..32 {
        whole_plane(&mut one, 0, size, &copy(start, bit));
        whole_plane(&mut one, 1, size, &zeros);
        one.step(frames).unwrap();
        let plane = written_plane(&mut one, alternate);
        let want = plane_of(&mut one, plane, size);
        for (at, word) in want.iter().enumerate() {
            assert_eq!(
                word >> 1,
                0,
                "{label}: the single-bit run set bits above 0 at {:?}",
                at_of(size, at)
            );
            assert_eq!(
                (got[at] >> bit) & 1,
                *word,
                "{label}: copy {bit} at {:?} after {frames} frames: the parallel world has {:#010x}, the single-bit run {word}",
                at_of(size, at),
                got[at]
            );
        }
        set += want.iter().filter(|word| **word == 1).count();
    }
    Compared {
        copies: 32,
        cells: cells(size),
        set,
    }
}

fn counted_request(rule: &Counted, more: &str) -> String {
    let (size, densities): ([u32; 3], &[u64]) = if rule.cube {
        ([16, 16, 16], &[120, 200, 300, 450, 600, 750, 900])
    } else {
        ([32, 32, 1], &[250, 350, 450, 600, 750])
    };
    let seed = u64::from(rule.born) ^ (u64::from(rule.kept) << 32) ^ u64::from(rule.cube);
    request(
        rule,
        "wrapped",
        &examples(rule, size, densities, seed, true, (1, 0)),
        more,
    )
}

fn pinned_request(name: &str) -> String {
    pinned::requests()
        .into_iter()
        .find(|(one, _)| *one == name)
        .unwrap()
        .1
}

fn kinds_of(answer: &Json) -> Vec<i64> {
    answer
        .get("chip")
        .and_then(|chip| chip.get("gates"))
        .and_then(Json::list)
        .unwrap()
        .iter()
        .map(|gate| int_at(gate, "kind"))
        .collect()
}

fn square_parity() -> Counted {
    Counted {
        cube: false,
        outer: false,
        places: vec![1, 3, 4, 5, 7],
        born: 0b10_1010,
        kept: 0,
    }
}

fn cube_majority() -> Counted {
    Counted {
        cube: true,
        outer: false,
        places: moore(true),
        born: 0x0FFF_C000,
        kept: 0,
    }
}

fn survivor() -> Counted {
    Counted {
        cube: false,
        outer: true,
        places: vec![1, 3, 4, 5, 7],
        born: 1 << 2,
        kept: 0b1_1111,
    }
}

#[test]
fn p1_32_soups_in_one_world_equal_32_single_bit_runs() {
    let Some(host) = device("law_parallel p1") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let cases: Vec<(&str, bool, u64, String)> = vec![
        ("outer life", false, 350, counted_request(&life(), "")),
        ("outer bays", true, 200, counted_request(&bays(), "")),
        (
            "totalistic majority",
            false,
            350,
            counted_request(&majority(), ""),
        ),
        (
            "totalistic majority",
            true,
            500,
            counted_request(&cube_majority(), ""),
        ),
        (
            "totalistic von_neumann parity",
            false,
            350,
            counted_request(&square_parity(), ",\"places\":\"von_neumann\""),
        ),
        (
            "totalistic von_neumann faces",
            true,
            500,
            counted_request(&cube_faces(), ",\"place_list\":[4,10,12,13,14,16,22]"),
        ),
        (
            "outer von_neumann survivor",
            false,
            350,
            counted_request(&survivor(), ",\"places\":\"von_neumann\""),
        ),
        ("minimal", false, 350, minimal_request(false)),
        ("minimal", true, 200, minimal_request(true)),
        (
            "outer life steps 4",
            false,
            350,
            pinned_request("outer-life-steps-4"),
        ),
        (
            "outer life vote",
            false,
            350,
            pinned_request("outer-life-vote"),
        ),
    ];
    let mut compared = 0;
    let mut constant = Vec::new();
    for (name, cube, permille, arguments) in &cases {
        let pair = Pair::learned(&mut server, arguments);
        if kinds_of(&pair.wide).contains(&15) {
            constant.push(*name);
        }
        let single = text_at(&pair.single, "law");
        let wide = text_at(&pair.wide, "law");
        let size = if *cube { [16, 16, 16] } else { [32, 32, 2] };
        for frames in [1_u32, 64] {
            let mut rng =
                Rng(0xC0 ^ u64::from(frames) ^ (u64::from(*cube) << 4) ^ (name.len() as u64));
            let start = packed(&mut rng, size, *permille);
            let label = format!(
                "{name} {} {}",
                if *cube { "3x3x3" } else { "3x3" },
                if frames == 1 {
                    "1 step"
                } else {
                    "64 frames alternate"
                }
            );
            let seen = copies_compared(&host, (&single, &wide), size, frames, &start, &label);
            println!(
                "p1 {label}: {} copies x {} cells equal, {} copy cells set; {}",
                seen.copies,
                seen.cells,
                seen.set,
                pair.sizes()
            );
            assert!(
                seen.set > 0,
                "{label}: every copy died, so nothing was compared"
            );
            compared += 1;
        }
    }
    assert_eq!(compared, 22);
    println!("p1 chips with a constant-one gate: {constant:?}");
    assert!(
        constant.contains(&"outer von_neumann survivor"),
        "{constant:?}"
    );
}

fn gl1_soups(side: u32) -> Vec<u32> {
    let size = [side; 3];
    let mut rng = Rng(5766);
    let mut out = vec![0_u32; cells(size)];
    let block = 10;
    let from = side / 2 - block / 2;
    for bit in 0..32_u32 {
        let permille = [150, 250, 350][(bit % 3) as usize];
        for z in from..from + block {
            for y in from..from + block {
                for x in from..from + block {
                    if rng.below(1000) < permille {
                        out[index(size, [x, y, z])] |= 1 << bit;
                    }
                }
            }
        }
    }
    out
}

fn served_read(server: &mut Server<'_>, number: i64, size: [u32; 3]) -> Vec<u32> {
    let per = (4096 / (size[0] * size[1])).max(1);
    let mut out = Vec::new();
    let mut z = 0;
    while z < size[2] {
        let top = (z + per).min(size[2]) - 1;
        let got = ok(
            server,
            "grit_world_read",
            &format!(
                "{{\"world\":{number},\"current\":true,\"from\":[0,0,{z}],\"to\":[{},{},{top}]}}",
                size[0] - 1,
                size[1] - 1
            ),
        );
        assert_eq!(int_at(&got, "plane"), int_at(&got, "frame") & 1);
        for cell in got.get("cells").and_then(Json::list).unwrap() {
            out.push(u32::try_from(cell.int().unwrap()).unwrap());
        }
        z = top + 1;
    }
    out
}

#[test]
fn p2_learned_bays_runs_32_soups_for_64_generations_without_a_hand_edit() {
    let Some(host) = device("law_parallel p2") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let arguments = request(
        &bays(),
        "wrapped",
        &examples(
            &bays(),
            [16, 16, 16],
            &[120, 180, 260, 400],
            0x5766,
            true,
            (1, 0),
        ),
        "",
    );
    let pair = Pair::learned(&mut server, &arguments);
    assert_eq!(
        written(pair.wide.get("tables").unwrap()),
        "[[[0,0,0,0,0,0,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],[0,0,0,0,0,1,1,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]]]"
    );
    let side = 32;
    let size = [side; 3];
    let made = ok(
        &mut server,
        "grit_world_create",
        &format!(
            "{{\"wide\":{side},\"deep\":{side},\"tall\":{side},\"law\":{},\"alternate\":[0,1],\"budget_ms\":{LONG}}}",
            quoted(&text_at(&pair.wide, "law"))
        ),
    );
    let number = int_at(&made, "world");
    let start = gl1_soups(side);
    let put: Vec<String> = spots(size)
        .into_iter()
        .filter(|at| start[index(size, *at)] != 0)
        .map(|at| format!("[{},{},{},{}]", at[0], at[1], at[2], start[index(size, at)]))
        .collect();
    ok(
        &mut server,
        "grit_world_write",
        &format!(
            "{{\"world\":{number},\"current\":true,\"cells\":[{}],\"budget_ms\":{LONG}}}",
            put.join(",")
        ),
    );
    let stepped = ok(
        &mut server,
        "grit_world_step",
        &format!("{{\"world\":{number},\"frames\":64,\"budget_ms\":{LONG}}}"),
    );
    assert_eq!(int_at(&stepped, "frame"), 64);
    assert_eq!(int_at(&stepped, "current"), 0);
    let got = served_read(&mut server, number, size);
    let mut one = world(
        &host,
        &iterating(&text_at(&pair.single, "law")),
        size,
        Stepping::Whole,
    );
    let zeros = vec![0_u32; cells(size)];
    let mut alive = Vec::new();
    for bit in 0..32 {
        whole_plane(&mut one, 0, size, &copy(&start, bit));
        whole_plane(&mut one, 1, size, &zeros);
        one.step(64).unwrap();
        assert_eq!(one.current().unwrap(), 0);
        let want = plane_of(&mut one, 0, size);
        let mine = copy(&got, bit);
        assert!(
            mine == want,
            "soup {bit}: generation 64 differs at {:?}",
            mine.iter()
                .zip(&want)
                .position(|(a, b)| a != b)
                .map(|at| at_of(size, at))
        );
        alive.push(want.iter().filter(|word| **word == 1).count());
    }
    println!(
        "p2 Bays 5766 learned with parallel, 32 soups in one {side}^3 world, 64 generations with alternate [0,1]: equal to 32 single-bit runs; alive per soup {alive:?}; {}",
        pair.sizes()
    );
}

const PINS: [(&str, &str); 5] = [
    (
        "outer-life",
        "b68d51eebbdad70d4476386f9691da65155aba81e5ec0eb2604c0bc6249c5962",
    ),
    (
        "totalistic-majority-zero",
        "5ea428bc7754f4b26b69541884cedaf8c0e090aa87043a4bef36d14b6e8ba73b",
    ),
    (
        "totalistic-faces-3d",
        "1ed870ef9f455b166ec4e7d1a020f8ec193517e9de8891f0c25aefee4b7ecef9",
    ),
    (
        "outer-life-steps-4",
        "676fc6f94d1aa4cb9b93de56b4482257a8d3b6602fb4cfe77d4f36133ecdcd7e",
    ),
    (
        "outer-life-vote",
        "b1fc603208dfb7d463536650527fb307d53647ed31a92849d108578005c78a97",
    ),
];

#[test]
fn p3_answers_without_the_option_are_the_bytes_recorded_on_base() {
    let mut server = Server::new(None);
    let requests = pinned::requests();
    assert_eq!(requests.len(), PINS.len());
    for ((name, arguments), (pinned_name, sum)) in requests.iter().zip(PINS) {
        assert_eq!(*name, pinned_name);
        let answer = written(&ok(&mut server, "grit_law_learn", arguments));
        assert_eq!(hex(&grit::branch::sha256(answer.as_bytes())), sum, "{name}");
        let off = format!("{},\"parallel\":false}}", &arguments[..arguments.len() - 1]);
        assert_eq!(
            written(&ok(&mut server, "grit_law_learn", &off)),
            answer,
            "{name}: parallel false"
        );
        let wide = ok(&mut server, "grit_law_learn", &parallel(arguments));
        assert_eq!(
            int_at(wide.get("parallel").unwrap(), "copies"),
            32,
            "{name}"
        );
        println!(
            "p3 {name}: sha256 {sum}, {} bytes, as on base 64b0d1a",
            answer.len()
        );
    }
}

fn refused_at(server: &mut Server<'_>, arguments: &str, code: &str, field: &str) {
    let got = no(server, "grit_law_learn", arguments);
    assert_eq!(text_at(&got, "code"), code, "{}", written(&got));
    assert_eq!(text_at(&got, "field"), field, "{}", written(&got));
}

#[test]
fn p4_misuse_is_refused_by_code_and_field() {
    let mut server = Server::new(None);
    let square = [8, 8, 1];
    let mut rng = Rng(44);
    let input = soup(&mut rng, cells(square), 400);
    let output = pinned::counted_step(&life(), square, &input, true);
    let pair = format!("[{}]", inline(square, &input, &output));
    let base = request(&life(), "wrapped", &pair, "");
    let with = |from: &str, to: &str| {
        assert!(base.contains(from));
        base.replacen(from, to, 1)
    };
    refused_at(
        &mut server,
        &parallel(&with("\"plane\":0,\"bit\":0", "\"plane\":0,\"bit\":3")),
        "parallel_bit",
        "$.reads[0].bit",
    );
    refused_at(
        &mut server,
        &parallel(&with("\"plane\":1,\"bit\":0", "\"plane\":1,\"bit\":31")),
        "parallel_bit",
        "$.writes[0].bit",
    );
    let rested = |rest: &str| format!("{},\"rest\":\"{rest}\"}}", &base[..base.len() - 1]);
    let kept = ok(&mut server, "grit_law_learn", &parallel(&rested("kept")));
    let cleared = ok(&mut server, "grit_law_learn", &parallel(&rested("cleared")));
    assert!(
        text_at(&kept, "law") == text_at(&cleared, "law"),
        "rest has no bits to act on in parallel form"
    );
    assert!(kept.get("rest").is_none());
    let moved = with("\"plane\":1,\"bit\":0", "\"plane\":1,\"bit\":1");
    let stepped_off = format!("{},\"steps\":2}}", &moved[..moved.len() - 1]);
    refused_at(&mut server, &stepped_off, "steps_writes", "$.writes");
    refused_at(
        &mut server,
        &parallel(&stepped_off),
        "parallel_bit",
        "$.writes[0].bit",
    );
    let wrong = format!("{},\"parallel\":1}}", &base[..base.len() - 1]);
    let got = no(&mut server, "grit_law_learn", &wrong);
    assert_eq!(text_at(&got, "field"), "$.parallel", "{}", written(&got));
    let two = with(
        "\"writes\":[{\"plane\":1,\"bit\":0}]",
        "\"writes\":[{\"plane\":1,\"bit\":0},{\"plane\":1,\"bit\":1}]",
    );
    refused_at(
        &mut server,
        &parallel(&two),
        "parallel_bit",
        "$.writes[1].bit",
    );
    let linear = with("\"rule\":\"outer\"", "\"rule\":\"linear\"");
    refused_at(
        &mut server,
        &parallel(&linear),
        "parallel_rule",
        "$.parallel",
    );
    let numbers = ok(&mut server, "grit_law_learn", &linear);
    assert!(numbers.get("parallel").is_none());
    let fine = ok(&mut server, "grit_law_learn", &parallel(&base));
    assert_eq!(int_at(fine.get("parallel").unwrap(), "copies"), 32);
    let circuit = Circuit {
        state: 0,
        inputs: 1,
        outputs: 1,
        gates: vec![Gate {
            kind: 5,
            left: 0,
            right: 0,
        }],
    };
    let wiring = |bit: u32| Wiring {
        name: "not".to_owned(),
        channels: 2,
        reads: vec![Read {
            plane: 0,
            at: [0, 0, 0],
            bit,
        }],
        writes: vec![Spot { plane: 1, bit: 0 }],
        keeps: Vec::new(),
        rest: Rest::Kept,
    };
    let why = Wired::new(circuit.clone(), wiring(2))
        .unwrap()
        .parallel()
        .unwrap_err();
    assert_eq!(why.path, "$.bind_inputs[0].bit");
    let wired = Wired::new(circuit, wiring(0)).unwrap();
    assert_eq!(wired.copies(), Copies::One);
    assert_eq!(wired.clone().parallel().unwrap().copies(), Copies::Word);
    println!(
        "p4 parallel_bit (reads, writes, steps, two writes on one plane), parallel_rule under linear, rest without effect (kept equals cleared), a non-boolean parallel and the chip-level bit refusal"
    );
}

fn next(state: &mut u64) -> u64 {
    let mut rng = Rng(*state);
    let out = rng.next();
    *state = rng.0;
    out
}

fn pick(state: &mut u64, most: u32) -> u32 {
    u32::try_from(next(state) % u64::from(most)).unwrap()
}

fn drawn(state: &mut u64) -> (Circuit, Wiring) {
    let held = pick(state, 2);
    let outputs = 1 + pick(state, 2);
    let inputs = 1 + pick(state, 6);
    let extra = pick(state, 20);
    let count = held + outputs + extra;
    let gates = (0..count)
        .map(|at| {
            let sources = held + inputs + at;
            Gate {
                kind: pick(state, KINDS),
                left: pick(state, sources),
                right: pick(state, sources),
            }
        })
        .collect();
    let circuit = Circuit {
        state: held,
        inputs,
        outputs,
        gates,
    };
    let mut planes: Vec<u32> = (0..4).collect();
    for at in (1..planes.len()).rev() {
        let other = pick(state, u32::try_from(at + 1).unwrap()) as usize;
        planes.swap(at, other);
    }
    let written = &planes[..(held + outputs) as usize];
    let keeps: Vec<Spot> = written[..held as usize]
        .iter()
        .map(|plane| Spot {
            plane: *plane,
            bit: 0,
        })
        .collect();
    let writes: Vec<Spot> = written[held as usize..]
        .iter()
        .map(|plane| Spot {
            plane: *plane,
            bit: 0,
        })
        .collect();
    let reads = (0..inputs)
        .map(|_| {
            let plane = pick(state, 4);
            let at = if written.contains(&plane) {
                [0, 0, 0]
            } else {
                [0, 1, 2].map(|_| i64::from(pick(state, 3)) - 1)
            };
            Read { plane, at, bit: 0 }
        })
        .collect();
    let wiring = Wiring {
        name: "drawn".to_owned(),
        channels: 4,
        reads,
        writes,
        keeps,
        rest: if pick(state, 2) == 0 {
            Rest::Kept
        } else {
            Rest::Cleared
        },
    };
    (circuit, wiring)
}

fn stepped(law: &Law, boxes: &[[u32; BOX]]) -> Vec<[u32; CORNERS]> {
    let own = |plane: usize, corner: usize| boxes[plane][halo::looked(corner, [0, 0, 0]).unwrap()];
    let words: [u32; CORNERS] = core::array::from_fn(|corner| own(0, corner));
    let upper: Vec<[u32; CORNERS]> = (1..boxes.len())
        .map(|plane| core::array::from_fn(|corner| own(plane, corner)))
        .collect();
    let got = law.step_halo(
        Oct {
            words,
            frame: 0,
            at: [0; 3],
            params: &[],
        },
        &upper,
        boxes,
    );
    let mut out = vec![got.ruled.out];
    out.extend(got.planes);
    out
}

#[test]
fn p0_every_bit_of_a_parallel_chip_law_is_the_single_bit_law_on_the_cpu() {
    let mut state = 0xB17_u64;
    let mut kinds = [0_u32; KINDS as usize];
    let mut boxes_checked = 0;
    let (mut wide_nodes, mut single_nodes, mut wide_bytes, mut single_bytes) = (0, 0, 0, 0);
    for _ in 0..64 {
        let (circuit, wiring) = drawn(&mut state);
        for gate in &circuit.gates {
            kinds[gate.kind as usize] += 1;
        }
        let single = Wired::new(circuit, wiring).unwrap();
        let wide = single.clone().parallel().unwrap();
        let single_law = Law::compile(&single.text()).unwrap();
        let wide_law = Law::compile(&wide.text()).unwrap();
        assert_eq!(wide_law.halo(), single_law.halo());
        wide_nodes += wide_law.nodes();
        single_nodes += single_law.nodes();
        wide_bytes += wide.text().len();
        single_bytes += single.text().len();
        for _ in 0..8 {
            let boxes: Vec<[u32; BOX]> = (0..4)
                .map(|_| core::array::from_fn(|_| u32::try_from(next(&mut state) >> 32).unwrap()))
                .collect();
            let got = stepped(&wide_law, &boxes);
            for bit in 0..32 {
                let bits: Vec<[u32; BOX]> = boxes
                    .iter()
                    .map(|one| one.map(|word| (word >> bit) & 1))
                    .collect();
                let want = stepped(&single_law, &bits);
                for (plane, words) in want.iter().enumerate() {
                    for corner in 0..CORNERS {
                        assert_eq!(words[corner] >> 1, 0);
                        assert_eq!(
                            (got[plane][corner] >> bit) & 1,
                            words[corner],
                            "copy {bit}, plane {plane}, corner {corner}\n{}",
                            wide.text()
                        );
                    }
                }
            }
            boxes_checked += 1;
        }
    }
    assert!(kinds.iter().all(|seen| *seen > 0), "{kinds:?}");
    println!(
        "p0 64 drawn chips, {boxes_checked} random boxes, 32 copies each, every gate kind {kinds:?}: equal; nodes {wide_nodes} parallel vs {single_nodes} single, bytes {wide_bytes} vs {single_bytes}"
    );
}

fn bit_moved(wiring: &Wiring, list: &str, at: usize, bit: u32) -> Wiring {
    let mut moved = wiring.clone();
    match list {
        READS => moved.reads[at].bit = bit,
        WRITES => moved.writes[at].bit = bit,
        _ => moved.keeps[at].bit = bit,
    }
    moved
}

fn parallel_refused(circuit: &Circuit, wiring: Wiring, list: &str, at: usize, bit: u32) {
    let single = Wired::new(circuit.clone(), wiring).unwrap();
    assert_eq!(single.copies(), Copies::One);
    Law::compile(&single.text()).unwrap();
    let why = single.parallel().unwrap_err();
    assert_eq!(why.fault, Fault::Bit, "{why:?}");
    assert_eq!(why.path, format!("$.{list}[{at}].bit"), "{why:?}");
    assert!(
        why.message
            .starts_with(&format!("{list}[{at}] names bit {bit}; in parallel form")),
        "{}",
        why.message
    );
}

#[test]
fn p5_the_chip_refuses_a_write_or_keep_bit_past_0_in_parallel_form_by_name() {
    let mut state = 0x7A3_u64;
    let mut seen = [0_u32; 3];
    let mut bits = Vec::new();
    for _ in 0..96 {
        let (circuit, wiring) = drawn(&mut state);
        let bit = 1 + pick(&mut state, 31);
        bits.push(bit);
        for (slot, (list, count)) in [
            (WRITES, wiring.writes.len()),
            (KEEPS, wiring.keeps.len()),
            (READS, wiring.reads.len()),
        ]
        .into_iter()
        .enumerate()
        {
            for at in 0..count {
                parallel_refused(&circuit, bit_moved(&wiring, list, at, bit), list, at, bit);
                seen[slot] += 1;
            }
        }
        if wiring.writes.len() == 2 {
            let both = bit_moved(&bit_moved(&wiring, WRITES, 1, 31), WRITES, 0, bit);
            parallel_refused(&circuit, both, WRITES, 0, bit);
            let later = bit_moved(&wiring, WRITES, 1, bit);
            parallel_refused(&circuit, later, WRITES, 1, bit);
        }
        if !wiring.keeps.is_empty() {
            let other = 1 + bit % 31;
            let kept = bit_moved(&wiring, KEEPS, 0, bit);
            let both = bit_moved(&kept, WRITES, 0, other);
            parallel_refused(&circuit, both.clone(), WRITES, 0, other);
            let all = bit_moved(&both, READS, 0, 7);
            parallel_refused(&circuit, all, READS, 0, 7);
        }
        assert_eq!(
            Wired::new(circuit, wiring)
                .unwrap()
                .parallel()
                .unwrap()
                .copies(),
            Copies::Word
        );
    }
    assert!(seen.iter().all(|count| *count >= 32), "{seen:?}");
    assert!(bits.contains(&31) && bits.contains(&1), "{bits:?}");
    println!(
        "p5 96 drawn chips: a write bit refused {} times, a keep bit {} times, a read bit {} times, each by its own path; reads before writes before keeps, the first entry of a list first",
        seen[0], seen[1], seen[2]
    );
}

const MORE_PINS: [(&str, &str, &str); 3] = [
    (
        "minimal-square",
        "8421a9d4d1ec0aff8c26976b3d2b6ac44082356a9db4e78fcddff8afe658e81e",
        "3e14075c2103ce69c9f1fd4341f0201cadb4ac6bc063f6094fe186c4b85dc1ec",
    ),
    (
        "outer-life-bits-3-to-5",
        "f792b28df9ebe48c2964e269346b74d5a2243ff3752c7664a529d5c5c1049926",
        "c8b7f3dc2f5d43c6ab7505c69db254c7999f14953f4709184f40d1fb9cb7fed4",
    ),
    (
        "outer-life-rest-cleared",
        "24264f5e2d0b9404a83c53f5fb709f8f12a045da75f456044a12a58b1b65ffe1",
        "267238dee561b99534769dfe21a9a95ccb56e4bdb4ba044d0ee7fa65e9b26dde",
    ),
];

fn learned_bytes(server: &mut Server<'_>, arguments: &str) -> (bool, String) {
    let got = server.heard(&line("grit_law_learn", arguments)).unwrap();
    let held = got.get("result").unwrap();
    let error = held.get("isError") == Some(&Json::Bool(true));
    (error, written(held.get("structuredContent").unwrap()))
}

fn sum_of(text: &str) -> String {
    hex(&grit::branch::sha256(text.as_bytes()))
}

fn pinned_pair(server: &mut Server<'_>, at: usize) -> (Json, bool, Json) {
    let (name, arguments) = more_requests().swap_remove(at);
    let (pinned, single, wide) = MORE_PINS[at];
    assert_eq!(name, pinned);
    let (error, answer) = learned_bytes(server, &arguments);
    assert!(!error, "{name}: {answer}");
    let (wide_error, wide_answer) = learned_bytes(server, &parallel(&arguments));
    println!(
        "p6 {name}: single sha256 {}, {} bytes; parallel sha256 {}, {} bytes, refused {wide_error}",
        sum_of(&answer),
        answer.len(),
        sum_of(&wide_answer),
        wide_answer.len()
    );
    assert_eq!(
        sum_of(&answer),
        single,
        "{name}: single, recorded on 64b0d1a"
    );
    assert_eq!(
        sum_of(&wide_answer),
        wide,
        "{name}: parallel, recorded on a016951"
    );
    let off = format!("{},\"parallel\":false}}", &arguments[..arguments.len() - 1]);
    assert_eq!(
        learned_bytes(server, &off),
        (false, answer.clone()),
        "{name}: parallel false"
    );
    (
        grit::wire::read(&answer).unwrap(),
        wide_error,
        grit::wire::read(&wide_answer).unwrap(),
    )
}

fn junk_boxes(state: &mut u64) -> Vec<[u32; BOX]> {
    (0..2)
        .map(|plane| {
            core::array::from_fn(|_| {
                let word = u32::try_from(next(state) >> 32).unwrap();
                if plane == 0 { word & 1 } else { word }
            })
        })
        .collect()
}

#[test]
fn p6_a_moved_bit_and_a_cleared_rest_answer_the_recorded_bytes() {
    let mut server = Server::new(None);
    assert_eq!(more_requests().len(), MORE_PINS.len());
    let (moved, moved_refused, refusal) = pinned_pair(&mut server, 1);
    assert!(moved_refused, "a moved bit has no parallel form");
    assert_eq!(text_at(&refusal, "code"), "parallel_bit");
    assert_eq!(text_at(&refusal, "field"), "$.reads[0].bit");
    assert_eq!(text_at(&moved, "rest"), "kept");
    let (cleared, cleared_refused, cleared_wide) = pinned_pair(&mut server, 2);
    assert!(!cleared_refused);
    assert_eq!(text_at(&cleared, "rest"), "cleared");
    assert!(cleared_wide.get("rest").is_none());
    assert_eq!(int_at(cleared_wide.get("parallel").unwrap(), "copies"), 32);
    let kept_request = more_requests()[2]
        .1
        .replace("\"rest\":\"cleared\"", "\"rest\":\"kept\"");
    let kept = ok(&mut server, "grit_law_learn", &kept_request);
    assert_eq!(text_at(&kept, "rest"), "kept");
    let cleared_law = Law::compile(&text_at(&cleared, "law")).unwrap();
    let kept_law = Law::compile(&text_at(&kept, "law")).unwrap();
    let moved_law = Law::compile(&text_at(&moved, "law")).unwrap();
    let mut state = 0x6E57_u64;
    let mut kept_rest = 0;
    let mut moved_set = 0;
    for _ in 0..64 {
        let boxes = junk_boxes(&mut state);
        let got = stepped(&cleared_law, &boxes);
        let kept_got = stepped(&kept_law, &boxes);
        let shifted: Vec<[u32; BOX]> = boxes
            .iter()
            .enumerate()
            .map(|(plane, one)| {
                if plane == 0 {
                    one.map(|word| (word << 3) | 0xFFFF_0000)
                } else {
                    *one
                }
            })
            .collect();
        let moved_got = stepped(&moved_law, &shifted);
        for corner in 0..CORNERS {
            let own = boxes[1][halo::looked(corner, [0, 0, 0]).unwrap()];
            assert_eq!(
                got[1][corner] >> 1,
                0,
                "the rest of the written word is cleared"
            );
            assert_eq!(kept_got[1][corner] & !1, own & !1, "kept keeps the rest");
            assert_eq!(got[1][corner], kept_got[1][corner] & 1);
            assert_eq!(
                moved_got[1][corner],
                (own & !(1 << 5)) | ((got[1][corner] & 1) << 5),
                "the moved law is the same law on bit 3 into bit 5, the rest kept"
            );
            kept_rest += u32::from(own >> 1 != 0);
            moved_set += got[1][corner] & 1;
        }
    }
    assert!(kept_rest > 400, "{kept_rest}");
    assert!(moved_set > 16, "{moved_set}");
    println!(
        "p6 rest cleared: bits 1 to 31 of every written word are 0 on 512 junk corners, kept keeps them; the bit 3 to 5 law equals it on bit 5"
    );
}

#[test]
fn p7_a_minimal_answer_and_its_parallel_form_are_the_recorded_bytes() {
    let Some(host) = device("law_parallel p7 minimal pinned") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let (minimal, refused, wide) = pinned_pair(&mut server, 0);
    assert_eq!(text_at(minimal.get("unseen").unwrap(), "rule"), "minimal");
    assert!(!refused);
    assert_eq!(int_at(wide.get("parallel").unwrap(), "copies"), 32);
    assert!(wide.get("rest").is_none());
    assert_eq!(
        written(minimal.get("chip").unwrap()),
        written(wide.get("chip").unwrap()),
        "the same circuit"
    );
}
