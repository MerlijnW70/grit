#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use grit::rule3::heat::{
    AIR, ASH, BOIL, CONDENSE, FIRE, FUEL, Heat, ICE, KINDS, LIFE, MELT, STEAM, TOP, TUNED, WALL,
    WATER, WINDOW, cell, conducts, diffused, kind_of, life_of, moved, reacted, temperature_of,
};
use grit::rule3::{CORNERS, Oct, Rule, noise};
use grit::sand3::{Grid3, reference};

const SEEDS: [u64; 4] = [1, 7, 0x5EED, 0xDEAD_BEEF];

fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut mixed = *state;
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    mixed ^ (mixed >> 31)
}

fn run(grid: Grid3, start: &[u32], first: u32, frames: u32) -> Vec<u32> {
    let mut cells = start.to_vec();
    for frame in first..first + frames {
        cells = reference(&Heat, grid, frame, &cells, &TUNED);
    }
    cells
}

fn heat_of(cells: &[u32], keep: impl Fn(usize) -> bool) -> u64 {
    cells
        .iter()
        .enumerate()
        .filter(|&(at, &word)| keep(at) && conducts(kind_of(word)))
        .map(|(_, &word)| u64::from(temperature_of(word)))
        .sum()
}

fn count(cells: &[u32], kind: u32) -> usize {
    cells.iter().filter(|&&word| kind_of(word) == kind).count()
}

/// Every kind but fuel and fire, at temperatures from 0 to 599, so nothing
/// burns: the kinds that conduct and move, walls, and inert kinds.
fn unburning(state: &mut u64) -> u32 {
    let roll = next(state);
    let kind = [
        AIR,
        AIR,
        AIR,
        WALL,
        ICE,
        WATER,
        WATER,
        STEAM,
        ASH,
        KINDS + 3,
    ][(roll % 10) as usize];
    let temperature = ((roll >> 8) % 600) as u32;
    cell(kind, temperature) | (((roll >> 32) as u32 & 0xFF) << 8)
}

#[test]
fn heat_is_conserved_on_a_torus_with_no_fire() {
    for (wide, deep, tall) in [(16, 16, 16), (8, 32, 4)] {
        let grid = Grid3::new(wide, deep, tall).unwrap();
        for seed in SEEDS {
            let mut state = seed;
            let start: Vec<u32> = (0..grid.cells()).map(|_| unburning(&mut state)).collect();
            let total = heat_of(&start, |_| true);
            let mut cells = start.clone();
            for frame in 0..256 {
                cells = reference(&Heat, grid, frame, &cells, &TUNED);
                assert_eq!(
                    heat_of(&cells, |_| true),
                    total,
                    "seed {seed:#x} frame {frame}"
                );
            }
            assert_ne!(cells, start);
        }
    }
}

/// A closed box of walls, its inside drawn at random, inside a world of air
/// at another temperature.
fn boxed(grid: Grid3, seed: u64, outside: u32) -> (Vec<u32>, Vec<bool>) {
    let mut state = seed;
    let mut cells = vec![cell(AIR, outside); grid.cells()];
    let mut inside = vec![false; grid.cells()];
    for z in 0..grid.tall {
        for y in 0..grid.deep {
            for x in 0..grid.wide {
                let at = grid.index(x, y, z);
                let low = |value: u32| value == 3 || value == 12;
                let within = |value: u32| (3..=12).contains(&value);
                if !(within(x) && within(y) && within(z)) {
                    continue;
                }
                if low(x) || low(y) || low(z) {
                    cells[at] = cell(WALL, 0);
                } else {
                    cells[at] = unburning(&mut state);
                    inside[at] = true;
                }
            }
        }
    }
    (cells, inside)
}

