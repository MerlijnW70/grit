#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use grit::rule3::nature::{
    AIR, ASH, BOIL, CHANCE, CHAR, CONDENSE, DEEPEST, EARTH, FALLING, FIRE, FLARE, GRANULAR, HELD,
    HOT, ICE, KINDLES, KINDS, LEAVES, LEAVES_CATCHES, LEAVES_IGNITES, LEAVES_LIFE, MELT, MINUS_X,
    MINUS_Z, Nature, PLUS_X, PLUS_Z, ROCK, SAND, SMOKE, SMOKE_MOST, SNOW, STEAM, TOP, WATER,
    WINDOW, WOOD, WOOD_CATCHES, WOOD_IGNITES, WOOD_LIFE, WOODEN, air, alive, cell, conducts,
    counted, density_of, depth_of, diffused, held_of, kind_of, life_of, moved, reacted, running_of,
    smoke_heat, steepness, stepped, temperature_of, tired_of, touched,
};
use grit::rule3::{CORNERS, Oct, Rule, noise};
use grit::sand3::{Grid3, reference};

const SEEDS: [u64; 4] = [1, 7, 0x5EED, 0xDEAD_BEEF];

/// The heat of the smoke the smoke test lets go, and the most a fire's
/// smoke can outlive it by.
const SMOKE_LIFE: u32 = 96;

fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut mixed = *state;
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    mixed ^ (mixed >> 31)
}

fn step(grid: Grid3, frame: u32, cells: &[u32]) -> Vec<u32> {
    reference(&Nature, grid, frame, cells, &[])
}

fn run(grid: Grid3, start: &[u32], first: u32, frames: u32) -> Vec<u32> {
    let mut cells = start.to_vec();
    for frame in first..first + frames {
        cells = step(grid, frame, &cells);
    }
    cells
}

/// Steps until eight frames in a row change nothing, and returns the world
/// and the frame after the last step; panics past `most` frames.
fn settled(grid: Grid3, start: &[u32], first: u32, most: u32) -> (Vec<u32>, u32) {
    let mut cells = start.to_vec();
    let mut frame = first;
    let mut still = 0;
    while still < 8 {
        assert!(frame < first + most, "not settled in {most} frames");
        let after = step(grid, frame, &cells);
        still = if after == cells { still + 1 } else { 0 };
        cells = after;
        frame += 1;
    }
    (cells, frame)
}

fn count(cells: &[u32], kind: u32) -> usize {
    cells.iter().filter(|&&word| kind_of(word) == kind).count()
}

fn heat_of(cells: &[u32]) -> u64 {
    cells
        .iter()
        .filter(|&&word| conducts(kind_of(word)))
        .map(|&word| u64::from(temperature_of(word)))
        .sum()
}

fn at(grid: Grid3, index: usize) -> (u32, u32, u32) {
    let plane = (grid.wide * grid.deep) as usize;
    let rest = index % plane;
    (
        (rest % grid.wide as usize) as u32,
        (rest / grid.wide as usize) as u32,
        (index / plane) as u32,
    )
}

#[test]
fn the_kind_is_the_low_nibble_and_the_temperature_the_high_half() {
    let mut state = 3;
    for _ in 0..100_000 {
        let word = next(&mut state) as u32;
        assert_eq!(kind_of(word), word & 0xF);
        assert_eq!(temperature_of(word), word >> 16);
    }
    for kind in 0..16 {
        for temperature in [0, 1, MELT, TOP] {
            let word = cell(kind, temperature);
            assert_eq!((kind_of(word), temperature_of(word)), (kind, temperature));
            assert_eq!(word & 0xFFF0, 0);
        }
    }
    assert_eq!(temperature_of(cell(AIR, TOP + 5)), TOP);
    let fire = alive(FIRE, 900, 37);
    assert_eq!(
        (kind_of(fire), temperature_of(fire), life_of(fire)),
        (FIRE, 900, 37)
    );
    assert_eq!(life_of(alive(SMOKE, 0, 255)), 255);
    for depth in 0..=DEEPEST + 2 {
        assert_eq!(depth_of(air(300, depth)), depth.min(DEEPEST));
        assert_eq!(kind_of(air(300, depth)), AIR);
    }
    assert_eq!(
        air(0, DEEPEST),
        0,
        "the word 0 is air at 0 that counts the most air under it"
    );
    let water = cell(WATER, 300) | (PLUS_X << 4) | HELD | (3 << 8);
    assert_eq!(
        (running_of(water), held_of(water), tired_of(water)),
        (PLUS_X, 1, 3)
    );
    assert_eq!(KINDS, 14);
    for kind in KINDS..16 {
        assert!(!conducts(kind));
        assert_eq!(density_of(kind), None);
    }
    assert!(!conducts(ROCK) && !conducts(FIRE));
}

