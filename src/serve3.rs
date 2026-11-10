//! The world tools: a solid world of the 3D listed engine that an agent
//! creates, writes, steps, reads and closes through MCP calls, without a
//! screen. [`Server`] holds the open worlds between calls and answers every
//! 2D tool through [`crate::serve::handle`] as before.

use std::collections::BTreeMap;

use simdr::lanes::LaneError;

use crate::compact::Plan;
use crate::host::Host;
use crate::layout::{BINDINGS, Dimension, GRAND, PARAMS, STATE, TALLY};
use crate::live::{self, Loop};
use crate::resident::{Groups, Resident};
use crate::rule3::heat::{self, Heat};
use crate::rule3::sand::{self, Sand};
use crate::rule3::{FIRST_PARAM, Rule};
use crate::sand::Grid;
use crate::sand3::Grid3;
use crate::serve::{answered, delivered, refused, said};
use crate::sweep;
use crate::wire::{Json, counted, map, text};

/// The most cells one `grit_world_read` returns, and one `grit_world_write`
/// takes: a box of 16 by 16 by 16.
pub const READ_MOST: u64 = 4096;

/// The most cells one `grit_world_slice` returns: a layer of 256 by 256.
pub const SLICE_MOST: u64 = 65536;

/// The most frames one `grit_world_step` runs.
pub const STEP_MOST: u32 = 65536;

/// How many frames go to the device at a time.
const BATCH: u32 = 64;

/// The words of the tally a 3D frame leaves.
const TALLY_WORDS: usize = Dimension::Three.tally_words();

/// The digits a slice of kinds is drawn with, one a kind.
const DIGITS: &[u8; 16] = b"0123456789abcdef";

/// A rule the world tools run by name. [`Named`] makes one of any
/// [`Rule`].
pub trait Listed {
    fn name(&self) -> &'static str;

    /// How many words of parameters the rule reads.
    fn params(&self) -> usize;

    /// The listed frame under the rule, as [`live::wired3_by`] builds it.
    fn wired(&self, subgroup: u32, grid: Grid, plan: &Plan) -> Result<Loop, LaneError>;

    /// The bindings' sizes, as [`live::sizes3`] gives them.
    fn sizes(&self, plan: &Plan) -> [usize; BINDINGS as usize];

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

    fn wired(&self, subgroup: u32, grid: Grid, plan: &Plan) -> Result<Loop, LaneError> {
        live::wired3_by(&self.rule, subgroup, grid, plan)
    }

    fn sizes(&self, plan: &Plan) -> [usize; BINDINGS as usize] {
        live::sizes3(&self.rule, plan)
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
    vec![
        named("sand", Sand, sand::material_of),
        named("heat", Heat, heat::kind_of),
    ]
}

/// Why a world tool refused a call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// A cell, a corner of a box or a layer lies outside the world.
    Outside { what: String, size: [u32; 3] },
    /// No rule has this name.
    UnknownRule {
        asked: String,
        known: Vec<(String, usize)>,
    },
    /// The rule reads another number of parameters.
    Params {
        rule: String,
        given: usize,
        wanted: usize,
    },
    /// A read, slice or write names more cells than one call carries.
    TooMany {
        what: &'static str,
        cells: u64,
        most: u64,
    },
    /// The world was closed.
    Closed { world: u32 },
    /// No world was ever opened under this number.
    NoWorld { world: u32 },
    /// A box whose first corner lies past its last on an axis.
    Reversed { from: [u32; 3], to: [u32; 3] },
    /// An axis other than x, y or z.
    Axis { asked: String },
    /// A step of no frames or of more than [`STEP_MOST`].
    Frames { asked: i64 },
    /// An argument of the wrong shape.
    Malformed { what: String },
}

