use grit::render3::{Light, Material, Table};
use grit::rule3::nature::{
    AIR, ASH, CHAR, EARTH, FIRE, ICE, LEAVES, ROCK, SAND, SMOKE, SNOW, STEAM, THAW, WATER, WOOD,
    WOOD_LIFE, air, alive, cell, open_air,
};
use grit::sand::Grid;

use super::life::{Cycle, KINDLING};
use super::scenes::{Example, Tool};

/// The seed of the valley the demo shows.
pub const SEED: u32 = 0x00C0_FFEE;

/// The temperature of the whole valley at rest: below
/// [`grit::rule3::nature::MELT`], so snow keeps, and the same in every
/// cell, so the valley at rest is quiet.
pub const AMBIENT: u32 = 260;

/// The sun the valley runs under: the thaw a sunlit snow cell takes in
/// each step it is lit, so it melts after
/// `grit::rule3::nature::THAW / SUN + 1` lit steps.
pub const SUN: u32 = 1;

/// A valley as the generator makes it: its words, and what a check needs
/// to know of it.
///
/// - `ground`: the top of the solid ground of each column, in cells above
///   the floor, column `x + wide * z`;
/// - `wet`: the height up to which water may lie in each column, as the
///   generator fills it, 0 where none may;
/// - `level`: the height of the lake's surface;
/// - `snowline`: the height above which snow lies, before its noise.
pub struct Made {
    pub cells: Vec<u32>,
    pub ground: Vec<u32>,
    pub wet: Vec<u32>,
    pub level: u32,
    pub snowline: u32,
}

pub fn mixed(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846C_A68B);
    x ^ (x >> 16)
}

fn hashed(x: u32, y: u32, z: u32, seed: u32) -> u32 {
    mixed(
        mixed(mixed(x ^ seed.rotate_left(7)) ^ y.wrapping_mul(0x9E37_79B9))
            ^ z.wrapping_mul(0x85EB_CA6B),
    )
}

fn unit(key: u32) -> f32 {
    (key & 0x00FF_FFFF) as f32 / 16_777_215.0
}

fn lattice(x: i32, z: i32, seed: u32) -> f32 {
    unit(hashed(x as u32, z as u32, 0x51, seed))
}

/// Smooth value noise in [0, 1] at `(x, z)`, in lattice cells.
fn value(x: f32, z: f32, seed: u32) -> f32 {
    let (i, j) = (x.floor(), z.floor());
    let (fu, fv) = (x - i, z - j);
    let (su, sv) = (
        fu * fu * fu * (fu * (fu * 6.0 - 15.0) + 10.0),
        fv * fv * fv * (fv * (fv * 6.0 - 15.0) + 10.0),
    );
    let (i, j) = (i as i32, j as i32);
    let a = lattice(i, j, seed);
    let b = lattice(i + 1, j, seed);
    let c = lattice(i, j + 1, seed);
    let d = lattice(i + 1, j + 1, seed);
    let low = a + (b - a) * su;
    let high = c + (d - c) * su;
    low + (high - low) * sv
}

/// Fractal noise in about [0, 1] at `(u, v)`, the share of the world, with
/// `octaves` layers from `scale` lattice cells across the world on.
fn fbm(u: f32, v: f32, scale: f32, octaves: u32, seed: u32) -> f32 {
    let mut sum = 0.0;
    let mut weight = 0.5;
    let mut total = 0.0;
    let mut size = scale;
    for octave in 0..octaves {
        sum += weight * value(u * size, v * size, seed.wrapping_add(octave * 0x3C6E_F372));
        total += weight;
        weight *= 0.5;
        size *= 2.03;
    }
    sum / total.max(1e-6)
}

/// Ridged fractal noise in about [0, 1]: sharp crests where plain noise
/// crosses its middle.
fn ridged(u: f32, v: f32, scale: f32, octaves: u32, seed: u32) -> f32 {
    let mut sum = 0.0;
    let mut weight = 0.5;
    let mut total = 0.0;
    let mut size = scale;
    let mut previous = 1.0;
    for octave in 0..octaves {
        let n = value(u * size, v * size, seed.wrapping_add(octave * 0x2545_F491));
        let crest = 1.0 - (2.0 * n - 1.0).abs();
        let crest = crest * crest * (0.4 + 0.6 * previous);
        previous = crest;
        sum += weight * crest;
        total += weight;
        weight *= 0.5;
        size *= 2.1;
    }
    sum / total.max(1e-6)
}

