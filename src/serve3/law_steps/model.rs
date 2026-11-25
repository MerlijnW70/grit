use crate::stencil::{Border, Stencil};

use super::super::law_real::Term;

pub const NONE: u32 = u32::MAX;

#[derive(Clone, Debug)]
pub struct Basis {
    pub stencil: Stencil,
    pub places: Vec<u32>,
    pub centre: usize,
    pub observed: usize,
    pub hidden: usize,
    pub terms: Vec<Term>,
}

impl Basis {
    #[must_use]
    pub fn new(stencil: Stencil, mask: u32, observed: usize, hidden: usize, poly: u32) -> Self {
        let places: Vec<u32> = (0..stencil.positions())
            .filter(|place| (mask >> place) & 1 == 1)
            .collect();
        let centre_place = stencil.place([0, 0, 0]);
        let centre = places
            .iter()
            .position(|one| *one == centre_place)
            .unwrap_or(0);
        let reads = observed + hidden;
        let mut terms = Vec::new();
        for read in 0..observed {
            for place in &places {
                terms.push(Term::Place {
                    read,
                    place: *place,
                });
            }
        }
        for read in observed..reads {
            terms.push(Term::Place {
                read,
                place: centre_place,
            });
        }
        if poly >= 2 {
            for left in 0..observed {
                for right in left..observed {
                    terms.push(Term::Pair { left, right });
                }
            }
            for left in 0..observed {
                for right in observed..reads {
                    terms.push(Term::Pair { left, right });
                }
            }
            if places.len() > 1 {
                for centre in 0..observed {
                    for around in 0..observed {
                        terms.push(Term::Spread { centre, around });
                    }
                }
            }
        }
        if poly >= 3 {
            for left in 0..observed {
                for right in left..observed {
                    for third in right..observed {
                        terms.push(Term::Cube { left, right, third });
                    }
                }
            }
        }
        Self {
            stencil,
            places,
            centre,
            observed,
            hidden,
            terms,
        }
    }

    #[must_use]
    pub const fn reads(&self) -> usize {
        self.observed + self.hidden
    }

    #[must_use]
    pub fn params(&self) -> usize {
        self.reads() * (self.terms.len() + 1)
    }

    #[must_use]
    pub fn coefficient(&self, write: usize, term: usize) -> usize {
        write * (self.terms.len() + 1) + term
    }

    #[must_use]
    pub fn bias(&self, write: usize) -> usize {
        write * (self.terms.len() + 1) + self.terms.len()
    }

    #[must_use]
    pub fn place_at(&self, place: u32) -> usize {
        self.places
            .iter()
            .position(|one| *one == place)
            .unwrap_or(self.centre)
    }

