//! What the netlist's one rule is worth, checked without a card.
//!
//! `src/netlist.rs` makes a single claim and hangs three properties off it: a gate may only name a
//! source below its own, and therefore a cycle cannot be written down, the array is already in
//! evaluation order, and a mutation has exactly one bound to respect.
//!
//! **A claim of that shape is worth nothing unasserted**, so each of the three is a test here rather
//! than a sentence there. None of them needs a device: the structure is arithmetic, and the passes
//! that will read it are not written yet.
//!
//! What is deliberately *not* here is any number about what it costs. Nothing in `src/netlist.rs`
//! has been dispatched.

// A test may panic — that is how it reports.
#![allow(clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use grit::netlist::{
    Exam, Fault, Gate, KIND_BITS, KINDS, Netlist, NotAnExam, SOURCE_BITS, SOURCES, TooWide, drawn,
    turned,
};

/// The three fields are the whole word, with nothing spare and nothing overlapping.
#[test]
fn the_packing_is_exactly_a_word() {
    assert_eq!(
        KIND_BITS + SOURCE_BITS * 2,
        32,
        "a kind and two references have to be a word between them"
    );
    assert_eq!(SOURCES, 1 << SOURCE_BITS);
    assert_eq!(KINDS, 1 << KIND_BITS);
}

/// Every gate whose fields are inside their fields survives the round trip.
///
/// Walked at the edges rather than everywhere: the two references and the kind are each checked at
/// nought, at one, and at the top of their field, against every combination of the other two's
/// edges. A field that was one bit short or one place over would fail at the top and nowhere else.
#[test]
fn a_gate_in_range_packs_and_unpacks_to_itself() {
    let edges = |limit: u32| [0, 1, limit / 2, limit - 2, limit - 1];
    for kind in edges(KINDS) {
        for left in edges(SOURCES) {
            for right in edges(SOURCES) {
                let gate = Gate { kind, left, right };
                assert_eq!(
                    Gate::unpack(gate.pack()),
                    gate,
                    "kind {kind}, left {left}, right {right} did not survive the word"
                );
            }
        }
    }
}

/// The three fields do not bleed into each other: each one alone moves only its own bits.
#[test]
fn each_field_owns_its_own_bits() {
    let none = Gate {
        kind: 0,
        left: 0,
        right: 0,
    };
    assert_eq!(none.pack(), 0);

    let kind = Gate {
        kind: KINDS - 1,
        ..none
    };
    let left = Gate {
        left: SOURCES - 1,
        ..none
    };
    let right = Gate {
        right: SOURCES - 1,
        ..none
    };
    assert_eq!(kind.pack() & left.pack(), 0, "kind and left share a bit");
    assert_eq!(kind.pack() & right.pack(), 0, "kind and right share a bit");
    assert_eq!(left.pack() & right.pack(), 0, "left and right share a bit");
    assert_eq!(
        kind.pack() | left.pack() | right.pack(),
        u32::MAX,
        "the three fields do not cover the word"
    );
}

/// A netlist of a shape the packing cannot hold is refused, and says which way.
#[test]
fn a_shape_past_the_packing_is_refused() {
    assert!(Netlist::over(5, 1, 96, 8).is_ok(), "a five-input chip fits");

    let sources = Netlist::over(8, 1, SOURCES, 8);
    assert_eq!(
        sources,
        Err(TooWide::Sources {
            sources: SOURCES + 8,
            limit: SOURCES
        }),
        "more sources than a reference names has to be refused"
    );

    let kinds = Netlist::over(4, 1, 16, KINDS + 1);
    assert_eq!(
        kinds,
        Err(TooWide::Kinds {
            kinds: KINDS + 1,
            limit: KINDS
        })
    );

    let outputs = Netlist::over(4, 9, 8, 4);
    assert_eq!(
        outputs,
        Err(TooWide::Outputs {
            outputs: 9,
            gates: 8
        })
    );

    // The largest netlist the packing does reach, so that the refusal above is one past a size that
    // works rather than one past nothing.
    assert!(
        Netlist::over(0, 1, SOURCES, KINDS).is_ok(),
        "the packing has to reach its own limit"
    );
}

