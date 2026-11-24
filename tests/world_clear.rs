#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;
#[path = "world_absorb/model.rs"]
mod model;
#[path = "law_parallel/pinned.rs"]
#[allow(dead_code)]
mod pinned;
#[path = "world_absorb/served.rs"]
mod served;

use common::device;

use grit::Host;
use grit::border::Box3;
use grit::branch::World;
use grit::law::Law;
use grit::serve3::Server;
use grit::wire::written as shown;

use model::{Rng, cells_of, first_difference, fitting, law_world, learned_law, soup, written};
use pinned::{bays, life};
use served::{BAYS_TABLES, LIFE_TABLES, int_at, learned, no, ok, quoted, text_at};

const KEEP: &str = r#"{"law":1,"name":"keep","params":0,"channels":1,"halo":0,"tally":{"counts":1,"weighted":false},"consts":{},"defs":{},"step":["let",[["out",["map","w","cells",["sel",["eq","w",0],0,["or","w",1]]]]],["vec","out",["vec",["count",["map","k",["range",8],["ne",["at","cells","k"],["at","out","k"]]]]]]]}"#;

struct Clear {
    from: [u32; 3],
    to: [u32; 3],
    planes: u32,
    bits: u32,
}

type Writes = Vec<Vec<([u32; 3], u32)>>;

fn modelled(size: [u32; 3], cells: &mut [u32], clear: &Clear) -> (u64, Writes) {
    let count = cells_of(size);
    let window = Box3::spanning(clear.from, clear.to, size);
    let planes = cells.len() / count;
    let mut changed = 0_u64;
    let mut writes = vec![Vec::new(); planes];
    for (plane, list) in writes.iter_mut().enumerate() {
        if (clear.planes >> plane) & 1 == 0 {
            continue;
        }
        for index in 0..count {
            let at = model::point_of(size, index);
            if !window.holds(size, at) {
                continue;
            }
            let word = &mut cells[plane * count + index];
            let kept = *word & !clear.bits;
            if kept != *word {
                *word = kept;
                changed += 1;
                list.push((at, kept));
            }
        }
    }
    (changed, writes)
}

fn checked(
    label: &str,
    cleared: &mut World<'_>,
    rewritten: &mut World<'_>,
    clears: &[Clear],
    frames: u32,
) {
    let size = cleared.size();
    for (nth, clear) in clears.iter().enumerate() {
        let mut want = cleared.cells().unwrap();
        let (changed, writes) = modelled(size, &mut want, clear);
        let done = cleared
            .clear(clear.from, clear.to, clear.planes, clear.bits)
            .unwrap();
        let got = cleared.cells().unwrap();
        if let Some(differs) = first_difference(size, &got, &want) {
            panic!(
                "{label}: clear {nth} from {:?} to {:?}: {differs}",
                clear.from, clear.to
            );
        }
        assert_eq!(
            done.words, changed,
            "{label}: clear {nth} counts the words it changed"
        );
        for (plane, list) in writes.iter().enumerate() {
            written(rewritten, u32::try_from(plane).unwrap(), list);
        }
        assert_eq!(
            rewritten.cells().unwrap(),
            got,
            "{label}: clear {nth}: the host rewrite holds other words"
        );
        println!(
            "r3 {label}: clear {nth} from {:?} to {:?} planes {:#b} bits {:#x}: {changed} words, {} lanes in {} passes",
            clear.from, clear.to, clear.planes, clear.bits, done.lanes, done.passes
        );
        for frame in 1..=frames {
            cleared.step(1).unwrap();
            rewritten.step(1).unwrap();
            let one = cleared.cells().unwrap();
            let other = rewritten.cells().unwrap();
            if let Some(differs) = first_difference(size, &one, &other) {
                panic!("{label}: clear {nth}: frame {frame} after it: {differs}");
            }
        }
    }
}

fn twins<'host>(
    host: &'host Host,
    law: &Law,
    size: [u32; 3],
    sparse: bool,
) -> (World<'host>, World<'host>) {
    (
        law_world(host, law, size, sparse, None),
        law_world(host, law, size, sparse, None),
    )
}

