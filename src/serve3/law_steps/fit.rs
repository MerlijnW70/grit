use super::super::budget::Stop;
use super::super::law_real::Term;
use super::model::{Basis, Grid, Law, Planes};

pub trait Drive: Send + Sync {
    fn put(&self, sub: usize, planes: &mut [Vec<f64>]);

    fn driven(&self, read: usize, cell: usize) -> bool;
}

pub struct Rewrite {
    pub read: usize,
    pub cells: Vec<usize>,
    pub values: Vec<Vec<f64>>,
    pub mask: Vec<bool>,
}

impl Rewrite {
    #[must_use]
    pub fn new(read: usize, cells: Vec<usize>, values: Vec<Vec<f64>>, count: usize) -> Self {
        let mut mask = vec![false; count];
        for cell in &cells {
            if let Some(slot) = mask.get_mut(*cell) {
                *slot = true;
            }
        }
        Self {
            read,
            cells,
            values,
            mask,
        }
    }
}

impl Drive for Rewrite {
    fn put(&self, sub: usize, planes: &mut [Vec<f64>]) {
        let (Some(row), Some(plane)) = (self.values.get(sub), planes.get_mut(self.read)) else {
            return;
        };
        for (cell, value) in self.cells.iter().zip(row) {
            if let Some(slot) = plane.get_mut(*cell) {
                *slot = *value;
            }
        }
    }

    fn driven(&self, read: usize, cell: usize) -> bool {
        read == self.read && self.mask.get(cell).copied().unwrap_or(false)
    }
}

pub struct Clip {
    pub grid: Grid,
    pub shots: Vec<Planes>,
    pub drive: Option<Box<dyn Drive>>,
}

impl Clip {
    pub fn put(&self, sub: usize, planes: &mut [Vec<f64>]) {
        if let Some(drive) = &self.drive {
            drive.put(sub, planes);
        }
    }

    #[must_use]
    pub fn scored(&self, read: usize, cell: usize) -> bool {
        self.drive
            .as_ref()
            .is_none_or(|drive| !drive.driven(read, cell))
    }

    pub fn still(&self, planes: &mut [Vec<f64>], width: usize, reads: usize) {
        let Some(drive) = &self.drive else {
            return;
        };
        for (read, plane) in planes.iter_mut().enumerate().take(reads) {
            for (cell, row) in plane.chunks_mut(width.max(1)).enumerate() {
                if drive.driven(read, cell) {
                    row.iter_mut().for_each(|one| *one = 0.0);
                }
            }
        }
    }
}

pub struct Setup<'a> {
    pub basis: &'a Basis,
    pub clips: &'a [Clip],
    pub steps: u32,
    pub limits: Vec<[f64; 2]>,
    pub settle: usize,
}

#[derive(Clone, Debug, Default)]
pub struct Normal {
    pub cost: f64,
    pub count: u64,
    pub square: Vec<f64>,
    pub toward: Vec<f64>,
}

impl Normal {
    fn new(params: usize, wanted: bool) -> Self {
        Self {
            cost: 0.0,
            count: 0,
            square: vec![0.0; if wanted { params * params } else { 0 }],
            toward: vec![0.0; if wanted { params } else { 0 }],
        }
    }

    fn take(&mut self, missed: f64, row: Option<&[f64]>) {
        self.cost += missed * missed;
        self.count += 1;
        let Some(row) = row else {
            return;
        };
        let params = row.len();
        for (left, one) in row.iter().enumerate() {
            if *one == 0.0 {
                continue;
            }
            if let Some(slot) = self.toward.get_mut(left) {
                *slot += missed * one;
            }
            if let Some(line) = self.square.get_mut(left * params..(left + 1) * params) {
                for (slot, other) in line.iter_mut().zip(row) {
                    *slot += one * other;
                }
            }
        }
    }
}

pub struct Run<'a> {
    pub setup: &'a Setup<'a>,
    pub work: u64,
    pub starts: Vec<Vec<Planes>>,
}

fn interpolated(from: &Planes, to: &Planes, share: f64) -> Planes {
    from.iter()
        .zip(to)
        .map(|(one, other)| {
            one.iter()
                .zip(other)
                .map(|(left, right)| left + (right - left) * share)
                .collect()
        })
        .collect()
}

