use std::sync::Arc;
use std::time::Instant;

use simdr::lanes::LaneError;

use crate::branch::{Stepping, World};
use crate::bricks::Bricks;
use crate::compact::Plan;
use crate::host::Host;
use crate::law::Law;
use crate::law::chip::{self, Circuit, Read, Rest, Spot, Wired, Wiring};
use crate::law::quiet::{self, Class};
use crate::layout::BINDINGS;
use crate::live::{self, Loop};
use crate::netlist::{Gate, SOURCES};
use crate::rule3::Declared;
use crate::sand::Grid;
use crate::sand3::Grid3;
use crate::wire::{Json, counted, map, text};

use super::errors::{Code, Refusal};
use super::journal::{Entry, Made};
use super::schema::{choice, object, property, whole, with, word};
use super::tools::{int, ints, limited, tool, unread, world_number};
use super::{Failed, Listed, Server, Tool};

pub const TOOLS: &[Tool] = &[
    Tool {
        name: "grit_world_params",
        shown: params_shown,
        run: params_written,
    },
    Tool {
        name: "grit_law_from_chip",
        shown: chip_shown,
        run: from_chip,
    },
];

pub const KINDS: u32 = 15;

pub const TALLIED_LISTED: &str = "listed octs";

pub const TALLIED_EVERY: &str = "every oct";

pub const TALLIED: &str = "a law world's tally sums its counts over the octs a frame steps, and its answers name which as tallied: every oct for a whole world; listed octs for a proved one, which sums only the octs the frame lists, so it equals the sum over every oct only for counts that are 0 on an oct the step leaves unchanged";

pub const LOOK: &str = "[\"look\", plane, dx, dy, dz, corner] is the word of plane at the corner's cell plus (dx, dy, dz), each -1, 0 or 1, wrapped on the torus; plane, offsets and corner are ints known when the law compiles. A law that looks declares halo 1, one that does not halo 0. It may look only at a frozen plane: one whose 8 out words are its own input words unchanged; a look at a plane the law writes is refused halo_written. A law that looks runs only on dense worlds";

pub struct Lawful {
    law: Law,
    class: Class,
}

impl Lawful {
    #[must_use]
    pub const fn new(law: Law, class: Class) -> Self {
        Self { law, class }
    }

    #[must_use]
    pub const fn law(&self) -> &Law {
        &self.law
    }

    #[must_use]
    pub const fn class(&self) -> &Class {
        &self.class
    }

    #[must_use]
    pub const fn tallied(&self) -> &'static str {
        if self.class.proved() {
            TALLIED_LISTED
        } else {
            TALLIED_EVERY
        }
    }

    #[must_use]
    pub fn told(&self) -> Vec<(&'static str, Json)> {
        let mut told = vec![
            ("law", text(&self.law.hex())),
            ("quiet", text(self.class.name())),
            ("tallied", text(self.tallied())),
        ];
        if let Some(why) = self.class.why() {
            told.push(("quiet_why", text(&why)));
        }
        told
    }
}

impl Listed for Lawful {
    fn name(&self) -> &str {
        self.law.name()
    }

    fn params(&self) -> usize {
        Declared::params(&self.law)
    }

    fn tally_words(&self) -> usize {
        Declared::tally(&self.law).words()
    }

    fn wired(&self, subgroup: u32, grid: Grid, plan: &Plan) -> Result<Loop, LaneError> {
        live::wired3_by(&self.law, subgroup, grid, plan)
    }

    fn sizes(&self, plan: &Plan) -> [usize; BINDINGS as usize] {
        live::sizes3(&self.law, plan)
    }

    fn bricked(&self, bricks: &Bricks, plan: &Plan) -> Result<Loop, LaneError> {
        crate::bricks::arranged(&self.law, bricks, plan, true)
    }

    fn bricked_sizes(&self, bricks: &Bricks, plan: &Plan) -> Vec<usize> {
        bricks.sizes(&self.law, plan)
    }

