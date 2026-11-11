//! What happens to each pixel's light after it is shaded: it is kept over
//! frames, its brightest part is blurred into a soft bloom, and it is mapped
//! to the screen.
//!
//! - [`resolve`] finds each pixel's point in the frame before, through the
//!   camera of that frame, and blends the light kept there, held within the
//!   light of its neighbours now so that what moved does not smear, with
//!   the light of this frame. A camera that holds still samples a slightly
//!   different point of each pixel every frame ([`super::Camera::rays_shifted`]),
//!   so edges and thin bands average out over frames. It keeps the glow of
//!   each pixel too, fading, so a glowing cell that moves leaves a short
//!   trail.
//! - [`bloom_down`] keeps the light past a threshold at a quarter of the
//!   screen on each side, [`bloom_across`] and [`bloom_up`] blur it.
//! - [`composite`] adds the trail and the bloom, maps the light to the
//!   screen, darkens the corners, and adds an ordered dither before it
//!   rounds each channel to 8 bits, so smooth gradients show no bands. The
//!   dither's threshold steps by the golden ratio's fraction, in sixteen
//!   bits, from one row to the next and by its square from one column to
//!   the next, so the thresholds down any column are spread evenly.
//!
//! Each pass is written once over one set of operations; [`develop`] runs them on the
//! CPU.

use simdr::lanes::LaneError;
use simdr::module::Id;

use super::emit::Em;
use super::shade::{
    Emitted, Look, Shader, Shading, V, direction_words, dot, kernel, lerp, lit, look_of, pick_v,
    placed, plus, reader_of, real_at, real_of, seat, v_map, v_zip,
};
use super::{
    BLOOM_SCALE, BLOOM_WORDS, Build, HDR_WORDS, HISTORY_WORDS, Layout, Screen, UNIT, flags, of,
    put, refuse, scene, word,
};
use crate::layout::PIXELS;

const GAMMA: f32 = 1.0 / 2.2;
const VIGNETTE: f32 = 0.16;
const BLUR_REACH: i32 = 6;
const BLUR_SPREAD: f32 = 3.0;
const TRAIL_SHOWN: f32 = 0.45;
const NEAREST: f32 = 1.0e-3;
const DITHER_DOWN: u32 = 40_503;
const DITHER_ACROSS: u32 = 49_471;

/// The weight of each pixel of the bloom blur, from six pixels before
/// to as far after, summing to 1.
#[must_use]
pub fn blur_weights() -> Vec<f32> {
    let raw: Vec<f32> = (-BLUR_REACH..=BLUR_REACH)
        .map(|at| (-((at * at) as f32) / (2.0 * BLUR_SPREAD * BLUR_SPREAD)).exp())
        .collect();
    let total: f32 = raw.iter().sum();
    raw.into_iter().map(|value| value / total).collect()
}

/// The pixel `(x, y)` moved by `(dx, dy)` and kept on `screen`, as an index.
fn kept_at<S: Shader>(
    s: &mut S,
    screen: Screen,
    (x, y): (S::W, S::W),
    (dx, dy): (i32, i32),
) -> Result<S::W, LaneError> {
    let mut held = [x, y];
    for (slot, (by, side)) in held.iter_mut().zip([(dx, screen.wide), (dy, screen.high)]) {
        let moved = s.u(by as u32)?;
        let moved = s.add(*slot, moved)?;
        let back = by < 0;
        let size = s.u(side)?;
        let inside = s.lt(moved, size)?;
        let edge = s.u(if back { 0 } else { side - 1 })?;
        *slot = s.pick(inside, moved, edge)?;
    }
    let [x, y] = held;
    let wide = s.u(screen.wide)?;
    let row = s.mul(y, wide)?;
    s.add(row, x)
}

/// The floats of a record of `size` words a pixel from `base`, at pixel
/// index `at`, fields `first` on, `count` of them.
fn record<S: Shader>(
    s: &mut S,
    (base, size): (usize, usize),
    at: S::W,
    first: usize,
    count: usize,
) -> Result<Vec<S::R>, LaneError> {
    let size = s.u(word(size))?;
    let start = s.mul(at, size)?;
    let base = s.u(word(base))?;
    let start = s.add(start, base)?;
    let mut out = Vec::with_capacity(count);
    for field in first..first + count {
        let here = plus(s, start, word(field))?;
        out.push(real_of(s, here)?);
    }
    Ok(out)
}

