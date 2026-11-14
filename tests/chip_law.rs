#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use std::collections::BTreeSet;

use common::device;

use grit::compact::Plan;
use grit::halo::{self, BOX};
use grit::law::Law;
use grit::law::chip::{Circuit, Read, Rest, Spot, Wired, Wiring};
use grit::law::quiet::{self, Class};
use grit::layout::{DENSE, Dimension, GRAND, PARAMS, PIXELS, STATE, Windows};
use grit::live::{self, Placed3};
use grit::netlist::{Chip, Gate, Netlist};
use grit::rule3::{CORNERS, Declared, Oct, Rule};
use grit::sand::Grid;
use grit::sand3::{self, Grid3};
use grit::serve3::Server;
use grit::wire::{Json, read, written};
use grit::{Groups, Host, Resident, sweep};

const X0_TASK: &str = include_str!("chip_law/x0_task.json");
const X0_HELD: &str = include_str!("chip_law/x0_held_answers.json");
const X0_FIT: &str = include_str!("chip_law/x0_fit_24.json");
const EDGE: &str = include_str!("halo3/edge2d.law.json");

const KINDS: u32 = 16;

fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut mixed = *state;
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    mixed ^ (mixed >> 31)
}

fn pick(state: &mut u64, most: u32) -> u32 {
    (next(state) % u64::from(most)) as u32
}

fn step_of(state: &mut u64) -> i64 {
    i64::from(pick(state, 3)) - 1
}

fn chip_of(circuit: &Circuit) -> Chip {
    let gates = circuit.gates.len() as u32;
    let carried = circuit.state + circuit.outputs;
    let shape = Netlist::over(circuit.state + circuit.inputs, carried, gates, KINDS).unwrap();
    Chip::of(
        shape,
        circuit.gates.iter().map(|gate| gate.pack()).collect(),
        (gates - carried..gates).collect(),
    )
    .unwrap()
}

fn drawn_circuit(state: &mut u64, held: u32, inputs: u32, outputs: u32, extra: u32) -> Circuit {
    let count = held + outputs + extra;
    let gates = (0..count)
        .map(|at| {
            let sources = held + inputs + at;
            Gate {
                kind: pick(state, KINDS),
                left: pick(state, sources),
                right: pick(state, sources),
            }
        })
        .collect();
    Circuit {
        state: held,
        inputs,
        outputs,
        gates,
    }
}

fn drawn_wiring(
    state: &mut u64,
    circuit: &Circuit,
    channels: u32,
    outside: bool,
    rest: Rest,
) -> Wiring {
    let written: Vec<u32> = if channels == 1 {
        vec![0]
    } else {
        loop {
            let held: Vec<u32> = (1..channels).filter(|_| pick(state, 2) == 0).collect();
            if !held.is_empty() {
                break held;
            }
        }
    };
    let mut taken: BTreeSet<(u32, [i64; 3], u32)> = BTreeSet::new();
    let spot = |state: &mut u64, taken: &mut BTreeSet<(u32, [i64; 3], u32)>| loop {
        let plane = written[pick(state, written.len() as u32) as usize];
        let bit = pick(state, 32);
        if taken
            .iter()
            .all(|(held, _, one)| (*held, *one) != (plane, bit))
        {
            taken.insert((plane, [0, 0, 0], bit));
            break Spot { plane, bit };
        }
    };
    let keeps: Vec<Spot> = (0..circuit.state)
        .map(|_| spot(state, &mut taken))
        .collect();
    let writes: Vec<Spot> = (0..circuit.outputs)
        .map(|_| spot(state, &mut taken))
        .collect();
    let reads = (0..circuit.inputs)
        .map(|_| {
            loop {
                let plane = pick(state, channels);
                let at = if written.contains(&plane) || !outside {
                    [0, 0, 0]
                } else {
                    [step_of(state), step_of(state), step_of(state)]
                };
                let bit = pick(state, 32);
                if taken.insert((plane, at, bit)) {
                    break Read { plane, at, bit };
                }
            }
        })
        .collect();
    Wiring {
        name: String::from("chip"),
        channels,
        reads,
        writes,
        keeps,
        rest,
    }
}

fn row_of(wired: &Wired, read_at: impl Fn(&Read) -> u32, own: impl Fn(u32) -> u32) -> u32 {
    let wiring = wired.wiring();
    let held = wired.circuit().state as usize;
    let mut row = 0;
    for (at, spot) in wiring.keeps.iter().enumerate() {
        row |= ((own(spot.plane) >> spot.bit) & 1) << at;
    }
    for (at, one) in wiring.reads.iter().enumerate() {
        row |= ((read_at(one) >> one.bit) & 1) << (held + at);
    }
    row
}

fn placed(wired: &Wired, plane: u32, before: u32, answer: u32) -> u32 {
    let wiring = wired.wiring();
    let mine: Vec<(usize, &Spot)> = wiring
        .keeps
        .iter()
        .chain(&wiring.writes)
        .enumerate()
        .filter(|(_, spot)| spot.plane == plane)
        .collect();
    if mine.is_empty() {
        return before;
    }
    let mask = mine
        .iter()
        .fold(0_u32, |mask, (_, spot)| mask | (1 << spot.bit));
    let mut held = match wiring.rest {
        Rest::Kept => before & !mask,
        Rest::Cleared => 0,
    };
    for (at, spot) in mine {
        held |= ((answer >> at) & 1) << spot.bit;
    }
    held
}

fn offset(at: [i64; 3]) -> [i32; 3] {
    at.map(|one| i32::try_from(one).unwrap())
}

fn compiled(text: &str) -> Law {
    Law::compile(text).unwrap_or_else(|why| panic!("{why}\n{text}"))
}

