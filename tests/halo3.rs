#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use common::device;

use grit::branch::{self, Made, Stepping, World};
use grit::compact::Plan;
use grit::geometry::Solid;
use grit::halo::{BOX, Halo};
use grit::law::quiet::{self, Class, Why};
use grit::law::{Code, Law, Refusal};
use grit::layout::{DENSE, Dimension, GRAND, PARAMS, PIXELS, STATE, Windows};
use grit::live::{self, Placed3};
use grit::rule3::{CORNERS, Declared, FIRST_PARAM, Oct, Rule};
use grit::sand::Grid;
use grit::sand3::{self, Grid3, lattice};
use grit::serve3::{Options, Server};
use grit::wire::{Json, read, written};
use grit::{Groups, Host, Resident, sweep};

const EDGE: &str = include_str!("halo3/edge2d.law.json");
const SOBEL: &str = include_str!("halo3/sobel3.law.json");
const BLUR: &str = include_str!("halo3/blur2.law.json");
const KERNELS: &str = include_str!("halo3/kernels.txt");

const FRAMES: u32 = 512;

const WRITES_EVERY: u32 = 16;

const SHAPE: (u32, u32, u32) = (16, 16, 8);

const TIERS: [bool; 2] = [false, true];

#[derive(Clone, Copy, Debug)]
enum Seam {
    X,
    Y,
    Z,
}

const SEAMS: [Seam; 3] = [Seam::X, Seam::Y, Seam::Z];

fn compiled(text: &str) -> Law {
    Law::compile(text).unwrap_or_else(|why| panic!("{why}"))
}

fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut mixed = *state;
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    mixed ^ (mixed >> 31)
}

fn below(state: &mut u64, most: u32) -> u32 {
    (next(state) % u64::from(most)) as u32
}

struct Subject {
    name: &'static str,
    text: &'static str,
    params: Vec<u32>,
    word: fn(&mut u64, usize) -> u32,
}

fn binary(state: &mut u64, _plane: usize) -> u32 {
    u32::from(next(state).is_multiple_of(3))
}

fn byte(state: &mut u64, _plane: usize) -> u32 {
    below(state, 256)
}

fn subjects() -> Vec<Subject> {
    vec![
        Subject {
            name: "edge2d",
            text: EDGE,
            params: vec![2],
            word: binary,
        },
        Subject {
            name: "sobel3",
            text: SOBEL,
            params: vec![5],
            word: binary,
        },
        Subject {
            name: "blur2",
            text: BLUR,
            params: Vec::new(),
            word: byte,
        },
    ]
}

fn grids((wide, deep, tall): (u32, u32, u32)) -> (Grid, Grid3) {
    (
        Grid::solid(wide, deep, tall).unwrap(),
        Grid3::new(wide, deep, tall).unwrap(),
    )
}

fn near_seam(grid: Grid3, seam: Seam, state: &mut u64) -> [u32; 3] {
    let mut at = [
        below(state, grid.wide),
        below(state, grid.deep),
        below(state, grid.tall),
    ];
    let (axis, extent) = match seam {
        Seam::X => (0, grid.wide),
        Seam::Y => (1, grid.deep),
        Seam::Z => (2, grid.tall),
    };
    at[axis] = [0, 1, extent - 2, extent - 1][below(state, 4) as usize];
    at
}

fn start(subject: &Subject, grid: Grid3, seam: Seam, seed: u64) -> Vec<u32> {
    let planes = compiled(subject.text).planes();
    let mut state = seed;
    let cells = grid.cells();
    let mut out = vec![0_u32; cells * planes];
    for plane in 0..planes {
        for _ in 0..cells / 6 {
            let [x, y, z] = near_seam(grid, seam, &mut state);
            out[plane * cells + grid.index(x, y, z)] = (subject.word)(&mut state, plane);
        }
        for _ in 0..cells / 32 {
            let at = below(&mut state, u32::try_from(cells).unwrap()) as usize;
            out[plane * cells + at] = (subject.word)(&mut state, plane);
        }
    }
    out
}

fn gathered(grid: Grid3, frame: u32, oct: u32, plane: &[u32]) -> [u32; BOX] {
    let [bx, by, bz] = lattice(grid, oct);
    let phase = frame & 1;
    let least = [2 * bx + phase, 2 * by + phase, 2 * bz + phase];
    let mut out = [0_u32; BOX];
    for z in 0..4_u32 {
        for y in 0..4_u32 {
            for x in 0..4_u32 {
                let place = grid.index(
                    (least[0] + x).wrapping_sub(1),
                    (least[1] + y).wrapping_sub(1),
                    (least[2] + z).wrapping_sub(1),
                );
                out[(x + 4 * y + 16 * z) as usize] = plane[place];
            }
        }
    }
    out
}

fn oct_step(
    law: &Law,
    grid: Grid3,
    frame: u32,
    oct: u32,
    read: &[u32],
    params: &[u32],
) -> ([usize; CORNERS], Vec<[u32; CORNERS]>) {
    let cells = grid.cells();
    let planes = law.planes();
    let places = grid.corners(frame, oct);
    let words = |plane: usize| places.map(|place| read[plane * cells + place]);
    let upper: Vec<[u32; CORNERS]> = (1..planes).map(words).collect();
    let halo: Vec<[u32; BOX]> = (0..planes)
        .map(|plane| gathered(grid, frame, oct, &read[plane * cells..(plane + 1) * cells]))
        .collect();
    let planed = law.step_halo(
        Oct {
            words: words(0),
            frame,
            at: lattice(grid, oct),
            params,
        },
        &upper,
        &halo,
    );
    let mut out = vec![planed.ruled.out];
    out.extend(planed.planes);
    (places, out)
}

struct Model {
    law: Law,
    grid: Grid3,
    solid: Grid,
    params: Vec<u32>,
    cells: Vec<u32>,
    awake: Vec<u8>,
    list: Vec<bool>,
}

impl Model {
    fn new(law: &Law, shape: (u32, u32, u32), params: &[u32], cells: &[u32]) -> Self {
        let (solid, grid) = grids(shape);
        let octs = grid.octs() as usize;
        Self {
            law: law.clone(),
            grid,
            solid,
            params: params.to_vec(),
            cells: cells.to_vec(),
            awake: vec![2; octs],
            list: vec![true; octs],
        }
    }

