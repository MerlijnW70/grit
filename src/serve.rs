use crate::board::seated;
use crate::compact::Plan;
use crate::host::Host;
use crate::layout::{BRUSH, GRAND, PARAMS, STATE};
use crate::machine::Machine;
use crate::marking::{
    BEST, BINDINGS, FEED, HELD, MARKED_INPUTS, SCORE, SEED, TRIAL, VALUES, WANTED, chosen, marked,
    stepped, turned_on, words_of,
};
use crate::netlist::{Chip, Exam, Gate, INPUTS, Netlist, OUTPUTS, SOURCES, drawn, turned};
use crate::resident::{Groups, Resident};
use crate::rule::{KNOB, Kind, POINT};
use crate::sand::{AIR, Grid, SAND, WALL, WATER, material_of};
use crate::serve3::errors::{Code, Refusal, shown, shown_value};
use crate::sweep;
use crate::wire::{Json, counted, map, text};

pub const CROWD: u32 = 65536;

pub const SIDE: u32 = 256;

pub const ROUNDS: u32 = 4000;

pub const WORK: u64 = 1 << 20;

pub const RUNGS: usize = 64;

pub const SEEDING: u32 = 0x1234_5678;

pub struct Found {
    pub marks: u32,
    pub full: u32,
    pub rounds: u32,
    pub crowd: u32,
    pub gates: Vec<Gate>,
    pub table: Vec<u32>,
    pub wrong: Vec<u32>,
    pub most: u32,
    pub longest: std::time::Duration,
}

fn columns(exam: &Exam) -> Vec<u32> {
    let shape = exam.shape();
    let wide = words_of(shape);
    let mut held = Vec::new();
    for output in 0..shape.outputs {
        for word in 0..wide {
            let mut one = 0_u32;
            for bit in 0..32_u32 {
                let row = word * 32 + bit;
                if row < shape.words() && (exam.asked(row) >> output) & 1 == 1 {
                    one |= 1 << bit;
                }
            }
            held.push(one);
        }
    }
    held
}

#[must_use]
pub const fn mixed(held: u32) -> u32 {
    let held = (held ^ (held >> 16)).wrapping_mul(0x85eb_ca6b);
    let held = (held ^ (held >> 13)).wrapping_mul(0xc2b2_ae35);
    held ^ (held >> 16)
}

#[must_use]
pub const fn restarted(seed: u32, restart: u32) -> u32 {
    if restart == 0 {
        seed
    } else {
        mixed(seed ^ mixed(restart.wrapping_mul(0x9e37_79b9)))
    }
}

const fn salt(seed: u32) -> u32 {
    mixed(seed) ^ mixed(SEEDING)
}

pub const ASIDE: u32 = 0x9e37_79b9;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rolls {
    state: u32,
    flip: u32,
}

impl Rolls {
    #[must_use]
    pub const fn from(start: u32) -> Self {
        if start == 0 {
            Self {
                state: ASIDE,
                flip: ASIDE,
            }
        } else {
            Self {
                state: start,
                flip: 0,
            }
        }
    }

    pub const fn rolled(&mut self) -> u32 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 17;
        self.state ^= self.state << 5;
        self.state ^ self.flip
    }
}

fn crowd_of(shape: Netlist, seed: u32, many: u32) -> Vec<u32> {
    let mut rolls = Rolls::from((SEEDING | 1) ^ salt(seed));
    (0..many)
        .flat_map(|_| {
            (0..shape.gates)
                .map(|at| drawn(&shape, at, rolls.rolled()).pack())
                .collect::<Vec<u32>>()
        })
        .collect()
}

#[must_use]
pub fn seeded(many: u32) -> Vec<u32> {
    seeded_by(many, SEEDING)
}

#[must_use]
pub fn seeded_by(many: u32, seed: u32) -> Vec<u32> {
    let salt = salt(seed);
    (0..many)
        .map(|who| match (who.wrapping_mul(2_654_435_761) | 1) ^ salt {
            0 => mixed(salt) | 1,
            one => one,
        })
        .collect()
}

#[must_use]
pub fn opening(shape: Netlist, seed: u32, restart: u32, many: u32) -> (Vec<u32>, Vec<u32>) {
    let seed = restarted(seed, restart);
    (crowd_of(shape, seed, many), seeded_by(many, seed))
}

fn times(parts: &[u32]) -> Result<usize, Box<dyn std::error::Error>> {
    parts
        .iter()
        .try_fold(1_usize, |held, one| held.checked_mul(*one as usize))
        .ok_or_else(|| {
            format!(
                "a buffer of {parts:?} words multiplied out is past what this machine can address"
            )
            .into()
        })
}

fn room(
    shape: Netlist,
    many: u32,
) -> Result<[usize; BINDINGS as usize], Box<dyn std::error::Error>> {
    let wide = words_of(shape);
    Ok([
        times(&[many, shape.gates])?,
        times(&[many, shape.gates])?,
        times(&[many, shape.sources(), wide])?,
        times(&[shape.outputs, wide])?,
        many as usize,
        many as usize,
        many as usize,
        1,
    ])
}

fn whole(many: u32, width: u32) -> u32 {
    many.div_ceil(width).saturating_mul(width)
}

fn unrolled(held: Machine, wide: u32) -> Result<(), Box<dyn std::error::Error>> {
    let each = u64::from(held.gates)
        + u64::from(held.state)
        + u64::from(held.inputs)
        + u64::from(held.outputs);
    let work = u64::from(held.steps) * each * u64::from(wide);
    if work > WORK {
        return Err(format!(
            "{} steps of {each} gates and wires over {wide} words of runs would unroll into {work} operations, over the {WORK} one kernel is allowed; ask for fewer steps, fewer gates or fewer runs",
            held.steps
        )
        .into());
    }
    Ok(())
}

fn fits_unrolled(machine: Machine, runs: u32) -> Result<(), Box<dyn std::error::Error>> {
    let wide = runs.div_ceil(32).max(1);
    if unrolled(machine, wide).is_ok() {
        return Ok(());
    }
    let each = u64::from(machine.gates)
        + u64::from(machine.state)
        + u64::from(machine.inputs)
        + u64::from(machine.outputs);
    let said = format!(
        "{runs} runs of {} steps of {each} gates and wires would unroll past the {WORK} operations one kernel is allowed",
        machine.steps
    );
    let steps = WORK / (each * u64::from(wide));
    let words = WORK / (each * u64::from(machine.steps.max(1)));
    let (field, limit, what) = if steps >= 1 {
        ("$.steps", steps, "steps fit with these runs")
    } else if words >= 1 {
        ("$.traces", words * 32, "runs fit with these steps")
    } else {
        ("$.steps", WORK / each, "steps fit with one word of runs")
    };
    let limit = u32::try_from(limit).unwrap_or(u32::MAX);
    Err(beyond(
        field.to_owned(),
        format!("{said}; at most {limit} {what}"),
        limit,
    ))
}

fn named(shape: Netlist) -> Vec<u32> {
    (shape.gates - shape.outputs..shape.gates).collect()
}

fn judged(shape: Netlist, exam: &Exam, gates: &[u32]) -> Result<Found, Box<dyn std::error::Error>> {
    let chip = Chip::of(shape, gates.to_vec(), named(shape))?;
    let table = chip.table();
    let wrong = (0..shape.words())
        .filter(|row| table.get(*row as usize).copied() != Some(exam.asked(*row)))
        .collect();
    Ok(Found {
        marks: exam.scored(&table),
        full: exam.marks(),
        rounds: 0,
        crowd: 0,
        gates: gates.iter().map(|one| Gate::unpack(*one)).collect(),
        table,
        wrong,
        most: CROWD,
        longest: std::time::Duration::ZERO,
    })
}

pub fn checked(
    shape: Netlist,
    exam: &Exam,
    gates: &[u32],
) -> Result<Found, Box<dyn std::error::Error>> {
    judged(shape, exam, gates)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Effort {
    pub crowd: u32,
    pub rounds: u32,
    pub seed: u32,
    pub stall: u32,
    pub shake: u32,
    pub submitted: u32,
}

pub const STALL: u32 = 300;

pub const SHAKE: u32 = 6;

impl Effort {
    #[must_use]
    pub const fn of(crowd: u32, rounds: u32) -> Self {
        Self {
            crowd,
            rounds,
            seed: SEEDING,
            stall: STALL,
            shake: SHAKE,
            submitted: u32::MAX,
        }
    }
}

#[must_use]
pub fn shaken(shape: Netlist, held: &[u32], shake: u32, seed: u32) -> Vec<u32> {
    let mut rolls = Rolls::from(seed);
    let mut roll = || rolls.rolled();
    let gates = shape.gates.max(1) as usize;
    let mut held = held.to_vec();
    for one in held.chunks_mut(gates) {
        for _ in 0..shake {
            let at = roll() % shape.gates.max(1);
            let place = roll();
            if let Some(word) = one.get_mut(at as usize) {
                *word = turned(&shape, at, Gate::unpack(*word), place).pack();
            }
        }
    }
    held
}

pub const LOOK: u32 = 100;

pub const MACHINE_LOOK: u32 = 50;

pub const SUBMISSION: std::time::Duration = std::time::Duration::from_millis(100);

pub const ROUND_MOST: std::time::Duration = std::time::Duration::from_secs(1);

pub const GROWN: u32 = 4;

struct Climb {
    shape: Netlist,
    many: u32,
    full: u32,
    look: u32,
    work: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pace {
    work: u64,
    most: u32,
    each: Option<f64>,
}

impl Pace {
    #[must_use]
    pub const fn over(work: u64, most: u32) -> Self {
        Self {
            work: if work == 0 { 1 } else { work },
            most: if most == 0 { 1 } else { most },
            each: None,
        }
    }

    #[must_use]
    pub fn next(&self, left: u32) -> u32 {
        let Some(each) = self.each else {
            return 1.min(left);
        };
        let round = each * self.work as f64;
        let fits = if round > 0.0 {
            (SUBMISSION.as_secs_f64() / round).floor()
        } else {
            f64::from(u32::MAX)
        };
        let fits = if fits >= f64::from(u32::MAX) {
            u32::MAX
        } else {
            fits as u32
        };
        fits.min(self.most).min(left).max(1.min(left))
    }

    pub fn learn(&mut self, round: std::time::Duration) {
        self.each = Some(round.as_secs_f64() / self.work as f64);
    }

    #[must_use]
    pub const fn measured(&self) -> Option<f64> {
        self.each
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gauge {
    pub round: std::time::Duration,
    pub most: u32,
    pub probes: u32,
    pub longest: std::time::Duration,
}

fn spent_on(resident: &Resident<'_>, wall: std::time::Duration) -> std::time::Duration {
    resident.on_device().map_or(wall, |device| device.max(wall))
}

fn gauged(
    resident: &mut Resident<'_>,
    width: u32,
    many: u32,
) -> Result<Gauge, Box<dyn std::error::Error>> {
    let total = many.div_ceil(width).max(1);
    let quarter = ROUND_MOST.as_secs_f64() / 4.0;
    let mut groups = 1_u32;
    let mut probes = 0_u32;
    let mut longest = std::time::Duration::ZERO;
    loop {
        let part = [Groups::Fixed(groups)];
        let stages: Vec<&[Groups]> = vec![&part, &part, &part];
        let began = std::time::Instant::now();
        let wall = resident.staged_frames(&stages, 1)?;
        longest = longest.max(began.elapsed());
        probes += 1;
        let spent = resident.on_device().unwrap_or(wall).as_secs_f64();
        let each = spent / f64::from(groups);
        let grown = f64::from(groups.saturating_mul(GROWN).min(total));
        let next = if each > 0.0 {
            (quarter / each).floor().min(grown)
        } else {
            grown
        };
        if groups < total && next > f64::from(groups) {
            groups = next as u32;
            continue;
        }
        let round = each * f64::from(total);
        let fits = if each > 0.0 {
            (ROUND_MOST.as_secs_f64() / each).floor()
        } else {
            f64::INFINITY
        };
        let most = if fits >= f64::from(u32::MAX / width) {
            u32::MAX / width * width
        } else {
            fits as u32 * width
        };
        return Ok(Gauge {
            round: std::time::Duration::from_secs_f64(round),
            most,
            probes,
            longest,
        });
    }
}

#[derive(Debug)]
pub struct Overlong {
    pub crowd: u32,
    pub most: u32,
    pub round: std::time::Duration,
}

impl std::fmt::Display for Overlong {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            out,
            "one round over a crowd of {} would keep this device busy for {:.0} ms in one go, measured on it, over the {} ms a submission may take before a driver resets the device; this device holds a crowd of at most {} at this shape",
            self.crowd,
            self.round.as_secs_f64() * 1e3,
            ROUND_MOST.as_millis(),
            self.most
        )
    }
}

impl std::error::Error for Overlong {}

fn refused_overlong(held: &Overlong) -> Box<dyn std::error::Error> {
    Box::new(Refusal::new(Code::TooMany, "$.crowd", held.to_string()).limited(counted(held.most)))
}

fn leader(best: &[u32]) -> (usize, u32) {
    best.iter()
        .copied()
        .enumerate()
        .max_by_key(|(_, one)| *one)
        .unwrap_or((0, 0))
}

pub trait Submits {
    fn submitted(&mut self, rounds: u32)
    -> Result<std::time::Duration, Box<dyn std::error::Error>>;
}

pub struct Staged<'held, 'host> {
    resident: &'held mut Resident<'host>,
    stages: &'held [&'held [Groups]],
}

impl<'held, 'host> Staged<'held, 'host> {
    pub const fn new(
        resident: &'held mut Resident<'host>,
        stages: &'held [&'held [Groups]],
    ) -> Self {
        Self { resident, stages }
    }
}

