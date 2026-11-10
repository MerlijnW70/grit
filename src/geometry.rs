//! The spatial shape of the world, written once for any number of axes.
//!
//! Every function here is generic over [`Arith`], so the same steps either
//! emit SPIR-V through [`Emit`] or run on the CPU through [`Plain`]. The 2D
//! engine builds its kernels with [`Plane`]; [`Solid`] is the 3D shape, run
//! only on the CPU so far.

use std::convert::Infallible;

use simdr::lanes::{LaneError, Lanes, Predicate, U32, Vector};

use crate::sand::Grid;

/// The operations the geometry is written in.
pub trait Arith {
    type Word: Copy;
    type Truth: Copy;
    type Error;

    fn splat(&mut self, bits: u32) -> Result<Self::Word, Self::Error>;
    fn add(&mut self, left: Self::Word, right: Self::Word) -> Result<Self::Word, Self::Error>;
    fn mul(&mut self, left: Self::Word, right: Self::Word) -> Result<Self::Word, Self::Error>;
    fn shift_left(&mut self, value: Self::Word, by: Self::Word) -> Result<Self::Word, Self::Error>;
    fn shift_right(&mut self, value: Self::Word, by: Self::Word)
    -> Result<Self::Word, Self::Error>;
    fn greater_than(
        &mut self,
        left: Self::Word,
        right: Self::Word,
    ) -> Result<Self::Truth, Self::Error>;
}

/// Emits each operation into a kernel, one lane vector at a time.
pub struct Emit<'held, 'module, const LANES: u32>(pub &'held mut Lanes<'module>);

impl<const LANES: u32> Arith for Emit<'_, '_, LANES> {
    type Word = Vector<U32, LANES>;
    type Truth = Predicate<LANES>;
    type Error = LaneError;

    fn splat(&mut self, bits: u32) -> Result<Self::Word, LaneError> {
        self.0.splat_bits::<U32, LANES>(bits)
    }

    fn add(&mut self, left: Self::Word, right: Self::Word) -> Result<Self::Word, LaneError> {
        self.0.add(left, right)
    }

    fn mul(&mut self, left: Self::Word, right: Self::Word) -> Result<Self::Word, LaneError> {
        self.0.mul(left, right)
    }

    fn shift_left(&mut self, value: Self::Word, by: Self::Word) -> Result<Self::Word, LaneError> {
        self.0.shift_left(value, by)
    }

    fn shift_right(&mut self, value: Self::Word, by: Self::Word) -> Result<Self::Word, LaneError> {
        self.0.shift_right_logical(value, by)
    }

    fn greater_than(
        &mut self,
        left: Self::Word,
        right: Self::Word,
    ) -> Result<Self::Truth, LaneError> {
        self.0.greater_than(left, right)
    }
}

/// Runs each operation on the CPU, with the wrapping arithmetic of a `u32`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Plain;

impl Arith for Plain {
    type Word = u32;
    type Truth = bool;
    type Error = Infallible;

    fn splat(&mut self, bits: u32) -> Result<u32, Infallible> {
        Ok(bits)
    }

    fn add(&mut self, left: u32, right: u32) -> Result<u32, Infallible> {
        Ok(left.wrapping_add(right))
    }

    fn mul(&mut self, left: u32, right: u32) -> Result<u32, Infallible> {
        Ok(left.wrapping_mul(right))
    }

    fn shift_left(&mut self, value: u32, by: u32) -> Result<u32, Infallible> {
        Ok(value.checked_shl(by).unwrap_or(0))
    }

    fn shift_right(&mut self, value: u32, by: u32) -> Result<u32, Infallible> {
        Ok(value.checked_shr(by).unwrap_or(0))
    }

    fn greater_than(&mut self, left: u32, right: u32) -> Result<bool, Infallible> {
        Ok(left > right)
    }
}

fn plainly<T>(done: Result<T, Infallible>) -> T {
    match done {
        Ok(value) => value,
        Err(never) => match never {},
    }
}

/// `value` modulo `2^bits`, written with the operations a lane has.
pub fn low_bits<A: Arith>(arith: &mut A, value: A::Word, bits: u32) -> Result<A::Word, A::Error> {
    let by = arith.splat(bits)?;
    let minus = arith.splat(u32::MAX)?;

    let above = arith.shift_right(value, by)?;
    let back = arith.shift_left(above, by)?;
    let away = arith.mul(back, minus)?;
    arith.add(value, away)
}

/// A block: its point in the lattice of blocks, and the index of each of
/// its cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placed<W, const AXES: usize, const CORNERS: usize> {
    pub block: [W; AXES],
    pub places: [W; CORNERS],
}

