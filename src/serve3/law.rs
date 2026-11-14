use std::sync::Arc;
use std::time::Instant;

use simdr::lanes::LaneError;

use crate::branch::{Stepping, World};
use crate::bricks::Bricks;
use crate::compact::Plan;
use crate::host::Host;
use crate::law::Law;
use crate::law::quiet::{self, Class};
use crate::layout::BINDINGS;
use crate::live::{self, Loop};
use crate::rule3::Declared;
use crate::sand::Grid;
use crate::sand3::Grid3;
use crate::wire::{Json, counted, map, text};

use super::errors::{Code, Refusal};
use super::journal::{Entry, Made};
use super::schema::word;
use super::tools::{int, ints, limited, tool, unread, world_number};
use super::{Failed, Listed, Server, Tool};

pub const TOOLS: &[Tool] = &[Tool {
    name: "grit_world_params",
    shown: params_shown,
    run: params_written,
}];

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
pub struct Refused(pub crate::law::Refusal);

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
            Refusal::new(Code::LawRefused, "$.law", self.to_string()),
            vec![("law", self.0.json())],
        )
    }
}

pub fn compiled(held: &str) -> Result<Law, Failed> {
    Law::compile(held).map_err(|why| Box::new(Refused(why)) as Failed)
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
