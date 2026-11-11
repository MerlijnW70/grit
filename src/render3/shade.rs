//! The march of every pixel's ray, and the light on what it finds.
//!
//! The light is written once over one set of operations, so the CPU and
//! the device run the same formulas in the same order; floats may still
//! differ in their last bits from one to the other. What the light reads of the
//! world, the cells, the bricks and the shadow rays, each side finds on
//! its own: the device through its emitted march, the CPU through
//! [`World`].

use simdr::kernel::{Kernel, Shape};
use simdr::lanes::LaneError;
use simdr::module::Id;

use super::detail;
use super::emit::{Em, Opened};
use super::march::{Marching, Ray, Reader, World, limit, sides};
use super::{
    BOUNCE_WORDS, BOUNCED, BRICK_BITS, Build, FLOOR_RAYS, FOG_WORDS, GAS_WORDS, GASSY, HDR_WORDS,
    HIT_WORDS, Layout, MATERIAL_WORDS, MISS, Rays, SPARE_WORDS, Screen, UNIT, brick_sides, flags,
    light, material, of, put, refuse, scene, word,
};
use crate::layout::{BINDINGS, PARAMS, PIXELS, STATE};
use crate::sand::Grid;
use crate::view3::Kinds;

const SUN_DISC: f32 = 0.9997;
const SUN_DISC_EDGE: f32 = 0.9993;
const SUN_DISC_GAIN: f32 = 9.0;
const SUN_HALO: f32 = 48.0;
const SUN_HALO_GAIN: f32 = 0.35;
const HORIZON_GLOW: f32 = 3.0;
pub(crate) const FAR: f32 = 1.0e30;
const SKY_DISTANCE: f32 = 1.0e5;
const FLOOR_REACH: f32 = 900.0;
const FLOOR_HAZE: f32 = 2.2;
const GAS_SUN: f32 = 0.85;
const GAS_SKY: f32 = 0.45;
const GAS_FAINT: f32 = 0.003;
const GAS_SHADE: f32 = 0.3;
const GAS_SCATTER: f32 = 0.5;
const GAS_SCATTER_POWER: f32 = 0.25;
const GAS_LOCAL: f32 = 2.0;
const GAS_LOCAL_STEPS: u32 = 2;
const BILLOW_SCALE: f32 = 0.1;
const BILLOW_FINE: f32 = 0.28;
const BILLOW_FINE_WEIGHT: f32 = 0.35;
const BILLOW_RISE: f32 = 0.5;
const COVER_GAIN: f32 = 5.0;
const COVER_FROM: f32 = 0.1;
const COVER_OVER: f32 = 0.55;
const COVER_LEAST: f32 = 1.0e-3;
const COVER_MOST: f32 = 3.0;
const EMBER_BELOW: f32 = 3.0;
const PHASE_SHARE: f32 = 0.8;
const HUE_WARM: [f32; 3] = [1.0, 0.3, -1.0];
const HUE_PATCH: f32 = 0.65;
const HUE_GROVE: f32 = 0.45;
const LINE_WIDE: f32 = 1.4;
const FLOOR_MARGIN: f32 = 0.05;
const PANE: f32 = 0.025;
const BAND_PIXELS: f32 = 1.5;
const BAND_FROM: f32 = 6.0;
const BAND_OVER: f32 = 6.0;
const ACTIVE_DIM: f32 = 0.2;
const ACTIVE_KEPT: f32 = 0.45;
const DRAWN_OUT: f32 = 0.85;
const BROAD_SCALE: f32 = 0.23;
const MIDDLE_SCALE: f32 = 1.7;
const FINE_SCALE: f32 = 4.3;
const GRAIN_MIDDLE: f32 = 1.2;
const GRAIN_FINE: f32 = 0.9;
const CRACK_FROM: f32 = 0.82;
const CRACK_OVER: f32 = 0.14;
const CRACK_DARK: f32 = 0.8;
const TILT_MIDDLE: f32 = 0.45;
const TILT_FINE: f32 = 0.55;
const NEAR_WATER: f32 = 5.0;
const MOSS_AWAY_FROM: f32 = 0.25;
const MOSS_BROAD: f32 = 1.3;
const MOSS_AWAY: f32 = 0.6;
const MOSS_NEAR: f32 = 0.7;
const MOSS_FROM: f32 = 0.95;
const MOSS_OVER: f32 = 0.35;
const WET_DARK: f32 = 0.55;
const ROUGH_SPREAD: f32 = 0.6;
const WET_ROUGH: f32 = 0.12;
const WET_SHINE: f32 = 0.03;
const SPARKLE_SCALE: f32 = 6.5;
const SPARKLE_TURN: f32 = 0.25;
const SPARKLE_FROM: f32 = 0.86;
const SPARKLE_OVER: f32 = 0.06;
const SPARKLE_GAIN: f32 = 6.0;
const CAUSTIC_SCALE: f32 = 0.55;
const CAUSTIC_PACE: f32 = 0.5;
const CAUSTIC_OFFSET: f32 = 5.3;
const CAUSTIC_GAIN: f32 = 6.0;
const CAUSTIC_FADE: f32 = 0.18;
const RIPPLE_WIDE: f32 = 0.42;
const RIPPLE_WIDE_PACE: f32 = 0.55;
const RIPPLE_FINE: f32 = 1.1;
const RIPPLE_FINE_PACE: f32 = 0.9;
const RIPPLE_FINE_WEIGHT: f32 = 0.5;
const SHORE_FROM: f32 = 0.05;
const SHORE_OVER: f32 = 1.2;
const FOAM_SCALE: f32 = 1.6;
const FOAM_PACE: f32 = 0.6;
const FOAM_LIFT: f32 = 0.35;
const FOAM_FROM: f32 = 0.45;
const FOAM_OVER: f32 = 0.25;
const SPRAY_FOAM: f32 = 2.5;
const SPRAY_FROM: f32 = 0.3;
const SPRAY_OVER: f32 = 0.3;
const FALL_SCALE: f32 = 1.3;
const FALL_STRETCH: f32 = 0.11;
const FALL_PACE: f32 = 9.0;
const FALL_FROM: f32 = 0.35;
const FALL_OVER: f32 = 0.35;
const FALL_BASE: f32 = 0.72;
const FALL_MOST: f32 = 0.9;
const FOAM_WRAP: f32 = 0.35;
const FOAM_MOST: f32 = 0.92;
const FOAM_WHITE: f32 = 0.85;
const FOAM_SKY: f32 = 0.55;
const SPRAY_HAZE: f32 = 1.5;
const SPRAY_MOST: f32 = 0.35;
const SPRAY_SUN: f32 = 0.5;
const DETAIL_FROM: f32 = 2.5;
const DETAIL_OVER: f32 = 2.5;
const GLOSSY_BELOW: f32 = 0.7;
const WATER_SUN: f32 = 0.45;
const WATER_SKY: f32 = 0.55;

/// The operations the light is written in.
pub(crate) trait Shader {
    type W: Copy;
    type R: Copy;
    type T: Copy;
    type G: Copy;

    fn u(&mut self, value: u32) -> Result<Self::W, LaneError>;
    fn f(&mut self, value: f32) -> Result<Self::R, LaneError>;
    fn add(&mut self, a: Self::W, b: Self::W) -> Result<Self::W, LaneError>;
    fn sub(&mut self, a: Self::W, b: Self::W) -> Result<Self::W, LaneError>;
    fn mul(&mut self, a: Self::W, b: Self::W) -> Result<Self::W, LaneError>;
    /// The whole quotient of two words; `b` is never 0 where it is used.
    fn div(&mut self, a: Self::W, b: Self::W) -> Result<Self::W, LaneError>;
    fn and(&mut self, a: Self::W, b: u32) -> Result<Self::W, LaneError>;
    fn shr(&mut self, a: Self::W, by: u32) -> Result<Self::W, LaneError>;
    fn shl(&mut self, a: Self::W, by: u32) -> Result<Self::W, LaneError>;
    fn or(&mut self, a: Self::W, b: Self::W) -> Result<Self::W, LaneError>;
    fn xor(&mut self, a: Self::W, b: Self::W) -> Result<Self::W, LaneError>;
    fn eq(&mut self, a: Self::W, b: Self::W) -> Result<Self::T, LaneError>;
    fn lt(&mut self, a: Self::W, b: Self::W) -> Result<Self::T, LaneError>;
    fn bit(&mut self, mask: Self::W, at: Self::W) -> Result<Self::T, LaneError>;
    fn both(&mut self, a: Self::T, b: Self::T) -> Result<Self::T, LaneError>;
    fn not(&mut self, a: Self::T) -> Result<Self::T, LaneError>;
    fn pick(&mut self, when: Self::T, yes: Self::W, no: Self::W) -> Result<Self::W, LaneError>;
    fn pick_r(&mut self, when: Self::T, yes: Self::R, no: Self::R) -> Result<Self::R, LaneError>;
    fn fadd(&mut self, a: Self::R, b: Self::R) -> Result<Self::R, LaneError>;
    fn fsub(&mut self, a: Self::R, b: Self::R) -> Result<Self::R, LaneError>;
    fn fmul(&mut self, a: Self::R, b: Self::R) -> Result<Self::R, LaneError>;
    fn fdiv(&mut self, a: Self::R, b: Self::R) -> Result<Self::R, LaneError>;
    fn fmin(&mut self, a: Self::R, b: Self::R) -> Result<Self::R, LaneError>;
    fn fmax(&mut self, a: Self::R, b: Self::R) -> Result<Self::R, LaneError>;
    fn clamp01(&mut self, a: Self::R) -> Result<Self::R, LaneError>;
    fn exp(&mut self, a: Self::R) -> Result<Self::R, LaneError>;
    fn pow(&mut self, a: Self::R, b: Self::R) -> Result<Self::R, LaneError>;
    fn sqrt(&mut self, a: Self::R) -> Result<Self::R, LaneError>;
    fn log(&mut self, a: Self::R) -> Result<Self::R, LaneError>;
    fn floor(&mut self, a: Self::R) -> Result<Self::R, LaneError>;
    fn fabs(&mut self, a: Self::R) -> Result<Self::R, LaneError>;
    fn fgt(&mut self, a: Self::R, b: Self::R) -> Result<Self::T, LaneError>;
    fn real(&mut self, a: Self::W) -> Result<Self::R, LaneError>;
    fn signed(&mut self, a: Self::W) -> Result<Self::R, LaneError>;
    fn whole(&mut self, a: Self::R) -> Result<Self::W, LaneError>;
    fn whole_signed(&mut self, a: Self::R) -> Result<Self::W, LaneError>;
    fn bits(&mut self, a: Self::W) -> Result<Self::R, LaneError>;
    /// The bits of a float, as a word.
    fn word_of(&mut self, a: Self::R) -> Result<Self::W, LaneError>;
    /// A word of `STATE`.
    fn cell(&mut self, at: Self::W) -> Result<Self::W, LaneError>;
    /// A word of `PIXELS` at a fixed place.
    fn fixed(&mut self, at: u32) -> Result<Self::W, LaneError>;
    /// A word of `PIXELS`.
    fn pixel(&mut self, at: Self::W) -> Result<Self::W, LaneError>;
    /// The engine's frame: the first word of `PARAMS`.
    fn frame(&mut self) -> Result<Self::W, LaneError>;
    /// Whether a ray from `origin` along `direction` meets a cell of a kind
    /// in `opaque`; nothing is traced where `going` is false.
    fn blocked(
        &mut self,
        opaque: Self::W,
        origin: [Self::W; 3],
        direction: [Self::W; 3],
        going: Self::T,
    ) -> Result<Self::T, LaneError>;
    /// The first cell a ray from `origin` along `direction` meets of a kind
    /// in `mask`, and the face it came in through; [`MISS`] and 0 when it
    /// meets none or where `going` is false.
    fn found(
        &mut self,
        mask: Self::W,
        origin: [Self::W; 3],
        direction: [Self::W; 3],
        going: Self::T,
    ) -> Result<(Self::W, Self::W), LaneError>;
    /// Opens a stretch of work only the pixels where `when` holds need.
    fn open(&mut self, when: Self::T) -> Result<Self::G, LaneError>;
    /// Closes the stretch `gate` opened: `given` where it held, else
    /// `otherwise`, made before it opened. Nothing made in the stretch is
    /// used after it but through what this gives.
    fn close(
        &mut self,
        gate: Self::G,
        given: &[Self::R],
        otherwise: &[Self::R],
    ) -> Result<Vec<Self::R>, LaneError>;
    /// [`Shader::close`] for words.
    fn close_words(
        &mut self,
        gate: Self::G,
        given: &[Self::W],
        otherwise: &[Self::W],
    ) -> Result<Vec<Self::W>, LaneError>;
}

/// Whether `a` or `b` holds.
pub(crate) fn either<S: Shader>(s: &mut S, a: S::T, b: S::T) -> Result<S::T, LaneError> {
    let not_a = s.not(a)?;
    let not_b = s.not(b)?;
    let neither = s.both(not_a, not_b)?;
    s.not(neither)
}

pub(crate) type V<R> = [R; 3];

/// What the light needs to know when it is written.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Look {
    pub grid: Grid,
    pub kinds: Kinds,
    pub screen: Screen,
    pub layout: Layout,
    pub suns: u32,
}

impl Look {
    /// Whether the light works out the fine detail of surfaces, the gloss,
    /// glints and caustics, and the rays and foam of a liquid's top: at
    /// more than one sun ray a corner.
    pub(crate) const fn rich(&self) -> bool {
        self.suns > 1
    }
}

/// A truth that never holds.
fn never<S: Shader>(s: &mut S) -> Result<S::T, LaneError> {
    let zero = s.f(0.0)?;
    s.fgt(zero, zero)
}

/// `when` where the light is [`Look::rich`], else never.
fn if_rich<S: Shader>(s: &mut S, look: &Look, when: S::T) -> Result<S::T, LaneError> {
    if look.rich() { Ok(when) } else { never(s) }
}

pub(crate) struct Lit<S: Shader> {
    pub toward: V<S::R>,
    pub sun: V<S::R>,
    pub zenith: V<S::R>,
    pub horizon: V<S::R>,
    pub ground: V<S::R>,
    pub floor: V<S::R>,
    pub haze: S::R,
    pub exposure: S::R,
    pub occlusion: S::R,
    pub edge: S::R,
    pub focal: S::R,
    pub lamp: S::R,
    pub bloom: S::R,
    pub threshold: S::R,
    pub glass: S::R,
    pub blend: S::R,
    pub trail: S::R,
    pub contact: S::R,
    pub glow: S::R,
}

pub(crate) struct Eye<S: Shader> {
    pub origin: V<S::R>,
    pub direction: V<S::R>,
    pub length: S::R,
    pub unit: V<S::R>,
}

pub(crate) fn v_map<S: Shader>(
    s: &mut S,
    a: V<S::R>,
    mut each: impl FnMut(&mut S, S::R) -> Result<S::R, LaneError>,
) -> Result<V<S::R>, LaneError> {
    Ok([each(s, a[0])?, each(s, a[1])?, each(s, a[2])?])
}

pub(crate) fn v_zip<S: Shader>(
    s: &mut S,
    a: V<S::R>,
    b: V<S::R>,
    mut each: impl FnMut(&mut S, S::R, S::R) -> Result<S::R, LaneError>,
) -> Result<V<S::R>, LaneError> {
    Ok([
        each(s, a[0], b[0])?,
        each(s, a[1], b[1])?,
        each(s, a[2], b[2])?,
    ])
}

pub(crate) fn dot<S: Shader>(s: &mut S, a: V<S::R>, b: V<S::R>) -> Result<S::R, LaneError> {
    let x = s.fmul(a[0], b[0])?;
    let y = s.fmul(a[1], b[1])?;
    let z = s.fmul(a[2], b[2])?;
    let xy = s.fadd(x, y)?;
    s.fadd(xy, z)
}

pub(crate) fn scaled<S: Shader>(s: &mut S, a: V<S::R>, by: S::R) -> Result<V<S::R>, LaneError> {
    v_map(s, a, |s, value| s.fmul(value, by))
}

pub(crate) fn mix<S: Shader>(
    s: &mut S,
    a: V<S::R>,
    b: V<S::R>,
    by: S::R,
) -> Result<V<S::R>, LaneError> {
    v_zip(s, a, b, |s, a, b| {
        let gap = s.fsub(b, a)?;
        let part = s.fmul(gap, by)?;
        s.fadd(a, part)
    })
}

pub(crate) fn lerp<S: Shader>(s: &mut S, a: S::R, b: S::R, by: S::R) -> Result<S::R, LaneError> {
    let gap = s.fsub(b, a)?;
    let part = s.fmul(gap, by)?;
    s.fadd(a, part)
}

pub(crate) fn pick_v<S: Shader>(
    s: &mut S,
    when: S::T,
    yes: V<S::R>,
    no: V<S::R>,
) -> Result<V<S::R>, LaneError> {
    Ok([
        s.pick_r(when, yes[0], no[0])?,
        s.pick_r(when, yes[1], no[1])?,
        s.pick_r(when, yes[2], no[2])?,
    ])
}

