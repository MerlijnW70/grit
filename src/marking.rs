use crate::netlist::{KIND_BITS, Netlist, SOURCE_BITS};

use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, Lanes, U32, Vector};

pub const GATES: u32 = 0;

pub const VALUES: u32 = 1;

pub const WANTED: u32 = 2;

pub const SCORE: u32 = 3;

pub const BINDINGS: u32 = 4;

pub const ROWS: u32 = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TooDeep {
    Inputs { inputs: u32, limit: u32 },
}

impl std::fmt::Display for TooDeep {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Inputs { inputs, limit } => write!(out, "{inputs} {limit}"),
        }
    }
}

impl std::error::Error for TooDeep {}

#[must_use]
pub fn column(input: u32) -> u32 {
    (0..ROWS)
        .filter(|row| (row >> input) & 1 == 1)
        .fold(0_u32, |held, row| held | (1 << row))
}

#[must_use]
pub fn rows_of(shape: Netlist) -> u32 {
    if shape.inputs >= 5 {
        u32::MAX
    } else {
        (1_u32 << shape.words()) - 1
    }
}

fn counted<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    held: Vector<U32, LANES>,
) -> Result<Vector<U32, LANES>, LaneError> {
    let one = lanes.splat_bits::<U32, LANES>(1)?;
    let two = lanes.splat_bits::<U32, LANES>(2)?;
    let four = lanes.splat_bits::<U32, LANES>(4)?;
    let byte = lanes.splat_bits::<U32, LANES>(24)?;
    let fives = lanes.splat_bits::<U32, LANES>(0x5555_5555)?;
    let threes = lanes.splat_bits::<U32, LANES>(0x3333_3333)?;
    let effs = lanes.splat_bits::<U32, LANES>(0x0f0f_0f0f)?;
    let ones = lanes.splat_bits::<U32, LANES>(0x0101_0101)?;

    let shifted = lanes.shift_right_logical(held, one)?;
    let paired = lanes.and(shifted, fives)?;
    let held = lanes.sub(held, paired)?;

    let low = lanes.and(held, threes)?;
    let shifted = lanes.shift_right_logical(held, two)?;
    let high = lanes.and(shifted, threes)?;
    let held = lanes.add(low, high)?;

    let shifted = lanes.shift_right_logical(held, four)?;
    let held = lanes.add(held, shifted)?;
    let held = lanes.and(held, effs)?;

    let held = lanes.mul(held, ones)?;
    lanes.shift_right_logical(held, byte)
}

pub fn marked(subgroup: u32, shape: Netlist) -> Result<Vec<u32>, Box<dyn std::error::Error>> {
    if shape.inputs > 5 {
        return Err(Box::new(TooDeep::Inputs {
            inputs: shape.inputs,
            limit: 5,
        }));
    }
    Ok(match subgroup {
        4 => marking::<4>(subgroup, shape)?,
        8 => marking::<8>(subgroup, shape)?,
        16 => marking::<16>(subgroup, shape)?,
        32 => marking::<32>(subgroup, shape)?,
        64 => marking::<64>(subgroup, shape)?,
        other => {
            return Err(Box::new(LaneError::NoMapping {
                lanes: other,
                width: subgroup,
            }));
        }
    })
}

