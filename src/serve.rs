use crate::board::seated;
use crate::host::Host;
use crate::machine::Machine;
use crate::marking::{
    BEST, BINDINGS, FEED, HELD, SCORE, SEED, TRIAL, VALUES, WANTED, chosen, marked, stepped,
    turned_on, words_of,
};
use crate::netlist::{Chip, Exam, Gate, Netlist, drawn};
use crate::rule::{KNOB, Kind, POINT};
use crate::resident::Groups;
use crate::compact::Plan;
use crate::layout::{BRUSH, GRAND, PARAMS, STATE};
use crate::sand::{AIR, Grid, SAND, WALL, WATER};
use crate::sweep;
use crate::wire::{Json, counted, map, text};

pub const CROWD: u32 = 65536;

pub const SIDE: u32 = 256;

pub const ROUNDS: u32 = 4000;

pub struct Found {
    pub marks: u32,
    pub full: u32,
    pub rounds: u32,
    pub crowd: u32,
    pub gates: Vec<Gate>,
    pub table: Vec<u32>,
    pub wrong: Vec<u32>,
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

#[must_use]
pub fn seeded(many: u32) -> Vec<u32> {
    (0..many)
        .map(|who| who.wrapping_mul(2_654_435_761) | 1)
        .collect()
}

fn room(shape: Netlist, many: u32) -> [usize; BINDINGS as usize] {
    let wide = words_of(shape);
    [
        (many * shape.gates) as usize,
        (many * shape.gates) as usize,
        (many * shape.sources() * wide) as usize,
        (shape.outputs * wide) as usize,
        many as usize,
        many as usize,
        many as usize,
        1,
    ]
}

fn named(shape: Netlist) -> Vec<u32> {
    (shape.gates - shape.outputs..shape.gates).collect()
}

fn judged(shape: Netlist, exam: &Exam, gates: &[u32]) -> Result<Found, Box<dyn std::error::Error>> {
    let chip = Chip::of(shape, gates.to_vec(), named(shape))?;
    let table = chip.table();
    let wrong = (0..shape.words())
        .filter(|row| table.get(*row as usize).copied() != Some(exam.asked(*row)))
        .collect();
    Ok(Found {
        marks: exam.scored(&table),
        full: exam.marks(),
        rounds: 0,
        crowd: 0,
        gates: gates.iter().map(|one| Gate::unpack(*one)).collect(),
        table,
        wrong,
    })
}

pub fn checked(
    shape: Netlist,
    exam: &Exam,
    gates: &[u32],
) -> Result<Found, Box<dyn std::error::Error>> {
    judged(shape, exam, gates)
}

pub fn sought(
    host: &Host,
    shape: Netlist,
    exam: &Exam,
    many: u32,
    rounds: u32,
) -> Result<Found, Box<dyn std::error::Error>> {
    let width = host.width();
    let turn = turned_on(width, shape)?;
    let mark = marked(width, shape)?;
    let pick = chosen(width, shape, SIDE, many)?;
    let modules: Vec<&[u32]> = vec![&turn, &mark, &pick];
    let sizes = room(shape, many);
    let mut resident = host.resident(&sizes, &modules)?;

    resident.write(HELD as usize, &crowd_of(shape, 0x1234_5678, many))?;
    for binding in [TRIAL, VALUES, SCORE, BEST] {
        let held = sizes.get(binding as usize).copied().unwrap_or(0);
        resident.write(binding as usize, &vec![0_u32; held])?;
    }
    resident.write(WANTED as usize, &columns(exam))?;
    let seeds = seeded(many);
    resident.write(SEED as usize, &seeds)?;

    let over = Groups::Fixed(many.div_ceil(width));
    let turning = [over];
    let marking = [over];
    let choosing = [over];
    let stages: Vec<&[Groups]> = vec![&turning, &marking, &choosing];

    let full = exam.marks();
    let batch = 100_u32;
    let mut done = 0_u32;
    let mut best = Vec::new();
    while done < rounds {
        let step = batch.min(rounds - done);
        resident.staged_frames(&stages, step)?;
        done += step;
        best = resident.read(BEST as usize, many as usize)?;
        if best.iter().any(|one| *one >= full) {
            break;
        }
    }
    let held = resident.read(HELD as usize, sizes.first().copied().unwrap_or(0))?;
    let who = best
        .iter()
        .enumerate()
        .max_by_key(|(_, one)| **one)
        .map_or(0, |(who, _)| who);
    let from = who * shape.gates as usize;
    let to = from + shape.gates as usize;
    let gates = held.get(from..to).unwrap_or(&[]);
    let mut found = judged(shape, exam, gates)?;
    found.rounds = done;
    found.crowd = many;
    Ok(found)
}

pub fn pressed(
    host: &Host,
    shape: Netlist,
    exam: &Exam,
    many: u32,
    rounds: u32,
) -> Result<Found, Box<dyn std::error::Error>> {
    let mut crowd = many;
    loop {
        let found = sought(host, shape, exam, crowd, rounds)?;
        if found.marks >= found.full || crowd >= CROWD {
            return Ok(found);
        }
        crowd = crowd.saturating_mul(2).min(CROWD);
    }
}

pub const PATIENCE: u32 = 2;

pub struct Rung {
    pub gates: u32,
    pub found: Found,
}

pub fn least(
    host: &Host,
    shape: Netlist,
    table: &[u32],
    many: u32,
    rounds: u32,
) -> Result<Vec<Rung>, Box<dyn std::error::Error>> {
    let mut rungs = Vec::new();
    let mut gates = shape.gates;
    let mut missed = 0;
    while gates >= shape.outputs.max(1) && missed < PATIENCE {
        let narrowed = Netlist::over(shape.inputs, shape.outputs, gates, shape.kinds)?;
        let exam = Exam::of(narrowed, table.to_vec())?;
        let found = pressed(host, narrowed, &exam, many, rounds)?;
        let solved = found.marks >= found.full;
        rungs.push(Rung { gates, found });
        if solved {
            missed = 0;
        } else {
            missed += 1;
        }
        gates -= 1;
    }
    Ok(rungs)
}

pub type Runs = Vec<(Vec<u32>, Vec<u32>)>;

pub struct Ran {
    pub marks: u32,
    pub full: u32,
    pub rounds: u32,
    pub crowd: u32,
    pub traces: u32,
    pub gates: Vec<Gate>,
    pub said: Vec<Vec<u32>>,
    pub wrong: Vec<(u32, u32)>,
}

fn laid(held: Machine, traces: &[(Vec<u32>, Vec<u32>)], wide: u32) -> (Vec<u32>, Vec<u32>) {
    let many = wide * 32;
    let mut feed = vec![0_u32; (held.steps * held.inputs * wide) as usize];
    let mut want = vec![0_u32; (held.steps * held.outputs * wide) as usize];
    for slot in 0..many {
        let Some((given, wanted)) = traces.get(slot as usize % traces.len().max(1)) else {
            continue;
        };
        let word = slot / 32;
        let bit = slot % 32;
        for tick in 0..held.steps {
            let one = given.get(tick as usize).copied().unwrap_or(0);
            for input in 0..held.inputs {
                let at = ((tick * held.inputs + input) * wide + word) as usize;
                if let Some(held) = feed.get_mut(at) {
                    *held |= ((one >> input) & 1) << bit;
                }
            }
            let one = wanted.get(tick as usize).copied().unwrap_or(0);
            for output in 0..held.outputs {
                let at = ((tick * held.outputs + output) * wide + word) as usize;
                if let Some(held) = want.get_mut(at) {
                    *held |= ((one >> output) & 1) << bit;
                }
            }
        }
    }
    (feed, want)
}

pub fn marked_machine(
    held: Machine,
    traces: &[(Vec<u32>, Vec<u32>)],
    gates: &[u32],
) -> Result<Ran, Box<dyn std::error::Error>> {
    let shape = held.shape()?;
    let chip = Chip::of(shape, gates.to_vec(), held.named())?;
    told_of(held, traces, &chip, gates, 0, 0)
}

fn told_of(
    held: Machine,
    traces: &[(Vec<u32>, Vec<u32>)],
    chip: &Chip,
    gates: &[u32],
    rounds: u32,
    crowd: u32,
) -> Result<Ran, Box<dyn std::error::Error>> {
    let mut marks = 0_u32;
    let mut said = Vec::new();
    let mut wrong = Vec::new();
    for (at, (given, wanted)) in traces.iter().enumerate() {
        let answers = held.ran(chip, given);
        marks += held.scored(chip, given, wanted);
        for (tick, one) in answers.iter().enumerate() {
            if wanted.get(tick).copied() != Some(*one) {
                wrong.push((u32::try_from(at)?, u32::try_from(tick)?));
            }
        }
        said.push(answers);
    }
    Ok(Ran {
        marks,
        full: held.marks() * u32::try_from(traces.len())?,
        rounds,
        crowd,
        traces: u32::try_from(traces.len())?,
        gates: gates.iter().map(|one| Gate::unpack(*one)).collect(),
        said,
        wrong,
    })
}

pub fn machined(
    host: &Host,
    held: Machine,
    traces: &[(Vec<u32>, Vec<u32>)],
    many: u32,
    rounds: u32,
) -> Result<Ran, Box<dyn std::error::Error>> {
    let shape = held.shape()?;
    let width = host.width();
    let wide = u32::try_from(traces.len())?.div_ceil(32).max(1);
    let (feed, want) = laid(held, traces, wide);

    let turn = turned_on(width, shape)?;
    let mark = stepped(width, held, wide)?;
    let pick = chosen(width, shape, SIDE, many)?;
    let modules: Vec<&[u32]> = vec![&turn, &mark, &pick];
    let sizes = [
        (many * shape.gates) as usize,
        (many * shape.gates) as usize,
        (many * shape.sources() * wide) as usize,
        want.len().max(1),
        many as usize,
        many as usize,
        many as usize,
        feed.len().max(1),
    ];
    let mut resident = host.resident(&sizes, &modules)?;
    resident.write(HELD as usize, &crowd_of(shape, 0x1234_5678, many))?;
    for binding in [TRIAL, VALUES, SCORE, BEST] {
        let room = sizes.get(binding as usize).copied().unwrap_or(0);
        resident.write(binding as usize, &vec![0_u32; room])?;
    }
    resident.write(WANTED as usize, &want)?;
    if !feed.is_empty() {
        resident.write(FEED as usize, &feed)?;
    }
    let seeds = seeded(many);
    resident.write(SEED as usize, &seeds)?;

    let over = Groups::Fixed(many.div_ceil(width));
    let turning = [over];
    let marking = [over];
    let choosing = [over];
    let stages: Vec<&[Groups]> = vec![&turning, &marking, &choosing];

    let full = held.marks() * wide * 32;
    let batch = 50_u32;
    let mut done = 0_u32;
    let mut best = Vec::new();
    while done < rounds {
        let step = batch.min(rounds - done);
        resident.staged_frames(&stages, step)?;
        done += step;
        best = resident.read(BEST as usize, many as usize)?;
        if best.iter().any(|one| *one >= full) {
            break;
        }
    }

    let chips = resident.read(HELD as usize, sizes.first().copied().unwrap_or(0))?;
    let who = best
        .iter()
        .enumerate()
        .max_by_key(|(_, one)| **one)
        .map_or(0, |(who, _)| who);
    let from = who * shape.gates as usize;
    let to = from + shape.gates as usize;
    let gates = chips.get(from..to).unwrap_or(&[]).to_vec();
    let chip = Chip::of(shape, gates.clone(), held.named())?;
    told_of(held, traces, &chip, &gates, done, many)
}

pub fn pressed_machine(
    host: &Host,
    held: Machine,
    traces: &[(Vec<u32>, Vec<u32>)],
    many: u32,
    rounds: u32,
) -> Result<Ran, Box<dyn std::error::Error>> {
    let mut crowd = many;
    loop {
        let ran = machined(host, held, traces, crowd, rounds)?;
        if ran.marks >= ran.full || crowd >= CROWD {
            return Ok(ran);
        }
        crowd = crowd.saturating_mul(2).min(CROWD);
    }
}

fn told(found: &Found, sought: bool) -> Json {
    let gates = Json::List(
        found
            .gates
            .iter()
            .map(|one| {
                map(vec![
                    ("kind", counted(one.kind)),
                    ("left", counted(one.left)),
                    ("right", counted(one.right)),
                ])
            })
            .collect(),
    );
    let mut held = vec![
        ("solved", Json::Bool(found.marks >= found.full)),
        ("marks", counted(found.marks)),
        ("of", counted(found.full)),
        ("gates", gates),
        (
            "table",
            Json::List(found.table.iter().map(|one| counted(*one)).collect()),
        ),
        (
            "wrong_rows",
            Json::List(found.wrong.iter().map(|one| counted(*one)).collect()),
        ),
        ("checked_on_the_host", Json::Bool(true)),
    ];
    if sought {
        held.push(("rounds", counted(found.rounds)));
        held.push(("crowd", counted(found.crowd)));
    }
    map(held)
}

fn asked(held: &Json) -> Result<(Netlist, Exam), Box<dyn std::error::Error>> {
    let inputs = held.counted("inputs").unwrap_or(4);
    let outputs = held.counted("outputs").unwrap_or(1);
    let gates = held.counted("gates").unwrap_or(8);
    let shape = Netlist::over(inputs, outputs, gates, 16)?;
    let table: Vec<u32> = held
        .get("table")
        .and_then(Json::list)
        .unwrap_or(&[])
        .iter()
        .filter_map(|one| one.int())
        .filter_map(|one| u32::try_from(one).ok())
        .collect();
    Ok((shape, Exam::of(shape, table)?))
}

fn given(held: &Json, shape: Netlist) -> Result<Vec<u32>, Box<dyn std::error::Error>> {
    let mut gates = Vec::new();
    for one in held.get("chip").and_then(Json::list).unwrap_or(&[]) {
        let gate = Gate {
            kind: one.counted("kind").unwrap_or(0),
            left: one.counted("left").unwrap_or(0),
            right: one.counted("right").unwrap_or(0),
        };
        gates.push(gate.pack());
    }
    if gates.len() != shape.gates as usize {
        return Err(Box::new(crate::netlist::NotAChip::Gates {
            given: gates.len(),
            wanted: shape.gates as usize,
        }));
    }
    Ok(gates)
}

fn tools() -> Json {
    let table = map(vec![
        ("type", text("array")),
        ("items", map(vec![("type", text("integer"))])),
        (
            "description",
            text("one word a row, the wanted output bits, lowest output in bit zero"),
        ),
    ]);
    let counting = |what: &str| map(vec![("type", text("integer")), ("description", text(what))]);
    let shape = vec![
        (
            "inputs",
            counting(
                "how many inputs; the table grows as two to this power, so every added input doubles the rows to mark",
            ),
        ),
        ("outputs", counting("how many outputs")),
        ("gates", counting("how many two-input gates it may use")),
        ("table", table),
    ];
    let mut seeking = shape.clone();
    seeking.push(("crowd", counting("how many candidates to start the search with; while it has not solved, the search doubles this on its own up to 65536, and the crowd in the answer is what it actually spent")));
    seeking.push(("rounds", counting("how many generations to allow")));
    let mut checking = shape;
    checking.push((
        "chip",
        map(vec![
            ("type", text("array")),
            (
                "description",
                text("the gates to check, each an object of kind, left and right"),
            ),
        ]),
    ));
    let held = |what: Vec<(&str, Json)>| {
        map(vec![
            ("type", text("object")),
            ("properties", map(what)),
            (
                "required",
                Json::List(vec![
                    text("inputs"),
                    text("outputs"),
                    text("gates"),
                    text("table"),
                ]),
            ),
        ])
    };
    let mut laddering = checking.clone();
    laddering.retain(|(name, _)| *name != "chip");
    laddering.push(("crowd", counting("how many candidates to start the search with; while it has not solved, the search doubles this on its own up to 65536, and the crowd in the answer is what it actually spent")));
    laddering.push((
        "rounds",
        counting("how many generations to allow at each rung"),
    ));
    let trace = map(vec![
        ("type", text("array")),
        (
            "items",
            map(vec![
                ("type", text("object")),
                (
                    "properties",
                    map(vec![
                        (
                            "feed",
                            map(vec![
                                ("type", text("array")),
                                (
                                    "description",
                                    text("one word a step, the input bits for that step"),
                                ),
                            ]),
                        ),
                        (
                            "want",
                            map(vec![
                                ("type", text("array")),
                                (
                                    "description",
                                    text("one word a step, the output bits wanted at that step"),
                                ),
                            ]),
                        ),
                    ]),
                ),
            ]),
        ),
        (
            "description",
            text("the runs it must get right, each a feed and the answer wanted"),
        ),
    ]);
    let running = vec![
        ("state", counting("how many bits it may remember")),
        ("inputs", counting("how many inputs it reads each step")),
        ("outputs", counting("how many outputs it writes each step")),
        ("gates", counting("how many two-input gates it may use")),
        (
            "steps",
            counting(
                "how many steps a run lasts; taken from the longest run given when it is left out. Steps are worth more than runs: the same count of marks spread over longer runs holds on unseen runs where short ones do not",
            ),
        ),
        ("traces", trace),
        (
            "crowd",
            counting(
                "how many candidates to start the search with; while it has not solved, the search doubles this on its own up to 65536, and the crowd in the answer is what it actually spent",
            ),
        ),
        ("rounds", counting("how many generations to allow")),
        (
            "chip",
            map(vec![
                ("type", text("array")),
                (
                    "description",
                    text(
                        "the gates of a machine to mark, each an object of kind, left and right; give this and nothing is searched, the machine is only run on the traces and marked, which is how a machine found on one set of runs is tried on runs it has never seen",
                    ),
                ),
            ]),
        ),
    ];
    Json::List(vec![
        map(vec![
            ("name", text("grit_machine")),
            (
                "description",
                text(
                    "Search for a synchronous machine, a circuit with state, that answers every run it is given. A run is a feed of input words and the output words wanted at each step; the machine starts from a state of all zeros. Returns the gates, what the best machine says on each run, and which step of which run it gets wrong, all marked on the host. Pass chip to mark a machine you already hold instead of searching for one. Solved means every run in the request is answered, which is not the same as holding the rule behind them: take the gates it returns, pass them back as chip with runs it has never seen, and read the marks. Runs that last longer pin a machine down on far fewer marks than short runs do.",
                ),
            ),
            (
                "inputSchema",
                map(vec![
                    ("type", text("object")),
                    ("properties", map(running)),
                    (
                        "required",
                        Json::List(vec![
                            text("state"),
                            text("inputs"),
                            text("outputs"),
                            text("gates"),
                            text("traces"),
                        ]),
                    ),
                ]),
            ),
        ]),
        map(vec![
            ("name", text("grit_synthesise")),
            (
                "description",
                text(
                    "Search for a circuit of two-input gates that answers a truth table exactly. Returns the circuit and the marks it scores, checked on the host, or the best it reached with the rounds and crowd it spent. While it has not solved, the search doubles the crowd itself up to 65536 before giving an answer, so a result that is not solved is evidence about the gate budget rather than about a crowd the caller happened to pick.",
                ),
            ),
            ("inputSchema", held(seeking)),
        ]),
        map(vec![
            ("name", text("grit_least")),
            (
                "description",
                text(
                    "Walk the gate budget down from the one given, searching at each rung, and keep going past a rung that fails until two in a row fail. Solving is not monotone in the budget: a smaller circuit can be found where a larger one was not, so stopping at the first failure names the edge of a hole rather than the floor. Returns the ladder, which shows any holes, the smallest budget that did solve, and its circuit. Each rung doubles its crowd up to 65536 before it gives up, so a rung that fails has been pressed rather than merely tried. A rung that did not solve means not found within the rounds and crowd spent, which the answer carries, and never that no such circuit exists.",
                ),
            ),
            ("inputSchema", held(laddering)),
        ]),
        map(vec![
            ("name", text("grit_slope")),
            (
                "description",
                text(
                    "Step a grid of cells forward under one of the rules and carry a weight back through the same walk, which is how far the cell you name depends on everything else. Returns where the grid ended, which cells the weight reached and how loudly, how far it travelled, and the two rule gradients as an exact fraction. The walk back is not a second model of the first: the frames are undone and recomputed, so a depth that would not fit on the card as a tape still runs. Nothing is searched and nothing is fitted; this is the derivative, and what to do with it is the caller's.",
                ),
            ),
            (
                "inputSchema",
                map(vec![
                    ("type", text("object")),
                    (
                        "properties",
                        map(vec![
                            ("wide", map(vec![("type", text("integer")), ("description", text("cells across; rounded up to a power of two"))])),
                            ("deep", map(vec![("type", text("integer")), ("description", text("cells down; rounded up to a power of two"))])),
                            (
                                "kind",
                                map(vec![
                                    ("type", text("string")),
                                    ("enum", Json::List(KINDS_NAMED.iter().map(|one| text(one)).collect())),
                                    ("description", text("which rule the cells follow")),
                                ]),
                            ),
                            ("turning", map(vec![("type", text("integer")), ("description", text("the first rule parameter, the one the gradient calls turning"))])),
                            ("swing", map(vec![("type", text("integer")), ("description", text("the second rule parameter"))])),
                            ("depth", map(vec![("type", text("integer")), ("description", text("how many frames to walk"))])),
                            ("at", map(vec![("type", text("integer")), ("description", text("the cell whose weight is carried back; the middle of the grid if left out"))])),
                            ("fill", map(vec![("type", text("integer")), ("description", text("what every cell holds before the named ones are set"))])),
                            (
                                "cells",
                                map(vec![
                                    ("type", text("array")),
                                    ("description", text("the cells to set before the walk, each an object of at and is; everything else takes fill")),
                                    ("items", map(vec![("type", text("object"))])),
                                ]),
                            ),
                            ("carry", map(vec![("type", text("boolean")), ("description", text("false walks forward only and answers without a slope"))])),
                            ("most", map(vec![("type", text("integer")), ("description", text("how many cells to name in each answer, loudest first"))])),
                        ]),
                    ),
                ]),
            ),
        ]),
        map(vec![
            ("name", text("grit_settle")),
            (
                "description",
                text(
                    "Walk a grid forward, see how far it landed from the one you asked for, and move the two rule parameters downhill against that distance. The step is taken from the walk's own adjoint rather than from a guess: the weight carried back is the sign of the residual a cell at a time, so what comes back is the gradient of the distance with respect to the parameters. Returns the trail it walked, the nearest it got, and the parameters that got it there. Distance is counted exactly on the host. A run that did not close means these parameters, at this step and this many rounds, did not get nearer - never that none would.",
                ),
            ),
            (
                "inputSchema",
                map(vec![
                    ("type", text("object")),
                    (
                        "properties",
                        map(vec![
                            ("wide", map(vec![("type", text("integer")), ("description", text("cells across; rounded up to a power of two"))])),
                            ("deep", map(vec![("type", text("integer")), ("description", text("cells down; rounded up to a power of two"))])),
                            (
                                "kind",
                                map(vec![
                                    ("type", text("string")),
                                    ("enum", Json::List(KINDS_NAMED.iter().map(|one| text(one)).collect())),
                                    ("description", text("which rule the cells follow")),
                                ]),
                            ),
                            ("turning", map(vec![("type", text("integer")), ("description", text("where the first rule parameter starts"))])),
                            ("swing", map(vec![("type", text("integer")), ("description", text("where the second rule parameter starts"))])),
                            ("depth", map(vec![("type", text("integer")), ("description", text("how many frames each round walks"))])),
                            ("rounds", map(vec![("type", text("integer")), ("description", text("how many downhill steps to take"))])),
                            ("step", map(vec![("type", text("integer")), ("description", text("how far a parameter moves in one round; they are whole numbers, so this is whole too"))])),
                            ("ceiling", map(vec![("type", text("integer")), ("description", text("how high a parameter may climb"))])),
                            ("fill", map(vec![("type", text("integer")), ("description", text("what every cell holds before the named ones are set, in both grids"))])),
                            (
                                "cells",
                                map(vec![
                                    ("type", text("array")),
                                    ("description", text("the grid to start from, each an object of at and is")),
                                    ("items", map(vec![("type", text("object"))])),
                                ]),
                            ),
                            (
                                "want",
                                map(vec![
                                    ("type", text("array")),
                                    ("description", text("the grid to land on, each an object of at and is; this is what the rounds walk towards")),
                                    ("items", map(vec![("type", text("object"))])),
                                ]),
                            ),
                        ]),
                    ),
                    ("required", Json::List(vec![text("want")])),
                ]),
            ),
        ]),
        map(vec![
            ("name", text("grit_pour")),
            (
                "description",
                text(
                    "Pour sand or water into a room and step it until it settles. Returns how much of each thing ended where, how many cells of sand and water lie in each row, and how many quads the work list actually named on an average look. That last number is the point of the engine rather than a detail of it: the frame costs what is moving, not what the grid could hold, so a settled room is cheap however large it is. Nothing is searched and nothing is judged; this is what happened.",
                ),
            ),
            (
                "inputSchema",
                map(vec![
                    ("type", text("object")),
                    (
                        "properties",
                        map(vec![
                            ("wide", map(vec![("type", text("integer")), ("description", text("cells across; rounded up to a power of two"))])),
                            ("deep", map(vec![("type", text("integer")), ("description", text("cells down; rounded up to a power of two"))])),
                            ("frames", map(vec![("type", text("integer")), ("description", text("how many frames to step"))])),
                            ("walls", map(vec![("type", text("boolean")), ("description", text("a floor and two sides, so falling things land; false lets everything leave by the bottom"))])),
                            (
                                "cells",
                                map(vec![
                                    ("type", text("array")),
                                    ("description", text("what to place before the first frame, each an object of at and is, where is names one of air, water, sand, wall")),
                                    ("items", map(vec![("type", text("object"))])),
                                ]),
                            ),
                            (
                                "brush",
                                map(vec![
                                    ("type", text("object")),
                                    ("description", text("a spout that pours: at names the cell it pours from, is what it pours, span how wide, frames how long it keeps pouring before it shuts off")),
                                ]),
                            ),
                            ("most", map(vec![("type", text("integer")), ("description", text("how many settled cells to name one by one"))])),
                            ("batch", map(vec![("type", text("integer")), ("description", text("how many frames go to the device at a time"))])),
                        ]),
                    ),
                ]),
            ),
        ]),
        map(vec![
            ("name", text("grit_check")),
            (
                "description",
                text(
                    "Mark a given circuit against a truth table and name the rows it gets wrong. Nothing is searched.",
                ),
            ),
            ("inputSchema", held(checking)),
        ]),
    ])
}

fn said(what: &str, wrong: bool) -> Json {
    map(vec![
        (
            "content",
            Json::List(vec![map(vec![
                ("type", text("text")),
                ("text", text(what)),
            ])]),
        ),
        ("isError", Json::Bool(wrong)),
    ])
}

fn answered(id: Json, result: Json) -> Json {
    map(vec![
        ("jsonrpc", text("2.0")),
        ("id", id),
        ("result", result),
    ])
}

fn refused(id: Json, code: i64, what: &str) -> Json {
    map(vec![
        ("jsonrpc", text("2.0")),
        ("id", id),
        (
            "error",
            map(vec![("code", Json::Int(code)), ("message", text(what))]),
        ),
    ])
}

fn asked_machine(held: &Json) -> Result<(Machine, Runs), Box<dyn std::error::Error>> {
    let counted = |name: &str, or: u32| held.counted(name).unwrap_or(or);
    let taken = |one: &Json, name: &str| -> Vec<u32> {
        one.get(name)
            .and_then(Json::list)
            .unwrap_or(&[])
            .iter()
            .filter_map(Json::int)
            .filter_map(|one| u32::try_from(one).ok())
            .collect()
    };
    let traces: Runs = held
        .get("traces")
        .and_then(Json::list)
        .unwrap_or(&[])
        .iter()
        .map(|one| (taken(one, "feed"), taken(one, "want")))
        .collect();
    let steps = traces
        .iter()
        .map(|(_, want)| u32::try_from(want.len()).unwrap_or(0))
        .max()
        .unwrap_or(0);
    let machine = Machine::over(
        counted("state", 2),
        counted("inputs", 1),
        counted("outputs", 1),
        counted("gates", 12),
        16,
        counted("steps", steps),
    )?;
    Ok((machine, traces))
}

fn kind_named(name: &str) -> Option<Kind> {
    Some(match name {
        "product" => Kind::Product,
        "sum" => Kind::Sum,
        "turn" => Kind::Turn,
        "bound" => Kind::Bound,
        "smooth" => Kind::Smooth,
        "living" => Kind::Living,
        "twinned" => Kind::Twinned,
        _ => return None,
    })
}

pub const KINDS_NAMED: [&str; 7] = [
    "product", "sum", "turn", "bound", "smooth", "living", "twinned",
];

/// A cell and what is in it, so neither the start nor the answer has to spell
/// out every cell of the grid. The engine is sparse and so is this.
fn sown(held: &Json, grid: Grid, fill: u32) -> Result<Vec<u32>, Box<dyn std::error::Error>> {
    let mut cells = vec![fill; grid.cells()];
    for one in held.get("cells").and_then(Json::list).unwrap_or(&[]) {
        let at = one.counted("at").ok_or("a cell needs an at")? as usize;
        let value = one.counted("is").unwrap_or(1);
        let held = cells.get_mut(at).ok_or_else(|| {
            format!("at {at} is outside the {} cells of this grid", grid.cells())
        })?;
        *held = value;
    }
    Ok(cells)
}

fn wanted(held: &Json, grid: Grid, fill: u32) -> Result<Vec<u32>, Box<dyn std::error::Error>> {
    let mut cells = vec![fill; grid.cells()];
    for one in held.get("want").and_then(Json::list).unwrap_or(&[]) {
        let at = one.counted("at").ok_or("a wanted cell needs an at")? as usize;
        let value = one.counted("is").unwrap_or(1);
        let held = cells.get_mut(at).ok_or_else(|| {
            format!("at {at} is outside the {} cells of this grid", grid.cells())
        })?;
        *held = value;
    }
    Ok(cells)
}

/// The non-zero entries, loudest first, and no more than `most` of them.
fn loudest(held: &[i32], most: usize) -> (Json, u32) {
    let mut heard: Vec<(usize, i32)> = held
        .iter()
        .enumerate()
        .filter(|(_, one)| **one != 0)
        .map(|(at, one)| (at, *one))
        .collect();
    let count = heard.len() as u32;
    heard.sort_by_key(|(at, one)| (std::cmp::Reverse(one.unsigned_abs()), *at));
    heard.truncate(most);
    (
        Json::List(
            heard
                .iter()
                .map(|(at, one)| {
                    map(vec![
                        ("at", counted(*at as u32)),
                        ("is", Json::Int(i64::from(*one))),
                    ])
                })
                .collect(),
        ),
        count,
    )
}

/// How far from `at` the weight reached, in cells, along the grid.
fn reached(grid: Grid, slope: &[i32], at: usize) -> u32 {
    let named = |one: usize| (one as u32 % grid.wide, one as u32 / grid.wide);
    let (home_x, home_y) = named(at);
    slope
        .iter()
        .enumerate()
        .filter(|(_, one)| **one != 0)
        .map(|(one, _)| {
            let (x, y) = named(one);
            x.abs_diff(home_x).max(y.abs_diff(home_y))
        })
        .max()
        .unwrap_or(0)
}

fn slid(host: &Host, held: &Json) -> Result<Json, Box<dyn std::error::Error>> {
    let wide = held.counted("wide").unwrap_or(32).clamp(2, 256);
    let deep = held.counted("deep").unwrap_or(32).clamp(2, 256);
    let grid = Grid::of(wide, deep);
    let named = held.get("kind").and_then(Json::text).unwrap_or("living");
    let kind = kind_named(named).ok_or_else(|| {
        format!(
            "kind {named} is not one of {}",
            KINDS_NAMED.join(", ")
        )
    })?;
    let turning = held.counted("turning").unwrap_or(KNOB);
    let swing = held.counted("swing").unwrap_or(0);
    let depth = held.counted("depth").unwrap_or(16).clamp(1, 1 << 12);
    let most = held.counted("most").unwrap_or(32).clamp(1, 4096) as usize;
    let middle = (grid.deep / 2 * grid.wide + grid.wide / 2) as usize;
    let at = held.counted("at").unwrap_or(middle as u32) as usize;
    if at >= grid.cells() {
        return Err(format!("at {at} is outside the {} cells of this grid", grid.cells()).into());
    }
    let cells = sown(held, grid, held.counted("fill").unwrap_or(0))?;

    let mut board = seated(host, grid, kind, (turning, swing), depth)?;
    // Json carries no bool accessor, and adding one for this would be a change
    // to the wire format's shape rather than to this tool.
    let carrying = !matches!(held.get("carry"), Some(Json::Bool(false)));
    let (ended, slope) = if carrying {
        let mut weight = vec![0_u32; grid.cells()];
        let seat = weight.get_mut(at).ok_or("at is outside the grid")?;
        *seat = 1 << POINT;
        // carry answers with the weight it carried back first and the grid it
        // ended on second. Both are as many u32 as there are cells, so taking
        // them the other way round compiles and quietly reports each as the
        // other: the grid looked like a slope that reached every cell in one
        // frame, and the slope looked like a grid.
        let (slope, ended) = board.carry(&cells, &weight)?;
        let slope: Vec<i32> = slope.iter().map(|one| *one as i32).collect();
        (ended, Some(slope))
    } else {
        (board.run(&cells)?, None)
    };

    let settled: Vec<i32> = ended.iter().map(|one| *one as i32).collect();
    let (shown, standing) = loudest(&settled, most);
    let mut answer = vec![
        (
            "grid",
            map(vec![
                ("wide", counted(grid.wide)),
                ("deep", counted(grid.deep)),
                ("cells", counted(grid.cells() as u32)),
            ]),
        ),
        ("kind", text(named)),
        ("depth", counted(depth)),
        ("at", counted(at as u32)),
        ("ended", shown),
        ("standing", counted(standing)),
        ("checked_on_the_host", Json::Bool(true)),
    ];
    if let Some(slope) = slope {
        let (shown, heard) = loudest(&slope, most);
        let (turning, swing, over) = board.told_parts()?;
        answer.push(("slope", shown));
        answer.push(("heard", counted(heard)));
        answer.push(("reach", counted(reached(grid, &slope, at))));
        answer.push((
            "told",
            map(vec![
                ("turning", Json::Int(turning)),
                ("swing", Json::Int(swing)),
                ("over", Json::Int(over)),
                (
                    "read_it_as",
                    text("turning and swing are each divided by over; they are kept apart so nothing is rounded here"),
                ),
            ]),
        ));
        answer.push((
            "heard_means",
            text("how many cells the weight at `at` reached after `depth` frames, which is what that cell's future is made of"),
        ));
    }
    Ok(map(answer))
}

/// One cell's worth of push, big enough that the weight does not round away.
const PUSH: i32 = 1 << POINT;

/// How far apart two grids are, counted a cell at a time.
fn apart(ended: &[u32], want: &[u32]) -> i64 {
    ended
        .iter()
        .zip(want)
        .map(|(one, other)| i64::from(*one as i32 - *other as i32).abs())
        .sum()
}

/// The subgradient of that distance: which way each cell wants to move.
///
/// This is what makes the walk back do anything useful. `carry` takes any
/// weight, not only a single cell lit up, so seeding it with the sign of the
/// residual makes `told` the gradient of the distance above with respect to
/// the two rule parameters. Nothing here is a model of the simulation; it is
/// the simulation's own adjoint, asked a different question.
fn pushed(ended: &[u32], want: &[u32]) -> Vec<u32> {
    ended
        .iter()
        .zip(want)
        .map(|(one, other)| {
            let gap = *one as i32 - *other as i32;
            (match gap.signum() {
                1 => PUSH,
                -1 => -PUSH,
                _ => 0,
            }) as u32
        })
        .collect()
}

/// Downhill, by whole numbers, because the parameters are whole numbers.
/// `against` turns the step around, for when the slope pointed the wrong way.
fn moved(now: u32, slope: i64, step: u32, most: u32, against: bool) -> u32 {
    let down = if against { -slope.signum() } else { slope.signum() };
    match down {
        1 => now.saturating_sub(step),
        -1 => now.saturating_add(step).min(most),
        _ => now,
    }
}

fn settled(host: &Host, held: &Json) -> Result<Json, Box<dyn std::error::Error>> {
    let wide = held.counted("wide").unwrap_or(32).clamp(2, 256);
    let deep = held.counted("deep").unwrap_or(32).clamp(2, 256);
    let grid = Grid::of(wide, deep);
    let named = held.get("kind").and_then(Json::text).unwrap_or("living");
    let kind =
        kind_named(named).ok_or_else(|| format!("kind {named} is not one of {}", KINDS_NAMED.join(", ")))?;
    let depth = held.counted("depth").unwrap_or(16).clamp(1, 1 << 12);
    let rounds = held.counted("rounds").unwrap_or(8).clamp(1, 1 << 10);
    let step = held.counted("step").unwrap_or(1).clamp(1, 1 << 16);
    let ceiling = held.counted("ceiling").unwrap_or(1 << 14);
    let fill = held.counted("fill").unwrap_or(0);
    let cells = sown(held, grid, fill)?;
    let want = wanted(held, grid, fill)?;

    let mut turning = held.counted("turning").unwrap_or(KNOB).min(ceiling);
    let mut swing = held.counted("swing").unwrap_or(0).min(ceiling);
    let mut board = seated(host, grid, kind, (turning, swing), depth)?;

    let mut trail = Vec::new();
    // The gap it reached, where it was, and which way it was pointing there.
    // The slope is kept with the point because a round that overshoots goes
    // back to the best one and tries again shorter, and it must not re-walk
    // the grid to remember which way downhill was.
    let mut best: Option<(i64, u32, u32, i64, i64)> = None;
    let mut step = step;
    let mut stalled = false;
    // Measured on a target the rule itself made: the distance falls all the way
    // to zero at the parameters that made it, and the walk back agrees about
    // which way that is over most of the range and disagrees over part of it.
    // The distance is exact and the slope is a hint, so a round that went the
    // wrong way tries the other way before it gives up any step.
    let mut against = false;
    let mut turned_around = 0_u32;
    let mut unmoved = 0_u32;
    for _ in 0..rounds {
        board.tune(turning, swing)?;
        // Forward once to see where it lands, then again inside carry, which
        // cannot know the residual before the grid has been walked.
        let ended = board.run(&cells)?;
        let gap = apart(&ended, &want);
        let (_, walked) = board.carry(&cells, &pushed(&ended, &want))?;
        let (down_turning, down_swing, over) = board.told_parts()?;
        trail.push(map(vec![
            ("turning", counted(turning)),
            ("swing", counted(swing)),
            ("apart", Json::Int(gap)),
            ("down_turning", Json::Int(down_turning)),
            ("down_swing", Json::Int(down_swing)),
            ("over", Json::Int(over)),
        ]));
        let _ = walked;
        if trail
            .last()
            .and_then(|one| one.get("apart"))
            .and_then(Json::int)
            .is_some_and(|held| held == gap)
        {
            unmoved += 1;
        }
        let (from_turning, from_swing, downhill) =
            if best.is_none_or(|(held, _, _, _, _)| gap < held) {
                best = Some((gap, turning, swing, down_turning, down_swing));
                (turning, swing, (down_turning, down_swing))
            } else if !against {
                against = true;
                turned_around += 1;
                match best {
                    Some((_, held_turning, held_swing, held_down, held_swung)) => {
                        (held_turning, held_swing, (held_down, held_swung))
                    }
                    None => {
                        stalled = true;
                        break;
                    }
                }
            } else {
                // Both ways were worse, so the step is too long. Sign-only steps
                // on whole numbers overshoot and then sit swinging between two
                // values; going back with half the step turns that into an answer.
                against = false;
                step /= 2;
                match best {
                    Some((_, held_turning, held_swing, held_down, held_swung)) if step > 0 => {
                        (held_turning, held_swing, (held_down, held_swung))
                    }
                    _ => {
                        stalled = true;
                        break;
                    }
                }
            };
        let moved_turning = moved(from_turning, downhill.0, step, ceiling, against);
        let moved_swing = moved(from_swing, downhill.1, step, ceiling, against);
        if moved_turning == from_turning && moved_swing == from_swing {
            stalled = true;
            break;
        }
        turning = moved_turning;
        swing = moved_swing;
    }

    let (gap, turning, swing, _, _) = best.ok_or("no round was walked")?;
    let first = trail.first().and_then(|one| one.get("apart")).and_then(Json::int);
    Ok(map(vec![
        (
            "grid",
            map(vec![
                ("wide", counted(grid.wide)),
                ("deep", counted(grid.deep)),
                ("cells", counted(grid.cells() as u32)),
            ]),
        ),
        ("kind", text(named)),
        ("depth", counted(depth)),
        ("rounds", counted(trail.len() as u32)),
        ("turning", counted(turning)),
        ("swing", counted(swing)),
        ("apart", Json::Int(gap)),
        ("started_apart", first.map_or(Json::Null, Json::Int)),
        ("closed", Json::Bool(first.is_some_and(|one| gap < one))),
        ("stalled", Json::Bool(stalled)),
        ("turned_around", counted(turned_around)),
        ("unmoved", counted(unmoved)),
        ("step", counted(step)),
        ("trail", Json::List(trail)),
        ("checked_on_the_host", Json::Bool(true)),
        (
            "apart_means",
            text("how far the walked grid is from the one asked for, counted a cell at a time; it is the thing the rounds walk downhill, and it is exact"),
        ),
        (
            "a_round_that_did_not_close",
            text("means these rule parameters, at this step and this many rounds, did not get nearer - not that no parameters would. Stalled means the step had nowhere smaller to go, so more rounds would not help; a step that ran out of rounds instead would. turned_around counts the rounds where the slope pointed uphill and the distance, which is exact, said otherwise. unmoved counts rounds where moving the parameters changed the distance by nothing at all, which means they have no purchase at this depth and these cell values rather than that they are right."),
        ),
    ]))
}

pub const STUFF_NAMED: [&str; 4] = ["air", "water", "sand", "wall"];

fn stuff_named(name: &str) -> Option<u32> {
    match name {
        "air" => Some(AIR),
        "water" => Some(WATER),
        "sand" => Some(SAND),
        "wall" => Some(WALL),
        _ => None,
    }
}

/// A room with a floor and two sides, which is what a falling thing needs to
/// land on. Without it everything leaves by the bottom and the answer is empty.
fn walled(grid: Grid, walls: bool) -> Vec<u32> {
    let mut cells = vec![AIR; grid.cells()];
    if !walls {
        return cells;
    }
    for row in 0..grid.deep {
        for column in 0..grid.wide {
            let edge = row + 1 >= grid.deep || column == 0 || column + 1 >= grid.wide;
            if edge
                && let Some(slot) = cells.get_mut((row * grid.wide + column) as usize)
            {
                *slot = WALL;
            }
        }
    }
    cells
}

fn poured(host: &Host, held: &Json) -> Result<Json, Box<dyn std::error::Error>> {
    let width = host.width();
    let wide = held.counted("wide").unwrap_or(128).clamp(16, 1024);
    let deep = held.counted("deep").unwrap_or(128).clamp(16, 1024);
    let grid = Grid::of(wide, deep);
    let frames = held.counted("frames").unwrap_or(200).clamp(1, 1 << 14);
    let batch = held.counted("batch").unwrap_or(20).clamp(1, 256);
    let most = held.counted("most").unwrap_or(64).clamp(1, 1 << 14) as usize;
    let walls = !matches!(held.get("walls"), Some(Json::Bool(false)));

    let quads = grid.quads();
    let plan = Plan::over(width, quads.div_ceil(sweep::cells(width)))?;
    let (passes, stages) = crate::live::wired(width, grid, &plan, None)?;
    let groups: Vec<&[Groups]> = stages.iter().map(Vec::as_slice).collect();
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let sizes = plan.sizes();
    let mut resident = host.resident(&sizes, &modules)?;
    for (binding, words) in sizes.iter().enumerate() {
        resident.write(binding, &vec![0_u32; *words])?;
    }

    let mut cells = walled(grid, walls);
    for one in held.get("cells").and_then(Json::list).unwrap_or(&[]) {
        let at = one.counted("at").ok_or("a cell needs an at")? as usize;
        let named = one.get("is").and_then(Json::text).unwrap_or("sand");
        let stuff = stuff_named(named)
            .ok_or_else(|| format!("{named} is not one of {}", STUFF_NAMED.join(", ")))?;
        let slot = cells
            .get_mut(at)
            .ok_or_else(|| format!("at {at} is outside the {} cells", grid.cells()))?;
        *slot = stuff;
    }

    resident.write(STATE as usize, &cells)?;
    resident.write(PARAMS as usize, &[u32::MAX, plan.dump()])?;
    crate::live::seed(&mut resident, grid, &plan)?;
    resident.staged(&groups)?;

    // The brush is what pours. Left out, whatever the caller placed is all
    // there is, and the answer is about how that settles.
    let brush = held.get("brush");
    let spout = brush.and_then(|one| one.counted("at")).map(|at| {
        (at % grid.wide, at / grid.wide)
    });
    let span = brush.and_then(|one| one.counted("span")).unwrap_or(49);
    let pouring = brush
        .and_then(|one| one.counted("frames"))
        .unwrap_or(frames / 2)
        .min(frames);
    let named = brush
        .and_then(|one| one.get("is"))
        .and_then(Json::text)
        .unwrap_or("sand");
    let stuff = stuff_named(named)
        .ok_or_else(|| format!("{named} is not one of {}", STUFF_NAMED.join(", ")))?;

    let mut listed = 0_u64;
    let mut looked = 0_u64;
    for frame in (0..frames).step_by(batch as usize) {
        if let Some((x, y)) = spout {
            let material = if frame < pouring { stuff } else { AIR };
            resident.write_at(PARAMS as usize, BRUSH, &[x, y, span, material])?;
        }
        resident.staged_frames(&groups, batch)?;
        listed += u64::from(
            resident
                .read(GRAND as usize, 1)?
                .first()
                .copied()
                .unwrap_or(0),
        );
        looked += 1;
    }

    let ended = resident.read(STATE as usize, grid.cells())?;
    let mut tally = [0_u64; 4];
    for one in &ended {
        if let Some(slot) = tally.get_mut(*one as usize) {
            *slot += 1;
        }
    }
    // Where the settled stuff lies, a row at a time. A pile and a puddle hold
    // the same number of cells and do not look the same lying down.
    let rows: Vec<Json> = (0..grid.deep)
        .map(|row| {
            let held = (0..grid.wide)
                .filter(|column| {
                    ended
                        .get((row * grid.wide + column) as usize)
                        .is_some_and(|one| *one == SAND || *one == WATER)
                })
                .count();
            counted(held as u32)
        })
        .collect();
    // Rows say how it lies down and cannot say how many heaps there are: two
    // piles and one wide one fill the same rows. Columns can.
    let columns: Vec<Json> = (0..grid.wide)
        .map(|column| {
            let held = (0..grid.deep)
                .filter(|row| {
                    ended
                        .get((row * grid.wide + column) as usize)
                        .is_some_and(|one| *one == SAND || *one == WATER)
                })
                .count();
            counted(held as u32)
        })
        .collect();
    let standing: Vec<(usize, u32)> = ended
        .iter()
        .enumerate()
        .filter(|(_, one)| **one != AIR && **one != WALL)
        .map(|(at, one)| (at, *one))
        .take(most)
        .collect();

    Ok(map(vec![
        (
            "grid",
            map(vec![
                ("wide", counted(grid.wide)),
                ("deep", counted(grid.deep)),
                ("cells", counted(grid.cells() as u32)),
            ]),
        ),
        ("frames", counted(frames)),
        (
            "tally",
            map(vec![
                ("air", Json::Int(tally.first().copied().unwrap_or(0) as i64)),
                ("water", Json::Int(tally.get(1).copied().unwrap_or(0) as i64)),
                ("sand", Json::Int(tally.get(2).copied().unwrap_or(0) as i64)),
                ("wall", Json::Int(tally.get(3).copied().unwrap_or(0) as i64)),
            ]),
        ),
        ("rows", Json::List(rows)),
        ("columns", Json::List(columns)),
        (
            "standing",
            Json::List(
                standing
                    .iter()
                    .map(|(at, one)| {
                        map(vec![
                            ("at", counted(*at as u32)),
                            (
                                "is",
                                text(STUFF_NAMED.get(*one as usize).copied().unwrap_or("air")),
                            ),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "listed",
            Json::Int((listed / looked.max(1)) as i64),
        ),
        ("of_quads", counted(quads)),
        ("checked_on_the_host", Json::Bool(true)),
        (
            "listed_means",
            text("how many quads the work list named on an average look, out of of_quads in the whole grid; the engine steps what the list names, so this is what the frame actually cost rather than how big the grid is"),
        ),
        (
            "rows_means",
            text("how many cells of sand or water lie in each row, top row first, and in each column left to right; a pile and a puddle hold the same count and do not lie down the same way, and two heaps and one wide one fill the same rows while the columns between them are empty"),
        ),
    ]))
}

fn ran(host: Option<&Host>, name: &str, held: &Json) -> Result<Json, Box<dyn std::error::Error>> {
    if name == "grit_slope" {
        let Some(host) = host else {
            return Err(Box::new(crate::host::Error::NoComputeDevice));
        };
        return slid(host, held);
    }
    if name == "grit_pour" {
        let Some(host) = host else {
            return Err(Box::new(crate::host::Error::NoComputeDevice));
        };
        return poured(host, held);
    }
    if name == "grit_settle" {
        let Some(host) = host else {
            return Err(Box::new(crate::host::Error::NoComputeDevice));
        };
        return settled(host, held);
    }
    if name == "grit_machine" {
        let (machine, traces) = asked_machine(held)?;
        let steps = machine.steps;
        let told = if held.get("chip").is_some() {
            let shape = machine.shape()?;
            marked_machine(machine, &traces, &given(held, shape)?)?
        } else {
            let Some(host) = host else {
                return Err(Box::new(crate::host::Error::NoComputeDevice));
            };
            let many = held.counted("crowd").unwrap_or(CROWD).clamp(64, 1 << 20);
            let rounds = held.counted("rounds").unwrap_or(ROUNDS).clamp(1, 1 << 20);
            pressed_machine(host, machine, &traces, many, rounds)?
        };
        return Ok(map(vec![
            ("solved", Json::Bool(told.marks >= told.full)),
            ("marks", counted(told.marks)),
            ("of", counted(told.full)),
            ("traces", counted(told.traces)),
            ("steps", counted(steps)),
            (
                "solved_means",
                text(
                    "every run in this request is answered. A machine can answer every run it was handed and still miss runs it has never seen; pass it back as chip with fresh runs to find out. Longer runs pin a machine down on far fewer marks than short ones do.",
                ),
            ),
            ("rounds", counted(told.rounds)),
            ("crowd", counted(told.crowd)),
            (
                "gates",
                Json::List(
                    told.gates
                        .iter()
                        .map(|one| {
                            map(vec![
                                ("kind", counted(one.kind)),
                                ("left", counted(one.left)),
                                ("right", counted(one.right)),
                            ])
                        })
                        .collect(),
                ),
            ),
            (
                "said",
                Json::List(
                    told.said
                        .iter()
                        .map(|one| Json::List(one.iter().map(|one| counted(*one)).collect()))
                        .collect(),
                ),
            ),
            (
                "wrong",
                Json::List(
                    told.wrong
                        .iter()
                        .map(|(trace, step)| {
                            map(vec![("trace", counted(*trace)), ("step", counted(*step))])
                        })
                        .collect(),
                ),
            ),
            ("checked_on_the_host", Json::Bool(true)),
        ]));
    }
    let (shape, exam) = asked(held)?;
    match name {
        "grit_check" => {
            let gates = given(held, shape)?;
            Ok(told(&checked(shape, &exam, &gates)?, false))
        }
        "grit_synthesise" => {
            let Some(host) = host else {
                return Err(Box::new(crate::host::Error::NoComputeDevice));
            };
            let many = held.counted("crowd").unwrap_or(CROWD).clamp(64, 1 << 20);
            let rounds = held.counted("rounds").unwrap_or(ROUNDS).clamp(1, 1 << 20);
            Ok(told(&pressed(host, shape, &exam, many, rounds)?, true))
        }
        "grit_least" => {
            let Some(host) = host else {
                return Err(Box::new(crate::host::Error::NoComputeDevice));
            };
            let many = held.counted("crowd").unwrap_or(CROWD).clamp(64, 1 << 20);
            let rounds = held.counted("rounds").unwrap_or(ROUNDS).clamp(1, 1 << 20);
            let table: Vec<u32> = (0..shape.words()).map(|row| exam.asked(row)).collect();
            let rungs = least(host, shape, &table, many, rounds)?;
            let smallest = rungs
                .iter()
                .filter(|rung| rung.found.marks >= rung.found.full)
                .map(|rung| rung.gates)
                .min();
            let ladder = Json::List(
                rungs
                    .iter()
                    .map(|rung| {
                        map(vec![
                            ("gates", counted(rung.gates)),
                            ("solved", Json::Bool(rung.found.marks >= rung.found.full)),
                            ("marks", counted(rung.found.marks)),
                            ("of", counted(rung.found.full)),
                            ("rounds", counted(rung.found.rounds)),
                        ])
                    })
                    .collect(),
            );
            let best = rungs
                .iter()
                .filter(|rung| rung.found.marks >= rung.found.full)
                .min_by_key(|rung| rung.gates)
                .map_or(Json::Null, |rung| told(&rung.found, true));
            let mut held = vec![
                ("smallest_that_solved", smallest.map_or(Json::Null, counted)),
                ("ladder", ladder),
                ("circuit", best),
                ("crowd", counted(many)),
                ("rounds_each", counted(rounds)),
                (
                    "a_rung_that_did_not_solve",
                    text(
                        "means not found within these rounds and the crowd the rung reached after doubling, not that none exists",
                    ),
                ),
            ];
            if smallest.is_none() {
                if let Some(rung) = rungs.iter().max_by_key(|rung| rung.found.marks) {
                    held.push(("best_attempt", told(&rung.found, true)));
                }
                held.push((
                    "nothing_solved",
                    text(
                        "the ladder ends after two rungs in a row fail, so starting on a budget that cannot be met leaves no circuit; start the ladder from more gates, or raise crowd, and ask again",
                    ),
                ));
            }
            Ok(map(held))
        }
        other => Err(Box::new(crate::netlist::TooWide::Kinds {
            kinds: other.len() as u32,
            limit: 0,
        })),
    }
}

#[must_use]
pub fn handle(host: Option<&Host>, request: &Json) -> Option<Json> {
    let way = request.get("method").and_then(Json::text).unwrap_or("");
    let id = request.get("id").cloned()?;
    match way {
        "initialize" => Some(answered(
            id,
            map(vec![
                ("protocolVersion", text("2024-11-05")),
                ("capabilities", map(vec![("tools", map(vec![]))])),
                (
                    "serverInfo",
                    map(vec![("name", text("grit")), ("version", text("0.0.0"))]),
                ),
            ]),
        )),
        "tools/list" => Some(answered(id, map(vec![("tools", tools())]))),
        "tools/call" => {
            let held = request.get("params");
            let name = held
                .and_then(|one| one.get("name"))
                .and_then(Json::text)
                .unwrap_or("");
            let arguments = held
                .and_then(|one| one.get("arguments"))
                .cloned()
                .unwrap_or(Json::Map(Vec::new()));
            match ran(host, name, &arguments) {
                Ok(result) => Some(answered(
                    id,
                    map(vec![
                        (
                            "content",
                            Json::List(vec![map(vec![
                                ("type", text("text")),
                                ("text", text(&crate::wire::written(&result))),
                            ])]),
                        ),
                        ("isError", Json::Bool(false)),
                        ("structuredContent", result),
                    ]),
                )),
                Err(why) => Some(answered(id, said(&why.to_string(), true))),
            }
        }
        other => Some(refused(id, -32601, other)),
    }
}
