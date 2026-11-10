#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use grit::compact::Plan;
use grit::layout::{BINDINGS, Dimension, GRAND, PARAMS, PIXELS, STATE, TALLY};
use grit::live;
use grit::rule3::heat::{self, Heat};
use grit::rule3::sand::Sand;
use grit::rule3::{CORNERS, FIRST_PARAM, Oct, Rule, Ruled, Word, noise, noise_emitted};
use grit::sand::{self as sand2, Grid};
use grit::sand3::{self, Grid3, lattice, reference};
use grit::sweep;
use grit::{Groups, Host, Resident};
use simdr::lanes::{LaneError, Lanes, U32};

const WORLDS: [(u32, u32, u32); 3] = [(16, 16, 16), (32, 32, 32), (64, 32, 16)];

const SEEDS: [u64; 4] = [1, 7, 0x5EED, 0xDEAD_BEEF];

const FRAMES: u32 = 64;

const TIERS: [bool; 2] = [false, true];

fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut mixed = *state;
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    mixed ^ (mixed >> 31)
}

/// A rule under test, with the worlds it is tried on.
struct Subject<R: Rule> {
    name: &'static str,
    rule: R,
    params: Vec<u32>,
    /// One random cell.
    cell: fn(&mut u64) -> u32,
    /// The cell that fills the world away from a seam.
    calm: u32,
    /// A world that falls quiet.
    room: fn(Grid3, u64) -> Vec<u32>,
    /// The host's write into a quiet room.
    poured: fn(Grid3) -> Vec<(u32, u32)>,
}

fn sand_cell(state: &mut u64) -> u32 {
    let roll = next(state);
    let kind = match roll % 20 {
        0..8 => sand2::AIR,
        8..12 => sand2::WATER,
        12..17 => sand2::SAND,
        _ => sand2::WALL,
    };
    if kind == sand2::WALL {
        return sand2::WALL;
    }
    let bits = ((roll >> 8) % 128) as u32;
    kind + (bits << 2)
}

/// A box with a wall floor and walls on the low x and z sides, which the
/// torus puts next to the high ones, and loose cells in its upper half.
fn sand_room(grid: Grid3, seed: u64) -> Vec<u32> {
    let mut state = seed;
    let mut cells = vec![sand2::AIR; grid.cells()];
    for z in 0..grid.tall {
        for y in 0..grid.deep {
            for x in 0..grid.wide {
                let wall = y + 1 == grid.deep || x == 0 || z == 0;
                let roll = next(&mut state);
                cells[grid.index(x, y, z)] = if wall {
                    sand2::WALL
                } else if y < grid.deep / 2 && roll.is_multiple_of(4) {
                    if roll.is_multiple_of(8) {
                        sand2::SAND
                    } else {
                        sand2::WATER
                    }
                } else {
                    sand2::AIR
                };
            }
        }
    }
    cells
}

fn sand_poured(grid: Grid3) -> Vec<(u32, u32)> {
    let mut cells = Vec::new();
    for (dx, dz, word) in [
        (0, 0, sand2::SAND),
        (1, 0, sand2::SAND),
        (0, 1, sand2::WATER),
        (1, 1, sand2::WATER),
        (2, 1, sand2::SAND),
    ] {
        let at = grid.index(grid.wide / 2 + dx, 1, grid.tall / 2 + dz);
        cells.push((u32::try_from(at).unwrap(), word));
    }
    cells
}

fn sand() -> Subject<Sand> {
    Subject {
        name: "sand",
        rule: Sand,
        params: Vec::new(),
        cell: sand_cell,
        calm: sand2::AIR,
        room: sand_room,
        poured: sand_poured,
    }
}

/// Every kind, the inert ones too, at temperatures from 0 to 1199 around
/// the thresholds, fires with any life, and random kept bits.
fn heat_cell(state: &mut u64) -> u32 {
    let roll = next(state);
    let kind = match roll % 32 {
        0..10 => heat::AIR,
        10..12 => heat::WALL,
        12..15 => heat::ICE,
        15..19 => heat::WATER,
        19..22 => heat::STEAM,
        22..26 => heat::FUEL,
        26..28 => heat::FIRE,
        28..31 => heat::ASH,
        _ => heat::KINDS + ((roll >> 40) % 8) as u32,
    };
    let temperature = ((roll >> 8) % 1200) as u32;
    let life = ((roll >> 20) % 16) as u32;
    let kept = ((roll >> 24) & 0xFF) as u32;
    heat::cell(kind, temperature) | (life << 4) | (kept << 8)
}