/// `(value - from) / over`, kept between 0 and 1, smoothed at both ends.
pub(crate) fn smooth<S: Shader>(
    s: &mut S,
    value: S::R,
    from: f32,
    over: f32,
) -> Result<S::R, LaneError> {
    let from = s.f(from)?;
    let over = s.f(1.0 / over)?;
    let gone = s.fsub(value, from)?;
    let part = s.fmul(gone, over)?;
    let t = s.clamp01(part)?;
    let three = s.f(3.0)?;
    let two = s.f(2.0)?;
    let twice = s.fmul(t, two)?;
    let rest = s.fsub(three, twice)?;
    let square = s.fmul(t, t)?;
    s.fmul(square, rest)
}

pub(crate) fn real_at<S: Shader>(s: &mut S, at: u32) -> Result<S::R, LaneError> {
    let held = s.fixed(at)?;
    s.bits(held)
}

pub(crate) fn reals_at<S: Shader>(s: &mut S, at: u32) -> Result<V<S::R>, LaneError> {
    Ok([real_at(s, at)?, real_at(s, at + 1)?, real_at(s, at + 2)?])
}

/// The float at word `at` of `PIXELS`, given as a word.
pub(crate) fn real_of<S: Shader>(s: &mut S, at: S::W) -> Result<S::R, LaneError> {
    let held = s.pixel(at)?;
    s.bits(held)
}

/// The word `at + by`.
pub(crate) fn plus<S: Shader>(s: &mut S, at: S::W, by: u32) -> Result<S::W, LaneError> {
    let by = s.u(by)?;
    s.add(at, by)
}

pub(crate) fn lit<S: Shader>(s: &mut S, look: &Look) -> Result<Lit<S>, LaneError> {
    let at = word(look.layout.scene() + scene::LIGHT);
    let one = |s: &mut S, field: usize| real_at(s, at + word(field));
    let three = |s: &mut S, field: usize| reals_at(s, at + word(field));
    Ok(Lit {
        toward: three(s, light::TOWARD)?,
        sun: three(s, light::SUN)?,
        zenith: three(s, light::ZENITH)?,
        horizon: three(s, light::HORIZON)?,
        ground: three(s, light::GROUND)?,
        floor: three(s, light::FLOOR)?,
        haze: one(s, light::HAZE)?,
        exposure: one(s, light::EXPOSURE)?,
        occlusion: one(s, light::OCCLUSION)?,
        edge: one(s, light::EDGE)?,
        focal: one(s, light::FOCAL)?,
        lamp: one(s, light::LAMP)?,
        bloom: one(s, light::BLOOM)?,
        threshold: one(s, light::THRESHOLD)?,
        glass: one(s, light::GLASS)?,
        blend: one(s, light::BLEND)?,
        trail: one(s, light::TRAIL)?,
        contact: one(s, light::CONTACT)?,
        glow: one(s, light::GLOW)?,
    })
}

/// The direction of pixel `(x, y)`, as two's complement words.
pub(crate) fn direction_words<S: Shader>(
    s: &mut S,
    look: &Look,
    (x, y): (S::W, S::W),
) -> Result<V<S::W>, LaneError> {
    let at = word(look.layout.scene());
    let one = s.u(1)?;
    let wide = s.u(look.screen.wide)?;
    let high = s.u(look.screen.high)?;
    let twice = s.add(x, x)?;
    let across = s.add(twice, one)?;
    let across = s.sub(across, wide)?;
    let twice = s.add(y, y)?;
    let down = s.sub(high, twice)?;
    let down = s.sub(down, one)?;
    let mut out = [one; 3];
    for axis in 0..3 {
        let forward = s.fixed(at + word(scene::FORWARD + axis))?;
        let right = s.fixed(at + word(scene::RIGHT + axis))?;
        let up = s.fixed(at + word(scene::UP + axis))?;
        let right = s.mul(across, right)?;
        let up = s.mul(down, up)?;
        let sum = s.add(forward, right)?;
        let held = s.add(sum, up)?;
        put(&mut out, axis, held);
    }
    Ok(out)
}

pub(crate) fn eye<S: Shader>(
    s: &mut S,
    look: &Look,
    at: (S::W, S::W),
) -> Result<Eye<S>, LaneError> {
    let base = word(look.layout.scene() + scene::ORIGIN);
    let sixteenth = s.f(1.0 / UNIT as f32)?;
    let mut origin = [sixteenth; 3];
    for (axis, slot) in origin.iter_mut().enumerate() {
        let held = s.fixed(base + word(axis))?;
        let held = s.signed(held)?;
        *slot = s.fmul(held, sixteenth)?;
    }
    let words = direction_words(s, look, at)?;
    let direction = [
        s.signed(words[0])?,
        s.signed(words[1])?,
        s.signed(words[2])?,
    ];
    let squared = dot(s, direction, direction)?;
    let length = s.sqrt(squared)?;
    let unit = v_map(s, direction, |s, value| s.fdiv(value, length))?;
    Ok(Eye {
        origin,
        direction,
        length,
        unit,
    })
}

/// The sky seen along the unit vector `way`: a smooth blend from a warm
/// glow at the horizon, brightest toward the sun, to a softer colour
/// overhead, with the sun's halo, and its disc when `disc`.
pub(crate) fn sky<S: Shader>(
    s: &mut S,
    lit: &Lit<S>,
    way: V<S::R>,
    disc: bool,
) -> Result<V<S::R>, LaneError> {
    let none = s.f(0.0)?;
    let one = s.f(1.0)?;
    let up = s.fsub(none, way[1])?;
    let high = s.clamp01(up)?;
    let curve = s.f(0.6)?;
    let lifted = s.fmax(high, none)?;
    let height = s.pow(lifted, curve)?;
    let colour = mix(s, lit.horizon, lit.zenith, height)?;
    let facing = dot(s, way, lit.toward)?;
    let toward = s.fmax(facing, none)?;
    let sunward = s.fadd(toward, one)?;
    let half = s.f(0.5)?;
    let sunward = s.fmul(sunward, half)?;
    let sunward = s.fmul(sunward, sunward)?;
    let fall = s.f(-HORIZON_GLOW)?;
    let above = s.fabs(up)?;
    let fall = s.fmul(above, fall)?;
    let near = s.exp(fall)?;
    let glow = s.fmul(near, sunward)?;
    let glow = s.fmul(glow, lit.glow)?;
    let halo = s.f(SUN_HALO)?;
    let halo = s.pow(toward, halo)?;
    let halo_gain = s.f(SUN_HALO_GAIN)?;
    let halo = s.fmul(halo, halo_gain)?;
    let mut light = s.fadd(glow, halo)?;
    if disc {
        let shown = smooth(s, facing, SUN_DISC_EDGE, SUN_DISC - SUN_DISC_EDGE)?;
        let gain = s.f(SUN_DISC_GAIN)?;
        let shown = s.fmul(shown, gain)?;
        light = s.fadd(light, shown)?;
    }
    let sun = scaled(s, lit.sun, light)?;
    v_zip(s, colour, sun, S::fadd)
}

/// A word hashed to spread its bits.
pub(crate) fn hash<S: Shader>(s: &mut S, value: S::W) -> Result<S::W, LaneError> {
    let mut x = value;
    for (shift, factor) in [(16, 0x7FEB_352D), (15, 0x846C_A68B)] {
        let moved = s.shr(x, shift)?;
        x = s.xor(x, moved)?;
        let factor = s.u(factor)?;
        x = s.mul(x, factor)?;
    }
    let moved = s.shr(x, 16)?;
    s.xor(x, moved)
}

/// What a ray that meets no cell sees, and how far it goes: the sky, or a
/// warm floor under the grid, lit by the sun and shadowed by the world
/// along one of the floor's sun rays, the one `pick` chooses, darkened
/// next to the world, and fading into the sky with distance.
fn backdrop<S: Shader>(
    s: &mut S,
    look: &Look,
    lit: &Lit<S>,
    eye: &Eye<S>,
    pick: S::W,
) -> Result<(V<S::R>, S::R), LaneError> {
    let grid = look.grid;
    let above = sky(s, lit, eye.unit, true)?;
    let zero = s.f(0.0)?;
    let one = s.f(1.0)?;
    let down = s.fgt(eye.direction[1], zero)?;
    let floor = s.f(grid.deep as f32)?;
    let gap = s.fsub(floor, eye.origin[1])?;
    let far = s.fdiv(gap, eye.direction[1])?;
    let reach = scaled(s, eye.direction, far)?;
    let point = v_zip(s, eye.origin, reach, S::fadd)?;
    let unit = s.f(UNIT as f32)?;
    let mut near = down;
    let mut origin = [s.u(grid.deep * UNIT - 1)?; 3];
    let mut outside = [zero; 2];
    for (at, axis) in [0, 2].into_iter().enumerate() {
        let side = of(sides(grid), axis) as f32;
        let low = s.f(side - FLOOR_REACH)?;
        let high = s.f(FLOOR_REACH)?;
        let over = s.fgt(of(point, axis), low)?;
        let under = s.fgt(high, of(point, axis))?;
        near = s.both(near, over)?;
        near = s.both(near, under)?;
        let sixteenths = s.fmul(of(point, axis), unit)?;
        let held = s.whole_signed(sixteenths)?;
        put(&mut origin, axis, held);
        let before = s.fsub(zero, of(point, axis))?;
        let end = s.f(side)?;
        let after = s.fsub(of(point, axis), end)?;
        let past = s.fmax(before, after)?;
        let past = s.fmax(past, zero)?;
        if let Some(slot) = outside.get_mut(at) {
            *slot = past;
        }
    }
    let zero_word = s.u(0)?;
    for slot in &mut origin {
        *slot = s.pick(near, *slot, zero_word)?;
    }
    let facing = s.fsub(zero, lit.toward[1])?;
    let facing = s.fmax(facing, zero)?;
    let sunny = s.fgt(facing, zero)?;
    let going = s.both(near, sunny)?;
    let opaque = s.fixed(word(look.layout.scene() + scene::OPAQUE))?;
    let rays_count = 2 * look.suns.max(1);
    let share = s.f(1.0 / rays_count as f32)?;
    let mut seen = zero;
    let base = s.u(word(look.layout.scene() + scene::FLOOR_RAYS))?;
    for at_ray in 0..rays_count {
        let step = s.u(at_ray * (FLOOR_RAYS as u32 / rays_count).max(1))?;
        let which = s.add(pick, step)?;
        let which = s.and(which, FLOOR_RAYS as u32 - 1)?;
        let three = s.u(3)?;
        let at = s.mul(which, three)?;
        let at = s.add(base, at)?;
        let mut rays = [at; 3];
        for (axis, slot) in rays.iter_mut().enumerate() {
            let here = plus(s, at, word(axis))?;
            *slot = s.pixel(here)?;
        }
        let blocked = s.blocked(opaque, origin, rays, going)?;
        let open = s.pick_r(blocked, zero, share)?;
        seen = s.fadd(seen, open)?;
    }

    let [dx, dz] = outside;
    let dx = s.fmul(dx, dx)?;
    let dz = s.fmul(dz, dz)?;
    let apart = s.fadd(dx, dz)?;
    let apart = s.sqrt(apart)?;
    let size = s.f(grid.wide.max(grid.tall) as f32)?;
    let tight = s.f(0.03)?;
    let tight = s.fmul(size, tight)?;
    let wide = s.f(0.16)?;
    let wide = s.fmul(size, wide)?;
    let into = s.fdiv(apart, tight)?;
    let into = s.fsub(zero, into)?;
    let close = s.exp(into)?;
    let broad = s.fdiv(apart, wide)?;
    let broad = s.fsub(zero, broad)?;
    let broad = s.exp(broad)?;
    let quarter = s.f(0.45)?;
    let broad = s.fmul(broad, quarter)?;
    let dark = s.fadd(close, broad)?;
    let dark = s.fmul(dark, lit.contact)?;
    let held = s.fsub(one, dark)?;
    let held = s.fmax(held, zero)?;

    let sky_amount = s.f(0.55)?;
    let soft = s.fmul(seen, held)?;
    let mut colour = [zero; 3];
    for (channel, slot) in colour.iter_mut().enumerate() {
        let sun = s.fmul(of(lit.sun, channel), facing)?;
        let sun = s.fmul(sun, soft)?;
        let fill = s.fadd(of(lit.zenith, channel), of(lit.horizon, channel))?;
        let fill = s.fmul(fill, sky_amount)?;
        let fill = s.fmul(fill, held)?;
        let light = s.fadd(sun, fill)?;
        *slot = s.fmul(of(lit.floor, channel), light)?;
    }
    let distance = s.fmul(far, eye.length)?;
    let thick = s.f(FLOOR_HAZE)?;
    let thick = s.fmul(lit.haze, thick)?;
    let thickness = s.fmul(distance, thick)?;
    let thickness = s.fsub(zero, thickness)?;
    let left = s.exp(thickness)?;
    let fog = s.fsub(one, left)?;
    let level = [eye.unit[0], zero, eye.unit[2]];
    let flat = dot(s, level, level)?;
    let flat = s.sqrt(flat)?;
    let tiny = s.f(1.0e-6)?;
    let flat = s.fmax(flat, tiny)?;
    let level = v_map(s, level, |s, value| s.fdiv(value, flat))?;
    let mist = sky(s, lit, level, false)?;
    let hazed = mix(s, colour, mist, fog)?;
    let colour = pick_v(s, down, hazed, above)?;
    let endless = s.f(SKY_DISTANCE)?;
    let distance = s.pick_r(down, distance, endless)?;
    Ok((colour, distance))
}

/// The cell's point on each axis, from its index.
pub(crate) fn point<S: Shader>(s: &mut S, grid: Grid, cell: S::W) -> Result<V<S::W>, LaneError> {
    let wide = grid.wide.trailing_zeros();
    let deep = grid.deep.trailing_zeros();
    let x = s.and(cell, grid.wide - 1)?;
    let y = s.shr(cell, wide)?;
    let y = s.and(y, grid.deep - 1)?;
    let z = s.shr(cell, wide + deep)?;
    let z = s.and(z, grid.tall - 1)?;
    Ok([x, y, z])
}

/// The axis of a face (3 for [`super::INSIDE`]) and whether it faces up its
/// axis.
fn face_parts<S: Shader>(s: &mut S, info: S::W) -> Result<(S::W, S::T), LaneError> {
    let face = s.and(info, 7)?;
    let axis = s.shr(face, 1)?;
    let odd = s.and(face, 1)?;
    let one = s.u(1)?;
    let up = s.eq(odd, one)?;
    Ok((axis, up))
}

/// How far along the eye's direction it meets the face a hit came in by.
fn entry<S: Shader>(
    s: &mut S,
    grid: Grid,
    eye: &Eye<S>,
    (cell, info): (S::W, S::W),
) -> Result<S::R, LaneError> {
    let at = point(s, grid, cell)?;
    let (axis, up) = face_parts(s, info)?;
    let none = s.f(0.0)?;
    let one = s.f(1.0)?;
    let lift = s.pick_r(up, one, none)?;
    let mut far = none;
    for which in 0..3 {
        let side = s.real(of(at, which))?;
        let plane = s.fadd(side, lift)?;
        let gap = s.fsub(plane, of(eye.origin, which))?;
        let here = s.fdiv(gap, of(eye.direction, which))?;
        let wanted = s.u(which as u32)?;
        let this = s.eq(axis, wanted)?;
        far = s.pick_r(this, here, far)?;
    }
    Ok(far)
}

/// How far along the eye's direction it enters and leaves the glass box:
/// the grid's box from `top` cells down from the grid's top; the entry is
/// 0 for an eye inside it.
fn boxed<S: Shader>(
    s: &mut S,
    grid: Grid,
    eye: &Eye<S>,
    top: S::R,
) -> Result<(S::R, S::R), LaneError> {
    let none = s.f(0.0)?;
    let far = s.f(FAR)?;
    let mut enters = s.f(-FAR)?;
    let mut leaves = far;
    for (axis, side) in sides(grid).into_iter().enumerate() {
        let side = s.f(side as f32)?;
        let floor = if axis == 1 { top } else { none };
        let way = of(eye.direction, axis);
        let tiny = s.f(1.0e-12)?;
        let size = s.fabs(way)?;
        let still = s.fgt(tiny, size)?;
        let one = s.f(1.0)?;
        let way = s.pick_r(still, one, way)?;
        let low = s.fsub(floor, of(eye.origin, axis))?;
        let low = s.fdiv(low, way)?;
        let high = s.fsub(side, of(eye.origin, axis))?;
        let high = s.fdiv(high, way)?;
        let first = s.fmin(low, high)?;
        let last = s.fmax(low, high)?;
        let outside_low = s.fgt(floor, of(eye.origin, axis))?;
        let outside_high = s.fgt(of(eye.origin, axis), side)?;
        let reach = s.f(-FAR)?;
        let without = s.pick_r(outside_low, far, reach)?;
        let without = s.pick_r(outside_high, far, without)?;
        let first = s.pick_r(still, without, first)?;
        let last = s.pick_r(still, far, last)?;
        enters = s.fmax(enters, first)?;
        leaves = s.fmin(leaves, last)?;
    }
    let enters = s.fmax(enters, none)?;
    Ok((enters, leaves))
}

