#![allow(dead_code)]
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use grit::compact::Plan;

use grit::layout::{DENSE, GRAND, PARAMS, STATE};

use grit::live::{self, Waking, corners, wake};

use grit::sand::Grid;

use grit::{Groups, Host};

use simdr::kernel::{Kernel, Shape};

use simdr::lanes::{LaneError, Lanes, U32, Vector};

pub const WIDE: u32 = 64;

pub const DEEP: u32 = 64;

pub const STEPS: u32 = 8;

pub const SLOPE: u32 = 2;

pub const TOLD_TURNING: u32 = 3;

pub const TOLD_SWING: u32 = 4;

pub const FIELD: u32 = 5;

pub const BINDINGS: u32 = 6;

pub const POINT: u32 = 16;

pub const PRODUCT: u32 = 20;

pub const SHARE: u32 = 4;

pub const SPREAD: [u32; 3] = [2, 3, 4];

pub const TURNING: u32 = 8303;

pub const SWING: u32 = 16384;

pub const STEP: u32 = 1 << 9;

pub const GRAIN: u32 = 2;

pub const SPARE: u32 = 8;

pub const ROOM: u32 = 1 << 15;

pub fn device(label: &str) -> Option<Host> {
    match Host::open() {
        Ok(Some(host)) => Some(host),
        Ok(None) => {
            eprintln!("SKIPPED {label}");
            None
        }
        Err(why) => {
            eprintln!("SKIPPED {label} {why}");
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Product,
    Sum,
    Turn,
    Bound,
    Smooth,
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
) -> Result<[Vector<U32, LANES>; 4], LaneError> {
    let [mut a, mut b, mut c, mut d] = held;
    if kind == Kind::Turn {
        return Ok([d, c, b, a]);
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
) -> Result<Both<LANES>, LaneError> {
    let [mut a, mut b, mut c, mut d] = held;
    let [mut ga, mut gb, mut gc, mut gd] = weight;
    if kind == Kind::Turn {
        let none = lanes.splat_bits::<U32, LANES>(0)?;
        return Ok(([d, c, b, a], [gd, gc, gb, ga], (none, none)));
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
            let mut out = Vec::with_capacity(3);
            for by in SPREAD {
                let shift = lanes.splat_bits::<U32, LANES>(by)?;
                out.push(lanes.shift_right_arithmetic(weight, shift)?);
            }
            return Ok((out[0], out[1], out[2]));
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
                (onward::<LANES>(&mut lanes, kind, held, angle)?, None)
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
                let (state_back, weight_back, told) =
                    backward::<LANES>(&mut lanes, kind, held, weight, angle)?;
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
        let here = places[0].id();
        for (binding, part) in [(TOLD_TURNING, told.0), (TOLD_SWING, told.1)] {
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

pub struct Board<'host> {
    pub resident: grit::Resident<'host>,
    pub over: Groups,
    pub grid: Grid,
    pub depth: u32,
    pub angle: (u32, u32),
    pub loud: u32,
    pub planted: bool,
}

impl Board<'_> {
    pub fn tune(&mut self, turning: u32, swing: u32) {
        self.angle = (turning, swing);
        let field = vec![turning; self.grid.cells()];
        self.resident.write(FIELD as usize, &field).unwrap();
        self.planted = true;
    }

    pub fn sow(&mut self, field: &[u32]) {
        self.resident.write(FIELD as usize, field).unwrap();
        self.planted = true;
    }

    pub fn run(&mut self, cells: &[u32]) -> Vec<u32> {
        if !self.planted {
            let field = vec![self.angle.0; self.grid.cells()];
            self.resident.write(FIELD as usize, &field).unwrap();
            self.planted = true;
        }
        self.resident.write(STATE as usize, cells).unwrap();
        self.resident
            .write_at(PARAMS as usize, 4, &[self.angle.0, self.angle.1])
            .unwrap();
        let ahead = [self.over, Groups::Fixed(0)];
        let stages: Vec<&[Groups]> = vec![&ahead];
        for frame in 0..self.depth {
            self.resident
                .write_at(PARAMS as usize, 0, &[frame])
                .unwrap();
            self.resident.staged(&stages).unwrap();
        }
        self.resident
            .read(STATE as usize, self.grid.cells())
            .unwrap()
    }

    pub fn slope(&mut self, cells: &[u32], at: usize) -> Vec<u32> {
        let mut weight = vec![0_u32; self.grid.cells()];
        weight[at] = 1 << POINT;
        self.carry(cells, &weight).0
    }

    pub fn wipe(&mut self) {
        let blank = vec![0_u32; self.grid.cells()];
        self.resident.write(TOLD_TURNING as usize, &blank).unwrap();
        self.resident.write(TOLD_SWING as usize, &blank).unwrap();
    }

    pub fn told_each(&mut self) -> Vec<i32> {
        self.resident
            .read(TOLD_TURNING as usize, self.grid.cells())
            .unwrap()
            .iter()
            .map(|one| *one as i32)
            .collect()
    }

    pub fn told(&mut self) -> (f64, f64) {
        let piled = |held: Vec<u32>| held.iter().map(|one| f64::from(*one as i32)).sum::<f64>();
        let turning = self
            .resident
            .read(TOLD_TURNING as usize, self.grid.cells())
            .unwrap();
        let swing = self
            .resident
            .read(TOLD_SWING as usize, self.grid.cells())
            .unwrap();
        let back = f64::from(1_u32 << SPARE) * f64::from(self.loud);
        (piled(turning) / back, piled(swing) / back)
    }

    pub fn carry(&mut self, cells: &[u32], weight: &[u32]) -> (Vec<u32>, Vec<u32>) {
        let ended = self.run(cells);
        self.wipe();
        let peak = weight
            .iter()
            .map(|one| (*one as i32).unsigned_abs())
            .max()
            .unwrap_or(1)
            .max(1);
        let room = (ROOM / peak).max(1);
        self.loud = 1_u32 << (31 - room.leading_zeros());
        let louder: Vec<u32> = weight
            .iter()
            .map(|one| ((*one as i32).wrapping_mul(self.loud as i32)) as u32)
            .collect();
        let weight = &louder;
        self.resident.write(STATE as usize, &ended).unwrap();
        self.resident
            .write_at(PARAMS as usize, 4, &[self.angle.0, self.angle.1])
            .unwrap();
        self.resident.write(SLOPE as usize, weight).unwrap();

        let back = [Groups::Fixed(0), self.over];
        let stages: Vec<&[Groups]> = vec![&back];
        for frame in (0..self.depth).rev() {
            self.resident
                .write_at(PARAMS as usize, 0, &[frame])
                .unwrap();
            self.resident.staged(&stages).unwrap();
        }
        let home = self
            .resident
            .read(STATE as usize, self.grid.cells())
            .unwrap();
        assert_eq!(home, cells);
        let carried = self
            .resident
            .read(SLOPE as usize, self.grid.cells())
            .unwrap()
            .iter()
            .map(|one| ((*one as i32) / self.loud as i32) as u32)
            .collect();
        (carried, ended)
    }
}

pub fn spread(grid: Grid, seed: u32) -> Vec<u32> {
    let mut state = seed;
    (0..grid.cells())
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state % CEILING
        })
        .collect()
}

pub const ROUNDS: u32 = 40;

pub const CEILING: u32 = 1 << 12;

pub const PACE: u32 = 3;

pub fn descended(
    board: &mut Board<'_>,
    grid: Grid,
    pace: u32,
    rounds: u32,
) -> (f64, f64, f64, f64) {
    let truth = spread(grid, 0x1234_5678);
    let target = board.run(&truth);
    let drift = spread(grid, 0x0BAD_C0DE);
    let mut guess: Vec<u32> = truth
        .iter()
        .zip(&drift)
        .map(|(one, other)| one.wrapping_add(other >> 2))
        .collect();

    let ended = board.run(&guess);
    let reached = ended
        .iter()
        .map(|one| f64::from(*one as i32).abs())
        .sum::<f64>()
        / ended.len() as f64;
    let first = apart(&ended, &target);
    let mut strength = 0.0;
    let mut last = first;
    for round in 0..rounds {
        let ended = board.run(&guess);
        let weight: Vec<u32> = ended
            .iter()
            .zip(&target)
            .map(|(one, other)| one.wrapping_sub(*other))
            .collect();
        let (slope, _) = board.carry(&guess, &weight);
        if round == 0 {
            strength = slope
                .iter()
                .map(|one| f64::from(*one as i32).abs())
                .sum::<f64>()
                / slope.len() as f64;
        }
        for (cell, step) in guess.iter_mut().zip(&slope) {
            let moved = i64::from(*cell) - i64::from((*step as i32) >> pace);
            *cell = moved.clamp(0, i64::from(CEILING) - 1) as u32;
        }
        last = apart(&board.run(&guess), &target);
    }
    (first, last, strength, reached)
}

pub fn leaning(slope: &[u32], guess: &[u32], truth: &[u32]) -> f64 {
    let mut along = 0.0_f64;
    let mut held = 0.0_f64;
    let mut home = 0.0_f64;
    for ((one, near), far) in slope.iter().zip(guess).zip(truth) {
        let step = f64::from(*one as i32);
        let away = f64::from(near.wrapping_sub(*far) as i32);
        along += step * away;
        held += step * step;
        home += away * away;
    }
    if held == 0.0 || home == 0.0 {
        return 0.0;
    }
    along / (held.sqrt() * home.sqrt())
}

pub fn blended(truth: &[u32], guess: &[u32], part: f64) -> Vec<u32> {
    truth
        .iter()
        .zip(guess)
        .map(|(far, near)| {
            let away = f64::from(near.wrapping_sub(*far) as i32);
            let moved = f64::from(*far as i32) + away * part;
            moved as i32 as u32
        })
        .collect()
}

pub fn making(grid: Grid) -> (Vec<u32>, Vec<u32>) {
    let truth = spread(grid, 0x1234_5678);
    let drift = spread(grid, 0x0BAD_C0DE);
    let guess = truth
        .iter()
        .zip(&drift)
        .map(|(one, other)| one.wrapping_add(other >> 2))
        .collect();
    (truth, guess)
}

pub fn stepping(board: &mut Board<'_>, target: &[u32], guess: &mut [u32], rounds: u32) {
    for _ in 0..rounds {
        let ended = board.run(guess);
        let weight: Vec<u32> = ended
            .iter()
            .zip(target)
            .map(|(one, other)| one.wrapping_sub(*other))
            .collect();
        let (slope, _) = board.carry(guess, &weight);
        for (cell, step) in guess.iter_mut().zip(&slope) {
            let moved = i64::from(*cell) - i64::from((*step as i32) >> PACE);
            *cell = moved.clamp(0, i64::from(CEILING) - 1) as u32;
        }
    }
}

pub fn climbed(board: &mut Board<'_>, grid: Grid, rungs: &[u32], rounds: u32) -> (f64, f64) {
    let (truth, mut guess) = making(grid);
    let strayed = apart(&guess, &truth);
    for depth in rungs {
        board.depth = *depth;
        let target = board.run(&truth);
        stepping(board, &target, &mut guess, rounds);
    }
    (strayed, apart(&guess, &truth))
}

pub fn squared(one: &[u32], other: &[u32]) -> f64 {
    one.iter()
        .zip(other)
        .map(|(held, want)| {
            let apart = f64::from(held.wrapping_sub(*want) as i32);
            apart * apart
        })
        .sum::<f64>()
        / 2.0
}

pub fn asking(board: &mut Board<'_>, start: &[u32], target: &[u32], angle: u32) -> f64 {
    let swing = board.angle.1;
    board.tune(angle, swing);
    let ended = board.run(start);
    let weight: Vec<u32> = ended
        .iter()
        .zip(target)
        .map(|(one, other)| one.wrapping_sub(*other))
        .collect();
    board.carry(start, &weight);
    board.told().0
}

pub fn muddied(target: &[u32], grid: Grid, seed: u32, loud: u32) -> Vec<u32> {
    let noise = spread(grid, seed);
    target
        .iter()
        .zip(&noise)
        .map(|(one, other)| {
            let span = 2 * loud + 1;
            let off = (other % span) as i32 - loud as i32;
            (*one as i32).wrapping_add(off) as u32
        })
        .collect()
}

pub fn hunted(board: &mut Board<'_>, start: &[u32], target: &[u32], from: u32) -> u32 {
    let mut here = from;
    let mut span = 1_i64;
    let low;
    let high;
    let leaning = asking(board, start, target, here) > 0.0;
    loop {
        let next = if leaning {
            i64::from(here) - span
        } else {
            i64::from(here) + span
        };
        let held = next.clamp(1, 4096) as u32;
        let now = asking(board, start, target, held) > 0.0;
        if now != leaning || span > 512 {
            (low, high) = if leaning {
                (i64::from(held), i64::from(here))
            } else {
                (i64::from(here), i64::from(held))
            };
            break;
        }
        here = held;
        span *= 2;
    }
    let (mut low, mut high) = (low, high);
    while high - low > 1 {
        let middle = (low + high) / 2;
        if asking(board, start, target, middle.clamp(1, 4096) as u32) > 0.0 {
            high = middle;
        } else {
            low = middle;
        }
    }
    low as u32
}

pub fn narrowed(board: &mut Board<'_>, start: &[u32], target: &[u32], from: u32) -> (u32, u32) {
    let mut runs = 0_u32;
    let loss = |board: &mut Board<'_>, at: u32, runs: &mut u32| {
        board.tune(at, board.angle.1);
        *runs += 1;
        squared(&board.run(start), target)
    };

    let mut low = 1_i64;
    let mut high = i64::from(from) * 2;
    while high - low > 2 {
        let third = (high - low) / 3;
        let left = low + third;
        let right = high - third;
        let here = loss(board, left as u32, &mut runs);
        let there = loss(board, right as u32, &mut runs);
        if here < there {
            high = right;
        } else {
            low = left;
        }
    }
    let mut best = low;
    let mut held = loss(board, low as u32, &mut runs);
    for at in (low + 1)..=high {
        let now = loss(board, at as u32, &mut runs);
        if now < held {
            held = now;
            best = at;
        }
    }
    (best as u32, runs)
}

pub fn strayed_where(one: &[u32], other: &[u32], live: &[bool]) -> f64 {
    let mut held = 0.0_f64;
    let mut seen = 0_usize;
    for ((a, b), on) in one.iter().zip(other).zip(live) {
        if *on {
            held += f64::from((*a as i32) - (*b as i32)).abs();
            seen += 1;
        }
    }
    held / seen.max(1) as f64
}

pub fn asked_for(
    board: &mut Board<'_>,
    start: &[u32],
    target: &[u32],
    guess: &[u32],
) -> (Vec<i32>, f64, f64) {
    board.sow(guess);
    let ended = board.run(start);
    let loss = squared(&ended, target);
    let weight: Vec<u32> = ended
        .iter()
        .zip(target)
        .map(|(one, other)| one.wrapping_sub(*other))
        .collect();
    board.sow(guess);
    board.carry(start, &weight);
    let told = board.told_each();
    let back = f64::from(1_u32 << SPARE) * f64::from(board.loud);
    let strength = told
        .iter()
        .map(|one| (f64::from(*one) / back).abs())
        .sum::<f64>()
        / told.len() as f64;
    (told, loss, strength)
}

pub fn worst(one: &[u32], other: &[u32]) -> f64 {
    one.iter()
        .zip(other)
        .map(|(a, b)| f64::from(a.wrapping_sub(*b) as i32).abs())
        .fold(0.0, f64::max)
}

pub fn apart(one: &[u32], other: &[u32]) -> f64 {
    one.iter()
        .zip(other)
        .map(|(a, b)| f64::from(a.wrapping_sub(*b) as i32).abs())
        .sum::<f64>()
        / one.len() as f64
}

pub const WIDER: u32 = 256;

pub const PATCH: u32 = 12;

pub fn over_list(subgroup: u32, grid: Grid, pass: Pass) -> Result<Vec<u32>, LaneError> {
    match subgroup {
        4 => over_listing::<4>(subgroup, grid, pass),
        8 => over_listing::<8>(subgroup, grid, pass),
        16 => over_listing::<16>(subgroup, grid, pass),
        32 => over_listing::<32>(subgroup, grid, pass),
        64 => over_listing::<64>(subgroup, grid, pass),
        other => Err(LaneError::NoMapping {
            lanes: other,
            width: subgroup,
        }),
    }
}

pub fn over_listing<const LANES: u32>(
    subgroup: u32,
    grid: Grid,
    pass: Pass,
) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, grit::layout::BINDINGS))?;
    let uint = kernel.index_type();
    let group = kernel.workgroup_index();
    let lane = kernel.local_index();
    let width = kernel.module().constant_u32(subgroup)?;
    let base = kernel.module().i_mul(uint, group, width)?;
    let slot = kernel.module().i_add(uint, base, lane)?;

    let head = kernel.module().constant_u32(0)?;
    let second = kernel.module().constant_u32(1)?;
    let frame = kernel.load_at(PARAMS, head)?;
    let grand = kernel.load_at(GRAND, head)?;
    let dump_base = kernel.load_at(PARAMS, second)?;

    let inside = {
        let mut lanes = kernel.lanes()?;
        let here = lanes.splat_id::<U32, LANES>(slot)?;
        let total = lanes.splat_id::<U32, LANES>(grand)?;
        let one = lanes.splat_bits::<U32, LANES>(1)?;
        let none = lanes.splat_bits::<U32, LANES>(0)?;
        let listed = lanes.greater_than(total, here)?;
        lanes.select(listed, one, none)?
    };

    let quad = kernel.load_at(DENSE, slot)?;
    let held = corners::<LANES>(&mut kernel, grid, quad, frame)?;
    let places = held.places;

    let state = [
        kernel.load_at(STATE, places[0].id())?,
        kernel.load_at(STATE, places[1].id())?,
        kernel.load_at(STATE, places[2].id())?,
        kernel.load_at(STATE, places[3].id())?,
    ];
    let carried = [
        kernel.load_at(SLOPE, places[0].id())?,
        kernel.load_at(SLOPE, places[1].id())?,
        kernel.load_at(SLOPE, places[2].id())?,
        kernel.load_at(SLOPE, places[3].id())?,
    ];

    let (state_next, weight_next, stirring) = {
        let mut lanes = kernel.lanes()?;
        let block = [
            lanes.splat_id::<U32, LANES>(state[0])?,
            lanes.splat_id::<U32, LANES>(state[1])?,
            lanes.splat_id::<U32, LANES>(state[2])?,
            lanes.splat_id::<U32, LANES>(state[3])?,
        ];
        let along = [
            lanes.splat_id::<U32, LANES>(carried[0])?,
            lanes.splat_id::<U32, LANES>(carried[1])?,
            lanes.splat_id::<U32, LANES>(carried[2])?,
            lanes.splat_id::<U32, LANES>(carried[3])?,
        ];
        let (state_next, weight_next) = match pass {
            Pass::Ahead => {
                let none = lanes.splat_bits::<U32, LANES>(0)?;
                (
                    onward::<LANES>(&mut lanes, Kind::Product, block, (none, none))?,
                    along,
                )
            }
            Pass::Back => {
                let none = lanes.splat_bits::<U32, LANES>(0)?;
                let (state, weight, _) =
                    backward::<LANES>(&mut lanes, Kind::Product, block, along, (none, none))?;
                (state, weight)
            }
        };

        let one = lanes.splat_bits::<U32, LANES>(1)?;
        let none = lanes.splat_bits::<U32, LANES>(0)?;
        let mut count = none;
        for (before, after) in block
            .into_iter()
            .chain(along)
            .zip(state_next.into_iter().chain(weight_next))
        {
            let same = lanes.equal(before, after)?;
            let differs = lanes.select(same, none, one)?;
            count = lanes.add(count, differs)?;
        }
        let moving = lanes.greater_than(count, none)?;
        let stirring = lanes.select(moving, one, none)?;
        (state_next, weight_next, stirring)
    };

    let spare = {
        let mut lanes = kernel.lanes()?;
        let past =
            lanes.splat_bits::<U32, LANES>(u32::try_from(grid.cells()).unwrap_or(u32::MAX))?;
        let mine = lanes.splat_id::<U32, LANES>(slot)?;
        let two = lanes.splat_bits::<U32, LANES>(2)?;
        let along = lanes.shift_left(mine, two)?;
        lanes.add(past, along)?
    };
    for (corner, place) in places.iter().enumerate() {
        let where_at = {
            let mut lanes = kernel.lanes()?;
            let none = lanes.splat_bits::<U32, LANES>(0)?;
            let step = lanes.splat_bits::<U32, LANES>(u32::try_from(corner).unwrap_or(0))?;
            let elsewhere = lanes.add(spare, step)?;
            let listed = lanes.greater_than(inside, none)?;
            lanes.select(listed, *place, elsewhere)?
        };
        if let (Some(one), Some(other)) = (state_next.get(corner), weight_next.get(corner)) {
            kernel.store_at(STATE, where_at.id(), one.id())?;
            kernel.store_at(SLOPE, where_at.id(), other.id())?;
        }
    }

    let where_at = Waking {
        subgroup,
        grid,
        column: held.along,
        row: held.down,
    };
    wake::<LANES>(&mut kernel, where_at, stirring, inside, dump_base)?;
    kernel.finish()
}

