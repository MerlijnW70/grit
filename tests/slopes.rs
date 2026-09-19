#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod lifting;

use grit::Groups;
use grit::sand::Grid;
use lifting::*;

#[test]
fn the_angles_slope_is_the_slope_a_nudge_to_the_angle_measures() {
    let Some(host) = device("the_angles_slope_is_the_slope_a_nudge_to_the_angle_measures") else {
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

    let base = TURNING / 64;
    let swing = SWING / 64;
    let mut board = Board {
        resident,
        over: Groups::Fixed(grid.quads().div_ceil(width)),
        grid,
        depth: 1,
        angle: (base, swing),
        loud: 1,
        planted: false,
    };

    let start = spread(grid, 0x1234_5678);
    eprintln!(
        "{:>7} {:>8} {:>6} {:>16} {:>16} {:>9}",
        "steps", "at", "secant", "told", "a nudge says", "apart"
    );

    let mut worst = 0.0_f64;
    for depth in [1_u32, 4, 16, 64, 256, 1024] {
        board.depth = depth;
        board.tune(base, swing);
        let target = board.run(&start);

        for away in [16_u32, 64, 192] {
            let here = base + away;
            board.tune(here, swing);
            let ended = board.run(&start);
            let weight: Vec<u32> = ended
                .iter()
                .zip(&target)
                .map(|(one, other)| one.wrapping_sub(*other))
                .collect();
            board.carry(&start, &weight);
            let (told, _) = board.told();

            let step = (away / 4).max(2);
            board.tune(here + step, swing);
            let above = squared(&board.run(&start), &target);
            board.tune(here - step, swing);
            let below = squared(&board.run(&start), &target);
            let nudged = (above - below) / f64::from(2 * step);

            let apart = (told - nudged).abs() / told.abs().max(nudged.abs()).max(1.0);
            eprintln!("{depth:>7} {here:>8} {step:>6} {told:>16.1} {nudged:>16.1} {apart:>8.4}");
            worst = worst.max(apart);
            assert!(told * nudged > 0.0, "{depth} {here} {told} {nudged}");
        }
    }

    eprintln!("worst disagreement anywhere {worst:.4}");
    assert!(worst < 0.10, "{worst}");
}

#[test]
fn the_angle_is_found_by_walking_the_slope_that_was_checked() {
    let Some(host) = device("the_angle_is_found_by_walking_the_slope_that_was_checked") else {
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

    eprintln!(
        "{:>7} {:>8} {:>8} {:>6} {:>34}",
        "steps", "from", "found", "asked", "the walk"
    );
    let mut told_reach = Vec::new();
    for depth in [1_u32, 16, 256, 1024] {
        board.depth = depth;
        board.tune(truth, swing);
        let target = board.run(&start);

        let mut here = truth + 16;
        let mut walked = vec![here];
        let mut asked = 1_u32;

        let mut span = 1_i64;
        let low;
        let high;
        let leaning = asking(&mut board, &start, &target, here) > 0.0;
        loop {
            let next = if leaning {
                i64::from(here) - span
            } else {
                i64::from(here) + span
            };
            let held = next.clamp(1, 4096) as u32;
            walked.push(held);
            asked += 1;
            let now = asking(&mut board, &start, &target, held) > 0.0;
            if now != leaning || span > 512 {
                (low, high) = if leaning {
                    (i64::from(held), i64::from(here))
                } else {
                    (i64::from(here), i64::from(held))
                };
                break;
            }
            span *= 2;
        }
        let (mut low, mut high) = (low, high);

        while high - low > 1 {
            let middle = (low + high) / 2;
            let held = middle.clamp(1, 4096) as u32;
            walked.push(held);
            asked += 1;
            if asking(&mut board, &start, &target, held) > 0.0 {
                high = middle;
            } else {
                low = middle;
            }
        }
        here = low as u32;

        let shown = walked
            .iter()
            .take(6)
            .map(|one| one.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        eprintln!(
            "{depth:>7} {:>8} {here:>8} {asked:>6}   {shown}",
            truth + 16
        );
        told_reach.push((depth, here.abs_diff(truth)));
    }

    for (depth, off) in &told_reach {
        eprintln!("{depth:>5} steps: landed {off} from {truth}");
    }
    let deepest = told_reach.last().map_or(u32::MAX, |(_, off)| *off);
    assert!(deepest <= 1, "{deepest}");
}

#[test]
fn the_smallest_difference_the_slope_can_point_at() {
    let Some(host) = device("the_smallest_difference_the_slope_can_point_at") else {
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

    let grids = [spread(grid, 0x1234_5678), spread(grid, 0x5EED_1057)];
    let aways = [1_u32, 2, 4, 8, 16, 32, 64];

    eprintln!(
        "{:>7} {}  {:>8}",
        "steps",
        aways
            .iter()
            .map(|one| format!("{one:>5}"))
            .collect::<String>(),
        "finest"
    );

    let mut found = Vec::new();
    for depth in [1_u32, 4, 16, 64, 256, 1024] {
        board.depth = depth;
        let targets: Vec<Vec<u32>> = grids
            .iter()
            .map(|start| {
                board.tune(truth, swing);
                board.run(start)
            })
            .collect();

        let mut shown = String::new();
        let mut finest = None;
        for away in aways {
            let mut sound = true;
            for (start, target) in grids.iter().zip(&targets) {
                let above = asking(&mut board, start, target, truth + away);
                let below = asking(&mut board, start, target, truth - away);
                if above <= 0.0 || below >= 0.0 {
                    sound = false;
                }
            }
            shown.push_str(if sound { "    y" } else { "    ." });
            if sound && finest.is_none() {
                finest = Some(away);
            }
        }
        match finest {
            Some(away) => {
                eprintln!("{depth:>7} {shown}  {away:>8}");
                found.push((depth, away));
            }
            None => eprintln!("{depth:>7} {shown}  {:>8}", "none"),
        }
    }
    board.tune(truth, swing);

    for (depth, away) in &found {
        eprintln!("{depth:>5} steps: points home from {away} out, both ways, on both grids");
    }
    let coarse = found.first().map_or(0, |(_, away)| *away);
    let fine = found.last().map_or(0, |(_, away)| *away);
    assert!(coarse >= fine, "{coarse} {fine}");
}

#[test]
fn what_depth_buys_is_signal_and_signal_buys_room_for_noise() {
    let Some(host) = device("what_depth_buys_is_signal_and_signal_buys_room_for_noise") else {
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
    let louds = [0_u32, 16, 64, 256, 1024, 4096];

    eprintln!(
        "{:>7} {}",
        "steps",
        louds
            .iter()
            .map(|one| format!("{:>10}", format!("noise {one}")))
            .collect::<String>()
    );

    let mut told = Vec::new();
    for depth in [1_u32, 16, 256, 1024] {
        board.depth = depth;
        board.tune(truth, swing);
        let clean = board.run(&start);

        let mut shown = String::new();
        let mut stood = 0_u32;
        for loud in louds {
            let target = if loud == 0 {
                clean.clone()
            } else {
                muddied(&clean, grid, 0x0FFE_1234, loud)
            };
            let found = hunted(&mut board, &start, &target, truth + 16);
            shown.push_str(&format!("{:>10}", found.abs_diff(truth)));
            if found.abs_diff(truth) <= 1 {
                stood = loud;
            }
        }
        eprintln!("{depth:>7} {shown}");
        told.push((depth, stood));
    }
    board.tune(truth, swing);

    for (depth, stood) in &told {
        eprintln!("{depth:>5} steps: still lands on the answer through noise of {stood}");
    }
    let shallow = told.first().map_or(0, |(_, stood)| *stood);
    let deep = told.last().map_or(0, |(_, stood)| *stood);
    assert!(deep >= shallow, "{shallow} {deep}");
}

#[test]
fn the_angle_is_found_where_the_tape_would_not_fit() {
    let Some(host) = device("the_angle_is_found_where_the_tape_would_not_fit") else {
        return;
    };
    let width = host.width();
    let truth = TURNING / 64;
    let swing = SWING / 64;
    let depth = 1024_u32;

    eprintln!(
        "{:>12} {:>10} {:>12} {:>8} {:>8} {:>10}",
        "cells", "held", "a tape", "found", "times", "took"
    );

    let mut told = Vec::new();
    for side in [64_u32, 512, 2048, 4096] {
        let grid = Grid::of(side, side);
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
        let mut board = Board {
            resident,
            over: Groups::Fixed(grid.quads().div_ceil(width)),
            grid,
            depth,
            angle: (truth, swing),
            loud: 1,
            planted: false,
        };

        let start = spread(grid, 0x1234_5678);
        board.tune(truth, swing);
        let target = board.run(&start);

        let began = std::time::Instant::now();
        let found = hunted(&mut board, &start, &target, truth + 16);
        let took = began.elapsed();

        let held = (grid.cells() * size_of::<u32>() * 4) as f64 / (1 << 20) as f64;
        let tape = f64::from(depth) * (grid.cells() * size_of::<u32>()) as f64 / (1 << 30) as f64;
        eprintln!(
            "{:>12} {held:>7.0} MiB {tape:>9.1} GiB {found:>8} {:>8.0} {took:>10.2?}",
            grid.cells(),
            tape * 1024.0 / held
        );
        told.push((grid.cells(), found));

        if side == 4096 {
            let away = 192_u32;
            let here = truth + away;
            let slope = asking(&mut board, &start, &target, here);
            let step = away / 4;
            board.tune(here + step, swing);
            let above = squared(&board.run(&start), &target);
            board.tune(here - step, swing);
            let below = squared(&board.run(&start), &target);
            let nudged = (above - below) / f64::from(2 * step);
            let apart = (slope - nudged).abs() / slope.abs().max(nudged.abs()).max(1.0);
            eprintln!(
                "at {} cells the slope says {slope:.0} and a nudge says {nudged:.0}, {apart:.4} apart",
                grid.cells()
            );
            assert!(apart < 0.10, "{apart}");
        }
    }

    for (cells, found) in &told {
        eprintln!(
            "{cells} cells: landed {} from {truth}",
            found.abs_diff(truth)
        );
    }
    for (_, found) in &told {
        assert_eq!(*found, truth);
    }
}

#[test]
fn what_the_same_answer_costs_without_any_gradient_at_all() {
    let Some(host) = device("what_the_same_answer_costs_without_any_gradient_at_all") else {
        return;
    };
    let width = host.width();
    let truth = TURNING / 64;
    let swing = SWING / 64;

    eprintln!(
        "{:>10} {:>7} {:>26} {:>26}",
        "cells", "steps", "walking the slope", "no slope at all"
    );

    let mut told = Vec::new();
    for (side, depth) in [(64_u32, 256_u32), (512, 1024)] {
        let grid = Grid::of(side, side);
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
        let mut board = Board {
            resident,
            over: Groups::Fixed(grid.quads().div_ceil(width)),
            grid,
            depth,
            angle: (truth, swing),
            loud: 1,
            planted: false,
        };

        let start = spread(grid, 0x1234_5678);
        board.tune(truth, swing);
        let target = board.run(&start);

        let began = std::time::Instant::now();
        let found = hunted(&mut board, &start, &target, truth + 16);
        let slope_took = began.elapsed();

        let began = std::time::Instant::now();
        let (blind, runs) = narrowed(&mut board, &start, &target, truth + 16);
        let blind_took = began.elapsed();

        eprintln!(
            "{:>10} {depth:>7}   {found:>5} in 30 passes {slope_took:>8.2?}   {blind:>5} in {runs:>2} passes {blind_took:>8.2?}",
            grid.cells()
        );
        told.push((grid.cells(), found, blind, runs));
    }

    for (cells, found, blind, runs) in &told {
        eprintln!(
            "{cells} cells: the slope found {found}, {runs} plain runs found {blind}, the answer is {truth}"
        );
    }
    for (_, found, blind, _) in &told {
        assert_eq!(*found, truth);
        assert_eq!(*blind, truth);
    }
}

#[test]
fn one_cells_share_of_the_slope_is_what_nudging_that_cell_measures() {
    let Some(host) = device("one_cells_share_of_the_slope_is_what_nudging_that_cell_measures")
    else {
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

    let base = TURNING / 64;
    let swing = SWING / 64;
    let mut board = Board {
        resident,
        over: Groups::Fixed(grid.quads().div_ceil(width)),
        grid,
        depth: 64,
        angle: (base, swing),
        loud: 1,
        planted: false,
    };

    let start = spread(grid, 0x1234_5678);
    let scatter = spread(grid, 0x00C0_FFEE);
    let truth: Vec<u32> = scatter.iter().map(|one| base + (one % 33) - 16).collect();
    board.tune(base, swing);
    board.sow(&truth);
    let target = board.run(&start);

    let guess = vec![base; grid.cells()];
    board.sow(&guess);
    let ended = board.run(&start);
    let weight: Vec<u32> = ended
        .iter()
        .zip(&target)
        .map(|(one, other)| one.wrapping_sub(*other))
        .collect();
    board.sow(&guess);
    board.carry(&start, &weight);
    let told = board.told_each();
    let back = f64::from(1_u32 << SPARE) * f64::from(board.loud);

    let mut ranked: Vec<(usize, i32)> = told
        .iter()
        .enumerate()
        .map(|(at, one)| (at, *one))
        .collect();
    ranked.sort_by_key(|(_, one)| -one.abs());

    eprintln!(
        "{:>8} {:>16} {:>16} {:>9}",
        "cell", "its share", "nudging it says", "apart"
    );
    let mut told_worst = Vec::new();
    for step in [8_i64, 32, 128] {
        let mut worst = 0.0_f64;
        let mut seen = 0_usize;
        for (at, raw) in ranked.iter().take(6) {
            let mine = f64::from(*raw) / back;

            let mut above = guess.clone();
            above[*at] = (i64::from(guess[*at]) + step) as u32;
            board.sow(&above);
            let high = squared(&board.run(&start), &target);

            let mut below = guess.clone();
            below[*at] = (i64::from(guess[*at]) - step) as u32;
            board.sow(&below);
            let low = squared(&board.run(&start), &target);

            let nudged = (high - low) / (2.0 * step as f64);
            let apart = (mine - nudged).abs() / mine.abs().max(nudged.abs()).max(1.0);
            if step == 32 {
                eprintln!("{at:>8} {mine:>16.1} {nudged:>16.1} {apart:>8.4}");
            }
            worst = worst.max(apart);
            seen += 1;
        }
        eprintln!("a nudge of {step:>4}: worst of {seen} is {worst:.4}");
        told_worst.push((step, worst));
    }

    let best = told_worst
        .iter()
        .map(|(_, one)| *one)
        .fold(f64::MAX, f64::min);
    let ends = told_worst
        .iter()
        .filter(|(step, _)| *step != 32)
        .map(|(_, one)| *one)
        .fold(0.0_f64, f64::max);
    eprintln!("best {best:.4}, and worse at both ends, up to {ends:.4}");
    assert!(best < 0.05, "{best}");
    assert!(ends > best, "{best} {ends}");
}

#[test]
fn many_numbers_at_once_are_what_the_walk_back_is_for() {
    let Some(host) = device("many_numbers_at_once_are_what_the_walk_back_is_for") else {
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

    let base = TURNING / 64;
    let swing = SWING / 64;
    let mut board = Board {
        resident,
        over: Groups::Fixed(grid.quads().div_ceil(width)),
        grid,
        depth: 256,
        angle: (base, swing),
        loud: 1,
        planted: false,
    };

    let start = spread(grid, 0x1234_5678);
    let scatter = spread(grid, 0x00C0_FFEE);

    let probe: Vec<u32> = scatter.iter().map(|one| base + (one % 33) - 16).collect();
    board.tune(base, swing);
    board.sow(&probe);
    let seen = board.run(&start);
    let weight: Vec<u32> = seen.iter().map(|one| one.wrapping_sub(base)).collect();
    board.sow(&probe);
    board.carry(&start, &weight);
    let live: Vec<bool> = board.told_each().iter().map(|one| *one != 0).collect();
    let counted = live.iter().filter(|one| **one).count();

    let truth: Vec<u32> = live
        .iter()
        .zip(&scatter)
        .map(|(on, one)| if *on { base + (one % 33) - 16 } else { base })
        .collect();

    eprintln!("{:>7} {:>10} {:>12}", "steps", "have one", "point right");
    for depth in [16_u32, 64, 256, 1024] {
        board.depth = depth;
        board.sow(&truth);
        let aim = board.run(&start);
        let guess = vec![base; grid.cells()];
        board.sow(&guess);
        let ended = board.run(&start);
        let weight: Vec<u32> = ended
            .iter()
            .zip(&aim)
            .map(|(one, other)| one.wrapping_sub(*other))
            .collect();
        board.sow(&guess);
        board.carry(&start, &weight);
        let told = board.told_each();
        let mut agreed = 0_usize;
        let mut spoke = 0_usize;
        for (((slope, held), want), on) in told.iter().zip(&guess).zip(&truth).zip(&live) {
            if !*on || *slope == 0 {
                continue;
            }
            spoke += 1;
            let off = (*held as i32) - (*want as i32);
            if off != 0 && slope.signum() == off.signum() {
                agreed += 1;
            }
        }
        eprintln!(
            "{depth:>7} {spoke:>10} {:>11.1}%",
            agreed as f64 * 100.0 / spoke.max(1) as f64
        );
    }
    board.depth = 1024;
    board.sow(&truth);
    let target = board.run(&start);

    let mut guess = vec![base; grid.cells()];
    let first = strayed_where(&guess, &truth, &live);

    let schedule = [1_u32; 32];
    let began = std::time::Instant::now();
    let mut walk: Vec<(f64, f64)> = Vec::new();
    for step in schedule {
        board.sow(&guess);
        let ended = board.run(&start);
        let weight: Vec<u32> = ended
            .iter()
            .zip(&target)
            .map(|(one, other)| one.wrapping_sub(*other))
            .collect();
        board.sow(&guess);
        board.carry(&start, &weight);
        let told = board.told_each();
        for (held, slope) in guess.iter_mut().zip(&told) {
            let moved = i64::from(*held) - i64::from(slope.signum()) * i64::from(step);
            *held = moved.clamp(1, 4096) as u32;
        }
        board.sow(&guess);
        walk.push((
            strayed_where(&guess, &truth, &live),
            squared(&board.run(&start), &target),
        ));
    }
    let took = began.elapsed();
    let last = strayed_where(&guess, &truth, &live);

    eprintln!("{counted} numbers, {} steps", board.depth);
    eprintln!(
        "out by {first:.2} at the start and {last:.2} after {} rounds, {took:.2?}",
        schedule.len()
    );
    let away = walk
        .iter()
        .enumerate()
        .filter(|(at, _)| at % 4 == 3)
        .map(|(_, (one, _))| format!("{one:.2}"))
        .collect::<Vec<_>>()
        .join(" ");
    let loss = walk
        .iter()
        .enumerate()
        .filter(|(at, _)| at % 4 == 3)
        .map(|(_, (_, one))| format!("{one:.3e}"))
        .collect::<Vec<_>>()
        .join(" ");
    eprintln!("away from the truth: {away}");
    eprintln!("the loss itself:     {loss}");
    eprintln!(
        "{} passes for all of them; one run apiece would be {}",
        schedule.len() * 3,
        schedule.len() * counted
    );
    let began_loss = walk.first().map_or(0.0, |(_, one)| *one);
    let ended_loss = walk.last().map_or(0.0, |(_, one)| *one);
    eprintln!("the loss went {began_loss:.3e} to {ended_loss:.3e}");
    assert!(ended_loss < began_loss, "{began_loss} {ended_loss}");
}

#[test]
fn whether_the_plateau_is_the_step_or_the_place() {
    let Some(host) = device("whether_the_plateau_is_the_step_or_the_place") else {
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

    let base = TURNING / 64;
    let swing = SWING / 64;
    let mut board = Board {
        resident,
        over: Groups::Fixed(grid.quads().div_ceil(width)),
        grid,
        depth: 256,
        angle: (base, swing),
        loud: 1,
        planted: false,
    };

    let start = spread(grid, 0x1234_5678);
    let scatter = spread(grid, 0x00C0_FFEE);
    let truth: Vec<u32> = scatter.iter().map(|one| base + (one % 33) - 16).collect();
    board.tune(base, swing);
    board.sow(&truth);
    let target = board.run(&start);

    let rounds = 24_u32;
    eprintln!(
        "{:>12} {:>12} {:>12} {:>12} {:>12}",
        "way", "loss first", "loss last", "slope first", "slope last"
    );

    let mut told_ways = Vec::new();
    for way in ["one each", "halving", "by size", "searched"] {
        let mut guess = vec![base; grid.cells()];
        let mut first = 0.0;
        let mut strong_first = 0.0;
        for round in 0..rounds {
            let (told, loss, strength) = asked_for(&mut board, &start, &target, &guess);
            if round == 0 {
                first = loss;
                strong_first = strength;
            }

            let widest = told.iter().map(|one| one.abs()).max().unwrap_or(1).max(1);
            if way == "searched" {
                let mut best = guess.clone();
                let mut held = loss;
                for step in [8_i64, 4, 2, 1] {
                    let tried: Vec<u32> = guess
                        .iter()
                        .zip(&told)
                        .map(|(one, slope)| {
                            let moved = i64::from(*one) - i64::from(slope.signum()) * step;
                            moved.clamp(1, 4096) as u32
                        })
                        .collect();
                    board.sow(&tried);
                    let now = squared(&board.run(&start), &target);
                    if now < held {
                        held = now;
                        best = tried;
                    }
                }
                guess = best;
                continue;
            }
            for (held, slope) in guess.iter_mut().zip(&told) {
                let moved = match way {
                    "one each" => i64::from(*held) - i64::from(slope.signum()),
                    "halving" => {
                        let step = 8_i64 >> (round / 6).min(3);
                        i64::from(*held) - i64::from(slope.signum()) * step
                    }
                    _ => {
                        let share = f64::from(*slope) / f64::from(widest);
                        i64::from(*held) - (share * 4.0).round() as i64
                    }
                };
                *held = moved.clamp(1, 4096) as u32;
            }
        }
        let (_, last, strong_last) = asked_for(&mut board, &start, &target, &guess);
        eprintln!(
            "{way:>12} {first:>12.3e} {last:>12.3e} {strong_first:>12.2} {strong_last:>12.2}"
        );
        told_ways.push((way, first, last, strong_first, strong_last));
    }

    for (way, first, last, _, _) in &told_ways {
        eprintln!("{way}: {:.2}x down", first / last.max(1.0));
    }
    let best = told_ways
        .iter()
        .map(|(_, first, last, _, _)| first / last.max(1.0))
        .fold(0.0_f64, f64::max);
    let standing = told_ways
        .iter()
        .find(|(way, _, _, _, _)| *way == "one each")
        .map_or((0.0, 0.0), |(_, _, _, began, ended)| (*began, *ended));
    eprintln!(
        "the simplest step leaves the slope at {:.0}% of what it started",
        standing.1 * 100.0 / standing.0.max(1.0)
    );
    assert!(best > 1.5, "{best}");
    assert!(standing.1 > standing.0 / 3.0, "{standing:?}");
}