/// How far along the eye's direction it leaves the grid.
fn leaving<S: Shader>(s: &mut S, grid: Grid, eye: &Eye<S>) -> Result<S::R, LaneError> {
    let none = s.f(0.0)?;
    let mut least = s.f(FAR)?;
    for (axis, side) in sides(grid).into_iter().enumerate() {
        let side = s.f(side as f32)?;
        let ahead = s.fgt(of(eye.direction, axis), none)?;
        let target = s.pick_r(ahead, side, none)?;
        let gap = s.fsub(target, of(eye.origin, axis))?;
        let here = s.fdiv(gap, of(eye.direction, axis))?;
        let behind = s.fgt(here, none)?;
        let moving = s.fgt(of(eye.direction, axis), none)?;
        let backwards = s.fgt(none, of(eye.direction, axis))?;
        let moving = s.not(moving)?;
        let still = s.not(backwards)?;
        let still = s.both(moving, still)?;
        let counted = s.not(still)?;
        let counted = s.both(counted, behind)?;
        let far = s.f(FAR)?;
        let here = s.pick_r(counted, here, far)?;
        least = s.fmin(least, here)?;
    }
    Ok(least)
}

pub(crate) fn kind_of<S: Shader>(s: &mut S, kinds: Kinds, held: S::W) -> Result<S::W, LaneError> {
    let moved = s.shr(held, kinds.shift())?;
    s.and(moved, kinds.mask())
}

pub(crate) struct Stuff<S: Shader> {
    pub albedo: V<S::R>,
    pub jitter: S::R,
    pub glow: S::R,
    pub clear: S::R,
    pub strata: S::R,
    pub gap: S::R,
}

pub(crate) fn stuff<S: Shader>(s: &mut S, look: &Look, kind: S::W) -> Result<Stuff<S>, LaneError> {
    let base = s.u(word(look.layout.table()))?;
    let size = s.u(word(MATERIAL_WORDS))?;
    let offset = s.mul(kind, size)?;
    let at = s.add(base, offset)?;
    let get = |s: &mut S, field: usize| -> Result<S::R, LaneError> {
        let here = plus(s, at, word(field))?;
        real_of(s, here)
    };
    Ok(Stuff {
        albedo: [
            get(s, material::ALBEDO)?,
            get(s, material::ALBEDO + 1)?,
            get(s, material::ALBEDO + 2)?,
        ],
        jitter: get(s, material::JITTER)?,
        glow: get(s, material::GLOW)?,
        clear: get(s, material::CLEAR)?,
        strata: get(s, material::STRATA)?,
        gap: get(s, material::GAP)?,
    })
}

/// `values` laid out on the grid's axes, given which axis a face is on:
/// the first goes along the face's axis, the second and third along the
/// next axes round from it.
fn on_axes<S: Shader>(
    s: &mut S,
    axis: S::W,
    [along, first, second]: V<S::W>,
) -> Result<V<S::W>, LaneError> {
    let x_axis = s.u(0)?;
    let y_axis = s.u(1)?;
    let on_x = s.eq(axis, x_axis)?;
    let on_y = s.eq(axis, y_axis)?;
    let x = s.pick(on_y, second, first)?;
    let x = s.pick(on_x, along, x)?;
    let y = s.pick(on_y, along, second)?;
    let y = s.pick(on_x, first, y)?;
    let z = s.pick(on_y, first, along)?;
    let z = s.pick(on_x, second, z)?;
    Ok([x, y, z])
}

fn from_axes<S: Shader>(s: &mut S, axis: S::W, values: V<S::R>) -> Result<[S::R; 2], LaneError> {
    let x_axis = s.u(0)?;
    let y_axis = s.u(1)?;
    let on_x = s.eq(axis, x_axis)?;
    let on_y = s.eq(axis, y_axis)?;
    let first = s.pick_r(on_y, values[2], values[0])?;
    let first = s.pick_r(on_x, values[1], first)?;
    let second = s.pick_r(on_y, values[0], values[1])?;
    let second = s.pick_r(on_x, values[2], second)?;
    Ok([first, second])
}

/// A cell next to the face, given by its offsets along the face's axis and
/// the next two: whether it is in the grid, and its index.
fn neighbour<S: Shader>(
    s: &mut S,
    grid: Grid,
    at: V<S::W>,
    axis: S::W,
    offsets: V<S::W>,
) -> Result<(S::T, S::W), LaneError> {
    let moved = on_axes(s, axis, offsets)?;
    let mut point = at;
    let mut inside = None;
    for (which, side) in sides(grid).into_iter().enumerate() {
        let held = s.add(of(at, which), of(moved, which))?;
        put(&mut point, which, held);
        let side = s.u(side)?;
        let held = s.lt(of(point, which), side)?;
        inside = Some(match inside {
            None => held,
            Some(before) => s.both(before, held)?,
        });
    }
    let inside = inside.ok_or(LaneError::BadCarry {
        given: 0,
        wanted: 3,
    })?;
    let wide = grid.wide.trailing_zeros();
    let deep = grid.deep.trailing_zeros();
    let z = s.shl(point[2], wide + deep)?;
    let y = s.shl(point[1], wide)?;
    let index = s.or(z, y)?;
    let index = s.or(index, point[0])?;
    let none = s.u(0)?;
    let index = s.pick(inside, index, none)?;
    Ok((inside, index))
}

/// A volume of `channels` floats a brick, starting at word `base`, read at
/// `point` in cells, blended between the eight bricks around it.
pub(crate) fn sampled<S: Shader>(
    s: &mut S,
    grid: Grid,
    base: u32,
    channels: u32,
    point: V<S::R>,
) -> Result<Vec<S::R>, LaneError> {
    sampled_from(s, grid, (base, channels), channels, point)
}

/// [`sampled`] of the first `channels` floats of a volume of `stride` floats
/// a brick.
pub(crate) fn sampled_from<S: Shader>(
    s: &mut S,
    grid: Grid,
    (base, stride): (u32, u32),
    channels: u32,
    point: V<S::R>,
) -> Result<Vec<S::R>, LaneError> {
    let sides = brick_sides(grid);
    let zero = s.f(0.0)?;
    let one = s.f(1.0)?;
    let quarter = s.f(1.0 / (1 << BRICK_BITS) as f32)?;
    let half = s.f(0.5)?;
    let mut low = [s.u(0)?; 3];
    let mut high = low;
    let mut part = [zero; 3];
    for axis in 0..3 {
        let side = of(sides, axis);
        let at = s.fmul(of(point, axis), quarter)?;
        let at = s.fsub(at, half)?;
        let below = s.floor(at)?;
        let rest = s.fsub(at, below)?;
        let last = s.f((side - 1) as f32)?;
        let first = s.fmax(below, zero)?;
        let first = s.fmin(first, last)?;
        let next = s.fadd(below, one)?;
        let next = s.fmax(next, zero)?;
        let next = s.fmin(next, last)?;
        let held = s.whole(first)?;
        put(&mut low, axis, held);
        let held = s.whole(next)?;
        put(&mut high, axis, held);
        let held = s.clamp01(rest)?;
        put(&mut part, axis, held);
    }
    let across = of(sides, 0).trailing_zeros();
    let down = of(sides, 1).trailing_zeros();
    let mut out = vec![zero; channels as usize];
    for corner in 0..8_usize {
        let pick = |axis: usize| (corner >> axis) & 1 == 1;
        let x = if pick(0) { high[0] } else { low[0] };
        let y = if pick(1) { high[1] } else { low[1] };
        let z = if pick(2) { high[2] } else { low[2] };
        let mut weight = one;
        for axis in 0..3 {
            let share = if pick(axis) {
                of(part, axis)
            } else {
                s.fsub(one, of(part, axis))?
            };
            weight = s.fmul(weight, share)?;
        }
        let z = s.shl(z, across + down)?;
        let y = s.shl(y, across)?;
        let index = s.or(z, y)?;
        let index = s.or(index, x)?;
        let count = s.u(stride)?;
        let index = s.mul(index, count)?;
        let start = s.u(base)?;
        let index = s.add(index, start)?;
        for (channel, slot) in (0_u32..).zip(out.iter_mut()) {
            let here = plus(s, index, channel)?;
            let value = real_of(s, here)?;
            let value = s.fmul(value, weight)?;
            *slot = s.fadd(*slot, value)?;
        }
    }
    Ok(out)
}

/// How much a gas scatters the sun toward an eye looking along the
/// whole-number `direction`: a Henyey-Greenstein lobe of the light's
/// `phase`, blended with even scattering, 1 on average over all ways.
pub(crate) fn gas_phase<S: Shader>(
    s: &mut S,
    look: &Look,
    direction: V<S::W>,
) -> Result<S::R, LaneError> {
    let at_light = word(look.layout.scene() + scene::LIGHT);
    let toward = reals_at(s, at_light + word(light::TOWARD))?;
    let g = real_at(s, at_light + word(light::PHASE))?;
    let way = [
        s.signed(direction[0])?,
        s.signed(direction[1])?,
        s.signed(direction[2])?,
    ];
    let way = detail::unit(s, way)?;
    let cosine = dot(s, way, toward)?;
    let one = s.f(1.0)?;
    let two = s.f(2.0)?;
    let square = s.fmul(g, g)?;
    let top = s.fsub(one, square)?;
    let bottom = s.fadd(one, square)?;
    let lean = s.fmul(two, g)?;
    let lean = s.fmul(lean, cosine)?;
    let bottom = s.fsub(bottom, lean)?;
    let tiny = s.f(1.0e-4)?;
    let bottom = s.fmax(bottom, tiny)?;
    let root = s.sqrt(bottom)?;
    let bottom = s.fmul(bottom, root)?;
    let lobe = s.fdiv(top, bottom)?;
    let share = s.f(PHASE_SHARE)?;
    lerp(s, one, lobe, share)
}

/// How much of the gas the smooth volume holds at `middle` a cell there
/// keeps, as a share of it, when the light is [`Look::rich`]: the volume's
/// cover there, broken by a rising noise, so the edge of the gas billows
/// and its inside stays whole.
fn billowed<S: Shader>(
    s: &mut S,
    look: &Look,
    middle: V<S::R>,
    taken: S::R,
) -> Result<S::R, LaneError> {
    let at_light = word(look.layout.scene() + scene::LIGHT);
    let billow = real_at(s, at_light + word(light::BILLOW))?;
    let clock = detail::clock(s, look)?;
    let rise = s.f(BILLOW_RISE)?;
    let risen = s.fmul(clock, rise)?;
    let lifted = s.fadd(middle[1], risen)?;
    let at = [middle[0], lifted, middle[2]];
    let half = s.f(0.5)?;
    let mut total = s.f(0.0)?;
    for (scale, weight, seed) in [
        (BILLOW_SCALE, 1.0 - BILLOW_FINE_WEIGHT, 0xB001),
        (BILLOW_FINE, BILLOW_FINE_WEIGHT, 0xB002),
    ] {
        let scale = s.f(scale)?;
        let scaled_at = scaled(s, at, scale)?;
        let found = detail::noise(s, scaled_at, seed)?;
        let weight = s.f(weight)?;
        let part = s.fmul(found.value, weight)?;
        total = s.fadd(total, part)?;
    }
    let gain = s.f(COVER_GAIN)?;
    let cover = s.fmul(taken, gain)?;
    let cover = s.clamp01(cover)?;
    let wobble = s.fsub(total, half)?;
    let wobble = s.fmul(wobble, billow)?;
    let broken = s.fadd(cover, wobble)?;
    let kept = smooth(s, broken, COVER_FROM, COVER_OVER)?;
    let least = s.f(COVER_LEAST)?;
    let cover = s.fmax(cover, least)?;
    let ratio = s.fdiv(kept, cover)?;
    let most = s.f(COVER_MOST)?;
    s.fmin(ratio, most)
}

/// The light a ray gathers through cell `cell` of a brick that holds gas,
/// and how much of the light behind it still shows: `fog` is the light so
/// far, red, green and blue, then the share that still shows. The gas is
/// read between the bricks around the cell's middle, so its edges are soft;
/// with the rich light its edge billows. The sun reaches it through the gas
/// toward the sun, near it a step or two at a time and beyond that as
/// [`super::sunlit`] found it a brick at a time, scattered by `phase`; the
/// sky reaches its inside less than its outside, and the glow of burning
/// cells lights it from below. A ray that no longer shows anything behind
/// it does no more work.
pub(crate) fn gas_step<S: Shader>(
    s: &mut S,
    look: &Look,
    cell: S::W,
    fog: [S::R; 4],
    phase: S::R,
) -> Result<[S::R; 4], LaneError> {
    let layout = look.layout;
    let grid = look.grid;
    let [red, green, blue, left] = fog;
    let zero = s.f(0.0)?;
    let one = s.f(1.0)?;
    let faint = s.f(GAS_FAINT)?;
    let showing = s.fgt(left, faint)?;
    let gate = s.open(showing)?;
    let at_light = word(layout.scene() + scene::LIGHT);
    let sun = reals_at(s, at_light + word(light::SUN))?;
    let toward = reals_at(s, at_light + word(light::TOWARD))?;
    let zenith = reals_at(s, at_light + word(light::ZENITH))?;
    let horizon = reals_at(s, at_light + word(light::HORIZON))?;
    let lamp_gain = real_at(s, at_light + word(light::LAMP))?;
    let ember = real_at(s, at_light + word(light::EMBER))?;
    let gain = real_at(s, at_light + word(light::SMOKE))?;
    let at = point(s, grid, cell)?;
    let half = s.f(0.5)?;
    let mut middle = [half; 3];
    for (axis, slot) in middle.iter_mut().enumerate() {
        let whole = s.real(of(at, axis))?;
        *slot = s.fadd(whole, half)?;
    }
    let thick = sampled(s, grid, word(layout.gas()), word(GAS_WORDS), middle)?;
    let through = sampled(s, grid, word(layout.sunlit()), 1, middle)?;
    let through = through.first().copied().unwrap_or(one);
    let below = s.f(EMBER_BELOW)?;
    let under = s.fadd(middle[1], below)?;
    let lamp = sampled(
        s,
        grid,
        word(layout.lamp(2)),
        3,
        [middle[0], under, middle[2]],
    )?;
    let taken = thick.first().copied().unwrap_or(zero);
    let ratio = if look.rich() {
        billowed(s, look, middle, taken)?
    } else {
        one
    };
    let near = if look.rich() {
        let mut gathered = zero;
        for step in 1..=GAS_LOCAL_STEPS {
            let far = s.f(GAS_LOCAL * step as f32)?;
            let ahead = scaled(s, toward, far)?;
            let ahead = v_zip(s, middle, ahead, S::fadd)?;
            let share = sampled_from(s, grid, (word(layout.gas()), word(GAS_WORDS)), 1, ahead)?;
            gathered = s.fadd(gathered, share.first().copied().unwrap_or(zero))?;
        }
        let length = s.f(GAS_LOCAL)?;
        let lost = s.fmul(gathered, length)?;
        let lost = s.fmul(lost, gain)?;
        let lost = s.fsub(zero, lost)?;
        s.exp(lost)?
    } else {
        one
    };
    let through = s.fmul(through, near)?;
    let taken = s.fmul(taken, ratio)?;
    let taken = s.fmul(taken, gain)?;
    let taken = s.clamp01(taken)?;
    let sun_gain = s.f(GAS_SUN)?;
    let sky_gain = s.f(GAS_SKY)?;
    let scatter_power = s.f(GAS_SCATTER_POWER)?;
    let scattered = s.pow(through, scatter_power)?;
    let scatter_gain = s.f(GAS_SCATTER)?;
    let scattered = s.fmul(scattered, scatter_gain)?;
    let lit_through = s.fadd(through, scattered)?;
    let sunned = s.fmul(lit_through, sun_gain)?;
    let sunned = s.fmul(sunned, phase)?;
    let open_sky = s.sqrt(through)?;
    let shade = s.f(GAS_SHADE)?;
    let open_sky = lerp(s, shade, one, open_sky)?;
    let sky_gain = s.fmul(sky_gain, open_sky)?;
    let glow_gain = s.fmul(lamp_gain, ember)?;
    let mut out = [red, green, blue];
    for (channel, slot) in out.iter_mut().enumerate() {
        let near = lamp.get(channel).copied().unwrap_or(zero);
        let near = s.fmul(near, glow_gain)?;
        let direct = s.fmul(of(sun, channel), sunned)?;
        let fill = s.fadd(of(zenith, channel), of(horizon, channel))?;
        let fill = s.fmul(fill, sky_gain)?;
        let light = s.fadd(direct, fill)?;
        let light = s.fadd(light, near)?;
        let colour = thick.get(channel + 1).copied().unwrap_or(zero);
        let colour = s.fmul(colour, ratio)?;
        let colour = s.fmul(colour, gain)?;
        let gathered = s.fmul(colour, light)?;
        let gathered = s.fmul(gathered, left)?;
        *slot = s.fadd(*slot, gathered)?;
    }
    let kept = s.fsub(one, taken)?;
    let kept_left = s.fmul(left, kept)?;
    let closed = s.close(
        gate,
        &[out[0], out[1], out[2], kept_left],
        &[red, green, blue, left],
    )?;
    let take = |at: usize| closed.get(at).copied().unwrap_or(zero);
    Ok([take(0), take(1), take(2), take(3)])
}

pub(crate) struct Surface<S: Shader> {
    pub colour: V<S::R>,
    pub glow: V<S::R>,
    /// The share of the sun that reaches it.
    pub shadow: S::R,
}

/// Where a surface lies under a liquid: whether it does, and the height of
/// the liquid's top, in cells, as y counts down.
pub(crate) struct Sea<S: Shader> {
    pub under: S::T,
    pub level: S::R,
}

