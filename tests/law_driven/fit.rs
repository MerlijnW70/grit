use grit::serve3::Server;
use grit::serve3::law_steps::fit::{Clip, Drive, Setup};
use grit::serve3::law_steps::fitted;
use grit::serve3::law_steps::model::{Basis, Grid, Planes};
use grit::stencil::{Border, Stencil};
use grit::wire::{Json, written};

use super::data::{
    ALPHA, CENTRE, Example, HOLDOUT, SHOTS, SIDE, SOURCE, STEPS, at_of, example, request,
};
use super::fed::Fed;
use super::paged::{encoded, packed};
use super::pinned::LONG;
use super::served::{FRAC, Rng, int_at, listed, no, ok, quoted, text_at};

pub const TOLERANCE: f64 = 2e-3;

fn sequence(example: &Example) -> String {
    let shots: Vec<String> = example
        .shots
        .iter()
        .map(|shot| format!("[{}]", listed(shot)))
        .collect();
    format!("[{}]", shots.join(","))
}

fn driven_list(example: &Example, source: [u32; 3]) -> String {
    let rows: Vec<String> = example
        .pulses
        .iter()
        .map(|one| format!("[{one}]"))
        .collect();
    format!(
        "{{\"plane\":0,\"cells\":[{}],\"values\":[{}]}}",
        listed(&source),
        rows.join(",")
    )
}

fn driven_page(example: &Example, source: [u32; 3]) -> String {
    format!(
        "{{\"cells\":[{}],\"format\":\"words\",\"data\":\"{}\"}}",
        listed(&source),
        encoded(&packed("words", &example.pulses))
    )
}

fn shown_example(example: &Example, driven: Option<String>) -> String {
    let driven = driven
        .map(|one| format!(",\"driven\":{one}"))
        .unwrap_or_default();
    format!(
        "{{\"size\":[{SIDE},{SIDE},1],\"sequence\":{}{driven}}}",
        sequence(example)
    )
}

fn fits(answer: &Json) -> Vec<String> {
    let mut out: Vec<String> = answer.get("coefficients").and_then(Json::list).unwrap()[0]
        .list()
        .unwrap()
        .iter()
        .map(|one| text_at(one, "fit"))
        .collect();
    out.push(text_at(
        &answer.get("bias").and_then(Json::list).unwrap()[0],
        "fit",
    ));
    out
}

fn wanted_of(answer: &Json) -> Vec<f64> {
    let terms = answer.get("terms").and_then(Json::list).unwrap();
    let mut out: Vec<f64> = terms
        .iter()
        .map(|term| {
            let at = term.get("at").and_then(Json::list).unwrap();
            let offset: Vec<i64> = at.iter().map(|one| one.int().unwrap()).collect();
            if offset == [0, 0, 0] { CENTRE } else { ALPHA }
        })
        .collect();
    out.push(0.0);
    out
}

fn worst(answer: &Json) -> f64 {
    fits(answer)
        .iter()
        .zip(wanted_of(answer))
        .map(|(got, want)| (got.parse::<f64>().unwrap() - want).abs())
        .fold(0.0, f64::max)
}

fn residual(answer: &Json) -> f64 {
    let list = answer.get("residual").and_then(Json::list).unwrap();
    text_at(&list[0], "fit_rms_real").parse().unwrap()
}

struct Hand {
    cell: usize,
    pulses: Vec<f64>,
}

impl Drive for Hand {
    fn put(&self, sub: usize, planes: &mut [Vec<f64>]) {
        planes[0][self.cell] = self.pulses[sub];
    }

    fn driven(&self, read: usize, cell: usize) -> bool {
        read == 0 && cell == self.cell
    }
}

fn real(word: u32) -> f64 {
    f64::from(word.cast_signed()) / f64::from(1_u32 << FRAC)
}

