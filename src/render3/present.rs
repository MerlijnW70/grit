//! The picture shown in a window straight from the device.
//!
//! [`window`] stretches the picture over the window's pixels and lays a
//! picture drawn by the host over it, both in `PIXELS`, after the words of
//! [`Layout::words`]. With the `window` feature, a `Presenter` copies those
//! words into an image of a Vulkan swapchain on the window and presents it, on
//! the queue the picture's passes run on, so nothing is read back to the host.
//!
//! The overlay holds a word a window pixel: its colour already weighed by
//! how much it covers, and how much it covers, out of 255, in the top byte.
//! A word of 0 lets the picture through unchanged.

use simdr::lanes::LaneError;
use simdr::module::Id;

use super::emit::Em;
use super::shade::{kernel, placed};
use super::{Build, Layout, Screen, refuse, word};
use crate::layout::PIXELS;

/// Where the window's words lie in `PIXELS`: its colours, then its overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Windowed {
    pub window: Screen,
    pub colours: usize,
    pub overlay: usize,
}

impl Windowed {
    #[must_use]
    pub const fn of(layout: &Layout, window: Screen) -> Self {
        let colours = layout.words();
        Self {
            window,
            colours,
            overlay: colours + window.pixels(),
        }
    }

    /// The words `PIXELS` holds for the picture and the window.
    #[must_use]
    pub const fn words(&self) -> usize {
        self.overlay + self.window.pixels()
    }
}

/// Where pixel `at` of a line `to` long reads a line `from` long: the first
/// pixel, the next, and the share of the next, out of 256.
#[must_use]
pub fn tap(at: u32, from: u32, to: u32) -> (u32, u32, u32) {
    let to = to.max(1);
    let last = from.saturating_sub(1);
    let across = (2 * at + 1) * from;
    let gone = across.saturating_sub(to);
    let place = (gone / to) * 128 + ((gone % to) * 128) / to;
    let low = (place >> 8).min(last);
    let high = (low + 1).min(last);
    let share = if place >> 8 > last { 0 } else { place & 0xFF };
    (low, high, share)
}

/// Two pixels, `0x00RRGGBB`, blended by `share` out of 256 toward `b`.
#[must_use]
pub const fn blend(a: u32, b: u32, share: u32) -> u32 {
    let keep = 256 - share;
    let red_blue = ((a & 0x00FF_00FF) * keep + (b & 0x00FF_00FF) * share) >> 8;
    let green = ((a & 0x0000_FF00) * keep + (b & 0x0000_FF00) * share) >> 8;
    (red_blue & 0x00FF_00FF) | (green & 0x0000_FF00)
}

/// `pixel` under the overlay word `over`.
#[must_use]
pub const fn covered(pixel: u32, over: u32) -> u32 {
    let cover = over >> 24;
    let keep = 255 - cover;
    let mut out = 0;
    let mut shift = 0;
    while shift < 24 {
        let shown = (pixel >> shift) & 0xFF;
        let laid = (over >> shift) & 0xFF;
        let value = laid + (shown * keep + 127) / 255;
        let value = if value > 255 { 255 } else { value };
        out |= value << shift;
        shift += 8;
    }
    out
}

/// `picture`, drawn on `from`, stretched over `to`, blended between the four
/// pixels around each, under `overlay`, as [`window`] writes it.
#[must_use]
pub fn windowed(picture: &[u32], from: Screen, to: Screen, overlay: &[u32]) -> Vec<u32> {
    let across: Vec<_> = (0..to.wide).map(|x| tap(x, from.wide, to.wide)).collect();
    let wide = from.wide as usize;
    let get = |x: u32, y: u32| {
        picture
            .get(y as usize * wide + x as usize)
            .copied()
            .unwrap_or(0)
    };
    let mut out = Vec::with_capacity(to.pixels());
    for y in 0..to.high {
        let (top, bottom, share_y) = tap(y, from.high, to.high);
        for &(left, right, share_x) in &across {
            let upper = blend(get(left, top), get(right, top), share_x);
            let lower = blend(get(left, bottom), get(right, bottom), share_x);
            let pixel = blend(upper, lower, share_y);
            let over = overlay.get(out.len()).copied().unwrap_or(0);
            out.push(covered(pixel, over));
        }
    }
    out
}