    fn reference(&self, grid: Grid3, frame: u32, cells: &[u32], params: &[u32]) -> Vec<u32> {
        crate::sand3::reference(&self.law, grid, frame, cells, params)
    }

    fn kind(&self, word: u32) -> u32 {
        word & KINDS
    }

    fn lawful(&self) -> Option<&Lawful> {
        Some(self)
    }
}

#[derive(Debug)]
pub struct Refused(pub crate::law::Refusal, pub &'static str);

impl std::fmt::Display for Refused {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(out, "the law does not compile: {}", self.0)
    }
}

impl std::error::Error for Refused {}

impl Refused {
    #[must_use]
    pub fn refusal(&self) -> (Refusal, Vec<(&'static str, Json)>) {
        (
            Refusal::new(Code::LawRefused, self.1, self.to_string()),
            vec![("law", self.0.json())],
        )
    }
}

pub fn compiled(held: &str) -> Result<Law, Failed> {
    Law::compile(held).map_err(|why| Box::new(Refused(why, "$.law")) as Failed)
}

#[must_use]
pub const fn stepping_of(class: &Class, sparse: bool) -> Stepping {
    if sparse || class.proved() {
        Stepping::Listed
    } else {
        Stepping::Whole
    }
}

#[must_use]
pub fn background_class(law: &Law, params: &[u32]) -> Class {
    quiet::class(law, params, &[])
}

#[must_use]
pub fn quiet_refused(law: &Law, class: &Class, field: &str) -> Failed {
    Box::new(Refusal::new(
        Code::QuietRefused,
        field,
        format!(
            "the law {} is whole under these params: {}; a sparse world steps only the octs it lists, so make the world dense, where it steps every oct",
            law.name(),
            class.why().unwrap_or_default()
        ),
    ))
}

pub fn halo_refused(law: &Law, sparse: bool, field: &str) -> Result<(), Failed> {
    crate::branch::looks_dense(law, sparse).map_err(|why| {
        let mut refusal = super::branch_refusal(&why);
        field.clone_into(&mut refusal.field);
        Box::new(refusal) as Failed
    })
}

pub fn made_world<'host>(
    host: &'host Host,
    held: &str,
    made: &Made,
) -> Result<World<'host>, Failed> {
    let law = compiled(held)?;
    halo_refused(&law, made.sparse, "$.sparse")?;
    let class = background_class(&law, &made.params);
    World::create_law_stepped(host, &law, &made.branch(), stepping_of(&class, made.sparse))
}

pub fn opened<'host>(
    host: &'host Host,
    image: &crate::branch::Image,
) -> Result<World<'host>, Failed> {
    match &image.law {
        crate::branch::Law::Rule { .. } => World::open(host, image),
        crate::branch::Law::Law { canonical, .. } => {
            let law = compiled(canonical)?;
            let class = background_class(&law, &image.params);
            let sparse = matches!(image.state, crate::branch::State::Sparse { .. });
            halo_refused(&law, sparse, "$.name")?;
            World::open_stepped(host, image, stepping_of(&class, sparse))
        }
    }
}

pub fn reparam(world: &mut World<'_>, params: &[u32]) -> Result<(), Failed> {
    let stepping = match world.law() {
        None => Stepping::Listed,
        Some(law) => stepping_of(&background_class(law, params), world.pool().is_some()),
    };
    world.set_params(params, stepping)
}

pub fn listed(host: &Host, world: &World<'_>) -> Result<Box<dyn Listed>, Failed> {
    let Some(law) = world.law().cloned() else {
        return super::rule_of(host, world.rule());
    };
    let class = background_class(&law, world.params());
    Ok(Box::new(Lawful::new(law, class)))
}

