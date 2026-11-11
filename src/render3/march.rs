//! The integer march of a ray through the grid, on the CPU and emitted.
//!
//! A ray starts at a point held in sixteenths of a cell and runs along an
//! integer direction. On each axis it keeps the cell it is in and how far,
//! in sixteenths along that axis, it is to the next cell wall. The next
//! wall the ray reaches is the one with the least `next / steep`, compared
//! across axes by cross products, so every step is exact. On a tie, x
//! goes before y and y before z.
//!
//! An empty brick is crossed in one jump: the ray leaves it through the
//! wall it reaches first, and on every other axis it crosses the walls it
//! would have crossed one by one before that, so a jump visits the same
//! cells a walk would, minus the empty ones. A ray that starts outside the
//! grid jumps to where it enters it in the same way.

use simdr::lanes::LaneError;
use simdr::module::Id;

use super::emit::Em;
use super::{BRICK_BITS, INSIDE, MISS, UNIT, UNIT_BITS, of, put};
use crate::layout::{PIXELS, STATE};
use crate::sand::Grid;
use crate::view3::Kinds;

const BIAS: u32 = 1 << 14;
const BRICK: u32 = 1 << BRICK_BITS;

/// The sides of `grid`: x, y and z.
#[must_use]
pub const fn sides(grid: Grid) -> [u32; 3] {
    [grid.wide, grid.deep, grid.tall]
}

/// Where a ray is: its cell on each axis, the sixteenths along each axis to
/// the next wall, how steep it runs along each axis, which way, and the
/// face it last came in through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ray {
    pub cell: [u32; 3],
    pub next: [u32; 3],
    pub steep: [u32; 3],
    pub ahead: [bool; 3],
    pub face: u32,
}

const fn later_or_tied(left: u32, right: u32) -> bool {
    left >= right
}

/// How many walls an axis crosses before a jump ends at `ends / steep_at`
/// on the axis that ends it: walls at the end itself count only when the
/// axis comes `before` the ending one.
const fn crossed(before: bool, ends: u32, steep: u32, ending: u32, next: u32) -> u32 {
    let product = ends.wrapping_mul(steep);
    let ending = if ending == 0 { 1 } else { ending };
    let (counts, reach) = if before {
        (true, product / ending)
    } else {
        (product != 0, product.wrapping_sub(1) / ending)
    };
    if counts && reach >= next {
        ((reach - next) >> UNIT_BITS) + 1
    } else {
        0
    }
}

impl Ray {
    /// The ray from `origin`, in sixteenths of a cell, along `direction`.
    #[must_use]
    pub fn from(origin: [i32; 3], direction: [i32; 3]) -> Self {
        let mut ray = Self {
            cell: [0; 3],
            next: [0; 3],
            steep: [0; 3],
            ahead: [false; 3],
            face: INSIDE,
        };
        for axis in 0..3 {
            let at = of(origin, axis) as u32;
            let towards = of(direction, axis);
            let steep = towards.unsigned_abs();
            let ahead = towards > 0;
            let part = at & (UNIT - 1);
            let held = (at.wrapping_add(BIAS) >> UNIT_BITS).wrapping_sub(BIAS >> UNIT_BITS);
            put(&mut ray.cell, axis, held);
            let held = steep;
            put(&mut ray.steep, axis, held);
            let held = ahead;
            put(&mut ray.ahead, axis, held);
            let held = if steep == 0 {
                UNIT
            } else if ahead {
                UNIT - part
            } else {
                part
            };
            put(&mut ray.next, axis, held);
        }
        ray
    }

    fn moved(mut self, by: [u32; 3]) -> Self {
        for axis in 0..3 {
            let held = if of(self.ahead, axis) {
                of(self.cell, axis).wrapping_add(of(by, axis))
            } else {
                of(self.cell, axis).wrapping_sub(of(by, axis))
            };
            put(&mut self.cell, axis, held);
            let held = of(self.next, axis).wrapping_add(of(by, axis) << UNIT_BITS);
            put(&mut self.next, axis, held);
        }
        self
    }

    const fn face_of(&self, axis: usize) -> u32 {
        2 * axis as u32 + if of(self.ahead, axis) { 0 } else { 1 }
    }

