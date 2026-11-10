#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use grit::sand::{AIR, PATIENCE, SAND, WALL, WATER};
use grit::sand3::{
    CORNERS, Grid3, HELD, MINUS_X, MINUS_Z, PLUS_X, PLUS_Z, RUNNING_MINUS_Z, RUNNING_PLUS_X,
    RUNNING_PLUS_Z, TIRED, held_of, hold_of, material_of, running_of, settle_oct,
    settle_reference3, tired_of, x_first,
};

const SIZES: [(u32, u32, u32); 7] = [
    (8, 8, 8),
    (16, 16, 16),
    (32, 32, 32),
    (8, 16, 32),
    (32, 8, 4),
    (2, 32, 8),
    (16, 2, 2),
];

const SEEDS: [u64; 4] = [1, 7, 0x5EED, 0xDEAD_BEEF];

const FRAMES: u32 = 24;

fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut mixed = *state;
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    mixed ^ (mixed >> 31)
}

fn word(state: &mut u64) -> u32 {
    let roll = next(state);
    let kind = match roll % 20 {
        0..8 => AIR,
        8..12 => WATER,
        12..17 => SAND,
        _ => WALL,
    };
    if kind == WALL {
        return WALL;
    }
    let bits = ((roll >> 8) % 128) as u32;
    kind + (bits << 2)
}

fn world(grid: Grid3, seed: u64) -> Vec<u32> {
    let mut state = seed;
    (0..grid.cells()).map(|_| word(&mut state)).collect()
}

fn run(grid: Grid3, from: u32, frames: u32, cells: &[u32]) -> Vec<u32> {
    let mut now = cells.to_vec();
    for frame in from..from + frames {
        now = settle_reference3(grid, frame, &now);
    }
    now
}

fn masses(cells: &[u32]) -> [usize; 4] {
    let mut count = [0; 4];
    for &cell in cells {
        count[material_of(cell) as usize] += 1;
    }
    count
}

fn shifted(grid: Grid3, cells: &[u32], by: (u32, u32, u32)) -> Vec<u32> {
    let mut out = vec![AIR; cells.len()];
    for z in 0..grid.tall {
        for y in 0..grid.deep {
            for x in 0..grid.wide {
                out[grid.index(x + by.0, y + by.1, z + by.2)] = cells[grid.index(x, y, z)];
            }
        }
    }
    out
}

fn swapped_way(word: u32) -> u32 {
    let way = running_of(word);
    let turned = if way & 3 == 0 { way } else { way ^ 4 };
    word - (way << 2) + (turned << 2)
}

fn transposed(grid: Grid3, cells: &[u32]) -> Vec<u32> {
    assert_eq!(grid.wide, grid.tall);
    let mut out = vec![AIR; cells.len()];
    for z in 0..grid.tall {
        for y in 0..grid.deep {
            for x in 0..grid.wide {
                out[grid.index(z, y, x)] = swapped_way(cells[grid.index(x, y, z)]);
            }
        }
    }
    out
}

#[test]
fn the_word_keeps_the_2d_fields_and_widens_running_to_three_bits() {
    assert_eq!(HELD, 32);
    assert_eq!(TIRED, 64);
    assert_eq!(RUNNING_PLUS_X, WATER + (PLUS_X << 2));
    for way in [0, MINUS_X, PLUS_X, MINUS_Z, PLUS_Z] {
        let word = WATER + (way << 2) + HELD + 5 * TIRED;
        assert_eq!(material_of(word), WATER);
        assert_eq!(running_of(word), way);
        assert_eq!(held_of(word), 1);
        assert_eq!(tired_of(word), 5);
    }
    assert_eq!(hold_of(WATER, WATER), WATER + HELD);
    assert_eq!(hold_of(WATER + HELD, SAND), WATER);
    assert_eq!(
        hold_of(RUNNING_PLUS_Z + 3 * TIRED, WATER),
        RUNNING_PLUS_Z + HELD + 3 * TIRED
    );
    assert_eq!(hold_of(SAND + HELD, WATER), SAND);
}