fn smooth(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn mix(a: f32, b: f32, by: f32) -> f32 {
    a + (b - a) * by
}

/// A square field of heights, one a column.
#[derive(Debug, Clone)]
struct Field {
    side: usize,
    data: Vec<f32>,
}

impl Field {
    fn new(side: usize) -> Self {
        Self {
            side,
            data: vec![0.0; side * side],
        }
    }

    fn slot(&self, x: i64, z: i64) -> usize {
        let last = self.side as i64 - 1;
        (z.clamp(0, last) * self.side as i64 + x.clamp(0, last)) as usize
    }

    fn at(&self, x: i64, z: i64) -> f32 {
        self.data.get(self.slot(x, z)).copied().unwrap_or(0.0)
    }

    fn set(&mut self, x: i64, z: i64, value: f32) {
        let slot = self.slot(x, z);
        if let Some(held) = self.data.get_mut(slot) {
            *held = value;
        }
    }

    fn add(&mut self, x: i64, z: i64, value: f32) {
        let slot = self.slot(x, z);
        if let Some(held) = self.data.get_mut(slot) {
            *held += value;
        }
    }

    /// The height between columns, and its slope along x and z.
    fn sampled(&self, x: f32, z: f32) -> (f32, f32, f32) {
        let (i, j) = (x.floor() as i64, z.floor() as i64);
        let (fx, fz) = (x - x.floor(), z - z.floor());
        let a = self.at(i, j);
        let b = self.at(i + 1, j);
        let c = self.at(i, j + 1);
        let d = self.at(i + 1, j + 1);
        let height = mix(mix(a, b, fx), mix(c, d, fx), fz);
        let along_x = mix(b - a, d - c, fz);
        let along_z = mix(c - a, d - b, fx);
        (height, along_x, along_z)
    }
}

/// A small random stream, the same for the same seed.
struct Stream(u32);

impl Stream {
    fn next(&mut self) -> f32 {
        self.0 = mixed(self.0.wrapping_add(0x9E37_79B9));
        unit(self.0)
    }
}

/// Droplets run down the field, take up what they can carry where they
/// speed up, and leave it where they slow down, so gullies cut into the
/// slopes and fans spread at their feet.
fn eroded(field: &mut Field, seed: u32) {
    let side = field.side as f32;
    let scale = side / 256.0;
    let drops = (field.side * field.side) / 3;
    let mut stream = Stream(seed ^ 0xD80B_5EED);
    let inertia = 0.08;
    let capacity = 3.0;
    let least = 0.02;
    let take = 0.25;
    let leave = 0.3;
    let dry = 0.025;
    let pull = 4.0;
    let longest = (56.0 * scale) as u32;
    for _ in 0..drops {
        let mut x = 2.0 + stream.next() * (side - 4.0);
        let mut z = 2.0 + stream.next() * (side - 4.0);
        let (mut dx, mut dz) = (0.0_f32, 0.0_f32);
        let mut speed = 1.0_f32;
        let mut water = 1.0_f32;
        let mut load = 0.0_f32;
        for _ in 0..longest {
            let (height, gx, gz) = field.sampled(x, z);
            dx = dx * inertia - gx * (1.0 - inertia);
            dz = dz * inertia - gz * (1.0 - inertia);
            let length = (dx * dx + dz * dz).sqrt();
            if length < 1e-5 {
                let turn = stream.next() * std::f32::consts::TAU;
                dx = turn.cos();
                dz = turn.sin();
            } else {
                dx /= length;
                dz /= length;
            }
            let (nx, nz) = (x + dx, z + dz);
            if nx < 1.0 || nz < 1.0 || nx > side - 2.0 || nz > side - 2.0 {
                break;
            }
            let (next, _, _) = field.sampled(nx, nz);
            let drop = next - height;
            let carry = (-drop).max(least) * speed * water * capacity;
            let (i, j) = (x.floor() as i64, z.floor() as i64);
            let (fx, fz) = (x - x.floor(), z - z.floor());
            if load > carry || drop > 0.0 {
                let laid = if drop > 0.0 {
                    drop.min(load)
                } else {
                    (load - carry) * leave
                };
                load -= laid;
                field.add(i, j, laid * (1.0 - fx) * (1.0 - fz));
                field.add(i + 1, j, laid * fx * (1.0 - fz));
                field.add(i, j + 1, laid * (1.0 - fx) * fz);
                field.add(i + 1, j + 1, laid * fx * fz);
            } else {
                let cut = ((carry - load) * take).min(-drop).min(1.5);
                let mut weights = [0.0_f32; 9];
                let mut total = 0.0;
                for (slot, weight) in weights.iter_mut().enumerate() {
                    let (ox, oz) = ((slot % 3) as f32 - 1.0, (slot / 3) as f32 - 1.0);
                    let far = ((ox - fx + 0.5).powi(2) + (oz - fz + 0.5).powi(2)).sqrt();
                    *weight = (1.6 - far).max(0.0);
                    total += *weight;
                }
                for (slot, weight) in weights.iter().enumerate() {
                    let (ox, oz) = ((slot % 3) as i64 - 1, (slot / 3) as i64 - 1);
                    field.add(i + ox, j + oz, -cut * weight / total.max(1e-6));
                }
                load += cut;
            }
            speed = (speed * speed - drop * pull).max(0.0).sqrt();
            water *= 1.0 - dry;
            x = nx;
            z = nz;
        }
    }
}

/// Material slides from each column to a lower neighbour wherever the
/// slope between them is steeper than `talus` allows there, so screes
/// form under steep ground.
fn slumped(field: &mut Field, passes: u32, talus: impl Fn(f32) -> f32) {
    let side = field.side as i64;
    const AROUND: [(i64, i64, f32); 8] = [
        (1, 0, 1.0),
        (-1, 0, 1.0),
        (0, 1, 1.0),
        (0, -1, 1.0),
        (1, 1, std::f32::consts::SQRT_2),
        (-1, 1, std::f32::consts::SQRT_2),
        (1, -1, std::f32::consts::SQRT_2),
        (-1, -1, std::f32::consts::SQRT_2),
    ];
    for _ in 0..passes {
        let mut moved = Field::new(field.side);
        for z in 0..side {
            for x in 0..side {
                let here = field.at(x, z);
                let limit = talus(here);
                let mut steepest = 0.0_f32;
                let mut total = 0.0_f32;
                for &(ox, oz, far) in &AROUND {
                    let excess = here - field.at(x + ox, z + oz) - limit * far;
                    if excess > 0.0 {
                        total += excess;
                        steepest = steepest.max(excess);
                    }
                }
                if total <= 0.0 {
                    continue;
                }
                let shed = steepest * 0.4;
                moved.add(x, z, -shed);
                for &(ox, oz, far) in &AROUND {
                    let excess = here - field.at(x + ox, z + oz) - limit * far;
                    if excess > 0.0 {
                        moved.add(x + ox, z + oz, shed * excess / total);
                    }
                }
            }
        }
        for (held, change) in field.data.iter_mut().zip(&moved.data) {
            *held += change;
        }
    }
}

/// A point in the world's plan, as shares of a side.
type Plan = [f32; 2];

fn apart(a: Plan, b: Plan) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// Where a point lies against a line through a path: how far from it, how
/// far along the whole path, and which stretch it is nearest.
fn nearest(path: &[(Plan, f32)], at: Plan) -> (f32, f32, usize) {
    let mut best = (f32::MAX, 0.0, 0);
    for (stretch, pair) in path.windows(2).enumerate() {
        let (Some(&(a, ha)), Some(&(b, hb))) = (pair.first(), pair.get(1)) else {
            continue;
        };
        let along = [b[0] - a[0], b[1] - a[1]];
        let length = (along[0] * along[0] + along[1] * along[1]).max(1e-9);
        let t = ((at[0] - a[0]) * along[0] + (at[1] - a[1]) * along[1]) / length;
        if stretch == FALL && t <= 0.0 {
            continue;
        }
        let t = t.clamp(0.0, 1.0);
        let foot = [a[0] + along[0] * t, a[1] + along[1] * t];
        let far = apart(foot, at);
        if far < best.0 {
            let bed = if stretch == FALL { hb } else { mix(ha, hb, t) };
            best = (far, bed, stretch);
        }
    }
    best
}

/// The plan of the valley, in shares of a side: the peak, the snowmelt
/// basin, the lip of the cliff, the pool under it, the river's bends, the
/// lake, and the forest where the fire starts.
const PEAK: Plan = [0.25, 0.23];
const SHOULDER: Plan = [0.12, 0.40];
const BASIN: Plan = [0.30, 0.39];
pub const LIP: Plan = [0.335, 0.47];
pub const POOL: Plan = [0.36, 0.525];
const LAKE: Plan = [0.87, 0.64];
const BURN: Plan = [0.68, 0.46];
const SPRING: Plan = [0.283, 0.372];

/// Heights, as shares of a side.
const FLOOR: f32 = 0.11;
const LEVEL: f32 = 0.165;
const SNOWLINE: f32 = 0.43;
/// The depth of snow every snow column holds over what its hollow and
/// slope give it, as a share of the world's height, so the sun melts the
/// snowfield down over many cycles of its surface.
const PACK: f32 = 0.02;
const LIP_HEIGHT: f32 = 0.41;
const POOL_HEIGHT: f32 = 0.265;

/// The stretch of the river that falls off the cliff: its bed is the pool's
/// all along, so the lip above it is a wall.
const FALL: usize = 3;

/// The river from the basin to the lake: each point and its bed's height.
const RIVER: [(Plan, f32); 10] = [
    (SPRING, 0.428),
    (BASIN, 0.421),
    ([0.32, 0.435], 0.414),
    (LIP, LIP_HEIGHT),
    (POOL, POOL_HEIGHT),
    ([0.42, 0.585], 0.25),
    ([0.49, 0.6], 0.235),
    ([0.56, 0.655], 0.215),
    ([0.62, 0.69], 0.192),
    ([0.72, 0.69], LEVEL - 0.01),
];

/// Whether `at` lies in the rocky slope of scree and sand left of the
/// river's lower reach.
fn scree(at: Plan, seed: u32) -> bool {
    apart(at, [0.4, 0.84]) + 0.22 * (fbm(at[0], at[1], 5.0, 4, seed ^ 0x5C) - 0.5) < 0.16
}

fn lake_share(at: Plan, seed: u32) -> f32 {
    let wobble = 0.16 * (fbm(at[0], at[1], 4.0, 3, seed ^ 0x1A4E) - 0.5);
    let du = (at[0] - LAKE[0]) / 0.21;
    let dv = (at[1] - LAKE[1]) / 0.19;
    1.0 - (du * du + dv * dv).sqrt() + wobble
}

/// The terrain before erosion: hills that fall toward the lake, a forested
/// slope on the right, a low rocky shore on the left, and the mountain with
/// a jagged ridge and its shoulder.
fn raw_height(at: Plan, seed: u32) -> f32 {
    let warp = [
        at[0] + 0.07 * (fbm(at[0], at[1], 3.0, 3, seed ^ 0x11) - 0.5),
        at[1] + 0.07 * (fbm(at[0], at[1], 3.0, 3, seed ^ 0x22) - 0.5),
    ];
    let front = smooth(0.5, 1.7, warp[0] + warp[1]);
    let hills = fbm(warp[0], warp[1], 4.0, 5, seed ^ 0x33);
    let low = 0.24 - 0.05 * front + 0.06 * (hills - 0.5);
    let right = (1.0 - apart(warp, [0.86, 0.12]) / 0.5).max(0.0);
    let back_left = (1.0 - apart(warp, [0.05, 0.62]) / 0.3).max(0.0);
    let shore = (1.0 - apart(warp, [0.45, 0.95]) / 0.32).max(0.0);
    let slopes = 0.15 * smooth(0.0, 1.0, right) + 0.07 * smooth(0.0, 1.0, back_left)
        - 0.05 * smooth(0.0, 1.0, shore);
    let crag = ridged(warp[0], warp[1], 6.0, 6, seed ^ 0x44);
    let cone = (1.0 - apart(warp, PEAK) / 0.42).max(0.0);
    let mountain = 0.44 * cone.powf(1.25) + 0.2 * cone.powf(0.8) * (crag - 0.45);
    let shoulder_cone = (1.0 - apart(warp, SHOULDER) / 0.2).max(0.0);
    let shoulder = 0.2 * shoulder_cone.powf(1.3) + 0.07 * shoulder_cone * (crag - 0.45);
    let ridge_line = (1.0 - apart(warp, [0.52, 0.1]) / 0.24).max(0.0);
    let ridge = 0.14 * ridge_line.powf(1.4) * (0.5 + crag);
    let peaks = mountain.max(shoulder).max(ridge);
    low + slopes + peaks
}

/// Carves the cirque, the gorge, the cliff, the river and the lake into
/// `field`, heights as shares of a side.
fn carved(field: &mut Field, seed: u32) {
    let side = field.side;
    let s = side as f32;
    let cell = 1.0 / s;
    let toward = {
        let [a, b] = [POOL, LIP];
        let length = apart(a, b).max(1e-6);
        [(a[0] - b[0]) / length, (a[1] - b[1]) / length]
    };
    for z in 0..side {
        for x in 0..side {
            let at = [(x as f32 + 0.5) / s, (z as f32 + 0.5) / s];
            let mut h = field.at(x as i64, z as i64);
            let rough = fbm(at[0], at[1], 24.0, 3, seed ^ 0x55) - 0.5;
            let along = (at[0] - LIP[0]) * toward[0] + (at[1] - LIP[1]) * toward[1];
            let across = (-(at[0] - LIP[0]) * toward[1] + (at[1] - LIP[1]) * toward[0]).abs();
            let shelf = 1.0 - smooth(0.12, 0.2, across);
            if along < 0.0 && along > -0.22 {
                let top = LIP_HEIGHT + 0.05 + 0.02 * rough - along * 0.3;
                h = mix(h, h.max(top), shelf * smooth(-0.22, -0.14, along));
            }
            if (0.0..0.16).contains(&along) {
                let floor = POOL_HEIGHT + 0.006 + 0.06 * along;
                let half = 0.03 + 0.3 * along + 0.006 * rough;
                let wall = floor + (across - half).max(0.0) * 3.2 + 0.01 * rough;
                let fade = 1.0 - smooth(0.1, 0.16, along);
                h = mix(h, h.min(wall), fade);
            }
            let pool = apart(at, POOL);
            if pool < 0.05 {
                h = h.min(POOL_HEIGHT - 0.012 + 0.6 * pool * pool / 0.05);
            }
            let (far, bed, stretch) = nearest(&RIVER, at);
            let (wide, steep) = if stretch < 1 {
                (0.02, 1.4)
            } else if stretch < 3 {
                (0.011, 2.4)
            } else if stretch < 5 {
                (0.014, 3.0)
            } else {
                (0.013, 0.9)
            };
            let bank = bed + (far - wide).max(0.0) * steep + 0.002 * rough;
            h = h.min(bank);
            let lake = lake_share(at, seed);
            if lake > -0.35 {
                let shore = LEVEL + 0.012 - 0.12 * lake.min(0.0);
                let bottom = LEVEL + 0.004 - 0.13 * smooth(0.0, 0.75, lake);
                let bed = if lake > 0.0 { bottom } else { shore };
                h = h.min(bed + 0.004 * rough);
            }
            let edge = x == 0 || z == 0 || x == side - 1 || z == side - 1;
            if edge && lake < -0.02 {
                h = h.max(LEVEL + 4.0 * cell);
            }
            field.set(x as i64, z as i64, h.max(FLOOR + 2.0 * cell));
        }
    }
}

/// What a column is made of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Skin {
    Rock,
    Earth,
    Sand,
    Snow,
    Bed,
}

