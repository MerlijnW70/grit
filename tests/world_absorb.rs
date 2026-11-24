#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;
#[path = "world_absorb/model.rs"]
mod model;
#[path = "law_parallel/pinned.rs"]
#[allow(dead_code)]
mod pinned;
#[cfg(feature = "faults")]
#[path = "world_absorb/pins.rs"]
mod pins;
#[path = "world_absorb/served.rs"]
mod served;

use std::collections::{BTreeMap, BTreeSet};

use common::device;

use grit::Host;
use grit::branch::World;
use grit::serve3::Server;
use grit::wire::{Json, written as shown};

use model::{
    Rng, first_difference, fitting, law_world, learned_law, rule_world, shell_writes, soup, written,
};
use pinned::{bays, life};
use served::{
    BAYS_TABLES, LIFE_TABLES, glider_2d, glider_3d, int_at, learned, no, ok, quoted, text_at,
};

#[cfg(feature = "faults")]
const RECORDED: [(&str, &str, &str); 4] = [
    (
        "life-dense-64x64x2",
        "b5eaaddf420215cd621e3f75e89592a7845cf1ac34feec16ec618a8a4c44ed5b",
        "d6d78d014cfcaa8fffef101f9b116afcbe01901f8b0d564f5120f836be4f1850",
    ),
    (
        "life-parallel-dense-64x64x2",
        "4d069fc0fa43ac964f70ded55415b5d15dc1ef8a27920b2e23852ae4621b2903",
        "d7a924f0edd971c3b7ee32c18da2d4d0a8face9e5d5b6cf2e89ad0de95a6898f",
    ),
    (
        "bays-sparse-64",
        "403cc1825b414ff0f7dcbf71932d7e3c6b8cfc62d5fd26572eaa4b50332c49d9",
        "0fe62bc0072dd3331892beab9039eca429b59965b176eb63eb0dc8e0d71f8043",
    ),
    (
        "bays-parallel-dense-32",
        "1454c4175daff3b4a0ce4b12f4d56214b23601973313ce5b45bbdf2359d260d3",
        "143b36487f8d6f393074cbbd2b39723cd0ae8e5c4acbe782681d1809ec257f41",
    ),
];

#[cfg(feature = "faults")]
const RECORDED_WIDTH: u32 = 32;

struct Case {
    label: &'static str,
    size: [u32; 3],
    shell: [u32; 3],
    frames: u32,
}

fn compared<'host>(
    host: &'host Host,
    make: &dyn Fn(Option<[u32; 3]>) -> World<'host>,
    case: &Case,
    start: &[Vec<([u32; 3], u32)>],
) {
    let mut absorbing = make(Some(case.shell));
    let mut zeroed = make(None);
    assert_eq!(
        absorbing.absorb().map(|held| held.cells()),
        Some(case.shell)
    );
    assert!(zeroed.absorb().is_none());
    for (plane, cells) in start.iter().enumerate() {
        written(&mut absorbing, u32::try_from(plane).unwrap(), cells);
        written(&mut zeroed, u32::try_from(plane).unwrap(), cells);
    }
    let planes = absorbing.planes() as usize;
    let mut reached = 0_u64;
    for frame in 1..=case.frames {
        absorbing.step(1).unwrap();
        zeroed.step(1).unwrap();
        let mut want = zeroed.cells().unwrap();
        let writes = shell_writes(case.size, case.shell, planes, &mut want);
        reached += writes.iter().map(Vec::len).sum::<usize>() as u64;
        let got = absorbing.cells().unwrap();
        if let Some(differs) = first_difference(case.size, &got, &want) {
            panic!("{}: frame {frame}: {differs}", case.label);
        }
        for (plane, cells) in writes.iter().enumerate() {
            written(&mut zeroed, u32::try_from(plane).unwrap(), cells);
        }
        if let Some(used) = absorbing.pool_use().unwrap() {
            assert!(
                !used.overflow && !used.lost,
                "{}: frame {frame}: {used:?}",
                case.label
            );
        }
    }
    assert!(
        reached > 0,
        "{}: nothing ever reached the shell, so the case shows nothing",
        case.label
    );
    println!(
        "r1 {} at width {}: {} frames equal to the host-zeroed world, {reached} shell words zeroed by the host",
        case.label,
        host.width(),
        case.frames
    );
}

