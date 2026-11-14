use std::collections::HashSet;
use std::time::{Duration, Instant};

use crate::host::Host;
use crate::marking::{SCORE, TRIAL, VALUES, WANTED, column, marked_by, rows_of, words_of};
use crate::netlist::{Exam, Gate, Netlist};
use crate::resident::{Groups, Resident};
use crate::serve::Pace;

pub const COMPLEMENT: u32 = 15;

pub const FIELDS: [&str; 3] = ["kind", "left", "right"];

pub const CREDIT_WORDS: u64 = 1 << 24;

pub const PAIRS: u64 = 1 << 22;

pub const PAIRS_MOST: u64 = 1 << 25;

pub const MOVES: u64 = 1 << 24;

pub const MOVES_MOST: u64 = 1 << 26;

pub const LISTED: u32 = 4096;

pub const PAIRS_LISTED: usize = 16;

pub const ESCAPE_ROUNDS: u64 = 128;

pub const DEPTH_MOST: u32 = 2;

pub const WORK: u64 = 1 << 31;

pub const WORK_MOST: u64 = 1 << 33;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Costs {
    pub tangent: u64,
    pub unit: u64,
    pub singles: u64,
}

impl Costs {
    #[must_use]
    pub fn of(shape: Netlist) -> Self {
        let wide = u64::from(words_of(shape));
        let gates = u64::from(shape.gates);
        Self {
            tangent: (gates + 1) * gates * wide,
            unit: (u64::from(shape.outputs) + 1) * wide,
            singles: counted_singles(shape),
        }
    }

    #[must_use]
    pub const fn least(&self) -> u64 {
        self.tangent
            .saturating_add(self.singles.saturating_mul(self.unit))
    }