#[test]
fn the_density_order_is_smoke_steam_air_water_then_the_granular_kinds() {
    let order = [SMOKE, STEAM, AIR, WATER, SAND];
    for pair in order.windows(2) {
        assert!(density_of(pair[0]).unwrap() < density_of(pair[1]).unwrap());
    }
    for kind in GRANULAR {
        assert_eq!(density_of(kind), density_of(SAND));
    }
    for kind in [ROCK, ICE, WOOD, LEAVES, FIRE] {
        assert_eq!(density_of(kind), None);
    }
    for upper in 0..16 {
        for lower in 0..16 {
            for x_first in [true, false] {
                let mut cells = [cell(ROCK, 200); CORNERS];
                cells[0] = cell(upper, 200);
                cells[4] = cell(lower, 210);
                let after = moved(cells, x_first);
                let trades = matches!(
                    (density_of(upper), density_of(lower)),
                    (Some(above), Some(below)) if above > below
                );
                if trades {
                    let fell = if upper == WATER {
                        cells[0] | FALLING
                    } else {
                        cells[0]
                    };
                    assert_eq!(
                        (after[0], after[4]),
                        (cells[4], fell),
                        "{upper} over {lower}"
                    );
                } else {
                    let unheld = |word: u32| word & !HELD;
                    assert_eq!(after.map(unheld), cells.map(unheld), "{upper} over {lower}");
                }
                cells[0] = cell(lower, 210);
                cells[4] = cell(upper, 200);
                if !trades {
                    continue;
                }
                let unheld = |word: u32| word & !HELD;
                assert_eq!(
                    moved(cells, x_first).map(unheld),
                    cells.map(unheld),
                    "{lower} over {upper} stays"
                );
            }
        }
    }
}

/// Kinds that cannot react at temperatures below 250: no fire, smoke or
/// steam, and fuel far below ignition.
fn inert(state: &mut u64) -> u32 {
    let roll = next(state);
    let kind = [
        AIR, AIR, AIR, AIR, EARTH, SAND, SNOW, ICE, WATER, WATER, WOOD, LEAVES, ASH, 13, 15,
    ][(roll % 15) as usize];
    let temperature = ((roll >> 8) % 250) as u32;
    let state_bits = if kind == WATER {
        ((roll >> 32) as u32 & 0x7F) << 4
    } else {
        ((roll >> 32) as u32 & 0xFFF) << 4
    };
    cell(kind, temperature) | state_bits
}

#[test]
fn mass_per_kind_is_kept_in_closed_boxes_where_nothing_reacts() {
    for (wide, deep, tall) in [(16, 16, 16), (8, 32, 16)] {
        let grid = Grid3::new(wide, deep, tall).unwrap();
        for seed in SEEDS {
            let mut state = seed;
            let mut cells: Vec<u32> = (0..grid.cells()).map(|_| inert(&mut state)).collect();
            for (index, word) in cells.iter_mut().enumerate() {
                let (x, y, z) = at(grid, index);
                if x == 0 || y == 0 || z == 0 {
                    *word = cell(ROCK, 100);
                }
            }
            let start = cells.clone();
            let masses = |cells: &[u32]| (0..16).map(|kind| count(cells, kind)).collect::<Vec<_>>();
            let want = masses(&start);
            for frame in 0..256 {
                cells = step(grid, frame, &cells);
                assert_eq!(masses(&cells), want, "seed {seed:#x} frame {frame}");
            }
            let walls = |cells: &[u32]| {
                cells
                    .iter()
                    .enumerate()
                    .filter(|&(index, _)| {
                        let (x, y, z) = at(grid, index);
                        x == 0 || y == 0 || z == 0
                    })
                    .map(|(_, &word)| word)
                    .collect::<Vec<_>>()
            };
            assert_eq!(walls(&cells), walls(&start));
            assert_ne!(cells, start, "nothing moved");
        }
    }
}

/// Every kind but fire, at temperatures below every ignition, with any
/// state: smoke fades, steam condenses, ice and snow melt.
fn unburning(state: &mut u64) -> u32 {
    let roll = next(state);
    let kind = loop {
        let kind = (next(state) % 16) as u32;
        if kind != FIRE {
            break kind;
        }
    };
    let temperature = ((roll >> 8) % u64::from(LEAVES_IGNITES)) as u32;
    let state = (roll >> 32) as u32 & 0xFFF;
    let state = if kind == SMOKE {
        (state & 0xF00) | ((state & 0xFF) % KINDLES)
    } else {
        state
    };
    cell(kind, temperature) | (state << 4)
}

#[test]
fn heat_is_conserved_without_fire() {
    for (wide, deep, tall) in [(16, 16, 16), (8, 32, 4)] {
        let grid = Grid3::new(wide, deep, tall).unwrap();
        for seed in SEEDS {
            let mut state = seed;
            let start: Vec<u32> = (0..grid.cells()).map(|_| unburning(&mut state)).collect();
            let total = heat_of(&start);
            let mut cells = start.clone();
            for frame in 0..256 {
                cells = step(grid, frame, &cells);
                assert_eq!(heat_of(&cells), total, "seed {seed:#x} frame {frame}");
                assert_eq!(count(&cells, FIRE), 0);
            }
            assert!(count(&start, STEAM) > 0 && count(&start, SMOKE) > 0);
            assert_eq!(count(&cells, SMOKE), 0, "the smoke faded");
        }
    }
}

const COLD: u32 = 200;

/// A rock floor at the bottom of a world of cold air.
fn floored(grid: Grid3) -> Vec<u32> {
    let mut cells = vec![air(COLD, 0); grid.cells()];
    for z in 0..grid.tall {
        for x in 0..grid.wide {
            cells[grid.index(x, grid.deep - 1, z)] = cell(ROCK, COLD);
        }
    }
    cells
}

