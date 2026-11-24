use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, U32};
use simdr::module::{Id, op};
use simdr::spec::{MemorySemantics, Scope};

use crate::bricks::{self, Bricks, ENTRY, UNIFORM, VALUE};
use crate::geometry::Arith;
use crate::host;
use crate::layout::Windows;
use crate::live::{lane_if, state_load};
use crate::resident::Groups;

use super::{Failed, Refused, World, sha256};

pub const SET_MOST: u32 = 16_384;

pub const MARKS_HELD: usize = 16;

const COUNT: u32 = 0;
const OVERFLOWED: u32 = 1;
const DROPPED: u32 = 2;
const BOX_WIDE: u32 = 3;
const BOX_DEEP: u32 = 4;
const BOX_CELLS: u32 = 5;
const CORNER: u32 = 6;
const START: u32 = 9;
const VALUES: u32 = 10;
const STRIDE: u32 = 11;
const MARKED: u32 = 12;
const ASKED: u32 = 12;
const HEAD: u32 = 16;
const ANSWERED: usize = 3;
const RECORD: u32 = 7;
pub const BITS: u32 = 32;
const TALLY_WORDS: u32 = HEAD + BITS * RECORD;
const RUN_LEAST: u32 = 32;
const ATOMIC_U_MIN: u16 = 237;
const ATOMIC_U_MAX: u16 = 239;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scanner {
    binding: usize,
    pass: usize,
    cap: u32,
    trips: u32,
    lanes: u32,
    run: u32,
}

impl Scanner {
    #[must_use]
    pub const fn words(&self) -> usize {
        (HEAD + 4 * self.cap) as usize
    }

    #[must_use]
    pub fn bound(&self, sizes: &[usize]) -> Vec<usize> {
        let mut bound = sizes.to_vec();
        bound.resize(self.binding, 1);
        bound.push(self.words());
        bound.push(TALLY_WORDS as usize);
        bound
    }

    #[must_use]
    pub const fn cap(&self) -> u32 {
        self.cap
    }

