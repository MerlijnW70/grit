use std::fmt;

use crate::law::limits;
use crate::machine::Machine;
use crate::netlist::{Chip, Gate, NotAChip, TooWide};
use crate::wire::{self, Json};

pub const KINDS: u32 = 16;

pub const BITS: u32 = 32;

pub const READS: &str = "bind_inputs";

pub const WRITES: &str = "bind_outputs";

pub const KEEPS: &str = "bind_state";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Read {
    pub plane: u32,
    pub at: [i64; 3],
    pub bit: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spot {
    pub plane: u32,
    pub bit: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rest {
    Kept,
    Cleared,
}

impl Rest {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Kept => "kept",
            Self::Cleared => "cleared",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Circuit {
    pub state: u32,
    pub inputs: u32,
    pub outputs: u32,
    pub gates: Vec<Gate>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wiring {
    pub name: String,
    pub channels: u32,
    pub reads: Vec<Read>,
    pub writes: Vec<Spot>,
    pub keeps: Vec<Spot>,
    pub rest: Rest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    Chip,
    Channels,
    Unbound,
    Overbound,
    Plane,
    Bit,
    Offset,
    Shared,
    Written,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refused {
    pub fault: Fault,
    pub path: String,
    pub message: String,
}

impl fmt::Display for Refused {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(out, "{}", self.message)
    }
}

impl std::error::Error for Refused {}

fn refused(fault: Fault, path: String, message: String) -> Refused {
    Refused {
        fault,
        path,
        message,
    }
}

#[derive(Clone, Debug)]
pub struct Wired {
    chip: Chip,
    machine: Machine,
    circuit: Circuit,
    wiring: Wiring,
}

fn chip_path(why: &NotAChip) -> String {
    match why {
        NotAChip::Gate { at, .. } => format!("$.gates[{at}]"),
        _ => String::from("$.gates"),
    }
}

fn too_wide_path(why: &TooWide) -> &'static str {
    match why {
        TooWide::Inputs { .. } => "$.inputs",
        TooWide::Answers { .. } => "$.outputs",
        _ => "$.gates",
    }
}

fn parsed(circuit: &Circuit) -> Result<(Machine, Chip), Refused> {
    let gates = u32::try_from(circuit.gates.len()).unwrap_or(u32::MAX);
    let machine = Machine::over(
        circuit.state,
        circuit.inputs,
        circuit.outputs,
        gates,
        KINDS,
        0,
    )
    .map_err(|why| {
        refused(
            Fault::Chip,
            too_wide_path(&why).to_owned(),
            format!(
                "the chip of {} state bits, {} inputs, {} outputs and {gates} gates does not parse: {why}",
                circuit.state, circuit.inputs, circuit.outputs
            ),
        )
    })?;
    let shape = machine.shape().map_err(|why| {
        refused(
            Fault::Chip,
            too_wide_path(&why).to_owned(),
            format!("the chip does not parse: {why}"),
        )
    })?;
    let packed: Vec<u32> = circuit.gates.iter().map(|gate| gate.pack()).collect();
    for (at, gate) in circuit.gates.iter().enumerate() {
        let at = u32::try_from(at).unwrap_or(u32::MAX);
        if let Err(fault) = shape.checked(at, *gate) {
            let why = NotAChip::Gate { at, fault };
            return Err(refused(
                Fault::Chip,
                chip_path(&why),
                format!("the chip does not parse: {why}"),
            ));
        }
    }
    let chip = Chip::of(shape, packed, machine.named()).map_err(|why| {
        refused(
            Fault::Chip,
            chip_path(&why),
            format!("the chip does not parse: {why}"),
        )
    })?;
    Ok((machine, chip))
}

fn counted(list: &str, given: usize, wanted: u32, what: &str) -> Result<(), Refused> {
    let wanted = wanted as usize;
    if given < wanted {
        return Err(refused(
            Fault::Unbound,
            format!("$.{list}"),
            format!(
                "{list} holds {given} bindings, and the chip has {wanted} {what}; {what} {given} to {} have no binding",
                wanted - 1
            ),
        ));
    }
    if given > wanted {
        return Err(refused(
            Fault::Overbound,
            format!("$.{list}[{wanted}]"),
            format!(
                "{list} holds {given} bindings, and the chip has only {wanted} {what}; give exactly one binding for each"
            ),
        ));
    }
    Ok(())
}

fn planed(list: &str, at: usize, plane: u32, channels: u32) -> Result<(), Refused> {
    if plane >= channels {
        return Err(refused(
            Fault::Plane,
            format!("$.{list}[{at}].plane"),
            format!(
                "{list}[{at}] names plane {plane}, past the {channels} channels, planes 0 to {}",
                channels - 1
            ),
        ));
    }
    Ok(())
}

fn bitted(list: &str, at: usize, bit: u32) -> Result<(), Refused> {
    if bit >= BITS {
        return Err(refused(
            Fault::Bit,
            format!("$.{list}[{at}].bit"),
            format!(
                "{list}[{at}] names bit {bit}, past the {BITS} bits of a word, bits 0 to {}",
                BITS - 1
            ),
        ));
    }
    Ok(())
}

const AXES: [&str; 3] = ["dx", "dy", "dz"];

fn checked(circuit: &Circuit, wiring: &Wiring) -> Result<(), Refused> {
    if wiring.channels == 0 || wiring.channels > limits::CHANNELS {
        return Err(refused(
            Fault::Channels,
            String::from("$.channels"),
            format!(
                "channels is {}, and a law has 1 to {} planes",
                wiring.channels,
                limits::CHANNELS
            ),
        ));
    }
    counted(READS, wiring.reads.len(), circuit.inputs, "inputs")?;
    counted(WRITES, wiring.writes.len(), circuit.outputs, "outputs")?;
    counted(KEEPS, wiring.keeps.len(), circuit.state, "state bits")?;
    for (at, read) in wiring.reads.iter().enumerate() {
        planed(READS, at, read.plane, wiring.channels)?;
        bitted(READS, at, read.bit)?;
        for (axis, step) in AXES.iter().zip(read.at) {
            if !(-1..=1).contains(&step) {
                return Err(refused(
                    Fault::Offset,
                    format!("$.{READS}[{at}].{axis}"),
                    format!(
                        "{READS}[{at}] reads {axis} {step}, and a binding reads a cell at most one step away on each axis, -1, 0 or 1"
                    ),
                ));
            }
        }
    }
    let spots = wiring
        .writes
        .iter()
        .enumerate()
        .map(|(at, spot)| (WRITES, at, spot))
        .chain(
            wiring
                .keeps
                .iter()
                .enumerate()
                .map(|(at, spot)| (KEEPS, at, spot)),
        );
    let mut taken: Vec<(Spot, String)> = Vec::new();
    for (list, at, spot) in spots {
        planed(list, at, spot.plane, wiring.channels)?;
        bitted(list, at, spot.bit)?;
        if let Some((_, first)) = taken.iter().find(|(held, _)| held == spot) {
            return Err(refused(
                Fault::Shared,
                format!("$.{list}[{at}]"),
                format!(
                    "{list}[{at}] writes bit {} of plane {}, which {first} already writes; each output and state bit needs a bit of its own",
                    spot.bit, spot.plane
                ),
            ));
        }
        taken.push((*spot, format!("{list}[{at}]")));
    }
    for (at, read) in wiring.reads.iter().enumerate() {
        if read.at != [0, 0, 0] && taken.iter().any(|(spot, _)| spot.plane == read.plane) {
            return Err(refused(
                Fault::Written,
                format!("$.{READS}[{at}]"),
                format!(
                    "{READS}[{at}] reads plane {} at offset {:?}, a plane the chip writes; a neighbour on a written plane changes in the same frame, so a binding reads a written plane only at offset 0, its own cell",
                    read.plane, read.at
                ),
            ));
        }
    }
    Ok(())
}

fn op(name: &str, args: Vec<Json>) -> Json {
    let mut held = Vec::with_capacity(args.len() + 1);
    held.push(wire::text(name));
    held.extend(args);
    Json::List(held)
}

fn word(value: u32) -> Json {
    Json::Int(i64::from(value))
}

fn named(name: &str) -> Json {
    wire::text(name)
}

fn bit_of(held: Json, bit: u32) -> Json {
    let shifted = if bit == 0 {
        held
    } else {
        op("shr", vec![held, word(bit)])
    };
    op("and", vec![shifted, word(1)])
}

fn not(held: Json) -> Json {
    op("xor", vec![held, word(1)])
}

#[must_use]
pub fn gate_expression(kind: u32, left: Json, right: Json) -> Json {
    match kind {
        0 => word(0),
        1 => not(op("or", vec![left, right])),
        2 => op("and", vec![left, not(right)]),
        3 => not(right),
        4 => op("and", vec![not(left), right]),
        5 => not(left),
        6 => op("xor", vec![left, right]),
        7 => not(op("and", vec![left, right])),
        8 => op("and", vec![left, right]),
        9 => not(op("xor", vec![left, right])),
        10 => left,
        11 => op("or", vec![left, not(right)]),
        12 => right,
        13 => op("or", vec![not(left), right]),
        14 => op("or", vec![left, right]),
        _ => word(1),
    }
}

impl Wired {
    pub fn new(circuit: Circuit, wiring: Wiring) -> Result<Self, Refused> {
        let (machine, chip) = parsed(&circuit)?;
        checked(&circuit, &wiring)?;
        Ok(Self {
            chip,
            machine,
            circuit,
            wiring,
        })
    }

    #[must_use]
    pub const fn chip(&self) -> &Chip {
        &self.chip
    }

    #[must_use]
    pub const fn machine(&self) -> &Machine {
        &self.machine
    }

    #[must_use]
    pub const fn circuit(&self) -> &Circuit {
        &self.circuit
    }

    #[must_use]
    pub const fn wiring(&self) -> &Wiring {
        &self.wiring
    }

    #[must_use]
    pub fn halo(&self) -> u32 {
        u32::from(self.wiring.reads.iter().any(|read| read.at != [0, 0, 0]))
    }

    #[must_use]
    pub fn written(&self) -> Vec<u32> {
        let mut planes: Vec<u32> = self
            .wiring
            .writes
            .iter()
            .chain(&self.wiring.keeps)
            .map(|spot| spot.plane)
            .collect();
        planes.sort_unstable();
        planes.dedup();
        planes
    }

    #[must_use]
    pub fn mask(&self, plane: u32) -> u32 {
        self.wiring
            .writes
            .iter()
            .chain(&self.wiring.keeps)
            .filter(|spot| spot.plane == plane)
            .fold(0, |mask, spot| mask | (1 << spot.bit))
    }

    fn own(&self, plane: u32, corner: &str) -> Json {
        if self.wiring.channels <= 1 {
            return op("at", vec![named("cells"), named(corner)]);
        }
        op(
            "at",
            vec![op("at", vec![named("cells"), named(corner)]), word(plane)],
        )
    }

    fn looked(&self, read: &Read) -> Json {
        if read.at == [0, 0, 0] {
            return self.own(read.plane, "c");
        }
        let [dx, dy, dz] = read.at;
        op(
            "look",
            vec![
                word(read.plane),
                Json::Int(dx),
                Json::Int(dy),
                Json::Int(dz),
                named("c"),
            ],
        )
    }

    fn source(&self, index: u32) -> String {
        let state = self.circuit.state;
        let reads = state + self.circuit.inputs;
        if index < state {
            format!("s{index}")
        } else if index < reads {
            format!("i{}", index - state)
        } else {
            format!("g{}", index - reads)
        }
    }

    fn spots(&self) -> Vec<(Spot, String)> {
        let carried = self.circuit.state + self.circuit.outputs;
        let first = u32::try_from(self.circuit.gates.len())
            .unwrap_or(u32::MAX)
            .saturating_sub(carried);
        self.wiring
            .keeps
            .iter()
            .chain(&self.wiring.writes)
            .enumerate()
            .map(|(at, spot)| {
                let gate = first + u32::try_from(at).unwrap_or(u32::MAX);
                (*spot, format!("g{gate}"))
            })
            .collect()
    }

    fn corner(&self) -> Json {
        let mut bound: Vec<Json> = Vec::new();
        let mut bind = |name: String, value: Json| {
            bound.push(Json::List(vec![wire::text(&name), value]));
        };
        for (at, spot) in self.wiring.keeps.iter().enumerate() {
            bind(
                format!("s{at}"),
                bit_of(self.own(spot.plane, "c"), spot.bit),
            );
        }
        for (at, read) in self.wiring.reads.iter().enumerate() {
            bind(format!("i{at}"), bit_of(self.looked(read), read.bit));
        }
        for (at, gate) in self.circuit.gates.iter().enumerate() {
            bind(
                format!("g{at}"),
                gate_expression(
                    gate.kind,
                    named(&self.source(gate.left)),
                    named(&self.source(gate.right)),
                ),
            );
        }
        let spots = self.spots();
        let mut finals: Vec<(u32, String)> = Vec::new();
        for plane in self.written() {
            let mut held = match self.wiring.rest {
                Rest::Kept => {
                    let kept = !self.mask(plane);
                    if kept == 0 {
                        None
                    } else {
                        Some(op("and", vec![self.own(plane, "c"), word(kept)]))
                    }
                }
                Rest::Cleared => None,
            };
            for (at, (spot, gate)) in spots
                .iter()
                .filter(|(spot, _)| spot.plane == plane)
                .enumerate()
            {
                let placed = if spot.bit == 0 {
                    named(gate)
                } else {
                    op("shl", vec![named(gate), word(spot.bit)])
                };
                let value = match held.take() {
                    None => placed,
                    Some(before) => op("or", vec![before, placed]),
                };
                let name = format!("w{plane}_{at}");
                bind(name.clone(), value);
                held = Some(named(&name));
            }
            if let Some(Json::Text(name)) = held {
                finals.push((plane, name));
            }
        }
        let plane_word = |plane: u32| {
            finals
                .iter()
                .find(|(held, _)| *held == plane)
                .map_or_else(|| self.own(plane, "c"), |(_, name)| named(name))
        };
        let body = if self.wiring.channels <= 1 {
            plane_word(0)
        } else {
            op("vec", (0..self.wiring.channels).map(plane_word).collect())
        };
        op("let", vec![Json::List(bound), body])
    }

    fn changed(&self) -> Json {
        let out = |plane: u32| {
            if self.wiring.channels <= 1 {
                op("at", vec![named("out"), named("k")])
            } else {
                op(
                    "at",
                    vec![op("at", vec![named("out"), named("k")]), word(plane)],
                )
            }
        };
        let mut held: Option<Json> = None;
        for plane in self.written() {
            let one = op("ne", vec![self.own(plane, "k"), out(plane)]);
            held = Some(match held {
                None => one,
                Some(before) => op("either", vec![before, one]),
            });
        }
        held.unwrap_or_else(|| op("ne", vec![word(0), word(0)]))
    }

    #[must_use]
    pub fn json(&self) -> Json {
        let corners = op(
            "map",
            vec![named("c"), op("range", vec![word(8)]), self.corner()],
        );
        let count = op(
            "count",
            vec![op(
                "map",
                vec![named("k"), op("range", vec![word(8)]), self.changed()],
            )],
        );
        let step = op(
            "let",
            vec![
                Json::List(vec![Json::List(vec![wire::text("out"), corners])]),
                op("vec", vec![named("out"), op("vec", vec![count])]),
            ],
        );
        wire::map(vec![
            ("law", word(limits::VERSION)),
            ("name", wire::text(&self.wiring.name)),
            ("params", word(0)),
            ("channels", word(self.wiring.channels)),
            ("halo", word(self.halo())),
            (
                "tally",
                wire::map(vec![("counts", word(1)), ("weighted", Json::Bool(false))]),
            ),
            ("consts", Json::Map(Vec::new())),
            ("defs", Json::Map(Vec::new())),
            ("step", step),
        ])
    }

    #[must_use]
    pub fn text(&self) -> String {
        wire::written(&self.json())
    }
}
