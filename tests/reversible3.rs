#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use grit::Host;
use grit::branch::{Made, World};
use grit::halo::BOX;
use grit::law::{Code, Law, Refusal, limits};
use grit::rule3::{CORNERS, Oct, Rule};
use grit::sand3::{Grid3, reference};
use grit::serve3::exec::Exec;
use grit::serve3::law::{background_class, stepping_of};
use grit::serve3::{Options, Server};
use grit::wire::{Json, written};

const FRAMES: u32 = 512;

const MIX: &str = include_str!("law/mix.law.json");
const STIR: &str = include_str!("law/stir.law.json");
const HEAT: &str = include_str!("law/heat.law.json");

fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn below(state: &mut u64, most: u64) -> u64 {
    next(state) % most
}

fn word(state: &mut u64) -> u32 {
    match below(state, 8) {
        0 => 0,
        1 => u32::MAX,
        2 => 1 << 31,
        3 => u32::try_from(below(state, 16)).unwrap(),
        _ => u32::try_from(next(state) >> 32).unwrap(),
    }
}

fn compiled(text: &str) -> Law {
    Law::compile(text).unwrap_or_else(|why| panic!("{why}\n{text}"))
}

fn refused(text: &str) -> Refusal {
    Law::compile(text).map(|_| ()).expect_err(text)
}

#[derive(Clone, Copy, Debug)]
struct Kind {
    channels: u32,
    params: u32,
    clock: bool,
    quiet: bool,
    looks: bool,
}

struct Writer<'s> {
    state: &'s mut u64,
    kind: Kind,
    versions: Vec<u32>,
    looked: bool,
}

impl Writer<'_> {
    fn words(&self) -> usize {
        CORNERS * self.kind.channels as usize
    }

    fn written_planes(&self) -> u32 {
        if self.kind.looks {
            self.kind.channels - 1
        } else {
            self.kind.channels
        }
    }

    fn named(&self, word: usize) -> String {
        format!("\"w{word}v{}\"", self.versions[word])
    }

    fn other(&mut self, target: usize) -> String {
        loop {
            let pick = usize::try_from(below(self.state, self.words() as u64)).unwrap();
            if pick != target {
                return self.named(pick);
            }
        }
    }

    fn look(&mut self, apart: bool) -> String {
        self.looked = true;
        let plane = self.kind.channels - 1;
        let mut offset = [0_i64; 3];
        loop {
            for slot in &mut offset {
                *slot = i64::try_from(below(self.state, 3)).unwrap() - 1;
            }
            if !apart || offset != [0, 0, 0] {
                break;
            }
        }
        let corner = below(self.state, 8);
        format!(
            "[\"look\",{plane},{},{},{},{corner}]",
            offset[0], offset[1], offset[2]
        )
    }

    fn leaf(&mut self, target: usize) -> String {
        match below(self.state, 6) {
            0 | 1 => self.other(target),
            2 if self.kind.params > 0 => {
                format!(
                    "[\"param\",{}]",
                    below(self.state, u64::from(self.kind.params))
                )
            }
            3 if self.kind.clock => match below(self.state, 3) {
                0 => "\"frame\"".to_owned(),
                1 => format!("[\"at\",\"at\",{}]", below(self.state, 3)),
                _ => "[\"noise\"]".to_owned(),
            },
            4 if self.kind.looks => self.look(false),
            _ => word(self.state).to_string(),
        }
    }

    fn compare(&mut self, depth: u32, target: usize) -> String {
        let ops = ["eq", "ne", "lt", "le", "gt", "ge"];
        let op = ops[usize::try_from(below(self.state, 6)).unwrap()];
        let left = self.expr(depth, target);
        let right = self.expr(depth, target);
        let made = format!("[\"{op}\",{left},{right}]");
        match below(self.state, 5) {
            0 => format!("[\"denied\",{made}]"),
            1 => {
                let other = self.compare(depth.saturating_sub(1), target);
                format!("[\"both\",{made},{other}]")
            }
            2 => {
                let other = self.compare(depth.saturating_sub(1), target);
                format!("[\"either\",{made},{other}]")
            }
            _ => made,
        }
    }

    fn expr(&mut self, depth: u32, target: usize) -> String {
        if depth == 0 || below(self.state, 4) == 0 {
            return self.leaf(target);
        }
        let less = depth - 1;
        match below(self.state, 7) {
            0..=2 => {
                let ops = [
                    "add", "sub", "mul", "xor", "and", "or", "div", "dot_u", "dot_s",
                ];
                let op = ops[usize::try_from(below(self.state, 9)).unwrap()];
                let left = self.expr(less, target);
                let right = self.expr(less, target);
                format!("[\"{op}\",{left},{right}]")
            }
            3 => {
                let op = if below(self.state, 2) == 0 {
                    "shl"
                } else {
                    "shr"
                };
                let shifted = self.expr(less, target);
                format!("[\"{op}\",{shifted},{}]", below(self.state, 32))
            }
            4 => format!("[\"not\",{}]", self.expr(less, target)),
            5 => {
                let flag = self.compare(less, target);
                let then = self.expr(less, target);
                let otherwise = self.expr(less, target);
                format!("[\"sel\",{flag},{then},{otherwise}]")
            }
            _ => format!("[\"word\",{}]", self.compare(less, target)),
        }
    }

    fn cell(&self, word: usize) -> String {
        let corner = word % CORNERS;
        let plane = word / CORNERS;
        if self.kind.channels <= 1 {
            format!("[\"at\",\"cells\",{corner}]")
        } else {
            format!("[\"at\",[\"at\",\"cells\",{corner}],{plane}]")
        }
    }
}

fn generated(kind: Kind, seed: u64, name: &str) -> String {
    let mut state = seed;
    let words = CORNERS * kind.channels as usize;
    let mut writer = Writer {
        state: &mut state,
        kind,
        versions: vec![0; words],
        looked: false,
    };
    let mut bindings: Vec<String> = (0..words)
        .map(|word| format!("[{},{}]", writer.named(word), writer.cell(word)))
        .collect();
    let steps = 3 + below(writer.state, 10);
    let writable = CORNERS * writer.written_planes() as usize;
    for step in 0..steps {
        let target = usize::try_from(below(writer.state, writable as u64)).unwrap();
        let op = ["add", "sub", "xor"][usize::try_from(below(writer.state, 3)).unwrap()];
        let mut by = writer.expr(3, target);
        if kind.looks && !writer.looked && step + 1 == steps {
            let look = writer.look(true);
            by = format!("[\"add\",{by},{look}]");
        }
        if kind.quiet {
            let other = writer.other(target);
            by = format!("[\"and\",{by},{other}]");
        }
        let before = writer.named(target);
        writer.versions[target] += 1;
        let after = writer.named(target);
        bindings.push(format!("[{after},[\"{op}\",{before},{by}]]"));
    }
    let out: Vec<String> = (0..CORNERS)
        .map(|corner| {
            if kind.channels <= 1 {
                return writer.named(corner);
            }
            let planes: Vec<String> = (0..kind.channels as usize)
                .map(|plane| writer.named(plane * CORNERS + corner))
                .collect();
            format!("[\"vec\",{}]", planes.join(","))
        })
        .collect();
    let first = writer.named(0);
    let origin = writer.cell(0);
    format!(
        "{{\"law\":1,\"name\":\"{name}\",\"params\":{},\"channels\":{},\"halo\":{},\"reversible\":true,\"tally\":{{\"counts\":1,\"weighted\":false}},\"consts\":{{}},\"defs\":{{}},\"step\":[\"let\",[{}],[\"vec\",[\"vec\",{}],[\"vec\",[\"word\",[\"ne\",{first},{origin}]]]]]}}",
        kind.params,
        kind.channels,
        u32::from(kind.looks),
        bindings.join(","),
        out.join(",")
    )
}

fn kinds() -> Vec<Kind> {
    let mut out = Vec::new();
    for channels in 1..=3 {
        for params in [0, 2] {
            for (clock, quiet) in [(false, true), (true, false), (false, false)] {
                for looks in [false, true] {
                    if looks && channels < 2 {
                        continue;
                    }
                    out.push(Kind {
                        channels,
                        params,
                        clock,
                        quiet,
                        looks,
                    });
                }
            }
        }
    }
    out
}

fn params_of(kind: Kind, seed: u64) -> Vec<u32> {
    let mut state = seed;
    (0..kind.params).map(|_| word(&mut state)).collect()
}

struct Case {
    words: [u32; CORNERS],
    upper: Vec<[u32; CORNERS]>,
    halo: Vec<[u32; BOX]>,
    frame: u32,
    at: [u32; 3],
}

