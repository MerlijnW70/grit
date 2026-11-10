#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use grit::rule::{self, Kind, Pass};
use grit::sand::{Grid, SOLID};
use simdr::lanes::LaneError;

const WIDTHS: [u32; 5] = [4, 8, 16, 32, 64];

const KINDS: [Kind; 7] = [
    Kind::Product,
    Kind::Sum,
    Kind::Turn,
    Kind::Bound,
    Kind::Smooth,
    Kind::Living,
    Kind::Twinned,
];

const FLAT: Grid = Grid::of(256, 128);

fn solids() -> [Grid; 4] {
    [
        Grid::solid(16, 16, 16).unwrap(),
        Grid::solid(64, 32, 2).unwrap(),
        Grid::solid(2, 2, 2).unwrap(),
        Grid::solid(256, 128, 4).unwrap(),
    ]
}

fn refused(built: Result<Vec<u32>, LaneError>, named: &str) {
    match built {
        Err(LaneError::NoSuchForm { operation, because }) => {
            assert_eq!(operation, named);
            assert_eq!(because, SOLID);
        }
        Err(other) => panic!("{named} refused a solid grid with {other:?}, not by name"),
        Ok(_) => panic!("{named} built a kernel for a solid grid"),
    }
}

#[test]
fn rule_built_refuses_a_solid_grid_by_name() {
    for width in WIDTHS {
        for pass in [Pass::Ahead, Pass::Back] {
            for kind in KINDS {
                assert!(rule::built(width, FLAT, pass, kind).is_ok());
                for grid in solids() {
                    refused(rule::built(width, grid, pass, kind), "rule::built");
                }
            }
        }
    }
}

#[test]
fn rule_building_refuses_a_solid_grid_by_name() {
    for pass in [Pass::Ahead, Pass::Back] {
        for kind in KINDS {
            assert!(rule::building::<16>(16, FLAT, pass, kind).is_ok());
            for grid in solids() {
                refused(rule::building::<16>(16, grid, pass, kind), "rule::building");
            }
        }
    }
}
