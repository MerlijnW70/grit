#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use std::collections::{BTreeMap, BTreeSet};

use grit::Host;
use grit::netlist::Gate;
use grit::serve3::Server;
use grit::stencil;
use grit::wire::{Json, read, written};

const TASK: &str = include_str!("law_learn/x0b_task.json");

const LONG: u64 = 600_000;

const CHUNK: usize = 4096;

const CENTRE: u32 = 4;

const NW: u32 = 0;

const NORTH: u32 = 1;

const WEST: u32 = 3;

const EAST: u32 = 5;

const SOUTH: u32 = 7;

const W9_IMAGES: usize = 3;

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

fn bool_at(held: &Json, key: &str) -> bool {
    match held.get(key) {
        Some(Json::Bool(one)) => *one,
        _ => panic!("no {key} in {}", written(held)),
    }
}

fn list_at<'held>(held: &'held Json, key: &str) -> &'held [Json] {
    held.get(key)
        .and_then(Json::list)
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

fn sha(text: &str) -> String {
    grit::law::hex(&grit::branch::sha256(text.as_bytes()))
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
        let got = self.ok(
            "grit_world_read",
            &format!(
                "{{\"world\":{world},\"plane\":1,\"from\":[0,0,0],\"to\":[{},{},0]}}",
                size[0] - 1,
                size[1] - 1
            ),
        );
        self.ok("grit_world_close", &format!("{{\"world\":{world}}}"));
        words(got.get("cells").unwrap())
    }
}

#[derive(Clone)]
struct Example {
    size: [u32; 3],
    input: Vec<u32>,
    output: Vec<u32>,
}

fn shown(examples: &[Example]) -> String {
    let shown: Vec<String> = examples
        .iter()
        .map(|one| {
            format!(
                "{{\"size\":[{},{},{}],\"input\":[{}],\"output\":[{}]}}",
                one.size[0],
                one.size[1],
                one.size[2],
                listed(&one.input),
                listed(&one.output)
            )
        })
        .collect();
    format!("[{}]", shown.join(","))
}

fn args(rule: &str, examples: &[Example], more: &str) -> String {
    format!(
        "{{\"stencil\":\"3x3\",\"reads\":[{{\"plane\":0,\"bit\":0}}],\"writes\":[{{\"plane\":1,\"bit\":0}}],\"border\":\"zero\",\"rule\":\"{rule}\",\"examples\":{},\"budget_ms\":{LONG}{more}}}",
        shown(examples)
    )
}

fn gates_text(gates: &[Gate]) -> String {
    let shown: Vec<String> = gates
        .iter()
        .map(|one| {
            format!(
                "{{\"kind\":{},\"left\":{},\"right\":{}}}",
                one.kind, one.left, one.right
            )
        })
        .collect();
    format!("[{}]", shown.join(","))
}

