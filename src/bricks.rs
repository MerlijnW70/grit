use std::convert::Infallible;

use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, Lanes, U32, Vector};
use simdr::module::Id;

use crate::compact::{Over, Plan, Tier};
use crate::geometry::{Arith, Emit, Plain, Solid, low_bits};
use crate::halo::{BOX, point};
use crate::layout::{DENSE, Dimension, GRAND, PARAMS, STATE, Windows};
use crate::live::{self, Loop, Stages};
use crate::rule3::{CORNERS, Rule};
use crate::sand::Grid;
use crate::{Error, Groups, Resident, Run};

pub const UNIFORM: u32 = u32::MAX;

pub const ENTRY: usize = 4;

pub const HELD: usize = 0;
pub const VALUE: usize = 1;
pub const TOUCHED: usize = 2;
pub const WANTED: usize = 3;

pub const TOP: usize = 0;
pub const OVERFLOW: usize = 1;
pub const LOST: usize = 2;
pub const SCRATCH: usize = 3;
pub const HEAD: usize = 4;

pub const TIER_BINDINGS: u32 = 8;

pub const LIST_BINDINGS: u32 = 2;

pub const TILE_BINDINGS: u32 = 3;

pub const TILE_WORDS: usize = 7;

pub const TILE_GROUPS: usize = 1;

pub const LANE_GROUPS: usize = 4;

pub const MOST_GROUPS: u32 = 65_535;

pub const SPARSE_BINDINGS: u32 = 2 + TIER_BINDINGS + LIST_BINDINGS + TILE_BINDINGS;

pub const AWAKE_LEVEL: u32 = 2;

pub const FLAT: &str = "a brick pool holds a solid grid laid out for three axes";
pub const SMALL: &str = "a side of the grid is shorter than a brick's side";
pub const UNPLANNED: &str = "the plan's blocks are not the grid's bricks";
pub const NO_SLOTS: &str = "a brick pool needs at least one slot";
pub const TOO_MANY_SLOTS: &str =
    "the pool's words pass the u32 index a kernel addresses STATE with";
pub const PLANES: &str = "a brick pool holds from 1 to rule3::MOST_PLANES planes";
pub const UNLIKE_PLANES: &str = "the rule's planes are not the brick pool's planes";

const OPERATION: &str = "bricks::Bricks::new";

