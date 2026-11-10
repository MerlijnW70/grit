//! A lit 3D picture of a solid world, made by compute passes that read
//! `STATE` directly.
//!
//! Four passes, all into `PIXELS`:
//!
//! - [`occupied`] keeps a word a brick of 4³ cells, 1 when the brick holds
//!   a cell of a kind anyone sees. It runs over the octs the engine stepped
//!   this frame and rebuilds only the bricks they touch, each once, so a
//!   quiet world costs nothing to keep current. [`filled`] rebuilds every
//!   brick.
//! - [`primary`] marches each pixel's ray through the grid with an integer
//!   walk ([`march`]) that crosses empty bricks in one jump. It keeps the
//!   first cell it sees and, past cells it can see through, the first it
//!   cannot.
//! - [`shade()`] lights what each pixel sees: a sun traced through the grid
//!   from each corner of the face, ambient occlusion from the cells around
//!   the face, a cool sky, colour that varies cell by cell, darker cell
//!   edges, glowing kinds, cells seen through, and haze.
//!
//! What a kind looks like is the caller's [`Table`]; the passes name no
//! material. The camera is [`Rays`], whole numbers, so where a ray lands is
//! exact and the CPU finds the same cell ([`march::World::seen`]).

pub mod emit;
pub mod march;
pub mod occupancy;
pub mod shade;

use simdr::lanes::LaneError;

use crate::Groups;
use crate::layout::{BINDINGS, PIXELS};
use crate::sand::Grid;
use crate::view3::{Kinds, PALETTE};

pub use self::occupancy::{filled, occupancy, occupied};
pub use self::shade::{Shading, primary, shade};

/// A ray's start is held in sixteenths of a cell.
pub const UNIT_BITS: u32 = 4;
pub const UNIT: u32 = 1 << UNIT_BITS;

/// The most cells a camera may lie from any wall of the grid, on each axis.
pub const REACH: i32 = 1023;

/// The largest a ray's direction may be on any axis.
pub const STEEPEST: i32 = (1 << 18) - 1;

/// A brick is `2^BRICK_BITS` cells a side.
pub const BRICK_BITS: u32 = 2;

/// The cell word of a pixel that sees nothing.
pub const MISS: u32 = u32::MAX;

/// The face of a hit when the ray started in the cell it hit.
pub const INSIDE: u32 = 6;

/// The most sun directions a frame may trace shadows along.
pub const SUNS: usize = 4;

/// The words of a kind in the material table.
pub const MATERIAL_WORDS: usize = 8;

/// The words a pixel's hits take: the first cell seen and its face and run,
/// then the first cell that cannot be seen through, and its face and run.
pub const HIT_WORDS: usize = 4;

/// The largest side of a grid the renderer draws.
pub const LARGEST_SIDE: u32 = 512;

/// The widest and tallest screen.
pub const LARGEST_SCREEN: u32 = 4096;

/// Why a pass refuses a grid.
pub const UNFIT: &str = "the grid is not solid, has a side under 4 or over 512, or the screen is empty or over 4096 a side";

/// The words of the scene block, in order.
pub mod scene {
    pub const ORIGIN: usize = 0;
    pub const FORWARD: usize = 3;
    pub const RIGHT: usize = 6;
    pub const UP: usize = 9;
    pub const VISIBLE: usize = 12;
    pub const OPAQUE: usize = 13;
    pub const SUN_RAYS: usize = 14;
    pub const LIGHT: usize = SUN_RAYS + 3 * super::SUNS;
    pub const WORDS: usize = LIGHT + super::LIGHT_WORDS;
}

/// The size of the picture, in pixels; any size from 1 by 1 to
/// [`LARGEST_SCREEN`] a side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Screen {
    pub wide: u32,
    pub high: u32,
}

impl Screen {
    #[must_use]
    pub const fn pixels(self) -> usize {
        self.wide as usize * self.high as usize
    }

    /// The workgroups a pass over every pixel dispatches.
    #[must_use]
    pub fn groups(self, subgroup: u32) -> Groups {
        Groups::Fixed(u32::try_from(self.pixels().div_ceil(subgroup.max(1) as usize)).unwrap_or(0))
    }
}

/// Whether the renderer draws `grid` on `screen`.
#[must_use]
pub fn fits(grid: Grid, screen: Screen) -> bool {
    let side = |side: u32| side.is_power_of_two() && (4..=LARGEST_SIDE).contains(&side);
    side(grid.wide)
        && side(grid.deep)
        && side(grid.tall)
        && (1..=LARGEST_SCREEN).contains(&screen.wide)
        && (1..=LARGEST_SCREEN).contains(&screen.high)
}