fn stepped(law: &Law, case: &Case, params: &[u32]) -> ([u32; CORNERS], Vec<[u32; CORNERS]>) {
    let planed = law.step_halo(
        Oct {
            words: case.words,
            frame: case.frame,
            at: case.at,
            params,
        },
        &case.upper,
        &case.halo,
    );
    (planed.ruled.out, planed.planes)
}

fn comes_back(law: &Law, inverse: &Law, case: &Case, params: &[u32]) -> bool {
    let (out, upper) = stepped(law, case, params);
    let after = Case {
        words: out,
        upper,
        halo: case.halo.clone(),
        frame: case.frame,
        at: case.at,
    };
    let (back, back_upper) = stepped(inverse, &after, params);
    let (round, round_upper) = stepped(
        law,
        &Case {
            words: back,
            upper: back_upper.clone(),
            halo: case.halo.clone(),
            frame: case.frame,
            at: case.at,
        },
        params,
    );
    back == case.words
        && back_upper == case.upper
        && round == after.words
        && round_upper == after.upper
}

fn sampled(state: &mut u64, kind: Kind) -> Case {
    let planes = kind.channels as usize;
    Case {
        words: core::array::from_fn(|_| word(state)),
        upper: (1..planes)
            .map(|_| core::array::from_fn(|_| word(state)))
            .collect(),
        halo: (0..planes)
            .map(|_| core::array::from_fn(|_| word(state)))
            .collect(),
        frame: word(state),
        at: core::array::from_fn(|_| word(state)),
    }
}

#[test]
fn every_generated_law_compiles_reversible_with_an_ordinary_inverse() {
    let mut seed = 0xA1A_u64;
    for (index, kind) in kinds().into_iter().enumerate() {
        for round in 0..4 {
            seed = next(&mut seed);
            let text = generated(kind, seed, &format!("gen{index}r{round}"));
            let law = compiled(&text);
            assert!(law.reversible());
            let inverse = law
                .inverse()
                .unwrap_or_else(|why| panic!("{why}\n{text}"))
                .unwrap_or_else(|| panic!("no inverse\n{text}"));
            assert!(inverse.reversible());
            assert_ne!(inverse.hex(), law.hex());
            assert_eq!(inverse.name(), format!("gen{index}r{round}_back"));
            assert_eq!(inverse.channels(), law.channels());
            assert_eq!(inverse.halo(), law.halo());
            assert_eq!(
                grit::rule3::Declared::params(inverse),
                grit::rule3::Declared::params(&law)
            );
            assert_eq!(
                grit::rule3::Declared::looks(inverse),
                grit::rule3::Declared::looks(&law)
            );
            let again = compiled(inverse.canonical());
            assert_eq!(again.hex(), inverse.hex());
            assert_eq!(again.dag(), inverse.dag());
            let twice = again.inverse().unwrap().unwrap();
            let params = params_of(kind, seed);
            let mut state = seed ^ 0x55;
            for _ in 0..256 {
                let case = sampled(&mut state, kind);
                assert_eq!(
                    stepped(twice, &case, &params),
                    stepped(&law, &case, &params),
                    "the inverse of the inverse steps as the law\n{text}"
                );
            }
        }
    }
}

#[test]
fn the_inverse_composed_with_the_law_is_the_identity_on_sampled_full_words() {
    let mut seed = 0x2A2_u64;
    let mut laws = 0;
    for (index, kind) in kinds().into_iter().enumerate() {
        seed = next(&mut seed);
        let text = generated(kind, seed, &format!("full{index}"));
        let law = compiled(&text);
        let inverse = law.inverse().unwrap().unwrap();
        let params = params_of(kind, seed);
        let mut state = seed;
        for sample in 0..1 << 16 {
            let case = sampled(&mut state, kind);
            assert!(
                comes_back(&law, inverse, &case, &params),
                "sample {sample} of {text}"
            );
        }
        laws += 1;
    }
    for text in [MIX, STIR] {
        let law = compiled(text);
        let inverse = law.inverse().unwrap().unwrap();
        let kind = Kind {
            channels: 1,
            params: 0,
            clock: false,
            quiet: false,
            looks: false,
        };
        let mut state = 0x77_u64;
        for _ in 0..1 << 16 {
            let params: Vec<u32> = (0..grit::rule3::Declared::params(&law))
                .map(|_| word(&mut state))
                .collect();
            let case = sampled(&mut state, kind);
            assert!(comes_back(&law, inverse, &case, &params), "{}", law.name());
        }
        laws += 1;
    }
    assert!(laws >= 30, "{laws} laws");
}

#[test]
fn the_inverse_composed_with_the_law_is_the_identity_on_every_oct_of_an_edge_domain() {
    const EDGE: [u32; 4] = [0, 1, 1 << 31, u32::MAX];
    let mut seed = 0xED6E_u64;
    let mut laws = Vec::new();
    for (index, kind) in kinds()
        .into_iter()
        .filter(|kind| kind.channels == 1)
        .enumerate()
    {
        seed = next(&mut seed);
        laws.push((
            compiled(&generated(kind, seed, &format!("edge{index}"))),
            params_of(kind, seed),
        ));
    }
    laws.push((compiled(MIX), vec![3, 1 << 20]));
    laws.push((compiled(STIR), vec![7, 1 << 12, u32::MAX]));
    for (law, params) in &laws {
        let inverse = law.inverse().unwrap().unwrap();
        for index in 0..4_u32.pow(8) {
            let words: [u32; CORNERS] =
                core::array::from_fn(|corner| EDGE[((index >> (2 * corner)) & 3) as usize]);
            let case = Case {
                words,
                upper: Vec::new(),
                halo: vec![[index; BOX]],
                frame: index,
                at: [index, index >> 3, index >> 6],
            };
            assert!(
                comes_back(law, inverse, &case, params),
                "oct {index} of {}",
                law.canonical()
            );
        }
    }
}

fn steps(stepped: &str) -> String {
    format!(
        "{{\"law\":1,\"name\":\"bad\",\"params\":1,\"channels\":2,\"halo\":0,\"reversible\":true,\"tally\":{{\"counts\":1,\"weighted\":false}},\"consts\":{{}},\"defs\":{{}},\"step\":[\"let\",[[\"a\",[\"at\",[\"at\",\"cells\",0],0]],[\"b\",[\"at\",[\"at\",\"cells\",1],0]],[\"c\",[\"at\",[\"at\",\"cells\",0],1]]{stepped}],[\"vec\",[\"vec\",[\"vec\",\"a\",\"c\"],[\"vec\",\"b\",[\"at\",[\"at\",\"cells\",1],1]],[\"vec\",[\"at\",[\"at\",\"cells\",2],0],[\"at\",[\"at\",\"cells\",2],1]],[\"vec\",[\"at\",[\"at\",\"cells\",3],0],[\"at\",[\"at\",\"cells\",3],1]],[\"vec\",[\"at\",[\"at\",\"cells\",4],0],[\"at\",[\"at\",\"cells\",4],1]],[\"vec\",[\"at\",[\"at\",\"cells\",5],0],[\"at\",[\"at\",\"cells\",5],1]],[\"vec\",[\"at\",[\"at\",\"cells\",6],0],[\"at\",[\"at\",\"cells\",6],1]],[\"vec\",[\"at\",[\"at\",\"cells\",7],0],[\"at\",[\"at\",\"cells\",7],1]]],[\"vec\",0]]]}}"
    )
}

fn not_lifting(text: &str, path: &str, found: &str) {
    let why = refused(text);
    assert_eq!(why.code, Code::Lifting, "{why}\n{text}");
    assert_eq!(why.code.name(), "not_lifting");
    assert_eq!(why.path, path, "{why}");
    assert!(why.found.contains(found), "{why}");
}

