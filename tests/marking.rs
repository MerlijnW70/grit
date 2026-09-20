#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use grit::marking::{BINDINGS, GATES, SCORE, VALUES, WANTED, marked};
use grit::netlist::{Chip, Exam, Gate, Netlist, drawn};
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
    (0..shape.outputs)
        .map(|output| {
            (0..shape.words()).fold(0_u32, |held, row| {
                held | (((exam.asked(row) >> output) & 1) << row)
            })
        })
        .collect()
}

fn crowd(shape: Netlist, seed: u32) -> Vec<u32> {
    let mut state = seed | 1;
    (0..CROWD)
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

fn named(shape: Netlist) -> Vec<u32> {
    (shape.gates - shape.outputs..shape.gates).collect()
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
    let sizes = [
        (CROWD * shape.gates) as usize,
        (CROWD * shape.sources()) as usize,
        shape.outputs as usize,
        CROWD as usize,
    ];
    assert_eq!(sizes.len(), BINDINGS as usize);
    let mut resident = host.resident(&sizes, &modules).unwrap();

    resident.write(GATES as usize, &held).unwrap();
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