pub(crate) fn refuse(operation: &'static str, grid: Grid, screen: Screen) -> Result<(), LaneError> {
    if fits(grid, screen) {
        Ok(())
    } else {
        Err(LaneError::NoSuchForm {
            operation,
            because: UNFIT,
        })
    }
}

/// The bricks of `grid`.
#[must_use]
pub const fn bricks(grid: Grid) -> u32 {
    (grid.wide >> BRICK_BITS) * (grid.deep >> BRICK_BITS) * (grid.tall >> BRICK_BITS)
}

/// Where each part of the picture lies in `PIXELS`: the colours, spare
/// words for lanes past the last pixel, the material table, the scene, the
/// hits, the brick occupancy and the brick claims.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub screen: Screen,
    pub subgroup: u32,
    pub bricks: u32,
}

impl Layout {
    #[must_use]
    pub const fn new(grid: Grid, screen: Screen, subgroup: u32) -> Self {
        Self {
            screen,
            subgroup,
            bricks: bricks(grid),
        }
    }

    #[must_use]
    pub const fn spare(self) -> usize {
        self.screen.pixels()
    }

    #[must_use]
    pub const fn table(self) -> usize {
        self.spare() + HIT_WORDS * self.subgroup as usize
    }

    #[must_use]
    pub const fn scene(self) -> usize {
        self.table() + PALETTE * MATERIAL_WORDS
    }

    #[must_use]
    pub const fn hits(self) -> usize {
        self.scene() + scene::WORDS
    }

    #[must_use]
    pub const fn occupancy(self) -> usize {
        self.hits() + HIT_WORDS * self.screen.pixels()
    }

    #[must_use]
    pub const fn claims(self) -> usize {
        self.occupancy() + self.bricks as usize
    }

    /// The words `PIXELS` holds for the picture.
    #[must_use]
    pub const fn words(self) -> usize {
        self.claims() + self.bricks as usize
    }

    /// `sizes` with `PIXELS` grown to [`Layout::words`].
    #[must_use]
    pub fn sized(self, mut sizes: [usize; BINDINGS as usize]) -> [usize; BINDINGS as usize] {
        if let Some(held) = sizes.get_mut(PIXELS as usize) {
            *held = (*held).max(self.words());
        }
        sizes
    }
}

/// The value of `values` on `axis`; an axis past z reads z.
#[must_use]
pub const fn of<T: Copy>(values: [T; 3], axis: usize) -> T {
    match axis {
        0 => values[0],
        1 => values[1],
        _ => values[2],
    }
}

/// `values` with `value` on `axis`; an axis past z writes z.
pub const fn put<T: Copy>(values: &mut [T; 3], axis: usize, value: T) {
    match axis {
        0 => values[0] = value,
        1 => values[1] = value,
        _ => values[2] = value,
    }
}

pub(crate) fn word(at: usize) -> u32 {
    u32::try_from(at).unwrap_or(u32::MAX)
}

/// A camera in whole numbers: where rays start, in sixteenths of a cell,
/// and the direction of pixel `(x, y)` of a screen `wide` by `high`:
/// `forward + (2x + 1 - wide) * right + (high - 2y - 1) * up`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rays {
    pub origin: [i32; 3],
    pub forward: [i32; 3],
    pub right: [i32; 3],
    pub up: [i32; 3],
}

impl Rays {
    /// The direction of pixel `(x, y)`.
    #[must_use]
    pub fn direction(&self, screen: Screen, (x, y): (u32, u32)) -> [i32; 3] {
        let across = (2 * i64::from(x) + 1 - i64::from(screen.wide)) as i32;
        let down = (i64::from(screen.high) - 2 * i64::from(y) - 1) as i32;
        let mut out = [0; 3];
        for axis in 0..3 {
            let held = of(self.forward, axis)
                .wrapping_add(across.wrapping_mul(of(self.right, axis)))
                .wrapping_add(down.wrapping_mul(of(self.up, axis)));
            put(&mut out, axis, held);
        }
        out
    }

