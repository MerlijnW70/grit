#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use grit::Host;
use grit::branch::{Made, Stepping, World};
use grit::law::Law;
use grit::serve3::Server;
use grit::wire::{Json, written};

const LONG: u64 = 600_000;

const ALL: i64 = 0xFFFF_FFFF;

const LIFE: Table = Table {
    birth: 1 << 3,
    survive: (1 << 2) | (1 << 3),
};

const LIFE_FULL: Table = Table {
    birth: 1 << 3,
    survive: (1 << 2) | (1 << 3) | (1 << 8),
};

const BAYS: Table = Table {
    birth: 1 << 6,
    survive: (1 << 5) | (1 << 6) | (1 << 7),
};

const BAYS_FULL: Table = Table {
    birth: 1 << 6,
    survive: (1 << 5) | (1 << 6) | (1 << 7) | (1 << 26),
};

const BLUR: &str = include_str!("halo3/blur2.law.json");

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Table {
    birth: u32,
    survive: u32,
}

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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Shape {
    Square,
    Cube,
}

impl Shape {
    const fn name(self) -> &'static str {
        match self {
            Self::Square => "3x3",
            Self::Cube => "3x3x3",
        }
    }

    const fn around(self) -> u32 {
        match self {
            Self::Square => 8,
            Self::Cube => 26,
        }
    }

    const fn reach(self) -> i64 {
        match self {
            Self::Square => 0,
            Self::Cube => 1,
        }
    }
}

fn index(size: [u32; 3], at: [u32; 3]) -> usize {
    (at[2] as usize * size[1] as usize + at[1] as usize) * size[0] as usize + at[0] as usize
}

fn cells(size: [u32; 3]) -> usize {
    size.iter().map(|side| *side as usize).product()
}

fn point(size: [u32; 3], at: usize) -> [u32; 3] {
    let x = at % size[0] as usize;
    let y = at / size[0] as usize % size[1] as usize;
    let z = at / (size[0] as usize * size[1] as usize);
    [x, y, z].map(|one| u32::try_from(one).unwrap())
}

fn wrapped(size: [u32; 3], at: [i64; 3]) -> [u32; 3] {
    [0, 1, 2].map(|axis| u32::try_from(at[axis].rem_euclid(i64::from(size[axis]))).unwrap())
}

fn generation(
    shape: Shape,
    table: Table,
    size: [u32; 3],
    words: &[u32],
    wrapping: bool,
    used: u32,
) -> Vec<u32> {
    let reach = shape.reach();
    let mut out = vec![0_u32; words.len()];
    let side = size.map(i64::from);
    for z in 0..size[2] {
        for y in 0..size[1] {
            for x in 0..size[0] {
                let mut counts = [0_u32; 32];
                for dz in -reach..=reach {
                    for dy in -1_i64..=1 {
                        for dx in -1_i64..=1 {
                            if dx == 0 && dy == 0 && dz == 0 {
                                continue;
                            }
                            let at = [i64::from(x) + dx, i64::from(y) + dy, i64::from(z) + dz];
                            let inside = at
                                .iter()
                                .zip(side)
                                .all(|(one, side)| (0..side).contains(one));
                            if !inside && !wrapping {
                                continue;
                            }
                            let word = words[index(size, wrapped(size, at))] & used;
                            if word == 0 {
                                continue;
                            }
                            for (bit, count) in counts.iter_mut().enumerate() {
                                *count += (word >> bit) & 1;
                            }
                        }
                    }
                }
                let here = index(size, [x, y, z]);
                let centre = words[here];
                let mut word = centre & !used;
                for (bit, count) in counts.iter().enumerate() {
                    if (used >> bit) & 1 == 0 {
                        continue;
                    }
                    let row = if (centre >> bit) & 1 == 1 {
                        table.survive
                    } else {
                        table.birth
                    };
                    word |= ((row >> count) & 1) << bit;
                }
                out[here] = word;
            }
        }
    }
    out
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

fn listed(held: &[u32]) -> String {
    let shown: Vec<String> = held.iter().map(u32::to_string).collect();
    format!("[{}]", shown.join(","))
}

fn quoted(text: &str) -> String {
    written(&Json::Text(text.to_owned()))
}

fn soup(rng: &mut Rng, size: [u32; 3], permille: u64) -> Vec<u32> {
    (0..cells(size))
        .map(|_| u32::from(rng.below(1000) < permille))
        .collect()
}

fn learned_on(server: &mut Server<'_>, shape: Shape, table: Table) -> String {
    let (size, densities): ([u32; 3], &[u64]) = match shape {
        Shape::Square => ([32, 32, 1], &[250, 300, 350, 400, 450, 500, 600, 700, 930]),
        Shape::Cube => ([16, 16, 16], &[120, 180, 220, 260, 320, 400, 930]),
    };
    let mut rng = Rng(u64::from(table.birth ^ table.survive.rotate_left(7)));
    let examples: Vec<String> = densities
        .iter()
        .map(|&permille| {
            let input = soup(&mut rng, size, permille);
            let output = generation(shape, table, size, &input, true, 1);
            format!(
                "{{\"size\":[{},{},{}],\"input\":[{}],\"output\":[{}]}}",
                size[0],
                size[1],
                size[2],
                listed(&input),
                listed(&output)
            )
        })
        .collect();
    let got = ok(
        server,
        "grit_law_learn",
        &format!(
            "{{\"stencil\":\"{}\",\"reads\":[{{\"plane\":0,\"bit\":0}}],\"writes\":[{{\"plane\":1,\"bit\":0}}],\"border\":\"wrapped\",\"rule\":\"outer\",\"examples\":[{}],\"budget_ms\":{LONG}}}",
            shape.name(),
            examples.join(",")
        ),
    );
    assert_eq!(
        written(got.get("tables").unwrap()),
        table_of(shape, table),
        "{shape:?} {table:?}"
    );
    text_at(&got, "law")
}

fn learned(host: &Host, shape: Shape, table: Table) -> String {
    let mut server = Server::new(Some(host));
    learned_on(&mut server, shape, table)
}

fn table_of(shape: Shape, table: Table) -> String {
    let row = |bits: u32| {
        let shown: Vec<String> = (0..=shape.around())
            .map(|count| ((bits >> count) & 1).to_string())
            .collect();
        format!("[{}]", shown.join(","))
    };
    format!("[[{},{}]]", row(table.birth), row(table.survive))
}

fn widened(json: &Json) -> Json {
    let Json::List(items) = json else {
        return json.clone();
    };
    match items.as_slice() {
        [Json::Text(op), Json::List(inner), Json::Int(1)]
            if op == "and"
                && matches!(inner.first(), Some(Json::Text(head)) if head == "look" || head == "at") =>
        {
            Json::List(inner.clone())
        }
        [Json::Text(op), one, Json::Int(1)] if op == "xor" => Json::List(vec![
            Json::Text("xor".to_owned()),
            widened(one),
            Json::Int(ALL),
        ]),
        [Json::Text(op), Json::List(kept), other]
            if op == "or"
                && kept.len() == 3
                && kept.first() == Some(&Json::Text("and".to_owned()))
                && kept.get(2) == Some(&Json::Int(ALL - 1)) =>
        {
            widened(other)
        }
        _ => Json::List(items.iter().map(widened).collect()),
    }
}

fn sliced(text: &str) -> String {
    let json = grit::law::parse::read(text).unwrap();
    let Json::Map(fields) = json else {
        panic!("a law is an object");
    };
    let out: Vec<(String, Json)> = fields
        .into_iter()
        .map(|(key, one)| {
            if key == "step" {
                (key, widened(&one))
            } else {
                (key, one)
            }
        })
        .collect();
    written(&Json::Map(out))
}

fn iterating(text: &str, pair: [u32; 2]) -> Law {
    let json = grit::law::parse::read(text).unwrap();
    let Json::Map(mut fields) = json else {
        panic!("a law is an object");
    };
    fields.push((
        "alternate".to_owned(),
        Json::List(pair.iter().map(|one| Json::Int(i64::from(*one))).collect()),
    ));
    Law::from_json(&Json::Map(fields)).unwrap()
}

fn world<'host>(host: &'host Host, law: &Law, size: [u32; 3], sparse: bool) -> World<'host> {
    world_with(host, law, size, sparse, None)
}

