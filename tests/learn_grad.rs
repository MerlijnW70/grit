#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;
#[cfg(feature = "faults")]
mod delayed;
mod halo_grad;

use common::device;

use grit::law::Law;
use grit::law::adjoint::{self, Aim, Descent, Fault, Posed, Reversible, Settled, Tried, Walked};
use grit::question::Region;
use grit::sand3::{Grid3, reference};
use grit::serve3::Server;
use grit::serve3::gradient::{CELLS_MOST, FRAMES_MOST, STEPS_MOST, WORK_MOST};
use grit::wire::{Json, written};

const MIX: &str = include_str!("law/mix.law.json");
const HEAT: &str = include_str!("law/heat.law.json");

const MIX_TRUTH: [u32; 2] = [3_000, 1_700];

const CALM: u32 = 1 << 22;

const SIDE: u32 = 16;

const LONG: &str = "600000";

fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn below(state: &mut u64, most: u64) -> u64 {
    next(state) % most
}

fn word(state: &mut u64) -> u32 {
    match below(state, 6) {
        0 => 0,
        1 => u32::MAX,
        2 => u32::try_from(below(state, 16)).unwrap(),
        _ => u32::try_from(next(state) >> 32).unwrap(),
    }
}

fn quoted(text: &str) -> String {
    written(&Json::Text(text.to_owned()))
}

fn line(id: i64, name: &str, arguments: &str) -> String {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"tools/call\",\"params\":{{\"name\":\"{name}\",\"arguments\":{arguments}}}}}"
    )
}

fn call(server: &mut Server<'_>, name: &str, arguments: &str) -> Json {
    server.heard(&line(7, name, arguments)).unwrap()
}

fn result(got: &Json, error: bool) -> Json {
    let held = got
        .get("result")
        .unwrap_or_else(|| panic!("{}", written(got)));
    assert_eq!(
        held.get("isError"),
        Some(&Json::Bool(error)),
        "{}",
        written(got)
    );
    held.get("structuredContent").cloned().unwrap()
}

fn answer(got: &Json) -> Json {
    result(got, false)
}

fn refusal(got: &Json) -> Json {
    result(got, true)
}

fn int_at(held: &Json, key: &str) -> i64 {
    held.get(key)
        .and_then(Json::int)
        .unwrap_or_else(|| panic!("no {key} in {}", written(held)))
}

fn text_at(held: &Json, key: &str) -> String {
    held.get(key)
        .and_then(Json::text)
        .unwrap_or_else(|| panic!("no {key} in {}", written(held)))
        .to_owned()
}

fn whole_of(held: &Json) -> u64 {
    match held {
        Json::Int(one) => u64::try_from(*one).unwrap(),
        Json::Text(one) => one.parse().unwrap(),
        other => panic!("not a whole number: {}", written(other)),
    }
}

fn words_of(held: &Json) -> Vec<u32> {
    held.list()
        .unwrap()
        .iter()
        .map(|one| u32::try_from(one.int().unwrap()).unwrap())
        .collect()
}

fn listed(words: &[u32]) -> String {
    format!("{words:?}")
}

fn compiled(text: &str) -> Law {
    Law::compile(text).unwrap_or_else(|why| panic!("{why}\n{text}"))
}

fn reversible(text: &str) -> Reversible {
    Reversible::of(&compiled(text)).unwrap_or_else(|why| panic!("{why}"))
}

fn law_text(name: &str, params: u32, planes: u32, bindings: &[String], out: &[String]) -> String {
    format!(
        "{{\"law\":1,\"name\":\"{name}\",\"params\":{params},\"channels\":{planes},\"halo\":0,\"reversible\":true,\"tally\":{{\"counts\":1,\"weighted\":false}},\"consts\":{{}},\"defs\":{{}},\"step\":[\"let\",[{}],[\"vec\",[\"vec\",{}],[\"vec\",0]]]}}",
        bindings.join(","),
        out.join(",")
    )
}

struct Lifter {
    planes: usize,
    divides: bool,
    versions: Vec<u32>,
    bindings: Vec<String>,
}

impl Lifter {
    fn new() -> Self {
        Self::planed(1)
    }

    fn planed(planes: usize) -> Self {
        let cell = |k: usize| {
            if planes == 1 {
                format!("[\"at\",\"cells\",{k}]")
            } else {
                format!("[\"at\",[\"at\",\"cells\",{}],{}]", k % 8, k / 8)
            }
        };
        Self {
            planes,
            divides: true,
            versions: vec![0; 8 * planes],
            bindings: (0..8 * planes)
                .map(|k| format!("[\"w{k}v0\",{}]", cell(k)))
                .collect(),
        }
    }

    fn words(&self) -> usize {
        8 * self.planes
    }

    fn named(&self, word: usize) -> String {
        format!("\"w{word}v{}\"", self.versions[word])
    }

    fn other(&self, state: &mut u64, target: usize) -> String {
        let words = self.words();
        let pick = (target + 1 + usize::try_from(below(state, words as u64 - 1)).unwrap()) % words;
        self.named(pick)
    }

    fn lift(&mut self, target: usize, op: &str, by: &str) {
        let before = self.named(target);
        self.versions[target] += 1;
        let after = self.named(target);
        self.bindings
            .push(format!("[{after},[\"{op}\",{before},{by}]]"));
    }

    fn text(&self, name: &str, params: u32) -> String {
        let out: Vec<String> = (0..8)
            .map(|k| {
                if self.planes == 1 {
                    self.named(k)
                } else {
                    let planes: Vec<String> = (0..self.planes)
                        .map(|plane| self.named(plane * 8 + k))
                        .collect();
                    format!("[\"vec\",{}]", planes.join(","))
                }
            })
            .collect();
        law_text(name, params, self.planes as u32, &self.bindings, &out)
    }
}

fn smooth(seed: u64) -> (String, Vec<u32>) {
    let mut state = seed;
    let params = 1 + u32::try_from(below(&mut state, 3)).unwrap();
    let mut lifter = Lifter::new();
    let steps = 3 + below(&mut state, 4);
    let mut used = vec![false; params as usize];
    for step in 0..steps {
        let target = usize::try_from(below(&mut state, 8)).unwrap();
        let op = ["add", "sub"][usize::try_from(below(&mut state, 2)).unwrap()];
        let source = lifter.other(&mut state, target);
        let drop = 5 + below(&mut state, 3);
        let point = 10 + below(&mut state, 4);
        let param = if step < u64::from(params) {
            step
        } else {
            below(&mut state, u64::from(params))
        };
        used[usize::try_from(param).unwrap()] = true;
        let mut by =
            format!("[\"shr\",[\"mul\",[\"shr\",{source},{drop}],[\"param\",{param}]],{point}]");
        if below(&mut state, 2) == 0 {
            let more = lifter.other(&mut state, target);
            by = format!(
                "[\"add\",{by},[\"shr\",{more},{}]]",
                6 + below(&mut state, 3)
            );
        }
        lifter.lift(target, op, &by);
    }
    assert!(used.iter().all(|one| *one));
    let truth = (0..params)
        .map(|_| 1_000 + u32::try_from(below(&mut state, 3_000)).unwrap())
        .collect();
    (lifter.text(&format!("smooth{seed}"), params), truth)
}

fn expr(state: &mut u64, lifter: &Lifter, target: usize, params: u32, depth: u32) -> String {
    if depth == 0 || below(state, 3) == 0 {
        return match below(state, 5) {
            0 | 1 => lifter.other(state, target),
            2 if params > 0 => format!("[\"param\",{}]", below(state, u64::from(params))),
            3 => match below(state, 3) {
                0 => "\"frame\"".to_owned(),
                1 => format!("[\"at\",\"at\",{}]", below(state, 3)),
                _ => "[\"noise\"]".to_owned(),
            },
            _ => word(state).to_string(),
        };
    }
    let less = depth - 1;
    match below(state, 6) {
        0..=2 => {
            let ops = ["add", "sub", "mul", "xor", "and", "or", "div"];
            let kinds = if lifter.divides { 7 } else { 6 };
            let op = ops[usize::try_from(below(state, kinds)).unwrap()];
            let left = expr(state, lifter, target, params, less);
            let right = expr(state, lifter, target, params, less);
            format!("[\"{op}\",{left},{right}]")
        }
        3 => {
            let op = ["shl", "shr"][usize::try_from(below(state, 2)).unwrap()];
            let shifted = expr(state, lifter, target, params, less);
            format!("[\"{op}\",{shifted},{}]", below(state, 32))
        }
        4 => {
            let left = expr(state, lifter, target, params, less);
            let right = expr(state, lifter, target, params, less);
            let then = expr(state, lifter, target, params, less);
            let otherwise = expr(state, lifter, target, params, less);
            format!("[\"sel\",[\"lt\",{left},{right}],{then},{otherwise}]")
        }
        _ => format!("[\"not\",{}]", expr(state, lifter, target, params, less)),
    }
}

fn random_law(seed: u64) -> (String, Vec<u32>) {
    random_planed(seed, 1, true)
}

fn random_planed(seed: u64, planes: usize, divides: bool) -> (String, Vec<u32>) {
    let mut state = seed;
    let params = u32::try_from(below(&mut state, 4)).unwrap();
    let mut lifter = Lifter::planed(planes);
    lifter.divides = divides;
    for _ in 0..2 + below(&mut state, 6) {
        let target = usize::try_from(below(&mut state, lifter.words() as u64)).unwrap();
        let op = ["add", "sub", "xor"][usize::try_from(below(&mut state, 3)).unwrap()];
        let by = expr(&mut state, &lifter, target, params, 3);
        lifter.lift(target, op, &by);
    }
    let values = (0..params).map(|_| word(&mut state)).collect();
    (lifter.text(&format!("random{seed}"), params), values)
}

fn grid() -> Grid3 {
    Grid3::new(SIDE, SIDE, SIDE).unwrap()
}

fn calm(seed: u64) -> Vec<u32> {
    let mut state = seed;
    (0..grid().cells())
        .map(|_| CALM - (1 << 21) + u32::try_from(below(&mut state, 1 << 22)).unwrap())
        .collect()
}

fn ran(law: &Reversible, start: &[u32], params: &[u32], first: u32, frames: u32) -> Vec<u32> {
    let mut cells = start.to_vec();
    for frame in first..first + frames {
        cells = reference(law.forward(), grid(), frame, &cells, params);
    }
    cells
}

fn boxed(region: Region, cells: &[u32]) -> Vec<u32> {
    let mut out = Vec::new();
    for z in region.from[2]..region.to[2] {
        for y in region.from[1]..region.to[1] {
            for x in region.from[0]..region.to[0] {
                out.push(cells[grid().index(x, y, z)]);
            }
        }
    }
    out
}

fn random_region(state: &mut u64) -> Region {
    let mut from = [0; 3];
    let mut to = [0; 3];
    for axis in 0..3 {
        let one = u32::try_from(below(state, u64::from(SIDE))).unwrap();
        let other = u32::try_from(below(state, u64::from(SIDE))).unwrap();
        from[axis] = one.min(other);
        to[axis] = one.max(other) + 1;
    }
    Region::new(from, to)
}

struct Inline<'a> {
    text: &'a str,
    start: &'a [u32],
    first: u32,
    params: &'a [u32],
    frames: u32,
    region: Region,
    target: &'a [u32],
}

impl Inline<'_> {
    fn arguments(&self, more: &str) -> String {
        let last = self.region.to.map(|one| one - 1);
        format!(
            "{{\"law\":{},\"size\":[{SIDE},{SIDE},{SIDE}],\"cells\":{},\"first\":{},\"params\":{},\"frames\":{},\"aim\":{{\"from\":{:?},\"to\":{last:?},\"target\":{}}},\"budget_ms\":{LONG}{more}}}",
            quoted(self.text),
            listed(self.start),
            self.first,
            listed(self.params),
            self.frames,
            self.region.from,
            listed(self.target)
        )
    }

    fn aim(&self) -> Aim<'_> {
        Aim {
            region: self.region,
            target: self.target,
        }
    }

    fn walked(&self, law: &Reversible) -> Result<Walked, Fault> {
        law.walk(
            grid(),
            self.start,
            self.params,
            self.first,
            self.frames,
            &self.aim(),
        )
    }

    fn descent(&self, law: &Reversible, most: u32) -> Result<Descent, Fault> {
        law.descend(
            &Posed {
                grid: grid(),
                start: self.start,
                first: self.first,
                frames: self.frames,
                aim: self.aim(),
            },
            self.params,
            most,
        )
    }
}

fn gradient_of(got: &Json) -> Vec<i128> {
    got.get("gradient")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .map(|one| one.text().unwrap().parse().unwrap())
        .collect()
}

fn trail_of(got: &Json) -> Vec<Tried> {
    got.get("trail")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .map(|one| Tried {
            params: words_of(one.get("params").unwrap()),
            objective: whole_of(one.get("objective").unwrap()),
            kept: one.get("kept") == Some(&Json::Bool(true)),
        })
        .collect()
}

fn best_of(trail: &[Tried]) -> (Vec<u32>, u64) {
    let lowest = trail.iter().map(|tried| tried.objective).min().unwrap();
    let best = trail
        .iter()
        .find(|tried| tried.objective == lowest)
        .unwrap();
    (best.params.clone(), best.objective)
}

const fn settled_name(settled: Option<Settled>) -> &'static str {
    match settled {
        Some(Settled::Converged) => "converged",
        Some(Settled::Local) => "local",
        Some(Settled::Steps) => "steps",
        None => "none",
    }
}

fn fault_code(why: Fault) -> &'static str {
    match why {
        Fault::Overflow => "overflow",
        Fault::Strayed { .. } => "strayed",
        other => panic!("a fault the tool refuses first: {other}"),
    }
}

const CUT: [&str; 6] = ["not", "and", "or", "xor", "dot_s", "dot_u"];

fn located(json: &Json, path: &str) -> Json {
    let mut here = json.clone();
    let mut rest = path.strip_prefix('$').unwrap();
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('.') {
            let end = after.find(['.', '[']).unwrap_or(after.len());
            here = here.get(&after[..end]).unwrap().clone();
            rest = &after[end..];
        } else {
            let after = rest.strip_prefix('[').unwrap();
            let end = after.find(']').unwrap();
            let index: usize = after[..end].parse().unwrap();
            here = here.list().unwrap()[index].clone();
            rest = &after[end + 1..];
        }
    }
    here
}