fn three<S: Shader>(s: &mut S, values: &[S::R]) -> Result<V<S::R>, LaneError> {
    let zero = s.f(0.0)?;
    Ok([0, 1, 2].map(|at| values.get(at).copied().unwrap_or(zero)))
}

fn frame_parity<S: Shader>(s: &mut S, look: &Look) -> Result<S::W, LaneError> {
    let drawn = s.fixed(word(look.layout.scene() + scene::DRAWN))?;
    s.and(drawn, 1)
}

/// A point in pixels of the frame before, and whether it lay in front of
/// that camera.
struct Before<S: Shader> {
    px: S::R,
    py: S::R,
    front: S::T,
}

/// Where, in pixels of the frame before, the point at `distance` along the
/// ray of pixel `(x, y)` was, and whether it was in front of that camera.
fn reprojected<S: Shader>(
    s: &mut S,
    look: &Look,
    (x, y): (S::W, S::W),
    distance: S::R,
) -> Result<Before<S>, LaneError> {
    let base = word(look.layout.scene());
    let sixteenth = s.f(1.0 / UNIT as f32)?;
    let mut origin = [sixteenth; 3];
    for (axis, slot) in origin.iter_mut().enumerate() {
        let held = s.fixed(base + word(super::scene::ORIGIN + axis))?;
        let held = s.signed(held)?;
        *slot = s.fmul(held, sixteenth)?;
    }
    let words = direction_words(s, look, (x, y))?;
    let direction = [
        s.signed(words[0])?,
        s.signed(words[1])?,
        s.signed(words[2])?,
    ];
    let squared = dot(s, direction, direction)?;
    let length = s.sqrt(squared)?;
    let unit = v_map(s, direction, |s, value| s.fdiv(value, length))?;
    let reach = v_map(s, unit, |s, value| s.fmul(value, distance))?;
    let point = v_zip(s, origin, reach, S::fadd)?;
    let before = base + word(scene::BEFORE);
    let mut camera = [[sixteenth; 3]; 4];
    for (at, slot) in camera.iter_mut().enumerate() {
        for (axis, value) in slot.iter_mut().enumerate() {
            *value = real_at(s, before + word(3 * at + axis))?;
        }
    }
    let [stood, forward, right, up] = camera;
    let apart = v_zip(s, point, stood, S::fsub)?;
    let ahead = dot(s, apart, forward)?;
    let nearest = s.f(NEAREST)?;
    let front = s.fgt(ahead, nearest)?;
    let ahead = s.fmax(ahead, nearest)?;
    let across = dot(s, apart, right)?;
    let across = s.fdiv(across, ahead)?;
    let down = dot(s, apart, up)?;
    let down = s.fdiv(down, ahead)?;
    let half = s.f(0.5)?;
    let wide = s.f(look.screen.wide as f32 - 1.0)?;
    let high = s.f(look.screen.high as f32 - 1.0)?;
    let px = s.fadd(across, wide)?;
    let px = s.fmul(px, half)?;
    let py = s.fsub(high, down)?;
    let py = s.fmul(py, half)?;
    Ok(Before { px, py, front })
}

