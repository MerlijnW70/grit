use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, U32};
use simdr::module::{Id, op};

use crate::bricks::{AWAKE_LEVEL, Bricks, ENTRY, LOST, UNIFORM, VALUE};
use crate::geometry::{Arith, Solid};
use crate::layout::{AWAKE, HOT, Windows};
use crate::live::{lane_if, state_load, state_store};
use crate::resident::Groups;
use crate::sand::Grid;

pub const MOST_GROUPS: u32 = 65_535;

const LOGICAL_NOT: u16 = 168;

pub const CAP: u32 = 16_384;

pub const NEEDS: u32 = 1 << 31;

pub const COUNT: u32 = 0;
pub const WORDS: u32 = 1;
const BASE: u32 = 2;
const UNITS: u32 = 3;
const START: u32 = 4;
const SPAN: u32 = 7;
const FROM: u32 = 10;
const ALONG: u32 = 13;
const PLANES: u32 = 16;
const KEEP: u32 = 17;
const MODE: u32 = 18;
pub const HEAD: u32 = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Absorb {
    cells: [u32; 3],
}

impl Absorb {
    #[must_use]
    pub const fn new(cells: [u32; 3]) -> Option<Self> {
        if cells[0] == 0 && cells[1] == 0 && cells[2] == 0 {
            None
        } else {
            Some(Self { cells })
        }
    }

    #[must_use]
    pub const fn cells(&self) -> [u32; 3] {
        self.cells
    }

    #[must_use]
    pub fn too_wide(&self, size: [u32; 3]) -> Option<usize> {
        self.cells
            .iter()
            .zip(size)
            .position(|(&cells, side)| u64::from(cells) * 2 >= u64::from(side))
    }

    #[must_use]
    pub fn holds(&self, size: [u32; 3], point: [u32; 3]) -> bool {
        self.cells
            .iter()
            .zip(size)
            .zip(point)
            .any(|((&cells, side), at)| at < cells || at >= side.saturating_sub(cells))
    }

    #[must_use]
    pub fn shell(&self, size: [u32; 3]) -> u64 {
        let whole: u64 = size.iter().map(|side| u64::from(*side)).product();
        let inner: u64 = self
            .cells
            .iter()
            .zip(size)
            .map(|(&cells, side)| u64::from(side).saturating_sub(2 * u64::from(cells)))
            .product();
        whole - inner
    }

    fn thick(&self) -> [u32; 3] {
        self.cells.map(|cells| cells.div_ceil(2))
    }
}