fn cut_refused(label: &str, got: &Json, text: &str) -> bool {
    let held = got.get("result").unwrap();
    if held.get("isError") != Some(&Json::Bool(true)) {
        return false;
    }
    let told = refusal(got);
    if text_at(&told, "code") != "no_gradient" {
        return false;
    }
    let law = told.get("law").unwrap();
    let found = text_at(law, "found");
    let node = located(&grit::wire::read(text).unwrap(), &text_at(law, "path"));
    let op = node.list().and_then(|one| one.first()).and_then(Json::text);
    assert!(
        CUT.contains(&found.as_str()) && op == Some(found.as_str()),
        "{label}: refused at {} ({op:?}) for {found}",
        text_at(law, "path")
    );
    true
}

fn same_gradient(label: &str, got: &Json, walked: &Result<Walked, Fault>) {
    match walked {
        Ok(walked) => {
            let got = answer(got);
            assert_eq!(
                whole_of(got.get("objective").unwrap()),
                walked.objective,
                "{label}: objective"
            );
            assert_eq!(gradient_of(&got), walked.told, "{label}: gradient");
            assert_eq!(int_at(&got, "fraction"), i64::from(adjoint::FRACTION));
            assert_eq!(got.get("home"), Some(&Json::Bool(true)), "{label}");
        }
        Err(why) => {
            let told = refusal(got);
            assert_eq!(text_at(&told, "code"), fault_code(*why), "{label}");
        }
    }
}

fn same_descent(label: &str, got: &Json, descent: &Result<Descent, Fault>, most: u32) {
    match descent {
        Ok(descent) => {
            let got = answer(got);
            let trail = trail_of(&got);
            assert_eq!(trail, descent.trail, "{label}: trail");
            assert_eq!(
                int_at(&got, "steps"),
                i64::from(descent.steps),
                "{label}: steps"
            );
            assert!(descent.steps <= most, "{label}: steps");
            let params = words_of(got.get("params").unwrap());
            let objective = whole_of(got.get("objective").unwrap());
            assert_eq!(
                (params.clone(), objective),
                (descent.params.clone(), descent.objective),
                "{label}: the library's answer"
            );
            assert_eq!((params, objective), best_of(&trail), "{label}: best");
            assert_eq!(
                text_at(&got, "stopped"),
                settled_name(descent.settled),
                "{label}: stopped"
            );
        }
        Err(why) => {
            let told = refusal(got);
            assert_eq!(text_at(&told, "code"), fault_code(*why), "{label}");
        }
    }
}

const RANDOM_LAWS: u64 = 48;

#[test]
fn g1_the_gradient_through_the_mcp_is_the_walk_exactly_on_random_laws_and_aims() {
    let mut server = Server::new(None);
    let mut overflowed = 0;
    let mut moved = 0;
    let mut cut = 0;
    for seed in 0..RANDOM_LAWS {
        let (text, values) = if seed % 2 == 0 {
            random_law(0xA1B0 + seed)
        } else {
            smooth(0xA1B0 + seed)
        };
        let law = reversible(&text);
        let mut state = seed.wrapping_mul(0x51ED);
        let start: Vec<u32> = if seed % 2 == 0 {
            (0..grid().cells()).map(|_| word(&mut state)).collect()
        } else {
            calm(seed)
        };
        let first = u32::try_from(below(&mut state, 5)).unwrap();
        let frames = 1 + u32::try_from(below(&mut state, 6)).unwrap();
        let region = random_region(&mut state);
        let ended = ran(&law, &start, &values, first, frames + 1);
        let target = boxed(region, &ended);
        let posed = Inline {
            text: &text,
            start: &start,
            first,
            params: &values,
            frames,
            region,
            target: &target,
        };
        let walked = posed.walked(&law);
        let most = 1 + u32::try_from(below(&mut state, 4)).unwrap();
        let got = call(&mut server, "grit_law_gradient", &posed.arguments(""));
        if cut_refused(&format!("law {seed}"), &got, &text) {
            let again = call(
                &mut server,
                "grit_law_descend",
                &posed.arguments(&format!(",\"steps\":{most}")),
            );
            assert!(cut_refused(&format!("law {seed}"), &again, &text));
            cut += 1;
            continue;
        }
        match &walked {
            Err(_) => overflowed += 1,
            Ok(walked) if walked.told.iter().any(|one| *one != 0) => moved += 1,
            Ok(_) => {}
        }
        same_gradient(&format!("law {seed}"), &got, &walked);
        let got = call(
            &mut server,
            "grit_law_descend",
            &posed.arguments(&format!(",\"steps\":{most}")),
        );
        same_descent(
            &format!("law {seed}"),
            &got,
            &posed.descent(&law, most),
            most,
        );
    }
    println!(
        "{RANDOM_LAWS} random laws and aims: {cut} refused no_gradient at a cut node, every other gradient and descent equal to the library; {moved} with a gradient that is not zero, {overflowed} past 128 bits"
    );
    assert!(moved >= RANDOM_LAWS / 3, "{moved}");
    assert!(cut >= RANDOM_LAWS / 8, "{cut}");
}

const STEP: u32 = 4;

const TOLERANCE_PERCENT: f64 = 1.0;

const SMOOTH_LAWS: u64 = 16;

const SMOOTH_FRAMES: u32 = 16;

const STIR: &str = include_str!("law/stir.law.json");

const STIR_TRUTH: [u32; 3] = [2_600, 900, 1_500];

const LOW: u32 = 1 << 20;

const HIGH: u32 = 1 << 26;

fn objective_through(server: &mut Server<'_>, posed: &Inline<'_>, params: &[u32]) -> u64 {
    let moved = Inline { params, ..*posed };
    let got = answer(&call(server, "grit_law_gradient", &moved.arguments("")));
    whole_of(got.get("objective").unwrap())
}

fn agrees(server: &mut Server<'_>, label: &str, law: &Reversible, posed: &Inline<'_>) -> f64 {
    let got = answer(&call(server, "grit_law_gradient", &posed.arguments("")));
    let told = gradient_of(&got);
    let away = whole_of(got.get("objective").unwrap());
    assert!(away > 0, "{label}");
    let exact = |params: &[u32]| {
        posed.aim().measured(
            grid(),
            &ran(law, posed.start, params, posed.first, posed.frames),
        )
    };
    assert_eq!(away, exact(posed.params), "{label}: the forward run");
    let mut worst = 0.0_f64;
    for (index, &slope) in told.iter().enumerate() {
        let mut up = posed.params.to_vec();
        up[index] += STEP;
        let mut down = posed.params.to_vec();
        down[index] -= STEP;
        let above = objective_through(server, posed, &up);
        let under = objective_through(server, posed, &down);
        assert_eq!(above, exact(&up), "{label}: the forward run");
        assert_eq!(under, exact(&down), "{label}: the forward run");
        let rise = i128::from(above) - i128::from(under);
        let differenced = rise as f64 / f64::from(2 * STEP);
        let slope = slope as f64 / 2_f64.powi(adjoint::FRACTION as i32);
        let off = 100.0 * (slope - differenced).abs() / differenced.abs();
        println!(
            "{label} params {:?} param {index}: adjoint {slope:.2}, finite difference {differenced:.2}, {off:.3} % apart",
            posed.params
        );
        assert_ne!(rise, 0, "{label} param {index}: flat");
        assert_eq!(
            slope.is_sign_positive(),
            differenced.is_sign_positive(),
            "{label} param {index}: sign"
        );
        assert!(
            off <= TOLERANCE_PERCENT,
            "{label} param {index}: {off} % apart\n{}",
            posed.text
        );
        worst = worst.max(off);
    }
    worst
}

fn smooth_far(seed: u64) -> (String, Vec<u32>) {
    let mut state = seed;
    let params = 1 + u32::try_from(below(&mut state, 3)).unwrap();
    let mut lifter = Lifter::new();
    let steps = 3 + below(&mut state, 4);
    for step in 0..steps {
        let target = usize::try_from(below(&mut state, 8)).unwrap();
        let op = ["add", "sub"][usize::try_from(below(&mut state, 2)).unwrap()];
        let source = lifter.other(&mut state, target);
        let drop = 7 + below(&mut state, 3);
        let point = 12 + below(&mut state, 3);
        let param = if step < u64::from(params) {
            step
        } else {
            below(&mut state, u64::from(params))
        };
        let mut by =
            format!("[\"shr\",[\"mul\",[\"shr\",{source},{drop}],[\"param\",{param}]],{point}]");
        if below(&mut state, 2) == 0 {
            let more = lifter.other(&mut state, target);
            by = format!(
                "[\"add\",{by},[\"shr\",{more},{}]]",
                8 + below(&mut state, 3)
            );
        }
        lifter.lift(target, op, &by);
    }
    let values = (0..params)
        .map(|_| 1_000 + u32::try_from(below(&mut state, 3_000)).unwrap())
        .collect();
    (lifter.text(&format!("far{seed}"), params), values)
}

fn far_start(seed: u64) -> Vec<u32> {
    let mut state = seed;
    (0..grid().cells())
        .map(|_| (1 << 24) - (1 << 22) + u32::try_from(below(&mut state, 1 << 23)).unwrap())
        .collect()
}

fn inside_band(label: &str, law: &Reversible, start: &[u32], params: &[u32], high: u32) {
    let mut cells = start.to_vec();
    for frame in 0..=SMOOTH_FRAMES {
        assert!(
            cells.iter().all(|one| (LOW..high).contains(one)),
            "{label}: a cell left {LOW}..{high} by frame {frame}, so the case can wrap or meet its target"
        );
        cells = reference(law.forward(), grid(), frame, &cells, params);
    }
}

fn far_target(state: &mut u64, region: Region) -> Vec<u32> {
    let under = below(state, 2) == 0;
    (0..region.cells())
        .map(|_| {
            if under {
                u32::try_from(below(state, u64::from(LOW))).unwrap()
            } else {
                HIGH + u32::try_from(below(state, u64::from(u32::MAX - HIGH))).unwrap()
            }
        })
        .collect()
}

const MIX_HIGH: u32 = 1 << 24;

type Smooth = (String, String, Vec<u32>, Vec<u32>, u32);

#[test]
fn g2_the_gradient_through_the_mcp_agrees_with_finite_differences_in_sign_and_to_one_percent() {
    let mut server = Server::new(None);
    let mut cases: Vec<Smooth> = Vec::new();
    for (name, text, truth) in [
        ("mix", MIX, MIX_TRUTH.to_vec()),
        ("stir", STIR, STIR_TRUTH.to_vec()),
    ] {
        for seed in [11_u64, 12] {
            cases.push((
                format!("{name} seed {seed}"),
                text.to_owned(),
                truth.clone(),
                calm(seed),
                MIX_HIGH,
            ));
        }
    }
    for seed in 0..SMOOTH_LAWS {
        let (text, values) = smooth_far(0x5300 + seed);
        cases.push((
            format!("far law {seed}"),
            text,
            values,
            far_start(0x77 + seed),
            HIGH,
        ));
    }
    for (label, text, values) in op_laws() {
        cases.push((label, text, values, far_start(0x0B), HIGH));
    }
    let mut worst = 0.0_f64;
    let mut checked = 0;
    for (case, (label, text, values, start, high)) in cases.iter().enumerate() {
        let law = reversible(text);
        let mut state = (case as u64).wrapping_mul(0xFA2) ^ 0x3D;
        let region = Region::whole([SIDE, SIDE, SIDE]);
        let target = far_target(&mut state, region);
        for scale in [115_u32, 85] {
            let params: Vec<u32> = values.iter().map(|one| one * scale / 100).collect();
            for moved in [i64::from(STEP), 0, -i64::from(STEP)] {
                let shifted: Vec<u32> = params
                    .iter()
                    .map(|one| u32::try_from(i64::from(*one) + moved).unwrap())
                    .collect();
                inside_band(label, &law, start, &shifted, *high);
            }
            let posed = Inline {
                text,
                start,
                first: 0,
                params: &params,
                frames: SMOOTH_FRAMES,
                region,
                target: &target,
            };
            worst = worst.max(agrees(&mut server, label, &law, &posed));
            checked += params.len();
        }
    }
    println!(
        "mix, stir, {SMOOTH_LAWS} random smooth laws and {} laws of one op each, whole boxes of targets all past one side, {checked} params: the gradient through the MCP is at most {worst:.3} % from the central finite difference of step {STEP}",
        op_laws().len()
    );
    let mut refused = 0;
    for (label, text) in cut_laws() {
        let law = reversible(&text);
        let start = far_start(0x0C);
        let mut state = 0xC07;
        let region = Region::whole([SIDE, SIDE, SIDE]);
        let target = far_target(&mut state, region);
        let params = [3_000_u32];
        let posed = Inline {
            text: &text,
            start: &start,
            first: 0,
            params: &params,
            frames: SMOOTH_FRAMES,
            region,
            target: &target,
        };
        let got = call(&mut server, "grit_law_gradient", &posed.arguments(""));
        if cut_refused(&label, &got, &text) {
            refused += 1;
            continue;
        }
        for moved in [0_u32, STEP, 2 * STEP] {
            inside_band(&label, &law, &start, &[params[0] + moved - STEP], HIGH);
        }
        agrees(&mut server, &label, &law, &posed);
    }
    println!(
        "{} laws with a cut op on a param's path: {refused} refused no_gradient, the rest agree with finite differences",
        cut_laws().len()
    );
}

const SHARE: &str = "[\"shr\",[\"mul\",[\"shr\",\"w0v0\",7],[\"param\",0]],13]";

fn one_lift(name: &str, params: u32, lifts: &[(usize, &str, String)]) -> String {
    let mut lifter = Lifter::new();
    for (word, op, by) in lifts {
        lifter.lift(*word, op, by);
    }
    lifter.text(name, params)
}

