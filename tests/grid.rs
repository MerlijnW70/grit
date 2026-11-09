#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use grit::sand::{Grid, Unfit};

#[test]
fn a_grid_whose_quads_pass_a_u32_is_refused() {
    assert_eq!(
        Grid::new(131_072, 131_072),
        Err(Unfit::TooMany {
            wide: 131_072,
            deep: 131_072
        })
    );
    assert_eq!(
        Grid::new(100_000, 100_000),
        Err(Unfit::TooMany {
            wide: 131_072,
            deep: 131_072
        })
    );
    assert_eq!(
        Grid::new(1 << 31, 1 << 3),
        Err(Unfit::TooMany {
            wide: 1 << 31,
            deep: 1 << 3
        })
    );
}

#[test]
fn a_side_that_rounds_past_a_u32_is_refused() {
    assert_eq!(
        Grid::new(3_000_000_000, 2),
        Err(Unfit::Side {
            asked: 3_000_000_000
        })
    );
    assert_eq!(
        Grid::new(2, (1 << 31) + 1),
        Err(Unfit::Side {
            asked: (1 << 31) + 1
        })
    );
    assert_eq!(
        Grid::new(u32::MAX, u32::MAX),
        Err(Unfit::Side { asked: u32::MAX })
    );
}

#[test]
fn the_largest_grids_that_still_count_are_taken() {
    let square = Grid::new(65_536, 65_536).unwrap();
    assert_eq!(square.quads(), 1 << 30);
    assert_eq!(square.cells(), 1 << 32);

    let long = Grid::new(131_072, 65_536).unwrap();
    assert_eq!(long.quads(), 1 << 31);

    let widest = Grid::new(1 << 31, 4).unwrap();
    assert_eq!(widest.quads(), 1 << 31);

    let last = Grid::new(1 << 17, 1 << 17);
    assert!(last.is_err());
    let under = Grid::new(1 << 17, 1 << 16).unwrap();
    assert_eq!(u64::from(under.quads()), 1 << 31);
    let edge = Grid::new(1 << 18, 1 << 16).unwrap_err();
    assert_eq!(
        edge,
        Unfit::TooMany {
            wide: 1 << 18,
            deep: 1 << 16
        }
    );
}

#[test]
fn a_grid_without_a_quad_is_refused() {
    for (wide, deep) in [(0, 0), (1, 64), (64, 1), (0, 2)] {
        let rounded = (
            wide.max(1_u32).next_power_of_two(),
            deep.max(1_u32).next_power_of_two(),
        );
        assert_eq!(
            Grid::new(wide, deep),
            Err(Unfit::Empty {
                wide: rounded.0,
                deep: rounded.1
            }),
            "{wide} x {deep}"
        );
    }
    assert_eq!(Grid::new(2, 2).unwrap().quads(), 1);
}

#[test]
fn a_grid_that_fits_is_the_one_of_gives() {
    for (wide, deep) in [
        (2, 2),
        (3, 7),
        (300, 7),
        (512, 256),
        (4096, 4096),
        (65_535, 3),
    ] {
        assert_eq!(
            Grid::new(wide, deep),
            Ok(Grid::of(wide, deep)),
            "{wide} x {deep}"
        );
    }
}

#[test]
fn a_refusal_says_what_did_not_fit() {
    let side = Unfit::Side {
        asked: 3_000_000_000,
    }
    .to_string();
    assert!(side.contains("3000000000"), "{side}");
    let many = Unfit::TooMany {
        wide: 131_072,
        deep: 131_072,
    }
    .to_string();
    assert!(many.contains("131072 by 131072"), "{many}");
    let empty = Unfit::Empty { wide: 1, deep: 64 }.to_string();
    assert!(empty.contains("1 by 64"), "{empty}");
    let boxed: Box<dyn std::error::Error> = Box::new(Unfit::Empty { wide: 1, deep: 1 });
    assert!(!boxed.to_string().is_empty());
}
