#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use std::time::Duration;

use grit::compact::Plan;
use grit::layout::{BINDINGS, DENSE, GRAND, PARAMS, STATE};
use grit::live::{Waking, corners, wake};
use grit::sand::Grid;
use grit::{Groups, Host, live, sweep};
use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, Lanes, U32, Vector};

const WIDE: u32 = 256;
const DEEP: u32 = 256;

const STEPS: u32 = 64;

const TANGENT: u32 = BINDINGS;
const CARRIED: u32 = BINDINGS + 1;
const HOLDING: u32 = BINDINGS + 2;

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

fn spread(grid: Grid, seed: u32, fill: u32) -> Vec<u32> {
    let mut state = seed;
    (0..grid.cells())
        .map(|at| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            if (at as u32).is_multiple_of(fill) {
                state >> 8
            } else {
                0
            }
        })
        .collect()
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
            cells[(row * grid.wide + column) as usize] = state >> 8;
        }
    }
    cells
}

fn dot(one: &[u32], other: &[u32]) -> u32 {
    one.iter()
        .zip(other)
        .fold(0_u32, |sum, (a, b)| sum.wrapping_add(a.wrapping_mul(*b)))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pass {
    Tangent,
    Adjoint,
}

fn ahead<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    held: [Vector<U32, LANES>; 4],
) -> Result<[Vector<U32, LANES>; 4], LaneError> {
    let [mut a, mut b, mut c, mut d] = held;
    let step = lanes.mul(b, c)?;
    let step = lanes.add(step, d)?;
    a = lanes.add(a, step)?;

    let step = lanes.mul(c, d)?;
    let step = lanes.add(step, a)?;
    b = lanes.add(b, step)?;

    let step = lanes.mul(d, a)?;
    let step = lanes.add(step, b)?;
    c = lanes.add(c, step)?;

    let step = lanes.mul(a, b)?;
    let step = lanes.add(step, c)?;
    d = lanes.add(d, step)?;
    Ok([a, b, c, d])
}

fn alongside<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    held: [Vector<U32, LANES>; 4],
    slope: [Vector<U32, LANES>; 4],
) -> Result<[Vector<U32, LANES>; 4], LaneError> {
    let [mut a, mut b, c, d] = held;
    let [mut va, mut vb, mut vc, mut vd] = slope;

    let first = lanes.mul(vb, c)?;
    let second = lanes.mul(b, vc)?;
    let step = lanes.add(first, second)?;
    let step = lanes.add(step, vd)?;
    va = lanes.add(va, step)?;
    let step = lanes.mul(b, c)?;
    let step = lanes.add(step, d)?;
    a = lanes.add(a, step)?;

    let first = lanes.mul(vc, d)?;
    let second = lanes.mul(c, vd)?;
    let step = lanes.add(first, second)?;
    let step = lanes.add(step, va)?;
    vb = lanes.add(vb, step)?;
    let step = lanes.mul(c, d)?;
    let step = lanes.add(step, a)?;
    b = lanes.add(b, step)?;

    let first = lanes.mul(vd, a)?;
    let second = lanes.mul(d, va)?;
    let step = lanes.add(first, second)?;
    let step = lanes.add(step, vb)?;
    vc = lanes.add(vc, step)?;
    let step = lanes.mul(d, a)?;
    let step = lanes.add(step, b)?;
    let c = lanes.add(c, step)?;
    let _ = c;

    let first = lanes.mul(va, b)?;
    let second = lanes.mul(a, vb)?;
    let step = lanes.add(first, second)?;
    let step = lanes.add(step, vc)?;
    vd = lanes.add(vd, step)?;

    Ok([va, vb, vc, vd])
}

type Block<const LANES: u32> = [Vector<U32, LANES>; 4];