fn op_laws() -> Vec<(String, String, Vec<u32>)> {
    let shared = |by: &str| {
        vec![
            (1, "add", by.to_owned()),
            (
                2,
                "add",
                "[\"shr\",[\"mul\",[\"shr\",\"w4v0\",7],[\"param\",0]],12]".to_owned(),
            ),
        ]
    };
    vec![
        (
            "shl".to_owned(),
            one_lift(
                "shl",
                1,
                &shared("[\"shl\",[\"shr\",[\"mul\",[\"shr\",\"w0v0\",7],[\"param\",0]],16],3]"),
            ),
            vec![3_000],
        ),
        (
            "div by a constant".to_owned(),
            one_lift(
                "divided",
                1,
                &shared("[\"div\",[\"mul\",[\"shr\",\"w0v0\",7],[\"param\",0]],9000]"),
            ),
            vec![3_000],
        ),
        (
            "div by a param".to_owned(),
            one_lift(
                "over",
                1,
                &shared("[\"div\",[\"mul\",[\"shr\",\"w0v0\",7],2000],[\"param\",0]]"),
            ),
            vec![3_000],
        ),
        (
            "mul of two params".to_owned(),
            one_lift(
                "squared",
                2,
                &[(
                    1,
                    "add",
                    "[\"shr\",[\"mul\",[\"shr\",\"w0v0\",12],[\"mul\",[\"param\",0],[\"param\",1]]],12]".to_owned(),
                )],
            ),
            vec![60, 50],
        ),
        (
            "sel arms on at".to_owned(),
            one_lift(
                "chosen",
                2,
                &[(
                    1,
                    "add",
                    "[\"sel\",[\"lt\",[\"at\",\"at\",0],4],[\"shr\",[\"mul\",[\"shr\",\"w0v0\",7],[\"param\",0]],13],[\"shr\",[\"mul\",[\"shr\",\"w2v0\",7],[\"param\",1]],12]]".to_owned(),
                )],
            ),
            vec![3_000, 2_000],
        ),
        (
            "a compare of a param".to_owned(),
            one_lift(
                "gated",
                1,
                &[(
                    1,
                    "add",
                    format!("[\"sel\",[\"gt\",[\"param\",0],100],{SHARE},0]"),
                )],
            ),
            vec![3_000],
        ),
        (
            "at, frame and noise beside a param".to_owned(),
            one_lift(
                "placed",
                1,
                &[(
                    1,
                    "add",
                    "[\"shr\",[\"mul\",[\"add\",[\"add\",[\"shr\",\"w0v0\",7],[\"shl\",[\"at\",\"at\",0],10]],[\"add\",[\"shl\",\"frame\",8],[\"shr\",[\"noise\"],18]]],[\"param\",0]],13]".to_owned(),
                )],
            ),
            vec![3_000],
        ),
    ]
}

fn cut_laws() -> Vec<(String, String)> {
    vec![
        (
            "sub of not".to_owned(),
            one_lift("notted", 1, &[(1, "sub", format!("[\"not\",{SHARE}]"))]),
        ),
        (
            "xor 1 after the add".to_owned(),
            one_lift(
                "xored",
                1,
                &[(1, "add", SHARE.to_owned()), (1, "xor", "1".to_owned())],
            ),
        ),
        (
            "and with a high mask".to_owned(),
            one_lift(
                "anded",
                1,
                &[(1, "add", format!("[\"and\",{SHARE},4294967040]"))],
            ),
        ),
        (
            "or with a low bit".to_owned(),
            one_lift("ored", 1, &[(1, "add", format!("[\"or\",{SHARE},1]"))]),
        ),
        (
            "dot_u lane 0".to_owned(),
            one_lift(
                "dotted",
                1,
                &[(
                    1,
                    "add",
                    "[\"dot_u\",[\"shr\",[\"mul\",[\"shr\",\"w0v0\",14],[\"param\",0]],12],200]"
                        .to_owned(),
                )],
            ),
        ),
        (
            "dot_s lane 0".to_owned(),
            one_lift(
                "signed",
                1,
                &[(
                    1,
                    "add",
                    "[\"dot_s\",[\"shr\",[\"mul\",[\"shr\",\"w0v0\",14],[\"param\",0]],12],100]"
                        .to_owned(),
                )],
            ),
        ),
        (
            "xor of a cell a param moved a frame before".to_owned(),
            one_lift(
                "later",
                1,
                &[
                    (1, "add", SHARE.to_owned()),
                    (3, "add", "[\"shr\",[\"xor\",\"w6v0\",5],8]".to_owned()),
                ],
            ),
        ),
    ]
}

trait Ask {
    fn ask(&mut self, name: &str, arguments: &str) -> Json;
}

impl Ask for Server<'_> {
    fn ask(&mut self, name: &str, arguments: &str) -> Json {
        call(self, name, arguments)
    }
}

fn create(asker: &mut dyn Ask, text: &str, params: &[u32], side: u32) -> i64 {
    created(asker, text, params, side, false)
}

fn created(asker: &mut dyn Ask, text: &str, params: &[u32], side: u32, sparse: bool) -> i64 {
    let made = answer(&asker.ask(
        "grit_world_create",
        &format!(
            "{{\"wide\":{side},\"deep\":{side},\"tall\":{side},\"law\":{},\"params\":{},\"sparse\":{sparse}}}",
            quoted(text),
            listed(params)
        ),
    ));
    int_at(&made, "world")
}

fn write_all(asker: &mut dyn Ask, world: i64, cells: &[u32]) {
    let mut placed = Vec::new();
    for z in 0..SIDE {
        for y in 0..SIDE {
            for x in 0..SIDE {
                placed.push(format!("[{x},{y},{z},{}]", cells[grid().index(x, y, z)]));
            }
        }
    }
    for chunk in placed.chunks(4096) {
        answer(&asker.ask(
            "grit_world_write",
            &format!(
                "{{\"world\":{world},\"cells\":[{}],\"budget_ms\":{LONG}}}",
                chunk.join(",")
            ),
        ));
    }
}

fn step(asker: &mut dyn Ask, world: i64, frames: u32) {
    answer(&asker.ask(
        "grit_world_step",
        &format!("{{\"world\":{world},\"frames\":{frames},\"budget_ms\":{LONG}}}"),
    ));
}

fn state(asker: &mut dyn Ask, world: i64, planes: u32) -> (i64, Vec<u32>) {
    let last = SIDE - 1;
    let mut cells = Vec::new();
    let mut frame = 0;
    for plane in 0..planes {
        let got = answer(&asker.ask(
            "grit_world_read",
            &format!(
                "{{\"world\":{world},\"from\":[0,0,0],\"to\":[{last},{last},{last}],\"plane\":{plane}}}"
            ),
        ));
        frame = int_at(&got, "frame");
        cells.extend(words_of(got.get("cells").unwrap()));
    }
    (frame, cells)
}

fn frame_only(asker: &mut dyn Ask, world: i64) -> i64 {
    let got = answer(&asker.ask(
        "grit_world_read",
        &format!("{{\"world\":{world},\"from\":[0,0,0],\"to\":[0,0,0]}}"),
    ));
    int_at(&got, "frame")
}

fn aimed_at(region: Region, target: &str) -> String {
    format!(
        "{{\"from\":{:?},\"to\":{:?},{target}}}",
        region.from,
        region.to.map(|one| one - 1)
    )
}

#[test]
fn g1_a_world_start_and_a_target_world_answer_the_walk_exactly() {
    let Some(host) = device("learn_grad world start") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let mut refused = 0;
    let mut accepted = 0;
    for seed in 0..6_u64 {
        let planes = 1 + usize::try_from(seed % 2).unwrap();
        let (text, values) = if seed < 4 {
            random_planed(0xD0 + seed, planes, false)
        } else {
            smooth(0xD0 + seed)
        };
        let planes = compiled(&text).channels();
        let law = reversible(&text);
        let mut state_of = seed ^ 0xBEEF;
        let written: Vec<u32> = if seed < 4 {
            (0..grid().cells()).map(|_| word(&mut state_of)).collect()
        } else {
            calm(seed)
        };
        let ahead = 3;
        let frames = 1 + u32::try_from(below(&mut state_of, 5)).unwrap();
        let sparse = seed >= 4;
        let start_world = created(&mut server, &text, &values, SIDE, sparse);
        write_all(&mut server, start_world, &written);
        step(&mut server, start_world, ahead);
        let other: Vec<u32> = values.iter().map(|one| one ^ 0x55).collect();
        let target_world = create(&mut server, &text, &other, SIDE);
        write_all(&mut server, target_world, &written);
        step(&mut server, target_world, ahead + frames);
        let (frame, start) = state(&mut server, start_world, planes);
        assert_eq!(frame, i64::from(ahead));
        let (_, aimed) = state(&mut server, target_world, 1);
        let region = random_region(&mut state_of);
        let target = boxed(region, &aimed);
        let aim = Aim {
            region,
            target: &target,
        };
        let walked = law.walk(grid(), &start, &values, ahead, frames, &aim);
        let aim_text = aimed_at(region, &format!("\"world\":{target_world}"));
        let got = server.ask(
            "grit_law_gradient",
            &format!(
                "{{\"world\":{start_world},\"frames\":{frames},\"aim\":{aim_text},\"budget_ms\":{LONG}}}"
            ),
        );
        if cut_refused(&format!("world law {seed}"), &got, &text) {
            refused += 1;
            println!("world law {seed}: refused no_gradient");
            continue;
        }
        same_gradient(&format!("world law {seed}"), &got, &walked);
        let most = 3;
        let descent = law.descend(
            &Posed {
                grid: grid(),
                start: &start,
                first: ahead,
                frames,
                aim,
            },
            &values,
            most,
        );
        let got = server.ask(
            "grit_law_descend",
            &format!(
                "{{\"world\":{start_world},\"params\":{},\"frames\":{frames},\"aim\":{aim_text},\"steps\":{most},\"budget_ms\":{LONG}}}",
                listed(&values)
            ),
        );
        same_descent(&format!("world law {seed}"), &got, &descent, most);
        assert_eq!(
            state(&mut server, start_world, planes),
            (i64::from(ahead), start.clone()),
            "world law {seed}: the start world is untouched"
        );
        if let Ok(walked) = walked {
            step(&mut server, start_world, frames);
            let (_, ended) = state(&mut server, start_world, planes);
            assert_eq!(
                ended, walked.ended,
                "world law {seed}: the walk's forward run is the world's own"
            );
        }
        accepted += 1;
        println!(
            "world law {seed} on {planes} planes, sparse {sparse}: gradient and descent equal the library"
        );
    }
    println!("{accepted} world laws answered, {refused} refused no_gradient");
    assert!(accepted >= 4, "{accepted}");
}

fn walks() -> Option<u64> {
    #[cfg(feature = "faults")]
    {
        Some(grit::serve3::gradient::faults::walks())
    }
    #[cfg(not(feature = "faults"))]
    {
        None
    }
}

struct Refused<'a> {
    arguments: String,
    code: &'a str,
    field: &'a str,
}

const fn refused_as<'a>(arguments: String, code: &'a str, field: &'a str) -> Refused<'a> {
    Refused {
        arguments,
        code,
        field,
    }
}

fn inline(text: &str, size: &str, cells: &[u32], params: &str, aim: &str) -> String {
    format!(
        "{{\"law\":{},\"size\":{size},\"cells\":{},\"params\":{params},\"frames\":2,\"aim\":{aim}}}",
        quoted(text),
        listed(cells)
    )
}