    fn listed(&mut self, frame: u32) {
        for mark in &mut self.awake {
            *mark = mark.saturating_sub(1);
        }
        let cells = self.grid.cells();
        let start = self.cells.clone();
        let mut changed = Vec::new();
        for oct in 0..self.grid.octs() {
            if !self.list[oct as usize] {
                continue;
            }
            let (places, out) = oct_step(&self.law, self.grid, frame, oct, &start, &self.params);
            let mut moved = false;
            for (plane, words) in out.iter().enumerate() {
                for (place, &word) in places.iter().zip(words) {
                    let slot = &mut self.cells[plane * cells + place];
                    moved |= *slot != word;
                    *slot = word;
                }
            }
            if moved {
                changed.push(oct);
            }
        }
        for oct in changed {
            for near in Solid::woken(self.solid, oct) {
                self.awake[near as usize] = 2;
            }
        }
        for (slot, &mark) in self.list.iter_mut().zip(&self.awake) {
            *slot = mark > 0;
        }
    }

    fn write(&mut self, plane: usize, cell: u32, word: u32) {
        self.cells[plane * self.grid.cells() + cell as usize] = word;
        for phase in 0..2 {
            let holder = Solid::held(self.solid, cell, phase).index;
            for near in Solid::woken(self.solid, holder) {
                self.awake[near as usize] = 2;
                self.list[near as usize] = true;
            }
        }
    }

    fn octs(&self) -> Vec<u32> {
        (0..self.grid.octs())
            .filter(|&oct| self.list[oct as usize])
            .collect()
    }
}

fn hashed(words: &[u32]) -> [u8; 32] {
    let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
    branch::sha256(&bytes)
}

struct Engine<'host> {
    live: Resident<'host>,
    full: Resident<'host>,
    stages: Vec<Vec<Groups>>,
    whole: [Groups; 2],
    plan: Plan,
    grid: Grid,
    reference: Grid3,
    windows: Windows,
    sizes: Vec<usize>,
    law: Law,
    params: Vec<u32>,
    want: Vec<u32>,
    model: Model,
    frame: u32,
    label: String,
    fewest: usize,
}

impl<'host> Engine<'host> {
    fn new(host: &'host Host, law: &Law, params: &[u32], coarse: bool) -> Option<Self> {
        let width = host.width();
        let (grid, reference) = grids(SHAPE);
        let plan = match Plan::over_in(
            width,
            grid.blocks().div_ceil(sweep::cells(width)),
            Dimension::Three,
        ) {
            Ok(plan) => plan,
            Err(too_big) => {
                common::skip(format_args!("width {width} {too_big}"));
                return None;
            }
        };
        let (passes, stages) = live::arranged3_by(law, width, grid, &plan, coarse).unwrap();
        let built = sand3::settle_by(law, width, grid, host.limits().groups).unwrap();
        let tick = live::tick(width).unwrap();
        let sizes = live::sizes3(law, &plan).to_vec();
        let windows = live::windows3(law, &plan);
        assert!(!windows.is_split());
        let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
        let held = host.resident(&sizes, &modules).unwrap();
        let modules: Vec<&[u32]> = vec![&tick, &built.words];
        let full = host.resident(&sizes, &modules).unwrap();
        Some(Self {
            live: held,
            full,
            stages,
            whole: [Groups::Fixed(1), built.spread.dispatch()],
            plan,
            grid,
            reference,
            windows,
            sizes,
            law: law.clone(),
            params: params.to_vec(),
            want: Vec::new(),
            model: Model::new(law, SHAPE, params, &[]),
            frame: 0,
            label: String::new(),
            fewest: usize::MAX,
        })
    }

    fn restart(&mut self, start: &[u32], label: String) {
        for held in [&mut self.live, &mut self.full] {
            for (binding, &words) in self.sizes.iter().enumerate() {
                if binding == PIXELS as usize {
                    continue;
                }
                held.write(binding, &vec![0_u32; words]).unwrap();
            }
            held.write(STATE as usize, start).unwrap();
            held.write(PARAMS as usize, &[u32::MAX, self.plan.dump()])
                .unwrap();
            if !self.params.is_empty() {
                held.write_at(PARAMS as usize, FIRST_PARAM, &self.params)
                    .unwrap();
            }
        }
        live::seed(&mut self.live, self.grid, &self.plan).unwrap();
        self.want = start.to_vec();
        self.model = Model::new(&self.law, SHAPE, &self.params, start);
        self.frame = 0;
        self.label = label;
        self.fewest = usize::MAX;
    }

    fn listed(&mut self) -> Vec<u32> {
        let grand = self.live.read(GRAND as usize, 1).unwrap()[0];
        let mut octs = self.live.read(DENSE as usize, grand as usize).unwrap();
        octs.sort_unstable();
        octs
    }

    fn parted(&self, what: &str, got: &[u32]) {
        if got == self.want {
            return;
        }
        let at = got
            .iter()
            .zip(&self.want)
            .position(|(a, b)| a != b)
            .unwrap();
        let cells = self.reference.cells();
        let (plane, cell) = (at / cells, at % cells);
        let count = got.iter().zip(&self.want).filter(|(a, b)| a != b).count();
        panic!(
            "{}: frame {} (phase {}): {what} differs at plane {plane} cell {cell}: {} against the CPU's {}; {count} words differ",
            self.label,
            self.frame,
            self.frame & 1,
            got[at],
            self.want[at]
        );
    }

    fn lists_as_the_model(&mut self, when: &str) {
        let device = self.listed();
        let model = self.model.octs();
        assert_eq!(
            device,
            model,
            "{}: {when} frame {}: the device lists {} octs, the model {}",
            self.label,
            self.frame,
            device.len(),
            model.len()
        );
        self.fewest = self.fewest.min(device.len());
    }

    fn step(&mut self) {
        let words = self.want.len();
        let stages: Vec<&[Groups]> = self.stages.iter().map(Vec::as_slice).collect();
        self.live.staged(&stages).unwrap();
        let listed = self.live.read(STATE as usize, words).unwrap();
        let whole: Vec<&[Groups]> = self.whole.iter().map(core::slice::from_ref).collect();
        let (_, full) = self
            .full
            .staged_reading(&whole, STATE as usize, words)
            .unwrap();
        self.want = sand3::reference(
            &self.law,
            self.reference,
            self.frame,
            &self.want,
            &self.params,
        );
        self.model.listed(self.frame);
        self.parted("the listed world", &listed);
        self.parted("the whole grid", &full);
        self.parted("the model", &self.model.cells.clone());
        assert_eq!(hashed(&listed), hashed(&self.want), "{}", self.label);
        assert_eq!(hashed(&full), hashed(&self.want), "{}", self.label);
        self.lists_as_the_model("after");
        self.frame += 1;
    }

