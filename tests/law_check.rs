#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;
#[path = "law_check/long.rs"]
mod long;
#[path = "law_parallel/pinned.rs"]
#[allow(dead_code)]
mod pinned;
#[path = "law_check/pins.rs"]
mod pins;
#[path = "law_check/served.rs"]
mod served;

use common::device;

use grit::serve3::Server;
use grit::wire::{Json, written};

use long::{Case, Given, arguments, assert_score, compared, host_score, long_way, scientific};
use pinned::{Counted, LONG, bays, counted_step, life, moore};
use served::{
    FRAC, HEAT, Rng, count, heat_field, heat_law, heat_step, index, int_at, learned, listed, no,
    ok, quoted, text_at,
};

const RECORDED: [(&str, &str); 6] = [
    (
        "life-session",
        "f28a61b82d17b0f8a63425dc34878db3dbb86519632aa11798239188a0472e15",
    ),
    (
        "heat-session",
        "d8f687f4ac7be461a23e976400a44a12a29388ab715b26ffb7b7527aadaa5004",
    ),
    (
        "tools-list",
        "3d474353f51d6ff7b575d067e3ff645dfc2b486d4c813c391df0867929054a8a",
    ),
    (
        "describe",
        "df5fa5d85c7be3b9d7175f26e0b5db8e1aab0153dfaf531d9b56c20801a09122",
    ),
    (
        "c1-life-tiled",
        "35232fbf8231bdabe7494b4a3fdd721b81e6daa4928747747aefab2df91c9529",
    ),
    (
        "cells-targets",
        "c6f5f7425fa51ef8aba41a7c0c07cdd5b4eeb42b07b71e43a406c47f2fa4d40c",
    ),
];

fn soup(rng: &mut Rng, size: [u32; 3], permille: u64, wide: bool) -> Vec<u32> {
    (0..count(size))
        .map(|_| {
            if rng.below(1000) >= permille {
                0
            } else if wide {
                rng.word()
            } else {
                1
            }
        })
        .collect()
}

fn layer(rng: &mut Rng, size: [u32; 3], permille: u64) -> Vec<u32> {
    let mut words = vec![0; count(size)];
    for y in 0..size[1] {
        for x in 0..size[0] {
            words[index(size, [x, y, 0])] = u32::from(rng.below(1000) < permille);
        }
    }
    words
}

fn case(label: &str, law: &str, size: [u32; 3], steps: u32) -> Case {
    Case {
        label: label.to_owned(),
        law: law.to_owned(),
        alternate: true,
        params: Vec::new(),
        size,
        sparse: false,
        absorb: None,
        zero: false,
        steps,
        planes: 2,
        sides: None,
        frac: None,
    }
}

fn run_of(answer: &Json) -> (String, i64) {
    let run = answer.get("run").unwrap();
    (text_at(run, "layout"), int_at(run, "worlds"))
}

fn x1_rule(born: &[u32], kept: &[u32], cube: bool) -> Counted {
    Counted {
        cube,
        outer: true,
        places: moore(cube),
        born: born.iter().map(|count| 1 << count).sum(),
        kept: kept.iter().map(|count| 1 << count).sum(),
    }
}

#[test]
fn c1_every_start_equals_its_own_world_the_long_way() {
    let Some(host) = device("law_check c1") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let life_law = learned(&mut server, &life(), false);
    let life_wide = learned(&mut server, &life(), true);
    let bays_law = learned(&mut server, &bays(), false);
    let bays_wide = learned(&mut server, &bays(), true);
    let heat = heat_law(&mut server);
    let flat = [32, 32, 2];
    let cube = [16, 16, 16];
    let mut rng = Rng(11);
    let mut cases: Vec<(Case, Vec<Vec<u32>>, &str)> = Vec::new();
    let mut add = |made: Case, many: usize, permille: u64, wide: bool, layout: &'static str| {
        let starts = (0..many)
            .map(|_| match made.frac {
                Some(frac) => heat_field(&mut rng, made.size, frac),
                None => soup(&mut rng, made.size, permille, wide),
            })
            .collect();
        cases.push((made, starts, layout));
    };
    add(
        case("life 2d wrapped", &life_law, flat, 6),
        5,
        350,
        false,
        "tiled",
    );
    add(
        Case {
            zero: true,
            ..case("life 2d zero", &life_law, flat, 5)
        },
        3,
        350,
        false,
        "apart",
    );
    add(
        Case {
            absorb: Some([1, 2, 0]),
            ..case("life 2d absorb", &life_law, flat, 5)
        },
        3,
        350,
        false,
        "apart",
    );
    add(
        case("life 2d 32 bits", &life_wide, flat, 5),
        3,
        900,
        true,
        "tiled",
    );
    add(
        Case {
            alternate: false,
            sides: Some((0, 1)),
            ..case("life 2d reads 0 writes 1", &life_law, flat, 3)
        },
        3,
        350,
        false,
        "tiled",
    );
    add(
        case("bays 3d wrapped", &bays_law, cube, 4),
        3,
        250,
        false,
        "tiled",
    );
    add(
        Case {
            zero: true,
            ..case("bays 3d 32 bits zero", &bays_wide, cube, 3)
        },
        2,
        900,
        true,
        "apart",
    );
    add(
        Case {
            sparse: true,
            ..case("bays 3d sparse wrapped", &bays_law, [32, 32, 32], 4)
        },
        2,
        200,
        false,
        "apart",
    );
    add(
        Case {
            sparse: true,
            zero: true,
            absorb: Some([1, 1, 1]),
            ..case("bays 3d sparse zero absorb", &bays_law, cube, 3)
        },
        2,
        250,
        false,
        "apart",
    );
    add(
        Case {
            frac: Some(FRAC),
            ..case("heat 2d real wrapped", &heat, flat, 7)
        },
        3,
        0,
        false,
        "tiled",
    );
    add(
        Case {
            frac: Some(FRAC),
            zero: true,
            ..case("heat 2d real zero", &heat, flat, 4)
        },
        2,
        0,
        false,
        "apart",
    );
    for (at, (made, starts, layout)) in cases.iter().enumerate() {
        let checked = compared(&mut server, made, starts, 100 + at as u64);
        let (got, worlds) = run_of(&checked.answer);
        assert_eq!(got, *layout, "{}", made.label);
        if *layout == "tiled" {
            assert_eq!(worlds, 1, "{}", made.label);
        } else {
            assert_eq!(worlds, starts.len() as i64, "{}", made.label);
        }
        println!(
            "C1 {}: {} starts, {got} in {worlds} worlds, equal to the long way",
            made.label,
            starts.len()
        );
    }
}