impl Run<'_> {
    fn law<'t>(&'t self, theta: &'t [f64]) -> Law<'t> {
        Law {
            basis: self.setup.basis,
            theta,
            limits: &self.setup.limits,
        }
    }
}

impl<'a> Run<'a> {
    #[must_use]
    pub const fn new(setup: &'a Setup<'a>) -> Self {
        Self {
            setup,
            work: 0,
            starts: Vec::new(),
        }
    }

    fn count(&mut self, cells: usize, slopes: bool) {
        let each = if slopes {
            1 + self.setup.basis.params()
        } else {
            1
        };
        self.work = self
            .work
            .saturating_add(u64::try_from(cells.saturating_mul(each)).unwrap_or(u64::MAX));
    }

    pub fn observed(
        &mut self,
        theta: &[f64],
        clip: &Clip,
        slopes: bool,
    ) -> Vec<(Planes, Option<Planes>)> {
        let basis = self.setup.basis;
        let count = basis.params();
        let cells = clip.grid.cells;
        let hidden: Vec<usize> = (basis.observed..basis.reads()).collect();
        let zero = vec![0.0; cells];
        let flat = vec![0.0; cells * count];
        let mut held: Planes = vec![zero.clone(); basis.hidden];
        let mut moved: Option<Planes> = slopes.then(|| vec![flat.clone(); basis.hidden]);
        let mut out = Vec::with_capacity(clip.shots.len());
        out.push((held.clone(), moved.clone()));
        let steps = self.setup.steps as usize;
        for (pair_at, pair) in clip.shots.windows(2).enumerate() {
            let [from, to] = pair else {
                continue;
            };
            for sub in 0..self.setup.steps {
                let share = f64::from(sub) / f64::from(self.setup.steps.max(1));
                let mut planes = interpolated(from, to, share);
                clip.put(pair_at * steps + sub as usize, &mut planes);
                planes.extend(held.iter().cloned());
                let tangents: Option<Planes> = moved.as_ref().map(|one| {
                    let mut all = vec![flat.clone(); basis.observed];
                    all.extend(one.iter().cloned());
                    all
                });
                self.count(cells, slopes);
                let (next, next_moved) =
                    self.law(theta)
                        .advance(&clip.grid, &planes, tangents.as_deref(), &hidden);
                held = next.into_iter().skip(basis.observed).collect();
                moved = next_moved.map(|one| one.into_iter().skip(basis.observed).collect());
            }
            out.push((held.clone(), moved.clone()));
        }
        out
    }

    pub fn begins(&self, clip: &Clip, horizon: usize) -> Vec<usize> {
        let last = clip.shots.len().saturating_sub(1);
        (self.setup.settle..last).step_by(horizon.max(1)).collect()
    }

    pub fn reset(&mut self, horizon: usize) {
        self.starts = self
            .setup
            .clips
            .iter()
            .map(|clip| {
                self.begins(clip, horizon)
                    .into_iter()
                    .map(|begin| clip.shots.get(begin).cloned().unwrap_or_default())
                    .collect()
            })
            .collect();
    }

