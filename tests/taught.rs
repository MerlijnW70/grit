#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod lifting;

use grit::Groups;
use grit::sand::Grid;
use lifting::*;

const SIDE: u32 = 64;

const DEPTH: u32 = 40;

const BLOB: u32 = 6;

const MASS: u32 = 1 << 14;

const GROUND: u32 = 1 << 11;

const READ: u32 = 8;

const ROUNDS: u32 = 150;

const PACE: i64 = 4;

fn blotted(grid: Grid, left: bool, seed: u32) -> Vec<u32> {
    let mut state = seed | 1;
    let mut roll = || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state
    };
    let across = if left { 6 } else { grid.wide - 6 - BLOB };
    let down = 6 + roll() % (grid.deep - 12 - BLOB);
    let mut cells = vec![GROUND; grid.cells()];
    for row in down..down + BLOB {
        for column in across..across + BLOB {
            cells[(row * grid.wide + column) as usize] = MASS;
        }
    }
    cells
}

fn window(grid: Grid) -> Vec<usize> {
    let from = grid.wide / 2 - READ / 2;
    let mut held = Vec::new();
    for row in 0..grid.deep {
        for column in from..from + READ {
            held.push((row * grid.wide + column) as usize);
        }
    }
    held
}

fn read_out(ended: &[u32], seen: &[usize]) -> f64 {
    seen.iter()
        .map(|at| f64::from(ended[*at] as i32))
        .sum::<f64>()
        / seen.len() as f64
}

struct Lesson {
    start: Vec<u32>,
    left: bool,
}

fn lessons(grid: Grid, from: u32, count: u32) -> Vec<Lesson> {
    (0..count)
        .map(|at| {
            let left = at % 2 == 0;
            Lesson {
                start: blotted(grid, left, from + at * 7919),
                left,
            }
        })
        .collect()
}

const HIGH: f64 = 1600.0;

fn wanted(left: bool) -> f64 {
    if left { HIGH } else { 0.0 }
}

fn spread_of(board: &mut Board<'_>, held: &[Lesson], seen: &[usize], left: bool) -> (f64, f64) {
    let mut low = f64::MAX;
    let mut high = f64::MIN;
    for one in held.iter().filter(|one| one.left == left) {
        let got = read_out(&board.run(&one.start), seen);
        low = low.min(got);
        high = high.max(got);
    }
    (low, high)
}

fn parted(board: &mut Board<'_>, held: &[Lesson], seen: &[usize]) -> (f64, f64) {
    let mut left = (0.0_f64, 0.0_f64);
    let mut right = (0.0_f64, 0.0_f64);
    for one in held {
        let got = read_out(&board.run(&one.start), seen);
        if one.left {
            left = (left.0 + got, left.1 + 1.0);
        } else {
            right = (right.0 + got, right.1 + 1.0);
        }
    }
    (left.0 / left.1.max(1.0), right.0 / right.1.max(1.0))
}

fn scored(board: &mut Board<'_>, held: &[Lesson], seen: &[usize], line: f64) -> f64 {
    let mut right = 0.0;
    for one in held {
        let got = read_out(&board.run(&one.start), seen);
        if (got > line) == one.left {
            right += 1.0;
        }
    }
    right / held.len() as f64
}

fn told_on(board: &mut Board<'_>, held: &[Lesson], seen: &[usize]) -> f64 {
    let mut loss = 0.0;
    for one in held {
        let got = read_out(&board.run(&one.start), seen);
        let want = wanted(one.left);
        loss += (got - want) * (got - want);
    }
    loss / held.len() as f64
}

