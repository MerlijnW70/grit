use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::fmt;

use super::dag::{AT, Dag, FIRST_PARAM, FRAME, Node};
use super::{Code, Law, Places, Refusal};
use crate::ops::{Cpu, Flag, Ops};
use crate::question::cpu::{self, World};
use crate::question::{Answer, Map, Measure, Reduce, Region, Seen};
use crate::rule3::{CORNERS, Declared, Oct};
use crate::sand3::{Grid3, lattice, reference};

pub const FRACTION: u32 = 64;

const WORDS: u32 = 8;

const EXPECTED: &str =
    "a sequence of lifting steps over the 8 words: b += f, b -= f or b ^= f, with f free of b";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lifting {
    Add,
    Sub,
    Xor,
}

impl Lifting {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Add => "+=",
            Self::Sub => "-=",
            Self::Xor => "^=",
        }
    }

    #[must_use]
    pub const fn undone(self) -> Self {
        match self {
            Self::Add => Self::Sub,
            Self::Sub => Self::Add,
            Self::Xor => Self::Xor,
        }
    }

    const fn node(self, word: u32, by: u32) -> Node {
        match self {
            Self::Add => Node::Add(word, by),
            Self::Sub => Node::Sub(word, by),
            Self::Xor => Node::Xor(word, by),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lift {
    pub word: usize,
    pub how: Lifting,
    pub node: u32,
    pub before: u32,
    pub by: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Broken {
    Written {
        word: usize,
        node: u32,
    },
    Dropped {
        word: usize,
        node: u32,
    },
    ReadsItself {
        word: usize,
        node: u32,
    },
    Shared {
        word: usize,
        other: usize,
        node: u32,
    },
    Tangled {
        word: usize,
        node: u32,
    },
}

impl Broken {
    #[must_use]
    pub const fn node(self) -> u32 {
        match self {
            Self::Written { node, .. }
            | Self::Dropped { node, .. }
            | Self::ReadsItself { node, .. }
            | Self::Shared { node, .. }
            | Self::Tangled { node, .. } => node,
        }
    }

    #[must_use]
    pub fn found(self, dag: &Dag) -> String {
        let made = named(dag, self.node());
        match self {
            Self::Written { word, .. } => {
                format!("word {word} is made by {made}, which is not add, sub or xor of the word")
            }
            Self::Dropped { word, .. } => {
                format!("word {word} is made by {made}, and neither side carries the word")
            }
            Self::ReadsItself { word, .. } => {
                format!("the step {made} on word {word} reads word {word} in what it adds")
            }
            Self::Shared { word, other, .. } => {
                format!("words {other} and {word} are both made by the one step {made}")
            }
            Self::Tangled { word, .. } => format!(
                "the step {made} on word {word} reads a word that is changed both before and after it"
            ),
        }
    }

    #[must_use]
    pub fn refusal(self, dag: &Dag, places: &Places) -> Refusal {
        Refusal {
            code: Code::Lifting,
            path: places
                .path(self.node())
                .unwrap_or_else(|| String::from("$.step")),
            expected: String::from(EXPECTED),
            found: self.found(dag),
            limit: None,
        }
    }
}

fn named(dag: &Dag, node: u32) -> String {
    let found = usize::try_from(node)
        .ok()
        .and_then(|index| dag.nodes().get(index))
        .copied();
    let op = match found {
        Some(Node::Input(which)) if which < WORDS => return format!("cell {which} unchanged"),
        Some(Node::Input(which)) => return format!("input {which}"),
        Some(Node::Const(value)) => return format!("the constant {value}"),
        Some(Node::And(..)) => "and",
        Some(Node::Or(..)) => "or",
        Some(Node::Xor(..)) => "xor",
        Some(Node::Not(..)) => "not",
        Some(Node::Shl(..)) => "shl",
        Some(Node::Shr(..)) => "shr",
        Some(Node::Add(..)) => "add",
        Some(Node::Sub(..)) => "sub",
        Some(Node::Mul(..)) => "mul",
        Some(Node::Div(..)) => "div",
        Some(Node::DotS(..)) => "dot_s",
        Some(Node::DotU(..)) => "dot_u",
        Some(Node::Eq(..)) => "eq",
        Some(Node::Ne(..)) => "ne",
        Some(Node::Lt(..)) => "lt",
        Some(Node::Le(..)) => "le",
        Some(Node::Gt(..)) => "gt",
        Some(Node::Ge(..)) => "ge",
        Some(Node::Select(..)) => "sel",
        None => return format!("node {node}"),
    };
    String::from(op)
}

const fn lifting(node: Node) -> Option<(Lifting, u32, u32)> {
    match node {
        Node::Add(left, right) => Some((Lifting::Add, left, right)),
        Node::Sub(left, right) => Some((Lifting::Sub, left, right)),
        Node::Xor(left, right) => Some((Lifting::Xor, left, right)),
        _ => None,
    }
}

const fn operands(node: Node) -> [Option<u32>; 3] {
    match node {
        Node::Input(_) | Node::Const(_) => [None, None, None],
        Node::Not(one) | Node::Shl(one, _) | Node::Shr(one, _) => [Some(one), None, None],
        Node::Select(flag, then, otherwise) => [Some(flag), Some(then), Some(otherwise)],
        Node::And(left, right)
        | Node::Or(left, right)
        | Node::Xor(left, right)
        | Node::Add(left, right)
        | Node::Sub(left, right)
        | Node::Mul(left, right)
        | Node::Div(left, right)
        | Node::DotS(left, right)
        | Node::DotU(left, right)
        | Node::Eq(left, right)
        | Node::Ne(left, right)
        | Node::Lt(left, right)
        | Node::Le(left, right)
        | Node::Gt(left, right)
        | Node::Ge(left, right) => [Some(left), Some(right), None],
    }
}

fn at<T: Copy + Default>(held: &[T], id: u32) -> T {
    usize::try_from(id)
        .ok()
        .and_then(|index| held.get(index))
        .copied()
        .unwrap_or_default()
}

fn node_at(dag: &Dag, id: u32) -> Node {
    usize::try_from(id)
        .ok()
        .and_then(|index| dag.nodes().get(index))
        .copied()
        .unwrap_or(Node::Const(0))
}

const fn bit(word: usize) -> u8 {
    1_u8 << (word & 7)
}

fn word_of(word: usize) -> u32 {
    u32::try_from(word).unwrap_or(u32::MAX)
}

struct Masks {
    carries: Vec<u8>,
    reaches: Vec<u8>,
}

fn masks(dag: &Dag) -> Masks {
    let nodes = dag.nodes();
    let mut carries = vec![0_u8; nodes.len()];
    let mut reaches = vec![0_u8; nodes.len()];
    for (index, &node) in nodes.iter().enumerate() {
        let (carried, reached) = match node {
            Node::Input(which) if which < WORDS => {
                let one = bit(usize::try_from(which).unwrap_or(0));
                (one, one)
            }
            other => {
                let reached = operands(other)
                    .iter()
                    .flatten()
                    .fold(0, |acc, &id| acc | at(&reaches, id));
                let carried = match lifting(other) {
                    Some((Lifting::Sub, left, _)) => at(&carries, left),
                    Some((_, left, right)) => at(&carries, left) | at(&carries, right),
                    None => 0,
                };
                (carried, reached)
            }
        };
        if let Some(slot) = carries.get_mut(index) {
            *slot = carried;
        }
        if let Some(slot) = reaches.get_mut(index) {
            *slot = reached;
        }
    }
    Masks { carries, reaches }
}

fn offender(dag: &Dag, masks: &Masks, word: usize, start: u32) -> Broken {
    let mut here = start;
    loop {
        match lifting(node_at(dag, here)) {
            Some((_, left, right)) => {
                let next = [left, right]
                    .into_iter()
                    .find(|&id| at(&masks.reaches, id) & bit(word) != 0);
                match next {
                    Some(id) => here = id,
                    None => return Broken::Dropped { word, node: here },
                }
            }
            None => return Broken::Written { word, node: here },
        }
    }
}

fn chain(dag: &Dag, masks: &Masks, word: usize) -> Result<Vec<Lift>, Broken> {
    let mut here = dag.out().get(word).copied().unwrap_or(u32::MAX);
    let mut lifts = Vec::new();
    loop {
        let node = node_at(dag, here);
        if node == Node::Input(word_of(word)) {
            lifts.reverse();
            return Ok(lifts);
        }
        let Some((how, left, right)) = lifting(node) else {
            return Err(offender(dag, masks, word, here));
        };
        let on_left = at(&masks.carries, left) & bit(word) != 0;
        let on_right = how != Lifting::Sub && at(&masks.carries, right) & bit(word) != 0;
        let (before, by) = match (on_left, on_right) {
            (true, false) => (left, right),
            (false, true) => (right, left),
            (true, true) => return Err(Broken::ReadsItself { word, node: here }),
            (false, false) => return Err(offender(dag, masks, word, here)),
        };
        lifts.push(Lift {
            word,
            how,
            node: here,
            before,
            by,
        });
        here = before;
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Version {
    word: usize,
    index: usize,
}

fn versions(dag: &Dag, chains: &[Vec<Lift>]) -> Result<Vec<Option<Version>>, Broken> {
    let mut held: Vec<Option<Version>> = vec![None; dag.len()];
    for (index, &node) in dag.nodes().iter().enumerate() {
        if let Node::Input(which) = node
            && which < WORDS
            && let Some(slot) = held.get_mut(index)
        {
            *slot = Some(Version {
                word: usize::try_from(which).unwrap_or(0),
                index: 0,
            });
        }
    }
    let mut conflicts = Vec::new();
    for (word, lifts) in chains.iter().enumerate() {
        for (place, lift) in lifts.iter().enumerate() {
            let slot = usize::try_from(lift.node)
                .ok()
                .and_then(|index| held.get_mut(index));
            match slot {
                Some(Some(other)) => conflicts.push(Broken::Shared {
                    word,
                    other: other.word,
                    node: lift.node,
                }),
                Some(slot) => {
                    *slot = Some(Version {
                        word,
                        index: place + 1,
                    });
                }
                None => {}
            }
        }
    }
    match conflicts.into_iter().min_by_key(|broken| broken.node()) {
        Some(broken) => Err(broken),
        None => Ok(held),
    }
}

fn reads(
    dag: &Dag,
    held: &[Option<Version>],
    lift: &Lift,
    stamp: &mut [u32],
    mark: u32,
) -> Result<Vec<Version>, Broken> {
    let mut found = Vec::new();
    let mut stack = vec![lift.by];
    while let Some(id) = stack.pop() {
        let Some(slot) = usize::try_from(id)
            .ok()
            .and_then(|index| stamp.get_mut(index))
        else {
            continue;
        };
        if *slot == mark {
            continue;
        }
        *slot = mark;
        if let Some(version) = at(held, id) {
            if version.word == lift.word {
                return Err(Broken::ReadsItself {
                    word: lift.word,
                    node: lift.node,
                });
            }
            found.push(version);
            continue;
        }
        stack.extend(operands(node_at(dag, id)).iter().flatten());
    }
    Ok(found)
}

struct Order {
    after: Vec<Vec<usize>>,
    waiting: Vec<usize>,
}

impl Order {
    fn edge(&mut self, from: Option<usize>, to: Option<usize>) {
        if let (Some(from), Some(to)) = (from, to)
            && let Some(list) = self.after.get_mut(from)
            && let Some(count) = self.waiting.get_mut(to)
        {
            list.push(to);
            *count += 1;
        }
    }
}

fn ordered(dag: &Dag, chains: &[Vec<Lift>]) -> Result<Vec<Lift>, Broken> {
    let held = versions(dag, chains)?;
    let all: Vec<Lift> = chains.iter().flatten().copied().collect();
    let mut place: HashMap<u32, usize> = HashMap::with_capacity(all.len());
    for (index, lift) in all.iter().enumerate() {
        place.insert(lift.node, index);
    }
    let lift_at = |word: usize, version: usize| -> Option<usize> {
        let lift = chains.get(word)?.get(version.checked_sub(1)?)?;
        place.get(&lift.node).copied()
    };
    let mut order = Order {
        after: vec![Vec::new(); all.len()],
        waiting: vec![0; all.len()],
    };
    let mut stamp = vec![u32::MAX; dag.len()];
    let mut broken = Vec::new();
    for (index, lift) in all.iter().enumerate() {
        let version = at(&held, lift.node).map_or(0, |one| one.index);
        order.edge(lift_at(lift.word, version.saturating_sub(1)), Some(index));
        let mark = u32::try_from(index).unwrap_or(u32::MAX - 1);
        match reads(dag, &held, lift, &mut stamp, mark) {
            Ok(found) => {
                for read in found {
                    order.edge(lift_at(read.word, read.index), Some(index));
                    order.edge(Some(index), lift_at(read.word, read.index + 1));
                }
            }
            Err(why) => broken.push(why),
        }
    }
    if let Some(why) = broken.into_iter().min_by_key(|why| why.node()) {
        return Err(why);
    }
    let mut ready: BinaryHeap<Reverse<(u32, usize)>> = all
        .iter()
        .enumerate()
        .filter(|(index, _)| order.waiting.get(*index) == Some(&0))
        .map(|(index, lift)| Reverse((lift.node, index)))
        .collect();
    let mut sorted = Vec::with_capacity(all.len());
    let mut done = vec![false; all.len()];
    while let Some(Reverse((_, index))) = ready.pop() {
        if let Some(lift) = all.get(index) {
            sorted.push(*lift);
        }
        if let Some(slot) = done.get_mut(index) {
            *slot = true;
        }
        let next: Vec<usize> = order.after.get(index).cloned().unwrap_or_default();
        for one in next {
            if let Some(count) = order.waiting.get_mut(one) {
                *count = count.saturating_sub(1);
                if *count == 0
                    && let Some(lift) = all.get(one)
                {
                    ready.push(Reverse((lift.node, one)));
                }
            }
        }
    }
    let stuck = all
        .iter()
        .zip(&done)
        .filter(|(_, done)| !**done)
        .map(|(lift, _)| *lift)
        .min_by_key(|lift| lift.node);
    match stuck {
        Some(lift) => Err(Broken::Tangled {
            word: lift.word,
            node: lift.node,
        }),
        None => Ok(sorted),
    }
}

pub fn lifted(dag: &Dag) -> Result<Vec<Lift>, Broken> {
    let masks = masks(dag);
    let mut chains = Vec::with_capacity(CORNERS);
    let mut broken = Vec::new();
    for word in 0..CORNERS {
        match chain(dag, &masks, word) {
            Ok(lifts) => chains.push(lifts),
            Err(why) => {
                broken.push(why);
                chains.push(Vec::new());
            }
        }
    }
    if let Some(why) = broken.into_iter().min_by_key(|why| why.node()) {
        return Err(why);
    }
    ordered(dag, &chains)
}

struct Copier<'d> {
    dag: &'d Dag,
    nodes: Vec<Node>,
    reaches: Vec<u8>,
    lasting: HashMap<u32, u32>,
}

impl Copier<'_> {
    fn push(&mut self, node: Node) -> u32 {
        let id = u32::try_from(self.nodes.len()).unwrap_or(u32::MAX);
        self.nodes.push(node);
        id
    }

    fn copied(&mut self, root: u32, now: &HashMap<u32, u32>) -> u32 {
        let mut made: HashMap<u32, u32> = HashMap::new();
        let mut stack = vec![(root, false)];
        while let Some((id, expanded)) = stack.pop() {
            if made.contains_key(&id) {
                continue;
            }
            if let Some(&held) = now.get(&id).or_else(|| self.lasting.get(&id)) {
                made.insert(id, held);
                continue;
            }
            let node = node_at(self.dag, id);
            if let Node::Input(_) = node {
                made.insert(id, id);
                continue;
            }
            if !expanded {
                stack.push((id, true));
                for one in operands(node).iter().flatten() {
                    if !made.contains_key(one) {
                        stack.push((*one, false));
                    }
                }
                continue;
            }
            let moved = |one: u32| made.get(&one).copied().unwrap_or(one);
            let rebuilt = match node {
                Node::Not(one) => Node::Not(moved(one)),
                Node::Shl(one, by) => Node::Shl(moved(one), by),
                Node::Shr(one, by) => Node::Shr(moved(one), by),
                Node::Select(flag, then, otherwise) => {
                    Node::Select(moved(flag), moved(then), moved(otherwise))
                }
                Node::And(left, right) => Node::And(moved(left), moved(right)),
                Node::Or(left, right) => Node::Or(moved(left), moved(right)),
                Node::Xor(left, right) => Node::Xor(moved(left), moved(right)),
                Node::Add(left, right) => Node::Add(moved(left), moved(right)),
                Node::Sub(left, right) => Node::Sub(moved(left), moved(right)),
                Node::Mul(left, right) => Node::Mul(moved(left), moved(right)),
                Node::Div(left, right) => Node::Div(moved(left), moved(right)),
                Node::DotS(left, right) => Node::DotS(moved(left), moved(right)),
                Node::DotU(left, right) => Node::DotU(moved(left), moved(right)),
                Node::Eq(left, right) => Node::Eq(moved(left), moved(right)),
                Node::Ne(left, right) => Node::Ne(moved(left), moved(right)),
                Node::Lt(left, right) => Node::Lt(moved(left), moved(right)),
                Node::Le(left, right) => Node::Le(moved(left), moved(right)),
                Node::Gt(left, right) => Node::Gt(moved(left), moved(right)),
                Node::Ge(left, right) => Node::Ge(moved(left), moved(right)),
                other => other,
            };
            let new = self.push(rebuilt);
            if at(&self.reaches, id) == 0 {
                self.lasting.insert(id, new);
            }
            made.insert(id, new);
        }
        made.get(&root).copied().unwrap_or(root)
    }
}

fn undone(dag: &Dag, steps: &[Lift]) -> Dag {
    let inputs = FIRST_PARAM
        .saturating_add(u32::try_from(dag.params()).unwrap_or(u32::MAX))
        .max(WORDS);
    let mut copier = Copier {
        dag,
        nodes: (0..inputs).map(Node::Input).collect(),
        reaches: masks(dag).reaches,
        lasting: HashMap::new(),
    };
    let mut current: [u32; CORNERS] = core::array::from_fn(word_of);
    let mut holding: [u32; CORNERS] = *dag.out();
    let mut now: HashMap<u32, u32> = holding.iter().copied().zip(current).collect();
    for lift in steps.iter().rev() {
        let by = copier.copied(lift.by, &now);
        let (Some(word), Some(held)) = (current.get_mut(lift.word), holding.get_mut(lift.word))
        else {
            continue;
        };
        let made = copier.push(lift.how.undone().node(*word, by));
        now.remove(held);
        *held = lift.before;
        *word = made;
        now.insert(lift.before, made);
    }
    let zero = copier.push(Node::Const(0));
    let counts = vec![zero; dag.counts().len()];
    Dag::new(copier.nodes, current, counts, dag.params(), dag.tally())
}

#[derive(Clone, Debug)]
pub struct Reversible {
    forward: Dag,
    inverse: Dag,
    steps: Vec<Lift>,
}

impl Reversible {
    pub fn of(law: &Law) -> Result<Self, Refusal> {
        let dag = law.dag();
        let steps = lifted(dag).map_err(|broken| broken.refusal(dag, law.places()))?;
        if !law.reversible() {
            return Err(Refusal {
                code: Code::Missing,
                path: String::from("$.reversible"),
                expected: String::from("\"reversible\": true, to take gradients through the law"),
                found: String::from("no such key"),
                limit: None,
            });
        }
        Ok(Self::from_steps(dag, steps))
    }

    pub fn of_dag(dag: &Dag) -> Result<Self, Broken> {
        lifted(dag).map(|steps| Self::from_steps(dag, steps))
    }

    fn from_steps(dag: &Dag, steps: Vec<Lift>) -> Self {
        Self {
            forward: dag.clone(),
            inverse: undone(dag, &steps),
            steps,
        }
    }

    #[must_use]
    pub const fn forward(&self) -> &Dag {
        &self.forward
    }

    #[must_use]
    pub const fn inverse(&self) -> &Dag {
        &self.inverse
    }

    #[must_use]
    pub fn steps(&self) -> &[Lift] {
        &self.steps
    }

    pub fn carried(
        &self,
        oct: Oct<'_, u32>,
        weights: [i128; CORNERS],
        told: &mut [i128],
    ) -> Result<[i128; CORNERS], Fault> {
        let values = evaluated(&self.forward, &oct);
        let mut adjoint = vec![0_i128; values.len()];
        for (&id, weight) in self.forward.out().iter().zip(weights) {
            added(&mut adjoint, id, weight)?;
        }
        let mut before = [0_i128; CORNERS];
        for (index, &node) in self.forward.nodes().iter().enumerate().rev() {
            let weight = adjoint.get(index).copied().unwrap_or(0);
            if weight == 0 {
                continue;
            }
            if let Node::Input(which) = node {
                if which < WORDS {
                    added(&mut before, which, weight)?;
                } else if let Some(param) = which.checked_sub(FIRST_PARAM) {
                    added(told, param, weight)?;
                }
                continue;
            }
            for (id, share) in shares(node, weight, &values)?.into_iter().flatten() {
                added(&mut adjoint, id, share)?;
            }
        }
        Ok(before)
    }

    pub fn walk(
        &self,
        grid: Grid3,
        start: &[u32],
        params: &[u32],
        first: u32,
        frames: u32,
        aim: &Aim<'_>,
    ) -> Result<Walked, Fault> {
        if start.len() != grid.cells() {
            return Err(Fault::Cells {
                given: start.len(),
                wanted: grid.cells(),
            });
        }
        if params.len() != self.forward.params() {
            return Err(Fault::Params {
                given: params.len(),
                wanted: self.forward.params(),
            });
        }
        aim.fits(grid)?;
        let mut cells = start.to_vec();
        for frame in (0..frames).map(|done| first.wrapping_add(done)) {
            cells = reference(&self.forward, grid, frame, &cells, params);
        }
        let ended = cells.clone();
        let objective = aim.measured(grid, &ended);
        let mut weights = aim.weights(grid, &ended);
        let mut told = vec![0_i128; params.len()];
        for frame in (0..frames).rev().map(|done| first.wrapping_add(done)) {
            let before = reference(&self.inverse, grid, frame, &cells, params);
            let mut carried = vec![0_i128; weights.len()];
            for oct in 0..grid.octs() {
                let places = grid.corners(frame, oct);
                let words = places.map(|place| before.get(place).copied().unwrap_or(0));
                let given = places.map(|place| weights.get(place).copied().unwrap_or(0));
                let back = self.carried(
                    Oct {
                        words,
                        frame,
                        at: lattice(grid, oct),
                        params,
                    },
                    given,
                    &mut told,
                )?;
                for (place, value) in places.into_iter().zip(back) {
                    if let Some(slot) = carried.get_mut(place) {
                        *slot = value;
                    }
                }
            }
            weights = carried;
            cells = before;
        }
        if let Some((at, (&was, &now))) = start
            .iter()
            .zip(&cells)
            .enumerate()
            .find(|(_, (was, now))| was != now)
        {
            return Err(Fault::Strayed { at, was, now });
        }
        Ok(Walked {
            ended,
            objective,
            told,
            slope: weights,
        })
    }

    pub fn descend(&self, world: &Posed<'_>, from: &[u32], most: u32) -> Result<Descent, Fault> {
        let mut params = from.to_vec();
        let mut trail = Vec::new();
        let mut planes: Vec<Plane> = Vec::new();
        let memory = params.len().max(1);
        loop {
            let walked = self.walk(
                world.grid,
                world.start,
                &params,
                world.first,
                world.frames,
                &world.aim,
            )?;
            trail.push((params.clone(), walked.objective));
            let steps = u32::try_from(trail.len().saturating_sub(1)).unwrap_or(u32::MAX);
            planes.push(Plane {
                params: params.clone(),
                objective: walked.objective,
                told: walked.told,
            });
            if planes.len() > memory {
                planes.remove(0);
            }
            let moved = stepped(&planes, &params);
            if walked.objective == 0 || steps >= most || moved.iter().all(|&one| one == 0) {
                return Ok(Descent {
                    params,
                    objective: walked.objective,
                    steps,
                    trail,
                });
            }
            for (param, delta) in params.iter_mut().zip(moved) {
                let next = i64::from(*param).saturating_add(delta);
                *param = u32::try_from(next.clamp(0, i64::from(u32::MAX))).unwrap_or(0);
            }
        }
    }
}

fn added(held: &mut [i128], id: u32, weight: i128) -> Result<(), Fault> {
    if let Some(slot) = usize::try_from(id)
        .ok()
        .and_then(|index| held.get_mut(index))
    {
        *slot = slot.checked_add(weight).ok_or(Fault::Overflow)?;
    }
    Ok(())
}

type Shares = [Option<(u32, i128)>; 2];

fn shares(node: Node, weight: i128, values: &[u32]) -> Result<Shares, Fault> {
    let value = |id: u32| i128::from(at(values, id));
    let times = |by: i128| weight.checked_mul(by).ok_or(Fault::Overflow);
    Ok(match node {
        Node::Add(left, right) => [Some((left, weight)), Some((right, weight))],
        Node::Sub(left, right) => [Some((left, weight)), Some((right, times(-1)?))],
        Node::Mul(left, right) => [
            Some((left, times(value(right))?)),
            Some((right, times(value(left))?)),
        ],
        Node::Div(left, right) => {
            let below = value(right);
            if below == 0 {
                [Some((left, weight)), None]
            } else {
                let square = below.checked_mul(below).ok_or(Fault::Overflow)?;
                let against = times(value(left))?
                    .checked_div(square)
                    .ok_or(Fault::Overflow)?;
                [
                    Some((left, weight.checked_div(below).ok_or(Fault::Overflow)?)),
                    Some((right, against.checked_neg().ok_or(Fault::Overflow)?)),
                ]
            }
        }
        Node::Shl(one, by) => [Some((one, times(1_i128 << (by & 31))?)), None],
        Node::Shr(one, by) => [Some((one, weight >> (by & 31))), None],
        Node::Select(flag, then, otherwise) => {
            if at(values, flag) == 0 {
                [Some((otherwise, weight)), None]
            } else {
                [Some((then, weight)), None]
            }
        }
        _ => [None, None],
    })
}

fn evaluated(dag: &Dag, oct: &Oct<'_, u32>) -> Vec<u32> {
    let cpu = &mut Cpu;
    let mut values: Vec<u32> = Vec::with_capacity(dag.len());
    for &node in dag.nodes() {
        let get = |id: u32| at(&values, id);
        let made = match node {
            Node::Input(which) => input(oct, which),
            Node::Const(value) => value,
            Node::Not(one) => cpu.not(get(one)),
            Node::Shl(one, by) => cpu.shift_left(get(one), by),
            Node::Shr(one, by) => cpu.shift_right(get(one), by),
            Node::Select(flag, then, otherwise) => {
                cpu.select(Flag::raised(get(flag)), get(then), get(otherwise))
            }
            Node::And(left, right) => cpu.and(get(left), get(right)),
            Node::Or(left, right) => cpu.or(get(left), get(right)),
            Node::Xor(left, right) => cpu.xor(get(left), get(right)),
            Node::Add(left, right) => cpu.add(get(left), get(right)),
            Node::Sub(left, right) => cpu.sub(get(left), get(right)),
            Node::Mul(left, right) => cpu.mul(get(left), get(right)),
            Node::Div(left, right) => cpu.div(get(left), get(right)),
            Node::DotS(left, right) => cpu.dot_signed(get(left), get(right)),
            Node::DotU(left, right) => cpu.dot_unsigned(get(left), get(right)),
            Node::Eq(left, right) => cpu.equal(get(left), get(right)).word(),
            Node::Ne(left, right) => cpu.not_equal(get(left), get(right)).word(),
            Node::Lt(left, right) => cpu.less(get(left), get(right)).word(),
            Node::Le(left, right) => cpu.less_equal(get(left), get(right)).word(),
            Node::Gt(left, right) => cpu.greater(get(left), get(right)).word(),
            Node::Ge(left, right) => cpu.greater_equal(get(left), get(right)).word(),
        };
        values.push(made);
    }
    values
}

fn input(oct: &Oct<'_, u32>, which: u32) -> u32 {
    let found = match which {
        0..WORDS => oct.words.get(usize::try_from(which).unwrap_or(usize::MAX)),
        FRAME => Some(&oct.frame),
        AT..FIRST_PARAM => usize::try_from(which - AT)
            .ok()
            .and_then(|axis| oct.at.get(axis)),
        _ => usize::try_from(which - FIRST_PARAM)
            .ok()
            .and_then(|param| oct.params.get(param)),
    };
    found.copied().unwrap_or(0)
}

fn magnitude(value: i128) -> u32 {
    128 - value.unsigned_abs().leading_zeros()
}

const NORMAL_BITS: u32 = 30;

const POINT_BITS: u32 = 32;

const SCALE_BITS: u32 = 20;

const DEPENDENT_BITS: u32 = 40;

const MOST_STEP: i128 = 1 << 31;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plane {
    pub params: Vec<u32>,
    pub objective: u64,
    pub told: Vec<i128>,
}

fn dot(left: &[i128], right: &[i128]) -> Option<i128> {
    left.iter()
        .zip(right)
        .try_fold(0_i128, |acc, (one, other)| {
            acc.checked_add(one.checked_mul(*other)?)
        })
}

fn shifted(value: i128, by: i64) -> Option<i128> {
    if by >= 0 {
        let by = u32::try_from(by).ok().filter(|&by| by < 127)?;
        let made = value.checked_mul(1_i128 << by)?;
        Some(made)
    } else {
        Some(value >> by.unsigned_abs().min(127))
    }
}

fn normal(plane: &Plane, current: &[u32]) -> Option<(Vec<i128>, i128)> {
    let largest = plane.told.iter().map(|one| magnitude(*one)).max()?;
    if largest == 0 {
        return None;
    }
    let shift = largest.saturating_sub(NORMAL_BITS);
    let normal: Vec<i128> = plane.told.iter().map(|one| one >> shift).collect();
    let reach = shifted(
        i128::from(plane.objective),
        i64::from(FRACTION) - i64::from(shift),
    )?;
    let moved: Vec<i128> = current
        .iter()
        .zip(&plane.params)
        .map(|(&now, &then)| i128::from(now) - i128::from(then))
        .collect();
    let along = dot(&normal, &moved)?;
    let offset = reach.checked_neg()?.checked_sub(along)?;
    Some((normal, shifted(offset, i64::from(POINT_BITS))?))
}

fn projected(planes: &[Plane], current: &[u32]) -> Option<Vec<i128>> {
    let mut delta = vec![0_i128; current.len()];
    let mut basis: Vec<(Vec<i128>, i128)> = Vec::new();
    for plane in planes.iter().rev() {
        let Some((normal, offset)) = normal(plane, current) else {
            continue;
        };
        let length = dot(&normal, &normal)?;
        let mut kept = normal.clone();
        for (held, square) in &basis {
            let share = dot(&normal, held)?
                .checked_mul(1_i128 << SCALE_BITS)?
                .checked_div(*square)?;
            for (one, other) in kept.iter_mut().zip(held) {
                *one = one.checked_sub(share.checked_mul(*other)? >> SCALE_BITS)?;
            }
        }
        let square = dot(&kept, &kept)?;
        if square == 0 || square < length >> DEPENDENT_BITS {
            continue;
        }
        let rest = offset.checked_sub(dot(&normal, &delta)?)?;
        let along = rest
            .checked_mul(1_i128 << SCALE_BITS)?
            .checked_div(square)?;
        for (one, other) in delta.iter_mut().zip(&kept) {
            *one = one.checked_add(along.checked_mul(*other)? >> SCALE_BITS)?;
        }
        basis.push((kept, square));
    }
    if basis.is_empty() {
        return None;
    }
    let half = 1_i128 << (POINT_BITS - 1);
    Some(
        delta
            .iter()
            .map(|one| (one.saturating_add(half) >> POINT_BITS).clamp(-MOST_STEP, MOST_STEP))
            .collect(),
    )
}

#[must_use]
pub fn stepped(planes: &[Plane], current: &[u32]) -> Vec<i64> {
    let mut moved: Vec<i64> = projected(planes, current)
        .unwrap_or_default()
        .into_iter()
        .map(|one| i64::try_from(one).unwrap_or(0))
        .collect();
    moved.resize(current.len(), 0);
    let objective = planes.last().map_or(0, |plane| plane.objective);
    if objective > 0
        && moved.iter().all(|&one| one == 0)
        && let Some(told) = planes.last().map(|plane| plane.told.as_slice())
        && let Some((index, &steepest)) = told
            .iter()
            .enumerate()
            .max_by_key(|(_, one)| one.unsigned_abs())
        && steepest != 0
        && let Some(slot) = moved.get_mut(index)
    {
        *slot = if steepest > 0 { -1 } else { 1 };
    }
    moved
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Plain;

impl Map for Plain {
    fn map<O: Ops>(&self, ops: &mut O, seen: Seen<'_, O::Word>) -> O::Word {
        match seen.cell() {
            Some(word) => word,
            None => ops.constant(0),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Aim<'t> {
    pub region: Region,
    pub target: &'t [u32],
}

const fn size(grid: Grid3) -> [u32; 3] {
    [grid.wide, grid.deep, grid.tall]
}

impl Aim<'_> {
    fn fits(&self, grid: Grid3) -> Result<(), Fault> {
        let inside = self
            .region
            .to
            .iter()
            .zip(size(grid))
            .all(|(&to, side)| to <= side)
            && self
                .region
                .from
                .iter()
                .zip(self.region.to)
                .all(|(&from, to)| from < to);
        if !inside {
            return Err(Fault::Region {
                region: self.region,
                size: size(grid),
            });
        }
        if self.region.cells() != self.target.len() as u128 {
            return Err(Fault::Target {
                given: self.target.len(),
                wanted: self.region.cells(),
            });
        }
        Ok(())
    }

    #[must_use]
    pub fn measured(&self, grid: Grid3, cells: &[u32]) -> u64 {
        let world = World {
            size: size(grid),
            cells,
        };
        match cpu::answer(
            &Plain,
            Reduce::Differ(Measure::Sum),
            world,
            self.region,
            self.target,
        ) {
            Answer::Differ(total) => total,
            _ => 0,
        }
    }

    #[must_use]
    pub fn weights(&self, grid: Grid3, cells: &[u32]) -> Vec<i128> {
        let mut weights = vec![0_i128; cells.len()];
        let [from_x, from_y, from_z] = self.region.from;
        let [to_x, to_y, to_z] = self.region.to;
        let mut aimed = self.target.iter();
        for z in from_z..to_z {
            for y in from_y..to_y {
                for x in from_x..to_x {
                    let wanted = aimed.next().copied().unwrap_or(0);
                    let place = grid.index(x, y, z);
                    let held = cells.get(place).copied().unwrap_or(0);
                    if let Some(slot) = weights.get_mut(place) {
                        *slot = match held.cmp(&wanted) {
                            core::cmp::Ordering::Greater => 1_i128 << FRACTION,
                            core::cmp::Ordering::Less => -(1_i128 << FRACTION),
                            core::cmp::Ordering::Equal => 0,
                        };
                    }
                }
            }
        }
        weights
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Posed<'w> {
    pub grid: Grid3,
    pub start: &'w [u32],
    pub first: u32,
    pub frames: u32,
    pub aim: Aim<'w>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Walked {
    pub ended: Vec<u32>,
    pub objective: u64,
    pub told: Vec<i128>,
    pub slope: Vec<i128>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Descent {
    pub params: Vec<u32>,
    pub objective: u64,
    pub steps: u32,
    pub trail: Vec<(Vec<u32>, u64)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    Strayed { at: usize, was: u32, now: u32 },
    Overflow,
    Cells { given: usize, wanted: usize },
    Params { given: usize, wanted: usize },
    Region { region: Region, size: [u32; 3] },
    Target { given: usize, wanted: u128 },
}

impl fmt::Display for Fault {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Strayed { at, was, now } => write!(
                out,
                "cell {at} did not come back to where it started: it was {was} and is now {now}"
            ),
            Self::Overflow => write!(
                out,
                "a gradient passed the 128-bit fixed point it is carried in"
            ),
            Self::Cells { given, wanted } => {
                write!(out, "the world has {wanted} cells and {given} were given")
            }
            Self::Params { given, wanted } => {
                write!(out, "the law reads {wanted} params and {given} were given")
            }
            Self::Region { region, size } => write!(
                out,
                "the box {:?} to {:?} is empty or leaves the world of size {size:?}",
                region.from, region.to
            ),
            Self::Target { given, wanted } => {
                write!(out, "the box holds {wanted} cells and the target {given}")
            }
        }
    }
}

impl std::error::Error for Fault {}