fn backward<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    held: Block<LANES>,
    weight: Block<LANES>,
) -> Result<(Block<LANES>, Block<LANES>), LaneError> {
    let [mut a, mut b, mut c, mut d] = held;
    let [mut ga, mut gb, mut gc, mut gd] = weight;

    let step = lanes.mul(a, b)?;
    let step = lanes.add(step, c)?;
    d = lanes.sub(d, step)?;
    let held = lanes.mul(b, gd)?;
    ga = lanes.add(ga, held)?;
    let held = lanes.mul(a, gd)?;
    gb = lanes.add(gb, held)?;
    gc = lanes.add(gc, gd)?;

    let step = lanes.mul(d, a)?;
    let step = lanes.add(step, b)?;
    c = lanes.sub(c, step)?;
    let held = lanes.mul(a, gc)?;
    gd = lanes.add(gd, held)?;
    let held = lanes.mul(d, gc)?;
    ga = lanes.add(ga, held)?;
    gb = lanes.add(gb, gc)?;

    let step = lanes.mul(c, d)?;
    let step = lanes.add(step, a)?;
    b = lanes.sub(b, step)?;
    let held = lanes.mul(d, gb)?;
    gc = lanes.add(gc, held)?;
    let held = lanes.mul(c, gb)?;
    gd = lanes.add(gd, held)?;
    ga = lanes.add(ga, gb)?;

    let step = lanes.mul(b, c)?;
    let step = lanes.add(step, d)?;
    a = lanes.sub(a, step)?;
    let held = lanes.mul(c, ga)?;
    gb = lanes.add(gb, held)?;
    let held = lanes.mul(b, ga)?;
    gc = lanes.add(gc, held)?;
    gd = lanes.add(gd, ga)?;

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
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, HOLDING))?;
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
        Pass::Tangent => Some([
            kernel.load_at(TANGENT, places[0].id())?,
            kernel.load_at(TANGENT, places[1].id())?,
            kernel.load_at(TANGENT, places[2].id())?,
            kernel.load_at(TANGENT, places[3].id())?,
        ]),
        Pass::Adjoint => Some([
            kernel.load_at(CARRIED, places[0].id())?,
            kernel.load_at(CARRIED, places[1].id())?,
            kernel.load_at(CARRIED, places[2].id())?,
            kernel.load_at(CARRIED, places[3].id())?,
        ]),
    };

    let (state_next, carried_next) = {
        let mut lanes = kernel.lanes()?;
        let held = [
            lanes.splat_id::<U32, LANES>(state[0])?,
            lanes.splat_id::<U32, LANES>(state[1])?,
            lanes.splat_id::<U32, LANES>(state[2])?,
            lanes.splat_id::<U32, LANES>(state[3])?,
        ];
        match (pass, carried) {
            (Pass::Tangent, Some(slope)) => {
                let slope = [
                    lanes.splat_id::<U32, LANES>(slope[0])?,
                    lanes.splat_id::<U32, LANES>(slope[1])?,
                    lanes.splat_id::<U32, LANES>(slope[2])?,
                    lanes.splat_id::<U32, LANES>(slope[3])?,
                ];
                let moved = alongside::<LANES>(&mut lanes, held, slope)?;
                (Some(ahead::<LANES>(&mut lanes, held)?), Some(moved))
            }
            (Pass::Adjoint, Some(weight)) => {
                let weight = [
                    lanes.splat_id::<U32, LANES>(weight[0])?,
                    lanes.splat_id::<U32, LANES>(weight[1])?,
                    lanes.splat_id::<U32, LANES>(weight[2])?,
                    lanes.splat_id::<U32, LANES>(weight[3])?,
                ];
                let (state_back, weight_back) = backward::<LANES>(&mut lanes, held, weight)?;
                (Some(state_back), Some(weight_back))
            }
            (_, None) => (None, None),
        }
    };

    if let Some(next) = state_next {
        for (place, value) in places.iter().zip(&next) {
            kernel.store_at(STATE, place.id(), value.id())?;
        }
    }
    if let Some(next) = carried_next {
        let binding = if pass == Pass::Tangent {
            TANGENT
        } else {
            CARRIED
        };
        for (place, value) in places.iter().zip(&next) {
            kernel.store_at(binding, place.id(), value.id())?;
        }
    }
    kernel.finish()
}