    fn write(&mut self, writes: &[(usize, u32, u32)]) {
        let cells = self.reference.cells();
        for &(plane, cell, word) in writes {
            let placed = Placed3 {
                windows: Some(self.windows),
                plane: u32::try_from(plane).unwrap(),
                around: true,
            };
            live::put3_in(
                &mut self.live,
                self.grid,
                &self.plan,
                placed,
                &[(cell, word)],
            )
            .unwrap();
            live::write_state(
                &mut self.full,
                self.windows,
                plane * cells + cell as usize,
                &[word],
            )
            .unwrap();
            self.want[plane * cells + cell as usize] = word;
            self.model.write(plane, cell, word);
        }
        self.lists_as_the_model("written before");
    }
}

fn written_cells(
    subject: &Subject,
    grid: Grid3,
    seam: Seam,
    state: &mut u64,
) -> Vec<(usize, u32, u32)> {
    let planes = compiled(subject.text).planes();
    (0..24)
        .map(|_| {
            let plane = below(state, u32::try_from(planes).unwrap()) as usize;
            let [x, y, z] = near_seam(grid, seam, state);
            let cell = u32::try_from(grid.index(x, y, z)).unwrap();
            (plane, cell, (subject.word)(state, plane))
        })
        .collect()
}

fn matrix(subject: &Subject) {
    let Some(host) = device(subject.name) else {
        return;
    };
    let law = compiled(subject.text);
    assert!(!law.looks().is_empty());
    assert_eq!(
        quiet::class(&law, &subject.params, &[]),
        Class::Proved,
        "{}",
        subject.name
    );
    for coarse in TIERS {
        let Some(mut engine) = Engine::new(&host, &law, &subject.params, coarse) else {
            continue;
        };
        for (index, seam) in SEAMS.into_iter().enumerate() {
            let seed = 0x4A10 + index as u64;
            let first = start(subject, engine.reference, seam, seed);
            let tier = if coarse { "coarse" } else { "fine" };
            engine.restart(&first, format!("{} {seam:?} seam {tier}", subject.name));
            let mut state = seed ^ 0xF00D;
            let mut moved = false;
            while engine.frame < FRAMES {
                if engine.frame > 0 && engine.frame.is_multiple_of(WRITES_EVERY) {
                    let writes = written_cells(subject, engine.reference, seam, &mut state);
                    engine.write(&writes);
                }
                let before = engine.want.clone();
                engine.step();
                moved |= before != engine.want;
            }
            assert!(moved, "{}: nothing moved", engine.label);
            assert!(
                engine.fewest < engine.grid.blocks() as usize,
                "{}: every oct stayed listed",
                engine.label
            );
            println!(
                "{} on {} width {}: {FRAMES} frames listed == whole == CPU == model, lists equal every frame, fewest listed {} of {}",
                engine.label,
                host.name(),
                host.width(),
                engine.fewest,
                engine.grid.blocks()
            );
        }
    }
}

#[test]
fn the_edge_law_steps_listed_as_whole_as_the_cpu_and_lists_as_the_model() {
    matrix(&subjects()[0]);
}

#[test]
fn the_sobel_law_steps_listed_as_whole_as_the_cpu_and_lists_as_the_model() {
    matrix(&subjects()[1]);
}

#[test]
fn the_blur_law_steps_listed_as_whole_as_the_cpu_and_lists_as_the_model() {
    matrix(&subjects()[2]);
}

fn made<'made>(law: &'made Law, params: &'made [u32], sparse: bool) -> Made<'made> {
    Made {
        rule: law.name(),
        params,
        size: [SHAPE.0, SHAPE.1, SHAPE.2],
        sparse,
        slots: None,
    }
}

fn point_of(grid: Grid3, cell: u32) -> [u32; 3] {
    let cell = cell as usize;
    let plane = (grid.wide * grid.deep) as usize;
    [
        (cell % grid.wide as usize) as u32,
        ((cell % plane) / grid.wide as usize) as u32,
        (cell / plane) as u32,
    ]
}

#[test]
fn a_written_cell_wakes_every_box_that_reads_it() {
    let Some(host) = device("halo write wake") else {
        return;
    };
    let subject = &subjects()[0];
    let law = compiled(subject.text);
    let (_, grid) = grids(SHAPE);
    let mut world = World::create_law(&host, &law, &made(&law, &subject.params, false)).unwrap();
    let mut want = vec![0_u32; grid.cells() * law.planes()];
    let mut state = 0xB0C5_u64;
    let mut frame = 0_u32;
    while frame < 96 {
        if frame.is_multiple_of(WRITES_EVERY) {
            let mut writes = Vec::new();
            for _ in 0..24 {
                let cell = below(&mut state, u32::try_from(grid.cells()).unwrap());
                let word = binary(&mut state, 0);
                writes.push((point_of(grid, cell), word));
                want[cell as usize] = word;
            }
            world.write_plane(0, &writes).unwrap();
        }
        world.step(1).unwrap();
        want = sand3::reference(&law, grid, frame, &want, &subject.params);
        let got = world.cells().unwrap();
        if got != want {
            let at = got.iter().zip(&want).position(|(a, b)| a != b).unwrap();
            panic!(
                "frame {frame}: word {at} is {} against the CPU's {}: a write did not wake a box that reads it",
                got[at], want[at]
            );
        }
        frame += 1;
    }
}

fn refusal(text: &str) -> Refusal {
    match Law::compile(text) {
        Ok(law) => panic!("compiled {} into {} nodes", law.name(), law.nodes()),
        Err(why) => why,
    }
}

fn refused_at(text: &str, code: Code, path: &str) -> Refusal {
    let why = refusal(text);
    assert_eq!((why.code, why.path.as_str()), (code, path), "{why}");
    assert!(Code::ALL.contains(&why.code));
    why
}

fn probe(channels: u32, halo: u32, params: u32, outs: &str) -> String {
    format!(
        "{{\"law\":1,\"name\":\"probe\",\"params\":{params},\"channels\":{channels},\"halo\":{halo},\"tally\":{{\"counts\":1,\"weighted\":false}},\"consts\":{{\"BACK\":4294967295}},\"defs\":{{}},\"step\":[\"vec\",[\"map\",\"c\",[\"range\",8],{outs}],[\"vec\",0]]}}"
    )
}

