#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use grit::rule3::code::{
    ADD, COPY, Code, DATA, HALT, INSTRUCTIONS, MINUS_X, MINUS_Y, MINUS_Z, MOVE, NOP, PAYLOAD,
    PLUS_X, PLUS_Y, PLUS_Z, TURN, WAYS, claim, instruction_of, payload_of, stepped, turned, way_of,
    word,
};
use grit::rule3::{CORNERS, Oct, Rule};
use grit::sand3::{Grid3, reference};

const SEEDS: [u64; 4] = [1, 7, 0x5EED, 0xDEAD_BEEF];

fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut mixed = *state;
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    mixed ^ (mixed >> 31)
}

fn frame(grid: Grid3, frame: u32, cells: &[u32]) -> Vec<u32> {
    reference(&Code, grid, frame, cells, &[])
}

fn run(grid: Grid3, start: &[u32], first: u32, frames: u32) -> Vec<u32> {
    let mut cells = start.to_vec();
    for at in first..first + frames {
        cells = frame(grid, at, &cells);
    }
    cells
}

fn world(grid: Grid3, cells: &[([u32; 3], u32)]) -> Vec<u32> {
    let mut out = vec![word(NOP, 0, 0); grid.cells()];
    for &([x, y, z], held) in cells {
        out[grid.index(x, y, z)] = held;
    }
    out
}

fn extent(grid: Grid3, axis: usize) -> u32 {
    [grid.wide, grid.deep, grid.tall][axis]
}

/// `from` moved `by` cells along `axis`, wrapped.
fn along(grid: Grid3, from: [u32; 3], axis: usize, by: i64) -> [u32; 3] {
    let mut out = from;
    let size = i64::from(extent(grid, axis));
    out[axis] = u32::try_from((i64::from(from[axis]) + by).rem_euclid(size)).unwrap();
    out
}

const AXES: [(u32, u32, usize); 3] = [
    (PLUS_X, MINUS_X, 0),
    (PLUS_Y, MINUS_Y, 1),
    (PLUS_Z, MINUS_Z, 2),
];

/// A packet in an empty world, started where it faces its way in frame 0,
/// is one cell further along its way after every frame, its payload with
/// it, and wraps the seam of its axis back to where it started.
#[test]
fn a_move_packet_crosses_the_world_and_wraps_each_seam() {
    let grid = Grid3::new(16, 8, 4).unwrap();
    for (plus, minus, axis) in AXES {
        for (way, sign, start) in [(plus, 1, [6, 2, 2]), (minus, -1, [5, 3, 1])] {
            let mut start = start;
            start[axis] = if sign > 0 { 2 } else { 1 };
            let packet = word(MOVE, way, 0x00AB_CDEF);
            let mut cells = world(grid, &[(start, packet)]);
            let size = extent(grid, axis);
            for done in 1..=2 * size {
                cells = frame(grid, done - 1, &cells);
                let now = along(grid, start, axis, sign * i64::from(done));
                assert_eq!(
                    cells,
                    world(grid, &[(now, packet)]),
                    "way {way} after {done} frames"
                );
            }
            assert_eq!(cells, world(grid, &[(start, packet)]));
        }
    }
}

/// A packet that starts where it does not face its way waits exactly one
/// frame, then moves one cell a frame.
#[test]
fn a_packet_out_of_step_waits_one_frame_and_then_moves_every_frame() {
    let grid = Grid3::new(8, 16, 8).unwrap();
    for (plus, minus, axis) in AXES {
        for (way, sign) in [(plus, 1_i64), (minus, -1)] {
            let mut start = [4, 4, 4];
            start[axis] = if sign > 0 { 3 } else { 4 };
            let packet = word(MOVE, way, 77);
            let mut cells = world(grid, &[(start, packet)]);
            cells = frame(grid, 0, &cells);
            assert_eq!(cells, world(grid, &[(start, packet)]), "way {way}");
            for done in 1..=10 {
                cells = frame(grid, done, &cells);
                let now = along(grid, start, axis, sign * i64::from(done));
                assert_eq!(
                    cells,
                    world(grid, &[(now, packet)]),
                    "way {way} frame {done}"
                );
            }
        }
    }
}

