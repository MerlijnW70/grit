//! Which bricks hold a cell anyone sees or gas, how much of each is gas,
//! how much light its glowing cells give off and how much of it is water,
//! kept current from the octs the engine steps.

use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, U32};
use simdr::module::Id;

use super::emit::Em;
use super::{
    BRICK_BITS, Build, GAS_WORDS, GASSY, MATERIAL_WORDS, Sparse, Table, WATER_WORDS, material, of,
    put, refuse_build, scene, word,
};
use crate::layout::{DENSE, GRAND, PARAMS, PIXELS};
use crate::sand::Grid;
use crate::view3::Kinds;

const SIDE: u32 = 1 << BRICK_BITS;
const CELLS: u32 = SIDE * SIDE * SIDE;

/// A word a brick of `grid`: bit 0 set when one of its cells has a kind in
/// `shown`, and [`GASSY`] when one has a kind in `gases`. The passes count
/// a brick as full for the kinds of [`Table::shown`].
#[must_use]
pub fn occupancy(grid: Grid, kinds: Kinds, shown: u32, gases: u32, cells: &[u32]) -> Vec<u32> {
    let across = grid.wide >> BRICK_BITS;
    let down = grid.deep >> BRICK_BITS;
    let mut out = vec![0; super::bricks(grid) as usize];
    for z in 0..grid.tall {
        for y in 0..grid.deep {
            for x in 0..grid.wide {
                let at = ((z * grid.deep + y) * grid.wide + x) as usize;
                let kind = kinds.of(cells.get(at).copied().unwrap_or(0));
                let brick = (((z >> BRICK_BITS) * down + (y >> BRICK_BITS)) * across
                    + (x >> BRICK_BITS)) as usize;
                if let Some(slot) = out.get_mut(brick) {
                    if (shown >> kind) & 1 == 1 {
                        *slot |= 1;
                    }
                    if (gases >> kind) & 1 == 1 {
                        *slot |= GASSY;
                    }
                }
            }
        }
    }
    out
}

fn kernel(build: &Build) -> Result<Kernel<U32>, LaneError> {
    Kernel::<U32>::new(Shape::new(build.subgroup, build.subgroup, build.bindings()))
}

/// The kinds a rebuild reads from the scene: those a brick counts as full
/// for, the gases, the glowing kinds and the liquids.
#[derive(Debug, Clone, Copy)]
struct Masks {
    shown: Id,
    gases: Id,
    glowing: Id,
    liquid: Id,
}

impl Masks {
    fn of(em: &mut Em<'_>, build: &Build) -> Result<Self, LaneError> {
        let base = word(build.layout().scene());
        Ok(Self {
            shown: em.load_u(PIXELS, base + word(scene::SHOWN))?,
            gases: em.load_u(PIXELS, base + word(scene::GAS))?,
            glowing: em.load_u(PIXELS, base + word(scene::GLOWING))?,
            liquid: em.load_u(PIXELS, base + word(scene::LIQUID))?,
        })
    }
}

