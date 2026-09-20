use crate::layout::{PARAMS, STATE};

use crate::live::corners;

use crate::sand::Grid;

use simdr::kernel::{Kernel, Shape};

use simdr::lanes::{LaneError, Lanes, U32, Vector};

pub const SLOPE: u32 = 2;

pub const TOLD_TURNING: u32 = 3;

pub const TOLD_SWING: u32 = 4;

pub const FIELD: u32 = 5;

pub const BINDINGS: u32 = 6;

pub const POINT: u32 = 16;

pub const PRODUCT: u32 = 20;

pub const SHARE: u32 = 4;

pub const SPREAD: [u32; 3] = [2, 3, 4];

pub const SPARE: u32 = 8;

pub const ROOM: u32 = 1 << 15;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Product,
    Sum,
    Turn,
    Bound,
    Smooth,
    Living,
    Twinned,
}

pub const KNOB: u32 = 12;

fn shifted_by<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    one: Vector<U32, LANES>,
    by: Vector<U32, LANES>,
    places: u32,
) -> Result<Vector<U32, LANES>, LaneError> {
    let amount = lanes.splat_bits::<U32, LANES>(places)?;
    let half = lanes.splat_bits::<U32, LANES>(1 << (places - 1))?;
    let held = lanes.mul(one, by)?;
    let held = lanes.add(held, half)?;
    lanes.shift_right_arithmetic(held, amount)
}

fn leaning_on<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    knob: Vector<U32, LANES>,
    on: Vector<U32, LANES>,
) -> Result<Pair<LANES>, LaneError> {
    let by = lanes.splat_bits::<U32, LANES>(KNOB)?;
    let held = lanes.mul(knob, on)?;
    let turning = lanes.shift_right_arithmetic(held, by)?;
    let swing = lanes.add(turning, turning)?;
    Ok((turning, swing))
}

pub type Pair<const LANES: u32> = (Vector<U32, LANES>, Vector<U32, LANES>);

pub fn pinned<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    gate: Vector<U32, LANES>,
    angle: Pair<LANES>,
) -> Result<Pair<LANES>, LaneError> {
    let one = lanes.splat_bits::<U32, LANES>(1)?;
    let none = lanes.splat_bits::<U32, LANES>(0)?;
    let on = lanes.equal(gate, one)?;
    let (turning, swing) = angle;
    let back_turning = lanes.sub(none, turning)?;
    let back_swing = lanes.sub(none, swing)?;
    Ok((
        lanes.select(on, turning, back_turning)?,
        lanes.select(on, swing, back_swing)?,
    ))
}

pub fn gated<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    first: Vector<U32, LANES>,
    second: Vector<U32, LANES>,
) -> Result<Vector<U32, LANES>, LaneError> {
    let one = lanes.splat_bits::<U32, LANES>(1)?;
    let sum = lanes.add(first, second)?;
    lanes.and(sum, one)
}

pub fn scaled<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    one: Vector<U32, LANES>,
    by: Vector<U32, LANES>,
) -> Result<Vector<U32, LANES>, LaneError> {
    let point = lanes.splat_bits::<U32, LANES>(POINT)?;
    let held = lanes.mul(one, by)?;
    lanes.shift_right_arithmetic(held, point)
}

pub fn finer<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    one: Vector<U32, LANES>,
    by: Vector<U32, LANES>,
) -> Result<Vector<U32, LANES>, LaneError> {
    let point = lanes.splat_bits::<U32, LANES>(POINT - SPARE)?;
    let held = lanes.mul(one, by)?;
    lanes.shift_right_arithmetic(held, point)
}

pub fn spun<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    held: Pair<LANES>,
    angle: Pair<LANES>,
    back: bool,
) -> Result<Pair<LANES>, LaneError> {
    let (mut first, mut second) = held;
    let (turning, swing) = angle;
    if back {
        let step = scaled::<LANES>(lanes, first, turning)?;
        second = lanes.sub(second, step)?;
        let step = scaled::<LANES>(lanes, second, swing)?;
        first = lanes.add(first, step)?;
        let step = scaled::<LANES>(lanes, first, turning)?;
        second = lanes.sub(second, step)?;
    } else {
        let step = scaled::<LANES>(lanes, first, turning)?;
        second = lanes.add(second, step)?;
        let step = scaled::<LANES>(lanes, second, swing)?;
        first = lanes.sub(first, step)?;
        let step = scaled::<LANES>(lanes, first, turning)?;
        second = lanes.add(second, step)?;
    }
    Ok((first, second))
}

