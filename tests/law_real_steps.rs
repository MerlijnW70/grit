#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use std::time::Instant;

use grit::sand3::{Grid3, reference};
use grit::serve3::Server;
use grit::wire::{Json, written};

mod common;

const LONG: u64 = 600_000;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1_u64 << 53) as f64
    }

    fn normal(&mut self) -> f64 {
        let one = self.unit().max(1e-300);
        let two = self.unit();
        (-2.0 * one.ln()).sqrt() * (std::f64::consts::TAU * two).cos()
    }
}

fn line(name: &str, arguments: &str) -> String {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{{\"name\":\"{name}\",\"arguments\":{arguments}}}}}"
    )
}

fn heard(server: &mut Server<'_>, name: &str, arguments: &str) -> Json {
    let sent = line(name, arguments);
    assert!(
        sent.len() <= grit::serve3::transport::LINE_MOST,
        "a line of {} bytes",
        sent.len()
    );
    server.heard(&sent).unwrap()
}

fn result(got: &Json, error: bool) -> Json {
    let held = got
        .get("result")
        .unwrap_or_else(|| panic!("{}", written(got)));
    assert_eq!(
        held.get("isError"),
        Some(&Json::Bool(error)),
        "{}",
        &written(got)[..written(got).len().min(4000)]
    );
    held.get("structuredContent").cloned().unwrap()
}

fn text_at(held: &Json, key: &str) -> String {
    held.get(key)
        .and_then(Json::text)
        .unwrap_or_else(|| {
            panic!(
                "no {key} in {}",
                &written(held)[..written(held).len().min(3000)]
            )
        })
        .to_owned()
}

fn real(held: &Json) -> f64 {
    held.text().unwrap().parse().unwrap()
}

fn listed<T: ToString>(held: &[T]) -> String {
    format!(
        "[{}]",
        held.iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn words(field: &[f64], frac: u32) -> Vec<i32> {
    let unit = f64::from(1_u32 << frac);
    field
        .iter()
        .map(|one| (one * unit).round() as i32)
        .collect()
}

fn ints(held: &Json) -> Vec<u32> {
    held.list()
        .unwrap()
        .iter()
        .map(|one| u32::try_from(one.int().unwrap()).unwrap())
        .collect()
}

fn signed(held: &Json) -> Vec<i32> {
    held.list()
        .unwrap()
        .iter()
        .map(|one| i32::try_from(one.int().unwrap()).unwrap())
        .collect()
}

fn torus(side: usize, x: i64, y: i64) -> usize {
    let s = side as i64;
    (y.rem_euclid(s) * s + x.rem_euclid(s)) as usize
}

struct Drift {
    centre: f64,
    east: f64,
    west: f64,
    north: f64,
    south: f64,
    bias: f64,
}

const DRIFT: Drift = Drift {
    centre: 0.6,
    east: 0.12,
    west: 0.06,
    north: 0.1,
    south: 0.1,
    bias: 0.01,
};

fn drift_step(side: usize, field: &[f64]) -> Vec<f64> {
    let mut out = vec![0.0; field.len()];
    for y in 0..side as i64 {
        for x in 0..side as i64 {
            let at = |dx: i64, dy: i64| field[torus(side, x + dx, y + dy)];
            out[torus(side, x, y)] = DRIFT.centre * at(0, 0)
                + DRIFT.east * at(1, 0)
                + DRIFT.west * at(-1, 0)
                + DRIFT.north * at(0, 1)
                + DRIFT.south * at(0, -1)
                + DRIFT.bias;
        }
    }
    out
}

fn smooth_field(rng: &mut Rng, side: usize) -> Vec<f64> {
    let mut waves = Vec::new();
    for _ in 0..6 {
        waves.push((
            1.0 + (rng.unit() * 4.0).floor(),
            1.0 + (rng.unit() * 4.0).floor(),
            rng.unit() * std::f64::consts::TAU,
            0.04 + 0.08 * rng.unit(),
        ));
    }
    (0..side * side)
        .map(|at| {
            let x = (at % side) as f64 / side as f64;
            let y = (at / side) as f64 / side as f64;
            0.5 + waves
                .iter()
                .map(|(kx, ky, phase, amp)| {
                    amp * (std::f64::consts::TAU * (kx * x + ky * y) + phase).sin()
                })
                .sum::<f64>()
        })
        .collect()
}

fn noisy(rng: &mut Rng, field: &[f64], sigma: f64) -> Vec<f64> {
    field.iter().map(|one| one + sigma * rng.normal()).collect()
}

fn unsigned(words: &[i32]) -> Vec<u32> {
    words.iter().map(|one| one.cast_unsigned()).collect()
}

fn shot_json(planes: &[Vec<i32>]) -> String {
    format!(
        "[{}]",
        planes
            .iter()
            .map(|one| listed(&unsigned(one)))
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn learn_args(side: usize, shots: &[Vec<Vec<i32>>], more: &str) -> String {
    let sequence = shots
        .iter()
        .map(|one| shot_json(one))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\"stencil\":\"3x3\",\"reads\":[{{\"plane\":0,\"bit\":0}}],\"writes\":[{{\"plane\":1,\"bit\":0}}],\"border\":\"wrapped\",\"rule\":\"linear\",\"places\":\"von_neumann\",\"examples\":[{{\"size\":[{side},{side},1],\"sequence\":[{sequence}]}}],\"budget_ms\":{LONG}{more}}}"
    )
}

fn run_law(
    answer: &Json,
    side: usize,
    start: &[Vec<i32>],
    count: u32,
    params: Option<&[u32]>,
) -> Vec<Vec<i32>> {
    let law = grit::law::Law::compile(&text_at(answer, "law")).unwrap();
    let params: Vec<u32> =
        params.map_or_else(|| ints(answer.get("params").unwrap()), <[u32]>::to_vec);
    let grid = Grid3::new(side as u32, side as u32, 2).unwrap();
    let cells = side * side;
    let channels = law.channels() as usize;
    let pairs = law.pairs().to_vec();
    let mut world = vec![0_u32; grid.cells() * channels];
    for (read, plane) in start.iter().enumerate() {
        let target = pairs[read][0] as usize;
        for layer in 0..2 {
            for (at, one) in plane.iter().enumerate() {
                world[target * grid.cells() + layer * cells + at] = one.cast_unsigned();
            }
        }
    }
    for frame in 0..count {
        world = reference(&law, grid, frame, &world, &params);
    }
    (0..start.len())
        .map(|read| {
            let plane = pairs[read][usize::from(!count.is_multiple_of(2))] as usize;
            world[plane * grid.cells()..plane * grid.cells() + cells]
                .iter()
                .map(|one| one.cast_signed())
                .collect()
        })
        .collect()
}

fn rms(left: &[f64], right: &[f64]) -> f64 {
    (left
        .iter()
        .zip(right)
        .map(|(one, other)| (one - other) * (one - other))
        .sum::<f64>()
        / left.len() as f64)
        .sqrt()
}

fn reals(words: &[i32], frac: u32) -> Vec<f64> {
    let unit = f64::from(1_u32 << frac);
    words.iter().map(|one| f64::from(*one) / unit).collect()
}

#[test]
fn v1_a_drift_law_fitted_through_k_steps_rolls_out_within_one_and_a_half_of_the_true_law() {
    let side = 64;
    let frac = 16;
    for steps in [1_u32, 4, 8] {
        for sigma in [0.005, 0.01] {
            let mut rng = Rng(100 + u64::from(steps) * 10 + (sigma * 1000.0) as u64);
            let mut clean = smooth_field(&mut rng, side);
            let shots_count = 12;
            let mut shots = Vec::new();
            for _ in 0..shots_count {
                shots.push(vec![words(&noisy(&mut rng, &clean, sigma), frac)]);
                for _ in 0..steps {
                    clean = drift_step(side, &clean);
                }
            }
            let mut server = Server::new(None);
            let began = Instant::now();
            let got = heard(
                &mut server,
                "grit_law_learn",
                &learn_args(
                    side,
                    &shots,
                    &format!(
                        ",\"steps\":{steps},\"frac\":{frac},\"range\":[0,{}]",
                        1 << frac
                    ),
                ),
            );
            let answer = result(&got, false);
            let spent = began.elapsed();
            let held_clean = smooth_field(&mut rng, side);
            let start = words(&noisy(&mut rng, &held_clean, sigma), frac);
            let start_real = reals(&start, frac);
            let mut truth = held_clean.clone();
            let mut from_start = start_real.clone();
            let mut learned_error = 0.0;
            let mut true_error = 0.0;
            let mut count = 0.0;
            let mut current = vec![start.clone()];
            for _ in 0..(64 / steps) {
                for _ in 0..steps {
                    truth = drift_step(side, &truth);
                    from_start = drift_step(side, &from_start);
                }
                current = run_law(&answer, side, &current, steps, None);
                let learned = reals(&current[0], frac);
                learned_error += rms(&learned, &truth).powi(2);
                true_error += rms(&from_start, &truth).powi(2);
                count += 1.0;
            }
            let learned_error = (learned_error / count).sqrt();
            let true_error = (true_error / count).sqrt();
            let coefficients = answer.get("coefficients").unwrap().list().unwrap()[0]
                .list()
                .unwrap();
            let fitted: Vec<f64> = coefficients
                .iter()
                .map(|one| real(one.get("fit").unwrap()))
                .collect();
            let wanted = [
                DRIFT.south,
                DRIFT.west,
                DRIFT.centre,
                DRIFT.east,
                DRIFT.north,
            ];
            let worst = fitted
                .iter()
                .zip(wanted)
                .map(|(one, other)| (one - other).abs())
                .fold(0.0, f64::max);
            let bias = real(
                answer.get("bias").unwrap().list().unwrap()[0]
                    .get("fit")
                    .unwrap(),
            );
            println!(
                "v1 k {steps} sigma {sigma}: learned rollout rms {learned_error:.3e}, true law from the same start {true_error:.3e}, ratio {:.3}; coefficients {fitted:.5?} worst error {worst:.2e}, bias {bias:.6} (true {}); coefficient_frac {:?}; fit {:.2?}",
                learned_error / true_error,
                DRIFT.bias,
                written(
                    answer
                        .get("fixed")
                        .unwrap()
                        .get("coefficient_frac")
                        .unwrap()
                ),
                spent
            );
            assert!(
                learned_error <= 1.5 * true_error,
                "k {steps} sigma {sigma}: {learned_error} against {true_error}"
            );
        }
    }
}

const Q: i64 = 1 << 16;

const LIFT: f64 = 0.5;

struct Medium {
    side: usize,
    v: Vec<i64>,
    w: Vec<i64>,
}

fn q(value: f64) -> i64 {
    (value * Q as f64).round() as i64
}

fn qmul(left: i64, right: i64) -> i64 {
    (left * right + Q / 2).div_euclid(Q)
}

impl Medium {
    fn spiral(side: usize) -> Self {
        let mut medium = Self {
            side,
            v: vec![0; side * side],
            w: vec![0; side * side],
        };
        for y in 0..side {
            for x in 0..4 {
                medium.v[y * side + x] = Q;
            }
        }
        for at in 0..12_000 {
            medium.step();
            if at == 200 {
                for y in side / 2..side {
                    for x in 0..side {
                        medium.v[y * side + x] = 0;
                        medium.w[y * side + x] = 0;
                    }
                }
            }
        }
        medium
    }

    fn step(&mut self) {
        let side = self.side;
        let diffusion = q(0.2);
        let threshold = q(0.1);
        let rate = q(0.5);
        let recovery = q(0.003);
        let coupling = q(0.5);
        let mut v = self.v.clone();
        let mut w = self.w.clone();
        for y in 0..side as i64 {
            for x in 0..side as i64 {
                let at = torus(side, x, y);
                let centre = self.v[at];
                let around = self.v[torus(side, x + 1, y)]
                    + self.v[torus(side, x - 1, y)]
                    + self.v[torus(side, x, y + 1)]
                    + self.v[torus(side, x, y - 1)]
                    - 4 * centre;
                let reaction = qmul(rate, qmul(qmul(centre, centre - threshold), Q - centre));
                v[at] = centre + qmul(diffusion, around) + reaction - qmul(coupling, self.w[at]);
                w[at] = self.w[at] + qmul(recovery, centre - self.w[at]);
            }
        }
        self.v = v;
        self.w = w;
    }

    fn voltage(&self) -> Vec<f64> {
        self.v
            .iter()
            .map(|one| *one as f64 / Q as f64 + LIFT)
            .collect()
    }
}

fn nrmse(guess: &[f64], truth: &[f64]) -> f64 {
    let mean = truth.iter().sum::<f64>() / truth.len() as f64;
    let spread = (truth
        .iter()
        .map(|one| (one - mean) * (one - mean))
        .sum::<f64>()
        / truth.len() as f64)
        .sqrt();
    rms(guess, truth) / spread
}

fn rewords(words: &[i32], from: u32, to: u32) -> Vec<i32> {
    words
        .iter()
        .map(|one| {
            (f64::from(*one) * f64::from(1_u32 << to) / f64::from(1_u32 << from)).round() as i32
        })
        .collect()
}

struct Excited {
    answer: Json,
    spent: std::time::Duration,
    model: [f64; 2],
    persistence: [f64; 2],
    linear: [f64; 2],
}

fn linear_rolled(linear: &Json, side: usize, start: &[i32], ahead: u32) -> Vec<i32> {
    let text = text_at(linear, "law").replacen('{', "{\"alternate\":[0,1],", 1);
    let law = grit::law::Law::compile(&text).unwrap();
    let grid = Grid3::new(side as u32, side as u32, 2).unwrap();
    let mut world = vec![0_u32; grid.cells() * 2];
    for layer in 0..2 {
        for (at, one) in start.iter().enumerate() {
            world[layer * side * side + at] = one.cast_unsigned();
        }
    }
    for frame in 0..ahead {
        world = reference(&law, grid, frame, &world, &[]);
    }
    let plane = usize::from(!ahead.is_multiple_of(2));
    world[plane * grid.cells()..plane * grid.cells() + side * side]
        .iter()
        .map(|one| one.cast_signed())
        .collect()
}

fn excited(steps: u32, side: usize, shots_count: usize, settle: usize, seed: u64) -> Excited {
    let frac = 10;
    let sigma = 0.01;
    let mut rng = Rng(seed);
    let mut medium = Medium::spiral(side);
    let mut truth = Vec::new();
    let warm = 24;
    let total = shots_count + warm + 60;
    for _ in 0..total {
        truth.push(medium.voltage());
        for _ in 0..steps {
            medium.step();
        }
    }
    let seen: Vec<Vec<i32>> = truth
        .iter()
        .map(|one| words(&noisy(&mut rng, one, sigma), frac))
        .collect();
    let training: Vec<Vec<Vec<i32>>> = seen[..shots_count]
        .iter()
        .map(|one| vec![one.clone()])
        .collect();
    let range = format!("[{},{}]", -(1 << frac) / 2, 2 << frac);
    let mut server = Server::new(None);
    let began = Instant::now();
    let got = heard(
        &mut server,
        "grit_law_learn",
        &learn_args(
            side,
            &training,
            &format!(
                ",\"steps\":{steps},\"hidden\":1,\"poly\":3,\"frac\":{frac},\"range\":{range},\"settle\":{settle},\"law_frac\":\"auto\""
            ),
        ),
    );
    let answer = result(&got, false);
    let spent = began.elapsed();
    let law_frac = u32::try_from(
        answer
            .get("fixed")
            .unwrap()
            .get("frac")
            .unwrap()
            .int()
            .unwrap(),
    )
    .unwrap();
    let scales = written(
        answer
            .get("fixed")
            .unwrap()
            .get("coefficient_frac")
            .unwrap(),
    );
    let window: Vec<Vec<Vec<i32>>> = seen[shots_count..shots_count + warm]
        .iter()
        .map(|one| vec![rewords(one, frac, law_frac)])
        .collect();
    let law_range = format!("[{},{}]", -(1 << law_frac) / 2, 2 << law_frac);
    let kept = heard(
        &mut server,
        "grit_law_learn",
        &learn_args(
            side,
            &window,
            &format!(
                ",\"steps\":{steps},\"hidden\":1,\"poly\":3,\"frac\":{law_frac},\"range\":{law_range},\"params\":{},\"coefficient_frac\":{scales}",
                written(answer.get("params").unwrap())
            ),
        ),
    );
    let kept = result(&kept, false);
    let state: Vec<Vec<i32>> = kept.get("state").unwrap().list().unwrap()[0]
        .list()
        .unwrap()
        .iter()
        .map(signed)
        .collect();
    let pairs: Vec<String> = seen[..shots_count]
        .windows(2)
        .take(6)
        .map(|pair| {
            format!(
                "{{\"size\":[{side},{side},1],\"input\":[{}],\"output\":[{}]}}",
                listed(&unsigned(&pair[0])),
                listed(&unsigned(&pair[1]))
            )
        })
        .collect();
    let linear_args = format!(
        "{{\"stencil\":\"3x3\",\"reads\":[{{\"plane\":0,\"bit\":0}}],\"writes\":[{{\"plane\":1,\"bit\":0}}],\"border\":\"wrapped\",\"rule\":\"linear\",\"places\":\"von_neumann\",\"frac\":{frac},\"clamp\":{range},\"range\":{range},\"examples\":[{}]}}",
        pairs.join(",")
    );
    let linear = result(&heard(&mut server, "grit_law_learn", &linear_args), false);
    let start_at = shots_count + warm - 1;
    let mut model = [0.0; 2];
    let mut persistence = [0.0; 2];
    let mut linear_scores = [0.0; 2];
    for (slot, ahead) in [10_usize, 50].into_iter().enumerate() {
        let target = &truth[start_at + ahead];
        let rolled = run_law(&answer, side, &state, steps * ahead as u32, None);
        model[slot] = nrmse(&reals(&rolled[0], law_frac), target);
        persistence[slot] = nrmse(&reals(&seen[start_at], frac), target);
        let guess = linear_rolled(&linear, side, &seen[start_at], ahead as u32);
        linear_scores[slot] = nrmse(&reals(&guess, frac), target);
    }
    Excited {
        answer,
        spent,
        model,
        persistence,
        linear: linear_scores,
    }
}

#[test]
fn v2_a_hidden_recovery_plane_predicts_held_out_voltage_past_both_baselines() {
    let mut failed = Vec::new();
    for steps in steps_wanted() {
        let found = excited(steps, 64, 32, 8, 900 + u64::from(steps));
        let baselines = found.answer.get("baselines").unwrap();
        println!(
            "v2 k {steps}: held-out NRMSE 10/50 law {:.3}/{:.3}, persistence {:.3}/{:.3}, linear {:.3}/{:.3}; in-sample law {} persistence {} linear {} beats {}; frac {}; coefficient_frac {}; stages {}; work {}; fit {:.2?}",
            found.model[0],
            found.model[1],
            found.persistence[0],
            found.persistence[1],
            found.linear[0],
            found.linear[1],
            written(baselines.get("law").unwrap()),
            written(baselines.get("persistence").unwrap()),
            written(baselines.get("linear").unwrap()),
            written(baselines.get("beats").unwrap()),
            written(found.answer.get("fixed").unwrap().get("frac").unwrap()),
            written(
                found
                    .answer
                    .get("fixed")
                    .unwrap()
                    .get("coefficient_frac")
                    .unwrap()
            ),
            written(found.answer.get("stages").unwrap()),
            written(found.answer.get("work").unwrap()),
            found.spent
        );
        for slot in 0..2 {
            let better = found.persistence[slot].min(found.linear[slot]);
            if found.model[slot] > MARGIN_HELD * better {
                failed.push(format!(
                    "k {steps} horizon {}: law {} against baselines {} and {}",
                    [10, 50][slot],
                    found.model[slot],
                    found.persistence[slot],
                    found.linear[slot]
                ));
            }
        }
        if baselines.get("beats") != Some(&Json::Bool(true)) {
            failed.push(format!("k {steps}: the answer does not say beats"));
        }
    }
    assert!(failed.is_empty(), "{failed:?}");
}

const MARGIN_HELD: f64 = 0.5;

fn steps_wanted() -> Vec<u32> {
    std::env::var("RV2_STEPS").map_or_else(
        |_| vec![8, 4, 1],
        |held| held.split(',').map(|one| one.parse().unwrap()).collect(),
    )
}

fn drift_answer(steps: u32, shots_count: usize, sigma: f64, seed: u64) -> Json {
    let side = 64;
    let frac = 16;
    let mut rng = Rng(seed);
    let mut clean = smooth_field(&mut rng, side);
    let mut shots = Vec::new();
    for _ in 0..shots_count {
        shots.push(vec![words(&noisy(&mut rng, &clean, sigma), frac)]);
        for _ in 0..steps {
            clean = drift_step(side, &clean);
        }
    }
    let mut server = Server::new(None);
    result(
        &heard(
            &mut server,
            "grit_law_learn",
            &learn_args(
                side,
                &shots,
                &format!(
                    ",\"steps\":{steps},\"frac\":{frac},\"range\":[0,{}]",
                    1 << frac
                ),
            ),
        ),
        false,
    )
}

struct Posed {
    law: String,
    side: usize,
    cells: Vec<u32>,
    truth: Vec<f64>,
    target: Vec<u32>,
    frac: u32,
}

fn posed_of(answer: &Json, seed: u64) -> Posed {
    let side = 16;
    let frac = u32::try_from(
        answer
            .get("fixed")
            .unwrap()
            .get("frac")
            .unwrap()
            .int()
            .unwrap(),
    )
    .unwrap();
    let mut rng = Rng(seed);
    let clean = smooth_field(&mut rng, side);
    let mut truth = clean.clone();
    for _ in 0..64 {
        truth = drift_step(side, &truth);
    }
    let start = words(&clean, frac);
    let seen = words(&noisy(&mut rng, &truth, 0.002), frac);
    let mut cells = vec![0_u32; side * side * 2 * 2];
    for layer in 0..2 {
        for (at, one) in start.iter().enumerate() {
            cells[layer * side * side + at] = one.cast_unsigned();
        }
    }
    let target: Vec<u32> = (0..2)
        .flat_map(|_| seen.iter().map(|one| one.cast_unsigned()))
        .collect();
    Posed {
        law: text_at(answer, "law"),
        side,
        cells,
        truth,
        target,
        frac,
    }
}

fn posed_args(posed: &Posed, params: &[u32], more: &str) -> String {
    let side = posed.side;
    format!(
        "{{\"law\":{},\"size\":[{side},{side},2],\"cells\":{},\"params\":{},\"frames\":64,\"aim\":{{\"from\":[0,0,0],\"to\":[{},{},1],\"target\":{}}},\"budget_ms\":{LONG}{more}}}",
        written(&Json::Text(posed.law.clone())),
        listed(&posed.cells),
        listed(params),
        side - 1,
        side - 1,
        listed(&posed.target)
    )
}

fn rollout_error(posed: &Posed, params: &[u32]) -> f64 {
    let law = grit::law::Law::compile(&posed.law).unwrap();
    let side = posed.side;
    let grid = Grid3::new(side as u32, side as u32, 2).unwrap();
    let mut world = posed.cells.clone();
    for frame in 0..64 {
        world = reference(&law, grid, frame, &world, params);
    }
    let got: Vec<i32> = world[..side * side]
        .iter()
        .map(|one| one.cast_signed())
        .collect();
    rms(&reals(&got, posed.frac), &posed.truth)
}

fn objective(answer: &Json) -> u128 {
    match answer.get("objective").unwrap() {
        Json::Int(value) => u128::try_from(*value).unwrap(),
        other => other.text().unwrap().parse().unwrap(),
    }
}

#[test]
fn v3_the_answer_law_carries_params_that_learn_params_and_descend_refine_with_no_text_edit() {
    let answer = drift_answer(1, 4, 0.02, 31);
    let params = ints(answer.get("params").unwrap());
    let posed = posed_of(&answer, 32);
    let before = rollout_error(&posed, &params);
    let mut server = Server::new(None);
    let scored = result(
        &heard(
            &mut server,
            "grit_law_score",
            &posed_args(&posed, &params, ""),
        ),
        false,
    );
    let start = objective(&scored);
    let bounds: Vec<String> = params
        .iter()
        .map(|one| format!("[{},{}]", one - 4096, one + 4096))
        .collect();
    let searched = result(
        &heard(
            &mut server,
            "grit_learn_params",
            &posed_args(
                &posed,
                &params,
                &format!(
                    ",\"bounds\":[{}],\"seed\":7,\"population\":8,\"generations\":40",
                    bounds.join(",")
                ),
            ),
        ),
        false,
    );
    let searched_params = ints(searched.get("params").unwrap());
    let searched_error = rollout_error(&posed, &searched_params);
    let descended = result(
        &heard(
            &mut server,
            "grit_law_descend",
            &posed_args(&posed, &params, ",\"steps\":24"),
        ),
        false,
    );
    let descended_params = ints(descended.get("params").unwrap());
    let descended_error = rollout_error(&posed, &descended_params);
    println!(
        "v3 nodes {}: objective {start} -> learn_params {} -> descend {} ({}); rollout rms against the truth {before:.3e} -> {searched_error:.3e} (learn_params) and {descended_error:.3e} (descend); params {params:?} -> {searched_params:?} / {descended_params:?}",
        written(answer.get("nodes").unwrap()),
        objective(&searched),
        objective(&descended),
        text_at(&descended, "stopped")
    );
    assert!(objective(&searched) < start);
    assert!(objective(&descended) < start);
    assert!(searched_error < before);
    assert!(descended_error < before);
}

#[test]
fn v3_the_kept_walk_gradient_of_an_iterating_law_matches_its_own_differences() {
    let answer = drift_answer(1, 4, 0.02, 33);
    let params = ints(answer.get("params").unwrap());
    let mut posed = posed_of(&answer, 34);
    let lift = 1_u32 << (posed.frac - 2);
    for one in &mut posed.target {
        *one = one.wrapping_add(lift);
    }
    let mut server = Server::new(None);
    let graded = result(
        &heard(
            &mut server,
            "grit_law_gradient",
            &posed_args(&posed, &params, ""),
        ),
        false,
    );
    let gradient: Vec<f64> = graded
        .get("gradient")
        .unwrap()
        .list()
        .unwrap()
        .iter()
        .map(|one| one.text().unwrap().parse::<f64>().unwrap() / 2_f64.powi(64))
        .collect();
    let base = objective(&graded) as f64;
    let mut checked = 0;
    for (at, slope) in gradient.iter().enumerate() {
        if slope.abs() < 1.0 {
            continue;
        }
        let step = 64_u32;
        let mut moved = params.clone();
        moved[at] += step;
        let up = objective(&result(
            &heard(
                &mut server,
                "grit_law_score",
                &posed_args(&posed, &moved, ""),
            ),
            false,
        )) as f64;
        moved[at] -= 2 * step;
        let down = objective(&result(
            &heard(
                &mut server,
                "grit_law_score",
                &posed_args(&posed, &moved, ""),
            ),
            false,
        )) as f64;
        let difference = (up - down) / f64::from(2 * step);
        println!(
            "v3 param {at}: gradient {slope:.4e}, central difference {difference:.4e}, objective {base}"
        );
        assert!(difference.signum() == slope.signum(), "param {at}");
        assert!(
            (difference - slope).abs() <= 0.25 * slope.abs(),
            "param {at}: {difference} against {slope}"
        );
        checked += 1;
    }
    assert!(checked >= 3);
}

fn learn_line(side: usize, shots: &[Vec<Vec<i32>>], more: &str) -> Json {
    let mut server = Server::new(None);
    heard(
        &mut server,
        "grit_law_learn",
        &learn_args(side, shots, more),
    )
}

fn drift_shots(side: usize, frac: u32, count: usize, seed: u64) -> Vec<Vec<Vec<i32>>> {
    let mut rng = Rng(seed);
    let mut clean = smooth_field(&mut rng, side);
    let mut shots = Vec::new();
    for _ in 0..count {
        shots.push(vec![words(&noisy(&mut rng, &clean, 0.005), frac)]);
        clean = drift_step(side, &clean);
    }
    shots
}

#[test]
fn v4_automatic_frac_keeps_twelve_coefficient_bits_or_refuses_and_a_low_frac_is_named() {
    let side = 32;
    let shots = drift_shots(side, 16, 6, 41);
    let automatic = result(
        &learn_line(
            side,
            &shots,
            ",\"steps\":2,\"frac\":16,\"range\":[-6554,72090],\"law_frac\":\"auto\"",
        ),
        false,
    );
    let fixed = automatic.get("fixed").unwrap();
    let scales = ints(fixed.get("coefficient_frac").unwrap());
    println!(
        "v4 auto: frac {}, coefficient_frac {scales:?}, from {}",
        written(fixed.get("frac").unwrap()),
        written(fixed.get("frac_from").unwrap())
    );
    assert!(scales.iter().all(|one| *one >= 12));
    assert_eq!(text_at(fixed, "frac_from"), "auto");
    assert!(automatic.get("warnings").is_none());
    let low = result(
        &learn_line(
            side,
            &shots,
            ",\"steps\":2,\"frac\":16,\"range\":[-6554,72090],\"law_frac\":26",
        ),
        false,
    );
    let scales = ints(low.get("fixed").unwrap().get("coefficient_frac").unwrap());
    let warnings = low.get("warnings").unwrap().list().unwrap();
    println!(
        "v4 law_frac 26: coefficient_frac {scales:?}, warnings {}",
        written(low.get("warnings").unwrap())
    );
    assert!(scales.iter().any(|one| *one < 12));
    assert_eq!(text_at(&warnings[0], "code"), "coefficient_bits");
    let coarse = drift_shots(side, 4, 6, 43);
    let wide = learn_line(
        side,
        &coarse,
        ",\"steps\":2,\"frac\":4,\"range\":[-1073741824,1073741824],\"law_frac\":\"auto\"",
    );
    let refused = result(&wide, true);
    println!("v4 wide range: {}", written(&refused));
    assert_eq!(text_at(&refused, "code"), "coefficient_bits");
    assert_eq!(text_at(&refused, "field"), "$.law_frac");
    let x1 = {
        let frac = 24;
        let pairs: Vec<String> = drift_shots(side, frac, 4, 42)
            .windows(2)
            .map(|pair| {
                format!(
                    "{{\"size\":[{side},{side},1],\"input\":[{}],\"output\":[{}]}}",
                    listed(&pair[0][0]),
                    listed(&pair[1][0])
                )
            })
            .collect();
        let args = format!(
            "{{\"stencil\":\"3x3\",\"reads\":[{{\"plane\":0,\"bit\":0}}],\"writes\":[{{\"plane\":1,\"bit\":0}}],\"border\":\"wrapped\",\"rule\":\"linear\",\"place_list\":[1,3,4,5,7],\"frac\":{frac},\"range\":[{},{}],\"examples\":[{}]}}",
            -(1 << frac) / 10,
            (1 << frac) * 11 / 10,
            pairs.join(",")
        );
        let mut server = Server::new(None);
        result(&heard(&mut server, "grit_law_learn", &args), false)
    };
    let scales = ints(x1.get("fixed").unwrap().get("coefficient_frac").unwrap());
    println!(
        "v4 X1 frac 24 one-step fit: coefficient_frac {scales:?}, warnings {}",
        written(x1.get("warnings").unwrap())
    );
    assert!(scales.iter().any(|one| *one < 12));
    assert_eq!(
        text_at(&x1.get("warnings").unwrap().list().unwrap()[0], "code"),
        "coefficient_bits"
    );
}

#[test]
fn v6_a_process_outside_the_class_is_reported_as_not_beating_the_baselines() {
    let side = 32;
    let frac = 12;
    let mut rng = Rng(61);
    let mut field: Vec<f64> = (0..side * side).map(|_| rng.unit()).collect();
    let mut shots = Vec::new();
    for _ in 0..12 {
        shots.push(vec![words(&noisy(&mut rng, &field, 0.01), frac)]);
        let mut moved = vec![0.0; field.len()];
        for y in 0..side as i64 {
            for x in 0..side as i64 {
                moved[torus(side, x, y)] = field[torus(side, x - 5, y + 3)];
            }
        }
        field = moved;
    }
    for hidden in [0, 1] {
        let answer = result(
            &learn_line(
                side,
                &shots,
                &format!(
                    ",\"steps\":1,\"hidden\":{hidden},\"poly\":2,\"frac\":{frac},\"range\":[-2048,6144]"
                ),
            ),
            false,
        );
        let baselines = answer.get("baselines").unwrap();
        println!(
            "v6 hidden {hidden}: {}; warnings {}",
            written(baselines),
            written(answer.get("warnings").unwrap())
        );
        assert_eq!(baselines.get("beats"), Some(&Json::Bool(false)));
        let warnings = answer.get("warnings").unwrap().list().unwrap();
        assert!(
            warnings
                .iter()
                .any(|one| text_at(one, "code") == "baselines_not_beaten")
        );
    }
}

#[test]
fn v2_a_hidden_plane_starts_at_zero_and_not_at_the_observed_plane() {
    let side = 8;
    let frac = 12;
    let scale = 16;
    let mut rng = Rng(71);
    let shots: Vec<Vec<Vec<i32>>> = (0..3)
        .map(|_| {
            vec![
                (0..side * side)
                    .map(|_| 1000 + (rng.unit() * 2000.0) as i32)
                    .collect(),
            ]
        })
        .collect();
    let half = 1_u64 << 31;
    let mut params = vec![half; 14];
    params[2] = half + (1 << scale);
    params[7 + 5] = half + (1 << scale);
    let answer = result(
        &learn_line(
            side,
            &shots,
            &format!(
                ",\"steps\":3,\"hidden\":1,\"frac\":{frac},\"range\":[-4096,8192],\"params\":{},\"coefficient_frac\":[{scale},{scale}]",
                listed(&params)
            ),
        ),
        false,
    );
    let state = answer.get("state").unwrap().list().unwrap()[0]
        .list()
        .unwrap();
    let observed = signed(&state[0]);
    let hidden = signed(&state[1]);
    println!(
        "v2 start rule: {}",
        text_at(answer.get("hidden").unwrap(), "start")
    );
    assert_eq!(observed, shots[2][0]);
    assert!(hidden.iter().all(|one| *one == 0), "{hidden:?}");
    assert_eq!(
        text_at(&answer, "method"),
        "params given: no fit; the hidden planes are run as start says"
    );
}

#[test]
fn v2_timing_of_a_hidden_fit_at_64_by_64_with_k_7_and_20_shots() {
    let found = excited(7, 64, 20, 5, 977);
    println!(
        "v2 timing: 64x64, k 7, 20 shots, settle 5: fit {:.2?}, work {} cell sub-steps; held-out NRMSE 10/50 law {:.3}/{:.3}, persistence {:.3}/{:.3}, linear {:.3}/{:.3}",
        found.spent,
        written(found.answer.get("work").unwrap()),
        found.model[0],
        found.model[1],
        found.persistence[0],
        found.persistence[1],
        found.linear[0],
        found.linear[1]
    );
    assert_eq!(found.answer.get("steps").and_then(Json::int), Some(7));
}

fn pair_examples(side: usize, frac: u32, seed: u64) -> String {
    let shots = drift_shots(side, frac, 4, seed);
    let pairs: Vec<String> = shots
        .windows(2)
        .map(|pair| {
            format!(
                "{{\"size\":[{side},{side},1],\"input\":[{}],\"output\":[{}]}}",
                listed(&pair[0][0]),
                listed(&pair[1][0])
            )
        })
        .collect();
    format!("[{}]", pairs.join(","))
}

fn soup(rng: &mut Rng, cells: usize) -> Vec<u32> {
    (0..cells).map(|_| u32::from(rng.unit() < 0.35)).collect()
}

fn life_step(side: usize, cells: &[u32]) -> Vec<u32> {
    let mut out = vec![0; cells.len()];
    for y in 0..side as i64 {
        for x in 0..side as i64 {
            let mut around = 0;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    if dx != 0 || dy != 0 {
                        around += cells[torus(side, x + dx, y + dy)];
                    }
                }
            }
            let centre = cells[torus(side, x, y)];
            out[torus(side, x, y)] = u32::from(around == 3 || (centre == 1 && around == 2));
        }
    }
    out
}

fn base_requests() -> Vec<(&'static str, &'static str, String)> {
    let heat = pair_examples(32, 16, 51);
    let fine = pair_examples(16, 12, 52);
    let mut rng = Rng(53);
    let side = 16;
    let life_pairs: Vec<String> = (0..8)
        .map(|_| {
            let input = soup(&mut rng, side * side);
            let mut output = input.clone();
            for _ in 0..2 {
                output = life_step(side, &output);
            }
            format!(
                "{{\"size\":[{side},{side},1],\"input\":[{}],\"output\":[{}]}}",
                listed(&input),
                listed(&output)
            )
        })
        .collect();
    let one = "{\"stencil\":\"3x3\",\"reads\":[{\"plane\":0,\"bit\":0}],\"writes\":[{\"plane\":1,\"bit\":0}],\"border\":\"wrapped\"";
    let gradient_law = "{\"law\":1,\"name\":\"plain\",\"params\":1,\"channels\":1,\"halo\":0,\"tally\":{\"counts\":1,\"weighted\":false},\"consts\":{},\"defs\":{},\"step\":[\"let\",[[\"out\",[\"map\",\"c\",[\"range\",8],[\"mul\",[\"at\",\"cells\",\"c\"],[\"param\",0]]]]],[\"vec\",\"out\",[\"vec\",0]]]}";
    vec![
        (
            "rv1_heat",
            "grit_law_learn",
            format!(
                "{one},\"rule\":\"linear\",\"places\":\"von_neumann\",\"frac\":16,\"examples\":{heat}}}"
            ),
        ),
        (
            "rv1_poly_clamp",
            "grit_law_learn",
            format!(
                "{one},\"rule\":\"linear\",\"poly\":2,\"frac\":12,\"clamp\":[-4096,8192],\"range\":[-4096,8192],\"examples\":{fine}}}"
            ),
        ),
        (
            "rv1_moore",
            "grit_law_learn",
            format!("{one},\"rule\":\"linear\",\"frac\":12,\"examples\":{fine}}}"),
        ),
        (
            "life_steps",
            "grit_law_learn",
            format!(
                "{one},\"rule\":\"outer\",\"steps\":2,\"examples\":[{}]}}",
                life_pairs.join(",")
            ),
        ),
        (
            "not_reversible",
            "grit_law_gradient",
            format!(
                "{{\"law\":{},\"size\":[4,4,4],\"cells\":{},\"params\":[3],\"frames\":2,\"aim\":{{\"from\":[0,0,0],\"to\":[3,3,3],\"target\":{}}}}}",
                written(&Json::Text(gradient_law.to_owned())),
                listed(&vec![1; 64]),
                listed(&vec![2; 64])
            ),
        ),
    ]
}

fn sha_of(held: &Json) -> String {
    grit::law::hex(&grit::branch::sha256(written(held).as_bytes()))
}

const BASE_SHA: [(&str, &str); 5] = [
    (
        "rv1_heat",
        "e393c55b6948457c1ea4eaf2aadc2ac329808d77b79b278c41f1026b7a0fbd6d",
    ),
    (
        "rv1_poly_clamp",
        "8c56e052f2842763c03157989aee31cec08c63dc49de7b8c078df3af257138ea",
    ),
    (
        "rv1_moore",
        "fb370697881b12af975f298492ff7f964dda3e4a1d6ac69068fee4982218185b",
    ),
    (
        "life_steps",
        "e54e67c6bab395a1a885205a71c680ae18438ff79292a86d14733a6a26e4a389",
    ),
    (
        "not_reversible",
        "47fc9afebaaa6e0d58c88e719988f0b0cdbdd74a16fa48ce258a9e21079b03ac",
    ),
];

#[test]
fn v5_requests_without_steps_or_hidden_answer_the_bytes_recorded_on_the_base() {
    let mut wrong = Vec::new();
    for (name, tool, arguments) in base_requests() {
        let mut server = Server::new(None);
        let got = heard(&mut server, tool, &arguments);
        let sha = sha_of(&got);
        eprintln!("BASE {name} {sha}");
        let pinned = BASE_SHA
            .iter()
            .find(|(one, _)| *one == name)
            .map(|(_, sha)| *sha)
            .unwrap();
        if sha != pinned {
            wrong.push(format!(
                "{name} {sha} {}",
                &written(&got)[..written(&got).len().min(300)]
            ));
        }
    }
    assert!(wrong.is_empty(), "{wrong:?}");
}

fn world_words(server: &mut Server<'_>, world: i64, plane: u32, side: usize) -> Vec<u32> {
    let read = result(
        &heard(
            server,
            "grit_world_read",
            &format!(
                "{{\"world\":{world},\"plane\":{plane},\"from\":[0,0,0],\"to\":[{},{},1]}}",
                side - 1,
                side - 1
            ),
        ),
        false,
    );
    ints(read.get("cells").unwrap())
}

#[test]
fn v3_a_two_pair_answer_law_steps_on_the_device_as_its_cpu_twin() {
    let Some(host) = common::device("two pair law on the device") else {
        return;
    };
    let side = 16;
    let scale = 16;
    let half = 1_u64 << 31;
    let at = |value: f64| (half as i64 + (value * f64::from(1_u32 << scale)).round() as i64) as u64;
    let mut params = vec![half; 14];
    for (term, value) in [(0, 0.1), (1, 0.1), (2, 0.6), (3, 0.1), (4, 0.1), (5, -0.1)] {
        params[term] = at(value);
    }
    params[7 + 2] = at(0.05);
    params[7 + 5] = at(0.9);
    params[13] = at(0.0001);
    let mut rng = Rng(81);
    let shots: Vec<Vec<Vec<i32>>> = (0..2)
        .map(|_| {
            vec![
                (0..side * side)
                    .map(|_| (rng.unit() * 4096.0) as i32)
                    .collect(),
            ]
        })
        .collect();
    let mut server = Server::new(Some(&host));
    let answer = result(
        &heard(
            &mut server,
            "grit_law_learn",
            &learn_args(
                side,
                &shots,
                &format!(
                    ",\"steps\":2,\"hidden\":1,\"frac\":12,\"range\":[-4096,8192],\"params\":{},\"coefficient_frac\":[{scale},{scale}]",
                    listed(&params)
                ),
            ),
        ),
        false,
    );
    let law_text = text_at(&answer, "law");
    let law = grit::law::Law::compile(&law_text).unwrap();
    assert_eq!(law.pairs(), &[[0, 1], [2, 3]]);
    let params32: Vec<u32> = params
        .iter()
        .map(|one| u32::try_from(*one).unwrap())
        .collect();
    let created = result(
        &heard(
            &mut server,
            "grit_world_create",
            &format!(
                "{{\"wide\":{side},\"deep\":{side},\"tall\":2,\"law\":{},\"params\":{}}}",
                written(&Json::Text(law_text.clone())),
                listed(&params32)
            ),
        ),
        false,
    );
    let world = created.get("world").and_then(Json::int).unwrap();
    let grid = Grid3::new(side as u32, side as u32, 2).unwrap();
    let mut cpu = vec![0_u32; grid.cells() * 4];
    for plane in [0_u32, 2] {
        let words: Vec<u32> = (0..grid.cells())
            .map(|_| (rng.unit() * 3000.0) as u32)
            .collect();
        let shown: Vec<String> = words
            .iter()
            .enumerate()
            .map(|(index, one)| {
                format!(
                    "[{},{},{},{one}]",
                    index % side,
                    (index / side) % side,
                    index / (side * side)
                )
            })
            .collect();
        result(
            &heard(
                &mut server,
                "grit_world_write",
                &format!(
                    "{{\"world\":{world},\"plane\":{plane},\"cells\":[{}]}}",
                    shown.join(",")
                ),
            ),
            false,
        );
        cpu[plane as usize * grid.cells()..(plane as usize + 1) * grid.cells()]
            .copy_from_slice(&words);
    }
    let frames = 7;
    result(
        &heard(
            &mut server,
            "grit_world_step",
            &format!("{{\"world\":{world},\"frames\":{frames}}}"),
        ),
        false,
    );
    for frame in 0..frames {
        cpu = reference(&law, grid, frame, &cpu, &params32);
    }
    for plane in 0..4_u32 {
        let device = world_words(&mut server, world, plane, side);
        let twin = &cpu[plane as usize * grid.cells()..(plane as usize + 1) * grid.cells()];
        assert_eq!(device, twin, "plane {plane}");
    }
    println!(
        "v3 device: a two-pair law of {} nodes steps {frames} frames on the device equal to its CPU twin on all 4 planes",
        law.nodes()
    );
}