struct Columns {
    side: usize,
    ground: Vec<u32>,
    skin: Vec<Skin>,
    deep: Vec<u32>,
    wet: Vec<u32>,
}

impl Columns {
    fn slot(&self, x: i64, z: i64) -> Option<usize> {
        let side = self.side as i64;
        (x >= 0 && z >= 0 && x < side && z < side).then(|| (z * side + x) as usize)
    }

    fn ground_at(&self, x: i64, z: i64) -> u32 {
        let side = self.side as i64;
        let slot = (z.clamp(0, side - 1) * side + x.clamp(0, side - 1)) as usize;
        self.ground.get(slot).copied().unwrap_or(0)
    }

    /// The deepest drop from column `(x, z)` to one of the eight around it.
    fn drop(&self, x: i64, z: i64) -> u32 {
        let here = self.ground_at(x, z);
        let mut most = 0;
        for oz in -1..=1 {
            for ox in -1..=1 {
                most = most.max(here.saturating_sub(self.ground_at(x + ox, z + oz)));
            }
        }
        most
    }
}

/// The world of `grid` made from `seed`: heights, erosion, carving, the
/// skin of each column, water, snow, trees and boulders, every cell at
/// [`AMBIENT`], and the air over the highest other cell of each column
/// open to the sky. Each snow cell starts with a thaw of its own, from a
/// hash, so the sun melts the snowfield cell by cell, not all at once.
#[allow(clippy::too_many_lines)]
pub fn made(grid: Grid, seed: u32) -> Made {
    let side = grid.wide.min(grid.tall).max(4) as usize;
    let s = side as f32;
    let tall = grid.deep as f32;
    let mut field = Field::new(side);
    for z in 0..side {
        for x in 0..side {
            let at = [(x as f32 + 0.5) / s, (z as f32 + 0.5) / s];
            field.set(x as i64, z as i64, raw_height(at, seed) * tall);
        }
    }
    eroded(&mut field, seed);
    let scale = (s / 256.0).max(0.25);
    let snow_cells = SNOWLINE * tall;
    slumped(&mut field, (10.0 * scale).ceil() as u32, |h| {
        mix(1.1, 2.6, smooth(snow_cells * 0.8, snow_cells * 1.3, h))
    });
    for held in &mut field.data {
        *held /= tall;
    }
    carved(&mut field, seed);
    let level = (LEVEL * tall).round() as u32;
    let pack = (PACK * tall) as u32;
    let snowline = snow_cells.round() as u32;
    let top = grid.deep.saturating_sub(4);

    let mut columns = Columns {
        side,
        ground: vec![0; side * side],
        skin: vec![Skin::Rock; side * side],
        deep: vec![0; side * side],
        wet: vec![0; side * side],
    };
    for z in 0..side {
        for x in 0..side {
            let h = (field.at(x as i64, z as i64) * tall)
                .round()
                .clamp(2.0, top as f32) as u32;
            if let Some(slot) = columns.ground.get_mut(z * side + x) {
                *slot = h;
            }
        }
    }

    let river_wet = |at: Plan| {
        let (far, bed, stretch) = nearest(&RIVER, at);
        let wide = if stretch < 1 {
            0.02
        } else if stretch < 3 {
            0.011
        } else {
            0.014
        };
        (far < wide).then_some(bed)
    };

    for z in 0..side {
        for x in 0..side {
            let at = [(x as f32 + 0.5) / s, (z as f32 + 0.5) / s];
            let (xi, zi) = (x as i64, z as i64);
            let h = columns.ground_at(xi, zi);
            let fall = columns.drop(xi, zi);
            let lake = lake_share(at, seed);
            let line = snow_cells + tall * 0.035 * (fbm(at[0], at[1], 9.0, 4, seed ^ 0x77) - 0.3);
            let around = (columns.ground_at(xi - 2, zi)
                + columns.ground_at(xi + 2, zi)
                + columns.ground_at(xi, zi - 2)
                + columns.ground_at(xi, zi + 2)) as f32
                / 4.0;
            let hollow = (around - h as f32).max(0.0);
            let sandy = fbm(at[0], at[1], 10.0, 3, seed ^ 0x88);
            let river = river_wet(at);
            let pool = apart(at, POOL) < 0.04;
            let basin = apart(at, BASIN) < 0.05;
            let (far_river, _, stretch) = nearest(&RIVER, at);
            let gorge = (stretch <= 4
                && far_river < 0.05 + 0.04 * fbm(at[0], at[1], 12.0, 3, seed ^ 0x6A))
                || apart(at, POOL) < 0.09 + 0.03 * fbm(at[0], at[1], 9.0, 2, seed ^ 0x6B);
            let (skin, deep) = if (h as f32) > line && fall <= 3 {
                (
                    Skin::Snow,
                    1 + (hollow * 0.8).min(4.0) as u32 + u32::from(fall <= 1) + pack,
                )
            } else if river.is_some() || pool || basin {
                (Skin::Bed, 2)
            } else if gorge {
                (Skin::Rock, 0)
            } else if fall <= 1 && lake > -0.14 {
                (Skin::Sand, 3)
            } else if scree(at, seed) && (fall >= 2 || sandy < 0.62) {
                (Skin::Rock, 0)
            } else if scree(at, seed) {
                (Skin::Sand, 2)
            } else if fall <= 4 && (h as f32) < snow_cells * 0.97 {
                (Skin::Earth, 2 + (hollow * 0.5).min(3.0) as u32)
            } else {
                (Skin::Rock, 0)
            };
            let slot = z * side + x;
            if let Some(held) = columns.skin.get_mut(slot) {
                *held = skin;
            }
            if let Some(held) = columns.deep.get_mut(slot) {
                *held = deep;
            }
            let wet = if lake > 0.0 && h < level {
                level
            } else if river.is_some() {
                h + 2
            } else if pool && h < (POOL_HEIGHT * tall) as u32 + 1 {
                (POOL_HEIGHT * tall) as u32 + 1
            } else {
                0
            };
            if let Some(held) = columns.wet.get_mut(slot) {
                *held = wet.min(top);
            }
        }
    }

    let (w, d, t) = (grid.wide as usize, grid.deep as usize, grid.tall as usize);
    let mut kinds = vec![AIR as u8; w * d * t];
    let index = |x: usize, h: u32, z: usize| -> Option<usize> {
        let y = (d as u32).checked_sub(1 + h)? as usize;
        (x < w && z < t && y < d).then(|| x + w * (y + d * z))
    };
    let strata = |h: u32, x: usize, z: usize| {
        let band = fbm(x as f32 / s, z as f32 / s, 3.0, 2, seed ^ 0x99) * 6.0;
        ((h as f32 + band) / 5.0).floor() as u32
    };
    for z in 0..t.min(side) {
        for x in 0..w.min(side) {
            let slot = z * side + x;
            let h = columns.ground.get(slot).copied().unwrap_or(0);
            let skin = columns.skin.get(slot).copied().unwrap_or(Skin::Rock);
            let deep = columns.deep.get(slot).copied().unwrap_or(0);
            let wet = columns.wet.get(slot).copied().unwrap_or(0);
            let flat = columns.drop(x as i64, z as i64) <= 1;
            for up in 0..h {
                let from_top = h - 1 - up;
                let rock_band = strata(up, x, z) % 7;
                let kind = if from_top < deep {
                    match skin {
                        Skin::Snow => SNOW as u8,
                        Skin::Earth => EARTH as u8,
                        Skin::Sand => SAND as u8,
                        Skin::Bed => {
                            if flat && hashed(x as u32, up, z as u32, seed).is_multiple_of(3) {
                                SAND as u8
                            } else {
                                ROCK as u8
                            }
                        }
                        Skin::Rock => ROCK as u8,
                    }
                } else if skin == Skin::Earth && from_top < deep + 3 && rock_band == 3 {
                    EARTH as u8
                } else {
                    ROCK as u8
                };
                if let Some(at) = index(x, up, z)
                    && let Some(held) = kinds.get_mut(at)
                {
                    *held = kind;
                }
            }
            for up in h..wet {
                if let Some(at) = index(x, up, z)
                    && let Some(held) = kinds.get_mut(at)
                {
                    *held = WATER as u8;
                }
            }
        }
    }

    let supported = |kinds: &[u8], x: usize, h: u32, z: usize| -> bool {
        h == 0
            || index(x, h - 1, z)
                .and_then(|at| kinds.get(at))
                .is_some_and(|&kind| u32::from(kind) != AIR)
    };
    for z in 0..t.min(side) {
        for x in 0..w.min(side) {
            let at = [(x as f32 + 0.5) / s, (z as f32 + 0.5) / s];
            let reach = apart(at, [0.27, 0.33]);
            let tongue = 1.0 - reach / 0.11;
            if tongue <= 0.0 {
                continue;
            }
            let slot = z * side + x;
            let h = columns.ground.get(slot).copied().unwrap_or(0);
            let wet = columns.wet.get(slot).copied().unwrap_or(0);
            if wet > 0 || columns.drop(x as i64, z as i64) > 3 {
                continue;
            }
            let thick = (tongue * 0.025 * tall + 1.0) as u32;
            for up in h..h + thick {
                if up >= top {
                    break;
                }
                if let Some(cell_at) = index(x, up, z)
                    && supported(&kinds, x, up, z)
                    && let Some(held) = kinds.get_mut(cell_at)
                    && u32::from(*held) == AIR
                {
                    *held = SNOW as u8;
                }
            }
        }
    }
    forest(
        &mut kinds,
        grid,
        &columns,
        &Ecology {
            seed,
            level,
            snow_cells,
            side,
        },
    );
    boulders(&mut kinds, grid, &columns, seed, level);

    let mut cells = vec![0_u32; w * d * t];
    for z in 0..t {
        for x in 0..w {
            let open = (0..d as u32)
                .rev()
                .find(|&h| {
                    index(x, h, z)
                        .and_then(|at| kinds.get(at))
                        .is_some_and(|&kind| u32::from(kind) != AIR)
                })
                .map_or(0, |h| h + 1);
            let mut over = 0_u32;
            for h in 0..d as u32 {
                let Some(at) = index(x, h, z) else {
                    continue;
                };
                let kind = kinds.get(at).copied().unwrap_or(0);
                let heat = AMBIENT;
                let word = if u32::from(kind) == AIR && h >= open {
                    open_air(heat, over)
                } else if u32::from(kind) == AIR {
                    air(heat, over)
                } else if u32::from(kind) == SNOW {
                    alive(
                        SNOW,
                        heat,
                        hashed(x as u32, h, z as u32, seed ^ 0x5A0) & THAW,
                    )
                } else {
                    cell(u32::from(kind), heat)
                };
                over = if u32::from(kind) == AIR {
                    (over + 1).min(7)
                } else {
                    0
                };
                if let Some(slot) = cells.get_mut(at) {
                    *slot = word;
                }
            }
        }
    }
    Made {
        cells,
        ground: columns.ground,
        wet: columns.wet,
        level,
        snowline,
    }
}