#[allow(clippy::too_many_lines)]
fn surface<S: Shader>(
    s: &mut S,
    look: &Look,
    lit: &Lit<S>,
    eye: &Eye<S>,
    (cell, info): (S::W, S::W),
    sea: &Sea<S>,
) -> Result<Surface<S>, LaneError> {
    let grid = look.grid;
    let opaque = s.fixed(word(look.layout.scene() + scene::OPAQUE))?;
    let held = s.cell(cell)?;
    let kind = kind_of(s, look.kinds, held)?;
    let material = stuff(s, look, kind)?;
    let at = point(s, grid, cell)?;
    let (axis, up) = face_parts(s, info)?;
    let three = s.u(3)?;
    let inside_face = s.eq(axis, three)?;
    let y_axis = s.u(1)?;
    let axis = s.pick(inside_face, y_axis, axis)?;
    let none_word = s.u(0)?;
    let one_word = s.u(1)?;
    let back_word = s.u(u32::MAX)?;
    let toward = s.pick(up, one_word, back_word)?;

    let zero = s.f(0.0)?;
    let one = s.f(1.0)?;
    let minus = s.f(-1.0)?;
    let sign = s.pick_r(up, one, minus)?;
    let mut normal = [zero; 3];
    for (which, slot) in normal.iter_mut().enumerate() {
        let wanted = s.u(which as u32)?;
        let this = s.eq(axis, wanted)?;
        *slot = s.pick_r(this, sign, zero)?;
    }

    let far = entry(s, grid, eye, (cell, info))?;
    let far = s.pick_r(inside_face, zero, far)?;
    let reach = scaled(s, eye.direction, far)?;
    let hit = v_zip(s, eye.origin, reach, S::fadd)?;
    let corner = [s.real(at[0])?, s.real(at[1])?, s.real(at[2])?];
    let inner = v_zip(s, hit, corner, S::fsub)?;
    let [u, v] = from_axes(s, axis, inner)?;
    let u = s.clamp01(u)?;
    let v = s.clamp01(v)?;

    let mut shut = [[zero; 3]; 3];
    let mut warmth = [zero; 3];
    let side_weight = s.f(0.6)?;
    let corner_weight = s.f(0.35)?;
    for (i, row) in shut.iter_mut().enumerate() {
        for (j, slot) in row.iter_mut().enumerate() {
            if i == 1 && j == 1 {
                continue;
            }
            let across = of([back_word, none_word, one_word], i);
            let along = of([back_word, none_word, one_word], j);
            let (inside, index) = neighbour(s, grid, at, axis, [toward, across, along])?;
            let near = s.cell(index)?;
            let near = kind_of(s, look.kinds, near)?;
            let solid = s.bit(opaque, near)?;
            let solid = s.both(inside, solid)?;
            *slot = s.pick_r(solid, one, zero)?;
            let other = stuff(s, look, near)?;
            let weight = if i == 1 || j == 1 {
                side_weight
            } else {
                corner_weight
            };
            let glow = s.fmul(other.glow, weight)?;
            let glow = s.pick_r(inside, glow, zero)?;
            let light = scaled(s, other.albedo, glow)?;
            warmth = v_zip(s, warmth, light, S::fadd)?;
        }
    }

    let facing = dot(s, normal, lit.toward)?;
    let facing = s.fmax(facing, zero)?;
    let lit_side = s.fgt(facing, zero)?;
    let suns = look.suns.max(1);
    let share = s.f(1.0 / suns as f32)?;
    let sixteen = s.u(UNIT)?;
    let mut sun_rays = Vec::with_capacity(suns as usize);
    for at_sun in 0..suns {
        let base = word(look.layout.scene() + scene::SUN_RAYS) + 3 * at_sun;
        sun_rays.push([s.fixed(base)?, s.fixed(base + 1)?, s.fixed(base + 2)?]);
    }
    let front = on_axes(s, axis, [toward, none_word, none_word])?;
    let mut ahead = at;
    for which in 0..3 {
        let held = s.add(of(at, which), of(front, which))?;
        put(&mut ahead, which, held);
        let held = s.mul(of(ahead, which), sixteen)?;
        put(&mut ahead, which, held);
    }
    let near_wall = s.u(1)?;
    let far_wall = s.u(UNIT - 1)?;
    let normal_inset = s.pick(up, near_wall, far_wall)?;
    let mut corners_ao = [zero; 4];
    let mut corners_sun = [zero; 4];
    for (corner, (ao_slot, sun_slot)) in corners_ao.iter_mut().zip(&mut corners_sun).enumerate() {
        let (i, j) = (corner & 1, corner >> 1);
        let side_a = of(of(shut, 2 * i), 1);
        let side_b = of(of(shut, 1), 2 * j);
        let diagonal = of(of(shut, 2 * i), 2 * j);
        let count = s.fadd(side_a, side_b)?;
        let both = s.fmul(side_a, side_b)?;
        let pinned = s.fgt(both, zero)?;
        let three_r = s.f(3.0)?;
        let count = s.fadd(count, diagonal)?;
        let count = s.pick_r(pinned, three_r, count)?;
        let dark = s.fmul(count, lit.occlusion)?;
        *ao_slot = s.fsub(one, dark)?;

        let first = if i == 1 { far_wall } else { near_wall };
        let second = if j == 1 { far_wall } else { near_wall };
        let inset = on_axes(s, axis, [normal_inset, first, second])?;
        let origin = [
            s.add(ahead[0], inset[0])?,
            s.add(ahead[1], inset[1])?,
            s.add(ahead[2], inset[2])?,
        ];
        let mut seen = zero;
        for rays in &sun_rays {
            let blocked = s.blocked(opaque, origin, *rays, lit_side)?;
            let open = s.pick_r(blocked, zero, share)?;
            seen = s.fadd(seen, open)?;
        }
        *sun_slot = seen;
    }
    let [ao_00, ao_10, ao_01, ao_11] = corners_ao;
    let ao_low = lerp(s, ao_00, ao_10, u)?;
    let ao_high = lerp(s, ao_01, ao_11, u)?;
    let occlusion = lerp(s, ao_low, ao_high, v)?;
    let [sun_00, sun_10, sun_01, sun_11] = corners_sun;
    let sun_low = lerp(s, sun_00, sun_10, u)?;
    let sun_high = lerp(s, sun_01, sun_11, u)?;
    let shadow = lerp(s, sun_low, sun_high, v)?;

    let half = s.f(0.5)?;
    let lifted = scaled(s, normal, half)?;
    let front_point = v_zip(s, hit, lifted, S::fadd)?;
    let lamp = sampled(s, grid, word(look.layout.lamp(2)), 3, front_point)?;
    let gases = s.fixed(word(look.layout.scene() + scene::GAS))?;
    let no_gas = s.eq(gases, none_word)?;
    let some_gas = s.not(no_gas)?;
    let sunward = s.both(lit_side, some_gas)?;
    let through = {
        let gate = s.open(sunward)?;
        let quarter = s.f(1.0 / (1 << BRICK_BITS) as f32)?;
        let centre = scaled(s, front_point, quarter)?;
        let value = super::volume::sun_through(s, look, centre, 0.5)?;
        s.close(gate, &[value], &[one])?
    };
    let through = through.first().copied().unwrap_or(one);
    let shadow = s.fmul(shadow, through)?;

    let mixed = hash(s, cell)?;
    let mut jitters = [zero; 4];
    let byte = s.f(1.0 / 255.0)?;
    for (at_byte, slot) in jitters.iter_mut().enumerate() {
        let part = s.shr(mixed, 8 * at_byte as u32)?;
        let part = s.and(part, 0xFF)?;
        let part = s.real(part)?;
        let part = s.fmul(part, byte)?;
        *slot = s.fsub(part, half)?;
    }
    let column = s.shr(at[0], 3)?;
    let row = s.shr(at[2], 3)?;
    let row = s.shl(row, 12)?;
    let column = s.or(column, row)?;
    let wobble = hash(s, column)?;
    let wobble = s.and(wobble, 3)?;
    let layer = s.add(at[1], wobble)?;
    let layer = s.shr(layer, 1)?;
    let layer = s.shl(layer, 4)?;
    let layer = s.or(layer, kind)?;
    let band = hash(s, layer)?;
    let band = s.and(band, 0xFF)?;
    let band = s.real(band)?;
    let band = s.fmul(band, byte)?;
    let band = s.fsub(band, half)?;
    let band = s.fmul(band, material.strata)?;
    let broad = s.f(1.6)?;
    let fine = s.f(0.4)?;
    let [red, green, blue, overall] = jitters;
    let all = s.fmul(overall, broad)?;
    let table_at = s.u(word(look.layout.table()))?;
    let size_word = s.u(word(MATERIAL_WORDS))?;
    let kind_at = s.mul(kind, size_word)?;
    let kind_at = s.add(table_at, kind_at)?;
    let field = |s: &mut S, at: usize| -> Result<S::R, LaneError> {
        let here = plus(s, kind_at, word(at))?;
        real_of(s, here)
    };
    let top = [
        field(s, material::TOP)?,
        field(s, material::TOP + 1)?,
        field(s, material::TOP + 2)?,
    ];
    let hue = field(s, material::HUE)?;
    let on_y = s.eq(axis, y_axis)?;
    let below = s.not(up)?;
    let looks_up = s.both(on_y, below)?;
    let face_colour = pick_v(s, looks_up, top, material.albedo)?;
    let patch = [s.shr(at[0], 2)?, s.shr(at[1], 2)?, s.shr(at[2], 2)?];
    let patch_y = s.shl(patch[1], 10)?;
    let patch_z = s.shl(patch[2], 20)?;
    let patch = s.or(patch[0], patch_y)?;
    let patch = s.or(patch, patch_z)?;
    let patch = s.xor(patch, kind)?;
    let patch = hash(s, patch)?;
    let patch = s.and(patch, 0xFF)?;
    let patch = s.real(patch)?;
    let patch = s.fmul(patch, byte)?;
    let patch = s.fsub(patch, half)?;
    let own_hue = hash(s, mixed)?;
    let own_hue = s.and(own_hue, 0xFF)?;
    let own_hue = s.real(own_hue)?;
    let own_hue = s.fmul(own_hue, byte)?;
    let own_hue = s.fsub(own_hue, half)?;
    let grove = [s.shr(at[0], 4)?, s.shr(at[1], 4)?, s.shr(at[2], 4)?];
    let grove_y = s.shl(grove[1], 10)?;
    let grove_z = s.shl(grove[2], 20)?;
    let grove = s.or(grove[0], grove_y)?;
    let grove = s.or(grove, grove_z)?;
    let grove_seed = s.u(0x00A5_5A00)?;
    let grove = s.xor(grove, grove_seed)?;
    let grove = hash(s, grove)?;
    let grove = s.and(grove, 0xFF)?;
    let grove = s.real(grove)?;
    let grove = s.fmul(grove, byte)?;
    let grove = s.fsub(grove, half)?;
    let patch_share = s.f(HUE_PATCH)?;
    let warmth_shift = lerp(s, own_hue, patch, patch_share)?;
    let grove_share = s.f(HUE_GROVE)?;
    let warmth_shift = lerp(s, warmth_shift, grove, grove_share)?;
    let two_r = s.f(2.0)?;
    let warmth_shift = s.fmul(warmth_shift, two_r)?;
    let warmth_shift = s.fmul(warmth_shift, hue)?;
    let mut albedo = [zero; 3];
    for (channel, slot) in albedo.iter_mut().enumerate() {
        let own = s.fmul(of([red, green, blue], channel), fine)?;
        let shift = s.fadd(all, own)?;
        let shift = s.fmul(shift, material.jitter)?;
        let factor = s.fadd(one, shift)?;
        let factor = s.fadd(factor, band)?;
        let factor = s.fmax(factor, zero)?;
        let warm = s.f(of(HUE_WARM, channel))?;
        let tinted = s.fmul(warmth_shift, warm)?;
        let tinted = s.fadd(one, tinted)?;
        let tinted = s.fmax(tinted, zero)?;
        let factor = s.fmul(factor, tinted)?;
        *slot = s.fmul(of(face_colour, channel), factor)?;
    }

    let skin = detail::skin(s, look, kind)?;
    let clock = detail::clock(s, look)?;
    let drawn_out = s.f(DRAWN_OUT)?;
    let stretch = s.fmul(skin.streak, drawn_out)?;
    let stretch = s.fsub(one, stretch)?;
    let scale = |s: &mut S, by: f32| -> Result<V<S::R>, LaneError> {
        let across = s.f(by)?;
        let down = s.fmul(across, stretch)?;
        Ok([across, down, across])
    };
    let wide_scale = scale(s, BROAD_SCALE)?;
    let middle_scale = scale(s, MIDDLE_SCALE)?;
    let fine_scale = scale(s, FINE_SCALE)?;
    let broad = if look.rich() {
        detail::noise_at(s, hit, wide_scale, 0x1001)?.value
    } else {
        half
    };
    let seen_far = s.fmul(far, eye.length)?;
    let seen_far = s.fmax(seen_far, one)?;
    let cell_size = s.fdiv(lit.focal, seen_far)?;
    let shown = smooth(s, cell_size, DETAIL_FROM, DETAIL_OVER)?;
    let detailed = s.fgt(shown, zero)?;
    let two = s.f(2.0)?;
    let closed = if look.rich() {
        let gate = s.open(detailed)?;
        let middle = detail::noise_at(s, hit, middle_scale, 0x2002)?;
        let fine = detail::noise_at(s, hit, fine_scale, 0x3003)?;
        let middle_part = s.fsub(middle.value, half)?;
        let fine_part = s.fsub(fine.value, half)?;
        let middle_weight = s.f(GRAIN_MIDDLE)?;
        let fine_weight = s.f(GRAIN_FINE)?;
        let middle_part = s.fmul(middle_part, middle_weight)?;
        let fine_part = s.fmul(fine_part, fine_weight)?;
        let grained = s.fadd(middle_part, fine_part)?;
        let grained = s.fmul(grained, skin.grain)?;
        let grain_factor = s.fadd(one, grained)?;
        let grain_factor = s.fmax(grain_factor, zero)?;
        let ridge = s.fmul(middle.value, two)?;
        let ridge = s.fsub(ridge, one)?;
        let ridge = s.fabs(ridge)?;
        let ridge = s.fsub(one, ridge)?;
        let crack = smooth(s, ridge, CRACK_FROM, CRACK_OVER)?;
        let crack = s.fmul(crack, skin.cracks)?;
        let crack_dark = s.f(CRACK_DARK)?;
        let crack = s.fmul(crack, crack_dark)?;
        let kept = s.fsub(one, crack)?;
        let grain_factor = s.fmul(grain_factor, kept)?;
        let middle_tilt = s.f(TILT_MIDDLE)?;
        let fine_tilt = s.f(TILT_FINE)?;
        let middle_slope = scaled(s, middle.slope, middle_tilt)?;
        let fine_slope = scaled(s, fine.slope, fine_tilt)?;
        let slope = v_zip(s, middle_slope, fine_slope, S::fadd)?;
        let along = dot(s, slope, normal)?;
        let flat = scaled(s, normal, along)?;
        let tangent = v_zip(s, slope, flat, S::fsub)?;
        let tangent = scaled(s, tangent, skin.bump)?;
        let given = [grain_factor, tangent[0], tangent[1], tangent[2], fine_part];
        s.close(gate, &given, &[one, zero, zero, zero, zero])?
    } else {
        vec![one, zero, zero, zero, zero]
    };
    let take = |at: usize| closed.get(at).copied().unwrap_or(zero);
    let grain_factor = lerp(s, one, take(0), shown)?;
    let tangent = scaled(s, [take(1), take(2), take(3)], shown)?;
    let fine_part = s.fmul(take(4), shown)?;
    let bumped = v_zip(s, normal, tangent, S::fsub)?;
    let bumped = detail::unit(s, bumped)?;

    let [liquid_share, _] = if look.rich() {
        detail::water_at(s, look, front_point)?
    } else {
        [zero; 2]
    };
    let near_gain = s.f(NEAR_WATER)?;
    let near = s.fmul(liquid_share, near_gain)?;
    let near = s.clamp01(near)?;
    let wet = s.fmul(near, skin.wets)?;
    let wet = s.clamp01(wet)?;
    let facing_bumped = dot(s, bumped, lit.toward)?;
    let away_from = s.f(MOSS_AWAY_FROM)?;
    let away = s.fsub(away_from, facing_bumped)?;
    let away = s.clamp01(away)?;
    let growth_broad = s.f(MOSS_BROAD)?;
    let growth_away = s.f(MOSS_AWAY)?;
    let growth_near = s.f(MOSS_NEAR)?;
    let growth = s.fmul(broad, growth_broad)?;
    let away_part = s.fmul(away, growth_away)?;
    let near_part = s.fmul(near, growth_near)?;
    let growth = s.fadd(growth, away_part)?;
    let growth = s.fadd(growth, near_part)?;
    let grown = smooth(s, growth, MOSS_FROM, MOSS_OVER)?;
    let grown = s.fmul(grown, skin.moss)?;
    let grown = s.clamp01(grown)?;
    let darken = s.f(WET_DARK)?;
    let darken = s.fmul(wet, darken)?;
    let wet_keep = s.fsub(one, darken)?;
    for (channel, slot) in albedo.iter_mut().enumerate() {
        let own = s.fmul(*slot, grain_factor)?;
        let moss = s.fmul(of(skin.tint, channel), grain_factor)?;
        let own = lerp(s, own, moss, grown)?;
        *slot = s.fmul(own, wet_keep)?;
    }
    let rough_spread = s.f(ROUGH_SPREAD)?;
    let rough_shift = s.fmul(fine_part, rough_spread)?;
    let rough_shift = s.fmul(rough_shift, skin.rough)?;
    let rough = s.fadd(skin.rough, rough_shift)?;
    let wet_rough = s.f(WET_ROUGH)?;
    let rough = lerp(s, rough, wet_rough, wet)?;
    let rough = s.clamp01(rough)?;
    let wet_shine = s.f(WET_SHINE)?;
    let wet_shine = s.fmul(wet, wet_shine)?;
    let shine = s.fmax(skin.shine, wet_shine)?;

    let back = v_map(s, eye.unit, |s, value| s.fsub(zero, value))?;
    let glinted = if look.rich() {
        detail::glint(s, bumped, back, lit.toward, (rough, shine))?
    } else {
        zero
    };
    let glints = s.fgt(skin.sparkle, zero)?;
    let glitter = if look.rich() {
        let gate = s.open(glints)?;
        let sparkle_scale = s.f(SPARKLE_SCALE)?;
        let turn = s.f(SPARKLE_TURN)?;
        let sparkle_at = scaled(s, back, turn)?;
        let sparkle_at = v_zip(s, hit, sparkle_at, S::fadd)?;
        let sparkle_at = scaled(s, sparkle_at, sparkle_scale)?;
        let glitter = detail::noise(s, sparkle_at, 0x5005)?;
        let glitter = smooth(s, glitter.value, SPARKLE_FROM, SPARKLE_OVER)?;
        let glitter = s.fmul(glitter, skin.sparkle)?;
        let sparkle_gain = s.f(SPARKLE_GAIN)?;
        let glitter = s.fmul(glitter, sparkle_gain)?;
        let facing_sun = s.fmax(facing_bumped, zero)?;
        let glitter = s.fmul(glitter, facing_sun)?;
        s.close(gate, &[glitter], &[zero])?
    } else {
        vec![zero]
    };
    let glitter = glitter.first().copied().unwrap_or(zero);
    let specular = s.fadd(glinted, glitter)?;
    let specular = s.fmul(specular, shadow)?;
    let glossy_below = s.f(GLOSSY_BELOW)?;
    let glossy = s.fgt(glossy_below, rough)?;
    let mirrored_sky = if look.rich() {
        let gate = s.open(glossy)?;
        let seen_back = dot(s, back, bumped)?;
        let reflects = detail::schlick(s, seen_back, shine)?;
        let smooth_part = s.fsub(one, rough)?;
        let smooth_part = s.fmul(smooth_part, smooth_part)?;
        let reflects = s.fmul(reflects, smooth_part)?;
        let reflects = s.fmul(reflects, occlusion)?;
        let way = detail::mirrored(s, eye.unit, bumped)?;
        let mirrored_sky = sky(s, lit, way, false)?;
        let mirrored_sky = scaled(s, mirrored_sky, reflects)?;
        s.close(gate, &mirrored_sky, &[zero; 3])?
    } else {
        vec![zero; 3]
    };
    let mirrored_sky = [0, 1, 2].map(|at| mirrored_sky.get(at).copied().unwrap_or(zero));

    let caustic = if look.rich() {
        let gate = s.open(sea.under)?;
        let level_gap = s.fsub(hit[1], sea.level)?;
        let depth = s.fmax(level_gap, zero)?;
        let caustic_scale = s.f(CAUSTIC_SCALE)?;
        let caustic_pace = s.f(CAUSTIC_PACE)?;
        let caustic_x = s.fmul(hit[0], caustic_scale)?;
        let caustic_z = s.fmul(hit[2], caustic_scale)?;
        let caustic_t = s.fmul(clock, caustic_pace)?;
        let first_wave = detail::noise(s, [caustic_x, caustic_t, caustic_z], 0x4004)?;
        let offset = s.f(CAUSTIC_OFFSET)?;
        let second_x = s.fadd(caustic_z, offset)?;
        let second_t = s.fsub(caustic_t, offset)?;
        let second_wave = detail::noise(s, [second_x, second_t, caustic_x], 0x4114)?;
        let mut caustic = one;
        for wave in [first_wave.value, second_wave.value] {
            let ridge = s.fmul(wave, two)?;
            let ridge = s.fsub(ridge, one)?;
            let ridge = s.fabs(ridge)?;
            let ridge = s.fsub(one, ridge)?;
            caustic = s.fmul(caustic, ridge)?;
        }
        let square = s.fmul(caustic, caustic)?;
        let caustic = s.fmul(square, square)?;
        let caustic_gain = s.f(CAUSTIC_GAIN)?;
        let caustic = s.fmul(caustic, caustic_gain)?;
        let fade = s.f(-CAUSTIC_FADE)?;
        let fade = s.fmul(depth, fade)?;
        let fade = s.exp(fade)?;
        let caustic = s.fmul(caustic, fade)?;
        let caustic = s.fmul(caustic, shadow)?;
        s.close(gate, &[caustic], &[zero])?
    } else {
        vec![zero]
    };
    let caustic = caustic.first().copied().unwrap_or(zero);
    let lit_bumped = s.fmax(facing_bumped, zero)?;
    let lit_bumped = s.pick_r(lit_side, lit_bumped, zero)?;

    let distance = s.fmul(far, eye.length)?;
    let distance = s.fmax(distance, one)?;
    let size = s.fdiv(lit.focal, distance)?;
    let u_far = s.fsub(one, u)?;
    let v_far = s.fsub(one, v)?;
    let near_u = s.fmin(u, u_far)?;
    let near_v = s.fmin(v, v_far)?;
    let rim = s.fmin(near_u, near_v)?;
    let pixel_wide = s.f(BAND_PIXELS)?;
    let band = s.fdiv(pixel_wide, size)?;
    let least_band = s.f(0.04)?;
    let band = s.fmax(band, least_band)?;
    let into = s.fdiv(rim, band)?;
    let into = s.clamp01(into)?;
    let outer = s.fsub(one, into)?;
    let fade = smooth(s, size, BAND_FROM, BAND_OVER)?;
    let edge = s.fmul(outer, lit.edge)?;
    let edge = s.fmul(edge, fade)?;
    let edge = s.fsub(one, edge)?;

    let tiny = s.f(1.0e-4)?;
    let gap = s.fmax(material.gap, tiny)?;
    let into_gap = s.fdiv(rim, gap)?;
    let into_gap = s.clamp01(into_gap)?;
    let has_gap = s.fgt(material.gap, zero)?;
    let open = s.pick_r(has_gap, into_gap, one)?;
    let deep = s.f(0.12)?;
    let rest = s.f(0.88)?;
    let open = s.fmul(open, rest)?;
    let open = s.fadd(open, deep)?;
    let kept_light = s.fmul(edge, open)?;

    let up_ness = s.fsub(zero, normal[1])?;
    let sky_part = s.f(0.15)?;
    let sky_base = s.f(0.2)?;
    let sky_amount = s.fmul(up_ness, sky_part)?;
    let sky_amount = s.fadd(sky_base, sky_amount)?;
    let ground_part = s.f(0.25)?;
    let ground_amount = s.fsub(one, up_ness)?;
    let ground_amount = s.fmul(ground_amount, ground_part)?;
    let mut colour = [zero; 3];
    let mut glow = [zero; 3];
    for channel in 0..3 {
        let sun = s.fmul(of(lit.sun, channel), lit_bumped)?;
        let sun = s.fmul(sun, shadow)?;
        let bright = s.fmul(of(lit.sun, channel), caustic)?;
        let sun = s.fadd(sun, bright)?;
        let fill = s.fadd(of(lit.zenith, channel), of(lit.horizon, channel))?;
        let sky = s.fmul(fill, sky_amount)?;
        let bounce = s.fmul(of(lit.ground, channel), ground_amount)?;
        let ambient = s.fadd(sky, bounce)?;
        let ambient = s.fmul(ambient, occlusion)?;
        let light = s.fadd(sun, ambient)?;
        let light = s.fadd(light, of(warmth, channel))?;
        let near = lamp.get(channel).copied().unwrap_or(zero);
        let near = s.fmul(near, lit.lamp)?;
        let light = s.fadd(light, near)?;
        let light = s.fmul(light, kept_light)?;
        let shade = s.fmul(of(albedo, channel), light)?;
        let shone = s.fmul(of(lit.sun, channel), specular)?;
        let shone = s.fadd(shone, of(mirrored_sky, channel))?;
        let shone = s.fmul(shone, kept_light)?;
        let shade = s.fadd(shade, shone)?;
        put(&mut colour, channel, shade);
        let own = s.fmul(of(albedo, channel), material.glow)?;
        let own = s.fmul(own, open)?;
        put(&mut glow, channel, own);
    }
    Ok(Surface {
        colour,
        glow,
        shadow,
    })
}

