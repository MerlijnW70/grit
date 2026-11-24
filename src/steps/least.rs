use std::sync::atomic::{AtomicUsize, Ordering};

use super::{Buffers, Field, Kernel, LANES, Lanes, Stepping, Words, field_of, valued};
use crate::stencil::Sample;

pub const FAMILY_KEPT: usize = 16;

pub const ROUND_BATCHES: usize = 64;

pub const SCREEN_CELLS: usize = 4096;

pub const BISECTED: u32 = 10;

pub const PINNED_NATS: u64 = 20_000_000;

pub const RATE_LEAST: f64 = 1e-6;

pub const RIVAL_STARTS: usize = 3;

pub const SURE: f64 = 4.0;

const THREADS_MOST: usize = 16;

const SALT: u64 = 0x6B4C_3149_5E8D_2A17;

const DRAWN: u64 = 0x2F1D_8A63_C4B7_0E59;

const REDRAWN: u64 = 0x5A3C_96E1_0B7D_F248;

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
    priors: Vec<(u64, u64)>,
    salt: u64,
}

fn priors_of(fields: &[Field], threshold: u64) -> Vec<(u64, u64)> {
    let rate = threshold as f64 / 4_294_967_296.0;
    fields
        .iter()
        .map(|field| {
            if threshold == 0 {
                return (0, 0);
            }
            let lit: u64 = field.bits.iter().sum();
            let seen = lit as f64 / field.cells.max(1) as f64;
            let dense = ((seen - rate) / (1.0 - 2.0 * rate)).clamp(1e-6, 1.0 - 1e-6);
            let live = (1.0 - dense) * rate / ((1.0 - dense) * rate + dense * (1.0 - rate));
            let dead = dense * rate / (dense * rate + (1.0 - dense) * (1.0 - rate));
            let scaled = |chance: f64| (chance * 4_294_967_296.0).round() as u64;
            (scaled(live), scaled(dead))
        })
        .collect()
}