/// An ADD cell at the head of a row of DATA, with a HALT at its end, walks
/// the row and leaves one cell that holds the sum of the row and its own
/// payload, modulo 2 to the 24. It eats one cell a frame from its first
/// frame in step; then nothing changes.
#[test]
fn an_add_chain_sums_a_row_of_data_values_into_one_cell() {
    let grid = Grid3::new(16, 16, 32).unwrap();
    let values = [3, 9, 0x00FF_FFFF, 12, 1000, 0, 5, 0x0080_0000, 41, 7, 2];
    for (plus, minus, axis) in AXES {
        for (way, sign) in [(plus, 1_i64), (minus, -1)] {
            let head = along(grid, [1, 1, 1], axis, 0);
            let own = 0x0012_3456;
            let mut cells = vec![(head, word(ADD, way, own))];
            for (offset, &value) in (1..).zip(&values) {
                cells.push((along(grid, head, axis, sign * offset), word(DATA, 0, value)));
            }
            let end = i64::try_from(values.len()).unwrap() + 1;
            let wall = along(grid, head, axis, sign * end);
            cells.push((wall, word(HALT, 0, 0)));
            let beside = along(grid, along(grid, head, (axis + 1) % 3, 1), axis, sign);
            cells.push((beside, word(DATA, 0, 99)));
            let start = world(grid, &cells);

            let sum = values
                .iter()
                .fold(own, |sum, &value| (sum + value) & PAYLOAD);
            let last = along(grid, head, axis, sign * (end - 1));
            let want = world(
                grid,
                &[
                    (last, word(ADD, way, sum)),
                    (wall, word(HALT, 0, 0)),
                    (beside, word(DATA, 0, 99)),
                ],
            );
            let waiting = u32::from(sign > 0);
            let frames = waiting + u32::try_from(values.len()).unwrap();
            let before = run(grid, &start, 0, frames - 1);
            assert_ne!(before, want, "way {way}: summed a frame early");
            let summed = run(grid, &start, 0, frames);
            assert_eq!(summed, want, "way {way}: after {frames} frames");
            assert_eq!(run(grid, &summed, frames, 9), want, "way {way}: not quiet");
            assert_eq!(
                payload_of(summed[grid.index(last[0], last[1], last[2])]),
                sum
            );
        }
    }
}

fn mover(way: u32, payload: u32) -> u32 {
    word(MOVE, way, payload)
}

/// Three packets claim one empty cell in one frame, along x, y and z. The x
/// packet takes it; the y packet takes it once it is free; the z packet
/// waits for both. Every frame is as worked out by hand, on every run.
#[test]
fn packets_that_meet_resolve_by_the_priority_the_same_way_every_run() {
    let grid = Grid3::new(8, 8, 8).unwrap();
    let (a, b, c) = (mover(PLUS_X, 0xA), mover(MINUS_Y, 0xB), mover(MINUS_Z, 0xC));
    let start = world(grid, &[([0, 0, 0], a), ([1, 1, 0], b), ([1, 0, 1], c)]);
    let expected: [[[u32; 3]; 3]; 6] = [
        [[1, 0, 0], [1, 1, 0], [1, 0, 1]],
        [[2, 0, 0], [1, 1, 0], [1, 0, 1]],
        [[3, 0, 0], [1, 0, 0], [1, 0, 1]],
        [[4, 0, 0], [1, 7, 0], [1, 0, 1]],
        [[5, 0, 0], [1, 6, 0], [1, 0, 0]],
        [[6, 0, 0], [1, 5, 0], [1, 0, 7]],
    ];
    let mut first = None;
    for _ in 0..8 {
        let mut cells = start.clone();
        let mut seen = Vec::new();
        for (done, places) in (0..).zip(expected) {
            cells = frame(grid, done, &cells);
            assert_eq!(
                cells,
                world(grid, &[(places[0], a), (places[1], b), (places[2], c)]),
                "after {} frames",
                done + 1
            );
            seen.push(cells.clone());
        }
        match &first {
            None => first = Some(seen),
            Some(held) => assert_eq!(held, &seen),
        }
    }
}

