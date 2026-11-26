#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;
#[path = "law_driven/data.rs"]
mod data;
#[path = "law_driven/fed.rs"]
mod fed;
#[path = "law_driven/fit.rs"]
mod fit;
#[path = "law_check/long.rs"]
mod long;
#[path = "world_pages/served.rs"]
mod paged;
#[path = "law_parallel/pinned.rs"]
#[allow(dead_code)]
mod pinned;
#[path = "law_driven/pins.rs"]
mod pins;
#[path = "law_check/served.rs"]
mod served;

use common::device;

use grit::serve3::Server;
use grit::wire::{Json, written};

use fed::{Fed, Form, Kind, cells_at, created, plane_words, put, shown, spread, stepped_with};
use pinned::{LONG, bays, life};
use served::{
    FRAC, Rng, box_words, count, heat_law, int_at, learned, listed, no, ok, quoted, set_of,
    text_at, words_hash,
};

const MAJORITY: &str = include_str!("world_write/majority.law.json");

struct Case {
    label: &'static str,
    law: String,
    params: &'static str,
    alternate: bool,
    size: [u32; 3],
    sparse: bool,
    absorb: Option<[u32; 3]>,
    kind: Kind,
    form: Form,
    plane: Option<u32>,
    frames: u32,
    planes: u32,
    soup: ([u32; 3], [u32; 3]),
}

fn start_words(rng: &mut Rng, case: &Case) -> Vec<([u32; 3], u32)> {
    let (from, to) = case.soup;
    let mut cells = Vec::new();
    for z in from[2]..=to[2] {
        for y in from[1]..=to[1] {
            for x in from[0]..=to[0] {
                let word = fed::word_of(rng, case.kind);
                if word != 0 {
                    cells.push(([x, y, z], word));
                }
            }
        }
    }
    cells
}

fn world_of(server: &mut Server<'_>, case: &Case) -> i64 {
    created(
        server,
        &case.law,
        case.params,
        case.size,
        (case.alternate, case.sparse, case.absorb),
    )
}

fn d1_case(server: &mut Server<'_>, case: &Case, seed: u64) {
    let mut rng = Rng(seed);
    let start = start_words(&mut rng, case);
    let count_of = if case.form == Form::BoxBits { 0 } else { 24 };
    let fed = if case.form == Form::BoxBits {
        let from = [case.soup.0[0] + 2, case.soup.0[1] + 3, case.soup.0[2]];
        let to = [from[0] + 4, from[1] + 2, from[2]];
        Fed::boxed(&mut rng, from, to, case.frames, Kind::Bit, case.plane)
    } else {
        let cells = spread(&mut rng, case.size, count_of, case.soup);
        Fed::new(&mut rng, cells, case.frames, case.kind, case.plane)
    };
    let side = fed.side();
    let short = world_of(server, case);
    let long_world = world_of(server, case);
    let untouched = world_of(server, case);
    for world in [short, long_world, untouched] {
        put(server, world, "\"plane\":0", &start);
    }
    let answer = stepped_with(server, short, case.frames, &fed.json(case.form));
    assert_eq!(
        int_at(&answer, "frame"),
        i64::from(case.frames),
        "{}",
        case.label
    );
    let told = answer.get("driven").unwrap();
    assert_eq!(
        int_at(told, "cells"),
        fed.cells.len() as i64,
        "{}",
        case.label
    );
    assert_eq!(
        int_at(told, "words"),
        (fed.cells.len() as u64 * u64::from(case.frames)) as i64,
        "{}",
        case.label
    );
    for at in 0..case.frames as usize {
        put(server, long_world, &side, &fed.row(at, [0, 0, 0]));
        ok(
            server,
            "grit_world_step",
            &format!("{{\"world\":{long_world},\"frames\":1,\"budget_ms\":{LONG}}}"),
        );
    }
    ok(
        server,
        "grit_world_step",
        &format!(
            "{{\"world\":{untouched},\"frames\":{},\"budget_ms\":{LONG}}}",
            case.frames
        ),
    );
    let mut differs = false;
    for plane in 0..case.planes {
        let one = plane_words(server, short, plane, case.size);
        let other = plane_words(server, long_world, plane, case.size);
        assert!(
            one == other,
            "{}: plane {plane} differs from the long way ({} vs {})",
            case.label,
            words_hash(&one),
            words_hash(&other)
        );
        differs |= one != plane_words(server, untouched, plane, case.size);
    }
    assert!(differs, "{}: driving changed nothing", case.label);
    let last = case.plane.unwrap_or((case.frames - 1) & 1);
    let shell = case.absorb.unwrap_or([0; 3]);
    let inside = |cell: &[u32; 3]| {
        (0..3).all(|axis| {
            shell[axis] == 0
                || (cell[axis] >= shell[axis] && cell[axis] < case.size[axis] - shell[axis])
        })
    };
    let (kept, words): (Vec<[u32; 3]>, Vec<u32>) = fed
        .cells
        .iter()
        .zip(&fed.values[case.frames as usize - 1])
        .filter(|(cell, _)| inside(cell))
        .map(|(cell, word)| (*cell, *word))
        .unzip();
    assert!(!kept.is_empty(), "{}", case.label);
    let seen = cells_at(server, short, last, &kept);
    assert_eq!(
        seen, words,
        "{}: the plane driven at the last frame does not show its words",
        case.label
    );
    for world in [short, long_world, untouched] {
        ok(
            server,
            "grit_world_close",
            &format!("{{\"world\":{world}}}"),
        );
    }
}

