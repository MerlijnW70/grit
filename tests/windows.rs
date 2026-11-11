#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use grit::layout::{BINDINGS, STATE, WINDOW_BITS, Windows};
use grit::live;
use grit::rule3::nature::Nature;
use grit::rule3::sand::Sand;
use grit::sand::Grid;

#[test]
fn state_splits_once_it_holds_four_gibibytes_into_windows_of_two() {
    assert_eq!(WINDOW_BITS, 29);
    let under = Windows::of((1 << 30) - 1);
    assert!(!under.is_split());
    assert_eq!(under.count(), 1);
    assert_eq!(under.bindings(), BINDINGS);
    assert_eq!(under.place((1 << 30) - 2), (STATE, (1 << 30) - 2));
    assert_eq!(under.held(0), (1 << 30) - 1);
    assert_eq!(
        under.laid(&[5; BINDINGS as usize]),
        vec![5; BINDINGS as usize]
    );

    let split = Windows::of(1 << 30);
    assert!(split.is_split());
    assert_eq!(split.count(), 2);
    assert_eq!(split.span(), 1 << 29);
    assert_eq!(split.bindings(), BINDINGS + 1);
    assert_eq!(split.place((1 << 29) - 1), (STATE, (1 << 29) - 1));
    assert_eq!(split.place(1 << 29), (BINDINGS, 0));
    let laid = split.laid(&[5; BINDINGS as usize]);
    assert_eq!(laid.len(), BINDINGS as usize + 1);
    assert_eq!(laid[STATE as usize], 1 << 29);
    assert_eq!(laid[BINDINGS as usize], 1 << 29);

    let four = Windows::of(1 << 31);
    assert_eq!(four.count(), 4);
    assert_eq!(four.place((1 << 31) - 1), (BINDINGS + 2, (1 << 29) - 1));
    assert_eq!(Windows::binding(0), STATE);
    assert_eq!(Windows::binding(3), BINDINGS + 2);
}

#[test]
fn a_world_in_one_binding_builds_the_settle_it_always_built() {
    for width in [4_u32, 32] {
        for side in [16_u32, 64, 256] {
            let grid = Grid::solid(side, side, side).unwrap();
            let windows = Windows::of(grid.cells());
            assert!(!windows.is_split());
            assert_eq!(
                live::settle3(width, grid).unwrap(),
                live::settle3_in(&Sand, width, grid, windows).unwrap()
            );
            assert_eq!(
                live::settle3_by(&Nature, width, grid).unwrap(),
                live::settle3_in(&Nature, width, grid, windows).unwrap()
            );
        }
    }
}

/// Whether `words`, a module, decorates a variable with binding `binding`.
fn declares(words: &[u32], binding: u32) -> bool {
    const DECORATE: u32 = (4 << 16) | 71;
    const BINDING: u32 = 33;
    words
        .windows(4)
        .any(|held| held[0] == DECORATE && held[2] == BINDING && held[3] == binding)
}

#[test]
fn a_split_world_builds_a_settle_that_declares_a_binding_a_window() {
    for (side, count) in [(512_u32, 1_u32), (1024, 2)] {
        let grid = Grid::solid(side, side, side).unwrap();
        let windows = Windows::of(grid.cells());
        assert_eq!(windows.count(), count);
        let built = live::settle3_in(&Sand, 32, grid, windows).unwrap();
        assert!(declares(&built, BINDINGS - 1));
        for extra in 0..2 {
            assert_eq!(
                declares(&built, BINDINGS + extra),
                extra + 1 < count,
                "{side}: binding {}",
                BINDINGS + extra
            );
        }
    }
}