    /// Whether the rays keep the march exact on `grid`: the start lies
    /// within [`REACH`] cells of every wall, and no pixel's direction
    /// passes [`STEEPEST`] on any axis.
    #[must_use]
    pub fn fit(&self, grid: Grid, screen: Screen) -> bool {
        let sides = march::sides(grid);
        let unit = UNIT as i64;
        let near = (0..3).all(|axis| {
            let at = i64::from(of(self.origin, axis));
            let side = i64::from(of(sides, axis));
            at >= unit * (side - i64::from(REACH)) && at <= unit * i64::from(REACH)
        });
        let corners = [
            (0, 0),
            (screen.wide - 1, 0),
            (0, screen.high - 1),
            (screen.wide - 1, screen.high - 1),
        ];
        let steep = corners.iter().all(|&(x, y)| {
            let across = 2 * i64::from(x) + 1 - i64::from(screen.wide);
            let down = i64::from(screen.high) - 2 * i64::from(y) - 1;
            (0..3).all(|axis| {
                let value = i64::from(of(self.forward, axis))
                    + across * i64::from(of(self.right, axis))
                    + down * i64::from(of(self.up, axis));
                value.abs() <= i64::from(STEEPEST)
            })
        });
        near && steep
    }
}

/// A camera in floats: where it is, in cells, which way it looks, and how
/// wide it sees. The y axis points down, as grains fall.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub at: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub view: f32,
}

fn normal(value: [f32; 3]) -> [f32; 3] {
    let length = (value[0] * value[0] + value[1] * value[1] + value[2] * value[2]).sqrt();
    if length > 0.0 {
        [value[0] / length, value[1] / length, value[2] / length]
    } else {
        value
    }
}

impl Camera {
    /// The camera at `at`, looking at `target`.
    #[must_use]
    pub fn looking(at: [f32; 3], target: [f32; 3], view: f32) -> Self {
        let way = normal([target[0] - at[0], target[1] - at[1], target[2] - at[2]]);
        Self {
            at,
            yaw: way[2].atan2(way[0]),
            pitch: way[1].clamp(-1.0, 1.0).asin(),
            view,
        }
    }

    /// The unit vectors forward, right and up on the screen.
    #[must_use]
    pub fn basis(&self) -> [[f32; 3]; 3] {
        let (pitch_sin, pitch_cos) = self.pitch.sin_cos();
        let (yaw_sin, yaw_cos) = self.yaw.sin_cos();
        let forward = [pitch_cos * yaw_cos, pitch_sin, pitch_cos * yaw_sin];
        let right = [-yaw_sin, 0.0, yaw_cos];
        let up = [
            right[1] * forward[2] - right[2] * forward[1],
            right[2] * forward[0] - right[0] * forward[2],
            right[0] * forward[1] - right[1] * forward[0],
        ];
        let up = if up[1] > 0.0 {
            [-up[0], -up[1], -up[2]]
        } else {
            up
        };
        [forward, right, normal(up)]
    }

    /// The camera kept within [`REACH`] of every wall of `grid`.
    #[must_use]
    pub fn kept(mut self, grid: Grid) -> Self {
        let sides = march::sides(grid);
        for axis in 0..3 {
            let side = of(sides, axis) as f32;
            let low = side - REACH as f32 + 1.0;
            let high = REACH as f32 - 1.0;
            let held = of(self.at, axis).clamp(low, high);
            put(&mut self.at, axis, held);
        }
        self
    }

    /// The camera's [`Rays`] for `screen` on `grid`.
    #[must_use]
    pub fn rays(&self, grid: Grid, screen: Screen) -> Rays {
        let kept = self.kept(grid);
        let [forward, right, up] = kept.basis();
        let spread = (kept.view * 0.5).tan() / screen.high.max(1) as f32;
        let width = (screen.wide.max(1) - 1) as f32;
        let height = (screen.high.max(1) - 1) as f32;
        let most = (0..3)
            .map(|axis| {
                of(forward, axis).abs()
                    + spread * (width * of(right, axis).abs() + height * of(up, axis).abs())
            })
            .fold(0.0_f32, f32::max)
            .max(f32::EPSILON);
        let mut scale = (STEEPEST as f32 - 4.0 * (width + height + 1.0)) / most;
        let origin = [0, 1, 2].map(|axis| (of(kept.at, axis) * UNIT as f32).round() as i32);
        loop {
            let rays = Rays {
                origin,
                forward: forward.map(|value| (value * scale).round() as i32),
                right: right.map(|value| (value * spread * scale).round() as i32),
                up: up.map(|value| (value * spread * scale).round() as i32),
            };
            if rays.fit(grid, screen) || scale < 1.0 {
                return rays;
            }
            scale *= 0.99;
        }
    }

