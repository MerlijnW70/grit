pub mod fit;
pub mod fixed;
pub mod model;

use crate::law::Law as Compiled;
use crate::stencil::{self, Border, Stencil};
use crate::wire::{Json, counted, map, text};

use super::Failed;
use super::budget::Stop;
use super::errors::{Code, Refusal};
use super::gradient::Told;
use super::law_real::{self, Fixed, Term};
use super::tools::{int, ints, limited, refused, unread};

use fit::{Clip, Run, Setup, Stage};
use fixed::{BITS_LEAST, Text, Words};
use model::{Basis, Grid, Planes};

pub const STEPS_MOST: u32 = 64;

pub const HIDDEN_MOST: u32 = 1;

pub const HORIZON_MOST: usize = 64;

pub const FIT_WORK_MOST: u64 = 1 << 27;

pub const MARGIN: f64 = 0.8;

pub const START: &str = "each hidden plane starts at 0 on an example's first shot and is run by the law over the example's shots, with the observed planes interpolated linearly across the k sub-steps of each shot; a rollout from shot t starts from the hidden planes so run up to t and from observed words fitted so the rollout meets the shots (scored also against shot t itself); state is the last shot's observed words with the hidden planes so run to it";

pub const METHOD: &str = "the observed reads' one-shot least squares fit, split into k equal sub-steps (each coefficient's distance from the identity divided by k), with each hidden plane started as a leaky copy of the observed centre less its mean over the examples, fed back into it, the 3 best of 12 such starts on one-shot rollouts each fitted at horizon 1 and the lowest kept; then Levenberg-Marquardt on the f64 form of the law (no rounding), whose residuals are the observed planes of rollouts of 1, 2, 4 and up to 64 shots of k sub-steps each, from every shot, the hidden planes run as start says; the Jacobian is carried forward through every sub-step exactly, so the fit runs on the host and needs no device; the hidden planes are then scaled so their run spans half the range, and the coefficients rounded to k fractional bits";

pub const SCALES: &str = "two families of hidden planes give the same observed planes: scale, hidden plane h times any a other than 0 with every coefficient times a to the power scales_as[h]; and offset, hidden plane h plus any c, which moves the coefficients and biases whose offset_moves[h] is true (each bias of a write the hidden centre enters, the hidden write's own bias, and the read centre's coefficient in a write where that read times the hidden centre enters); a coefficient with scales_as 0 and offset_moves false for every hidden plane is pinned by the examples, the others only in combinations both families keep, such as the coefficient of a hidden centre in a read write times the read centre's coefficient in that hidden write; the fit fixes the scale by scaling the hidden run to half the range and the offset by the start at 0, so these are a choice, not a finding";

pub struct Request<'a> {
    pub stencil: Stencil,
    pub reads: &'a [u32],
    pub writes: &'a [u32],
    pub mask: u32,
    pub poly: u32,
    pub frac: u32,
    pub range: Option<[i32; 2]>,
    pub border: Border,
    pub name: &'a str,
    pub channels: Option<u32>,
}

struct Example {
    size: [u32; 3],
    shots: Vec<Vec<Vec<u32>>>,
}

fn example_refused(at: usize, key: &str, message: String) -> Failed {
    let field = if key.is_empty() {
        format!("$.examples[{at}]")
    } else {
        format!("$.examples[{at}].{key}")
    };
    refused(Code::Example, &field, message)
}

fn examples_of(
    held: &Json,
    reads: usize,
    border: Border,
    stencil: Stencil,
) -> Result<Vec<Example>, Failed> {
    let given_list = held
        .get("examples")
        .and_then(Json::list)
        .ok_or_else(|| unread("examples"))?;
    let mut out = Vec::with_capacity(given_list.len());
    let mut total = 0_u64;
    for (at, one) in given_list.iter().enumerate() {
        if one.get("world").is_some() {
            return Err(example_refused(
                at,
                "world",
                "under steps or hidden every example is inline: size with input and output, or size with sequence".to_owned(),
            ));
        }
        let size = ints(one.get("size").ok_or_else(|| unread("size"))?, "size")?;
        let size = <[u32; 3]>::try_from(size.as_slice()).map_err(|_| unread("size"))?;
        let cells = size.iter().map(|side| u64::from(*side)).product::<u64>();
        if border == Border::Wrapped {
            for (axis, side) in size.iter().enumerate().take(stencil.axes()) {
                if *side < 2 || !side.is_power_of_two() {
                    return Err(example_refused(
                        at,
                        "size",
                        format!(
                            "side {axis} is {side}; a wrapped example runs alike only on a world of its own size, so each side the stencil reads across must be a power of two from 2"
                        ),
                    ));
                }
            }
        }
        let shot_of = |words: &Json, key: &str| -> Result<Vec<Vec<u32>>, Failed> {
            let lists = words
                .list()
                .ok_or_else(|| example_refused(at, key, format!("{key} holds lists of words")))?;
            if lists.len() != reads {
                return Err(example_refused(
                    at,
                    key,
                    format!(
                        "{key} holds {} lists of words, and {reads} were wanted, one for each read",
                        lists.len()
                    ),
                ));
            }
            lists
                .iter()
                .map(|list| {
                    let words = ints(list, key)?;
                    if words.len() as u64 != cells {
                        return Err(example_refused(
                            at,
                            key,
                            format!(
                                "a list holds {} words, and the example's {} by {} by {} cells need {cells}",
                                words.len(),
                                size[0],
                                size[1],
                                size[2]
                            ),
                        ));
                    }
                    Ok(words)
                })
                .collect()
        };
        let shots = match (one.get("sequence"), one.get("input"), one.get("output")) {
            (Some(sequence), None, None) => {
                let given_list = sequence.list().ok_or_else(|| {
                    example_refused(at, "sequence", "sequence holds shots".to_owned())
                })?;
                if given_list.len() < 2 {
                    return Err(example_refused(
                        at,
                        "sequence",
                        format!(
                            "sequence holds {} shots, and a fit needs at least 2",
                            given_list.len()
                        ),
                    ));
                }
                given_list
                    .iter()
                    .map(|shot| shot_of(shot, "sequence"))
                    .collect::<Result<Vec<_>, Failed>>()?
            }
            (None, Some(input), Some(output)) => {
                vec![shot_of(input, "input")?, shot_of(output, "output")?]
            }
            _ => {
                return Err(example_refused(
                    at,
                    "",
                    "under steps or hidden an example has size with input and output, or size with sequence, not both".to_owned(),
                ));
            }
        };
        total = total.saturating_add(cells.saturating_mul(shots.len() as u64));
        if total > super::law_learn::CELLS_MOST {
            return Err(limited(
                Code::LearnLimit,
                &format!("$.examples[{at}]"),
                format!(
                    "the examples up to this one hold {total} cells over their shots, more than the {} a call takes",
                    super::law_learn::CELLS_MOST
                ),
                Json::Int(i64::try_from(super::law_learn::CELLS_MOST).unwrap_or(i64::MAX)),
            ));
        }
        out.push(Example { size, shots });
    }
    Ok(out)
}