#[must_use]
pub fn shell_uniform(bricks: &Bricks, table: &[u32], absorb: Absorb) -> Vec<u32> {
    let grid = bricks.grid();
    let size = [grid.wide, grid.deep, grid.tall];
    let sides = bricks.sides();
    let lattice = bricks.lattice();
    let mut found = Vec::new();
    for brick in 0..bricks.count() {
        let slot = table
            .get(brick as usize * ENTRY)
            .copied()
            .unwrap_or(UNIFORM);
        if slot != UNIFORM {
            continue;
        }
        let set = (0..bricks.planes()).any(|plane| {
            table
                .get(bricks.value_at(plane, brick))
                .is_some_and(|value| *value != 0)
        });
        if !set {
            continue;
        }
        let point = [
            brick % lattice[0],
            brick / lattice[0] % lattice[1],
            brick / lattice[0] / lattice[1],
        ];
        let touches = absorb.cells.iter().zip(point).zip(sides).zip(size).any(
            |(((&cells, at), brick), side)| {
                let first = at * brick;
                cells > 0 && (first < cells || first + brick > side - cells)
            },
        );
        if touches {
            found.push(brick);
        }
    }
    found
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clearer {
    pub binding: u32,
    pub pass: usize,
}

impl Clearer {
    #[must_use]
    pub const fn words() -> usize {
        (HEAD + CAP) as usize
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dense {
    pub subgroup: u32,
    pub grid: Grid,
    pub windows: Windows,
    pub planes: u32,
}

impl Dense {
    const fn octs(&self) -> [u32; 3] {
        [self.grid.wide / 2, self.grid.deep / 2, self.grid.tall / 2]
    }

    const fn size(&self) -> [u32; 3] {
        [self.grid.wide, self.grid.deep, self.grid.tall]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Box3 {
    pub from: [u32; 3],
    pub along: [u32; 3],
}

impl Box3 {
    #[must_use]
    pub fn spanning(from: [u32; 3], to: [u32; 3], size: [u32; 3]) -> Self {
        let mut along = [0; 3];
        for (axis, slot) in along.iter_mut().enumerate() {
            let side = size.get(axis).copied().unwrap_or(1);
            let low = from.get(axis).copied().unwrap_or(0);
            let high = to.get(axis).copied().unwrap_or(0);
            *slot = (high.wrapping_sub(low) & side.wrapping_sub(1)) + 1;
        }
        Self { from, along }
    }

    #[must_use]
    pub fn holds(&self, size: [u32; 3], point: [u32; 3]) -> bool {
        (0..3).all(|axis| {
            let side = size.get(axis).copied().unwrap_or(1);
            let at = point.get(axis).copied().unwrap_or(0);
            let low = self.from.get(axis).copied().unwrap_or(0);
            let along = self.along.get(axis).copied().unwrap_or(0);
            (at.wrapping_sub(low) & side.wrapping_sub(1)) < along
        })
    }

    #[must_use]
    pub fn cells(&self) -> u64 {
        self.along.iter().map(|side| u64::from(*side)).product()
    }

    #[must_use]
    pub fn units(&self, size: [u32; 3], unit: [u32; 3]) -> ([u32; 3], [u32; 3]) {
        let mut start = [0; 3];
        let mut span = [0; 3];
        for axis in 0..3 {
            let side = size.get(axis).copied().unwrap_or(1);
            let per = unit.get(axis).copied().unwrap_or(1).max(1);
            let low = self.from.get(axis).copied().unwrap_or(0);
            let along = self.along.get(axis).copied().unwrap_or(0);
            let units = side / per;
            let first = low / per;
            let last = (u64::from(low) + u64::from(along) - 1) / u64::from(per);
            let count = u32::try_from(last - u64::from(first) + 1).unwrap_or(u32::MAX);
            if let (Some(at), Some(over)) = (start.get_mut(axis), span.get_mut(axis)) {
                if count >= units {
                    *at = 0;
                    *over = units;
                } else {
                    *at = first;
                    *over = count;
                }
            }
        }
        (start, span)
    }

    #[must_use]
    pub fn words(&self, start: [u32; 3], span: [u32; 3], planes: u32, keep: u32) -> Vec<u32> {
        let mut words = vec![0_u32; HEAD as usize];
        let mut put = |at: u32, value: u32| {
            if let Some(slot) = words.get_mut(at as usize) {
                *slot = value;
            }
        };
        for axis in 0..3_u32 {
            let index = axis as usize;
            put(START + axis, start.get(index).copied().unwrap_or(0));
            put(SPAN + axis, span.get(index).copied().unwrap_or(1));
            put(FROM + axis, self.from.get(index).copied().unwrap_or(0));
            put(ALONG + axis, self.along.get(index).copied().unwrap_or(1));
        }
        put(PLANES, planes);
        put(KEEP, keep);
        words
    }
}

pub fn chunk(words: &mut [u32], base: u32, units: u32, acting: bool) {
    for (at, value) in [
        (COUNT, 0),
        (WORDS, 0),
        (BASE, base),
        (UNITS, units),
        (MODE, u32::from(acting)),
    ] {
        if let Some(slot) = words.get_mut(at as usize) {
            *slot = value;
        }
    }
}

struct Scalar<'kernel>(&'kernel mut Kernel<U32>);

impl Arith for Scalar<'_> {
    type Word = Id;
    type Truth = Id;
    type Error = LaneError;

    fn splat(&mut self, bits: u32) -> Result<Id, LaneError> {
        Ok(self.0.module().constant_u32(bits)?)
    }

    fn add(&mut self, left: Id, right: Id) -> Result<Id, LaneError> {
        let uint = self.0.index_type();
        Ok(self.0.module().i_add(uint, left, right)?)
    }

    fn mul(&mut self, left: Id, right: Id) -> Result<Id, LaneError> {
        let uint = self.0.index_type();
        Ok(self.0.module().i_mul(uint, left, right)?)
    }

    fn shift_left(&mut self, value: Id, by: Id) -> Result<Id, LaneError> {
        let uint = self.0.index_type();
        Ok(self
            .0
            .module()
            .binary(op::SHIFT_LEFT_LOGICAL, uint, value, by)?)
    }

    fn shift_right(&mut self, value: Id, by: Id) -> Result<Id, LaneError> {
        let uint = self.0.index_type();
        Ok(self
            .0
            .module()
            .binary(op::SHIFT_RIGHT_LOGICAL, uint, value, by)?)
    }

    fn greater_than(&mut self, left: Id, right: Id) -> Result<Id, LaneError> {
        let truth = self.0.module().type_bool()?;
        Ok(self
            .0
            .module()
            .binary(op::U_GREATER_THAN, truth, left, right)?)
    }
}

fn con(kernel: &mut Kernel<U32>, value: u32) -> Result<Id, LaneError> {
    Ok(kernel.module().constant_u32(value)?)
}

fn num(kernel: &mut Kernel<U32>, code: u16, left: Id, right: Id) -> Result<Id, LaneError> {
    let uint = kernel.index_type();
    Ok(kernel.module().binary(code, uint, left, right)?)
}

fn add(kernel: &mut Kernel<U32>, left: Id, right: Id) -> Result<Id, LaneError> {
    num(kernel, op::I_ADD, left, right)
}

fn sub(kernel: &mut Kernel<U32>, left: Id, right: Id) -> Result<Id, LaneError> {
    num(kernel, op::I_SUB, left, right)
}

fn mul(kernel: &mut Kernel<U32>, left: Id, right: Id) -> Result<Id, LaneError> {
    num(kernel, op::I_MUL, left, right)
}

fn div(kernel: &mut Kernel<U32>, left: Id, right: Id) -> Result<Id, LaneError> {
    num(kernel, op::U_DIV, left, right)
}

fn bits_and(kernel: &mut Kernel<U32>, left: Id, right: Id) -> Result<Id, LaneError> {
    num(kernel, op::BITWISE_AND, left, right)
}

fn shr(kernel: &mut Kernel<U32>, value: Id, by: u32) -> Result<Id, LaneError> {
    let by = con(kernel, by)?;
    num(kernel, op::SHIFT_RIGHT_LOGICAL, value, by)
}

fn shl(kernel: &mut Kernel<U32>, value: Id, by: u32) -> Result<Id, LaneError> {
    let by = con(kernel, by)?;
    num(kernel, op::SHIFT_LEFT_LOGICAL, value, by)
}

fn low(kernel: &mut Kernel<U32>, value: Id, side: u32) -> Result<Id, LaneError> {
    let mask = con(kernel, side.wrapping_sub(1))?;
    bits_and(kernel, value, mask)
}

fn truth(kernel: &mut Kernel<U32>, code: u16, left: Id, right: Id) -> Result<Id, LaneError> {
    let kind = kernel.module().type_bool()?;
    Ok(kernel.module().binary(code, kind, left, right)?)
}

fn below(kernel: &mut Kernel<U32>, left: Id, right: Id) -> Result<Id, LaneError> {
    truth(kernel, op::U_LESS_THAN, left, right)
}

fn both(kernel: &mut Kernel<U32>, left: Id, right: Id) -> Result<Id, LaneError> {
    let kind = kernel.module().type_bool()?;
    Ok(kernel.module().logical_and(kind, left, right)?)
}

fn either(kernel: &mut Kernel<U32>, left: Id, right: Id) -> Result<Id, LaneError> {
    let kind = kernel.module().type_bool()?;
    Ok(kernel.module().logical_or(kind, left, right)?)
}

fn pick(kernel: &mut Kernel<U32>, when: Id, yes: Id, other: Id) -> Result<Id, LaneError> {
    let uint = kernel.index_type();
    Ok(kernel.module().select(uint, when, yes, other)?)
}

fn counted(kernel: &mut Kernel<U32>, when: Id) -> Result<Id, LaneError> {
    let one = con(kernel, 1)?;
    let none = con(kernel, 0)?;
    pick(kernel, when, one, none)
}

fn head(kernel: &mut Kernel<U32>, binding: u32, at: u32) -> Result<Id, LaneError> {
    let place = con(kernel, at)?;
    kernel.load_at(binding, place)
}

fn lane_at(kernel: &mut Kernel<U32>, subgroup: u32) -> Result<Id, LaneError> {
    let group = kernel.workgroup_index();
    let lane = kernel.local_index();
    let width = con(kernel, subgroup)?;
    let base = mul(kernel, group, width)?;
    add(kernel, base, lane)
}

fn corner_of(kernel: &mut Kernel<U32>, oct: [Id; 3], corner: u32) -> Result<[Id; 3], LaneError> {
    let mut cell = oct;
    for (axis, value) in cell.iter_mut().enumerate() {
        let doubled = shl(kernel, *value, 1)?;
        let bit = con(kernel, (corner >> axis) & 1)?;
        *value = add(kernel, doubled, bit)?;
    }
    Ok(cell)
}

fn in_shell(
    kernel: &mut Kernel<U32>,
    absorb: Absorb,
    size: [u32; 3],
    cell: [Id; 3],
) -> Result<Id, LaneError> {
    let mut held: Option<Id> = None;
    for ((&cells, side), at) in absorb.cells.iter().zip(size).zip(cell) {
        if cells == 0 {
            continue;
        }
        let near = con(kernel, cells)?;
        let far = con(kernel, side - cells)?;
        let low_side = below(kernel, at, near)?;
        let high_side = below(kernel, at, far)?;
        let kind = kernel.module().type_bool()?;
        let high_side = kernel.module().unary(LOGICAL_NOT, kind, high_side)?;
        let here = either(kernel, low_side, high_side)?;
        held = Some(match held {
            None => here,
            Some(before) => either(kernel, before, here)?,
        });
    }
    match held {
        Some(found) => Ok(found),
        None => {
            let none = con(kernel, 0)?;
            below(kernel, none, none)
        }
    }
}

fn oct_touches(
    kernel: &mut Kernel<U32>,
    absorb: Absorb,
    size: [u32; 3],
    oct: [Id; 3],
) -> Result<Id, LaneError> {
    let low_corner = corner_of(kernel, oct, 0)?;
    let high_corner = corner_of(kernel, oct, 7)?;
    let mut held: Option<Id> = None;
    for (((&cells, side), first), last) in absorb
        .cells
        .iter()
        .zip(size)
        .zip(low_corner)
        .zip(high_corner)
    {
        if cells == 0 {
            continue;
        }
        let near = con(kernel, cells)?;
        let far = con(kernel, side - cells)?;
        let low_side = below(kernel, first, near)?;
        let high_side = below(kernel, last, far)?;
        let kind = kernel.module().type_bool()?;
        let high_side = kernel.module().unary(LOGICAL_NOT, kind, high_side)?;
        let here = either(kernel, low_side, high_side)?;
        held = Some(match held {
            None => here,
            Some(before) => either(kernel, before, here)?,
        });
    }
    match held {
        Some(found) => Ok(found),
        None => {
            let none = con(kernel, 0)?;
            below(kernel, none, none)
        }
    }
}

fn dense_index(kernel: &mut Kernel<U32>, size: [u32; 3], cell: [Id; 3]) -> Result<Id, LaneError> {
    let [x, y, z] = cell;
    let layer = shl(kernel, z, size[1].trailing_zeros())?;
    let row = add(kernel, layer, y)?;
    let lifted = shl(kernel, row, size[0].trailing_zeros())?;
    add(kernel, lifted, x)
}

fn back_one(kernel: &mut Kernel<U32>, oct: [Id; 3], sides: [u32; 3]) -> Result<[Id; 3], LaneError> {
    let mut out = oct;
    for (value, side) in out.iter_mut().zip(sides) {
        let minus = con(kernel, side.wrapping_sub(1))?;
        let moved = add(kernel, *value, minus)?;
        *value = low(kernel, moved, side)?;
    }
    Ok(out)
}

fn wake_dense(kernel: &mut Kernel<U32>, dense: &Dense, oct: [Id; 3]) -> Result<(), LaneError> {
    let per_block = crate::sweep::cells(dense.subgroup);
    let mark = con(kernel, 2)?;
    for offset in Solid::near() {
        let near = Solid::neighbour(&mut Scalar(kernel), dense.grid, oct, offset)?;
        kernel.store_at(AWAKE, near, mark)?;
        let block = shr(kernel, near, per_block.trailing_zeros())?;
        kernel.store_at(HOT, block, mark)?;
    }
    Ok(())
}

fn wake_sparse(kernel: &mut Kernel<U32>, bricks: &Bricks, oct: [Id; 3]) -> Result<(), LaneError> {
    let mark = con(kernel, AWAKE_LEVEL)?;
    let table = bricks.table();
    for offset in Solid::near() {
        let near = bricks.neighbour(&mut Scalar(kernel), oct, offset)?;
        let brick = shr(kernel, near, bricks.octs_bits())?;
        let local = low(kernel, near, bricks.per_block())?;
        let four = con(kernel, ENTRY as u32)?;
        let entry_at = mul(kernel, brick, four)?;
        let entry = kernel.load_at(table, entry_at)?;
        let uniform = con(kernel, UNIFORM)?;
        let held = truth(kernel, op::I_NOT_EQUAL, entry, uniform)?;
        lane_if(kernel, held, |kernel| {
            let per = con(kernel, bricks.per_block())?;
            let start = mul(kernel, entry, per)?;
            let at = add(kernel, start, local)?;
            kernel.store_at(AWAKE, at, mark)
        })?;
        kernel.store_at(HOT, brick, mark)?;
    }
    Ok(())
}

fn absorbed_oct<F>(
    kernel: &mut Kernel<U32>,
    absorb: Absorb,
    size: [u32; 3],
    (oct, planes): ([Id; 3], u32),
    mut address: F,
) -> Result<Id, LaneError>
where
    F: FnMut(&mut Kernel<U32>, [Id; 3], u32, u32) -> Result<(Windows, Id), LaneError>,
{
    let mut changed = con(kernel, 0)?;
    for corner in 0..8 {
        let cell = corner_of(kernel, oct, corner)?;
        let shell = in_shell(kernel, absorb, size, cell)?;
        for plane in 0..planes {
            let (windows, at) = address(kernel, cell, corner, plane)?;
            let word = state_load(kernel, windows, at)?;
            let none = con(kernel, 0)?;
            let set = truth(kernel, op::I_NOT_EQUAL, word, none)?;
            let hit = both(kernel, shell, set)?;
            lane_if(kernel, hit, |kernel| {
                let zero = con(kernel, 0)?;
                state_store(kernel, windows, at, zero)
            })?;
            let one = counted(kernel, hit)?;
            changed = num(kernel, op::BITWISE_OR, changed, one)?;
        }
    }
    Ok(changed)
}

#[must_use]
pub fn shell_octs(grid: Grid, absorb: Absorb) -> u64 {
    let octs = [grid.wide / 2, grid.deep / 2, grid.tall / 2];
    let whole: u64 = octs.iter().map(|side| u64::from(*side)).product();
    let inner: u64 = octs
        .iter()
        .zip(absorb.thick())
        .map(|(&side, thick)| u64::from(side).saturating_sub(2 * u64::from(thick)))
        .product();
    whole - inner
}

fn trips_of(units: u64, subgroup: u32) -> (u32, u32) {
    let reach = u64::from(subgroup.max(1)) * u64::from(MOST_GROUPS);
    let trips = units.div_ceil(reach).max(1);
    let groups = units.div_ceil(u64::from(subgroup.max(1)) * trips);
    (
        u32::try_from(trips).unwrap_or(u32::MAX),
        u32::try_from(groups).unwrap_or(u32::MAX),
    )
}

fn shell_point(
    kernel: &mut Kernel<U32>,
    octs: [u32; 3],
    thick: [u32; 3],
    lane: Id,
) -> Result<[Id; 3], LaneError> {
    let [wide, deep, tall] = octs;
    let [tx, ty, tz] = thick;
    let middle_tall = tall - 2 * tz;
    let middle_deep = deep - 2 * ty;
    let per_z = tz * wide * deep;
    let per_y = ty * wide * middle_tall;
    let per_x = tx * middle_deep * middle_tall;
    let in_z = {
        let bound = con(kernel, 2 * per_z)?;
        below(kernel, lane, bound)?
    };
    let past_z = {
        let bound = con(kernel, 2 * per_z)?;
        sub(kernel, lane, bound)?
    };
    let in_y = {
        let bound = con(kernel, 2 * per_y)?;
        below(kernel, past_z, bound)?
    };
    let past_y = {
        let bound = con(kernel, 2 * per_y)?;
        sub(kernel, past_z, bound)?
    };

    let z_point = {
        let per = con(kernel, per_z.max(1))?;
        let side = div(kernel, lane, per)?;
        let back = mul(kernel, side, per)?;
        let rest = sub(kernel, lane, back)?;
        let x = low(kernel, rest, wide)?;
        let shifted = shr(kernel, rest, wide.trailing_zeros())?;
        let y = low(kernel, shifted, deep)?;
        let layer = shr(kernel, rest, wide.trailing_zeros() + deep.trailing_zeros())?;
        let far = con(kernel, tall - tz)?;
        let high = add(kernel, far, layer)?;
        let none = con(kernel, 0)?;
        let upper = truth(kernel, op::I_NOT_EQUAL, side, none)?;
        let z = pick(kernel, upper, high, layer)?;
        [x, y, z]
    };
    let y_point = {
        let per = con(kernel, per_y.max(1))?;
        let side = div(kernel, past_z, per)?;
        let back = mul(kernel, side, per)?;
        let rest = sub(kernel, past_z, back)?;
        let x = low(kernel, rest, wide)?;
        let shifted = shr(kernel, rest, wide.trailing_zeros())?;
        let thick = con(kernel, ty.max(1))?;
        let layer = div(kernel, shifted, thick)?;
        let back = mul(kernel, layer, thick)?;
        let row = sub(kernel, shifted, back)?;
        let far = con(kernel, deep - ty)?;
        let high = add(kernel, far, row)?;
        let none = con(kernel, 0)?;
        let upper = truth(kernel, op::I_NOT_EQUAL, side, none)?;
        let y = pick(kernel, upper, high, row)?;
        let lift = con(kernel, tz)?;
        let z = add(kernel, layer, lift)?;
        [x, y, z]
    };
    let x_point = {
        let per = con(kernel, per_x.max(1))?;
        let side = div(kernel, past_y, per)?;
        let back = mul(kernel, side, per)?;
        let rest = sub(kernel, past_y, back)?;
        let thick = con(kernel, tx.max(1))?;
        let shifted = div(kernel, rest, thick)?;
        let back = mul(kernel, shifted, thick)?;
        let column = sub(kernel, rest, back)?;
        let middle = con(kernel, middle_deep.max(1))?;
        let layer = div(kernel, shifted, middle)?;
        let back = mul(kernel, layer, middle)?;
        let row = sub(kernel, shifted, back)?;
        let far = con(kernel, wide - tx)?;
        let high = add(kernel, far, column)?;
        let none = con(kernel, 0)?;
        let upper = truth(kernel, op::I_NOT_EQUAL, side, none)?;
        let x = pick(kernel, upper, high, column)?;
        let lift_y = con(kernel, ty)?;
        let y = add(kernel, row, lift_y)?;
        let lift_z = con(kernel, tz)?;
        let z = add(kernel, layer, lift_z)?;
        [x, y, z]
    };
    let mut point = x_point;
    for ((slot, from_y), from_z) in point.iter_mut().zip(y_point).zip(z_point) {
        let inner = pick(kernel, in_y, from_y, *slot)?;
        *slot = pick(kernel, in_z, from_z, inner)?;
    }
    Ok(point)
}

pub fn absorb_dense(dense: &Dense, absorb: Absorb) -> Result<(Vec<u32>, Groups), LaneError> {
    let subgroup = dense.subgroup;
    let octs = dense.octs();
    let size = dense.size();
    let thick = absorb.thick();
    let units = shell_octs(dense.grid, absorb);
    let total = u32::try_from(units).map_err(|_| LaneError::NoMapping {
        lanes: u32::MAX,
        width: subgroup,
    })?;
    let (trips, groups) = trips_of(units, subgroup);
    let cells = u32::try_from(dense.grid.cells()).unwrap_or(u32::MAX);
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, dense.windows.bindings()))?;
    let uint = kernel.index_type();
    let at = lane_at(&mut kernel, subgroup)?;
    let per = con(&mut kernel, trips)?;
    let begin = mul(&mut kernel, at, per)?;
    let nothing = con(&mut kernel, 0)?;
    kernel.repeat_rolled(trips, uint, nothing, |kernel, carried, counter| {
        let lane = add(kernel, begin, counter)?;
        let bound = con(kernel, total)?;
        let valid = below(kernel, lane, bound)?;
        lane_if(kernel, valid, |kernel| {
            let oct = shell_point(kernel, octs, thick, lane)?;
            let windows = dense.windows;
            let changed = absorbed_oct(
                kernel,
                absorb,
                size,
                (oct, dense.planes),
                |kernel, cell, _, plane| {
                    let index = dense_index(kernel, size, cell)?;
                    let lift = con(kernel, plane.wrapping_mul(cells))?;
                    Ok((windows, add(kernel, index, lift)?))
                },
            )?;
            let none = con(kernel, 0)?;
            let woke = truth(kernel, op::I_NOT_EQUAL, changed, none)?;
            lane_if(kernel, woke, |kernel| {
                wake_dense(kernel, dense, oct)?;
                let earlier = back_one(kernel, oct, octs)?;
                wake_dense(kernel, dense, earlier)
            })
        })?;
        Ok(carried)
    })?;
    Ok((kernel.finish()?, Groups::Fixed(groups)))
}

fn tiled_brick(kernel: &mut Kernel<U32>, bricks: &Bricks) -> Result<Id, LaneError> {
    use crate::sweep::PER_LANE;

    let subgroup = bricks.subgroup();
    let group = kernel.workgroup_index();
    let lane = kernel.local_index();
    let rank = shr(kernel, group, PER_LANE.trailing_zeros())?;
    let tile = kernel.load_at(bricks.tile_list(), rank)?;
    let within = low(kernel, group, PER_LANE)?;
    let width = con(kernel, subgroup)?;
    let within = mul(kernel, within, width)?;
    let within = add(kernel, within, lane)?;
    let start = shl(kernel, tile, bricks.per_block().trailing_zeros())?;
    add(kernel, start, within)
}

fn brick_point(kernel: &mut Kernel<U32>, bricks: &Bricks, brick: Id) -> Result<[Id; 3], LaneError> {
    let lattice = bricks.lattice();
    let x = low(kernel, brick, lattice[0])?;
    let rest = shr(kernel, brick, lattice[0].trailing_zeros())?;
    let y = low(kernel, rest, lattice[1])?;
    let z = shr(kernel, rest, lattice[1].trailing_zeros())?;
    Ok([x, y, z])
}

fn brick_number(
    kernel: &mut Kernel<U32>,
    bricks: &Bricks,
    point: [Id; 3],
) -> Result<Id, LaneError> {
    let lattice = bricks.lattice();
    let layer = shl(kernel, point[2], lattice[1].trailing_zeros())?;
    let row = add(kernel, layer, point[1])?;
    let lifted = shl(kernel, row, lattice[0].trailing_zeros())?;
    add(kernel, lifted, point[0])
}

struct Inside {
    oct: [Id; 3],
    local: [Id; 3],
}

fn brick_oct(
    kernel: &mut Kernel<U32>,
    bricks: &Bricks,
    origin: [Id; 3],
    local: Id,
) -> Result<Inside, LaneError> {
    let bits = bricks.bits();
    let x = low(kernel, local, 1 << bits[0])?;
    let rest = shr(kernel, local, bits[0])?;
    let y = low(kernel, rest, 1 << bits[1])?;
    let z = shr(kernel, rest, bits[1])?;
    let inner = [x, y, z];
    let mut oct = inner;
    for (((slot, from), bit), part) in oct.iter_mut().zip(origin).zip(bits).zip(inner) {
        let base = shl(kernel, from, bit)?;
        *slot = add(kernel, base, part)?;
    }
    Ok(Inside { oct, local: inner })
}

fn brick_offset(
    kernel: &mut Kernel<U32>,
    bricks: &Bricks,
    local: [Id; 3],
    corner: u32,
) -> Result<Id, LaneError> {
    let sides = bricks.sides();
    let cell = corner_of(kernel, local, corner)?;
    let layer = shl(kernel, cell[2], sides[1].trailing_zeros())?;
    let row = add(kernel, layer, cell[1])?;
    let lifted = shl(kernel, row, sides[0].trailing_zeros())?;
    add(kernel, lifted, cell[0])
}

fn value_place(
    kernel: &mut Kernel<U32>,
    bricks: &Bricks,
    brick: Id,
    plane: usize,
) -> Result<Id, LaneError> {
    if plane == 0 {
        let four = con(kernel, ENTRY as u32)?;
        let entry = mul(kernel, brick, four)?;
        let value = con(kernel, VALUE as u32)?;
        add(kernel, entry, value)
    } else {
        let start = con(kernel, bricks.plane_values(plane))?;
        add(kernel, start, brick)
    }
}

pub fn absorb_sparse(bricks: &Bricks, absorb: Absorb) -> Result<(Vec<u32>, Groups), LaneError> {
    let subgroup = bricks.subgroup();
    let size = {
        let grid = bricks.grid();
        [grid.wide, grid.deep, grid.tall]
    };
    let sides = bricks.sides();
    let table = bricks.table();
    let windows = bricks.windows();
    let cells = u32::try_from(bricks.cells()).unwrap_or(u32::MAX);
    let planes = bricks.planes();
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, bricks.bindings()))?;
    let uint = kernel.index_type();
    let lane = tiled_brick(&mut kernel, bricks)?;
    let count = con(&mut kernel, bricks.count())?;
    let inside = below(&mut kernel, lane, count)?;
    let none = con(&mut kernel, 0)?;
    let brick = pick(&mut kernel, inside, lane, none)?;
    let point = brick_point(&mut kernel, bricks, brick)?;
    let mut origin = point;
    for (slot, side) in origin.iter_mut().zip(sides) {
        *slot = shl(&mut kernel, *slot, side.trailing_zeros())?;
    }
    let touches = {
        let mut held: Option<Id> = None;
        for (((&shell, side), first), brick_side) in
            absorb.cells.iter().zip(size).zip(origin).zip(sides)
        {
            if shell == 0 {
                continue;
            }
            let near = con(&mut kernel, shell)?;
            let low_side = below(&mut kernel, first, near)?;
            let reach = con(&mut kernel, brick_side)?;
            let last = add(&mut kernel, first, reach)?;
            let far = con(&mut kernel, side - shell + 1)?;
            let high_side = below(&mut kernel, last, far)?;
            let kind = kernel.module().type_bool()?;
            let high_side = kernel.module().unary(LOGICAL_NOT, kind, high_side)?;
            let here = either(&mut kernel, low_side, high_side)?;
            held = Some(match held {
                None => here,
                Some(before) => either(&mut kernel, before, here)?,
            });
        }
        match held {
            Some(found) => found,
            None => {
                let none = con(&mut kernel, 0)?;
                below(&mut kernel, none, none)?
            }
        }
    };
    let four = con(&mut kernel, ENTRY as u32)?;
    let entry_at = mul(&mut kernel, brick, four)?;
    let entry = kernel.load_at(table, entry_at)?;
    let uniform = con(&mut kernel, UNIFORM)?;
    let held = truth(&mut kernel, op::I_NOT_EQUAL, entry, uniform)?;
    let looked = both(&mut kernel, inside, touches)?;
    let stepped = both(&mut kernel, looked, held)?;
    lane_if(&mut kernel, stepped, |kernel| {
        let per = con(kernel, cells)?;
        let base = mul(kernel, entry, per)?;
        let nothing = con(kernel, 0)?;
        kernel.repeat_rolled(
            bricks.per_block(),
            uint,
            nothing,
            |kernel, carried, local| {
                let Inside { oct, local } = brick_oct(kernel, bricks, point, local)?;
                let near = oct_touches(kernel, absorb, size, oct)?;
                lane_if(kernel, near, |kernel| {
                    let changed = absorbed_oct(
                        kernel,
                        absorb,
                        size,
                        (oct, u32::try_from(planes).unwrap_or(1)),
                        |kernel, _, corner, plane| {
                            let offset = brick_offset(kernel, bricks, local, corner)?;
                            let within = add(kernel, base, offset)?;
                            let lift = con(kernel, bricks.plane_start(plane as usize))?;
                            Ok((windows, add(kernel, within, lift)?))
                        },
                    )?;
                    let none = con(kernel, 0)?;
                    let woke = truth(kernel, op::I_NOT_EQUAL, changed, none)?;
                    lane_if(kernel, woke, |kernel| {
                        wake_sparse(kernel, bricks, oct)?;
                        let octs = Solid::blocks(bricks.grid());
                        let earlier = back_one(kernel, oct, octs)?;
                        wake_sparse(kernel, bricks, earlier)
                    })
                })?;
                Ok(carried)
            },
        )?;
        Ok(())
    })?;
    let kind = kernel.module().type_bool()?;
    let not_held = kernel.module().unary(LOGICAL_NOT, kind, held)?;
    let still = both(&mut kernel, looked, not_held)?;
    lane_if(&mut kernel, still, |kernel| {
        let mut any = con(kernel, 0)?;
        for plane in 0..planes {
            let at = value_place(kernel, bricks, brick, plane)?;
            let value = kernel.load_at(table, at)?;
            any = num(kernel, op::BITWISE_OR, any, value)?;
        }
        let none = con(kernel, 0)?;
        let set = truth(kernel, op::I_NOT_EQUAL, any, none)?;
        lane_if(kernel, set, |kernel| {
            let flag = con(kernel, LOST as u32)?;
            let one = con(kernel, 1)?;
            kernel.store_at(bricks.slots(), flag, one)
        })
    })?;
    Ok((kernel.finish()?, bricks.lane_groups()))
}