const fn refused(because: &'static str) -> LaneError {
    LaneError::NoSuchForm {
        operation: OPERATION,
        because,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bricks {
    grid: Grid,
    subgroup: u32,
    bits: [u32; 3],
    lattice: [u32; 3],
    capacity: u32,
    windows: Windows,
    most: u32,
    cold: u32,
    planes: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    Dry,
    Settle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Listing {
    Listed,
    Every,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolUse {
    pub capacity: u32,
    pub held: u32,
    pub free: u32,
    pub overflow: bool,
    pub lost: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packed {
    pub table: Vec<u32>,
    pub pool: Vec<u32>,
    pub slots: Vec<u32>,
    pub held: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    Uniform(u32),
    Mixed(Vec<u32>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell<W> {
    pub brick: W,
    pub offset: W,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placed<W> {
    pub block: [W; 3],
    pub cells: [Cell<W>; CORNERS],
}

fn plainly<T>(done: Result<T, Infallible>) -> T {
    match done {
        Ok(value) => value,
        Err(never) => match never {},
    }
}

impl Bricks {
    pub fn new(grid: Grid, plan: &Plan, capacity: u32) -> Result<Self, LaneError> {
        let words = Self::pool_words_of(plan, capacity)?;
        Self::laid(grid, plan, capacity, Windows::of(words))
    }

    #[cfg(feature = "faults")]
    pub fn split_at(grid: Grid, plan: &Plan, capacity: u32, bits: u32) -> Result<Self, LaneError> {
        let words = Self::pool_words_of(plan, capacity)?;
        Self::laid(grid, plan, capacity, Windows::split_at(words, bits))
    }

    fn pool_words_of(plan: &Plan, capacity: u32) -> Result<usize, LaneError> {
        if capacity == 0 {
            return Err(refused(NO_SLOTS));
        }
        let cells = u64::from(plan.per_block) * 8;
        let words = u64::from(capacity) * cells + 1;
        if words > Plan::MOST_WORDS {
            return Err(refused(TOO_MANY_SLOTS));
        }
        usize::try_from(words).map_err(|_| refused(TOO_MANY_SLOTS))
    }

    fn laid(grid: Grid, plan: &Plan, capacity: u32, windows: Windows) -> Result<Self, LaneError> {
        if plan.dimension != Dimension::Three || grid.tall < 2 {
            return Err(refused(FLAT));
        }
        let k = plan.per_block.trailing_zeros();
        let bits = [k.div_ceil(3), (k + 1) / 3, k / 3];
        let sides = [grid.wide, grid.deep, grid.tall];
        let mut lattice = [0; 3];
        for ((slot, side), bit) in lattice.iter_mut().zip(sides).zip(bits) {
            let brick = 2_u32 << bit;
            if side < brick {
                return Err(refused(SMALL));
            }
            *slot = side / brick;
        }
        let bricks = u64::from(lattice[0]) * u64::from(lattice[1]) * u64::from(lattice[2]);
        if bricks != u64::from(plan.blocks) {
            return Err(refused(UNPLANNED));
        }
        Ok(Self {
            grid,
            subgroup: plan.subgroup,
            bits,
            lattice,
            capacity,
            windows,
            most: MOST_GROUPS,
            cold: plan.padded(),
            planes: 1,
        })
    }

    pub fn with_planes(self, planes: u32) -> Result<Self, LaneError> {
        if planes == 0 || planes as usize > crate::rule3::MOST_PLANES {
            return Err(refused(PLANES));
        }
        let words = u64::from(planes) * u64::from(self.capacity) * self.cells() as u64 + 1;
        if words > Plan::MOST_WORDS {
            return Err(refused(TOO_MANY_SLOTS));
        }
        let words = usize::try_from(words).map_err(|_| refused(TOO_MANY_SLOTS))?;
        Ok(Self {
            windows: Windows::at(words, self.windows.bits()),
            planes,
            ..self
        })
    }

    #[must_use]
    pub const fn planes(&self) -> usize {
        self.planes as usize
    }

    #[must_use]
    pub const fn plane_words(&self) -> usize {
        self.capacity as usize * self.cells()
    }

    #[must_use]
    pub const fn plane_start(&self, plane: usize) -> u32 {
        (plane * self.plane_words()) as u32
    }

    #[must_use]
    pub const fn plane_values(&self, plane: usize) -> u32 {
        let count = self.count() as usize;
        (count * ENTRY + ENTRY + (plane - 1) * (count + 1)) as u32
    }

    #[must_use]
    pub const fn value_at(&self, plane: usize, brick: u32) -> usize {
        if plane == 0 {
            brick as usize * ENTRY + VALUE
        } else {
            self.plane_values(plane) as usize + brick as usize
        }
    }

    #[cfg(feature = "faults")]
    #[must_use]
    pub const fn taking_at_most(mut self, groups: u32) -> Self {
        self.most = groups;
        self
    }

    #[must_use]
    pub const fn grid(&self) -> Grid {
        self.grid
    }

    #[must_use]
    pub const fn subgroup(&self) -> u32 {
        self.subgroup
    }

    #[must_use]
    pub const fn bits(&self) -> [u32; 3] {
        self.bits
    }

    #[must_use]
    pub const fn lattice(&self) -> [u32; 3] {
        self.lattice
    }

    #[must_use]
    pub const fn capacity(&self) -> u32 {
        self.capacity
    }

    #[must_use]
    pub const fn windows(&self) -> Windows {
        self.windows
    }

    #[must_use]
    pub const fn octs_bits(&self) -> u32 {
        self.bits[0] + self.bits[1] + self.bits[2]
    }

    #[must_use]
    pub const fn cell_bits(&self) -> u32 {
        self.octs_bits() + 3
    }

    #[must_use]
    pub const fn cells(&self) -> usize {
        1 << self.cell_bits()
    }

    #[must_use]
    pub const fn count(&self) -> u32 {
        self.lattice[0] * self.lattice[1] * self.lattice[2]
    }

    #[must_use]
    pub const fn sides(&self) -> [u32; 3] {
        [2 << self.bits[0], 2 << self.bits[1], 2 << self.bits[2]]
    }

    const fn inner(&self) -> [u32; 3] {
        [1 << self.bits[0], 1 << self.bits[1], 1 << self.bits[2]]
    }

    #[must_use]
    pub const fn takes(&self) -> u32 {
        self.count().div_ceil(self.most).next_power_of_two()
    }

    #[must_use]
    pub const fn pool_words(&self) -> usize {
        self.planes as usize * self.capacity as usize * self.cells() + 1
    }

    #[must_use]
    pub const fn scratch(&self) -> usize {
        self.planes as usize * self.capacity as usize * self.cells()
    }

    #[must_use]
    pub const fn table(&self) -> u32 {
        self.windows.bindings()
    }

    #[must_use]
    pub const fn slots(&self) -> u32 {
        self.windows.bindings() + 1
    }

    #[must_use]
    pub const fn bindings(&self) -> u32 {
        self.windows.bindings() + SPARSE_BINDINGS
    }

    #[must_use]
    pub const fn whole_list(&self) -> u32 {
        self.windows.bindings() + 2 + TIER_BINDINGS
    }

    #[must_use]
    pub const fn whole_count(&self) -> u32 {
        self.whole_list() + 1
    }

    #[must_use]
    pub const fn tiles(&self) -> u32 {
        self.whole_count() + 1
    }

    #[must_use]
    pub const fn tile_list(&self) -> u32 {
        self.whole_count() + 2
    }

    #[must_use]
    pub const fn tile_args(&self) -> u32 {
        self.whole_count() + 3
    }

    #[must_use]
    pub const fn tile_count(&self) -> u32 {
        self.count().div_ceil(self.per_block())
    }

    #[must_use]
    pub const fn tile_lattice(&self) -> [u32; 3] {
        let per = self.per_block();
        let [wide, deep, tall] = self.lattice;
        if self.count() <= per {
            [1, 1, 1]
        } else if per <= wide {
            [wide / per, deep, tall]
        } else if per <= wide * deep {
            [1, deep / (per / wide), tall]
        } else {
            [1, 1, tall / (per / (wide * deep))]
        }
    }

    #[must_use]
    pub const fn tile_groups(&self) -> Groups {
        Groups::From {
            binding: self.tile_args() as usize,
            at: TILE_GROUPS,
        }
    }

    #[must_use]
    pub const fn lane_groups(&self) -> Groups {
        Groups::From {
            binding: self.tile_args() as usize,
            at: LANE_GROUPS,
        }
    }

    #[must_use]
    pub const fn per_block(&self) -> u32 {
        1 << self.octs_bits()
    }

    #[must_use]
    pub const fn pool_octs(&self) -> u32 {
        self.capacity * self.per_block()
    }

    #[must_use]
    pub const fn dump(&self) -> u32 {
        self.pool_octs()
    }

    #[must_use]
    pub const fn whole_tier(&self) -> Tier {
        let tier = self.tier();
        Tier {
            dense: self.whole_list(),
            grand: self.whole_count(),
            ..tier
        }
    }

    #[must_use]
    pub const fn tier(&self) -> Tier {
        let first = self.windows.bindings() + 2;
        Tier {
            flags: first,
            buckets: first + 1,
            counts: first + 2,
            bases: first + 3,
            totals: first + 4,
            dense: first + 5,
            grand: first + 6,
            indirect: first + 7,
        }
    }

    #[must_use]
    pub const fn table_words(&self) -> usize {
        let count = self.count() as usize;
        count * ENTRY + ENTRY + (self.planes as usize - 1) * (count + 1)
    }

    #[must_use]
    pub fn sizes<R: Rule>(&self, rule: &R, plan: &Plan) -> Vec<usize> {
        use crate::layout::{AWAKE, BUCKETS};

        let mut sizes = live::sizes3(rule, plan);
        let capacity = self.capacity as usize;
        let pool_octs = self.pool_octs() as usize;
        let subgroup = plan.subgroup as usize;
        for (binding, words) in [
            (STATE, self.pool_words()),
            (AWAKE, pool_octs),
            (
                BUCKETS,
                capacity * crate::sweep::stride(plan.subgroup) as usize,
            ),
            (DENSE, pool_octs + capacity * subgroup + 1),
        ] {
            if let Some(size) = sizes.get_mut(binding as usize) {
                *size = words;
            }
        }
        let mut bound = self.windows.laid(&sizes);
        bound.resize(self.windows.bindings() as usize, 1);
        let padded = plan.padded() as usize;
        let per_block = plan.per_block as usize;
        let tiles = plan.tiles as usize;
        let hot_padded = plan.hot_tiles as usize * per_block;
        bound.extend([
            self.table_words(),
            HEAD + self.capacity as usize,
            padded + 1,
            tiles * crate::sweep::stride(plan.subgroup) as usize,
            hot_padded,
            hot_padded,
            per_block,
            padded + tiles * subgroup,
            1,
            3,
            padded + tiles * subgroup,
            1,
            self.tile_count() as usize + 1,
            self.tile_count() as usize + 1,
            TILE_WORDS,
        ]);
        bound
    }

    pub fn oct_point<A: Arith>(
        &self,
        arith: &mut A,
        index: A::Word,
    ) -> Result<[A::Word; 3], A::Error> {
        let k = self.octs_bits();
        let by = arith.splat(k)?;
        let brick = arith.shift_right(index, by)?;
        let local = low_bits(arith, index, k)?;
        let inside = Solid::split(arith, self.inner(), local)?;
        let at = Solid::split(arith, self.lattice, brick)?;
        let mut point = inside;
        for ((value, high), bits) in point.iter_mut().zip(at).zip(self.bits) {
            let by = arith.splat(bits)?;
            let lifted = arith.shift_left(high, by)?;
            *value = arith.add(lifted, *value)?;
        }
        Ok(point)
    }

    pub fn oct_number<A: Arith>(
        &self,
        arith: &mut A,
        point: [A::Word; 3],
    ) -> Result<A::Word, A::Error> {
        let mut high = point;
        let mut low = point;
        for ((upper, lower), bits) in high.iter_mut().zip(low.iter_mut()).zip(self.bits) {
            let by = arith.splat(bits)?;
            *upper = arith.shift_right(*upper, by)?;
            *lower = low_bits(arith, *lower, bits)?;
        }
        let brick = Solid::at(arith, self.lattice, high)?;
        let local = Solid::at(arith, self.inner(), low)?;
        let by = arith.splat(self.octs_bits())?;
        let lifted = arith.shift_left(brick, by)?;
        arith.add(lifted, local)
    }

    pub fn neighbour<A: Arith>(
        &self,
        arith: &mut A,
        point: [A::Word; 3],
        offset: [u32; 3],
    ) -> Result<A::Word, A::Error> {
        let one = arith.splat(1)?;
        let minus = arith.splat(u32::MAX)?;
        let back = arith.mul(one, minus)?;
        let mut near = point;
        for ((value, step), extent) in near.iter_mut().zip(offset).zip(Solid::blocks(self.grid)) {
            let step = arith.splat(step)?;
            *value = arith.add(*value, step)?;
            *value = arith.add(*value, back)?;
            *value = low_bits(arith, *value, extent.trailing_zeros())?;
        }
        self.oct_number(arith, near)
    }

    pub fn place<A: Arith>(
        &self,
        arith: &mut A,
        point: [A::Word; 3],
    ) -> Result<Cell<A::Word>, A::Error> {
        let mut high = point;
        let mut low = point;
        for (((upper, lower), bits), extent) in high
            .iter_mut()
            .zip(low.iter_mut())
            .zip(self.bits)
            .zip(Solid::extents(self.grid))
        {
            let wrapped = low_bits(arith, *upper, extent.trailing_zeros())?;
            let by = arith.splat(bits + 1)?;
            *upper = arith.shift_right(wrapped, by)?;
            *lower = low_bits(arith, wrapped, bits + 1)?;
        }
        let brick = Solid::at(arith, self.lattice, high)?;
        let offset = Solid::at(arith, self.sides(), low)?;
        Ok(Cell { brick, offset })
    }

    pub fn placed<A: Arith>(
        &self,
        arith: &mut A,
        index: A::Word,
        frame: A::Word,
    ) -> Result<Placed<A::Word>, A::Error> {
        let phase = low_bits(arith, frame, 1)?;
        let block = self.oct_point(arith, index)?;
        let corners = Solid::corners(arith, block, phase)?;
        let mut out = [Cell {
            brick: index,
            offset: index,
        }; CORNERS];
        for (slot, from) in out.iter_mut().zip(crate::sand3::REFERENCE) {
            if let Some(&corner) = corners.get(from) {
                *slot = self.place(arith, corner)?;
            }
        }
        Ok(Placed { block, cells: out })
    }

    #[must_use]
    pub fn oct_at(&self, index: u32) -> [u32; 3] {
        plainly(self.oct_point(&mut Plain, index))
    }

    #[must_use]
    pub fn oct_of(&self, point: [u32; 3]) -> u32 {
        plainly(self.oct_number(&mut Plain, point))
    }

    #[must_use]
    pub fn cell_of(&self, point: [u32; 3]) -> Cell<u32> {
        plainly(self.place(&mut Plain, point))
    }

    #[must_use]
    pub fn cells_of(&self, index: u32, frame: u32) -> Placed<u32> {
        plainly(self.placed(&mut Plain, index, frame))
    }

    #[must_use]
    pub fn point_of(&self, brick: u32, offset: u32) -> [u32; 3] {
        let at = plainly(Solid::split(&mut Plain, self.lattice, brick));
        let within = plainly(Solid::split(&mut Plain, self.sides(), offset));
        let sides = self.sides();
        [
            at[0] * sides[0] + within[0],
            at[1] * sides[1] + within[1],
            at[2] * sides[2] + within[2],
        ]
    }

    #[must_use]
    pub fn woken(&self, index: u32) -> [u32; 27] {
        let point = self.oct_at(index);
        Solid::near().map(|offset| plainly(self.neighbour(&mut Plain, point, offset)))
    }

    pub fn pack(&self, cells: &[u32]) -> Result<Packed, Error> {
        let grid = self.grid;
        let planes = self.planes();
        let stride = grid.cells();
        if cells.len() != stride * planes {
            return Err(Error::Overrun {
                index: STATE as usize,
                needed: stride * planes,
                held: cells.len(),
            });
        }
        let per = self.cells();
        self.compose_planes(|brick| {
            (0..planes)
                .map(|plane| {
                    let mut words = vec![0_u32; per];
                    for (offset, word) in (0_u32..).zip(words.iter_mut()) {
                        let point = self.point_of(brick, offset);
                        let at = plane * stride + Solid::cell(grid, point) as usize;
                        *word = cells.get(at).copied().unwrap_or_default();
                    }
                    Content::Mixed(words)
                })
                .collect()
        })
    }

    pub fn compose<F>(&self, mut content: F) -> Result<Packed, Error>
    where
        F: FnMut(u32) -> Content,
    {
        self.compose_planes(|brick| vec![content(brick)])
    }

    pub fn compose_planes<F>(&self, mut content: F) -> Result<Packed, Error>
    where
        F: FnMut(u32) -> Vec<Content>,
    {
        let per = self.cells();
        let planes = self.planes();
        let mut table = vec![0_u32; self.table_words()];
        let mut pools: Vec<Vec<u32>> = vec![Vec::new(); planes];
        let mut held = 0_u32;
        for brick in 0..self.count() {
            let entry = brick as usize * ENTRY;
            let mut given = content(brick).into_iter();
            let mut words: Vec<Content> = Vec::with_capacity(planes);
            for _ in 0..planes {
                words.push(given.next().unwrap_or(Content::Uniform(0)));
            }
            let mut flat: Vec<Option<u32>> = Vec::with_capacity(planes);
            for one in &words {
                flat.push(match one {
                    Content::Uniform(value) => Some(*value),
                    Content::Mixed(words) => {
                        if words.len() != per {
                            return Err(Error::Overrun {
                                index: STATE as usize,
                                needed: per,
                                held: words.len(),
                            });
                        }
                        let first = words.first().copied().unwrap_or_default();
                        words.iter().all(|&word| word == first).then_some(first)
                    }
                });
            }
            if flat.iter().all(Option::is_some) {
                for (plane, value) in flat.iter().enumerate() {
                    let value = value.unwrap_or_default();
                    if plane == 0 {
                        if let Some(slot) = table.get_mut(entry..entry + 2) {
                            slot.copy_from_slice(&[UNIFORM, value]);
                        }
                    } else if let Some(slot) = table.get_mut(self.value_at(plane, brick)) {
                        *slot = value;
                    }
                }
                continue;
            }
            if held == self.capacity {
                return Err(Error::PoolFull {
                    slots: self.capacity,
                });
            }
            if let Some(slot) = table.get_mut(entry) {
                *slot = held;
            }
            for (pool, one) in pools.iter_mut().zip(words) {
                match one {
                    Content::Uniform(value) => pool.extend(std::iter::repeat_n(value, per)),
                    Content::Mixed(words) => pool.extend_from_slice(&words),
                }
            }
            held += 1;
        }
        let pool = pools.concat();
        let top = self.capacity - held;
        let mut slots = vec![top, 0, 0, 0];
        slots.extend((0..top).map(|place| self.capacity - 1 - place));
        Ok(Packed {
            table,
            pool,
            slots,
            held,
        })
    }

    pub fn restacked(&self, packed: Packed) -> Result<Packed, Error> {
        if packed.held > self.capacity {
            return Err(Error::PoolFull {
                slots: self.capacity,
            });
        }
        let top = self.capacity - packed.held;
        let mut slots = vec![top, 0, 0, 0];
        slots.extend((0..top).map(|place| self.capacity - 1 - place));
        Ok(Packed { slots, ..packed })
    }

    #[must_use]
    pub fn brick_plane(&self, table: &[u32], pool: &[u32], brick: u32, plane: usize) -> Content {
        let entry = brick as usize * ENTRY;
        let slot = table.get(entry).copied().unwrap_or(UNIFORM);
        let value = table
            .get(self.value_at(plane, brick))
            .copied()
            .unwrap_or_default();
        if slot == UNIFORM {
            return Content::Uniform(value);
        }
        let per = self.cells();
        let from = plane * self.plane_words() + slot as usize * per;
        Content::Mixed(pool.get(from..from + per).unwrap_or_default().to_vec())
    }

    pub fn raw(&self, held: &mut Resident<'_>) -> Result<(Vec<u32>, Vec<u32>), Error> {
        let table = held.read(self.table() as usize, self.table_words())?;
        let pool = live::read_state(held, self.windows, self.scratch())?;
        Ok((table, pool))
    }

    #[must_use]
    pub fn brick_words(&self, table: &[u32], pool: &[u32], brick: u32) -> Content {
        let entry = brick as usize * ENTRY;
        let slot = table.get(entry).copied().unwrap_or(UNIFORM);
        let value = table.get(entry + VALUE).copied().unwrap_or_default();
        if slot == UNIFORM {
            return Content::Uniform(value);
        }
        let per = self.cells();
        let from = slot as usize * per;
        Content::Mixed(pool.get(from..from + per).unwrap_or_default().to_vec())
    }

    pub fn write(
        &self,
        held: &mut Resident<'_>,
        plan: &Plan,
        cells: &[u32],
    ) -> Result<PoolUse, Error> {
        let packed = self.pack(cells)?;
        self.write_packed(held, plan, &packed)
    }

    pub fn write_packed(
        &self,
        held: &mut Resident<'_>,
        plan: &Plan,
        packed: &Packed,
    ) -> Result<PoolUse, Error> {
        held.write(self.table() as usize, &packed.table)?;
        held.write(self.slots() as usize, &packed.slots)?;
        let span = packed.held as usize * self.cells();
        if self.planes() == 1 || span == 0 {
            live::write_state(held, self.windows, 0, &packed.pool)?;
        } else {
            for (plane, words) in packed.pool.chunks(span).take(self.planes()).enumerate() {
                live::write_state(held, self.windows, plane * self.plane_words(), words)?;
            }
        }
        let tier = self.tier();
        let hot_padded = plan.hot_tiles as usize * plan.per_block as usize;
        for (binding, words) in [
            (tier.counts, hot_padded),
            (tier.bases, hot_padded),
            (tier.totals, plan.per_block as usize),
            (tier.grand, 1),
        ] {
            held.write(binding as usize, &vec![0; words])?;
        }
        Ok(PoolUse {
            capacity: self.capacity,
            held: packed.held,
            free: self.capacity - packed.held,
            overflow: false,
            lost: false,
        })
    }

    pub fn pool_use(&self, held: &mut Resident<'_>) -> Result<PoolUse, Error> {
        let header = held.read(self.slots() as usize, HEAD)?;
        let word = |at: usize| header.get(at).copied().unwrap_or_default();
        let free = word(TOP).min(self.capacity);
        Ok(PoolUse {
            capacity: self.capacity,
            held: self.capacity - free,
            free,
            overflow: word(OVERFLOW) != 0,
            lost: word(LOST) != 0,
        })
    }

    pub fn check(&self, held: &mut Resident<'_>) -> Result<PoolUse, Error> {
        let used = self.pool_use(held)?;
        if used.overflow {
            return Err(Error::PoolFull {
                slots: self.capacity,
            });
        }
        if used.lost {
            return Err(Error::LostWrite);
        }
        Ok(used)
    }

    pub fn read(&self, held: &mut Resident<'_>) -> Result<Vec<u32>, Error> {
        self.check(held)?;
        let table = held.read(self.table() as usize, self.table_words())?;
        let pool = live::read_state(held, self.windows, self.scratch())?;
        Ok(self.unpack(&table, &pool))
    }

    #[must_use]
    pub fn unpack(&self, table: &[u32], pool: &[u32]) -> Vec<u32> {
        let grid = self.grid;
        let per = self.cells();
        let stride = grid.cells();
        let mut cells = vec![0_u32; stride * self.planes()];
        for plane in 0..self.planes() {
            let from = plane * self.plane_words();
            for brick in 0..self.count() {
                let entry = brick as usize * ENTRY;
                let slot = table.get(entry).copied().unwrap_or(UNIFORM);
                let value = table
                    .get(self.value_at(plane, brick))
                    .copied()
                    .unwrap_or_default();
                for offset in 0..per {
                    let word = if slot == UNIFORM {
                        value
                    } else {
                        pool.get(from + slot as usize * per + offset)
                            .copied()
                            .unwrap_or_default()
                    };
                    let point = self.point_of(brick, u32::try_from(offset).unwrap_or(u32::MAX));
                    let at = plane * stride + Solid::cell(grid, point) as usize;
                    if let Some(cell) = cells.get_mut(at) {
                        *cell = word;
                    }
                }
            }
        }
        cells
    }

    pub fn put(&self, held: &mut Resident<'_>, cells: &[(u32, u32)]) -> Result<(), Error> {
        let grid = self.grid;
        let past = grid.cells();
        if let Some(&(cell, _)) = cells.iter().find(|(cell, _)| *cell as usize >= past) {
            return Err(Error::Overrun {
                index: STATE as usize,
                needed: cell as usize + 1,
                held: past,
            });
        }
        let points: Vec<([u32; 3], u32)> = cells
            .iter()
            .map(|&(cell, word)| (Solid::point(grid, cell), word))
            .collect();
        self.put_at(held, &points)
    }

    pub fn put_at(&self, held: &mut Resident<'_>, cells: &[([u32; 3], u32)]) -> Result<(), Error> {
        self.put_plane_at(held, 0, cells)
    }

    pub fn put_plane_at(
        &self,
        held: &mut Resident<'_>,
        plane: usize,
        cells: &[([u32; 3], u32)],
    ) -> Result<(), Error> {
        self.put_plane_in(held, plane, cells, false)
    }

    pub fn put_plane_in(
        &self,
        held: &mut Resident<'_>,
        plane: usize,
        cells: &[([u32; 3], u32)],
        around: bool,
    ) -> Result<(), Error> {
        use std::collections::{BTreeMap, BTreeSet};

        if plane >= self.planes() {
            return Err(Error::Overrun {
                index: STATE as usize,
                needed: (plane + 1) * self.plane_words(),
                held: self.scratch(),
            });
        }

        use crate::layout::{AWAKE, HOT};

        let grid = self.grid;
        let sides = [grid.wide, grid.deep, grid.tall];
        if let Some(&(point, _)) = cells
            .iter()
            .find(|(point, _)| point.iter().zip(sides).any(|(&at, side)| at >= side))
        {
            let needed = point
                .iter()
                .zip(sides)
                .map(|(&at, side)| u64::from(at.max(side)))
                .product::<u64>();
            return Err(Error::Overrun {
                index: STATE as usize,
                needed: usize::try_from(needed).unwrap_or(usize::MAX),
                held: grid.cells(),
            });
        }
        self.check(held)?;
        let per = self.cells();
        let per_block = self.per_block() as usize;
        let planes = self.planes();
        let by = self.octs_bits();
        let mut touched = BTreeSet::new();
        for &(point, _) in cells {
            touched.insert(self.cell_of(point).brick);
            for phase in 0..2 {
                let oct = self.oct_of(self.holder(point, phase));
                touched.insert(oct >> by);
                if around {
                    touched.extend(self.woken(oct).map(|near| near >> by));
                }
            }
        }
        let touched: Vec<u32> = touched.into_iter().collect();
        let table_binding = self.table() as usize;
        let mut asked = vec![
            (self.slots() as usize, 0, HEAD),
            (GRAND as usize, 0, 1),
            (self.whole_count() as usize, 0, 1),
        ];
        for (from, count) in live::spans(&touched) {
            asked.push((table_binding, from * ENTRY, count * ENTRY));
        }
        for filled in 1..planes {
            for (from, count) in live::spans(&touched) {
                asked.push((table_binding, self.value_at(filled, from as u32), count));
            }
        }
        asked.push((self.tile_args() as usize, 0, 1));
        let mut table: BTreeMap<usize, u32> = BTreeMap::new();
        let mut answers = held.read_runs(&asked)?;
        let lit_count = first_word(answers.pop());
        let mut answers = answers.into_iter();
        let mut header = answers.next().unwrap_or_default();
        let octs = first_word(answers.next());
        let bricks = first_word(answers.next());
        for (from, _) in live::spans(&touched) {
            for (at, word) in (from * ENTRY..).zip(answers.next().unwrap_or_default()) {
                table.insert(at, word);
            }
        }
        for filled in 1..planes {
            for (from, _) in live::spans(&touched) {
                let start = self.value_at(filled, from as u32);
                for (at, word) in (start..).zip(answers.next().unwrap_or_default()) {
                    table.insert(at, word);
                }
            }
        }
        let word_at = |table: &BTreeMap<usize, u32>, at: usize, or: u32| {
            table.get(&at).copied().unwrap_or(or)
        };
        let top = header.get(TOP).copied().unwrap_or_default() as usize;

        let mut wanted = BTreeSet::new();
        for &(point, word) in cells {
            let Cell { brick, .. } = self.cell_of(point);
            let slot = word_at(&table, brick as usize * ENTRY, UNIFORM);
            let value = word_at(&table, self.value_at(plane, brick), 0);
            if slot == UNIFORM && word != value {
                wanted.insert(brick);
            }
        }
        if wanted.len() > top {
            return Err(Error::PoolFull {
                slots: self.capacity,
            });
        }
        let wanted: Vec<u32> = wanted.into_iter().collect();
        let low = top - wanted.len();
        let mut asked = vec![(self.slots() as usize, HEAD + low, wanted.len())];
        asked.extend(live::spans(&wanted).map(|(from, count)| (HOT as usize, from, count)));
        asked.push((DENSE as usize, 0, octs as usize));
        asked.push((self.whole_list() as usize, 0, bricks as usize));
        asked.push((self.tile_list() as usize, 0, lit_count as usize));
        let mut got = held.read_runs(&asked)?;
        let old_tiles = got.pop().unwrap_or_default();
        let old_whole = got.pop().unwrap_or_default();
        let old_listed = got.pop().unwrap_or_default();
        let mut got = got.into_iter();
        let stack = got.next().unwrap_or_default();
        let levels: BTreeMap<u32, u32> = wanted.iter().copied().zip(got.flatten()).collect();
        let mut listed: BTreeSet<u32> = old_listed.iter().copied().collect();
        let mut whole: BTreeSet<u32> = old_whole.iter().copied().collect();

        let mut pool: BTreeMap<usize, Vec<u32>> = BTreeMap::new();
        let mut words: BTreeMap<usize, u32> = BTreeMap::new();
        let mut marks: BTreeMap<usize, Vec<u32>> = BTreeMap::new();
        let mut awake: BTreeMap<usize, u32> = BTreeMap::new();
        let mut entries: BTreeMap<usize, u32> = BTreeMap::new();
        let mut top = top;
        let mut woken = BTreeSet::new();
        for &(point, word) in cells {
            let Cell { brick, offset } = self.cell_of(point);
            let entry = brick as usize * ENTRY;
            let slot = word_at(&table, entry, UNIFORM);
            let value = word_at(&table, self.value_at(plane, brick), 0);
            let slot = if slot != UNIFORM {
                Some(slot)
            } else if word == value {
                None
            } else {
                let Some(&taken) = top
                    .checked_sub(1)
                    .and_then(|at| at.checked_sub(low))
                    .and_then(|at| stack.get(at))
                else {
                    return Err(Error::PoolFull {
                        slots: self.capacity,
                    });
                };
                top -= 1;
                for filled in 0..planes {
                    let kept = word_at(&table, self.value_at(filled, brick), 0);
                    pool.insert(
                        filled * self.plane_words() + taken as usize * per,
                        vec![kept; per],
                    );
                }
                let level = levels.get(&brick).copied().unwrap_or_default();
                marks.insert(taken as usize * per_block, vec![level; per_block]);
                table.insert(entry, taken);
                entries.insert(entry, taken);
                Some(taken)
            };
            if let Some(slot) = slot {
                let at = plane * self.plane_words() + slot as usize * per + offset as usize;
                placed(&mut pool, &mut words, at, word);
            }
            for phase in 0..2 {
                let oct = self.oct_of(self.holder(point, phase));
                if around {
                    woken.extend(self.woken(oct));
                } else {
                    woken.insert(oct);
                }
            }
        }
        if let Some(word) = header.get_mut(TOP) {
            *word = u32::try_from(top).unwrap_or(u32::MAX);
        }

        let mut hot = BTreeSet::new();
        for &oct in &woken {
            let brick = oct >> by;
            hot.insert(brick);
            let slot = word_at(&table, brick as usize * ENTRY, UNIFORM);
            if slot == UNIFORM {
                whole.insert(brick);
                continue;
            }
            let local = (oct & (self.per_block() - 1)) as usize;
            placed(
                &mut marks,
                &mut awake,
                slot as usize * per_block + local,
                AWAKE_LEVEL,
            );
            if !whole.contains(&brick) {
                listed.insert(oct);
            }
        }
        listed.retain(|oct| !whole.contains(&(oct >> by)));
        held.woke(woken.len());

        let mut tiles: BTreeSet<u32> = old_tiles.iter().copied().collect();
        for &brick in &hot {
            tiles.extend(self.around_brick(brick).map(|near| near / self.per_block()));
        }
        let tiles: Vec<u32> = tiles.into_iter().collect();
        let tile_words = self.tile_words(&tiles);
        let listed: Vec<u32> = listed.into_iter().collect();
        let whole: Vec<u32> = whole.into_iter().collect();
        let hot: Vec<u32> = hot.into_iter().collect();
        let marked = vec![AWAKE_LEVEL; hot.len()];
        let mut runs = Vec::new();
        for (&at, block) in &pool {
            live::state_runs(self.windows, at, block, &mut runs);
        }
        let (word_spans, word_values) = runs_of(&words);
        for &(at, length, from) in &word_spans {
            let part = word_values.get(from..from + length).unwrap_or_default();
            live::state_runs(self.windows, at, part, &mut runs);
        }
        for (&at, block) in &marks {
            runs.push(Run {
                index: AWAKE as usize,
                at,
                words: block,
            });
        }
        let (awake_spans, awake_values) = runs_of(&awake);
        for &(at, length, from) in &awake_spans {
            runs.push(Run {
                index: AWAKE as usize,
                at,
                words: awake_values.get(from..from + length).unwrap_or_default(),
            });
        }
        for (from, length) in live::spans(&hot) {
            runs.push(Run {
                index: HOT as usize,
                at: from,
                words: marked.get(..length).unwrap_or_default(),
            });
        }
        let entry_words: Vec<(usize, [u32; 1])> =
            entries.iter().map(|(&at, &slot)| (at, [slot])).collect();
        for (at, slot) in &entry_words {
            runs.push(Run {
                index: table_binding,
                at: *at,
                words: slot,
            });
        }
        runs.push(Run {
            index: self.slots() as usize,
            at: 0,
            words: &header,
        });
        let listed_from = live::changed_from(&old_listed, &listed);
        let whole_from = live::changed_from(&old_whole, &whole);
        let counts = self.listing(&listed, &whole);
        runs.push(Run {
            index: DENSE as usize,
            at: listed_from,
            words: listed.get(listed_from..).unwrap_or_default(),
        });
        runs.push(Run {
            index: self.whole_list() as usize,
            at: whole_from,
            words: whole.get(whole_from..).unwrap_or_default(),
        });
        runs.push(Run {
            index: GRAND as usize,
            at: 0,
            words: counts.get(..1).unwrap_or_default(),
        });
        runs.push(Run {
            index: self.whole_count() as usize,
            at: 0,
            words: counts.get(1..2).unwrap_or_default(),
        });
        runs.push(Run {
            index: crate::layout::INDIRECT as usize,
            at: 0,
            words: counts.get(2..).unwrap_or_default(),
        });
        let tiles_from = live::changed_from(&old_tiles, &tiles);
        runs.push(Run {
            index: self.tile_list() as usize,
            at: tiles_from,
            words: tiles.get(tiles_from..).unwrap_or_default(),
        });
        runs.push(Run {
            index: self.tile_args() as usize,
            at: 0,
            words: &tile_words,
        });
        runs.retain(|run| !run.words.is_empty());
        held.write_many(&runs)
    }

    pub fn wake_bricks(&self, held: &mut Resident<'_>, changed: &[u32]) -> Result<(), Error> {
        use std::collections::{BTreeMap, BTreeSet};

        use crate::layout::{AWAKE, HOT};

        let mut woken = BTreeSet::new();
        for &brick in changed {
            woken.extend(self.around_brick(brick));
        }
        let woken: Vec<u32> = woken.into_iter().collect();
        if woken.is_empty() {
            return Ok(());
        }
        let table_binding = self.table() as usize;
        let mut asked = vec![
            (GRAND as usize, 0, 1),
            (self.whole_count() as usize, 0, 1),
            (self.tile_args() as usize, 0, 1),
        ];
        for (from, count) in live::spans(&woken) {
            asked.push((table_binding, from * ENTRY, count * ENTRY));
        }
        let mut answers = held.read_runs(&asked)?.into_iter();
        let octs = first_word(answers.next());
        let bricks = first_word(answers.next());
        let lit_count = first_word(answers.next());
        let mut slots: BTreeMap<u32, u32> = BTreeMap::new();
        for (from, _) in live::spans(&woken) {
            let entries = answers.next().unwrap_or_default();
            for (brick, entry) in (from..).zip(entries.chunks(ENTRY)) {
                let slot = entry.first().copied().unwrap_or(UNIFORM);
                slots.insert(u32::try_from(brick).unwrap_or(u32::MAX), slot);
            }
        }
        let mut got = held.read_runs(&[
            (DENSE as usize, 0, octs as usize),
            (self.whole_list() as usize, 0, bricks as usize),
            (self.tile_list() as usize, 0, lit_count as usize),
        ])?;
        let old_tiles = got.pop().unwrap_or_default();
        let old_whole = got.pop().unwrap_or_default();
        let old_listed = got.pop().unwrap_or_default();
        let by = self.octs_bits();
        let mut whole: BTreeSet<u32> = old_whole.iter().copied().collect();
        whole.extend(woken.iter().copied());
        let listed: Vec<u32> = old_listed
            .iter()
            .copied()
            .filter(|oct| !whole.contains(&(oct >> by)))
            .collect();
        let whole: Vec<u32> = whole.into_iter().collect();
        let mut tiles: BTreeSet<u32> = old_tiles.iter().copied().collect();
        for &brick in &woken {
            tiles.extend(self.around_brick(brick).map(|near| near / self.per_block()));
        }
        let tiles: Vec<u32> = tiles.into_iter().collect();
        let tile_words = self.tile_words(&tiles);
        let per_block = self.per_block() as usize;
        let mut taken: Vec<u32> = slots
            .values()
            .copied()
            .filter(|&slot| slot != UNIFORM)
            .collect();
        taken.sort_unstable();
        taken.dedup();
        let longest = live::spans(&taken)
            .map(|(_, length)| length)
            .max()
            .unwrap_or(0);
        let marks = vec![AWAKE_LEVEL; per_block * longest.max(woken.len())];
        let mut runs = Vec::new();
        for (first, length) in live::spans(&taken) {
            runs.push(Run {
                index: AWAKE as usize,
                at: first * per_block,
                words: marks.get(..length * per_block).unwrap_or_default(),
            });
        }
        for (from, length) in live::spans(&woken) {
            runs.push(Run {
                index: HOT as usize,
                at: from,
                words: marks.get(..length).unwrap_or_default(),
            });
        }
        let listed_from = live::changed_from(&old_listed, &listed);
        let whole_from = live::changed_from(&old_whole, &whole);
        let counts = self.listing(&listed, &whole);
        runs.push(Run {
            index: DENSE as usize,
            at: listed_from,
            words: listed.get(listed_from..).unwrap_or_default(),
        });
        runs.push(Run {
            index: self.whole_list() as usize,
            at: whole_from,
            words: whole.get(whole_from..).unwrap_or_default(),
        });
        runs.push(Run {
            index: GRAND as usize,
            at: 0,
            words: counts.get(..1).unwrap_or_default(),
        });
        runs.push(Run {
            index: self.whole_count() as usize,
            at: 0,
            words: counts.get(1..2).unwrap_or_default(),
        });
        runs.push(Run {
            index: crate::layout::INDIRECT as usize,
            at: 0,
            words: counts.get(2..).unwrap_or_default(),
        });
        let tiles_from = live::changed_from(&old_tiles, &tiles);
        runs.push(Run {
            index: self.tile_list() as usize,
            at: tiles_from,
            words: tiles.get(tiles_from..).unwrap_or_default(),
        });
        runs.push(Run {
            index: self.tile_args() as usize,
            at: 0,
            words: &tile_words,
        });
        runs.retain(|run| !run.words.is_empty());
        held.write_many(&runs)?;
        held.woke(woken.len() * per_block);
        Ok(())
    }

    #[must_use]
    pub fn around_brick(&self, brick: u32) -> [u32; 27] {
        let at = plainly(Solid::split(&mut Plain, self.lattice, brick));
        Solid::near().map(|offset| {
            let mut point = at;
            for ((value, step), side) in point.iter_mut().zip(offset).zip(self.lattice) {
                *value = (*value + side + step - 1) % side;
            }
            plainly(Solid::at(&mut Plain, self.lattice, point))
        })
    }

    #[must_use]
    pub fn tile_words(&self, tiles: &[u32]) -> [u32; TILE_WORDS] {
        let count = u32::try_from(tiles.len()).unwrap_or(u32::MAX);
        let lanes = count.saturating_mul(crate::sweep::PER_LANE);
        [count, count, 1, 1, lanes, 1, 1]
    }

    fn listing(&self, octs: &[u32], bricks: &[u32]) -> [u32; 5] {
        let octs_count = u32::try_from(octs.len()).unwrap_or(u32::MAX);
        let bricks_count = u32::try_from(bricks.len()).unwrap_or(u32::MAX);
        let groups =
            octs_count.div_ceil(self.subgroup) + bricks_count * (self.per_block() / self.subgroup);
        [octs_count, bricks_count, groups, 1, 1]
    }

    #[must_use]
    pub fn holder(&self, point: [u32; 3], phase: u32) -> [u32; 3] {
        let sides = [self.grid.wide, self.grid.deep, self.grid.tall];
        let mut block = [0; 3];
        for ((slot, at), side) in block.iter_mut().zip(point).zip(sides) {
            let shifted = (at + side - (phase & 1)) % side;
            *slot = shifted / 2;
        }
        block
    }

    pub fn listed(&self, held: &mut Resident<'_>) -> Result<(u32, u32), Error> {
        let octs = held.read(GRAND as usize, 1)?.first().copied().unwrap_or(0);
        let bricks = held
            .read(self.whole_count() as usize, 1)?
            .first()
            .copied()
            .unwrap_or(0);
        Ok((octs, bricks))
    }

    fn list(&self, held: &mut Resident<'_>, octs: &[u32], bricks: &[u32]) -> Result<(), Error> {
        use crate::layout::INDIRECT;

        let octs_count = u32::try_from(octs.len()).unwrap_or(u32::MAX);
        let bricks_count = u32::try_from(bricks.len()).unwrap_or(u32::MAX);
        if !octs.is_empty() {
            held.write(DENSE as usize, octs)?;
        }
        if !bricks.is_empty() {
            held.write(self.whole_list() as usize, bricks)?;
        }
        held.write(GRAND as usize, &[octs_count])?;
        held.write(self.whole_count() as usize, &[bricks_count])?;
        let groups =
            octs_count.div_ceil(self.subgroup) + bricks_count * (self.per_block() / self.subgroup);
        held.write(INDIRECT as usize, &[groups, 1, 1])
    }

    pub fn unstamp(&self, held: &mut Resident<'_>) -> Result<(), Error> {
        let mut table = held.read(self.table() as usize, self.table_words())?;
        for entry in table.chunks_mut(ENTRY).take(self.count() as usize) {
            for at in [TOUCHED, WANTED] {
                if let Some(word) = entry.get_mut(at) {
                    *word = 0;
                }
            }
        }
        held.write(self.table() as usize, &table)
    }

    pub fn seed(&self, held: &mut Resident<'_>, plan: &Plan) -> Result<(), Error> {
        use crate::layout::{AWAKE, HOT, HOT_COUNTS, HOT_DENSE, HOT_GRAND, HOT_INDIRECT, TALLY};

        let first = held.read(PARAMS as usize, 1)?;
        let frame = first.first().copied().unwrap_or(u32::MAX);
        held.write(PARAMS as usize, &[frame, self.dump()])?;
        let table = held.read(self.table() as usize, self.table_words())?;
        let per_block = self.per_block() as usize;
        let runs = self.held_runs(&table);
        let longest = runs.iter().map(|&(_, slots)| slots).max().unwrap_or(0);
        let marked = vec![AWAKE_LEVEL; per_block * longest as usize];
        let writes: Vec<Run<'_>> = runs
            .iter()
            .map(|&(first, slots)| Run {
                index: AWAKE as usize,
                at: first as usize * per_block,
                words: marked.get(..slots as usize * per_block).unwrap_or_default(),
            })
            .collect();
        held.write_many(&writes)?;
        let count = self.count();
        held.write(HOT as usize, &vec![AWAKE_LEVEL; count as usize])?;
        held.write_at(HOT as usize, self.cold as usize, &[0])?;
        held.write(HOT_DENSE as usize, &(0..count).collect::<Vec<u32>>())?;
        held.write(HOT_GRAND as usize, &[count])?;
        held.write(HOT_INDIRECT as usize, &[count, 1, 1])?;
        let hot_counts = plan.hot_tiles as usize * plan.per_block as usize;
        held.write(HOT_COUNTS as usize, &vec![0; hot_counts.max(1)])?;
        held.write(
            TALLY as usize,
            &vec![0; Dimension::Three.tally_words() * live::TALLY_SPAN],
        )?;
        let tiles: Vec<u32> = (0..self.tile_count()).collect();
        held.write(self.tile_list() as usize, &tiles)?;
        held.write(self.tile_args() as usize, &self.tile_words(&tiles))?;
        self.list(held, &[], &(0..count).collect::<Vec<u32>>())
    }

    #[must_use]
    pub fn held_runs(&self, table: &[u32]) -> Vec<(u32, u32)> {
        let mut slots: Vec<u32> = table
            .chunks(ENTRY)
            .take(self.count() as usize)
            .filter_map(|entry| entry.first().copied())
            .filter(|&slot| slot != UNIFORM)
            .collect();
        slots.sort_unstable();
        slots.dedup();
        let mut runs: Vec<(u32, u32)> = Vec::new();
        for slot in slots {
            match runs.last_mut() {
                Some((first, length)) if *first + *length == slot => *length += 1,
                _ => runs.push((slot, 1)),
            }
        }
        runs
    }
}

macro_rules! widths {
    ($subgroup:expr, $call:ident, [$($kind:ty),*], ($($arg:expr),*)) => {
        match $subgroup {
            4 => $call::<4 $(, $kind)*>($($arg),*),
            8 => $call::<8 $(, $kind)*>($($arg),*),
            16 => $call::<16 $(, $kind)*>($($arg),*),
            32 => $call::<32 $(, $kind)*>($($arg),*),
            64 => $call::<64 $(, $kind)*>($($arg),*),
            other => Err(LaneError::NoMapping {
                lanes: other,
                width: $subgroup,
            }),
        }
    };
}

pub fn settle<R: Rule>(
    rule: &R,
    bricks: &Bricks,
    act: Act,
    listing: Listing,
) -> Result<Vec<u32>, LaneError> {
    if rule.planes() != bricks.planes() {
        return Err(LaneError::NoSuchForm {
            operation: "bricks::settle",
            because: UNLIKE_PLANES,
        });
    }
    if rule.looks().planes() > bricks.planes() {
        return Err(LaneError::NoSuchForm {
            operation: "bricks::settle",
            because: crate::sand3::PAST_HALO,
        });
    }
    let subgroup = bricks.subgroup;
    widths!(subgroup, settled, [R], (rule, bricks, act, listing))
}

fn ones<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    truth: simdr::lanes::Predicate<LANES>,
) -> Result<Vector<U32, LANES>, LaneError> {
    let one = lanes.splat_bits::<U32, LANES>(1)?;
    let none = lanes.splat_bits::<U32, LANES>(0)?;
    lanes.select(truth, one, none)
}

fn truth<const LANES: u32>(
    kernel: &mut Kernel<U32>,
    when: Vector<U32, LANES>,
) -> Result<Id, LaneError> {
    let mut lanes = kernel.lanes()?;
    let none = lanes.splat_bits::<U32, LANES>(0)?;
    let holds = lanes.greater_than(when, none)?;
    holds
        .strips()
        .first()
        .copied()
        .ok_or(LaneError::NoOpenBlock { arm: "when" })
}

fn store_if<const LANES: u32>(
    kernel: &mut Kernel<U32>,
    when: Vector<U32, LANES>,
    binding: u32,
    at: Id,
    value: Id,
) -> Result<(), LaneError> {
    let holds = truth::<LANES>(kernel, when)?;
    live::lane_if(kernel, holds, |kernel| kernel.store_at(binding, at, value))
}

fn listed_oct<const LANES: u32>(
    kernel: &mut Kernel<U32>,
    bricks: &Bricks,
    slot: Id,
) -> Result<(Id, Vector<U32, LANES>), LaneError> {
    let subgroup = bricks.subgroup;
    let head = kernel.module().constant_u32(0)?;
    let grand = kernel.load_at(GRAND, head)?;
    let whole = kernel.load_at(bricks.whole_count(), head)?;
    let (dense_at, whole_at, local, in_list, inside) = {
        let mut lanes = kernel.lanes()?;
        let here = lanes.splat_id::<U32, LANES>(slot)?;
        let listed = lanes.splat_id::<U32, LANES>(grand)?;
        let bricks_listed = lanes.splat_id::<U32, LANES>(whole)?;
        let short = lanes.splat_bits::<U32, LANES>(subgroup - 1)?;
        let mask = lanes.splat_bits::<U32, LANES>(!(subgroup - 1))?;
        let by = lanes.splat_bits::<U32, LANES>(bricks.octs_bits())?;
        let low = lanes.splat_bits::<U32, LANES>(bricks.per_block() - 1)?;
        let none = lanes.splat_bits::<U32, LANES>(0)?;
        let rounded = lanes.add(listed, short)?;
        let rounded = lanes.and(rounded, mask)?;
        let in_list = lanes.greater_than(rounded, here)?;
        let past = lanes.sub(here, rounded)?;
        let which = lanes.shift_right_logical(past, by)?;
        let local = lanes.and(past, low)?;
        let in_dense = lanes.greater_than(listed, here)?;
        let in_whole = lanes.greater_than(bricks_listed, which)?;
        let dense_at = lanes.select(in_list, here, none)?;
        let whole_at = lanes.select(in_whole, which, none)?;
        let whole_at = lanes.select(in_list, none, whole_at)?;
        let in_dense = ones(&mut lanes, in_dense)?;
        let in_whole = ones(&mut lanes, in_whole)?;
        let inside = lanes.select(in_list, in_dense, in_whole)?;
        (
            dense_at,
            whole_at,
            local,
            ones(&mut lanes, in_list)?,
            inside,
        )
    };
    let from_list = kernel.load_at(DENSE, dense_at.id())?;
    let brick = kernel.load_at(bricks.whole_list(), whole_at.id())?;
    let oct = {
        let mut lanes = kernel.lanes()?;
        let from_list = lanes.splat_id::<U32, LANES>(from_list)?;
        let brick = lanes.splat_id::<U32, LANES>(brick)?;
        let by = lanes.splat_bits::<U32, LANES>(bricks.octs_bits())?;
        let none = lanes.splat_bits::<U32, LANES>(0)?;
        let lifted = lanes.shift_left(brick, by)?;
        let from_brick = lanes.add(lifted, local)?;
        let listed = lanes.greater_than(in_list, none)?;
        lanes.select(listed, from_list, from_brick)?
    };
    Ok((oct.id(), inside))
}

fn wake<const LANES: u32>(
    kernel: &mut Kernel<U32>,
    bricks: &Bricks,
    at: [Vector<U32, LANES>; 3],
    waking: Vector<U32, LANES>,
) -> Result<(), LaneError> {
    use crate::geometry::Solid;
    use crate::layout::{AWAKE, HOT};

    let table = bricks.table();
    crate::gate::if_any::<LANES, _>(kernel, waking.id(), |kernel| {
        for offset in Solid::near() {
            let (entry_at, brick, local) = {
                let mut lanes = kernel.lanes()?;
                let near = bricks.neighbour(&mut Emit::<LANES>(&mut lanes), at, offset)?;
                let by = lanes.splat_bits::<U32, LANES>(bricks.octs_bits())?;
                let low = lanes.splat_bits::<U32, LANES>(bricks.per_block() - 1)?;
                let four = lanes.splat_bits::<U32, LANES>(ENTRY as u32)?;
                let brick = lanes.shift_right_logical(near, by)?;
                let local = lanes.and(near, low)?;
                (lanes.mul(brick, four)?, brick, local)
            };
            let entry = kernel.load_at(table, entry_at.id())?;
            let (flag_at, both) = {
                let mut lanes = kernel.lanes()?;
                let entry = lanes.splat_id::<U32, LANES>(entry)?;
                let uniform = lanes.splat_bits::<U32, LANES>(UNIFORM)?;
                let per = lanes.splat_bits::<U32, LANES>(bricks.per_block())?;
                let held = lanes.not_equal(entry, uniform)?;
                let held = ones(&mut lanes, held)?;
                let both = lanes.mul(held, waking)?;
                let start = lanes.mul(entry, per)?;
                (lanes.add(start, local)?, both)
            };
            let mark = kernel.module().constant_u32(AWAKE_LEVEL)?;
            store_if::<LANES>(kernel, both, AWAKE, flag_at.id(), mark)?;
            store_if::<LANES>(kernel, waking, HOT, brick.id(), mark)?;
        }
        Ok(())
    })
}

fn settled<const LANES: u32, R: Rule>(
    rule: &R,
    bricks: &Bricks,
    act: Act,
    listing: Listing,
) -> Result<Vec<u32>, LaneError> {
    use crate::sand3::{Given, Placing, frame_and_params, ruled, ruled_planes};

    let subgroup = bricks.subgroup;
    let windows = bricks.windows();
    let table = bricks.table();
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, bricks.bindings()))?;
    let uint = kernel.index_type();

    let group = kernel.workgroup_index();
    let lane = kernel.local_index();
    let width = kernel.module().constant_u32(subgroup)?;
    let base = kernel.module().i_mul(uint, group, width)?;
    let slot = kernel.module().i_add(uint, base, lane)?;

    let (frame, params) = frame_and_params(&mut kernel, rule)?;

    let (oct, inside) = match listing {
        Listing::Listed => listed_oct::<LANES>(&mut kernel, bricks, slot)?,
        Listing::Every => {
            let mut lanes = kernel.lanes()?;
            let one = lanes.splat_bits::<U32, LANES>(1)?;
            (slot, one)
        }
    };

    let stopped = {
        let at = kernel.module().constant_u32(OVERFLOW as u32)?;
        kernel.load_at(bricks.slots(), at)?
    };

    let placed = {
        let mut lanes = kernel.lanes()?;
        let which = lanes.splat_id::<U32, LANES>(oct)?;
        let now = lanes.splat_id::<U32, LANES>(frame)?;
        bricks.placed(&mut Emit::<LANES>(&mut lanes), which, now)?
    };

    let looks = rule.looks();
    let planes = bricks.planes();
    let guard = match listing {
        Listing::Listed if !looks.is_empty() => Some(inside),
        Listing::Listed | Listing::Every => None,
    };
    let origin = box_origin::<LANES>(&mut kernel, placed.block, frame)?;
    let seen = box_seen(rule, bricks);
    let flat = corner_words::<LANES>(&mut kernel, bricks, &placed, frame)?;
    let mut flat = flat.into_iter();
    let mut taken =
        |count: usize| -> Vec<Id> { (0..count).map(|_| flat.next().unwrap_or(frame)).collect() };
    let as_corners = |ids: Vec<Id>| -> [Id; CORNERS] {
        let mut out = [frame; CORNERS];
        for (slot, id) in out.iter_mut().zip(ids) {
            *slot = id;
        }
        out
    };
    let values = as_corners(taken(CORNERS));
    let address_ids = taken(CORNERS);
    let holding_ids = taken(CORNERS);
    let words = as_corners(taken(CORNERS));
    let mut plane_values: Vec<[Id; CORNERS]> = Vec::new();
    let mut plane_words: Vec<[Id; CORNERS]> = Vec::new();
    let mut plane_address_ids: Vec<Vec<Id>> = Vec::new();
    for _ in 1..planes {
        plane_values.push(as_corners(taken(CORNERS)));
        plane_words.push(as_corners(taken(CORNERS)));
        plane_address_ids.push(taken(CORNERS));
    }
    let (addresses, holding, plane_addresses) = {
        let lanes = kernel.lanes()?;
        let vectors = |ids: &[Id]| -> Result<Vec<Vector<U32, LANES>>, LaneError> {
            ids.iter()
                .map(|&id| lanes.splat_id::<U32, LANES>(id))
                .collect()
        };
        let addresses = vectors(&address_ids)?;
        let holding = vectors(&holding_ids)?;
        let mut plane_addresses = Vec::with_capacity(plane_address_ids.len());
        for ids in &plane_address_ids {
            plane_addresses.push(vectors(ids)?);
        }
        (addresses, holding, plane_addresses)
    };

    let stepped = |kernel: &mut Kernel<U32>| {
        let halo = if looks.is_empty() {
            Vec::new()
        } else {
            let words = box_words::<LANES>(kernel, bricks, (origin, guard), &seen)?;
            halo_of(&seen, planes, &words, frame)
        };
        let mut lanes = kernel.lanes()?;
        let given = Given {
            frame,
            params: &params,
        };
        let placing = Placing {
            places: [placed.block[0]; CORNERS],
            block: placed.block,
        };
        if !looks.is_empty() {
            let mut loaded = vec![words];
            loaded.extend(plane_words.iter().copied());
            let planed = crate::sand3::ruled_halo::<LANES, R>(
                &mut lanes,
                rule,
                &placing,
                (&loaded, &halo),
                given,
            )?;
            return Ok((planed.ruled, planed.planes));
        }
        if planes > 1 {
            let mut loaded = vec![words];
            loaded.extend(plane_words.iter().copied());
            let planed = ruled_planes::<LANES, R>(&mut lanes, rule, &placing, &loaded, given)?;
            return Ok((planed.ruled, planed.planes));
        }
        Ok::<_, LaneError>((
            ruled::<LANES, R>(&mut lanes, rule, &placing, words, given)?,
            Vec::new(),
        ))
    };

    match act {
        Act::Dry => {
            let stamp = {
                let mut lanes = kernel.lanes()?;
                let now = lanes.splat_id::<U32, LANES>(frame)?;
                let one = lanes.splat_bits::<U32, LANES>(1)?;
                let stamp = lanes.add(now, one)?;
                lanes.max(stamp, one)?
            };
            let mut uniform_any = None;
            for (index, (cell, &held)) in placed.cells.iter().zip(&holding).enumerate() {
                let (touched, first, empty) = {
                    let mut lanes = kernel.lanes()?;
                    let four = lanes.splat_bits::<U32, LANES>(ENTRY as u32)?;
                    let touched_at = lanes.splat_bits::<U32, LANES>(TOUCHED as u32)?;
                    let none = lanes.splat_bits::<U32, LANES>(0)?;
                    let entry = lanes.mul(cell.brick, four)?;
                    let mut first = inside;
                    for earlier in placed.cells.iter().take(index) {
                        let other = lanes.not_equal(earlier.brick, cell.brick)?;
                        let other = ones(&mut lanes, other)?;
                        first = lanes.mul(first, other)?;
                    }
                    let empty = lanes.equal(held, none)?;
                    let empty = ones(&mut lanes, empty)?;
                    let empty = match uniform_any {
                        None => empty,
                        Some(so_far) => lanes.max(so_far, empty)?,
                    };
                    (lanes.add(entry, touched_at)?, first, empty)
                };
                uniform_any = Some(empty);
                store_if::<LANES>(&mut kernel, first, table, touched.id(), stamp.id())?;
            }
            let needs = {
                let mut lanes = kernel.lanes()?;
                let none = lanes.splat_bits::<U32, LANES>(0)?;
                let any = uniform_any.map_or(Ok(none), Ok::<_, LaneError>)?;
                lanes.mul(any, inside)?
            };
            crate::gate::if_any::<LANES, _>(&mut kernel, needs.id(), |kernel| {
                let (step, upper) = stepped(kernel)?;
                for (corner, ((cell, &value), (&held, out))) in placed
                    .cells
                    .iter()
                    .zip(&values)
                    .zip(holding.iter().zip(&step.out))
                    .enumerate()
                {
                    let (wanted, wants) = {
                        let mut lanes = kernel.lanes()?;
                        let none = lanes.splat_bits::<U32, LANES>(0)?;
                        let four = lanes.splat_bits::<U32, LANES>(ENTRY as u32)?;
                        let wanted_at = lanes.splat_bits::<U32, LANES>(WANTED as u32)?;
                        let entry = lanes.mul(cell.brick, four)?;
                        let value = lanes.splat_id::<U32, LANES>(value)?;
                        let differs = lanes.not_equal(*out, value)?;
                        let mut differs = ones(&mut lanes, differs)?;
                        for (outs, plane_value) in upper.iter().zip(&plane_values) {
                            if let (Some(out), Some(&plane_value)) =
                                (outs.get(corner), plane_value.get(corner))
                            {
                                let plane_value = lanes.splat_id::<U32, LANES>(plane_value)?;
                                let other = lanes.not_equal(*out, plane_value)?;
                                let other = ones(&mut lanes, other)?;
                                differs = lanes.max(differs, other)?;
                            }
                        }
                        let uniform = lanes.equal(held, none)?;
                        let uniform = ones(&mut lanes, uniform)?;
                        let wants = lanes.mul(differs, uniform)?;
                        let wants = lanes.mul(wants, inside)?;
                        (lanes.add(entry, wanted_at)?, wants)
                    };
                    store_if::<LANES>(kernel, wants, table, wanted.id(), stamp.id())?;
                }
                Ok(())
            })?;
        }
        Act::Settle => {
            let (step, upper) = stepped(&mut kernel)?;
            let go = {
                let mut lanes = kernel.lanes()?;
                let none = lanes.splat_bits::<U32, LANES>(0)?;
                let stopped = lanes.splat_id::<U32, LANES>(stopped)?;
                let running = lanes.equal(stopped, none)?;
                let running = ones(&mut lanes, running)?;
                lanes.mul(running, inside)?
            };
            for (((address, &held), &value), out) in
                addresses.iter().zip(&holding).zip(&values).zip(&step.out)
            {
                let (kept, lost) = {
                    let mut lanes = kernel.lanes()?;
                    let none = lanes.splat_bits::<U32, LANES>(0)?;
                    let kept = lanes.mul(go, held)?;
                    let value = lanes.splat_id::<U32, LANES>(value)?;
                    let differs = lanes.not_equal(*out, value)?;
                    let differs = ones(&mut lanes, differs)?;
                    let uniform = lanes.equal(held, none)?;
                    let uniform = ones(&mut lanes, uniform)?;
                    let lost = lanes.mul(differs, uniform)?;
                    (kept, lanes.mul(lost, go)?)
                };
                let storing = truth::<LANES>(&mut kernel, kept)?;
                live::lane_if(&mut kernel, storing, |kernel| {
                    live::state_store(kernel, windows, address.id(), out.id())
                })?;
                let flagged = kernel.module().constant_u32(LOST as u32)?;
                let one = kernel.module().constant_u32(1)?;
                store_if::<LANES>(&mut kernel, lost, bricks.slots(), flagged, one)?;
            }
            for ((outs, plane_value), plane_address) in
                upper.iter().zip(&plane_values).zip(&plane_addresses)
            {
                for (((address, &held), &value), out) in plane_address
                    .iter()
                    .zip(&holding)
                    .zip(plane_value)
                    .zip(outs)
                {
                    let (kept, lost) = {
                        let mut lanes = kernel.lanes()?;
                        let none = lanes.splat_bits::<U32, LANES>(0)?;
                        let kept = lanes.mul(go, held)?;
                        let value = lanes.splat_id::<U32, LANES>(value)?;
                        let differs = lanes.not_equal(*out, value)?;
                        let differs = ones(&mut lanes, differs)?;
                        let uniform = lanes.equal(held, none)?;
                        let uniform = ones(&mut lanes, uniform)?;
                        let lost = lanes.mul(differs, uniform)?;
                        (kept, lanes.mul(lost, go)?)
                    };
                    let storing = truth::<LANES>(&mut kernel, kept)?;
                    live::lane_if(&mut kernel, storing, |kernel| {
                        live::state_store(kernel, windows, address.id(), out.id())
                    })?;
                    let flagged = kernel.module().constant_u32(LOST as u32)?;
                    let one = kernel.module().constant_u32(1)?;
                    store_if::<LANES>(&mut kernel, lost, bricks.slots(), flagged, one)?;
                }
            }

            if listing == Listing::Listed {
                let (waking, counting) = {
                    let mut lanes = kernel.lanes()?;
                    let one = lanes.splat_bits::<U32, LANES>(1)?;
                    let none = lanes.splat_bits::<U32, LANES>(0)?;
                    let mut touched = none;
                    for (&before, &after) in words.iter().zip(&step.out) {
                        let before = lanes.splat_id::<U32, LANES>(before)?;
                        let same = lanes.equal(before, after)?;
                        let differs = lanes.select(same, none, one)?;
                        touched = lanes.add(touched, differs)?;
                    }
                    for (befores, afters) in plane_words.iter().zip(&upper) {
                        for (&before, &after) in befores.iter().zip(afters) {
                            let before = lanes.splat_id::<U32, LANES>(before)?;
                            let same = lanes.equal(before, after)?;
                            let differs = lanes.select(same, none, one)?;
                            touched = lanes.add(touched, differs)?;
                        }
                    }
                    (lanes.mul(touched, go)?, go)
                };
                let at = placed.block;
                wake::<LANES>(&mut kernel, bricks, at, waking)?;
                live::tally3::<LANES>(
                    &mut kernel,
                    live::Tallied3 {
                        counted: &step.counts,
                        at,
                        inside: counting,
                        weighted: rule.tally().weighted,
                    },
                )?;
            }
        }
    }
    kernel.finish()
}

fn box_origin<const LANES: u32>(
    kernel: &mut Kernel<U32>,
    block: [Vector<U32, LANES>; 3],
    now: Id,
) -> Result<[Vector<U32, LANES>; 3], LaneError> {
    let mut lanes = kernel.lanes()?;
    let current = lanes.splat_id::<U32, LANES>(now)?;
    let one = lanes.splat_bits::<U32, LANES>(1)?;
    let back = lanes.splat_bits::<U32, LANES>(u32::MAX)?;
    let phase = lanes.and(current, one)?;
    let mut origin = block;
    for value in &mut origin {
        *value = lanes.shift_left(*value, one)?;
        *value = lanes.add(*value, phase)?;
        *value = lanes.add(*value, back)?;
    }
    Ok(origin)
}

fn box_seen<R: Rule>(rule: &R, bricks: &Bricks) -> Vec<(usize, Vec<usize>)> {
    let looks = rule.looks();
    if looks.is_empty() {
        return Vec::new();
    }
    (0..BOX)
        .map(|at| {
            let wanted: Vec<usize> = (0..bricks.planes())
                .filter(|&plane| (looks.cells(plane) >> at) & 1 == 1)
                .collect();
            (at, wanted)
        })
        .filter(|(_, wanted)| !wanted.is_empty())
        .collect()
}

fn halo_of(seen: &[(usize, Vec<usize>)], planes: usize, words: &[Id], now: Id) -> Vec<[Id; BOX]> {
    let mut halo = vec![[now; BOX]; planes];
    let mut words = words.iter();
    for (at, wanted) in seen {
        for &plane in wanted {
            let word = words.next().copied().unwrap_or(now);
            if let Some(slot) = halo.get_mut(plane).and_then(|row| row.get_mut(*at)) {
                *slot = word;
            }
        }
    }
    halo
}

fn corner_words<const LANES: u32>(
    kernel: &mut Kernel<U32>,
    bricks: &Bricks,
    placed: &Placed<Vector<U32, LANES>>,
    now: Id,
) -> Result<Vec<Id>, LaneError> {
    let windows = bricks.windows();
    let table = bricks.table();
    let mut entries = [now; CORNERS];
    let mut values = [now; CORNERS];
    for ((entry, value), cell) in entries.iter_mut().zip(values.iter_mut()).zip(&placed.cells) {
        let at = {
            let mut lanes = kernel.lanes()?;
            let four = lanes.splat_bits::<U32, LANES>(ENTRY as u32)?;
            let one = lanes.splat_bits::<U32, LANES>(1)?;
            let at = lanes.mul(cell.brick, four)?;
            let next = lanes.add(at, one)?;
            (at, next)
        };
        *entry = kernel.load_at(table, at.0.id())?;
        *value = kernel.load_at(table, at.1.id())?;
    }

    let cell_bits = bricks.cell_bits();
    let mut addresses = Vec::with_capacity(CORNERS);
    let mut holding = Vec::with_capacity(CORNERS);
    let mut words = Vec::with_capacity(CORNERS);
    for ((&entry, &value), cell) in entries.iter().zip(&values).zip(&placed.cells) {
        let (address, held) = {
            let mut lanes = kernel.lanes()?;
            let entry = lanes.splat_id::<U32, LANES>(entry)?;
            let uniform = lanes.splat_bits::<U32, LANES>(UNIFORM)?;
            let by = lanes.splat_bits::<U32, LANES>(cell_bits)?;
            let none = lanes.splat_bits::<U32, LANES>(0)?;
            let held = lanes.not_equal(entry, uniform)?;
            let start = lanes.shift_left(entry, by)?;
            let address = lanes.add(start, cell.offset)?;
            let address = lanes.select(held, address, none)?;
            (address, ones(&mut lanes, held)?)
        };
        let loaded = live::state_load(kernel, windows, address.id())?;
        words.push({
            let mut lanes = kernel.lanes()?;
            let loaded = lanes.splat_id::<U32, LANES>(loaded)?;
            let value = lanes.splat_id::<U32, LANES>(value)?;
            let none = lanes.splat_bits::<U32, LANES>(0)?;
            let kept = lanes.greater_than(held, none)?;
            lanes.select(kept, loaded, value)?.id()
        });
        addresses.push(address);
        holding.push(held);
    }

    let mut out: Vec<Id> = values.to_vec();
    out.extend(addresses.iter().map(Vector::id));
    out.extend(holding.iter().map(Vector::id));
    out.extend(words.iter().copied());
    for plane in 1..bricks.planes() {
        let mut values_of = Vec::with_capacity(CORNERS);
        let mut words_of = Vec::with_capacity(CORNERS);
        let mut addresses_of = Vec::with_capacity(CORNERS);
        for (cell, (address, held)) in placed.cells.iter().zip(addresses.iter().zip(&holding)) {
            let (value_at, lifted) = {
                let mut lanes = kernel.lanes()?;
                let base = lanes.splat_bits::<U32, LANES>(bricks.plane_values(plane))?;
                let by = lanes.splat_bits::<U32, LANES>(bricks.plane_start(plane))?;
                (lanes.add(cell.brick, base)?, lanes.add(*address, by)?)
            };
            let value = kernel.load_at(table, value_at.id())?;
            let loaded = live::state_load(kernel, windows, lifted.id())?;
            words_of.push({
                let mut lanes = kernel.lanes()?;
                let loaded = lanes.splat_id::<U32, LANES>(loaded)?;
                let value = lanes.splat_id::<U32, LANES>(value)?;
                let none = lanes.splat_bits::<U32, LANES>(0)?;
                let kept = lanes.greater_than(*held, none)?;
                lanes.select(kept, loaded, value)?.id()
            });
            values_of.push(value);
            addresses_of.push(lifted.id());
        }
        out.extend(values_of);
        out.extend(words_of);
        out.extend(addresses_of);
    }
    Ok(out)
}

fn box_words<const LANES: u32>(
    kernel: &mut Kernel<U32>,
    bricks: &Bricks,
    (origin, inside): ([Vector<U32, LANES>; 3], Option<Vector<U32, LANES>>),
    seen: &[(usize, Vec<usize>)],
) -> Result<Vec<Id>, LaneError> {
    let windows = bricks.windows();
    let table = bricks.table();
    let mut out = Vec::new();
    for (at, wanted) in seen {
        let (entry_at, brick, offset) = {
            let mut lanes = kernel.lanes()?;
            let mut here = origin;
            for (value, step) in here.iter_mut().zip(point(*at)) {
                let step = lanes.splat_bits::<U32, LANES>(u32::try_from(step).unwrap_or(0))?;
                *value = lanes.add(*value, step)?;
            }
            let cell = bricks.place(&mut Emit::<LANES>(&mut lanes), here)?;
            let four = lanes.splat_bits::<U32, LANES>(ENTRY as u32)?;
            let entry_at = lanes.mul(cell.brick, four)?;
            match inside {
                Some(inside) => {
                    let none = lanes.splat_bits::<U32, LANES>(0)?;
                    let listed = lanes.greater_than(inside, none)?;
                    (
                        lanes.select(listed, entry_at, none)?,
                        lanes.select(listed, cell.brick, none)?,
                        lanes.select(listed, cell.offset, none)?,
                    )
                }
                None => (entry_at, cell.brick, cell.offset),
            }
        };
        let entry = kernel.load_at(table, entry_at.id())?;
        let (address, held) = {
            let mut lanes = kernel.lanes()?;
            let entry = lanes.splat_id::<U32, LANES>(entry)?;
            let uniform = lanes.splat_bits::<U32, LANES>(UNIFORM)?;
            let by = lanes.splat_bits::<U32, LANES>(bricks.cell_bits())?;
            let none = lanes.splat_bits::<U32, LANES>(0)?;
            let held = lanes.not_equal(entry, uniform)?;
            let start = lanes.shift_left(entry, by)?;
            let address = lanes.add(start, offset)?;
            (lanes.select(held, address, none)?, ones(&mut lanes, held)?)
        };
        for &plane in wanted {
            let (value_at, lifted) = {
                let mut lanes = kernel.lanes()?;
                let value_at = if plane == 0 {
                    let one = lanes.splat_bits::<U32, LANES>(VALUE as u32)?;
                    lanes.add(entry_at, one)?
                } else {
                    let base = lanes.splat_bits::<U32, LANES>(bricks.plane_values(plane))?;
                    lanes.add(brick, base)?
                };
                let by = lanes.splat_bits::<U32, LANES>(bricks.plane_start(plane))?;
                (value_at, lanes.add(address, by)?)
            };
            let value = kernel.load_at(table, value_at.id())?;
            let loaded = live::state_load(kernel, windows, lifted.id())?;
            let word = {
                let mut lanes = kernel.lanes()?;
                let loaded = lanes.splat_id::<U32, LANES>(loaded)?;
                let value = lanes.splat_id::<U32, LANES>(value)?;
                let none = lanes.splat_bits::<U32, LANES>(0)?;
                let kept = lanes.greater_than(held, none)?;
                lanes.select(kept, loaded, value)?
            };
            out.push(word.id());
        }
    }
    Ok(out)
}

fn per_brick(kernel: &mut Kernel<U32>, subgroup: u32) -> Result<Id, LaneError> {
    let uint = kernel.index_type();
    let group = kernel.workgroup_index();
    let lane = kernel.local_index();
    let width = kernel.module().constant_u32(subgroup)?;
    let base = kernel.module().i_mul(uint, group, width)?;
    Ok(kernel.module().i_add(uint, base, lane)?)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reach {
    Each,
    Tiled,
}

fn brick_lane<const LANES: u32>(
    kernel: &mut Kernel<U32>,
    bricks: &Bricks,
    reach: Reach,
) -> Result<Id, LaneError> {
    use crate::sweep::PER_LANE;

    let subgroup = bricks.subgroup;
    if reach == Reach::Each {
        return per_brick(kernel, subgroup);
    }
    let group = kernel.workgroup_index();
    let lane = kernel.local_index();
    let rank = {
        let mut lanes = kernel.lanes()?;
        let here = lanes.splat_id::<U32, LANES>(group)?;
        let by = lanes.splat_bits::<U32, LANES>(PER_LANE.trailing_zeros())?;
        lanes.shift_right_logical(here, by)?
    };
    let tile = kernel.load_at(bricks.tile_list(), rank.id())?;
    let mut lanes = kernel.lanes()?;
    let here = lanes.splat_id::<U32, LANES>(group)?;
    let seat = lanes.splat_id::<U32, LANES>(lane)?;
    let tile = lanes.splat_id::<U32, LANES>(tile)?;
    let low = lanes.splat_bits::<U32, LANES>(PER_LANE - 1)?;
    let width = lanes.splat_bits::<U32, LANES>(subgroup)?;
    let by = lanes.splat_bits::<U32, LANES>(bricks.per_block().trailing_zeros())?;
    let within = lanes.and(here, low)?;
    let within = lanes.mul(within, width)?;
    let within = lanes.add(within, seat)?;
    let start = lanes.shift_left(tile, by)?;
    Ok(lanes.add(start, within)?.id())
}

pub fn lighting(bricks: &Bricks) -> Result<Vec<u32>, LaneError> {
    let subgroup = bricks.subgroup;
    widths!(subgroup, lit, [], (bricks))
}

fn lit<const LANES: u32>(bricks: &Bricks) -> Result<Vec<u32>, LaneError> {
    use crate::geometry::Solid;
    use crate::layout::HOT_COUNTS;

    let subgroup = bricks.subgroup;
    let lattice = bricks.tile_lattice();
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, bricks.bindings()))?;
    let tile = crate::compact::owned(&mut kernel, Over::Listed(bricks.tile_list()))?;
    let hot = kernel.load_at(HOT_COUNTS, tile)?;
    let lane = kernel.local_index();
    let head = kernel.module().constant_u32(0)?;
    let now = kernel.load_at(PARAMS, head)?;
    let near = Solid::near();
    let reach = u32::try_from(near.len()).unwrap_or(u32::MAX);
    let (stamp, at) = {
        let mut lanes = kernel.lanes()?;
        let now = lanes.splat_id::<U32, LANES>(now)?;
        let one = lanes.splat_bits::<U32, LANES>(1)?;
        let stamp = lanes.add(now, one)?;
        let stamp = lanes.max(stamp, one)?;
        let here = lanes.splat_id::<U32, LANES>(tile)?;
        (
            stamp,
            Solid::split(&mut Emit::<LANES>(&mut lanes), lattice, here)?,
        )
    };
    for round in 0..reach.div_ceil(subgroup) {
        let (tile, inside) = {
            let mut lanes = kernel.lanes()?;
            let seat = lanes.splat_id::<U32, LANES>(lane)?;
            let from = lanes.splat_bits::<U32, LANES>(round * subgroup)?;
            let which = lanes.add(seat, from)?;
            let count = lanes.splat_bits::<U32, LANES>(reach)?;
            let inside = lanes.greater_than(count, which)?;
            let none = lanes.splat_bits::<U32, LANES>(0)?;
            let warm = lanes.splat_id::<U32, LANES>(hot)?;
            let warm = lanes.greater_than(warm, none)?;
            let warm = ones(&mut lanes, warm)?;
            let inside = ones(&mut lanes, inside)?;
            let inside = lanes.mul(inside, warm)?;
            let back = lanes.splat_bits::<U32, LANES>(u32::MAX)?;
            let eleven = lanes.splat_bits::<U32, LANES>(11)?;
            let five = lanes.splat_bits::<U32, LANES>(5)?;
            let three = lanes.splat_bits::<U32, LANES>(3)?;
            let mut rest = which;
            let mut point = at;
            for value in &mut point {
                let scaled = lanes.mul(rest, eleven)?;
                let third = lanes.shift_right_logical(scaled, five)?;
                let whole = lanes.mul(third, three)?;
                let step = lanes.sub(rest, whole)?;
                rest = third;
                *value = lanes.add(*value, step)?;
                *value = lanes.add(*value, back)?;
            }
            (
                Solid::at(&mut Emit::<LANES>(&mut lanes), lattice, point)?,
                inside,
            )
        };
        let mark = kernel.load_at(bricks.tiles(), tile.id())?;
        let unlit = {
            let mut lanes = kernel.lanes()?;
            let mark = lanes.splat_id::<U32, LANES>(mark)?;
            let other = lanes.not_equal(mark, stamp)?;
            let other = ones(&mut lanes, other)?;
            lanes.mul(other, inside)?
        };
        store_if::<LANES>(&mut kernel, unlit, bricks.tiles(), tile.id(), stamp.id())?;
    }
    kernel.finish()
}

pub fn tiling(bricks: &Bricks) -> Result<Vec<u32>, LaneError> {
    let subgroup = bricks.subgroup;
    widths!(subgroup, tiled, [], (bricks))
}

#[must_use]
pub const fn tile_run(bricks: &Bricks) -> u32 {
    let per_lane = bricks
        .tile_count()
        .div_ceil(bricks.subgroup)
        .next_power_of_two();
    if per_lane > 64 { 64 } else { per_lane }
}

fn tiled<const LANES: u32>(bricks: &Bricks) -> Result<Vec<u32>, LaneError> {
    use crate::sweep::PER_LANE;

    let subgroup = bricks.subgroup;
    let count = bricks.tile_count();
    let run = tile_run(bricks);
    let span = subgroup * run;
    let rounds = count.div_ceil(span);
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, bricks.bindings()))?;
    let uint = kernel.index_type();
    let lane = kernel.local_index();
    let head = kernel.module().constant_u32(0)?;
    let now = kernel.load_at(PARAMS, head)?;
    let (stamp, before) = {
        let mut lanes = kernel.lanes()?;
        let now = lanes.splat_id::<U32, LANES>(now)?;
        let one = lanes.splat_bits::<U32, LANES>(1)?;
        let stamp = lanes.add(now, one)?;
        (lanes.max(stamp, one)?, lanes.max(now, one)?)
    };
    let found = kernel.repeat_rolled_many(rounds, uint, &[head], |kernel, carried, along| {
        let base = carried.first().copied().unwrap_or(head);
        let start = {
            let mut lanes = kernel.lanes()?;
            let round = lanes.splat_id::<U32, LANES>(along)?;
            let seat = lanes.splat_id::<U32, LANES>(lane)?;
            let wide = lanes.splat_bits::<U32, LANES>(span)?;
            let per = lanes.splat_bits::<U32, LANES>(run)?;
            let from = lanes.mul(round, wide)?;
            let mine = lanes.mul(seat, per)?;
            lanes.add(from, mine)?
        };
        let mut tiles = Vec::with_capacity(run as usize);
        let mut marks = Vec::with_capacity(run as usize);
        for step in 0..run {
            let (tile, at) = {
                let mut lanes = kernel.lanes()?;
                let along = lanes.splat_bits::<U32, LANES>(step)?;
                let past = lanes.splat_bits::<U32, LANES>(count)?;
                let tile = lanes.add(start, along)?;
                let inside = lanes.greater_than(past, tile)?;
                (tile, lanes.select(inside, tile, past)?)
            };
            marks.push(kernel.load_at(bricks.tiles(), at.id())?);
            tiles.push(tile);
        }
        let mut chosen = Vec::with_capacity(run as usize);
        let mut serial = Vec::with_capacity(run as usize);
        let held = {
            let mut lanes = kernel.lanes()?;
            let mut running = lanes.splat_bits::<U32, LANES>(0)?;
            for (&mark, &tile) in marks.iter().zip(&tiles) {
                let mark = lanes.splat_id::<U32, LANES>(mark)?;
                let past = lanes.splat_bits::<U32, LANES>(count)?;
                let fresh = lanes.equal(mark, stamp)?;
                let fresh = ones(&mut lanes, fresh)?;
                let older = lanes.equal(mark, before)?;
                let older = ones(&mut lanes, older)?;
                let inside = lanes.greater_than(past, tile)?;
                let inside = ones(&mut lanes, inside)?;
                let either = lanes.max(fresh, older)?;
                let either = lanes.mul(either, inside)?;
                serial.push(running);
                running = lanes.add(running, either)?;
                chosen.push(either);
            }
            running
        };
        let (prior, total) = {
            let mut lanes = kernel.lanes()?;
            let prior = lanes.prefix_sum_exclusive::<U32, LANES>(held)?;
            let total = lanes.reduce_sum::<U32, LANES>(held)?;
            (prior, total)
        };
        for ((&tile, &either), &earlier) in tiles.iter().zip(&chosen).zip(&serial) {
            let place = {
                let mut lanes = kernel.lanes()?;
                let base = lanes.splat_id::<U32, LANES>(base)?;
                let within = lanes.add(prior, earlier)?;
                lanes.add(base, within)?
            };
            store_if::<LANES>(kernel, either, bricks.tile_list(), place.id(), tile.id())?;
        }
        Ok(vec![kernel.module().i_add(uint, base, total)?])
    })?;
    let total = found.first().copied().unwrap_or(head);
    let per = kernel.module().constant_u32(PER_LANE)?;
    let lanes_total = kernel.module().i_mul(uint, total, per)?;
    let one = kernel.module().constant_u32(1)?;
    let args = bricks.tile_args();
    for (at, value) in [
        (0, total),
        (TILE_GROUPS as u32, total),
        (TILE_GROUPS as u32 + 1, one),
        (TILE_GROUPS as u32 + 2, one),
        (LANE_GROUPS as u32, lanes_total),
        (LANE_GROUPS as u32 + 1, one),
        (LANE_GROUPS as u32 + 2, one),
    ] {
        let at = kernel.module().constant_u32(at)?;
        kernel.store_at(args, at, value)?;
    }
    kernel.finish()
}

pub fn quiet(bricks: &Bricks, plan: &Plan) -> Result<Vec<u32>, LaneError> {
    quiet_in(bricks, plan, Reach::Each)
}

fn quiet_in(bricks: &Bricks, plan: &Plan, reach: Reach) -> Result<Vec<u32>, LaneError> {
    let subgroup = bricks.subgroup;
    widths!(subgroup, quieted, [], (bricks, plan, reach))
}

fn quieted<const LANES: u32>(
    bricks: &Bricks,
    plan: &Plan,
    reach: Reach,
) -> Result<Vec<u32>, LaneError> {
    let subgroup = bricks.subgroup;
    let windows = bricks.windows();
    let table = bricks.table();
    let tier = bricks.tier();
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, bricks.bindings()))?;
    let brick = brick_lane::<LANES>(&mut kernel, bricks, reach)?;
    let head = kernel.module().constant_u32(0)?;
    let frame = kernel.load_at(PARAMS, head)?;

    let count = bricks.count();
    let (at, flag_at, inside) = {
        let mut lanes = kernel.lanes()?;
        let here = lanes.splat_id::<U32, LANES>(brick)?;
        let bricks_held = lanes.splat_bits::<U32, LANES>(count)?;
        let cold = lanes.splat_bits::<U32, LANES>(plan.padded())?;
        let four = lanes.splat_bits::<U32, LANES>(ENTRY as u32)?;
        let inside = lanes.greater_than(bricks_held, here)?;
        let clamped = lanes.select(inside, here, bricks_held)?;
        let at = lanes.mul(clamped, four)?;
        let flag_at = lanes.select(inside, here, cold)?;
        (at, flag_at, ones(&mut lanes, inside)?)
    };
    let entry = kernel.load_at(table, at.id())?;
    let touched_at = {
        let mut lanes = kernel.lanes()?;
        let by = lanes.splat_bits::<U32, LANES>(TOUCHED as u32)?;
        lanes.add(at, by)?
    };
    let touched = kernel.load_at(table, touched_at.id())?;
    let (candidate, start) = {
        let mut lanes = kernel.lanes()?;
        let entry = lanes.splat_id::<U32, LANES>(entry)?;
        let touched = lanes.splat_id::<U32, LANES>(touched)?;
        let now = lanes.splat_id::<U32, LANES>(frame)?;
        let uniform = lanes.splat_bits::<U32, LANES>(UNIFORM)?;
        let by = lanes.splat_bits::<U32, LANES>(bricks.cell_bits())?;
        let none = lanes.splat_bits::<U32, LANES>(0)?;
        let held = lanes.not_equal(entry, uniform)?;
        let held = ones(&mut lanes, held)?;
        let one = lanes.splat_bits::<U32, LANES>(1)?;
        let before = lanes.max(now, one)?;
        let stamp = lanes.add(now, one)?;
        let stamp = lanes.max(stamp, one)?;
        let last = lanes.equal(touched, before)?;
        let last = ones(&mut lanes, last)?;
        let fresh = lanes.not_equal(touched, stamp)?;
        let fresh = ones(&mut lanes, fresh)?;
        let quiet = lanes.mul(last, fresh)?;
        let candidate = lanes.mul(held, quiet)?;
        let candidate = lanes.mul(candidate, inside)?;
        let picked = lanes.greater_than(candidate, none)?;
        let start = lanes.shift_left(entry, by)?;
        let start = lanes.select(picked, start, none)?;
        (candidate, start)
    };
    let nothing = kernel.module().constant_u32(0)?;
    kernel.store_at(tier.flags, flag_at.id(), nothing)?;

    let words = u32::try_from(bricks.cells()).unwrap_or(u32::MAX);
    crate::gate::if_any::<LANES, _>(&mut kernel, candidate.id(), |kernel| {
        let uint = kernel.index_type();
        let first = live::state_load(kernel, windows, start.id())?;
        let zero = kernel.module().constant_u32(0)?;
        let differs =
            kernel.repeat_rolled_many(words - 1, uint, &[zero], |kernel, carried, along| {
                let one = kernel.module().constant_u32(1)?;
                let next = kernel.module().i_add(uint, start.id(), one)?;
                let at = kernel.module().i_add(uint, next, along)?;
                let word = live::state_load(kernel, windows, at)?;
                let sum = {
                    let mut lanes = kernel.lanes()?;
                    let word = lanes.splat_id::<U32, LANES>(word)?;
                    let first = lanes.splat_id::<U32, LANES>(first)?;
                    let so_far =
                        lanes.splat_id::<U32, LANES>(carried.first().copied().unwrap_or(zero))?;
                    let other = lanes.not_equal(word, first)?;
                    let other = ones(&mut lanes, other)?;
                    lanes.add(so_far, other)?
                };
                Ok(vec![sum.id()])
            })?;
        let mut differs = differs.first().copied().unwrap_or(zero);
        for plane in 1..bricks.planes() {
            let by = kernel.module().constant_u32(bricks.plane_start(plane))?;
            let lifted = kernel.module().i_add(uint, start.id(), by)?;
            differs = uneven::<LANES>(kernel, windows, lifted, words, differs)?;
        }
        let flag = {
            let mut lanes = kernel.lanes()?;
            let differs = lanes.splat_id::<U32, LANES>(differs)?;
            let none = lanes.splat_bits::<U32, LANES>(0)?;
            let same = lanes.equal(differs, none)?;
            let same = ones(&mut lanes, same)?;
            lanes.mul(same, candidate)?
        };
        kernel.store_at(tier.flags, flag_at.id(), flag.id())
    })?;
    kernel.finish()
}

