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

#[test]
fn a_rule_can_be_rough_in_its_state_and_smooth_in_its_number() {
    let Some(host) = device("a_rule_can_be_rough_in_its_state_and_smooth_in_its_number") else {
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
        built(width, grid, Pass::Ahead, Kind::Living).unwrap(),
        built(width, grid, Pass::Back, Kind::Living).unwrap(),
    ];
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let resident = host.resident(&sizes, &modules).unwrap();

    let truth = 256_u32;
    let mut board = Board {
        resident,
        over: Groups::Fixed(grid.quads().div_ceil(width)),
        grid,
        depth: 16,
        angle: (truth, 0),
        loud: 1,
        planted: false,
    };
    let start = spread(grid, 0x1234_5678);

    let aways = [-256_i64, -64, -16, -4, -1, 0, 1, 4, 16, 64, 256];
    eprintln!(
        "{:>7} {:>10} {}",
        "steps",
        "state",
        aways
            .iter()
            .map(|one| format!("{one:>9}"))
            .collect::<String>()
    );

    let mut told = Vec::new();
    for depth in [16_u32, 64, 256] {
        board.depth = depth;
        board.tune(truth, 0);
        let target = board.run(&start);
        let reached = target
            .iter()
            .map(|one| f64::from(*one as i32).abs())
            .sum::<f64>()
            / target.len() as f64;

        let mut walk = Vec::new();
        let mut shown = String::new();
        for away in aways {
            board.tune((i64::from(truth) + away) as u32, 0);
            let loss = apart(&board.run(&start), &target);
            shown.push_str(&format!("{loss:>9.2}"));
            walk.push(loss);
        }
        eprintln!("{depth:>7} {reached:>10.1} {shown}");
        told.push((depth, reached, walk));
    }
    board.tune(truth, 0);

    for (depth, _, walk) in &told {
        let dents = (1..walk.len() - 1)
            .filter(|at| walk[*at] > 0.0 && walk[*at] < walk[at - 1] && walk[*at] < walk[at + 1])
            .count();
        assert_eq!(dents, 0, "{depth}");
    }
    let bounded = told.iter().map(|(_, one, _)| *one).collect::<Vec<_>>();
    let widest = bounded.iter().fold(0.0_f64, |a, b| a.max(*b));
    let least = bounded.iter().fold(f64::MAX, |a, b| a.min(*b));
    assert!(widest < least * 2.0, "{least} {widest}");
}

#[test]
fn the_living_rule_does_not_obey_superposition_and_the_smooth_one_does() {
    let Some(host) = device("the_living_rule_does_not_obey_superposition_and_the_smooth_one_does")
    else {
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

    let one = spread(grid, 0x1234_5678);
    let other = spread(grid, 0x0BAD_C0DE);
    let both: Vec<u32> = one
        .iter()
        .zip(&other)
        .map(|(a, b)| a.wrapping_add(*b))
        .collect();
    let nothing = vec![0_u32; grid.cells()];

    eprintln!(
        "{:>8} {:>16} {:>16} {:>9}",
        "rule", "size of the run", "what is left", "share"
    );
    let mut told = Vec::new();
    for (named, kind, knob) in [
        ("smooth", Kind::Smooth, TURNING / 64),
        ("living", Kind::Living, 256),
    ] {
        let passes = [
            built(width, grid, Pass::Ahead, kind).unwrap(),
            built(width, grid, Pass::Back, kind).unwrap(),
        ];
        let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
        let resident = host.resident(&sizes, &modules).unwrap();
        let mut board = Board {
            resident,
            over: Groups::Fixed(grid.quads().div_ceil(width)),
            grid,
            depth: 16,
            angle: (knob, SWING / 64),
            loud: 1,
            planted: false,
        };
        board.tune(knob, SWING / 64);

        for depth in [4_u32, 16, 64] {
            board.depth = depth;
            let ran_both = board.run(&both);
            let ran_one = board.run(&one);
            let ran_other = board.run(&other);
            let ran_none = board.run(&nothing);

            let mut left = 0.0_f64;
            let mut size = 0.0_f64;
            for (((all, a), b), none) in
                ran_both.iter().zip(&ran_one).zip(&ran_other).zip(&ran_none)
            {
                let held = (*all as i32)
                    .wrapping_sub(*a as i32)
                    .wrapping_sub(*b as i32)
                    .wrapping_add(*none as i32);
                left += f64::from(held).abs();
                size += f64::from(*all as i32).abs();
            }
            let share = left / size.max(1.0);
            eprintln!(
                "{named:>8} {depth:>7} {:>16.1} {:>16.1} {share:>8.4}",
                size / ran_both.len() as f64,
                left / ran_both.len() as f64
            );
            told.push((named, depth, share));
        }
    }

    let held = |want: &str, wanted: u32| {
        told.iter()
            .find(|(named, depth, _)| *named == want && *depth == wanted)
            .map_or(0.0, |(_, _, share)| *share)
    };
    for depth in [4_u32, 16, 64] {
        let smooth = held("smooth", depth);
        let living = held("living", depth);
        eprintln!(
            "{depth:>3} steps: living leaves {:.1} times what smooth does",
            living / smooth.max(f64::MIN_POSITIVE)
        );
        assert!(living > smooth * 10.0, "{depth} {smooth} {living}");
    }
}