impl Submits for Staged<'_, '_> {
    fn submitted(
        &mut self,
        rounds: u32,
    ) -> Result<std::time::Duration, Box<dyn std::error::Error>> {
        let wall = self.resident.staged_frames(self.stages, rounds)?;
        Ok(spent_on(self.resident, wall))
    }
}

pub fn walked(
    submits: &mut impl Submits,
    pace: &mut Pace,
    rounds: u32,
) -> Result<std::time::Duration, Box<dyn std::error::Error>> {
    let mut gone = 0_u32;
    let mut longest = std::time::Duration::ZERO;
    while gone < rounds {
        let step = pace.next(rounds - gone);
        let began = std::time::Instant::now();
        let round = submits.submitted(step)?;
        longest = longest.max(began.elapsed());
        pace.learn(round);
        gone += step;
    }
    Ok(longest)
}

fn climbed(
    resident: &mut Resident<'_>,
    stages: &[&[Groups]],
    climb: &Climb,
    effort: Effort,
    restart: u32,
    gauge: Gauge,
) -> Result<(u32, Vec<u32>, std::time::Duration), Box<dyn std::error::Error>> {
    let gates = climb.shape.gates as usize;
    let mut longest = gauge.longest;
    let room = climb.many as usize * gates;
    let mut pace = Pace::over(climb.work, effort.submitted);
    pace.learn(gauge.round);
    let mut done = 0_u32;
    let mut top = 0_u32;
    let mut since = 0_u32;
    let mut shakes = 0_u32;
    let mut kept: Option<(u32, Vec<u32>)> = None;
    let mut best = Vec::new();
    while done < effort.rounds {
        let step = climb.look.min(effort.rounds - done);
        longest = longest.max(walked(&mut Staged::new(resident, stages), &mut pace, step)?);
        done += step;
        best = resident.read(BEST as usize, climb.many as usize)?;
        let (who, now) = leader(&best);
        if now >= climb.full {
            break;
        }
        let taken = best.iter().all(|one| *one == now);
        if now > top || !taken {
            top = top.max(now);
            since = 0;
        } else {
            since = since.saturating_add(step);
        }
        if effort.stall > 0 && since >= effort.stall && done < effort.rounds {
            let held = resident.read(HELD as usize, room)?;
            if kept.as_ref().is_none_or(|(score, _)| now > *score) {
                let from = who * gates;
                kept = Some((now, held.get(from..from + gates).unwrap_or(&[]).to_vec()));
            }
            shakes += 1;
            let seed = mixed(restarted(effort.seed, restart) ^ mixed(shakes));
            resident.write(
                HELD as usize,
                &shaken(climb.shape, &held, effort.shake, seed),
            )?;
            resident.write(BEST as usize, &vec![0_u32; climb.many as usize])?;
            top = 0;
            since = 0;
        }
    }
    let (who, now) = leader(&best);
    if let Some((score, gates)) = kept
        && score > now
    {
        return Ok((done, gates, longest));
    }
    let held = resident.read(HELD as usize, room)?;
    let from = who * gates;
    Ok((
        done,
        held.get(from..from + gates).unwrap_or(&[]).to_vec(),
        longest,
    ))
}

pub fn sought(
    host: &Host,
    shape: Netlist,
    exam: &Exam,
    effort: Effort,
    restart: u32,
) -> Result<Found, Box<dyn std::error::Error>> {
    sought_within(host, shape, exam, effort, restart)?.map_err(|held| refused_overlong(&held))
}

fn sought_within(
    host: &Host,
    shape: Netlist,
    exam: &Exam,
    effort: Effort,
    restart: u32,
) -> Result<Result<Found, Overlong>, Box<dyn std::error::Error>> {
    let width = host.width();
    let many = whole(effort.crowd, width);
    let turn = turned_on(width, shape)?;
    let mark = marked(width, shape)?;
    let pick = chosen(width, shape, SIDE, many)?;
    let modules: Vec<&[u32]> = vec![&turn, &mark, &pick];
    let sizes = room(shape, many)?;
    let mut resident = host.resident(&sizes, &modules)?;

    let (crowd, seeds) = opening(shape, effort.seed, restart, many);
    resident.write(HELD as usize, &crowd)?;
    for binding in [TRIAL, VALUES, SCORE, BEST] {
        let held = sizes.get(binding as usize).copied().unwrap_or(0);
        resident.write(binding as usize, &vec![0_u32; held])?;
    }
    resident.write(WANTED as usize, &columns(exam))?;
    resident.write(SEED as usize, &seeds)?;
    let gauge = gauged(&mut resident, width, many)?;
    if gauge.round > ROUND_MOST {
        return Ok(Err(Overlong {
            crowd: many,
            most: gauge.most,
            round: gauge.round,
        }));
    }
    resident.write(HELD as usize, &crowd)?;
    resident.write(SEED as usize, &seeds)?;
    resident.write(BEST as usize, &vec![0_u32; many as usize])?;

    let over = Groups::Fixed(many.div_ceil(width));
    let turning = [over];
    let marking = [over];
    let choosing = [over];
    let stages: Vec<&[Groups]> = vec![&turning, &marking, &choosing];

    let climb = Climb {
        shape,
        many,
        full: exam.marks(),
        look: LOOK,
        work: u64::from(many) * u64::from(shape.gates) * u64::from(words_of(shape)),
    };
    let (done, gates, longest) = climbed(&mut resident, &stages, &climb, effort, restart, gauge)?;
    let mut found = judged(shape, exam, &gates)?;
    found.rounds = done;
    found.crowd = many;
    found.most = gauge.most;
    found.longest = longest;
    Ok(Ok(found))
}

pub fn pressed(
    host: &Host,
    shape: Netlist,
    exam: &Exam,
    effort: Effort,
) -> Result<Found, Box<dyn std::error::Error>> {
    let mut effort = effort;
    let mut restart = 0_u32;
    let mut before: Option<Found> = None;
    loop {
        let mut found = match sought_within(host, shape, exam, effort, restart)? {
            Ok(found) => found,
            Err(held) => return before.ok_or_else(|| refused_overlong(&held)),
        };
        if let Some(held) = &before {
            found.longest = found.longest.max(held.longest);
        }
        let most = CROWD.min(found.most);
        if found.marks >= found.full || effort.crowd >= most {
            return Ok(found);
        }
        effort.crowd = effort.crowd.saturating_mul(2).min(most);
        restart += 1;
        before = Some(found);
    }
}

pub const PATIENCE: u32 = 2;

pub struct Rung {
    pub gates: u32,
    pub found: Found,
}

pub fn least(
    host: &Host,
    shape: Netlist,
    table: &[u32],
    effort: Effort,
) -> Result<Vec<Rung>, Box<dyn std::error::Error>> {
    let mut rungs = Vec::new();
    let mut gates = shape.gates;
    let mut missed = 0;
    while gates >= shape.outputs.max(1) && missed < PATIENCE && rungs.len() < RUNGS {
        let narrowed = Netlist::over(shape.inputs, shape.outputs, gates, shape.kinds)?;
        let exam = Exam::of(narrowed, table.to_vec())?;
        let found = pressed(host, narrowed, &exam, effort)?;
        let solved = found.marks >= found.full;
        rungs.push(Rung { gates, found });
        if solved {
            missed = 0;
        } else {
            missed += 1;
        }
        gates -= 1;
    }
    Ok(rungs)
}

pub type Runs = Vec<(Vec<u32>, Vec<u32>)>;

pub struct Ran {
    pub marks: u32,
    pub full: u32,
    pub rounds: u32,
    pub crowd: u32,
    pub traces: u32,
    pub gates: Vec<Gate>,
    pub said: Vec<Vec<u32>>,
    pub wrong: Vec<(u32, u32)>,
    pub most: u32,
    pub longest: std::time::Duration,
}

fn laid(held: Machine, traces: &[(Vec<u32>, Vec<u32>)], wide: u32) -> (Vec<u32>, Vec<u32>) {
    let many = wide * 32;
    let mut feed = vec![0_u32; held.steps as usize * held.inputs as usize * wide as usize];
    let mut want = vec![0_u32; held.steps as usize * held.outputs as usize * wide as usize];
    for slot in 0..many {
        let Some((given, wanted)) = traces.get(slot as usize % traces.len().max(1)) else {
            continue;
        };
        let word = slot / 32;
        let bit = slot % 32;
        for tick in 0..held.steps {
            let one = given.get(tick as usize).copied().unwrap_or(0);
            for input in 0..held.inputs {
                let at = ((tick * held.inputs + input) * wide + word) as usize;
                if let Some(held) = feed.get_mut(at) {
                    *held |= ((one >> input) & 1) << bit;
                }
            }
            let one = wanted.get(tick as usize).copied().unwrap_or(0);
            for output in 0..held.outputs {
                let at = ((tick * held.outputs + output) * wide + word) as usize;
                if let Some(held) = want.get_mut(at) {
                    *held |= ((one >> output) & 1) << bit;
                }
            }
        }
    }
    (feed, want)
}