/// The cells of other kinds than air over each column, z then x.
fn heights(grid: Grid3, cells: &[u32]) -> Vec<Vec<u32>> {
    (0..grid.tall)
        .map(|z| {
            (0..grid.wide)
                .map(|x| {
                    let column =
                        (0..grid.deep - 1).filter(|&y| kind_of(cells[grid.index(x, y, z)]) != AIR);
                    u32::try_from(column.count()).unwrap()
                })
                .collect()
        })
        .collect()
}

const POURED: u32 = 800;

/// Pours [`POURED`] grains of `kind`, one a frame, at the middle of the top
/// of a 32 x 64 x 32 world with a rock floor, and lets them settle. Returns
/// the peak's height and the base's radius, the farthest column with a
/// grain along either axis from the middle.
fn pile(kind: u32) -> (u32, u32) {
    let grid = Grid3::new(32, 64, 32).unwrap();
    let mut cells = floored(grid);
    let source = grid.index(16, 0, 16);
    let mut poured = 0;
    let mut frame = 0;
    while poured < POURED {
        if kind_of(cells[source]) == AIR {
            cells[source] = cell(kind, COLD);
            poured += 1;
        }
        cells = step(grid, frame, &cells);
        frame += 1;
    }
    let (cells, _) = settled(grid, &cells, frame, 4096);
    assert_eq!(count(&cells, kind), POURED as usize);
    let heights = heights(grid, &cells);
    let peak = heights.iter().flatten().copied().max().unwrap();
    let mut radius = 0;
    for (z, row) in (0_u32..).zip(&heights) {
        for (x, &high) in (0_u32..).zip(row) {
            if high > 0 {
                radius = radius.max(x.abs_diff(16).max(z.abs_diff(16)));
            }
        }
    }
    (peak, radius)
}

/// Each granular kind poured on a floor piles up to a cone whose angle,
/// the arc tangent of its height over its radius, rises from sand to ash,
/// snow and earth.
#[test]
fn each_granular_kind_piles_at_its_own_angle() {
    let mut angles = Vec::new();
    for kind in [SAND, ASH, SNOW, EARTH] {
        let (peak, radius) = pile(kind);
        let angle = f64::from(peak).atan2(f64::from(radius)).to_degrees();
        println!(
            "kind {kind} steepness {:?}: peak {peak}, radius {radius}, angle {angle:.1} degrees",
            steepness(kind)
        );
        angles.push(angle);
    }
    assert!((40.0..=50.0).contains(&angles[0]), "{angles:?}");
    for pair in angles.windows(2) {
        assert!(pair[1] > pair[0] + 3.0, "{angles:?}");
    }
}

/// A shelf of rock high on the left, a sheer drop, and a basin of rock
/// below on the right, the same along z, with a river three deep on the
/// shelf.
fn ledge(grid: Grid3) -> Vec<u32> {
    let mut cells = vec![air(280, 0); grid.cells()];
    for z in 0..grid.tall {
        for y in 0..grid.deep {
            for x in 0..grid.wide {
                let rock =
                    x == 0 || x + 1 == grid.wide || y + 4 >= grid.deep || (x < 14 && y >= 12);
                let river = !rock && x < 14 && (9..12).contains(&y);
                cells[grid.index(x, y, z)] = if rock {
                    cell(ROCK, 280)
                } else if river {
                    cell(WATER, 280)
                } else {
                    air(280, 0)
                };
            }
        }
    }
    cells
}

/// The river runs off the edge of the shelf: water falls through the open
/// air under the lip, a waterfall, and gathers in the basin, whose water
/// rises from none; no water is lost.
#[test]
fn a_river_falls_off_a_ledge_as_a_waterfall_and_fills_the_basin_below() {
    let grid = Grid3::new(32, 32, 8).unwrap();
    let start = ledge(grid);
    let water = count(&start, WATER);
    let basin = |cells: &[u32]| {
        (0..cells.len())
            .filter(|&index| {
                let (x, y, _) = at(grid, index);
                x >= 14 && y + 4 < grid.deep && y >= 20 && kind_of(cells[index]) == WATER
            })
            .count()
    };
    let falling = |cells: &[u32]| {
        (0..cells.len())
            .filter(|&index| {
                let (x, y, _) = at(grid, index);
                (14..17).contains(&x) && (13..20).contains(&y) && kind_of(cells[index]) == WATER
            })
            .count()
    };
    assert_eq!(basin(&start), 0);
    let mut cells = start;
    let mut most_falling = 0;
    let mut frames_falling = 0;
    for frame in 0..512 {
        cells = step(grid, frame, &cells);
        assert_eq!(count(&cells, WATER), water, "frame {frame}");
        let now = falling(&cells);
        most_falling = most_falling.max(now);
        frames_falling += u32::from(now > 0);
    }
    let gathered = basin(&cells);
    println!(
        "{water} water: at most {most_falling} falling under the lip, falling in {frames_falling} frames, {gathered} in the basin"
    );
    assert!(frames_falling >= 8, "{frames_falling}");
    assert!(most_falling >= 8, "{most_falling}");
    assert!(gathered * 2 >= water, "{gathered} of {water}");
}

