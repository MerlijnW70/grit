//! The fine detail of surfaces and water, written once over [`Shader`] so
//! the CPU runs the same formulas: a smooth value noise and its slope, the
//! sun's glint off a surface by its roughness, the two rays a liquid's top
//! sends, in whole numbers, and the cheap light of what a mirrored ray
//! meets.

use simdr::lanes::LaneError;

use super::shade::{
    Eye, Lit, Look, Shader, V, dot, hash, kind_of, plus, point, scaled, stuff, v_map, v_zip,
};
use super::{BOUNCED, MISS, UNIT, UNIT_BITS, material, of, put, scene, word};

/// The factors that spread a lattice point's axes over a word.
const SPREAD: [u32; 3] = [0x9E37_79B1, 0x85EB_CA77, 0xC2B2_AE3D];

/// The side of a ripple's lattice cell, in sixteenths of a cell.
const RIPPLE_BITS: u32 = 6;

/// The frames of the draw count a ripple moves one sixteenth in.
const RIPPLE_PACE: u32 = 2;

/// A noise value in [0, 1] and its slope along each axis.
pub(crate) struct Noise<S: Shader> {
    pub value: S::R,
    pub slope: V<S::R>,
}

/// Smooth value noise at `at`, with lattice points one apart, and its slope.
pub(crate) fn noise<S: Shader>(s: &mut S, at: V<S::R>, seed: u32) -> Result<Noise<S>, LaneError> {
    let zero = s.f(0.0)?;
    let one = s.f(1.0)?;
    let two = s.f(2.0)?;
    let three = s.f(3.0)?;
    let six = s.f(6.0)?;
    let mut base = s.u(seed)?;
    let mut eased = [zero; 3];
    let mut rate = [zero; 3];
    for axis in 0..3 {
        let value = of(at, axis);
        let low = s.floor(value)?;
        let part = s.fsub(value, low)?;
        let lattice = s.whole_signed(low)?;
        let factor = s.u(of(SPREAD, axis))?;
        let spread = s.mul(lattice, factor)?;
        base = s.add(base, spread)?;
        let twice = s.fmul(part, two)?;
        let rest = s.fsub(three, twice)?;
        let square = s.fmul(part, part)?;
        put(&mut eased, axis, s.fmul(square, rest)?);
        let left = s.fsub(one, part)?;
        let product = s.fmul(part, left)?;
        put(&mut rate, axis, s.fmul(product, six)?);
    }
    let scale = s.f(1.0 / 65535.0)?;
    let mut corners = [zero; 8];
    for (corner, slot) in corners.iter_mut().enumerate() {
        let offset = (0..3)
            .filter(|axis| (corner >> axis) & 1 == 1)
            .fold(0_u32, |sum, axis| sum.wrapping_add(of(SPREAD, axis)));
        let at = plus(s, base, offset)?;
        let mixed = hash(s, at)?;
        let mixed = s.and(mixed, 0xFFFF)?;
        let mixed = s.real(mixed)?;
        *slot = s.fmul(mixed, scale)?;
    }
    let [a, b, c, d, e, f, g, h] = corners;
    let k1 = s.fsub(b, a)?;
    let k2 = s.fsub(c, a)?;
    let k3 = s.fsub(e, a)?;
    let ab = s.fsub(a, b)?;
    let k4 = s.fsub(ab, c)?;
    let k4 = s.fadd(k4, d)?;
    let ac = s.fsub(a, c)?;
    let k5 = s.fsub(ac, e)?;
    let k5 = s.fadd(k5, g)?;
    let k6 = s.fsub(ab, e)?;
    let k6 = s.fadd(k6, f)?;
    let k7 = s.fsub(b, a)?;
    let k7 = s.fadd(k7, c)?;
    let k7 = s.fsub(k7, d)?;
    let k7 = s.fadd(k7, e)?;
    let k7 = s.fsub(k7, f)?;
    let k7 = s.fsub(k7, g)?;
    let k7 = s.fadd(k7, h)?;
    let [ux, uy, uz] = eased;
    let xy = s.fmul(ux, uy)?;
    let yz = s.fmul(uy, uz)?;
    let zx = s.fmul(uz, ux)?;
    let xyz = s.fmul(xy, uz)?;
    let mut value = a;
    for (k, u) in [
        (k1, ux),
        (k2, uy),
        (k3, uz),
        (k4, xy),
        (k5, yz),
        (k6, zx),
        (k7, xyz),
    ] {
        let term = s.fmul(k, u)?;
        value = s.fadd(value, term)?;
    }
    let mut slope = [zero; 3];
    for (axis, (first, (k_a, u_a), (k_b, u_b), pair)) in [
        (k1, (k4, uy), (k6, uz), yz),
        (k2, (k5, uz), (k4, ux), zx),
        (k3, (k6, ux), (k5, uy), xy),
    ]
    .into_iter()
    .enumerate()
    {
        let one_side = s.fmul(k_a, u_a)?;
        let other = s.fmul(k_b, u_b)?;
        let corner = s.fmul(k7, pair)?;
        let sum = s.fadd(first, one_side)?;
        let sum = s.fadd(sum, other)?;
        let sum = s.fadd(sum, corner)?;
        put(&mut slope, axis, s.fmul(sum, of(rate, axis))?);
    }
    Ok(Noise { value, slope })
}