/// What a pane of glass adds to the light, how much of the light behind it
/// shows, and whether it stands above the floor.
struct Glass<S: Shader> {
    add: V<S::R>,
    keep: S::R,
    standing: S::T,
}

/// The glass around the grid's box, seen at `far` along the eye's
/// direction: what it adds to the light, how much of the light behind it
/// shows, and whether the point is above the floor: the box has no glass
/// under it.
fn pane<S: Shader>(
    s: &mut S,
    look: &Look,
    lit: &Lit<S>,
    eye: &Eye<S>,
    (far, top): (S::R, S::R),
) -> Result<Glass<S>, LaneError> {
    let grid = look.grid;
    let zero = s.f(0.0)?;
    let one = s.f(1.0)?;
    let reach = scaled(s, eye.direction, far)?;
    let point = v_zip(s, eye.origin, reach, S::fadd)?;
    let bottom = s.f(grid.deep as f32)?;
    let above = s.fsub(bottom, point[1])?;
    let margin = s.f(FLOOR_MARGIN)?;
    let standing = s.fgt(above, margin)?;
    let mut apart = [zero; 3];
    for (axis, side) in sides(grid).into_iter().enumerate() {
        let side = s.f(side as f32)?;
        let floor = if axis == 1 { top } else { zero };
        let low = s.fsub(of(point, axis), floor)?;
        let low = s.fabs(low)?;
        let high = s.fsub(side, of(point, axis))?;
        let high = s.fabs(high)?;
        let held = s.fmin(low, high)?;
        put(&mut apart, axis, held);
    }
    let [a, b, c] = apart;
    let least = s.fmin(a, b)?;
    let least = s.fmin(least, c)?;
    let most = s.fmax(a, b)?;
    let most = s.fmax(most, c)?;
    let sum = s.fadd(a, b)?;
    let sum = s.fadd(sum, c)?;
    let middle = s.fsub(sum, least)?;
    let middle = s.fsub(middle, most)?;
    let distance = s.fmul(far, eye.length)?;
    let distance = s.fmax(distance, one)?;
    let size = s.fdiv(lit.focal, distance)?;
    let pixels = s.fmul(middle, size)?;
    let wide = s.f(LINE_WIDE)?;
    let line = s.fsub(wide, pixels)?;
    let line = s.clamp01(line)?;
    let line = s.fmul(line, lit.glass)?;
    let faint = s.f(0.28)?;
    let line = s.fmul(line, faint)?;
    let tint = [s.f(0.78)?, s.f(0.92)?, s.f(0.82)?];
    let shine = s.f(PANE)?;
    let shine = s.fmul(shine, lit.glass)?;
    let mut add = [zero; 3];
    for (channel, slot) in add.iter_mut().enumerate() {
        let reflected = s.fmul(of(lit.horizon, channel), shine)?;
        let drawn = s.fmul(of(tint, channel), line)?;
        *slot = s.fadd(reflected, drawn)?;
    }
    let keep = s.fsub(one, shine)?;
    Ok(Glass {
        add,
        keep,
        standing,
    })
}

/// The light of a pixel before it is kept over frames.
pub(crate) struct Hdr<S: Shader> {
    pub colour: V<S::R>,
    pub glow: V<S::R>,
    pub distance: S::R,
}