#[test]
fn c1_tiled_starts_do_not_reach_each_other_across_the_seams() {
    let Some(host) = device("law_check c1 seams") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let law = learned(&mut server, &life(), false);
    let size = [16, 16, 2];
    let made = case("gliders on the seams", &law, size, 24);
    let mut starts = Vec::new();
    for at in 0..4_u32 {
        let mut words = vec![0; count(size)];
        let glider = [[1_u32, 0], [2, 1], [0, 2], [1, 2], [2, 2]];
        for [a, b] in glider {
            let x = (14 + a + at) % 16;
            let y = (15 + b) % 16;
            words[index(size, [x, y, 0])] = 1;
        }
        for x in 0..16 {
            if x % 4 == at % 4 {
                words[index(size, [x, 7, 0])] = 1;
                words[index(size, [x, 8, 0])] = 1;
            }
        }
        starts.push(words);
    }
    let checked = compared(&mut server, &made, &starts, 7);
    let run = checked.answer.get("run").unwrap();
    assert_eq!(text_at(run, "layout"), "tiled");
    assert_eq!(int_at(run, "margin"), 48);
    let fingerprints: Vec<String> = checked
        .answer
        .get("starts")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .map(|one| text_at(one, "fingerprint"))
        .collect();
    for (got, long) in fingerprints.iter().zip(&checked.long) {
        assert_eq!(got, &long.fingerprint);
    }
}

#[test]
fn c1_boxes_of_open_worlds_start_and_aim_as_their_words_do() {
    let Some(host) = device("law_check c1 boxes") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let law = learned(&mut server, &life(), false);
    let made = ok(
        &mut server,
        "grit_world_create",
        &format!(
            "{{\"wide\":64,\"deep\":64,\"tall\":2,\"law\":{},\"params\":[],\"alternate\":[0,1],\"budget_ms\":{LONG}}}",
            quoted(&law)
        ),
    );
    let world = int_at(&made, "world");
    let mut rng = Rng(3);
    let big = [64, 64, 2];
    let words = soup(&mut rng, big, 300, false);
    served::write(&mut server, world, 0, &served::set_of(big, &words));
    let size = [32, 32, 2];
    let corners = [[0_u32, 0, 0], [32, 0, 0], [16, 32, 0]];
    let boxed: Vec<Vec<u32>> = corners
        .iter()
        .map(|corner| served::box_words(&mut server, world, 0, *corner, size))
        .collect();
    let item = |corner: [u32; 3]| {
        format!(
            "{{\"world\":{world},\"from\":{},\"targets\":[{{\"frame\":0,\"world\":{world},\"from\":{}}}]}}",
            listed(&corner),
            listed(&corner)
        )
    };
    let from_worlds: Vec<String> = corners.iter().map(|corner| item(*corner)).collect();
    let by_world = ok(
        &mut server,
        "grit_law_check",
        &format!(
            "{{\"law\":{},\"alternate\":[0,1],\"size\":[32,32,2],\"frames\":5,\"starts\":[{}],\"budget_ms\":{LONG}}}",
            quoted(&law),
            from_worlds.join(",")
        ),
    );
    let given: Vec<Given> = boxed
        .iter()
        .map(|start| Given {
            start: start.clone(),
            targets: vec![(0, start.clone())],
        })
        .collect();
    let by_words = ok(
        &mut server,
        "grit_law_check",
        &arguments(&case("boxes", &law, size, 5), &given, ""),
    );
    assert_eq!(
        written(by_world.get("starts").unwrap()),
        written(by_words.get("starts").unwrap())
    );
    for shown in by_world.get("starts").and_then(Json::list).unwrap() {
        let target = &shown.get("targets").and_then(Json::list).unwrap()[0];
        assert_eq!(int_at(target, "wrong"), 0);
    }
}

