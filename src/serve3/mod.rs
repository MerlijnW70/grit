//! The world tools: a solid world of the 3D listed engine that an agent
//! creates, writes, steps, reads and closes through MCP calls, without a
//! screen. [`Server`] holds the open worlds between calls and answers every
//! 2D tool through [`crate::serve::handle`] as before.

use std::collections::BTreeMap;

use simdr::lanes::LaneError;

use crate::bricks::{Bricks, Content, PoolUse};

use crate::compact::Plan;
use crate::host::Host;
use crate::layout::{BINDINGS, GRAND, STATE};
use crate::live::{self, Loop};
use crate::resident::{Groups, Resident};
use crate::rule3::Rule;
use crate::rule3::code::{self, Code};
use crate::rule3::embed::{self, Embed};
use crate::rule3::heat::{self, Heat};
use crate::rule3::nature::{self, Nature};
use crate::rule3::sand::{self, Sand};
use crate::sand::Grid;
use crate::sand3::Grid3;
use crate::wire::{Json, counted, map, text};

pub mod ask;
pub mod branch;
pub mod errors;
pub mod law;
pub mod learn;
pub mod schema;
pub mod tools;
pub mod transport;

use errors::{Refusal, refusal_of};
use schema::Check;

/// The most cells one `grit_world_read` returns, and one `grit_world_write`
/// takes: a box of 16 by 16 by 16.
pub const READ_MOST: u64 = 4096;

/// The most cells one `grit_world_slice` returns: a layer of 256 by 256.
pub const SLICE_MOST: u64 = 65536;

/// The most frames one `grit_world_step` runs.
pub const STEP_MOST: u32 = 65536;

pub const POOL_LEAST_CELLS: u64 = 1 << 21;

pub const PAGE_BYTES: usize = (512 - 4) << 10;

/// How many frames go to the device at a time.
const BATCH: u32 = 64;

/// The digits a slice of kinds is drawn with, one a kind.
const DIGITS: &[u8; 16] = b"0123456789abcdef";

/// A rule the world tools run by name. [`Named`] makes one of any
/// [`Rule`].
pub trait Listed {
    fn name(&self) -> &'static str;

    /// How many words of parameters the rule reads.
    fn params(&self) -> usize;

    /// The words of the tally a frame under the rule leaves, as
    /// [`crate::rule3::Tally::words`] gives them.
    fn tally_words(&self) -> usize;

    /// The listed frame under the rule, as [`live::wired3_by`] builds it.
    fn wired(&self, subgroup: u32, grid: Grid, plan: &Plan) -> Result<Loop, LaneError>;

    /// The bindings' sizes, as [`live::sizes3`] gives them.
    fn sizes(&self, plan: &Plan) -> [usize; BINDINGS as usize];

    fn bricked(&self, bricks: &Bricks, plan: &Plan) -> Result<Loop, LaneError>;

    fn bricked_sizes(&self, bricks: &Bricks, plan: &Plan) -> Vec<usize>;

    /// One frame on the CPU, as [`crate::sand3::reference`] steps it.
    fn reference(&self, grid: Grid3, frame: u32, cells: &[u32], params: &[u32]) -> Vec<u32>;

    /// The kind of a cell word, from 0 to 15; a slice shows a kind past 15
    /// as `+`.
    fn kind(&self, word: u32) -> u32;
}

/// A [`Rule`] under a name, with the kind of its words.
pub struct Named<R> {
    pub name: &'static str,
    pub rule: R,
    pub kind: fn(u32) -> u32,
}

