#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod lifting;

use grit::Groups;
use grit::sand::Grid;
use lifting::*;

#[test]
fn how_deep_the_descent_still_finds_its_way() {
    let Some(host) = device("how_deep_the_descent_still_finds_its_way") else {
        return;
    };
    let width = host.width();
    let grid = Grid::of(WIDE, DEEP);
    let sizes = [
        8,
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
    ];

    eprintln!(
        "{:>9} {:>7} {:>14} {:>14} {:>14} {:>14} {:>9}",
        "rule", "steps", "state", "slope", "error first", "error last", "shrank by"
    );
    let mut reach = Vec::new();
    for kind in [Kind::Product, Kind::Sum, Kind::Turn, Kind::Bound] {
        let passes = [
            built(width, grid, Pass::Ahead, kind).unwrap(),
            built(width, grid, Pass::Back, kind).unwrap(),
        ];
        let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
        let named = match kind {
            Kind::Product => "product",
            Kind::Sum => "sum",
            Kind::Turn => "turn",
            Kind::Bound => "bound",
            Kind::Smooth => "smooth",
        };
        let mut deepest = 0_u32;
        for depth in [8_u32, 32, 128, 512] {
            let resident = host.resident(&sizes, &modules).unwrap();
            let mut board = Board {
                resident,
                over: Groups::Fixed(grid.quads().div_ceil(width)),
                grid,
                depth,
                angle: (TURNING, SWING),
                loud: 1,
                planted: false,
            };
            let (first, last, strength, reached) = descended(&mut board, grid, PACE, ROUNDS);
            let shrank = if last > 0.0 { first / last } else { f64::MAX };
            eprintln!(
                "{named:>9} {depth:>7} {reached:>14.1} {strength:>14.1} {first:>14.1} {last:>14.1} {shrank:>8.1}x"
            );
            if last < first / 10.0 {
                deepest = depth;
            }
        }
        eprintln!("{named} still arrives at {deepest} steps");
        reach.push((named, deepest));
    }
    assert!(reach.iter().any(|(_, deepest)| *deepest >= 8));
}

