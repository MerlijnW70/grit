use crate::stencil::{Border, Counting, Sample, Tally, index};

pub const STEPS_MOST: u32 = 8;

pub const CLASSES_MOST: u32 = 18;

pub const SHOWN_MOST: usize = 16;

pub const PROBES: usize = 64;

const PLANES: usize = 5;

const LANES: u64 = 64;

const NONE: u32 = u32::MAX;

const LOW: [u64; 6] = [
    0xAAAA_AAAA_AAAA_AAAA,
    0xCCCC_CCCC_CCCC_CCCC,
    0xF0F0_F0F0_F0F0_F0F0,
    0xFF00_FF00_FF00_FF00,
    0xFFFF_0000_FFFF_0000,
    0xFFFF_FFFF_0000_0000,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stepping {
    pub counting: Counting,
    pub steps: u32,
    pub border: Border,
    pub read: u32,
    pub write: u32,
}

impl Stepping {
    #[must_use]
    pub fn classes(&self) -> u32 {
        self.counting.classes()
    }

    #[must_use]
    pub fn space(&self) -> u64 {
        1_u64.checked_shl(self.classes()).unwrap_or(u64::MAX)
    }

    #[must_use]
    pub fn fits(&self) -> bool {
        self.classes() <= CLASSES_MOST
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub consistent: u64,
    pub codes: Vec<u64>,
    pub closest: Option<(u64, u64)>,
    pub checked: u64,
    pub complete: bool,
    pub cells: u64,
}

#[must_use]
pub fn entry(code: u64, class: u32) -> u32 {
    u32::from((code >> class) & 1 == 1)
}

#[must_use]
pub fn table_of(code: u64, classes: u32) -> Vec<u32> {
    (0..classes).map(|class| entry(code, class)).collect()
}

struct Kernel {
    outer: bool,
    counts: usize,
    planes: usize,
    offsets: Vec<[i64; 3]>,
    axes: usize,
}

impl Kernel {
    fn of(counting: Counting) -> Self {
        let offsets: Vec<[i64; 3]> = counting
            .counted()
            .into_iter()
            .map(|place| counting.stencil.offset(place))
            .collect();
        let most = offsets.len();
        let planes = (usize::BITS - most.leading_zeros()) as usize;
        Self {
            outer: counting.tally == Tally::Outer,
            counts: counting.counts() as usize,
            planes: planes.min(PLANES),
            offsets,
            axes: counting.stencil.axes(),
        }
    }

    fn sized(&self, size: [usize; 3]) -> Vec<isize> {
        let wide = isize::try_from(size[0]).unwrap_or(isize::MAX);
        let layer = wide.saturating_mul(isize::try_from(size[1]).unwrap_or(isize::MAX));
        self.offsets
            .iter()
            .map(|offset| {
                let [dx, dy, dz] = offset.map(|one| isize::try_from(one).unwrap_or(0));
                dz.saturating_mul(layer)
                    .saturating_add(dy.saturating_mul(wide))
                    .saturating_add(dx)
            })
            .collect()
    }
}

struct Field {
    size: [usize; 3],
    cells: usize,
    near: [[Vec<u32>; 3]; 3],
    deltas: Vec<isize>,
    seed: Vec<u32>,
    targets: Vec<(usize, bool)>,
}

impl Field {
    fn spot(&self, at: [usize; 3], offset: [i64; 3]) -> usize {
        let mut spot = [0_usize; 3];
        for (axis, slot) in spot.iter_mut().enumerate() {
            let step = usize::try_from(offset.get(axis).copied().unwrap_or(0) + 1).unwrap_or(1);
            let moved = self
                .near
                .get(axis)
                .and_then(|held| held.get(step))
                .and_then(|held| held.get(at.get(axis).copied().unwrap_or(0)))
                .copied()
                .unwrap_or(NONE);
            if moved == NONE {
                return self.cells;
            }
            *slot = moved as usize;
        }
        (spot[2] * self.size[1] + spot[1]) * self.size[0] + spot[0]
    }

    fn inside(&self, at: [usize; 3], axes: usize) -> bool {
        at.iter()
            .zip(self.size)
            .take(axes)
            .all(|(one, side)| *one >= 1 && one + 1 < side)
    }

    fn coords(&self, cell: usize) -> [usize; 3] {
        let wide = self.size[0].max(1);
        let deep = self.size[1].max(1);
        [cell % wide, cell / wide % deep, cell / wide / deep]
    }

    fn reads(&self, kernel: &Kernel, cell: usize, out: &mut Vec<usize>) {
        let at = self.coords(cell);
        for offset in &kernel.offsets {
            let spot = self.spot(at, *offset);
            if spot < self.cells {
                out.push(spot);
            }
        }
        if kernel.outer {
            out.push(cell);
        }
    }
}

fn near_of(side: usize, border: Border) -> [Vec<u32>; 3] {
    let signed = i64::try_from(side).unwrap_or(i64::MAX);
    let moved = |step: i64| -> Vec<u32> {
        (0..signed)
            .map(|value| {
                let to = value + step;
                let inside = if (0..signed).contains(&to) {
                    Some(to)
                } else if border == Border::Wrapped {
                    Some(to.rem_euclid(signed.max(1)))
                } else {
                    None
                };
                inside
                    .and_then(|one| u32::try_from(one).ok())
                    .unwrap_or(NONE)
            })
            .collect()
    };
    [moved(-1), moved(0), moved(1)]
}

fn field_of(stepping: &Stepping, kernel: &Kernel, sample: &Sample<'_>) -> Field {
    let size = sample.size.map(|side| side as usize);
    let cells = size.iter().product();
    let near = [
        near_of(size[0], stepping.border),
        near_of(size[1], stepping.border),
        near_of(size[2], stepping.border),
    ];
    let mut field = Field {
        size,
        cells,
        near,
        deltas: kernel.sized(size),
        seed: Vec::new(),
        targets: Vec::new(),
    };
    let words = sample.inputs.first().copied().unwrap_or(&[]);
    let bits: Vec<u64> = (0..cells)
        .map(|cell| {
            words
                .get(cell)
                .map_or(0, |word| u64::from((word >> stepping.read) & 1))
        })
        .collect();
    let mut seed = Vec::with_capacity(cells);
    for cell in 0..cells {
        let at = field.coords(cell);
        let n: u64 = kernel
            .offsets
            .iter()
            .map(|offset| bits.get(field.spot(at, *offset)).copied().unwrap_or(0))
            .sum();
        let n = u32::try_from(n).unwrap_or(0);
        let class = if kernel.outer {
            let centre = u32::try_from(bits.get(cell).copied().unwrap_or(0)).unwrap_or(0);
            centre * u32::try_from(kernel.counts).unwrap_or(0) + n
        } else {
            n
        };
        seed.push(class);
    }
    field.seed = seed;
    let wanted = sample.outputs.first().copied().unwrap_or(&[]);
    let [wide, deep, tall] = sample.inner;
    for z in 0..tall {
        for y in 0..deep {
            for x in 0..wide {
                let at = [
                    (sample.from[0] + x) as usize,
                    (sample.from[1] + y) as usize,
                    (sample.from[2] + z) as usize,
                ];
                let cell = (at[2] * size[1] + at[1]) * size[0] + at[0];
                let bit = wanted
                    .get(index(sample.inner, [x, y, z]))
                    .is_some_and(|word| (word >> stepping.write) & 1 == 1);
                field.targets.push((cell, bit));
            }
        }
    }
    field
}

struct Words {
    dark: Vec<u64>,
    lit: Vec<u64>,
    seeded: Vec<u64>,
}

impl Words {
    fn of(kernel: &Kernel, batch: u64, classes: u32, lanes: u64) -> Self {
        let word = |class: u32| -> u64 {
            if class >= classes {
                return 0;
            }
            let held = if class < 6 {
                LOW.get(class as usize).copied().unwrap_or(0)
            } else if (batch >> (class - 6)) & 1 == 1 {
                u64::MAX
            } else {
                0
            };
            held & lanes
        };
        let counts = u32::try_from(kernel.counts).unwrap_or(0);
        let seeded: Vec<u64> = (0..classes).map(word).collect();
        let dark: Vec<u64> = (0..counts).map(word).collect();
        let lit: Vec<u64> = if kernel.outer {
            (counts..2 * counts).map(word).collect()
        } else {
            dark.clone()
        };
        Self { dark, lit, seeded }
    }
}

fn valued(kernel: &Kernel, field: &Field, words: &Words, prev: &[u64], cell: usize) -> u64 {
    let at = field.coords(cell);
    let mut sum = [0_u64; PLANES];
    let inside = field.inside(at, kernel.axes);
    for (offset, delta) in kernel.offsets.iter().zip(&field.deltas) {
        let spot = if inside {
            cell.checked_add_signed(*delta).unwrap_or(field.cells)
        } else {
            field.spot(at, *offset)
        };
        let mut carry = prev.get(spot).copied().unwrap_or(0);
        for plane in sum.iter_mut().take(kernel.planes) {
            let both = *plane & carry;
            *plane ^= carry;
            carry = both;
        }
    }
    let centre = if kernel.outer {
        prev.get(cell).copied().unwrap_or(0)
    } else {
        0
    };
    let mut out = 0_u64;
    for (n, (dark, lit)) in words.dark.iter().zip(&words.lit).enumerate() {
        if dark | lit == 0 {
            continue;
        }
        let mut equal = u64::MAX;
        for (plane, held) in sum.iter().enumerate().take(kernel.planes) {
            equal &= if (n >> plane) & 1 == 1 { *held } else { !*held };
        }
        out |= equal & ((centre & lit) | (!centre & dark));
    }
    out
}

struct Probe {
    field: usize,
    target: usize,
    wanted: bool,
    cone: Vec<Vec<usize>>,
    work: u64,
}

fn stride_of(count: usize) -> usize {
    let mut stride = (count * 382 / 1000).max(1);
    while gcd(stride, count) != 1 {
        stride += 1;
    }
    stride
}

const fn gcd(mut a: usize, mut b: usize) -> usize {
    while b != 0 {
        let rest = a % b;
        a = b;
        b = rest;
    }
    a
}

fn probes_of(kernel: &Kernel, fields: &[Field], steps: u32) -> Vec<Probe> {
    let strides: Vec<usize> = fields
        .iter()
        .map(|one| stride_of(one.targets.len().max(1)))
        .collect();
    let longest = fields
        .iter()
        .map(|one| one.targets.len())
        .max()
        .unwrap_or(0);
    let mut picked = Vec::new();
    'rounds: for round in 0..longest {
        for (at, field) in fields.iter().enumerate() {
            let count = field.targets.len();
            if round >= count {
                continue;
            }
            let stride = strides.get(at).copied().unwrap_or(1);
            let Some((target, wanted)) = field.targets.get(round * stride % count).copied() else {
                continue;
            };
            picked.push((at, target, wanted));
            if picked.len() >= PROBES {
                break 'rounds;
            }
        }
    }
    picked
        .into_iter()
        .map(|(at, target, wanted)| {
            let Some(field) = fields.get(at) else {
                return Probe {
                    field: at,
                    target,
                    wanted,
                    cone: Vec::new(),
                    work: 0,
                };
            };
            let mut cone: Vec<Vec<usize>> = Vec::new();
            let mut outer = vec![target];
            for _ in 1..steps {
                let mut held = Vec::new();
                for cell in &outer {
                    field.reads(kernel, *cell, &mut held);
                }
                held.sort_unstable();
                held.dedup();
                cone.push(held.clone());
                outer = held;
            }
            cone.reverse();
            let work = cone.iter().map(|one| one.len() as u64).sum::<u64>() + 1;
            Probe {
                field: at,
                target,
                wanted,
                cone,
                work,
            }
        })
        .collect()
}

struct Lanes {
    alive: u64,
    wrong: [u64; LANES as usize],
}

impl Lanes {
    fn missed(&mut self, missing: u64, best: u64) {
        let mut left = missing & self.alive;
        while left != 0 {
            let lane = left.trailing_zeros() as usize;
            left &= left - 1;
            if let Some(count) = self.wrong.get_mut(lane) {
                *count += 1;
                if *count > best {
                    self.alive &= !(1_u64 << lane);
                }
            }
        }
    }
}

struct Buffers {
    one: Vec<u64>,
    two: Vec<u64>,
}

impl Buffers {
    fn bordered(&mut self, field: &Field) {
        for held in [&mut self.one, &mut self.two] {
            if let Some(slot) = held.get_mut(field.cells) {
                *slot = 0;
            }
        }
    }
}

fn probed(
    kernel: &Kernel,
    fields: &[Field],
    words: &Words,
    probe: &Probe,
    buffers: &mut Buffers,
) -> u64 {
    let Some(field) = fields.get(probe.field) else {
        return 0;
    };
    buffers.bordered(field);
    let mut layers = probe.cone.iter();
    if let Some(base) = layers.next() {
        for cell in base {
            let class = field.seed.get(*cell).copied().unwrap_or(0) as usize;
            if let Some(slot) = buffers.one.get_mut(*cell) {
                *slot = words.seeded.get(class).copied().unwrap_or(0);
            }
        }
    }
    for layer in layers {
        for cell in layer {
            let value = valued(kernel, field, words, &buffers.one, *cell);
            if let Some(slot) = buffers.two.get_mut(*cell) {
                *slot = value;
            }
        }
        std::mem::swap(&mut buffers.one, &mut buffers.two);
    }
    if probe.cone.is_empty() {
        let class = field.seed.get(probe.target).copied().unwrap_or(0) as usize;
        return words.seeded.get(class).copied().unwrap_or(0);
    }
    valued(kernel, field, words, &buffers.one, probe.target)
}

fn checked_whole(
    kernel: &Kernel,
    field: &Field,
    words: &Words,
    steps: u32,
    buffers: &mut Buffers,
    lanes: &mut Lanes,
    best: u64,
) {
    buffers.bordered(field);
    for (cell, class) in field.seed.iter().enumerate() {
        if let Some(slot) = buffers.one.get_mut(cell) {
            *slot = words.seeded.get(*class as usize).copied().unwrap_or(0);
        }
    }
    for _ in 2..steps {
        for cell in 0..field.cells {
            let value = valued(kernel, field, words, &buffers.one, cell);
            if let Some(slot) = buffers.two.get_mut(cell) {
                *slot = value;
            }
        }
        std::mem::swap(&mut buffers.one, &mut buffers.two);
    }
    for (cell, wanted) in &field.targets {
        if lanes.alive == 0 {
            return;
        }
        let value = if steps >= 2 {
            valued(kernel, field, words, &buffers.one, *cell)
        } else {
            buffers.one.get(*cell).copied().unwrap_or(0)
        };
        let want = if *wanted { u64::MAX } else { 0 };
        lanes.missed(value ^ want, best);
    }
}

pub fn searched(
    stepping: &Stepping,
    samples: &[Sample<'_>],
    halted: &mut dyn FnMut() -> bool,
) -> Found {
    let classes = stepping.classes();
    let space = stepping.space();
    let lanes_most = space.min(LANES);
    let mask = if lanes_most >= LANES {
        u64::MAX
    } else {
        (1_u64 << lanes_most) - 1
    };
    let batches = space.div_ceil(LANES);
    let kernel = Kernel::of(stepping.counting);
    let fields: Vec<Field> = samples
        .iter()
        .map(|one| field_of(stepping, &kernel, one))
        .collect();
    let cells = fields.iter().map(|one| one.targets.len() as u64).sum();
    let steps = stepping.steps.max(1);
    let probes = probes_of(&kernel, &fields, steps);
    let whole_work: u64 = fields
        .iter()
        .map(|one| one.cells as u64 * u64::from(steps - 1) + one.targets.len() as u64)
        .sum();
    let largest = fields.iter().map(|one| one.cells).max().unwrap_or(0) + 1;
    let mut buffers = Buffers {
        one: vec![0; largest],
        two: vec![0; largest],
    };
    let mut found = Found {
        consistent: 0,
        codes: Vec::new(),
        closest: None,
        checked: 0,
        complete: false,
        cells,
    };
    let mut best = u64::MAX;
    for batch in 0..batches {
        if halted() {
            return found;
        }
        let words = Words::of(&kernel, batch, classes, mask);
        let mut lanes = Lanes {
            alive: mask,
            wrong: [0; LANES as usize],
        };
        if best < PROBES as u64 {
            let mut work = 0_u64;
            for probe in &probes {
                if lanes.alive == 0 || work.saturating_mul(2) > whole_work {
                    break;
                }
                work += probe.work;
                let value = probed(&kernel, &fields, &words, probe, &mut buffers);
                let want = if probe.wanted { u64::MAX } else { 0 };
                lanes.missed(value ^ want, best);
            }
        }
        if lanes.alive != 0 {
            lanes.wrong = [0; LANES as usize];
            for field in &fields {
                if lanes.alive == 0 {
                    break;
                }
                checked_whole(
                    &kernel,
                    field,
                    &words,
                    steps,
                    &mut buffers,
                    &mut lanes,
                    best,
                );
            }
        }
        for lane in 0..lanes_most {
            if (lanes.alive >> lane) & 1 == 0 {
                continue;
            }
            let code = batch * LANES + lane;
            let wrong = lanes.wrong.get(lane as usize).copied().unwrap_or(0);
            best = best.min(wrong);
            if wrong == 0 {
                found.consistent += 1;
                if found.codes.len() < SHOWN_MOST {
                    found.codes.push(code);
                }
            } else if found
                .closest
                .is_none_or(|(least, held)| (wrong, code) < (least, held))
            {
                found.closest = Some((wrong, code));
            }
        }
        found.checked += lanes_most;
    }
    found.complete = true;
    found
}