/// [`noise`] at `at` scaled by `scale`, its slope given along the unscaled
/// axes.
pub(crate) fn noise_at<S: Shader>(
    s: &mut S,
    at: V<S::R>,
    scale: V<S::R>,
    seed: u32,
) -> Result<Noise<S>, LaneError> {
    let scaled_at = v_zip(s, at, scale, S::fmul)?;
    let found = noise(s, scaled_at, seed)?;
    let slope = v_zip(s, found.slope, scale, S::fmul)?;
    Ok(Noise {
        value: found.value,
        slope,
    })
}

/// `value` made one long.
pub(crate) fn unit<S: Shader>(s: &mut S, value: V<S::R>) -> Result<V<S::R>, LaneError> {
    let squared = dot(s, value, value)?;
    let tiny = s.f(1.0e-12)?;
    let squared = s.fmax(squared, tiny)?;
    let length = s.sqrt(squared)?;
    v_map(s, value, |s, part| s.fdiv(part, length))
}

/// The share of light reflected at `cosine` from the normal, by Schlick's
/// rule, for a surface that reflects `shine` of it head on.
pub(crate) fn schlick<S: Shader>(s: &mut S, cosine: S::R, shine: S::R) -> Result<S::R, LaneError> {
    let one = s.f(1.0)?;
    let zero = s.f(0.0)?;
    let cosine = s.fmax(cosine, zero)?;
    let rest = s.fsub(one, cosine)?;
    let rest = s.fmin(rest, one)?;
    let square = s.fmul(rest, rest)?;
    let fourth = s.fmul(square, square)?;
    let fifth = s.fmul(fourth, rest)?;
    let left = s.fsub(one, shine)?;
    let part = s.fmul(left, fifth)?;
    s.fadd(shine, part)
}

/// The most a glint may give, as a share of the sun.
const GLINT_MOST: f32 = 40.0;

/// The light of the sun from `toward` that a surface of normal `normal`,
/// roughness `rough` and head-on reflectance `shine` sends to an eye that
/// lies along `back`, for each unit of sun: a GGX spread of the
/// reflection, Schlick's share of it, and Smith's shadowing of its
/// roughness.
pub(crate) fn glint<S: Shader>(
    s: &mut S,
    normal: V<S::R>,
    back: V<S::R>,
    toward: V<S::R>,
    (rough, shine): (S::R, S::R),
) -> Result<S::R, LaneError> {
    let zero = s.f(0.0)?;
    let one = s.f(1.0)?;
    let half = v_zip(s, toward, back, S::fadd)?;
    let half = unit(s, half)?;
    let lit = dot(s, normal, toward)?;
    let lit = s.fmax(lit, zero)?;
    let seen = dot(s, normal, back)?;
    let least = s.f(1.0e-3)?;
    let seen = s.fmax(seen, least)?;
    let middle = dot(s, normal, half)?;
    let middle = s.fmax(middle, zero)?;
    let turned = dot(s, back, half)?;
    let smallest = s.f(0.03)?;
    let rough = s.fmax(rough, smallest)?;
    let alpha = s.fmul(rough, rough)?;
    let alpha2 = s.fmul(alpha, alpha)?;
    let less = s.fsub(alpha2, one)?;
    let square = s.fmul(middle, middle)?;
    let spread = s.fmul(square, less)?;
    let spread = s.fadd(spread, one)?;
    let spread = s.fmul(spread, spread)?;
    let pi = s.f(std::f32::consts::PI)?;
    let spread = s.fmul(spread, pi)?;
    let spread = s.fdiv(alpha2, spread)?;
    let fresnel = schlick(s, turned, shine)?;
    let half_alpha = s.f(0.5)?;
    let k = s.fmul(alpha, half_alpha)?;
    let keep = s.fsub(one, k)?;
    let mut shadowed = one;
    for cosine in [lit, seen] {
        let under = s.fmul(cosine, keep)?;
        let under = s.fadd(under, k)?;
        let part = s.fdiv(cosine, under)?;
        shadowed = s.fmul(shadowed, part)?;
    }
    let four = s.f(4.0)?;
    let below = s.fmul(seen, four)?;
    let value = s.fmul(spread, fresnel)?;
    let value = s.fmul(value, shadowed)?;
    let value = s.fdiv(value, below)?;
    let most = s.f(GLINT_MOST)?;
    s.fmin(value, most)
}

