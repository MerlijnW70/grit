#![allow(clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use grit::netlist::{
    Chip, Exam, Gate, KIND_BITS, KINDS, Netlist, NotAChip, NotAnExam, SOURCE_BITS, SOURCES,
    answered, drawn, turned,
};
use grit::sand::{
    AIR, Grid, HELD, PATIENCE, RUNNING_LEFT, RUNNING_RIGHT, SAND, TIRED, WALL, WATER, WEIGHTS,
    held_of, hold_of, material_of, running_of, settle_reference, tired_of,
};
use grit::sand::{View, brush, paint, settle};
use simdr::decode;
use simdr::module::op;

fn netlist() -> Netlist {
    Netlist::over(4, 2, 8, 4).expect("a netlist")
}

#[test]
fn a_gate_packed_and_unpacked_is_the_gate_it_was() {
    for kind in 0..4 {
        for left in [0, 1, 7, 15] {
            for right in [0, 2, 9, 15] {
                let one = Gate { kind, left, right };
                let back = Gate::unpack(one.pack());
                assert_eq!(back.kind, kind, "kind lost in {one:?}");
                assert_eq!(back.left, left, "left lost in {one:?}");
                assert_eq!(back.right, right, "right lost in {one:?}");
            }
        }
    }
}

#[test]
fn a_gates_three_fields_occupy_three_different_places_in_the_word() {
    let kind = Gate {
        kind: 1,
        left: 0,
        right: 0,
    }
    .pack();
    let left = Gate {
        kind: 0,
        left: 1,
        right: 0,
    }
    .pack();
    let right = Gate {
        kind: 0,
        left: 0,
        right: 1,
    }
    .pack();

    assert_ne!(kind, left);
    assert_ne!(left, right);
    assert_ne!(kind, right);
    assert_eq!(
        Gate {
            kind: 0,
            left: 0,
            right: 0
        }
        .pack(),
        0
    );
}

#[test]
fn a_netlists_sources_are_its_inputs_and_its_gates_together() {
    let one = netlist();
    assert_eq!(one.sources(), one.inputs + one.gates);
    assert_eq!(one.sources(), 12);
}

#[test]
fn a_gate_may_read_everything_named_before_it_and_nothing_after() {
    let one = netlist();

    assert_eq!(one.below(0), one.inputs, "the first gate sees the inputs");
    assert_eq!(one.below(3), one.inputs + 3);
    assert_eq!(one.below(one.gates - 1), one.sources() - 1);
}

#[test]
fn a_netlist_of_n_inputs_has_two_to_the_n_words_of_truth_table() {
    for inputs in 1..=8 {
        let one = Netlist::over(inputs, 1, 1, 2).expect("a netlist");
        assert_eq!(one.words(), 1 << inputs, "at {inputs} input(s)");
    }
}

#[test]
fn a_gate_reaching_forward_is_a_fault_and_one_reaching_back_is_not() {
    let one = netlist();
    let backward = Gate {
        kind: 0,
        left: 0,
        right: 1,
    };
    assert!(one.checked(2, backward).is_ok());

    let forward = Gate {
        kind: 0,
        left: one.below(2),
        right: 0,
    };
    assert!(
        one.checked(2, forward).is_err(),
        "a gate read a source that does not exist yet"
    );

    let forward_right = Gate {
        kind: 0,
        left: 0,
        right: one.below(2),
    };
    assert!(
        one.checked(2, forward_right).is_err(),
        "the right side is unchecked"
    );
}

#[test]
fn faults_names_every_gate_that_is_wrong_and_no_gate_that_is_right() {
    let one = netlist();
    let good = Gate {
        kind: 0,
        left: 0,
        right: 1,
    }
    .pack();
    let bad = Gate {
        kind: 0,
        left: 11,
        right: 0,
    }
    .pack();

    let all_good: Vec<u32> = (0..one.gates).map(|_| good).collect();
    assert!(one.faults(&all_good).is_empty());

    let mut mixed = all_good.clone();
    mixed[0] = bad;
    let found = one.faults(&mixed);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].0, 0, "the wrong gate is not the one named");
}

#[test]
fn every_size_a_netlist_asks_for_is_room_for_something() {
    let one = netlist();
    for (at, size) in one.sizes().iter().enumerate() {
        assert!(*size > 0, "netlist buffer {at} sized nought");
    }
}