fn by_hand(examples: &[Example]) -> Vec<String> {
    let stencil = Stencil::Square;
    let mask = [[0, -1, 0], [-1, 0, 0], [0, 0, 0], [1, 0, 0], [0, 1, 0]]
        .iter()
        .fold(0_u32, |held, at| held | (1 << stencil.place(*at)));
    let basis = Basis::new(stencil, mask, 1, 0, 1);
    let size = [SIDE, SIDE, 1];
    let fitted_shots = SHOTS - HOLDOUT;
    let fitted_subs = (fitted_shots - 1) * STEPS as usize;
    let mut least = i32::MAX;
    let mut most = i32::MIN;
    for one in examples {
        for word in one.shots[..fitted_shots]
            .iter()
            .flatten()
            .chain(&one.pulses[..fitted_subs])
        {
            least = least.min(word.cast_signed());
            most = most.max(word.cast_signed());
        }
    }
    let range = [real(least.cast_unsigned()), real(most.cast_unsigned())];
    let clips: Vec<Clip> = examples
        .iter()
        .map(|one| Clip {
            grid: Grid::new(size, &basis, Border::Wrapped),
            shots: one.shots[..fitted_shots]
                .iter()
                .map(|shot| -> Planes { vec![shot.iter().map(|word| real(*word)).collect()] })
                .collect(),
            drive: Some(Box::new(Hand {
                cell: at_of(size, SOURCE),
                pulses: one.pulses.iter().map(|word| real(*word)).collect(),
            })),
        })
        .collect();
    let setup = Setup {
        basis: &basis,
        clips: &clips,
        steps: STEPS,
        limits: vec![range],
        settle: 0,
    };
    let reach = range[0].abs().max(range[1].abs());
    let found = fitted(&setup, reach, &mut || None).unwrap();
    let mut out: Vec<String> = (0..basis.terms.len())
        .map(|term| format!("{:.9e}", found.theta[basis.coefficient(0, term)]))
        .collect();
    out.push(format!("{:.9e}", found.theta[basis.bias(0)]));
    out
}

pub fn d2(server: &mut Server<'_>) {
    let mut rng = Rng(0xD2);
    let size = [SIDE, SIDE, 1];
    let examples: Vec<Example> = (0..3)
        .map(|at| example(&mut rng, size, at, SOURCE, SHOTS, STEPS))
        .collect();
    let listed_form: Vec<String> = examples
        .iter()
        .map(|one| shown_example(one, Some(driven_list(one, SOURCE))))
        .collect();
    let paged_form: Vec<String> = examples
        .iter()
        .map(|one| shown_example(one, Some(driven_page(one, SOURCE))))
        .collect();
    let bare: Vec<String> = examples
        .iter()
        .map(|one| shown_example(one, None))
        .collect();
    let driven = ok(server, "grit_law_learn", &request(&listed_form, ""));
    let paged = ok(server, "grit_law_learn", &request(&paged_form, ""));
    assert_eq!(
        written(&driven),
        written(&paged),
        "the page form answers other bytes than the list form"
    );
    let told = driven.get("driven").unwrap();
    assert_eq!(int_at(told, "examples"), 3);
    assert_eq!(int_at(told, "cells"), 3);
    let hand = by_hand(&examples);
    assert_eq!(
        fits(&driven),
        hand,
        "the driven fit differs from the fit with the source rewritten by hand each sub-step"
    );
    let plain = ok(server, "grit_law_learn", &request(&bare, ""));
    let (with, without) = (worst(&driven), worst(&plain));
    let (fit_with, fit_without) = (residual(&driven), residual(&plain));
    println!(
        "D2 driven: worst coefficient error {with:.3e}, fit rms {fit_with:.3e}; without driven: {without:.3e}, fit rms {fit_without:.3e}"
    );
    println!("D2 driven fit {:?}", fits(&driven));
    println!("D2 plain fit {:?}", fits(&plain));
    assert!(
        with < TOLERANCE,
        "the driven fit misses the source law by {with:.3e}, past {TOLERANCE:.0e}"
    );
    assert!(
        without > 10.0 * with && fit_without > 10.0 * fit_with,
        "without driven the fit should be worse: {without:.3e} vs {with:.3e}, rms {fit_without:.3e} vs {fit_with:.3e}"
    );
}

pub const TISSUE: u32 = 25;

pub const MARGIN: u32 = 3;

pub const PACING: usize = 38;

pub const SUBS: u32 = 3;

pub const AHEAD: [u32; 3] = [1, 10, 50];

fn a2_law(server: &mut Server<'_>) -> (String, String) {
    let mut rng = Rng(0xA2);
    let size = [SIDE, SIDE, 1];
    let examples: Vec<String> = (0..2)
        .map(|at| {
            let one = example(&mut rng, size, at, SOURCE, 10, SUBS);
            shown_example(&one, Some(driven_list(&one, SOURCE)))
        })
        .collect();
    let got = ok(
        server,
        "grit_law_learn",
        &format!(
            "{{\"stencil\":\"3x3\",\"reads\":[{{\"plane\":0,\"bit\":0}}],\"writes\":[{{\"plane\":1,\"bit\":0}}],\"border\":\"wrapped\",\"rule\":\"linear\",\"places\":\"von_neumann\",\"frac\":{FRAC},\"steps\":{SUBS},\"hidden\":1,\"examples\":[{}],\"budget_ms\":{LONG}}}",
            examples.join(",")
        ),
    );
    let params: Vec<String> = got
        .get("params")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .map(|one| one.int().unwrap().to_string())
        .collect();
    (text_at(&got, "law"), format!("[{}]", params.join(",")))
}