pub fn planned(
    host: &Host,
    name: &str,
    law: Option<&str>,
    params: &[u32],
) -> Result<Box<dyn Listed>, Failed> {
    let Some(held) = law else {
        return super::rule_of(host, name);
    };
    let law = compiled(held)?;
    let class = background_class(&law, params);
    Ok(Box::new(Lawful::new(law, class)))
}

#[must_use]
pub fn image_text(image: &crate::branch::Image) -> Option<Arc<str>> {
    match &image.law {
        crate::branch::Law::Rule { .. } => None,
        crate::branch::Law::Law { canonical, .. } => Some(Arc::from(canonical.as_str())),
    }
}

#[must_use]
pub fn hash_of(held: &str) -> String {
    crate::law::hex(&crate::branch::sha256(held.as_bytes()))
}

fn params_shown() -> Json {
    tool(
        "grit_world_params",
        "Give a world new params between frames; its next frame steps every oct. A law world's quiet class is decided again.",
        vec![
            ("world", world_number()),
            (
                "params",
                map(vec![
                    ("type", text("array")),
                    ("items", word()),
                    (
                        "description",
                        text("exactly as many words as the world's rule or law reads"),
                    ),
                ]),
            ),
        ],
        &["world", "params"],
    )
}

fn params_written(server: &mut Server<'_>, held: &Json) -> Result<Json, Failed> {
    let number = int(held, "world")?;
    let params = ints(
        held.get("params").ok_or_else(|| unread("params"))?,
        "params",
    )?;
    let live = server.world(held)?;
    let wanted = live.rule.params();
    if params.len() != wanted {
        return Err(limited(
            Code::Params,
            "$.params",
            format!(
                "{} reads {wanted} parameters, and {} were given",
                live.rule.name(),
                params.len()
            ),
            counted(u32::try_from(wanted).unwrap_or(u32::MAX)),
        ));
    }
    let sparse = live.world.pool().is_some();
    let decided = match live.world.law().cloned() {
        None => None,
        Some(law) => {
            let uniforms = live.world.uniforms()?;
            let class = quiet::class(&law, &params, &uniforms);
            if sparse && !class.proved() {
                return Err(quiet_refused(&law, &class, "$.params"));
            }
            Some((law, class))
        }
    };
    let stepping = decided
        .as_ref()
        .map_or(Stepping::Listed, |(_, class)| stepping_of(class, sparse));
    let owed = server.owed(number)?;
    let began = Instant::now();
    server.world(held)?.world.set_params(&params, stepping)?;
    let spent = began.elapsed();
    let journal = server
        .journaled(number, Entry::Params(params.clone()), spent)
        .or(owed);
    let snapshot = server.settle(number);
    let live = server.world(held)?;
    if let Some((law, class)) = decided {
        live.rule = Box::new(Lawful::new(law, class));
    }
    let listed = live.world.listed()?;
    let mut answer = vec![
        ("world", counted(number)),
        (
            "params",
            Json::List(params.iter().map(|one| counted(*one)).collect()),
        ),
        ("frame", counted(live.world.frame())),
        ("listed", counted(listed)),
    ];
    if let Some(lawful) = live.rule.lawful() {
        answer.extend(lawful.told());
    }
    if let Some(note) = journal.or(snapshot) {
        answer.push(("journal", note));
    }
    super::tools::reported(live, answer)
}

pub const FROM_CHIP: &str = "grit_law_from_chip turns a chip into a law: chip source i is state bit i below state, then input i - state, then the gates; input j reads bind_inputs[j].bit of plane bind_inputs[j].plane at the cell plus (dx, dy, dz), wrapped on the torus; output j sets bind_outputs[j].bit; state bit j is read from and written back to bind_state[j].bit at the cell itself; the other bits of a written word are kept or cleared as rest says, and a plane the chip does not write is left as it is. Each gate becomes at most two word operations per cell. A binding reads a plane the chip writes only at offset 0";

fn bits_shown(what: &str) -> Json {
    whole(what, 0, i64::from(chip::BITS - 1))
}