#[test]
fn the_slope_pushed_forward_and_the_weight_carried_back_agree() {
    let Some(host) = device("the_slope_pushed_forward_and_the_weight_carried_back_agree") else {
        return;
    };
    let width = host.width();
    let grid = Grid::of(WIDE, DEEP);
    let quads = grid.quads();

    let passes = [
        built(width, grid, Pass::Tangent).unwrap(),
        built(width, grid, Pass::Adjoint).unwrap(),
    ];
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();

    let mut sizes = [1_usize; HOLDING as usize];
    sizes[STATE as usize] = grid.cells();
    sizes[TANGENT as usize] = grid.cells();
    sizes[CARRIED as usize] = grid.cells();
    sizes[PARAMS as usize] = 8;
    let mut resident = host.resident(&sizes, &modules).unwrap();

    let start = spread(grid, 0x1234_5678, 7);
    let slope = spread(grid, 0x9ABC_DEF1, 3);
    let weight = spread(grid, 0x0FED_CBA9, 5);

    resident.write(STATE as usize, &start).unwrap();
    resident.write(TANGENT as usize, &slope).unwrap();
    resident.write(CARRIED as usize, &weight).unwrap();

    let over = Groups::Fixed(quads.div_ceil(width));
    let none = Groups::Fixed(0);
    let ahead_only = [over, none];
    let back_only = [none, over];
    let forward: Vec<&[Groups]> = vec![&ahead_only];
    let backward: Vec<&[Groups]> = vec![&back_only];

    for frame in 0..STEPS {
        resident.write_at(PARAMS as usize, 0, &[frame]).unwrap();
        resident.staged(&forward).unwrap();
    }
    let pushed = resident.read(TANGENT as usize, grid.cells()).unwrap();
    let ended = resident.read(STATE as usize, grid.cells()).unwrap();
    assert_ne!(ended, start);
    assert_ne!(pushed, slope);

    for frame in (0..STEPS).rev() {
        resident.write_at(PARAMS as usize, 0, &[frame]).unwrap();
        resident.staged(&backward).unwrap();
    }
    let carried = resident.read(CARRIED as usize, grid.cells()).unwrap();
    let home = resident.read(STATE as usize, grid.cells()).unwrap();

    let ahead_side = dot(&pushed, &weight);
    let back_side = dot(&slope, &carried);
    eprintln!("{} {STEPS} {ahead_side} {back_side}", grid.cells());

    assert_eq!(home, start);
    assert_eq!(ahead_side, back_side);
}

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
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, HOLDING))?;
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
    let places = corners::<LANES>(&mut kernel, grid, quad, frame)?.places;
    let holding = if pass == Pass::Tangent {
        TANGENT
    } else {
        CARRIED
    };

    let state = [
        kernel.load_at(STATE, places[0].id())?,
        kernel.load_at(STATE, places[1].id())?,
        kernel.load_at(STATE, places[2].id())?,
        kernel.load_at(STATE, places[3].id())?,
    ];
    let carried = [
        kernel.load_at(holding, places[0].id())?,
        kernel.load_at(holding, places[1].id())?,
        kernel.load_at(holding, places[2].id())?,
        kernel.load_at(holding, places[3].id())?,
    ];

    let (state_next, carried_next, stirring) = {
        let mut lanes = kernel.lanes()?;
        let held = [
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
        let (state_next, carried_next) = if pass == Pass::Tangent {
            let moved = alongside::<LANES>(&mut lanes, held, along)?;
            (ahead::<LANES>(&mut lanes, held)?, moved)
        } else {
            backward::<LANES>(&mut lanes, held, along)?
        };

        let one = lanes.splat_bits::<U32, LANES>(1)?;
        let none = lanes.splat_bits::<U32, LANES>(0)?;
        let mut count = none;
        for (before, after) in held
            .into_iter()
            .chain(along)
            .zip(state_next.into_iter().chain(carried_next))
        {
            let same = lanes.equal(before, after)?;
            let differs = lanes.select(same, none, one)?;
            count = lanes.add(count, differs)?;
        }
        let moving = lanes.greater_than(count, none)?;
        let stirring = lanes.select(moving, one, none)?;
        (state_next, carried_next, stirring)
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
            let held = lanes.greater_than(inside, none)?;
            lanes.select(held, *place, elsewhere)?
        };
        if let (Some(state_value), Some(carried_value)) =
            (state_next.get(corner), carried_next.get(corner))
        {
            kernel.store_at(STATE, where_at.id(), state_value.id())?;
            kernel.store_at(holding, where_at.id(), carried_value.id())?;
        }
    }

    let held = corners::<LANES>(&mut kernel, grid, quad, frame)?;
    let where_at = Waking {
        subgroup,
        grid,
        column: held.along,
        row: held.down,
    };
    wake::<LANES>(&mut kernel, where_at, stirring, inside, dump_base)?;
    kernel.finish()
}