#[test]
fn c2_x1_checks_are_one_call_each() {
    let Some(host) = device("law_check c2") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let mut rng = Rng(2026);
    let p1 = x1_rule(&[3, 6, 8], &[2, 4, 5], false);
    let p1_law = learned(&mut server, &p1, false);
    let size = [64, 64, 2];
    let plane = [64, 64, 1];
    let host_run = |rule: &Counted, cells: [u32; 3], words: &[u32], steps: u32| {
        let mut out = words.to_vec();
        for _ in 0..steps {
            out = counted_step(rule, cells, &out, true);
        }
        out
    };
    let lift = |words: &[u32]| {
        let mut out = words.to_vec();
        out.resize(count(size), 0);
        out
    };
    let held: Vec<Vec<u32>> = (0..4).map(|_| layer(&mut rng, size, 90)).collect();
    let given: Vec<Given> = held
        .iter()
        .map(|start| Given {
            start: start.clone(),
            targets: vec![(8, lift(&host_run(&p1, plane, &start[..count(plane)], 8)))],
        })
        .collect();
    let made = case("x1 p1 held", &p1_law, size, 8);
    let answer = ok(&mut server, "grit_law_check", &arguments(&made, &given, ""));
    let mut long_calls = 0;
    for (shown, start) in answer
        .get("starts")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .zip(&held)
    {
        let long = long_way(&mut server, &made, start, &[]);
        long_calls += long.calls;
        assert_eq!(text_at(shown, "fingerprint"), long.fingerprint);
        let target = &shown.get("targets").and_then(Json::list).unwrap()[0];
        assert_eq!(int_at(target, "wrong"), 0, "{}", written(target));
    }
    assert_eq!(run_of(&answer), ("tiled".to_owned(), 1));
    println!("C2 p1 held: 4 starts of 64x64, 8 frames: 1 call, the long way {long_calls} calls");
    let mut pairs = Vec::new();
    let mut flips = Vec::new();
    for _ in 0..12 {
        let start = layer(&mut rng, size, 300);
        let mut after = lift(&host_run(&p1, plane, &start[..count(plane)], 1));
        let mut flipped = 0;
        for word in after.iter_mut().take(count(plane)) {
            if rng.below(100) == 0 {
                *word ^= 1;
                flipped += 1;
            }
        }
        flips.push(flipped);
        pairs.push(Given {
            start,
            targets: vec![(1, after)],
        });
    }
    let training = case("x1 p1 training", &p1_law, size, 1);
    let answer = ok(
        &mut server,
        "grit_law_check",
        &arguments(&training, &pairs, ""),
    );
    for (shown, wanted) in answer
        .get("starts")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .zip(&flips)
    {
        let target = &shown.get("targets").and_then(Json::list).unwrap()[0];
        assert_eq!(int_at(target, "wrong"), *wanted);
    }
    let totals = answer.get("totals").unwrap();
    assert_eq!(int_at(totals, "targets"), 12);
    assert_eq!(int_at(totals, "wrong"), flips.iter().sum::<i64>());
    println!(
        "C2 p1 training: 12 pairs, 1 call, {} in {} worlds",
        run_of(&answer).0,
        run_of(&answer).1
    );
    let p2 = x1_rule(&[5, 6], &[4, 5, 6], true);
    let p2_law = learned(&mut server, &p2, false);
    let cube = [32, 32, 32];
    let starts: Vec<Vec<u32>> = (0..4).map(|_| soup(&mut rng, cube, 150, false)).collect();
    let given: Vec<Given> = starts
        .iter()
        .map(|start| Given {
            start: start.clone(),
            targets: vec![(8, host_run(&p2, cube, start, 8))],
        })
        .collect();
    let made = case("x1 p2 held", &p2_law, cube, 8);
    let answer = ok(&mut server, "grit_law_check", &arguments(&made, &given, ""));
    assert_eq!(run_of(&answer), ("tiled".to_owned(), 1));
    let mut long_calls = 0;
    for (shown, start) in answer
        .get("starts")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .zip(&starts)
    {
        let long = long_way(&mut server, &made, start, &[]);
        long_calls += long.calls;
        assert_eq!(text_at(shown, "fingerprint"), long.fingerprint);
        let target = &shown.get("targets").and_then(Json::list).unwrap()[0];
        assert_eq!(int_at(target, "wrong"), 0, "{}", written(target));
    }
    println!("C2 p2 held: 4 starts of 32^3, 8 frames: 1 call, the long way {long_calls} calls");
    let heat = heat_law(&mut server);
    let made = Case {
        frac: Some(FRAC),
        ..case("x1 p3 held", &heat, size, 10)
    };
    let starts: Vec<Vec<u32>> = (0..4).map(|_| heat_field(&mut rng, size, FRAC)).collect();
    let given: Vec<Given> = starts
        .iter()
        .map(|start| {
            let mut words = start.clone();
            for _ in 0..10 {
                words = heat_step(size, &words, &HEAT, FRAC);
            }
            Given {
                start: start.clone(),
                targets: vec![(10, words)],
            }
        })
        .collect();
    let answer = ok(&mut server, "grit_law_check", &arguments(&made, &given, ""));
    assert_eq!(run_of(&answer), ("tiled".to_owned(), 1));
    let totals = answer.get("totals").unwrap();
    println!(
        "C2 p3 held: 4 starts of 64x64, 10 frames, 1 call: rms_real {} max_real {}",
        text_at(totals, "rms_real"),
        text_at(totals, "max_real")
    );
}

