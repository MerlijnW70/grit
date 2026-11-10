#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::BTreeSet;

use grit::geometry::{self, Plain, Plane, Solid};
use grit::sand::{Grid, Unfit};

const BOXES: [(u32, u32, u32); 3] = [(4, 4, 4), (8, 4, 2), (16, 8, 4)];

fn solid(wide: u32, deep: u32, tall: u32) -> Grid {
    Grid::solid(wide, deep, tall).unwrap()
}

#[test]
fn a_solid_grid_rounds_every_side_and_counts_octs() {
    let grid = solid(5, 3, 3);
    assert_eq!((grid.wide, grid.deep, grid.tall), (8, 4, 4));
    assert_eq!(grid.axes(), 3);
    assert_eq!(grid.cells(), 128);
    assert_eq!(grid.blocks(), 16);
    assert_eq!(grid.quads(), 8);

    for tall in [0, 1] {
        let flat = solid(8, 4, tall);
        assert_eq!(flat, Grid::new(8, 4).unwrap());
        assert_eq!(flat.tall, 1);
        assert_eq!(flat.axes(), 2);
        assert_eq!(flat.blocks(), flat.quads());
    }
    assert_eq!(Grid::of(8, 4).tall, 1);

    assert_eq!(
        Grid::solid(1 << 16, 1 << 16, 8),
        Err(Unfit::TooTall {
            wide: 1 << 16,
            deep: 1 << 16,
            tall: 8
        })
    );
    assert!(Grid::solid(1 << 16, 1 << 16, 4).is_ok());
    assert_eq!(
        Grid::solid(4, 4, u32::MAX),
        Err(Unfit::Side { asked: u32::MAX })
    );
    assert_eq!(Grid::solid(1, 4, 4), Err(Unfit::Empty { wide: 1, deep: 4 }));
    let said = Unfit::TooTall {
        wide: 2,
        deep: 4,
        tall: 8,
    }
    .to_string();
    assert!(said.contains("2 by 4 by 8"), "{said}");
}

#[test]
fn low_bits_keeps_the_low_bits() {
    for value in [0, 1, 2, 3, 7, 8, 255, 256, 0x8000_0000, u32::MAX] {
        for bits in 0..32 {
            let kept = geometry::low_bits(&mut Plain, value, bits).unwrap();
            assert_eq!(kept, value & ((1 << bits) - 1), "{value} {bits}");
        }
    }
}

#[test]
fn every_cell_of_a_box_round_trips_through_its_point() {
    for (wide, deep, tall) in BOXES {
        let grid = solid(wide, deep, tall);
        let mut seen = BTreeSet::new();
        for z in 0..tall {
            for y in 0..deep {
                for x in 0..wide {
                    let cell = Solid::cell(grid, [x, y, z]);
                    assert_eq!(cell, x + wide * (y + deep * z), "{x} {y} {z}");
                    assert_eq!(Solid::point(grid, cell), [x, y, z]);
                    assert!(seen.insert(cell));
                }
            }
        }
        assert_eq!(seen.len(), grid.cells());
        for cell in 0..u32::try_from(grid.cells()).unwrap() {
            assert_eq!(Solid::cell(grid, Solid::point(grid, cell)), cell);
        }
    }
}

#[test]
fn hand_computed_cells_of_an_eight_by_four_by_two_box() {
    let grid = solid(8, 4, 2);
    assert_eq!(Solid::cell(grid, [3, 2, 1]), 51);
    assert_eq!(Solid::cell(grid, [7, 3, 1]), 63);
    assert_eq!(Solid::cell(grid, [0, 0, 1]), 32);
    assert_eq!(Solid::point(grid, 45), [5, 1, 1]);
    assert_eq!(Solid::extents(grid), [8, 4, 2]);
    assert_eq!(Solid::blocks(grid), [4, 2, 1]);
}