    const fn entries(&self) -> u32 {
        HEAD + 2 * self.cap
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plane {
    Numbered(u32),
    Current,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mark {
    pub id: String,
    pub plane: Plane,
    pub from: [u32; 3],
    pub to: [u32; 3],
    pub cells: Vec<(u32, u32)>,
}

impl Mark {
    #[must_use]
    pub fn new(plane: Plane, from: [u32; 3], to: [u32; 3], cells: Vec<(u32, u32)>) -> Self {
        let mut bytes = b"grit mark\0".to_vec();
        let (kind, number) = match plane {
            Plane::Numbered(number) => (0_u32, number),
            Plane::Current => (1, 0),
        };
        for word in [kind, number].iter().chain(&from).chain(&to) {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        for (index, word) in &cells {
            bytes.extend_from_slice(&index.to_le_bytes());
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        let id = sha256(&bytes)
            .iter()
            .take(8)
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Self {
            id,
            plane,
            from,
            to,
            cells,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tallied {
    pub count: u32,
    pub least: [u32; 3],
    pub most: [u32; 3],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scanned {
    pub count: u64,
    pub cells: Option<Vec<(u32, u32)>>,
}

pub(crate) enum Source {
    Dense(Windows),
    Sparse(Bricks),
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

struct Built {
    binding: u32,
    size: [u32; 3],
    cap: u32,
    source: Source,
}

fn constant(kernel: &mut Kernel<U32>, value: u32) -> Result<Id, LaneError> {
    Ok(kernel.module().constant_u32(value)?)
}

fn head(kernel: &mut Kernel<U32>, binding: u32, at: u32) -> Result<Id, LaneError> {
    let place = constant(kernel, at)?;
    kernel.load_at(binding, place)
}

fn compared(kernel: &mut Kernel<U32>, code: u16, left: Id, right: Id) -> Result<Id, LaneError> {
    let truth = kernel.module().type_bool()?;
    Ok(kernel.module().binary(code, truth, left, right)?)
}

fn word_at(
    kernel: &mut Kernel<U32>,
    built: &Built,
    point: [Id; 3],
    index: Id,
) -> Result<Id, LaneError> {
    let uint = kernel.index_type();
    let start = head(kernel, built.binding, START)?;
    match &built.source {
        Source::Dense(windows) => {
            let at = kernel.module().i_add(uint, start, index)?;
            state_load(kernel, *windows, at)
        }
        Source::Sparse(held) => {
            let cell = held.place(&mut Scalar(kernel), point)?;
            let table = held.table();
            let four = constant(kernel, ENTRY as u32)?;
            let entry = kernel.module().i_mul(uint, cell.brick, four)?;
            let slot = kernel.load_at(table, entry)?;
            let values = head(kernel, built.binding, VALUES)?;
            let stride = head(kernel, built.binding, STRIDE)?;
            let along = kernel.module().i_mul(uint, cell.brick, stride)?;
            let value_at = kernel.module().i_add(uint, values, along)?;
            let value = kernel.load_at(table, value_at)?;
            let uniform = constant(kernel, UNIFORM)?;
            let held_slot = compared(kernel, op::I_NOT_EQUAL, slot, uniform)?;
            let nothing = constant(kernel, 0)?;
            let safe = kernel.module().select(uint, held_slot, slot, nothing)?;
            let per = constant(kernel, held.cells() as u32)?;
            let base = kernel.module().i_mul(uint, safe, per)?;
            let within = kernel.module().i_add(uint, base, cell.offset)?;
            let at = kernel.module().i_add(uint, start, within)?;
            let pooled = state_load(kernel, held.windows(), at)?;
            Ok(kernel.module().select(uint, held_slot, pooled, value)?)
        }
    }
}

fn marked_word(kernel: &mut Kernel<U32>, built: &Built, index: Id) -> Result<Id, LaneError> {
    let uint = kernel.index_type();
    let binding = built.binding;
    let marked = head(kernel, binding, MARKED)?;
    let nothing = constant(kernel, 0)?;
    let one = constant(kernel, 1)?;
    let two = constant(kernel, 2)?;
    let entries_at = constant(kernel, HEAD)?;
    let steps = 32 - built.cap.leading_zeros();
    let found = kernel.repeat_rolled_many(steps, uint, &[nothing, marked], |kernel, held, _| {
        let (Some(&low), Some(&high)) = (held.first(), held.get(1)) else {
            return Err(LaneError::BadCarry {
                given: held.len(),
                wanted: 2,
            });
        };
        let open = compared(kernel, op::U_LESS_THAN, low, high)?;
        let sum = kernel.module().i_add(uint, low, high)?;
        let middle = kernel
            .module()
            .binary(op::SHIFT_RIGHT_LOGICAL, uint, sum, one)?;
        let doubled = kernel.module().i_mul(uint, middle, two)?;
        let at = kernel.module().i_add(uint, entries_at, doubled)?;
        let key = kernel.load_at(binding, at)?;
        let below = compared(kernel, op::U_LESS_THAN, key, index)?;
        let above = compared(kernel, op::U_GREATER_THAN_EQUAL, key, index)?;
        let truth = kernel.module().type_bool()?;
        let up = kernel.module().logical_and(truth, open, below)?;
        let down = kernel.module().logical_and(truth, open, above)?;
        let past = kernel.module().i_add(uint, middle, one)?;
        let low = kernel.module().select(uint, up, past, low)?;
        let high = kernel.module().select(uint, down, middle, high)?;
        Ok(vec![low, high])
    })?;
    let low = found.first().copied().ok_or(LaneError::BadCarry {
        given: found.len(),
        wanted: 2,
    })?;
    let doubled = kernel.module().i_mul(uint, low, two)?;
    let at = kernel.module().i_add(uint, entries_at, doubled)?;
    let key = kernel.load_at(binding, at)?;
    let next_word = kernel.module().i_add(uint, at, one)?;
    let word = kernel.load_at(binding, next_word)?;
    let within = compared(kernel, op::U_LESS_THAN, low, marked)?;
    let equal = compared(kernel, op::I_EQUAL, key, index)?;
    let truth = kernel.module().type_bool()?;
    let hit = kernel.module().logical_and(truth, within, equal)?;
    Ok(kernel.module().select(uint, hit, word, nothing)?)
}

fn placed(
    kernel: &mut Kernel<U32>,
    built: &Built,
    cell: Id,
) -> Result<(Id, [Id; 3], Id), LaneError> {
    let uint = kernel.index_type();
    let binding = built.binding;
    let cells = head(kernel, binding, BOX_CELLS)?;
    let inside = compared(kernel, op::U_LESS_THAN, cell, cells)?;
    let wide = head(kernel, binding, BOX_WIDE)?;
    let deep = head(kernel, binding, BOX_DEEP)?;
    let row = kernel.module().u_div(uint, cell, wide)?;
    let back = kernel.module().i_mul(uint, row, wide)?;
    let x = kernel.module().i_sub(uint, cell, back)?;
    let z = kernel.module().u_div(uint, row, deep)?;
    let back = kernel.module().i_mul(uint, z, deep)?;
    let y = kernel.module().i_sub(uint, row, back)?;
    let nothing = constant(kernel, 0)?;
    let mut point = [x, y, z];
    for (axis, value) in point.iter_mut().enumerate() {
        let corner = head(kernel, binding, CORNER + axis as u32)?;
        let placed = kernel.module().i_add(uint, corner, *value)?;
        *value = kernel.module().select(uint, inside, placed, nothing)?;
    }
    let [wide_world, deep_world, _] = built.size;
    let deep_world = constant(kernel, deep_world)?;
    let wide_world = constant(kernel, wide_world)?;
    let layer = kernel.module().i_mul(uint, point[2], deep_world)?;
    let layer = kernel.module().i_add(uint, layer, point[1])?;
    let index = kernel.module().i_mul(uint, layer, wide_world)?;
    let index = kernel.module().i_add(uint, index, point[0])?;
    Ok((inside, point, index))
}

fn one_cell(kernel: &mut Kernel<U32>, built: &Built, cell: Id) -> Result<(), LaneError> {
    let binding = built.binding;
    let (inside, point, index) = placed(kernel, built, cell)?;
    let word = word_at(kernel, built, point, index)?;
    let was = marked_word(kernel, built, index)?;
    let other = compared(kernel, op::I_NOT_EQUAL, word, was)?;
    let truth = kernel.module().type_bool()?;
    let differs = kernel.module().logical_and(truth, inside, other)?;
    let cap = built.cap;
    lane_if(kernel, differs, |kernel| {
        let uint = kernel.index_type();
        let counted = constant(kernel, COUNT)?;
        let one = constant(kernel, 1)?;
        let slot = kernel.atomic_add_at(binding, counted, one)?;
        let room = constant(kernel, cap)?;
        let fits = compared(kernel, op::U_LESS_THAN, slot, room)?;
        lane_if(kernel, fits, |kernel| {
            let two = constant(kernel, 2)?;
            let doubled = kernel.module().i_mul(uint, slot, two)?;
            let out_at = constant(kernel, HEAD + 2 * cap)?;
            let at = kernel.module().i_add(uint, out_at, doubled)?;
            kernel.store_at(binding, at, index)?;
            let one = constant(kernel, 1)?;
            let next_word = kernel.module().i_add(uint, at, one)?;
            kernel.store_at(binding, next_word, word)
        })
    })
}

fn opened(subgroup: u32, built: &Built) -> Result<(Kernel<U32>, Id), LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, built.binding + 1))?;
    let uint = kernel.index_type();
    let group = kernel.workgroup_index();
    let lane = kernel.local_index();
    let width = constant(&mut kernel, subgroup)?;
    let base = kernel.module().i_mul(uint, group, width)?;
    let lane_at = kernel.module().i_add(uint, base, lane)?;
    if let Source::Sparse(held) = &built.source {
        let slots = held.slots();
        let nothing = constant(&mut kernel, 0)?;
        let leader = compared(&mut kernel, op::I_EQUAL, lane_at, nothing)?;
        lane_if(&mut kernel, leader, |kernel| {
            for (from, into) in [(bricks::OVERFLOW, OVERFLOWED), (bricks::LOST, DROPPED)] {
                let at = constant(kernel, from as u32)?;
                let flag = kernel.load_at(slots, at)?;
                let place = constant(kernel, into)?;
                kernel.store_at(built.binding, place, flag)?;
            }
            Ok(())
        })?;
    }
    Ok((kernel, lane_at))
}

fn module(subgroup: u32, built: &Built, trips: u32) -> Result<Vec<u32>, LaneError> {
    let (mut kernel, lane_at) = opened(subgroup, built)?;
    let uint = kernel.index_type();
    if trips <= 1 {
        one_cell(&mut kernel, built, lane_at)?;
    } else {
        let per = constant(&mut kernel, trips)?;
        let begin = kernel.module().i_mul(uint, lane_at, per)?;
        let nothing = constant(&mut kernel, 0)?;
        kernel.repeat_rolled(trips, uint, nothing, |kernel, carried, counter| {
            let uint = kernel.index_type();
            let cell = kernel.module().i_add(uint, begin, counter)?;
            one_cell(kernel, built, cell)?;
            Ok(carried)
        })?;
    }
    kernel.finish()
}

fn bitwise(kernel: &mut Kernel<U32>, code: u16, left: Id, right: Id) -> Result<Id, LaneError> {
    let uint = kernel.index_type();
    Ok(kernel.module().binary(code, uint, left, right)?)
}

fn atomic_at(
    kernel: &mut Kernel<U32>,
    code: u16,
    binding: u32,
    index: Id,
    value: Id,
) -> Result<Id, LaneError> {
    let pointer = kernel.element_pointer_to(binding, index)?;
    let element = kernel.element();
    let scope = kernel.module().scope(Scope::Device)?;
    let semantics = kernel.module().memory_semantics(MemorySemantics::None)?;
    Ok(kernel
        .module()
        .atomic(code, element, pointer, scope, semantics, value)?)
}

fn tally_cell(
    kernel: &mut Kernel<U32>,
    built: &Built,
    begin: Id,
    counter: Id,
) -> Result<([Id; 3], Id), LaneError> {
    let uint = kernel.index_type();
    let cell = kernel.module().i_add(uint, begin, counter)?;
    let (inside, point, index) = placed(kernel, built, cell)?;
    let word = word_at(kernel, built, point, index)?;
    let asked = head(kernel, built.binding, ASKED)?;
    let kept = bitwise(kernel, op::BITWISE_AND, word, asked)?;
    let nothing = constant(kernel, 0)?;
    let word = kernel.module().select(uint, inside, kept, nothing)?;
    Ok((point, word))
}

fn bit_record(
    kernel: &mut Kernel<U32>,
    built: &Built,
    begin: Id,
    bit: Id,
    run: u32,
) -> Result<(), LaneError> {
    let uint = kernel.index_type();
    let nothing = constant(kernel, 0)?;
    let one = constant(kernel, 1)?;
    let full = constant(kernel, u32::MAX)?;
    let initial = [nothing, full, full, full, nothing, nothing, nothing];
    let carried = kernel.repeat_rolled_many(run, uint, &initial, |kernel, held, counter| {
        let missing = || LaneError::BadCarry {
            given: held.len(),
            wanted: RECORD as usize,
        };
        let (point, word) = tally_cell(kernel, built, begin, counter)?;
        let shifted = bitwise(kernel, op::SHIFT_RIGHT_LOGICAL, word, bit)?;
        let low = bitwise(kernel, op::BITWISE_AND, shifted, one)?;
        let set = compared(kernel, op::I_NOT_EQUAL, low, nothing)?;
        let count = held.first().copied().ok_or_else(missing)?;
        let mut next = vec![kernel.module().i_add(uint, count, low)?];
        for slot in 1..RECORD as usize {
            let was = held.get(slot).copied().ok_or_else(missing)?;
            let at = point.get((slot - 1) % 3).copied().ok_or_else(missing)?;
            let code = if slot <= 3 {
                op::U_LESS_THAN
            } else {
                op::U_GREATER_THAN
            };
            let beyond = compared(kernel, code, at, was)?;
            let truth = kernel.module().type_bool()?;
            let taken = kernel.module().logical_and(truth, set, beyond)?;
            next.push(kernel.module().select(uint, taken, at, was)?);
        }
        Ok(next)
    })?;
    let width = constant(kernel, RECORD)?;
    let start = constant(kernel, HEAD)?;
    let along = kernel.module().i_mul(uint, bit, width)?;
    let record = kernel.module().i_add(uint, start, along)?;
    for (slot, value) in (0_u32..).zip(&carried) {
        let offset = constant(kernel, slot)?;
        let at = kernel.module().i_add(uint, record, offset)?;
        let (code, beyond) = match slot {
            0 => {
                atomic_at(kernel, op::ATOMIC_I_ADD, built.binding, at, *value)?;
                continue;
            }
            1..=3 => (ATOMIC_U_MIN, op::U_LESS_THAN),
            _ => (ATOMIC_U_MAX, op::U_GREATER_THAN),
        };
        let held = kernel.atomic_load_at(built.binding, at)?;
        let better = compared(kernel, beyond, *value, held)?;
        lane_if(kernel, better, |kernel| {
            atomic_at(kernel, code, built.binding, at, *value).map(|_| ())
        })?;
    }
    Ok(())
}

fn tally_module(subgroup: u32, built: &Built, run: u32) -> Result<Vec<u32>, LaneError> {
    let (mut kernel, lane_at) = opened(subgroup, built)?;
    let uint = kernel.index_type();
    let per = constant(&mut kernel, run)?;
    let begin = kernel.module().i_mul(uint, lane_at, per)?;
    let nothing = constant(&mut kernel, 0)?;
    let held = kernel.repeat_rolled(run, uint, nothing, |kernel, carried, counter| {
        let (_, word) = tally_cell(kernel, built, begin, counter)?;
        bitwise(kernel, op::BITWISE_OR, carried, word)
    })?;
    kernel.repeat_rolled(BITS, uint, nothing, |kernel, carried, bit| {
        let one = constant(kernel, 1)?;
        let zero = constant(kernel, 0)?;
        let shifted = bitwise(kernel, op::SHIFT_RIGHT_LOGICAL, held, bit)?;
        let low = bitwise(kernel, op::BITWISE_AND, shifted, one)?;
        let on = compared(kernel, op::I_NOT_EQUAL, low, zero)?;
        lane_if(kernel, on, |kernel| {
            bit_record(kernel, built, begin, bit, run)
        })?;
        Ok(carried)
    })?;
    kernel.finish()
}

pub(crate) fn scanner(
    subgroup: u32,
    binding: usize,
    pass: usize,
    size: [u32; 3],
    source: Source,
) -> Result<(Scanner, [Vec<u32>; 2]), Failed> {
    let plane: u64 = size.iter().map(|side| u64::from(*side)).product();
    let cap = u32::try_from(plane.min(u64::from(SET_MOST)))?;
    let reach = u64::from(subgroup.max(1)) * u64::from(bricks::MOST_GROUPS);
    let trips = u32::try_from(plane.div_ceil(reach).max(1))?;
    let run = u32::try_from(plane.div_ceil(reach).max(u64::from(RUN_LEAST)))?;
    let built = Built {
        binding: u32::try_from(binding)?,
        size,
        cap,
        source,
    };
    let words = module(subgroup, &built, trips)?;
    let tallied = Built {
        binding: built.binding + 1,
        ..built
    };
    let tally = tally_module(subgroup, &tallied, run)?;
    Ok((
        Scanner {
            binding,
            pass,
            cap,
            trips,
            lanes: subgroup.max(1),
            run,
        },
        [words, tally],
    ))
}

impl World<'_> {
    fn located(&self, plane: u32, from: [u32; 3], to: [u32; 3]) -> Result<(Vec<u32>, u64), Failed> {
        if plane >= self.planes() {
            return Err(Box::new(Refused::Malformed {
                what: format!("plane {plane} is past the world's {} planes", self.planes()),
            }));
        }
        let size = self.size();
        for at in [from, to] {
            if at.iter().zip(size).any(|(one, side)| *one >= side) {
                return Err(Box::new(Refused::Outside { at, size }));
            }
        }
        if from.iter().zip(to).any(|(one, other)| *one > other) {
            return Err(Box::new(Refused::Malformed {
                what: format!("the box from {from:?} to {to:?} has its first corner past its last"),
            }));
        }
        let mut sides = [1_u32; 3];
        for ((side, one), other) in sides.iter_mut().zip(from).zip(to) {
            *side = other - one + 1;
        }
        let cells: u64 = sides.iter().map(|side| u64::from(*side)).product();
        let (start, values, stride) = match self.pool {
            Some(held) => {
                let (values, stride) = if plane == 0 {
                    (VALUE as u32, ENTRY as u32)
                } else {
                    (held.plane_values(plane as usize), 1)
                };
                (held.plane_start(plane as usize), values, stride)
            }
            None => (u32::try_from(plane as usize * self.grid.cells())?, 0, 0),
        };
        let mut words = vec![0_u32; HEAD as usize];
        for (at, value) in [
            (BOX_WIDE, sides[0]),
            (BOX_DEEP, sides[1]),
            (BOX_CELLS, u32::try_from(cells)?),
            (CORNER, from[0]),
            (CORNER + 1, from[1]),
            (CORNER + 2, from[2]),
            (START, start),
            (VALUES, values),
            (STRIDE, stride),
        ] {
            if let Some(slot) = words.get_mut(at as usize) {
                *slot = value;
            }
        }
        Ok((words, cells))
    }

    fn flagged(&self, got: &[u32]) -> Result<(), Failed> {
        if got.get(OVERFLOWED as usize).copied().unwrap_or_default() != 0 {
            return Err(Box::new(host::Error::PoolFull {
                slots: self.pool.map_or(0, |held| held.capacity()),
            }));
        }
        if got.get(DROPPED as usize).copied().unwrap_or_default() != 0 {
            return Err(Box::new(host::Error::LostWrite));
        }
        Ok(())
    }

    pub fn bit_tally(
        &mut self,
        plane: u32,
        from: [u32; 3],
        to: [u32; 3],
        asked: u32,
    ) -> Result<Vec<Tallied>, Failed> {
        let (mut words, cells) = self.located(plane, from, to)?;
        let scanner = self.scanner;
        let per = u64::from(scanner.lanes) * u64::from(scanner.run);
        let groups = u32::try_from(cells.div_ceil(per).max(1))?;
        if let Some(slot) = words.get_mut(ASKED as usize) {
            *slot = asked;
        }
        for _ in 0..BITS {
            words.extend_from_slice(&[0, u32::MAX, u32::MAX, u32::MAX, 0, 0, 0]);
        }
        let binding = scanner.binding + 1;
        self.live.write(binding, &words)?;
        let quiet = vec![Groups::Fixed(0); scanner.pass + 1];
        let run = [Groups::Fixed(groups)];
        let rest = self.after_scanner();
        let got = self
            .live
            .run_reading(&[&quiet, &run, &rest], binding, TALLY_WORDS as usize)?;
        self.flagged(&got)?;
        let records = got.get(HEAD as usize..).unwrap_or_default();
        Ok(records
            .as_chunks::<{ RECORD as usize }>()
            .0
            .iter()
            .map(|record| Tallied {
                count: record[0],
                least: [record[1], record[2], record[3]],
                most: [record[4], record[5], record[6]],
            })
            .collect())
    }

    pub fn scan(
        &mut self,
        plane: u32,
        from: [u32; 3],
        to: [u32; 3],
        against: &[(u32, u32)],
        most: u32,
    ) -> Result<Scanned, Failed> {
        let (head_words, cells) = self.located(plane, from, to)?;
        let scanner = self.scanner;
        let sorted = against.windows(2).all(|pair| match pair {
            [one, other] => one.0 < other.0,
            _ => true,
        });
        if against.len() > scanner.cap as usize || !sorted {
            return Err(Box::new(Refused::Malformed {
                what: format!(
                    "a mark of {} cells, sorted {sorted}, past the {} a scan compares",
                    against.len(),
                    scanner.cap
                ),
            }));
        }
        let mut words = head_words;
        if let Some(slot) = words.get_mut(MARKED as usize) {
            *slot = u32::try_from(against.len())?;
        }
        for &(index, word) in against {
            words.push(index);
            words.push(word);
        }
        self.live.write(scanner.binding, &words)?;
        let per = u64::from(scanner.lanes) * u64::from(scanner.trips);
        let groups = u32::try_from(cells.div_ceil(per).max(1))?;
        let quiet = vec![Groups::Fixed(0); scanner.pass];
        let run = [Groups::Fixed(groups)];
        let tally = [Groups::Fixed(0)];
        let rest = self.after_scanner();
        let got =
            self.live
                .run_reading(&[&quiet, &run, &tally, &rest], scanner.binding, ANSWERED)?;
        let count = got.first().copied().unwrap_or_default();
        self.flagged(&got)?;
        if count > most.min(scanner.cap) {
            return Ok(Scanned {
                count: u64::from(count),
                cells: None,
            });
        }
        let pairs = self.live.read_at(
            scanner.binding,
            scanner.entries() as usize,
            2 * count as usize,
        )?;
        let mut found: Vec<(u32, u32)> = pairs
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| (pair[0], pair[1]))
            .collect();
        found.sort_unstable();
        Ok(Scanned {
            count: u64::from(count),
            cells: Some(found),
        })
    }

    fn after_scanner(&self) -> Vec<Groups> {
        vec![Groups::Fixed(0); self.passes.len().saturating_sub(self.scanner.pass + 2)]
    }

    #[must_use]
    pub const fn scan_cap(&self) -> u32 {
        self.scanner.cap
    }

    pub fn keep_mark(&mut self, mark: Mark) {
        self.marks.retain(|held| held.id != mark.id);
        self.marks.push(mark);
        if self.marks.len() > MARKS_HELD {
            self.marks.remove(0);
        }
    }

    #[must_use]
    pub fn mark(&self, id: &str) -> Option<&Mark> {
        self.marks.iter().find(|held| held.id == id)
    }

    #[must_use]
    pub fn marks(&self) -> &[Mark] {
        &self.marks
    }
}