/// The block that holds a cell, from [`Space::holder`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Held<W, const AXES: usize> {
    pub block: [W; AXES],
    pub index: W,
    pub corner: W,
}

/// A world of `AXES` axes, cut into blocks of two cells along every axis.
/// `CORNERS` is the number of cells in a block and `NEAR` the number of
/// blocks a changed block wakes, itself included.
#[derive(Debug, Clone, Copy)]
pub struct Space<const AXES: usize, const CORNERS: usize, const NEAR: usize>;

/// The 2D world of quads: the only shape the engine builds kernels for.
pub type Plane = Space<2, 4, 9>;

/// The 3D world of octs.
pub type Solid = Space<3, 8, 27>;

impl<const AXES: usize, const CORNERS: usize, const NEAR: usize> Space<AXES, CORNERS, NEAR> {
    const SOUND: () =
        assert!(AXES >= 1 && AXES <= 3 && CORNERS == 1 << AXES && NEAR == 3_usize.pow(AXES as u32));

    /// The cells along each axis: wide, deep and then tall.
    #[must_use]
    pub fn extents(grid: Grid) -> [u32; AXES] {
        let () = Self::SOUND;
        let all = [grid.wide, grid.deep, grid.tall];
        let mut out = [1; AXES];
        for (slot, extent) in out.iter_mut().zip(all) {
            *slot = extent;
        }
        out
    }

    /// The blocks along each axis.
    #[must_use]
    pub fn blocks(grid: Grid) -> [u32; AXES] {
        Self::extents(grid).map(|extent| extent / 2)
    }

    /// The index of the cell at `point`, every axis wrapped onto the torus.
    /// The first axis runs fastest.
    pub fn at<A: Arith>(
        arith: &mut A,
        extents: [u32; AXES],
        point: [A::Word; AXES],
    ) -> Result<A::Word, A::Error> {
        let () = Self::SOUND;
        let mut wrapped = point;
        for (value, extent) in wrapped.iter_mut().zip(extents) {
            *value = low_bits(arith, *value, extent.trailing_zeros())?;
        }
        Self::number(arith, wrapped, extents.map(u32::trailing_zeros))
    }

    /// `values` read as one number, the first axis lowest and axis `i`
    /// taking `bits[i]` bits; nothing is wrapped.
    fn number<A: Arith>(
        arith: &mut A,
        values: [A::Word; AXES],
        bits: [u32; AXES],
    ) -> Result<A::Word, A::Error> {
        let mut from_last = values.into_iter().zip(bits).rev();
        let Some((mut number, _)) = from_last.next() else {
            return arith.splat(0);
        };
        for (value, width) in from_last {
            let by = arith.splat(width)?;
            number = arith.shift_left(number, by)?;
            number = arith.add(number, value)?;
        }
        Ok(number)
    }

    /// The point of `index` in a box of `extents`: every axis but the last
    /// is taken modulo its extent, and the last holds what is left.
    pub fn split<A: Arith>(
        arith: &mut A,
        extents: [u32; AXES],
        index: A::Word,
    ) -> Result<[A::Word; AXES], A::Error> {
        let mut point = [index; AXES];
        let mut rest = index;
        for (axis, (slot, extent)) in point.iter_mut().zip(extents).enumerate() {
            if axis + 1 == AXES {
                *slot = rest;
            } else {
                let by = arith.splat(extent.trailing_zeros())?;
                let next = arith.shift_right(rest, by)?;
                *slot = low_bits(arith, rest, extent.trailing_zeros())?;
                rest = next;
            }
        }
        Ok(point)
    }

    /// The cells of the block at `block` in the phase `phase`, 0 or 1, as
    /// points not yet wrapped. Corner `k` lies one cell further along every
    /// axis whose bit is set in `k`.
    pub fn corners<A: Arith>(
        arith: &mut A,
        block: [A::Word; AXES],
        phase: A::Word,
    ) -> Result<[[A::Word; AXES]; CORNERS], A::Error> {
        let () = Self::SOUND;
        let one = arith.splat(1)?;
        let mut first = block;
        for value in &mut first {
            *value = arith.shift_left(*value, one)?;
        }
        for value in &mut first {
            *value = arith.add(*value, phase)?;
        }
        let mut further = first;
        for value in &mut further {
            *value = arith.add(*value, one)?;
        }
        let mut corners = [first; CORNERS];
        for (which, corner) in corners.iter_mut().enumerate() {
            for (axis, (value, past)) in corner.iter_mut().zip(further).enumerate() {
                if (which >> axis) & 1 == 1 {
                    *value = past;
                }
            }
        }
        Ok(corners)
    }