fn world_with<'host>(
    host: &'host Host,
    law: &Law,
    size: [u32; 3],
    sparse: bool,
    slots: Option<u32>,
) -> World<'host> {
    World::create_law_stepped(
        host,
        law,
        &Made {
            rule: law.name(),
            params: &[],
            size,
            sparse,
            slots,
        },
        Stepping::Listed,
    )
    .unwrap()
}

fn first_difference(got: &[u32], want: &[u32], size: [u32; 3]) -> String {
    let count = cells(size);
    got.iter()
        .zip(want)
        .position(|(one, other)| one != other)
        .map_or_else(String::new, |at| {
            let [x, y, z] = point(size, at % count);
            format!(
                "first at plane {} [{x},{y},{z}]: sparse {:#x}, dense {:#x}",
                at / count,
                got[at],
                want[at]
            )
        })
}

type Cells = Vec<([u32; 3], u32)>;

#[derive(Clone)]
struct Case {
    label: String,
    law: Law,
    size: [u32; 3],
    start: Vec<(u32, Cells)>,
    writes: Vec<(u32, Cells)>,
    uniform: bool,
}

fn written_plane(law: &Law, frame: u32) -> u32 {
    law.current(frame).unwrap_or(0)
}

fn exact(host: &Host, case: &Case) -> Exact {
    let size = case.size;
    let mut sparse = world(host, &case.law, size, true);
    let mut dense = world(host, &case.law, size, false);
    let mut batch = world(host, &case.law, size, true);
    for one in [&mut sparse, &mut dense, &mut batch] {
        for (plane, cells) in &case.start {
            one.write_plane(*plane, cells).unwrap();
        }
    }
    let mut most_held = 0;
    let mut released = 0;
    let mut uniform_set = 0;
    let mut held_before = sparse.pool_use().unwrap().unwrap().held;
    for k in 1..=64_u32 {
        if let Some((_, cells)) = case.writes.iter().find(|(at, _)| *at == k - 1) {
            let plane = written_plane(&case.law, sparse.frame());
            for one in [&mut sparse, &mut dense] {
                one.write_plane(plane, cells).unwrap();
            }
        }
        sparse.step(1).unwrap();
        dense.step(1).unwrap();
        let got = sparse.cells().unwrap();
        let want = dense.cells().unwrap();
        assert!(
            got == want,
            "{}: k {k} differs: {}",
            case.label,
            first_difference(&got, &want, size)
        );
        let used = sparse.pool_use().unwrap().unwrap();
        assert!(
            !used.overflow && !used.lost,
            "{}: k {k} {used:?}",
            case.label
        );
        most_held = most_held.max(used.held);
        if sparse
            .uniforms()
            .unwrap()
            .iter()
            .any(|value| value.iter().any(|word| *word != 0))
        {
            uniform_set += 1;
        }
        released += held_before.saturating_sub(used.held);
        held_before = used.held;
    }
    let mut at = 0;
    for (frame, cells) in &case.writes {
        if *frame > at {
            batch.step(frame - at).unwrap();
            at = *frame;
        }
        let plane = written_plane(&case.law, batch.frame());
        batch.write_plane(plane, cells).unwrap();
    }
    batch.step(64 - at).unwrap();
    let got = batch.cells().unwrap();
    let want = dense.cells().unwrap();
    assert!(
        got == want,
        "{}: 64 frames in few calls differ: {}",
        case.label,
        first_difference(&got, &want, size)
    );
    Exact {
        most_held,
        released,
        uniform_set,
    }
}

struct Exact {
    most_held: u32,
    released: u32,
    uniform_set: u32,
}

fn word_of(rng: &mut Rng, bits: u32) -> u32 {
    if bits == 1 { 1 } else { rng.word() | 1 }
}

fn straddling(rng: &mut Rng, size: [u32; 3], shape: Shape, bits: u32) -> Vec<([u32; 3], u32)> {
    let (half, permille) = match shape {
        Shape::Square => (6_i64, 380),
        Shape::Cube => (5_i64, 230),
    };
    let depth: Vec<i64> = match shape {
        Shape::Square => vec![3, 8],
        Shape::Cube => (-half..half).collect(),
    };
    let mut out = Vec::new();
    for &z in &depth {
        for y in -half..half {
            for x in -half..half {
                if rng.below(1000) < permille {
                    out.push((wrapped(size, [x, y, z]), word_of(rng, bits)));
                }
            }
        }
    }
    out
}