#[test]
fn every_refused_look_is_named_by_its_code_and_path() {
    let ok = probe(
        2,
        1,
        0,
        "[\"vec\",[\"at\",[\"at\",\"cells\",\"c\"],0],[\"look\",0,-1,1,0,\"c\"]]",
    );
    assert_eq!(compiled(&ok).halo(), 1);
    let at = "$.step[1][3][2]";
    let why = refused_at(
        &probe(
            2,
            1,
            0,
            "[\"vec\",[\"at\",[\"at\",\"cells\",\"c\"],0],[\"look\",0,2,0,0,\"c\"]]",
        ),
        Code::HaloRadius,
        &format!("{at}[2]"),
    );
    assert_eq!(why.found, "2");
    assert_eq!(why.limit, Some(1));
    refused_at(
        &probe(
            2,
            1,
            0,
            "[\"vec\",[\"at\",[\"at\",\"cells\",\"c\"],0],[\"look\",0,0,-2,0,\"c\"]]",
        ),
        Code::HaloRadius,
        &format!("{at}[3]"),
    );
    refused_at(
        &probe(
            2,
            1,
            0,
            "[\"vec\",[\"at\",[\"at\",\"cells\",\"c\"],0],[\"look\",0,0,0,[\"sub\",\"BACK\",1],\"c\"]]",
        ),
        Code::HaloRadius,
        &format!("{at}[4]"),
    );
    refused_at(
        &probe(
            2,
            1,
            1,
            "[\"vec\",[\"at\",[\"at\",\"cells\",\"c\"],0],[\"look\",[\"param\",0],0,0,0,\"c\"]]",
        ),
        Code::HaloConstant,
        &format!("{at}[1]"),
    );
    refused_at(
        &probe(
            2,
            1,
            0,
            "[\"vec\",[\"at\",[\"at\",\"cells\",\"c\"],0],[\"look\",0,[\"at\",[\"at\",\"cells\",\"c\"],0],0,0,\"c\"]]",
        ),
        Code::HaloConstant,
        &format!("{at}[2]"),
    );
    refused_at(
        &probe(
            2,
            1,
            0,
            "[\"vec\",[\"at\",[\"at\",\"cells\",\"c\"],0],[\"look\",0,0,0,0,[\"at\",[\"at\",\"cells\",\"c\"],1]]]",
        ),
        Code::HaloConstant,
        &format!("{at}[5]"),
    );
    let why = refused_at(
        &probe(
            2,
            1,
            0,
            "[\"vec\",[\"at\",[\"at\",\"cells\",\"c\"],0],[\"look\",2,0,0,0,\"c\"]]",
        ),
        Code::HaloPlane,
        &format!("{at}[1]"),
    );
    assert_eq!(why.limit, Some(2));
    refused_at(
        &probe(
            2,
            1,
            0,
            "[\"vec\",[\"at\",[\"at\",\"cells\",\"c\"],0],[\"look\",0,0,0,0,8]]",
        ),
        Code::Index,
        &format!("{at}[5]"),
    );
    let why = refused_at(
        &probe(
            2,
            0,
            0,
            "[\"vec\",[\"at\",[\"at\",\"cells\",\"c\"],0],[\"look\",0,1,0,0,\"c\"]]",
        ),
        Code::HaloDeclared,
        "$.halo",
    );
    assert_eq!(why.found, format!("halo 0, and a look at {at}"));
    let why = refused_at(
        &probe(
            2,
            1,
            0,
            "[\"vec\",[\"at\",[\"at\",\"cells\",\"c\"],0],[\"at\",[\"at\",\"cells\",\"c\"],0]]",
        ),
        Code::HaloDeclared,
        "$.halo",
    );
    assert_eq!(why.found, "halo 1, and no look past the oct");
    refused_at(
        &probe(
            2,
            2,
            0,
            "[\"vec\",[\"at\",[\"at\",\"cells\",\"c\"],0],[\"look\",0,1,0,0,\"c\"]]",
        ),
        Code::Halo,
        "$.halo",
    );
    let reversible = ok.replacen("\"halo\":1,", "\"halo\":1,\"reversible\":true,", 1);
    refused_at(&reversible, Code::Unsupported, "$.reversible");
}

#[test]
fn the_declared_halo_is_whether_a_look_leaves_the_oct() {
    let inside = "[\"vec\",[\"at\",[\"at\",\"cells\",\"c\"],0],[\"add\",[\"look\",0,0,0,0,\"c\"],[\"look\",0,1,0,0,[\"and\",\"c\",6]]]]";
    let why = refused_at(&probe(2, 1, 0, inside), Code::HaloDeclared, "$.halo");
    assert_eq!(why.found, "halo 1, and no look past the oct");
    let law = compiled(&probe(2, 0, 0, inside));
    assert_eq!(law.halo(), 0);
    assert!(law.looks().is_empty());
    assert!(branch::looks_dense(&law, true).is_ok());
    let twin = compiled(&probe(
        2,
        0,
        0,
        "[\"vec\",[\"at\",[\"at\",\"cells\",\"c\"],0],[\"add\",[\"at\",[\"at\",\"cells\",\"c\"],0],[\"at\",[\"at\",\"cells\",[\"or\",[\"and\",\"c\",6],1]],0]]]",
    ));
    let mut state = 0x1A51_u64;
    for _ in 0..64 {
        let words: [u32; CORNERS] = core::array::from_fn(|_| below(&mut state, 1000));
        let upper = [core::array::from_fn(|_| below(&mut state, 1000))];
        let oct = || Oct {
            words,
            frame: 0,
            at: [0; 3],
            params: &[],
        };
        assert_eq!(
            law.step_planes(oct(), &upper),
            twin.step_planes(oct(), &upper)
        );
    }
    let written = compiled(&probe(
        2,
        0,
        0,
        "[\"vec\",[\"at\",[\"at\",\"cells\",\"c\"],0],[\"add\",[\"look\",1,0,0,0,\"c\"],1]]",
    ));
    assert!(written.looks().is_empty());
    let outside =
        "[\"vec\",[\"at\",[\"at\",\"cells\",\"c\"],0],[\"look\",0,0,0,0,[\"look\",0,0,0,0,\"c\"]]]";
    refused_at(
        &probe(2, 0, 0, outside),
        Code::HaloConstant,
        "$.step[1][3][2][5]",
    );
    for (text, halo) in [(EDGE, 1), (SOBEL, 1), (BLUR, 1)] {
        let law = compiled(text);
        assert_eq!(law.halo(), halo);
        assert_eq!(law.halo() == 1, !law.looks().is_empty());
        assert!(branch::looks_dense(&law, true).is_err());
    }
    for corner in 0..CORNERS {
        for dx in [-1_i32, 0, 1] {
            let look = format!(
                "[\"vec\",[\"at\",[\"at\",\"cells\",\"c\"],0],[\"look\",0,{dx},0,0,{corner}]]"
            );
            let leaves = grit::halo::own(grit::halo::looked(corner, [dx, 0, 0]).unwrap()).is_none();
            let declared = u32::from(leaves);
            let law = compiled(&probe(2, declared, 0, &look));
            assert_eq!(law.halo() == 1, !law.looks().is_empty(), "{look}");
            refused_at(
                &probe(2, 1 - declared, 0, &look),
                Code::HaloDeclared,
                "$.halo",
            );
        }
    }
}