fn refused_with(server: &mut Server<'_>, arguments: &str, code: &str, field: &str) -> Json {
    let got = no(server, "grit_law_check", arguments);
    assert_eq!(
        (text_at(&got, "code"), text_at(&got, "field")),
        (code.to_owned(), field.to_owned()),
        "{}",
        written(&got)
    );
    assert!(got.get("starts").is_none());
    got
}

#[test]
fn c3_requests_past_a_limit_are_refused_by_name_with_the_limit() {
    let Some(host) = device("law_check c3") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let law = quoted(&learned(&mut server, &life(), false));
    let whole = quoted(include_str!("law/heat.law.json"));
    let empty = "{\"cells\":[]}";
    let many = |count: usize| vec![empty; count].join(",");
    let ask = |size: &str, frames: u32, starts: &str, more: &str| {
        format!(
            "{{\"law\":{law},\"alternate\":[0,1],\"size\":{size},\"frames\":{frames},\"starts\":[{starts}]{more}}}"
        )
    };
    let got = refused_with(
        &mut server,
        &ask("[4,4,2]", 1, &many(257), ""),
        "too_many",
        "$.starts",
    );
    assert_eq!(int_at(&got, "limit"), 256);
    let fits = ok(
        &mut server,
        "grit_law_check",
        &ask("[4,4,2]", 1, &many(256), ""),
    );
    assert_eq!(fits.get("starts").and_then(Json::list).unwrap().len(), 256);
    let got = refused_with(
        &mut server,
        &ask("[128,128,128]", 1, &many(3), ""),
        "too_many",
        "$.starts",
    );
    assert_eq!(int_at(&got, "limit"), 2);
    let got = refused_with(
        &mut server,
        &ask("[4,4,2]", 4097, empty, ""),
        "frames",
        "$.frames",
    );
    assert_eq!(int_at(&got, "limit"), 4096);
    let got = refused_with(
        &mut server,
        &ask("[128,128,128]", 2000, &many(2), ""),
        "frames",
        "$.frames",
    );
    assert_eq!(int_at(&got, "limit"), 1024);
    let got = refused_with(
        &mut server,
        &ask("[60,64,2]", 1, empty, ""),
        "check_start",
        "$.size",
    );
    assert_eq!(written(got.get("limit").unwrap()), "[64,64,2]");
    refused_with(
        &mut server,
        &ask("[4,4,2]", 1, "", ""),
        "check_start",
        "$.starts",
    );
    refused_with(
        &mut server,
        &ask("[4,4,2]", 1, "{\"cells\":[],\"dense\":[]}", ""),
        "check_start",
        "$.starts[0]",
    );
    let got = refused_with(
        &mut server,
        &ask("[4,4,2]", 1, "{\"cells\":[[1,1,0,1],[4,0,0,1]]}", ""),
        "check_start",
        "$.starts[0].cells[1]",
    );
    assert_eq!(written(got.get("limit").unwrap()), "[3,3,1]");
    let got = refused_with(
        &mut server,
        &ask("[4,4,2]", 1, "{\"dense\":[1,2,3]}", ""),
        "check_start",
        "$.starts[0].dense",
    );
    assert_eq!(int_at(&got, "limit"), 32);
    let got = refused_with(
        &mut server,
        &ask(
            "[4,4,2]",
            3,
            "{\"cells\":[],\"targets\":[{\"cells\":[]},{\"frame\":4,\"cells\":[]}]}",
            "",
        ),
        "check_target",
        "$.starts[0].targets[1].frame",
    );
    assert_eq!(int_at(&got, "limit"), 3);
    let got = refused_with(
        &mut server,
        &ask(
            "[4,4,2]",
            3,
            "{\"cells\":[]},{\"cells\":[],\"targets\":[{\"dense\":[0]}]}",
            "",
        ),
        "check_target",
        "$.starts[1].targets[0].dense",
    );
    assert_eq!(int_at(&got, "limit"), 32);
    refused_with(
        &mut server,
        &ask(
            "[4,4,2]",
            3,
            "{\"cells\":[],\"targets\":[{\"cells\":[[0,0,2,1]]}]}",
            "",
        ),
        "check_target",
        "$.starts[0].targets[0].cells[0]",
    );
    refused_with(
        &mut server,
        &ask("[4,4,2]", 3, "{\"cells\":[],\"targets\":[{}]}", ""),
        "check_target",
        "$.starts[0].targets[0]",
    );
    let targets = vec!["{\"cells\":[]}"; 1025].join(",");
    let got = refused_with(
        &mut server,
        &ask(
            "[4,4,2]",
            1,
            &format!("{{\"cells\":[],\"targets\":[{targets}]}}"),
            "",
        ),
        "too_many",
        "$.starts[0].targets",
    );
    assert_eq!(int_at(&got, "limit"), 1024);
    let got = refused_with(
        &mut server,
        &format!(
            "{{\"law\":{law},\"size\":[4,4,2],\"frames\":1,\"writes\":2,\"starts\":[{empty}]}}"
        ),
        "outside",
        "$.writes",
    );
    assert_eq!(int_at(&got, "limit"), 1);
    refused_with(
        &mut server,
        &format!(
            "{{\"law\":{law},\"size\":[4,4,2],\"frames\":1,\"reads\":2,\"starts\":[{empty}]}}"
        ),
        "outside",
        "$.reads",
    );
    refused_with(
        &mut server,
        &format!(
            "{{\"law\":{law},\"size\":[4,4,2],\"frames\":1,\"current\":true,\"starts\":[{empty}]}}"
        ),
        "not_iterating",
        "$.current",
    );
    refused_with(
        &mut server,
        &ask("[4,4,2]", 1, empty, ",\"current\":true,\"reads\":0"),
        "plane_or_current",
        "$.reads",
    );
    refused_with(
        &mut server,
        &ask("[4,4,2]", 1, empty, ",\"params\":[1]"),
        "params",
        "$.params",
    );
    refused_with(
        &mut server,
        &format!(
            "{{\"law\":{whole},\"params\":[300,7,900],\"size\":[16,16,16],\"sparse\":true,\"frames\":1,\"starts\":[{empty}]}}"
        ),
        "quiet_refused",
        "$.sparse",
    );
    refused_with(
        &mut server,
        &ask("[4,4,2]", 1, empty, ",\"absorb\":[2,0,0]"),
        "absorb_wide",
        "$.absorb[0]",
    );
    refused_with(
        &mut server,
        &ask("[4,4,2]", 1, empty, ",\"frac\":8"),
        "malformed",
        "$.frac",
    );
    for more in [",\"page\":0", ",\"return\":false,\"page\":1"] {
        refused_with(
            &mut server,
            &ask("[4,4,2]", 1, empty, more),
            "malformed",
            "$.page",
        );
    }
    refused_with(
        &mut server,
        &ask("[4,4,2]", 1, "{\"world\":999,\"from\":[0,0,0]}", ""),
        "no_world",
        "$.starts[0].world",
    );
    let got = refused_with(
        &mut server,
        &ask("[4,4,2]", 1, empty, ",\"return\":true,\"page\":1"),
        "outside",
        "$.page",
    );
    assert_eq!(int_at(&got, "limit"), 0);
    let described = ok(&mut server, "grit_describe", "{}");
    let codes: Vec<String> = described
        .get("codes")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .map(|one| text_at(one, "code"))
        .collect();
    for code in pins::NEW_CODES {
        assert!(codes.iter().any(|one| one == code), "{code} not described");
    }
}