pub fn marking<const LANES: u32>(
    subgroup: u32,
    shape: Netlist,
) -> Result<Vec<u32>, Box<dyn std::error::Error>> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.index_type();
    let group = kernel.workgroup_index();
    let lane = kernel.local_index();
    let width = kernel.module().constant_u32(subgroup)?;
    let base = kernel.module().i_mul(uint, group, width)?;
    let who = kernel.module().i_add(uint, base, lane)?;

    let sources = kernel.module().constant_u32(shape.sources())?;
    let mine = kernel.module().i_mul(uint, who, sources)?;
    let count = kernel.module().constant_u32(shape.gates)?;
    let ours = kernel.module().i_mul(uint, who, count)?;

    for input in 0..shape.inputs {
        let step = kernel.module().constant_u32(input)?;
        let place = kernel.module().i_add(uint, mine, step)?;
        let value = kernel.module().constant_u32(column(input))?;
        kernel.store_at(VALUES, place, value)?;
    }

    for at in 0..shape.gates {
        let step = kernel.module().constant_u32(at)?;
        let place = kernel.module().i_add(uint, ours, step)?;
        let packed = kernel.load_at(GATES, place)?;

        let (left, right, kinds) = {
            let mut lanes = kernel.lanes()?;
            let word = lanes.splat_id::<U32, LANES>(packed)?;
            let row = lanes.splat_id::<U32, LANES>(mine)?;
            let by = lanes.splat_bits::<U32, LANES>(KIND_BITS)?;
            let far = lanes.splat_bits::<U32, LANES>(KIND_BITS + SOURCE_BITS)?;
            let mask = lanes.splat_bits::<U32, LANES>((1 << SOURCE_BITS) - 1)?;

            let shifted = lanes.shift_right_logical(word, by)?;
            let left = lanes.and(shifted, mask)?;
            let left = lanes.add(row, left)?;

            let shifted = lanes.shift_right_logical(word, far)?;
            let right = lanes.and(shifted, mask)?;
            let right = lanes.add(row, right)?;

            let mut kinds = Vec::with_capacity(4);
            for bit in 0..4_u32 {
                let by = lanes.splat_bits::<U32, LANES>(bit)?;
                let one = lanes.splat_bits::<U32, LANES>(1)?;
                let none = lanes.splat_bits::<U32, LANES>(0)?;
                let all = lanes.splat_bits::<U32, LANES>(u32::MAX)?;
                let shifted = lanes.shift_right_logical(word, by)?;
                let set = lanes.and(shifted, one)?;
                let live = lanes.equal(set, one)?;
                kinds.push(lanes.select(live, all, none)?.id());
            }
            (left.id(), right.id(), kinds)
        };

        let low = kernel.load_at(VALUES, left)?;
        let high = kernel.load_at(VALUES, right)?;

        let answered = {
            let mut lanes = kernel.lanes()?;
            let low = lanes.splat_id::<U32, LANES>(low)?;
            let high = lanes.splat_id::<U32, LANES>(high)?;
            let not_low = lanes.not(low)?;
            let not_high = lanes.not(high)?;
            let mut held = lanes.splat_bits::<U32, LANES>(0)?;
            for (bit, kind) in kinds.iter().enumerate() {
                let kind = lanes.splat_id::<U32, LANES>(*kind)?;
                let one = if bit & 1 == 0 { not_low } else { low };
                let other = if bit & 2 == 0 { not_high } else { high };
                let both = lanes.and(one, other)?;
                let part = lanes.and(kind, both)?;
                held = lanes.or(held, part)?;
            }
            held.id()
        };

        let step = kernel.module().constant_u32(shape.inputs + at)?;
        let place = kernel.module().i_add(uint, mine, step)?;
        kernel.store_at(VALUES, place, answered)?;
    }

    let mut total = None;
    for output in 0..shape.outputs {
        let named = shape.inputs + shape.gates - shape.outputs + output;
        let step = kernel.module().constant_u32(named)?;
        let place = kernel.module().i_add(uint, mine, step)?;
        let got = kernel.load_at(VALUES, place)?;
        let asked = kernel.module().constant_u32(output)?;
        let wanted = kernel.load_at(WANTED, asked)?;
        let marks = {
            let mut lanes = kernel.lanes()?;
            let got = lanes.splat_id::<U32, LANES>(got)?;
            let wanted = lanes.splat_id::<U32, LANES>(wanted)?;
            let rows = lanes.splat_bits::<U32, LANES>(rows_of(shape))?;
            let apart = lanes.xor(got, wanted)?;
            let same = lanes.not(apart)?;
            let same = lanes.and(same, rows)?;
            let marks = counted::<LANES>(&mut lanes, same)?;
            match total {
                None => marks.id(),
                Some(before) => {
                    let before = lanes.splat_id::<U32, LANES>(before)?;
                    lanes.add(before, marks)?.id()
                }
            }
        };
        total = Some(marks);
    }
    if let Some(marks) = total {
        kernel.store_at(SCORE, who, marks)?;
    }
    Ok(kernel.finish()?)
}
