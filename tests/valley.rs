#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

#[path = "../src/bin/grit3d/font.rs"]
#[allow(dead_code)]
mod font;
#[path = "../src/bin/grit3d/life.rs"]
#[allow(dead_code)]
mod life;
#[path = "../src/bin/grit3d/scenes.rs"]
#[allow(dead_code)]
mod scenes;
#[path = "../src/bin/grit3d/session.rs"]
#[allow(dead_code)]
mod session;
#[path = "../src/bin/grit3d/ui.rs"]
#[allow(dead_code)]
mod ui;
#[path = "../src/bin/grit3d/valley.rs"]
#[allow(dead_code)]
mod valley;

use grit::render3::Screen;
use grit::rule3::nature::{
    self, AIR, ASH, EARTH, FIRE, GRANULAR, KINDS, LEAVES, Nature, ROCK, SAND, SMOKE, SNOW, STEAM,
    WATER, WOOD,
};
use grit::sand::Grid;
use grit::sand3::{self, Grid3};

use life::{Act, Cycle, Phase};
use scenes::Which;
use session::{COUNT_EVERY, Session, Setup};
use valley::{AMBIENT, FEED_EVERY, SEED};

/// The side of the valley the engine tests run on.
const SIDE: u32 = 64;

/// The frames the engine runs past the warm-up in the equality test.
const FRAMES: u32 = 200;

/// The engine step at which the fire of the first round is lit on the
/// valley of [`SIDE`]: its warm-up and then its calm.
const LIT: u32 = 160 + 64;

/// The engine steps within which that fire is out and its smoke gone.
const OUT: std::ops::RangeInclusive<u32> = (LIT + 160)..=(LIT + 400);

fn grid(side: u32) -> Grid {
    Grid::solid(side, side, side).unwrap()
}

fn hash(cells: &[u32]) -> u64 {
    cells.iter().fold(0xCBF2_9CE4_8422_2325_u64, |hash, &word| {
        (hash ^ u64::from(word)).wrapping_mul(0x0100_0000_01B3)
    })
}

fn at(side: u32, x: u32, y: u32, z: u32) -> usize {
    (x + side * (y + side * z)) as usize
}

#[test]
fn the_same_seed_makes_the_same_valley_and_another_seed_another() {
    for side in [32, 64, 128] {
        let one = valley::made(grid(side), SEED).cells;
        let two = valley::made(grid(side), SEED).cells;
        let other = valley::made(grid(side), SEED ^ 1).cells;
        assert_eq!(one.len(), (side * side * side) as usize);
        assert_eq!(hash(&one), hash(&two), "{side}^3: one seed, two worlds");
        assert_ne!(hash(&one), hash(&other), "{side}^3: two seeds, one world");
        println!(
            "{side}^3: seed {SEED:#x} gives world hash {:#018x} twice; seed {:#x} gives {:#018x}",
            hash(&one),
            SEED ^ 1,
            hash(&other)
        );
    }
}

#[test]
fn the_valley_is_a_world_the_nature_rule_can_run() {
    for side in [32, 64, 128, 256] {
        let made = valley::made(grid(side), SEED);
        let cells = &made.cells;
        let mut counts = [0_usize; 16];
        for z in 0..side {
            for x in 0..side {
                let column = (z * side + x) as usize;
                let ground = made.ground[column];
                let wet = made.wet[column];
                let mut under_air = 0;
                for h in 0..side {
                    let y = side - 1 - h;
                    let word = cells[at(side, x, y, z)];
                    let kind = nature::kind_of(word);
                    counts[kind as usize] += 1;
                    assert!(kind < KINDS, "{side}^3 ({x},{y},{z}): inert kind {kind}");
                    assert!(
                        ![FIRE, ASH, SMOKE, STEAM].contains(&kind),
                        "{side}^3 ({x},{y},{z}): kind {kind} in a world at rest"
                    );
                    assert_eq!(
                        nature::temperature_of(word),
                        AMBIENT,
                        "{side}^3 ({x},{y},{z}): not at the valley's temperature"
                    );
                    if h < ground {
                        assert!(
                            [ROCK, EARTH, SAND, SNOW].contains(&kind),
                            "{side}^3 ({x},{y},{z}): kind {kind} inside the ground"
                        );
                    }
                    if kind == WATER {
                        assert!(
                            h >= ground && h < wet,
                            "{side}^3 ({x},{y},{z}): water at {h} outside its column's water, {ground} to {wet}"
                        );
                    }
                    if h >= ground && h < wet {
                        assert_eq!(
                            kind, WATER,
                            "{side}^3 ({x},{y},{z}): no water under the waterline"
                        );
                    }
                    if GRANULAR.contains(&kind) && h > 0 {
                        let below = nature::kind_of(cells[at(side, x, y + 1, z)]);
                        assert_ne!(below, AIR, "{side}^3 ({x},{y},{z}): a grain over air");
                    }
                    if [WOOD, LEAVES].contains(&kind) {
                        assert!(h >= ground, "{side}^3 ({x},{y},{z}): fuel in the ground");
                    }
                    if kind == AIR {
                        assert_eq!(
                            nature::depth_of(word),
                            under_air.min(nature::DEEPEST),
                            "{side}^3 ({x},{y},{z}): the air counts the air under it wrong"
                        );
                        under_air += 1;
                    } else {
                        under_air = 0;
                    }
                }
                assert!(
                    wet == 0 || wet > ground,
                    "{side}^3 ({x},{z}): a waterline under the ground"
                );
            }
        }
        for kind in [ROCK, EARTH, SAND, SNOW, WATER, WOOD, LEAVES] {
            assert!(
                counts[kind as usize] > 0,
                "{side}^3: no cell of kind {kind}"
            );
        }
        println!(
            "{side}^3: rock {}, earth {}, sand {}, snow {}, water {}, wood {}, leaves {}, air {}; lake level {}",
            counts[ROCK as usize],
            counts[EARTH as usize],
            counts[SAND as usize],
            counts[SNOW as usize],
            counts[WATER as usize],
            counts[WOOD as usize],
            counts[LEAVES as usize],
            counts[AIR as usize],
            made.level
        );
    }
}