/// A record of the frame before read at pixel `(px, y)` in floats,
/// blended between the four pixels around it.
fn blended<S: Shader>(
    s: &mut S,
    look: &Look,
    (base, size): (usize, usize),
    (px, py): (S::R, S::R),
    count: usize,
) -> Result<Vec<S::R>, LaneError> {
    let screen = look.screen;
    let zero = s.f(0.0)?;
    let one = s.f(1.0)?;
    let mut low = [s.u(0)?; 2];
    let mut high = low;
    let mut part = [zero; 2];
    for (at, (value, side)) in [(px, screen.wide), (py, screen.high)]
        .into_iter()
        .enumerate()
    {
        let below = s.floor(value)?;
        let rest = s.fsub(value, below)?;
        let last = s.f((side - 1) as f32)?;
        let first = s.fmax(below, zero)?;
        let first = s.fmin(first, last)?;
        let next = s.fadd(below, one)?;
        let next = s.fmax(next, zero)?;
        let next = s.fmin(next, last)?;
        if let (Some(l), Some(h), Some(p)) = (low.get_mut(at), high.get_mut(at), part.get_mut(at)) {
            *l = s.whole(first)?;
            *h = s.whole(next)?;
            *p = s.clamp01(rest)?;
        }
    }
    let wide = s.u(screen.wide)?;
    let mut out = vec![zero; count];
    for corner in 0..4_usize {
        let right = corner & 1 == 1;
        let lower = corner >> 1 == 1;
        let x = if right { high[0] } else { low[0] };
        let y = if lower { high[1] } else { low[1] };
        let wx = if right {
            part[0]
        } else {
            s.fsub(one, part[0])?
        };
        let wy = if lower {
            part[1]
        } else {
            s.fsub(one, part[1])?
        };
        let weight = s.fmul(wx, wy)?;
        let row = s.mul(y, wide)?;
        let at = s.add(row, x)?;
        let values = record(s, (base, size), at, 0, count)?;
        for (slot, value) in out.iter_mut().zip(values) {
            let value = s.fmul(value, weight)?;
            *slot = s.fadd(*slot, value)?;
        }
    }
    Ok(out)
}

/// The words [`resolve`] writes for pixel `(x, y)`: the light kept over
/// frames and the fading glow.
#[allow(clippy::too_many_lines)]
pub(crate) fn resolved<S: Shader>(
    s: &mut S,
    look: &Look,
    (x, y): (S::W, S::W),
) -> Result<Vec<S::W>, LaneError> {
    let layout = look.layout;
    let screen = look.screen;
    let lit = lit(s, look)?;
    let hdr = (layout.hdr(), HDR_WORDS);
    let here = kept_at(s, screen, (x, y), (0, 0))?;
    let now = record(s, hdr, here, 0, 7)?;
    let colour = three(s, now.get(0..3).unwrap_or(&[]))?;
    let glow = three(s, now.get(3..6).unwrap_or(&[]))?;
    let zero = s.f(0.0)?;
    let distance = now.get(6).copied().unwrap_or(zero);
    let mut least = colour;
    let mut most = colour;
    for dy in -1..=1 {
        for dx in -1..=1 {
            if dx == 0 && dy == 0 {
                continue;
            }
            let at = kept_at(s, screen, (x, y), (dx, dy))?;
            let near = record(s, hdr, at, 0, 3)?;
            let near = three(s, &near)?;
            least = v_zip(s, least, near, S::fmin)?;
            most = v_zip(s, most, near, S::fmax)?;
        }
    }
    let parity = frame_parity(s, look)?;
    let one_word = s.u(1)?;
    let other = s.xor(parity, one_word)?;
    let Before { px, py, front } = reprojected(s, look, (x, y), distance)?;
    let half = s.f(-0.5)?;
    let wide = s.f(screen.wide as f32 - 0.5)?;
    let high = s.f(screen.high as f32 - 0.5)?;
    let in_x = s.fgt(px, half)?;
    let in_x2 = s.fgt(wide, px)?;
    let in_y = s.fgt(py, half)?;
    let in_y2 = s.fgt(high, py)?;
    let valid = s.both(front, in_x)?;
    let valid = s.both(valid, in_x2)?;
    let valid = s.both(valid, in_y)?;
    let valid = s.both(valid, in_y2)?;
    let flagged = s.fixed(word(layout.scene() + scene::FLAGS))?;
    let reset = s.and(flagged, flags::RESET)?;
    let none = s.u(0)?;
    let kept = s.eq(reset, none)?;
    let valid = s.both(valid, kept)?;
    let one = s.f(1.0)?;
    let blend = s.pick_r(valid, lit.blend, one)?;
    let mut before_base = Vec::with_capacity(2);
    for parity_of in [0_u32, 1] {
        before_base.push(layout.history(parity_of));
    }
    let base_zero = before_base.first().copied().unwrap_or(0);
    let base_one = before_base.get(1).copied().unwrap_or(0);
    let from_zero = blended(s, look, (base_zero, HISTORY_WORDS), (px, py), HISTORY_WORDS)?;
    let from_one = blended(s, look, (base_one, HISTORY_WORDS), (px, py), HISTORY_WORDS)?;
    let zero_word = s.u(0)?;
    let other_is_zero = s.eq(other, zero_word)?;
    let mut past = Vec::with_capacity(HISTORY_WORDS);
    for (a, b) in from_zero.iter().zip(&from_one) {
        past.push(s.pick_r(other_is_zero, *a, *b)?);
    }
    let past_colour = three(s, past.get(0..3).unwrap_or(&[]))?;
    let past_trail = three(s, past.get(3..6).unwrap_or(&[]))?;
    let held = v_zip(s, past_colour, least, S::fmax)?;
    let held = v_zip(s, held, most, S::fmin)?;
    let mut kept_colour = [zero; 3];
    for channel in 0..3 {
        let value = lerp(s, of(held, channel), of(colour, channel), blend)?;
        put(&mut kept_colour, channel, value);
    }
    let fading = v_map(s, past_trail, |s, value| s.fmul(value, lit.trail))?;
    let fading = pick_v(s, valid, fading, [zero; 3])?;
    let trail = v_zip(s, fading, glow, S::fmax)?;
    let mut out = Vec::with_capacity(HISTORY_WORDS);
    for value in kept_colour.into_iter().chain(trail) {
        out.push(s.word_of(value)?);
    }
    Ok(out)
}

