#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use grit::compact::Plan;
use grit::layout::{DENSE, GRAND, PARAMS, STATE};
use grit::live::{self, Waking, corners, wake};
use grit::sand::Grid;
use grit::{Groups, Host, sweep};
use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, Lanes, U32, Vector};

const WIDE: u32 = 64;
const DEEP: u32 = 64;

const STEPS: u32 = 8;

const SLOPE: u32 = 2;
const TOLD_TURNING: u32 = 3;
const TOLD_SWING: u32 = 4;
const FIELD: u32 = 5;
const BINDINGS: u32 = 6;

const POINT: u32 = 16;

const PRODUCT: u32 = 20;
const SHARE: u32 = 4;

const SPREAD: [u32; 3] = [2, 3, 4];

const TURNING: u32 = 8303;

const SWING: u32 = 16384;

const STEP: u32 = 1 << 9;

const GRAIN: u32 = 2;

const SPARE: u32 = 8;

const ROOM: u32 = 1 << 15;

fn device(label: &str) -> Option<Host> {
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
enum Kind {
    Product,
    Sum,
    Turn,
    Bound,
    Smooth,
}

type Pair<const LANES: u32> = (Vector<U32, LANES>, Vector<U32, LANES>);

fn pinned<const LANES: u32>(
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

fn gated<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    first: Vector<U32, LANES>,
    second: Vector<U32, LANES>,
) -> Result<Vector<U32, LANES>, LaneError> {
    let one = lanes.splat_bits::<U32, LANES>(1)?;
    let sum = lanes.add(first, second)?;
    lanes.and(sum, one)
}

fn scaled<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    one: Vector<U32, LANES>,
    by: Vector<U32, LANES>,
) -> Result<Vector<U32, LANES>, LaneError> {
    let point = lanes.splat_bits::<U32, LANES>(POINT)?;
    let held = lanes.mul(one, by)?;
    lanes.shift_right_arithmetic(held, point)
}

fn finer<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    one: Vector<U32, LANES>,
    by: Vector<U32, LANES>,
) -> Result<Vector<U32, LANES>, LaneError> {
    let point = lanes.splat_bits::<U32, LANES>(POINT - SPARE)?;
    let held = lanes.mul(one, by)?;
    lanes.shift_right_arithmetic(held, point)
}