#[test]
fn a_look_at_a_written_plane_is_refused_halo_written_with_the_plane_named() {
    let at = "$.step[1][3][2]";
    let why = refused_at(
        &probe(
            2,
            1,
            0,
            "[\"vec\",[\"at\",[\"at\",\"cells\",\"c\"],0],[\"look\",1,1,0,0,\"c\"]]",
        ),
        Code::HaloWritten,
        at,
    );
    assert!(why.found.contains("plane 1"), "{why}");
    assert!(why.expected.contains("plane 1"), "{why}");
    let why = refused_at(
        &probe(
            2,
            1,
            0,
            "[\"vec\",[\"add\",[\"look\",0,0,0,0,\"c\"],1],[\"look\",0,1,0,0,\"c\"]]",
        ),
        Code::HaloWritten,
        "$.step[1][3][2]",
    );
    assert!(why.found.contains("plane 0"), "{why}");
    refused_at(
        &probe(
            2,
            1,
            0,
            "[\"vec\",[\"look\",0,0,0,0,[\"xor\",\"c\",1]],[\"look\",0,1,0,0,\"c\"]]",
        ),
        Code::HaloWritten,
        "$.step[1][3][2]",
    );
    let kept = compiled(&probe(
        2,
        1,
        0,
        "[\"vec\",[\"look\",0,0,0,0,\"c\"],[\"look\",0,1,0,0,\"c\"]]",
    ));
    assert!(kept.looks().looked(0));
    let unused = probe(
        2,
        0,
        0,
        "[\"vec\",[\"at\",[\"at\",\"cells\",\"c\"],0],[\"let\",[[\"dead\",[\"look\",1,1,0,0,\"c\"]]],[\"add\",[\"at\",[\"at\",\"cells\",\"c\"],1],1]]]",
    );
    refused_at(&unused, Code::HaloDeclared, "$.halo");
    let unused = unused.replacen("\"halo\":0", "\"halo\":1", 1);
    let why = refusal(&unused);
    assert_eq!(why.code, Code::HaloWritten, "{why}");
}

#[test]
fn a_look_inside_the_oct_is_the_corner_and_outside_reads_the_box() {
    let law = compiled(&probe(
        2,
        1,
        0,
        "[\"vec\",[\"at\",[\"at\",\"cells\",\"c\"],0],[\"add\",[\"look\",0,0,0,0,\"c\"],[\"mul\",256,[\"look\",0,1,0,0,\"c\"]]]]",
    ));
    let looks = law.looks();
    assert_eq!(looks.planes(), 1);
    assert_eq!(looks.loads(), 4);
    let halo: Vec<[u32; BOX]> = vec![
        core::array::from_fn(|at| u32::try_from(at).unwrap() + 1000),
        [0; BOX],
    ];
    let words: [u32; CORNERS] = core::array::from_fn(|corner| u32::try_from(corner).unwrap());
    let planed = law.step_halo(
        Oct {
            words,
            frame: 0,
            at: [0; 3],
            params: &[],
        },
        &[[0; CORNERS]],
        &halo,
    );
    for corner in 0..CORNERS {
        let [x, y, z] = grit::halo::corner_point(corner);
        let east = if x == 0 {
            words[corner ^ 1]
        } else {
            u32::try_from(grit::halo::cell([x + 2, y + 1, z + 1])).unwrap() + 1000
        };
        assert_eq!(
            planed.planes[0][corner],
            words[corner].wrapping_add(east.wrapping_mul(256)),
            "corner {corner}"
        );
    }
    assert_eq!(Halo::NONE.loads(), 0);
}

#[test]
fn a_halo_law_that_reads_the_frame_is_whole_and_equals_the_cpu_dense() {
    let Some(host) = device("halo whole") else {
        return;
    };
    let text = EDGE
        .replace("\"name\": \"edge2d\"", "\"name\": \"edgeframe\"")
        .replace(
            "[\"param\", 0]",
            "[\"add\", [\"param\", 0], [\"and\", \"frame\", 1]]",
        );
    let law = compiled(&text);
    let params = [1_u32];
    let class = quiet::class(&law, &params, &[]);
    assert!(matches!(class, Class::Whole(Why::Reads(_))), "{class:?}");
    let noisy = EDGE
        .replace("\"name\": \"edge2d\"", "\"name\": \"edgenoise\"")
        .replace("[\"param\", 0]", "[\"and\", [\"noise\"], 3]");
    let noisy = compiled(&noisy);
    assert!(matches!(
        quiet::class(&noisy, &params, &[]),
        Class::Whole(Why::Reads(_))
    ));
    let (_, grid) = grids(SHAPE);
    for (law, params) in [(&law, &params[..]), (&noisy, &params[..])] {
        let made = made(law, params, false);
        let mut world = World::create_law_stepped(&host, law, &made, Stepping::Whole).unwrap();
        let mut want = start(&subjects()[0], grid, Seam::X, 0x77);
        let mut writes = Vec::new();
        let cells = grid.cells();
        for plane in 0..2 {
            for cell in 0..cells {
                let word = want[plane * cells + cell];
                if word != 0 {
                    writes.push((plane, (point_of(grid, u32::try_from(cell).unwrap()), word)));
                }
            }
        }
        for plane in 0..2 {
            let mine: Vec<([u32; 3], u32)> = writes
                .iter()
                .filter(|(at, _)| *at == plane)
                .map(|(_, one)| *one)
                .collect();
            world
                .write_plane(u32::try_from(plane).unwrap(), &mine)
                .unwrap();
        }
        let mut state = 0x88_u64;
        for frame in 0..64 {
            if frame > 0 && frame % WRITES_EVERY == 0 {
                let mut some = Vec::new();
                for _ in 0..24 {
                    let cell = below(&mut state, u32::try_from(cells).unwrap());
                    let word = binary(&mut state, 0);
                    some.push((point_of(grid, cell), word));
                    want[cell as usize] = word;
                }
                world.write_plane(0, &some).unwrap();
            }
            world.step(1).unwrap();
            want = sand3::reference(law, grid, frame, &want, params);
            assert_eq!(world.cells().unwrap(), want, "{} frame {frame}", law.name());
        }
    }
}