/// Inside one oct, each target claimed along two or three axes goes to the
/// first axis of the priority, x, then y, then z, whatever the claimant.
#[test]
fn every_conflict_in_an_oct_goes_to_the_first_axis_of_the_priority() {
    let ways = |corner: usize, axis: usize| -> u32 {
        let high = u32::from(corner & [1, 4, 2][axis] != 0);
        2 * u32::try_from(axis).unwrap() + high
    };
    for target in 0..CORNERS {
        for set in 1..8_usize {
            let claimants: Vec<usize> = (0..3).filter(|axis| set & (1 << axis) != 0).collect();
            for kind in [MOVE, COPY] {
                let mut words = [word(HALT, 0, 0); CORNERS];
                words[target] = word(NOP, 0, 0);
                for &axis in &claimants {
                    let from = target ^ [1, 4, 2][axis];
                    words[from] = word(kind, ways(from, axis), 100 + u32::try_from(axis).unwrap());
                }
                let out = stepped(words);
                let winner = claimants[0];
                let from = target ^ [1, 4, 2][winner];
                assert_eq!(out[target], words[from], "target {target} set {set:b}");
                for &axis in &claimants {
                    let other = target ^ [1, 4, 2][axis];
                    let left = if axis == winner && kind == MOVE {
                        word(NOP, 0, 0)
                    } else {
                        words[other]
                    };
                    assert_eq!(out[other], left, "target {target} set {set:b} axis {axis}");
                }
            }
            let mut words = [word(HALT, 0, 0); CORNERS];
            words[target] = word(DATA, 0, 1000);
            for &axis in &claimants {
                let from = target ^ [1, 4, 2][axis];
                words[from] = word(ADD, ways(from, axis), u32::try_from(axis).unwrap() + 1);
            }
            let out = stepped(words);
            let winner = claimants[0];
            let from = target ^ [1, 4, 2][winner];
            assert_eq!(
                out[target],
                word(
                    ADD,
                    ways(from, winner),
                    1001 + u32::try_from(winner).unwrap()
                )
            );
            assert_eq!(out[from], word(NOP, 0, 0));
        }
    }
}