#[test]
fn c4_base_answers_are_the_bytes_recorded_on_base() {
    let Some(host) = device("law_check c4") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let got = [
        ("life-session", pins::life_session(&mut server)),
        ("heat-session", pins::heat_session(&mut server)),
        ("tools-list", pins::listing(&mut Server::new(None))),
        ("describe", pins::described(&mut Server::new(None))),
        ("c1-life-tiled", pins::c1_case(&mut server)),
        ("cells-targets", pins::cells_targets(&mut server)),
    ];
    for (name, hash) in &got {
        println!("RECORD {name} {hash}");
    }
    for ((name, hash), (pinned, wanted)) in got.iter().zip(RECORDED) {
        assert_eq!(*name, pinned);
        assert_eq!(hash, wanted, "{name}");
    }
}

#[test]
fn c5_scores_equal_scores_counted_on_the_host() {
    let Some(host) = device("law_check c5") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let law = learned(&mut server, &life(), false);
    let size = [32, 32, 2];
    let plane = [32, 32, 1];
    let mut rng = Rng(5);
    let made = case("c5 life", &law, size, 6);
    let start = layer(&mut rng, size, 350);
    let mut states = Vec::new();
    let mut words = start[..count(plane)].to_vec();
    for at in 0..=6 {
        let mut whole = words.clone();
        whole.resize(count(size), 0);
        states.push(whole);
        if at < 6 {
            words = counted_step(&life(), plane, &words, true);
        }
    }
    let mut last = states[6].clone();
    let end = count(size) - 1;
    last[end] ^= 1;
    let corner = index(size, [31, 31, 0]);
    last[corner] ^= 1;
    let mut middle = states[3].clone();
    let spot = index(size, [5, 9, 0]);
    middle[spot] ^= 1;
    let later = index(size, [7, 20, 0]);
    middle[later] ^= 1;
    let given = [Given {
        start: start.clone(),
        targets: vec![
            (6, last.clone()),
            (3, middle.clone()),
            (6, states[6].clone()),
        ],
    }];
    let answer = ok(&mut server, "grit_law_check", &arguments(&made, &given, ""));
    let shown = &answer.get("starts").and_then(Json::list).unwrap()[0];
    let targets = shown.get("targets").and_then(Json::list).unwrap();
    assert_eq!(int_at(&targets[0], "frame"), 6);
    assert_eq!(int_at(&targets[0], "wrong"), 2);
    assert_eq!(int_at(&targets[0], "right"), count(size) as i64 - 2);
    assert_eq!(
        written(targets[0].get("first_wrong").unwrap()),
        format!(
            "{{\"at\":[31,31,0],\"want\":{},\"got\":{}}}",
            last[corner], states[6][corner]
        )
    );
    assert_score(
        "c5 last",
        &targets[0],
        &host_score(size, &states[6], &last, None),
    );
    assert_eq!(int_at(&targets[1], "wrong"), 2);
    assert_eq!(
        written(targets[1].get("first_wrong").unwrap()),
        format!(
            "{{\"at\":[5,9,0],\"want\":{},\"got\":{}}}",
            middle[spot], states[3][spot]
        )
    );
    assert_eq!(int_at(&targets[2], "wrong"), 0);
    assert!(targets[2].get("first_wrong").is_none());
    let totals = answer.get("totals").unwrap();
    assert_eq!(int_at(totals, "wrong"), 4);
    assert_eq!(int_at(totals, "targets"), 3);
    let heat = heat_law(&mut server);
    let real = Case {
        frac: Some(FRAC),
        ..case("c5 heat", &heat, size, 5)
    };
    let start = heat_field(&mut rng, size, FRAC);
    let long = long_way(&mut server, &real, &start, &[]);
    let mut target = long.end.clone();
    let moved = [(3_usize, 700_i32), (100, -1300), (end, 2500)];
    for (at, by) in moved {
        target[at] = target[at].wrapping_add_signed(by);
    }
    let given = [Given {
        start,
        targets: vec![(5, target.clone())],
    }];
    let answer = ok(&mut server, "grit_law_check", &arguments(&real, &given, ""));
    let shown = &answer.get("starts").and_then(Json::list).unwrap()[0];
    let score = &shown.get("targets").and_then(Json::list).unwrap()[0];
    let squares: u128 = moved
        .iter()
        .map(|(_, by)| u128::from(by.unsigned_abs()).pow(2))
        .sum();
    let cells = count(size) as f64;
    let rms = (squares as f64 / cells).sqrt();
    assert_eq!(int_at(score, "max"), 2500);
    assert_eq!(text_at(score, "sum_squares"), squares.to_string());
    assert_eq!(text_at(score, "rms"), scientific(rms));
    assert_eq!(text_at(score, "max_real"), scientific(2500.0 / 65536.0));
    assert_eq!(text_at(score, "rms_real"), scientific(rms / 65536.0));
    assert_eq!(
        written(score.get("first_wrong").unwrap()),
        format!(
            "{{\"at\":[3,0,0],\"want\":{},\"got\":{}}}",
            target[3], long.end[3]
        )
    );
    assert_score(
        "c5 heat",
        score,
        &host_score(size, &long.end, &target, Some(FRAC)),
    );
    assert!(!text_at(&answer, "format").is_empty());
    assert_eq!(int_at(&answer, "frac"), i64::from(FRAC));
}