/// Emits the rebuild of the brick at `at`, given as its coordinates, whose
/// index is `brick`: its occupancy word, its share of gas, the light it
/// gives off and its water. When the light changed, the frame plus one goes
/// into [`super::Layout::changed`].
fn rebuilt(
    em: &mut Em<'_>,
    build: &Build,
    at: [Id; 3],
    masks: Masks,
    brick: Id,
) -> Result<(), LaneError> {
    let grid = build.grid;
    let layout = build.layout();
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
    let nothing = em.f(0.0)?;
    let nothing = em.float_bits(nothing)?;
    let kinds = build.kinds;
    let table = em.u(word(layout.table()))?;
    let size = em.u(word(MATERIAL_WORDS))?;
    let initial = [
        one, none, none, nothing, nothing, nothing, nothing, nothing, nothing, nothing, none, none,
    ];
    let mut plain = None;
    if let Some(point) = em.world_point(corner)?
        && let Some(entry) = em.entry(point)?
    {
        let kind = em.shr(entry.value, kinds.shift())?;
        let kind = em.and_u(kind, kinds.mask())?;
        let busy = em.or(masks.glowing, masks.gases)?;
        let busy = em.or(busy, masks.liquid)?;
        let busy = em.bit(busy, kind)?;
        let calm = em.not(busy)?;
        let uniform = em.not(entry.held)?;
        let shown = em.bit(masks.shown, kind)?;
        let shown = em.flag(shown)?;
        plain = Some((em.both(uniform, calm)?, shown));
    }
    let walk = |em: &mut Em<'_>| -> Result<Vec<Id>, LaneError> {
        em.looped(CELLS, &initial, |em, words, run| {
            let get = |at: usize| words.get(at).copied().unwrap_or(none);
            let x = em.and_u(run, SIDE - 1)?;
            let y = em.shr(run, BRICK_BITS)?;
            let y = em.and_u(y, SIDE - 1)?;
            let y = em.shl(y, wide)?;
            let z_in = em.shr(run, 2 * BRICK_BITS)?;
            let z = em.shl(z_in, wide + deep)?;
            let cell = em.add(first, x)?;
            let cell = em.add(cell, y)?;
            let cell = em.add(cell, z)?;
            let column = em.add(corner[0], x)?;
            let height = em.shr(run, BRICK_BITS)?;
            let height = em.and_u(height, SIDE - 1)?;
            let height = em.add(corner[1], height)?;
            let row = em.add(corner[2], z_in)?;
            let held = em.cell(cell)?;
            let kind = em.shr(held, kinds.shift())?;
            let kind = em.and_u(kind, kinds.mask())?;
            let shown = em.bit(masks.shown, kind)?;
            let shown = em.flag(shown)?;
            let seen = em.or(get(1), shown)?;
            let gas = em.bit(masks.gases, kind)?;
            let gas = em.flag(gas)?;
            let gas = em.add(get(2), gas)?;
            let glows = em.bit(masks.glowing, kind)?;
            let light = [get(3), get(4), get(5)];
            let light = em.when(glows, &light, |em| {
                let at = em.mul(kind, size)?;
                let at = em.add(table, at)?;
                let glow_at = em.u(word(material::GLOW))?;
                let glow_at = em.add(at, glow_at)?;
                let glow = em.load(PIXELS, glow_at)?;
                let glow = em.bits_float(glow)?;
                let mut out = Vec::with_capacity(3);
                for (channel, &before) in light.iter().enumerate() {
                    let here = em.u(word(material::ALBEDO + channel))?;
                    let here = em.add(at, here)?;
                    let albedo = em.load(PIXELS, here)?;
                    let albedo = em.bits_float(albedo)?;
                    let given = em.fmul(albedo, glow)?;
                    let before = em.bits_float(before)?;
                    let sum = em.fadd(before, given)?;
                    out.push(em.float_bits(sum)?);
                }
                Ok(out)
            })?;
            let gassy = em.bit(masks.gases, kind)?;
            let thick = [get(6), get(7), get(8), get(9)];
            let thick = em.when(gassy, &thick, |em| {
                let at = em.mul(kind, size)?;
                let at = em.add(table, at)?;
                let clear_at = em.u(word(material::CLEAR))?;
                let clear_at = em.add(at, clear_at)?;
                let clear = em.load(PIXELS, clear_at)?;
                let clear = em.bits_float(clear)?;
                let clear = em.clamp01(clear)?;
                let whole = em.f(1.0)?;
                let taken = em.fsub(whole, clear)?;
                let mut out = Vec::with_capacity(4);
                let before = em.bits_float(thick[0])?;
                let sum = em.fadd(before, taken)?;
                out.push(em.float_bits(sum)?);
                for (channel, &before) in thick.iter().skip(1).enumerate() {
                    let here = em.u(word(material::ALBEDO + channel))?;
                    let here = em.add(at, here)?;
                    let albedo = em.load(PIXELS, here)?;
                    let albedo = em.bits_float(albedo)?;
                    let given = em.fmul(albedo, taken)?;
                    let before = em.bits_float(before)?;
                    let sum = em.fadd(before, given)?;
                    out.push(em.float_bits(sum)?);
                }
                Ok(out)
            })?;
            let liquid = em.bit(masks.liquid, kind)?;
            let wet = em.flag(liquid)?;
            let wet = em.add(get(10), wet)?;
            let open = em.when(liquid, &[none], |em| {
                let found = open_beside(em, build, masks, cell, [column, height, row])?;
                let at = em.mul(kind, size)?;
                let at = em.add(table, at)?;
                let falls_at = em.u(word(material::FALLS))?;
                let falls_at = em.add(at, falls_at)?;
                let falls = em.load(PIXELS, falls_at)?;
                let marked = em.ne(falls, none)?;
                let bit_at = em.sub(falls, one)?;
                let bit_at = em.pick(marked, bit_at, none)?;
                let flagged = em.bit(held, bit_at)?;
                let open = em.pick_t(marked, flagged, found)?;
                Ok(vec![em.flag(open)?])
            })?;
            let open = open.first().copied().unwrap_or(none);
            let spray = em.add(get(11), open)?;
            let mut next = vec![one, seen, gas];
            next.extend(light);
            next.extend(thick);
            next.push(wet);
            next.push(spray);
            Ok(next)
        })
    };
    let out = match plain {
        Some((plain, shown)) => em.branch(
            plain,
            |_| {
                Ok(vec![
                    one, shown, none, nothing, nothing, nothing, nothing, nothing, nothing,
                    nothing, none, none,
                ])
            },
            walk,
        )?,
        None => walk(em)?,
    };
    let get = |at: usize| out.get(at).copied().unwrap_or(none);
    let occupancy = em.u(word(layout.occupancy()))?;
    let into = em.add(occupancy, brick)?;
    let some = em.ne(get(2), none)?;
    let gassy = em.u(GASSY)?;
    let gassy = em.pick(some, gassy, none)?;
    let held = em.or(get(1), gassy)?;
    em.store(PIXELS, into, held)?;
    let share = em.f(1.0 / CELLS as f32)?;
    let gas = em.u(word(layout.gas()))?;
    let size = em.u(word(GAS_WORDS))?;
    let start = em.mul(brick, size)?;
    let start = em.add(gas, start)?;
    for field in 0..GAS_WORDS {
        let value = em.bits_float(get(6 + field))?;
        let value = em.fmul(value, share)?;
        let value = em.float_bits(value)?;
        let step = em.u(word(field))?;
        let into = em.add(start, step)?;
        em.store(PIXELS, into, value)?;
    }
    let three = em.u(3)?;
    let emission = em.u(word(layout.emission()))?;
    let base = em.mul(brick, three)?;
    let base = em.add(emission, base)?;
    let mut changed = em.ne(none, none)?;
    for channel in 0..3_u32 {
        let step = em.u(channel)?;
        let into = em.add(base, step)?;
        let before = em.load(PIXELS, into)?;
        let now = get(3 + channel as usize);
        let differs = em.ne(before, now)?;
        changed = em.either(changed, differs)?;
        em.store(PIXELS, into, now)?;
    }
    let _ = em.when(changed, &[], |em| {
        let frame = em.load_u(PARAMS, 0)?;
        let stamp = em.add(frame, one)?;
        let at = em.u(word(layout.changed()))?;
        em.store(PIXELS, at, stamp)?;
        Ok(Vec::new())
    })?;
    let water = em.u(word(layout.water()))?;
    let size = em.u(word(WATER_WORDS))?;
    let start = em.mul(brick, size)?;
    let start = em.add(water, start)?;
    for field in 0..WATER_WORDS {
        let count = em.to_float(get(10 + field))?;
        let value = em.fmul(count, share)?;
        let value = em.float_bits(value)?;
        let step = em.u(word(field))?;
        let into = em.add(start, step)?;
        em.store(PIXELS, into, value)?;
    }
    Ok(())
}