    /// Whether the ray's cell lies in a grid of `sides`.
    #[must_use]
    pub fn inside(&self, sides: [u32; 3]) -> bool {
        (0..3).all(|axis| of(self.cell, axis) < of(sides, axis))
    }

    /// The ray moved to the first cell of a grid of `sides` it reaches, or
    /// none when it never reaches one. A ray already inside stays put.
    #[must_use]
    pub fn entered(self, sides: [u32; 3]) -> Option<Self> {
        let mut ends = [0; 3];
        let mut over = [1; 3];
        let mut walls = [0; 3];
        let mut within = [false; 3];
        let mut fine = true;
        for axis in 0..3 {
            let cell = of(self.cell, axis);
            let held = cell < of(sides, axis);
            put(&mut within, axis, held);
            let below = cell >= 1 << 31;
            let towards = of(self.steep, axis) != 0
                && if of(self.ahead, axis) {
                    below
                } else {
                    !below && cell >= of(sides, axis)
                };
            fine &= of(within, axis) || towards;
            let held = if of(self.ahead, axis) {
                0_u32.wrapping_sub(cell)
            } else {
                cell.wrapping_sub(of(sides, axis)).wrapping_add(1)
            };
            put(&mut walls, axis, held);
            if !of(within, axis) {
                let held =
                    of(self.next, axis).wrapping_add(of(walls, axis).wrapping_sub(1) << UNIT_BITS);
                put(&mut ends, axis, held);
                let held = of(self.steep, axis);
                put(&mut over, axis, held);
            }
        }
        if !fine {
            return None;
        }
        if within.iter().all(|&held| held) {
            return Some(self);
        }
        let later = |left: usize, right: usize| {
            !of(within, left)
                && (of(within, right)
                    || later_or_tied(
                        of(ends, left).wrapping_mul(of(over, right)),
                        of(ends, right).wrapping_mul(of(over, left)),
                    ))
        };
        let last = if later(2, 0) && later(2, 1) {
            2
        } else if later(1, 0) {
            1
        } else {
            0
        };
        let mut by = [0; 3];
        for axis in 0..3 {
            let held = if axis == last {
                of(walls, axis)
            } else {
                crossed(
                    axis < last,
                    of(ends, last),
                    of(self.steep, axis),
                    of(self.steep, last),
                    of(self.next, axis),
                )
            };
            put(&mut by, axis, held);
        }
        let mut ray = self.moved(by);
        ray.face = ray.face_of(last);
        ray.inside(sides).then_some(ray)
    }

    fn first(&self, ends: [u32; 3]) -> usize {
        let earlier = |left: usize, right: usize| {
            of(ends, left).wrapping_mul(of(self.steep, right))
                <= of(ends, right).wrapping_mul(of(self.steep, left))
        };
        if earlier(0, 1) && earlier(0, 2) {
            0
        } else if earlier(1, 2) {
            1
        } else {
            2
        }
    }

    /// The ray one wall on.
    #[must_use]
    pub fn stepped(self) -> Self {
        let axis = self.first(self.next);
        let mut by = [0; 3];
        let held = 1;
        put(&mut by, axis, held);
        let mut ray = self.moved(by);
        ray.face = ray.face_of(axis);
        ray
    }

    /// The ray past the brick it is in, through the brick's first wall.
    #[must_use]
    pub fn jumped(self) -> Self {
        let mut walls = [0; 3];
        let mut ends = [0; 3];
        for axis in 0..3 {
            let within = of(self.cell, axis) & (BRICK - 1);
            let held = if of(self.ahead, axis) {
                BRICK - within
            } else {
                within + 1
            };
            put(&mut walls, axis, held);
            let held = of(self.next, axis).wrapping_add((of(walls, axis) - 1) << UNIT_BITS);
            put(&mut ends, axis, held);
        }
        let first = self.first(ends);
        let mut by = [0; 3];
        for axis in 0..3 {
            let held = if axis == first {
                of(walls, axis)
            } else {
                crossed(
                    axis < first,
                    of(ends, first),
                    of(self.steep, axis),
                    of(self.steep, first),
                    of(self.next, axis),
                )
            };
            put(&mut by, axis, held);
        }
        let mut ray = self.moved(by);
        ray.face = ray.face_of(first);
        ray
    }
}

