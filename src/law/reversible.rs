use super::adjoint::{Lift, Reversible};
use super::dag::{AT, Dag, FIRST_PARAM, FRAME, Node};
use super::{Law, Refusal, limits};
use crate::halo::{self, BOX};
use crate::rule3::{CORNERS, Tally};
use crate::wire::{self, Json};

pub const SUFFIX: &str = "_back";

#[derive(Clone, Copy, Debug)]
pub(crate) struct Shape<'s> {
    pub name: &'s str,
    pub params: usize,
    pub channels: u32,
    pub halo: u32,
    pub tally: Tally,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Kind {
    #[default]
    Word,
    Flag,
    Int(u32),
}

#[must_use]
pub fn out_path(json: &Json, word: usize, channels: u32) -> String {
    let mut path = String::from("$.step");
    let Some(mut here) = json.get("step") else {
        return path;
    };
    while let Json::List(items) = here
        && items.first().and_then(Json::text) == Some("let")
        && let Some(body) = items.get(2)
    {
        here = body;
        path.push_str("[2]");
    }
    let listed = |held: &Json| match held {
        Json::List(items) if items.first().and_then(Json::text) == Some("vec") => {
            Some(items.clone())
        }
        _ => None,
    };
    let Some(result) = listed(here) else {
        return path;
    };
    let Some(out) = result.get(1) else {
        return path;
    };
    path.push_str("[1]");
    let Some(corners) = listed(out) else {
        return path;
    };
    let corner = word % CORNERS;
    let Some(cell) = corners.get(corner + 1) else {
        return path;
    };
    path.push_str(&format!("[{}]", corner + 1));
    if channels <= 1 {
        return path;
    }
    if let Some(planes) = listed(cell)
        && planes.len() > word / CORNERS + 1
    {
        path.push_str(&format!("[{}]", word / CORNERS + 1));
    }
    path
}

#[must_use]
pub fn back_name(name: &str) -> String {
    let room = limits::NAME.saturating_sub(SUFFIX.len());
    let mut end = name.len().min(room);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{SUFFIX}", name.get(..end).unwrap_or_default())
}

pub(crate) fn inverse(shape: Shape<'_>, dag: &Dag, steps: Vec<Lift>) -> Result<Law, Refusal> {
    let undone = Reversible::from_steps(dag, steps);
    let json = written(shape, undone.inverse());
    Law::derived(&json).map_err(|why| Refusal {
        code: why.code,
        path: String::from("$.reversible"),
        expected: String::from("a reversible law whose derived inverse passes every law limit"),
        found: format!("its inverse is refused: {why}"),
        limit: why.limit,
    })
}

fn op(name: &str, args: Vec<Json>) -> Json {
    let mut held = Vec::with_capacity(args.len() + 1);
    held.push(wire::text(name));
    held.extend(args);
    Json::List(held)
}

fn int(value: u32) -> Json {
    Json::Int(i64::from(value))
}

fn bound(id: usize) -> String {
    format!("v{id}")
}

fn at<T: Copy + Default>(held: &[T], id: u32) -> T {
    usize::try_from(id)
        .ok()
        .and_then(|index| held.get(index))
        .copied()
        .unwrap_or_default()
}

fn kinds(dag: &Dag) -> Vec<Kind> {
    let mut held: Vec<Kind> = Vec::with_capacity(dag.len());
    for &node in dag.nodes() {
        let kind = match node {
            Node::Const(value) => Kind::Int(value),
            Node::Eq(..)
            | Node::Ne(..)
            | Node::Lt(..)
            | Node::Le(..)
            | Node::Gt(..)
            | Node::Ge(..) => Kind::Flag,
            Node::And(left, right) | Node::Or(left, right)
                if at(&held, left) == Kind::Flag && at(&held, right) == Kind::Flag =>
            {
                Kind::Flag
            }
            Node::Xor(left, right) if denied(&held, left, right).is_some() => Kind::Flag,
            _ => Kind::Word,
        };
        held.push(kind);
    }
    held
}