/// **The rule.** A reference at or above the gate's own source is refused, on either side.
#[test]
fn a_gate_may_only_name_a_source_below_its_own() {
    let held = Netlist::over(5, 1, 96, 8).expect("netlist");

    // Gate nought sees the five inputs and nothing else.
    assert_eq!(held.below(0), 5);
    assert!(
        held.checked(
            0,
            Gate {
                kind: 0,
                left: 4,
                right: 0
            }
        )
        .is_ok()
    );
    assert_eq!(
        held.checked(
            0,
            Gate {
                kind: 0,
                left: 5,
                right: 0
            }
        ),
        Err(Fault::Ahead {
            side: 0,
            names: 5,
            below: 5
        }),
        "gate nought naming gate nought is a gate reading itself"
    );

    // Gate seven sees the five inputs and gates nought through six.
    assert_eq!(held.below(7), 12);
    assert!(
        held.checked(
            7,
            Gate {
                kind: 7,
                left: 11,
                right: 0
            }
        )
        .is_ok(),
        "the gate one below is the highest it may read"
    );
    assert_eq!(
        held.checked(
            7,
            Gate {
                kind: 0,
                left: 0,
                right: 12
            }
        ),
        Err(Fault::Ahead {
            side: 1,
            names: 12,
            below: 12
        }),
        "reading its own answer is the first thing forbidden"
    );
    assert_eq!(
        held.checked(
            7,
            Gate {
                kind: 0,
                left: 0,
                right: 95
            }
        ),
        Err(Fault::Ahead {
            side: 1,
            names: 95,
            below: 12
        }),
        "and a reference far ahead is the same fault"
    );

    assert_eq!(
        held.checked(
            7,
            Gate {
                kind: 8,
                left: 0,
                right: 0
            }
        ),
        Err(Fault::NoSuchKind { kind: 8, kinds: 8 })
    );
}

/// Every gate of a small netlist, exhaustively, agrees with the rule written out by hand.
///
/// Three inputs and four gates is 7 sources and 4 places, so `4 × 4 × 7 × 7` gates is 784 —
/// small enough to walk all of them and say that `checked` and the sentence in the module doc are
/// the same function.
#[test]
fn the_rule_and_the_check_are_the_same_function() {
    let held = Netlist::over(3, 1, 4, 4).expect("netlist");
    let mut refused = 0;
    let mut allowed = 0;
    for at in 0..held.gates {
        for kind in 0..8 {
            for left in 0..held.sources() {
                for right in 0..held.sources() {
                    let gate = Gate { kind, left, right };
                    let by_hand =
                        left < held.inputs + at && right < held.inputs + at && kind < held.kinds;
                    assert_eq!(
                        held.checked(at, gate).is_ok(),
                        by_hand,
                        "at {at}, kind {kind}, left {left}, right {right}"
                    );
                    if by_hand {
                        allowed += 1;
                    } else {
                        refused += 1;
                    }
                }
            }
        }
    }
    // Both arms have to have been taken, or the loop above proves nothing.
    assert!(
        allowed > 0 && refused > 0,
        "{allowed} allowed, {refused} refused"
    );
}

