#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use grit::compact::Plan;
use grit::layout::{AWAKE, DENSE, Dimension, GRAND, HOT, INDIRECT, PARAMS, TALLY};
use grit::live;
use grit::sand::Grid;
use grit::sweep;
use grit::{Host, Resident};
use simdr::lanes::LaneError;

const SOLIDS: [(u32, u32, u32); 4] = [(16, 16, 16), (32, 32, 32), (64, 32, 16), (2, 2, 2)];

fn plan_in(width: u32, grid: Grid, dimension: Dimension) -> Plan {
    Plan::over_in(
        width,
        grid.blocks().div_ceil(sweep::cells(width)),
        dimension,
    )
    .unwrap()
}

fn resident<'host>(host: &'host Host, plan: &Plan) -> Resident<'host> {
    let tick = live::tick(host.width()).unwrap();
    let sizes = plan.sizes();
    let mut held = host.resident(&sizes, &[&tick]).unwrap();
    for (binding, &words) in sizes.iter().enumerate() {
        held.write(binding, &vec![0xFFFF_FFFF; words]).unwrap();
    }
    held
}

/// What `seed` leaves in the list and the flags: every block of the grid
/// listed in order and marked 2, the rest of `AWAKE` zero.
fn seeded(held: &mut Resident<'_>, plan: &Plan, blocks: u32, label: &str) {
    let sizes = plan.sizes();
    assert_eq!(held.read(GRAND as usize, 1).unwrap(), [blocks], "{label}");
    assert_eq!(
        held.read(INDIRECT as usize, 3).unwrap(),
        [blocks.div_ceil(plan.subgroup), 1, 1],
        "{label}"
    );
    let listed = held.read(DENSE as usize, blocks as usize).unwrap();
    assert_eq!(listed, (0..blocks).collect::<Vec<u32>>(), "{label}");
    let awake = held.read(AWAKE as usize, sizes[AWAKE as usize]).unwrap();
    assert!(
        awake[..blocks as usize].iter().all(|&mark| mark == 2),
        "{label}"
    );
    assert!(
        awake[blocks as usize..].iter().all(|&mark| mark == 0),
        "{label}"
    );
    let hot = held.read(HOT as usize, sizes[HOT as usize]).unwrap();
    assert!(
        hot[..plan.blocks as usize].iter().all(|&mark| mark == 2),
        "{label}"
    );
    let tally = held.read(TALLY as usize, sizes[TALLY as usize]).unwrap();
    assert_eq!(tally, vec![0; plan.dimension.tally_words()], "{label}");
}

#[test]
fn seed_lists_and_wakes_every_oct_of_a_solid_grid() {
    let Some(host) = device("seed3") else {
        return;
    };
    let width = host.width();
    for (wide, deep, tall) in SOLIDS {
        let grid = Grid::solid(wide, deep, tall).unwrap();
        let plan = plan_in(width, grid, Dimension::Three);
        let mut held = resident(&host, &plan);
        live::seed(&mut held, grid, &plan).unwrap();
        let octs = grid.blocks();
        if tall > 2 {
            assert!(octs > grid.quads());
        }
        seeded(&mut held, &plan, octs, &format!("{wide}x{deep}x{tall}"));
        assert_eq!(
            held.read(TALLY as usize, 7).unwrap(),
            [0; 7],
            "the 3D tally is seven words"
        );
    }
}

#[test]
fn seed_still_lists_every_quad_of_a_flat_grid() {
    let Some(host) = device("seed2") else {
        return;
    };
    let width = host.width();
    for (wide, deep) in [(256, 128), (64, 64), (2, 2)] {
        let grid = Grid::of(wide, deep);
        let plan = plan_in(width, grid, Dimension::Two);
        let mut held = resident(&host, &plan);
        live::seed(&mut held, grid, &plan).unwrap();
        seeded(&mut held, &plan, grid.quads(), &format!("{wide}x{deep}"));
    }
}

#[test]
fn seed_refuses_a_plan_for_another_number_of_axes_and_writes_nothing() {
    let Some(host) = device("seed unlike") else {
        return;
    };
    let width = host.width();
    let solid = Grid::solid(16, 16, 16).unwrap();
    let flat = Grid::of(64, 64);
    for (grid, plan) in [
        (solid, plan_in(width, solid, Dimension::Two)),
        (flat, plan_in(width, flat, Dimension::Three)),
    ] {
        let mut held = resident(&host, &plan);
        match live::seed(&mut held, grid, &plan) {
            Err(grit::Error::Lane(LaneError::NoSuchForm { operation, because })) => {
                assert_eq!(operation, "live::seed");
                assert_eq!(because, live::UNLIKE);
            }
            other => panic!("a plan for other axes seeded the grid: {other:?}"),
        }
        assert_eq!(
            held.read(PARAMS as usize, 2).unwrap(),
            [0xFFFF_FFFF; 2],
            "the refused seed wrote PARAMS"
        );
        assert_eq!(held.read(GRAND as usize, 1).unwrap(), [0xFFFF_FFFF]);
    }
}