    /// The cells of the block numbered `index` at the phase of `frame`, and
    /// the block's point in the lattice of blocks.
    pub fn block<A: Arith>(
        arith: &mut A,
        grid: Grid,
        index: A::Word,
        frame: A::Word,
    ) -> Result<Placed<A::Word, AXES, CORNERS>, A::Error> {
        let phase = low_bits(arith, frame, 1)?;
        let block = Self::split(arith, Self::blocks(grid), index)?;
        let points = Self::corners(arith, block, phase)?;
        let mut places = [index; CORNERS];
        for (place, point) in places.iter_mut().zip(points) {
            *place = Self::at(arith, Self::extents(grid), point)?;
        }
        Ok(Placed { block, places })
    }

    /// The block that holds the cell at `point` in the phase `phase`: its
    /// point in the lattice of blocks, its index, and which of its corners
    /// the cell is, numbered as in [`Self::corners`].
    pub fn holder<A: Arith>(
        arith: &mut A,
        grid: Grid,
        point: [A::Word; AXES],
        phase: A::Word,
    ) -> Result<Held<A::Word, AXES>, A::Error> {
        let minus = arith.splat(u32::MAX)?;
        let back = arith.mul(phase, minus)?;
        let mut shifted = point;
        for (value, extent) in shifted.iter_mut().zip(Self::extents(grid)) {
            *value = arith.add(*value, back)?;
            *value = low_bits(arith, *value, extent.trailing_zeros())?;
        }
        let mut corners = shifted;
        for value in &mut corners {
            *value = low_bits(arith, *value, 1)?;
        }
        let one = arith.splat(1)?;
        let mut block = shifted;
        for value in &mut block {
            *value = arith.shift_right(*value, one)?;
        }
        let index = Self::number(arith, block, Self::blocks(grid).map(u32::trailing_zeros))?;
        let corner = Self::number(arith, corners, [1; AXES])?;
        Ok(Held {
            block,
            index,
            corner,
        })
    }

    /// The offsets a changed block wakes, each axis in `0..3` and meaning one
    /// less, the first axis running fastest.
    #[must_use]
    pub fn near() -> [[u32; AXES]; NEAR] {
        let () = Self::SOUND;
        let mut out = [[0; AXES]; NEAR];
        for (number, offset) in (0_u32..).zip(out.iter_mut()) {
            let mut rest = number;
            for value in offset.iter_mut() {
                *value = rest % 3;
                rest /= 3;
            }
        }
        out
    }

    /// The index in the lattice of blocks of the block `offset` (from
    /// [`Self::near`]) away from `block`, wrapped onto the torus.
    pub fn neighbour<A: Arith>(
        arith: &mut A,
        grid: Grid,
        block: [A::Word; AXES],
        offset: [u32; AXES],
    ) -> Result<A::Word, A::Error> {
        let one = arith.splat(1)?;
        let minus = arith.splat(u32::MAX)?;
        let mut steps = block;
        for (step, value) in steps.iter_mut().zip(offset) {
            *step = arith.splat(value)?;
        }
        let back = arith.mul(one, minus)?;
        let mut near = block;
        for (value, step) in near.iter_mut().zip(steps) {
            *value = arith.add(*value, step)?;
            *value = arith.add(*value, back)?;
        }
        Self::at(arith, Self::blocks(grid), near)
    }

    /// The place of brush seat `seat` within a box `side` cells along every
    /// axis, `side` a power of two.
    pub fn seat<A: Arith>(
        arith: &mut A,
        side: u32,
        seat: A::Word,
    ) -> Result<[A::Word; AXES], A::Error> {
        let mut point = [seat; AXES];
        let mut rest = seat;
        for (axis, slot) in point.iter_mut().enumerate() {
            if axis + 1 == AXES {
                *slot = rest;
            } else {
                let by = arith.splat(side.trailing_zeros())?;
                *slot = low_bits(arith, rest, side.trailing_zeros())?;
                rest = arith.shift_right(rest, by)?;
            }
        }
        Ok(point)
    }

    /// The cell brush seat `seat` paints: its place in the box, with the box
    /// centred on `centre`, wrapped onto the torus axis by axis.
    pub fn seated<A: Arith>(
        arith: &mut A,
        grid: Grid,
        side: u32,
        seat: A::Word,
        centre: [A::Word; AXES],
    ) -> Result<[A::Word; AXES], A::Error> {
        let within = Self::seat(arith, side, seat)?;
        let half = arith.splat(side / 2)?;
        let minus = arith.splat(u32::MAX)?;
        let back = arith.mul(half, minus)?;
        let mut point = centre;
        for (value, inside) in point.iter_mut().zip(within) {
            *value = arith.add(*value, inside)?;
            *value = arith.add(*value, back)?;
        }
        for (value, extent) in point.iter_mut().zip(Self::extents(grid)) {
            *value = low_bits(arith, *value, extent.trailing_zeros())?;
        }
        Ok(point)
    }

