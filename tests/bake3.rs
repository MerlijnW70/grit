#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod baking;
mod common;

use baking::{Mesh, Node, Scene, cuboid, glb, node, sphere};
use common::device;

use grit::bake::{self, Asset, Map, Options};
use grit::compact::Plan;
use grit::layout::{BINDINGS, Dimension, GRAND, PARAMS, PIXELS, STATE};
use grit::live;
use grit::rule3::nature::{self, Nature};
use grit::sand::Grid;
use grit::sand3::{self, Grid3, reference};
use grit::sweep;
use grit::{Groups, Host, Resident};

const WORLDS: [(u32, u32, u32); 2] = [(16, 16, 16), (32, 16, 16)];

const TIERS: [bool; 2] = [false, true];

const HEAT: u32 = 300;

/// The rows from this one down are rock; the rest is air.
const FLOOR: u32 = 12;

/// How many frames each placement is held to the whole grid before the
/// world is let fall quiet again.
const FRAMES: u32 = 64;

const PATIENCE_FRAMES: u32 = 2048;

const MAP: &str = r#"{"rule":"nature","heat":300,"default":"ROCK","materials":{"bark":"WOOD","needles":"LEAVES","sand":"SAND"}}"#;

/// A pine: a trunk of bark, a crown of needles, and a lump of sand beside
/// the crown that falls once placed.
fn pine() -> Asset {
    let scene = Scene {
        meshes: vec![
            cuboid([-0.15, 0.0, -0.15], [0.15, 1.2, 0.15], Some(0)),
            Mesh {
                material: Some(1),
                ..sphere(0.6, 16, 8, false)
            },
            cuboid([0.7, 0.8, -0.1], [0.95, 1.1, 0.2], Some(2)),
        ],
        materials: vec![Some("bark"), Some("needles"), Some("sand")],
        nodes: vec![
            node(0),
            Node {
                mesh: 1,
                transform: r#","translation":[0,1.5,0]"#.to_owned(),
            },
            node(2),
        ],
    };
    let model = bake::read_glb(&glb(&scene)).unwrap();
    let map = Map::read(MAP.as_bytes()).unwrap();
    bake::bake(
        &model,
        &map,
        Options {
            size: 0.2,
            scale: 1.0,
        },
    )
    .unwrap()
}

fn valley(grid: Grid3) -> Vec<u32> {
    let mut cells = vec![nature::cell(nature::AIR, HEAT); grid.cells()];
    for z in 0..grid.tall {
        for y in FLOOR..grid.deep {
            for x in 0..grid.wide {
                cells[grid.index(x, y, z)] = nature::cell(nature::ROCK, HEAT);
            }
        }
    }
    cells
}

fn plan_of(width: u32, grid: Grid) -> Option<Plan> {
    match Plan::over_in(
        width,
        grid.blocks().div_ceil(sweep::cells(width)),
        Dimension::Three,
    ) {
        Ok(plan) => Some(plan),
        Err(too_big) => {
            common::skip(format_args!("width {width} {too_big}"));
            None
        }
    }
}

fn clear_all(held: &mut Resident<'_>, sizes: &[usize; BINDINGS as usize]) {
    for (binding, &words) in sizes.iter().enumerate() {
        if binding == PIXELS as usize {
            continue;
        }
        held.write(binding, &vec![0_u32; words]).unwrap();
    }
}

/// The listed engine and the whole-grid kernel under the nature rule on
/// the same solid world, stepped side by side and held to the CPU.
struct Engine<'host> {
    live: Resident<'host>,
    full: Resident<'host>,
    stages: Vec<Vec<Groups>>,
    whole: [Groups; 2],
    plan: Plan,
    grid: Grid,
    reference: Grid3,
    want: Vec<u32>,
    frame: u32,
    label: String,
}

impl<'host> Engine<'host> {
    fn new(host: &'host Host, (wide, deep, tall): (u32, u32, u32), coarse: bool) -> Option<Self> {
        let width = host.width();
        let grid = Grid::solid(wide, deep, tall).unwrap();
        let reference = Grid3::new(wide, deep, tall).unwrap();
        let plan = plan_of(width, grid)?;
        let (passes, stages) = live::arranged3_by(&Nature, width, grid, &plan, coarse).unwrap();
        let built = sand3::settle_by(&Nature, width, grid, host.limits().groups).unwrap();
        let tick = live::tick(width).unwrap();
        let sizes = live::sizes3(&Nature, &plan);
        let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
        let live = host.resident(&sizes, &modules).unwrap();
        let modules: Vec<&[u32]> = vec![&tick, &built.words];
        let full = host.resident(&sizes, &modules).unwrap();
        Some(Self {
            live,
            full,
            stages,
            whole: [Groups::Fixed(1), built.spread.dispatch()],
            plan,
            grid,
            reference,
            want: Vec::new(),
            frame: 0,
            label: String::new(),
        })
    }

    fn restart(&mut self, start: &[u32], label: String) {
        let sizes = live::sizes3(&Nature, &self.plan);
        for held in [&mut self.live, &mut self.full] {
            clear_all(held, &sizes);
            held.write(STATE as usize, start).unwrap();
            held.write(PARAMS as usize, &[u32::MAX, self.plan.dump()])
                .unwrap();
        }
        live::seed(&mut self.live, self.grid, &self.plan).unwrap();
        for held in [&mut self.live, &mut self.full] {
            held.write(PARAMS as usize, &[u32::MAX]).unwrap();
        }
        self.want = start.to_vec();
        self.frame = 0;
        self.label = label;
    }