impl Refused {
    /// The name an answer carries in `refused`.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Outside { .. } => "outside",
            Self::UnknownRule { .. } => "unknown_rule",
            Self::Params { .. } => "params",
            Self::TooMany { .. } => "too_many",
            Self::Closed { .. } => "closed",
            Self::NoWorld { .. } => "no_world",
            Self::Reversed { .. } => "reversed",
            Self::Axis { .. } => "axis",
            Self::Frames { .. } => "frames",
            Self::Malformed { .. } => "malformed",
        }
    }
}

fn listing(known: &[(String, usize)]) -> String {
    known
        .iter()
        .map(|(name, params)| format!("{name} ({params} parameters)"))
        .collect::<Vec<_>>()
        .join(", ")
}

impl std::fmt::Display for Refused {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Outside { what, size } => write!(
                out,
                "{what} is outside the world of {} by {} by {} cells",
                size[0], size[1], size[2]
            ),
            Self::UnknownRule { asked, known } => {
                write!(out, "rule {asked:?} is not one of {}", listing(known))
            }
            Self::Params {
                rule,
                given,
                wanted,
            } => write!(
                out,
                "rule {rule} reads {wanted} parameters, and {given} were given"
            ),
            Self::TooMany { what, cells, most } => {
                write!(
                    out,
                    "{what} names {cells} cells, more than the {most} one call carries"
                )
            }
            Self::Closed { world } => write!(out, "world {world} was closed"),
            Self::NoWorld { world } => write!(out, "no world {world} was ever opened"),
            Self::Reversed { from, to } => write!(
                out,
                "the box from {from:?} to {to:?} has its first corner past its last"
            ),
            Self::Axis { asked } => write!(out, "axis {asked:?} is not one of x, y, z"),
            Self::Frames { asked } => {
                write!(
                    out,
                    "a step of {asked} frames is not one from 1 to {STEP_MOST}"
                )
            }
            Self::Malformed { what } => write!(out, "{what}"),
        }
    }
}

impl std::error::Error for Refused {}

/// The name of any refusal a world tool gives: a [`Refused`], or the
/// engine's own error for a world the device cannot hold.
#[must_use]
pub fn refusal_name(why: &(dyn std::error::Error + 'static)) -> &'static str {
    if let Some(held) = why.downcast_ref::<Refused>() {
        return held.name();
    }
    if why.downcast_ref::<crate::sand::Unfit>().is_some() {
        return "unfit";
    }
    if why.downcast_ref::<crate::compact::TooBig>().is_some() {
        return "too_big";
    }
    if why.downcast_ref::<LaneError>().is_some() {
        return "lane";
    }
    match why.downcast_ref::<crate::host::Error>() {
        Some(crate::host::Error::TooLarge { .. }) => "too_large",
        Some(crate::host::Error::TooManyGroups { .. }) => "too_many_groups",
        Some(crate::host::Error::NoComputeDevice) => "no_compute_device",
        Some(crate::host::Error::Lane(_)) => "lane",
        Some(_) => "device",
        None => "error",
    }
}

type Failed = Box<dyn std::error::Error>;

fn malformed(what: String) -> Failed {
    Box::new(Refused::Malformed { what })
}

fn whole(one: &Json) -> Option<u32> {
    one.int().and_then(|one| u32::try_from(one).ok())
}

fn number(held: &Json, key: &str) -> Result<Option<u32>, Failed> {
    held.get(key).map_or(Ok(None), |one| {
        whole(one).map(Some).ok_or_else(|| {
            malformed(format!(
                "{key} is {}, which is not a whole number from 0 to {}",
                crate::wire::written(one),
                u32::MAX
            ))
        })
    })
}

fn needed(held: &Json, key: &str) -> Result<u32, Failed> {
    number(held, key)?.ok_or_else(|| malformed(format!("{key} is missing")))
}

fn words(one: &Json, what: &str) -> Result<Vec<u32>, Failed> {
    let list = one.list().ok_or_else(|| {
        malformed(format!(
            "{what} is {}, which is not a list",
            crate::wire::written(one)
        ))
    })?;
    list.iter()
        .map(|one| {
            whole(one).ok_or_else(|| {
                malformed(format!(
                    "{what} holds {}, which is not a whole number from 0 to {}",
                    crate::wire::written(one),
                    u32::MAX
                ))
            })
        })
        .collect()
}

fn point(held: &Json, key: &str) -> Result<[u32; 3], Failed> {
    let one = held
        .get(key)
        .ok_or_else(|| malformed(format!("{key} is missing")))?;
    let got = words(one, key)?;
    <[u32; 3]>::try_from(got.as_slice())
        .map_err(|_| malformed(format!("{key} is {got:?}, which is not [x, y, z]")))
}

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
}

impl World<'_> {
    const fn size(&self) -> [u32; 3] {
        [self.solid.wide, self.solid.deep, self.solid.tall]
    }

    fn inside(&self, at: [u32; 3], what: &str) -> Result<(), Failed> {
        if at.iter().zip(self.size()).all(|(one, side)| *one < side) {
            return Ok(());
        }
        Err(Box::new(Refused::Outside {
            what: format!("{what} {at:?}"),
            size: self.size(),
        }))
    }

    fn index(&self, at: [u32; 3]) -> usize {
        self.solid.index(at[0], at[1], at[2])
    }

    fn listed(&mut self) -> Result<u32, Failed> {
        Ok(self
            .live
            .read(GRAND as usize, 1)?
            .first()
            .copied()
            .unwrap_or(0))
    }
}