#[test]
fn every_form_that_is_not_lifting_steps_is_refused_by_name_and_path() {
    let accepted = steps(",[\"a\",[\"add\",\"a\",[\"mul\",\"b\",[\"param\",0]]]]");
    assert!(compiled(&accepted).reversible());
    let cases = [
        (
            ",[\"a\",[\"add\",\"a\",[\"mul\",\"a\",3]]]",
            "$.step[1][3][1]",
            "reads word 0 in what it adds",
        ),
        (
            ",[\"a\",[\"sub\",\"a\",[\"shr\",[\"add\",\"b\",\"a\"],1]]]",
            "$.step[1][3][1]",
            "reads word 0 in what it adds",
        ),
        (
            ",[\"a\",[\"mul\",\"b\",3]]",
            "$.step[1][3][1]",
            "word 0 is made by mul",
        ),
        (
            ",[\"a\",\"b\"]",
            "$.step[2][1][1][1]",
            "word 0 is made by cell 1 unchanged",
        ),
        (
            ",[\"a\",[\"or\",\"a\",\"b\"]]",
            "$.step[1][3][1]",
            "word 0 is made by or",
        ),
        (
            ",[\"a\",[\"mul\",\"a\",3]]",
            "$.step[1][3][1]",
            "word 0 is made by mul",
        ),
        (
            ",[\"a\",[\"shl\",\"a\",1]]",
            "$.step[1][3][1]",
            "word 0 is made by shl",
        ),
        (
            ",[\"a\",[\"sel\",[\"lt\",\"b\",9],\"a\",\"b\"]]",
            "$.step[1][3][1]",
            "word 0 is made by sel",
        ),
        (
            ",[\"t\",[\"add\",\"a\",\"b\"]],[\"a\",\"t\"],[\"b\",\"t\"]",
            "$.step[1][3][1]",
            "are both made by the one step add",
        ),
        (
            ",[\"a\",[\"sub\",[\"param\",0],\"a\"]]",
            "$.step[1][3][1]",
            "neither side carries the word",
        ),
    ];
    for (stepped, path, found) in cases {
        not_lifting(&steps(stepped), path, found);
    }
}

fn grid_of(size: [u32; 3]) -> Grid3 {
    Grid3::new(size[0], size[1], size[2]).unwrap()
}

fn walked_forward(law: &Law, grid: Grid3, start: &[u32], params: &[u32]) -> Vec<u32> {
    let mut cells = start.to_vec();
    for frame in 0..FRAMES {
        cells = reference(law, grid, frame, &cells, params);
    }
    cells
}

fn walked_back(inverse: &Law, grid: Grid3, from: &[u32], params: &[u32]) -> Vec<u32> {
    let mut cells = from.to_vec();
    for frame in (0..FRAMES).rev() {
        cells = reference(inverse, grid, frame, &cells, params);
    }
    cells
}

fn filled(grid: Grid3, planes: u32, seed: u64, sparse: bool) -> Vec<u32> {
    let mut state = seed;
    let stride = grid.cells();
    let mut cells = vec![0_u32; stride * planes as usize];
    if !sparse {
        for cell in &mut cells {
            *cell = word(&mut state);
        }
        return cells;
    }
    let edges = [
        [0, 0, 0],
        [grid.wide - 1, grid.deep - 1, grid.tall - 1],
        [grid.wide - 1, 0, 0],
        [0, grid.deep - 1, 0],
        [0, 0, grid.tall - 1],
    ];
    for [x, y, z] in edges {
        for dz in 0..2 {
            for dy in 0..2 {
                for dx in 0..2 {
                    for plane in 0..planes as usize {
                        let at = grid.index(x + dx, y + dy, z + dz);
                        cells[plane * stride + at] = word(&mut state) | 1;
                    }
                }
            }
        }
    }
    for _ in 0..24 {
        let at = usize::try_from(below(&mut state, stride as u64)).unwrap();
        for plane in 0..planes as usize {
            cells[plane * stride + at] = word(&mut state) | 1;
        }
    }
    cells
}

fn made<'a>(law: &'a Law, params: &'a [u32], size: [u32; 3], sparse: bool) -> Made<'a> {
    Made {
        rule: law.name(),
        params,
        size,
        sparse,
        slots: None,
    }
}

fn engine<'host>(
    host: &'host Host,
    law: &Law,
    params: &[u32],
    size: [u32; 3],
    sparse: bool,
) -> World<'host> {
    let class = background_class(law, params);
    World::create_law_stepped(
        host,
        law,
        &made(law, params, size, sparse),
        stepping_of(&class, sparse),
    )
    .unwrap_or_else(|why| panic!("{why}: {}", law.canonical()))
}

fn put(world: &mut World<'_>, grid: Grid3, planes: u32, cells: &[u32]) {
    let stride = grid.cells();
    for plane in 0..planes {
        let mut placed = Vec::new();
        for z in 0..grid.tall {
            for y in 0..grid.deep {
                for x in 0..grid.wide {
                    let word = cells[plane as usize * stride + grid.index(x, y, z)];
                    if word != 0 {
                        placed.push(([x, y, z], word));
                    }
                }
            }
        }
        for chunk in placed.chunks(4096) {
            world.write_plane(plane, chunk).unwrap();
        }
    }
}

fn first_oct(grid: Grid3, start: &[u32], now: &[u32]) -> String {
    match start.iter().zip(now).position(|(was, is)| was != is) {
        None => "none".to_owned(),
        Some(at) => {
            let cell = at % grid.cells();
            let plane = at / grid.cells();
            let x = cell % grid.wide as usize;
            let y = cell / grid.wide as usize % grid.deep as usize;
            let z = cell / grid.wide as usize / grid.deep as usize;
            format!(
                "cell [{x}, {y}, {z}] plane {plane}: was {} now {}",
                start[at], now[at]
            )
        }
    }
}

fn home_on_device(
    host: &Host,
    law: &Law,
    params: &[u32],
    size: [u32; 3],
    sparse: bool,
    start: &[u32],
    far: Option<&[u32]>,
) {
    let grid = grid_of(size);
    let planes = law.channels();
    let mut world = engine(host, law, params, size, sparse);
    assert!(world.reversible());
    put(&mut world, grid, planes, start);
    assert_eq!(world.cells().unwrap(), start, "written {}", law.name());
    let hash = world.state_hash().unwrap();
    let mut state = u64::from(size[0]) ^ u64::from(law.hash()[0]);
    let mut left = FRAMES;
    while left > 0 {
        let step = (1 + u32::try_from(below(&mut state, 200)).unwrap()).min(left);
        world.step(step).unwrap();
        left -= step;
    }
    assert_eq!(world.frame(), FRAMES);
    if let Some(far) = far {
        let got = world.cells().unwrap();
        assert!(
            got == far,
            "{} on the device after {FRAMES} frames differs from the cpu twin at {}",
            law.name(),
            first_oct(grid, far, &got)
        );
    }
    let mut left = FRAMES;
    while left > 0 {
        let step = (1 + u32::try_from(below(&mut state, 200)).unwrap()).min(left);
        world.back(step).unwrap();
        left -= step;
    }
    assert_eq!(world.frame(), 0);
    let got = world.cells().unwrap();
    assert!(
        got == start,
        "{} ({}, {size:?}) strayed walking back: first differing {}\n{}",
        law.name(),
        if sparse { "sparse" } else { "dense" },
        first_oct(grid, start, &got),
        law.canonical()
    );
    assert_eq!(world.state_hash().unwrap(), hash);
    world.step(64).unwrap();
    world.back(32).unwrap();
    world.step(16).unwrap();
    world.back(48).unwrap();
    assert_eq!(world.frame(), 0);
    assert_eq!(world.cells().unwrap(), start, "{} turning", law.name());
}

fn home_on_cpu(law: &Law, params: &[u32], grid: Grid3, start: &[u32]) -> Vec<u32> {
    let inverse = law.inverse().unwrap().unwrap();
    let far = walked_forward(law, grid, start, params);
    let back = walked_back(inverse, grid, &far, params);
    assert!(
        back == start,
        "{} strayed on the cpu twin: first differing {}",
        law.name(),
        first_oct(grid, start, &back)
    );
    far
}

fn chosen(count: usize, seed: u64) -> Vec<(Kind, String, Vec<u32>)> {
    let mut seed = seed;
    let mut out = Vec::new();
    let every = kinds();
    for index in 0..count {
        let kind = every[index * 7 % every.len()];
        seed = next(&mut seed);
        out.push((
            kind,
            generated(kind, seed, &format!("walk{index}")),
            params_of(kind, seed),
        ));
    }
    out
}

#[test]
fn random_reversible_laws_come_home_dense_on_the_device_and_the_cpu_twin() {
    let Some(host) = device("reversible laws come home dense") else {
        return;
    };
    let size = [16, 8, 32];
    let grid = grid_of(size);
    for (index, (kind, text, params)) in chosen(12, 0x1D1).into_iter().enumerate() {
        let law = compiled(&text);
        let start = filled(grid, kind.channels, 0xD0 + index as u64, false);
        let far = home_on_cpu(&law, &params, grid, &start);
        home_on_device(&host, &law, &params, size, false, &start, Some(&far));
    }
    for (text, params) in [(MIX, vec![3_u32, 1 << 20]), (STIR, vec![7, 1 << 12, 99])] {
        let law = compiled(text);
        let start = filled(grid, 1, 0x515, false);
        let far = home_on_cpu(&law, &params, grid, &start);
        home_on_device(&host, &law, &params, size, false, &start, Some(&far));
    }
}