fn soups(
    rng: &mut Rng,
    size: [u32; 3],
    planes: usize,
    wide: bool,
    spots: &[[u32; 3]],
    side: [u32; 3],
) -> Vec<Vec<([u32; 3], u32)>> {
    let mut out = vec![Vec::new(); planes];
    let mut word = |rng: &mut Rng| if wide { rng.word() } else { 1 };
    for &spot in spots {
        let from = [
            spot[0].min(size[0] - side[0]),
            spot[1].min(size[1] - side[1]),
            spot[2].min(size[2] - side[2]),
        ];
        out[0].extend(soup(rng, from, side, 380, &mut word));
    }
    out
}

#[test]
fn r1_absorb_equals_a_host_zeroed_shell_frame_for_frame() {
    let Some(host) = device("world_absorb r1") else {
        return;
    };
    let life_one = learned_law(&life(), false, LIFE_TABLES);
    let life_wide = learned_law(&life(), true, LIFE_TABLES);
    let bays_one = learned_law(&bays(), false, BAYS_TABLES);
    let bays_wide = learned_law(&bays(), true, BAYS_TABLES);
    let mut rng = Rng(0xAB50);

    let flat = [64, 64, 2];
    let edges = [[0, 0, 0], [50, 3, 0], [4, 48, 0], [44, 44, 0], [24, 24, 0]];
    for (label, law, wide, shell) in [
        (
            "life 2D dense, 1 bit, shell [1,1,0]",
            &life_one,
            false,
            [1, 1, 0],
        ),
        (
            "life 2D dense, 1 bit, shell [3,5,0]",
            &life_one,
            false,
            [3, 5, 0],
        ),
        (
            "life 2D dense, 32 bits, shell [2,1,0]",
            &life_wide,
            true,
            [2, 1, 0],
        ),
    ] {
        let start = soups(&mut rng, flat, 2, wide, &edges, [16, 16, 2]);
        let case = Case {
            label,
            size: flat,
            shell,
            frames: 160,
        };
        compared(
            &host,
            &|absorb| law_world(&host, law, flat, false, absorb),
            &case,
            &start,
        );
    }

    let cube = [32, 32, 32];
    let corners = [[0, 0, 0], [22, 20, 18], [6, 24, 2], [12, 12, 12]];
    let start = soups(&mut rng, cube, 2, true, &corners, [10, 10, 10]);
    compared(
        &host,
        &|absorb| law_world(&host, &bays_wide, cube, false, absorb),
        &Case {
            label: "bays 3D dense, 32 bits, shell [2,2,2]",
            size: cube,
            shell: [2, 2, 2],
            frames: 48,
        },
        &start,
    );

    let side = fitting(&host, &[64]);
    let sparse_cube = [side; 3];
    let near = [
        [1, 1, 1],
        [side - 14, 4, side - 13],
        [30, side - 12, 20],
        [24, 24, 24],
    ];
    for (label, law, wide, shell) in [
        (
            "bays 3D sparse, 1 bit, shell [1,1,1]",
            &bays_one,
            false,
            [1, 1, 1],
        ),
        (
            "bays 3D sparse, 32 bits, shell [3,2,1]",
            &bays_wide,
            true,
            [3, 2, 1],
        ),
    ] {
        let start = soups(&mut rng, sparse_cube, 2, wide, &near, [12, 12, 12]);
        compared(
            &host,
            &|absorb| law_world(&host, law, sparse_cube, true, absorb),
            &Case {
                label,
                size: sparse_cube,
                shell,
                frames: 64,
            },
            &start,
        );
    }

    for (label, size, sparse, shell) in [
        (
            "sand rule dense, shell [1,1,1]",
            [32, 32, 32],
            false,
            [1, 1, 1],
        ),
        (
            "sand rule sparse, shell [2,2,2]",
            sparse_cube,
            true,
            [2, 2, 2],
        ),
    ] {
        let mut sand = vec![Vec::new()];
        let mut word = |rng: &mut Rng| [1_u32, 2, 3][usize::try_from(rng.below(3)).unwrap()];
        for spot in [
            [0, 0, 0],
            [size[0] - 9, 3, size[2] - 9],
            [10, size[1] - 9, 6],
        ] {
            sand[0].extend(soup(&mut rng, spot, [8, 8, 8], 450, &mut word));
        }
        compared(
            &host,
            &|absorb| rule_world(&host, "sand", size, sparse, absorb),
            &Case {
                label,
                size,
                shell,
                frames: 40,
            },
            &sand,
        );
    }
}