fn scientific(value: f64) -> Json {
    text(&format!("{value:.9e}"))
}

fn term_shown(basis: &Basis, term: Term) -> Json {
    let read = |one: usize| counted(u32::try_from(one).unwrap_or(u32::MAX));
    match term {
        Term::Place { read: one, place } => {
            let [dx, dy, dz] = basis.stencil.offset(place);
            map(vec![
                ("term", text("place")),
                ("read", read(one)),
                ("place", counted(place)),
                (
                    "at",
                    Json::List(vec![Json::Int(dx), Json::Int(dy), Json::Int(dz)]),
                ),
            ])
        }
        Term::Pair { left, right } => map(vec![
            ("term", text("product")),
            ("reads", Json::List(vec![read(left), read(right)])),
        ]),
        Term::Spread { centre, around } => map(vec![
            ("term", text("centre_times_others")),
            ("reads", Json::List(vec![read(centre), read(around)])),
        ]),
        Term::Cube { left, right, third } => map(vec![
            ("term", text("cube")),
            (
                "reads",
                Json::List(vec![read(left), read(right), read(third)]),
            ),
        ]),
    }
}

struct Planned {
    basis: Basis,
    reads: Vec<u32>,
    writes: Vec<u32>,
    channels: u32,
    steps: u32,
    hidden: u32,
}

fn planned(request: &Request<'_>, held: &Json) -> Result<Planned, Failed> {
    let steps = match held.get("steps") {
        None => 1,
        Some(_) => int(held, "steps")?,
    };
    if steps == 0 {
        return Err(refused(
            Code::Malformed,
            "$.steps",
            "steps is 0, and a law is applied at least once between shots".to_owned(),
        ));
    }
    if steps > STEPS_MOST {
        return Err(limited(
            Code::StepsLimit,
            "$.steps",
            format!(
                "steps is {steps}, and rule linear fits a law applied at most {STEPS_MOST} times between shots"
            ),
            counted(STEPS_MOST),
        ));
    }
    let hidden = match held.get("hidden") {
        None => 0,
        Some(_) => int(held, "hidden")?,
    };
    if hidden > HIDDEN_MOST {
        return Err(limited(
            Code::LearnLimit,
            "$.hidden",
            format!("hidden is {hidden}, and rule linear fits at most {HIDDEN_MOST} hidden planes"),
            counted(HIDDEN_MOST),
        ));
    }
    if request.reads.len() != request.writes.len() {
        return Err(refused(
            Code::StepsWrites,
            "$.writes",
            format!(
                "under steps or hidden each write is the next value of the read at the same position, so a law with {} reads takes {} writes, and {} were given",
                request.reads.len(),
                request.reads.len(),
                request.writes.len()
            ),
        ));
    }
    let observed = request.reads.len();
    let needed = 2 * (observed + hidden as usize);
    let most = crate::law::limits::CHANNELS as usize;
    if needed > most {
        return Err(limited(
            Code::LearnLimit,
            "$.hidden",
            format!(
                "{observed} reads and {hidden} hidden planes, each with a plane it is written to, need {needed} planes, more than the {most} a law holds"
            ),
            counted(u32::try_from((most / 2).saturating_sub(observed)).unwrap_or(0)),
        ));
    }
    let mut free: Vec<u32> = (0..crate::law::limits::CHANNELS)
        .filter(|plane| !request.reads.contains(plane) && !request.writes.contains(plane))
        .collect();
    let mut reads = request.reads.to_vec();
    let mut writes = request.writes.to_vec();
    let hidden_reads: Vec<u32> = free.drain(..hidden as usize).collect();
    let hidden_writes: Vec<u32> = free.drain(..hidden as usize).collect();
    reads.extend(hidden_reads);
    writes.extend(hidden_writes);
    let highest = reads.iter().chain(&writes).copied().max().unwrap_or(0) + 1;
    let channels = request.channels.unwrap_or(highest).max(highest);
    let basis = Basis::new(
        request.stencil,
        request.mask,
        observed,
        hidden as usize,
        request.poly,
    );
    let params = basis.params();
    if params > crate::law::limits::PARAMS {
        return Err(limited(
            Code::LearnLimit,
            "$.poly",
            format!(
                "{} terms and a bias for each of {} writes make {params} params, more than the {} a law holds; lower poly, count fewer places or drop a hidden plane",
                basis.terms.len(),
                basis.reads(),
                crate::law::limits::PARAMS
            ),
            counted(u32::try_from(crate::law::limits::PARAMS).unwrap_or(u32::MAX)),
        ));
    }
    Ok(Planned {
        basis,
        reads,
        writes,
        channels,
        steps,
        hidden,
    })
}

enum FracAsked {
    Given(u32),
    Automatic,
}

fn frac_asked(held: &Json) -> Result<Option<FracAsked>, Failed> {
    let automatic = match held.get("auto_frac") {
        None => false,
        Some(Json::Bool(one)) => *one,
        Some(_) => return Err(unread("auto_frac")),
    };
    match (automatic, held.get("law_frac")) {
        (true, Some(_)) => Err(refused(
            Code::Malformed,
            "$.auto_frac",
            "auto_frac true chooses the law's frac, and law_frac names one; give one of them"
                .to_owned(),
        )),
        (true, None) => Ok(Some(FracAsked::Automatic)),
        (false, None) => Ok(None),
        (false, Some(_)) => {
            let frac = int(held, "law_frac")?;
            if frac > fixed::FRAC_MOST {
                return Err(limited(
                    Code::PlaneFormat,
                    "$.law_frac",
                    format!(
                        "law_frac {frac} leaves no bit for the sign and the whole part of a 32-bit word; it goes up to {}",
                        fixed::FRAC_MOST
                    ),
                    counted(fixed::FRAC_MOST),
                ));
            }
            Ok(Some(FracAsked::Given(frac)))
        }
    }
}

#[must_use]
pub fn bits_least(steps: u32) -> u32 {
    BITS_LEAST + (u32::BITS - steps.max(1).saturating_sub(1).leading_zeros())
}