fn spun<const LANES: u32>(
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

type Unwound<const LANES: u32> = (Pair<LANES>, Pair<LANES>, Pair<LANES>);

fn unspun<const LANES: u32>(
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
enum Pass {
    Ahead,
    Back,
}

fn mixed<const LANES: u32>(
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

fn onward<const LANES: u32>(
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

type Both<const LANES: u32> = (
    [Vector<U32, LANES>; 4],
    [Vector<U32, LANES>; 4],
    Pair<LANES>,
);

fn backward<const LANES: u32>(
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

fn built(subgroup: u32, grid: Grid, pass: Pass, kind: Kind) -> Result<Vec<u32>, LaneError> {
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

fn building<const LANES: u32>(
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

struct Board<'host> {
    resident: grit::Resident<'host>,
    over: Groups,
    grid: Grid,
    depth: u32,
    angle: (u32, u32),
    loud: u32,
    planted: bool,
}

impl Board<'_> {
    fn tune(&mut self, turning: u32, swing: u32) {
        self.angle = (turning, swing);
        let field = vec![turning; self.grid.cells()];
        self.resident.write(FIELD as usize, &field).unwrap();
        self.planted = true;
    }

    fn sow(&mut self, field: &[u32]) {
        self.resident.write(FIELD as usize, field).unwrap();
        self.planted = true;
    }

    fn run(&mut self, cells: &[u32]) -> Vec<u32> {
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

    fn slope(&mut self, cells: &[u32], at: usize) -> Vec<u32> {
        let mut weight = vec![0_u32; self.grid.cells()];
        weight[at] = 1 << POINT;
        self.carry(cells, &weight).0
    }

    fn wipe(&mut self) {
        let blank = vec![0_u32; self.grid.cells()];
        self.resident.write(TOLD_TURNING as usize, &blank).unwrap();
        self.resident.write(TOLD_SWING as usize, &blank).unwrap();
    }

    fn told_each(&mut self) -> Vec<i32> {
        self.resident
            .read(TOLD_TURNING as usize, self.grid.cells())
            .unwrap()
            .iter()
            .map(|one| *one as i32)
            .collect()
    }

    fn told(&mut self) -> (f64, f64) {
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

    fn carry(&mut self, cells: &[u32], weight: &[u32]) -> (Vec<u32>, Vec<u32>) {
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

fn spread(grid: Grid, seed: u32) -> Vec<u32> {
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

#[test]
fn the_slope_the_weights_report_is_the_slope_a_nudge_measures() {
    let Some(host) = device("the_slope_the_weights_report_is_the_slope_a_nudge_measures") else {
        return;
    };
    let width = host.width();
    let grid = Grid::of(WIDE, DEEP);
    let passes = [
        built(width, grid, Pass::Ahead, Kind::Product).unwrap(),
        built(width, grid, Pass::Back, Kind::Product).unwrap(),
    ];
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();

    let sizes = [
        8,
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
    ];
    let resident = host.resident(&sizes, &modules).unwrap();
    let mut board = Board {
        resident,
        over: Groups::Fixed(grid.quads().div_ceil(width)),
        grid,
        depth: STEPS,
        angle: (TURNING, SWING),
        loud: 1,
        planted: false,
    };

    let start = spread(grid, 0x1234_5678);
    let watched = (grid.deep / 2 * grid.wide + grid.wide / 2) as usize;
    let carried = board.slope(&start, watched);

    let mut seen = 0;
    let mut worst = 0.0_f64;
    for at in [
        watched,
        watched - 1,
        watched + 1,
        watched - WIDE as usize,
        7,
        4095,
    ] {
        let told = carried[at];
        if told == 0 {
            continue;
        }
        let mut nudged = start.clone();
        nudged[at] = nudged[at].wrapping_add(STEP);
        let moved = board.run(&nudged)[watched].wrapping_sub(board.run(&start)[watched]);

        let wanted = ((u64::from(told) * u64::from(STEP)) >> POINT) as u32;
        let apart = moved.abs_diff(wanted);
        let share = f64::from(apart) / f64::from(moved.max(wanted).max(1));
        eprintln!("{at} {told} {moved} {wanted} {apart} {share:.4}");
        assert!(apart <= GRAIN || share < 0.05, "{at} {moved} {wanted}");
        worst = worst.max(share);
        seen += 1;
    }

    assert!(seen >= 3);
    eprintln!("{worst:.4}");
}

#[test]
fn the_bounded_rules_slope_is_the_slope_a_nudge_measures() {
    let Some(host) = device("the_bounded_rules_slope_is_the_slope_a_nudge_measures") else {
        return;
    };
    let width = host.width();
    let grid = Grid::of(WIDE, DEEP);
    let passes = [
        built(width, grid, Pass::Ahead, Kind::Bound).unwrap(),
        built(width, grid, Pass::Back, Kind::Bound).unwrap(),
    ];
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();

    let sizes = [
        8,
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
    ];
    let resident = host.resident(&sizes, &modules).unwrap();
    let mut board = Board {
        resident,
        over: Groups::Fixed(grid.quads().div_ceil(width)),
        grid,
        depth: STEPS,
        angle: (TURNING, SWING),
        loud: 1,
        planted: false,
    };

    let start = spread(grid, 0x1234_5678);
    let watched = (grid.deep / 2 * grid.wide + grid.wide / 2) as usize;
    let carried = board.slope(&start, watched);
    let settled = board.run(&start)[watched];

    let mut seen = 0;
    let mut worst = 0.0_f64;
    for at in 0..grid.cells() {
        let told = carried[at];
        if told == 0 || seen >= 6 {
            continue;
        }
        let mut nudged = start.clone();
        nudged[at] = nudged[at].wrapping_add(STEP);
        let moved = board.run(&nudged)[watched].wrapping_sub(settled);
        let wanted = (((told as i32) as i64 * i64::from(STEP)) >> POINT) as i32 as u32;
        let apart = moved.abs_diff(wanted);
        let share = f64::from(apart) / f64::from(moved.max(wanted).max(1));
        eprintln!(
            "{at} {} {} {} {apart} {share:.4}",
            told as i32, moved as i32, wanted as i32
        );
        worst = worst.max(share);
        seen += 1;
    }

    eprintln!("{seen} points, worst {worst:.4}");
    assert!(seen >= 3);
    assert!(worst < 0.05, "{worst}");
}

const ROUNDS: u32 = 40;

const CEILING: u32 = 1 << 12;

const PACE: u32 = 3;

fn descended(board: &mut Board<'_>, grid: Grid, pace: u32, rounds: u32) -> (f64, f64, f64, f64) {
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

fn leaning(slope: &[u32], guess: &[u32], truth: &[u32]) -> f64 {
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

fn blended(truth: &[u32], guess: &[u32], part: f64) -> Vec<u32> {
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

fn making(grid: Grid) -> (Vec<u32>, Vec<u32>) {
    let truth = spread(grid, 0x1234_5678);
    let drift = spread(grid, 0x0BAD_C0DE);
    let guess = truth
        .iter()
        .zip(&drift)
        .map(|(one, other)| one.wrapping_add(other >> 2))
        .collect();
    (truth, guess)
}

fn stepping(board: &mut Board<'_>, target: &[u32], guess: &mut [u32], rounds: u32) {
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

fn climbed(board: &mut Board<'_>, grid: Grid, rungs: &[u32], rounds: u32) -> (f64, f64) {
    let (truth, mut guess) = making(grid);
    let strayed = apart(&guess, &truth);
    for depth in rungs {
        board.depth = *depth;
        let target = board.run(&truth);
        stepping(board, &target, &mut guess, rounds);
    }
    (strayed, apart(&guess, &truth))
}

#[test]
fn only_the_last_depth_counts_and_the_shallowest_wins() {
    let Some(host) = device("only_the_last_depth_counts_and_the_shallowest_wins") else {
        return;
    };
    let width = host.width();
    let grid = Grid::of(WIDE, DEEP);
    let passes = [
        built(width, grid, Pass::Ahead, Kind::Bound).unwrap(),
        built(width, grid, Pass::Back, Kind::Bound).unwrap(),
    ];
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let sizes = [
        8,
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
    ];
    let rounds = 30_u32;

    let ways: [(&str, Vec<u32>); 7] = [
        ("one", vec![1, 1, 1, 1]),
        ("four", vec![4, 4, 4, 4]),
        ("sixteen", vec![16, 16, 16, 16]),
        ("sixty four", vec![64, 64, 64, 64]),
        ("deep", vec![128, 128, 128, 128]),
        ("climbing", vec![16, 32, 64, 128]),
        ("falling", vec![128, 64, 32, 16]),
    ];

    eprintln!(
        "{:>9} {:>16} {:>14} {:>14} {:>9}",
        "way", "steps", "strayed", "left", "closed"
    );
    let mut told = Vec::new();
    for (named, rungs) in &ways {
        let resident = host.resident(&sizes, &modules).unwrap();
        let mut board = Board {
            resident,
            over: Groups::Fixed(grid.quads().div_ceil(width)),
            grid,
            depth: 16,
            angle: (TURNING / 64, SWING / 64),
            loud: 1,
            planted: false,
        };
        let (strayed, left) = climbed(&mut board, grid, rungs, rounds);
        let closed = if left > 0.0 { strayed / left } else { f64::MAX };
        let shown = rungs
            .iter()
            .map(|one| one.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        eprintln!("{named:>9} {shown:>16} {strayed:>14.1} {left:>14.1} {closed:>8.1}x");
        told.push((*named, closed));
    }

    let named = |want: &str| {
        told.iter()
            .find(|(held, _)| *held == want)
            .map_or(0.0, |(_, closed)| *closed)
    };
    let flat = [
        named("one"),
        named("four"),
        named("sixteen"),
        named("sixty four"),
        named("deep"),
    ];
    eprintln!(
        "one step closes {:.1}x and a hundred and twenty eight {:.1}x",
        flat[0], flat[4]
    );
    for pair in flat.windows(2) {
        assert!(pair[0] > pair[1], "{pair:?}");
    }
    assert!((named("falling") - named("sixteen")).abs() < 0.1);
    assert!(named("climbing") < named("sixteen"));
}

#[test]
fn whether_the_descent_works_where_the_loss_can_still_see() {
    let Some(host) = device("whether_the_descent_works_where_the_loss_can_still_see") else {
        return;
    };
    let width = host.width();
    let grid = Grid::of(WIDE, DEEP);
    let passes = [
        built(width, grid, Pass::Ahead, Kind::Bound).unwrap(),
        built(width, grid, Pass::Back, Kind::Bound).unwrap(),
    ];
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let sizes = [
        8,
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
    ];

    eprintln!(
        "{:>9} {:>7} {:>14} {:>14} {:>9}",
        "turning", "steps", "error first", "error last", "shrank"
    );
    let mut told = Vec::new();
    for share in [1_u32, 8, 64] {
        for depth in [32_u32, 256] {
            let resident = host.resident(&sizes, &modules).unwrap();
            let mut board = Board {
                resident,
                over: Groups::Fixed(grid.quads().div_ceil(width)),
                grid,
                depth,
                angle: (TURNING / share, SWING / share),
                loud: 1,
                planted: false,
            };
            let (first, last, _, _) = descended(&mut board, grid, PACE, ROUNDS);
            let shrank = if last > 0.0 { first / last } else { f64::MAX };
            eprintln!(
                "{:>9} {depth:>7} {first:>14.1} {last:>14.1} {shrank:>8.1}x",
                TURNING / share
            );
            told.push((TURNING / share, depth, shrank));
        }
    }

    let weak = told
        .iter()
        .filter(|(turning, _, _)| *turning < 200)
        .map(|(_, _, shrank)| *shrank)
        .fold(0.0_f64, f64::max);
    let strong = told
        .iter()
        .filter(|(turning, _, _)| *turning > 8000)
        .map(|(_, _, shrank)| *shrank)
        .fold(0.0_f64, f64::max);
    eprintln!("weakest mixing reaches {weak:.1}x, strongest {strong:.1}x");
    assert!(weak > strong, "{weak} {strong}");
}

fn squared(one: &[u32], other: &[u32]) -> f64 {
    one.iter()
        .zip(other)
        .map(|(held, want)| {
            let apart = f64::from(held.wrapping_sub(*want) as i32);
            apart * apart
        })
        .sum::<f64>()
        / 2.0
}

fn asking(board: &mut Board<'_>, start: &[u32], target: &[u32], angle: u32) -> f64 {
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

#[test]
fn whether_the_weight_stays_the_size_it_started() {
    let Some(host) = device("whether_the_weight_stays_the_size_it_started") else {
        return;
    };
    let width = host.width();
    let grid = Grid::of(WIDE, DEEP);
    let passes = [
        built(width, grid, Pass::Ahead, Kind::Smooth).unwrap(),
        built(width, grid, Pass::Back, Kind::Smooth).unwrap(),
    ];
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let sizes = [
        8,
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
    ];
    let resident = host.resident(&sizes, &modules).unwrap();

    let truth = TURNING / 64;
    let swing = SWING / 64;
    let mut board = Board {
        resident,
        over: Groups::Fixed(grid.quads().div_ceil(width)),
        grid,
        depth: 1,
        angle: (truth, swing),
        loud: 1,
        planted: false,
    };
    let start = spread(grid, 0x1234_5678);
    let biggest = |held: &[u32]| {
        held.iter()
            .map(|one| f64::from(*one as i32).abs())
            .fold(0.0_f64, f64::max)
    };

    eprintln!(
        "{:>7} {:>16} {:>16} {:>9}",
        "steps", "weight put in", "weight come back", "grown"
    );
    for depth in [16_u32, 64, 256, 1024] {
        board.depth = depth;
        board.tune(truth, swing);
        let target = board.run(&start);
        board.tune(truth + 16, swing);
        let ended = board.run(&start);
        let weight: Vec<u32> = ended
            .iter()
            .zip(&target)
            .map(|(one, other)| one.wrapping_sub(*other))
            .collect();
        let (carried, _) = board.carry(&start, &weight);
        let went = biggest(&weight);
        let came = biggest(&carried);
        eprintln!(
            "{depth:>7} {went:>16.0} {came:>16.0} {:>8.1}x",
            came / went.max(1.0)
        );
    }
    board.tune(truth, swing);
}

#[test]
fn how_rough_the_loss_is_close_up_at_each_depth() {
    let Some(host) = device("how_rough_the_loss_is_close_up_at_each_depth") else {
        return;
    };
    let width = host.width();
    let grid = Grid::of(WIDE, DEEP);
    let passes = [
        built(width, grid, Pass::Ahead, Kind::Smooth).unwrap(),
        built(width, grid, Pass::Back, Kind::Smooth).unwrap(),
    ];
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let sizes = [
        8,
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
    ];
    let resident = host.resident(&sizes, &modules).unwrap();

    let truth = TURNING / 64;
    let swing = SWING / 64;
    let mut board = Board {
        resident,
        over: Groups::Fixed(grid.quads().div_ceil(width)),
        grid,
        depth: 1,
        angle: (truth, swing),
        loud: 1,
        planted: false,
    };
    let start = spread(grid, 0x1234_5678);

    eprintln!("{:>7} {:>10} {:>14}", "steps", "dents", "worst dent");
    let mut told = Vec::new();
    for depth in [16_u32, 64, 256, 1024] {
        board.depth = depth;
        board.tune(truth, swing);
        let target = board.run(&start);

        let mut walk = Vec::new();
        for here in truth..=(truth + 48) {
            board.tune(here, swing);
            walk.push(squared(&board.run(&start), &target));
        }

        let mut dents = 0;
        let mut worst = 0_u32;
        for at in 1..walk.len() - 1 {
            if walk[at] < walk[at - 1] && walk[at] < walk[at + 1] {
                dents += 1;
                worst = worst.max(at as u32);
            }
        }
        eprintln!("{depth:>7} {dents:>10} {worst:>14}");
        told.push((depth, dents));
    }
    board.tune(truth, swing);

    for (depth, dents) in &told {
        eprintln!("{depth:>5} steps: {dents} places to get stuck between 0 and 48 out");
    }
}

fn muddied(target: &[u32], grid: Grid, seed: u32, loud: u32) -> Vec<u32> {
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

fn hunted(board: &mut Board<'_>, start: &[u32], target: &[u32], from: u32) -> u32 {
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

#[test]
fn what_depth_buys_is_signal_and_signal_buys_room_for_noise() {
    let Some(host) = device("what_depth_buys_is_signal_and_signal_buys_room_for_noise") else {
        return;
    };
    let width = host.width();
    let grid = Grid::of(WIDE, DEEP);
    let passes = [
        built(width, grid, Pass::Ahead, Kind::Smooth).unwrap(),
        built(width, grid, Pass::Back, Kind::Smooth).unwrap(),
    ];
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let sizes = [
        8,
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
    ];
    let resident = host.resident(&sizes, &modules).unwrap();

    let truth = TURNING / 64;
    let swing = SWING / 64;
    let mut board = Board {
        resident,
        over: Groups::Fixed(grid.quads().div_ceil(width)),
        grid,
        depth: 1,
        angle: (truth, swing),
        loud: 1,
        planted: false,
    };
    let start = spread(grid, 0x1234_5678);
    let louds = [0_u32, 16, 64, 256, 1024, 4096];

    eprintln!(
        "{:>7} {}",
        "steps",
        louds
            .iter()
            .map(|one| format!("{:>10}", format!("noise {one}")))
            .collect::<String>()
    );

    let mut told = Vec::new();
    for depth in [1_u32, 16, 256, 1024] {
        board.depth = depth;
        board.tune(truth, swing);
        let clean = board.run(&start);

        let mut shown = String::new();
        let mut stood = 0_u32;
        for loud in louds {
            let target = if loud == 0 {
                clean.clone()
            } else {
                muddied(&clean, grid, 0x0FFE_1234, loud)
            };
            let found = hunted(&mut board, &start, &target, truth + 16);
            shown.push_str(&format!("{:>10}", found.abs_diff(truth)));
            if found.abs_diff(truth) <= 1 {
                stood = loud;
            }
        }
        eprintln!("{depth:>7} {shown}");
        told.push((depth, stood));
    }
    board.tune(truth, swing);

    for (depth, stood) in &told {
        eprintln!("{depth:>5} steps: still lands on the answer through noise of {stood}");
    }
    let shallow = told.first().map_or(0, |(_, stood)| *stood);
    let deep = told.last().map_or(0, |(_, stood)| *stood);
    assert!(deep >= shallow, "{shallow} {deep}");
}

fn narrowed(board: &mut Board<'_>, start: &[u32], target: &[u32], from: u32) -> (u32, u32) {
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

fn strayed_where(one: &[u32], other: &[u32], live: &[bool]) -> f64 {
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

fn asked_for(
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

#[test]
fn whether_the_plateau_is_the_step_or_the_place() {
    let Some(host) = device("whether_the_plateau_is_the_step_or_the_place") else {
        return;
    };
    let width = host.width();
    let grid = Grid::of(WIDE, DEEP);
    let passes = [
        built(width, grid, Pass::Ahead, Kind::Smooth).unwrap(),
        built(width, grid, Pass::Back, Kind::Smooth).unwrap(),
    ];
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let sizes = [
        8,
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
    ];
    let resident = host.resident(&sizes, &modules).unwrap();

    let base = TURNING / 64;
    let swing = SWING / 64;
    let mut board = Board {
        resident,
        over: Groups::Fixed(grid.quads().div_ceil(width)),
        grid,
        depth: 256,
        angle: (base, swing),
        loud: 1,
        planted: false,
    };

    let start = spread(grid, 0x1234_5678);
    let scatter = spread(grid, 0x00C0_FFEE);
    let truth: Vec<u32> = scatter.iter().map(|one| base + (one % 33) - 16).collect();
    board.tune(base, swing);
    board.sow(&truth);
    let target = board.run(&start);

    let rounds = 24_u32;
    eprintln!(
        "{:>12} {:>12} {:>12} {:>12} {:>12}",
        "way", "loss first", "loss last", "slope first", "slope last"
    );

    let mut told_ways = Vec::new();
    for way in ["one each", "halving", "by size", "searched"] {
        let mut guess = vec![base; grid.cells()];
        let mut first = 0.0;
        let mut strong_first = 0.0;
        for round in 0..rounds {
            let (told, loss, strength) = asked_for(&mut board, &start, &target, &guess);
            if round == 0 {
                first = loss;
                strong_first = strength;
            }

            let widest = told.iter().map(|one| one.abs()).max().unwrap_or(1).max(1);
            if way == "searched" {
                let mut best = guess.clone();
                let mut held = loss;
                for step in [8_i64, 4, 2, 1] {
                    let tried: Vec<u32> = guess
                        .iter()
                        .zip(&told)
                        .map(|(one, slope)| {
                            let moved = i64::from(*one) - i64::from(slope.signum()) * step;
                            moved.clamp(1, 4096) as u32
                        })
                        .collect();
                    board.sow(&tried);
                    let now = squared(&board.run(&start), &target);
                    if now < held {
                        held = now;
                        best = tried;
                    }
                }
                guess = best;
                continue;
            }
            for (held, slope) in guess.iter_mut().zip(&told) {
                let moved = match way {
                    "one each" => i64::from(*held) - i64::from(slope.signum()),
                    "halving" => {
                        let step = 8_i64 >> (round / 6).min(3);
                        i64::from(*held) - i64::from(slope.signum()) * step
                    }
                    _ => {
                        let share = f64::from(*slope) / f64::from(widest);
                        i64::from(*held) - (share * 4.0).round() as i64
                    }
                };
                *held = moved.clamp(1, 4096) as u32;
            }
        }
        let (_, last, strong_last) = asked_for(&mut board, &start, &target, &guess);
        eprintln!(
            "{way:>12} {first:>12.3e} {last:>12.3e} {strong_first:>12.2} {strong_last:>12.2}"
        );
        told_ways.push((way, first, last, strong_first, strong_last));
    }

    for (way, first, last, _, _) in &told_ways {
        eprintln!("{way}: {:.2}x down", first / last.max(1.0));
    }
    let best = told_ways
        .iter()
        .map(|(_, first, last, _, _)| first / last.max(1.0))
        .fold(0.0_f64, f64::max);
    let standing = told_ways
        .iter()
        .find(|(way, _, _, _, _)| *way == "one each")
        .map_or((0.0, 0.0), |(_, _, _, began, ended)| (*began, *ended));
    eprintln!(
        "the simplest step leaves the slope at {:.0}% of what it started",
        standing.1 * 100.0 / standing.0.max(1.0)
    );
    assert!(best > 1.5, "{best}");
    assert!(standing.1 > standing.0 / 3.0, "{standing:?}");
}

#[test]
fn one_cells_share_of_the_slope_is_what_nudging_that_cell_measures() {
    let Some(host) = device("one_cells_share_of_the_slope_is_what_nudging_that_cell_measures")
    else {
        return;
    };
    let width = host.width();
    let grid = Grid::of(WIDE, DEEP);
    let passes = [
        built(width, grid, Pass::Ahead, Kind::Smooth).unwrap(),
        built(width, grid, Pass::Back, Kind::Smooth).unwrap(),
    ];
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let sizes = [
        8,
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
    ];
    let resident = host.resident(&sizes, &modules).unwrap();

    let base = TURNING / 64;
    let swing = SWING / 64;
    let mut board = Board {
        resident,
        over: Groups::Fixed(grid.quads().div_ceil(width)),
        grid,
        depth: 64,
        angle: (base, swing),
        loud: 1,
        planted: false,
    };

    let start = spread(grid, 0x1234_5678);
    let scatter = spread(grid, 0x00C0_FFEE);
    let truth: Vec<u32> = scatter.iter().map(|one| base + (one % 33) - 16).collect();
    board.tune(base, swing);
    board.sow(&truth);
    let target = board.run(&start);

    let guess = vec![base; grid.cells()];
    board.sow(&guess);
    let ended = board.run(&start);
    let weight: Vec<u32> = ended
        .iter()
        .zip(&target)
        .map(|(one, other)| one.wrapping_sub(*other))
        .collect();
    board.sow(&guess);
    board.carry(&start, &weight);
    let told = board.told_each();
    let back = f64::from(1_u32 << SPARE) * f64::from(board.loud);

    let mut ranked: Vec<(usize, i32)> = told
        .iter()
        .enumerate()
        .map(|(at, one)| (at, *one))
        .collect();
    ranked.sort_by_key(|(_, one)| -one.abs());

    eprintln!(
        "{:>8} {:>16} {:>16} {:>9}",
        "cell", "its share", "nudging it says", "apart"
    );
    let mut told_worst = Vec::new();
    for step in [8_i64, 32, 128] {
        let mut worst = 0.0_f64;
        let mut seen = 0_usize;
        for (at, raw) in ranked.iter().take(6) {
            let mine = f64::from(*raw) / back;

            let mut above = guess.clone();
            above[*at] = (i64::from(guess[*at]) + step) as u32;
            board.sow(&above);
            let high = squared(&board.run(&start), &target);

            let mut below = guess.clone();
            below[*at] = (i64::from(guess[*at]) - step) as u32;
            board.sow(&below);
            let low = squared(&board.run(&start), &target);

            let nudged = (high - low) / (2.0 * step as f64);
            let apart = (mine - nudged).abs() / mine.abs().max(nudged.abs()).max(1.0);
            if step == 32 {
                eprintln!("{at:>8} {mine:>16.1} {nudged:>16.1} {apart:>8.4}");
            }
            worst = worst.max(apart);
            seen += 1;
        }
        eprintln!("a nudge of {step:>4}: worst of {seen} is {worst:.4}");
        told_worst.push((step, worst));
    }

    let best = told_worst
        .iter()
        .map(|(_, one)| *one)
        .fold(f64::MAX, f64::min);
    let ends = told_worst
        .iter()
        .filter(|(step, _)| *step != 32)
        .map(|(_, one)| *one)
        .fold(0.0_f64, f64::max);
    eprintln!("best {best:.4}, and worse at both ends, up to {ends:.4}");
    assert!(best < 0.05, "{best}");
    assert!(ends > best, "{best} {ends}");
}

#[test]
fn many_numbers_at_once_are_what_the_walk_back_is_for() {
    let Some(host) = device("many_numbers_at_once_are_what_the_walk_back_is_for") else {
        return;
    };
    let width = host.width();
    let grid = Grid::of(WIDE, DEEP);
    let passes = [
        built(width, grid, Pass::Ahead, Kind::Smooth).unwrap(),
        built(width, grid, Pass::Back, Kind::Smooth).unwrap(),
    ];
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let sizes = [
        8,
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
    ];
    let resident = host.resident(&sizes, &modules).unwrap();

    let base = TURNING / 64;
    let swing = SWING / 64;
    let mut board = Board {
        resident,
        over: Groups::Fixed(grid.quads().div_ceil(width)),
        grid,
        depth: 256,
        angle: (base, swing),
        loud: 1,
        planted: false,
    };

    let start = spread(grid, 0x1234_5678);
    let scatter = spread(grid, 0x00C0_FFEE);

    let probe: Vec<u32> = scatter.iter().map(|one| base + (one % 33) - 16).collect();
    board.tune(base, swing);
    board.sow(&probe);
    let seen = board.run(&start);
    let weight: Vec<u32> = seen.iter().map(|one| one.wrapping_sub(base)).collect();
    board.sow(&probe);
    board.carry(&start, &weight);
    let live: Vec<bool> = board.told_each().iter().map(|one| *one != 0).collect();
    let counted = live.iter().filter(|one| **one).count();

    let truth: Vec<u32> = live
        .iter()
        .zip(&scatter)
        .map(|(on, one)| if *on { base + (one % 33) - 16 } else { base })
        .collect();

    eprintln!("{:>7} {:>10} {:>12}", "steps", "have one", "point right");
    for depth in [16_u32, 64, 256, 1024] {
        board.depth = depth;
        board.sow(&truth);
        let aim = board.run(&start);
        let guess = vec![base; grid.cells()];
        board.sow(&guess);
        let ended = board.run(&start);
        let weight: Vec<u32> = ended
            .iter()
            .zip(&aim)
            .map(|(one, other)| one.wrapping_sub(*other))
            .collect();
        board.sow(&guess);
        board.carry(&start, &weight);
        let told = board.told_each();
        let mut agreed = 0_usize;
        let mut spoke = 0_usize;
        for (((slope, held), want), on) in told.iter().zip(&guess).zip(&truth).zip(&live) {
            if !*on || *slope == 0 {
                continue;
            }
            spoke += 1;
            let off = (*held as i32) - (*want as i32);
            if off != 0 && slope.signum() == off.signum() {
                agreed += 1;
            }
        }
        eprintln!(
            "{depth:>7} {spoke:>10} {:>11.1}%",
            agreed as f64 * 100.0 / spoke.max(1) as f64
        );
    }
    board.depth = 1024;
    board.sow(&truth);
    let target = board.run(&start);

    let mut guess = vec![base; grid.cells()];
    let first = strayed_where(&guess, &truth, &live);

    let schedule = [1_u32; 32];
    let began = std::time::Instant::now();
    let mut walk: Vec<(f64, f64)> = Vec::new();
    for step in schedule {
        board.sow(&guess);
        let ended = board.run(&start);
        let weight: Vec<u32> = ended
            .iter()
            .zip(&target)
            .map(|(one, other)| one.wrapping_sub(*other))
            .collect();
        board.sow(&guess);
        board.carry(&start, &weight);
        let told = board.told_each();
        for (held, slope) in guess.iter_mut().zip(&told) {
            let moved = i64::from(*held) - i64::from(slope.signum()) * i64::from(step);
            *held = moved.clamp(1, 4096) as u32;
        }
        board.sow(&guess);
        walk.push((
            strayed_where(&guess, &truth, &live),
            squared(&board.run(&start), &target),
        ));
    }
    let took = began.elapsed();
    let last = strayed_where(&guess, &truth, &live);

    eprintln!("{counted} numbers, {} steps", board.depth);
    eprintln!(
        "out by {first:.2} at the start and {last:.2} after {} rounds, {took:.2?}",
        schedule.len()
    );
    let away = walk
        .iter()
        .enumerate()
        .filter(|(at, _)| at % 4 == 3)
        .map(|(_, (one, _))| format!("{one:.2}"))
        .collect::<Vec<_>>()
        .join(" ");
    let loss = walk
        .iter()
        .enumerate()
        .filter(|(at, _)| at % 4 == 3)
        .map(|(_, (_, one))| format!("{one:.3e}"))
        .collect::<Vec<_>>()
        .join(" ");
    eprintln!("away from the truth: {away}");
    eprintln!("the loss itself:     {loss}");
    eprintln!(
        "{} passes for all of them; one run apiece would be {}",
        schedule.len() * 3,
        schedule.len() * counted
    );
    let began_loss = walk.first().map_or(0.0, |(_, one)| *one);
    let ended_loss = walk.last().map_or(0.0, |(_, one)| *one);
    eprintln!("the loss went {began_loss:.3e} to {ended_loss:.3e}");
    assert!(ended_loss < began_loss, "{began_loss} {ended_loss}");
}

#[test]
fn what_the_same_answer_costs_without_any_gradient_at_all() {
    let Some(host) = device("what_the_same_answer_costs_without_any_gradient_at_all") else {
        return;
    };
    let width = host.width();
    let truth = TURNING / 64;
    let swing = SWING / 64;

    eprintln!(
        "{:>10} {:>7} {:>26} {:>26}",
        "cells", "steps", "walking the slope", "no slope at all"
    );

    let mut told = Vec::new();
    for (side, depth) in [(64_u32, 256_u32), (512, 1024)] {
        let grid = Grid::of(side, side);
        let passes = [
            built(width, grid, Pass::Ahead, Kind::Smooth).unwrap(),
            built(width, grid, Pass::Back, Kind::Smooth).unwrap(),
        ];
        let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
        let sizes = [
            8,
            grid.cells(),
            grid.cells(),
            grid.cells(),
            grid.cells(),
            grid.cells(),
        ];
        let resident = host.resident(&sizes, &modules).unwrap();
        let mut board = Board {
            resident,
            over: Groups::Fixed(grid.quads().div_ceil(width)),
            grid,
            depth,
            angle: (truth, swing),
            loud: 1,
            planted: false,
        };

        let start = spread(grid, 0x1234_5678);
        board.tune(truth, swing);
        let target = board.run(&start);

        let began = std::time::Instant::now();
        let found = hunted(&mut board, &start, &target, truth + 16);
        let slope_took = began.elapsed();

        let began = std::time::Instant::now();
        let (blind, runs) = narrowed(&mut board, &start, &target, truth + 16);
        let blind_took = began.elapsed();

        eprintln!(
            "{:>10} {depth:>7}   {found:>5} in 30 passes {slope_took:>8.2?}   {blind:>5} in {runs:>2} passes {blind_took:>8.2?}",
            grid.cells()
        );
        told.push((grid.cells(), found, blind, runs));
    }

    for (cells, found, blind, runs) in &told {
        eprintln!(
            "{cells} cells: the slope found {found}, {runs} plain runs found {blind}, the answer is {truth}"
        );
    }
    for (_, found, blind, _) in &told {
        assert_eq!(*found, truth);
        assert_eq!(*blind, truth);
    }
}

#[test]
fn the_angle_is_found_where_the_tape_would_not_fit() {
    let Some(host) = device("the_angle_is_found_where_the_tape_would_not_fit") else {
        return;
    };
    let width = host.width();
    let truth = TURNING / 64;
    let swing = SWING / 64;
    let depth = 1024_u32;

    eprintln!(
        "{:>12} {:>10} {:>12} {:>8} {:>8} {:>10}",
        "cells", "held", "a tape", "found", "times", "took"
    );

    let mut told = Vec::new();
    for side in [64_u32, 512, 2048, 4096] {
        let grid = Grid::of(side, side);
        let passes = [
            built(width, grid, Pass::Ahead, Kind::Smooth).unwrap(),
            built(width, grid, Pass::Back, Kind::Smooth).unwrap(),
        ];
        let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
        let sizes = [
            8,
            grid.cells(),
            grid.cells(),
            grid.cells(),
            grid.cells(),
            grid.cells(),
        ];
        let resident = host.resident(&sizes, &modules).unwrap();
        let mut board = Board {
            resident,
            over: Groups::Fixed(grid.quads().div_ceil(width)),
            grid,
            depth,
            angle: (truth, swing),
            loud: 1,
            planted: false,
        };

        let start = spread(grid, 0x1234_5678);
        board.tune(truth, swing);
        let target = board.run(&start);

        let began = std::time::Instant::now();
        let found = hunted(&mut board, &start, &target, truth + 16);
        let took = began.elapsed();

        let held = (grid.cells() * size_of::<u32>() * 4) as f64 / (1 << 20) as f64;
        let tape = f64::from(depth) * (grid.cells() * size_of::<u32>()) as f64 / (1 << 30) as f64;
        eprintln!(
            "{:>12} {held:>7.0} MiB {tape:>9.1} GiB {found:>8} {:>8.0} {took:>10.2?}",
            grid.cells(),
            tape * 1024.0 / held
        );
        told.push((grid.cells(), found));

        if side == 4096 {
            let away = 192_u32;
            let here = truth + away;
            let slope = asking(&mut board, &start, &target, here);
            let step = away / 4;
            board.tune(here + step, swing);
            let above = squared(&board.run(&start), &target);
            board.tune(here - step, swing);
            let below = squared(&board.run(&start), &target);
            let nudged = (above - below) / f64::from(2 * step);
            let apart = (slope - nudged).abs() / slope.abs().max(nudged.abs()).max(1.0);
            eprintln!(
                "at {} cells the slope says {slope:.0} and a nudge says {nudged:.0}, {apart:.4} apart",
                grid.cells()
            );
            assert!(apart < 0.10, "{apart}");
        }
    }

    for (cells, found) in &told {
        eprintln!(
            "{cells} cells: landed {} from {truth}",
            found.abs_diff(truth)
        );
    }
    for (_, found) in &told {
        assert_eq!(*found, truth);
    }
}

#[test]
fn the_smallest_difference_the_slope_can_point_at() {
    let Some(host) = device("the_smallest_difference_the_slope_can_point_at") else {
        return;
    };
    let width = host.width();
    let grid = Grid::of(WIDE, DEEP);
    let passes = [
        built(width, grid, Pass::Ahead, Kind::Smooth).unwrap(),
        built(width, grid, Pass::Back, Kind::Smooth).unwrap(),
    ];
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let sizes = [
        8,
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
    ];
    let resident = host.resident(&sizes, &modules).unwrap();

    let truth = TURNING / 64;
    let swing = SWING / 64;
    let mut board = Board {
        resident,
        over: Groups::Fixed(grid.quads().div_ceil(width)),
        grid,
        depth: 1,
        angle: (truth, swing),
        loud: 1,
        planted: false,
    };

    let grids = [spread(grid, 0x1234_5678), spread(grid, 0x5EED_1057)];
    let aways = [1_u32, 2, 4, 8, 16, 32, 64];

    eprintln!(
        "{:>7} {}  {:>8}",
        "steps",
        aways
            .iter()
            .map(|one| format!("{one:>5}"))
            .collect::<String>(),
        "finest"
    );

    let mut found = Vec::new();
    for depth in [1_u32, 4, 16, 64, 256, 1024] {
        board.depth = depth;
        let targets: Vec<Vec<u32>> = grids
            .iter()
            .map(|start| {
                board.tune(truth, swing);
                board.run(start)
            })
            .collect();

        let mut shown = String::new();
        let mut finest = None;
        for away in aways {
            let mut sound = true;
            for (start, target) in grids.iter().zip(&targets) {
                let above = asking(&mut board, start, target, truth + away);
                let below = asking(&mut board, start, target, truth - away);
                if above <= 0.0 || below >= 0.0 {
                    sound = false;
                }
            }
            shown.push_str(if sound { "    y" } else { "    ." });
            if sound && finest.is_none() {
                finest = Some(away);
            }
        }
        match finest {
            Some(away) => {
                eprintln!("{depth:>7} {shown}  {away:>8}");
                found.push((depth, away));
            }
            None => eprintln!("{depth:>7} {shown}  {:>8}", "none"),
        }
    }
    board.tune(truth, swing);

    for (depth, away) in &found {
        eprintln!("{depth:>5} steps: points home from {away} out, both ways, on both grids");
    }
    let coarse = found.first().map_or(0, |(_, away)| *away);
    let fine = found.last().map_or(0, |(_, away)| *away);
    assert!(coarse >= fine, "{coarse} {fine}");
}

#[test]
fn the_angle_is_found_by_walking_the_slope_that_was_checked() {
    let Some(host) = device("the_angle_is_found_by_walking_the_slope_that_was_checked") else {
        return;
    };
    let width = host.width();
    let grid = Grid::of(WIDE, DEEP);
    let passes = [
        built(width, grid, Pass::Ahead, Kind::Smooth).unwrap(),
        built(width, grid, Pass::Back, Kind::Smooth).unwrap(),
    ];
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let sizes = [
        8,
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
    ];
    let resident = host.resident(&sizes, &modules).unwrap();

    let truth = TURNING / 64;
    let swing = SWING / 64;
    let mut board = Board {
        resident,
        over: Groups::Fixed(grid.quads().div_ceil(width)),
        grid,
        depth: 1,
        angle: (truth, swing),
        loud: 1,
        planted: false,
    };
    let start = spread(grid, 0x1234_5678);

    eprintln!(
        "{:>7} {:>8} {:>8} {:>6} {:>34}",
        "steps", "from", "found", "asked", "the walk"
    );
    let mut told_reach = Vec::new();
    for depth in [1_u32, 16, 256, 1024] {
        board.depth = depth;
        board.tune(truth, swing);
        let target = board.run(&start);

        let mut here = truth + 16;
        let mut walked = vec![here];
        let mut asked = 1_u32;

        let mut span = 1_i64;
        let low;
        let high;
        let leaning = asking(&mut board, &start, &target, here) > 0.0;
        loop {
            let next = if leaning {
                i64::from(here) - span
            } else {
                i64::from(here) + span
            };
            let held = next.clamp(1, 4096) as u32;
            walked.push(held);
            asked += 1;
            let now = asking(&mut board, &start, &target, held) > 0.0;
            if now != leaning || span > 512 {
                (low, high) = if leaning {
                    (i64::from(held), i64::from(here))
                } else {
                    (i64::from(here), i64::from(held))
                };
                break;
            }
            span *= 2;
        }
        let (mut low, mut high) = (low, high);

        while high - low > 1 {
            let middle = (low + high) / 2;
            let held = middle.clamp(1, 4096) as u32;
            walked.push(held);
            asked += 1;
            if asking(&mut board, &start, &target, held) > 0.0 {
                high = middle;
            } else {
                low = middle;
            }
        }
        here = low as u32;

        let shown = walked
            .iter()
            .take(6)
            .map(|one| one.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        eprintln!(
            "{depth:>7} {:>8} {here:>8} {asked:>6}   {shown}",
            truth + 16
        );
        told_reach.push((depth, here.abs_diff(truth)));
    }

    for (depth, off) in &told_reach {
        eprintln!("{depth:>5} steps: landed {off} from {truth}");
    }
    let deepest = told_reach.last().map_or(u32::MAX, |(_, off)| *off);
    assert!(deepest <= 1, "{deepest}");
}

#[test]
fn the_angles_slope_is_the_slope_a_nudge_to_the_angle_measures() {
    let Some(host) = device("the_angles_slope_is_the_slope_a_nudge_to_the_angle_measures") else {
        return;
    };
    let width = host.width();
    let grid = Grid::of(WIDE, DEEP);
    let passes = [
        built(width, grid, Pass::Ahead, Kind::Smooth).unwrap(),
        built(width, grid, Pass::Back, Kind::Smooth).unwrap(),
    ];
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let sizes = [
        8,
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
    ];
    let resident = host.resident(&sizes, &modules).unwrap();

    let base = TURNING / 64;
    let swing = SWING / 64;
    let mut board = Board {
        resident,
        over: Groups::Fixed(grid.quads().div_ceil(width)),
        grid,
        depth: 1,
        angle: (base, swing),
        loud: 1,
        planted: false,
    };

    let start = spread(grid, 0x1234_5678);
    eprintln!(
        "{:>7} {:>8} {:>6} {:>16} {:>16} {:>9}",
        "steps", "at", "secant", "told", "a nudge says", "apart"
    );

    let mut worst = 0.0_f64;
    for depth in [1_u32, 4, 16, 64, 256, 1024] {
        board.depth = depth;
        board.tune(base, swing);
        let target = board.run(&start);

        for away in [16_u32, 64, 192] {
            let here = base + away;
            board.tune(here, swing);
            let ended = board.run(&start);
            let weight: Vec<u32> = ended
                .iter()
                .zip(&target)
                .map(|(one, other)| one.wrapping_sub(*other))
                .collect();
            board.carry(&start, &weight);
            let (told, _) = board.told();

            let step = (away / 4).max(2);
            board.tune(here + step, swing);
            let above = squared(&board.run(&start), &target);
            board.tune(here - step, swing);
            let below = squared(&board.run(&start), &target);
            let nudged = (above - below) / f64::from(2 * step);

            let apart = (told - nudged).abs() / told.abs().max(nudged.abs()).max(1.0);
            eprintln!("{depth:>7} {here:>8} {step:>6} {told:>16.1} {nudged:>16.1} {apart:>8.4}");
            worst = worst.max(apart);
            assert!(told * nudged > 0.0, "{depth} {here} {told} {nudged}");
        }
    }

    eprintln!("worst disagreement anywhere {worst:.4}");
    assert!(worst < 0.10, "{worst}");
}

#[test]
fn the_window_where_the_angle_can_be_asked_for() {
    let Some(host) = device("the_window_where_the_angle_can_be_asked_for") else {
        return;
    };
    let width = host.width();
    let grid = Grid::of(WIDE, DEEP);
    let passes = [
        built(width, grid, Pass::Ahead, Kind::Smooth).unwrap(),
        built(width, grid, Pass::Back, Kind::Smooth).unwrap(),
    ];
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let sizes = [
        8,
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
    ];
    let resident = host.resident(&sizes, &modules).unwrap();

    let base = TURNING / 64;
    let swing = SWING / 64;
    let mut board = Board {
        resident,
        over: Groups::Fixed(grid.quads().div_ceil(width)),
        grid,
        depth: 1,
        angle: (base, swing),
        loud: 1,
        planted: false,
    };

    let start = spread(grid, 0x1234_5678);
    let stranger = spread(grid, 0x0BAD_C0DE);
    let aways = [-64_i64, -16, -4, -1, 0, 1, 4, 16, 64];

    eprintln!(
        "{:>7} {:>10} {}",
        "steps",
        "saturated",
        aways
            .iter()
            .map(|one| format!("{one:>9}"))
            .collect::<String>()
    );

    let mut told = Vec::new();
    for depth in [1_u32, 4, 16, 64, 256, 1024] {
        board.depth = depth;
        board.tune(base, swing);
        let target = board.run(&start);
        let level = apart(&board.run(&stranger), &target);

        let mut walk = Vec::new();
        let mut shown = String::new();
        for away in aways {
            board.tune((i64::from(base) + away) as u32, swing);
            let loss = apart(&board.run(&start), &target);
            shown.push_str(&format!("{loss:>9.2}"));
            walk.push(loss);
        }
        eprintln!("{depth:>7} {level:>10.0} {shown}");

        let nearest = aways
            .iter()
            .zip(&walk)
            .find(|(away, _)| **away == 1)
            .map_or(0.0, |(_, loss)| *loss);
        told.push((depth, level, nearest));
    }
    board.tune(base, swing);

    for (depth, level, nearest) in &told {
        eprintln!("{depth:>5} steps, saturated {level:.0}: one unit out is worth {nearest:.2}");
    }

    let least = told.first().map_or(0.0, |(_, _, near)| *near);
    let most = told.last().map_or(0.0, |(_, _, near)| *near);
    assert!(least > 0.0, "{least}");
    assert!(most > least * 100.0, "{least} {most}");
}

#[test]
fn how_far_back_the_loss_can_still_see() {
    let Some(host) = device("how_far_back_the_loss_can_still_see") else {
        return;
    };
    let width = host.width();
    let grid = Grid::of(WIDE, DEEP);
    let passes = [
        built(width, grid, Pass::Ahead, Kind::Bound).unwrap(),
        built(width, grid, Pass::Back, Kind::Bound).unwrap(),
    ];
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let sizes = [
        8,
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
    ];
    let resident = host.resident(&sizes, &modules).unwrap();
    let mut board = Board {
        resident,
        over: Groups::Fixed(grid.quads().div_ceil(width)),
        grid,
        depth: 1,
        angle: (TURNING, SWING),
        loud: 1,
        planted: false,
    };

    let start = spread(grid, 0x1234_5678);
    let stranger = spread(grid, 0x0BAD_C0DE);

    let depths = [1_u32, 4, 16, 64, 256, 1024];
    eprintln!(
        "{:>9} {:>10} {}",
        "turning",
        "saturated",
        depths
            .iter()
            .map(|one| format!("{one:>9}"))
            .collect::<String>()
    );

    let mut reaches = Vec::new();
    for share in [64_u32, 8, 1] {
        let base = TURNING / share;
        let mut told = String::new();
        let mut reach = 0_u32;
        let mut level = 0.0_f64;
        for depth in depths {
            board.depth = depth;
            board.tune(base, SWING / share);
            let target = board.run(&start);
            level = apart(&board.run(&stranger), &target);
            board.tune(base + 8, SWING / share);
            let loss = apart(&board.run(&start), &target);
            told.push_str(&format!("{loss:>9.0}"));
            if loss < level / 2.0 {
                reach = depth;
            }
        }
        eprintln!("{base:>9} {level:>10.0} {told}");
        reaches.push((base, reach));
    }

    for (base, reach) in &reaches {
        eprintln!("at turning {base} the loss still sees a nudge {reach} steps back");
    }
    let weakest = reaches.first().copied().unwrap_or_default();
    let strongest = reaches.last().copied().unwrap_or_default();
    assert!(weakest.1 > strongest.1, "{weakest:?} {strongest:?}");
    assert!(weakest.1 >= 1024);
}

#[test]
fn whether_the_loss_can_see_the_angle_it_was_made_with() {
    let Some(host) = device("whether_the_loss_can_see_the_angle_it_was_made_with") else {
        return;
    };
    let width = host.width();
    let grid = Grid::of(WIDE, DEEP);
    let passes = [
        built(width, grid, Pass::Ahead, Kind::Bound).unwrap(),
        built(width, grid, Pass::Back, Kind::Bound).unwrap(),
    ];
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let sizes = [
        8,
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
    ];
    let resident = host.resident(&sizes, &modules).unwrap();
    let mut board = Board {
        resident,
        over: Groups::Fixed(grid.quads().div_ceil(width)),
        grid,
        depth: 32,
        angle: (TURNING, SWING),
        loud: 1,
        planted: false,
    };

    let start = spread(grid, 0x1234_5678);
    let target = board.run(&start);

    eprintln!("{:>9} {:>9} {:>14}", "turning", "away", "loss");
    let mut seen: Vec<(i64, f64)> = Vec::new();
    for away in [
        -4096_i64, -2048, -1024, -256, -64, -8, 0, 8, 64, 256, 1024, 2048, 4096,
    ] {
        let turning = (i64::from(TURNING) + away) as u32;
        board.tune(turning, SWING);
        let loss = apart(&board.run(&start), &target);
        eprintln!("{turning:>9} {away:>9} {loss:>14.1}");
        seen.push((away, loss));
    }
    board.tune(TURNING, SWING);

    let bottom = seen
        .iter()
        .min_by(|one, other| one.1.total_cmp(&other.1))
        .copied()
        .unwrap();
    let edge = seen.last().copied().unwrap();
    eprintln!(
        "lowest at {} away, {:.1}; furthest {:.1}",
        bottom.0, bottom.1, edge.1
    );
    assert_eq!(bottom.0, 0);
}

#[test]
fn whether_the_slope_points_home() {
    let Some(host) = device("whether_the_slope_points_home") else {
        return;
    };
    let width = host.width();
    let grid = Grid::of(WIDE, DEEP);
    let sizes = [
        8,
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
    ];

    for kind in [Kind::Turn, Kind::Bound] {
        let passes = [
            built(width, grid, Pass::Ahead, kind).unwrap(),
            built(width, grid, Pass::Back, kind).unwrap(),
        ];
        let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
        let named = if kind == Kind::Turn { "turn" } else { "bound" };
        let resident = host.resident(&sizes, &modules).unwrap();
        let mut board = Board {
            resident,
            over: Groups::Fixed(grid.quads().div_ceil(width)),
            grid,
            depth: 32,
            angle: (TURNING, SWING),
            loud: 1,
            planted: false,
        };

        let truth = spread(grid, 0x1234_5678);
        let target = board.run(&truth);
        let drift = spread(grid, 0x0BAD_C0DE);
        let guess: Vec<u32> = truth
            .iter()
            .zip(&drift)
            .map(|(one, other)| one.wrapping_add(other >> 2))
            .collect();

        let ended = board.run(&guess);
        let weight: Vec<u32> = ended
            .iter()
            .zip(&target)
            .map(|(one, other)| one.wrapping_sub(*other))
            .collect();
        let (slope, _) = board.carry(&guess, &weight);
        let points = leaning(&slope, &guess, &truth);

        let mut walk = Vec::new();
        for part in [0.0_f64, 0.25, 0.5, 0.75, 1.0] {
            let held = blended(&truth, &guess, part);
            walk.push(apart(&board.run(&held), &target));
        }
        eprintln!(
            "{named:>6} points home {points:+.4}   loss along the way {:?}",
            walk.iter()
                .map(|one| one.round() as i64)
                .collect::<Vec<_>>()
        );

        let near = walk[1];
        let far = walk[4];
        let level = (far - near) / far.max(1.0);
        assert!(points > 0.5, "{named} {points}");
        if kind == Kind::Turn {
            assert!(level > 0.5, "{named} {level}");
        } else {
            assert!(level < 0.1, "{named} {level}");
        }
    }
}

#[test]
fn whether_the_bounded_rule_is_stalled_or_only_slow() {
    let Some(host) = device("whether_the_bounded_rule_is_stalled_or_only_slow") else {
        return;
    };
    let width = host.width();
    let grid = Grid::of(WIDE, DEEP);
    let passes = [
        built(width, grid, Pass::Ahead, Kind::Bound).unwrap(),
        built(width, grid, Pass::Back, Kind::Bound).unwrap(),
    ];
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let sizes = [
        8,
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
    ];

    eprintln!(
        "{:>7} {:>14} {:>14} {:>9}",
        "rounds", "error first", "error last", "shrank"
    );
    let mut climbing = 0.0_f64;
    for rounds in [40_u32, 160, 640] {
        let resident = host.resident(&sizes, &modules).unwrap();
        let mut board = Board {
            resident,
            over: Groups::Fixed(grid.quads().div_ceil(width)),
            grid,
            depth: 32,
            angle: (TURNING, SWING),
            loud: 1,
            planted: false,
        };
        let (first, last, _, _) = descended(&mut board, grid, PACE, rounds);
        let shrank = if last > 0.0 { first / last } else { f64::MAX };
        eprintln!("{rounds:>7} {first:>14.1} {last:>14.1} {shrank:>8.1}x");
        climbing = climbing.max(shrank);
    }
    eprintln!("best {climbing:.1}x");
    assert!(climbing > 1.0);
}

#[test]
fn the_step_the_bounded_rule_wants_is_a_smaller_one() {
    let Some(host) = device("the_step_the_bounded_rule_wants_is_a_smaller_one") else {
        return;
    };
    let width = host.width();
    let grid = Grid::of(WIDE, DEEP);
    let passes = [
        built(width, grid, Pass::Ahead, Kind::Bound).unwrap(),
        built(width, grid, Pass::Back, Kind::Bound).unwrap(),
    ];
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let sizes = [
        8,
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
    ];

    eprintln!(
        "{:>7} {:>6} {:>14} {:>14} {:>9}",
        "steps", "pace", "error first", "error last", "shrank"
    );
    let mut best = (0_u32, 0_u32, 1.0_f64);
    for depth in [8_u32, 128] {
        for pace in [0_u32, 1, 2, 3, 5] {
            let resident = host.resident(&sizes, &modules).unwrap();
            let mut board = Board {
                resident,
                over: Groups::Fixed(grid.quads().div_ceil(width)),
                grid,
                depth,
                angle: (TURNING, SWING),
                loud: 1,
                planted: false,
            };
            let (first, last, _, _) = descended(&mut board, grid, pace, ROUNDS);
            let shrank = if last > 0.0 { first / last } else { f64::MAX };
            eprintln!("{depth:>7} {pace:>6} {first:>14.1} {last:>14.1} {shrank:>8.1}x");
            if shrank > best.2 {
                best = (depth, pace, shrank);
            }
        }
    }
    eprintln!(
        "best {:.1}x at {} steps with pace {}",
        best.2, best.0, best.1
    );
    assert!(best.2 > 1.0);
}

#[test]
fn how_deep_the_descent_still_finds_its_way() {
    let Some(host) = device("how_deep_the_descent_still_finds_its_way") else {
        return;
    };
    let width = host.width();
    let grid = Grid::of(WIDE, DEEP);
    let sizes = [
        8,
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
    ];

    eprintln!(
        "{:>9} {:>7} {:>14} {:>14} {:>14} {:>14} {:>9}",
        "rule", "steps", "state", "slope", "error first", "error last", "shrank by"
    );
    let mut reach = Vec::new();
    for kind in [Kind::Product, Kind::Sum, Kind::Turn, Kind::Bound] {
        let passes = [
            built(width, grid, Pass::Ahead, kind).unwrap(),
            built(width, grid, Pass::Back, kind).unwrap(),
        ];
        let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
        let named = match kind {
            Kind::Product => "product",
            Kind::Sum => "sum",
            Kind::Turn => "turn",
            Kind::Bound => "bound",
            Kind::Smooth => "smooth",
        };
        let mut deepest = 0_u32;
        for depth in [8_u32, 32, 128, 512] {
            let resident = host.resident(&sizes, &modules).unwrap();
            let mut board = Board {
                resident,
                over: Groups::Fixed(grid.quads().div_ceil(width)),
                grid,
                depth,
                angle: (TURNING, SWING),
                loud: 1,
                planted: false,
            };
            let (first, last, strength, reached) = descended(&mut board, grid, PACE, ROUNDS);
            let shrank = if last > 0.0 { first / last } else { f64::MAX };
            eprintln!(
                "{named:>9} {depth:>7} {reached:>14.1} {strength:>14.1} {first:>14.1} {last:>14.1} {shrank:>8.1}x"
            );
            if last < first / 10.0 {
                deepest = depth;
            }
        }
        eprintln!("{named} still arrives at {deepest} steps");
        reach.push((named, deepest));
    }
    assert!(reach.iter().any(|(_, deepest)| *deepest >= 8));
}

fn worst(one: &[u32], other: &[u32]) -> f64 {
    one.iter()
        .zip(other)
        .map(|(a, b)| f64::from(a.wrapping_sub(*b) as i32).abs())
        .fold(0.0, f64::max)
}

fn apart(one: &[u32], other: &[u32]) -> f64 {
    one.iter()
        .zip(other)
        .map(|(a, b)| f64::from(a.wrapping_sub(*b) as i32).abs())
        .sum::<f64>()
        / one.len() as f64
}

#[test]
fn the_grid_is_talked_back_to_the_start_it_came_from() {
    let Some(host) = device("the_grid_is_talked_back_to_the_start_it_came_from") else {
        return;
    };
    let width = host.width();
    let grid = Grid::of(WIDE, DEEP);
    let passes = [
        built(width, grid, Pass::Ahead, Kind::Product).unwrap(),
        built(width, grid, Pass::Back, Kind::Product).unwrap(),
    ];
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let sizes = [
        8,
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
    ];
    let resident = host.resident(&sizes, &modules).unwrap();
    let mut board = Board {
        resident,
        over: Groups::Fixed(grid.quads().div_ceil(width)),
        grid,
        depth: STEPS,
        angle: (TURNING, SWING),
        loud: 1,
        planted: false,
    };

    let truth = spread(grid, 0x1234_5678);
    let target = board.run(&truth);

    let drift = spread(grid, 0x0BAD_C0DE);
    let mut guess: Vec<u32> = truth
        .iter()
        .zip(&drift)
        .map(|(one, other)| one.wrapping_add(other >> 2))
        .collect();

    let strayed = apart(&guess, &truth);
    let first = apart(&board.run(&guess), &target);
    let mut last = first;
    let mut steps = Vec::new();
    for round in 0..ROUNDS {
        let ended = board.run(&guess);
        let weight: Vec<u32> = ended
            .iter()
            .zip(&target)
            .map(|(one, other)| one.wrapping_sub(*other))
            .collect();
        let (slope, _) = board.carry(&guess, &weight);
        if round == 0 {
            let strength: f64 = slope
                .iter()
                .map(|one| f64::from(*one as i32).abs())
                .sum::<f64>()
                / slope.len() as f64;
            let pull: f64 = weight
                .iter()
                .map(|one| f64::from(*one as i32).abs())
                .sum::<f64>()
                / weight.len() as f64;
            eprintln!("pull {pull:.1} slope {strength:.1}");
        }
        for (cell, step) in guess.iter_mut().zip(&slope) {
            let moved = i64::from(*cell) - i64::from((*step as i32) >> PACE);
            *cell = moved.clamp(0, i64::from(CEILING) - 1) as u32;
        }
        last = apart(&board.run(&guess), &target);
        if round % 8 == 0 || round + 1 == ROUNDS {
            steps.push((round, last));
        }
    }

    for (round, held) in &steps {
        eprintln!("{round} {held:.1}");
    }
    let found = apart(&guess, &truth);
    eprintln!("{first:.1} {last:.1} {strayed:.1} {found:.1}");

    assert!(last < first / 10.0, "{first} {last}");
    assert!(found < strayed / 10.0, "{strayed} {found}");
}

const WIDER: u32 = 256;

const PATCH: u32 = 12;

fn over_list(subgroup: u32, grid: Grid, pass: Pass) -> Result<Vec<u32>, LaneError> {
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

fn over_listing<const LANES: u32>(
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

struct Listed<'host> {
    resident: grit::Resident<'host>,
    ahead: Vec<Vec<Groups>>,
    back: Vec<Vec<Groups>>,
    grid: Grid,
    plan: Plan,
}

impl Listed<'_> {
    fn rouse(&mut self, cells: &[u32], weight: &[u32]) {
        self.resident.write(STATE as usize, cells).unwrap();
        self.resident.write(SLOPE as usize, weight).unwrap();
        self.resident
            .write(PARAMS as usize, &[0, self.plan.dump()])
            .unwrap();
        live::seed(&mut self.resident, self.grid, &self.plan).unwrap();
    }

    fn run(&mut self, cells: &[u32]) -> (Vec<u32>, u32) {
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

    fn carry(&mut self, ended: &[u32], weight: &[u32]) -> Vec<u32> {
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

fn patched(grid: Grid, seed: u32, span: u32) -> Vec<u32> {
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

#[test]
fn the_patch_is_talked_back_while_the_rest_of_the_grid_sleeps() {
    let Some(host) = device("the_patch_is_talked_back_while_the_rest_sleeps") else {
        return;
    };
    let width = host.width();
    let grid = Grid::of(WIDER, WIDER);
    let plan = Plan::over(width, grid.quads().div_ceil(sweep::cells(width))).unwrap();
    let keeping = live::keeping(width, &plan, false).unwrap();
    let none = Groups::Fixed(0);

    let shaped = |first: Groups, second: Groups| {
        let mut held = keeping.before.clone();
        held.push(vec![
            (over_list(width, grid, Pass::Ahead).unwrap(), first),
            (over_list(width, grid, Pass::Back).unwrap(), second),
        ]);
        held.extend(keeping.after.clone());
        held
    };
    let (passes, ahead) = live::staged(shaped(keeping.over, none));
    let (_, back) = live::staged(shaped(none, keeping.over));
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();

    let mut sizes = plan.sizes();
    sizes[SLOPE as usize] = sizes[STATE as usize];
    let resident = host.resident(&sizes, &modules).unwrap();
    let mut board = Listed {
        resident,
        ahead,
        back,
        grid,
        plan,
    };

    let truth = patched(grid, 0x1234_5678, PATCH);
    let (target, awake) = board.run(&truth);
    let stirred = worst(&target, &truth);
    let touched = target.iter().filter(|word| **word != 0).count();
    eprintln!("{} {awake} {stirred:.1} {touched}", grid.quads());

    let drift = patched(grid, 0x0BAD_C0DE, PATCH);
    let mut guess: Vec<u32> = truth
        .iter()
        .zip(&drift)
        .map(|(one, other)| one.wrapping_add(other >> 2))
        .collect();

    let strayed = worst(&guess, &truth);
    let first = worst(&board.run(&guess).0, &target);
    let mut last = first;
    for round in 0..ROUNDS {
        let (ended, _) = board.run(&guess);
        let weight: Vec<u32> = ended
            .iter()
            .zip(&target)
            .map(|(one, other)| one.wrapping_sub(*other))
            .collect();
        let slope = board.carry(&ended, &weight);
        for (cell, step) in guess.iter_mut().zip(&slope) {
            let moved = i64::from(*cell) - i64::from((*step as i32) >> PACE);
            *cell = moved.clamp(0, i64::from(CEILING) - 1) as u32;
        }
        last = worst(&board.run(&guess).0, &target);
        if round % 8 == 0 {
            eprintln!("{round} {last:.1}");
        }
    }
    let found = worst(&guess, &truth);
    eprintln!("{first:.1} {last:.1} {strayed:.1} {found:.1}");

    assert!(awake < grid.quads() / 4);
    assert!(awake > 64, "{awake}");
    assert!(stirred > 1000.0, "{stirred}");
    assert!(touched > (PATCH * PATCH) as usize, "{touched}");
    assert!(last < first / 10.0);
    assert!(found < strayed / 10.0);
}