    pub fn normal(&mut self, theta: &[f64], horizon: usize, slopes: bool) -> Normal {
        let basis = self.setup.basis;
        let count = basis.params();
        let all: Vec<usize> = (0..basis.reads()).collect();
        let mut normal = Normal::new(count, slopes);
        for (index, clip) in self.setup.clips.iter().enumerate() {
            let cells = clip.grid.cells;
            let watched = self.observed(theta, clip, slopes);
            for (segment, begin) in self.begins(clip, horizon).into_iter().enumerate() {
                let (Some(shot), Some((hidden, hidden_moved))) =
                    (clip.shots.get(begin), watched.get(begin))
                else {
                    continue;
                };
                let start: Planes = self
                    .starts
                    .get(index)
                    .and_then(|one| one.get(segment))
                    .cloned()
                    .unwrap_or_else(|| shot.clone());
                for (read, (now, target)) in start.iter().zip(shot).enumerate() {
                    for (cell, (one, other)) in now.iter().zip(target).enumerate() {
                        if clip.scored(read, cell) {
                            normal.take(one - other, None);
                        }
                    }
                }
                let steps = self.setup.steps as usize;
                let mut planes: Planes = start;
                planes.extend(hidden.iter().cloned());
                let mut tangents: Option<Planes> = hidden_moved.as_ref().map(|one| {
                    let mut all_moved = vec![vec![0.0; cells * count]; basis.observed];
                    all_moved.extend(one.iter().cloned());
                    all_moved
                });
                for ahead in 1..=horizon {
                    let Some(wanted) = clip.shots.get(begin + ahead) else {
                        break;
                    };
                    for sub in 0..steps {
                        clip.put((begin + ahead - 1) * steps + sub, &mut planes);
                        if let Some(moving) = tangents.as_mut() {
                            clip.still(moving, count, basis.observed);
                        }
                        self.count(cells, slopes);
                        let (next, next_moved) =
                            self.law(theta)
                                .advance(&clip.grid, &planes, tangents.as_deref(), &all);
                        planes = next;
                        tangents = next_moved;
                    }
                    for (read, (now, target)) in planes.iter().zip(wanted).enumerate() {
                        for (cell, (one, other)) in now.iter().zip(target).enumerate() {
                            if !clip.scored(read, cell) {
                                continue;
                            }
                            let row = tangents
                                .as_ref()
                                .and_then(|all_moved| all_moved.get(read))
                                .and_then(|plane| plane.get(cell * count..(cell + 1) * count));
                            normal.take(one - other, row);
                        }
                    }
                }
            }
        }
        normal
    }

    fn segment_cost(
        &mut self,
        theta: &[f64],
        piece: &Piece<'_>,
        start: &Planes,
        slope: bool,
    ) -> (f64, Planes) {
        let basis = self.setup.basis;
        let clip = piece.clip;
        let all: Vec<usize> = (0..basis.reads()).collect();
        let cells = clip.grid.cells;
        let mut cost = 0.0;
        let mut planes: Planes = start.clone();
        planes.extend(piece.hidden.iter().cloned());
        let mut held: Vec<Planes> = Vec::new();
        let mut targets: Vec<(usize, &Planes)> = Vec::new();
        let steps = self.setup.steps as usize;
        let base = piece.begin * steps;
        let mut index = 0_usize;
        for ahead in 1..=piece.horizon {
            let Some(wanted) = clip.shots.get(piece.begin + ahead) else {
                break;
            };
            for _ in 0..steps {
                self.count(cells, false);
                clip.put(base + index, &mut planes);
                if slope {
                    held.push(planes.clone());
                }
                planes = self.law(theta).advance(&clip.grid, &planes, None, &all).0;
                index += 1;
            }
            for (read, (now, target)) in planes.iter().zip(wanted).enumerate() {
                for (cell, (one, other)) in now.iter().zip(target).enumerate() {
                    if clip.scored(read, cell) {
                        cost += (one - other) * (one - other);
                    }
                }
            }
            targets.push((index, wanted));
        }
        let shot = clip.shots.get(piece.begin);
        if let Some(shot) = shot {
            for (read, (now, target)) in start.iter().zip(shot).enumerate() {
                for (cell, (one, other)) in now.iter().zip(target).enumerate() {
                    if clip.scored(read, cell) {
                        cost += (one - other) * (one - other);
                    }
                }
            }
        }
        if !slope {
            return (cost, Vec::new());
        }
        let mut given: Planes = vec![vec![0.0; cells]; basis.reads()];
        let mut current = planes;
        for sub in (0..held.len()).rev() {
            for (at, wanted) in &targets {
                if *at != sub + 1 {
                    continue;
                }
                for (read, (now, target)) in current.iter().zip(*wanted).enumerate() {
                    if let Some(slot) = given.get_mut(read) {
                        for (cell, ((add, one), other)) in
                            slot.iter_mut().zip(now).zip(target).enumerate()
                        {
                            if clip.scored(read, cell) {
                                *add += 2.0 * (one - other);
                            }
                        }
                    }
                }
            }
            let Some(before) = held.get(sub) else {
                continue;
            };
            self.count(cells, false);
            given = self.law(theta).back(&clip.grid, before, &all, &given);
            clip.still(&mut given, 1, basis.observed);
            current.clone_from(before);
        }
        let mut gradient: Planes = given.into_iter().take(basis.observed).collect();
        if let Some(shot) = shot {
            for (read, ((plane, now), target)) in
                gradient.iter_mut().zip(start).zip(shot).enumerate()
            {
                for (cell, ((add, one), other)) in plane.iter_mut().zip(now).zip(target).enumerate()
                {
                    if clip.scored(read, cell) {
                        *add += 2.0 * (one - other);
                    }
                }
            }
        }
        (cost, gradient)
    }