fn rows_checked(wired: &Wired, law: &Law, rows: impl Iterator<Item = u32>, state: &mut u64) -> u64 {
    let chip = chip_of(wired.circuit());
    let planes = wired.wiring().channels as usize;
    let own_cell = |corner: usize| halo::looked(corner, [0, 0, 0]).unwrap();
    let mut checked = 0_u64;
    for row in rows {
        let target = (row % 8) as usize;
        let mut boxes: Vec<[u32; BOX]> = (0..planes)
            .map(|_| core::array::from_fn(|_| next(state) as u32))
            .collect();
        let mut set = |plane: u32, cell: usize, bit: u32, value: u32| {
            let word = &mut boxes[plane as usize][cell];
            *word = (*word & !(1 << bit)) | (value << bit);
        };
        let held = wired.circuit().state;
        for (at, spot) in wired.wiring().keeps.iter().enumerate() {
            set(spot.plane, own_cell(target), spot.bit, (row >> at) & 1);
        }
        for (at, one) in wired.wiring().reads.iter().enumerate() {
            let cell = halo::looked(target, offset(one.at)).unwrap();
            set(one.plane, cell, one.bit, (row >> (held as usize + at)) & 1);
        }
        let own = |plane: usize, corner: usize| boxes[plane][own_cell(corner)];
        let words: [u32; CORNERS] = core::array::from_fn(|corner| own(0, corner));
        let upper: Vec<[u32; CORNERS]> = (1..planes)
            .map(|plane| core::array::from_fn(|corner| own(plane, corner)))
            .collect();
        let got = law.step_halo(
            Oct {
                words,
                frame: 0,
                at: [0; 3],
                params: &[],
            },
            &upper,
            &boxes,
        );
        let mut out = vec![got.ruled.out];
        out.extend(got.planes);
        let mut changed = 0;
        for corner in 0..CORNERS {
            let gathered = row_of(
                wired,
                |one| boxes[one.plane as usize][halo::looked(corner, offset(one.at)).unwrap()],
                |plane| own(plane as usize, corner),
            );
            if corner == target {
                assert_eq!(gathered, row);
            }
            let answer = chip.answers(gathered);
            let mut moved = false;
            for (plane, words) in out.iter().enumerate() {
                let want = placed(wired, plane as u32, own(plane, corner), answer);
                assert_eq!(
                    words[corner],
                    want,
                    "row {gathered:#x} corner {corner} plane {plane}: the law gives {:#x}, the chip {want:#x}\n{}",
                    words[corner],
                    wired.text()
                );
                moved |= want != own(plane, corner);
            }
            changed += u32::from(moved);
        }
        assert_eq!(got.ruled.counts, vec![changed]);
        checked += 1;
    }
    checked
}

#[test]
fn every_input_row_of_a_chip_steps_as_the_chip_on_the_cpu() {
    let mut state = 0xB1_C41F_u64;
    let mut kinds = [false; KINDS as usize];
    let mut shapes = BTreeSet::new();
    let mut rows = 0_u64;
    for case in 0..48_u32 {
        let held = if case % 3 == 0 {
            0
        } else {
            1 + pick(&mut state, 3)
        };
        let reads = if case % 4 == 0 {
            16
        } else {
            1 + pick(&mut state, 16)
        };
        let held = held.min(reads);
        let inputs = reads - held;
        let outputs = 1 + pick(&mut state, 3);
        let extra = pick(&mut state, 25);
        let circuit = drawn_circuit(&mut state, held, inputs, outputs, extra);
        let channels = 1 + pick(&mut state, 4);
        let outside = channels > 1 && case % 5 != 1;
        let rest = if case % 2 == 0 {
            Rest::Kept
        } else {
            Rest::Cleared
        };
        let wiring = drawn_wiring(&mut state, &circuit, channels, outside, rest);
        for gate in &circuit.gates {
            kinds[gate.kind as usize] = true;
        }
        let wired = Wired::new(circuit, wiring).unwrap();
        let law = compiled(&wired.text());
        assert_eq!(law.halo(), wired.halo());
        assert_eq!(law.halo() == 1, !law.looks().is_empty());
        assert_eq!(
            law.halo() == 1,
            wired.wiring().reads.iter().any(|one| one.at != [0, 0, 0])
        );
        shapes.insert((held > 0, law.halo(), rest == Rest::Kept, channels));
        rows += rows_checked(&wired, &law, 0..1_u32 << reads, &mut state);
    }
    assert!(kinds.iter().all(|seen| *seen), "{kinds:?}");
    for wanted in [(false, 0), (false, 1), (true, 0), (true, 1)] {
        assert!(
            shapes.iter().any(|one| (one.0, one.1) == wanted),
            "{wanted:?} {shapes:?}"
        );
    }
    println!("48 chips, {rows} input rows, every row equal to the chip on the CPU twin");
}

#[test]
fn a_wide_chip_steps_as_the_chip_on_sampled_boxes() {
    let mut state = 0xB1_77DE_u64;
    for (held, inputs) in [(0, 24), (3, 28), (7, 20)] {
        let circuit = drawn_circuit(&mut state, held, inputs, 2, 40);
        let wiring = drawn_wiring(&mut state, &circuit, 4, true, Rest::Kept);
        let wired = Wired::new(circuit, wiring).unwrap();
        let law = compiled(&wired.text());
        let width = held + inputs;
        let mut sampler = state ^ 0x5A;
        let rows: Vec<u32> = (0..1_u32 << 16)
            .map(|_| pick(&mut sampler, 1 << width))
            .collect();
        assert_eq!(
            rows_checked(&wired, &law, rows.into_iter(), &mut state),
            1 << 16
        );
    }
}

fn grids((wide, deep, tall): (u32, u32, u32)) -> (Grid, Grid3) {
    (
        Grid::solid(wide, deep, tall).unwrap(),
        Grid3::new(wide, deep, tall).unwrap(),
    )
}

