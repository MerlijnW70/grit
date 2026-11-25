#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::BTreeMap;

use grit::serve3::Server;
use std::time::Instant;

use grit::Host;
use grit::wire::{Json, written};

mod common;

use common::device;

const LONG: u64 = 600_000;

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

fn sha_of(held: &Json) -> String {
    grit::law::hex(&grit::branch::sha256(written(held).as_bytes()))
}

fn listed<T: ToString>(held: &[T]) -> String {
    format!(
        "[{}]",
        held.iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn heard(server: &mut Server<'_>, name: &str, arguments: &str) -> Json {
    server.heard(&line(name, arguments)).unwrap()
}

fn bits(rng: &mut Rng, cells: usize, permille: u64) -> Vec<u32> {
    (0..cells)
        .map(|_| u32::from(rng.below(1000) < permille))
        .collect()
}

fn counted_step(
    side: usize,
    cells: &[u32],
    wrapped: bool,
    table: &dyn Fn(u32, u32) -> u32,
) -> Vec<u32> {
    let at = |x: i64, y: i64| -> u32 {
        let side = side as i64;
        if wrapped {
            cells[(y.rem_euclid(side) * side + x.rem_euclid(side)) as usize]
        } else if (0..side).contains(&x) && (0..side).contains(&y) {
            cells[(y * side + x) as usize]
        } else {
            0
        }
    };
    let mut out = vec![0; cells.len()];
    for y in 0..side as i64 {
        for x in 0..side as i64 {
            let mut around = 0;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    if dx != 0 || dy != 0 {
                        around += at(x + dx, y + dy);
                    }
                }
            }
            out[(y * side as i64 + x) as usize] = table(at(x, y), around);
        }
    }
    out
}

fn counted_examples(
    seed: u64,
    many: usize,
    wrapped: bool,
    table: &dyn Fn(u32, u32) -> u32,
) -> String {
    let mut rng = Rng(seed);
    let side = 16;
    let shown: Vec<String> = (0..many)
        .map(|_| {
            let input = bits(&mut rng, side * side, 350);
            let output = counted_step(side, &input, wrapped, table);
            format!(
                "{{\"size\":[{side},{side},1],\"input\":[{}],\"output\":[{}]}}",
                listed(&input),
                listed(&output)
            )
        })
        .collect();
    format!("[{}]", shown.join(","))
}

fn bit_args(rule: &str, border: &str, examples: &str, more: &str) -> String {
    format!(
        "{{\"stencil\":\"3x3\",\"reads\":[{{\"plane\":0,\"bit\":0}}],\"writes\":[{{\"plane\":1,\"bit\":0}}],\"border\":\"{border}\",\"rule\":\"{rule}\",\"examples\":{examples},\"budget_ms\":{LONG}{more}}}"
    )
}

fn life(centre: u32, around: u32) -> u32 {
    u32::from(around == 3 || (centre == 1 && around == 2))
}

fn majority(centre: u32, around: u32) -> u32 {
    u32::from(centre + around >= 5)
}

const BASE_SHA: [(&str, &str); 5] = [
    (
        "life",
        "9ed28152bcbfbc1ade762ee5fe5b142555912687b7708cf64204a1f3e66fa90d",
    ),
    (
        "majority",
        "567138b6b270173602064de86eadf9e8a09fd4538422b30209798d45ec98d44b",
    ),
    (
        "vn",
        "7437b778030534bf382b333de70e9b8288a0d06fb8d6fa0ab9ed4b66487c0916",
    ),
    (
        "not_counting",
        "bb6d2326df3a97cbe14ae04296bf8dd693e915cfdf57ec499d701a7b029ab5cc",
    ),
    (
        "places_rule",
        "1ff79d7ad05a4b367b87fb6c03d8a5990965ab4e1b4ea59dd6f494ba8bd1f264",
    ),
];

fn base_requests() -> Vec<(&'static str, String, bool)> {
    let life_pairs = counted_examples(61, 6, true, &life);
    let major = counted_examples(62, 6, false, &majority);
    let vn = counted_examples(63, 6, true, &|centre, around| {
        u32::from(centre + around >= 4)
    });
    vec![
        ("life", bit_args("outer", "wrapped", &life_pairs, ""), false),
        (
            "majority",
            bit_args("totalistic", "zero", &major, ""),
            false,
        ),
        (
            "vn",
            bit_args("outer", "wrapped", &vn, ",\"places\":\"moore\""),
            false,
        ),
        (
            "not_counting",
            bit_args("totalistic", "wrapped", &life_pairs, ""),
            true,
        ),
        (
            "places_rule",
            bit_args("minimal", "wrapped", &life_pairs, ",\"places\":\"moore\""),
            true,
        ),
    ]
}

