#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use grit::marking::{
    BEST, BINDINGS, HELD, SCORE, SEED, TRIAL, VALUES, WANTED, chosen, marked, turned_on, words_of,
};
use grit::netlist::{Chip, Exam, Gate, Netlist, drawn, turned};
use grit::{Groups, Host};

const CROWD: u32 = 256;

fn device(label: &str) -> Option<Host> {
    match Host::open() {
        Ok(Some(host)) => Some(host),
        Ok(None) => {
            eprintln!("SKIPPED {label}");
            None
        }
        Err(why) => {
            eprintln!("SKIPPED {label} {why}");
            None
        }
    }
}

fn shape() -> Netlist {
    Netlist::over(4, 2, 8, 16).unwrap()
}

fn asked(shape: Netlist) -> Exam {
    let answers: Vec<u32> = (0..shape.words())
        .map(|row| {
            let parity = (0..shape.inputs).fold(0, |held, bit| held ^ ((row >> bit) & 1));
            let all = (0..shape.inputs).fold(1, |held, bit| held & ((row >> bit) & 1));
            parity | (all << 1)
        })
        .collect();
    Exam::of(shape, answers).unwrap()
}

fn columns(exam: &Exam) -> Vec<u32> {
    let shape = exam.shape();
    let wide = words_of(shape);
    let mut held = Vec::new();
    for output in 0..shape.outputs {
        for word in 0..wide {
            let mut one = 0_u32;
            for bit in 0..32_u32 {
                let row = word * 32 + bit;
                if row < shape.words() && (exam.asked(row) >> output) & 1 == 1 {
                    one |= 1 << bit;
                }
            }
            held.push(one);
        }
    }
    held
}

fn crowd_of(shape: Netlist, seed: u32, many: u32) -> Vec<u32> {
    let mut state = seed | 1;
    (0..many)
        .flat_map(|_| {
            (0..shape.gates)
                .map(|at| {
                    state ^= state << 13;
                    state ^= state >> 17;
                    state ^= state << 5;
                    drawn(&shape, at, state).pack()
                })
                .collect::<Vec<u32>>()
        })
        .collect()
}

fn crowd(shape: Netlist, seed: u32) -> Vec<u32> {
    crowd_of(shape, seed, CROWD)
}

fn room_for(shape: Netlist, many: u32) -> [usize; BINDINGS as usize] {
    [
        (many * shape.gates) as usize,
        (many * shape.gates) as usize,
        (many * shape.sources() * words_of(shape)) as usize,
        (shape.outputs * words_of(shape)) as usize,
        many as usize,
        many as usize,
        many as usize,
        1,
    ]
}

fn room(shape: Netlist) -> [usize; BINDINGS as usize] {
    room_for(shape, CROWD)
}

fn named(shape: Netlist) -> Vec<u32> {
    (shape.gates - shape.outputs..shape.gates).collect()
}

fn asked_of(shape: Netlist) -> Exam {
    let answers: Vec<u32> = (0..shape.words())
        .map(|row| {
            let parity = (0..shape.inputs).fold(0, |held, bit| held ^ ((row >> bit) & 1));
            let all = (0..shape.inputs).fold(1, |held, bit| held & ((row >> bit) & 1));
            parity | (all << 1)
        })
        .collect();
    Exam::of(shape, answers).unwrap()
}

