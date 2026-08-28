//! A circuit as a flat array, in an order that makes a cycle unsayable.
//!
//! # What this is for
//!
//! Everything else in this crate moves a *grid*: cells that know where they are, and a work list of
//! the ones worth touching. A circuit is the other shape — a gate does not know where it is, it
//! knows what it reads, and what it reads is another gate's answer. Two things follow that the grid
//! never had to face: an ordering problem, because a gate cannot be evaluated before its inputs, and
//! a legality problem, because a reference can point anywhere at all.
//!
//! Both are solved by the same decision, and it is the only real one in this file.
//!
//! # A source is a number, and a gate may only name a smaller one
//!
//! There are `inputs + gates` **sources**. The first `inputs` of them are the exam's input bits, in
//! order. Source `inputs + n` is gate `n`'s answer. A gate may name any source **strictly below its
//! own**, and no other.
//!
//! ```text
//!   sources    0 .. inputs            the input bits
//!              inputs .. inputs+gates gate nought's answer, gate one's, ...
//!
//!   gate n     may name 0 .. inputs+n
//! ```
//!
//! That one rule is worth stating three times over, because it does three jobs:
//!
//! * **A cycle cannot be written down.** Not "is rejected" — cannot be represented. A reference is
//!   below the gate that holds it, so following references only ever goes down, and a walk that only
//!   goes down cannot return.
//! * **There is no scheduling.** Evaluating the array in index order is a topological order by
//!   construction, so one pass over `0..gates` is the whole circuit and no pass has to work out what
//!   is ready.
//! * **A mutation has one bound to respect.** Whatever else a mutation does to a gate, if both its
//!   references stay below the gate's own source number the result is a legal circuit. That is the
//!   whole of the safety, and [`Netlist::checked`] is it written down.
//!
//! # One word a gate, and what that costs
//!
//! A gate is a kind and two references, packed into a single `u32`: [`KIND_BITS`] for the kind and
//! [`SOURCE_BITS`] for each reference. So a netlist is one flat array of words a device can load
//! without a stride, an offset table or a second indirection — and the price is a ceiling on how
//! many sources there can be, which [`Netlist::over`] refuses past rather than wrapping into.
//!
//! The split is not arbitrary and it is not free either way round: bits given to the kind are bits
//! the circuit cannot grow into. What is here is the largest reference field that leaves a byte for
//! the kind.
//!
//! # What is not here yet
//!
//! **No passes, and therefore no numbers.** This file is the structure and its rule, checked on the
//! host. Nothing in it has been dispatched, so nothing in it claims a cost. What a gate pass and a
//! mutation pass are worth belongs to the record that measures them.

use std::fmt;

/// Bits a gate's kind is given.
///
/// A byte, which leaves the two references the rest of the word — see [`SOURCE_BITS`].
pub const KIND_BITS: u32 = 8;

/// Bits each of a gate's two references is given.
///
/// `(32 - KIND_BITS) / 2`. A reference names a source, so this is what bounds
/// [`Netlist::sources`].
pub const SOURCE_BITS: u32 = (32 - KIND_BITS) / 2;

/// The most sources a reference can name, which is the most a netlist can hold.
pub const SOURCES: u32 = 1 << SOURCE_BITS;

/// The most kinds a gate can be.
pub const KINDS: u32 = 1 << KIND_BITS;

/// One gate: what it does, and the two sources it reads.
///
/// The references are **source numbers**, not gate numbers: source `n` below `inputs` is input `n`,
/// and source `inputs + n` is gate `n`'s answer. See the module doc for why they are one space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gate {
    /// Which operation, as an index into whatever kit the caller drew from.
    pub kind: u32,
    /// The source its first input reads.
    pub left: u32,
    /// The source its second input reads.
    pub right: u32,
}

impl Gate {
    /// The gate as one word: kind in the low bits, then the two references.
    ///
    /// **Wrapping is not possible here and not checked.** A `Gate` whose fields are past their
    /// fields' reach cannot be built by [`Gate::unpack`] and is refused by [`Netlist::checked`]
    /// before it reaches a buffer; this is the two of them in the order the shader reads them.
    #[must_use]
    pub const fn pack(self) -> u32 {
        (self.kind & (KINDS - 1))
            | ((self.left & (SOURCES - 1)) << KIND_BITS)
            | ((self.right & (SOURCES - 1)) << (KIND_BITS + SOURCE_BITS))
    }

    /// The word as a gate, which is [`Gate::pack`] the other way round.
    #[must_use]
    pub const fn unpack(word: u32) -> Self {
        Self {
            kind: word & (KINDS - 1),
            left: (word >> KIND_BITS) & (SOURCES - 1),
            right: (word >> (KIND_BITS + SOURCE_BITS)) & (SOURCES - 1),
        }
    }
}

/// Why a gate is not a legal one at the place it sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    /// A reference names a source at or above the gate's own, which is a cycle or a forward read.
    Ahead {
        /// Which of the two, as nought for the left and one for the right.
        side: u32,
        /// The source it named.
        names: u32,
        /// The lowest source it may not name, which is the gate's own.
        below: u32,
    },
    /// The kind is past what the kit holds.
    NoSuchKind {
        /// The kind the gate holds.
        kind: u32,
        /// How many the kit has.
        kinds: u32,
    },
}

