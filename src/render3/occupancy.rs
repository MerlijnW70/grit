//! Which bricks hold a cell anyone sees, kept current from the octs the
//! engine steps.

use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, U32};
use simdr::module::Id;

use super::emit::Em;
use super::{BRICK_BITS, Build, of, put, refuse, scene, word};
use crate::layout::{BINDINGS, DENSE, GRAND, PARAMS, PIXELS, STATE};
use crate::sand::Grid;
use crate::view3::Kinds;

const SIDE: u32 = 1 << BRICK_BITS;
const CELLS: u32 = SIDE * SIDE * SIDE;

/// A word a brick of `grid`: 1 when one of its cells has a kind in
/// `visible`, else 0.
#[must_use]
pub fn occupancy(grid: Grid, kinds: Kinds, visible: u32, cells: &[u32]) -> Vec<u32> {
    let across = grid.wide >> BRICK_BITS;
    let down = grid.deep >> BRICK_BITS;
    let mut out = vec![0; super::bricks(grid) as usize];
    for z in 0..grid.tall {
        for y in 0..grid.deep {
            for x in 0..grid.wide {
                let at = ((z * grid.deep + y) * grid.wide + x) as usize;
                let kind = kinds.of(cells.get(at).copied().unwrap_or(0));
                if (visible >> kind) & 1 == 1 {
                    let brick = (((z >> BRICK_BITS) * down + (y >> BRICK_BITS)) * across
                        + (x >> BRICK_BITS)) as usize;
                    if let Some(slot) = out.get_mut(brick) {
                        *slot = 1;
                    }
                }
            }
        }
    }
    out
}

fn kernel(subgroup: u32) -> Result<Kernel<U32>, LaneError> {
    Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))
}

/// Emits the rebuild of brick `brick`, given as its coordinates.
fn rebuilt(
    em: &mut Em<'_>,
    build: &Build,
    at: [Id; 3],
    visible: Id,
    into: Id,
) -> Result<(), LaneError> {
    let grid = build.grid;
    let wide = grid.wide.trailing_zeros();
    let deep = grid.deep.trailing_zeros();
    let mut corner = [at[0]; 3];
    for axis in 0..3 {
        let held = em.shl(of(at, axis), BRICK_BITS)?;
        put(&mut corner, axis, held);
    }
    let z = em.shl(corner[2], wide + deep)?;
    let y = em.shl(corner[1], wide)?;
    let first = em.or(z, y)?;
    let first = em.or(first, corner[0])?;
    let one = em.u(1)?;
    let none = em.u(0)?;
    let kinds = build.kinds;
    let out = em.looped(CELLS, &[one, none], |em, words, run| {
        let seen = words.get(1).copied().unwrap_or(none);
        let x = em.and_u(run, SIDE - 1)?;
        let y = em.shr(run, BRICK_BITS)?;
        let y = em.and_u(y, SIDE - 1)?;
        let y = em.shl(y, wide)?;
        let z = em.shr(run, 2 * BRICK_BITS)?;
        let z = em.shl(z, wide + deep)?;
        let cell = em.add(first, x)?;
        let cell = em.add(cell, y)?;
        let cell = em.add(cell, z)?;
        let held = em.load(STATE, cell)?;
        let kind = em.shr(held, kinds.shift())?;
        let kind = em.and_u(kind, kinds.mask())?;
        let shown = em.bit(visible, kind)?;
        let shown = em.flag(shown)?;
        let seen = em.or(seen, shown)?;
        Ok(vec![one, seen])
    })?;
    let seen = out.get(1).copied().unwrap_or(none);
    em.store(PIXELS, into, seen)
}