/// What a ray reads: the grid, its words, the occupancy of its bricks, how
/// words map to kinds, and which kinds it sees and which it cannot see
/// through.
#[derive(Debug, Clone, Copy)]
pub struct World<'held> {
    pub grid: Grid,
    pub kinds: Kinds,
    pub visible: u32,
    pub opaque: u32,
    pub cells: &'held [u32],
    pub bricks: &'held [u32],
}

/// The index of the cell at `cell`.
#[must_use]
pub fn index(grid: Grid, cell: [u32; 3]) -> u32 {
    let wide = grid.wide.trailing_zeros();
    let plane = wide + grid.deep.trailing_zeros();
    (cell[2] << plane) | (cell[1] << wide) | cell[0]
}

/// The index of the brick that holds `cell`.
#[must_use]
pub fn brick_of(grid: Grid, cell: [u32; 3]) -> u32 {
    let wide = grid.wide.trailing_zeros() - BRICK_BITS;
    let plane = wide + grid.deep.trailing_zeros() - BRICK_BITS;
    ((cell[2] >> BRICK_BITS) << plane) | ((cell[1] >> BRICK_BITS) << wide) | (cell[0] >> BRICK_BITS)
}

/// The most runs a march over `grid` takes.
#[must_use]
pub const fn limit(grid: Grid) -> u32 {
    grid.wide + grid.deep + grid.tall + 2
}

const fn bit(mask: u32, at: u32) -> bool {
    (mask >> (at & 31)) & 1 == 1
}

impl World<'_> {
    pub(crate) fn kind_at(&self, ray: &Ray) -> (u32, u32, bool) {
        let cell = index(self.grid, ray.cell);
        let full = self
            .bricks
            .get(brick_of(self.grid, ray.cell) as usize)
            .copied()
            .unwrap_or(0)
            != 0;
        let word = self.cells.get(cell as usize).copied().unwrap_or(0);
        (cell, self.kinds.of(word), full)
    }

    /// What the pixel of a ray from `origin` along `direction` shows: the
    /// first cell it sees and, past cells it can see through, the first it
    /// cannot. Each is a cell index and a word holding the face it came in
    /// through and, from bit 3 on, the run it was found in; a miss is
    /// [`MISS`] and 0.
    #[must_use]
    pub fn seen(&self, origin: [i32; 3], direction: [i32; 3]) -> [u32; 4] {
        let sides = sides(self.grid);
        let mut out = [MISS, 0, MISS, 0];
        let Some(mut ray) = Ray::from(origin, direction).entered(sides) else {
            return out;
        };
        let mut through = false;
        for run in 0..limit(self.grid) {
            let (cell, kind, full) = self.kind_at(&ray);
            let mask = if through { self.opaque } else { self.visible };
            let stops = full && bit(mask, kind);
            let clear = bit(self.visible & !self.opaque, kind);
            let found = ray.face | (run << 3);
            if !through && stops {
                out[0] = cell;
                out[1] = found;
            }
            let enters = !through && stops && clear;
            if stops && !enters {
                out[2] = cell;
                out[3] = found;
                return out;
            }
            through |= enters;
            ray = if full { ray.stepped() } else { ray.jumped() };
            if !ray.inside(sides) {
                return out;
            }
        }
        out
    }

    /// Whether a ray from `origin` along `direction` meets a cell it cannot
    /// see through before it leaves the grid.
    #[must_use]
    pub fn blocked(&self, origin: [i32; 3], direction: [i32; 3]) -> bool {
        let sides = sides(self.grid);
        let Some(mut ray) = Ray::from(origin, direction).entered(sides) else {
            return false;
        };
        for _ in 0..limit(self.grid) {
            let (_, kind, full) = self.kind_at(&ray);
            if full && bit(self.opaque, kind) {
                return true;
            }
            ray = if full { ray.stepped() } else { ray.jumped() };
            if !ray.inside(sides) {
                return false;
            }
        }
        false
    }

    /// The first cell a ray sees, walked one wall at a time from its start
    /// with no brick read at all, and the face it came in through: what
    /// [`World::seen`] finds first, by the plain walk.
    #[must_use]
    pub fn walked(&self, origin: [i32; 3], direction: [i32; 3]) -> Option<(u32, u32)> {
        let sides = sides(self.grid);
        let mut ray = Ray::from(origin, direction);
        let far = 3 * (1 << 10) + sides.iter().sum::<u32>() + 3;
        for _ in 0..far {
            if ray.inside(sides) {
                let cell = index(self.grid, ray.cell);
                let word = self.cells.get(cell as usize).copied().unwrap_or(0);
                if bit(self.visible, self.kinds.of(word)) {
                    return Some((cell, ray.face));
                }
            }
            ray = ray.stepped();
        }
        None
    }
}