#[test]
fn a_drawn_gate_is_legal_where_it_is_drawn() {
    let one = netlist();
    for at in 0..one.gates {
        for roll in [0_u32, 1, 7, 12345, u32::MAX] {
            let gate = drawn(&one, at, roll);
            assert!(
                one.checked(at, gate).is_ok(),
                "gate {gate:?} drawn at {at} from {roll} reads ahead of itself"
            );
        }
    }
}

#[test]
fn a_turned_gate_is_still_legal_and_is_not_always_the_gate_it_was() {
    let one = netlist();
    let start = drawn(&one, 5, 99);
    let mut moved = 0;

    for roll in 0..64_u32 {
        let after = turned(&one, 5, start, roll);
        assert!(
            one.checked(5, after).is_ok(),
            "turning produced an illegal gate: {after:?}"
        );
        if after.pack() != start.pack() {
            moved += 1;
        }
    }

    assert!(moved > 0, "turning never changed anything");
}

#[test]
fn an_exam_asks_each_word_of_its_own_truth_table() {
    let shape = Netlist::over(3, 1, 4, 4).expect("a netlist");
    let answers: Vec<u32> = (0..shape.words()).map(|one| one % 2).collect();
    let exam = Exam::of(shape, answers).expect("an exam");

    assert_eq!(exam.shape().inputs, 3);
    for word in 0..shape.words() {
        let _ = exam.asked(word);
    }
}

#[test]
fn an_exam_scores_a_perfect_answer_above_a_wrong_one() {
    let shape = Netlist::over(3, 1, 4, 4).expect("a netlist");
    let answers: Vec<u32> = (0..shape.words()).map(|one| one % 2).collect();
    let exam = Exam::of(shape, answers.clone()).expect("an exam");

    let perfect = exam.scored(&answers);
    let wrong: Vec<u32> = answers.iter().map(|one| one ^ 1).collect();
    let worst = exam.scored(&wrong);

    assert!(
        perfect > worst,
        "a right answer scored {perfect} and a wrong one {worst}"
    );
}

#[test]
fn a_cell_word_carries_four_fields_that_do_not_read_each_other() {
    assert_eq!(material_of(0), 0);

    let sand = 3_u32;
    assert_eq!(material_of(sand), sand, "the material is the low bits");

    let running = running_of(sand);
    assert_eq!(running, 0, "a bare material carries nothing running");
}

#[test]
fn the_material_is_the_low_field_and_the_rest_of_the_word_does_not_reach_it() {
    for material in 0..4_u32 {
        for high in [0_u32, 0xFF00, 0x00FF_0000, 0xFFFF_0000] {
            assert_eq!(
                material_of(material | high),
                material_of(material_of(material) | high),
                "the high bits reached the material"
            );
        }
    }
}

#[test]
fn holding_and_tiredness_are_read_from_different_places() {
    let mut differed = false;
    for word in (0..4096_u32).step_by(7) {
        if held_of(word) != tired_of(word) {
            differed = true;
            break;
        }
    }
    assert!(differed, "held and tired read the same field");
}

#[test]
fn water_over_water_carries_more_than_water_over_anything_else() {
    let water = 1_u32;
    let sand = 2_u32;

    let over_water = hold_of(water, water);
    let over_sand = hold_of(water, sand);

    assert!(
        over_water > over_sand,
        "water over water carried {over_water} and over sand {over_sand}"
    );
}

#[test]
fn a_grid_is_as_many_cells_as_it_is_wide_and_deep() {
    for (wide, deep) in [(8_u32, 8_u32), (64, 32), (128, 128)] {
        let one = Grid::of(wide, deep);
        assert_eq!(one.wide, wide);
        assert_eq!(one.deep, deep);
        assert_eq!(one.cells(), wide as usize * deep as usize);
        assert!(one.quads() > 0);
        assert!(one.across() > 0 && one.down() > 0);
    }
}

#[test]
fn a_wider_grid_holds_more_cells_and_a_deeper_one_does_too() {
    let base = Grid::of(64, 64).cells();
    assert!(Grid::of(128, 64).cells() > base);
    assert!(Grid::of(64, 128).cells() > base);
}

#[test]
fn the_netlists_derived_constants_are_the_numbers_they_derive_to() {
    assert_eq!(KIND_BITS, 8);
    assert_eq!(SOURCE_BITS, (32 - KIND_BITS) / 2);
    assert_eq!(SOURCE_BITS, 12);
    assert_eq!(SOURCES, 1 << SOURCE_BITS);
    assert_eq!(SOURCES, 4096);
    assert_eq!(KINDS, 1 << KIND_BITS);
    assert_eq!(KINDS, 256);
    assert_eq!(
        KIND_BITS + SOURCE_BITS * 2,
        32,
        "the three fields no longer fill a word exactly"
    );
}