enum Slot<'host> {
    Open(Box<World<'host>>),
    Closed,
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
pub const WORLD_TOOLS: [&str; 7] = [
    "grit_world_rules",
    "grit_world_create",
    "grit_world_write",
    "grit_world_step",
    "grit_world_read",
    "grit_world_slice",
    "grit_world_close",
];

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
        let world = needed(held, "world")?;
        match self.worlds.get_mut(&world) {
            Some(Slot::Open(open)) => Ok(open),
            Some(Slot::Closed) => Err(Box::new(Refused::Closed { world })),
            None => Err(Box::new(Refused::NoWorld { world })),
        }
    }

    fn created(&mut self, held: &Json) -> Result<Json, Failed> {
        let mut known = rules();
        let listed: Vec<(String, usize)> = known
            .iter()
            .map(|one| (one.name().to_owned(), one.params()))
            .collect();
        let asked = held.get("rule").and_then(Json::text).unwrap_or("");
        let at = known
            .iter()
            .position(|one| one.name() == asked)
            .ok_or_else(|| Refused::UnknownRule {
                asked: asked.to_owned(),
                known: listed,
            })?;
        let rule = known.swap_remove(at);
        let params = match held.get("params") {
            None => Vec::new(),
            Some(one) => words(one, "params")?,
        };
        if params.len() != rule.params() {
            return Err(Box::new(Refused::Params {
                rule: rule.name().to_owned(),
                given: params.len(),
                wanted: rule.params(),
            }));
        }
        let wide = needed(held, "wide")?;
        let deep = needed(held, "deep")?;
        let tall = needed(held, "tall")?;
        let host = self.host.ok_or(crate::host::Error::NoComputeDevice)?;

        let grid = Grid::solid(wide, deep, tall)?;
        let width = host.width();
        let plan = Plan::over_in(
            width,
            grid.blocks().div_ceil(sweep::cells(width)),
            Dimension::Three,
        )?;
        let (passes, stages) = rule.wired(width, grid, &plan)?;
        let solid = Grid3::new(grid.wide, grid.deep, grid.tall).ok_or_else(|| {
            malformed(format!(
                "a world of {wide} by {deep} by {tall} cells is not solid"
            ))
        })?;
        let sizes = rule.sizes(&plan);
        let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
        let mut live = host.resident(&sizes, &modules)?;
        for (binding, room) in sizes.iter().enumerate() {
            live.write(binding, &vec![0_u32; *room])?;
        }
        if !params.is_empty() {
            live.write_at(PARAMS as usize, FIRST_PARAM, &params)?;
        }
        live.write(PARAMS as usize, &[u32::MAX, plan.dump()])?;
        live::seed(&mut live, grid, &plan)?;

        let mut world = World {
            rule,
            live,
            stages,
            grid,
            solid,
            plan,
            frame: 0,
        };
        let listed = world.listed()?;
        let id = self.next;
        self.next = id
            .checked_add(1)
            .ok_or_else(|| malformed("every world number has been used".to_owned()))?;
        let answer = map(vec![
            ("world", counted(id)),
            ("rule", text(world.rule.name())),
            (
                "params",
                Json::List(params.iter().map(|one| counted(*one)).collect()),
            ),
            ("size", sized(world.size())),
            ("cells", Json::Int(i64::try_from(solid.cells())?)),
            ("frame", counted(world.frame)),
            ("listed", counted(listed)),
        ]);
        self.worlds.insert(id, Slot::Open(Box::new(world)));
        Ok(answer)
    }

    fn written(&mut self, held: &Json) -> Result<Json, Failed> {
        let world = self.world(held)?;
        let given = held
            .get("cells")
            .ok_or_else(|| malformed("cells is missing".to_owned()))?;
        let list = given.list().ok_or_else(|| {
            malformed(format!(
                "cells is {}, which is not a list",
                crate::wire::written(given)
            ))
        })?;
        let many = u64::try_from(list.len())?;
        if many > READ_MOST {
            return Err(Box::new(Refused::TooMany {
                what: "the write",
                cells: many,
                most: READ_MOST,
            }));
        }
        let mut cells = Vec::with_capacity(list.len());
        for (at, one) in list.iter().enumerate() {
            let got = words(one, &format!("cell {at}"))?;
            let [x, y, z, word] = <[u32; 4]>::try_from(got.as_slice()).map_err(|_| {
                malformed(format!(
                    "cell {at} is {got:?}, which is not [x, y, z, word]"
                ))
            })?;
            world.inside([x, y, z], &format!("cell {at} at"))?;
            cells.push((u32::try_from(world.index([x, y, z]))?, word));
        }
        live::put3(&mut world.live, world.grid, &world.plan, &cells)?;
        let listed = world.listed()?;
        Ok(map(vec![
            ("world", counted(needed(held, "world")?)),
            ("written", counted(u32::try_from(cells.len())?)),
            ("frame", counted(world.frame)),
            ("listed", counted(listed)),
        ]))
    }

    fn stepped(&mut self, held: &Json) -> Result<Json, Failed> {
        let world = self.world(held)?;
        let asked = match held.get("frames") {
            None => 1,
            Some(one) => one.int().ok_or_else(|| {
                malformed(format!(
                    "frames is {}, which is not a whole number",
                    crate::wire::written(one)
                ))
            })?,
        };
        let frames = u32::try_from(asked)
            .ok()
            .filter(|one| (1..=STEP_MOST).contains(one))
            .ok_or(Refused::Frames { asked })?;
        let groups: Vec<&[Groups]> = world.stages.iter().map(Vec::as_slice).collect();
        let mut done = 0_u32;
        while done < frames {
            let step = BATCH.min(frames - done);
            world.live.staged_frames(&groups, step)?;
            done += step;
        }
        world.frame = world.frame.wrapping_add(frames);
        let tally = world.live.read(TALLY as usize, TALLY_WORDS)?;
        let listed = world.listed()?;
        Ok(map(vec![
            ("world", counted(needed(held, "world")?)),
            ("frames", counted(frames)),
            ("frame", counted(world.frame)),
            (
                "tally",
                Json::List(tally.iter().map(|one| counted(*one)).collect()),
            ),
            ("listed", counted(listed)),
        ]))
    }

    fn read(&mut self, held: &Json) -> Result<Json, Failed> {
        let world = self.world(held)?;
        let from = point(held, "from")?;
        let to = point(held, "to")?;
        world.inside(from, "from")?;
        world.inside(to, "to")?;
        if from.iter().zip(to).any(|(one, other)| *one > other) {
            return Err(Box::new(Refused::Reversed { from, to }));
        }
        let cells: u64 = from
            .iter()
            .zip(to)
            .map(|(one, other)| u64::from(other - one) + 1)
            .product();
        if cells > READ_MOST {
            return Err(Box::new(Refused::TooMany {
                what: "the box",
                cells,
                most: READ_MOST,
            }));
        }
        let state = world.live.read(STATE as usize, world.index(to) + 1)?;
        let mut words = Vec::new();
        let mut kinds = BTreeMap::new();
        for z in from[2]..=to[2] {
            for y in from[1]..=to[1] {
                for x in from[0]..=to[0] {
                    let word = state
                        .get(world.index([x, y, z]))
                        .copied()
                        .ok_or("the world read back short")?;
                    *kinds.entry(world.rule.kind(word)).or_insert(0_u32) += 1;
                    words.push(counted(word));
                }
            }
        }
        Ok(map(vec![
            ("world", counted(needed(held, "world")?)),
            ("frame", counted(world.frame)),
            ("from", sized(from)),
            ("to", sized(to)),
            ("cells", Json::List(words)),
            (
                "kinds",
                Json::List(
                    kinds
                        .into_iter()
                        .map(|(kind, many)| {
                            map(vec![("kind", counted(kind)), ("cells", counted(many))])
                        })
                        .collect(),
                ),
            ),
        ]))
    }

    fn sliced(&mut self, held: &Json) -> Result<Json, Failed> {
        let world = self.world(held)?;
        let axis = held.get("axis").and_then(Json::text).unwrap_or("z");
        let (across, down, through) = match axis {
            "x" => (1, 2, 0),
            "y" => (0, 2, 1),
            "z" => (0, 1, 2),
            other => {
                return Err(Box::new(Refused::Axis {
                    asked: other.to_owned(),
                }));
            }
        };
        let at = needed(held, "at")?;
        let size = world.size();
        let side = |axis: usize| size.get(axis).copied().unwrap_or(0);
        if at >= side(through) {
            return Err(Box::new(Refused::Outside {
                what: format!("layer {axis} {at}"),
                size,
            }));
        }
        let shown = held.get("as").and_then(Json::text).unwrap_or("kinds");
        if shown != "kinds" && shown != "words" {
            return Err(malformed(format!(
                "as is {shown:?}, which is not kinds or words"
            )));
        }
        let cells = u64::from(side(across)) * u64::from(side(down));
        if cells > SLICE_MOST {
            return Err(Box::new(Refused::TooMany {
                what: "the slice",
                cells,
                most: SLICE_MOST,
            }));
        }
        let place = |row: u32, column: u32| {
            let mut held = [0_u32; 3];
            for (axis, value) in [(across, column), (down, row), (through, at)] {
                if let Some(slot) = held.get_mut(axis) {
                    *slot = value;
                }
            }
            held
        };
        let last = place(side(down) - 1, side(across) - 1);
        let state = world.live.read(STATE as usize, world.index(last) + 1)?;
        let mut rows = Vec::new();
        for row in 0..side(down) {
            let mut kinds = String::new();
            let mut words = Vec::new();
            for column in 0..side(across) {
                let word = state
                    .get(world.index(place(row, column)))
                    .copied()
                    .ok_or("the world read back short")?;
                if shown == "kinds" {
                    let kind = world.rule.kind(word);
                    kinds.push(
                        DIGITS
                            .get(kind as usize)
                            .map_or('+', |digit| char::from(*digit)),
                    );
                } else {
                    words.push(counted(word));
                }
            }
            rows.push(if shown == "kinds" {
                text(&kinds)
            } else {
                Json::List(words)
            });
        }
        let named = ["x", "y", "z"];
        let name = |axis: usize| text(named.get(axis).copied().unwrap_or("x"));
        Ok(map(vec![
            ("world", counted(needed(held, "world")?)),
            ("frame", counted(world.frame)),
            ("axis", text(axis)),
            ("at", counted(at)),
            ("columns", name(across)),
            ("rows_along", name(down)),
            ("as", text(shown)),
            ("rows", Json::List(rows)),
        ]))
    }

    fn closed(&mut self, held: &Json) -> Result<Json, Failed> {
        let world = needed(held, "world")?;
        self.world(held)?;
        self.worlds.insert(world, Slot::Closed);
        Ok(map(vec![
            ("world", counted(world)),
            ("closed", Json::Bool(true)),
        ]))
    }

    fn ran(&mut self, name: &str, held: &Json) -> Result<Json, Failed> {
        match name {
            "grit_world_rules" => Ok(map(vec![("rules", known_rules())])),
            "grit_world_create" => self.created(held),
            "grit_world_write" => self.written(held),
            "grit_world_step" => self.stepped(held),
            "grit_world_read" => self.read(held),
            "grit_world_slice" => self.sliced(held),
            _ => self.closed(held),
        }
    }

    /// Answers one request, as [`crate::serve::handle`] does for the 2D
    /// tools.
    pub fn handle(&mut self, request: &Json) -> Option<Json> {
        let way = request.get("method").and_then(Json::text).unwrap_or("");
        let id = request.get("id").cloned()?;
        if way == "tools/list" {
            let mut tools = match crate::serve::tools() {
                Json::List(held) => held,
                other => vec![other],
            };
            tools.extend(world_tools());
            return Some(answered(id, map(vec![("tools", Json::List(tools))])));
        }
        let params = request.get("params");
        let name = params
            .and_then(|one| one.get("name"))
            .and_then(Json::text)
            .unwrap_or("");
        if way != "tools/call" || !WORLD_TOOLS.contains(&name) {
            return crate::serve::handle(self.host, request);
        }
        let arguments = params
            .and_then(|one| one.get("arguments"))
            .cloned()
            .unwrap_or(Json::Map(Vec::new()));
        Some(match self.ran(name, &arguments) {
            Ok(result) => delivered(id, result),
            Err(why) => answered(id, refusal(why.as_ref())),
        })
    }

    /// Reads one line and answers it, as [`crate::serve::heard`] does.
    pub fn heard(&mut self, line: &str) -> Option<Json> {
        let request = match crate::wire::read(line) {
            Ok(request) => request,
            Err(why) => {
                return Some(refused(
                    Json::Null,
                    -32700,
                    &format!("the line is not JSON this server reads: {why}"),
                ));
            }
        };
        let answered =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.handle(&request)));
        answered.unwrap_or_else(|_| {
            request.get("id").cloned().map(|id| {
                refused(
                    id,
                    -32603,
                    "the server hit a fault while answering this request and has no answer for it",
                )
            })
        })
    }
}