impl fmt::Display for Fault {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ahead { side, names, below } => write!(
                out,
                "the {} reference names source {names}, which is not below {below}",
                if *side == 0 { "left" } else { "right" }
            ),
            Self::NoSuchKind { kind, kinds } => {
                write!(out, "kind {kind} in a kit of {kinds}")
            }
        }
    }
}

impl std::error::Error for Fault {}

/// A netlist that does not fit the packing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TooWide {
    /// More sources than a reference field can name.
    Sources {
        /// What was asked for: `inputs + gates`.
        sources: u32,
        /// The most a reference reaches.
        limit: u32,
    },
    /// More kinds than a gate's kind field can hold.
    Kinds {
        /// What was asked for.
        kinds: u32,
        /// The most the field holds.
        limit: u32,
    },
    /// More outputs than there are gates to name.
    Outputs {
        /// What was asked for.
        outputs: u32,
        /// How many gates there are.
        gates: u32,
    },
}

impl fmt::Display for TooWide {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sources { sources, limit } => write!(
                out,
                "{sources} sources needs a wider reference than {SOURCE_BITS} bits; \
                 {limit} is what it reaches"
            ),
            Self::Kinds { kinds, limit } => write!(
                out,
                "{kinds} kinds needs a wider field than {KIND_BITS} bits; {limit} is what it holds"
            ),
            Self::Outputs { outputs, gates } => {
                write!(out, "{outputs} outputs over {gates} gates")
            }
        }
    }
}

impl std::error::Error for TooWide {}

/// How wide a circuit is, how long, and how many answers come off it.
///
/// The sizes a device needs are derived from these three and nothing else — see
/// [`Netlist::sizes`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Netlist {
    /// How many input bits the exam asks about.
    pub inputs: u32,
    /// How many answer bits come off the circuit.
    pub outputs: u32,
    /// How many gates the circuit holds.
    pub gates: u32,
    /// How many kinds the kit it was drawn from holds.
    pub kinds: u32,
}

impl Netlist {
    /// A netlist of this shape, or why the packing does not reach it.
    ///
    /// # Errors
    ///
    /// [`TooWide::Sources`] past what a reference field names, [`TooWide::Kinds`] past what a kind
    /// field holds, and [`TooWide::Outputs`] when more answers are asked for than there are gates
    /// to take them off.
    pub const fn over(inputs: u32, outputs: u32, gates: u32, kinds: u32) -> Result<Self, TooWide> {
        let sources = inputs + gates;
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

    /// How many sources there are: the inputs, then one a gate.
    #[must_use]
    pub const fn sources(&self) -> u32 {
        self.inputs + self.gates
    }

    /// The lowest source gate `at` may **not** name, which is its own.
    #[must_use]
    pub const fn below(&self, at: u32) -> u32 {
        self.inputs + at
    }

    /// How many input words an exam over this many inputs asks about.
    ///
    /// Every combination, which is what makes a truth table a table.
    #[must_use]
    pub const fn words(&self) -> u32 {
        1 << self.inputs
    }

    /// Whether a gate is legal at the place it sits.
    ///
    /// **This is the whole of the rule.** A mutation that keeps this true cannot make a circuit that
    /// has a cycle in it, reads an answer that has not been computed, or names an operation the kit
    /// does not have.
    ///
    /// # Errors
    ///
    /// [`Fault::Ahead`] for a reference at or above the gate's own source, naming which of the two;
    /// [`Fault::NoSuchKind`] for a kind past the kit.
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

    /// Every gate in a packed netlist that is not legal where it sits.
    ///
    /// Reads `self.gates` words and no more; a slice shorter than that is checked as far as it goes,
    /// which is what a caller reading a partial buffer back would want.
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

    /// The words each of the three arrays needs: the gates, the answers' sources, and the exam.
    ///
    /// The exam is one word a row of the truth table, holding the answer bits — which is why it is
    /// [`Netlist::words`] long and not `words × outputs`.
    #[must_use]
    pub const fn sizes(&self) -> [usize; 3] {
        [
            self.gates as usize,
            self.outputs as usize,
            self.words() as usize,
        ]
    }
}

/// A gate drawn at `at` from an arbitrary roll, which is legal whatever the roll is.
///
/// # The whole of the mutation engine's safety
///
/// A search that turns gates over needs somewhere for randomness to enter, and the only question
/// that matters is what happens when the randomness is wrong. Two arrangements were available: draw
/// freely and reject what is illegal, or draw from a space in which nothing illegal can be named.
/// This is the second, and it is not a preference — a rejecting engine has to *decide* what to do
/// with a rejected draw, and every answer to that is a bias nobody measured: draw again and the loop
/// is unbounded, clamp and the top of the range is over-represented, leave the gate alone and a gate
/// near the start of the array is mutated less often than one near the end.
///
/// So the roll is reduced into the legal range instead. Three fields, each taken modulo what it may
/// hold: the kind modulo the kit, and each reference modulo [`Netlist::below`], which is exactly the
/// rule. **[`Netlist::checked`] cannot fail on what this returns**, for any `at` inside the netlist
/// and any roll at all. `tests/netlist.rs` walks three regions of the roll space for it — every
/// value up to the construction's own period, the top of the word, and a stride across the whole of
/// `u32` — because a claim about arithmetic is not made truer by walking four thousand million
/// values, only slower.
///
/// # What it does not claim
///
/// **It is not uniform.** `below` and `kinds` do not divide 2³², so the low values of each field
/// come up slightly more often than the high ones. A draw that has to be uniform needs rejection,
/// which is the thing this exists to avoid, and nothing here has been measured to need it.
///
/// Gate nought is the one place where the reference range is the inputs alone, and the first
/// argument is what makes that fall out rather than be special-cased.
#[must_use]
pub const fn drawn(netlist: &Netlist, at: u32, roll: u32) -> Gate {
    let below = netlist.below(at);
    let kind = roll % netlist.kinds;
    let left = (roll / netlist.kinds) % below;
    let right = (roll / (netlist.kinds * below)) % below;
    Gate { kind, left, right }
}

/// One field of a gate re-drawn, and the other two left where they were.
///
/// **One gate turned is what a generation does**, and turning all three of its fields is a different
/// experiment from turning one. Which field is the low two bits of the roll rather than a fourth
/// random number, so that a caller holding one word holds the whole decision.
///
/// Legal for the same reason [`drawn`] is, and by the same arithmetic: whichever field is re-drawn
/// is reduced into what that field may hold. A gate that was already legal stays legal, and one that
/// was not is not repaired — a mutation is not a check, and [`Netlist::checked`] is where a caller
/// asks that question.
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

/// The answers a circuit is being asked for: one row of the truth table an input word.
///
/// # Why the table is a word a row and not a bit a row
///
/// Because a row's answer is `outputs` bits and the exams this is for have more than one. One word a
/// row holds up to thirty-two answer bits with no packing arithmetic anywhere — a device reads
/// `answers[word]` and has the whole row. At five inputs the whole table is thirty-two words.
///
/// The bit for output `n` is bit `n`. Bits at or above `outputs` are not part of the exam and
/// [`Exam::of`] refuses a table that sets one, because a table with a stray bit in it scores against
/// an answer nobody asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exam {
    /// The shape the table was written for.
    shape: Netlist,
    /// One word a row of the table, in input-word order.
    answers: Vec<u32>,
}

