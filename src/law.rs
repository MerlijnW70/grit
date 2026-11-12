use std::fmt;

use crate::ops::Ops;
use crate::rule3::{Declared, Oct, OneStep, Ruled, Tally};
use crate::wire::{self, Json};

pub mod adjoint;
pub mod dag;
pub mod expand;
pub mod learn;
pub mod limits;
pub mod parse;
pub mod quiet;
pub mod trace;

pub use dag::{Dag, Node};
pub use trace::trace;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Code {
    Json,
    Size,
    Trailing,
    Shape,
    Missing,
    UnknownKey,
    DuplicateKey,
    Version,
    Params,
    Counts,
    Channels,
    Halo,
    Nodes,
    Work,
    Depth,
    Name,
    Literal,
    Reserved,
    UnknownName,
    UnknownOp,
    Arity,
    Type,
    Index,
    Recursion,
    Result,
    Unsupported,
}

impl Code {
    pub const ALL: [Self; 26] = [
        Self::Json,
        Self::Size,
        Self::Trailing,
        Self::Shape,
        Self::Missing,
        Self::UnknownKey,
        Self::DuplicateKey,
        Self::Version,
        Self::Params,
        Self::Counts,
        Self::Channels,
        Self::Halo,
        Self::Nodes,
        Self::Work,
        Self::Depth,
        Self::Name,
        Self::Literal,
        Self::Reserved,
        Self::UnknownName,
        Self::UnknownOp,
        Self::Arity,
        Self::Type,
        Self::Index,
        Self::Recursion,
        Self::Result,
        Self::Unsupported,
    ];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Size => "size",
            Self::Trailing => "trailing",
            Self::Shape => "shape",
            Self::Missing => "missing",
            Self::UnknownKey => "unknown_key",
            Self::DuplicateKey => "duplicate_key",
            Self::Version => "version",
            Self::Params => "params",
            Self::Counts => "counts",
            Self::Channels => "channels",
            Self::Halo => "halo",
            Self::Nodes => "nodes",
            Self::Work => "work",
            Self::Depth => "depth",
            Self::Name => "name",
            Self::Literal => "literal",
            Self::Reserved => "reserved",
            Self::UnknownName => "unknown_name",
            Self::UnknownOp => "unknown_op",
            Self::Arity => "arity",
            Self::Type => "type",
            Self::Index => "index",
            Self::Recursion => "recursion",
            Self::Result => "result",
            Self::Unsupported => "unsupported",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub code: Code,
    pub path: String,
    pub expected: String,
    pub found: String,
    pub limit: Option<u64>,
}

impl Refusal {
    #[must_use]
    pub fn json(&self) -> Json {
        let mut fields = vec![
            ("code", wire::text(self.code.name())),
            ("path", wire::text(&self.path)),
            ("expected", wire::text(&self.expected)),
            ("found", wire::text(&self.found)),
        ];
        if let Some(limit) = self.limit {
            fields.push(("limit", Json::Int(i64::try_from(limit).unwrap_or(i64::MAX))));
        }
        wire::map(fields)
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            out,
            "{} at {}: expected {}, found {}",
            self.code.name(),
            self.path,
            self.expected,
            self.found
        )?;
        match self.limit {
            Some(limit) => write!(out, " (limit {limit})"),
            None => Ok(()),
        }
    }
}

impl std::error::Error for Refusal {}

#[derive(Clone, Debug)]
pub struct Law {
    name: String,
    channels: u32,
    halo: u32,
    dag: Dag,
    canonical: String,
    hash: [u8; 32],
    work: u64,
    depth: usize,
}

impl Law {
    pub fn compile(text: &str) -> Result<Self, Refusal> {
        let json = parse::read(text)?;
        Self::from_json(&json)
    }

    pub fn from_json(json: &Json) -> Result<Self, Refusal> {
        let program = parse::program(json)?;
        let canonical = canonical(json);
        if canonical.len() > limits::BODY {
            return Err(Refusal {
                code: Code::Size,
                path: String::from("$"),
                expected: format!("a canonical law of at most {} bytes", limits::BODY),
                found: format!("{} bytes", canonical.len()),
                limit: Some(limits::BODY as u64),
            });
        }
        let expanded = expand::expand(&program)?;
        let hash = sha256(canonical.as_bytes());
        Ok(Self {
            name: program.name.to_owned(),
            channels: program.channels,
            halo: program.halo,
            dag: expanded.dag,
            canonical,
            hash,
            work: expanded.work,
            depth: expanded.depth,
        })
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn channels(&self) -> u32 {
        self.channels
    }

    #[must_use]
    pub const fn halo(&self) -> u32 {
        self.halo
    }

    #[must_use]
    pub const fn dag(&self) -> &Dag {
        &self.dag
    }

    #[must_use]
    pub fn nodes(&self) -> usize {
        self.dag.len()
    }

    #[must_use]
    pub const fn work(&self) -> u64 {
        self.work
    }

    #[must_use]
    pub const fn depth(&self) -> usize {
        self.depth
    }

    #[must_use]
    pub fn canonical(&self) -> &str {
        &self.canonical
    }

    #[must_use]
    pub const fn hash(&self) -> [u8; 32] {
        self.hash
    }

    #[must_use]
    pub fn hex(&self) -> String {
        hex(&self.hash)
    }
}

impl Declared for Law {
    fn params(&self) -> usize {
        self.dag.params()
    }

