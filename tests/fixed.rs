#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod lifting;

use grit::compact::Plan;
use grit::layout::STATE;
use grit::live;
use grit::sand::Grid;
use grit::{Groups, sweep};
use lifting::*;

#[test]
fn the_slope_the_weights_report_is_the_slope_a_nudge_measures() {
    let Some(host) = device("the_slope_the_weights_report_is_the_slope_a_nudge_measures") else {
        return;
    };
    let width = host.width();
    let grid = Grid::of(WIDE, DEEP);
    let passes = [
        built(width, grid, Pass::Ahead, Kind::Product).unwrap(),
        built(width, grid, Pass::Back, Kind::Product).unwrap(),
    ];
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();

    let sizes = [
        8,
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
    ];
    let resident = host.resident(&sizes, &modules).unwrap();
    let mut board = Board {
        resident,
        over: Groups::Fixed(grid.quads().div_ceil(width)),
        grid,
        depth: STEPS,
        angle: (TURNING, SWING),
        loud: 1,
        planted: false,
    };

    let start = spread(grid, 0x1234_5678);
    let watched = (grid.deep / 2 * grid.wide + grid.wide / 2) as usize;
    let carried = board.slope(&start, watched);

    let mut seen = 0;
    let mut worst = 0.0_f64;
    for at in [
        watched,
        watched - 1,
        watched + 1,
        watched - WIDE as usize,
        7,
        4095,
    ] {
        let told = carried[at];
        if told == 0 {
            continue;
        }
        let mut nudged = start.clone();
        nudged[at] = nudged[at].wrapping_add(STEP);
        let moved = board.run(&nudged)[watched].wrapping_sub(board.run(&start)[watched]);

        let wanted = ((u64::from(told) * u64::from(STEP)) >> POINT) as u32;
        let apart = moved.abs_diff(wanted);
        let share = f64::from(apart) / f64::from(moved.max(wanted).max(1));
        eprintln!("{at} {told} {moved} {wanted} {apart} {share:.4}");
        assert!(apart <= GRAIN || share < 0.05, "{at} {moved} {wanted}");
        worst = worst.max(share);
        seen += 1;
    }

    assert!(seen >= 3);
    eprintln!("{worst:.4}");
}

#[test]
fn the_bounded_rules_slope_is_the_slope_a_nudge_measures() {
    let Some(host) = device("the_bounded_rules_slope_is_the_slope_a_nudge_measures") else {
        return;
    };
    let width = host.width();
    let grid = Grid::of(WIDE, DEEP);
    let passes = [
        built(width, grid, Pass::Ahead, Kind::Bound).unwrap(),
        built(width, grid, Pass::Back, Kind::Bound).unwrap(),
    ];
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();

    let sizes = [
        8,
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
    ];
    let resident = host.resident(&sizes, &modules).unwrap();
    let mut board = Board {
        resident,
        over: Groups::Fixed(grid.quads().div_ceil(width)),
        grid,
        depth: STEPS,
        angle: (TURNING, SWING),
        loud: 1,
        planted: false,
    };

    let start = spread(grid, 0x1234_5678);
    let watched = (grid.deep / 2 * grid.wide + grid.wide / 2) as usize;
    let carried = board.slope(&start, watched);
    let settled = board.run(&start)[watched];

    let mut seen = 0;
    let mut worst = 0.0_f64;
    for at in 0..grid.cells() {
        let told = carried[at];
        if told == 0 || seen >= 6 {
            continue;
        }
        let mut nudged = start.clone();
        nudged[at] = nudged[at].wrapping_add(STEP);
        let moved = board.run(&nudged)[watched].wrapping_sub(settled);
        let wanted = (((told as i32) as i64 * i64::from(STEP)) >> POINT) as i32 as u32;
        let apart = moved.abs_diff(wanted);
        let share = f64::from(apart) / f64::from(moved.max(wanted).max(1));
        eprintln!(
            "{at} {} {} {} {apart} {share:.4}",
            told as i32, moved as i32, wanted as i32
        );
        worst = worst.max(share);
        seen += 1;
    }

    eprintln!("{seen} points, worst {worst:.4}");
    assert!(seen >= 3);
    assert!(worst < 0.05, "{worst}");
}