/// The light of pixel `(x, y)`, given its four hit words, its four fog
/// words and its four bounced words.
#[allow(clippy::too_many_lines)]
pub(crate) fn seen<S: Shader>(
    s: &mut S,
    look: &Look,
    (x, y): (S::W, S::W),
    hit: [S::W; 4],
    fog: [S::W; 4],
    bounced: [S::W; 4],
) -> Result<Hdr<S>, LaneError> {
    let lit = lit(s, look)?;
    let eye = eye(s, look, (x, y))?;
    let miss = s.u(MISS)?;
    let [first, first_info, behind, behind_info] = hit;
    let sent = detail::sends(s, look, (first, first_info))?;
    let raw_first = first;
    let first_missed = s.eq(first, miss)?;
    let has_first = s.not(first_missed)?;
    let behind_missed = s.eq(behind, miss)?;
    let has_behind = s.not(behind_missed)?;
    let same = s.eq(first, behind)?;
    let apart = s.not(same)?;
    let layered = s.both(has_first, apart)?;
    let nothing = s.u(0)?;
    let first = s.pick(first_missed, nothing, first)?;
    let behind = s.pick(behind_missed, nothing, behind)?;

    let [mirror_cell, mirror_info, bent_cell, bent_info] = bounced;
    let flagged = s.and(mirror_info, BOUNCED)?;
    let unflagged = s.eq(flagged, nothing)?;
    let flagged = s.not(unflagged)?;
    let valid = s.both(sent, flagged)?;
    let valid = if_rich(s, look, valid)?;
    let not_valid = s.not(valid)?;
    let mirror_missed = s.eq(mirror_cell, miss)?;
    let mirror_has = s.not(mirror_missed)?;
    let mirror_cell = s.pick(mirror_missed, nothing, mirror_cell)?;
    let bent_missed = s.eq(bent_cell, miss)?;
    let bent_has = s.not(bent_missed)?;
    let bent_cell = s.pick(bent_missed, nothing, bent_cell)?;
    let zero = s.f(0.0)?;
    let mut fallback = Vec::with_capacity(21);
    for _ in 0..2 {
        fallback.extend(eye.origin);
        fallback.extend(eye.direction);
        fallback.push(eye.length);
        fallback.extend(eye.unit);
    }
    fallback.push(zero);
    let closed = if look.rich() {
        let gate = s.open(valid)?;
        let rays = detail::bounce_rays(s, look, (x, y), (raw_first, first_info), false)?;
        let mut given = Vec::with_capacity(21);
        for made in [rays.mirror, rays.bent] {
            let seen_eye = detail::eye_of(s, made)?;
            given.extend(seen_eye.origin);
            given.extend(seen_eye.direction);
            given.push(seen_eye.length);
            given.extend(seen_eye.unit);
        }
        let sixteenth = s.f(1.0 / UNIT as f32)?;
        let level = s.signed(of(rays.met, 1))?;
        given.push(s.fmul(level, sixteenth)?);
        s.close(gate, &given, &fallback)?
    } else {
        fallback
    };
    let take = |at: usize| closed.get(at).copied().unwrap_or(zero);
    let eye_at = |from: usize| Eye::<S> {
        origin: [take(from), take(from + 1), take(from + 2)],
        direction: [take(from + 3), take(from + 4), take(from + 5)],
        length: take(from + 6),
        unit: [take(from + 7), take(from + 8), take(from + 9)],
    };
    let mirror_eye = eye_at(0);
    let bent_eye = eye_at(10);
    let level = take(20);

    let wide = s.u(look.screen.wide)?;
    let seat = s.mul(y, wide)?;
    let seat = s.add(seat, x)?;
    let drawn = s.fixed(word(look.layout.scene() + scene::DRAWN))?;
    let stir = s.u(0x9E37_79B9)?;
    let stir = s.mul(drawn, stir)?;
    let pick = s.xor(seat, stir)?;
    let pick = hash(s, pick)?;
    let (backdrop, far_away) = backdrop(s, look, &lit, &eye, pick)?;
    let ground_cell = s.pick(valid, bent_cell, behind)?;
    let ground_info = s.pick(valid, bent_info, behind_info)?;
    let bent_found = s.both(valid, bent_has)?;
    let behind_found = s.both(not_valid, has_behind)?;
    let neither = s.not(bent_found)?;
    let nor = s.not(behind_found)?;
    let none_found = s.both(neither, nor)?;
    let ground_has = s.not(none_found)?;
    let ground_eye = Eye {
        origin: pick_v(s, valid, bent_eye.origin, eye.origin)?,
        direction: pick_v(s, valid, bent_eye.direction, eye.direction)?,
        length: s.pick_r(valid, bent_eye.length, eye.length)?,
        unit: pick_v(s, valid, bent_eye.unit, eye.unit)?,
    };
    let sea = Sea {
        under: valid,
        level,
    };
    let ground = surface(s, look, &lit, &ground_eye, (ground_cell, ground_info), &sea)?;
    let zero = s.f(0.0)?;
    let base = pick_v(s, ground_has, ground.colour, backdrop)?;
    let base_glow = [zero; 3];
    let base_glow = pick_v(s, ground_has, ground.glow, base_glow)?;

    let one = s.f(1.0)?;
    let half = s.f(0.5)?;
    let start = entry(s, look.grid, &eye, (first, first_info))?;
    let end = entry(s, look.grid, &eye, (behind, behind_info))?;
    let out = leaving(s, look.grid, &eye)?;
    let end = s.pick_r(has_behind, end, out)?;
    let thick = s.fsub(end, start)?;
    let thick = s.fmax(thick, zero)?;
    let thick = s.fmul(thick, eye.length)?;
    let bent_end = entry(s, look.grid, &bent_eye, (bent_cell, bent_info))?;
    let bent_out = leaving(s, look.grid, &bent_eye)?;
    let bent_end = s.pick_r(bent_has, bent_end, bent_out)?;
    let bent_end = s.fmax(bent_end, zero)?;
    let bent_thick = s.fmul(bent_end, bent_eye.length)?;
    let thick = s.pick_r(valid, bent_thick, thick)?;
    let held = s.cell(first)?;
    let kind = kind_of(s, look.kinds, held)?;
    let water = stuff(s, look, kind)?;
    let wet_skin = detail::skin(s, look, kind)?;
    let liquid = s.both(wet_skin.liquid, has_first)?;
    let (axis, up) = face_parts(s, first_info)?;
    let minus = s.f(-1.0)?;
    let sign = s.pick_r(up, one, minus)?;
    let mut normal = [zero; 3];
    for (which, slot) in normal.iter_mut().enumerate() {
        let wanted = s.u(which as u32)?;
        let this = s.eq(axis, wanted)?;
        *slot = s.pick_r(this, sign, zero)?;
    }
    let reach = scaled(s, eye.direction, start)?;
    let met = v_zip(s, eye.origin, reach, S::fadd)?;
    let lifted = scaled(s, normal, half)?;
    let front = v_zip(s, met, lifted, S::fadd)?;
    let [_, spray] = if look.rich() {
        detail::water_at(s, look, front)?
    } else {
        [zero; 2]
    };
    let tiny = s.f(1.0e-4)?;
    let clear = s.fmax(water.clear, tiny)?;
    let clear = s.fmin(clear, one)?;
    let keep = s.log(clear)?;
    let tint = s.f(1.2)?;
    let facing = dot(s, normal, lit.toward)?;
    let facing = s.fmax(facing, zero)?;
    let sun_share = s.f(WATER_SUN)?;
    let sky_share = s.f(WATER_SKY)?;
    let mut through = [zero; 3];
    let mut mean = zero;
    for (channel, slot) in through.iter_mut().enumerate() {
        let colour_kept = s.fsub(tint, of(water.albedo, channel))?;
        let lost = s.fmul(keep, colour_kept)?;
        let lost = s.fmul(lost, thick)?;
        *slot = s.exp(lost)?;
        mean = s.fadd(mean, *slot)?;
    }
    let third = s.f(1.0 / 3.0)?;
    let mean = s.fmul(mean, third)?;
    let deepness = s.fsub(one, mean)?;
    let tone = mix(s, water.albedo, wet_skin.deep, deepness)?;
    let mut layer = [zero; 3];
    let mut layer_glow = [zero; 3];
    for channel in 0..3 {
        let sun = s.fmul(of(lit.sun, channel), facing)?;
        let sun = s.fmul(sun, sun_share)?;
        let sky = s.fmul(of(lit.zenith, channel), sky_share)?;
        let scatter = s.fadd(sun, sky)?;
        let scatter = s.fmul(scatter, of(tone, channel))?;
        let glow = s.fmul(of(water.albedo, channel), water.glow)?;
        let scatter = s.fadd(scatter, glow)?;
        let held = lerp(s, scatter, of(base, channel), of(through, channel))?;
        put(&mut layer, channel, held);
        let kept_glow = s.fmul(of(base_glow, channel), of(through, channel))?;
        put(&mut layer_glow, channel, kept_glow);
    }

    let y_axis = s.u(1)?;
    let opaque = s.fixed(word(look.layout.scene() + scene::OPAQUE))?;
    let at = point(s, look.grid, first)?;
    let fallback = [
        normal[0], normal[1], normal[2], zero, zero, zero, one, zero, zero,
    ];
    let closed = if look.rich() {
        let gate = s.open(liquid)?;
        let clock = detail::clock(s, look)?;
        let mut ripple_slope = [zero; 3];
        for (scale, pace, weight, seed) in [
            (RIPPLE_WIDE, RIPPLE_WIDE_PACE, 1.0, 0x6006),
            (RIPPLE_FINE, RIPPLE_FINE_PACE, RIPPLE_FINE_WEIGHT, 0x7007),
        ] {
            let scale = s.f(scale)?;
            let pace = s.f(pace)?;
            let across = s.fmul(met[0], scale)?;
            let deep = s.fmul(met[2], scale)?;
            let time = s.fmul(clock, pace)?;
            let wave = detail::noise(s, [across, time, deep], seed)?;
            let weight = s.f(weight)?;
            let weight = s.fmul(weight, scale)?;
            let tilt = scaled(s, wave.slope, weight)?;
            ripple_slope = v_zip(s, ripple_slope, tilt, S::fadd)?;
        }
        let on_y = s.eq(axis, y_axis)?;
        let below = s.not(up)?;
        let top = s.both(on_y, below)?;
        let amount = s.fsub(zero, wet_skin.bump)?;
        let tilt_x = s.fmul(ripple_slope[0], amount)?;
        let tilt_z = s.fmul(ripple_slope[2], amount)?;
        let rippled = detail::unit(s, [tilt_x, minus, tilt_z])?;
        let normal_water = pick_v(s, top, rippled, normal)?;
        let mirrored_hit = s.both(valid, mirror_has)?;
        let glance_gate = s.open(mirrored_hit)?;
        let glanced = detail::glance(s, look, &lit, &mirror_eye, (mirror_cell, mirror_info))?;
        let glanced = s.close(glance_gate, &glanced, &[zero; 3])?;
        let glanced = [0, 1, 2].map(|at| glanced.get(at).copied().unwrap_or(zero));

        let sun_open = s.pick_r(valid, ground.shadow, one)?;
        let back = v_map(s, eye.unit, |s, value| s.fsub(zero, value))?;
        let sparkle = detail::glint(
            s,
            normal_water,
            back,
            lit.toward,
            (wet_skin.rough, wet_skin.shine),
        )?;
        let sparkle = s.fmul(sparkle, sun_open)?;

        let corner = [s.real(at[0])?, s.real(at[1])?, s.real(at[2])?];
        let inner = v_zip(s, met, corner, S::fsub)?;
        let across = s.clamp01(inner[0])?;
        let deep = s.clamp01(inner[2])?;
        let mut nearest = s.f(FAR)?;
        let back_word = s.u(u32::MAX)?;
        let one_word = s.u(1)?;
        for (offsets, low, on_x) in [
            ([nothing, nothing, back_word], true, true),
            ([nothing, nothing, one_word], false, true),
            ([nothing, back_word, nothing], true, false),
            ([nothing, one_word, nothing], false, false),
        ] {
            let (inside, index) = neighbour(s, look.grid, at, y_axis, offsets)?;
            let near = s.cell(index)?;
            let near = kind_of(s, look.kinds, near)?;
            let solid = s.bit(opaque, near)?;
            let solid = s.both(inside, solid)?;
            let along = if on_x { across } else { deep };
            let gap = if low { along } else { s.fsub(one, along)? };
            let closer = s.fmin(nearest, gap)?;
            nearest = s.pick_r(solid, closer, nearest)?;
        }
        let shore = smooth(s, nearest, SHORE_FROM, SHORE_OVER)?;
        let shore = s.fsub(one, shore)?;
        let foam_scale = s.f(FOAM_SCALE)?;
        let foam_pace = s.f(FOAM_PACE)?;
        let foam_x = s.fmul(met[0], foam_scale)?;
        let foam_z = s.fmul(met[2], foam_scale)?;
        let foam_t = s.fmul(clock, foam_pace)?;
        let churn = detail::noise(s, [foam_x, foam_t, foam_z], 0x8008)?;
        let lift = s.f(FOAM_LIFT)?;
        let lift = s.fmul(shore, lift)?;
        let broken = s.fadd(churn.value, lift)?;
        let broken = smooth(s, broken, FOAM_FROM, FOAM_OVER)?;
        let shore_foam = s.fmul(shore, broken)?;
        let spray_gain = s.f(SPRAY_FOAM)?;
        let spray_foam = s.fmul(spray, spray_gain)?;
        let spray_foam = s.clamp01(spray_foam)?;
        let churned = smooth(s, churn.value, SPRAY_FROM, SPRAY_OVER)?;
        let spray_foam = s.fmul(spray_foam, churned)?;
        let foam = s.fmax(shore_foam, spray_foam)?;
        let foam = s.pick_r(top, foam, zero)?;

        let three = s.u(3)?;
        let inside_face = s.eq(axis, three)?;
        let flat = either(s, on_y, inside_face)?;
        let side = s.not(flat)?;
        let mut front_inside = s.eq(nothing, nothing)?;
        for (which, size) in sides(look.grid).into_iter().enumerate() {
            let wanted = s.u(which as u32)?;
            let this = s.eq(axis, wanted)?;
            let coordinate = of(at, which);
            let last = s.u(size - 1)?;
            let at_last = s.eq(coordinate, last)?;
            let at_first = s.eq(coordinate, nothing)?;
            let last_edge = s.both(up, at_last)?;
            let first_edge = s.both(below, at_first)?;
            let edge = either(s, last_edge, first_edge)?;
            let away = s.not(edge)?;
            let unchanged = s.not(this)?;
            let fine = either(s, unchanged, away)?;
            front_inside = s.both(front_inside, fine)?;
        }
        let deep_edge = s.u(look.grid.deep - 1)?;
        let on_floor = s.eq(at[1], deep_edge)?;
        let toward_word = s.pick(up, one_word, back_word)?;
        let (_, ahead) = neighbour(s, look.grid, at, axis, [toward_word, nothing, nothing])?;
        let lower = s.u(look.grid.wide)?;
        let lower = s.add(ahead, lower)?;
        let lower = s.pick(on_floor, ahead, lower)?;
        let under = s.cell(lower)?;
        let under = kind_of(s, look.kinds, under)?;
        let visible = s.fixed(word(look.layout.scene() + scene::VISIBLE))?;
        let rests = s.bit(visible, under)?;
        let rests = either(s, rests, on_floor)?;
        let free = s.not(rests)?;
        let on_top = s.eq(at[1], nothing)?;
        let higher = s.u(look.grid.wide)?;
        let higher = s.sub(first, higher)?;
        let higher = s.pick(on_top, first, higher)?;
        let over = s.cell(higher)?;
        let over = kind_of(s, look.kinds, over)?;
        let liquids = s.fixed(word(look.layout.scene() + scene::LIQUID))?;
        let fed = s.bit(liquids, over)?;
        let not_top = s.not(on_top)?;
        let fed = s.both(fed, not_top)?;
        let found = s.both(side, front_inside)?;
        let found = s.both(found, free)?;
        let found = s.both(found, fed)?;
        let table_at = s.u(word(look.layout.table()))?;
        let size_word = s.u(word(MATERIAL_WORDS))?;
        let falls_at = s.mul(kind, size_word)?;
        let falls_at = s.add(table_at, falls_at)?;
        let falls_at = plus(s, falls_at, word(material::FALLS))?;
        let falls = s.pixel(falls_at)?;
        let unmarked = s.eq(falls, nothing)?;
        let marked = s.not(unmarked)?;
        let bit_at = s.sub(falls, one_word)?;
        let bit_at = s.pick(marked, bit_at, nothing)?;
        let flagged = s.bit(held, bit_at)?;
        let flagged = s.both(flagged, marked)?;
        let below_step = s.u(look.grid.wide)?;
        let below_cell = s.add(first, below_step)?;
        let below_cell = s.pick(on_floor, first, below_cell)?;
        let beneath = s.cell(below_cell)?;
        let beneath = kind_of(s, look.kinds, beneath)?;
        let held_up = s.bit(visible, beneath)?;
        let held_up = either(s, held_up, on_floor)?;
        let dropping = s.not(held_up)?;
        let found = either(s, found, dropping)?;
        let found = s.both(found, unmarked)?;
        let falling = either(s, flagged, found)?;
        let fall_scale = s.f(FALL_SCALE)?;
        let fall_stretch = s.f(FALL_STRETCH)?;
        let fall_pace = s.f(FALL_PACE)?;
        let fall_x = s.fmul(met[0], fall_scale)?;
        let fall_z = s.fmul(met[2], fall_scale)?;
        let flowing = s.fmul(clock, fall_pace)?;
        let fall_y = s.fsub(met[1], flowing)?;
        let fall_y = s.fmul(fall_y, fall_stretch)?;
        let fall_gate = s.open(falling)?;
        let streaks = detail::noise(s, [fall_x, fall_y, fall_z], 0x9009)?;
        let streaks = s.close(fall_gate, &[streaks.value], &[zero])?;
        let streaks = streaks.first().copied().unwrap_or(zero);
        let streaks = smooth(s, streaks, FALL_FROM, FALL_OVER)?;
        let fall_base = s.f(FALL_BASE)?;
        let fall_rest = s.f(1.0 - FALL_BASE)?;
        let white = s.fmul(streaks, fall_rest)?;
        let white = s.fadd(white, fall_base)?;
        let white = s.pick_r(falling, white, zero)?;
        let foam_most = s.f(FOAM_MOST)?;
        let foam = s.fmul(foam, foam_most)?;
        let fall_most = s.f(FALL_MOST)?;
        let white = s.fmul(white, fall_most)?;
        let foamy = s.fmax(foam, white)?;
        let given = [
            normal_water[0],
            normal_water[1],
            normal_water[2],
            glanced[0],
            glanced[1],
            glanced[2],
            sun_open,
            sparkle,
            foamy,
        ];
        s.close(gate, &given, &fallback)?
    } else {
        fallback.to_vec()
    };
    let take = |at: usize| closed.get(at).copied().unwrap_or(zero);
    let normal_water = [take(0), take(1), take(2)];
    let glanced = [take(3), take(4), take(5)];
    let sun_open = take(6);
    let sparkle = take(7);
    let foamy = take(8);

    let cosine = dot(s, eye.unit, normal_water)?;
    let cosine = s.fsub(zero, cosine)?;
    let cosine = s.fmax(cosine, zero)?;
    let rest = s.fsub(one, cosine)?;
    let five = s.f(5.0)?;
    let rest = s.pow(rest, five)?;
    let spread = s.f(0.98)?;
    let least = s.f(0.02)?;
    let fresnel = s.fmul(rest, spread)?;
    let fresnel = s.fadd(fresnel, least)?;
    let way = detail::mirrored(s, eye.unit, normal_water)?;
    let sky_seen = sky(s, &lit, way, false)?;
    let mirrored_hit = s.both(valid, mirror_has)?;
    let reflected = pick_v(s, mirrored_hit, glanced, sky_seen)?;
    let clean = s.fsub(one, foamy)?;
    let fresnel = s.fmul(fresnel, clean)?;
    let sparkle = s.fmul(sparkle, clean)?;
    let foam_facing = dot(s, normal_water, lit.toward)?;
    let foam_facing = s.fmax(foam_facing, zero)?;
    let foam_wrap = s.f(FOAM_WRAP)?;
    let foam_facing = lerp(s, foam_wrap, one, foam_facing)?;
    let foam_facing = s.fmul(foam_facing, sun_open)?;
    let foam_white = s.f(FOAM_WHITE)?;
    let foam_sky = s.f(FOAM_SKY)?;
    for channel in 0..3 {
        let sun = s.fmul(of(lit.sun, channel), foam_facing)?;
        let fill = s.fadd(of(lit.zenith, channel), of(lit.horizon, channel))?;
        let fill = s.fmul(fill, foam_sky)?;
        let light = s.fadd(sun, fill)?;
        let froth = s.fmul(light, foam_white)?;
        let held = lerp(s, of(layer, channel), froth, foamy)?;
        let held = lerp(s, held, of(reflected, channel), fresnel)?;
        let shone = s.fmul(of(lit.sun, channel), sparkle)?;
        put(&mut layer, channel, s.fadd(held, shone)?);
    }
    let colour = pick_v(s, layered, layer, base)?;
    let glow = pick_v(s, layered, layer_glow, base_glow)?;

    let distance = s.fmul(start, eye.length)?;
    let thickness = s.fmul(distance, lit.haze)?;
    let thickness = s.fsub(zero, thickness)?;
    let left = s.exp(thickness)?;
    let haze = s.fsub(one, left)?;
    let mist = sky(s, &lit, eye.unit, false)?;
    let hazed = mix(s, colour, mist, haze)?;
    let spray_haze = s.f(SPRAY_HAZE)?;
    let spray_haze = s.fmul(spray, spray_haze)?;
    let spray_haze = s.clamp01(spray_haze)?;
    let spray_most = s.f(SPRAY_MOST)?;
    let spray_haze = s.fmul(spray_haze, spray_most)?;
    let mut spray_light = [zero; 3];
    let spray_sun = s.f(SPRAY_SUN)?;
    for (channel, slot) in spray_light.iter_mut().enumerate() {
        let sun = s.fmul(of(lit.sun, channel), spray_sun)?;
        let fill = s.fadd(of(lit.zenith, channel), of(lit.horizon, channel))?;
        *slot = s.fadd(sun, fill)?;
    }
    let hazed = mix(s, hazed, spray_light, spray_haze)?;
    let colour = pick_v(s, has_first, hazed, colour)?;
    let faded = scaled(s, glow, left)?;
    let glow = pick_v(s, has_first, faded, glow)?;

    let fog = [
        s.bits(fog[0])?,
        s.bits(fog[1])?,
        s.bits(fog[2])?,
        s.bits(fog[3])?,
    ];
    let shows = fog[3];
    let mut fogged = [zero; 3];
    let mut fogged_glow = [zero; 3];
    for channel in 0..3 {
        let kept = s.fmul(of(colour, channel), shows)?;
        let held = s.fadd(kept, of([fog[0], fog[1], fog[2]], channel))?;
        put(&mut fogged, channel, held);
        let kept = s.fmul(of(glow, channel), shows)?;
        put(&mut fogged_glow, channel, kept);
    }

    let endless = s.f(FAR)?;
    let reached = s.pick_r(has_first, start, endless)?;
    let top = real_at(s, word(look.layout.scene() + scene::TOP))?;
    let (enters, leaves) = boxed(s, look.grid, &eye, top)?;
    let met = s.fgt(leaves, enters)?;
    let mut colour = fogged;
    let mut glow = fogged_glow;
    let floor_far = s.fdiv(far_away, eye.length)?;
    let until = s.pick_r(has_first, reached, floor_far)?;
    for crossing in [leaves, enters] {
        let ahead = s.fgt(crossing, zero)?;
        let before = s.fgt(until, crossing)?;
        let shown = s.both(ahead, before)?;
        let shown = s.both(shown, met)?;
        let Glass {
            add,
            keep,
            standing,
        } = pane(s, look, &lit, &eye, (crossing, top))?;
        let shown = s.both(shown, standing)?;
        let keep = s.pick_r(shown, keep, one)?;
        for channel in 0..3 {
            let kept = s.fmul(of(colour, channel), keep)?;
            let added = s.pick_r(shown, of(add, channel), zero)?;
            let held = s.fadd(kept, added)?;
            put(&mut colour, channel, held);
            let kept = s.fmul(of(glow, channel), keep)?;
            put(&mut glow, channel, kept);
        }
    }

    let flagged = s.fixed(word(look.layout.scene() + scene::FLAGS))?;
    let overlay = s.and(flagged, flags::ACTIVE)?;
    let none_word = s.u(0)?;
    let overlay_off = s.eq(overlay, none_word)?;
    let marked = active_at(s, look, first)?;
    let marked = s.both(marked, has_first)?;
    let dim = s.f(ACTIVE_DIM)?;
    let teal = [s.f(0.06)?, s.f(0.34)?, s.f(0.28)?];
    let kept = s.f(ACTIVE_KEPT)?;
    let mut shown = [zero; 3];
    let mut shown_glow = [zero; 3];
    for channel in 0..3 {
        let dimmed = s.fmul(of(colour, channel), dim)?;
        let lifted = s.fmul(of(colour, channel), kept)?;
        let lifted = s.fadd(lifted, of(teal, channel))?;
        let over = s.pick_r(marked, lifted, dimmed)?;
        let held = s.pick_r(overlay_off, of(colour, channel), over)?;
        put(&mut shown, channel, held);
        let dimmed = s.fmul(of(glow, channel), dim)?;
        let over = s.pick_r(marked, of(teal, channel), dimmed)?;
        let held = s.pick_r(overlay_off, of(glow, channel), over)?;
        put(&mut shown_glow, channel, held);
    }

    let surface_far = s.fmul(start, eye.length)?;
    let distance = s.pick_r(has_first, surface_far, far_away)?;
    Ok(Hdr {
        colour: shown,
        glow: shown_glow,
        distance,
    })
}