/// A box of HALT walls with a wall inside it that leaves a gap, a COPY cell
/// in it, and DATA and empty cells outside it. The copy fills every empty
/// cell inside, changes nothing outside, grows every frame until the box is
/// full, and changes nothing after.
#[test]
fn a_copy_loop_fills_a_walled_region_and_then_goes_quiet() {
    let grid = Grid3::new(16, 16, 16).unwrap();
    let seed = word(COPY, PLUS_Y, 0x0012_3456);
    let inside = |x: u32, y: u32, z: u32| {
        (2..=9).contains(&x) && (3..=8).contains(&y) && (1..=6).contains(&z)
    };
    let shell = |x: u32, y: u32, z: u32| {
        (1..=10).contains(&x) && (2..=9).contains(&y) && (0..=7).contains(&z) && !inside(x, y, z)
    };
    let inner = |x: u32, y: u32, z: u32| x == 6 && inside(x, y, z) && !(y == 8 && z == 6);
    let mut state = 0x5EED;
    let mut start = vec![word(NOP, 0, 0); grid.cells()];
    let mut want = start.clone();
    let mut room = 0;
    for z in 0..16 {
        for y in 0..16 {
            for x in 0..16 {
                let at = grid.index(x, y, z);
                let roll = next(&mut state);
                let (first, last) = if shell(x, y, z) || inner(x, y, z) {
                    (word(HALT, 0, 0), word(HALT, 0, 0))
                } else if inside(x, y, z) {
                    room += 1;
                    let cell = if [x, y, z] == [3, 4, 2] {
                        seed
                    } else {
                        word(NOP, 0, 0)
                    };
                    (cell, seed)
                } else if roll.is_multiple_of(5) {
                    let data = word(DATA, 0, (roll >> 8) as u32);
                    (data, data)
                } else {
                    (word(NOP, 0, 0), word(NOP, 0, 0))
                };
                start[at] = first;
                want[at] = last;
            }
        }
    }
    let copies = |cells: &[u32]| cells.iter().filter(|&&cell| cell == seed).count();
    let mut model: Vec<bool> = start.iter().map(|&cell| cell == seed).collect();
    let mut cells = start;
    let mut filled = None;
    let mut still = 0;
    for done in 0..64_u32 {
        let phase = done & 1;
        let mut grown = model.clone();
        for z in 0..16 {
            for y in 0..16 {
                for x in 0..16 {
                    let at = grid.index(x, y, z);
                    if model[at] || cells[at] != word(NOP, 0, 0) || !inside(x, y, z) {
                        continue;
                    }
                    for axis in 0..3 {
                        for sign in [-1, 1] {
                            let near = along(grid, [x, y, z], axis, sign);
                            let low = if sign > 0 {
                                [x, y, z][axis]
                            } else {
                                near[axis]
                            };
                            if model[grid.index(near[0], near[1], near[2])] && low % 2 == phase {
                                grown[at] = true;
                            }
                        }
                    }
                }
            }
        }
        model = grown;
        let after = frame(grid, done, &cells);
        let modelled: Vec<bool> = after.iter().map(|&cell| cell == seed).collect();
        assert_eq!(modelled, model, "frame {done}: the copy left the model");
        let changed = after != cells;
        match filled {
            None => {
                still = if changed { 0 } else { still + 1 };
                assert!(still < 2, "the copy stalled for two frames at frame {done}");
                if copies(&after) == room {
                    filled = Some(done + 1);
                }
            }
            Some(at) => assert!(!changed, "frame {done} changed the world, full after {at}"),
        }
        cells = after;
    }
    assert_eq!(cells, want);
    let filled = filled.unwrap();
    println!("{room} cells filled after {filled} frames, then quiet");
    assert_eq!((room, filled), (253, 26));
}

/// A corridor one cell wide along z with walls all round: from a COPY at its
/// low end, in step in frame 0, one more cell is filled every frame.
#[test]
fn a_copy_fills_a_corridor_one_cell_a_frame() {
    let grid = Grid3::new(4, 4, 16).unwrap();
    let seed = word(COPY, 0, 9);
    let mut cells = vec![word(HALT, 0, 0); grid.cells()];
    for z in 2..=13 {
        cells[grid.index(1, 1, z)] = word(NOP, 0, 0);
    }
    cells[grid.index(1, 1, 2)] = seed;
    for done in 1..=12 {
        cells = frame(grid, done - 1, &cells);
        let full: Vec<u32> = (2..=13)
            .filter(|&z| cells[grid.index(1, 1, z)] == seed)
            .collect();
        let want: Vec<u32> = (2..=13)
            .take(usize::try_from(done + 1).unwrap().min(12))
            .collect();
        assert_eq!(full, want, "after {done} frames");
    }
}

/// A TURN cell sends a packet that comes next to it along the turn's way, and
/// a packet that faces a HALT cell stops as DATA with its payload.
#[test]
fn a_turn_sends_a_packet_round_a_corner_into_a_halt() {
    let grid = Grid3::new(16, 16, 16).unwrap();
    let packet = mover(PLUS_X, 0x0042_4242);
    let start = world(
        grid,
        &[
            ([2, 4, 4], packet),
            ([9, 4, 4], word(TURN, PLUS_Z, 0)),
            ([8, 4, 11], word(HALT, 0, 0)),
        ],
    );
    let mut cells = start;
    let mut stopped = None;
    for done in 0..40 {
        cells = frame(grid, done, &cells);
        if cells[grid.index(8, 4, 10)] == word(DATA, PLUS_Z, 0x0042_4242) {
            stopped = Some(done + 1);
            break;
        }
    }
    let stopped = stopped.unwrap();
    assert_eq!(
        cells,
        world(
            grid,
            &[
                ([8, 4, 10], word(DATA, PLUS_Z, 0x0042_4242)),
                ([9, 4, 4], word(TURN, PLUS_Z, 0)),
                ([8, 4, 11], word(HALT, 0, 0)),
            ],
        )
    );
    assert_eq!(stopped, 13);
    assert_eq!(run(grid, &cells, stopped, 8), cells);
}