/// The pass that rebuilds every brick's word, one brick a lane; dispatch
/// it over [`super::bricks`] lanes.
pub fn filled(build: &Build) -> Result<Vec<u32>, LaneError> {
    refuse("render3::filled", build.grid, build.screen)?;
    let layout = build.layout();
    let grid = build.grid;
    let mut kernel = kernel(build.subgroup)?;
    let mut em = Em::new(&mut kernel)?;
    let brick = em.invocation(build.subgroup)?;
    let count = em.u(layout.bricks)?;
    let inside = em.lt(brick, count)?;
    let visible = em.load_u(PIXELS, word(layout.scene() + scene::VISIBLE))?;
    let across = (grid.wide >> BRICK_BITS).trailing_zeros();
    let down = (grid.deep >> BRICK_BITS).trailing_zeros();
    let x = em.and_u(brick, (1 << across) - 1)?;
    let y = em.shr(brick, across)?;
    let y = em.and_u(y, (1 << down) - 1)?;
    let z = em.shr(brick, across + down)?;
    let base = em.u(word(layout.occupancy()))?;
    let into = em.add(base, brick)?;
    let _ = em.when(inside, &[], |em| {
        rebuilt(em, build, [x, y, z], visible, into)?;
        Ok(Vec::new())
    })?;
    kernel.finish()
}

/// The pass that rebuilds the word of each brick an oct listed this frame
/// touches, once a brick however many listed octs touch it. It runs over
/// the listed octs, as the engine's step does, after that step. A brick's
/// claim word holds the frame that last claimed it, plus one.
pub fn occupied(build: &Build) -> Result<Vec<u32>, LaneError> {
    refuse("render3::occupied", build.grid, build.screen)?;
    let layout = build.layout();
    let grid = build.grid;
    let mut kernel = kernel(build.subgroup)?;
    let mut em = Em::new(&mut kernel)?;
    let slot = em.invocation(build.subgroup)?;
    let grand = em.load_u(GRAND, 0)?;
    let dump = em.load_u(PARAMS, 1)?;
    let claimed = em.load(DENSE, dump)?;
    let total = em.add(grand, claimed)?;
    let listed = em.lt(slot, total)?;
    let visible = em.load_u(PIXELS, word(layout.scene() + scene::VISIBLE))?;
    let _ = em.when(listed, &[], |em| {
        let oct = em.load(DENSE, slot)?;
        let frame = em.load_u(PARAMS, 0)?;
        let phase = em.and_u(frame, 1)?;
        let one = em.u(1)?;
        let stamp = em.add(frame, one)?;
        let sides = [grid.wide, grid.deep, grid.tall];
        let halves = sides.map(|side| (side / 2).trailing_zeros());
        let mut spans = [[oct; 2]; 3];
        let mut shift = 0;
        for axis in 0..3 {
            let lattice = em.shr(oct, shift)?;
            let lattice = em.and_u(lattice, (1 << of(halves, axis)) - 1)?;
            shift += of(halves, axis);
            let low = em.shl(lattice, 1)?;
            let low = em.add(low, phase)?;
            let low = em.and_u(low, of(sides, axis) - 1)?;
            let one = em.u(1)?;
            let high = em.add(low, one)?;
            let high = em.and_u(high, of(sides, axis) - 1)?;
            let held = [em.shr(low, BRICK_BITS)?, em.shr(high, BRICK_BITS)?];
            put(&mut spans, axis, held);
        }
        let across = (grid.wide >> BRICK_BITS).trailing_zeros();
        let down = (grid.deep >> BRICK_BITS).trailing_zeros();
        let claims = em.u(word(layout.claims()))?;
        let bricks = em.u(word(layout.occupancy()))?;
        for corner in 0..8_usize {
            let at = [0, 1, 2].map(|axis| {
                let [low, high] = of(spans, axis);
                if (corner >> axis) & 1 == 1 { high } else { low }
            });
            let z = em.shl(at[2], across + down)?;
            let y = em.shl(at[1], across)?;
            let brick = em.or(z, y)?;
            let brick = em.or(brick, at[0])?;
            let claim = em.add(claims, brick)?;
            let before = em.kernel.atomic_exchange_at(PIXELS, claim, stamp)?;
            let mine = em.ne(before, stamp)?;
            let into = em.add(bricks, brick)?;
            let _ = em.when(mine, &[], |em| {
                rebuilt(em, build, at, visible, into)?;
                Ok(Vec::new())
            })?;
        }
        Ok(Vec::new())
    })?;
    kernel.finish()
}