struct Ecology {
    seed: u32,
    level: u32,
    snow_cells: f32,
    side: usize,
}

/// Pines placed by ecology: in the band between the shore and the tree
/// line, on gentle earth away from water, in clumps the noise draws, each
/// grown from its own variant; dead snags among them, more of them where
/// the fire will start.
fn forest(kinds: &mut [u8], grid: Grid, columns: &Columns, ecology: &Ecology) {
    let Ecology {
        seed,
        level,
        snow_cells,
        side,
    } = *ecology;
    let s = side as f32;
    let tall = grid.deep as f32;
    let spacing = (s / 56.0).max(1.6);
    let cells = (s / spacing) as u32;
    let margin = 3_i64;
    for cz in 0..cells {
        for cx in 0..cells {
            let key = hashed(cx, cz, 0x7EE, seed);
            let x = ((cx as f32 + 0.15 + 0.7 * unit(key)) * spacing) as i64;
            let z = ((cz as f32 + 0.15 + 0.7 * unit(mixed(key))) * spacing) as i64;
            if x < margin || z < margin || x >= s as i64 - margin || z >= s as i64 - margin {
                continue;
            }
            let Some(slot) = columns.slot(x, z) else {
                continue;
            };
            if columns.skin.get(slot) != Some(&Skin::Earth) {
                continue;
            }
            let h = columns.ground.get(slot).copied().unwrap_or(0);
            let at = [x as f32 / s, z as f32 / s];
            let line = snow_cells * (0.9 + 0.08 * fbm(at[0], at[1], 6.0, 3, seed ^ 0xAB));
            if h <= level + 2 || h as f32 > line || columns.drop(x, z) > 2 {
                continue;
            }
            let mut dry = true;
            for oz in -3..=3 {
                for ox in -3..=3 {
                    if columns
                        .slot(x + ox, z + oz)
                        .and_then(|near| columns.wet.get(near))
                        .is_some_and(|&wet| wet > 0)
                    {
                        dry = false;
                    }
                }
            }
            if !dry {
                continue;
            }
            let clumps = fbm(at[0], at[1], 9.0, 4, seed ^ 0xC1);
            let height_share = (h as f32 / line).clamp(0.0, 1.0);
            let near_burn = (1.0 - apart(at, BURN) / 0.16).max(0.0);
            let right = smooth(0.45, 0.6, at[0]) * (1.0 - smooth(0.55, 0.75, at[1]));
            let thin =
                0.42 + 0.25 * smooth(0.75, 1.0, height_share) - 0.15 * near_burn - 0.14 * right;
            if clumps < thin && unit(mixed(key ^ 0x5)) > 0.12 {
                continue;
            }
            let variant = mixed(key ^ 0xF00D);
            let size =
                mix(1.0, 0.55, smooth(0.6, 1.0, height_share)) * (0.75 + 0.5 * unit(variant));
            let dead = unit(mixed(variant)) < 0.04 + 0.22 * near_burn;
            let base = [x, i64::from(h), z];
            let height = (tall * 0.06 * size).max(4.0);
            grow(kinds, grid, base, height, variant, dead);
        }
    }
}