#[test]
fn g3_every_refusal_names_its_code_and_path_before_any_work() {
    let Some(host) = device("learn_grad refusals") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let mix = create(&mut server, MIX, &MIX_TRUTH, SIDE);
    write_all(&mut server, mix, &calm(1));
    step(&mut server, mix, 2);
    let sand = int_at(
        &answer(&server.ask(
            "grit_world_create",
            &format!("{{\"wide\":{SIDE},\"deep\":{SIDE},\"tall\":{SIDE},\"rule\":\"sand\"}}"),
        )),
        "world",
    );
    let heat = create(&mut server, HEAT, &[300, 4, 7], SIDE);
    let looks = create(&mut server, &cut_looker(), &[3], SIDE);
    let notted = cut_laws().remove(0).1;
    let cutter = create(&mut server, &notted, &[3_000], SIDE);
    let small = create(&mut server, MIX, &MIX_TRUTH, 8);
    let wide = create(&mut server, MIX, &MIX_TRUTH, 32);
    let large = int_at(
        &answer(&server.ask(
            "grit_world_create",
            &format!(
                "{{\"wide\":64,\"deep\":64,\"tall\":16,\"law\":{},\"params\":[1,2]}}",
                quoted(MIX)
            ),
        )),
        "world",
    );
    let before = state(&mut server, mix, 1);
    let worlds = [sand, heat, looks, cutter, small, wide, large];
    let frames_before: Vec<i64> = worlds
        .iter()
        .map(|world| frame_only(&mut server, *world))
        .collect();
    let walked = walks();
    let aim = "{\"from\":[0,0,0],\"to\":[1,1,1],\"target\":[1,2,3,4,5,6,7,8]}";
    let tiny = vec![CALM; 512];
    let marked_heat = HEAT.replacen("\"halo\": 0,", "\"halo\": 0, \"reversible\": true,", 1);
    assert_ne!(marked_heat, HEAT);
    let unmarked_mix = MIX.replacen("\"reversible\": true,", "", 1);
    assert_ne!(unmarked_mix, MIX);
    let looking = cut_looker();
    let boxed_at = |from: &str, to: &str, rest: &str| {
        format!("{{\"world\":{mix},\"frames\":2,\"aim\":{{\"from\":{from},\"to\":{to}{rest}}}}}")
    };
    let cases = [
        refused_as(
            format!("{{\"world\":{sand},\"frames\":2,\"aim\":{aim}}}"),
            "not_reversible",
            "$.world",
        ),
        refused_as(
            format!("{{\"world\":{heat},\"frames\":2,\"aim\":{aim}}}"),
            "not_reversible",
            "$.world",
        ),
        refused_as(
            inline(&marked_heat, "[8,8,8]", &tiny, "[1,2,3]", aim),
            "not_reversible",
            "$.law",
        ),
        refused_as(
            inline(&unmarked_mix, "[8,8,8]", &tiny, "[1,2]", aim),
            "not_reversible",
            "$.law",
        ),
        refused_as(
            format!("{{\"world\":{looks},\"frames\":2,\"aim\":{aim}}}"),
            "no_gradient",
            "$.world",
        ),
        refused_as(
            inline(&looking, "[8,8,8]", &[CALM; 1024], "[3]", aim),
            "no_gradient",
            "$.law",
        ),
        refused_as(
            format!("{{\"world\":{cutter},\"frames\":2,\"aim\":{aim}}}"),
            "no_gradient",
            "$.world",
        ),
        refused_as(
            inline(&notted, "[8,8,8]", &tiny, "[3000]", aim),
            "no_gradient",
            "$.law",
        ),
        refused_as(
            format!("{{\"world\":{mix},\"params\":[1],\"frames\":2,\"aim\":{aim}}}"),
            "params",
            "$.params",
        ),
        refused_as(
            inline(MIX, "[8,8,8]", &tiny, "[1,2,3]", aim),
            "params",
            "$.params",
        ),
        refused_as(
            boxed_at("[16,0,0]", "[16,0,0]", ",\"target\":[1]"),
            "outside",
            "$.aim.from",
        ),
        refused_as(
            boxed_at("[0,0,0]", "[0,16,0]", ",\"target\":[1]"),
            "outside",
            "$.aim.to",
        ),
        refused_as(
            boxed_at("[3,0,0]", "[2,5,5]", ",\"target\":[1]"),
            "reversed",
            "$.aim.to",
        ),
        refused_as(
            boxed_at("[0,0,0]", "[1,1,1]", ",\"target\":[1,2,3,4,5,6,7]"),
            "target_size",
            "$.aim.target",
        ),
        refused_as(
            boxed_at("[0,0,0]", "[15,15,15]", &format!(",\"world\":{small}")),
            "outside",
            "$.aim.world",
        ),
        refused_as(
            boxed_at("[0,0,0]", "[0,0,0]", ",\"world\":999"),
            "no_world",
            "$.aim.world",
        ),
        refused_as(
            boxed_at(
                "[0,0,0]",
                "[0,0,0]",
                &format!(",\"world\":{small},\"target\":[1]"),
            ),
            "aim",
            "$.aim",
        ),
        refused_as(boxed_at("[0,0,0]", "[0,0,0]", ""), "aim", "$.aim"),
        refused_as(
            format!("{{\"world\":{large},\"frames\":2,\"aim\":{aim}}}"),
            "walk_limit",
            "$.world",
        ),
        refused_as(
            format!("{{\"world\":{wide},\"frames\":{FRAMES_MOST},\"aim\":{aim}}}"),
            "walk_limit",
            "$.frames",
        ),
        refused_as(
            format!("{{\"world\":{mix},\"frames\":0,\"aim\":{aim}}}"),
            "frames",
            "$.frames",
        ),
        refused_as(
            format!(
                "{{\"world\":{mix},\"frames\":{},\"aim\":{aim}}}",
                FRAMES_MOST + 1
            ),
            "frames",
            "$.frames",
        ),
        refused_as(
            format!(
                "{{\"world\":{mix},\"law\":{},\"frames\":2,\"aim\":{aim}}}",
                quoted(MIX)
            ),
            "start",
            "$",
        ),
        refused_as(format!("{{\"frames\":2,\"aim\":{aim}}}"), "start", "$"),
        refused_as(
            format!("{{\"world\":{mix},\"first\":3,\"frames\":2,\"aim\":{aim}}}"),
            "start",
            "$.first",
        ),
        refused_as(
            format!("{{\"world\":{mix},\"size\":[8,8,8],\"frames\":2,\"aim\":{aim}}}"),
            "start",
            "$.size",
        ),
        refused_as(
            inline(MIX, "[8,8,8]", &tiny[1..], "[1,2]", aim),
            "start",
            "$.cells",
        ),
        refused_as(
            inline(MIX, "[6,8,8]", &tiny, "[1,2]", aim),
            "start",
            "$.size",
        ),
        refused_as(
            format!("{{\"world\":999,\"frames\":2,\"aim\":{aim}}}"),
            "no_world",
            "$.world",
        ),
        refused_as(
            format!("{{\"world\":{mix},\"frames\":2,\"aim\":{aim},\"fraems\":2}}"),
            "unknown_key",
            "$.fraems",
        ),
    ];
    let mut seen = 0;
    for tool in ["grit_law_gradient", "grit_law_descend"] {
        for case in &cases {
            let arguments = if tool == "grit_law_descend" {
                case.arguments.replacen('{', "{\"steps\":4,", 1)
            } else {
                case.arguments.clone()
            };
            let told = refusal(&server.ask(tool, &arguments));
            assert_eq!(
                (
                    text_at(&told, "code").as_str(),
                    text_at(&told, "field").as_str()
                ),
                (case.code, case.field),
                "{tool} {}: {}",
                case.arguments,
                written(&told)
            );
            if case.arguments.contains(&format!("\"world\":{cutter},"))
                || case.code == "no_gradient" && case.arguments.contains("notted")
            {
                let law = told.get("law").unwrap();
                assert_eq!(text_at(law, "found"), "not", "{}", written(&told));
                let node = located(&grit::wire::read(&notted).unwrap(), &text_at(law, "path"));
                assert_eq!(node.list().unwrap()[0].text(), Some("not"));
            }
            if case.code == "not_reversible" && case.arguments.contains("\"law\"")
                || case.arguments.contains(&format!("\"world\":{heat},"))
            {
                let law = told.get("law").unwrap();
                assert!(
                    ["not_lifting", "missing"].contains(&text_at(law, "code").as_str())
                        && text_at(law, "path").starts_with("$."),
                    "{}",
                    written(&told)
                );
            }
            seen += 1;
        }
    }
    for steps in [0, STEPS_MOST + 1] {
        let told = refusal(&server.ask(
            "grit_law_descend",
            &format!("{{\"world\":{mix},\"frames\":2,\"aim\":{aim},\"steps\":{steps}}}"),
        ));
        assert_eq!(text_at(&told, "field"), "$.steps");
    }
    let unmarked = refusal(&server.ask(
        "grit_law_gradient",
        &inline(&unmarked_mix, "[8,8,8]", &tiny, "[1,2]", aim),
    ));
    let law = unmarked.get("law").unwrap();
    assert_eq!(
        (text_at(law, "code"), text_at(law, "path")),
        ("missing".to_owned(), "$.reversible".to_owned())
    );
    assert_eq!(walks(), walked, "a refusal ran a walk");
    assert_eq!(state(&mut server, mix, 1), before);
    let frames_after: Vec<i64> = worlds
        .iter()
        .map(|world| frame_only(&mut server, *world))
        .collect();
    assert_eq!(frames_after, frames_before);
    println!(
        "{seen} refusals by code and path, none ran a walk (walks counted: {walked:?}); limits: cells {CELLS_MOST}, work {WORK_MOST}"
    );
}

const GUESSES: [[u32; 2]; 3] = [[2_100, 2_250], [3_600, 1_300], [2_700, 1_900]];

const LOOP_FRAMES: u32 = 8;

const LOOP_STEPS: u32 = 16;

const LOOP_BUDGET_MS: u32 = 30_000;

fn scripted(server: &mut Server<'_>) -> Vec<Json> {
    let start = calm(0x5C);
    let hidden = create(server, MIX, &MIX_TRUTH, SIDE);
    write_all(server, hidden, &start);
    step(server, hidden, LOOP_FRAMES);
    let last = SIDE - 1;
    let mut descents = Vec::new();
    for guess in GUESSES {
        let learner = create(server, MIX, &guess, SIDE);
        write_all(server, learner, &start);
        let descent = answer(&server.ask(
            "grit_law_descend",
            &format!(
                "{{\"world\":{learner},\"frames\":{LOOP_FRAMES},\"aim\":{{\"from\":[0,0,0],\"to\":[{last},{last},{last}],\"world\":{hidden}}},\"steps\":{LOOP_STEPS},\"budget_ms\":{LOOP_BUDGET_MS}}}"
            ),
        ));
        let learned = words_of(descent.get("params").unwrap());
        answer(&server.ask(
            "grit_world_params",
            &format!("{{\"world\":{learner},\"params\":{}}}", listed(&learned)),
        ));
        step(server, learner, LOOP_FRAMES);
        assert_eq!(
            state(server, learner, 1),
            state(server, hidden, 1),
            "from {guess:?} the learned params step the learner onto the hidden world"
        );
        descents.push(descent);
    }
    descents
}

#[test]
fn g5_a_client_that_knows_only_the_mcp_recovers_the_hidden_params() {
    let Some(host) = device("learn_grad scripted client") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let descents = scripted(&mut server);
    for (guess, descent) in GUESSES.iter().zip(&descents) {
        assert_eq!(whole_of(descent.get("objective").unwrap()), 0, "{guess:?}");
        assert_eq!(words_of(descent.get("params").unwrap()), MIX_TRUTH);
        assert_eq!(text_at(descent, "stopped"), "converged");
        let steps = int_at(descent, "steps");
        assert!(steps <= i64::from(LOOP_STEPS));
        assert_ne!(guess, &MIX_TRUTH);
        println!(
            "from {guess:?} the descent found {MIX_TRUTH:?} in {steps} steps over {LOOP_FRAMES} frames within {LOOP_BUDGET_MS} ms: {}",
            written(descent.get("trail").unwrap())
        );
    }
    let mut again = Server::new(Some(&host));
    let replayed = scripted(&mut again);
    for (one, other) in descents.iter().zip(&replayed) {
        assert_eq!(written(one), written(other), "deterministic");
    }
}

#[test]
fn the_tools_are_listed_described_and_checked_against_their_schemas() {
    let mut server = Server::new(None);
    let listed = server
        .heard("{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}")
        .unwrap();
    let tools = listed
        .get("result")
        .and_then(|one| one.get("tools"))
        .and_then(Json::list)
        .unwrap();
    for (name, keys) in [
        (
            "grit_law_gradient",
            &[
                "world",
                "law",
                "size",
                "cells",
                "first",
                "params",
                "frames",
                "aim",
                "budget_ms",
            ][..],
        ),
        (
            "grit_law_descend",
            &[
                "world",
                "law",
                "size",
                "cells",
                "first",
                "params",
                "frames",
                "aim",
                "steps",
                "budget_ms",
            ][..],
        ),
    ] {
        let tool = tools
            .iter()
            .find(|one| one.get("name").and_then(Json::text) == Some(name))
            .unwrap();
        let properties = tool
            .get("inputSchema")
            .and_then(|one| one.get("properties"))
            .unwrap();
        let Json::Map(held) = properties else {
            panic!("{name}")
        };
        let names: Vec<&str> = held.iter().map(|(key, _)| key.as_str()).collect();
        assert_eq!(names, keys, "{name}");
    }
    let described = answer(&call(&mut server, "grit_describe", "{}"));
    let codes: Vec<String> = described
        .get("codes")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .map(|one| text_at(one, "code"))
        .collect();
    for code in [
        "not_reversible",
        "no_gradient",
        "target_size",
        "overflow",
        "strayed",
        "start",
        "aim",
        "walk_limit",
    ] {
        assert!(codes.iter().any(|one| one == code), "{code}");
    }
    let limits = described.get("limits").unwrap();
    assert_eq!(
        int_at(limits, "walk_cells"),
        i64::try_from(CELLS_MOST).unwrap()
    );
    assert_eq!(
        int_at(limits, "walk_work"),
        i64::try_from(WORK_MOST).unwrap()
    );
    assert_eq!(int_at(limits, "walk_frames"), i64::from(FRAMES_MOST));
    assert_eq!(int_at(limits, "descend_steps"), i64::from(STEPS_MOST));
    let told = text_at(limits, "law_gradient");
    for word in ["Reversible::walk", "2^64", "no_gradient", "walk_work"] {
        assert!(told.contains(word), "{word}: {told}");
    }
    let rates = described
        .get("submissions")
        .and_then(|one| one.get("rates"))
        .unwrap();
    assert!(rates.get("walk").is_some(), "{}", written(rates));
}

#[test]
fn g1_a_gradient_past_128_bits_is_refused_as_overflow_like_the_walk() {
    let mut lifter = Lifter::new();
    lifter.lift(1, "add", "[\"mul\",\"w0v0\",[\"param\",0]]");
    lifter.lift(0, "add", "[\"mul\",\"w1v1\",\"w1v1\"]");
    let text = lifter.text("square", 1);
    let law = reversible(&text);
    let mut state = 0x0F;
    let start: Vec<u32> = (0..grid().cells())
        .map(|_| u32::MAX - u32::try_from(below(&mut state, 1 << 16)).unwrap())
        .collect();
    let target = vec![0; grid().cells()];
    let posed = Inline {
        text: &text,
        start: &start,
        first: 0,
        params: &[u32::MAX - 3],
        frames: 4,
        region: Region::whole([SIDE, SIDE, SIDE]),
        target: &target,
    };
    let walked = posed.walked(&law);
    assert_eq!(walked, Err(Fault::Overflow), "the case must overflow");
    let mut server = Server::new(None);
    let got = call(&mut server, "grit_law_gradient", &posed.arguments(""));
    same_gradient("square", &got, &walked);
    assert_eq!(text_at(&refusal(&got), "field"), "$.frames");
    let got = call(
        &mut server,
        "grit_law_descend",
        &posed.arguments(",\"steps\":2"),
    );
    same_descent("square", &got, &posed.descent(&law, 2), 2);
}

fn own_objective(region: Region, target: &[u32], cells: &[u32]) -> u64 {
    boxed(region, cells)
        .iter()
        .zip(target)
        .map(|(&cell, &wanted)| u64::from(cell.abs_diff(wanted)))
        .sum()
}

struct Owned {
    law: Reversible,
    start: Vec<u32>,
    first: u32,
    frames: u32,
    region: Region,
    target: Vec<u32>,
}

impl Owned {
    fn objective(&self, params: &[u32]) -> u64 {
        own_objective(
            self.region,
            &self.target,
            &ran(&self.law, &self.start, params, self.first, self.frames),
        )
    }

    fn posed(&self) -> Posed<'_> {
        Posed {
            grid: grid(),
            start: &self.start,
            first: self.first,
            frames: self.frames,
            aim: Aim {
                region: self.region,
                target: &self.target,
            },
        }
    }
}

fn never_worse(label: &str, owned: &Owned, from: &[u32], most: u32, descent: &Descent) {
    never_worse_by(label, |params| owned.objective(params), from, most, descent);
}