#[test]
fn r6_requests_of_the_other_rules_answer_the_bytes_recorded_on_the_base() {
    let pinned: BTreeMap<&str, &str> = BASE_SHA.into_iter().collect();
    let mut server = Server::new(None);
    let mut wrong = Vec::new();
    for (name, arguments, error) in base_requests() {
        let got = heard(&mut server, "grit_law_learn", &arguments);
        let told = result(&got, error);
        if error {
            assert_eq!(text_at(&told, "refused"), name);
        }
        let sha = sha_of(&got);
        eprintln!("BASE {name} {sha}");
        if sha != pinned[name] {
            wrong.push(format!("{name} {sha}"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:?}");
}

const READ_MOST: usize = 4096;

fn scaled(frac: u32) -> f64 {
    f64::from(1_u32 << frac)
}

fn fixed_word(value: f64, frac: u32) -> u32 {
    ((value * scaled(frac)).round() as i64 as i32).cast_unsigned()
}

fn real_of(word: u32, frac: u32) -> f64 {
    f64::from(word.cast_signed()) / scaled(frac)
}

fn index(size: [usize; 3], at: [usize; 3]) -> usize {
    (at[2] * size[1] + at[1]) * size[0] + at[0]
}

fn moved(size: [usize; 3], at: [usize; 3], by: [i64; 3]) -> usize {
    let mut out = [0; 3];
    for axis in 0..3 {
        out[axis] = (at[axis] as i64 + by[axis]).rem_euclid(size[axis] as i64) as usize;
    }
    index(size, out)
}

fn cells(size: [usize; 3]) -> Vec<[usize; 3]> {
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

fn offset_of(place: u32, cube: bool) -> [i64; 3] {
    let place = i64::from(place);
    let dz = if cube { place / 9 - 1 } else { 0 };
    [place % 3 - 1, place / 3 % 3 - 1, dz]
}

fn stencil_step(
    size: [usize; 3],
    field: &[f64],
    weights: &[([i64; 3], f64)],
    bias: f64,
) -> Vec<f64> {
    cells(size)
        .into_iter()
        .map(|at| {
            weights
                .iter()
                .map(|(by, weight)| weight * field[moved(size, at, *by)])
                .sum::<f64>()
                + bias
        })
        .collect()
}

fn five_point(alpha: f64) -> Vec<([i64; 3], f64)> {
    vec![
        ([0, 0, 0], 1.0 - 4.0 * alpha),
        ([1, 0, 0], alpha),
        ([-1, 0, 0], alpha),
        ([0, 1, 0], alpha),
        ([0, -1, 0], alpha),
    ]
}

fn nine_point(alpha: f64) -> Vec<([i64; 3], f64)> {
    let mut out = vec![([0, 0, 0], 1.0 - 20.0 * alpha / 6.0)];
    for dy in -1..=1_i64 {
        for dx in -1..=1_i64 {
            match dx.abs() + dy.abs() {
                1 => out.push(([dx, dy, 0], 4.0 * alpha / 6.0)),
                2 => out.push(([dx, dy, 0], alpha / 6.0)),
                _ => {}
            }
        }
    }
    out
}

fn seven_point(alpha: f64) -> Vec<([i64; 3], f64)> {
    let mut out = vec![([0, 0, 0], 1.0 - 6.0 * alpha)];
    for axis in 0..3 {
        for sign in [-1, 1] {
            let mut by = [0; 3];
            by[axis] = sign;
            out.push((by, alpha));
        }
    }
    out
}

fn uniform(rng: &mut Rng, least: f64, most: f64) -> f64 {
    least + (most - least) * (rng.below(1 << 24) as f64 / f64::from(1_u32 << 24))
}

fn gaussian(rng: &mut Rng) -> f64 {
    let one = (rng.below(1 << 24) as f64 + 0.5) / f64::from(1_u32 << 24);
    let two = rng.below(1 << 24) as f64 / f64::from(1_u32 << 24);
    (-2.0 * one.ln()).sqrt() * (std::f64::consts::TAU * two).cos()
}

fn field(rng: &mut Rng, count: usize, least: f64, most: f64, frac: u32) -> Vec<u32> {
    (0..count)
        .map(|_| fixed_word(uniform(rng, least, most), frac))
        .collect()
}

fn reals(words: &[u32], frac: u32) -> Vec<f64> {
    words.iter().map(|one| real_of(*one, frac)).collect()
}

fn words_of(values: &[f64], frac: u32) -> Vec<u32> {
    values.iter().map(|one| fixed_word(*one, frac)).collect()
}

fn inline(size: [usize; 3], inputs: &[Vec<u32>], outputs: &[Vec<u32>]) -> String {
    let shown = |lists: &[Vec<u32>]| {
        lists
            .iter()
            .map(|one| listed(one))
            .collect::<Vec<_>>()
            .join(",")
    };
    format!(
        "{{\"size\":[{},{},{}],\"input\":[{}],\"output\":[{}]}}",
        size[0],
        size[1],
        size[2],
        shown(inputs),
        shown(outputs)
    )
}

fn spots(planes: &[u32]) -> String {
    let shown: Vec<String> = planes
        .iter()
        .map(|plane| format!("{{\"plane\":{plane},\"bit\":0}}"))
        .collect();
    format!("[{}]", shown.join(","))
}

fn linear_args(
    stencil: &str,
    reads: &[u32],
    writes: &[u32],
    examples: &[String],
    more: &str,
) -> String {
    format!(
        "{{\"stencil\":\"{stencil}\",\"reads\":{},\"writes\":{},\"border\":\"wrapped\",\"rule\":\"linear\",\"examples\":[{}],\"budget_ms\":{LONG}{more}}}",
        spots(reads),
        spots(writes),
        examples.join(",")
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

    fn call(&mut self, name: &str, arguments: &str, error: bool) -> Json {
        result(&heard(&mut self.server, name, arguments), error)
    }

    fn ok(&mut self, name: &str, arguments: &str) -> Json {
        self.call(name, arguments, false)
    }

    fn learn(&mut self, arguments: &str) -> Learned {
        Learned::of(self.ok("grit_law_learn", arguments))
    }

    fn refused(&mut self, arguments: &str, code: &str, field: &str) -> Json {
        let told = self.call("grit_law_learn", arguments, true);
        assert_eq!(
            (text_at(&told, "refused"), text_at(&told, "field")),
            (code.to_owned(), field.to_owned()),
            "{}",
            written(&told)
        );
        told
    }

    fn world(&mut self, law: &str, size: [usize; 3], more: &str) -> u64 {
        let got = self.ok(
            "grit_world_create",
            &format!(
                "{{\"wide\":{},\"deep\":{},\"tall\":{},\"law\":{}{more}}}",
                size[0],
                size[1],
                size[2],
                written(&Json::Text(law.to_owned()))
            ),
        );
        got.get("world").and_then(Json::int).unwrap() as u64
    }

    fn write(&mut self, world: u64, plane: u32, size: [usize; 3], words: &[u32]) {
        let all = cells(size);
        for piece in all.chunks(READ_MOST) {
            let shown: Vec<String> = piece
                .iter()
                .map(|at| {
                    format!(
                        "[{},{},{},{}]",
                        at[0],
                        at[1],
                        at[2],
                        words[index(size, *at)]
                    )
                })
                .collect();
            self.ok(
                "grit_world_write",
                &format!(
                    "{{\"world\":{world},\"plane\":{plane},\"cells\":[{}],\"budget_ms\":{LONG}}}",
                    shown.join(",")
                ),
            );
        }
    }

    fn step(&mut self, world: u64, frames: u32) {
        self.ok(
            "grit_world_step",
            &format!("{{\"world\":{world},\"frames\":{frames},\"budget_ms\":{LONG}}}"),
        );
    }

    fn read(&mut self, world: u64, plane: u32, size: [usize; 3]) -> Vec<u32> {
        let mut out = vec![0; size[0] * size[1] * size[2]];
        let rows = (READ_MOST / size[0]).max(1);
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
                let words: Vec<u32> = got
                    .get("cells")
                    .and_then(Json::list)
                    .unwrap()
                    .iter()
                    .map(|one| one.int().unwrap() as u32)
                    .collect();
                for (at, word) in words.into_iter().enumerate() {
                    let x = at % size[0];
                    let row = y + at / size[0];
                    out[index(size, [x, row, z])] = word;
                }
                y = last + 1;
            }
        }
        out
    }

    fn close(&mut self, world: u64) {
        self.ok("grit_world_close", &format!("{{\"world\":{world}}}"));
    }
}

#[derive(Clone, Debug)]
enum Kind {
    Place(usize, [i64; 3]),
    Product(usize, usize),
    Others(usize, usize),
    Cube(usize, usize, usize),
}

#[derive(Clone, Debug)]
struct Coefficient {
    fixed: i64,
    fit: f64,
    stderr: f64,
}

struct Learned {
    answer: Json,
    law: String,
    frac: u32,
    scales: Vec<u32>,
    kinds: Vec<Kind>,
    coefficients: Vec<Vec<Coefficient>>,
    bias: Vec<Coefficient>,
    clamp: Option<[i64; 2]>,
}

fn real_at(held: &Json, key: &str) -> f64 {
    text_at(held, key).parse().unwrap()
}

fn int_of(held: &Json, key: &str) -> i64 {
    held.get(key)
        .and_then(Json::int)
        .unwrap_or_else(|| panic!("no {key} in {}", written(held)))
}

fn reads_of(held: &Json) -> Vec<usize> {
    held.get("reads")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .map(|one| one.int().unwrap() as usize)
        .collect()
}

fn coefficient(held: &Json) -> Coefficient {
    Coefficient {
        fixed: int_of(held, "fixed"),
        fit: real_at(held, "fit"),
        stderr: real_at(held, "stderr"),
    }
}

impl Learned {
    fn of(answer: Json) -> Self {
        let fixed = answer.get("fixed").unwrap();
        let frac = int_of(fixed, "frac") as u32;
        let scales = fixed
            .get("coefficient_frac")
            .and_then(Json::list)
            .unwrap()
            .iter()
            .map(|one| one.int().unwrap() as u32)
            .collect();
        let clamp = fixed.get("clamp").map(|one| {
            let held = one.list().unwrap();
            [held[0].int().unwrap(), held[1].int().unwrap()]
        });
        let kinds = answer
            .get("terms")
            .and_then(Json::list)
            .unwrap()
            .iter()
            .map(|one| match text_at(one, "term").as_str() {
                "place" => {
                    let at = one.get("at").and_then(Json::list).unwrap();
                    Kind::Place(
                        int_of(one, "read") as usize,
                        [
                            at[0].int().unwrap(),
                            at[1].int().unwrap(),
                            at[2].int().unwrap(),
                        ],
                    )
                }
                "product" => {
                    let reads = reads_of(one);
                    Kind::Product(reads[0], reads[1])
                }
                "centre_times_others" => {
                    let reads = reads_of(one);
                    Kind::Others(reads[0], reads[1])
                }
                "cube" => {
                    let reads = reads_of(one);
                    Kind::Cube(reads[0], reads[1], reads[2])
                }
                other => panic!("term {other}"),
            })
            .collect();
        let coefficients = answer
            .get("coefficients")
            .and_then(Json::list)
            .unwrap()
            .iter()
            .map(|write| write.list().unwrap().iter().map(coefficient).collect())
            .collect();
        let bias = answer
            .get("bias")
            .and_then(Json::list)
            .unwrap()
            .iter()
            .map(coefficient)
            .collect();
        Self {
            law: text_at(&answer, "law"),
            answer,
            frac,
            scales,
            kinds,
            coefficients,
            bias,
            clamp,
        }
    }

    fn place(&self, write: usize, read: usize, at: [i64; 3]) -> Option<&Coefficient> {
        self.kinds
            .iter()
            .position(|kind| matches!(kind, Kind::Place(one, by) if *one == read && *by == at))
            .map(|term| &self.coefficients[write][term])
    }

    fn term(&self, write: usize, wanted: &dyn Fn(&Kind) -> bool) -> Option<&Coefficient> {
        self.kinds
            .iter()
            .position(wanted)
            .map(|term| &self.coefficients[write][term])
    }

    fn residual(&self, write: usize) -> &Json {
        &self.answer.get("residual").and_then(Json::list).unwrap()[write]
    }

    fn invariant(&self) -> bool {
        self.answer
            .get("symmetry")
            .and_then(|one| one.get("invariant"))
            == Some(&Json::Bool(true))
    }

    fn evaluate(&self, write: usize, look: &dyn Fn(usize, [i64; 3]) -> u32) -> u32 {
        let frac = self.frac;
        let rnd = |value: u32| -> u32 {
            if frac == 0 {
                return value;
            }
            let signed = i64::from(value.cast_signed());
            ((signed + (1_i64 << (frac - 1))).div_euclid(1_i64 << frac) as i32).cast_unsigned()
        };
        let centre = |read: usize| look(read, [0, 0, 0]);
        let others = |read: usize| -> u32 {
            self.kinds
                .iter()
                .filter_map(|kind| match kind {
                    Kind::Place(one, at) if *one == read && *at != [0, 0, 0] => Some(*at),
                    _ => None,
                })
                .fold(0_u32, |sum, at| sum.wrapping_add(look(read, at)))
        };
        let scale = self.scales[write];
        let mut total = self.bias[write].fixed;
        for (kind, one) in self.kinds.iter().zip(&self.coefficients[write]) {
            let term = match kind {
                Kind::Place(read, at) => look(*read, *at),
                Kind::Product(left, right) => rnd(centre(*left).wrapping_mul(centre(*right))),
                Kind::Others(read, around) => rnd(centre(*read).wrapping_mul(others(*around))),
                Kind::Cube(left, right, third) => {
                    rnd(rnd(centre(*left).wrapping_mul(centre(*right)))
                        .wrapping_mul(centre(*third)))
                }
            };
            total += one.fixed * i64::from(term.cast_signed());
        }
        let mut answer = (total + (1_i64 << (scale - 1))).div_euclid(1_i64 << scale);
        if let Some([least, most]) = self.clamp {
            answer = answer.clamp(least, most);
        }
        (answer as i32).cast_unsigned()
    }

    fn stepped(&self, size: [usize; 3], reads: &[Vec<u32>]) -> Vec<Vec<u32>> {
        (0..self.coefficients.len())
            .map(|write| {
                cells(size)
                    .into_iter()
                    .map(|at| self.evaluate(write, &|read, by| reads[read][moved(size, at, by)]))
                    .collect()
            })
            .collect()
    }
}

fn close_to(fit: &Coefficient, scale: u32, wanted: f64, what: &str) {
    let spread = 6.0 * fit.stderr + 1e-9;
    assert!(
        (fit.fit - wanted).abs() <= spread,
        "{what}: fit {} against {wanted}, stderr {}",
        fit.fit,
        fit.stderr
    );
    let constant = fit.fixed as f64 / scaled(scale);
    assert!(
        (constant - wanted).abs() <= spread + 0.5 / scaled(scale),
        "{what}: fixed {} ({constant}) against {wanted} at 2^-{scale}",
        fit.fixed
    );
}

fn heat_examples(
    rng: &mut Rng,
    size: [usize; 3],
    many: usize,
    weights: &[([i64; 3], f64)],
    frac: u32,
) -> Vec<String> {
    let count = size[0] * size[1] * size[2];
    (0..many)
        .map(|_| {
            let input = field(rng, count, 0.0, 1.0, frac);
            let output = words_of(
                &stencil_step(size, &reals(&input, frac), weights, 0.0),
                frac,
            );
            inline(size, &[input], &[output])
        })
        .collect()
}

fn recovered(learned: &Learned, weights: &[([i64; 3], f64)], cube: bool, mask: &[u32]) {
    let scale = learned.scales[0];
    for place in mask {
        let at = offset_of(*place, cube);
        let wanted = weights
            .iter()
            .find(|(by, _)| *by == at)
            .map_or(0.0, |(_, weight)| *weight);
        close_to(
            learned.place(0, 0, at).unwrap(),
            scale,
            wanted,
            &format!("place {place}"),
        );
    }
    let bias = &learned.bias[0];
    assert!(
        bias.fit.abs() <= 6.0 * bias.stderr + 1e-9,
        "bias {} stderr {}",
        bias.fit,
        bias.stderr
    );
}

fn alpha_of(learned: &Learned, faces: &[[i64; 3]], factor: f64) -> f64 {
    faces
        .iter()
        .map(|at| learned.place(0, 0, *at).unwrap().fit)
        .sum::<f64>()
        / faces.len() as f64
        / factor
}

const ALPHA5: f64 = 0.151_234_567;

const ALPHA9: f64 = 0.201_234_567;

const ALPHA7: f64 = 0.101_234_567;

const FACES2: [[i64; 3]; 4] = [[1, 0, 0], [-1, 0, 0], [0, 1, 0], [0, -1, 0]];

const FACES3: [[i64; 3]; 6] = [
    [1, 0, 0],
    [-1, 0, 0],
    [0, 1, 0],
    [0, -1, 0],
    [0, 0, 1],
    [0, 0, -1],
];

#[test]
fn r1_heat_in_two_and_three_dimensions_recovers_alpha() {
    let mut mcp = Mcp::new(None);
    let mut rng = Rng(101);
    let frac = 16;
    let square = [64, 64, 1];
    let five = five_point(ALPHA5);
    let examples = heat_examples(&mut rng, square, 4, &five, frac);
    let learned = mcp.learn(&linear_args(
        "3x3",
        &[0],
        &[1],
        &examples,
        ",\"places\":\"von_neumann\"",
    ));
    recovered(&learned, &five, false, &[1, 3, 4, 5, 7]);
    assert!((alpha_of(&learned, &FACES2, 1.0) - ALPHA5).abs() < 1e-5);
    assert!(
        learned.invariant(),
        "{}",
        written(learned.answer.get("symmetry").unwrap())
    );
    let nine = nine_point(ALPHA9);
    let examples = heat_examples(&mut rng, square, 4, &nine, frac);
    let learned = mcp.learn(&linear_args("3x3", &[0], &[1], &examples, ""));
    recovered(&learned, &nine, false, &(0..9).collect::<Vec<_>>());
    assert!((alpha_of(&learned, &FACES2, 4.0 / 6.0) - ALPHA9).abs() < 1e-5);
    assert!(learned.invariant());
    let seven = seven_point(ALPHA7);
    let cube = [16, 16, 16];
    let examples = heat_examples(&mut rng, cube, 4, &seven, frac);
    let learned = mcp.learn(&linear_args("3x3x3", &[0], &[1], &examples, ""));
    recovered(&learned, &seven, true, &(0..27).collect::<Vec<_>>());
    assert!((alpha_of(&learned, &FACES3, 1.0) - ALPHA7).abs() < 1e-5);
    assert!(learned.invariant());
    assert_eq!(
        text_at(learned.answer.get("symmetry").unwrap(), "group"),
        "Oh"
    );
}

fn iterate_alternating(
    mcp: &mut Mcp<'_>,
    learned: &Learned,
    size: [usize; 3],
    start: &[u32],
    frames: u32,
) -> Vec<u32> {
    let world = mcp.world(&learned.law, size, ",\"alternate\":[0,1]");
    mcp.write(world, 0, size, start);
    mcp.step(world, frames);
    let got = mcp.read(world, frames % 2, size);
    mcp.close(world);
    got
}

fn heat_bound(learned: &Learned, weights: &[([i64; 3], f64)], frames: u32) -> f64 {
    let frac = learned.frac;
    let scale = learned.scales[0];
    let off: f64 = learned
        .kinds
        .iter()
        .zip(&learned.coefficients[0])
        .map(|(kind, one)| {
            let Kind::Place(_, at) = kind else {
                panic!("heat has place terms only");
            };
            let wanted = weights
                .iter()
                .find(|(by, _)| by == at)
                .map_or(0.0, |(_, weight)| *weight);
            (one.fixed as f64 / scaled(scale) - wanted).abs()
        })
        .sum();
    let step =
        0.5 / scaled(frac) + off + (learned.bias[0].fixed as f64 / scaled(scale + frac)).abs();
    f64::from(frames) * step
}

#[test]
fn r1_heat_learned_runs_100_device_steps_within_the_bound_of_the_host_simulation() {
    let Some(host) =
        device("r1_heat_learned_runs_100_device_steps_within_the_bound_of_the_host_simulation")
    else {
        return;
    };
    let mut mcp = Mcp::new(Some(&host));
    let mut rng = Rng(102);
    let frac = 16;
    let flat = [64, 64, 1];
    let size = [64, 64, 2];
    for (weights, mask) in [
        (five_point(ALPHA5), ",\"places\":\"von_neumann\""),
        (nine_point(ALPHA9), ""),
    ] {
        let examples = heat_examples(&mut rng, flat, 4, &weights, frac);
        let learned = mcp.learn(&linear_args("3x3", &[0], &[1], &examples, mask));
        assert_eq!(
            written(learned.answer.get("iterate").unwrap()),
            "{\"alternate\":[0,1]}"
        );
        let count = size[0] * size[1] * size[2];
        let start = field(&mut rng, count, 0.0, 1.0, frac);
        let frames = 100;
        let on_device = iterate_alternating(&mut mcp, &learned, size, &start, frames);
        let mut host_words = vec![start.clone()];
        let mut truth = reals(&start, frac);
        for _ in 0..frames {
            host_words = learned.stepped(size, &host_words);
            truth = stencil_step(size, &truth, &weights, 0.0);
        }
        assert!(
            on_device == host_words[0],
            "device and host formula differ after {frames} frames"
        );
        let bound = heat_bound(&learned, &weights, frames);
        let worst = on_device
            .iter()
            .zip(&truth)
            .map(|(word, real)| (real_of(*word, frac) - real).abs())
            .fold(0.0, f64::max);
        eprintln!("heat 100 steps: worst {worst:e}, bound {bound:e}");
        assert!(worst <= bound, "worst {worst} past {bound}");
    }
    let seven = seven_point(ALPHA7);
    let cube = [16, 16, 16];
    let examples = heat_examples(&mut rng, cube, 4, &seven, frac);
    let learned = mcp.learn(&linear_args(
        "3x3x3",
        &[0],
        &[1],
        &examples,
        ",\"places\":\"von_neumann\"",
    ));
    let start = field(&mut rng, 16 * 16 * 16, 0.0, 1.0, frac);
    let on_device = iterate_alternating(&mut mcp, &learned, cube, &start, 100);
    let mut host_words = vec![start.clone()];
    let mut truth = reals(&start, frac);
    for _ in 0..100 {
        host_words = learned.stepped(cube, &host_words);
        truth = stencil_step(cube, &truth, &seven, 0.0);
    }
    assert!(
        on_device == host_words[0],
        "3D device and host formula differ"
    );
    let bound = heat_bound(&learned, &seven, 100);
    let worst = on_device
        .iter()
        .zip(&truth)
        .map(|(word, real)| (real_of(*word, frac) - real).abs())
        .fold(0.0, f64::max);
    eprintln!("heat 3D 100 steps: worst {worst:e}, bound {bound:e}");
    assert!(worst <= bound, "worst {worst} past {bound}");
}

const DU: f64 = 0.160_123_457;

const DV: f64 = 0.080_234_561;

const FEED: f64 = 0.035_345_671;

const KILL: f64 = 0.060_456_781;

const GS_FRAC: u32 = 14;

fn gray_scott(size: [usize; 3], u: &[f64], v: &[f64]) -> (Vec<f64>, Vec<f64>) {
    let mut next_u = vec![0.0; u.len()];
    let mut next_v = vec![0.0; v.len()];
    for at in cells(size) {
        let here = index(size, at);
        let laplace = |field: &[f64]| {
            FACES2
                .iter()
                .map(|by| field[moved(size, at, *by)])
                .sum::<f64>()
                - 4.0 * field[here]
        };
        let reaction = u[here] * v[here] * v[here];
        next_u[here] = u[here] + DU * laplace(u) - reaction + FEED * (1.0 - u[here]);
        next_v[here] = v[here] + DV * laplace(v) + reaction - (FEED + KILL) * v[here];
    }
    (next_u, next_v)
}

fn gs_examples(rng: &mut Rng, size: [usize; 3], many: usize) -> Vec<String> {
    let count = size[0] * size[1] * size[2];
    (0..many)
        .map(|_| {
            let u = field(rng, count, 0.0, 1.0, GS_FRAC);
            let v = field(rng, count, 0.0, 1.0, GS_FRAC);
            let (next_u, next_v) = gray_scott(size, &reals(&u, GS_FRAC), &reals(&v, GS_FRAC));
            inline(
                size,
                &[u, v],
                &[words_of(&next_u, GS_FRAC), words_of(&next_v, GS_FRAC)],
            )
        })
        .collect()
}

fn gs_args(examples: &[String], poly: u32) -> String {
    linear_args(
        "3x3",
        &[0, 1],
        &[2, 3],
        examples,
        &format!(",\"places\":\"von_neumann\",\"frac\":{GS_FRAC},\"poly\":{poly}"),
    )
}

fn cube_of(left: usize, right: usize, third: usize) -> impl Fn(&Kind) -> bool {
    move |kind| matches!(kind, Kind::Cube(a, b, c) if (*a, *b, *c) == (left, right, third))
}

fn gs_truth(learned: &Learned) -> Vec<Vec<(Kind, f64)>> {
    let mut out = Vec::new();
    for (write, (diffusion, decay)) in [(DU, FEED), (DV, FEED + KILL)].into_iter().enumerate() {
        let mut wanted = Vec::new();
        for kind in &learned.kinds {
            let value = match kind {
                Kind::Place(read, at) if *read == write => {
                    if *at == [0, 0, 0] {
                        1.0 - 4.0 * diffusion - decay
                    } else {
                        diffusion
                    }
                }
                Kind::Cube(0, 1, 1) => {
                    if write == 0 {
                        -1.0
                    } else {
                        1.0
                    }
                }
                _ => 0.0,
            };
            wanted.push((kind.clone(), value));
        }
        out.push(wanted);
    }
    out
}

#[test]
fn r2_gray_scott_under_poly_3_recovers_the_reaction_and_poly_2_cannot() {
    let mut mcp = Mcp::new(None);
    let mut rng = Rng(201);
    let size = [64, 64, 1];
    let examples = gs_examples(&mut rng, size, 3);
    let begun = Instant::now();
    let learned = mcp.learn(&gs_args(&examples, 3));
    eprintln!("gray-scott 64x64x3 fit: {:?}", begun.elapsed());
    assert!(learned.kinds.iter().any(cube_of(0, 1, 1)));
    for (write, wanted) in gs_truth(&learned).iter().enumerate() {
        let scale = learned.scales[write];
        for (at, (kind, value)) in wanted.iter().enumerate() {
            close_to(
                &learned.coefficients[write][at],
                scale,
                *value,
                &format!("write {write} term {kind:?}"),
            );
        }
    }
    let feed = &learned.bias[0];
    assert!(
        (feed.fit - FEED).abs() <= 6.0 * feed.stderr + 1e-9,
        "{feed:?}"
    );
    let none = &learned.bias[1];
    assert!(none.fit.abs() <= 6.0 * none.stderr + 1e-9, "{none:?}");
    for write in 0..2 {
        let rms = real_at(learned.residual(write), "rms_real");
        assert!(rms < 1.0 / scaled(GS_FRAC), "write {write} rms {rms}");
    }
    let quadratic = mcp.learn(&gs_args(&examples, 2));
    assert!(
        !quadratic
            .kinds
            .iter()
            .any(|kind| matches!(kind, Kind::Cube(..)))
    );
    let missed = real_at(quadratic.residual(0), "rms_real");
    eprintln!("gray-scott poly 2 rms {missed:e}");
    assert!(missed > 100.0 / scaled(GS_FRAC), "poly 2 rms {missed}");
}

#[test]
fn r2_gray_scott_learned_runs_100_device_steps_against_the_host() {
    let Some(host) = device("r2_gray_scott_learned_runs_100_device_steps_against_the_host") else {
        return;
    };
    let mut mcp = Mcp::new(Some(&host));
    let mut rng = Rng(202);
    let flat = [64, 64, 1];
    let examples = gs_examples(&mut rng, flat, 3);
    let learned = mcp.learn(&gs_args(&examples, 3));
    assert_eq!(
        written(learned.answer.get("iterate").unwrap().get("copy").unwrap()),
        "[[2,0],[3,1]]"
    );
    let size = [64, 64, 2];
    let count = size[0] * size[1] * size[2];
    let mut u = field(&mut rng, count, 0.0, 1.0, GS_FRAC);
    let mut v = field(&mut rng, count, 0.0, 0.5, GS_FRAC);
    let mut true_u = reals(&u, GS_FRAC);
    let mut true_v = reals(&v, GS_FRAC);
    let world = mcp.world(&learned.law, size, "");
    mcp.write(world, 0, size, &u);
    mcp.write(world, 1, size, &v);
    let frames = 100;
    for frame in 0..frames {
        mcp.step(world, 1);
        let device_u = mcp.read(world, 2, size);
        let device_v = mcp.read(world, 3, size);
        let host = learned.stepped(size, &[u.clone(), v.clone()]);
        assert!(
            device_u == host[0] && device_v == host[1],
            "frame {frame}: device and host formula differ"
        );
        u = device_u;
        v = device_v;
        mcp.write(world, 0, size, &u);
        mcp.write(world, 1, size, &v);
        let (next_u, next_v) = gray_scott(size, &true_u, &true_v);
        true_u = next_u;
        true_v = next_v;
    }
    mcp.close(world);
    let worst = u
        .iter()
        .zip(&true_u)
        .chain(v.iter().zip(&true_v))
        .map(|(word, real)| (real_of(*word, GS_FRAC) - real).abs())
        .fold(0.0, f64::max);
    eprintln!("gray-scott 100 steps: worst {worst:e}");
    let tolerance = f64::from(frames) * 0.5 / scaled(GS_FRAC);
    assert!(worst <= tolerance, "worst {worst} past {tolerance}");
}

fn noisy_examples(rng: &mut Rng, sigma: f64, frac: u32) -> Vec<String> {
    let size = [64, 64, 1];
    let weights = five_point(ALPHA5);
    (0..4)
        .map(|_| {
            let input = field(rng, 64 * 64, 0.0, 1.0, frac);
            let clean = stencil_step(size, &reals(&input, frac), &weights, 0.0);
            let noisy: Vec<f64> = clean
                .iter()
                .map(|one| one + sigma * gaussian(rng))
                .collect();
            inline(size, &[input], &[words_of(&noisy, frac)])
        })
        .collect()
}

#[test]
fn r3_noise_at_one_and_five_percent_is_reported_and_the_error_scales_with_it() {
    let mut mcp = Mcp::new(None);
    let frac = 16;
    let weights = five_point(ALPHA5);
    let mut spreads = Vec::new();
    for sigma in [0.01, 0.05] {
        let mut rng = Rng(301);
        let examples = noisy_examples(&mut rng, sigma, frac);
        let learned = mcp.learn(&linear_args(
            "3x3",
            &[0],
            &[1],
            &examples,
            ",\"places\":\"von_neumann\"",
        ));
        recovered(&learned, &weights, false, &[1, 3, 4, 5, 7]);
        let rms = real_at(learned.residual(0), "rms_real");
        eprintln!("sigma {sigma}: residual rms {rms}");
        assert!(
            (rms / sigma - 1.0).abs() < 0.03,
            "rms {rms} at sigma {sigma}"
        );
        let theory = sigma / ((64.0 * 64.0 * 4.0_f64).sqrt() * (1.0_f64 / 12.0).sqrt());
        let stderr = learned.place(0, 0, [1, 0, 0]).unwrap().stderr;
        assert!(
            (stderr / theory - 1.0).abs() < 0.1,
            "stderr {stderr} against sigma / (sqrt(n) std(x)) {theory}"
        );
        assert!(learned.invariant());
        spreads.push(stderr);
    }
    let ratio = spreads[1] / spreads[0];
    assert!((ratio - 5.0).abs() < 0.25, "stderr ratio {ratio}");
}

fn halves() -> Vec<String> {
    let size = [64, 64, 1];
    let mut rng = Rng(401);
    (0..2)
        .map(|_| {
            let input: Vec<u32> = (0..64 * 64)
                .map(|_| ((rng.below(8192) as i32) - 4096).cast_unsigned())
                .collect();
            let output: Vec<u32> = input
                .iter()
                .map(|one| (i64::from(one.cast_signed()) + 1).div_euclid(2) as i32)
                .map(i32::cast_unsigned)
                .collect();
            inline(size, &[input], &[output])
        })
        .collect()
}

#[test]
fn r4_the_fixed_point_law_on_the_device_equals_the_host_formula_and_rounds_half_up() {
    let Some(host) =
        device("r4_the_fixed_point_law_on_the_device_equals_the_host_formula_and_rounds_half_up")
    else {
        return;
    };
    let mut mcp = Mcp::new(Some(&host));
    let frac = 12;
    let learned = mcp.learn(&linear_args(
        "3x3",
        &[0],
        &[1],
        &halves(),
        &format!(",\"place_list\":[4],\"frac\":{frac}"),
    ));
    assert_eq!(int_of(learned.residual(0), "max"), 0);
    let size = [64, 64, 2];
    let input: Vec<u32> = (0_i32..64 * 64 * 2)
        .map(|at| ((at % 8192) - 4096).cast_unsigned())
        .collect();
    let world = mcp.world(&learned.law, size, "");
    mcp.write(world, 0, size, &input);
    mcp.step(world, 1);
    let got = mcp.read(world, 1, size);
    mcp.close(world);
    for (word, answer) in input.iter().zip(&got) {
        let x = i64::from(word.cast_signed());
        assert_eq!(
            i64::from(answer.cast_signed()),
            (x + 1).div_euclid(2),
            "x {x}: half up"
        );
    }
    let odd: Vec<i64> = [-7_i64, -3, -1, 1, 3, 7].to_vec();
    for x in odd {
        let at = input
            .iter()
            .position(|one| i64::from(one.cast_signed()) == x)
            .unwrap();
        let answer = i64::from(got[at].cast_signed());
        assert_ne!(answer, x.div_euclid(2), "x {x} is not truncated");
        if x < 0 {
            assert_ne!(
                answer,
                (x as f64 / 2.0).round() as i64,
                "x {x} is not rounded half away from zero"
            );
        }
    }
    let mut rng = Rng(402);
    let flat = [32, 32, 1];
    let weights: Vec<([i64; 3], f64)> = vec![
        ([0, 0, 0], 0.512_345),
        ([-1, 0, 0], 0.301_234),
        ([0, 1, 0], -0.212_345),
        ([1, 1, 0], 0.071_234),
    ];
    let examples: Vec<String> = (0..4)
        .map(|_| {
            let input = field(&mut rng, 32 * 32, -0.5, 1.0, frac);
            let real = reals(&input, frac);
            let linear = stencil_step(flat, &real, &weights, -0.053_21);
            let out: Vec<f64> = linear
                .iter()
                .zip(&real)
                .map(|(one, x)| one + 0.123_45 * x * x)
                .collect();
            inline(flat, &[input], &[words_of(&out, frac)])
        })
        .collect();
    let learned = mcp.learn(&linear_args(
        "3x3",
        &[0],
        &[1],
        &examples,
        &format!(",\"frac\":{frac},\"poly\":2,\"clamp\":[-1024,3072]"),
    ));
    assert!(!learned.invariant());
    close_to(
        learned.place(0, 0, [-1, 0, 0]).unwrap(),
        learned.scales[0],
        0.301_234,
        "west",
    );
    close_to(
        learned.place(0, 0, [1, 0, 0]).unwrap(),
        learned.scales[0],
        0.0,
        "east",
    );
    let product = learned
        .term(0, &|kind| matches!(kind, Kind::Product(0, 0)))
        .unwrap();
    close_to(product, learned.scales[0], 0.123_45, "square");
    assert!((learned.bias[0].fit + 0.053_21).abs() <= 6.0 * learned.bias[0].stderr + 1e-9);
    let input = field(&mut rng, 32 * 32 * 2, -0.5, 1.0, frac);
    let size = [32, 32, 2];
    let world = mcp.world(&learned.law, size, "");
    mcp.write(world, 0, size, &input);
    mcp.step(world, 1);
    let got = mcp.read(world, 1, size);
    mcp.close(world);
    let host_words = learned.stepped(size, &[input]);
    let clamped = got
        .iter()
        .filter(|one| [-1024, 3072].contains(&one.cast_signed()))
        .count();
    assert!(clamped > 0, "the clamp is reached");
    assert!(got == host_words[0], "device and host formula differ");
}

#[test]
fn r5_refusals_name_their_code_and_field() {
    let mut mcp = Mcp::new(None);
    let mut rng = Rng(501);
    let size = [16, 16, 1];
    let input = field(&mut rng, 256, 0.0, 1.0, 16);
    let output = words_of(
        &stencil_step(size, &reals(&input, 16), &five_point(ALPHA5), 0.0),
        16,
    );
    let one = vec![inline(
        size,
        std::slice::from_ref(&input),
        std::slice::from_ref(&output),
    )];
    let bit_one = linear_args("3x3", &[0], &[1], &one, "").replacen(
        "{\"plane\":0,\"bit\":0}",
        "{\"plane\":0,\"bit\":1}",
        1,
    );
    mcp.refused(&bit_one, "plane_format", "$.reads[0].bit");
    mcp.refused(
        &linear_args("3x3", &[0], &[1], &one, ",\"frac\":31"),
        "plane_format",
        "$.frac",
    );
    mcp.refused(
        &linear_args("3x3", &[0], &[1], &one, ",\"range\":[0,1000]"),
        "plane_format",
        "$.range",
    );
    let cube_size = [4, 4, 4];
    let cube_input = field(&mut rng, 64, 0.0, 1.0, 16);
    let cube = vec![inline(
        cube_size,
        &[cube_input.clone(), cube_input],
        &[vec![0; 64]],
    )];
    let told = mcp.refused(
        &linear_args("3x3x3", &[0, 1], &[2], &cube, ""),
        "learn_limit",
        "$.reads",
    );
    assert_eq!(int_of(&told, "limit"), 1);
    let told = mcp.refused(
        &linear_args("3x3", &[0], &[1], &one, ",\"poly\":4"),
        "poly_limit",
        "$.poly",
    );
    assert_eq!(int_of(&told, "limit"), 3);
    let twice = vec![inline(
        size,
        &[input.clone(), input.clone()],
        std::slice::from_ref(&output),
    )];
    let told = mcp.refused(
        &linear_args("3x3", &[0, 1], &[2], &twice, ",\"places\":\"von_neumann\""),
        "ill_conditioned",
        "$.examples",
    );
    let condition = text_at(&told, "condition");
    eprintln!("dependent reads: condition {condition}");
    assert!(condition == "infinite" || condition.parse::<f64>().unwrap() > 1e10);
    let near: Vec<u32> = input
        .iter()
        .map(|one| one + u32::from(rng.below(64) == 0))
        .collect();
    let close = vec![inline(
        size,
        &[input.clone(), near],
        std::slice::from_ref(&output),
    )];
    let told = mcp.refused(
        &linear_args("3x3", &[0, 1], &[2], &close, ",\"places\":\"von_neumann\""),
        "ill_conditioned",
        "$.examples",
    );
    let condition = text_at(&told, "condition");
    eprintln!("nearly dependent reads: condition {condition}");
    assert!(condition.parse::<f64>().unwrap() > 1e10);
    let flat = vec![inline(
        size,
        &[input.clone(), vec![77; 256]],
        std::slice::from_ref(&output),
    )];
    let told = mcp.refused(
        &linear_args("3x3", &[0, 1], &[2], &flat, ",\"places\":\"von_neumann\""),
        "ill_conditioned",
        "$.examples",
    );
    assert_eq!(text_at(&told, "condition"), "infinite");
    let told = mcp.refused(
        &linear_args("3x3", &[0], &[1], &one, ",\"poly\":2"),
        "fixed_range",
        "$.frac",
    );
    assert!(int_of(&told, "limit") < 16);
    mcp.refused(
        &bit_args(
            "outer",
            "wrapped",
            &counted_examples(1, 1, true, &life),
            ",\"poly\":2",
        ),
        "linear_rule",
        "$.poly",
    );
    mcp.refused(
        &linear_args("3x3", &[0], &[1], &one, ",\"law_frac\":12"),
        "steps_rule",
        "$.law_frac",
    );
    mcp.refused(
        &linear_args("3x3", &[0], &[1], &one, ",\"noise\":\"vote\""),
        "noise_rule",
        "$.noise",
    );
    mcp.refused(
        &linear_args("3x3", &[0], &[1], &one, ",\"clamp\":[5,1]"),
        "reversed",
        "$.clamp",
    );
    mcp.refused(
        &linear_args("3x3", &[0], &[0], &one, ""),
        "halo_written",
        "$.reads[0].plane",
    );
}

#[test]
fn timing_gray_scott_256_fit_and_step_on_the_device() {
    let Some(host) = device("timing_gray_scott_256_fit_and_step_on_the_device") else {
        return;
    };
    let mut mcp = Mcp::new(Some(&host));
    let mut rng = Rng(601);
    let size = [256, 256, 2];
    let flat = [256, 256, 1];
    let keep = "{\"law\":1,\"name\":\"keep4\",\"params\":0,\"channels\":4,\"halo\":0,\"tally\":{\"counts\":1,\"weighted\":false},\"consts\":{},\"defs\":{},\"step\":[\"vec\",[\"map\",\"c\",[\"range\",8],[\"at\",\"cells\",\"c\"]],[\"vec\",0]]}";
    let examples = mcp.world(keep, size, "");
    let count = size[0] * size[1] * size[2];
    let u = field(&mut rng, count, 0.0, 1.0, GS_FRAC);
    let v = field(&mut rng, count, 0.0, 1.0, GS_FRAC);
    let (next_u, next_v) = gray_scott(size, &reals(&u, GS_FRAC), &reals(&v, GS_FRAC));
    for (plane, words) in [
        (0, u.clone()),
        (1, v.clone()),
        (2, words_of(&next_u, GS_FRAC)),
        (3, words_of(&next_v, GS_FRAC)),
    ] {
        mcp.write(examples, plane, size, &words);
    }
    let arguments = format!(
        "{{\"stencil\":\"3x3\",\"reads\":{},\"writes\":{},\"border\":\"wrapped\",\"rule\":\"linear\",\"examples\":[{{\"world\":{examples},\"from\":[0,0,0],\"to\":[255,255,0]}}],\"places\":\"von_neumann\",\"frac\":{GS_FRAC},\"poly\":3,\"budget_ms\":{LONG}}}",
        spots(&[0, 1]),
        spots(&[2, 3])
    );
    let mut fits = Vec::new();
    let mut learned = None;
    for _ in 0..3 {
        let begun = Instant::now();
        let got = mcp.learn(&arguments);
        fits.push(begun.elapsed());
        learned = Some(got);
    }
    let learned = learned.unwrap();
    assert_eq!(
        int_of(learned.answer.get("examples").unwrap(), "cells"),
        65536
    );
    for (write, wanted) in gs_truth(&learned).iter().enumerate() {
        for (at, (kind, value)) in wanted.iter().enumerate() {
            close_to(
                &learned.coefficients[write][at],
                learned.scales[write],
                *value,
                &format!("256: write {write} term {kind:?}"),
            );
        }
    }
    let world = mcp.world(&learned.law, size, "");
    mcp.write(world, 0, size, &u);
    mcp.write(world, 1, size, &v);
    mcp.step(world, 1);
    let host_words = learned.stepped(flat, &[u[..65536].to_vec(), v[..65536].to_vec()]);
    let device_u = mcp.read(world, 2, flat);
    assert!(
        device_u == host_words[0],
        "256: device and host formula differ"
    );
    let mut steps = Vec::new();
    for _ in 0..3 {
        let begun = Instant::now();
        mcp.step(world, 100);
        steps.push(begun.elapsed() / 100);
    }
    mcp.close(world);
    mcp.close(examples);
    fits.sort();
    steps.sort();
    eprintln!(
        "TIMING gray-scott 256x256 (two planes, 21 terms): fit min {:?} median {:?}; step (256x256x2 world, 100 frames a call) min {:?} median {:?}",
        fits[0], fits[1], steps[0], steps[1]
    );
}

#[test]
fn r5_describe_names_the_linear_limits_and_codes() {
    let mut mcp = Mcp::new(None);
    let told = mcp.ok("grit_describe", "{}");
    let limits = told.get("limits").unwrap();
    assert_eq!(limits.counted("law_linear_poly_most"), Some(3));
    assert_eq!(limits.counted("law_linear_frac_most"), Some(30));
    assert_eq!(text_at(limits, "law_linear_condition_most"), "1e10");
    assert!(text_at(limits, "law_linear_rounding").contains("ties toward +infinity"));
    let codes: Vec<String> = told
        .get("codes")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .map(|one| text_at(one, "code"))
        .collect();
    for code in [
        "plane_format",
        "poly_limit",
        "ill_conditioned",
        "fixed_range",
        "linear_rule",
    ] {
        assert!(codes.iter().any(|one| one == code), "{code}");
    }
}

const SKEW: f64 = 0.003_456_789;

fn skewed(alpha: f64) -> Vec<([i64; 3], f64)> {
    vec![
        ([0, 0, 0], 1.0 - 4.0 * alpha),
        ([1, 0, 0], alpha - SKEW),
        ([-1, 0, 0], alpha + SKEW),
        ([0, 1, 0], alpha),
        ([0, -1, 0], alpha),
    ]
}

fn noisy_skewed(rng: &mut Rng, sigma: f64, frac: u32) -> Vec<String> {
    let size = [64, 64, 1];
    let weights = skewed(ALPHA5);
    (0..4)
        .map(|_| {
            let input = field(rng, 64 * 64, 0.0, 1.0, frac);
            let clean = stencil_step(size, &reals(&input, frac), &weights, 0.0);
            let noisy: Vec<f64> = clean
                .iter()
                .map(|one| one + sigma * gaussian(rng))
                .collect();
            inline(size, &[input], &[words_of(&noisy, frac)])
        })
        .collect()
}

#[test]
fn r7_an_asymmetric_law_under_one_percent_noise_is_reported_not_symmetric() {
    let mut mcp = Mcp::new(None);
    let frac = 16;
    let sigma = 0.01;
    let mut rng = Rng(701);
    let examples = noisy_skewed(&mut rng, sigma, frac);
    let learned = mcp.learn(&linear_args(
        "3x3",
        &[0],
        &[1],
        &examples,
        ",\"places\":\"von_neumann\"",
    ));
    recovered(&learned, &skewed(ALPHA5), false, &[1, 3, 4, 5, 7]);
    let rms = real_at(learned.residual(0), "rms_real");
    assert!(
        (rms / sigma - 1.0).abs() < 0.03,
        "rms {rms} at sigma {sigma}"
    );
    let symmetry = learned.answer.get("symmetry").unwrap();
    assert!(!learned.invariant(), "{}", written(symmetry));
    let one = &symmetry.get("writes").and_then(Json::list).unwrap()[0];
    assert_eq!(one.get("invariant"), Some(&Json::Bool(false)));
    let places: Vec<i64> = one
        .get("places")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .map(|place| place.int().unwrap())
        .collect();
    let mut sorted = places.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, [3, 5], "west and east: {}", written(one));
    let deviation = real_at(one, "deviation");
    let tolerance = real_at(one, "tolerance");
    let west = learned.place(0, 0, [-1, 0, 0]).unwrap();
    let east = learned.place(0, 0, [1, 0, 0]).unwrap();
    assert!(
        (deviation - (west.fit - east.fit).abs()).abs() < 1e-9,
        "{deviation} {} {}",
        west.fit,
        east.fit
    );
    assert!(
        (deviation - 2.0 * SKEW).abs() <= 6.0 * (west.stderr + east.stderr),
        "the deviation is the skew: {deviation}"
    );
    let floor = 1.0 / scaled(learned.scales[0]);
    assert!(tolerance > 4.0 * floor, "{tolerance} {floor}");
    let pair = (west.stderr.powi(2) + east.stderr.powi(2)).sqrt();
    assert!(
        tolerance > 4.0 * pair && tolerance < 8.0 * pair,
        "about 6 standard errors of the difference: {tolerance}, {pair}"
    );
    let ratio = deviation / tolerance;
    eprintln!(
        "r7 skew {SKEW}: deviation {deviation:.4e}, tolerance {tolerance:.4e}, ratio {ratio:.3}"
    );
    assert!(
        ratio > 2.0 && ratio < 5.0,
        "past the tolerance and inside ten times it: {ratio}"
    );
    let mut rng = Rng(701);
    let level = noisy_examples(&mut rng, sigma, frac);
    let even = mcp.learn(&linear_args(
        "3x3",
        &[0],
        &[1],
        &level,
        ",\"places\":\"von_neumann\"",
    ));
    assert!(even.invariant(), "the same noise without skew is symmetric");
}

#[test]
fn r8_a_clamp_at_the_edge_of_the_written_words_holds_on_the_device() {
    let Some(host) = device("r8_a_clamp_at_the_edge_of_the_written_words_holds_on_the_device")
    else {
        return;
    };
    let mut mcp = Mcp::new(Some(&host));
    let mut rng = Rng(802);
    let frac = 16;
    let examples = heat_examples(&mut rng, [64, 64, 1], 2, &five_point(ALPHA5), frac);
    let places = ",\"places\":\"von_neumann\"";
    let open = mcp.learn(&linear_args("3x3", &[0], &[1], &examples, places));
    let [low, high] = reach(open.scales[0]);
    let (int_low, int_high) = (i64::from(i32::MIN), i64::from(i32::MAX));
    let size = [32, 32, 2];
    let input = field(&mut rng, 32 * 32 * 2, 0.0, 1.0, frac);
    let mut held = Vec::new();
    for clamp in [
        [high, int_high],
        [int_low, low],
        [low, low],
        [int_low, int_high],
        [3000, 40_000],
    ] {
        let learned = mcp.learn(&linear_args(
            "3x3",
            &[0],
            &[1],
            &examples,
            &clamped(places, clamp),
        ));
        let world = mcp.world(&learned.law, size, "");
        mcp.write(world, 0, size, &input);
        mcp.step(world, 1);
        let got = mcp.read(world, 1, size);
        mcp.close(world);
        let host_words = learned.stepped(size, std::slice::from_ref(&input));
        let free = open.stepped(size, std::slice::from_ref(&input));
        assert!(
            got == host_words[0],
            "{clamp:?}: device and host formula differ"
        );
        for (word, unclamped) in got.iter().zip(&free[0]) {
            let word = i64::from(word.cast_signed());
            let unclamped = i64::from(unclamped.cast_signed());
            assert!((low..=high).contains(&unclamped), "{unclamped}");
            assert_eq!(word, unclamped.clamp(clamp[0], clamp[1]), "{clamp:?}");
        }
        held.push(
            got.iter()
                .filter(|one| clamp.contains(&i64::from(one.cast_signed())))
                .count(),
        );
    }
    assert_eq!(held[0], 32 * 32 * 2, "every word is the upper edge");
    assert_eq!(held[1], 32 * 32 * 2, "every word is the lower edge");
    assert_eq!(held[2], 32 * 32 * 2);
    assert!(held[4] > 0, "{held:?}");
    eprintln!("r8 device: written words in [{low}, {high}], words at a clamp bound {held:?}");
}

fn two_writes(rng: &mut Rng, frac: u32) -> Vec<String> {
    let size = [64, 64, 1];
    let heat = five_point(ALPHA5);
    let small = vec![([0, 0, 0], 0.010_123_4), ([1, 0, 0], -0.004_321)];
    (0..3)
        .map(|_| {
            let input = field(rng, 64 * 64, 0.0, 1.0, frac);
            let real = reals(&input, frac);
            let first = words_of(&stencil_step(size, &real, &heat, 0.0), frac);
            let second = words_of(&stencil_step(size, &real, &small, 0.0), frac);
            inline(size, &[input], &[first, second])
        })
        .collect()
}

fn reach(scale: u32) -> [i64; 2] {
    let half = 1_i64 << (31 - scale);
    [-half, half - 1]
}

fn limit_of(told: &Json) -> [i64; 2] {
    let held = told.get("limit").and_then(Json::list).unwrap();
    [held[0].int().unwrap(), held[1].int().unwrap()]
}

fn clamped(more: &str, clamp: [i64; 2]) -> String {
    format!("{more},\"clamp\":[{},{}]", clamp[0], clamp[1])
}

#[test]
fn r8_a_clamp_that_meets_no_word_the_law_writes_is_refused_by_name() {
    let mut mcp = Mcp::new(None);
    let mut rng = Rng(801);
    let frac = 16;
    let examples = heat_examples(&mut rng, [64, 64, 1], 2, &five_point(ALPHA5), frac);
    let places = ",\"places\":\"von_neumann\"";
    let args = |clamp: [i64; 2]| linear_args("3x3", &[0], &[1], &examples, &clamped(places, clamp));
    let open = mcp.learn(&linear_args("3x3", &[0], &[1], &examples, places));
    let scale = open.scales[0];
    let [low, high] = reach(scale);
    eprintln!("r8 coefficient_frac {scale}: written words in [{low}, {high}]");
    let (int_low, int_high) = (i64::from(i32::MIN), i64::from(i32::MAX));
    for clamp in [
        [high + 1, int_high],
        [high + 1, high + 1],
        [int_low, low - 1],
        [low - 1, low - 1],
        [high + 1000, high + 2000],
    ] {
        let told = mcp.refused(&args(clamp), "clamp_range", "$.clamp");
        assert_eq!(limit_of(&told), [low, high], "{clamp:?}");
        assert!(
            text_at(&told, "message").contains(&format!("[{low}, {high}]")),
            "{}",
            written(&told)
        );
        assert!(text_at(&told, "hint").ends_with('.'));
    }
    for clamp in [
        [high, int_high],
        [high, high],
        [int_low, low],
        [low, low],
        [int_low, int_high],
        [low - 1, high + 1],
        [-1024, 3072],
    ] {
        let learned = mcp.learn(&args(clamp));
        assert_eq!(learned.scales, open.scales, "{clamp:?}: the clamp leaves k");
        assert_eq!(learned.clamp, Some(clamp));
    }
    let examples = two_writes(&mut rng, frac);
    let open = mcp.learn(&linear_args("3x3", &[0], &[1, 2], &examples, places));
    let [wide, narrow] = [open.scales[0], open.scales[1]];
    assert!(narrow > wide, "{:?}", open.scales);
    let [low, high] = reach(narrow);
    let [_, wide_high] = reach(wide);
    for (clamp, refused) in [
        ([high + 1, wide_high], true),
        ([int_low, low - 1], true),
        ([high, wide_high], false),
    ] {
        let more = clamped(places, clamp);
        let request = linear_args("3x3", &[0], &[1, 2], &examples, &more);
        if refused {
            let told = mcp.refused(&request, "clamp_range", "$.clamp");
            assert_eq!(limit_of(&told), [low, high], "the narrowest of the writes");
        } else {
            assert_eq!(mcp.learn(&request).scales, open.scales);
        }
    }
    eprintln!(
        "r8 two writes, coefficient_frac {wide} and {narrow}: a clamp past [{low}, {high}] refused"
    );
}

#[test]
fn r9_ill_conditioned_is_named_before_fixed_range_whatever_the_order() {
    let mut mcp = Mcp::new(None);
    let mut rng = Rng(901);
    let size = [16, 16, 1];
    let input = field(&mut rng, 256, 0.0, 1.0, 16);
    let other = field(&mut rng, 256, 0.0, 1.0, 16);
    let output = words_of(
        &stencil_step(size, &reals(&input, 16), &five_point(ALPHA5), 0.0),
        16,
    );
    let near: Vec<u32> = input
        .iter()
        .map(|one| one + u32::from(rng.below(64) == 0))
        .collect();
    let pair = |second: &[u32], swapped: bool| {
        let reads = if swapped {
            vec![second.to_vec(), input.clone()]
        } else {
            vec![input.clone(), second.to_vec()]
        };
        vec![inline(size, &reads, std::slice::from_ref(&output))]
    };
    let places = ",\"places\":\"von_neumann\"";
    let lead = |examples: &[String], more: &str| {
        let after = linear_args("3x3", &[0, 1], &[2], examples, more);
        let before = after
            .replacen(more, "", 1)
            .replacen("{", &format!("{{{},", &more[1..]), 1);
        assert_ne!(after, before);
        [after, before]
    };
    for swapped in [false, true] {
        let free = pair(&other, swapped);
        for request in lead(&free, &format!("{places},\"poly\":2")) {
            mcp.refused(&request, "fixed_range", "$.frac");
        }
        for (label, second) in [
            ("identical", input.clone()),
            ("near", near.clone()),
            ("flat", vec![77; 256]),
        ] {
            let examples = pair(&second, swapped);
            for request in lead(&examples, places) {
                mcp.refused(&request, "ill_conditioned", "$.examples");
            }
            for request in lead(&examples, &format!("{places},\"poly\":2"))
                .into_iter()
                .chain(lead(&examples, &format!(",\"poly\":2{places}")))
            {
                let told = mcp.refused(&request, "ill_conditioned", "$.examples");
                assert!(told.get("condition").is_some(), "{label}");
                assert!(told.get("limit").and_then(Json::text).is_some(), "{label}");
            }
        }
    }
}