const ROOM_HEAT: u32 = 300;

/// The sand room's walls, insulating, with air, water, steam, ice, ash and
/// fuel below ignition in its upper half, all at one temperature and no
/// fire: the ice melts, the steam condenses, the water falls, and the room
/// falls quiet.
fn heat_room(grid: Grid3, seed: u64) -> Vec<u32> {
    let mut state = seed;
    let mut cells = vec![heat::cell(heat::AIR, ROOM_HEAT); grid.cells()];
    for z in 0..grid.tall {
        for y in 0..grid.deep {
            for x in 0..grid.wide {
                let wall = y + 1 == grid.deep || x == 0 || z == 0;
                let roll = next(&mut state);
                let kind = if wall {
                    heat::WALL
                } else if y < grid.deep / 2 && roll.is_multiple_of(4) {
                    [
                        heat::WATER,
                        heat::STEAM,
                        heat::ICE,
                        heat::ASH,
                        heat::FUEL,
                        heat::WATER,
                    ][((roll >> 8) % 6) as usize]
                } else {
                    heat::AIR
                };
                cells[grid.index(x, y, z)] = heat::cell(kind, ROOM_HEAT);
            }
        }
    }
    cells
}

/// A fire with fuel around it high in the room, a hot cell and some water.
fn heat_poured(grid: Grid3) -> Vec<(u32, u32)> {
    let mut cells = Vec::new();
    let lit = heat::cell(heat::FIRE, 800) | (heat::LIFE << 4);
    for (dx, dz, word) in [
        (0, 0, lit),
        (1, 0, heat::cell(heat::FUEL, ROOM_HEAT)),
        (0, 1, heat::cell(heat::FUEL, ROOM_HEAT)),
        (1, 1, heat::cell(heat::WATER, ROOM_HEAT)),
        (2, 1, heat::cell(heat::AIR, 9000)),
    ] {
        let at = grid.index(grid.wide / 2 + dx, 1, grid.tall / 2 + dz);
        cells.push((u32::try_from(at).unwrap(), word));
    }
    cells
}

fn heated() -> Subject<Heat> {
    Subject {
        name: "heat",
        rule: Heat,
        params: heat::TUNED.to_vec(),
        cell: heat_cell,
        calm: heat::cell(heat::AIR, ROOM_HEAT),
        room: heat_room,
        poured: heat_poured,
    }
}

fn random<R: Rule>(subject: &Subject<R>, grid: Grid3, seed: u64) -> Vec<u32> {
    let mut state = seed;
    (0..grid.cells())
        .map(|_| (subject.cell)(&mut state))
        .collect()
}

#[derive(Clone, Copy)]
enum Seam {
    X,
    Y,
    Z,
}

/// Random cells within two cells of one seam of `seam`'s axis, on both
/// sides, and the calm cell everywhere else.
fn on_seam<R: Rule>(subject: &Subject<R>, grid: Grid3, seam: Seam, seed: u64) -> Vec<u32> {
    let mut state = seed;
    let mut cells = vec![subject.calm; grid.cells()];
    for z in 0..grid.tall {
        for y in 0..grid.deep {
            for x in 0..grid.wide {
                let (at, extent) = match seam {
                    Seam::X => (x, grid.wide),
                    Seam::Y => (y, grid.deep),
                    Seam::Z => (z, grid.tall),
                };
                let roll = (subject.cell)(&mut state);
                if at < 2 || at + 2 >= extent {
                    cells[grid.index(x, y, z)] = roll;
                }
            }
        }
    }
    cells
}