#[test]
fn quiet_reversible_laws_come_home_sparse_on_the_device_and_the_cpu_twin() {
    let Some(host) = device("reversible laws come home sparse") else {
        return;
    };
    let size = [32, 16, 32];
    let grid = grid_of(size);
    let mut seed = 0x5_9A25_u64;
    let quiet: Vec<Kind> = kinds()
        .into_iter()
        .filter(|kind| kind.quiet && !kind.looks)
        .collect();
    for (index, kind) in quiet.into_iter().enumerate() {
        seed = next(&mut seed);
        let text = generated(kind, seed, &format!("sparse{index}"));
        let law = compiled(&text);
        let params = params_of(kind, seed);
        assert!(background_class(&law, &params).proved(), "{text}");
        let start = filled(grid, kind.channels, seed, true);
        let far = home_on_cpu(&law, &params, grid, &start);
        home_on_device(&host, &law, &params, size, true, &start, Some(&far));
    }
}

#[test]
fn a_reversible_law_comes_home_on_the_coarse_tier() {
    let Some(host) = device("reversible law comes home coarse") else {
        return;
    };
    let octs = u64::from(grit::live::COARSE_FROM) * u64::from(grit::sweep::cells(host.width()));
    let cells = octs * 8;
    let bits = cells.trailing_zeros();
    let side = 1_u32 << (bits / 3);
    let tall = u32::try_from(cells / u64::from(side) / u64::from(side)).unwrap();
    let size = [side, side, tall];
    let grid = grid_of(size);
    let blocks = grid.octs() / grit::sweep::cells(host.width());
    assert!(grit::live::coarse_pays(blocks), "{size:?}");
    let kind = Kind {
        channels: 2,
        params: 2,
        clock: false,
        quiet: true,
        looks: false,
    };
    let text = generated(kind, 0xC0A25E, "coarse");
    let law = compiled(&text);
    let params = params_of(kind, 0xC0A25E);
    let start = filled(grid, kind.channels, 0xC0, true);
    let mut early = start.clone();
    for frame in 0..4 {
        early = reference(&law, grid, frame, &early, &params);
    }
    let mut world = engine(&host, &law, &params, size, false);
    put(&mut world, grid, kind.channels, &start);
    world.step(4).unwrap();
    assert_eq!(
        world.cells().unwrap(),
        early,
        "coarse device against the twin"
    );
    world.back(4).unwrap();
    assert_eq!(world.cells().unwrap(), start);
    drop(world);
    home_on_device(&host, &law, &params, size, false, &start, None);
    home_on_device(&host, &law, &params, size, true, &start, None);
}

#[test]
fn a_law_that_looks_at_a_frozen_plane_comes_home_across_every_seam() {
    let Some(host) = device("reversible looks come home") else {
        return;
    };
    for size in [[16, 8, 32], [8, 32, 16], [2, 4, 2]] {
        let grid = grid_of(size);
        let mut seed = 0x100C_u64 ^ u64::from(size[0]);
        for (index, kind) in kinds()
            .into_iter()
            .filter(|kind| kind.looks)
            .enumerate()
            .take(6)
        {
            seed = next(&mut seed);
            let text = generated(kind, seed, &format!("looks{index}"));
            let law = compiled(&text);
            assert_eq!(law.halo(), 1);
            let params = params_of(kind, seed);
            let start = filled(grid, kind.channels, seed, false);
            let far = home_on_cpu(&law, &params, grid, &start);
            home_on_device(&host, &law, &params, size, false, &start, Some(&far));
        }
    }
}

#[test]
fn a_law_that_is_not_reversible_is_refused_a_walk_back_before_any_work() {
    let Some(host) = device("not reversible refused") else {
        return;
    };
    let plain = MIX.replacen("\"reversible\": true,", "", 1);
    let law = compiled(&plain);
    assert!(!law.reversible());
    assert!(law.inverse().unwrap().is_none());
    let size = [8, 8, 8];
    let grid = grid_of(size);
    let start = filled(grid, 1, 9, false);
    let mut world = engine(&host, &law, &[1, 2], size, false);
    assert!(!world.reversible());
    put(&mut world, grid, 1, &start);
    world.step(3).unwrap();
    let hash = world.state_hash().unwrap();
    let why = world.back(1).unwrap_err();
    assert!(
        why.downcast_ref::<grit::branch::back::NotReversible>()
            .is_some(),
        "{why}"
    );
    assert_eq!(world.frame(), 3);
    assert_eq!(world.state_hash().unwrap(), hash);
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "grit-reversible3-{label}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn dir(&self) -> PathBuf {
        self.0.join("worlds")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn line(id: i64, name: &str, arguments: &str) -> String {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"tools/call\",\"params\":{{\"name\":\"{name}\",\"arguments\":{arguments}}}}}"
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
        written(got)
    );
    held.get("structuredContent").cloned().unwrap()
}

fn answer(got: &Json) -> Json {
    result(got, false)
}