fn never_worse_by(
    label: &str,
    objective: impl Fn(&[u32]) -> u64,
    from: &[u32],
    most: u32,
    descent: &Descent,
) {
    let trail = &descent.trail;
    assert_eq!(
        trail[0],
        Tried {
            params: from.to_vec(),
            objective: objective(from),
            kept: true,
        },
        "{label}: the trail starts at the start"
    );
    let mut kept = trail[0].clone();
    for (at, tried) in trail.iter().enumerate().skip(1) {
        assert_eq!(
            tried.objective,
            objective(&tried.params),
            "{label}: trail {at} is the forward run's objective"
        );
        assert_eq!(
            tried.kept,
            tried.objective < kept.objective,
            "{label}: trail {at} is kept exactly when it is lower than the kept point {}: {trail:?}",
            kept.objective
        );
        assert!(
            trail[..at].iter().all(|one| one.params != tried.params),
            "{label}: trail {at} walks a point again"
        );
        if tried.kept {
            kept = tried.clone();
        }
    }
    assert!(
        descent.objective <= trail[0].objective,
        "{label}: worse than the start"
    );
    assert_eq!(
        (descent.params.clone(), descent.objective),
        (kept.params.clone(), kept.objective),
        "{label}: the answer is the last kept point"
    );
    assert_eq!(
        (descent.params.clone(), descent.objective),
        best_of(trail),
        "{label}: the answer is the earliest lowest point walked"
    );
    assert_eq!(descent.steps as usize, trail.len() - 1, "{label}");
    assert!(descent.steps <= most, "{label}: steps");
    match descent.settled {
        Some(Settled::Converged) => assert_eq!(descent.objective, 0, "{label}"),
        Some(Settled::Steps) => {
            assert!(descent.objective > 0 && descent.steps == most, "{label}");
        }
        Some(Settled::Local) => {
            assert!(descent.objective > 0 && descent.steps < most, "{label}");
        }
        None => panic!("{label}: the library descent did not settle"),
    }
}

const SWEEP: u64 = 160;

fn swept_law(seed: u64) -> (String, Vec<u32>) {
    if seed.is_multiple_of(2) {
        return smooth(0xD100 + seed);
    }
    let mut tried = seed;
    loop {
        let (text, values) = random_law(0xD1C0_0000 + tried);
        if grit::serve3::gradient::cut(reversible(&text).forward()).is_none() {
            return (text, values);
        }
        tried += SWEEP;
    }
}

fn swept_start(seed: u64, kind: u64) -> Vec<u32> {
    let mut state = seed ^ 0x5717;
    match kind {
        0 => calm(seed),
        1 => (0..grid().cells()).map(|_| word(&mut state)).collect(),
        _ => (0..grid().cells())
            .map(|_| (1 << 26) + u32::try_from(below(&mut state, 1 << 26)).unwrap())
            .collect(),
    }
}

fn near(state: &mut u64, values: &[u32]) -> Vec<u32> {
    values
        .iter()
        .map(|&one| {
            let off = i64::try_from(below(state, 81)).unwrap() - 40;
            let made = i64::from(one) * (100 + off) / 100;
            u32::try_from(made.clamp(0, i64::from(u32::MAX))).unwrap()
        })
        .collect()
}

#[test]
fn d1_a_descent_never_answers_worse_than_its_start_and_its_kept_objectives_fall() {
    let mut descended = 0;
    let mut overflowed = 0;
    let mut lower = 0;
    let mut rejected = 0;
    let mut climbed_past_start = 0;
    let mut kinds = [0; 3];
    for seed in 0..SWEEP {
        let (text, truth) = swept_law(seed);
        let law = reversible(&text);
        let mut state = seed.wrapping_mul(0x9E37) + 1;
        let kind = below(&mut state, 3);
        let start = swept_start(seed, kind);
        let first = u32::try_from(below(&mut state, 5)).unwrap();
        let frames = 1 + u32::try_from(below(&mut state, 8)).unwrap();
        let region = if below(&mut state, 4) == 0 {
            random_region(&mut state)
        } else {
            Region::whole([SIDE, SIDE, SIDE])
        };
        let target = if below(&mut state, 4) == 0 {
            (0..region.cells()).map(|_| word(&mut state)).collect()
        } else {
            boxed(region, &ran(&law, &start, &truth, first, frames))
        };
        let from = near(&mut state, &truth);
        let most = 1 + u32::try_from(below(&mut state, 24)).unwrap();
        let owned = Owned {
            law,
            start,
            first,
            frames,
            region,
            target,
        };
        let label = format!("case {seed} (start kind {kind}, {frames} frames, {most} steps)");
        let descent = match owned.law.descend(&owned.posed(), &from, most) {
            Ok(descent) => descent,
            Err(Fault::Overflow) => {
                overflowed += 1;
                continue;
            }
            Err(other) => panic!("{label}: {other}"),
        };
        never_worse(&label, &owned, &from, most, &descent);
        descended += 1;
        kinds[usize::try_from(kind).unwrap()] += 1;
        if descent.objective < descent.trail[0].objective {
            lower += 1;
        }
        if descent.trail.iter().any(|tried| !tried.kept) {
            rejected += 1;
        }
        if descent
            .trail
            .last()
            .is_some_and(|last| last.objective > descent.trail[0].objective)
        {
            climbed_past_start += 1;
        }
    }
    println!(
        "{SWEEP} cases: {descended} descended ({kinds:?} by start kind), {overflowed} past 128 bits at a walk; {lower} answered lower than their start, {rejected} walked a point they did not keep, {climbed_past_start} walked last a point worse than their start; every answer no worse than its start and every kept objective lower than the one before"
    );
    assert!(descended >= SWEEP * 9 / 10, "{descended}");
    assert!(kinds.iter().all(|one| *one >= SWEEP / 5), "{kinds:?}");
    assert!(lower >= SWEEP / 3, "{lower}");
    assert!(rejected >= descended / 4, "{rejected}");
    assert!(climbed_past_start >= SWEEP / 16, "{climbed_past_start}");
}

const JUDGE_FROM: [u32; 2] = [2_100, 2_250];

const JUDGE_FRAMES: u32 = 8;

const JUDGE_STEPS: u32 = 16;

const JUDGE_START: u64 = 588_373_928_375;

const JUDGE_BEST: [u32; 2] = [232, 0];

const JUDGE_LOWEST: u64 = 553_884_330_929;

const JUDGE_WALKED: i64 = 11;

const RIPPLE: i64 = 12;

fn judge_words() -> Vec<[u32; 4]> {
    let mut words = Vec::new();
    for x in (0..SIDE).step_by(2) {
        for y in (0..SIDE).step_by(2) {
            for z in (0..SIDE).step_by(4) {
                let mixed = (u64::from(x * 7 + y * 13 + z * 29) * 2_654_435_761) & 0xFFFF_FFFF;
                words.push([x, y, z, u32::try_from(mixed).unwrap()]);
            }
        }
    }
    words
}

fn judge_start() -> Vec<u32> {
    let mut cells = vec![0; grid().cells()];
    for [x, y, z, word] in judge_words() {
        cells[grid().index(x, y, z)] = word;
    }
    cells
}

fn judge_owned(start: Vec<u32>) -> Owned {
    let law = reversible(MIX);
    let region = Region::whole([SIDE, SIDE, SIDE]);
    let target = boxed(region, &ran(&law, &start, &MIX_TRUTH, 0, JUDGE_FRAMES));
    Owned {
        law,
        start,
        first: 0,
        frames: JUDGE_FRAMES,
        region,
        target,
    }
}

fn judge_arguments(owned: &Owned) -> String {
    Inline {
        text: MIX,
        start: &owned.start,
        first: 0,
        params: &JUDGE_FROM,
        frames: JUDGE_FRAMES,
        region: owned.region,
        target: &owned.target,
    }
    .arguments(&format!(",\"steps\":{JUDGE_STEPS}"))
}

fn checked_judge(got: &Json, owned: &Owned) {
    let descent = owned
        .law
        .descend(&owned.posed(), &JUDGE_FROM, JUDGE_STEPS)
        .unwrap();
    never_worse(
        "the judge's case",
        owned,
        &JUDGE_FROM,
        JUDGE_STEPS,
        &descent,
    );
    assert_eq!(trail_of(got), descent.trail, "the library's trail");
    assert_eq!(descent.trail[0].objective, JUDGE_START);
    assert_eq!(words_of(got.get("params").unwrap()), JUDGE_BEST);
    assert_eq!(whole_of(got.get("objective").unwrap()), JUDGE_LOWEST);
    assert_eq!(text_at(got, "stopped"), "local");
    assert_eq!(int_at(got, "steps"), JUDGE_WALKED);
}

fn ripple(owned: &Owned, axis: usize, sign: i64) -> i64 {
    let mut before = 0;
    for distance in 1..=4 * RIPPLE {
        let mut params = MIX_TRUTH;
        params[axis] = u32::try_from(i64::from(params[axis]) + sign * distance).unwrap();
        let objective = owned.objective(&params);
        if objective <= before {
            return distance;
        }
        before = objective;
    }
    panic!("the objective rises {} units from the truth", 4 * RIPPLE);
}

#[test]
fn d2_the_judges_case_answers_its_documented_best_and_a_calm_start_reaches_zero() {
    let owned = judge_owned(judge_start());
    let mut server = Server::new(None);
    let got = answer(&call(
        &mut server,
        "grit_law_descend",
        &judge_arguments(&owned),
    ));
    checked_judge(&got, &owned);
    let mut widest = 0;
    for axis in 0..2 {
        for sign in [-1, 1] {
            let ends = ripple(&owned, axis, sign);
            assert!(ends <= RIPPLE, "axis {axis} {sign}: {ends}");
            widest = widest.max(ends);
        }
        assert!(
            i64::from(JUDGE_FROM[axis].abs_diff(MIX_TRUTH[axis])) > 40 * RIPPLE,
            "the start lies far outside the ripple"
        );
    }
    println!(
        "the judge's case: from {JUDGE_FROM:?} (objective {JUDGE_START}) to {JUDGE_BEST:?} (objective {JUDGE_LOWEST}), local after {JUDGE_WALKED} walks; the objective rises at most {widest} units from {MIX_TRUTH:?} along each param, and the start is {} and {} away",
        JUDGE_FROM[0].abs_diff(MIX_TRUTH[0]),
        JUDGE_FROM[1].abs_diff(MIX_TRUTH[1])
    );
    let calm = judge_owned(calm(0x5C));
    let got = answer(&call(
        &mut server,
        "grit_law_descend",
        &Inline {
            text: MIX,
            start: &calm.start,
            first: 0,
            params: &JUDGE_FROM,
            frames: JUDGE_FRAMES,
            region: calm.region,
            target: &calm.target,
        }
        .arguments(&format!(",\"steps\":{JUDGE_STEPS}")),
    ));
    let descent = calm
        .law
        .descend(&calm.posed(), &JUDGE_FROM, JUDGE_STEPS)
        .unwrap();
    never_worse("calm", &calm, &JUDGE_FROM, JUDGE_STEPS, &descent);
    assert_eq!(trail_of(&got), descent.trail);
    assert_eq!(words_of(got.get("params").unwrap()), MIX_TRUTH);
    assert_eq!(whole_of(got.get("objective").unwrap()), 0);
    assert_eq!(text_at(&got, "stopped"), "converged");
    assert_eq!(int_at(&got, "steps"), 3);
}

const BASIN_SEEDS: u64 = 16;

#[test]
fn d2_from_inside_the_basin_a_calm_start_reaches_zero_within_sixteen_steps() {
    let mut most = 0;
    for (name, text, truth) in [
        ("mix", MIX, MIX_TRUTH.to_vec()),
        ("stir", STIR, STIR_TRUTH.to_vec()),
    ] {
        for seed in 0..BASIN_SEEDS {
            let law = reversible(text);
            let start = calm(0xBA51 + seed);
            let region = Region::whole([SIDE, SIDE, SIDE]);
            let target = boxed(region, &ran(&law, &start, &truth, 0, JUDGE_FRAMES));
            let owned = Owned {
                law,
                start,
                first: 0,
                frames: JUDGE_FRAMES,
                region,
                target,
            };
            let mut state = seed + 1;
            let from = near(&mut state, &truth);
            let descent = owned
                .law
                .descend(&owned.posed(), &from, JUDGE_STEPS)
                .unwrap();
            let label = format!("{name} seed {seed} from {from:?}");
            never_worse(&label, &owned, &from, JUDGE_STEPS, &descent);
            assert_eq!(descent.objective, 0, "{label}: {:?}", descent.trail);
            assert_eq!(descent.params, truth, "{label}");
            most = most.max(descent.steps);
        }
    }
    println!(
        "mix and stir, {BASIN_SEEDS} calm starts each, from up to 40 % off the truth: objective 0 in at most {most} of {JUDGE_STEPS} steps"
    );
}

#[test]
fn d4_the_same_descent_again_on_a_fresh_server_and_fewer_steps_a_prefix() {
    let owned = judge_owned(judge_start());
    let arguments = judge_arguments(&owned);
    let mut server = Server::new(None);
    let whole = written(&answer(&call(&mut server, "grit_law_descend", &arguments)));
    let again = written(&answer(&call(&mut server, "grit_law_descend", &arguments)));
    let fresh = written(&answer(&call(
        &mut Server::new(None),
        "grit_law_descend",
        &arguments,
    )));
    assert_eq!(again, whole, "the same server, the same answer");
    assert_eq!(fresh, whole, "a fresh server, the same answer");
    let full = trail_of(&grit::wire::read(&whole).unwrap());
    for steps in 1..JUDGE_STEPS {
        let shorter = Inline {
            text: MIX,
            start: &owned.start,
            first: 0,
            params: &JUDGE_FROM,
            frames: JUDGE_FRAMES,
            region: owned.region,
            target: &owned.target,
        }
        .arguments(&format!(",\"steps\":{steps}"));
        let got = answer(&call(&mut server, "grit_law_descend", &shorter));
        let trail = trail_of(&got);
        let walked = trail.len();
        assert_eq!(trail[..], full[..walked], "{steps} steps: a prefix");
        assert_eq!(
            (
                words_of(got.get("params").unwrap()),
                whole_of(got.get("objective").unwrap())
            ),
            best_of(&trail),
            "{steps} steps"
        );
        assert_eq!(
            walked,
            full.len().min(usize::try_from(steps).unwrap() + 1),
            "{steps} steps"
        );
    }
    let pinned = grit::law::hex(&grit::branch::sha256(whole.as_bytes()));
    println!("the judge's descent answer sha256 {pinned}");
    assert_eq!(pinned, JUDGE_PINNED, "the answer on any machine and OS");
}

const JUDGE_PINNED: &str = "07e867a9668d992f5cde995a6d19bbdb6c9d0f0b4e9c73ebb2ee20b3c5d4ddc1";