#[test]
fn a_grid_refuses_sides_that_hold_no_oct_or_overflow_and_rounds_the_rest() {
    assert_eq!(
        Grid3::new(5, 3, 9),
        Some(Grid3 {
            wide: 8,
            deep: 4,
            tall: 16
        })
    );
    assert_eq!(Grid3::new(1, 8, 8), None);
    assert_eq!(Grid3::new(8, 0, 8), None);
    assert_eq!(Grid3::new(8, 8, u32::MAX), None);
    assert_eq!(Grid3::new(1 << 16, 1 << 16, 2), None);
    let grid = Grid3::new(1 << 16, 1 << 14, 2).unwrap();
    assert_eq!(grid.cells(), 1 << 31);
    assert_eq!(grid.octs(), 1 << 28);
}

#[test]
fn every_cell_is_touched_by_exactly_one_oct_per_frame() {
    for (wide, deep, tall) in SIZES.into_iter().chain([(2, 2, 2), (4, 2, 64)]) {
        let grid = Grid3::new(wide, deep, tall).unwrap();
        for frame in [0, 1, 2, 3, 0x8000_0001] {
            let mut touched = vec![0_u32; grid.cells()];
            for oct in 0..grid.octs() {
                for place in grid.corners(frame, oct) {
                    touched[place] += 1;
                }
            }
            assert!(
                touched.iter().all(|&count| count == 1),
                "{wide}x{deep}x{tall} frame {frame}"
            );
        }
    }
}

#[test]
fn an_oct_starts_at_twice_its_coordinates_plus_the_phase_on_all_three_axes() {
    let grid = Grid3::new(8, 4, 16).unwrap();
    let oct = 1 + 4 * (1 + 2 * 3);
    assert_eq!(
        grid.corners(0, oct),
        [
            grid.index(2, 2, 6),
            grid.index(3, 2, 6),
            grid.index(2, 2, 7),
            grid.index(3, 2, 7),
            grid.index(2, 3, 6),
            grid.index(3, 3, 6),
            grid.index(2, 3, 7),
            grid.index(3, 3, 7),
        ]
    );
    let last = grid.octs() - 1;
    assert_eq!(
        grid.corners(1, last),
        [
            grid.index(7, 3, 15),
            grid.index(0, 3, 15),
            grid.index(7, 3, 0),
            grid.index(0, 3, 0),
            grid.index(7, 0, 15),
            grid.index(0, 0, 15),
            grid.index(7, 0, 0),
            grid.index(0, 0, 0),
        ]
    );
}

#[test]
fn mass_is_conserved_per_material_on_random_worlds() {
    for (wide, deep, tall) in SIZES {
        let grid = Grid3::new(wide, deep, tall).unwrap();
        for seed in SEEDS {
            let start = world(grid, seed);
            let before = masses(&start);
            let mut now = start;
            for frame in 0..FRAMES {
                now = settle_reference3(grid, frame, &now);
                assert_eq!(
                    masses(&now),
                    before,
                    "{wide}x{deep}x{tall} seed {seed} frame {frame}"
                );
            }
        }
    }
}

#[test]
fn walls_never_move_or_change_on_random_worlds() {
    for (wide, deep, tall) in SIZES {
        let grid = Grid3::new(wide, deep, tall).unwrap();
        for seed in SEEDS {
            let start = world(grid, seed);
            let walls: Vec<usize> = (0..start.len()).filter(|&at| start[at] == WALL).collect();
            assert!(!walls.is_empty());
            let mut now = start;
            for frame in 0..FRAMES {
                now = settle_reference3(grid, frame, &now);
                let still: Vec<usize> = (0..now.len())
                    .filter(|&at| material_of(now[at]) == WALL)
                    .collect();
                assert_eq!(
                    still, walls,
                    "{wide}x{deep}x{tall} seed {seed} frame {frame}"
                );
                assert!(walls.iter().all(|&at| now[at] == WALL));
            }
        }
    }
}

#[test]
fn two_runs_from_one_state_give_one_result() {
    for (wide, deep, tall) in SIZES {
        let grid = Grid3::new(wide, deep, tall).unwrap();
        for seed in SEEDS {
            let start = world(grid, seed);
            let one = run(grid, 0, FRAMES, &start);
            let two = run(grid, 0, FRAMES, &start);
            assert_eq!(one, two);
            assert_ne!(one, start, "{wide}x{deep}x{tall} seed {seed}");
        }
    }
}