    pub fn refine(&mut self, theta: &[f64], horizon: usize) {
        let clips = self.setup.clips;
        for (index, clip) in clips.iter().enumerate() {
            let watched = self.observed(theta, clip, false);
            for (segment, begin) in self.begins(clip, horizon).into_iter().enumerate() {
                let Some((hidden, _)) = watched.get(begin) else {
                    continue;
                };
                let Some(start) = self
                    .starts
                    .get(index)
                    .and_then(|one| one.get(segment))
                    .cloned()
                else {
                    continue;
                };
                let piece = Piece {
                    clip,
                    begin,
                    horizon,
                    rounds: START_ROUNDS,
                    hidden: hidden.clone(),
                };
                let found = self.settled(theta, &piece, &start);
                if let Some(slot) = self
                    .starts
                    .get_mut(index)
                    .and_then(|one| one.get_mut(segment))
                {
                    *slot = found;
                }
            }
        }
    }

    fn settled(&mut self, theta: &[f64], piece: &Piece<'_>, start: &Planes) -> Planes {
        let shape: Vec<usize> = start.iter().map(Vec::len).collect();
        let mut point = flat(start);
        let mut memory: Vec<(Vec<f64>, Vec<f64>)> = Vec::new();
        let (mut cost, gradient) = self.segment_cost(theta, piece, start, true);
        let mut gradient = flat(&gradient);
        for _ in 0..piece.rounds {
            let direction = two_loop(&gradient, &memory);
            let along = dot(&direction, &gradient);
            if along >= 0.0 {
                break;
            }
            let mut length = 1.0;
            let mut taken = None;
            for _ in 0..START_HALVINGS {
                let trial: Vec<f64> = point
                    .iter()
                    .zip(&direction)
                    .map(|(one, by)| one + length * by)
                    .collect();
                let (trial_cost, _) =
                    self.segment_cost(theta, piece, &unflat(&trial, &shape), false);
                if trial_cost.is_finite() && trial_cost <= cost + 1e-4 * length * along {
                    taken = Some((trial, trial_cost));
                    break;
                }
                length *= 0.5;
            }
            let Some((trial, trial_cost)) = taken else {
                break;
            };
            let (_, next) = self.segment_cost(theta, piece, &unflat(&trial, &shape), true);
            let next = flat(&next);
            let moved: Vec<f64> = trial
                .iter()
                .zip(&point)
                .map(|(one, other)| one - other)
                .collect();
            let turned: Vec<f64> = next
                .iter()
                .zip(&gradient)
                .map(|(one, other)| one - other)
                .collect();
            if dot(&moved, &turned) > 0.0 {
                memory.push((moved, turned));
                if memory.len() > START_MEMORY {
                    memory.remove(0);
                }
            }
            let gain = (cost - trial_cost) / cost.max(f64::MIN_POSITIVE);
            point = trial;
            gradient = next;
            cost = trial_cost;
            if gain < SETTLED {
                break;
            }
        }
        unflat(&point, &shape)
    }
}

struct Piece<'c> {
    clip: &'c Clip,
    begin: usize,
    horizon: usize,
    rounds: u32,
    hidden: Planes,
}

pub const START_ROUNDS: u32 = 8;

pub const START_HALVINGS: u32 = 20;

pub const START_MEMORY: usize = 5;