#[test]
fn every_axis_wraps_onto_the_torus() {
    let grid = solid(8, 4, 2);
    let back = u32::MAX;
    assert_eq!(Solid::cell(grid, [8, 0, 0]), 0);
    assert_eq!(Solid::cell(grid, [back, 0, 0]), 7);
    assert_eq!(Solid::cell(grid, [0, 4, 0]), 0);
    assert_eq!(Solid::cell(grid, [0, back, 0]), 24);
    assert_eq!(Solid::cell(grid, [0, 0, 2]), 0);
    assert_eq!(Solid::cell(grid, [0, 0, back]), 32);
    assert_eq!(Solid::cell(grid, [back, back, back]), 63);
    assert_eq!(Solid::cell(grid, [9, 6, 3]), 1 + 2 * 8 + 32);
}

#[test]
fn hand_computed_octs_at_both_phases() {
    let grid = solid(8, 4, 2);
    assert_eq!(Solid::cells_of(grid, 0, 0), [0, 1, 8, 9, 32, 33, 40, 41]);
    assert_eq!(Solid::cells_of(grid, 0, 1), [41, 42, 49, 50, 9, 10, 17, 18]);
    assert_eq!(
        Solid::cells_of(grid, 5, 0),
        [2, 3, 10, 11, 34, 35, 42, 43].map(|cell| cell + 16)
    );

    let cube = solid(4, 4, 4);
    assert_eq!(Solid::cells_of(cube, 7, 1), [63, 60, 51, 48, 15, 12, 3, 0]);
    assert_eq!(Solid::cells_of(cube, 7, 3), Solid::cells_of(cube, 7, 1));
    assert_eq!(Solid::cells_of(cube, 7, 2), Solid::cells_of(cube, 7, 0));
}

#[test]
fn the_octs_of_each_phase_hold_every_cell_once() {
    for (wide, deep, tall) in BOXES {
        let grid = solid(wide, deep, tall);
        assert_eq!(grid.blocks() as usize * 8, grid.cells());
        for frame in [0, 1] {
            let mut held = vec![0_u32; grid.cells()];
            for oct in 0..grid.blocks() {
                let cells = Solid::cells_of(grid, oct, frame);
                let points = cells.map(|cell| Solid::point(grid, cell));
                let first = points[0];
                for (corner, point) in points.iter().enumerate() {
                    for axis in 0..3 {
                        let step = u32::from((corner >> axis) & 1 == 1);
                        let extent = Solid::extents(grid)[axis];
                        assert_eq!(point[axis], (first[axis] + step) % extent);
                    }
                }
                for cell in cells {
                    held[cell as usize] += 1;
                }
            }
            assert!(
                held.iter().all(|&count| count == 1),
                "{wide}x{deep}x{tall} at phase {frame}"
            );
        }
    }
}

#[test]
fn the_wake_is_the_twenty_six_neighbours_and_the_oct_itself() {
    let steps = Solid::steps();
    let all: BTreeSet<[i32; 3]> = steps.into_iter().collect();
    assert_eq!(all.len(), 27);
    for dz in -1..=1 {
        for dy in -1..=1 {
            for dx in -1..=1 {
                assert!(all.contains(&[dx, dy, dz]));
            }
        }
    }
    assert_eq!(steps[0], [-1, -1, -1]);
    assert_eq!(steps[1], [0, -1, -1]);
    assert_eq!(steps[3], [-1, 0, -1]);
    assert_eq!(steps[9], [-1, -1, 0]);
    assert_eq!(steps[13], [0, 0, 0]);

    let grid = solid(8, 8, 8);
    let [across, down, up] = Solid::blocks(grid);
    for oct in 0..grid.blocks() {
        let woken = Solid::woken(grid, oct);
        let distinct: BTreeSet<u32> = woken.into_iter().collect();
        assert_eq!(distinct.len(), 27, "oct {oct}");
        assert!(distinct.contains(&oct));
        let at = [oct % across, oct / across % down, oct / (across * down)];
        for (near, step) in woken.into_iter().zip(steps) {
            let there = [near % across, near / across % down, near / (across * down)];
            for (axis, extent) in [across, down, up].into_iter().enumerate() {
                let moved = (at[axis] + extent).wrapping_add_signed(step[axis]) % extent;
                assert_eq!(there[axis], moved, "oct {oct} step {step:?}");
            }
        }
    }
    assert_eq!(Solid::woken(grid, 0)[0], 63);
}