fn parallel_life(server: &mut Server<'_>) -> String {
    learned(server, &life(), true)
}

#[test]
fn d1_a_driven_step_equals_writing_then_stepping_one_frame_at_a_time() {
    let Some(host) = device("law driven d1") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let life_law = learned(&mut server, &life(), false);
    let bays_law = learned(&mut server, &bays(), false);
    let parallel = parallel_life(&mut server);
    let heat = heat_law(&mut server);
    let flat = ([0, 0, 0], [63, 63, 0]);
    let cube = ([4, 4, 4], [27, 27, 27]);
    let cases = [
        Case {
            label: "life 2d dense wrapped list",
            law: life_law.clone(),
            params: "[]",
            alternate: true,
            size: [64, 64, 2],
            sparse: false,
            absorb: None,
            kind: Kind::Bit,
            form: Form::List,
            plane: None,
            frames: 9,
            planes: 2,
            soup: flat,
        },
        Case {
            label: "life 2d dense absorb box bits",
            law: life_law.clone(),
            params: "[]",
            alternate: true,
            size: [64, 64, 2],
            sparse: false,
            absorb: Some([3, 3, 0]),
            kind: Kind::Bit,
            form: Form::BoxBits,
            plane: None,
            frames: 8,
            planes: 2,
            soup: flat,
        },
        Case {
            label: "life 2d layers sparse words",
            law: life_law.clone(),
            params: "[]",
            alternate: true,
            size: [64, 64, 64],
            sparse: true,
            absorb: None,
            kind: Kind::Bit,
            form: Form::Words,
            plane: None,
            frames: 7,
            planes: 2,
            soup: ([8, 8, 10], [47, 47, 12]),
        },
        Case {
            label: "life 2d layers sparse absorb list",
            law: life_law,
            params: "[]",
            alternate: true,
            size: [64, 64, 64],
            sparse: true,
            absorb: Some([2, 2, 0]),
            kind: Kind::Bit,
            form: Form::List,
            plane: None,
            frames: 6,
            planes: 2,
            soup: ([0, 0, 30], [40, 40, 31]),
        },
        Case {
            label: "bays 3d dense list",
            law: bays_law.clone(),
            params: "[]",
            alternate: true,
            size: [32, 32, 32],
            sparse: false,
            absorb: None,
            kind: Kind::Bit,
            form: Form::List,
            plane: None,
            frames: 5,
            planes: 2,
            soup: cube,
        },
        Case {
            label: "bays 3d sparse absorb words",
            law: bays_law,
            params: "[]",
            alternate: true,
            size: [64, 64, 64],
            sparse: true,
            absorb: Some([2, 2, 2]),
            kind: Kind::Bit,
            form: Form::Words,
            plane: None,
            frames: 4,
            planes: 2,
            soup: ([6, 6, 6], [29, 29, 29]),
        },
        Case {
            label: "life parallel 32-bit words",
            law: parallel,
            params: "[]",
            alternate: true,
            size: [64, 64, 2],
            sparse: false,
            absorb: None,
            kind: Kind::Word,
            form: Form::Words,
            plane: None,
            frames: 6,
            planes: 2,
            soup: flat,
        },
        Case {
            label: "heat fixed-point wrapped list",
            law: heat.clone(),
            params: "[]",
            alternate: true,
            size: [32, 32, 2],
            sparse: false,
            absorb: None,
            kind: Kind::Fixed,
            form: Form::List,
            plane: None,
            frames: 10,
            planes: 2,
            soup: ([0, 0, 0], [31, 31, 0]),
        },
        Case {
            label: "heat fixed-point absorb words",
            law: heat,
            params: "[]",
            alternate: true,
            size: [32, 32, 2],
            sparse: false,
            absorb: Some([2, 2, 0]),
            kind: Kind::Fixed,
            form: Form::Words,
            plane: None,
            frames: 7,
            planes: 2,
            soup: ([0, 0, 0], [31, 31, 0]),
        },
        Case {
            label: "majority plane 0 without alternate",
            law: MAJORITY.to_owned(),
            params: "[]",
            alternate: false,
            size: [64, 64, 2],
            sparse: false,
            absorb: None,
            kind: Kind::Bit,
            form: Form::List,
            plane: Some(0),
            frames: 6,
            planes: 2,
            soup: flat,
        },
    ];
    for (at, case) in cases.iter().enumerate() {
        d1_case(&mut server, case, 0xD1_0000 + at as u64);
    }
}