fn refusal(got: &Json) -> Json {
    result(got, true)
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

struct Run {
    exec: Exec,
    answers: Receiver<Json>,
    id: i64,
    early: BTreeMap<i64, Json>,
}

impl Run {
    fn new(dir: Option<PathBuf>) -> Self {
        let pattern = std::env::var("GRIT_DEVICE")
            .ok()
            .filter(|one| !one.is_empty());
        let (exec, answers) = Exec::spawn(
            Box::new(move || Host::matching(pattern.as_deref())),
            &Options { dir, memory: None },
        )
        .unwrap();
        Self {
            exec,
            answers,
            id: 100,
            early: BTreeMap::new(),
        }
    }

    fn ask(&mut self, name: &str, arguments: &str) -> Json {
        self.id += 1;
        let id = self.id;
        self.exec.line(&line(id, name, arguments));
        if let Some(got) = self.early.remove(&id) {
            return got;
        }
        loop {
            let got = self.answers.recv_timeout(Duration::from_secs(300)).unwrap();
            let came = got.get("id").and_then(Json::int).unwrap();
            if came == id {
                return got;
            }
            self.early.insert(came, got);
        }
    }
}

const SIDE: u32 = 16;

struct Model {
    law: Law,
    params: Vec<u32>,
    cells: Vec<u32>,
    frame: u32,
}

impl Model {
    fn grid() -> Grid3 {
        grid_of([SIDE, SIDE, SIDE])
    }

    fn step(&mut self, frames: u32) {
        for _ in 0..frames {
            self.cells = reference(
                &self.law,
                Self::grid(),
                self.frame,
                &self.cells,
                &self.params,
            );
            self.frame += 1;
        }
    }

    fn back(&mut self, frames: u32) {
        let inverse = self.law.inverse().unwrap().unwrap().clone();
        for _ in 0..frames {
            self.frame -= 1;
            self.cells = reference(
                &inverse,
                Self::grid(),
                self.frame,
                &self.cells,
                &self.params,
            );
        }
    }
}

fn read_all(run: &mut Run, world: i64) -> Vec<u32> {
    let mut cells = Vec::new();
    let last = SIDE - 1;
    for z in (0..SIDE).step_by(8) {
        let got = answer(&run.ask(
            "grit_world_read",
            &format!(
                "{{\"world\":{world},\"from\":[0,0,{z}],\"to\":[{last},{last},{}]}}",
                z + 7
            ),
        ));
        cells.extend(
            got.get("cells")
                .and_then(Json::list)
                .unwrap()
                .iter()
                .map(|one| u32::try_from(one.int().unwrap()).unwrap()),
        );
    }
    cells
}

fn write_all(run: &mut Run, world: i64, cells: &[u32]) {
    let grid = Model::grid();
    let mut placed = Vec::new();
    for z in 0..SIDE {
        for y in 0..SIDE {
            for x in 0..SIDE {
                let word = cells[grid.index(x, y, z)];
                if word != 0 {
                    placed.push(format!("[{x},{y},{z},{word}]"));
                }
            }
        }
    }
    for chunk in placed.chunks(4096) {
        answer(&run.ask(
            "grit_world_write",
            &format!(
                "{{\"world\":{world},\"cells\":[{}],\"budget_ms\":600000}}",
                chunk.join(",")
            ),
        ));
    }
}

fn stepped_by(run: &mut Run, world: i64, frames: u32) -> Json {
    answer(&run.ask(
        "grit_world_step",
        &format!("{{\"world\":{world},\"frames\":{frames},\"budget_ms\":600000}}"),
    ))
}

fn back_by(run: &mut Run, world: i64, frames: u32, home: bool) -> Json {
    answer(&run.ask(
        "grit_world_back",
        &format!("{{\"world\":{world},\"frames\":{frames},\"home\":{home},\"budget_ms\":600000}}"),
    ))
}

fn state_of(run: &mut Run, world: i64, name: &str) -> (String, i64) {
    let got = answer(&run.ask(
        "grit_world_save",
        &format!("{{\"world\":{world},\"name\":\"{name}\",\"replace\":true}}"),
    ));
    (text_at(&got, "state"), int_at(&got, "frame"))
}

fn walking_law() -> (String, Vec<u32>) {
    let kind = Kind {
        channels: 1,
        params: 2,
        clock: false,
        quiet: true,
        looks: false,
    };
    (generated(kind, 0x4A1C, "walker"), params_of(kind, 0x4A1C))
}

fn created(run: &mut Run, text: &str, params: &[u32], sparse: bool) -> i64 {
    let made = answer(&run.ask(
        "grit_world_create",
        &format!(
            "{{\"wide\":{SIDE},\"deep\":{SIDE},\"tall\":{SIDE},\"law\":{},\"params\":{params:?},\"sparse\":{sparse}}}",
            quoted(text)
        ),
    ));
    int_at(&made, "world")
}

#[test]
fn a_walked_back_world_survives_replay_save_open_and_fork_at_its_frame_and_state() {
    let Some(host) = device("walked back world survives") else {
        return;
    };
    drop(host);
    let scratch = Scratch::new("survives");
    let (text, params) = walking_law();
    let law = compiled(&text);
    let start = filled(Model::grid(), 1, 0x77A, true);
    let mut run = Run::new(Some(scratch.dir()));
    let mut held: Vec<(i64, Model)> = Vec::new();
    for sparse in [false, true] {
        let world = created(&mut run, &text, &params, sparse);
        write_all(&mut run, world, &start);
        let mut model = Model {
            law: law.clone(),
            params: params.clone(),
            cells: start.clone(),
            frame: 0,
        };
        stepped_by(&mut run, world, 40);
        model.step(40);
        let walked = back_by(&mut run, world, 25, false);
        assert_eq!(int_at(&walked, "frame"), 15);
        assert_eq!(
            text_at(&walked, "inverse"),
            law.inverse().unwrap().unwrap().hex()
        );
        assert_eq!(walked.get("sparse"), sparse.then_some(&Json::Bool(true)));
        assert_eq!(walked.get("pool_use").is_some(), sparse);
        model.back(25);
        stepped_by(&mut run, world, 9);
        model.step(9);
        let checked = back_by(&mut run, world, 20, true);
        assert_eq!(checked.get("home"), Some(&Json::Bool(true)));
        model.back(20);
        assert_eq!(model.frame, 4);
        assert_eq!(read_all(&mut run, world), model.cells, "world {world}");
        held.push((world, model));
    }
    let mut states = Vec::new();
    for (world, model) in &held {
        let (state, frame) = state_of(&mut run, *world, &format!("walked-{world}"));
        assert_eq!(frame, i64::from(model.frame));
        states.push(state);
    }
    let source = held[0].0;
    let forked = answer(&run.ask("grit_world_fork", &format!("{{\"world\":{source}}}")));
    let fork = int_at(&forked, "world");
    assert_eq!(int_at(&forked, "frame"), 4);
    let (fork_state, _) = state_of(&mut run, fork, "fork");
    assert_eq!(fork_state, states[0]);
    back_by(&mut run, fork, 3, false);
    back_by(&mut run, source, 3, false);
    let (one, _) = state_of(&mut run, fork, "fork-a");
    let (other, _) = state_of(&mut run, source, "fork-b");
    assert_eq!(one, other);
    stepped_by(&mut run, fork, 3);
    stepped_by(&mut run, source, 3);
    stepped_by(&mut run, source, 6);
    back_by(&mut run, fork, 2, false);
    let restored = answer(&run.ask(
        "grit_world_restore",
        &format!("{{\"world\":{source},\"from\":{fork}}}"),
    ));
    assert_eq!(int_at(&restored, "frame"), 2);
    let (taken, _) = state_of(&mut run, source, "restored");
    let (given, _) = state_of(&mut run, fork, "given");
    assert_eq!(taken, given);
    back_by(&mut run, source, 2, true);
    back_by(&mut run, fork, 2, true);
    let (one, frame) = state_of(&mut run, source, "restored-a");
    let (other, _) = state_of(&mut run, fork, "restored-b");
    assert_eq!(one, other);
    assert_eq!(frame, 0);
    assert_eq!(read_all(&mut run, source), start);
    stepped_by(&mut run, source, 4);
    assert_eq!(read_all(&mut run, source), held[0].1.cells);
    run.exec.finish();

    let mut again = Run::new(Some(scratch.dir()));
    for ((world, model), state) in held.iter().zip(&states) {
        let opened = answer(&again.ask(
            "grit_world_open",
            &format!("{{\"name\":\"world-{world}\"}}"),
        ));
        assert_eq!(int_at(&opened, "frame"), i64::from(model.frame));
        assert_eq!(
            read_all(&mut again, *world),
            model.cells,
            "kept world {world}"
        );
        let (replayed, frame) = state_of(&mut again, *world, &format!("again-{world}"));
        assert_eq!(&replayed, state, "kept world {world}");
        assert_eq!(frame, i64::from(model.frame));
        let file = answer(&again.ask(
            "grit_world_open",
            &format!("{{\"name\":\"walked-{world}\"}}"),
        ));
        let opened_world = int_at(&file, "world");
        assert_eq!(int_at(&file, "frame"), i64::from(model.frame));
        let (from_file, _) = state_of(&mut again, opened_world, &format!("file-{world}"));
        assert_eq!(&from_file, state, "saved world {world}");
        back_by(&mut again, opened_world, 4, true);
        let mut back = Model {
            law: model.law.clone(),
            params: model.params.clone(),
            cells: model.cells.clone(),
            frame: model.frame,
        };
        back.back(4);
        assert_eq!(read_all(&mut again, opened_world), back.cells);
        assert_eq!(back.cells, start);
        let past = refusal(&again.ask(
            "grit_world_back",
            &format!("{{\"world\":{opened_world},\"frames\":1}}"),
        ));
        assert_eq!(text_at(&past, "code"), "past_start", "{}", written(&past));
        assert_eq!(int_at(&past, "limit"), 0);
    }
    again.exec.finish();
}

#[test]
fn a_world_that_is_not_reversible_is_refused_a_walk_back_by_name() {
    let Some(host) = device("not reversible tool") else {
        return;
    };
    drop(host);
    let mut run = Run::new(None);
    let plain = MIX.replacen("\"reversible\": true,", "", 1);
    for (arguments, rule) in [
        (
            format!(
                "{{\"wide\":8,\"deep\":8,\"tall\":8,\"law\":{},\"params\":[1,2]}}",
                quoted(&plain)
            ),
            "mix",
        ),
        (
            format!(
                "{{\"wide\":8,\"deep\":8,\"tall\":8,\"law\":{},\"params\":[1,2,3]}}",
                quoted(HEAT)
            ),
            "heat",
        ),
        (
            "{\"wide\":8,\"deep\":8,\"tall\":8,\"rule\":\"sand\",\"params\":[]}".to_owned(),
            "sand",
        ),
    ] {
        let made = run.ask("grit_world_create", &arguments);
        let Some(world) = made
            .get("result")
            .and_then(|one| one.get("structuredContent"))
            .and_then(|one| one.get("world"))
            .and_then(Json::int)
        else {
            panic!("{}", written(&made));
        };
        stepped_by(&mut run, world, 5);
        let before = read_all_small(&mut run, world);
        let told = refusal(&run.ask(
            "grit_world_back",
            &format!("{{\"world\":{world},\"frames\":2}}"),
        ));
        assert_eq!(
            text_at(&told, "code"),
            "not_reversible",
            "{}",
            written(&told)
        );
        assert_eq!(text_at(&told, "field"), "$.world");
        assert!(
            text_at(&told, "message").contains(rule),
            "{}",
            written(&told)
        );
        assert!(text_at(&told, "message").contains("nothing ran"));
        let listed = answer(&run.ask("grit_world_list", "{}"));
        let item = listed
            .get("items")
            .and_then(Json::list)
            .unwrap()
            .iter()
            .find(|one| one.get("world").and_then(Json::int) == Some(world))
            .cloned()
            .unwrap();
        assert_eq!(int_at(&item, "frame"), 5);
        assert_eq!(read_all_small(&mut run, world), before);
    }
    run.exec.finish();
}

fn read_all_small(run: &mut Run, world: i64) -> Vec<u32> {
    let got = answer(&run.ask(
        "grit_world_read",
        &format!("{{\"world\":{world},\"from\":[0,0,0],\"to\":[7,7,7]}}"),
    ));
    got.get("cells")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .map(|one| u32::try_from(one.int().unwrap()).unwrap())
        .collect()
}

#[test]
fn the_back_tool_is_listed_described_and_checked_against_its_schema() {
    let mut server = Server::new(None);
    let listed = server
        .heard("{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}")
        .unwrap();
    let tools = listed
        .get("result")
        .and_then(|one| one.get("tools"))
        .and_then(Json::list)
        .unwrap();
    let back = tools
        .iter()
        .find(|one| one.get("name").and_then(Json::text) == Some("grit_world_back"))
        .unwrap();
    let schema = back.get("inputSchema").unwrap();
    let properties = schema.get("properties").unwrap();
    for key in ["world", "frames", "home", "budget_ms"] {
        assert!(properties.get(key).is_some(), "{key}");
    }
    let described = answer(&server.heard(&line(2, "grit_describe", "{}")).unwrap());
    let codes: Vec<String> = described
        .get("codes")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .map(|one| text_at(one, "code"))
        .collect();
    for code in ["not_reversible", "past_start", "strayed"] {
        assert!(codes.iter().any(|one| one == code), "{code}");
    }
    let limits = described.get("limits").unwrap();
    assert!(text_at(limits, "law_reversible").contains("not_lifting"));
    for (arguments, code) in [
        ("{\"world\":1,\"frames\":0}", "frames"),
        ("{\"world\":1,\"frames\":65537}", "frames"),
        ("{\"world\":1,\"frame\":3}", "unknown_key"),
        ("{\"world\":1,\"home\":1}", "malformed"),
        ("{\"world\":1}", "no_world"),
    ] {
        let told = refusal(
            &server
                .heard(&line(3, "grit_world_back", arguments))
                .unwrap(),
        );
        assert_eq!(
            text_at(&told, "code"),
            code,
            "{arguments}: {}",
            written(&told)
        );
    }
}

const SWAP: &str = r#"{"law":1,"name":"drift","params":0,"channels":1,"halo":0,"reversible":true,"tally":{"counts":1,"weighted":false},"consts":{},"defs":{},"step":["let",[["a0",["at","cells",0]],["b0",["at","cells",1]],["a2",["at","cells",2]],["b2",["at","cells",3]],["a4",["at","cells",4]],["b4",["at","cells",5]],["a6",["at","cells",6]],["b6",["at","cells",7]],["a0x",["xor","a0","b0"]],["b0x",["xor","b0","a0x"]],["a0y",["xor","a0x","b0x"]],["a2x",["xor","a2","b2"]],["b2x",["xor","b2","a2x"]],["a2y",["xor","a2x","b2x"]],["a4x",["xor","a4","b4"]],["b4x",["xor","b4","a4x"]],["a4y",["xor","a4x","b4x"]],["a6x",["xor","a6","b6"]],["b6x",["xor","b6","a6x"]],["a6y",["xor","a6x","b6x"]]],["vec",["vec","a0y","b0x","a2y","b2x","a4y","b4x","a6y","b6x"],["vec",["word",["ne","a0","b0"]]]]]}"#;

fn swap_world(run: &mut Run) -> i64 {
    let made = answer(&run.ask(
        "grit_world_create",
        &format!(
            "{{\"wide\":256,\"deep\":32,\"tall\":32,\"law\":{},\"params\":[],\"sparse\":true,\"slots\":4}}",
            quoted(SWAP)
        ),
    ));
    let world = int_at(&made, "world");
    answer(&run.ask(
        "grit_world_write",
        &format!("{{\"world\":{world},\"cells\":[[0,0,0,7]]}}"),
    ));
    world
}

fn params_by(run: &mut Run, world: i64, params: &[u32]) {
    answer(&run.ask(
        "grit_world_params",
        &format!("{{\"world\":{world},\"params\":{params:?}}}"),
    ));
}

#[test]
fn a_journal_of_writes_steps_params_and_backs_replays_to_the_live_world() {
    let Some(host) = device("journal with backs replays") else {
        return;
    };
    drop(host);
    let scratch = Scratch::new("journal-backs");
    let (text, params) = walking_law();
    let law = compiled(&text);
    let start = filled(Model::grid(), 1, 0x10A, true);
    let mut run = Run::new(Some(scratch.dir()));
    let mut held = Vec::new();
    for sparse in [false, true] {
        let world = created(&mut run, &text, &params, sparse);
        write_all(&mut run, world, &start);
        stepped_by(&mut run, world, 30);
        back_by(&mut run, world, 10, false);
        let other = created(&mut run, &text, &params, sparse);
        write_all(&mut run, other, &start);
        stepped_by(&mut run, other, 20);
        let changed = [params[0] ^ 0x55, params[1].wrapping_add(3)];
        params_by(&mut run, other, &changed);
        back_by(&mut run, other, 15, false);
        let mut model = Model {
            law: law.clone(),
            params: params.clone(),
            cells: start.clone(),
            frame: 0,
        };
        model.step(20);
        model.params = changed.to_vec();
        model.back(15);
        assert_eq!(
            read_all(&mut run, other),
            model.cells,
            "[write, step, params, back]"
        );
        for one in [world, other] {
            let (state, frame) = state_of(&mut run, one, &format!("live-{one}"));
            held.push((one, state, frame, read_all(&mut run, one)));
        }
    }
    run.exec.finish();
    let mut again = Run::new(Some(scratch.dir()));
    for (world, state, frame, cells) in &held {
        let opened = answer(&again.ask(
            "grit_world_open",
            &format!("{{\"name\":\"world-{world}\"}}"),
        ));
        assert_eq!(int_at(&opened, "frame"), *frame, "world {world}");
        let (replayed, at) = state_of(&mut again, *world, &format!("replayed-{world}"));
        assert_eq!(&replayed, state, "world {world} replayed its journal");
        assert_eq!(at, *frame);
        assert_eq!(&read_all(&mut again, *world), cells, "world {world}");
    }
    again.exec.finish();
}

#[test]
fn a_walk_back_that_overflows_its_pool_is_refused_and_keeps_the_world() {
    let Some(host) = device("walk back overflows its pool") else {
        return;
    };
    drop(host);
    let scratch = Scratch::new("overflow");
    let mut run = Run::new(Some(scratch.dir()));
    let world = swap_world(&mut run);
    stepped_by(&mut run, world, 240);
    answer(&run.ask(
        "grit_world_write",
        &format!("{{\"world\":{world},\"cells\":[[31,0,0,9],[95,8,8,10]]}}"),
    ));
    let (before, frame) = state_of(&mut run, world, "before");
    assert_eq!(frame, 240);
    let told = refusal(&run.ask(
        "grit_world_back",
        &format!("{{\"world\":{world},\"frames\":240,\"budget_ms\":600000}}"),
    ));
    assert_eq!(text_at(&told, "code"), "pool_full", "{}", written(&told));
    let message = text_at(&told, "message");
    assert!(message.contains("nothing was journaled"), "{message}");
    assert!(message.contains("the world is at frame 240"), "{message}");
    let (after, frame) = state_of(&mut run, world, "after");
    assert_eq!(frame, 240);
    assert_eq!(after, before, "the world is left as it was before the walk");
    stepped_by(&mut run, world, 2);
    back_by(&mut run, world, 2, true);
    run.exec.finish();
    let mut again = Run::new(Some(scratch.dir()));
    answer(&again.ask(
        "grit_world_open",
        &format!("{{\"name\":\"world-{world}\"}}"),
    ));
    let (kept, frame) = state_of(&mut again, world, "kept");
    assert_eq!(frame, 240);
    assert_eq!(kept, before);
    again.exec.finish();
}

#[cfg(feature = "faults")]
mod faulted {
    use super::*;

    #[test]
    fn a_walk_stopped_by_its_budget_then_walked_home_comes_home() {
        let Some(host) = device("budget stop then home") else {
            return;
        };
        let mut server = Server::new(Some(&host));
        let (text, params) = walking_law();
        let call = |server: &mut Server<'_>, name: &str, arguments: String| {
            server.heard(&line(9, name, &arguments)).unwrap()
        };
        let made = answer(&call(
            &mut server,
            "grit_world_create",
            format!(
                "{{\"wide\":{SIDE},\"deep\":{SIDE},\"tall\":{SIDE},\"law\":{},\"params\":{params:?},\"sparse\":true}}",
                quoted(&text)
            ),
        ));
        let world = int_at(&made, "world");
        let start = filled(Model::grid(), 1, 0xB0E, true);
        let grid = Model::grid();
        let mut placed = Vec::new();
        for z in 0..SIDE {
            for y in 0..SIDE {
                for x in 0..SIDE {
                    let word = start[grid.index(x, y, z)];
                    if word != 0 {
                        placed.push(format!("[{x},{y},{z},{word}]"));
                    }
                }
            }
        }
        answer(&call(
            &mut server,
            "grit_world_write",
            format!("{{\"world\":{world},\"cells\":[{}]}}", placed.join(",")),
        ));
        answer(&call(
            &mut server,
            "grit_world_step",
            format!("{{\"world\":{world},\"frames\":300,\"budget_ms\":600000}}"),
        ));
        grit::serve3::tools::faults::stall_every(Duration::from_millis(5));
        let stopped = answer(&call(
            &mut server,
            "grit_world_back",
            format!("{{\"world\":{world},\"frames\":300,\"budget_ms\":300}}"),
        ));
        grit::serve3::tools::faults::disarm();
        assert_eq!(
            text_at(&stopped, "stopped"),
            "budget",
            "{}",
            written(&stopped)
        );
        let ran = u32::try_from(int_at(&stopped, "ran")).unwrap();
        assert!(ran < 300);
        let home = answer(&call(
            &mut server,
            "grit_world_back",
            format!(
                "{{\"world\":{world},\"frames\":{},\"home\":true,\"budget_ms\":600000}}",
                300 - ran
            ),
        ));
        assert_eq!(
            home.get("home"),
            Some(&Json::Bool(true)),
            "{}",
            written(&home)
        );
        assert_eq!(int_at(&home, "frame"), 0);
        let last = SIDE - 1;
        let mut cells = Vec::new();
        for z in (0..SIDE).step_by(8) {
            let got = answer(&call(
                &mut server,
                "grit_world_read",
                format!(
                    "{{\"world\":{world},\"from\":[0,0,{z}],\"to\":[{last},{last},{}]}}",
                    z + 7
                ),
            ));
            cells.extend(
                got.get("cells")
                    .and_then(Json::list)
                    .unwrap()
                    .iter()
                    .map(|one| u32::try_from(one.int().unwrap()).unwrap()),
            );
        }
        assert_eq!(cells, start);
    }

    #[test]
    fn a_lost_device_during_and_after_a_sparse_walk_back_keeps_the_world_exact() {
        let Some(host) = device("lost device sparse walk") else {
            return;
        };
        drop(host);
        let scratch = Scratch::new("lost-sparse");
        let mut run = Run::new(Some(scratch.dir()));
        let world = swap_world(&mut run);
        let (start, _) = state_of(&mut run, world, "start");
        stepped_by(&mut run, world, 240);
        let (far, frame) = state_of(&mut run, world, "far");
        assert_eq!(frame, 240);
        run.exec.faults().lock().unwrap().wedge_after_batch = Some(0);
        let lost = refusal(&run.ask(
            "grit_world_back",
            &format!("{{\"world\":{world},\"frames\":240,\"budget_ms\":600000}}"),
        ));
        assert_eq!(text_at(&lost, "code"), "device_lost", "{}", written(&lost));
        assert!(
            text_at(&lost, "message").contains("0 of the 240 frames were walked back"),
            "{}",
            written(&lost)
        );
        let (during, frame) = state_of(&mut run, world, "during");
        assert_eq!(frame, 240);
        assert_eq!(
            during, far,
            "a walk lost with the device leaves the world where it was"
        );
        back_by(&mut run, world, 240, false);
        let (walked, frame) = state_of(&mut run, world, "walked");
        assert_eq!(frame, 0);
        assert_eq!(walked, start);
        run.exec.faults().lock().unwrap().wedge_before_call = true;
        let told = refusal(&run.ask(
            "grit_world_read",
            &format!("{{\"world\":{world},\"from\":[0,0,0],\"to\":[0,0,0]}}"),
        ));
        assert_eq!(text_at(&told, "code"), "device_lost", "{}", written(&told));
        let (rebuilt, frame) = state_of(&mut run, world, "rebuilt");
        assert_eq!(frame, 0);
        assert_eq!(
            rebuilt, start,
            "rebuilt from a journal that ends in a walk back"
        );
        stepped_by(&mut run, world, 240);
        let (again, _) = state_of(&mut run, world, "again");
        assert_eq!(again, far);
        run.exec.finish();
    }

    #[test]
    fn a_walked_back_world_is_rebuilt_exactly_after_a_lost_device() {
        let Some(host) = device("walked back world lost device") else {
            return;
        };
        drop(host);
        let scratch = Scratch::new("lost");
        let (text, params) = walking_law();
        let law = compiled(&text);
        let start = filled(Model::grid(), 1, 0x1057, true);
        let mut run = Run::new(Some(scratch.dir()));
        let mut held = Vec::new();
        for sparse in [false, true] {
            let world = created(&mut run, &text, &params, sparse);
            write_all(&mut run, world, &start);
            let mut model = Model {
                law: law.clone(),
                params: params.clone(),
                cells: start.clone(),
                frame: 0,
            };
            stepped_by(&mut run, world, 60);
            model.step(60);
            back_by(&mut run, world, 21, false);
            model.back(21);
            held.push((world, model));
        }
        let mut states = Vec::new();
        for (world, model) in &held {
            let (state, frame) = state_of(&mut run, *world, &format!("before-{world}"));
            assert_eq!(frame, i64::from(model.frame));
            states.push(state);
        }
        {
            let faults = run.exec.faults();
            faults.lock().unwrap().wedge_before_call = true;
        }
        let first = held[0].0;
        let told = refusal(&run.ask(
            "grit_world_back",
            &format!("{{\"world\":{first},\"frames\":2}}"),
        ));
        assert_eq!(text_at(&told, "code"), "device_lost", "{}", written(&told));
        for ((world, model), state) in held.iter().zip(&states) {
            assert_eq!(read_all(&mut run, *world), model.cells, "world {world}");
            let (rebuilt, frame) = state_of(&mut run, *world, &format!("after-{world}"));
            assert_eq!(&rebuilt, state, "world {world} after the lost device");
            assert_eq!(frame, i64::from(model.frame));
        }
        let (world, model) = &mut held[1];
        let world = *world;
        {
            let faults = run.exec.faults();
            faults.lock().unwrap().wedge_after_batch = Some(0);
        }
        let lost = refusal(&run.ask(
            "grit_world_back",
            &format!("{{\"world\":{world},\"frames\":30,\"budget_ms\":600000}}"),
        ));
        assert_eq!(text_at(&lost, "code"), "device_lost", "{}", written(&lost));
        let message = text_at(&lost, "message");
        let ran: u32 = message
            .split(" of the 30 frames were walked back")
            .next()
            .and_then(|head| head.rsplit(' ').next())
            .and_then(|count| count.parse().ok())
            .unwrap_or_else(|| panic!("{message}"));
        assert_eq!(ran, 0, "{message}");
        assert!(message.contains("the world is at frame 39"), "{message}");
        model.back(ran);
        assert_eq!(
            read_all(&mut run, world),
            model.cells,
            "after a lost walk back"
        );
        back_by(&mut run, world, model.frame, true);
        model.back(model.frame);
        assert_eq!(model.cells, start);
        assert_eq!(read_all(&mut run, world), start);
        run.exec.finish();
    }

    #[test]
    fn a_strayed_walk_is_answered_strayed_and_leaves_the_world_as_it_was() {
        let Some(host) = device("strayed walk") else {
            return;
        };
        drop(host);
        let mut run = Run::new(None);
        let (text, params) = walking_law();
        let world = created(&mut run, &text, &params, false);
        let start = filled(Model::grid(), 1, 0x57A, false);
        write_all(&mut run, world, &start);
        stepped_by(&mut run, world, 12);
        let before = read_all(&mut run, world);
        run.exec.faults().lock().unwrap().stray_walk = true;
        let told = refusal(&run.ask(
            "grit_world_back",
            &format!("{{\"world\":{world},\"frames\":5,\"home\":true}}"),
        ));
        assert_eq!(text_at(&told, "code"), "strayed", "{}", written(&told));
        let strayed = told.get("strayed").unwrap();
        assert_eq!(strayed.get("cell").and_then(Json::list).unwrap().len(), 3);
        assert_eq!(read_all(&mut run, world), before);
        let listed = answer(&run.ask("grit_world_list", "{}"));
        let item = listed
            .get("items")
            .and_then(Json::list)
            .unwrap()
            .iter()
            .find(|one| one.get("world").and_then(Json::int) == Some(world))
            .cloned()
            .unwrap();
        assert_eq!(int_at(&item, "frame"), 12);
        run.exec.finish();
    }

    #[test]
    fn a_walk_back_stopped_by_its_budget_keeps_every_frame_it_walked() {
        let Some(host) = device("walk back budget") else {
            return;
        };
        let mut server = Server::new(Some(&host));
        let (text, params) = walking_law();
        let law = compiled(&text);
        let made = answer(
            &server
                .heard(&line(
                    1,
                    "grit_world_create",
                    &format!(
                        "{{\"wide\":{SIDE},\"deep\":{SIDE},\"tall\":{SIDE},\"law\":{},\"params\":{params:?}}}",
                        quoted(&text)
                    ),
                ))
                .unwrap(),
        );
        let world = int_at(&made, "world");
        let start = filled(Model::grid(), 1, 0xB0D, true);
        let mut placed = Vec::new();
        let grid = Model::grid();
        for z in 0..SIDE {
            for y in 0..SIDE {
                for x in 0..SIDE {
                    let word = start[grid.index(x, y, z)];
                    if word != 0 {
                        placed.push(format!("[{x},{y},{z},{word}]"));
                    }
                }
            }
        }
        answer(
            &server
                .heard(&line(
                    2,
                    "grit_world_write",
                    &format!("{{\"world\":{world},\"cells\":[{}]}}", placed.join(",")),
                ))
                .unwrap(),
        );
        let mut model = Model {
            law,
            params,
            cells: start,
            frame: 0,
        };
        answer(
            &server
                .heard(&line(
                    3,
                    "grit_world_step",
                    &format!("{{\"world\":{world},\"frames\":300,\"budget_ms\":600000}}"),
                ))
                .unwrap(),
        );
        model.step(300);
        grit::serve3::tools::faults::stall_every(Duration::from_millis(5));
        let walked = answer(
            &server
                .heard(&line(
                    4,
                    "grit_world_back",
                    &format!("{{\"world\":{world},\"frames\":300,\"budget_ms\":300}}"),
                ))
                .unwrap(),
        );
        grit::serve3::tools::faults::disarm();
        assert_eq!(
            text_at(&walked, "stopped"),
            "budget",
            "{}",
            written(&walked)
        );
        let ran = u32::try_from(int_at(&walked, "ran")).unwrap();
        assert!(ran < 300, "{}", written(&walked));
        assert_eq!(int_at(&walked, "frame"), i64::from(300 - ran));
        model.back(ran);
        let last = SIDE - 1;
        let got = answer(
            &server
                .heard(&line(
                    5,
                    "grit_world_read",
                    &format!("{{\"world\":{world},\"from\":[0,0,0],\"to\":[{last},{last},7]}}"),
                ))
                .unwrap(),
        );
        let cells: Vec<u32> = got
            .get("cells")
            .and_then(Json::list)
            .unwrap()
            .iter()
            .map(|one| u32::try_from(one.int().unwrap()).unwrap())
            .collect();
        assert_eq!(cells, model.cells[..cells.len()]);
    }
}