pub fn marked_machine(
    held: Machine,
    traces: &[(Vec<u32>, Vec<u32>)],
    gates: &[u32],
) -> Result<Ran, Box<dyn std::error::Error>> {
    let shape = held.shape()?;
    let chip = Chip::of(shape, gates.to_vec(), held.named())?;
    told_of(held, traces, &chip, gates, 0, 0)
}

fn told_of(
    held: Machine,
    traces: &[(Vec<u32>, Vec<u32>)],
    chip: &Chip,
    gates: &[u32],
    rounds: u32,
    crowd: u32,
) -> Result<Ran, Box<dyn std::error::Error>> {
    let mut marks = 0_u32;
    let mut said = Vec::new();
    let mut wrong = Vec::new();
    for (at, (given, wanted)) in traces.iter().enumerate() {
        let answers = held.ran(chip, given);
        marks += held.scored(chip, given, wanted);
        for (tick, one) in answers.iter().enumerate() {
            if wanted.get(tick).copied() != Some(*one) {
                wrong.push((u32::try_from(at)?, u32::try_from(tick)?));
            }
        }
        said.push(answers);
    }
    Ok(Ran {
        marks,
        full: held.marks() * u32::try_from(traces.len())?,
        rounds,
        crowd,
        traces: u32::try_from(traces.len())?,
        gates: gates.iter().map(|one| Gate::unpack(*one)).collect(),
        said,
        wrong,
        most: CROWD,
        longest: std::time::Duration::ZERO,
    })
}

pub fn machined(
    host: &Host,
    held: Machine,
    traces: &[(Vec<u32>, Vec<u32>)],
    effort: Effort,
    restart: u32,
) -> Result<Ran, Box<dyn std::error::Error>> {
    machined_within(host, held, traces, effort, restart)?.map_err(|held| refused_overlong(&held))
}

fn machined_within(
    host: &Host,
    held: Machine,
    traces: &[(Vec<u32>, Vec<u32>)],
    effort: Effort,
    restart: u32,
) -> Result<Result<Ran, Overlong>, Box<dyn std::error::Error>> {
    let shape = held.shape()?;
    let width = host.width();
    let many = whole(effort.crowd, width);
    let wide = u32::try_from(traces.len())?.div_ceil(32).max(1);
    unrolled(held, wide)?;
    let (feed, want) = laid(held, traces, wide);

    let turn = turned_on(width, shape)?;
    let mark = stepped(width, held, wide)?;
    let pick = chosen(width, shape, SIDE, many)?;
    let modules: Vec<&[u32]> = vec![&turn, &mark, &pick];
    let sizes = [
        times(&[many, shape.gates])?,
        times(&[many, shape.gates])?,
        times(&[many, shape.sources(), wide])?,
        want.len().max(1),
        many as usize,
        many as usize,
        many as usize,
        feed.len().max(1),
    ];
    let mut resident = host.resident(&sizes, &modules)?;
    let (crowd, seeds) = opening(shape, effort.seed, restart, many);
    resident.write(HELD as usize, &crowd)?;
    for binding in [TRIAL, VALUES, SCORE, BEST] {
        let room = sizes.get(binding as usize).copied().unwrap_or(0);
        resident.write(binding as usize, &vec![0_u32; room])?;
    }
    resident.write(WANTED as usize, &want)?;
    if !feed.is_empty() {
        resident.write(FEED as usize, &feed)?;
    }
    resident.write(SEED as usize, &seeds)?;
    let gauge = gauged(&mut resident, width, many)?;
    if gauge.round > ROUND_MOST {
        return Ok(Err(Overlong {
            crowd: many,
            most: gauge.most,
            round: gauge.round,
        }));
    }
    resident.write(HELD as usize, &crowd)?;
    resident.write(SEED as usize, &seeds)?;
    resident.write(BEST as usize, &vec![0_u32; many as usize])?;

    let over = Groups::Fixed(many.div_ceil(width));
    let turning = [over];
    let marking = [over];
    let choosing = [over];
    let stages: Vec<&[Groups]> = vec![&turning, &marking, &choosing];

    let climb = Climb {
        shape,
        many,
        full: held.marks() * wide * 32,
        look: MACHINE_LOOK,
        work: u64::from(many)
            * u64::from(held.steps)
            * (u64::from(held.gates)
                + u64::from(held.state)
                + u64::from(held.inputs)
                + u64::from(held.outputs))
            * u64::from(wide),
    };
    let (done, gates, longest) = climbed(&mut resident, &stages, &climb, effort, restart, gauge)?;
    let chip = Chip::of(shape, gates.clone(), held.named())?;
    let mut ran = told_of(held, traces, &chip, &gates, done, many)?;
    ran.most = gauge.most;
    ran.longest = longest;
    Ok(Ok(ran))
}

pub fn pressed_machine(
    host: &Host,
    held: Machine,
    traces: &[(Vec<u32>, Vec<u32>)],
    effort: Effort,
) -> Result<Ran, Box<dyn std::error::Error>> {
    let mut effort = effort;
    let mut restart = 0_u32;
    let mut before: Option<Ran> = None;
    loop {
        let mut ran = match machined_within(host, held, traces, effort, restart)? {
            Ok(ran) => ran,
            Err(over) => return before.ok_or_else(|| refused_overlong(&over)),
        };
        if let Some(earlier) = &before {
            ran.longest = ran.longest.max(earlier.longest);
        }
        let most = CROWD.min(ran.most);
        if ran.marks >= ran.full || effort.crowd >= most {
            return Ok(ran);
        }
        effort.crowd = effort.crowd.saturating_mul(2).min(most);
        restart += 1;
        before = Some(ran);
    }
}

fn told(found: &Found, sought: bool) -> Json {
    let gates = Json::List(
        found
            .gates
            .iter()
            .map(|one| {
                map(vec![
                    ("kind", counted(one.kind)),
                    ("left", counted(one.left)),
                    ("right", counted(one.right)),
                ])
            })
            .collect(),
    );
    let mut held = vec![
        ("solved", Json::Bool(found.marks >= found.full)),
        ("marks", counted(found.marks)),
        ("of", counted(found.full)),
        ("gates", gates),
        (
            "table",
            Json::List(found.table.iter().map(|one| counted(*one)).collect()),
        ),
        (
            "wrong_rows",
            Json::List(found.wrong.iter().map(|one| counted(*one)).collect()),
        ),
        ("checked_on_the_host", Json::Bool(true)),
    ];
    if sought {
        held.push(("rounds", counted(found.rounds)));
        held.push(("crowd", counted(found.crowd)));
    }
    map(held)
}

fn malformed(field: String, what: String) -> Box<dyn std::error::Error> {
    Box::new(Refusal::new(Code::Malformed, &field, what))
}

fn numbers(held: &Json, place: &str, key: &str) -> Result<Vec<u32>, Box<dyn std::error::Error>> {
    let Some(one) = held.get(key) else {
        return Ok(Vec::new());
    };
    let list = one.list().ok_or_else(|| {
        malformed(
            format!("{place}.{key}"),
            format!("{key} is {}, which is not a list", shown_value(one)),
        )
    })?;
    list.iter()
        .enumerate()
        .map(|(at, one)| {
            one.int()
                .and_then(|one| u32::try_from(one).ok())
                .ok_or_else(|| {
                    malformed(
                        format!("{place}.{key}[{at}]"),
                        format!(
                            "{key} entry {at} is {}, which is not a whole number from 0 to {}",
                            shown_value(one),
                            u32::MAX
                        ),
                    )
                })
        })
        .collect()
}

fn widest(bits: u32) -> u32 {
    u32::try_from((1_u64 << bits.min(32)) - 1).unwrap_or(u32::MAX)
}

fn fitting(
    words: &[u32],
    place: &str,
    steps: u32,
    bits: u32,
    what: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if words.len() != steps as usize {
        return Err(beyond(
            place.to_owned(),
            format!(
                "{what} holds {} steps where the machine runs {steps}; every run must give one word a step for its feed and its want",
                words.len()
            ),
            steps,
        ));
    }
    let most = widest(bits);
    for (at, word) in words.iter().enumerate() {
        if *word > most {
            return Err(beyond(
                format!("{place}[{at}]"),
                format!("{what} step {at} is {word}, which sets a bit past the {bits} it has"),
                most,
            ));
        }
    }
    Ok(())
}

fn present(held: &Json, key: &str) -> Result<(), Box<dyn std::error::Error>> {
    if held.get(key).is_none() {
        return Err(malformed(
            format!("$.{key}"),
            format!("{key} is missing from {}", shown_value(held)),
        ));
    }
    Ok(())
}

fn gated(
    held: &Json,
    gates: &Whole,
    least: u32,
    reads: u32,
    written_by: &str,
) -> Result<u32, Box<dyn std::error::Error>> {
    if let Some(value) = held.get(gates.key).and_then(Json::int)
        && (0..i64::from(least)).contains(&value)
    {
        return Err(beyond(
            "$.gates".to_owned(),
            format!(
                "gates is {value}, and the last {least} gates are its {written_by}, so gates must be at least {least}"
            ),
            least,
        ));
    }
    let gates = gates.count(held, "$")?;
    let most = SOURCES - reads;
    if gates > most {
        return Err(beyond(
            "$.gates".to_owned(),
            format!(
                "gates is {gates}, and the {reads} inputs plus the gates must fit the {SOURCES} sources a gate can name, so gates may be at most {most}"
            ),
            most,
        ));
    }
    Ok(gates)
}

fn asked(held: &Json, inputs: &Whole) -> Result<(Netlist, Exam), Box<dyn std::error::Error>> {
    let inputs = inputs.count(held, "$")?;
    let outputs = OUTPUTS_WHOLE.count(held, "$")?;
    let gates = gated(held, &GATES_WHOLE, outputs, inputs, "outputs")?;
    let shape = Netlist::over(inputs, outputs, gates, KINDS_HELD)?;
    present(held, "table")?;
    let table = numbers(held, "$", "table")?;
    let rows = shape.words();
    if table.len() != rows as usize {
        return Err(beyond(
            "$.table".to_owned(),
            format!(
                "the table holds {} rows, and {inputs} inputs need exactly {rows}, one for every combination of the inputs",
                table.len()
            ),
            rows,
        ));
    }
    let most = widest(outputs);
    for (at, row) in table.iter().enumerate() {
        if *row > most {
            return Err(beyond(
                format!("$.table[{at}]"),
                format!("row {at} is {row}, which sets a bit past the {outputs} outputs"),
                most,
            ));
        }
    }
    Ok((shape, Exam::of(shape, table)?))
}