#[test]
fn the_flat_wake_is_the_three_by_three_in_the_engine_order() {
    assert_eq!(
        Plane::steps(),
        [
            [-1, -1],
            [0, -1],
            [1, -1],
            [-1, 0],
            [0, 0],
            [1, 0],
            [-1, 1],
            [0, 1],
            [1, 1]
        ]
    );
    let grid = Grid::new(16, 8).unwrap();
    assert_eq!(Plane::woken(grid, 0), [31, 24, 25, 7, 0, 1, 15, 8, 9]);
}

#[test]
fn the_flat_quads_are_the_ones_the_reference_settles() {
    for (wide, deep) in [(2, 2), (16, 8), (8, 32)] {
        let grid = Grid::new(wide, deep).unwrap();
        for frame in [0, 1, 2, 7] {
            let phase = frame & 1;
            for quad in 0..grid.quads() {
                let column = ((quad % grid.across()) * 2 + phase) & (wide - 1);
                let row = ((quad / grid.across()) * 2 + phase) & (deep - 1);
                let right = (column + 1) & (wide - 1);
                let under = (row + 1) & (deep - 1);
                assert_eq!(
                    Plane::cells_of(grid, quad, frame),
                    [
                        row * wide + column,
                        row * wide + right,
                        under * wide + column,
                        under * wide + right
                    ],
                    "{wide}x{deep} quad {quad} frame {frame}"
                );
            }
        }
    }
}

#[test]
fn the_seats_of_a_box_cover_it_once() {
    for side in [2, 4, 8] {
        let mut seen = BTreeSet::new();
        for seat in 0..side * side * side {
            let at = Solid::seat(&mut Plain, side, seat).unwrap();
            assert_eq!(seat, at[0] + side * (at[1] + side * at[2]));
            assert!(at.iter().all(|&value| value < side));
            seen.insert(at);
        }
        assert_eq!(seen.len(), (side * side * side) as usize);
    }
    let flat = Plane::seat(&mut Plain, 32, 33).unwrap();
    assert_eq!(flat, [1, 1]);
}

#[test]
fn the_brush_box_is_centred_on_the_brush_and_wraps() {
    let grid = solid(64, 32, 16);
    let centre = [20, 9, 5];
    let side = 16;
    let mut seen = BTreeSet::new();
    for seat in 0..side * side * side {
        let point = Solid::painted(grid, side, seat, centre);
        let within = Solid::seat(&mut Plain, side, seat).unwrap();
        for axis in 0..3 {
            let extent = Solid::extents(grid)[axis];
            assert_eq!(
                point[axis],
                (centre[axis] + extent + within[axis] - side / 2) % extent
            );
        }
        seen.insert(point);
    }
    assert_eq!(seen.len(), (side * side * side) as usize);
    assert_eq!(Solid::painted(grid, side, 0, [0, 0, 0]), [56, 24, 8]);
}

fn brute_ball(reach: u32, half: i32) -> usize {
    let mut count = 0;
    for dz in -half..half {
        for dy in -half..half {
            for dx in -half..half {
                if dx * dx + dy * dy + dz * dz < reach.cast_signed() {
                    count += 1;
                }
            }
        }
    }
    count
}