fn life_step(size: [u32; 3], live: &BTreeSet<[u32; 3]>) -> BTreeSet<[u32; 3]> {
    let mut counts: BTreeMap<[u32; 3], u32> = BTreeMap::new();
    for cell in live {
        for dy in [size[1] - 1, 0, 1] {
            for dx in [size[0] - 1, 0, 1] {
                if dx == 0 && dy == 0 {
                    continue;
                }
                let near = [(cell[0] + dx) % size[0], (cell[1] + dy) % size[1], cell[2]];
                *counts.entry(near).or_default() += 1;
            }
        }
    }
    counts
        .into_iter()
        .filter(|(cell, count)| *count == 3 || (*count == 2 && live.contains(cell)))
        .map(|(cell, _)| cell)
        .collect()
}

fn live_cells(world: &mut World<'_>, size: [u32; 3]) -> BTreeSet<[u32; 3]> {
    let plane = world.current().unwrap() as usize;
    let count = model::cells_of(size);
    let cells = world.cells().unwrap();
    (0..count)
        .filter(|at| cells[plane * count + at] != 0)
        .map(|at| model::point_of(size, at))
        .collect()
}

#[test]
fn r2_ships_stop_at_the_shell_and_never_return() {
    let Some(host) = device("world_absorb r2") else {
        return;
    };
    let life_one = learned_law(&life(), false, LIFE_TABLES);
    let flat = [64, 64, 2];
    let shell = [1, 1, 0];
    let start: BTreeSet<[u32; 3]> = glider_2d([44, 44, 0], [false, false]).into_iter().collect();
    let glider: Vec<([u32; 3], u32)> = start.iter().map(|at| (*at, 1)).collect();
    let mut open = law_world(&host, &life_one, flat, false, None);
    let mut walled = law_world(&host, &life_one, flat, false, Some(shell));
    written(&mut open, 0, &glider);
    written(&mut walled, 0, &glider);
    let mut model = start.clone();
    let mut settled: Option<(u32, BTreeSet<[u32; 3]>)> = None;
    for frame in 1..=1024_u32 {
        open.step(1).unwrap();
        walled.step(1).unwrap();
        model = life_step(flat, &model)
            .into_iter()
            .filter(|at| !model::in_shell(flat, shell, *at))
            .collect();
        let got = live_cells(&mut walled, flat);
        assert!(
            got == model,
            "life glider: frame {frame}: {got:?} against {model:?}"
        );
        match &settled {
            Some((since, still)) => assert!(
                got == *still,
                "life glider: moved again at frame {frame}, still since {since}: {got:?}"
            ),
            None if frame > 8 && got == life_step(flat, &got) && got.len() < start.len() => {
                settled = Some((frame, got));
            }
            None => {}
        }
        if frame % 256 == 0 {
            assert_eq!(
                live_cells(&mut open, flat),
                start,
                "the open world brings its glider back every 256 frames"
            );
        }
    }
    let (since, still) = settled.expect("the glider never stopped at the shell");
    assert!(
        still.iter().all(|at| at[0] >= 60 && at[1] >= 60),
        "the glider stopped away from the wall it flew into: {still:?}"
    );
    println!(
        "r2 life glider in 64x64x2 with absorb {shell:?}: still from frame {since} as {} cells at the shell, {still:?}, through frame 1024, where the open world has brought it back 4 times",
        still.len()
    );

    let bays_one = learned_law(&bays(), false, BAYS_TABLES);
    let side = fitting(&host, &[256, 128]);
    let flights = [
        ("into a corner", glider_3d([70, 72, side / 2], [true, true])),
        (
            "along a face",
            served::glider_along([70, 8, side / 2], [0, 2, 1], [true, true]),
        ),
    ];
    for (label, cells) in flights {
        flown(&host, &bays_one, side, label, &cells);
    }
}

