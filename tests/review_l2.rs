#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::device;

use grit::serve3::{Options, Server};
use grit::wire::{Json, written};

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
        (self.next() % most) as u32
    }
}

fn call(server: &mut Server<'_>, name: &str, arguments: &str) -> Json {
    let line = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"tools/call\",\"params\":{{\"name\":\"{name}\",\"arguments\":{arguments}}}}}"
    );
    server.heard(&line).unwrap()
}

fn res(got: &Json, error: bool) -> Json {
    let held = got
        .get("result")
        .unwrap_or_else(|| panic!("{}", written(got)));
    let text = written(got);
    assert_eq!(
        held.get("isError"),
        Some(&Json::Bool(error)),
        "{}",
        &text[..text.len().min(3000)]
    );
    held.get("structuredContent").cloned().unwrap()
}

fn ok(server: &mut Server<'_>, name: &str, arguments: &str) -> Json {
    res(&call(server, name, arguments), false)
}

fn no(server: &mut Server<'_>, name: &str, arguments: &str) -> Json {
    res(&call(server, name, arguments), true)
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

fn ints(held: &Json) -> Vec<i64> {
    held.list()
        .unwrap()
        .iter()
        .map(|one| one.int().unwrap())
        .collect()
}

fn list(words: &[u32]) -> String {
    let parts: Vec<String> = words.iter().map(u32::to_string).collect();
    format!("[{}]", parts.join(","))
}

#[derive(Clone, Copy)]
struct G {
    kind: u32,
    left: u32,
    right: u32,
}

fn eval(inputs: u32, outputs: u32, gates: &[G], row: u32) -> u32 {
    let mut values: Vec<u32> = Vec::new();
    for gate in gates {
        let pick = |at: u32, values: &Vec<u32>| {
            if at < inputs {
                (row >> at) & 1
            } else {
                values[(at - inputs) as usize]
            }
        };
        let l = pick(gate.left, &values);
        let r = pick(gate.right, &values);
        values.push((gate.kind >> (l | (r << 1))) & 1);
    }
    let first = values.len() - outputs as usize;
    (0..outputs).fold(0, |held, at| held | (values[first + at as usize] << at))
}

fn gates_of(held: &Json) -> Vec<G> {
    held.list()
        .unwrap()
        .iter()
        .map(|one| G {
            kind: int_at(one, "kind") as u32,
            left: int_at(one, "left") as u32,
            right: int_at(one, "right") as u32,
        })
        .collect()
}

fn random_gates(rng: &mut Rng, inputs: u32, many: u32) -> Vec<G> {
    (0..many)
        .map(|at| G {
            kind: [6, 8, 14, 9, 2, 4][rng.below(6) as usize],
            left: rng.below(u64::from(inputs + at)),
            right: rng.below(u64::from(inputs + at)),
        })
        .collect()
}

fn idx(size: [u32; 3], at: [u32; 3]) -> usize {
    ((at[2] * size[1] + at[1]) * size[0] + at[0]) as usize
}

fn offsets(cube: bool) -> Vec<[i64; 3]> {
    let mut out = Vec::new();
    let zs: Vec<i64> = if cube { vec![-1, 0, 1] } else { vec![0] };
    for dz in &zs {
        for dy in -1..=1 {
            for dx in -1..=1 {
                out.push([dx, dy, *dz]);
            }
        }
    }
    out
}

fn hood(cube: bool, size: [u32; 3], planes: &[Vec<u32>], at: [u32; 3], wrap: bool) -> u32 {
    let places = offsets(cube);
    let positions = places.len() as u32;
    let mut row = 0;
    for (read, bits) in planes.iter().enumerate() {
        for (place, off) in places.iter().enumerate() {
            let mut spot = [0_u32; 3];
            let mut inside = true;
            for axis in 0..3 {
                let side = i64::from(size[axis]);
                let mut value = i64::from(at[axis]) + off[axis];
                if wrap {
                    value = value.rem_euclid(side);
                }
                if (0..side).contains(&value) {
                    spot[axis] = value as u32;
                } else {
                    inside = false;
                }
            }
            if inside && bits[idx(size, spot)] & 1 == 1 {
                row |= 1 << (read as u32 * positions + place as u32);
            }
        }
    }
    row
}

fn all_at(size: [u32; 3]) -> Vec<[u32; 3]> {
    let mut out = Vec::new();
    for z in 0..size[2] {
        for y in 0..size[1] {
            for x in 0..size[0] {
                out.push([x, y, z]);
            }
        }
    }
    out
}

fn world(server: &mut Server<'_>, law: &str, size: [u32; 3]) -> i64 {
    let got = ok(
        server,
        "grit_world_create",
        &format!(
            "{{\"wide\":{},\"deep\":{},\"tall\":{},\"law\":{}}}",
            size[0],
            size[1],
            size[2],
            written(&Json::Text(law.to_owned()))
        ),
    );
    int_at(&got, "world")
}

fn write(server: &mut Server<'_>, number: i64, plane: u32, cells: &[([u32; 3], u32)]) {
    for piece in cells.chunks(2048) {
        let parts: Vec<String> = piece
            .iter()
            .map(|(at, word)| format!("[{},{},{},{word}]", at[0], at[1], at[2]))
            .collect();
        ok(
            server,
            "grit_world_write",
            &format!(
                "{{\"world\":{number},\"plane\":{plane},\"cells\":[{}],\"budget_ms\":600000}}",
                parts.join(",")
            ),
        );
    }
}

fn read_box(server: &mut Server<'_>, number: i64, plane: u32, size: [u32; 3]) -> Vec<u32> {
    let mut out = Vec::new();
    for z in 0..size[2] {
        for y in 0..size[1] {
            let got = ok(
                server,
                "grit_world_read",
                &format!(
                    "{{\"world\":{number},\"plane\":{plane},\"from\":[0,{y},{z}],\"to\":[{},{y},{z}]}}",
                    size[0] - 1
                ),
            );
            out.extend(
                ints(got.get("cells").unwrap())
                    .into_iter()
                    .map(|one| one as u32),
            );
        }
    }
    out
}

fn step(server: &mut Server<'_>, number: i64) {
    ok(
        server,
        "grit_world_step",
        &format!("{{\"world\":{number},\"frames\":1,\"budget_ms\":600000}}"),
    );
}

fn rounded(size: [u32; 3]) -> [u32; 3] {
    size.map(|side| side.next_power_of_two().max(2))
}

#[test]
fn r1_zero_border_two_reads_two_writes_on_odd_examples_runs_as_truth_on_padded_worlds() {
    let Some(host) = device("review l2 r1") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let reads: [(u32, u32); 2] = [(2, 3), (0, 0)];
    let writes: [(u32, u32); 2] = [(1, 5), (3, 0)];
    for seed in [3_u64, 4] {
        let mut rng = Rng(seed);
        let truth = random_gates(&mut rng, 18, 5);
        let truth = {
            let mut held = truth;
            let n = held.len() as u32 + 18;
            held.push(G {
                kind: 10,
                left: n - 2,
                right: n - 2,
            });
            held
        };
        let sizes = [[5_u32, 7, 1], [6, 3, 1], [7, 5, 1]];
        let mut examples = Vec::new();
        let mut owned = Vec::new();
        let mut seen: BTreeMap<u32, u32> = BTreeMap::new();
        for size in sizes {
            let cells = (size[0] * size[1] * size[2]) as usize;
            let a: Vec<u32> = (0..cells).map(|_| rng.below(2)).collect();
            let b: Vec<u32> = (0..cells).map(|_| rng.below(2)).collect();
            let planes = vec![a.clone(), b.clone()];
            let mut out0 = Vec::new();
            let mut out1 = Vec::new();
            for at in all_at(size) {
                let row = hood(false, size, &planes, at, false);
                let out = eval(18, 2, &truth, row);
                seen.insert(row, out);
                out0.push(out & 1);
                out1.push(out >> 1);
            }
            examples.push(format!(
                "{{\"size\":[{},{},{}],\"input\":[{},{}],\"output\":[{},{}]}}",
                size[0],
                size[1],
                size[2],
                list(&a.iter().map(|v| v << reads[0].1).collect::<Vec<_>>()),
                list(&b.iter().map(|v| v << reads[1].1).collect::<Vec<_>>()),
                list(&out0.iter().map(|v| v << writes[0].1).collect::<Vec<_>>()),
                list(&out1.iter().map(|v| v << writes[1].1).collect::<Vec<_>>())
            ));
            owned.push((size, a, b, out0, out1));
        }
        let got = ok(
            &mut server,
            "grit_law_learn",
            &format!(
                "{{\"stencil\":\"3x3\",\"reads\":[{{\"plane\":{},\"bit\":{}}},{{\"plane\":{},\"bit\":{}}}],\"writes\":[{{\"plane\":{},\"bit\":{}}},{{\"plane\":{},\"bit\":{}}}],\"channels\":4,\"border\":\"zero\",\"rule\":\"minimal\",\"examples\":[{}],\"gates\":7,\"budget_ms\":600000}}",
                reads[0].0,
                reads[0].1,
                reads[1].0,
                reads[1].1,
                writes[0].0,
                writes[0].1,
                writes[1].0,
                writes[1].1,
                examples.join(",")
            ),
        );
        let report = got.get("examples").unwrap();
        assert_eq!(int_at(report, "seen"), seen.len() as i64, "seen count");
        assert_eq!(int_at(report, "rows"), 1 << 18);
        assert_eq!(
            int_at(got.get("unseen").unwrap(), "rows"),
            (1 << 18) - seen.len() as i64
        );
        let chip = gates_of(got.get("chip").unwrap().get("gates").unwrap());
        for (row, out) in &seen {
            assert_eq!(eval(18, 2, &chip, *row), *out, "chip on seen row {row}");
        }
        let law = text_at(&got, "law");
        for (size, a, b, out0, out1) in &owned {
            let room = [size[0] + 1, size[1] + 1, 2];
            let number = world(&mut server, &law, room);
            let mut cells_a = Vec::new();
            let mut cells_b = Vec::new();
            for at in all_at(*size) {
                let i = idx(*size, at);
                let noise_a = rng.below(1 << 16) & !(1 << reads[0].1);
                let noise_b = rng.below(1 << 16) & !(1 << reads[1].1);
                cells_a.push((at, noise_a | (a[i] << reads[0].1)));
                cells_b.push((at, noise_b | (b[i] << reads[1].1)));
            }
            write(&mut server, number, reads[0].0, &cells_a);
            write(&mut server, number, reads[1].0, &cells_b);
            step(&mut server, number);
            let full = rounded(room);
            let p1 = read_box(&mut server, number, writes[0].0, full);
            let p3 = read_box(&mut server, number, writes[1].0, full);
            for at in all_at(*size) {
                let i = idx(*size, at);
                let j = idx(full, at);
                assert_eq!(
                    (p1[j] >> writes[0].1) & 1,
                    out0[i],
                    "seed {seed} size {size:?} output 0 at {at:?}"
                );
                assert_eq!(
                    (p3[j] >> writes[1].1) & 1,
                    out1[i],
                    "seed {seed} size {size:?} output 1 at {at:?}"
                );
            }
            ok(
                &mut server,
                "grit_world_close",
                &format!("{{\"world\":{number}}}"),
            );
        }
    }
}

fn identity_law(server: &mut Server<'_>) -> String {
    let got = ok(
        server,
        "grit_law_from_chip",
        "{\"inputs\":1,\"outputs\":1,\"gates\":[{\"kind\":10,\"left\":0,\"right\":0}],\"bind_inputs\":[{\"plane\":2,\"bit\":0}],\"bind_outputs\":[{\"plane\":3,\"bit\":0}],\"channels\":4}",
    );
    text_at(&got, "law")
}

#[test]
fn r2_cube_wrapped_box_on_a_rounded_world_runs_as_truth_inside_the_box() {
    let Some(host) = device("review l2 r2") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let asked = [6_u32, 3, 3];
    let full = rounded(asked);
    let base = identity_law(&mut server);
    for seed in [7_u64, 8] {
        let mut rng = Rng(seed);
        let mut truth = random_gates(&mut rng, 27, 4);
        let n = truth.len() as u32 + 27;
        truth.push(G {
            kind: 6,
            left: n - 1,
            right: rng.below(27),
        });
        let input: Vec<u32> = (0..full.iter().product::<u32>())
            .map(|_| rng.below(2))
            .collect();
        let outs: Vec<u32> = all_at(full)
            .into_iter()
            .map(|at| {
                eval(
                    27,
                    1,
                    &truth,
                    hood(true, full, std::slice::from_ref(&input), at, true),
                )
            })
            .collect();
        let example = world(&mut server, &base, asked);
        let cells_in: Vec<([u32; 3], u32)> = all_at(full)
            .into_iter()
            .map(|at| (at, input[idx(full, at)]))
            .collect();
        let cells_out: Vec<([u32; 3], u32)> = all_at(full)
            .into_iter()
            .map(|at| (at, outs[idx(full, at)]))
            .collect();
        write(&mut server, example, 0, &cells_in);
        write(&mut server, example, 1, &cells_out);
        let from = [2_u32, 1, 0];
        let to = [6_u32, 3, 3];
        let got = ok(
            &mut server,
            "grit_law_learn",
            &format!(
                "{{\"stencil\":\"3x3x3\",\"reads\":[{{\"plane\":0,\"bit\":0}}],\"writes\":[{{\"plane\":1,\"bit\":0}}],\"border\":\"wrapped\",\"rule\":\"minimal\",\"examples\":[{{\"world\":{example},\"from\":[{},{},{}],\"to\":[{},{},{}]}}],\"gates\":10,\"budget_ms\":600000}}",
                from[0], from[1], from[2], to[0], to[1], to[2]
            ),
        );
        let mut seen = BTreeSet::new();
        for at in all_at(full) {
            if (0..3).all(|axis| at[axis] >= from[axis] && at[axis] <= to[axis]) {
                seen.insert(hood(true, full, std::slice::from_ref(&input), at, true));
            }
        }
        assert_eq!(
            int_at(got.get("examples").unwrap(), "seen"),
            seen.len() as i64
        );
        let law = text_at(&got, "law");
        let run = world(&mut server, &law, asked);
        write(&mut server, run, 0, &cells_in);
        step(&mut server, run);
        let got_out = read_box(&mut server, run, 1, full);
        for at in all_at(full) {
            if (0..3).all(|axis| at[axis] >= from[axis] && at[axis] <= to[axis]) {
                assert_eq!(
                    got_out[idx(full, at)] & 1,
                    outs[idx(full, at)],
                    "seed {seed} at {at:?}"
                );
            }
        }
        ok(
            &mut server,
            "grit_world_close",
            &format!("{{\"world\":{run}}}"),
        );
        ok(
            &mut server,
            "grit_world_close",
            &format!("{{\"world\":{example}}}"),
        );
    }
}

#[cfg(feature = "faults")]
fn d4() -> Vec<[[i64; 2]; 2]> {
    let mut out = Vec::new();
    for swap in [false, true] {
        for sx in [1, -1] {
            for sy in [1, -1] {
                let m = if swap {
                    [[0, sx], [sy, 0]]
                } else {
                    [[sx, 0], [0, sy]]
                };
                out.push(m);
            }
        }
    }
    out
}

#[cfg(feature = "faults")]
fn turned(row: u32, m: [[i64; 2]; 2]) -> u32 {
    let mut out = 0;
    for place in 0..9_i64 {
        if (row >> place) & 1 == 0 {
            continue;
        }
        let (x, y) = (place % 3 - 1, place / 3 - 1);
        let nx = m[0][0] * x + m[0][1] * y;
        let ny = m[1][0] * x + m[1][1] * y;
        out |= 1 << ((ny + 1) * 3 + nx + 1);
    }
    out
}

#[cfg(feature = "faults")]
fn tiled_run(server: &mut Server<'_>, law: &str) -> Vec<u32> {
    let size = [128_u32, 64, 2];
    let number = world(server, law, size);
    let mut cells = Vec::new();
    for row in 0..512_u32 {
        let cx = (row % 32) * 4 + 1;
        let cy = (row / 32) * 4 + 1;
        for place in 0..9 {
            if (row >> place) & 1 == 1 {
                cells.push(([cx + place % 3 - 1, cy + place / 3 - 1, 0], 1));
            }
        }
    }
    write(server, number, 0, &cells);
    step(server, number);
    let out = read_box(server, number, 1, [128, 64, 2]);
    ok(
        server,
        "grit_world_close",
        &format!("{{\"world\":{number}}}"),
    );
    (0..512_u32)
        .map(|row| out[idx([128, 64, 2], [(row % 32) * 4 + 1, (row / 32) * 4 + 1, 0])] & 1)
        .collect()
}

#[cfg(feature = "faults")]
fn frozen(host: &grit::Host) -> (Server<'_>, std::sync::Arc<grit::clock::Fake>) {
    let fake = std::sync::Arc::new(grit::clock::Fake::new());
    let time: std::sync::Arc<dyn grit::clock::Time> = fake.clone();
    let server = Server::timed(Some(host), &Options::default(), time).unwrap();
    (server, fake)
}

#[cfg(feature = "faults")]
#[test]
fn r3_every_row_runs_on_a_world_as_the_reported_table_and_symmetric_tables_are_invariant() {
    let Some(host) = device("review l2 r3") else {
        return;
    };
    let (mut server, fake) = frozen(&host);
    let size = [8_u32, 8, 1];
    let mut rng = Rng(31);
    let input: Vec<u32> = (0..64).map(|_| u32::from(rng.below(3) == 0)).collect();
    let truth = |row: u32| -> u32 {
        let c = (row >> 4) & 1;
        let cross = ((row >> 1) ^ (row >> 3) ^ (row >> 5) ^ (row >> 7)) & 1;
        c & cross | (u32::from((row & 0x145) == 0x145))
    };
    let mut seen: BTreeMap<u32, u32> = BTreeMap::new();
    let mut output = Vec::new();
    for at in all_at(size) {
        let row = hood(false, size, std::slice::from_ref(&input), at, true);
        seen.insert(row, truth(row));
        output.push(truth(row));
    }
    for rule in ["minimal", "symmetric"] {
        let args = format!(
            "{{\"stencil\":\"3x3\",\"reads\":[{{\"plane\":0,\"bit\":0}}],\"writes\":[{{\"plane\":1,\"bit\":0}}],\"border\":\"wrapped\",\"rule\":\"{rule}\",\"examples\":[{{\"size\":[8,8,1],\"input\":[{}],\"output\":[{}]}}],\"gates\":14,\"budget_ms\":600000}}",
            list(&input),
            list(&output)
        );
        let got = ok(&mut server, "grit_law_learn", &args);
        let again = ok(&mut server, "grit_law_learn", &args);
        assert_eq!(text_at(&got, "stopped"), "ladder", "rule {rule}");
        assert_eq!(written(&got), written(&again), "same bytes, rule {rule}");
        assert_eq!(fake.elapsed(), std::time::Duration::ZERO);
        let unseen = got.get("unseen").unwrap();
        let table: Vec<u32> = ints(unseen.get("table").unwrap())
            .into_iter()
            .map(|one| one as u32)
            .collect();
        let listed: Vec<u32> = ints(unseen.get("unseen_rows").unwrap())
            .into_iter()
            .map(|one| one as u32)
            .collect();
        let mine: Vec<u32> = (0..512).filter(|row| !seen.contains_key(row)).collect();
        assert_eq!(listed, mine, "unseen rows, rule {rule}");
        assert_eq!(int_at(unseen, "rows"), mine.len() as i64);
        let ones = mine.iter().filter(|row| table[**row as usize] == 1).count() as i64;
        assert_eq!(
            ints(unseen.get("ones").unwrap()),
            vec![ones],
            "ones, rule {rule}"
        );
        assert_eq!(
            ints(unseen.get("zeros").unwrap()),
            vec![mine.len() as i64 - ones]
        );
        for (row, out) in &seen {
            assert_eq!(
                table[*row as usize], *out,
                "seen row {row} in table, rule {rule}"
            );
        }
        let law = text_at(&got, "law");
        let ran = tiled_run(&mut server, &law);
        assert_eq!(
            ran, table,
            "every row on a world equals the reported table, rule {rule}"
        );
        if rule == "symmetric" {
            for m in d4() {
                for row in 0..512 {
                    assert_eq!(
                        table[row as usize],
                        table[turned(row, m) as usize],
                        "row {row} under {m:?}"
                    );
                }
            }
        }
    }
    let r = 0b000_000_011_u32;
    let image = turned(r, [[-1, 0], [0, 1]]);
    assert_ne!(r, image);
    let mut a = vec![0_u32; 64];
    let mut out = vec![0_u32; 64];
    a[idx(size, [1, 1, 0])] = 1;
    a[idx(size, [2, 1, 0])] = 1;
    let row_left = hood(false, size, &[a.clone()], [2, 1, 0], true);
    let row_right = hood(false, size, &[a.clone()], [1, 1, 0], true);
    assert_eq!(row_right, turned(row_left, [[-1, 0], [0, 1]]));
    out[idx(size, [2, 1, 0])] = 1;
    let told = no(
        &mut server,
        "grit_law_learn",
        &format!(
            "{{\"stencil\":\"3x3\",\"reads\":[{{\"plane\":0,\"bit\":0}}],\"writes\":[{{\"plane\":1,\"bit\":0}}],\"border\":\"wrapped\",\"rule\":\"symmetric\",\"examples\":[{{\"size\":[8,8,1],\"input\":[{}],\"output\":[{}]}}],\"budget_ms\":600000}}",
            list(&a),
            list(&out)
        ),
    );
    assert_eq!(
        text_at(&told, "refused"),
        "asymmetric",
        "{}",
        written(&told)
    );
}

#[cfg(feature = "faults")]
fn cube_group() -> Vec<[[i64; 3]; 3]> {
    let perms = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    let mut out = Vec::new();
    for p in perms {
        for signs in 0..8 {
            let mut m = [[0_i64; 3]; 3];
            for axis in 0..3 {
                m[axis][p[axis]] = if (signs >> axis) & 1 == 1 { -1 } else { 1 };
            }
            out.push(m);
        }
    }
    out
}

#[cfg(feature = "faults")]
fn cube_turned(row: u32, m: [[i64; 3]; 3]) -> u32 {
    let mut out = 0;
    for place in 0..27_i64 {
        if (row >> place) & 1 == 0 {
            continue;
        }
        let v = [place % 3 - 1, place / 3 % 3 - 1, place / 9 - 1];
        let n: Vec<i64> = (0..3)
            .map(|a| (0..3).map(|b| m[a][b] * v[b]).sum())
            .collect();
        out |= 1 << ((n[2] + 1) * 9 + (n[1] + 1) * 3 + n[0] + 1);
    }
    out
}

#[cfg(feature = "faults")]
#[test]
fn r4_a_symmetric_cube_law_is_invariant_under_48_moves_and_reports_its_unseen_counts() {
    let Some(host) = device("review l2 r4") else {
        return;
    };
    let (mut server, fake) = frozen(&host);
    let size = [4_u32, 4, 4];
    let mut rng = Rng(77);
    let input: Vec<u32> = (0..64).map(|_| u32::from(rng.below(4) == 0)).collect();
    let truth = |row: u32| {
        ((row >> 13)
            & ((row >> 4) ^ (row >> 10) ^ (row >> 12) ^ (row >> 14) ^ (row >> 16) ^ (row >> 22)))
            & 1
    };
    let mut output = Vec::new();
    let mut seen = BTreeSet::new();
    for at in all_at(size) {
        let row = hood(true, size, std::slice::from_ref(&input), at, true);
        seen.insert(row);
        output.push(truth(row));
    }
    let got = ok(
        &mut server,
        "grit_law_learn",
        &format!(
            "{{\"stencil\":\"3x3x3\",\"reads\":[{{\"plane\":0,\"bit\":0}}],\"writes\":[{{\"plane\":1,\"bit\":0}}],\"border\":\"wrapped\",\"rule\":\"symmetric\",\"examples\":[{{\"size\":[4,4,4],\"input\":[{}],\"output\":[{}]}}],\"gates\":8,\"budget_ms\":600000}}",
            list(&input),
            list(&output)
        ),
    );
    assert_eq!(fake.elapsed(), std::time::Duration::ZERO);
    assert_eq!(text_at(&got, "stopped"), "ladder");
    assert_eq!(
        int_at(got.get("examples").unwrap(), "seen"),
        seen.len() as i64
    );
    let chip = gates_of(got.get("chip").unwrap().get("gates").unwrap());
    let group = cube_group();
    assert_eq!(group.len(), 48);
    for _ in 0..4000 {
        let row = (rng.next() as u32) & ((1 << 27) - 1);
        let base = eval(27, 1, &chip, row);
        for m in &group {
            assert_eq!(eval(27, 1, &chip, cube_turned(row, *m)), base, "row {row}");
        }
    }
    let lanes: [u64; 6] = [
        0xAAAA_AAAA_AAAA_AAAA,
        0xCCCC_CCCC_CCCC_CCCC,
        0xF0F0_F0F0_F0F0_F0F0,
        0xFF00_FF00_FF00_FF00,
        0xFFFF_0000_FFFF_0000,
        0xFFFF_FFFF_0000_0000,
    ];
    let mut all = 0_i64;
    let mut values: Vec<u64> = Vec::with_capacity(27 + chip.len());
    for block in 0..(1_u64 << 21) {
        values.clear();
        for input in 0..27_u64 {
            values.push(if input < 6 {
                lanes[input as usize]
            } else if (block >> (input - 6)) & 1 == 1 {
                u64::MAX
            } else {
                0
            });
        }
        for gate in &chip {
            let l = values[gate.left as usize];
            let r = values[gate.right as usize];
            let mut v = 0;
            for (bit, mask) in [(!l & !r), (l & !r), (!l & r), (l & r)].iter().enumerate() {
                if (gate.kind >> bit) & 1 == 1 {
                    v |= mask;
                }
            }
            values.push(v);
        }
        all += i64::from(values.last().unwrap().count_ones());
    }
    let on_seen = seen
        .iter()
        .filter(|row| eval(27, 1, &chip, **row) == 1)
        .count() as i64;
    let ones = all - on_seen;
    let unseen = got.get("unseen").unwrap();
    assert_eq!(ints(unseen.get("ones").unwrap()), vec![ones]);
    assert_eq!(
        ints(unseen.get("zeros").unwrap()),
        vec![(1_i64 << 27) - seen.len() as i64 - ones]
    );
}

#[test]
fn r5_a_large_symmetric_cube_request_is_refused_within_its_budget() {
    let Some(host) = device("review l2 r5") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let size = [128_u32, 128, 64];
    let mut rng = Rng(5);
    let base = identity_law(&mut server);
    let number = world(&mut server, &base, size);
    let cells: Vec<([u32; 3], u32)> = all_at(size)
        .into_iter()
        .filter(|_| rng.below(2) == 1)
        .map(|at| (at, 1))
        .collect();
    write(&mut server, number, 0, &cells);
    for rule in ["minimal", "symmetric"] {
        let args = format!(
            "{{\"stencil\":\"3x3x3\",\"reads\":[{{\"plane\":0,\"bit\":0}}],\"writes\":[{{\"plane\":1,\"bit\":0}}],\"border\":\"wrapped\",\"rule\":\"{rule}\",\"examples\":[{{\"world\":{number}}}],\"budget_ms\":1000}}"
        );
        let began = std::time::Instant::now();
        let told = no(&mut server, "grit_law_learn", &args);
        let took = began.elapsed();
        eprintln!(
            "r5 {rule}: refused {} after {took:?}: {}",
            text_at(&told, "refused"),
            text_at(&told, "message")
        );
        assert!(
            took < std::time::Duration::from_millis(3000),
            "rule {rule}: a 1000 ms budget took {took:?} before its refusal"
        );
    }
}

fn journal_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("REVIEW_L2_JOURNAL").map(std::path::PathBuf::from)
}

