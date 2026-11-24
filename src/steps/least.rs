use std::sync::atomic::{AtomicUsize, Ordering};

use super::{Buffers, Field, Kernel, LANES, Lanes, Stepping, Words, field_of, valued};
use crate::stencil::Sample;

pub const FAMILY_KEPT: usize = 16;

pub const ROUND_BATCHES: usize = 64;

pub const SCREEN_CELLS: usize = 4096;

pub const BISECTED: u32 = 10;

pub const CHANCE_LEAST: f64 = 1e-6;

pub const PINNED_NATS: u64 = 20_000_000;

const THREADS_MOST: usize = 16;

const SALT: u64 = 0x6B4C_3149_5E8D_2A17;

const DRAWN: u64 = 0x2F1D_8A63_C4B7_0E59;

pub const DRAWS: usize = 16;

const PLANES: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Aim {
    Wrong,
    Likely(u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Search {
    Exhaustive,
    Local,
}

impl Search {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Exhaustive => "exhaustive",
            Self::Local => "local",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Move {
    pub flipped: Vec<u32>,
    pub score: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Noise {
    pub per_million: u64,
    pub expected: u64,
    pub basis: u64,
    pub far_above: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Doubt {
    pub margins: Vec<u64>,
    pub changed: Vec<u64>,
    pub pinned: Vec<bool>,
    pub used: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Learned {
    pub search: Search,
    pub code: u64,
    pub wrong: u64,
    pub score: u64,
    pub rate: Option<u64>,
    pub cells: u64,
    pub checked: u64,
    pub complete: bool,
    pub tied: u64,
    pub family: Option<(u64, u64)>,
    pub moves: Vec<Move>,
    pub doubt: Option<Doubt>,
    pub noise: Option<Noise>,
    pub units: u64,
}

struct Plan {
    layers: Vec<Vec<u32>>,
    targets: Vec<(u32, bool)>,
}

impl Plan {
    fn of(kernel: &Kernel, field: &Field, targets: Vec<(u32, bool)>, steps: u32) -> Self {
        let mut layers = Vec::new();
        let mut outer: Vec<usize> = targets.iter().map(|one| one.0 as usize).collect();
        outer.sort_unstable();
        outer.dedup();
        let mut held = Vec::new();
        for _ in 1..steps {
            held.clear();
            for cell in &outer {
                field.reads(kernel, *cell, &mut held);
            }
            held.sort_unstable();
            held.dedup();
            layers.push(
                held.iter()
                    .filter_map(|one| u32::try_from(*one).ok())
                    .collect::<Vec<u32>>(),
            );
            outer.clone_from(&held);
        }
        layers.reverse();
        Self { layers, targets }
    }

    fn cells(&self) -> u64 {
        self.layers.iter().map(|one| one.len() as u64).sum::<u64>() + self.targets.len() as u64
    }
}

struct Scorer {
    kernel: Kernel,
    fields: Vec<Field>,
    plans: Vec<Plan>,
    plus: Vec<Plan>,
    classes: u32,
    steps: u32,
    largest: usize,
    aim: Aim,
    losses: [[u64; DRAWS + 1]; 2],
}

fn losses_of(threshold: u64) -> [[u64; DRAWS + 1]; 2] {
    let rate = threshold as f64 / 4_294_967_296.0;
    let nats = |chance: f64| (-chance.ln() * 1_000_000.0).round() as u64;
    let mut held = [[0_u64; DRAWS + 1]; 2];
    for count in 0..=DRAWS {
        let one = rate + (1.0 - 2.0 * rate) * (count as f64 + 0.5) / (DRAWS as f64 + 1.0);
        if let Some(slot) = held.get_mut(0).and_then(|row| row.get_mut(count)) {
            *slot = nats(1.0 - one);
        }
        if let Some(slot) = held.get_mut(1).and_then(|row| row.get_mut(count)) {
            *slot = nats(one);
        }
    }
    held
}

fn threads() -> usize {
    std::thread::available_parallelism()
        .map_or(1, std::num::NonZero::get)
        .clamp(1, THREADS_MOST)
}

fn spread<T, F>(jobs: usize, largest: usize, work: F) -> Option<Vec<T>>
where
    T: Send,
    F: Fn(usize, &mut Buffers) -> T + Sync,
{
    let workers = threads().min(jobs);
    let fresh = || Buffers {
        one: vec![0; largest],
        two: vec![0; largest],
    };
    if workers <= 1 {
        let mut buffers = fresh();
        return Some((0..jobs).map(|job| work(job, &mut buffers)).collect());
    }
    let next = AtomicUsize::new(0);
    let parts: Vec<Option<Vec<(usize, T)>>> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                scope.spawn(|| {
                    let mut buffers = fresh();
                    let mut held = Vec::new();
                    loop {
                        let job = next.fetch_add(1, Ordering::Relaxed);
                        if job >= jobs {
                            break;
                        }
                        held.push((job, work(job, &mut buffers)));
                    }
                    held
                })
            })
            .collect();
        handles.into_iter().map(|one| one.join().ok()).collect()
    });
    let mut all: Vec<(usize, T)> = Vec::with_capacity(jobs);
    for part in parts {
        all.extend(part?);
    }
    all.sort_by_key(|one| one.0);
    (all.len() == jobs).then(|| all.into_iter().map(|one| one.1).collect())
}

fn mask_of(lanes: usize) -> u64 {
    if lanes >= LANES as usize {
        u64::MAX
    } else {
        (1_u64 << lanes) - 1
    }
}

fn tabled(kernel: &Kernel, tables: &[u64], classes: u32) -> Words {
    let word = |class: u32| -> u64 {
        tables
            .iter()
            .take(LANES as usize)
            .enumerate()
            .fold(0_u64, |held, (lane, code)| {
                held | (((code >> class) & 1) << lane)
            })
    };
    let counts = u32::try_from(kernel.counts).unwrap_or(0);
    let seeded: Vec<u64> = (0..classes).map(word).collect();
    let dark: Vec<u64> = (0..counts).map(word).collect();
    let lit: Vec<u64> = if kernel.outer {
        (counts..2 * counts).map(word).collect()
    } else {
        dark.clone()
    };
    Words { dark, lit, seeded }
}

fn planned(
    scorer: &Scorer,
    at: usize,
    plan: &Plan,
    words: &Words,
    buffers: &mut Buffers,
    lanes: &mut Lanes,
    bound: u64,
) -> u64 {
    let Some(field) = scorer.fields.get(at) else {
        return 0;
    };
    buffers.bordered(field);
    let mut layers = plan.layers.iter();
    if let Some(base) = layers.next() {
        for cell in base {
            let class = field.seed.get(*cell as usize).copied().unwrap_or(0) as usize;
            if let Some(slot) = buffers.one.get_mut(*cell as usize) {
                *slot = words.seeded.get(class).copied().unwrap_or(0);
            }
        }
    }
    for layer in layers {
        for cell in layer {
            let value = valued(&scorer.kernel, field, words, &buffers.one, *cell as usize);
            if let Some(slot) = buffers.two.get_mut(*cell as usize) {
                *slot = value;
            }
        }
        std::mem::swap(&mut buffers.one, &mut buffers.two);
    }
    let mut units = plan.cells() - plan.targets.len() as u64;
    for (cell, wanted) in &plan.targets {
        if lanes.alive == 0 {
            break;
        }
        let cell = *cell as usize;
        let value = if plan.layers.is_empty() {
            let class = field.seed.get(cell).copied().unwrap_or(0) as usize;
            words.seeded.get(class).copied().unwrap_or(0)
        } else {
            valued(&scorer.kernel, field, words, &buffers.one, cell)
        };
        let want = if *wanted { u64::MAX } else { 0 };
        lanes.missed(value ^ want, bound);
        units += 1;
    }
    units
}

fn drawn(at: usize, cell: usize, draw: usize, threshold: u64) -> bool {
    let key = DRAWN ^ ((at as u64) << 40) ^ ((cell as u64) << 6) ^ (draw as u64);
    mixed(key.wrapping_mul(0x9E37_79B9_7F4A_7C15)) >> 32 < threshold
}

fn likely_planned(
    scorer: &Scorer,
    at: usize,
    plan: &Plan,
    words: &Words,
    buffers: &mut Buffers,
    lanes: &mut Lanes,
    (bound, threshold): (u64, u64),
) -> u64 {
    let Some(field) = scorer.fields.get(at) else {
        return 0;
    };
    let mut hits = vec![0_u64; plan.targets.len() * DRAWS];
    for draw in 0..DRAWS {
        buffers.bordered(field);
        let mut layers = plan.layers.iter();
        if let Some(base) = layers.next() {
            for cell in base {
                let cell = *cell as usize;
                let lit = field.bits.get(cell).copied().unwrap_or(0) == 1;
                if let Some(slot) = buffers.one.get_mut(cell) {
                    *slot = if lit ^ drawn(at, cell, draw, threshold) {
                        u64::MAX
                    } else {
                        0
                    };
                }
            }
        }
        for layer in layers {
            for cell in layer {
                let value = valued(&scorer.kernel, field, words, &buffers.one, *cell as usize);
                if let Some(slot) = buffers.two.get_mut(*cell as usize) {
                    *slot = value;
                }
            }
            std::mem::swap(&mut buffers.one, &mut buffers.two);
        }
        for (index, (cell, _)) in plan.targets.iter().enumerate() {
            let value = valued(&scorer.kernel, field, words, &buffers.one, *cell as usize);
            if let Some(slot) = hits.get_mut(index * DRAWS + draw) {
                *slot = value;
            }
        }
    }
    let units = plan.cells() * DRAWS as u64;
    for (index, (_, wanted)) in plan.targets.iter().enumerate() {
        if lanes.alive == 0 {
            break;
        }
        let mut planes = [0_u64; PLANES];
        for draw in 0..DRAWS {
            let mut carry = hits.get(index * DRAWS + draw).copied().unwrap_or(0);
            for plane in &mut planes {
                let both = *plane & carry;
                *plane ^= carry;
                carry = both;
            }
        }
        let row = scorer
            .losses
            .get(usize::from(*wanted))
            .copied()
            .unwrap_or([0; DRAWS + 1]);
        let mut left = lanes.alive;
        while left != 0 {
            let lane = left.trailing_zeros() as usize;
            left &= left - 1;
            let count = planes
                .iter()
                .enumerate()
                .fold(0_usize, |held, (bit, plane)| {
                    held | ((((plane >> lane) & 1) as usize) << bit)
                });
            lanes.charged(lane, row.get(count).copied().unwrap_or(0), bound);
        }
    }
    units
}

struct Scored {
    wrong: Vec<u64>,
    alive: u64,
    units: u64,
}

impl Scorer {
    fn of(stepping: &Stepping, samples: &[Sample<'_>], aim: Aim) -> Self {
        let kernel = Kernel::of(stepping.counting);
        let steps = stepping.steps.max(1);
        let fields: Vec<Field> = samples
            .iter()
            .map(|one| field_of(stepping, &kernel, one))
            .collect();
        let planned_by = |reach: u32| -> Vec<Plan> {
            fields
                .iter()
                .map(|field| {
                    let targets = field
                        .targets
                        .iter()
                        .filter_map(|(cell, wanted)| {
                            u32::try_from(*cell).ok().map(|one| (one, *wanted))
                        })
                        .collect();
                    Plan::of(&kernel, field, targets, reach)
                })
                .collect()
        };
        let plans = planned_by(steps);
        let (plus, losses) = match aim {
            Aim::Wrong => (Vec::new(), [[0; DRAWS + 1]; 2]),
            Aim::Likely(threshold) => (planned_by(steps + 1), losses_of(threshold)),
        };
        let largest = fields.iter().map(|one| one.cells).max().unwrap_or(0) + 1;
        Self {
            kernel,
            fields,
            plans,
            plus,
            classes: stepping.classes(),
            steps,
            largest,
            aim,
            losses,
        }
    }

    fn reach(&self) -> u32 {
        match self.aim {
            Aim::Wrong => self.steps,
            Aim::Likely(_) => self.steps + 1,
        }
    }

    fn scored_on(
        &self,
        at: usize,
        plan: &Plan,
        words: &Words,
        buffers: &mut Buffers,
        lanes: &mut Lanes,
        bound: u64,
    ) -> u64 {
        match self.aim {
            Aim::Wrong => planned(self, at, plan, words, buffers, lanes, bound),
            Aim::Likely(threshold) => {
                likely_planned(self, at, plan, words, buffers, lanes, (bound, threshold))
            }
        }
    }

    fn aimed(&self) -> &[Plan] {
        match self.aim {
            Aim::Wrong => &self.plans,
            Aim::Likely(_) => &self.plus,
        }
    }

    fn wrong_of(&self, code: u64) -> Option<u64> {
        self.wrongs(&[code])?.first().copied()
    }

    fn cells(&self) -> u64 {
        self.fields.iter().map(|one| one.targets.len() as u64).sum()
    }

    fn exact(&self, tables: &[u64]) -> Option<Scored> {
        let words = tabled(&self.kernel, tables, self.classes);
        let lanes = tables.len().min(LANES as usize);
        let mask = mask_of(lanes);
        let aimed = self.aimed();
        let parts = spread(aimed.len(), self.largest, |at, buffers| {
            let mut held = Lanes {
                alive: mask,
                wrong: [0; LANES as usize],
            };
            let units = aimed.get(at).map_or(0, |plan| {
                self.scored_on(at, plan, &words, buffers, &mut held, u64::MAX)
            });
            (held.wrong, units)
        })?;
        let mut wrong = vec![0_u64; lanes];
        let mut units = 0_u64;
        for (part, used) in parts {
            units += used;
            for (total, one) in wrong.iter_mut().zip(part) {
                *total += one;
            }
        }
        Some(Scored {
            wrong,
            alive: mask,
            units,
        })
    }

    fn bounded(
        &self,
        plans: &[(usize, &Plan)],
        batches: &[Vec<u64>],
        bound: u64,
    ) -> Option<Vec<Scored>> {
        spread(batches.len(), self.largest, |job, buffers| {
            let tables: &[u64] = batches.get(job).map_or(&[], Vec::as_slice);
            let words = tabled(&self.kernel, tables, self.classes);
            let lanes = tables.len().min(LANES as usize);
            let mut held = Lanes {
                alive: mask_of(lanes),
                wrong: [0; LANES as usize],
            };
            let mut units = 0_u64;
            for (at, plan) in plans {
                if held.alive == 0 {
                    break;
                }
                units += self.scored_on(*at, plan, &words, buffers, &mut held, bound);
            }
            Scored {
                wrong: held.wrong.iter().take(lanes).copied().collect(),
                alive: held.alive,
                units,
            }
        })
    }

    fn whole(&self) -> Vec<(usize, &Plan)> {
        self.aimed().iter().enumerate().collect()
    }

    fn windows(&self) -> Vec<(usize, Plan)> {
        let mut chosen = Vec::new();
        let mut taken = 0_usize;
        for (at, field) in self.fields.iter().enumerate() {
            if taken >= SCREEN_CELLS {
                break;
            }
            let coords: Vec<[usize; 3]> = field
                .targets
                .iter()
                .map(|(cell, _)| field.coords(*cell))
                .collect();
            let mut least = [usize::MAX; 3];
            let mut most = [0_usize; 3];
            for one in &coords {
                for axis in 0..3 {
                    if let (Some(low), Some(high), Some(here)) =
                        (least.get_mut(axis), most.get_mut(axis), one.get(axis))
                    {
                        *low = (*low).min(*here);
                        *high = (*high).max(*here);
                    }
                }
            }
            let wide: Vec<usize> = (0..3)
                .map(|axis| {
                    most.get(axis).copied().unwrap_or(0) + 1
                        - least
                            .get(axis)
                            .copied()
                            .unwrap_or(0)
                            .min(most.get(axis).copied().unwrap_or(0))
                })
                .collect();
            let dims = wide.iter().filter(|one| **one > 1).count();
            let side = match dims {
                0 | 1 => SCREEN_CELLS / 2,
                2 => 45,
                _ => 12,
            };
            let mut from = [0_usize; 3];
            let mut to = [0_usize; 3];
            for axis in 0..3 {
                let extent = wide.get(axis).copied().unwrap_or(1);
                let low = least.get(axis).copied().unwrap_or(0);
                let kept = extent.min(side);
                let start = low + (extent - kept) / 2;
                if let (Some(begin), Some(end)) = (from.get_mut(axis), to.get_mut(axis)) {
                    *begin = start;
                    *end = start + kept;
                }
            }
            let targets: Vec<(u32, bool)> = field
                .targets
                .iter()
                .zip(&coords)
                .filter(|(_, here)| {
                    (0..3).all(|axis| {
                        let value = here.get(axis).copied().unwrap_or(0);
                        value >= from.get(axis).copied().unwrap_or(0)
                            && value < to.get(axis).copied().unwrap_or(0)
                    })
                })
                .filter_map(|((cell, wanted), _)| {
                    u32::try_from(*cell).ok().map(|one| (one, *wanted))
                })
                .collect();
            taken += targets.len();
            chosen.push((at, Plan::of(&self.kernel, field, targets, self.reach())));
        }
        chosen
    }

    fn wrongs(&self, tables: &[u64]) -> Option<Vec<u64>> {
        let mut held = Vec::with_capacity(tables.len());
        for chunk in tables.chunks(LANES as usize) {
            let words = tabled(&self.kernel, chunk, self.classes);
            let mask = mask_of(chunk.len());
            let parts = spread(self.plans.len(), self.largest, |at, buffers| {
                let mut lanes = Lanes {
                    alive: mask,
                    wrong: [0; LANES as usize],
                };
                if let Some(plan) = self.plans.get(at) {
                    planned(self, at, plan, &words, buffers, &mut lanes, u64::MAX);
                }
                lanes.wrong
            })?;
            let mut sums = vec![0_u64; chunk.len()];
            for part in parts {
                for (total, one) in sums.iter_mut().zip(part) {
                    *total += one;
                }
            }
            held.extend(sums);
        }
        Some(held)
    }

    fn changed(&self, tables: &[u64], predicted: &[Vec<bool>]) -> Option<Vec<u64>> {
        let words = tabled(&self.kernel, tables, self.classes);
        let lanes = tables.len().min(LANES as usize);
        let mask = mask_of(lanes);
        let parts = spread(self.plans.len(), self.largest, |at, buffers| {
            let mut held = Lanes {
                alive: mask,
                wrong: [0; LANES as usize],
            };
            if let (Some(plan), Some(said)) = (self.plans.get(at), predicted.get(at)) {
                let mirrored = Plan {
                    layers: plan.layers.clone(),
                    targets: plan
                        .targets
                        .iter()
                        .zip(said)
                        .map(|((cell, _), one)| (*cell, *one))
                        .collect(),
                };
                planned(self, at, &mirrored, &words, buffers, &mut held, u64::MAX);
            }
            held.wrong
        })?;
        let mut moved = vec![0_u64; lanes];
        for part in parts {
            for (total, one) in moved.iter_mut().zip(part) {
                *total += one;
            }
        }
        Some(moved)
    }

    fn traced(&self, code: u64) -> Option<(Vec<u64>, Vec<Vec<bool>>)> {
        let classes = self.classes as usize;
        let parts = spread(self.plans.len(), self.largest, |at, buffers| {
            let (Some(field), Some(plan)) = (self.fields.get(at), self.plans.get(at)) else {
                return (vec![0_u64; classes], Vec::new());
            };
            traced_field(self, field, plan, code, buffers)
        })?;
        let mut used = vec![0_u64; classes];
        let mut predicted = Vec::with_capacity(parts.len());
        for (part, said) in parts {
            for (total, one) in used.iter_mut().zip(part) {
                *total += one;
            }
            predicted.push(said);
        }
        Some((used, predicted))
    }
}

fn class_of(scorer: &Scorer, field: &Field, prev: &[u64], cell: usize) -> u32 {
    let kernel = &scorer.kernel;
    let at = field.coords(cell);
    let inside = field.inside(at, kernel.axes);
    let mut n = 0_u64;
    for (offset, delta) in kernel.offsets.iter().zip(&field.deltas) {
        let spot = if inside {
            cell.checked_add_signed(*delta).unwrap_or(field.cells)
        } else {
            field.spot(at, *offset)
        };
        n += prev.get(spot).copied().unwrap_or(0) & 1;
    }
    let n = u32::try_from(n).unwrap_or(0);
    if kernel.outer {
        let centre = u32::try_from(prev.get(cell).copied().unwrap_or(0) & 1).unwrap_or(0);
        centre * u32::try_from(kernel.counts).unwrap_or(0) + n
    } else {
        n
    }
}

fn traced_field(
    scorer: &Scorer,
    field: &Field,
    plan: &Plan,
    code: u64,
    buffers: &mut Buffers,
) -> (Vec<u64>, Vec<bool>) {
    let mut used = vec![0_u64; scorer.classes as usize];
    let mut count = |class: u32| {
        if let Some(slot) = used.get_mut(class as usize) {
            *slot += 1;
        }
        (code >> class) & 1
    };
    buffers.bordered(field);
    let mut layers = plan.layers.iter();
    if let Some(base) = layers.next() {
        for cell in base {
            let class = field.seed.get(*cell as usize).copied().unwrap_or(0);
            let value = count(class);
            if let Some(slot) = buffers.one.get_mut(*cell as usize) {
                *slot = value;
            }
        }
    }
    for layer in layers {
        for cell in layer {
            let class = class_of(scorer, field, &buffers.one, *cell as usize);
            let value = count(class);
            if let Some(slot) = buffers.two.get_mut(*cell as usize) {
                *slot = value;
            }
        }
        std::mem::swap(&mut buffers.one, &mut buffers.two);
    }
    let mut said = Vec::with_capacity(plan.targets.len());
    for (cell, _) in &plan.targets {
        let cell = *cell as usize;
        let class = if plan.layers.is_empty() {
            field.seed.get(cell).copied().unwrap_or(0)
        } else {
            class_of(scorer, field, &buffers.one, cell)
        };
        said.push(count(class) == 1);
    }
    (used, said)
}

const fn mixed(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn flips(at: usize, cell: usize, threshold: u64) -> u64 {
    if threshold == 0 {
        return 0;
    }
    let key = SALT ^ ((at as u64) << 40) ^ ((cell as u64) << 6);
    (0..LANES).fold(0_u64, |held, lane| {
        if mixed(key.wrapping_add(lane).wrapping_mul(0x9E37_79B9_7F4A_7C15)) >> 32 < threshold {
            held | (1_u64 << lane)
        } else {
            held
        }
    })
}

fn sensed(
    scorer: &Scorer,
    at: usize,
    words: &Words,
    said: &[bool],
    threshold: u64,
    buffers: &mut Buffers,
) -> (Vec<u64>, u64) {
    let Some(field) = scorer.fields.get(at) else {
        return (Vec::new(), 0);
    };
    buffers.bordered(field);
    for cell in 0..field.cells {
        let base = if field.bits.get(cell).copied().unwrap_or(0) == 1 {
            u64::MAX
        } else {
            0
        };
        if let Some(slot) = buffers.one.get_mut(cell) {
            *slot = base ^ flips(at, cell, threshold);
        }
    }
    let mut units = 0_u64;
    for _ in 1..scorer.steps {
        for cell in 0..field.cells {
            let value = valued(&scorer.kernel, field, words, &buffers.one, cell);
            if let Some(slot) = buffers.two.get_mut(cell) {
                *slot = value;
            }
        }
        units += field.cells as u64;
        std::mem::swap(&mut buffers.one, &mut buffers.two);
    }
    let changed = field
        .targets
        .iter()
        .zip(said)
        .map(|((cell, _), one)| {
            let value = valued(&scorer.kernel, field, words, &buffers.one, *cell);
            let base = if *one { u64::MAX } else { 0 };
            u64::from((value ^ base).count_ones())
        })
        .collect();
    units += field.targets.len() as u64;
    (changed, units)
}

struct Groups {
    sizes: Vec<u64>,
    wrong: Vec<u64>,
}

fn per_million(value: f64) -> u64 {
    if value.is_finite() && value > 0.0 {
        (value * 1_000_000.0).round() as u64
    } else {
        0
    }
}

fn lower_half(groups: &Groups, half: usize) -> Vec<usize> {
    let units = groups.sizes.len() / 2;
    let total: u64 = (0..units)
        .map(|group| groups.sizes.get(group * 2 + half).copied().unwrap_or(0))
        .sum();
    let mut order: Vec<(u64, u64, usize)> = (0..units)
        .filter_map(|group| {
            let size = groups.sizes.get(group * 2 + half).copied().unwrap_or(0);
            (size > 0).then(|| {
                (
                    groups.wrong.get(group * 2 + half).copied().unwrap_or(0),
                    size,
                    group,
                )
            })
        })
        .collect();
    order.sort_by(|one, two| {
        (u128::from(one.0) * u128::from(two.1))
            .cmp(&(u128::from(two.0) * u128::from(one.1)))
            .then(one.2.cmp(&two.2))
    });
    let enough = total.div_ceil(2);
    let mut cells = 0_u64;
    let mut kept = Vec::new();
    for (_, size, group) in order {
        if cells >= enough {
            break;
        }
        cells += size;
        kept.push(group * 2 + (1 - half));
    }
    kept
}

fn simulated(
    scorer: &Scorer,
    words: &Words,
    predicted: &[Vec<bool>],
    rate: f64,
    units: &mut u64,
) -> Option<Vec<u64>> {
    let keys = scorer.classes as usize * 4;
    let mut changed = vec![0_u64; keys];
    let threshold = (rate * 4_294_967_296.0).round() as u64;
    if threshold == 0 {
        return Some(changed);
    }
    let parts = spread(scorer.fields.len(), scorer.largest, |at, buffers| {
        let said: &[bool] = predicted.get(at).map_or(&[], Vec::as_slice);
        sensed(scorer, at, words, said, threshold, buffers)
    })?;
    for (at, (part, used)) in parts.into_iter().enumerate() {
        *units += used;
        let (Some(field), Some(said)) = (scorer.fields.get(at), predicted.get(at)) else {
            continue;
        };
        let split = halves(scorer, at);
        for ((((cell, _), one), count), half) in field.targets.iter().zip(said).zip(part).zip(split)
        {
            if let Some(slot) = changed.get_mut(key_of(field, half, *cell, *one)) {
                *slot += count;
            }
        }
    }
    Some(changed)
}

fn key_of(field: &Field, half: usize, cell: usize, said: bool) -> usize {
    (field.seed.get(cell).copied().unwrap_or(0) as usize * 2 + usize::from(said)) * 2 + half
}

fn halves(scorer: &Scorer, at: usize) -> Vec<usize> {
    let Some(field) = scorer.fields.get(at) else {
        return Vec::new();
    };
    if scorer.fields.len() >= 2 {
        return vec![at % 2; field.targets.len()];
    }
    let coords: Vec<[usize; 3]> = field
        .targets
        .iter()
        .map(|(cell, _)| field.coords(*cell))
        .collect();
    let axis = (0..3)
        .rev()
        .find(|axis| {
            let low = coords
                .iter()
                .map(|one| one.get(*axis).copied().unwrap_or(0))
                .min();
            let high = coords
                .iter()
                .map(|one| one.get(*axis).copied().unwrap_or(0))
                .max();
            low != high
        })
        .unwrap_or(0);
    let low = coords
        .iter()
        .map(|one| one.get(axis).copied().unwrap_or(0))
        .min()
        .unwrap_or(0);
    let high = coords
        .iter()
        .map(|one| one.get(axis).copied().unwrap_or(0))
        .max()
        .unwrap_or(0);
    let middle = low + (high - low).div_ceil(2);
    coords
        .iter()
        .map(|one| usize::from(one.get(axis).copied().unwrap_or(0) >= middle))
        .collect()
}

fn explained(groups: &Groups, keys: &[usize], changed: &[u64], rate: f64) -> f64 {
    keys.iter()
        .map(|key| {
            let size = groups.sizes.get(*key).copied().unwrap_or(0) as f64;
            let sense = changed.get(*key).copied().unwrap_or(0) as f64 / LANES as f64;
            size * rate + (1.0 - 2.0 * rate) * sense
        })
        .sum()
}

fn estimated(
    scorer: &Scorer,
    code: u64,
    predicted: &[Vec<bool>],
    units: &mut u64,
) -> Option<Noise> {
    let keys = scorer.classes as usize * 4;
    let mut groups = Groups {
        sizes: vec![0; keys],
        wrong: vec![0; keys],
    };
    for (at, (field, said)) in scorer.fields.iter().zip(predicted).enumerate() {
        let split = halves(scorer, at);
        for (((cell, wanted), one), half) in field.targets.iter().zip(said).zip(split) {
            let key = key_of(field, half, *cell, *one);
            if let Some(slot) = groups.sizes.get_mut(key) {
                *slot += 1;
            }
            if one != wanted
                && let Some(slot) = groups.wrong.get_mut(key)
            {
                *slot += 1;
            }
        }
    }
    let words = tabled(&scorer.kernel, &[code; LANES as usize], scorer.classes);
    let mut lower = lower_half(&groups, 0);
    lower.extend(lower_half(&groups, 1));
    let basis: u64 = lower
        .iter()
        .map(|key| groups.sizes.get(*key).copied().unwrap_or(0))
        .sum();
    let seen: u64 = lower
        .iter()
        .map(|key| groups.wrong.get(*key).copied().unwrap_or(0))
        .sum();
    let (mut low, mut high) = (
        0.0_f64,
        if basis == 0 {
            0.0
        } else {
            seen as f64 / basis as f64
        },
    );
    if high > 0.0 {
        for _ in 0..BISECTED {
            let middle = (low + high) / 2.0;
            let changed = simulated(scorer, &words, predicted, middle, units)?;
            if explained(&groups, &lower, &changed, middle) > seen as f64 {
                high = middle;
            } else {
                low = middle;
            }
        }
    }
    let rate = (low + high) / 2.0;
    let changed = simulated(scorer, &words, predicted, rate, units)?;
    let all: Vec<usize> = (0..keys).collect();
    let expected = explained(&groups, &all, &changed, rate);
    let wrong: u64 = groups.wrong.iter().sum();
    let bound = 2.0 * expected + 4.0 * expected.sqrt() + 16.0;
    Some(Noise {
        per_million: per_million(rate),
        expected: expected.round() as u64,
        basis,
        far_above: wrong as f64 > bound,
    })
}

fn chance(trials: u64, least: u64, share: f64) -> f64 {
    if least == 0 {
        return 1.0;
    }
    if share <= 0.0 {
        return 0.0;
    }
    if share >= 1.0 {
        return 1.0;
    }
    let (ln_one, ln_none) = (share.ln(), (1.0 - share).ln());
    let mut ln_choose = 0.0_f64;
    for at in 0..least {
        ln_choose += ((trials - at) as f64).ln() - ((at + 1) as f64).ln();
    }
    let mut total = 0.0_f64;
    let mut ln_term = ln_choose + least as f64 * ln_one + (trials - least) as f64 * ln_none;
    for hits in least..=trials {
        total += ln_term.exp();
        if hits < trials {
            ln_term += ((trials - hits) as f64).ln() - ((hits + 1) as f64).ln() + ln_one - ln_none;
        }
    }
    total.min(1.0)
}

fn intervals(counts: u32) -> Vec<u64> {
    let mut held = vec![0_u64];
    for low in 0..counts {
        for high in low..counts {
            let upper = if high + 1 >= 64 {
                u64::MAX
            } else {
                (1_u64 << (high + 1)) - 1
            };
            held.push(upper & !((1_u64 << low) - 1));
        }
    }
    held
}

fn family(scorer: &Scorer) -> Vec<u64> {
    let counts = u32::try_from(scorer.kernel.counts).unwrap_or(0);
    let spans = intervals(counts);
    if scorer.kernel.outer {
        spans
            .iter()
            .flat_map(|born| spans.iter().map(move |kept| born | (kept << counts)))
            .collect()
    } else {
        spans
    }
}

pub fn scored(stepping: &Stepping, samples: &[Sample<'_>], tables: &[u64]) -> Option<Vec<u64>> {
    let scorer = Scorer::of(stepping, samples, Aim::Wrong);
    let mut wrong = Vec::with_capacity(tables.len());
    for chunk in tables.chunks(LANES as usize) {
        wrong.extend(scorer.exact(chunk)?.wrong);
    }
    Some(wrong)
}

pub fn surprised(
    stepping: &Stepping,
    samples: &[Sample<'_>],
    tables: &[u64],
    per_million: u64,
) -> Option<Vec<u64>> {
    let scorer = Scorer::of(stepping, samples, Aim::Likely(threshold_of(per_million)));
    let mut held = Vec::with_capacity(tables.len());
    for chunk in tables.chunks(LANES as usize) {
        held.extend(scorer.exact(chunk)?.wrong);
    }
    Some(held)
}

fn threshold_of(per_million: u64) -> u64 {
    (u128::from(per_million) * (1_u128 << 32) / 1_000_000) as u64
}

fn batched(tables: &[u64]) -> Vec<Vec<u64>> {
    tables.chunks(LANES as usize).map(<[u64]>::to_vec).collect()
}

struct Run<'a> {
    scorer: Scorer,
    halted: &'a mut dyn FnMut() -> bool,
    checked: u64,
    units: u64,
}

impl Run<'_> {
    fn stop(&mut self) -> bool {
        (self.halted)()
    }

    fn exact(&mut self, tables: &[u64]) -> Option<Vec<u64>> {
        let scored = self.scorer.exact(tables)?;
        self.checked += tables.len() as u64;
        self.units += scored.units;
        Some(scored.wrong)
    }

    fn doubt(&mut self, code: u64, score: u64, flipped: &[u64]) -> Option<(Doubt, Vec<Vec<bool>>)> {
        let (used, predicted) = self.scorer.traced(code)?;
        let cells = self.scorer.plans.iter().map(Plan::cells).sum::<u64>();
        self.units += cells * 2;
        let margins: Vec<u64> = flipped
            .iter()
            .map(|one| one.saturating_sub(score))
            .collect();
        let tables: Vec<u64> = (0..self.scorer.classes)
            .map(|class| code ^ (1_u64 << class))
            .collect();
        let changed = self.scorer.changed(&tables, &predicted)?;
        let pinned = match self.scorer.aim {
            Aim::Wrong => {
                let share = score as f64 / self.scorer.cells().max(1) as f64;
                margins
                    .iter()
                    .zip(&changed)
                    .map(|(margin, moved)| {
                        *margin > 0
                            && chance(*moved, (*moved + *margin).div_ceil(2), share) < CHANCE_LEAST
                    })
                    .collect()
            }
            Aim::Likely(_) => {
                let wrong = self.scorer.wrongs(&[code])?.first().copied().unwrap_or(0);
                let share = wrong as f64 / self.scorer.cells().max(1) as f64;
                let flips = self.scorer.wrongs(&tables)?;
                margins
                    .iter()
                    .zip(&changed)
                    .zip(&flips)
                    .map(|((margin, moved), flipped)| {
                        let gained = flipped.saturating_sub(wrong);
                        *margin >= PINNED_NATS
                            && gained > 0
                            && chance(*moved, (*moved + gained).div_ceil(2), share) < CHANCE_LEAST
                    })
                    .collect()
            }
        };
        Some((
            Doubt {
                margins,
                changed,
                pinned,
                used,
            },
            predicted,
        ))
    }

    fn singles(&mut self, code: u64) -> Option<Vec<u64>> {
        let tables: Vec<u64> = (0..self.scorer.classes)
            .map(|class| code ^ (1_u64 << class))
            .collect();
        self.exact(&tables)
    }
}

fn blank(search: Search, code: u64, score: u64, cells: u64) -> Learned {
    Learned {
        search,
        code,
        wrong: 0,
        score,
        rate: None,
        cells,
        checked: 0,
        complete: false,
        tied: 0,
        family: None,
        moves: Vec::new(),
        doubt: None,
        noise: None,
        units: 0,
    }
}

fn finished(run: &mut Run<'_>, learned: &mut Learned, least: bool) -> Option<()> {
    if run.stop() {
        return Some(());
    }
    let flipped = run.singles(learned.code)?;
    settled(run, learned, least, &flipped)
}

fn settled(run: &mut Run<'_>, learned: &mut Learned, least: bool, flipped: &[u64]) -> Option<()> {
    let (doubt, predicted) = run.doubt(learned.code, learned.score, flipped)?;
    learned.doubt = Some(doubt);
    if least {
        if run.stop() {
            return Some(());
        }
        let mut units = 0_u64;
        learned.noise = estimated(&run.scorer, learned.code, &predicted, &mut units);
        run.units += units;
    }
    learned.complete = true;
    Some(())
}

fn closed(run: &Run<'_>, mut learned: Learned) -> Option<Learned> {
    learned.wrong = match run.scorer.aim {
        Aim::Wrong => learned.score,
        Aim::Likely(_) => run.scorer.wrong_of(learned.code)?,
    };
    learned.checked = run.checked;
    learned.units = run.units;
    Some(learned)
}

fn exhaustive_run(run: &mut Run<'_>, space: u64, least: bool) -> Option<Option<Learned>> {
    let batches = space.div_ceil(LANES);
    let mut found: Option<(u64, u64, u64)> = None;
    let mut done = 0_u64;
    let mut stopped = false;
    while done < batches {
        if run.stop() {
            stopped = true;
            break;
        }
        let upto = (done + ROUND_BATCHES as u64).min(batches);
        let round: Vec<Vec<u64>> = (done..upto)
            .map(|batch| {
                let low = batch * LANES;
                (low..(low + LANES).min(space)).collect()
            })
            .collect();
        let bound = found.map_or(u64::MAX, |one| one.0);
        let scored = run.scorer.bounded(&run.scorer.whole(), &round, bound)?;
        for (batch, one) in (done..upto).zip(&scored) {
            run.units += one.units;
            for (lane, score) in one.wrong.iter().enumerate() {
                if (one.alive >> lane) & 1 == 0 {
                    continue;
                }
                let code = batch * LANES + lane as u64;
                found = match found {
                    Some((least_score, held, tied)) if *score == least_score => {
                        Some((least_score, held, tied + 1))
                    }
                    Some((least_score, _, _)) if *score > least_score => found,
                    _ => Some((*score, code, 1)),
                };
            }
        }
        run.checked += (upto * LANES).min(space) - (done * LANES).min(space);
        done = upto;
    }
    let Some((score, code, tied)) = found else {
        return Some(None);
    };
    let mut learned = blank(Search::Exhaustive, code, score, run.scorer.cells());
    learned.tied = tied;
    if !stopped {
        finished(run, &mut learned, least)?;
    }
    Some(Some(closed(run, learned)?))
}

pub fn exhaustive(
    stepping: &Stepping,
    samples: &[Sample<'_>],
    least: bool,
    halted: &mut dyn FnMut() -> bool,
) -> Option<Option<Learned>> {
    let mut run = Run {
        scorer: Scorer::of(stepping, samples, Aim::Wrong),
        halted,
        checked: 0,
        units: 0,
    };
    exhaustive_run(&mut run, stepping.space(), least)
}

fn screened(run: &mut Run<'_>) -> Option<Option<Vec<(u64, u64)>>> {
    let tables = family(&run.scorer);
    let batches = batched(&tables);
    let owned = run.scorer.windows();
    let plans: Vec<(usize, &Plan)> = owned.iter().map(|(at, plan)| (*at, plan)).collect();
    let mut kept: Vec<(u64, u64)> = Vec::new();
    for chunk in batches.chunks(ROUND_BATCHES) {
        if run.stop() {
            return Some(None);
        }
        let bound = if kept.len() >= FAMILY_KEPT {
            kept.last().map_or(u64::MAX, |one| one.0)
        } else {
            u64::MAX
        };
        let scored = run.scorer.bounded(&plans, chunk, bound)?;
        for (at, one) in scored.iter().enumerate() {
            run.units += one.units;
            let batch = chunk.get(at).map_or(&[][..], Vec::as_slice);
            run.checked += batch.len() as u64;
            for (lane, code) in batch.iter().enumerate() {
                if (one.alive >> lane) & 1 == 0 {
                    continue;
                }
                kept.push((one.wrong.get(lane).copied().unwrap_or(u64::MAX), *code));
            }
        }
        kept.sort_unstable();
        kept.truncate(FAMILY_KEPT);
    }
    Some(Some(kept))
}

fn local_run(run: &mut Run<'_>, least: bool, start: Option<u64>) -> Option<Option<Learned>> {
    let (code, score, family) = match start {
        Some(code) => {
            if run.stop() {
                return Some(None);
            }
            let score = run.exact(&[code])?.first().copied().unwrap_or(u64::MAX);
            (code, score, None)
        }
        None => {
            let Some(kept) = screened(run)? else {
                return Some(None);
            };
            if run.stop() {
                return Some(None);
            }
            let starts: Vec<u64> = kept.iter().map(|one| one.1).collect();
            let scores = run.exact(&starts)?;
            let Some((score, code)) = scores
                .iter()
                .zip(&starts)
                .map(|(score, code)| (*score, *code))
                .min()
            else {
                return Some(None);
            };
            (code, score, Some((code, score)))
        }
    };
    let mut learned = blank(Search::Local, code, score, run.scorer.cells());
    learned.family = family;
    loop {
        if run.stop() {
            break;
        }
        let flipped = run.singles(learned.code)?;
        let best = flipped
            .iter()
            .enumerate()
            .map(|(class, score)| (*score, class))
            .min();
        if let Some((better, class)) = best
            && better < learned.score
        {
            learned.code ^= 1_u64 << class;
            learned.score = better;
            learned.moves.push(Move {
                flipped: vec![u32::try_from(class).unwrap_or(0)],
                score: better,
            });
            continue;
        }
        if learned.score > 0 {
            if run.stop() {
                break;
            }
            if let Some((score, one, two)) = paired(run, learned.code, learned.score)? {
                learned.code ^= (1_u64 << one) ^ (1_u64 << two);
                learned.score = score;
                learned.moves.push(Move {
                    flipped: vec![one, two],
                    score,
                });
                continue;
            }
        }
        settled(run, &mut learned, least, &flipped)?;
        break;
    }
    Some(Some(closed(run, learned)?))
}

pub fn local(
    stepping: &Stepping,
    samples: &[Sample<'_>],
    least: bool,
    halted: &mut dyn FnMut() -> bool,
) -> Option<Option<Learned>> {
    let mut run = Run {
        scorer: Scorer::of(stepping, samples, Aim::Wrong),
        halted,
        checked: 0,
        units: 0,
    };
    local_run(&mut run, least, None)
}

pub const START_SHARE: u64 = 16;

pub const FITTED: u32 = 12;

pub const ROUNDS_MOST: u32 = 3;

impl Scorer {
    fn retuned(&mut self, per_million: u64) {
        let threshold = threshold_of(per_million);
        self.aim = Aim::Likely(threshold);
        self.losses = losses_of(threshold);
    }
}

fn fitted(run: &mut Run<'_>, code: u64, rate: u64) -> Option<u64> {
    let low = (rate as f64 / 8.0).max(1.0).ln();
    let high = (rate as f64 * 8.0).min(250_000.0).ln();
    let golden = (5.0_f64.sqrt() - 1.0) / 2.0;
    let mut ends = (low, high);
    let score = |run: &mut Run<'_>, at: f64| -> Option<u64> {
        run.scorer.retuned(at.exp().round() as u64);
        run.exact(&[code])?.first().copied()
    };
    let mut left = ends.1 - golden * (ends.1 - ends.0);
    let mut right = ends.0 + golden * (ends.1 - ends.0);
    let mut left_score = score(run, left)?;
    let mut right_score = score(run, right)?;
    for _ in 2..FITTED {
        if left_score <= right_score {
            ends.1 = right;
            right = left;
            right_score = left_score;
            left = ends.1 - golden * (ends.1 - ends.0);
            left_score = score(run, left)?;
        } else {
            ends.0 = left;
            left = right;
            left_score = right_score;
            right = ends.0 + golden * (ends.1 - ends.0);
            right_score = score(run, right)?;
        }
    }
    let best = if left_score <= right_score {
        left
    } else {
        right
    };
    Some((best.exp().round() as u64).max(1))
}

pub fn likely(
    stepping: &Stepping,
    samples: &[Sample<'_>],
    search: Search,
    halted: &mut dyn FnMut() -> bool,
) -> Option<Option<Learned>> {
    let space = stepping.space();
    let mut run = Run {
        scorer: Scorer::of(stepping, samples, Aim::Wrong),
        halted,
        checked: 0,
        units: 0,
    };
    let first_pass = match search {
        Search::Exhaustive => exhaustive_run(&mut run, space, true)?,
        Search::Local => local_run(&mut run, true, None)?,
    };
    let Some(first_pass) = first_pass else {
        return Some(None);
    };
    let Some(noise) = first_pass.noise.filter(|_| first_pass.complete) else {
        return Some(Some(first_pass));
    };
    if noise.per_million == 0 {
        return Some(Some(first_pass));
    }
    let mut rate = (noise.per_million / START_SHARE).max(1);
    let (checked, units) = (run.checked, run.units);
    let mut run = Run {
        scorer: Scorer::of(stepping, samples, Aim::Likely(threshold_of(rate))),
        halted: run.halted,
        checked,
        units,
    };
    let mut held = first_pass;
    for round in 0..ROUNDS_MOST {
        run.scorer.retuned(rate);
        let found = match (search, round) {
            (Search::Exhaustive, 0) => exhaustive_run(&mut run, space, true)?,
            (Search::Local, 0) => local_run(&mut run, true, None)?,
            _ => local_run(&mut run, true, Some(held.code))?,
        };
        let Some(mut found) = found else {
            break;
        };
        found.rate = Some(rate);
        let complete = found.complete;
        let code = found.code;
        held = found;
        if !complete || round + 1 == ROUNDS_MOST || run.stop() {
            break;
        }
        let next = fitted(&mut run, code, rate)?;
        if next.abs_diff(rate) * 5 <= rate {
            break;
        }
        rate = next;
    }
    held.checked = run.checked;
    held.units = run.units;
    Some(Some(held))
}

fn paired(run: &mut Run<'_>, code: u64, score: u64) -> Option<Option<(u64, u32, u32)>> {
    let classes = run.scorer.classes;
    let (used, _) = run.scorer.traced(code)?;
    run.units += run.scorer.plans.iter().map(Plan::cells).sum::<u64>();
    let reached = |class: u32| used.get(class as usize).copied().unwrap_or(0) > 0;
    let mut pairs = Vec::new();
    for one in 0..classes {
        for two in one + 1..classes {
            if reached(one) || reached(two) {
                pairs.push((one, two));
            }
        }
    }
    let tables: Vec<u64> = pairs
        .iter()
        .map(|(one, two)| code ^ (1_u64 << one) ^ (1_u64 << two))
        .collect();
    let batches = batched(&tables);
    let scored = run
        .scorer
        .bounded(&run.scorer.whole(), &batches, score - 1)?;
    let mut best: Option<(u64, usize)> = None;
    for (batch, one) in scored.iter().enumerate() {
        run.units += one.units;
        run.checked += batches.get(batch).map_or(0, |held| held.len() as u64);
        for (lane, held) in one.wrong.iter().enumerate() {
            if (one.alive >> lane) & 1 == 0 {
                continue;
            }
            let index = batch * LANES as usize + lane;
            if best.is_none_or(|kept| (*held, index) < kept) {
                best = Some((*held, index));
            }
        }
    }
    Some(
        best.and_then(|(found, index)| pairs.get(index).map(|pair| (found, pair.0, pair.1)))
            .filter(|(found, _, _)| *found < score),
    )
}