const SHEAR: &str = r#"{"law":1,"name":"shear","params":1,"channels":1,"halo":0,"reversible":true,"tally":{"counts":1,"weighted":false},"consts":{},"defs":{},
"step":["let",[["a",["at","cells",0]],["b",["at","cells",1]],["b2",["add","b",["mul","a",["param",0]]]],["a2",["xor","a",["shr","b2",3]]]],
["vec",["vec","a2","b2",["at","cells",2],["at","cells",3],["at","cells",4],["at","cells",5],["at","cells",6],["at","cells",7]],["vec",0]]]}"#;

#[test]
fn the_documented_example_is_a_reversible_law_that_comes_home() {
    let docs = include_str!("../docs/law-worlds.md").replace("\r\n", "\n");
    assert!(
        docs.contains(SHEAR),
        "docs/law-worlds.md holds the example verbatim"
    );
    let law = compiled(SHEAR);
    assert_eq!(law.inverse().unwrap().unwrap().name(), "shear_back");
    let grid = grid_of([8, 8, 8]);
    let start = filled(grid, 1, 0x5EA, false);
    home_on_cpu(&law, &[3], grid, &start);
}

fn bounded_law(params: u32, defs: &str, step: &str) -> String {
    format!(
        "{{\"law\":1,\"name\":\"probe\",\"params\":{params},\"channels\":1,\"halo\":0,\"tally\":{{\"counts\":1,\"weighted\":false}},\"consts\":{{}},\"defs\":{{{defs}}},\"step\":{step}}}"
    )
}
fn lifted_law(lifts: u32) -> String {
    let cells: Vec<String> = (0..8)
        .map(|k| match k {
            0 => format!(
                "[\"fold\",\"w\",[\"at\",\"cells\",0],\"i\",[\"range\",{lifts}],[\"add\",\"w\",\"b\"]]"
            ),
            _ => format!("[\"at\",\"cells\",{k}]"),
        })
        .collect();
    let shared = limits::NODES as u32 - lifts - 40;
    bounded_law(
        1,
        "",
        &format!(
            "[\"let\",[[\"b\",[\"fold\",\"a\",[\"param\",0],\"i\",[\"range\",{shared}],[\"not\",\"a\"]]]],[\"vec\",[\"vec\",{}],[\"vec\",0]]]",
            cells.join(",")
        ),
    )
    .replacen("\"halo\":0,", "\"halo\":0,\"reversible\":true,", 1)
}
fn mixed_law(steps: u32, seed: u64) -> String {
    let mut state = seed;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let mut current: Vec<String> = (0..8).map(|k| format!("[\"at\",\"cells\",{k}]")).collect();
    let mut binds = Vec::new();
    for step in 0..steps {
        let word = (next() % 8) as usize;
        let others: Vec<&String> = (0..8)
            .filter(|&other| other != word)
            .map(|other| &current[other])
            .collect();
        let one = others[(next() % 7) as usize];
        let two = others[(next() % 7) as usize];
        let by = format!("[\"add\",{one},{two}]");
        let made = if next() % 2 == 0 {
            format!(
                "[\"{}\",{by},{}]",
                ["add", "xor"][(next() % 2) as usize],
                current[word]
            )
        } else {
            format!(
                "[\"{}\",{},{by}]",
                ["add", "sub", "xor"][(next() % 3) as usize],
                current[word]
            )
        };
        binds.push(format!("[\"w{step}\",{made}]"));
        current[word] = format!("\"w{step}\"");
    }
    bounded_law(
        0,
        "",
        &format!(
            "[\"let\",[{}],[\"vec\",[\"vec\",{}],[\"vec\",0]]]",
            binds.join(","),
            current.join(",")
        ),
    )
    .replacen("\"halo\":0,", "\"halo\":0,\"reversible\":true,", 1)
}