/// The fire and smoke cells of `cells`, as the rule's tally counts them.
fn burning(cells: &[u32]) -> (u32, u32) {
    let fire = cells
        .iter()
        .filter(|&&word| nature::kind_of(word) == FIRE)
        .count();
    let smoke = cells
        .iter()
        .filter(|&&word| nature::kind_of(word) == SMOKE)
        .count();
    (fire as u32, smoke as u32)
}

/// The valley run on the CPU as the demo runs it on the device: the feed
/// written before every step whose number it is due at, the rule's step,
/// and the cycle told the fire and smoke when it counts them.
struct Cpu {
    grid3: Grid3,
    cells: Vec<u32>,
    feed: Vec<(u32, u32)>,
    life: Cycle,
    start: Vec<u32>,
    warm: u32,
    steps: u32,
}

impl Cpu {
    fn new(side: u32) -> Self {
        let example = Which::Valley.example();
        let grid = grid(side);
        let cells = (example.scene)(grid);
        let feed = (example.feed.unwrap())(grid, &cells);
        let life = (example.life.unwrap())(grid, &cells);
        let mut cpu = Self {
            grid3: Grid3::new(side, side, side).unwrap(),
            cells,
            feed,
            life,
            start: Vec::new(),
            warm: example.warm,
            steps: 0,
        };
        for _ in 0..example.warm {
            cpu.fed();
            cpu.cells = sand3::reference(&Nature, cpu.grid3, cpu.steps, &cpu.cells, &[]);
            cpu.steps += 1;
        }
        cpu.start = cpu.cells.clone();
        cpu
    }

    fn fed(&mut self) {
        if self.steps.is_multiple_of(FEED_EVERY) {
            for &(cell, word) in &self.feed {
                self.cells[cell as usize] = word;
            }
        }
    }

    fn step(&mut self) {
        self.fed();
        self.cells = sand3::reference(&Nature, self.grid3, self.steps, &self.cells, &[]);
        self.steps += 1;
        let counted = (self.life.phase == Phase::Burning && self.steps.is_multiple_of(COUNT_EVERY))
            .then(|| burning(&self.cells));
        match self.life.stepped(counted) {
            Act::Nothing => {}
            Act::Light(cells) => {
                for (cell, word) in cells {
                    self.cells[cell as usize] = word;
                }
            }
            Act::Restart => {
                self.cells = self.start.clone();
                self.steps = self.warm;
            }
        }
    }
}

fn setup(side: u32) -> Setup {
    Setup {
        which: Which::Valley,
        side,
        screen: Screen { wide: 16, high: 12 },
        suns: 1,
    }
}

#[test]
fn the_warmed_valley_and_two_hundred_frames_on_the_listed_engine_equal_the_cpu() {
    let Some(host) = device("valley frames") else {
        return;
    };
    let mut session = Session::open(&host, setup(SIDE)).unwrap();
    let mut cpu = Cpu::new(SIDE);
    assert_eq!(
        session.state().unwrap(),
        cpu.cells,
        "the warmed-up valley differs"
    );
    let mut lit = None;
    for frame in 0..FRAMES {
        session.advance(1).unwrap();
        cpu.step();
        if lit.is_none() && cpu.life.phase == Phase::Burning {
            lit = Some(cpu.steps);
        }
        if frame % 25 == 24 || frame + 1 == FRAMES {
            let got = session.state().unwrap();
            let differ = got.iter().zip(&cpu.cells).filter(|(a, b)| a != b).count();
            assert_eq!(differ, 0, "step {}: {differ} cells differ", cpu.steps);
        }
    }
    assert_eq!(session.steps, cpu.steps);
    assert_eq!(session.life.as_ref().unwrap(), &cpu.life);
    let (fire, smoke) = burning(&cpu.cells);
    println!(
        "{} on {SIDE}^3, width {}: the warm-up of {} frames and {FRAMES} frames more, with the feed and the fire lit at step {lit:?}, equal the CPU; {fire} fire and {smoke} smoke cells at the end",
        host.name(),
        host.width(),
        cpu.warm
    );
    assert_eq!(lit, Some(LIT), "the fire was not lit when its calm ended");
    assert!(fire + smoke > 0, "nothing burns at the end");
}