#[test]
fn whether_the_weight_stays_the_size_it_started() {
    let Some(host) = device("whether_the_weight_stays_the_size_it_started") else {
        return;
    };
    let width = host.width();
    let grid = Grid::of(WIDE, DEEP);
    let passes = [
        built(width, grid, Pass::Ahead, Kind::Smooth).unwrap(),
        built(width, grid, Pass::Back, Kind::Smooth).unwrap(),
    ];
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let sizes = [
        8,
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
    ];
    let resident = host.resident(&sizes, &modules).unwrap();

    let truth = TURNING / 64;
    let swing = SWING / 64;
    let mut board = Board {
        resident,
        over: Groups::Fixed(grid.quads().div_ceil(width)),
        grid,
        depth: 1,
        angle: (truth, swing),
        loud: 1,
        planted: false,
    };
    let start = spread(grid, 0x1234_5678);
    let biggest = |held: &[u32]| {
        held.iter()
            .map(|one| f64::from(*one as i32).abs())
            .fold(0.0_f64, f64::max)
    };

    eprintln!(
        "{:>7} {:>16} {:>16} {:>9}",
        "steps", "weight put in", "weight come back", "grown"
    );
    for depth in [16_u32, 64, 256, 1024] {
        board.depth = depth;
        board.tune(truth, swing);
        let target = board.run(&start);
        board.tune(truth + 16, swing);
        let ended = board.run(&start);
        let weight: Vec<u32> = ended
            .iter()
            .zip(&target)
            .map(|(one, other)| one.wrapping_sub(*other))
            .collect();
        let (carried, _) = board.carry(&start, &weight);
        let went = biggest(&weight);
        let came = biggest(&carried);
        eprintln!(
            "{depth:>7} {went:>16.0} {came:>16.0} {:>8.1}x",
            came / went.max(1.0)
        );
    }
    board.tune(truth, swing);
}

#[test]
fn the_grid_is_talked_back_to_the_start_it_came_from() {
    let Some(host) = device("the_grid_is_talked_back_to_the_start_it_came_from") else {
        return;
    };
    let width = host.width();
    let grid = Grid::of(WIDE, DEEP);
    let passes = [
        built(width, grid, Pass::Ahead, Kind::Product).unwrap(),
        built(width, grid, Pass::Back, Kind::Product).unwrap(),
    ];
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let sizes = [
        8,
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
    ];
    let resident = host.resident(&sizes, &modules).unwrap();
    let mut board = Board {
        resident,
        over: Groups::Fixed(grid.quads().div_ceil(width)),
        grid,
        depth: STEPS,
        angle: (TURNING, SWING),
        loud: 1,
        planted: false,
    };

    let truth = spread(grid, 0x1234_5678);
    let target = board.run(&truth);

    let drift = spread(grid, 0x0BAD_C0DE);
    let mut guess: Vec<u32> = truth
        .iter()
        .zip(&drift)
        .map(|(one, other)| one.wrapping_add(other >> 2))
        .collect();

    let strayed = apart(&guess, &truth);
    let first = apart(&board.run(&guess), &target);
    let mut last = first;
    let mut steps = Vec::new();
    for round in 0..ROUNDS {
        let ended = board.run(&guess);
        let weight: Vec<u32> = ended
            .iter()
            .zip(&target)
            .map(|(one, other)| one.wrapping_sub(*other))
            .collect();
        let (slope, _) = board.carry(&guess, &weight);
        if round == 0 {
            let strength: f64 = slope
                .iter()
                .map(|one| f64::from(*one as i32).abs())
                .sum::<f64>()
                / slope.len() as f64;
            let pull: f64 = weight
                .iter()
                .map(|one| f64::from(*one as i32).abs())
                .sum::<f64>()
                / weight.len() as f64;
            eprintln!("pull {pull:.1} slope {strength:.1}");
        }
        for (cell, step) in guess.iter_mut().zip(&slope) {
            let moved = i64::from(*cell) - i64::from((*step as i32) >> PACE);
            *cell = moved.clamp(0, i64::from(CEILING) - 1) as u32;
        }
        last = apart(&board.run(&guess), &target);
        if round % 8 == 0 || round + 1 == ROUNDS {
            steps.push((round, last));
        }
    }

    for (round, held) in &steps {
        eprintln!("{round} {held:.1}");
    }
    let found = apart(&guess, &truth);
    eprintln!("{first:.1} {last:.1} {strayed:.1} {found:.1}");

    assert!(last < first / 10.0, "{first} {last}");
    assert!(found < strayed / 10.0, "{strayed} {found}");
}

