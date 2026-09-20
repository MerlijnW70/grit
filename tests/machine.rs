#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use grit::machine::Machine;
use grit::netlist::{Chip, Gate};

fn counter() -> (Machine, Chip) {
    let held = Machine::over(2, 0, 2, 4, 16, 8).unwrap();
    let shape = held.shape().unwrap();
    let gates = vec![
        Gate {
            kind: 0b0001,
            left: 0,
            right: 0,
        },
        Gate {
            kind: 0b0110,
            left: 0,
            right: 1,
        },
        Gate {
            kind: 0b1000,
            left: 0,
            right: 0,
        },
        Gate {
            kind: 0b1000,
            left: 1,
            right: 1,
        },
    ];
    let packed: Vec<u32> = gates.iter().map(|one| one.pack()).collect();
    let chip = Chip::of(shape, packed, held.named()).unwrap();
    (held, chip)
}

#[test]
fn a_machine_is_a_netlist_that_reads_its_own_last_answer() {
    let held = Machine::over(2, 3, 1, 9, 16, 8).unwrap();
    let shape = held.shape().unwrap();
    assert_eq!(
        shape.inputs, 5,
        "state and inputs together are what it reads"
    );
    assert_eq!(
        shape.outputs, 3,
        "state and outputs together are what it writes"
    );
    assert_eq!(held.marks(), 8, "eight steps of one output");
    assert_eq!(held.named(), vec![6, 7, 8]);
}

#[test]
fn a_counter_built_by_hand_counts() {
    let (held, chip) = counter();
    let said = held.ran(&chip, &[0; 8]);
    assert_eq!(
        said,
        vec![0, 1, 2, 3, 0, 1, 2, 3],
        "it should wrap at four and start again"
    );
}

#[test]
fn a_counter_is_marked_full_against_its_own_trace_and_short_against_another() {
    let (held, chip) = counter();
    let feed = [0_u32; 8];
    let want = [0, 1, 2, 3, 0, 1, 2, 3];
    assert_eq!(held.scored(&chip, &feed, &want), held.marks());

    let wrong = [0, 1, 2, 3, 0, 1, 2, 2];
    let short = held.scored(&chip, &feed, &wrong);
    assert_eq!(
        short,
        held.marks() - 1,
        "one bit of one step differs, so one mark is lost"
    );
}

#[test]
fn a_machine_that_ignores_its_input_says_the_same_whatever_it_is_fed() {
    let (held, chip) = counter();
    let one = held.ran(&chip, &[0; 8]);
    let other = held.ran(&chip, &[7; 8]);
    assert_eq!(one, other, "this counter has no inputs to read");
}

use grit::marking::{BINDINGS, FEED, SCORE, TRIAL, VALUES, WANTED, stepped, words_of};
use grit::netlist::{Netlist, drawn};
use grit::{Groups, Host};

const CROWD: u32 = 256;

const TRACES: u32 = 32;

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

fn drew(shape: Netlist, seed: u32, many: u32) -> Vec<u32> {
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

fn rolled(seed: u32, many: usize) -> Vec<u32> {
    let mut state = seed | 1;
    (0..many)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state
        })
        .collect()
}

fn bit_of(columns: &[u32], at: usize, trace: u32) -> u32 {
    (columns.get(at).copied().unwrap_or(0) >> trace) & 1
}

#[test]
fn a_machine_runs_on_the_device_exactly_as_it_runs_on_the_host() {
    let Some(host) = device("a_machine_runs_on_the_device_exactly_as_it_runs_on_the_host") else {
        return;
    };
    let width = host.width();
    let held = Machine::over(2, 1, 1, 8, 16, 8).unwrap();
    let shape = held.shape().unwrap();
    let wide = TRACES / 32;
    let module = stepped(width, held, wide).unwrap();
    let modules: Vec<&[u32]> = vec![&module];

    let feed = rolled(0x0BAD_C0DE, (held.steps * held.inputs * wide) as usize);
    let want = rolled(0xFEED_FACE, (held.steps * held.outputs * wide) as usize);
    let chips = drew(shape, 0x1234_5678, CROWD);

    let sizes = [
        (CROWD * shape.gates) as usize,
        (CROWD * shape.gates) as usize,
        (CROWD * shape.sources() * words_of(shape).max(wide)) as usize,
        want.len(),
        CROWD as usize,
        CROWD as usize,
        CROWD as usize,
        feed.len(),
    ];
    assert_eq!(sizes.len(), BINDINGS as usize);
    let mut resident = host.resident(&sizes, &modules).unwrap();
    resident.write(TRIAL as usize, &chips).unwrap();
    resident
        .write(VALUES as usize, &vec![0_u32; sizes[VALUES as usize]])
        .unwrap();
    resident.write(WANTED as usize, &want).unwrap();
    resident.write(FEED as usize, &feed).unwrap();
    resident
        .write(SCORE as usize, &vec![0_u32; CROWD as usize])
        .unwrap();

    let over = [Groups::Fixed(CROWD.div_ceil(width))];
    let stages: Vec<&[Groups]> = vec![&over];
    resident.staged(&stages).unwrap();
    let scored = resident.read(SCORE as usize, CROWD as usize).unwrap();

    let names = held.named();
    let mut apart = 0_usize;
    let mut most = 0_u32;
    for who in 0..CROWD as usize {
        let gates = chips[who * shape.gates as usize..(who + 1) * shape.gates as usize].to_vec();
        let chip = Chip::of(shape, gates, names.clone()).unwrap();
        let mut marks = 0_u32;
        for trace in 0..TRACES {
            let mine: Vec<u32> = (0..held.steps)
                .map(|tick| {
                    (0..held.inputs).fold(0_u32, |word, input| {
                        let at = ((tick * held.inputs + input) * wide) as usize;
                        word | (bit_of(&feed, at, trace) << input)
                    })
                })
                .collect();
            let theirs: Vec<u32> = (0..held.steps)
                .map(|tick| {
                    (0..held.outputs).fold(0_u32, |word, output| {
                        let at = ((tick * held.outputs + output) * wide) as usize;
                        word | (bit_of(&want, at, trace) << output)
                    })
                })
                .collect();
            marks += held.scored(&chip, &mine, &theirs);
        }
        most = most.max(marks);
        if scored[who] != marks {
            if apart < 3 {
                eprintln!("machine {who}: device {} host {marks}", scored[who]);
            }
            apart += 1;
        }
    }
    eprintln!(
        "{CROWD} machines of {} gates, {} state bits, {} steps and {TRACES} traces: {} marks each, best {most}",
        shape.gates,
        held.state,
        held.steps,
        held.marks() * TRACES
    );
    assert_eq!(apart, 0, "{apart} of {CROWD} machines scored differently");
}