#[test]
fn heat_is_conserved_inside_a_closed_box_of_walls() {
    let grid = Grid3::new(16, 16, 16).unwrap();
    for seed in SEEDS {
        for outside in [0, 300, 5000] {
            let (start, inside) = boxed(grid, seed, outside);
            let within = heat_of(&start, |at| inside[at]);
            let without = heat_of(&start, |at| !inside[at]);
            let mut cells = start.clone();
            for frame in 0..256 {
                cells = reference(&Heat, grid, frame, &cells, &TUNED);
                assert_eq!(heat_of(&cells, |at| inside[at]), within, "frame {frame}");
                assert_eq!(heat_of(&cells, |at| !inside[at]), without, "frame {frame}");
                for (at, &word) in cells.iter().enumerate() {
                    if !inside[at] && kind_of(start[at]) == WALL {
                        assert_eq!(word, start[at]);
                    }
                }
            }
        }
    }
}

#[test]
fn an_isolated_hot_spot_spreads_and_flattens() {
    for (wide, deep, tall) in [(16, 16, 16), (32, 8, 16)] {
        let grid = Grid3::new(wide, deep, tall).unwrap();
        let mut cells = vec![cell(AIR, 300); grid.cells()];
        let spot = grid.index(wide / 2 + 1, deep / 2 + 1, tall / 2 + 1);
        cells[spot] = cell(AIR, 40_300);
        let total = heat_of(&cells, |_| true);
        let mut warm = 1;
        let mut highest = 40_300;
        let mut lowest = 300;
        let mut frame = 0;
        while highest > lowest + 1 {
            assert!(
                frame < 4096,
                "{wide}x{deep}x{tall}: still {lowest} to {highest}"
            );
            cells = reference(&Heat, grid, frame, &cells, &TUNED);
            let now_high = cells
                .iter()
                .map(|&word| temperature_of(word))
                .max()
                .unwrap();
            let now_low = cells
                .iter()
                .map(|&word| temperature_of(word))
                .min()
                .unwrap();
            assert!(now_high <= highest && now_low >= lowest, "frame {frame}");
            let now_warm = cells
                .iter()
                .filter(|&&word| temperature_of(word) > 300)
                .count();
            if frame < 16 {
                assert!(now_warm > warm, "frame {frame}: {now_warm} warm cells");
            }
            (highest, lowest, warm) = (now_high, now_low, now_warm);
            frame += 1;
        }
        assert_eq!(heat_of(&cells, |_| true), total);
        let even = u32::try_from(total / grid.cells() as u64).unwrap();
        assert!(lowest == even, "{lowest} {even}");
        println!("{wide}x{deep}x{tall}: flat at {lowest} to {highest} after {frame} frames");
    }
}

/// A room of air at `air`, with walls on the low x, low z and high y sides
/// (the floor), which the torus sets next to the other sides.
fn room(grid: Grid3, air: u32) -> Vec<u32> {
    let mut cells = vec![cell(AIR, air); grid.cells()];
    for z in 0..grid.tall {
        for y in 0..grid.deep {
            for x in 0..grid.wide {
                if y + 1 == grid.deep || x == 0 || z == 0 {
                    cells[grid.index(x, y, z)] = cell(WALL, 0);
                }
            }
        }
    }
    cells
}

#[test]
fn ice_in_a_warm_room_melts_and_the_water_stays_water() {
    let grid = Grid3::new(16, 16, 16).unwrap();
    let mut cells = room(grid, 330);
    let mut ice = 0;
    for z in 6..10 {
        for y in 4..8 {
            for x in 6..10 {
                cells[grid.index(x, y, z)] = cell(ICE, 200);
                ice += 1;
            }
        }
    }
    let total = heat_of(&cells, |_| true);
    let mut frame = 0;
    while count(&cells, ICE) > 0 {
        assert!(frame < 2048, "ice left: {}", count(&cells, ICE));
        cells = reference(&Heat, grid, frame, &cells, &TUNED);
        assert_eq!(count(&cells, ICE) + count(&cells, WATER), ice);
        frame += 1;
    }
    assert_eq!(heat_of(&cells, |_| true), total);
    let after = run(grid, &cells, frame, 256);
    assert_eq!(count(&after, WATER), ice);
    for (at, &word) in after.iter().enumerate() {
        if kind_of(word) == WATER {
            let below = at + (grid.wide as usize);
            assert!(matches!(kind_of(after[below % after.len()]), WATER | WALL));
        }
    }
    println!("{ice} ice cells melted in {frame} frames");
}