/// A heap of snow and a block of ice on a floor in still air: in warm air
/// both melt to water, in cold air neither does.
#[test]
fn snow_and_ice_melt_in_warm_air_and_keep_in_cold_air() {
    let grid = Grid3::new(16, 16, 16).unwrap();
    for (warmth, melts) in [(300, true), (MELT - 1, false)] {
        let mut cells = vec![air(warmth, 0); grid.cells()];
        for z in 0..16 {
            for x in 0..16 {
                cells[grid.index(x, 15, z)] = cell(ROCK, warmth);
            }
        }
        for z in 4..8 {
            for x in 4..8 {
                for y in 11..15 {
                    cells[grid.index(x, y, z)] = cell(SNOW, 250);
                    cells[grid.index(x + 6, y, z + 4)] = cell(ICE, 250);
                }
            }
        }
        let frozen = count(&cells, SNOW) + count(&cells, ICE);
        let cells = run(grid, &cells, 0, 400);
        let left = count(&cells, SNOW) + count(&cells, ICE);
        println!(
            "air at {warmth}: {left} of {frozen} frozen cells left, {} water",
            count(&cells, WATER)
        );
        if melts {
            assert_eq!(left, 0);
            assert_eq!(count(&cells, WATER), frozen);
        } else {
            assert_eq!(left, frozen);
            assert_eq!(count(&cells, WATER), 0);
        }
    }
}

const FOREST: u32 = 300;

/// Rock under earth, and trees three cells apart, trunks of wood five tall
/// under crowns of leaves three wide whose edges touch.
fn forest(grid: Grid3) -> Vec<u32> {
    let mut cells = vec![air(FOREST, 0); grid.cells()];
    let floor = grid.deep - 1;
    for z in 0..grid.tall {
        for x in 0..grid.wide {
            cells[grid.index(x, floor, z)] = cell(ROCK, FOREST);
            cells[grid.index(x, floor - 1, z)] = cell(EARTH, FOREST);
        }
    }
    for tz in (2..grid.tall - 2).step_by(3) {
        for tx in (2..grid.wide - 2).step_by(3) {
            for y in floor - 6..floor - 1 {
                cells[grid.index(tx, y, tz)] = cell(WOOD, FOREST);
            }
            for y in floor - 9..floor - 5 {
                for dz in 0..3 {
                    for dx in 0..3 {
                        let at = grid.index(tx + dx - 1, y, tz + dz - 1);
                        if kind_of(cells[at]) == AIR {
                            cells[at] = cell(LEAVES, FOREST);
                        }
                    }
                }
            }
        }
    }
    cells
}

/// One fire at the top of a trunk spreads through the forest, which burns
/// to ash; smoke rises above the crowns while it burns, and fades once the
/// fire is out.
#[test]
fn a_fire_in_a_small_forest_spreads_burns_out_and_leaves_ash() {
    let grid = Grid3::new(32, 32, 32).unwrap();
    let mut cells = forest(grid);
    let fuel = count(&cells, WOOD) + count(&cells, LEAVES);
    let crowns = grid.deep - 10;
    cells[grid.index(2, grid.deep - 7, 2)] = alive(FIRE, 900, WOOD_LIFE);
    let mut most_fire = 0;
    let mut highest = grid.deep;
    let mut out_at = None;
    let mut frame = 0;
    while frame < 2000 {
        cells = step(grid, frame, &cells);
        frame += 1;
        let fire = count(&cells, FIRE);
        most_fire = most_fire.max(fire);
        for (index, &word) in cells.iter().enumerate() {
            if kind_of(word) == SMOKE {
                highest = highest.min(at(grid, index).1);
            }
        }
        if fire == 0 && out_at.is_none() {
            out_at = Some(frame);
        }
        if fire == 0 && count(&cells, SMOKE) == 0 {
            break;
        }
    }
    let left = count(&cells, WOOD) + count(&cells, LEAVES);
    let out_at = out_at.unwrap();
    println!(
        "{fuel} fuel, {left} left, most fire {most_fire}, fire out at frame {out_at}, smoke gone at {frame}, highest smoke at y {highest}, crowns from y {crowns}"
    );
    assert!(left * 20 <= fuel, "{left} of {fuel} unburned");
    assert_eq!(count(&cells, ASH) + count(&cells, CHAR), fuel - left);
    assert!(count(&cells, CHAR) > 0, "no trunk stands as char");
    assert_eq!(count(&cells, FIRE), 0);
    assert_eq!(count(&cells, SMOKE), 0, "the smoke never faded");
    assert!(highest + 4 < crowns, "smoke rose only to y {highest}");
    assert!(frame > out_at, "smoke outlived the fire");
    assert!(frame <= out_at + SMOKE_MOST + 1);
}