#[test]
fn a_netlist_takes_every_source_up_to_the_limit_and_refuses_the_one_past_it() {
    assert!(
        Netlist::over(SOURCES - 1, 1, 1, 4).is_ok(),
        "the limit fits"
    );
    assert!(
        Netlist::over(SOURCES, 1, 1, 4).is_err(),
        "one past the limit was accepted"
    );
    assert!(
        Netlist::over(SOURCES / 2, 1, SOURCES / 2, 4).is_ok(),
        "inputs and gates summing exactly to the limit is the limit, and fits"
    );
    assert!(
        Netlist::over(SOURCES / 2, 1, SOURCES / 2 + 1, 4).is_err(),
        "inputs and gates together are what the limit bounds"
    );
}

#[test]
fn a_fault_says_which_side_of_the_gate_was_wrong() {
    let one = netlist();
    let left_wrong = Gate {
        kind: 0,
        left: one.below(1),
        right: 0,
    };
    let right_wrong = Gate {
        kind: 0,
        left: 0,
        right: one.below(1),
    };

    let left = one.checked(1, left_wrong).expect_err("a fault");
    let right = one.checked(1, right_wrong).expect_err("a fault");

    assert!(
        left.to_string().contains("left"),
        "the left fault says: {left}"
    );
    assert!(
        right.to_string().contains("right"),
        "the right fault says: {right}"
    );
    assert_ne!(left.to_string(), right.to_string());
}

#[test]
fn a_kind_outside_the_kit_is_a_fault_that_names_both_numbers() {
    let one = Netlist::over(4, 1, 4, 4).expect("a netlist");
    let beyond = Gate {
        kind: 9,
        left: 0,
        right: 1,
    };

    let fault = one.checked(1, beyond).expect_err("a kind fault");
    let said = fault.to_string();
    assert!(
        said.contains('9'),
        "the fault does not name the kind: {said}"
    );
    assert!(said.contains('4'), "nor the kit: {said}");
}

#[test]
fn answering_a_gate_depends_on_its_kind_and_on_both_its_inputs() {
    let mut seen = std::collections::BTreeSet::new();
    for kind in 0..4_u32 {
        let gate = Gate {
            kind,
            left: 0,
            right: 1,
        };
        seen.insert((
            answered(gate, 0, 0),
            answered(gate, 0, 1),
            answered(gate, 1, 0),
            answered(gate, 1, 1),
        ));
    }
    assert!(
        seen.len() > 1,
        "every kind answers the same truth table: {seen:?}"
    );

    let listening = (0..4_u32).filter(|kind| {
        let gate = Gate {
            kind: *kind,
            left: 0,
            right: 1,
        };
        let rows = [
            answered(gate, 0, 0),
            answered(gate, 0, 1),
            answered(gate, 1, 0),
            answered(gate, 1, 1),
        ];
        rows.iter().any(|one| *one != rows[0])
    });
    assert!(
        listening.count() > 0,
        "no kind in the kit answers differently for different inputs"
    );
}

#[test]
fn the_four_materials_are_four_different_numbers_in_weight_order() {
    assert_eq!(AIR, 0);
    assert_eq!(WATER, 1);
    assert_eq!(SAND, 2);
    assert_eq!(WALL, 3);
    assert_eq!(WEIGHTS, [AIR, WATER, SAND, WALL]);
    let order = WEIGHTS;
    assert!(
        order.windows(2).all(|pair| pair[0] < pair[1]),
        "the materials are not in weight order: {order:?}"
    );
}

#[test]
fn running_water_is_water_with_a_direction_and_not_another_material() {
    assert_eq!(RUNNING_LEFT, WATER + 4);
    assert_eq!(RUNNING_RIGHT, WATER + 8);
    assert_ne!(RUNNING_LEFT, RUNNING_RIGHT);
    let flags = [RUNNING_LEFT, RUNNING_RIGHT];
    assert!(
        flags.iter().all(|one| *one > WEIGHTS[3]),
        "a direction must not read as a material: {flags:?}"
    );
    assert!(flags[1] > flags[0]);
}

