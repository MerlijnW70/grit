#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use grit::board::{Board, seated};
use grit::rule::{Kind, POINT};
use grit::sand::Grid;

const SIDE: u32 = 8;

const DEPTHS: [u32; 5] = [1, 2, 4, 8, 16];

const EDGES: [i32; 32] = [
    0,
    1,
    -1,
    i32::MIN,
    i32::MAX,
    i32::MIN + 1,
    i32::MAX - 1,
    46_340,
    -46_340,
    46_341,
    -46_341,
    65_536,
    -65_536,
    32_768,
    1 << 20,
    -(1 << 20),
    (1 << 20) - 1,
    -(1 << 20) + 1,
    -(1 << 20) - 1,
    1_024,
    -1_024,
    1_023,
    -1_023,
    1_025,
    -1_025,
    15,
    16,
    17,
    -15,
    -16,
    -17,
    -2,
];

const MILD: i32 = 1 << 14;

const NUDGE: i32 = 1 << 12;

const LEAP: i64 = 8;

fn grid() -> Grid {
    Grid::of(SIDE, SIDE)
}

fn mixed(first: i32, second: i32, third: i32) -> i32 {
    (first.wrapping_mul(second) >> 20).wrapping_add(third >> 4)
}

fn quads(frame: u32) -> Vec<[usize; 4]> {
    let phase = frame & 1;
    let at = |column: u32, row: u32| ((row % SIDE) * SIDE + column % SIDE) as usize;
    let mut all = Vec::new();
    for row in 0..SIDE / 2 {
        for column in 0..SIDE / 2 {
            let x = 2 * column + phase;
            let y = 2 * row + phase;
            all.push([at(x, y), at(x + 1, y), at(x, y + 1), at(x + 1, y + 1)]);
        }
    }
    all
}

fn modelled(start: &[i32], depth: u32) -> Vec<i32> {
    let mut cells = start.to_vec();
    for frame in 0..depth {
        for [ia, ib, ic, id] in quads(frame) {
            let (mut a, mut b, mut c, mut d) = (cells[ia], cells[ib], cells[ic], cells[id]);
            a = a.wrapping_add(mixed(b, c, d));
            b = b.wrapping_add(mixed(c, d, a));
            c = c.wrapping_add(mixed(d, a, b));
            d = d.wrapping_add(mixed(a, b, c));
            cells[ia] = a;
            cells[ib] = b;
            cells[ic] = c;
            cells[id] = d;
        }
    }
    cells
}

fn unwrapped(start: &[i32], depth: u32) -> bool {
    let fits = |value: i64| i32::try_from(value).is_ok();
    let mixed_wide = |first: i64, second: i64, third: i64| {
        let product = first * second;
        (fits(product), (product >> 20) + (third >> 4))
    };
    let mut cells: Vec<i64> = start.iter().map(|one| i64::from(*one)).collect();
    for frame in 0..depth {
        for [ia, ib, ic, id] in quads(frame) {
            let mut block = [cells[ia], cells[ib], cells[ic], cells[id]];
            for (target, [first, second, third]) in [
                (0, [1, 2, 3]),
                (1, [2, 3, 0]),
                (2, [3, 0, 1]),
                (3, [0, 1, 2]),
            ] {
                let (held, step) = mixed_wide(block[first], block[second], block[third]);
                block[target] += step;
                if !held || !fits(step) || !fits(block[target]) {
                    return false;
                }
            }
            [cells[ia], cells[ib], cells[ic], cells[id]] = block;
        }
    }
    true
}

fn rolled(seed: u32, count: usize, mut pick: impl FnMut(u32) -> i32) -> Vec<i32> {
    let mut state = seed;
    (0..count)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            pick(state)
        })
        .collect()
}

fn mild(seed: u32) -> Vec<i32> {
    let span = (2 * MILD + 1) as u32;
    rolled(seed, grid().cells(), |roll| (roll % span) as i32 - MILD)
}

fn edged() -> Vec<Vec<i32>> {
    let cells = grid().cells();
    let count = EDGES.len();
    let mut all: Vec<Vec<i32>> = [1_usize, 3, 5, 7, 11, 13]
        .into_iter()
        .map(|stride| (0..cells).map(|at| EDGES[(at * stride) % count]).collect())
        .collect();
    for seed in [0x1234_5678_u32, 0x0BAD_C0DE, 0xDEAD_BEEF, 0x5EED_0001] {
        all.push(rolled(seed, cells, |roll| EDGES[roll as usize % count]));
    }
    all.push(rolled(0x2545_F491, cells, |roll| roll as i32));
    all.push(mild(0x0F0F_F0F0));
    all
}

fn words(cells: &[i32]) -> Vec<u32> {
    cells.iter().map(|one| *one as u32).collect()
}

fn signed(cells: &[u32]) -> Vec<i32> {
    cells.iter().map(|one| *one as i32).collect()
}

fn walked(board: &mut Board<'_>, cells: &[i32]) -> Vec<i32> {
    signed(&board.run(&words(cells)).unwrap())
}

#[test]
fn the_product_step_on_the_device_is_the_signed_model_bit_for_bit() {
    let Some(host) = device("the_product_step_on_the_device_is_the_signed_model_bit_for_bit")
    else {
        return;
    };
    let mut board = seated(&host, grid(), Kind::Product, (0, 0), 1).unwrap();
    let starts = edged();
    for depth in DEPTHS {
        board.depth = depth;
        for (nth, start) in starts.iter().enumerate() {
            let ended = walked(&mut board, start);
            let wanted = modelled(start, depth);
            let differ = ended
                .iter()
                .zip(&wanted)
                .enumerate()
                .find(|(_, (one, other))| one != other);
            if let Some((at, (one, other))) = differ {
                panic!(
                    "grid {nth} at depth {depth}: cell {at} is {one} on the device and {other} in the signed model"
                );
            }
        }
    }
}