pub type Unwound<const LANES: u32> = (Pair<LANES>, Pair<LANES>, Pair<LANES>);

pub fn unspun<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    held: Pair<LANES>,
    weight: Pair<LANES>,
    angle: Pair<LANES>,
) -> Result<Unwound<LANES>, LaneError> {
    let (mut first, mut second) = held;
    let (mut gfirst, mut gsecond) = weight;
    let (turning, swing) = angle;
    let none = lanes.splat_bits::<U32, LANES>(0)?;

    let mut told_turning = finer::<LANES>(lanes, gsecond, first)?;
    let step = scaled::<LANES>(lanes, first, turning)?;
    second = lanes.sub(second, step)?;
    let step = scaled::<LANES>(lanes, gsecond, turning)?;
    gfirst = lanes.add(gfirst, step)?;

    let against = finer::<LANES>(lanes, gfirst, second)?;
    let told_swing = lanes.sub(none, against)?;
    let step = scaled::<LANES>(lanes, second, swing)?;
    first = lanes.add(first, step)?;
    let step = scaled::<LANES>(lanes, gfirst, swing)?;
    gsecond = lanes.sub(gsecond, step)?;

    let more = finer::<LANES>(lanes, gsecond, first)?;
    told_turning = lanes.add(told_turning, more)?;
    let step = scaled::<LANES>(lanes, first, turning)?;
    second = lanes.sub(second, step)?;
    let step = scaled::<LANES>(lanes, gsecond, turning)?;
    gfirst = lanes.add(gfirst, step)?;

    Ok((
        (first, second),
        (gfirst, gsecond),
        (told_turning, told_swing),
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pass {
    Ahead,
    Back,
}

pub fn mixed<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    kind: Kind,
    first: Vector<U32, LANES>,
    second: Vector<U32, LANES>,
    third: Vector<U32, LANES>,
) -> Result<Vector<U32, LANES>, LaneError> {
    if kind == Kind::Sum {
        let mut held = lanes.splat_bits::<U32, LANES>(0)?;
        for (one, by) in [first, second, third].into_iter().zip(SPREAD) {
            let shift = lanes.splat_bits::<U32, LANES>(by)?;
            let part = lanes.shift_right_arithmetic(one, shift)?;
            held = lanes.add(held, part)?;
        }
        return Ok(held);
    }
    let wide = lanes.splat_bits::<U32, LANES>(PRODUCT)?;
    let narrow = lanes.splat_bits::<U32, LANES>(SHARE)?;
    let held = lanes.mul(first, second)?;
    let held = lanes.shift_right_logical(held, wide)?;
    let part = lanes.shift_right_logical(third, narrow)?;
    lanes.add(held, part)
}

pub fn onward<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    kind: Kind,
    held: [Vector<U32, LANES>; 4],
    angle: Pair<LANES>,
    twin: Vector<U32, LANES>,
) -> Result<[Vector<U32, LANES>; 4], LaneError> {
    let [mut a, mut b, mut c, mut d] = held;
    if kind == Kind::Turn {
        return Ok([d, c, b, a]);
    }
    if kind == Kind::Twinned {
        let (knob, _) = angle;
        let turned = leaning_on::<LANES>(lanes, knob, c)?;
        let (first, second) = spun::<LANES>(lanes, (a, b), turned, false)?;
        let turned = leaning_on::<LANES>(lanes, twin, first)?;
        let (third, fourth) = spun::<LANES>(lanes, (c, d), turned, false)?;
        return Ok([fourth, third, second, first]);
    }
    if kind == Kind::Living {
        let (knob, _) = angle;
        let turned = leaning_on::<LANES>(lanes, knob, c)?;
        let (first, second) = spun::<LANES>(lanes, (a, b), turned, false)?;
        let turned = leaning_on::<LANES>(lanes, knob, first)?;
        let (third, fourth) = spun::<LANES>(lanes, (c, d), turned, false)?;
        return Ok([fourth, third, second, first]);
    }
    if kind == Kind::Bound || kind == Kind::Smooth {
        let steady = kind == Kind::Smooth;
        let gate = if steady {
            lanes.splat_bits::<U32, LANES>(1)?
        } else {
            gated::<LANES>(lanes, c, d)?
        };
        let turned = pinned::<LANES>(lanes, gate, angle)?;
        let (first, second) = spun::<LANES>(lanes, (a, b), turned, false)?;
        let gate = if steady {
            lanes.splat_bits::<U32, LANES>(1)?
        } else {
            gated::<LANES>(lanes, first, second)?
        };
        let turned = pinned::<LANES>(lanes, gate, angle)?;
        let (third, fourth) = spun::<LANES>(lanes, (c, d), turned, false)?;
        return Ok([fourth, third, second, first]);
    }
    let step = mixed::<LANES>(lanes, kind, b, c, d)?;
    a = lanes.add(a, step)?;
    let step = mixed::<LANES>(lanes, kind, c, d, a)?;
    b = lanes.add(b, step)?;
    let step = mixed::<LANES>(lanes, kind, d, a, b)?;
    c = lanes.add(c, step)?;
    let step = mixed::<LANES>(lanes, kind, a, b, c)?;
    d = lanes.add(d, step)?;
    Ok([a, b, c, d])
}

pub type Both<const LANES: u32> = (
    [Vector<U32, LANES>; 4],
    [Vector<U32, LANES>; 4],
    Pair<LANES>,
);

pub fn backward<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    kind: Kind,
    held: [Vector<U32, LANES>; 4],
    weight: [Vector<U32, LANES>; 4],
    angle: Pair<LANES>,
    twin: Vector<U32, LANES>,
) -> Result<Both<LANES>, LaneError> {
    let [mut a, mut b, mut c, mut d] = held;
    let [mut ga, mut gb, mut gc, mut gd] = weight;
    if kind == Kind::Twinned {
        let (knob, _) = angle;
        let (first, second, third, fourth) = (d, c, b, a);
        let (gfirst, gsecond, gthird, gfourth) = (gd, gc, gb, ga);

        let later = leaning_on::<LANES>(lanes, twin, first)?;
        let ((third, fourth), (gthird, gfourth), late) =
            unspun::<LANES>(lanes, (third, fourth), (gthird, gfourth), later)?;
        let late_turn = {
            let twice = lanes.add(late.1, late.1)?;
            lanes.add(late.0, twice)?
        };
        let told_late = shifted_by::<LANES>(lanes, late_turn, first, KNOB)?;
        let feed = shifted_by::<LANES>(lanes, late_turn, twin, KNOB + SPARE)?;
        let gfirst = lanes.add(gfirst, feed)?;

        let early = leaning_on::<LANES>(lanes, knob, third)?;
        let ((first, second), (gfirst, gsecond), early_told) =
            unspun::<LANES>(lanes, (first, second), (gfirst, gsecond), early)?;
        let early_turn = {
            let twice = lanes.add(early_told.1, early_told.1)?;
            lanes.add(early_told.0, twice)?
        };
        let told_early = shifted_by::<LANES>(lanes, early_turn, third, KNOB)?;
        let feed = shifted_by::<LANES>(lanes, early_turn, knob, KNOB + SPARE)?;
        let gthird = lanes.add(gthird, feed)?;

        return Ok((
            [first, second, third, fourth],
            [gfirst, gsecond, gthird, gfourth],
            (told_early, told_late),
        ));
    }
    if kind == Kind::Turn {
        let none = lanes.splat_bits::<U32, LANES>(0)?;
        return Ok(([d, c, b, a], [gd, gc, gb, ga], (none, none)));
    }
    if kind == Kind::Living {
        let (knob, _) = angle;
        let (first, second, third, fourth) = (d, c, b, a);
        let (gfirst, gsecond, gthird, gfourth) = (gd, gc, gb, ga);

        let later = leaning_on::<LANES>(lanes, knob, first)?;
        let ((third, fourth), (gthird, gfourth), late) =
            unspun::<LANES>(lanes, (third, fourth), (gthird, gfourth), later)?;
        let late_turn = {
            let twice = lanes.add(late.1, late.1)?;
            lanes.add(late.0, twice)?
        };
        let told_late = shifted_by::<LANES>(lanes, late_turn, first, KNOB)?;
        let feed = shifted_by::<LANES>(lanes, late_turn, knob, KNOB + SPARE)?;
        let gfirst = lanes.add(gfirst, feed)?;

        let early = leaning_on::<LANES>(lanes, knob, third)?;
        let ((first, second), (gfirst, gsecond), early_told) =
            unspun::<LANES>(lanes, (first, second), (gfirst, gsecond), early)?;
        let early_turn = {
            let twice = lanes.add(early_told.1, early_told.1)?;
            lanes.add(early_told.0, twice)?
        };
        let told_early = shifted_by::<LANES>(lanes, early_turn, third, KNOB)?;
        let feed = shifted_by::<LANES>(lanes, early_turn, knob, KNOB + SPARE)?;
        let gthird = lanes.add(gthird, feed)?;

        let told_knob = lanes.add(told_late, told_early)?;
        let none = lanes.splat_bits::<U32, LANES>(0)?;
        return Ok((
            [first, second, third, fourth],
            [gfirst, gsecond, gthird, gfourth],
            (told_knob, none),
        ));
    }
    if kind == Kind::Bound || kind == Kind::Smooth {
        let steady = kind == Kind::Smooth;
        let (first, second, third, fourth) = (d, c, b, a);
        let (gfirst, gsecond, gthird, gfourth) = (gd, gc, gb, ga);
        let gate = if steady {
            lanes.splat_bits::<U32, LANES>(1)?
        } else {
            gated::<LANES>(lanes, first, second)?
        };
        let later = pinned::<LANES>(lanes, gate, angle)?;
        let ((third, fourth), (gthird, gfourth), late_told) =
            unspun::<LANES>(lanes, (third, fourth), (gthird, gfourth), later)?;
        let gate = if steady {
            lanes.splat_bits::<U32, LANES>(1)?
        } else {
            gated::<LANES>(lanes, third, fourth)?
        };
        let early = pinned::<LANES>(lanes, gate, angle)?;
        let ((first, second), (gfirst, gsecond), early_told) =
            unspun::<LANES>(lanes, (first, second), (gfirst, gsecond), early)?;
        let told_turning = lanes.add(late_told.0, early_told.0)?;
        let told_swing = lanes.add(late_told.1, early_told.1)?;
        return Ok((
            [first, second, third, fourth],
            [gfirst, gsecond, gthird, gfourth],
            (told_turning, told_swing),
        ));
    }
    let wide = lanes.splat_bits::<U32, LANES>(PRODUCT)?;
    let narrow = lanes.splat_bits::<U32, LANES>(SHARE)?;

    let against = |lanes: &mut Lanes<'_>,
                   weight: Vector<U32, LANES>,
                   first: Vector<U32, LANES>,
                   second: Vector<U32, LANES>| {
        if kind == Kind::Sum {
            let [near, middling, far] = SPREAD;
            let by = lanes.splat_bits::<U32, LANES>(near)?;
            let nearer = lanes.shift_right_arithmetic(weight, by)?;
            let by = lanes.splat_bits::<U32, LANES>(middling)?;
            let midway = lanes.shift_right_arithmetic(weight, by)?;
            let by = lanes.splat_bits::<U32, LANES>(far)?;
            let further = lanes.shift_right_arithmetic(weight, by)?;
            return Ok((nearer, midway, further));
        }
        let held = lanes.mul(weight, second)?;
        let to_first = lanes.shift_right_arithmetic(held, wide)?;
        let held = lanes.mul(weight, first)?;
        let to_second = lanes.shift_right_arithmetic(held, wide)?;
        let to_third = lanes.shift_right_arithmetic(weight, narrow)?;
        Ok::<_, LaneError>((to_first, to_second, to_third))
    };

    let step = mixed::<LANES>(lanes, kind, a, b, c)?;
    d = lanes.sub(d, step)?;
    let (to_a, to_b, to_c) = against(lanes, gd, a, b)?;
    ga = lanes.add(ga, to_a)?;
    gb = lanes.add(gb, to_b)?;
    gc = lanes.add(gc, to_c)?;

    let step = mixed::<LANES>(lanes, kind, d, a, b)?;
    c = lanes.sub(c, step)?;
    let (to_d, to_a, to_b) = against(lanes, gc, d, a)?;
    gd = lanes.add(gd, to_d)?;
    ga = lanes.add(ga, to_a)?;
    gb = lanes.add(gb, to_b)?;

    let step = mixed::<LANES>(lanes, kind, c, d, a)?;
    b = lanes.sub(b, step)?;
    let (to_c, to_d, to_a) = against(lanes, gb, c, d)?;
    gc = lanes.add(gc, to_c)?;
    gd = lanes.add(gd, to_d)?;
    ga = lanes.add(ga, to_a)?;

    let step = mixed::<LANES>(lanes, kind, b, c, d)?;
    a = lanes.sub(a, step)?;
    let (to_b, to_c, to_d) = against(lanes, ga, b, c)?;
    gb = lanes.add(gb, to_b)?;
    gc = lanes.add(gc, to_c)?;
    gd = lanes.add(gd, to_d)?;

    let none = lanes.splat_bits::<U32, LANES>(0)?;
    Ok(([a, b, c, d], [ga, gb, gc, gd], (none, none)))
}

