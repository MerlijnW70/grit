#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod lifting;

use grit::Groups;
use grit::sand::Grid;
use lifting::*;

#[test]
fn a_norm_keeping_rule_descends_the_same_at_every_depth() {
    let Some(host) = device("a_norm_keeping_rule_descends_the_same_at_every_depth") else {
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
    let passes = [
        built(width, grid, Pass::Ahead, Kind::Turn).unwrap(),
        built(width, grid, Pass::Back, Kind::Turn).unwrap(),
    ];
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();

    eprintln!(
        "{:>7} {:>14} {:>14} {:>14} {:>14} {:>9}",
        "steps", "state", "slope", "error first", "error last", "shrank by"
    );
    let mut seen = Vec::new();
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
            "{depth:>7} {reached:>14.1} {strength:>14.1} {first:>14.1} {last:>14.1} {shrank:>8.1}x"
        );
        seen.push((reached, strength, first, last));
    }

    let held = seen.first().copied().unwrap_or_default();
    for now in &seen {
        for (a, b) in [
            (held.0, now.0),
            (held.1, now.1),
            (held.2, now.2),
            (held.3, now.3),
        ] {
            assert!((a - b).abs() <= a.abs() * 1e-9, "{a} {b}");
        }
    }
    assert!(held.3 < held.2 / 10.0, "{held:?}");
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