/// `way` mirrored across the plane of the unit `normal`.
pub(crate) fn mirrored<S: Shader>(
    s: &mut S,
    way: V<S::R>,
    normal: V<S::R>,
) -> Result<V<S::R>, LaneError> {
    let along = dot(s, way, normal)?;
    let two = s.f(2.0)?;
    let along = s.fmul(along, two)?;
    let push = scaled(s, normal, along)?;
    v_zip(s, way, push, S::fsub)
}

/// The two rays the top of a liquid sends from a pixel, in whole numbers:
/// the point it was met at in sixteenths, and the start and direction of
/// the mirrored ray and of the bent ray.
pub(crate) struct Bounce<S: Shader> {
    pub met: V<S::W>,
    pub mirror: (V<S::W>, V<S::W>),
    pub bent: (V<S::W>, V<S::W>),
}

/// The size of a two's complement word, and whether it is below zero.
fn size_of<S: Shader>(s: &mut S, value: S::W) -> Result<(S::W, S::T), LaneError> {
    let sign = s.u(1 << 31)?;
    let positive = s.lt(value, sign)?;
    let below = s.not(positive)?;
    let none = s.u(0)?;
    let flipped = s.sub(none, value)?;
    Ok((s.pick(below, flipped, value)?, below))
}

/// `size` given the sign `below`.
fn signed_of<S: Shader>(s: &mut S, size: S::W, below: S::T) -> Result<S::W, LaneError> {
    let none = s.u(0)?;
    let flipped = s.sub(none, size)?;
    s.pick(below, flipped, size)
}

/// A ripple at the point `(across, deep)` of a liquid's top, in sixteenths,
/// in whole numbers: a value noise of two drifting lattices, from 0 to 255.
fn ripple<S: Shader>(
    s: &mut S,
    (across, deep): (S::W, S::W),
    drawn: S::W,
    seed: u32,
) -> Result<S::W, LaneError> {
    let mut sum = s.u(0)?;
    for (layer, (bits, drift_x, drift_z)) in [(RIPPLE_BITS, 1, 0), (RIPPLE_BITS - 1, 0, 1)]
        .into_iter()
        .enumerate()
    {
        let pace = s.shr(drawn, RIPPLE_PACE + layer as u32)?;
        let x = if drift_x == 1 {
            s.add(across, pace)?
        } else {
            s.sub(across, pace)?
        };
        let z = if drift_z == 1 {
            s.add(deep, pace)?
        } else {
            s.sub(deep, pace)?
        };
        let side = 1 << bits;
        let ix = s.shr(x, bits)?;
        let iz = s.shr(z, bits)?;
        let fx = s.and(x, side - 1)?;
        let fz = s.and(z, side - 1)?;
        let full = s.u(side)?;
        let gx = s.sub(full, fx)?;
        let gz = s.sub(full, fz)?;
        let spread_x = s.u(of(SPREAD, 0))?;
        let spread_z = s.u(of(SPREAD, 2))?;
        let key = s.mul(ix, spread_x)?;
        let row = s.mul(iz, spread_z)?;
        let key = s.add(key, row)?;
        let key = plus(s, key, seed.wrapping_add(layer as u32 * 0x1F3D))?;
        let mut corners = [key; 4];
        for (at, slot) in corners.iter_mut().enumerate() {
            let mut offset = 0_u32;
            if at & 1 == 1 {
                offset = offset.wrapping_add(of(SPREAD, 0));
            }
            if at & 2 == 2 {
                offset = offset.wrapping_add(of(SPREAD, 2));
            }
            let here = plus(s, key, offset)?;
            let mixed = hash(s, here)?;
            *slot = s.and(mixed, 0xFF)?;
        }
        let [v00, v10, v01, v11] = corners;
        let mut rows = [v00; 2];
        for (slot, (left, right)) in rows.iter_mut().zip([(v00, v10), (v01, v11)]) {
            let left = s.mul(left, gx)?;
            let right = s.mul(right, fx)?;
            *slot = s.add(left, right)?;
        }
        let top = s.mul(rows[0], gz)?;
        let bottom = s.mul(rows[1], fz)?;
        let value = s.add(top, bottom)?;
        let value = s.shr(value, 2 * bits)?;
        sum = s.add(sum, value)?;
    }
    s.shr(sum, 1)
}

