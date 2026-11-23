#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use std::collections::BTreeMap;

use grit::Host;
use grit::netlist::Gate;
use grit::serve3::Server;
use grit::stencil::{self, Stencil};
use grit::wire::{Json, written};

const LONG: u64 = 600_000;

const CHUNK: usize = 4096;

const TRAIN: u64 = 20_261_123;

const HEX: [u32; 7] = [1, 2, 3, 4, 5, 6, 7];

const HEX_OFFSETS: [(i64, i64); 6] = [(1, 0), (-1, 0), (0, 1), (0, -1), (1, -1), (-1, 1)];

const VN_SQUARE: [u32; 5] = [1, 3, 4, 5, 7];

const VN_CUBE: [u32; 7] = [4, 10, 12, 13, 14, 16, 22];

const BASE_SHA: [(&str, &str); 6] = [
    (
        "a1",
        "31aa3032589f900175a162bbdd7ef7474a4a4201f0bf932675449fa7f429f5df",
    ),
    (
        "a2",
        "4bff623e041b7aab388f61b89319bcb585499c8f54b5a5e238da53bc2a8e73c2",
    ),
    (
        "not_counting",
        "ab2d36e09ce5da9e425b321245dda403ebfa5a034e9c5a9857776f9d5b67dd4d",
    ),
    (
        "vote",
        "fc8f39c8beaaf7ac367b08eab686eb650a995baa1c855d7945a6e8b93ee10da7",
    ),
    (
        "conflict",
        "3c0469ba83e602cc55bb700dd21c59ee24b204ecaad7e68d2c9ee9ceddabb1e5",
    ),
    (
        "cube",
        "922ea714fbb1ec35940314c5adc7a2f756fa5be64aa1304b50b91d3cb540e2e0",
    ),
];

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, most: u64) -> u32 {
        u32::try_from(self.next() % most).unwrap()
    }
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
    assert_eq!(
        held.get("isError"),
        Some(&Json::Bool(error)),
        "{}",
        &written(got)[..written(got).len().min(4000)]
    );
    held.get("structuredContent").cloned().unwrap()
}

fn text_at(held: &Json, key: &str) -> String {
    held.get(key)
        .and_then(Json::text)
        .unwrap_or_else(|| panic!("no {key} in {}", written(held)))
        .to_owned()
}

fn int_at(held: &Json, key: &str) -> i64 {
    held.get(key)
        .and_then(Json::int)
        .unwrap_or_else(|| panic!("no {key} in {}", written(held)))
}

fn words(held: &Json) -> Vec<u32> {
    held.list()
        .unwrap()
        .iter()
        .map(|one| u32::try_from(one.int().unwrap()).unwrap())
        .collect()
}