/// Whether a liquid cell at `cell`, at `[column, height, row]` of the grid,
/// falls free: it is not on the grid's floor, the cell under it is liquid
/// or of no kind in `shown`, and a cell beside it along x or z inside the
/// grid is of no kind in `shown` and has none under it either: open air
/// it can spray into.
fn open_beside(
    em: &mut Em<'_>,
    build: &Build,
    masks: Masks,
    cell: Id,
    [column, height, row]: [Id; 3],
) -> Result<Id, LaneError> {
    let grid = build.grid;
    let kinds = build.kinds;
    let shown = masks.shown;
    let none = em.u(0)?;
    let bottom = em.u(grid.deep - 1)?;
    let floored = em.eq(height, bottom)?;
    let lower_step = em.u(grid.wide)?;
    let lower = em.add(cell, lower_step)?;
    let lower = em.pick(floored, cell, lower)?;
    let under = em.cell(lower)?;
    let under = em.shr(under, kinds.shift())?;
    let under = em.and_u(under, kinds.mask())?;
    let held_up = em.bit(shown, under)?;
    let wet_under = em.bit(masks.liquid, under)?;
    let dry_under = em.not(wet_under)?;
    let held_up = em.both(held_up, dry_under)?;
    let rests = em.either(floored, held_up)?;
    let free = em.not(rests)?;
    let mut open = em.ne(none, none)?;
    let plane = grid.wide.trailing_zeros() + grid.deep.trailing_zeros();
    for (along, step, side) in [(column, 1_u32, grid.wide), (row, 1 << plane, grid.tall)] {
        let last = em.u(side - 1)?;
        let stride = em.u(step)?;
        for low in [true, false] {
            let inside = if low {
                em.ne(along, none)?
            } else {
                em.ne(along, last)?
            };
            let at = if low {
                em.sub(cell, stride)?
            } else {
                em.add(cell, stride)?
            };
            let at = em.pick(inside, at, cell)?;
            let held = em.cell(at)?;
            let kind = em.shr(held, kinds.shift())?;
            let kind = em.and_u(kind, kinds.mask())?;
            let seen = em.bit(shown, kind)?;
            let empty = em.not(seen)?;
            let empty = em.both(empty, inside)?;
            let beneath = em.add(at, lower_step)?;
            let beneath = em.pick(floored, at, beneath)?;
            let below = em.cell(beneath)?;
            let below = em.shr(below, kinds.shift())?;
            let below = em.and_u(below, kinds.mask())?;
            let below = em.bit(shown, below)?;
            let drop = em.not(below)?;
            let drop = em.both(drop, empty)?;
            open = em.either(open, drop)?;
        }
    }
    em.both(open, free)
}