fn plane_shown() -> Json {
    whole(
        "the plane, below channels",
        0,
        i64::from(crate::law::limits::CHANNELS - 1),
    )
}

fn offset_shown(axis: &str) -> Json {
    with(
        whole(
            &format!("the step along {axis} from the cell, -1, 0 or 1; 0 if left out"),
            -1,
            1,
        ),
        vec![("default", Json::Int(0))],
    )
}

fn list_shown(items: Json, what: &str, most: i64) -> Json {
    map(vec![
        ("type", text("array")),
        ("items", items),
        ("description", text(what)),
        ("maxItems", Json::Int(most)),
    ])
}

fn spot_shown() -> Json {
    object(
        vec![
            ("plane", plane_shown()),
            ("bit", bits_shown("the bit of that plane's word")),
        ],
        &["plane", "bit"],
    )
}

fn chip_shown() -> Json {
    let source = i64::from(SOURCES) - 2;
    let gate = object(
        vec![
            (
                "kind",
                whole(
                    "which of the 16 two-input functions, as its truth table: bit left + 2 right of kind is the answer",
                    0,
                    i64::from(chip::KINDS - 1),
                ),
            ),
            (
                "left",
                whole(
                    "the source it reads first: the state bits, then the inputs, then the gates before it",
                    0,
                    source,
                ),
            ),
            ("right", whole("the source it reads second", 0, source)),
        ],
        &["kind", "left", "right"],
    );
    let read = object(
        vec![
            ("plane", plane_shown()),
            ("dx", offset_shown("x")),
            ("dy", offset_shown("y")),
            ("dz", offset_shown("z")),
            ("bit", bits_shown("the bit of that cell's word")),
        ],
        &["plane", "bit"],
    );
    let chip_inputs = i64::from(crate::netlist::INPUTS);
    super::tools::tool(
        "grit_law_from_chip",
        "Turn a chip from grit_machine or grit_synthesise into a law a world runs: each input reads a bit of a cell at most one step away, each output and state bit a bit of the cell.",
        vec![
            (
                "inputs",
                whole(
                    "the chip's inputs, as grit_machine and grit_synthesise take them",
                    0,
                    chip_inputs,
                ),
            ),
            (
                "outputs",
                whole("the chip's outputs", 1, i64::from(crate::netlist::OUTPUTS)),
            ),
            (
                "state",
                with(
                    whole(
                        "the chip's state bits, as grit_machine takes them; 0 if left out",
                        0,
                        chip_inputs,
                    ),
                    vec![("default", Json::Int(0))],
                ),
            ),
            (
                "gates",
                list_shown(
                    gate,
                    "the chip's gates, as grit_machine and grit_synthesise answer them; the last state plus outputs gates are the state bits, then the outputs",
                    source + 1,
                ),
            ),
            (
                chip::READS,
                list_shown(
                    read,
                    "one binding for each chip input, in order",
                    chip_inputs,
                ),
            ),
            (
                chip::WRITES,
                list_shown(
                    spot_shown(),
                    "one binding for each chip output, in order",
                    i64::from(crate::netlist::OUTPUTS),
                ),
            ),
            (
                chip::KEEPS,
                with(
                    list_shown(
                        spot_shown(),
                        "one binding for each state bit, in order, read and written at the cell itself; none if left out",
                        chip_inputs,
                    ),
                    vec![("default", Json::List(Vec::new()))],
                ),
            ),
            (
                "channels",
                whole(
                    "the planes of the world the law runs on",
                    1,
                    i64::from(crate::law::limits::CHANNELS),
                ),
            ),
            (
                "rest",
                with(
                    choice(
                        "what the bits of a written word that no output or state bit is on become; kept if left out",
                        &["kept", "cleared"],
                    ),
                    vec![("default", text("kept"))],
                ),
            ),
            (
                "name",
                with(
                    property("string", "the law's name; chip if left out"),
                    vec![("default", text("chip"))],
                ),
            ),
        ],
        &[
            "inputs",
            "outputs",
            "gates",
            chip::READS,
            chip::WRITES,
            "channels",
        ],
    )
}