fn grids(wide: u32, deep: u32, tall: u32) -> (Grid, Grid3) {
    let grid = Grid::solid(wide, deep, tall).unwrap();
    let reference = Grid3::new(wide, deep, tall).unwrap();
    (grid, reference)
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

/// The listed engine and the whole-grid kernel under one rule on the same
/// solid world, stepped side by side and held to the CPU reference.
struct Engine<'host, 'subject, R: Rule> {
    subject: &'subject Subject<R>,
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

impl<'host, 'subject, R: Rule> Engine<'host, 'subject, R> {
    fn new(
        host: &'host Host,
        subject: &'subject Subject<R>,
        (wide, deep, tall): (u32, u32, u32),
        coarse: bool,
    ) -> Option<Self> {
        let width = host.width();
        let (grid, reference) = grids(wide, deep, tall);
        let plan = plan_of(width, grid)?;
        let (passes, stages) =
            live::arranged3_by(&subject.rule, width, grid, &plan, coarse).unwrap();
        let built = sand3::settle_by(&subject.rule, width, grid, host.limits().groups).unwrap();
        let tick = live::tick(width).unwrap();
        let sizes = live::sizes3(&subject.rule, &plan);
        let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
        let live = host.resident(&sizes, &modules).unwrap();
        let modules: Vec<&[u32]> = vec![&tick, &built.words];
        let full = host.resident(&sizes, &modules).unwrap();
        Some(Self {
            subject,
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
        self.restart_at(start, label, 0);
    }

    /// Puts `start` and the rule's parameters in both worlds and seeds the
    /// listed engine, so the next frame is `frame` everywhere.
    fn restart_at(&mut self, start: &[u32], label: String, frame: u32) {
        let sizes = live::sizes3(&self.subject.rule, &self.plan);
        for held in [&mut self.live, &mut self.full] {
            clear_all(held, &sizes);
            held.write(STATE as usize, start).unwrap();
            if !self.subject.params.is_empty() {
                held.write_at(PARAMS as usize, FIRST_PARAM, &self.subject.params)
                    .unwrap();
            }
            held.write(PARAMS as usize, &[u32::MAX, self.plan.dump()])
                .unwrap();
        }
        live::seed(&mut self.live, self.grid, &self.plan).unwrap();
        for held in [&mut self.live, &mut self.full] {
            held.write(PARAMS as usize, &[frame.wrapping_sub(1)])
                .unwrap();
        }
        self.want = start.to_vec();
        self.frame = frame;
        self.label = label;
    }

    fn parted(&self, what: &str, got: &[u32]) -> Option<String> {
        let cells = self.reference.cells();
        let at = (0..cells).find(|&at| got[at] != self.want[at])?;
        let plane = (self.reference.wide * self.reference.deep) as usize;
        let (z, rest) = (at / plane, at % plane);
        let (y, x) = (
            rest / self.reference.wide as usize,
            rest % self.reference.wide as usize,
        );
        let count = (0..cells).filter(|&at| got[at] != self.want[at]).count();
        Some(format!(
            "{}: frame {} (phase {}) cell ({x}, {y}, {z}): {what} {:#x}, reference {:#x}; {count} cells differ",
            self.label,
            self.frame,
            self.frame & 1,
            got[at],
            self.want[at]
        ))
    }

    /// Steps the whole grid and the CPU one frame, and holds the whole grid
    /// to the CPU and the listed world `listed` to the whole grid.
    fn check(&mut self, listed: &[u32]) -> Option<String> {
        let cells = self.reference.cells();
        let stages: Vec<&[Groups]> = self.whole.iter().map(core::slice::from_ref).collect();
        let (_, whole) = self
            .full
            .staged_reading(&stages, STATE as usize, cells)
            .unwrap();
        self.want = reference(
            &self.subject.rule,
            self.reference,
            self.frame,
            &self.want,
            &self.subject.params,
        );
        let parted = self.parted("whole grid", &whole).or_else(|| {
            let held = std::mem::replace(&mut self.want, whole);
            let parted = self.parted("listed against the whole grid", listed);
            self.want = held;
            parted
        });
        self.frame = self.frame.wrapping_add(1);
        parted
    }

    /// Runs `flying` listed frames, all submitted before any is read, and
    /// checks each. Returns the octs listed after each.
    fn frames(&mut self, flying: usize) -> Vec<u32> {
        let cells = self.reference.cells();
        let groups: Vec<&[Groups]> = self.stages.iter().map(Vec::as_slice).collect();
        let mut tickets = Vec::with_capacity(flying);
        for _ in 0..flying {
            tickets.push(
                self.live
                    .submit_reading_both(&groups, (STATE as usize, cells), (GRAND as usize, 1))
                    .unwrap(),
            );
        }
        let mut listed = Vec::with_capacity(flying);
        for ticket in tickets {
            let (_, world, grand) = self.live.wait_reading_both(ticket).unwrap();
            if let Some(parted) = self.check(&world) {
                panic!("{parted}");
            }
            listed.push(grand[0]);
        }
        listed
    }

    fn frame(&mut self) -> u32 {
        self.frames(1)[0]
    }

    fn run(&mut self, frames: u32, flying: usize) -> Vec<u32> {
        let mut listed = Vec::new();
        let mut done = 0;
        while done < frames {
            listed.extend(self.frames(flying));
            done += u32::try_from(flying).unwrap();
        }
        listed
    }
}

fn label(name: &str, (wide, deep, tall): (u32, u32, u32), coarse: bool, seed: u64) -> String {
    let tier = if coarse { "coarse" } else { "fine" };
    format!("{name} {wide}x{deep}x{tall} {tier} seed {seed:#x}")
}

/// Every world of [`WORLDS`] from every seed, [`FRAMES`] frames, on both
/// tiers, `flying` frames in flight, from the worlds `start` draws.
fn matrix<R: Rule>(
    subject: &Subject<R>,
    test: &str,
    start: impl Fn(&Subject<R>, Grid3, u64) -> Vec<u32>,
    flying: usize,
) {
    let name = format!("{} {test}", subject.name);
    let Some(host) = device(&name) else {
        return;
    };
    for shape in WORLDS {
        for coarse in TIERS {
            let Some(mut engine) = Engine::new(&host, subject, shape, coarse) else {
                continue;
            };
            let mut fewest = u32::MAX;
            for seed in SEEDS {
                let first = start(subject, engine.reference, seed);
                engine.restart(&first, label(&name, shape, coarse, seed));
                let listed = engine.run(FRAMES, flying);
                assert_ne!(engine.want, first, "{}: nothing moved", engine.label);
                fewest = fewest.min(listed.iter().copied().min().unwrap_or(u32::MAX));
            }
            println!(
                "{name} {} width {} {}x{}x{} coarse {coarse}: {} seeds x {FRAMES} frames equal, fewest listed {fewest} of {}",
                host.name(),
                host.width(),
                shape.0,
                shape.1,
                shape.2,
                SEEDS.len(),
                engine.grid.blocks()
            );
        }
    }
}

fn seam<R: Rule>(subject: &Subject<R>, which: Seam, test: &str) {
    matrix(
        subject,
        test,
        |subject, grid, seed| on_seam(subject, grid, which, seed),
        1,
    );
}

const PATIENCE_FRAMES: u32 = 2048;

/// Runs until the list is empty, checking every frame, and returns the frame
/// it emptied in.
fn fall_quiet<R: Rule>(engine: &mut Engine<'_, '_, R>) -> u32 {
    let until = engine.frame + PATIENCE_FRAMES;
    while engine.frame < until {
        if engine.frame() == 0 {
            let quiet = engine.frame;
            for _ in 0..4 {
                assert_eq!(engine.frame(), 0, "{}: the list filled again", engine.label);
            }
            return quiet;
        }
    }
    panic!(
        "{}: the list never emptied in {PATIENCE_FRAMES} frames",
        engine.label
    );
}

const QUIET_WORLDS: [(u32, u32, u32); 2] = [(16, 16, 16), (32, 16, 16)];

/// The room falls quiet, the host writes into it through [`live::put3`]
/// (and into the whole grid and the CPU world), the two engines agree for
/// [`FRAMES`] frames with something listed, and the room falls quiet again.
fn quiet_then_woken<R: Rule>(subject: &Subject<R>, flying: usize) {
    let name = format!("{} quiet {flying} in flight", subject.name);
    let Some(host) = device(&name) else {
        return;
    };
    for shape in QUIET_WORLDS {
        for coarse in TIERS {
            let Some(mut engine) = Engine::new(&host, subject, shape, coarse) else {
                continue;
            };
            for seed in SEEDS {
                let start = (subject.room)(engine.reference, seed);
                engine.restart(&start, label(&name, shape, coarse, seed));
                let quiet = fall_quiet(&mut engine);
                let cells = (subject.poured)(engine.reference);
                for &(cell, word) in &cells {
                    engine
                        .full
                        .write_at(STATE as usize, cell as usize, &[word])
                        .unwrap();
                    engine.want[cell as usize] = word;
                }
                live::put3(&mut engine.live, engine.grid, &engine.plan, &cells).unwrap();
                let woken = engine.frame;
                let listed = engine.run(FRAMES, flying);
                assert!(
                    listed.iter().any(|&count| count > 0),
                    "{}: the write woke nothing",
                    engine.label
                );
                let again = fall_quiet(&mut engine);
                println!(
                    "{} {} width {}: quiet at frame {quiet}, written at {woken}, quiet again at {again}",
                    engine.label,
                    host.name(),
                    host.width()
                );
            }
        }
    }
}

#[test]
fn sand_listed_and_whole_equal_the_reference_on_random_worlds() {
    matrix(&sand(), "random", random, 1);
}

#[test]
fn heat_listed_and_whole_equal_the_reference_on_random_worlds() {
    matrix(&heated(), "random", random, 1);
}

#[test]
fn sand_listed_and_whole_equal_the_reference_across_the_x_seam() {
    seam(&sand(), Seam::X, "x seam");
}

#[test]
fn heat_listed_and_whole_equal_the_reference_across_the_x_seam() {
    seam(&heated(), Seam::X, "x seam");
}

#[test]
fn sand_listed_and_whole_equal_the_reference_across_the_y_seam() {
    seam(&sand(), Seam::Y, "y seam");
}

#[test]
fn heat_listed_and_whole_equal_the_reference_across_the_y_seam() {
    seam(&heated(), Seam::Y, "y seam");
}

#[test]
fn sand_listed_and_whole_equal_the_reference_across_the_z_seam() {
    seam(&sand(), Seam::Z, "z seam");
}

#[test]
fn heat_listed_and_whole_equal_the_reference_across_the_z_seam() {
    seam(&heated(), Seam::Z, "z seam");
}

#[test]
fn sand_with_two_frames_in_flight_equals_the_reference() {
    matrix(&sand(), "in flight", random, 2);
}

#[test]
fn heat_with_two_frames_in_flight_equals_the_reference() {
    matrix(&heated(), "in flight", random, 2);
}

#[test]
fn sand_falls_quiet_and_a_host_write_wakes_it() {
    quiet_then_woken(&sand(), 1);
}

#[test]
fn heat_falls_quiet_and_a_host_write_wakes_it() {
    quiet_then_woken(&heated(), 1);
}

#[test]
fn sand_falls_quiet_and_wakes_with_two_frames_in_flight() {
    quiet_then_woken(&sand(), 2);
}

#[test]
fn heat_falls_quiet_and_wakes_with_two_frames_in_flight() {
    quiet_then_woken(&heated(), 2);
}

/// A world at one temperature with no fire: the list drains to empty, and
/// every word is the CPU's.
#[test]
fn a_heat_world_at_one_temperature_without_fire_drains_the_list() {
    let subject = heated();
    let Some(host) = device("heat uniform") else {
        return;
    };
    for shape in [(16, 16, 16), (64, 32, 16)] {
        for coarse in TIERS {
            let Some(mut engine) = Engine::new(&host, &subject, shape, coarse) else {
                continue;
            };
            for (index, seed) in SEEDS.into_iter().enumerate() {
                let start = heat_room(engine.reference, seed);
                let first = u32::try_from(index).unwrap() * 3;
                engine.restart_at(&start, label("heat uniform", shape, coarse, seed), first);
                let quiet = fall_quiet(&mut engine);
                assert!(engine.want.iter().all(|&word| {
                    heat::kind_of(word) == heat::WALL || heat::temperature_of(word) == ROOM_HEAT
                }));
                assert_eq!(engine.live.read(GRAND as usize, 1).unwrap(), [0]);
                println!("{} quiet at frame {quiet}", engine.label);
            }
        }
    }
}

/// A world that is quiet from the start: nothing is listed after the seed's
/// two frames.
#[test]
fn a_still_heat_world_lists_nothing_after_the_seed() {
    let subject = heated();
    let Some(host) = device("heat still") else {
        return;
    };
    for coarse in TIERS {
        let Some(mut engine) = Engine::new(&host, &subject, (32, 32, 32), coarse) else {
            continue;
        };
        let start = vec![heat::cell(heat::ASH, 500); engine.reference.cells()];
        engine.restart(&start, label("heat still", (32, 32, 32), coarse, 0));
        let listed = engine.run(6, 1);
        assert_eq!(listed[1..], [0, 0, 0, 0, 0], "{listed:?}");
        assert_eq!(engine.want, start);
    }
}

#[test]
fn the_tally_holds_the_counts_of_the_heat_rule() {
    let subject = heated();
    let Some(host) = device("heat tally") else {
        return;
    };
    for coarse in TIERS {
        let Some(mut engine) = Engine::new(&host, &subject, (16, 16, 16), coarse) else {
            continue;
        };
        let start = random(&subject, engine.reference, SEEDS[0]);
        engine.restart(&start, label("heat tally", (16, 16, 16), coarse, SEEDS[0]));
        let before = engine.want.clone();
        engine.frame();
        let got = engine.live.read(TALLY as usize, 7).unwrap();
        let mut wanted = [0_u32; 7];
        for oct in 0..engine.reference.octs() {
            let places = engine.reference.corners(0, oct);
            let ruled = Heat.step(Oct {
                words: places.map(|at| before[at]),
                frame: 0,
                at: lattice(engine.reference, oct),
                params: &heat::TUNED,
            });
            for (word, count) in wanted.iter_mut().zip(ruled.counts) {
                *word += count;
            }
            for (word, value) in wanted[3..6].iter_mut().zip(lattice(engine.reference, oct)) {
                *word += (value >> grit::layout::COARSE) * ruled.counts[0];
            }
            wanted[6] += u32::from(ruled.counts[0] > 0);
        }
        assert_eq!(got, wanted, "{}", engine.label);
        assert!(wanted[1] > 0 && wanted[2] > 0);
    }
}

/// A probe rule: every cell becomes the engine's noise for its oct, mixed
/// with its corner and with the two parameters it reads. It changes every
/// oct every frame.
struct Noisy;

impl Rule for Noisy {
    fn params(&self) -> usize {
        2
    }

    fn step(&self, oct: Oct<'_, u32>) -> Ruled<u32> {
        let made = noise(oct.at, oct.frame);
        let out = std::array::from_fn(|corner| {
            let corner = u32::try_from(corner).unwrap();
            (made ^ corner.wrapping_mul(oct.params[0])).wrapping_add(oct.params[1])
        });
        Ruled {
            out,
            counts: [8, 0, 0],
        }
    }

    fn emit<const LANES: u32>(
        &self,
        lanes: &mut Lanes<'_>,
        oct: Oct<'_, Word<LANES>>,
    ) -> Result<Ruled<Word<LANES>>, LaneError> {
        let made = noise_emitted::<LANES>(lanes, oct.at, oct.frame)?;
        let mut out = [made; CORNERS];
        for (corner, word) in (0_u32..).zip(&mut out) {
            let corner = lanes.splat_bits::<U32, LANES>(corner)?;
            let scaled = lanes.mul(corner, oct.params[0])?;
            let mixed = lanes.xor(made, scaled)?;
            *word = lanes.add(mixed, oct.params[1])?;
        }
        let eight = lanes.splat_bits::<U32, LANES>(8)?;
        let none = lanes.splat_bits::<U32, LANES>(0)?;
        Ok(Ruled {
            out,
            counts: [eight, none, none],
        })
    }
}

fn noisy() -> Subject<Noisy> {
    Subject {
        name: "noise",
        rule: Noisy,
        params: vec![0x0101_0101, 0x2468_ACE0],
        cell: |state| next(state) as u32,
        calm: 0,
        room: |grid, _| vec![0; grid.cells()],
        poured: |_| Vec::new(),
    }
}

/// The noise the device computes for every oct of every frame equals the
/// CPU's, in the whole grid and in the listed frame, including where the
/// frame counter wraps: 4 seeds x 64 frames over every oct of each world.
#[test]
fn the_device_noise_equals_the_cpu_noise_for_every_oct_and_frame() {
    let subject = noisy();
    matrix(&subject, "random", random, 1);
    let Some(host) = device("noise wrap") else {
        return;
    };
    for coarse in TIERS {
        let shape = (64, 32, 16);
        let Some(mut engine) = Engine::new(&host, &subject, shape, coarse) else {
            continue;
        };
        let start = random(&subject, engine.reference, SEEDS[3]);
        let late = u32::MAX - 31;
        engine.restart_at(&start, label("noise wrap", shape, coarse, SEEDS[3]), late);
        engine.run(FRAMES, 1);
        assert_eq!(engine.frame, late.wrapping_add(FRAMES));
    }
}

/// The rule property the listed frame needs, held on the CPU: an oct that a
/// step leaves unchanged is left unchanged at every frame after, at the same
/// lattice point.
fn quiet_stays_quiet<R: Rule>(subject: &Subject<R>, octs: &mut dyn FnMut(&mut u64) -> [u32; 8]) {
    let mut state = 0x9E1_u64;
    let mut quiet = 0;
    for _ in 0..200_000 {
        let words = octs(&mut state);
        let at = [
            next(&mut state) as u32 % 64,
            next(&mut state) as u32 % 64,
            next(&mut state) as u32 % 64,
        ];
        let frame = next(&mut state) as u32;
        let step = |frame: u32| {
            subject
                .rule
                .step(Oct {
                    words,
                    frame,
                    at,
                    params: &subject.params,
                })
                .out
        };
        if step(frame) != words {
            continue;
        }
        quiet += 1;
        for later in 1..=8 {
            assert_eq!(
                step(frame.wrapping_add(later)),
                words,
                "{}: {words:x?} at {at:?} quiet at frame {frame}, not at {later} later",
                subject.name
            );
        }
    }
    assert!(
        quiet > 1000,
        "{}: only {quiet} quiet octs drawn",
        subject.name
    );
    println!("{}: {quiet} quiet octs stayed quiet", subject.name);
}

#[test]
fn a_quiet_oct_stays_quiet_at_every_later_frame_under_both_rules() {
    let sand = sand();
    quiet_stays_quiet(&sand, &mut |state| {
        let mut words: [u32; 8] = std::array::from_fn(|_| sand_cell(state));
        if next(state).is_multiple_of(2) {
            words[4..].fill(sand2::WALL);
        }
        words
    });
    let heated = heated();
    quiet_stays_quiet(&heated, &mut |state| {
        let temperature = (next(state) % 1200) as u32;
        let mut words: [u32; 8] = std::array::from_fn(|_| heat_cell(state));
        for word in &mut words {
            if heat::conducts(heat::kind_of(*word)) {
                *word = (*word & 0xFFFF) | (temperature << 16);
            }
        }
        words
    });
}

/// Each bit of the noise over every lattice point of a 64-cube and 4 frames,
/// 1 048 576 inputs, is set within 4 standard deviations (2048) of half
/// the time, and so is each bit of the change between neighbouring points
/// and frames.
#[test]
fn every_bit_of_the_noise_is_set_about_half_the_time() {
    let mut set = [0_u64; 32];
    let mut flipped = [[0_u64; 32]; 4];
    let mut inputs = 0_u64;
    for frame in [0, 1, 2, u32::MAX] {
        for z in 0..64 {
            for y in 0..64 {
                for x in 0..64 {
                    let made = noise([x, y, z], frame);
                    let others = [
                        noise([x + 1, y, z], frame),
                        noise([x, y + 1, z], frame),
                        noise([x, y, z + 1], frame),
                        noise([x, y, z], frame.wrapping_add(1)),
                    ];
                    for bit in 0..32 {
                        set[bit] += u64::from((made >> bit) & 1);
                        for (count, other) in flipped.iter_mut().zip(others) {
                            count[bit] += u64::from(((made ^ other) >> bit) & 1);
                        }
                    }
                    inputs += 1;
                }
            }
        }
    }
    assert_eq!(inputs, 1 << 20);
    let half = inputs / 2;
    let bound = 2048;
    for (bit, &count) in set.iter().enumerate() {
        assert!(
            count.abs_diff(half) <= bound,
            "bit {bit}: {count} of {inputs}"
        );
    }
    for (way, counts) in flipped.iter().enumerate() {
        for (bit, &count) in counts.iter().enumerate() {
            assert!(
                count.abs_diff(half) <= bound,
                "way {way} bit {bit}: {count} of {inputs}"
            );
        }
    }
    let worst = set.iter().map(|&count| count.abs_diff(half)).max().unwrap();
    println!("worst bit {worst} from {half} of {inputs}");
}

#[test]
fn the_kernels_read_the_rules_parameters_and_nothing_more() {
    for (wide, deep, tall) in [(16, 16, 16), (64, 32, 16)] {
        let (grid, _) = grids(wide, deep, tall);
        for width in [4, 8, 16, 32, 64] {
            for (rule_params, words) in [
                (
                    0,
                    sand3::settle_by(&Sand, width, grid, u32::MAX)
                        .unwrap()
                        .words,
                ),
                (
                    3,
                    sand3::settle_by(&Heat, width, grid, u32::MAX)
                        .unwrap()
                        .words,
                ),
                (2, sand3::settle_by(&Noisy, width, grid, 1).unwrap().words),
            ] {
                assert_eq!(params_read(&words), 1 + rule_params);
            }
            assert_eq!(
                sand3::settle(width, grid, u32::MAX).unwrap(),
                sand3::settle_by(&Sand, width, grid, u32::MAX).unwrap()
            );
            assert_eq!(
                live::settle3(width, grid).unwrap(),
                live::settle3_by(&Sand, width, grid).unwrap()
            );
            assert_eq!(
                params_read(&live::settle3_by(&Heat, width, grid).unwrap()),
                5
            );
        }
    }
    let plan = Plan::over_in(16, 2, Dimension::Three).unwrap();
    assert_eq!(live::sizes3(&Sand, &plan), plan.sizes());
    let mut grown = plan.sizes();
    grown[PARAMS as usize] = FIRST_PARAM + 3;
    assert_eq!(live::sizes3(&Heat, &plan), grown);
    assert_eq!(FIRST_PARAM, Dimension::Three.params_words());
}

/// The loads of `PARAMS` in `words`, counted in the SPIR-V as written.
fn params_read(words: &[u32]) -> usize {
    let mut instructions = Vec::new();
    let mut at = 5;
    while at < words.len() {
        let count = (words[at] >> 16) as usize;
        assert!(count > 0);
        instructions.push((words[at] & 0xFFFF, &words[at + 1..at + count]));
        at += count;
    }
    let mut bound = std::collections::HashMap::new();
    for (code, operands) in &instructions {
        if *code == 71 && operands[1] == 33 {
            bound.insert(operands[0], operands[2]);
        }
    }
    let mut chains = std::collections::HashSet::new();
    for (code, operands) in &instructions {
        if *code == 65 && bound.get(&operands[2]) == Some(&PARAMS) {
            chains.insert(operands[1]);
        }
    }
    instructions
        .iter()
        .filter(|(code, operands)| *code == 61 && chains.contains(&operands[2]))
        .count()
}

/// Worlds with fewer octs than a subgroup, or one oct along an axis, where
/// the 27 octs a change wakes are partly or wholly the same oct.
const SMALL: [(u32, u32, u32); 6] = [
    (2, 2, 2),
    (4, 4, 4),
    (2, 8, 4),
    (8, 8, 2),
    (32, 2, 8),
    (2, 2, 64),
];

#[test]
fn heat_listed_and_whole_equal_the_reference_on_small_and_thin_worlds() {
    let subject = heated();
    let Some(host) = device("heat small") else {
        return;
    };
    for shape in SMALL {
        for coarse in TIERS {
            let Some(mut engine) = Engine::new(&host, &subject, shape, coarse) else {
                continue;
            };
            for seed in SEEDS {
                let first = random(&subject, engine.reference, seed);
                engine.restart(&first, label("heat small", shape, coarse, seed));
                engine.run(FRAMES, 1);
                let room = heat_room(engine.reference, seed);
                engine.restart(&room, label("heat small room", shape, coarse, seed));
                engine.run(FRAMES, 1);
            }
        }
    }
}

/// Words with every bit at random, so temperatures reach the top and kinds
/// past the known ones turn up, under parameters at their edges: fuel that
/// is always past ignition and always or never lucky, a flare past the top,
/// and the frame counter wrapping.
#[test]
fn heat_listed_and_whole_equal_the_reference_on_wild_words_and_edge_parameters() {
    let Some(host) = device("heat wild") else {
        return;
    };
    for params in [
        [0, 16, u32::MAX],
        [0, 0, 0],
        [u32::MAX, 0, 0],
        [heat::TOP, 8, 1],
        [1, 1, heat::TOP],
    ] {
        let subject = Subject {
            name: "heat wild",
            params: params.to_vec(),
            cell: |state| next(state) as u32,
            ..heated()
        };
        for shape in [(16, 16, 16), (64, 32, 16)] {
            for coarse in TIERS {
                let Some(mut engine) = Engine::new(&host, &subject, shape, coarse) else {
                    continue;
                };
                for (seed, first) in [(SEEDS[0], 0), (SEEDS[1], u32::MAX - 31)] {
                    let start = random(&subject, engine.reference, seed);
                    let named = format!("heat wild {params:?}");
                    engine.restart_at(&start, label(&named, shape, coarse, seed), first);
                    engine.run(FRAMES, 1);
                }
            }
        }
    }
}