#[test]
fn the_ball_holds_what_a_brute_force_count_holds() {
    let grid = solid(64, 32, 16);
    let centre = [20, 12, 8];
    let side = 16;
    let count = |reach: u32| {
        (0..side * side * side)
            .filter(|&seat| {
                let point = Solid::painted(grid, side, seat, centre);
                Solid::reached(point, centre, reach)
            })
            .count()
    };
    for (reach, by_hand) in [(0, 0), (1, 1), (2, 7), (3, 19), (4, 27), (5, 33), (6, 57)] {
        assert_eq!(count(reach), by_hand, "reach {reach}");
    }
    for reach in (0..200).step_by(7) {
        assert_eq!(count(reach), brute_ball(reach, 8), "reach {reach}");
    }
    assert_eq!(count(193), 4096);
}

#[test]
fn the_disc_holds_what_a_brute_force_count_holds() {
    let grid = Grid::new(256, 128).unwrap();
    let centre = [100, 60];
    for reach in [0, 1, 2, 3, 5, 10, 50, 100, 255, 256, 257, 600] {
        let painted = (0..32 * 32)
            .filter(|&seat| {
                let point = Plane::painted(grid, 32, seat, centre);
                Plane::reached(point, centre, reach)
            })
            .count();
        let mut brute = 0;
        for dy in -16_i32..16 {
            for dx in -16_i32..16 {
                if dx * dx + dy * dy < reach.cast_signed() {
                    brute += 1;
                }
            }
        }
        assert_eq!(painted, brute, "reach {reach}");
    }
}

#[test]
fn the_box_test_takes_distances_on_the_torus() {
    let grid = solid(64, 32, 16);
    let inside = |point: [u32; 3], centre: [u32; 3]| {
        let apart = Solid::apart(&mut Plain, point, centre).unwrap();
        Solid::boxed(&mut Plain, grid, apart, [4, 8])
            .unwrap()
            .into_iter()
            .all(|held| held)
    };
    assert!(inside([0, 0, 0], [0, 0, 0]));
    assert!(inside([60, 28, 12], [0, 0, 0]));
    assert!(!inside([59, 0, 0], [0, 0, 0]));
    assert!(inside([3, 3, 3], [0, 0, 0]));
    assert!(!inside([4, 0, 0], [0, 0, 0]));
    assert!(!inside([0, 0, 4], [0, 0, 0]));
    assert!(!inside([0, 0, 11], [0, 0, 0]));
    assert!(inside([1, 30, 15], [62, 1, 2]));
}

#[test]
fn every_cell_is_held_by_the_corner_that_names_it() {
    for (wide, deep, tall) in BOXES {
        let grid = solid(wide, deep, tall);
        for frame in [0, 1, 6, 9] {
            for cell in 0..u32::try_from(grid.cells()).unwrap() {
                let held = Solid::held(grid, cell, frame);
                assert!(held.index < grid.blocks());
                assert!(held.corner < 8);
                let [across, down, _] = Solid::blocks(grid);
                let [x, y, z] = held.block;
                assert_eq!(held.index, x + across * (y + down * z));
                assert_eq!(
                    Solid::cells_of(grid, held.index, frame)[held.corner as usize],
                    cell,
                    "{wide}x{deep}x{tall} cell {cell} frame {frame}"
                );
            }
        }
    }
    let grid = solid(8, 4, 2);
    let held = Solid::held(grid, 0, 1);
    assert_eq!((held.block, held.index, held.corner), ([3, 1, 0], 7, 7));
    let held = Solid::held(grid, 41, 1);
    assert_eq!((held.block, held.index, held.corner), ([0, 0, 0], 0, 0));
    let held = Solid::held(grid, 50, 0);
    assert_eq!((held.block, held.index, held.corner), ([1, 1, 0], 5, 4));

    let flat = Grid::new(16, 8).unwrap();
    for frame in [0, 1] {
        for cell in 0..u32::try_from(flat.cells()).unwrap() {
            let held = Plane::held(flat, cell, frame);
            assert_eq!(
                Plane::cells_of(flat, held.index, frame)[held.corner as usize],
                cell
            );
        }
    }
}