fn listed(held: &[u32]) -> String {
    format!(
        "[{}]",
        held.iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn shown_of(held: &Json, key: &str) -> String {
    written(
        held.get(key)
            .unwrap_or_else(|| panic!("no {key} in {}", written(held))),
    )
}

fn without(held: &Json, key: &str) -> String {
    match held {
        Json::Map(fields) => written(&Json::Map(
            fields
                .iter()
                .filter(|(name, _)| name != key)
                .cloned()
                .collect(),
        )),
        other => written(other),
    }
}

fn sha_of(held: &Json) -> String {
    grit::law::hex(&grit::branch::sha256(written(held).as_bytes()))
}

fn gates_of(held: &Json) -> Vec<Gate> {
    held.list()
        .unwrap()
        .iter()
        .map(|one| Gate {
            kind: u32::try_from(int_at(one, "kind")).unwrap(),
            left: u32::try_from(int_at(one, "left")).unwrap(),
            right: u32::try_from(int_at(one, "right")).unwrap(),
        })
        .collect()
}

struct Mcp<'host> {
    server: Server<'host>,
}

impl<'host> Mcp<'host> {
    fn new(host: Option<&'host Host>) -> Self {
        Self {
            server: Server::new(host),
        }
    }

    fn raw(&mut self, name: &str, arguments: &str) -> Json {
        self.server.heard(&line(name, arguments)).unwrap()
    }

    fn ok(&mut self, name: &str, arguments: &str) -> Json {
        result(&self.raw(name, arguments), false)
    }

    fn no(&mut self, name: &str, arguments: &str) -> Json {
        result(&self.raw(name, arguments), true)
    }

    fn world(&mut self, law: &str, size: [u32; 3]) -> u32 {
        let got = self.ok(
            "grit_world_create",
            &format!(
                "{{\"wide\":{},\"deep\":{},\"tall\":{},\"law\":{}}}",
                size[0],
                size[1],
                size[2],
                written(&Json::Text(law.to_owned()))
            ),
        );
        u32::try_from(int_at(&got, "world")).unwrap()
    }

    fn write(&mut self, world: u32, cells: &[([u32; 3], u32)]) {
        for piece in cells.chunks(CHUNK) {
            let shown: Vec<String> = piece
                .iter()
                .map(|(at, word)| format!("[{},{},{},{word}]", at[0], at[1], at[2]))
                .collect();
            let got = self.ok(
                "grit_world_write",
                &format!(
                    "{{\"world\":{world},\"plane\":0,\"cells\":[{}],\"budget_ms\":{LONG}}}",
                    shown.join(",")
                ),
            );
            assert_eq!(int_at(&got, "written"), piece.len() as i64);
        }
    }

    fn read(&mut self, world: u32, size: [u32; 3]) -> Vec<u32> {
        let mut out = Vec::new();
        let rows = (CHUNK as u32 / size[0]).max(1);
        for z in 0..size[2] {
            let mut y = 0;
            while y < size[1] {
                let last = (y + rows).min(size[1]) - 1;
                let got = self.ok(
                    "grit_world_read",
                    &format!(
                        "{{\"world\":{world},\"plane\":1,\"from\":[0,{y},{z}],\"to\":[{},{last},{z}]}}",
                        size[0] - 1
                    ),
                );
                out.extend(words(got.get("cells").unwrap()));
                y = last + 1;
            }
        }
        out
    }

    fn ran(&mut self, law: &str, size: [u32; 3], input: &[u32]) -> Vec<u32> {
        let world = self.world(law, size);
        let mut cells = Vec::new();
        for z in 0..size[2] {
            for y in 0..size[1] {
                for x in 0..size[0] {
                    let word = input[stencil::index(size, [x, y, z])];
                    if word != 0 {
                        cells.push(([x, y, z], word));
                    }
                }
            }
        }
        self.write(world, &cells);
        self.ok(
            "grit_world_step",
            &format!("{{\"world\":{world},\"frames\":1,\"budget_ms\":{LONG}}}"),
        );
        let got = self.read(world, size);
        self.ok("grit_world_close", &format!("{{\"world\":{world}}}"));
        got
    }
}

fn random_bits(rng: &mut Rng, cells: usize, permille: u64) -> Vec<u32> {
    (0..cells)
        .map(|_| u32::from(u64::from(rng.below(1000)) < permille))
        .collect()
}

fn flipped(rng: &mut Rng, held: &[u32], permille: u64) -> Vec<u32> {
    held.iter()
        .map(|one| one ^ u32::from(u64::from(rng.below(1000)) < permille))
        .collect()
}

fn cells(size: [u32; 3]) -> usize {
    size.iter().product::<u32>() as usize
}

fn own_offset(stencil: Stencil, place: u32) -> [i64; 3] {
    let p = i64::from(place);
    let dz = if stencil == Stencil::Cube {
        p / 9 - 1
    } else {
        0
    };
    [p % 3 - 1, p / 3 % 3 - 1, dz]
}

fn own_rows(stencil: Stencil, size: [u32; 3], input: &[u32], wrapped: bool) -> Vec<u32> {
    let mut rows = Vec::new();
    for z in 0..size[2] {
        for y in 0..size[1] {
            for x in 0..size[0] {
                let mut row = 0_u32;
                for place in 0..stencil.positions() {
                    let offset = own_offset(stencil, place);
                    let at = [
                        i64::from(x) + offset[0],
                        i64::from(y) + offset[1],
                        i64::from(z) + offset[2],
                    ];
                    let mut spot = [0_u32; 3];
                    let mut inside = true;
                    for axis in 0..3 {
                        let side = i64::from(size[axis]);
                        let value = if wrapped {
                            at[axis].rem_euclid(side)
                        } else {
                            at[axis]
                        };
                        if (0..side).contains(&value) {
                            spot[axis] = u32::try_from(value).unwrap();
                        } else {
                            inside = false;
                        }
                    }
                    if inside && input[stencil::index(size, spot)] & 1 == 1 {
                        row |= 1 << place;
                    }
                }
                rows.push(row);
            }
        }
    }
    rows
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Rule {
    Totalistic,
    Outer,
}

impl Rule {
    const fn name(self) -> &'static str {
        match self {
            Self::Totalistic => "totalistic",
            Self::Outer => "outer",
        }
    }
}

fn own_centre(stencil: Stencil) -> u32 {
    match stencil {
        Stencil::Square => 4,
        Stencil::Cube => 13,
    }
}

fn every(stencil: Stencil) -> Vec<u32> {
    (0..stencil.positions()).collect()
}

fn own_counts(rule: Rule, mask: &[u32]) -> u32 {
    match rule {
        Rule::Totalistic => mask.len() as u32 + 1,
        Rule::Outer => mask.len() as u32,
    }
}

fn own_classes(rule: Rule, mask: &[u32]) -> u32 {
    match rule {
        Rule::Totalistic => own_counts(rule, mask),
        Rule::Outer => 2 * own_counts(rule, mask),
    }
}

fn own_class(rule: Rule, stencil: Stencil, mask: &[u32], row: u32) -> u32 {
    let centre = own_centre(stencil);
    let n = mask
        .iter()
        .filter(|place| rule == Rule::Totalistic || **place != centre)
        .filter(|place| (row >> **place) & 1 == 1)
        .count() as u32;
    match rule {
        Rule::Totalistic => n,
        Rule::Outer => ((row >> centre) & 1) * own_counts(rule, mask) + n,
    }
}

fn grouped(rule: Rule, mask: &[u32], each: &dyn Fn(u32) -> Option<String>) -> String {
    let counts = own_counts(rule, mask);
    let row = |from: u32| -> String {
        let held: Vec<String> = (from..from + counts).filter_map(each).collect();
        format!("[{}]", held.join(","))
    };
    match rule {
        Rule::Totalistic => row(0),
        Rule::Outer => format!("[{},{}]", row(0), row(counts)),
    }
}

struct Example {
    size: [u32; 3],
    input: Vec<u32>,
    output: Vec<u32>,
}

fn shown(examples: &[Example], writes: u32) -> String {
    let shown: Vec<String> = examples
        .iter()
        .map(|one| {
            let outs: Vec<String> = (0..writes)
                .map(|write| {
                    let held: Vec<u32> =
                        one.output.iter().map(|word| word & (1 << write)).collect();
                    listed(&held)
                })
                .collect();
            format!(
                "{{\"size\":[{},{},{}],\"input\":[{}],\"output\":[{}]}}",
                one.size[0],
                one.size[1],
                one.size[2],
                listed(&one.input),
                outs.join(",")
            )
        })
        .collect();
    format!("[{}]", shown.join(","))
}

fn args(
    stencil: Stencil,
    rule: &str,
    border: &str,
    examples: &[Example],
    writes: u32,
    more: &str,
) -> String {
    let shown_writes: Vec<String> = (0..writes)
        .map(|bit| format!("{{\"plane\":1,\"bit\":{bit}}}"))
        .collect();
    format!(
        "{{\"stencil\":\"{}\",\"reads\":[{{\"plane\":0,\"bit\":0}}],\"writes\":[{}],\"border\":\"{border}\",\"rule\":\"{rule}\",\"examples\":{}{more},\"budget_ms\":{LONG}}}",
        stencil.name(),
        shown_writes.join(","),
        shown(examples, writes)
    )
}

fn labelled(
    stencil: Stencil,
    inputs: Vec<(Vec<u32>, [u32; 3])>,
    wrapped: bool,
    truth: &dyn Fn(u32) -> u32,
) -> Vec<Example> {
    inputs
        .into_iter()
        .map(|(input, size)| {
            let output = own_rows(stencil, size, &input, wrapped)
                .into_iter()
                .map(truth)
                .collect();
            Example {
                size,
                input,
                output,
            }
        })
        .collect()
}

fn random_inputs(rng: &mut Rng, size: [u32; 3], densities: &[u64]) -> Vec<(Vec<u32>, [u32; 3])> {
    densities
        .iter()
        .map(|permille| (random_bits(rng, cells(size), *permille), size))
        .collect()
}

fn hex_step(size: [u32; 3], input: &[u32], born: &[u32], kept: &[u32]) -> Vec<u32> {
    let [wide, deep, tall] = size;
    let mut out = vec![0_u32; input.len()];
    for z in 0..tall {
        for y in 0..deep {
            for x in 0..wide {
                let n = HEX_OFFSETS
                    .iter()
                    .filter(|(dx, dy)| {
                        let at = [
                            u32::try_from((i64::from(x) + dx).rem_euclid(i64::from(wide))).unwrap(),
                            u32::try_from((i64::from(y) + dy).rem_euclid(i64::from(deep))).unwrap(),
                            z,
                        ];
                        input[stencil::index(size, at)] & 1 == 1
                    })
                    .count() as u32;
                let here = stencil::index(size, [x, y, z]);
                out[here] = u32::from(if input[here] & 1 == 1 {
                    kept.contains(&n)
                } else {
                    born.contains(&n)
                });
            }
        }
    }
    out
}

fn hex_life(size: [u32; 3], input: &[u32]) -> Vec<u32> {
    hex_step(size, input, &[2], &[3, 4])
}

fn hex_pairs(seed: u64, count: usize, size: [u32; 3]) -> Vec<Example> {
    let mut rng = Rng(seed);
    (0..count)
        .map(|at| {
            let mut input = random_bits(&mut rng, cells(size), 350);
            if at >= count / 2 {
                for _ in 0..10 {
                    input = hex_life(size, &input);
                }
            }
            let output = hex_life(size, &input);
            Example {
                size,
                input,
                output,
            }
        })
        .collect()
}

const HEX_LIFE_TABLES: &str = "[[[0,0,1,0,0,0,0],[0,0,0,1,1,0,0]]]";

fn faces(size: [u32; 3], input: &[u32], at: [u32; 3], axes: usize) -> u32 {
    let mut n = 0;
    for axis in 0..axes {
        for step in [-1_i64, 1] {
            let mut spot = at;
            spot[axis] =
                u32::try_from((i64::from(at[axis]) + step).rem_euclid(i64::from(size[axis])))
                    .unwrap();
            n += input[stencil::index(size, spot)] & 1;
        }
    }
    n
}

fn vn_square_step(size: [u32; 3], input: &[u32]) -> Vec<u32> {
    let mut out = vec![0_u32; input.len()];
    for z in 0..size[2] {
        for y in 0..size[1] {
            for x in 0..size[0] {
                out[stencil::index(size, [x, y, z])] =
                    u32::from(faces(size, input, [x, y, z], 2) >= 2);
            }
        }
    }
    out
}

fn vn_cube_step(size: [u32; 3], input: &[u32]) -> Vec<u32> {
    let mut out = vec![0_u32; input.len()];
    for z in 0..size[2] {
        for y in 0..size[1] {
            for x in 0..size[0] {
                let here = stencil::index(size, [x, y, z]);
                let n = faces(size, input, [x, y, z], 3) + (input[here] & 1);
                out[here] = u32::from(n >= 4);
            }
        }
    }
    out
}

fn stepped_pairs(
    rng: &mut Rng,
    size: [u32; 3],
    densities: &[u64],
    step: fn([u32; 3], &[u32]) -> Vec<u32>,
) -> Vec<Example> {
    densities
        .iter()
        .map(|permille| {
            let input = random_bits(rng, cells(size), *permille);
            let output = step(size, &input);
            Example {
                size,
                input,
                output,
            }
        })
        .collect()
}

fn hex_world(mcp: &mut Mcp<'_>, law: &str, seed: u64) {
    let size = [64, 64, 2];
    let layer = [64, 64, 1];
    let mut rng = Rng(seed);
    let mut image = random_bits(&mut rng, 64 * 64, 350);
    let wanted = hex_life(layer, &image);
    image.extend(vec![0_u32; 64 * 64]);
    let ran = mcp.ran(law, size, &image);
    for (at, want) in wanted.iter().enumerate() {
        assert_eq!(ran[at] & 1, *want, "world {seed} cell {at}");
    }
}

fn expected_never(
    rule: Rule,
    stencil: Stencil,
    mask: &[u32],
    examples: &[Example],
    wrapped: bool,
) -> String {
    let mut shown = vec![false; own_classes(rule, mask) as usize];
    for one in examples {
        for row in own_rows(stencil, one.size, &one.input, wrapped) {
            shown[own_class(rule, stencil, mask, row) as usize] = true;
        }
    }
    let counts = own_counts(rule, mask);
    format!(
        "[{}]",
        grouped(rule, mask, &|class| {
            (!shown[class as usize]).then(|| (class % counts).to_string())
        })
    )
}

#[test]
fn m1_a_hex_rule_learns_its_table_under_hex_places_and_steps_as_the_host() {
    let Some(host) = device("law_mask m1") else {
        return;
    };
    let mut mcp = Mcp::new(Some(&host));
    let examples = hex_pairs(TRAIN, 16, [32, 32, 1]);
    let got = mcp.ok(
        "grit_law_learn",
        &args(
            Stencil::Square,
            "outer",
            "wrapped",
            &examples,
            1,
            ",\"places\":\"hex\"",
        ),
    );
    assert_eq!(shown_of(&got, "tables"), HEX_LIFE_TABLES);
    assert_eq!(
        shown_of(&got, "unseen_counts"),
        expected_never(Rule::Outer, Stencil::Square, &HEX, &examples, true)
    );
    assert_eq!(
        shown_of(&got, "places"),
        "{\"list\":[1,2,3,4,5,6,7],\"preset\":\"hex\"}"
    );
    assert_eq!(int_at(got.get("examples").unwrap(), "conflicts"), 0);
    assert!(int_at(&got, "gates") <= 600);
    let law = text_at(&got, "law");
    for seed in [1, 2, 3] {
        hex_world(&mut mcp, &law, TRAIN + seed);
    }
    let told = mcp.no(
        "grit_law_learn",
        &args(Stencil::Square, "outer", "wrapped", &examples, 1, ""),
    );
    assert_eq!(text_at(&told, "refused"), "not_counting");
    assert_eq!(text_at(&told, "field"), "$.rule");
    let moore = mcp.ok(
        "grit_law_learn",
        &args(
            Stencil::Square,
            "outer",
            "wrapped",
            &examples,
            1,
            ",\"noise\":\"vote\"",
        ),
    );
    let deviating = words(moore.get("deviating").unwrap())[0];
    assert!(deviating > 0, "the 3x3 count cannot hold a hex rule");
    let hex = mcp.ok(
        "grit_law_learn",
        &args(
            Stencil::Square,
            "outer",
            "wrapped",
            &examples,
            1,
            ",\"places\":\"hex\",\"noise\":\"vote\"",
        ),
    );
    assert_eq!(shown_of(&hex, "deviating"), "[0]");
    assert_eq!(shown_of(&hex, "tables"), HEX_LIFE_TABLES);
    println!("m1: moore vote deviates on {deviating} of 16384 cells, hex on 0");
}

#[test]
fn m2_von_neumann_rules_learn_exact_tables_in_2d_and_3d_and_run_as_the_host() {
    let Some(host) = device("law_mask m2") else {
        return;
    };
    let mut mcp = Mcp::new(Some(&host));
    let mut rng = Rng(TRAIN + 2);
    let square = stepped_pairs(
        &mut rng,
        [32, 32, 1],
        &[200, 300, 400, 500, 600, 700],
        vn_square_step,
    );
    let got = mcp.ok(
        "grit_law_learn",
        &args(
            Stencil::Square,
            "outer",
            "wrapped",
            &square,
            1,
            ",\"places\":\"von_neumann\"",
        ),
    );
    assert_eq!(shown_of(&got, "tables"), "[[[0,0,1,1,1],[0,0,1,1,1]]]");
    assert_eq!(shown_of(&got, "unseen_counts"), "[[[],[]]]");
    assert_eq!(
        shown_of(&got, "places"),
        "{\"list\":[1,3,4,5,7],\"preset\":\"von_neumann\"}"
    );
    assert!(int_at(&got, "gates") <= 600);
    let size = [64, 64, 2];
    let mut image = random_bits(&mut rng, 64 * 64, 450);
    let wanted = vn_square_step([64, 64, 1], &image);
    image.extend(vec![0_u32; 64 * 64]);
    let ran = mcp.ran(&text_at(&got, "law"), size, &image);
    for (at, want) in wanted.iter().enumerate() {
        assert_eq!(ran[at] & 1, *want, "2d cell {at}");
    }
    let cube = stepped_pairs(
        &mut rng,
        [16, 16, 16],
        &[150, 300, 450, 550, 700, 850],
        vn_cube_step,
    );
    let got = mcp.ok(
        "grit_law_learn",
        &args(
            Stencil::Cube,
            "totalistic",
            "wrapped",
            &cube,
            1,
            ",\"places\":\"von_neumann\"",
        ),
    );
    assert_eq!(shown_of(&got, "tables"), "[[0,0,0,0,1,1,1,1]]");
    assert_eq!(shown_of(&got, "unseen_counts"), "[[]]");
    assert_eq!(
        shown_of(&got, "places"),
        "{\"list\":[4,10,12,13,14,16,22],\"preset\":\"von_neumann\"}"
    );
    assert!(int_at(&got, "gates") <= 600);
    let size = [32, 32, 16];
    let image = random_bits(&mut rng, cells(size), 500);
    let wanted = vn_cube_step(size, &image);
    let ran = mcp.ran(&text_at(&got, "law"), size, &image);
    for (at, want) in wanted.iter().enumerate() {
        assert_eq!(ran[at] & 1, *want, "3d cell {at}");
    }
}

fn random_counting(
    rng: &mut Rng,
    rule: Rule,
    stencil: Stencil,
    mask: &[u32],
    writes: u32,
) -> Vec<Example> {
    let table: Vec<u32> = (0..own_classes(rule, mask))
        .map(|_| rng.below(1 << writes))
        .collect();
    let size = match stencil {
        Stencil::Square => [32, 32, 1],
        Stencil::Cube => [16, 16, 16],
    };
    let inputs = random_inputs(rng, size, &[150, 350, 500, 650, 850]);
    labelled(stencil, inputs, true, &|row| {
        table[own_class(rule, stencil, mask, row) as usize]
    })
}

#[test]
fn m3_a_place_list_equal_to_a_preset_answers_the_same_law() {
    let mut mcp = Mcp::new(None);
    let cases: [(Stencil, &str, Vec<u32>, Rule); 6] = [
        (
            Stencil::Square,
            "moore",
            every(Stencil::Square),
            Rule::Outer,
        ),
        (
            Stencil::Square,
            "von_neumann",
            VN_SQUARE.to_vec(),
            Rule::Totalistic,
        ),
        (Stencil::Square, "hex", HEX.to_vec(), Rule::Outer),
        (Stencil::Square, "hex", HEX.to_vec(), Rule::Totalistic),
        (
            Stencil::Cube,
            "moore",
            every(Stencil::Cube),
            Rule::Totalistic,
        ),
        (Stencil::Cube, "von_neumann", VN_CUBE.to_vec(), Rule::Outer),
    ];
    for (at, (stencil, preset, mask, rule)) in cases.iter().enumerate() {
        let mut rng = Rng(TRAIN + 30 + at as u64);
        let examples = random_counting(&mut rng, *rule, *stencil, mask, 2);
        let mut turned = mask.clone();
        turned.reverse();
        turned.rotate_left(at % mask.len());
        let named = mcp.ok(
            "grit_law_learn",
            &args(
                *stencil,
                rule.name(),
                "wrapped",
                &examples,
                2,
                &format!(",\"places\":\"{preset}\""),
            ),
        );
        let listed_places = mcp.ok(
            "grit_law_learn",
            &args(
                *stencil,
                rule.name(),
                "wrapped",
                &examples,
                2,
                &format!(",\"place_list\":{}", listed(&turned)),
            ),
        );
        let case = format!("{} {preset} {}", stencil.name(), rule.name());
        for key in ["law", "hash", "chip", "tables", "unseen_counts", "unseen"] {
            assert_eq!(
                shown_of(&named, key),
                shown_of(&listed_places, key),
                "{case} {key}"
            );
        }
        assert_eq!(
            shown_of(&named, "places"),
            format!("{{\"list\":{},\"preset\":\"{preset}\"}}", listed(mask)),
            "{case}"
        );
        assert_eq!(
            shown_of(&listed_places, "places"),
            format!("{{\"list\":{}}}", listed(mask)),
            "{case}"
        );
        if *preset == "moore" {
            let bare = mcp.ok(
                "grit_law_learn",
                &args(*stencil, rule.name(), "wrapped", &examples, 2, ""),
            );
            for key in ["law", "hash", "chip", "tables", "unseen_counts"] {
                assert_eq!(shown_of(&named, key), shown_of(&bare, key), "{case} {key}");
            }
            assert!(bare.get("places").is_none());
        }
    }
}

fn refused_as(mcp: &mut Mcp<'_>, arguments: &str, code: &str, field: &str) {
    let told = mcp.no("grit_law_learn", arguments);
    assert_eq!(
        (text_at(&told, "refused"), text_at(&told, "field")),
        (code.to_owned(), field.to_owned()),
        "{}",
        written(&told)
    );
}

#[test]
fn m4_each_wrong_mask_or_support_is_refused_by_its_code_and_field() {
    let mut mcp = Mcp::new(None);
    let mut rng = Rng(TRAIN + 4);
    let square = random_counting(&mut rng, Rule::Outer, Stencil::Square, &HEX, 1);
    let cube = random_counting(&mut rng, Rule::Outer, Stencil::Cube, &VN_CUBE, 1);
    let flat = |rule: &str, more: &str| args(Stencil::Square, rule, "wrapped", &square, 1, more);
    for (arguments, code, field) in [
        (
            flat("outer", ",\"place_list\":[0,1,2,3]"),
            "places",
            "$.place_list",
        ),
        (
            flat("outer", ",\"place_list\":[1,4,4]"),
            "places",
            "$.place_list[2]",
        ),
        (
            flat("outer", ",\"place_list\":[4,9]"),
            "places",
            "$.place_list[1]",
        ),
        (
            flat("totalistic", ",\"places\":\"hex\",\"place_list\":[4]"),
            "places",
            "$.place_list",
        ),
        (
            args(
                Stencil::Cube,
                "outer",
                "wrapped",
                &cube,
                1,
                ",\"place_list\":[13,27]",
            ),
            "places",
            "$.place_list[1]",
        ),
        (
            args(
                Stencil::Cube,
                "outer",
                "wrapped",
                &cube,
                1,
                ",\"places\":\"hex\"",
            ),
            "places",
            "$.places",
        ),
        (
            flat("minimal", ",\"places\":\"hex\""),
            "places_rule",
            "$.places",
        ),
        (
            flat("symmetric", ",\"place_list\":[4]"),
            "places_rule",
            "$.place_list",
        ),
        (
            flat("outer", ",\"noise\":\"refuse\",\"min_support\":3"),
            "support_noise",
            "$.min_support",
        ),
        (
            flat("outer", ",\"min_support\":3"),
            "support_noise",
            "$.min_support",
        ),
        (
            flat("outer", ",\"noise\":\"vote\",\"min_support\":0"),
            "malformed",
            "$.min_support",
        ),
        (
            flat("outer", ",\"noise\":\"vote\",\"min_support\":577"),
            "support_limit",
            "$.min_support",
        ),
        (
            flat("outer", ",\"places\":\"ring\""),
            "malformed",
            "$.places",
        ),
    ] {
        refused_as(&mut mcp, &arguments, code, field);
    }
    let most = mcp.no(
        "grit_law_learn",
        &flat("outer", ",\"noise\":\"vote\",\"min_support\":577"),
    );
    assert_eq!(int_at(&most, "limit"), 576);
    mcp.ok(
        "grit_law_learn",
        &flat(
            "outer",
            ",\"noise\":\"vote\",\"min_support\":576,\"places\":\"hex\"",
        ),
    );
    mcp.ok(
        "grit_law_learn",
        &args(
            Stencil::Cube,
            "outer",
            "wrapped",
            &cube,
            1,
            ",\"noise\":\"vote\",\"min_support\":1728,\"place_list\":[4,10,12,13,14,16,22]",
        ),
    );
    let described = written(&mcp.ok("grit_describe", "{}"));
    for code in [
        "\"places\"",
        "\"places_rule\"",
        "\"support_noise\"",
        "\"support_limit\"",
    ] {
        assert!(described.contains(code), "{code}");
    }
}

fn unmasked_pattern(stencil: Stencil, size: [u32; 3], mask: &[u32]) -> (Vec<u32>, Vec<[u32; 3]>) {
    let mut pattern = vec![0_u32; cells(size)];
    let mut probes = Vec::new();
    let tiles: Vec<u32> = (0..3).map(|axis| size[axis] / 3).collect();
    let tall = if stencil == Stencil::Cube {
        tiles[2]
    } else {
        1
    };
    for tz in 0..tall {
        for ty in 0..tiles[1] {
            for tx in 0..tiles[0] {
                let z = if stencil == Stencil::Cube {
                    tz * 3 + 1
                } else {
                    0
                };
                let probe = [tx * 3 + 1, ty * 3 + 1, z];
                probes.push(probe);
                for place in (0..stencil.positions()).filter(|place| !mask.contains(place)) {
                    let offset = own_offset(stencil, place);
                    let at = [
                        u32::try_from(i64::from(probe[0]) + offset[0]).unwrap(),
                        u32::try_from(i64::from(probe[1]) + offset[1]).unwrap(),
                        u32::try_from(i64::from(probe[2]) + offset[2]).unwrap(),
                    ];
                    pattern[stencil::index(size, at)] ^= 1;
                }
            }
        }
    }
    (pattern, probes)
}

fn unread_checked(
    mcp: &mut Mcp<'_>,
    stencil: Stencil,
    size: [u32; 3],
    mask: &[u32],
    got: &Json,
    seed: u64,
    truth: &dyn Fn(u32) -> u32,
) {
    let law = text_at(got, "law");
    let mut rng = Rng(seed);
    let mut image = random_bits(&mut rng, cells(size), 450);
    if stencil == Stencil::Square {
        let layer = (size[0] * size[1]) as usize;
        for one in &mut image[layer..] {
            *one = 0;
        }
    }
    let (pattern, probes) = unmasked_pattern(stencil, size, mask);
    let flipped_image: Vec<u32> = image.iter().zip(&pattern).map(|(a, b)| a ^ b).collect();
    assert!(
        pattern.iter().filter(|one| **one == 1).count() >= probes.len(),
        "every probe has unmasked places flipped"
    );
    let rows = own_rows(stencil, size, &image, true);
    let rows_flipped = own_rows(stencil, size, &flipped_image, true);
    let plain = mcp.ran(&law, size, &image);
    let moved = mcp.ran(&law, size, &flipped_image);
    let mut changed_rows = 0;
    for probe in &probes {
        let at = stencil::index(size, *probe);
        changed_rows += usize::from(rows[at] != rows_flipped[at]);
        assert_eq!(plain[at] & 1, truth(rows[at]), "probe {probe:?}");
        assert_eq!(
            moved[at] & 1,
            plain[at] & 1,
            "probe {probe:?} read an unmasked place"
        );
    }
    assert!(
        changed_rows * 10 >= probes.len() * 9,
        "the flips reach the probes' rows"
    );
    let chip = gates_of(got.get("chip").unwrap().get("gates").unwrap());
    let unmasked: u32 = (0..stencil.positions())
        .filter(|place| !mask.contains(place))
        .fold(0, |held, place| held | (1 << place));
    let mut picks = Rng(seed + 1);
    for _ in 0..4096 {
        let row = picks.below(1 << stencil.positions());
        let other = row ^ (picks.below(1 << stencil.positions()) & unmasked);
        assert_eq!(
            stencil::answer(stencil.positions(), 1, &chip, row),
            stencil::answer(stencil.positions(), 1, &chip, other),
            "rows {row} and {other} differ only in unmasked places"
        );
    }
}

#[test]
fn m5_a_masked_law_does_not_read_its_unmasked_places() {
    let Some(host) = device("law_mask m5") else {
        return;
    };
    let mut mcp = Mcp::new(Some(&host));
    let hex = mcp.ok(
        "grit_law_learn",
        &args(
            Stencil::Square,
            "outer",
            "wrapped",
            &hex_pairs(TRAIN + 5, 8, [32, 32, 1]),
            1,
            ",\"places\":\"hex\"",
        ),
    );
    assert_eq!(shown_of(&hex, "tables"), HEX_LIFE_TABLES);
    let hex_truth = |row: u32| {
        let n = [1_u32, 2, 3, 5, 6, 7]
            .iter()
            .filter(|place| (row >> **place) & 1 == 1)
            .count();
        u32::from(if (row >> 4) & 1 == 1 {
            n == 3 || n == 4
        } else {
            n == 2
        })
    };
    unread_checked(
        &mut mcp,
        Stencil::Square,
        [64, 64, 2],
        &HEX,
        &hex,
        TRAIN + 50,
        &hex_truth,
    );
    let mut rng = Rng(TRAIN + 51);
    let cube = stepped_pairs(
        &mut rng,
        [16, 16, 16],
        &[200, 400, 500, 600, 800],
        vn_cube_step,
    );
    let vn = mcp.ok(
        "grit_law_learn",
        &args(
            Stencil::Cube,
            "totalistic",
            "wrapped",
            &cube,
            1,
            &format!(",\"place_list\":{}", listed(&VN_CUBE)),
        ),
    );
    let vn_truth = |row: u32| {
        u32::from(
            VN_CUBE
                .iter()
                .filter(|place| (row >> **place) & 1 == 1)
                .count()
                >= 4,
        )
    };
    unread_checked(
        &mut mcp,
        Stencil::Cube,
        [32, 32, 16],
        &VN_CUBE,
        &vn,
        TRAIN + 52,
        &vn_truth,
    );
}

struct Vote {
    tallies: Vec<Vec<[u64; 2]>>,
}

fn host_tallies(
    rule: Rule,
    stencil: Stencil,
    mask: &[u32],
    examples: &[Example],
    wrapped: bool,
    writes: u32,
) -> Vote {
    let mut tallies = vec![vec![[0_u64; 2]; own_classes(rule, mask) as usize]; writes as usize];
    for one in examples {
        let rows = own_rows(stencil, one.size, &one.input, wrapped);
        for (row, out) in rows.iter().zip(&one.output) {
            let class = own_class(rule, stencil, mask, *row) as usize;
            for (write, held) in tallies.iter_mut().enumerate() {
                held[class][((out >> write) & 1) as usize] += 1;
            }
        }
    }
    Vote { tallies }
}

fn thin_checked(got: &Json, rule: Rule, mask: &[u32], vote: &Vote, least: u64) -> usize {
    let counts = own_counts(rule, mask);
    let mut listed_thin = 0;
    let thin: Vec<String> = vote
        .tallies
        .iter()
        .map(|held| {
            grouped(rule, mask, &|class| {
                let [zeros, ones] = held[class as usize];
                let many = zeros + ones;
                (many > 0 && many < least).then(|| {
                    format!(
                        "{{\"count\":{},\"tally\":[{zeros},{ones}]}}",
                        class % counts
                    )
                })
            })
        })
        .collect();
    for held in &vote.tallies {
        listed_thin += held
            .iter()
            .filter(|[zeros, ones]| zeros + ones > 0 && zeros + ones < least)
            .count();
    }
    assert_eq!(shown_of(got, "thin"), format!("[{}]", thin.join(",")));
    let tables: Vec<String> = vote
        .tallies
        .iter()
        .map(|held| {
            grouped(rule, mask, &|class| {
                let [zeros, ones] = held[class as usize];
                Some(u32::from(ones > zeros).to_string())
            })
        })
        .collect();
    assert_eq!(
        shown_of(got, "tables"),
        format!("[{}]", tables.join(",")),
        "the table is the vote"
    );
    listed_thin
}

fn single_block() -> Vec<Example> {
    let size = [8, 8, 1];
    let mut input = vec![0_u32; 64];
    for y in 2..5 {
        for x in 2..5 {
            input[stencil::index(size, [x, y, 0])] = 1;
        }
    }
    let output = own_rows(Stencil::Square, size, &input, false)
        .iter()
        .map(|row| u32::from(row.count_ones() >= 5))
        .collect();
    vec![Example {
        size,
        input,
        output,
    }]
}

#[test]
fn m6_a_count_shown_by_few_cells_is_listed_thin_and_still_decided_by_the_vote() {
    let mut mcp = Mcp::new(None);
    let mut examples = single_block();
    let mut rng = Rng(TRAIN + 6);
    examples.extend(labelled(
        Stencil::Square,
        random_inputs(&mut rng, [16, 16, 1], &[150, 250]),
        false,
        &|row| u32::from(row.count_ones() >= 5),
    ));
    let mask = every(Stencil::Square);
    let vote = host_tallies(
        Rule::Totalistic,
        Stencil::Square,
        &mask,
        &examples,
        false,
        1,
    );
    assert_eq!(vote.tallies[0][9], [0, 1], "one cell shows count 9");
    let bare = mcp.ok(
        "grit_law_learn",
        &args(
            Stencil::Square,
            "totalistic",
            "zero",
            &examples,
            1,
            ",\"noise\":\"vote\"",
        ),
    );
    assert!(bare.get("thin").is_none());
    let got = mcp.ok(
        "grit_law_learn",
        &args(
            Stencil::Square,
            "totalistic",
            "zero",
            &examples,
            1,
            ",\"noise\":\"vote\",\"min_support\":3",
        ),
    );
    let many = thin_checked(&got, Rule::Totalistic, &mask, &vote, 3);
    assert!(
        shown_of(&got, "thin").contains("{\"count\":9,\"tally\":[0,1]}"),
        "{}",
        shown_of(&got, "thin")
    );
    assert!(many >= 1);
    assert_eq!(
        without(&got, "thin"),
        written(&bare),
        "thin changes nothing else"
    );
    let one = mcp.ok(
        "grit_law_learn",
        &args(
            Stencil::Square,
            "totalistic",
            "zero",
            &examples,
            1,
            ",\"noise\":\"vote\",\"min_support\":1",
        ),
    );
    assert_eq!(shown_of(&one, "thin"), "[[]]");
    let clean = hex_pairs(TRAIN + 60, 6, [16, 16, 1]);
    let mut flips = Rng(TRAIN + 61);
    let noisy: Vec<Example> = clean
        .iter()
        .map(|one| {
            let second = hex_step(one.size, &one.input, &[3, 4], &[1, 2, 3]);
            let output: Vec<u32> = one
                .output
                .iter()
                .zip(&second)
                .map(|(a, b)| a | (b << 1))
                .collect();
            let output = flipped(&mut flips, &output, 30);
            let high = flipped(&mut flips, &vec![0; output.len()], 30);
            Example {
                size: one.size,
                input: one.input.clone(),
                output: output
                    .iter()
                    .zip(&high)
                    .map(|(a, b)| a ^ (b << 1))
                    .collect(),
            }
        })
        .collect();
    let vote = host_tallies(Rule::Outer, Stencil::Square, &HEX, &noisy, true, 2);
    for least in [20_u32, 200] {
        let got = mcp.ok(
            "grit_law_learn",
            &args(
                Stencil::Square,
                "outer",
                "wrapped",
                &noisy,
                2,
                &format!(",\"places\":\"hex\",\"noise\":\"vote\",\"min_support\":{least}"),
            ),
        );
        let many = thin_checked(&got, Rule::Outer, &HEX, &vote, u64::from(least));
        println!("m6 hex outer min_support {least}: {many} thin entries over 2 writes");
        if least == 200 {
            assert!(many >= 2, "some hex counts rest on few cells");
        }
    }
}

struct Base {
    name: &'static str,
    arguments: String,
    error: bool,
}

fn majority_square(row: u32) -> u32 {
    u32::from((row & 0x1FF).count_ones() >= 5)
}

fn life(row: u32) -> u32 {
    let centre = (row >> 4) & 1;
    let around = (row & 0x1FF).count_ones() - centre;
    u32::from(around == 3 || (centre == 1 && around == 2))
}

fn base_requests() -> Vec<Base> {
    let mut rng = Rng(101);
    let densities = [300, 500, 700, 300, 500, 700, 300, 500, 700, 300, 500, 700];
    let a1 = labelled(
        Stencil::Square,
        random_inputs(&mut rng, [32, 32, 1], &densities),
        false,
        &majority_square,
    );
    let mut rng = Rng(202);
    let a2 = labelled(
        Stencil::Square,
        random_inputs(
            &mut rng,
            [32, 32, 1],
            &[250, 300, 350, 400, 450, 500, 600, 700],
        ),
        true,
        &life,
    );
    let mut rng = Rng(TRAIN + 7);
    let noisy: Vec<Example> = a2
        .iter()
        .map(|one| Example {
            size: one.size,
            input: one.input.clone(),
            output: flipped(&mut rng, &one.output, 10),
        })
        .collect();
    let mut rng = Rng(TRAIN + 8);
    let cube_table: Vec<u32> = (0..28).map(|_| rng.below(4)).collect();
    let cube = labelled(
        Stencil::Cube,
        random_inputs(&mut rng, [16, 16, 16], &[100, 300, 500, 700, 900]),
        true,
        &|row| cube_table[row.count_ones() as usize],
    );
    vec![
        Base {
            name: "a1",
            arguments: args(Stencil::Square, "totalistic", "zero", &a1, 1, ""),
            error: false,
        },
        Base {
            name: "a2",
            arguments: args(Stencil::Square, "outer", "wrapped", &a2, 1, ""),
            error: false,
        },
        Base {
            name: "not_counting",
            arguments: args(Stencil::Square, "totalistic", "wrapped", &a2, 1, ""),
            error: true,
        },
        Base {
            name: "vote",
            arguments: args(
                Stencil::Square,
                "outer",
                "wrapped",
                &noisy,
                1,
                ",\"noise\":\"vote\"",
            ),
            error: false,
        },
        Base {
            name: "conflict",
            arguments: args(Stencil::Square, "outer", "wrapped", &noisy, 1, ""),
            error: true,
        },
        Base {
            name: "cube",
            arguments: args(Stencil::Cube, "totalistic", "wrapped", &cube, 2, ""),
            error: false,
        },
    ]
}

#[test]
fn m7_requests_without_the_new_arguments_answer_the_base_bytes_and_every_answer_repeats() {
    let host = device("law_mask m7");
    let pinned: BTreeMap<&str, &str> = BASE_SHA.into_iter().collect();
    let mut mcp = Mcp::new(None);
    let mut seen = Vec::new();
    for one in base_requests() {
        let got = mcp.raw("grit_law_learn", &one.arguments);
        let told = result(&got, one.error);
        if one.error {
            let code = text_at(&told, "refused");
            assert_eq!(code, one.name, "{}", one.name);
        }
        let sha = sha_of(&got);
        println!("m7 {} {sha}", one.name);
        seen.push((one.name, sha));
    }
    for (name, sha) in &seen {
        assert_eq!(sha.as_str(), pinned[name], "{name} as on the base");
    }
    let mut examples = hex_pairs(TRAIN + 70, 4, [32, 32, 1]);
    let mut rng = Rng(TRAIN + 71);
    for one in &mut examples {
        one.output = flipped(&mut rng, &one.output, 20);
    }
    let cube = random_counting(
        &mut Rng(TRAIN + 72),
        Rule::Outer,
        Stencil::Cube,
        &VN_CUBE,
        2,
    );
    let mut requests: Vec<String> = base_requests()
        .into_iter()
        .filter(|one| !one.error)
        .map(|one| one.arguments)
        .collect();
    requests.extend([
        args(
            Stencil::Square,
            "outer",
            "wrapped",
            &examples,
            1,
            ",\"places\":\"hex\",\"noise\":\"vote\",\"min_support\":40",
        ),
        args(
            Stencil::Square,
            "totalistic",
            "zero",
            &examples,
            1,
            ",\"place_list\":[7,1,4,3,5],\"noise\":\"vote\"",
        ),
        args(
            Stencil::Cube,
            "outer",
            "wrapped",
            &cube,
            2,
            ",\"places\":\"von_neumann\"",
        ),
    ]);
    for arguments in &requests {
        let mut first = Mcp::new(host.as_ref());
        let one = written(&first.ok("grit_law_learn", arguments));
        let two = written(&first.ok("grit_law_learn", arguments));
        drop(first);
        let mut again = Mcp::new(host.as_ref());
        let three = written(&again.ok("grit_law_learn", arguments));
        let mut bare = Mcp::new(None);
        let four = written(&bare.ok("grit_law_learn", arguments));
        assert_eq!(one, two);
        assert_eq!(one, three, "after a restart");
        assert_eq!(one, four, "with no device");
    }
}

fn unseen_counted(got: &Json, writes: u32, case: &str) -> u64 {
    let unseen = got.get("unseen").unwrap();
    let table = words(unseen.get("table").unwrap());
    let rows = words(unseen.get("unseen_rows").unwrap());
    assert_eq!(table.len(), 512, "{case}");
    assert_eq!(int_at(unseen, "rows"), rows.len() as i64, "{case}");
    let ones: Vec<u32> = (0..writes)
        .map(|write| {
            rows.iter()
                .filter(|row| (table[**row as usize] >> write) & 1 == 1)
                .count() as u32
        })
        .collect();
    let zeros: Vec<u32> = ones.iter().map(|one| rows.len() as u32 - one).collect();
    assert_eq!(words(unseen.get("ones").unwrap()), ones, "{case} ones");
    assert_eq!(words(unseen.get("zeros").unwrap()), zeros, "{case} zeros");
    ones.iter().map(|one| u64::from(*one)).sum()
}

#[test]
fn m8_masked_unseen_ones_and_zeros_count_the_unseen_table() {
    let mut mcp = Mcp::new(None);
    let mut every_case = Vec::new();
    for (at, (preset, mask)) in [("hex", HEX.to_vec()), ("von_neumann", VN_SQUARE.to_vec())]
        .iter()
        .enumerate()
    {
        for (turn, rule) in [Rule::Totalistic, Rule::Outer].into_iter().enumerate() {
            let mut rng = Rng(TRAIN + 80 + 2 * at as u64 + turn as u64);
            let table: Vec<u32> = (0..own_classes(rule, mask))
                .map(|class| if class == 0 { 0 } else { rng.below(4) })
                .collect();
            let inputs = random_inputs(&mut rng, [8, 8, 1], &[300, 600]);
            let examples = labelled(Stencil::Square, inputs, true, &|row| {
                table[own_class(rule, Stencil::Square, mask, row) as usize]
            });
            let case = format!("{preset} {}", rule.name());
            let got = mcp.ok(
                "grit_law_learn",
                &args(
                    Stencil::Square,
                    rule.name(),
                    "wrapped",
                    &examples,
                    2,
                    &format!(",\"places\":\"{preset}\""),
                ),
            );
            let ones = unseen_counted(&got, 2, &case);
            let rows = int_at(got.get("unseen").unwrap(), "rows");
            println!("m8 {case}: {rows} unseen rows, {ones} of them 1 over both writes");
            assert!(rows > 64, "{case}: only {rows} rows unseen");
            every_case.push((case, ones));
        }
    }
    for (case, ones) in &every_case {
        assert!(
            *ones > 0,
            "{case}: no unseen row decided 1, so the count is not tested"
        );
    }
}

#[test]
fn m9_the_centre_alone_counts_one_place_and_an_explicit_moore_is_no_mask() {
    let mut mcp = Mcp::new(None);
    for rule in [Rule::Totalistic, Rule::Outer] {
        for flip in [0_u32, 1] {
            let mut rng = Rng(TRAIN + 90 + u64::from(flip) + 2 * (rule == Rule::Outer) as u64);
            let inputs = random_inputs(&mut rng, [32, 32, 1], &[200, 500, 800]);
            let truth = move |row: u32| ((row >> 4) & 1) ^ flip;
            let examples = labelled(Stencil::Square, inputs, true, &truth);
            let case = format!("centre {} flip {flip}", rule.name());
            let got = mcp.ok(
                "grit_law_learn",
                &args(
                    Stencil::Square,
                    rule.name(),
                    "wrapped",
                    &examples,
                    1,
                    ",\"place_list\":[4]",
                ),
            );
            let tables = match rule {
                Rule::Totalistic => format!("[[{},{}]]", flip, 1 - flip),
                Rule::Outer => format!("[[[{}],[{}]]]", flip, 1 - flip),
            };
            assert_eq!(shown_of(&got, "tables"), tables, "{case}");
            assert_eq!(shown_of(&got, "places"), "{\"list\":[4]}", "{case}");
            let never = match rule {
                Rule::Totalistic => "[[]]",
                Rule::Outer => "[[[],[]]]",
            };
            assert_eq!(shown_of(&got, "unseen_counts"), never, "{case}");
            let table = words(got.get("unseen").unwrap().get("table").unwrap());
            for (row, word) in table.iter().enumerate() {
                assert_eq!(*word, truth(row as u32), "{case} row {row}");
            }
            unseen_counted(&got, 1, &case);
        }
    }
    for (at, (stencil, rule)) in [
        (Stencil::Square, Rule::Totalistic),
        (Stencil::Square, Rule::Outer),
        (Stencil::Cube, Rule::Totalistic),
        (Stencil::Cube, Rule::Outer),
    ]
    .into_iter()
    .enumerate()
    {
        let mut rng = Rng(TRAIN + 95 + at as u64);
        let examples = random_counting(&mut rng, rule, stencil, &every(stencil), 2);
        for noise in ["", ",\"noise\":\"vote\""] {
            let case = format!("{} {} moore{noise}", stencil.name(), rule.name());
            let bare = mcp.ok(
                "grit_law_learn",
                &args(stencil, rule.name(), "wrapped", &examples, 2, noise),
            );
            let named = mcp.ok(
                "grit_law_learn",
                &args(
                    stencil,
                    rule.name(),
                    "wrapped",
                    &examples,
                    2,
                    &format!("{noise},\"places\":\"moore\""),
                ),
            );
            for key in ["law", "hash", "chip", "tables", "unseen_counts"] {
                assert_eq!(shown_of(&named, key), shown_of(&bare, key), "{case} {key}");
            }
            let (one, other) = (named.get("unseen").unwrap(), bare.get("unseen").unwrap());
            for key in ["rows", "rule", "zeros", "ones"] {
                assert_eq!(
                    shown_of(one, key),
                    shown_of(other, key),
                    "{case} unseen {key}"
                );
            }
            if !noise.is_empty() {
                for key in ["tallies", "ties", "deviating", "deviating_share"] {
                    assert_eq!(shown_of(&named, key), shown_of(&bare, key), "{case} {key}");
                }
            }
            assert_eq!(
                shown_of(&named, "places"),
                format!(
                    "{{\"list\":{},\"preset\":\"moore\"}}",
                    listed(&every(stencil))
                ),
                "{case}"
            );
        }
    }
}
