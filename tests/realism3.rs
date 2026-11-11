#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use grit::rule3::CORNERS;
use grit::rule3::nature::{
    AIR, ASH, CHAR, EARTH, FALLING, FIRE, FLARE, HOT, ICE, KINDLES, LEAVES, LEAVES_CATCHES,
    LEAVES_CATCHES_ABOVE, Nature, RISE, ROCK, SAND, SKY, SMOKE, SMOKE_DEGREES, SNOW, STEAM, THAW,
    WATER, WOOD, WOOD_CATCHES, WOOD_CATCHES_ABOVE, WOOD_LIFE, air, alive, cell, falling_of,
    kind_of, life_of, open_air, smoke_heat, stepped, sunned, thaw_of, touched,
};
use grit::sand3::{Grid3, reference};

fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut mixed = *state;
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    mixed ^ (mixed >> 31)
}

fn step(grid: Grid3, frame: u32, cells: &[u32], params: &[u32]) -> Vec<u32> {
    reference(&Nature, grid, frame, cells, params)
}

fn count(cells: &[u32], kind: u32) -> usize {
    cells.iter().filter(|&&word| kind_of(word) == kind).count()
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

const COOL: u32 = 260;

/// A world of still air at [`COOL`] over a rock floor two cells thick.
fn floored(grid: Grid3) -> Vec<u32> {
    let mut cells = vec![air(COOL, 0); grid.cells()];
    for z in 0..grid.tall {
        for x in 0..grid.wide {
            for y in grid.deep - 2..grid.deep {
                cells[grid.index(x, y, z)] = cell(ROCK, COOL);
            }
        }
    }
    cells
}

/// The heat of the burner the plume rises from.
const BURNER: u32 = 500;

/// The smoke over a burner of fire at [`BURNER`] in the middle of a floor,
/// kept burning every frame: per band of `band` rows up from the burner,
/// the smoke cells summed over the frames from `from` on, and the spread of
/// their places across x and z around the band's mean, the root of the
/// summed variances.
fn plume(grid: Grid3, frames: u32, from: u32, band: u32) -> Vec<(u64, f64)> {
    let mut cells = floored(grid);
    let (cx, cz) = (grid.wide / 2, grid.tall / 2);
    let base = grid.deep - 3;
    let burner: Vec<usize> = (0..2)
        .flat_map(|dz| (0..2).map(move |dx| (dx, dz)))
        .map(|(dx, dz)| grid.index(cx + dx, base, cz + dz))
        .collect();
    let bands = (base / band) as usize;
    let mut sums = vec![(0_u64, 0.0_f64, 0.0_f64, 0.0_f64, 0.0_f64); bands];
    for frame in 0..frames {
        for &place in &burner {
            cells[place] = alive(FIRE, BURNER, WOOD_LIFE);
        }
        cells = step(grid, frame, &cells, &[]);
        if frame < from {
            continue;
        }
        for (index, &word) in cells.iter().enumerate() {
            if kind_of(word) != SMOKE {
                continue;
            }
            let (x, y, z) = at(grid, index);
            if y >= base {
                continue;
            }
            let which = ((base - 1 - y) / band) as usize;
            if let Some(held) = sums.get_mut(which) {
                let (x, z) = (f64::from(x), f64::from(z));
                held.0 += 1;
                held.1 += x;
                held.2 += x * x;
                held.3 += z;
                held.4 += z * z;
            }
        }
    }
    sums.iter()
        .map(|&(many, sx, sxx, sz, szz)| {
            if many == 0 {
                return (0, 0.0);
            }
            let n = many as f64;
            let vx = sxx / n - (sx / n) * (sx / n);
            let vz = szz / n - (sz / n) * (sz / n);
            (many, (vx + vz).max(0.0).sqrt())
        })
        .collect()
}

/// Smoke over a steady fire rises as a narrow column and widens with
/// height into a cap: the band at the top of the plume is wider than the
/// band over the fire, and holds more smoke, where the cooled smoke slows
/// and spreads.
#[test]
fn a_plume_widens_with_height_into_a_cap() {
    let grid = Grid3::new(64, 128, 64).unwrap();
    let band = 16;
    let bands = plume(grid, 320, 192, band);
    for (which, &(many, spread)) in bands.iter().enumerate() {
        println!(
            "rows {} to {} over the fire: {many} smoke cells over the frames, spread {spread:.2} cells",
            which as u32 * band,
            (which as u32 + 1) * band - 1
        );
    }
    let (low_many, low_spread) = bands[0];
    let (top, &(top_many, top_spread)) = bands
        .iter()
        .enumerate()
        .max_by_key(|(_, held)| held.0)
        .unwrap();
    println!(
        "the cap: band {top}, {top_many} cells, spread {top_spread:.2}; the column over the fire: {low_many} cells, spread {low_spread:.2}"
    );
    assert!(top >= 2, "the most smoke lies in band {top}, not in a cap");
    assert!(top_many > low_many * 2, "{top_many} against {low_many}");
    assert!(
        top_spread > low_spread * 2.0,
        "{top_spread:.2} against {low_spread:.2}"
    );
    let widest = bands.iter().map(|held| held.1).fold(0.0_f64, f64::max);
    assert!(widest >= top_spread);
}

/// Smoke over a fire takes its heat from the fire, a step of heat for each
/// [`SMOKE_DEGREES`] degrees of the hotter of the fire and the air, up to
/// 255, so a hotter fire makes smoke that lives longer.
#[test]
fn smoke_takes_its_heat_from_the_fire_under_it() {
    for (flame, warmth) in [(300, 260), (600, 260), (1000, 260), (260, 900), (5000, 0)] {
        let mut cells = [cell(ROCK, 0); CORNERS];
        cells[4] = alive(FIRE, flame, 9);
        cells[0] = air(warmth, 3);
        let after = touched(cells, 0xFFFF_FFFF);
        let made = after[0];
        let hottest = flame.max(warmth + FLARE);
        assert_eq!(kind_of(made), SMOKE);
        assert_eq!(life_of(made), (hottest / SMOKE_DEGREES).min(255));
        assert_eq!(life_of(made), smoke_heat(hottest));
    }
}

/// A puff of smoke released in still air: the steps until the last of it
/// fades, and its mean rise over its first 12 steps.
fn puff(heat: u32) -> (u32, f64) {
    let grid = Grid3::new(16, 256, 16).unwrap();
    let mut cells = vec![air(COOL, 0); grid.cells()];
    let low = grid.deep - 8;
    for z in 6..10 {
        for x in 6..10 {
            cells[grid.index(x, low, z)] = alive(SMOKE, COOL, heat);
        }
    }
    let mean = |cells: &[u32]| {
        let rows: Vec<f64> = cells
            .iter()
            .enumerate()
            .filter(|(_, word)| kind_of(**word) == SMOKE)
            .map(|(index, _)| f64::from(at(grid, index).1))
            .collect();
        rows.iter().sum::<f64>() / rows.len().max(1) as f64
    };
    let start = mean(&cells);
    let mut rose = 0.0;
    let mut frame = 0;
    while count(&cells, SMOKE) > 0 {
        cells = step(grid, frame, &cells, &[]);
        frame += 1;
        if frame == 12 {
            rose = start - mean(&cells);
        }
        assert!(frame <= heat + 1, "smoke of heat {heat} outlived it");
    }
    (frame, rose)
}

/// Hot smoke rises faster than cooling smoke and lives for its heat:
/// smoke rises every step while it is hot, and slower as it cools.
#[test]
fn hot_smoke_rises_faster_and_lives_by_its_heat() {
    let mut runs = Vec::new();
    for heat in [24, 48, 120, 240] {
        let (gone, rose) = puff(heat);
        println!(
            "smoke of heat {heat}: gone after {gone} steps, rose {rose:.1} rows in its first 12"
        );
        runs.push((heat, gone, rose));
    }
    for pair in runs.windows(2) {
        assert!(pair[1].1 > pair[0].1, "{runs:?}");
        assert!(pair[1].2 > pair[0].2 || pair[0].0 >= HOT, "{runs:?}");
    }
    let (_, _, hot_rose) = runs[3];
    let (_, _, cool_rose) = runs[0];
    assert!(
        hot_rose >= 11.0,
        "hot smoke rose {hot_rose:.1} rows in 12 steps"
    );
    assert!(
        cool_rose * 2.0 < hot_rose,
        "{cool_rose:.1} against {hot_rose:.1}"
    );
    assert_eq!(HOT, 15 * RISE);
}

/// An even slope rising along x, six cells high at x 0 and one cell
/// higher every second column, under touching crowns of leaves on trunks
/// of wood.
fn slope(grid: Grid3) -> Vec<u32> {
    let mut cells = vec![air(COOL, 7); grid.cells()];
    let ground = |x: u32| 6 + x / 2;
    let floor = |x: u32| grid.deep - 1 - ground(x);
    for z in 0..grid.tall {
        for x in 0..grid.wide {
            for y in floor(x) + 1..grid.deep {
                cells[grid.index(x, y, z)] = cell(ROCK, COOL);
            }
        }
    }
    for z in (1..grid.tall).step_by(3) {
        for x in (1..grid.wide - 1).step_by(3) {
            let foot = floor(x);
            for up in 0..7 {
                cells[grid.index(x, foot - up, z)] = cell(WOOD, COOL);
            }
            for up in 3..8 {
                for dz in 0..3 {
                    for dx in 0..3 {
                        let slot =
                            grid.index(x + dx - 1, foot - up, (z + dz + grid.tall - 1) % grid.tall);
                        if kind_of(cells[slot]) == AIR {
                            cells[slot] = cell(LEAVES, COOL);
                        }
                    }
                }
            }
        }
    }
    cells
}

/// Lights the slope at column `middle` across all of z and returns, after
/// `frames` steps from `first`, how far downhill and uphill along x fuel
/// has burned: the farthest column, each way, where a cell that was fuel
/// is no longer.
fn reach(cells: &[u32], grid: Grid3, middle: u32, first: u32, frames: u32) -> (u32, u32) {
    let start = cells.to_vec();
    let mut cells = cells.to_vec();
    for z in 0..grid.tall {
        for y in 0..grid.deep {
            let slot = grid.index(middle, y, z);
            if kind_of(cells[slot]) == LEAVES {
                cells[slot] = alive(FIRE, 900, WOOD_LIFE);
            }
        }
    }
    for frame in first..first + frames {
        cells = step(grid, frame, &cells, &[]);
    }
    let (mut down, mut up) = (0, 0);
    for (index, (&before, &after)) in start.iter().zip(&cells).enumerate() {
        let fuel = |word: u32| [WOOD, LEAVES].contains(&kind_of(word));
        if fuel(before) && !fuel(after) {
            let (x, _, _) = at(grid, index);
            if x < middle {
                down = down.max(middle - x);
            } else {
                up = up.max(x - middle);
            }
        }
    }
    (down, up)
}

/// On an even slope, a line of fire lit across it runs uphill clearly
/// faster than downhill: fuel catches more readily over fire or hot smoke
/// than beside or under it.
#[test]
fn fire_runs_uphill_faster_than_downhill() {
    let grid = Grid3::new(128, 128, 16).unwrap();
    let cells = slope(grid);
    let middle = 64;
    let mut totals = (0, 0);
    for (round, frames) in [(0_u32, 48_u32), (1, 48), (2, 48), (3, 48)] {
        let (down, up) = reach(&cells, grid, middle, round * 1000, frames);
        println!(
            "round {round}: after {frames} steps the fire burned {down} columns downhill and {up} uphill"
        );
        totals.0 += down;
        totals.1 += up;
    }
    let factor = f64::from(totals.1) / f64::from(totals.0.max(1));
    println!(
        "over the rounds: {} downhill, {} uphill, uphill {factor:.2} times downhill",
        totals.0, totals.1
    );
    assert!(totals.0 > 0, "the fire did not spread downhill at all");
    assert!(factor >= 2.0, "uphill only {factor:.2} times downhill");
    const { assert!(LEAVES_CATCHES_ABOVE > LEAVES_CATCHES && WOOD_CATCHES_ABOVE > WOOD_CATCHES) };
}

/// Fuel over fire or hot smoke catches more readily than fuel beside or
/// under it, for every nibble of the noise.
#[test]
fn fuel_over_fire_or_hot_smoke_catches_more_readily() {
    for fuel in [WOOD, LEAVES] {
        let (beside, above) = if fuel == WOOD {
            (WOOD_CATCHES, WOOD_CATCHES_ABOVE)
        } else {
            (LEAVES_CATCHES, LEAVES_CATCHES_ABOVE)
        };
        for roll in 0..16_u32 {
            let noise = roll * 0x1111_1111;
            let caught =
                |cells: [u32; CORNERS], at: usize| kind_of(touched(cells, noise)[at]) == FIRE;
            let mut over = [cell(ROCK, 300); CORNERS];
            over[0] = cell(fuel, 300);
            over[5] = alive(FIRE, 900, 9);
            assert_eq!(
                caught(over, 0),
                roll < above,
                "fuel {fuel} over fire, roll {roll}"
            );
            let mut smoky = over;
            smoky[5] = alive(SMOKE, 300, KINDLES);
            assert_eq!(
                caught(smoky, 0),
                roll < above,
                "fuel {fuel} over hot smoke, roll {roll}"
            );
            smoky[5] = alive(SMOKE, 300, KINDLES - 1);
            assert!(
                !caught(smoky, 0),
                "fuel {fuel} over cooler smoke, roll {roll}"
            );
            let mut side = [cell(ROCK, 300); CORNERS];
            side[0] = cell(fuel, 300);
            side[1] = alive(FIRE, 900, 9);
            assert_eq!(
                caught(side, 0),
                roll < beside,
                "fuel {fuel} beside fire, roll {roll}"
            );
            let mut under = [cell(ROCK, 300); CORNERS];
            under[4] = cell(fuel, 300);
            under[0] = alive(FIRE, 900, 9);
            assert_eq!(
                caught(under, 4),
                roll < beside,
                "fuel {fuel} under fire, roll {roll}"
            );
            under[0] = alive(SMOKE, 300, 200);
            assert!(
                !caught(under, 4),
                "fuel {fuel} under hot smoke, roll {roll}"
            );
        }
    }
}

/// Rock under earth, and trees three cells apart, trunks of wood five tall
/// under crowns of leaves three wide whose edges touch.
fn forest(grid: Grid3) -> Vec<u32> {
    let mut cells = vec![air(300, 0); grid.cells()];
    let floor = grid.deep - 1;
    for z in 0..grid.tall {
        for x in 0..grid.wide {
            cells[grid.index(x, floor, z)] = cell(ROCK, 300);
            cells[grid.index(x, floor - 1, z)] = cell(EARTH, 300);
        }
    }
    for tz in (2..grid.tall - 2).step_by(3) {
        for tx in (2..grid.wide - 2).step_by(3) {
            for y in floor - 6..floor - 1 {
                cells[grid.index(tx, y, tz)] = cell(WOOD, 300);
            }
            for y in floor - 9..floor - 5 {
                for dz in 0..3 {
                    for dx in 0..3 {
                        let at = grid.index(tx + dx - 1, y, tz + dz - 1);
                        if kind_of(cells[at]) == AIR {
                            cells[at] = cell(LEAVES, 300);
                        }
                    }
                }
            }
        }
    }
    cells
}

/// After a forest fire, every burnt trunk cell stands where its wood stood,
/// as char, which never moves, while the burnt leaves fell as ash: a
/// forest of standing black trunks.
#[test]
fn charred_trunks_stand_after_a_fire() {
    let grid = Grid3::new(32, 32, 32).unwrap();
    let mut start = forest(grid);
    start[grid.index(2, grid.deep - 7, 2)] = alive(FIRE, 900, WOOD_LIFE);
    let mut cells = start.clone();
    let mut frame = 0;
    while frame < 4000 && count(&cells, FIRE) + count(&cells, SMOKE) > 0 {
        let before = cells.clone();
        cells = step(grid, frame, &cells, &[]);
        for (index, (&was, &now)) in before.iter().zip(&cells).enumerate() {
            if kind_of(was) == CHAR {
                assert_eq!(
                    kind_of(now),
                    CHAR,
                    "char moved or changed at {:?}",
                    at(grid, index)
                );
            }
        }
        frame += 1;
    }
    let wood = count(&start, WOOD);
    let leaves = count(&start, LEAVES);
    let chars = count(&cells, CHAR);
    let ashes = count(&cells, ASH);
    let mut standing = 0;
    let mut tall = 0;
    for (index, &word) in cells.iter().enumerate() {
        if kind_of(word) != CHAR {
            continue;
        }
        assert_eq!(
            kind_of(start[index]),
            WOOD,
            "char where no wood stood: {:?}",
            at(grid, index)
        );
        standing += 1;
        let (x, y, z) = at(grid, index);
        if kind_of(cells[grid.index(x, y + 1, z)]) == CHAR {
            tall += 1;
        }
    }
    let burnt_leaves = leaves - count(&cells, LEAVES);
    println!(
        "{wood} wood and {leaves} leaves: {chars} char standing ({tall} on char), {ashes} ash, {} wood and {} leaves left, out by frame {frame}",
        count(&cells, WOOD),
        count(&cells, LEAVES)
    );
    assert_eq!(
        count(&cells, FIRE) + count(&cells, SMOKE),
        0,
        "still burning"
    );
    assert_eq!(standing, wood - count(&cells, WOOD));
    assert!(
        chars * 2 >= wood,
        "only {chars} of {wood} trunk cells charred"
    );
    assert!(tall * 2 >= chars, "the char does not stand in trunks");
    assert_eq!(ashes, burnt_leaves + 1, "the leaves and the kindling");
    for (index, &word) in cells.iter().enumerate() {
        if kind_of(word) == ASH {
            let (x, y, z) = at(grid, index);
            let under = kind_of(cells[grid.index(x, y + 1, z)]);
            assert_ne!(under, AIR, "ash left hanging at {x},{y},{z}");
        }
    }
}

/// A snowfield one cell deep on a rock floor, half of it under a roof of
/// rock that shades it, in a world whose air is `opened` to the sky from
/// the start or not.
fn snowfield(grid: Grid3, opened: bool) -> (Vec<u32>, Vec<usize>, Vec<usize>) {
    let mut cells = vec![air(COOL, 0); grid.cells()];
    let floor = grid.deep - 4;
    let (mut lit, mut shaded) = (Vec::new(), Vec::new());
    for z in 0..grid.tall {
        for x in 0..grid.wide {
            for y in floor..grid.deep {
                cells[grid.index(x, y, z)] = cell(ROCK, COOL);
            }
            let slot = grid.index(x, floor - 1, z);
            cells[slot] = cell(SNOW, COOL);
            let roofed = x < grid.wide / 2;
            if roofed {
                cells[grid.index(x, floor - 4, z)] = cell(ROCK, COOL);
                shaded.push(slot);
            } else {
                lit.push(slot);
            }
        }
    }
    if opened {
        for z in 0..grid.tall {
            for x in 0..grid.wide {
                for y in 0..grid.deep {
                    let slot = grid.index(x, y, z);
                    if kind_of(cells[slot]) != AIR {
                        break;
                    }
                    cells[slot] |= SKY;
                }
            }
        }
    }
    (cells, lit, shaded)
}

/// Snow open to the sky melts under the sun at the rate the sun gives, a
/// cell after `THAW / sun + 1` lit steps, and it is lit every second
/// frame; snow under a roof keeps, and with no sun nothing changes at all.
#[test]
fn snow_melts_in_the_sun_at_its_rate_and_keeps_in_the_shade() {
    let grid = Grid3::new(16, 32, 16).unwrap();
    for sun in [4_u32, 8, 32, 255] {
        let lit_steps = THAW / sun + 1;
        let (start, lit, shaded) = snowfield(grid, true);
        let start = (0..64).fold(start, |cells, frame| step(grid, frame, &cells, &[]));
        let params = [sun];
        let mut cells = start.clone();
        let mut melted_at = None;
        let mut frame = 0;
        while frame < 4 * lit_steps + 8 {
            let before = cells.clone();
            cells = step(grid, frame, &cells, &params);
            frame += 1;
            for (index, (&was, &now)) in before.iter().zip(&cells).enumerate() {
                if was != now && melted_at.is_none() {
                    assert!(
                        lit.contains(&index),
                        "sun {sun} frame {frame}: a cell changed that is no lit snow: {:?}",
                        at(grid, index)
                    );
                    assert_eq!(kind_of(was), SNOW);
                }
            }
            if melted_at.is_none() && lit.iter().all(|&slot| kind_of(cells[slot]) != SNOW) {
                melted_at = Some(frame);
            }
            for &slot in &shaded {
                assert_eq!(cells[slot], start[slot], "sun {sun}: shaded snow changed");
            }
        }
        let melted_at = melted_at.expect("the lit snow never melted");
        println!(
            "sun {sun}: a lit snow cell melts after {lit_steps} lit steps; all {} lit cells melted by frame {melted_at}; {} shaded cells kept",
            lit.len(),
            shaded.len()
        );
        assert!(
            (2 * lit_steps - 1..=2 * lit_steps).contains(&melted_at),
            "sun {sun}: melted at frame {melted_at}, not at {}",
            2 * lit_steps
        );
        assert_eq!(count(&cells, SNOW), shaded.len());
    }
    let (start, _, _) = snowfield(grid, true);
    let start = (0..64).fold(start, |cells, frame| step(grid, frame, &cells, &[]));
    let mut cells = start.clone();
    for frame in 64..320 {
        cells = step(grid, frame, &cells, &[0]);
    }
    assert_eq!(cells, start, "with no sun the field changed");
}

/// The sky comes down every open column one row a frame, so snow in a
/// world whose air does not know the sky yet melts once it does, and the
/// sky bit then stays as it is.
#[test]
fn the_sky_comes_down_the_open_columns_and_stays() {
    let grid = Grid3::new(16, 32, 16).unwrap();
    let (start, lit, shaded) = snowfield(grid, false);
    let (opened, _, _) = snowfield(grid, true);
    let params = [255];
    let mut cells = start;
    let mut frame = 0;
    while lit.iter().any(|&slot| kind_of(cells[slot]) == SNOW) {
        cells = step(grid, frame, &cells, &params);
        frame += 1;
        assert!(frame < 2 * grid.deep + 8, "the sky never reached the snow");
    }
    println!(
        "the sky reached the snow {} rows down and melted it by frame {frame}",
        grid.deep - 5
    );
    for _ in 0..64 {
        cells = step(grid, frame, &cells, &params);
        frame += 1;
    }
    for (index, (&now, &want)) in cells.iter().zip(&opened).enumerate() {
        if kind_of(want) == AIR && kind_of(now) == AIR {
            assert_eq!(now & SKY, want & SKY, "sky bit at {:?}", at(grid, index));
        }
    }
    for &slot in &shaded {
        assert_eq!(kind_of(cells[slot]), SNOW);
        assert_eq!(thaw_of(cells[slot]), 0);
    }
    let settled = cells.clone();
    for _ in 0..8 {
        cells = step(grid, frame, &cells, &params);
        frame += 1;
    }
    assert_eq!(
        cells, settled,
        "the field did not fall quiet once its lit snow was gone"
    );
}

/// The sun stage on one oct: the top row is lit, a lower cell under sky
/// air is lit, air takes the sky bit it is lit by, but in phase 1 lower air
/// only gains it, and lit snow and ice take in thaw and melt to water at
/// their own temperature past [`THAW`].
#[test]
fn the_sun_stage_lights_down_the_column_and_thaws_what_it_lights() {
    let mut cells = [cell(ROCK, 200); CORNERS];
    cells[0] = open_air(200, 0);
    cells[4] = cell(SNOW, 200);
    cells[1] = air(200, 0);
    cells[5] = cell(ICE, 210) | (250 << 4);
    cells[2] = cell(LEAVES, 200);
    cells[6] = open_air(200, 0);
    cells[3] = open_air(200, 0);
    cells[7] = air(200, 0);
    let after = sunned(cells, 8, false, false);
    assert_eq!(after[0], cells[0]);
    assert_eq!((kind_of(after[4]), thaw_of(after[4])), (SNOW, 8));
    assert_eq!(
        after[1], cells[1],
        "an upper cell outside the top row stays"
    );
    assert_eq!(after[5], cells[5], "ice under air without the sky stays");
    assert_eq!(after[6], cells[6] & !SKY, "air under leaves loses the sky");
    assert_eq!(after[7], cells[7] | SKY, "air under sky air takes it");
    let top = sunned(cells, 8, true, false);
    assert_eq!(top[1], cells[1] | SKY, "air in the top row takes the sky");
    assert_eq!(top[5], cell(WATER, 210), "ice past its thaw melts");
    assert_eq!(top[2], cells[2]);
    let gaining = sunned(cells, 8, false, true);
    assert_eq!(
        gaining[6], cells[6],
        "in phase 1 lower air only gains the sky"
    );
    assert_eq!(gaining[7], cells[7] | SKY);
    assert_eq!((kind_of(gaining[4]), thaw_of(gaining[4])), (SNOW, 8));
    let mut seam = cells;
    seam[0] = cell(ROCK, 200);
    seam[4] = open_air(200, 0);
    assert_eq!(
        sunned(seam, 8, false, true)[4],
        seam[4],
        "the top row under the bottom row keeps its sky"
    );
    for top in [false, true] {
        for gaining in [false, true] {
            assert_eq!(sunned(cells, 0, top, gaining), cells, "no sun, no change");
        }
    }
}

/// A ledge of rock with water standing on it at the top, a drop under its
/// lip and a basin at the bottom.
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

/// Random octs whose water is tired out, so it moves only by falling, with
/// each cell's corner marked in its kept bits; `kinds` the kinds drawn.
fn marked_octs(state: &mut u64, kinds: &[u32]) -> [u32; CORNERS] {
    let mut cells = [0_u32; CORNERS];
    for (corner, word) in cells.iter_mut().enumerate() {
        let roll = next(state);
        let kind = kinds[(roll % kinds.len() as u64) as usize];
        let bits = ((roll >> 8) & 0xFF) as u32;
        let temperature = ((roll >> 20) % 273) as u32;
        let bits = if kind == WATER {
            (bits & 0x88) | 0x70
        } else {
            bits
        };
        *word = cell(kind, temperature) | (bits << 4) | ((corner as u32) << 12);
    }
    cells
}

/// The falling bit marks exactly the water that fell straight down in the
/// step. On octs of air, smoke, rock and water whose water is tired out,
/// each water cell moves at most once, so the marks in the kept bits tell
/// where it came from: the bit is set exactly on water that came from the
/// cell right above it. On octs of every kind but fire, water marked
/// falling lies in the lower layer under a lighter cell. In a waterfall,
/// marked water lies under a lighter cell too, and none is marked once the
/// basin is still.
#[test]
fn the_falling_bit_is_set_exactly_on_water_that_fell_free() {
    let mut state = 0xFA11_u64;
    let mut fell = 0;
    let mut stood = 0;
    for round in 0..200_000_u32 {
        let cells = marked_octs(&mut state, &[AIR, AIR, WATER, WATER, ROCK, SMOKE]);
        let noise = next(&mut state) as u32;
        let words = stepped(cells, noise, round % 2 == 0);
        for (corner, &word) in words.iter().enumerate() {
            if kind_of(word) != WATER {
                continue;
            }
            let from = ((word >> 12) & 7) as usize;
            let straight = corner >= 4 && from == corner - 4;
            assert_eq!(
                falling_of(word),
                straight,
                "{cells:x?} noise {noise:#x} -> {words:x?}: corner {corner} from {from}"
            );
            if straight {
                fell += 1;
            } else {
                stood += 1;
            }
        }
    }
    println!("{fell} water cells fell straight and were marked, {stood} did not and were not");
    assert!(fell > 10_000 && stood > 10_000);
    let mut marked = 0;
    for round in 0..200_000_u32 {
        let cells = marked_octs(
            &mut state,
            &[
                AIR, AIR, WATER, WATER, ROCK, SMOKE, STEAM, SAND, SNOW, ASH, EARTH, WOOD,
            ],
        );
        let noise = next(&mut state) as u32;
        let words = stepped(cells, noise, round % 2 == 0);
        for (corner, &word) in words.iter().enumerate() {
            if falling_of(word) {
                marked += 1;
                assert!(
                    corner >= 4,
                    "{cells:x?} -> {words:x?}: marked in the upper layer"
                );
                let over = kind_of(words[corner - 4]);
                assert!(
                    [AIR, SMOKE, STEAM].contains(&over),
                    "{cells:x?} -> {words:x?}: marked under kind {over}"
                );
            }
        }
    }
    println!("{marked} marked water cells among all kinds, each under a lighter cell");
    assert!(marked > 10_000);

    let grid = Grid3::new(32, 32, 8).unwrap();
    let mut cells = ledge(grid);
    let mut marked = 0;
    for frame in 0..512 {
        cells = step(grid, frame, &cells, &[]);
        for (index, &word) in cells.iter().enumerate() {
            if !falling_of(word) {
                continue;
            }
            marked += 1;
            let (x, y, z) = at(grid, index);
            let over = kind_of(cells[grid.index(x, y - 1, z)]);
            assert!(
                [AIR, SMOKE, STEAM].contains(&over),
                "marked water at {x},{y},{z} under kind {over}"
            );
        }
        if frame == 40 {
            let under_lip = cells
                .iter()
                .enumerate()
                .filter(|(index, word)| {
                    let (x, y, _) = at(grid, *index);
                    kind_of(**word) == WATER && (14..17).contains(&x) && (13..20).contains(&y)
                })
                .collect::<Vec<_>>();
            let falling = under_lip
                .iter()
                .filter(|(_, word)| falling_of(**word))
                .count();
            println!(
                "frame 40: {} water cells under the lip, {falling} of them marked falling",
                under_lip.len()
            );
            assert!(!under_lip.is_empty() && falling == under_lip.len());
        }
    }
    println!("{marked} falling marks over 512 frames");
    assert!(marked > 100);
    let mut frame = 512;
    let mut quiet = 0;
    while quiet < 8 {
        assert!(frame < 8192, "the basin never stilled");
        let after = step(grid, frame, &cells, &[]);
        quiet = if after == cells { quiet + 1 } else { 0 };
        cells = after;
        frame += 1;
    }
    let left = cells.iter().filter(|&&word| falling_of(word)).count();
    println!("still by frame {frame}: {left} water cells marked falling");
    assert_eq!(
        left, 0,
        "water still marked falling once the basin is still"
    );
    assert!(
        cells
            .iter()
            .all(|&word| kind_of(word) != WATER || word & FALLING == 0)
    );
}