#[test]
fn the_descent_reaches_the_search_or_lower_at_the_same_runs_on_smooth_starts() {
    let mut server = Server::new(None);
    let mut rows = [[0_u32; 3]; 3];
    for seed in 0..96_u64 {
        let (text, truth) = smooth(0xD100 + seed);
        let law = reversible(&text);
        let mut state = seed.wrapping_mul(0x9E37) + 1;
        let kind = below(&mut state, 3);
        let start = swept_start(seed, kind);
        let frames = 1 + u32::try_from(below(&mut state, 8)).unwrap();
        let region = Region::whole([SIDE, SIDE, SIDE]);
        let target = boxed(region, &ran(&law, &start, &truth, 0, frames));
        let from = near(&mut state, &truth);
        let posed = Inline {
            text: &text,
            start: &start,
            first: 0,
            params: &from,
            frames,
            region,
            target: &target,
        };
        let descended = call(
            &mut server,
            "grit_law_descend",
            &posed.arguments(&format!(",\"steps\":{JUDGE_STEPS}")),
        );
        let descended = answer(&descended);
        let runs = u32::try_from(trail_of(&descended).len()).unwrap();
        let generations = (runs - 1).div_ceil(4).max(1);
        let bounds: Vec<[u32; 2]> = from.iter().map(|_| [0, 8_191]).collect();
        let searched = answer(&call(
            &mut server,
            "grit_learn_params",
            &posed.arguments(&format!(
                ",\"bounds\":{bounds:?},\"seed\":{seed},\"population\":4,\"generations\":{generations}"
            )),
        ));
        let scored = u32::try_from(int_at(&searched, "scored")).unwrap();
        assert!(scored >= runs, "seed {seed}: {scored} runs against {runs}");
        let low = whole_of(descended.get("objective").unwrap());
        let found = whole_of(searched.get("objective").unwrap());
        let row = &mut rows[usize::try_from(kind).unwrap()];
        row[usize::from(low > found) + usize::from(low >= found)] += 1;
        if kind == 0 {
            assert_eq!(low, 0, "seed {seed}: a calm start reaches 0");
            assert!(low <= found, "seed {seed}");
        }
    }
    for (kind, [won, tied, lost]) in rows.iter().enumerate() {
        println!(
            "start kind {kind}: the descent lower in {won}, equal in {tied}, the search lower in {lost}, the search given at least the descent's walks in runs"
        );
    }
    assert_eq!(rows[0][2], 0);
}

mod judged {
    use super::*;

    use std::io::{BufRead, BufReader, Write};
    use std::path::{Path, PathBuf};
    use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "grit-learngrad-judged-{}-{nanos}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn dir(&self) -> PathBuf {
            self.0.join("worlds")
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct Process {
        child: Child,
        input: ChildStdin,
        output: BufReader<ChildStdout>,
        id: i64,
    }

    impl Process {
        fn start(dir: &Path) -> Self {
            let mut child = Command::new(env!("CARGO_BIN_EXE_grit-mcp"))
                .arg("--dir")
                .arg(dir)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap();
            let input = child.stdin.take().unwrap();
            let output = BufReader::new(child.stdout.take().unwrap());
            Self {
                child,
                input,
                output,
                id: 0,
            }
        }

        fn kill(mut self) {
            self.child.kill().unwrap();
            self.child.wait().unwrap();
        }
    }

    impl Ask for Process {
        fn ask(&mut self, name: &str, arguments: &str) -> Json {
            self.id += 1;
            writeln!(self.input, "{}", line(self.id, name, arguments)).unwrap();
            self.input.flush().unwrap();
            loop {
                let mut held = String::new();
                assert!(
                    self.output.read_line(&mut held).unwrap() > 0,
                    "the server ended"
                );
                let got = grit::wire::read(held.trim_end()).unwrap();
                if got.get("id").and_then(Json::int) == Some(self.id) {
                    return got;
                }
            }
        }
    }

    fn opened(asker: &mut dyn Ask) -> (i64, i64) {
        let placed: Vec<String> = judge_words()
            .iter()
            .map(|[x, y, z, word]| format!("[{x},{y},{z},{word}]"))
            .collect();
        let mut worlds = Vec::new();
        for params in [JUDGE_FROM, MIX_TRUTH] {
            let world = create(asker, MIX, &params, SIDE);
            answer(&asker.ask(
                "grit_world_write",
                &format!(
                    "{{\"world\":{world},\"cells\":[{}],\"budget_ms\":{LONG}}}",
                    placed.join(",")
                ),
            ));
            worlds.push(world);
        }
        step(asker, worlds[1], JUDGE_FRAMES);
        (worlds[0], worlds[1])
    }

    fn descend(asker: &mut dyn Ask, learner: i64, target: i64) -> Json {
        let last = SIDE - 1;
        answer(&asker.ask(
            "grit_law_descend",
            &format!(
                "{{\"world\":{learner},\"frames\":{JUDGE_FRAMES},\"aim\":{{\"from\":[0,0,0],\"to\":[{last},{last},{last}],\"world\":{target}}},\"steps\":{JUDGE_STEPS},\"budget_ms\":60000}}"
            ),
        ))
    }

    fn unworlded(got: &Json) -> String {
        match got {
            Json::Map(fields) => written(&Json::Map(
                fields
                    .iter()
                    .filter(|(key, _)| key != "world")
                    .cloned()
                    .collect(),
            )),
            other => written(other),
        }
    }

    #[test]
    fn d2_d4_the_judges_descent_through_grit_mcp_worlds_answers_the_same_after_a_restart() {
        let Some(host) = device("learn_grad judged") else {
            return;
        };
        drop(host);
        let owned = judge_owned(judge_start());
        let inline = written(&answer(&call(
            &mut Server::new(None),
            "grit_law_descend",
            &judge_arguments(&owned),
        )));
        let scratch = Scratch::new();
        let mut first = Process::start(&scratch.dir());
        let (learner, target) = opened(&mut first);
        let before = descend(&mut first, learner, target);
        checked_judge(&before, &owned);
        assert_eq!(int_at(&before, "world"), learner);
        assert_eq!(unworlded(&before), inline, "a world start is its cells");
        first.kill();
        let mut second = Process::start(&scratch.dir());
        for world in [learner, target] {
            answer(&second.ask(
                "grit_world_open",
                &format!("{{\"name\":\"world-{world}\"}}"),
            ));
        }
        let after = descend(&mut second, learner, target);
        assert_eq!(written(&after), written(&before), "across a restart");
        second.kill();
    }
}

#[cfg(feature = "faults")]
mod faulted {
    use super::*;

    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::Receiver;
    use std::time::{Duration, Instant};

    use grit::Host;
    use grit::clock::{Fake, Time};
    use grit::serve3::Options;
    use grit::serve3::exec::Exec;

    use grit::serve3::gradient::faults::{cancel_after, disarm};
    use grit::serve3::tools::faults::{disarm as unstall, stall_every};

    use super::delayed::Waiting;

    struct Run {
        exec: Exec,
        answers: Receiver<Json>,
        id: i64,
    }

    impl Run {
        fn new() -> Self {
            let pattern = std::env::var("GRIT_DEVICE")
                .ok()
                .filter(|one| !one.is_empty());
            let (exec, answers) = Exec::spawn(
                Box::new(move || Host::matching(pattern.as_deref())),
                &Options::default(),
            )
            .unwrap();
            Self {
                exec,
                answers,
                id: 100,
            }
        }
    }

    impl Ask for Run {
        fn ask(&mut self, name: &str, arguments: &str) -> Json {
            self.id += 1;
            self.exec.line(&line(self.id, name, arguments));
            loop {
                let got = self.answers.recv_timeout(Duration::from_secs(300)).unwrap();
                if got.get("id").and_then(Json::int) == Some(self.id) {
                    return got;
                }
            }
        }
    }

    const WALK_FLOOR: Duration = Duration::from_millis(1000);

    const WAITED: Duration = Duration::from_millis(4);

    const FLOOR: Duration = Duration::from_millis(20);

    struct Case {
        text: String,
        truth: Vec<u32>,
        from: Vec<u32>,
        start: Vec<u32>,
        frames: u32,
    }

    fn turning() -> Case {
        let (text, truth) = smooth(0x7704);
        let from = truth.iter().map(|one| one * 13 / 10).collect();
        Case {
            text,
            truth,
            from,
            start: calm(4),
            frames: 8,
        }
    }

    struct Worlds {
        learner: i64,
        hidden: i64,
    }

    fn opened(server: &mut dyn Ask, case: &Case) -> Worlds {
        let learner = create(server, &case.text, &case.from, SIDE);
        write_all(server, learner, &case.start);
        let hidden = create(server, &case.text, &case.truth, SIDE);
        write_all(server, hidden, &case.start);
        step(server, hidden, case.frames);
        Worlds { learner, hidden }
    }

    fn asked(case: &Case, worlds: &Worlds, steps: Option<u32>, budget: u64) -> String {
        let last = SIDE - 1;
        let steps = steps.map_or_else(String::new, |steps| format!(",\"steps\":{steps}"));
        format!(
            "{{\"world\":{},\"frames\":{},\"aim\":{{\"from\":[0,0,0],\"to\":[{last},{last},{last}],\"world\":{}}}{steps},\"budget_ms\":{budget}}}",
            worlds.learner, case.frames, worlds.hidden
        )
    }

    fn library(case: &Case, most: u32) -> Descent {
        let law = reversible(&case.text);
        let target = ran(&law, &case.start, &case.truth, 0, case.frames);
        law.descend(
            &Posed {
                grid: grid(),
                start: &case.start,
                first: 0,
                frames: case.frames,
                aim: Aim {
                    region: Region::whole([SIDE, SIDE, SIDE]),
                    target: &target,
                },
            },
            &case.from,
            most,
        )
        .unwrap()
    }

    fn climbs(descent: &Descent) -> usize {
        let objectives: Vec<u64> = descent.trail.iter().map(|tried| tried.objective).collect();
        (1..objectives.len())
            .find(|&at| objectives[at] > objectives[..at].iter().copied().min().unwrap())
            .unwrap_or_else(|| panic!("the case never climbs: {objectives:?}"))
    }

    type Kept = ((i64, Vec<u32>), (i64, Vec<u32>));

    fn kept(server: &mut dyn Ask, worlds: &Worlds) -> Kept {
        (
            state(server, worlds.learner, 1),
            state(server, worlds.hidden, 1),
        )
    }

    fn stopped_part_way(got: &Json, whole: &Descent, walked: usize, why: &str) {
        let trail = trail_of(got);
        assert_eq!(trail.len(), walked, "{}", written(got));
        assert_eq!(trail[..], whole.trail[..walked], "a prefix of the library");
        assert_eq!(text_at(got, "stopped"), why);
        let (best, lowest) = best_of(&trail);
        assert_ne!(
            trail.last().unwrap().params,
            best,
            "the case must stop on a step worse than the best"
        );
        assert!(
            !trail.last().unwrap().kept,
            "a step worse than the best is not kept"
        );
        assert_eq!(
            words_of(got.get("params").unwrap()),
            best,
            "the best so far"
        );
        assert_eq!(whole_of(got.get("objective").unwrap()), lowest);
        assert_eq!(int_at(got, "steps"), i64::try_from(walked - 1).unwrap());
    }

    fn stopped_early(got: &Json, whole: &Descent, walked: usize) {
        let trail = trail_of(got);
        assert_eq!(trail.len(), walked, "{}", written(got));
        assert_eq!(trail[..], whole.trail[..walked], "a prefix of the library");
        assert_eq!(text_at(got, "stopped"), "budget");
        let (best, lowest) = best_of(&trail);
        assert_eq!(
            words_of(got.get("params").unwrap()),
            best,
            "the best so far"
        );
        assert_eq!(whole_of(got.get("objective").unwrap()), lowest);
        assert_eq!(int_at(got, "steps"), i64::try_from(walked - 1).unwrap());
    }

    #[derive(Debug, PartialEq, Eq)]
    struct Told {
        answers: Vec<String>,
        walks: Vec<u64>,
        elapsed: Vec<Duration>,
        slept: Vec<Duration>,
        steps: u64,
    }

    struct Session {
        got: Vec<Json>,
        took: Vec<Duration>,
        told: Told,
    }

    #[derive(Debug)]
    struct Switched {
        waiting: Waiting,
        on: AtomicBool,
    }

    impl Time for Switched {
        fn now(&self) -> Instant {
            if self.on.load(Ordering::SeqCst) {
                self.waiting.now()
            } else {
                self.waiting.fake().now()
            }
        }

        fn sleep(&self, time: Duration) {
            if self.on.load(Ordering::SeqCst) {
                self.waiting.sleep(time);
            } else {
                self.waiting.fake().sleep(time);
            }
        }
    }

    fn session(
        host: &Host,
        time: Arc<dyn Time>,
        fake: &Fake,
        on: &dyn Fn(bool),
        (case, budget, stall): (&Case, u64, bool),
    ) -> Session {
        let mut server = Server::timed(Some(host), &Options::default(), time).unwrap();
        let worlds = opened(&mut server, case);
        let before = kept(&mut server, &worlds);
        let mut got = Vec::new();
        let mut took = Vec::new();
        let mut told = Told {
            answers: Vec::new(),
            walks: Vec::new(),
            elapsed: Vec::new(),
            slept: Vec::new(),
            steps: 0,
        };
        if stall {
            stall_every(WALK_FLOOR);
        }
        on(true);
        for _ in 0..2 {
            let (at, slept, ran) = (fake.elapsed(), fake.slept(), walks().unwrap());
            let began = Instant::now();
            let one =
                answer(&server.ask("grit_law_descend", &asked(case, &worlds, Some(12), budget)));
            took.push(began.elapsed());
            told.elapsed.push(fake.elapsed() - at);
            told.slept.push(fake.slept() - slept);
            told.walks.push(walks().unwrap() - ran);
            assert_eq!(
                u64::try_from(trail_of(&one).len()).unwrap(),
                *told.walks.last().unwrap(),
                "the trail holds every walk made"
            );
            told.answers.push(written(&one));
            got.push(one);
        }
        on(false);
        unstall();
        assert_eq!(kept(&mut server, &worlds), before, "no world moved");
        told.steps = fake.steps();
        Session { got, took, told }
    }