fn glider(shape: Shape, at: [u32; 3], word: u32) -> Vec<([u32; 3], u32)> {
    let flat = [[1, 0], [2, 1], [0, 2], [1, 2], [2, 2]];
    let layers: &[u32] = match shape {
        Shape::Square => &[0],
        Shape::Cube => &[0, 1],
    };
    flat.iter()
        .flat_map(|[x, y]| {
            layers
                .iter()
                .map(move |z| ([at[0] + x, at[1] + y, at[2] + z], word))
        })
        .collect()
}

fn block(from: [u32; 3], to: [u32; 3], word: u32) -> Vec<([u32; 3], u32)> {
    let mut out = Vec::new();
    for z in from[2]..to[2] {
        for y in from[1]..to[1] {
            for x in from[0]..to[0] {
                out.push(([x, y, z], word));
            }
        }
    }
    out
}

fn scattered(rng: &mut Rng, size: [u32; 3], count: usize, bits: u32) -> Vec<([u32; 3], u32)> {
    (0..count)
        .map(|_| {
            let at = [0, 1, 2].map(|axis| u32::try_from(rng.below(u64::from(size[axis]))).unwrap());
            (at, word_of(rng, bits))
        })
        .collect()
}

fn counting_case(host: &Host, shape: Shape, table: Table, bits: u32, full: bool) -> Case {
    let text = learned(host, shape, table);
    let text = if bits == 32 { sliced(&text) } else { text };
    let law = iterating(&text, [0, 1]);
    let size = match (shape, full) {
        (Shape::Square, false) => [64, 64, 16],
        (Shape::Square, true) => [128, 128, 16],
        (Shape::Cube, false) => [64, 64, 64],
        (Shape::Cube, true) => [128, 128, 128],
    };
    let mut rng =
        Rng(u64::from(bits) * 1000 + u64::from(full) * 10 + u64::from(shape == Shape::Cube));
    let many = if bits == 1 { 1 } else { u32::MAX };
    let mut start = straddling(&mut rng, size, shape, bits);
    start.extend(glider(shape, [26, 25, 7], many));
    start.extend(glider(shape, [44, 9, 15], many));
    start.extend(scattered(&mut rng, size, 6, bits));
    if full {
        let (from, to) = match shape {
            Shape::Square => ([32, 32, 0], [96, 96, 16]),
            Shape::Cube => ([32, 32, 32], [96, 96, 96]),
        };
        start.extend(block(from, to, many));
    }
    let mut writes = Vec::new();
    for frame in [9_u32, 24, 41] {
        let mut cells = scattered(&mut rng, size, 4, bits);
        let corner = [
            u32::try_from(rng.below(48)).unwrap(),
            u32::try_from(rng.below(48)).unwrap(),
            u32::try_from(rng.below(u64::from(size[2] - 2))).unwrap(),
        ];
        cells.extend(glider(shape, corner, many));
        writes.push((frame, cells));
    }
    let name = match (shape, full) {
        (Shape::Square, false) => "Life",
        (Shape::Square, true) => "Life with S8",
        (Shape::Cube, false) => "Bays 5766",
        (Shape::Cube, true) => "Bays 5766 with S26",
    };
    Case {
        label: format!("{name} {} {bits} bit", shape.name()),
        law,
        size,
        start: vec![(0, start)],
        writes,
        uniform: full,
    }
}

fn blur_case() -> Case {
    let law = Law::compile(BLUR).unwrap();
    let size = [128, 128, 16];
    let mut rng = Rng(77);
    let mut low = block([16, 16, 0], [80, 80, 16], 7);
    low.extend(scattered(&mut rng, size, 40, 32));
    let mut high = block([64, 0, 0], [128, 96, 16], 0x0001_0000);
    high.extend(scattered(&mut rng, size, 40, 32));
    let start = vec![(0, low), (1, high)];
    let mut writes = Vec::new();
    for frame in [5_u32, 17, 30] {
        writes.push((frame, scattered(&mut rng, size, 12, 32)));
    }
    Case {
        label: "blur2 without alternate".to_owned(),
        law,
        size,
        start,
        writes,
        uniform: true,
    }
}

#[test]
fn h1_a_sparse_world_equals_the_dense_world_every_frame() {
    let Some(host) = device("sparse_halo h1") else {
        return;
    };
    let mut cases = Vec::new();
    for (shape, table, full) in [
        (Shape::Square, LIFE, false),
        (Shape::Square, LIFE_FULL, true),
        (Shape::Cube, BAYS, false),
        (Shape::Cube, BAYS_FULL, true),
    ] {
        for bits in [1, 32] {
            cases.push(counting_case(&host, shape, table, bits, full));
        }
    }
    cases.push(blur_case());
    for case in &cases {
        let got = exact(&host, case);
        println!(
            "h1 {}: 64 frames equal to dense, at most {} bricks held, {} released, {} frames with a uniform brick of nonzero words",
            case.label, got.most_held, got.released, got.uniform_set
        );
        assert!(got.released > 0, "{}: no brick returned", case.label);
        if case.uniform {
            assert!(
                got.uniform_set >= 8,
                "{}: uniform bricks of nonzero words beside live ones in {} frames",
                case.label,
                got.uniform_set
            );
        }
    }
}

type Live = std::collections::BTreeSet<[u32; 3]>;

fn stepped_set(shape: Shape, table: Table, size: [u32; 3], live: &Live) -> Live {
    let reach = shape.reach();
    let mut counts: std::collections::BTreeMap<[u32; 3], u32> = std::collections::BTreeMap::new();
    for cell in live {
        counts.entry(*cell).or_insert(0);
        for dz in -reach..=reach {
            for dy in -1_i64..=1 {
                for dx in -1_i64..=1 {
                    if dx == 0 && dy == 0 && dz == 0 {
                        continue;
                    }
                    let at = [
                        i64::from(cell[0]) + dx,
                        i64::from(cell[1]) + dy,
                        i64::from(cell[2]) + dz,
                    ];
                    *counts.entry(wrapped(size, at)).or_insert(0) += 1;
                }
            }
        }
    }
    counts
        .into_iter()
        .filter(|(cell, count)| {
            let row = if live.contains(cell) {
                table.survive
            } else {
                table.birth
            };
            (row >> count) & 1 == 1
        })
        .map(|(cell, _)| cell)
        .collect()
}