/// **A cycle cannot be written down.** Following references from any legal gate reaches an input.
///
/// This is the property the ordering is for, and it is checked the only way that means anything:
/// walk every reference from every gate of a legal netlist and demand the walk ends. A netlist that
/// admitted a cycle would not end, so the walk is bounded by the number of sources and the bound
/// failing *is* the failure.
#[test]
fn following_references_always_reaches_an_input() {
    let held = Netlist::over(5, 1, 96, 8).expect("netlist");

    // A netlist where every gate reads the two sources directly below it, which is the densest
    // legal wiring there is: every reference is as high as the rule allows.
    let packed: Vec<u32> = (0..held.gates)
        .map(|at| {
            let below = held.below(at);
            Gate {
                kind: at % held.kinds,
                left: below - 1,
                right: below - 2,
            }
            .pack()
        })
        .collect();
    assert!(
        held.faults(&packed).is_empty(),
        "the densest legal wiring has to be legal"
    );

    for start in 0..held.gates {
        let mut at = held.below(start);
        let mut steps = 0;
        while at >= held.inputs {
            let gate = Gate::unpack(packed[(at - held.inputs) as usize]);
            assert!(
                gate.left < at,
                "a walk that does not go down is a walk that can return"
            );
            at = gate.left;
            steps += 1;
            assert!(
                steps <= held.sources(),
                "the walk from gate {start} has taken {steps} steps over {} sources",
                held.sources()
            );
        }
        assert!(
            at < held.inputs,
            "the walk ended somewhere that is not an input"
        );
    }
}

/// A netlist read back with faults in it names every one of them, and where.
#[test]
fn every_fault_is_found_and_placed() {
    let held = Netlist::over(4, 1, 8, 4).expect("netlist");
    let mut packed: Vec<u32> = (0..held.gates)
        .map(|_| {
            Gate {
                kind: 0,
                left: 0,
                right: 1,
            }
            .pack()
        })
        .collect();

    assert!(
        held.faults(&packed).is_empty(),
        "the plain netlist is legal"
    );

    // Gate two reads gate two, gate five holds a kind the kit does not have.
    packed[2] = Gate {
        kind: 0,
        left: 6,
        right: 0,
    }
    .pack();
    packed[5] = Gate {
        kind: 9,
        left: 0,
        right: 1,
    }
    .pack();

    let found = held.faults(&packed);
    assert_eq!(
        found,
        vec![
            (
                2,
                Fault::Ahead {
                    side: 0,
                    names: 6,
                    below: 6
                }
            ),
            (5, Fault::NoSuchKind { kind: 9, kinds: 4 }),
        ]
    );
}

/// The sizes a device would be asked for come off the three numbers and nothing else.
#[test]
fn the_sizes_are_the_shape_and_nothing_else() {
    let held = Netlist::over(5, 3, 96, 8).expect("netlist");
    assert_eq!(
        held.words(),
        32,
        "five inputs is a table of thirty-two rows"
    );
    assert_eq!(held.sources(), 101);
    assert_eq!(held.sizes(), [96, 3, 32]);

    let four = Netlist::over(4, 1, 96, 8).expect("netlist");
    assert_eq!(four.words(), 16);
    assert_eq!(four.sizes(), [96, 1, 16]);
}

/// A fault and a refusal both say what they are, because they end up in a message someone reads.
#[test]
fn what_they_say_when_they_are_printed() {
    let ahead = Fault::Ahead {
        side: 1,
        names: 12,
        below: 12,
    };
    assert_eq!(
        ahead.to_string(),
        "the right reference names source 12, which is not below 12"
    );
    assert_eq!(
        Fault::NoSuchKind { kind: 9, kinds: 4 }.to_string(),
        "kind 9 in a kit of 4"
    );
    assert_eq!(
        TooWide::Outputs {
            outputs: 9,
            gates: 8
        }
        .to_string(),
        "9 outputs over 8 gates"
    );
}

/// **The safety.** A drawn gate is legal for every roll, at every place, whatever the roll is.
///
/// The construction reduces each field into what that field may hold, so legality does not depend
/// on the roll at all — which is a claim about arithmetic and is checked as one. The rolls walked
/// are every value up to the construction's own period, the top of the word, and a stride across
/// the whole of `u32` so that no region of it is unvisited.
#[test]
fn a_drawn_gate_is_legal_for_every_roll() {
    let held = Netlist::over(3, 1, 4, 4).expect("netlist");
    let mut seen = 0_u64;
    for at in 0..held.gates {
        let period = u64::from(held.kinds) * u64::from(held.below(at)) * u64::from(held.below(at));
        let walked = (0..=period)
            .map(|roll| u32::try_from(roll).unwrap_or(u32::MAX))
            .chain((u32::MAX - 65_535)..=u32::MAX)
            .chain((0..4096).map(|step: u32| step.wrapping_mul(1_048_573)));
        for roll in walked {
            let gate = drawn(&held, at, roll);
            assert!(
                held.checked(at, gate).is_ok(),
                "roll {roll} at {at} drew {gate:?}, which is not legal there"
            );
            assert!(gate.left < held.below(at) && gate.right < held.below(at));
            assert!(gate.kind < held.kinds);
            seen += 1;
        }
    }
    assert!(seen > 100_000, "only {seen} rolls were walked");
}

