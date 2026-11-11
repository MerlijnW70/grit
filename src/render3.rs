//! A lit 3D picture of a solid world, made by compute passes that read
//! `STATE` directly.
//!
//! The passes, all into `PIXELS`:
//!
//! - [`occupied`] keeps a word a brick of 4³ cells, 1 when the brick holds
//!   a cell of a kind anyone sees or of a gas, and with it the light the
//!   brick's glowing cells give off and the share of it that is gas. It
//!   runs over the octs the engine stepped this frame and rebuilds only the
//!   bricks they touch, each once, so a quiet world costs nothing to keep
//!   current. It stamps each oct it runs over. [`filled`] rebuilds every
//!   brick.
//! - [`lit_across`], [`lit_down`] and [`lit_deep`] spread the light of the
//!   glowing cells over the bricks around them, one axis each, in frames
//!   in which a brick changed the light it gives off. [`sunlit`] finds how
//!   much sun reaches each brick through the gas.
//! - [`primary`] marches each pixel's ray through the grid with an integer
//!   walk ([`march`]) that crosses empty bricks in one jump. It keeps the
//!   first cell it sees and, past cells it can see through, the first it
//!   cannot, and gathers the light of the gas it crosses on the way.
//! - [`bounce`] sends two more rays from each pixel whose first cell is the
//!   top of a liquid: one mirrored off the surface, one bent into it, each
//!   by the same integer march, so the CPU finds the same cells.
//! - [`shade()`] lights what each pixel sees: a sun traced through the grid
//!   from each corner of the face, ambient occlusion from the cells around
//!   the face, the light of glowing cells nearby, a sky, colour that varies
//!   cell by cell and layer by layer, darker cell edges, glowing kinds,
//!   cells seen through, gas, haze, the floor the world stands on and the
//!   glass around it.
//! - [`resolve`] keeps each pixel's light over frames, found again where
//!   the camera has moved, so the edges a frame samples at a slightly
//!   different point each time average out.
//! - [`bloom_down`], [`bloom_across`] and [`bloom_up`] blur the brightest
//!   light, and [`composite`] adds it, maps the light to the screen and
//!   writes each pixel's colour.
//!
//! What a kind looks like is the caller's [`Table`]; the passes name no
//! material. The camera is [`Rays`], whole numbers, so where a ray lands is
//! exact and the CPU finds the same cell ([`march::World::seen`]).

mod detail;
pub mod emit;
pub mod march;
pub mod occupancy;
pub mod post;
pub mod shade;
pub mod volume;

use simdr::lanes::LaneError;

use crate::Groups;
use crate::layout::{BINDINGS, PIXELS};
use crate::sand::Grid;
use crate::view3::{Kinds, PALETTE};

pub use self::occupancy::{filled, occupancy, occupied};
pub use self::post::{bloom_across, bloom_down, bloom_up, composite, resolve};
pub use self::shade::{Shading, bounce, primary, shade};
pub use self::volume::{lit_across, lit_deep, lit_down, sunlit};

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

/// The sun directions the floor picks its shadow rays from; a power of two.
pub const FLOOR_RAYS: usize = 16;

/// The words of a kind in the material table.
pub const MATERIAL_WORDS: usize = 26;

/// The words a pixel's bounced rays take: the cell the mirrored ray meets
/// and its face, then the cell the bent ray meets and its face.
pub const BOUNCE_WORDS: usize = 4;

/// The bit of a bounced face word that tells the pixel sent its rays.
pub const BOUNCED: u32 = 8;

/// The words of a brick's water: the share of its cells that are liquid,
/// and the share that are liquid with open air beside them.
pub const WATER_WORDS: usize = 2;

/// The words a pixel's hits take: the first cell seen and its face and run,
/// then the first cell that cannot be seen through, and its face and run.
pub const HIT_WORDS: usize = 4;

/// The words of the gas a pixel's ray crosses before its first hit: the
/// light it gathers, red, green and blue, then how much of what lies
/// behind still shows.
pub const FOG_WORDS: usize = 4;

/// The words of a pixel's light before it is kept over frames: its colour,
/// the part of it that its own cell glows, and how far its ray went.
pub const HDR_WORDS: usize = 8;

/// The words of a pixel's light kept over frames: its colour and the glow
/// that fades behind a moving glowing cell.
pub const HISTORY_WORDS: usize = 6;

