use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::netlist::Gate;

pub const OR: u32 = 14;

pub const ORDER: &str = "row bit j is input j, so a row is the sum of input j times 2^j; input j reads read j / positions at stencil place j % positions, and place k is the offset (k % 3 - 1, k / 3 % 3 - 1, k / 9 - 1) as (dx, dy, dz), dz 0 for 3x3; output j is bit j of the row's word";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stencil {
    Square,
    Cube,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Border {
    Zero,
    Wrapped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bit {
    pub plane: u32,
    pub bit: u32,
}

impl Stencil {
    #[must_use]
    pub const fn positions(self) -> u32 {
        match self {
            Self::Square => 9,
            Self::Cube => 27,
        }
    }

    #[must_use]
    pub const fn axes(self) -> usize {
        match self {
            Self::Square => 2,
            Self::Cube => 3,
        }
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Square => "3x3",
            Self::Cube => "3x3x3",
        }
    }

    #[must_use]
    pub const fn offset(self, place: u32) -> [i64; 3] {
        let place = place as i64;
        let dz = match self {
            Self::Square => 0,
            Self::Cube => place / 9 - 1,
        };
        [place % 3 - 1, place / 3 % 3 - 1, dz]
    }

    #[must_use]
    pub const fn place(self, at: [i64; 3]) -> u32 {
        let [x, y, z] = at;
        let layer = match self {
            Self::Square => 0,
            Self::Cube => (z + 1) * 9,
        };
        (layer + (y + 1) * 3 + (x + 1)) as u32
    }

    #[must_use]
    pub fn symmetries(self) -> Vec<Vec<u32>> {
        let axes = self.axes();
        let mut orders: Vec<Vec<usize>> = Vec::new();
        permutations(&mut (0..axes).collect(), 0, &mut orders);
        orders.sort();
        let mut moves = Vec::new();
        for order in &orders {
            for signs in 0..(1_u32 << axes) {
                let moved = (0..self.positions())
                    .map(|place| {
                        let from = self.offset(place);
                        let mut to = from;
                        for (axis, slot) in to.iter_mut().enumerate().take(axes) {
                            let source = order.get(axis).copied().unwrap_or(axis);
                            let value = from.get(source).copied().unwrap_or(0);
                            *slot = if (signs >> axis) & 1 == 1 {
                                -value
                            } else {
                                value
                            };
                        }
                        self.place(to)
                    })
                    .collect();
                moves.push(moved);
            }
        }
        moves
    }
}

fn permutations(held: &mut Vec<usize>, from: usize, out: &mut Vec<Vec<usize>>) {
    if from + 1 >= held.len() {
        out.push(held.clone());
        return;
    }
    for at in from..held.len() {
        held.swap(from, at);
        permutations(held, from + 1, out);
        held.swap(from, at);
    }
}

#[must_use]
pub fn inputs_moved(stencil: Stencil, reads: u32, places: &[u32]) -> Vec<u32> {
    let positions = stencil.positions();
    (0..positions * reads)
        .map(|input| {
            let read = input / positions;
            let place = places
                .get((input % positions) as usize)
                .copied()
                .unwrap_or(0);
            read * positions + place
        })
        .collect()
}

#[must_use]
pub fn image(row: u32, moved: &[u32]) -> u32 {
    moved
        .iter()
        .enumerate()
        .filter(|(input, _)| (row >> input) & 1 == 1)
        .fold(0, |held, (_, to)| held | (1 << to))
}

pub struct Sample<'held> {
    pub size: [u32; 3],
    pub inputs: Vec<&'held [u32]>,
    pub from: [u32; 3],
    pub inner: [u32; 3],
    pub outputs: Vec<&'held [u32]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seen {
    pub outputs: u32,
    pub example: u32,
    pub at: [u32; 3],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rows {
    pub inputs: u32,
    pub seen: BTreeMap<u32, Seen>,
    pub conflicts: BTreeMap<u32, Vec<Seen>>,
    pub cells: u64,
}

#[must_use]
pub fn index(size: [u32; 3], at: [u32; 3]) -> usize {
    (at[2] as usize * size[1] as usize + at[1] as usize) * size[0] as usize + at[0] as usize
}

fn spot_of(size: [u32; 3], at: [i64; 3], border: Border) -> Option<[u32; 3]> {
    let mut spot = [0_u32; 3];
    for ((slot, value), side) in spot.iter_mut().zip(at).zip(size) {
        let side = i64::from(side.max(1));
        let inside = if (0..side).contains(&value) {
            value
        } else if border == Border::Wrapped {
            value.rem_euclid(side)
        } else {
            return None;
        };
        *slot = u32::try_from(inside).ok()?;
    }
    Some(spot)
}

#[must_use]
pub fn row_at(
    stencil: Stencil,
    reads: &[Bit],
    sample: &Sample<'_>,
    at: [u32; 3],
    border: Border,
) -> u32 {
    let positions = stencil.positions();
    let mut row = 0_u32;
    for (read, bit) in reads.iter().enumerate() {
        for place in 0..positions {
            let offset = stencil.offset(place);
            let wanted = [
                i64::from(sample.from[0]) + i64::from(at[0]) + offset[0],
                i64::from(sample.from[1]) + i64::from(at[1]) + offset[1],
                i64::from(sample.from[2]) + i64::from(at[2]) + offset[2],
            ];
            let value = spot_of(sample.size, wanted, border)
                .and_then(|spot| {
                    sample
                        .inputs
                        .get(read)
                        .and_then(|words| words.get(index(sample.size, spot)))
                })
                .map_or(0, |word| (word >> bit.bit) & 1);
            let input = u32::try_from(read).unwrap_or(0) * positions + place;
            row |= value << input;
        }
    }
    row
}

#[must_use]
pub fn outputs_at(writes: &[Bit], sample: &Sample<'_>, at: [u32; 3]) -> u32 {
    writes
        .iter()
        .enumerate()
        .fold(0_u32, |held, (output, spot)| {
            let word = sample
                .outputs
                .get(output)
                .and_then(|words| words.get(index(sample.inner, at)))
                .copied()
                .unwrap_or(0);
            held | (((word >> spot.bit) & 1) << output)
        })
}

#[must_use]
pub fn rows_of(
    stencil: Stencil,
    reads: &[Bit],
    writes: &[Bit],
    samples: &[Sample<'_>],
    border: Border,
) -> Rows {
    let inputs = stencil.positions() * u32::try_from(reads.len()).unwrap_or(0);
    let mut seen: BTreeMap<u32, Seen> = BTreeMap::new();
    let mut conflicts: BTreeMap<u32, Vec<Seen>> = BTreeMap::new();
    let mut cells = 0_u64;
    for (example, sample) in samples.iter().enumerate() {
        let [wide, deep, tall] = sample.inner;
        for z in 0..tall {
            for y in 0..deep {
                for x in 0..wide {
                    let at = [x, y, z];
                    cells += 1;
                    let row = row_at(stencil, reads, sample, at, border);
                    let here = Seen {
                        outputs: outputs_at(writes, sample, at),
                        example: u32::try_from(example).unwrap_or(u32::MAX),
                        at,
                    };
                    match seen.get(&row) {
                        None => {
                            seen.insert(row, here);
                        }
                        Some(first) if first.outputs != here.outputs => {
                            let listed = conflicts.entry(row).or_insert_with(|| vec![*first]);
                            if listed.iter().all(|one| one.outputs != here.outputs) {
                                listed.push(here);
                            }
                        }
                        Some(_) => {}
                    }
                }
            }
        }
    }
    Rows {
        inputs,
        seen,
        conflicts,
        cells,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Orbits {
    pub expanded: BTreeMap<u32, u32>,
    pub broken: BTreeMap<u32, BTreeMap<u32, u32>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cut {
    Rows(usize),
    Halted,
}

pub const CHECK_EVERY: usize = 4096;

pub fn orbits(
    seen: &BTreeMap<u32, Seen>,
    moves: &[Vec<u32>],
    most: usize,
    halted: &mut dyn FnMut() -> bool,
) -> Result<Orbits, Cut> {
    let mut groups: BTreeMap<u32, BTreeMap<u32, u32>> = BTreeMap::new();
    for (done, (row, one)) in seen.iter().enumerate() {
        if done.is_multiple_of(CHECK_EVERY) && halted() {
            return Err(Cut::Halted);
        }
        let least = moves
            .iter()
            .map(|moved| image(*row, moved))
            .min()
            .unwrap_or(*row);
        groups.entry(least).or_default().insert(*row, one.outputs);
    }
    let mut expanded = BTreeMap::new();
    let mut broken = BTreeMap::new();
    let mut done = 0_usize;
    for (least, rows) in groups {
        let values: BTreeSet<u32> = rows.values().copied().collect();
        if values.len() > 1 {
            broken.insert(least, rows);
            continue;
        }
        let Some(value) = values.first().copied() else {
            continue;
        };
        for row in rows.keys() {
            if done.is_multiple_of(CHECK_EVERY) && halted() {
                return Err(Cut::Halted);
            }
            done += 1;
            for moved in moves {
                expanded.insert(image(*row, moved), value);
            }
            if expanded.len() > most {
                return Err(Cut::Rows(expanded.len()));
            }
        }
    }
    Ok(Orbits { expanded, broken })
}

#[must_use]
pub fn live(inputs: u32, outputs: u32, gates: &[Gate]) -> Vec<Gate> {
    let count = gates.len();
    let mut needed = vec![false; count];
    for slot in needed
        .iter_mut()
        .skip(count.saturating_sub(outputs as usize))
    {
        *slot = true;
    }
    for at in (0..count).rev() {
        if !needed.get(at).copied().unwrap_or(false) {
            continue;
        }
        if let Some(gate) = gates.get(at) {
            for source in [gate.left, gate.right] {
                if source >= inputs
                    && let Some(slot) = needed.get_mut((source - inputs) as usize)
                {
                    *slot = true;
                }
            }
        }
    }
    let mut renamed = vec![0_u32; count];
    let mut kept = Vec::new();
    for (at, gate) in gates.iter().enumerate() {
        if !needed.get(at).copied().unwrap_or(false) {
            continue;
        }
        let name = |source: u32| {
            if source < inputs {
                source
            } else {
                renamed
                    .get((source - inputs) as usize)
                    .copied()
                    .unwrap_or(0)
            }
        };
        let moved = Gate {
            kind: gate.kind,
            left: name(gate.left),
            right: name(gate.right),
        };
        if let Some(slot) = renamed.get_mut(at) {
            *slot = inputs + u32::try_from(kept.len()).unwrap_or(0);
        }
        kept.push(moved);
    }
    kept
}

#[must_use]
pub fn symmetrised(inputs: u32, outputs: u32, gates: &[Gate], moves: &[Vec<u32>]) -> Vec<Gate> {
    let size = u32::try_from(gates.len()).unwrap_or(0);
    let mut held = Vec::new();
    for moved in moves {
        let base = u32::try_from(held.len()).unwrap_or(0);
        for gate in gates {
            let name = |source: u32| {
                if source < inputs {
                    moved.get(source as usize).copied().unwrap_or(source)
                } else {
                    source + base
                }
            };
            held.push(Gate {
                kind: gate.kind,
                left: name(gate.left),
                right: name(gate.right),
            });
        }
    }
    let copies = u32::try_from(moves.len()).unwrap_or(0);
    let output_of = |copy: u32, output: u32| inputs + copy * size + size - outputs + output;
    let mut last = Vec::new();
    for output in 0..outputs {
        let mut acc = output_of(0, output);
        for copy in 1..copies {
            let gate = Gate {
                kind: OR,
                left: acc,
                right: output_of(copy, output),
            };
            if copy + 1 == copies {
                last.push(gate);
            } else {
                acc = inputs + u32::try_from(held.len()).unwrap_or(0);
                held.push(gate);
            }
        }
        if copies == 1 {
            last.push(Gate {
                kind: OR,
                left: acc,
                right: acc,
            });
        }
    }
    held.extend(last);
    held
}

#[must_use]
pub fn answer(inputs: u32, outputs: u32, gates: &[Gate], row: u32) -> u32 {
    let mut values: Vec<u32> = Vec::with_capacity(gates.len());
    let value = |values: &Vec<u32>, source: u32| {
        if source < inputs {
            (row >> source) & 1
        } else {
            values.get((source - inputs) as usize).copied().unwrap_or(0)
        }
    };
    for gate in gates {
        let left = value(&values, gate.left);
        let right = value(&values, gate.right);
        values.push((gate.kind >> (left | (right << 1))) & 1);
    }
    let from = values.len().saturating_sub(outputs as usize);
    values
        .iter()
        .skip(from)
        .enumerate()
        .fold(0, |held, (output, bit)| held | (bit << output))
}

#[must_use]
pub fn padded(inputs: u32, gates: &[Gate], budget: u32) -> Vec<Gate> {
    let lead = budget.saturating_sub(u32::try_from(gates.len()).unwrap_or(u32::MAX));
    let moved = |source: u32| {
        if source < inputs {
            source
        } else {
            source + lead
        }
    };
    (0..lead)
        .map(|_| Gate {
            kind: 0,
            left: 0,
            right: 0,
        })
        .chain(gates.iter().map(|gate| Gate {
            kind: gate.kind,
            left: moved(gate.left),
            right: moved(gate.right),
        }))
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Miss {
    pub row: u32,
    pub seen: Seen,
    pub answered: u32,
}

#[must_use]
pub fn missed(inputs: u32, outputs: u32, gates: &[Gate], seen: &BTreeMap<u32, Seen>) -> Vec<Miss> {
    seen.iter()
        .filter_map(|(row, one)| {
            let answered = answer(inputs, outputs, gates, *row);
            (answered != one.outputs).then_some(Miss {
                row: *row,
                seen: *one,
                answered,
            })
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Change {
    pub row: u32,
    pub seen: bool,
    pub was: u32,
    pub now: u32,
}

#[must_use]
pub fn changed(
    inputs: u32,
    outputs: u32,
    (before, after): (&[Gate], &[Gate]),
    seen: &BTreeMap<u32, Seen>,
    every: bool,
) -> Vec<Change> {
    let differing = |row: u32| {
        let was = answer(inputs, outputs, before, row);
        let now = answer(inputs, outputs, after, row);
        (was != now).then_some(Change {
            row,
            seen: seen.contains_key(&row),
            was,
            now,
        })
    };
    let mut held: Vec<Change> = if every {
        (0..1_u32 << inputs).filter_map(differing).collect()
    } else {
        seen.keys().copied().filter_map(differing).collect()
    };
    held.sort_by_key(|one| (!one.seen, one.row));
    held
}

const LOW: [u64; 6] = [
    0xAAAA_AAAA_AAAA_AAAA,
    0xCCCC_CCCC_CCCC_CCCC,
    0xF0F0_F0F0_F0F0_F0F0,
    0xFF00_FF00_FF00_FF00,
    0xFFFF_0000_FFFF_0000,
    0xFFFF_FFFF_0000_0000,
];

const fn kind_of(kind: u32, left: u64, right: u64) -> u64 {
    let mut out = 0_u64;
    if kind & 1 == 1 {
        out |= !left & !right;
    }
    if kind & 2 == 2 {
        out |= left & !right;
    }
    if kind & 4 == 4 {
        out |= !left & right;
    }
    if kind & 8 == 8 {
        out |= left & right;
    }
    out
}

fn block(inputs: u32, gates: &[Gate], number: u64, values: &mut Vec<u64>) {
    values.clear();
    for input in 0..inputs {
        values.push(if input < 6 {
            LOW.get(input as usize).copied().unwrap_or(0)
        } else if (number >> (input - 6)) & 1 == 1 {
            u64::MAX
        } else {
            0
        });
    }
    for gate in gates {
        let left = values.get(gate.left as usize).copied().unwrap_or(0);
        let right = values.get(gate.right as usize).copied().unwrap_or(0);
        values.push(kind_of(gate.kind, left, right));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Count {
    pub inputs: u32,
    pub outputs: u32,
    pub copies: u32,
}

impl Count {
    #[must_use]
    pub const fn blocks(&self) -> u64 {
        (1_u64 << self.inputs).div_ceil(64)
    }

    #[must_use]
    pub const fn law_gates(&self, gates: u32) -> u32 {
        if self.copies > 1 {
            self.copies
                .saturating_mul(gates)
                .saturating_add(self.outputs.saturating_mul(self.copies - 1))
        } else {
            gates
        }
    }

    #[must_use]
    pub const fn units(&self, gates: u32) -> u64 {
        self.blocks()
            .saturating_mul(self.inputs as u64 + self.law_gates(gates) as u64)
    }
}

pub fn ones(
    inputs: u32,
    outputs: u32,
    gates: &[Gate],
    halted: &mut dyn FnMut() -> bool,
) -> Option<Vec<u64>> {
    let rows = 1_u64 << inputs;
    let blocks = rows.div_ceil(64);
    let lanes = if rows < 64 {
        (1_u64 << rows) - 1
    } else {
        u64::MAX
    };
    let mut ones = vec![0_u64; outputs as usize];
    let mut values = Vec::with_capacity(inputs as usize + gates.len());
    let first = (inputs as usize + gates.len()).saturating_sub(outputs as usize);
    for number in 0..blocks {
        if number.is_multiple_of(CHECK_EVERY as u64) && halted() {
            return None;
        }
        block(inputs, gates, number, &mut values);
        for (output, held) in ones.iter_mut().enumerate() {
            let word = values.get(first + output).copied().unwrap_or(0) & lanes;
            *held += u64::from(word.count_ones());
        }
    }
    Some(ones)
}

#[must_use]
pub fn table(inputs: u32, outputs: u32, gates: &[Gate]) -> Vec<u32> {
    (0..1_u32 << inputs)
        .map(|row| answer(inputs, outputs, gates, row))
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tally {
    Totalistic,
    Outer,
}

impl Tally {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Totalistic => "totalistic",
            Self::Outer => "outer",
        }
    }

    #[must_use]
    pub fn counted(self, stencil: Stencil) -> Vec<u32> {
        Counting::moore(self, stencil).counted()
    }

    #[must_use]
    pub fn counts(self, stencil: Stencil) -> u32 {
        Counting::moore(self, stencil).counts()
    }

    #[must_use]
    pub fn classes(self, stencil: Stencil) -> u32 {
        Counting::moore(self, stencil).classes()
    }

    #[must_use]
    pub fn class(self, stencil: Stencil, row: u32) -> u32 {
        Counting::moore(self, stencil).class(row)
    }

    #[must_use]
    pub fn rows_in(self, stencil: Stencil, class: u32) -> u64 {
        Counting::moore(self, stencil).rows_in(class)
    }
}

pub const PRESETS: [&str; 3] = ["moore", "von_neumann", "hex"];

pub const HEX: [[i64; 3]; 6] = [
    [1, 0, 0],
    [-1, 0, 0],
    [0, 1, 0],
    [0, -1, 0],
    [1, -1, 0],
    [-1, 1, 0],
];

#[must_use]
pub fn preset(stencil: Stencil, name: &str) -> Option<u32> {
    let every = (1_u32 << stencil.positions()) - 1;
    let centre = 1_u32 << stencil.place([0, 0, 0]);
    match name {
        "moore" => Some(every),
        "von_neumann" => Some(
            (0..stencil.positions())
                .filter(|place| {
                    stencil
                        .offset(*place)
                        .iter()
                        .map(|one| one.unsigned_abs())
                        .sum::<u64>()
                        <= 1
                })
                .fold(0, |held, place| held | (1 << place)),
        ),
        "hex" if stencil == Stencil::Square => Some(
            HEX.iter()
                .fold(centre, |held, at| held | (1 << stencil.place(*at))),
        ),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Counting {
    pub tally: Tally,
    pub stencil: Stencil,
    pub mask: u32,
}

impl Counting {
    #[must_use]
    pub const fn moore(tally: Tally, stencil: Stencil) -> Self {
        Self {
            tally,
            stencil,
            mask: (1 << stencil.positions()) - 1,
        }
    }

    #[must_use]
    pub fn places(self) -> Vec<u32> {
        (0..self.stencil.positions())
            .filter(|place| (self.mask >> place) & 1 == 1)
            .collect()
    }

    #[must_use]
    pub fn counted(self) -> Vec<u32> {
        let centre = self.stencil.place([0, 0, 0]);
        self.places()
            .into_iter()
            .filter(|place| self.tally == Tally::Totalistic || *place != centre)
            .collect()
    }

    #[must_use]
    pub fn counts(self) -> u32 {
        u32::try_from(self.counted().len()).unwrap_or(0) + 1
    }

    #[must_use]
    pub fn classes(self) -> u32 {
        match self.tally {
            Tally::Totalistic => self.counts(),
            Tally::Outer => 2 * self.counts(),
        }
    }

    #[must_use]
    pub fn class(self, row: u32) -> u32 {
        let n = self
            .counted()
            .iter()
            .filter(|place| (row >> **place) & 1 == 1)
            .count();
        let n = u32::try_from(n).unwrap_or(0);
        match self.tally {
            Tally::Totalistic => n,
            Tally::Outer => {
                let centre = (row >> self.stencil.place([0, 0, 0])) & 1;
                centre * self.counts() + n
            }
        }
    }

    #[must_use]
    pub fn rows_in(self, class: u32) -> u64 {
        let places = u64::from(self.counts() - 1);
        let n = u64::from(class % self.counts());
        let centre = u64::from(self.tally == Tally::Outer);
        let free = u64::from(self.stencil.positions()) - places - centre;
        (0..n).fold(1_u64, |held, at| held * (places - at) / (at + 1)) << free
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Split {
    pub write: u32,
    pub class: u32,
    pub zero: (u32, Seen),
    pub one: (u32, Seen),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tallied {
    pub table: Vec<Option<u32>>,
    pub split: Vec<Split>,
}

const fn order_of(one: &Seen) -> u128 {
    ((one.example as u128) << 96)
        | ((one.at[2] as u128) << 64)
        | ((one.at[1] as u128) << 32)
        | one.at[0] as u128
}

type Marks = Vec<Vec<[Option<(u32, Seen)>; 2]>>;

#[must_use]
pub fn tallied(counting: Counting, outputs: u32, seen: &BTreeMap<u32, Seen>) -> Tallied {
    let classes = counting.classes() as usize;
    let mut table: Vec<Option<u32>> = vec![None; classes];
    let mut marks: Marks = vec![vec![[None, None]; outputs as usize]; classes];
    for (row, one) in seen {
        let class = counting.class(*row) as usize;
        if let Some(slot) = table.get_mut(class) {
            *slot = Some(one.outputs);
        }
        let Some(held) = marks.get_mut(class) else {
            continue;
        };
        for (write, pair) in held.iter_mut().enumerate() {
            let bit = ((one.outputs >> write) & 1) as usize;
            if let Some(mark) = pair.get_mut(bit) {
                let keep = mark
                    .as_ref()
                    .is_none_or(|(_, before)| order_of(one) < order_of(before));
                if keep {
                    *mark = Some((*row, *one));
                }
            }
        }
    }
    let mut split = Vec::new();
    for (class, held) in marks.iter().enumerate() {
        for (write, pair) in held.iter().enumerate() {
            if let [Some(zero), Some(one)] = pair {
                split.push(Split {
                    write: u32::try_from(write).unwrap_or(u32::MAX),
                    class: u32::try_from(class).unwrap_or(u32::MAX),
                    zero: *zero,
                    one: *one,
                });
            }
        }
    }
    Tallied { table, split }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Deviation {
    pub example: u32,
    pub at: [u32; 3],
    pub class: u32,
    pub seen: u32,
    pub law: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Voted {
    pub tallies: Vec<Vec<[u64; 2]>>,
    pub table: Vec<Option<u32>>,
    pub ties: Vec<Vec<u32>>,
    pub deviating: Vec<u64>,
    pub deviations: Vec<Vec<Deviation>>,
    pub cells: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct Voting<'held> {
    pub counting: Counting,
    pub reads: &'held [Bit],
    pub writes: &'held [Bit],
    pub border: Border,
}

fn each_cell(samples: &[Sample<'_>], visit: &mut dyn FnMut(u32, [u32; 3], &Sample<'_>)) {
    for (example, sample) in samples.iter().enumerate() {
        let example = u32::try_from(example).unwrap_or(u32::MAX);
        let [wide, deep, tall] = sample.inner;
        for z in 0..tall {
            for y in 0..deep {
                for x in 0..wide {
                    visit(example, [x, y, z], sample);
                }
            }
        }
    }
}

#[must_use]
pub fn voted(voting: Voting<'_>, samples: &[Sample<'_>], most: usize) -> Voted {
    let Voting {
        counting,
        reads,
        writes,
        border,
    } = voting;
    let stencil = counting.stencil;
    let classes = counting.classes() as usize;
    let mut tallies: Vec<Vec<[u64; 2]>> = vec![vec![[0, 0]; classes]; writes.len()];
    let mut shown = vec![false; classes];
    let mut cells = 0_u64;
    each_cell(samples, &mut |_, at, sample| {
        cells += 1;
        let class = counting.class(row_at(stencil, reads, sample, at, border)) as usize;
        if let Some(slot) = shown.get_mut(class) {
            *slot = true;
        }
        let outputs = outputs_at(writes, sample, at);
        for (write, held) in tallies.iter_mut().enumerate() {
            let bit = ((outputs >> write) & 1) as usize;
            if let Some(count) = held.get_mut(class).and_then(|pair| pair.get_mut(bit)) {
                *count += 1;
            }
        }
    });
    let mut ties: Vec<Vec<u32>> = vec![Vec::new(); writes.len()];
    let table: Vec<Option<u32>> = shown
        .iter()
        .enumerate()
        .map(|(class, seen)| {
            seen.then(|| {
                tallies.iter().zip(ties.iter_mut()).enumerate().fold(
                    0_u32,
                    |word, (write, (held, tied))| {
                        let [zeros, ones] = held.get(class).copied().unwrap_or([0, 0]);
                        if zeros == ones {
                            tied.push(u32::try_from(class).unwrap_or(u32::MAX));
                        }
                        word | (u32::from(ones > zeros) << write)
                    },
                )
            })
        })
        .collect();
    let mut deviating = vec![0_u64; writes.len()];
    let mut deviations: Vec<Vec<Deviation>> = vec![Vec::new(); writes.len()];
    each_cell(samples, &mut |example, at, sample| {
        let class = counting.class(row_at(stencil, reads, sample, at, border));
        let law = table.get(class as usize).copied().flatten().unwrap_or(0);
        let outputs = outputs_at(writes, sample, at);
        for (write, (count, held)) in deviating.iter_mut().zip(deviations.iter_mut()).enumerate() {
            let seen = (outputs >> write) & 1;
            let wanted = (law >> write) & 1;
            if seen != wanted {
                *count += 1;
                if held.len() < most {
                    held.push(Deviation {
                        example,
                        at,
                        class,
                        seen,
                        law: wanted,
                    });
                }
            }
        }
    });
    Voted {
        tallies,
        table,
        ties,
        deviating,
        deviations,
        cells,
    }
}

struct Wires {
    inputs: u32,
    gates: Vec<Gate>,
}

impl Wires {
    fn gate(&mut self, kind: u32, left: u32, right: u32) -> u32 {
        let name = self.inputs + u32::try_from(self.gates.len()).unwrap_or(u32::MAX);
        self.gates.push(Gate { kind, left, right });
        name
    }

    fn constant(&mut self, value: bool) -> u32 {
        self.gate(if value { 15 } else { 0 }, 0, 0)
    }

    fn summed(&mut self, places: &[u32]) -> Vec<u32> {
        let mut columns: Vec<VecDeque<u32>> = vec![places.iter().copied().collect()];
        let mut weight = 0;
        while let Some(column) = columns.get_mut(weight) {
            let (sum, carry) = if column.len() >= 3 {
                let (Some(a), Some(b), Some(c)) =
                    (column.pop_front(), column.pop_front(), column.pop_front())
                else {
                    break;
                };
                let ab = self.gate(6, a, b);
                let sum = self.gate(6, ab, c);
                let both = self.gate(8, a, b);
                let other = self.gate(8, ab, c);
                (sum, self.gate(OR, both, other))
            } else if column.len() == 2 {
                let (Some(a), Some(b)) = (column.pop_front(), column.pop_front()) else {
                    break;
                };
                (self.gate(6, a, b), self.gate(8, a, b))
            } else {
                weight += 1;
                continue;
            };
            if let Some(column) = columns.get_mut(weight) {
                column.push_back(sum);
            }
            match columns.get_mut(weight + 1) {
                Some(next) => next.push_back(carry),
                None => columns.push(VecDeque::from([carry])),
            }
        }
        let mut bits = Vec::with_capacity(columns.len());
        for column in &columns {
            let bit = match column.front() {
                Some(bit) => *bit,
                None => self.constant(false),
            };
            bits.push(bit);
        }
        bits
    }

    fn decoded(&mut self, bits: &[u32], limit: u32) -> Vec<u32> {
        match bits {
            [] => vec![self.constant(true)],
            [bit] => {
                let not = self.gate(5, *bit, *bit);
                [not, *bit].into_iter().take(limit as usize).collect()
            }
            _ => {
                let half = bits.len() / 2;
                let (low, high) = bits.split_at(half);
                let lows = self.decoded(low, 1 << half);
                let highs = self.decoded(high, limit.div_ceil(1 << half));
                let mask = (1_u32 << half) - 1;
                (0..limit)
                    .map(|value| {
                        let left = lows.get((value & mask) as usize).copied().unwrap_or(0);
                        let right = highs.get((value >> half) as usize).copied().unwrap_or(0);
                        self.gate(8, left, right)
                    })
                    .collect()
            }
        }
    }

    fn joined(&mut self, lines: &[u32]) -> u32 {
        let mut held: Option<u32> = None;
        for line in lines {
            held = Some(match held {
                None => *line,
                Some(before) => self.gate(OR, before, *line),
            });
        }
        held.unwrap_or_else(|| self.constant(false))
    }

    fn picked(&mut self, lines: &[u32], wanted: &[bool]) -> u32 {
        let side = |value: bool| -> Vec<u32> {
            lines
                .iter()
                .zip(wanted)
                .filter(|(_, want)| **want == value)
                .map(|(line, _)| *line)
                .collect()
        };
        let ones = side(true);
        let zeros = side(false);
        if zeros.is_empty() {
            return self.constant(true);
        }
        if ones.len() <= zeros.len() {
            return self.joined(&ones);
        }
        let none = self.joined(&zeros);
        self.gate(5, none, none)
    }
}

#[must_use]
pub fn tally_gates(tally: Tally, stencil: Stencil, outputs: u32, table: &[u32]) -> Vec<Gate> {
    counted_gates(Counting::moore(tally, stencil), outputs, table)
}

#[must_use]
pub fn counted_gates(counting: Counting, outputs: u32, table: &[u32]) -> Vec<Gate> {
    let Counting { tally, stencil, .. } = counting;
    let inputs = stencil.positions();
    let mut wires = Wires {
        inputs,
        gates: Vec::new(),
    };
    let counts = counting.counts();
    let bits = wires.summed(&counting.counted());
    let lines = wires.decoded(&bits, counts);
    let centre = stencil.place([0, 0, 0]);
    let mut outs = Vec::with_capacity(outputs as usize);
    for write in 0..outputs {
        let wanted = |from: u32| -> Vec<bool> {
            (from..from + counts)
                .map(|class| {
                    table
                        .get(class as usize)
                        .is_some_and(|word| (word >> write) & 1 == 1)
                })
                .collect()
        };
        let out = match tally {
            Tally::Totalistic => wires.picked(&lines, &wanted(0)),
            Tally::Outer => {
                let off = wires.picked(&lines, &wanted(0));
                let on = wires.picked(&lines, &wanted(counts));
                let lit = wires.gate(8, centre, on);
                let dark = wires.gate(4, centre, off);
                wires.gate(OR, lit, dark)
            }
        };
        outs.push(out);
    }
    for out in outs {
        wires.gate(10, out, out);
    }
    live(inputs, outputs, &wires.gates)
}