    #[must_use]
    pub fn hidden_power(&self, term: usize, hidden: usize) -> u32 {
        let plane = self.observed + hidden;
        let count = |one: usize| u32::from(one == plane);
        match self.terms.get(term) {
            Some(Term::Place { read, .. }) => count(*read),
            Some(Term::Pair { left, right }) => count(*left) + count(*right),
            Some(Term::Spread { centre, around }) => count(*centre) + count(*around),
            Some(Term::Cube { left, right, third }) => count(*left) + count(*right) + count(*third),
            None => 0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Grid {
    pub size: [u32; 3],
    pub cells: usize,
    pub near: Vec<u32>,
    pub width: usize,
}

impl Grid {
    #[must_use]
    pub fn new(size: [u32; 3], basis: &Basis, border: Border) -> Self {
        let cells = size.iter().map(|one| *one as usize).product::<usize>();
        let width = basis.places.len();
        let mut near = Vec::with_capacity(cells * width);
        for z in 0..size[2] {
            for y in 0..size[1] {
                for x in 0..size[0] {
                    for place in &basis.places {
                        let offset = basis.stencil.offset(*place);
                        let at = [
                            i64::from(x) + offset[0],
                            i64::from(y) + offset[1],
                            i64::from(z) + offset[2],
                        ];
                        near.push(spot(size, at, border));
                    }
                }
            }
        }
        Self {
            size,
            cells,
            near,
            width,
        }
    }

    #[must_use]
    pub fn near(&self, cell: usize, at: usize) -> u32 {
        self.near
            .get(cell * self.width + at)
            .copied()
            .unwrap_or(NONE)
    }
}

fn spot(size: [u32; 3], at: [i64; 3], border: Border) -> u32 {
    let mut index = 0_i64;
    let mut stride = 1_i64;
    for (value, side) in at.iter().zip(size) {
        let side = i64::from(side.max(1));
        let inside = if (0..side).contains(value) {
            *value
        } else if border == Border::Wrapped {
            value.rem_euclid(side)
        } else {
            return NONE;
        };
        index += inside * stride;
        stride *= side;
    }
    u32::try_from(index).unwrap_or(NONE)
}

pub type Planes = Vec<Vec<f64>>;

pub struct Law<'a> {
    pub basis: &'a Basis,
    pub theta: &'a [f64],
    pub limits: &'a [[f64; 2]],
}

fn read(planes: &[Vec<f64>], plane: usize, cell: u32) -> f64 {
    if cell == NONE {
        return 0.0;
    }
    planes
        .get(plane)
        .and_then(|one| one.get(cell as usize))
        .copied()
        .unwrap_or(0.0)
}

fn tangent(tangents: &[Vec<f64>], plane: usize, cell: u32, count: usize) -> Option<&[f64]> {
    if cell == NONE {
        return None;
    }
    let from = cell as usize * count;
    tangents
        .get(plane)
        .and_then(|one| one.get(from..from + count))
}

fn add_scaled(out: &mut [f64], from: Option<&[f64]>, by: f64) {
    if let Some(from) = from {
        for (slot, one) in out.iter_mut().zip(from) {
            *slot += by * one;
        }
    }
}

impl Law<'_> {
    fn coefficient(&self, write: usize, term: usize) -> f64 {
        self.theta
            .get(self.basis.coefficient(write, term))
            .copied()
            .unwrap_or(0.0)
    }

    fn bias(&self, write: usize) -> f64 {
        self.theta
            .get(self.basis.bias(write))
            .copied()
            .unwrap_or(0.0)
    }

    pub fn values(&self, grid: &Grid, planes: &[Vec<f64>], cell: usize, out: &mut Vec<f64>) {
        out.clear();
        let basis = self.basis;
        let centre = |plane: usize| read(planes, plane, u32::try_from(cell).unwrap_or(NONE));
        for term in &basis.terms {
            let value = match *term {
                Term::Place { read: plane, place } => {
                    read(planes, plane, grid.near(cell, basis.place_at(place)))
                }
                Term::Pair { left, right } => centre(left) * centre(right),
                Term::Spread {
                    centre: one,
                    around,
                } => {
                    let sum: f64 = (0..grid.width)
                        .filter(|at| *at != basis.centre)
                        .map(|at| read(planes, around, grid.near(cell, at)))
                        .sum();
                    centre(one) * sum
                }
                Term::Cube { left, right, third } => centre(left) * centre(right) * centre(third),
            };
            out.push(value);
        }
    }

    fn slopes(
        &self,
        grid: &Grid,
        planes: &[Vec<f64>],
        tangents: &[Vec<f64>],
        cell: usize,
        count: usize,
        out: &mut [f64],
    ) {
        out.iter_mut().for_each(|one| *one = 0.0);
        let basis = self.basis;
        let own = u32::try_from(cell).unwrap_or(NONE);
        let centre = |plane: usize| read(planes, plane, own);
        let slope = |plane: usize| tangent(tangents, plane, own, count);
        for (term, row) in basis.terms.iter().zip(out.chunks_mut(count)) {
            match *term {
                Term::Place { read: plane, place } => {
                    add_scaled(
                        row,
                        tangent(
                            tangents,
                            plane,
                            grid.near(cell, basis.place_at(place)),
                            count,
                        ),
                        1.0,
                    );
                }
                Term::Pair { left, right } => {
                    add_scaled(row, slope(left), centre(right));
                    add_scaled(row, slope(right), centre(left));
                }
                Term::Spread {
                    centre: one,
                    around,
                } => {
                    let mut sum = 0.0;
                    for at in (0..grid.width).filter(|at| *at != basis.centre) {
                        let near = grid.near(cell, at);
                        sum += read(planes, around, near);
                        add_scaled(row, tangent(tangents, around, near, count), centre(one));
                    }
                    add_scaled(row, slope(one), sum);
                }
                Term::Cube { left, right, third } => {
                    add_scaled(row, slope(left), centre(right) * centre(third));
                    add_scaled(row, slope(right), centre(left) * centre(third));
                    add_scaled(row, slope(third), centre(left) * centre(right));
                }
            }
        }
    }

    fn part(
        &self,
        grid: &Grid,
        planes: &[Vec<f64>],
        tangents: Option<&[Vec<f64>]>,
        writes: &[usize],
        cells: std::ops::Range<usize>,
    ) -> (Planes, Option<Planes>) {
        let count = self.basis.params();
        let terms = self.basis.terms.len();
        let span = cells.len();
        let mut out: Planes = vec![vec![0.0; span]; writes.len()];
        let mut moved: Option<Planes> =
            tangents.map(|_| vec![vec![0.0; span * count]; writes.len()]);
        let mut values = Vec::with_capacity(terms);
        let mut rows = vec![0.0; if tangents.is_some() { terms * count } else { 0 }];
        for (local, cell) in cells.enumerate() {
            self.values(grid, planes, cell, &mut values);
            if let Some(tangents) = tangents {
                self.slopes(grid, planes, tangents, cell, count, &mut rows);
            }
            for (slot_at, &write) in writes.iter().enumerate() {
                let mut sum = self.bias(write);
                for (term, value) in values.iter().enumerate() {
                    sum += self.coefficient(write, term) * value;
                }
                let [least, most] = self
                    .limits
                    .get(write)
                    .copied()
                    .unwrap_or([f64::MIN, f64::MAX]);
                let clamped = !(least..=most).contains(&sum);
                if let Some(slot) = out.get_mut(slot_at).and_then(|one| one.get_mut(local)) {
                    *slot = sum.clamp(least, most);
                }
                if clamped {
                    continue;
                }
                if let Some(slot) = moved
                    .as_mut()
                    .and_then(|one| one.get_mut(slot_at))
                    .and_then(|one| one.get_mut(local * count..(local + 1) * count))
                {
                    for (term, row) in rows.chunks(count).enumerate() {
                        add_scaled(slot, Some(row), self.coefficient(write, term));
                        if let (Some(one), Some(value)) = (
                            slot.get_mut(self.basis.coefficient(write, term)),
                            values.get(term),
                        ) {
                            *one += value;
                        }
                    }
                    if let Some(one) = slot.get_mut(self.basis.bias(write)) {
                        *one += 1.0;
                    }
                }
            }
        }
        (out, moved)
    }

    pub fn advance(
        &self,
        grid: &Grid,
        planes: &[Vec<f64>],
        tangents: Option<&[Vec<f64>]>,
        writes: &[usize],
    ) -> (Planes, Option<Planes>) {
        let count = self.basis.params();
        let threads = std::thread::available_parallelism()
            .map_or(1, std::num::NonZeroUsize::get)
            .clamp(1, THREADS_MOST);
        let pieces = if grid.cells >= PIECE_LEAST * 2 {
            threads.min(grid.cells / PIECE_LEAST)
        } else {
            1
        };
        let each = grid.cells.div_ceil(pieces.max(1));
        let ranges: Vec<std::ops::Range<usize>> = (0..pieces)
            .map(|piece| piece * each..((piece + 1) * each).min(grid.cells))
            .filter(|range| !range.is_empty())
            .collect();
        let parts: Vec<(Planes, Option<Planes>)> = if ranges.len() > 1 {
            std::thread::scope(|scope| {
                let handles: Vec<_> = ranges
                    .iter()
                    .map(|range| {
                        let range = range.clone();
                        scope.spawn(move || self.part(grid, planes, tangents, writes, range))
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|one| {
                        one.join()
                            .unwrap_or_else(|_| (Vec::new(), tangents.map(|_| Vec::new())))
                    })
                    .collect()
            })
        } else {
            ranges
                .iter()
                .map(|range| self.part(grid, planes, tangents, writes, range.clone()))
                .collect()
        };
        let mut out: Planes = planes.to_vec();
        let mut moved: Option<Planes> = tangents.map(<[Vec<f64>]>::to_vec);
        for (range, (values, slopes)) in ranges.iter().zip(parts) {
            for (slot_at, &write) in writes.iter().enumerate() {
                if let (Some(target), Some(source)) = (
                    out.get_mut(write)
                        .and_then(|one| one.get_mut(range.clone())),
                    values.get(slot_at),
                ) {
                    target.copy_from_slice(source);
                }
                if let (Some(target), Some(source)) = (
                    moved
                        .as_mut()
                        .and_then(|one| one.get_mut(write))
                        .and_then(|one| one.get_mut(range.start * count..range.end * count)),
                    slopes.as_ref().and_then(|one| one.get(slot_at)),
                ) {
                    target.copy_from_slice(source);
                }
            }
        }
        (out, moved)
    }
}

pub const THREADS_MOST: usize = 8;

pub const PIECE_LEAST: usize = 512;

fn give(into: &mut [Vec<f64>], plane: usize, cell: u32, value: f64) {
    if cell == NONE || value == 0.0 {
        return;
    }
    if let Some(slot) = into
        .get_mut(plane)
        .and_then(|one| one.get_mut(cell as usize))
    {
        *slot += value;
    }
}

impl Law<'_> {
    pub fn back(
        &self,
        grid: &Grid,
        planes: &[Vec<f64>],
        writes: &[usize],
        given: &[Vec<f64>],
    ) -> Planes {
        let basis = self.basis;
        let mut into: Planes = given.to_vec();
        for &write in writes {
            if let Some(plane) = into.get_mut(write) {
                plane.iter_mut().for_each(|one| *one = 0.0);
            }
        }
        let mut values = Vec::with_capacity(basis.terms.len());
        for cell in 0..grid.cells {
            let own = u32::try_from(cell).unwrap_or(NONE);
            let mut weights = vec![0.0; basis.terms.len()];
            let mut any = false;
            self.values(grid, planes, cell, &mut values);
            for &write in writes {
                let weight = given
                    .get(write)
                    .and_then(|one| one.get(cell))
                    .copied()
                    .unwrap_or(0.0);
                if weight == 0.0 {
                    continue;
                }
                let mut sum = self.bias(write);
                for (term, value) in values.iter().enumerate() {
                    sum += self.coefficient(write, term) * value;
                }
                let [least, most] = self
                    .limits
                    .get(write)
                    .copied()
                    .unwrap_or([f64::MIN, f64::MAX]);
                if !(least..=most).contains(&sum) {
                    continue;
                }
                for (term, slot) in weights.iter_mut().enumerate() {
                    *slot += weight * self.coefficient(write, term);
                }
                any = true;
            }
            if !any {
                continue;
            }
            let centre = |plane: usize| read(planes, plane, own);
            for (term, weight) in basis.terms.iter().zip(&weights) {
                let weight = *weight;
                if weight == 0.0 {
                    continue;
                }
                match *term {
                    Term::Place { read: plane, place } => {
                        give(
                            &mut into,
                            plane,
                            grid.near(cell, basis.place_at(place)),
                            weight,
                        );
                    }
                    Term::Pair { left, right } => {
                        give(&mut into, left, own, weight * centre(right));
                        give(&mut into, right, own, weight * centre(left));
                    }
                    Term::Spread {
                        centre: one,
                        around,
                    } => {
                        let mut sum = 0.0;
                        for at in (0..grid.width).filter(|at| *at != basis.centre) {
                            let near = grid.near(cell, at);
                            sum += read(planes, around, near);
                            give(&mut into, around, near, weight * centre(one));
                        }
                        give(&mut into, one, own, weight * sum);
                    }
                    Term::Cube { left, right, third } => {
                        give(&mut into, left, own, weight * centre(right) * centre(third));
                        give(&mut into, right, own, weight * centre(left) * centre(third));
                        give(&mut into, third, own, weight * centre(left) * centre(right));
                    }
                }
            }
        }
        into
    }
}
