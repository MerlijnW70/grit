use std::collections::{BTreeMap, BTreeSet};

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

#[must_use]
pub fn orbits(seen: &BTreeMap<u32, Seen>, moves: &[Vec<u32>]) -> Orbits {
    let mut groups: BTreeMap<u32, BTreeMap<u32, u32>> = BTreeMap::new();
    for (row, one) in seen {
        let images: Vec<u32> = moves.iter().map(|moved| image(*row, moved)).collect();
        let least = images.iter().copied().min().unwrap_or(*row);
        groups.entry(least).or_default().insert(*row, one.outputs);
    }
    let mut expanded = BTreeMap::new();
    let mut broken = BTreeMap::new();
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
            for moved in moves {
                expanded.insert(image(*row, moved), value);
            }
        }
    }
    Orbits { expanded, broken }
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

#[must_use]
pub fn ones(inputs: u32, outputs: u32, gates: &[Gate]) -> Vec<u64> {
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
        block(inputs, gates, number, &mut values);
        for (output, held) in ones.iter_mut().enumerate() {
            let word = values.get(first + output).copied().unwrap_or(0) & lanes;
            *held += u64::from(word.count_ones());
        }
    }
    ones
}

#[must_use]
pub fn table(inputs: u32, outputs: u32, gates: &[Gate]) -> Vec<u32> {
    (0..1_u32 << inputs)
        .map(|row| answer(inputs, outputs, gates, row))
        .collect()
}