fn uneven<const LANES: u32>(
    kernel: &mut Kernel<U32>,
    windows: Windows,
    start: Id,
    words: u32,
    so_far: Id,
) -> Result<Id, LaneError> {
    let uint = kernel.index_type();
    let first = live::state_load(kernel, windows, start)?;
    let differs =
        kernel.repeat_rolled_many(words - 1, uint, &[so_far], |kernel, carried, along| {
            let one = kernel.module().constant_u32(1)?;
            let next = kernel.module().i_add(uint, start, one)?;
            let at = kernel.module().i_add(uint, next, along)?;
            let word = live::state_load(kernel, windows, at)?;
            let sum = {
                let mut lanes = kernel.lanes()?;
                let word = lanes.splat_id::<U32, LANES>(word)?;
                let first = lanes.splat_id::<U32, LANES>(first)?;
                let carried =
                    lanes.splat_id::<U32, LANES>(carried.first().copied().unwrap_or(so_far))?;
                let other = lanes.not_equal(word, first)?;
                let other = ones(&mut lanes, other)?;
                lanes.add(carried, other)?
            };
            Ok(vec![sum.id()])
        })?;
    Ok(differs.first().copied().unwrap_or(so_far))
}

pub fn wants(bricks: &Bricks, plan: &Plan) -> Result<Vec<u32>, LaneError> {
    wants_in(bricks, plan, Reach::Each)
}