pub struct Listed<'host> {
    pub resident: grit::Resident<'host>,
    pub ahead: Vec<Vec<Groups>>,
    pub back: Vec<Vec<Groups>>,
    pub grid: Grid,
    pub plan: Plan,
}

impl Listed<'_> {
    pub fn rouse(&mut self, cells: &[u32], weight: &[u32]) {
        self.resident.write(STATE as usize, cells).unwrap();
        self.resident.write(SLOPE as usize, weight).unwrap();
        self.resident
            .write(PARAMS as usize, &[0, self.plan.dump()])
            .unwrap();
        live::seed(&mut self.resident, self.grid, &self.plan).unwrap();
    }

    pub fn run(&mut self, cells: &[u32]) -> (Vec<u32>, u32) {
        let quiet = vec![0_u32; self.grid.cells()];
        self.rouse(cells, &quiet);
        let stages: Vec<&[Groups]> = self.ahead.iter().map(Vec::as_slice).collect();
        for frame in 0..STEPS {
            self.resident
                .write_at(PARAMS as usize, 0, &[frame])
                .unwrap();
            self.resident.staged(&stages).unwrap();
        }
        let awake = self
            .resident
            .read(GRAND as usize, 1)
            .unwrap()
            .first()
            .copied()
            .unwrap_or(0);
        (
            self.resident
                .read(STATE as usize, self.grid.cells())
                .unwrap(),
            awake,
        )
    }

    pub fn carry(&mut self, ended: &[u32], weight: &[u32]) -> Vec<u32> {
        self.rouse(ended, weight);
        let stages: Vec<&[Groups]> = self.back.iter().map(Vec::as_slice).collect();
        for frame in (0..STEPS).rev() {
            self.resident
                .write_at(PARAMS as usize, 0, &[frame])
                .unwrap();
            self.resident.staged(&stages).unwrap();
        }
        self.resident
            .read(SLOPE as usize, self.grid.cells())
            .unwrap()
    }
}

pub fn patched(grid: Grid, seed: u32, span: u32) -> Vec<u32> {
    let mut state = seed;
    let mut cells = vec![0_u32; grid.cells()];
    let middle = (grid.wide / 2, grid.deep / 2);
    for row in middle.1..middle.1 + span {
        for column in middle.0..middle.0 + span {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            cells[(row * grid.wide + column) as usize] = state % CEILING;
        }
    }
    cells
}
