use crate::geometry::{Arith, Solid, low_bits};
use crate::rule3::{CORNERS, MOST_PLANES};
use crate::sand::Grid;
use crate::sand3::Grid3;

pub const RADIUS: i32 = 1;

pub const SIDE: usize = 4;

pub const BOX: usize = SIDE * SIDE * SIDE;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Halo {
    cells: [u64; MOST_PLANES],
}

impl Halo {
    pub const NONE: Self = Self {
        cells: [0; MOST_PLANES],
    };

    #[must_use]
    pub fn is_empty(self) -> bool {
        self.cells.iter().all(|&held| held == 0)
    }

    #[must_use]
    pub fn with(mut self, plane: usize, cell: usize) -> Self {
        if let Some(held) = self.cells.get_mut(plane)
            && cell < BOX
        {
            *held |= 1 << cell;
        }
        self
    }

    #[must_use]
    pub fn cells(self, plane: usize) -> u64 {
        self.cells.get(plane).copied().unwrap_or(0)
    }

    #[must_use]
    pub fn looked(self, plane: usize) -> bool {
        self.cells(plane) != 0
    }

    #[must_use]
    pub fn planes(self) -> usize {
        self.cells
            .iter()
            .rposition(|&held| held != 0)
            .map_or(0, |last| last + 1)
    }

    pub fn each(self) -> impl Iterator<Item = (usize, usize)> {
        (0..MOST_PLANES).flat_map(move |plane| {
            (0..BOX)
                .filter(move |&cell| (self.cells(plane) >> cell) & 1 == 1)
                .map(move |cell| (plane, cell))
        })
    }

    #[must_use]
    pub fn loads(self) -> usize {
        self.cells
            .iter()
            .map(|held| held.count_ones() as usize)
            .sum()
    }
}

#[must_use]
pub const fn cell(point: [usize; 3]) -> usize {
    point[0] + SIDE * (point[1] + SIDE * point[2])
}

#[must_use]
pub const fn point(cell: usize) -> [usize; 3] {
    [cell % SIDE, (cell / SIDE) % SIDE, cell / (SIDE * SIDE)]
}

#[must_use]
pub const fn corner_point(corner: usize) -> [usize; 3] {
    [corner & 1, corner >> 2, (corner >> 1) & 1]
}

#[must_use]
pub fn looked(corner: usize, offset: [i32; 3]) -> Option<usize> {
    if corner >= CORNERS || offset.iter().any(|step| step.abs() > RADIUS) {
        return None;
    }
    let mut at = [0_usize; 3];
    for ((slot, from), step) in at.iter_mut().zip(corner_point(corner)).zip(offset) {
        *slot = usize::try_from(i32::try_from(from).ok()? + 1 + step).ok()?;
    }
    Some(cell(at))
}

#[must_use]
pub fn own(cell: usize) -> Option<usize> {
    let [x, y, z] = point(cell);
    let inside = |value: usize| (1..=2).contains(&value);
    (cell < BOX && inside(x) && inside(y) && inside(z)).then(|| (x - 1) + 2 * (z - 1) + 4 * (y - 1))
}

#[must_use]
pub fn boxed(grid: Grid3, frame: u32, oct: u32, plane: &[u32]) -> Option<[u32; BOX]> {
    let [bx, by, bz] = crate::sand3::lattice(grid, oct);
    let phase = frame & 1;
    let least = [bx * 2 + phase, by * 2 + phase, bz * 2 + phase];
    let mut out = [0_u32; BOX];
    for (at, slot) in out.iter_mut().enumerate() {
        let [x, y, z] = point(at).map(|value| u32::try_from(value).unwrap_or(0));
        let place = grid.index(
            least[0].wrapping_add(x).wrapping_sub(1),
            least[1].wrapping_add(y).wrapping_sub(1),
            least[2].wrapping_add(z).wrapping_sub(1),
        );
        *slot = *plane.get(place)?;
    }
    Some(out)
}

pub fn places<A: Arith>(
    arith: &mut A,
    grid: Grid,
    block: [A::Word; 3],
    frame: A::Word,
    halo: Halo,
) -> Result<Vec<(usize, A::Word)>, A::Error> {
    let phase = low_bits(arith, frame, 1)?;
    let one = arith.splat(1)?;
    let back = arith.splat(u32::MAX)?;
    let mut origin = block;
    for value in &mut origin {
        *value = arith.shift_left(*value, one)?;
        *value = arith.add(*value, phase)?;
        *value = arith.add(*value, back)?;
    }
    let mut wanted = [false; BOX];
    for (_, at) in halo.each() {
        if let Some(slot) = wanted.get_mut(at) {
            *slot = true;
        }
    }
    let mut out = Vec::new();
    for (at, _) in wanted.iter().enumerate().filter(|(_, wanted)| **wanted) {
        let mut here = origin;
        for (value, step) in here.iter_mut().zip(point(at)) {
            let step = arith.splat(u32::try_from(step).unwrap_or(0))?;
            *value = arith.add(*value, step)?;
        }
        out.push((at, Solid::at(arith, Solid::extents(grid), here)?));
    }
    Ok(out)
}
