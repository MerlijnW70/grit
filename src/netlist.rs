use std::fmt;

pub const KIND_BITS: u32 = 8;

pub const SOURCE_BITS: u32 = (32 - KIND_BITS) / 2;

pub const SOURCES: u32 = 1 << SOURCE_BITS;

pub const KINDS: u32 = 1 << KIND_BITS;

pub const INPUTS: u32 = 31;

pub const OUTPUTS: u32 = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gate {
    pub kind: u32,
    pub left: u32,
    pub right: u32,
}

impl Gate {
    #[must_use]
    pub const fn pack(self) -> u32 {
        (self.kind & (KINDS - 1))
            | ((self.left & (SOURCES - 1)) << KIND_BITS)
            | ((self.right & (SOURCES - 1)) << (KIND_BITS + SOURCE_BITS))
    }

    #[must_use]
    pub const fn unpack(word: u32) -> Self {
        Self {
            kind: word & (KINDS - 1),
            left: (word >> KIND_BITS) & (SOURCES - 1),
            right: (word >> (KIND_BITS + SOURCE_BITS)) & (SOURCES - 1),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    Ahead { side: u32, names: u32, below: u32 },
    NoSuchKind { kind: u32, kinds: u32 },
}

impl fmt::Display for Fault {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ahead { side, names, below } => write!(
                out,
                "the {} input names source {names}, which is not below {below}; a gate may only read an input or a gate before it",
                if *side == 0 { "left" } else { "right" }
            ),
            Self::NoSuchKind { kind, kinds } => {
                write!(
                    out,
                    "kind {kind} is not one of the {kinds} kinds in the kit, which are numbered 0 to {}",
                    kinds.saturating_sub(1)
                )
            }
        }
    }
}

impl std::error::Error for Fault {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TooWide {
    Inputs { inputs: u32, limit: u32 },
    Answers { outputs: u32, limit: u32 },
    Steps { steps: u32, limit: u32 },
    Sources { sources: u32, limit: u32 },
    Kinds { kinds: u32, limit: u32 },
    Outputs { outputs: u32, gates: u32 },
}

impl fmt::Display for TooWide {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Inputs { inputs, limit } => write!(
                out,
                "{inputs} inputs asked for; a circuit reads at least one input and at most {limit}, the bits of one row word"
            ),
            Self::Answers { outputs, limit } => write!(
                out,
                "{outputs} outputs asked for, over the {limit} bits one answer word holds"
            ),
            Self::Steps { steps, limit } => write!(
                out,
                "{steps} steps asked for, over the {limit} one run may hold"
            ),
            Self::Sources { sources, limit } => write!(
                out,
                "inputs plus gates is {sources}, over the {limit} a {SOURCE_BITS} bit source field can name"
            ),
            Self::Kinds { kinds, limit } => write!(
                out,
                "{kinds} gate kinds asked for, over the {limit} a {KIND_BITS} bit kind field can name"
            ),
            Self::Outputs { outputs, gates } => {
                write!(
                    out,
                    "{outputs} outputs asked of a circuit holding {gates} gates; every output is a gate, so gates must be at least outputs"
                )
            }
        }
    }
}

impl std::error::Error for TooWide {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Netlist {
    pub inputs: u32,
    pub outputs: u32,
    pub gates: u32,
    pub kinds: u32,
}

impl Netlist {
    pub const fn over(inputs: u32, outputs: u32, gates: u32, kinds: u32) -> Result<Self, TooWide> {
        if inputs == 0 || inputs > INPUTS {
            return Err(TooWide::Inputs {
                inputs,
                limit: INPUTS,
            });
        }
        if outputs > OUTPUTS {
            return Err(TooWide::Answers {
                outputs,
                limit: OUTPUTS,
            });
        }
        let sources = inputs.saturating_add(gates);
        if sources > SOURCES {
            return Err(TooWide::Sources {
                sources,
                limit: SOURCES,
            });
        }
        if kinds > KINDS {
            return Err(TooWide::Kinds {
                kinds,
                limit: KINDS,
            });
        }
        if outputs > gates {
            return Err(TooWide::Outputs { outputs, gates });
        }
        Ok(Self {
            inputs,
            outputs,
            gates,
            kinds,
        })
    }

