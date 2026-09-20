use crate::netlist::{KIND_BITS, Netlist, SOURCE_BITS};

use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, Lanes, U32, Vector};

pub const HELD: u32 = 0;

pub const TRIAL: u32 = 1;

pub const VALUES: u32 = 2;

pub const WANTED: u32 = 3;

pub const SCORE: u32 = 4;

pub const BEST: u32 = 5;

pub const SEED: u32 = 6;

pub const BINDINGS: u32 = 7;

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

pub fn turned_on(subgroup: u32, shape: Netlist) -> Result<Vec<u32>, Box<dyn std::error::Error>> {
    Ok(match subgroup {
        4 => turning::<4>(subgroup, shape)?,
        8 => turning::<8>(subgroup, shape)?,
        16 => turning::<16>(subgroup, shape)?,
        32 => turning::<32>(subgroup, shape)?,
        64 => turning::<64>(subgroup, shape)?,
        other => {
            return Err(Box::new(LaneError::NoMapping {
                lanes: other,
                width: subgroup,
            }));
        }
    })
}

pub fn chosen(
    subgroup: u32,
    shape: Netlist,
    spread: u32,
    crowd: u32,
) -> Result<Vec<u32>, Box<dyn std::error::Error>> {
    Ok(match subgroup {
        4 => choosing::<4>(subgroup, shape, spread, crowd)?,
        8 => choosing::<8>(subgroup, shape, spread, crowd)?,
        16 => choosing::<16>(subgroup, shape, spread, crowd)?,
        32 => choosing::<32>(subgroup, shape, spread, crowd)?,
        64 => choosing::<64>(subgroup, shape, spread, crowd)?,
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
        let packed = kernel.load_at(TRIAL, place)?;

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

fn rolled<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    held: Vector<U32, LANES>,
) -> Result<Vector<U32, LANES>, LaneError> {
    let up = lanes.splat_bits::<U32, LANES>(13)?;
    let down = lanes.splat_bits::<U32, LANES>(17)?;
    let over = lanes.splat_bits::<U32, LANES>(5)?;
    let shifted = lanes.shift_left(held, up)?;
    let held = lanes.xor(held, shifted)?;
    let shifted = lanes.shift_right_logical(held, down)?;
    let held = lanes.xor(held, shifted)?;
    let shifted = lanes.shift_left(held, over)?;
    lanes.xor(held, shifted)
}

fn modulo<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    held: Vector<U32, LANES>,
    by: Vector<U32, LANES>,
) -> Result<Vector<U32, LANES>, LaneError> {
    let whole = lanes.div(held, by)?;
    let back = lanes.mul(whole, by)?;
    lanes.sub(held, back)
}

pub fn turning<const LANES: u32>(
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
    let count = kernel.module().constant_u32(shape.gates)?;
    let ours = kernel.module().i_mul(uint, who, count)?;

    for at in 0..shape.gates {
        let step = kernel.module().constant_u32(at)?;
        let place = kernel.module().i_add(uint, ours, step)?;
        let word = kernel.load_at(HELD, place)?;
        kernel.store_at(TRIAL, place, word)?;
    }

    let seed = kernel.load_at(SEED, who)?;
    let (next, place, at, roll) = {
        let mut lanes = kernel.lanes()?;
        let held = lanes.splat_id::<U32, LANES>(seed)?;
        let first = rolled::<LANES>(&mut lanes, held)?;
        let gates = lanes.splat_bits::<U32, LANES>(shape.gates)?;
        let at = modulo::<LANES>(&mut lanes, first, gates)?;
        let roll = rolled::<LANES>(&mut lanes, first)?;
        let row = lanes.splat_id::<U32, LANES>(ours)?;
        let place = lanes.add(row, at)?;
        (roll.id(), place.id(), at.id(), roll.id())
    };
    kernel.store_at(SEED, who, next)?;
    let packed = kernel.load_at(TRIAL, place)?;

    let turned = {
        let mut lanes = kernel.lanes()?;
        let word = lanes.splat_id::<U32, LANES>(packed)?;
        let roll = lanes.splat_id::<U32, LANES>(roll)?;
        let at = lanes.splat_id::<U32, LANES>(at)?;

        let two = lanes.splat_bits::<U32, LANES>(2)?;
        let three = lanes.splat_bits::<U32, LANES>(3)?;
        let none = lanes.splat_bits::<U32, LANES>(0)?;
        let one = lanes.splat_bits::<U32, LANES>(1)?;
        let inputs = lanes.splat_bits::<U32, LANES>(shape.inputs)?;
        let kinds = lanes.splat_bits::<U32, LANES>(shape.kinds)?;
        let by = lanes.splat_bits::<U32, LANES>(KIND_BITS)?;
        let far = lanes.splat_bits::<U32, LANES>(KIND_BITS + SOURCE_BITS)?;
        let mask = lanes.splat_bits::<U32, LANES>((1 << SOURCE_BITS) - 1)?;
        let kind_mask = lanes.splat_bits::<U32, LANES>((1 << KIND_BITS) - 1)?;

        let below = lanes.add(inputs, at)?;
        let rest = lanes.shift_right_logical(roll, two)?;
        let which = lanes.and(roll, three)?;
        let kind = modulo::<LANES>(&mut lanes, rest, kinds)?;
        let side = modulo::<LANES>(&mut lanes, rest, below)?;

        let was_kind = lanes.and(word, kind_mask)?;
        let shifted = lanes.shift_right_logical(word, by)?;
        let was_left = lanes.and(shifted, mask)?;
        let shifted = lanes.shift_right_logical(word, far)?;
        let was_right = lanes.and(shifted, mask)?;

        let turns_kind = lanes.equal(which, none)?;
        let turns_left = lanes.equal(which, one)?;
        let turns_right = lanes.greater_equal(which, two)?;
        let kind = lanes.select(turns_kind, kind, was_kind)?;
        let left = lanes.select(turns_left, side, was_left)?;
        let right = lanes.select(turns_right, side, was_right)?;

        let left = lanes.shift_left(left, by)?;
        let right = lanes.shift_left(right, far)?;
        let held = lanes.or(kind, left)?;
        lanes.or(held, right)?.id()
    };
    kernel.store_at(TRIAL, place, turned)?;
    Ok(kernel.finish()?)
}

pub fn choosing<const LANES: u32>(
    subgroup: u32,
    shape: Netlist,
    spread: u32,
    crowd: u32,
) -> Result<Vec<u32>, Box<dyn std::error::Error>> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.index_type();
    let group = kernel.workgroup_index();
    let lane = kernel.local_index();
    let width = kernel.module().constant_u32(subgroup)?;
    let base = kernel.module().i_mul(uint, group, width)?;
    let who = kernel.module().i_add(uint, base, lane)?;
    let count = kernel.module().constant_u32(shape.gates)?;
    let ours = kernel.module().i_mul(uint, who, count)?;

    let mut beside = Vec::new();
    for by in [
        1,
        crowd.saturating_sub(1),
        spread,
        crowd.saturating_sub(spread),
    ] {
        if spread == 0 {
            break;
        }
        let near = {
            let mut lanes = kernel.lanes()?;
            let who = lanes.splat_id::<U32, LANES>(who)?;
            let by = lanes.splat_bits::<U32, LANES>(by)?;
            let all = lanes.splat_bits::<U32, LANES>(crowd.max(1))?;
            let past = lanes.add(who, by)?;
            modulo::<LANES>(&mut lanes, past, all)?.id()
        };
        let mark = kernel.load_at(SCORE, near)?;
        beside.push((near, mark));
    }

    let score = kernel.load_at(SCORE, who)?;
    let best = kernel.load_at(BEST, who)?;
    let (other, theirs) = {
        let mut lanes = kernel.lanes()?;
        let none = lanes.splat_bits::<U32, LANES>(0)?;
        let mut most = none;
        let mut from = lanes.splat_id::<U32, LANES>(who)?;
        for (near, mark) in &beside {
            let near = lanes.splat_id::<U32, LANES>(*near)?;
            let mark = lanes.splat_id::<U32, LANES>(*mark)?;
            let over = lanes.greater_than(mark, most)?;
            most = lanes.select(over, mark, most)?;
            from = lanes.select(over, near, from)?;
        }
        (most.id(), from.id())
    };
    let theirs = kernel.module().i_mul(uint, theirs, count)?;
    let (keeping, taking, kept) = {
        let mut lanes = kernel.lanes()?;
        let score = lanes.splat_id::<U32, LANES>(score)?;
        let best = lanes.splat_id::<U32, LANES>(best)?;
        let one = lanes.splat_bits::<U32, LANES>(1)?;
        let none = lanes.splat_bits::<U32, LANES>(0)?;
        let other = lanes.splat_id::<U32, LANES>(other)?;
        let over = lanes.greater_than(other, score)?;
        let near = lanes.select(over, other, score)?;
        let keep = lanes.greater_equal(near, best)?;
        let from_other = lanes.select(over, one, none)?;
        let keeping = lanes.select(keep, one, none)?;
        let take = lanes.and(from_other, keeping)?;
        let kept = lanes.select(keep, near, best)?;
        (keeping.id(), take.id(), kept.id())
    };
    kernel.store_at(BEST, who, kept)?;

    for at in 0..shape.gates {
        let step = kernel.module().constant_u32(at)?;
        let place = kernel.module().i_add(uint, ours, step)?;
        let yonder = kernel.module().i_add(uint, theirs, step)?;
        let mine = kernel.load_at(TRIAL, place)?;
        let other = kernel.load_at(TRIAL, yonder)?;
        let held = kernel.load_at(HELD, place)?;
        let taken = {
            let mut lanes = kernel.lanes()?;
            let one = lanes.splat_bits::<U32, LANES>(1)?;
            let keeping = lanes.splat_id::<U32, LANES>(keeping)?;
            let taking = lanes.splat_id::<U32, LANES>(taking)?;
            let keep = lanes.equal(keeping, one)?;
            let take = lanes.equal(taking, one)?;
            let mine = lanes.splat_id::<U32, LANES>(mine)?;
            let other = lanes.splat_id::<U32, LANES>(other)?;
            let held = lanes.splat_id::<U32, LANES>(held)?;
            let trial = lanes.select(take, other, mine)?;
            lanes.select(keep, trial, held)?.id()
        };
        kernel.store_at(HELD, place, taken)?;
    }
    Ok(kernel.finish()?)
}