impl<R: Rule> Listed for Named<R> {
    fn name(&self) -> &'static str {
        self.name
    }

    fn params(&self) -> usize {
        self.rule.params()
    }

    fn tally_words(&self) -> usize {
        self.rule.tally().words()
    }

    fn wired(&self, subgroup: u32, grid: Grid, plan: &Plan) -> Result<Loop, LaneError> {
        live::wired3_by(&self.rule, subgroup, grid, plan)
    }

    fn sizes(&self, plan: &Plan) -> [usize; BINDINGS as usize] {
        live::sizes3(&self.rule, plan)
    }

    fn bricked(&self, bricks: &Bricks, plan: &Plan) -> Result<Loop, LaneError> {
        crate::bricks::arranged(&self.rule, bricks, plan, true)
    }

    fn bricked_sizes(&self, bricks: &Bricks, plan: &Plan) -> Vec<usize> {
        bricks.sizes(&self.rule, plan)
    }

    fn reference(&self, grid: Grid3, frame: u32, cells: &[u32], params: &[u32]) -> Vec<u32> {
        crate::sand3::reference(&self.rule, grid, frame, cells, params)
    }

    fn kind(&self, word: u32) -> u32 {
        (self.kind)(word)
    }
}

/// `rule` under `name`, with the kind of its words, as [`rules`] lists it.
#[must_use]
pub fn named<R: Rule + 'static>(
    name: &'static str,
    rule: R,
    kind: fn(u32) -> u32,
) -> Box<dyn Listed> {
    Box::new(Named { name, rule, kind })
}

/// Every rule the world tools know, in the order they are listed. A new
/// rule is one more line here.
#[must_use]
pub fn rules() -> Vec<Box<dyn Listed>> {
    rules_on(None)
}

#[must_use]
pub fn rules_on(host: Option<&Host>) -> Vec<Box<dyn Listed>> {
    let embedding = host.map_or_else(Embed::default, Embed::on);
    vec![
        named("sand", Sand, sand::material_of),
        named("heat", Heat, heat::kind_of),
        named("code", Code, code::instruction_of),
        named("nature", Nature, nature::kind_of),
        named("embed", embedding, embed::kind_of),
    ]
}