/// The emitted form of [`Ray`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct Marching {
    pub cell: [Id; 3],
    pub next: [Id; 3],
    pub steep: [Id; 3],
    pub ahead: [Id; 3],
    pub face: Id,
}

/// [`crossed`], emitted, with `before` known only on the device.
fn crossed_em(
    em: &mut Em<'_>,
    before: Id,
    ends: Id,
    steep: Id,
    ending: Id,
    next: Id,
) -> Result<Id, LaneError> {
    let product = em.mul(ends, steep)?;
    let none = em.u(0)?;
    let one = em.u(1)?;
    let zero = em.eq(ending, none)?;
    let ending = em.pick(zero, one, ending)?;
    let less = em.sub(product, one)?;
    let dividend = em.pick(before, product, less)?;
    let reach = em.div(dividend, ending)?;
    let some = em.ne(product, none)?;
    let counts = em.either(before, some)?;
    let far = em.ge(reach, next)?;
    let counts = em.both(counts, far)?;
    let past = em.sub(reach, next)?;
    let past = em.shr(past, UNIT_BITS)?;
    let past = em.add(past, one)?;
    em.pick(counts, past, none)
}

impl Marching {
    /// The ray from `origin` along `direction`, each a two's complement word
    /// an axis.
    pub fn from(em: &mut Em<'_>, origin: [Id; 3], direction: [Id; 3]) -> Result<Self, LaneError> {
        let none = em.u(0)?;
        let unit = em.u(UNIT)?;
        let bias = em.u(BIAS)?;
        let back = em.u(BIAS >> UNIT_BITS)?;
        let sign = em.u(1 << 31)?;
        let mut cell = [none; 3];
        let mut next = [none; 3];
        let mut steep = [none; 3];
        let mut ahead = [none; 3];
        for axis in 0..3 {
            let at = of(origin, axis);
            let towards = of(direction, axis);
            let below = em.ge(towards, sign)?;
            let flipped = em.sub(none, towards)?;
            let size = em.pick(below, flipped, towards)?;
            let moving = em.ne(towards, none)?;
            let up = em.not(below)?;
            let up = em.both(up, moving)?;
            let part = em.and_u(at, UNIT - 1)?;
            let shifted = em.add(at, bias)?;
            let shifted = em.shr(shifted, UNIT_BITS)?;
            let held = em.sub(shifted, back)?;
            put(&mut cell, axis, held);
            let rest = em.sub(unit, part)?;
            let gap = em.pick(up, rest, part)?;
            let still = em.eq(size, none)?;
            let held = em.pick(still, unit, gap)?;
            put(&mut next, axis, held);
            let held = size;
            put(&mut steep, axis, held);
            let held = up;
            put(&mut ahead, axis, held);
        }
        let face = em.u(INSIDE)?;
        Ok(Self {
            cell,
            next,
            steep,
            ahead,
            face,
        })
    }

    fn moved(&self, em: &mut Em<'_>, by: [Id; 3]) -> Result<Self, LaneError> {
        let mut ray = *self;
        for axis in 0..3 {
            let up = em.add(of(self.cell, axis), of(by, axis))?;
            let down = em.sub(of(self.cell, axis), of(by, axis))?;
            let held = em.pick(of(self.ahead, axis), up, down)?;
            put(&mut ray.cell, axis, held);
            let far = em.shl(of(by, axis), UNIT_BITS)?;
            let held = em.add(of(self.next, axis), far)?;
            put(&mut ray.next, axis, held);
        }
        Ok(ray)
    }

    fn face_of(&self, em: &mut Em<'_>, axis: [Id; 3]) -> Result<Id, LaneError> {
        let mut face = em.u(0)?;
        for (at, &chosen) in axis.iter().enumerate() {
            let low = em.u(2 * at as u32)?;
            let high = em.u(2 * at as u32 + 1)?;
            let this = em.pick(of(self.ahead, at), low, high)?;
            face = em.pick(chosen, this, face)?;
        }
        Ok(face)
    }