    fn tally(&self) -> Tally {
        self.dag.tally()
    }
}

impl OneStep for Law {
    fn one_step<O: Ops>(&self, ops: &mut O, oct: Oct<'_, O::Word>) -> Ruled<O::Word> {
        self.dag.replay(ops, oct)
    }
}

fn sorted(json: &Json) -> Json {
    match json {
        Json::List(items) => Json::List(items.iter().map(sorted).collect()),
        Json::Map(fields) => {
            let mut held: Vec<(String, Json)> = fields
                .iter()
                .map(|(name, one)| (name.clone(), sorted(one)))
                .collect();
            held.sort_by(|left, right| left.0.cmp(&right.0));
            Json::Map(held)
        }
        other => other.clone(),
    }
}

fn canonical(json: &Json) -> String {
    wire::written(&sorted(json))
}

#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

const ROUND: [u32; 64] = [
    0x428a_2f98,
    0x7137_4491,
    0xb5c0_fbcf,
    0xe9b5_dba5,
    0x3956_c25b,
    0x59f1_11f1,
    0x923f_82a4,
    0xab1c_5ed5,
    0xd807_aa98,
    0x1283_5b01,
    0x2431_85be,
    0x550c_7dc3,
    0x72be_5d74,
    0x80de_b1fe,
    0x9bdc_06a7,
    0xc19b_f174,
    0xe49b_69c1,
    0xefbe_4786,
    0x0fc1_9dc6,
    0x240c_a1cc,
    0x2de9_2c6f,
    0x4a74_84aa,
    0x5cb0_a9dc,
    0x76f9_88da,
    0x983e_5152,
    0xa831_c66d,
    0xb003_27c8,
    0xbf59_7fc7,
    0xc6e0_0bf3,
    0xd5a7_9147,
    0x06ca_6351,
    0x1429_2967,
    0x27b7_0a85,
    0x2e1b_2138,
    0x4d2c_6dfc,
    0x5338_0d13,
    0x650a_7354,
    0x766a_0abb,
    0x81c2_c92e,
    0x9272_2c85,
    0xa2bf_e8a1,
    0xa81a_664b,
    0xc24b_8b70,
    0xc76c_51a3,
    0xd192_e819,
    0xd699_0624,
    0xf40e_3585,
    0x106a_a070,
    0x19a4_c116,
    0x1e37_6c08,
    0x2748_774c,
    0x34b0_bcb5,
    0x391c_0cb3,
    0x4ed8_aa4a,
    0x5b9c_ca4f,
    0x682e_6ff3,
    0x748f_82ee,
    0x78a5_636f,
    0x84c8_7814,
    0x8cc7_0208,
    0x90be_fffa,
    0xa450_6ceb,
    0xbef9_a3f7,
    0xc671_78f2,
];

const START: [u32; 8] = [
    0x6a09_e667,
    0xbb67_ae85,
    0x3c6e_f372,
    0xa54f_f53a,
    0x510e_527f,
    0x9b05_688c,
    0x1f83_d9ab,
    0x5be0_cd19,
];

fn compressed(state: &mut [u32; 8], block: &[u8; 64]) {
    let mut schedule = [0_u32; 64];
    for (slot, bytes) in schedule.iter_mut().zip(block.as_chunks::<4>().0) {
        *slot = u32::from_be_bytes(*bytes);
    }
    for at in 16..64 {
        let early = schedule.get(at - 15).copied().unwrap_or(0);
        let late = schedule.get(at - 2).copied().unwrap_or(0);
        let low = early.rotate_right(7) ^ early.rotate_right(18) ^ (early >> 3);
        let high = late.rotate_right(17) ^ late.rotate_right(19) ^ (late >> 10);
        let sum = schedule
            .get(at - 16)
            .copied()
            .unwrap_or(0)
            .wrapping_add(low)
            .wrapping_add(schedule.get(at - 7).copied().unwrap_or(0))
            .wrapping_add(high);
        if let Some(slot) = schedule.get_mut(at) {
            *slot = sum;
        }
    }
    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
    for (&round, &word) in ROUND.iter().zip(&schedule) {
        let big_e = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let choice = (e & f) ^ (!e & g);
        let first = h
            .wrapping_add(big_e)
            .wrapping_add(choice)
            .wrapping_add(round)
            .wrapping_add(word);
        let big_a = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let majority = (a & b) ^ (a & c) ^ (b & c);
        let second = big_a.wrapping_add(majority);
        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(first);
        d = c;
        c = b;
        b = a;
        a = first.wrapping_add(second);
    }
    for (slot, add) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
        *slot = slot.wrapping_add(add);
    }
}

#[must_use]
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut state = START;
    let (blocks, rest) = bytes.as_chunks::<64>();
    for block in blocks {
        compressed(&mut state, block);
    }
    let mut tail = rest.to_vec();
    tail.push(0x80);
    while tail.len() % 64 != 56 {
        tail.push(0);
    }
    let bits = (bytes.len() as u64).wrapping_mul(8);
    tail.extend_from_slice(&bits.to_be_bytes());
    for block in tail.as_chunks::<64>().0 {
        compressed(&mut state, block);
    }
    let mut out = [0_u8; 32];
    for (place, word) in out.as_chunks_mut::<4>().0.iter_mut().zip(state) {
        *place = word.to_be_bytes();
    }
    out
}