pub fn bits_refused(most: u32, least: u32) -> Failed {
    Box::new(
        Refusal::new(
            Code::CoefficientBits,
            "$.auto_frac",
            format!(
                "no frac from 0 to {} keeps {least} fractional bits (12 plus log2 of steps, rounded up) in every coefficient over the range; the most any frac keeps is {most}; narrow range, lower poly or give law_frac",
                fixed::FRAC_MOST
            ),
        )
        .limited(counted(most)),
    )
}

pub fn bits_warning(scales: &[u32], wanted: u32) -> Option<Json> {
    let least = scales.iter().copied().min()?;
    (least < wanted).then(|| {
        map(vec![
            ("code", text("coefficient_bits")),
            ("coefficient_frac", counted(least)),
            ("least", counted(wanted)),
            (
                "message",
                text(&format!(
                    "a coefficient keeps {least} fractional bits, fewer than {wanted} (12, plus log2 of steps rounded up, for the rounding that k steps add): each is rounded to 2^-{least}, which shows as drift over many steps; lower frac or give auto_frac true"
                )),
            ),
        ])
    })
}

struct Reals {
    coefficients: Vec<Vec<f64>>,
    biases: Vec<f64>,
}

fn reals_of(basis: &Basis, theta: &[f64]) -> Reals {
    let terms = basis.terms.len();
    Reals {
        coefficients: (0..basis.reads())
            .map(|write| {
                (0..terms)
                    .map(|term| {
                        theta
                            .get(basis.coefficient(write, term))
                            .copied()
                            .unwrap_or(0.0)
                    })
                    .collect()
            })
            .collect(),
        biases: (0..basis.reads())
            .map(|write| theta.get(basis.bias(write)).copied().unwrap_or(0.0))
            .collect(),
    }
}

fn rescaled(basis: &Basis, theta: &mut [f64], hidden: usize, by: f64) {
    let write = basis.observed + hidden;
    for one in 0..basis.reads() {
        let lift = if one == write { by } else { 1.0 };
        for term in 0..basis.terms.len() {
            let power = basis.hidden_power(term, hidden);
            if let Some(slot) = theta.get_mut(basis.coefficient(one, term)) {
                *slot *= lift / by.powi(i32::try_from(power).unwrap_or(0));
            }
        }
        if let Some(slot) = theta.get_mut(basis.bias(one)) {
            *slot *= lift;
        }
    }
}

fn real_planes(words: &[Vec<u32>], frac: u32) -> Planes {
    let unit = f64::from(1_u32 << frac);
    words
        .iter()
        .map(|plane| {
            plane
                .iter()
                .map(|one| f64::from(one.cast_signed()) / unit)
                .collect()
        })
        .collect()
}

struct Scored {
    model: Vec<f64>,
    persistence: Vec<f64>,
    linear: Vec<f64>,
}

struct Sums {
    error: f64,
    target: f64,
    target_square: f64,
    count: f64,
}

impl Sums {
    const fn new() -> Self {
        Self {
            error: 0.0,
            target: 0.0,
            target_square: 0.0,
            count: 0.0,
        }
    }

    fn take(&mut self, guess: f64, wanted: f64) {
        self.error += (guess - wanted) * (guess - wanted);
        self.target += wanted;
        self.target_square += wanted * wanted;
        self.count += 1.0;
    }

    fn spread(&self) -> f64 {
        if self.count == 0.0 {
            return 0.0;
        }
        let mean = self.target / self.count;
        (self.target_square / self.count - mean * mean)
            .max(0.0)
            .sqrt()
    }

    fn nrmse(&self) -> f64 {
        if self.count == 0.0 {
            return 0.0;
        }
        (self.error / self.count).sqrt() / self.spread().max(f64::MIN_POSITIVE)
    }
}