/// The words of a pixel of the blurred glow.
pub const BLOOM_WORDS: usize = 3;

/// The words of a brick's gas.
pub const GAS_WORDS: usize = 4;

/// The bit of a brick's occupancy word that tells it holds gas.
pub const GASSY: u32 = 2;

/// The most words a lane past the last pixel or brick writes.
pub const SPARE_WORDS: usize = 8;

/// The bloom is blurred at a quarter of the screen on each side.
pub const BLOOM_SCALE: u32 = 4;

/// How many bricks the light of a glowing cell reaches on each side of its
/// own along each axis.
pub const LIGHT_REACH: u32 = 4;

/// How many bricks a ray from a brick toward the sun crosses to find the
/// gas between it and the sun.
pub const SUN_STEPS: u32 = 24;

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
    /// The kinds a brick counts as full for: seen, or gas.
    pub const SHOWN: usize = SUN_RAYS + 3 * super::SUNS;
    pub const GAS: usize = SHOWN + 1;
    pub const GLOWING: usize = GAS + 1;
    /// The kinds drawn as a liquid.
    pub const LIQUID: usize = GLOWING + 1;
    /// The bits of [`super::flags`].
    pub const FLAGS: usize = LIQUID + 1;
    /// A count the host raises every frame it draws.
    pub const DRAWN: usize = FLAGS + 1;
    /// [`super::FLOOR_RAYS`] whole-number directions toward the sun, spread
    /// over a square across it: each pixel of the floor traces some of
    /// them, starting at one its own hash picks.
    pub const FLOOR_RAYS: usize = DRAWN + 1;
    /// The camera of the frame before, in floats: where it stood, its
    /// forward, and its right and up each divided by the spread of a pixel.
    pub const BEFORE: usize = FLOOR_RAYS + 3 * super::FLOOR_RAYS;
    pub const LIGHT: usize = BEFORE + 12;
    pub const WORDS: usize = LIGHT + super::LIGHT_WORDS;
}

/// The bits of [`scene::FLAGS`].
pub mod flags {
    /// Nothing is kept from the frames before.
    pub const RESET: u32 = 1;
    /// The octs the engine stepped glow, and the rest is dimmed.
    pub const ACTIVE: u32 = 2;
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

    /// The screen the bloom is blurred on.
    #[must_use]
    pub const fn quarter(self) -> Self {
        Self {
            wide: self.wide.div_ceil(BLOOM_SCALE),
            high: self.high.div_ceil(BLOOM_SCALE),
        }
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

/// The bricks of `grid` along each axis.
#[must_use]
pub const fn brick_sides(grid: Grid) -> [u32; 3] {
    [
        grid.wide >> BRICK_BITS,
        grid.deep >> BRICK_BITS,
        grid.tall >> BRICK_BITS,
    ]
}

/// Where each part of the picture lies in `PIXELS`.
///
/// In order: the colours, spare words for lanes past the last pixel or
/// brick, the material table, the scene, the hits, the gas each ray
/// crosses, the light of each pixel, the light kept over frames twice over
/// (one written while the other is read), the bloom twice over, the brick
/// occupancy, the brick claims, the light each brick gives off, the share
/// of each brick that is gas, the light of glowing cells spread over the
/// bricks (two halfway steps, then the result), how much sun reaches each
/// brick through the gas, a stamp an oct, the frame the light last
/// changed, the water of each brick, and the bounced rays of each pixel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub screen: Screen,
    pub subgroup: u32,
    pub bricks: u32,
    pub octs: u32,
}

impl Layout {
    #[must_use]
    pub const fn new(grid: Grid, screen: Screen, subgroup: u32) -> Self {
        Self {
            screen,
            subgroup,
            bricks: bricks(grid),
            octs: grid.blocks(),
        }
    }

    const fn pixels(self) -> usize {
        self.screen.pixels()
    }

    const fn quarters(self) -> usize {
        self.screen.quarter().pixels()
    }

    #[must_use]
    pub const fn spare(self) -> usize {
        self.pixels()
    }