    /// The camera moved by `by` cells along its own forward, right and up.
    #[must_use]
    pub fn flown(mut self, by: [f32; 3]) -> Self {
        let basis = self.basis();
        for (along, amount) in basis.iter().zip(by) {
            for axis in 0..3 {
                let held = of(self.at, axis) + of(*along, axis) * amount;
                put(&mut self.at, axis, held);
            }
        }
        self
    }
}

/// How a kind looks: its colour in linear light, how much its cells vary
/// in brightness, how brightly it glows, and how much light one cell of it
/// lets through: 1 is empty, 0 is opaque, between is seen through.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Material {
    pub albedo: [f32; 3],
    pub jitter: f32,
    pub glow: f32,
    pub clear: f32,
}

impl Material {
    /// A kind nothing sees.
    pub const EMPTY: Self = Self {
        albedo: [0.0; 3],
        jitter: 0.0,
        glow: 0.0,
        clear: 1.0,
    };

    /// An opaque kind of colour `rgb`, an sRGB triple, whose cells vary by
    /// `jitter`.
    #[must_use]
    pub fn solid(rgb: u32, jitter: f32) -> Self {
        let channel = |shift: u32| (((rgb >> shift) & 0xFF) as f32 / 255.0).powf(2.2);
        Self {
            albedo: [channel(16), channel(8), channel(0)],
            jitter,
            glow: 0.0,
            clear: 0.0,
        }
    }

    #[must_use]
    pub const fn glowing(mut self, glow: f32) -> Self {
        self.glow = glow;
        self
    }

    #[must_use]
    pub const fn clear(mut self, clear: f32) -> Self {
        self.clear = clear;
        self
    }

    fn words(self) -> [u32; MATERIAL_WORDS] {
        [
            self.albedo[0].to_bits(),
            self.albedo[1].to_bits(),
            self.albedo[2].to_bits(),
            self.jitter.to_bits(),
            self.glow.to_bits(),
            self.clear.to_bits(),
            0,
            0,
        ]
    }
}

/// A material for every kind a [`Kinds`] mapping gives.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Table(pub [Material; PALETTE]);

impl Table {
    /// Every kind empty.
    #[must_use]
    pub const fn empty() -> Self {
        Self([Material::EMPTY; PALETTE])
    }

    /// The table with `kind` drawn as `material`.
    #[must_use]
    pub fn with(mut self, kind: u32, material: Material) -> Self {
        if let Some(slot) = self.0.get_mut(kind as usize) {
            *slot = material;
        }
        self
    }

    #[must_use]
    pub fn material(&self, kind: u32) -> Material {
        self.0
            .get(kind as usize)
            .copied()
            .unwrap_or(Material::EMPTY)
    }

    /// The kinds anyone sees, a bit each.
    #[must_use]
    pub fn visible(&self) -> u32 {
        (0_u32..)
            .zip(self.0.iter())
            .filter(|(_, material)| material.clear < 1.0)
            .fold(0, |mask, (kind, _)| mask | (1 << kind))
    }

    /// The kinds nobody sees through, a bit each.
    #[must_use]
    pub fn opaque(&self) -> u32 {
        (0_u32..)
            .zip(self.0.iter())
            .filter(|(_, material)| material.clear <= 0.0)
            .fold(0, |mask, (kind, _)| mask | (1 << kind))
    }

    /// The table as the words `PIXELS` holds from [`Layout::table`] on.
    #[must_use]
    pub fn words(&self) -> Vec<u32> {
        self.0
            .iter()
            .flat_map(|material| material.words())
            .collect()
    }
}

/// The light of a frame: the way to the sun and its colour, the sky's
/// colour overhead and at the horizon, the light thrown up from the
/// ground, the colour of the plane under the world, how thick the haze is
/// a cell, the exposure, how much each neighbour darkens a corner, how
/// dark a cell's edge gets, and the screen's focal length in pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Light {
    pub toward_sun: [f32; 3],
    pub sun: [f32; 3],
    pub zenith: [f32; 3],
    pub horizon: [f32; 3],
    pub ground: [f32; 3],
    pub floor: [f32; 3],
    pub haze: f32,
    pub exposure: f32,
    pub occlusion: f32,
    pub edge: f32,
    pub focal: f32,
}

/// The words of a [`Light`].
pub const LIGHT_WORDS: usize = 23;