#[test]
fn a_table_too_wide_for_one_word_is_marked_the_same_on_both_sides() {
    let Some(host) = device("a_table_too_wide_for_one_word_is_marked_the_same_on_both_sides")
    else {
        return;
    };
    let width = host.width();
    eprintln!(
        "{:>8} {:>7} {:>7} {:>8} {:>10}",
        "inputs", "rows", "words", "chips", "apart"
    );
    for inputs in [4_u32, 5, 6, 8, 10] {
        let shape = Netlist::over(inputs, 2, 8, 16).unwrap();
        let exam = asked_of(shape);
        let held = crowd(shape, 0x1234_5678 ^ inputs);
        let module = marked(width, shape).unwrap();
        let modules: Vec<&[u32]> = vec![&module];
        let sizes = room(shape);
        let mut resident = host.resident(&sizes, &modules).unwrap();
        resident.write(TRIAL as usize, &held).unwrap();
        resident
            .write(VALUES as usize, &vec![0_u32; sizes[VALUES as usize]])
            .unwrap();
        resident.write(WANTED as usize, &columns(&exam)).unwrap();
        resident
            .write(SCORE as usize, &vec![0_u32; CROWD as usize])
            .unwrap();
        let over = [Groups::Fixed(CROWD.div_ceil(width))];
        let stages: Vec<&[Groups]> = vec![&over];
        resident.staged(&stages).unwrap();
        let scored = resident.read(SCORE as usize, CROWD as usize).unwrap();

        let outputs = named(shape);
        let mut apart = 0_usize;
        for who in 0..CROWD as usize {
            let gates = held[who * shape.gates as usize..(who + 1) * shape.gates as usize].to_vec();
            let chip = Chip::of(shape, gates, outputs.clone()).unwrap();
            let want = exam.scored(&chip.table());
            if scored[who] != want {
                if apart < 3 {
                    eprintln!(
                        "  {inputs} inputs, chip {who}: device {} host {want}",
                        scored[who]
                    );
                }
                apart += 1;
            }
        }
        eprintln!(
            "{inputs:>8} {:>7} {:>7} {CROWD:>8} {apart:>10}",
            shape.words(),
            words_of(shape)
        );
        assert_eq!(
            apart, 0,
            "{inputs} inputs: {apart} chips marked differently"
        );
    }
}

#[test]
fn the_device_marks_every_chip_exactly_as_the_host_does() {
    let Some(host) = device("the_device_marks_every_chip_exactly_as_the_host_does") else {
        return;
    };
    let width = host.width();
    let shape = shape();
    let exam = asked(shape);
    let held = crowd(shape, 0x1234_5678);

    let module = marked(width, shape).unwrap();
    let modules: Vec<&[u32]> = vec![&module];
    let sizes = room(shape);
    assert_eq!(sizes.len(), BINDINGS as usize);
    let mut resident = host.resident(&sizes, &modules).unwrap();

    resident.write(TRIAL as usize, &held).unwrap();
    resident
        .write(VALUES as usize, &vec![0_u32; sizes[VALUES as usize]])
        .unwrap();
    resident.write(WANTED as usize, &columns(&exam)).unwrap();
    resident
        .write(SCORE as usize, &vec![0_u32; CROWD as usize])
        .unwrap();

    let over = [Groups::Fixed(CROWD.div_ceil(width))];
    let stages: Vec<&[Groups]> = vec![&over];
    resident.staged(&stages).unwrap();
    let scored = resident.read(SCORE as usize, CROWD as usize).unwrap();

    let outputs = named(shape);
    let mut apart = 0_usize;
    let mut best = 0_u32;
    for who in 0..CROWD as usize {
        let gates = held[who * shape.gates as usize..(who + 1) * shape.gates as usize].to_vec();
        let chip = Chip::of(shape, gates, outputs.clone()).unwrap();
        let want = exam.scored(&chip.table());
        best = best.max(want);
        if scored[who] != want {
            if apart < 4 {
                eprintln!(
                    "chip {who}: the device says {} and the host {want}",
                    scored[who]
                );
            }
            apart += 1;
        }
    }
    eprintln!(
        "{CROWD} chips of {} gates, {} marks each, the best drawn one scores {best}",
        shape.gates,
        exam.marks()
    );
    assert_eq!(apart, 0, "{apart} of {CROWD} chips are marked differently");
}

#[test]
fn a_gate_packed_by_the_host_is_the_gate_the_device_reads() {
    let shape = shape();
    let gate = Gate {
        kind: 0b0110,
        left: 1,
        right: 3,
    };
    assert_eq!(Gate::unpack(gate.pack()), gate);
    assert!(shape.checked(4, gate).is_ok());
}

fn marking_on(host: &Host, shape: Netlist) -> (Vec<u32>, [usize; BINDINGS as usize]) {
    (marked(host.width(), shape).unwrap(), room(shape))
}