fn put(kinds: &mut [u8], grid: Grid, [x, h, z]: [i64; 3], kind: u32, over_air_only: bool) {
    let (w, d, t) = (grid.wide as i64, grid.deep as i64, grid.tall as i64);
    let y = d - 1 - h;
    if x < 0 || z < 0 || y < 0 || x >= w || z >= t || y >= d || h < 0 {
        return;
    }
    let at = (x + w * (y + d * z)) as usize;
    if let Some(held) = kinds.get_mut(at)
        && (!over_air_only || u32::from(*held) == AIR)
    {
        *held = kind as u8;
    }
}

/// One pine of `height` cells on the ground at `base`, its shape from
/// `variant`: a trunk, tiers of needles that droop toward their tips and
/// thin toward the top, a ragged outline, and a lean.
fn grow(kinds: &mut [u8], grid: Grid, base: [i64; 3], height: f32, variant: u32, dead: bool) {
    let [x, h, z] = base;
    let r = |shift: u32| unit(mixed(variant.rotate_left(shift)));
    let crown_from = (height * (0.18 + 0.15 * r(3))).max(1.0);
    let reach = height * (0.15 + 0.06 * r(5)) + 0.4;
    let droop = 0.35 + 0.4 * r(7);
    let gap = if height > 10.0 {
        2.0 + (r(9) * 1.6).floor()
    } else {
        2.0
    };
    let lean = [(r(11) - 0.5) * 0.12, (r(13) - 0.5) * 0.12];
    let sectors: [f32; 8] = std::array::from_fn(|at| 0.6 + 0.55 * r(17 + at as u32 * 2));
    let trunk = if dead { height * 0.8 } else { height - 1.0 };
    for up in 0..trunk as i64 {
        let shift = [
            (lean[0] * up as f32).round() as i64,
            (lean[1] * up as f32).round() as i64,
        ];
        put(
            kinds,
            grid,
            [x + shift[0], h + up, z + shift[1]],
            WOOD,
            true,
        );
    }
    let mut tier = crown_from;
    let mut index = 0_u32;
    while tier < height {
        let share = (tier - crown_from) / (height - crown_from).max(1.0);
        let radius = reach * (1.0 - share).powf(0.85) + 0.5;
        let centre = [x as f32 + lean[0] * tier, z as f32 + lean[1] * tier];
        let span = radius.ceil() as i64 + 1;
        for dz in -span..=span {
            for dx in -span..=span {
                let (fx, fz) = (dx as f32, dz as f32);
                let far = (fx * fx + fz * fz).sqrt();
                let angle = fz.atan2(fx);
                let sector = (((angle / std::f32::consts::TAU) + 1.0) * 8.0) as usize % 8;
                let limit = radius * sectors.get(sector).copied().unwrap_or(1.0);
                if far > limit + 0.3 {
                    continue;
                }
                let jag = mixed(
                    variant
                        ^ index.wrapping_mul(0x9E37)
                        ^ ((dx + 16) as u32 * 31 + (dz + 16) as u32),
                );
                if dead {
                    let branch =
                        (dx == 0 || dz == 0) && far <= radius * 0.6 && jag.is_multiple_of(3);
                    if branch && index.is_multiple_of(2) {
                        let sink = (droop * far * 0.5).round() as i64;
                        put(
                            kinds,
                            grid,
                            [
                                centre[0] as i64 + dx,
                                h + tier as i64 - sink,
                                centre[1] as i64 + dz,
                            ],
                            WOOD,
                            true,
                        );
                    }
                    continue;
                }
                if unit(jag) < 0.15 + 0.25 * (far / limit.max(0.5)) {
                    continue;
                }
                let sink = (droop * far * far / radius.max(1.0)).round() as i64;
                let cell_h = h + tier as i64 - sink;
                put(
                    kinds,
                    grid,
                    [
                        centre[0].round() as i64 + dx,
                        cell_h,
                        centre[1].round() as i64 + dz,
                    ],
                    LEAVES,
                    true,
                );
                if far < radius * 0.7 && unit(jag >> 8) < 0.5 {
                    put(
                        kinds,
                        grid,
                        [
                            centre[0].round() as i64 + dx,
                            cell_h + 1,
                            centre[1].round() as i64 + dz,
                        ],
                        LEAVES,
                        true,
                    );
                }
            }
        }
        tier += gap;
        index += 1;
    }
    if !dead {
        let tip = [x as f32 + lean[0] * height, z as f32 + lean[1] * height];
        for up in 0..2 {
            put(
                kinds,
                grid,
                [
                    tip[0].round() as i64,
                    h + height as i64 - 1 + up,
                    tip[1].round() as i64,
                ],
                LEAVES,
                true,
            );
        }
    }
}

