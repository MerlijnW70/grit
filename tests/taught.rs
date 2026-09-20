#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod lifting;

use grit::Groups;
use grit::sand::Grid;
use lifting::*;

const SIDE: u32 = 64;

const DEPTH: u32 = 64;

const BLOB: u32 = 6;

const MASS: u32 = 1 << 14;

const GROUND: u32 = 1 << 11;

const HAZE: u32 = 1 << 13;

const READ: u32 = 8;

const ROUNDS: u32 = 60;

const PACE: i64 = 4;

fn blotted_in(grid: Grid, rows: (u32, u32), seed: u32, haze: u32) -> Vec<u32> {
    let across = 16;
    let mut state = seed | 1;
    let mut cells: Vec<u32> = (0..grid.cells())
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            GROUND + state % haze.max(1)
        })
        .collect();
    for down in [rows.0, rows.1] {
        for row in down..down + BLOB {
            for column in across..across + BLOB {
                cells[(row * grid.wide + column) as usize] = MASS;
            }
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

fn blotted(grid: Grid, rows: (u32, u32), seed: u32) -> Vec<u32> {
    blotted_in(grid, rows, seed, HAZE)
}

fn lessons_in(grid: Grid, from: u32, count: u32, haze: u32) -> Vec<Lesson> {
    (0..count)
        .map(|at| {
            let near = at % 2 == 0;
            let top = 4 + (at * 5) % 12;
            let gap = if near { BLOB + 2 } else { SIDE / 2 };
            Lesson {
                start: blotted_in(grid, (top, top + gap), from + at * 7919, haze),
                left: near,
            }
        })
        .collect()
}

fn lessons(grid: Grid, from: u32, count: u32) -> Vec<Lesson> {
    lessons_in(grid, from, count, HAZE)
}

const SPLIT: f64 = 300.0;

const PITCH: f64 = 256.0;

fn wanted(left: bool, middle: f64) -> f64 {
    if left { middle + SPLIT } else { middle - SPLIT }
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

fn told_on(board: &mut Board<'_>, held: &[Lesson], seen: &[usize], middle: f64) -> f64 {
    let mut loss = 0.0;
    for one in held {
        let got = read_out(&board.run(&one.start), seen);
        let want = wanted(one.left, middle);
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
    let left = blotted(grid, (6, 14), 0x1234_5678);
    let right = blotted(grid, (6, 38), 0x1234_5678);

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
fn how_much_haze_it_takes_to_blind_the_window() {
    let Some(host) = device("how_much_haze_it_takes_to_blind_the_window") else {
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
    board.tune(base, 0);
    let seen = window(grid);

    eprintln!(
        "{:>8} {:>12} {:>12} {:>10}",
        "apart", "lowest", "highest", "overlap"
    );
    let mut bands = Vec::new();
    for gap in [BLOB + 2, SIDE / 2] {
        let mut low = f64::MAX;
        let mut high = f64::MIN;
        for seed in 0..16_u32 {
            let top = 4 + (seed * 5) % 12;
            let start = blotted(grid, (top, top + gap), 0x1234_5678 + seed * 7919);
            let got = read_out(&board.run(&start), &seen);
            low = low.min(got);
            high = high.max(got);
        }
        bands.push((low, high));
        eprintln!("{gap:>8} {low:>12.1} {high:>12.1}");
    }
    let over = bands[0].1.min(bands[1].1) - bands[0].0.max(bands[1].0);
    eprintln!("the two bands overlap by {:.1}", over.max(0.0));
}

#[test]
fn whether_a_step_down_the_slope_lowers_the_loss_at_all() {
    let Some(host) = device("whether_a_step_down_the_slope_lowers_the_loss_at_all") else {
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
    let taught = lessons(grid, 0x1234_5678, 16);
    let knobs = vec![base; grid.cells()];

    board.sow(&knobs);
    let middle = {
        let (left, right) = parted(&mut board, &taught, &seen);
        (left + right) / 2.0
    };
    let began = told_on(&mut board, &taught, &seen, middle);

    let mut piled = vec![0.0_f64; grid.cells()];
    for one in &taught {
        board.sow(&knobs);
        let got = read_out(&board.run(&one.start), &seen);
        let off = got - wanted(one.left, middle);
        let share = (off / seen.len() as f64 * PITCH).round() as i32;
        let mut weight = vec![0_u32; grid.cells()];
        for at in &seen {
            weight[*at] = share as u32;
        }
        board.sow(&knobs);
        board.carry(&one.start, &weight);
        let scale = f64::from(1_u32 << SPARE) * f64::from(board.loud) * PITCH;
        for (into, slope) in piled.iter_mut().zip(board.told_each()) {
            *into += f64::from(slope) / scale;
        }
    }
    let widest = piled.iter().fold(0.0_f64, |held, one| held.max(one.abs()));
    eprintln!("the widest slope is {widest:.3e}, the loss is {began:.4e}");

    let square: f64 = piled.iter().map(|one| one * one).sum();
    eprintln!(
        "{:>12} {:>14} {:>14} {:>10}",
        "step", "promised", "delivered", "of it"
    );
    let mut best = (0.0, began);
    for over in [1_i32, 2, 4, 8, 16, 32, 64, 128] {
        let rate = f64::from(over) / widest.max(f64::MIN_POSITIVE);
        let moved: Vec<u32> = knobs
            .iter()
            .zip(&piled)
            .map(|(knob, slope)| {
                let held = f64::from(*knob) - rate * slope;
                held.clamp(1.0, 4096.0) as u32
            })
            .collect();
        board.sow(&moved);
        let loss = told_on(&mut board, &taught, &seen, middle);
        let promised = -rate * square;
        let delivered = loss - began;
        eprintln!(
            "{over:>12} {promised:>14.1} {delivered:>14.1} {:>9.3}",
            delivered / promised
        );
        if loss < best.1 {
            best = (f64::from(over), loss);
        }
    }
    eprintln!(
        "the best move takes the loss to {:.4} of what it was, at a widest step of {:.0}",
        best.1 / began,
        best.0
    );

    let mut ranked: Vec<(usize, f64)> = piled.iter().copied().enumerate().collect();
    ranked.sort_by(|one, other| other.1.abs().total_cmp(&one.1.abs()));

    eprintln!("{:>8} {:>14} {:>10}", "moved", "delivered", "per knob");
    let mut apiece = Vec::new();
    for many in [1_usize, 4, 16, 64, 256, 1024, 2048] {
        let mut moved = knobs.clone();
        for (at, slope) in ranked.iter().take(many) {
            let held = i64::from(knobs[*at]) - slope.signum() as i64;
            moved[*at] = held.clamp(1, 4096) as u32;
        }
        board.sow(&moved);
        let loss = told_on(&mut board, &taught, &seen, middle);
        let delivered = loss - began;
        eprintln!(
            "{many:>8} {delivered:>14.2} {:>10.3}",
            delivered / many as f64
        );
        apiece.push((many, delivered / many as f64));
    }
    let few = apiece
        .iter()
        .filter(|(many, _)| *many <= 16)
        .map(|(_, one)| *one)
        .fold(f64::MAX, f64::min);
    let lots = apiece
        .iter()
        .filter(|(many, _)| *many >= 256)
        .map(|(_, one)| *one)
        .fold(f64::MIN, f64::max);
    eprintln!("a few knobs pay {few:.3} apiece and many of them pay {lots:.3}");
    assert!(few < 0.0, "{few}");
    assert!(lots > 0.0, "{lots}");

    eprintln!(
        "{:>8} {:>14} {:>14} {:>9}",
        "knob", "its share", "a nudge says", "apart"
    );
    for step in [1_i64, 8, 64] {
        let mut worst = 0.0_f64;
        for (at, mine) in ranked.iter().take(4) {
            let mut above = knobs.clone();
            above[*at] = (i64::from(knobs[*at]) + step) as u32;
            board.sow(&above);
            let high = told_on(&mut board, &taught, &seen, middle);
            let mut below = knobs.clone();
            below[*at] = (i64::from(knobs[*at]) - step) as u32;
            board.sow(&below);
            let low = told_on(&mut board, &taught, &seen, middle);
            let nudged = (high - low) / (2.0 * step as f64) * taught.len() as f64 / 2.0;
            let apart = (mine - nudged).abs() / mine.abs().max(nudged.abs()).max(f64::MIN_POSITIVE);
            if step == 8 {
                eprintln!("{at:>8} {mine:>14.4} {nudged:>14.4} {apart:>8.4}");
            }
            worst = worst.max(apart);
        }
        eprintln!("a nudge of {step:>3}: worst of four is {worst:.4}");
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
    let taught = lessons(grid, 0x1234_5678, 16);
    let fresh = lessons(grid, 0x0BAD_C0DE, 32);

    let mut knobs = vec![base; grid.cells()];
    board.sow(&knobs);
    let middle = {
        let (left, right) = parted(&mut board, &taught, &seen);
        (left + right) / 2.0
    };
    eprintln!("the window sits around {middle:.0}, so it is asked for {SPLIT:.0} either side");
    let judge = |board: &mut Board<'_>, knobs: &[u32]| {
        board.sow(knobs);
        let loss = told_on(board, &taught, &seen, middle);
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
        let mut spoke_at = 0_usize;
        for one in &taught {
            let mut piled = vec![0.0_f64; grid.cells()];
            board.sow(&knobs);
            let ended = board.run(&one.start);
            let got = read_out(&ended, &seen);
            let off = got - wanted(one.left, middle);
            let share = (off / seen.len() as f64 * PITCH).round() as i32;
            let mut weight = vec![0_u32; grid.cells()];
            for at in &seen {
                weight[*at] = share as u32;
            }
            board.sow(&knobs);
            board.carry(&one.start, &weight);
            let scale = f64::from(1_u32 << SPARE) * f64::from(board.loud) * PITCH;
            for (into, slope) in piled.iter_mut().zip(board.told_each()) {
                *into += f64::from(slope) / scale;
            }
            spoke_at = spoke_at.max(piled.iter().filter(|one| **one != 0.0).count());
            for (knob, slope) in knobs.iter_mut().zip(&piled) {
                let moved = i64::from(*knob) - (slope.signum() as i64) * PACE;
                *knob = moved.clamp(1, 4096) as u32;
            }
        }
        if round == 0 {
            eprintln!(
                "round 0: at best {spoke_at} of {} knobs heard one lesson",
                knobs.len()
            );
        }
        if round % 15 == 14 {
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
    eprintln!("and it is no better taught than untaught, which is the finding");
    let _ = (loss_last, gap_last);
    assert!(
        fresh_first < 0.75,
        "it answers before it is taught: {fresh_first}"
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
    let start = blotted(grid, (6, 14), 0x1234_5678);

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

fn slope_of(
    board: &mut Board<'_>,
    held: &[Lesson],
    seen: &[usize],
    knobs: &[u32],
    middle: f64,
) -> Vec<f64> {
    let mut piled = vec![0.0_f64; knobs.len()];
    for one in held {
        board.sow(knobs);
        let got = read_out(&board.run(&one.start), seen);
        let off = got - wanted(one.left, middle);
        let share = (off / seen.len() as f64 * PITCH).round() as i32;
        let mut weight = vec![0_u32; knobs.len()];
        for at in seen {
            weight[*at] = share as u32;
        }
        board.sow(knobs);
        board.carry(&one.start, &weight);
        let scale = f64::from(1_u32 << SPARE) * f64::from(board.loud) * PITCH;
        for (into, slope) in piled.iter_mut().zip(board.told_each()) {
            *into += f64::from(slope) / scale;
        }
    }
    piled
}

#[test]
fn a_handful_at_a_time_is_what_the_slope_will_pay_for() {
    let Some(host) = device("a_handful_at_a_time_is_what_the_slope_will_pay_for") else {
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
    let fresh = lessons(grid, 0x0BAD_C0DE, 32);
    let mut knobs = vec![base; grid.cells()];

    board.sow(&knobs);
    let middle = {
        let (left, right) = parted(&mut board, &taught, &seen);
        (left + right) / 2.0
    };
    let judge = |board: &mut Board<'_>, knobs: &[u32]| {
        board.sow(knobs);
        let loss = told_on(board, &taught, &seen, middle);
        let (left, right) = parted(board, &taught, &seen);
        let line = (left + right) / 2.0;
        (loss, left - right, scored(board, &fresh, &seen, line))
    };

    let (loss_first, gap_first, fresh_first) = judge(&mut board, &knobs);
    eprintln!("{:>7} {:>13} {:>9} {:>9}", "moved", "loss", "gap", "fresh");
    eprintln!(
        "{:>7} {loss_first:>13.4e} {gap_first:>9.1} {:>8.0}%",
        0,
        fresh_first * 100.0
    );

    let mut touched = std::collections::BTreeSet::new();
    let handful = 16_usize;
    let sweeps = 256_u32;
    let live: Vec<usize> = slope_of(&mut board, &taught, &seen, &knobs, middle)
        .iter()
        .enumerate()
        .filter(|(_, one)| **one != 0.0)
        .map(|(at, _)| at)
        .collect();
    eprintln!(
        "{} knobs answer at all, taken {handful} at a time in turn",
        live.len()
    );
    for sweep in 0..sweeps {
        let piled = slope_of(&mut board, &taught, &seen, &knobs, middle);
        let from = (sweep as usize * handful) % live.len();
        for step in 0..handful {
            let at = live[(from + step) % live.len()];
            let slope = piled[at];
            if slope == 0.0 {
                continue;
            }
            let held = i64::from(knobs[at]) - slope.signum() as i64;
            knobs[at] = held.clamp(1, 4096) as u32;
            touched.insert(at);
        }
        if sweep % 64 == 63 {
            let (loss, gap, on_fresh) = judge(&mut board, &knobs);
            eprintln!(
                "{:>7} {loss:>13.4e} {gap:>9.1} {:>8.0}%",
                (sweep + 1) as usize * handful,
                on_fresh * 100.0
            );
        }
    }

    let (loss_last, gap_last, fresh_last) = judge(&mut board, &knobs);
    eprintln!(
        "loss {loss_first:.4e} to {loss_last:.4e}, gap {gap_first:.1} to {gap_last:.1}, fresh {:.0}% to {:.0}%",
        fresh_first * 100.0,
        fresh_last * 100.0
    );
    let strayed = knobs.iter().filter(|one| **one != base).count();
    eprintln!(
        "{} different knobs were moved, {strayed} ended away from where they began, and the loss moved {:.5} of itself",
        touched.len(),
        (loss_last - loss_first).abs() / loss_first
    );
    assert_eq!(touched.len(), live.len(), "{}", touched.len());
    assert!(
        (loss_last - loss_first).abs() < loss_first * 1e-4,
        "{loss_first} {loss_last}"
    );
}

#[test]
fn whether_any_field_at_all_can_answer_it() {
    let Some(host) = device("whether_any_field_at_all_can_answer_it") else {
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
    let fresh = lessons(grid, 0x0BAD_C0DE, 32);

    let scattered = |seed: u32, span: u32| {
        let mut state = seed | 1;
        (0..grid.cells())
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                let off = (state % (2 * span + 1)) as i64 - i64::from(span);
                (i64::from(base) + off).clamp(1, 4096) as u32
            })
            .collect::<Vec<u32>>()
    };

    eprintln!(
        "{:>8} {:>8} {:>10} {:>10} {:>10}",
        "spread", "tries", "best gap", "on taught", "then fresh"
    );
    let mut ever = 0.0_f64;
    for span in [0_u32, 64, 255, 1023, 2047] {
        let tries = if span == 0 { 1 } else { 24 };
        let mut best_gap = 0.0_f64;
        let mut picked = (0.0_f64, 0.0_f64);
        for go in 0..tries {
            let knobs = if span == 0 {
                vec![base; grid.cells()]
            } else {
                scattered(0xC0FF_EE00 + go * 7919, span)
            };
            board.sow(&knobs);
            let (left, right) = parted(&mut board, &taught, &seen);
            let line = (left + right) / 2.0;
            let on_taught = scored(&mut board, &taught, &seen, line);
            best_gap = best_gap.max((left - right).abs());
            if on_taught > picked.0 {
                picked = (on_taught, scored(&mut board, &fresh, &seen, line));
            }
        }
        eprintln!(
            "{span:>8} {tries:>8} {best_gap:>10.1} {:>9.0}% {:>9.0}%",
            picked.0 * 100.0,
            picked.1 * 100.0
        );
        ever = ever.max(picked.1);
    }
    eprintln!(
        "picking each spread's best on the taught set, the fresh set gives at most {:.0}%",
        ever * 100.0
    );
    assert!(ever < 0.60, "a field generalises after all: {ever}");
}

#[test]
fn where_between_too_clear_and_too_hazy_a_field_could_matter() {
    let Some(host) = device("where_between_too_clear_and_too_hazy_a_field_could_matter") else {
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

    let scattered = |seed: u32, span: u32| {
        let mut state = seed | 1;
        (0..grid.cells())
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                let off = (state % (2 * span + 1)) as i64 - i64::from(span);
                (i64::from(base) + off).clamp(1, 4096) as u32
            })
            .collect::<Vec<u32>>()
    };

    eprintln!(
        "{:>8} {:>10} {:>12} {:>12}",
        "haze", "plain", "best found", "it gained"
    );
    let mut told = Vec::new();
    for over in 9_u32..=13 {
        let haze = 1 << over;
        let taught = lessons_in(grid, 0x1234_5678, 16, haze);
        let fresh = lessons_in(grid, 0x0BAD_C0DE, 128, haze);

        let line_of = |board: &mut Board<'_>, knobs: &[u32]| {
            board.sow(knobs);
            let (left, right) = parted(board, &taught, &seen);
            (left + right) / 2.0
        };

        let flat = vec![base; grid.cells()];
        let line = line_of(&mut board, &flat);
        let plain = scored(&mut board, &fresh, &seen, line);

        let mut picked = (0.0_f64, flat.clone(), 0.0_f64);
        for go in 0..20_u32 {
            let knobs = scattered(0xC0FF_EE00 + go * 7919, 1 << (9 + go % 3));
            let line = line_of(&mut board, &knobs);
            let on_taught = scored(&mut board, &taught, &seen, line);
            if on_taught > picked.0 {
                picked = (on_taught, knobs, line);
            }
        }
        board.sow(&picked.1);
        let picked = (picked.0, scored(&mut board, &fresh, &seen, picked.2));
        eprintln!(
            "{haze:>8} {:>9.0}% {:>11.0}% {:>11.0}%",
            plain * 100.0,
            picked.1 * 100.0,
            (picked.1 - plain) * 100.0
        );
        told.push((haze, plain, picked.1));
    }

    let best = told
        .iter()
        .max_by(|one, other| (one.2 - one.1).total_cmp(&(other.2 - other.1)))
        .copied()
        .unwrap_or_default();
    eprintln!(
        "the most a field ever gained over the plain one is {:.0} points, at haze {}",
        (best.2 - best.1) * 100.0,
        best.0
    );
    assert!(best.2 - best.1 > 0.05, "{best:?}");
    assert!(best.1 > 0.5 && best.1 < 0.9, "{best:?}");
}

#[test]
fn the_slope_against_the_bar_that_chance_set() {
    let Some(host) = device("the_slope_against_the_bar_that_chance_set") else {
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

    let haze = 1024_u32;
    let seen = window(grid);
    let taught = lessons_in(grid, 0x1234_5678, 16, haze);
    let fresh = lessons_in(grid, 0x0BAD_C0DE, 128, haze);

    let flat = vec![base; grid.cells()];
    board.sow(&flat);
    let middle = {
        let (left, right) = parted(&mut board, &taught, &seen);
        (left + right) / 2.0
    };
    let judge = |board: &mut Board<'_>, knobs: &[u32]| {
        board.sow(knobs);
        let (left, right) = parted(board, &taught, &seen);
        let line = (left + right) / 2.0;
        (
            told_on(board, &taught, &seen, middle),
            scored(board, &fresh, &seen, line),
        )
    };

    let (loss_flat, fresh_flat) = judge(&mut board, &flat);
    eprintln!(
        "the plain field reads {:.0}% of the fresh set, and twenty guesses found 82%",
        fresh_flat * 100.0
    );

    eprintln!("{:>10} {:>7} {:>13} {:>9}", "way", "round", "loss", "fresh");
    let mut told = Vec::new();
    for (named, handful) in [("all of them", 0_usize), ("best sixteen", 16)] {
        let mut knobs = flat.clone();
        for round in 0..60_u32 {
            let piled = slope_of(&mut board, &taught, &seen, &knobs, middle);
            if handful == 0 {
                for (knob, slope) in knobs.iter_mut().zip(&piled) {
                    if *slope == 0.0 {
                        continue;
                    }
                    let held = i64::from(*knob) - slope.signum() as i64;
                    *knob = held.clamp(1, 4096) as u32;
                }
            } else {
                let mut ranked: Vec<(usize, f64)> = piled.iter().copied().enumerate().collect();
                ranked.sort_by(|one, other| other.1.abs().total_cmp(&one.1.abs()));
                for (at, slope) in ranked.iter().take(handful) {
                    let held = i64::from(knobs[*at]) - slope.signum() as i64;
                    knobs[*at] = held.clamp(1, 4096) as u32;
                }
            }
            if round % 20 == 19 {
                let (loss, on_fresh) = judge(&mut board, &knobs);
                eprintln!(
                    "{named:>10} {:>7} {loss:>13.4e} {:>8.0}%",
                    round + 1,
                    on_fresh * 100.0
                );
            }
        }
        let (loss, on_fresh) = judge(&mut board, &knobs);
        let travelled = knobs
            .iter()
            .map(|one| f64::from(*one as i32) - f64::from(base))
            .map(f64::abs)
            .sum::<f64>()
            / knobs.len() as f64;
        told.push((named, loss, on_fresh, travelled));
    }

    eprintln!(
        "plain {:.0}% at loss {loss_flat:.4e}; guessing 82%",
        fresh_flat * 100.0
    );
    for (named, loss, on_fresh, travelled) in &told {
        eprintln!(
            "{named} {:.0}% at loss {loss:.4e}, having moved each knob {travelled:.1} on average",
            on_fresh * 100.0
        );
    }
    eprintln!("the fields that beat the plain one were scattered by 512 to 2048");
    for (named, loss, _, _) in &told {
        assert!(*loss > loss_flat, "{named} went downhill after all: {loss}");
    }
}

fn kept_of(grid: Grid, whole: bool, seen: &[usize]) -> Vec<usize> {
    if whole {
        (0..grid.cells()).collect()
    } else {
        seen.to_vec()
    }
}

#[test]
fn which_of_them_takes_the_promise_out_of_the_slope() {
    let Some(host) = device("which_of_them_takes_the_promise_out_of_the_slope") else {
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
    let seen = window(grid);
    let base = 256_u32;
    let start = blotted_in(grid, (6, 26), 0x1234_5678, 1024);

    eprintln!(
        "{:>8} {:>8} {:>7} {:>14} {:>14} {:>9}",
        "rule", "loss on", "steps", "promised", "delivered", "of it"
    );
    let mut told = Vec::new();
    for (named, kind) in [("smooth", Kind::Smooth), ("living", Kind::Living)] {
        let passes = [
            built(width, grid, Pass::Ahead, kind).unwrap(),
            built(width, grid, Pass::Back, kind).unwrap(),
        ];
        let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
        for whole in [true, false] {
            for depth in [16_u32, 64] {
                let resident = host.resident(&sizes, &modules).unwrap();
                let mut board = Board {
                    resident,
                    over: Groups::Fixed(grid.quads().div_ceil(width)),
                    grid,
                    depth,
                    angle: (base, SWING / 64),
                    loud: 1,
                    planted: false,
                };
                let kept = kept_of(grid, whole, &seen);

                let truth: Vec<u32> = (0..grid.cells())
                    .map(|at| base + ((at as u32 * 2654435761) % 129) - 64)
                    .collect();
                board.sow(&truth);
                let target = board.run(&start);

                let knobs = vec![base; grid.cells()];
                board.sow(&knobs);
                let ended = board.run(&start);
                let loss = |held: &[u32]| {
                    kept.iter()
                        .map(|at| {
                            let off = f64::from(held[*at].wrapping_sub(target[*at]) as i32);
                            off * off / 2.0
                        })
                        .sum::<f64>()
                };
                let began = loss(&ended);

                let mut weight = vec![0_u32; grid.cells()];
                for at in &kept {
                    weight[*at] = ended[*at].wrapping_sub(target[*at]);
                }
                board.sow(&knobs);
                board.carry(&start, &weight);
                let scale = f64::from(1_u32 << SPARE) * f64::from(board.loud);
                let piled: Vec<f64> = board
                    .told_each()
                    .iter()
                    .map(|one| f64::from(*one) / scale)
                    .collect();

                let promised: f64 = -piled.iter().map(|one| one.abs()).sum::<f64>();
                let moved: Vec<u32> = knobs
                    .iter()
                    .zip(&piled)
                    .map(|(knob, slope)| {
                        let held = i64::from(*knob) - slope.signum() as i64;
                        held.clamp(1, 4096) as u32
                    })
                    .collect();
                board.sow(&moved);
                let delivered = loss(&board.run(&start)) - began;
                let of_it = delivered / promised;
                let on = if whole { "whole" } else { "window" };
                eprintln!(
                    "{named:>8} {on:>8} {depth:>7} {promised:>14.1} {delivered:>14.1} {of_it:>9.3}"
                );
                told.push((named, on, depth, of_it));
            }
        }
    }

    for (named, on, depth, of_it) in &told {
        if *of_it > 0.5 {
            eprintln!("{named} on the {on} at {depth} steps keeps {of_it:.2} of its promise");
        }
    }
    let best = told
        .iter()
        .map(|(_, _, _, one)| *one)
        .fold(f64::MIN, f64::max);
    eprintln!("the most any of them delivers is {best:.3} of what it promised");
}

#[test]
fn what_it_is_about_the_lesson_that_breaks_the_promise() {
    let Some(host) = device("what_it_is_about_the_lesson_that_breaks_the_promise") else {
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
    let base = 256_u32;
    let seen = window(grid);
    let haze = 1024_u32;

    eprintln!(
        "{:>22} {:>7} {:>14} {:>14} {:>9}",
        "asked", "lessons", "promised", "delivered", "of it"
    );
    let mut told = Vec::new();
    for (named, many, reachable, by_size) in [
        ("a grid it can reach", 1_usize, true, false),
        ("a number it cannot", 1, false, false),
        ("that, four times", 4, false, false),
        ("that, sixteen times", 16, false, false),
        ("sixteen, moved by size", 16, false, true),
        ("sixteen that agree", 16, false, false),
    ] {
        let resident = host.resident(&sizes, &modules).unwrap();
        let mut board = Board {
            resident,
            over: Groups::Fixed(grid.quads().div_ceil(width)),
            grid,
            depth: DEPTH,
            angle: (base, 0),
            loud: 1,
            planted: false,
        };
        let mut held = lessons_in(grid, 0x1234_5678, many as u32, haze);
        if named == "sixteen that agree" {
            for one in &mut held {
                one.left = true;
            }
        }
        let knobs = vec![base; grid.cells()];

        let truth: Vec<u32> = (0..grid.cells())
            .map(|at| base + ((at as u32 * 2654435761) % 129) - 64)
            .collect();
        board.sow(&truth);
        let aim = board.run(&held[0].start);
        board.sow(&knobs);
        let middle = {
            let (left, right) = parted(&mut board, &held, &seen);
            (left + right) / 2.0
        };

        let loss = |board: &mut Board<'_>, field: &[u32]| {
            board.sow(field);
            let mut sum = 0.0;
            for one in &held {
                let ended = board.run(&one.start);
                if reachable {
                    sum += seen
                        .iter()
                        .map(|at| {
                            let off = f64::from(ended[*at].wrapping_sub(aim[*at]) as i32);
                            off * off / 2.0
                        })
                        .sum::<f64>();
                } else {
                    let got = read_out(&ended, &seen);
                    let off = got - wanted(one.left, middle);
                    sum += off * off / 2.0;
                }
            }
            sum
        };

        let began = loss(&mut board, &knobs);
        let mut piled = vec![0.0_f64; grid.cells()];
        for one in &held {
            board.sow(&knobs);
            let ended = board.run(&one.start);
            let mut weight = vec![0_u32; grid.cells()];
            if reachable {
                for at in &seen {
                    weight[*at] = ended[*at].wrapping_sub(aim[*at]);
                }
            } else {
                let got = read_out(&ended, &seen);
                let off = got - wanted(one.left, middle);
                let share = (off / seen.len() as f64 * PITCH).round() as i32;
                for at in &seen {
                    weight[*at] = share as u32;
                }
            }
            board.sow(&knobs);
            board.carry(&one.start, &weight);
            let mut scale = f64::from(1_u32 << SPARE) * f64::from(board.loud);
            if !reachable {
                scale *= PITCH;
            }
            for (into, slope) in piled.iter_mut().zip(board.told_each()) {
                *into += f64::from(slope) / scale;
            }
        }

        let widest = piled.iter().fold(0.0_f64, |held, one| held.max(one.abs()));
        let promised: f64 = if by_size {
            -piled.iter().map(|one| one * one).sum::<f64>() / widest.max(f64::MIN_POSITIVE)
        } else {
            -piled.iter().map(|one| one.abs()).sum::<f64>()
        };
        let moved: Vec<u32> = knobs
            .iter()
            .zip(&piled)
            .map(|(knob, slope)| {
                let step = if by_size {
                    (slope / widest.max(f64::MIN_POSITIVE)).round() as i64
                } else {
                    slope.signum() as i64
                };
                let held = i64::from(*knob) - step;
                held.clamp(1, 4096) as u32
            })
            .collect();
        let delivered = loss(&mut board, &moved) - began;
        let of_it = delivered / promised;
        eprintln!("{named:>22} {many:>7} {promised:>14.1} {delivered:>14.1} {of_it:>9.3}");
        told.push((named, of_it));
    }

    for (named, of_it) in &told {
        eprintln!("{named}: {of_it:.3}");
    }
    let held = |want: &str| {
        told.iter()
            .find(|(named, _)| *named == want)
            .map_or(0.0, |(_, of_it)| *of_it)
    };
    assert!(
        held("a number it cannot") > 0.5,
        "{}",
        held("a number it cannot")
    );
    assert!(
        held("sixteen that agree") > 0.5,
        "{}",
        held("sixteen that agree")
    );
    assert!(
        held("that, sixteen times") < 0.0,
        "{}",
        held("that, sixteen times")
    );
}

#[test]
fn a_shadow_field_finer_than_a_whole_knob() {
    let Some(host) = device("a_shadow_field_finer_than_a_whole_knob") else {
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

    let haze = 1024_u32;
    let seen = window(grid);
    let taught = lessons_in(grid, 0x1234_5678, 16, haze);
    let fresh = lessons_in(grid, 0x0BAD_C0DE, 128, haze);

    let flat = vec![base; grid.cells()];
    board.sow(&flat);
    let middle = {
        let (left, right) = parted(&mut board, &taught, &seen);
        (left + right) / 2.0
    };
    let judge = |board: &mut Board<'_>, knobs: &[u32]| {
        board.sow(knobs);
        let (left, right) = parted(board, &taught, &seen);
        let line = (left + right) / 2.0;
        (
            told_on(board, &taught, &seen, middle),
            scored(board, &fresh, &seen, line),
        )
    };
    let (loss_flat, fresh_flat) = judge(&mut board, &flat);

    let mut shadow: Vec<f64> = flat.iter().map(|one| f64::from(*one)).collect();
    let cast = |shadow: &[f64]| -> Vec<u32> {
        shadow
            .iter()
            .map(|one| one.round().clamp(1.0, 4096.0) as u32)
            .collect()
    };

    eprintln!(
        "{:>7} {:>13} {:>9} {:>10}",
        "round", "loss", "fresh", "travelled"
    );
    eprintln!(
        "{:>7} {loss_flat:>13.4e} {:>8.0}% {:>10.2}",
        0,
        fresh_flat * 100.0,
        0.0
    );

    let rounds = 100_u32;
    for round in 0..rounds {
        let knobs = cast(&shadow);
        let piled = slope_of(&mut board, &taught, &seen, &knobs, middle);
        let widest = piled.iter().fold(0.0_f64, |held, one| held.max(one.abs()));
        if widest <= 0.0 {
            break;
        }
        let rate = 0.1 / widest;
        for (held, slope) in shadow.iter_mut().zip(&piled) {
            *held -= rate * slope;
        }
        if round % 25 == 24 {
            let knobs = cast(&shadow);
            let (loss, on_fresh) = judge(&mut board, &knobs);
            let travelled = shadow
                .iter()
                .map(|one| (one - f64::from(base)).abs())
                .sum::<f64>()
                / shadow.len() as f64;
            eprintln!(
                "{:>7} {loss:>13.4e} {:>8.0}% {travelled:>10.2}",
                round + 1,
                on_fresh * 100.0
            );
        }
    }

    let knobs = cast(&shadow);
    let (loss_last, fresh_last) = judge(&mut board, &knobs);
    let stirred = knobs.iter().filter(|one| **one != base).count();
    eprintln!(
        "loss {loss_flat:.4e} to {loss_last:.4e}, fresh {:.0}% to {:.0}%, {stirred} knobs ended elsewhere",
        fresh_flat * 100.0,
        fresh_last * 100.0
    );
    eprintln!("the plain field reads 72% and twenty guesses found 82%");
}

#[test]
fn whether_the_summed_slope_is_signal_or_its_own_error() {
    let Some(host) = device("whether_the_summed_slope_is_signal_or_its_own_error") else {
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
    let haze = 1024_u32;
    let seen = window(grid);
    let knobs = vec![base; grid.cells()];

    eprintln!(
        "{:>8} {:>10} {:>10} {:>12}",
        "lessons", "told", "by nudging", "signs agree"
    );
    let mut told_all = Vec::new();
    for many in [1_usize, 16] {
        let held = lessons_in(grid, 0x1234_5678, many as u32, haze);
        board.sow(&knobs);
        let middle = {
            let (left, right) = parted(&mut board, &held, &seen);
            (left + right) / 2.0
        };
        let loss = |board: &mut Board<'_>, field: &[u32]| {
            board.sow(field);
            told_on(board, &held, &seen, middle) * held.len() as f64
        };

        let gathered = |board: &mut Board<'_>, evenly: bool| {
            let mut piled = vec![0.0_f64; grid.cells()];
            for one in &held {
                board.sow(&knobs);
                let ended = board.run(&one.start);
                let got = read_out(&ended, &seen);
                let off = got - wanted(one.left, middle);
                let level = if evenly { 4096.0 } else { PITCH };
                let share = if evenly {
                    (off.signum() * level).round() as i32
                } else {
                    (off / seen.len() as f64 * level).round() as i32
                };
                let mut weight = vec![0_u32; grid.cells()];
                for at in &seen {
                    weight[*at] = share as u32;
                }
                board.sow(&knobs);
                board.carry(&one.start, &weight);
                let mut scale = f64::from(1_u32 << SPARE) * f64::from(board.loud);
                if evenly {
                    scale *= level * seen.len() as f64 / off.abs().max(f64::MIN_POSITIVE);
                } else {
                    scale *= level;
                }
                for (into, slope) in piled.iter_mut().zip(board.told_each()) {
                    *into += f64::from(slope) / scale;
                }
            }
            piled
        };
        let piled = gathered(&mut board, false);
        let evenly = gathered(&mut board, true);

        let mut ranked: Vec<(usize, f64)> = piled.iter().copied().enumerate().collect();
        ranked.sort_by(|one, other| other.1.abs().total_cmp(&one.1.abs()));

        let step = 16_i64;
        let mut agreed = 0_usize;
        let mut looked = 0_usize;
        let mut told_size = (0.0_f64, 0.0_f64);
        for (at, mine) in ranked.iter().take(32) {
            let mut above = knobs.clone();
            above[*at] = (i64::from(knobs[*at]) + step) as u32;
            let high = loss(&mut board, &above);
            let mut below = knobs.clone();
            below[*at] = (i64::from(knobs[*at]) - step) as u32;
            let low = loss(&mut board, &below);
            let nudged = (high - low) / (2.0 * step as f64);
            if nudged != 0.0 {
                looked += 1;
                if mine.signum() == nudged.signum() {
                    agreed += 1;
                }
                told_size = (told_size.0 + mine.abs(), told_size.1 + nudged.abs());
            }
        }
        let mut agreed_evenly = 0_usize;
        for (at, _) in ranked.iter().take(32) {
            let mut above = knobs.clone();
            above[*at] = (i64::from(knobs[*at]) + step) as u32;
            let high = loss(&mut board, &above);
            let mut below = knobs.clone();
            below[*at] = (i64::from(knobs[*at]) - step) as u32;
            let low = loss(&mut board, &below);
            let nudged = (high - low) / (2.0 * step as f64);
            if nudged != 0.0 && evenly[*at].signum() == nudged.signum() {
                agreed_evenly += 1;
            }
        }
        let share = agreed as f64 * 100.0 / looked.max(1) as f64;
        let share_evenly = agreed_evenly as f64 * 100.0 / looked.max(1) as f64;
        eprintln!("    the same lessons weighed evenly agree {share_evenly:.0}%");
        eprintln!(
            "{many:>8} {:>10.3} {:>10.3} {share:>11.0}%",
            told_size.0 / looked.max(1) as f64,
            told_size.1 / looked.max(1) as f64
        );
        told_all.push((many, share));
    }

    for (many, share) in &told_all {
        eprintln!(
            "with {many} lessons the summed slope points the way a nudge does {share:.0}% of the time"
        );
    }
    let one = told_all.first().map_or(0.0, |(_, share)| *share);
    let lots = told_all.last().map_or(0.0, |(_, share)| *share);
    assert!(one > 70.0, "{one}");
    assert!(lots < 50.0, "{lots}");
}

#[test]
fn whether_one_lesson_at_a_time_gets_anywhere() {
    let Some(host) = device("whether_one_lesson_at_a_time_gets_anywhere") else {
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

    let haze = 1024_u32;
    let seen = window(grid);
    let taught = lessons_in(grid, 0x1234_5678, 16, haze);
    let fresh = lessons_in(grid, 0x0BAD_C0DE, 128, haze);

    let flat = vec![base; grid.cells()];
    board.sow(&flat);
    let middle = {
        let (left, right) = parted(&mut board, &taught, &seen);
        (left + right) / 2.0
    };
    let judge = |board: &mut Board<'_>, knobs: &[u32]| {
        board.sow(knobs);
        let (left, right) = parted(board, &taught, &seen);
        let line = (left + right) / 2.0;
        (
            told_on(board, &taught, &seen, middle),
            scored(board, &fresh, &seen, line),
        )
    };

    let (loss_flat, fresh_flat) = judge(&mut board, &flat);
    eprintln!(
        "the plain field reads {:.0}% of the fresh set at loss {loss_flat:.4e}, and twenty guesses found 82%",
        fresh_flat * 100.0
    );

    eprintln!("{:>6} {:>7} {:>13} {:>9}", "step", "round", "loss", "fresh");
    let mut told = Vec::new();
    let mut best = (0.0_f64, 0_i64, 0_u32);
    for step in [1_i64, PACE, 16, 64] {
        let mut knobs = flat.clone();
        for round in 0..ROUNDS {
            for one in &taught {
                let piled = slope_of(
                    &mut board,
                    core::slice::from_ref(one),
                    &seen,
                    &knobs,
                    middle,
                );
                for (knob, slope) in knobs.iter_mut().zip(&piled) {
                    if *slope == 0.0 {
                        continue;
                    }
                    let held = i64::from(*knob) - slope.signum() as i64 * step;
                    *knob = held.clamp(1, 4096) as u32;
                }
            }
            if round % 20 == 19 {
                let (loss, on_fresh) = judge(&mut board, &knobs);
                if on_fresh > best.0 {
                    best = (on_fresh, step, round + 1);
                }
                eprintln!(
                    "{step:>6} {:>7} {loss:>13.4e} {:>8.0}%",
                    round + 1,
                    on_fresh * 100.0
                );
            }
        }
        let (loss, on_fresh) = judge(&mut board, &knobs);
        let travelled = knobs
            .iter()
            .map(|one| f64::from(*one as i32) - f64::from(base))
            .map(f64::abs)
            .sum::<f64>()
            / knobs.len() as f64;
        told.push((step, loss, on_fresh, travelled));
    }

    eprintln!(
        "plain {:.0}% at loss {loss_flat:.4e}; guessing 82%; sixty batch rounds ended at 67%",
        fresh_flat * 100.0
    );
    for (step, loss, on_fresh, travelled) in &told {
        eprintln!(
            "a step of {step} taken one lesson at a time ends at {:.0}% and loss {loss:.4e}, each knob having moved {travelled:.1}",
            on_fresh * 100.0
        );
    }
    eprintln!(
        "the best reading is {:.0}% at a step of {} after {} rounds",
        best.0 * 100.0,
        best.1,
        best.2
    );
    let held = told
        .iter()
        .find(|(step, _, _, _)| *step == PACE)
        .map_or((0, 0.0, 0.0, 0.0), |one| *one);
    assert!(
        held.2 > fresh_flat,
        "a step of {} read no better than the plain field: {} against {fresh_flat}",
        PACE,
        held.2
    );
    assert!(
        held.1 < loss_flat,
        "a step of {} did not go downhill: {} against {loss_flat}",
        PACE,
        held.1
    );
    assert!(
        best.0 > 0.82,
        "one lesson at a time never beat the bar chance set: {}",
        best.0
    );
}

#[test]
fn where_between_four_and_sixteen_the_step_stops_paying() {
    let Some(host) = device("where_between_four_and_sixteen_the_step_stops_paying") else {
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

    let haze = 1024_u32;
    let seen = window(grid);
    let taught = lessons_in(grid, 0x1234_5678, 16, haze);
    let fresh = lessons_in(grid, 0x0BAD_C0DE, 128, haze);

    let flat = vec![base; grid.cells()];
    board.sow(&flat);
    let middle = {
        let (left, right) = parted(&mut board, &taught, &seen);
        (left + right) / 2.0
    };
    let judge = |board: &mut Board<'_>, knobs: &[u32]| {
        board.sow(knobs);
        let (left, right) = parted(board, &taught, &seen);
        let line = (left + right) / 2.0;
        (
            told_on(board, &taught, &seen, middle),
            scored(board, &fresh, &seen, line),
        )
    };

    let (loss_flat, fresh_flat) = judge(&mut board, &flat);
    eprintln!(
        "the plain field reads {:.0}% at loss {loss_flat:.4e}; a step of 4 ended at 90% and one of 16 peaked at 91% and left",
        fresh_flat * 100.0
    );

    eprintln!(
        "{:>6} {:>7} {:>13} {:>9} {:>9}",
        "step", "round", "loss", "fresh", "moved"
    );
    let travel = |knobs: &[u32]| {
        knobs
            .iter()
            .map(|one| f64::from(*one as i32) - f64::from(base))
            .map(f64::abs)
            .sum::<f64>()
            / knobs.len() as f64
    };
    let mut told = Vec::new();
    for step in [PACE, 6, 8, 12, 16] {
        let mut knobs = flat.clone();
        let mut top = (0.0_f64, 0_u32, 0.0_f64);
        let mut ending = (0.0_f64, 0.0_f64);
        for round in 0..ROUNDS {
            for one in &taught {
                let piled = slope_of(
                    &mut board,
                    core::slice::from_ref(one),
                    &seen,
                    &knobs,
                    middle,
                );
                for (knob, slope) in knobs.iter_mut().zip(&piled) {
                    if *slope == 0.0 {
                        continue;
                    }
                    let held = i64::from(*knob) - slope.signum() as i64 * step;
                    *knob = held.clamp(1, 4096) as u32;
                }
            }
            if round % 10 == 9 {
                let (loss, on_fresh) = judge(&mut board, &knobs);
                let moved = travel(&knobs);
                if on_fresh > top.0 {
                    top = (on_fresh, round + 1, moved);
                }
                ending = (loss, on_fresh);
                eprintln!(
                    "{step:>6} {:>7} {loss:>13.4e} {:>8.0}% {moved:>9.1}",
                    round + 1,
                    on_fresh * 100.0
                );
            }
        }
        told.push((step, top.0, top.1, top.2, ending.1));
    }

    eprintln!(
        "{:>6} {:>10} {:>9} {:>10} {:>10}",
        "step", "peak", "at round", "moved by", "at sixty"
    );
    for (step, peak, at, moved, ending) in &told {
        eprintln!(
            "{step:>6} {:>9.0}% {at:>9} {moved:>10.1} {:>9.0}%",
            peak * 100.0,
            ending * 100.0
        );
    }
    let high = told.iter().fold(0.0_f64, |held, one| held.max(one.1));
    let low = told.iter().fold(1.0_f64, |held, one| held.min(one.1));
    let far = told.iter().fold(0.0_f64, |held, one| held.max(one.3));
    let near = told.iter().fold(f64::MAX, |held, one| held.min(one.3));
    eprintln!(
        "every step in the band peaks between {:.0}% and {:.0}%, having moved each knob between {near:.1} and {far:.1}",
        low * 100.0,
        high * 100.0
    );
    assert!(
        low > 0.82,
        "a step in the band failed to beat the bar chance set: {low}"
    );
    assert!(
        high - low < 0.05,
        "the band has a best step after all: {low} to {high}"
    );
    let last = told.last().map_or(0.0, |one| one.4);
    assert!(last < 0.60, "the coarsest step held on to its peak: {last}");
}

#[test]
fn whether_a_shrinking_step_keeps_what_the_coarse_one_found() {
    let Some(host) = device("whether_a_shrinking_step_keeps_what_the_coarse_one_found") else {
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

    let haze = 1024_u32;
    let seen = window(grid);
    let taught = lessons_in(grid, 0x1234_5678, 16, haze);
    let fresh = lessons_in(grid, 0x0BAD_C0DE, 128, haze);

    let flat = vec![base; grid.cells()];
    board.sow(&flat);
    let middle = {
        let (left, right) = parted(&mut board, &taught, &seen);
        (left + right) / 2.0
    };
    let judge = |board: &mut Board<'_>, knobs: &[u32]| {
        board.sow(knobs);
        let (left, right) = parted(board, &taught, &seen);
        let line = (left + right) / 2.0;
        (
            told_on(board, &taught, &seen, middle),
            scored(board, &fresh, &seen, line),
        )
    };

    let (loss_flat, fresh_flat) = judge(&mut board, &flat);
    eprintln!(
        "the plain field reads {:.0}% at loss {loss_flat:.4e}; held at 16 the descent peaked at 91% by round 20 and ended at 55%",
        fresh_flat * 100.0
    );

    eprintln!(
        "{:>16} {:>7} {:>6} {:>13} {:>9}",
        "way", "round", "step", "loss", "fresh"
    );
    let mut told = Vec::new();
    for (named, from, shrink) in [
        ("held at 4", PACE, Shrink::Never),
        ("held at 16", 16, Shrink::Never),
        ("halved each ten", 16, Shrink::Each(10)),
        ("halved on plateau", 16, Shrink::Stalled(3)),
    ] {
        let mut knobs = flat.clone();
        let mut step = from;
        let mut lowest = f64::MAX;
        let mut waited = 0_u32;
        let mut top = (0.0_f64, 0_u32);
        let mut ending = (0.0_f64, 0.0_f64);
        for round in 0..ROUNDS {
            for one in &taught {
                let piled = slope_of(
                    &mut board,
                    core::slice::from_ref(one),
                    &seen,
                    &knobs,
                    middle,
                );
                for (knob, slope) in knobs.iter_mut().zip(&piled) {
                    if *slope == 0.0 {
                        continue;
                    }
                    let held = i64::from(*knob) - slope.signum() as i64 * step;
                    *knob = held.clamp(1, 4096) as u32;
                }
            }
            match shrink {
                Shrink::Never => {}
                Shrink::Each(every) => {
                    if round % every == every - 1 {
                        step = (step / 2).max(1);
                    }
                }
                Shrink::Stalled(patience) => {
                    board.sow(&knobs);
                    let loss = told_on(&mut board, &taught, &seen, middle);
                    if loss < lowest {
                        lowest = loss;
                        waited = 0;
                    } else {
                        waited += 1;
                        if waited >= patience {
                            step = (step / 2).max(1);
                            waited = 0;
                        }
                    }
                }
            }
            if round % 10 == 9 {
                let (loss, on_fresh) = judge(&mut board, &knobs);
                if on_fresh > top.0 {
                    top = (on_fresh, round + 1);
                }
                ending = (loss, on_fresh);
                eprintln!(
                    "{named:>16} {:>7} {step:>6} {loss:>13.4e} {:>8.0}%",
                    round + 1,
                    on_fresh * 100.0
                );
            }
        }
        told.push((named, top.0, top.1, ending.1, ending.0));
    }

    eprintln!(
        "{:>16} {:>10} {:>9} {:>10} {:>13}",
        "way", "peak", "at round", "at sixty", "loss at sixty"
    );
    for (named, peak, at, ending, loss) in &told {
        eprintln!(
            "{named:>16} {:>9.0}% {at:>9} {:>9.0}% {loss:>13.4e}",
            peak * 100.0,
            ending * 100.0
        );
    }
    let way = |want: &str| {
        told.iter()
            .find(|(named, _, _, _, _)| *named == want)
            .map_or(("none", 0.0, 0_u32, 0.0, 0.0), |one| *one)
    };
    let coarse = way("held at 16");
    let shrunk = way("halved each ten");
    let slow = way("held at 4");
    eprintln!(
        "shrinking keeps {:.0}% where holding kept {:.0}%, and arrives at round {} where a step of 4 takes {}",
        shrunk.3 * 100.0,
        coarse.3 * 100.0,
        shrunk.2,
        slow.2
    );
    assert!(
        coarse.3 < 0.60,
        "the coarse step held on to its peak after all: {}",
        coarse.3
    );
    assert!(
        shrunk.3 > 0.85,
        "shrinking did not keep what the coarse step found: {}",
        shrunk.3
    );
    assert!(
        shrunk.2 < slow.2,
        "shrinking was no faster than a step of 4: round {} against {}",
        shrunk.2,
        slow.2
    );
    let high = told.iter().fold(0.0_f64, |held, one| held.max(one.1));
    assert!(high < 0.95, "the ceiling moved after all: {high}");
}

#[derive(Clone, Copy)]
enum Shrink {
    Never,
    Each(u32),
    Stalled(u32),
}

#[test]
fn how_far_down_a_step_finer_than_a_knob_can_go() {
    let Some(host) = device("how_far_down_a_step_finer_than_a_knob_can_go") else {
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

    let haze = 1024_u32;
    let seen = window(grid);
    let taught = lessons_in(grid, 0x1234_5678, 16, haze);
    let fresh = lessons_in(grid, 0x0BAD_C0DE, 128, haze);

    let flat = vec![base; grid.cells()];
    board.sow(&flat);
    let middle = {
        let (left, right) = parted(&mut board, &taught, &seen);
        (left + right) / 2.0
    };
    let judge = |board: &mut Board<'_>, knobs: &[u32]| {
        board.sow(knobs);
        let (left, right) = parted(board, &taught, &seen);
        let line = (left + right) / 2.0;
        (
            told_on(board, &taught, &seen, middle),
            scored(board, &fresh, &seen, line),
        )
    };
    let cast = |shadow: &[f64]| -> Vec<u32> {
        shadow
            .iter()
            .map(|one| one.round().clamp(1.0, 4096.0) as u32)
            .collect()
    };

    let (loss_flat, fresh_flat) = judge(&mut board, &flat);
    eprintln!(
        "the plain field reads {:.0}% at loss {loss_flat:.4e}; a whole-knob step reached 91% and could not stay",
        fresh_flat * 100.0
    );

    eprintln!(
        "{:>7} {:>7} {:>7} {:>13} {:>9} {:>10}",
        "by", "pace", "round", "loss", "fresh", "moved"
    );
    let mut told = Vec::new();
    for (named, from, by_sign, halve_each, floor_at) in [
        ("size", 1.0_f64, false, 0_u32, 0.0_f64),
        ("size", 16.0, false, 0, 0.0),
        ("sign", 0.25, true, 0, 0.0),
        ("sign", 1.0, true, 0, 0.0),
        ("sign", 4.0, true, 0, 0.0),
        ("sign", 16.0, true, 0, 0.0),
        ("to 1", 16.0, true, 10, 1.0),
        ("to 1/8", 16.0, true, 10, 0.125),
    ] {
        let mut pace = from;
        let mut shadow: Vec<f64> = flat.iter().map(|one| f64::from(*one)).collect();
        let mut top = (0.0_f64, 0_u32);
        let mut floor = f64::MAX;
        let mut ending = (0.0_f64, 0.0_f64);
        for round in 0..ROUNDS {
            for one in &taught {
                let knobs = cast(&shadow);
                let piled = slope_of(
                    &mut board,
                    core::slice::from_ref(one),
                    &seen,
                    &knobs,
                    middle,
                );
                let widest = piled.iter().fold(0.0_f64, |held, one| held.max(one.abs()));
                if widest <= 0.0 {
                    continue;
                }
                let rate = pace / widest;
                for (held, slope) in shadow.iter_mut().zip(&piled) {
                    let by = if by_sign {
                        pace * slope.signum()
                    } else {
                        rate * slope
                    };
                    if *slope == 0.0 {
                        continue;
                    }
                    *held = (*held - by).clamp(1.0, 4096.0);
                }
            }
            if halve_each > 0 && round % halve_each == halve_each - 1 {
                pace = (pace / 2.0).max(floor_at);
            }
            if round % 10 == 9 {
                let knobs = cast(&shadow);
                let (loss, on_fresh) = judge(&mut board, &knobs);
                let moved = shadow
                    .iter()
                    .map(|one| (one - f64::from(base)).abs())
                    .sum::<f64>()
                    / shadow.len() as f64;
                if on_fresh > top.0 {
                    top = (on_fresh, round + 1);
                }
                floor = floor.min(loss);
                ending = (loss, on_fresh);
                eprintln!(
                    "{named:>7} {pace:>7.2} {:>7} {loss:>13.4e} {:>8.0}% {moved:>10.2}",
                    round + 1,
                    on_fresh * 100.0
                );
            }
        }
        told.push((named, pace, top.0, top.1, ending.1, floor));
    }

    eprintln!(
        "{:>7} {:>7} {:>10} {:>9} {:>10} {:>13}",
        "by", "pace", "peak", "at round", "at sixty", "lowest loss"
    );
    for (named, pace, peak, at, ending, floor) in &told {
        eprintln!(
            "{named:>7} {pace:>7.2} {:>9.0}% {at:>9} {:>9.0}% {floor:>13.4e}",
            peak * 100.0,
            ending * 100.0
        );
    }
    let lowest = told.iter().fold(f64::MAX, |held, one| held.min(one.5));
    eprintln!("the lowest loss anything reached is {lowest:.4e}");
    let by = |want: &str| {
        told.iter()
            .filter(|(named, _, _, _, _, _)| *named == want)
            .fold((0.0_f64, f64::MAX), |held, one| {
                (held.0.max(one.2), held.1.min(one.5))
            })
    };
    let size = by("size");
    let sign = by("sign");
    eprintln!(
        "moving by size peaks at {:.0}% and bottoms at {:.4e}; by sign, {:.0}% and {:.4e}",
        size.0 * 100.0,
        size.1,
        sign.0 * 100.0,
        sign.1
    );
    assert!(
        size.0 < fresh_flat + 0.01,
        "moving by size read better than the plain field after all: {}",
        size.0
    );
    assert!(
        sign.0 > 0.82,
        "a shadow field stepped by sign failed to beat the bar chance set: {}",
        sign.0
    );
    let whole = by("to 1");
    let fine = by("to 1/8");
    eprintln!(
        "shrinking to a whole knob bottoms at {:.4e}, shrinking to an eighth at {:.4e}",
        whole.1, fine.1
    );
    assert!(
        (fine.1 - whole.1).abs() / whole.1 < 0.01,
        "carrying the field finer than a knob found a different floor: {} against {}",
        fine.1,
        whole.1
    );
    assert!(
        fine.1 > sign.1,
        "the finest shrinking beat the coarse step's floor after all: {} against {}",
        fine.1,
        sign.1
    );
}

fn settled_by(
    board: &mut Board<'_>,
    held: &[Lesson],
    seen: &[usize],
    from: &[u32],
    middle: f64,
    pace: f64,
    rounds: u32,
) -> Vec<f64> {
    let mut shadow: Vec<f64> = from.iter().map(|one| f64::from(*one)).collect();
    for _ in 0..rounds {
        for one in held {
            let knobs: Vec<u32> = shadow
                .iter()
                .map(|one| one.round().clamp(1.0, 4096.0) as u32)
                .collect();
            let piled = slope_of(board, core::slice::from_ref(one), seen, &knobs, middle);
            for (into, slope) in shadow.iter_mut().zip(&piled) {
                if *slope == 0.0 {
                    continue;
                }
                *into = (*into - pace * slope.signum()).clamp(1.0, 4096.0);
            }
        }
    }
    shadow
}

#[test]
fn whether_the_place_it_leaves_is_a_minimum_at_all() {
    let Some(host) = device("whether_the_place_it_leaves_is_a_minimum_at_all") else {
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

    let haze = 1024_u32;
    let seen = window(grid);
    let taught = lessons_in(grid, 0x1234_5678, 16, haze);
    let fresh = lessons_in(grid, 0x0BAD_C0DE, 128, haze);

    let flat = vec![base; grid.cells()];
    board.sow(&flat);
    let middle = {
        let (left, right) = parted(&mut board, &taught, &seen);
        (left + right) / 2.0
    };
    let cast = |shadow: &[f64]| -> Vec<u32> {
        shadow
            .iter()
            .map(|one| one.round().clamp(1.0, 4096.0) as u32)
            .collect()
    };
    let loss_at = |board: &mut Board<'_>, knobs: &[u32]| {
        board.sow(knobs);
        told_on(board, &taught, &seen, middle)
    };
    let reads = |board: &mut Board<'_>, knobs: &[u32]| {
        board.sow(knobs);
        let (left, right) = parted(board, &taught, &seen);
        scored(board, &fresh, &seen, (left + right) / 2.0)
    };

    let settled = settled_by(&mut board, &taught, &seen, &flat, middle, 16.0, 20);
    let rested = cast(&settled);
    let loss_settled = loss_at(&mut board, &rested);
    let fresh_settled = reads(&mut board, &rested);
    eprintln!(
        "twenty rounds at a pace of 16 leave the field at loss {loss_settled:.4e} reading {:.0}%",
        fresh_settled * 100.0
    );

    eprintln!(
        "{:>7} {:>7} {:>13} {:>10} {:>10}",
        "pace", "round", "loss", "net", "path"
    );
    let mut told = Vec::new();
    for pace in [0.25_f64, 1.0, 4.0, 16.0] {
        let mut shadow = settled.clone();
        let mut path = 0.0_f64;
        let mut rise = Vec::new();
        for round in 0..20_u32 {
            let before = shadow.clone();
            for one in &taught {
                let knobs = cast(&shadow);
                let piled = slope_of(
                    &mut board,
                    core::slice::from_ref(one),
                    &seen,
                    &knobs,
                    middle,
                );
                for (into, slope) in shadow.iter_mut().zip(&piled) {
                    if *slope == 0.0 {
                        continue;
                    }
                    *into = (*into - pace * slope.signum()).clamp(1.0, 4096.0);
                }
            }
            path += shadow
                .iter()
                .zip(&before)
                .map(|(now, then)| (now - then).abs())
                .sum::<f64>()
                / shadow.len() as f64;
            if round % 5 == 4 {
                let knobs = cast(&shadow);
                let loss = loss_at(&mut board, &knobs);
                let net = shadow
                    .iter()
                    .zip(&settled)
                    .map(|(now, then)| (now - then).abs())
                    .sum::<f64>()
                    / shadow.len() as f64;
                rise.push(loss);
                eprintln!(
                    "{pace:>7.2} {:>7} {loss:>13.4e} {net:>10.2} {path:>10.2}",
                    round + 1
                );
            }
        }
        let net = shadow
            .iter()
            .zip(&settled)
            .map(|(now, then)| (now - then).abs())
            .sum::<f64>()
            / shadow.len() as f64;
        let ended = rise.last().copied().unwrap_or(loss_settled);
        told.push((pace, ended - loss_settled, net, path));
    }

    eprintln!(
        "{:>7} {:>14} {:>14} {:>10} {:>10} {:>10}",
        "pace", "loss risen by", "per pace", "net", "path", "net/path"
    );
    for (pace, risen, net, path) in &told {
        eprintln!(
            "{pace:>7.2} {risen:>14.1} {:>14.1} {net:>10.2} {path:>10.2} {:>10.3}",
            risen / pace,
            net / path.max(f64::MIN_POSITIVE)
        );
    }

    let nudge = 16_i64;
    let summed = |board: &mut Board<'_>, knobs: &[u32]| {
        board.sow(knobs);
        told_on(board, &taught, &seen, middle) * taught.len() as f64
    };
    let truly = |board: &mut Board<'_>, knobs: &[u32]| {
        let mut sizes = Vec::new();
        for at in (0..grid.cells()).step_by(grid.cells() / 16) {
            let mut above = knobs.to_vec();
            above[at] = (i64::from(knobs[at]) + nudge).clamp(1, 4096) as u32;
            let high = summed(board, &above);
            let mut below = knobs.to_vec();
            below[at] = (i64::from(knobs[at]) - nudge).clamp(1, 4096) as u32;
            let low = summed(board, &below);
            sizes.push((high - low) / (2.0 * nudge as f64));
        }
        let wide = sizes.iter().map(|one| one.abs()).sum::<f64>() / sizes.len() as f64;
        let leaning = sizes.iter().sum::<f64>() / sizes.len() as f64;
        (wide, leaning)
    };
    let (wide_flat, lean_flat) = truly(&mut board, &flat);
    let (wide_rest, lean_rest) = truly(&mut board, &rested);
    eprintln!(
        "nudging sixteen knobs, the true slope averages {wide_flat:.3} at the plain field and {wide_rest:.3} at the settled one"
    );
    eprintln!("and leans {lean_flat:.3} against {lean_rest:.3}");
    let straightest = told.iter().fold(1.0_f64, |held, (_, _, net, path)| {
        held.min(net / path.max(f64::MIN_POSITIVE))
    });
    assert!(
        straightest > 0.8,
        "the field wanders rather than walks: {straightest}"
    );
    let stepped: Vec<f64> = told
        .iter()
        .filter(|(pace, _, _, _)| *pace >= 1.0)
        .map(|(pace, risen, _, _)| risen / pace)
        .collect();
    let most = stepped.iter().fold(0.0_f64, |held, one| held.max(*one));
    let least = stepped.iter().fold(f64::MAX, |held, one| held.min(*one));
    assert!(
        most < least * 2.0,
        "the rise is not linear in the pace: {least} to {most}"
    );
    assert!(
        wide_rest > 0.4,
        "the settled field is flat after all: {wide_rest}"
    );
}

#[test]
fn whether_the_whole_descent_is_one_direction() {
    let Some(host) = device("whether_the_whole_descent_is_one_direction") else {
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

    let haze = 1024_u32;
    let seen = window(grid);
    let taught = lessons_in(grid, 0x1234_5678, 16, haze);
    let fresh = lessons_in(grid, 0x0BAD_C0DE, 128, haze);

    let flat = vec![base; grid.cells()];
    board.sow(&flat);
    let middle = {
        let (left, right) = parted(&mut board, &taught, &seen);
        (left + right) / 2.0
    };
    let judge = |board: &mut Board<'_>, knobs: &[u32]| {
        board.sow(knobs);
        let (left, right) = parted(board, &taught, &seen);
        let line = (left + right) / 2.0;
        (
            told_on(board, &taught, &seen, middle),
            scored(board, &fresh, &seen, line),
        )
    };

    let settled = settled_by(&mut board, &taught, &seen, &flat, middle, 16.0, 20);
    let rested: Vec<u32> = settled
        .iter()
        .map(|one| one.round().clamp(1.0, 4096.0) as u32)
        .collect();
    let onward = settled_by(&mut board, &taught, &seen, &rested, middle, 16.0, 20);

    let early: Vec<f64> = settled.iter().map(|one| one - f64::from(base)).collect();
    let later: Vec<f64> = onward
        .iter()
        .zip(&settled)
        .map(|(now, then)| now - then)
        .collect();
    let agreed = early
        .iter()
        .zip(&later)
        .filter(|(one, other)| one.signum() == other.signum() && **one != 0.0)
        .count();
    let stirred = early.iter().filter(|one| **one != 0.0).count();
    eprintln!(
        "of {stirred} knobs the descent moved in its first twenty rounds, {agreed} are still moving the same way in the next twenty"
    );

    let (loss_flat, fresh_flat) = judge(&mut board, &flat);
    let (loss_rest, fresh_rest) = judge(&mut board, &rested);
    eprintln!(
        "the plain field reads {:.0}% at {loss_flat:.4e} and the settled one {:.0}% at {loss_rest:.4e}",
        fresh_flat * 100.0,
        fresh_rest * 100.0
    );

    eprintln!("{:>7} {:>13} {:>9}", "along", "loss", "fresh");
    let mut best = (0.0_f64, f64::MAX, 0.0_f64);
    for tenth in 0..=24_u32 {
        let along = f64::from(tenth) / 8.0;
        let knobs: Vec<u32> = early
            .iter()
            .map(|one| (f64::from(base) + along * one).round().clamp(1.0, 4096.0) as u32)
            .collect();
        let (loss, on_fresh) = judge(&mut board, &knobs);
        if loss < best.1 {
            best = (along, loss, on_fresh);
        }
        if tenth % 2 == 0 {
            eprintln!("{along:>7.3} {loss:>13.4e} {:>8.0}%", on_fresh * 100.0);
        }
    }
    eprintln!(
        "the best point on the line is {:.3} of the way along, at loss {:.4e} reading {:.0}%",
        best.0,
        best.1,
        best.2 * 100.0
    );
    eprintln!("sixty rounds of descent never read better than 91%");
    assert!(
        best.1 < loss_rest,
        "the line holds nothing the descent did not already have: {} against {loss_rest}",
        best.1
    );
    assert!(
        best.2 >= 0.90,
        "walking the line does not reach what the descent reached: {}",
        best.2
    );
}

#[test]
fn whether_the_knobs_the_slope_never_names_matter_at_all() {
    let Some(host) = device("whether_the_knobs_the_slope_never_names_matter_at_all") else {
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

    let haze = 1024_u32;
    let seen = window(grid);
    let taught = lessons_in(grid, 0x1234_5678, 16, haze);
    let flat = vec![base; grid.cells()];
    board.sow(&flat);
    let middle = {
        let (left, right) = parted(&mut board, &taught, &seen);
        (left + right) / 2.0
    };

    let piled = slope_of(&mut board, &taught, &seen, &flat, middle);
    let named: Vec<usize> = piled
        .iter()
        .enumerate()
        .filter(|(_, one)| **one != 0.0)
        .map(|(at, _)| at)
        .collect();
    let silent: Vec<usize> = piled
        .iter()
        .enumerate()
        .filter(|(_, one)| **one == 0.0)
        .map(|(at, _)| at)
        .collect();
    eprintln!(
        "of {} knobs the summed slope names {} and leaves {} silent",
        grid.cells(),
        named.len(),
        silent.len()
    );

    let mut one_at_a_time = 0_usize;
    for lesson in &taught {
        let each = slope_of(
            &mut board,
            core::slice::from_ref(lesson),
            &seen,
            &flat,
            middle,
        );
        one_at_a_time = one_at_a_time.max(each.iter().filter(|one| **one != 0.0).count());
    }
    eprintln!("the widest a single lesson names is {one_at_a_time}");

    let nudge = 16_i64;
    let summed = |board: &mut Board<'_>, knobs: &[u32]| {
        board.sow(knobs);
        told_on(board, &taught, &seen, middle) * taught.len() as f64
    };
    let sampled = |held: &[usize], want: usize| -> Vec<usize> {
        if held.len() <= want {
            return held.to_vec();
        }
        let every = held.len() / want;
        held.iter().step_by(every).take(want).copied().collect()
    };
    let truly = |board: &mut Board<'_>, at: usize| {
        let mut above = flat.clone();
        above[at] = (i64::from(flat[at]) + nudge).clamp(1, 4096) as u32;
        let high = summed(board, &above);
        let mut below = flat.clone();
        below[at] = (i64::from(flat[at]) - nudge).clamp(1, 4096) as u32;
        let low = summed(board, &below);
        (high - low) / (2.0 * nudge as f64)
    };

    eprintln!(
        "{:>10} {:>8} {:>14} {:>14}",
        "knobs", "looked", "mean |slope|", "ever moved"
    );
    let mut told = Vec::new();
    for (what, held) in [("named", &named), ("silent", &silent)] {
        let picked = sampled(held, 24);
        let mut sizes = Vec::new();
        for at in &picked {
            sizes.push(truly(&mut board, *at));
        }
        let wide = sizes.iter().map(|one| one.abs()).sum::<f64>() / sizes.len() as f64;
        let moved = sizes.iter().filter(|one| **one != 0.0).count();
        eprintln!("{what:>10} {:>8} {wide:>14.4} {moved:>14}", picked.len());
        told.push((what, wide, moved, picked.len()));
    }

    let named_wide = told.first().map_or(0.0, |one| one.1);
    let silent_wide = told.get(1).map_or(0.0, |one| one.1);
    let silent_moved = told.get(1).map_or(0, |one| one.2);
    eprintln!(
        "a knob the slope never names still moves the loss by {silent_wide:.4} against {named_wide:.4} for one it does, and {silent_moved} of the sampled silent ones move it at all"
    );
    assert_eq!(
        named.len(),
        grid.cells() / 2,
        "the slope names other than half the knobs"
    );
    assert_eq!(
        silent_moved, 0,
        "a knob the slope never names moves the loss, so the silence is a truncation"
    );
    assert!(
        named_wide > 0.4,
        "the knobs the slope does name move the loss by nothing: {named_wide}"
    );
}

#[test]
fn how_many_knobs_the_lessons_agree_about() {
    let Some(host) = device("how_many_knobs_the_lessons_agree_about") else {
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

    let haze = 1024_u32;
    let seen = window(grid);
    let taught = lessons_in(grid, 0x1234_5678, 16, haze);
    let flat = vec![base; grid.cells()];
    board.sow(&flat);
    let middle = {
        let (left, right) = parted(&mut board, &taught, &seen);
        (left + right) / 2.0
    };

    let mut each = Vec::new();
    for lesson in &taught {
        each.push(slope_of(
            &mut board,
            core::slice::from_ref(lesson),
            &seen,
            &flat,
            middle,
        ));
    }

    let mut leaning = vec![0_i32; grid.cells()];
    let mut heard = vec![0_i32; grid.cells()];
    for piled in &each {
        for (at, slope) in piled.iter().enumerate() {
            if *slope > 0.0 {
                leaning[at] += 1;
                heard[at] += 1;
            } else if *slope < 0.0 {
                leaning[at] -= 1;
                heard[at] += 1;
            }
        }
    }

    let named = heard.iter().filter(|one| **one > 0).count();
    eprintln!("of {} knobs, {named} hear any lesson at all", grid.cells());

    eprintln!("{:>10} {:>10} {:>12}", "agreed", "knobs", "of the named");
    let mut level = Vec::new();
    for least in [8_i32, 10, 12, 14, 16] {
        let held = leaning
            .iter()
            .zip(&heard)
            .filter(|(lean, heard)| **heard > 0 && lean.abs() >= least)
            .count();
        eprintln!(
            "{least:>9}+ {held:>10} {:>11.0}%",
            held as f64 * 100.0 / named.max(1) as f64
        );
        level.push((least, held));
    }

    let split = leaning
        .iter()
        .zip(&heard)
        .filter(|(lean, heard)| **heard > 0 && lean.abs() <= 2)
        .count();
    eprintln!("{split} of the named knobs are pulled within one lesson of evenly, and go nowhere");
    eprintln!("twenty rounds of descent left 367 knobs somewhere other than where they started");
    let firm = level
        .iter()
        .find(|(least, _)| *least == 12)
        .map_or(0, |(_, held)| *held);
    eprintln!("the knobs twelve or more lessons agree about number {firm}");
    assert_eq!(
        named,
        grid.cells() / 2,
        "other than half the knobs hear a lesson"
    );
    let ten = level
        .iter()
        .find(|(least, _)| *least == 10)
        .map_or(usize::MAX, |(_, held)| *held);
    assert_eq!(
        ten, 0,
        "there is a knob ten of the sixteen lessons agree about after all"
    );
    assert!(
        split as f64 > named as f64 * 0.9,
        "the lessons are not evenly split after all: {split} of {named}"
    );
}