#[test]
fn the_fire_starts_by_itself_burns_out_and_the_valley_starts_again() {
    let Some(host) = device("valley fire") else {
        return;
    };
    let mut session = Session::open(&host, setup(SIDE)).unwrap();
    let start = session.state().unwrap();
    let fuel = |cells: &[u32]| {
        cells
            .iter()
            .filter(|&&word| [WOOD, LEAVES].contains(&nature::kind_of(word)))
            .count()
    };
    let before = fuel(&start);
    let mut lit = None;
    let mut out = None;
    let mut burned = 0;
    let mut most = (0, 0);
    let mut restarted = false;
    for _ in 0..OUT.end() + 200 {
        let phase = session.life.as_ref().unwrap().phase;
        session.advance(1).unwrap();
        let now = session.life.as_ref().unwrap().phase;
        if phase == Phase::Calm && now == Phase::Burning {
            lit = Some(session.steps);
        }
        if phase == Phase::Burning {
            let counted = session.counts().unwrap();
            most = (most.0.max(counted.0), most.1.max(counted.1));
        }
        if phase == Phase::Burning && now == Phase::After {
            out = Some(session.steps);
            burned = before - fuel(&session.state().unwrap());
        }
        if phase == Phase::After && now == Phase::Calm {
            restarted = true;
            assert_eq!(
                session.state().unwrap(),
                start,
                "the restart is not the start"
            );
            assert_eq!(session.steps, session.example.warm);
            break;
        }
    }
    println!(
        "{} width {}: lit at step {lit:?}, out at step {out:?} (window {OUT:?}); {burned} of {before} fuel cells burned; at most {} fire and {} smoke cells; restarted {restarted}",
        host.name(),
        host.width(),
        most.0,
        most.1
    );
    assert_eq!(lit, Some(LIT), "the fire was not lit when its calm ended");
    let out = out.expect("the fire never burned out");
    assert!(OUT.contains(&out), "out at step {out}, outside {OUT:?}");
    assert!(
        burned > before / 10,
        "only {burned} of {before} fuel cells burned"
    );
    assert!(most.1 > most.0, "the fire made less smoke than fire");
    assert!(restarted, "the valley did not start again");
}

/// A forest of touching crowns on an even slope that rises along x, lit
/// in its middle: how far the burned cells reach downhill and uphill after
/// each of a few spans of steps.
fn spread_on_a_slope(frames: u32) -> Vec<(u32, u32, u32)> {
    use grit::rule3::nature::{air, alive, cell};
    let side = 64_u32;
    let grid3 = Grid3::new(side, side, side).unwrap();
    let mut cells = vec![air(AMBIENT, nature::DEEPEST); (side * side * side) as usize];
    let ground = |x: u32| 8 + x / 2;
    for z in 0..side {
        for x in 0..side {
            for h in 0..ground(x) {
                cells[at(side, x, side - 1 - h, z)] = cell(ROCK, AMBIENT);
            }
        }
    }
    for z in (4..side - 4).step_by(3) {
        for x in (4..side - 4).step_by(3) {
            let foot = ground(x);
            for up in 0..8 {
                cells[at(side, x, side - 1 - foot - up, z)] = cell(WOOD, AMBIENT);
            }
            for up in 3..9 {
                for (dx, dz) in (0..9).map(|at| (at % 3, at / 3)) {
                    let slot = at(side, x + dx - 1, side - 1 - foot - up, z + dz - 1);
                    if nature::kind_of(cells[slot]) == AIR {
                        cells[slot] = cell(LEAVES, AMBIENT);
                    }
                }
            }
        }
    }
    let middle = side / 2 + 1;
    for up in 3..6 {
        cells[at(side, middle, side - 1 - ground(middle) - up, middle)] =
            alive(FIRE, life::KINDLING, nature::WOOD_LIFE);
    }
    let mut out = Vec::new();
    for frame in 0..frames {
        cells = sand3::reference(&Nature, grid3, frame, &cells, &[]);
        if frame % 8 == 7 {
            let (mut low, mut high) = (middle, middle);
            for z in middle - 4..=middle + 4 {
                for x in 0..side {
                    for y in 0..side - 2 - ground(x) {
                        if [FIRE, ASH].contains(&nature::kind_of(cells[at(side, x, y, z)])) {
                            low = low.min(x);
                            high = high.max(x);
                        }
                    }
                }
            }
            out.push((frame + 1, middle - low, high - middle));
        }
    }
    out
}

#[test]
fn a_fire_on_an_even_slope_spreads_downhill_and_uphill() {
    let spread = spread_on_a_slope(32);
    for &(frames, down, up) in &spread {
        println!("after {frames} steps the fire reaches {down} cells downhill and {up} uphill");
    }
    let &(_, down, up) = spread.last().unwrap();
    assert!(down > 8 && up > 8, "the fire did not spread both ways");
}
