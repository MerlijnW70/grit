#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use grit::layout::{PARAMS, STATE};
use grit::live::corners;
use grit::sand::Grid;
use grit::{Groups, Host};
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
enum Pass {
    Ahead,
    Back,
}

fn mixed<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    first: Vector<U32, LANES>,
    second: Vector<U32, LANES>,
    third: Vector<U32, LANES>,
) -> Result<Vector<U32, LANES>, LaneError> {
    let wide = lanes.splat_bits::<U32, LANES>(PRODUCT)?;
    let narrow = lanes.splat_bits::<U32, LANES>(SHARE)?;
    let held = lanes.mul(first, second)?;
    let held = lanes.shift_right_logical(held, wide)?;
    let part = lanes.shift_right_logical(third, narrow)?;
    lanes.add(held, part)
}

fn onward<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    held: [Vector<U32, LANES>; 4],
) -> Result<[Vector<U32, LANES>; 4], LaneError> {
    let [mut a, mut b, mut c, mut d] = held;
    let step = mixed::<LANES>(lanes, b, c, d)?;
    a = lanes.add(a, step)?;
    let step = mixed::<LANES>(lanes, c, d, a)?;
    b = lanes.add(b, step)?;
    let step = mixed::<LANES>(lanes, d, a, b)?;
    c = lanes.add(c, step)?;
    let step = mixed::<LANES>(lanes, a, b, c)?;
    d = lanes.add(d, step)?;
    Ok([a, b, c, d])
}

type Both<const LANES: u32> = ([Vector<U32, LANES>; 4], [Vector<U32, LANES>; 4]);

fn backward<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    held: [Vector<U32, LANES>; 4],
    weight: [Vector<U32, LANES>; 4],
) -> Result<Both<LANES>, LaneError> {
    let [mut a, mut b, mut c, mut d] = held;
    let [mut ga, mut gb, mut gc, mut gd] = weight;
    let wide = lanes.splat_bits::<U32, LANES>(PRODUCT)?;
    let narrow = lanes.splat_bits::<U32, LANES>(SHARE)?;

    let against = |lanes: &mut Lanes<'_>,
                   weight: Vector<U32, LANES>,
                   first: Vector<U32, LANES>,
                   second: Vector<U32, LANES>| {
        let held = lanes.mul(weight, second)?;
        let to_first = lanes.shift_right_arithmetic(held, wide)?;
        let held = lanes.mul(weight, first)?;
        let to_second = lanes.shift_right_arithmetic(held, wide)?;
        let to_third = lanes.shift_right_arithmetic(weight, narrow)?;
        Ok::<_, LaneError>((to_first, to_second, to_third))
    };

    let step = mixed::<LANES>(lanes, a, b, c)?;
    d = lanes.sub(d, step)?;
    let (to_a, to_b, to_c) = against(lanes, gd, a, b)?;
    ga = lanes.add(ga, to_a)?;
    gb = lanes.add(gb, to_b)?;
    gc = lanes.add(gc, to_c)?;

    let step = mixed::<LANES>(lanes, d, a, b)?;
    c = lanes.sub(c, step)?;
    let (to_d, to_a, to_b) = against(lanes, gc, d, a)?;
    gd = lanes.add(gd, to_d)?;
    ga = lanes.add(ga, to_a)?;
    gb = lanes.add(gb, to_b)?;

    let step = mixed::<LANES>(lanes, c, d, a)?;
    b = lanes.sub(b, step)?;
    let (to_c, to_d, to_a) = against(lanes, gb, c, d)?;
    gc = lanes.add(gc, to_c)?;
    gd = lanes.add(gd, to_d)?;
    ga = lanes.add(ga, to_a)?;

    let step = mixed::<LANES>(lanes, b, c, d)?;
    a = lanes.sub(a, step)?;
    let (to_b, to_c, to_d) = against(lanes, ga, b, c)?;
    gb = lanes.add(gb, to_b)?;
    gc = lanes.add(gc, to_c)?;
    gd = lanes.add(gd, to_d)?;

    Ok(([a, b, c, d], [ga, gb, gc, gd]))
}

fn built(subgroup: u32, grid: Grid, pass: Pass) -> Result<Vec<u32>, LaneError> {
    match subgroup {
        4 => building::<4>(subgroup, grid, pass),
        8 => building::<8>(subgroup, grid, pass),
        16 => building::<16>(subgroup, grid, pass),
        32 => building::<32>(subgroup, grid, pass),
        64 => building::<64>(subgroup, grid, pass),
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
            None => (onward::<LANES>(&mut lanes, held)?, None),
            Some(weight) => {
                let weight = [
                    lanes.splat_id::<U32, LANES>(weight[0])?,
                    lanes.splat_id::<U32, LANES>(weight[1])?,
                    lanes.splat_id::<U32, LANES>(weight[2])?,
                    lanes.splat_id::<U32, LANES>(weight[3])?,
                ];
                let (state_back, weight_back) = backward::<LANES>(&mut lanes, held, weight)?;
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
}

impl Board<'_> {
    fn run(&mut self, cells: &[u32]) -> Vec<u32> {
        self.resident.write(STATE as usize, cells).unwrap();
        let ahead = [self.over, Groups::Fixed(0)];
        let stages: Vec<&[Groups]> = vec![&ahead];
        for frame in 0..STEPS {
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
        for frame in (0..STEPS).rev() {
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
        built(width, grid, Pass::Ahead).unwrap(),
        built(width, grid, Pass::Back).unwrap(),
    ];
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();

    let sizes = [8, grid.cells(), grid.cells()];
    let resident = host.resident(&sizes, &modules).unwrap();
    let mut board = Board {
        resident,
        over: Groups::Fixed(grid.quads().div_ceil(width)),
        grid,
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

const ROUNDS: u32 = 40;

const CEILING: u32 = 1 << 12;

const PACE: u32 = 3;

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
        built(width, grid, Pass::Ahead).unwrap(),
        built(width, grid, Pass::Back).unwrap(),
    ];
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let sizes = [8, grid.cells(), grid.cells()];
    let resident = host.resident(&sizes, &modules).unwrap();
    let mut board = Board {
        resident,
        over: Groups::Fixed(grid.quads().div_ceil(width)),
        grid,
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
            eprintln!("gewicht {pull:.1} helling {strength:.1}");
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