#[test]
fn a_fire_in_a_block_of_fuel_burns_out_and_leaves_ash() {
    let grid = Grid3::new(16, 16, 16).unwrap();
    for seed in SEEDS {
        let mut cells = room(grid, 300);
        let mut fuel = 0;
        for z in 5..11 {
            for y in 8..14 {
                for x in 5..11 {
                    cells[grid.index(x, y, z)] = cell(FUEL, 300);
                    fuel += 1;
                }
            }
        }
        let first = (seed % 8) as u32;
        let lit = grid.index(7 + first % 2, 10, 8);
        cells[lit] = cell(FIRE, 900) | (LIFE << 4);
        let mut frame = first;
        let mut burned = false;
        while count(&cells, FIRE) > 0 {
            assert!(frame < 4096, "seed {seed:#x}: still burning");
            cells = reference(&Heat, grid, frame, &cells, &TUNED);
            burned |= count(&cells, FIRE) > 1;
            assert_eq!(
                count(&cells, FUEL) + count(&cells, FIRE) + count(&cells, ASH),
                fuel
            );
            frame += 1;
        }
        assert!(burned, "seed {seed:#x}: the fire never spread");
        assert_eq!(count(&cells, ASH), fuel, "seed {seed:#x}");
        assert_eq!(count(&cells, FUEL), 0, "seed {seed:#x}");
        println!("seed {seed:#x}: {fuel} fuel cells burned to ash by frame {frame}");
    }
}

#[test]
fn heat_crosses_every_seam_of_the_torus() {
    for (wide, deep, tall) in [(16, 16, 16), (8, 4, 32)] {
        let grid = Grid3::new(wide, deep, tall).unwrap();
        let mut cells = vec![cell(AIR, 0); grid.cells()];
        cells[grid.index(0, 0, 0)] = cell(AIR, 64_000);
        let mut reached = [false; 3];
        for frame in 0..8 {
            cells = reference(&Heat, grid, frame, &cells, &TUNED);
            let warm = |x, y, z| temperature_of(cells[grid.index(x, y, z)]) > 0;
            reached[0] |= warm(wide - 1, 0, 0);
            reached[1] |= warm(0, deep - 1, 0);
            reached[2] |= warm(0, 0, tall - 1);
        }
        assert_eq!(reached, [true; 3], "{wide}x{deep}x{tall}");
    }
}

fn shifted(grid: Grid3, cells: &[u32], by: [u32; 3]) -> Vec<u32> {
    let mut out = vec![0; cells.len()];
    for z in 0..grid.tall {
        for y in 0..grid.deep {
            for x in 0..grid.wide {
                out[grid.index(x + by[0], y + by[1], z + by[2])] = cells[grid.index(x, y, z)];
            }
        }
    }
    out
}

#[test]
fn a_world_moved_by_two_cells_along_any_axis_wraps_to_the_same_world_moved() {
    let grid = Grid3::new(16, 8, 32).unwrap();
    for seed in SEEDS {
        let mut state = seed;
        let start: Vec<u32> = (0..grid.cells()).map(|_| unburning(&mut state)).collect();
        let end = run(grid, &start, 0, 64);
        for by in [[2, 0, 0], [0, 2, 0], [0, 0, 2], [14, 6, 30], [6, 2, 18]] {
            let moved_end = run(grid, &shifted(grid, &start, by), 0, 64);
            assert_eq!(
                moved_end,
                shifted(grid, &end, by),
                "seed {seed:#x} by {by:?}"
            );
        }
    }
}

/// A world with every kind, fires and fuel past ignition among them, so the
/// noise decides.
fn burning(grid: Grid3, seed: u64) -> Vec<u32> {
    let mut state = seed;
    (0..grid.cells())
        .map(|_| {
            let roll = next(&mut state);
            let kind = (roll % u64::from(KINDS)) as u32;
            let temperature = ((roll >> 8) % 1200) as u32;
            cell(kind, temperature) | (((roll >> 32) as u32 & 0xF) << 4)
        })
        .collect()
}