fn flown(host: &Host, law: &grit::law::Law, side: u32, label: &str, cells: &[[u32; 3]]) {
    let size = [side; 3];
    let ship: Vec<([u32; 3], u32)> = cells.iter().map(|at| (*at, 1)).collect();
    let mut world = law_world(host, law, size, true, Some([1, 1, 1]));
    let pool = world.pool().unwrap();
    let sides = pool.sides();
    let empty = world.pool_use().unwrap().unwrap().held;
    written(&mut world, 0, &ship);
    let mut most = 0;
    let mut still: Option<(u32, u32, [u32; 3], [u32; 3])> = None;
    let mut frame = 0_u32;
    let until = 12 * side;
    let top = [side - 1; 3];
    while frame < until {
        world.step(8).unwrap();
        frame += 8;
        let used = world.pool_use().unwrap().unwrap();
        assert!(
            !used.overflow && !used.lost,
            "{label}: frame {frame}: {used:?}"
        );
        most = most.max(used.held);
        let plane = world.current().unwrap();
        let seen = world.bit_tally(plane, [0, 0, 0], top, 1).unwrap()[0];
        let shape = (seen.count, seen.least, seen.most);
        match still {
            Some((since, count, least, highest)) => assert!(
                shape == (count, least, highest),
                "bays glider {label}: moved again at frame {frame}, still since {since}: {shape:?}"
            ),
            None if frame >= 256 && seen.count < 10 => {
                world.step(1).unwrap();
                let other = world.current().unwrap();
                let next = world.bit_tally(other, [0, 0, 0], top, 1).unwrap()[0];
                world.step(1).unwrap();
                frame += 2;
                if (next.count, next.least, next.most) == shape {
                    still = Some((frame, seen.count, seen.least, seen.most));
                }
            }
            None => {}
        }
    }
    let (since, count, least, highest) = still
        .unwrap_or_else(|| panic!("the Bays glider {label} never stopped in a {side}^3 world"));
    assert!(
        least.iter().filter(|at| **at <= 4).count() >= 1,
        "{label}: the ship stopped away from the shell: {least:?}"
    );
    let spanned: u32 = (0..3)
        .map(|axis| highest[axis] / sides[axis] - least[axis] / sides[axis] + 1)
        .product();
    let held = world.pool_use().unwrap().unwrap().held;
    assert!(
        most > empty + spanned,
        "{label}: the glider never took more bricks than its rest"
    );
    assert_eq!(
        held,
        empty + spanned,
        "{label}: bricks the ship held and left empty are not released"
    );
    println!(
        "r2 Bays glider {label} in sparse {side}^3 at width {} with absorb [1,1,1]: still from frame {since} as {count} cells from {least:?} to {highest:?} through frame {until} (more than 4 crossings of the world); held at most {most} bricks in flight, {held} at rest",
        host.width()
    );
}

fn server_law(server: &mut Server<'_>) -> String {
    learned(server, &life(), false, LIFE_TABLES)
}