impl Light {
    /// A low warm sun under a cool sky, for a screen `high` pixels tall seen
    /// through a camera of view `view`.
    #[must_use]
    pub fn evening(high: u32, view: f32) -> Self {
        Self {
            toward_sun: normal([-0.3, -0.5, -0.8]),
            sun: [3.3, 2.5, 1.65],
            zenith: [0.17, 0.34, 0.82],
            horizon: [0.56, 0.66, 0.84],
            ground: [0.30, 0.24, 0.17],
            floor: [0.24, 0.23, 0.21],
            haze: 0.001,
            exposure: 0.95,
            occlusion: 0.24,
            edge: 0.25,
            focal: high as f32 * 0.5 / (view * 0.5).tan(),
        }
    }

    /// The light as words, in [`scene`] order from [`scene::LIGHT`] on.
    #[must_use]
    pub fn words(&self) -> [u32; LIGHT_WORDS] {
        let mut out = [0; LIGHT_WORDS];
        let tail = [
            self.haze,
            self.exposure,
            self.occlusion,
            self.edge,
            self.focal,
        ];
        let values = self
            .toward_sun
            .iter()
            .chain(&self.sun)
            .chain(&self.zenith)
            .chain(&self.horizon)
            .chain(&self.ground)
            .chain(&self.floor)
            .chain(&tail);
        for (slot, value) in out.iter_mut().zip(values) {
            *slot = value.to_bits();
        }
        out
    }

    /// `count` whole-number directions toward the sun, up to [`SUNS`],
    /// spread over a disc `spread` wide around it, so shadow edges soften
    /// with distance.
    #[must_use]
    pub fn sun_rays(&self, count: usize, spread: f32) -> Vec<[i32; 3]> {
        let toward = normal(self.toward_sun);
        let side = normal([toward[2], 0.0, -toward[0]]);
        let other = normal([
            toward[1] * side[2] - toward[2] * side[1],
            toward[2] * side[0] - toward[0] * side[2],
            toward[0] * side[1] - toward[1] * side[0],
        ]);
        let offsets: &[(f32, f32)] = match count {
            0 | 1 => &[(0.0, 0.0)],
            2 => &[(-0.5, -0.25), (0.5, 0.25)],
            3 => &[(0.0, 0.58), (-0.5, -0.29), (0.5, -0.29)],
            _ => &[(-0.5, -0.5), (0.5, -0.5), (-0.5, 0.5), (0.5, 0.5)],
        };
        let scale = (STEEPEST / 2) as f32;
        offsets
            .iter()
            .map(|&(a, b)| {
                let way = normal([0, 1, 2].map(|axis| {
                    of(toward, axis) + spread * (a * of(side, axis) + b * of(other, axis))
                }));
                way.map(|value| (value * scale).round() as i32)
            })
            .collect()
    }
}

/// The scene block of `PIXELS`: the rays, the kinds seen and opaque, the
/// sun rays and the light.
#[must_use]
pub fn scene_words(rays: &Rays, table: &Table, suns: &[[i32; 3]], light: &Light) -> Vec<u32> {
    let mut out = vec![0; scene::WORDS];
    let mut put = |at: usize, value: u32| {
        if let Some(slot) = out.get_mut(at) {
            *slot = value;
        }
    };
    for axis in 0..3 {
        put(scene::ORIGIN + axis, of(rays.origin, axis) as u32);
        put(scene::FORWARD + axis, of(rays.forward, axis) as u32);
        put(scene::RIGHT + axis, of(rays.right, axis) as u32);
        put(scene::UP + axis, of(rays.up, axis) as u32);
    }
    put(scene::VISIBLE, table.visible());
    put(scene::OPAQUE, table.opaque());
    for (at, sun) in suns.iter().take(SUNS).enumerate() {
        for axis in 0..3 {
            put(scene::SUN_RAYS + 3 * at + axis, of(*sun, axis) as u32);
        }
    }
    for (at, value) in light.words().into_iter().enumerate() {
        put(scene::LIGHT + at, value);
    }
    out
}

/// What the passes need to know when they are built: the grid, how its
/// words map to kinds, the screen, the subgroup width, and how many sun
/// rays a corner traces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Build {
    pub grid: Grid,
    pub kinds: Kinds,
    pub screen: Screen,
    pub subgroup: u32,
    pub suns: u32,
}

impl Build {
    #[must_use]
    pub const fn layout(&self) -> Layout {
        Layout::new(self.grid, self.screen, self.subgroup)
    }

