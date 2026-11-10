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

use super::emit::Em;
use super::march::{Marching, Reader, World, sides};
use super::{
    Build, HIT_WORDS, Layout, MATERIAL_WORDS, MISS, Rays, Screen, UNIT, of, put, refuse, scene,
    word,
};
use crate::layout::{BINDINGS, PIXELS, STATE};
use crate::sand::Grid;
use crate::view3::Kinds;

const SKY_DISC: f32 = 1400.0;
const SKY_DISC_GAIN: f32 = 14.0;
const SUN_HALO: f32 = 10.0;
const SUN_HALO_GAIN: f32 = 0.3;
const GAMMA: f32 = 1.0 / 2.2;
const VIGNETTE: f32 = 0.16;
const FAR: f32 = 1.0e30;
const FLOOR_REACH: f32 = 900.0;

/// The operations the light is written in.
pub(crate) trait Shader {
    type W: Copy;
    type R: Copy;
    type T: Copy;

    fn u(&mut self, value: u32) -> Result<Self::W, LaneError>;
    fn f(&mut self, value: f32) -> Result<Self::R, LaneError>;
    fn add(&mut self, a: Self::W, b: Self::W) -> Result<Self::W, LaneError>;
    fn sub(&mut self, a: Self::W, b: Self::W) -> Result<Self::W, LaneError>;
    fn mul(&mut self, a: Self::W, b: Self::W) -> Result<Self::W, LaneError>;
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
    fn fgt(&mut self, a: Self::R, b: Self::R) -> Result<Self::T, LaneError>;
    fn real(&mut self, a: Self::W) -> Result<Self::R, LaneError>;
    fn signed(&mut self, a: Self::W) -> Result<Self::R, LaneError>;
    fn whole(&mut self, a: Self::R) -> Result<Self::W, LaneError>;
    fn whole_signed(&mut self, a: Self::R) -> Result<Self::W, LaneError>;
    fn bits(&mut self, a: Self::W) -> Result<Self::R, LaneError>;
    /// A word of `STATE`.
    fn cell(&mut self, at: Self::W) -> Result<Self::W, LaneError>;
    /// A word of `PIXELS` at a fixed place.
    fn fixed(&mut self, at: u32) -> Result<Self::W, LaneError>;
    /// A word of `PIXELS`.
    fn pixel(&mut self, at: Self::W) -> Result<Self::W, LaneError>;
    /// Whether a ray from `origin` along `direction` meets a cell of a kind
    /// in `opaque`; nothing is traced where `going` is false.
    fn blocked(
        &mut self,
        opaque: Self::W,
        origin: [Self::W; 3],
        direction: [Self::W; 3],
        going: Self::T,
    ) -> Result<Self::T, LaneError>;
}

type V<R> = [R; 3];

/// What the light needs to know when it is written.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Look {
    pub grid: Grid,
    pub kinds: Kinds,
    pub screen: Screen,
    pub layout: Layout,
    pub suns: u32,
}

struct Lit<S: Shader> {
    toward: V<S::R>,
    sun: V<S::R>,
    zenith: V<S::R>,
    horizon: V<S::R>,
    ground: V<S::R>,
    floor: V<S::R>,
    haze: S::R,
    exposure: S::R,
    occlusion: S::R,
    edge: S::R,
    focal: S::R,
}

struct Eye<S: Shader> {
    origin: V<S::R>,
    direction: V<S::R>,
    length: S::R,
    unit: V<S::R>,
}

struct Surface<S: Shader> {
    colour: V<S::R>,
}

fn v_map<S: Shader>(
    s: &mut S,
    a: V<S::R>,
    mut each: impl FnMut(&mut S, S::R) -> Result<S::R, LaneError>,
) -> Result<V<S::R>, LaneError> {
    Ok([each(s, a[0])?, each(s, a[1])?, each(s, a[2])?])
}