#[test]
fn two_runs_from_one_state_give_one_result() {
    let grid = Grid3::new(16, 16, 16).unwrap();
    for seed in SEEDS {
        let start = burning(grid, seed);
        let once = run(grid, &start, 0, 64);
        let again = run(grid, &start, 0, 64);
        assert_eq!(once, again);
        let later = run(grid, &start, 2, 64);
        assert_ne!(
            once, later,
            "seed {seed:#x}: the noise did not depend on the frame"
        );
    }
}

#[test]
fn each_reaction_happens_at_its_threshold() {
    let params = TUNED;
    let [ignite, chance, flare] = params;
    assert_eq!(
        reacted(cell(ICE, MELT - 1), 0, &params),
        cell(ICE, MELT - 1)
    );
    assert_eq!(reacted(cell(ICE, MELT), 0, &params), cell(WATER, MELT));
    assert_eq!(
        reacted(cell(WATER, BOIL - 1), 0, &params),
        cell(WATER, BOIL - 1)
    );
    assert_eq!(reacted(cell(WATER, BOIL), 0, &params), cell(STEAM, BOIL));
    assert_eq!(
        reacted(cell(STEAM, CONDENSE), 0, &params),
        cell(STEAM, CONDENSE)
    );
    assert_eq!(
        reacted(cell(STEAM, CONDENSE - 1), 0, &params),
        cell(WATER, CONDENSE - 1)
    );
    assert_eq!(
        reacted(cell(FUEL, ignite - 1), 0, &params),
        cell(FUEL, ignite - 1)
    );
    assert_eq!(
        reacted(cell(FUEL, ignite), chance - 1, &params),
        cell(FIRE, ignite) | (LIFE << 4)
    );
    assert_eq!(
        reacted(cell(FUEL, ignite), chance, &params),
        cell(FUEL, ignite + 1)
    );
    assert_eq!(
        reacted(cell(FUEL, ignite + WINDOW), 15, &params),
        cell(FIRE, ignite + WINDOW) | (LIFE << 4)
    );
    assert_eq!(
        reacted(cell(FUEL, TOP), 15, &[TOP, 0, 0]),
        cell(FIRE, TOP) | (LIFE << 4)
    );
    let burning = cell(FIRE, 700) | (3 << 4);
    assert_eq!(
        reacted(burning, 0, &params),
        cell(FIRE, 700 + flare) | (2 << 4)
    );
    assert_eq!(reacted(cell(FIRE, 700), 0, &params), cell(ASH, 700 + flare));
    assert_eq!(
        reacted(cell(FIRE, TOP - 5) | (1 << 4), 0, &params),
        cell(FIRE, TOP)
    );
    assert_eq!(
        reacted(cell(FIRE, 10) | (1 << 4), 0, &[0, 0, u32::MAX]),
        cell(FIRE, TOP)
    );
    for kind in [AIR, WALL, ASH, KINDS, 15] {
        let word = cell(kind, 5000) | 0xAB00;
        assert_eq!(reacted(word, 0, &params), word);
    }
    assert_eq!(life_of(cell(FIRE, 0) | (LIFE << 4)), LIFE);
}

#[test]
fn diffusion_keeps_the_heat_of_an_oct_and_skips_walls_and_inert_kinds() {
    let mut state = 0x0D1F_u64;
    for _ in 0..200_000 {
        let oct: [u32; CORNERS] = std::array::from_fn(|_| next(&mut state) as u32);
        let after = diffused(oct);
        let sum = |cells: &[u32; CORNERS]| -> u64 {
            cells
                .iter()
                .filter(|&&word| conducts(kind_of(word)))
                .map(|&word| u64::from(temperature_of(word)))
                .sum()
        };
        assert_eq!(sum(&after), sum(&oct), "{oct:x?}");
        let shared: Vec<u32> = after
            .iter()
            .filter(|&&word| conducts(kind_of(word)))
            .map(|&word| temperature_of(word))
            .collect();
        if let (Some(&high), Some(&low)) = (shared.iter().max(), shared.iter().min()) {
            assert!(high - low <= 1);
            assert!(shared.windows(2).all(|pair| pair[0] >= pair[1]));
        }
        for (before, now) in oct.iter().zip(&after) {
            assert_eq!(before & 0xFFFF, now & 0xFFFF);
            if !conducts(kind_of(*before)) {
                assert_eq!(before, now);
            }
        }
    }
}