    /// `point` less `centre`, axis by axis.
    pub fn apart<A: Arith>(
        arith: &mut A,
        point: [A::Word; AXES],
        centre: [A::Word; AXES],
    ) -> Result<[A::Word; AXES], A::Error> {
        let minus = arith.splat(u32::MAX)?;
        let mut away = centre;
        for value in &mut away {
            *value = arith.mul(*value, minus)?;
        }
        let mut apart = point;
        for (value, back) in apart.iter_mut().zip(away) {
            *value = arith.add(*value, back)?;
        }
        Ok(apart)
    }

    /// Whether a cell `apart` from the brush lies inside its disc, or its
    /// ball in 3D: the squared distance under `reach`.
    pub fn inside<A: Arith>(
        arith: &mut A,
        apart: [A::Word; AXES],
        reach: A::Word,
    ) -> Result<A::Truth, A::Error> {
        let mut squares = apart;
        for value in &mut squares {
            *value = arith.mul(*value, *value)?;
        }
        let mut each = squares.into_iter();
        let Some(mut far) = each.next() else {
            return arith.greater_than(reach, reach);
        };
        for square in each {
            far = arith.add(far, square)?;
        }
        arith.greater_than(reach, far)
    }

    /// Whether a cell `apart` from the brush lies inside the brush's box,
    /// `whole` cells along every axis and centred on the brush, one answer
    /// per axis; `half` holds half of `whole`. Distances are taken on the
    /// torus.
    pub fn boxed<A: Arith>(
        arith: &mut A,
        grid: Grid,
        apart: [A::Word; AXES],
        [half, whole]: [A::Word; 2],
    ) -> Result<Vec<A::Truth>, A::Error> {
        let mut from = apart;
        for (value, extent) in from.iter_mut().zip(Self::extents(grid)) {
            *value = arith.add(*value, half)?;
            *value = low_bits(arith, *value, extent.trailing_zeros())?;
        }
        let mut tests = Vec::with_capacity(AXES);
        for value in from {
            tests.push(arith.greater_than(whole, value)?);
        }
        Ok(tests)
    }

    /// The index of the cell at `point`, on the CPU.
    #[must_use]
    pub fn cell(grid: Grid, point: [u32; AXES]) -> u32 {
        plainly(Self::at(&mut Plain, Self::extents(grid), point))
    }

    /// The point of the cell numbered `cell`, on the CPU.
    #[must_use]
    pub fn point(grid: Grid, cell: u32) -> [u32; AXES] {
        plainly(Self::split(&mut Plain, Self::extents(grid), cell))
    }

    /// The cells of block `block` at the phase of `frame`, on the CPU.
    #[must_use]
    pub fn cells_of(grid: Grid, block: u32, frame: u32) -> [u32; CORNERS] {
        plainly(Self::block(&mut Plain, grid, block, frame)).places
    }

    /// The block and corner that hold the cell `cell` at the phase of
    /// `frame`, on the CPU.
    #[must_use]
    pub fn held(grid: Grid, cell: u32, frame: u32) -> Held<u32, AXES> {
        let point = Self::point(grid, cell);
        plainly(Self::holder(&mut Plain, grid, point, frame & 1))
    }

    /// The blocks a change in block `block` wakes, in the order the engine
    /// wakes them, on the CPU.
    #[must_use]
    pub fn woken(grid: Grid, block: u32) -> [u32; NEAR] {
        let mut arith = Plain;
        let point = plainly(Self::split(&mut arith, Self::blocks(grid), block));
        Self::near().map(|offset| plainly(Self::neighbour(&mut arith, grid, point, offset)))
    }

    /// The offsets of [`Self::near`] as steps of -1, 0 or 1.
    #[must_use]
    pub fn steps() -> [[i32; AXES]; NEAR] {
        Self::near().map(|offset| offset.map(|value| value.cast_signed() - 1))
    }

    /// The cell brush seat `seat` paints with the brush at `centre`, on the CPU.
    #[must_use]
    pub fn painted(grid: Grid, side: u32, seat: u32, centre: [u32; AXES]) -> [u32; AXES] {
        plainly(Self::seated(&mut Plain, grid, side, seat, centre))
    }

    /// Whether the cell at `point` lies in the brush's disc or ball, on the CPU.
    #[must_use]
    pub fn reached(point: [u32; AXES], centre: [u32; AXES], reach: u32) -> bool {
        let mut arith = Plain;
        let apart = plainly(Self::apart(&mut arith, point, centre));
        plainly(Self::inside(&mut arith, apart, reach))
    }
}