/// How far, in whole numbers, a ripple of `value` turns a direction whose
/// largest part is `most`.
fn turned<S: Shader>(s: &mut S, value: S::W, most: S::W) -> Result<S::W, LaneError> {
    let step = s.shr(most, 9)?;
    let up = s.mul(value, step)?;
    let up = s.shr(up, 3)?;
    let middle = s.u(128)?;
    let down = s.mul(middle, step)?;
    let down = s.shr(down, 3)?;
    s.sub(up, down)
}

/// Whether a pixel whose first hit is `first`, met by face `info`, sends
/// the rays of a liquid's top: the hit is the top face of a kind in the
/// scene's liquids.
pub(crate) fn sends<S: Shader>(
    s: &mut S,
    look: &Look,
    (first, info): (S::W, S::W),
) -> Result<S::T, LaneError> {
    let base = word(look.layout.scene());
    let miss = s.u(MISS)?;
    let none = s.u(0)?;
    let missed = s.eq(first, miss)?;
    let has = s.not(missed)?;
    let cell = s.pick(missed, none, first)?;
    let face = s.and(info, 7)?;
    let top_face = s.u(2)?;
    let top = s.eq(face, top_face)?;
    let liquids = s.fixed(base + word(scene::LIQUID))?;
    let held = s.cell(cell)?;
    let kind = kind_of(s, look.kinds, held)?;
    let liquid = s.bit(liquids, kind)?;
    let sent = s.both(has, top)?;
    s.both(sent, liquid)
}