#[test]
fn steam_rises_water_falls_and_solids_stay() {
    let mut oct = [cell(AIR, 1); CORNERS];
    oct[0] = cell(WATER, 2);
    oct[1] = cell(AIR, 3);
    oct[5] = cell(STEAM, 4);
    oct[2] = cell(STEAM, 5);
    oct[6] = cell(WATER, 6);
    oct[3] = cell(WATER, 7);
    oct[7] = cell(ASH, 8);
    let after = moved(oct);
    assert_eq!(after[0], cell(AIR, 1));
    assert_eq!(after[4], cell(WATER, 2));
    assert_eq!(after[1], cell(STEAM, 4));
    assert_eq!(after[5], cell(AIR, 3));
    assert_eq!(after[2], cell(STEAM, 5));
    assert_eq!(after[6], cell(WATER, 6));
    assert_eq!(after[3], cell(WATER, 7));
    assert_eq!(after[7], cell(ASH, 8));
}

#[test]
fn the_step_reads_its_parameters_and_its_noise() {
    let mut oct = [cell(AIR, 640); CORNERS];
    oct[0] = cell(FUEL, 640);
    let lit = |params: &[u32], frame: u32| {
        Heat.step(Oct {
            words: oct,
            frame,
            at: [3, 4, 5],
            params,
        })
        .out
        .iter()
        .any(|&word| kind_of(word) == FIRE)
    };
    assert!(!lit(&[700, 16, 0], 0));
    assert!(lit(&[600, 16, 0], 0));
    assert!(!lit(&[600, 0, 0], 0));
    let caught = (0..256).filter(|&frame| lit(&[600, 4, 0], frame)).count();
    assert!((32..=96).contains(&caught), "{caught} of 256");
    let nibble = noise([3, 4, 5], 9) & 0xF;
    assert!(lit(&[600, nibble + 1, 0], 9));
    assert!(!lit(&[600, nibble, 0], 9));
}

const REST_FRAMES: u32 = 256;
const REST_SPARE: u32 = 8;

fn last_change(grid: Grid3, start: &[u32], frames: u32) -> Option<(u32, usize)> {
    let mut cells = start.to_vec();
    let mut last = None;
    for frame in 0..frames {
        let next = reference(&Heat, grid, frame, &cells, &TUNED);
        if let Some(at) = (0..cells.len()).find(|&at| next[at] != cells[at]) {
            last = Some((frame, at));
        }
        cells = next;
    }
    last
}

#[test]
fn a_heat_world_of_air_with_one_odd_cell_comes_to_rest() {
    let grid = Grid3::new(16, 16, 16).unwrap();
    let mut failures = Vec::new();
    for odd in [261, 300, 9000] {
        let mut cells = vec![cell(AIR, 260); grid.cells()];
        cells[grid.index(5, 6, 7)] = cell(AIR, odd);
        let total = heat_of(&cells, |_| true);
        let last = last_change(grid, &cells, REST_FRAMES);
        println!("odd cell {odd}: last change {last:?}");
        if let Some((frame, at)) = last
            && frame + REST_SPARE >= REST_FRAMES
        {
            failures.push(format!(
                "odd cell {odd}: cell {at} still changes at frame {frame} of {REST_FRAMES}"
            ));
        }
        let settled = run(grid, &cells, 0, REST_FRAMES);
        assert_eq!(heat_of(&settled, |_| true), total, "odd cell {odd}");
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