pub fn learned(
    request: &Request<'_>,
    held: &Json,
    halt: &mut dyn FnMut() -> Option<Stop>,
) -> Result<Vec<(&'static str, Json)>, Failed> {
    let plan = planned(request, held)?;
    let basis = &plan.basis;
    let asked_frac = frac_asked(held)?;
    let kept_params = match held.get("params") {
        None => None,
        Some(one) => Some(ints(one, "params")?),
    };
    let kept_scales = match held.get("coefficient_frac") {
        None => None,
        Some(one) => Some(ints(one, "coefficient_frac")?),
    };
    match (&kept_params, &kept_scales) {
        (Some(params), Some(scales)) => {
            if params.len() != basis.params() {
                return Err(limited(
                    Code::Params,
                    "$.params",
                    format!(
                        "this request's law reads {} params, and {} were given",
                        basis.params(),
                        params.len()
                    ),
                    counted(u32::try_from(basis.params()).unwrap_or(u32::MAX)),
                ));
            }
            if scales.len() != basis.reads()
                || scales
                    .iter()
                    .any(|one| *one == 0 || *one > law_real::SCALE_MOST)
            {
                return Err(limited(
                    Code::Params,
                    "$.coefficient_frac",
                    format!(
                        "coefficient_frac names the fractional bits of each of the {} writes' coefficients, each from 1 to {}",
                        basis.reads(),
                        law_real::SCALE_MOST
                    ),
                    counted(u32::try_from(basis.reads()).unwrap_or(u32::MAX)),
                ));
            }
            if asked_frac.is_some() {
                return Err(refused(
                    Code::Params,
                    "$.law_frac",
                    "params hold a law whose frac is frac; law_frac and auto_frac go with a fit"
                        .to_owned(),
                ));
            }
        }
        (None, None) => {}
        (Some(_), None) => {
            return Err(refused(
                Code::Params,
                "$.coefficient_frac",
                "params come with coefficient_frac, the fractional bits of each write's coefficients, from the answer that gave them".to_owned(),
            ));
        }
        (None, Some(_)) => {
            return Err(refused(
                Code::Params,
                "$.params",
                "coefficient_frac goes with params".to_owned(),
            ));
        }
    }
    let examples = examples_of(held, basis.observed, request.border, request.stencil)?;
    let settle = match held.get("settle") {
        None => 0,
        Some(_) => int(held, "settle")? as usize,
    };
    let holdout = match held.get("holdout") {
        None => 0,
        Some(_) => int(held, "holdout")? as usize,
    };
    let shortest = examples
        .iter()
        .map(|one| one.shots.len())
        .min()
        .unwrap_or(2);
    if settle + holdout + 2 > shortest {
        return Err(limited(
            Code::TooFewShots,
            if holdout > 0 { "$.holdout" } else { "$.settle" },
            format!(
                "settle {settle} and holdout {holdout} leave the shortest example, of {shortest} shots, no rollout that starts after the settled shots and reaches a fitted shot"
            ),
            counted(u32::try_from(shortest.saturating_sub(2 + holdout)).unwrap_or(0)),
        ));
    }
    let frac = request.frac;
    let unit = f64::from(1_u32 << frac);
    let observed_range: [i32; 2] = match request.range {
        Some(range) => {
            for (at, example) in examples.iter().enumerate() {
                for shot in &example.shots {
                    for words in shot {
                        if let Some(word) = words
                            .iter()
                            .map(|one| one.cast_signed())
                            .find(|one| *one < range[0] || *one > range[1])
                        {
                            return Err(refused(
                                Code::PlaneFormat,
                                "$.range",
                                format!(
                                    "example {at} holds word {word}, outside range [{}, {}]; range must hold every example word",
                                    range[0], range[1]
                                ),
                            ));
                        }
                    }
                }
            }
            range
        }
        None => {
            let mut least = i32::MAX;
            let mut most = i32::MIN;
            for example in &examples {
                for shot in example
                    .shots
                    .iter()
                    .take(example.shots.len().saturating_sub(holdout))
                {
                    for word in shot.iter().flatten() {
                        least = least.min(word.cast_signed());
                        most = most.max(word.cast_signed());
                    }
                }
            }
            [least.min(most), most]
        }
    };
    let range_real = [
        f64::from(observed_range[0]) / unit,
        f64::from(observed_range[1]) / unit,
    ];
    let reach = range_real[0].abs().max(range_real[1].abs());
    let tangents = u64::try_from(basis.params()).unwrap_or(u64::MAX) + 1;
    let pass: u64 = examples
        .iter()
        .map(|one| {
            one.size
                .iter()
                .map(|side| u64::from(*side))
                .product::<u64>()
                .saturating_mul(one.shots.len().saturating_sub(holdout) as u64)
        })
        .sum::<u64>()
        .saturating_mul(u64::from(plan.steps))
        .saturating_mul(tangents);
    if pass > FIT_WORK_MOST {
        return Err(limited(
            Code::LearnLimit,
            "$.steps",
            format!(
                "one pass of the fit steps the examples' cells over their fitted shots times steps, carrying the law and its {} params: {pass} units, more than the {FIT_WORK_MOST} one pass may take",
                basis.params()
            ),
            Json::Int(i64::try_from(FIT_WORK_MOST).unwrap_or(i64::MAX)),
        ));
    }
    let clips: Vec<Clip> = examples
        .iter()
        .map(|example| Clip {
            grid: Grid::new(example.size, basis, request.border),
            shots: example
                .shots
                .iter()
                .take(example.shots.len().saturating_sub(holdout))
                .map(|shot| real_planes(shot, frac))
                .collect(),
        })
        .collect();
    let fitting_limits: Vec<[f64; 2]> = (0..basis.reads())
        .map(|write| {
            if write < basis.observed {
                range_real
            } else {
                [-4.0 * reach, 4.0 * reach]
            }
        })
        .collect();
    let setup = Setup {
        basis,
        clips: &clips,
        steps: plan.steps,
        limits: fitting_limits,
        settle,
    };
    let mut run = Run::new(&setup);
    let mut stages: Vec<Stage> = Vec::new();
    let mut stopped: Option<Stop> = None;
    let longest = clips.iter().map(|one| one.shots.len()).max().unwrap_or(2);
    let (theta, scales_given) = match (&kept_params, &kept_scales) {
        (Some(params), Some(scales)) => {
            let fixed = fixed::fixed_of(basis, scales, params);
            let mut theta = vec![0.0; basis.params()];
            for (write, one) in fixed.iter().enumerate() {
                let lift = f64::from(1_u32 << one.scale);
                for (term, coefficient) in one.coefficients.iter().enumerate() {
                    if let Some(slot) = theta.get_mut(basis.coefficient(write, term)) {
                        *slot = f64::from(*coefficient) / lift;
                    }
                }
                if let Some(slot) = theta.get_mut(basis.bias(write)) {
                    *slot = f64::from(one.bias) / lift / unit;
                }
            }
            (theta, Some(fixed))
        }
        _ => {
            let begun = fit::macro_start(&setup);
            let horizons = fit::horizons(longest, HORIZON_MOST);
            let mut best: Option<(Vec<f64>, Stage)> = None;
            for mut candidate in fit::hidden_starts(&mut run, &begun) {
                let stage = run.descend(&mut candidate, 1, halt);
                if let Some(stop) = stage.stopped {
                    stopped = Some(stop);
                }
                if stage.rounds > 0
                    && stage.cost.is_finite()
                    && best.as_ref().is_none_or(|(_, held)| stage.cost < held.cost)
                {
                    best = Some((candidate, stage));
                }
                if stopped.is_some() {
                    break;
                }
            }
            let Some((theta_found, first_stage)) = best else {
                let stop = stopped.unwrap_or(Stop::Budget);
                return Err(refused(
                    stop.code(),
                    if stop == Stop::Budget {
                        "$.budget_ms"
                    } else {
                        "$"
                    },
                    format!(
                        "the fit stopped on {} before its first round of Levenberg-Marquardt; nothing was learned",
                        stop.name()
                    ),
                ));
            };
            let mut theta = theta_found;
            stages.push(first_stage);
            for horizon in horizons.into_iter().skip(1) {
                if stopped.is_some() {
                    break;
                }
                let stage = run.descend(&mut theta, horizon, halt);
                stopped = stage.stopped;
                stages.push(stage);
            }
            for hidden in 0..basis.hidden {
                let mut most = 0.0_f64;
                for clip in &clips {
                    for (planes, _) in run.observed(&theta, clip, false) {
                        if let Some(plane) = planes.get(hidden) {
                            most = plane.iter().fold(most, |held, one| held.max(one.abs()));
                        }
                    }
                }
                if most > 0.0 && reach > 0.0 {
                    rescaled(basis, &mut theta, hidden, 0.5 * reach / most);
                }
            }
            (theta, None)
        }
    };
    let reals = reals_of(basis, &theta);
    let mut warnings = Vec::new();
    let scales_checked = scales_given.is_some();
    let (law_frac, fixed_list, mode): (u32, Vec<Fixed>, &str) = match (scales_given, asked_frac) {
        (Some(fixed), _) => (frac, fixed, "params"),
        (None, Some(FracAsked::Automatic)) => {
            let chosen = fixed::automatic(
                basis,
                range_real,
                &reals.coefficients,
                &reals.biases,
                bits_least(plan.steps),
            )
            .map_err(|most| bits_refused(most, bits_least(plan.steps)))?;
            (chosen.frac, chosen.fixed, "auto")
        }
        (None, given) => {
            let law_frac = match given {
                Some(FracAsked::Given(one)) => one,
                _ => frac,
            };
            let Some(chosen) = fixed::quantised(
                basis,
                law_frac,
                range_real,
                &reals.coefficients,
                &reals.biases,
            ) else {
                let fits = (0..law_frac).rev().find(|one| {
                    fixed::quantised(basis, *one, range_real, &reals.coefficients, &reals.biases)
                        .is_some()
                });
                let message = format!(
                    "at frac {law_frac} the terms over the range need more than the 32 bits one word holds for the sum of coefficients times terms; lower frac, narrow range or drop poly"
                );
                return Err(match fits {
                    Some(one) => Box::new(
                        Refusal::new(Code::FixedRange, "$.frac", message).limited(counted(one)),
                    ),
                    None => Box::new(Refusal::new(Code::FixedRange, "$.frac", message)),
                });
            };
            (
                law_frac,
                chosen.fixed,
                if given.is_some() { "given" } else { "frac" },
            )
        }
    };
    let scales: Vec<u32> = fixed_list.iter().map(|one| one.scale).collect();
    if let Some(warning) = bits_warning(&scales, bits_least(plan.steps)) {
        warnings.push(warning);
    }
    if law_frac < frac {
        warnings.push(map(vec![
            ("code", text("frac_below_examples")),
            ("frac", counted(law_frac)),
            ("example_frac", counted(frac)),
            (
                "message",
                text(&format!(
                    "the law's frac {law_frac} is below the examples' frac {frac}: a world under the law holds each value to 2^-{law_frac}, coarser than the examples; narrow range or give law_frac"
                )),
            ),
        ]));
    }
    if scales_checked {
        let bound = fixed::bounds(basis, law_frac, fixed::reach_of(range_real, law_frac));
        let fits = bound.as_ref().is_some_and(|bound| {
            fixed_list.iter().all(|one| {
                let reach = one
                    .coefficients
                    .iter()
                    .zip(bound)
                    .fold(0_u128, |sum, (coefficient, term)| {
                        sum.saturating_add(
                            u128::from(coefficient.unsigned_abs()).saturating_mul(*term),
                        )
                    })
                    .saturating_add(u128::from(one.bias.unsigned_abs()))
                    .saturating_add(1_u128 << (one.scale - 1));
                reach < 1_u128 << 31
            })
        });
        if !fits {
            return Err(refused(
                Code::FixedRange,
                "$.params",
                format!(
                    "the params' coefficients times the terms over range, at frac {law_frac} and their coefficient_frac, need more than the 32 bits one word holds, so the law could wrap inside range; narrow range or give params that fit"
                ),
            ));
        }
    }
    let law_unit = f64::from(1_u32 << law_frac);
    let observed_words = fixed::word_range(range_real, law_frac);
    let reach_word = fixed::reach_of(range_real, law_frac);
    let reach_i32 = i32::try_from(reach_word.min(i32::MAX as u64)).unwrap_or(i32::MAX);
    let clamps: Vec<[i32; 2]> = (0..basis.reads())
        .map(|write| {
            if write < basis.observed {
                observed_words
            } else {
                [-reach_i32, reach_i32]
            }
        })
        .collect();
    let params = fixed::param_words(basis, &fixed_list);
    let text_of = Text {
        basis,
        fixed: &fixed_list,
        frac: law_frac,
        clamps: &clamps,
        reads: &plan.reads,
        writes: &plan.writes,
        channels: plan.channels,
        name: request.name,
    };
    let written = text_of.written();
    let law = Compiled::compile(&written).map_err(|why| {
        Box::new(Told(
            Refusal::new(
                Code::LawRefused,
                "$",
                format!("the law made from the fit does not compile: {why}"),
            ),
            vec![("law", why.json())],
        )) as Failed
    })?;
    let words = Words {
        basis,
        fixed: &fixed_list,
        frac: law_frac,
        clamps: &clamps,
    };
    let rescale = |word: u32| -> u32 {
        let value = f64::from(word.cast_signed()) / unit * law_unit;
        let rounded = value
            .round()
            .clamp(f64::from(i32::MIN), f64::from(i32::MAX));
        (rounded as i32).cast_unsigned()
    };
    let word_examples: Vec<Vec<Vec<Vec<u32>>>> = examples
        .iter()
        .map(|example| {
            example
                .shots
                .iter()
                .map(|shot| {
                    shot.iter()
                        .map(|plane| plane.iter().map(|one| rescale(*one)).collect())
                        .collect()
                })
                .collect()
        })
        .collect();
    let hidden_planes: Vec<usize> = (basis.observed..basis.reads()).collect();
    let all_planes: Vec<usize> = (0..basis.reads()).collect();
    let mut residual_sums: Vec<(f64, f64, f64)> = vec![(0.0, 0.0, 0.0); basis.observed];
    let checked = [1_usize, holdout.max(1)];
    let mut scored: Vec<[Sums; 3]> = checked
        .iter()
        .map(|_| [Sums::new(), Sums::new(), Sums::new()])
        .collect();
    let linear = linear_baseline(basis, &clips, range_real);
    let mut states = Vec::with_capacity(examples.len());
    for (shots, clip) in word_examples.iter().zip(&clips) {
        let grid = &clip.grid;
        let mut hidden: Vec<Vec<u32>> = vec![vec![0; grid.cells]; basis.hidden];
        let mut watched = vec![hidden.clone()];
        for pair in shots.windows(2) {
            let [from, to] = pair else {
                continue;
            };
            for sub in 0..plan.steps {
                let mut planes: Vec<Vec<u32>> = from
                    .iter()
                    .zip(to)
                    .map(|(one, other)| fixed::between(one, other, sub, plan.steps))
                    .collect();
                planes.extend(hidden.iter().cloned());
                hidden = words
                    .advance(grid, &planes, &hidden_planes)
                    .into_iter()
                    .skip(basis.observed)
                    .collect();
            }
            watched.push(hidden.clone());
        }
        let mut last_state: Vec<Vec<u32>> = shots.iter().last().cloned().unwrap_or_default();
        last_state.extend(watched.iter().last().cloned().unwrap_or_default());
        states.push(Json::List(
            last_state
                .iter()
                .map(|plane| {
                    Json::List(
                        plane
                            .iter()
                            .map(|one| Json::Int(i64::from(one.cast_signed())))
                            .collect(),
                    )
                })
                .collect(),
        ));
        let fitted = shots.len().saturating_sub(holdout);
        for begin in settle..fitted.saturating_sub(1) {
            let (Some(shot), Some(hidden_now), Some(wanted)) =
                (shots.get(begin), watched.get(begin), shots.get(begin + 1))
            else {
                continue;
            };
            let mut planes = shot.clone();
            planes.extend(hidden_now.iter().cloned());
            for _ in 0..plan.steps {
                planes = words.advance(grid, &planes, &all_planes);
            }
            for (read, (now, target)) in planes.iter().zip(wanted).enumerate() {
                for (one, other) in now.iter().zip(target) {
                    let missed = f64::from(one.cast_signed()) / law_unit
                        - f64::from(other.cast_signed()) / law_unit;
                    if let Some(slot) = residual_sums.get_mut(read) {
                        slot.0 += missed * missed;
                        slot.1 = slot.1.max(missed.abs());
                        slot.2 += 1.0;
                    }
                }
            }
        }
        if holdout == 0 {
            continue;
        }
        let begin = fitted.saturating_sub(1);
        let (Some(shot), Some(hidden_now)) = (shots.get(begin), watched.get(begin)) else {
            continue;
        };
        let mut planes = shot.clone();
        planes.extend(hidden_now.iter().cloned());
        let mut line: Planes = real_planes(shot, law_frac);
        for ahead in 1..=holdout {
            let Some(wanted) = shots.get(begin + ahead) else {
                break;
            };
            for _ in 0..plan.steps {
                planes = words.advance(grid, &planes, &all_planes);
            }
            line = linear.advanced(grid, &line);
            for read in 0..basis.observed {
                let (Some(now), Some(target), Some(still_shot), Some(lined)) = (
                    planes.get(read),
                    wanted.get(read),
                    shot.get(read),
                    line.get(read),
                ) else {
                    continue;
                };
                for (((one, other), still), guess) in
                    now.iter().zip(target).zip(still_shot).zip(lined)
                {
                    let got = f64::from(one.cast_signed()) / law_unit;
                    let aim = f64::from(other.cast_signed()) / law_unit;
                    for (at, horizon) in checked.iter().enumerate() {
                        if *horizon == ahead
                            && let Some(three) = scored.get_mut(at)
                        {
                            let [model, persistence, linear_sums] = three;
                            model.take(got, aim);
                            persistence.take(f64::from(still.cast_signed()) / law_unit, aim);
                            linear_sums.take(*guess, aim);
                        }
                    }
                }
            }
        }
    }
    let mut fit_rms = vec![0.0; basis.observed];
    {
        let mut check = Run::new(&setup);
        check.reset(1);
        let normal = check.normal(&theta, 1, false);
        let each = (normal.cost / (normal.count.max(1) as f64)).sqrt();
        fit_rms.iter_mut().for_each(|one| *one = each);
        run.work = run.work.saturating_add(check.work);
    }
    let result = Scored {
        model: scored.iter().map(|one| one[0].nrmse()).collect(),
        persistence: scored.iter().map(|one| one[1].nrmse()).collect(),
        linear: scored.iter().map(|one| one[2].nrmse()).collect(),
    };
    let at_last = |list: &[f64]| list.iter().last().copied().unwrap_or(0.0);
    let best_baseline = at_last(&result.persistence).min(at_last(&result.linear));
    let flat = scored
        .iter()
        .last()
        .is_none_or(|three| three[0].spread() == 0.0);
    let beats = (holdout > 0 && !flat).then(|| at_last(&result.model) < MARGIN * best_baseline);
    match beats {
        Some(false) => warnings.push(map(vec![
            ("code", text("baselines_not_beaten")),
            (
                "message",
                text(&format!(
                    "on the {holdout} held-out shots the law's forecast has NRMSE {:.4}, not below {MARGIN} times the better baseline's {:.4}: the process may lie outside the law's class (radius 1, k sub-steps, the terms of poly and hidden)",
                    at_last(&result.model),
                    best_baseline
                )),
            ),
        ])),
        Some(true) => warnings.push(map(vec![
            ("code", text("forecast_unchecked")),
            ("checked", counted(u32::try_from(holdout).unwrap_or(0))),
            (
                "message",
                text(&format!(
                    "the forecast is checked {holdout} shots ahead and no further; a horizon past {holdout} shots is not judged"
                )),
            ),
        ])),
        None if holdout > 0 => warnings.push(map(vec![
            ("code", text("forecast_unchecked")),
            ("checked", counted(0)),
            (
                "message",
                text(&format!(
                    "the observed planes do not vary over the {holdout} held-out shots, so their NRMSE has no scale and the law's forecast is not judged against the baselines"
                )),
            ),
        ])),
        None => warnings.push(map(vec![
            ("code", text("forecast_unchecked")),
            ("checked", counted(0)),
            (
                "message",
                text("no shots were held out, so the law's forecast is not judged against the baselines; give holdout"),
            ),
        ])),
    }
    let class = super::law::background_class(&law, &params);
    let mut answer = vec![
        ("law", text(&written)),
        ("hash", text(&law.hex())),
        ("name", text(law.name())),
        ("channels", counted(law.channels())),
        ("halo", counted(law.halo())),
        ("quiet", text(class.name())),
    ];
    if let Some(why) = class.why() {
        answer.push(("quiet_why", text(&why)));
    }
    let pairs: Vec<Json> = plan
        .reads
        .iter()
        .zip(&plan.writes)
        .map(|(read, write)| Json::List(vec![counted(*read), counted(*write)]))
        .collect();
    let term_at = |wanted: Term| basis.terms.iter().position(|term| *term == wanted);
    let centre_place = basis.stencil.place([0, 0, 0]);
    let held_fixed = |write: usize, term: Option<usize>| {
        term.and_then(|at| {
            fixed_list
                .get(write)
                .and_then(|one| one.coefficients.get(at))
        })
        .is_some_and(|one| *one != 0)
    };
    let coefficient_moves = |write: usize, term: usize, hidden: usize| -> bool {
        let plane = basis.observed + hidden;
        match basis.terms.get(term) {
            Some(Term::Place { read, place })
                if *read < basis.observed && *place == centre_place =>
            {
                held_fixed(
                    write,
                    term_at(Term::Pair {
                        left: *read,
                        right: plane,
                    }),
                )
            }
            _ => false,
        }
    };
    let bias_moves = |write: usize, hidden: usize| -> bool {
        let plane = basis.observed + hidden;
        write == plane
            || held_fixed(
                write,
                term_at(Term::Place {
                    read: plane,
                    place: centre_place,
                }),
            )
    };
    let coefficients_shown: Vec<Json> = fixed_list
        .iter()
        .enumerate()
        .map(|(write, one)| {
            Json::List(
                one.coefficients
                    .iter()
                    .enumerate()
                    .map(|(term, constant)| {
                        let scales_as: Vec<Json> = (0..basis.hidden)
                            .map(|hidden| {
                                let lift = i64::from(u8::from(write == basis.observed + hidden));
                                Json::Int(lift - i64::from(basis.hidden_power(term, hidden)))
                            })
                            .collect();
                        let mut fields = vec![
                            (
                                "value",
                                text(&law_real::decimal(i128::from(*constant), one.scale)),
                            ),
                            ("fixed", Json::Int(i64::from(*constant))),
                            (
                                "fit",
                                scientific(
                                    reals
                                        .coefficients
                                        .get(write)
                                        .and_then(|list| list.get(term))
                                        .copied()
                                        .unwrap_or(0.0),
                                ),
                            ),
                            (
                                "param",
                                counted(u32::try_from(basis.coefficient(write, term)).unwrap_or(0)),
                            ),
                        ];
                        if basis.hidden > 0 {
                            fields.push(("scales_as", Json::List(scales_as)));
                            fields.push((
                                "offset_moves",
                                Json::List(
                                    (0..basis.hidden)
                                        .map(|hidden| {
                                            Json::Bool(coefficient_moves(write, term, hidden))
                                        })
                                        .collect(),
                                ),
                            ));
                        }
                        map(fields)
                    })
                    .collect(),
            )
        })
        .collect();
    let biases_shown: Vec<Json> = fixed_list
        .iter()
        .enumerate()
        .map(|(write, one)| {
            let mut fields = vec![
                (
                    "value",
                    text(&law_real::decimal(
                        i128::from(one.bias),
                        one.scale + law_frac,
                    )),
                ),
                ("fixed", Json::Int(i64::from(one.bias))),
                (
                    "fit",
                    scientific(reals.biases.get(write).copied().unwrap_or(0.0)),
                ),
                (
                    "param",
                    counted(u32::try_from(basis.bias(write)).unwrap_or(0)),
                ),
            ];
            if basis.hidden > 0 {
                let scales_as: Vec<Json> = (0..basis.hidden)
                    .map(|hidden| Json::Int(i64::from(u8::from(write == basis.observed + hidden))))
                    .collect();
                fields.push(("scales_as", Json::List(scales_as)));
                fields.push((
                    "offset_moves",
                    Json::List(
                        (0..basis.hidden)
                            .map(|hidden| Json::Bool(bias_moves(write, hidden)))
                            .collect(),
                    ),
                ));
            }
            map(fields)
        })
        .collect();
    let residuals: Vec<Json> = residual_sums
        .iter()
        .zip(&fit_rms)
        .map(|((square, far, count), fitted)| {
            map(vec![
                ("rms_real", scientific((square / count.max(1.0)).sqrt())),
                ("max_real", scientific(*far)),
                ("fit_rms_real", scientific(*fitted)),
            ])
        })
        .collect();
    let horizons_shown = Json::List(
        checked
            .iter()
            .map(|one| counted(u32::try_from(*one).unwrap_or(0)))
            .collect(),
    );
    let unscaled: Vec<bool> = scored
        .iter()
        .map(|three| three[0].count > 0.0 && three[0].spread() == 0.0)
        .collect();
    let given_list = |list: &[f64]| {
        Json::List(
            list.iter()
                .zip(&unscaled)
                .map(|(one, flat)| if *flat { Json::Null } else { scientific(*one) })
                .collect(),
        )
    };
    let baselines = map(vec![
        ("shots_ahead", horizons_shown),
        ("law", given_list(&result.model)),
        ("persistence", given_list(&result.persistence)),
        ("linear", given_list(&result.linear)),
        ("margin", scientific(MARGIN)),
        ("held_out", counted(u32::try_from(holdout).unwrap_or(0))),
        ("beats", beats.map_or(Json::Null, Json::Bool)),
        (
            "means",
            text(
                "NRMSE (root mean square error over the standard deviation of the wanted words) of the observed planes on the held_out shots at the end of each example, which a fit never sees (with params there is no fit), forecast from the last shot before them shots_ahead shots on (null where those shots' observed planes do not vary, so NRMSE has no scale): law, the answer's fixed-point law with the hidden planes run as start says; persistence, that shot unchanged; linear, the best one-shot linear fit on the observed reads alone (their counted places and a bias, least squares on the fitted shots, clamped to the range) applied once a shot; beats when the law's NRMSE at the last horizon is below margin times the better baseline's, null when nothing is held out or the held-out observed planes do not vary; horizons past held_out are not judged",
            ),
        ),
    ]);
    let mut stability = Vec::new();
    for write in 0..basis.observed {
        stability.push(amplification(basis, &reals, write));
    }
    let unstable = stability.iter().any(|one| *one > 1.0 + 1e-9);
    if unstable {
        warnings.push(map(vec![
            ("code", text("unstable_linear_part")),
            (
                "message",
                text("the linear place part of an observed write amplifies some spatial wave by more than 1 a sub-step: explicit stepping may grow a checkerboard; more steps or a smaller range of diffusion keeps it below 1"),
            ),
        ]));
    }
    let hidden_shown = map(vec![
        ("count", counted(plan.hidden)),
        (
            "planes",
            Json::List(
                plan.reads
                    .iter()
                    .zip(&plan.writes)
                    .skip(basis.observed)
                    .map(|(read, write)| {
                        map(vec![("read", counted(*read)), ("write", counted(*write))])
                    })
                    .collect(),
            ),
        ),
        ("start", text(START)),
        ("identifiable", text(SCALES)),
    ]);
    let stages_shown: Vec<Json> = stages
        .iter()
        .map(|one| {
            map(vec![
                ("horizon", counted(u32::try_from(one.horizon).unwrap_or(0))),
                ("rounds", counted(one.rounds)),
                (
                    "rms_real",
                    scientific((one.cost / (one.count.max(1) as f64)).sqrt()),
                ),
            ])
        })
        .collect();
    let range_words = Json::List(vec![
        Json::Int(i64::from(observed_words[0])),
        Json::Int(i64::from(observed_words[1])),
    ]);
    answer.extend([
        ("stepping", text(if class.proved() { "listed" } else { "whole" })),
        ("nodes", counted(u32::try_from(law.nodes()).unwrap_or(u32::MAX))),
        ("rule", text("linear")),
        ("means", text(law_real::LINEAR)),
        ("poly", counted(request.poly)),
        ("steps", counted(plan.steps)),
        ("settle", counted(u32::try_from(settle).unwrap_or(0))),
        ("holdout", counted(u32::try_from(holdout).unwrap_or(0))),
        ("params", Json::List(params.iter().map(|one| counted(*one)).collect())),
        (
            "iterate",
            map(vec![
                (
                    "alternate",
                    match pairs.as_slice() {
                        [one] => one.clone(),
                        _ => Json::List(pairs.clone()),
                    },
                ),
                ("frames", counted(plan.steps)),
            ]),
        ),
        ("hidden", hidden_shown),
        (
            "fixed",
            map(vec![
                ("frac", counted(law_frac)),
                ("example_frac", counted(frac)),
                ("frac_from", text(mode)),
                ("coefficient_frac", Json::List(scales.iter().map(|one| counted(*one)).collect())),
                ("coefficient_frac_least", counted(bits_least(plan.steps))),
                ("format", text(law_real::FORMAT)),
                ("rounding", text(law_real::ROUNDING)),
                ("range", range_words),
                (
                    "clamp",
                    Json::List(
                        clamps
                            .iter()
                            .map(|one| Json::List(vec![Json::Int(i64::from(one[0])), Json::Int(i64::from(one[1]))]))
                            .collect(),
                    ),
                ),
                (
                    "param_format",
                    text("each param is a coefficient's fixed word, or a write's bias B, plus 2^31 as an unsigned word, so params order as the numbers do"),
                ),
            ]),
        ),
        (
            "terms",
            Json::List(basis.terms.iter().map(|one| term_shown(basis, *one)).collect()),
        ),
        ("coefficients", Json::List(coefficients_shown)),
        ("bias", Json::List(biases_shown)),
        ("residual", Json::List(residuals)),
        ("baselines", baselines),
        (
            "stability",
            map(vec![
                ("amplification", Json::List(stability.iter().map(|one| scientific(*one)).collect())),
                ("means", text("per observed write, the largest gain of its counted places' coefficients over the spatial waves of a 16-cell period on every axis: above 1 explicit stepping can grow a checkerboard")),
            ]),
        ),
        ("method", text(if kept_params.is_some() { "params given: no fit; the hidden planes are run as start says" } else { METHOD })),
        ("stages", Json::List(stages_shown)),
        (
            "stopped",
            text(stopped.map_or("complete", Stop::name)),
        ),
        ("work", Json::Int(i64::try_from(run.work).unwrap_or(i64::MAX))),
        ("state", Json::List(states)),
    ]);
    if !warnings.is_empty() {
        answer.push(("warnings", Json::List(warnings)));
    }
    Ok(answer)
}