/// The rays the top of a liquid sends from pixel `(x, y)` whose first hit
/// is `first`, met by face `info`, for a pixel that [`sends`] them. The
/// point met is found in whole numbers from the ray's start and direction,
/// kept on the face; the mirrored ray starts a sixteenth above it with its
/// fall turned up, the bent ray starts on it with its sideways run cut to
/// three quarters; both are turned by a ripple that drifts with the draw
/// count, the bent ray by half as much, when `rippled`; the light, which
/// only needs where along them the cells they met lie, leaves the ripple
/// out. Every part of a direction is at
/// most half the camera ray's largest part and a sixteenth of it, so the
/// march stays exact.
pub(crate) fn bounce_rays<S: Shader>(
    s: &mut S,
    look: &Look,
    (x, y): (S::W, S::W),
    (first, _info): (S::W, S::W),
    rippled: bool,
) -> Result<Bounce<S>, LaneError> {
    let base = word(look.layout.scene());
    let mut origin = [first; 3];
    for (axis, slot) in origin.iter_mut().enumerate() {
        *slot = s.fixed(base + word(scene::ORIGIN + axis))?;
    }
    let direction = super::shade::direction_words(s, look, (x, y))?;
    let miss = s.u(MISS)?;
    let none = s.u(0)?;
    let missed = s.eq(first, miss)?;
    let cell = s.pick(missed, none, first)?;
    let at = point(s, look.grid, cell)?;
    let mut sizes = [none; 3];
    let mut below = [missed; 3];
    for axis in 0..3 {
        let (size, sign) = size_of(s, of(direction, axis))?;
        put(&mut sizes, axis, size);
        put(&mut below, axis, sign);
    }
    let [size_x, size_y, size_z] = sizes;
    let most = {
        let bigger = s.lt(size_x, size_y)?;
        let most = s.pick(bigger, size_y, size_x)?;
        let bigger = s.lt(most, size_z)?;
        s.pick(bigger, size_z, most)?
    };
    let surface = s.shl(of(at, 1), UNIT_BITS)?;
    let fall = s.sub(surface, of(origin, 1))?;
    let over = s.shr(size_y, 3)?;
    let zero_over = s.eq(over, none)?;
    let one = s.u(1)?;
    let over = s.pick(zero_over, one, over)?;
    let sign_bit = s.u(1 << 31)?;
    let last = s.u(UNIT - 1)?;
    let mut met = [surface; 3];
    for axis in [0, 2] {
        let run = s.shr(of(sizes, axis), 3)?;
        let run = s.mul(run, fall)?;
        let run = s.div(run, over)?;
        let ahead = s.add(of(origin, axis), run)?;
        let back = s.sub(of(origin, axis), run)?;
        let reached = s.pick(of(below, axis), back, ahead)?;
        let start = s.shl(of(at, axis), UNIT_BITS)?;
        let into = s.sub(reached, start)?;
        let positive = s.lt(into, sign_bit)?;
        let past = s.lt(last, into)?;
        let into = s.pick(past, last, into)?;
        let into = s.pick(positive, into, none)?;
        put(&mut met, axis, s.add(start, into)?);
    }
    let drawn = s.fixed(base + word(scene::DRAWN))?;
    let place = (of(met, 0), of(met, 2));
    let mut mirror_way = [none; 3];
    let mut bent_way = [none; 3];
    let gentle = s.shr(most, 1)?;
    for (axis, seed) in [(0, 0x51A7_E001_u32), (2, 0x51A7_E002)] {
        let size = of(sizes, axis);
        let sign = of(below, axis);
        let half = s.shr(size, 1)?;
        let half = signed_of(s, half, sign)?;
        let three = s.u(3)?;
        let cut = s.mul(size, three)?;
        let cut = s.shr(cut, 3)?;
        let cut = signed_of(s, cut, sign)?;
        if rippled {
            let wave = ripple(s, place, drawn, seed)?;
            let tilt = turned(s, wave, most)?;
            put(&mut mirror_way, axis, s.add(half, tilt)?);
            let tilt = turned(s, wave, gentle)?;
            put(&mut bent_way, axis, s.add(cut, tilt)?);
        } else {
            put(&mut mirror_way, axis, half);
            put(&mut bent_way, axis, cut);
        }
    }
    let half_fall = s.shr(size_y, 1)?;
    put(&mut mirror_way, 1, s.sub(none, half_fall)?);
    put(&mut bent_way, 1, half_fall);
    let mut above = met;
    put(&mut above, 1, s.sub(surface, one)?);
    Ok(Bounce {
        met,
        mirror: (above, mirror_way),
        bent: (met, bent_way),
    })
}

/// The four words of the bounced rays of pixel `(x, y)`: the cell the
/// mirrored ray meets first of the kinds seen and its face, with
/// [`BOUNCED`] set when the pixel sent its rays, then the cell the bent
/// ray meets first of the kinds nobody sees through, and its face. A pixel
/// sends no rays where the light is not rich.
pub(crate) fn bounced_words<S: Shader>(
    s: &mut S,
    look: &Look,
    at: (S::W, S::W),
    hit: (S::W, S::W),
) -> Result<[S::W; 4], LaneError> {
    let miss = s.u(MISS)?;
    let none = s.u(0)?;
    if !look.rich() {
        return Ok([miss, none, miss, none]);
    }
    let sent = sends(s, look, hit)?;
    let base = word(look.layout.scene());
    let visible = s.fixed(base + word(scene::VISIBLE))?;
    let opaque = s.fixed(base + word(scene::OPAQUE))?;
    let gate = s.open(sent)?;
    let rays = bounce_rays(s, look, at, hit, true)?;
    let (mirror_cell, mirror_face) = s.found(visible, rays.mirror.0, rays.mirror.1, sent)?;
    let (bent_cell, bent_face) = s.found(opaque, rays.bent.0, rays.bent.1, sent)?;
    let flag = s.u(BOUNCED)?;
    let mirror_face = s.or(mirror_face, flag)?;
    let given = [mirror_cell, mirror_face, bent_cell, bent_face];
    let words = s.close_words(gate, &given, &[miss, none, miss, none])?;
    let take = |at: usize| words.get(at).copied().unwrap_or(none);
    Ok([take(0), take(1), take(2), take(3)])
}