struct Start {
    past: Vec<u32>,
    hidden: Vec<u32>,
    pacing: Vec<[u32; 3]>,
    next: Vec<Vec<u32>>,
}

fn a2_start(rng: &mut Rng) -> Start {
    let cells = (TISSUE * TISSUE) as usize;
    let past = (0..cells).map(|_| rng.below(4096) as u32).collect();
    let hidden = (0..cells).map(|_| rng.below(2048) as u32).collect();
    let mut pacing = Vec::new();
    while pacing.len() < PACING {
        let cell = [rng.below(6) as u32 + 2, rng.below(TISSUE as u64) as u32, 0];
        if !pacing.contains(&cell) {
            pacing.push(cell);
        }
    }
    let next = (0..AHEAD[2])
        .map(|_| (0..PACING).map(|_| rng.below(4096) as u32).collect())
        .collect();
    Start {
        past,
        hidden,
        pacing,
        next,
    }
}

struct Calls<'a, 'h> {
    server: &'a mut Server<'h>,
    count: usize,
}

impl Calls<'_, '_> {
    fn ok(&mut self, name: &str, arguments: &str) -> Json {
        self.count += 1;
        ok(self.server, name, arguments)
    }
}

fn layered(starts: &[Start]) -> ([u32; 3], Vec<u32>, Vec<u32>) {
    let size = [32, 32, (starts.len() as u32).next_power_of_two().max(2)];
    let layer = (size[0] * size[1]) as usize;
    let mut past = vec![0_u32; layer * size[2] as usize];
    let mut hidden = past.clone();
    for (z, start) in starts.iter().enumerate() {
        for y in 0..TISSUE {
            for x in 0..TISSUE {
                let from = (y * TISSUE + x) as usize;
                let to = z * layer + ((y + MARGIN) * size[0] + x + MARGIN) as usize;
                past[to] = start.past[from];
                hidden[to] = start.hidden[from];
            }
        }
    }
    (size, past, hidden)
}

fn feed(starts: &[Start], from: u32, to: u32) -> Fed {
    let mut cells = Vec::new();
    for (z, start) in starts.iter().enumerate() {
        for cell in &start.pacing {
            cells.push([cell[0] + MARGIN, cell[1] + MARGIN, z as u32]);
        }
    }
    let values = (from * SUBS..to * SUBS)
        .map(|sub| {
            let frame = (sub / SUBS) as usize;
            starts
                .iter()
                .flat_map(|start| start.next[frame].iter().copied())
                .collect()
        })
        .collect();
    Fed {
        cells,
        values,
        plane: None,
        from: None,
    }
}

fn replay(
    server: &mut Server<'_>,
    law: &(String, String),
    starts: &[Start],
    ahead: &[u32],
) -> (usize, Vec<Vec<u32>>) {
    let (size, past, hidden) = layered(starts);
    let mut calls = Calls { server, count: 0 };
    let pages = format!(
        "[{{\"plane\":0,\"data\":\"{}\"}},{{\"plane\":2,\"data\":\"{}\"}}]",
        encoded(&packed("words", &past)),
        encoded(&packed("words", &hidden))
    );
    let made = calls.ok(
        "grit_world_create",
        &format!(
            "{{\"wide\":{},\"deep\":{},\"tall\":{},\"law\":{},\"params\":{},\"absorb\":[{MARGIN},{MARGIN},0],\"pages\":{pages},\"budget_ms\":{LONG}}}",
            size[0],
            size[1],
            size[2],
            quoted(&law.0),
            law.1
        ),
    );
    let world = int_at(&made, "world");
    let mut now = 0;
    let mut frames = Vec::new();
    for horizon in ahead {
        let fed = feed(starts, now, *horizon);
        calls.ok(
            "grit_world_step",
            &format!(
                "{{\"world\":{world},\"frames\":{},\"driven\":{},\"budget_ms\":{LONG}}}",
                (horizon - now) * SUBS,
                fed.json(super::fed::Form::Words)
            ),
        );
        now = *horizon;
        let got = calls.ok(
            "grit_world_read",
            &format!(
                "{{\"world\":{world},\"current\":true,\"form\":\"page\",\"from\":[0,0,0],\"to\":[{},{},{}]}}",
                size[0] - 1,
                size[1] - 1,
                size[2] - 1
            ),
        );
        frames.push(super::paged::unpacked(
            "words",
            &super::paged::decoded(&text_at(&got, "data")),
            (size[0] * size[1] * size[2]) as usize,
        ));
    }
    let count = calls.count;
    ok(
        server,
        "grit_world_close",
        &format!("{{\"world\":{world}}}"),
    );
    (count, frames)
}

fn layer_of(words: &[u32], size: [u32; 3], z: u32) -> Vec<u32> {
    let layer = (size[0] * size[1]) as usize;
    words[z as usize * layer..(z as usize + 1) * layer].to_vec()
}