/// Every word with every bit at random over a world of DATA and HALT cells
/// only: no frame changes a word.
#[test]
fn a_world_of_data_and_halt_is_quiet_from_its_first_frame() {
    let grid = Grid3::new(16, 8, 32).unwrap();
    for seed in SEEDS {
        let mut state = seed;
        let start: Vec<u32> = (0..grid.cells())
            .map(|_| {
                let roll = next(&mut state) as u32;
                let instruction = if roll.is_multiple_of(3) { HALT } else { DATA };
                (roll & !(0xF << 24)) | (instruction << 24)
            })
            .collect();
        for first in [0, 1, u32::MAX] {
            let mut cells = start.clone();
            for done in 0..4 {
                cells = frame(grid, first.wrapping_add(done), &cells);
                assert_eq!(cells, start, "seed {seed:#x} first {first}");
            }
        }
    }
}

fn wild(state: &mut u64) -> u32 {
    let roll = next(state);
    let instruction = (roll % 9) as u32;
    let instruction = if instruction == 8 {
        7 + ((roll >> 40) % 9) as u32
    } else {
        instruction
    };
    let way = ((roll >> 8) % 8) as u32;
    let payload = (roll >> 16) as u32;
    let spare = ((roll >> 50) & 1) as u32;
    word(instruction, way, payload) | (spare << 31)
}

/// The step reads only the eight words: the same words at any frame, any
/// point and with any parameters give the same result, so an oct that is
/// quiet once is quiet for good.
#[test]
fn the_step_is_a_function_of_the_eight_words_alone() {
    let mut state = 0xC0DE;
    let mut changed = 0;
    for _ in 0..100_000 {
        let words: [u32; CORNERS] = std::array::from_fn(|_| wild(&mut state));
        let base = Code.step(Oct {
            words,
            frame: 0,
            at: [0, 0, 0],
            params: &[],
        });
        let params = [next(&mut state) as u32, next(&mut state) as u32];
        let other = Code.step(Oct {
            words,
            frame: next(&mut state) as u32,
            at: [
                next(&mut state) as u32,
                next(&mut state) as u32,
                next(&mut state) as u32,
            ],
            params: &params,
        });
        assert_eq!(base, other, "{words:x?}");
        assert_eq!(base.out, stepped(words));
        changed += usize::from(base.out != words);
    }
    assert!(changed > 1000);
}

/// No step makes or loses a payload except by the documented ways: the
/// cells that are not empty after a step are the cells before, less each
/// MOVE or ADD whose claim won, plus each claimed cell. Every word in the
/// output is a word of the input, a cell emptied, a packet stopped as DATA,
/// or an ADD with a sum.
#[test]
fn every_word_after_a_step_comes_from_the_words_before() {
    let mut state = 0xFACE;
    for _ in 0..100_000 {
        let words: [u32; CORNERS] = std::array::from_fn(|_| wild(&mut state));
        let out = stepped(words);
        let cells = turned(words);
        for (corner, &after) in out.iter().enumerate() {
            let before = cells[corner];
            if after == before {
                continue;
            }
            let came = (0..3).any(|axis: usize| {
                let from = corner ^ [1, 4, 2][axis];
                claim(cells[from], from, before, axis) == Some(after)
            });
            let emptied = after == 0 && matches!(instruction_of(before), MOVE | ADD);
            let stopped = instruction_of(before) == MOVE
                && instruction_of(after) == DATA
                && after & !(0xF << 24) == before & !(0xF << 24);
            assert!(
                came || emptied || stopped,
                "{words:x?} corner {corner}: {after:#x}"
            );
        }
    }
}