    fn faked(host: &Host, case: &Case, budget: u64, stall: bool) -> Session {
        let fake = Arc::new(Fake::new());
        session(
            host,
            Arc::clone(&fake) as Arc<dyn Time>,
            &fake,
            &|_| {},
            (case, budget, stall),
        )
    }

    fn twice(host: &Host, case: &Case, budget: u64, stall: bool) -> Session {
        let first = faked(host, case, budget, stall);
        let second = faked(host, case, budget, stall);
        assert_eq!(
            first.told, second.told,
            "budget {budget}: one session twice in one process"
        );
        first
    }

    fn ms(time: Duration) -> u64 {
        u64::try_from(time.as_millis()).unwrap()
    }

    #[test]
    fn g4_a_descent_stopped_by_its_budget_returns_the_best_so_far_and_the_same_again() {
        let Some(host) = device("learn_grad budget stop") else {
            return;
        };
        let case = turning();
        let whole = library(&case, 12);
        let walked = climbs(&whole) + 1;
        let walks = u32::try_from(walked).unwrap();
        for (budget, made) in [
            (WALK_FLOOR * walks + WALK_FLOOR / 2, walks),
            (WALK_FLOOR * walks, walks),
            (WALK_FLOOR * walks - Duration::from_millis(1), walks - 1),
        ] {
            let held = twice(&host, &case, ms(budget), true);
            println!(
                "a budget of {budget:?} made {:?} walks in {:?} of fake time",
                held.told.walks, held.told.elapsed
            );
            for got in &held.got {
                if made == walks {
                    stopped_part_way(got, &whole, walked, "budget");
                } else {
                    stopped_early(got, &whole, walked - 1);
                }
            }
            assert_eq!(
                held.told.walks,
                [u64::from(made); 2],
                "a budget of {budget:?} makes floor(budget / walk) walks of {WALK_FLOOR:?}"
            );
            assert_eq!(held.told.elapsed, [WALK_FLOOR * made; 2], "{budget:?}");
            assert_eq!(held.told.slept, [WALK_FLOOR * made; 2], "{budget:?}");
            assert_eq!(held.told.steps, 0);
            assert_eq!(
                held.told.answers[0], held.told.answers[1],
                "the same inputs, the same answer"
            );
        }
    }

    #[test]
    fn g4_a_descent_on_a_frozen_clock_answers_alike_at_any_budget_and_stops_on_none() {
        let Some(host) = device("learn_grad frozen") else {
            return;
        };
        let case = turning();
        let whole = library(&case, 12);
        let held = [1, 600_000].map(|budget| twice(&host, &case, budget, false));
        for (budget, one) in [1, 600_000].iter().zip(&held) {
            for got in &one.got {
                assert_eq!(
                    text_at(got, "stopped"),
                    settled_name(whole.settled),
                    "budget {budget}"
                );
                assert_eq!(trail_of(got), whole.trail, "budget {budget}: the library");
            }
            assert_eq!(one.told.elapsed, [Duration::ZERO; 2], "budget {budget}");
            assert_eq!(one.told.slept, [Duration::ZERO; 2], "budget {budget}");
            assert_eq!(one.told.answers[0], one.told.answers[1]);
        }
        assert_eq!(
            held[0].told, held[1].told,
            "a frozen clock answered differently at budgets of 1 ms and 600000 ms"
        );
    }

    #[test]
    fn g4_a_descent_under_a_waiting_clock_answers_as_on_the_fake_clock() {
        let Some(host) = device("learn_grad waiting") else {
            return;
        };
        let case = turning();
        let whole = library(&case, 12);
        let walks = u32::try_from(climbs(&whole) + 1).unwrap();
        for (budget, stall) in [
            (1, false),
            (600_000, false),
            (ms(WALK_FLOOR * walks + WALK_FLOOR / 2), true),
        ] {
            let plain = faked(&host, &case, budget, stall);
            let switched = Arc::new(Switched {
                waiting: Waiting::new(WAITED, WAITED),
                on: AtomicBool::new(false),
            });
            let delayed = session(
                &host,
                Arc::clone(&switched) as Arc<dyn Time>,
                switched.waiting.fake(),
                &|on| switched.on.store(on, Ordering::SeqCst),
                (&case, budget, stall),
            );
            let (waits, waited) = switched.waiting.waited();
            println!(
                "budget {budget}: {waits} waits, {waited:?} in all, the calls took {:?}",
                delayed.took
            );
            assert_eq!(
                plain.told, delayed.told,
                "budget {budget}: real time spent at the clock moved the descent"
            );
            if budget == 1 {
                for took in &delayed.took {
                    assert!(*took >= FLOOR, "budget {budget}: the call took {took:?}");
                }
            }
        }
    }

    #[test]
    fn g4_a_descent_cancelled_between_walks_returns_the_best_so_far_and_the_same_again() {
        let Some(host) = device("learn_grad cancel") else {
            return;
        };
        let case = turning();
        let whole = library(&case, 12);
        let walked = climbs(&whole) + 1;
        let mut server = Server::new(Some(&host));
        let worlds = opened(&mut server, &case);
        let before = kept(&mut server, &worlds);
        let mut answers = Vec::new();
        for _ in 0..2 {
            let flag = Arc::new(AtomicBool::new(false));
            server.arriving(None, Some(Arc::clone(&flag)));
            cancel_after(u64::try_from(walked).unwrap());
            let got = answer(&server.ask(
                "grit_law_descend",
                &asked(&case, &worlds, Some(12), 600_000),
            ));
            assert!(flag.load(Ordering::SeqCst));
            server.arriving(None, None);
            stopped_part_way(&got, &whole, walked, "cancelled");
            answers.push(written(&got));
        }
        disarm();
        assert_eq!(answers[0], answers[1]);
        assert_eq!(kept(&mut server, &worlds), before);
        let flag = Arc::new(AtomicBool::new(false));
        server.arriving(None, Some(Arc::clone(&flag)));
        cancel_after(1);
        let got = server.ask("grit_law_gradient", &asked(&case, &worlds, None, 600_000));
        server.arriving(None, None);
        disarm();
        assert_eq!(
            answer(&got).get("home"),
            Some(&Json::Bool(true)),
            "a gradient is one walk and answers whole"
        );
    }

    #[test]
    fn g3_a_walk_the_budget_cannot_fit_is_refused_by_evidence_before_it_runs() {
        let case = turning();
        let law = reversible(&case.text);
        let target = ran(&law, &case.start, &case.truth, 0, case.frames);
        let posed = Inline {
            text: &case.text,
            start: &case.start,
            first: 0,
            params: &case.from,
            frames: case.frames,
            region: Region::whole([SIDE, SIDE, SIDE]),
            target: &target,
        };
        let mut server = Server::new(None);
        stall_every(WALK_FLOOR);
        let began = Instant::now();
        answer(&call(
            &mut server,
            "grit_law_gradient",
            &posed.arguments(""),
        ));
        assert!(began.elapsed() >= WALK_FLOOR);
        let short = u64::try_from(WALK_FLOOR.as_millis()).unwrap() / 2;
        let ran_before = walks();
        for (tool, more) in [
            ("grit_law_gradient", String::new()),
            ("grit_law_descend", ",\"steps\":3".to_owned()),
        ] {
            let arguments = posed.arguments(&more).replace(
                &format!("\"budget_ms\":{LONG}"),
                &format!("\"budget_ms\":{short}"),
            );
            let told = refusal(&call(&mut server, tool, &arguments));
            assert_eq!(
                (text_at(&told, "code"), text_at(&told, "field")),
                ("budget".to_owned(), "$.budget_ms".to_owned()),
                "{tool}: {}",
                written(&told)
            );
            assert!(text_at(&told, "message").contains("nothing ran"));
        }
        assert_eq!(walks(), ran_before, "a budget refusal ran a walk");
        unstall();
    }

    #[test]
    fn h5_a_walk_of_a_law_that_looks_the_budget_cannot_fit_is_refused_by_evidence() {
        let (case, _) = spread_case(2, &[2_400, 2_000], 4);
        let mut server = Server::new(None);
        stall_every(WALK_FLOOR);
        let began = Instant::now();
        answer(&call(
            &mut server,
            "grit_law_gradient",
            &posed_case(&case, ""),
        ));
        assert!(began.elapsed() >= WALK_FLOOR);
        let short = u64::try_from(WALK_FLOOR.as_millis()).unwrap() / 2;
        let ran_before = walks();
        for (tool, more) in [
            ("grit_law_gradient", String::new()),
            ("grit_law_descend", ",\"steps\":3".to_owned()),
        ] {
            let arguments = posed_case(&case, &more).replace(
                &format!("\"budget_ms\":{LONG}"),
                &format!("\"budget_ms\":{short}"),
            );
            let told = refusal(&call(&mut server, tool, &arguments));
            assert_eq!(
                (text_at(&told, "code"), text_at(&told, "field")),
                ("budget".to_owned(), "$.budget_ms".to_owned()),
                "{tool}: {}",
                written(&told)
            );
            assert!(text_at(&told, "message").contains("nothing ran"));
        }
        assert_eq!(walks(), ran_before, "a budget refusal ran a walk");
        unstall();
    }

    #[test]
    fn g3_a_walk_that_strays_answers_strayed_with_where_and_touches_no_world() {
        let Some(host) = device("learn_grad strayed") else {
            return;
        };
        let case = turning();
        let mut server = Server::new(Some(&host));
        let worlds = opened(&mut server, &case);
        let before = kept(&mut server, &worlds);
        for (tool, steps) in [("grit_law_gradient", None), ("grit_law_descend", Some(3))] {
            server.faults().lock().unwrap().stray_walk = true;
            let told = refusal(&server.ask(tool, &asked(&case, &worlds, steps, 600_000)));
            assert_eq!(text_at(&told, "code"), "strayed", "{}", written(&told));
            let strayed = told.get("strayed").unwrap();
            let cell = words_of(strayed.get("cell").unwrap());
            assert!(cell.iter().all(|one| *one < SIDE), "{}", written(&told));
            assert_eq!(int_at(strayed, "plane"), 0);
            assert_ne!(int_at(strayed, "was"), int_at(strayed, "now"));
            let at = grid().index(cell[0], cell[1], cell[2]);
            assert_eq!(
                u32::try_from(int_at(strayed, "was")).unwrap(),
                case.start[at],
                "was is the start's word"
            );
        }
        assert_eq!(kept(&mut server, &worlds), before);
        let fine =
            answer(&server.ask("grit_law_descend", &asked(&case, &worlds, Some(3), 600_000)));
        assert_eq!(trail_of(&fine), library(&case, 3).trail);
    }

    #[test]
    fn g4_a_device_lost_at_a_descent_leaves_every_world_and_the_next_answer_as_before() {
        let Some(host) = device("learn_grad lost device") else {
            return;
        };
        drop(host);
        let case = turning();
        let mut run = Run::new();
        let worlds = opened(&mut run, &case);
        let before = kept(&mut run, &worlds);
        let descend = asked(&case, &worlds, Some(3), 600_000);
        let clean = answer(&run.ask("grit_law_descend", &descend));
        for (tool, arguments) in [
            ("grit_law_descend", descend.clone()),
            ("grit_law_gradient", asked(&case, &worlds, None, 600_000)),
        ] {
            run.exec.faults().lock().unwrap().wedge_before_call = true;
            let told = refusal(&run.ask(tool, &arguments));
            assert_eq!(text_at(&told, "code"), "device_lost", "{}", written(&told));
            assert_eq!(kept(&mut run, &worlds), before, "rebuilt as before");
        }
        let again = answer(&run.ask("grit_law_descend", &descend));
        assert_eq!(written(&again), written(&clean));
        run.exec.finish();
    }
}

fn posed_case(case: &halo_grad::Case, more: &str) -> String {
    let last = case.region.to.map(|one| one - 1);
    format!(
        "{{\"law\":{},\"size\":{:?},\"cells\":{},\"first\":{},\"params\":{},\"frames\":{},\"aim\":{{\"from\":{:?},\"to\":{last:?},\"target\":{}}},\"budget_ms\":{LONG}{more}}}",
        quoted(&case.text),
        case.size,
        listed(&case.start),
        case.first,
        listed(&case.params),
        case.frames,
        case.region.from,
        listed(&case.target)
    )
}

fn descent_of(case: &halo_grad::Case, law: &Reversible, most: u32) -> Result<Descent, Fault> {
    law.descend(
        &Posed {
            grid: case.grid(),
            start: &case.start,
            first: case.first,
            frames: case.frames,
            aim: case.aim(),
        },
        &case.params,
        most,
    )
}

const LOOKING_LAWS: u64 = 32;

#[test]
fn h4_a_law_that_looks_answers_the_walk_exactly_through_the_mcp() {
    let mut server = Server::new(None);
    let mut moved = 0;
    for seed in 0..LOOKING_LAWS {
        let drawn = halo_grad::case(0x4400 + seed);
        let case = &drawn.case;
        let label = format!("looking law {seed}");
        let walked = case.walked(&drawn.reversible);
        if walked
            .as_ref()
            .is_ok_and(|one| one.told.iter().any(|told| *told != 0))
        {
            moved += 1;
        }
        let got = call(&mut server, "grit_law_gradient", &posed_case(case, ""));
        same_gradient(&label, &got, &walked);
        let most = 1 + u32::try_from(seed % 4).unwrap();
        let got = call(
            &mut server,
            "grit_law_descend",
            &posed_case(case, &format!(",\"steps\":{most}")),
        );
        same_descent(
            &label,
            &got,
            &descent_of(case, &drawn.reversible, most),
            most,
        );
    }
    println!(
        "{LOOKING_LAWS} laws that look: gradient and descent equal the library, {moved} with a gradient that is not zero"
    );
    assert!(moved >= LOOKING_LAWS / 4, "{moved}");
}