fn unit_point(
    kernel: &mut Kernel<U32>,
    binding: u32,
    lattice: [u32; 3],
    unit: Id,
) -> Result<[Id; 3], LaneError> {
    let wide = head(kernel, binding, SPAN)?;
    let deep = head(kernel, binding, SPAN + 1)?;
    let row = div(kernel, unit, wide)?;
    let back = mul(kernel, row, wide)?;
    let x = sub(kernel, unit, back)?;
    let z = div(kernel, row, deep)?;
    let back = mul(kernel, z, deep)?;
    let y = sub(kernel, row, back)?;
    let mut point = [x, y, z];
    for (axis, (slot, side)) in point.iter_mut().zip(lattice).enumerate() {
        let start = head(kernel, binding, START + axis as u32)?;
        let moved = add(kernel, start, *slot)?;
        *slot = low(kernel, moved, side)?;
    }
    Ok(point)
}

fn in_box(
    kernel: &mut Kernel<U32>,
    binding: u32,
    size: [u32; 3],
    cell: [Id; 3],
) -> Result<Id, LaneError> {
    let mut held: Option<Id> = None;
    for (axis, (side, at)) in size.iter().zip(cell).enumerate() {
        let from = head(kernel, binding, FROM + axis as u32)?;
        let along = head(kernel, binding, ALONG + axis as u32)?;
        let apart = sub(kernel, at, from)?;
        let apart = low(kernel, apart, *side)?;
        let here = below(kernel, apart, along)?;
        held = Some(match held {
            None => here,
            Some(before) => both(kernel, before, here)?,
        });
    }
    held.ok_or(LaneError::NoOpenBlock { arm: "box" })
}