#[test]
fn the_carried_fields_sit_above_one_another_and_do_not_overlap() {
    assert_eq!(TIRED, HELD << 1, "tired is the bit above held");
    assert_eq!(TIRED, HELD * 2);
    let carry = [HELD, TIRED];
    assert!(
        carry.iter().all(|one| *one > WEIGHTS[3]),
        "the carry begins at or below the material: {carry:?}"
    );
    assert_eq!(
        PATIENCE & (PATIENCE + 1),
        0,
        "patience is not a run of ones: {PATIENCE}"
    );
}

#[test]
fn each_accessor_reads_its_own_field_out_of_a_word_built_by_hand() {
    let word = SAND | (WATER << 2) | HELD | TIRED;

    assert_eq!(material_of(word), SAND, "material");
    assert_eq!(running_of(word), WATER, "running");
    assert_eq!(held_of(word), 1, "held");
    assert_eq!(tired_of(word), 1, "tired");

    let bare = SAND;
    assert_eq!(material_of(bare), SAND);
    assert_eq!(running_of(bare), 0);
    assert_eq!(held_of(bare), 0);
    assert_eq!(tired_of(bare), 0);
}

#[test]
fn a_field_set_to_its_widest_does_not_spill_into_the_one_above_it() {
    let word = WALL | (WALL << 2);
    assert_eq!(material_of(word), WALL);
    assert_eq!(running_of(word), WALL);
    assert_eq!(held_of(word), 0, "a full material set the held bit");
    assert_eq!(tired_of(word), 0, "a full material made the cell tired");
}

#[test]
fn holding_grows_with_tiredness_and_a_worn_cell_carries_more() {
    let fresh = hold_of(WATER, SAND);
    let worn = hold_of(WATER | TIRED, SAND);

    assert!(
        worn > fresh,
        "a tired cell carried {worn} and a fresh one {fresh}"
    );
}

#[test]
fn a_grids_quads_are_its_halves_multiplied_and_not_its_sides() {
    let one = Grid::of(8, 8);
    assert_eq!(one.across(), 4);
    assert_eq!(one.down(), 4);
    assert_eq!(one.quads(), 16);
    assert_eq!(one.cells(), 64);

    let wide = Grid::of(16, 8);
    assert_eq!(wide.across(), 8);
    assert_eq!(wide.down(), 4);
    assert_eq!(wide.quads(), 32);
    assert_eq!(wide.cells(), 128);
}

#[test]
fn holding_is_the_number_it_is_for_each_pair_of_materials() {
    assert_eq!(HELD, 16);
    assert_eq!(TIRED, 32);
    assert_eq!(hold_of(WATER, WATER), 17);
    assert_eq!(hold_of(WATER, SAND), 1);
    assert_eq!(hold_of(SAND, WATER), 2);
    assert_eq!(hold_of(SAND, SAND), 2);
    assert_eq!(hold_of(WATER | TIRED, WATER), 49);
}

#[test]
fn the_reference_settle_moves_the_cells_it_moves_and_leaves_the_rest() {
    let grid = Grid::of(8, 8);
    let cells: Vec<u32> = (0..grid.cells()).map(|one| (one % 4) as u32 & 3).collect();

    let first = settle_reference(grid, 0, &cells);

    assert_eq!(first.len(), cells.len());
    assert_eq!(
        &first[..16],
        [0, 0, 2, 3, 0, 0, 2, 3, 1, 1, 2, 3, 1, 1, 2, 3]
    );
    assert_eq!(first.iter().map(|one| u64::from(*one)).sum::<u64>(), 96);
}

#[test]
fn the_two_phases_of_a_settle_are_offset_from_one_another() {
    let grid = Grid::of(8, 8);
    let cells: Vec<u32> = (0..grid.cells()).map(|one| (one % 4) as u32 & 3).collect();

    let even = settle_reference(grid, 0, &cells);
    let odd = settle_reference(grid, 1, &cells);

    assert_eq!(
        &odd[..16],
        [0, 2, 2, 3, 0, 2, 2, 3, 0, 17, 1, 3, 0, 17, 1, 3]
    );
    assert_eq!(odd.iter().map(|one| u64::from(*one)).sum::<u64>(), 224);
    assert_ne!(even, odd, "the phase does not reach the neighbourhood");
}