#[test]
fn the_step_the_bounded_rule_wants_is_a_smaller_one() {
    let Some(host) = device("the_step_the_bounded_rule_wants_is_a_smaller_one") else {
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

    eprintln!(
        "{:>7} {:>6} {:>14} {:>14} {:>9}",
        "steps", "pace", "error first", "error last", "shrank"
    );
    let mut best = (0_u32, 0_u32, 1.0_f64);
    for depth in [8_u32, 128] {
        for pace in [0_u32, 1, 2, 3, 5] {
            let resident = host.resident(&sizes, &modules).unwrap();
            let mut board = Board {
                resident,
                over: Groups::Fixed(grid.quads().div_ceil(width)),
                grid,
                depth,
                angle: (TURNING, SWING),
                loud: 1,
                planted: false,
            };
            let (first, last, _, _) = descended(&mut board, grid, pace, ROUNDS);
            let shrank = if last > 0.0 { first / last } else { f64::MAX };
            eprintln!("{depth:>7} {pace:>6} {first:>14.1} {last:>14.1} {shrank:>8.1}x");
            if shrank > best.2 {
                best = (depth, pace, shrank);
            }
        }
    }
    eprintln!(
        "best {:.1}x at {} steps with pace {}",
        best.2, best.0, best.1
    );
    assert!(best.2 > 1.0);
}

#[test]
fn whether_the_bounded_rule_is_stalled_or_only_slow() {
    let Some(host) = device("whether_the_bounded_rule_is_stalled_or_only_slow") else {
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

    eprintln!(
        "{:>7} {:>14} {:>14} {:>9}",
        "rounds", "error first", "error last", "shrank"
    );
    let mut climbing = 0.0_f64;
    for rounds in [40_u32, 160, 640] {
        let resident = host.resident(&sizes, &modules).unwrap();
        let mut board = Board {
            resident,
            over: Groups::Fixed(grid.quads().div_ceil(width)),
            grid,
            depth: 32,
            angle: (TURNING, SWING),
            loud: 1,
            planted: false,
        };
        let (first, last, _, _) = descended(&mut board, grid, PACE, rounds);
        let shrank = if last > 0.0 { first / last } else { f64::MAX };
        eprintln!("{rounds:>7} {first:>14.1} {last:>14.1} {shrank:>8.1}x");
        climbing = climbing.max(shrank);
    }
    eprintln!("best {climbing:.1}x");
    assert!(climbing > 1.0);
}

#[test]
fn whether_the_slope_points_home() {
    let Some(host) = device("whether_the_slope_points_home") else {
        return;
    };
    let width = host.width();
    let grid = Grid::of(WIDE, DEEP);
    let sizes = [
        8,
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
        grid.cells(),
    ];

    for kind in [Kind::Turn, Kind::Bound] {
        let passes = [
            built(width, grid, Pass::Ahead, kind).unwrap(),
            built(width, grid, Pass::Back, kind).unwrap(),
        ];
        let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
        let named = if kind == Kind::Turn { "turn" } else { "bound" };
        let resident = host.resident(&sizes, &modules).unwrap();
        let mut board = Board {
            resident,
            over: Groups::Fixed(grid.quads().div_ceil(width)),
            grid,
            depth: 32,
            angle: (TURNING, SWING),
            loud: 1,
            planted: false,
        };

        let truth = spread(grid, 0x1234_5678);
        let target = board.run(&truth);
        let drift = spread(grid, 0x0BAD_C0DE);
        let guess: Vec<u32> = truth
            .iter()
            .zip(&drift)
            .map(|(one, other)| one.wrapping_add(other >> 2))
            .collect();

        let ended = board.run(&guess);
        let weight: Vec<u32> = ended
            .iter()
            .zip(&target)
            .map(|(one, other)| one.wrapping_sub(*other))
            .collect();
        let (slope, _) = board.carry(&guess, &weight);
        let points = leaning(&slope, &guess, &truth);

        let mut walk = Vec::new();
        for part in [0.0_f64, 0.25, 0.5, 0.75, 1.0] {
            let held = blended(&truth, &guess, part);
            walk.push(apart(&board.run(&held), &target));
        }
        eprintln!(
            "{named:>6} points home {points:+.4}   loss along the way {:?}",
            walk.iter()
                .map(|one| one.round() as i64)
                .collect::<Vec<_>>()
        );

        let near = walk[1];
        let far = walk[4];
        let level = (far - near) / far.max(1.0);
        assert!(points > 0.5, "{named} {points}");
        if kind == Kind::Turn {
            assert!(level > 0.5, "{named} {level}");
        } else {
            assert!(level < 0.1, "{named} {level}");
        }
    }
}

#[test]
fn only_the_last_depth_counts_and_the_shallowest_wins() {
    let Some(host) = device("only_the_last_depth_counts_and_the_shallowest_wins") else {
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
    let rounds = 30_u32;

    let ways: [(&str, Vec<u32>); 7] = [
        ("one", vec![1, 1, 1, 1]),
        ("four", vec![4, 4, 4, 4]),
        ("sixteen", vec![16, 16, 16, 16]),
        ("sixty four", vec![64, 64, 64, 64]),
        ("deep", vec![128, 128, 128, 128]),
        ("climbing", vec![16, 32, 64, 128]),
        ("falling", vec![128, 64, 32, 16]),
    ];

    eprintln!(
        "{:>9} {:>16} {:>14} {:>14} {:>9}",
        "way", "steps", "strayed", "left", "closed"
    );
    let mut told = Vec::new();
    for (named, rungs) in &ways {
        let resident = host.resident(&sizes, &modules).unwrap();
        let mut board = Board {
            resident,
            over: Groups::Fixed(grid.quads().div_ceil(width)),
            grid,
            depth: 16,
            angle: (TURNING / 64, SWING / 64),
            loud: 1,
            planted: false,
        };
        let (strayed, left) = climbed(&mut board, grid, rungs, rounds);
        let closed = if left > 0.0 { strayed / left } else { f64::MAX };
        let shown = rungs
            .iter()
            .map(|one| one.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        eprintln!("{named:>9} {shown:>16} {strayed:>14.1} {left:>14.1} {closed:>8.1}x");
        told.push((*named, closed));
    }

    let named = |want: &str| {
        told.iter()
            .find(|(held, _)| *held == want)
            .map_or(0.0, |(_, closed)| *closed)
    };
    let flat = [
        named("one"),
        named("four"),
        named("sixteen"),
        named("sixty four"),
        named("deep"),
    ];
    eprintln!(
        "one step closes {:.1}x and a hundred and twenty eight {:.1}x",
        flat[0], flat[4]
    );
    for pair in flat.windows(2) {
        assert!(pair[0] > pair[1], "{pair:?}");
    }
    assert!((named("falling") - named("sixteen")).abs() < 0.1);
    assert!(named("climbing") < named("sixteen"));
}

#[test]
fn whether_the_descent_works_where_the_loss_can_still_see() {
    let Some(host) = device("whether_the_descent_works_where_the_loss_can_still_see") else {
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

    eprintln!(
        "{:>9} {:>7} {:>14} {:>14} {:>9}",
        "turning", "steps", "error first", "error last", "shrank"
    );
    let mut told = Vec::new();
    for share in [1_u32, 8, 64] {
        for depth in [32_u32, 256] {
            let resident = host.resident(&sizes, &modules).unwrap();
            let mut board = Board {
                resident,
                over: Groups::Fixed(grid.quads().div_ceil(width)),
                grid,
                depth,
                angle: (TURNING / share, SWING / share),
                loud: 1,
                planted: false,
            };
            let (first, last, _, _) = descended(&mut board, grid, PACE, ROUNDS);
            let shrank = if last > 0.0 { first / last } else { f64::MAX };
            eprintln!(
                "{:>9} {depth:>7} {first:>14.1} {last:>14.1} {shrank:>8.1}x",
                TURNING / share
            );
            told.push((TURNING / share, depth, shrank));
        }
    }

    let weak = told
        .iter()
        .filter(|(turning, _, _)| *turning < 200)
        .map(|(_, _, shrank)| *shrank)
        .fold(0.0_f64, f64::max);
    let strong = told
        .iter()
        .filter(|(turning, _, _)| *turning > 8000)
        .map(|(_, _, shrank)| *shrank)
        .fold(0.0_f64, f64::max);
    eprintln!("weakest mixing reaches {weak:.1}x, strongest {strong:.1}x");
    assert!(weak > strong, "{weak} {strong}");
}

#[test]
fn how_rough_the_loss_is_close_up_at_each_depth() {
    let Some(host) = device("how_rough_the_loss_is_close_up_at_each_depth") else {
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

    eprintln!("{:>7} {:>10} {:>14}", "steps", "dents", "worst dent");
    let mut told = Vec::new();
    for depth in [16_u32, 64, 256, 1024] {
        board.depth = depth;
        board.tune(truth, swing);
        let target = board.run(&start);

        let mut walk = Vec::new();
        for here in truth..=(truth + 48) {
            board.tune(here, swing);
            walk.push(squared(&board.run(&start), &target));
        }

        let mut dents = 0;
        let mut worst = 0_u32;
        for at in 1..walk.len() - 1 {
            if walk[at] < walk[at - 1] && walk[at] < walk[at + 1] {
                dents += 1;
                worst = worst.max(at as u32);
            }
        }
        eprintln!("{depth:>7} {dents:>10} {worst:>14}");
        told.push((depth, dents));
    }
    board.tune(truth, swing);

    for (depth, dents) in &told {
        eprintln!("{depth:>5} steps: {dents} places to get stuck between 0 and 48 out");
    }
}

#[test]
fn how_far_back_the_loss_can_still_see() {
    let Some(host) = device("how_far_back_the_loss_can_still_see") else {
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
        depth: 1,
        angle: (TURNING, SWING),
        loud: 1,
        planted: false,
    };

    let start = spread(grid, 0x1234_5678);
    let stranger = spread(grid, 0x0BAD_C0DE);

    let depths = [1_u32, 4, 16, 64, 256, 1024];
    eprintln!(
        "{:>9} {:>10} {}",
        "turning",
        "saturated",
        depths
            .iter()
            .map(|one| format!("{one:>9}"))
            .collect::<String>()
    );

    let mut reaches = Vec::new();
    for share in [64_u32, 8, 1] {
        let base = TURNING / share;
        let mut told = String::new();
        let mut reach = 0_u32;
        let mut level = 0.0_f64;
        for depth in depths {
            board.depth = depth;
            board.tune(base, SWING / share);
            let target = board.run(&start);
            level = apart(&board.run(&stranger), &target);
            board.tune(base + 8, SWING / share);
            let loss = apart(&board.run(&start), &target);
            told.push_str(&format!("{loss:>9.0}"));
            if loss < level / 2.0 {
                reach = depth;
            }
        }
        eprintln!("{base:>9} {level:>10.0} {told}");
        reaches.push((base, reach));
    }

    for (base, reach) in &reaches {
        eprintln!("at turning {base} the loss still sees a nudge {reach} steps back");
    }
    let weakest = reaches.first().copied().unwrap_or_default();
    let strongest = reaches.last().copied().unwrap_or_default();
    assert!(weakest.1 > strongest.1, "{weakest:?} {strongest:?}");
    assert!(weakest.1 >= 1024);
}

#[test]
fn whether_the_loss_can_see_the_angle_it_was_made_with() {
    let Some(host) = device("whether_the_loss_can_see_the_angle_it_was_made_with") else {
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
        depth: 32,
        angle: (TURNING, SWING),
        loud: 1,
        planted: false,
    };

    let start = spread(grid, 0x1234_5678);
    let target = board.run(&start);

    eprintln!("{:>9} {:>9} {:>14}", "turning", "away", "loss");
    let mut seen: Vec<(i64, f64)> = Vec::new();
    for away in [
        -4096_i64, -2048, -1024, -256, -64, -8, 0, 8, 64, 256, 1024, 2048, 4096,
    ] {
        let turning = (i64::from(TURNING) + away) as u32;
        board.tune(turning, SWING);
        let loss = apart(&board.run(&start), &target);
        eprintln!("{turning:>9} {away:>9} {loss:>14.1}");
        seen.push((away, loss));
    }
    board.tune(TURNING, SWING);

    let bottom = seen
        .iter()
        .min_by(|one, other| one.1.total_cmp(&other.1))
        .copied()
        .unwrap();
    let edge = seen.last().copied().unwrap();
    eprintln!(
        "lowest at {} away, {:.1}; furthest {:.1}",
        bottom.0, bottom.1, edge.1
    );
    assert_eq!(bottom.0, 0);
}

#[test]
fn the_window_where_the_angle_can_be_asked_for() {
    let Some(host) = device("the_window_where_the_angle_can_be_asked_for") else {
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
    let stranger = spread(grid, 0x0BAD_C0DE);
    let aways = [-64_i64, -16, -4, -1, 0, 1, 4, 16, 64];

    eprintln!(
        "{:>7} {:>10} {}",
        "steps",
        "saturated",
        aways
            .iter()
            .map(|one| format!("{one:>9}"))
            .collect::<String>()
    );

    let mut told = Vec::new();
    for depth in [1_u32, 4, 16, 64, 256, 1024] {
        board.depth = depth;
        board.tune(base, swing);
        let target = board.run(&start);
        let level = apart(&board.run(&stranger), &target);

        let mut walk = Vec::new();
        let mut shown = String::new();
        for away in aways {
            board.tune((i64::from(base) + away) as u32, swing);
            let loss = apart(&board.run(&start), &target);
            shown.push_str(&format!("{loss:>9.2}"));
            walk.push(loss);
        }
        eprintln!("{depth:>7} {level:>10.0} {shown}");

        let nearest = aways
            .iter()
            .zip(&walk)
            .find(|(away, _)| **away == 1)
            .map_or(0.0, |(_, loss)| *loss);
        told.push((depth, level, nearest));
    }
    board.tune(base, swing);

    for (depth, level, nearest) in &told {
        eprintln!("{depth:>5} steps, saturated {level:.0}: one unit out is worth {nearest:.2}");
    }

    let least = told.first().map_or(0.0, |(_, _, near)| *near);
    let most = told.last().map_or(0.0, |(_, _, near)| *near);
    assert!(least > 0.0, "{least}");
    assert!(most > least * 100.0, "{least} {most}");
}