#[test]
fn the_patch_is_talked_back_while_the_rest_of_the_grid_sleeps() {
    let Some(host) = device("the_patch_is_talked_back_while_the_rest_sleeps") else {
        return;
    };
    let width = host.width();
    let grid = Grid::of(WIDER, WIDER);
    let plan = Plan::over(width, grid.quads().div_ceil(sweep::cells(width))).unwrap();
    let keeping = live::keeping(width, &plan, false).unwrap();
    let none = Groups::Fixed(0);

    let shaped = |first: Groups, second: Groups| {
        let mut held = keeping.before.clone();
        held.push(vec![
            (over_list(width, grid, Pass::Ahead).unwrap(), first),
            (over_list(width, grid, Pass::Back).unwrap(), second),
        ]);
        held.extend(keeping.after.clone());
        held
    };
    let (passes, ahead) = live::staged(shaped(keeping.over, none));
    let (_, back) = live::staged(shaped(none, keeping.over));
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();

    let mut sizes = plan.sizes();
    sizes[SLOPE as usize] = sizes[STATE as usize];
    let resident = host.resident(&sizes, &modules).unwrap();
    let mut board = Listed {
        resident,
        ahead,
        back,
        grid,
        plan,
    };

    let truth = patched(grid, 0x1234_5678, PATCH);
    let (target, awake) = board.run(&truth);
    let stirred = worst(&target, &truth);
    let touched = target.iter().filter(|word| **word != 0).count();
    eprintln!("{} {awake} {stirred:.1} {touched}", grid.quads());

    let drift = patched(grid, 0x0BAD_C0DE, PATCH);
    let mut guess: Vec<u32> = truth
        .iter()
        .zip(&drift)
        .map(|(one, other)| one.wrapping_add(other >> 2))
        .collect();

    let strayed = worst(&guess, &truth);
    let first = worst(&board.run(&guess).0, &target);
    let mut last = first;
    for round in 0..ROUNDS {
        let (ended, _) = board.run(&guess);
        let weight: Vec<u32> = ended
            .iter()
            .zip(&target)
            .map(|(one, other)| one.wrapping_sub(*other))
            .collect();
        let slope = board.carry(&ended, &weight);
        for (cell, step) in guess.iter_mut().zip(&slope) {
            let moved = i64::from(*cell) - i64::from((*step as i32) >> PACE);
            *cell = moved.clamp(0, i64::from(CEILING) - 1) as u32;
        }
        last = worst(&board.run(&guess).0, &target);
        if round % 8 == 0 {
            eprintln!("{round} {last:.1}");
        }
    }
    let found = worst(&guess, &truth);
    eprintln!("{first:.1} {last:.1} {strayed:.1} {found:.1}");

    assert!(awake < grid.quads() / 4);
    assert!(awake > 64, "{awake}");
    assert!(stirred > 1000.0, "{stirred}");
    assert!(touched > (PATCH * PATCH) as usize, "{touched}");
    assert!(last < first / 10.0);
    assert!(found < strayed / 10.0);
}