#[test]
fn where_the_blot_has_got_to_by_each_depth() {
    let Some(host) = device("where_the_blot_has_got_to_by_each_depth") else {
        return;
    };
    let width = host.width();
    let grid = Grid::of(SIDE, SIDE);
    let sizes = [
        8,
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
    ];
    let passes = [
        built(width, grid, Pass::Ahead, Kind::Living).unwrap(),
        built(width, grid, Pass::Back, Kind::Living).unwrap(),
    ];
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let resident = host.resident(&sizes, &modules).unwrap();
    let base = 256_u32;
    let mut board = Board {
        resident,
        over: Groups::Fixed(grid.quads().div_ceil(width)),
        grid,
        depth: 1,
        angle: (base, 0),
        loud: 1,
        planted: false,
    };
    board.tune(base, 0);

    let seen = window(grid);
    let left = blotted(grid, true, 0x1234_5678);
    let right = blotted(grid, false, 0x1234_5678);

    eprintln!(
        "{:>7} {:>14} {:>14} {:>14}",
        "steps", "left reads", "right reads", "apart"
    );
    for depth in [4_u32, 8, 16, 24, 32, 40, 48, 56, 64] {
        board.depth = depth;
        let a = read_out(&board.run(&left), &seen);
        let b = read_out(&board.run(&right), &seen);
        eprintln!("{depth:>7} {a:>14.0} {b:>14.0} {:>14.0}", (a - b).abs());
    }
}

#[test]
fn a_field_of_knobs_is_taught_to_say_which_side_the_blot_was_on() {
    let Some(host) = device("a_field_of_knobs_is_taught_to_say_which_side_the_blot_was_on") else {
        return;
    };
    let width = host.width();
    let grid = Grid::of(SIDE, SIDE);
    let sizes = [
        8,
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
    ];
    let passes = [
        built(width, grid, Pass::Ahead, Kind::Living).unwrap(),
        built(width, grid, Pass::Back, Kind::Living).unwrap(),
    ];
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let resident = host.resident(&sizes, &modules).unwrap();

    let base = 256_u32;
    let mut board = Board {
        resident,
        over: Groups::Fixed(grid.quads().div_ceil(width)),
        grid,
        depth: DEPTH,
        angle: (base, 0),
        loud: 1,
        planted: false,
    };

    let seen = window(grid);
    let taught = lessons(grid, 0x1234_5678, 8);
    let fresh = lessons(grid, 0x0BAD_C0DE, 8);

    let mut knobs = vec![base; grid.cells()];
    let judge = |board: &mut Board<'_>, knobs: &[u32]| {
        board.sow(knobs);
        let loss = told_on(board, &taught, &seen);
        let (left, right) = parted(board, &taught, &seen);
        let line = (left + right) / 2.0;
        let on_taught = scored(board, &taught, &seen, line);
        let on_fresh = scored(board, &fresh, &seen, line);
        (loss, left - right, on_taught, on_fresh)
    };

    let (loss_first, gap_first, taught_first, fresh_first) = judge(&mut board, &knobs);
    eprintln!(
        "{:>7} {:>13} {:>9} {:>9} {:>9}",
        "round", "loss", "gap", "taught", "fresh"
    );
    eprintln!(
        "{:>7} {loss_first:>13.3e} {gap_first:>9.0} {:>8.0}% {:>8.0}%",
        "start",
        taught_first * 100.0,
        fresh_first * 100.0
    );

    for round in 0..ROUNDS {
        let mut piled = vec![0_i64; grid.cells()];
        for one in &taught {
            board.sow(&knobs);
            let ended = board.run(&one.start);
            let got = read_out(&ended, &seen);
            let off = got - wanted(one.left);
            let share = (off / seen.len() as f64) as i32;
            let mut weight = vec![0_u32; grid.cells()];
            for at in &seen {
                weight[*at] = share as u32;
            }
            board.sow(&knobs);
            board.carry(&one.start, &weight);
            let scale = f64::from(1_u32 << SPARE) * f64::from(board.loud);
            for (into, slope) in piled.iter_mut().zip(board.told_each()) {
                *into += (f64::from(slope) / scale) as i64;
            }
        }
        if round == 0 {
            let spoke = piled.iter().filter(|one| **one != 0).count();
            let widest = piled.iter().map(|one| one.abs()).max().unwrap_or(0);
            let piled_sum: i64 = piled.iter().map(|one| one.abs()).sum();
            eprintln!(
                "round 0: {spoke} of {} knobs have a slope, widest {widest}, total {piled_sum}",
                piled.len()
            );
        }
        for (knob, slope) in knobs.iter_mut().zip(&piled) {
            let moved = i64::from(*knob) - slope.signum() * PACE;
            *knob = moved.clamp(1, 4096) as u32;
        }
        if round % 30 == 29 {
            let (loss, gap, on_taught, on_fresh) = judge(&mut board, &knobs);
            eprintln!(
                "{round:>7} {loss:>13.3e} {gap:>9.0} {:>8.0}% {:>8.0}%",
                on_taught * 100.0,
                on_fresh * 100.0
            );
        }
    }

    let (loss_last, gap_last, taught_last, fresh_last) = judge(&mut board, &knobs);
    eprintln!("loss {loss_first:.3e} to {loss_last:.3e}, gap {gap_first:.0} to {gap_last:.0}");
    board.sow(&knobs);
    let lows = spread_of(&mut board, &taught, &seen, true);
    let highs = spread_of(&mut board, &taught, &seen, false);
    eprintln!(
        "left reads {:.0} to {:.0}, right reads {:.0} to {:.0}, and they overlap by {:.0}",
        lows.0,
        lows.1,
        highs.0,
        highs.1,
        (lows.1.min(highs.1) - lows.0.max(highs.0)).max(0.0)
    );
    eprintln!(
        "taught {:.0}% to {:.0}%, fresh {:.0}% to {:.0}%",
        taught_first * 100.0,
        taught_last * 100.0,
        fresh_first * 100.0,
        fresh_last * 100.0
    );
    eprintln!(
        "it read them apart before any of this, so what fell is the loss and not the ignorance"
    );
    assert!(loss_last < loss_first, "{loss_first} {loss_last}");
    assert!(gap_last > gap_first, "{gap_first} {gap_last}");
    assert!(
        (taught_first - 1.0).abs() < f64::EPSILON,
        "the task is only worth keeping while the untaught field already answers it: {taught_first}"
    );
}