struct Ran {
    pushed: Vec<u32>,
    carried: Vec<u32>,
    home: Vec<u32>,
    awake: u32,
    ahead: Duration,
    back: Duration,
}

const ROUNDS: u32 = 5;

type Wiring = (Vec<Vec<u32>>, Vec<Vec<Groups>>, Vec<Vec<Groups>>);

fn wiring(width: u32, grid: Grid, plan: &Plan) -> Wiring {
    let none = Groups::Fixed(0);
    let keeping = live::keeping(width, plan, true).unwrap();

    let rules = |first: Groups, second: Groups| {
        vec![
            (over_list(width, grid, Pass::Tangent).unwrap(), first),
            (over_list(width, grid, Pass::Adjoint).unwrap(), second),
        ]
    };
    let shaped = |first: Groups, second: Groups| {
        let mut stages = keeping.before.clone();
        stages.push(rules(first, second));
        stages.extend(keeping.after.clone());
        stages
    };

    let (passes, ahead) = live::staged(shaped(keeping.over, none));
    let (_, back) = live::staged(shaped(none, keeping.over));
    (passes, ahead, back)
}

fn ran_over(host: &Host, grid: Grid, held: [&[u32]; 3], sparse: bool, rounds: u32) -> Ran {
    let width = host.width();
    let quads = grid.quads();
    let plan = Plan::over(width, quads.div_ceil(sweep::cells(width))).unwrap();
    let [start, slope, weight] = held;

    let over = Groups::Fixed(quads.div_ceil(width));
    let none = Groups::Fixed(0);
    let whole_ahead = [over, none];
    let whole_back = [none, over];

    let (sparse_passes, sparse_ahead, sparse_back) = wiring(width, grid, &plan);
    let whole_passes = vec![
        built(width, grid, Pass::Tangent).unwrap(),
        built(width, grid, Pass::Adjoint).unwrap(),
    ];
    let passes = if sparse { sparse_passes } else { whole_passes };
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();

    let plan_sizes = plan.sizes();
    let mut sizes = [1_usize; HOLDING as usize];
    for (slot, words) in sizes.iter_mut().zip(plan_sizes) {
        *slot = words;
    }
    sizes[TANGENT as usize] = plan_sizes[STATE as usize];
    sizes[CARRIED as usize] = plan_sizes[STATE as usize];

    let mut resident = host.resident(&sizes, &modules).unwrap();
    for (binding, words) in sizes.iter().enumerate() {
        resident.write(binding, &vec![0_u32; *words]).unwrap();
    }
    resident.write(STATE as usize, start).unwrap();
    resident.write(TANGENT as usize, slope).unwrap();
    resident.write(CARRIED as usize, weight).unwrap();
    resident
        .write(PARAMS as usize, &[u32::MAX, plan.dump()])
        .unwrap();
    live::seed(&mut resident, grid, &plan).unwrap();

    let forward: Vec<&[Groups]> = if sparse {
        sparse_ahead.iter().map(Vec::as_slice).collect()
    } else {
        vec![&whole_ahead]
    };
    let backward: Vec<&[Groups]> = if sparse {
        sparse_back.iter().map(Vec::as_slice).collect()
    } else {
        vec![&whole_back]
    };

    if rounds > 1 {
        let warming = std::time::Instant::now();
        while warming.elapsed() < Duration::from_millis(400) {
            resident.staged(&forward).unwrap();
        }
    }

    let mut fastest_ahead = Duration::MAX;
    let mut fastest_back = Duration::MAX;
    let mut awake = quads;
    let mut pushed = Vec::new();
    for _ in 0..rounds {
        let mut spent = Duration::ZERO;
        for frame in 0..STEPS {
            resident.write_at(PARAMS as usize, 0, &[frame]).unwrap();
            spent += resident.staged(&forward).unwrap();
        }
        fastest_ahead = fastest_ahead.min(spent / STEPS);
        pushed = resident.read(TANGENT as usize, grid.cells()).unwrap();
        if sparse {
            awake = awake.min(
                resident
                    .read(GRAND as usize, 1)
                    .unwrap()
                    .first()
                    .copied()
                    .unwrap_or(0),
            );
        }

        let mut spent = Duration::ZERO;
        for frame in (0..STEPS).rev() {
            resident.write_at(PARAMS as usize, 0, &[frame]).unwrap();
            spent += resident.staged(&backward).unwrap();
        }
        fastest_back = fastest_back.min(spent / STEPS);
    }

    Ran {
        pushed,
        carried: resident.read(CARRIED as usize, grid.cells()).unwrap(),
        home: resident.read(STATE as usize, grid.cells()).unwrap(),
        awake,
        ahead: fastest_ahead,
        back: fastest_back,
    }
}