fn start_text(gates: &[Gate]) -> String {
    format!(
        ",\"start\":{{\"inputs\":9,\"outputs\":1,\"gates\":{}}}",
        gates_text(gates)
    )
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

fn chip_of(got: &Json) -> Vec<Gate> {
    gates_of(got.get("chip").unwrap().get("gates").unwrap())
}

fn own_answer(gates: &[Gate], row: u32) -> u32 {
    let mut values: Vec<u32> = (0..9).map(|input| (row >> input) & 1).collect();
    for gate in gates {
        let left = values[gate.left as usize];
        let right = values[gate.right as usize];
        values.push((gate.kind >> (left + 2 * right)) & 1);
    }
    *values.last().unwrap()
}

fn own_moves() -> Vec<[[i64; 2]; 2]> {
    let mut held = Vec::new();
    for swap in [false, true] {
        for sx in [1, -1] {
            for sy in [1, -1] {
                held.push(if swap {
                    [[0, sx], [sy, 0]]
                } else {
                    [[sx, 0], [0, sy]]
                });
            }
        }
    }
    held
}

fn own_image(row: u32, matrix: &[[i64; 2]; 2]) -> u32 {
    let mut out = 0;
    for place in 0..9_u32 {
        if (row >> place) & 1 == 0 {
            continue;
        }
        let at = [i64::from(place % 3) - 1, i64::from(place / 3) - 1];
        let to = [
            matrix[0][0] * at[0] + matrix[0][1] * at[1],
            matrix[1][0] * at[0] + matrix[1][1] * at[1],
        ];
        out |= 1 << ((to[1] + 1) * 3 + to[0] + 1);
    }
    out
}

fn own_or_of_images(gates: &[Gate], row: u32) -> u32 {
    u32::from(
        own_moves()
            .iter()
            .any(|matrix| own_answer(gates, own_image(row, matrix)) == 1),
    )
}

fn own_rows(size: [u32; 3], input: &[u32]) -> Vec<u32> {
    let mut rows = Vec::new();
    for y in 0..size[1] {
        for x in 0..size[0] {
            let mut row = 0_u32;
            for place in 0..9_u32 {
                let at = [
                    i64::from(x) + i64::from(place % 3) - 1,
                    i64::from(y) + i64::from(place / 3) - 1,
                ];
                if (0..i64::from(size[0])).contains(&at[0])
                    && (0..i64::from(size[1])).contains(&at[1])
                    && input[(at[1] as usize) * size[0] as usize + at[0] as usize] & 1 == 1
                {
                    row |= 1 << place;
                }
            }
            rows.push(row);
        }
    }
    rows
}

fn labelled(inputs: Vec<(Vec<u32>, [u32; 3])>, truth: &dyn Fn(u32) -> u32) -> Vec<Example> {
    inputs
        .into_iter()
        .map(|(input, size)| {
            let output = own_rows(size, &input)
                .iter()
                .map(|row| truth(*row))
                .collect();
            Example {
                size,
                input,
                output,
            }
        })
        .collect()
}

fn first_seen(examples: &[Example]) -> BTreeMap<u32, (u32, [u32; 3], u32)> {
    let mut held = BTreeMap::new();
    for (at, one) in examples.iter().enumerate() {
        let rows = own_rows(one.size, &one.input);
        for y in 0..one.size[1] {
            for x in 0..one.size[0] {
                let index = (y * one.size[0] + x) as usize;
                held.entry(rows[index]).or_insert((
                    u32::try_from(at).unwrap(),
                    [x, y, 0],
                    one.output[index] & 1,
                ));
            }
        }
    }
    held
}

fn random_bits(rng: &mut Rng, cells: usize, permille: u64) -> Vec<u32> {
    (0..cells)
        .map(|_| u32::from(u64::from(rng.below(1000)) < permille))
        .collect()
}

fn majority(row: u32) -> u32 {
    u32::from(row.count_ones() >= 5)
}

fn majority_inputs() -> Vec<(Vec<u32>, [u32; 3])> {
    let mut rng = Rng(101);
    let densities = [300, 500, 700, 300, 500, 700, 300, 500, 700, 300, 500, 700];
    let mut held: Vec<(Vec<u32>, [u32; 3])> = densities
        .iter()
        .map(|permille| (random_bits(&mut rng, 1024, *permille), [32, 32, 1]))
        .collect();
    let size = [256, 16, 1];
    for half in 0..2_u32 {
        let mut image = vec![0_u32; 4096];
        for at in 0..256_u32 {
            let row = half * 256 + at;
            let base = [(at % 64) * 4 + 1, (at / 64) * 4 + 1];
            for place in 0..9 {
                if (row >> place) & 1 == 1 {
                    let x = base[0] + place % 3 - 1;
                    let y = base[1] + place / 3 - 1;
                    image[(y * size[0] + x) as usize] = 1;
                }
            }
        }
        held.push((image, size));
    }
    held
}

fn x0b_inputs() -> Vec<(Vec<u32>, Vec<u32>)> {
    let task = read(TASK).unwrap();
    let flat = |held: &Json| -> Vec<u32> { held.list().unwrap().iter().flat_map(words).collect() };
    task.get("train")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .map(|pair| {
            (
                flat(pair.get("input").unwrap()),
                flat(pair.get("output").unwrap()),
            )
        })
        .collect()
}

fn x0b(many: usize) -> Vec<Example> {
    x0b_inputs()
        .into_iter()
        .take(many)
        .map(|(input, output)| Example {
            size: [32, 32, 1],
            input,
            output,
        })
        .collect()
}

fn edge(row: u32) -> u32 {
    let on = |place: u32| (row >> place) & 1 == 1;
    u32::from(on(CENTRE) && !(on(NORTH) && on(SOUTH) && on(EAST) && on(WEST)))
}

fn edge_nw(row: u32) -> u32 {
    let on = |place: u32| (row >> place) & 1 == 1;
    u32::from(on(CENTRE) && !(on(NORTH) && on(SOUTH) && on(EAST) && on(WEST) && on(NW)))
}

fn centre_copy() -> Vec<Gate> {
    vec![Gate {
        kind: 10,
        left: CENTRE,
        right: CENTRE,
    }]
}

fn three_sided() -> Vec<Gate> {
    vec![
        Gate {
            kind: 8,
            left: NORTH,
            right: SOUTH,
        },
        Gate {
            kind: 8,
            left: 9,
            right: EAST,
        },
        Gate {
            kind: 2,
            left: CENTRE,
            right: 10,
        },
    ]
}

fn seen_of(examples: &[Example]) -> BTreeMap<u32, u32> {
    first_seen(examples)
        .into_iter()
        .map(|(row, (_, _, out))| (row, out))
        .collect()
}

fn table_of(got: &Json) -> Vec<u32> {
    words(got.get("unseen").unwrap().get("table").unwrap())
}

fn answers_every_seen_row(got: &Json, seen: &BTreeMap<u32, u32>, what: &str) {
    let table = table_of(got);
    let chip = chip_of(got);
    for (row, out) in seen {
        assert_eq!(table[*row as usize], *out, "{what}: seen row {row}");
        assert_eq!(
            own_answer(&chip, *row),
            *out,
            "{what}: the chip on row {row}"
        );
    }
}

fn changed_checked(got: &Json, start_law: &dyn Fn(u32) -> u32, seen: &BTreeMap<u32, u32>) {
    let table = table_of(got);
    let start = got.get("start").unwrap();
    let changed: Vec<(u32, bool)> = (0..512_u32)
        .filter(|row| start_law(*row) != table[*row as usize])
        .map(|row| (row, seen.contains_key(&row)))
        .collect();
    let on_seen = changed.iter().filter(|(_, seen)| *seen).count();
    assert_eq!(int_at(start, "changed"), changed.len() as i64, "changed");
    assert_eq!(int_at(start, "changed_seen"), on_seen as i64);
    assert_eq!(
        int_at(start, "changed_unseen"),
        (changed.len() - on_seen) as i64
    );
    assert_eq!(text_at(start, "changed_over"), "every row");
    let mut order: Vec<(u32, bool)> = changed.clone();
    order.sort_by_key(|(row, seen)| (!*seen, *row));
    let shown: Vec<(u32, bool, u32, u32)> = list_at(start, "changed_rows")
        .iter()
        .map(|one| {
            (
                u32::try_from(int_at(one, "row")).unwrap(),
                bool_at(one, "seen"),
                u32::try_from(int_at(one, "start")).unwrap(),
                u32::try_from(int_at(one, "law")).unwrap(),
            )
        })
        .collect();
    let wanted: Vec<(u32, bool, u32, u32)> = order
        .iter()
        .take(64)
        .map(|(row, seen)| (*row, *seen, start_law(*row), table[*row as usize]))
        .collect();
    assert_eq!(shown, wanted, "changed rows, seen first");
}

fn wrong_checked(got: &Json, start_law: &dyn Fn(u32) -> u32, examples: &[Example]) -> usize {
    let start = got.get("start").unwrap();
    let first = first_seen(examples);
    let wrong: Vec<u32> = first
        .iter()
        .filter(|(row, (_, _, out))| start_law(**row) != *out)
        .map(|(row, _)| *row)
        .collect();
    assert_eq!(int_at(start, "wrong"), wrong.len() as i64, "wrong");
    let shown = list_at(start, "wrong_rows");
    assert_eq!(shown.len(), wrong.len().min(64));
    for (one, row) in shown.iter().zip(&wrong) {
        let (example, at, out) = first[row];
        assert_eq!(int_at(one, "row"), i64::from(*row));
        assert_eq!(int_at(one, "outputs"), i64::from(out), "row {row}");
        assert_eq!(int_at(one, "answered"), i64::from(start_law(*row)));
        assert_eq!(int_at(one, "example"), i64::from(example), "row {row}");
        assert_eq!(words(one.get("at").unwrap()), at, "row {row}");
    }
    assert_eq!(bool_at(start, "solved"), wrong.is_empty());
    wrong.len()
}

fn ladder_of(got: &Json) -> Vec<Json> {
    list_at(got, "ladder").to_vec()
}

fn majority_chip(mcp: &mut Mcp<'_>, truth: &dyn Fn(u32) -> u32) -> Vec<Gate> {
    let examples = labelled(majority_inputs(), truth);
    let got = mcp.ok("grit_law_learn", &args("totalistic", &examples, ""));
    chip_of(&got)
}

#[test]
fn w1_an_exact_majority_start_is_never_beaten_by_a_larger_law_and_walks_down_from_below_it() {
    let Some(host) = device("law_warm w1") else {
        return;
    };
    let mut mcp = Mcp::new(Some(&host));
    let start = majority_chip(&mut mcp, &majority);
    assert_eq!(start.len(), 49, "the totalistic majority law is 49 gates");
    let examples = labelled(majority_inputs(), &majority);
    let seen = seen_of(&examples);
    assert_eq!(seen.len(), 512, "the examples show every row");
    let got = mcp.ok(
        "grit_law_learn",
        &args(
            "minimal",
            &examples,
            &format!(",\"rounds\":8{}", start_text(&start)),
        ),
    );
    assert_eq!(text_at(&got, "stopped"), "ladder", "{}", written(&got));
    let found = got.get("found").unwrap();
    assert!(int_at(found, "live") <= 49, "{}", written(found));
    assert!(int_at(&got, "gates") <= 49);
    let table = table_of(&got);
    let wanted: Vec<u32> = (0..512).map(majority).collect();
    assert_eq!(table, wanted, "majority on all 512 rows");
    let ladder = ladder_of(&got);
    assert_eq!(int_at(&ladder[0], "gates"), 49);
    assert!(bool_at(&ladder[0], "start"));
    assert!(bool_at(&ladder[0], "solved"));
    assert_eq!(int_at(&ladder[1], "gates"), 48, "the walk starts below it");
    assert!(
        ladder
            .iter()
            .skip(1)
            .all(|rung| rung.get("start").is_none())
    );
    let start_told = got.get("start").unwrap();
    assert_eq!(int_at(start_told, "gates"), 49);
    assert_eq!(int_at(start_told, "wrong"), 0);
    assert!(bool_at(start_told, "solved"));
    changed_checked(&got, &majority, &seen);
}

#[test]
fn w1_a_start_that_misses_one_row_is_a_member_of_the_first_rungs_crowd() {
    let Some(host) = device("law_warm w1 sown") else {
        return;
    };
    let mut mcp = Mcp::new(Some(&host));
    let short = |row: u32| u32::from(majority(row) == 1 && row != 511);
    let start = majority_chip(&mut mcp, &short);
    for row in 0..512 {
        assert_eq!(own_answer(&start, row), short(row), "row {row}");
    }
    let examples = labelled(majority_inputs(), &majority);
    let arguments = args(
        "minimal",
        &examples,
        &format!(",\"crowd\":64,\"rounds\":1{}", start_text(&start)),
    );
    let raw = mcp.raw("grit_law_learn", &arguments);
    let failed = raw.get("result").and_then(|one| one.get("isError")) == Some(&Json::Bool(true));
    let got = result(&raw, failed);
    if failed {
        assert_eq!(text_at(&got, "refused"), "unsolved", "{}", written(&got));
    } else {
        assert_eq!(int_at(got.get("start").unwrap(), "wrong"), 1);
        assert_eq!(
            words(
                list_at(got.get("start").unwrap(), "wrong_rows")[0]
                    .get("at")
                    .unwrap()
            )
            .len(),
            3
        );
    }
    let ladder = ladder_of(&got);
    assert!(start.len() > 16);
    assert_eq!(
        int_at(&ladder[0], "gates"),
        start.len() as i64,
        "the ladder starts at the start's gates, past gates 16"
    );
    assert!(
        ladder[0].get("start").is_none(),
        "a start that misses is no rung"
    );
    assert_eq!(int_at(&ladder[0], "of"), 512);
    assert!(
        int_at(&ladder[0], "marks") >= 511,
        "the first rung keeps the start's 511 marks: {}",
        written(&ladder[0])
    );
}

#[test]
fn w2_a_law_learned_on_one_edge_rule_adjusts_to_another() {
    let Some(host) = device("law_warm w2") else {
        return;
    };
    let mut mcp = Mcp::new(Some(&host));
    let first = x0b(24);
    let learned = mcp.ok("grit_law_learn", &args("minimal", &first, ""));
    let old = chip_of(&learned);
    let old_law = |row: u32| own_answer(&old, row);
    for (row, out) in seen_of(&first) {
        assert_eq!(old_law(row), out, "A's law answers A on row {row}");
    }
    let second = labelled(
        x0b_inputs()
            .into_iter()
            .map(|(input, _)| (input, [32, 32, 1]))
            .collect(),
        &edge_nw,
    );
    let seen = seen_of(&second);
    let got = mcp.ok(
        "grit_law_learn",
        &args(
            "minimal",
            &second,
            &format!(
                ",\"gates\":7,\"rounds\":500,\"start\":{}",
                written(learned.get("chip").unwrap())
            ),
        ),
    );
    let wrong = wrong_checked(&got, &old_law, &second);
    let disagree: BTreeSet<u32> = seen
        .iter()
        .filter(|(row, out)| old_law(**row) != **out)
        .map(|(row, _)| *row)
        .collect();
    assert!(
        !disagree.is_empty(),
        "B differs from A's law on some seen rows"
    );
    assert_eq!(wrong, disagree.len());
    let named: BTreeSet<u32> = list_at(got.get("start").unwrap(), "wrong_rows")
        .iter()
        .map(|one| u32::try_from(int_at(one, "row")).unwrap())
        .collect();
    assert_eq!(named, disagree, "start.wrong names exactly those rows");
    answers_every_seen_row(&got, &seen, "B");
    let law = text_at(&got, "law");
    for (at, one) in second.iter().enumerate() {
        let size = [64, 64, 2];
        let mut image = vec![0_u32; 64 * 64 * 2];
        for y in 0..32 {
            for x in 0..32 {
                image[y * 64 + x] = one.input[y * 32 + x];
            }
        }
        let ran = mcp.ran(&law, size, &image);
        for y in 0..32 {
            for x in 0..32 {
                assert_eq!(
                    ran[y * 64 + x] & 1,
                    one.output[y * 32 + x],
                    "B example {at} cell ({x}, {y}) on a world"
                );
            }
        }
    }
    changed_checked(&got, &old_law, &seen);
    let changed_seen: BTreeSet<u32> = list_at(got.get("start").unwrap(), "changed_rows")
        .iter()
        .filter(|one| bool_at(one, "seen"))
        .map(|one| u32::try_from(int_at(one, "row")).unwrap())
        .collect();
    assert_eq!(changed_seen, disagree, "changed lists them");
}

#[test]
fn w4_each_start_refusal_by_its_code_and_field_before_any_device_work() {
    let mut none = Mcp::new(None);
    let blank = Example {
        size: [4, 4, 1],
        input: vec![0; 16],
        output: vec![0; 16],
    };
    let examples = [blank];
    let chain = |many: u32| -> Vec<Gate> {
        (0..many)
            .map(|at| Gate {
                kind: 10,
                left: if at == 0 { 0 } else { 8 + at },
                right: 0,
            })
            .collect()
    };
    let copy = gates_text(&centre_copy());
    let cases: Vec<(String, &str, &str)> = vec![
        (
            args(
                "minimal",
                &examples,
                &format!(",\"start\":{{\"inputs\":8,\"outputs\":1,\"gates\":{copy}}}"),
            ),
            "start_shape",
            "$.start",
        ),
        (
            args(
                "minimal",
                &examples,
                &format!(",\"start\":{{\"inputs\":9,\"outputs\":2,\"gates\":{copy}}}"),
            ),
            "start_shape",
            "$.start",
        ),
        (
            args(
                "symmetric",
                &examples,
                &format!(",\"start\":{{\"inputs\":18,\"outputs\":1,\"gates\":{copy}}}"),
            ),
            "start_shape",
            "$.start",
        ),
        (
            args(
                "minimal",
                &examples,
                &format!(",\"start\":{{\"inputs\":9,\"outputs\":1,\"state\":1,\"gates\":{copy}}}"),
            ),
            "start_shape",
            "$.start",
        ),
        (
            args(
                "minimal",
                &examples,
                ",\"start\":{\"inputs\":9,\"outputs\":1,\"gates\":[{\"kind\":8,\"left\":9,\"right\":0}]}",
            ),
            "chip_refused",
            "$.start.gates[0]",
        ),
        (
            args(
                "minimal",
                &examples,
                ",\"start\":{\"inputs\":9,\"outputs\":1,\"gates\":[{\"kind\":8,\"left\":1,\"right\":0},{\"kind\":16,\"left\":9,\"right\":2}]}",
            ),
            "chip_refused",
            "$.start.gates[1]",
        ),
        (
            args(
                "minimal",
                &examples,
                ",\"start\":{\"inputs\":9,\"outputs\":2,\"gates\":[{\"kind\":8,\"left\":1,\"right\":0}]}",
            ),
            "start_shape",
            "$.start",
        ),
        (
            args(
                "minimal",
                &examples,
                ",\"start\":{\"inputs\":9,\"outputs\":1}",
            ),
            "malformed",
            "$.start.gates",
        ),
        (
            args(
                "minimal",
                &examples,
                ",\"start\":{\"inputs\":9,\"outputs\":1,\"gates\":5}",
            ),
            "malformed",
            "$.start.gates",
        ),
        (
            args(
                "minimal",
                &examples,
                &format!(
                    ",\"start\":{{\"inputs\":9,\"outputs\":1,\"gates\":{copy},\"bind_inputs\":[]}}"
                ),
            ),
            "unknown_key",
            "$.start.bind_inputs",
        ),
        (
            args("totalistic", &examples, &start_text(&centre_copy())),
            "start_rule",
            "$.start",
        ),
        (
            args("outer", &examples, &start_text(&centre_copy())),
            "start_rule",
            "$.start",
        ),
        (
            args("minimal", &examples, &start_text(&chain(257))),
            "learn_limit",
            "$.start",
        ),
        (
            args("symmetric", &examples, &start_text(&chain(75))),
            "learn_limit",
            "$.start",
        ),
    ];
    for (arguments, code, field) in &cases {
        let told = none.no("grit_law_learn", arguments);
        assert_eq!(
            (text_at(&told, "refused"), text_at(&told, "field")),
            ((*code).to_owned(), (*field).to_owned()),
            "{}",
            written(&told)
        );
    }
    let told = none.no(
        "grit_law_learn",
        &args(
            "minimal",
            &examples,
            &format!(",\"start\":{{\"inputs\":8,\"outputs\":1,\"gates\":{copy}}}"),
        ),
    );
    assert_eq!((int_at(&told, "inputs"), int_at(&told, "outputs")), (9, 1));
    let told = none.no(
        "grit_law_learn",
        &args("minimal", &examples, &start_text(&chain(257))),
    );
    assert_eq!(int_at(&told, "limit"), 256);
    let told = none.no(
        "grit_law_learn",
        &args("symmetric", &examples, &start_text(&chain(75))),
    );
    assert_eq!(int_at(&told, "limit"), 74);
    let fits = none.no(
        "grit_law_learn",
        &args("symmetric", &examples, &start_text(&chain(74))),
    );
    assert_eq!(text_at(&fits, "refused"), "no_compute_device");
    let dead: Vec<Gate> = chain(300).into_iter().chain(centre_copy()).collect();
    let live = none.no(
        "grit_law_learn",
        &args("minimal", &examples, &start_text(&dead)),
    );
    assert_eq!(
        text_at(&live, "refused"),
        "no_compute_device",
        "only live gates count against the limit"
    );
}

#[test]
fn w5_a_symmetric_start_holds_by_the_or_of_its_images_or_seeds_the_search() {
    let Some(host) = device("law_warm w5") else {
        return;
    };
    let mut mcp = Mcp::new(Some(&host));
    let examples = x0b(8);
    let seen = seen_of(&examples);
    let start = three_sided();
    for (row, out) in &seen {
        assert_eq!(
            edge(*row),
            *out,
            "the X0b pairs are the edge law on row {row}"
        );
    }
    assert!(
        seen.iter()
            .any(|(row, out)| own_answer(&start, *row) != *out),
        "the circuit alone misses seen rows"
    );
    for (row, out) in &seen {
        assert_eq!(own_or_of_images(&start, *row), *out, "row {row}");
    }
    let got = mcp.ok(
        "grit_law_learn",
        &args("symmetric", &examples, &start_text(&start)),
    );
    assert_eq!(text_at(&got, "stopped"), "ladder", "{}", written(&got));
    let told = got.get("start").unwrap();
    assert!(bool_at(told, "solved"));
    assert_eq!(int_at(told, "wrong"), 0);
    assert_eq!(int_at(told, "images_wrong"), 0);
    assert_eq!(int_at(told, "gates"), 3);
    let ladder = ladder_of(&got);
    assert!(bool_at(&ladder[0], "start"));
    assert_eq!(int_at(&ladder[0], "gates"), 3);
    assert_eq!(int_at(&ladder[1], "gates"), 2);
    assert!(int_at(got.get("found").unwrap(), "live") <= 3);
    answers_every_seen_row(&got, &seen, "symmetric holds");
    let table = table_of(&got);
    for row in 0..512_u32 {
        for matrix in own_moves() {
            assert_eq!(table[own_image(row, &matrix) as usize], table[row as usize]);
        }
    }
    changed_checked(&got, &|row| own_or_of_images(&start, row), &seen);

    let copy = centre_copy();
    let mut plus = vec![0_u32; 64];
    for (x, y) in [(3, 3), (2, 3), (4, 3), (3, 2), (3, 4), (2, 2)] {
        plus[y * 8 + x] = 1;
    }
    let examples: Vec<Example> = examples
        .into_iter()
        .chain(labelled(vec![(plus, [8, 8, 1])], &edge))
        .collect();
    let seen = seen_of(&examples);
    let got = mcp.ok(
        "grit_law_learn",
        &args(
            "symmetric",
            &examples,
            &format!(",\"gates\":6,\"rounds\":500{}", start_text(&copy)),
        ),
    );
    let copy_law = |row: u32| own_or_of_images(&copy, row);
    let wrong = wrong_checked(&got, &copy_law, &examples);
    assert!(wrong > 0);
    let told = got.get("start").unwrap();
    let wrong_rows: Vec<u32> = seen
        .iter()
        .filter(|(row, out)| copy_law(**row) != **out)
        .map(|(row, _)| *row)
        .collect();
    let images: BTreeSet<u32> = wrong_rows
        .iter()
        .flat_map(|row| {
            own_moves()
                .into_iter()
                .map(move |matrix| own_image(*row, &matrix))
        })
        .collect();
    assert_eq!(int_at(told, "images_wrong"), images.len() as i64);
    assert_eq!(int_at(&ladder_of(&got)[0], "gates"), 6);
    answers_every_seen_row(&got, &seen, "symmetric seeded");
    changed_checked(&got, &copy_law, &seen);
    let forced: BTreeSet<u32> = images
        .iter()
        .copied()
        .filter(|row| !seen.contains_key(row))
        .collect();
    assert!(
        !forced.is_empty(),
        "some images of the wrong rows are unseen"
    );
    assert!(
        int_at(told, "changed_unseen") >= forced.len() as i64,
        "the symmetric law decides the unseen images of the wrong rows as their seen rows: {}",
        written(told)
    );
}

#[test]
fn w6_a_start_answers_the_same_bytes_twice_and_after_a_restart() {
    let Some(host) = device("law_warm w6") else {
        return;
    };
    let examples = x0b(8);
    let requests = [
        args("symmetric", &examples, &start_text(&three_sided())),
        args(
            "minimal",
            &examples,
            &format!(",\"gates\":6,\"rounds\":300{}", start_text(&centre_copy())),
        ),
    ];
    for arguments in &requests {
        let mut first = Mcp::new(Some(&host));
        let one = written(&first.ok("grit_law_learn", arguments));
        let two = written(&first.ok("grit_law_learn", arguments));
        drop(first);
        let mut again = Mcp::new(Some(&host));
        let three = written(&again.ok("grit_law_learn", arguments));
        assert!(one.contains("\"stopped\":\"ladder\""), "{one}");
        assert_eq!(one, two);
        assert_eq!(one, three, "after a restart");
    }
}

#[test]
fn w7_requests_without_start_answer_the_bytes_of_b806c05() {
    let Some(host) = device("law_warm w7") else {
        return;
    };
    let mut input = vec![0_u32; 64];
    let mut output = vec![0_u32; 64];
    input[2 * 8 + 2] = 1;
    input[2 * 8 + 3] = 1;
    output[2 * 8 + 2] = 1;
    let small = [Example {
        size: [8, 8, 1],
        input,
        output,
    }];
    let cases = [
        (
            args("minimal", &small, ""),
            "74e9e6acb88afad6d4dab85813eb4ae84ae71612d27686dee677db2bfe25dc4c",
        ),
        (
            args("symmetric", &x0b(8), ""),
            "1ff76127505b8e2a5368d4d365a8f734c9c191e66bb3f1af527f8162b7d40bd9",
        ),
        (
            args("minimal", &x0b(24), ""),
            "b46cc463ec767b1abb2c23b77de809b157906469a97b9595ede92d8290dba361",
        ),
    ];
    for (arguments, pinned) in &cases {
        let mut mcp = Mcp::new(Some(&host));
        let got = written(&mcp.ok("grit_law_learn", arguments));
        assert!(!got.contains("\"start\""));
        assert_eq!(sha(&got), *pinned, "{got}");
    }
}

fn parity_chain() -> Vec<Gate> {
    (0..8_u32)
        .map(|at| Gate {
            kind: 6,
            left: if at == 0 { 0 } else { 8 + at },
            right: at + 1,
        })
        .collect()
}

fn ran_first_rung(mcp: &mut Mcp<'_>, arguments: &str) -> Json {
    let raw = mcp.raw("grit_law_learn", arguments);
    let failed = raw.get("result").and_then(|one| one.get("isError")) == Some(&Json::Bool(true));
    let got = result(&raw, failed);
    if failed {
        assert_eq!(text_at(&got, "refused"), "unsolved", "{}", written(&got));
    }
    ladder_of(&got)[0].clone()
}

fn slot_marks(start: &dyn Fn(u32) -> u32, seen: &BTreeMap<u32, u32>) -> (u32, u32) {
    let rows: Vec<(u32, u32)> = seen.iter().map(|(row, out)| (*row, *out)).collect();
    let right = |(row, out): (u32, u32)| u32::from(start(row) == out);
    let slots = rows.len().div_ceil(32) * 32;
    let on_rows = rows.iter().copied().map(right).sum();
    let on_slots = (0..slots).map(|slot| right(rows[slot % rows.len()])).sum();
    (on_rows, on_slots)
}

#[test]
fn w8_a_sown_start_keeps_its_marks_through_a_few_rounds_at_a_small_crowd() {
    let Some(host) = device("law_warm w8") else {
        return;
    };
    let mut mcp = Mcp::new(Some(&host));
    let start = parity_chain();
    let parity = |row: u32| row.count_ones() & 1;
    for row in 0..512 {
        assert_eq!(own_answer(&start, row), parity(row), "row {row}");
    }
    let flipped = [383_u32, 447, 479, 503, 507, 509, 510, 511];
    let truth = |row: u32| parity(row) ^ u32::from(flipped.contains(&row));
    let examples = labelled(majority_inputs(), &truth);
    let seen = seen_of(&examples);
    assert_eq!(seen.len(), 512, "the examples show every row");
    let (right, _) = slot_marks(&|row| own_answer(&start, row), &seen);
    assert_eq!(right, 512 - flipped.len() as u32);
    for seed in [1, 2, 3] {
        let arguments = args(
            "minimal",
            &examples,
            &format!(
                ",\"gates\":8,\"crowd\":64,\"rounds\":2,\"seed\":{seed}{}",
                start_text(&start)
            ),
        );
        let first = ran_first_rung(&mut mcp, &arguments);
        println!("seed {seed}: first rung {}", written(&first));
        assert_eq!(int_at(&first, "gates"), 8, "seed {seed}");
        assert_eq!(int_at(&first, "of"), 512, "seed {seed}");
        assert!(
            int_at(&first, "marks") >= i64::from(right),
            "seed {seed}: the first rung keeps the start's {right} marks: {}",
            written(&first)
        );
    }
}

#[test]
fn w9_a_start_padded_into_more_gates_keeps_its_marks_over_wrapped_slots() {
    let Some(host) = device("law_warm w9") else {
        return;
    };
    let mut mcp = Mcp::new(Some(&host));
    let short = |row: u32| u32::from(majority(row) == 1 && row != 511);
    let start = majority_chip(&mut mcp, &short);
    let live = start.len();
    let inputs: Vec<(Vec<u32>, [u32; 3])> = majority_inputs().into_iter().take(W9_IMAGES).collect();
    let examples = labelled(inputs, &majority);
    let seen = seen_of(&examples);
    assert_ne!(
        seen.len() % 32,
        0,
        "{} seen rows leave slots to wrap",
        seen.len()
    );
    assert!(seen.contains_key(&511), "the start misses a seen row");
    let wrapped = seen.len().div_ceil(32) * 32 - seen.len();
    assert!(
        seen.iter()
            .take(wrapped)
            .all(|(row, out)| own_answer(&start, *row) == *out),
        "the start answers every wrapped row"
    );
    let (right, on_slots) = slot_marks(&|row| own_answer(&start, row), &seen);
    assert_eq!(right, u32::try_from(seen.len()).unwrap() - 1);
    assert_eq!(on_slots, right + u32::try_from(wrapped).unwrap());
    let gates = live + 13;
    for seed in [1, 2, 3, 4] {
        let arguments = args(
            "minimal",
            &examples,
            &format!(
                ",\"gates\":{gates},\"crowd\":64,\"rounds\":1,\"seed\":{seed}{}",
                start_text(&start)
            ),
        );
        let first = ran_first_rung(&mut mcp, &arguments);
        println!("seed {seed}: first rung {}", written(&first));
        assert_eq!(int_at(&first, "gates"), gates as i64, "seed {seed}");
        assert_eq!(int_at(&first, "of"), seen.len() as i64, "seed {seed}");
        assert!(
            int_at(&first, "marks") >= i64::from(right),
            "seed {seed}: the first rung keeps the padded start's {right} marks: {}",
            written(&first)
        );
    }
}

#[cfg(feature = "faults")]
mod faulted {
    use super::*;

    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use grit::clock::{Fake, Time};
    use grit::serve3::Options;
    use grit::serve3::law_learn::Point;
    use grit::serve3::law_learn::faults::{builds, cancel_after, deadline_passed_at, disarm};

    fn timed(host: &Host) -> (Mcp<'_>, Arc<Fake>) {
        let fake = Arc::new(Fake::new());
        let time: Arc<dyn Time> = fake.clone();
        let server = Server::timed(Some(host), &Options::default(), time).unwrap();
        (Mcp { server }, fake)
    }

    fn braked(host: &Host, arguments: &str, point: Point, ordinal: u64) -> (Json, u64) {
        let (mut mcp, fake) = timed(host);
        braked_on(&mut mcp, &fake, arguments, (point, ordinal))
    }

    fn braked_on(
        mcp: &mut Mcp<'_>,
        fake: &Arc<Fake>,
        arguments: &str,
        (point, ordinal): (Point, u64),
    ) -> (Json, u64) {
        disarm();
        deadline_passed_at(point, ordinal, Arc::clone(fake), LONG + 1);
        let got = mcp.raw("grit_law_learn", arguments);
        let built = builds();
        disarm();
        (got, built)
    }

    fn is_the_start(got: &Json, start: &[Gate], why: &str) {
        assert_eq!(text_at(got, "stopped"), why, "{}", written(got));
        assert!(got.get("hint").is_some());
        let ladder = ladder_of(got);
        assert_eq!(ladder.len(), 1, "{}", written(got));
        assert!(bool_at(&ladder[0], "start"));
        let found = got.get("found").unwrap();
        assert_eq!(gates_of(found.get("chip").unwrap()), start);
        assert_eq!(int_at(found, "gates"), start.len() as i64);
        assert_eq!(int_at(got.get("start").unwrap(), "changed"), 0);
    }

    #[test]
    fn w1_a_budget_that_stops_before_any_rung_answers_the_start_itself() {
        let Some(host) = device("law_warm w1 budget") else {
            return;
        };
        let mut mcp = Mcp::new(Some(&host));
        let start = majority_chip(&mut mcp, &majority);
        let examples = labelled(majority_inputs(), &majority);
        let arguments = args("minimal", &examples, &start_text(&start));
        let mut answers = Vec::new();
        for _ in 0..2 {
            let (got, built) = braked(&host, &arguments, Point::Calibration, 0);
            let got = result(&got, false);
            assert_eq!(built, 0, "no search kernel was built");
            is_the_start(&got, &start, "budget");
            assert_eq!(table_of(&got), (0..512).map(majority).collect::<Vec<_>>());
            answers.push(written(&got));
        }
        assert_eq!(
            answers[0], answers[1],
            "the start itself, the same bytes again"
        );
        let mut mcp = Mcp::new(Some(&host));
        let flag = Arc::new(AtomicBool::new(false));
        mcp.server.arriving(None, Some(Arc::clone(&flag)));
        disarm();
        cancel_after(0);
        let got = mcp.ok("grit_law_learn", &arguments);
        disarm();
        assert!(flag.load(Ordering::SeqCst));
        is_the_start(&got, &start, "cancelled");
    }

    #[test]
    fn w3_a_start_that_answers_every_seen_row_of_a_random_table_bounds_the_answer() {
        let Some(host) = device("law_warm w3") else {
            return;
        };
        let mut mcp = Mcp::new(Some(&host));
        for seed in 0..20_u64 {
            let mut rng = Rng(3000 + seed);
            let truth: Vec<u32> = (0..512).map(|_| rng.below(2)).collect();
            let input = random_bits(&mut rng, 16, 500);
            let examples = labelled(vec![(input.clone(), [4, 4, 1])], &|row| truth[row as usize]);
            let seen = seen_of(&examples);
            let start = minterms(&seen);
            for (row, out) in &seen {
                assert_eq!(own_answer(&start, *row), *out, "seed {seed}: the start");
            }
            let arguments = args(
                "minimal",
                &examples,
                &format!(",\"crowd\":64,\"rounds\":50{}", start_text(&start)),
            );
            let (got, _) = braked(&host, &arguments, Point::Build, 1);
            let got = result(&got, false);
            let told = got.get("start").unwrap();
            assert!(bool_at(told, "solved"), "seed {seed}");
            assert_eq!(int_at(told, "gates"), start.len() as i64);
            assert!(
                int_at(got.get("found").unwrap(), "live") <= start.len() as i64,
                "seed {seed}: {}",
                written(got.get("found").unwrap())
            );
            answers_every_seen_row(&got, &seen, &format!("seed {seed}"));
            let size = [8, 8, 2];
            let mut image = vec![0_u32; 128];
            for y in 0..4 {
                for x in 0..4 {
                    image[y * 8 + x] = input[y * 4 + x];
                }
            }
            let ran = mcp.ran(&text_at(&got, "law"), size, &image);
            for y in 0..4 {
                for x in 0..4 {
                    assert_eq!(
                        ran[y * 8 + x] & 1,
                        examples[0].output[y * 4 + x],
                        "seed {seed}: cell ({x}, {y}) on a world"
                    );
                }
            }
        }
    }

    fn minterms(seen: &BTreeMap<u32, u32>) -> Vec<Gate> {
        let mut gates: Vec<Gate> = Vec::new();
        let mut terms = Vec::new();
        for (row, _) in seen.iter().filter(|(_, out)| **out == 1) {
            let bit = |place: u32| (row >> place) & 1;
            gates.push(Gate {
                kind: 1 << (bit(0) + 2 * bit(1)),
                left: 0,
                right: 1,
            });
            for place in 2..9 {
                let last = 9 + u32::try_from(gates.len()).unwrap() - 1;
                gates.push(Gate {
                    kind: 1 << (1 + 2 * bit(place)),
                    left: last,
                    right: place,
                });
            }
            terms.push(9 + u32::try_from(gates.len()).unwrap() - 1);
        }
        match terms.split_first() {
            None => gates.push(Gate {
                kind: 0,
                left: 0,
                right: 0,
            }),
            Some((head, rest)) => {
                let mut held = *head;
                for one in rest {
                    gates.push(Gate {
                        kind: 14,
                        left: held,
                        right: *one,
                    });
                    held = 9 + u32::try_from(gates.len()).unwrap() - 1;
                }
            }
        }
        gates
    }

    #[test]
    fn w5_a_symmetric_start_that_holds_answers_itself_when_the_expansion_is_cancelled() {
        let Some(host) = device("law_warm w5 expansion") else {
            return;
        };
        let examples = x0b(8);
        let start = three_sided();
        let mut mcp = Mcp::new(Some(&host));
        let flag = Arc::new(AtomicBool::new(false));
        mcp.server.arriving(None, Some(Arc::clone(&flag)));
        disarm();
        cancel_after(0);
        let got = mcp.ok(
            "grit_law_learn",
            &args("symmetric", &examples, &start_text(&start)),
        );
        let built = builds();
        disarm();
        assert!(flag.load(Ordering::SeqCst));
        assert_eq!(built, 0);
        is_the_start(&got, &start, "cancelled");
        assert_eq!(int_at(got.get("start").unwrap(), "images_wrong"), 0);
        assert!(got.get("unseen").unwrap().get("by_symmetry").is_none());
        answers_every_seen_row(&got, &seen_of(&examples), "cancelled expansion");
        let mut mcp = Mcp::new(Some(&host));
        let flag = Arc::new(AtomicBool::new(false));
        mcp.server.arriving(None, Some(Arc::clone(&flag)));
        disarm();
        cancel_after(0);
        let told = mcp.no(
            "grit_law_learn",
            &args("symmetric", &examples, &start_text(&centre_copy())),
        );
        disarm();
        assert_eq!(text_at(&told, "refused"), "cancelled", "{}", written(&told));
        assert!(text_at(&told, "message").contains("expanding"));
    }

    #[test]
    fn w6_the_start_itself_answers_the_same_bytes_after_a_restart() {
        let Some(host) = device("law_warm w6 budget") else {
            return;
        };
        let examples = x0b(8);
        let arguments = args("symmetric", &examples, &start_text(&three_sided()));
        let mut answers = Vec::new();
        let (mut mcp, fake) = timed(&host);
        for _ in 0..2 {
            let (got, _) = braked_on(&mut mcp, &fake, &arguments, (Point::Calibration, 0));
            answers.push(result(&got, false));
        }
        drop(mcp);
        let (got, _) = braked(&host, &arguments, Point::Calibration, 0);
        answers.push(result(&got, false));
        for got in &answers {
            is_the_start(got, &three_sided(), "budget");
        }
        assert_eq!(
            written(&answers[0]),
            written(&answers[1]),
            "twice in one server"
        );
        assert_eq!(
            written(&answers[0]),
            written(&answers[2]),
            "after a restart"
        );
    }
}