/// The light of pixel `(x, y)` as shown: the light kept over frames and the
/// glow left behind a moving glowing cell.
fn shown_light<S: Shader>(s: &mut S, look: &Look, at: S::W) -> Result<V<S::R>, LaneError> {
    let layout = look.layout;
    let parity = frame_parity(s, look)?;
    let zero_word = s.u(0)?;
    let is_zero = s.eq(parity, zero_word)?;
    let from_zero = record(s, (layout.history(0), HISTORY_WORDS), at, 0, HISTORY_WORDS)?;
    let from_one = record(s, (layout.history(1), HISTORY_WORDS), at, 0, HISTORY_WORDS)?;
    let mut kept = Vec::with_capacity(HISTORY_WORDS);
    for (a, b) in from_zero.iter().zip(&from_one) {
        kept.push(s.pick_r(is_zero, *a, *b)?);
    }
    let colour = three(s, kept.get(0..3).unwrap_or(&[]))?;
    let trail = three(s, kept.get(3..6).unwrap_or(&[]))?;
    let glow = record(s, (layout.hdr(), HDR_WORDS), at, 3, 3)?;
    let glow = three(s, &glow)?;
    let zero = s.f(0.0)?;
    let shown = s.f(TRAIL_SHOWN)?;
    let mut out = [zero; 3];
    for channel in 0..3 {
        let extra = s.fsub(of(trail, channel), of(glow, channel))?;
        let extra = s.fmax(extra, zero)?;
        let extra = s.fmul(extra, shown)?;
        let held = s.fadd(of(colour, channel), extra)?;
        put(&mut out, channel, held);
    }
    Ok(out)
}

