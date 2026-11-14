use crate::halo::BOX;
use crate::law::Law;
use crate::law::dag::{AT, Dag, FIRST_PARAM, FRAME, Node};
use crate::rule3::{CORNERS, Declared, Oct, Rule};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Reads {
    pub frame: bool,
    pub at: bool,
}

impl Reads {
    #[must_use]
    pub const fn any(self) -> bool {
        self.frame || self.at
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Class {
    Proved,
    Whole(Why),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Why {
    Reads(Reads),
    Moves { value: Vec<u32>, to: Vec<u32> },
}

impl Class {
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Proved => "proved",
            Self::Whole(_) => "whole",
        }
    }

    #[must_use]
    pub const fn proved(&self) -> bool {
        matches!(self, Self::Proved)
    }

    #[must_use]
    pub fn why(&self) -> Option<String> {
        match self {
            Self::Proved => None,
            Self::Whole(Why::Reads(reads)) => {
                let read = match (reads.frame, reads.at) {
                    (true, true) => "frame and at (noise reads both)",
                    (true, false) => "frame",
                    _ => "at",
                };
                Some(format!("its cells depend on {read}"))
            }
            Self::Whole(Why::Moves { value, to }) => Some(format!(
                "an oct holding {value:?} on every cell steps to {to:?}, so a uniform brick of it is not a fixed point"
            )),
        }
    }
}

const FRAME_BIT: u8 = 1;

const AT_BIT: u8 = 2;

const fn input_bits(which: u32) -> u8 {
    match which {
        FRAME => FRAME_BIT,
        AT..FIRST_PARAM => AT_BIT,
        _ => 0,
    }
}

fn bits_of(bits: &[u8], id: u32) -> u8 {
    usize::try_from(id)
        .ok()
        .and_then(|at| bits.get(at))
        .copied()
        .unwrap_or(0)
}

#[must_use]
pub fn reads(dag: &Dag) -> Reads {
    let mut bits: Vec<u8> = Vec::with_capacity(dag.len());
    for &node in dag.nodes() {
        let made = match node {
            Node::Input(which) => input_bits(which),
            Node::Const(_) => 0,
            Node::Not(one) | Node::Shl(one, _) | Node::Shr(one, _) => bits_of(&bits, one),
            Node::Select(flag, then, otherwise) => {
                bits_of(&bits, flag) | bits_of(&bits, then) | bits_of(&bits, otherwise)
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
            | Node::Ge(left, right) => bits_of(&bits, left) | bits_of(&bits, right),
        };
        bits.push(made);
    }
    let read = dag
        .out()
        .iter()
        .chain(dag.planes().iter().flatten())
        .fold(0_u8, |all, &id| all | bits_of(&bits, id));
    Reads {
        frame: read & FRAME_BIT != 0,
        at: read & AT_BIT != 0,
    }
}

#[must_use]
pub fn stepped(law: &Law, params: &[u32], value: &[u32]) -> Vec<u32> {
    let planes = law.planes();
    let word = |plane: usize| value.get(plane).copied().unwrap_or(0);
    let oct = Oct {
        words: [word(0); CORNERS],
        frame: 0,
        at: [0; 3],
        params,
    };
    let upper: Vec<[u32; CORNERS]> = (1..planes).map(|plane| [word(plane); CORNERS]).collect();
    let planed = if law.looks().is_empty() {
        law.step_planes(oct, &upper)
    } else {
        let halo: Vec<[u32; BOX]> = (0..planes).map(|plane| [word(plane); BOX]).collect();
        law.step_halo(oct, &upper, &halo)
    };
    let mut out = Vec::with_capacity(planes * CORNERS);
    out.extend_from_slice(&planed.ruled.out);
    for plane in 1..planes {
        match planed.planes.get(plane - 1) {
            Some(words) => out.extend_from_slice(words),
            None => out.extend_from_slice(&[word(plane); CORNERS]),
        }
    }
    out
}

#[must_use]
pub fn moved(law: &Law, params: &[u32], value: &[u32]) -> Option<Vec<u32>> {
    let out = stepped(law, params, value);
    let still = out.chunks(CORNERS).enumerate().all(|(plane, words)| {
        let held = value.get(plane).copied().unwrap_or(0);
        words.iter().all(|&word| word == held)
    });
    (!still).then_some(out)
}

#[must_use]
pub fn background(law: &Law) -> Vec<u32> {
    vec![0; law.planes()]
}

#[must_use]
pub fn class(law: &Law, params: &[u32], uniforms: &[Vec<u32>]) -> Class {
    let read = reads(law.dag());
    if read.any() {
        return Class::Whole(Why::Reads(read));
    }
    let empty = background(law);
    for value in std::iter::once(&empty).chain(uniforms) {
        if let Some(to) = moved(law, params, value) {
            return Class::Whole(Why::Moves {
                value: value.clone(),
                to,
            });
        }
    }
    Class::Proved
}