fn number_at(held: &Json, key: &str) -> i64 {
    held.get(key).and_then(Json::int).unwrap_or(0)
}

fn word_at(held: &Json, key: &str) -> u32 {
    u32::try_from(number_at(held, key)).unwrap_or(u32::MAX)
}

fn items<'held>(held: &'held Json, key: &str) -> &'held [Json] {
    held.get(key).and_then(Json::list).unwrap_or(&[])
}

fn spots_of(held: &Json, key: &str) -> Vec<Spot> {
    items(held, key)
        .iter()
        .map(|one| Spot {
            plane: word_at(one, "plane"),
            bit: word_at(one, "bit"),
        })
        .collect()
}

fn chip_refused(why: &chip::Refused) -> Failed {
    let code = match why.fault {
        chip::Fault::Chip => Code::ChipRefused,
        chip::Fault::Channels => Code::Malformed,
        chip::Fault::Unbound => Code::Unbound,
        chip::Fault::Overbound => Code::Overbound,
        chip::Fault::Plane => Code::PlanePast,
        chip::Fault::Bit => Code::BitPast,
        chip::Fault::Offset => Code::OffsetPast,
        chip::Fault::Shared => Code::BitShared,
        chip::Fault::Written => Code::HaloWritten,
    };
    Box::new(Refusal::new(code, &why.path, why.message.clone()))
}

fn from_chip(_: &mut Server<'_>, held: &Json) -> Result<Json, Failed> {
    let circuit = Circuit {
        state: word_at(held, "state"),
        inputs: word_at(held, "inputs"),
        outputs: word_at(held, "outputs"),
        gates: items(held, "gates")
            .iter()
            .map(|one| Gate {
                kind: word_at(one, "kind"),
                left: word_at(one, "left"),
                right: word_at(one, "right"),
            })
            .collect(),
    };
    let rest = match held.get("rest").and_then(Json::text) {
        Some("cleared") => Rest::Cleared,
        _ => Rest::Kept,
    };
    let wiring = Wiring {
        name: held
            .get("name")
            .and_then(Json::text)
            .unwrap_or("chip")
            .to_owned(),
        channels: word_at(held, "channels"),
        reads: items(held, chip::READS)
            .iter()
            .map(|one| Read {
                plane: word_at(one, "plane"),
                at: [
                    number_at(one, "dx"),
                    number_at(one, "dy"),
                    number_at(one, "dz"),
                ],
                bit: word_at(one, "bit"),
            })
            .collect(),
        writes: spots_of(held, chip::WRITES),
        keeps: spots_of(held, chip::KEEPS),
        rest,
    };
    let wired = Wired::new(circuit, wiring).map_err(|why| chip_refused(&why))?;
    let written = wired.text();
    let law = Law::compile(&written).map_err(|why| Box::new(Refused(why, "$")) as Failed)?;
    let class = background_class(&law, &[]);
    let mut answer = vec![
        ("law", text(&written)),
        ("hash", text(&law.hex())),
        ("name", text(law.name())),
        ("channels", counted(law.channels())),
        ("halo", counted(law.halo())),
        ("quiet", text(class.name())),
        (
            "stepping",
            text(if class.proved() { "listed" } else { "whole" }),
        ),
        ("rest", text(wired.wiring().rest.name())),
        (
            "nodes",
            counted(u32::try_from(law.nodes()).unwrap_or(u32::MAX)),
        ),
        (
            "gates",
            counted(u32::try_from(wired.circuit().gates.len()).unwrap_or(u32::MAX)),
        ),
    ];
    if let Some(why) = class.why() {
        answer.insert(6, ("quiet_why", text(&why)));
    }
    Ok(map(answer))
}