#[test]
fn r7_misuse_is_refused_by_code_and_field() {
    let Some(host) = device("world_absorb r7") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let law = quoted(&server_law(&mut server));
    let wide = no(
        &mut server,
        "grit_world_create",
        &format!(
            "{{\"wide\":64,\"deep\":64,\"tall\":2,\"law\":{law},\"alternate\":[0,1],\"absorb\":[1,32,0]}}"
        ),
    );
    assert_eq!(text_at(&wide, "code"), "absorb_wide", "{}", shown(&wide));
    assert_eq!(text_at(&wide, "field"), "$.absorb[1]");
    assert_eq!(int_at(&wide, "limit"), 31);
    let tall = no(
        &mut server,
        "grit_world_create",
        &format!(
            "{{\"wide\":64,\"deep\":64,\"tall\":2,\"law\":{law},\"alternate\":[0,1],\"absorb\":[1,1,1]}}"
        ),
    );
    assert_eq!(text_at(&tall, "code"), "absorb_wide");
    assert_eq!(text_at(&tall, "field"), "$.absorb[2]");
    assert_eq!(int_at(&tall, "limit"), 0);
    let short = no(
        &mut server,
        "grit_world_create",
        &format!(
            "{{\"wide\":64,\"deep\":64,\"tall\":2,\"law\":{law},\"alternate\":[0,1],\"absorb\":[1,1]}}"
        ),
    );
    assert_eq!(text_at(&short, "field"), "$.absorb");
    let made = ok(
        &mut server,
        "grit_world_create",
        &format!(
            "{{\"wide\":64,\"deep\":64,\"tall\":2,\"law\":{law},\"alternate\":[0,1],\"absorb\":[31,31,0]}}"
        ),
    );
    assert_eq!(shown(made.get("absorb").unwrap()), "[31,31,0]");
    let plain = ok(
        &mut server,
        "grit_world_create",
        &format!(
            "{{\"wide\":64,\"deep\":64,\"tall\":2,\"law\":{law},\"alternate\":[0,1],\"absorb\":[0,0,0]}}"
        ),
    );
    assert!(plain.get("absorb").is_none(), "{}", shown(&plain));
    let number = int_at(&made, "world");
    let back = no(
        &mut server,
        "grit_world_back",
        &format!("{{\"world\":{number},\"frames\":1}}"),
    );
    assert_eq!(text_at(&back, "code"), "not_reversible");
    let other = int_at(&plain, "world");
    let unlike = no(
        &mut server,
        "grit_world_restore",
        &format!("{{\"world\":{other},\"from\":{number}}}"),
    );
    assert_eq!(text_at(&unlike, "code"), "unlike", "{}", shown(&unlike));
    let described = ok(&mut server, "grit_describe", "{}");
    let codes: Vec<String> = described
        .get("codes")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .map(|one| text_at(one, "code"))
        .collect();
    for code in ["absorb_wide", "write_form"] {
        assert!(codes.iter().any(|one| one == code), "{code} not described");
    }
}

#[cfg(feature = "faults")]
#[test]
fn r5_worlds_without_absorb_answer_and_journal_as_recorded_on_base() {
    let Some(host) = device("world_absorb r5") else {
        return;
    };
    let mut got = Vec::new();
    for case in &pins::CASES {
        let (answers, journal) = pins::trace(&host, case);
        println!("RECORD {} {answers} {journal}", case.name);
        got.push((case.name, answers, journal));
    }
    for (((name, answers, journal), (pinned_name, want_answers, want_journal)), case) in
        got.iter().zip(RECORDED).zip(&pins::CASES)
    {
        assert_eq!(*name, pinned_name);
        assert_eq!(answers, want_answers, "{name}: answers");
        if !case.sparse || host.width() == RECORDED_WIDTH {
            assert_eq!(journal, want_journal, "{name}: journal");
        }
    }
}

#[cfg(feature = "faults")]
#[test]
fn r4_absorb_is_journaled_through_fork_restore_save_open_and_restart() {
    let Some(host) = device("world_absorb r4") else {
        return;
    };
    for case in &pins::CASES {
        let shell = if case.size[2] == 2 {
            "[2,1,0]"
        } else {
            "[1,2,1]"
        };
        let traced = pins::absorbing(&host, case, shell);
        println!("r4 {} with absorb {shell}: {traced}", case.name);
    }
}
