#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use grit::netlist::{
    Chip, Exam, Fault, Gate, KIND_BITS, KINDS, Netlist, NotAChip, NotAnExam, SOURCE_BITS, SOURCES,
    TooWide, answered, drawn, turned,
};

#[test]
fn the_packing_is_exactly_a_word() {
    assert_eq!(KIND_BITS + SOURCE_BITS * 2, 32);
    assert_eq!(SOURCES, 1 << SOURCE_BITS);
    assert_eq!(KINDS, 1 << KIND_BITS);
}

#[test]
fn a_gate_in_range_packs_and_unpacks_to_itself() {
    let edges = |limit: u32| [0, 1, limit / 2, limit - 2, limit - 1];
    for kind in edges(KINDS) {
        for left in edges(SOURCES) {
            for right in edges(SOURCES) {
                let gate = Gate { kind, left, right };
                assert_eq!(Gate::unpack(gate.pack()), gate);
            }
        }
    }
}

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
    assert_eq!(kind.pack() & left.pack(), 0);
    assert_eq!(kind.pack() & right.pack(), 0);
    assert_eq!(left.pack() & right.pack(), 0);
    assert_eq!(kind.pack() | left.pack() | right.pack(), u32::MAX);
}

#[test]
fn a_shape_past_the_packing_is_refused() {
    assert!(Netlist::over(5, 1, 96, 8).is_ok());

    let sources = Netlist::over(8, 1, SOURCES, 8);
    assert_eq!(
        sources,
        Err(TooWide::Sources {
            sources: SOURCES + 8,
            limit: SOURCES
        })
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

    assert!(Netlist::over(0, 1, SOURCES, KINDS).is_ok());
}

#[test]
fn a_gate_may_only_name_a_source_below_its_own() {
    let held = Netlist::over(5, 1, 96, 8).unwrap();

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
        })
    );

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
        .is_ok()
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
        })
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
        })
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

#[test]
fn the_rule_and_the_check_are_the_same_function() {
    let held = Netlist::over(3, 1, 4, 4).unwrap();
    let mut refused = 0;
    let mut allowed = 0;
    for at in 0..held.gates {
        for kind in 0..8 {
            for left in 0..held.sources() {
                for right in 0..held.sources() {
                    let gate = Gate { kind, left, right };
                    let by_hand =
                        left < held.inputs + at && right < held.inputs + at && kind < held.kinds;
                    assert_eq!(held.checked(at, gate).is_ok(), by_hand);
                    if by_hand {
                        allowed += 1;
                    } else {
                        refused += 1;
                    }
                }
            }
        }
    }
    assert!(allowed > 0 && refused > 0);
}

#[test]
fn following_references_always_reaches_an_input() {
    let held = Netlist::over(5, 1, 96, 8).unwrap();

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
    assert!(held.faults(&packed).is_empty());

    for start in 0..held.gates {
        let mut at = held.below(start);
        let mut steps = 0;
        while at >= held.inputs {
            let gate = Gate::unpack(packed[(at - held.inputs) as usize]);
            assert!(gate.left < at);
            at = gate.left;
            steps += 1;
            assert!(steps <= held.sources());
        }
        assert!(at < held.inputs);
    }
}