    #[must_use]
    pub const fn first(&self) -> u64 {
        self.least()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Change {
    pub at: u32,
    pub field: u32,
    pub value: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Board {
    shape: Netlist,
    wide: usize,
    rows: Vec<u32>,
    want: Vec<u32>,
    partial: bool,
    cared: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tangent {
    pub base: Vec<u32>,
    pub right: Vec<u32>,
    pub flipped: Vec<u32>,
    pub marks: u32,
}

#[must_use]
pub const fn kinded(kind: u32, left: u32, right: u32) -> u32 {
    (on(kind, 0) & !left & !right)
        | (on(kind, 1) & left & !right)
        | (on(kind, 2) & !left & right)
        | (on(kind, 3) & left & right)
}

const fn on(kind: u32, bit: u32) -> u32 {
    if (kind >> bit) & 1 == 1 { u32::MAX } else { 0 }
}

const fn bit(kind: u32, at: u32) -> u32 {
    (kind >> at) & 1
}

#[must_use]
pub const fn reads(kind: u32) -> (bool, bool) {
    let left = bit(kind, 0) != bit(kind, 1) || bit(kind, 2) != bit(kind, 3);
    let right = bit(kind, 0) != bit(kind, 2) || bit(kind, 1) != bit(kind, 3);
    (left, right)
}

#[must_use]
pub fn changed(gate: Gate, change: Change) -> Gate {
    match change.field {
        0 => Gate {
            kind: change.value,
            ..gate
        },
        1 => Gate {
            left: change.value,
            ..gate
        },
        _ => Gate {
            right: change.value,
            ..gate
        },
    }
}

#[must_use]
pub fn applied(circuit: &[u32], change: Change) -> Vec<u32> {
    let mut held = circuit.to_vec();
    if let Some(word) = held.get_mut(change.at as usize) {
        *word = changed(Gate::unpack(*word), change).pack();
    }
    held
}

#[must_use]
pub fn singles(shape: Netlist, circuit: &[u32]) -> Vec<Change> {
    let mut out = Vec::new();
    for (at, packed) in (0..shape.gates).zip(circuit) {
        let gate = Gate::unpack(*packed);
        let below = shape.below(at);
        for (field, among, now) in [
            (0, shape.kinds, gate.kind),
            (1, below, gate.left),
            (2, below, gate.right),
        ] {
            out.extend(
                (0..among)
                    .filter(|value| *value != now)
                    .map(|value| Change { at, field, value }),
            );
        }
    }
    out
}

#[must_use]
pub fn counted_singles(shape: Netlist) -> u64 {
    (0..shape.gates)
        .map(|at| {
            u64::from(shape.kinds.saturating_sub(1))
                + 2 * u64::from(shape.below(at).saturating_sub(1))
        })
        .sum()
}

fn words(held: &[u32], at: usize, wide: usize) -> &[u32] {
    held.get(at * wide..(at + 1) * wide).unwrap_or(&[])
}

impl Board {
    #[must_use]
    pub fn of(exam: &Exam) -> Self {
        let shape = exam.shape();
        let wide = words_of(shape) as usize;
        let mut rows: Vec<u32> = (0..wide as u32).map(|word| rows_of(shape, word)).collect();
        let mut want = vec![0_u32; shape.outputs as usize * wide];
        if exam.is_partial() {
            for (word, mask) in rows.iter_mut().enumerate() {
                *mask = (0..32_u32)
                    .filter(|bit| exam.cares(word as u32 * 32 + bit))
                    .fold(0, |held, bit| held | (1 << bit));
            }
        }
        for row in (0..shape.words()).filter(|row| exam.cares(*row)) {
            let asked = exam.asked(row);
            for output in 0..shape.outputs {
                if (asked >> output) & 1 == 1
                    && let Some(word) = want.get_mut(output as usize * wide + (row / 32) as usize)
                {
                    *word |= 1 << (row % 32);
                }
            }
        }
        Self {
            shape,
            wide,
            rows,
            want,
            partial: exam.is_partial(),
            cared: exam.cared(),
        }
    }

    #[must_use]
    pub const fn partial(&self) -> bool {
        self.partial
    }

    #[must_use]
    pub const fn cared(&self) -> u32 {
        self.cared
    }

    #[must_use]
    pub fn wanted(&self) -> Vec<u32> {
        let mut held = self.want.clone();
        if self.partial {
            held.extend_from_slice(&self.rows);
        }
        held
    }

    #[must_use]
    pub const fn shape(&self) -> Netlist {
        self.shape
    }

    #[must_use]
    pub const fn wide(&self) -> usize {
        self.wide
    }

    #[must_use]
    pub fn full(&self) -> u32 {
        self.cared * self.shape.outputs
    }

    fn first(&self) -> usize {
        (self.shape.inputs + self.shape.gates - self.shape.outputs) as usize
    }

    #[must_use]
    pub fn values(&self, circuit: &[u32]) -> Vec<u32> {
        let wide = self.wide;
        let mut held = Vec::with_capacity(self.shape.sources() as usize * wide);
        for input in 0..self.shape.inputs {
            for word in 0..wide as u32 {
                held.push(column(input, word));
            }
        }
        for packed in circuit.iter().take(self.shape.gates as usize) {
            let gate = Gate::unpack(*packed);
            for word in 0..wide {
                let left = held
                    .get(gate.left as usize * wide + word)
                    .copied()
                    .unwrap_or(0);
                let right = held
                    .get(gate.right as usize * wide + word)
                    .copied()
                    .unwrap_or(0);
                held.push(kinded(gate.kind, left, right));
            }
        }
        held
    }

    #[must_use]
    pub fn rights_of(&self, outputs: &[u32]) -> Vec<u32> {
        outputs
            .iter()
            .zip(&self.want)
            .zip(self.rows.iter().cycle())
            .map(|((got, want), rows)| !(got ^ want) & rows)
            .collect()
    }

    #[must_use]
    pub fn rights(&self, values: &[u32]) -> Vec<u32> {
        let from = self.first() * self.wide;
        let span = self.shape.outputs as usize * self.wide;
        self.rights_of(values.get(from..from + span).unwrap_or(&[]))
    }

    #[must_use]
    pub fn marks(&self, circuit: &[u32]) -> u32 {
        self.rights(&self.values(circuit))
            .iter()
            .map(|one| one.count_ones())
            .sum()
    }

    #[must_use]
    pub fn tangent(&self, circuit: &[u32]) -> Tangent {
        let base = self.values(circuit);
        let right = self.rights(&base);
        let mut flipped = Vec::with_capacity(self.shape.gates as usize * right.len());
        for at in 0..self.shape.gates as usize {
            let mut held = circuit.to_vec();
            if let Some(word) = held.get_mut(at) {
                *word ^= COMPLEMENT;
            }
            flipped.extend(self.rights(&self.values(&held)));
        }
        let marks = right.iter().map(|one| one.count_ones()).sum();
        Tangent {
            base,
            right,
            flipped,
            marks,
        }
    }

    fn effect(&self, tangent: &Tangent, at: u32, kind: u32, left: &[u32], right: &[u32]) -> i64 {
        let wide = self.wide;
        let outputs = self.shape.outputs as usize;
        let was = words(&tangent.base, (self.shape.inputs + at) as usize, wide);
        let flipped = tangent
            .flipped
            .get(at as usize * outputs * wide..(at as usize + 1) * outputs * wide)
            .unwrap_or(&[]);
        let mut total = 0_i64;
        let sides = left.iter().zip(right);
        for (word, (((left, right), was), rows)) in sides.zip(was).zip(&self.rows).enumerate() {
            let moved = (kinded(kind, *left, *right) ^ was) & rows;
            if moved == 0 {
                continue;
            }
            for output in 0..outputs {
                let at = output * wide + word;
                let fixed = flipped.get(at).copied().unwrap_or(0) & moved;
                let lost = tangent.right.get(at).copied().unwrap_or(0) & moved;
                total += i64::from(fixed.count_ones()) - i64::from(lost.count_ones());
            }
        }
        total
    }

    #[must_use]
    pub fn credit(&self, tangent: &Tangent, circuit: &[u32], change: Change) -> i64 {
        let Some(packed) = circuit.get(change.at as usize) else {
            return 0;
        };
        let gate = changed(Gate::unpack(*packed), change);
        let left = words(&tangent.base, gate.left as usize, self.wide);
        let right = words(&tangent.base, gate.right as usize, self.wide);
        self.effect(tangent, change.at, gate.kind, left, right)
    }

    #[must_use]
    pub fn dead(&self, circuit: &[u32]) -> Vec<bool> {
        let inputs = self.shape.inputs as usize;
        let gates = self.shape.gates as usize;
        let mut live = vec![false; inputs + gates];
        for one in live.iter_mut().skip(self.first()) {
            *one = true;
        }
        for at in (0..gates).rev() {
            if !live.get(inputs + at).copied().unwrap_or(false) {
                continue;
            }
            let gate = Gate::unpack(circuit.get(at).copied().unwrap_or(0));
            let (left, right) = reads(gate.kind);
            for (uses, source) in [(left, gate.left), (right, gate.right)] {
                if uses && let Some(one) = live.get_mut(source as usize) {
                    *one = true;
                }
            }
        }
        live.into_iter().skip(inputs).map(|one| !one).collect()
    }

    #[must_use]
    pub fn masks(&self, tangent: &Tangent, at: u32) -> (Vec<u32>, Vec<u32>) {
        let span = self.shape.outputs as usize * self.wide;
        let flipped = tangent
            .flipped
            .get(at as usize * span..(at as usize + 1) * span)
            .unwrap_or(&[]);
        flipped
            .iter()
            .zip(&tangent.right)
            .map(|(flipped, right)| (flipped & !right, right & !flipped))
            .unzip()
    }
}

pub trait Complements {
    fn tangents(
        &mut self,
        board: &Board,
        circuits: &[Vec<u32>],
    ) -> Result<Vec<Tangent>, Box<dyn std::error::Error>>;
}

pub struct Twin;

impl Complements for Twin {
    fn tangents(
        &mut self,
        board: &Board,
        circuits: &[Vec<u32>],
    ) -> Result<Vec<Tangent>, Box<dyn std::error::Error>> {
        Ok(circuits.iter().map(|one| board.tangent(one)).collect())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Spent {
    pub submissions: u64,
    pub circuits: u64,
    pub most: u32,
    pub longest: Duration,
    pub wall: Duration,
}

pub struct Marker<'held, 'host> {
    resident: &'held mut Resident<'host>,
    pipelines: usize,
    at: usize,
    width: u32,
    room: u32,
    pace: Pace,
    pub spent: Spent,
}

#[must_use]
pub fn room(shape: Netlist, most: u32) -> u32 {
    let each = u64::from(shape.sources()) * u64::from(words_of(shape));
    let fits = CREDIT_WORDS / each.max(1);
    u32::try_from(fits.min(u64::from(most))).unwrap_or(most)
}

#[must_use]
pub fn sizes(shape: Netlist, room: u32) -> [usize; 8] {
    sizes_by(shape, room, false)
}

#[must_use]
pub fn sizes_by(shape: Netlist, room: u32, partial: bool) -> [usize; 8] {
    let room = room as usize;
    let wide = words_of(shape) as usize;
    [
        1,
        room * shape.gates as usize,
        room * shape.sources() as usize * wide,
        (shape.outputs as usize + usize::from(partial)) * wide,
        room,
        1,
        1,
        1,
    ]
}

pub fn opened<'host>(
    host: &'host Host,
    board: &Board,
    room: u32,
) -> Result<Resident<'host>, Box<dyn std::error::Error>> {
    let mark = marked_by(host.width(), board.shape, board.partial)?;
    let modules: Vec<&[u32]> = vec![&mark];
    let mut resident = host.resident(&sizes_by(board.shape, room, board.partial), &modules)?;
    resident.write(WANTED as usize, &board.wanted())?;
    Ok(resident)
}

impl<'held, 'host> Marker<'held, 'host> {
    #[must_use]
    pub fn within(
        resident: &'held mut Resident<'host>,
        pipelines: usize,
        at: usize,
        width: u32,
        room: u32,
        shape: Netlist,
    ) -> Self {
        let width = width.max(1);
        let room = room / width * width;
        let work = u64::from(shape.gates) * u64::from(words_of(shape));
        Self {
            resident,
            pipelines,
            at,
            width,
            room,
            pace: Pace::over(work, room),
            spent: Spent::default(),
        }
    }

    fn submitted(
        &mut self,
        board: &Board,
        flat: &[u32],
        many: usize,
    ) -> Result<(Vec<u32>, Vec<u32>), Box<dyn std::error::Error>> {
        let span = board.shape.sources() as usize * board.wide;
        let gates = board.shape.gates as usize;
        let groups = u32::try_from(many)?.div_ceil(self.width);
        let lanes = groups as usize * self.width as usize;
        if lanes > self.room as usize || flat.len() != many * gates {
            return Err(format!(
                "{many} circuits in {groups} groups of {} lanes do not fit the {} circuits the device holds",
                self.width, self.room
            )
            .into());
        }
        let mut whole = flat.to_vec();
        whole.resize(lanes * gates, 0);
        let began = Instant::now();
        self.resident.write(TRIAL as usize, &whole)?;
        let held: Vec<[Groups; 1]> = (0..self.pipelines)
            .map(|one| [Groups::Fixed(if one == self.at { groups } else { 0 })])
            .collect();
        let stages: Vec<&[Groups]> = held.iter().map(|one| &one[..]).collect();
        let wall = self.resident.staged_frames(&stages, 1)?;
        let spent = self
            .resident
            .on_device()
            .map_or(wall, |device| device.max(wall));
        let scores = self.resident.read(SCORE as usize, many)?;
        let values = self.resident.read(VALUES as usize, many * span)?;
        let took = began.elapsed();
        self.spent.submissions += 1;
        self.spent.circuits += many as u64;
        self.spent.most = self.spent.most.max(u32::try_from(many)?);
        self.spent.longest = self.spent.longest.max(took);
        self.spent.wall += took;
        self.pace.learn(spent / u32::try_from(many)?.max(1));
        Ok((scores, values))
    }
}

impl Complements for Marker<'_, '_> {
    fn tangents(
        &mut self,
        board: &Board,
        circuits: &[Vec<u32>],
    ) -> Result<Vec<Tangent>, Box<dyn std::error::Error>> {
        if self.room == 0 {
            return Err("the device holds no circuit to mark".into());
        }
        let gates = board.shape.gates as usize;
        let group = gates + 1;
        let wide = board.wide;
        let span = board.shape.sources() as usize * wide;
        let outputs = board.shape.outputs as usize * wide;
        let first = board.first() * wide;
        let total = circuits.len() * group;
        let mut out: Vec<Tangent> = circuits
            .iter()
            .map(|_| Tangent {
                base: Vec::new(),
                right: Vec::new(),
                flipped: Vec::with_capacity(gates * outputs),
                marks: 0,
            })
            .collect();
        let mut done = 0_usize;
        while done < total {
            let left = u32::try_from(total - done).unwrap_or(u32::MAX);
            let step = (self.pace.next(left) as usize).clamp(1, total - done);
            let mut flat = Vec::with_capacity(step * gates);
            for one in done..done + step {
                let circuit = circuits.get(one / group).map_or(&[][..], Vec::as_slice);
                let from = flat.len();
                flat.extend_from_slice(circuit);
                if let Some(at) = (one % group).checked_sub(1)
                    && let Some(word) = flat.get_mut(from + at)
                {
                    *word ^= COMPLEMENT;
                }
            }
            let (scores, values) = self.submitted(board, &flat, step)?;
            for (here, one) in (done..done + step).enumerate() {
                let Some(tangent) = out.get_mut(one / group) else {
                    continue;
                };
                if one % group == 0 {
                    tangent.base = values
                        .get(here * span..(here + 1) * span)
                        .unwrap_or(&[])
                        .to_vec();
                    tangent.right = board.rights(&tangent.base);
                    tangent.marks = scores.get(here).copied().unwrap_or(0);
                } else {
                    let from = here * span + first;
                    tangent
                        .flipped
                        .extend(board.rights_of(values.get(from..from + outputs).unwrap_or(&[])));
                }
            }
            done += step;
        }
        Ok(out)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Singles {
    pub changes: Vec<(i64, Change)>,
    pub improving: u64,
    pub neutral: u64,
    pub best: i64,
}

#[must_use]
pub fn credited(board: &Board, tangent: &Tangent, circuit: &[u32]) -> Singles {
    let mut changes: Vec<(i64, Change)> = singles(board.shape, circuit)
        .into_iter()
        .map(|change| (board.credit(tangent, circuit, change), change))
        .collect();
    changes.sort_by(|one, other| other.0.cmp(&one.0).then(one.1.cmp(&other.1)));
    let improving = changes.iter().filter(|one| one.0 > 0).count() as u64;
    let neutral = changes.iter().filter(|one| one.0 == 0).count() as u64;
    let best = changes.first().map_or(i64::MIN, |one| one.0);
    Singles {
        changes,
        improving,
        neutral,
        best,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pairs {
    pub evaluated: u64,
    pub total: u64,
    pub firsts: u64,
    pub circuits: u64,
    pub improving: u64,
    pub best: i64,
    pub top: Vec<(i64, Change, Change)>,
}

impl Pairs {
    #[must_use]
    pub const fn complete(&self) -> bool {
        self.evaluated == self.total
    }
}

fn ranked(top: &mut Vec<(i64, Change, Change)>, one: (i64, Change, Change), most: usize) {
    if most == 0 {
        return;
    }
    let worse = |held: &(i64, Change, Change)| {
        one.0 > held.0 || (one.0 == held.0 && (one.1, one.2) < (held.1, held.2))
    };
    if top.len() >= most && top.last().is_none_or(|held| !worse(held)) {
        return;
    }
    let at = top.iter().position(worse).unwrap_or(top.len());
    top.insert(at, one);
    top.truncate(most);
}

pub fn paired(
    board: &Board,
    source: &mut impl Complements,
    circuit: &[u32],
    budget: u64,
    most: usize,
) -> Result<Pairs, Box<dyn std::error::Error>> {
    let firsts = singles(board.shape, circuit);
    let each = firsts.len() as u64;
    let total = each * each;
    let group = u64::from(board.shape.gates) + 1;
    let taken = budget.checked_div(each).unwrap_or(0).min(each);
    let base = board.marks(circuit);
    let mut found = Pairs {
        evaluated: 0,
        total,
        firsts: taken,
        circuits: taken * group,
        improving: 0,
        best: i64::MIN,
        top: Vec::new(),
    };
    let chunk = 4096_usize;
    for part in firsts.get(..taken as usize).unwrap_or(&[]).chunks(chunk) {
        let mids: Vec<Vec<u32>> = part.iter().map(|one| applied(circuit, *one)).collect();
        let tangents = source.tangents(board, &mids)?;
        for ((first, mid), tangent) in part.iter().zip(&mids).zip(&tangents) {
            let lift = i64::from(tangent.marks) - i64::from(base);
            for second in singles(board.shape, mid) {
                let gain = lift + board.credit(tangent, mid, second);
                found.evaluated += 1;
                found.best = found.best.max(gain);
                if gain > 0 {
                    found.improving += 1;
                }
                ranked(&mut found.top, (gain, *first, second), most);
            }
        }
    }
    Ok(found)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Insert {
    pub dead: u32,
    pub kind: u32,
    pub left: u32,
    pub right: u32,
    pub into: u32,
    pub side: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Repair {
    pub at: u32,
    pub kind: u32,
    pub other: u32,
    pub dead: u32,
    pub dead_kind: u32,
    pub left: u32,
    pub right: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Moves<T> {
    pub evaluated: u64,
    pub complete: bool,
    pub improving: u64,
    pub best: Option<(i64, T)>,
}

impl<T> Moves<T> {
    const fn over() -> Self {
        Self {
            evaluated: 0,
            complete: true,
            improving: 0,
            best: None,
        }
    }

    fn seen(&mut self, gain: i64, one: T) {
        self.evaluated += 1;
        if gain > 0 {
            self.improving += 1;
        }
        if self.best.as_ref().is_none_or(|held| gain > held.0) {
            self.best = Some((gain, one));
        }
    }
}

#[must_use]
pub fn inserted(shape: Netlist, circuit: &[u32], one: Insert) -> Vec<u32> {
    let mut held = circuit.to_vec();
    if let Some(word) = held.get_mut(one.dead as usize) {
        *word = Gate {
            kind: one.kind,
            left: one.left,
            right: one.right,
        }
        .pack();
    }
    if let Some(word) = held.get_mut(one.into as usize) {
        let gate = Gate::unpack(*word);
        let source = shape.inputs + one.dead;
        *word = if one.side == 0 {
            Gate {
                left: source,
                ..gate
            }
        } else {
            Gate {
                right: source,
                ..gate
            }
        }
        .pack();
    }
    held
}

#[must_use]
pub fn repaired(shape: Netlist, circuit: &[u32], one: Repair) -> Vec<u32> {
    let mut held = circuit.to_vec();
    if let Some(word) = held.get_mut(one.dead as usize) {
        *word = Gate {
            kind: one.dead_kind,
            left: one.left,
            right: one.right,
        }
        .pack();
    }
    if let Some(word) = held.get_mut(one.at as usize) {
        *word = Gate {
            kind: one.kind,
            left: shape.inputs + one.dead,
            right: one.other,
        }
        .pack();
    }
    held
}

type Recipe = (u32, u32, u32);

struct Made {
    at: u32,
    values: Vec<Vec<u32>>,
    recipes: Vec<Recipe>,
}

struct Pool {
    made: Vec<Made>,
    evaluated: u64,
    complete: bool,
}

fn pooled(board: &Board, tangent: &Tangent, dead: &[bool], budget: u64) -> Pool {
    let wide = board.wide;
    let mut pool = Pool {
        made: Vec::new(),
        evaluated: 0,
        complete: true,
    };
    for (at, is_dead) in (0..board.shape.gates).zip(dead) {
        if !*is_dead {
            continue;
        }
        let below = board.shape.below(at);
        let mut seen: HashSet<Vec<u32>> = HashSet::new();
        let mut values = Vec::new();
        let mut recipes = Vec::new();
        'all: for kind in 0..board.shape.kinds {
            for left in 0..below {
                for right in 0..below {
                    if pool.evaluated >= budget {
                        pool.complete = false;
                        break 'all;
                    }
                    pool.evaluated += 1;
                    let one: Vec<u32> = words(&tangent.base, left as usize, wide)
                        .iter()
                        .zip(words(&tangent.base, right as usize, wide))
                        .zip(&board.rows)
                        .map(|((left, right), rows)| kinded(kind, *left, *right) & rows)
                        .collect();
                    if seen.insert(one.clone()) {
                        values.push(one);
                        recipes.push((kind, left, right));
                    }
                }
            }
        }
        pool.made.push(Made {
            at,
            values,
            recipes,
        });
        if !pool.complete {
            break;
        }
    }
    pool
}

#[must_use]
pub fn inserts(board: &Board, tangent: &Tangent, circuit: &[u32], budget: u64) -> Moves<Insert> {
    let dead = board.dead(circuit);
    let pool = pooled(board, tangent, &dead, budget);
    let mut found = Moves::over();
    found.evaluated = pool.evaluated;
    found.complete = pool.complete;
    let wide = board.wide;
    'all: for made in &pool.made {
        let at = &made.at;
        for into in at + 1..board.shape.gates {
            let gate = Gate::unpack(circuit.get(into as usize).copied().unwrap_or(0));
            let left = words(&tangent.base, gate.left as usize, wide);
            let right = words(&tangent.base, gate.right as usize, wide);
            for side in 0..2_u32 {
                for (value, recipe) in made.values.iter().zip(&made.recipes) {
                    if found.evaluated >= budget {
                        found.complete = false;
                        break 'all;
                    }
                    let gain = if side == 0 {
                        board.effect(tangent, into, gate.kind, value, right)
                    } else {
                        board.effect(tangent, into, gate.kind, left, value)
                    };
                    found.seen(
                        gain,
                        Insert {
                            dead: *at,
                            kind: recipe.0,
                            left: recipe.1,
                            right: recipe.2,
                            into,
                            side,
                        },
                    );
                }
            }
        }
    }
    found
}

#[must_use]
pub fn repairs(board: &Board, tangent: &Tangent, circuit: &[u32], budget: u64) -> Moves<Repair> {
    let dead = board.dead(circuit);
    let pool = pooled(board, tangent, &dead, budget);
    let mut found = Moves::over();
    found.evaluated = pool.evaluated;
    found.complete = pool.complete;
    let wide = board.wide;
    let inputs = board.shape.inputs;
    'all: for at in 0..board.shape.gates {
        if dead.get(at as usize).copied().unwrap_or(true) {
            continue;
        }
        let mut seen: HashSet<&[u32]> = HashSet::new();
        let mut union: Vec<(&[u32], u32, Recipe)> = Vec::new();
        for made in &pool.made {
            if made.at >= at {
                break;
            }
            for (value, recipe) in made.values.iter().zip(&made.recipes) {
                if seen.insert(value) {
                    union.push((value, made.at, *recipe));
                }
            }
        }
        for (value, dead_at, recipe) in &union {
            for other in 0..inputs + at {
                if other >= inputs && dead.get((other - inputs) as usize).copied().unwrap_or(true) {
                    continue;
                }
                let theirs = words(&tangent.base, other as usize, wide);
                for kind in 0..board.shape.kinds {
                    if found.evaluated >= budget {
                        found.complete = false;
                        break 'all;
                    }
                    let gain = board.effect(tangent, at, kind, value, theirs);
                    found.seen(
                        gain,
                        Repair {
                            at,
                            kind,
                            other,
                            dead: *dead_at,
                            dead_kind: recipe.0,
                            left: recipe.1,
                            right: recipe.2,
                        },
                    );
                }
            }
        }
    }
    found
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Single,
    Insert,
    Repair,
    Pair,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Escaped {
    pub circuit: Vec<u32>,
    pub marks: u32,
    pub evaluated: u64,
    pub steps: Vec<(Step, u32)>,
}

pub fn escape(
    board: &Board,
    source: &mut impl Complements,
    circuit: &[u32],
    budget: u64,
) -> Result<Escaped, Box<dyn std::error::Error>> {
    let group = u64::from(board.shape.gates) + 1;
    let mut held = circuit.to_vec();
    let mut marks = board.marks(&held);
    let mut evaluated = 0_u64;
    let mut steps = Vec::new();
    let full = board.full();
    while marks < full && evaluated + group <= budget {
        let tangent = source
            .tangents(board, std::slice::from_ref(&held))?
            .pop()
            .ok_or("the device answered no complement batch")?;
        evaluated += group;
        let each = counted_singles(board.shape);
        if evaluated + each > budget {
            break;
        }
        evaluated += each;
        let found = credited(board, &tangent, &held);
        if let Some((gain, change)) = found.changes.first().copied()
            && gain > 0
        {
            held = applied(&held, change);
            marks = board.marks(&held);
            steps.push((Step::Single, marks));
            continue;
        }
        let found = inserts(board, &tangent, &held, budget - evaluated);
        evaluated += found.evaluated;
        if let Some((gain, one)) = found.best
            && gain > 0
        {
            held = inserted(board.shape, &held, one);
            marks = board.marks(&held);
            steps.push((Step::Insert, marks));
            continue;
        }
        let found = repairs(board, &tangent, &held, budget - evaluated);
        evaluated += found.evaluated;
        if let Some((gain, one)) = found.best
            && gain > 0
        {
            held = repaired(board.shape, &held, one);
            marks = board.marks(&held);
            steps.push((Step::Repair, marks));
            continue;
        }
        let left = budget - evaluated;
        let afford = left / (each + group);
        if afford == 0 {
            break;
        }
        let found = paired(board, source, &held, afford * each, 1)?;
        evaluated += found.evaluated + found.circuits;
        if let Some((gain, first, second)) = found.top.first().copied()
            && gain > 0
        {
            held = applied(&applied(&held, first), second);
            marks = board.marks(&held);
            steps.push((Step::Pair, marks));
            continue;
        }
        break;
    }
    Ok(Escaped {
        circuit: held,
        marks,
        evaluated,
        steps,
    })
}
