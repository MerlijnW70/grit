use super::dag::{AT, Dag, FIRST_PARAM, FRAME, Node};
use crate::ops::{Flag, Ops};
use crate::rule3::{CORNERS, Oct, OneStep};

#[derive(Debug, Default)]
pub(crate) struct Tracer {
    pub(crate) nodes: Vec<Node>,
}

impl Tracer {
    pub(crate) fn with_inputs(params: usize) -> Self {
        let inputs = FIRST_PARAM.saturating_add(u32::try_from(params).unwrap_or(u32::MAX));
        Self {
            nodes: (0..inputs).map(Node::Input).collect(),
        }
    }

    pub(crate) fn input(&mut self, which: u32) -> u32 {
        self.push(Node::Input(which))
    }

    fn push(&mut self, node: Node) -> u32 {
        let id = u32::try_from(self.nodes.len()).unwrap_or(u32::MAX);
        self.nodes.push(node);
        id
    }

    fn flagged(&mut self, node: Node) -> Flag<u32> {
        Flag::raised(self.push(node))
    }
}

impl Ops for Tracer {
    type Word = u32;

    fn constant(&mut self, value: u32) -> u32 {
        self.push(Node::Const(value))
    }

    fn and(&mut self, left: u32, right: u32) -> u32 {
        self.push(Node::And(left, right))
    }

    fn or(&mut self, left: u32, right: u32) -> u32 {
        self.push(Node::Or(left, right))
    }

    fn xor(&mut self, left: u32, right: u32) -> u32 {
        self.push(Node::Xor(left, right))
    }

    fn not(&mut self, value: u32) -> u32 {
        self.push(Node::Not(value))
    }

    fn shift_left(&mut self, value: u32, by: u32) -> u32 {
        self.push(Node::Shl(value, by))
    }

    fn shift_right(&mut self, value: u32, by: u32) -> u32 {
        self.push(Node::Shr(value, by))
    }

    fn add(&mut self, left: u32, right: u32) -> u32 {
        self.push(Node::Add(left, right))
    }

    fn sub(&mut self, left: u32, right: u32) -> u32 {
        self.push(Node::Sub(left, right))
    }

    fn mul(&mut self, left: u32, right: u32) -> u32 {
        self.push(Node::Mul(left, right))
    }

    fn div(&mut self, left: u32, right: u32) -> u32 {
        self.push(Node::Div(left, right))
    }

    fn dot_signed(&mut self, left: u32, right: u32) -> u32 {
        self.push(Node::DotS(left, right))
    }

    fn dot_unsigned(&mut self, left: u32, right: u32) -> u32 {
        self.push(Node::DotU(left, right))
    }

    fn equal(&mut self, left: u32, right: u32) -> Flag<u32> {
        self.flagged(Node::Eq(left, right))
    }

    fn not_equal(&mut self, left: u32, right: u32) -> Flag<u32> {
        self.flagged(Node::Ne(left, right))
    }

    fn less(&mut self, left: u32, right: u32) -> Flag<u32> {
        self.flagged(Node::Lt(left, right))
    }

    fn less_equal(&mut self, left: u32, right: u32) -> Flag<u32> {
        self.flagged(Node::Le(left, right))
    }

    fn greater(&mut self, left: u32, right: u32) -> Flag<u32> {
        self.flagged(Node::Gt(left, right))
    }

    fn greater_equal(&mut self, left: u32, right: u32) -> Flag<u32> {
        self.flagged(Node::Ge(left, right))
    }

    fn select(&mut self, flag: Flag<u32>, then: u32, otherwise: u32) -> u32 {
        self.push(Node::Select(flag.word(), then, otherwise))
    }
}

pub fn trace<R: OneStep>(rule: &R) -> Dag {
    let params = rule.params();
    let mut tracer = Tracer::with_inputs(params);
    let ids: Vec<u32> = (FIRST_PARAM..).take(params).collect();
    let words: [u32; CORNERS] = core::array::from_fn(|corner| u32::try_from(corner).unwrap_or(0));
    let ruled = rule.one_step(
        &mut tracer,
        Oct {
            words,
            frame: FRAME,
            at: [AT, AT + 1, AT + 2],
            params: &ids,
        },
    );
    Dag::new(tracer.nodes, ruled.out, ruled.counts, params, rule.tally())
}