fn wants_in(bricks: &Bricks, plan: &Plan, reach: Reach) -> Result<Vec<u32>, LaneError> {
    let subgroup = bricks.subgroup;
    widths!(subgroup, wanting, [], (bricks, plan, reach))
}

fn wanting<const LANES: u32>(
    bricks: &Bricks,
    plan: &Plan,
    reach: Reach,
) -> Result<Vec<u32>, LaneError> {
    let subgroup = bricks.subgroup;
    let table = bricks.table();
    let tier = bricks.tier();
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, bricks.bindings()))?;
    let brick = brick_lane::<LANES>(&mut kernel, bricks, reach)?;
    let head = kernel.module().constant_u32(0)?;
    let frame = kernel.load_at(PARAMS, head)?;

    let (at, wanted_at, flag_at, inside) = {
        let mut lanes = kernel.lanes()?;
        let here = lanes.splat_id::<U32, LANES>(brick)?;
        let bricks_held = lanes.splat_bits::<U32, LANES>(bricks.count())?;
        let cold = lanes.splat_bits::<U32, LANES>(plan.padded())?;
        let four = lanes.splat_bits::<U32, LANES>(ENTRY as u32)?;
        let by = lanes.splat_bits::<U32, LANES>(WANTED as u32)?;
        let inside = lanes.greater_than(bricks_held, here)?;
        let clamped = lanes.select(inside, here, bricks_held)?;
        let at = lanes.mul(clamped, four)?;
        let wanted_at = lanes.add(at, by)?;
        let flag_at = lanes.select(inside, here, cold)?;
        (at, wanted_at, flag_at, ones(&mut lanes, inside)?)
    };
    let entry = kernel.load_at(table, at.id())?;
    let wanted = kernel.load_at(table, wanted_at.id())?;
    let flag = {
        let mut lanes = kernel.lanes()?;
        let entry = lanes.splat_id::<U32, LANES>(entry)?;
        let wanted = lanes.splat_id::<U32, LANES>(wanted)?;
        let now = lanes.splat_id::<U32, LANES>(frame)?;
        let one = lanes.splat_bits::<U32, LANES>(1)?;
        let uniform = lanes.splat_bits::<U32, LANES>(UNIFORM)?;
        let stamp = lanes.add(now, one)?;
        let stamp = lanes.max(stamp, one)?;
        let fresh = lanes.equal(wanted, stamp)?;
        let fresh = ones(&mut lanes, fresh)?;
        let empty = lanes.equal(entry, uniform)?;
        let empty = ones(&mut lanes, empty)?;
        let flag = lanes.mul(fresh, empty)?;
        lanes.mul(flag, inside)?
    };
    kernel.store_at(tier.flags, flag_at.id(), flag.id())?;
    kernel.finish()
}