fn v_zip<S: Shader>(
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

fn dot<S: Shader>(s: &mut S, a: V<S::R>, b: V<S::R>) -> Result<S::R, LaneError> {
    let x = s.fmul(a[0], b[0])?;
    let y = s.fmul(a[1], b[1])?;
    let z = s.fmul(a[2], b[2])?;
    let xy = s.fadd(x, y)?;
    s.fadd(xy, z)
}

fn scaled<S: Shader>(s: &mut S, a: V<S::R>, by: S::R) -> Result<V<S::R>, LaneError> {
    v_map(s, a, |s, value| s.fmul(value, by))
}

fn mix<S: Shader>(s: &mut S, a: V<S::R>, b: V<S::R>, by: S::R) -> Result<V<S::R>, LaneError> {
    v_zip(s, a, b, |s, a, b| {
        let gap = s.fsub(b, a)?;
        let part = s.fmul(gap, by)?;
        s.fadd(a, part)
    })
}

fn lerp<S: Shader>(s: &mut S, a: S::R, b: S::R, by: S::R) -> Result<S::R, LaneError> {
    let gap = s.fsub(b, a)?;
    let part = s.fmul(gap, by)?;
    s.fadd(a, part)
}

fn pick_v<S: Shader>(
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

fn real_at<S: Shader>(s: &mut S, at: u32) -> Result<S::R, LaneError> {
    let held = s.fixed(at)?;
    s.bits(held)
}

fn reals_at<S: Shader>(s: &mut S, at: u32) -> Result<V<S::R>, LaneError> {
    Ok([real_at(s, at)?, real_at(s, at + 1)?, real_at(s, at + 2)?])
}

fn lit<S: Shader>(s: &mut S, look: &Look) -> Result<Lit<S>, LaneError> {
    let at = word(look.layout.scene() + scene::LIGHT);
    Ok(Lit {
        toward: reals_at(s, at)?,
        sun: reals_at(s, at + 3)?,
        zenith: reals_at(s, at + 6)?,
        horizon: reals_at(s, at + 9)?,
        ground: reals_at(s, at + 12)?,
        floor: reals_at(s, at + 15)?,
        haze: real_at(s, at + 18)?,
        exposure: real_at(s, at + 19)?,
        occlusion: real_at(s, at + 20)?,
        edge: real_at(s, at + 21)?,
        focal: real_at(s, at + 22)?,
    })
}

/// The direction of pixel `(x, y)`, as two's complement words.
fn direction_words<S: Shader>(
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

fn eye<S: Shader>(s: &mut S, look: &Look, at: (S::W, S::W)) -> Result<Eye<S>, LaneError> {
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

fn sky<S: Shader>(s: &mut S, lit: &Lit<S>, way: V<S::R>, disc: bool) -> Result<V<S::R>, LaneError> {
    let none = s.f(0.0)?;
    let up = s.fsub(none, way[1])?;
    let high = s.clamp01(up)?;
    let high = s.sqrt(high)?;
    let colour = mix(s, lit.horizon, lit.zenith, high)?;
    let three = s.f(3.0)?;
    let below = s.fsub(none, up)?;
    let below = s.fmul(below, three)?;
    let below = s.clamp01(below)?;
    let dim = s.f(0.82)?;
    let floor = scaled(s, lit.horizon, dim)?;
    let colour = mix(s, colour, floor, below)?;
    let facing = dot(s, way, lit.toward)?;
    let facing = s.fmax(facing, none)?;
    let halo = s.f(SUN_HALO)?;
    let halo = s.pow(facing, halo)?;
    let halo_gain = s.f(SUN_HALO_GAIN)?;
    let mut glow = s.fmul(halo, halo_gain)?;
    if disc {
        let sharp = s.f(SKY_DISC)?;
        let sharp = s.pow(facing, sharp)?;
        let gain = s.f(SKY_DISC_GAIN)?;
        let sharp = s.fmul(sharp, gain)?;
        glow = s.fadd(glow, sharp)?;
    }
    let sun = scaled(s, lit.sun, glow)?;
    v_zip(s, colour, sun, S::fadd)
}

/// What a ray that meets no cell sees: the sky, or a plane under the grid
/// lit by the sun, shadowed by the world and hazed with distance.
fn backdrop<S: Shader>(
    s: &mut S,
    look: &Look,
    lit: &Lit<S>,
    eye: &Eye<S>,
) -> Result<V<S::R>, LaneError> {
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
    for axis in [0, 2] {
        let low = s.f(of(sides(grid), axis) as f32 - FLOOR_REACH)?;
        let high = s.f(FLOOR_REACH)?;
        let over = s.fgt(of(point, axis), low)?;
        let under = s.fgt(high, of(point, axis))?;
        near = s.both(near, over)?;
        near = s.both(near, under)?;
        let sixteenths = s.fmul(of(point, axis), unit)?;
        let held = s.whole_signed(sixteenths)?;
        put(&mut origin, axis, held);
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
    let suns = look.suns.max(1);
    let share = s.f(1.0 / suns as f32)?;
    let mut seen = zero;
    for at_sun in 0..suns {
        let base = word(look.layout.scene() + scene::SUN_RAYS) + 3 * at_sun;
        let rays = [s.fixed(base)?, s.fixed(base + 1)?, s.fixed(base + 2)?];
        let blocked = s.blocked(opaque, origin, rays, going)?;
        let open = s.pick_r(blocked, zero, share)?;
        seen = s.fadd(seen, open)?;
    }
    let sky_amount = s.f(0.3)?;
    let mut colour = [zero; 3];
    for (channel, slot) in colour.iter_mut().enumerate() {
        let sun = s.fmul(of(lit.sun, channel), facing)?;
        let sun = s.fmul(sun, seen)?;
        let fill = s.fadd(of(lit.zenith, channel), of(lit.horizon, channel))?;
        let sky = s.fmul(fill, sky_amount)?;
        let light = s.fadd(sun, sky)?;
        let albedo = of(lit.floor, channel);
        *slot = s.fmul(albedo, light)?;
    }
    let distance = s.fmul(far, eye.length)?;
    let thickness = s.fmul(distance, lit.haze)?;
    let thickness = s.fsub(zero, thickness)?;
    let left = s.exp(thickness)?;
    let fog = s.fsub(one, left)?;
    let mist = sky(s, lit, eye.unit, false)?;
    let hazed = mix(s, colour, mist, fog)?;
    pick_v(s, down, hazed, above)
}

/// The cell's point on each axis, from its index.
fn point<S: Shader>(s: &mut S, grid: Grid, cell: S::W) -> Result<V<S::W>, LaneError> {
    let wide = grid.wide.trailing_zeros();
    let deep = grid.deep.trailing_zeros();
    let x = s.and(cell, grid.wide - 1)?;
    let y = s.shr(cell, wide)?;
    let y = s.and(y, grid.deep - 1)?;
    let z = s.shr(cell, wide + deep)?;
    let z = s.and(z, grid.tall - 1)?;
    Ok([x, y, z])
}

/// The axis of a face (3 for [`INSIDE`]) and whether it faces up its axis.
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

fn kind_of<S: Shader>(s: &mut S, kinds: Kinds, held: S::W) -> Result<S::W, LaneError> {
    let moved = s.shr(held, kinds.shift())?;
    s.and(moved, kinds.mask())
}

struct Stuff<S: Shader> {
    albedo: V<S::R>,
    jitter: S::R,
    glow: S::R,
    clear: S::R,
}

fn stuff<S: Shader>(s: &mut S, look: &Look, kind: S::W) -> Result<Stuff<S>, LaneError> {
    let base = s.u(word(look.layout.table()))?;
    let size = s.u(word(MATERIAL_WORDS))?;
    let offset = s.mul(kind, size)?;
    let at = s.add(base, offset)?;
    let mut values = Vec::with_capacity(6);
    for slot in 0..6 {
        let step = s.u(slot)?;
        let here = s.add(at, step)?;
        let held = s.pixel(here)?;
        values.push(s.bits(held)?);
    }
    let get = |at: usize| values.get(at).copied();
    let missing = LaneError::BadCarry {
        given: values.len(),
        wanted: 6,
    };
    match (get(0), get(1), get(2), get(3), get(4), get(5)) {
        (Some(r), Some(g), Some(b), Some(jitter), Some(glow), Some(clear)) => Ok(Stuff {
            albedo: [r, g, b],
            jitter,
            glow,
            clear,
        }),
        _ => Err(missing),
    }
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

fn hash<S: Shader>(s: &mut S, value: S::W) -> Result<S::W, LaneError> {
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

#[allow(clippy::too_many_lines)]
fn surface<S: Shader>(
    s: &mut S,
    look: &Look,
    lit: &Lit<S>,
    eye: &Eye<S>,
    (cell, info): (S::W, S::W),
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
        {
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
    }
    let [ao_00, ao_10, ao_01, ao_11] = corners_ao;
    let ao_low = lerp(s, ao_00, ao_10, u)?;
    let ao_high = lerp(s, ao_01, ao_11, u)?;
    let occlusion = lerp(s, ao_low, ao_high, v)?;
    let [sun_00, sun_10, sun_01, sun_11] = corners_sun;
    let sun_low = lerp(s, sun_00, sun_10, u)?;
    let sun_high = lerp(s, sun_01, sun_11, u)?;
    let shadow = lerp(s, sun_low, sun_high, v)?;

    let mixed = hash(s, cell)?;
    let mut jitters = [zero; 4];
    let byte = s.f(1.0 / 255.0)?;
    let half = s.f(0.5)?;
    for (at_byte, slot) in jitters.iter_mut().enumerate() {
        let part = s.shr(mixed, 8 * at_byte as u32)?;
        let part = s.and(part, 0xFF)?;
        let part = s.real(part)?;
        let part = s.fmul(part, byte)?;
        *slot = s.fsub(part, half)?;
    }
    let broad = s.f(1.6)?;
    let fine = s.f(0.4)?;
    let [red, green, blue, overall] = jitters;
    let all = s.fmul(overall, broad)?;
    let mut albedo = [zero; 3];
    for (channel, slot) in albedo.iter_mut().enumerate() {
        let own = s.fmul(of([red, green, blue], channel), fine)?;
        let shift = s.fadd(all, own)?;
        let shift = s.fmul(shift, material.jitter)?;
        let factor = s.fadd(one, shift)?;
        let factor = s.fmax(factor, zero)?;
        *slot = s.fmul(of(material.albedo, channel), factor)?;
    }

    let distance = s.fmul(far, eye.length)?;
    let distance = s.fmax(distance, one)?;
    let size = s.fdiv(lit.focal, distance)?;
    let u_far = s.fsub(one, u)?;
    let v_far = s.fsub(one, v)?;
    let near_u = s.fmin(u, u_far)?;
    let near_v = s.fmin(v, v_far)?;
    let rim = s.fmin(near_u, near_v)?;
    let pixel_wide = s.f(1.2)?;
    let band = s.fdiv(pixel_wide, size)?;
    let least_band = s.f(0.06)?;
    let band = s.fmax(band, least_band)?;
    let into = s.fdiv(rim, band)?;
    let into = s.clamp01(into)?;
    let outer = s.fsub(one, into)?;
    let fade_from = s.f(3.0)?;
    let fade = s.fsub(size, fade_from)?;
    let quarter = s.f(0.25)?;
    let fade = s.fmul(fade, quarter)?;
    let fade = s.clamp01(fade)?;
    let edge = s.fmul(outer, lit.edge)?;
    let edge = s.fmul(edge, fade)?;
    let edge = s.fsub(one, edge)?;

    let up_ness = s.fsub(zero, normal[1])?;
    let sky_part = s.f(0.15)?;
    let sky_base = s.f(0.15)?;
    let sky_amount = s.fmul(up_ness, sky_part)?;
    let sky_amount = s.fadd(sky_base, sky_amount)?;
    let ground_part = s.f(0.25)?;
    let ground_amount = s.fsub(one, up_ness)?;
    let ground_amount = s.fmul(ground_amount, ground_part)?;
    let mut colour = [zero; 3];
    for channel in 0..3 {
        let sun = s.fmul(of(lit.sun, channel), facing)?;
        let sun = s.fmul(sun, shadow)?;
        let fill = s.fadd(of(lit.zenith, channel), of(lit.horizon, channel))?;
        let sky = s.fmul(fill, sky_amount)?;
        let bounce = s.fmul(of(lit.ground, channel), ground_amount)?;
        let ambient = s.fadd(sky, bounce)?;
        let ambient = s.fmul(ambient, occlusion)?;
        let light = s.fadd(sun, ambient)?;
        let light = s.fadd(light, of(warmth, channel))?;
        let light = s.fmul(light, edge)?;
        let own = s.fmul(of(albedo, channel), material.glow)?;
        let shade = s.fmul(of(albedo, channel), light)?;
        let held = s.fadd(shade, own)?;
        put(&mut colour, channel, held);
    }
    Ok(Surface { colour })
}

fn tone<S: Shader>(s: &mut S, value: S::R) -> Result<S::R, LaneError> {
    let a = s.f(2.51)?;
    let b = s.f(0.03)?;
    let c = s.f(2.43)?;
    let d = s.f(0.59)?;
    let e = s.f(0.14)?;
    let top = s.fmul(value, a)?;
    let top = s.fadd(top, b)?;
    let top = s.fmul(top, value)?;
    let bottom = s.fmul(value, c)?;
    let bottom = s.fadd(bottom, d)?;
    let bottom = s.fmul(bottom, value)?;
    let bottom = s.fadd(bottom, e)?;
    let mapped = s.fdiv(top, bottom)?;
    let mapped = s.clamp01(mapped)?;
    let gamma = s.f(GAMMA)?;
    s.pow(mapped, gamma)
}

/// The colour of pixel `(x, y)`, given its four hit words, as `0x00RRGGBB`.
#[allow(clippy::too_many_lines)]
pub(crate) fn colour<S: Shader>(
    s: &mut S,
    look: &Look,
    (x, y): (S::W, S::W),
    hit: [S::W; 4],
) -> Result<S::W, LaneError> {
    let lit = lit(s, look)?;
    let eye = eye(s, look, (x, y))?;
    let miss = s.u(MISS)?;
    let [first, first_info, behind, behind_info] = hit;
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

    let backdrop = backdrop(s, look, &lit, &eye)?;
    let ground = surface(s, look, &lit, &eye, (behind, behind_info))?;
    let base = pick_v(s, has_behind, ground.colour, backdrop)?;

    let zero = s.f(0.0)?;
    let one = s.f(1.0)?;
    let start = entry(s, look.grid, &eye, (first, first_info))?;
    let end = entry(s, look.grid, &eye, (behind, behind_info))?;
    let out = leaving(s, look.grid, &eye)?;
    let end = s.pick_r(has_behind, end, out)?;
    let thick = s.fsub(end, start)?;
    let thick = s.fmax(thick, zero)?;
    let thick = s.fmul(thick, eye.length)?;
    let held = s.cell(first)?;
    let kind = kind_of(s, look.kinds, held)?;
    let water = stuff(s, look, kind)?;
    let (axis, up) = face_parts(s, first_info)?;
    let minus = s.f(-1.0)?;
    let sign = s.pick_r(up, one, minus)?;
    let mut normal = [zero; 3];
    for (which, slot) in normal.iter_mut().enumerate() {
        let wanted = s.u(which as u32)?;
        let this = s.eq(axis, wanted)?;
        *slot = s.pick_r(this, sign, zero)?;
    }
    let tiny = s.f(1.0e-4)?;
    let clear = s.fmax(water.clear, tiny)?;
    let clear = s.fmin(clear, one)?;
    let keep = s.log(clear)?;
    let tint = s.f(1.2)?;
    let facing = dot(s, normal, lit.toward)?;
    let facing = s.fmax(facing, zero)?;
    let sun_share = s.f(0.3)?;
    let sky_share = s.f(0.4)?;
    let mut layer = [zero; 3];
    for channel in 0..3 {
        let colour_kept = s.fsub(tint, of(water.albedo, channel))?;
        let lost = s.fmul(keep, colour_kept)?;
        let lost = s.fmul(lost, thick)?;
        let through = s.exp(lost)?;
        let sun = s.fmul(of(lit.sun, channel), facing)?;
        let sun = s.fmul(sun, sun_share)?;
        let sky = s.fmul(of(lit.zenith, channel), sky_share)?;
        let scatter = s.fadd(sun, sky)?;
        let scatter = s.fmul(scatter, of(water.albedo, channel))?;
        let glow = s.fmul(of(water.albedo, channel), water.glow)?;
        let scatter = s.fadd(scatter, glow)?;
        let held = lerp(s, scatter, of(base, channel), through)?;
        put(&mut layer, channel, held);
    }
    let cosine = dot(s, eye.unit, normal)?;
    let cosine = s.fsub(zero, cosine)?;
    let cosine = s.fmax(cosine, zero)?;
    let rest = s.fsub(one, cosine)?;
    let five = s.f(5.0)?;
    let rest = s.pow(rest, five)?;
    let spread = s.f(0.98)?;
    let least = s.f(0.02)?;
    let fresnel = s.fmul(rest, spread)?;
    let fresnel = s.fadd(fresnel, least)?;
    let bounce = dot(s, eye.unit, normal)?;
    let two = s.f(2.0)?;
    let bounce = s.fmul(bounce, two)?;
    let push = scaled(s, normal, bounce)?;
    let mirrored = v_zip(s, eye.unit, push, S::fsub)?;
    let reflected = sky(s, &lit, mirrored, true)?;
    let layer = mix(s, layer, reflected, fresnel)?;
    let colour = pick_v(s, layered, layer, base)?;

    let distance = s.fmul(start, eye.length)?;
    let thickness = s.fmul(distance, lit.haze)?;
    let thickness = s.fsub(zero, thickness)?;
    let left = s.exp(thickness)?;
    let fog = s.fsub(one, left)?;
    let mist = sky(s, &lit, eye.unit, false)?;
    let hazed = mix(s, colour, mist, fog)?;
    let colour = pick_v(s, has_first, hazed, colour)?;

    let half = s.f(0.5)?;
    let fx = s.real(x)?;
    let fy = s.real(y)?;
    let mid_x = s.f(look.screen.wide as f32 * 0.5)?;
    let mid_y = s.f(look.screen.high as f32 * 0.5)?;
    let dx = s.fadd(fx, half)?;
    let dx = s.fsub(dx, mid_x)?;
    let dy = s.fadd(fy, half)?;
    let dy = s.fsub(dy, mid_y)?;
    let dx = s.fmul(dx, dx)?;
    let dy = s.fmul(dy, dy)?;
    let r = s.fadd(dx, dy)?;
    let corner = look.screen.wide as f32 * look.screen.wide as f32 * 0.25
        + look.screen.high as f32 * look.screen.high as f32 * 0.25;
    let corner = s.f(VIGNETTE / corner.max(1.0))?;
    let dark = s.fmul(r, corner)?;
    let vignette = s.fsub(one, dark)?;
    let most = s.f(255.0)?;
    let wide = s.u(look.screen.wide)?;
    let seat = s.mul(y, wide)?;
    let seat = s.add(seat, x)?;
    let noise = hash(s, seat)?;
    let noise = s.and(noise, 0xFF)?;
    let noise = s.real(noise)?;
    let step = s.f(1.0 / 256.0)?;
    let noise = s.fmul(noise, step)?;
    let mut packed = s.u(0)?;
    for (channel, shift) in [(0, 16), (1, 8), (2, 0)] {
        let value = s.fmul(of(colour, channel), lit.exposure)?;
        let value = tone(s, value)?;
        let value = s.fmul(value, vignette)?;
        let value = s.fmul(value, most)?;
        let value = s.fadd(value, noise)?;
        let value = s.fmin(value, most)?;
        let value = s.fmax(value, zero)?;
        let value = s.whole(value)?;
        let value = s.shl(value, shift)?;
        packed = s.or(packed, value)?;
    }
    Ok(packed)
}

/// The CPU side of [`Shader`].
pub(crate) struct Plainly<'held> {
    pub world: World<'held>,
    pub pixels: &'held [u32],
}

impl Shader for Plainly<'_> {
    type W = u32;
    type R = f32;
    type T = bool;

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
    fn cell(&mut self, at: u32) -> Result<u32, LaneError> {
        Ok(self.world.cells.get(at as usize).copied().unwrap_or(0))
    }
    fn fixed(&mut self, at: u32) -> Result<u32, LaneError> {
        Ok(self.pixels.get(at as usize).copied().unwrap_or(0))
    }
    fn pixel(&mut self, at: u32) -> Result<u32, LaneError> {
        Ok(self.pixels.get(at as usize).copied().unwrap_or(0))
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
    fn cell(&mut self, at: Id) -> Result<Id, LaneError> {
        self.em.load(STATE, at)
    }
    fn fixed(&mut self, at: u32) -> Result<Id, LaneError> {
        self.em.load_u(PIXELS, at)
    }
    fn pixel(&mut self, at: Id) -> Result<Id, LaneError> {
        self.em.load(PIXELS, at)
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
}

fn kernel(subgroup: u32) -> Result<simdr::kernel::Kernel<simdr::lanes::U32>, LaneError> {
    Kernel::new(Shape::new(subgroup, subgroup, BINDINGS))
}

fn look_of(build: &Build) -> Look {
    Look {
        grid: build.grid,
        kinds: build.kinds,
        screen: build.screen,
        layout: build.layout(),
        suns: build.suns.clamp(1, super::SUNS as u32),
    }
}

/// The pixel a lane works on, whether it is on the screen, and its `x` and
/// `y`.
fn placed(em: &mut Em<'_>, build: &Build) -> Result<(Id, Id, Id, Id), LaneError> {
    let pixel = em.invocation(build.subgroup)?;
    let count = em.u(word(build.screen.pixels()))?;
    let shown = em.lt(pixel, count)?;
    let wide = em.u(build.screen.wide)?;
    let y = em.div(pixel, wide)?;
    let row = em.mul(y, wide)?;
    let x = em.sub(pixel, row)?;
    Ok((pixel, shown, x, y))
}

/// The pass that marches every pixel's ray and writes its four hit words
/// from [`Layout::hits`] on, as [`World::seen`] finds them.
pub fn primary(build: &Build) -> Result<Vec<u32>, LaneError> {
    refuse("render3::primary", build.grid, build.screen)?;
    let layout = build.layout();
    let grid = build.grid;
    let look = look_of(build);
    let reader = Reader {
        grid,
        kinds: build.kinds,
        bricks_at: word(layout.occupancy()),
    };
    let mut kernel = kernel(build.subgroup)?;
    let mut em = Em::new(&mut kernel)?;
    let (pixel, shown, x, y) = placed(&mut em, build)?;
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

    let start = Marching::from(&mut em, origin, direction)?;
    let (start, alive) = start.entered(&mut em, sides(grid))?;
    let alive = em.both(alive, shown)?;
    let alive = em.flag(alive)?;
    let none = em.u(0)?;
    let miss = em.u(MISS)?;
    let mut initial = vec![alive, none, miss, none, miss, none];
    initial.extend(start.carried());
    let out = em.looped(super::march::limit(grid), &initial, |em, words, run| {
        let get = |at: usize| words.get(at).copied().unwrap_or(none);
        let ray = start.with(words.get(6..).unwrap_or(&[]));
        let through = em.ne_u(get(1), 0)?;
        let (cell, kind, full) = reader.read(em, &ray)?;
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
        let through = em.either(through, enters)?;
        let through = em.flag(through)?;
        let moved = ray.advanced(em, full)?;
        let inside = moved.inside(em, sides(grid))?;
        let going = em.not(ends)?;
        let going = em.both(going, inside)?;
        let going = em.flag(going)?;
        let mut next = vec![going, through, first, first_info, behind, behind_info];
        next.extend(moved.carried());
        Ok(next)
    })?;
    let hits = em.u(word(layout.hits()))?;
    let spare = em.u(word(layout.spare()))?;
    let count = em.u(word(build.screen.pixels()))?;
    let past = em.sub(pixel, count)?;
    let four = em.u(word(HIT_WORDS))?;
    let at = em.mul(pixel, four)?;
    let at = em.add(hits, at)?;
    let past = em.mul(past, four)?;
    let past = em.add(spare, past)?;
    let at = em.pick(shown, at, past)?;
    for slot in 0..HIT_WORDS {
        let step = em.u(word(slot))?;
        let into = em.add(at, step)?;
        let value = out.get(slot + 2).copied().unwrap_or(none);
        em.store(PIXELS, into, value)?;
    }
    kernel.finish()
}

/// The pass that lights every pixel from its hit words and writes its
/// colour, `0x00RRGGBB`, into the first [`Screen::pixels`] words.
pub fn shade(build: &Build) -> Result<Vec<u32>, LaneError> {
    refuse("render3::shade", build.grid, build.screen)?;
    let layout = build.layout();
    let look = look_of(build);
    let reader = Reader {
        grid: build.grid,
        kinds: build.kinds,
        bricks_at: word(layout.occupancy()),
    };
    let mut kernel = kernel(build.subgroup)?;
    let mut em = Em::new(&mut kernel)?;
    let (pixel, shown, x, y) = placed(&mut em, build)?;
    let none = em.u(0)?;
    let seat = em.pick(shown, pixel, none)?;
    let four = em.u(word(HIT_WORDS))?;
    let at = em.mul(seat, four)?;
    let hits = em.u(word(layout.hits()))?;
    let at = em.add(hits, at)?;
    let mut hit = [none; 4];
    for (slot, value) in hit.iter_mut().enumerate() {
        let step = em.u(word(slot))?;
        let from = em.add(at, step)?;
        *value = em.load(PIXELS, from)?;
    }
    let colour = {
        let mut plain = Emitted {
            em: &mut em,
            reader,
        };
        colour(&mut plain, &look, (x, y), hit)?
    };
    let spare = em.u(word(layout.spare()))?;
    let count = em.u(word(build.screen.pixels()))?;
    let past = em.sub(pixel, count)?;
    let past = em.add(spare, past)?;
    let into = em.pick(shown, pixel, past)?;
    em.store(PIXELS, into, colour)?;
    kernel.finish()
}

/// The picture on the CPU: the rays, the light, the materials and the sun
/// rays a frame uses, over a world.
#[derive(Debug, Clone)]
pub struct Shading<'held> {
    pub world: World<'held>,
    pub screen: Screen,
    pub layout: Layout,
    /// `PIXELS` as the passes read it: at least up to the end of the scene.
    pub pixels: &'held [u32],
    pub suns: u32,
}

impl Shading<'_> {
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

    /// The colour of every pixel, as [`shade`] writes it, from the hit
    /// words `hits`.
    pub fn picture(&self, hits: &[u32]) -> Result<Vec<u32>, LaneError> {
        let look = Look {
            grid: self.world.grid,
            kinds: self.world.kinds,
            screen: self.screen,
            layout: self.layout,
            suns: self.suns.clamp(1, super::SUNS as u32),
        };
        let mut plain = Plainly {
            world: self.world,
            pixels: self.pixels,
        };
        let mut out = Vec::with_capacity(self.screen.pixels());
        for (at, hit) in hits.chunks(HIT_WORDS).enumerate() {
            let at = word(at);
            let x = at % self.screen.wide;
            let y = at / self.screen.wide;
            let get = |slot: usize| hit.get(slot).copied().unwrap_or(MISS);
            out.push(colour(
                &mut plain,
                &look,
                (x, y),
                [get(0), get(1), get(2), get(3)],
            )?);
        }
        Ok(out)
    }
}