const SAND: &str = include_str!("law/sand.law.json");

const MIX: &str = include_str!("law/mix.law.json");

const EMBED: &str = include_str!("law/embed.law.json");

const BLUR: &str = include_str!("halo3/blur2.law.json");

fn in_place(label: &str, law: &str, params: Vec<u32>, size: [u32; 3], steps: u32) -> Case {
    Case {
        alternate: false,
        params,
        planes: 1,
        sides: Some((0, 0)),
        ..case(label, law, size, steps)
    }
}

fn filled(rng: &mut Rng, size: [u32; 3], permille: u64, cell: fn(&mut Rng) -> u32) -> Vec<u32> {
    (0..count(size))
        .map(|_| {
            if rng.below(1000) < permille {
                cell(rng)
            } else {
                0
            }
        })
        .collect()
}

type Planted = (Case, u64, fn(&mut Rng) -> u32, u32);

fn sand_cell(rng: &mut Rng) -> u32 {
    [grit::sand::SAND, grit::sand::WATER, grit::sand::WALL][rng.below(3) as usize]
}

fn embed_cell(rng: &mut Rng) -> u32 {
    let roll = rng.next();
    match roll % 4 {
        0 => grit::rule3::embed::embedding([(roll >> 8) as i8, (roll >> 16) as i8, 0, 0]),
        1 => ((roll >> 24) % 5) as u32,
        _ => (roll >> 16) as u32,
    }
}