fn given(held: &Json, shape: Netlist) -> Result<Vec<u32>, Box<dyn std::error::Error>> {
    let mut gates = Vec::new();
    let chip = match held.get("chip") {
        None => &[][..],
        Some(one) => one.list().ok_or_else(|| {
            malformed(
                "$.chip".to_owned(),
                format!("chip is {}, which is not a list of gates", shown_value(one)),
            )
        })?,
    };
    if chip.len() != shape.gates as usize {
        return Err(beyond(
            "$.chip".to_owned(),
            format!(
                "chip holds {} gates where the circuit has {}; give exactly one gate for each",
                chip.len(),
                shape.gates
            ),
            shape.gates,
        ));
    }
    for (at, one) in chip.iter().enumerate() {
        let place = format!("$.chip[{at}]");
        let below = shape.below(u32::try_from(at)?);
        let kind = KIND_WHOLE.count(one, &place)?;
        let mut sides = [0_u32; 2];
        for (side, knob) in sides.iter_mut().zip([&LEFT_WHOLE, &RIGHT_WHOLE]) {
            let named = knob.count(one, &place)?;
            if named >= below {
                return Err(beyond(
                    format!("{place}.{}", knob.key),
                    format!(
                        "gate {at} reads source {named} on its {}, and a gate may only read an input or a gate before it, the sources 0 to {}",
                        knob.key,
                        below - 1
                    ),
                    below - 1,
                ));
            }
            *side = named;
        }
        let [left, right] = sides;
        gates.push(Gate { kind, left, right }.pack());
    }
    Ok(gates)
}

#[derive(Debug, Clone, Copy)]
struct Whole {
    key: &'static str,
    least: i64,
    most: i64,
    or: Option<i64>,
    said: &'static str,
}

const WORD: i64 = u32::MAX as i64;

impl Whole {
    fn shown(&self) -> (&'static str, Json) {
        let mut held = vec![
            ("type", text("integer")),
            ("minimum", Json::Int(self.least)),
            ("maximum", Json::Int(self.most)),
        ];
        if let Some(or) = self.or {
            held.push(("default", Json::Int(or)));
        }
        held.push(("description", text(self.said)));
        (self.key, map(held))
    }

    fn read(&self, held: &Json, place: &str) -> Result<i64, Box<dyn std::error::Error>> {
        let field = format!("{place}.{}", self.key);
        let Some(one) = held.get(self.key) else {
            return self.or.ok_or_else(|| {
                malformed(
                    field,
                    format!("{} is missing from {}", self.key, shown_value(held)),
                )
            });
        };
        match one.int() {
            Some(value) if (self.least..=self.most).contains(&value) => Ok(value),
            Some(value) => Err(Box::new(
                Refusal::new(
                    Code::Malformed,
                    &field,
                    format!(
                        "{} is {value}, outside {} to {}",
                        self.key, self.least, self.most
                    ),
                )
                .limited(Json::Int(if value < self.least {
                    self.least
                } else {
                    self.most
                })),
            )),
            None => Err(malformed(
                field,
                format!(
                    "{} is {}, which is not a whole number from {} to {}",
                    self.key,
                    shown_value(one),
                    self.least,
                    self.most
                ),
            )),
        }
    }

    fn count(&self, held: &Json, place: &str) -> Result<u32, Box<dyn std::error::Error>> {
        Ok(u32::try_from(self.read(held, place)?)?)
    }

    fn cell(&self, held: &Json, place: &str) -> Result<u32, Box<dyn std::error::Error>> {
        Ok(i32::try_from(self.read(held, place)?)?.cast_unsigned())
    }
}

fn beyond(field: String, what: String, limit: u32) -> Box<dyn std::error::Error> {
    Box::new(Refusal::new(Code::Malformed, &field, what).limited(counted(limit)))
}

fn flag(held: &Json, place: &str, key: &str, or: bool) -> Result<bool, Box<dyn std::error::Error>> {
    match held.get(key) {
        None => Ok(or),
        Some(Json::Bool(one)) => Ok(*one),
        Some(one) => Err(malformed(
            format!("{place}.{key}"),
            format!("{key} is {}, which is not true or false", shown_value(one)),
        )),
    }
}

fn named_in<'held>(
    held: &'held Json,
    place: &str,
    key: &str,
    among: &[&str],
    or: &'held str,
) -> Result<&'held str, Box<dyn std::error::Error>> {
    let Some(one) = held.get(key) else {
        return Ok(or);
    };
    match one.text() {
        Some(name) if among.contains(&name) => Ok(name),
        _ => Err(malformed(
            format!("{place}.{key}"),
            format!(
                "{key} is {}, which is not one of {}",
                shown_value(one),
                among.join(", ")
            ),
        )),
    }
}

fn listed<'held>(
    held: &'held Json,
    place: &str,
    key: &str,
) -> Result<&'held [Json], Box<dyn std::error::Error>> {
    match held.get(key) {
        None => Ok(&[]),
        Some(one) => one.list().ok_or_else(|| {
            malformed(
                format!("{place}.{key}"),
                format!("{key} is {}, which is not a list", shown_value(one)),
            )
        }),
    }
}

fn choice(key: &'static str, among: &[&str], or: &str, said: &'static str) -> (&'static str, Json) {
    (
        key,
        map(vec![
            ("type", text("string")),
            (
                "enum",
                Json::List(among.iter().map(|one| text(one)).collect()),
            ),
            ("default", text(or)),
            ("description", text(said)),
        ]),
    )
}

fn yes_or_no(key: &'static str, or: bool, said: &'static str) -> (&'static str, Json) {
    (
        key,
        map(vec![
            ("type", text("boolean")),
            ("default", Json::Bool(or)),
            ("description", text(said)),
        ]),
    )
}