#[test]
fn water_puts_out_fire_and_makes_steam() {
    for fire_at in 0..CORNERS {
        for water_at in 0..CORNERS {
            if fire_at == water_at {
                continue;
            }
            let mut cells = [cell(ROCK, 300); CORNERS];
            cells[fire_at] = alive(FIRE, 900, 20) | (0x5 << 12);
            cells[water_at] = cell(WATER, 280);
            let after = touched(cells, 0);
            assert_eq!(after[fire_at], cell(ASH, 900) | (0x5 << 12));
            assert_eq!(after[water_at], cell(STEAM, BOIL));
        }
    }
    let mut cells = [cell(ROCK, 300); CORNERS];
    cells[1] = alive(FIRE, 900, 20);
    cells[2] = cell(WATER, 2000);
    assert_eq!(touched(cells, 0)[2], cell(STEAM, 2000));

    let grid = Grid3::new(16, 16, 16).unwrap();
    for wet in [false, true] {
        let mut cells = vec![air(300, 0); grid.cells()];
        for z in 0..16 {
            for x in 0..16 {
                cells[grid.index(x, 15, z)] = cell(ROCK, 300);
            }
        }
        for y in 9..15 {
            cells[grid.index(8, y, 8)] = alive(FIRE, 900, WOOD_LIFE);
        }
        if wet {
            for y in 2..8 {
                for z in 7..10 {
                    for x in 7..10 {
                        cells[grid.index(x, y, z)] = cell(WATER, 280);
                    }
                }
            }
        }
        let mut steam = 0;
        for frame in 0..20 {
            cells = step(grid, frame, &cells);
            steam = steam.max(count(&cells, STEAM));
        }
        println!(
            "wet {wet}: fire {} ash {} most steam {steam}",
            count(&cells, FIRE),
            count(&cells, ASH)
        );
        if wet {
            assert_eq!(count(&cells, FIRE), 0);
            assert_eq!(count(&cells, ASH), 6);
            assert!(steam > 0);
        } else {
            assert_eq!(count(&cells, FIRE), 6);
            assert_eq!(steam, 0);
        }
    }
}

/// Steam at the bottom of a closed shaft of air as warm as it rises to the
/// top; once the air is cold it condenses back to water, which falls to
/// the floor. No water is lost.
#[test]
fn steam_rises_and_condenses_to_water_when_cold() {
    let grid = Grid3::new(8, 32, 8).unwrap();
    let warm = BOIL;
    let mut cells = vec![air(warm, 0); grid.cells()];
    for (index, word) in cells.iter_mut().enumerate() {
        let (x, y, z) = at(grid, index);
        if x == 0 || z == 0 || y == 31 || y == 0 {
            *word = cell(ROCK, warm);
        }
    }
    for x in 2..6 {
        cells[grid.index(x, 30, 4)] = cell(STEAM, warm);
    }
    let (risen, frame) = settled(grid, &cells, 0, 1000);
    let mut heights = Vec::new();
    for (index, &word) in risen.iter().enumerate() {
        if kind_of(word) == STEAM {
            heights.push(at(grid, index).1);
        }
    }
    println!("steam settled at y {heights:?} by frame {frame}");
    assert_eq!(heights, [1, 1, 1, 1], "the steam rose to the top");
    let cold: Vec<u32> = risen
        .iter()
        .map(|&word| (word & 0xFFFF) | ((CONDENSE - 100) << 16))
        .collect();
    let (cells, _) = settled(grid, &cold, frame, 1000);
    assert_eq!(count(&cells, STEAM), 0);
    assert_eq!(count(&cells, WATER), 4);
    for (index, &word) in cells.iter().enumerate() {
        if kind_of(word) == WATER {
            assert_eq!(at(grid, index).1, 30, "the water fell to the floor");
        }
    }
}