pub fn restamp(bricks: &Bricks) -> Result<Vec<u32>, LaneError> {
    let subgroup = bricks.subgroup;
    widths!(subgroup, restamped, [], (bricks))
}

fn restamped<const LANES: u32>(bricks: &Bricks) -> Result<Vec<u32>, LaneError> {
    let subgroup = bricks.subgroup;
    let table = bricks.table();
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, bricks.bindings()))?;
    let brick = per_brick(&mut kernel, subgroup)?;
    let head = kernel.module().constant_u32(0)?;
    let frame = kernel.load_at(PARAMS, head)?;
    let (touched_at, inside) = {
        let mut lanes = kernel.lanes()?;
        let here = lanes.splat_id::<U32, LANES>(brick)?;
        let bricks_held = lanes.splat_bits::<U32, LANES>(bricks.count())?;
        let four = lanes.splat_bits::<U32, LANES>(ENTRY as u32)?;
        let by = lanes.splat_bits::<U32, LANES>(TOUCHED as u32)?;
        let inside = lanes.greater_than(bricks_held, here)?;
        let clamped = lanes.select(inside, here, bricks_held)?;
        let at = lanes.mul(clamped, four)?;
        (lanes.add(at, by)?, ones(&mut lanes, inside)?)
    };
    let touched = kernel.load_at(table, touched_at.id())?;
    let (when, earlier) = {
        let mut lanes = kernel.lanes()?;
        let touched = lanes.splat_id::<U32, LANES>(touched)?;
        let now = lanes.splat_id::<U32, LANES>(frame)?;
        let one = lanes.splat_bits::<U32, LANES>(1)?;
        let back = lanes.splat_bits::<U32, LANES>(u32::MAX)?;
        let stamp = lanes.add(now, one)?;
        let stamp = lanes.max(stamp, one)?;
        let before = lanes.add(now, back)?;
        let earlier = lanes.max(before, one)?;
        let fresh = lanes.equal(touched, stamp)?;
        let fresh = ones(&mut lanes, fresh)?;
        (lanes.mul(fresh, inside)?, earlier)
    };
    store_if::<LANES>(&mut kernel, when, table, touched_at.id(), earlier.id())?;
    let (tile_at, tile_inside) = {
        let mut lanes = kernel.lanes()?;
        let here = lanes.splat_id::<U32, LANES>(brick)?;
        let count = lanes.splat_bits::<U32, LANES>(bricks.tile_count())?;
        let inside = lanes.greater_than(count, here)?;
        (
            lanes.select(inside, here, count)?,
            ones(&mut lanes, inside)?,
        )
    };
    let mark = kernel.load_at(bricks.tiles(), tile_at.id())?;
    let lit = {
        let mut lanes = kernel.lanes()?;
        let mark = lanes.splat_id::<U32, LANES>(mark)?;
        let now = lanes.splat_id::<U32, LANES>(frame)?;
        let one = lanes.splat_bits::<U32, LANES>(1)?;
        let stamp = lanes.add(now, one)?;
        let stamp = lanes.max(stamp, one)?;
        let fresh = lanes.equal(mark, stamp)?;
        let fresh = ones(&mut lanes, fresh)?;
        lanes.mul(fresh, tile_inside)?
    };
    store_if::<LANES>(&mut kernel, lit, bricks.tiles(), tile_at.id(), earlier.id())?;
    kernel.finish()
}