fn byte_cell(rng: &mut Rng) -> u32 {
    rng.below(256) as u32
}

fn word_cell(rng: &mut Rng) -> u32 {
    rng.word()
}

#[test]
fn c1_tiled_laws_that_read_frame_step_octs_or_look_equal_their_own_worlds() {
    let Some(host) = device("law_check c1 laws") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let cube = [16, 16, 16];
    let mut rng = Rng(41);
    let embed = vec![grit::rule3::embed::embedding([1, -1, 2, -2])];
    for steps in [1_u32, 2, 5, 9] {
        let made: [Planted; 4] = [
            (
                in_place("sand", SAND, Vec::new(), cube, steps),
                450,
                sand_cell,
                0,
            ),
            (
                in_place("mix", MIX, vec![3, 1 << 20], cube, steps),
                700,
                word_cell,
                0,
            ),
            (
                in_place("embed", EMBED, embed.clone(), cube, steps),
                500,
                embed_cell,
                0,
            ),
            (
                Case {
                    planes: 3,
                    sides: Some((steps % 2, 2)),
                    ..in_place("blur2", BLUR, Vec::new(), cube, steps)
                },
                600,
                byte_cell,
                1,
            ),
        ];
        for (at, (made, permille, cell, halo)) in made.into_iter().enumerate() {
            let starts: Vec<Vec<u32>> = (0..4)
                .map(|_| filled(&mut rng, made.size, permille, cell))
                .collect();
            let seed = 300 + u64::from(steps) * 10 + at as u64;
            let checked = compared(&mut server, &made, &starts, seed);
            let run = checked.answer.get("run").unwrap();
            let label = format!("{} k={steps}", made.label);
            assert_eq!(text_at(run, "layout"), "tiled", "{label}");
            assert_eq!(int_at(run, "worlds"), 1, "{label}");
            assert_eq!(
                int_at(run, "margin"),
                i64::from(((1 + halo) * steps).next_multiple_of(2)),
                "{label}"
            );
            println!("C1 {label}: 4 tiled starts equal to the long way");
        }
    }
}

#[test]
fn c1_a_zero_border_start_steps_as_life_with_a_dead_outside() {
    let Some(host) = device("law_check c1 zero") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let law = learned(&mut server, &life(), false);
    let mut rng = Rng(17);
    for (size, steps) in [
        ([32, 32, 2], 1_u32),
        ([32, 32, 2], 2),
        ([32, 32, 2], 5),
        ([64, 16, 2], 9),
    ] {
        let plane = [size[0], size[1], 1];
        let start = layer(&mut rng, size, 400);
        let mut dead = start[..count(plane)].to_vec();
        let mut torus = dead.clone();
        let mut states = Vec::new();
        for at in 0..=steps {
            let mut whole = dead.clone();
            whole.resize(count(size), 0);
            states.push((at, whole));
            if at < steps {
                dead = counted_step(&life(), plane, &dead, false);
                torus = counted_step(&life(), plane, &torus, true);
            }
        }
        let label = format!("life zero {size:?} k={steps}");
        let end = states.last().unwrap().1.clone();
        assert_ne!(
            torus,
            end[..count(plane)],
            "{label}: the border must matter"
        );
        let made = Case {
            zero: true,
            ..case(&label, &law, size, steps)
        };
        let given = [Given {
            start,
            targets: states.clone(),
        }];
        let asked = arguments(&made, &given, ",\"return\":true");
        let answer = ok(&mut server, "grit_law_check", &asked);
        assert_eq!(run_of(&answer), ("apart".to_owned(), 1), "{label}");
        let shown = &answer.get("starts").and_then(Json::list).unwrap()[0];
        let scores = shown.get("targets").and_then(Json::list).unwrap();
        assert_eq!(scores.len(), states.len(), "{label}");
        for ((at, state), score) in states.iter().zip(scores) {
            assert_eq!(int_at(score, "frame"), i64::from(*at), "{label}");
            assert_score(
                &format!("{label} frame {at}"),
                score,
                &host_score(size, state, state, None),
            );
        }
        let got = long::returned(&mut server, &asked, &answer);
        let wanted: Vec<(u32, [u32; 3], u32)> = served::set_of(size, &end)
            .into_iter()
            .map(|(spot, word)| (0, spot, word))
            .collect();
        assert!(got == wanted, "{label}: returned cells differ");
    }
}