    pub fn inside(&self, em: &mut Em<'_>, sides: [u32; 3]) -> Result<Id, LaneError> {
        let mut all = None;
        for axis in 0..3 {
            let side = em.u(of(sides, axis))?;
            let held = em.lt(of(self.cell, axis), side)?;
            all = Some(match all {
                None => held,
                Some(before) => em.both(before, held)?,
            });
        }
        all.ok_or(LaneError::BadCarry {
            given: 0,
            wanted: 3,
        })
    }

    fn chosen(
        em: &mut Em<'_>,
        choose: impl Fn(&mut Em<'_>, usize, usize) -> Result<Id, LaneError>,
        latest: bool,
    ) -> Result<[Id; 3], LaneError> {
        let (lead, middle, tail) = if latest { (2, 1, 0) } else { (0, 1, 2) };
        let lead_first = choose(em, lead, middle)?;
        let lead_other = choose(em, lead, tail)?;
        let lead_wins = em.both(lead_first, lead_other)?;
        let middle_wins = choose(em, middle, tail)?;
        let not_lead = em.not(lead_wins)?;
        let middle_wins = em.both(not_lead, middle_wins)?;
        let either = em.either(lead_wins, middle_wins)?;
        let tail_wins = em.not(either)?;
        let mut out = [lead_wins; 3];
        let held = lead_wins;
        put(&mut out, lead, held);
        let held = middle_wins;
        put(&mut out, middle, held);
        let held = tail_wins;
        put(&mut out, tail, held);
        Ok(out)
    }

    fn by_choice(em: &mut Em<'_>, chosen: [Id; 3], values: [Id; 3]) -> Result<Id, LaneError> {
        let mut out = values[0];
        for axis in 1..3 {
            out = em.pick(of(chosen, axis), of(values, axis), out)?;
        }
        Ok(out)
    }

    fn spread(
        &self,
        em: &mut Em<'_>,
        chosen: [Id; 3],
        ends: [Id; 3],
        walls: [Id; 3],
    ) -> Result<Self, LaneError> {
        let end = Self::by_choice(em, chosen, ends)?;
        let ending = Self::by_choice(em, chosen, self.steep)?;
        let [on_x, on_y, on_z] = chosen;
        let after_x = em.either(on_y, on_z)?;
        let never = em.ne(end, end)?;
        let before = [after_x, on_z, never];
        let mut by = walls;
        for axis in 0..3 {
            let count = crossed_em(
                em,
                of(before, axis),
                end,
                of(self.steep, axis),
                ending,
                of(self.next, axis),
            )?;
            let held = em.pick(of(chosen, axis), of(walls, axis), count)?;
            put(&mut by, axis, held);
        }
        let _ = on_x;
        let mut ray = self.moved(em, by)?;
        ray.face = self.face_of(em, chosen)?;
        Ok(ray)
    }

    /// The ray moved to the first cell of the grid of `sides` it reaches,
    /// and whether it reaches one.
    pub fn entered(&self, em: &mut Em<'_>, sides: [u32; 3]) -> Result<(Self, Id), LaneError> {
        let none = em.u(0)?;
        let one = em.u(1)?;
        let sign = em.u(1 << 31)?;
        let mut ends = [none; 3];
        let mut over = [one; 3];
        let mut walls = [none; 3];
        let mut within = [none; 3];
        let mut fine = em.eq(none, none)?;
        let mut all_within = fine;
        for axis in 0..3 {
            let cell = of(self.cell, axis);
            let side = em.u(of(sides, axis))?;
            let held = em.lt(cell, side)?;
            put(&mut within, axis, held);
            let below = em.ge(cell, sign)?;
            let moving = em.ne(of(self.steep, axis), none)?;
            let above = em.not(below)?;
            let past = em.ge(cell, side)?;
            let beyond = em.both(above, past)?;
            let towards = em.pick_t(of(self.ahead, axis), below, beyond)?;
            let towards = em.both(moving, towards)?;
            let here = em.either(of(within, axis), towards)?;
            fine = em.both(fine, here)?;
            all_within = em.both(all_within, of(within, axis))?;
            let up = em.sub(none, cell)?;
            let down = em.sub(cell, side)?;
            let down = em.add(down, one)?;
            let held = em.pick(of(self.ahead, axis), up, down)?;
            put(&mut walls, axis, held);
            let more = em.sub(of(walls, axis), one)?;
            let more = em.shl(more, UNIT_BITS)?;
            let end = em.add(of(self.next, axis), more)?;
            let held = em.pick(of(within, axis), none, end)?;
            put(&mut ends, axis, held);
            let held = em.pick(of(within, axis), one, of(self.steep, axis))?;
            put(&mut over, axis, held);
        }
        let chosen = Self::chosen(
            em,
            |em, left, right| {
                let outside = em.not(of(within, left))?;
                let a = em.mul(of(ends, left), of(over, right))?;
                let b = em.mul(of(ends, right), of(over, left))?;
                let later = em.ge(a, b)?;
                let later = em.either(of(within, right), later)?;
                em.both(outside, later)
            },
            true,
        )?;
        let jumped = self.spread(em, chosen, ends, walls)?;
        let mut ray = *self;
        for axis in 0..3 {
            let held = em.pick(all_within, of(self.cell, axis), of(jumped.cell, axis))?;
            put(&mut ray.cell, axis, held);
            let held = em.pick(all_within, of(self.next, axis), of(jumped.next, axis))?;
            put(&mut ray.next, axis, held);
        }
        ray.face = em.pick(all_within, self.face, jumped.face)?;
        let inside = ray.inside(em, sides)?;
        let alive = em.both(fine, inside)?;
        Ok((ray, alive))
    }

    fn first(&self, em: &mut Em<'_>, ends: [Id; 3]) -> Result<[Id; 3], LaneError> {
        let steep = self.steep;
        Self::chosen(
            em,
            |em, left, right| {
                let a = em.mul(of(ends, left), of(steep, right))?;
                let b = em.mul(of(ends, right), of(steep, left))?;
                em.le(a, b)
            },
            false,
        )
    }

    /// The ray one wall on.
    pub fn stepped(&self, em: &mut Em<'_>) -> Result<Self, LaneError> {
        let chosen = self.first(em, self.next)?;
        let mut by = [self.face; 3];
        for axis in 0..3 {
            let held = em.flag(of(chosen, axis))?;
            put(&mut by, axis, held);
        }
        let mut ray = self.moved(em, by)?;
        ray.face = self.face_of(em, chosen)?;
        Ok(ray)
    }

    /// The ray past the brick it is in.
    pub fn jumped(&self, em: &mut Em<'_>) -> Result<Self, LaneError> {
        let one = em.u(1)?;
        let brick = em.u(BRICK)?;
        let mut walls = [one; 3];
        let mut ends = [one; 3];
        for axis in 0..3 {
            let within = em.and_u(of(self.cell, axis), BRICK - 1)?;
            let up = em.sub(brick, within)?;
            let down = em.add(within, one)?;
            let held = em.pick(of(self.ahead, axis), up, down)?;
            put(&mut walls, axis, held);
            let more = em.sub(of(walls, axis), one)?;
            let more = em.shl(more, UNIT_BITS)?;
            let held = em.add(of(self.next, axis), more)?;
            put(&mut ends, axis, held);
        }
        let chosen = self.first(em, ends)?;
        self.spread(em, chosen, ends, walls)
    }

    /// The ray one wall on where its brick is `full`, else past its brick.
    pub fn advanced(&self, em: &mut Em<'_>, full: Id) -> Result<Self, LaneError> {
        let moved = em.branch(
            full,
            |em| Ok(self.stepped(em)?.carried().to_vec()),
            |em| Ok(self.jumped(em)?.carried().to_vec()),
        )?;
        Ok(self.with(&moved))
    }

    /// The cell index and brick index of the ray's cell in `grid`.
    pub fn places(&self, em: &mut Em<'_>, grid: Grid) -> Result<(Id, Id), LaneError> {
        let wide = grid.wide.trailing_zeros();
        let deep = grid.deep.trailing_zeros();
        let z = em.shl(self.cell[2], wide + deep)?;
        let y = em.shl(self.cell[1], wide)?;
        let cell = em.or(z, y)?;
        let cell = em.or(cell, self.cell[0])?;
        let mut parts = [self.cell[0]; 3];
        for axis in 0..3 {
            let held = em.shr(of(self.cell, axis), BRICK_BITS)?;
            put(&mut parts, axis, held);
        }
        let wide = wide - BRICK_BITS;
        let deep = deep - BRICK_BITS;
        let z = em.shl(parts[2], wide + deep)?;
        let y = em.shl(parts[1], wide)?;
        let brick = em.or(z, y)?;
        let brick = em.or(brick, parts[0])?;
        Ok((cell, brick))
    }

    /// The ray's state as words, for a loop to carry.
    pub fn carried(&self) -> [Id; 7] {
        [
            self.cell[0],
            self.cell[1],
            self.cell[2],
            self.next[0],
            self.next[1],
            self.next[2],
            self.face,
        ]
    }

    /// The ray with its state taken from words a loop carried.
    #[must_use]
    pub fn with(&self, words: &[Id]) -> Self {
        let get = |at: usize, or: Id| words.get(at).copied().unwrap_or(or);
        Self {
            cell: [
                get(0, self.cell[0]),
                get(1, self.cell[1]),
                get(2, self.cell[2]),
            ],
            next: [
                get(3, self.next[0]),
                get(4, self.next[1]),
                get(5, self.next[2]),
            ],
            steep: self.steep,
            ahead: self.ahead,
            face: get(6, self.face),
        }
    }
}

/// Where a march reads: the grid, how its words map to kinds, and the word
/// of `PIXELS` its brick occupancy starts at.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Reader {
    pub grid: Grid,
    pub kinds: Kinds,
    pub bricks_at: u32,
}