fn denied(held: &[Kind], left: u32, right: u32) -> Option<u32> {
    match (at(held, left), at(held, right)) {
        (Kind::Flag, Kind::Int(1)) => Some(left),
        (Kind::Int(1), Kind::Flag) => Some(right),
        _ => None,
    }
}

fn reached(dag: &Dag) -> Vec<bool> {
    let mut live = vec![false; dag.len()];
    let mut pending: Vec<u32> = dag
        .out()
        .iter()
        .chain(dag.planes().iter().flatten())
        .chain(dag.counts())
        .copied()
        .collect();
    while let Some(id) = pending.pop() {
        let Some(slot) = usize::try_from(id)
            .ok()
            .and_then(|index| live.get_mut(index))
        else {
            continue;
        };
        if *slot {
            continue;
        }
        *slot = true;
        let node = usize::try_from(id)
            .ok()
            .and_then(|index| dag.nodes().get(index))
            .copied();
        pending.extend(match node {
            Some(Node::Not(one) | Node::Shl(one, _) | Node::Shr(one, _)) => vec![one],
            Some(Node::Select(flag, then, otherwise)) => vec![flag, then, otherwise],
            Some(
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
                | Node::Ge(left, right),
            ) => vec![left, right],
            _ => Vec::new(),
        });
    }
    live
}

struct Writer<'d> {
    dag: &'d Dag,
    kinds: Vec<Kind>,
    channels: u32,
}

impl Writer<'_> {
    fn word(&self, id: u32) -> Json {
        match at(&self.kinds, id) {
            Kind::Int(value) => int(value),
            Kind::Flag => op("word", vec![Json::Text(bound(id as usize))]),
            Kind::Word => Json::Text(bound(id as usize)),
        }
    }

    fn flag(&self, id: u32) -> Json {
        match at(&self.kinds, id) {
            Kind::Flag => Json::Text(bound(id as usize)),
            Kind::Int(value) => op("ne", vec![int(value), int(0)]),
            Kind::Word => op("ne", vec![self.word(id), int(0)]),
        }
    }

    fn cell(&self, corner: u32, plane: u32) -> Json {
        let cell = op("at", vec![wire::text("cells"), int(corner)]);
        if self.channels <= 1 {
            return cell;
        }
        op("at", vec![cell, int(plane)])
    }

    fn input(&self, which: u32) -> Json {
        let corners = u32::try_from(CORNERS).unwrap_or(8);
        let first_plane = self.dag.first_plane();
        let first_halo = self.dag.first_halo();
        let boxed = u32::try_from(BOX).unwrap_or(u32::MAX);
        match which {
            _ if which >= first_halo => {
                let above = which - first_halo;
                let plane = above / boxed;
                let cell = usize::try_from(above % boxed).unwrap_or(usize::MAX);
                looked(plane, cell)
            }
            _ if which >= first_plane => {
                let above = which - first_plane;
                self.cell(above % corners, above / corners + 1)
            }
            0..FRAME => self.cell(which, 0),
            FRAME => wire::text("frame"),
            AT..FIRST_PARAM => op("at", vec![wire::text("at"), int(which - AT)]),
            _ => op("param", vec![int(which - FIRST_PARAM)]),
        }
    }

    fn value(&self, node: Node) -> Json {
        let word = |id: u32| self.word(id);
        let paired = |name: &str, left: u32, right: u32| op(name, vec![word(left), word(right)]);
        match node {
            Node::Input(which) => self.input(which),
            Node::Const(value) => int(value),
            Node::Not(one) => op("not", vec![word(one)]),
            Node::Shl(one, by) => op("shl", vec![word(one), int(by)]),
            Node::Shr(one, by) => op("shr", vec![word(one), int(by)]),
            Node::Select(flag, then, otherwise) => {
                op("sel", vec![self.flag(flag), word(then), word(otherwise)])
            }
            Node::And(left, right) if self.both_flags(left, right) => {
                op("both", vec![self.flag(left), self.flag(right)])
            }
            Node::Or(left, right) if self.both_flags(left, right) => {
                op("either", vec![self.flag(left), self.flag(right)])
            }
            Node::Xor(left, right) => match denied(&self.kinds, left, right) {
                Some(flag) => op("denied", vec![self.flag(flag)]),
                None => paired("xor", left, right),
            },
            Node::And(left, right) => paired("and", left, right),
            Node::Or(left, right) => paired("or", left, right),
            Node::Add(left, right) => paired("add", left, right),
            Node::Sub(left, right) => paired("sub", left, right),
            Node::Mul(left, right) => paired("mul", left, right),
            Node::Div(left, right) => paired("div", left, right),
            Node::DotS(left, right) => paired("dot_s", left, right),
            Node::DotU(left, right) => paired("dot_u", left, right),
            Node::Eq(left, right) => paired("eq", left, right),
            Node::Ne(left, right) => paired("ne", left, right),
            Node::Lt(left, right) => paired("lt", left, right),
            Node::Le(left, right) => paired("le", left, right),
            Node::Gt(left, right) => paired("gt", left, right),
            Node::Ge(left, right) => paired("ge", left, right),
        }
    }

    fn both_flags(&self, left: u32, right: u32) -> bool {
        at(&self.kinds, left) == Kind::Flag && at(&self.kinds, right) == Kind::Flag
    }
}