    #[must_use]
    pub const fn sources(&self) -> u32 {
        self.inputs + self.gates
    }

    #[must_use]
    pub const fn below(&self, at: u32) -> u32 {
        self.inputs + at
    }

    #[must_use]
    pub const fn words(&self) -> u32 {
        1 << self.inputs
    }

    pub const fn checked(&self, at: u32, gate: Gate) -> Result<(), Fault> {
        let below = self.below(at);
        if gate.left >= below {
            return Err(Fault::Ahead {
                side: 0,
                names: gate.left,
                below,
            });
        }
        if gate.right >= below {
            return Err(Fault::Ahead {
                side: 1,
                names: gate.right,
                below,
            });
        }
        if gate.kind >= self.kinds {
            return Err(Fault::NoSuchKind {
                kind: gate.kind,
                kinds: self.kinds,
            });
        }
        Ok(())
    }

    #[must_use]
    pub fn faults(&self, packed: &[u32]) -> Vec<(u32, Fault)> {
        packed
            .iter()
            .take(self.gates as usize)
            .enumerate()
            .filter_map(|(at, word)| {
                let at = u32::try_from(at).unwrap_or(u32::MAX);
                self.checked(at, Gate::unpack(*word))
                    .err()
                    .map(|fault| (at, fault))
            })
            .collect()
    }

