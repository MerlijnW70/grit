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
const BINDINGS: u32 = 3;

const POINT: u32 = 16;

const PRODUCT: u32 = 20;
const SHARE: u32 = 4;

const SPREAD: [u32; 3] = [2, 3, 4];

const TURNING: u32 = 8303;

const SWING: u32 = 16384;

const STEP: u32 = 1 << 9;

const GRAIN: u32 = 2;

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
}

type Pair<const LANES: u32> = (Vector<U32, LANES>, Vector<U32, LANES>);

fn pinned<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    gate: Vector<U32, LANES>,
) -> Result<Pair<LANES>, LaneError> {
    let one = lanes.splat_bits::<U32, LANES>(1)?;
    let on = lanes.equal(gate, one)?;
    let turning = lanes.splat_bits::<U32, LANES>(TURNING)?;
    let swing = lanes.splat_bits::<U32, LANES>(SWING)?;
    let back_turning = lanes.splat_bits::<U32, LANES>(TURNING.wrapping_neg())?;
    let back_swing = lanes.splat_bits::<U32, LANES>(SWING.wrapping_neg())?;
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

fn slid<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    weight: Pair<LANES>,
    angle: Pair<LANES>,
) -> Result<Pair<LANES>, LaneError> {
    let (mut first, mut second) = weight;
    let (turning, swing) = angle;
    let step = scaled::<LANES>(lanes, second, turning)?;
    first = lanes.add(first, step)?;
    let step = scaled::<LANES>(lanes, first, swing)?;
    second = lanes.sub(second, step)?;
    let step = scaled::<LANES>(lanes, second, turning)?;
    first = lanes.add(first, step)?;
    Ok((first, second))
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
) -> Result<[Vector<U32, LANES>; 4], LaneError> {
    let [mut a, mut b, mut c, mut d] = held;
    if kind == Kind::Turn {
        return Ok([d, c, b, a]);
    }
    if kind == Kind::Bound {
        let gate = gated::<LANES>(lanes, c, d)?;
        let angle = pinned::<LANES>(lanes, gate)?;
        let (first, second) = spun::<LANES>(lanes, (a, b), angle, false)?;
        let gate = gated::<LANES>(lanes, first, second)?;
        let angle = pinned::<LANES>(lanes, gate)?;
        let (third, fourth) = spun::<LANES>(lanes, (c, d), angle, false)?;
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

type Both<const LANES: u32> = ([Vector<U32, LANES>; 4], [Vector<U32, LANES>; 4]);

fn backward<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    kind: Kind,
    held: [Vector<U32, LANES>; 4],
    weight: [Vector<U32, LANES>; 4],
) -> Result<Both<LANES>, LaneError> {
    let [mut a, mut b, mut c, mut d] = held;
    let [mut ga, mut gb, mut gc, mut gd] = weight;
    if kind == Kind::Turn {
        return Ok(([d, c, b, a], [gd, gc, gb, ga]));
    }
    if kind == Kind::Bound {
        let (first, second, third, fourth) = (d, c, b, a);
        let (gfirst, gsecond, gthird, gfourth) = (gd, gc, gb, ga);
        let gate = gated::<LANES>(lanes, first, second)?;
        let later = pinned::<LANES>(lanes, gate)?;
        let (third, fourth) = spun::<LANES>(lanes, (third, fourth), later, true)?;
        let gate = gated::<LANES>(lanes, third, fourth)?;
        let early = pinned::<LANES>(lanes, gate)?;
        let (first, second) = spun::<LANES>(lanes, (first, second), early, true)?;
        let (gthird, gfourth) = slid::<LANES>(lanes, (gthird, gfourth), later)?;
        let (gfirst, gsecond) = slid::<LANES>(lanes, (gfirst, gsecond), early)?;
        return Ok((
            [first, second, third, fourth],
            [gfirst, gsecond, gthird, gfourth],
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

    Ok(([a, b, c, d], [ga, gb, gc, gd]))
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
    let places = corners::<LANES>(&mut kernel, grid, quad, frame)?.places;

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
            None => (onward::<LANES>(&mut lanes, kind, held)?, None),
            Some(weight) => {
                let weight = [
                    lanes.splat_id::<U32, LANES>(weight[0])?,
                    lanes.splat_id::<U32, LANES>(weight[1])?,
                    lanes.splat_id::<U32, LANES>(weight[2])?,
                    lanes.splat_id::<U32, LANES>(weight[3])?,
                ];
                let (state_back, weight_back) = backward::<LANES>(&mut lanes, kind, held, weight)?;
                (state_back, Some(weight_back))
            }
        }
    };

    for (place, value) in places.iter().zip(&state_next) {
        kernel.store_at(STATE, place.id(), value.id())?;
    }
    if let Some(next) = weight_next {
        for (place, value) in places.iter().zip(&next) {
            kernel.store_at(SLOPE, place.id(), value.id())?;
        }
    }
    kernel.finish()
}

struct Board<'host> {
    resident: grit::Resident<'host>,
    over: Groups,
    grid: Grid,
    depth: u32,
}

impl Board<'_> {
    fn run(&mut self, cells: &[u32]) -> Vec<u32> {
        self.resident.write(STATE as usize, cells).unwrap();
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

    fn carry(&mut self, cells: &[u32], weight: &[u32]) -> (Vec<u32>, Vec<u32>) {
        let ended = self.run(cells);
        self.resident.write(STATE as usize, &ended).unwrap();
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
        (
            self.resident
                .read(SLOPE as usize, self.grid.cells())
                .unwrap(),
            ended,
        )
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

    let sizes = [8, grid.cells(), grid.cells()];
    let resident = host.resident(&sizes, &modules).unwrap();
    let mut board = Board {
        resident,
        over: Groups::Fixed(grid.quads().div_ceil(width)),
        grid,
        depth: STEPS,
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

    let sizes = [8, grid.cells(), grid.cells()];
    let resident = host.resident(&sizes, &modules).unwrap();
    let mut board = Board {
        resident,
        over: Groups::Fixed(grid.quads().div_ceil(width)),
        grid,
        depth: STEPS,
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
    let sizes = [8, grid.cells(), grid.cells()];

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
    let sizes = [8, grid.cells(), grid.cells()];

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
    let sizes = [8, grid.cells(), grid.cells()];

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
        };
        let mut deepest = 0_u32;
        for depth in [8_u32, 32, 128, 512] {
            let resident = host.resident(&sizes, &modules).unwrap();
            let mut board = Board {
                resident,
                over: Groups::Fixed(grid.quads().div_ceil(width)),
                grid,
                depth,
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
    let sizes = [8, grid.cells(), grid.cells()];
    let resident = host.resident(&sizes, &modules).unwrap();
    let mut board = Board {
        resident,
        over: Groups::Fixed(grid.quads().div_ceil(width)),
        grid,
        depth: STEPS,
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
            Pass::Ahead => (onward::<LANES>(&mut lanes, Kind::Product, block)?, along),
            Pass::Back => backward::<LANES>(&mut lanes, Kind::Product, block, along)?,
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