#[test]
fn a_gate_answers_by_the_low_bit_of_each_input_and_ignores_the_rest() {
    for (kind, wanted) in [
        (0b1010_u32, [0, 0, 0, 0, 1, 1, 1, 1, 0, 0, 0, 0, 1, 1, 1, 1]),
        (0b0110, [0, 1, 0, 1, 1, 0, 1, 0, 0, 1, 0, 1, 1, 0, 1, 0]),
        (0b1100, [0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1]),
    ] {
        let gate = Gate {
            kind,
            left: 0,
            right: 1,
        };
        let mut got = Vec::new();
        for left in 0..4_u32 {
            for right in 0..4_u32 {
                got.push(answered(gate, left, right));
            }
        }
        assert_eq!(got, wanted, "kind {kind:04b}");
    }
}

#[test]
fn held_water_beside_air_starts_running_and_carries_its_tiredness_with_it() {
    let grid = Grid::of(8, 8);
    let mut cells = vec![AIR; grid.cells()];
    for row in 0..4_usize {
        cells[row * 8] = WATER | HELD;
    }

    let even = settle_reference(grid, 0, &cells);
    let odd = settle_reference(grid, 1, &cells);

    assert_eq!(
        &even[..16],
        [0, 0, 0, 0, 0, 0, 0, 0, 1, 57, 0, 0, 0, 0, 0, 0]
    );
    assert_eq!(
        &odd[..16],
        [0, 0, 0, 0, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 0]
    );
    assert_eq!(even.iter().map(|one| u64::from(*one)).sum::<u64>(), 116);
    assert_eq!(odd.iter().map(|one| u64::from(*one)).sum::<u64>(), 108);
}

#[test]
fn a_netlist_takes_a_kit_of_every_size_up_to_the_limit_and_refuses_the_one_past_it() {
    assert!(Netlist::over(4, 1, 4, KINDS).is_ok(), "the kit limit fits");
    assert!(
        Netlist::over(4, 1, 4, KINDS + 1).is_err(),
        "a kit past the limit was accepted"
    );
}

#[test]
fn a_netlist_with_more_outputs_than_gates_has_nothing_to_read_them_from() {
    assert!(Netlist::over(4, 4, 4, 4).is_ok(), "one output a gate fits");
    assert!(
        Netlist::over(4, 5, 4, 4).is_err(),
        "more outputs than gates was accepted"
    );
}

#[test]
fn a_gate_of_the_last_kind_in_the_kit_is_legal_and_the_one_after_it_is_not() {
    let one = Netlist::over(4, 1, 4, 4).expect("a netlist");
    let last = Gate {
        kind: one.kinds - 1,
        left: 0,
        right: 1,
    };
    let past = Gate {
        kind: one.kinds,
        left: 0,
        right: 1,
    };

    assert!(one.checked(1, last).is_ok(), "the last kind was refused");
    assert!(one.checked(1, past).is_err(), "a kind past the kit passed");
}

#[test]
fn an_exam_refuses_a_table_of_the_wrong_height() {
    let shape = Netlist::over(3, 1, 4, 4).expect("a netlist");
    let wanted = shape.words() as usize;

    assert!(Exam::of(shape, vec![0; wanted]).is_ok());
    assert_eq!(
        Exam::of(shape, vec![0; wanted - 1]),
        Err(NotAnExam::Rows {
            given: wanted - 1,
            wanted
        })
    );
    assert_eq!(
        Exam::of(shape, vec![0; wanted + 1]),
        Err(NotAnExam::Rows {
            given: wanted + 1,
            wanted
        })
    );
}

#[test]
fn an_exam_refuses_an_answer_with_a_bit_above_the_outputs_it_has() {
    let shape = Netlist::over(3, 2, 4, 4).expect("a netlist");
    let wanted = shape.words() as usize;

    let mut answers = vec![0_u32; wanted];
    answers[1] = 0b11;
    assert!(
        Exam::of(shape, answers).is_ok(),
        "two outputs hold two bits"
    );

    let mut stray = vec![0_u32; wanted];
    stray[1] = 0b100;
    assert_eq!(
        Exam::of(shape, stray),
        Err(NotAnExam::Stray {
            row: 1,
            holds: 0b100,
            outputs: 2
        })
    );
}

#[test]
fn an_exam_is_worth_one_mark_a_word_a_output() {
    for (inputs, outputs) in [(1_u32, 1_u32), (3, 2), (5, 4)] {
        let shape = Netlist::over(inputs, outputs, outputs, 4).expect("a netlist");
        let exam = Exam::of(shape, vec![0; shape.words() as usize]).expect("an exam");
        assert_eq!(
            exam.marks(),
            shape.words() * outputs,
            "at {inputs} input(s) and {outputs} output(s)"
        );
    }
}