/// The words [`bloom_down`] writes for quarter pixel `(x, y)`.
pub(crate) fn bright<S: Shader>(
    s: &mut S,
    look: &Look,
    (x, y): (S::W, S::W),
) -> Result<Vec<S::W>, LaneError> {
    let lit = lit(s, look)?;
    let zero = s.f(0.0)?;
    let scale = s.u(BLOOM_SCALE)?;
    let left = s.mul(x, scale)?;
    let top = s.mul(y, scale)?;
    let mut sum = [zero; 3];
    let weights = [s.f(0.2126)?, s.f(0.7152)?, s.f(0.0722)?];
    let tiny = s.f(1.0e-4)?;
    for dy in 0..BLOOM_SCALE {
        for dx in 0..BLOOM_SCALE {
            let at = kept_at(s, look.screen, (left, top), (dx as i32, dy as i32))?;
            let light = shown_light(s, look, at)?;
            let luma = dot(s, light, weights)?;
            let over = s.fsub(luma, lit.threshold)?;
            let over = s.fmax(over, zero)?;
            let luma = s.fmax(luma, tiny)?;
            let share = s.fdiv(over, luma)?;
            for channel in 0..3 {
                let part = s.fmul(of(light, channel), share)?;
                let added = s.fadd(of(sum, channel), part)?;
                put(&mut sum, channel, added);
            }
        }
    }
    let each = s.f(1.0 / (BLOOM_SCALE * BLOOM_SCALE) as f32)?;
    let mut out = Vec::with_capacity(BLOOM_WORDS);
    for value in sum {
        let value = s.fmul(value, each)?;
        out.push(s.word_of(value)?);
    }
    Ok(out)
}