    #[must_use]
    pub const fn sizes(&self) -> [usize; 3] {
        [
            self.gates as usize,
            self.outputs as usize,
            self.words() as usize,
        ]
    }
}

#[must_use]
pub const fn drawn(netlist: &Netlist, at: u32, roll: u32) -> Gate {
    let below = netlist.below(at);
    let kind = roll % netlist.kinds;
    let left = (roll / netlist.kinds) % below;
    let right = (roll / (netlist.kinds * below)) % below;
    Gate { kind, left, right }
}

#[must_use]
pub const fn turned(netlist: &Netlist, at: u32, gate: Gate, roll: u32) -> Gate {
    let below = netlist.below(at);
    let rest = roll >> 2;
    match roll & 3 {
        0 => Gate {
            kind: rest % netlist.kinds,
            ..gate
        },
        1 => Gate {
            left: rest % below,
            ..gate
        },
        _ => Gate {
            right: rest % below,
            ..gate
        },
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exam {
    shape: Netlist,
    answers: Vec<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotAnExam {
    Rows { given: usize, wanted: usize },
    Stray { row: u32, holds: u32, outputs: u32 },
}

impl fmt::Display for NotAnExam {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rows { given, wanted } => {
                write!(
                    out,
                    "the table holds {given} rows, but this many inputs needs exactly {wanted}; one row for every combination of the inputs"
                )
            }
            Self::Stray {
                row,
                holds,
                outputs,
            } => write!(
                out,
                "row {row} is {holds:#x}, which sets a bit that no output holds; with {outputs} outputs a row may only name bits 0 to {}",
                outputs.saturating_sub(1)
            ),
        }
    }
}

impl std::error::Error for NotAnExam {}

const fn spare(outputs: u32) -> u32 {
    if outputs >= 32 {
        0
    } else {
        u32::MAX << outputs
    }
}

impl Exam {
    pub fn of(shape: Netlist, answers: Vec<u32>) -> Result<Self, NotAnExam> {
        let wanted = shape.words() as usize;
        if answers.len() != wanted {
            return Err(NotAnExam::Rows {
                given: answers.len(),
                wanted,
            });
        }
        let past = spare(shape.outputs);
        for (row, holds) in answers.iter().enumerate() {
            if holds & past != 0 {
                return Err(NotAnExam::Stray {
                    row: u32::try_from(row).unwrap_or(u32::MAX),
                    holds: *holds,
                    outputs: shape.outputs,
                });
            }
        }
        Ok(Self { shape, answers })
    }

    #[must_use]
    pub const fn shape(&self) -> Netlist {
        self.shape
    }

    #[must_use]
    pub fn asked(&self, word: u32) -> u32 {
        self.answers.get(word as usize).copied().unwrap_or(0)
    }

    #[must_use]
    pub const fn marks(&self) -> u32 {
        self.shape.words() * self.shape.outputs
    }

    #[must_use]
    pub fn scored(&self, got: &[u32]) -> u32 {
        let past = spare(self.shape.outputs);
        self.answers
            .iter()
            .enumerate()
            .map(|(row, wanted)| {
                let held = got.get(row).copied().unwrap_or(!*wanted);
                (!(held ^ wanted) & !past).count_ones()
            })
            .sum()
    }
}

#[must_use]
pub const fn answered(gate: Gate, left: u32, right: u32) -> u32 {
    (gate.kind >> ((left & 1) | ((right & 1) << 1))) & 1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotAChip {
    Gates { given: usize, wanted: usize },
    Gate { at: u32, fault: Fault },
    Outputs { given: usize, wanted: usize },
    NoSuchGate { output: u32, names: u32, gates: u32 },
}

impl fmt::Display for NotAChip {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gates { given, wanted } => {
                write!(
                    out,
                    "the chip holds {given} gates, but {wanted} were asked for"
                )
            }
            Self::Gate { at, fault } => write!(out, "gate {at} is not legal: {fault}"),
            Self::Outputs { given, wanted } => {
                write!(
                    out,
                    "{given} gates are named as outputs, but {wanted} were asked for"
                )
            }
            Self::NoSuchGate {
                output,
                names,
                gates,
            } => write!(
                out,
                "output {output} names gate {names}, but the chip holds {gates} gates, numbered 0 to {}",
                gates.saturating_sub(1)
            ),
        }
    }
}

impl std::error::Error for NotAChip {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chip {
    shape: Netlist,
    gates: Vec<u32>,
    outputs: Vec<u32>,
}

impl Chip {
    pub fn of(shape: Netlist, gates: Vec<u32>, outputs: Vec<u32>) -> Result<Self, NotAChip> {
        if gates.len() != shape.gates as usize {
            return Err(NotAChip::Gates {
                given: gates.len(),
                wanted: shape.gates as usize,
            });
        }
        if outputs.len() != shape.outputs as usize {
            return Err(NotAChip::Outputs {
                given: outputs.len(),
                wanted: shape.outputs as usize,
            });
        }
        if let Some((at, fault)) = shape.faults(&gates).first() {
            return Err(NotAChip::Gate {
                at: *at,
                fault: *fault,
            });
        }
        for (output, names) in outputs.iter().enumerate() {
            if *names >= shape.gates {
                return Err(NotAChip::NoSuchGate {
                    output: u32::try_from(output).unwrap_or(u32::MAX),
                    names: *names,
                    gates: shape.gates,
                });
            }
        }
        Ok(Self {
            shape,
            gates,
            outputs,
        })
    }

    #[must_use]
    pub const fn shape(&self) -> Netlist {
        self.shape
    }

    #[must_use]
    pub fn sourced(&self, word: u32) -> Vec<u32> {
        let mut held = Vec::with_capacity(self.shape.sources() as usize);
        for input in 0..self.shape.inputs {
            held.push((word >> input) & 1);
        }
        for packed in &self.gates {
            let gate = Gate::unpack(*packed);
            let left = held.get(gate.left as usize).copied().unwrap_or(0);
            let right = held.get(gate.right as usize).copied().unwrap_or(0);
            held.push(answered(gate, left, right));
        }
        held
    }

    #[must_use]
    pub fn answers(&self, word: u32) -> u32 {
        let sourced = self.sourced(word);
        self.outputs
            .iter()
            .enumerate()
            .map(|(bit, gate)| {
                let at = self.shape.inputs as usize + *gate as usize;
                let held = sourced.get(at).copied().unwrap_or(0);
                held << bit
            })
            .fold(0, |word, bit| word | bit)
    }

    #[must_use]
    pub fn table(&self) -> Vec<u32> {
        (0..self.shape.words())
            .map(|word| self.answers(word))
            .collect()
    }
}