/// And at the width the exam is for, over the same three regions of the roll space.
#[test]
fn a_drawn_gate_is_legal_at_five_inputs() {
    let held = Netlist::over(5, 1, 96, 8).expect("netlist");
    for at in 0..held.gates {
        let walked = (0..20_000_u32)
            .chain((u32::MAX - 20_000)..=u32::MAX)
            .chain((0..20_000).map(|step: u32| step.wrapping_mul(214_749)));
        for roll in walked {
            let gate = drawn(&held, at, roll);
            assert!(
                held.checked(at, gate).is_ok(),
                "roll {roll} at {at} drew {gate:?}"
            );
        }
    }
}

/// A turned gate is legal, and it turns exactly one field.
///
/// Both halves matter. Legal is the safety; one field is what makes "one gate turned" the
/// experiment it says it is rather than a fresh draw wearing its name.
#[test]
fn turning_a_gate_moves_one_field_and_keeps_it_legal() {
    let held = Netlist::over(5, 1, 96, 8).expect("netlist");
    let mut moved = [0_u32; 3];
    for at in 0..held.gates {
        let was = drawn(&held, at, at.wrapping_mul(2_654_435_761));
        for roll in (0..8192_u32).chain((u32::MAX - 4096)..=u32::MAX) {
            let now = turned(&held, at, was, roll);
            assert!(
                held.checked(at, now).is_ok(),
                "roll {roll} at {at} turned {was:?} into {now:?}"
            );
            let differs = u32::from(now.kind != was.kind)
                + u32::from(now.left != was.left)
                + u32::from(now.right != was.right);
            assert!(
                differs <= 1,
                "{was:?} became {now:?}, which is {differs} fields"
            );
            if now.kind != was.kind {
                moved[0] += 1;
            }
            if now.left != was.left {
                moved[1] += 1;
            }
            if now.right != was.right {
                moved[2] += 1;
            }
        }
    }
    // All three fields have to be reachable, or the mutation cannot search the whole space.
    assert!(
        moved[0] > 0 && moved[1] > 0 && moved[2] > 0,
        "kind moved {}, left {}, right {}",
        moved[0],
        moved[1],
        moved[2]
    );
}

/// A gate that was already illegal is not repaired by turning it, and says so.
#[test]
fn turning_does_not_repair_what_was_already_wrong() {
    let held = Netlist::over(5, 1, 96, 8).expect("netlist");
    let bad = Gate {
        kind: 0,
        left: 90,
        right: 0,
    };
    assert!(held.checked(3, bad).is_err(), "the gate has to start wrong");
    // Roll 2 turns the right reference, so the left one is left where it was.
    let now = turned(&held, 3, bad, 2);
    assert_eq!(now.left, 90, "turning the right must not touch the left");
    assert!(
        held.checked(3, now).is_err(),
        "a mutation is not a check and must not look like one"
    );
}