pub fn d3(server: &mut Server<'_>) {
    let law = a2_law(server);
    let mut rng = Rng(0xD3);
    let starts: Vec<Start> = (0..15).map(|_| a2_start(&mut rng)).collect();
    let (alone, _) = replay(server, &law, &starts[..1], &[AHEAD[2]]);
    println!("D3 one held start, frame 50 only: {alone} calls");
    assert_eq!(alone, 3, "create with pages, one driven step, one read");
    let (one_start, single) = replay(server, &law, &starts[..1], &AHEAD);
    println!("D3 one held start, frames 1, 10 and 50: {one_start} calls");
    assert_eq!(one_start, 7);
    let (all, together) = replay(server, &law, &starts, &AHEAD);
    let per = all as f64 / starts.len() as f64;
    println!(
        "D3 {} held starts as z layers, frames 1, 10 and 50: {all} calls, {per:.2} a start",
        starts.len()
    );
    assert_eq!(all, 7);
    assert!(per <= 3.0);
    let size_one = [32, 32, 2];
    let size_all = [32, 32, 16];
    for (frame, (alone_words, all_words)) in single.iter().zip(&together).enumerate() {
        assert_eq!(
            layer_of(alone_words, size_one, 0),
            layer_of(all_words, size_all, 0),
            "frame {} of start 0 differs between its own world and the layered one",
            AHEAD[frame]
        );
    }
}

pub fn d4_fits(server: &mut Server<'_>) {
    let mut rng = Rng(0xD4F);
    let size = [SIDE, SIDE, 1];
    let one = example(&mut rng, size, 0, SOURCE, 6, STEPS);
    let rows: Vec<String> = one.pulses.iter().map(|word| format!("[{word}]")).collect();
    let refused = |server: &mut Server<'_>, driven: &str, more: &str, code: &str, field: &str| {
        let shown_one = format!(
            "{{\"size\":[{SIDE},{SIDE},1],\"sequence\":{},\"driven\":{driven}}}",
            sequence(&one)
        );
        let got = no(
            server,
            "grit_law_learn",
            &request(&[shown_one], more).replace(&format!(",\"holdout\":{HOLDOUT}"), ""),
        );
        assert_eq!(
            (text_at(&got, "refused"), text_at(&got, "field")),
            (code.to_owned(), field.to_owned()),
            "{driven:.200}: {}",
            written(&got)
        );
        got
    };
    let short = format!(
        "{{\"cells\":[[5,7,0]],\"values\":[{}]}}",
        rows[1..].join(",")
    );
    let got = refused(
        server,
        &short,
        "",
        "driven_frames",
        "$.examples[0].driven.values",
    );
    assert_eq!(
        got.get("limit").and_then(Json::int),
        Some(((6 - 1) * STEPS) as i64)
    );
    let all = rows.join(",");
    refused(
        server,
        &format!("{{\"plane\":1,\"cells\":[[5,7,0]],\"values\":[{all}]}}"),
        "",
        "driven_written",
        "$.examples[0].driven.plane",
    );
    refused(
        server,
        &format!("{{\"plane\":3,\"cells\":[[5,7,0]],\"values\":[{all}]}}"),
        "",
        "outside",
        "$.examples[0].driven.plane",
    );
    refused(
        server,
        &format!("{{\"cells\":[[16,7,0]],\"values\":[{all}]}}"),
        "",
        "outside",
        "$.examples[0].driven.cells[0]",
    );
    refused(
        server,
        &format!("{{\"current\":true,\"cells\":[[5,7,0]],\"values\":[{all}]}}"),
        "",
        "malformed",
        "$.examples[0].driven.current",
    );
    let flat = |rule: &str| {
        format!(
            "{{\"stencil\":\"3x3\",\"reads\":[{{\"plane\":0,\"bit\":0}}],\"writes\":[{{\"plane\":1,\"bit\":0}}],\"border\":\"wrapped\",\"rule\":\"{rule}\",\"examples\":[{{\"size\":[{SIDE},{SIDE},1],\"input\":[{}],\"output\":[{}],\"driven\":{{\"cells\":[[5,7,0]],\"values\":[[1]]}}}}],\"budget_ms\":{LONG}}}",
            listed(&one.shots[0]),
            listed(&one.shots[1])
        )
    };
    for (rule, code) in [("linear", "steps_rule"), ("outer", "linear_rule")] {
        let got = no(server, "grit_law_learn", &flat(rule));
        assert_eq!(
            (text_at(&got, "refused"), text_at(&got, "field")),
            (code.to_owned(), "$.examples[0].driven".to_owned()),
            "{rule}: {}",
            written(&got)
        );
    }
}
