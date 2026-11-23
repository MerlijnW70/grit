#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use std::collections::{BTreeMap, BTreeSet};

use grit::Host;
use grit::netlist::Gate;
use grit::serve3::Server;
use grit::stencil::{self, Stencil, Tally};
use grit::wire::{Json, read, written};

const TASK: &str = include_str!("law_learn/x0b_task.json");

const LONG: u64 = 600_000;

const CHUNK: usize = 4096;

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

    fn write(&mut self, world: u32, plane: u32, cells: &[([u32; 3], u32)]) {
        for piece in cells.chunks(CHUNK) {
            let shown: Vec<String> = piece
                .iter()
                .map(|(at, word)| format!("[{},{},{},{word}]", at[0], at[1], at[2]))
                .collect();
            let got = self.ok(
                "grit_world_write",
                &format!(
                    "{{\"world\":{world},\"plane\":{plane},\"cells\":[{}],\"budget_ms\":{LONG}}}",
                    shown.join(",")
                ),
            );
            assert_eq!(int_at(&got, "written"), piece.len() as i64);
        }
    }

    fn step(&mut self, world: u32) {
        self.ok(
            "grit_world_step",
            &format!("{{\"world\":{world},\"frames\":1,\"budget_ms\":{LONG}}}"),
        );
    }

    fn read(&mut self, world: u32, plane: u32, size: [u32; 3]) -> Vec<u32> {
        let mut out = Vec::new();
        let rows = (CHUNK as u32 / size[0]).max(1);
        for z in 0..size[2] {
            let mut y = 0;
            while y < size[1] {
                let last = (y + rows).min(size[1]) - 1;
                let got = self.ok(
                    "grit_world_read",
                    &format!(
                        "{{\"world\":{world},\"plane\":{plane},\"from\":[0,{y},{z}],\"to\":[{},{last},{z}]}}",
                        size[0] - 1
                    ),
                );
                out.extend(words(got.get("cells").unwrap()));
                y = last + 1;
            }
        }
        out
    }

    fn close(&mut self, world: u32) {
        self.ok("grit_world_close", &format!("{{\"world\":{world}}}"));
    }

    fn ran(&mut self, law: &str, size: [u32; 3], input: &[u32]) -> Vec<u32> {
        let world = self.world(law, size);
        let blank = vec![0_u32; input.len()];
        let got = self.again(world, size, (&blank, input));
        self.close(world);
        got
    }

    fn again(&mut self, world: u32, size: [u32; 3], (before, input): (&[u32], &[u32])) -> Vec<u32> {
        let changed: Vec<([u32; 3], u32)> = cells_of(size, input)
            .into_iter()
            .filter(|(at, word)| before[stencil::index(size, *at)] != *word)
            .collect();
        self.write(world, 0, &changed);
        self.step(world);
        self.read(world, 1, size)
    }
}