fn tap_em(em: &mut Em<'_>, at: Id, from: u32, to: u32) -> Result<(Id, Id, Id), LaneError> {
    let to = to.max(1);
    let last = em.u(from.saturating_sub(1))?;
    let twice = em.add(at, at)?;
    let one = em.u(1)?;
    let odd = em.add(twice, one)?;
    let from_word = em.u(from)?;
    let across = em.mul(odd, from_word)?;
    let to_word = em.u(to)?;
    let short = em.lt(across, to_word)?;
    let gone = em.sub(across, to_word)?;
    let none = em.u(0)?;
    let gone = em.pick(short, none, gone)?;
    let quotient = em.div(gone, to_word)?;
    let whole = em.shl(quotient, 7)?;
    let back = em.mul(quotient, to_word)?;
    let rest = em.sub(gone, back)?;
    let rest = em.shl(rest, 7)?;
    let rest = em.div(rest, to_word)?;
    let place = em.add(whole, rest)?;
    let low = em.shr(place, 8)?;
    let past = em.lt(last, low)?;
    let low = em.pick(past, last, low)?;
    let next = em.add(low, one)?;
    let over = em.lt(last, next)?;
    let high = em.pick(over, last, next)?;
    let share = em.and_u(place, 0xFF)?;
    let share = em.pick(past, none, share)?;
    Ok((low, high, share))
}

fn blend_em(em: &mut Em<'_>, a: Id, b: Id, share: Id) -> Result<Id, LaneError> {
    let full = em.u(256)?;
    let keep = em.sub(full, share)?;
    let mut out = em.u(0)?;
    for mask in [0x00FF_00FF, 0x0000_FF00] {
        let mask_word = em.u(mask)?;
        let left = em.and(a, mask_word)?;
        let left = em.mul(left, keep)?;
        let right = em.and(b, mask_word)?;
        let right = em.mul(right, share)?;
        let sum = em.add(left, right)?;
        let sum = em.shr(sum, 8)?;
        let sum = em.and(sum, mask_word)?;
        out = em.or(out, sum)?;
    }
    Ok(out)
}

fn covered_em(em: &mut Em<'_>, pixel: Id, over: Id) -> Result<Id, LaneError> {
    let cover = em.shr(over, 24)?;
    let most = em.u(255)?;
    let keep = em.sub(most, cover)?;
    let half = em.u(127)?;
    let mut out = em.u(0)?;
    for shift in [0, 8, 16] {
        let shown = em.shr(pixel, shift)?;
        let shown = em.and_u(shown, 0xFF)?;
        let laid = em.shr(over, shift)?;
        let laid = em.and_u(laid, 0xFF)?;
        let kept = em.mul(shown, keep)?;
        let kept = em.add(kept, half)?;
        let kept = em.div(kept, most)?;
        let value = em.add(laid, kept)?;
        let past = em.lt(most, value)?;
        let value = em.pick(past, most, value)?;
        let value = em.shl(value, shift)?;
        out = em.or(out, value)?;
    }
    Ok(out)
}

/// The pass that stretches the picture over `window` and lays the overlay
/// over it, into the words [`Windowed::of`] gives. Dispatch it over
/// `window.groups(subgroup)`.
pub fn window(build: &Build, window: Screen) -> Result<Vec<u32>, LaneError> {
    refuse("render3::window", build.grid, build.screen)?;
    refuse("render3::window", build.grid, window)?;
    let layout = build.layout();
    let held = Windowed::of(&layout, window);
    let from = build.screen;
    let mut kernel = kernel(build)?;
    let mut em = Em::reading(&mut kernel, build.windows)?;
    let (pixel, shown, x, y) = placed(&mut em, build.subgroup, window)?;
    let none = em.u(0)?;
    let x = em.pick(shown, x, none)?;
    let y = em.pick(shown, y, none)?;
    let (left, right, share_x) = tap_em(&mut em, x, from.wide, window.wide)?;
    let (top, bottom, share_y) = tap_em(&mut em, y, from.high, window.high)?;
    let wide = em.u(from.wide)?;
    let read = |em: &mut Em<'_>, column: Id, row: Id| -> Result<Id, LaneError> {
        let at = em.mul(row, wide)?;
        let at = em.add(at, column)?;
        em.load(PIXELS, at)
    };
    let a = read(&mut em, left, top)?;
    let b = read(&mut em, right, top)?;
    let c = read(&mut em, left, bottom)?;
    let d = read(&mut em, right, bottom)?;
    let upper = blend_em(&mut em, a, b, share_x)?;
    let lower = blend_em(&mut em, c, d, share_x)?;
    let picture = blend_em(&mut em, upper, lower, share_y)?;
    let place = em.pick(shown, pixel, none)?;
    let overlay = em.u(word(held.overlay))?;
    let overlay = em.add(overlay, place)?;
    let over = em.load(PIXELS, overlay)?;
    let value = covered_em(&mut em, picture, over)?;
    let colours = em.u(word(held.colours))?;
    let into = em.add(colours, pixel)?;
    let spare = em.u(word(layout.spare()))?;
    let into = em.pick(shown, into, spare)?;
    em.store(PIXELS, into, value)?;
    kernel.finish()
}

#[cfg(feature = "window")]
pub use super::swapchain::{Presenter, Refused};