#[test]
fn every_fault_is_found_and_placed() {
    let held = Netlist::over(4, 1, 8, 4).unwrap();
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

    assert!(held.faults(&packed).is_empty());

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

#[test]
fn the_sizes_are_the_shape_and_nothing_else() {
    let held = Netlist::over(5, 3, 96, 8).unwrap();
    assert_eq!(held.words(), 32);
    assert_eq!(held.sources(), 101);
    assert_eq!(held.sizes(), [96, 3, 32]);

    let four = Netlist::over(4, 1, 96, 8).unwrap();
    assert_eq!(four.words(), 16);
    assert_eq!(four.sizes(), [96, 1, 16]);
}

#[test]
fn what_they_say_when_they_are_printed() {
    let ahead = Fault::Ahead {
        side: 1,
        names: 12,
        below: 12,
    };
    assert_eq!(
        ahead.to_string(),
        "the right input names source 12, which is not below 12; a gate may only read an input or a gate before it"
    );
    assert_eq!(
        Fault::Ahead {
            side: 0,
            names: 12,
            below: 12
        }
        .to_string(),
        "the left input names source 12, which is not below 12; a gate may only read an input or a gate before it"
    );
    assert_eq!(
        Fault::NoSuchKind { kind: 9, kinds: 4 }.to_string(),
        "kind 9 is not one of the 4 kinds in the kit, which are numbered 0 to 3"
    );
    assert_eq!(
        TooWide::Outputs {
            outputs: 9,
            gates: 8
        }
        .to_string(),
        "9 outputs asked of a circuit holding 8 gates; every output is a gate, so gates must be at least outputs"
    );
}

#[test]
fn a_drawn_gate_is_legal_for_every_roll() {
    let held = Netlist::over(3, 1, 4, 4).unwrap();
    let mut seen = 0_u64;
    for at in 0..held.gates {
        let period = u64::from(held.kinds) * u64::from(held.below(at)) * u64::from(held.below(at));
        let walked = (0..=period)
            .map(|roll| u32::try_from(roll).unwrap_or(u32::MAX))
            .chain((u32::MAX - 65_535)..=u32::MAX)
            .chain((0..4096).map(|step: u32| step.wrapping_mul(1_048_573)));
        for roll in walked {
            let gate = drawn(&held, at, roll);
            assert!(held.checked(at, gate).is_ok());
            assert!(gate.left < held.below(at) && gate.right < held.below(at));
            assert!(gate.kind < held.kinds);
            seen += 1;
        }
    }
    assert!(seen > 100_000);
}

#[test]
fn a_drawn_gate_is_legal_at_five_inputs() {
    let held = Netlist::over(5, 1, 96, 8).unwrap();
    for at in 0..held.gates {
        let walked = (0..20_000_u32)
            .chain((u32::MAX - 20_000)..=u32::MAX)
            .chain((0..20_000).map(|step: u32| step.wrapping_mul(214_749)));
        for roll in walked {
            let gate = drawn(&held, at, roll);
            assert!(held.checked(at, gate).is_ok());
        }
    }
}

#[test]
fn turning_a_gate_moves_one_field_and_keeps_it_legal() {
    let held = Netlist::over(5, 1, 96, 8).unwrap();
    let mut moved = [0_u32; 3];
    for at in 0..held.gates {
        let was = drawn(&held, at, at.wrapping_mul(2_654_435_761));
        for roll in (0..8192_u32).chain((u32::MAX - 4096)..=u32::MAX) {
            let now = turned(&held, at, was, roll);
            assert!(held.checked(at, now).is_ok());
            let differs = u32::from(now.kind != was.kind)
                + u32::from(now.left != was.left)
                + u32::from(now.right != was.right);
            assert!(differs <= 1);
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
    assert!(moved[0] > 0 && moved[1] > 0 && moved[2] > 0);
}

#[test]
fn turning_does_not_repair_what_was_already_wrong() {
    let held = Netlist::over(5, 1, 96, 8).unwrap();
    let bad = Gate {
        kind: 0,
        left: 90,
        right: 0,
    };
    assert!(held.checked(3, bad).is_err());
    let now = turned(&held, 3, bad, 2);
    assert_eq!(now.left, 90);
    assert!(held.checked(3, now).is_err());
}

#[test]
fn a_table_that_is_not_the_shape_is_refused() {
    let five = Netlist::over(5, 1, 96, 8).unwrap();
    assert_eq!(five.words(), 32);

    assert_eq!(
        Exam::of(five, vec![0; 16]),
        Err(NotAnExam::Rows {
            given: 16,
            wanted: 32
        })
    );

    let mut stray = vec![0_u32; 32];
    stray[9] = 0b10;
    assert_eq!(
        Exam::of(five, stray),
        Err(NotAnExam::Stray {
            row: 9,
            holds: 0b10,
            outputs: 1
        })
    );

    assert!(Exam::of(five, vec![1; 32]).is_ok());

    let three = Netlist::over(5, 3, 96, 8).unwrap();
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

#[test]
fn the_exam_of_five_inputs_is_thirty_two_rows() {
    let shape = Netlist::over(5, 1, 96, 8).unwrap();
    let answers: Vec<u32> = (0..32_u32).map(|word| word.count_ones() & 1).collect();
    let exam = Exam::of(shape, answers).unwrap();

    assert_eq!(exam.marks(), 32);
    assert_eq!(exam.shape().words(), 32);
    assert_eq!(exam.asked(0), 0);
    assert_eq!(exam.asked(1), 1);
    assert_eq!(exam.asked(3), 0);
    assert_eq!(exam.asked(31), 1);

    let wide = Netlist::over(5, 3, 96, 8).unwrap();
    let counted: Vec<u32> = (0..32_u32).map(u32::count_ones).collect();
    let count = Exam::of(wide, counted).unwrap();
    assert_eq!(count.marks(), 96);
    assert_eq!(count.asked(31), 5);
}

#[test]
fn a_score_counts_the_bits_that_agree() {
    let shape = Netlist::over(3, 2, 8, 4).unwrap();
    let answers: Vec<u32> = (0..8_u32).map(|word| word & 0b11).collect();
    let exam = Exam::of(shape, answers.clone()).unwrap();
    assert_eq!(exam.marks(), 16);

    assert_eq!(exam.scored(&answers), 16);

    let inverted: Vec<u32> = answers.iter().map(|held| !*held).collect();
    assert_eq!(exam.scored(&inverted), 0);

    let mut nearly = inverted.clone();
    nearly[0] = answers[0] ^ 0b10;
    assert_eq!(exam.scored(&nearly), 1);

    let noisy: Vec<u32> = answers.iter().map(|held| held | 0xffff_fffc).collect();
    assert_eq!(exam.scored(&noisy), 16);

    assert_eq!(exam.scored(&answers[..4]), 8);
    assert_eq!(exam.scored(&[]), 0);
}

#[test]
fn what_a_refused_table_says() {
    assert_eq!(
        NotAnExam::Rows {
            given: 16,
            wanted: 32
        }
        .to_string(),
        "the table holds 16 rows, but this many inputs needs exactly 32; one row for every combination of the inputs"
    );
    assert_eq!(
        NotAnExam::Stray {
            row: 9,
            holds: 0b10,
            outputs: 1
        }
        .to_string(),
        "row 9 is 0x2, which sets a bit that no output holds; with 1 outputs a row may only name bits 0 to 0"
    );
}

#[test]
fn a_kind_is_its_own_truth_table() {
    for kind in 0..16_u32 {
        for right in 0..2_u32 {
            for left in 0..2_u32 {
                let by_hand = (kind >> (left | (right << 1))) & 1;
                assert_eq!(
                    answered(
                        Gate {
                            kind,
                            left: 0,
                            right: 0
                        },
                        left,
                        right
                    ),
                    by_hand
                );
            }
        }
    }
}

#[test]
fn the_gates_that_have_names_answer_what_they_are_called() {
    let of = |kind: u32| {
        let gate = Gate {
            kind,
            left: 0,
            right: 0,
        };
        [
            answered(gate, 0, 0),
            answered(gate, 1, 0),
            answered(gate, 0, 1),
            answered(gate, 1, 1),
        ]
    };
    assert_eq!(of(0b0000), [0, 0, 0, 0]);
    assert_eq!(of(0b1111), [1, 1, 1, 1]);
    assert_eq!(of(0b1000), [0, 0, 0, 1]);
    assert_eq!(of(0b1110), [0, 1, 1, 1]);
    assert_eq!(of(0b0110), [0, 1, 1, 0]);
    assert_eq!(of(0b1001), [1, 0, 0, 1]);
    assert_eq!(of(0b1010), [0, 1, 0, 1]);
    assert_eq!(of(0b1100), [0, 0, 1, 1]);
    assert_eq!(of(0b0101), [1, 0, 1, 0]);

    let eight = Netlist::over(3, 1, 4, 8).unwrap();
    let sat = |kind: u32| {
        eight.checked(
            1,
            Gate {
                kind,
                left: 0,
                right: 1,
            },
        )
    };
    assert!(sat(0b0110).is_ok());
    assert_eq!(sat(0b1000), Err(Fault::NoSuchKind { kind: 8, kinds: 8 }));
}

#[test]
fn a_pair_of_arrays_that_is_not_a_chip_is_refused() {
    let shape = Netlist::over(3, 1, 4, 16).unwrap();
    let plain: Vec<u32> = std::iter::repeat_n(
        Gate {
            kind: 0b0110,
            left: 0,
            right: 1,
        }
        .pack(),
        4,
    )
    .collect();
    assert!(Chip::of(shape, plain.clone(), vec![3]).is_ok());

    assert_eq!(
        Chip::of(shape, vec![0; 3], vec![3]),
        Err(NotAChip::Gates {
            given: 3,
            wanted: 4
        })
    );
    assert_eq!(
        Chip::of(shape, plain.clone(), vec![3, 2]),
        Err(NotAChip::Outputs {
            given: 2,
            wanted: 1
        })
    );
    assert_eq!(
        Chip::of(shape, plain.clone(), vec![4]),
        Err(NotAChip::NoSuchGate {
            output: 0,
            names: 4,
            gates: 4
        })
    );

    let mut ahead = plain;
    ahead[1] = Gate {
        kind: 0,
        left: 0,
        right: 4,
    }
    .pack();
    assert_eq!(
        Chip::of(shape, ahead, vec![3]),
        Err(NotAChip::Gate {
            at: 1,
            fault: Fault::Ahead {
                side: 1,
                names: 4,
                below: 4
            }
        })
    );
}

#[test]
fn the_inputs_are_the_words_bits() {
    let shape = Netlist::over(5, 1, 1, 16).unwrap();
    let chip = Chip::of(
        shape,
        vec![
            Gate {
                kind: 0b1010,
                left: 0,
                right: 0,
            }
            .pack(),
        ],
        vec![0],
    )
    .unwrap();

    assert_eq!(chip.sourced(0)[..5], [0, 0, 0, 0, 0]);
    assert_eq!(chip.sourced(1)[..5], [1, 0, 0, 0, 0]);
    assert_eq!(chip.sourced(0b10101)[..5], [1, 0, 1, 0, 1]);
    assert_eq!(chip.sourced(31)[..5], [1, 1, 1, 1, 1]);
}

#[test]
fn a_hand_built_chip_passes_the_five_input_parity_exam() {
    let shape = Netlist::over(5, 1, 4, 16).unwrap();
    let xor = |left: u32, right: u32| {
        Gate {
            kind: 0b0110,
            left,
            right,
        }
        .pack()
    };
    let chip = Chip::of(
        shape,
        vec![xor(0, 1), xor(5, 2), xor(6, 3), xor(7, 4)],
        vec![3],
    )
    .unwrap();

    let answers: Vec<u32> = (0..32_u32).map(|word| word.count_ones() & 1).collect();
    let exam = Exam::of(shape, answers.clone()).unwrap();

    assert_eq!(chip.table(), answers);
    assert_eq!(exam.marks(), 32);
    assert_eq!(exam.scored(&chip.table()), 32);
}

#[test]
fn a_chip_that_is_wrong_scores_what_it_is_worth() {
    let shape = Netlist::over(5, 1, 4, 16).unwrap();
    let xor = |left: u32, right: u32| {
        Gate {
            kind: 0b0110,
            left,
            right,
        }
        .pack()
    };
    let chip = Chip::of(
        shape,
        vec![xor(0, 1), xor(5, 2), xor(6, 3), xor(7, 7)],
        vec![2],
    )
    .unwrap();

    let answers: Vec<u32> = (0..32_u32).map(|word| word.count_ones() & 1).collect();
    let exam = Exam::of(shape, answers).unwrap();
    assert_eq!(exam.scored(&chip.table()), 16);
}

#[test]
fn a_chip_can_carry_more_than_one_answer() {
    let shape = Netlist::over(2, 2, 3, 16).unwrap();
    let chip = Chip::of(
        shape,
        vec![
            Gate {
                kind: 0b0110,
                left: 0,
                right: 1,
            }
            .pack(),
            Gate {
                kind: 0b1000,
                left: 0,
                right: 1,
            }
            .pack(),
            Gate {
                kind: 0b0000,
                left: 0,
                right: 0,
            }
            .pack(),
        ],
        vec![0, 1],
    )
    .unwrap();

    assert_eq!(chip.table(), vec![0, 1, 1, 2]);

    let exam = Exam::of(shape, vec![0, 1, 1, 2]).unwrap();
    assert_eq!(exam.marks(), 8);
    assert_eq!(exam.scored(&chip.table()), 8);
}

#[test]
fn what_a_refused_chip_says() {
    assert_eq!(
        NotAChip::Gates {
            given: 3,
            wanted: 4
        }
        .to_string(),
        "the chip holds 3 gates, but 4 were asked for"
    );
    assert_eq!(
        NotAChip::NoSuchGate {
            output: 0,
            names: 4,
            gates: 4
        }
        .to_string(),
        "output 0 names gate 4, but the chip holds 4 gates, numbered 0 to 3"
    );
    assert_eq!(
        NotAChip::Gate {
            at: 1,
            fault: Fault::NoSuchKind { kind: 9, kinds: 4 }
        }
        .to_string(),
        "gate 1 is not legal: kind 9 is not one of the 4 kinds in the kit, which are numbered 0 to 3"
    );
    assert_eq!(
        NotAChip::Outputs {
            given: 1,
            wanted: 2
        }
        .to_string(),
        "1 gates are named as outputs, but 2 were asked for"
    );
}

#[test]
fn everything_a_mutation_draws_is_something_a_chip_can_run() {
    let shape = Netlist::over(5, 1, 32, 16).unwrap();
    for roll in (0..512_u32).chain((u32::MAX - 64)..=u32::MAX) {
        let gates: Vec<u32> = (0..shape.gates)
            .map(|at| {
                drawn(
                    &shape,
                    at,
                    roll.wrapping_add(at.wrapping_mul(2_654_435_761)),
                )
                .pack()
            })
            .collect();
        let chip = Chip::of(shape, gates, vec![shape.gates - 1])
            .unwrap_or_else(|why| panic!("{roll} {why}"));
        for row in chip.table() {
            assert!(row <= 1);
        }
    }
}
