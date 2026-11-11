use std::convert::Infallible;

use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, Lanes, U32, Vector};
use simdr::module::Id;

use crate::compact::{Over, Plan, Tier};
use crate::geometry::{Arith, Emit, Plain, Solid, low_bits};
use crate::layout::{DENSE, Dimension, GRAND, PARAMS, STATE, Windows};
use crate::live::{self, Loop, Stages};
use crate::rule3::{CORNERS, Rule};
use crate::sand::Grid;
use crate::{Error, Groups, Resident};

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

pub const MOST_GROUPS: u32 = 65_535;

pub const SPARSE_BINDINGS: u32 = 2 + TIER_BINDINGS;

pub const FLAT: &str = "a brick pool holds a solid grid laid out for three axes";
pub const SMALL: &str = "a side of the grid is shorter than a brick's side";
pub const UNPLANNED: &str = "the plan's blocks are not the grid's bricks";
pub const NO_SLOTS: &str = "a brick pool needs at least one slot";
pub const TOO_MANY_SLOTS: &str =
    "the pool's words pass the u32 index a kernel addresses STATE with";

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
        })
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
        self.capacity as usize * self.cells() + 1
    }

    #[must_use]
    pub const fn scratch(&self) -> usize {
        self.capacity as usize * self.cells()
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
        self.count() as usize * ENTRY + ENTRY
    }

    #[must_use]
    pub fn sizes<R: Rule>(&self, rule: &R, plan: &Plan) -> Vec<usize> {
        let mut sizes = live::sizes3(rule, plan);
        if let Some(state) = sizes.get_mut(STATE as usize) {
            *state = self.pool_words();
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
            padded + tiles * plan.subgroup as usize,
            1,
            3,
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
        if cells.len() != grid.cells() {
            return Err(Error::Overrun {
                index: STATE as usize,
                needed: grid.cells(),
                held: cells.len(),
            });
        }
        let per = self.cells();
        let mut table = vec![0_u32; self.table_words()];
        let mut pool = Vec::new();
        let mut held = 0_u32;
        let mut words = vec![0_u32; per];
        for brick in 0..self.count() {
            for (offset, word) in (0_u32..).zip(words.iter_mut()) {
                let point = self.point_of(brick, offset);
                let at = Solid::cell(grid, point) as usize;
                *word = cells.get(at).copied().unwrap_or_default();
            }
            let first = words.first().copied().unwrap_or_default();
            let entry = brick as usize * ENTRY;
            if words.iter().all(|&word| word == first) {
                if let Some(slot) = table.get_mut(entry..entry + 2) {
                    slot.copy_from_slice(&[UNIFORM, first]);
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
            pool.extend_from_slice(&words);
            held += 1;
        }
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

    pub fn write(
        &self,
        held: &mut Resident<'_>,
        plan: &Plan,
        cells: &[u32],
    ) -> Result<PoolUse, Error> {
        let packed = self.pack(cells)?;
        held.write(self.table() as usize, &packed.table)?;
        held.write(self.slots() as usize, &packed.slots)?;
        live::write_state(held, self.windows, 0, &packed.pool)?;
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
        let mut cells = vec![0_u32; grid.cells()];
        for brick in 0..self.count() {
            let entry = brick as usize * ENTRY;
            let slot = table.get(entry).copied().unwrap_or(UNIFORM);
            let value = table.get(entry + VALUE).copied().unwrap_or_default();
            for offset in 0..per {
                let word = if slot == UNIFORM {
                    value
                } else {
                    pool.get(slot as usize * per + offset)
                        .copied()
                        .unwrap_or_default()
                };
                let point = self.point_of(brick, u32::try_from(offset).unwrap_or(u32::MAX));
                if let Some(cell) = cells.get_mut(Solid::cell(grid, point) as usize) {
                    *cell = word;
                }
            }
        }
        cells
    }

    pub fn put(
        &self,
        held: &mut Resident<'_>,
        plan: &Plan,
        cells: &[(u32, u32)],
    ) -> Result<(), Error> {
        use crate::layout::{AWAKE, HOT, INDIRECT};

        let grid = self.grid;
        let past = grid.cells();
        if let Some(&(cell, _)) = cells.iter().find(|(cell, _)| *cell as usize >= past) {
            return Err(Error::Overrun {
                index: STATE as usize,
                needed: cell as usize + 1,
                held: past,
            });
        }
        self.check(held)?;
        let mut table = held.read(self.table() as usize, self.table_words())?;
        let mut header = held.read(self.slots() as usize, HEAD)?;
        let top = header.get(TOP).copied().unwrap_or_default() as usize;
        let stack = held.read(self.slots() as usize, HEAD + top)?;
        let mut top = top;
        let per = self.cells();

        let grand = held.read(GRAND as usize, 1)?.first().copied().unwrap_or(0);
        let mut octs: std::collections::BTreeSet<u32> = held
            .read(DENSE as usize, grand as usize)?
            .into_iter()
            .collect();
        let mut wanted = std::collections::BTreeSet::new();
        for &(cell, word) in cells {
            let Cell { brick, .. } = self.cell_of(Solid::point(grid, cell));
            let entry = brick as usize * ENTRY;
            let slot = table.get(entry).copied().unwrap_or(UNIFORM);
            let value = table.get(entry + VALUE).copied().unwrap_or_default();
            if slot == UNIFORM && word != value {
                wanted.insert(brick);
            }
        }
        if wanted.len() > top {
            return Err(Error::PoolFull {
                slots: self.capacity,
            });
        }

        let mut woken = std::collections::BTreeSet::new();
        for &(cell, word) in cells {
            let Cell { brick, offset } = self.cell_of(Solid::point(grid, cell));
            let entry = brick as usize * ENTRY;
            let slot = table.get(entry).copied().unwrap_or(UNIFORM);
            let value = table.get(entry + VALUE).copied().unwrap_or_default();
            let slot = if slot != UNIFORM {
                Some(slot)
            } else if word == value {
                None
            } else {
                let Some(&taken) = top.checked_sub(1).and_then(|at| stack.get(HEAD + at)) else {
                    return Err(Error::PoolFull {
                        slots: self.capacity,
                    });
                };
                top -= 1;
                live::write_state(held, self.windows, taken as usize * per, &vec![value; per])?;
                if let Some(place) = table.get_mut(entry) {
                    *place = taken;
                }
                held.write_at(self.table() as usize, entry, &[taken])?;
                Some(taken)
            };
            if let Some(slot) = slot {
                live::write_state(
                    held,
                    self.windows,
                    slot as usize * per + offset as usize,
                    &[word],
                )?;
            }
            for phase in 0..2 {
                let holder = Solid::held(grid, cell, phase);
                woken.insert(self.oct_of(holder.block));
            }
        }
        if let Some(word) = header.get_mut(TOP) {
            *word = u32::try_from(top).unwrap_or(u32::MAX);
        }
        held.write(self.slots() as usize, &header)?;

        let by = plan.per_block.trailing_zeros();
        for &oct in &woken {
            held.write_at(AWAKE as usize, oct as usize, &[2])?;
            held.write_at(HOT as usize, (oct >> by) as usize, &[2])?;
        }
        octs.extend(woken);
        let list: Vec<u32> = octs.into_iter().collect();
        let count = u32::try_from(list.len()).unwrap_or(u32::MAX);
        held.write(DENSE as usize, &list)?;
        held.write(GRAND as usize, &[count])?;
        held.write(INDIRECT as usize, &[count.div_ceil(plan.subgroup), 1, 1])
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

fn settled<const LANES: u32, R: Rule>(
    rule: &R,
    bricks: &Bricks,
    act: Act,
    listing: Listing,
) -> Result<Vec<u32>, LaneError> {
    use crate::sand3::{Given, Placing, frame_and_params, ruled};

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
    let second = kernel.module().constant_u32(1)?;
    let dump_base = kernel.load_at(PARAMS, second)?;

    let (oct, inside, counted_in) = match listing {
        Listing::Listed => {
            let head = kernel.module().constant_u32(0)?;
            let grand = kernel.load_at(GRAND, head)?;
            let claimed = kernel.load_at(DENSE, dump_base)?;
            let (inside, counted_in) = {
                let mut lanes = kernel.lanes()?;
                let here = lanes.splat_id::<U32, LANES>(slot)?;
                let listed = lanes.splat_id::<U32, LANES>(grand)?;
                let added = lanes.splat_id::<U32, LANES>(claimed)?;
                let total = lanes.add(listed, added)?;
                let settled = lanes.greater_than(total, here)?;
                let counted = lanes.greater_than(listed, here)?;
                (ones(&mut lanes, settled)?, ones(&mut lanes, counted)?)
            };
            (kernel.load_at(DENSE, slot)?, inside, counted_in)
        }
        Listing::Every => {
            let mut lanes = kernel.lanes()?;
            let one = lanes.splat_bits::<U32, LANES>(1)?;
            (slot, one, one)
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

    let mut entries = [frame; CORNERS];
    let mut values = [frame; CORNERS];
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
    let mut words = [frame; CORNERS];
    for (((word, &entry), &value), cell) in words
        .iter_mut()
        .zip(&entries)
        .zip(&values)
        .zip(&placed.cells)
    {
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
        let loaded = live::state_load(&mut kernel, windows, address.id())?;
        *word = {
            let mut lanes = kernel.lanes()?;
            let loaded = lanes.splat_id::<U32, LANES>(loaded)?;
            let value = lanes.splat_id::<U32, LANES>(value)?;
            let none = lanes.splat_bits::<U32, LANES>(0)?;
            let kept = lanes.greater_than(held, none)?;
            lanes.select(kept, loaded, value)?.id()
        };
        addresses.push(address);
        holding.push(held);
    }

    let step = {
        let mut lanes = kernel.lanes()?;
        let given = Given {
            frame,
            params: &params,
        };
        let placing = Placing {
            places: [placed.block[0]; CORNERS],
            block: placed.block,
        };
        ruled::<LANES, R>(&mut lanes, rule, &placing, words, given)?
    };

    match act {
        Act::Dry => {
            let spare = u32::try_from(bricks.count() as usize * ENTRY).unwrap_or(u32::MAX);
            for ((cell, &value), (&held, out)) in placed
                .cells
                .iter()
                .zip(&values)
                .zip(holding.iter().zip(&step.out))
            {
                let (stamp, touched, wanted) = {
                    let mut lanes = kernel.lanes()?;
                    let now = lanes.splat_id::<U32, LANES>(frame)?;
                    let one = lanes.splat_bits::<U32, LANES>(1)?;
                    let none = lanes.splat_bits::<U32, LANES>(0)?;
                    let four = lanes.splat_bits::<U32, LANES>(ENTRY as u32)?;
                    let scratch = lanes.splat_bits::<U32, LANES>(spare)?;
                    let scratch_next = lanes.splat_bits::<U32, LANES>(spare + 1)?;
                    let touched_at = lanes.splat_bits::<U32, LANES>(TOUCHED as u32)?;
                    let wanted_at = lanes.splat_bits::<U32, LANES>(WANTED as u32)?;
                    let stamp = lanes.add(now, one)?;
                    let stamp = lanes.max(stamp, one)?;
                    let entry = lanes.mul(cell.brick, four)?;
                    let listed = lanes.greater_than(inside, none)?;
                    let touched = lanes.add(entry, touched_at)?;
                    let touched = lanes.select(listed, touched, scratch)?;
                    let value = lanes.splat_id::<U32, LANES>(value)?;
                    let differs = lanes.not_equal(*out, value)?;
                    let differs = ones(&mut lanes, differs)?;
                    let uniform = lanes.equal(held, none)?;
                    let uniform = ones(&mut lanes, uniform)?;
                    let wants = lanes.mul(differs, uniform)?;
                    let wants = lanes.mul(wants, inside)?;
                    let wants = lanes.greater_than(wants, none)?;
                    let wanted = lanes.add(entry, wanted_at)?;
                    let wanted = lanes.select(wants, wanted, scratch_next)?;
                    (stamp, touched, wanted)
                };
                kernel.store_at(table, touched.id(), stamp.id())?;
                kernel.store_at(table, wanted.id(), stamp.id())?;
            }
        }
        Act::Settle => {
            let pool_scratch = u32::try_from(bricks.scratch()).unwrap_or(u32::MAX);
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
                let (at, lost) = {
                    let mut lanes = kernel.lanes()?;
                    let none = lanes.splat_bits::<U32, LANES>(0)?;
                    let scratch = lanes.splat_bits::<U32, LANES>(pool_scratch)?;
                    let kept = lanes.mul(go, held)?;
                    let kept = lanes.greater_than(kept, none)?;
                    let at = lanes.select(kept, *address, scratch)?;
                    let value = lanes.splat_id::<U32, LANES>(value)?;
                    let differs = lanes.not_equal(*out, value)?;
                    let differs = ones(&mut lanes, differs)?;
                    let uniform = lanes.equal(held, none)?;
                    let uniform = ones(&mut lanes, uniform)?;
                    let lost = lanes.mul(differs, uniform)?;
                    let lost = lanes.mul(lost, go)?;
                    let lost = lanes.greater_than(lost, none)?;
                    let flagged = lanes.splat_bits::<U32, LANES>(LOST as u32)?;
                    let elsewhere = lanes.splat_bits::<U32, LANES>(SCRATCH as u32)?;
                    (at, lanes.select(lost, flagged, elsewhere)?)
                };
                live::state_store(&mut kernel, windows, at.id(), out.id())?;
                let one = kernel.module().constant_u32(1)?;
                kernel.store_at(bricks.slots(), lost.id(), one)?;
            }

            if listing == Listing::Listed {
                let (touched, waking, counting) = {
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
                    let running = lanes.greater_than(go, none)?;
                    let counting = lanes.select(running, counted_in, none)?;
                    (touched, go, counting)
                };
                let grid = bricks.grid;
                let at = placed.block;
                live::woken_by::<LANES, 3, 8, 27, _>(
                    &mut kernel,
                    live::Block { subgroup, grid, at },
                    [touched, waking],
                    dump_base,
                    |lanes, offset| bricks.neighbour(&mut Emit::<LANES>(lanes), at, offset),
                )?;
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

fn per_brick(kernel: &mut Kernel<U32>, subgroup: u32) -> Result<Id, LaneError> {
    let uint = kernel.index_type();
    let group = kernel.workgroup_index();
    let lane = kernel.local_index();
    let width = kernel.module().constant_u32(subgroup)?;
    let base = kernel.module().i_mul(uint, group, width)?;
    Ok(kernel.module().i_add(uint, base, lane)?)
}

pub fn quiet(bricks: &Bricks, plan: &Plan) -> Result<Vec<u32>, LaneError> {
    let subgroup = bricks.subgroup;
    widths!(subgroup, quieted, [], (bricks, plan))
}

fn quieted<const LANES: u32>(bricks: &Bricks, plan: &Plan) -> Result<Vec<u32>, LaneError> {
    let subgroup = bricks.subgroup;
    let windows = bricks.windows();
    let table = bricks.table();
    let tier = bricks.tier();
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, bricks.bindings()))?;
    let brick = per_brick(&mut kernel, subgroup)?;
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
        let differs = differs.first().copied().unwrap_or(zero);
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

pub fn wants(bricks: &Bricks, plan: &Plan) -> Result<Vec<u32>, LaneError> {
    let subgroup = bricks.subgroup;
    widths!(subgroup, wanting, [], (bricks, plan))
}

fn wanting<const LANES: u32>(bricks: &Bricks, plan: &Plan) -> Result<Vec<u32>, LaneError> {
    let subgroup = bricks.subgroup;
    let table = bricks.table();
    let tier = bricks.tier();
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, bricks.bindings()))?;
    let brick = per_brick(&mut kernel, subgroup)?;
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
    let (at, inside) = {
        let mut lanes = kernel.lanes()?;
        let here = lanes.splat_id::<U32, LANES>(rank)?;
        let count = lanes.splat_id::<U32, LANES>(grand)?;
        let listed = lanes.splat_id::<U32, LANES>(listed)?;
        let bricks_held = lanes.splat_bits::<U32, LANES>(bricks.count())?;
        let four = lanes.splat_bits::<U32, LANES>(ENTRY as u32)?;
        let inside = lanes.greater_than(count, here)?;
        let brick = lanes.select(inside, listed, bricks_held)?;
        (lanes.mul(brick, four)?, ones(&mut lanes, inside)?)
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
    let per_lane = u32::try_from(bricks.cells()).unwrap_or(u32::MAX) / subgroup;
    let start = {
        let mut lanes = kernel.lanes()?;
        let slot = lanes.splat_id::<U32, LANES>(slot)?;
        let seat = lanes.splat_id::<U32, LANES>(lane)?;
        let by = lanes.splat_bits::<U32, LANES>(bricks.cell_bits())?;
        let run = lanes.splat_bits::<U32, LANES>(per_lane)?;
        let first = lanes.shift_left(slot, by)?;
        let along = lanes.mul(seat, run)?;
        lanes.add(first, along)?
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

fn ranked(bricks: &Bricks, plan: &Plan, per_group: u32) -> Result<Stages, LaneError> {
    let subgroup = bricks.subgroup;
    let tier = bricks.tier();
    let tiles = Groups::Fixed(plan.tiles);
    let mut stages = vec![vec![(
        live::mark_below(subgroup, tier, Over::Every, bricks.count())?,
        tiles,
    )]];
    stages.extend(brick_scans(subgroup, plan, tier)?);
    stages.push(vec![
        (
            crate::compact::gather(subgroup, tier, Over::Every, plan.padded())?,
            tiles,
        ),
        (
            crate::compact::prepare(subgroup, per_group, tier)?,
            Groups::Counting(1),
        ),
    ]);
    Ok(stages)
}

fn pooled(bricks: &Bricks, plan: &Plan, flags: Groups) -> Result<Stages, LaneError> {
    let subgroup = bricks.subgroup;
    let tier = bricks.tier();
    let listed = Groups::From {
        binding: tier.indirect as usize,
        at: 0,
    };
    let one = Groups::Fixed(1);
    let mut stages = vec![vec![(quiet(bricks, plan)?, flags)]];
    stages.extend(ranked(bricks, plan, subgroup)?);
    let mut freeing = vec![(release(bricks)?, listed)];
    freeing.extend(wipe_brick_totals(subgroup, plan, tier)?);
    stages.push(freeing);
    stages.push(vec![(lift(bricks)?, one), (wants(bricks, plan)?, flags)]);
    stages.extend(ranked(bricks, plan, bricks.takes())?);
    stages.push(vec![(take(bricks)?, listed)]);
    Ok(stages)
}

pub fn arranged<R: Rule>(
    rule: &R,
    bricks: &Bricks,
    plan: &Plan,
    coarse: bool,
) -> Result<Loop, LaneError> {
    let subgroup = bricks.subgroup;
    let grid = bricks.grid;
    live::fits("bricks::arranged", grid, plan)?;
    let flags = Groups::Fixed(bricks.count().div_ceil(subgroup));
    let tier = bricks.tier();

    let mut first = live::opening3(rule, subgroup, plan, coarse)?;
    first.extend(wipe_brick_totals(subgroup, plan, tier)?);
    let mut stages: Stages = vec![
        first,
        vec![(
            settle(rule, bricks, Act::Dry, Listing::Listed)?,
            Plan::listed(),
        )],
    ];
    stages.extend(pooled(bricks, plan, flags)?);
    stages.push(vec![
        (drop_taken(bricks)?, Groups::Fixed(1)),
        (
            settle(rule, bricks, Act::Settle, Listing::Listed)?,
            Plan::listed(),
        ),
    ]);
    stages.extend(live::closing3(subgroup, grid, plan, coarse)?);
    Ok(live::staged(stages))
}

pub fn whole<R: Rule>(rule: &R, bricks: &Bricks, plan: &Plan) -> Result<Loop, LaneError> {
    let subgroup = bricks.subgroup;
    let grid = bricks.grid;
    live::fits("bricks::whole", grid, plan)?;
    let flags = Groups::Fixed(bricks.count().div_ceil(subgroup));
    let every = Groups::Fixed(grid.blocks().div_ceil(subgroup));
    let tier = bricks.tier();

    let mut first = vec![(live::tick(subgroup)?, Groups::Fixed(1))];
    first.extend(wipe_brick_totals(subgroup, plan, tier)?);
    let mut stages: Stages = vec![
        first,
        vec![(settle(rule, bricks, Act::Dry, Listing::Every)?, every)],
    ];
    stages.extend(pooled(bricks, plan, flags)?);
    stages.push(vec![
        (drop_taken(bricks)?, Groups::Fixed(1)),
        (settle(rule, bricks, Act::Settle, Listing::Every)?, every),
    ]);
    Ok(live::staged(stages))
}