fn dot(left: &[f64], right: &[f64]) -> f64 {
    left.iter().zip(right).map(|(one, other)| one * other).sum()
}

fn flat(planes: &Planes) -> Vec<f64> {
    planes.iter().flatten().copied().collect()
}

fn unflat(point: &[f64], shape: &[usize]) -> Planes {
    let mut out = Vec::with_capacity(shape.len());
    let mut from = 0;
    for length in shape {
        out.push(
            point
                .get(from..from + length)
                .map(<[f64]>::to_vec)
                .unwrap_or_default(),
        );
        from += length;
    }
    out
}

fn two_loop(gradient: &[f64], memory: &[(Vec<f64>, Vec<f64>)]) -> Vec<f64> {
    let mut out: Vec<f64> = gradient.to_vec();
    let mut alphas = Vec::with_capacity(memory.len());
    for (moved, turned) in memory.iter().rev() {
        let rho = 1.0 / dot(turned, moved);
        let alpha = rho * dot(moved, &out);
        for (slot, one) in out.iter_mut().zip(turned) {
            *slot -= alpha * one;
        }
        alphas.push((alpha, rho));
    }
    let scale = memory.iter().last().map_or(0.5, |(moved, turned)| {
        dot(moved, turned) / dot(turned, turned).max(f64::MIN_POSITIVE)
    });
    for slot in &mut out {
        *slot *= scale;
    }
    for ((moved, turned), (alpha, rho)) in memory.iter().zip(alphas.iter().rev()) {
        let beta = rho * dot(turned, &out);
        for (slot, one) in out.iter_mut().zip(moved) {
            *slot += (alpha - beta) * one;
        }
    }
    out.iter().map(|one| -one).collect()
}

fn system(square: &[f64], toward: &[f64], damping: f64) -> Option<Vec<f64>> {
    let side = toward.len();
    let at = |row: usize, column: usize| square.get(row * side + column).copied().unwrap_or(0.0);
    let mut lower = vec![0.0; side * side];
    for row in 0..side {
        for column in 0..=row {
            let mut sum = at(row, column);
            if row == column {
                sum += damping * at(row, row).max(1e-12);
            }
            for inner in 0..column {
                sum -= lower.get(row * side + inner).copied().unwrap_or(0.0)
                    * lower.get(column * side + inner).copied().unwrap_or(0.0);
            }
            if row == column {
                if sum <= 0.0 || !sum.is_finite() {
                    return None;
                }
                if let Some(slot) = lower.get_mut(row * side + row) {
                    *slot = sum.sqrt();
                }
            } else {
                let diagonal = lower.get(column * side + column).copied().unwrap_or(1.0);
                if let Some(slot) = lower.get_mut(row * side + column) {
                    *slot = sum / diagonal;
                }
            }
        }
    }
    let low = |row: usize, column: usize| lower.get(row * side + column).copied().unwrap_or(0.0);
    let mut middle = vec![0.0; side];
    for row in 0..side {
        let mut sum = -toward.get(row).copied().unwrap_or(0.0);
        for inner in 0..row {
            sum -= low(row, inner) * middle.get(inner).copied().unwrap_or(0.0);
        }
        if let Some(slot) = middle.get_mut(row) {
            *slot = sum / low(row, row);
        }
    }
    let mut out = vec![0.0; side];
    for row in (0..side).rev() {
        let mut sum = middle.get(row).copied().unwrap_or(0.0);
        for inner in (row + 1)..side {
            sum -= low(inner, row) * out.get(inner).copied().unwrap_or(0.0);
        }
        if let Some(slot) = out.get_mut(row) {
            *slot = sum / low(row, row);
        }
    }
    out.iter().all(|one| one.is_finite()).then_some(out)
}

#[derive(Clone, Debug)]
pub struct Stage {
    pub horizon: usize,
    pub rounds: u32,
    pub cost: f64,
    pub count: u64,
    pub stopped: Option<Stop>,
}

pub const ROUNDS: u32 = 24;

pub const DAMPING: f64 = 1e-3;

pub const DAMPING_MOST: f64 = 1e10;

pub const SETTLED: f64 = 1e-7;