fn looked(plane: u32, cell: usize) -> Json {
    for corner in 0..CORNERS {
        for dz in -1..=1 {
            for dy in -1..=1 {
                for dx in -1..=1 {
                    if halo::looked(corner, [dx, dy, dz]) == Some(cell) && halo::own(cell).is_none()
                    {
                        return op(
                            "look",
                            vec![
                                int(plane),
                                Json::Int(i64::from(dx)),
                                Json::Int(i64::from(dy)),
                                Json::Int(i64::from(dz)),
                                Json::Int(i64::try_from(corner).unwrap_or(0)),
                            ],
                        );
                    }
                }
            }
        }
    }
    int(0)
}

fn written(shape: Shape<'_>, dag: &Dag) -> Json {
    let writer = Writer {
        dag,
        kinds: kinds(dag),
        channels: shape.channels,
    };
    let live = reached(dag);
    let mut bindings: Vec<Json> = dag
        .halo()
        .each()
        .map(|(plane, cell)| {
            Json::List(vec![
                Json::Text(format!("h{plane}c{cell}")),
                looked(u32::try_from(plane).unwrap_or(0), cell),
            ])
        })
        .collect();
    for (index, &node) in dag.nodes().iter().enumerate() {
        if !live.get(index).copied().unwrap_or(false) || matches!(node, Node::Const(_)) {
            continue;
        }
        bindings.push(Json::List(vec![
            Json::Text(bound(index)),
            writer.value(node),
        ]));
    }
    let out: Vec<Json> = (0..CORNERS)
        .map(|corner| {
            let first = dag.out().get(corner).copied().unwrap_or(0);
            if shape.channels <= 1 {
                return writer.word(first);
            }
            let mut words = vec![writer.word(first)];
            words.extend(
                dag.planes()
                    .iter()
                    .map(|plane| writer.word(plane.get(corner).copied().unwrap_or(0))),
            );
            op("vec", words)
        })
        .collect();
    let counts: Vec<Json> = dag.counts().iter().map(|&id| writer.word(id)).collect();
    let step = op(
        "let",
        vec![
            Json::List(bindings),
            op("vec", vec![op("vec", out), op("vec", counts)]),
        ],
    );
    let wide = |value: usize| Json::Int(i64::try_from(value).unwrap_or(i64::MAX));
    wire::map(vec![
        ("law", int(limits::VERSION)),
        ("name", wire::text(&back_name(shape.name))),
        ("params", wide(shape.params)),
        ("channels", int(shape.channels)),
        ("halo", int(shape.halo)),
        ("reversible", Json::Bool(true)),
        (
            "tally",
            wire::map(vec![
                ("counts", wide(shape.tally.counts)),
                ("weighted", Json::Bool(shape.tally.weighted)),
            ]),
        ),
        ("consts", Json::Map(Vec::new())),
        ("defs", Json::Map(Vec::new())),
        ("step", step),
    ])
}