#[test]
fn a_sparse_world_refuses_a_law_that_looks() {
    let Some(host) = device("halo sparse") else {
        return;
    };
    let law = compiled(EDGE);
    let why = World::create_law(&host, &law, &made(&law, &[2], true))
        .err()
        .unwrap();
    let refused = why.downcast_ref::<branch::Refused>().unwrap();
    assert_eq!(refused.name(), "halo_sparse", "{refused}");
    let width = host.width();
    let (grid, _) = grids((32, 32, 32));
    let plan = Plan::over_in(
        width,
        grid.blocks().div_ceil(sweep::cells(width)),
        Dimension::Three,
    )
    .unwrap();
    let pool = grit::bricks::Bricks::new(grid, &plan, plan.blocks)
        .unwrap()
        .with_planes(2)
        .unwrap();
    assert!(grit::bricks::arranged(&law, &pool, &plan, true).is_err());
    assert!(grit::bricks::whole(&law, &pool, &plan).is_err());
}

fn line(id: i64, name: &str, arguments: &str) -> String {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"tools/call\",\"params\":{{\"name\":\"{name}\",\"arguments\":{arguments}}}}}"
    )
}

fn result(got: &Json, error: bool) -> Json {
    let result = got.get("result").unwrap();
    assert_eq!(
        result.get("isError"),
        Some(&Json::Bool(error)),
        "{}",
        written(got)
    );
    result.get("structuredContent").cloned().unwrap()
}

fn text_at(json: &Json, key: &str) -> String {
    json.get(key)
        .and_then(Json::text)
        .unwrap_or_else(|| panic!("no {key} in {}", written(json)))
        .to_owned()
}

fn int_at(json: &Json, key: &str) -> i64 {
    json.get(key)
        .and_then(Json::int)
        .unwrap_or_else(|| panic!("no {key} in {}", written(json)))
}

fn quoted(text: &str) -> String {
    written(&Json::Text(text.to_owned()))
}

fn making(law: &str, params: &[u32], sparse: bool) -> String {
    format!(
        "{{\"wide\":{},\"deep\":{},\"tall\":{},\"law\":{},\"params\":{params:?},\"sparse\":{sparse}}}",
        SHAPE.0,
        SHAPE.1,
        SHAPE.2,
        quoted(law)
    )
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("grit-halo3-{label}-{}-{nanos}", std::process::id()));
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

trait Asking {
    fn asked(&mut self, name: &str, arguments: &str) -> Json;
}

struct InProcess<'host>(Server<'host>);

impl Asking for InProcess<'_> {
    fn asked(&mut self, name: &str, arguments: &str) -> Json {
        self.0.heard(&line(7, name, arguments)).unwrap()
    }
}

struct Process {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    id: i64,
}

impl Process {
    fn start(dir: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_grit-mcp"))
            .arg("--dir")
            .arg(dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            input,
            output,
            id: 0,
        }
    }

    fn kill(mut self) {
        self.child.kill().unwrap();
        self.child.wait().unwrap();
    }
}

impl Asking for Process {
    fn asked(&mut self, name: &str, arguments: &str) -> Json {
        self.id += 1;
        writeln!(self.input, "{}", line(self.id, name, arguments)).unwrap();
        self.input.flush().unwrap();
        loop {
            let mut held = String::new();
            assert!(
                self.output.read_line(&mut held).unwrap() > 0,
                "the server ended"
            );
            let got = read(held.trim_end()).unwrap();
            if got.get("id").and_then(Json::int) == Some(self.id) {
                return got;
            }
        }
    }
}

struct Run {
    law: Law,
    params: Vec<u32>,
    grid: Grid3,
    cells: Vec<u32>,
    frame: u32,
    state: u64,
    word: fn(&mut u64, usize) -> u32,
}

impl Run {
    fn new(subject: &Subject, seed: u64) -> Self {
        let law = compiled(subject.text);
        let (_, grid) = grids(SHAPE);
        let cells = vec![0; grid.cells() * law.planes()];
        Self {
            law,
            params: subject.params.clone(),
            grid,
            cells,
            frame: 0,
            state: seed,
            word: subject.word,
        }
    }

    fn forked(&self, seed: u64) -> Self {
        Self {
            law: self.law.clone(),
            params: self.params.clone(),
            grid: self.grid,
            cells: self.cells.clone(),
            frame: self.frame,
            state: seed,
            word: self.word,
        }
    }

    fn write(&mut self, asking: &mut impl Asking, world: i64) {
        let mut placed = Vec::new();
        for _ in 0..48 {
            let cell = below(&mut self.state, u32::try_from(self.grid.cells()).unwrap());
            let word = (self.word)(&mut self.state, 0);
            self.cells[cell as usize] = word;
            let [x, y, z] = point_of(self.grid, cell);
            placed.push(format!("[{x},{y},{z},{word}]"));
        }
        result(
            &asking.asked(
                "grit_world_write",
                &format!("{{\"world\":{world},\"cells\":[{}]}}", placed.join(",")),
            ),
            false,
        );
    }

    fn step(&mut self, asking: &mut impl Asking, world: i64, frames: u32) {
        result(
            &asking.asked(
                "grit_world_step",
                &format!("{{\"world\":{world},\"frames\":{frames},\"budget_ms\":600000}}"),
            ),
            false,
        );
        self.ran(frames);
    }

    fn ran(&mut self, frames: u32) {
        for _ in 0..frames {
            self.cells =
                sand3::reference(&self.law, self.grid, self.frame, &self.cells, &self.params);
            self.frame += 1;
        }
    }

    fn state(&self) -> String {
        let image = branch::Image {
            law: branch::Law::Law {
                name: self.law.name().to_owned(),
                canonical: self.law.canonical().to_owned(),
            },
            params: self.params.clone(),
            frame: self.frame,
            size: [self.grid.wide, self.grid.deep, self.grid.tall],
            state: branch::State::Dense(self.cells.clone()),
        };
        grit::law::hex(&grit::serve3::journal::state_of(&image))
    }
}

fn saved(asking: &mut impl Asking, world: i64, name: &str) -> String {
    text_at(
        &result(
            &asking.asked(
                "grit_world_save",
                &format!("{{\"world\":{world},\"name\":\"{name}\",\"replace\":true}}"),
            ),
            false,
        ),
        "state",
    )
}

