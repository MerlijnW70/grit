//! Light held a brick at a time: the light of glowing cells spread over the
//! bricks around them, and how much sun reaches each brick through gas.
//!
//! The light of the glowing cells is spread by a blur along each axis in
//! turn, so it reaches [`LIGHT_REACH`] bricks around each brick that gives
//! it off for a cost that grows with the reach, not its cube. The three
//! passes run only in frames in which a brick changed the light it gives
//! off; a frame in which no glowing cell moved leaves the light as it is.
//!
//! Each brick's light is written once over one set of operations, so the CPU finds it
//! the same way ([`Volumes`]).

use simdr::lanes::LaneError;
use simdr::module::Id;

use super::emit::Em;
use super::march::World;
use super::shade::{
    Emitted, Look, Plainly, Shader, kernel, look_of, plus, reader_of, real_at, real_of, seat,
};
use super::{
    BRICK_BITS, Build, GAS_WORDS, GASSY, LIGHT_REACH, Layout, SUN_STEPS, brick_sides, light, of,
    put, refuse_build, scene, word,
};
use crate::layout::{PARAMS, PIXELS};
use crate::sand::Grid;
use crate::view3::Kinds;

const SPREAD: f32 = 1.6;

/// The weight of each brick from [`LIGHT_REACH`] before to as far after,
/// summing to 1.
#[must_use]
pub fn weights() -> Vec<f32> {
    let reach = LIGHT_REACH as i32;
    let raw: Vec<f32> = (-reach..=reach)
        .map(|at| (-((at * at) as f32) / (2.0 * SPREAD * SPREAD)).exp())
        .collect();
    let total: f32 = raw.iter().sum();
    raw.into_iter().map(|value| value / total).collect()
}

/// Where a spread along `axis` reads from and writes to.
const fn ends(layout: &Layout, axis: usize) -> (usize, usize) {
    match axis {
        0 => (layout.emission(), layout.lamp(0)),
        1 => (layout.lamp(0), layout.lamp(1)),
        _ => (layout.lamp(1), layout.lamp(2)),
    }
}

/// The light of brick `brick` after the spread along `axis`.
fn spread<S: Shader>(
    s: &mut S,
    look: &Look,
    axis: usize,
    brick: S::W,
) -> Result<[S::R; 3], LaneError> {
    let sides = brick_sides(look.grid);
    let across = sides[0].trailing_zeros();
    let down = sides[1].trailing_zeros();
    let (shift, side) = match axis {
        0 => (0, sides[0]),
        1 => (across, sides[1]),
        _ => (across + down, sides[2]),
    };
    let along = s.shr(brick, shift)?;
    let along = s.and(along, side - 1)?;
    let (from, _) = ends(&look.layout, axis);
    let zero = s.f(0.0)?;
    let none = s.u(0)?;
    let three = s.u(3)?;
    let start = s.u(word(from))?;
    let mut out = [zero; 3];
    let reach = LIGHT_REACH as i32;
    for (offset, weight) in (-reach..=reach).zip(weights()) {
        let moved = s.u(offset as u32)?;
        let there = s.add(along, moved)?;
        let size = s.u(side)?;
        let inside = s.lt(there, size)?;
        let step = s.u((offset as u32).wrapping_shl(shift))?;
        let other = s.add(brick, step)?;
        let other = s.pick(inside, other, none)?;
        let at = s.mul(other, three)?;
        let at = s.add(start, at)?;
        let weight = s.f(weight)?;
        let weight = s.pick_r(inside, weight, zero)?;
        for (channel, slot) in (0_u32..).zip(out.iter_mut()) {
            let here = plus(s, at, channel)?;
            let value = real_of(s, here)?;
            let value = s.fmul(value, weight)?;
            *slot = s.fadd(*slot, value)?;
        }
    }
    Ok(out)
}

