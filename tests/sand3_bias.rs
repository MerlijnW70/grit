#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use grit::sand::{AIR, SAND, WALL, WATER};
use grit::sand3::{Grid3, material_of, settle_reference3};

const SIDE: u32 = 64;
const MIDDLE: u32 = SIDE / 2;
const SEEDS: [u64; 4] = [1, 2, 3, 4];

const SAND_POURED: u32 = 3000;
const WATER_POURED: u32 = 1000;
const RESTING: u32 = 400;
const RIM: u32 = 24;
const RIM_HIGH: u32 = 4;

const SAND_BOUND: Bound = Bound {
    apart: 0.015,
    spread: 0.01,
};
const WATER_BOUND: Bound = Bound {
    apart: 0.2,
    spread: 0.1,
};

struct Bound {
    apart: f64,
    spread: f64,
}

fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut mixed = *state;
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    mixed ^ (mixed >> 31)
}

struct Profiles {
    poured: usize,
    across_xz: f64,
    mirror_x: f64,
    mirror_z: f64,
    spread: f64,
    reach: u32,
}

fn poured(kind: u32, count: u32, seed: u64) -> Profiles {
    let grid = Grid3::new(SIDE, SIDE, SIDE).unwrap();
    let floor = grid.deep - 1;
    let mut cells = vec![AIR; grid.cells()];
    for z in 0..grid.tall {
        for x in 0..grid.wide {
            cells[grid.index(x, floor, z)] = WALL;
            let from = x.abs_diff(MIDDLE).max(z.abs_diff(MIDDLE));
            if from == RIM {
                for y in floor - RIM_HIGH..floor {
                    cells[grid.index(x, y, z)] = WALL;
                }
            }
        }
    }

    let mut state = seed;
    let mut frame = 0;
    let mut poured = 0;
    while frame < count + RESTING {
        if frame < count {
            let roll = next(&mut state);
            let x = MIDDLE + (roll % 3) as u32 - 1;
            let z = MIDDLE + ((roll >> 8) % 3) as u32 - 1;
            let source = grid.index(x, 1, z);
            if cells[source] == AIR {
                cells[source] = kind;
                poured += 1;
            }
        }
        cells = settle_reference3(grid, frame, &cells);
        frame += 1;
    }

    let mut along_x = vec![0_i64; SIDE as usize];
    let mut along_z = vec![0_i64; SIDE as usize];
    let mut reach = 0;
    for z in 0..grid.tall {
        for y in 0..floor {
            for x in 0..grid.wide {
                if material_of(cells[grid.index(x, y, z)]) == kind && kind != WALL {
                    along_x[x as usize] += 1;
                    along_z[z as usize] += 1;
                    reach = reach.max(x.abs_diff(MIDDLE)).max(z.abs_diff(MIDDLE));
                }
            }
        }
    }
    let total: i64 = along_x.iter().sum();
    assert_eq!(total, poured as i64);
    assert_eq!(along_z.iter().sum::<i64>(), total);
    assert!(reach < RIM, "the {kind} left the basin");

    let apart = |one: &dyn Fn(usize) -> i64, other: &dyn Fn(usize) -> i64| {
        let sum: i64 = (0..SIDE as usize)
            .map(|at| (one(at) - other(at)).abs())
            .sum();
        sum as f64 / (2 * total) as f64
    };
    let second = |profile: &[i64]| -> f64 {
        profile
            .iter()
            .enumerate()
            .map(|(at, &mass)| {
                let off = at as f64 - f64::from(MIDDLE);
                off * off * mass as f64
            })
            .sum()
    };
    let (wide_x, wide_z) = (second(&along_x), second(&along_z));
    let mirrored = |at: usize| (2 * MIDDLE as usize - at) % SIDE as usize;
    Profiles {
        poured,
        across_xz: apart(&|at| along_x[at], &|at| along_z[at]),
        mirror_x: apart(&|at| along_x[at], &|at| along_x[mirrored(at)]),
        mirror_z: apart(&|at| along_z[at], &|at| along_z[mirrored(at)]),
        spread: (wide_x - wide_z) / (wide_x + wide_z),
        reach,
    }
}

fn measured(kind: u32, count: u32, bound: &Bound) {
    let mut worst: f64 = 0.0;
    let mut spread = 0.0;
    for seed in SEEDS {
        let profiles = poured(kind, count, seed);
        println!(
            "material {kind} seed {seed}: poured {} reach {} x against z {:.4} spread x over z {:+.4} x mirrored {:.4} z mirrored {:.4}",
            profiles.poured,
            profiles.reach,
            profiles.across_xz,
            profiles.spread,
            profiles.mirror_x,
            profiles.mirror_z
        );
        worst = worst.max(profiles.across_xz);
        spread += profiles.spread / SEEDS.len() as f64;
    }
    println!(
        "material {kind}: worst x against z {worst:.4} bound {}, mean spread x over z {spread:+.4} bound {}",
        bound.apart, bound.spread
    );
    assert!(
        worst <= bound.apart,
        "x against z {worst} over {}",
        bound.apart
    );
    assert!(
        spread.abs() <= bound.spread,
        "spread x over z {spread} over {}",
        bound.spread
    );
}

#[test]
fn a_sand_pile_poured_in_the_middle_spreads_alike_along_x_and_z() {
    measured(SAND, SAND_POURED, &SAND_BOUND);
}

#[test]
fn water_poured_on_a_floor_spreads_alike_along_x_and_z() {
    measured(WATER, WATER_POURED, &WATER_BOUND);
}