impl Reader {
    /// The cell index at the ray's cell, the kind of its word, and whether
    /// its brick holds a cell anyone sees.
    pub fn read(&self, em: &mut Em<'_>, ray: &Marching) -> Result<(Id, Id, Id), LaneError> {
        let (cell, kind, full, _) = self.read_brick(em, ray)?;
        Ok((cell, kind, full))
    }

    /// [`Reader::read`], and the occupancy word of the ray's brick.
    pub fn read_brick(
        &self,
        em: &mut Em<'_>,
        ray: &Marching,
    ) -> Result<(Id, Id, Id, Id), LaneError> {
        let (cell, brick) = ray.places(em, self.grid)?;
        let base = em.u(self.bricks_at)?;
        let slot = em.add(base, brick)?;
        let held = em.load(PIXELS, slot)?;
        let full = em.ne_u(held, 0)?;
        let none = em.u(0)?;
        let word = em.when(full, &[none], |em| Ok(vec![em.load(STATE, cell)?]))?;
        let word = word.first().copied().unwrap_or(none);
        let kind = em.shr(word, self.kinds.shift())?;
        let kind = em.and_u(kind, self.kinds.mask())?;
        Ok((cell, kind, full, held))
    }

    /// Whether a ray from `origin` along `direction` meets a cell of a kind
    /// in `opaque` before it leaves the grid, as [`World::blocked`] finds it.
    /// No ray is traced where `going` is false.
    pub fn blocked(
        &self,
        em: &mut Em<'_>,
        opaque: Id,
        (origin, direction): ([Id; 3], [Id; 3]),
        going: Id,
    ) -> Result<Id, LaneError> {
        let grid = self.grid;
        let start = Marching::from(em, origin, direction)?;
        let (start, alive) = start.entered(em, sides(grid))?;
        let alive = em.both(alive, going)?;
        let alive = em.flag(alive)?;
        let none = em.u(0)?;
        let mut initial = vec![alive, none];
        initial.extend(start.carried());
        let out = em.looped(limit(grid), &initial, |em, words, _| {
            let ray = start.with(words.get(2..).unwrap_or(&[]));
            let blocked = words.get(1).copied().unwrap_or(none);
            let (_, kind, full) = self.read(em, &ray)?;
            let solid = em.bit(opaque, kind)?;
            let stops = em.both(full, solid)?;
            let moved = ray.advanced(em, full)?;
            let inside = moved.inside(em, sides(grid))?;
            let go = em.not(stops)?;
            let go = em.both(go, inside)?;
            let alive = em.flag(go)?;
            let hit = em.flag(stops)?;
            let blocked = em.or(blocked, hit)?;
            let mut next = vec![alive, blocked];
            next.extend(moved.carried());
            Ok(next)
        })?;
        let blocked = out.get(1).copied().unwrap_or(none);
        em.ne_u(blocked, 0)
    }
}