fn read_live(world: &mut World<'_>, size: [u32; 3]) -> Live {
    let plane = world.current().unwrap();
    let to = size.map(|side| side - 1);
    let got = world.scan(plane, [0, 0, 0], to, &[], u32::MAX).unwrap();
    got.cells
        .unwrap()
        .into_iter()
        .map(|(at, word)| {
            assert_eq!(word, 1);
            point(size, at as usize)
        })
        .collect()
}

fn bound(sides: [u32; 3], recent: &[Live], margin: i64) -> (u64, [[i64; 2]; 3]) {
    let mut span = [[i64::MAX, i64::MIN]; 3];
    for cell in recent.iter().flatten() {
        for axis in 0..3 {
            let at = i64::from(cell[axis]);
            span[axis][0] = span[axis][0].min(at - margin);
            span[axis][1] = span[axis][1].max(at + margin);
        }
    }
    let mut count = 1_u64;
    for axis in 0..3 {
        let side = i64::from(sides[axis]);
        let across = span[axis][1].div_euclid(side) - span[axis][0].div_euclid(side) + 1;
        count *= u64::try_from(across).unwrap();
    }
    (count, span)
}

fn bricks_of(sides: [u32; 3], live: &Live) -> std::collections::BTreeSet<[u32; 3]> {
    live.iter()
        .map(|cell| [0, 1, 2].map(|axis| cell[axis] / sides[axis]))
        .collect()
}

struct Flight {
    frames: u32,
    most_held: u32,
    most_bound: u64,
    released: u32,
    visited: usize,
    last_held: u32,
    moved: [i64; 3],
}

fn flown(
    host: &Host,
    shape: Shape,
    table: Table,
    size: [u32; 3],
    start: &[[u32; 3]],
    frames: u32,
) -> Flight {
    let law = iterating(&learned(host, shape, table), [0, 1]);
    let mut one = world(host, &law, size, true);
    let sides = one.pool().unwrap().sides();
    let cells: Vec<([u32; 3], u32)> = start.iter().map(|at| (*at, 1)).collect();
    one.write_plane(0, &cells).unwrap();
    let mut live: Live = start.iter().copied().collect();
    let mut recent = vec![live.clone()];
    let mut visited = bricks_of(sides, &live);
    let mut held_before = one.pool_use().unwrap().unwrap().held;
    let mut flight = Flight {
        frames,
        most_held: 0,
        most_bound: 0,
        released: 0,
        visited: 0,
        last_held: 0,
        moved: [0; 3],
    };
    for frame in 1..=frames {
        one.step(1).unwrap();
        live = stepped_set(shape, table, size, &live);
        let got = read_live(&mut one, size);
        assert!(
            got == live,
            "{shape:?} glider at frame {frame}: device {got:?}, host {live:?}"
        );
        recent.push(live.clone());
        if recent.len() > 5 {
            recent.remove(0);
        }
        visited.extend(bricks_of(sides, &live));
        let used = one.pool_use().unwrap().unwrap();
        assert!(!used.overflow && !used.lost, "frame {frame}: {used:?}");
        let (most, span) = bound(sides, &recent, 4);
        assert!(
            u64::from(used.held) <= most,
            "{shape:?} at frame {frame}: {} bricks held, the glider's span {span:?} touches at most {most} bricks of sides {sides:?}",
            used.held
        );
        flight.most_held = flight.most_held.max(used.held);
        flight.most_bound = flight.most_bound.max(most);
        flight.released += held_before.saturating_sub(used.held);
        held_before = used.held;
        flight.last_held = used.held;
    }
    assert_eq!(
        live.len(),
        start.len(),
        "{shape:?}: the glider kept its cells"
    );
    let low = |cells: &Live, axis: usize| {
        cells
            .iter()
            .map(|cell| i64::from(cell[axis]))
            .min()
            .unwrap()
    };
    let begun: Live = start.iter().copied().collect();
    flight.moved = [0, 1, 2].map(|axis| low(&live, axis) - low(&begun, axis));
    flight.visited = visited.len();
    assert!(
        flight.visited > usize::try_from(flight.most_held).unwrap(),
        "{shape:?}: the glider visited {} bricks and at most {} were held at once",
        flight.visited,
        flight.most_held
    );
    assert!(flight.released > 0, "{shape:?}: no brick returned");
    flight
}

#[test]
fn h2_a_flying_glider_keeps_a_bounded_pool() {
    let Some(host) = device("sparse_halo h2") else {
        return;
    };
    let life: Vec<[u32; 3]> = glider(Shape::Square, [500, 500, 7], 1)
        .into_iter()
        .map(|(at, _)| at)
        .collect();
    let flight = flown(&host, Shape::Square, LIFE, [1024, 1024, 16], &life, 300);
    println!(
        "h2 Life glider on a sparse 1024x1024x16 world: {} frames equal to the host, moved {:?}, at most {} bricks held (bound at most {}), {} at the end, {} released, {} bricks visited",
        flight.frames,
        flight.moved,
        flight.most_held,
        flight.most_bound,
        flight.last_held,
        flight.released,
        flight.visited
    );
    let bays: Vec<[u32; 3]> = glider(Shape::Cube, [128, 128, 128], 1)
        .into_iter()
        .map(|(at, _)| at)
        .collect();
    let flight = flown(&host, Shape::Cube, BAYS, [256, 256, 256], &bays, 280);
    println!(
        "h2 Bays 5766 glider on a sparse 256^3 world: {} frames equal to the host, moved {:?}, at most {} bricks held (bound at most {}), {} at the end, {} released, {} bricks visited",
        flight.frames,
        flight.moved,
        flight.most_held,
        flight.most_bound,
        flight.last_held,
        flight.released,
        flight.visited
    );
}

struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "grit-sparse-halo-{label}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn served_cells(size: [u32; 3], words: &[u32]) -> Vec<String> {
    (0..words.len())
        .filter(|at| words[*at] != 0)
        .map(|at| {
            let [x, y, z] = point(size, at);
            format!("[{x},{y},{z},{}]", words[at])
        })
        .collect()
}

fn served_write(server: &mut Server<'_>, world: i64, cells: &[String]) -> Json {
    let mut last = Json::Null;
    for piece in cells.chunks(4096) {
        last = ok(
            server,
            "grit_world_write",
            &format!(
                "{{\"world\":{world},\"current\":true,\"cells\":[{}],\"budget_ms\":{LONG}}}",
                piece.join(",")
            ),
        );
    }
    last
}

