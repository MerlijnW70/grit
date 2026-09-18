#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use grit::layout::{BINDINGS, PARAMS, STATE};
use grit::sand::{self, Grid};
use grit::{Groups, Host};
use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, Lanes, U32, Vector};
use simdr::module::Id;

const WIDE: u32 = 256;
const DEEP: u32 = 256;

const STEPS: u32 = 64;

const TANGENT: u32 = 2;
const CARRIED: u32 = 3;

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

fn corners<const LANES: u32>(
    kernel: &mut Kernel<U32>,
    grid: Grid,
    quad: Id,
    frame: Id,
) -> Result<[Vector<U32, LANES>; 4], LaneError> {
    let mut lanes = kernel.lanes()?;
    let which = lanes.splat_id::<U32, LANES>(quad)?;
    let now = lanes.splat_id::<U32, LANES>(frame)?;

    let phase = sand::low_bits::<LANES>(&mut lanes, now, 1)?;
    let by = lanes.splat_bits::<U32, LANES>(grid.across().trailing_zeros())?;
    let down = lanes.shift_right_logical(which, by)?;
    let along = sand::low_bits::<LANES>(&mut lanes, which, grid.across().trailing_zeros())?;

    let one = lanes.splat_bits::<U32, LANES>(1)?;
    let doubled_x = lanes.shift_left(along, one)?;
    let doubled_y = lanes.shift_left(down, one)?;
    let column = lanes.add(doubled_x, phase)?;
    let row = lanes.add(doubled_y, phase)?;
    let right = lanes.add(column, one)?;
    let under = lanes.add(row, one)?;

    Ok([
        sand::at::<LANES>(&mut lanes, grid, column, row)?,
        sand::at::<LANES>(&mut lanes, grid, right, row)?,
        sand::at::<LANES>(&mut lanes, grid, column, under)?,
        sand::at::<LANES>(&mut lanes, grid, right, under)?,
    ])
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
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.index_type();
    let group = kernel.workgroup_index();
    let lane = kernel.local_index();
    let width = kernel.module().constant_u32(subgroup)?;
    let base = kernel.module().i_mul(uint, group, width)?;
    let quad = kernel.module().i_add(uint, base, lane)?;

    let head = kernel.module().constant_u32(0)?;
    let frame = kernel.load_at(PARAMS, head)?;
    let places = corners::<LANES>(&mut kernel, grid, quad, frame)?;

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

    let mut sizes = [1_usize; BINDINGS as usize];
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