    /// Every pass, in the order [`Build::stages`] dispatches them:
    /// [`occupied`], [`filled`], [`primary`], [`shade()`].
    pub fn passes(&self) -> Result<Vec<Vec<u32>>, LaneError> {
        Ok(vec![
            occupied(self)?,
            filled(self)?,
            primary(self)?,
            shade(self)?,
        ])
    }

    /// The groups of the stages after the engine's: the full rebuild when
    /// `refill`, then the march, then the shading.
    #[must_use]
    pub fn stages(&self, refill: bool) -> Vec<Vec<Groups>> {
        let bricks =
            u32::try_from((bricks(self.grid) as usize).div_ceil(self.subgroup.max(1) as usize))
                .unwrap_or(0);
        vec![
            vec![Groups::Fixed(if refill { bricks } else { 0 })],
            vec![self.screen.groups(self.subgroup)],
            vec![self.screen.groups(self.subgroup)],
        ]
    }
}

/// The engine's frame with [`occupied`] put in its own stage right after
/// the stage that steps the listed octs, and the remaining passes of
/// [`Build::passes`] after the engine's own; or none when the frame has no
/// stage of the listed groups alone.
#[must_use]
pub fn watching(engine: crate::live::Loop, passes: Vec<Vec<u32>>) -> Option<crate::live::Loop> {
    let (mut modules, mut groups) = engine;
    let listed = crate::compact::Plan::listed();
    let stepped = groups.iter().position(|stage| stage == &vec![listed])?;
    let before: usize = groups.iter().take(stepped + 1).map(Vec::len).sum();
    let mut rest = passes.into_iter();
    let watcher = rest.next()?;
    modules.insert(before, watcher);
    groups.insert(stepped + 1, vec![listed]);
    modules.extend(rest);
    Some((modules, groups))
}

fn crc_table() -> [u32; 256] {
    let mut table = [0_u32; 256];
    for (n, slot) in (0_u32..).zip(table.iter_mut()) {
        let mut c = n;
        for _ in 0..8 {
            c = if c & 1 == 1 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
        *slot = c;
    }
    table
}

fn crc(table: &[u32; 256], bytes: &[u8]) -> u32 {
    let mut c = u32::MAX;
    for &byte in bytes {
        let at = ((c ^ u32::from(byte)) & 0xFF) as usize;
        c = table.get(at).copied().unwrap_or(0) ^ (c >> 8);
    }
    c ^ u32::MAX
}

fn chunk(out: &mut Vec<u8>, table: &[u32; 256], kind: &[u8; 4], data: &[u8]) {
    let length = u32::try_from(data.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&length.to_be_bytes());
    let mut body = kind.to_vec();
    body.extend_from_slice(data);
    out.extend_from_slice(&body);
    out.extend_from_slice(&crc(table, &body).to_be_bytes());
}

/// `colours`, `0x00RRGGBB` a pixel, row by row, as an uncompressed PNG.
#[must_use]
pub fn png(colours: &[u32], screen: Screen) -> Vec<u8> {
    let mut raw = Vec::with_capacity(colours.len() * 3 + screen.high as usize);
    for row in colours.chunks(screen.wide.max(1) as usize) {
        raw.push(0);
        for &pixel in row {
            raw.extend_from_slice(pixel.to_be_bytes().get(1..4).unwrap_or(&[0, 0, 0]));
        }
    }
    let mut packed = vec![0x78, 0x01];
    let mut blocks = raw.chunks(0xFFFF).peekable();
    while let Some(block) = blocks.next() {
        packed.push(u8::from(blocks.peek().is_none()));
        let length = u16::try_from(block.len()).unwrap_or(u16::MAX);
        packed.extend_from_slice(&length.to_le_bytes());
        packed.extend_from_slice(&(!length).to_le_bytes());
        packed.extend_from_slice(block);
    }
    let (mut a, mut b) = (1_u32, 0_u32);
    for &byte in &raw {
        a = (a + u32::from(byte)) % 65521;
        b = (b + a) % 65521;
    }
    packed.extend_from_slice(&((b << 16) | a).to_be_bytes());

    let table = crc_table();
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut header = Vec::new();
    header.extend_from_slice(&screen.wide.to_be_bytes());
    header.extend_from_slice(&screen.high.to_be_bytes());
    header.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut out, &table, b"IHDR", &header);
    chunk(&mut out, &table, b"IDAT", &packed);
    chunk(&mut out, &table, b"IEND", &[]);
    out
}
