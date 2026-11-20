#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use std::collections::{BTreeMap, BTreeSet};

use grit::Host;
use grit::netlist::Gate;
use grit::serve3::Server;
use grit::stencil::{self, Stencil};
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
            let listed: Vec<String> = piece
                .iter()
                .map(|(at, word)| format!("[{},{},{},{word}]", at[0], at[1], at[2]))
                .collect();
            let got = self.ok(
                "grit_world_write",
                &format!(
                    "{{\"world\":{world},\"plane\":{plane},\"cells\":[{}],\"budget_ms\":{LONG}}}",
                    listed.join(",")
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

    fn ran(&mut self, law: &str, size: [u32; 3], plane: u32, input: &[u32], out: u32) -> Vec<u32> {
        let world = self.world(law, size);
        let blank = vec![0_u32; input.len()];
        let got = self.again(world, size, plane, (&blank, input), out);
        self.close(world);
        got
    }

    fn again(
        &mut self,
        world: u32,
        size: [u32; 3],
        plane: u32,
        (before, input): (&[u32], &[u32]),
        out: u32,
    ) -> Vec<u32> {
        let changed: Vec<([u32; 3], u32)> = cells_of(size, input)
            .into_iter()
            .filter(|(at, word)| before[stencil::index(size, *at)] != *word)
            .collect();
        self.write(world, plane, &changed);
        self.step(world);
        self.read(world, out, size)
    }
}

fn cells_of(size: [u32; 3], words: &[u32]) -> Vec<([u32; 3], u32)> {
    let mut out = Vec::new();
    for z in 0..size[2] {
        for y in 0..size[1] {
            for x in 0..size[0] {
                out.push(([x, y, z], words[stencil::index(size, [x, y, z])]));
            }
        }
    }
    out
}

fn gates_text(gates: &[Gate]) -> String {
    let listed: Vec<String> = gates
        .iter()
        .map(|one| {
            format!(
                "{{\"kind\":{},\"left\":{},\"right\":{}}}",
                one.kind, one.left, one.right
            )
        })
        .collect();
    format!("[{}]", listed.join(","))
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

fn random_circuit(rng: &mut Rng, inputs: u32, many: u32) -> Vec<Gate> {
    (0..many)
        .map(|at| Gate {
            kind: 1 + rng.below(14),
            left: rng.below(u64::from(inputs + at)),
            right: rng.below(u64::from(inputs + at)),
        })
        .collect()
}

fn bindings(stencil: Stencil, plane: u32) -> String {
    let listed: Vec<String> = (0..stencil.positions())
        .map(|place| {
            let [dx, dy, dz] = stencil.offset(place);
            format!("{{\"plane\":{plane},\"dx\":{dx},\"dy\":{dy},\"dz\":{dz},\"bit\":0}}")
        })
        .collect();
    format!("[{}]", listed.join(","))
}

fn chip_law(mcp: &mut Mcp<'_>, stencil: Stencil, gates: &[Gate], read: u32, write: u32) -> String {
    let got = mcp.ok(
        "grit_law_from_chip",
        &format!(
            "{{\"inputs\":{},\"outputs\":1,\"gates\":{},\"bind_inputs\":{},\"bind_outputs\":[{{\"plane\":{write},\"bit\":0}}],\"channels\":2}}",
            stencil.positions(),
            gates_text(gates),
            bindings(stencil, read)
        ),
    );
    text_at(&got, "law")
}

fn learn_args(
    stencil: Stencil,
    rule: &str,
    border: &str,
    examples: &str,
    read: u32,
    write: u32,
    more: &str,
) -> String {
    format!(
        "{{\"stencil\":\"{}\",\"reads\":[{{\"plane\":{read},\"bit\":0}}],\"writes\":[{{\"plane\":{write},\"bit\":0}}],\"border\":\"{border}\",\"rule\":\"{rule}\",\"examples\":{examples},\"budget_ms\":{LONG}{more}}}",
        stencil.name()
    )
}

fn inline(size: [u32; 3], input: &[u32], output: &[u32]) -> String {
    format!(
        "{{\"size\":[{},{},{}],\"input\":[{}],\"output\":[{}]}}",
        size[0],
        size[1],
        size[2],
        listed(input),
        listed(output)
    )
}

fn random_bits(rng: &mut Rng, cells: usize, out_of: u64) -> Vec<u32> {
    (0..cells)
        .map(|_| u32::from(rng.below(out_of) == 0))
        .collect()
}

fn own_rows(stencil: Stencil, size: [u32; 3], input: &[u32], wrapped: bool) -> Vec<u32> {
    let mut rows = Vec::new();
    for z in 0..size[2] {
        for y in 0..size[1] {
            for x in 0..size[0] {
                let mut row = 0_u32;
                for place in 0..stencil.positions() {
                    let [dx, dy, dz] = stencil.offset(place);
                    let at = [i64::from(x) + dx, i64::from(y) + dy, i64::from(z) + dz];
                    let mut spot = [0_u32; 3];
                    let mut inside = true;
                    for axis in 0..3 {
                        let side = i64::from(size[axis]);
                        let value = if wrapped {
                            at[axis].rem_euclid(side)
                        } else {
                            at[axis]
                        };
                        if !(0..side).contains(&value) {
                            inside = false;
                        } else {
                            spot[axis] = u32::try_from(value).unwrap();
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

fn seen_of(rows: &[u32], outputs: &[u32]) -> BTreeMap<u32, u32> {
    rows.iter()
        .zip(outputs)
        .map(|(row, out)| (*row, out & 1))
        .collect()
}

fn round_trip(host: &Host, stencil: Stencil, size: [u32; 3], seed: u64, truth: &[Gate], top: u32) {
    let mut rng = Rng(seed);
    let mut mcp = Mcp::new(Some(host));
    let inputs = stencil.positions();
    let truth_law = chip_law(&mut mcp, stencil, truth, 1, 0);
    let cells = size.iter().product::<u32>() as usize;
    let input = random_bits(&mut rng, cells, 2);
    let hidden = mcp.world(&truth_law, size);
    let ones: Vec<([u32; 3], u32)> = cells_of(size, &input)
        .into_iter()
        .filter(|(_, word)| *word != 0)
        .collect();
    mcp.write(hidden, 1, &ones);
    mcp.step(hidden);
    let wanted = mcp.read(hidden, 0, size);
    let rows = own_rows(stencil, size, &input, true);
    for (row, out) in rows.iter().zip(&wanted) {
        assert_eq!(
            stencil::answer(inputs, 1, truth, *row),
            *out,
            "the truth law answers its chip"
        );
    }
    let seen = seen_of(&rows, &wanted);
    let got = mcp.ok(
        "grit_law_learn",
        &learn_args(
            stencil,
            "minimal",
            "wrapped",
            &format!("[{{\"world\":{hidden}}}]"),
            1,
            0,
            &format!(",\"gates\":{top},\"crowd\":2048,\"rounds\":2000"),
        ),
    );
    let examples = got.get("examples").unwrap();
    assert_eq!(int_at(examples, "seen"), seen.len() as i64, "seen rows");
    assert_eq!(int_at(examples, "rows"), 1_i64 << inputs);
    assert_eq!(
        int_at(got.get("unseen").unwrap(), "rows"),
        (1_i64 << inputs) - seen.len() as i64
    );
    let law = text_at(&got, "law");
    let on_device = mcp.ran(&law, size, 1, &input, 0);
    assert_eq!(
        on_device, wanted,
        "the learned law on a world equals the truth"
    );
    let start: Vec<u32> = vec![0; cells]
        .into_iter()
        .chain(input.iter().copied())
        .collect();
    let scored = mcp.ok(
        "grit_law_score",
        &format!(
            "{{\"law\":{},\"size\":[{},{},{}],\"cells\":{},\"frames\":1,\"aim\":{{\"from\":[0,0,0],\"to\":[{},{},{}],\"target\":{}}},\"budget_ms\":{LONG}}}",
            written(&Json::Text(law.clone())),
            size[0],
            size[1],
            size[2],
            listed(&start),
            size[0] - 1,
            size[1] - 1,
            size[2] - 1,
            listed(&wanted)
        ),
    );
    assert_eq!(
        scored.get("objective").and_then(Json::int),
        Some(0),
        "the CPU twin of the learned law equals the truth: {}",
        written(&scored)
    );
}

#[test]
fn l1_a_learned_3x3_law_equals_its_truth_on_every_seen_neighbourhood() {
    let Some(host) = device("law_learn l1 3x3") else {
        return;
    };
    for seed in [11, 12, 13] {
        let truth = random_circuit(&mut Rng(seed * 7), 9, 6);
        round_trip(&host, Stencil::Square, [16, 16, 2], seed, &truth, 12);
    }
    for place in 0..9 {
        let copy = [Gate {
            kind: 10,
            left: place,
            right: place,
        }];
        round_trip(&host, Stencil::Square, [16, 16, 2], 14, &copy, 2);
    }
}

#[test]
fn l1_a_learned_3x3x3_law_equals_its_truth_on_every_seen_neighbourhood() {
    let Some(host) = device("law_learn l1 3x3x3") else {
        return;
    };
    for seed in [21, 22] {
        let truth = random_circuit(&mut Rng(seed * 7), 27, 6);
        round_trip(&host, Stencil::Cube, [8, 8, 8], seed, &truth, 12);
    }
    for place in 0..27 {
        let copy = [Gate {
            kind: 10,
            left: place,
            right: place,
        }];
        round_trip(&host, Stencil::Cube, [8, 8, 8], 23, &copy, 2);
    }
}

type Pairs = Vec<(Vec<u32>, Vec<u32>)>;

fn x0b() -> (Pairs, Vec<Vec<u32>>) {
    let task = read(TASK).unwrap();
    let flat = |held: &Json| -> Vec<u32> { held.list().unwrap().iter().flat_map(words).collect() };
    let train = task
        .get("train")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .map(|pair| {
            (
                flat(pair.get("input").unwrap()),
                flat(pair.get("output").unwrap()),
            )
        })
        .collect();
    let held = task
        .get("held")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .map(|one| flat(one.get("input").unwrap()))
        .collect();
    (train, held)
}

fn x0b_examples(train: &[(Vec<u32>, Vec<u32>)]) -> String {
    let listed: Vec<String> = train
        .iter()
        .map(|(input, output)| inline([32, 32, 1], input, output))
        .collect();
    format!("[{}]", listed.join(","))
}

fn every_neighbourhood(inputs: u32) -> (Vec<u32>, [u32; 3]) {
    let rows = 1_u32 << inputs;
    let across = 64_u32;
    let size = [across * 4, (rows / across) * 4, 2];
    let mut image = vec![0_u32; size.iter().product::<u32>() as usize];
    for row in 0..rows {
        let base = [(row % across) * 4 + 1, (row / across) * 4 + 1];
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
}

fn centres(out: &[u32], size: [u32; 3]) -> Vec<u32> {
    (0..512_u32)
        .map(|row| {
            let at = [(row % 64) * 4 + 1, (row / 64) * 4 + 1, 0];
            out[stencil::index(size, at)] & 1
        })
        .collect()
}

fn learned_x0b(mcp: &mut Mcp<'_>, rule: &str) -> Json {
    let (train, _) = x0b();
    mcp.ok(
        "grit_law_learn",
        &learn_args(
            Stencil::Square,
            rule,
            "zero",
            &x0b_examples(&train),
            0,
            1,
            "",
        ),
    )
}

#[test]
fn l2_the_report_counts_every_row_and_each_decided_row_runs_as_its_table_says() {
    let Some(host) = device("law_learn l2") else {
        return;
    };
    let (train, _) = x0b();
    let mut seen: BTreeMap<u32, u32> = BTreeMap::new();
    for (input, output) in &train {
        let rows = own_rows(Stencil::Square, [32, 32, 1], input, false);
        seen.extend(seen_of(&rows, output));
    }
    let mut mcp = Mcp::new(Some(&host));
    let (image, size) = every_neighbourhood(9);
    for rule in ["minimal", "symmetric"] {
        let got = learned_x0b(&mut mcp, rule);
        let examples = got.get("examples").unwrap();
        assert_eq!(int_at(examples, "seen"), seen.len() as i64, "{rule}");
        assert_eq!(int_at(examples, "conflicts"), 0);
        let unseen = got.get("unseen").unwrap();
        assert_eq!(int_at(unseen, "rows"), 512 - seen.len() as i64, "{rule}");
        assert_eq!(text_at(unseen, "rule"), rule);
        let table = words(unseen.get("table").unwrap());
        let never: Vec<u32> = (0..512).filter(|row| !seen.contains_key(row)).collect();
        assert_eq!(words(unseen.get("unseen_rows").unwrap()), never, "{rule}");
        let ones = never
            .iter()
            .filter(|row| table[**row as usize] == 1)
            .count() as i64;
        assert_eq!(
            words(unseen.get("ones").unwrap()),
            [u32::try_from(ones).unwrap()]
        );
        assert_eq!(
            words(unseen.get("zeros").unwrap()),
            [u32::try_from(never.len() as i64 - ones).unwrap()]
        );
        let ran = mcp.ran(&text_at(&got, "law"), size, 0, &image, 1);
        let on_world = centres(&ran, size);
        assert_eq!(
            on_world, table,
            "{rule}: the world runs every row as the table says"
        );
        for (row, out) in &seen {
            assert_eq!(table[*row as usize], *out, "{rule}: seen row {row}");
        }
        let chip = got.get("chip").unwrap();
        let gates = gates_of(chip.get("gates").unwrap());
        assert_eq!(stencil::table(9, 1, &gates), table, "{rule}: the chip");
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
        for other in 0..3 {
            map[other][axis] = to[other];
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

fn invariant(
    mcp: &mut Mcp<'_>,
    law: &str,
    stencil: Stencil,
    size: [u32; 3],
    seed: u64,
    density: u64,
) {
    let mut rng = Rng(seed);
    let cells = size.iter().product::<u32>() as usize;
    let image = random_bits(&mut rng, cells, density);
    let world = mcp.world(law, size);
    let blank = vec![0_u32; cells];
    let base = mcp.again(world, size, 0, (&blank, &image), 1);
    let mut held = image.clone();
    for (at, places) in stencil.symmetries().iter().enumerate() {
        let moved = turned(stencil, size, &image, places);
        let out = mcp.again(world, size, 0, (&held, &moved), 1);
        held = moved;
        assert_eq!(
            out,
            turned(stencil, size, &base, places),
            "symmetry {at}: the law's output turns with its input"
        );
    }
    mcp.close(world);
}

#[test]
fn l3_a_symmetric_3x3_law_turns_with_its_input_on_a_world() {
    let Some(host) = device("law_learn l3 3x3") else {
        return;
    };
    let mut mcp = Mcp::new(Some(&host));
    let got = learned_x0b(&mut mcp, "symmetric");
    let law = text_at(&got, "law");
    let table = words(got.get("unseen").unwrap().get("table").unwrap());
    let moves = Stencil::Square.symmetries();
    for row in 0..512_u32 {
        for moved in &moves {
            assert_eq!(
                table[stencil::image(row, moved) as usize],
                table[row as usize],
                "row {row}"
            );
        }
    }
    invariant(&mut mcp, &law, Stencil::Square, [32, 32, 2], 31, 2);
    invariant(&mut mcp, &law, Stencil::Square, [32, 32, 2], 32, 4);
}

fn symmetric_truth() -> Vec<Gate> {
    let face: Vec<u32> = [
        [1, 0, 0],
        [-1, 0, 0],
        [0, 1, 0],
        [0, -1, 0],
        [0, 0, 1],
        [0, 0, -1],
    ]
    .iter()
    .map(|at| Stencil::Cube.place(*at))
    .collect();
    let centre = Stencil::Cube.place([0, 0, 0]);
    let mut gates = vec![Gate {
        kind: 14,
        left: face[0],
        right: face[1],
    }];
    for one in &face[2..] {
        let last = 27 + u32::try_from(gates.len()).unwrap() - 1;
        gates.push(Gate {
            kind: 14,
            left: last,
            right: *one,
        });
    }
    let last = 27 + u32::try_from(gates.len()).unwrap() - 1;
    gates.push(Gate {
        kind: 8,
        left: last,
        right: centre,
    });
    gates
}

#[test]
fn l3_a_symmetric_3x3x3_law_turns_with_its_input_on_a_world() {
    let Some(host) = device("law_learn l3 3x3x3") else {
        return;
    };
    let mut mcp = Mcp::new(Some(&host));
    let truth = symmetric_truth();
    let size = [8, 8, 8];
    let mut input = vec![0_u32; 512];
    for at in [[2, 2, 2], [3, 2, 2], [6, 6, 5], [2, 6, 6], [3, 7, 7]] {
        input[stencil::index(size, at)] = 1;
    }
    let rows = own_rows(Stencil::Cube, size, &input, true);
    let wanted: Vec<u32> = rows
        .iter()
        .map(|row| stencil::answer(27, 1, &truth, *row))
        .collect();
    let got = mcp.ok(
        "grit_law_learn",
        &learn_args(
            Stencil::Cube,
            "symmetric",
            "wrapped",
            &format!("[{}]", inline(size, &input, &wanted)),
            0,
            1,
            ",\"gates\":8,\"crowd\":16384,\"rounds\":500",
        ),
    );
    let unseen = got.get("unseen").unwrap();
    assert!(unseen.get("table").is_none(), "27 bits answer counts only");
    let ones = u64::from(words(unseen.get("ones").unwrap())[0]);
    let zeros = u64::from(words(unseen.get("zeros").unwrap())[0]);
    let seen = seen_of(&rows, &wanted);
    assert_eq!(ones + zeros + seen.len() as u64, 1 << 27);
    let gates = gates_of(got.get("chip").unwrap().get("gates").unwrap());
    let all = stencil::ones(27, 1, &gates, &mut || false).unwrap()[0];
    let on_seen = seen.values().filter(|one| **one == 1).count() as u64;
    assert_eq!(all - on_seen, ones, "the decided ones are the chip's");
    let law = text_at(&got, "law");
    assert_eq!(mcp.ran(&law, size, 0, &input, 1), wanted, "the examples");
    invariant(&mut mcp, &law, Stencil::Cube, size, 42, 5);
}

#[test]
fn l3_rows_that_break_the_symmetry_are_refused_by_name() {
    let Some(host) = device("law_learn l3 refusal") else {
        return;
    };
    let mut mcp = Mcp::new(Some(&host));
    let size = [8, 8, 1];
    let mut input = vec![0_u32; 64];
    let mut output = vec![0_u32; 64];
    input[stencil::index(size, [2, 2, 0])] = 1;
    input[stencil::index(size, [3, 2, 0])] = 1;
    output[stencil::index(size, [2, 2, 0])] = 1;
    output[stencil::index(size, [3, 2, 0])] = 0;
    let examples = format!("[{}]", inline(size, &input, &output));
    let told = mcp.no(
        "grit_law_learn",
        &learn_args(Stencil::Square, "symmetric", "zero", &examples, 0, 1, ""),
    );
    assert_eq!(
        text_at(&told, "refused"),
        "asymmetric",
        "{}",
        written(&told)
    );
    assert_eq!(text_at(&told, "field"), "$.rule");
    let left = 1_u32 << 4 | 1 << 5;
    let right = 1_u32 << 4 | 1 << 3;
    let orbits = told.get("orbits").and_then(Json::list).unwrap();
    let named: BTreeSet<(u32, u32)> = orbits
        .iter()
        .flat_map(|orbit| orbit.get("rows").and_then(Json::list).unwrap().iter())
        .map(|one| {
            (
                u32::try_from(int_at(one, "row")).unwrap(),
                u32::try_from(int_at(one, "outputs")).unwrap(),
            )
        })
        .collect();
    assert!(named.contains(&(left, 1)), "{named:?}");
    assert!(named.contains(&(right, 0)), "{named:?}");
    let fine = mcp.ok(
        "grit_law_learn",
        &learn_args(Stencil::Square, "minimal", "zero", &examples, 0, 1, ""),
    );
    assert_eq!(text_at(&fine, "stopped"), "ladder");
}

#[test]
fn l4_the_x0b_pairs_learn_a_law_whose_world_answers_every_training_cell() {
    let Some(host) = device("law_learn l4") else {
        return;
    };
    let (train, _) = x0b();
    let mut mcp = Mcp::new(Some(&host));
    let size = [256, 256, 2];
    let mut image = vec![0_u32; 256 * 256 * 2];
    let place = |at: usize| ((at % 7) as u32 * 33, (at / 7) as u32 * 33);
    for (at, (input, _)) in train.iter().enumerate() {
        let (ox, oy) = place(at);
        for y in 0..32 {
            for x in 0..32 {
                image[stencil::index(size, [ox + x, oy + y, 0])] = input[(y * 32 + x) as usize];
            }
        }
    }
    for rule in ["minimal", "symmetric"] {
        let got = learned_x0b(&mut mcp, rule);
        let ran = mcp.ran(&text_at(&got, "law"), size, 0, &image, 1);
        let mut right = 0;
        for (at, (_, output)) in train.iter().enumerate() {
            let (ox, oy) = place(at);
            for y in 0..32 {
                for x in 0..32 {
                    assert_eq!(
                        ran[stencil::index(size, [ox + x, oy + y, 0])] & 1,
                        output[(y * 32 + x) as usize],
                        "{rule}: pair {at} cell ({x}, {y})"
                    );
                    right += 1;
                }
            }
        }
        assert_eq!(right, 24 * 1024);
    }
}

#[test]
fn l5_the_same_request_answers_the_same_bytes_after_a_restart() {
    let Some(host) = device("law_learn l5") else {
        return;
    };
    let (train, _) = x0b();
    let arguments = learn_args(
        Stencil::Square,
        "symmetric",
        "zero",
        &x0b_examples(&train[..8]),
        0,
        1,
        "",
    );
    let mut first = Mcp::new(Some(&host));
    let one = written(&first.ok("grit_law_learn", &arguments));
    let two = written(&first.ok("grit_law_learn", &arguments));
    drop(first);
    let mut again = Mcp::new(Some(&host));
    let three = written(&again.ok("grit_law_learn", &arguments));
    assert_eq!(one, two);
    assert_eq!(one, three, "after a restart");
}

#[test]
fn a_neighbourhood_seen_with_two_outputs_is_refused_with_both() {
    let Some(host) = device("law_learn conflict") else {
        return;
    };
    let mut mcp = Mcp::new(Some(&host));
    let size = [4, 4, 1];
    let input = vec![0_u32; 16];
    let mut output = vec![0_u32; 16];
    output[5] = 1;
    let examples = format!("[{}]", inline(size, &input, &output));
    let told = mcp.no(
        "grit_law_learn",
        &learn_args(Stencil::Square, "minimal", "zero", &examples, 0, 1, ""),
    );
    assert_eq!(text_at(&told, "refused"), "conflict", "{}", written(&told));
    assert_eq!(int_at(&told, "conflicts"), 1);
    let rows = told.get("rows").and_then(Json::list).unwrap();
    assert_eq!(int_at(&rows[0], "row"), 0);
    let seen = rows[0].get("seen").and_then(Json::list).unwrap();
    let outputs: BTreeSet<i64> = seen.iter().map(|one| int_at(one, "outputs")).collect();
    assert_eq!(outputs, BTreeSet::from([0, 1]));
    assert!(
        seen.iter()
            .any(|one| words(one.get("at").unwrap()) == [1, 1, 0])
    );
}

#[test]
fn malformed_requests_are_refused_by_name_before_any_search() {
    let mut none = Mcp::new(None);
    let examples = format!("[{}]", inline([4, 4, 1], &[0; 16], &[0; 16]));
    let told = none.no(
        "grit_law_learn",
        &learn_args(Stencil::Square, "minimal", "zero", &examples, 0, 1, ""),
    );
    assert_eq!(text_at(&told, "refused"), "no_compute_device");
    let Some(host) = device("law_learn refusals") else {
        return;
    };
    let mut mcp = Mcp::new(Some(&host));
    let cases = [
        (
            learn_args(Stencil::Square, "minimal", "zero", &examples, 0, 0, ""),
            "halo_written",
            "$.reads[0].plane",
        ),
        (
            learn_args(Stencil::Square, "minimal", "wrapped", &format!("[{}]", inline([3, 4, 1], &[0; 12], &[0; 12])), 0, 1, ""),
            "example",
            "$.examples[0].size",
        ),
        (
            learn_args(Stencil::Square, "minimal", "zero", &format!("[{}]", inline([4, 4, 1], &[0; 15], &[0; 16])), 0, 1, ""),
            "example",
            "$.examples[0].input[0]",
        ),
        (
            learn_args(Stencil::Square, "minimal", "zero", "[{\"world\":9}]", 0, 1, ""),
            "no_world",
            "$.examples[0].world",
        ),
        (
            learn_args(Stencil::Square, "minimal", "zero", "[{}]", 0, 1, ""),
            "example",
            "$.examples[0]",
        ),
        (
            learn_args(Stencil::Square, "minimal", "zero", &examples, 0, 1, ",\"channels\":1"),
            "plane_past",
            "$.writes[0].plane",
        ),
        (
            "{\"stencil\":\"3x3x3\",\"reads\":[{\"plane\":0,\"bit\":0},{\"plane\":0,\"bit\":1}],\"writes\":[{\"plane\":1,\"bit\":0}],\"border\":\"zero\",\"rule\":\"minimal\",\"examples\":[]}".to_owned(),
            "malformed",
            "$.examples",
        ),
        (
            format!("{{\"stencil\":\"3x3x3\",\"reads\":[{{\"plane\":0,\"bit\":0}},{{\"plane\":0,\"bit\":1}}],\"writes\":[{{\"plane\":1,\"bit\":0}}],\"border\":\"zero\",\"rule\":\"minimal\",\"examples\":{examples}}}"),
            "learn_limit",
            "$.reads",
        ),
    ];
    for (arguments, code, field) in &cases {
        let told = mcp.no("grit_law_learn", arguments);
        assert_eq!(
            (text_at(&told, "refused"), text_at(&told, "field")),
            ((*code).to_owned(), (*field).to_owned()),
            "{}",
            written(&told)
        );
    }
}

#[test]
fn the_row_order_is_stated_where_tables_are_taken_and_answered() {
    let mut mcp = Mcp::new(None);
    let listed = mcp
        .server
        .heard("{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}")
        .unwrap();
    let tools = listed
        .get("result")
        .and_then(|one| one.get("tools"))
        .and_then(Json::list)
        .unwrap();
    for name in [
        "grit_synthesise",
        "grit_machine",
        "grit_least",
        "grit_law_learn",
    ] {
        let tool = tools
            .iter()
            .find(|one| one.get("name").and_then(Json::text) == Some(name))
            .unwrap();
        let said = written(tool);
        assert!(said.contains("bit j"), "{name}: {said}");
    }
    let described = mcp.ok("grit_describe", "{}");
    let limits = described.get("limits").unwrap();
    assert_eq!(text_at(limits, "law_learn_rows"), stencil::ORDER);
}

#[test]
fn a_symmetry_group_has_eight_or_forty_eight_distinct_moves() {
    for (stencil, many) in [(Stencil::Square, 8), (Stencil::Cube, 48)] {
        let moves = stencil.symmetries();
        let distinct: BTreeSet<Vec<u32>> = moves.iter().cloned().collect();
        assert_eq!(distinct.len(), many);
        for moved in &moves {
            let centre = stencil.place([0, 0, 0]);
            assert_eq!(moved[centre as usize], centre);
            for other in &moves {
                let both: Vec<u32> = (0..stencil.positions())
                    .map(|place| other[moved[place as usize] as usize])
                    .collect();
                assert!(distinct.contains(&both), "closed under composition");
            }
        }
    }
}

fn cube_examples(seed: u64, many: usize) -> String {
    let mut rng = Rng(seed);
    let size = [16, 16, 16];
    let listed: Vec<String> = (0..many)
        .map(|_| {
            let input = random_bits(&mut rng, 4096, 2);
            let output = vec![0_u32; 4096];
            inline(size, &input, &output)
        })
        .collect();
    format!("[{}]", listed.join(","))
}

#[test]
fn a_symmetric_cube_ladder_starts_where_its_law_fits() {
    let Some(host) = device("law_learn gates capped") else {
        return;
    };
    let mut mcp = Mcp::new(Some(&host));
    let size = [8, 8, 8];
    let mut input = vec![0_u32; 512];
    for at in [[2, 2, 2], [3, 2, 2], [6, 6, 5], [2, 6, 6], [3, 7, 7]] {
        input[stencil::index(size, at)] = 1;
    }
    let truth = symmetric_truth();
    let rows = own_rows(Stencil::Cube, size, &input, true);
    let wanted: Vec<u32> = rows
        .iter()
        .map(|row| stencil::answer(27, 1, &truth, *row))
        .collect();
    let got = mcp.ok(
        "grit_law_learn",
        &learn_args(
            Stencil::Cube,
            "symmetric",
            "wrapped",
            &format!("[{}]", inline(size, &input, &wanted)),
            0,
            1,
            ",\"gates\":40,\"crowd\":16384,\"rounds\":2000",
        ),
    );
    let capped = got.get("gates_capped").unwrap();
    assert_eq!(int_at(capped, "asked"), 40);
    assert_eq!(
        int_at(capped, "used"),
        11,
        "48 copies of 11 and 47 ors fit 600"
    );
    let ladder = got.get("ladder").and_then(Json::list).unwrap();
    assert_eq!(int_at(&ladder[0], "gates"), 11);
    assert!(int_at(&got, "gates") <= 600);
    let fine = mcp.ok(
        "grit_law_learn",
        &learn_args(
            Stencil::Cube,
            "symmetric",
            "wrapped",
            &format!("[{}]", inline(size, &input, &wanted)),
            0,
            1,
            ",\"gates\":8,\"crowd\":16384,\"rounds\":2000",
        ),
    );
    assert!(fine.get("gates_capped").is_none());
}

#[test]
fn images_past_one_kernel_are_refused_before_any_search() {
    let Some(host) = device("law_learn images past a kernel") else {
        return;
    };
    let mut mcp = Mcp::new(Some(&host));
    let examples = cube_examples(61, 8);
    let told = mcp.no(
        "grit_law_learn",
        &learn_args(Stencil::Cube, "symmetric", "wrapped", &examples, 0, 1, ""),
    );
    assert_eq!(
        (text_at(&told, "refused"), text_at(&told, "field")),
        ("learn_limit".to_owned(), "$.examples".to_owned()),
        "{}",
        written(&told)
    );
    assert!(
        text_at(&told, "message").contains("images"),
        "{}",
        written(&told)
    );
    assert_eq!(int_at(&told, "limit"), 860_352);
}

#[cfg(feature = "faults")]
mod faulted {
    use super::*;

    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::Receiver;
    use std::time::Duration;

    use grit::clock::{Fake, Time};
    use grit::serve3::Options;
    use grit::serve3::exec::Exec;
    use grit::serve3::law_learn::Point;
    use grit::serve3::law_learn::faults::{
        builds, builds_slowed_by, calibration_slowed, calibrations, cancel_after, ceiling_at,
        checks, deadline_passed_at, disarm, kernel_most, reached, rounds_each_submission,
        submissions,
    };
    use grit::stencil::Count;

    fn arguments() -> String {
        let (train, _) = x0b();
        learn_args(
            Stencil::Square,
            "minimal",
            "zero",
            &x0b_examples(&train[..8]),
            0,
            1,
            "",
        )
    }

    fn ladder(got: &Json) -> Vec<String> {
        got.get("ladder")
            .and_then(Json::list)
            .unwrap()
            .iter()
            .map(written)
            .collect()
    }

    fn whole(host: &Host) -> (Json, u64) {
        let mut mcp = Mcp::new(Some(host));
        disarm();
        rounds_each_submission(1);
        let before = checks();
        let got = mcp.ok("grit_law_learn", &arguments());
        disarm();
        assert_eq!(text_at(&got, "stopped"), "ladder");
        (got, checks() - before)
    }

    fn timed(host: &Host) -> (Mcp<'_>, Arc<Fake>) {
        let fake = Arc::new(Fake::new());
        let time: Arc<dyn Time> = fake.clone();
        let server = Server::timed(Some(host), &Options::default(), time).unwrap();
        (Mcp { server }, fake)
    }

    fn within(arguments: &str, budget: u64) -> String {
        let long = format!("\"budget_ms\":{LONG}");
        assert!(arguments.contains(&long));
        arguments.replace(&long, &format!("\"budget_ms\":{budget}"))
    }

    fn top_words(host: &Host, whole: &Json, gates: u32) -> u32 {
        let seen = u32::try_from(int_at(whole.get("examples").unwrap(), "seen")).unwrap();
        let machine =
            grit::machine::Machine::over(0, 9, 1, gates, grit::law::chip::KINDS, 1).unwrap();
        let words = grit::marking::stepped_rolled(host.width(), machine, seen.div_ceil(32).max(1))
            .unwrap()
            .len();
        u32::try_from(words).unwrap()
    }

    fn patience() -> Duration {
        grit::Resident::patience_of(std::env::var("GRIT_PATIENCE_SECS").ok().as_deref())
    }

    fn frozen(host: &Host) -> (Json, [u64; 5], u64, u64) {
        let (mut mcp, fake) = timed(host);
        disarm();
        rounds_each_submission(1);
        let got = mcp.ok("grit_law_learn", &arguments());
        let counts = [
            Point::Calibration,
            Point::Build,
            Point::Submission,
            Point::Expansion,
            Point::Count,
        ]
        .map(reached);
        let made = (builds(), submissions());
        disarm();
        assert_eq!(fake.elapsed(), Duration::ZERO, "the clock stayed frozen");
        assert_eq!(text_at(&got, "stopped"), "ladder", "{}", written(&got));
        (got, counts, made.0, made.1)
    }

    fn braked_at(host: &Host, point: Point, ordinal: u64) -> (Json, u64, u64, u64) {
        let (mut mcp, fake) = timed(host);
        disarm();
        rounds_each_submission(1);
        deadline_passed_at(point, ordinal, Arc::clone(&fake), LONG + 1);
        let got = mcp.raw("grit_law_learn", &arguments());
        let counts = (reached(point), builds(), submissions());
        disarm();
        assert!(
            fake.elapsed() > Duration::from_millis(LONG),
            "the {} point {ordinal} passed the deadline",
            point.name()
        );
        (got, counts.0, counts.1, counts.2)
    }

    fn on_budget(got: &Json, whole: &Json, point: Point, ordinal: u64) -> Option<Json> {
        let failed =
            got.get("result").and_then(|one| one.get("isError")) == Some(&Json::Bool(true));
        if failed {
            let told = result(got, true);
            assert_eq!(
                (text_at(&told, "refused"), text_at(&told, "field")),
                ("budget".to_owned(), "$.budget_ms".to_owned()),
                "{} {ordinal}: {}",
                point.name(),
                written(&told)
            );
            assert!(text_at(&told, "message").contains("nothing was learned"));
            return None;
        }
        let held = result(got, false);
        stopped_at(&held, whole, "budget");
        Some(held)
    }

    fn stopped_at(got: &Json, whole: &Json, why: &str) {
        assert_eq!(text_at(got, "stopped"), why, "{}", written(got));
        assert!(got.get("hint").is_some());
        let part = ladder(got);
        let all = ladder(whole);
        assert!(part.len() < all.len(), "the ladder stopped early");
        assert_eq!(part[..], all[..part.len()], "a prefix of the whole ladder");
        let rungs = got.get("ladder").and_then(Json::list).unwrap();
        let least = rungs
            .iter()
            .filter(|rung| rung.get("solved") == Some(&Json::Bool(true)))
            .map(|rung| int_at(rung, "gates"))
            .min()
            .unwrap();
        assert_eq!(
            int_at(got.get("found").unwrap(), "gates"),
            least,
            "the smallest solved so far"
        );
    }

    #[test]
    fn a_ladder_cancelled_between_looks_answers_its_smallest_solved_circuit_the_same_again() {
        let Some(host) = device("law_learn cancel") else {
            return;
        };
        let (full, spent) = whole(&host);
        let mut answers = Vec::new();
        for _ in 0..2 {
            let mut mcp = Mcp::new(Some(&host));
            let flag = Arc::new(AtomicBool::new(false));
            mcp.server.arriving(None, Some(Arc::clone(&flag)));
            rounds_each_submission(1);
            cancel_after(spent / 2);
            let got = mcp.ok("grit_law_learn", &arguments());
            disarm();
            assert!(flag.load(Ordering::SeqCst));
            stopped_at(&got, &full, "cancelled");
            answers.push(written(&got));
        }
        assert_eq!(answers[0], answers[1]);
        let mut mcp = Mcp::new(Some(&host));
        let flag = Arc::new(AtomicBool::new(false));
        mcp.server.arriving(None, Some(Arc::clone(&flag)));
        cancel_after(0);
        let told = mcp.no("grit_law_learn", &arguments());
        disarm();
        assert_eq!(text_at(&told, "refused"), "cancelled", "{}", written(&told));
        assert!(text_at(&told, "message").contains("nothing was learned"));
    }

    #[test]
    fn an_expansion_cancelled_part_way_is_refused_by_name_before_any_search() {
        let Some(host) = device("law_learn expansion cancelled") else {
            return;
        };
        let examples = cube_examples(62, 2);
        let arguments = learn_args(Stencil::Cube, "symmetric", "wrapped", &examples, 0, 1, "");
        let mut mcp = Mcp::new(Some(&host));
        let flag = Arc::new(AtomicBool::new(false));
        mcp.server.arriving(None, Some(Arc::clone(&flag)));
        cancel_after(1);
        let before = checks();
        let told = mcp.no("grit_law_learn", &arguments);
        disarm();
        assert!(flag.load(Ordering::SeqCst));
        assert_eq!(text_at(&told, "refused"), "cancelled", "{}", written(&told));
        assert!(text_at(&told, "message").contains("expanding"));
        assert_eq!(
            checks() - before,
            2,
            "the second check of the expansion stopped it"
        );
    }

    fn copied(seed: u64, size: [u32; 3], gates: u32) -> String {
        let mut rng = Rng(seed);
        let cells = size.iter().product::<u32>() as usize;
        let input = random_bits(&mut rng, cells, 2);
        learn_args(
            Stencil::Cube,
            "minimal",
            "wrapped",
            &format!("[{}]", inline(size, &input, &input)),
            0,
            1,
            &format!(",\"gates\":{gates},\"crowd\":64,\"rounds\":1"),
        )
    }

    fn kernel_of(host: &Host, arguments: &str) -> (Json, usize) {
        let mut mcp = Mcp::new(Some(host));
        disarm();
        let got = mcp.raw("grit_law_learn", arguments);
        let words = kernel_most();
        disarm();
        (got, words)
    }

    #[test]
    fn a_search_kernel_loops_over_its_rows_and_stays_within_its_bound() {
        let Some(host) = device("law_learn kernel bound") else {
            return;
        };
        let (_, small) = kernel_of(&host, &copied(71, [4, 4, 4], 8));
        let (_, large) = kernel_of(&host, &copied(72, [16, 16, 16], 8));
        assert!(small > 0, "a search kernel was built");
        assert!(
            large.abs_diff(small) * 32 < small,
            "64 rows built {small} words and about 4096 rows {large}; only constants differ"
        );
    }

    #[test]
    fn a_top_rung_whose_build_the_evidence_puts_past_half_the_patience_is_refused_before_any_search()
     {
        let Some(host) = device("law_learn kernel refused") else {
            return;
        };
        let patience =
            grit::Resident::patience_of(std::env::var("GRIT_PATIENCE_SECS").ok().as_deref());
        let machine =
            grit::machine::Machine::over(0, 27, 1, 64, grit::law::chip::KINDS, 1).unwrap();
        let top = grit::marking::stepped_rolled(host.width(), machine, 2)
            .unwrap()
            .len();
        let each = patience / u32::try_from(top).unwrap();
        let mut mcp = Mcp::new(Some(&host));
        disarm();
        builds_slowed_by(each);
        let told = mcp.no("grit_law_learn", &copied(73, [4, 4, 4], 64));
        let (built, measured) = (kernel_most(), calibrations());
        disarm();
        assert_eq!(
            (text_at(&told, "refused"), text_at(&told, "field")),
            ("learn_limit".to_owned(), "$.gates".to_owned()),
            "{}",
            written(&told)
        );
        assert_eq!(built, 0, "no search kernel was built before the refusal");
        assert!(measured >= 1, "the device's build was measured");
        assert!(int_at(&told, "limit") < 64, "{}", written(&told));
        let mut mcp = Mcp::new(Some(&host));
        disarm();
        builds_slowed_by(each / 4);
        let got = mcp.raw("grit_law_learn", &copied(73, [4, 4, 4], 64));
        let built = kernel_most();
        assert!(
            calibrations() > 1,
            "a quarter of the patience is admitted by measuring larger builds"
        );
        disarm();
        if let Some(told) = got
            .get("result")
            .and_then(|one| one.get("structuredContent"))
            .and_then(|one| one.get("refused"))
        {
            assert_ne!(told.text(), Some("learn_limit"), "{}", written(&got));
        }
        assert!(
            built > 0,
            "the same request without the slowed builds searched"
        );
    }

    #[test]
    fn a_crowd_past_one_binding_is_capped_and_named() {
        let Some(host) = device("law_learn crowd capped") else {
            return;
        };
        let mut mcp = Mcp::new(Some(&host));
        let free = mcp.ok("grit_law_learn", &arguments());
        assert!(free.get("crowd_capped").is_none(), "{}", written(&free));
        let rows = u64::try_from(int_at(free.get("examples").unwrap(), "seen")).unwrap();
        let wide = rows.div_ceil(32).max(1);
        let each = (9 + 16) * wide;
        let width = u64::from(host.width());
        let crowd = 1024_u64.max(width * 4);
        let mut mcp = Mcp::new(Some(&host));
        ceiling_at(crowd * each * 4);
        let got = mcp.ok("grit_law_learn", &arguments());
        disarm();
        let capped = got
            .get("crowd_capped")
            .unwrap_or_else(|| panic!("{}", written(&got)));
        assert_eq!(int_at(capped, "asked"), 4096);
        assert_eq!(
            int_at(capped, "used"),
            i64::try_from(crowd / width * width).unwrap()
        );
        let mut mcp = Mcp::new(Some(&host));
        ceiling_at(each * 4 * (width.max(64) - 1));
        let told = mcp.no("grit_law_learn", &arguments());
        disarm();
        assert_eq!(
            (text_at(&told, "refused"), text_at(&told, "field")),
            ("learn_limit".to_owned(), "$.examples".to_owned()),
            "{}",
            written(&told)
        );
    }

    #[test]
    fn w1_a_clock_that_never_reaches_the_deadline_never_stops_or_refuses_on_budget() {
        let Some(host) = device("law_learn w1") else {
            return;
        };
        let (full, _) = whole(&host);
        let each = patience() / (4 * top_words(&host, &full, 16));
        let mut answers = Vec::new();
        for budget in [1, 2, LONG] {
            let (mut mcp, fake) = timed(&host);
            disarm();
            builds_slowed_by(each);
            let got = mcp.raw("grit_law_learn", &within(&arguments(), budget));
            let measured = calibrations();
            disarm();
            let got = result(&got, false);
            assert_eq!(fake.elapsed(), Duration::ZERO);
            assert!(measured >= 1, "budget {budget}: the builds were measured");
            assert_eq!(
                text_at(&got, "stopped"),
                "ladder",
                "budget {budget}: {}",
                written(&got)
            );
            answers.push(written(&got));
        }
        assert_eq!(answers[0], answers[1]);
        assert_eq!(answers[0], answers[2]);
        assert_eq!(answers[0], written(&full));
    }

    #[test]
    fn w2_a_deadline_passed_at_a_named_point_answers_the_rungs_before_it_the_same_again() {
        let Some(host) = device("law_learn w2") else {
            return;
        };
        let (full, counts, made, sent) = frozen(&host);
        let (calibrated, built, submitted) = (counts[0], counts[1], counts[2]);
        assert_eq!(built, made, "every build point was followed by a build");
        assert_eq!(submitted, sent, "every submission point by a submission");
        assert!(
            built >= 4 && submitted >= 200,
            "{built} builds, {submitted} submissions"
        );
        let looks = submitted / 50;
        let cases = [
            (Point::Calibration, 0, false),
            (Point::Calibration, calibrated - 1, false),
            (Point::Build, 0, false),
            (Point::Build, built / 2, true),
            (Point::Build, built - 1, true),
            (Point::Submission, 0, false),
            (Point::Submission, looks / 2 * 50 + 25, true),
            (Point::Submission, submitted - 1, true),
        ];
        for (point, ordinal, answers) in cases {
            let mut seen = Vec::new();
            for _ in 0..2 {
                let (got, at, builds_after, sent_after) = braked_at(&host, point, ordinal);
                assert_eq!(
                    at,
                    ordinal + 1,
                    "{}: no later point was reached",
                    point.name()
                );
                if point == Point::Build {
                    assert_eq!(builds_after, ordinal, "no build started after the deadline");
                }
                if point == Point::Submission {
                    assert_eq!(sent_after, ordinal, "no submission after the deadline");
                }
                let held = on_budget(&got, &full, point, ordinal);
                assert_eq!(
                    held.is_some(),
                    answers,
                    "{} {ordinal}: {}",
                    point.name(),
                    written(&got)
                );
                seen.push(written(&got));
            }
            assert_eq!(seen[0], seen[1], "{} {ordinal}", point.name());
        }
    }

    #[test]
    fn w3_once_the_deadline_passes_inside_a_look_no_further_submission_starts() {
        let Some(host) = device("law_learn w3") else {
            return;
        };
        let (_, counts, _, sent) = frozen(&host);
        assert_eq!(counts[2], sent);
        for ordinal in [25, sent / 50 / 2 * 50 + 17] {
            let (_, at, _, after) = braked_at(&host, Point::Submission, ordinal);
            assert_eq!(at, ordinal + 1);
            assert_eq!(
                after, ordinal,
                "the deadline passed before submission {ordinal}, inside a look of 50"
            );
        }
    }

    #[test]
    fn c1_a_deadline_passed_during_the_count_changes_no_byte_of_the_answer() {
        let Some(host) = device("law_learn c1") else {
            return;
        };
        let (full, counts, _, _) = frozen(&host);
        assert!(counts[4] >= 1, "the count was checked");
        let (got, at, _, _) = braked_at(&host, Point::Count, 0);
        assert_eq!(at, counts[4]);
        assert_eq!(written(&result(&got, false)), written(&full));
    }

    #[test]
    fn c2_count_units_are_the_hand_counted_blocks_times_inputs_and_law_gates() {
        let fake = Fake::new();
        let minimal = Count {
            inputs: 9,
            outputs: 1,
            copies: 1,
        };
        let square = Count {
            inputs: 9,
            outputs: 1,
            copies: 8,
        };
        let cube = Count {
            inputs: 27,
            outputs: 1,
            copies: 48,
        };
        let cube_two = Count {
            inputs: 27,
            outputs: 2,
            copies: 48,
        };
        let small = Count {
            inputs: 3,
            outputs: 1,
            copies: 1,
        };
        for _ in 0..2 {
            assert_eq!(minimal.blocks(), 8);
            assert_eq!(minimal.units(5), 8 * (9 + 5));
            assert_eq!(square.law_gates(5), 47);
            assert_eq!(square.units(5), 8 * (9 + 47));
            assert_eq!(cube.blocks(), 1 << 21);
            assert_eq!(cube.law_gates(8), 431);
            assert_eq!(cube.units(8), 960_495_616);
            assert_eq!(cube_two.law_gates(8), 478);
            assert_eq!(cube_two.units(8), 1_059_061_760);
            assert_eq!(small.blocks(), 1);
            assert_eq!(small.units(3), 6);
        }
        assert_eq!(fake.reads(), 0);
        assert_eq!(fake.elapsed(), Duration::ZERO);
    }

    #[test]
    fn p2_a_later_slower_calibration_never_lowers_the_admitted_rung() {
        let Some(host) = device("law_learn p2") else {
            return;
        };
        let machine =
            grit::machine::Machine::over(0, 27, 1, 64, grit::law::chip::KINDS, 1).unwrap();
        let top = grit::marking::stepped_rolled(host.width(), machine, 2)
            .unwrap()
            .len();
        let each = patience() / u32::try_from(top).unwrap();
        let mut mcp = Mcp::new(Some(&host));
        disarm();
        builds_slowed_by(each);
        let plain = mcp.no("grit_law_learn", &copied(73, [4, 4, 4], 64));
        let measured = calibrations();
        disarm();
        assert_eq!(
            text_at(&plain, "refused"),
            "learn_limit",
            "{}",
            written(&plain)
        );
        assert!(measured >= 2, "{measured} calibrations");
        for slowed in [patience(), patience() * 1000] {
            let mut mcp = Mcp::new(Some(&host));
            disarm();
            builds_slowed_by(each);
            calibration_slowed(measured - 1, slowed);
            let told = mcp.no("grit_law_learn", &copied(73, [4, 4, 4], 64));
            let again = calibrations();
            let built = kernel_most();
            disarm();
            assert_eq!(
                text_at(&told, "refused"),
                "learn_limit",
                "{}",
                written(&told)
            );
            assert_eq!(again, measured);
            assert_eq!(built, 0);
            assert_eq!(
                int_at(&told, "limit"),
                int_at(&plain, "limit"),
                "a slower last sample lowered the admitted rung: {}",
                written(&told)
            );
        }
    }

    struct Run {
        exec: Exec,
        answers: Receiver<Json>,
        id: i64,
    }

    impl Run {
        fn new() -> Self {
            let pattern = std::env::var("GRIT_DEVICE")
                .ok()
                .filter(|one| !one.is_empty());
            let (exec, answers) = Exec::spawn(
                Box::new(move || Host::matching(pattern.as_deref())),
                &Options::default(),
            )
            .unwrap();
            Self {
                exec,
                answers,
                id: 100,
            }
        }

        fn ask(&mut self, name: &str, arguments: &str) -> Json {
            self.id += 1;
            self.exec.line(&format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":{},\"method\":\"tools/call\",\"params\":{{\"name\":\"{name}\",\"arguments\":{arguments}}}}}",
                self.id
            ));
            loop {
                let got = self.answers.recv_timeout(Duration::from_secs(600)).unwrap();
                if got.get("id").and_then(Json::int) == Some(self.id) {
                    return got;
                }
            }
        }
    }

    #[test]
    fn a_device_lost_during_a_ladder_is_refused_and_the_next_answer_is_as_before() {
        let Some(host) = device("law_learn lost device") else {
            return;
        };
        drop(host);
        let mut run = Run::new();
        let clean = written(&result(&run.ask("grit_law_learn", &arguments()), false));
        run.exec.faults().lock().unwrap().wedge_after_checks = Some(5);
        let told = result(&run.ask("grit_law_learn", &arguments()), true);
        assert_eq!(
            text_at(&told, "refused"),
            "device_lost",
            "{}",
            written(&told)
        );
        let again = written(&result(&run.ask("grit_law_learn", &arguments()), false));
        assert_eq!(again, clean);
        run.exec.faults().lock().unwrap().wedge_before_call = true;
        let told = result(&run.ask("grit_law_learn", &arguments()), true);
        assert_eq!(
            text_at(&told, "refused"),
            "device_lost",
            "{}",
            written(&told)
        );
        let again = written(&result(&run.ask("grit_law_learn", &arguments()), false));
        assert_eq!(again, clean);
        run.exec.finish();
    }
}

fn own_group(axes: usize) -> Vec<[[i64; 3]; 3]> {
    let orders: Vec<Vec<usize>> = if axes == 2 {
        vec![vec![0, 1], vec![1, 0]]
    } else {
        vec![
            vec![0, 1, 2],
            vec![0, 2, 1],
            vec![1, 0, 2],
            vec![1, 2, 0],
            vec![2, 0, 1],
            vec![2, 1, 0],
        ]
    };
    let mut out = Vec::new();
    for order in &orders {
        for signs in 0..(1 << axes) {
            let mut matrix = [[0_i64; 3]; 3];
            for row in 0..3 {
                if row < axes {
                    matrix[row][order[row]] = if (signs >> row) & 1 == 1 { -1 } else { 1 };
                } else {
                    matrix[row][row] = 1;
                }
            }
            out.push(matrix);
        }
    }
    out
}

fn own_place(at: [i64; 3], axes: usize) -> u32 {
    let layer = if axes == 2 { 0 } else { (at[2] + 1) * 9 };
    u32::try_from(layer + (at[1] + 1) * 3 + (at[0] + 1)).unwrap()
}

fn own_offset(place: u32, axes: usize) -> [i64; 3] {
    let place = i64::from(place);
    let dz = if axes == 2 { 0 } else { place / 9 - 1 };
    [place % 3 - 1, place / 3 % 3 - 1, dz]
}

fn own_image(row: u32, matrix: &[[i64; 3]; 3], axes: usize) -> u32 {
    let places = if axes == 2 { 9 } else { 27 };
    (0..places)
        .filter(|place| (row >> place) & 1 == 1)
        .map(|place| {
            let from = own_offset(place, axes);
            let mut to = [0_i64; 3];
            for (row, slot) in to.iter_mut().enumerate() {
                *slot = (0..3).map(|col| matrix[row][col] * from[col]).sum();
            }
            1 << own_place(to, axes)
        })
        .fold(0, |held, bit| held | bit)
}

fn refused_under_each_symmetry(host: &Host, stencil: Stencil, chain: &[[i64; 3]], size: [u32; 3]) {
    let axes = stencil.axes();
    let group = own_group(axes);
    let pattern = chain
        .iter()
        .fold(0_u32, |held, at| held | 1 << own_place(*at, axes));
    let orbit: BTreeSet<u32> = group
        .iter()
        .map(|matrix| own_image(pattern, matrix, axes))
        .collect();
    assert_eq!(
        orbit.len(),
        group.len(),
        "a pattern no symmetry but one fixes"
    );
    let least = *orbit.iter().next().unwrap();
    let first = [3, 3, if axes == 2 { 0 } else { 3 }];
    let second = [11, 3, if axes == 2 { 0 } else { 3 }];
    let mut mcp = Mcp::new(Some(host));
    let cells = size.iter().product::<u32>() as usize;
    for matrix in group.iter().skip(1) {
        let other = own_image(least, matrix, axes);
        let mut input = vec![0_u32; cells];
        for (centre, row) in [(first, least), (second, other)] {
            for place in 0..stencil.positions() {
                if (row >> place) & 1 == 1 {
                    let at = own_offset(place, axes);
                    let spot = [0, 1, 2]
                        .map(|axis| u32::try_from(i64::from(centre[axis]) + at[axis]).unwrap());
                    input[stencil::index(size, spot)] = 1;
                }
            }
        }
        let mut output = vec![0_u32; cells];
        output[stencil::index(size, first)] = 1;
        let rows = own_rows(stencil, size, &input, false);
        assert_eq!(rows[stencil::index(size, first)], least);
        assert_eq!(rows[stencil::index(size, second)], other);
        let inside = rows.iter().filter(|row| orbit.contains(row)).count();
        assert_eq!(inside, 2, "only the two patches see the pattern");
        let told = mcp.no(
            "grit_law_learn",
            &learn_args(
                stencil,
                "symmetric",
                "zero",
                &format!("[{}]", inline(size, &input, &output)),
                0,
                1,
                "",
            ),
        );
        assert_eq!(
            text_at(&told, "refused"),
            "asymmetric",
            "{matrix:?}: {}",
            written(&told)
        );
        let named: BTreeSet<(i64, i64)> = told
            .get("orbits")
            .and_then(Json::list)
            .unwrap()
            .iter()
            .flat_map(|one| one.get("rows").and_then(Json::list).unwrap().iter())
            .map(|one| (int_at(one, "row"), int_at(one, "outputs")))
            .collect();
        assert!(named.contains(&(i64::from(least), 1)), "{named:?}");
        assert!(named.contains(&(i64::from(other), 0)), "{named:?}");
    }
}

#[test]
fn l3_a_pattern_and_any_one_of_its_images_seen_apart_are_refused_by_name() {
    let Some(host) = device("law_learn l3 every symmetry") else {
        return;
    };
    refused_under_each_symmetry(
        &host,
        Stencil::Square,
        &[[0, 0, 0], [0, -1, 0], [1, 1, 0]],
        [16, 8, 1],
    );
    refused_under_each_symmetry(
        &host,
        Stencil::Cube,
        &[[0, 0, 0], [-1, -1, -1], [1, 0, -1]],
        [16, 8, 8],
    );
}

fn scores_of(
    host: &Host,
    module: &[u32],
    held: grit::machine::Machine,
    wide: u32,
    told: bool,
) -> Vec<u32> {
    use grit::Groups;
    use grit::marking::{BINDINGS, FEED, SCORE, TRIAL, VALUES, WANTED};
    const CROWD: u32 = 256;
    let shape = held.shape().unwrap();
    let mut rng = Rng(0x5EED);
    let mut words = |many: u32| -> Vec<u32> { (0..many).map(|_| rng.next() as u32).collect() };
    let mut feed = words(held.steps * held.inputs * wide);
    if told {
        feed.push(wide);
    }
    let want = words(held.steps * held.outputs * wide);
    let mut state = 0x1234_5679_u32;
    let chips: Vec<u32> = (0..CROWD * shape.gates)
        .map(|at| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            grit::netlist::drawn(&shape, at % shape.gates, state).pack()
        })
        .collect();
    let sizes = [
        (CROWD * shape.gates) as usize,
        (CROWD * shape.gates) as usize,
        (CROWD * shape.sources() * wide) as usize,
        want.len(),
        CROWD as usize,
        CROWD as usize,
        CROWD as usize,
        feed.len(),
    ];
    assert_eq!(sizes.len(), BINDINGS as usize);
    let modules: Vec<&[u32]> = vec![module];
    let mut resident = host.resident(&sizes, &modules).unwrap();
    resident.write(TRIAL as usize, &chips).unwrap();
    resident
        .write(VALUES as usize, &vec![0_u32; sizes[VALUES as usize]])
        .unwrap();
    resident.write(WANTED as usize, &want).unwrap();
    resident.write(FEED as usize, &feed).unwrap();
    resident
        .write(SCORE as usize, &vec![0_u32; CROWD as usize])
        .unwrap();
    let over = [Groups::Fixed(CROWD.div_ceil(host.width()))];
    let stages: Vec<&[Groups]> = vec![&over];
    resident.staged(&stages).unwrap();
    resident.read(SCORE as usize, CROWD as usize).unwrap()
}

#[test]
fn a_search_kernel_rolled_over_its_rows_scores_every_candidate_as_the_unrolled_one() {
    let Some(host) = device("law_learn rolled kernel") else {
        return;
    };
    for (state, inputs, outputs, gates, steps, wide) in
        [(0, 27, 1, 8, 1, 3), (2, 3, 2, 9, 3, 5), (0, 9, 1, 14, 1, 1)]
    {
        let held = grit::machine::Machine::over(state, inputs, outputs, gates, 16, steps).unwrap();
        let unrolled = grit::marking::stepped(host.width(), held, wide).unwrap();
        let rolled = grit::marking::stepped_rolled(host.width(), held, wide).unwrap();
        let before = scores_of(&host, &unrolled, held, wide, false);
        assert!(before.iter().any(|one| *one > 0));
        assert_eq!(
            scores_of(&host, &rolled, held, wide, true),
            before,
            "state {state}, inputs {inputs}, outputs {outputs}, gates {gates}, steps {steps}, wide {wide}"
        );
    }
}
