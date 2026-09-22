#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod lifting;

use grit::sand::Grid;
use lifting::*;

/// Every rule, walked forward and undone, at depths a run would use.
///
/// `Board::carry` reads the state back after the walk home and returns
/// `Error::Strayed` when a cell does not land on what it started as. Nothing in
/// the suite asserted that it does not, so a rule that stopped being reversible
/// would be caught only where some other test happened to call `carry` and
/// unwrap it. This is the one that asks on purpose.
///
/// It matters beyond the rules that are here. The reconstruction is what buys
/// the constant memory -- the buffers are the same six at depth 1 and at depth
/// 256 because no frame is kept -- so a rule that strays is a rule that cannot
/// be carried back at all.
const DEPTHS: [u32; 4] = [1, 4, 64, 256];

const KINDS: [Kind; 7] = [
    Kind::Product,
    Kind::Sum,
    Kind::Turn,
    Kind::Bound,
    Kind::Smooth,
    Kind::Living,
    Kind::Twinned,
];

fn named(kind: Kind) -> &'static str {
    match kind {
        Kind::Product => "product",
        Kind::Sum => "sum",
        Kind::Turn => "turn",
        Kind::Bound => "bound",
        Kind::Smooth => "smooth",
        Kind::Living => "living",
        Kind::Twinned => "twinned",
    }
}

/// How many cells the grain reaches, by kind and by the depths above.
///
/// Measured, and the same on three devices: a subgroup of 4 under lavapipe, of
/// 32 on an RTX 4080 and of 64 on an AMD iGPU. The arithmetic is whole numbers
/// throughout, so there is nothing here for a device to round differently.
///
/// The table is the point, not decoration. `turn` is `[d, c, b, a]`, a
/// permutation, so it carries the grain somewhere else and never spreads it --
/// one cell at every depth. `bound` and `smooth` stop at 64, which is this
/// grid's width: they reach along one axis and not the other. The rest spread
/// both ways and saturate.
const REACHED: [[usize; 4]; 7] = [
    [4, 25, 4096, 4096],
    [4, 64, 3702, 4096],
    [1, 1, 1, 1],
    [2, 8, 64, 64],
    [2, 8, 64, 64],
    [3, 20, 1727, 4065],
    [3, 20, 1727, 4065],
];

#[test]
fn every_rule_lands_back_on_what_it_started_as() {
    let Some(host) = device("every_rule_lands_back_on_what_it_started_as") else {
        return;
    };
    let grid = Grid::of(WIDE, DEEP);
    let start = spread(grid, 0x1234_5678);
    let middle = (grid.deep / 2 * grid.wide + grid.wide / 2) as usize;

    eprintln!(
        "{:>9} {:>7} {:>9} {:>9}",
        "kind", "depth", "reached", "wanted"
    );
    for (which, kind) in KINDS.into_iter().enumerate() {
        for (nth, depth) in DEPTHS.into_iter().enumerate() {
            let mut board = seated(&host, grid, kind, (TURNING / 64, SWING / 64), depth).unwrap();

            // `slope` is `carry` with a single grain of weight at one cell, and
            // it is the call that comes back `Strayed` when the walk home does
            // not land on what it set out from. Unwrapping it is the assertion:
            // there is no other way for this to be an error.
            let carried = board.slope(&start, middle).unwrap_or_else(|why| {
                panic!(
                    "{} at depth {depth} did not come home: {why:?}",
                    named(kind)
                )
            });

            let reached = carried.iter().filter(|one| **one != 0).count();
            let wanted = REACHED[which][nth];
            eprintln!("{:>9} {depth:>7} {reached:>9} {wanted:>9}", named(kind));
            assert_eq!(
                reached,
                wanted,
                "{} at depth {depth} reaches a different number of cells than it did",
                named(kind)
            );
        }
    }
}