/// A table that is not the shape's table is refused, and says which way.
#[test]
fn a_table_that_is_not_the_shape_is_refused() {
    let five = Netlist::over(5, 1, 96, 8).expect("netlist");
    assert_eq!(five.words(), 32);

    assert_eq!(
        Exam::of(five, vec![0; 16]),
        Err(NotAnExam::Rows {
            given: 16,
            wanted: 32
        }),
        "a sixteen-row table is a four-input exam"
    );

    let mut stray = vec![0_u32; 32];
    stray[9] = 0b10;
    assert_eq!(
        Exam::of(five, stray),
        Err(NotAnExam::Stray {
            row: 9,
            holds: 0b10,
            outputs: 1
        }),
        "one output means bit nought and no other"
    );

    assert!(Exam::of(five, vec![1; 32]).is_ok());

    // Three outputs give three bits, and the fourth is still refused.
    let three = Netlist::over(5, 3, 96, 8).expect("netlist");
    assert!(Exam::of(three, vec![0b111; 32]).is_ok());
    assert_eq!(
        Exam::of(three, vec![0b1000; 32]),
        Err(NotAnExam::Stray {
            row: 0,
            holds: 0b1000,
            outputs: 3
        })
    );
}

/// **The five-input exam.** Thirty-two rows, and what a perfect score is worth.
#[test]
fn the_exam_of_five_inputs_is_thirty_two_rows() {
    let shape = Netlist::over(5, 1, 96, 8).expect("netlist");
    // The parity of five inputs, written out the way an exam is: one row an input word.
    let answers: Vec<u32> = (0..32_u32).map(|word| word.count_ones() & 1).collect();
    let exam = Exam::of(shape, answers).expect("exam");

    assert_eq!(
        exam.marks(),
        32,
        "one answer bit for each of thirty-two rows"
    );
    assert_eq!(exam.shape().words(), 32);
    assert_eq!(exam.asked(0), 0, "no ones is even");
    assert_eq!(exam.asked(1), 1);
    assert_eq!(exam.asked(3), 0, "two ones is even");
    assert_eq!(exam.asked(31), 1, "five ones is odd");

    // Three answer bits over the same width is three times the marks.
    let wide = Netlist::over(5, 3, 96, 8).expect("netlist");
    let counted: Vec<u32> = (0..32_u32).map(u32::count_ones).collect();
    let count = Exam::of(wide, counted).expect("exam");
    assert_eq!(count.marks(), 96);
    assert_eq!(count.asked(31), 5, "five ones is a hundred and one");
}

/// A score is the bits that agree, over the bits the exam asked about and no others.
#[test]
fn a_score_counts_the_bits_that_agree() {
    let shape = Netlist::over(3, 2, 8, 4).expect("netlist");
    let answers: Vec<u32> = (0..8_u32).map(|word| word & 0b11).collect();
    let exam = Exam::of(shape, answers.clone()).expect("exam");
    assert_eq!(exam.marks(), 16, "eight rows of two answers");

    assert_eq!(
        exam.scored(&answers),
        16,
        "the answers themselves are perfect"
    );

    let inverted: Vec<u32> = answers.iter().map(|held| !*held).collect();
    assert_eq!(
        exam.scored(&inverted),
        0,
        "the complement gets nothing right"
    );

    // A run that is right about one bit of one row and wrong about the rest.
    let mut nearly = inverted.clone();
    nearly[0] = answers[0] ^ 0b10;
    assert_eq!(exam.scored(&nearly), 1);

    // Bits past the two the exam asks about are not counted either way.
    let noisy: Vec<u32> = answers.iter().map(|held| held | 0xffff_fffc).collect();
    assert_eq!(
        exam.scored(&noisy),
        16,
        "a bit nobody asked about must not cost a mark"
    );

    // A buffer that came back short scores nothing for the rows it does not reach.
    assert_eq!(exam.scored(&answers[..4]), 8);
    assert_eq!(exam.scored(&[]), 0);
}

/// What the exam's refusals say when they are printed.
#[test]
fn what_a_refused_table_says() {
    assert_eq!(
        NotAnExam::Rows {
            given: 16,
            wanted: 32
        }
        .to_string(),
        "16 rows for a table of 32"
    );
    assert_eq!(
        NotAnExam::Stray {
            row: 9,
            holds: 0b10,
            outputs: 1
        }
        .to_string(),
        "row 9 holds 0x2, which sets a bit past the 1 asked for"
    );
}