fn wrapped(grid: Grid3, at: [u32; 3], step: [i64; 3]) -> usize {
    let sides = [grid.wide, grid.deep, grid.tall];
    let moved: [u32; 3] = core::array::from_fn(|axis| {
        let side = i64::from(sides[axis]);
        u32::try_from((i64::from(at[axis]) + step[axis]).rem_euclid(side)).unwrap()
    });
    grid.index(moved[0], moved[1], moved[2])
}

fn point(grid: Grid3, cell: usize) -> [u32; 3] {
    let cell = cell as u32;
    [
        cell % grid.wide,
        (cell / grid.wide) % grid.deep,
        cell / (grid.wide * grid.deep),
    ]
}

fn chip_row(wired: &Wired, grid: Grid3, cells: &[u32], cell: usize) -> u32 {
    let count = grid.cells();
    let at = point(grid, cell);
    row_of(
        wired,
        |one| cells[one.plane as usize * count + wrapped(grid, at, one.at)],
        |plane| cells[plane as usize * count + cell],
    )
}

fn chip_step(wired: &Wired, chip: &Chip, grid: Grid3, cells: &[u32]) -> Vec<u32> {
    let count = grid.cells();
    let mut out = cells.to_vec();
    for cell in 0..count {
        let answer = chip.answers(chip_row(wired, grid, cells, cell));
        for plane in wired.written() {
            let slot = plane as usize * count + cell;
            out[slot] = placed(wired, plane, cells[slot], answer);
        }
    }
    out
}

fn every_row_filled(wired: &Wired, grid: Grid3, state: &mut u64) -> Vec<u32> {
    let count = grid.cells();
    let inputs = wired.circuit().inputs;
    let mut plane: Vec<u32> = (0..count).map(|_| next(state) as u32).collect();
    for (at, one) in wired.wiring().reads.iter().enumerate() {
        assert_eq!(one.plane, 0);
        for cell in 0..count {
            let target = cell as u32 % (1 << inputs);
            let bit = (target >> at) & 1;
            let word = &mut plane[wrapped(grid, point(grid, cell), one.at)];
            *word = (*word & !(1 << one.bit)) | (bit << one.bit);
        }
    }
    plane
}

fn plane_rows(wired: &Wired, grid: Grid3, cells: &[u32]) -> usize {
    let held = wired.circuit().state;
    (0..grid.cells())
        .map(|cell| chip_row(wired, grid, cells, cell) >> held)
        .collect::<BTreeSet<u32>>()
        .len()
}

const SHAPE: (u32, u32, u32) = (16, 16, 8);

const FRAMES: u32 = 64;

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
    want: Vec<u32>,
    frame: u32,
    label: String,
    fewest: u32,
}

impl<'host> Engine<'host> {
    fn new(host: &'host Host, law: &Law, coarse: bool) -> Option<Self> {
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
            want: Vec::new(),
            frame: 0,
            label: String::new(),
            fewest: u32::MAX,
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
        }
        live::seed(&mut self.live, self.grid, &self.plan).unwrap();
        self.want = start.to_vec();
        self.frame = 0;
        self.label = label;
        self.fewest = u32::MAX;
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
        panic!(
            "{}: frame {}: {what} differs at plane {} cell {}: {} against {}",
            self.label,
            self.frame,
            at / cells,
            at % cells,
            got[at],
            self.want[at]
        );
    }

    fn step(&mut self, wired: &Wired, chip: &Chip) {
        let words = self.want.len();
        let stages: Vec<&[Groups]> = self.stages.iter().map(Vec::as_slice).collect();
        self.live.staged(&stages).unwrap();
        let listed = self.live.read(STATE as usize, words).unwrap();
        let whole: Vec<&[Groups]> = self.whole.iter().map(core::slice::from_ref).collect();
        let (_, full) = self
            .full
            .staged_reading(&whole, STATE as usize, words)
            .unwrap();
        let by_chip = chip_step(wired, chip, self.reference, &self.want);
        self.want = sand3::reference(&self.law, self.reference, self.frame, &self.want, &[]);
        self.parted("the chip stepped per cell", &by_chip);
        self.parted("the listed world", &listed);
        self.parted("the whole grid", &full);
        let grand = self.live.read(GRAND as usize, 1).unwrap()[0];
        let _ = self.live.read(DENSE as usize, grand as usize).unwrap();
        self.fewest = self.fewest.min(grand);
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
        }
    }
}

fn proved_chip(state: &mut u64, held: u32, inputs: u32, rest: Rest) -> Wired {
    loop {
        let extra = 12 + pick(state, 12);
        let circuit = drawn_circuit(state, held, inputs, 2, extra);
        if chip_of(&circuit).answers(0) != 0 {
            continue;
        }
        let mut bits: Vec<u32> = (0..32).collect();
        for at in (1..bits.len()).rev() {
            bits.swap(at, pick(state, at as u32 + 1) as usize);
        }
        let reads = (0..inputs as usize)
            .map(|at| Read {
                plane: 0,
                at: [step_of(state), step_of(state), step_of(state)],
                bit: bits[at],
            })
            .collect();
        let mut spots: Vec<Spot> = Vec::new();
        while spots.len() < (held + 2) as usize {
            let spot = Spot {
                plane: 1,
                bit: pick(state, 32),
            };
            if !spots.contains(&spot) {
                spots.push(spot);
            }
        }
        let writes = spots.split_off(held as usize);
        let wiring = Wiring {
            name: String::from("chip"),
            channels: 3,
            reads,
            writes,
            keeps: spots,
            rest,
        };
        return Wired::new(circuit, wiring).unwrap();
    }
}