fn served_read(server: &mut Server<'_>, world: i64, size: [u32; 3]) -> Vec<u32> {
    let got = ok(
        server,
        "grit_world_read",
        &format!(
            "{{\"world\":{world},\"current\":true,\"form\":\"set\",\"from\":[0,0,0],\"to\":[{},{},{}]}}",
            size[0] - 1,
            size[1] - 1,
            size[2] - 1
        ),
    );
    assert_eq!(
        int_at(&got, "plane"),
        int_at(&got, "frame") & 1,
        "{}",
        written(&got)
    );
    let mut out = vec![0_u32; cells(size)];
    for cell in got.get("cells").and_then(Json::list).unwrap() {
        let parts: Vec<u32> = cell
            .list()
            .unwrap()
            .iter()
            .map(|one| u32::try_from(one.int().unwrap()).unwrap())
            .collect();
        out[index(size, [parts[0], parts[1], parts[2]])] = parts[3];
    }
    out
}

fn served_step(server: &mut Server<'_>, world: i64, frames: u32) -> Json {
    let got = ok(
        server,
        "grit_world_step",
        &format!("{{\"world\":{world},\"frames\":{frames},\"budget_ms\":{LONG}}}"),
    );
    assert_eq!(
        int_at(&got, "current"),
        int_at(&got, "frame") & 1,
        "{}",
        written(&got)
    );
    let used = got
        .get("pool_use")
        .unwrap_or_else(|| panic!("{}", written(&got)));
    assert!(
        used.get("held").and_then(Json::int).is_some(),
        "{}",
        written(&got)
    );
    got
}

fn generations(size: [u32; 3], from: &[u32], count: u32) -> Vec<u32> {
    let mut held = from.to_vec();
    for _ in 0..count {
        held = generation(Shape::Square, LIFE, size, &held, true, 1);
    }
    held
}

fn sparse_create(law: &str, size: [u32; 3], more: &str) -> String {
    format!(
        "{{\"wide\":{},\"deep\":{},\"tall\":{},\"law\":{}{more},\"sparse\":true,\"budget_ms\":{LONG}}}",
        size[0],
        size[1],
        size[2],
        quoted(law)
    )
}

#[test]
fn h3_parity_survives_journal_fork_restore_save_open_params_and_restart_on_a_sparse_world() {
    let Some(host) = device("sparse_halo h3") else {
        return;
    };
    let law = learned(&host, Shape::Square, LIFE);
    let size = [64, 64, 16];
    let mut rng = Rng(303);
    let mut start = vec![0_u32; cells(size)];
    for z in [2_u32, 9] {
        for y in 0..48 {
            for x in 0..48 {
                if rng.below(1000) < 300 {
                    start[index(size, [x, y, z])] = 1;
                }
            }
        }
    }
    for (at, _) in glider(Shape::Square, [50, 50, 14], 1) {
        start[index(size, at)] = 1;
    }
    let scratch = Scratch::new("h3");
    let options = grit::serve3::Options {
        dir: Some(scratch.0.join("worlds")),
        memory: None,
    };
    let (number, kept) = {
        let mut server = Server::with(Some(&host), &options).unwrap();
        let made = ok(
            &mut server,
            "grit_world_create",
            &sparse_create(&law, size, ",\"alternate\":[0,1]"),
        );
        assert_eq!(written(made.get("alternate").unwrap()), "[0,1]");
        assert_eq!(text_at(&made, "quiet"), "proved");
        let number = int_at(&made, "world");
        served_write(&mut server, number, &served_cells(size, &start));
        served_step(&mut server, number, 3);
        let mut host_cells = generations(size, &start, 3);
        assert_eq!(served_read(&mut server, number, size), host_cells);
        let fork = ok(
            &mut server,
            "grit_world_fork",
            &format!("{{\"world\":{number}}}"),
        );
        let forked = int_at(&fork, "world");
        assert_eq!(int_at(&fork, "frame"), 3);
        assert_eq!(
            served_read(&mut server, forked, size),
            host_cells,
            "the fork reads the current plane of its source"
        );
        served_step(&mut server, forked, 5);
        let fork_host = generations(size, &host_cells, 5);
        assert_eq!(
            served_read(&mut server, forked, size),
            fork_host,
            "the fork continues from an odd frame"
        );
        let mut extra = vec![0_u32; cells(size)];
        for at in [
            [63, 30, 5],
            [0, 30, 5],
            [1, 30, 5],
            [40, 60, 12],
            [41, 60, 12],
            [42, 60, 12],
            [31, 15, 7],
            [32, 15, 7],
            [33, 15, 7],
        ] {
            extra[index(size, at)] = 1;
        }
        let wrote = served_write(&mut server, number, &served_cells(size, &extra));
        assert_eq!(int_at(&wrote, "plane"), 1);
        for (cell, word) in host_cells.iter_mut().zip(&extra) {
            *cell |= word;
        }
        served_step(&mut server, number, 2);
        host_cells = generations(size, &host_cells, 2);
        ok(
            &mut server,
            "grit_world_params",
            &format!("{{\"world\":{number},\"params\":[]}}"),
        );
        served_step(&mut server, number, 1);
        host_cells = generations(size, &host_cells, 1);
        assert_eq!(
            served_read(&mut server, number, size),
            host_cells,
            "after host writes and params, frame 6"
        );
        ok(
            &mut server,
            "grit_world_restore",
            &format!("{{\"world\":{number},\"from\":{forked}}}"),
        );
        let restored = served_step(&mut server, number, 1);
        assert_eq!(int_at(&restored, "frame"), 9);
        host_cells = generations(size, &fork_host, 1);
        assert_eq!(
            served_read(&mut server, number, size),
            host_cells,
            "restored from the fork at frame 8, then frame 9"
        );
        ok(
            &mut server,
            "grit_world_save",
            &format!("{{\"world\":{number},\"name\":\"odd\"}}"),
        );
        let opened = ok(&mut server, "grit_world_open", "{\"name\":\"odd\"}");
        let reopened = int_at(&opened, "world");
        assert_eq!(int_at(&opened, "frame"), 9);
        assert_eq!(written(opened.get("alternate").unwrap()), "[0,1]");
        served_step(&mut server, reopened, 7);
        let open_host = generations(size, &host_cells, 7);
        assert_eq!(
            served_read(&mut server, reopened, size),
            open_host,
            "opened at odd frame 9, then 7 frames"
        );
        let back = no(
            &mut server,
            "grit_world_back",
            &format!("{{\"world\":{number},\"frames\":1}}"),
        );
        assert_eq!(
            text_at(&back, "code"),
            "not_reversible",
            "{}",
            written(&back)
        );
        served_step(&mut server, number, 4);
        host_cells = generations(size, &host_cells, 4);
        assert_eq!(served_read(&mut server, number, size), host_cells);
        (number, host_cells)
    };
    let mut again = Server::with(Some(&host), &options).unwrap();
    let opened = ok(
        &mut again,
        "grit_world_open",
        &format!("{{\"name\":\"world-{number}\",\"budget_ms\":{LONG}}}"),
    );
    let replayed = int_at(&opened, "world");
    assert_eq!(int_at(&opened, "frame"), 13);
    assert_eq!(
        served_read(&mut again, replayed, size),
        kept,
        "the journal replays to odd frame 13"
    );
    served_step(&mut again, replayed, 5);
    assert_eq!(
        served_read(&mut again, replayed, size),
        generations(size, &kept, 5),
        "after a restart at odd frame 13, 5 more frames"
    );
    println!(
        "h3 one sparse Life world: frames 3 fork 5, writes at 3 across the seam and into empty bricks, params at 5, restore at 8, save and open at 9, back refused, restart at 13 and 5 more, every read equal to the host, pool use in every step answer"
    );
}