pub fn release(bricks: &Bricks) -> Result<Vec<u32>, LaneError> {
    let subgroup = bricks.subgroup;
    widths!(subgroup, released, [], (bricks))
}

fn released<const LANES: u32>(bricks: &Bricks) -> Result<Vec<u32>, LaneError> {
    let subgroup = bricks.subgroup;
    let windows = bricks.windows();
    let table = bricks.table();
    let slots = bricks.slots();
    let tier = bricks.tier();
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, bricks.bindings()))?;
    let rank = per_brick(&mut kernel, subgroup)?;
    let head = kernel.module().constant_u32(0)?;
    let grand = kernel.load_at(tier.grand, head)?;
    let top = kernel.load_at(slots, head)?;
    let listed = kernel.load_at(tier.dense, rank)?;

    let spare = u32::try_from(bricks.count() as usize * ENTRY).unwrap_or(u32::MAX);
    let (at, inside, brick) = {
        let mut lanes = kernel.lanes()?;
        let here = lanes.splat_id::<U32, LANES>(rank)?;
        let count = lanes.splat_id::<U32, LANES>(grand)?;
        let listed = lanes.splat_id::<U32, LANES>(listed)?;
        let bricks_held = lanes.splat_bits::<U32, LANES>(bricks.count())?;
        let four = lanes.splat_bits::<U32, LANES>(ENTRY as u32)?;
        let inside = lanes.greater_than(count, here)?;
        let brick = lanes.select(inside, listed, bricks_held)?;
        (lanes.mul(brick, four)?, ones(&mut lanes, inside)?, brick)
    };
    let entry = kernel.load_at(table, at.id())?;
    let start = {
        let mut lanes = kernel.lanes()?;
        let entry = lanes.splat_id::<U32, LANES>(entry)?;
        let by = lanes.splat_bits::<U32, LANES>(bricks.cell_bits())?;
        let none = lanes.splat_bits::<U32, LANES>(0)?;
        let picked = lanes.greater_than(inside, none)?;
        let start = lanes.shift_left(entry, by)?;
        lanes.select(picked, start, none)?
    };
    let value = live::state_load(&mut kernel, windows, start.id())?;
    let (push, entry_at, value_at) = {
        let mut lanes = kernel.lanes()?;
        let here = lanes.splat_id::<U32, LANES>(rank)?;
        let top = lanes.splat_id::<U32, LANES>(top)?;
        let none = lanes.splat_bits::<U32, LANES>(0)?;
        let one = lanes.splat_bits::<U32, LANES>(1)?;
        let header = lanes.splat_bits::<U32, LANES>(HEAD as u32)?;
        let scratch = lanes.splat_bits::<U32, LANES>(SCRATCH as u32)?;
        let spare_entry = lanes.splat_bits::<U32, LANES>(spare + 2)?;
        let spare_value = lanes.splat_bits::<U32, LANES>(spare + 3)?;
        let picked = lanes.greater_than(inside, none)?;
        let push = lanes.add(header, top)?;
        let push = lanes.add(push, here)?;
        let push = lanes.select(picked, push, scratch)?;
        let entry_at = lanes.select(picked, at, spare_entry)?;
        let value_at = lanes.add(at, one)?;
        let value_at = lanes.select(picked, value_at, spare_value)?;
        (push, entry_at, value_at)
    };
    let uniform = kernel.module().constant_u32(UNIFORM)?;
    kernel.store_at(slots, push.id(), entry)?;
    kernel.store_at(table, entry_at.id(), uniform)?;
    kernel.store_at(table, value_at.id(), value)?;
    for plane in 1..bricks.planes() {
        let lifted = {
            let mut lanes = kernel.lanes()?;
            let by = lanes.splat_bits::<U32, LANES>(bricks.plane_start(plane))?;
            lanes.add(start, by)?
        };
        let value = live::state_load(&mut kernel, windows, lifted.id())?;
        let value_at = {
            let mut lanes = kernel.lanes()?;
            let base = lanes.splat_bits::<U32, LANES>(bricks.plane_values(plane))?;
            lanes.add(brick, base)?
        };
        kernel.store_at(table, value_at.id(), value)?;
    }
    kernel.finish()
}