/// Boulders and scree: blocks of rock at the feet of cliffs and steep
/// ground, and a few along the river.
fn boulders(kinds: &mut [u8], grid: Grid, columns: &Columns, seed: u32, level: u32) {
    let side = columns.side as i64;
    let s = side as f32;
    let tries = (side * side) as u32 / 40;
    for at in 0..tries {
        let key = hashed(at, 0xB0, 0x1D, seed);
        let x = i64::from(key % side as u32);
        let z = i64::from((key >> 12) % side as u32);
        let Some(slot) = columns.slot(x, z) else {
            continue;
        };
        let h = columns.ground.get(slot).copied().unwrap_or(0);
        if columns.wet.get(slot).is_some_and(|&wet| wet > 0) || h <= level {
            continue;
        }
        let mut cliff = 0_u32;
        for oz in -4..=4 {
            for ox in -4..=4 {
                cliff = cliff.max(columns.ground_at(x + ox, z + oz).saturating_sub(h));
            }
        }
        let skin = columns.skin.get(slot).copied().unwrap_or(Skin::Rock);
        let at = [x as f32 / s, z as f32 / s];
        let chance = if cliff > 6 {
            0.7
        } else if skin == Skin::Bed {
            0.5
        } else if scree(at, seed) {
            0.45
        } else {
            0.06
        };
        if unit(mixed(key)) > chance || skin == Skin::Snow {
            continue;
        }
        let radius = (0.6 + 2.2 * unit(mixed(key ^ 0x3)) * (s / 256.0).sqrt()).max(0.6);
        let span = radius.ceil() as i64;
        for dz in -span..=span {
            for dy in 0..=span {
                for dx in -span..=span {
                    let far = ((dx * dx + dz * dz) as f32 + (dy as f32 * 1.3).powi(2)).sqrt();
                    let jag = unit(hashed((x + dx) as u32, dy as u32, (z + dz) as u32, key)) * 0.6;
                    if far + jag > radius + 0.3 {
                        continue;
                    }
                    let foot = columns.ground_at(x + dx, z + dz);
                    let wet = columns
                        .slot(x + dx, z + dz)
                        .and_then(|near| columns.wet.get(near))
                        .is_none_or(|&wet| wet > 0);
                    if wet || foot + 1 < h || foot > h + 1 {
                        continue;
                    }
                    put(
                        kinds,
                        grid,
                        [x + dx, i64::from(h) + dy - 1, z + dz],
                        ROCK,
                        false,
                    );
                }
            }
        }
    }
}