fn check_case(label: &str, law: &str, size: [u32; 3], steps: u32) -> long::Case {
    long::Case {
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

struct Long {
    states: Vec<(u32, Vec<u32>)>,
    fingerprint: String,
}

fn long_check(
    server: &mut Server<'_>,
    case: &long::Case,
    start: &[u32],
    fed: Option<&Fed>,
    stops: &[u32],
) -> Long {
    let (size, origin, absorb) = case.world_size();
    let world = created(
        server,
        &case.law,
        &listed(&case.params),
        size,
        (case.alternate, case.sparse, absorb),
    );
    let cells: Vec<([u32; 3], u32)> = set_of(case.size, start)
        .into_iter()
        .map(|(at, word)| ([0, 1, 2].map(|axis| at[axis] + origin[axis]), word))
        .collect();
    put(
        server,
        world,
        &format!("\"plane\":{}", case.plane(true, 0)),
        &cells,
    );
    let mut wanted = stops.to_vec();
    wanted.push(case.steps);
    wanted.sort_unstable();
    wanted.dedup();
    let mut now = 0;
    let mut states = Vec::new();
    for at in wanted {
        while now < at {
            match fed {
                Some(given) => {
                    let side = match given.plane {
                        Some(plane) => format!("\"plane\":{plane}"),
                        None => format!("\"plane\":{}", case.plane(true, now)),
                    };
                    put(server, world, &side, &given.row(now as usize, origin));
                    ok(
                        server,
                        "grit_world_step",
                        &format!("{{\"world\":{world},\"frames\":1,\"budget_ms\":{LONG}}}"),
                    );
                    now += 1;
                }
                None => {
                    ok(
                        server,
                        "grit_world_step",
                        &format!(
                            "{{\"world\":{world},\"frames\":{},\"budget_ms\":{LONG}}}",
                            at - now
                        ),
                    );
                    now = at;
                }
            }
        }
        states.push((
            at,
            box_words(server, world, case.plane(false, at), origin, case.size),
        ));
    }
    let mut every = Vec::new();
    for plane in 0..case.planes {
        every.extend(box_words(server, world, plane, origin, case.size));
    }
    ok(
        server,
        "grit_world_close",
        &format!("{{\"world\":{world}}}"),
    );
    Long {
        states,
        fingerprint: words_hash(&every),
    }
}

fn flipped(words: &[u32], at: usize, real: bool) -> Vec<u32> {
    let mut out = words.to_vec();
    out[at] = if real {
        out[at].wrapping_add(1 << 10)
    } else {
        out[at] ^ 1
    };
    out
}

fn check_driven(server: &mut Server<'_>, case: &long::Case, seed: u64, more: &str) {
    let mut rng = Rng(seed);
    let real = case.frac.is_some();
    let kind = if real { Kind::Fixed } else { Kind::Bit };
    let box_cells = count(case.size);
    let starts: Vec<Vec<u32>> = (0..3)
        .map(|_| {
            (0..box_cells)
                .map(|_| fed::word_of(&mut rng, kind))
                .collect()
        })
        .collect();
    let whole = ([0, 0, 0], case.size.map(|side| side - 1));
    let feds: Vec<Option<Fed>> = (0..3)
        .map(|at| {
            (at != 1).then(|| {
                let cells = spread(&mut rng, case.size, 9, whole);
                Fed::new(&mut rng, cells, case.steps, kind, None)
            })
        })
        .collect();
    let stops = [case.steps / 2, case.steps];
    let mut shown_starts = Vec::new();
    let mut wanted = Vec::new();
    for (start, given) in starts.iter().zip(&feds) {
        let long = long_check(server, case, start, given.as_ref(), &stops);
        let driven_at: Vec<usize> = given
            .as_ref()
            .map(|one| {
                one.cells
                    .iter()
                    .map(|cell| served::index(case.size, *cell))
                    .collect()
            })
            .unwrap_or_default();
        let other = (0..box_cells).find(|at| !driven_at.contains(at)).unwrap();
        let mut targets = Vec::new();
        let mut scores = Vec::new();
        for (when, state) in &long.states {
            let mut aimed = flipped(state, other, real);
            if let Some(cell) = driven_at.first() {
                aimed = flipped(&aimed, *cell, real);
            }
            targets.push(format!("{{\"frame\":{when},\"dense\":{}}}", listed(&aimed)));
            let left_out = driven_at.len();
            scores.push(((box_cells - left_out) as i64, 1_i64));
        }
        let driven = given
            .as_ref()
            .map(|one| format!(",\"driven\":{}", one.json(Form::List)))
            .unwrap_or_default();
        shown_starts.push(format!(
            "{{\"dense\":{}{driven},\"targets\":[{}]}}",
            listed(start),
            targets.join(",")
        ));
        wanted.push((long.fingerprint, scores, driven_at.len()));
    }
    let alternate = if case.alternate {
        ",\"alternate\":[0,1]"
    } else {
        ""
    };
    let border = if case.zero { "zero" } else { "wrapped" };
    let absorb = case
        .absorb
        .map(|shell| format!(",\"absorb\":{}", listed(&shell)))
        .unwrap_or_default();
    let values = case
        .frac
        .map(|frac| format!(",\"values\":\"real\",\"frac\":{frac}"))
        .unwrap_or_default();
    let got = ok(
        server,
        "grit_law_check",
        &format!(
            "{{\"law\":{},\"params\":{}{alternate},\"size\":{},\"sparse\":{},\"border\":\"{border}\"{absorb}{values},\"frames\":{},\"starts\":[{}]{more},\"budget_ms\":{LONG}}}",
            quoted(&case.law),
            listed(&case.params),
            listed(&case.size),
            case.sparse,
            case.steps,
            shown_starts.join(",")
        ),
    );
    assert!(
        got.get("driven").is_some(),
        "{}: {}",
        case.label,
        shown(&got)
    );
    let run = got.get("run").unwrap();
    assert_eq!(text_at(run, "layout"), "apart", "{}", case.label);
    let listed_starts = got.get("starts").and_then(Json::list).unwrap();
    for (at, (one, (fingerprint, scores, driven))) in listed_starts.iter().zip(&wanted).enumerate()
    {
        assert_eq!(
            &text_at(one, "fingerprint"),
            fingerprint,
            "{} start {at}: the end state differs from the long way",
            case.label
        );
        match one.get("driven") {
            Some(held) => assert_eq!(held.int(), Some(*driven as i64), "{}", case.label),
            None => assert_eq!(*driven, 0, "{}", case.label),
        }
        let targets = one.get("targets").and_then(Json::list).unwrap();
        for (target, (cells, flips)) in targets.iter().zip(scores) {
            let flips = *flips;
            assert_eq!(int_at(target, "cells"), *cells, "{} start {at}", case.label);
            assert_eq!(int_at(target, "wrong"), flips, "{} start {at}", case.label);
            assert_eq!(
                int_at(target, "right"),
                *cells - flips,
                "{} start {at}",
                case.label
            );
        }
    }
}

#[test]
fn d1_a_driven_check_equals_the_same_check_run_the_long_way() {
    let Some(host) = device("law driven d1 check") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let life_law = learned(&mut server, &life(), false);
    let bays_law = learned(&mut server, &bays(), false);
    let heat = heat_law(&mut server);
    check_driven(
        &mut server,
        &check_case("life wrapped", &life_law, [32, 32, 2], 7),
        0xC1,
        "",
    );
    let mut zero = check_case("life zero border", &life_law, [32, 32, 2], 6);
    zero.zero = true;
    check_driven(&mut server, &zero, 0xC2, "");
    let mut sparse = check_case("bays 3d sparse absorb", &bays_law, [32, 32, 32], 4);
    sparse.sparse = true;
    sparse.absorb = Some([1, 1, 1]);
    check_driven(&mut server, &sparse, 0xC3, "");
    check_driven(
        &mut server,
        &check_case("bays 3d wrapped", &bays_law, [16, 16, 16], 4),
        0xC4,
        "",
    );
    let mut real = check_case("heat zero border real", &heat, [16, 16, 2], 8);
    real.zero = true;
    real.frac = Some(FRAC);
    check_driven(&mut server, &real, 0xC5, "");
}

#[test]
fn d2_a_driven_fit_equals_the_fit_rewritten_by_hand_and_recovers_the_source_law() {
    let Some(host) = device("law driven d2") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    fit::d2(&mut server);
}

#[test]
fn d3_an_a2_shaped_held_start_runs_in_three_calls() {
    let Some(host) = device("law driven d3") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    fit::d3(&mut server);
}

fn refused_with(
    server: &mut Server<'_>,
    name: &str,
    arguments: &str,
    code: &str,
    field: &str,
) -> Json {
    let got = no(server, name, arguments);
    assert_eq!(
        (
            text_at(&got, "refused").as_str(),
            text_at(&got, "field").as_str()
        ),
        (code, field),
        "{arguments:.400}: {}",
        shown(&got)
    );
    got
}

fn step_args(world: i64, frames: u32, driven: &str) -> String {
    format!("{{\"world\":{world},\"frames\":{frames},\"driven\":{driven},\"budget_ms\":{LONG}}}")
}

#[test]
fn d4_each_driven_limit_is_refused_by_name_and_nothing_runs() {
    let Some(host) = device("law driven d4") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let life_law = learned(&mut server, &life(), false);
    let world = created(
        &mut server,
        &life_law,
        "[]",
        [64, 64, 2],
        (true, false, None),
    );
    let mut rng = Rng(0xD4);
    let start: Vec<([u32; 3], u32)> = (0..400)
        .map(|_| ([rng.below(64) as u32, rng.below(64) as u32, 0], 1))
        .collect();
    put(&mut server, world, "\"plane\":0", &start);
    let before = [
        plane_words(&mut server, world, 0, [64, 64, 2]),
        plane_words(&mut server, world, 1, [64, 64, 2]),
    ];
    let one = |cells: &str, values: &str| format!("{{\"current\":true,{cells},{values}}}");
    let got = refused_with(
        &mut server,
        "grit_world_step",
        &step_args(
            world,
            2,
            &one("\"cells\":[[1,1,0],[64,3,0]]", "\"values\":[[1,0],[0,1]]"),
        ),
        "outside",
        "$.driven.cells[1]",
    );
    assert_eq!(written(got.get("limit").unwrap()), "[63,63,1]");
    refused_with(
        &mut server,
        "grit_world_step",
        &step_args(
            world,
            2,
            &one(
                "\"from\":[60,60,0],\"to\":[64,61,0]",
                "\"values\":[[1],[1]]",
            ),
        ),
        "outside",
        "$.driven.to",
    );
    let got = refused_with(
        &mut server,
        "grit_world_step",
        &step_args(
            world,
            3,
            &one("\"cells\":[[1,1,0],[2,2,0]]", "\"values\":[[1,0],[0,1]]"),
        ),
        "driven_frames",
        "$.driven.values",
    );
    assert_eq!(got.get("limit").and_then(Json::int), Some(3));
    let got = refused_with(
        &mut server,
        "grit_world_step",
        &step_args(
            world,
            2,
            &one("\"cells\":[[1,1,0],[2,2,0]]", "\"values\":[[1,0],[0]]"),
        ),
        "driven_length",
        "$.driven.values[1]",
    );
    assert_eq!(got.get("limit").and_then(Json::int), Some(2));
    let three_frames = paged::encoded(&paged::packed("words", &[1, 0, 1, 1, 0, 0]));
    refused_with(
        &mut server,
        "grit_world_step",
        &step_args(
            world,
            2,
            &one(
                "\"cells\":[[1,1,0],[2,2,0]]",
                &format!("\"format\":\"words\",\"data\":\"{three_frames}\""),
            ),
        ),
        "driven_frames",
        "$.driven.data",
    );
    let odd = paged::encoded(&paged::packed("words", &[1, 0, 1]));
    refused_with(
        &mut server,
        "grit_world_step",
        &step_args(
            world,
            2,
            &one(
                "\"cells\":[[1,1,0],[2,2,0]]",
                &format!("\"format\":\"words\",\"data\":\"{odd}\""),
            ),
        ),
        "driven_length",
        "$.driven.data",
    );
    refused_with(
        &mut server,
        "grit_world_step",
        &step_args(
            world,
            1,
            "{\"plane\":0,\"cells\":[[1,1,0]],\"values\":[[1]]}",
        ),
        "driven_written",
        "$.driven.plane",
    );
    refused_with(
        &mut server,
        "grit_world_step",
        &step_args(
            world,
            1,
            "{\"plane\":1,\"cells\":[[1,1,0]],\"values\":[[1]]}",
        ),
        "driven_written",
        "$.driven.plane",
    );
    refused_with(
        &mut server,
        "grit_world_step",
        &step_args(
            world,
            1,
            "{\"plane\":2,\"cells\":[[1,1,0]],\"values\":[[1]]}",
        ),
        "outside",
        "$.driven.plane",
    );
    refused_with(
        &mut server,
        "grit_world_step",
        &step_args(
            world,
            1,
            &one("\"cells\":[[1,1,0],[1,1,0]]", "\"values\":[[1,0]]"),
        ),
        "malformed",
        "$.driven.cells[1]",
    );
    let wide = grit::serve3::page::PAGE_WORDS as usize;
    let frames = 13;
    let many = wide / frames as usize + 1;
    let cells: Vec<String> = (0..many)
        .map(|at| format!("[{},{},{}]", at % 64, at / 64 % 64, at / 4096))
        .collect();
    let got = refused_with(
        &mut server,
        "grit_world_step",
        &step_args(
            world,
            frames,
            &format!(
                "{{\"current\":true,\"cells\":[{}],\"values\":[]}}",
                cells.join(",")
            ),
        ),
        "page_limit",
        "$.driven",
    );
    assert_eq!(got.get("limit").and_then(Json::int), Some(wide as i64));
    let majority = created(
        &mut server,
        MAJORITY,
        "[]",
        [64, 64, 2],
        (false, false, None),
    );
    refused_with(
        &mut server,
        "grit_world_step",
        &step_args(
            majority,
            1,
            "{\"plane\":1,\"cells\":[[1,1,0]],\"values\":[[1]]}",
        ),
        "driven_written",
        "$.driven.plane",
    );
    refused_with(
        &mut server,
        "grit_world_step",
        &step_args(
            majority,
            1,
            "{\"current\":true,\"cells\":[[1,1,0]],\"values\":[[1]]}",
        ),
        "not_iterating",
        "$.driven.current",
    );
    let rule = ok(
        &mut server,
        "grit_world_create",
        "{\"wide\":64,\"deep\":64,\"tall\":2,\"rule\":\"sand\"}",
    );
    let rule_world = int_at(&rule, "world");
    refused_with(
        &mut server,
        "grit_world_step",
        &step_args(
            rule_world,
            1,
            "{\"plane\":0,\"cells\":[[1,1,0]],\"values\":[[1]]}",
        ),
        "driven_written",
        "$.driven",
    );
    let after = [
        plane_words(&mut server, world, 0, [64, 64, 2]),
        plane_words(&mut server, world, 1, [64, 64, 2]),
    ];
    assert!(before == after, "a refused driven step changed the world");
    let fine = stepped_with(
        &mut server,
        world,
        1,
        &one("\"cells\":[[1,1,0]]", "\"values\":[[1]]"),
    );
    assert_eq!(
        int_at(&fine, "frame"),
        1,
        "a refused step advanced the world"
    );
    let start = listed(&vec![0_u32; 32 * 32 * 2]);
    let check = |driven: &str| {
        format!(
            "{{\"law\":{},\"alternate\":[0,1],\"size\":[32,32,2],\"frames\":2,\"starts\":[{{\"dense\":{start}}},{{\"dense\":{start},\"driven\":{driven}}}],\"budget_ms\":{LONG}}}",
            quoted(&life_law)
        )
    };
    refused_with(
        &mut server,
        "grit_law_check",
        &check("{\"cells\":[[32,0,0]],\"values\":[[1],[1]]}"),
        "outside",
        "$.starts[1].driven.cells[0]",
    );
    refused_with(
        &mut server,
        "grit_law_check",
        &check("{\"cells\":[[3,0,0]],\"values\":[[1]]}"),
        "driven_frames",
        "$.starts[1].driven.values",
    );
    refused_with(
        &mut server,
        "grit_law_check",
        &check("{\"plane\":1,\"cells\":[[3,0,0]],\"values\":[[1],[1]]}"),
        "driven_written",
        "$.starts[1].driven.plane",
    );
    fit::d4_fits(&mut server);
}

#[test]
fn d5_base_answers_keep_their_bytes() {
    let Some(host) = device("law driven d5") else {
        return;
    };
    let got = [
        (
            "step-session",
            pins::step_session(&mut Server::new(Some(&host))),
        ),
        (
            "check-session",
            pins::check_session(&mut Server::new(Some(&host))),
        ),
        (
            "learn-session",
            pins::learn_session(&mut Server::new(Some(&host))),
        ),
        ("tools-list", pins::listing(&mut Server::new(None))),
        ("describe", pins::described(&mut Server::new(None))),
    ];
    for (name, hash) in &got {
        println!("RECORD {name} {hash}");
    }
    let mut differing = Vec::new();
    for ((name, hash), (pinned, wanted)) in got.iter().zip(pins::RECORDED) {
        assert_eq!(*name, pinned);
        if hash != wanted {
            differing.push(*name);
        }
    }
    assert!(
        differing.is_empty(),
        "the answers differ from the base's: {differing:?}"
    );
    pins::new_forms_described(&mut Server::new(None));
}