pub fn lift(bricks: &Bricks) -> Result<Vec<u32>, LaneError> {
    moved(bricks, true)
}

pub fn drop_taken(bricks: &Bricks) -> Result<Vec<u32>, LaneError> {
    moved(bricks, false)
}

fn moved(bricks: &Bricks, up: bool) -> Result<Vec<u32>, LaneError> {
    use simdr::module::op;

    let slots = bricks.slots();
    let tier = bricks.tier();
    let mut kernel = Kernel::<U32>::new(Shape::new(bricks.subgroup, 1, bricks.bindings()))?;
    let uint = kernel.index_type();
    let head = kernel.module().constant_u32(0)?;
    let top = kernel.load_at(slots, head)?;
    let count = kernel.load_at(tier.grand, head)?;
    let next = if up {
        kernel.module().i_add(uint, top, count)?
    } else {
        let truth = kernel.module().type_bool()?;
        let short = kernel.module().binary(op::U_LESS_THAN, truth, top, count)?;
        let taken = kernel.module().select(uint, short, top, count)?;
        kernel.module().binary(op::I_SUB, uint, top, taken)?
    };
    kernel.store_at(slots, head, next)?;
    kernel.finish()
}

pub fn take(bricks: &Bricks) -> Result<Vec<u32>, LaneError> {
    let subgroup = bricks.subgroup;
    widths!(subgroup, taken, [], (bricks))
}

fn taken<const LANES: u32>(bricks: &Bricks) -> Result<Vec<u32>, LaneError> {
    use simdr::module::op;

    let subgroup = bricks.subgroup;
    let tier = bricks.tier();
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, bricks.bindings()))?;
    let uint = kernel.index_type();
    let group = kernel.workgroup_index();
    let head = kernel.module().constant_u32(0)?;
    let grand = kernel.load_at(tier.grand, head)?;
    let per = bricks.takes();
    let span = kernel.module().constant_u32(per)?;
    let first = kernel.module().i_mul(uint, group, span)?;
    let _ = kernel.repeat_rolled_many(per, uint, &[first], |kernel, carried, _| {
        let rank = carried.first().copied().unwrap_or(first);
        let truth = kernel.module().type_bool()?;
        let listed = kernel
            .module()
            .binary(op::U_LESS_THAN, truth, rank, grand)?;
        live::lane_if(kernel, listed, |kernel| {
            take_one::<LANES>(kernel, bricks, rank)
        })?;
        let one = kernel.module().constant_u32(1)?;
        Ok(vec![kernel.module().i_add(uint, rank, one)?])
    })?;
    kernel.finish()
}

fn take_one<const LANES: u32>(
    kernel: &mut Kernel<U32>,
    bricks: &Bricks,
    rank: Id,
) -> Result<(), LaneError> {
    let subgroup = bricks.subgroup;
    let windows = bricks.windows();
    let table = bricks.table();
    let slots = bricks.slots();
    let tier = bricks.tier();
    let uint = kernel.index_type();
    let lane = kernel.local_index();
    let head = kernel.module().constant_u32(0)?;
    let top = kernel.load_at(slots, head)?;
    let brick = kernel.load_at(tier.dense, rank)?;

    let (pop, ok, value_at, entry_at) = {
        let mut lanes = kernel.lanes()?;
        let here = lanes.splat_id::<U32, LANES>(rank)?;
        let top = lanes.splat_id::<U32, LANES>(top)?;
        let brick = lanes.splat_id::<U32, LANES>(brick)?;
        let one = lanes.splat_bits::<U32, LANES>(1)?;
        let four = lanes.splat_bits::<U32, LANES>(ENTRY as u32)?;
        let header = lanes.splat_bits::<U32, LANES>(HEAD as u32 - 1)?;
        let scratch = lanes.splat_bits::<U32, LANES>(SCRATCH as u32)?;
        let ok = lanes.greater_than(top, here)?;
        let pop = lanes.add(header, top)?;
        let pop = lanes.sub(pop, here)?;
        let pop = lanes.select(ok, pop, scratch)?;
        let entry_at = lanes.mul(brick, four)?;
        let value_at = lanes.add(entry_at, one)?;
        (pop, ones(&mut lanes, ok)?, value_at, entry_at)
    };
    let slot = kernel.load_at(slots, pop.id())?;
    let value = kernel.load_at(table, value_at.id())?;
    let mut plane_values = Vec::new();
    for plane in 1..bricks.planes() {
        let base = kernel.module().constant_u32(bricks.plane_values(plane))?;
        let at = kernel.module().i_add(uint, brick, base)?;
        plane_values.push((plane, kernel.load_at(table, at)?));
    }
    let level = kernel.load_at(crate::layout::HOT, brick)?;
    let per_lane = u32::try_from(bricks.cells()).unwrap_or(u32::MAX) / subgroup;
    let flags_per_lane = bricks.per_block() / subgroup;
    let (start, flags_from) = {
        let mut lanes = kernel.lanes()?;
        let slot = lanes.splat_id::<U32, LANES>(slot)?;
        let seat = lanes.splat_id::<U32, LANES>(lane)?;
        let by = lanes.splat_bits::<U32, LANES>(bricks.cell_bits())?;
        let run = lanes.splat_bits::<U32, LANES>(per_lane)?;
        let first = lanes.shift_left(slot, by)?;
        let along = lanes.mul(seat, run)?;
        let by = lanes.splat_bits::<U32, LANES>(bricks.octs_bits())?;
        let flags_run = lanes.splat_bits::<U32, LANES>(flags_per_lane)?;
        let flags_first = lanes.shift_left(slot, by)?;
        let flags_along = lanes.mul(seat, flags_run)?;
        (
            lanes.add(first, along)?,
            lanes.add(flags_first, flags_along)?,
        )
    };
    let taking = {
        let mut lanes = kernel.lanes()?;
        let none = lanes.splat_bits::<U32, LANES>(0)?;
        let taking = lanes.greater_than(ok, none)?;
        taking
            .strips()
            .first()
            .copied()
            .ok_or(LaneError::NoOpenBlock { arm: "taking" })?
    };
    live::lane_if(kernel, taking, |kernel| {
        let _ =
            kernel.repeat_rolled_many(per_lane, uint, &[start.id()], |kernel, carried, _| {
                let at = carried.first().copied().unwrap_or(head);
                live::state_store(kernel, windows, at, value)?;
                let one = kernel.module().constant_u32(1)?;
                Ok(vec![kernel.module().i_add(uint, at, one)?])
            })?;
        for &(plane, value) in &plane_values {
            let by = kernel.module().constant_u32(bricks.plane_start(plane))?;
            let lifted = kernel.module().i_add(uint, start.id(), by)?;
            let _ =
                kernel.repeat_rolled_many(per_lane, uint, &[lifted], |kernel, carried, _| {
                    let at = carried.first().copied().unwrap_or(head);
                    live::state_store(kernel, windows, at, value)?;
                    let one = kernel.module().constant_u32(1)?;
                    Ok(vec![kernel.module().i_add(uint, at, one)?])
                })?;
        }
        let _ = kernel.repeat_rolled_many(
            flags_per_lane,
            uint,
            &[flags_from.id()],
            |kernel, carried, _| {
                let at = carried.first().copied().unwrap_or(head);
                kernel.store_at(crate::layout::AWAKE, at, level)?;
                let one = kernel.module().constant_u32(1)?;
                Ok(vec![kernel.module().i_add(uint, at, one)?])
            },
        )?;
        kernel.store_at(table, entry_at.id(), slot)
    })?;
    let over_at = {
        let mut lanes = kernel.lanes()?;
        let none = lanes.splat_bits::<U32, LANES>(0)?;
        let flagged = lanes.splat_bits::<U32, LANES>(OVERFLOW as u32)?;
        let scratch = lanes.splat_bits::<U32, LANES>(SCRATCH as u32)?;
        let fine = lanes.greater_than(ok, none)?;
        lanes.select(fine, scratch, flagged)?
    };
    let one = kernel.module().constant_u32(1)?;
    kernel.store_at(slots, over_at.id(), one)
}

fn held_or_not(kernel: &mut Kernel<U32>, entry: Id) -> Result<(Id, Id), LaneError> {
    use simdr::module::op;

    let truth = kernel.module().type_bool()?;
    let uniform = kernel.module().constant_u32(UNIFORM)?;
    let held = kernel
        .module()
        .binary(op::I_NOT_EQUAL, truth, entry, uniform)?;
    let not_held = kernel.module().binary(op::I_EQUAL, truth, entry, uniform)?;
    Ok((held, not_held))
}

fn lowered<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    was: Vector<U32, LANES>,
) -> Result<Vector<U32, LANES>, LaneError> {
    let one = lanes.splat_bits::<U32, LANES>(1)?;
    let floored = lanes.max(was, one)?;
    lanes.sub(floored, one)
}

pub fn decay(bricks: &Bricks, over: Over) -> Result<Vec<u32>, LaneError> {
    let subgroup = bricks.subgroup;
    widths!(subgroup, decayed, [], (bricks, over))
}

fn decayed<const LANES: u32>(bricks: &Bricks, over: Over) -> Result<Vec<u32>, LaneError> {
    use crate::layout::{AWAKE, HOT};
    use crate::sweep::PER_LANE;

    let subgroup = bricks.subgroup;
    let table = bricks.table();
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, bricks.bindings()))?;
    let uint = kernel.index_type();
    let block = crate::compact::owned(&mut kernel, over)?;
    let lane = kernel.local_index();
    let four = kernel.module().constant_u32(ENTRY as u32)?;
    let at = kernel.module().i_mul(uint, block, four)?;
    let entry = kernel.load_at(table, at)?;
    let (held, not_held) = held_or_not(&mut kernel, entry)?;
    let nothing = kernel.module().constant_u32(0)?;
    let truth = kernel.module().type_bool()?;
    let leader = kernel
        .module()
        .binary(simdr::module::op::I_EQUAL, truth, lane, nothing)?;
    live::lane_if(&mut kernel, leader, |kernel| {
        kernel.store_at(crate::layout::COUNTS, block, nothing)
    })?;

    live::lane_if(&mut kernel, held, |kernel| {
        let uint = kernel.index_type();
        let span = kernel.module().constant_u32(bricks.per_block())?;
        let start = kernel.module().i_mul(uint, entry, span)?;
        let run = kernel.module().constant_u32(PER_LANE)?;
        let along = kernel.module().i_mul(uint, lane, run)?;
        let mine = kernel.module().i_add(uint, start, along)?;
        let mut places = Vec::with_capacity(PER_LANE as usize);
        let mut was = Vec::with_capacity(PER_LANE as usize);
        for offset in 0..PER_LANE {
            let along = kernel.module().constant_u32(offset)?;
            let place = kernel.module().i_add(uint, mine, along)?;
            was.push(kernel.load_at(AWAKE, place)?);
            places.push(place);
        }
        let mut highest = kernel.module().constant_u32(0)?;
        for (&place, &held) in places.iter().zip(&was) {
            let (lower, top) = {
                let mut lanes = kernel.lanes()?;
                let held = lanes.splat_id::<U32, 1>(held)?;
                let lower = lowered(&mut lanes, held)?;
                let so_far = lanes.splat_id::<U32, 1>(highest)?;
                (lower, lanes.max(so_far, lower)?)
            };
            kernel.store_at(AWAKE, place, lower.id())?;
            highest = top.id();
        }
        let flag = {
            let mut lanes = kernel.lanes()?;
            let mine = lanes.splat_id::<U32, LANES>(highest)?;
            lanes.reduce_max::<U32, LANES>(mine)?
        };
        kernel.store_at(HOT, block, flag)
    })?;
    live::lane_if(&mut kernel, not_held, |kernel| {
        let level = kernel.load_at(HOT, block)?;
        let lower = {
            let mut lanes = kernel.lanes()?;
            let level = lanes.splat_id::<U32, 1>(level)?;
            lowered(&mut lanes, level)?
        };
        kernel.store_at(HOT, block, lower.id())
    })?;
    kernel.finish()
}

pub fn mark(bricks: &Bricks, over: Over) -> Result<Vec<u32>, LaneError> {
    let subgroup = bricks.subgroup;
    widths!(subgroup, marked, [], (bricks, over))
}

fn marked<const LANES: u32>(bricks: &Bricks, over: Over) -> Result<Vec<u32>, LaneError> {
    use crate::layout::AWAKE;
    use crate::sweep::{self, PER_LANE};

    let subgroup = bricks.subgroup;
    let table = bricks.table();
    let fine = Tier::FINE;
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, bricks.bindings()))?;
    let uint = kernel.index_type();
    let block = crate::compact::owned(&mut kernel, over)?;
    let lane = kernel.local_index();
    let four = kernel.module().constant_u32(ENTRY as u32)?;
    let at = kernel.module().i_mul(uint, block, four)?;
    let entry = kernel.load_at(table, at)?;
    let (held, _) = held_or_not(&mut kernel, entry)?;

    live::lane_if(&mut kernel, held, |kernel| {
        let uint = kernel.index_type();
        let span = kernel.module().constant_u32(bricks.per_block())?;
        let run = kernel.module().constant_u32(PER_LANE)?;
        let along = kernel.module().i_mul(uint, lane, run)?;
        let start = kernel.module().i_mul(uint, entry, span)?;
        let mine = kernel.module().i_add(uint, start, along)?;
        let first = kernel.module().i_mul(uint, block, span)?;
        let named = kernel.module().i_add(uint, first, along)?;
        let wide = kernel.module().constant_u32(sweep::stride(subgroup))?;
        let region = kernel.module().i_mul(uint, entry, wide)?;
        let dump_start = kernel.module().i_add(uint, region, span)?;
        let dump_mine = kernel.module().i_add(uint, dump_start, lane)?;

        let mut octs = Vec::with_capacity(PER_LANE as usize);
        let mut flags = Vec::with_capacity(PER_LANE as usize);
        for step in 0..PER_LANE {
            let along = kernel.module().constant_u32(step)?;
            let place = kernel.module().i_add(uint, mine, along)?;
            flags.push(kernel.load_at(AWAKE, place)?);
            octs.push(kernel.module().i_add(uint, named, along)?);
        }
        let mut serial = Vec::with_capacity(PER_LANE as usize);
        let carried = {
            let mut lanes = kernel.lanes()?;
            let mut running = lanes.splat_bits::<U32, LANES>(0)?;
            for &flag in &flags {
                let held = lanes.splat_id::<U32, LANES>(flag)?;
                let none = lanes.splat_bits::<U32, LANES>(0)?;
                let listed = lanes.greater_than(held, none)?;
                let counted = ones(&mut lanes, listed)?;
                serial.push(running);
                running = lanes.add(running, counted)?;
            }
            running
        };
        let (before, total) = {
            let mut lanes = kernel.lanes()?;
            let before = lanes.prefix_sum_exclusive::<U32, LANES>(carried)?;
            let total = lanes.reduce_sum::<U32, LANES>(carried)?;
            (before, total)
        };
        kernel.store_at(fine.counts, block, total)?;
        for ((oct, &flag), earlier) in octs.iter().zip(&flags).zip(serial) {
            let place = {
                let mut lanes = kernel.lanes()?;
                let here = lanes.splat_id::<U32, LANES>(region)?;
                let elsewhere = lanes.splat_id::<U32, LANES>(dump_mine)?;
                let none = lanes.splat_bits::<U32, LANES>(0)?;
                let awake = lanes.splat_id::<U32, LANES>(flag)?;
                let within = lanes.add(before, earlier)?;
                let bucket = lanes.add(here, within)?;
                let listed = lanes.greater_than(awake, none)?;
                lanes.select(listed, bucket, elsewhere)?
            };
            kernel.store_at(fine.buckets, place.id(), *oct)?;
        }
        Ok(())
    })?;
    kernel.finish()
}