/// The pass that rebuilds every brick's word, one brick a lane; dispatch
/// it over [`super::bricks`] lanes.
pub fn filled(build: &Build) -> Result<Vec<u32>, LaneError> {
    refuse_build("render3::filled", build)?;
    let layout = build.layout();
    let grid = build.grid;
    let mut kernel = kernel(build)?;
    let mut em = Em::read_by(&mut kernel, build)?;
    let brick = em.invocation(build.subgroup)?;
    let count = em.u(layout.bricks)?;
    let inside = em.lt(brick, count)?;
    let masks = Masks::of(&mut em, build)?;
    let across = (grid.wide >> BRICK_BITS).trailing_zeros();
    let down = (grid.deep >> BRICK_BITS).trailing_zeros();
    let x = em.and_u(brick, (1 << across) - 1)?;
    let y = em.shr(brick, across)?;
    let y = em.and_u(y, (1 << down) - 1)?;
    let z = em.shr(brick, across + down)?;
    let _ = em.when(inside, &[], |em| {
        rebuilt(em, build, [x, y, z], masks, brick)?;
        Ok(Vec::new())
    })?;
    kernel.finish()
}

/// The pass that rebuilds the word of each brick an oct listed this frame
/// touches, once a brick however many listed octs touch it. It runs over
/// the listed octs, as the engine's step does, after that step. A brick's
/// claim word holds the frame that last claimed it, plus one.
pub fn occupied(build: &Build) -> Result<Vec<u32>, LaneError> {
    refuse_build("render3::occupied", build)?;
    if let Some(sparse) = build.sparse {
        return occupied_sparse(build, sparse);
    }
    let layout = build.layout();
    let grid = build.grid;
    let mut kernel = kernel(build)?;
    let mut em = Em::read_by(&mut kernel, build)?;
    let slot = em.invocation(build.subgroup)?;
    let grand = em.load_u(GRAND, 0)?;
    let dump = em.load_u(PARAMS, 1)?;
    let claimed = em.load(DENSE, dump)?;
    let total = em.add(grand, claimed)?;
    let listed = em.lt(slot, total)?;
    let masks = Masks::of(&mut em, build)?;
    let _ = em.when(listed, &[], |em| {
        let oct = em.load(DENSE, slot)?;
        let frame = em.load_u(PARAMS, 0)?;
        let phase = em.and_u(frame, 1)?;
        let one = em.u(1)?;
        let stamp = em.add(frame, one)?;
        let active = em.u(word(layout.active()))?;
        let mark = em.add(active, oct)?;
        em.store(PIXELS, mark, stamp)?;
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
            let _ = em.when(mine, &[], |em| {
                rebuilt(em, build, at, masks, brick)?;
                Ok(Vec::new())
            })?;
        }
        Ok(Vec::new())
    })?;
    kernel.finish()
}