fn filled(
    worlds: &mut [&mut World<'_>],
    rng: &mut Rng,
    size: [u32; 3],
    spots: &[[u32; 3]],
    side: [u32; 3],
) {
    let planes = worlds[0].planes();
    for plane in 0..planes {
        let mut cells = Vec::new();
        let mut word = |rng: &mut Rng| rng.word();
        for &spot in spots {
            let from = [
                spot[0].min(size[0] - side[0]),
                spot[1].min(size[1] - side[1]),
                spot[2].min(size[2] - side[2]),
            ];
            cells.extend(soup(rng, from, side, 420, &mut word));
        }
        for world in worlds.iter_mut() {
            written(world, plane, &cells);
        }
    }
}

#[test]
fn r3_clear_zeroes_exactly_its_bits_against_a_host_model() {
    let Some(host) = device("world_clear r3 model") else {
        return;
    };
    let life_wide = learned_law(&life(), true, LIFE_TABLES);
    let bays_wide = learned_law(&bays(), true, BAYS_TABLES);
    let mut rng = Rng(0xC1EA);

    let flat = [64, 64, 2];
    let (mut one, mut other) = twins(&host, &life_wide, flat, false);
    filled(
        &mut [&mut one, &mut other],
        &mut rng,
        flat,
        &[[0, 0, 0], [40, 40, 0], [20, 50, 0]],
        [24, 20, 2],
    );
    one.step(3).unwrap();
    other.step(3).unwrap();
    checked(
        "life 2D dense, 32 bits",
        &mut one,
        &mut other,
        &[
            Clear {
                from: [50, 56, 0],
                to: [9, 7, 1],
                planes: 0b11,
                bits: 0x0000_00F0,
            },
            Clear {
                from: [10, 10, 0],
                to: [40, 30, 0],
                planes: 0b01,
                bits: 0xFFFF_0000,
            },
            Clear {
                from: [63, 0, 1],
                to: [0, 63, 1],
                planes: 0b10,
                bits: 0x8000_0001,
            },
        ],
        6,
    );

    let cube = [32, 32, 32];
    let (mut one, mut other) = twins(&host, &bays_wide, cube, false);
    filled(
        &mut [&mut one, &mut other],
        &mut rng,
        cube,
        &[[0, 0, 0], [20, 22, 18]],
        [12, 12, 12],
    );
    one.step(3).unwrap();
    other.step(3).unwrap();
    checked(
        "bays 3D dense, 32 bits",
        &mut one,
        &mut other,
        &[
            Clear {
                from: [28, 29, 30],
                to: [5, 4, 3],
                planes: 0b11,
                bits: 0x0F0F_0F0F,
            },
            Clear {
                from: [1, 2, 3],
                to: [30, 29, 28],
                planes: 0b01,
                bits: 0x0000_0400,
            },
        ],
        4,
    );

    let side = fitting(&host, &[64]);
    let sparse = [side; 3];
    let sides = law_world(&host, &bays_wide, sparse, true, None)
        .pool()
        .unwrap()
        .sides();
    let (mut one, mut other) = twins(&host, &bays_wide, sparse, true);
    filled(
        &mut [&mut one, &mut other],
        &mut rng,
        sparse,
        &[
            [0, 0, 0],
            [sides[0] - 5, sides[1] - 6, sides[2] - 3],
            [side - 10, 20, side - 9],
        ],
        [11, 11, 11],
    );
    one.step(3).unwrap();
    other.step(3).unwrap();
    checked(
        "bays 3D sparse, 32 bits",
        &mut one,
        &mut other,
        &[
            Clear {
                from: [sides[0] - 3, sides[1] - 2, sides[2] - 1],
                to: [sides[0] + 2, sides[1] + 3, sides[2] + 1],
                planes: 0b11,
                bits: 0xFFFF_FFFF,
            },
            Clear {
                from: [side - 5, 18, side - 4],
                to: [2, 25, 3],
                planes: 0b01,
                bits: 0x00FF_00FF,
            },
            Clear {
                from: [0, 0, 0],
                to: [side - 1, side - 1, side - 1],
                planes: 0b10,
                bits: 0x1000_0000,
            },
        ],
        4,
    );

    let keep = Law::compile(KEEP).unwrap();
    let (mut one, mut other) = twins(&host, &keep, sparse, true);
    let block = |at: [u32; 3], word: u32| -> Vec<([u32; 3], u32)> {
        let mut cells = Vec::new();
        for z in 0..sides[2] {
            for y in 0..sides[1] {
                for x in 0..sides[0] {
                    cells.push(([at[0] + x, at[1] + y, at[2] + z], word));
                }
            }
        }
        cells
    };
    let whole = block([sides[0], sides[1], sides[2]], 0b111);
    let split = block([2 * sides[0], 2 * sides[1], 0], 0b1101);
    for world in [&mut one, &mut other] {
        written(world, 0, &whole);
        written(world, 0, &split);
        world.step(4).unwrap();
    }
    let used = one.pool_use().unwrap().unwrap();
    assert_eq!(used.held, 0, "the solid bricks went uniform: {used:?}");
    checked(
        "keep sparse, uniform bricks",
        &mut one,
        &mut other,
        &[
            Clear {
                from: [sides[0], sides[1], sides[2]],
                to: [2 * sides[0] - 1, 2 * sides[1] - 1, 2 * sides[2] - 1],
                planes: 1,
                bits: 0b010,
            },
            Clear {
                from: [2 * sides[0] + 1, 2 * sides[1], 0],
                to: [3 * sides[0] - 2, 3 * sides[1] - 1, 1],
                planes: 1,
                bits: 0b0100,
            },
        ],
        4,
    );
    let used = one.pool_use().unwrap().unwrap();
    assert!(!used.overflow && !used.lost, "{used:?}");
    println!("r3 keep sparse: {used:?} after the clears and 8 frames");
}

fn masked(words: &[u32], keep: u32) -> Vec<u32> {
    words.iter().map(|word| word & keep).collect()
}

#[test]
fn r3_one_bit_of_32_copies_is_cleared_while_the_other_31_step_on() {
    let Some(host) = device("world_clear r3 copies") else {
        return;
    };
    let life_wide = learned_law(&life(), true, LIFE_TABLES);
    let bays_wide = learned_law(&bays(), true, BAYS_TABLES);
    let mut rng = Rng(0x32C0);
    let side = fitting(&host, &[64]);
    for (label, law, size, sparse, frames) in [
        ("life 2D dense 64x64x2", &life_wide, [64, 64, 2], false, 80),
        ("bays 3D sparse", &bays_wide, [side; 3], true, 40),
    ] {
        let mut world = law_world(&host, law, size, sparse, None);
        let spots = [
            [size[0] / 4, size[1] / 4, 0],
            [size[0] / 2, size[1] / 2, size[2] / 3],
        ];
        let edge = [
            (size[0] / 3).min(24),
            (size[1] / 3).min(24),
            size[2].min(16),
        ];
        filled(&mut [&mut world], &mut rng, size, &spots, edge);
        world.step(12).unwrap();
        let mut twin = world.fork().unwrap();
        let bit = 7;
        let top = [size[0] - 1, size[1] - 1, size[2] - 1];
        let before = world.cells().unwrap();
        let done = world.clear([0, 0, 0], top, 0b11, 1 << bit).unwrap();
        assert!(
            done.words > 0,
            "{label}: copy {bit} was empty before its clear"
        );
        let after = world.cells().unwrap();
        assert_eq!(
            masked(&after, 1 << bit),
            vec![0; after.len()],
            "{label}: copy {bit} is not empty"
        );
        assert_eq!(
            masked(&after, !(1 << bit)),
            masked(&before, !(1 << bit)),
            "{label}: the clear touched another copy"
        );
        for frame in 1..=frames {
            world.step(1).unwrap();
            twin.step(1).unwrap();
            if frame % 8 != 0 && frame != frames {
                continue;
            }
            let one = world.cells().unwrap();
            let other = twin.cells().unwrap();
            assert!(
                masked(&one, !(1 << bit)) == masked(&other, !(1 << bit)),
                "{label}: frame {frame}: the other copies differ from a world never cleared"
            );
            assert!(
                masked(&one, 1 << bit).iter().all(|word| *word == 0),
                "{label}: frame {frame}: copy {bit} came back"
            );
        }
        println!(
            "r3 {label}: copy {bit} cleared on the device ({} words), the other 31 equal to an uncleared fork for {frames} frames",
            done.words
        );
    }
}

#[test]
fn r3_the_work_of_a_clear_follows_its_box() {
    let Some(host) = device("world_clear r3 work") else {
        return;
    };
    let bays_wide = learned_law(&bays(), true, BAYS_TABLES);
    let small = fitting(&host, &[64]);
    let large = fitting(&host, &[256, 128]);
    for (label, sizes, sparse) in [
        ("dense", [[32, 32, 32], [128, 128, 128]], false),
        ("sparse", [[small; 3], [large; 3]], true),
    ] {
        let mut seen = Vec::new();
        for size in sizes {
            let mut world = law_world(&host, &bays_wide, size, sparse, None);
            let mut rng = Rng(0x5A5A);
            filled(
                &mut [&mut world],
                &mut rng,
                size,
                &[[4, 4, 4]],
                [10, 10, 10],
            );
            world.step(2).unwrap();
            let listed = u64::from(world.listed().unwrap());
            let before = world.traffic();
            let done = world.clear([2, 3, 4], [13, 12, 11], 0b11, 0xFFFF).unwrap();
            let moved = world.traffic().since(before);
            println!(
                "r3 {label} {size:?}: {} words cleared by {} lanes in {} passes; {} submissions, {} bytes up, {} bytes down",
                done.words,
                done.lanes,
                done.passes,
                moved.submits,
                moved.uploaded,
                moved.downloaded
            );
            seen.push((done, moved, listed));
        }
        let (small_done, small_moved, _) = seen[0];
        let (large_done, large_moved, _) = seen[1];
        assert_eq!(
            small_done.lanes, large_done.lanes,
            "{label}: lanes follow the box"
        );
        assert_eq!(
            small_done.passes, large_done.passes,
            "{label}: passes follow the box"
        );
        assert_eq!(
            small_done.words, large_done.words,
            "{label}: the same box holds the same words"
        );
        assert_eq!(
            small_moved.submits, large_moved.submits,
            "{label}: submissions follow the box"
        );
        assert_eq!(
            small_moved.woken, large_moved.woken,
            "{label}: wakes follow the box"
        );
        for (_, moved, listed) in &seen {
            let head = u64::from(grit::border::HEAD) * u64::from(large_done.passes);
            let bound = 4 * (head + 4 * moved.woken + 2 * listed + 64);
            assert!(
                moved.uploaded <= bound && moved.downloaded <= bound,
                "{label}: {moved:?} past {bound} bytes, with {listed} octs listed"
            );
        }
    }
}

#[test]
fn r3_bricks_a_clear_empties_are_released() {
    let Some(host) = device("world_clear r3 release") else {
        return;
    };
    let bays_one = learned_law(&bays(), false, BAYS_TABLES);
    let side = fitting(&host, &[64]);
    let size = [side; 3];
    let mut world = law_world(&host, &bays_one, size, true, None);
    let empty = world.pool_use().unwrap().unwrap().held;
    let mut rng = Rng(0xE5);
    let mut word = |_: &mut Rng| 1;
    let start = soup(&mut rng, [20, 20, 20], [14, 14, 14], 300, &mut word);
    written(&mut world, 0, &start);
    world.step(4).unwrap();
    let busy = world.pool_use().unwrap().unwrap().held;
    assert!(busy > empty, "the soup took no brick");
    let done = world
        .clear([10, 10, 10], [50, 50, 50], 0b11, u32::MAX)
        .unwrap();
    assert!(done.words > 0);
    world.step(4).unwrap();
    let held = world.pool_use().unwrap().unwrap().held;
    assert_eq!(
        held, empty,
        "the bricks the clear emptied were not released"
    );
    println!(
        "r3 sparse {side}^3: {busy} bricks held before the clear, {held} four frames after it"
    );

    let keep = Law::compile(KEEP).unwrap();
    let mut quiet = law_world(&host, &keep, size, true, None);
    let mut rng = Rng(0xE6);
    let start = soup(&mut rng, [6, 6, 6], [30, 30, 30], 200, &mut word);
    written(&mut quiet, 0, &start);
    quiet.step(6).unwrap();
    let still = quiet.pool_use().unwrap().unwrap().held;
    let listed = quiet.listed().unwrap();
    assert!(still > empty, "the quiet pattern took no brick");
    assert_eq!(listed, 0, "the pattern is not quiet");
    let done = quiet.clear([0, 0, 0], [40, 40, 40], 1, u32::MAX).unwrap();
    assert_eq!(done.words, start.len() as u64);
    quiet.step(4).unwrap();
    let held = quiet.pool_use().unwrap().unwrap().held;
    assert_eq!(
        held, empty,
        "the bricks the clear emptied in a quiet world were not released"
    );
    println!(
        "r3 sparse {side}^3 under keep, quiet with nothing listed: {still} bricks held before the clear, {held} four frames after it"
    );
}

fn line_law(server: &mut Server<'_>) -> String {
    quoted(&learned(server, &life(), true, LIFE_TABLES))
}

#[test]
fn r7_misuse_of_clear_is_refused_by_code_and_field() {
    let Some(host) = device("world_clear r7") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let law = line_law(&mut server);
    let made = ok(
        &mut server,
        "grit_world_create",
        &format!("{{\"wide\":64,\"deep\":64,\"tall\":2,\"law\":{law},\"alternate\":[0,1]}}"),
    );
    let world = int_at(&made, "world");
    let cases = [
        ("\"from\":[64,0,0],\"to\":[1,1,1]", "outside", "$.from"),
        ("\"from\":[0,0,0],\"to\":[1,1,2]", "outside", "$.to"),
        (
            "\"from\":[0,0,0],\"to\":[1,1,1],\"bits\":[3,32]",
            "bit_past",
            "$.bits[1]",
        ),
        (
            "\"from\":[0,0,0],\"to\":[1,1,1],\"bits\":[3,3]",
            "malformed",
            "$.bits[1]",
        ),
        (
            "\"from\":[0,0,0],\"to\":[1,1,1],\"planes\":[2]",
            "outside",
            "$.planes[0]",
        ),
        (
            "\"from\":[0,0,0],\"to\":[1,1,1],\"planes\":[1,1]",
            "malformed",
            "$.planes[1]",
        ),
        (
            "\"from\":[0,0,0],\"to\":[1,1,1],\"cells\":[[0,0,0,1]]",
            "write_form",
            "$.cells",
        ),
        ("\"from\":[0,0,0]", "write_form", "$.to"),
        (
            "\"from\":[0,0,0],\"to\":[1,1,1],\"planes\":[0],\"current\":true",
            "write_form",
            "$.current",
        ),
        (
            "\"from\":[0,0,0],\"to\":[1,1,1],\"plane\":1",
            "write_form",
            "$.plane",
        ),
    ];
    for (arguments, code, field) in cases {
        let got = no(
            &mut server,
            "grit_world_write",
            &format!("{{\"world\":{world},\"form\":\"clear\",{arguments}}}"),
        );
        assert_eq!(text_at(&got, "code"), code, "{arguments}: {}", shown(&got));
        assert_eq!(
            text_at(&got, "field"),
            field,
            "{arguments}: {}",
            shown(&got)
        );
    }
    let outside = no(
        &mut server,
        "grit_world_write",
        &format!("{{\"world\":{world},\"form\":\"clear\",\"from\":[64,0,0],\"to\":[1,1,1]}}"),
    );
    assert_eq!(shown(outside.get("limit").unwrap()), "[63,63,1]");
    for (arguments, field) in [
        ("\"cells\":[[0,0,0,1]],\"from\":[0,0,0]", "$.from"),
        ("\"cells\":[[0,0,0,1]],\"bits\":[1]", "$.bits"),
        ("\"cells\":[[0,0,0,1]],\"planes\":[0]", "$.planes"),
    ] {
        let got = no(
            &mut server,
            "grit_world_write",
            &format!("{{\"world\":{world},{arguments}}}"),
        );
        assert_eq!(text_at(&got, "code"), "write_form", "{arguments}");
        assert_eq!(text_at(&got, "field"), field, "{arguments}");
    }
    let missing = no(
        &mut server,
        "grit_world_write",
        &format!("{{\"world\":{world}}}"),
    );
    assert_eq!(text_at(&missing, "code"), "write_form");
    assert_eq!(text_at(&missing, "field"), "$.cells");
    let fine = ok(
        &mut server,
        "grit_world_write",
        &format!(
            "{{\"world\":{world},\"form\":\"clear\",\"from\":[60,60,0],\"to\":[3,3,1],\"bits\":[0,31]}}"
        ),
    );
    assert_eq!(shown(fine.get("bits").unwrap()), "[0,31]");
    assert_eq!(shown(fine.get("planes").unwrap()), "[0,1]");
    assert_eq!(int_at(&fine, "cleared"), 0);
}

#[cfg(feature = "faults")]
#[test]
fn r4_clears_are_journaled_through_fork_save_open_and_restart() {
    let Some(host) = device("world_clear r4") else {
        return;
    };
    let side = fitting(&host, &[64]);
    for (label, size, sparse, shell) in [
        ("dense 64x64x2", [64, 64, 2], false, ""),
        ("sparse", [side; 3], true, ""),
        ("sparse absorbing", [side; 3], true, ",\"absorb\":[1,2,3]"),
    ] {
        let got = journaled(&host, size, sparse, shell);
        println!("r4 {label}: {got}");
    }
}

#[cfg(feature = "faults")]
fn journaled(host: &Host, size: [u32; 3], sparse: bool, more: &str) -> String {
    use std::sync::Arc;

    use grit::clock::Fake;
    use served::{Scratch, every_plane, step, write};

    let scratch = Scratch::new("clear");
    let dir = scratch.dir();
    let planes = 2;
    let (world, want) = {
        let fake = Arc::new(Fake::new());
        let mut server = timed(host, &dir, &fake);
        let law = quoted(&learned(&mut server, &bays(), true, BAYS_TABLES));
        let made = ok(
            &mut server,
            "grit_world_create",
            &format!(
                "{{\"wide\":{},\"deep\":{},\"tall\":{},\"law\":{law},\"alternate\":[0,1],\"sparse\":{sparse}{more},\"budget_ms\":600000}}",
                size[0], size[1], size[2]
            ),
        );
        let world = int_at(&made, "world");
        let mut rng = Rng(0x10C);
        let mut word = |rng: &mut Rng| rng.word();
        let tall = size[2].min(12);
        let cells = soup(&mut rng, [2, 2, 0], [20, 20, tall], 400, &mut word);
        write(&mut server, world, 0, &cells);
        step(&mut server, world, 5);
        let cleared = ok(
            &mut server,
            "grit_world_write",
            &format!(
                "{{\"world\":{world},\"form\":\"clear\",\"from\":[{},6,0],\"to\":[9,{},{}],\"bits\":[1,5,30],\"budget_ms\":600000}}",
                size[0] - 2,
                size[1] - 3,
                size[2] - 1
            ),
        );
        assert!(int_at(&cleared, "cleared") > 0, "{}", shown(&cleared));
        let fork = int_at(
            &ok(
                &mut server,
                "grit_world_fork",
                &format!("{{\"world\":{world}}}"),
            ),
            "world",
        );
        ok(
            &mut server,
            "grit_world_write",
            &format!(
                "{{\"world\":{world},\"form\":\"clear\",\"from\":[4,4,0],\"to\":[12,12,{}],\"planes\":[0],\"budget_ms\":600000}}",
                size[2] - 1
            ),
        );
        ok(
            &mut server,
            "grit_world_write",
            &format!(
                "{{\"world\":{fork},\"form\":\"clear\",\"from\":[4,4,0],\"to\":[12,12,{}],\"planes\":[0],\"budget_ms\":600000}}",
                size[2] - 1
            ),
        );
        step(&mut server, world, 7);
        step(&mut server, fork, 7);
        let here = every_plane(&mut server, world, size, planes);
        assert_eq!(
            here,
            every_plane(&mut server, fork, size, planes),
            "a fork clears and steps as its source"
        );
        ok(
            &mut server,
            "grit_world_save",
            &format!("{{\"world\":{fork},\"name\":\"cleared\"}}"),
        );
        let opened = int_at(
            &ok(&mut server, "grit_world_open", "{\"name\":\"cleared\"}"),
            "world",
        );
        step(&mut server, opened, 4);
        step(&mut server, world, 4);
        let later = every_plane(&mut server, world, size, planes);
        assert_eq!(
            later,
            every_plane(&mut server, opened, size, planes),
            "an opened file steps as its source"
        );
        drop(server);
        (world, later)
    };
    let fake = Arc::new(Fake::new());
    let mut again = timed(host, &dir, &fake);
    let opened = ok(
        &mut again,
        "grit_world_open",
        &format!("{{\"name\":\"world-{world}\",\"budget_ms\":600000}}"),
    );
    let reopened = int_at(&opened, "world");
    let replayed = every_plane(&mut again, reopened, size, planes);
    assert!(
        replayed == want,
        "the world replayed from its journal reads back otherwise"
    );
    format!(
        "clears replay from the journal and survive fork, save and open; {} set words",
        served::set_total(&want)
    )
}

#[cfg(feature = "faults")]
fn timed<'host>(
    host: &'host Host,
    dir: &std::path::Path,
    fake: &std::sync::Arc<grit::clock::Fake>,
) -> Server<'host> {
    use grit::clock::Time;
    use grit::serve3::Options;

    let time: std::sync::Arc<dyn Time> = std::sync::Arc::clone(fake) as std::sync::Arc<dyn Time>;
    let options = Options {
        dir: Some(dir.to_path_buf()),
        memory: None,
    };
    Server::timed(Some(host), &options, time).unwrap()
}