#[test]
fn a_chip_refuses_a_bag_of_gates_that_is_not_its_own_size() {
    let shape = Netlist::over(4, 1, 4, 4).expect("a netlist");
    let good = Gate {
        kind: 0,
        left: 0,
        right: 1,
    }
    .pack();
    let gates = vec![good; shape.gates as usize];

    assert!(Chip::of(shape, gates.clone(), vec![0]).is_ok());
    assert_eq!(
        Chip::of(shape, vec![good; 3], vec![0]),
        Err(NotAChip::Gates {
            given: 3,
            wanted: 4
        })
    );
    assert_eq!(
        Chip::of(shape, vec![good; 5], vec![0]),
        Err(NotAChip::Gates {
            given: 5,
            wanted: 4
        })
    );
}

#[test]
fn turning_reads_the_roll_two_bits_at_a_time_and_the_rest_after() {
    let one = netlist();
    let start = drawn(&one, 5, 99);

    assert_eq!(
        turned(&one, 5, start, 7).pack(),
        turned(&one, 5, start, 7).pack(),
        "turning is not a function of its roll"
    );

    let reached: std::collections::BTreeSet<u32> = (0..256_u32)
        .map(|roll| turned(&one, 5, start, roll).pack())
        .collect();
    assert!(
        reached.len() > 3,
        "the roll reaches only {} gate(s) from one start",
        reached.len()
    );
}

fn wire_through() -> Gate {
    Gate {
        kind: 0b1010,
        left: 0,
        right: 0,
    }
}

#[test]
fn a_chip_reads_each_input_from_its_own_bit_of_the_word() {
    let shape = Netlist::over(2, 1, 1, KINDS).expect("a netlist");
    let chip = Chip::of(shape, vec![wire_through().pack()], vec![0]).expect("a chip");

    assert_eq!(chip.sourced(0b00)[..2], [0, 0]);
    assert_eq!(chip.sourced(0b01)[..2], [1, 0], "bit 0 is input 0");
    assert_eq!(chip.sourced(0b10)[..2], [0, 1], "bit 1 is input 1");
    assert_eq!(chip.sourced(0b11)[..2], [1, 1]);
}

#[test]
fn a_chip_answers_from_the_gate_an_output_names_and_puts_it_at_that_outputs_bit() {
    let shape = Netlist::over(2, 2, 2, KINDS).expect("a netlist");
    let first = wire_through();
    let second = Gate {
        kind: 0b1100,
        left: 1,
        right: 1,
    };
    let chip = Chip::of(shape, vec![first.pack(), second.pack()], vec![0, 1]).expect("a chip");

    for word in 0..4_u32 {
        let sourced = chip.sourced(word);
        let wanted = sourced[2] | (sourced[3] << 1);
        assert_eq!(
            chip.answers(word),
            wanted,
            "word {word:02b}: sources {sourced:?}"
        );
    }

    let swapped = Chip::of(shape, vec![first.pack(), second.pack()], vec![1, 0]).expect("a chip");
    let differs = (0..4_u32).any(|word| swapped.answers(word) != chip.answers(word));
    assert!(
        differs,
        "naming the gates the other way round changed nothing"
    );
}

#[test]
fn a_chip_refuses_a_bag_of_outputs_that_is_not_its_own_size() {
    let shape = Netlist::over(4, 2, 4, KINDS).expect("a netlist");
    let gates = vec![wire_through().pack(); 4];

    assert!(Chip::of(shape, gates.clone(), vec![0, 1]).is_ok());
    assert_eq!(
        Chip::of(shape, gates.clone(), vec![0]),
        Err(NotAChip::Outputs {
            given: 1,
            wanted: 2
        })
    );
    assert_eq!(
        Chip::of(shape, gates, vec![0, 1, 2]),
        Err(NotAChip::Outputs {
            given: 3,
            wanted: 2
        })
    );
}

#[test]
fn a_chip_refuses_an_output_naming_a_gate_it_does_not_have() {
    let shape = Netlist::over(4, 1, 4, KINDS).expect("a netlist");
    let gates = vec![wire_through().pack(); 4];

    assert!(
        Chip::of(shape, gates.clone(), vec![shape.gates - 1]).is_ok(),
        "the last gate is nameable"
    );
    assert_eq!(
        Chip::of(shape, gates, vec![shape.gates]),
        Err(NotAChip::NoSuchGate {
            output: 0,
            names: shape.gates,
            gates: shape.gates
        })
    );
}