/// Smoke let go in still air rises, drifts toward plus x on average, and
/// fades to air after its life.
#[test]
fn smoke_rises_drifts_with_the_wind_and_fades_to_air() {
    let grid = Grid3::new(64, 64, 16).unwrap();
    let mut cells = vec![air(300, 0); grid.cells()];
    for z in 4..12 {
        for x in 28..36 {
            cells[grid.index(x, 56, z)] = alive(SMOKE, 300, SMOKE_LIFE);
        }
    }
    let mean = |cells: &[u32]| {
        let mut sum = (0_u64, 0_u64);
        let mut many = 0_u64;
        for (index, &word) in cells.iter().enumerate() {
            if kind_of(word) == SMOKE {
                let (x, y, _) = at(grid, index);
                sum.0 += u64::from(x);
                sum.1 += u64::from(y);
                many += 1;
            }
        }
        (sum.0 as f64 / many as f64, sum.1 as f64 / many as f64)
    };
    let before = mean(&cells);
    let smoke = count(&cells, SMOKE);
    let hot = run(grid, &cells, 0, SMOKE_LIFE - HOT);
    assert_eq!(count(&hot, SMOKE), smoke, "hot smoke does not spread");
    let half = run(grid, &cells, 0, SMOKE_LIFE / 2);
    let drifted = mean(&half);
    println!(
        "smoke from {before:?} to {drifted:?} in {} frames",
        SMOKE_LIFE / 2
    );
    assert!(count(&half, SMOKE) >= smoke, "smoke was lost");
    assert!(drifted.0 > before.0 + 2.0, "no drift toward plus x");
    assert!(drifted.1 < before.1 - 8.0, "the smoke did not rise");
    let late = run(grid, &half, SMOKE_LIFE / 2, SMOKE_LIFE / 2 + 1);
    assert_eq!(count(&late, SMOKE), 0);
    assert_eq!(count(&late, AIR), grid.cells());
    assert_eq!(heat_of(&late), heat_of(&cells));
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

/// A world in which the noise decides nothing, moved by an even number of
/// cells along any axis, steps to the same world moved: the rule sees no
/// seam. And a grain, smoke and running water each cross a seam.
#[test]
fn wrap_holds_on_all_three_axes() {
    let grid = Grid3::new(16, 8, 32).unwrap();
    for seed in SEEDS {
        let mut state = seed;
        let start: Vec<u32> = (0..grid.cells()).map(|_| inert(&mut state)).collect();
        let end = run(grid, &start, 0, 64);
        assert_ne!(end, start);
        for by in [[2, 0, 0], [0, 2, 0], [0, 0, 2], [14, 6, 30], [6, 2, 18]] {
            let moved_end = run(grid, &shifted(grid, &start, by), 0, 64);
            assert_eq!(
                moved_end,
                shifted(grid, &end, by),
                "seed {seed:#x} by {by:?}"
            );
        }
    }

    let grid = Grid3::new(8, 8, 8).unwrap();
    let mut cells = vec![air(200, 0); grid.cells()];
    cells[grid.index(3, 7, 3)] = cell(SAND, 200);
    let after = step(grid, 1, &cells);
    assert_eq!(
        kind_of(after[grid.index(3, 0, 3)]),
        SAND,
        "a grain falls across the y seam"
    );

    let mut cells = vec![air(200, 0); grid.cells()];
    cells[grid.index(3, 0, 3)] = alive(SMOKE, 200, 50);
    let after = step(grid, 1, &cells);
    let risen = after
        .iter()
        .position(|&word| kind_of(word) == SMOKE)
        .unwrap();
    assert_eq!(at(grid, risen).1, 7, "smoke rises across the y seam");

    for (way, from, to) in [
        (PLUS_X, (7, 4, 4), (0, 4, 4)),
        (PLUS_Z, (4, 4, 7), (4, 4, 0)),
        (MINUS_X, (0, 4, 4), (7, 4, 4)),
        (MINUS_Z, (4, 4, 0), (4, 4, 7)),
    ] {
        let mut cells = vec![cell(ROCK, 200); grid.cells()];
        let runner = cell(WATER, 200) | (way << 4);
        cells[grid.index(from.0, from.1, from.2)] = runner;
        cells[grid.index(to.0, to.1, to.2)] = air(200, 0);
        let after = step(grid, 1, &cells);
        assert_eq!(
            after[grid.index(to.0, to.1, to.2)],
            runner,
            "{from:?} to {to:?}"
        );
        assert_eq!(kind_of(after[grid.index(from.0, from.1, from.2)]), AIR);
    }
}

/// Every kind, fire and fuel past ignition among them, so the noise
/// decides.
fn burning(grid: Grid3, seed: u64) -> Vec<u32> {
    let mut state = seed;
    (0..grid.cells())
        .map(|_| {
            let roll = next(&mut state);
            let kind = (roll % 16) as u32;
            let temperature = ((roll >> 8) % 1200) as u32;
            cell(kind, temperature) | (((roll >> 32) as u32 & 0xFFF) << 4)
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
        let later = run(grid, &start, 4, 64);
        assert_ne!(
            once, later,
            "seed {seed:#x}: the noise did not depend on the frame"
        );
    }
}

#[test]
fn each_reaction_happens_at_its_threshold() {
    let kept = 0xA << 12;
    for frozen in [SNOW, ICE] {
        assert_eq!(
            reacted(cell(frozen, MELT - 1) | kept, 0),
            cell(frozen, MELT - 1) | kept
        );
        assert_eq!(
            reacted(cell(frozen, MELT) | kept | (0x3F << 4), 0),
            cell(WATER, MELT) | kept
        );
    }
    assert_eq!(reacted(cell(WATER, BOIL - 1), 0), cell(WATER, BOIL - 1));
    assert_eq!(reacted(cell(WATER, BOIL) | HELD, 0), cell(STEAM, BOIL));
    assert_eq!(reacted(cell(STEAM, CONDENSE), 0), cell(STEAM, CONDENSE));
    assert_eq!(
        reacted(cell(STEAM, CONDENSE - 1), 0),
        cell(WATER, CONDENSE - 1)
    );
    for (fuel, ignites, life, wooden) in [
        (WOOD, WOOD_IGNITES, WOOD_LIFE, WOODEN),
        (LEAVES, LEAVES_IGNITES, LEAVES_LIFE, 0),
    ] {
        for nibble in 0..16 {
            assert_eq!(
                reacted(cell(fuel, ignites - 1), nibble),
                cell(fuel, ignites - 1)
            );
            let lucky = reacted(cell(fuel, ignites) | kept, nibble);
            if nibble < CHANCE {
                assert_eq!(lucky, alive(FIRE, ignites, life) | wooden | kept);
                assert_eq!(life_of(lucky), life);
            } else {
                assert_eq!(lucky, cell(fuel, ignites + 1) | kept, "warms by one");
            }
            let late = ignites + WINDOW;
            assert_eq!(
                reacted(cell(fuel, late), nibble),
                alive(FIRE, late, life) | wooden
            );
            assert_eq!(reacted(cell(fuel, late - 1), 15), cell(fuel, late));
            assert_eq!(
                reacted(cell(fuel, TOP), nibble),
                alive(FIRE, TOP, life) | wooden
            );
        }
    }
    assert_eq!(
        reacted(alive(FIRE, 700, 5) | WOODEN | kept, 0),
        alive(FIRE, 700, 4) | WOODEN | kept
    );
    assert_eq!(
        reacted(alive(FIRE, 700, 0) | WOODEN | kept, 0),
        cell(CHAR, 700) | kept,
        "burnt wood stands as char"
    );
    assert_eq!(
        reacted(alive(FIRE, 700, 5) | kept, 0),
        alive(FIRE, 700, 4) | kept
    );
    assert_eq!(
        reacted(alive(FIRE, 700, 0) | kept, 0),
        cell(ASH, 700) | kept
    );
    assert_eq!(
        reacted(alive(SMOKE, 700, 9) | kept, 3),
        alive(SMOKE, 700, 8) | kept
    );
    assert_eq!(
        reacted(alive(SMOKE, 700, 0) | kept, 3),
        cell(AIR, 700) | kept
    );
    for kind in [AIR, ROCK, EARTH, SAND, ASH, 13, 14, 15] {
        let word = cell(kind, 5000) | 0xFFF0;
        for nibble in 0..16 {
            assert_eq!(reacted(word, nibble), word);
        }
    }

    let mut cells = [cell(ROCK, 300); CORNERS];
    cells[4] = alive(FIRE, 900, 20);
    cells[0] = air(300, 0);
    cells[5] = cell(WOOD, 300);
    cells[6] = cell(LEAVES, 300);
    cells[1] = cell(EARTH, 300);
    for nibble in 0..16_u32 {
        let noise = nibble * 0x1111_1111;
        let after = touched(cells, noise);
        assert_eq!(kind_of(after[0]), SMOKE, "air over fire smokes");
        assert_eq!(life_of(after[0]), smoke_heat(900));
        assert_eq!(temperature_of(after[0]), 300 + FLARE);
        assert_eq!(after[4], cells[4], "the fire keeps its heat");
        assert_eq!(after[2], cells[2], "rock does not warm");
        assert_eq!(after[1], cell(EARTH, 300 + FLARE));
        let wood = if nibble < WOOD_CATCHES {
            alive(FIRE, 300, WOOD_LIFE) | WOODEN
        } else {
            cell(WOOD, 300 + FLARE)
        };
        let leaves = if nibble < LEAVES_CATCHES {
            alive(FIRE, 300, LEAVES_LIFE)
        } else {
            cell(LEAVES, 300 + FLARE)
        };
        assert_eq!((after[5], after[6]), (wood, leaves), "nibble {nibble}");
    }
    let mut cold = cells;
    cold[4] = cell(ASH, 900);
    assert_eq!(touched(cold, 0), cold, "nothing happens without fire");
}

#[test]
fn diffusion_keeps_the_heat_of_an_oct_and_skips_rock_fire_and_inert_kinds() {
    let mut state = 11;
    for _ in 0..100_000 {
        let cells: [u32; CORNERS] = std::array::from_fn(|_| {
            let roll = next(&mut state);
            cell((roll % 16) as u32, ((roll >> 8) % 70_000) as u32)
                | (((roll >> 40) as u32 & 0xFFF) << 4)
        });
        let after = diffused(cells);
        assert_eq!(heat_of(&after), heat_of(&cells));
        for (before, now) in cells.iter().zip(&after) {
            assert_eq!(before & 0xFFFF, now & 0xFFFF);
            if !conducts(kind_of(*before)) {
                assert_eq!(before, now);
            }
        }
    }
}

#[test]
fn air_counts_the_air_under_it() {
    let mut cells = [cell(ROCK, 0); CORNERS];
    for depth in 0..=DEEPEST {
        cells[0] = air(9, 0);
        cells[4] = air(9, depth);
        assert_eq!(depth_of(counted(cells)[0]), (depth + 1).min(DEEPEST));
        assert_eq!(counted(cells)[4], cells[4], "a lower cell keeps its count");
    }
    cells[0] = air(9, DEEPEST);
    for kind in 1..16 {
        cells[4] = cell(kind, 9);
        assert_eq!(depth_of(counted(cells)[0]), 0);
    }
    cells[0] = cell(SAND, 9) | 0x30;
    assert_eq!(counted(cells)[0], cells[0], "only air counts");
}

/// A grain falls along a diagonal only past the air its steepness needs;
/// water and sand fall past any lighter cell.
#[test]
fn a_grain_slides_only_past_the_air_its_steepness_needs() {
    for kind in [SAND, ASH, SNOW, EARTH, WATER] {
        let need = steepness(kind).map_or(0, |steep| steep - 1);
        for depth in 0..=DEEPEST {
            let mut cells = [cell(ROCK, 0); CORNERS];
            cells[0] = cell(kind, 0);
            cells[5] = air(0, depth);
            let after = moved(cells, true);
            if depth >= need {
                assert_eq!(
                    (after[0], after[5]),
                    (air(0, depth), cell(kind, 0)),
                    "{kind} depth {depth}"
                );
            } else {
                assert_eq!(after, cells, "{kind} depth {depth}");
            }
        }
        for lighter in [SMOKE, STEAM] {
            let mut cells = [cell(ROCK, 0); CORNERS];
            cells[0] = cell(kind, 0);
            cells[7] = cell(lighter, 0);
            let after = moved(cells, true);
            assert_eq!(after != cells, need == 0, "{kind} into {lighter}");
        }
    }
}

/// The rule's step is [`stepped`] with the engine's noise for the oct and
/// the frame, and the axis order the frame gives.
#[test]
fn the_rule_reads_its_noise_and_its_axis_order_from_the_frame() {
    let mut state = 5;
    for _ in 0..20_000 {
        let words: [u32; CORNERS] = std::array::from_fn(|_| {
            let roll = next(&mut state);
            cell((roll % 16) as u32, ((roll >> 8) % 1200) as u32)
                | (((roll >> 40) as u32 & 0xFFF) << 4)
        });
        let at = [
            next(&mut state) as u32 % 64,
            next(&mut state) as u32 % 64,
            next(&mut state) as u32 % 64,
        ];
        let frame = next(&mut state) as u32;
        let ruled = Nature.step(Oct {
            words,
            frame,
            at,
            params: &[],
        });
        assert_eq!(
            ruled.out,
            stepped(words, noise(at, frame), (frame >> 1) & 1 == 0)
        );
    }
}

const VALLEY: u32 = 260;

fn mix(x: u32, z: u32, seed: u64) -> u64 {
    let mut state = seed ^ (u64::from(x) << 32) ^ u64::from(z).wrapping_mul(0x9E37_79B9);
    next(&mut state)
}

/// Rock ridges along the seams and a bowl between them, earth on the
/// slopes, snow on the ridges, sand on the low ground, a lake in the bowl
/// and trees of wood and leaves, all at one temperature below every
/// threshold.
fn valley(grid: Grid3, seed: u64) -> Vec<u32> {
    let mut cells = vec![air(VALLEY, 0); grid.cells()];
    let reach = (grid.wide / 2).min(grid.tall / 2);
    let lake = 4;
    for z in 0..grid.tall {
        for x in 0..grid.wide {
            let near = x.min(grid.wide - x).min(z.min(grid.tall - z));
            let roll = mix(x, z, seed);
            let high = (2 + (reach - near) * 2 / 3 + (roll % 2) as u32).min(grid.deep - 2);
            let top = grid.deep - high;
            for y in top..grid.deep {
                cells[grid.index(x, y, z)] = cell(ROCK, VALLEY);
            }
            let surface = if near <= 1 {
                SNOW
            } else if high <= lake + 1 {
                SAND
            } else {
                EARTH
            };
            cells[grid.index(x, top, z)] = cell(surface, VALLEY);
            for y in grid.deep - lake..top {
                cells[grid.index(x, y, z)] = cell(WATER, VALLEY);
            }
            if surface == EARTH && (roll >> 8).is_multiple_of(7) && top >= 4 {
                cells[grid.index(x, top - 1, z)] = cell(WOOD, VALLEY);
                cells[grid.index(x, top - 2, z)] = cell(WOOD, VALLEY);
                cells[grid.index(x, top - 3, z)] = cell(LEAVES, VALLEY);
            }
        }
    }
    cells
}

/// A generated valley at rest settles and then nothing changes: its kinds
/// keep their counts and its temperature stays.
#[test]
fn a_valley_at_one_temperature_falls_quiet_and_stays_quiet() {
    for (wide, deep, tall) in [(16, 16, 16), (32, 16, 16), (32, 32, 32)] {
        let grid = Grid3::new(wide, deep, tall).unwrap();
        for seed in SEEDS {
            let start = valley(grid, seed);
            for kind in [ROCK, EARTH, SAND, SNOW, WATER, WOOD, LEAVES] {
                assert!(count(&start, kind) > 0, "{kind}");
            }
            let (cells, frame) = settled(grid, &start, 0, 512);
            assert_eq!(run(grid, &cells, frame, 64), cells);
            for kind in 1..KINDS {
                assert_eq!(count(&cells, kind), count(&start, kind), "kind {kind}");
            }
            assert!(cells.iter().all(|&word| temperature_of(word) == VALLEY));
            println!(
                "{wide}x{deep}x{tall} seed {seed:#x}: quiet from frame {}",
                frame - 8
            );
        }
    }
}

/// Fire, fuel, water and snow dropped into a quiet valley: the fire burns
/// out, the smoke fades, and the valley falls quiet again.
#[test]
fn a_fire_dropped_into_a_quiet_valley_burns_out_and_the_valley_falls_quiet_again() {
    for (wide, deep, tall) in [(16, 16, 16), (32, 16, 16)] {
        let grid = Grid3::new(wide, deep, tall).unwrap();
        for seed in SEEDS {
            let (mut cells, frame) = settled(grid, &valley(grid, seed), 0, 512);
            for (dx, dz, word) in [
                (0, 0, alive(FIRE, 900, WOOD_LIFE)),
                (1, 0, cell(LEAVES, VALLEY)),
                (0, 1, cell(WOOD, VALLEY)),
                (1, 1, cell(LEAVES, VALLEY)),
                (4, 1, cell(WATER, VALLEY)),
                (4, 3, cell(SNOW, VALLEY)),
            ] {
                cells[grid.index(wide / 2 + dx, 1, tall / 2 + dz)] = word;
            }
            let ash = count(&cells, ASH) + count(&cells, CHAR);
            let (cells, quiet) = settled(grid, &cells, frame, 2048);
            assert_eq!(count(&cells, FIRE) + count(&cells, SMOKE), 0);
            assert!(quiet - frame > WOOD_LIFE, "the fire did not burn its life");
            assert_eq!(
                count(&cells, ASH) + count(&cells, CHAR),
                ash + 4,
                "the fire and its three fuel cells"
            );
            println!(
                "{wide}x{deep}x{tall} seed {seed:#x}: quiet again {} frames after the drop",
                quiet - frame
            );
        }
    }
}