pub fn built(subgroup: u32, grid: Grid, pass: Pass, kind: Kind) -> Result<Vec<u32>, LaneError> {
    match subgroup {
        4 => building::<4>(subgroup, grid, pass, kind),
        8 => building::<8>(subgroup, grid, pass, kind),
        16 => building::<16>(subgroup, grid, pass, kind),
        32 => building::<32>(subgroup, grid, pass, kind),
        64 => building::<64>(subgroup, grid, pass, kind),
        other => Err(LaneError::NoMapping {
            lanes: other,
            width: subgroup,
        }),
    }
}

pub fn building<const LANES: u32>(
    subgroup: u32,
    grid: Grid,
    pass: Pass,
    kind: Kind,
) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.index_type();
    let group = kernel.workgroup_index();
    let lane = kernel.local_index();
    let width = kernel.module().constant_u32(subgroup)?;
    let base = kernel.module().i_mul(uint, group, width)?;
    let quad = kernel.module().i_add(uint, base, lane)?;

    let head = kernel.module().constant_u32(0)?;
    let frame = kernel.load_at(PARAMS, head)?;
    let at_swing = kernel.module().constant_u32(5)?;
    let swing = kernel.load_at(PARAMS, at_swing)?;
    let places = corners::<LANES>(&mut kernel, grid, quad, frame)?.places;
    let turning = kernel.load_at(FIELD, places[0].id())?;
    let twinned = kind == Kind::Twinned;
    let twin = if twinned {
        kernel.load_at(FIELD, places[1].id())?
    } else {
        turning
    };

    let state = [
        kernel.load_at(STATE, places[0].id())?,
        kernel.load_at(STATE, places[1].id())?,
        kernel.load_at(STATE, places[2].id())?,
        kernel.load_at(STATE, places[3].id())?,
    ];
    let carried = match pass {
        Pass::Ahead => None,
        Pass::Back => Some([
            kernel.load_at(SLOPE, places[0].id())?,
            kernel.load_at(SLOPE, places[1].id())?,
            kernel.load_at(SLOPE, places[2].id())?,
            kernel.load_at(SLOPE, places[3].id())?,
        ]),
    };

    let (state_next, weight_next) = {
        let mut lanes = kernel.lanes()?;
        let held = [
            lanes.splat_id::<U32, LANES>(state[0])?,
            lanes.splat_id::<U32, LANES>(state[1])?,
            lanes.splat_id::<U32, LANES>(state[2])?,
            lanes.splat_id::<U32, LANES>(state[3])?,
        ];
        match carried {
            None => {
                let angle = (
                    lanes.splat_id::<U32, LANES>(turning)?,
                    lanes.splat_id::<U32, LANES>(swing)?,
                );
                let twin = lanes.splat_id::<U32, LANES>(twin)?;
                (onward::<LANES>(&mut lanes, kind, held, angle, twin)?, None)
            }
            Some(weight) => {
                let weight = [
                    lanes.splat_id::<U32, LANES>(weight[0])?,
                    lanes.splat_id::<U32, LANES>(weight[1])?,
                    lanes.splat_id::<U32, LANES>(weight[2])?,
                    lanes.splat_id::<U32, LANES>(weight[3])?,
                ];
                let angle = (
                    lanes.splat_id::<U32, LANES>(turning)?,
                    lanes.splat_id::<U32, LANES>(swing)?,
                );
                let twin = lanes.splat_id::<U32, LANES>(twin)?;
                let (state_back, weight_back, told) =
                    backward::<LANES>(&mut lanes, kind, held, weight, angle, twin)?;
                (state_back, Some((weight_back, told)))
            }
        }
    };

    for (place, value) in places.iter().zip(&state_next) {
        kernel.store_at(STATE, place.id(), value.id())?;
    }
    if let Some((next, told)) = weight_next {
        for (place, value) in places.iter().zip(&next) {
            kernel.store_at(SLOPE, place.id(), value.id())?;
        }
        let piling = if twinned {
            [
                (TOLD_TURNING, places[0].id(), told.0),
                (TOLD_TURNING, places[1].id(), told.1),
            ]
        } else {
            [
                (TOLD_TURNING, places[0].id(), told.0),
                (TOLD_SWING, places[0].id(), told.1),
            ]
        };
        for (binding, here, part) in piling {
            let held = kernel.load_at(binding, here)?;
            let piled = {
                let mut lanes = kernel.lanes()?;
                let before = lanes.splat_id::<U32, LANES>(held)?;
                lanes.add(before, part)?
            };
            kernel.store_at(binding, here, piled.id())?;
        }
    }
    kernel.finish()
}
