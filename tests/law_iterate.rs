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

const LIFE: (u32, u32) = (1 << 3, (1 << 2) | (1 << 3));

const BAYS: (u32, u32) = (1 << 6, (1 << 5) | (1 << 6) | (1 << 7));

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

    const fn rule(self) -> (u32, u32) {
        match self {
            Self::Square => LIFE,
            Self::Cube => BAYS,
        }
    }

    const fn around(self) -> u32 {
        match self {
            Self::Square => 8,
            Self::Cube => 26,
        }
    }
}

fn index(size: [u32; 3], at: [u32; 3]) -> usize {
    (at[2] as usize * size[1] as usize + at[1] as usize) * size[0] as usize + at[0] as usize
}

fn cells(size: [u32; 3]) -> usize {
    size.iter().map(|side| *side as usize).product()
}

fn generation(shape: Shape, size: [u32; 3], words: &[u32], wrapped: bool, used: u32) -> Vec<u32> {
    let (birth, survive) = shape.rule();
    let reach: i64 = match shape {
        Shape::Square => 0,
        Shape::Cube => 1,
    };
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
                            if !inside && !wrapped {
                                continue;
                            }
                            let spot = [0, 1, 2].map(|axis| {
                                u32::try_from(at[axis].rem_euclid(side[axis])).unwrap()
                            });
                            let word = words[index(size, spot)] & used;
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
                    let table = if (centre >> bit) & 1 == 1 {
                        survive
                    } else {
                        birth
                    };
                    word |= ((table >> count) & 1) << bit;
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

fn soup(rng: &mut Rng, size: [u32; 3], permille: u64, used: u32) -> Vec<u32> {
    (0..cells(size))
        .map(|_| {
            if used == 1 {
                u32::from(rng.below(1000) < permille)
            } else {
                (0..32).fold(0_u32, |word, bit| {
                    word | (u32::from(rng.below(1000) < permille) << bit)
                })
            }
        })
        .collect()
}

fn learned_answer(host: &Host, shape: Shape, border: &str) -> Json {
    let mut server = Server::new(Some(host));
    learned_answer_on(&mut server, shape, border)
}

fn learned_answer_on(server: &mut Server<'_>, shape: Shape, border: &str) -> Json {
    let wrapped = border == "wrapped";
    let (size, densities): ([u32; 3], &[u64]) = match shape {
        Shape::Square => ([32, 32, 1], &[250, 300, 350, 400, 450, 500, 600, 700]),
        Shape::Cube => ([16, 16, 16], &[120, 180, 220, 260, 320, 400]),
    };
    let mut rng = Rng(match (shape, wrapped) {
        (Shape::Square, true) => 11,
        (Shape::Square, false) => 12,
        (Shape::Cube, true) => 13,
        (Shape::Cube, false) => 14,
    });
    let examples: Vec<String> = densities
        .iter()
        .map(|&permille| {
            let input = soup(&mut rng, size, permille, 1);
            let output = generation(shape, size, &input, wrapped, 1);
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
    ok(
        server,
        "grit_law_learn",
        &format!(
            "{{\"stencil\":\"{}\",\"reads\":[{{\"plane\":0,\"bit\":0}}],\"writes\":[{{\"plane\":1,\"bit\":0}}],\"border\":\"{border}\",\"rule\":\"outer\",\"examples\":[{}],\"budget_ms\":{LONG}}}",
            shape.name(),
            examples.join(",")
        ),
    )
}

fn table_of(shape: Shape) -> String {
    let (birth, survive) = shape.rule();
    let row = |table: u32| {
        let shown: Vec<String> = (0..=shape.around())
            .map(|count| ((table >> count) & 1).to_string())
            .collect();
        format!("[{}]", shown.join(","))
    };
    format!("[[{},{}]]", row(birth), row(survive))
}

fn learned(host: &Host, shape: Shape, border: &str) -> String {
    let got = learned_answer(host, shape, border);
    assert_eq!(
        written(got.get("tables").unwrap()),
        table_of(shape),
        "{shape:?} {border}"
    );
    text_at(&got, "law")
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

fn world<'host>(
    host: &'host Host,
    law: &Law,
    size: [u32; 3],
    sparse: bool,
    stepping: Stepping,
) -> World<'host> {
    World::create_law_stepped(
        host,
        law,
        &Made {
            rule: law.name(),
            params: &[],
            size,
            sparse,
            slots: None,
        },
        stepping,
    )
    .unwrap()
}

fn seeded(world: &mut World<'_>, plane: u32, size: [u32; 3], words: &[u32]) {
    let mut put = Vec::new();
    for z in 0..size[2] {
        for y in 0..size[1] {
            for x in 0..size[0] {
                let word = words[index(size, [x, y, z])];
                if word != 0 {
                    put.push(([x, y, z], word));
                }
            }
        }
    }
    world.write_plane(plane, &put).unwrap();
}

fn plane_of(world: &mut World<'_>, plane: u32, size: [u32; 3]) -> Vec<u32> {
    let all = world.cells().unwrap();
    let count = cells(size);
    all[plane as usize * count..(plane as usize + 1) * count].to_vec()
}

fn current_of(world: &mut World<'_>, size: [u32; 3]) -> Vec<u32> {
    let plane = world.current().unwrap();
    assert_eq!(plane, world.frame() & 1, "frame {}", world.frame());
    plane_of(world, plane, size)
}

fn first_difference(got: &[u32], want: &[u32], size: [u32; 3]) -> String {
    got.iter()
        .zip(want)
        .position(|(one, other)| one != other)
        .map_or_else(String::new, |at| {
            let x = at % size[0] as usize;
            let y = at / size[0] as usize % size[1] as usize;
            let z = at / (size[0] as usize * size[1] as usize);
            format!(
                "first at [{x},{y},{z}]: device {:#x}, host {:#x}",
                got[at], want[at]
            )
        })
}

fn margolus(size: [u32; 3], frame: u32, from: &[u32], into: &[u32], fly: bool) -> Vec<u32> {
    let mut out = into.to_vec();
    let phase = frame & 1;
    for l in 0..size[2] / 2 {
        for j in 0..size[1] / 2 {
            for i in 0..size[0] / 2 {
                let corner = |k: u32| {
                    let at = [
                        (2 * i + phase + (k & 1)) % size[0],
                        (2 * j + phase + (k >> 2)) % size[1],
                        (2 * l + phase + ((k >> 1) & 1)) % size[2],
                    ];
                    index(size, at)
                };
                for k in 0..8 {
                    out[corner(k)] = if fly {
                        from[corner(k ^ 7)]
                    } else {
                        from[corner((k + 1) % 8)] ^ (from[corner(k)] << 1)
                    };
                }
            }
        }
    }
    out
}

fn turn_law(fly: bool) -> String {
    let corner = |k: u32| {
        let next = (k + 1) % 8;
        let made = if fly {
            format!("[\"at\",[\"at\",\"cells\",{}],0]", k ^ 7)
        } else {
            format!(
                "[\"xor\",[\"at\",[\"at\",\"cells\",{next}],0],[\"shl\",[\"at\",[\"at\",\"cells\",{k}],0],1]]"
            )
        };
        format!("[\"vec\",[\"at\",[\"at\",\"cells\",{k}],0],{made}]")
    };
    let out: Vec<String> = (0..8).map(corner).collect();
    format!(
        "{{\"law\":1,\"name\":\"turn\",\"params\":0,\"channels\":2,\"halo\":0,\"tally\":{{\"counts\":1,\"weighted\":false}},\"consts\":{{}},\"defs\":{{}},\"step\":[\"vec\",[\"vec\",{}],[\"vec\",0]]}}",
        out.join(",")
    )
}

type Next = dyn Fn(u32, &[u32], &[u32]) -> Vec<u32>;

struct Run {
    label: String,
    frames: u32,
}

fn counted_exact(
    host: &Host,
    law: &Law,
    size: [u32; 3],
    (sparse, stepping): (bool, Stepping),
    start: &[u32],
    next: &Next,
    label: &str,
) -> Run {
    let mut one = world(host, law, size, sparse, stepping);
    seeded(&mut one, 0, size, start);
    let mut planes = [start.to_vec(), vec![0_u32; start.len()]];
    let mut whole = world(host, law, size, sparse, stepping);
    seeded(&mut whole, 0, size, start);
    for frame in 0..64_u32 {
        let from = (frame & 1) as usize;
        let into = 1 - from;
        planes[into] = next(frame, &planes[from], &planes[into]);
        one.step(1).unwrap();
        assert_eq!(one.frame(), frame + 1);
        let got = current_of(&mut one, size);
        assert!(
            got == planes[into],
            "{label}: k {} differs: {}",
            frame + 1,
            first_difference(&got, &planes[into], size)
        );
        let other = plane_of(&mut one, from as u32, size);
        assert!(
            other == planes[from],
            "{label}: k {}, the looked-at plane changed: {}",
            frame + 1,
            first_difference(&other, &planes[from], size)
        );
    }
    whole.step(64).unwrap();
    let got = current_of(&mut whole, size);
    assert!(
        got == planes[0],
        "{label}: 64 frames in one call differ: {}",
        first_difference(&got, &planes[0], size)
    );
    Run {
        label: label.to_owned(),
        frames: 64,
    }
}

#[test]
fn i1_k_frames_equal_k_host_iterations_bit_for_bit() {
    let Some(host) = device("law_iterate i1") else {
        return;
    };
    let mut runs = Vec::new();
    for shape in [Shape::Square, Shape::Cube] {
        let size = match shape {
            Shape::Square => [32, 32, 2],
            Shape::Cube => [16, 16, 16],
        };
        for border in ["wrapped", "zero"] {
            let text = learned(&host, shape, border);
            for used in [1_u32, u32::MAX] {
                let law_text = if used == 1 {
                    text.clone()
                } else {
                    sliced(&text)
                };
                let law = iterating(&law_text, [0, 1]);
                for stepping in [Stepping::Listed, Stepping::Whole] {
                    let mut rng =
                        Rng(u64::from(used) ^ ((shape as u64) << 8) ^ (border.len() as u64));
                    let permille = match shape {
                        Shape::Square => 300,
                        Shape::Cube => 180,
                    };
                    let start = soup(&mut rng, size, permille, used);
                    let label = format!(
                        "{} {border} {} {stepping:?} dense",
                        shape.name(),
                        if used == 1 { "1 bit" } else { "32 bits" }
                    );
                    let next = move |_: u32, from: &[u32], _: &[u32]| {
                        generation(shape, size, from, true, used)
                    };
                    runs.push(counted_exact(
                        &host,
                        &law,
                        size,
                        (false, stepping),
                        &start,
                        &next,
                        &label,
                    ));
                }
            }
        }
    }
    let turn = Law::compile(&turn_law(false)).unwrap();
    let turn = iterating(turn.canonical(), [0, 1]);
    for (size, sparse, stepping) in [
        ([16, 16, 16], false, Stepping::Listed),
        ([16, 16, 16], false, Stepping::Whole),
        ([32, 32, 32], true, Stepping::Listed),
    ] {
        let mut rng = Rng(77 + u64::from(sparse));
        let mut start = vec![0_u32; cells(size)];
        for z in 4..12 {
            for y in 4..12 {
                for x in 4..12 {
                    start[index(size, [x, y, z])] = rng.word();
                }
            }
        }
        let label = format!(
            "turn 32 bits {stepping:?} {}",
            if sparse { "sparse" } else { "dense" }
        );
        let next =
            move |frame: u32, from: &[u32], into: &[u32]| margolus(size, frame, from, into, false);
        runs.push(counted_exact(
            &host,
            &turn,
            size,
            (sparse, stepping),
            &start,
            &next,
            &label,
        ));
    }
    for run in &runs {
        println!(
            "i1 {}: k 1..={} equal, and {} frames in one call",
            run.label, run.frames, run.frames
        );
    }
    println!(
        "i1 cases {}, frames compared {}",
        runs.len(),
        runs.len() * 65
    );
    assert_eq!(runs.len(), 19);
}

fn placed(size: [u32; 3], points: &[[u32; 3]]) -> Vec<u32> {
    let mut out = vec![0_u32; cells(size)];
    for at in points {
        out[index(size, *at)] = 1;
    }
    out
}

fn moved(
    host: &Host,
    shape: Shape,
    size: [u32; 3],
    points: &[[u32; 3]],
    (frames, batch): (u32, u32),
) -> (u32, u32) {
    let law = iterating(&learned(host, shape, "wrapped"), [0, 1]);
    let mut one = world(host, &law, size, false, Stepping::Listed);
    let start = placed(size, points);
    seeded(&mut one, 0, size, &start);
    let mut host_cells = start.clone();
    let blocks = one.listed().unwrap();
    let mut most_listed = 0;
    let mut done = 0;
    while done < frames {
        one.step(batch).unwrap();
        for _ in 0..batch {
            host_cells = generation(shape, size, &host_cells, true, 1);
        }
        done += batch;
        let got = current_of(&mut one, size);
        assert!(
            got == host_cells,
            "{shape:?} glider at frame {done}: {}",
            first_difference(&got, &host_cells, size)
        );
        let listed = one.listed().unwrap();
        most_listed = most_listed.max(listed);
        assert!(
            listed > 0 && listed < blocks / 4,
            "{shape:?} at frame {done}: listed {listed} of {blocks}"
        );
    }
    let population = host_cells.iter().filter(|word| **word != 0).count();
    assert_eq!(
        population,
        points.len(),
        "{shape:?}: the glider kept its cells"
    );
    assert!(host_cells != start, "{shape:?}: the glider moved");
    (done, most_listed)
}

#[test]
fn i2_gliders_move_on_listed_worlds_and_still_worlds_go_quiet() {
    let Some(host) = device("law_iterate i2") else {
        return;
    };
    let life = [[1, 0, 0], [2, 1, 0], [0, 2, 0], [1, 2, 0], [2, 2, 0]]
        .map(|[x, y, z]| [x + 20, y + 20, z]);
    let (frames, listed) = moved(&host, Shape::Square, [64, 64, 2], &life, (504, 8));
    println!(
        "i2 Life glider: {frames} frames on a listed 64x64x2 world, at most {listed} octs listed, equal to the host every 8"
    );
    let bays: Vec<[u32; 3]> = [[1, 0], [2, 1], [0, 2], [1, 2], [2, 2]]
        .into_iter()
        .flat_map(|[x, y]| [0, 1].map(|z| [x + 10, y + 10, z + 10]))
        .collect();
    let (frames, listed) = moved(&host, Shape::Cube, [32, 32, 32], &bays, (252, 4));
    println!(
        "i2 Bays 5766 glider: {frames} frames on a listed 32^3 world, at most {listed} octs listed, equal to the host every 4"
    );

    let law = iterating(&learned(&host, Shape::Square, "wrapped"), [0, 1]);
    let size = [32, 32, 2];
    let still = placed(
        size,
        &[
            [5, 5, 0],
            [6, 5, 0],
            [5, 6, 0],
            [6, 6, 0],
            [20, 10, 0],
            [21, 10, 0],
            [22, 10, 0],
        ],
    );
    let mut quiet = world(&host, &law, size, false, Stepping::Listed);
    seeded(&mut quiet, 0, size, &still);
    let mut host_cells = still.clone();
    let mut fell = None;
    for frame in 1..=16_u32 {
        quiet.step(1).unwrap();
        host_cells = generation(Shape::Square, size, &host_cells, true, 1);
        let got = current_of(&mut quiet, size);
        assert!(got == host_cells, "still life and blinker at frame {frame}");
        if fell.is_none() && quiet.listed().unwrap() == 0 {
            fell = Some(frame);
        }
    }
    let fell = fell.expect("a block and a blinker go quiet");
    println!(
        "i2 a block and a blinker: no oct listed from frame {fell}, and both planes still read the right generations to frame 16"
    );
    assert!(fell <= 4, "quiet from frame {fell}");

    let fly = iterating(Law::compile(&turn_law(true)).unwrap().canonical(), [0, 1]);
    let size = [32, 32, 32];
    let mut sparse = world(&host, &fly, size, true, Stepping::Listed);
    let mut start = vec![0_u32; cells(size)];
    start[index(size, [3, 3, 3])] = 0x8000_0001;
    start[index(size, [17, 9, 30])] = 0x0F0F_00F0;
    seeded(&mut sparse, 0, size, &start);
    let mut planes = [start.clone(), vec![0_u32; start.len()]];
    let mut frame = 0_u32;
    for _ in 0..31 {
        for _ in 0..8 {
            let from = (frame & 1) as usize;
            planes[1 - from] = margolus(size, frame, &planes[from], &planes[1 - from], true);
            frame += 1;
        }
        sparse.step(8).unwrap();
        let got = current_of(&mut sparse, size);
        assert!(
            got == planes[0],
            "two flying words on a sparse world at frame {frame}: {}",
            first_difference(&got, &planes[0], size)
        );
        assert!(sparse.pool_use().unwrap().is_some());
    }
    let held: Vec<usize> = (0..planes[0].len())
        .filter(|at| planes[0][*at] != 0)
        .collect();
    assert_eq!(held.len(), 2);
    assert!(planes[0] != start, "the words moved");
    println!(
        "i2 two words flying across a sparse 32^3 world: {frame} frames, equal to the host every 8, now at cells {held:?}"
    );
}

fn alternated_create(law: &str, size: [u32; 3], more: &str) -> String {
    format!(
        "{{\"wide\":{},\"deep\":{},\"tall\":{},\"law\":{},\"alternate\":[0,1]{more},\"budget_ms\":{LONG}}}",
        size[0],
        size[1],
        size[2],
        quoted(law)
    )
}

fn served_cells(size: [u32; 3], words: &[u32]) -> Vec<String> {
    let mut out = Vec::new();
    for z in 0..size[2] {
        for y in 0..size[1] {
            for x in 0..size[0] {
                let word = words[index(size, [x, y, z])];
                if word != 0 {
                    out.push(format!("[{x},{y},{z},{word}]"));
                }
            }
        }
    }
    out
}

fn served_write(server: &mut Server<'_>, world: i64, plane: &str, cells: &[String]) -> Json {
    let mut last = Json::Null;
    for piece in cells.chunks(4096) {
        last = ok(
            server,
            "grit_world_write",
            &format!(
                "{{\"world\":{world},{plane},\"cells\":[{}],\"budget_ms\":{LONG}}}",
                piece.join(",")
            ),
        );
    }
    last
}

fn served_read(server: &mut Server<'_>, world: i64, size: [u32; 3], plane: &str) -> Vec<u32> {
    let per = (4096 / (size[0] * size[1])).max(1);
    let mut out = Vec::new();
    let mut z = 0;
    while z < size[2] {
        let top = (z + per).min(size[2]) - 1;
        let got = ok(
            server,
            "grit_world_read",
            &format!(
                "{{\"world\":{world},{plane},\"from\":[0,0,{z}],\"to\":[{},{},{top}]}}",
                size[0] - 1,
                size[1] - 1
            ),
        );
        if plane.contains("current") {
            let frame = int_at(&got, "frame");
            assert_eq!(int_at(&got, "plane"), frame & 1, "{}", written(&got));
        }
        for cell in got.get("cells").and_then(Json::list).unwrap() {
            out.push(u32::try_from(cell.int().unwrap()).unwrap());
        }
        z = top + 1;
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
    got
}

fn generations(size: [u32; 3], from: &[u32], count: u32) -> Vec<u32> {
    let mut held = from.to_vec();
    for _ in 0..count {
        held = generation(Shape::Square, size, &held, true, 1);
    }
    held
}

struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "grit-law-iterate-{label}-{}-{nanos}",
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

#[test]
fn i3_parity_survives_journal_fork_restore_save_open_params_and_restart() {
    let Some(host) = device("law_iterate i3") else {
        return;
    };
    let law = learned(&host, Shape::Square, "wrapped");
    let size = [32, 32, 2];
    let mut rng = Rng(303);
    let start = soup(&mut rng, size, 300, 1);
    let scratch = Scratch::new("i3");
    let options = grit::serve3::Options {
        dir: Some(scratch.0.join("worlds")),
        memory: None,
    };
    let (number, kept) = {
        let mut server = Server::with(Some(&host), &options).unwrap();
        let made = ok(
            &mut server,
            "grit_world_create",
            &alternated_create(&law, size, ""),
        );
        assert_eq!(written(made.get("alternate").unwrap()), "[0,1]");
        let number = int_at(&made, "world");
        served_write(
            &mut server,
            number,
            "\"current\":true",
            &served_cells(size, &start),
        );
        let mut host_cells = start.clone();
        served_step(&mut server, number, 3);
        host_cells = generations(size, &host_cells, 3);
        assert_eq!(
            served_read(&mut server, number, size, "\"current\":true"),
            host_cells
        );
        let fork = ok(
            &mut server,
            "grit_world_fork",
            &format!("{{\"world\":{number}}}"),
        );
        let forked = int_at(&fork, "world");
        assert_eq!(int_at(&fork, "frame"), 3);
        let fork_cells = served_read(&mut server, forked, size, "\"current\":true");
        assert_eq!(
            fork_cells, host_cells,
            "the fork reads the current plane of its source"
        );
        served_step(&mut server, forked, 5);
        let fork_host = generations(size, &host_cells, 5);
        assert_eq!(
            served_read(&mut server, forked, size, "\"current\":true"),
            fork_host,
            "the fork continues from an odd frame"
        );
        let mut extra = vec![0_u32; cells(size)];
        for at in [
            [1, 1, 0],
            [2, 1, 0],
            [3, 1, 0],
            [9, 20, 1],
            [10, 20, 1],
            [11, 20, 1],
        ] {
            extra[index(size, at)] = 1;
        }
        let wrote = served_write(
            &mut server,
            number,
            "\"current\":true",
            &served_cells(size, &extra),
        );
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
            served_read(&mut server, number, size, "\"current\":true"),
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
            served_read(&mut server, number, size, "\"current\":true"),
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
            served_read(&mut server, reopened, size, "\"current\":true"),
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
        assert_eq!(
            served_read(&mut server, number, size, "\"current\":true"),
            host_cells
        );
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
        served_read(&mut again, replayed, size, "\"current\":true"),
        kept,
        "the journal replays to odd frame 13"
    );
    served_step(&mut again, replayed, 5);
    assert_eq!(
        served_read(&mut again, replayed, size, "\"current\":true"),
        generations(size, &kept, 5),
        "after a restart at odd frame 13, 5 more frames"
    );
    println!(
        "i3 one Life world: frames 3 fork 5, writes at 3, params at 5, restore at 8, save and open at 9, back refused, restart at 13 and 5 more, every read equal to the host"
    );
}

#[test]
fn i5_misuse_is_refused_by_code_and_field() {
    let Some(host) = device("law_iterate i5") else {
        return;
    };
    let life = learned(&host, Shape::Square, "wrapped");
    let mut server = Server::new(Some(&host));
    let size = [16, 16, 2];
    let create = |law: &str, pair: &str, more: &str| {
        format!(
            "{{\"wide\":{},\"deep\":{},\"tall\":{},\"law\":{},\"alternate\":{pair}{more}}}",
            size[0],
            size[1],
            size[2],
            quoted(law)
        )
    };
    let refused =
        |server: &mut Server<'_>, arguments: &str, code: &str, field: &str, inner: &str| {
            let got = no(server, "grit_world_create", arguments);
            assert_eq!(text_at(&got, "code"), code, "{}", written(&got));
            assert_eq!(text_at(&got, "field"), field, "{}", written(&got));
            if !inner.is_empty() {
                assert_eq!(
                    text_at(got.get("law").unwrap(), "code"),
                    inner,
                    "{}",
                    written(&got)
                );
            }
            println!("i5 {code} {inner} at {field}");
        };
    refused(
        &mut server,
        &create(&life, "[0,0]", ""),
        "law_refused",
        "$.alternate",
        "alternate_plane",
    );
    refused(
        &mut server,
        &create(&life, "[0,2]", ""),
        "law_refused",
        "$.alternate",
        "alternate_plane",
    );
    refused(
        &mut server,
        &create(&life, "[1,0]", ""),
        "law_refused",
        "$.alternate",
        "alternate_written",
    );
    let cell = "[\"vec\",[\"at\",[\"at\",\"cells\",\"c\"],0],\"w1_0\"]";
    assert!(life.contains(cell));
    let three = life.replacen("\"channels\":2", "\"channels\":3", 1).replace(
        cell,
        "[\"vec\",[\"at\",[\"at\",\"cells\",\"c\"],0],\"w1_0\",[\"at\",[\"at\",\"cells\",\"c\"],2]]",
    );
    refused(
        &mut server,
        &create(&three, "[0,2]", ""),
        "law_refused",
        "$.alternate",
        "alternate_unwritten",
    );
    let both = life
        .replacen("\"channels\":2", "\"channels\":3", 1)
        .replace(
            cell,
            "[\"vec\",[\"at\",[\"at\",\"cells\",\"c\"],0],\"w1_0\",\"w1_0\"]",
        );
    refused(
        &mut server,
        &create(&both, "[0,1]", ""),
        "law_refused",
        "$.alternate",
        "alternate_other",
    );
    let blind = blind_law();
    refused(
        &mut server,
        &create(&blind, "[0,1]", ""),
        "law_refused",
        "$.alternate",
        "alternate_unread",
    );
    refused(
        &mut server,
        &create(&reversible_law(), "[0,1]", ""),
        "law_refused",
        "$.alternate",
        "alternate_reversible",
    );
    refused(
        &mut server,
        &create(&life, "[0,1]", ",\"sparse\":true"),
        "halo_sparse",
        "$.sparse",
        "",
    );
    let carried = format!("{},\"alternate\":[1,0]}}", &life[..life.len() - 1]);
    refused(
        &mut server,
        &create(&carried, "[0,1]", ""),
        "malformed",
        "$.alternate",
        "",
    );
    let bad = format!("{},\"alternate\":[0]}}", &life[..life.len() - 1]);
    let got = no(
        &mut server,
        "grit_world_create",
        &format!(
            "{{\"wide\":16,\"deep\":16,\"tall\":2,\"law\":{}}}",
            quoted(&bad)
        ),
    );
    assert_eq!(text_at(&got, "code"), "law_refused");
    assert_eq!(text_at(got.get("law").unwrap(), "path"), "$.alternate");
    let got = no(
        &mut server,
        "grit_world_create",
        "{\"wide\":16,\"deep\":16,\"tall\":16,\"rule\":\"sand\",\"alternate\":[0,1]}",
    );
    assert_eq!(text_at(&got, "code"), "malformed");
    assert_eq!(text_at(&got, "field"), "$.alternate");
    let plain = ok(
        &mut server,
        "grit_world_create",
        &format!(
            "{{\"wide\":16,\"deep\":16,\"tall\":2,\"law\":{}}}",
            quoted(&life)
        ),
    );
    assert!(plain.get("alternate").is_none());
    let plain = int_at(&plain, "world");
    let got = no(
        &mut server,
        "grit_world_read",
        &format!("{{\"world\":{plain},\"from\":[0,0,0],\"to\":[1,1,0],\"current\":true}}"),
    );
    assert_eq!(text_at(&got, "code"), "not_iterating");
    assert_eq!(text_at(&got, "field"), "$.current");
    let made = ok(
        &mut server,
        "grit_world_create",
        &create(&life, "[0,1]", ""),
    );
    let iterating = int_at(&made, "world");
    let got = no(
        &mut server,
        "grit_world_read",
        &format!(
            "{{\"world\":{iterating},\"from\":[0,0,0],\"to\":[1,1,0],\"current\":true,\"plane\":1}}"
        ),
    );
    assert_eq!(text_at(&got, "code"), "malformed");
    assert_eq!(text_at(&got, "field"), "$.plane");
    let got = no(
        &mut server,
        "grit_world_back",
        &format!("{{\"world\":{iterating},\"frames\":1}}"),
    );
    assert_eq!(text_at(&got, "code"), "not_reversible");
    println!(
        "i5 not_iterating at $.current, malformed at $.plane and $.alternate, not_reversible for back"
    );
}

fn blind_law() -> String {
    let out: Vec<String> = (0..8)
        .map(|k| format!("[\"vec\",[\"at\",[\"at\",\"cells\",{k}],0],7]"))
        .collect();
    format!(
        "{{\"law\":1,\"name\":\"blind\",\"params\":0,\"channels\":2,\"halo\":0,\"tally\":{{\"counts\":1,\"weighted\":false}},\"consts\":{{}},\"defs\":{{}},\"step\":[\"vec\",[\"vec\",{}],[\"vec\",0]]}}",
        out.join(",")
    )
}

fn reversible_law() -> String {
    let out: Vec<String> = (0..8)
        .map(|k| {
            format!(
                "[\"vec\",[\"at\",[\"at\",\"cells\",{k}],0],[\"add\",[\"at\",[\"at\",\"cells\",{k}],1],[\"at\",[\"at\",\"cells\",{k}],0]]]"
            )
        })
        .collect();
    format!(
        "{{\"law\":1,\"name\":\"lift\",\"params\":0,\"channels\":2,\"halo\":0,\"reversible\":true,\"tally\":{{\"counts\":1,\"weighted\":false}},\"consts\":{{}},\"defs\":{{}},\"step\":[\"vec\",[\"vec\",{}],[\"vec\",0]]}}",
        out.join(",")
    )
}

#[test]
fn i6_a_learned_outer_life_runs_64_generations_as_the_host() {
    let Some(host) = device("law_iterate i6") else {
        return;
    };
    let answer = learned_answer(&host, Shape::Square, "wrapped");
    assert_eq!(
        written(answer.get("tables").unwrap()),
        table_of(Shape::Square)
    );
    let law = text_at(&answer, "law");
    let mut server = Server::new(Some(&host));
    let size = [64, 64, 2];
    let made = ok(
        &mut server,
        "grit_world_create",
        &alternated_create(&law, size, ""),
    );
    assert_eq!(text_at(&made, "quiet"), "proved");
    let number = int_at(&made, "world");
    let mut rng = Rng(606);
    let start = soup(&mut rng, size, 350, 1);
    served_write(
        &mut server,
        number,
        "\"current\":true",
        &served_cells(size, &start),
    );
    let stepped = served_step(&mut server, number, 64);
    assert_eq!(int_at(&stepped, "frame"), 64);
    assert_eq!(int_at(&stepped, "current"), 0);
    let got = served_read(&mut server, number, size, "\"current\":true");
    let want = generations(size, &start, 64);
    assert!(got == want, "{}", first_difference(&got, &want, size));
    let alive = want.iter().filter(|word| **word != 0).count();
    println!(
        "i6 learned outer Life, alternate [0,1]: 64 generations in one step call equal the host, {alive} cells alive"
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

    use super::{LONG, Rng, Scratch, Shape, device, generation, int_at, ok, quoted, soup};

    fn hex(sum: &[u8; 32]) -> String {
        sum.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    const BLUR: &str = include_str!("halo3/blur2.law.json");

    const MIX: &str = include_str!("law/mix.law.json");

    const STEADY: [&str; 7] = [
        "world", "from", "name", "written", "frame", "frames", "tally",
    ];

    struct Case {
        name: &'static str,
        learned: Option<(Shape, &'static str)>,
        size: [u32; 3],
        params: &'static str,
        sparse: bool,
        planes: u32,
        mask: u32,
        seed: u64,
    }

    const CASES: [Case; 4] = [
        Case {
            name: "learned-life",
            learned: Some((Shape::Square, "wrapped")),
            size: [32, 32, 2],
            params: "[]",
            sparse: false,
            planes: 2,
            mask: 1,
            seed: 0x11FE,
        },
        Case {
            name: "learned-bays",
            learned: Some((Shape::Cube, "zero")),
            size: [16, 16, 16],
            params: "[]",
            sparse: false,
            planes: 2,
            mask: 1,
            seed: 0xBA75,
        },
        Case {
            name: "hand-blur",
            learned: None,
            size: [32, 32, 8],
            params: "[]",
            sparse: false,
            planes: 3,
            mask: 0xFFFF,
            seed: 0xB1B2,
        },
        Case {
            name: "mix-sparse",
            learned: None,
            size: [64, 64, 32],
            params: "[1,2]",
            sparse: true,
            planes: 1,
            mask: 15,
            seed: 0x5A75,
        },
    ];

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

    fn write(server: &mut Server<'_>, world: i64, plane: u32, cells: &[String]) -> Json {
        ok(
            server,
            "grit_world_write",
            &format!(
                "{{\"world\":{world},\"plane\":{plane},\"cells\":[{}],\"budget_ms\":{LONG}}}",
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

    fn at_of(size: [u32; 3], at: usize) -> [usize; 3] {
        [
            at % size[0] as usize,
            at / size[0] as usize % size[1] as usize,
            at / (size[0] as usize * size[1] as usize),
        ]
    }

    fn copied_back(
        server: &mut Server<'_>,
        world: i64,
        case: &Case,
        shape: Shape,
        held: &mut Vec<u32>,
    ) {
        let size = case.size;
        let next = read_plane(server, world, size, 1);
        assert_eq!(
            next,
            generation(shape, size, held, true, 1),
            "{}",
            case.name
        );
        let cells: Vec<String> = (0..next.len())
            .filter(|at| next[*at] != held[*at])
            .map(|at| {
                let [x, y, z] = at_of(size, at);
                format!("[{x},{y},{z},{}]", next[at])
            })
            .collect();
        for piece in cells.chunks(4096) {
            write(server, world, 0, piece);
        }
        *held = next;
    }

    fn played(server: &mut Server<'_>, case: &Case) -> (i64, Vec<String>) {
        let mut answers = Vec::new();
        let law = match case.learned {
            Some((shape, border)) => {
                let got = super::learned_answer_on(server, shape, border);
                answers.push(written(&got));
                super::text_at(&got, "law")
            }
            None if case.sparse => MIX.to_owned(),
            None => BLUR.to_owned(),
        };
        let made = ok(
            server,
            "grit_world_create",
            &format!(
                "{{\"wide\":{},\"deep\":{},\"tall\":{},\"law\":{},\"params\":{},\"sparse\":{},\"budget_ms\":{LONG}}}",
                case.size[0],
                case.size[1],
                case.size[2],
                quoted(&law),
                case.params,
                case.sparse
            ),
        );
        let world = int_at(&made, "world");
        answers.push(steady(&made, case.sparse));
        let size = case.size;
        if let Some((shape, _)) = case.learned {
            let mut rng = Rng(case.seed);
            let mut held = soup(&mut rng, size, 300, 1);
            let cells: Vec<String> = (0..held.len())
                .filter(|at| held[*at] != 0)
                .map(|at| {
                    let [x, y, z] = at_of(size, at);
                    format!("[{x},{y},{z},1]")
                })
                .collect();
            for piece in cells.chunks(4096) {
                answers.push(steady(&write(server, world, 0, piece), case.sparse));
            }
            for _ in 0..3 {
                answers.push(steady(&step(server, world, 1), case.sparse));
                copied_back(server, world, case, shape, &mut held);
            }
        } else {
            let first = scattered(case.seed, size, 600, case.mask);
            answers.push(steady(&write(server, world, 0, &first), case.sparse));
        }
        let fork = ok(server, "grit_world_fork", &format!("{{\"world\":{world}}}"));
        let forked = int_at(&fork, "world");
        answers.push(steady(&fork, case.sparse));
        answers.push(steady(&step(server, world, 2), case.sparse));
        let more = scattered(case.seed ^ 1, size, 200, case.mask);
        answers.push(steady(&write(server, world, 0, &more), case.sparse));
        if case.planes > 1 {
            let upper = scattered(case.seed ^ 2, size, 100, case.mask);
            answers.push(steady(&write(server, world, 1, &upper), case.sparse));
        }
        answers.push(steady(&step(server, world, 3), case.sparse));
        answers.push(read_all(server, world, case));
        let restored = ok(
            server,
            "grit_world_restore",
            &format!("{{\"world\":{forked},\"from\":{world}}}"),
        );
        answers.push(steady(&restored, case.sparse));
        answers.push(steady(&step(server, forked, 2), case.sparse));
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

    const RECORDED: [(&str, &str, &str); 4] = [
        (
            "learned-life",
            "d515be105161e94923f20cd3d3ac80a22143c8ffd19d24a6d525ca2831fcd2ed",
            "13923f15562347444db8901b9cf6f450dfbeadd3e8a78b927931409afe761307",
        ),
        (
            "learned-bays",
            "39c8950058e2ad895963a1cc6c30740a5cce3a395040097f9e07fa9ef89472c6",
            "16b88b7a7a796ae50b52aa19ed92ce90b132c236f2a06949087022956cbb2465",
        ),
        (
            "hand-blur",
            "fba9644c32c07421b1f74eeea5a761a1071b68c9d6ba2de1c5ef6c43456d0056",
            "6324cce8024dd0a6a46826dd1d6b61847f22f8e35dc299f4f26c277206ef31d9",
        ),
        (
            "mix-sparse",
            "75c667e29cee24c4ad08f852ba6dd4426fad5c2a6e7e2ae9a1c775a28777cede",
            "",
        ),
    ];

    #[test]
    fn i4_existing_laws_answer_read_and_journal_as_on_the_recorded_base() {
        let Some(host) = device("law_iterate i4") else {
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
            if !want_journal.is_empty() {
                assert_eq!(journal, want_journal, "{name}: journal");
            }
        }
    }
}