fn looking_law(name: &str, look: &str, extra: &str) -> String {
    format!(
        "{{\"law\":1,\"name\":\"{name}\",\"params\":0,\"channels\":2,\"halo\":1{extra},\"tally\":{{\"counts\":1,\"weighted\":false}},\"consts\":{{}},\"defs\":{{}},\"step\":[\"vec\",[\"map\",\"c\",[\"range\",8],[\"vec\",[\"at\",[\"at\",\"cells\",\"c\"],0],[\"add\",[\"at\",[\"at\",\"cells\",\"c\"],1],{look}]]],[\"vec\",0]]}}"
    )
}

fn refused_law(server: &mut Server<'_>, law: &str, size: [u32; 3]) -> Json {
    no(server, "grit_world_create", &sparse_create(law, size, ""))
}

#[test]
fn h5_what_a_sparse_world_still_refuses_is_refused_by_name() {
    let Some(host) = device("sparse_halo h5") else {
        return;
    };
    let size = [32, 32, 16];
    let mut server = Server::new(Some(&host));

    let far = looking_law("far", "[\"look\",0,2,0,0,\"c\"]", "");
    let told = refused_law(&mut server, &far, size);
    assert_eq!(text_at(&told, "code"), "law_refused", "{}", written(&told));
    assert_eq!(text_at(&told, "field"), "$.law");
    assert_eq!(
        text_at(told.get("law").unwrap(), "code"),
        "halo_radius",
        "{}",
        written(&told)
    );

    let past = looking_law("past", "[\"look\",2,1,0,0,\"c\"]", "");
    let told = refused_law(&mut server, &past, size);
    assert_eq!(text_at(&told, "code"), "law_refused", "{}", written(&told));
    assert_eq!(
        text_at(told.get("law").unwrap(), "code"),
        "halo_plane",
        "{}",
        written(&told)
    );

    let written_look = looking_law("own", "[\"look\",1,1,0,0,\"c\"]", "");
    let told = refused_law(&mut server, &written_look, size);
    assert_eq!(text_at(&told, "code"), "law_refused", "{}", written(&told));
    assert_eq!(
        text_at(told.get("law").unwrap(), "code"),
        "halo_written",
        "{}",
        written(&told)
    );

    let timed = looking_law(
        "timed",
        "[\"add\",[\"look\",0,1,0,0,\"c\"],[\"and\",\"frame\",1]]",
        "",
    );
    let told = refused_law(&mut server, &timed, size);
    assert_eq!(
        text_at(&told, "code"),
        "quiet_refused",
        "{}",
        written(&told)
    );
    assert_eq!(text_at(&told, "field"), "$.sparse");

    let fine = looking_law("fine", "[\"look\",0,1,0,0,\"c\"]", "");
    let made = ok(
        &mut server,
        "grit_world_create",
        &sparse_create(&fine, size, ""),
    );
    assert_eq!(text_at(&made, "quiet"), "proved");
    assert!(int_at(&made, "world") >= 0, "{}", written(&made));

    let described = ok(&mut server, "grit_describe", "{}");
    let codes: Vec<String> = described
        .get("codes")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .map(|one| text_at(one, "code"))
        .collect();
    assert!(!codes.iter().any(|one| one == "halo_sparse"));
    for code in ["law_refused", "quiet_refused"] {
        assert!(codes.iter().any(|one| one == code), "{code}");
    }
    let look = text_at(described.get("limits").unwrap(), "law_look");
    assert!(look.contains("dense and sparse"), "{look}");
    println!(
        "h5 a sparse world refuses halo_radius, halo_plane and halo_written as law_refused, a looking law that reads frame as quiet_refused, and makes a proved looking law; halo_sparse is gone"
    );
}

#[test]
fn h5_a_reversible_looking_law_walks_back_on_a_sparse_world() {
    let Some(host) = device("sparse_halo h5 back") else {
        return;
    };
    let text = looking_law(
        "lift",
        "[\"add\",[\"look\",0,1,0,0,\"c\"],[\"look\",0,0,-1,1,\"c\"]]",
        ",\"reversible\":true",
    );
    let law = Law::compile(&text).unwrap();
    let size = [64, 64, 16];
    let mut rng = Rng(5150);
    let low = scattered(&mut rng, size, 60, 32);
    let high = scattered(&mut rng, size, 60, 32);
    let mut sparse = world(&host, &law, size, true);
    let mut dense = world(&host, &law, size, false);
    for one in [&mut sparse, &mut dense] {
        one.write_plane(0, &low).unwrap();
        one.write_plane(1, &high).unwrap();
    }
    let start = sparse.cells().unwrap();
    for k in 1..=20 {
        sparse.step(1).unwrap();
        dense.step(1).unwrap();
        let got = sparse.cells().unwrap();
        let want = dense.cells().unwrap();
        assert!(
            got == want,
            "forward k {k}: {}",
            first_difference(&got, &want, size)
        );
    }
    sparse.back(20).unwrap();
    dense.back(20).unwrap();
    assert_eq!(sparse.frame(), 0);
    let got = sparse.cells().unwrap();
    assert!(
        got == dense.cells().unwrap(),
        "walked back: {}",
        first_difference(&got, &dense.cells().unwrap(), size)
    );
    assert!(got == start, "{}", first_difference(&got, &start, size));
    println!(
        "h5 a reversible law that looks steps 20 frames on a sparse world equal to dense, and walks back to its start"
    );
}