fn cells_of(size: [u32; 3], held: &[u32]) -> Vec<([u32; 3], u32)> {
    let mut out = Vec::new();
    for z in 0..size[2] {
        for y in 0..size[1] {
            for x in 0..size[0] {
                out.push(([x, y, z], held[stencil::index(size, [x, y, z])]));
            }
        }
    }
    out
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

fn args(stencil: Stencil, rule: &str, border: &str, examples: &str, writes: u32) -> String {
    let shown: Vec<String> = (0..writes)
        .map(|bit| format!("{{\"plane\":1,\"bit\":{bit}}}"))
        .collect();
    format!(
        "{{\"stencil\":\"{}\",\"reads\":[{{\"plane\":0,\"bit\":0}}],\"writes\":[{}],\"border\":\"{border}\",\"rule\":\"{rule}\",\"examples\":{examples},\"budget_ms\":{LONG}}}",
        stencil.name(),
        shown.join(",")
    )
}

fn random_bits(rng: &mut Rng, cells: usize, permille: u64) -> Vec<u32> {
    (0..cells)
        .map(|_| u32::from(u64::from(rng.below(1000)) < permille))
        .collect()
}

fn own_centre(stencil: Stencil) -> u32 {
    match stencil {
        Stencil::Square => 4,
        Stencil::Cube => 13,
    }
}

fn own_counts(tally: Tally, stencil: Stencil) -> u32 {
    match tally {
        Tally::Totalistic => stencil.positions() + 1,
        Tally::Outer => stencil.positions(),
    }
}

fn own_class(tally: Tally, stencil: Stencil, row: u32) -> u32 {
    let all = row.count_ones();
    match tally {
        Tally::Totalistic => all,
        Tally::Outer => {
            let centre = (row >> own_centre(stencil)) & 1;
            centre * own_counts(tally, stencil) + all - centre
        }
    }
}

fn own_classes(tally: Tally, stencil: Stencil) -> u32 {
    match tally {
        Tally::Totalistic => own_counts(tally, stencil),
        Tally::Outer => 2 * own_counts(tally, stencil),
    }
}

fn own_rows(stencil: Stencil, size: [u32; 3], input: &[u32], wrapped: bool) -> Vec<u32> {
    let mut rows = Vec::new();
    for z in 0..size[2] {
        for y in 0..size[1] {
            for x in 0..size[0] {
                let mut row = 0_u32;
                for place in 0..stencil.positions() {
                    let p = i64::from(place);
                    let dz = if stencil == Stencil::Cube {
                        p / 9 - 1
                    } else {
                        0
                    };
                    let at = [
                        i64::from(x) + p % 3 - 1,
                        i64::from(y) + p / 3 % 3 - 1,
                        i64::from(z) + dz,
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

fn labelled(
    stencil: Stencil,
    inputs: Vec<(Vec<u32>, [u32; 3])>,
    wrapped: bool,
    truth: &dyn Fn(u32) -> u32,
) -> (Vec<Example>, BTreeMap<u32, u32>) {
    let mut seen = BTreeMap::new();
    let examples = inputs
        .into_iter()
        .map(|(input, size)| {
            let rows = own_rows(stencil, size, &input, wrapped);
            let output: Vec<u32> = rows.iter().map(|row| truth(*row)).collect();
            for (row, out) in rows.iter().zip(&output) {
                seen.insert(*row, *out);
            }
            Example {
                size,
                input,
                output,
            }
        })
        .collect();
    (examples, seen)
}

fn noisy(rng: &mut Rng, size: [u32; 3], densities: &[u64]) -> Vec<(Vec<u32>, [u32; 3])> {
    let cells = size.iter().product::<u32>() as usize;
    densities
        .iter()
        .map(|permille| (random_bits(rng, cells, *permille), size))
        .collect()
}

fn every_row_halves() -> Vec<(Vec<u32>, [u32; 3])> {
    let size = [256, 16, 1];
    (0..2_u32)
        .map(|half| {
            let mut image = vec![0_u32; 4096];
            for at in 0..256_u32 {
                let row = half * 256 + at;
                let base = [(at % 64) * 4 + 1, (at / 64) * 4 + 1];
                for place in 0..9 {
                    if (row >> place) & 1 == 1 {
                        let [dx, dy, _] = Stencil::Square.offset(place);
                        let x = u32::try_from(i64::from(base[0]) + dx).unwrap();
                        let y = u32::try_from(i64::from(base[1]) + dy).unwrap();
                        image[stencil::index(size, [x, y, 0])] = 1;
                    }
                }
            }
            (image, size)
        })
        .collect()
}

fn every_row_world() -> (Vec<u32>, [u32; 3]) {
    let size = [256, 32, 2];
    let mut image = vec![0_u32; 256 * 32 * 2];
    for (half, (one, _)) in every_row_halves().iter().enumerate() {
        for y in 0..16 {
            for x in 0..256 {
                image[stencil::index(size, [x, y + 16 * half as u32, 0])] =
                    one[stencil::index([256, 16, 1], [x, y, 0])];
            }
        }
    }
    (image, size)
}

fn every_row_centre(row: u32) -> [u32; 3] {
    [(row % 64) * 4 + 1, (row / 64) * 4 + 1, 0]
}

fn random_table(rng: &mut Rng, tally: Tally, stencil: Stencil, writes: u32) -> Vec<u32> {
    (0..own_classes(tally, stencil))
        .map(|_| rng.below(1 << writes))
        .collect()
}

fn expected_tables(
    tally: Tally,
    stencil: Stencil,
    writes: u32,
    table: &[u32],
    seen: &BTreeMap<u32, u32>,
) -> (String, String) {
    let shown_classes: BTreeSet<u32> = seen
        .keys()
        .map(|row| own_class(tally, stencil, *row))
        .collect();
    let counts = own_counts(tally, stencil);
    let group = |each: &dyn Fn(u32) -> Option<String>| -> String {
        let row = |from: u32| -> String {
            let held: Vec<String> = (from..from + counts).filter_map(each).collect();
            format!("[{}]", held.join(","))
        };
        match tally {
            Tally::Totalistic => row(0),
            Tally::Outer => format!("[{},{}]", row(0), row(counts)),
        }
    };
    let mut tables = Vec::new();
    let mut never = Vec::new();
    for write in 0..writes {
        tables.push(group(&|class| {
            let bit = if shown_classes.contains(&class) {
                (table[class as usize] >> write) & 1
            } else {
                0
            };
            Some(bit.to_string())
        }));
        never.push(group(&|class| {
            (!shown_classes.contains(&class)).then(|| (class % counts).to_string())
        }));
    }
    (
        format!("[{}]", tables.join(",")),
        format!("[{}]", never.join(",")),
    )
}

fn counted_truth(tally: Tally, stencil: Stencil, table: Vec<u32>) -> impl Fn(u32) -> u32 {
    move |row| table[own_class(tally, stencil, row) as usize]
}

fn checked_answer(got: &Json, tables: &str, never: &str) {
    assert_eq!(written(got.get("tables").unwrap()), tables, "tables");
    assert_eq!(
        written(got.get("unseen_counts").unwrap()),
        never,
        "unseen counts"
    );
    assert_eq!(text_at(got, "stopped"), "no_search");
    assert_eq!(written(got.get("ladder").unwrap()), "[]");
    assert_eq!(int_at(got.get("examples").unwrap(), "conflicts"), 0);
    for key in ["law", "stencil", "row_order", "border", "runs_as", "hash"] {
        text_at(got, key);
    }
}

fn majority_square(row: u32) -> u32 {
    u32::from((row & 0x1FF).count_ones() >= 5)
}

#[test]
fn a1_majority_from_twelve_examples_learns_its_table_and_runs_as_the_host() {
    let Some(host) = device("law_count a1") else {
        return;
    };
    let mut rng = Rng(101);
    let densities = [300, 500, 700, 300, 500, 700, 300, 500, 700, 300, 500, 700];
    let inputs = noisy(&mut rng, [32, 32, 1], &densities);
    let (examples, seen) = labelled(Stencil::Square, inputs, false, &majority_square);
    let mut mcp = Mcp::new(Some(&host));
    let got = mcp.ok(
        "grit_law_learn",
        &args(
            Stencil::Square,
            "totalistic",
            "zero",
            &shown(&examples, 1),
            1,
        ),
    );
    let table: Vec<u32> = (0..10).map(|n| u32::from(n >= 5)).collect();
    let (tables, never) = expected_tables(Tally::Totalistic, Stencil::Square, 1, &table, &seen);
    assert_eq!(tables, "[[0,0,0,0,0,1,1,1,1,1]]");
    checked_answer(&got, &tables, &never);
    assert_eq!(
        int_at(got.get("examples").unwrap(), "seen"),
        seen.len() as i64
    );
    let size = [64, 64, 2];
    let mut image = random_bits(&mut rng, 64 * 64, 500);
    image.extend(vec![0_u32; 64 * 64]);
    let ran = mcp.ran(&text_at(&got, "law"), size, &image);
    let rows = own_rows(Stencil::Square, size, &image, true);
    for (at, row) in rows.iter().enumerate().take(64 * 64) {
        assert_eq!(ran[at] & 1, majority_square(*row), "cell {at}");
    }
}

fn life(row: u32) -> u32 {
    let centre = (row >> 4) & 1;
    let around = (row & 0x1FF).count_ones() - centre;
    u32::from(around == 3 || (centre == 1 && around == 2))
}

#[test]
fn a2_life_learns_under_outer_and_steps_as_the_host() {
    let Some(host) = device("law_count a2") else {
        return;
    };
    let mut rng = Rng(202);
    let inputs = noisy(
        &mut rng,
        [32, 32, 1],
        &[250, 300, 350, 400, 450, 500, 600, 700],
    );
    let (examples, seen) = labelled(Stencil::Square, inputs, true, &life);
    let mut mcp = Mcp::new(Some(&host));
    let got = mcp.ok(
        "grit_law_learn",
        &args(Stencil::Square, "outer", "wrapped", &shown(&examples, 1), 1),
    );
    let table: Vec<u32> = (0..18)
        .map(|class| u32::from(class == 3 || class == 9 + 2 || class == 9 + 3))
        .collect();
    let (tables, never) = expected_tables(Tally::Outer, Stencil::Square, 1, &table, &seen);
    assert_eq!(
        written(got.get("tables").unwrap()),
        "[[[0,0,0,1,0,0,0,0,0],[0,0,1,1,0,0,0,0,0]]]"
    );
    checked_answer(&got, &tables, &never);
    let law = text_at(&got, "law");
    let size = [32, 32, 2];
    for (seed, permille) in [(1, 300), (2, 400), (3, 500)] {
        let mut world_rng = Rng(seed);
        let mut image = random_bits(&mut world_rng, 32 * 32, permille);
        image.extend(vec![0_u32; 32 * 32]);
        let ran = mcp.ran(&law, size, &image);
        let rows = own_rows(Stencil::Square, size, &image, true);
        for (at, row) in rows.iter().enumerate().take(32 * 32) {
            assert_eq!(ran[at] & 1, life(*row), "world {seed} cell {at}");
        }
    }
}

const SPREAD: [u64; 7] = [30, 150, 300, 500, 700, 850, 970];

const LAYERS: [u64; 25] = [
    970, 900, 800, 700, 600, 500, 450, 400, 350, 300, 250, 200, 150, 120, 100, 80, 60, 50, 40, 30,
    20, 15, 10, 5, 2,
];

fn majority_cube(row: u32) -> u32 {
    u32::from(row.count_ones() >= 14)
}

#[test]
fn a3_cube_majority_learns_under_totalistic_and_runs_as_the_host() {
    let Some(host) = device("law_count a3") else {
        return;
    };
    let mut rng = Rng(303);
    let inputs = noisy(&mut rng, [16, 16, 16], &SPREAD);
    let (examples, seen) = labelled(Stencil::Cube, inputs, true, &majority_cube);
    let classes: BTreeSet<u32> = seen.keys().map(|row| row.count_ones()).collect();
    assert_eq!(classes.len(), 28, "the examples show every count");
    let mut mcp = Mcp::new(Some(&host));
    let got = mcp.ok(
        "grit_law_learn",
        &args(
            Stencil::Cube,
            "totalistic",
            "wrapped",
            &shown(&examples, 1),
            1,
        ),
    );
    let table: Vec<u32> = (0..28).map(|n| u32::from(n >= 14)).collect();
    let (tables, never) = expected_tables(Tally::Totalistic, Stencil::Cube, 1, &table, &seen);
    assert_eq!(never, "[[]]");
    checked_answer(&got, &tables, &never);
    let size = [16, 16, 16];
    let image = random_bits(&mut rng, 4096, 500);
    let ran = mcp.ran(&text_at(&got, "law"), size, &image);
    let rows = own_rows(Stencil::Cube, size, &image, true);
    for (at, row) in rows.iter().enumerate() {
        assert_eq!(ran[at] & 1, majority_cube(*row), "cell {at}");
    }
}

#[test]
fn a4_erosion_and_dilation_learn_with_no_unseen_counts() {
    let mut mcp = Mcp::new(None);
    for (name, truth, table) in [
        (
            "erosion",
            (|row: u32| u32::from(row & 0x1FF == 0x1FF)) as fn(u32) -> u32,
            "[[0,0,0,0,0,0,0,0,0,1]]",
        ),
        (
            "dilation",
            (|row: u32| u32::from(row & 0x1FF != 0)) as fn(u32) -> u32,
            "[[0,1,1,1,1,1,1,1,1,1]]",
        ),
    ] {
        let (examples, seen) = labelled(Stencil::Square, every_row_halves(), false, &truth);
        assert_eq!(seen.len(), 512, "{name}");
        let got = mcp.ok(
            "grit_law_learn",
            &args(
                Stencil::Square,
                "totalistic",
                "zero",
                &shown(&examples, 1),
                1,
            ),
        );
        checked_answer(&got, table, "[[]]");
        let whole = words(got.get("unseen").unwrap().get("table").unwrap());
        let wanted: Vec<u32> = (0..512).map(truth).collect();
        assert_eq!(whole, wanted, "{name}");
        assert_eq!(int_at(got.get("unseen").unwrap(), "rows"), 0, "{name}");
    }
}

type Pairs = Vec<(Vec<u32>, Vec<u32>)>;

fn x0b() -> Pairs {
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

fn first_seen(
    examples: &[Example],
    wrapped: bool,
    stencil: Stencil,
) -> BTreeMap<u32, (u32, [u32; 3], u32)> {
    let mut held = BTreeMap::new();
    for (at, one) in examples.iter().enumerate() {
        let rows = own_rows(stencil, one.size, &one.input, wrapped);
        for z in 0..one.size[2] {
            for y in 0..one.size[1] {
                for x in 0..one.size[0] {
                    let index = stencil::index(one.size, [x, y, z]);
                    held.entry(rows[index]).or_insert((
                        u32::try_from(at).unwrap(),
                        [x, y, z],
                        one.output[index],
                    ));
                }
            }
        }
    }
    held
}

fn split_checked(
    told: &Json,
    tally: Tally,
    stencil: Stencil,
    examples: &[Example],
    wrapped: bool,
) -> usize {
    assert_eq!(
        text_at(told, "refused"),
        "not_counting",
        "{}",
        written(told)
    );
    assert_eq!(text_at(told, "field"), "$.rule");
    let first = first_seen(examples, wrapped, stencil);
    let counts = told.get("counts").and_then(Json::list).unwrap();
    assert!(!counts.is_empty());
    assert!(int_at(told, "not_counting") >= counts.len() as i64);
    for one in counts {
        let count = u32::try_from(int_at(one, "count")).unwrap();
        let centre = match tally {
            Tally::Totalistic => 0,
            Tally::Outer => u32::try_from(int_at(one, "centre")).unwrap(),
        };
        let write = u32::try_from(int_at(one, "write")).unwrap();
        let class = centre * own_counts(tally, stencil) + count;
        let seen = one.get("seen").and_then(Json::list).unwrap();
        assert_eq!(seen.len(), 2);
        for (output, shown) in seen.iter().enumerate() {
            assert_eq!(int_at(shown, "output"), output as i64);
            let row = u32::try_from(int_at(shown, "row")).unwrap();
            assert_eq!(own_class(tally, stencil, row), class, "row {row}");
            let (example, at, out) = first[&row];
            assert_eq!(int_at(shown, "example"), i64::from(example), "row {row}");
            assert_eq!(words(shown.get("at").unwrap()), at, "row {row}");
            assert_eq!((out >> write) & 1, output as u32, "row {row}");
        }
    }
    counts.len()
}

#[test]
fn a5_the_x0b_edge_pairs_are_refused_as_not_counting() {
    let train = x0b();
    let examples: Vec<Example> = train
        .into_iter()
        .map(|(input, output)| Example {
            size: [32, 32, 1],
            input,
            output,
        })
        .collect();
    let mut mcp = Mcp::new(None);
    for tally in [Tally::Totalistic, Tally::Outer] {
        let told = mcp.no(
            "grit_law_learn",
            &args(
                Stencil::Square,
                tally.name(),
                "zero",
                &shown(&examples, 1),
                1,
            ),
        );
        split_checked(&told, tally, Stencil::Square, &examples, false);
    }
}

#[test]
fn a6_each_refusal_by_its_code_and_field_and_unseen_counts_named() {
    let mut mcp = Mcp::new(None);
    let blank = format!(
        "[{{\"size\":[4,4,1],\"input\":[{z}],\"output\":[{z}]}}]",
        z = listed(&[0; 16])
    );
    for (stencil, rule) in [
        (Stencil::Square, "totalistic"),
        (Stencil::Square, "outer"),
        (Stencil::Cube, "totalistic"),
        (Stencil::Cube, "outer"),
    ] {
        let two = format!(
            "{{\"stencil\":\"{}\",\"reads\":[{{\"plane\":0,\"bit\":0}},{{\"plane\":0,\"bit\":1}}],\"writes\":[{{\"plane\":1,\"bit\":0}}],\"border\":\"zero\",\"rule\":\"{rule}\",\"examples\":{blank}}}",
            stencil.name()
        );
        let told = mcp.no("grit_law_learn", &two);
        assert_eq!(
            (text_at(&told, "refused"), text_at(&told, "field")),
            ("count_reads".to_owned(), "$.reads".to_owned()),
            "{}",
            written(&told)
        );
        assert_eq!(int_at(&told, "limit"), 1);
    }
    let size = [8, 8, 1];
    let mut input = vec![0_u32; 64];
    let mut output = vec![0_u32; 64];
    input[stencil::index(size, [2, 2, 0])] = 1;
    input[stencil::index(size, [3, 2, 0])] = 1;
    output[stencil::index(size, [2, 2, 0])] = 1;
    let examples = vec![Example {
        size,
        input,
        output,
    }];
    for tally in [Tally::Totalistic, Tally::Outer] {
        let told = mcp.no(
            "grit_law_learn",
            &args(
                Stencil::Square,
                tally.name(),
                "zero",
                &shown(&examples, 1),
                1,
            ),
        );
        split_checked(&told, tally, Stencil::Square, &examples, false);
        assert_eq!(int_at(&told, "not_counting"), 1, "{}", written(&told));
        let named = &told.get("counts").and_then(Json::list).unwrap()[0];
        let (count, centre) = match tally {
            Tally::Totalistic => (2, None),
            Tally::Outer => (1, Some(1)),
        };
        assert_eq!(int_at(named, "count"), count);
        assert_eq!(named.get("centre").and_then(Json::int), centre);
        let seen = named.get("seen").and_then(Json::list).unwrap();
        assert_eq!(int_at(&seen[1], "row"), 1 << 4 | 1 << 5);
        assert_eq!(words(seen[1].get("at").unwrap()), [2, 2, 0]);
    }
    let mut rng = Rng(606);
    let inputs = noisy(&mut rng, [8, 8, 1], &[100, 150]);
    let table: Vec<u32> = (0..18).map(|class| class % 2).collect();
    for tally in [Tally::Totalistic, Tally::Outer] {
        let truth = counted_truth(tally, Stencil::Square, table.clone());
        let (examples, seen) = labelled(Stencil::Square, inputs.clone(), false, &truth);
        let got = mcp.ok(
            "grit_law_learn",
            &args(
                Stencil::Square,
                tally.name(),
                "zero",
                &shown(&examples, 1),
                1,
            ),
        );
        let (tables, never) = expected_tables(tally, Stencil::Square, 1, &table, &seen);
        assert_ne!(never, "[[]]", "the examples miss some counts");
        assert_ne!(never, "[[[],[]]]", "the examples miss some counts");
        checked_answer(&got, &tables, &never);
        let unseen = got.get("unseen").unwrap();
        let whole = words(unseen.get("table").unwrap());
        let shown_classes: BTreeSet<u32> = seen
            .keys()
            .map(|row| own_class(tally, Stencil::Square, *row))
            .collect();
        let mut ones = 0;
        for row in 0..512_u32 {
            let class = own_class(tally, Stencil::Square, row);
            let wanted = if shown_classes.contains(&class) {
                table[class as usize]
            } else {
                0
            };
            assert_eq!(whole[row as usize], wanted, "{} row {row}", tally.name());
            if !seen.contains_key(&row) && wanted == 1 {
                ones += 1;
            }
        }
        assert_eq!(int_at(unseen, "rows"), 512 - seen.len() as i64);
        assert_eq!(words(unseen.get("ones").unwrap()), [ones]);
        assert_eq!(
            words(unseen.get("zeros").unwrap()),
            [512 - u32::try_from(seen.len()).unwrap() - ones]
        );
    }
}

fn turned(stencil: Stencil, size: [u32; 3], image: &[u32], places: &[u32]) -> Vec<u32> {
    let mut out = vec![0_u32; image.len()];
    let axes = stencil.axes();
    let mut map = [[0_i64; 3]; 3];
    for axis in 0..axes {
        let mut unit = [0_i64; 3];
        unit[axis] = 1;
        let from = stencil.place(unit);
        let to = stencil.offset(places[from as usize]);
        for (other, row) in map.iter_mut().enumerate() {
            row[axis] = to[other];
        }
    }
    for z in 0..size[2] {
        for y in 0..size[1] {
            for x in 0..size[0] {
                let at = [i64::from(x), i64::from(y), i64::from(z)];
                let mut to = [0_u32; 3];
                for other in 0..3 {
                    let value = if other < axes {
                        (0..axes)
                            .map(|axis| map[other][axis] * at[axis])
                            .sum::<i64>()
                    } else {
                        at[other]
                    };
                    to[other] = u32::try_from(value.rem_euclid(i64::from(size[other]))).unwrap();
                }
                out[stencil::index(size, to)] = image[stencil::index(size, [x, y, z])];
            }
        }
    }
    out
}

fn learned_random(
    mcp: &mut Mcp<'_>,
    tally: Tally,
    stencil: Stencil,
    writes: u32,
    seed: u64,
) -> (Json, Vec<u32>) {
    let mut rng = Rng(seed);
    let table = random_table(&mut rng, tally, stencil, writes);
    let truth = counted_truth(tally, stencil, table.clone());
    let (inputs, wrapped) = match stencil {
        Stencil::Square => (every_row_halves(), false),
        Stencil::Cube => (noisy(&mut rng, [16, 16, 16], &SPREAD), true),
    };
    let (examples, seen) = labelled(stencil, inputs, wrapped, &truth);
    let classes: BTreeSet<u32> = seen
        .keys()
        .map(|row| own_class(tally, stencil, *row))
        .collect();
    assert_eq!(
        classes.len() as u32,
        own_classes(tally, stencil),
        "the examples show every count"
    );
    let got = mcp.ok(
        "grit_law_learn",
        &args(
            stencil,
            tally.name(),
            if wrapped { "wrapped" } else { "zero" },
            &shown(&examples, writes),
            writes,
        ),
    );
    let (tables, never) = expected_tables(tally, stencil, writes, &table, &seen);
    checked_answer(&got, &tables, &never);
    (got, table)
}

#[test]
fn a7_a_counting_law_turns_with_its_input_on_a_world() {
    let Some(host) = device("law_count a7") else {
        return;
    };
    let mut mcp = Mcp::new(Some(&host));
    for (stencil, size) in [(Stencil::Square, [32, 32, 2]), (Stencil::Cube, [8, 8, 8])] {
        for (tally, seed) in [(Tally::Totalistic, 71), (Tally::Outer, 72)] {
            let (got, _) = learned_random(&mut mcp, tally, stencil, 2, seed);
            let law = text_at(&got, "law");
            let cells = size.iter().product::<u32>() as usize;
            let mut rng = Rng(seed + 100);
            let mut image = random_bits(&mut rng, cells, 450);
            if stencil == Stencil::Square {
                for one in image.iter_mut().skip(cells / 2) {
                    *one = 0;
                }
            }
            let world = mcp.world(&law, size);
            let blank = vec![0_u32; cells];
            let base = mcp.again(world, size, (&blank, &image));
            let mut held = image.clone();
            for (at, places) in stencil.symmetries().iter().enumerate() {
                let moved = turned(stencil, size, &image, places);
                let out = mcp.again(world, size, (&held, &moved));
                held = moved;
                assert_eq!(
                    out,
                    turned(stencil, size, &base, places),
                    "{} {}: move {at}",
                    stencil.name(),
                    tally.name()
                );
            }
            mcp.close(world);
        }
    }
}

#[test]
fn a8_the_same_request_answers_the_same_bytes_after_a_restart() {
    let host = device("law_count a8");
    let mut rng = Rng(808);
    let square = noisy(&mut rng, [32, 32, 1], &[300, 500, 700]);
    let (square, _) = labelled(Stencil::Square, square, false, &majority_square);
    let cube = noisy(&mut rng, [16, 16, 16], &SPREAD);
    let table = random_table(&mut rng, Tally::Outer, Stencil::Cube, 8);
    let (cube, _) = labelled(
        Stencil::Cube,
        cube,
        true,
        &counted_truth(Tally::Outer, Stencil::Cube, table),
    );
    let requests = [
        args(Stencil::Square, "totalistic", "zero", &shown(&square, 1), 1),
        args(Stencil::Square, "outer", "zero", &shown(&square, 1), 1),
        args(Stencil::Cube, "outer", "wrapped", &shown(&cube, 8), 8),
    ];
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

fn host_checked(
    tally: Tally,
    stencil: Stencil,
    writes: u32,
    table: &[u32],
    gates: &[Gate],
    rows: &mut dyn Iterator<Item = u32>,
) -> usize {
    let mut many = 0;
    for row in rows {
        let wanted = table[own_class(tally, stencil, row) as usize] & ((1 << writes) - 1);
        assert_eq!(
            stencil::answer(stencil.positions(), writes, gates, row),
            wanted,
            "{} {} row {row}",
            stencil.name(),
            tally.name()
        );
        many += 1;
    }
    many
}

#[test]
fn g1_the_counting_circuit_equals_its_table_on_the_host_and_fits_a_law() {
    for tally in [Tally::Totalistic, Tally::Outer] {
        for class in 0..own_classes(tally, Stencil::Square) {
            let many = (0..512_u32)
                .filter(|row| own_class(tally, Stencil::Square, *row) == class)
                .count() as u64;
            assert_eq!(tally.rows_in(Stencil::Square, class), many);
        }
        let all: u64 = (0..own_classes(tally, Stencil::Cube))
            .map(|class| tally.rows_in(Stencil::Cube, class))
            .sum();
        assert_eq!(all, 1 << 27);
        for seed in 0..20 {
            let mut rng = Rng(900 + seed);
            let table = random_table(&mut rng, tally, Stencil::Square, 3);
            let gates = stencil::tally_gates(tally, Stencil::Square, 3, &table);
            assert_eq!(
                host_checked(tally, Stencil::Square, 3, &table, &gates, &mut (0..512)),
                512
            );
            let cube = random_table(&mut rng, tally, Stencil::Cube, 8);
            let gates = stencil::tally_gates(tally, Stencil::Cube, 8, &cube);
            assert!(gates.len() <= 600, "{} gates", gates.len());
            let mut picked = Vec::new();
            for class in 0..own_classes(tally, Stencil::Cube) {
                let counts = own_counts(tally, Stencil::Cube);
                let (centre, n) = (class / counts, class % counts);
                let mut row = if tally == Tally::Outer {
                    centre << 13
                } else {
                    0
                };
                let mut set = 0;
                let mut place = 0;
                while set < n {
                    if tally == Tally::Totalistic || place != 13 {
                        row |= 1 << place;
                        set += 1;
                    }
                    place += 1;
                }
                picked.push(row);
            }
            for _ in 0..2000 {
                picked.push(rng.below(1 << 27));
            }
            host_checked(
                tally,
                Stencil::Cube,
                8,
                &cube,
                &gates,
                &mut picked.into_iter(),
            );
        }
    }
}

#[test]
fn r8_a_counting_law_equals_its_table_on_every_row_on_a_world() {
    let Some(host) = device("law_count r8") else {
        return;
    };
    let mut mcp = Mcp::new(Some(&host));
    for (tally, seed) in [(Tally::Totalistic, 81), (Tally::Outer, 82)] {
        let (got, table) = learned_random(&mut mcp, tally, Stencil::Square, 3, seed);
        let (image, size) = every_row_world();
        let ran = mcp.ran(&text_at(&got, "law"), size, &image);
        let whole = words(got.get("unseen").unwrap().get("table").unwrap());
        let chip = gates_of(got.get("chip").unwrap().get("gates").unwrap());
        assert_eq!(stencil::table(9, 3, &chip), whole, "the chip");
        for row in 0..512_u32 {
            let wanted = table[own_class(tally, Stencil::Square, row) as usize];
            assert_eq!(
                ran[stencil::index(size, every_row_centre(row))] & 7,
                wanted,
                "{} row {row}",
                tally.name()
            );
            assert_eq!(
                whole[row as usize],
                wanted,
                "{} table row {row}",
                tally.name()
            );
        }
    }
    for (tally, seed) in [(Tally::Totalistic, 83), (Tally::Outer, 84)] {
        let (got, table) = learned_random(&mut mcp, tally, Stencil::Cube, 8, seed);
        assert!(int_at(&got, "gates") <= 600);
        let size = [64, 64, 32];
        let mut rng = Rng(seed + 1000);
        let mut image = Vec::with_capacity(64 * 64 * 32);
        for permille in LAYERS {
            image.extend(random_bits(&mut rng, 64 * 64, permille));
        }
        assert!(image.len() >= 100_000, "seeded random rows");
        image.resize(64 * 64 * 32, 0);
        let rows = own_rows(Stencil::Cube, size, &image, true);
        let classes: BTreeSet<u32> = rows
            .iter()
            .map(|row| own_class(tally, Stencil::Cube, *row))
            .collect();
        assert_eq!(
            classes.len() as u32,
            own_classes(tally, Stencil::Cube),
            "the world holds every count class"
        );
        let ran = mcp.ran(&text_at(&got, "law"), size, &image);
        for (at, row) in rows.iter().enumerate() {
            assert_eq!(
                ran[at] & 0xFF,
                table[own_class(tally, Stencil::Cube, *row) as usize],
                "{} cell {at}",
                tally.name()
            );
        }
    }
}

fn refusal(told: &Json, code: &str, field: &str) {
    assert_eq!(
        (text_at(told, "refused"), text_at(told, "field")),
        (code.to_owned(), field.to_owned()),
        "{}",
        written(told)
    );
}

fn lone_pixel(writes: &dyn Fn([u32; 3]) -> u32) -> Example {
    let size = [8, 8, 1];
    let mut input = vec![0_u32; 64];
    input[stencil::index(size, [4, 4, 0])] = 1;
    let mut output = vec![0_u32; 64];
    for y in 0..8 {
        for x in 0..8 {
            output[stencil::index(size, [x, y, 0])] = writes([x, y, 0]);
        }
    }
    Example {
        size,
        input,
        output,
    }
}

fn seen_entry(held: &Json) -> (i64, i64, i64, Vec<u32>) {
    (
        int_at(held, "output"),
        int_at(held, "row"),
        int_at(held, "example"),
        words(held.get("at").unwrap()),
    )
}

#[test]
fn c1_a_row_seen_with_both_outputs_is_refused_as_a_conflict_under_counting_rules() {
    let mut mcp = Mcp::new(None);
    for (stencil, size, at) in [
        (Stencil::Square, [8, 8, 1], [3, 5, 0]),
        (Stencil::Cube, [4, 4, 4], [2, 1, 3]),
    ] {
        let cells = size.iter().product::<u32>() as usize;
        let mut output = vec![0_u32; cells];
        output[stencil::index(size, at)] = 1;
        let examples = vec![Example {
            size,
            input: vec![0; cells],
            output,
        }];
        for tally in [Tally::Totalistic, Tally::Outer] {
            let told = mcp.no(
                "grit_law_learn",
                &args(stencil, tally.name(), "zero", &shown(&examples, 1), 1),
            );
            refusal(&told, "conflict", "$.examples");
            assert_eq!(int_at(&told, "conflicts"), 1, "{}", written(&told));
            let rows = told.get("rows").and_then(Json::list).unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(int_at(&rows[0], "row"), 0);
            let seen: Vec<(i64, i64, Vec<u32>)> = rows[0]
                .get("seen")
                .and_then(Json::list)
                .unwrap()
                .iter()
                .map(|one| {
                    (
                        int_at(one, "outputs"),
                        int_at(one, "example"),
                        words(one.get("at").unwrap()),
                    )
                })
                .collect();
            assert_eq!(
                seen,
                [(0, 0, vec![0, 0, 0]), (1, 0, at.to_vec())],
                "{} {}",
                stencil.name(),
                tally.name()
            );
        }
    }
}

#[test]
fn c2_a_count_split_only_in_the_second_write_is_refused_as_not_counting() {
    let mut mcp = Mcp::new(None);
    let near = |at: [u32; 3]| at[0].abs_diff(4) <= 1 && at[1].abs_diff(4) <= 1;
    let examples = vec![lone_pixel(&|at| {
        u32::from(near(at)) | u32::from(at == [5, 4, 0]) << 1
    })];
    for tally in [Tally::Totalistic, Tally::Outer] {
        let told = mcp.no(
            "grit_law_learn",
            &args(
                Stencil::Square,
                tally.name(),
                "zero",
                &shown(&examples, 2),
                2,
            ),
        );
        split_checked(&told, tally, Stencil::Square, &examples, false);
        assert_eq!(int_at(&told, "not_counting"), 1, "{}", written(&told));
        let named = &told.get("counts").and_then(Json::list).unwrap()[0];
        assert_eq!(int_at(named, "write"), 1, "{}", written(&told));
        assert_eq!(int_at(named, "count"), 1);
        let centre = (tally == Tally::Outer).then_some(0);
        assert_eq!(named.get("centre").and_then(Json::int), centre);
    }
}

#[test]
fn c3_the_first_seen_row_of_a_count_is_the_one_listed_for_each_output() {
    let mut mcp = Mcp::new(None);
    let examples = vec![lone_pixel(&|at| u32::from(at == [5, 4, 0]))];
    let first = first_seen(&examples, false, Stencil::Square);
    assert_eq!(first[&256].1, [3, 3, 0]);
    assert_eq!(first[&1].1, [5, 5, 0]);
    for tally in [Tally::Totalistic, Tally::Outer] {
        let told = mcp.no(
            "grit_law_learn",
            &args(
                Stencil::Square,
                tally.name(),
                "zero",
                &shown(&examples, 1),
                1,
            ),
        );
        split_checked(&told, tally, Stencil::Square, &examples, false);
        assert_eq!(int_at(&told, "not_counting"), 1, "{}", written(&told));
        let named = &told.get("counts").and_then(Json::list).unwrap()[0];
        let seen: Vec<(i64, i64, i64, Vec<u32>)> = named
            .get("seen")
            .and_then(Json::list)
            .unwrap()
            .iter()
            .map(seen_entry)
            .collect();
        assert_eq!(
            seen,
            [(0, 256, 0, vec![3, 3, 0]), (1, 8, 0, vec![5, 4, 0])],
            "{}: the row of each output is the one seen first in the examples",
            tally.name()
        );
    }
}

#[test]
fn c4_planes_and_bits_are_refused_by_name_under_counting_rules() {
    let mut mcp = Mcp::new(None);
    let blank = |writes: u32| {
        shown(
            &[Example {
                size: [4, 4, 1],
                input: vec![0; 16],
                output: vec![0; 16],
            }],
            writes,
        )
    };
    let request = |rule: &str, reads: &str, writes: &str, count: u32, more: &str| {
        format!(
            "{{\"stencil\":\"3x3\",\"reads\":{reads},\"writes\":{writes},\"border\":\"zero\",\"rule\":\"{rule}\",\"examples\":{}{more}}}",
            blank(count)
        )
    };
    for tally in [Tally::Totalistic, Tally::Outer] {
        let rule = tally.name();
        let cases = [
            (
                request(
                    rule,
                    "[{\"plane\":2,\"bit\":0}]",
                    "[{\"plane\":1,\"bit\":0}]",
                    1,
                    ",\"channels\":2",
                ),
                "plane_past",
                "$.reads[0].plane",
                Some(1),
            ),
            (
                request(
                    rule,
                    "[{\"plane\":0,\"bit\":0}]",
                    "[{\"plane\":3,\"bit\":0}]",
                    1,
                    ",\"channels\":2",
                ),
                "plane_past",
                "$.writes[0].plane",
                Some(1),
            ),
            (
                request(
                    rule,
                    "[{\"plane\":1,\"bit\":0}]",
                    "[{\"plane\":1,\"bit\":1}]",
                    1,
                    "",
                ),
                "halo_written",
                "$.reads[0].plane",
                None,
            ),
            (
                request(
                    rule,
                    "[{\"plane\":0,\"bit\":0}]",
                    "[{\"plane\":1,\"bit\":0},{\"plane\":1,\"bit\":0}]",
                    2,
                    "",
                ),
                "bit_shared",
                "$.writes[1]",
                None,
            ),
        ];
        for (arguments, code, field, limit) in &cases {
            let told = mcp.no("grit_law_learn", arguments);
            refusal(&told, code, field);
            assert_eq!(
                told.get("limit").and_then(Json::int),
                *limit,
                "{rule} {code}"
            );
        }
    }
}
