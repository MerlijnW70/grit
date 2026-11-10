#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use grit::compact::Plan;
use grit::layout::BINDINGS;
use grit::live::{self, Waking};
use grit::sand::{self, Grid, SOLID, View};
use grit::sweep;
use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, U32};

const WIDTHS: [u32; 5] = [4, 8, 16, 32, 64];

fn solids() -> [Grid; 4] {
    [
        Grid::solid(16, 16, 16).unwrap(),
        Grid::solid(64, 32, 2).unwrap(),
        Grid::solid(2, 2, 2).unwrap(),
        Grid::solid(256, 128, 4).unwrap(),
    ]
}

const FLAT: Grid = Grid::of(256, 128);

fn plan(width: u32) -> Plan {
    Plan::over(width, FLAT.quads().div_ceil(sweep::cells(width))).unwrap()
}

fn refused<T>(built: Result<T, LaneError>, named: &str) {
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
fn a_flat_grid_and_a_grid_one_cell_tall_are_the_same_grid() {
    assert_eq!(Grid::solid(256, 128, 1).unwrap(), FLAT);
    assert_eq!(Grid::solid(256, 128, 0).unwrap(), FLAT);
    assert!(sand::flat("any", FLAT).is_ok());
    for grid in solids() {
        assert!(grid.tall > 1);
        refused(sand::flat("any", grid), "any");
    }
}

#[test]
fn sand_settle_refuses_a_solid_grid_by_name() {
    for width in WIDTHS {
        assert!(sand::settle(width, FLAT).is_ok());
        for grid in solids() {
            refused(sand::settle(width, grid), "sand::settle");
        }
    }
}

#[test]
fn sand_brush_refuses_a_solid_grid_by_name() {
    for width in WIDTHS {
        for waking in [false, true] {
            assert!(sand::brush(width, FLAT, waking).is_ok());
            for grid in solids() {
                refused(sand::brush(width, grid, waking), "sand::brush");
            }
        }
    }
}

#[test]
fn sand_paint_refuses_a_solid_grid_by_name() {
    let view = View::of(64, 32);
    for width in WIDTHS {
        assert!(sand::paint(width, FLAT, view).is_ok());
        for grid in solids() {
            refused(sand::paint(width, grid, view), "sand::paint");
        }
    }
}

#[test]
fn sand_at_refuses_a_solid_grid_by_name() {
    let mut kernel = Kernel::<U32>::new(Shape::new(4, 4, BINDINGS)).unwrap();
    let mut lanes = kernel.lanes().unwrap();
    let column = lanes.splat_bits::<U32, 4>(3).unwrap();
    let row = lanes.splat_bits::<U32, 4>(5).unwrap();
    assert!(sand::at::<4>(&mut lanes, FLAT, column, row).is_ok());
    for grid in solids() {
        refused(sand::at::<4>(&mut lanes, grid, column, row), "sand::at");
    }
}

#[test]
fn live_settle_refuses_a_solid_grid_by_name() {
    for width in WIDTHS {
        assert!(live::settle(width, FLAT).is_ok());
        for grid in solids() {
            refused(live::settle(width, grid), "live::settle");
        }
    }
}

#[test]
fn live_wired_refuses_a_solid_grid_by_name() {
    for width in WIDTHS {
        let plan = plan(width);
        assert!(live::wired(width, FLAT, &plan, None).is_ok());
        for grid in solids() {
            refused(live::wired(width, grid, &plan, None), "live::wired");
        }
    }
}

#[test]
fn live_arranged_refuses_a_solid_grid_by_name() {
    let view = View::of(64, 32);
    for width in WIDTHS {
        let plan = plan(width);
        for coarse in [false, true] {
            for picture in [None, Some(view)] {
                assert!(live::arranged(width, FLAT, &plan, picture, coarse).is_ok());
                for grid in solids() {
                    refused(
                        live::arranged(width, grid, &plan, picture, coarse),
                        "live::arranged",
                    );
                }
            }
        }
    }
}

#[test]
fn live_both_refuses_a_solid_grid_by_name() {
    for width in WIDTHS {
        let plan = plan(width);
        assert!(live::both(width, FLAT, &plan, None).is_ok());
        for grid in solids() {
            refused(live::both(width, grid, &plan, None), "live::both");
        }
    }
}

#[test]
fn live_corners_refuses_a_solid_grid_by_name() {
    let mut kernel = Kernel::<U32>::new(Shape::new(4, 4, BINDINGS)).unwrap();
    let quad = kernel.module().constant_u32(7).unwrap();
    let frame = kernel.module().constant_u32(1).unwrap();
    assert!(live::corners::<4>(&mut kernel, FLAT, quad, frame).is_ok());
    for grid in solids() {
        refused(
            live::corners::<4>(&mut kernel, grid, quad, frame),
            "live::corners",
        );
    }
}

#[test]
fn live_wake_refuses_a_solid_grid_by_name() {
    let mut kernel = Kernel::<U32>::new(Shape::new(4, 4, BINDINGS)).unwrap();
    let dump = kernel.module().constant_u32(64).unwrap();
    let (column, row, one) = {
        let mut lanes = kernel.lanes().unwrap();
        (
            lanes.splat_bits::<U32, 4>(2).unwrap(),
            lanes.splat_bits::<U32, 4>(3).unwrap(),
            lanes.splat_bits::<U32, 4>(1).unwrap(),
        )
    };
    let waking = |grid| Waking {
        subgroup: 4,
        grid,
        column,
        row,
    };
    assert!(live::wake::<4>(&mut kernel, waking(FLAT), one, one, dump).is_ok());
    for grid in solids() {
        refused(
            live::wake::<4>(&mut kernel, waking(grid), one, one, dump),
            "live::wake",
        );
    }
}