#[test]
fn a_column_of_sand_over_empty_space_falls_and_stacks() {
    for (wide, deep, tall) in [(16, 32, 16), (8, 16, 32), (32, 32, 8)] {
        let grid = Grid3::new(wide, deep, tall).unwrap();
        let floor = grid.deep - 1;
        let (x, z) = (grid.wide / 2, grid.tall / 2);
        let grains = 6;
        let mut cells = vec![AIR; grid.cells()];
        for along in 0..grid.wide {
            for across in 0..grid.tall {
                cells[grid.index(along, floor, across)] = WALL;
            }
        }
        for y in 1..=grains {
            cells[grid.index(x, y, z)] = SAND;
        }

        let highest = |cells: &[u32]| {
            (0..grid.deep)
                .find(|&y| {
                    (0..grid.tall)
                        .any(|z| (0..grid.wide).any(|x| cells[grid.index(x, y, z)] == SAND))
                })
                .unwrap()
        };
        let mut now = cells;
        let mut frame = 0;
        let mut top = highest(&now);
        assert_eq!(top, 1);
        while frame < 4 * grid.deep {
            now = settle_reference3(grid, frame, &now);
            frame += 1;
            let reached = highest(&now);
            assert!(reached >= top, "the column rose at frame {frame}");
            top = reached;
        }
        assert!(top > grains, "the column did not fall");
        assert_eq!(masses(&now)[SAND as usize], grains as usize);

        for z in 0..grid.tall {
            for y in 0..floor {
                for x in 0..grid.wide {
                    if now[grid.index(x, y, z)] == SAND {
                        assert_ne!(
                            material_of(now[grid.index(x, y + 1, z)]),
                            AIR,
                            "a grain hangs over air at {x} {y} {z}"
                        );
                    }
                }
            }
        }
        assert_eq!(now[grid.index(x, floor - 1, z)], SAND);
        assert_eq!(settle_reference3(grid, frame, &now), now);
        assert_eq!(settle_reference3(grid, frame + 1, &now), now);
    }
}

#[test]
fn a_grain_falls_along_each_diagonal_when_only_that_one_is_open() {
    let edges_x = [(0, 5), (1, 4), (2, 7), (3, 6)];
    let edges_z = [(0, 6), (1, 7), (2, 4), (3, 5)];
    let corners = [(0, 7), (1, 6), (2, 5), (3, 4)];
    for (name, pairs) in [
        ("x edge", edges_x),
        ("z edge", edges_z),
        ("corner", corners),
    ] {
        for (upper, lower) in pairs {
            for first in [true, false] {
                let mut oct = [WALL; CORNERS];
                oct[upper] = SAND;
                oct[lower] = AIR;
                let after = settle_oct(oct, first);
                let mut want = [WALL; CORNERS];
                want[upper] = AIR;
                want[lower] = SAND;
                assert_eq!(
                    after, want,
                    "{name} from {upper} to {lower}, x first {first}"
                );
            }
        }
    }
}

#[test]
fn a_grain_with_both_edges_open_takes_the_first_axis_and_never_the_corner_first() {
    for first in [true, false] {
        for upper in 0..4_usize {
            let mut oct = [AIR; CORNERS];
            oct[upper] = SAND;
            oct[upper + 4] = WALL;
            let after = settle_oct(oct, first);
            let bit = if first { 1 } else { 2 };
            let mut want = [AIR; CORNERS];
            want[upper + 4] = WALL;
            want[4 + (upper ^ bit)] = SAND;
            assert_eq!(after, want, "upper {upper} x first {first}");
        }
    }
}

#[test]
fn held_water_starts_along_the_first_open_axis_and_runs_along_it() {
    let still = WATER + HELD;
    let mut oct = [WALL; CORNERS];
    oct[0] = still;
    oct[1] = AIR;
    oct[2] = AIR;
    oct[4] = WATER;
    let x = settle_oct(oct, true);
    assert_eq!(x[1], RUNNING_PLUS_X + HELD + TIRED);
    assert_eq!(x[0], AIR);
    let z = settle_oct(oct, false);
    assert_eq!(z[2], RUNNING_PLUS_Z + HELD + TIRED);
    assert_eq!(z[0], AIR);

    oct[1] = SAND;
    let blocked = settle_oct(oct, true);
    assert_eq!(blocked[2], RUNNING_PLUS_Z + HELD + TIRED);

    oct[0] = still + PATIENCE * TIRED;
    let worn = settle_oct(oct, false);
    assert_eq!(worn[0], still + PATIENCE * TIRED);
    assert_eq!(worn[2], AIR);
}