fn amplification(basis: &Basis, reals: &Reals, write: usize) -> f64 {
    let Some(list) = reals.coefficients.get(write) else {
        return 0.0;
    };
    let places: Vec<([i64; 3], f64)> = basis
        .terms
        .iter()
        .zip(list)
        .filter_map(|(term, value)| match term {
            Term::Place { read, place } if *read == write => {
                Some((basis.stencil.offset(*place), *value))
            }
            _ => None,
        })
        .collect();
    let axes = basis.stencil.axes();
    let period = 16_i32;
    let mut most = 0.0_f64;
    let steps = if axes == 3 { period } else { 1 };
    for kx in 0..period {
        for ky in 0..period {
            for kz in 0..steps {
                let wave = [kx, ky, kz]
                    .map(|one| f64::from(one) * std::f64::consts::TAU / f64::from(period));
                let (mut cosines, mut sines) = (0.0, 0.0);
                for (offset, value) in &places {
                    let phase: f64 = offset
                        .iter()
                        .zip(wave)
                        .map(|(one, by)| *one as f64 * by)
                        .sum();
                    cosines += value * phase.cos();
                    sines += value * phase.sin();
                }
                most = most.max(cosines.hypot(sines));
            }
        }
    }
    most
}

struct Linear {
    places: Vec<u32>,
    stencil_centre: usize,
    coefficients: Vec<Vec<f64>>,
    range: [f64; 2],
}

