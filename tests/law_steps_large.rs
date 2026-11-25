#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use grit::Host;
use grit::serve3::Server;
use grit::stencil::{Border, Counting, Sample, Stencil};
use grit::steps::Stepping;
use grit::steps::least::{self, Learned};
use grit::wire::{Json, written};

const LONG: u64 = 600_000;

const BASE_SHA: [(&str, &str); 5] = [
    (
        "vn3_outer_k2",
        "0ff8d34be666ec628ccdaa64d9e12c1d9a13d376872bc7b0e710c0c6d3e47d80",
    ),
    (
        "vn3_total_k3_zero",
        "cafd00ddbaeb1cb551840d52b60aae6e383e75b9c5994685cf327445ef104bf9",
    ),
    (
        "life_k2",
        "375e11e7e09782891263ab4a63fb3d564d99d1412224c9c876185020ad87517e",
    ),
    (
        "noisy_k2_refused",
        "c69a1ee8525527f64e262f20b0070520ae084a859fecc32e8af327ba54ca1569",
    ),
    (
        "hex_total_k4",
        "2abad47f428df0ae6130852b66b6a2c8577c01f7d6b9d6a3ace6c92223a59331",
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

    fn below(&mut self, most: u64) -> u64 {
        self.next() % most
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tally {
    Totalistic,
    Outer,
}

#[derive(Clone, Debug)]
struct Rule {
    tally: Tally,
    cube: bool,
    places: Vec<u32>,
    code: u64,
}

impl Rule {
    fn centre(&self) -> u32 {
        if self.cube { 13 } else { 4 }
    }

    fn counted(&self) -> Vec<u32> {
        self.places
            .iter()
            .copied()
            .filter(|place| self.tally == Tally::Totalistic || *place != self.centre())
            .collect()
    }

    fn counts(&self) -> u32 {
        self.counted().len() as u32 + 1
    }

    fn classes(&self) -> u32 {
        match self.tally {
            Tally::Totalistic => self.counts(),
            Tally::Outer => 2 * self.counts(),
        }
    }

    fn of(tally: Tally, cube: bool, places: &[u32], born: &[u32], kept: &[u32]) -> Self {
        let mut rule = Self {
            tally,
            cube,
            places: places.to_vec(),
            code: 0,
        };
        let counts = rule.counts();
        rule.code = born.iter().map(|n| 1_u64 << n).sum::<u64>()
            + kept.iter().map(|n| 1_u64 << (counts + n)).sum::<u64>();
        rule
    }

    fn with(&self, code: u64) -> Self {
        Self {
            code,
            ..self.clone()
        }
    }

    fn name(&self) -> &'static str {
        match self.tally {
            Tally::Totalistic => "totalistic",
            Tally::Outer => "outer",
        }
    }

    fn offsets(&self) -> Vec<[i64; 3]> {
        self.counted()
            .iter()
            .map(|place| {
                let p = i64::from(*place);
                [
                    p % 3 - 1,
                    p / 3 % 3 - 1,
                    if self.cube { p / 9 - 1 } else { 0 },
                ]
            })
            .collect()
    }
}

fn moore(cube: bool) -> Vec<u32> {
    (0..if cube { 27 } else { 9 }).collect()
}

fn von_neumann(cube: bool) -> Vec<u32> {
    if cube {
        vec![4, 10, 12, 13, 14, 16, 22]
    } else {
        vec![1, 3, 4, 5, 7]
    }
}

#[derive(Clone, Copy, Debug)]
struct Size([usize; 3]);

impl Size {
    const fn cells(self) -> usize {
        self.0[0] * self.0[1] * self.0[2]
    }

    fn at(self, at: [i64; 3], wrapped: bool) -> Option<usize> {
        let mut spot = [0_usize; 3];
        for axis in 0..3 {
            let side = i64::try_from(self.0[axis]).unwrap();
            let one = if wrapped {
                at[axis].rem_euclid(side)
            } else {
                at[axis]
            };
            if !(0..side).contains(&one) {
                return None;
            }
            spot[axis] = usize::try_from(one).unwrap();
        }
        Some((spot[2] * self.0[1] + spot[1]) * self.0[0] + spot[0])
    }
}

fn class_at(
    size: Size,
    cells: &[u8],
    wrapped: bool,
    rule: &Rule,
    offsets: &[[i64; 3]],
    cell: usize,
) -> u32 {
    let [wide, deep, _] = size.0;
    let at = [
        (cell % wide) as i64,
        (cell / wide % deep) as i64,
        (cell / wide / deep) as i64,
    ];
    let mut n = 0_u32;
    for offset in offsets {
        if let Some(spot) = size.at(
            [at[0] + offset[0], at[1] + offset[1], at[2] + offset[2]],
            wrapped,
        ) {
            n += u32::from(cells[spot]);
        }
    }
    match rule.tally {
        Tally::Totalistic => n,
        Tally::Outer => u32::from(cells[cell]) * rule.counts() + n,
    }
}

fn stepped(size: Size, cells: &[u8], wrapped: bool, rule: &Rule) -> Vec<u8> {
    let offsets = rule.offsets();
    (0..size.cells())
        .map(|cell| {
            u8::from((rule.code >> class_at(size, cells, wrapped, rule, &offsets, cell)) & 1 == 1)
        })
        .collect()
}

fn applied(size: Size, cells: &[u8], wrapped: bool, rule: &Rule, steps: u32) -> Vec<u8> {
    let mut held = cells.to_vec();
    for _ in 0..steps {
        held = stepped(size, &held, wrapped, rule);
    }
    held
}

fn soup(rng: &mut Rng, size: Size, permille: u64) -> Vec<u8> {
    (0..size.cells())
        .map(|_| u8::from(rng.below(1000) < permille))
        .collect()
}

fn flipped(rng: &mut Rng, cells: &[u8], per_million: u64) -> Vec<u8> {
    cells
        .iter()
        .map(|one| one ^ u8::from(rng.below(1_000_000) < per_million))
        .collect()
}

#[derive(Clone)]
struct Pair {
    size: Size,
    input: Vec<u8>,
    output: Vec<u8>,
}

fn pairs_of(
    seed: u64,
    densities: &[u64],
    size: Size,
    rule: &Rule,
    (steps, wrapped, per_million): (u32, bool, u64),
) -> Vec<Pair> {
    let mut rng = Rng(seed);
    densities
        .iter()
        .map(|permille| {
            let clean = soup(&mut rng, size, *permille);
            let after = applied(size, &clean, wrapped, rule, steps);
            Pair {
                size,
                input: flipped(&mut rng, &clean, per_million),
                output: flipped(&mut rng, &after, per_million),
            }
        })
        .collect()
}

fn listed(cells: &[u8]) -> String {
    let mut out = String::with_capacity(cells.len() * 2 + 2);
    out.push('[');
    for (at, one) in cells.iter().enumerate() {
        if at > 0 {
            out.push(',');
        }
        out.push(if *one == 0 { '0' } else { '1' });
    }
    out.push(']');
    out
}

fn examples_of(pairs: &[Pair]) -> String {
    let shown: Vec<String> = pairs
        .iter()
        .map(|one| {
            let [wide, deep, tall] = one.size.0;
            format!(
                "{{\"size\":[{wide},{deep},{tall}],\"input\":[{}],\"output\":[{}]}}",
                listed(&one.input),
                listed(&one.output)
            )
        })
        .collect();
    format!("[{}]", shown.join(","))
}

fn places_of(rule: &Rule) -> String {
    let listed: Vec<String> = rule.places.iter().map(u32::to_string).collect();
    format!("[{}]", listed.join(","))
}

fn learn_args(rule: &Rule, border: &str, pairs: &[Pair], steps: u32, more: &str) -> String {
    format!(
        "{{\"stencil\":\"{}\",\"reads\":[{{\"plane\":0,\"bit\":0}}],\"writes\":[{{\"plane\":1,\"bit\":0}}],\"border\":\"{border}\",\"rule\":\"{}\",\"place_list\":{},\"steps\":{steps},\"examples\":{},\"budget_ms\":{LONG}{more}}}",
        if rule.cube { "3x3x3" } else { "3x3" },
        rule.name(),
        places_of(rule),
        examples_of(pairs)
    )
}

fn line(name: &str, arguments: &str) -> String {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{{\"name\":\"{name}\",\"arguments\":{arguments}}}}}"
    )
}

fn heard(server: &mut Server<'_>, arguments: &str) -> Json {
    server.heard(&line("grit_law_learn", arguments)).unwrap()
}

fn content(got: &Json) -> (bool, Json) {
    let held = got
        .get("result")
        .unwrap_or_else(|| panic!("{}", written(got)));
    (
        held.get("isError") == Some(&Json::Bool(true)),
        held.get("structuredContent").cloned().unwrap(),
    )
}

fn sha_of(held: &Json) -> String {
    grit::law::hex(&grit::branch::sha256(written(held).as_bytes()))
}

fn base_requests() -> Vec<(&'static str, String)> {
    let vn3 = Rule::of(Tally::Outer, true, &von_neumann(true), &[1, 2], &[1, 2, 3]);
    let vn3_pairs = pairs_of(31_001, &[150, 300], Size([16, 16, 16]), &vn3, (2, true, 0));
    let total3 = Rule::of(Tally::Totalistic, true, &von_neumann(true), &[2, 3, 4], &[]);
    let total3_pairs = pairs_of(
        31_002,
        &[200, 350, 120],
        Size([12, 12, 12]),
        &total3,
        (3, false, 0),
    );
    let life = Rule::of(Tally::Outer, false, &moore(false), &[3], &[2, 3]);
    let life_pairs = pairs_of(
        31_003,
        &[350, 350, 200, 450],
        Size([32, 32, 1]),
        &life,
        (2, true, 0),
    );
    let noisy_pairs = pairs_of(
        31_004,
        &[350, 350, 300],
        Size([32, 32, 1]),
        &life,
        (2, true, 5_000),
    );
    let hex = Rule::of(
        Tally::Totalistic,
        false,
        &[1, 2, 3, 4, 5, 6, 7],
        &[2, 3],
        &[],
    );
    let hex_pairs = pairs_of(
        31_005,
        &[300, 400, 250, 350],
        Size([32, 32, 1]),
        &hex,
        (4, true, 0),
    );
    vec![
        (
            "vn3_outer_k2",
            learn_args(&vn3, "wrapped", &vn3_pairs, 2, ""),
        ),
        (
            "vn3_total_k3_zero",
            learn_args(&total3, "zero", &total3_pairs, 3, ""),
        ),
        ("life_k2", learn_args(&life, "wrapped", &life_pairs, 2, "")),
        (
            "noisy_k2_refused",
            learn_args(&life, "wrapped", &noisy_pairs, 2, ""),
        ),
        (
            "hex_total_k4",
            learn_args(&hex, "wrapped", &hex_pairs, 4, ""),
        ),
    ]
}

#[test]
fn k4_requests_that_worked_before_give_the_recorded_bytes() {
    let mut server = quiet(None);
    let mut got = Vec::new();
    for (name, arguments) in base_requests() {
        let (refused, held) = content(&heard(&mut server, &arguments));
        assert_eq!(refused, name.ends_with("refused"), "{name}");
        got.push((name, sha_of(&held)));
    }
    let want: Vec<(&str, String)> = BASE_SHA
        .iter()
        .map(|(name, sha)| (*name, (*sha).to_owned()))
        .collect();
    assert_eq!(got, want);
}

fn counting_of(rule: &Rule) -> Counting {
    let stencil = if rule.cube {
        Stencil::Cube
    } else {
        Stencil::Square
    };
    let tally = match rule.tally {
        Tally::Totalistic => grit::stencil::Tally::Totalistic,
        Tally::Outer => grit::stencil::Tally::Outer,
    };
    Counting {
        tally,
        stencil,
        mask: rule.places.iter().map(|place| 1_u32 << place).sum(),
    }
}

struct Words {
    inputs: Vec<Vec<u32>>,
    outputs: Vec<Vec<u32>>,
    sizes: Vec<[u32; 3]>,
}

fn words_of(pairs: &[Pair]) -> Words {
    Words {
        inputs: pairs
            .iter()
            .map(|one| one.input.iter().map(|bit| u32::from(*bit)).collect())
            .collect(),
        outputs: pairs
            .iter()
            .map(|one| one.output.iter().map(|bit| u32::from(*bit)).collect())
            .collect(),
        sizes: pairs
            .iter()
            .map(|one| one.size.0.map(|side| u32::try_from(side).unwrap()))
            .collect(),
    }
}

fn samples_of(words: &Words) -> Vec<Sample<'_>> {
    words
        .inputs
        .iter()
        .zip(&words.outputs)
        .zip(&words.sizes)
        .map(|((input, output), size)| Sample {
            size: *size,
            inputs: vec![input.as_slice()],
            from: [0; 3],
            inner: *size,
            outputs: vec![output.as_slice()],
        })
        .collect()
}

fn stepping_of(rule: &Rule, steps: u32) -> Stepping {
    Stepping {
        counting: counting_of(rule),
        steps,
        border: Border::Wrapped,
        read: 0,
        write: 0,
    }
}

fn wrong_of(rule: &Rule, pairs: &[Pair], steps: u32) -> u64 {
    pairs
        .iter()
        .map(|one| {
            applied(one.size, &one.input, true, rule, steps)
                .iter()
                .zip(&one.output)
                .filter(|(a, b)| a != b)
                .count() as u64
        })
        .sum()
}

fn reached_of(rule: &Rule, pairs: &[Pair], steps: u32) -> Vec<bool> {
    let offsets = rule.offsets();
    let mut reached = vec![false; rule.classes() as usize];
    for one in pairs {
        let mut held = one.input.clone();
        for _ in 0..steps {
            for cell in 0..one.size.cells() {
                reached[class_at(one.size, &held, true, rule, &offsets, cell) as usize] = true;
            }
            held = stepped(one.size, &held, true, rule);
        }
    }
    reached
}

fn local_learned(rule: &Rule, pairs: &[Pair], steps: u32, local: bool) -> Learned {
    let words = words_of(pairs);
    let samples = samples_of(&words);
    let stepping = stepping_of(rule, steps);
    let found = if local {
        least::local(&stepping, &samples, true, &mut || false)
    } else {
        least::exhaustive(&stepping, &samples, true, &mut || false)
    };
    found.unwrap().unwrap()
}

fn against(rule: &Rule, code: u64, margins: &[u64]) -> Vec<(u32, u64)> {
    (0..rule.classes())
        .filter(|class| ((code ^ rule.code) >> class) & 1 == 1)
        .map(|class| (class, margins[class as usize]))
        .filter(|(_, margin)| *margin > 0)
        .collect()
}

fn wrongly_pinned(rule: &Rule, got: &Learned) -> Vec<(u32, u64)> {
    let doubt = got.doubt.as_ref().unwrap();
    let held: Vec<u64> = doubt
        .margins
        .iter()
        .zip(&doubt.pinned)
        .map(|(margin, pinned)| if *pinned { (*margin).max(1) } else { 0 })
        .collect();
    against(rule, got.code, &held)
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

fn answered(server: &mut Server<'_>, arguments: &str) -> Json {
    let (refused, held) = content(&heard(server, arguments));
    let shown = written(&held);
    assert!(!refused, "{}", &shown[..shown.len().min(3000)]);
    held
}

fn finished(label: &str, got: &Json) {
    let stopped = got.get("stopped").and_then(Json::text).unwrap_or("absent");
    let hint = got.get("hint").and_then(Json::text).unwrap_or("");
    assert!(
        stopped == "complete" && got.get("complete") == Some(&Json::Bool(true)),
        "{label}: the answer stopped {stopped}, so it claims no table: {hint}"
    );
}

#[cfg(feature = "faults")]
fn quiet(host: Option<&Host>) -> Server<'_> {
    let time: std::sync::Arc<dyn grit::clock::Time> = std::sync::Arc::new(grit::clock::Fake::new());
    Server::timed(host, &grit::serve3::Options::default(), time).unwrap()
}

#[cfg(not(feature = "faults"))]
fn quiet(host: Option<&Host>) -> Server<'_> {
    Server::new(host)
}

fn reference_only(name: &str) -> bool {
    if std::env::var_os("CI").is_none() {
        return false;
    }
    common::skip(format_args!(
        "{name} is sized for the reference machine's host CPU, skipped under CI; its small case runs at every width"
    ));
    true
}

fn refused_as(server: &mut Server<'_>, arguments: &str, code: &str, field: &str) -> Json {
    let (refused, held) = content(&heard(server, arguments));
    assert!(
        refused,
        "{}",
        &written(&held)[..written(&held).len().min(3000)]
    );
    assert_eq!(
        (text_at(&held, "refused"), text_at(&held, "field")),
        (code.to_owned(), field.to_owned()),
        "{}",
        written(&held)
    );
    held
}

fn flat_of(held: &Json) -> Vec<Json> {
    let write = &held.list().unwrap()[0];
    match write.list().unwrap().first() {
        Some(Json::List(_)) => write
            .list()
            .unwrap()
            .iter()
            .flat_map(|half| half.list().unwrap().iter().cloned())
            .collect(),
        _ => write.list().unwrap().to_vec(),
    }
}

fn code_of(tables: &Json) -> u64 {
    flat_of(tables)
        .iter()
        .enumerate()
        .map(|(class, bit)| u64::from(bit.int() == Some(1)) << class)
        .sum()
}

fn entries_of(rule: &Rule, held: &Json) -> Vec<u32> {
    held.list()
        .unwrap()
        .iter()
        .map(|one| {
            let count = u32::try_from(int_at(one, "count")).unwrap();
            let centre = one.get("centre").and_then(Json::int).unwrap_or(0);
            u32::try_from(centre).unwrap() * rule.counts() + count
        })
        .collect()
}

#[test]
fn k5_what_is_still_out_of_reach_is_refused_by_name() {
    let mut server = quiet(None);
    let life = Rule::of(Tally::Outer, false, &moore(false), &[3], &[2, 3]);
    let pairs = pairs_of(35_001, &[350, 300], Size([16, 16, 1]), &life, (2, true, 0));
    let swapped = |rule: &str| {
        learn_args(&life, "wrapped", &pairs, 2, "").replace(
            "\"rule\":\"outer\",\"place_list\":[0,1,2,3,4,5,6,7,8],\"steps\":2,",
            &format!("\"rule\":\"{rule}\","),
        )
    };
    refused_as(
        &mut server,
        &swapped("minimal").replace("\"budget_ms\"", "\"search\":\"local\",\"budget_ms\""),
        "search_rule",
        "$.search",
    );
    refused_as(
        &mut server,
        &swapped("symmetric").replace("\"budget_ms\"", "\"noise\":\"least_wrong\",\"budget_ms\""),
        "noise_rule",
        "$.noise",
    );
    refused_as(
        &mut server,
        &swapped("minimal").replace("\"budget_ms\"", "\"noise\":\"likely\",\"budget_ms\""),
        "noise_rule",
        "$.noise",
    );
    refused_as(
        &mut server,
        &learn_args(&life, "wrapped", &pairs, 2, ",\"noise\":\"vote\""),
        "steps_noise",
        "$.noise",
    );
    refused_as(
        &mut server,
        &learn_args(
            &life,
            "wrapped",
            &pairs,
            1,
            ",\"noise\":\"vote\",\"search\":\"local\"",
        ),
        "steps_noise",
        "$.noise",
    );
    let shown = written(&refused_as(
        &mut server,
        &learn_args(&life, "wrapped", &pairs, 2, ",\"search\":\"deep\""),
        "malformed",
        "$.search",
    ));
    assert!(shown.contains("search"), "{shown}");
    let cube = Rule::of(Tally::Outer, true, &moore(true), &[5, 6], &[4, 5, 6]);
    let cubes = pairs_of(35_002, &[150], Size([8, 8, 8]), &cube, (2, true, 0));
    let told = refused_as(
        &mut server,
        &learn_args(&cube, "wrapped", &cubes, 2, ",\"search\":\"exhaustive\""),
        "steps_space",
        "$.steps",
    );
    assert_eq!(int_at(&told, "classes"), 54);
    refused_as(
        &mut server,
        &learn_args(&life, "wrapped", &pairs, 2, ",\"noise\":\"least_wrong\"").replace(
            "\"writes\":[{\"plane\":1,\"bit\":0}]",
            "\"writes\":[{\"plane\":0,\"bit\":0}]",
        ),
        "halo_written",
        "$.reads[0].plane",
    );
    refused_as(
        &mut server,
        &learn_args(&life, "wrapped", &pairs, 2, ",\"noise\":\"least_wrong\"").replace(
            "\"writes\":[{\"plane\":1,\"bit\":0}]",
            "\"writes\":[{\"plane\":1,\"bit\":0},{\"plane\":2,\"bit\":0}]",
        ),
        "steps_writes",
        "$.writes",
    );
    refused_as(
        &mut server,
        &learn_args(
            &life,
            "wrapped",
            &pairs,
            2,
            ",\"noise\":\"least_wrong\",\"min_support\":3",
        ),
        "support_noise",
        "$.min_support",
    );
    let noisy = pairs_of(
        35_003,
        &[300, 350],
        Size([16, 16, 16]),
        &cube,
        (2, true, 10_000),
    );
    let told = refused_as(
        &mut server,
        &learn_args(&cube, "wrapped", &noisy, 2, ""),
        "no_rule_k",
        "$.examples",
    );
    assert_eq!(text_at(&told, "search"), "local");
    assert!(int_at(told.get("closest").unwrap(), "wrong") > 0);
    assert!(text_at(&told, "message").contains("no proof"));
}

const NOISES: [u64; 3] = [0, 5_000, 10_000];

const CORE_USED: u64 = 1_000;

const SMALL_SIDE: usize = 16;

const SMALL_RECOVERED: [(u32, usize); 3] = [(2, 1), (3, 0), (4, 0)];

const SMALL_PINS: [usize; 4] = [0, 3, 10, 16];

const SOUPS: [u64; 4] = [100, 250, 150, 300];

fn bays() -> Rule {
    Rule::of(Tally::Outer, true, &moore(true), &[6], &[5, 6, 7])
}

fn x1_rule() -> Rule {
    Rule::of(Tally::Outer, true, &moore(true), &[5, 6], &[4, 5, 6])
}

fn total3() -> Rule {
    Rule::of(Tally::Totalistic, true, &moore(true), &[6, 7, 8, 9], &[])
}

fn life() -> Rule {
    Rule::of(Tally::Outer, false, &moore(false), &[3], &[2, 3])
}

fn likely_learned(rule: &Rule, pairs: &[Pair], steps: u32, search: least::Search) -> Learned {
    let words = words_of(pairs);
    let samples = samples_of(&words);
    least::likely(&stepping_of(rule, steps), &samples, search, &mut || false)
        .unwrap()
        .unwrap()
}

fn matrix_pairs(rule: &Rule, seed: u64, steps: u32, noise: u64) -> Vec<Pair> {
    sized_pairs(rule, seed, steps, noise, 32)
}

fn sized_pairs(rule: &Rule, seed: u64, steps: u32, noise: u64, side: usize) -> Vec<Pair> {
    let size = if rule.cube {
        Size([side, side, side])
    } else {
        Size([side * 3, side * 3, 1])
    };
    pairs_of(seed, &SOUPS, size, rule, (steps, true, noise))
}

fn recovered(label: &str, rule: &Rule, pairs: &[Pair], steps: u32, got: &Learned) {
    let wrongly = wrongly_pinned(rule, got);
    assert!(
        wrongly.is_empty(),
        "{label}: entries pinned against the true table (class, margin): {wrongly:?}; answer {} true {}",
        got.code,
        rule.code
    );
    assert!(got.complete, "{label}");
    assert_eq!(
        got.wrong,
        wrong_of(&rule.with(got.code), pairs, steps),
        "{label}: wrong recounted"
    );
}

fn families() -> [(&'static str, Rule, least::Search); 4] {
    [
        ("b56_s456", x1_rule(), least::Search::Local),
        ("bays_5766", bays(), least::Search::Local),
        ("totalistic_3d", total3(), least::Search::Local),
        ("life_2d", life(), least::Search::Exhaustive),
    ]
}

fn rules_recovered(side: usize, cases: &[(u32, usize)]) {
    for (name, rule, search) in &families() {
        for (steps, at) in cases {
            let (steps, at) = (*steps, *at);
            let noise = NOISES[at];
            let label = format!("{name} side {side} k {steps} noise {noise} per million");
            let seed = 41_000 + u64::from(steps) * 10 + at as u64;
            let pairs = sized_pairs(rule, seed, steps, noise, side);
            let got = likely_learned(rule, &pairs, steps, *search);
            recovered(&label, rule, &pairs, steps, &got);
            if noise == 0 {
                assert_eq!(got.wrong, 0, "{label}");
                assert_eq!(got.rate, None, "{label}: no noise, no likelihood pass");
            } else {
                assert!(got.rate.is_some(), "{label}");
            }
            let doubt = got.doubt.as_ref().unwrap();
            let core: Vec<u32> = (0..rule.classes())
                .filter(|class| {
                    (rule.code >> class) & 1 == 1 && doubt.used[*class as usize] >= CORE_USED
                })
                .collect();
            assert!(
                !core.is_empty(),
                "{label}: the data reaches no live entry of the rule"
            );
            for class in &core {
                assert!(
                    doubt.pinned[*class as usize] && (got.code >> class) & 1 == 1,
                    "{label}: live entry {class}, used {} times, is not learned",
                    doubt.used[*class as usize]
                );
            }
        }
    }
}

#[test]
fn k1_large_rules_seen_every_k_steps_are_recovered_on_every_pinned_entry() {
    if reference_only("k1_large_rules_seen_every_k_steps_are_recovered_on_every_pinned_entry") {
        return;
    }
    let mut cases = Vec::new();
    for steps in [2_u32, 3, 4] {
        for at in 0..NOISES.len() {
            cases.push((steps, at));
        }
    }
    rules_recovered(32, &cases);
}

#[test]
fn k1_small_rules_seen_every_k_steps_are_recovered_on_every_pinned_entry() {
    rules_recovered(SMALL_SIDE, &SMALL_RECOVERED);
}

#[test]
fn k1_the_fewest_wrong_cells_recover_the_rule_where_an_input_flip_does_not_spread_far() {
    for (name, rule) in [("b56_s456", x1_rule()), ("bays_5766", bays())] {
        for (at, noise) in NOISES.iter().enumerate() {
            let label = format!("{name} k 2 noise {noise}");
            let pairs = matrix_pairs(&rule, 42_000 + at as u64, 2, *noise);
            let got = local_learned(&rule, &pairs, 2, true);
            recovered(&label, &rule, &pairs, 2, &got);
            assert!(got.wrong <= wrong_of(&rule, &pairs, 2), "{label}");
        }
    }
}

fn margins_of(rule: &Rule, held: &Json) -> Vec<i64> {
    flat_of(held)
        .iter()
        .map(|one| one.int().unwrap())
        .collect::<Vec<_>>()[..rule.classes() as usize]
        .to_vec()
}

fn doubt_checked(label: &str, rule: &Rule, pairs: &[Pair], steps: u32, got: &Json) {
    finished(label, got);
    let code = code_of(got.get("tables").unwrap());
    let answer = rule.with(code);
    let wrong = wrong_of(&answer, pairs, steps);
    assert_eq!(
        int_at(got, "wrong"),
        i64::try_from(wrong).unwrap(),
        "{label}"
    );
    let margins = margins_of(rule, got.get("margins").unwrap());
    let pinned = flat_of(got.get("pinned").unwrap());
    let reached = reached_of(&answer, pairs, steps);
    let predicted: Vec<Vec<u8>> = pairs
        .iter()
        .map(|one| applied(one.size, &one.input, true, &answer, steps))
        .collect();
    let mut unreached = Vec::new();
    let mut unpinned = Vec::new();
    for class in 0..rule.classes() {
        let other = rule.with(code ^ (1 << class));
        let flipped = wrong_of(&other, pairs, steps);
        let margin = i64::try_from(flipped).unwrap() - i64::try_from(wrong).unwrap();
        assert_eq!(
            margins[class as usize], margin,
            "{label}: margin of entry {class}"
        );
        let moved: u64 = pairs
            .iter()
            .zip(&predicted)
            .map(|(one, said)| {
                applied(one.size, &one.input, true, &other, steps)
                    .iter()
                    .zip(said)
                    .filter(|(a, b)| a != b)
                    .count() as u64
            })
            .sum();
        let claimed = pinned[class as usize] != Json::Null;
        if claimed {
            assert!(
                margin > 0 && moved > 0,
                "{label}: entry {class} margin {margin} over {moved} changed cells is claimed"
            );
            assert_eq!(
                pinned[class as usize].int(),
                Some(i64::from((code >> class) & 1 == 1)),
                "{label}: entry {class}"
            );
        }
        if wrong == 0 {
            assert_eq!(claimed, margin > 0, "{label}: entry {class} without noise");
        }
        if !reached[class as usize] {
            assert_eq!(
                margin, 0,
                "{label}: an unreached entry {class} moved the score"
            );
            unreached.push(class);
        } else if !claimed {
            unpinned.push(class);
        }
    }
    assert_eq!(
        entries_of(rule, got.get("unreached").unwrap()),
        unreached,
        "{label}"
    );
    assert_eq!(
        entries_of(rule, got.get("unpinned").unwrap()),
        unpinned,
        "{label}"
    );
}

fn moves_checked(label: &str, rule: &Rule, pairs: &[Pair], steps: u32, got: &Json) {
    finished(label, got);
    let family = got.get("family").unwrap();
    let mut code = code_of(family.get("tables").unwrap());
    let mut wrong = wrong_of(&rule.with(code), pairs, steps);
    assert_eq!(
        int_at(family, "wrong"),
        i64::try_from(wrong).unwrap(),
        "{label}: family"
    );
    for one in got.get("moves").and_then(Json::list).unwrap() {
        for class in entries_of(rule, one.get("flipped").unwrap()) {
            code ^= 1 << class;
        }
        let after = wrong_of(&rule.with(code), pairs, steps);
        assert_eq!(
            int_at(one, "wrong"),
            i64::try_from(after).unwrap(),
            "{label}: a move"
        );
        assert!(
            after < wrong,
            "{label}: a kept flip raised the wrong cells from {wrong} to {after}"
        );
        wrong = after;
    }
    assert_eq!(
        code,
        code_of(got.get("tables").unwrap()),
        "{label}: moves lead to the answer"
    );
}

#[test]
fn k2_margins_and_unreached_entries_match_a_host_check_of_single_flips() {
    let mut server = quiet(None);
    let cube = x1_rule();
    for (steps, noise) in [(2, 5_000), (3, 0)] {
        let label = format!("3x3x3 k {steps} noise {noise}");
        let pairs = pairs_of(
            43_000 + u64::from(steps),
            &[120, 250, 180],
            Size([16, 16, 16]),
            &cube,
            (steps, true, noise),
        );
        let got = answered(
            &mut server,
            &learn_args(
                &cube,
                "wrapped",
                &pairs,
                steps,
                ",\"noise\":\"least_wrong\"",
            ),
        );
        assert_eq!(text_at(&got, "search"), "local", "{label}");
        doubt_checked(&label, &cube, &pairs, steps, &got);
        moves_checked(&label, &cube, &pairs, steps, &got);
    }
    let start = Rule::of(Tally::Outer, true, &moore(true), &[4], &[3, 4, 5, 6, 7]);
    let label = "a start far from the rule";
    let pairs = pairs_of(
        43_100,
        &[150, 300],
        Size([16, 16, 16]),
        &start,
        (2, true, 0),
    );
    let got = answered(
        &mut server,
        &learn_args(&start, "wrapped", &pairs, 2, ",\"noise\":\"least_wrong\""),
    );
    doubt_checked(label, &start, &pairs, 2, &got);
    moves_checked(label, &start, &pairs, 2, &got);
    let flat = Rule::of(Tally::Outer, false, &moore(false), &[3, 6], &[2, 3]);
    let pairs = pairs_of(
        43_200,
        &[350, 300, 400],
        Size([32, 32, 1]),
        &flat,
        (3, true, 10_000),
    );
    let got = answered(
        &mut server,
        &learn_args(&flat, "wrapped", &pairs, 3, ",\"noise\":\"least_wrong\""),
    );
    assert_eq!(text_at(&got, "search"), "exhaustive");
    doubt_checked("3x3 exhaustive", &flat, &pairs, 3, &got);
    let tied = int_at(&got, "tied");
    assert!(tied >= 1);
    let zero_margins = margins_of(&flat, got.get("margins").unwrap())
        .iter()
        .filter(|one| **one == 0)
        .count();
    if zero_margins > 0 {
        assert!(
            tied >= 2,
            "an entry with margin 0 leaves a second table at the least"
        );
    }
    let local = answered(
        &mut server,
        &learn_args(
            &flat,
            "wrapped",
            &pairs,
            3,
            ",\"noise\":\"least_wrong\",\"search\":\"local\"",
        ),
    );
    doubt_checked("3x3 local", &flat, &pairs, 3, &local);
    moves_checked("3x3 local", &flat, &pairs, 3, &local);
    assert!(int_at(&local, "wrong") >= int_at(&got, "wrong"));
}

fn directed(size: Size, cells: &[u8]) -> Vec<u8> {
    let [wide, deep, _] = size.0;
    let even = stepped(size, cells, true, &x1_rule());
    let odd = stepped(size, cells, true, &bays());
    (0..size.cells())
        .map(|cell| {
            if (cell % wide + cell / wide % deep + cell / wide / deep) % 2 == 0 {
                even[cell]
            } else {
                odd[cell]
            }
        })
        .collect()
}

fn directed_pairs(seed: u64, size: Size, steps: u32, noise: u64) -> Vec<Pair> {
    let mut rng = Rng(seed);
    [100_u64, 200, 150, 250, 120, 180]
        .iter()
        .map(|permille| {
            let clean = soup(&mut rng, size, *permille);
            let mut after = clean.clone();
            for _ in 0..steps {
                after = directed(size, &after);
            }
            Pair {
                size,
                input: flipped(&mut rng, &clean, noise),
                output: flipped(&mut rng, &after, noise),
            }
        })
        .collect()
}

#[test]
fn k3_noise_near_the_estimate_gives_the_true_table_and_another_family_is_far_above_it() {
    for (steps, noise) in [(2, 5_000_u64), (2, 10_000), (3, 5_000)] {
        let label = format!("k {steps} noise {noise}");
        let rule = x1_rule();
        let pairs = matrix_pairs(&rule, 44_000 + noise + u64::from(steps), steps, noise);
        let got = local_learned(&rule, &pairs, steps, true);
        recovered(&label, &rule, &pairs, steps, &got);
        let estimate = got.noise.unwrap();
        assert!(
            estimate.per_million * 10 >= noise * 7 && estimate.per_million * 10 <= noise * 14,
            "{label}: estimated {} per million",
            estimate.per_million
        );
        assert!(
            !estimate.far_above,
            "{label}: {estimate:?} wrong {}",
            got.wrong
        );
    }
    let rule = x1_rule();
    let pairs = directed_pairs(44_100, Size([32, 32, 32]), 1, 5_000);
    let got = local_learned(&rule, &pairs, 1, true);
    let estimate = got.noise.unwrap();
    assert!(estimate.far_above, "{estimate:?} wrong {}", got.wrong);
    assert!(got.wrong > 2 * estimate.expected);
    let mut server = quiet(None);
    let pairs = directed_pairs(44_200, Size([16, 16, 16]), 1, 5_000);
    let got = answered(
        &mut server,
        &learn_args(&rule, "wrapped", &pairs, 1, ",\"noise\":\"least_wrong\""),
    );
    finished("another family", &got);
    let estimate = got.get("noise_estimate").unwrap();
    assert_eq!(
        estimate.get("far_above"),
        Some(&Json::Bool(true)),
        "{} wrong {}",
        written(estimate),
        int_at(&got, "wrong")
    );
    assert!(text_at(&got, "warning").contains("not this rule with noise"));
}

fn called(server: &mut Server<'_>, name: &str, arguments: &str) -> Json {
    let got = server.heard(&line(name, arguments)).unwrap();
    let (refused, held) = content(&got);
    assert!(
        !refused,
        "{name}: {}",
        &written(&held)[..written(&held).len().min(2000)]
    );
    held
}

fn written_cells(server: &mut Server<'_>, world: i64, plane: &str, cells: &[u8], size: Size) {
    let [wide, deep, _] = size.0;
    let set: Vec<String> = cells
        .iter()
        .enumerate()
        .filter(|(_, one)| **one == 1)
        .map(|(cell, _)| {
            format!(
                "[{},{},{},1]",
                cell % wide,
                cell / wide % deep,
                cell / wide / deep
            )
        })
        .collect();
    for chunk in set.chunks(4096) {
        called(
            server,
            "grit_world_write",
            &format!(
                "{{\"world\":{world},{plane},\"cells\":[{}],\"budget_ms\":{LONG}}}",
                chunk.join(",")
            ),
        );
    }
}

fn read_set(server: &mut Server<'_>, world: i64, size: Size) -> Vec<u8> {
    let [wide, deep, tall] = size.0;
    let mut cells = vec![0_u8; size.cells()];
    for z in 0..tall {
        let read = called(
            server,
            "grit_world_read",
            &format!(
                "{{\"world\":{world},\"current\":true,\"form\":\"set\",\"from\":[0,0,{z}],\"to\":[{},{},{z}]}}",
                wide - 1,
                deep - 1
            ),
        );
        for one in read.get("cells").and_then(Json::list).unwrap() {
            let at: Vec<i64> = one
                .list()
                .unwrap()
                .iter()
                .map(|v| v.int().unwrap())
                .collect();
            let cell = (usize::try_from(at[2]).unwrap() * deep + usize::try_from(at[1]).unwrap())
                * wide
                + usize::try_from(at[0]).unwrap();
            cells[cell] = u8::from(at[3] & 1 == 1);
        }
    }
    cells
}

fn law_of(held: &Json) -> String {
    written(&Json::Text(text_at(held, "law")))
}

fn world_learned(host: &Host, size: Size, densities: &[u64], cases: &[(u32, &str)]) {
    let mut server = quiet(Some(host));
    let rule = x1_rule();
    let [wide, deep, tall] = size.0;
    let carrier_pairs = pairs_of(45_000, &[200], Size([8, 8, 8]), &rule, (1, true, 0));
    let carrier = answered(
        &mut server,
        &learn_args(&rule, "wrapped", &carrier_pairs, 1, "").replace(",\"steps\":1", ""),
    );
    for (steps, search) in cases {
        let steps = *steps;
        let pairs = pairs_of(
            45_000 + u64::from(steps),
            densities,
            size,
            &rule,
            (steps, true, 5_000),
        );
        let mut worlds = Vec::new();
        for one in &pairs {
            let made = called(
                &mut server,
                "grit_world_create",
                &format!(
                    "{{\"wide\":{wide},\"deep\":{deep},\"tall\":{tall},\"law\":{},\"budget_ms\":{LONG}}}",
                    law_of(&carrier)
                ),
            );
            let world = int_at(&made, "world");
            written_cells(&mut server, world, "\"plane\":0", &one.input, size);
            written_cells(&mut server, world, "\"plane\":1", &one.output, size);
            worlds.push(world);
        }
        let examples: Vec<String> = worlds
            .iter()
            .map(|one| format!("{{\"world\":{one}}}"))
            .collect();
        let arguments = format!(
            "{{\"stencil\":\"3x3x3\",\"reads\":[{{\"plane\":0,\"bit\":0}}],\"writes\":[{{\"plane\":1,\"bit\":0}}],\"border\":\"wrapped\",\"rule\":\"outer\",\"places\":\"moore\",\"steps\":{steps},\"noise\":\"{search}\",\"examples\":[{}],\"budget_ms\":{LONG}}}",
            examples.join(",")
        );
        let got = answered(&mut server, &arguments);
        let label = format!("world examples {wide}x{deep}x{tall} k {steps} {search}");
        finished(&label, &got);
        assert_eq!(text_at(&got, "search"), "local", "{label}");
        let code = code_of(got.get("tables").unwrap());
        let margins: Vec<u64> = flat_of(got.get("pinned").unwrap())
            .iter()
            .map(|one| u64::from(*one != Json::Null))
            .collect();
        assert!(margins.contains(&1), "{label}: no entry is pinned");
        assert!(
            against(&rule, code, &margins).is_empty(),
            "{label}: {}",
            written(got.get("pinned").unwrap())
        );
        assert_eq!(
            got.get("noise_estimate")
                .and_then(|one| one.get("far_above")),
            Some(&Json::Bool(false)),
            "{label}"
        );
        let mut mismatched = 0_u64;
        for one in &pairs {
            let made = called(
                &mut server,
                "grit_world_create",
                &format!(
                    "{{\"wide\":{wide},\"deep\":{deep},\"tall\":{tall},\"law\":{},\"alternate\":[0,1],\"budget_ms\":{LONG}}}",
                    law_of(&got)
                ),
            );
            let world = int_at(&made, "world");
            written_cells(&mut server, world, "\"current\":true", &one.input, size);
            called(
                &mut server,
                "grit_world_step",
                &format!("{{\"world\":{world},\"frames\":{steps},\"budget_ms\":{LONG}}}"),
            );
            let ran = read_set(&mut server, world, size);
            assert_eq!(
                ran,
                applied(size, &one.input, true, &rule.with(code), steps),
                "{label}: the world runs the table"
            );
            mismatched += ran.iter().zip(&one.output).filter(|(a, b)| a != b).count() as u64;
            called(
                &mut server,
                "grit_world_close",
                &format!("{{\"world\":{world}}}"),
            );
        }
        assert_eq!(
            i64::try_from(mismatched).unwrap(),
            int_at(&got, "wrong"),
            "{label}"
        );
        for world in worlds {
            called(
                &mut server,
                "grit_world_close",
                &format!("{{\"world\":{world}}}"),
            );
        }
    }
}

const SMALL_WORLDS: [u64; 6] = [100, 250, 150, 300, 200, 120];

const X1_CASES: [(u32, &str); 2] = [(2, "least_wrong"), (4, "likely")];

#[test]
fn k1_an_x1_sized_case_learned_from_world_examples_runs_on_a_world_within_the_noise() {
    let host = device("law_steps_large world");
    let Some(host) = host.as_ref() else {
        return;
    };
    if reference_only(
        "k1_an_x1_sized_case_learned_from_world_examples_runs_on_a_world_within_the_noise",
    ) {
        return;
    }
    world_learned(
        host,
        Size([32, 32, 32]),
        &[100, 250, 150, 300, 200, 120],
        &X1_CASES,
    );
}

#[test]
fn k1_a_small_case_learned_from_world_examples_runs_on_a_world_within_the_noise() {
    let host = device("law_steps_large small world");
    let Some(host) = host.as_ref() else {
        return;
    };
    world_learned(host, Size([16, 16, 16]), &SMALL_WORLDS, &X1_CASES);
}

#[cfg(feature = "faults")]
mod braked {
    use super::*;

    use std::sync::Arc;
    use std::time::Duration;

    use grit::clock::{Fake, Time};
    use grit::serve3::Options;
    use grit::serve3::law_learn::Point;
    use grit::serve3::law_learn::faults::{deadline_passed_at, disarm, reached};

    fn timed() -> (Server<'static>, Arc<Fake>) {
        let fake = Arc::new(Fake::new());
        let time: Arc<dyn Time> = fake.clone();
        (
            Server::timed(None, &Options::default(), time).unwrap(),
            fake,
        )
    }

    fn arguments() -> String {
        let rule = life();
        let pairs = pairs_of(
            46_000,
            &[350, 300, 400],
            Size([32, 32, 1]),
            &rule,
            (3, true, 5_000),
        );
        learn_args(
            &rule,
            "wrapped",
            &pairs,
            3,
            ",\"noise\":\"least_wrong\",\"search\":\"local\"",
        )
    }

    #[test]
    fn a_stopped_local_search_claims_no_entry_and_reads_no_clock() {
        let (mut server, fake) = timed();
        disarm();
        let whole = answered(&mut server, &arguments());
        let brakes = reached(Point::Search);
        disarm();
        assert_eq!(fake.elapsed(), Duration::ZERO, "the search read no clock");
        assert!(brakes >= 4, "{brakes} brakes");
        assert_eq!(text_at(&whole, "stopped"), "complete");
        assert!(whole.get("pinned").is_some());
        for ordinal in [0, 1] {
            let (mut server, fake) = timed();
            disarm();
            deadline_passed_at(Point::Search, ordinal, Arc::clone(&fake), LONG + 1);
            let told = refused_as(&mut server, &arguments(), "budget", "$.budget_ms");
            disarm();
            assert_eq!(text_at(&told, "search"), "local");
        }
        let (mut server, fake) = timed();
        disarm();
        deadline_passed_at(Point::Search, 2, Arc::clone(&fake), LONG + 1);
        let late = answered(&mut server, &arguments());
        disarm();
        assert_eq!(text_at(&late, "stopped"), "budget");
        assert_eq!(late.get("complete"), Some(&Json::Bool(false)));
        for key in [
            "margins",
            "pinned",
            "unpinned",
            "unreached",
            "noise_estimate",
        ] {
            assert!(late.get(key).is_none(), "a stopped search shows {key}");
        }
        assert!(late.get("hint").is_some());
        let again = answered(&mut Server::new(None), &arguments());
        assert_eq!(
            written(&whole),
            written(&again),
            "a complete search is the same bytes"
        );
    }

    #[test]
    fn a_stopped_answer_fails_the_check_by_name() {
        let (mut server, _fake) = timed();
        disarm();
        let whole = answered(&mut server, &arguments());
        finished("a complete search", &whole);
        for ordinal in [2, 3] {
            let (mut server, fake) = timed();
            disarm();
            deadline_passed_at(Point::Search, ordinal, Arc::clone(&fake), LONG + 1);
            let late = answered(&mut server, &arguments());
            disarm();
            assert_eq!(text_at(&late, "stopped"), "budget");
            let caught = std::panic::catch_unwind(|| finished("late", &late));
            let told = caught
                .err()
                .and_then(|held| held.downcast::<String>().ok())
                .map_or_else(String::new, |held| *held);
            assert!(
                told.contains("late: the answer stopped budget"),
                "a stopped answer at brake {ordinal} passed the check: {told:?}"
            );
        }
    }
}

fn falsely_pinned(rule: &Rule, got: &Learned) -> Vec<u32> {
    let Some(doubt) = got.doubt.as_ref() else {
        return Vec::new();
    };
    (0..rule.classes())
        .filter(|class| doubt.pinned[*class as usize] && ((got.code ^ rule.code) >> class) & 1 == 1)
        .collect()
}

fn pinned_count(got: &Learned) -> usize {
    got.doubt
        .as_ref()
        .map_or(0, |doubt| doubt.pinned.iter().filter(|one| **one).count())
}

fn pin_cases() -> Vec<(String, Rule, Vec<u64>, u32)> {
    let odd = Rule::of(Tally::Outer, true, &moore(true), &[5, 6, 10], &[4, 5, 6]);
    let sparse = [20_u64, 30, 40, 50, 20, 30, 40, 50];
    let dense = [100_u64, 250, 150, 300];
    let mut cases: Vec<(String, Rule, Vec<u64>, u32)> = Vec::new();
    for (name, rule) in [("b56_s456", x1_rule()), ("bays_5766", bays())] {
        for steps in 1..=4_u32 {
            cases.push((
                format!("{name} sparse k {steps}"),
                rule.clone(),
                sparse.to_vec(),
                steps,
            ));
            cases.push((
                format!("{name} dense k {steps}"),
                rule.clone(),
                dense.to_vec(),
                steps,
            ));
        }
    }
    cases.push((
        "b5_6_10_s456 10% k 1".to_owned(),
        odd.clone(),
        vec![100; 8],
        1,
    ));
    cases.push(("b5_6_10_s456 8% k 1".to_owned(), odd, vec![80; 8], 1));
    cases
}

fn nothing_pinned_against(side: usize, picked: &[usize]) {
    let cases = pin_cases();
    for at in picked {
        let (label, rule, densities, steps) = &cases[*at];
        let pairs = pairs_of(
            47_000 + *at as u64,
            densities,
            Size([side, side, side]),
            rule,
            (*steps, true, 10_000),
        );
        let fewest = local_learned(rule, &pairs, *steps, true);
        let likely = likely_learned(rule, &pairs, *steps, least::Search::Local);
        let wrongly = falsely_pinned(rule, &fewest);
        let wrongly_likely = falsely_pinned(rule, &likely);
        eprintln!(
            "PINS {label} side {side}: least_wrong pinned {} false {:?} far_above {} | likely pinned {} false {:?} rate {:?} recovered {}",
            pinned_count(&fewest),
            wrongly,
            fewest.noise.is_some_and(|one| one.far_above),
            pinned_count(&likely),
            wrongly_likely,
            likely.rate,
            likely.code == rule.code
        );
        assert!(fewest.complete && likely.complete, "{label}");
        assert!(
            wrongly.is_empty(),
            "{label}: least_wrong pins {wrongly:?} against the rule"
        );
        assert!(
            wrongly_likely.is_empty(),
            "{label}: likely pins {wrongly_likely:?} against the rule"
        );
    }
}

#[test]
fn k2_no_entry_is_pinned_against_the_rule_on_sparse_or_dense_inputs_in_either_mode() {
    if reference_only(
        "k2_no_entry_is_pinned_against_the_rule_on_sparse_or_dense_inputs_in_either_mode",
    ) {
        return;
    }
    let every: Vec<usize> = (0..pin_cases().len()).collect();
    nothing_pinned_against(32, &every);
}

#[test]
fn k2_no_entry_is_pinned_against_the_rule_in_a_small_case_of_either_mode() {
    nothing_pinned_against(SMALL_SIDE, &SMALL_PINS);
}

#[cfg(feature = "faults")]
mod stopped_everywhere {
    use super::*;

    use std::sync::Arc;

    use grit::clock::{Fake, Time};
    use grit::serve3::Options;
    use grit::serve3::law_learn::Point;
    use grit::serve3::law_learn::faults::{deadline_passed_at, disarm, reached};

    fn timed() -> (Server<'static>, Arc<Fake>) {
        let fake = Arc::new(Fake::new());
        let time: Arc<dyn Time> = fake.clone();
        (
            Server::timed(None, &Options::default(), time).unwrap(),
            fake,
        )
    }

    fn arguments(noise: &str) -> String {
        let rule = life();
        let pairs = pairs_of(
            48_000,
            &[350, 300, 400],
            Size([32, 32, 1]),
            &rule,
            (3, true, 5_000),
        );
        learn_args(
            &rule,
            "wrapped",
            &pairs,
            3,
            &format!(",\"noise\":\"{noise}\",\"search\":\"local\""),
        )
    }

    #[test]
    fn a_stop_at_any_brake_of_either_mode_claims_nothing() {
        for noise in ["least_wrong", "likely"] {
            let (mut server, _fake) = timed();
            disarm();
            let whole = answered(&mut server, &arguments(noise));
            let brakes = reached(Point::Search);
            disarm();
            assert_eq!(text_at(&whole, "stopped"), "complete", "{noise}");
            assert_eq!(text_at(&whole, "noise"), noise);
            assert!(whole.get("pinned").is_some(), "{noise}");
            assert!(brakes > 4, "{noise}: {brakes} brakes");
            let mut answered_late = 0;
            for ordinal in 0..brakes {
                let (mut server, fake) = timed();
                disarm();
                deadline_passed_at(Point::Search, ordinal, Arc::clone(&fake), LONG + 1);
                let (refused, late) = content(&heard(&mut server, &arguments(noise)));
                disarm();
                let label = format!("{noise} stopped at brake {ordinal} of {brakes}");
                if refused {
                    assert_eq!(text_at(&late, "refused"), "budget", "{label}");
                    continue;
                }
                answered_late += 1;
                assert_eq!(
                    text_at(&late, "stopped"),
                    "budget",
                    "{label}: {}",
                    written(&late)
                );
                assert_eq!(late.get("complete"), Some(&Json::Bool(false)), "{label}");
                assert_eq!(text_at(&late, "noise"), noise, "{label}");
                assert!(late.get("hint").is_some(), "{label}");
                for key in [
                    "margins",
                    "pinned",
                    "unpinned",
                    "unreached",
                    "noise_estimate",
                    "warning",
                    "assumed_flips_per_million",
                ] {
                    if key == "assumed_flips_per_million" && late.get("surprise").is_some() {
                        continue;
                    }
                    assert!(
                        late.get(key).is_none(),
                        "{label}: a stopped search shows {key}"
                    );
                }
            }
            assert!(answered_late > 0, "{noise}: no brake answered a table");
        }
    }
}

fn fewest_cases() -> Vec<(usize, u32, usize)> {
    let mut cases = Vec::new();
    for family in 0..3 {
        for steps in [3_u32, 4] {
            for at in 1..NOISES.len() {
                cases.push((family, steps, at));
            }
        }
    }
    cases
}

fn fewest_honest(side: usize, cases: &[(usize, u32, usize)]) {
    let rules = [
        ("b56_s456", x1_rule()),
        ("bays_5766", bays()),
        ("totalistic_3d", total3()),
    ];
    for (family, steps, at) in cases {
        let (name, rule) = &rules[*family];
        let (steps, at) = (*steps, *at);
        let noise = NOISES[at];
        let label = format!("{name} side {side} k {steps} noise {noise} per million");
        let seed = 41_000 + u64::from(steps) * 10 + at as u64;
        let pairs = sized_pairs(rule, seed, steps, noise, side);
        let got = local_learned(rule, &pairs, steps, true);
        let estimate = got.noise.unwrap();
        let wrongly = falsely_pinned(rule, &got);
        eprintln!(
            "FEWEST {label}: pinned {} false {wrongly:?} estimate {} far_above {}",
            pinned_count(&got),
            estimate.per_million,
            estimate.far_above
        );
        assert!(got.complete, "{label}");
        assert!(
            wrongly.is_empty(),
            "{label}: pins {wrongly:?} against the rule"
        );
        if estimate.far_above {
            assert_eq!(pinned_count(&got), 0, "{label}: far above yet pinned");
        }
    }
}

#[test]
fn k3_the_fewest_wrong_cells_pin_nothing_false_or_are_far_above_in_the_noisy_cells_past_two_steps()
{
    if reference_only(
        "k3_the_fewest_wrong_cells_pin_nothing_false_or_are_far_above_in_the_noisy_cells_past_two_steps",
    ) {
        return;
    }
    fewest_honest(32, &fewest_cases());
}

#[test]
fn k3_the_fewest_wrong_cells_pin_nothing_false_or_are_far_above_in_a_small_case() {
    fewest_honest(SMALL_SIDE, &fewest_cases());
}