/// Whether the oct that holds `cell` at the phase of the engine's frame was
/// stepped this frame.
pub(crate) fn active_at<S: Shader>(s: &mut S, look: &Look, cell: S::W) -> Result<S::T, LaneError> {
    let grid = look.grid;
    let frame = s.frame()?;
    let phase = s.and(frame, 1)?;
    let at = point(s, grid, cell)?;
    let mut oct = s.u(0)?;
    let mut shift = 0;
    for (axis, side) in sides(grid).into_iter().enumerate() {
        let moved = s.sub(of(at, axis), phase)?;
        let moved = s.and(moved, side - 1)?;
        let lattice = s.shr(moved, 1)?;
        let placed = s.shl(lattice, shift)?;
        oct = s.or(oct, placed)?;
        shift += (side / 2).trailing_zeros();
    }
    let base = s.u(word(look.layout.active()))?;
    let at = s.add(base, oct)?;
    let stamp = s.pixel(at)?;
    let next = plus(s, frame, 1)?;
    s.eq(stamp, next)
}

/// The CPU side of [`Shader`].
pub(crate) struct Plainly<'held> {
    pub world: World<'held>,
    pub pixels: &'held [u32],
    pub frame: u32,
}

impl Shader for Plainly<'_> {
    type W = u32;
    type R = f32;
    type T = bool;
    type G = bool;

    fn open(&mut self, when: bool) -> Result<bool, LaneError> {
        Ok(when)
    }
    fn close(
        &mut self,
        gate: bool,
        given: &[f32],
        otherwise: &[f32],
    ) -> Result<Vec<f32>, LaneError> {
        Ok(if gate { given } else { otherwise }.to_vec())
    }
    fn close_words(
        &mut self,
        gate: bool,
        given: &[u32],
        otherwise: &[u32],
    ) -> Result<Vec<u32>, LaneError> {
        Ok(if gate { given } else { otherwise }.to_vec())
    }

    fn u(&mut self, value: u32) -> Result<u32, LaneError> {
        Ok(value)
    }
    fn f(&mut self, value: f32) -> Result<f32, LaneError> {
        Ok(value)
    }
    fn add(&mut self, a: u32, b: u32) -> Result<u32, LaneError> {
        Ok(a.wrapping_add(b))
    }
    fn sub(&mut self, a: u32, b: u32) -> Result<u32, LaneError> {
        Ok(a.wrapping_sub(b))
    }
    fn mul(&mut self, a: u32, b: u32) -> Result<u32, LaneError> {
        Ok(a.wrapping_mul(b))
    }
    fn div(&mut self, a: u32, b: u32) -> Result<u32, LaneError> {
        Ok(a.checked_div(b).unwrap_or(u32::MAX))
    }
    fn and(&mut self, a: u32, b: u32) -> Result<u32, LaneError> {
        Ok(a & b)
    }
    fn shr(&mut self, a: u32, by: u32) -> Result<u32, LaneError> {
        Ok(a.checked_shr(by).unwrap_or(0))
    }
    fn shl(&mut self, a: u32, by: u32) -> Result<u32, LaneError> {
        Ok(a.checked_shl(by).unwrap_or(0))
    }
    fn or(&mut self, a: u32, b: u32) -> Result<u32, LaneError> {
        Ok(a | b)
    }
    fn xor(&mut self, a: u32, b: u32) -> Result<u32, LaneError> {
        Ok(a ^ b)
    }
    fn eq(&mut self, a: u32, b: u32) -> Result<bool, LaneError> {
        Ok(a == b)
    }
    fn lt(&mut self, a: u32, b: u32) -> Result<bool, LaneError> {
        Ok(a < b)
    }
    fn bit(&mut self, mask: u32, at: u32) -> Result<bool, LaneError> {
        Ok(mask.checked_shr(at).unwrap_or(0) & 1 == 1)
    }
    fn both(&mut self, a: bool, b: bool) -> Result<bool, LaneError> {
        Ok(a && b)
    }
    fn not(&mut self, a: bool) -> Result<bool, LaneError> {
        Ok(!a)
    }
    fn pick(&mut self, when: bool, yes: u32, no: u32) -> Result<u32, LaneError> {
        Ok(if when { yes } else { no })
    }
    fn pick_r(&mut self, when: bool, yes: f32, no: f32) -> Result<f32, LaneError> {
        Ok(if when { yes } else { no })
    }
    fn fadd(&mut self, a: f32, b: f32) -> Result<f32, LaneError> {
        Ok(a + b)
    }
    fn fsub(&mut self, a: f32, b: f32) -> Result<f32, LaneError> {
        Ok(a - b)
    }
    fn fmul(&mut self, a: f32, b: f32) -> Result<f32, LaneError> {
        Ok(a * b)
    }
    fn fdiv(&mut self, a: f32, b: f32) -> Result<f32, LaneError> {
        Ok(a / b)
    }
    fn fmin(&mut self, a: f32, b: f32) -> Result<f32, LaneError> {
        Ok(a.min(b))
    }
    fn fmax(&mut self, a: f32, b: f32) -> Result<f32, LaneError> {
        Ok(a.max(b))
    }
    fn clamp01(&mut self, a: f32) -> Result<f32, LaneError> {
        Ok(a.clamp(0.0, 1.0))
    }
    fn exp(&mut self, a: f32) -> Result<f32, LaneError> {
        Ok(a.exp())
    }
    fn pow(&mut self, a: f32, b: f32) -> Result<f32, LaneError> {
        Ok(a.powf(b))
    }
    fn sqrt(&mut self, a: f32) -> Result<f32, LaneError> {
        Ok(a.sqrt())
    }
    fn log(&mut self, a: f32) -> Result<f32, LaneError> {
        Ok(a.ln())
    }
    fn floor(&mut self, a: f32) -> Result<f32, LaneError> {
        Ok(a.floor())
    }
    fn fabs(&mut self, a: f32) -> Result<f32, LaneError> {
        Ok(a.abs())
    }
    fn fgt(&mut self, a: f32, b: f32) -> Result<bool, LaneError> {
        Ok(a > b)
    }
    fn real(&mut self, a: u32) -> Result<f32, LaneError> {
        Ok(a as f32)
    }
    fn signed(&mut self, a: u32) -> Result<f32, LaneError> {
        Ok(a as i32 as f32)
    }
    fn whole(&mut self, a: f32) -> Result<u32, LaneError> {
        Ok(a as u32)
    }
    fn whole_signed(&mut self, a: f32) -> Result<u32, LaneError> {
        Ok(a as i32 as u32)
    }
    fn bits(&mut self, a: u32) -> Result<f32, LaneError> {
        Ok(f32::from_bits(a))
    }
    fn word_of(&mut self, a: f32) -> Result<u32, LaneError> {
        Ok(a.to_bits())
    }
    fn cell(&mut self, at: u32) -> Result<u32, LaneError> {
        Ok(self.world.cells.get(at as usize).copied().unwrap_or(0))
    }
    fn fixed(&mut self, at: u32) -> Result<u32, LaneError> {
        Ok(self.pixels.get(at as usize).copied().unwrap_or(0))
    }
    fn pixel(&mut self, at: u32) -> Result<u32, LaneError> {
        Ok(self.pixels.get(at as usize).copied().unwrap_or(0))
    }
    fn frame(&mut self) -> Result<u32, LaneError> {
        Ok(self.frame)
    }
    fn blocked(
        &mut self,
        _opaque: u32,
        origin: [u32; 3],
        direction: [u32; 3],
        going: bool,
    ) -> Result<bool, LaneError> {
        Ok(going
            && self.world.blocked(
                origin.map(|value| value as i32),
                direction.map(|value| value as i32),
            ))
    }
    fn found(
        &mut self,
        mask: u32,
        origin: [u32; 3],
        direction: [u32; 3],
        going: bool,
    ) -> Result<(u32, u32), LaneError> {
        Ok(if going {
            self.world.found(
                origin.map(|value| value as i32),
                direction.map(|value| value as i32),
                mask,
            )
        } else {
            (MISS, 0)
        })
    }
}

/// The device side of [`Shader`].
pub(crate) struct Emitted<'held, 'kernel> {
    pub em: &'held mut Em<'kernel>,
    pub reader: Reader,
}

impl Shader for Emitted<'_, '_> {
    type W = Id;
    type R = Id;
    type T = Id;
    type G = Opened;

    fn open(&mut self, when: Id) -> Result<Opened, LaneError> {
        self.em.open(when)
    }
    fn close(
        &mut self,
        gate: Opened,
        given: &[Id],
        otherwise: &[Id],
    ) -> Result<Vec<Id>, LaneError> {
        self.em.close(gate, given, otherwise)
    }
    fn close_words(
        &mut self,
        gate: Opened,
        given: &[Id],
        otherwise: &[Id],
    ) -> Result<Vec<Id>, LaneError> {
        self.em.close_words(gate, given, otherwise)
    }

    fn u(&mut self, value: u32) -> Result<Id, LaneError> {
        self.em.u(value)
    }
    fn f(&mut self, value: f32) -> Result<Id, LaneError> {
        self.em.f(value)
    }
    fn add(&mut self, a: Id, b: Id) -> Result<Id, LaneError> {
        self.em.add(a, b)
    }
    fn sub(&mut self, a: Id, b: Id) -> Result<Id, LaneError> {
        self.em.sub(a, b)
    }
    fn mul(&mut self, a: Id, b: Id) -> Result<Id, LaneError> {
        self.em.mul(a, b)
    }
    fn div(&mut self, a: Id, b: Id) -> Result<Id, LaneError> {
        self.em.div(a, b)
    }
    fn and(&mut self, a: Id, b: u32) -> Result<Id, LaneError> {
        self.em.and_u(a, b)
    }
    fn shr(&mut self, a: Id, by: u32) -> Result<Id, LaneError> {
        self.em.shr(a, by)
    }
    fn shl(&mut self, a: Id, by: u32) -> Result<Id, LaneError> {
        self.em.shl(a, by)
    }
    fn or(&mut self, a: Id, b: Id) -> Result<Id, LaneError> {
        self.em.or(a, b)
    }
    fn xor(&mut self, a: Id, b: Id) -> Result<Id, LaneError> {
        self.em.xor(a, b)
    }
    fn eq(&mut self, a: Id, b: Id) -> Result<Id, LaneError> {
        self.em.eq(a, b)
    }
    fn lt(&mut self, a: Id, b: Id) -> Result<Id, LaneError> {
        self.em.lt(a, b)
    }
    fn bit(&mut self, mask: Id, at: Id) -> Result<Id, LaneError> {
        self.em.bit(mask, at)
    }
    fn both(&mut self, a: Id, b: Id) -> Result<Id, LaneError> {
        self.em.both(a, b)
    }
    fn not(&mut self, a: Id) -> Result<Id, LaneError> {
        self.em.not(a)
    }
    fn pick(&mut self, when: Id, yes: Id, no: Id) -> Result<Id, LaneError> {
        self.em.pick(when, yes, no)
    }
    fn pick_r(&mut self, when: Id, yes: Id, no: Id) -> Result<Id, LaneError> {
        self.em.pick_f(when, yes, no)
    }
    fn fadd(&mut self, a: Id, b: Id) -> Result<Id, LaneError> {
        self.em.fadd(a, b)
    }
    fn fsub(&mut self, a: Id, b: Id) -> Result<Id, LaneError> {
        self.em.fsub(a, b)
    }
    fn fmul(&mut self, a: Id, b: Id) -> Result<Id, LaneError> {
        self.em.fmul(a, b)
    }
    fn fdiv(&mut self, a: Id, b: Id) -> Result<Id, LaneError> {
        self.em.fdiv(a, b)
    }
    fn fmin(&mut self, a: Id, b: Id) -> Result<Id, LaneError> {
        self.em.fmin(a, b)
    }
    fn fmax(&mut self, a: Id, b: Id) -> Result<Id, LaneError> {
        self.em.fmax(a, b)
    }
    fn clamp01(&mut self, a: Id) -> Result<Id, LaneError> {
        self.em.clamp01(a)
    }
    fn exp(&mut self, a: Id) -> Result<Id, LaneError> {
        self.em.exp(a)
    }
    fn pow(&mut self, a: Id, b: Id) -> Result<Id, LaneError> {
        self.em.pow(a, b)
    }
    fn sqrt(&mut self, a: Id) -> Result<Id, LaneError> {
        self.em.sqrt(a)
    }
    fn log(&mut self, a: Id) -> Result<Id, LaneError> {
        self.em.log(a)
    }
    fn floor(&mut self, a: Id) -> Result<Id, LaneError> {
        self.em.floor(a)
    }
    fn fabs(&mut self, a: Id) -> Result<Id, LaneError> {
        self.em.fabs(a)
    }
    fn fgt(&mut self, a: Id, b: Id) -> Result<Id, LaneError> {
        self.em.fgt(a, b)
    }
    fn real(&mut self, a: Id) -> Result<Id, LaneError> {
        self.em.to_float(a)
    }
    fn signed(&mut self, a: Id) -> Result<Id, LaneError> {
        self.em.signed_float(a)
    }
    fn whole(&mut self, a: Id) -> Result<Id, LaneError> {
        self.em.to_word(a)
    }
    fn whole_signed(&mut self, a: Id) -> Result<Id, LaneError> {
        self.em.to_signed_word(a)
    }
    fn bits(&mut self, a: Id) -> Result<Id, LaneError> {
        self.em.bits_float(a)
    }
    fn word_of(&mut self, a: Id) -> Result<Id, LaneError> {
        self.em.float_bits(a)
    }
    fn cell(&mut self, at: Id) -> Result<Id, LaneError> {
        self.em.load(STATE, at)
    }
    fn fixed(&mut self, at: u32) -> Result<Id, LaneError> {
        self.em.load_u(PIXELS, at)
    }
    fn pixel(&mut self, at: Id) -> Result<Id, LaneError> {
        self.em.load(PIXELS, at)
    }
    fn frame(&mut self) -> Result<Id, LaneError> {
        self.em.load_u(PARAMS, 0)
    }
    fn blocked(
        &mut self,
        opaque: Id,
        origin: [Id; 3],
        direction: [Id; 3],
        going: Id,
    ) -> Result<Id, LaneError> {
        self.reader
            .blocked(self.em, opaque, (origin, direction), going)
    }
    fn found(
        &mut self,
        mask: Id,
        origin: [Id; 3],
        direction: [Id; 3],
        going: Id,
    ) -> Result<(Id, Id), LaneError> {
        self.reader.found(self.em, mask, (origin, direction), going)
    }
}

