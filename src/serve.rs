use crate::host::Host;
use crate::machine::Machine;
use crate::marking::{
    BEST, BINDINGS, FEED, HELD, SCORE, SEED, TRIAL, VALUES, WANTED, chosen, marked, stepped,
    turned_on, words_of,
};
use crate::netlist::{Chip, Exam, Gate, Netlist, drawn};
use crate::resident::Groups;
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

fn ran(host: Option<&Host>, name: &str, held: &Json) -> Result<Json, Box<dyn std::error::Error>> {
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