fn cleared_oct<F>(
    kernel: &mut Kernel<U32>,
    binding: u32,
    size: [u32; 3],
    (oct, planes): ([Id; 3], u32),
    mut address: F,
) -> Result<Id, LaneError>
where
    F: FnMut(&mut Kernel<U32>, [Id; 3], u32, u32) -> Result<(Windows, Id), LaneError>,
{
    let chosen = head(kernel, binding, PLANES)?;
    let keep = head(kernel, binding, KEEP)?;
    let mut changed = con(kernel, 0)?;
    for corner in 0..8 {
        let cell = corner_of(kernel, oct, corner)?;
        let boxed = in_box(kernel, binding, size, cell)?;
        for plane in 0..planes {
            let picked = shr(kernel, chosen, plane)?;
            let one = con(kernel, 1)?;
            let picked = bits_and(kernel, picked, one)?;
            let picked = truth(kernel, op::I_EQUAL, picked, one)?;
            let (windows, at) = address(kernel, cell, corner, plane)?;
            let word = state_load(kernel, windows, at)?;
            let kept = bits_and(kernel, word, keep)?;
            let differs = truth(kernel, op::I_NOT_EQUAL, kept, word)?;
            let hit = both(kernel, boxed, picked)?;
            let hit = both(kernel, hit, differs)?;
            lane_if(kernel, hit, |kernel| state_store(kernel, windows, at, kept))?;
            let one = counted(kernel, hit)?;
            changed = add(kernel, changed, one)?;
        }
    }
    Ok(changed)
}