#[test]
fn each_instruction_does_what_the_module_says() {
    let empty = word(NOP, 0, 0);
    let fill = |corner: usize, held: u32| {
        let mut words = [word(HALT, 0, 0); CORNERS];
        words[corner] = held;
        words
    };

    let mut words = [empty; CORNERS];
    words[0] = word(MOVE, PLUS_X, 5);
    let out = stepped(words);
    assert_eq!((out[0], out[1]), (empty, word(MOVE, PLUS_X, 5)));
    words[0] = word(MOVE, MINUS_X, 5);
    assert_eq!(
        stepped(words),
        words,
        "a packet that faces out of the oct waits"
    );
    words[0] = word(MOVE, 6, 5);
    assert_eq!(stepped(words), words, "way 6 is no way");
    words[0] = word(MOVE, 7, 5);
    assert_eq!(stepped(words), words, "way 7 is no way");

    let mut words = fill(0, word(MOVE, PLUS_Y, 8) | (1 << 31));
    words[4] = word(HALT, 0, 0);
    let out = stepped(words);
    assert_eq!(out[0], word(DATA, PLUS_Y, 8) | (1 << 31));
    words[4] = word(DATA, 0, 1);
    assert_eq!(stepped(words), words, "a packet does not enter DATA");

    let mut words = [word(DATA, 0, 1); CORNERS];
    words[7] = word(COPY, MINUS_Z, 3) | (1 << 31);
    words[6] = empty;
    words[5] = empty;
    words[3] = empty;
    words[4] = empty;
    let out = stepped(words);
    for corner in [6, 5, 3] {
        assert_eq!(out[corner], words[7], "copy into corner {corner}");
    }
    assert_eq!(
        out[4], empty,
        "corner 4 is not a face neighbour of corner 7"
    );
    assert_eq!(out[7], words[7]);

    let mut words = fill(2, word(ADD, MINUS_Z, PAYLOAD));
    words[0] = word(DATA, PLUS_X, 2);
    let out = stepped(words);
    assert_eq!((out[2], out[0]), (empty, word(ADD, MINUS_Z, 1)));
    words[0] = empty;
    assert_eq!(stepped(words), words, "an ADD walks only into DATA");

    let mut words = [empty; CORNERS];
    words[0] = word(MOVE, MINUS_X, 1);
    words[4] = word(TURN, PLUS_X, 0);
    words[2] = word(TURN, PLUS_Y, 0);
    let out = stepped(words);
    assert_eq!(out[0], empty);
    assert_eq!(
        out[1],
        word(MOVE, PLUS_X, 1),
        "the y turn comes first and sends it on"
    );
    assert_eq!((out[2], out[4]), (words[2], words[4]));
    words[2] = word(TURN, PLUS_Y, 0);
    words[4] = word(TURN, PLUS_Y, 0);
    let out = stepped(words);
    assert_eq!(
        out[0],
        word(MOVE, PLUS_Y, 1),
        "a packet never enters a turn"
    );
    words[1] = word(TURN, MINUS_Z, 0);
    let out = stepped(words);
    assert_eq!(out[0], word(MOVE, MINUS_Z, 1), "the x turn comes first");

    for instruction in [DATA, TURN, HALT, INSTRUCTIONS, 15] {
        for way in 0..8 {
            let mut words = [empty; CORNERS];
            words[0] = word(instruction, way, 1);
            words[7] = word(DATA, 0, 2);
            assert_eq!(stepped(words), words, "instruction {instruction} way {way}");
        }
    }
    assert_eq!(WAYS, 6);
    assert_eq!(way_of(word(MOVE, MINUS_Z, PAYLOAD)), MINUS_Z);
    assert_eq!(payload_of(word(MOVE, MINUS_Z, u32::MAX)), PAYLOAD);
    assert_eq!(instruction_of(word(ADD, 7, u32::MAX)), ADD);
}

/// A head-on pair of packets blocks each other: both wait for good, and the
/// world is quiet.
#[test]
fn packets_head_on_wait_for_good() {
    let grid = Grid3::new(8, 8, 8).unwrap();
    let start = world(
        grid,
        &[
            ([2, 0, 0], mover(PLUS_X, 1)),
            ([3, 0, 0], mover(MINUS_X, 2)),
        ],
    );
    assert_eq!(run(grid, &start, 0, 16), start);
}