#[test]
fn whether_a_crowd_of_climbers_finds_the_table() {
    let Some(host) = device("whether_a_crowd_of_climbers_finds_the_table") else {
        return;
    };
    let width = host.width();
    let shape = shape();
    let exam = asked(shape);
    let (module, sizes) = marking_on(&host, shape);
    let modules: Vec<&[u32]> = vec![&module];
    let mut resident = host.resident(&sizes, &modules).unwrap();
    resident.write(WANTED as usize, &columns(&exam)).unwrap();
    resident
        .write(VALUES as usize, &vec![0_u32; sizes[VALUES as usize]])
        .unwrap();
    let over = [Groups::Fixed(CROWD.div_ceil(width))];
    let stages: Vec<&[Groups]> = vec![&over];

    let marks = |resident: &mut grit::Resident<'_>, held: &[u32]| {
        resident.write(TRIAL as usize, held).unwrap();
        resident.staged(&stages).unwrap();
        resident.read(SCORE as usize, CROWD as usize).unwrap()
    };

    let mut held = crowd(shape, 0x1234_5678);
    let mut best = marks(&mut resident, &held);
    let mut state = 0x0BAD_C0DE_u32;
    let full = exam.marks();
    eprintln!(
        "{:>7} {:>8} {:>9} {:>9}",
        "round", "best", "average", "solved"
    );
    let mut rounds = 0_u32;
    for round in 0..2000_u32 {
        rounds = round + 1;
        let mut trial = held.clone();
        for who in 0..CROWD as usize {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            let at = state % shape.gates;
            let place = who * shape.gates as usize + at as usize;
            let gate = Gate::unpack(trial[place]);
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            trial[place] = turned(&shape, at, gate, state).pack();
        }
        let got = marks(&mut resident, &trial);
        for who in 0..CROWD as usize {
            if got[who] >= best[who] {
                best[who] = got[who];
                let from = who * shape.gates as usize;
                let to = from + shape.gates as usize;
                held[from..to].copy_from_slice(&trial[from..to]);
            }
        }
        let top = best.iter().copied().max().unwrap_or(0);
        let solved = best.iter().filter(|one| **one == full).count();
        if round % 200 == 199 || solved > 0 {
            let sum: u32 = best.iter().sum();
            eprintln!(
                "{:>7} {top:>8} {:>9.1} {solved:>9}",
                round + 1,
                f64::from(sum) / f64::from(CROWD)
            );
        }
        if solved > 0 {
            break;
        }
    }
    let top = best.iter().copied().max().unwrap_or(0);
    let solved = best.iter().filter(|one| **one == full).count();
    eprintln!(
        "after {rounds} rounds {solved} of {CROWD} climbers hold a perfect {full}, the best is {top}"
    );
    assert!(top > 23, "the crowd never beat the best drawn chip: {top}");
}