#[test]
fn save_open_fork_and_restore_of_a_dense_halo_world_give_the_cpu_run() {
    let Some(host) = device("halo save open fork restore") else {
        return;
    };
    let scratch = Scratch::new("branch");
    let mut server = InProcess(
        Server::with(
            Some(&host),
            &Options {
                dir: Some(scratch.dir()),
                memory: None,
            },
        )
        .unwrap(),
    );
    let mut run = Run::new(&subjects()[0], 0x5A1);
    let made = result(
        &server.asked("grit_world_create", &making(EDGE, &[2], false)),
        false,
    );
    assert_eq!(text_at(&made, "quiet"), "proved");
    let world = int_at(&made, "world");
    for _ in 0..4 {
        run.write(&mut server, world);
        run.step(&mut server, world, 7);
    }
    assert_eq!(saved(&mut server, world, "edge"), run.state());
    let fork = int_at(
        &result(
            &server.asked("grit_world_fork", &format!("{{\"world\":{world}}}")),
            false,
        ),
        "world",
    );
    let mut forked = run.forked(0xF0);
    run.write(&mut server, world);
    run.step(&mut server, world, 5);
    forked.write(&mut server, fork);
    forked.step(&mut server, fork, 9);
    assert_eq!(saved(&mut server, fork, "fork"), forked.state());
    assert_eq!(saved(&mut server, world, "world"), run.state());
    result(
        &server.asked(
            "grit_world_restore",
            &format!("{{\"world\":{world},\"from\":{fork}}}"),
        ),
        false,
    );
    run.cells.clone_from(&forked.cells);
    run.frame = forked.frame;
    run.write(&mut server, world);
    run.step(&mut server, world, 6);
    assert_eq!(saved(&mut server, world, "restored"), run.state());
    let opened = int_at(
        &result(
            &server.asked("grit_world_open", "{\"name\":\"fork\"}"),
            false,
        ),
        "world",
    );
    forked.write(&mut server, opened);
    forked.step(&mut server, opened, 11);
    assert_eq!(saved(&mut server, opened, "opened"), forked.state());
}

#[test]
fn a_crash_and_replay_of_a_dense_halo_world_gives_the_cpu_run() {
    let Some(host) = device("halo crash replay") else {
        return;
    };
    drop(host);
    let scratch = Scratch::new("crash");
    let mut first = Process::start(&scratch.dir());
    let mut run = Run::new(&subjects()[1], 0xC4A5);
    let world = int_at(
        &result(
            &first.asked("grit_world_create", &making(SOBEL, &[5], false)),
            false,
        ),
        "world",
    );
    for _ in 0..5 {
        run.write(&mut first, world);
        run.step(&mut first, world, 6);
    }
    first.kill();
    let mut second = Process::start(&scratch.dir());
    let opened = result(
        &second.asked(
            "grit_world_open",
            &format!("{{\"name\":\"world-{world}\"}}"),
        ),
        false,
    );
    assert_eq!(int_at(&opened, "frame"), i64::from(run.frame));
    assert_eq!(saved(&mut second, world, "replayed"), run.state());
    run.write(&mut second, world);
    run.step(&mut second, world, 9);
    assert_eq!(saved(&mut second, world, "after"), run.state());
    second.kill();
}

#[test]
fn a_sparse_world_and_a_sparse_file_refuse_a_halo_law_through_the_tools() {
    let Some(host) = device("halo sparse tools") else {
        return;
    };
    let scratch = Scratch::new("sparse");
    let mut server = InProcess(
        Server::with(
            Some(&host),
            &Options {
                dir: Some(scratch.dir()),
                memory: None,
            },
        )
        .unwrap(),
    );
    let told = result(
        &server.asked("grit_world_create", &making(EDGE, &[2], true)),
        true,
    );
    assert_eq!(text_at(&told, "code"), "halo_sparse", "{}", written(&told));
    assert_eq!(text_at(&told, "field"), "$.sparse");
    assert!(text_at(&told, "hint").contains("dense"));
    let flat = "{\"law\":1,\"name\":\"edge2d\",\"params\":1,\"channels\":2,\"halo\":0,\"tally\":{\"counts\":1,\"weighted\":false},\"consts\":{},\"defs\":{},\"step\":[\"vec\",\"cells\",[\"vec\",[\"param\",0]]]}";
    let world = int_at(
        &result(
            &server.asked(
                "grit_world_create",
                &making(flat, &[0], true).replace(
                    &format!(
                        "\"wide\":{},\"deep\":{},\"tall\":{}",
                        SHAPE.0, SHAPE.1, SHAPE.2
                    ),
                    "\"wide\":32,\"deep\":32,\"tall\":32",
                ),
            ),
            false,
        ),
        "world",
    );
    saved(&mut server, world, "sparse");
    let path = scratch.dir().join("sparse.grit");
    let mut image = branch::format::fetch(&path).unwrap();
    assert!(matches!(image.state, branch::State::Sparse { .. }));
    image.law = branch::Law::Law {
        name: "edge2d".to_owned(),
        canonical: compiled(EDGE).canonical().to_owned(),
    };
    branch::format::store(&path, &image).unwrap();
    let told = result(
        &server.asked("grit_world_open", "{\"name\":\"sparse\"}"),
        true,
    );
    assert_eq!(text_at(&told, "code"), "halo_sparse", "{}", written(&told));
    assert_eq!(text_at(&told, "field"), "$.name");
    let why = World::open(&host, &image).err().unwrap();
    assert_eq!(
        why.downcast_ref::<branch::Refused>().unwrap().name(),
        "halo_sparse"
    );
    let described = result(&server.asked("grit_describe", "{}"), false);
    let codes: Vec<String> = described
        .get("codes")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .map(|one| text_at(one, "code"))
        .collect();
    assert!(codes.iter().any(|one| one == "halo_sparse"));
    let look = text_at(described.get("limits").unwrap(), "law_look");
    for needed in [
        "[\"look\", plane, dx, dy, dz, corner]",
        "frozen",
        "halo_written",
        "dense",
    ] {
        assert!(look.contains(needed), "{needed}: {look}");
    }
}

#[test]
fn halo_free_laws_emit_the_kernels_on_record() {
    let mut want = std::collections::BTreeMap::new();
    for line in KERNELS.lines().filter(|line| !line.trim().is_empty()) {
        let (name, hash) = line.trim_end().rsplit_once(' ').unwrap();
        want.insert(name.to_owned(), hash.to_owned());
    }
    let mut got = std::collections::BTreeMap::new();
    for width in [4, 8, 16, 32, 64] {
        for (name, hash) in kernel_catalog(width) {
            got.insert(name, hash);
        }
    }
    assert_eq!(got.len(), want.len());
    for (name, hash) in &want {
        assert_eq!(got.get(name), Some(hash), "{name}");
    }
}