/// How much sun reaches a point `centre`, in bricks, through the gas
/// between it and the sun, found at [`SUN_STEPS`] points a brick apart
/// toward the sun, the first `first` bricks from it.
pub(crate) fn sun_through<S: Shader>(
    s: &mut S,
    look: &Look,
    centre: [S::R; 3],
    first: f32,
) -> Result<S::R, LaneError> {
    let grid = look.grid;
    let sides = brick_sides(grid);
    let across = sides[0].trailing_zeros();
    let down = sides[1].trailing_zeros();
    let side = (1 << BRICK_BITS) as f32;
    let at_light = word(look.layout.scene() + scene::LIGHT);
    let toward = [
        real_at(s, at_light + word(light::TOWARD))?,
        real_at(s, at_light + word(light::TOWARD) + 1)?,
        real_at(s, at_light + word(light::TOWARD) + 2)?,
    ];
    let smoke = real_at(s, at_light + word(light::SMOKE))?;
    let shifts = [0, across, across + down];
    let zero = s.f(0.0)?;
    let gas = s.u(word(look.layout.gas()))?;
    let none = s.u(0)?;
    let mut total = zero;
    for step in 1..=SUN_STEPS {
        let mut index = none;
        let mut inside = s.eq(none, none)?;
        for axis in 0..3 {
            let far = s.f(first + (step - 1) as f32)?;
            let moved = s.fmul(of(toward, axis), far)?;
            let at = s.fadd(of(centre, axis), moved)?;
            let below = s.fgt(zero, at)?;
            let last = s.f(of(sides, axis) as f32)?;
            let unders = s.fgt(last, at)?;
            let above = s.not(below)?;
            let held = s.both(above, unders)?;
            inside = s.both(inside, held)?;
            let kept = s.fmax(at, zero)?;
            let whole = s.whole(kept)?;
            let whole = s.and(whole, of(sides, axis) - 1)?;
            let placed = s.shl(whole, of(shifts, axis))?;
            index = s.or(index, placed)?;
        }
        let four = s.u(word(GAS_WORDS))?;
        let index = s.mul(index, four)?;
        let at = s.add(gas, index)?;
        let share = real_of(s, at)?;
        let share = s.pick_r(inside, share, zero)?;
        total = s.fadd(total, share)?;
    }
    let density = s.f(side)?;
    let density = s.fmul(density, smoke)?;
    let lost = s.fmul(total, density)?;
    let lost = s.fsub(zero, lost)?;
    s.exp(lost)
}

/// How much sun reaches brick `brick` through the gas between it and the
/// sun, found [`SUN_STEPS`] bricks toward the sun, when the brick holds
/// gas; 1 for a brick that holds none, which no ray walks from.
fn through<S: Shader>(s: &mut S, look: &Look, brick: S::W) -> Result<S::R, LaneError> {
    let grid = look.grid;
    let sides = brick_sides(grid);
    let across = sides[0].trailing_zeros();
    let down = sides[1].trailing_zeros();
    let occupancy = s.u(word(look.layout.occupancy()))?;
    let at = s.add(occupancy, brick)?;
    let held = s.pixel(at)?;
    let gassy = s.and(held, GASSY)?;
    let none = s.u(0)?;
    let clean = s.eq(gassy, none)?;
    let walked = s.not(clean)?;
    let one = s.f(1.0)?;
    let gate = s.open(walked)?;
    let half = s.f(0.5)?;
    let mut centre = [half; 3];
    let shifts = [0, across, across + down];
    for axis in 0..3 {
        let part = s.shr(brick, of(shifts, axis))?;
        let part = s.and(part, of(sides, axis) - 1)?;
        let part = s.real(part)?;
        let held = s.fadd(part, half)?;
        put(&mut centre, axis, held);
    }
    let value = sun_through(s, look, centre, 1.0)?;
    let closed = s.close(gate, &[value], &[one])?;
    Ok(closed.first().copied().unwrap_or(one))
}

/// The lane's brick, whether it is one, and how many there are.
fn bricked(em: &mut Em<'_>, build: &Build) -> Result<(Id, Id, Id), LaneError> {
    let brick = em.invocation(build.subgroup)?;
    let count = em.u(build.layout().bricks)?;
    let inside = em.lt(brick, count)?;
    Ok((brick, inside, count))
}