#[test]
fn the_product_adjoint_is_the_slope_finite_differences_measure() {
    let Some(host) = device("the_product_adjoint_is_the_slope_finite_differences_measure") else {
        return;
    };
    let mut board = seated(&host, grid(), Kind::Product, (8303, 16384), 1).unwrap();
    let start = mild(0x1234_5678);
    let watched = [
        (SIDE / 2 * SIDE + SIDE / 2) as usize,
        0,
        (3 * SIDE + SIDE - 2) as usize,
    ];
    for depth in DEPTHS {
        board.depth = depth;
        let grain = 4 * i64::from(depth) + 16;
        for nudge in [-NUDGE, 0, NUDGE] {
            let shifted: Vec<i32> = start.iter().map(|one| one + nudge).collect();
            assert!(
                unwrapped(&shifted, depth),
                "the run from the gradient grid moved by {nudge} wraps at depth {depth}"
            );
        }
        let mut slopes = Vec::new();
        for at in watched {
            slopes.push(signed(&board.slope(&words(&start), at).unwrap()));
            let (turning, swing, _) = board.told_parts().unwrap();
            assert_eq!((turning, swing), (0, 0), "depth {depth}");
        }
        let mut rises = Vec::new();
        for cell in 0..start.len() {
            let mut up = start.clone();
            up[cell] += NUDGE;
            let mut down = start.clone();
            down[cell] -= NUDGE;
            let high = walked(&mut board, &up);
            let low = walked(&mut board, &down);
            rises.push((high, low));
        }
        for (at, carried) in watched.into_iter().zip(&slopes) {
            let mut resolved = 0;
            let mut worst = 0.0_f64;
            for (cell, (told, (high, low))) in carried.iter().zip(&rises).enumerate() {
                let rise = i64::from(high[at]) - i64::from(low[at]);
                let measured = (rise << POINT) / (2 * i64::from(NUDGE));
                let told = i64::from(*told);
                let apart = (told - measured).abs();
                let large = told.abs().max(measured.abs());
                if large >= 100 * grain {
                    resolved += 1;
                    let share = apart as f64 / large as f64;
                    worst = worst.max(share);
                    assert!(
                        told.signum() == measured.signum() && 100 * apart <= large,
                        "depth {depth}, watching {at}, cell {cell}: the adjoint says {told} and the nudge measures {measured}"
                    );
                } else {
                    assert!(
                        apart <= 2 * grain,
                        "depth {depth}, watching {at}, cell {cell}: the adjoint says {told} and the nudge measures {measured}, below the grain of {grain}"
                    );
                }
            }
            eprintln!("depth {depth} watching {at}: {resolved} resolved, worst {worst:.4}");
            assert!(resolved >= 2, "depth {depth}, watching {at}: {resolved}");
        }
        let mut turned = seated(&host, grid(), Kind::Product, (0, 0), depth).unwrap();
        assert_eq!(
            walked(&mut turned, &start),
            walked(&mut board, &start),
            "turning and swing move nothing under product at depth {depth}"
        );
    }
}

#[test]
fn a_one_unit_change_moves_the_product_outcome_without_a_jump() {
    let Some(host) = device("a_one_unit_change_moves_the_product_outcome_without_a_jump") else {
        return;
    };
    let mut board = seated(&host, grid(), Kind::Product, (0, 0), 1).unwrap();
    let start = mild(0x0BAD_C0DE);
    let swept = [
        (SIDE / 2 * SIDE + SIDE / 2) as usize,
        (SIDE / 2 * SIDE + SIDE / 2 + 1) as usize,
        0,
        (SIDE * SIDE - 1) as usize,
    ];
    let leap = |one: &[i32], other: &[i32]| {
        one.iter()
            .zip(other)
            .map(|(a, b)| (i64::from(*a) - i64::from(*b)).abs())
            .max()
            .unwrap_or(0)
    };
    for depth in DEPTHS {
        board.depth = depth;
        let mut worst = 0;
        for cell in swept {
            let mut before: Option<Vec<i32>> = None;
            for value in -48..=48 {
                let mut cells = start.clone();
                cells[cell] = value;
                assert!(unwrapped(&cells, depth), "{cell} {value} {depth}");
                let ended = walked(&mut board, &cells);
                if let Some(held) = &before {
                    let moved = leap(&ended, held);
                    worst = worst.max(moved);
                    assert!(
                        moved <= LEAP,
                        "depth {depth}: moving cell {cell} from {} to {value} moved an outcome by {moved}",
                        value - 1
                    );
                }
                before = Some(ended);
            }
        }
        eprintln!("depth {depth}: largest move {worst}");

        let cell = swept[0];
        let mut before: Option<(i32, Vec<i32>)> = None;
        for value in [
            i32::MAX - 2,
            i32::MAX - 1,
            i32::MAX,
            i32::MIN,
            i32::MIN + 1,
            i32::MIN + 2,
        ] {
            let mut cells = start.clone();
            cells[cell] = value;
            let ended = walked(&mut board, &cells);
            assert_eq!(ended, modelled(&cells, depth), "{value} {depth}");
            if let Some((was, held)) = &before
                && *was == i32::MAX
            {
                let moved = leap(&ended, held);
                assert!(
                    moved > LEAP,
                    "depth {depth}: the 32-bit wrap from {was} to {value} is the model's own discontinuity and moved only {moved}"
                );
            }
            before = Some((value, ended));
        }
    }
}
