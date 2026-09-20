#![allow(dead_code)]
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use crate::lifting::*;

use grit::sand::Grid;

pub const SIDE: u32 = 64;

pub const DEPTH: u32 = 64;

pub const BLOB: u32 = 6;

pub const MASS: u32 = 1 << 14;

pub const GROUND: u32 = 1 << 11;

pub const HAZE: u32 = 1 << 13;

pub const READ: u32 = 8;

pub const SITTINGS: u32 = 60;

pub const STRIDE: i64 = 4;

pub fn blotted_in(grid: Grid, rows: (u32, u32), seed: u32, haze: u32) -> Vec<u32> {
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

pub fn window(grid: Grid) -> Vec<usize> {
    let from = grid.wide / 2 - READ / 2;
    let mut held = Vec::new();
    for row in 0..grid.deep {
        for column in from..from + READ {
            held.push((row * grid.wide + column) as usize);
        }
    }
    held
}

pub fn read_out(ended: &[u32], seen: &[usize]) -> f64 {
    seen.iter()
        .map(|at| f64::from(ended[*at] as i32))
        .sum::<f64>()
        / seen.len() as f64
}

pub struct Lesson {
    pub start: Vec<u32>,
    pub left: bool,
}

pub fn blotted(grid: Grid, rows: (u32, u32), seed: u32) -> Vec<u32> {
    blotted_in(grid, rows, seed, HAZE)
}

pub fn lessons_in(grid: Grid, from: u32, count: u32, haze: u32) -> Vec<Lesson> {
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

pub fn lessons(grid: Grid, from: u32, count: u32) -> Vec<Lesson> {
    lessons_in(grid, from, count, HAZE)
}

pub const SPLIT: f64 = 300.0;

pub const PITCH: f64 = 256.0;

pub fn wanted(left: bool, middle: f64) -> f64 {
    if left { middle + SPLIT } else { middle - SPLIT }
}

pub fn spread_of(board: &mut Board<'_>, held: &[Lesson], seen: &[usize], left: bool) -> (f64, f64) {
    let mut low = f64::MAX;
    let mut high = f64::MIN;
    for one in held.iter().filter(|one| one.left == left) {
        let got = read_out(&board.run(&one.start).unwrap(), seen);
        low = low.min(got);
        high = high.max(got);
    }
    (low, high)
}

pub fn parted(board: &mut Board<'_>, held: &[Lesson], seen: &[usize]) -> (f64, f64) {
    let mut left = (0.0_f64, 0.0_f64);
    let mut right = (0.0_f64, 0.0_f64);
    for one in held {
        let got = read_out(&board.run(&one.start).unwrap(), seen);
        if one.left {
            left = (left.0 + got, left.1 + 1.0);
        } else {
            right = (right.0 + got, right.1 + 1.0);
        }
    }
    (left.0 / left.1.max(1.0), right.0 / right.1.max(1.0))
}

pub fn scored(board: &mut Board<'_>, held: &[Lesson], seen: &[usize], line: f64) -> f64 {
    let mut right = 0.0;
    for one in held {
        let got = read_out(&board.run(&one.start).unwrap(), seen);
        if (got > line) == one.left {
            right += 1.0;
        }
    }
    right / held.len() as f64
}

pub fn told_on(board: &mut Board<'_>, held: &[Lesson], seen: &[usize], middle: f64) -> f64 {
    let mut loss = 0.0;
    for one in held {
        let got = read_out(&board.run(&one.start).unwrap(), seen);
        let want = wanted(one.left, middle);
        loss += (got - want) * (got - want);
    }
    loss / held.len() as f64
}

pub fn slope_of(
    board: &mut Board<'_>,
    held: &[Lesson],
    seen: &[usize],
    knobs: &[u32],
    middle: f64,
) -> Vec<f64> {
    let mut piled = vec![0.0_f64; knobs.len()];
    for one in held {
        board.sow(knobs).unwrap();
        let got = read_out(&board.run(&one.start).unwrap(), seen);
        let off = got - wanted(one.left, middle);
        let share = (off / seen.len() as f64 * PITCH).round() as i32;
        let mut weight = vec![0_u32; knobs.len()];
        for at in seen {
            weight[*at] = share as u32;
        }
        board.sow(knobs).unwrap();
        board.carry(&one.start, &weight).unwrap();
        let scale = f64::from(1_u32 << SPARE) * f64::from(board.loud) * PITCH;
        for (into, slope) in piled.iter_mut().zip(board.told_each().unwrap()) {
            *into += f64::from(slope) / scale;
        }
    }
    piled
}

pub fn kept_of(grid: Grid, whole: bool, seen: &[usize]) -> Vec<usize> {
    if whole {
        (0..grid.cells()).collect()
    } else {
        seen.to_vec()
    }
}

pub fn settled_by(
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

pub fn carried_from(
    board: &mut Board<'_>,
    held: &[Lesson],
    seen: &[usize],
    from: &[f64],
    middle: f64,
    pace: f64,
    rounds: u32,
) -> Vec<f64> {
    let mut shadow = from.to_vec();
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

pub fn best_along(
    board: &mut Board<'_>,
    held: &[Lesson],
    seen: &[usize],
    middle: f64,
    from: &[f64],
    way: &[f64],
) -> (f64, f64, bool) {
    let loss_of = |board: &mut Board<'_>, along: f64| {
        let put: Vec<u32> = from
            .iter()
            .zip(way)
            .map(|(one, step)| {
                (one + along * step)
                    .clamp(1.0, 4096.0)
                    .round()
                    .clamp(1.0, 4096.0) as u32
            })
            .collect();
        board.sow(&put).unwrap();
        told_on(board, held, seen, middle)
    };
    let mut reach = 4.0_f64;
    let mut best = (0.0_f64, loss_of(board, 0.0));
    loop {
        let mut found = best;
        let mut at = -reach;
        while at <= reach {
            if at != 0.0 {
                let loss = loss_of(board, at);
                if loss < found.1 {
                    found = (at, loss);
                }
            }
            at += reach / 16.0;
        }
        best = found;
        if best.0.abs() < reach - reach / 16.0 || reach >= 512.0 {
            break;
        }
        reach *= 2.0;
    }
    let mut span = reach / 16.0;
    while span > 1.0 / 64.0 {
        let mut found = best;
        for step in [-2_i32, -1, 1, 2] {
            let at = best.0 + f64::from(step) * span / 2.0;
            if at == 0.0 {
                continue;
            }
            let loss = loss_of(board, at);
            if loss < found.1 {
                found = (at, loss);
            }
        }
        best = found;
        span /= 2.0;
    }
    (best.0, best.1, best.0.abs() >= reach - reach / 16.0)
}
