use std::fmt;

use crate::ops::Ops;
use crate::rule3::{CORNERS, Declared, Oct, OneStep, Planed, Ruled, Tally};
use crate::wire::{self, Json};

pub mod adjoint;
pub mod dag;
pub mod expand;
pub mod learn;
pub mod limits;
pub mod parse;
pub mod quiet;
pub mod trace;
pub mod value;

pub use dag::{Dag, Node};
pub use expand::Places;
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
    ValueDepth,
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
    Lifting,
    HaloRadius,
    HaloConstant,
    HaloPlane,
    HaloDeclared,
    HaloWritten,
}

impl Code {
    pub const ALL: [Self; 33] = [
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
        Self::ValueDepth,
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
        Self::Lifting,
        Self::HaloRadius,
        Self::HaloConstant,
        Self::HaloPlane,
        Self::HaloDeclared,
        Self::HaloWritten,
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
            Self::ValueDepth => "value_depth",
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
            Self::Lifting => "lifting",
            Self::HaloRadius => "halo_radius",
            Self::HaloConstant => "halo_constant",
            Self::HaloPlane => "halo_plane",
            Self::HaloDeclared => "halo_declared",
            Self::HaloWritten => "halo_written",
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
    reversible: bool,
    dag: Dag,
    places: Places,
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
        let places = Places::of(json, &expanded.sites);
        if program.reversible && program.halo > 0 {
            return Err(Refusal {
                code: Code::Unsupported,
                path: String::from("$.reversible"),
                expected: String::from(
                    "a reversible law that does not look: the inverse of a look is not built",
                ),
                found: String::from("reversible true and halo 1"),
                limit: None,
            });
        }
        if program.reversible {
            adjoint::lifted(&expanded.dag)
                .map_err(|broken| broken.refusal(&expanded.dag, &places))?;
        }
        let hash = crate::branch::sha256(canonical.as_bytes());
        Ok(Self {
            name: program.name.to_owned(),
            channels: program.channels,
            halo: program.halo,
            reversible: program.reversible,
            dag: expanded.dag,
            places,
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
    pub const fn reversible(&self) -> bool {
        self.reversible
    }

    #[must_use]
    pub const fn dag(&self) -> &Dag {
        &self.dag
    }

    #[must_use]
    pub const fn places(&self) -> &Places {
        &self.places
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

    fn planes(&self) -> usize {
        usize::try_from(self.channels).unwrap_or(1).max(1)
    }

    fn looks(&self) -> crate::halo::Halo {
        self.dag.halo()
    }
}

impl OneStep for Law {
    fn one_step<O: Ops>(&self, ops: &mut O, oct: Oct<'_, O::Word>) -> Ruled<O::Word> {
        self.dag.replay(ops, oct)
    }

    fn one_step_planes<O: Ops>(
        &self,
        ops: &mut O,
        oct: Oct<'_, O::Word>,
        planes: &[[O::Word; CORNERS]],
    ) -> Planed<O::Word> {
        self.dag.replay_planes(ops, oct, planes)
    }

    fn one_step_halo<O: Ops>(
        &self,
        ops: &mut O,
        oct: Oct<'_, O::Word>,
        planes: &[[O::Word; CORNERS]],
        halo: &[[O::Word; crate::halo::BOX]],
    ) -> Planed<O::Word> {
        self.dag.replay_halo(ops, oct, planes, halo)
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