/// The cells where the fire of a round may start: the top of each trunk
/// in the part of the forest the plan marks for it, as `[x, y, z]`.
pub fn sites(grid: Grid, cells: &[u32]) -> Vec<[u32; 3]> {
    let s = grid.wide as f32;
    let (w, d) = (grid.wide as usize, grid.deep as usize);
    let mut out = Vec::new();
    for z in 0..grid.tall {
        for x in 0..grid.wide {
            let at = [x as f32 / s, z as f32 / s];
            if apart(at, BURN) > 0.06 {
                continue;
            }
            let kind = |x: u32, y: u32, z: u32| {
                let slot = x as usize + w * (y as usize + d * z as usize);
                cells.get(slot).map_or(AIR, |&word| word & 0xF)
            };
            let Some(y) = (0..grid.deep).find(|&y| kind(x, y, z) == WOOD) else {
                continue;
            };
            let leafy = [(1, 0), (0, 1), (2, 0), (0, 2)].iter().any(|&(dx, dz)| {
                (0..3).any(|dy| {
                    let down = (y + dy).min(grid.deep - 1);
                    [
                        kind((x + dx).min(grid.wide - 1), down, z),
                        kind(x.saturating_sub(dx), down, z),
                        kind(x, down, (z + dz).min(grid.tall - 1)),
                        kind(x, down, z.saturating_sub(dz)),
                    ]
                    .contains(&LEAVES)
                })
            });
            if leafy {
                out.push([x, y, z]);
            }
        }
    }
    out
}

