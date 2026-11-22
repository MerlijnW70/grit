#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::BTreeSet;

use grit::netlist::Gate;
use grit::stencil::{self, Stencil};

const AND: u32 = 8;

fn moves_of(stencil: Stencil) -> Vec<Vec<u32>> {
    stencil
        .symmetries()
        .iter()
        .map(|places| stencil::inputs_moved(stencil, 1, places))
        .collect()
}

fn and_of(inputs: u32, places: &[u32]) -> Vec<Gate> {
    let mut gates = vec![Gate {
        kind: AND,
        left: places[0],
        right: places[1],
    }];
    for &place in &places[2..] {
        let left = inputs + u32::try_from(gates.len()).unwrap() - 1;
        gates.push(Gate {
            kind: AND,
            left,
            right: place,
        });
    }
    gates
}

fn ored(inputs: u32, gates: &[Gate], moves: &[Vec<u32>], row: u32) -> u32 {
    moves.iter().fold(0, |held, moved| {
        held | stencil::answer(inputs, 1, gates, stencil::image(row, moved))
    })
}

fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn pinned(stencil: Stencil, places: &[u32], copies: usize, others: &[u32]) {
    let inputs = stencil.positions();
    let moves = moves_of(stencil);
    assert_eq!(moves.len(), copies, "{}", stencil.name());
    let gates = and_of(inputs, places);
    let law = stencil::symmetrised(inputs, 1, &gates, &moves);
    assert_eq!(
        law.len(),
        copies * gates.len() + (copies - 1),
        "{}: one copy of the circuit per move and one or for each copy after the first",
        stencil.name()
    );
    let pattern = places.iter().fold(0, |held, place| held | (1 << place));
    let images: BTreeSet<u32> = moves
        .iter()
        .map(|moved| stencil::image(pattern, moved))
        .collect();
    assert_eq!(
        images.len(),
        copies,
        "{}: only the identity leaves the pattern as it is",
        stencil.name()
    );
    for &row in &images {
        let answering = moves
            .iter()
            .filter(|moved| stencil::answer(inputs, 1, &gates, stencil::image(row, moved)) == 1)
            .count();
        assert_eq!(answering, 1, "{}: row {row}", stencil.name());
        assert_eq!(
            stencil::answer(inputs, 1, &law, row),
            1,
            "{}: row {row} is answered by its own copy alone",
            stencil.name()
        );
    }
    for &row in images.iter().chain(others) {
        assert_eq!(
            stencil::answer(inputs, 1, &law, row),
            ored(inputs, &gates, &moves, row),
            "{}: row {row}",
            stencil.name()
        );
    }
}

#[test]
fn the_square_law_is_the_or_of_eight_copies_each_needed_once() {
    let all: Vec<u32> = (0..1 << 9).collect();
    pinned(Stencil::Square, &[0, 1], 8, &all);
}

#[test]
fn the_cube_law_is_the_or_of_forty_eight_copies_each_needed_once() {
    let mut state = 0x5EED_0048_u64;
    let others: Vec<u32> = (0..1_024)
        .map(|at| {
            let mut row = next(&mut state);
            if at % 2 == 1 {
                row &= next(&mut state) & next(&mut state);
            }
            u32::try_from(row & ((1 << 27) - 1)).unwrap()
        })
        .collect();
    pinned(Stencil::Cube, &[0, 1, 4], 48, &others);
}