#[test]
fn r6_a_journal_written_by_main_replays_the_same_cells() {
    let Some(dir) = journal_dir() else {
        return;
    };
    let Some(host) = device("review l2 r6") else {
        return;
    };
    let options = Options {
        dir: Some(dir.join("worlds")),
        memory: None,
    };
    let dump = dir.join("cells.txt");
    let mode = std::env::var("REVIEW_L2_MODE").unwrap();
    let size = [16_u32, 16, 2];
    if mode == "write" {
        let mut server = Server::with(Some(&host), &options).unwrap();
        let law = text_at(
            &ok(
                &mut server,
                "grit_law_from_chip",
                "{\"inputs\":2,\"outputs\":1,\"gates\":[{\"kind\":6,\"left\":0,\"right\":1}],\"bind_inputs\":[{\"plane\":0,\"dx\":1,\"dy\":0,\"dz\":0,\"bit\":0},{\"plane\":0,\"dx\":0,\"dy\":-1,\"dz\":1,\"bit\":2}],\"bind_outputs\":[{\"plane\":1,\"bit\":7}],\"channels\":3}",
            ),
            "law",
        );
        let number = world(&mut server, &law, size);
        let mut rng = Rng(99);
        for round in 0..3 {
            let cells: Vec<([u32; 3], u32)> = (0..40)
                .map(|_| {
                    (
                        [rng.below(16), rng.below(16), rng.below(2)],
                        rng.below(1 << 20),
                    )
                })
                .collect();
            let parts: Vec<String> = cells
                .iter()
                .map(|(at, word)| format!("[{},{},{},{word}]", at[0], at[1], at[2]))
                .collect();
            ok(
                &mut server,
                "grit_world_write",
                &format!("{{\"world\":{number},\"cells\":[{}]}}", parts.join(",")),
            );
            for _ in 0..=round {
                step(&mut server, number);
            }
        }
        let mut text = format!("{number}\n");
        for plane in 0..3 {
            text.push_str(&format!(
                "{:?}\n",
                read_box(&mut server, number, plane, size)
            ));
        }
        std::fs::write(&dump, text).unwrap();
        return;
    }
    let held = std::fs::read_to_string(&dump).unwrap();
    let mut lines = held.lines();
    let number: i64 = lines.next().unwrap().parse().unwrap();
    let mut server = Server::with(Some(&host), &options).unwrap();
    let opened = ok(
        &mut server,
        "grit_world_open",
        &format!("{{\"name\":\"world-{number}\"}}"),
    );
    let again = int_at(&opened, "world");
    for plane in 0..3 {
        assert_eq!(
            format!("{:?}", read_box(&mut server, again, plane, size)),
            lines.next().unwrap(),
            "plane {plane} after replay by this build"
        );
    }
    eprintln!("r6: opened {}", written(&opened));
}