fn valley_scene(grid: Grid) -> Vec<u32> {
    made(grid, SEED).cells
}

/// How many engine steps apart the feed is written.
pub const FEED_EVERY: u32 = 4;

/// The seep, as the cells written before every [`FEED_EVERY`]th step: air
/// at the lake's floor, which bubbles up and takes water away, so the lake
/// the snowmelt fills rises only slowly. The river and the waterfall need
/// no spring: the sun melts the snowfield, and its water runs down.
pub fn feed(grid: Grid, cells: &[u32]) -> Vec<(u32, u32)> {
    let s = grid.wide as f32;
    let (w, d) = (grid.wide as usize, grid.deep as usize);
    let word = |x: u32, y: u32, z: u32| {
        cells
            .get(x as usize + w * (y as usize + d * z as usize))
            .copied()
            .unwrap_or(0)
    };
    let place = |at: Plan| {
        (
            ((at[0] * s) as u32).min(grid.wide - 1),
            ((at[1] * s) as u32).min(grid.tall - 1),
        )
    };
    let mut out = Vec::new();
    let scale = (s / 256.0).max(0.25);
    let (lx, lz) = place(LAKE);
    let sink = (3.0 * scale).ceil() as u32;
    for z in lz..(lz + sink).min(grid.tall) {
        let Some(bottom) = (0..grid.deep)
            .rev()
            .find(|&y| word(lx, y, z) & 0xF == WATER)
        else {
            continue;
        };
        for y in bottom.saturating_sub(FEED_EVERY - 1)..=bottom {
            for x in lx..(lx + sink).min(grid.wide) {
                let held = word(x, y, z);
                if held & 0xF == WATER {
                    let heat = grit::rule3::nature::temperature_of(held);
                    out.push((grit::geometry::Solid::cell(grid, [x, y, z]), air(heat, 0)));
                }
            }
        }
    }
    out.sort_unstable();
    out.dedup_by_key(|held| held.0);
    out
}

fn valley_life(grid: Grid, cells: &[u32]) -> Cycle {
    let calm = grid.wide.max(64);
    Cycle::new(grid, sites(grid, cells), calm, calm / 2)
}

fn valley_light(mut light: Light) -> Light {
    let rise = -light.toward_sun[1];
    let across = (1.0 - rise * rise).max(0.0).sqrt();
    let (x, z) = (0.8, -0.6_f32);
    let length = (x * x + z * z).sqrt().max(1e-6);
    light.toward_sun = [x / length * across, -rise, z / length * across];
    light.smoke = 2.2;
    light.exposure = 0.95;
    light
}

/// The valley: snow on a jagged peak, which the sun melts into a river
/// down a gorge and off a cliff into a lake, and a pine forest that burns
/// once in a while.
pub fn example() -> Example {
    let table = Table::empty()
        .with(ROCK, Material::solid(0x006E_6C6A, 0.22).strata(0.42))
        .with(EARTH, Material::solid(0x0055_4A30, 0.24).strata(0.18))
        .with(SAND, Material::solid(0x00D6_B572, 0.12).strata(0.06))
        .with(SNOW, Material::solid(0x00EE_F2F8, 0.05))
        .with(ICE, Material::solid(0x00B8_D4E2, 0.06).clear(0.55))
        .with(WATER, Material::solid(0x002A_7A84, 0.02).clear(0.8))
        .with(WOOD, Material::solid(0x003E_2E22, 0.22))
        .with(LEAVES, Material::solid(0x0026_3E1E, 0.34))
        .with(FIRE, Material::solid(0x00FF_6418, 0.5).glowing(2.0))
        .with(ASH, Material::solid(0x003A_3735, 0.28))
        .with(SMOKE, Material::solid(0x0058_524D, 0.0).gas(0.62))
        .with(STEAM, Material::solid(0x00D8_DADC, 0.0).gas(0.86))
        .with(CHAR, Material::solid(0x001C_1816, 0.3));
    Example {
        name: "valley",
        rule: "nature",
        params: vec![SUN],
        table,
        tools: vec![
            Tool {
                name: "light fire",
                word: alive(FIRE, KINDLING, WOOD_LIFE),
                radius: 2,
            },
            Tool {
                name: "pour water",
                word: cell(WATER, AMBIENT),
                radius: 3,
            },
            Tool {
                name: "drop snow",
                word: cell(SNOW, AMBIENT),
                radius: 3,
            },
            Tool {
                name: "place rock",
                word: cell(ROCK, AMBIENT),
                radius: 2,
            },
            Tool {
                name: "erase",
                word: air(AMBIENT, 0),
                radius: 4,
            },
        ],
        scene: valley_scene,
        warm: 160,
        dusk: 0.0,
        light: valley_light,
        pace: Some(6.0),
        life: Some(valley_life),
        feed: Some(feed),
    }
}