#[test]
fn c3_targets_of_every_form_count_against_the_host_memory_limit() {
    let Some(host) = device("law_check c3 memory") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let law = quoted(&learned(&mut server, &life(), false));
    let most = grit::serve3::check::HOST_MOST;
    let mib = 1_u64 << 20;
    let ask = |size: &str, targets: &str, more: &str| {
        format!(
            "{{\"law\":{law},\"alternate\":[0,1],\"size\":{size},\"frames\":1,\"starts\":[{{\"cells\":[],\"targets\":[{targets}]}}]{more},\"budget_ms\":{LONG}}}"
        )
    };
    let from_world = "{\"world\":999,\"from\":[0,0,0]}";
    let worlds = |many: usize| vec![from_world; many].join(",");
    let big = "[256,128,128]";
    assert_eq!(656 * mib + 23 * 16 * mib, most);
    let got = no(&mut server, "grit_law_check", &ask(big, &worlds(23), ""));
    assert_eq!(
        (text_at(&got, "code"), text_at(&got, "field")),
        (
            "no_world".to_owned(),
            "$.starts[0].targets[0].world".to_owned()
        ),
        "{}",
        written(&got)
    );
    let got = refused_with(
        &mut server,
        &ask(big, &worlds(24), ""),
        "too_many",
        "$.starts",
    );
    assert_eq!(int_at(&got, "limit") as u64, most);
    let counted = (656 + 24 * 16) * mib;
    assert!(
        text_at(&got, "message").contains(&counted.to_string()),
        "{}",
        written(&got)
    );
    let got = refused_with(
        &mut server,
        &ask(big, &worlds(23), ",\"return\":true"),
        "too_many",
        "$.starts",
    );
    assert_eq!(int_at(&got, "limit") as u64, most);
    let empty = vec!["{\"cells\":[]}"; 1024].join(",");
    let got = ok(
        &mut server,
        "grit_law_check",
        &ask("[64,64,64]", &empty, ""),
    );
    let totals = got.get("totals").unwrap();
    assert_eq!(int_at(totals, "targets"), 1024);
    assert_eq!(int_at(totals, "wrong"), 0);
    refused_with(
        &mut server,
        &ask("[64,64,64]", &worlds(1024), ""),
        "too_many",
        "$.starts[0].targets",
    );
    refused_with(
        &mut server,
        &format!(
            "{{\"law\":{law},\"alternate\":[0,1],\"size\":[64,64,64],\"frames\":1,\"starts\":[{{\"cells\":[],\"targets\":[{}]}},{{\"cells\":[],\"targets\":[{}]}}],\"budget_ms\":{LONG}}}",
            worlds(100),
            worlds(900)
        ),
        "too_many",
        "$.starts[1].targets",
    );
}

#[test]
fn c3_a_zero_border_check_at_the_host_memory_limit_runs_and_one_past_it_is_refused() {
    let Some(host) = device("law_check c3 memory zero") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let law = quoted(&learned(&mut server, &life(), false));
    let most = grit::serve3::check::HOST_MOST;
    let mib = 1_u64 << 20;
    let made = ok(
        &mut server,
        "grit_world_create",
        &format!(
            "{{\"wide\":128,\"deep\":64,\"tall\":64,\"law\":{law},\"params\":[],\"alternate\":[0,1],\"budget_ms\":{LONG}}}"
        ),
    );
    let world = int_at(&made, "world");
    served::write(&mut server, world, 0, &[([3, 4, 5], 1), ([4, 4, 5], 1)]);
    let ask = |many: usize, aimed: usize| {
        let target = format!("{{\"world\":{world},\"from\":[0,0,0],\"plane\":0}}");
        let targets = vec![target.as_str(); aimed].join(",");
        let mut starts = vec![format!(
            "{{\"cells\":[[1,1,1,1],[2,1,1,1],[3,1,1,1]],\"targets\":[{targets}]}}"
        )];
        starts.extend((1..many).map(|_| "{\"cells\":[[1,1,1,1]]}".to_owned()));
        format!(
            "{{\"law\":{law},\"alternate\":[0,1],\"size\":[128,64,64],\"border\":\"zero\",\"frames\":1,\"starts\":[{}],\"budget_ms\":{LONG}}}",
            starts.join(",")
        )
    };
    assert_eq!((66 * 8 + 2 * 58 + 380) * mib, most);
    let got = ok(&mut server, "grit_law_check", &ask(8, 58));
    assert_eq!(run_of(&got), ("apart".to_owned(), 8));
    let totals = got.get("totals").unwrap();
    assert_eq!(int_at(totals, "starts"), 8);
    assert_eq!(int_at(totals, "targets"), 58);
    assert_eq!(int_at(totals, "cells"), 58 * 128 * 64 * 64);
    assert_eq!(int_at(totals, "wrong"), 58 * 5);
    let got = refused_with(&mut server, &ask(8, 59), "too_many", "$.starts");
    assert_eq!(int_at(&got, "limit") as u64, most);
    let counted = (66 * 8 + 2 * 59 + 380) * mib;
    assert!(
        text_at(&got, "message").contains(&counted.to_string()),
        "{}",
        written(&got)
    );
}