impl Run<'_> {
    pub fn descend(
        &mut self,
        theta: &mut Vec<f64>,
        horizon: usize,
        halt: &mut dyn FnMut() -> Option<Stop>,
    ) -> Stage {
        let mut damping = DAMPING;
        let mut stopped = halt();
        if stopped.is_some() {
            return Stage {
                horizon,
                rounds: 0,
                cost: f64::INFINITY,
                count: 0,
                stopped,
            };
        }
        self.reset(horizon);
        self.refine(theta, horizon);
        let mut here = self.normal(theta, horizon, true);
        let mut rounds = 0;
        let mut settled = false;
        while rounds < ROUNDS && !settled {
            if let Some(stop) = halt() {
                stopped = Some(stop);
                break;
            }
            rounds += 1;
            let mut moved = false;
            while damping < DAMPING_MOST {
                let Some(step) = system(&here.square, &here.toward, damping) else {
                    damping *= 4.0;
                    continue;
                };
                let trial: Vec<f64> = theta.iter().zip(&step).map(|(one, by)| one + by).collect();
                let tried = self.normal(&trial, horizon, false);
                if tried.cost.is_finite() && tried.cost < here.cost {
                    let gain = (here.cost - tried.cost) / here.cost.max(f64::MIN_POSITIVE);
                    *theta = trial;
                    damping = (damping / 3.0).max(1e-9);
                    moved = true;
                    settled = gain < SETTLED;
                    break;
                }
                damping *= 4.0;
            }
            if !moved || settled || rounds >= ROUNDS {
                break;
            }
            if rounds % REFINE_EVERY == 0 {
                self.refine(theta, horizon);
            }
            here = self.normal(theta, horizon, true);
        }
        let last = self.normal(theta, horizon, false);
        Stage {
            horizon,
            rounds,
            cost: last.cost,
            count: last.count,
            stopped,
        }
    }
}

pub const REFINE_EVERY: u32 = 4;

pub fn least_squares(rows: &[Vec<f64>], wanted: &[f64]) -> Option<Vec<f64>> {
    let side = rows.iter().next().map_or(0, Vec::len);
    let mut square = vec![0.0; side * side];
    let mut toward = vec![0.0; side];
    for (row, target) in rows.iter().zip(wanted) {
        for (left, one) in row.iter().enumerate() {
            if let Some(slot) = toward.get_mut(left) {
                *slot -= one * target;
            }
            if let Some(line) = square.get_mut(left * side..(left + 1) * side) {
                for (slot, other) in line.iter_mut().zip(row) {
                    *slot += one * other;
                }
            }
        }
    }
    system(&square, &toward, 1e-10)
}

pub fn macro_start(setup: &Setup<'_>) -> Vec<f64> {
    let basis = setup.basis;
    let count = basis.params();
    let mut theta = vec![0.0; count];
    let centre_place = basis.stencil.place([0, 0, 0]);
    let own = |read: usize| {
        basis.terms.iter().position(|term| {
            *term
                == Term::Place {
                    read,
                    place: centre_place,
                }
        })
    };
    for write in 0..basis.reads() {
        if let Some(term) = own(write)
            && let Some(slot) = theta.get_mut(basis.coefficient(write, term))
        {
            *slot = 1.0;
        }
    }
    let seen: Vec<usize> = (0..basis.terms.len())
        .filter(|term| (0..basis.hidden).all(|hidden| basis.hidden_power(*term, hidden) == 0))
        .collect();
    let steps = f64::from(setup.steps.max(1));
    let blank = vec![0.0; count];
    let law = Law {
        basis,
        theta: &blank,
        limits: &setup.limits,
    };
    for write in 0..basis.observed {
        let mut rows = Vec::new();
        let mut wanted = Vec::new();
        let mut values = Vec::new();
        for clip in setup.clips {
            for (pair_at, pair) in clip.shots.windows(2).enumerate() {
                let [from, to] = pair else {
                    continue;
                };
                let mut planes = from.clone();
                clip.put(pair_at * setup.steps as usize, &mut planes);
                planes.extend((0..basis.hidden).map(|_| vec![0.0; clip.grid.cells]));
                for cell in 0..clip.grid.cells {
                    if !clip.scored(write, cell) {
                        continue;
                    }
                    law.values(&clip.grid, &planes, cell, &mut values);
                    let mut row: Vec<f64> = seen
                        .iter()
                        .map(|term| values.get(*term).copied().unwrap_or(0.0))
                        .collect();
                    row.push(1.0);
                    rows.push(row);
                    wanted.push(
                        to.get(write)
                            .and_then(|plane| plane.get(cell))
                            .copied()
                            .unwrap_or(0.0),
                    );
                }
            }
        }
        let Some(found) = least_squares(&rows, &wanted) else {
            continue;
        };
        let identity = own(write);
        for (at, term) in seen.iter().enumerate() {
            let one = found.get(at).copied().unwrap_or(0.0);
            let base = if Some(*term) == identity { 1.0 } else { 0.0 };
            if let Some(slot) = theta.get_mut(basis.coefficient(write, *term)) {
                *slot = base + (one - base) / steps;
            }
        }
        if let Some(slot) = theta.get_mut(basis.bias(write)) {
            *slot = found.last().copied().unwrap_or(0.0) / steps;
        }
    }
    theta
}