#[test]
fn how_far_a_slope_from_a_small_window_reaches() {
    let Some(host) = device("how_far_a_slope_from_a_small_window_reaches") else {
        return;
    };
    let width = host.width();
    let grid = Grid::of(SIDE, SIDE);
    let sizes = [
        8,
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
    ];
    let passes = [
        built(width, grid, Pass::Ahead, Kind::Living).unwrap(),
        built(width, grid, Pass::Back, Kind::Living).unwrap(),
    ];
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let resident = host.resident(&sizes, &modules).unwrap();
    let base = 256_u32;
    let mut board = Board {
        resident,
        over: Groups::Fixed(grid.quads().div_ceil(width)),
        grid,
        depth: 1,
        angle: (base, 0),
        loud: 1,
        planted: false,
    };
    board.tune(base, 0);

    let seen = window(grid);
    let start = blotted(grid, true, 0x1234_5678);

    eprintln!(
        "{:>7} {:>10} {:>12} {:>10}",
        "steps", "knobs told", "widest", "weight in"
    );
    let mut told = Vec::new();
    for depth in [1_u32, 2, 4, 8, 16, 40] {
        board.depth = depth;
        let mut weight = vec![0_u32; grid.cells()];
        for at in &seen {
            weight[*at] = (-11_i32) as u32;
        }
        board.carry(&start, &weight);
        let held = board.told_each();
        let spoke = held.iter().filter(|one| **one != 0).count();
        let widest = held.iter().map(|one| one.abs()).max().unwrap_or(0);
        eprintln!("{depth:>7} {spoke:>10} {widest:>12} {:>10}", board.loud);
        told.push((depth, spoke));
    }

    for (depth, spoke) in &told {
        eprintln!(
            "{depth:>3} steps: {spoke} of {} knobs hear it",
            grid.cells()
        );
    }
    let near = told.first().map_or(0, |(_, spoke)| *spoke);
    let far = told.last().map_or(0, |(_, spoke)| *spoke);
    eprintln!("from {near} at one step to {far} at forty");
}