fn spread_pass(build: &Build, axis: usize, operation: &'static str) -> Result<Vec<u32>, LaneError> {
    refuse_build(operation, build)?;
    let layout = build.layout();
    let look = look_of(build);
    let reader = reader_of(build);
    let mut kernel = kernel(build)?;
    let mut em = Em::read_by(&mut kernel, build)?;
    let (brick, inside, count) = bricked(&mut em, build)?;
    let frame = em.load_u(PARAMS, 0)?;
    let one = em.u(1)?;
    let now = em.add(frame, one)?;
    let changed = em.load_u(PIXELS, word(layout.changed()))?;
    let fresh = em.eq(changed, now)?;
    let none = em.u(0)?;
    let place = em.pick(inside, brick, none)?;
    let (_, to) = ends(&layout, axis);
    let at = seat(&mut em, &layout, (brick, inside, count), to, 3)?;
    let _ = em.when(fresh, &[], |em| {
        let mut plain = Emitted { em, reader };
        let light = spread(&mut plain, &look, axis, place)?;
        for (slot, value) in (0_u32..).zip(light) {
            let value = plain.word_of(value)?;
            let into = plus(&mut plain, at, slot)?;
            plain.em.store(PIXELS, into, value)?;
        }
        Ok(Vec::new())
    })?;
    kernel.finish()
}

/// The pass that spreads the light the bricks give off along x, in frames
/// in which a brick changed it. Dispatch it over [`super::bricks`] lanes.
pub fn lit_across(build: &Build) -> Result<Vec<u32>, LaneError> {
    spread_pass(build, 0, "render3::lit_across")
}

/// The pass that spreads the light along y, after [`lit_across`].
pub fn lit_down(build: &Build) -> Result<Vec<u32>, LaneError> {
    spread_pass(build, 1, "render3::lit_down")
}

/// The pass that spreads the light along z, after [`lit_down`]: the light
/// the shading reads.
pub fn lit_deep(build: &Build) -> Result<Vec<u32>, LaneError> {
    spread_pass(build, 2, "render3::lit_deep")
}

/// The pass that finds how much sun reaches each brick that holds gas
/// through the gas; only those bricks walk toward the sun.
pub fn sunlit(build: &Build) -> Result<Vec<u32>, LaneError> {
    refuse_build("render3::sunlit", build)?;
    let layout = build.layout();
    let look = look_of(build);
    let reader = reader_of(build);
    let mut kernel = kernel(build)?;
    let mut em = Em::read_by(&mut kernel, build)?;
    let (brick, inside, count) = bricked(&mut em, build)?;
    let none = em.u(0)?;
    let place = em.pick(inside, brick, none)?;
    let value = {
        let mut plain = Emitted {
            em: &mut em,
            reader,
        };
        let value = through(&mut plain, &look, place)?;
        plain.word_of(value)?
    };
    let at = seat(&mut em, &layout, (brick, inside, count), layout.sunlit(), 1)?;
    em.store(PIXELS, at, value)?;
    kernel.finish()
}

/// The brick volumes on the CPU, read from `pixels` as the passes read
/// `PIXELS`.
#[derive(Debug, Clone, Copy)]
pub struct Volumes<'held> {
    pub grid: Grid,
    pub kinds: Kinds,
    pub layout: Layout,
    pub pixels: &'held [u32],
}

impl Volumes<'_> {
    fn look(&self) -> Look {
        Look {
            grid: self.grid,
            kinds: self.kinds,
            screen: self.layout.screen,
            layout: self.layout,
            suns: 1,
        }
    }

    fn plain(&self) -> Plainly<'_> {
        Plainly {
            world: World {
                grid: self.grid,
                kinds: self.kinds,
                visible: 0,
                opaque: 0,
                cells: &[],
                bricks: &[],
            },
            pixels: self.pixels,
            frame: 0,
        }
    }

    /// The words the spread along `axis` writes, red, green and blue a
    /// brick, from what `pixels` holds before it.
    pub fn spread(&self, axis: usize) -> Result<Vec<u32>, LaneError> {
        let look = self.look();
        let mut plain = self.plain();
        let mut out = Vec::with_capacity(3 * self.layout.bricks as usize);
        for brick in 0..self.layout.bricks {
            out.extend(spread(&mut plain, &look, axis, brick)?.map(f32::to_bits));
        }
        Ok(out)
    }

    /// The words [`sunlit`] writes, one a brick.
    pub fn sunlit(&self) -> Result<Vec<u32>, LaneError> {
        let look = self.look();
        let mut plain = self.plain();
        let mut out = Vec::with_capacity(self.layout.bricks as usize);
        for brick in 0..self.layout.bricks {
            out.push(through(&mut plain, &look, brick)?.to_bits());
        }
        Ok(out)
    }
}