fn appended(kernel: &mut Kernel<U32>, binding: u32, unit: Id) -> Result<(), LaneError> {
    let counter = con(kernel, COUNT)?;
    let one = con(kernel, 1)?;
    let slot = kernel.atomic_add_at(binding, counter, one)?;
    let room = con(kernel, CAP)?;
    let fits = below(kernel, slot, room)?;
    lane_if(kernel, fits, |kernel| {
        let start = con(kernel, HEAD)?;
        let at = add(kernel, start, slot)?;
        kernel.store_at(binding, at, unit)
    })
}

fn tallied(kernel: &mut Kernel<U32>, binding: u32, words: Id) -> Result<(), LaneError> {
    let at = con(kernel, WORDS)?;
    kernel.atomic_add_at(binding, at, words).map(|_| ())
}

fn valid_unit(
    kernel: &mut Kernel<U32>,
    binding: u32,
    subgroup: u32,
) -> Result<(Id, Id), LaneError> {
    let at = lane_at(kernel, subgroup)?;
    let units = head(kernel, binding, UNITS)?;
    let valid = below(kernel, at, units)?;
    let base = head(kernel, binding, BASE)?;
    let unit = add(kernel, base, at)?;
    Ok((unit, valid))
}

pub fn clear_dense(dense: &Dense, binding: u32) -> Result<Vec<u32>, LaneError> {
    let subgroup = dense.subgroup;
    let octs = dense.octs();
    let size = dense.size();
    let cells = u32::try_from(dense.grid.cells()).unwrap_or(u32::MAX);
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, binding + 1))?;
    let (unit, valid) = valid_unit(&mut kernel, binding, subgroup)?;
    let windows = dense.windows;
    lane_if(&mut kernel, valid, |kernel| {
        let oct = unit_point(kernel, binding, octs, unit)?;
        let changed = cleared_oct(
            kernel,
            binding,
            size,
            (oct, dense.planes),
            |kernel, cell, _, plane| {
                let index = dense_index(kernel, size, cell)?;
                let lift = con(kernel, plane.wrapping_mul(cells))?;
                Ok((windows, add(kernel, index, lift)?))
            },
        )?;
        let none = con(kernel, 0)?;
        let some = truth(kernel, op::I_NOT_EQUAL, changed, none)?;
        lane_if(kernel, some, |kernel| {
            let index = {
                let layer = shl(kernel, oct[2], octs[1].trailing_zeros())?;
                let row = add(kernel, layer, oct[1])?;
                let lifted = shl(kernel, row, octs[0].trailing_zeros())?;
                add(kernel, lifted, oct[0])?
            };
            appended(kernel, binding, index)?;
            tallied(kernel, binding, changed)
        })
    })?;
    kernel.finish()
}