pub const LEAKS: [f64; 4] = [0.001, 0.003, 0.01, 0.03];

pub const COUPLINGS: [f64; 3] = [0.1, 0.3, 1.0];

pub fn hidden_starts(run: &mut Run<'_>, theta: &[f64]) -> Vec<Vec<f64>> {
    let basis = run.setup.basis;
    if basis.hidden == 0 {
        return vec![theta.to_vec()];
    }
    let centre_place = basis.stencil.place([0, 0, 0]);
    let own = |read: usize| {
        basis.terms.iter().position(|term| {
            *term
                == Term::Place {
                    read,
                    place: centre_place,
                }
        })
    };
    let means: Vec<f64> = (0..basis.observed)
        .map(|read| {
            let mut sum = 0.0;
            let mut count = 0.0;
            for clip in run.setup.clips {
                for shot in &clip.shots {
                    if let Some(plane) = shot.get(read) {
                        sum += plane.iter().sum::<f64>();
                        count += plane.len() as f64;
                    }
                }
            }
            sum / f64::max(count, 1.0)
        })
        .collect();
    let mut tried: Vec<(f64, Vec<f64>)> = Vec::new();
    for rate in LEAKS {
        for coupling in COUPLINGS {
            let mut trial = theta.to_vec();
            for hidden in 0..basis.hidden {
                let plane = basis.observed + hidden;
                let source = hidden % basis.observed.max(1);
                if let Some(term) = own(plane) {
                    if let Some(slot) = trial.get_mut(basis.coefficient(plane, term)) {
                        *slot = 1.0 - rate;
                    }
                    if let Some(slot) = trial.get_mut(basis.coefficient(source, term)) {
                        *slot = -coupling;
                    }
                }
                if let Some(term) = own(source)
                    && let Some(slot) = trial.get_mut(basis.coefficient(plane, term))
                {
                    *slot = rate;
                }
                if let Some(slot) = trial.get_mut(basis.bias(plane)) {
                    *slot = -rate * means.get(source).copied().unwrap_or(0.0);
                }
            }
            run.reset(1);
            let cost = run.normal(&trial, 1, false).cost;
            if cost.is_finite() {
                tried.push((cost, trial));
            }
        }
    }
    tried.sort_by(|left, right| left.0.total_cmp(&right.0));
    let kept: Vec<Vec<f64>> = tried
        .into_iter()
        .take(STARTS_KEPT)
        .map(|(_, trial)| trial)
        .collect();
    if kept.is_empty() {
        vec![theta.to_vec()]
    } else {
        kept
    }
}

pub const STARTS_KEPT: usize = 3;

pub fn horizons(shots: usize, most: usize) -> Vec<usize> {
    let longest = shots.saturating_sub(1).clamp(1, most.max(1));
    let mut out = vec![1];
    while let Some(&last) = out.last() {
        if last >= longest {
            break;
        }
        out.push((last * 2).min(longest));
    }
    out
}