#[test]
fn a_runner_blocked_along_its_axis_stops_and_keeps_its_held_and_tired_bits() {
    let mut oct = [AIR; CORNERS];
    oct[3] = RUNNING_MINUS_Z + HELD + 2 * TIRED;
    oct[1] = WALL;
    oct[7] = WATER;
    oct[4] = WALL;
    oct[5] = WALL;
    oct[6] = WALL;
    oct[2] = WATER;
    for first in [true, false] {
        let after = settle_oct(oct, first);
        assert_eq!(after[3], WATER + HELD + 2 * TIRED, "x first {first}");
        assert_eq!(after[2], WATER);
    }
}

#[test]
fn a_falling_cell_keeps_only_its_material_and_the_cell_it_passes_keeps_its_word() {
    let mut oct = [WALL; CORNERS];
    oct[0] = RUNNING_PLUS_X + HELD + 3 * TIRED;
    oct[4] = AIR + (MINUS_Z << 2) + HELD;
    let after = settle_oct(oct, true);
    assert_eq!(after[4], WATER);
    assert_eq!(after[0], AIR + (MINUS_Z << 2) + HELD);
}

#[test]
fn wrap_holds_on_all_three_axes() {
    for (wide, deep, tall) in SIZES {
        let grid = Grid3::new(wide, deep, tall).unwrap();
        let start = world(grid, 99);
        for by in [
            (2, 0, 0),
            (0, 2, 0),
            (0, 0, 2),
            (wide - 2, deep - 2, tall - 2),
        ] {
            for frame in 0..6 {
                let moved_first = settle_reference3(grid, frame, &shifted(grid, &start, by));
                let moved_after = shifted(grid, &settle_reference3(grid, frame, &start), by);
                assert_eq!(
                    moved_first, moved_after,
                    "{wide}x{deep}x{tall} by {by:?} frame {frame}"
                );
            }
        }
    }
}

#[test]
fn cells_cross_every_seam() {
    let grid = Grid3::new(8, 8, 8).unwrap();
    let last = 7;

    let mut fall = vec![AIR; grid.cells()];
    fall[grid.index(3, last, 3)] = SAND;
    let after = settle_reference3(grid, 1, &fall);
    assert_eq!(after[grid.index(3, 0, 3)], SAND);

    for (way, from, to) in [
        (RUNNING_PLUS_X, (last, 4, 4), (0, 4, 4)),
        (RUNNING_PLUS_Z, (4, 4, last), (4, 4, 0)),
        (WATER + (MINUS_X << 2), (0, 4, 4), (last, 4, 4)),
        (RUNNING_MINUS_Z, (4, 4, 0), (4, 4, last)),
    ] {
        let mut cells = vec![WALL; grid.cells()];
        cells[grid.index(from.0, from.1, from.2)] = way;
        cells[grid.index(to.0, to.1, to.2)] = AIR;
        let after = settle_reference3(grid, 1, &cells);
        assert_eq!(
            after[grid.index(to.0, to.1, to.2)],
            way,
            "{from:?} to {to:?}"
        );
        assert_eq!(after[grid.index(from.0, from.1, from.2)], AIR);
    }
}

#[test]
fn the_axis_order_flips_every_two_frames_so_each_phase_sees_both() {
    assert!(x_first(0));
    assert!(x_first(1));
    assert!(!x_first(2));
    assert!(!x_first(3));
    assert!(x_first(4));
    for frame in 0..64 {
        assert_ne!(x_first(frame), x_first(frame + 2));
        assert_eq!(frame & 1, (frame + 2) & 1);
    }
}

#[test]
fn swapping_x_and_z_is_the_same_as_running_two_frames_later() {
    for (side, deep) in [(8, 8), (16, 8), (32, 32), (8, 32)] {
        let grid = Grid3::new(side, deep, side).unwrap();
        for seed in SEEDS {
            let start = world(grid, seed);
            for frame in 0..8 {
                let now = run(grid, frame, FRAMES, &start);
                let swapped = run(grid, frame + 2, FRAMES, &transposed(grid, &start));
                assert_eq!(
                    swapped,
                    transposed(grid, &now),
                    "{side}x{deep} seed {seed} frame {frame}"
                );
            }
        }
    }
}