#[cfg(feature = "faults")]
mod pinned {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use grit::Host;
    use grit::clock::{Fake, Time};
    use grit::serve3::{Options, Server};
    use grit::wire::{Json, written};

    use super::{
        BAYS, BLUR, LIFE, LONG, Rng, Scratch, Shape, device, int_at, learned_on, ok, quoted, sliced,
    };

    fn hex(sum: &[u8; 32]) -> String {
        sum.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    const STEADY: [&str; 7] = [
        "world", "from", "name", "written", "frame", "frames", "tally",
    ];

    #[derive(Clone, Copy)]
    enum Made {
        Learned(Shape, bool),
        Blur,
        Turn,
        Rule(&'static str),
    }

    struct Case {
        name: &'static str,
        made: Made,
        size: [u32; 3],
        sparse: bool,
        planes: u32,
        mask: u32,
        seed: u64,
    }

    const CASES: [Case; 5] = [
        Case {
            name: "dense-life-iterating",
            made: Made::Learned(Shape::Square, false),
            size: [64, 64, 16],
            sparse: false,
            planes: 2,
            mask: 1,
            seed: 0x51F1,
        },
        Case {
            name: "dense-bays-32-iterating",
            made: Made::Learned(Shape::Cube, true),
            size: [32, 32, 32],
            sparse: false,
            planes: 2,
            mask: u32::MAX,
            seed: 0x51F2,
        },
        Case {
            name: "dense-blur",
            made: Made::Blur,
            size: [64, 64, 16],
            sparse: false,
            planes: 3,
            mask: 0xFFFF,
            seed: 0x51F3,
        },
        Case {
            name: "sparse-turn-iterating",
            made: Made::Turn,
            size: [64, 64, 32],
            sparse: true,
            planes: 2,
            mask: u32::MAX,
            seed: 0x51F4,
        },
        Case {
            name: "sparse-sand",
            made: Made::Rule("sand"),
            size: [64, 64, 32],
            sparse: true,
            planes: 1,
            mask: 3,
            seed: 0x51F5,
        },
    ];

    fn turn_law() -> String {
        let corner = |k: u32| {
            format!(
                "[\"vec\",[\"at\",[\"at\",\"cells\",{k}],0],[\"at\",[\"at\",\"cells\",{}],0]]",
                k ^ 7
            )
        };
        let out: Vec<String> = (0..8).map(corner).collect();
        format!(
            "{{\"law\":1,\"name\":\"turn\",\"params\":0,\"channels\":2,\"halo\":0,\"tally\":{{\"counts\":1,\"weighted\":false}},\"consts\":{{}},\"defs\":{{}},\"step\":[\"vec\",[\"vec\",{}],[\"vec\",0]]}}",
            out.join(",")
        )
    }

    fn timed<'host>(host: &'host Host, dir: &Path, fake: &Arc<Fake>) -> Server<'host> {
        let time: Arc<dyn Time> = Arc::clone(fake) as Arc<dyn Time>;
        let options = Options {
            dir: Some(dir.to_path_buf()),
            memory: None,
        };
        Server::timed(Some(host), &options, time).unwrap()
    }

    fn steady(answer: &Json, sparse: bool) -> String {
        match answer {
            Json::Map(pairs) if sparse => written(&Json::Map(
                pairs
                    .iter()
                    .filter(|(key, _)| STEADY.contains(&key.as_str()))
                    .cloned()
                    .collect(),
            )),
            other => written(other),
        }
    }

    fn scattered(seed: u64, size: [u32; 3], count: usize, mask: u32) -> Vec<String> {
        let mut rng = Rng(seed);
        (0..count)
            .map(|_| {
                let x = u32::try_from(rng.below(u64::from(size[0]))).unwrap();
                let y = u32::try_from(rng.below(u64::from(size[1]))).unwrap();
                let z = u32::try_from(rng.below(u64::from(size[2]))).unwrap();
                format!("[{x},{y},{z},{}]", rng.word() & mask)
            })
            .collect()
    }

    fn write(server: &mut Server<'_>, world: i64, plane: &str, cells: &[String]) -> Json {
        ok(
            server,
            "grit_world_write",
            &format!(
                "{{\"world\":{world},{plane},\"cells\":[{}],\"budget_ms\":{LONG}}}",
                cells.join(",")
            ),
        )
    }

    fn step(server: &mut Server<'_>, world: i64, frames: u32) -> Json {
        ok(
            server,
            "grit_world_step",
            &format!("{{\"world\":{world},\"frames\":{frames},\"budget_ms\":{LONG}}}"),
        )
    }

    fn read_plane(server: &mut Server<'_>, world: i64, size: [u32; 3], plane: u32) -> Vec<u32> {
        let per = (4096 / (size[0] * size[1])).max(1);
        let mut words = Vec::new();
        let mut z = 0;
        while z < size[2] {
            let top = (z + per).min(size[2]) - 1;
            let got = ok(
                server,
                "grit_world_read",
                &format!(
                    "{{\"world\":{world},\"plane\":{plane},\"from\":[0,0,{z}],\"to\":[{},{},{top}]}}",
                    size[0] - 1,
                    size[1] - 1
                ),
            );
            for cell in got.get("cells").and_then(Json::list).unwrap() {
                words.push(u32::try_from(cell.int().unwrap()).unwrap());
            }
            z = top + 1;
        }
        words
    }

    fn read_all(server: &mut Server<'_>, world: i64, case: &Case) -> String {
        let mut words = Vec::new();
        for plane in 0..case.planes {
            words.extend(read_plane(server, world, case.size, plane));
        }
        let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
        hex(&grit::branch::sha256(&bytes))
    }