#[test]
fn an_exam_of_thirty_two_outputs_has_no_bits_left_over_to_stray_into() {
    let wide = Netlist::over(1, 32, 32, 4).expect("a netlist");
    let answers = vec![u32::MAX; wide.words() as usize];

    assert!(
        Exam::of(wide, answers).is_ok(),
        "with thirty-two outputs every bit is an output and none is stray"
    );
}

#[test]
fn a_view_is_as_many_pixels_as_it_is_wide_and_deep() {
    for (wide, deep) in [(8_u32, 8_u32), (16, 8), (64, 32)] {
        let one = View::of(wide, deep);
        assert_eq!(one.pixels(), one.wide as usize * one.deep as usize);
    }
    assert_eq!(View::of(8, 8).pixels(), 64);
    assert_eq!(View::of(16, 8).pixels(), 128);
    assert!(View::of(16, 8).pixels() > View::of(8, 8).pixels());
}

#[test]
fn a_view_rounds_each_side_up_to_a_power_of_two() {
    let one = View::of(9, 17);
    assert_eq!(one.wide, 16);
    assert_eq!(one.deep, 32);
    assert_eq!(one.pixels(), 512);
}

fn constants(words: &[u32]) -> Vec<u32> {
    let mut held: Vec<u32> = decode::body(words)
        .filter(|one| one.opcode() == op::CONSTANT)
        .filter_map(|one| one.operands().get(2).copied())
        .collect();
    held.sort_unstable();
    held.dedup();
    held
}

#[test]
fn turning_picks_a_field_from_the_low_two_bits_and_its_value_from_the_rest() {
    let one = Netlist::over(4, 2, 8, KINDS).expect("a netlist");
    let start = drawn(&one, 5, 99);
    assert_eq!((start.kind, start.left, start.right), (99, 0, 0));

    let wanted = [
        (0, 0, 0),
        (99, 0, 0),
        (99, 0, 0),
        (99, 0, 0),
        (1, 0, 0),
        (99, 1, 0),
        (99, 0, 1),
        (99, 0, 1),
    ];
    for (roll, want) in wanted.iter().enumerate() {
        let after = turned(&one, 5, start, roll as u32);
        assert_eq!((after.kind, after.left, after.right), *want, "roll {roll}");
    }
}

#[test]
fn held_water_runs_toward_the_air_beside_it_and_the_two_directions_differ() {
    let grid = Grid::of(8, 8);

    let mut rightward = vec![AIR; grid.cells()];
    rightward[0] = WATER | HELD;
    rightward[8] = SAND;
    assert_eq!(
        &settle_reference(grid, 0, &rightward)[..12],
        [0, 0, 0, 0, 0, 0, 0, 0, 2, 1, 0, 0]
    );

    let mut leftward = vec![AIR; grid.cells()];
    leftward[1] = WATER | HELD;
    assert_eq!(
        &settle_reference(grid, 0, &leftward)[..12],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0]
    );
}

#[test]
fn a_waking_brush_emits_the_comparison_a_calm_one_has_no_use_for() {
    let grid = Grid::of(8, 8);
    let waking = brush(32, grid, true).expect("a waking brush");
    let calm = brush(32, grid, false).expect("a calm brush");

    assert_eq!(
        constants(&waking),
        [0, 1, 2, 3, 4, 5, 8, 16, 32, 256, 4_294_967_295]
    );
    assert_eq!(constants(&calm), [0, 1, 2, 3, 4, 5, 16, 32, 4_294_967_295]);
    assert_ne!(waking, calm, "waking changed nothing about the module");
}

#[test]
fn a_chip_reads_the_gate_above_its_inputs_and_not_the_input_below_them() {
    let shape = Netlist::over(2, 1, 2, KINDS).expect("a netlist");
    let wire = Gate {
        kind: 0b1010,
        left: 0,
        right: 0,
    }
    .pack();
    let inverter = Gate {
        kind: 0b0001,
        left: 1,
        right: 1,
    }
    .pack();
    let chip = Chip::of(shape, vec![wire, inverter], vec![1]).expect("a chip");

    assert_eq!(chip.sourced(0b10), [0, 1, 0, 0]);
    assert_eq!(
        (0..4_u32)
            .map(|word| chip.answers(word))
            .collect::<Vec<_>>(),
        [1, 1, 0, 0]
    );
}