fn object(properties: Vec<(&'static str, Json)>, required: &[&str]) -> Json {
    map(vec![
        ("type", text("object")),
        ("properties", map(properties)),
        (
            "required",
            Json::List(required.iter().map(|one| text(one)).collect()),
        ),
    ])
}

fn array(items: Json, said: &str) -> Json {
    map(vec![
        ("type", text("array")),
        ("items", items),
        ("description", text(said)),
    ])
}

const CROWDED: Whole = Whole {
    key: "crowd",
    least: 64,
    most: CROWD as i64,
    or: Some(CROWD as i64),
    said: "how many candidates to start the search with, rounded up to whole subgroups of the device; while it has not solved, the search doubles this on its own up to 65536, or up to the largest crowd whose round this device runs within a second, measured on it, and the crowd in the answer is what it actually spent; a starting crowd whose round would run longer is refused with that largest crowd as its limit",
};

const GENERATIONS: Whole = Whole {
    key: "rounds",
    least: 1,
    most: 1 << 20,
    or: Some(ROUNDS as i64),
    said: "how many generations to allow, at each rung of a ladder",
};

const SEEDED: Whole = Whole {
    key: "seed",
    least: 0,
    most: WORD,
    or: Some(SEEDING as i64),
    said: "where the search starts; the same seed and arguments give the same answer, and each time the search raises its own crowd it starts again from a fresh seed drawn from this one",
};

const STALLED: Whole = Whole {
    key: "stall",
    least: 0,
    most: 1 << 20,
    or: Some(STALL as i64),
    said: "how many rounds the whole crowd may stand on one score before every candidate is shaken out of where it stands, the best so far kept aside; 0 never shakes",
};

const SHAKEN: Whole = Whole {
    key: "shake",
    least: 0,
    most: SHAKES as i64,
    or: Some(SHAKE as i64),
    said: "how many changes each candidate takes at once when the crowd is shaken, so it can cross a ridge that one change at a time cannot",
};

const KINDS_HELD: u32 = 16;

const SYNTH_INPUTS: Whole = Whole {
    key: "inputs",
    least: 1,
    most: MARKED_INPUTS as i64,
    or: None,
    said: "how many inputs; the table grows as two to this power, so every added input doubles the rows to mark, and the device marks at most 4096 rows",
};

const CHECK_INPUTS: Whole = Whole {
    key: "inputs",
    least: 1,
    most: INPUTS as i64,
    or: None,
    said: "how many inputs; the table grows as two to this power, so every added input doubles the rows to mark",
};

const OUTPUTS_WHOLE: Whole = Whole {
    key: "outputs",
    least: 1,
    most: OUTPUTS as i64,
    or: None,
    said: "how many outputs; each is one of the last gates, so gates must be at least this",
};

const GATES_WHOLE: Whole = Whole {
    key: "gates",
    least: 1,
    most: SOURCES as i64 - 1,
    or: None,
    said: "how many two-input gates it may use; at least the outputs, and the inputs plus the gates at most 4096",
};

const STATE_WHOLE: Whole = Whole {
    key: "state",
    least: 0,
    most: INPUTS as i64,
    or: None,
    said: "how many bits it may remember; the state plus the inputs at most 31, and the state plus the outputs at most 32",
};

const MACHINE_INPUTS: Whole = Whole {
    key: "inputs",
    least: 0,
    most: INPUTS as i64,
    or: None,
    said: "how many inputs it reads each step; the state plus the inputs from 1 to 31",
};

const MACHINE_OUTPUTS: Whole = Whole {
    key: "outputs",
    least: 1,
    most: OUTPUTS as i64,
    or: None,
    said: "how many outputs it writes each step; the state plus the outputs at most 32",
};

const MACHINE_GATES: Whole = Whole {
    key: "gates",
    least: 1,
    most: SOURCES as i64 - 1,
    or: None,
    said: "how many two-input gates it may use; at least the state plus the outputs, and the state, the inputs and the gates together at most 4096",
};

const KIND_WHOLE: Whole = Whole {
    key: "kind",
    least: 0,
    most: KINDS_HELD as i64 - 1,
    or: None,
    said: "which of the sixteen two-input functions, as its truth table",
};

const LEFT_WHOLE: Whole = Whole {
    key: "left",
    least: 0,
    most: SOURCES as i64 - 2,
    or: None,
    said: "the source it reads first: an input, then the gates before it, so below the inputs plus its own place",
};

const RIGHT_WHOLE: Whole = Whole {
    key: "right",
    least: 0,
    most: SOURCES as i64 - 2,
    or: None,
    said: "the source it reads second, below the inputs plus its own place",
};

const STEPS: Whole = Whole {
    key: "steps",
    least: 1,
    most: crate::machine::STEPS as i64,
    or: None,
    said: "how many steps a run lasts; taken from the longest run given when it is left out. Steps are worth more than runs: the same count of marks spread over longer runs holds on unseen runs where short ones do not",
};

const fn sized(key: &'static str, least: i64, most: i64, or: i64, said: &'static str) -> Whole {
    Whole {
        key,
        least,
        most,
        or: Some(or),
        said,
    }
}

const fn needed_whole(key: &'static str, said: &'static str) -> Whole {
    Whole {
        key,
        least: 0,
        most: WORD,
        or: None,
        said,
    }
}

const SLOPE_WIDE: Whole = sized(
    "wide",
    2,
    256,
    32,
    "cells across; rounded up to a power of two",
);

const SLOPE_DEEP: Whole = sized(
    "deep",
    2,
    256,
    32,
    "cells down; rounded up to a power of two",
);

const POUR_WIDE: Whole = sized(
    "wide",
    16,
    1024,
    128,
    "cells across; rounded up to a power of two",
);

const POUR_DEEP: Whole = sized(
    "deep",
    16,
    1024,
    128,
    "cells down; rounded up to a power of two",
);

const DEPTH: Whole = sized("depth", 1, 1 << 12, 16, "how many frames to walk");

const LOUDEST: Whole = sized(
    "most",
    1,
    1 << 12,
    32,
    "how many cells to name in each answer, loudest first",
);

const TURNING: Whole = sized(
    "turning",
    0,
    WORD,
    KNOB as i64,
    "the first rule parameter, the one the gradient calls turning",
);

const SWING: Whole = sized("swing", 0, WORD, 0, "the second rule parameter");

const AT: Whole = Whole {
    key: "at",
    least: 0,
    most: WORD,
    or: None,
    said: "the cell whose weight is carried back, numbered row by row from the top left; the middle of the grid if left out, and it must lie inside the grid",
};

const FILL: Whole = sized(
    "fill",
    i32::MIN as i64,
    i32::MAX as i64,
    0,
    "what every cell holds before the named ones are set; a cell holds a signed 32-bit number, written as the tools answer it",
);

const IS: Whole = sized(
    "is",
    i32::MIN as i64,
    i32::MAX as i64,
    1,
    "what the cell holds, a signed 32-bit number exactly as the tools answer a cell",
);

const PLACE: Whole = needed_whole(
    "at",
    "the cell, numbered row by row from the top left; it must lie inside the grid",
);

const SETTLE_ROUNDS: Whole = sized(
    "rounds",
    1,
    1 << 10,
    8,
    "how many rounds to walk, each one forward walk and one walk back",
);

const STEP: Whole = sized(
    "step",
    1,
    1 << 16,
    1,
    "how far a parameter moves in one round to start with; they are whole numbers, so this is whole too, and it halves each time both ways along the slope came out worse",
);

const CEILING: Whole = sized(
    "ceiling",
    0,
    WORD,
    1 << 14,
    "how high a parameter may climb; the starting turning and swing may not be above it",
);

const FRAMES: Whole = sized("frames", 1, 1 << 14, 200, "how many frames to step");

const BATCH: Whole = sized(
    "batch",
    1,
    256,
    20,
    "how many frames go to the device at a time",
);

const STANDING: Whole = sized(
    "most",
    1,
    1 << 14,
    64,
    "how many settled cells to name one by one",
);

const SPAN: Whole = sized("span", 0, WORD, 49, "how wide the spout pours");

const SPOUT: Whole = Whole {
    key: "at",
    least: 0,
    most: WORD,
    or: None,
    said: "the cell the spout pours from, numbered row by row from the top left; it must lie inside the grid, and without it nothing pours",
};

const POURING: Whole = Whole {
    key: "frames",
    least: 0,
    most: WORD,
    or: None,
    said: "how many frames the spout pours before it shuts off; half the frames of the run when left out, and never more than they are",
};

fn whole_item() -> Json {
    map(vec![
        ("type", text("integer")),
        ("minimum", Json::Int(0)),
        ("maximum", Json::Int(WORD)),
    ])
}

fn gate() -> Json {
    object(
        vec![KIND_WHOLE.shown(), LEFT_WHOLE.shown(), RIGHT_WHOLE.shown()],
        &["kind", "left", "right"],
    )
}

fn cell_list(said: &str) -> Json {
    array(object(vec![PLACE.shown(), IS.shown()], &["at"]), said)
}

fn tool(name: &str, said: &str, schema: Json) -> Json {
    map(vec![
        ("name", text(name)),
        ("description", text(said)),
        ("inputSchema", schema),
    ])
}

pub(crate) fn tools() -> Json {
    let table = (
        "table",
        array(
            whole_item(),
            "one word a row, the wanted output bits, lowest output in bit zero, below two to the outputs; exactly two to the inputs rows",
        ),
    );
    let shape = vec![
        SYNTH_INPUTS.shown(),
        OUTPUTS_WHOLE.shown(),
        GATES_WHOLE.shown(),
        table,
    ];
    let search = [
        CROWDED.shown(),
        GENERATIONS.shown(),
        SEEDED.shown(),
        STALLED.shown(),
        SHAKEN.shown(),
    ];
    let mut seeking = shape.clone();
    seeking.extend(search.iter().cloned());
    let mut checking = shape.clone();
    if let Some(first) = checking.first_mut() {
        *first = CHECK_INPUTS.shown();
    }
    checking.push((
        "chip",
        array(
            gate(),
            "the gates to check, each an object of kind, left and right",
        ),
    ));
    let trace = array(
        object(
            vec![
                (
                    "feed",
                    array(
                        whole_item(),
                        "one word a step, the input bits for that step, below two to the inputs; exactly steps of them",
                    ),
                ),
                (
                    "want",
                    array(
                        whole_item(),
                        "one word a step, the output bits wanted at that step, below two to the outputs; exactly steps of them",
                    ),
                ),
            ],
            &[],
        ),
        "the runs it must get right, each a feed and the answer wanted",
    );
    let mut running = vec![
        STATE_WHOLE.shown(),
        MACHINE_INPUTS.shown(),
        MACHINE_OUTPUTS.shown(),
        MACHINE_GATES.shown(),
        STEPS.shown(),
        ("traces", trace),
    ];
    running.extend(search.iter().cloned());
    running.push((
        "chip",
        array(
            gate(),
            "the gates of a machine to mark, each an object of kind, left and right; give this and nothing is searched, the machine is only run on the traces and marked, which is how a machine found on one set of runs is tried on runs it has never seen",
        ),
    ));
    let kind = choice(
        "kind",
        &KINDS_NAMED,
        "living",
        "which rule the cells follow",
    );
    let sloping = vec![
        SLOPE_WIDE.shown(),
        SLOPE_DEEP.shown(),
        kind.clone(),
        TURNING.shown(),
        SWING.shown(),
        DEPTH.shown(),
        AT.shown(),
        FILL.shown(),
        (
            "cells",
            cell_list(
                "the cells to set before the walk, each an object of at and is; everything else takes fill",
            ),
        ),
        yes_or_no(
            "carry",
            true,
            "false walks forward only and answers without a slope",
        ),
        LOUDEST.shown(),
    ];
    let settling = vec![
        SLOPE_WIDE.shown(),
        SLOPE_DEEP.shown(),
        kind,
        Whole {
            said: "where the first rule parameter starts",
            ..TURNING
        }
        .shown(),
        Whole {
            said: "where the second rule parameter starts",
            ..SWING
        }
        .shown(),
        Whole {
            said: "how many frames each round walks",
            ..DEPTH
        }
        .shown(),
        SETTLE_ROUNDS.shown(),
        STEP.shown(),
        CEILING.shown(),
        Whole {
            said: "what every cell holds before the named ones are set, in both grids; a cell holds a signed 32-bit number, written as the tools answer it",
            ..FILL
        }
        .shown(),
        (
            "cells",
            cell_list("the grid to start from, each an object of at and is"),
        ),
        (
            "want",
            cell_list(
                "the grid to land on, each an object of at and is, exactly as grit_slope answers ended; this is what the rounds walk towards",
            ),
        ),
    ];
    let stuff = |said: &'static str| choice("is", &STUFF_NAMED, "sand", said);
    let pouring = vec![
        POUR_WIDE.shown(),
        POUR_DEEP.shown(),
        FRAMES.shown(),
        yes_or_no(
            "walls",
            true,
            "a floor and two sides, so falling things land; false lets everything leave by the bottom",
        ),
        (
            "cells",
            array(
                object(vec![PLACE.shown(), stuff("what to place there")], &["at"]),
                "what to place before the first frame, each an object of at and is, where is names one of air, water, sand, wall",
            ),
        ),
        (
            "brush",
            map(vec![
                ("type", text("object")),
                (
                    "description",
                    text(
                        "a spout that pours: at names the cell it pours from, is what it pours, span how wide, frames how long it keeps pouring before it shuts off",
                    ),
                ),
                (
                    "properties",
                    map(vec![
                        SPOUT.shown(),
                        stuff("what the spout pours"),
                        SPAN.shown(),
                        POURING.shown(),
                    ]),
                ),
            ]),
        ),
        STANDING.shown(),
        BATCH.shown(),
    ];
    Json::List(vec![
        tool(
            "grit_machine",
            "Search for a synchronous machine, a circuit with state, that answers every run it is given. A run is a feed of input words and the output words wanted at each step; the machine starts from a state of all zeros. Returns the gates, what the best machine says on each run, and which step of which run it gets wrong, all marked on the host. Pass chip to mark a machine you already hold instead of searching for one. Solved means every run in the request is answered, which is not the same as holding the rule behind them: take the gates it returns, pass them back as chip with runs it has never seen, and read the marks. Runs that last longer pin a machine down on far fewer marks than short runs do.",
            object(running, &["state", "inputs", "outputs", "gates", "traces"]),
        ),
        tool(
            "grit_synthesise",
            "Search for a circuit of two-input gates that answers a truth table exactly. Returns the circuit and the marks it scores, checked on the host, or the best it reached with the rounds and crowd it spent. While it has not solved, the search doubles the crowd itself up to 65536 before giving an answer, so a result that is not solved is evidence about the gate budget rather than about a crowd the caller happened to pick.",
            object(seeking.clone(), &["inputs", "outputs", "gates", "table"]),
        ),
        tool(
            "grit_least",
            "Walk the gate budget down from the one given, searching at each rung, and keep going past a rung that fails until two in a row fail. Solving is not monotone in the budget: a smaller circuit can be found where a larger one was not, so stopping at the first failure names the edge of a hole rather than the floor. Returns the ladder, which shows any holes, the smallest budget that did solve, and its circuit. Each rung doubles its crowd up to 65536 before it gives up, so a rung that fails has been pressed rather than merely tried. A rung that did not solve means not found within the rounds and crowd spent, which the answer carries, and never that no such circuit exists.",
            object(seeking, &["inputs", "outputs", "gates", "table"]),
        ),
        tool(
            "grit_slope",
            "Step a grid of cells forward under one of the rules and carry a weight back through the same walk, which is how far the cell you name depends on everything else. Returns where the grid ended, which cells the weight reached and how loudly, how far it travelled, and the two rule gradients as an exact fraction. The walk back is not a second model of the first: the frames are undone and recomputed, so a depth that would not fit on the card as a tape still runs. Nothing is searched and nothing is fitted; this is the derivative, and what to do with it is the caller's. Every cell value, given or answered, is a signed 32-bit number, so what one call answers another takes back as it is.",
            object(sloping, &[]),
        ),
        tool(
            "grit_settle",
            "Walk a grid forward, see how far it landed from the one you asked for, and move the two rule parameters downhill against that distance. The step is taken from the walk's own adjoint rather than from a guess: the weight carried back is the sign of the residual a cell at a time, so what comes back is the gradient of the distance with respect to the parameters. Whenever the slope gives it no step left to take, it tries every neighbour of its best point at a step of one before it stops. Returns the trail it walked, the nearest it got, and the parameters that got it there. Distance is counted exactly on the host. A run that did not close means these parameters, at this step and this many rounds, did not get nearer - never that none would.",
            object(settling, &["want"]),
        ),
        tool(
            "grit_pour",
            "Pour sand or water into a room and step it until it settles. Returns how much of each thing ended where, how many cells of sand and water lie in each row, and how many quads the work list actually named on an average look. That last number is the point of the engine rather than a detail of it: the frame costs what is moving, not what the grid could hold, so a settled room is cheap however large it is. Nothing is searched and nothing is judged; this is what happened.",
            object(pouring, &[]),
        ),
        tool(
            "grit_check",
            "Mark a given circuit against a truth table and name the rows it gets wrong. Nothing is searched.",
            object(checking, &["inputs", "outputs", "gates", "table", "chip"]),
        ),
    ])
}

fn asked_machine(held: &Json) -> Result<(Machine, Runs), Box<dyn std::error::Error>> {
    present(held, "traces")?;
    let given = match held.get("traces") {
        None => &[][..],
        Some(one) => one.list().ok_or_else(|| {
            malformed(
                "$.traces".to_owned(),
                format!(
                    "traces is {}, which is not a list of runs",
                    shown_value(one)
                ),
            )
        })?,
    };
    let traces: Runs = given
        .iter()
        .enumerate()
        .map(|(at, one)| {
            let place = format!("$.traces[{at}]");
            Ok((numbers(one, &place, "feed")?, numbers(one, &place, "want")?))
        })
        .collect::<Result<_, Box<dyn std::error::Error>>>()?;
    let longest = traces
        .iter()
        .map(|(_, want)| u32::try_from(want.len()).unwrap_or(u32::MAX))
        .max()
        .unwrap_or(0);
    let steps = match held.get("steps") {
        None => longest,
        Some(_) => STEPS.count(held, "$")?,
    };
    let state = STATE_WHOLE.count(held, "$")?;
    let inputs = MACHINE_INPUTS.count(held, "$")?;
    let reads = state + inputs;
    if reads == 0 || reads > INPUTS {
        let limit = if reads == 0 { 1 } else { INPUTS - state };
        return Err(beyond(
            "$.inputs".to_owned(),
            format!(
                "inputs is {inputs} with {state} bits of state, and a machine reads its state and its inputs together, from 1 to {INPUTS} bits"
            ),
            limit,
        ));
    }
    let outputs = MACHINE_OUTPUTS.count(held, "$")?;
    if state + outputs > OUTPUTS {
        return Err(beyond(
            "$.outputs".to_owned(),
            format!(
                "outputs is {outputs} with {state} bits of state, and a machine writes its state and its outputs together, at most {OUTPUTS} bits"
            ),
            OUTPUTS - state,
        ));
    }
    let gates = gated(
        held,
        &MACHINE_GATES,
        state + outputs,
        reads,
        "state bits and outputs",
    )?;
    if steps > crate::machine::STEPS {
        return Err(beyond(
            "$.traces".to_owned(),
            format!(
                "the longest run holds {steps} steps, over the {} one run may hold",
                crate::machine::STEPS
            ),
            crate::machine::STEPS,
        ));
    }
    let machine = Machine::over(state, inputs, outputs, gates, KINDS_HELD, steps)?;
    if traces.is_empty() || machine.steps == 0 {
        return Err(malformed(
            "$.traces".to_owned(),
            "a machine is marked on its runs, and no run with a step in it was given".to_owned(),
        ));
    }
    for (at, (feed, want)) in traces.iter().enumerate() {
        fitting(
            want,
            &format!("$.traces[{at}].want"),
            machine.steps,
            machine.outputs,
            "want",
        )?;
        fitting(
            feed,
            &format!("$.traces[{at}].feed"),
            machine.steps,
            machine.inputs,
            "feed",
        )?;
    }
    Ok((machine, traces))
}

fn kind_named(name: &str) -> Option<Kind> {
    Some(match name {
        "product" => Kind::Product,
        "sum" => Kind::Sum,
        "turn" => Kind::Turn,
        "bound" => Kind::Bound,
        "smooth" => Kind::Smooth,
        "living" => Kind::Living,
        "twinned" => Kind::Twinned,
        _ => return None,
    })
}

pub const KINDS_NAMED: [&str; 7] = [
    "product", "sum", "turn", "bound", "smooth", "living", "twinned",
];

/// A cell and what is in it, so neither the start nor the answer has to spell
/// out every cell of the grid. The engine is sparse and so is this.
fn inside(grid: Grid, at: u32, field: String) -> Result<usize, Box<dyn std::error::Error>> {
    let cells = u32::try_from(grid.cells())?;
    if at >= cells {
        return Err(beyond(
            field,
            format!("at {at} is outside the {cells} cells of this grid"),
            cells - 1,
        ));
    }
    Ok(at as usize)
}

fn sown(
    held: &Json,
    key: &str,
    grid: Grid,
    fill: u32,
) -> Result<Vec<u32>, Box<dyn std::error::Error>> {
    let mut cells = vec![fill; grid.cells()];
    for (place, one) in listed(held, "$", key)?.iter().enumerate() {
        let here = format!("$.{key}[{place}]");
        let at = inside(grid, PLACE.count(one, &here)?, format!("{here}.at"))?;
        let value = IS.cell(one, &here)?;
        if let Some(slot) = cells.get_mut(at) {
            *slot = value;
        }
    }
    Ok(cells)
}

fn rule_named(held: &Json) -> Result<(&str, Kind), Box<dyn std::error::Error>> {
    let named = named_in(held, "$", "kind", &KINDS_NAMED, "living")?;
    let kind = kind_named(named).ok_or_else(|| {
        malformed(
            "$.kind".to_owned(),
            format!(
                "kind {} is not one of {}",
                shown(named),
                KINDS_NAMED.join(", ")
            ),
        )
    })?;
    Ok((named, kind))
}

/// The non-zero entries, loudest first, and no more than `most` of them.
fn loudest(held: &[i32], most: usize, quiet: i32) -> (Json, u32) {
    let mut heard: Vec<(usize, i32)> = held
        .iter()
        .enumerate()
        .filter(|(_, one)| **one != quiet)
        .map(|(at, one)| (at, *one))
        .collect();
    let count = heard.len() as u32;
    heard.sort_by_key(|(at, one)| {
        (
            std::cmp::Reverse((i64::from(*one) - i64::from(quiet)).unsigned_abs()),
            *at,
        )
    });
    heard.truncate(most);
    (
        Json::List(
            heard
                .iter()
                .map(|(at, one)| {
                    map(vec![
                        ("at", counted(*at as u32)),
                        ("is", Json::Int(i64::from(*one))),
                    ])
                })
                .collect(),
        ),
        count,
    )
}

/// How far from `at` the weight reached, in cells, along the grid.
fn reached(grid: Grid, slope: &[i32], at: usize) -> u32 {
    let named = |one: usize| (one as u32 % grid.wide, one as u32 / grid.wide);
    let (home_x, home_y) = named(at);
    slope
        .iter()
        .enumerate()
        .filter(|(_, one)| **one != 0)
        .map(|(one, _)| {
            let (x, y) = named(one);
            x.abs_diff(home_x).max(y.abs_diff(home_y))
        })
        .max()
        .unwrap_or(0)
}

fn device(host: Option<&Host>) -> Result<&Host, Box<dyn std::error::Error>> {
    host.ok_or_else(|| Box::new(crate::host::Error::NoComputeDevice).into())
}

fn slid(host: Option<&Host>, held: &Json) -> Result<Json, Box<dyn std::error::Error>> {
    let wide = SLOPE_WIDE.count(held, "$")?;
    let deep = SLOPE_DEEP.count(held, "$")?;
    let grid = Grid::of(wide, deep);
    let (named, kind) = rule_named(held)?;
    let turning = TURNING.count(held, "$")?;
    let swing = SWING.count(held, "$")?;
    let depth = DEPTH.count(held, "$")?;
    let most = LOUDEST.count(held, "$")? as usize;
    let middle = grid.deep / 2 * grid.wide + grid.wide / 2;
    let at = match held.get("at") {
        None => middle,
        Some(_) => AT.count(held, "$")?,
    };
    let at = inside(grid, at, "$.at".to_owned())?;
    let carrying = flag(held, "$", "carry", true)?;
    let fill = FILL.cell(held, "$")?;
    let cells = sown(held, "cells", grid, fill)?;
    let host = device(host)?;

    let mut board = seated(host, grid, kind, (turning, swing), depth)?;
    let (ended, slope) = if carrying {
        let mut weight = vec![0_u32; grid.cells()];
        let seat = weight.get_mut(at).ok_or("at is outside the grid")?;
        *seat = 1 << POINT;
        // carry answers with the weight it carried back first and the grid it
        // ended on second. Both are as many u32 as there are cells, so taking
        // them the other way round compiles and quietly reports each as the
        // other: the grid looked like a slope that reached every cell in one
        // frame, and the slope looked like a grid.
        let (slope, ended) = board.carry(&cells, &weight)?;
        let slope: Vec<i32> = slope.iter().map(|one| *one as i32).collect();
        (ended, Some(slope))
    } else {
        (board.run(&cells)?, None)
    };

    let settled: Vec<i32> = ended.iter().map(|one| *one as i32).collect();
    let (shown, standing) = loudest(&settled, most, fill.cast_signed());
    let mut answer = vec![
        (
            "grid",
            map(vec![
                ("wide", counted(grid.wide)),
                ("deep", counted(grid.deep)),
                ("cells", counted(grid.cells() as u32)),
            ]),
        ),
        ("kind", text(named)),
        ("depth", counted(depth)),
        ("at", counted(at as u32)),
        ("ended", shown),
        ("standing", counted(standing)),
        ("fill", Json::Int(i64::from(fill.cast_signed()))),
        (
            "ended_means",
            text(
                "every cell that does not hold fill, up to most of them, those furthest from fill first; a cell left unnamed holds fill, so ended passed back as want with the same fill is this grid",
            ),
        ),
        ("checked_on_the_host", Json::Bool(true)),
    ];
    if let Some(slope) = slope {
        let (shown, heard) = loudest(&slope, most, 0);
        let (turning, swing, over) = board.told_parts()?;
        answer.push(("slope", shown));
        answer.push(("heard", counted(heard)));
        answer.push(("reach", counted(reached(grid, &slope, at))));
        answer.push((
            "told",
            map(vec![
                ("turning", Json::Int(turning)),
                ("swing", Json::Int(swing)),
                ("over", Json::Int(over)),
                (
                    "read_it_as",
                    text("turning and swing are each divided by over; they are kept apart so nothing is rounded here"),
                ),
            ]),
        ));
        answer.push((
            "heard_means",
            text("how many cells the weight at `at` reached after `depth` frames, which is what that cell's future is made of"),
        ));
    }
    Ok(map(answer))
}

/// One cell's worth of push, big enough that the weight does not round away.
const PUSH: i32 = 1 << POINT;

/// How far apart two grids are, counted a cell at a time.
fn apart(ended: &[u32], want: &[u32]) -> i64 {
    ended
        .iter()
        .zip(want)
        .map(|(one, other)| (i64::from(*one as i32) - i64::from(*other as i32)).abs())
        .sum()
}

/// The subgradient of that distance: which way each cell wants to move.
///
/// This is what makes the walk back do anything useful. `carry` takes any
/// weight, not only a single cell lit up, so seeding it with the sign of the
/// residual makes `told` the gradient of the distance above with respect to
/// the two rule parameters. Nothing here is a model of the simulation; it is
/// the simulation's own adjoint, asked a different question.
fn pushed(ended: &[u32], want: &[u32]) -> Vec<u32> {
    ended
        .iter()
        .zip(want)
        .map(|(one, other)| {
            let gap = i64::from(*one as i32) - i64::from(*other as i32);
            (match gap.signum() {
                1 => PUSH,
                -1 => -PUSH,
                _ => 0,
            }) as u32
        })
        .collect()
}

/// Downhill, by whole numbers, because the parameters are whole numbers.
/// `against` turns the step around, for when the slope pointed the wrong way.
fn moved(now: u32, slope: i64, step: u32, most: u32, against: bool) -> u32 {
    let down = if against {
        -slope.signum()
    } else {
        slope.signum()
    };
    match down {
        1 => now.saturating_sub(step),
        -1 => now.saturating_add(step).min(most),
        _ => now,
    }
}

fn beside(turning: u32, swing: u32, ceiling: u32) -> Vec<(u32, u32)> {
    let mut all = Vec::new();
    for (across, down) in [
        (1, 1),
        (-1, -1),
        (1, -1),
        (-1, 1),
        (0, 1),
        (0, -1),
        (1, 0),
        (-1, 0),
    ] {
        let one = (
            turning.saturating_add_signed(across).min(ceiling),
            swing.saturating_add_signed(down).min(ceiling),
        );
        if one != (turning, swing) && !all.contains(&one) {
            all.push(one);
        }
    }
    all
}

fn under(held: &Json, knob: &Whole, ceiling: u32) -> Result<u32, Box<dyn std::error::Error>> {
    let value = knob.count(held, "$")?;
    if value > ceiling {
        return Err(beyond(
            format!("$.{}", knob.key),
            format!(
                "{} starts at {value}, above the ceiling of {ceiling} it may climb to",
                knob.key
            ),
            ceiling,
        ));
    }
    Ok(value)
}

fn untried(
    (turning, swing): (u32, u32),
    ceiling: u32,
    seen: &std::collections::BTreeSet<(u32, u32)>,
) -> Vec<(u32, u32)> {
    beside(turning, swing, ceiling)
        .into_iter()
        .filter(|one| !seen.contains(one))
        .collect()
}

fn settled(host: Option<&Host>, held: &Json) -> Result<Json, Box<dyn std::error::Error>> {
    let wide = SLOPE_WIDE.count(held, "$")?;
    let deep = SLOPE_DEEP.count(held, "$")?;
    let grid = Grid::of(wide, deep);
    let (named, kind) = rule_named(held)?;
    let depth = DEPTH.count(held, "$")?;
    let rounds = SETTLE_ROUNDS.count(held, "$")?;
    let step = STEP.count(held, "$")?;
    let ceiling = CEILING.count(held, "$")?;
    let fill = FILL.cell(held, "$")?;
    let cells = sown(held, "cells", grid, fill)?;
    present(held, "want")?;
    let want = sown(held, "want", grid, fill)?;

    let mut turning = under(held, &TURNING, ceiling)?;
    let mut swing = under(held, &SWING, ceiling)?;
    let host = device(host)?;
    let mut board = seated(host, grid, kind, (turning, swing), depth)?;

    let mut trail = Vec::new();
    // The gap it reached, where it was, and which way it was pointing there.
    // The slope is kept with the point because a round that overshoots goes
    // back to the best one and tries again shorter, and it must not re-walk
    // the grid to remember which way downhill was.
    let mut best: Option<(i64, u32, u32, i64, i64)> = None;
    let mut step = step;
    let mut stalled = false;
    // Measured on a target the rule itself made: the distance falls all the way
    // to zero at the parameters that made it, and the walk back agrees about
    // which way that is over most of the range and disagrees over part of it.
    // The distance is exact and the slope is a hint, so a round that went the
    // wrong way tries the other way before it gives up any step.
    let mut against = false;
    let mut turned_around = 0_u32;
    let mut unmoved = 0_u32;
    let mut seen = std::collections::BTreeSet::new();
    let mut probing: Vec<(u32, u32)> = Vec::new();
    let mut probed = 0_u32;
    let mut probing_from = None;
    let mut probe = false;
    for _ in 0..rounds {
        seen.insert((turning, swing));
        if probe {
            probed += 1;
        }
        board.tune(turning, swing)?;
        // Forward once to see where it lands, then again inside carry, which
        // cannot know the residual before the grid has been walked.
        let ended = board.run(&cells)?;
        let gap = apart(&ended, &want);
        let (_, walked) = board.carry(&cells, &pushed(&ended, &want))?;
        let (down_turning, down_swing, over) = board.told_parts()?;
        // Read before the push: the round that just walked has to be compared
        // with the one before it, and after the push the last entry is itself.
        let before = trail
            .last()
            .and_then(|one: &Json| one.get("apart"))
            .and_then(Json::int);
        trail.push(map(vec![
            ("turning", counted(turning)),
            ("swing", counted(swing)),
            ("apart", Json::Int(gap)),
            ("down_turning", Json::Int(down_turning)),
            ("down_swing", Json::Int(down_swing)),
            ("over", Json::Int(over)),
        ]));
        let _ = walked;
        if before.is_some_and(|held| held == gap) {
            unmoved += 1;
        }
        let improved = match best {
            Some((held, ..)) if gap >= held => false,
            _ => {
                best = Some((gap, turning, swing, down_turning, down_swing));
                probing.clear();
                probing_from = None;
                true
            }
        };
        let Some((_, best_turning, best_swing, best_down, best_swung)) = best else {
            break;
        };
        let home = (best_turning, best_swing);
        let mut failed = !improved;
        let next = loop {
            if probing_from == Some(home) {
                break probing.pop();
            }
            if failed {
                failed = false;
                if !against {
                    against = true;
                    turned_around += 1;
                } else if step > 1 {
                    against = false;
                    step /= 2;
                } else {
                    probing = untried(home, ceiling, &seen);
                    probing_from = Some(home);
                    continue;
                }
            }
            let candidate = (
                moved(best_turning, best_down, step, ceiling, against),
                moved(best_swing, best_swung, step, ceiling, against),
            );
            if candidate != home && !seen.contains(&candidate) {
                break Some(candidate);
            }
            if !against {
                against = true;
            } else if step > 1 {
                against = false;
                step /= 2;
            } else {
                probing = untried(home, ceiling, &seen);
                probing_from = Some(home);
            }
        };
        match next {
            Some(one) => {
                probe = probing_from == Some(home);
                (turning, swing) = one;
            }
            None => {
                step = 0;
                stalled = true;
                break;
            }
        }
    }

    let (gap, turning, swing, _, _) = best.ok_or("no round was walked")?;
    let first = trail
        .first()
        .and_then(|one| one.get("apart"))
        .and_then(Json::int);
    Ok(map(vec![
        (
            "grid",
            map(vec![
                ("wide", counted(grid.wide)),
                ("deep", counted(grid.deep)),
                ("cells", counted(grid.cells() as u32)),
            ]),
        ),
        ("kind", text(named)),
        ("depth", counted(depth)),
        ("rounds", counted(trail.len() as u32)),
        ("turning", counted(turning)),
        ("swing", counted(swing)),
        ("apart", Json::Int(gap)),
        ("started_apart", first.map_or(Json::Null, Json::Int)),
        ("closed", Json::Bool(first.is_some_and(|one| gap < one))),
        ("stalled", Json::Bool(stalled)),
        ("turned_around", counted(turned_around)),
        ("unmoved", counted(unmoved)),
        ("probed", counted(probed)),
        ("step", counted(step)),
        ("trail", Json::List(trail)),
        ("checked_on_the_host", Json::Bool(true)),
        (
            "apart_means",
            text(
                "how far the walked grid is from the one asked for, counted a cell at a time; it is the thing the rounds walk downhill, and it is exact",
            ),
        ),
        (
            "a_round_that_did_not_close",
            text(
                "means these rule parameters, at this step and this many rounds, did not get nearer - not that no parameters would. Stalled means a step of one found no neighbour of the best point nearer than it, so more rounds would not help; a walk that ran out of rounds instead would. probed counts the rounds spent on those neighbours. turned_around counts the rounds where the slope pointed uphill and the distance, which is exact, said otherwise. unmoved counts rounds where moving the parameters changed the distance by nothing at all, which means they have no purchase at this depth and these cell values rather than that they are right.",
            ),
        ),
    ]))
}

pub const STUFF_NAMED: [&str; 4] = ["air", "water", "sand", "wall"];

fn stuff_named(name: &str) -> Option<u32> {
    match name {
        "air" => Some(AIR),
        "water" => Some(WATER),
        "sand" => Some(SAND),
        "wall" => Some(WALL),
        _ => None,
    }
}

/// A room with a floor and two sides, which is what a falling thing needs to
/// land on. Without it everything leaves by the bottom and the answer is empty.
fn walled(grid: Grid, walls: bool) -> Vec<u32> {
    let mut cells = vec![AIR; grid.cells()];
    if !walls {
        return cells;
    }
    for row in 0..grid.deep {
        for column in 0..grid.wide {
            let edge = row + 1 >= grid.deep || column == 0 || column + 1 >= grid.wide;
            if edge && let Some(slot) = cells.get_mut((row * grid.wide + column) as usize) {
                *slot = WALL;
            }
        }
    }
    cells
}

fn poured(host: Option<&Host>, held: &Json) -> Result<Json, Box<dyn std::error::Error>> {
    let wide = POUR_WIDE.count(held, "$")?;
    let deep = POUR_DEEP.count(held, "$")?;
    let grid = Grid::of(wide, deep);
    let frames = FRAMES.count(held, "$")?;
    let batch = BATCH.count(held, "$")?;
    let most = STANDING.count(held, "$")? as usize;
    let walls = flag(held, "$", "walls", true)?;
    let mut placed = Vec::new();
    for (place, one) in listed(held, "$", "cells")?.iter().enumerate() {
        let here = format!("$.cells[{place}]");
        let at = inside(grid, PLACE.count(one, &here)?, format!("{here}.at"))?;
        let named = named_in(one, &here, "is", &STUFF_NAMED, "sand")?;
        let stuff = stuff_named(named).ok_or_else(|| {
            malformed(
                format!("{here}.is"),
                format!("{} is not one of {}", shown(named), STUFF_NAMED.join(", ")),
            )
        })?;
        placed.push((at, stuff));
    }
    let brush = match held.get("brush") {
        None => None,
        Some(one @ Json::Map(_)) => Some(one),
        Some(one) => {
            return Err(malformed(
                "$.brush".to_owned(),
                format!("brush is {}, which is not an object", shown_value(one)),
            ));
        }
    };
    let spout = match brush.and_then(|one| one.get("at")) {
        None => None,
        Some(_) => {
            let at = SPOUT.count(brush.unwrap_or(&Json::Null), "$.brush")?;
            let at = inside(grid, at, "$.brush.at".to_owned())?;
            let at = u32::try_from(at)?;
            Some((at % grid.wide, at / grid.wide))
        }
    };
    let empty = map(Vec::new());
    let brushed = brush.unwrap_or(&empty);
    let span = SPAN.count(brushed, "$.brush")?;
    let pouring = match brushed.get("frames") {
        None => frames / 2,
        Some(_) => {
            let pouring = POURING.count(brushed, "$.brush")?;
            if pouring > frames {
                return Err(beyond(
                    "$.brush.frames".to_owned(),
                    format!("the spout pours for {pouring} frames of a run of {frames}"),
                    frames,
                ));
            }
            pouring
        }
    };
    let named = named_in(brushed, "$.brush", "is", &STUFF_NAMED, "sand")?;
    let stuff = stuff_named(named).ok_or_else(|| {
        malformed(
            "$.brush.is".to_owned(),
            format!("{} is not one of {}", shown(named), STUFF_NAMED.join(", ")),
        )
    })?;
    let host = device(host)?;
    let width = host.width();

    let quads = grid.quads();
    let plan = Plan::over(width, quads.div_ceil(sweep::cells(width)))?;
    let (passes, stages) = crate::live::wired(width, grid, &plan, None)?;
    let groups: Vec<&[Groups]> = stages.iter().map(Vec::as_slice).collect();
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let sizes = plan.sizes();
    let mut resident = host.resident(&sizes, &modules)?;
    for (binding, words) in sizes.iter().enumerate() {
        resident.write(binding, &vec![0_u32; *words])?;
    }

    let mut cells = walled(grid, walls);
    for (at, stuff) in placed {
        if let Some(slot) = cells.get_mut(at) {
            *slot = stuff;
        }
    }

    resident.write(STATE as usize, &cells)?;
    resident.write(PARAMS as usize, &[u32::MAX, plan.dump()])?;
    crate::live::seed(&mut resident, grid, &plan)?;

    // A step never crosses the frame the spout shuts at, so the pour stops on
    // that frame and not at the end of the batch it falls in.
    let mut listed = 0_u64;
    let mut looked = 0_u64;
    let mut frame = 0_u32;
    while frame < frames {
        let pouring_now = frame < pouring;
        let until = if pouring_now { pouring } else { frames };
        let step = batch.min(until - frame);
        if let Some((x, y)) = spout {
            let material = if pouring_now { stuff } else { AIR };
            resident.write_at(PARAMS as usize, BRUSH, &[x, y, span, material])?;
        }
        resident.staged_frames(&groups, step)?;
        frame += step;
        listed += u64::from(
            resident
                .read(GRAND as usize, 1)?
                .first()
                .copied()
                .unwrap_or(0),
        );
        looked += 1;
    }

    let ended = resident.read(STATE as usize, grid.cells())?;
    let mut tally = [0_u64; 4];
    for one in &ended {
        if let Some(slot) = tally.get_mut(material_of(*one) as usize) {
            *slot += 1;
        }
    }
    // Where the settled stuff lies, a row at a time. A pile and a puddle hold
    // the same number of cells and do not look the same lying down.
    let rows: Vec<Json> = (0..grid.deep)
        .map(|row| {
            let held = (0..grid.wide)
                .filter(|column| {
                    ended
                        .get((row * grid.wide + column) as usize)
                        .is_some_and(|one| matches!(material_of(*one), SAND | WATER))
                })
                .count();
            counted(held as u32)
        })
        .collect();
    // Rows say how it lies down and cannot say how many heaps there are: two
    // piles and one wide one fill the same rows. Columns can.
    let columns: Vec<Json> = (0..grid.wide)
        .map(|column| {
            let held = (0..grid.deep)
                .filter(|row| {
                    ended
                        .get((row * grid.wide + column) as usize)
                        .is_some_and(|one| matches!(material_of(*one), SAND | WATER))
                })
                .count();
            counted(held as u32)
        })
        .collect();
    let standing: Vec<(usize, u32)> = ended
        .iter()
        .enumerate()
        .map(|(at, one)| (at, material_of(*one)))
        .filter(|(_, one)| *one != AIR && *one != WALL)
        .take(most)
        .collect();

    Ok(map(vec![
        (
            "grid",
            map(vec![
                ("wide", counted(grid.wide)),
                ("deep", counted(grid.deep)),
                ("cells", counted(grid.cells() as u32)),
            ]),
        ),
        ("frames", counted(frames)),
        (
            "tally",
            map(vec![
                ("air", Json::Int(tally.first().copied().unwrap_or(0) as i64)),
                (
                    "water",
                    Json::Int(tally.get(1).copied().unwrap_or(0) as i64),
                ),
                ("sand", Json::Int(tally.get(2).copied().unwrap_or(0) as i64)),
                ("wall", Json::Int(tally.get(3).copied().unwrap_or(0) as i64)),
            ]),
        ),
        ("rows", Json::List(rows)),
        ("columns", Json::List(columns)),
        (
            "standing",
            Json::List(
                standing
                    .iter()
                    .map(|(at, one)| {
                        map(vec![
                            ("at", counted(*at as u32)),
                            (
                                "is",
                                text(STUFF_NAMED.get(*one as usize).copied().unwrap_or("air")),
                            ),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("listed", Json::Int((listed / looked.max(1)) as i64)),
        ("of_quads", counted(quads)),
        ("checked_on_the_host", Json::Bool(true)),
        (
            "listed_means",
            text(
                "how many quads the work list named on an average look, out of of_quads in the whole grid; the engine steps what the list names, so this is what the frame actually cost rather than how big the grid is",
            ),
        ),
        (
            "rows_means",
            text(
                "how many cells of sand or water lie in each row, top row first, and in each column left to right; a pile and a puddle hold the same count and do not lie down the same way, and two heaps and one wide one fill the same rows while the columns between them are empty",
            ),
        ),
    ]))
}

pub const SHAKES: u32 = 64;

fn effort(held: &Json) -> Result<Effort, Box<dyn std::error::Error>> {
    Ok(Effort {
        crowd: CROWDED.count(held, "$")?,
        rounds: GENERATIONS.count(held, "$")?,
        seed: SEEDED.count(held, "$")?,
        stall: STALLED.count(held, "$")?,
        shake: SHAKEN.count(held, "$")?,
        submitted: u32::MAX,
    })
}

pub(crate) fn ran(
    host: Option<&Host>,
    name: &str,
    held: &Json,
) -> Result<Json, Box<dyn std::error::Error>> {
    if name == "grit_slope" {
        return slid(host, held);
    }
    if name == "grit_pour" {
        return poured(host, held);
    }
    if name == "grit_settle" {
        return settled(host, held);
    }
    if name == "grit_machine" {
        let (machine, traces) = asked_machine(held)?;
        let steps = machine.steps;
        let effort = effort(held)?;
        let told = if held.get("chip").is_some() {
            let shape = machine.shape()?;
            marked_machine(machine, &traces, &given(held, shape)?)?
        } else {
            fits_unrolled(machine, u32::try_from(traces.len())?)?;
            pressed_machine(device(host)?, machine, &traces, effort)?
        };
        return Ok(map(vec![
            ("solved", Json::Bool(told.marks >= told.full)),
            ("marks", counted(told.marks)),
            ("of", counted(told.full)),
            ("traces", counted(told.traces)),
            ("steps", counted(steps)),
            (
                "solved_means",
                text(
                    "every run in this request is answered. A machine can answer every run it was handed and still miss runs it has never seen; pass it back as chip with fresh runs to find out. Longer runs pin a machine down on far fewer marks than short ones do.",
                ),
            ),
            ("rounds", counted(told.rounds)),
            ("crowd", counted(told.crowd)),
            (
                "gates",
                Json::List(
                    told.gates
                        .iter()
                        .map(|one| {
                            map(vec![
                                ("kind", counted(one.kind)),
                                ("left", counted(one.left)),
                                ("right", counted(one.right)),
                            ])
                        })
                        .collect(),
                ),
            ),
            (
                "said",
                Json::List(
                    told.said
                        .iter()
                        .map(|one| Json::List(one.iter().map(|one| counted(*one)).collect()))
                        .collect(),
                ),
            ),
            (
                "wrong",
                Json::List(
                    told.wrong
                        .iter()
                        .map(|(trace, step)| {
                            map(vec![("trace", counted(*trace)), ("step", counted(*step))])
                        })
                        .collect(),
                ),
            ),
            ("checked_on_the_host", Json::Bool(true)),
        ]));
    }
    let inputs = if name == "grit_check" {
        &CHECK_INPUTS
    } else {
        &SYNTH_INPUTS
    };
    let (shape, exam) = asked(held, inputs)?;
    match name {
        "grit_check" => {
            present(held, "chip")?;
            let gates = given(held, shape)?;
            Ok(told(&checked(shape, &exam, &gates)?, false))
        }
        "grit_synthesise" => {
            let effort = effort(held)?;
            Ok(told(&pressed(device(host)?, shape, &exam, effort)?, true))
        }
        "grit_least" => {
            let effort = effort(held)?;
            let host = device(host)?;
            let many = effort.crowd;
            let rounds = effort.rounds;
            let table: Vec<u32> = (0..shape.words()).map(|row| exam.asked(row)).collect();
            let rungs = least(host, shape, &table, effort)?;
            let smallest = rungs
                .iter()
                .filter(|rung| rung.found.marks >= rung.found.full)
                .map(|rung| rung.gates)
                .min();
            let ladder = Json::List(
                rungs
                    .iter()
                    .map(|rung| {
                        map(vec![
                            ("gates", counted(rung.gates)),
                            ("solved", Json::Bool(rung.found.marks >= rung.found.full)),
                            ("marks", counted(rung.found.marks)),
                            ("of", counted(rung.found.full)),
                            ("rounds", counted(rung.found.rounds)),
                        ])
                    })
                    .collect(),
            );
            let best = rungs
                .iter()
                .filter(|rung| rung.found.marks >= rung.found.full)
                .min_by_key(|rung| rung.gates)
                .map_or(Json::Null, |rung| told(&rung.found, true));
            let mut held = vec![
                ("smallest_that_solved", smallest.map_or(Json::Null, counted)),
                ("ladder", ladder),
                ("circuit", best),
                ("crowd", counted(many)),
                ("rounds_each", counted(rounds)),
                (
                    "a_rung_that_did_not_solve",
                    text(
                        "means not found within these rounds and the crowd the rung reached after doubling, not that none exists",
                    ),
                ),
            ];
            let failed_twice = rungs
                .iter()
                .rev()
                .take(PATIENCE as usize)
                .all(|rung| rung.found.marks < rung.found.full);
            let at_bottom = rungs
                .last()
                .is_none_or(|rung| rung.gates <= shape.outputs.max(1));
            if !failed_twice && !at_bottom {
                held.push((
                    "cut_short",
                    text(&format!(
                        "the ladder stopped at its limit of {RUNGS} rungs while rungs were still solving, so a smaller budget may still hold it; ask again starting from smallest_that_solved"
                    )),
                ));
            }
            if smallest.is_none() {
                if let Some(rung) = rungs.iter().max_by_key(|rung| rung.found.marks) {
                    held.push(("best_attempt", told(&rung.found, true)));
                }
                held.push((
                    "nothing_solved",
                    text(
                        "the ladder ends after two rungs in a row fail, so starting on a budget that cannot be met leaves no circuit; start the ladder from more gates, or raise crowd, and ask again",
                    ),
                ));
            }
            Ok(map(held))
        }
        other => Err(Box::new(crate::netlist::TooWide::Kinds {
            kinds: other.len() as u32,
            limit: 0,
        })),
    }
}

#[must_use]
pub fn handle(host: Option<&Host>, request: &Json) -> Option<Json> {
    crate::serve3::transport::handle(&mut crate::serve3::Flat { host }, request)
}

#[must_use]
pub fn heard(host: Option<&Host>, line: &str) -> Option<Json> {
    crate::serve3::transport::heard(&mut crate::serve3::Flat { host }, line)
}