fn over_the_list(host: &Host, grid: Grid, start: &[u32], slope: &[u32], weight: &[u32]) -> Ran {
    ran_over(host, grid, [start, slope, weight], true, 1)
}

fn over_them_all(host: &Host, grid: Grid, start: &[u32], slope: &[u32], weight: &[u32]) -> Ran {
    ran_over(host, grid, [start, slope, weight], false, 1)
}

#[test]
fn the_weights_walk_back_over_the_list_the_engine_kept() {
    let Some(host) = device("the_weights_walk_back_over_the_list_the_engine_kept") else {
        return;
    };
    let grid = Grid::of(WIDE, DEEP);
    let start = patched(grid, 0x1234_5678, 12);
    let slope = patched(grid, 0x9ABC_DEF1, 12);
    let weight = patched(grid, 0x0FED_CBA9, 12);

    let ran = over_the_list(&host, grid, &start, &slope, &weight);
    let ahead_side = dot(&ran.pushed, &weight);
    let back_side = dot(&slope, &ran.carried);
    eprintln!(
        "{} {STEPS} {} {ahead_side} {back_side}",
        grid.quads(),
        ran.awake
    );

    assert_ne!(ran.pushed, slope);
    assert!(ran.awake < grid.quads());
    assert_eq!(ran.home, start);
    assert_eq!(ahead_side, back_side);
}

#[test]
fn the_list_carries_the_weights_the_whole_grid_would_have() {
    let Some(host) = device("the_list_carries_the_weights_the_whole_grid_would_have") else {
        return;
    };
    let grid = Grid::of(WIDE, DEEP);
    let start = patched(grid, 0x1234_5678, 12);
    let slope = patched(grid, 0x9ABC_DEF1, 12);
    let weight = patched(grid, 0x0FED_CBA9, 12);

    let listed = over_the_list(&host, grid, &start, &slope, &weight);
    let whole = over_them_all(&host, grid, &start, &slope, &weight);

    let slopes = listed
        .pushed
        .iter()
        .zip(&whole.pushed)
        .filter(|(one, other)| one != other)
        .count();
    let weights = listed
        .carried
        .iter()
        .zip(&whole.carried)
        .filter(|(one, other)| one != other)
        .count();
    eprintln!(
        "{} {STEPS} {} {slopes} {weights}",
        grid.quads(),
        listed.awake
    );

    assert_ne!(whole.pushed, slope);
    assert_eq!(slopes, 0);
    assert_eq!(weights, 0);
    assert_eq!(listed.home, start);
    assert_eq!(whole.home, start);
}