#[test]
fn a_proved_chip_law_steps_listed_as_whole_as_the_chip_on_the_device() {
    let Some(host) = device("chip law listed whole cpu") else {
        return;
    };
    let mut state = 0xB2_0DE5_u64;
    let (_, grid) = grids(SHAPE);
    let count = grid.cells();
    for (held, inputs, rest) in [(0, 11, Rest::Kept), (2, 9, Rest::Cleared)] {
        let wired = proved_chip(&mut state, held, inputs, rest);
        let chip = chip_of(wired.circuit());
        let law = compiled(&wired.text());
        assert_eq!(law.halo(), 1);
        assert_eq!(quiet::class(&law, &[], &[]), Class::Proved);
        let mut start = every_row_filled(&wired, grid, &mut state);
        start.extend((0..count).map(|_| next(&mut state) as u32));
        start.extend((0..count).map(|_| next(&mut state) as u32));
        assert_eq!(plane_rows(&wired, grid, &start), 1 << inputs);
        for coarse in [false, true] {
            let Some(mut engine) = Engine::new(&host, &law, coarse) else {
                continue;
            };
            let tier = if coarse { "coarse" } else { "fine" };
            engine.restart(&start, format!("state {held} inputs {inputs} {tier}"));
            while engine.frame < FRAMES {
                if engine.frame > 0 && engine.frame.is_multiple_of(16) {
                    let writes: Vec<(usize, u32, u32)> = (0..24)
                        .map(|_| {
                            (
                                pick(&mut state, 2) as usize,
                                pick(&mut state, count as u32),
                                next(&mut state) as u32,
                            )
                        })
                        .collect();
                    engine.write(&writes);
                }
                engine.step(&wired, &chip);
            }
            println!(
                "{} on {} width {}: {FRAMES} frames listed == whole == CPU == chip per cell, fewest listed {} of {}",
                engine.label,
                host.name(),
                host.width(),
                engine.fewest,
                engine.grid.blocks()
            );
        }
    }
}