fn losses_of(threshold: u64) -> [[u64; DRAWS + 1]; 2] {
    let rate = (threshold as f64 / 4_294_967_296.0).max(RATE_LEAST);
    let nats = |chance: f64| (-chance.ln() * 1_000_000.0).round() as u64;
    let mut held = [[0_u64; DRAWS + 1]; 2];
    for count in 0..=DRAWS {
        let one = rate + (1.0 - 2.0 * rate) * count as f64 / DRAWS as f64;
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

fn drawn(salt: u64, at: usize, cell: usize, draw: usize, threshold: u64) -> bool {
    let key = salt ^ ((at as u64) << 40) ^ ((cell as u64) << 6) ^ (draw as u64);
    mixed(key.wrapping_mul(0x9E37_79B9_7F4A_7C15)) >> 32 < threshold
}

fn likely_planned(
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
    let (live, dead) = scorer.priors.get(at).copied().unwrap_or((0, 0));
    let mut hits = vec![0_u64; plan.targets.len() * DRAWS];
    for draw in 0..DRAWS {
        buffers.bordered(field);
        let mut layers = plan.layers.iter();
        if let Some(base) = layers.next() {
            for cell in base {
                let cell = *cell as usize;
                let lit = field.bits.get(cell).copied().unwrap_or(0) == 1;
                if let Some(slot) = buffers.one.get_mut(cell) {
                    *slot = if lit
                        ^ drawn(scorer.salt, at, cell, draw, if lit { live } else { dead })
                    {
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
    parts: Vec<Vec<u64>>,
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
        let (plus, losses, priors) = match aim {
            Aim::Wrong => (Vec::new(), [[0; DRAWS + 1]; 2], Vec::new()),
            Aim::Likely(threshold) => (
                planned_by(steps + 1),
                losses_of(threshold),
                priors_of(&fields, threshold),
            ),
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
            priors,
            salt: DRAWN,
        }
    }

    fn aimed_at(&mut self, aim: Aim) {
        if let Aim::Likely(threshold) = aim {
            if self.plus.is_empty() {
                let reach = self.steps + 1;
                self.plus = self
                    .fields
                    .iter()
                    .map(|field| {
                        let targets = field
                            .targets
                            .iter()
                            .filter_map(|(cell, wanted)| {
                                u32::try_from(*cell).ok().map(|one| (one, *wanted))
                            })
                            .collect();
                        Plan::of(&self.kernel, field, targets, reach)
                    })
                    .collect();
            }
            self.losses = losses_of(threshold);
            self.priors = priors_of(&self.fields, threshold);
        }
        self.aim = aim;
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
            Aim::Likely(_) => likely_planned(self, at, plan, words, buffers, lanes, bound),
        }
    }

    fn aimed(&self) -> &[Plan] {
        match self.aim {
            Aim::Wrong => &self.plans,
            Aim::Likely(_) => &self.plus,
        }
    }

    fn blocked(&self) -> Vec<(usize, Plan)> {
        let mut chosen = Vec::new();
        for (at, field) in self.fields.iter().enumerate() {
            let coords: Vec<[usize; 3]> = field
                .targets
                .iter()
                .map(|(cell, _)| field.coords(*cell))
                .collect();
            let mut middle = [usize::MAX; 3];
            for (axis, slot) in middle.iter_mut().enumerate() {
                let values = coords.iter().map(|one| one.get(axis).copied().unwrap_or(0));
                let low = values.clone().min().unwrap_or(0);
                let high = values.max().unwrap_or(0);
                if high > low {
                    *slot = low + (high - low).div_ceil(2);
                }
            }
            let mut groups: Vec<Vec<(u32, bool)>> = vec![Vec::new(); 8];
            for ((cell, wanted), here) in field.targets.iter().zip(&coords) {
                let block = (0..3).fold(0_usize, |held, axis| {
                    let value = here.get(axis).copied().unwrap_or(0);
                    held | (usize::from(value >= middle.get(axis).copied().unwrap_or(usize::MAX))
                        << axis)
                });
                if let (Some(group), Ok(cell)) = (groups.get_mut(block), u32::try_from(*cell)) {
                    group.push((cell, *wanted));
                }
            }
            for group in groups.into_iter().filter(|one| !one.is_empty()) {
                chosen.push((at, Plan::of(&self.kernel, field, group, self.reach())));
            }
        }
        chosen
    }

    fn split(&self, tables: &[u64]) -> Option<Scored> {
        let words = tabled(&self.kernel, tables, self.classes);
        let lanes = tables.len().min(LANES as usize);
        let mask = mask_of(lanes);
        let blocks = self.blocked();
        let parts = spread(blocks.len(), self.largest, |job, buffers| {
            let mut held = Lanes {
                alive: mask,
                wrong: [0; LANES as usize],
            };
            let units = blocks.get(job).map_or(0, |(at, plan)| {
                self.scored_on(*at, plan, &words, buffers, &mut held, u64::MAX)
            });
            (held.wrong, units)
        })?;
        let mut wrong = vec![0_u64; lanes];
        let mut units = 0_u64;
        let mut each = Vec::with_capacity(parts.len());
        for (part, used) in parts {
            units += used;
            for (total, one) in wrong.iter_mut().zip(part) {
                *total += one;
            }
            each.push(part.iter().take(lanes).copied().collect());
        }
        Some(Scored {
            wrong,
            alive: mask,
            units,
            parts: each,
        })
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
        let mut each = Vec::with_capacity(parts.len());
        for (part, used) in parts {
            units += used;
            for (total, one) in wrong.iter_mut().zip(part) {
                *total += one;
            }
            each.push(part.iter().take(lanes).copied().collect());
        }
        Some(Scored {
            wrong,
            alive: mask,
            units,
            parts: each,
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
                parts: Vec::new(),
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

    fn weighted(
        &self,
        tables: &[u64],
        predicted: &[Vec<bool>],
        weights: &[Vec<u64>],
    ) -> Option<Vec<u64>> {
        let words = tabled(&self.kernel, tables, self.classes);
        let lanes = tables.len().min(LANES as usize);
        let mask = mask_of(lanes);
        let parts = spread(self.plans.len(), self.largest, |at, buffers| {
            let mut held = Lanes {
                alive: mask,
                wrong: [0; LANES as usize],
            };
            let (Some(plan), Some(said), Some(weight), Some(field)) = (
                self.plans.get(at),
                predicted.get(at),
                weights.get(at),
                self.fields.get(at),
            ) else {
                return held.wrong;
            };
            let mirrored = Plan {
                layers: plan.layers.clone(),
                targets: Vec::new(),
            };
            let mut probe = Lanes {
                alive: mask,
                wrong: [0; LANES as usize],
            };
            planned(self, at, &mirrored, &words, buffers, &mut probe, u64::MAX);
            for (((cell, _), one), amount) in plan.targets.iter().zip(said).zip(weight) {
                let cell = *cell as usize;
                let value = if plan.layers.is_empty() {
                    let class = field.seed.get(cell).copied().unwrap_or(0) as usize;
                    words.seeded.get(class).copied().unwrap_or(0)
                } else {
                    valued(&self.kernel, field, &words, &buffers.one, cell)
                };
                let base = if *one { u64::MAX } else { 0 };
                let mut left = (value ^ base) & mask;
                while left != 0 {
                    let lane = left.trailing_zeros() as usize;
                    left &= left - 1;
                    held.charged(lane, *amount, u64::MAX);
                }
            }
            held.wrong
        })?;
        let mut sums = vec![0_u64; lanes];
        for part in parts {
            for (total, one) in sums.iter_mut().zip(part) {
                *total += one;
            }
        }
        Some(sums)
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
    (live, dead): (u64, u64),
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
            *slot = base ^ flips(at, cell, if base == 0 { dead } else { live });
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

fn sensed_all(
    scorer: &Scorer,
    words: &Words,
    predicted: &[Vec<bool>],
    threshold: u64,
) -> Option<(Vec<Vec<u64>>, u64)> {
    if threshold == 0 {
        return Some((
            scorer
                .fields
                .iter()
                .map(|field| vec![0; field.targets.len()])
                .collect(),
            0,
        ));
    }
    let priors = priors_of(&scorer.fields, threshold);
    let parts = spread(scorer.fields.len(), scorer.largest, |at, buffers| {
        let said: &[bool] = predicted.get(at).map_or(&[], Vec::as_slice);
        let prior = priors.get(at).copied().unwrap_or((0, 0));
        sensed(scorer, at, words, said, prior, buffers)
    })?;
    let units = parts.iter().map(|one| one.1).sum();
    Some((parts.into_iter().map(|one| one.0).collect(), units))
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
    let (parts, used) = sensed_all(scorer, words, predicted, threshold)?;
    *units += used;
    for (at, part) in parts.into_iter().enumerate() {
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

fn allowed(live: u64, cells: u64) -> f64 {
    if cells == 0 {
        return 0.5;
    }
    let share = live as f64 / cells as f64;
    let spread = 4.0 * (share * (1.0 - share) / cells as f64).sqrt() + 1.0 / cells as f64;
    (share.min(1.0 - share) + spread).min(0.5)
}

fn most_rate(scorer: &Scorer) -> f64 {
    scorer
        .fields
        .iter()
        .flat_map(|field| {
            let before = allowed(field.bits.iter().sum(), field.cells as u64);
            let after = allowed(
                field.targets.iter().filter(|one| one.1).count() as u64,
                field.targets.len() as u64,
            );
            [before, after]
        })
        .fold(0.5_f64, f64::min)
}

fn estimated(run: &mut Run<'_>, code: u64, predicted: &[Vec<bool>]) -> Option<Option<Noise>> {
    let mut used = 0_u64;
    let units = &mut used;
    let keys = run.scorer.classes as usize * 4;
    let mut groups = Groups {
        sizes: vec![0; keys],
        wrong: vec![0; keys],
    };
    for (at, (field, said)) in run.scorer.fields.iter().zip(predicted).enumerate() {
        let split = halves(&run.scorer, at);
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
    let words = tabled(
        &run.scorer.kernel,
        &[code; LANES as usize],
        run.scorer.classes,
    );
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
    let most = most_rate(&run.scorer);
    let share = if basis == 0 {
        0.0
    } else {
        seen as f64 / basis as f64
    };
    let (mut low, mut high) = (0.0_f64, share.min(most));
    let mut beyond = false;
    if share > most {
        if run.stop() {
            run.units += used;
            return Some(None);
        }
        let changed = simulated(&run.scorer, &words, predicted, most, units)?;
        beyond = explained(&groups, &lower, &changed, most) < seen as f64;
        if beyond {
            low = most;
        }
    }
    if high > 0.0 && !beyond {
        for _ in 0..BISECTED {
            if run.stop() {
                run.units += used;
                return Some(None);
            }
            let middle = (low + high) / 2.0;
            let changed = simulated(&run.scorer, &words, predicted, middle, units)?;
            if explained(&groups, &lower, &changed, middle) > seen as f64 {
                high = middle;
            } else {
                low = middle;
            }
        }
    }
    let rate = (low + high) / 2.0;
    if run.stop() {
        run.units += used;
        return Some(None);
    }
    let changed = simulated(&run.scorer, &words, predicted, rate, units)?;
    let all: Vec<usize> = (0..keys).collect();
    let expected = explained(&groups, &all, &changed, rate);
    let wrong: u64 = groups.wrong.iter().sum();
    let bound = 2.0 * expected + 4.0 * expected.sqrt() + 16.0;
    run.units += used;
    Some(Some(Noise {
        per_million: per_million(rate),
        expected: expected.round() as u64,
        basis,
        far_above: beyond || wrong as f64 > bound,
    }))
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
    stopped: bool,
    pinning: bool,
    starts: Vec<u64>,
    rivals: Vec<u64>,
}

impl Run<'_> {
    fn stop(&mut self) -> bool {
        if !self.stopped && (self.halted)() {
            self.stopped = true;
        }
        self.stopped
    }

    fn exact(&mut self, tables: &[u64]) -> Option<Vec<u64>> {
        let scored = self.scorer.exact(tables)?;
        self.checked += tables.len() as u64;
        self.units += scored.units;
        Some(scored.wrong)
    }

    fn pins(&mut self, code: u64, held: &Held<'_>, threshold: u64) -> Option<Option<Vec<bool>>> {
        let wrong = self.scorer.wrong_of(code)?;
        let clean = threshold == 0 && wrong == 0;
        let rate = if threshold == 0 && !clean {
            let share = wrong as f64 / self.scorer.cells().max(1) as f64;
            ((share * 4_294_967_296.0).round() as u64).max(1)
        } else {
            threshold
        };
        let held_aim = self.scorer.aim;
        self.scorer
            .aimed_at(if clean { Aim::Wrong } else { Aim::Likely(rate) });
        let pinned = self.profiled(code, held, wrong, (!clean).then_some(rate));
        self.scorer.aimed_at(held_aim);
        pinned
    }

    fn fitting(&mut self, code: u64, held: &Held<'_>, wrong: u64, rate: u64) -> Option<Vec<bool>> {
        let classes = self.scorer.classes;
        let tables: Vec<u64> = (0..classes).map(|class| code ^ (1_u64 << class)).collect();
        let flipped_wrong = self.scorer.wrongs(&tables)?;
        let words = tabled(&self.scorer.kernel, &[code; LANES as usize], classes);
        let (sensed, used) = sensed_all(&self.scorer, &words, held.predicted, rate)?;
        self.units += used;
        let spread_on = self.scorer.weighted(&tables, held.predicted, &sensed)?;
        let chance = rate as f64 / 4_294_967_296.0;
        Some(
            (0..tables.len())
                .map(|class| {
                    let moved = held.changed.get(class).copied().unwrap_or(0);
                    let gained = i128::from(flipped_wrong.get(class).copied().unwrap_or(0))
                        - i128::from(wrong);
                    let missed = (i128::from(moved) - gained).max(0) / 2;
                    let expected = chance * moved as f64
                        + (1.0 - 2.0 * chance) * spread_on.get(class).copied().unwrap_or(0) as f64
                            / LANES as f64;
                    missed as f64 <= expected + 4.0 * expected.sqrt() + 4.0
                })
                .collect(),
        )
    }

    fn profiled(
        &mut self,
        code: u64,
        held: &Held<'_>,
        wrong: u64,
        rate: Option<u64>,
    ) -> Option<Option<Vec<bool>>> {
        let classes = self.scorer.classes;
        let least = if rate.is_some() { PINNED_NATS } else { 1 };
        let mut pool = Pool::of(classes);
        let mut all = vec![code];
        all.extend((0..classes).map(|class| code ^ (1_u64 << class)));
        let scored = self.scorer.split(&all)?;
        self.units += scored.units;
        for (table, score) in all.iter().zip(&scored.wrong) {
            pool.add(*table, *score);
        }
        let mut rivals: Vec<u64> = self
            .rivals
            .iter()
            .copied()
            .filter(|one| *one != code)
            .collect();
        rivals.sort_unstable();
        rivals.dedup();
        for chunk in rivals.chunks(LANES as usize) {
            let one = self.scorer.exact(chunk)?;
            self.units += one.units;
            for (table, score) in chunk.iter().zip(&one.wrong) {
                pool.add(*table, *score);
            }
        }
        let fits = match rate {
            Some(rate) => self.fitting(code, held, wrong, rate)?,
            None => vec![true; classes as usize],
        };
        let redrawn = if rate.is_some() {
            self.scorer.salt = REDRAWN;
            let again = self.scorer.exact(&all);
            self.scorer.salt = DRAWN;
            let again = again?;
            self.units += again.units;
            again.wrong
        } else {
            scored.wrong.clone()
        };
        let gap = |scores: &[u64], class: u32| -> u64 {
            scores
                .get(class as usize + 1)
                .copied()
                .unwrap_or(0)
                .saturating_sub(scores.first().copied().unwrap_or(0))
        };
        let candidates: Vec<u32> = (0..classes)
            .filter(|class| {
                let at = *class as usize;
                held.margins.get(at).copied().unwrap_or(0) > 0
                    && held.changed.get(at).copied().unwrap_or(0) > 0
                    && fits.get(at).copied().unwrap_or(false)
                    && gap(&scored.wrong, *class) >= least
                    && gap(&redrawn, *class) >= least
                    && (rate.is_none()
                        || proven(&lane_of(&scored, 0), &lane_of(&scored, at + 1), least))
            })
            .collect();
        let base = scored.wrong.first().copied().unwrap_or(u64::MAX);
        let mut walks: Vec<(Option<u32>, u64, u64)> = vec![(None, code, base)];
        walks.extend(candidates.iter().map(|class| {
            (
                Some(*class),
                code ^ (1_u64 << class),
                scored
                    .wrong
                    .get(*class as usize + 1)
                    .copied()
                    .unwrap_or(u64::MAX),
            )
        }));
        while !walks.is_empty() {
            if self.stop() {
                return Some(None);
            }
            let mut tables = Vec::new();
            let mut owners = Vec::new();
            for (at, (frozen, now, _)) in walks.iter().enumerate() {
                for class in (0..classes).filter(|one| Some(*one) != *frozen) {
                    tables.push(now ^ (1_u64 << class));
                    owners.push((at, class));
                }
            }
            let batches = batched(&tables);
            let scored = self
                .scorer
                .bounded(&self.scorer.whole(), &batches, u64::MAX)?;
            let mut best: Vec<Option<(u64, u32)>> = vec![None; walks.len()];
            let mut flat = Vec::with_capacity(tables.len());
            for one in &scored {
                self.units += one.units;
                flat.extend(one.wrong.iter().copied());
            }
            for ((table, score), (at, class)) in tables.iter().zip(&flat).zip(&owners) {
                pool.add(*table, *score);
                if let Some(slot) = best.get_mut(*at)
                    && slot.is_none_or(|kept| (*score, *class) < kept)
                {
                    *slot = Some((*score, *class));
                }
            }
            walks = walks
                .into_iter()
                .zip(best)
                .filter_map(|((frozen, now, score), found)| match found {
                    Some((better, class)) if better < score => {
                        Some((frozen, now ^ (1_u64 << class), better))
                    }
                    _ => None,
                })
                .collect();
        }
        let profiled: Vec<u32> = candidates
            .iter()
            .copied()
            .filter(|class| {
                pool.other(code, *class).0 >= pool.same(code, *class).0.saturating_add(least)
            })
            .collect();
        if rate.is_none() {
            return Some(Some(
                (0..classes)
                    .map(|class| profiled.contains(&class))
                    .collect(),
            ));
        }
        let mut pairs: Vec<u64> = profiled
            .iter()
            .flat_map(|class| [pool.same(code, *class).1, pool.other(code, *class).1])
            .collect();
        pairs.sort_unstable();
        pairs.dedup();
        let mut first = Vec::with_capacity(pairs.len());
        let mut again = Vec::with_capacity(pairs.len());
        for chunk in pairs.chunks(LANES as usize) {
            let one = self.scorer.split(chunk)?;
            self.units += one.units;
            first.push((chunk.to_vec(), one));
            self.scorer.salt = REDRAWN;
            let other = self.scorer.exact(chunk);
            self.scorer.salt = DRAWN;
            let other = other?;
            self.units += other.units;
            again.push(other);
        }
        let looked = |table: u64| -> Option<(Vec<u64>, u64)> {
            for ((chunk, one), other) in first.iter().zip(&again) {
                if let Some(lane) = chunk.iter().position(|held| *held == table) {
                    return Some((lane_of(one, lane), other.wrong.get(lane).copied()?));
                }
            }
            None
        };
        Some(Some(
            (0..classes)
                .map(|class| {
                    if !profiled.contains(&class) {
                        return false;
                    }
                    let (Some(same), Some(other)) = (
                        looked(pool.same(code, class).1),
                        looked(pool.other(code, class).1),
                    ) else {
                        return false;
                    };
                    proven(&same.0, &other.0, least) && other.1 >= same.1.saturating_add(least)
                })
                .collect(),
        ))
    }

    fn singles(&mut self, code: u64) -> Option<Vec<u64>> {
        let tables: Vec<u64> = (0..self.scorer.classes)
            .map(|class| code ^ (1_u64 << class))
            .collect();
        self.exact(&tables)
    }
}

fn lane_of(scored: &Scored, lane: usize) -> Vec<u64> {
    scored
        .parts
        .iter()
        .map(|part| part.get(lane).copied().unwrap_or(0))
        .collect()
}

fn proven(same: &[u64], other: &[u64], least: u64) -> bool {
    let gaps: Vec<f64> = other
        .iter()
        .zip(same)
        .map(|(one, two)| *one as f64 - *two as f64)
        .collect();
    let units = gaps.len();
    if units < 2 {
        return false;
    }
    let total: f64 = gaps.iter().sum();
    let mean = total / units as f64;
    let spread = gaps.iter().map(|one| (one - mean).powi(2)).sum::<f64>() / (units - 1) as f64;
    let error = (units as f64 * spread).sqrt();
    total >= least as f64 && total >= SURE * error
}

struct Held<'a> {
    margins: &'a [u64],
    changed: &'a [u64],
    predicted: &'a [Vec<bool>],
}

struct Pool {
    best: Vec<[(u64, u64); 2]>,
}

impl Pool {
    fn of(classes: u32) -> Self {
        Self {
            best: vec![[(u64::MAX, 0); 2]; classes as usize],
        }
    }

    fn add(&mut self, table: u64, score: u64) {
        for (class, slot) in self.best.iter_mut().enumerate() {
            let value = ((table >> class) & 1) as usize;
            if let Some(kept) = slot.get_mut(value) {
                *kept = (*kept).min((score, table));
            }
        }
    }

    fn side(&self, code: u64, class: u32, flip: u64) -> (u64, u64) {
        let value = (((code >> class) & 1) ^ flip) as usize;
        self.best
            .get(class as usize)
            .and_then(|slot| slot.get(value))
            .copied()
            .unwrap_or((u64::MAX, code))
    }

    fn same(&self, code: u64, class: u32) -> (u64, u64) {
        self.side(code, class, 0)
    }

    fn other(&self, code: u64, class: u32) -> (u64, u64) {
        self.side(code, class, 1)
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
    if run.stop() {
        return Some(());
    }
    let code = learned.code;
    let (used, predicted) = run.scorer.traced(code)?;
    run.units += run.scorer.plans.iter().map(Plan::cells).sum::<u64>() * 2;
    let margins: Vec<u64> = flipped
        .iter()
        .map(|one| one.saturating_sub(learned.score))
        .collect();
    let tables: Vec<u64> = (0..run.scorer.classes)
        .map(|class| code ^ (1_u64 << class))
        .collect();
    let changed = run.scorer.changed(&tables, &predicted)?;
    let noise = if least {
        if run.stop() {
            return Some(());
        }
        let Some(noise) = estimated(run, code, &predicted)? else {
            return Some(());
        };
        Some(noise)
    } else {
        None
    };
    if run.stop() {
        return Some(());
    }
    let threshold = match run.scorer.aim {
        Aim::Likely(threshold) => threshold,
        Aim::Wrong => threshold_of(noise.map_or(0, |one| one.per_million)),
    };
    let held = Held {
        margins: &margins,
        changed: &changed,
        predicted: &predicted,
    };
    let mut pinned = if run.pinning {
        let Some(pinned) = run.pins(code, &held, threshold)? else {
            return Some(());
        };
        pinned
    } else {
        vec![false; run.scorer.classes as usize]
    };
    if noise.is_some_and(|one| one.far_above) {
        pinned.fill(false);
    }
    learned.doubt = Some(Doubt {
        margins,
        changed,
        pinned,
        used,
    });
    learned.noise = noise;
    learned.complete = true;
    Some(())
}

fn closed(run: &Run<'_>, mut learned: Learned) -> Option<Learned> {
    if run.stopped {
        learned.complete = false;
        learned.doubt = None;
        learned.noise = None;
    }
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
        stopped: false,
        pinning: true,
        starts: Vec::new(),
        rivals: Vec::new(),
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

fn descended(run: &mut Run<'_>, start: u64) -> Option<Option<u64>> {
    let mut code = start;
    let mut score = run.exact(&[code])?.first().copied().unwrap_or(u64::MAX);
    loop {
        if run.stop() {
            return Some(None);
        }
        let flipped = run.singles(code)?;
        match flipped
            .iter()
            .enumerate()
            .map(|(class, one)| (*one, class))
            .min()
        {
            Some((better, class)) if better < score => {
                code ^= 1_u64 << class;
                score = better;
            }
            _ => return Some(Some(code)),
        }
    }
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
            let mut ranked: Vec<(u64, u64)> = scores
                .iter()
                .zip(&starts)
                .map(|(score, code)| (*score, *code))
                .collect();
            ranked.sort_unstable();
            let Some((score, code)) = ranked.first().copied() else {
                return Some(None);
            };
            run.starts = ranked
                .iter()
                .skip(1)
                .take(RIVAL_STARTS)
                .map(|one| one.1)
                .collect();
            run.rivals.extend(ranked.iter().skip(1).map(|one| one.1));
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
        let starts = std::mem::take(&mut run.starts);
        for start in starts {
            if run.stop() {
                break;
            }
            let Some(rival) = descended(run, start)? else {
                break;
            };
            run.rivals.push(rival);
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
        stopped: false,
        pinning: true,
        starts: Vec::new(),
        rivals: Vec::new(),
    };
    local_run(&mut run, least, None)
}

pub const START_SHARE: u64 = 16;

pub const FITTED: u32 = 12;

pub const ROUNDS_MOST: u32 = 3;

impl Scorer {
    fn retuned(&mut self, per_million: u64) {
        self.aimed_at(Aim::Likely(threshold_of(per_million)));
    }
}

fn fitted(run: &mut Run<'_>, code: u64, rate: u64) -> Option<Option<u64>> {
    let most = (most_rate(&run.scorer) * 1_000_000.0).max(1.0);
    let high = (rate as f64 * 8.0).min(most).ln();
    let low = (rate as f64 / 8.0).max(1.0).ln().min(high);
    let golden = (5.0_f64.sqrt() - 1.0) / 2.0;
    let mut ends = (low, high);
    let score = |run: &mut Run<'_>, at: f64| -> Option<Option<u64>> {
        if run.stop() {
            return Some(None);
        }
        run.scorer.retuned(at.exp().round() as u64);
        Some(run.exact(&[code])?.first().copied())
    };
    let mut left = ends.1 - golden * (ends.1 - ends.0);
    let mut right = ends.0 + golden * (ends.1 - ends.0);
    let Some(mut left_score) = score(run, left)? else {
        return Some(None);
    };
    let Some(mut right_score) = score(run, right)? else {
        return Some(None);
    };
    for _ in 2..FITTED {
        if left_score <= right_score {
            ends.1 = right;
            right = left;
            right_score = left_score;
            left = ends.1 - golden * (ends.1 - ends.0);
            let Some(held) = score(run, left)? else {
                return Some(None);
            };
            left_score = held;
        } else {
            ends.0 = left;
            left = right;
            left_score = right_score;
            right = ends.0 + golden * (ends.1 - ends.0);
            let Some(held) = score(run, right)? else {
                return Some(None);
            };
            right_score = held;
        }
    }
    let best = if left_score <= right_score {
        left
    } else {
        right
    };
    Some(Some((best.exp().round() as u64).max(1)))
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
        stopped: false,
        pinning: true,
        starts: Vec::new(),
        rivals: Vec::new(),
    };
    run.pinning = false;
    let first_pass = match search {
        Search::Exhaustive => exhaustive_run(&mut run, space, true)?,
        Search::Local => local_run(&mut run, true, None)?,
    };
    let Some(mut first_pass) = first_pass else {
        return Some(None);
    };
    let Some(noise) = first_pass.noise.filter(|_| first_pass.complete) else {
        return Some(Some(first_pass));
    };
    if noise.per_million == 0 {
        run.pinning = true;
        finished(&mut run, &mut first_pass, true)?;
        return Some(Some(closed(&run, first_pass)?));
    }
    let mut rate = (noise.per_million / START_SHARE).max(1);
    let mut run = Run {
        scorer: run.scorer,
        halted: run.halted,
        checked: run.checked,
        units: run.units,
        stopped: run.stopped,
        pinning: true,
        starts: Vec::new(),
        rivals: {
            let mut held = run.rivals;
            held.push(first_pass.code);
            held
        },
    };
    run.scorer.retuned(rate);
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
        let Some(next) = fitted(&mut run, code, rate)? else {
            break;
        };
        if next.abs_diff(rate) * 5 <= rate {
            break;
        }
        rate = next;
    }
    if run.stopped {
        held.complete = false;
        held.doubt = None;
        held.noise = None;
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