const NANOS_PER_UNIT: f64 = if cfg!(debug_assertions) {
    4000.0
} else {
    400.0
};

#[test]
fn the_inverse_of_the_largest_laws_is_derived_in_time_bounded_per_unit_of_the_inverse() {
    let texts: Vec<(String, String)> = [2_000, 4_000, 8_000]
        .into_iter()
        .map(|lifts| {
            (
                format!("{lifts} lifts over a shared subtree"),
                lifted_law(lifts),
            )
        })
        .chain(
            [(2_000_u32, 1_u64), (8_100, 2)]
                .into_iter()
                .map(|(steps, seed)| {
                    (
                        format!("{steps} steps reading each other"),
                        mixed_law(steps, seed),
                    )
                }),
        )
        .collect();
    for (name, text) in texts {
        let mut took = Duration::MAX;
        let mut units = 0;
        for _ in 0..3 {
            let law = compiled(&text);
            let began = std::time::Instant::now();
            let inverse = law.inverse().unwrap().unwrap();
            took = took.min(began.elapsed());
            units = inverse.work() + inverse.nodes() as u64 + inverse.canonical().len() as u64;
        }
        let nanos = took.as_secs_f64() * 1e9 / units as f64;
        println!(
            "{name}: inverse derived in {:.2} ms, {units} units of the inverse, {nanos:.1} ns a unit",
            took.as_secs_f64() * 1e3
        );
        assert!(nanos <= NANOS_PER_UNIT, "{name}: {nanos:.1} ns a unit");
    }
}