#[test]
fn whether_the_crowd_climbs_without_the_host() {
    let Some(host) = device("whether_the_crowd_climbs_without_the_host") else {
        return;
    };
    let width = host.width();
    let shape = shape();
    let exam = asked(shape);
    let turn = turned_on(width, shape).unwrap();
    let mark = marked(width, shape).unwrap();
    let pick = chosen(width, shape, 0, CROWD).unwrap();
    let modules: Vec<&[u32]> = vec![&turn, &mark, &pick];
    let sizes = room(shape);
    let mut resident = host.resident(&sizes, &modules).unwrap();

    let over = Groups::Fixed(CROWD.div_ceil(width));
    let turning = [over];
    let marking = [over];
    let choosing = [over];
    let stages: Vec<&[Groups]> = vec![&turning, &marking, &choosing];

    resident
        .write(HELD as usize, &crowd(shape, 0x1234_5678))
        .unwrap();
    resident
        .write(TRIAL as usize, &vec![0_u32; sizes[TRIAL as usize]])
        .unwrap();
    resident
        .write(VALUES as usize, &vec![0_u32; sizes[VALUES as usize]])
        .unwrap();
    resident.write(WANTED as usize, &columns(&exam)).unwrap();
    resident
        .write(SCORE as usize, &vec![0_u32; CROWD as usize])
        .unwrap();
    resident
        .write(BEST as usize, &vec![0_u32; CROWD as usize])
        .unwrap();
    let seeds: Vec<u32> = (0..CROWD).map(|who| (who * 2_654_435_761) | 1).collect();
    resident.write(SEED as usize, &seeds).unwrap();

    let full = exam.marks();
    let batch = 100_u32;
    let started = std::time::Instant::now();
    eprintln!(
        "{:>8} {:>8} {:>9} {:>9}",
        "rounds", "best", "average", "solved"
    );
    let mut done = 0_u32;
    let mut top = 0_u32;
    let mut solved = 0_usize;
    for _ in 0..30_u32 {
        resident.staged_frames(&stages, batch).unwrap();
        done += batch;
        let best = resident.read(BEST as usize, CROWD as usize).unwrap();
        top = best.iter().copied().max().unwrap_or(0);
        solved = best.iter().filter(|one| **one == full).count();
        let sum: u32 = best.iter().copied().sum();
        eprintln!(
            "{done:>8} {top:>8} {:>9.1} {solved:>9}",
            f64::from(sum) / f64::from(CROWD)
        );
        if solved > 0 {
            break;
        }
    }
    let took = started.elapsed();
    eprintln!("{done} rounds on the device in {took:?}, {solved} of {CROWD} hold a perfect {full}");

    let held = resident.read(HELD as usize, sizes[HELD as usize]).unwrap();
    let best = resident.read(BEST as usize, CROWD as usize).unwrap();
    let outputs = named(shape);
    let mut checked = 0_usize;
    for (who, mark) in best.iter().enumerate() {
        if *mark != full {
            continue;
        }
        let gates = held[who * shape.gates as usize..(who + 1) * shape.gates as usize].to_vec();
        let chip = Chip::of(shape, gates, outputs.clone()).unwrap();
        assert_eq!(
            exam.scored(&chip.table()),
            full,
            "chip {who} is held to be perfect and is not"
        );
        checked += 1;
    }
    eprintln!("{checked} of them are perfect when the host marks them itself");
    assert!(top > 23, "the crowd never beat the best drawn chip: {top}");
    assert!(checked > 0, "nothing solved it");
}

#[test]
fn how_wide_the_crowd_can_be_and_how_fast_it_climbs() {
    let Some(host) = device("how_wide_the_crowd_can_be_and_how_fast_it_climbs") else {
        return;
    };
    let width = host.width();
    let shape = shape();
    let exam = asked(shape);
    let full = exam.marks();
    let turn = turned_on(width, shape).unwrap();
    let mark = marked(width, shape).unwrap();
    let pick = chosen(width, shape, 0, CROWD).unwrap();
    let modules: Vec<&[u32]> = vec![&turn, &mark, &pick];

    eprintln!(
        "{:>9} {:>8} {:>12} {:>13} {:>9}",
        "crowd", "rounds", "took", "a round", "solved"
    );
    let mut told = Vec::new();
    for many in [256_u32, 4096, 65536] {
        let sizes = room_for(shape, many);
        let mut resident = host.resident(&sizes, &modules).unwrap();
        resident
            .write(HELD as usize, &crowd_of(shape, 0x1234_5678, many))
            .unwrap();
        for binding in [TRIAL, VALUES, SCORE, BEST] {
            resident
                .write(binding as usize, &vec![0_u32; sizes[binding as usize]])
                .unwrap();
        }
        resident.write(WANTED as usize, &columns(&exam)).unwrap();
        let seeds: Vec<u32> = (0..many).map(|who| (who * 2_654_435_761) | 1).collect();
        resident.write(SEED as usize, &seeds).unwrap();

        let over = Groups::Fixed(many.div_ceil(width));
        let turning = [over];
        let marking = [over];
        let choosing = [over];
        let stages: Vec<&[Groups]> = vec![&turning, &marking, &choosing];

        let rounds = 1000_u32;
        let started = std::time::Instant::now();
        resident.staged_frames(&stages, rounds).unwrap();
        let best = resident.read(BEST as usize, many as usize).unwrap();
        let took = started.elapsed();
        let solved = best.iter().filter(|one| **one == full).count();
        eprintln!(
            "{many:>9} {rounds:>8} {took:>12?} {:>13?} {solved:>9}",
            took / rounds,
        );
        told.push((many, solved, took));
    }
    for (many, solved, took) in &told {
        eprintln!("{many} climbers, {solved} of them perfect, in {took:?}");
    }
    let (_, solved, _) = told
        .last()
        .copied()
        .unwrap_or((0, 0, std::time::Duration::ZERO));
    assert!(solved > 0, "the widest crowd solved nothing");
}