fn occupied_sparse(build: &Build, sparse: Sparse) -> Result<Vec<u32>, LaneError> {
    let layout = build.layout();
    let grid = build.grid;
    let subgroup = build.subgroup;
    let mut kernel = kernel(build)?;
    let mut em = Em::read_by(&mut kernel, build)?;
    let slot = em.invocation(subgroup)?;
    let k = sparse.octs_bits();
    let grand = em.load_u(GRAND, 0)?;
    let whole = em.load_u(sparse.whole_count, 0)?;
    let short = em.u(subgroup - 1)?;
    let rounded = em.add(grand, short)?;
    let rounded = em.and_u(rounded, !(subgroup - 1))?;
    let in_list = em.lt(slot, rounded)?;
    let in_dense = em.lt(slot, grand)?;
    let past = em.sub(slot, rounded)?;
    let which = em.shr(past, k)?;
    let local = em.and_u(past, (1 << k) - 1)?;
    let in_whole = em.lt(which, whole)?;
    let listed = em.pick_t(in_list, in_dense, in_whole)?;
    let masks = Masks::of(&mut em, build)?;
    let origin = em.origin().ok_or(LaneError::NoSuchForm {
        operation: "render3::occupied",
        because: super::UNFIT_SPARSE,
    })?;
    let _ = em.when(listed, &[], |em| {
        let none = em.u(0)?;
        let dense_at = em.pick(in_list, slot, none)?;
        let whole_at = em.pick(in_list, none, which)?;
        let from_list = em.load(DENSE, dense_at)?;
        let brick = em.load(sparse.whole_list, whole_at)?;
        let lifted = em.shl(brick, k)?;
        let from_brick = em.add(lifted, local)?;
        let oct = em.pick(in_list, from_list, from_brick)?;
        let brick = em.shr(oct, k)?;
        let inner = em.and_u(oct, (1 << k) - 1)?;
        let lattice = sparse.lattice.map(u32::trailing_zeros);
        let mut point = [oct; 3];
        let (mut inner_shift, mut outer_shift) = (0, 0);
        for axis in 0..3 {
            let bits = of(sparse.bits, axis);
            let across = of(lattice, axis);
            let low = em.shr(inner, inner_shift)?;
            let low = em.and_u(low, (1 << bits) - 1)?;
            let high = em.shr(brick, outer_shift)?;
            let high = em.and_u(high, (1 << across) - 1)?;
            let high = em.shl(high, bits)?;
            put(&mut point, axis, em.or(high, low)?);
            inner_shift += bits;
            outer_shift += across;
        }
        let frame = em.load_u(PARAMS, 0)?;
        let phase = em.and_u(frame, 1)?;
        let one = em.u(1)?;
        let stamp = em.add(frame, one)?;
        let sides = sparse.sides();
        let window = [grid.wide, grid.deep, grid.tall];
        let halves = window.map(|side| (side / 2).trailing_zeros());
        let mut seen = em.eq(none, none)?;
        let mut number = none;
        let mut shift = 0;
        for axis in 0..3 {
            let half = em.shr(of(origin, axis), 1)?;
            let at = em.sub(of(point, axis), half)?;
            let at = em.and_u(at, of(sides, axis) / 2 - 1)?;
            let limit = em.u(of(window, axis) / 2)?;
            let inside = em.lt(at, limit)?;
            seen = em.both(seen, inside)?;
            let lifted = em.shl(at, shift)?;
            number = em.or(number, lifted)?;
            shift += of(halves, axis);
        }
        let _ = em.when(seen, &[], |em| {
            let active = em.u(word(layout.active()))?;
            let mark = em.add(active, number)?;
            em.store(PIXELS, mark, stamp)?;
            Ok(Vec::new())
        })?;
        let mut spans = [[oct; 2]; 3];
        let mut within = [[seen; 2]; 3];
        for axis in 0..3 {
            let side = of(sides, axis);
            let low = em.shl(of(point, axis), 1)?;
            let low = em.add(low, phase)?;
            let low = em.and_u(low, side - 1)?;
            let high = em.add(low, one)?;
            let high = em.and_u(high, side - 1)?;
            let limit = em.u(of(window, axis))?;
            let mut held = [low; 2];
            let mut inside = [seen; 2];
            for (at, (slot, flag)) in [low, high]
                .into_iter()
                .zip(held.iter_mut().zip(inside.iter_mut()))
            {
                let moved = em.sub(at, of(origin, axis))?;
                let moved = em.and_u(moved, side - 1)?;
                *flag = em.lt(moved, limit)?;
                *slot = em.shr(moved, BRICK_BITS)?;
            }
            put(&mut spans, axis, held);
            put(&mut within, axis, inside);
        }
        let across = (grid.wide >> BRICK_BITS).trailing_zeros();
        let down = (grid.deep >> BRICK_BITS).trailing_zeros();
        let claims = em.u(word(layout.claims()))?;
        for corner in 0..8_usize {
            let pick = |axis: usize| usize::from((corner >> axis) & 1 == 1);
            let at = [0, 1, 2].map(|axis| of(spans, axis).get(pick(axis)).copied().unwrap_or(oct));
            let flags =
                [0, 1, 2].map(|axis| of(within, axis).get(pick(axis)).copied().unwrap_or(none));
            let inside = em.both(flags[0], flags[1])?;
            let inside = em.both(inside, flags[2])?;
            let _ = em.when(inside, &[], |em| {
                let z = em.shl(at[2], across + down)?;
                let y = em.shl(at[1], across)?;
                let brick = em.or(z, y)?;
                let brick = em.or(brick, at[0])?;
                let claim = em.add(claims, brick)?;
                let before = em.kernel.atomic_exchange_at(PIXELS, claim, stamp)?;
                let mine = em.ne(before, stamp)?;
                let _ = em.when(mine, &[], |em| {
                    rebuilt(em, build, at, masks, brick)?;
                    Ok(Vec::new())
                })?;
                Ok(Vec::new())
            })?;
        }
        Ok(Vec::new())
    })?;
    kernel.finish()
}

