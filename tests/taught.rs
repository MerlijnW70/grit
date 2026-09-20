#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod lessons;
mod lifting;

use grit::sand::Grid;
use lessons::*;
use lifting::*;

#[test]
fn where_the_blot_has_got_to_by_each_depth() {
    let Some(host) = device("where_the_blot_has_got_to_by_each_depth") else {
        return;
    };
    let grid = Grid::of(SIDE, SIDE);
    let base = 256_u32;
    let mut board = seated(&host, grid, Kind::Living, (base, 0), 1);
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
    let grid = Grid::of(SIDE, SIDE);
    let base = 256_u32;
    let mut board = seated(&host, grid, Kind::Living, (base, 0), DEPTH);
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
fn how_far_a_slope_from_a_small_window_reaches() {
    let Some(host) = device("how_far_a_slope_from_a_small_window_reaches") else {
        return;
    };
    let grid = Grid::of(SIDE, SIDE);
    let base = 256_u32;
    let mut board = seated(&host, grid, Kind::Living, (base, 0), 1);
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

#[test]
fn whether_a_step_down_the_slope_lowers_the_loss_at_all() {
    let Some(host) = device("whether_a_step_down_the_slope_lowers_the_loss_at_all") else {
        return;
    };
    let grid = Grid::of(SIDE, SIDE);
    let base = 256_u32;
    let mut board = seated(&host, grid, Kind::Living, (base, 0), DEPTH);

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
    let grid = Grid::of(SIDE, SIDE);
    let base = 256_u32;
    let mut board = seated(&host, grid, Kind::Living, (base, 0), DEPTH);

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

    for round in 0..SITTINGS {
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
                let moved = i64::from(*knob) - (slope.signum() as i64) * STRIDE;
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
fn whether_the_knobs_the_slope_never_names_matter_at_all() {
    let Some(host) = device("whether_the_knobs_the_slope_never_names_matter_at_all") else {
        return;
    };
    let grid = Grid::of(SIDE, SIDE);
    let base = 256_u32;
    let mut board = seated(&host, grid, Kind::Living, (base, 0), DEPTH);

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
    let grid = Grid::of(SIDE, SIDE);
    let base = 256_u32;
    let mut board = seated(&host, grid, Kind::Living, (base, 0), DEPTH);

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

#[test]
fn which_half_of_the_knobs_the_rule_leaves_silent() {
    let Some(host) = device("which_half_of_the_knobs_the_rule_leaves_silent") else {
        return;
    };
    let grid = Grid::of(SIDE, SIDE);
    let base = 256_u32;
    let seen = window(grid);
    let flat = vec![base; grid.cells()];
    let haze = 1024_u32;
    let taught = lessons_in(grid, 0x1234_5678, 16, haze);

    eprintln!(
        "{:>8} {:>6} {:>8} {:>9} {:>11} {:>11} {:>11}",
        "rule", "depth", "heard", "of them", "even sum", "even row", "even column"
    );
    let mut told = Vec::new();
    for kind in [Kind::Living, Kind::Smooth] {
        let mut reached = Vec::new();
        for depth in [1_u32, 4, 16, DEPTH] {
            let mut board = seated(&host, grid, kind, (base, 0), depth);
            board.sow(&flat);
            let middle = {
                let (left, right) = parted(&mut board, &taught, &seen);
                (left + right) / 2.0
            };
            let piled = slope_of(&mut board, &taught, &seen, &flat, middle);
            let heard: Vec<usize> = piled
                .iter()
                .enumerate()
                .filter(|(_, one)| **one != 0.0)
                .map(|(at, _)| at)
                .collect();
            let share = |pick: &dyn Fn(u32, u32) -> bool| {
                let held = heard
                    .iter()
                    .filter(|at| {
                        let row = **at as u32 / grid.wide;
                        let column = **at as u32 % grid.wide;
                        pick(row, column)
                    })
                    .count();
                held as f64 * 100.0 / heard.len().max(1) as f64
            };
            let evenly = share(&|row, column| (row + column).is_multiple_of(2));
            eprintln!(
                "{:>8} {depth:>6} {:>8} {:>8.0}% {evenly:>10.0}% {:>10.0}% {:>10.0}%",
                format!("{kind:?}"),
                heard.len(),
                heard.len() as f64 * 100.0 / grid.cells() as f64,
                share(&|row, _| row.is_multiple_of(2)),
                share(&|_, column| column.is_multiple_of(2))
            );
            reached.push(heard.len());
            told.push((kind, depth, heard.len(), evenly));
        }
        assert!(
            reached.windows(2).all(|two| two[0] < two[1]),
            "the slope does not reach further with depth: {reached:?}"
        );
    }

    let mut board = seated(&host, grid, Kind::Living, (base, 0), DEPTH);
    board.sow(&flat);
    let middle = {
        let (left, right) = parted(&mut board, &taught, &seen);
        (left + right) / 2.0
    };
    let piled = slope_of(&mut board, &taught, &seen, &flat, middle);
    eprintln!("the first four rows, a dot where the slope is silent:");
    for row in 0..4_u32 {
        let mut drawn = String::new();
        for column in 0..grid.wide.min(48) {
            let at = (row * grid.wide + column) as usize;
            drawn.push(if piled[at] == 0.0 { '.' } else { '#' });
        }
        eprintln!("  {row:>2} {drawn}");
    }
    eprintln!(
        "the rule loads its knob at the corner of the block, and `corners` puts that corner at"
    );
    eprintln!("(2a + phase, 2d + phase), which has an even sum in both phases");

    for (kind, depth, heard, evenly) in &told {
        assert!(
            (evenly - 100.0).abs() < f64::EPSILON,
            "{kind:?} at depth {depth} hears a knob off the even sublattice: {evenly}%"
        );
        if *depth == DEPTH {
            assert_eq!(
                *heard,
                grid.cells() / 2,
                "{kind:?} at depth {depth} hears other than one sublattice"
            );
        }
    }
}