pub fn wholes(bricks: &Bricks) -> Result<Vec<u32>, LaneError> {
    wholes_in(bricks, Reach::Each)
}

fn wholes_in(bricks: &Bricks, reach: Reach) -> Result<Vec<u32>, LaneError> {
    let subgroup = bricks.subgroup;
    widths!(subgroup, wholed, [], (bricks, reach))
}

fn wholed<const LANES: u32>(bricks: &Bricks, reach: Reach) -> Result<Vec<u32>, LaneError> {
    use crate::layout::HOT;

    let subgroup = bricks.subgroup;
    let table = bricks.table();
    let tier = bricks.tier();
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, bricks.bindings()))?;
    let brick = brick_lane::<LANES>(&mut kernel, bricks, reach)?;
    let (at, hot_at) = {
        let mut lanes = kernel.lanes()?;
        let here = lanes.splat_id::<U32, LANES>(brick)?;
        let count = lanes.splat_bits::<U32, LANES>(bricks.count())?;
        let cold = lanes.splat_bits::<U32, LANES>(bricks.cold)?;
        let four = lanes.splat_bits::<U32, LANES>(ENTRY as u32)?;
        let inside = lanes.greater_than(count, here)?;
        let clamped = lanes.select(inside, here, count)?;
        let hot_at = lanes.select(inside, here, cold)?;
        (lanes.mul(clamped, four)?, hot_at)
    };
    let entry = kernel.load_at(table, at.id())?;
    let level = kernel.load_at(HOT, hot_at.id())?;
    let flag = {
        let mut lanes = kernel.lanes()?;
        let entry = lanes.splat_id::<U32, LANES>(entry)?;
        let level = lanes.splat_id::<U32, LANES>(level)?;
        let uniform = lanes.splat_bits::<U32, LANES>(UNIFORM)?;
        let none = lanes.splat_bits::<U32, LANES>(0)?;
        let empty = lanes.equal(entry, uniform)?;
        let empty = ones(&mut lanes, empty)?;
        let awake = lanes.greater_than(level, none)?;
        let awake = ones(&mut lanes, awake)?;
        lanes.mul(empty, awake)?
    };
    kernel.store_at(tier.flags, hot_at.id(), flag.id())?;
    kernel.finish()
}

pub fn listing(bricks: &Bricks) -> Result<Vec<u32>, LaneError> {
    use crate::layout::INDIRECT;
    use simdr::module::op;

    let subgroup = bricks.subgroup;
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, 1, bricks.bindings()))?;
    let uint = kernel.index_type();
    let head = kernel.module().constant_u32(0)?;
    let grand = kernel.load_at(GRAND, head)?;
    let whole = kernel.load_at(bricks.whole_count(), head)?;
    let short = kernel.module().constant_u32(subgroup - 1)?;
    let by = kernel.module().constant_u32(subgroup.trailing_zeros())?;
    let per = kernel
        .module()
        .constant_u32(bricks.per_block() / subgroup)?;
    let rounded = kernel.module().i_add(uint, grand, short)?;
    let groups = kernel
        .module()
        .binary(op::SHIFT_RIGHT_LOGICAL, uint, rounded, by)?;
    let more = kernel.module().i_mul(uint, whole, per)?;
    let total = kernel.module().i_add(uint, groups, more)?;
    let one = kernel.module().constant_u32(1)?;
    let two = kernel.module().constant_u32(2)?;
    kernel.store_at(INDIRECT, head, total)?;
    kernel.store_at(INDIRECT, one, one)?;
    kernel.store_at(INDIRECT, two, one)?;
    kernel.finish()
}

fn brick_scans(subgroup: u32, plan: &Plan, tier: Tier) -> Result<Stages, LaneError> {
    let one = Groups::Fixed(1);
    Ok(if plan.hot_tiles <= 1 {
        vec![vec![(
            crate::compact::whole(subgroup, tier.counts, tier.bases, tier)?,
            one,
        )]]
    } else {
        vec![
            vec![(
                crate::compact::scan(subgroup, tier.counts, tier.bases, tier.totals)?,
                Groups::Fixed(plan.hot_tiles),
            )],
            vec![(
                crate::compact::scan(subgroup, tier.totals, tier.totals, tier.grand)?,
                one,
            )],
        ]
    })
}

fn wipe_brick_totals(
    subgroup: u32,
    plan: &Plan,
    tier: Tier,
) -> Result<Option<(Vec<u32>, Groups)>, LaneError> {
    if plan.hot_tiles <= 1 {
        return Ok(None);
    }
    Ok(Some((
        live::wipe_counts(subgroup, tier.totals)?,
        Groups::Fixed(1),
    )))
}

fn tier_over(bricks: &Bricks, plan: &Plan, reach: Reach) -> (Over, Groups) {
    match reach {
        Reach::Each => (Over::Every, Groups::Fixed(plan.tiles)),
        Reach::Tiled => (Over::Listed(bricks.tile_list()), bricks.tile_groups()),
    }
}

fn ranked(bricks: &Bricks, plan: &Plan, per_group: u32, reach: Reach) -> Result<Stages, LaneError> {
    ranked_in(bricks, plan, per_group, (bricks.tier(), reach))
}

fn ranked_in(
    bricks: &Bricks,
    plan: &Plan,
    per_group: u32,
    (tier, reach): (Tier, Reach),
) -> Result<Stages, LaneError> {
    let subgroup = bricks.subgroup;
    let (over, tiles) = tier_over(bricks, plan, reach);
    let mut stages = vec![vec![(
        live::mark_below(subgroup, tier, over, bricks.count())?,
        tiles,
    )]];
    stages.extend(brick_scans(subgroup, plan, tier)?);
    stages.push(vec![
        (
            crate::compact::gather(subgroup, tier, over, plan.padded())?,
            tiles,
        ),
        (
            crate::compact::prepare(subgroup, per_group, tier)?,
            Groups::Counting(1),
        ),
    ]);
    Ok(stages)
}

fn pooled(bricks: &Bricks, plan: &Plan, reach: Reach) -> Result<Stages, LaneError> {
    let subgroup = bricks.subgroup;
    let tier = bricks.tier();
    let flags = match reach {
        Reach::Each => Groups::Fixed(bricks.count().div_ceil(subgroup)),
        Reach::Tiled => bricks.lane_groups(),
    };
    let listed = Groups::From {
        binding: tier.indirect as usize,
        at: 0,
    };
    let one = Groups::Fixed(1);
    let mut stages = vec![vec![(quiet_in(bricks, plan, reach)?, flags)]];
    stages.extend(ranked(bricks, plan, subgroup, reach)?);
    let mut freeing = vec![(release(bricks)?, listed)];
    freeing.extend(wipe_brick_totals(subgroup, plan, tier)?);
    stages.push(freeing);
    stages.push(vec![
        (lift(bricks)?, one),
        (wants_in(bricks, plan, reach)?, flags),
    ]);
    stages.extend(ranked(bricks, plan, bricks.takes(), reach)?);
    stages.push(vec![(take(bricks)?, listed)]);
    Ok(stages)
}

fn fits(operation: &'static str, bricks: &Bricks, plan: &Plan) -> Result<(), LaneError> {
    live::fits_pool(operation, bricks.grid, plan)
}

fn opening<R: Rule>(
    rule: &R,
    bricks: &Bricks,
    plan: &Plan,
    coarse: bool,
) -> Result<Vec<(Vec<u32>, Groups)>, LaneError> {
    use crate::layout::HOT_COUNTS;
    use crate::sweep::COUNTS;

    let subgroup = bricks.subgroup;
    let (listed, hot) = live::tiers(plan, coarse);
    let tiles = Groups::Fixed(plan.tiles);
    let tier_tiles = Groups::Fixed(plan.hot_tiles);
    let one = Groups::Fixed(1);
    let mut first = vec![
        (live::tick(subgroup)?, one),
        (decay(bricks, listed)?, hot),
        (live::wipe3(subgroup, rule.tally().words())?, one),
    ];
    if coarse {
        first.push((
            live::wipe_counts(subgroup, bricks.tier().counts)?,
            tier_tiles,
        ));
        first.push((live::wipe_counts(subgroup, HOT_COUNTS)?, tier_tiles));
    } else {
        first.push((live::wipe_counts(subgroup, COUNTS)?, tiles));
    }
    first.extend(live::totals_wiped(subgroup, plan)?);
    first.extend(wipe_brick_totals(subgroup, plan, bricks.tier())?);
    Ok(first)
}

fn fine_scans(bricks: &Bricks, plan: &Plan, coarse: bool) -> Result<Stages, LaneError> {
    use crate::compact::Along;
    use crate::layout::{BASES, COUNTS, TOTALS};

    let subgroup = bricks.subgroup;
    if !coarse || plan.levels() == 1 {
        return live::fine_scans(subgroup, plan);
    }
    let one = Groups::Fixed(1);
    let tiles = bricks.tile_groups();
    let opened = vec![(
        crate::compact::scan_over(
            subgroup,
            COUNTS,
            BASES,
            TOTALS,
            Over::Listed(bricks.tile_list()),
        )?,
        tiles,
    )];
    Ok(if plan.levels() == 2 {
        vec![
            opened,
            vec![(crate::compact::scan(subgroup, TOTALS, TOTALS, GRAND)?, one)],
        ]
    } else {
        vec![
            opened,
            vec![(
                crate::compact::scan_at(
                    subgroup,
                    TOTALS,
                    TOTALS,
                    TOTALS,
                    Along {
                        from: 0,
                        aggregate: plan.upper(),
                    },
                )?,
                Groups::Fixed(plan.supers()),
            )],
            vec![(
                crate::compact::scan_at(
                    subgroup,
                    TOTALS,
                    TOTALS,
                    GRAND,
                    Along {
                        from: plan.upper(),
                        aggregate: 0,
                    },
                )?,
                one,
            )],
        ]
    })
}

fn closing(bricks: &Bricks, plan: &Plan, coarse: bool) -> Result<Stages, LaneError> {
    let subgroup = bricks.subgroup;
    let (listed, hot) = live::tiers(plan, coarse);
    let reach = if coarse { Reach::Tiled } else { Reach::Each };
    let (over, tiles) = tier_over(bricks, plan, reach);
    let one = Groups::Fixed(1);
    let mut chain: Stages = Vec::new();
    if coarse {
        chain.push(vec![(
            live::mark_below(subgroup, Tier::COARSE, over, plan.blocks)?,
            tiles,
        )]);
        chain.extend(live::coarse_scans(subgroup, plan)?);
        chain.push(vec![
            (
                crate::compact::gather(subgroup, Tier::COARSE, over, plan.hot_dump())?,
                tiles,
            ),
            (
                crate::compact::prepare(subgroup, 1, Tier::COARSE)?,
                Groups::Counting(1),
            ),
        ]);
    }
    let mut marking = vec![(mark(bricks, listed)?, hot)];
    if coarse {
        marking.push((lighting(bricks)?, tiles));
    }
    chain.push(marking);
    chain.extend(fine_scans(bricks, plan, coarse)?);
    let held = crate::compact::Held {
        table: bricks.table(),
        entry: ENTRY as u32,
        reach: bricks.bindings(),
        dump: bricks.dump(),
        upper: (plan.levels() > 2).then(|| plan.upper()),
    };
    let mut gathering = vec![(
        crate::compact::gather_held(subgroup, Tier::FINE, listed, held)?,
        hot,
    )];
    if coarse {
        gathering.push((tiling(bricks)?, one));
    }
    chain.push(gathering);

    let whole = bricks.whole_tier();
    let flags = match reach {
        Reach::Each => Groups::Fixed(bricks.count().div_ceil(subgroup)),
        Reach::Tiled => bricks.lane_groups(),
    };
    let mut opened = vec![(wholes_in(bricks, reach)?, flags)];
    opened.extend(wipe_brick_totals(subgroup, plan, bricks.tier())?);
    let mut beside: Stages = vec![
        opened,
        vec![(
            live::mark_below(subgroup, whole, over, bricks.count())?,
            tiles,
        )],
    ];
    beside.extend(brick_scans(subgroup, plan, whole)?);
    beside.push(vec![(
        crate::compact::gather(subgroup, whole, over, plan.padded())?,
        tiles,
    )]);

    let deepest = chain.len().max(beside.len());
    let mut chain = chain.into_iter();
    let mut beside = beside.into_iter();
    let mut stages: Stages = Vec::with_capacity(deepest + 1);
    for _ in 0..deepest {
        let mut stage = chain.next().unwrap_or_default();
        stage.extend(beside.next().unwrap_or_default());
        stages.push(stage);
    }
    stages.push(vec![(listing(bricks)?, one)]);
    Ok(stages)
}

pub fn arranged<R: Rule>(
    rule: &R,
    bricks: &Bricks,
    plan: &Plan,
    coarse: bool,
) -> Result<Loop, LaneError> {
    fits("bricks::arranged", bricks, plan)?;
    let reach = if coarse { Reach::Tiled } else { Reach::Each };

    let mut stages: Stages = vec![
        opening(rule, bricks, plan, coarse)?,
        vec![(
            settle(rule, bricks, Act::Dry, Listing::Listed)?,
            Plan::listed(),
        )],
    ];
    stages.extend(pooled(bricks, plan, reach)?);
    stages.push(vec![
        (drop_taken(bricks)?, Groups::Fixed(1)),
        (
            settle(rule, bricks, Act::Settle, Listing::Listed)?,
            Plan::listed(),
        ),
    ]);
    stages.extend(closing(bricks, plan, coarse)?);
    Ok(live::staged(stages))
}

fn placed(
    blocks: &mut std::collections::BTreeMap<usize, Vec<u32>>,
    singles: &mut std::collections::BTreeMap<usize, u32>,
    at: usize,
    word: u32,
) {
    if let Some((&start, block)) = blocks.range_mut(..=at).next_back()
        && let Some(place) = block.get_mut(at - start)
    {
        *place = word;
        return;
    }
    singles.insert(at, word);
}

fn runs_of(
    singles: &std::collections::BTreeMap<usize, u32>,
) -> (Vec<(usize, usize, usize)>, Vec<u32>) {
    let mut found: Vec<(usize, usize, usize)> = Vec::new();
    for (along, &at) in singles.keys().enumerate() {
        match found.last_mut() {
            Some((start, length, _)) if *start + *length == at => *length += 1,
            _ => found.push((at, 1, along)),
        }
    }
    (found, singles.values().copied().collect())
}

fn first_word(words: Option<Vec<u32>>) -> u32 {
    words.and_then(|held| held.first().copied()).unwrap_or(0)
}

pub fn whole<R: Rule>(rule: &R, bricks: &Bricks, plan: &Plan) -> Result<Loop, LaneError> {
    let subgroup = bricks.subgroup;
    let grid = bricks.grid;
    fits("bricks::whole", bricks, plan)?;
    let every = Groups::Fixed(grid.blocks().div_ceil(subgroup));
    let tier = bricks.tier();

    let mut first = vec![(live::tick(subgroup)?, Groups::Fixed(1))];
    first.extend(wipe_brick_totals(subgroup, plan, tier)?);
    let mut stages: Stages = vec![
        first,
        vec![(settle(rule, bricks, Act::Dry, Listing::Every)?, every)],
    ];
    stages.extend(pooled(bricks, plan, Reach::Each)?);
    stages.push(vec![
        (drop_taken(bricks)?, Groups::Fixed(1)),
        (settle(rule, bricks, Act::Settle, Listing::Every)?, every),
    ]);
    Ok(live::staged(stages))
}
