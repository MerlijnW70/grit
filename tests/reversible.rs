#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use grit::layout::{BINDINGS, PARAMS, STATE};
use grit::sand::{self, Grid};
use grit::{Groups, Host};
use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, U32};

const WIDE: u32 = 1024;
const DEEP: u32 = 1024;

const STEPS: u32 = 1024;

const TWIST: [u32; 4] = [0x9E37_79B9, 0x85EB_CA6B, 0xC2B2_AE35, 0x27D4_EB2F];

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

fn turned(subgroup: u32, grid: Grid, back: bool) -> Result<Vec<u32>, LaneError> {
    match subgroup {
        4 => turning::<4>(subgroup, grid, back),
        8 => turning::<8>(subgroup, grid, back),
        16 => turning::<16>(subgroup, grid, back),
        32 => turning::<32>(subgroup, grid, back),
        64 => turning::<64>(subgroup, grid, back),
        other => Err(LaneError::NoMapping {
            lanes: other,
            width: subgroup,
        }),
    }
}

fn turning<const LANES: u32>(subgroup: u32, grid: Grid, back: bool) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.index_type();

    let group = kernel.workgroup_index();
    let lane = kernel.local_index();
    let width = kernel.module().constant_u32(subgroup)?;
    let base = kernel.module().i_mul(uint, group, width)?;
    let quad = kernel.module().i_add(uint, base, lane)?;

    let head = kernel.module().constant_u32(0)?;
    let frame = kernel.load_at(PARAMS, head)?;

    let places = {
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

        [
            sand::at::<LANES>(&mut lanes, grid, column, row)?,
            sand::at::<LANES>(&mut lanes, grid, right, row)?,
            sand::at::<LANES>(&mut lanes, grid, column, under)?,
            sand::at::<LANES>(&mut lanes, grid, right, under)?,
        ]
    };

    let held = [
        kernel.load_at(STATE, places[0].id())?,
        kernel.load_at(STATE, places[1].id())?,
        kernel.load_at(STATE, places[2].id())?,
        kernel.load_at(STATE, places[3].id())?,
    ];

    let turned = {
        let mut lanes = kernel.lanes()?;
        let held = [
            lanes.splat_id::<U32, LANES>(held[0])?,
            lanes.splat_id::<U32, LANES>(held[1])?,
            lanes.splat_id::<U32, LANES>(held[2])?,
            lanes.splat_id::<U32, LANES>(held[3])?,
        ];
        let taken = if back {
            [held[1], held[3], held[0], held[2]]
        } else {
            [held[2], held[0], held[3], held[1]]
        };
        let mut out = Vec::with_capacity(TWIST.len());
        for (value, step) in taken.into_iter().zip(TWIST) {
            let by = lanes.splat_bits::<U32, LANES>(step)?;
            out.push(if back {
                lanes.sub(value, by)?
            } else {
                lanes.add(value, by)?
            });
        }
        out
    };

    for (place, value) in places.iter().zip(&turned) {
        kernel.store_at(STATE, place.id(), value.id())?;
    }
    kernel.finish()
}

#[must_use]
fn scattered(grid: Grid, seed: u32) -> Vec<u32> {
    let mut state = seed;
    (0..grid.cells())
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state
        })
        .collect()
}

#[must_use]
fn turn_reference(grid: Grid, frame: u32, back: bool, cells: &[u32]) -> Vec<u32> {
    let mut out = cells.to_vec();
    let phase = frame & 1;
    for down in 0..grid.down() {
        for along in 0..grid.across() {
            let column = along * 2 + phase;
            let row = down * 2 + phase;
            let right = (column + 1) & (grid.wide - 1);
            let under = (row + 1) & (grid.deep - 1);
            let places = [
                (row * grid.wide + column) as usize,
                (row * grid.wide + right) as usize,
                (under * grid.wide + column) as usize,
                (under * grid.wide + right) as usize,
            ];
            let held = places.map(|at| cells.get(at).copied().unwrap_or(0));
            let taken = if back {
                [held[1], held[3], held[0], held[2]]
            } else {
                [held[2], held[0], held[3], held[1]]
            };
            let mut turned = [0_u32; 4];
            for ((slot, value), step) in turned.iter_mut().zip(taken).zip(TWIST) {
                *slot = if back {
                    value.wrapping_sub(step)
                } else {
                    value.wrapping_add(step)
                };
            }
            for (at, value) in places.into_iter().zip(turned) {
                if let Some(slot) = out.get_mut(at) {
                    *slot = value;
                }
            }
        }
    }
    out
}