    fn played(server: &mut Server<'_>, case: &Case) -> (i64, Vec<String>) {
        let mut answers = Vec::new();
        let (made, iterating) = match case.made {
            Made::Learned(shape, wide) => {
                let table = if shape == Shape::Square { LIFE } else { BAYS };
                let text = learned_on(server, shape, table);
                let text = if wide { sliced(&text) } else { text };
                answers.push(text.clone());
                (
                    format!("\"law\":{},\"alternate\":[0,1]", quoted(&text)),
                    true,
                )
            }
            Made::Blur => (format!("\"law\":{}", quoted(BLUR)), false),
            Made::Turn => (
                format!("\"law\":{},\"alternate\":[0,1]", quoted(&turn_law())),
                true,
            ),
            Made::Rule(name) => (format!("\"rule\":\"{name}\""), false),
        };
        let plane = if iterating {
            "\"current\":true".to_owned()
        } else {
            "\"plane\":0".to_owned()
        };
        let created = ok(
            server,
            "grit_world_create",
            &format!(
                "{{\"wide\":{},\"deep\":{},\"tall\":{},{made},\"sparse\":{},\"budget_ms\":{LONG}}}",
                case.size[0], case.size[1], case.size[2], case.sparse
            ),
        );
        let world = int_at(&created, "world");
        answers.push(steady(&created, case.sparse));
        let size = case.size;
        let first = scattered(case.seed, size, 900, case.mask);
        answers.push(steady(&write(server, world, &plane, &first), case.sparse));
        answers.push(steady(&step(server, world, 3), case.sparse));
        let fork = ok(server, "grit_world_fork", &format!("{{\"world\":{world}}}"));
        let forked = int_at(&fork, "world");
        answers.push(steady(&fork, case.sparse));
        answers.push(steady(&step(server, world, 2), case.sparse));
        let more = scattered(case.seed ^ 1, size, 300, case.mask);
        answers.push(steady(&write(server, world, &plane, &more), case.sparse));
        if case.planes > 2 {
            let upper = scattered(case.seed ^ 2, size, 200, case.mask);
            answers.push(steady(
                &write(server, world, "\"plane\":1", &upper),
                case.sparse,
            ));
        }
        answers.push(steady(&step(server, world, 5), case.sparse));
        answers.push(read_all(server, world, case));
        let restored = ok(
            server,
            "grit_world_restore",
            &format!("{{\"world\":{forked},\"from\":{world}}}"),
        );
        answers.push(steady(&restored, case.sparse));
        answers.push(steady(&step(server, forked, 3), case.sparse));
        answers.push(read_all(server, forked, case));
        let saved = ok(
            server,
            "grit_world_save",
            &format!("{{\"world\":{forked},\"name\":\"kept\"}}"),
        );
        answers.push(steady(&saved, case.sparse));
        let opened = ok(server, "grit_world_open", "{\"name\":\"kept\"}");
        let reopened = int_at(&opened, "world");
        answers.push(steady(&opened, case.sparse));
        answers.push(steady(&step(server, reopened, 1), case.sparse));
        answers.push(read_all(server, reopened, case));
        answers.push(steady(&step(server, world, 7), case.sparse));
        answers.push(read_all(server, world, case));
        (world, answers)
    }

    fn files(dir: &Path, into: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                files(&path, into);
            } else if path.file_name().and_then(|name| name.to_str()) != Some("lock") {
                into.push(path);
            }
        }
    }

    fn journal_hash(dir: &Path) -> String {
        let mut names = Vec::new();
        files(dir, &mut names);
        names.sort();
        let mut joined = Vec::new();
        for path in names {
            let name = path
                .strip_prefix(dir)
                .unwrap()
                .to_str()
                .unwrap()
                .replace('\\', "/");
            joined.extend_from_slice(name.as_bytes());
            joined.push(0);
            joined.extend_from_slice(&std::fs::read(&path).unwrap());
            joined.push(0);
        }
        hex(&grit::branch::sha256(&joined))
    }

    fn trace(host: &Host, case: &Case) -> (String, String) {
        let scratch = Scratch::new(case.name);
        let dir = scratch.0.join("worlds");
        let fake = Arc::new(Fake::new());
        let (world, answers) = {
            let mut server = timed(host, &dir, &fake);
            played(&mut server, case)
        };
        let journal = journal_hash(&dir);
        let fake = Arc::new(Fake::new());
        let mut again = timed(host, &dir, &fake);
        let opened = ok(
            &mut again,
            "grit_world_open",
            &format!("{{\"name\":\"world-{world}\",\"budget_ms\":{LONG}}}"),
        );
        let reopened = int_at(&opened, "world");
        let replayed = read_all(&mut again, reopened, case);
        assert_eq!(Some(&replayed), answers.last(), "{}: replayed", case.name);
        let mut both = answers;
        both.push(steady(&opened, case.sparse));
        both.push(replayed);
        (
            hex(&grit::branch::sha256(both.join("\n").as_bytes())),
            journal,
        )
    }

    const RECORDED: [(&str, &str, &str); 5] = [
        (
            "dense-life-iterating",
            "6b06e5c2ccfde5802f311a54b15a349c5efcf827d960280355142519bdee815c",
            "455a4f69b890a0f08e72000d6513c6c3a4f867aeca7a406ab930847c37eaa8ac",
        ),
        (
            "dense-bays-32-iterating",
            "6a5740bfe4426cc38f943754f138e95f976e34c8a59bf95629486a205e76315a",
            "bdadf9becde74cfae72f9e0299267e609ffef34a50b168b10b3eec798ea08231",
        ),
        (
            "dense-blur",
            "1450ecf3c2b18ea7d412b528b0c1353a665b2d3fae516f48844ea53076bd2e27",
            "3bcef2009ce78ad52e8019549179b3a1a15aeaa37945319e6719bb2ac93d449a",
        ),
        (
            "sparse-turn-iterating",
            "014d43b7ab2607a0cc4e47c3ae2a8fb1ea2cd5d7576df5010144165d83361b02",
            "",
        ),
        (
            "sparse-sand",
            "2bbd20c2b7a47aed38c304ef6a7f1f94f97a0b9a90941b97444220f18cde12dc",
            "",
        ),
    ];

    #[test]
    fn h4_dense_and_sparse_non_looking_worlds_answer_read_and_journal_as_on_base() {
        let Some(host) = device("sparse_halo h4") else {
            return;
        };
        let got: Vec<(String, String)> = CASES.iter().map(|case| trace(&host, case)).collect();
        for (case, (answers, journal)) in CASES.iter().zip(&got) {
            println!("RECORD {} {answers} {journal}", case.name);
        }
        for ((case, (answers, journal)), (name, want_answers, want_journal)) in
            CASES.iter().zip(&got).zip(RECORDED)
        {
            assert_eq!(case.name, name);
            assert_eq!(answers, want_answers, "{name}: answers and reads");
            if !case.sparse {
                assert_eq!(journal, want_journal, "{name}: journal");
            }
        }
    }
}