fn covered(
    kernel: &mut Kernel<U32>,
    binding: u32,
    size: [u32; 3],
    origin: [Id; 3],
    sides: [u32; 3],
) -> Result<Id, LaneError> {
    let mut held: Option<Id> = None;
    for (axis, ((side, at), brick_side)) in size.iter().zip(origin).zip(sides).enumerate() {
        let from = head(kernel, binding, FROM + axis as u32)?;
        let along = head(kernel, binding, ALONG + axis as u32)?;
        let apart = sub(kernel, at, from)?;
        let apart = low(kernel, apart, *side)?;
        let reach = con(kernel, brick_side)?;
        let last = add(kernel, apart, reach)?;
        let room = con(kernel, 1)?;
        let room = add(kernel, along, room)?;
        let inside = below(kernel, last, room)?;
        let whole_side = con(kernel, *side)?;
        let whole = truth(kernel, op::I_EQUAL, along, whole_side)?;
        let here = either(kernel, inside, whole)?;
        held = Some(match held {
            None => here,
            Some(before) => both(kernel, before, here)?,
        });
    }
    held.ok_or(LaneError::NoOpenBlock { arm: "covered" })
}

pub fn clear_sparse(bricks: &Bricks, binding: u32) -> Result<Vec<u32>, LaneError> {
    let subgroup = bricks.subgroup();
    let size = {
        let grid = bricks.grid();
        [grid.wide, grid.deep, grid.tall]
    };
    let sides = bricks.sides();
    let table = bricks.table();
    let windows = bricks.windows();
    let cells = u32::try_from(bricks.cells()).unwrap_or(u32::MAX);
    let planes = bricks.planes();
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, binding + 1))?;
    let uint = kernel.index_type();
    let (unit, valid) = valid_unit(&mut kernel, binding, subgroup)?;
    lane_if(&mut kernel, valid, |kernel| {
        let point = unit_point(kernel, binding, bricks.lattice(), unit)?;
        let brick = brick_number(kernel, bricks, point)?;
        let mut origin = point;
        for (slot, side) in origin.iter_mut().zip(sides) {
            *slot = shl(kernel, *slot, side.trailing_zeros())?;
        }
        let four = con(kernel, ENTRY as u32)?;
        let entry_at = mul(kernel, brick, four)?;
        let entry = kernel.load_at(table, entry_at)?;
        let uniform = con(kernel, UNIFORM)?;
        let held = truth(kernel, op::I_NOT_EQUAL, entry, uniform)?;
        let mode = head(kernel, binding, MODE)?;
        let one = con(kernel, 1)?;
        let acting = truth(kernel, op::I_EQUAL, mode, one)?;
        let clearing = both(kernel, held, acting)?;
        lane_if(kernel, clearing, |kernel| {
            let per = con(kernel, cells)?;
            let base = mul(kernel, entry, per)?;
            let nothing = con(kernel, 0)?;
            let changed = kernel.repeat_rolled(
                bricks.per_block(),
                uint,
                nothing,
                |kernel, carried, local| {
                    let Inside { oct, local } = brick_oct(kernel, bricks, point, local)?;
                    let more = cleared_oct(
                        kernel,
                        binding,
                        size,
                        (oct, u32::try_from(planes).unwrap_or(1)),
                        |kernel, _, corner, plane| {
                            let offset = brick_offset(kernel, bricks, local, corner)?;
                            let within = add(kernel, base, offset)?;
                            let lift = con(kernel, bricks.plane_start(plane as usize))?;
                            Ok((windows, add(kernel, within, lift)?))
                        },
                    )?;
                    add(kernel, carried, more)
                },
            )?;
            let none = con(kernel, 0)?;
            let some = truth(kernel, op::I_NOT_EQUAL, changed, none)?;
            lane_if(kernel, some, |kernel| {
                appended(kernel, binding, brick)?;
                tallied(kernel, binding, changed)
            })
        })?;
        let kind = kernel.module().type_bool()?;
        let not_held = kernel.module().unary(LOGICAL_NOT, kind, held)?;
        lane_if(kernel, not_held, |kernel| {
            let whole = covered(kernel, binding, size, origin, sides)?;
            let chosen = head(kernel, binding, PLANES)?;
            let keep = head(kernel, binding, KEEP)?;
            let mut stored = con(kernel, 0)?;
            let mut wanted = con(kernel, 0)?;
            for plane in 0..planes {
                let at = value_place(kernel, bricks, brick, plane)?;
                let value = kernel.load_at(table, at)?;
                let kept = bits_and(kernel, value, keep)?;
                let differs = truth(kernel, op::I_NOT_EQUAL, kept, value)?;
                let picked = shr(kernel, chosen, u32::try_from(plane).unwrap_or(0))?;
                let one = con(kernel, 1)?;
                let picked = bits_and(kernel, picked, one)?;
                let picked = truth(kernel, op::I_EQUAL, picked, one)?;
                let hit = both(kernel, differs, picked)?;
                let fits = both(kernel, hit, whole)?;
                let fits = both(kernel, fits, acting)?;
                lane_if(kernel, fits, |kernel| kernel.store_at(table, at, kept))?;
                let one = counted(kernel, fits)?;
                stored = add(kernel, stored, one)?;
                let kind = kernel.module().type_bool()?;
                let partial = kernel.module().unary(LOGICAL_NOT, kind, whole)?;
                let short = both(kernel, hit, partial)?;
                let one = counted(kernel, short)?;
                wanted = num(kernel, op::BITWISE_OR, wanted, one)?;
            }
            let none = con(kernel, 0)?;
            let some = truth(kernel, op::I_NOT_EQUAL, stored, none)?;
            lane_if(kernel, some, |kernel| {
                appended(kernel, binding, brick)?;
                let per = con(kernel, cells)?;
                let words = mul(kernel, stored, per)?;
                tallied(kernel, binding, words)
            })?;
            let asks = truth(kernel, op::I_NOT_EQUAL, wanted, none)?;
            lane_if(kernel, asks, |kernel| {
                let flag = con(kernel, NEEDS)?;
                let marked = num(kernel, op::BITWISE_OR, brick, flag)?;
                appended(kernel, binding, marked)
            })
        })
    })?;
    kernel.finish()
}