fn sized(held: [u32; 3]) -> Json {
    Json::List(held.iter().map(|one| counted(*one)).collect())
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

fn refusal(why: &(dyn std::error::Error + 'static)) -> Json {
    let message = why.to_string();
    let mut told = vec![
        ("refused", text(refusal_name(why))),
        ("message", text(&message)),
    ];
    if matches!(
        why.downcast_ref::<Refused>(),
        Some(Refused::UnknownRule { .. } | Refused::Params { .. })
    ) {
        told.push(("rules", known_rules()));
    }
    let mut answer = match said(&message, true) {
        Json::Map(held) => held,
        other => vec![("content".to_owned(), other)],
    };
    answer.push(("structuredContent".to_owned(), map(told)));
    Json::Map(answer)
}

fn property(kind: &str, what: &str) -> Json {
    map(vec![("type", text(kind)), ("description", text(what))])
}

fn tool(name: &str, what: &str, properties: Vec<(&str, Json)>, required: &[&str]) -> Json {
    map(vec![
        ("name", text(name)),
        ("description", text(what)),
        (
            "inputSchema",
            map(vec![
                ("type", text("object")),
                ("properties", map(properties)),
                (
                    "required",
                    Json::List(required.iter().map(|one| text(one)).collect()),
                ),
            ]),
        ),
    ])
}

fn world_tools() -> Vec<Json> {
    let world = || property("integer", "the number grit_world_create answered with");
    let corner = |what: &str| {
        map(vec![
            ("type", text("array")),
            ("items", map(vec![("type", text("integer"))])),
            ("description", text(what)),
        ])
    };
    vec![
        tool(
            "grit_world_rules",
            "Name the rules a world can follow and how many parameters each reads.",
            Vec::new(),
            &[],
        ),
        tool(
            "grit_world_create",
            "Open a solid world of wide by deep by tall cells, each side rounded up to a power of two, every cell 0, under a rule. Returns its number. Refused when the device cannot hold it.",
            vec![
                ("wide", property("integer", "cells along x")),
                ("deep", property("integer", "cells along y, which is down")),
                ("tall", property("integer", "cells along z; at least 2")),
                ("rule", property("string", "a name grit_world_rules lists")),
                (
                    "params",
                    map(vec![
                        ("type", text("array")),
                        ("items", map(vec![("type", text("integer"))])),
                        (
                            "description",
                            text(
                                "exactly as many words as the rule reads; heat reads ignition, chance out of 16, flare",
                            ),
                        ),
                    ]),
                ),
            ],
            &["wide", "deep", "tall", "rule"],
        ),
        tool(
            "grit_world_write",
            &format!(
                "Set cells between frames and wake them, so the next frame steps them. At most {READ_MOST} cells a call; the last write to a cell wins."
            ),
            vec![
                ("world", world()),
                (
                    "cells",
                    map(vec![
                        ("type", text("array")),
                        (
                            "items",
                            map(vec![
                                ("type", text("array")),
                                ("items", map(vec![("type", text("integer"))])),
                            ]),
                        ),
                        ("description", text("each [x, y, z, word]")),
                    ]),
                ),
            ],
            &["world", "cells"],
        ),
        tool(
            "grit_world_step",
            &format!(
                "Run frames, 1 to {STEP_MOST}. Returns the frame count so far, the 7 tally words of the last frame and how many octs it lists for the next."
            ),
            vec![
                ("world", world()),
                (
                    "frames",
                    property("integer", "how many frames; 1 if left out"),
                ),
            ],
            &["world"],
        ),
        tool(
            "grit_world_read",
            &format!(
                "Read the cells of a box, corners included, x fastest, then y, then z, and count them by kind. At most {READ_MOST} cells."
            ),
            vec![
                ("world", world()),
                ("from", corner("the least corner, [x, y, z]")),
                ("to", corner("the greatest corner, [x, y, z]")),
            ],
            &["world", "from", "to"],
        ),
        tool(
            "grit_world_slice",
            &format!(
                "Read one layer across an axis as rows. Kinds are one hex digit a cell, + past 15; words are numbers. At most {SLICE_MOST} cells."
            ),
            vec![
                ("world", world()),
                ("axis", property("string", "x, y or z; z if left out")),
                ("at", property("integer", "which layer along the axis")),
                (
                    "as",
                    property("string", "kinds or words; kinds if left out"),
                ),
            ],
            &["world", "at"],
        ),
        tool(
            "grit_world_close",
            "Close a world and free its memory on the device. Its number is never used again.",
            vec![("world", world())],
            &["world"],
        ),
    ]
}