/// The eye of a ray that starts at `origin`, in sixteenths, and runs along
/// `direction`, both whole numbers.
pub(crate) fn eye_of<S: Shader>(
    s: &mut S,
    (origin, direction): (V<S::W>, V<S::W>),
) -> Result<Eye<S>, LaneError> {
    let sixteenth = s.f(1.0 / UNIT as f32)?;
    let mut start = [sixteenth; 3];
    let mut way = [sixteenth; 3];
    for axis in 0..3 {
        let held = s.signed(of(origin, axis))?;
        put(&mut start, axis, s.fmul(held, sixteenth)?);
        put(&mut way, axis, s.signed(of(direction, axis))?);
    }
    let squared = dot(s, way, way)?;
    let tiny = s.f(1.0e-12)?;
    let squared = s.fmax(squared, tiny)?;
    let length = s.sqrt(squared)?;
    let unit_way = v_map(s, way, |s, value| s.fdiv(value, length))?;
    Ok(Eye {
        origin: start,
        direction: way,
        length,
        unit: unit_way,
    })
}

/// The light a mirrored ray brings back from cell `cell`, met by face
/// `info` along `eye`: its colour lit by the sun and the sky, without
/// shadow or detail, and faded into the haze with its distance.
pub(crate) fn glance<S: Shader>(
    s: &mut S,
    look: &Look,
    lit: &Lit<S>,
    eye: &Eye<S>,
    (cell, info): (S::W, S::W),
) -> Result<V<S::R>, LaneError> {
    let held = s.cell(cell)?;
    let kind = kind_of(s, look.kinds, held)?;
    let made = stuff(s, look, kind)?;
    let table = s.u(word(look.layout.table()))?;
    let size = s.u(word(super::MATERIAL_WORDS))?;
    let offset = s.mul(kind, size)?;
    let at = s.add(table, offset)?;
    let liquid_at = plus(s, at, word(material::LIQUID))?;
    let liquid = s.pixel(liquid_at)?;
    let none = s.u(0)?;
    let liquid = s.eq(liquid, none)?;
    let liquid = s.not(liquid)?;
    let mut deep = [made.clear; 3];
    for (channel, slot) in deep.iter_mut().enumerate() {
        let here = plus(s, at, word(material::DEEP + channel))?;
        *slot = super::shade::real_of(s, here)?;
    }
    let albedo = super::shade::pick_v(s, liquid, deep, made.albedo)?;
    let mixed = hash(s, cell)?;
    let mixed = s.and(mixed, 0xFF)?;
    let mixed = s.real(mixed)?;
    let byte = s.f(1.0 / 255.0)?;
    let mixed = s.fmul(mixed, byte)?;
    let half = s.f(0.5)?;
    let mixed = s.fsub(mixed, half)?;
    let shift = s.fmul(mixed, made.jitter)?;
    let one = s.f(1.0)?;
    let factor = s.fadd(one, shift)?;
    let face = s.and(info, 7)?;
    let axis = s.shr(face, 1)?;
    let odd = s.and(face, 1)?;
    let odd_face = s.u(1)?;
    let up = s.eq(odd, odd_face)?;
    let zero = s.f(0.0)?;
    let minus = s.f(-1.0)?;
    let sign = s.pick_r(up, one, minus)?;
    let mut normal = [zero; 3];
    for (which, slot) in normal.iter_mut().enumerate() {
        let wanted = s.u(which as u32)?;
        let this = s.eq(axis, wanted)?;
        *slot = s.pick_r(this, sign, zero)?;
    }
    let facing = dot(s, normal, lit.toward)?;
    let facing = s.fmax(facing, zero)?;
    let up_ness = s.fsub(zero, normal[1])?;
    let sky_part = s.f(0.15)?;
    let sky_base = s.f(0.22)?;
    let sky_amount = s.fmul(up_ness, sky_part)?;
    let sky_amount = s.fadd(sky_amount, sky_base)?;
    let at_point = point(s, look.grid, cell)?;
    let mut far = zero;
    for which in 0..3 {
        let side = s.real(of(at_point, which))?;
        let lift = s.pick_r(up, one, zero)?;
        let plane = s.fadd(side, lift)?;
        let gap = s.fsub(plane, of(eye.origin, which))?;
        let here = s.fdiv(gap, of(eye.direction, which))?;
        let wanted = s.u(which as u32)?;
        let this = s.eq(axis, wanted)?;
        far = s.pick_r(this, here, far)?;
    }
    let distance = s.fmul(far, eye.length)?;
    let distance = s.fmax(distance, zero)?;
    let mut colour = [zero; 3];
    for (channel, slot) in colour.iter_mut().enumerate() {
        let sun = s.fmul(of(lit.sun, channel), facing)?;
        let fill = s.fadd(of(lit.zenith, channel), of(lit.horizon, channel))?;
        let fill = s.fmul(fill, sky_amount)?;
        let light = s.fadd(sun, fill)?;
        let own = s.fmul(of(albedo, channel), factor)?;
        let shade = s.fmul(own, light)?;
        let glow = s.fmul(of(albedo, channel), made.glow)?;
        *slot = s.fadd(shade, glow)?;
    }
    let thickness = s.fmul(distance, lit.haze)?;
    let thickness = s.fsub(zero, thickness)?;
    let left = s.exp(thickness)?;
    let haze = s.fsub(one, left)?;
    super::shade::mix(s, colour, lit.horizon, haze)
}