use grit::marking::{BEST, HELD, SEED, chosen, turned_on};

#[test]
fn a_counter_that_holds_when_it_is_not_told_to_count_is_found_by_searching() {
    let Some(host) =
        device("a_counter_that_holds_when_it_is_not_told_to_count_is_found_by_searching")
    else {
        return;
    };
    let width = host.width();
    let held = Machine::over(2, 1, 2, 12, 16, 16).unwrap();
    let shape = held.shape().unwrap();
    let wide = TRACES / 32;
    let many = 65536_u32;

    let feed = rolled(0x0BAD_C0DE, (held.steps * held.inputs * wide) as usize);
    let mut want = vec![0_u32; (held.steps * held.outputs * wide) as usize];
    for trace in 0..TRACES {
        let mut count = 0_u32;
        for tick in 0..held.steps {
            for output in 0..held.outputs {
                let at = ((tick * held.outputs + output) * wide) as usize;
                if let Some(one) = want.get_mut(at) {
                    *one |= ((count >> output) & 1) << trace;
                }
            }
            let at = (tick * held.inputs * wide) as usize;
            if bit_of(&feed, at, trace) == 1 {
                count = (count + 1) & 3;
            }
        }
    }

    let turn = turned_on(width, shape).unwrap();
    let mark = stepped(width, held, wide).unwrap();
    let pick = chosen(width, shape, 256, many).unwrap();
    let modules: Vec<&[u32]> = vec![&turn, &mark, &pick];
    let sizes = [
        (many * shape.gates) as usize,
        (many * shape.gates) as usize,
        (many * shape.sources() * wide.max(1)) as usize,
        want.len(),
        many as usize,
        many as usize,
        many as usize,
        feed.len().max(1),
    ];
    let mut resident = host.resident(&sizes, &modules).unwrap();
    resident
        .write(HELD as usize, &drew(shape, 0x1234_5678, many))
        .unwrap();
    for binding in [TRIAL, VALUES, SCORE, BEST] {
        resident
            .write(binding as usize, &vec![0_u32; sizes[binding as usize]])
            .unwrap();
    }
    resident.write(WANTED as usize, &want).unwrap();
    resident.write(FEED as usize, &feed).unwrap();
    let seeds: Vec<u32> = (0..many).map(|who| (who * 2_654_435_761) | 1).collect();
    resident.write(SEED as usize, &seeds).unwrap();

    let over = Groups::Fixed(many.div_ceil(width));
    let turning = [over];
    let marking = [over];
    let choosing = [over];
    let stages: Vec<&[Groups]> = vec![&turning, &marking, &choosing];

    let full = held.marks() * TRACES;
    let started = std::time::Instant::now();
    let mut rounds = 0_u32;
    let mut top = 0_u32;
    let mut solved = 0_usize;
    eprintln!(
        "{:>8} {:>8} {:>8} {:>11}",
        "rounds", "best", "solved", "took"
    );
    for _ in 0..40_u32 {
        resident.staged_frames(&stages, 25).unwrap();
        rounds += 25;
        let best = resident.read(BEST as usize, many as usize).unwrap();
        top = best.iter().copied().max().unwrap_or(0);
        solved = best.iter().filter(|one| **one >= full).count();
        eprintln!(
            "{rounds:>8} {top:>8} {solved:>8} {:>11?}",
            started.elapsed()
        );
        if solved > 0 {
            break;
        }
    }
    eprintln!(
        "a machine of {} gates over {TRACES} traces, {full} marks",
        shape.gates
    );

    let chips = resident.read(HELD as usize, sizes[HELD as usize]).unwrap();
    let best = resident.read(BEST as usize, many as usize).unwrap();
    let who = best
        .iter()
        .enumerate()
        .max_by_key(|(_, one)| **one)
        .map_or(0, |(who, _)| who);
    let gates = chips[who * shape.gates as usize..(who + 1) * shape.gates as usize].to_vec();
    let chip = Chip::of(shape, gates, held.named()).unwrap();
    let mine: Vec<u32> = (0..held.steps)
        .map(|tick| bit_of(&feed, (tick * held.inputs * wide) as usize, 0))
        .collect();
    eprintln!(
        "told {mine:?} the best machine says {:?}",
        held.ran(&chip, &mine)
    );
    assert!(
        solved > 0,
        "nothing counted: the best reached {top} of {full}"
    );
}