    #[must_use]
    pub const fn table(self) -> usize {
        self.spare() + SPARE_WORDS * self.subgroup as usize
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
    pub const fn fog(self) -> usize {
        self.hits() + HIT_WORDS * self.pixels()
    }

    #[must_use]
    pub const fn hdr(self) -> usize {
        self.fog() + FOG_WORDS * self.pixels()
    }

    /// The light kept over frames, written in frames of parity `parity`.
    #[must_use]
    pub const fn history(self, parity: u32) -> usize {
        self.hdr()
            + HDR_WORDS * self.pixels()
            + (parity & 1) as usize * HISTORY_WORDS * self.pixels()
    }

    #[must_use]
    pub const fn bloom(self, which: u32) -> usize {
        self.history(0)
            + 2 * HISTORY_WORDS * self.pixels()
            + (which & 1) as usize * BLOOM_WORDS * self.quarters()
    }

    #[must_use]
    pub const fn occupancy(self) -> usize {
        self.bloom(0) + 2 * BLOOM_WORDS * self.quarters()
    }

    #[must_use]
    pub const fn claims(self) -> usize {
        self.occupancy() + self.bricks as usize
    }

    /// The light each brick gives off: red, green and blue a brick.
    #[must_use]
    pub const fn emission(self) -> usize {
        self.claims() + self.bricks as usize
    }

    /// The gas of each brick, [`GAS_WORDS`] a brick: the share of light a
    /// cell of it takes, on average over its cells, then that share times
    /// the gas's colour, red, green and blue.
    #[must_use]
    pub const fn gas(self) -> usize {
        self.emission() + 3 * self.bricks as usize
    }

    /// The light of glowing cells spread over the bricks: after the spread
    /// along x (0), along y too (1), and along all three (2).
    #[must_use]
    pub const fn lamp(self, step: u32) -> usize {
        self.gas() + GAS_WORDS * self.bricks as usize + 3 * self.bricks as usize * step as usize
    }

    #[must_use]
    pub const fn sunlit(self) -> usize {
        self.lamp(3)
    }

    #[must_use]
    pub const fn active(self) -> usize {
        self.sunlit() + self.bricks as usize
    }

    /// The word that holds the frame, plus one, in which a brick last
    /// changed the light it gives off.
    #[must_use]
    pub const fn changed(self) -> usize {
        self.active() + self.octs as usize
    }

    /// The water of each brick, [`WATER_WORDS`] a brick.
    #[must_use]
    pub const fn water(self) -> usize {
        self.changed() + 1
    }

    /// The bounced rays of each pixel, [`BOUNCE_WORDS`] a pixel.
    #[must_use]
    pub const fn bounces(self) -> usize {
        self.water() + WATER_WORDS * self.bricks as usize
    }

    /// The words `PIXELS` holds for the picture.
    #[must_use]
    pub const fn words(self) -> usize {
        self.bounces() + BOUNCE_WORDS * self.pixels()
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

    /// How far one pixel of `screen` turns the ray, as a share of forward.
    #[must_use]
    pub fn spread(&self, screen: Screen) -> f32 {
        (self.view * 0.5).tan() / screen.high.max(1) as f32
    }

    /// The camera's [`Rays`] for `screen` on `grid`.
    #[must_use]
    pub fn rays(&self, grid: Grid, screen: Screen) -> Rays {
        self.rays_shifted(grid, screen, [0.0, 0.0])
    }

    /// The camera's [`Rays`] for `screen` on `grid`, every ray moved by
    /// `shift` pixels, right and down: how a frame of a still camera
    /// samples a different point of each pixel.
    #[must_use]
    pub fn rays_shifted(&self, grid: Grid, screen: Screen, shift: [f32; 2]) -> Rays {
        let kept = self.kept(grid);
        let [forward, right, up] = kept.basis();
        let spread = kept.spread(screen);
        let width = (screen.wide.max(1) - 1) as f32;
        let height = (screen.high.max(1) - 1) as f32;
        let aimed = [0, 1, 2].map(|axis| {
            of(forward, axis)
                + 2.0 * spread * (shift[0] * of(right, axis) - shift[1] * of(up, axis))
        });
        let most = (0..3)
            .map(|axis| {
                of(aimed, axis).abs()
                    + spread * (width * of(right, axis).abs() + height * of(up, axis).abs())
            })
            .fold(0.0_f32, f32::max)
            .max(f32::EPSILON);
        let mut scale = (STEEPEST as f32 - 4.0 * (width + height + 1.0)) / most;
        let origin = [0, 1, 2].map(|axis| (of(kept.at, axis) * UNIT as f32).round() as i32);
        loop {
            let rays = Rays {
                origin,
                forward: aimed.map(|value| (value * scale).round() as i32),
                right: right.map(|value| (value * spread * scale).round() as i32),
                up: up.map(|value| (value * spread * scale).round() as i32),
            };
            if rays.fit(grid, screen) || scale < 1.0 {
                return rays;
            }
            scale *= 0.99;
        }
    }

    /// The words of [`scene::BEFORE`] for this camera on `screen` and
    /// `grid`: where it stands, its forward, and its right and up divided
    /// by the spread of a pixel.
    #[must_use]
    pub fn before(&self, grid: Grid, screen: Screen) -> [u32; 12] {
        let kept = self.kept(grid);
        let [forward, right, up] = kept.basis();
        let spread = kept.spread(screen).max(f32::EPSILON);
        let mut out = [0; 12];
        let values = kept
            .at
            .iter()
            .copied()
            .chain(forward)
            .chain(right.map(|value| value / spread))
            .chain(up.map(|value| value / spread));
        for (slot, value) in out.iter_mut().zip(values) {
            *slot = value.to_bits();
        }
        out
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

/// How a kind looks.
///
/// - `albedo`: its colour in linear light;
/// - `jitter`: how much its cells vary in brightness;
/// - `glow`: how brightly it glows;
/// - `clear`: how much light one cell of it lets through: 1 is empty, 0 is
///   opaque, between is seen through;
/// - `gas`: drawn as a soft volume that light passes through, never as a
///   cell with faces; one cell of it lets `clear` of the light through;
/// - `strata`: how much its colour changes from one layer of cells to the
///   next, so a cut through it shows bands;
/// - `gap`: the share of a cell's face, at its rim, that is dark, so cells
///   of a kind side by side stay apart;
/// - `rough`: how rough its surface is, from 0, a mirror, to 1, matte;
/// - `shine`: the share of light it reflects head on;
/// - `grain`: how much a fine noise over its surface varies its colour;
/// - `bump`: how much that noise tilts the light across a face;
/// - `cracks`: how dark the thin lines of a ridged noise run over it;
/// - `streak`: how far its noise is drawn out up and down, from 0, even,
///   toward 1, long streaks;
/// - `moss`, `tint`: how much of its surface turns `tint` where it faces
///   away from the sun or lies near water, and in patches;
/// - `wets`: how much darker and shinier it gets next to water;
/// - `sparkle`: how often its surface glints in the sun;
/// - `liquid`: drawn as a liquid: its top mirrors and bends light, its
///   colour deepens to `deep` with the water a ray crosses, and it foams.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Material {
    pub albedo: [f32; 3],
    pub jitter: f32,
    pub glow: f32,
    pub clear: f32,
    pub gas: bool,
    pub strata: f32,
    pub gap: f32,
    pub rough: f32,
    pub shine: f32,
    pub grain: f32,
    pub bump: f32,
    pub cracks: f32,
    pub streak: f32,
    pub moss: f32,
    pub tint: [f32; 3],
    pub wets: f32,
    pub sparkle: f32,
    pub liquid: bool,
    pub deep: [f32; 3],
}

impl Material {
    /// A kind nothing sees.
    pub const EMPTY: Self = Self {
        albedo: [0.0; 3],
        jitter: 0.0,
        glow: 0.0,
        clear: 1.0,
        gas: false,
        strata: 0.0,
        gap: 0.0,
        rough: 1.0,
        shine: 0.0,
        grain: 0.0,
        bump: 0.0,
        cracks: 0.0,
        streak: 0.0,
        moss: 0.0,
        tint: [0.0; 3],
        wets: 0.0,
        sparkle: 0.0,
        liquid: false,
        deep: [0.0; 3],
    };

    /// The linear light of `rgb`, an sRGB triple.
    #[must_use]
    pub fn linear(rgb: u32) -> [f32; 3] {
        let channel = |shift: u32| (((rgb >> shift) & 0xFF) as f32 / 255.0).powf(2.2);
        [channel(16), channel(8), channel(0)]
    }

    /// An opaque kind of colour `rgb`, an sRGB triple, whose cells vary by
    /// `jitter`.
    #[must_use]
    pub fn solid(rgb: u32, jitter: f32) -> Self {
        let albedo = Self::linear(rgb);
        Self {
            albedo,
            jitter,
            clear: 0.0,
            rough: 0.9,
            shine: 0.04,
            tint: albedo,
            deep: albedo,
            ..Self::EMPTY
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

    /// The kind as a gas of which one cell lets `clear` of the light
    /// through.
    #[must_use]
    pub const fn gas(mut self, clear: f32) -> Self {
        self.gas = true;
        self.clear = clear;
        self
    }

    #[must_use]
    pub const fn strata(mut self, strata: f32) -> Self {
        self.strata = strata;
        self
    }

    #[must_use]
    pub const fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    /// The surface as rough as `rough`, reflecting `shine` of the light
    /// head on.
    #[must_use]
    pub const fn surface(mut self, rough: f32, shine: f32) -> Self {
        self.rough = rough;
        self.shine = shine;
        self
    }

    /// Its fine detail: `grain` of colour, `bump` of tilt, `cracks` and
    /// `streak`.
    #[must_use]
    pub const fn detail(mut self, grain: f32, bump: f32, cracks: f32, streak: f32) -> Self {
        self.grain = grain;
        self.bump = bump;
        self.cracks = cracks;
        self.streak = streak;
        self
    }

    /// Moss or lichen of colour `rgb`, an sRGB triple, over `moss` of it.
    #[must_use]
    pub fn mossy(mut self, moss: f32, rgb: u32) -> Self {
        self.moss = moss;
        self.tint = Self::linear(rgb);
        self
    }

    #[must_use]
    pub const fn wets(mut self, wets: f32) -> Self {
        self.wets = wets;
        self
    }

    #[must_use]
    pub const fn sparkle(mut self, sparkle: f32) -> Self {
        self.sparkle = sparkle;
        self
    }

    /// The kind as a liquid whose colour deepens to `rgb`, an sRGB triple.
    #[must_use]
    pub fn liquid(mut self, rgb: u32) -> Self {
        self.liquid = true;
        self.deep = Self::linear(rgb);
        self
    }

    fn words(self) -> [u32; MATERIAL_WORDS] {
        let mut out = [0; MATERIAL_WORDS];
        let values = [
            self.albedo[0].to_bits(),
            self.albedo[1].to_bits(),
            self.albedo[2].to_bits(),
            self.jitter.to_bits(),
            self.glow.to_bits(),
            self.clear.to_bits(),
            u32::from(self.gas),
            self.strata.to_bits(),
            self.gap.to_bits(),
            self.rough.to_bits(),
            self.shine.to_bits(),
            self.grain.to_bits(),
            self.bump.to_bits(),
            self.cracks.to_bits(),
            self.streak.to_bits(),
            self.moss.to_bits(),
            self.tint[0].to_bits(),
            self.tint[1].to_bits(),
            self.tint[2].to_bits(),
            self.wets.to_bits(),
            self.sparkle.to_bits(),
            u32::from(self.liquid),
            self.deep[0].to_bits(),
            self.deep[1].to_bits(),
            self.deep[2].to_bits(),
        ];
        for (slot, value) in out.iter_mut().zip(values) {
            *slot = value;
        }
        out
    }
}

/// The words of a [`Material`]: where each field lies.
pub mod material {
    pub const ALBEDO: usize = 0;
    pub const JITTER: usize = 3;
    pub const GLOW: usize = 4;
    pub const CLEAR: usize = 5;
    pub const GAS: usize = 6;
    pub const STRATA: usize = 7;
    pub const GAP: usize = 8;
    pub const ROUGH: usize = 9;
    pub const SHINE: usize = 10;
    pub const GRAIN: usize = 11;
    pub const BUMP: usize = 12;
    pub const CRACKS: usize = 13;
    pub const STREAK: usize = 14;
    pub const MOSS: usize = 15;
    pub const TINT: usize = 16;
    pub const WETS: usize = 19;
    pub const SPARKLE: usize = 20;
    pub const LIQUID: usize = 21;
    pub const DEEP: usize = 22;
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

    fn mask(&self, keep: impl Fn(&Material) -> bool) -> u32 {
        (0_u32..)
            .zip(self.0.iter())
            .filter(|(_, material)| keep(material))
            .fold(0, |mask, (kind, _)| mask | (1 << kind))
    }

    /// The kinds a ray stops at, a bit each: not empty and not gas.
    #[must_use]
    pub fn visible(&self) -> u32 {
        self.mask(|material| material.clear < 1.0 && !material.gas)
    }

    /// The kinds nobody sees through, a bit each.
    #[must_use]
    pub fn opaque(&self) -> u32 {
        self.mask(|material| material.clear <= 0.0 && !material.gas)
    }

    /// The kinds drawn as gas, a bit each.
    #[must_use]
    pub fn gases(&self) -> u32 {
        self.mask(|material| material.gas)
    }

    /// The kinds a brick counts as full for: seen, or gas.
    #[must_use]
    pub fn shown(&self) -> u32 {
        self.visible() | self.gases()
    }

    /// The kinds that glow, a bit each.
    #[must_use]
    pub fn glowing(&self) -> u32 {
        self.mask(|material| material.glow > 0.0)
    }

    /// The kinds drawn as a liquid, a bit each: seen, and liquid.
    #[must_use]
    pub fn liquids(&self) -> u32 {
        self.mask(|material| material.liquid) & self.visible()
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

/// The light of a frame.
///
/// - `toward_sun`, `sun`: the way to the sun and its colour;
/// - `zenith`, `horizon`: the sky's colour overhead and at the horizon;
/// - `ground`: the light thrown up from the ground;
/// - `floor`: the colour of the floor under the world;
/// - `haze`: how thick the haze is a cell;
/// - `exposure`;
/// - `occlusion`: how much each neighbour darkens a corner;
/// - `edge`: how dark a cell's edge gets;
/// - `focal`: the screen's focal length in pixels;
/// - `lamp`: how strongly the light of glowing cells lights what is near;
/// - `bloom`, `threshold`: how much light past `threshold` spreads around
///   a bright pixel;
/// - `glass`: how bright the glass frame around the world is;
/// - `blend`: the share of a new frame in the light kept over frames;
/// - `trail`: how much of the glow behind a moving glowing cell is left a
///   frame later;
/// - `smoke`: how much thicker gas is drawn than one cell of it takes
///   light, for rays from the eye and toward the sun alike;
/// - `contact`: how dark the floor is right next to the world;
/// - `glow`: how bright the sky is around the sun near the horizon.
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
    pub lamp: f32,
    pub bloom: f32,
    pub threshold: f32,
    pub glass: f32,
    pub blend: f32,
    pub trail: f32,
    pub smoke: f32,
    pub contact: f32,
    pub glow: f32,
}

/// The words of a [`Light`].
pub const LIGHT_WORDS: usize = 33;

/// Where each field of a [`Light`] lies in its words.
pub mod light {
    pub const TOWARD: usize = 0;
    pub const SUN: usize = 3;
    pub const ZENITH: usize = 6;
    pub const HORIZON: usize = 9;
    pub const GROUND: usize = 12;
    pub const FLOOR: usize = 15;
    pub const HAZE: usize = 18;
    pub const EXPOSURE: usize = 19;
    pub const OCCLUSION: usize = 20;
    pub const EDGE: usize = 21;
    pub const FOCAL: usize = 22;
    pub const LAMP: usize = 23;
    pub const BLOOM: usize = 24;
    pub const THRESHOLD: usize = 25;
    pub const GLASS: usize = 26;
    pub const BLEND: usize = 27;
    pub const TRAIL: usize = 28;
    pub const SMOKE: usize = 29;
    pub const CONTACT: usize = 30;
    pub const GLOW: usize = 31;
}

fn mixed(a: [f32; 3], b: [f32; 3], by: f32) -> [f32; 3] {
    [0, 1, 2].map(|axis| of(a, axis) + (of(b, axis) - of(a, axis)) * by)
}

impl Light {
    /// A low warm sun under a soft studio sky, for a screen `high` pixels
    /// tall seen through a camera of view `view`.
    #[must_use]
    pub fn evening(high: u32, view: f32) -> Self {
        Self::at(high, view, 0.0)
    }

    /// The light at `dusk`: 0 is a warm late afternoon, 1 is the dusk after
    /// sunset, with the sun near the horizon and the sky dim.
    #[must_use]
    pub fn at(high: u32, view: f32, dusk: f32) -> Self {
        let dusk = dusk.clamp(0.0, 1.0);
        let rise = 0.42 - 0.36 * dusk;
        let toward_sun = normal([0.62, -rise, -0.66]);
        let day = 1.0 - dusk;
        let sun = mixed([2.7, 2.15, 1.5], [0.9, 0.38, 0.16], dusk);
        Self {
            toward_sun,
            sun,
            zenith: mixed([0.33, 0.31, 0.29], [0.035, 0.04, 0.07], dusk),
            horizon: mixed([0.52, 0.45, 0.38], [0.16, 0.09, 0.07], dusk),
            ground: mixed([0.26, 0.22, 0.17], [0.05, 0.035, 0.03], dusk),
            floor: [0.30, 0.27, 0.24],
            haze: 0.00012,
            exposure: 0.85 + 0.5 * dusk,
            occlusion: 0.22,
            edge: 0.22,
            focal: high as f32 * 0.5 / (view * 0.5).tan(),
            lamp: 1.0,
            bloom: 0.16,
            threshold: 1.0 * day + 0.5 * dusk,
            glass: 1.0,
            blend: 0.12,
            trail: 0.72,
            smoke: 2.0,
            contact: 0.6,
            glow: 0.55 * day + 0.3 * dusk,
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
            self.lamp,
            self.bloom,
            self.threshold,
            self.glass,
            self.blend,
            self.trail,
            self.smoke,
            self.contact,
            self.glow,
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

    /// [`FLOOR_RAYS`] whole-number directions toward the sun, one from the
    /// middle of each cell of a square grid across the way to the sun,
    /// `spread` from its middle to each side, in an order that spreads
    /// neighbours apart.
    #[must_use]
    pub fn floor_rays(&self, spread: f32) -> Vec<[i32; 3]> {
        let toward = normal(self.toward_sun);
        let side = normal([toward[2], 0.0, -toward[0]]);
        let other = normal([
            toward[1] * side[2] - toward[2] * side[1],
            toward[2] * side[0] - toward[0] * side[2],
            toward[0] * side[1] - toward[1] * side[0],
        ]);
        let scale = (STEEPEST / 2) as f32;
        let side_count = (FLOOR_RAYS as f32).sqrt() as usize;
        (0..FLOOR_RAYS)
            .map(|at| {
                let cell = (at * 7) % FLOOR_RAYS;
                let a = ((cell % side_count) as f32 + 0.5) / side_count as f32 * 2.0 - 1.0;
                let b = ((cell / side_count) as f32 + 0.5) / side_count as f32 * 2.0 - 1.0;
                let way = normal([0, 1, 2].map(|axis| {
                    of(toward, axis) + spread * (a * of(side, axis) + b * of(other, axis))
                }));
                way.map(|value| (value * scale).round() as i32)
            })
            .collect()
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

/// What a frame tells the passes beyond the camera, the table and the
/// light: the sun rays of the faces, how widely the floor's sun rays
/// spread, the camera of the frame before, the [`flags`], and a count
/// raised every frame drawn.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Frame {
    pub suns: Vec<[i32; 3]>,
    pub floor: f32,
    pub before: [u32; 12],
    pub flags: u32,
    pub drawn: u32,
}

/// The scene block of `PIXELS`: the rays, the kinds seen, opaque, gas and
/// glowing, the sun rays, the frame's flags and count, the camera before,
/// and the light.
#[must_use]
pub fn scene_words(rays: &Rays, table: &Table, suns: &[[i32; 3]], light: &Light) -> Vec<u32> {
    let frame = Frame {
        suns: suns.to_vec(),
        floor: 0.05,
        flags: flags::RESET,
        ..Frame::default()
    };
    framed_words(rays, table, &frame, light)
}

/// [`scene_words`] with everything a [`Frame`] holds.
#[must_use]
pub fn framed_words(rays: &Rays, table: &Table, frame: &Frame, light: &Light) -> Vec<u32> {
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
    put(scene::SHOWN, table.shown());
    put(scene::GAS, table.gases());
    put(scene::GLOWING, table.glowing());
    put(scene::LIQUID, table.liquids());
    put(scene::FLAGS, frame.flags);
    put(scene::DRAWN, frame.drawn);
    for (at, sun) in frame.suns.iter().take(SUNS).enumerate() {
        for axis in 0..3 {
            put(scene::SUN_RAYS + 3 * at + axis, of(*sun, axis) as u32);
        }
    }
    for (at, ray) in light.floor_rays(frame.floor).into_iter().enumerate() {
        for axis in 0..3 {
            put(scene::FLOOR_RAYS + 3 * at + axis, of(ray, axis) as u32);
        }
    }
    for (at, value) in frame.before.into_iter().enumerate() {
        put(scene::BEFORE + at, value);
    }
    for (at, value) in light.words().into_iter().enumerate() {
        put(scene::LIGHT + at, value);
    }
    out
}

/// What the passes need to know when they are built: the grid, how its
/// words map to kinds, the screen, the subgroup width, and how many sun
/// rays a corner traces. With one sun ray, the light is the plain one of a
/// low quality: it leaves out the fine detail of surfaces, their gloss,
/// glints and caustics, and the rays and foam of a liquid's top.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Build {
    pub grid: Grid,
    pub kinds: Kinds,
    pub screen: Screen,
    pub subgroup: u32,
    pub suns: u32,
}

/// Which of the optional stages a frame runs: the full rebuild of every
/// brick, the spread of the light of glowing cells, and the sun through
/// the gas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Steps {
    pub refill: bool,
    pub lamp: bool,
    pub sunlit: bool,
}

impl Steps {
    pub const ALL: Self = Self {
        refill: true,
        lamp: true,
        sunlit: true,
    };

    /// Every stage but the full rebuild.
    pub const KEPT: Self = Self {
        refill: false,
        lamp: true,
        sunlit: true,
    };
}

/// The stages after the engine's, in the order [`Build::stages`] gives
/// them.
pub mod stage {
    pub const FILLED: usize = 0;
    pub const LIT_ACROSS: usize = 1;
    pub const LIT_DOWN: usize = 2;
    pub const LIT_DEEP: usize = 3;
    pub const SUNLIT: usize = 4;
    pub const PRIMARY: usize = 5;
    pub const BOUNCE: usize = 6;
    pub const SHADE: usize = 7;
    pub const RESOLVE: usize = 8;
    pub const BLOOM_DOWN: usize = 9;
    pub const BLOOM_ACROSS: usize = 10;
    pub const BLOOM_UP: usize = 11;
    pub const COMPOSITE: usize = 12;
    pub const COUNT: usize = 13;
    pub const NAMES: [&str; COUNT] = [
        "filled",
        "lit_across",
        "lit_down",
        "lit_deep",
        "sunlit",
        "primary",
        "bounce",
        "shade",
        "resolve",
        "bloom_down",
        "bloom_across",
        "bloom_up",
        "composite",
    ];
}

impl Build {
    #[must_use]
    pub const fn layout(&self) -> Layout {
        Layout::new(self.grid, self.screen, self.subgroup)
    }

    /// Every pass: [`occupied`], then one a stage in [`stage`] order.
    pub fn passes(&self) -> Result<Vec<Vec<u32>>, LaneError> {
        Ok(vec![
            occupied(self)?,
            filled(self)?,
            lit_across(self)?,
            lit_down(self)?,
            lit_deep(self)?,
            sunlit(self)?,
            primary(self)?,
            bounce(self)?,
            shade(self)?,
            resolve(self)?,
            bloom_down(self)?,
            bloom_across(self)?,
            bloom_up(self)?,
            composite(self)?,
        ])
    }

    /// The groups of a pass over every brick.
    #[must_use]
    pub fn brick_groups(&self) -> Groups {
        Groups::Fixed(
            u32::try_from((bricks(self.grid) as usize).div_ceil(self.subgroup.max(1) as usize))
                .unwrap_or(0),
        )
    }

    /// The groups of the stages after the engine's, in [`stage`] order,
    /// running the optional ones `steps` asks for.
    #[must_use]
    pub fn stages(&self, steps: Steps) -> Vec<Vec<Groups>> {
        let bricks = self.brick_groups();
        let none = Groups::Fixed(0);
        let pixels = self.screen.groups(self.subgroup);
        let quarter = self.screen.quarter().groups(self.subgroup);
        let lamp = if steps.lamp { bricks } else { none };
        vec![
            vec![if steps.refill { bricks } else { none }],
            vec![lamp],
            vec![lamp],
            vec![lamp],
            vec![if steps.sunlit { bricks } else { none }],
            vec![pixels],
            vec![pixels],
            vec![pixels],
            vec![pixels],
            vec![quarter],
            vec![quarter],
            vec![quarter],
            vec![pixels],
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