/// Why a table is not an exam for the shape it was handed with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotAnExam {
    /// The table is not one row an input word.
    Rows {
        /// How many rows were given.
        given: usize,
        /// How many the shape asks for.
        wanted: usize,
    },
    /// A row sets a bit past the answers the shape has.
    Stray {
        /// Which row.
        row: u32,
        /// What it held.
        holds: u32,
        /// How many answer bits there are.
        outputs: u32,
    },
}

impl fmt::Display for NotAnExam {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rows { given, wanted } => {
                write!(out, "{given} rows for a table of {wanted}")
            }
            Self::Stray {
                row,
                holds,
                outputs,
            } => write!(
                out,
                "row {row} holds {holds:#x}, which sets a bit past the {outputs} asked for"
            ),
        }
    }
}

impl std::error::Error for NotAnExam {}

/// Bits a word holds above `outputs`, which an exam may not set and a score may not count.
const fn spare(outputs: u32) -> u32 {
    if outputs >= 32 {
        0
    } else {
        u32::MAX << outputs
    }
}

impl Exam {
    /// A table for this shape, or why it is not one.
    ///
    /// # Errors
    ///
    /// [`NotAnExam::Rows`] when the table is not [`Netlist::words`] long, and [`NotAnExam::Stray`]
    /// for the first row that sets a bit past [`Netlist::outputs`].
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

    /// The shape this table was written for.
    #[must_use]
    pub const fn shape(&self) -> Netlist {
        self.shape
    }

    /// The answer bits for one input word.
    #[must_use]
    pub fn asked(&self, word: u32) -> u32 {
        self.answers.get(word as usize).copied().unwrap_or(0)
    }

    /// How many answer bits there are to get right: one an output, an input word.
    ///
    /// This is what a perfect score is, and the number a report divides by.
    #[must_use]
    pub const fn marks(&self) -> u32 {
        self.shape.words() * self.shape.outputs
    }

    /// How many of them a run got right.
    ///
    /// `got` is one word a row, in the same order and with the same bit for each output. **A row the
    /// slice does not reach scores nought** rather than panicking, which is what a caller holding a
    /// buffer that came back short would want — and it is nought and not "all wrong by luck": the
    /// missing row is counted as the complement of what was asked, so no bit of it agrees.
    #[must_use]
    pub fn scored(&self, got: &[u32]) -> u32 {
        let past = spare(self.shape.outputs);
        self.answers
            .iter()
            .enumerate()
            .map(|(row, wanted)| {
                let held = got.get(row).copied().unwrap_or(!*wanted);
                // Ones where the two agree, over the bits the exam asked about and no others.
                (!(held ^ wanted) & !past).count_ones()
            })
            .sum()
    }
}