/// The light each brick of `grid` gives off, red, green and blue a brick,
/// summed over its cells in the order [`filled`] reads them, and its gas,
/// [`GAS_WORDS`] a brick: what the rebuild writes from
/// [`super::Layout::emission`] and [`super::Layout::gas`] on.
#[must_use]
pub fn emission(grid: Grid, kinds: Kinds, table: &Table, cells: &[u32]) -> (Vec<f32>, Vec<f32>) {
    let count = super::bricks(grid) as usize;
    let mut light = vec![0.0_f32; 3 * count];
    let mut gas = vec![0.0_f32; GAS_WORDS * count];
    let [across, down, _] = super::brick_sides(grid);
    for brick in 0..count {
        let brick_word = super::word(brick);
        let bx = brick_word % across;
        let by = (brick_word / across) % down;
        let bz = brick_word / (across * down);
        let mut sum = [0.0_f32; 3];
        let mut thick = [0.0_f32; GAS_WORDS];
        for run in 0..CELLS {
            let x = (bx << BRICK_BITS) + (run & (SIDE - 1));
            let y = (by << BRICK_BITS) + ((run >> BRICK_BITS) & (SIDE - 1));
            let z = (bz << BRICK_BITS) + (run >> (2 * BRICK_BITS));
            let at = ((z * grid.deep + y) * grid.wide + x) as usize;
            let kind = kinds.of(cells.get(at).copied().unwrap_or(0));
            let made = table.material(kind);
            if made.gas {
                let taken = 1.0 - made.clear.clamp(0.0, 1.0);
                for (slot, value) in thick.iter_mut().zip(
                    [taken]
                        .into_iter()
                        .chain(made.albedo.map(|albedo| albedo * taken)),
                ) {
                    *slot += value;
                }
            }
            if made.glow > 0.0 {
                for (slot, albedo) in sum.iter_mut().zip(made.albedo) {
                    *slot += albedo * made.glow;
                }
            }
        }
        for (channel, value) in sum.into_iter().enumerate() {
            if let Some(slot) = light.get_mut(3 * brick + channel) {
                *slot = value;
            }
        }
        for (field, value) in thick.into_iter().enumerate() {
            if let Some(slot) = gas.get_mut(GAS_WORDS * brick + field) {
                *slot = value * (1.0 / CELLS as f32);
            }
        }
    }
    (light, gas)
}

