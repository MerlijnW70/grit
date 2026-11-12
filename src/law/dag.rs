use crate::ops::{Flag, Ops};
use crate::rule3::{CORNERS, Declared, Oct, OneStep, Ruled, Tally};

pub const FRAME: u32 = 8;

pub const AT: u32 = 9;

pub const FIRST_PARAM: u32 = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Node {
    Input(u32),
    Const(u32),
    And(u32, u32),
    Or(u32, u32),
    Xor(u32, u32),
    Not(u32),
    Shl(u32, u32),
    Shr(u32, u32),
    Add(u32, u32),
    Sub(u32, u32),
    Mul(u32, u32),
    Div(u32, u32),
    DotS(u32, u32),
    DotU(u32, u32),
    Eq(u32, u32),
    Ne(u32, u32),
    Lt(u32, u32),
    Le(u32, u32),
    Gt(u32, u32),
    Ge(u32, u32),
    Select(u32, u32, u32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dag {
    nodes: Vec<Node>,
    out: [u32; CORNERS],
    counts: Vec<u32>,
    params: usize,
    tally: Tally,
}

fn held<O: Ops>(ops: &mut O, values: &[O::Word], id: u32) -> O::Word {
    match usize::try_from(id).ok().and_then(|at| values.get(at)) {
        Some(&word) => word,
        None => ops.constant(0),
    }
}

fn input<O: Ops>(ops: &mut O, oct: &Oct<'_, O::Word>, which: u32) -> O::Word {
    let found = match which {
        0..FRAME => usize::try_from(which)
            .ok()
            .and_then(|at| oct.words.get(at))
            .copied(),
        FRAME => Some(oct.frame),
        AT..FIRST_PARAM => usize::try_from(which - AT)
            .ok()
            .and_then(|at| oct.at.get(at))
            .copied(),
        _ => usize::try_from(which - FIRST_PARAM)
            .ok()
            .and_then(|at| oct.params.get(at))
            .copied(),
    };
    match found {
        Some(word) => word,
        None => ops.constant(0),
    }
}

impl Dag {
    pub(crate) const fn new(
        nodes: Vec<Node>,
        out: [u32; CORNERS],
        counts: Vec<u32>,
        params: usize,
        tally: Tally,
    ) -> Self {
        Self {
            nodes,
            out,
            counts,
            params,
            tally,
        }
    }

    #[must_use]
    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    #[must_use]
    pub const fn out(&self) -> &[u32; CORNERS] {
        &self.out
    }

    #[must_use]
    pub fn counts(&self) -> &[u32] {
        &self.counts
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn replay<O: Ops>(&self, ops: &mut O, oct: Oct<'_, O::Word>) -> Ruled<O::Word> {
        let mut values: Vec<O::Word> = Vec::with_capacity(self.nodes.len());
        for &node in &self.nodes {
            let made = match node {
                Node::Input(which) => input(ops, &oct, which),
                Node::Const(value) => ops.constant(value),
                Node::Not(one) => {
                    let one = held(ops, &values, one);
                    ops.not(one)
                }
                Node::Shl(one, by) => {
                    let one = held(ops, &values, one);
                    ops.shift_left(one, by)
                }
                Node::Shr(one, by) => {
                    let one = held(ops, &values, one);
                    ops.shift_right(one, by)
                }
                Node::Select(flag, then, otherwise) => {
                    let flag = held(ops, &values, flag);
                    let then = held(ops, &values, then);
                    let otherwise = held(ops, &values, otherwise);
                    ops.select(Flag::raised(flag), then, otherwise)
                }
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
                | Node::Ge(left, right) => {
                    let left = held(ops, &values, left);
                    let right = held(ops, &values, right);
                    paired(ops, node, left, right)
                }
            };
            values.push(made);
        }
        let out = self.out.map(|id| held(ops, &values, id));
        let counts = self
            .counts
            .iter()
            .map(|&id| held(ops, &values, id))
            .collect();
        Ruled { out, counts }
    }
}

fn paired<O: Ops>(ops: &mut O, node: Node, left: O::Word, right: O::Word) -> O::Word {
    match node {
        Node::And(..) => ops.and(left, right),
        Node::Or(..) => ops.or(left, right),
        Node::Xor(..) => ops.xor(left, right),
        Node::Add(..) => ops.add(left, right),
        Node::Sub(..) => ops.sub(left, right),
        Node::Mul(..) => ops.mul(left, right),
        Node::Div(..) => ops.div(left, right),
        Node::DotS(..) => ops.dot_signed(left, right),
        Node::DotU(..) => ops.dot_unsigned(left, right),
        Node::Eq(..) => ops.equal(left, right).word(),
        Node::Ne(..) => ops.not_equal(left, right).word(),
        Node::Lt(..) => ops.less(left, right).word(),
        Node::Le(..) => ops.less_equal(left, right).word(),
        Node::Gt(..) => ops.greater(left, right).word(),
        Node::Ge(..) => ops.greater_equal(left, right).word(),
        Node::Input(_)
        | Node::Const(_)
        | Node::Not(_)
        | Node::Shl(..)
        | Node::Shr(..)
        | Node::Select(..) => left,
    }
}

impl Declared for Dag {
    fn params(&self) -> usize {
        self.params
    }

    fn tally(&self) -> Tally {
        self.tally
    }
}

impl OneStep for Dag {
    fn one_step<O: Ops>(&self, ops: &mut O, oct: Oct<'_, O::Word>) -> Ruled<O::Word> {
        self.replay(ops, oct)
    }
}