#[test]
fn h4_a_world_of_a_law_that_looks_answers_the_walk_exactly() {
    let Some(host) = device("learn_grad looking world") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let text = halo_grad::spread_law();
    let law = reversible(&text);
    let truth = halo_grad::SPREAD_TRUTH;
    let start_world = create(&mut server, &text, &truth, SIDE);
    write_all(&mut server, start_world, &calm(0x44));
    step(&mut server, start_world, 3);
    let other: Vec<u32> = truth.iter().map(|one| one + 40).collect();
    let target_world = create(&mut server, &text, &other, SIDE);
    write_all(&mut server, target_world, &calm(0x44));
    step(&mut server, target_world, 3 + 4);
    let (frame, start) = state(&mut server, start_world, 2);
    assert_eq!(frame, 3);
    let (_, aimed) = state(&mut server, target_world, 1);
    let region = Region::new([2, 0, 1], [14, 16, 9]);
    let target = boxed(region, &aimed);
    let aim = Aim {
        region,
        target: &target,
    };
    let aim_text = aimed_at(region, &format!("\"world\":{target_world}"));
    let walked = law.walk(grid(), &start, &truth, 3, 4, &aim);
    assert_ne!(walked.as_ref().unwrap().told[1], 0);
    let got = server.ask(
        "grit_law_gradient",
        &format!(
            "{{\"world\":{start_world},\"frames\":4,\"aim\":{aim_text},\"budget_ms\":{LONG}}}"
        ),
    );
    same_gradient("looking world", &got, &walked);
    let posed = Posed {
        grid: grid(),
        start: &start,
        first: 3,
        frames: 4,
        aim,
    };
    let got = server.ask(
        "grit_law_descend",
        &format!(
            "{{\"world\":{start_world},\"frames\":4,\"aim\":{aim_text},\"steps\":4,\"budget_ms\":{LONG}}}"
        ),
    );
    same_descent("looking world", &got, &law.descend(&posed, &truth, 4), 4);
    assert_eq!(state(&mut server, start_world, 2), (3, start));
}

const SPREAD_SEEDS: u64 = 16;

const SPREAD_SIZE: [u32; 3] = [16, 16, 8];

fn spread_case(seed: u64, from: &[u32], frames: u32) -> (halo_grad::Case, Reversible) {
    let text = halo_grad::spread_law();
    let law = reversible(&text);
    let grid = Grid3::new(SPREAD_SIZE[0], SPREAD_SIZE[1], SPREAD_SIZE[2]).unwrap();
    let mut case = halo_grad::Case {
        seed,
        text,
        params: from.to_vec(),
        size: SPREAD_SIZE,
        planes: 2,
        frozen: vec![false, true],
        frames,
        first: 0,
        start: halo_grad::spread_start(grid, 0x5B0 + seed),
        region: Region::whole(SPREAD_SIZE),
        target: Vec::new(),
    };
    let ended = halo_grad::ran(
        &compiled(&case.text),
        &case.posing(),
        &halo_grad::SPREAD_TRUTH,
    );
    case.target = halo_grad::boxed(grid, case.region, &ended);
    (case, law)
}

#[test]
fn h3_a_descent_through_the_mcp_learns_the_params_of_a_law_that_looks() {
    let truth = halo_grad::SPREAD_TRUTH.to_vec();
    let compiled_law = compiled(&halo_grad::spread_law());
    let mut server = Server::new(None);
    let mut most = 0;
    for seed in 0..SPREAD_SEEDS {
        let mut state = seed + 7;
        let from = near(&mut state, &truth);
        let (case, law) = spread_case(seed, &from, JUDGE_FRAMES);
        let label = format!("spread seed {seed} from {from:?}");
        let got = answer(&call(
            &mut server,
            "grit_law_descend",
            &posed_case(&case, &format!(",\"steps\":{JUDGE_STEPS}")),
        ));
        let descent = descent_of(&case, &law, JUDGE_STEPS).unwrap();
        let objective = |params: &[u32]| {
            let ended = halo_grad::ran(&compiled_law, &case.posing(), params);
            halo_grad::boxed(case.grid(), case.region, &ended)
                .iter()
                .zip(&case.target)
                .map(|(&cell, &wanted)| u64::from(cell.abs_diff(wanted)))
                .sum()
        };
        never_worse_by(&label, objective, &from, JUDGE_STEPS, &descent);
        assert_eq!(trail_of(&got), descent.trail, "{label}");
        assert_eq!(
            whole_of(got.get("objective").unwrap()),
            0,
            "{label}: {:?}",
            descent.trail
        );
        assert_eq!(words_of(got.get("params").unwrap()), truth, "{label}");
        assert_eq!(text_at(&got, "stopped"), "converged", "{label}");
        most = most.max(descent.steps);
    }
    println!(
        "spread, {SPREAD_SEEDS} calm starts from up to 40 % off {:?}: objective 0 through the MCP in at most {most} of {JUDGE_STEPS} steps",
        halo_grad::SPREAD_TRUTH
    );
}

fn cut_looker() -> String {
    let mut lifter = Lifter::planed(2);
    lifter.lift(
        0,
        "add",
        "[\"xor\",[\"mul\",\"w1v0\",[\"param\",0]],[\"look\",1,-1,0,0,0]]",
    );
    lifter
        .text("cut_looker", 1)
        .replace("\"halo\":0", "\"halo\":1")
}

fn written_looker() -> String {
    let mut lifter = Lifter::planed(2);
    lifter.lift(0, "add", "[\"mul\",[\"look\",1,-1,0,0,0],[\"param\",0]]");
    lifter.lift(9, "add", "[\"param\",0]");
    lifter
        .text("written_looker", 1)
        .replace("\"halo\":0", "\"halo\":1")
}

#[test]
fn h5_a_law_that_looks_and_cannot_be_covered_is_refused_by_name_before_any_walk() {
    let mut server = Server::new(None);
    let walked = walks();
    let aim = "{\"from\":[0,0,0],\"to\":[1,1,1],\"target\":[1,2,3,4,5,6,7,8]}";
    let cells = vec![CALM; 1024];
    let cut_text = cut_looker();
    let written_text = written_looker();
    for tool in ["grit_law_gradient", "grit_law_descend"] {
        let steps = if tool == "grit_law_descend" {
            "\"steps\":4,"
        } else {
            ""
        };
        let told = refusal(&call(
            &mut server,
            tool,
            &format!(
                "{{{steps}{}",
                &inline(&cut_text, "[8,8,8]", &cells, "[3]", aim)[1..]
            ),
        ));
        assert_eq!(
            (text_at(&told, "code"), text_at(&told, "field")),
            ("no_gradient".to_owned(), "$.law".to_owned()),
            "{}",
            written(&told)
        );
        assert!(text_at(&told, "message").contains("nothing ran"));
        let law = told.get("law").unwrap();
        assert_eq!(text_at(law, "found"), "xor");
        let node = located(&grit::wire::read(&cut_text).unwrap(), &text_at(law, "path"));
        assert_eq!(
            node.list().unwrap()[0].text(),
            Some("xor"),
            "{}",
            written(&told)
        );
        let told = refusal(&call(
            &mut server,
            tool,
            &format!(
                "{{{steps}{}",
                &inline(&written_text, "[8,8,8]", &cells, "[3]", aim)[1..]
            ),
        ));
        assert_eq!(
            (text_at(&told, "code"), text_at(&told, "field")),
            ("law_refused".to_owned(), "$.law".to_owned()),
            "{}",
            written(&told)
        );
        let law = told.get("law").unwrap();
        assert_eq!(text_at(law, "code"), "halo_written", "{}", written(&told));
        let node = located(
            &grit::wire::read(&written_text).unwrap(),
            &text_at(law, "path"),
        );
        assert_eq!(
            node.list().unwrap()[0].text(),
            Some("look"),
            "{}",
            written(&told)
        );
    }
    assert_eq!(walks(), walked, "a refusal ran a walk");
}

#[test]
fn h5_the_walk_limit_counts_the_looks_of_a_law_before_any_walk() {
    let Some(host) = device("learn_grad looking walk limit") else {
        return;
    };
    let text = halo_grad::spread_law();
    let law = reversible(&text);
    let octs = u64::from(grid().octs());
    let nodes = u64::try_from(law.forward().len() + law.inverse().len()).unwrap() + 128;
    let looking = nodes + 3 * 64;
    let fits = WORK_MOST / (octs * looking);
    let unlooked = WORK_MOST / (octs * nodes);
    assert!(
        fits < unlooked && unlooked < u64::from(FRAMES_MOST),
        "{fits} {unlooked}"
    );
    let mut server = Server::new(Some(&host));
    let world = create(&mut server, &text, &halo_grad::SPREAD_TRUTH, SIDE);
    write_all(&mut server, world, &calm(5));
    let before = state(&mut server, world, 2);
    let aim = "{\"from\":[0,0,0],\"to\":[1,1,1],\"target\":[1,2,3,4,5,6,7,8]}";
    let walked = walks();
    for frames in [fits + 1, unlooked] {
        for (tool, more) in [
            ("grit_law_gradient", ""),
            ("grit_law_descend", ",\"steps\":2"),
        ] {
            let told = refusal(&server.ask(
                tool,
                &format!("{{\"world\":{world},\"frames\":{frames},\"aim\":{aim}{more}}}"),
            ));
            assert_eq!(
                (text_at(&told, "code"), text_at(&told, "field")),
                ("walk_limit".to_owned(), "$.frames".to_owned()),
                "{tool} {frames}: {}",
                written(&told)
            );
            assert_eq!(
                told.get("limit").and_then(Json::int),
                Some(i64::try_from(fits).unwrap()),
                "{}",
                written(&told)
            );
            let units = octs * frames * looking;
            assert!(
                text_at(&told, "message").contains(&format!(
                    "is {units} units of work (octs times frames times the nodes of both plus 128 plus 192 for each of the 1 planes it looks at)"
                )),
                "{}",
                written(&told)
            );
        }
    }
    assert_eq!(walks(), walked, "a refusal ran a walk");
    assert_eq!(state(&mut server, world, 2), before);
}

fn unhinted(held: Json) -> Json {
    match held {
        Json::Map(pairs) => {
            let refused = pairs
                .iter()
                .any(|(key, value)| key == "code" && value.text() == Some("no_gradient"));
            Json::Map(
                pairs
                    .into_iter()
                    .filter(|(key, _)| !(refused && key == "hint"))
                    .map(|(key, value)| (key, unhinted(value)))
                    .collect(),
            )
        }
        Json::List(items) => Json::List(items.into_iter().map(unhinted).collect()),
        other => other,
    }
}

const MAIN_ANSWERS: &str = "305f4c04046a2417ddc957895a447a74bdbae0e00eea4a4f44abbae276eee0b6";

#[test]
fn h2_a_law_that_does_not_look_answers_through_the_mcp_byte_for_byte_as_on_main() {
    let mut server = Server::new(None);
    let mut heard = Vec::new();
    let mut asked = 0;
    for seed in 0..RANDOM_LAWS * 2 {
        let (text, values) = if seed % 2 == 0 {
            random_law(0xA1B0 + seed)
        } else {
            smooth(0xA1B0 + seed)
        };
        let law = reversible(&text);
        let mut state = seed.wrapping_mul(0x51ED);
        let start: Vec<u32> = if seed % 2 == 0 {
            (0..grid().cells()).map(|_| word(&mut state)).collect()
        } else {
            calm(seed)
        };
        let first = u32::try_from(below(&mut state, 5)).unwrap();
        let frames = 1 + u32::try_from(below(&mut state, 6)).unwrap();
        let region = random_region(&mut state);
        let target = boxed(region, &ran(&law, &start, &values, first, frames + 1));
        let posed = Inline {
            text: &text,
            start: &start,
            first,
            params: &values,
            frames,
            region,
            target: &target,
        };
        let most = 1 + u32::try_from(below(&mut state, 4)).unwrap();
        for more in [String::new(), format!(",\"steps\":{most}")] {
            let tool = if more.is_empty() {
                "grit_law_gradient"
            } else {
                "grit_law_descend"
            };
            heard.extend(
                written(&unhinted(call(&mut server, tool, &posed.arguments(&more)))).into_bytes(),
            );
            asked += 1;
        }
    }
    let mut cases: Vec<(String, Vec<u32>, Vec<u32>)> = vec![
        (MIX.to_owned(), MIX_TRUTH.to_vec(), calm(0x5C)),
        (STIR.to_owned(), STIR_TRUTH.to_vec(), calm(0xBA51)),
    ];
    for seed in 0..SMOOTH_LAWS {
        let (text, values) = smooth_far(0x5300 + seed);
        cases.push((text, values, far_start(0x77 + seed)));
    }
    for (_, text, values) in op_laws() {
        cases.push((text, values, far_start(0x0B)));
    }
    for (_, text) in cut_laws() {
        cases.push((text, vec![3_000], far_start(0x0C)));
    }
    for (case, (text, values, start)) in cases.iter().enumerate() {
        let law = reversible(text);
        let mut state = (case as u64).wrapping_mul(0xFA2) ^ 0x3D;
        let region = Region::whole([SIDE, SIDE, SIDE]);
        let target = if case < 2 {
            boxed(region, &ran(&law, start, values, 0, JUDGE_FRAMES))
        } else {
            far_target(&mut state, region)
        };
        let from: Vec<u32> = values.iter().map(|one| one * 85 / 100).collect();
        let posed = Inline {
            text,
            start,
            first: 0,
            params: &from,
            frames: JUDGE_FRAMES,
            region,
            target: &target,
        };
        for more in ["", ",\"steps\":16"] {
            let tool = if more.is_empty() {
                "grit_law_gradient"
            } else {
                "grit_law_descend"
            };
            heard.extend(
                written(&unhinted(call(&mut server, tool, &posed.arguments(more)))).into_bytes(),
            );
            asked += 1;
        }
    }
    let tiny = vec![CALM; 512];
    let aim = "{\"from\":[0,0,0],\"to\":[1,1,1],\"target\":[1,2,3,4,5,6,7,8]}";
    for (text, size, cells, params, frames) in [
        (MIX, "[8,8,8]", &tiny, "[1,2]", FRAMES_MOST),
        (MIX, "[8,8,8]", &tiny, "[1,2,3]", 2),
        (MIX, "[8,8,8]", &tiny[1..].to_vec(), "[1,2]", 2),
    ] {
        let arguments = inline(text, size, cells, params, aim)
            .replace("\"frames\":2", &format!("\"frames\":{frames}"));
        for tool in ["grit_law_gradient", "grit_law_descend"] {
            let arguments = if tool == "grit_law_descend" {
                arguments.replacen('{', "{\"steps\":4,", 1)
            } else {
                arguments.clone()
            };
            heard.extend(written(&unhinted(call(&mut server, tool, &arguments))).into_bytes());
            asked += 1;
        }
    }
    let digest = grit::law::hex(&grit::branch::format::sha256(&heard));
    println!("{asked} answers, {} bytes, sha256 {digest}", heard.len());
    assert_eq!(digest, MAIN_ANSWERS);
}