/// The name of any refusal a world tool gives: a [`Refusal`], or the
/// engine's own error for a world the device cannot hold.
#[must_use]
pub fn refusal_name(why: &(dyn std::error::Error + 'static)) -> &'static str {
    refusal_of(why).code.name()
}

pub type Failed = Box<dyn std::error::Error>;

/// One open world: the listed engine under its rule, and the frame it is
/// on.
struct World<'host> {
    rule: Box<dyn Listed>,
    live: Resident<'host>,
    stages: Vec<Vec<Groups>>,
    grid: Grid,
    solid: Grid3,
    plan: Plan,
    frame: u32,
    pool: Option<Bricks>,
}

impl World<'_> {
    const fn size(&self) -> [u32; 3] {
        [self.solid.wide, self.solid.deep, self.solid.tall]
    }

    fn index(&self, at: [u32; 3]) -> usize {
        self.solid.index(at[0], at[1], at[2])
    }

    fn listed(&mut self) -> Result<u32, Failed> {
        if let Some(bricks) = self.pool.as_ref() {
            let (octs, whole) = bricks.listed(&mut self.live)?;
            return Ok(octs.saturating_add(whole.saturating_mul(bricks.per_block())));
        }
        Ok(self
            .live
            .read(GRAND as usize, 1)?
            .first()
            .copied()
            .unwrap_or(0))
    }

    fn reported(&mut self, mut answer: Vec<(&'static str, Json)>) -> Result<Json, Failed> {
        if let Some(bricks) = self.pool.as_ref() {
            let used = bricks.pool_use(&mut self.live)?;
            answer.push(("sparse", Json::Bool(true)));
            answer.push(("pool_use", pooled(used)));
        }
        Ok(map(answer))
    }

    fn cells_at(&mut self, points: &[[u32; 3]]) -> Result<Vec<u32>, Failed> {
        let Some(bricks) = self.pool.as_ref() else {
            let last = points.iter().map(|&at| self.index(at)).max().unwrap_or(0);
            let state = self.live.read(STATE as usize, last + 1)?;
            return points
                .iter()
                .map(|&at| {
                    state
                        .get(self.index(at))
                        .copied()
                        .ok_or_else(|| "the world read back short".into())
                })
                .collect();
        };
        bricks.check(&mut self.live)?;
        let table = self
            .live
            .read(bricks.table() as usize, bricks.table_words())?;
        let per = bricks.cells();
        let places: Vec<crate::bricks::Cell<u32>> =
            points.iter().map(|&at| bricks.cell_of(at)).collect();
        let slot_of = |brick: u32| {
            table
                .get(brick as usize * crate::bricks::ENTRY)
                .copied()
                .unwrap_or(crate::bricks::UNIFORM)
        };
        let reach = places
            .iter()
            .map(|place| slot_of(place.brick))
            .filter(|&slot| slot != crate::bricks::UNIFORM)
            .map(|slot| (slot as usize + 1) * per)
            .max()
            .unwrap_or(0);
        let pool = live::read_state(&mut self.live, bricks.windows(), reach)?;
        Ok(places
            .iter()
            .map(
                |place| match bricks.brick_words(&table, &pool, place.brick) {
                    Content::Uniform(value) => value,
                    Content::Mixed(words) => words.get(place.offset as usize).copied().unwrap_or(0),
                },
            )
            .collect())
    }
}

fn pooled(used: PoolUse) -> Json {
    map(vec![
        ("held", counted(used.held)),
        ("free", counted(used.free)),
        ("capacity", counted(used.capacity)),
        ("overflow", Json::Bool(used.overflow)),
        ("lost", Json::Bool(used.lost)),
    ])
}

enum Slot<'host> {
    Open(Box<World<'host>>),
    Closed,
}

pub struct Tool {
    pub name: &'static str,
    pub shown: fn() -> Json,
    pub run: fn(&mut Server<'_>, &Json) -> Result<Json, Failed>,
}

/// The MCP server with its open worlds. Every 2D tool answers as
/// [`crate::serve::handle`] does; `tools/list` names the 2D tools and then
/// the world tools.
pub struct Server<'host> {
    host: Option<&'host Host>,
    worlds: BTreeMap<u32, Slot<'host>>,
    next: u32,
}

/// The names of the world tools, in the order `tools/list` gives them.
pub const WORLD_TOOLS: [&str; 8] = [
    "grit_world_rules",
    "grit_world_create",
    "grit_world_write",
    "grit_world_step",
    "grit_world_read",
    "grit_world_slice",
    "grit_world_close",
    "grit_describe",
];

fn served() -> impl Iterator<Item = &'static Tool> {
    tools::TOOLS
        .iter()
        .chain(law::TOOLS)
        .chain(ask::TOOLS)
        .chain(branch::TOOLS)
        .chain(learn::TOOLS)
}

fn shut(tool: Json) -> Json {
    match tool {
        Json::Map(held) => Json::Map(
            held.into_iter()
                .map(|(key, one)| {
                    if key == "inputSchema" {
                        (key, schema::closed(one))
                    } else {
                        (key, one)
                    }
                })
                .collect(),
        ),
        other => other,
    }
}

fn flat() -> Vec<Json> {
    match crate::serve::tools() {
        Json::List(held) => held.into_iter().map(shut).collect(),
        other => vec![shut(other)],
    }
}

fn input(shown: &Json) -> Json {
    shown
        .get("inputSchema")
        .cloned()
        .unwrap_or_else(|| schema::object(Vec::new(), &[]))
}

fn delivery(result: Json) -> Json {
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
    ])
}

fn known_rules() -> Json {
    Json::List(
        rules()
            .iter()
            .map(|one| {
                map(vec![
                    ("name", text(one.name())),
                    (
                        "params",
                        counted(u32::try_from(one.params()).unwrap_or(u32::MAX)),
                    ),
                ])
            })
            .collect(),
    )
}