impl Linear {
    fn advanced(&self, grid: &Grid, planes: &Planes) -> Planes {
        let _ = self.stencil_centre;
        self.coefficients
            .iter()
            .enumerate()
            .map(|(read, list)| {
                (0..grid.cells)
                    .map(|cell| {
                        let mut sum = list.iter().last().copied().unwrap_or(0.0);
                        for (at, value) in list.iter().take(self.places.len()).enumerate() {
                            let near = grid.near(cell, at);
                            if near != model::NONE {
                                sum += value
                                    * planes
                                        .get(read)
                                        .and_then(|plane| plane.get(near as usize))
                                        .copied()
                                        .unwrap_or(0.0);
                            }
                        }
                        sum.clamp(self.range[0], self.range[1])
                    })
                    .collect()
            })
            .collect()
    }
}

fn linear_baseline(basis: &Basis, clips: &[Clip], range: [f64; 2]) -> Linear {
    let mut coefficients = Vec::with_capacity(basis.observed);
    for read in 0..basis.observed {
        let mut rows = Vec::new();
        let mut wanted = Vec::new();
        for clip in clips {
            for pair in clip.shots.windows(2) {
                let [from, to] = pair else {
                    continue;
                };
                for cell in 0..clip.grid.cells {
                    let mut row: Vec<f64> = (0..basis.places.len())
                        .map(|at| {
                            let near = clip.grid.near(cell, at);
                            if near == model::NONE {
                                0.0
                            } else {
                                from.get(read)
                                    .and_then(|plane| plane.get(near as usize))
                                    .copied()
                                    .unwrap_or(0.0)
                            }
                        })
                        .collect();
                    row.push(1.0);
                    rows.push(row);
                    wanted.push(
                        to.get(read)
                            .and_then(|plane| plane.get(cell))
                            .copied()
                            .unwrap_or(0.0),
                    );
                }
            }
        }
        coefficients.push(fit::least_squares(&rows, &wanted).unwrap_or_else(|| {
            let mut list = vec![0.0; basis.places.len() + 1];
            if let Some(slot) = list.get_mut(basis.centre) {
                *slot = 1.0;
            }
            list
        }));
    }
    Linear {
        places: basis.places.clone(),
        stencil_centre: basis.centre,
        coefficients,
        range,
    }
}

#[must_use]
pub fn order_text() -> &'static str {
    stencil::ORDER
}