pub(crate) fn kernel(subgroup: u32) -> Result<simdr::kernel::Kernel<simdr::lanes::U32>, LaneError> {
    Kernel::new(Shape::new(subgroup, subgroup, BINDINGS))
}

pub(crate) fn look_of(build: &Build) -> Look {
    Look {
        grid: build.grid,
        kinds: build.kinds,
        screen: build.screen,
        layout: build.layout(),
        suns: build.suns.clamp(1, super::SUNS as u32),
    }
}

pub(crate) fn reader_of(build: &Build) -> Reader {
    Reader {
        grid: build.grid,
        kinds: build.kinds,
        bricks_at: word(build.layout().occupancy()),
    }
}

/// The pixel a lane works on of a screen of `screen`, whether it is on the
/// screen, and its `x` and `y`.
pub(crate) fn placed(
    em: &mut Em<'_>,
    subgroup: u32,
    screen: Screen,
) -> Result<(Id, Id, Id, Id), LaneError> {
    let pixel = em.invocation(subgroup)?;
    let count = em.u(word(screen.pixels()))?;
    let shown = em.lt(pixel, count)?;
    let wide = em.u(screen.wide)?;
    let y = em.div(pixel, wide)?;
    let row = em.mul(y, wide)?;
    let x = em.sub(pixel, row)?;
    Ok((pixel, shown, x, y))
}

/// Where a lane stores word `slot` of its record of `words` words that
/// starts at `base`: its own record when `shown`, else the spare words.
pub(crate) fn seat(
    em: &mut Em<'_>,
    layout: &Layout,
    (lane, shown, count): (Id, Id, Id),
    base: usize,
    words: usize,
) -> Result<Id, LaneError> {
    let size = em.u(word(words))?;
    let at = em.mul(lane, size)?;
    let start = em.u(word(base))?;
    let at = em.add(start, at)?;
    let past = em.sub(lane, count)?;
    let spare_size = em.u(word(SPARE_WORDS))?;
    let past = em.mul(past, spare_size)?;
    let spare = em.u(word(layout.spare()))?;
    let past = em.add(spare, past)?;
    em.pick(shown, at, past)
}

/// The pass that marches every pixel's ray and writes its four hit words
/// from [`Layout::hits`] on, as [`World::seen`] finds them, and the gas it
/// crosses before its first hit from [`Layout::fog`] on.
#[allow(clippy::too_many_lines)]
pub fn primary(build: &Build) -> Result<Vec<u32>, LaneError> {
    refuse("render3::primary", build.grid, build.screen)?;
    let layout = build.layout();
    let grid = build.grid;
    let look = look_of(build);
    let reader = reader_of(build);
    let mut kernel = kernel(build.subgroup)?;
    let mut em = Em::new(&mut kernel)?;
    let (pixel, shown, x, y) = placed(&mut em, build.subgroup, build.screen)?;
    let base = word(layout.scene());
    let mut origin = [pixel; 3];
    for (axis, slot) in origin.iter_mut().enumerate() {
        *slot = em.load_u(PIXELS, base + word(scene::ORIGIN + axis))?;
    }
    let direction = {
        let mut plain = Emitted {
            em: &mut em,
            reader,
        };
        direction_words(&mut plain, &look, (x, y))?
    };
    let visible = em.load_u(PIXELS, base + word(scene::VISIBLE))?;
    let opaque = em.load_u(PIXELS, base + word(scene::OPAQUE))?;
    let all = em.u(u32::MAX)?;
    let not_opaque = em.xor(opaque, all)?;
    let clear_kinds = em.and(visible, not_opaque)?;

    let phase = {
        let mut plain = Emitted {
            em: &mut em,
            reader,
        };
        gas_phase(&mut plain, &look, direction)?
    };
    let start = Marching::from(&mut em, origin, direction)?;
    let (start, alive) = start.entered(&mut em, sides(grid))?;
    let alive = em.both(alive, shown)?;
    let alive = em.flag(alive)?;
    let none = em.u(0)?;
    let miss = em.u(MISS)?;
    let nothing = em.f(0.0)?;
    let nothing = em.float_bits(nothing)?;
    let whole = em.f(1.0)?;
    let whole = em.float_bits(whole)?;
    const CARRIED: usize = 10;
    let mut initial = vec![
        alive, none, miss, none, miss, none, nothing, nothing, nothing, whole,
    ];
    initial.extend(start.carried());
    let out = em.looped(limit(grid), &initial, |em, words, run| {
        let get = |at: usize| words.get(at).copied().unwrap_or(none);
        let ray = start.with(words.get(CARRIED..).unwrap_or(&[]));
        let through = em.ne_u(get(1), 0)?;
        let (cell, kind, full, brick) = reader.read_brick(em, &ray)?;
        let mask = em.pick(through, opaque, visible)?;
        let seen = em.bit(mask, kind)?;
        let stops = em.both(full, seen)?;
        let clear = em.bit(clear_kinds, kind)?;
        let run = em.shl(run, 3)?;
        let found = em.or(ray.face, run)?;
        let before = em.not(through)?;
        let fresh = em.both(before, stops)?;
        let first = em.pick(fresh, cell, get(2))?;
        let first_info = em.pick(fresh, found, get(3))?;
        let enters = em.both(fresh, clear)?;
        let stays = em.not(enters)?;
        let ends = em.both(stops, stays)?;
        let behind = em.pick(ends, cell, get(4))?;
        let behind_info = em.pick(ends, found, get(5))?;
        let gassy = em.and_u(brick, GASSY)?;
        let gas = em.ne_u(gassy, 0)?;
        let seen_now = em.not(stops)?;
        let gas = em.both(gas, seen_now)?;
        let gas = em.both(gas, before)?;
        let fog_words = [get(6), get(7), get(8), get(9)];
        let fog_words = em.when(gas, &fog_words, |em| {
            let mut plain = Emitted { em, reader };
            let fog = [
                plain.bits(fog_words[0])?,
                plain.bits(fog_words[1])?,
                plain.bits(fog_words[2])?,
                plain.bits(fog_words[3])?,
            ];
            let fog = gas_step(&mut plain, &look, cell, fog, phase)?;
            let mut out = Vec::with_capacity(4);
            for value in fog {
                out.push(plain.word_of(value)?);
            }
            Ok(out)
        })?;
        let through = em.either(through, enters)?;
        let through = em.flag(through)?;
        let moved = ray.advanced(em, full)?;
        let inside = moved.inside(em, sides(grid))?;
        let going = em.not(ends)?;
        let going = em.both(going, inside)?;
        let going = em.flag(going)?;
        let mut next = vec![going, through, first, first_info, behind, behind_info];
        next.extend(fog_words);
        next.extend(moved.carried());
        Ok(next)
    })?;
    let count = em.u(word(build.screen.pixels()))?;
    let at = seat(
        &mut em,
        &layout,
        (pixel, shown, count),
        layout.hits(),
        HIT_WORDS,
    )?;
    for slot in 0..HIT_WORDS {
        let step = em.u(word(slot))?;
        let into = em.add(at, step)?;
        let value = out.get(slot + 2).copied().unwrap_or(none);
        em.store(PIXELS, into, value)?;
    }
    let at = seat(
        &mut em,
        &layout,
        (pixel, shown, count),
        layout.fog(),
        FOG_WORDS,
    )?;
    for slot in 0..FOG_WORDS {
        let step = em.u(word(slot))?;
        let into = em.add(at, step)?;
        let value = out.get(slot + 6).copied().unwrap_or(none);
        em.store(PIXELS, into, value)?;
    }
    kernel.finish()
}

/// The pass that lights every pixel from its hit words and its gas and
/// writes its light, its glow and how far its ray went, from
/// [`Layout::hdr`] on.
pub fn shade(build: &Build) -> Result<Vec<u32>, LaneError> {
    refuse("render3::shade", build.grid, build.screen)?;
    let layout = build.layout();
    let look = look_of(build);
    let reader = reader_of(build);
    let mut kernel = kernel(build.subgroup)?;
    let mut em = Em::new(&mut kernel)?;
    let (pixel, shown, x, y) = placed(&mut em, build.subgroup, build.screen)?;
    let none = em.u(0)?;
    let place = em.pick(shown, pixel, none)?;
    let mut hit = [none; HIT_WORDS];
    let mut fog = [none; FOG_WORDS];
    let mut bounced = [none; BOUNCE_WORDS];
    for (base, size, into) in [
        (layout.hits(), HIT_WORDS, &mut hit),
        (layout.fog(), FOG_WORDS, &mut fog),
        (layout.bounces(), BOUNCE_WORDS, &mut bounced),
    ] {
        let size_word = em.u(word(size))?;
        let at = em.mul(place, size_word)?;
        let start = em.u(word(base))?;
        let at = em.add(start, at)?;
        for (slot, value) in into.iter_mut().enumerate() {
            let step = em.u(word(slot))?;
            let from = em.add(at, step)?;
            *value = em.load(PIXELS, from)?;
        }
    }
    let words = {
        let mut plain = Emitted {
            em: &mut em,
            reader,
        };
        let light = seen(&mut plain, &look, (x, y), hit, fog, bounced)?;
        hdr_words(&mut plain, &light)?
    };
    let count = em.u(word(build.screen.pixels()))?;
    let at = seat(
        &mut em,
        &layout,
        (pixel, shown, count),
        layout.hdr(),
        HDR_WORDS,
    )?;
    for (slot, value) in words.into_iter().enumerate() {
        let step = em.u(word(slot))?;
        let into = em.add(at, step)?;
        em.store(PIXELS, into, value)?;
    }
    kernel.finish()
}

/// The pass that sends the two rays of every pixel whose first cell is the
/// top of a liquid, and writes what they meet from [`Layout::bounces`] on,
/// as [`Shading::bounces`] finds it.
pub fn bounce(build: &Build) -> Result<Vec<u32>, LaneError> {
    refuse("render3::bounce", build.grid, build.screen)?;
    let layout = build.layout();
    let look = look_of(build);
    let reader = reader_of(build);
    let mut kernel = kernel(build.subgroup)?;
    let mut em = Em::new(&mut kernel)?;
    let (pixel, shown, x, y) = placed(&mut em, build.subgroup, build.screen)?;
    let none = em.u(0)?;
    let place = em.pick(shown, pixel, none)?;
    let size = em.u(word(HIT_WORDS))?;
    let at = em.mul(place, size)?;
    let start = em.u(word(layout.hits()))?;
    let at = em.add(start, at)?;
    let first = em.load(PIXELS, at)?;
    let one = em.u(1)?;
    let next = em.add(at, one)?;
    let info = em.load(PIXELS, next)?;
    let words = {
        let mut plain = Emitted {
            em: &mut em,
            reader,
        };
        detail::bounced_words(&mut plain, &look, (x, y), (first, info))?
    };
    let count = em.u(word(build.screen.pixels()))?;
    let at = seat(
        &mut em,
        &layout,
        (pixel, shown, count),
        layout.bounces(),
        BOUNCE_WORDS,
    )?;
    for (slot, value) in words.into_iter().enumerate() {
        let step = em.u(word(slot))?;
        let into = em.add(at, step)?;
        em.store(PIXELS, into, value)?;
    }
    kernel.finish()
}

/// The words of a pixel's [`Hdr`], in [`Layout::hdr`] order.
pub(crate) fn hdr_words<S: Shader>(s: &mut S, light: &Hdr<S>) -> Result<Vec<S::W>, LaneError> {
    let mut out = Vec::with_capacity(HDR_WORDS);
    for value in light.colour.into_iter().chain(light.glow) {
        out.push(s.word_of(value)?);
    }
    out.push(s.word_of(light.distance)?);
    out.push(s.u(0)?);
    Ok(out)
}

/// The picture on the CPU: the rays, the light, the materials and the sun
/// rays a frame uses, over a world.
#[derive(Debug, Clone)]
pub struct Shading<'held> {
    pub world: World<'held>,
    pub screen: Screen,
    pub layout: Layout,
    /// `PIXELS` as the passes read it.
    pub pixels: &'held [u32],
    pub suns: u32,
    /// The engine's frame, as the first word of `PARAMS` holds it.
    pub frame: u32,
}

impl Shading<'_> {
    pub(crate) fn look(&self) -> Look {
        Look {
            grid: self.world.grid,
            kinds: self.world.kinds,
            screen: self.screen,
            layout: self.layout,
            suns: self.suns.clamp(1, super::SUNS as u32),
        }
    }

    pub(crate) fn plain(&self) -> Plainly<'_> {
        Plainly {
            world: self.world,
            pixels: self.pixels,
            frame: self.frame,
        }
    }

    /// The four hit words of every pixel, as [`primary`] writes them.
    #[must_use]
    pub fn hits(&self, rays: &Rays) -> Vec<u32> {
        let mut out = Vec::with_capacity(self.screen.pixels() * HIT_WORDS);
        for y in 0..self.screen.high {
            for x in 0..self.screen.wide {
                let direction = rays.direction(self.screen, (x, y));
                out.extend(self.world.seen(rays.origin, direction));
            }
        }
        out
    }

    /// The four fog words of every pixel, as [`primary`] writes them: the
    /// gas each ray crosses before its first hit, read through the gas,
    /// sun and lamp volumes `pixels` holds.
    pub fn fog(&self, rays: &Rays) -> Result<Vec<u32>, LaneError> {
        let look = self.look();
        let mut plain = self.plain();
        let mut out = Vec::with_capacity(self.screen.pixels() * FOG_WORDS);
        for y in 0..self.screen.high {
            for x in 0..self.screen.wide {
                let direction = rays.direction(self.screen, (x, y));
                let phase = gas_phase(&mut plain, &look, direction.map(|value| value as u32))?;
                let mut fog = [0.0, 0.0, 0.0, 1.0];
                let mut failed = None;
                self.world.gassed(rays.origin, direction, |cell| {
                    if failed.is_none() {
                        match gas_step(&mut plain, &look, cell, fog, phase) {
                            Ok(next) => fog = next,
                            Err(why) => failed = Some(why),
                        }
                    }
                });
                if let Some(why) = failed {
                    return Err(why);
                }
                out.extend(fog.map(f32::to_bits));
            }
        }
        Ok(out)
    }

    /// The four bounced words of every pixel, as [`bounce`] writes them,
    /// from the hit words `pixels` holds.
    pub fn bounces(&self) -> Result<Vec<u32>, LaneError> {
        let look = self.look();
        let mut plain = self.plain();
        let layout = self.layout;
        let mut out = Vec::with_capacity(self.screen.pixels() * BOUNCE_WORDS);
        for at in 0..self.screen.pixels() {
            let x = word(at) % self.screen.wide;
            let y = word(at) / self.screen.wide;
            let get = |slot: usize| {
                self.pixels
                    .get(layout.hits() + at * HIT_WORDS + slot)
                    .copied()
                    .unwrap_or(0)
            };
            out.extend(detail::bounced_words(
                &mut plain,
                &look,
                (x, y),
                (get(0), get(1)),
            )?);
        }
        Ok(out)
    }

    /// The words [`shade`] writes for every pixel, from the hit, fog and
    /// bounced words `pixels` holds.
    pub fn shaded(&self) -> Result<Vec<u32>, LaneError> {
        let look = self.look();
        let mut plain = self.plain();
        let layout = self.layout;
        let mut out = Vec::with_capacity(self.screen.pixels() * HDR_WORDS);
        for at in 0..self.screen.pixels() {
            let x = word(at) % self.screen.wide;
            let y = word(at) / self.screen.wide;
            let get = |base: usize, slot: usize| {
                self.pixels.get(base + at * 4 + slot).copied().unwrap_or(0)
            };
            let hit = [0, 1, 2, 3].map(|slot| get(layout.hits(), slot));
            let fog = [0, 1, 2, 3].map(|slot| get(layout.fog(), slot));
            let bounced = [0, 1, 2, 3].map(|slot| get(layout.bounces(), slot));
            let light = seen(&mut plain, &look, (x, y), hit, fog, bounced)?;
            out.extend(hdr_words(&mut plain, &light)?);
        }
        Ok(out)
    }
}

impl World<'_> {
    /// Calls `visit` with every cell of a brick that holds gas a ray from
    /// `origin` along `direction` crosses before it first meets a cell it
    /// sees, as [`primary`] gathers them.
    pub fn gassed(&self, origin: [i32; 3], direction: [i32; 3], mut visit: impl FnMut(u32)) {
        let sides = sides(self.grid);
        let Some(mut ray) = Ray::from(origin, direction).entered(sides) else {
            return;
        };
        for _ in 0..limit(self.grid) {
            let (cell, kind, full) = self.kind_at(&ray);
            if full && (self.visible >> (kind & 31)) & 1 == 1 {
                return;
            }
            let brick = self
                .bricks
                .get(super::march::brick_of(self.grid, ray.cell) as usize)
                .copied()
                .unwrap_or(0);
            if brick & GASSY != 0 {
                visit(cell);
            }
            ray = if full { ray.stepped() } else { ray.jumped() };
            if !ray.inside(sides) {
                return;
            }
        }
    }
}