/// The words a blur of the bloom along x (`across`) or y writes for
/// quarter pixel `(x, y)`, reading the bloom from `from`.
pub(crate) fn blurred<S: Shader>(
    s: &mut S,
    look: &Look,
    (x, y): (S::W, S::W),
    across: bool,
    from: usize,
) -> Result<Vec<S::W>, LaneError> {
    let quarter = look.screen.quarter();
    let zero = s.f(0.0)?;
    let mut sum = [zero; 3];
    for (offset, weight) in (-BLUR_REACH..=BLUR_REACH).zip(blur_weights()) {
        let by = if across { (offset, 0) } else { (0, offset) };
        let at = kept_at(s, quarter, (x, y), by)?;
        let values = record(s, (from, BLOOM_WORDS), at, 0, 3)?;
        let weight = s.f(weight)?;
        for (channel, value) in values.into_iter().enumerate() {
            let part = s.fmul(value, weight)?;
            let added = s.fadd(of(sum, channel), part)?;
            put(&mut sum, channel, added);
        }
    }
    let mut out = Vec::with_capacity(BLOOM_WORDS);
    for value in sum {
        out.push(s.word_of(value)?);
    }
    Ok(out)
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

/// The colour of pixel `(x, y)`, `0x00RRGGBB`.
pub(crate) fn composed<S: Shader>(
    s: &mut S,
    look: &Look,
    (x, y): (S::W, S::W),
) -> Result<S::W, LaneError> {
    let lit = lit(s, look)?;
    let screen = look.screen;
    let quarter = screen.quarter();
    let here = kept_at(s, screen, (x, y), (0, 0))?;
    let light = shown_light(s, look, here)?;
    let half = s.f(0.5)?;
    let fx = s.real(x)?;
    let fy = s.real(y)?;
    let scale = s.f(1.0 / BLOOM_SCALE as f32)?;
    let qx = s.fadd(fx, half)?;
    let qx = s.fmul(qx, scale)?;
    let qx = s.fsub(qx, half)?;
    let qy = s.fadd(fy, half)?;
    let qy = s.fmul(qy, scale)?;
    let qy = s.fsub(qy, half)?;
    let quarter_look = Look {
        screen: quarter,
        ..*look
    };
    let bloom = blended(
        s,
        &quarter_look,
        (look.layout.bloom(0), BLOOM_WORDS),
        (qx, qy),
        3,
    )?;
    let bloom = three(s, &bloom)?;
    let one = s.f(1.0)?;
    let mid_x = s.f(screen.wide as f32 * 0.5)?;
    let mid_y = s.f(screen.high as f32 * 0.5)?;
    let dx = s.fadd(fx, half)?;
    let dx = s.fsub(dx, mid_x)?;
    let dy = s.fadd(fy, half)?;
    let dy = s.fsub(dy, mid_y)?;
    let dx = s.fmul(dx, dx)?;
    let dy = s.fmul(dy, dy)?;
    let r = s.fadd(dx, dy)?;
    let corner = screen.wide as f32 * screen.wide as f32 * 0.25
        + screen.high as f32 * screen.high as f32 * 0.25;
    let corner = s.f(VIGNETTE / corner.max(1.0))?;
    let dark = s.fmul(r, corner)?;
    let vignette = s.fsub(one, dark)?;
    let most = s.f(255.0)?;
    let down = s.u(DITHER_DOWN)?;
    let down = s.mul(y, down)?;
    let across = s.u(DITHER_ACROSS)?;
    let across = s.mul(x, across)?;
    let order = s.add(down, across)?;
    let order = s.and(order, 0xFFFF)?;
    let order = s.real(order)?;
    let step = s.f(1.0 / 65536.0)?;
    let noise = s.fmul(order, step)?;
    let zero = s.f(0.0)?;
    let mut packed = s.u(0)?;
    for (channel, shift) in [(0, 16), (1, 8), (2, 0)] {
        let glow = s.fmul(of(bloom, channel), lit.bloom)?;
        let value = s.fadd(of(light, channel), glow)?;
        let value = s.fmul(value, lit.exposure)?;
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

fn pixel_pass(
    build: &Build,
    operation: &'static str,
    screen: Screen,
    (base, words): (usize, usize),
    body: impl FnOnce(&mut Emitted<'_, '_>, &Look, (Id, Id)) -> Result<Vec<Id>, LaneError>,
) -> Result<Vec<u32>, LaneError> {
    refuse(operation, build.grid, build.screen)?;
    let layout = build.layout();
    let look = look_of(build);
    let reader = reader_of(build);
    let mut kernel = kernel(build.subgroup)?;
    let mut em = Em::new(&mut kernel)?;
    let (pixel, shown, x, y) = placed(&mut em, build.subgroup, screen)?;
    let none = em.u(0)?;
    let x = em.pick(shown, x, none)?;
    let y = em.pick(shown, y, none)?;
    let values = {
        let mut plain = Emitted {
            em: &mut em,
            reader,
        };
        body(&mut plain, &look, (x, y))?
    };
    let count = em.u(word(screen.pixels()))?;
    let at = seat(&mut em, &layout, (pixel, shown, count), base, words)?;
    for (slot, value) in values.into_iter().enumerate() {
        let step = em.u(word(slot))?;
        let into = em.add(at, step)?;
        em.store(PIXELS, into, value)?;
    }
    kernel.finish()
}

fn history_seat(em: &mut Em<'_>, layout: &Layout, scene_at: usize) -> Result<Id, LaneError> {
    let drawn = em.load_u(PIXELS, word(scene_at + scene::DRAWN))?;
    let parity = em.and_u(drawn, 1)?;
    let none = em.u(0)?;
    let is_zero = em.eq(parity, none)?;
    let zero = em.u(word(layout.history(0)))?;
    let one = em.u(word(layout.history(1)))?;
    em.pick(is_zero, zero, one)
}

/// The pass that keeps each pixel's light over frames, into the history of
/// this frame's parity.
pub fn resolve(build: &Build) -> Result<Vec<u32>, LaneError> {
    refuse("render3::resolve", build.grid, build.screen)?;
    let layout = build.layout();
    let look = look_of(build);
    let reader = reader_of(build);
    let mut kernel = kernel(build.subgroup)?;
    let mut em = Em::new(&mut kernel)?;
    let (pixel, shown, x, y) = placed(&mut em, build.subgroup, build.screen)?;
    let none = em.u(0)?;
    let x = em.pick(shown, x, none)?;
    let y = em.pick(shown, y, none)?;
    let values = {
        let mut plain = Emitted {
            em: &mut em,
            reader,
        };
        resolved(&mut plain, &look, (x, y))?
    };
    let base = history_seat(&mut em, &layout, layout.scene())?;
    let size = em.u(word(HISTORY_WORDS))?;
    let at = em.mul(pixel, size)?;
    let at = em.add(base, at)?;
    let count = em.u(word(build.screen.pixels()))?;
    let past = em.sub(pixel, count)?;
    let spare_size = em.u(word(super::SPARE_WORDS))?;
    let past = em.mul(past, spare_size)?;
    let spare = em.u(word(layout.spare()))?;
    let past = em.add(spare, past)?;
    let at = em.pick(shown, at, past)?;
    for (slot, value) in values.into_iter().enumerate() {
        let step = em.u(word(slot))?;
        let into = em.add(at, step)?;
        em.store(PIXELS, into, value)?;
    }
    kernel.finish()
}

/// The pass that keeps the light past the threshold at a quarter of the
/// screen on each side.
pub fn bloom_down(build: &Build) -> Result<Vec<u32>, LaneError> {
    let layout = build.layout();
    pixel_pass(
        build,
        "render3::bloom_down",
        build.screen.quarter(),
        (layout.bloom(0), BLOOM_WORDS),
        |s, look, at| bright(s, look, at),
    )
}

/// The pass that blurs the bloom along x.
pub fn bloom_across(build: &Build) -> Result<Vec<u32>, LaneError> {
    let layout = build.layout();
    pixel_pass(
        build,
        "render3::bloom_across",
        build.screen.quarter(),
        (layout.bloom(1), BLOOM_WORDS),
        |s, look, at| blurred(s, look, at, true, layout.bloom(0)),
    )
}

/// The pass that blurs the bloom along y.
pub fn bloom_up(build: &Build) -> Result<Vec<u32>, LaneError> {
    let layout = build.layout();
    pixel_pass(
        build,
        "render3::bloom_up",
        build.screen.quarter(),
        (layout.bloom(0), BLOOM_WORDS),
        |s, look, at| blurred(s, look, at, false, layout.bloom(1)),
    )
}

/// The pass that writes each pixel's colour, `0x00RRGGBB`, into the first
/// [`Screen::pixels`] words.
pub fn composite(build: &Build) -> Result<Vec<u32>, LaneError> {
    pixel_pass(
        build,
        "render3::composite",
        build.screen,
        (0, 1),
        |s, look, at| Ok(vec![composed(s, look, at)?]),
    )
}

fn every<T>(
    screen: Screen,
    mut each: impl FnMut((u32, u32)) -> Result<Vec<T>, LaneError>,
) -> Result<Vec<T>, LaneError> {
    let mut out = Vec::new();
    for y in 0..screen.high {
        for x in 0..screen.wide {
            out.extend(each((x, y))?);
        }
    }
    Ok(out)
}

fn written(image: &mut [u32], at: usize, words: &[u32]) {
    if let Some(slot) = image.get_mut(at..at + words.len()) {
        slot.copy_from_slice(words);
    }
}

/// Runs the passes after the march on the CPU over `image`, a copy of
/// `PIXELS` that holds the hits, the fog, the volumes, the history and the
/// scene, as [`shade()`](super::shade()), [`resolve`], the bloom and
/// [`composite`] write it, and gives the colours.
pub fn develop(shading: &Shading<'_>, image: &mut [u32]) -> Result<Vec<u32>, LaneError> {
    let layout = shading.layout;
    let screen = shading.screen;
    let hdr = {
        let held = Shading {
            pixels: image,
            ..shading.clone()
        };
        held.shaded()?
    };
    written(image, layout.hdr(), &hdr);
    let parity = image
        .get(layout.scene() + scene::DRAWN)
        .copied()
        .unwrap_or(0);
    let history = {
        let held = Shading {
            pixels: image,
            ..shading.clone()
        };
        let look = held.look();
        let mut plain = held.plain();
        every(screen, |at| resolved(&mut plain, &look, at))?
    };
    written(image, layout.history(parity), &history);
    for step in 0..3 {
        let words = {
            let held = Shading {
                pixels: image,
                ..shading.clone()
            };
            let look = held.look();
            let mut plain = held.plain();
            every(screen.quarter(), |at| match step {
                0 => bright(&mut plain, &look, at),
                1 => blurred(&mut plain, &look, at, true, layout.bloom(0)),
                _ => blurred(&mut plain, &look, at, false, layout.bloom(1)),
            })?
        };
        written(image, layout.bloom(u32::from(step == 1)), &words);
    }
    let held = Shading {
        pixels: image,
        ..shading.clone()
    };
    let look = held.look();
    let mut plain = held.plain();
    every(screen, |at| Ok(vec![composed(&mut plain, &look, at)?]))
}