#[test]
fn a_block_turned_back_is_the_block_it_was() {
    let grid = Grid::of(8, 8);
    let cells = scattered(grid, 0x1234_5678);
    let mut held = cells.clone();
    for frame in 0..16 {
        held = turn_reference(grid, frame, false, &held);
    }
    assert_ne!(held, cells);
    for frame in (0..16).rev() {
        held = turn_reference(grid, frame, true, &held);
    }
    assert_eq!(held, cells);
}

#[test]
fn a_thousand_steps_forward_and_back_is_the_grid_it_started_as() {
    let Some(host) = device("a_thousand_steps_forward_and_back") else {
        return;
    };
    let width = host.width();
    let grid = Grid::of(WIDE, DEEP);
    let quads = grid.quads();

    let ahead = turned(width, grid, false).unwrap();
    let back = turned(width, grid, true).unwrap();
    let modules: Vec<&[u32]> = vec![&ahead, &back];

    let mut sizes = [1_usize; BINDINGS as usize];
    sizes[STATE as usize] = grid.cells();
    sizes[PARAMS as usize] = 8;
    let mut resident = host.resident(&sizes, &modules).unwrap();

    let cells = scattered(grid, 0x5EED_1234);
    resident.write(STATE as usize, &cells).unwrap();

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
    let stirred = resident.read(STATE as usize, grid.cells()).unwrap();
    assert_ne!(stirred, cells);

    for frame in (0..STEPS).rev() {
        resident.write_at(PARAMS as usize, 0, &[frame]).unwrap();
        resident.staged(&backward).unwrap();
    }
    let home = resident.read(STATE as usize, grid.cells()).unwrap();

    let apart = home
        .iter()
        .zip(&cells)
        .filter(|(one, other)| one != other)
        .count();
    eprintln!("{} {} {apart}", grid.cells(), STEPS);
    assert_eq!(apart, 0);
}

#[test]
fn the_device_turns_the_block_the_reference_turns() {
    let Some(host) = device("the_device_turns_the_block_the_reference_turns") else {
        return;
    };
    let width = host.width();
    let grid = Grid::of(64, 64);
    let quads = grid.quads();

    let ahead = turned(width, grid, false).unwrap();
    let back = turned(width, grid, true).unwrap();
    let modules: Vec<&[u32]> = vec![&ahead, &back];

    let mut sizes = [1_usize; BINDINGS as usize];
    sizes[STATE as usize] = grid.cells();
    sizes[PARAMS as usize] = 8;
    let mut resident = host.resident(&sizes, &modules).unwrap();

    let cells = scattered(grid, 0x0BAD_F00D);
    resident.write(STATE as usize, &cells).unwrap();

    let over = Groups::Fixed(quads.div_ceil(width));
    let none = Groups::Fixed(0);
    let ahead_only = [over, none];
    let forward: Vec<&[Groups]> = vec![&ahead_only];

    let mut expected = cells;
    for frame in 0..8 {
        resident.write_at(PARAMS as usize, 0, &[frame]).unwrap();
        resident.staged(&forward).unwrap();
        expected = turn_reference(grid, frame, false, &expected);
        let seen = resident.read(STATE as usize, grid.cells()).unwrap();
        assert_eq!(seen, expected, "frame {frame}");
    }
}