fn line(name: &str, arguments: &str) -> String {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{{\"name\":\"{name}\",\"arguments\":{arguments}}}}}"
    )
}

fn asked(server: &mut Server<'_>, name: &str, arguments: &str) -> Json {
    server.heard(&line(name, arguments)).unwrap()
}

fn answered(got: &Json) -> Json {
    let result = got.get("result").unwrap();
    assert_eq!(
        result.get("isError"),
        Some(&Json::Bool(false)),
        "{}",
        written(got)
    );
    result.get("structuredContent").cloned().unwrap()
}

fn refused(got: &Json) -> Json {
    let result = got.get("result").unwrap();
    assert_eq!(
        result.get("isError"),
        Some(&Json::Bool(true)),
        "{}",
        written(got)
    );
    result.get("structuredContent").cloned().unwrap()
}

fn text_at(json: &Json, key: &str) -> String {
    json.get(key)
        .and_then(Json::text)
        .unwrap_or_else(|| panic!("{key} in {}", written(json)))
        .to_owned()
}

fn int_at(json: &Json, key: &str) -> i64 {
    json.get(key)
        .and_then(Json::int)
        .unwrap_or_else(|| panic!("{key} in {}", written(json)))
}

fn gates_text(gates: &[Gate]) -> String {
    let each: Vec<String> = gates
        .iter()
        .map(|gate| {
            format!(
                "{{\"kind\":{},\"left\":{},\"right\":{}}}",
                gate.kind, gate.left, gate.right
            )
        })
        .collect();
    format!("[{}]", each.join(","))
}

fn spots_text(spots: &[Spot]) -> String {
    let each: Vec<String> = spots
        .iter()
        .map(|spot| format!("{{\"plane\":{},\"bit\":{}}}", spot.plane, spot.bit))
        .collect();
    format!("[{}]", each.join(","))
}

fn chip_arguments(circuit: &Circuit, wiring: &Wiring) -> String {
    let reads: Vec<String> = wiring
        .reads
        .iter()
        .map(|one| {
            format!(
                "{{\"plane\":{},\"dx\":{},\"dy\":{},\"dz\":{},\"bit\":{}}}",
                one.plane, one.at[0], one.at[1], one.at[2], one.bit
            )
        })
        .collect();
    format!(
        "{{\"inputs\":{},\"outputs\":{},\"state\":{},\"gates\":{},\"bind_inputs\":[{}],\"bind_outputs\":{},\"bind_state\":{},\"channels\":{},\"rest\":\"{}\",\"name\":{}}}",
        circuit.inputs,
        circuit.outputs,
        circuit.state,
        gates_text(&circuit.gates),
        reads.join(","),
        spots_text(&wiring.writes),
        spots_text(&wiring.keeps),
        wiring.channels,
        wiring.rest.name(),
        quoted(&wiring.name)
    )
}

fn quoted(text: &str) -> String {
    written(&Json::Text(text.to_owned()))
}

fn world_of(server: &mut Server<'_>, law: &str, size: [u32; 3]) -> i64 {
    let made = answered(&asked(
        server,
        "grit_world_create",
        &format!(
            "{{\"wide\":{},\"deep\":{},\"tall\":{},\"law\":{}}}",
            size[0],
            size[1],
            size[2],
            quoted(law)
        ),
    ));
    int_at(&made, "world")
}

fn write_plane0(server: &mut Server<'_>, world: i64, cells: &[([u32; 3], u32)]) {
    for chunk in cells.chunks(4096) {
        let each: Vec<String> = chunk
            .iter()
            .map(|(at, word)| format!("[{},{},{},{word}]", at[0], at[1], at[2]))
            .collect();
        let got = answered(&asked(
            server,
            "grit_world_write",
            &format!("{{\"world\":{world},\"cells\":[{}]}}", each.join(",")),
        ));
        assert_eq!(int_at(&got, "written"), chunk.len() as i64);
    }
}

fn read_plane(
    server: &mut Server<'_>,
    world: i64,
    from: [u32; 3],
    to: [u32; 3],
    plane: u32,
) -> Vec<u32> {
    let got = answered(&asked(
        server,
        "grit_world_read",
        &format!(
            "{{\"world\":{world},\"from\":[{},{},{}],\"to\":[{},{},{}],\"plane\":{plane}}}",
            from[0], from[1], from[2], to[0], to[1], to[2]
        ),
    ));
    got.get("cells")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .map(|one| u32::try_from(one.int().unwrap()).unwrap())
        .collect()
}

fn step_world(server: &mut Server<'_>, world: i64, frames: u32) {
    answered(&asked(
        server,
        "grit_world_step",
        &format!("{{\"world\":{world},\"frames\":{frames}}}"),
    ));
}

fn whole_plane(server: &mut Server<'_>, world: i64, grid: Grid3, plane: u32) -> Vec<u32> {
    let layer = grid.wide * grid.deep;
    let layers = (4096 / layer).max(1);
    let mut out = Vec::new();
    let mut z = 0;
    while z < grid.tall {
        let last = (z + layers - 1).min(grid.tall - 1);
        out.extend(read_plane(
            server,
            world,
            [0, 0, z],
            [grid.wide - 1, grid.deep - 1, last],
            plane,
        ));
        z = last + 1;
    }
    out
}

fn law_through_tool(server: &mut Server<'_>, wired: &Wired) -> Json {
    answered(&asked(
        server,
        "grit_law_from_chip",
        &chip_arguments(wired.circuit(), wired.wiring()),
    ))
}

#[test]
fn a_chip_law_from_the_tool_answers_every_input_row_on_the_device() {
    let Some(host) = device("chip law every row through the tools") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let mut state = 0xB2_EEE1_u64;
    for (inputs, rest) in [(12, Rest::Cleared), (12, Rest::Kept)] {
        let wired = proved_chip(&mut state, 0, inputs, rest);
        let chip = chip_of(wired.circuit());
        let told = law_through_tool(&mut server, &wired);
        let law = text_at(&told, "law");
        assert_eq!(law, wired.text());
        assert_eq!(text_at(&told, "hash"), compiled(&law).hex());
        assert_eq!(int_at(&told, "halo"), 1);
        assert_eq!(text_at(&told, "quiet"), "proved");
        let grid = Grid3::new(64, 64, 2).unwrap();
        let count = grid.cells();
        let world = world_of(&mut server, &law, [64, 64, 2]);
        let plane0 = every_row_filled(&wired, grid, &mut state);
        let mut cells = plane0.clone();
        cells.extend(vec![0_u32; 2 * count]);
        assert_eq!(plane_rows(&wired, grid, &cells), 1 << inputs);
        let writes: Vec<([u32; 3], u32)> = plane0
            .iter()
            .enumerate()
            .map(|(cell, word)| (point(grid, cell), *word))
            .collect();
        write_plane0(&mut server, world, &writes);
        step_world(&mut server, world, 1);
        let want = chip_step(&wired, &chip, grid, &cells);
        for plane in 0..3 {
            let got = whole_plane(&mut server, world, grid, plane);
            assert_eq!(
                got,
                want[plane as usize * count..(plane as usize + 1) * count],
                "plane {plane}"
            );
        }
        println!(
            "{inputs} inputs, rest {}: every one of {} rows occurs on a {count} cell world; plane 1 equals the chip on every cell",
            rest.name(),
            1 << inputs
        );
    }
}

#[test]
fn a_chip_with_state_runs_64_frames_as_the_chip_stepped_64_times() {
    let Some(host) = device("chip law with state through the tools") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let mut state = 0xB5_57A7_u64;
    let grid = Grid3::new(16, 16, 2).unwrap();
    let count = grid.cells();
    let mut runs = 0;
    while runs < 2 {
        let circuit = drawn_circuit(&mut state, 3, 4, 1, 16);
        let mut wiring = drawn_wiring(&mut state, &circuit, 2, true, Rest::Kept);
        for one in &mut wiring.reads {
            one.plane = 0;
            one.bit = 0;
        }
        for (at, one) in wiring.reads.iter_mut().enumerate() {
            one.at = [
                i64::from(at as u32 % 3) - 1,
                i64::from(at as u32 / 3) - 1,
                0,
            ];
        }
        let wired = Wired::new(circuit, wiring).unwrap();
        let chip = chip_of(wired.circuit());
        let told = law_through_tool(&mut server, &wired);
        let world = world_of(&mut server, &text_at(&told, "law"), [16, 16, 2]);
        let plane0: Vec<u32> = (0..count).map(|_| pick(&mut state, 2)).collect();
        let writes: Vec<([u32; 3], u32)> = plane0
            .iter()
            .enumerate()
            .map(|(cell, word)| (point(grid, cell), *word))
            .collect();
        write_plane0(&mut server, world, &writes);
        let mut cells = plane0;
        cells.extend(vec![0_u32; count]);
        let keeps = wired.wiring().keeps.clone();
        let held = |cells: &[u32]| -> Vec<u32> {
            (0..count)
                .map(|cell| {
                    keeps.iter().enumerate().fold(0, |row, (at, spot)| {
                        row | (((cells[count + cell] >> spot.bit) & 1) << at)
                    })
                })
                .collect()
        };
        let mut moves = 0_u32;
        for frame in 0..FRAMES {
            let before = held(&cells);
            cells = chip_step(&wired, &chip, grid, &cells);
            step_world(&mut server, world, 1);
            let got = whole_plane(&mut server, world, grid, 1);
            assert_eq!(got, cells[count..], "frame {frame}");
            moves += u32::from(before != held(&cells) && frame > 0);
        }
        if moves == 0 {
            continue;
        }
        runs += 1;
        println!(
            "state {} bits: {FRAMES} frames equal the chip stepped per cell, state changed on {moves} frames after the first, quiet {}",
            keeps.len(),
            text_at(&told, "quiet")
        );
    }
}

type Image = Vec<Vec<u32>>;

fn image(json: &Json) -> Image {
    json.list()
        .unwrap()
        .iter()
        .map(|row| {
            row.list()
                .unwrap()
                .iter()
                .map(|one| u32::try_from(one.int().unwrap()).unwrap())
                .collect()
        })
        .collect()
}

fn neighbourhood(img: &[Vec<u32>], y: usize, x: usize, torus: bool) -> u32 {
    let side = img.len() as i64;
    let mut word = 0;
    let mut bit = 0;
    for dy in -1..=1_i64 {
        for dx in -1..=1_i64 {
            let (yy, xx) = (y as i64 + dy, x as i64 + dx);
            let value = if torus {
                img[yy.rem_euclid(side) as usize][xx.rem_euclid(side) as usize]
            } else if (0..side).contains(&yy) && (0..side).contains(&xx) {
                img[yy as usize][xx as usize]
            } else {
                0
            };
            word |= value << bit;
            bit += 1;
        }
    }
    word
}

fn sobel(word: u32) -> u32 {
    let bit = |at: u32| i64::from((word >> at) & 1);
    let gx = (bit(2) + 2 * bit(5) + bit(8)) - (bit(0) + 2 * bit(3) + bit(6));
    let gy = (bit(6) + 2 * bit(7) + bit(8)) - (bit(0) + 2 * bit(1) + bit(2));
    u32::from(gx.abs() + gy.abs() >= 4)
}

fn x0_circuit() -> Circuit {
    let fit = read(X0_FIT).unwrap();
    let gates: Vec<Gate> = fit
        .get("gates")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .map(|one| Gate {
            kind: one.counted("kind").unwrap(),
            left: one.counted("left").unwrap(),
            right: one.counted("right").unwrap(),
        })
        .collect();
    assert_eq!(gates.len(), 24);
    Circuit {
        state: 0,
        inputs: 9,
        outputs: 1,
        gates,
    }
}

fn x0_wiring(rest: Rest) -> Wiring {
    Wiring {
        name: String::from("x0_edge"),
        channels: 2,
        reads: (0..9_i64)
            .map(|k| Read {
                plane: 0,
                at: [k % 3 - 1, k / 3 - 1, 0],
                bit: 0,
            })
            .collect(),
        writes: vec![Spot { plane: 1, bit: 0 }],
        keeps: Vec::new(),
        rest,
    }
}

struct Loop {
    torus: Vec<Vec<Vec<u32>>>,
    padded: Vec<Vec<Vec<u32>>>,
}

fn image_cells(img: &[Vec<u32>]) -> Vec<([u32; 3], u32)> {
    let mut cells = Vec::new();
    for (y, row) in img.iter().enumerate() {
        for (x, value) in row.iter().enumerate() {
            cells.push(([x as u32, y as u32, 0], *value));
        }
    }
    cells
}

fn x0_loop(host: &Host) -> Loop {
    let task = read(X0_TASK).unwrap();
    let side = task.counted("size").unwrap() as usize;
    let train: Vec<(Image, Image)> = task
        .get("train")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .map(|one| {
            (
                image(one.get("input").unwrap()),
                image(one.get("output").unwrap()),
            )
        })
        .collect();
    let held: Vec<Vec<Vec<u32>>> = task
        .get("held")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .map(|one| image(one.get("input").unwrap()))
        .collect();
    let mut server = Server::new(Some(host));
    let circuit = x0_circuit();
    let traces: Vec<String> = train
        .iter()
        .map(|(input, output)| {
            let feed: Vec<String> = (0..side * side)
                .map(|at| neighbourhood(input, at / side, at % side, false).to_string())
                .collect();
            let want: Vec<String> = output.iter().flatten().map(u32::to_string).collect();
            format!(
                "{{\"feed\":[{}],\"want\":[{}]}}",
                feed.join(","),
                want.join(",")
            )
        })
        .collect();
    let marked = answered(&asked(
        &mut server,
        "grit_machine",
        &format!(
            "{{\"state\":0,\"inputs\":9,\"outputs\":1,\"gates\":24,\"chip\":{},\"traces\":[{}]}}",
            gates_text(&circuit.gates),
            traces.join(",")
        ),
    ));
    assert_eq!(marked.get("solved"), Some(&Json::Bool(true)));
    assert_eq!(int_at(&marked, "marks"), int_at(&marked, "of"));
    let wiring = x0_wiring(Rest::Cleared);
    let told = answered(&asked(
        &mut server,
        "grit_law_from_chip",
        &chip_arguments(&circuit, &wiring),
    ));
    assert_eq!(int_at(&told, "halo"), 1);
    assert_eq!(text_at(&told, "quiet"), "proved");
    let law = text_at(&told, "law");
    let mut run = |size: u32| -> Vec<Vec<Vec<u32>>> {
        let world = world_of(&mut server, &law, [size, size, 2]);
        held.iter()
            .map(|img| {
                write_plane0(&mut server, world, &image_cells(img));
                step_world(&mut server, world, 1);
                let edge = side as u32 - 1;
                let got = read_plane(&mut server, world, [0, 0, 0], [edge, edge, 0], 1);
                got.chunks(side).map(<[u32]>::to_vec).collect()
            })
            .collect()
    };
    let torus = run(side as u32);
    let padded = run(2 * side as u32);
    Loop { torus, padded }
}

#[test]
fn the_x0_edge_chip_runs_on_a_world_as_exactly_the_chip() {
    let Some(host) = device("x0 loop through the tools") else {
        return;
    };
    let task = read(X0_TASK).unwrap();
    let side = task.counted("size").unwrap() as usize;
    let held: Vec<Vec<Vec<u32>>> = task
        .get("held")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .map(|one| image(one.get("input").unwrap()))
        .collect();
    let x0: Vec<Vec<Vec<u32>>> = read(X0_HELD)
        .unwrap()
        .list()
        .unwrap()
        .iter()
        .map(image)
        .collect();
    let chip = chip_of(&x0_circuit());
    let first = x0_loop(&host);
    let again = x0_loop(&host);
    assert_eq!(first.torus, again.torus);
    assert_eq!(first.padded, again.padded);
    let cells = held.len() * side * side;
    let mut torus_x0 = 0;
    let mut torus_sobel = 0;
    let mut padded_sobel = 0;
    let mut seam = 0;
    let mut seam_cells = 0;
    let places = (0..held.len()).flat_map(|index| (0..side * side).map(move |at| (index, at)));
    for (index, at) in places {
        let (y, x) = (at / side, at % side);
        let img = &held[index];
        let wrap = neighbourhood(img, y, x, true);
        let zero = neighbourhood(img, y, x, false);
        let got = first.torus[index][y][x];
        assert_eq!(
            got,
            chip.answers(wrap),
            "torus image {index} row {y} col {x}"
        );
        assert_eq!(
            first.padded[index][y][x],
            chip.answers(zero),
            "padded image {index} row {y} col {x}"
        );
        assert_eq!(first.padded[index][y][x], x0[index][y][x]);
        torus_x0 += usize::from(got == x0[index][y][x]);
        torus_sobel += usize::from(got == sobel(zero));
        padded_sobel += usize::from(first.padded[index][y][x] == sobel(zero));
        seam_cells += usize::from(wrap != zero);
        seam += usize::from(wrap != zero && chip.answers(wrap) != chip.answers(zero));
    }
    println!(
        "x0 loop: torus {side}x{side} world equals the chip on its wrapped 3x3 on {cells} of {cells} cells; padded {}x{} world equals the chip on X0's zero border and X0's held answers on {cells} of {cells}",
        2 * side,
        2 * side
    );
    println!(
        "x0 loop: against X0's held answers the torus world matches {torus_x0} of {cells}; against Sobel |gx|+|gy|>=4 with X0's zero border the torus world matches {torus_sobel}, the padded world {padded_sobel}"
    );
    println!(
        "x0 loop: {seam_cells} cells have a wrapped 3x3 unlike their zero-border 3x3; the chip answers differently on {seam} of them"
    );
}

fn base_arguments() -> String {
    chip_arguments(&x0_circuit(), &x0_wiring(Rest::Kept))
}

fn edited(change: impl FnOnce(&mut Circuit, &mut Wiring)) -> String {
    let mut circuit = x0_circuit();
    let mut wiring = x0_wiring(Rest::Kept);
    change(&mut circuit, &mut wiring);
    chip_arguments(&circuit, &wiring)
}

fn replaced(arguments: &str, from: &str, to: &str) -> String {
    assert!(arguments.contains(from), "{from} in {arguments}");
    arguments.replacen(from, to, 1)
}

#[test]
fn every_refusal_of_the_chip_tool_is_named_with_its_path() {
    let mut server = Server::new(None);
    let base = base_arguments();
    let ok = answered(&asked(&mut server, "grit_law_from_chip", &base));
    assert_eq!(
        text_at(&ok, "law"),
        Wired::new(x0_circuit(), x0_wiring(Rest::Kept))
            .unwrap()
            .text()
    );
    let many = Circuit {
        state: 0,
        inputs: 2,
        outputs: 1,
        gates: (0..2400)
            .map(|at| Gate {
                kind: 6,
                left: at + 1,
                right: at,
            })
            .collect(),
    };
    let many_wiring = Wiring {
        name: String::from("chip"),
        channels: 2,
        reads: vec![
            Read {
                plane: 0,
                at: [1, 0, 0],
                bit: 0,
            },
            Read {
                plane: 0,
                at: [0, 1, 0],
                bit: 0,
            },
        ],
        writes: vec![Spot { plane: 1, bit: 0 }],
        keeps: Vec::new(),
        rest: Rest::Kept,
    };
    let cases: Vec<(&str, String, &str, &str)> = vec![
        (
            "an input without a binding",
            edited(|_, wiring| {
                wiring.reads.pop();
            }),
            "unbound",
            "$.bind_inputs",
        ),
        (
            "an output without a binding",
            edited(|_, wiring| wiring.writes.clear()),
            "unbound",
            "$.bind_outputs",
        ),
        (
            "a state bit without a binding",
            edited(|circuit, _| {
                circuit.state = 1;
                circuit.gates.insert(
                    0,
                    Gate {
                        kind: 8,
                        left: 0,
                        right: 1,
                    },
                );
                for gate in &mut circuit.gates[1..] {
                    gate.left += 1;
                    gate.right += 1;
                }
            }),
            "unbound",
            "$.bind_state",
        ),
        (
            "an input binding past the chip",
            edited(|_, wiring| {
                wiring.reads.push(Read {
                    plane: 0,
                    at: [0, 0, 0],
                    bit: 1,
                });
            }),
            "overbound",
            "$.bind_inputs[9]",
        ),
        (
            "an input bit past 31",
            replaced(&base, "\"dz\":0,\"bit\":0}", "\"dz\":0,\"bit\":32}"),
            "bit_past",
            "$.bind_inputs[0].bit",
        ),
        (
            "an output bit past 31",
            edited(|_, wiring| wiring.writes[0].bit = 40),
            "bit_past",
            "$.bind_outputs[0].bit",
        ),
        (
            "an output plane past channels",
            edited(|_, wiring| wiring.writes[0].plane = 2),
            "plane_past",
            "$.bind_outputs[0].plane",
        ),
        (
            "an input plane past every channel count",
            edited(|_, wiring| wiring.reads[4].plane = 9),
            "plane_past",
            "$.bind_inputs[4].plane",
        ),
        (
            "an offset past 1",
            edited(|_, wiring| wiring.reads[3].at = [0, -2, 0]),
            "offset_past",
            "$.bind_inputs[3].dy",
        ),
        (
            "two outputs on one bit",
            edited(|circuit, wiring| {
                circuit.outputs = 2;
                wiring.writes.push(Spot { plane: 1, bit: 0 });
            }),
            "bit_shared",
            "$.bind_outputs[1]",
        ),
        (
            "a state bit on an output's bit",
            edited(|circuit, wiring| {
                circuit.state = 1;
                circuit.gates.insert(
                    0,
                    Gate {
                        kind: 8,
                        left: 0,
                        right: 1,
                    },
                );
                for gate in &mut circuit.gates[1..] {
                    gate.left += 1;
                    gate.right += 1;
                }
                wiring.keeps.push(Spot { plane: 1, bit: 0 });
            }),
            "bit_shared",
            "$.bind_state[0]",
        ),
        (
            "a gate that reads a gate after it",
            edited(|circuit, _| circuit.gates[0].left = 9),
            "chip_refused",
            "$.gates[0]",
        ),
        (
            "a gate kind past 15",
            edited(|circuit, _| circuit.gates[2].kind = 16),
            "chip_refused",
            "$.gates[2].kind",
        ),
        (
            "fewer gates than outputs",
            edited(|circuit, _| {
                circuit.outputs = 1;
                circuit.gates.clear();
            }),
            "chip_refused",
            "$.gates",
        ),
        (
            "inputs and state past 31",
            edited(|circuit, _| circuit.state = 30),
            "chip_refused",
            "$.inputs",
        ),
        (
            "a look at a written plane",
            edited(|_, wiring| wiring.reads[2].plane = 1),
            "halo_written",
            "$.bind_inputs[2]",
        ),
        (
            "a chip too large for a law",
            chip_arguments(&many, &many_wiring),
            "law_refused",
            "$",
        ),
    ];
    for (what, arguments, code, field) in cases {
        let got = refused(&asked(&mut server, "grit_law_from_chip", &arguments));
        assert_eq!(text_at(&got, "refused"), code, "{what}: {}", written(&got));
        assert_eq!(text_at(&got, "field"), field, "{what}: {}", written(&got));
        assert!(text_at(&got, "hint").ends_with('.'), "{what}");
        if code == "law_refused" {
            let inner = got.get("law").unwrap();
            assert_eq!(text_at(inner, "code"), "nodes", "{what}: {}", written(&got));
        }
    }
}

#[test]
fn a_plane_past_the_world_is_refused_on_read_and_slice() {
    let Some(host) = device("chip law plane read refusal") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let law = Wired::new(x0_circuit(), x0_wiring(Rest::Kept))
        .unwrap()
        .text();
    let world = world_of(&mut server, &law, [16, 16, 2]);
    for (name, arguments) in [
        (
            "grit_world_read",
            format!("{{\"world\":{world},\"from\":[0,0,0],\"to\":[1,1,1],\"plane\":2}}"),
        ),
        (
            "grit_world_slice",
            format!("{{\"world\":{world},\"at\":0,\"plane\":2}}"),
        ),
    ] {
        let got = refused(&asked(&mut server, name, &arguments));
        assert_eq!(text_at(&got, "refused"), "outside", "{}", written(&got));
        assert_eq!(text_at(&got, "field"), "$.plane");
    }
    let sliced = answered(&asked(
        &mut server,
        "grit_world_slice",
        &format!("{{\"world\":{world},\"at\":0,\"plane\":1,\"as\":\"words\"}}"),
    ));
    assert_eq!(sliced.get("rows").and_then(Json::list).unwrap().len(), 16);
}

#[test]
#[ignore = "a timing measurement for notes.md, run alone with --ignored"]
fn the_cost_of_a_chip_law_against_the_edge_law() {
    use grit::branch::{Made, Stepping, World};
    let Some(host) = device("chip law cost") else {
        return;
    };
    let chip_law = compiled(
        &Wired::new(x0_circuit(), x0_wiring(Rest::Cleared))
            .unwrap()
            .text(),
    );
    let edge = compiled(EDGE);
    let size = [256_u32, 256, 16];
    let cells = (size[0] * size[1] * size[2]) as usize;
    let mut state = 0xC057_u64;
    let start: Vec<([u32; 3], u32)> = (0..cells)
        .map(|cell| {
            let cell = cell as u32;
            (
                [
                    cell % size[0],
                    (cell / size[0]) % size[1],
                    cell / (size[0] * size[1]),
                ],
                u32::from(next(&mut state).is_multiple_of(3)),
            )
        })
        .collect();
    let frames = 256;
    let octs = (cells / 8) as f64;
    let mut times: Vec<(&str, Vec<f64>)> = vec![("chip", Vec::new()), ("edge", Vec::new())];
    for _ in 0..3 {
        for (at, (law, params)) in [(&chip_law, vec![]), (&edge, vec![3])].iter().enumerate() {
            let made = Made {
                rule: law.name(),
                params,
                size,
                sparse: false,
                slots: None,
            };
            let mut world = World::create_law_stepped(&host, law, &made, Stepping::Whole).unwrap();
            world.write_plane(0, &start).unwrap();
            world.step(4).unwrap();
            let _ = world.cells_at(&[[0, 0, 0]]).unwrap();
            let began = std::time::Instant::now();
            world.step(frames).unwrap();
            let _ = world.cells_at(&[[0, 0, 0]]).unwrap();
            let spent = began.elapsed().as_secs_f64();
            times[at].1.push(spent * 1e9 / (octs * f64::from(frames)));
        }
    }
    for (name, mut held) in times {
        held.sort_by(f64::total_cmp);
        println!(
            "{name}: ns per oct stepped, min {:.3} median {:.3} over 3 rounds on {} ({}x{}x{}, {frames} frames whole)",
            held[0],
            held[1],
            host.name(),
            size[0],
            size[1],
            size[2]
        );
    }
    println!(
        "nodes: chip law {}, edge law {}",
        chip_law.nodes(),
        edge.nodes()
    );
}