    fn parted(&self, what: &str, got: &[u32]) -> Option<String> {
        let cells = self.reference.cells();
        let at = (0..cells).find(|&at| got[at] != self.want[at])?;
        let count = (0..cells).filter(|&at| got[at] != self.want[at]).count();
        Some(format!(
            "{}: frame {} cell {at}: {what} {:#x}, reference {:#x}; {count} cells differ",
            self.label, self.frame, got[at], self.want[at]
        ))
    }

    /// Steps the whole grid, the listed engine and the CPU one frame, holds
    /// the whole grid to the CPU and the listed world to the whole grid,
    /// and returns the octs the listed engine lists after it.
    fn frame(&mut self) -> u32 {
        let cells = self.reference.cells();
        let groups: Vec<&[Groups]> = self.stages.iter().map(Vec::as_slice).collect();
        let ticket = self
            .live
            .submit_reading_both(&groups, (STATE as usize, cells), (GRAND as usize, 1))
            .unwrap();
        let (_, listed, grand) = self.live.wait_reading_both(ticket).unwrap();
        let stages: Vec<&[Groups]> = self.whole.iter().map(core::slice::from_ref).collect();
        let (_, whole) = self
            .full
            .staged_reading(&stages, STATE as usize, cells)
            .unwrap();
        self.want = reference(&Nature, self.reference, self.frame, &self.want, &[]);
        if let Some(parted) = self.parted("whole grid", &whole) {
            panic!("{parted}");
        }
        self.want = whole;
        if let Some(parted) = self.parted("listed against the whole grid", &listed) {
            panic!("{parted}");
        }
        self.frame = self.frame.wrapping_add(1);
        grand[0]
    }

    /// Runs until the list stays empty for four frames, and returns the
    /// frame it emptied in.
    fn fall_quiet(&mut self) -> u32 {
        let until = self.frame + PATIENCE_FRAMES;
        while self.frame < until {
            if self.frame() == 0 {
                let quiet = self.frame;
                for _ in 0..4 {
                    assert_eq!(self.frame(), 0, "{}: the list filled again", self.label);
                }
                return quiet;
            }
        }
        panic!(
            "{}: the list never emptied in {PATIENCE_FRAMES} frames",
            self.label
        );
    }
}

/// A baked pine placed into a quiet nature world, at four turns one after
/// the other, wakes it, and the listed engine equals the whole grid and
/// the CPU every frame after each placement until it is quiet again.
#[test]
fn a_baked_asset_placed_into_a_quiet_nature_world_wakes_it_and_the_engines_agree() {
    let Some(host) = device("bake into nature") else {
        return;
    };
    let asset = pine();
    let [wide, deep, tall] = asset.size;
    assert!(deep < FLOOR, "the pine is {deep} cells tall");
    let kinds: Vec<u32> = [nature::WOOD, nature::LEAVES, nature::SAND]
        .into_iter()
        .filter(|&kind| {
            asset
                .cells
                .iter()
                .flatten()
                .any(|&word| nature::kind_of(word) == kind)
        })
        .collect();
    assert_eq!(kinds.len(), 3, "the pine lacks a part: {kinds:?}");
    for shape in WORLDS {
        for coarse in TIERS {
            let Some(mut engine) = Engine::new(&host, shape, coarse) else {
                continue;
            };
            let tier = if coarse { "coarse" } else { "fine" };
            let label = format!("bake {}x{}x{} {tier}", shape.0, shape.1, shape.2);
            engine.restart(&valley(engine.reference), label);
            let mut quiet = engine.fall_quiet();
            for turns in 0..4 {
                let at = [shape.0 / 2 - 3 + 2 * turns, FLOOR - 1, shape.2 / 2];
                let size = [shape.0, shape.1, shape.2];
                let cells = asset.placed(size, at, turns).unwrap();
                let written = bake::place(
                    &mut engine.live,
                    engine.grid,
                    &engine.plan,
                    &asset,
                    at,
                    turns,
                )
                .unwrap();
                assert_eq!(written, cells);
                for &(cell, word) in &cells {
                    engine
                        .full
                        .write_at(STATE as usize, cell as usize, &[word])
                        .unwrap();
                    engine.want[cell as usize] = word;
                }
                let listed_now = engine
                    .live
                    .read(GRAND as usize, 1)
                    .unwrap()
                    .first()
                    .copied()
                    .unwrap_or(0);
                assert!(
                    listed_now > 0,
                    "{}: the placement listed nothing",
                    engine.label
                );
                let woken = engine.frame;
                let listed: Vec<u32> = (0..FRAMES).map(|_| engine.frame()).collect();
                assert!(
                    listed.iter().any(|&count| count > 0),
                    "{}: the placement woke nothing",
                    engine.label
                );
                let again = engine.fall_quiet();
                println!(
                    "{} {} width {}: pine {wide}x{deep}x{tall} turned {turns} at {at:?}, {} cells, quiet at frame {quiet}, placed at {woken}, most listed {}, quiet again at {again}",
                    engine.label,
                    host.name(),
                    host.width(),
                    cells.len(),
                    listed.iter().max().unwrap_or(&0)
                );
                quiet = again;
            }
        }
    }
}