#[test]
fn the_list_starts_paying_when_the_grid_outgrows_the_bookkeeping() {
    let Some(host) = device("the_list_starts_paying_when_the_grid_outgrows") else {
        return;
    };
    let mut won = false;
    for side in [512_u32, 1024, 2048, 4096] {
        let grid = Grid::of(side, side);
        let cells = patched(grid, 0x1234_5678, 12);
        let held = [cells.as_slice(), cells.as_slice(), cells.as_slice()];

        let whole = ran_over(&host, grid, held, false, ROUNDS);
        let listed = ran_over(&host, grid, held, true, ROUNDS);
        eprintln!(
            "{} {} {:.3?} {:.3?} {:.3?} {:.3?}",
            grid.quads(),
            listed.awake,
            whole.ahead,
            whole.back,
            listed.ahead,
            listed.back
        );
        assert!(listed.awake < grid.quads() || side == 512);
        if listed.ahead < whole.ahead && listed.back < whole.back {
            won = true;
        }
    }
    assert!(won);
}

const WIDEST: u32 = 1024;

const NAMED: [&str; 9] = [
    "decay wipe counts",
    "the rule",
    "mark coarse",
    "scan coarse",
    "gather coarse",
    "mark fine",
    "scan fine counts",
    "scan fine totals",
    "gather fine",
];

#[test]
fn what_a_listed_frame_spends_and_where() {
    let Some(host) = device("what_a_listed_frame_spends_and_where") else {
        return;
    };
    let width = host.width();
    let grid = Grid::of(WIDEST, WIDEST);
    let quads = grid.quads();
    let plan = Plan::over(width, quads.div_ceil(sweep::cells(width))).unwrap();
    let cells = patched(grid, 0x1234_5678, 12);

    let (passes, ahead, _) = wiring(width, grid, &plan);
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let plan_sizes = plan.sizes();
    let mut sizes = [1_usize; HOLDING as usize];
    for (slot, words) in sizes.iter_mut().zip(plan_sizes) {
        *slot = words;
    }
    sizes[TANGENT as usize] = plan_sizes[STATE as usize];
    sizes[CARRIED as usize] = plan_sizes[STATE as usize];

    let mut resident = host.resident(&sizes, &modules).unwrap();
    for (binding, words) in sizes.iter().enumerate() {
        resident.write(binding, &vec![0_u32; *words]).unwrap();
    }
    resident.write(STATE as usize, &cells).unwrap();
    resident.write(TANGENT as usize, &cells).unwrap();
    resident.write(CARRIED as usize, &cells).unwrap();
    resident
        .write(PARAMS as usize, &[u32::MAX, plan.dump()])
        .unwrap();
    live::seed(&mut resident, grid, &plan).unwrap();

    let forward: Vec<&[Groups]> = ahead.iter().map(Vec::as_slice).collect();
    let warming = std::time::Instant::now();
    while warming.elapsed() < Duration::from_millis(400) {
        resident.staged(&forward).unwrap();
    }

    let mut best = vec![Duration::MAX; NAMED.len()];
    let mut whole = Duration::MAX;
    for _ in 0..ROUNDS {
        for frame in 0..STEPS {
            resident.write_at(PARAMS as usize, 0, &[frame]).unwrap();
            resident.staged(&forward).unwrap();
        }
        let spans = resident.stage_times().to_vec();
        if spans.len() != NAMED.len() {
            eprintln!("SKIPPED {} {}", spans.len(), NAMED.len());
            return;
        }
        for (slot, span) in best.iter_mut().zip(&spans) {
            *slot = (*slot).min(*span);
        }
        whole = whole.min(resident.on_device().unwrap_or(Duration::MAX));
    }

    let counted: Duration = best.iter().sum();
    for (name, span) in NAMED.iter().zip(&best) {
        let share = span.as_secs_f64() / counted.as_secs_f64() * 100.0;
        eprintln!("{name:<20} {span:>10.3?} {share:>5.1}%");
    }
    eprintln!("{counted:.3?} {whole:.3?} {}", resident.stage_times().len());

    assert!(counted <= whole);
    assert!(best.iter().all(|span| *span > Duration::ZERO));
}