/// The draw count of the frame, in seconds at sixty a second.
pub(crate) fn clock<S: Shader>(s: &mut S, look: &Look) -> Result<S::R, LaneError> {
    let drawn = s.fixed(word(look.layout.scene() + scene::DRAWN))?;
    let drawn = s.real(drawn)?;
    let rate = s.f(1.0 / 60.0)?;
    s.fmul(drawn, rate)
}

/// The fields of a material beyond what [`stuff`] reads: its roughness,
/// head-on reflectance, detail, moss and its colour, how it wets, how it
/// glints, whether it is a liquid and the colour it deepens to.
pub(crate) struct Skin<S: Shader> {
    pub rough: S::R,
    pub shine: S::R,
    pub grain: S::R,
    pub bump: S::R,
    pub cracks: S::R,
    pub streak: S::R,
    pub moss: S::R,
    pub tint: V<S::R>,
    pub wets: S::R,
    pub sparkle: S::R,
    pub liquid: S::T,
    pub deep: V<S::R>,
}

pub(crate) fn skin<S: Shader>(s: &mut S, look: &Look, kind: S::W) -> Result<Skin<S>, LaneError> {
    let base = s.u(word(look.layout.table()))?;
    let size = s.u(word(super::MATERIAL_WORDS))?;
    let offset = s.mul(kind, size)?;
    let at = s.add(base, offset)?;
    let get = |s: &mut S, field: usize| -> Result<S::R, LaneError> {
        let here = plus(s, at, word(field))?;
        super::shade::real_of(s, here)
    };
    let liquid_at = plus(s, at, word(material::LIQUID))?;
    let liquid = s.pixel(liquid_at)?;
    let none = s.u(0)?;
    let dry = s.eq(liquid, none)?;
    let liquid = s.not(dry)?;
    Ok(Skin {
        rough: get(s, material::ROUGH)?,
        shine: get(s, material::SHINE)?,
        grain: get(s, material::GRAIN)?,
        bump: get(s, material::BUMP)?,
        cracks: get(s, material::CRACKS)?,
        streak: get(s, material::STREAK)?,
        moss: get(s, material::MOSS)?,
        tint: [
            get(s, material::TINT)?,
            get(s, material::TINT + 1)?,
            get(s, material::TINT + 2)?,
        ],
        wets: get(s, material::WETS)?,
        sparkle: get(s, material::SPARKLE)?,
        liquid,
        deep: [
            get(s, material::DEEP)?,
            get(s, material::DEEP + 1)?,
            get(s, material::DEEP + 2)?,
        ],
    })
}

/// The water of the bricks around `point`, blended between them: the
/// share of liquid, and the share of liquid open to the air beside it.
pub(crate) fn water_at<S: Shader>(
    s: &mut S,
    look: &Look,
    point: V<S::R>,
) -> Result<[S::R; 2], LaneError> {
    let read = super::shade::sampled(
        s,
        look.grid,
        word(look.layout.water()),
        word(super::WATER_WORDS),
        point,
    )?;
    let zero = s.f(0.0)?;
    Ok([
        read.first().copied().unwrap_or(zero),
        read.get(1).copied().unwrap_or(zero),
    ])
}