fn listing() -> String {
    rules()
        .iter()
        .map(|one| format!("{} ({} parameters)", one.name(), one.params()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn world_refusal(mut refusal: Refusal, arguments: &Json) -> Json {
    let code = refusal.code;
    if code == errors::Code::UnknownRule {
        let asked = match arguments.get("rule") {
            None => "\"\"".to_owned(),
            Some(Json::Text(asked)) => errors::shown(&format!("{asked:?}")),
            Some(other) => errors::shown_value(other),
        };
        refusal.message = format!("rule {asked} is not one of {}", listing());
    }
    let more = if matches!(code, errors::Code::UnknownRule | errors::Code::Params) {
        vec![("rules", known_rules())]
    } else {
        Vec::new()
    };
    transport::refused(&refusal, more)
}

pub struct Flat<'host> {
    pub host: Option<&'host Host>,
}

fn flat_called(host: Option<&Host>, listed: &[Json], name: &str, arguments: &Json) -> Option<Json> {
    let shown = listed
        .iter()
        .find(|one| one.get("name").and_then(Json::text) == Some(name))?;
    if let Err(refusal) = schema::checked(name, &input(shown), arguments, Check::Keys) {
        return Some(transport::refused(&refusal, Vec::new()));
    }
    Some(match crate::serve::ran(host, name, arguments) {
        Ok(result) => delivery(result),
        Err(why) => transport::refused(&refusal_of(why.as_ref()), Vec::new()),
    })
}

impl transport::Served for Flat<'_> {
    fn listed(&self) -> Vec<Json> {
        flat()
    }

    fn called(&mut self, name: &str, arguments: &Json) -> Option<Json> {
        flat_called(self.host, &flat(), name, arguments)
    }
}

impl<'host> Server<'host> {
    #[must_use]
    pub const fn new(host: Option<&'host Host>) -> Self {
        Self {
            host,
            worlds: BTreeMap::new(),
            next: 1,
        }
    }

    /// How many worlds are open.
    #[must_use]
    pub fn open(&self) -> usize {
        self.worlds
            .values()
            .filter(|slot| matches!(slot, Slot::Open(_)))
            .count()
    }

    fn world(&mut self, held: &Json) -> Result<&mut World<'host>, Failed> {
        let world = tools::int(held, "world")?;
        match self.worlds.get_mut(&world) {
            Some(Slot::Open(open)) => Ok(open),
            Some(Slot::Closed) => Err(Box::new(Refusal::new(
                errors::Code::Closed,
                "$.world",
                format!("world {world} was closed"),
            ))),
            None => Err(Box::new(Refusal::new(
                errors::Code::NoWorld,
                "$.world",
                format!("no world {world} was ever opened"),
            ))),
        }
    }

    /// Answers one request, as [`crate::serve::handle`] does for the 2D
    /// tools.
    pub fn handle(&mut self, request: &Json) -> Option<Json> {
        transport::handle(self, request)
    }

    /// Reads one line and answers it, as [`crate::serve::heard`] does.
    pub fn heard(&mut self, line: &str) -> Option<Json> {
        transport::heard(self, line)
    }
}

impl transport::Served for Server<'_> {
    fn listed(&self) -> Vec<Json> {
        let mut listed = flat();
        listed.extend(served().map(|tool| (tool.shown)()));
        listed
    }

    fn called(&mut self, name: &str, arguments: &Json) -> Option<Json> {
        let Some(tool) = served().find(|tool| tool.name == name) else {
            return flat_called(self.host, &flat(), name, arguments);
        };
        let given = match schema::checked(name, &input(&(tool.shown)()), arguments, Check::Whole) {
            Ok(given) => given,
            Err(refusal) => return Some(world_refusal(refusal, arguments)),
        };
        Some(match (tool.run)(self, &given) {
            Ok(result) => delivery(result),
            Err(why) => world_refusal(refusal_of(why.as_ref()), arguments),
        })
    }
}