#[test]
fn running_water_stopped_by_a_wall_is_put_back_as_the_fields_it_had() {
    let grid = Grid::of(8, 8);
    let mut blocked = vec![AIR; grid.cells()];
    blocked[0] = WATER | RUNNING_RIGHT;
    blocked[1] = WALL;

    assert_eq!(
        &settle_reference(grid, 0, &blocked)[..8],
        [0, 3, 0, 0, 0, 0, 0, 0]
    );
}

#[test]
fn water_already_running_is_not_started_again() {
    let grid = Grid::of(8, 8);
    let mut running = vec![AIR; grid.cells()];
    running[0] = WATER | RUNNING_RIGHT | HELD;

    assert_eq!(
        &settle_reference(grid, 0, &running)[..12],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0]
    );
}

#[test]
fn the_second_cell_of_a_quad_starts_only_when_the_first_is_air() {
    let grid = Grid::of(8, 8);
    let mut blocked = vec![AIR; grid.cells()];
    blocked[1] = WATER | HELD;
    blocked[0] = SAND;

    assert_eq!(
        &settle_reference(grid, 0, &blocked)[..12],
        [0, 0, 0, 0, 0, 0, 0, 0, 2, 1, 0, 0]
    );
}

#[test]
fn a_cell_worn_to_its_patience_does_not_start_running() {
    let grid = Grid::of(8, 8);
    let mut worn = vec![AIR; grid.cells()];
    worn[0] = WATER | HELD | (PATIENCE * TIRED);

    assert_eq!(
        &settle_reference(grid, 0, &worn)[..12],
        [0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0]
    );
}

#[test]
fn the_settle_and_paint_passes_bake_the_constants_their_arithmetic_produces() {
    let grid = Grid::of(8, 8);

    assert_eq!(
        constants(&settle(32, grid).expect("a settle module")),
        [0, 1, 2, 3, 4, 5, 7, 9, 16, 32, 4_294_967_295]
    );
    assert_eq!(
        constants(&paint(32, grid, View::of(8, 8)).expect("a paint module")),
        [
            0,
            1,
            2,
            3,
            6,
            7,
            32,
            1_119_778,
            2_387_880,
            4_475_474,
            14_267_242,
            4_294_967_295
        ]
    );
}

fn pinned_quad(set: &dyn Fn(&mut Vec<u32>)) -> Vec<u32> {
    let grid = Grid::of(8, 8);
    let mut cells = vec![AIR; grid.cells()];
    cells[8] = WALL;
    cells[9] = WALL;
    set(&mut cells);
    settle_reference(grid, 0, &cells)
}

#[test]
fn the_starting_rules_are_read_where_the_fall_cannot_overwrite_them() {
    assert_eq!(
        &pinned_quad(&|cells| cells[0] = WATER | HELD)[..4],
        [1, 0, 0, 0]
    );
    assert_eq!(
        &pinned_quad(&|cells| cells[1] = WATER | HELD)[..4],
        [0, 1, 0, 0]
    );
    assert_eq!(&pinned_quad(&|cells| cells[0] = WATER)[..4], [1, 0, 0, 0]);
}

#[test]
fn a_cell_with_something_beside_it_does_not_start_and_is_put_back() {
    assert_eq!(
        &pinned_quad(&|cells| {
            cells[0] = WATER | HELD;
            cells[1] = WALL;
        })[..4],
        [1, 3, 0, 0]
    );
    assert_eq!(
        &pinned_quad(&|cells| {
            cells[0] = WATER | RUNNING_RIGHT;
            cells[1] = WALL;
        })[..4],
        [1, 3, 0, 0]
    );
}

#[test]
fn water_that_is_already_running_moves_rather_than_starting_again() {
    assert_eq!(
        &pinned_quad(&|cells| cells[0] = WATER | RUNNING_RIGHT | HELD)[..4],
        [0, 9, 0, 0]
    );
}

#[test]
fn a_cell_at_its_patience_keeps_its_wear_and_one_below_it_keeps_less() {
    assert_eq!(
        &pinned_quad(&|cells| cells[0] = WATER | HELD | (PATIENCE * TIRED))[..4],
        [225, 0, 0, 0]
    );
    assert_eq!(
        &pinned_quad(&|cells| cells[0] = WATER | HELD | ((PATIENCE - 1) * TIRED))[..4],
        [193, 0, 0, 0]
    );
}