#[cfg(feature = "faults")]
mod faulted {
    use super::*;

    use std::collections::BTreeMap;
    use std::sync::mpsc::Receiver;
    use std::time::Duration;

    use grit::serve3::exec::Exec;

    struct Threaded {
        exec: Exec,
        answers: Receiver<Json>,
        id: i64,
        early: BTreeMap<i64, Json>,
    }

    impl Threaded {
        fn new(options: &Options) -> Self {
            let pattern = std::env::var("GRIT_DEVICE")
                .ok()
                .filter(|one| !one.is_empty());
            let (exec, answers) = Exec::spawn(
                Box::new(move || Host::matching(pattern.as_deref())),
                options,
            )
            .unwrap();
            Self {
                exec,
                answers,
                id: 100,
                early: BTreeMap::new(),
            }
        }
    }

    impl Asking for Threaded {
        fn asked(&mut self, name: &str, arguments: &str) -> Json {
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

    #[test]
    fn after_a_lost_device_a_dense_halo_world_is_the_cpu_run() {
        let Some(host) = device("halo lost device") else {
            return;
        };
        drop(host);
        let scratch = Scratch::new("lost");
        let mut threaded = Threaded::new(&Options {
            dir: Some(scratch.dir()),
            memory: None,
        });
        let mut run = Run::new(&subjects()[2], 0x1057);
        let world = int_at(
            &result(
                &threaded.asked("grit_world_create", &making(BLUR, &[], false)),
                false,
            ),
            "world",
        );
        for _ in 0..4 {
            run.write(&mut threaded, world);
            run.step(&mut threaded, world, 5);
        }
        {
            let faults = threaded.exec.faults();
            faults.lock().unwrap().wedge_after_batch = Some(1);
        }
        let lost = threaded.asked(
            "grit_world_step",
            &format!("{{\"world\":{world},\"frames\":200,\"budget_ms\":600000}}"),
        );
        let told = result(&lost, true);
        assert_eq!(text_at(&told, "code"), "device_lost", "{}", written(&told));
        let message = text_at(&told, "message");
        let ran: u32 = message
            .split(" of the 200 frames ran")
            .next()
            .and_then(|head| head.rsplit(' ').next())
            .and_then(|count| count.parse().ok())
            .unwrap_or_else(|| panic!("{message}"));
        run.ran(ran);
        for note in told.get("restored").and_then(Json::list).unwrap() {
            assert!(note.get("failed").is_none(), "{}", written(note));
        }
        assert_eq!(saved(&mut threaded, world, "rebuilt"), run.state());
        run.write(&mut threaded, world);
        run.step(&mut threaded, world, 8);
        assert_eq!(saved(&mut threaded, world, "after"), run.state());
        threaded.exec.finish();
    }
}

const SHIPPED: [(&str, &str); 8] = [
    ("code", include_str!("law/code.law.json")),
    ("embed", include_str!("law/embed.law.json")),
    ("evo", include_str!("law/evo.law.json")),
    ("heat", include_str!("law/heat.law.json")),
    ("mix", include_str!("law/mix.law.json")),
    ("nature", include_str!("law/nature.law.json")),
    ("sand", include_str!("law/sand.law.json")),
    ("stir", include_str!("law/stir.law.json")),
];

fn planed_law(planes: u32) -> String {
    let words: Vec<String> = (0..planes)
        .map(|plane| format!("[\"add\",[\"at\",\"c\",{plane}],{plane}]"))
        .collect();
    format!(
        "{{\"law\":1,\"name\":\"planed{planes}\",\"params\":1,\"channels\":{planes},\"halo\":0,\"tally\":{{\"counts\":1,\"weighted\":false}},\"consts\":{{}},\"defs\":{{}},\"step\":[\"let\",[[\"out\",[\"map\",\"c\",\"cells\",[\"vec\",{}]]]],[\"vec\",\"out\",[\"vec\",[\"param\",0]]]]}}",
        words.join(",")
    )
}

fn words_hash(words: &[u32]) -> String {
    let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
    grit::law::hex(&grit::branch::sha256(&bytes))
}

fn joined(passes: &[Vec<u32>]) -> Vec<u32> {
    let mut out = Vec::new();
    for pass in passes {
        out.push(u32::try_from(pass.len()).unwrap());
        out.extend_from_slice(pass);
    }
    out
}

fn kernel_catalog(width: u32) -> Vec<(String, String)> {
    let grid = grit::sand::Grid::solid(16, 16, 16).unwrap();
    let plan = grit::compact::Plan::over_in(
        width,
        grid.blocks().div_ceil(grit::sweep::cells(width)),
        grit::layout::Dimension::Three,
    )
    .unwrap();
    let mut texts: Vec<(String, String)> = SHIPPED
        .iter()
        .map(|(name, text)| ((*name).to_owned(), (*text).to_owned()))
        .collect();
    for planes in 2..=4 {
        texts.push((format!("planed{planes}"), planed_law(planes)));
    }
    let mut out = Vec::new();
    for (name, text) in &texts {
        let law = grit::law::Law::compile(text).unwrap();
        let whole = grit::sand3::settle_by(&law, width, grid, u32::MAX).unwrap();
        out.push((
            format!("{width} sand3::settle_by/{name}"),
            words_hash(&whole.words),
        ));
        let listed = grit::live::settle3_by(&law, width, grid).unwrap();
        out.push((
            format!("{width} live::settle3_by/{name}"),
            words_hash(&listed),
        ));
        for coarse in [false, true] {
            let (passes, _) = grit::live::arranged3_by(&law, width, grid, &plan, coarse).unwrap();
            out.push((
                format!("{width} live::arranged3_by/{name}/{coarse}"),
                words_hash(&joined(&passes)),
            ));
        }
        let switch =
            u32::try_from(grit::live::sizes3(&law, &plan)[grit::layout::PARAMS as usize]).unwrap();
        let (passes, _) = grit::live::wired3_switched_by(&law, width, grid, &plan, switch).unwrap();
        out.push((
            format!("{width} live::wired3_switched_by/{name}"),
            words_hash(&joined(&passes)),
        ));
        let pool = grit::bricks::Bricks::new(grid, &plan, plan.blocks)
            .unwrap()
            .with_planes(u32::try_from(grit::rule3::Declared::planes(&law)).unwrap())
            .unwrap();
        let (passes, _) = grit::bricks::arranged(&law, &pool, &plan, true).unwrap();
        out.push((
            format!("{width} bricks::arranged/{name}"),
            words_hash(&joined(&passes)),
        ));
    }
    out
}
