#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use grit::law;
use grit::rule3::code::Code as CodeRule;
use grit::rule3::heat::{Heat, TUNED};
use grit::rule3::nature::Nature;
use grit::rule3::sand::Sand;
use grit::rule3::{CORNERS, Oct, OneStep, Rule};
use grit::sand::Grid;
use grit::sand3;

const TRACED_OCTS: u32 = 200_000;

const WIDTHS: [u32; 5] = [4, 8, 16, 32, 64];

fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut mixed = *state;
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    mixed ^ (mixed >> 31)
}

fn word(state: &mut u64) -> u32 {
    (next(state) >> 16) as u32
}

#[derive(Clone, Copy, Debug)]
struct Case {
    words: [u32; CORNERS],
    frame: u32,
    at: [u32; 3],
}

fn ruled<R: OneStep>(rule: &R, case: Case, params: &[u32]) -> (Vec<u32>, Vec<u32>) {
    let made = Rule::step(
        rule,
        Oct {
            words: case.words,
            frame: case.frame,
            at: case.at,
            params,
        },
    );
    (made.out.to_vec(), made.counts)
}

fn traced_equals<R: OneStep>(name: &str, rule: &R, params: &[u32], masks: &[u32]) {
    let dag = law::trace(rule);
    let mut state = 0x1234_5678_9ABC_DEF1_u64;
    let mut differ = 0;
    for at in 0..TRACED_OCTS {
        let mask = masks[at as usize % masks.len()];
        let case = Case {
            words: core::array::from_fn(|_| word(&mut state) & mask),
            frame: word(&mut state),
            at: [word(&mut state), word(&mut state), word(&mut state)],
        };
        if ruled(rule, case, params) != ruled(&dag, case, params) {
            differ += 1;
        }
    }
    println!(
        "{name}: {} nodes; {differ} of {TRACED_OCTS} octs differ",
        dag.len()
    );
    assert_eq!(differ, 0, "{name}");
    let grid = Grid::solid(32, 32, 32).unwrap();
    for width in WIDTHS {
        let one = sand3::settle_by(rule, width, grid, u32::MAX).unwrap().words;
        let other = sand3::settle_by(&dag, width, grid, u32::MAX).unwrap().words;
        println!(
            "{name} width {width}: {} words, identical {}",
            one.len(),
            one == other
        );
        assert_eq!(one, other, "{name} width {width}");
    }
}

#[test]
fn the_trace_of_sand_replays_with_identical_kernels() {
    traced_equals("sand", &Sand, &[], &[0x1FF, 0x3, u32::MAX]);
}

#[test]
fn the_trace_of_heat_replays_with_identical_kernels() {
    traced_equals("heat", &Heat, &TUNED, &[0xFFFF_F00F, 0x0FFF_F007, u32::MAX]);
}

#[test]
fn the_trace_of_code_replays_with_identical_kernels() {
    traced_equals(
        "code",
        &CodeRule,
        &[],
        &[u32::MAX, 0x7F00_00FF, 0x0700_00FF],
    );
}

#[test]
fn the_trace_of_nature_replays_with_identical_kernels() {
    traced_equals(
        "nature",
        &Nature,
        &[3],
        &[0xFFFF_0FFF, 0x0FFF_0F0F, u32::MAX, 0x0000_000F],
    );
}