/// The water of each brick of `grid`, [`WATER_WORDS`] a brick: the share of
/// its cells of a liquid kind, and the share that fall free: those whose
/// word has the bit their material's `falls` names, or, for a material
/// that names none, those above a cell of a liquid or of no kind `table`
/// shows, inside the grid, and with a cell of no kind it shows beside them
/// along x or z inside the grid, and none it shows under that one: what
/// the rebuild writes from [`super::Layout::water`] on.
#[must_use]
pub fn watered(grid: Grid, kinds: Kinds, table: &Table, cells: &[u32]) -> Vec<f32> {
    let count = super::bricks(grid) as usize;
    let liquid = table.liquids();
    let shown = table.shown();
    let mut out = vec![0.0_f32; WATER_WORDS * count];
    let [across, down, _] = super::brick_sides(grid);
    let kind_at = |x: u32, y: u32, z: u32| {
        let at = ((z * grid.deep + y) * grid.wide + x) as usize;
        kinds.of(cells.get(at).copied().unwrap_or(0))
    };
    let bit = |mask: u32, kind: u32| (mask >> (kind & 31)) & 1 == 1;
    for brick in 0..count {
        let brick_word = super::word(brick);
        let bx = brick_word % across;
        let by = (brick_word / across) % down;
        let bz = brick_word / (across * down);
        let mut wet = 0_u32;
        let mut spray = 0_u32;
        for run in 0..CELLS {
            let x = (bx << BRICK_BITS) + (run & (SIDE - 1));
            let y = (by << BRICK_BITS) + ((run >> BRICK_BITS) & (SIDE - 1));
            let z = (bz << BRICK_BITS) + (run >> (2 * BRICK_BITS));
            let kind = kind_at(x, y, z);
            if !bit(liquid, kind) {
                continue;
            }
            wet += 1;
            let falls = table.material(kind).falls;
            if falls > 0 {
                let at = ((z * grid.deep + y) * grid.wide + x) as usize;
                let held = cells.get(at).copied().unwrap_or(0);
                if (held >> ((falls - 1) & 31)) & 1 == 1 {
                    spray += 1;
                }
                continue;
            }
            let rests = y + 1 >= grid.deep || {
                let under = kind_at(x, y + 1, z);
                bit(shown, under) && !bit(liquid, under)
            };
            if rests {
                continue;
            }
            let beside = [
                (x > 0).then(|| (x - 1, z)),
                (x + 1 < grid.wide).then(|| (x + 1, z)),
                (z > 0).then(|| (x, z - 1)),
                (z + 1 < grid.tall).then(|| (x, z + 1)),
            ];
            let open = |(at_x, at_z): (u32, u32)| {
                !bit(shown, kind_at(at_x, y, at_z)) && !bit(shown, kind_at(at_x, y + 1, at_z))
            };
            if beside.into_iter().flatten().any(open) {
                spray += 1;
            }
        }
        for (field, value) in [wet, spray].into_iter().enumerate() {
            if let Some(slot) = out.get_mut(WATER_WORDS * brick + field) {
                *slot = value as f32 * (1.0 / CELLS as f32);
            }
        }
    }
    out
}
