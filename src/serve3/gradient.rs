use crate::law::Law;
use crate::law::adjoint::{self, Aim, Descender, Fault, Next, Reversible, Settled};
use crate::law::dag::{FIRST_PARAM, Node};
use crate::question::Region;
use crate::sand3::Grid3;
use crate::wire::{Json, counted, map, text};

use super::budget::{Meter, Stop, Work};
use super::errors::{Code, Refusal};
use super::schema::{object, property, whole, word};
use super::tools::{exactly, int, ints, limited, refused, tool, unread, words_of};
use super::{Failed, READ_MOST, Server, Tool};

pub const TOOLS: &[Tool] = &[
    Tool {
        name: "grit_law_gradient",
        shown: gradient_shown,
        run: graded,
    },
    Tool {
        name: "grit_law_descend",
        shown: descend_shown,
        run: descended,
    },
];

pub const CELLS_MOST: u64 = 1 << 15;

pub const FRAMES_MOST: u32 = 512;

pub const STEPS_MOST: u32 = 256;

pub const WORK_MOST: u64 = 1 << 26;

pub const OCT_UNITS: u64 = 128;

pub const LOOK_UNITS: u64 = 3 * crate::halo::BOX as u64;

pub const SIDE_MOST: u32 = 1024;

pub const GRADIENT: &str = "grit_law_gradient and grit_law_descend run on the CPU, exactly as the library's Reversible::walk and Reversible::descend. The start is a world's cells on every plane at its frame, or law, size and cells given inline from frame first (0 if left out). The objective is the sum over the aim's box, on plane 0, of |cell - target| after frames frames. gradient[i] is d objective / d params[i] times 2^64, a decimal string: the derivative of every add, sub, mul, div, shift and sel arm on the way from params[i] to a cell, carried back by recomputing every frame with the inverse, where a compare (and so a sel's choice) counts as constant; a look (halo 1) reads a frozen plane, so it moves with no param, and the walk back reads the same neighbour at the same frame and carries the look's share back to that cell; a walk that does not come home answers strayed. A law is refused no_gradient, at the path of the first such node, when a not, and, or, xor, dot_s or dot_u takes a value that moves with a param (directly or through a cell an earlier frame moved) and its result reaches a cell other than through a compare. A look at a plane the law writes is refused when the law compiles (halo_written). A descent walks its start, then walks one trial point per step: the secant step of the gradients of its last walks, after a trial that is not lower one retry at most as long and then each at most half as long, then moves of 1 in one param against its gradient. It keeps a trial only when its objective is lower than the kept point's, so its answer is never worse than its start and the kept objectives fall strictly; it finds a local optimum, not the global one. It stops converged (objective 0), local (no trial it can make is lower), steps, or on budget or cancelled before a walk, and answers the last kept point; the trail holds every point walked, with kept. A walk costs its octs times frames times (the nodes of the law and its inverse plus 128, plus 192 for each plane the law looks at); it may cost at most walk_work, on a start of at most walk_cells words, and cannot stop part way, so an answer can come one walk late. Refused before any walk: not_reversible (with the lifting refusal as law), no_gradient (with its path as law), params, outside, reversed, target_size, aim, start, walk_limit, and budget when the fastest of the last three walks the server measured at the largest size not above this one, by its thread's CPU time and never time spent waiting, does not fit the time left. Neither writes a world or its journal: set learned params with grit_world_params";

pub mod faults {
    #[cfg(feature = "faults")]
    use std::cell::Cell;

    #[cfg(feature = "faults")]
    thread_local! {
        static WALKS: Cell<u64> = const { Cell::new(0) };
        static CANCEL: Cell<Option<u64>> = const { Cell::new(None) };
    }

    #[cfg(feature = "faults")]
    #[must_use]
    pub fn walks() -> u64 {
        WALKS.get()
    }

    #[cfg(feature = "faults")]
    pub fn cancel_after(walks: u64) {
        CANCEL.set(Some(walks));
    }

    #[cfg(feature = "faults")]
    pub fn disarm() {
        CANCEL.set(None);
    }

    #[cfg(feature = "faults")]
    pub(super) fn walked() -> bool {
        WALKS.set(WALKS.get().saturating_add(1));
        match CANCEL.get() {
            Some(0 | 1) => {
                CANCEL.set(None);
                true
            }
            Some(left) => {
                CANCEL.set(Some(left - 1));
                false
            }
            None => false,
        }
    }

    #[cfg(not(feature = "faults"))]
    pub(super) const fn walked() -> bool {
        false
    }
}

fn corner_shown(what: &str) -> Json {
    map(vec![
        ("type", text("array")),
        ("items", word()),
        ("description", text(what)),
        ("minItems", Json::Int(3)),
        ("maxItems", Json::Int(3)),
    ])
}

fn words_shown(what: &str, most: i64) -> Json {
    map(vec![
        ("type", text("array")),
        ("items", word()),
        ("description", text(what)),
        ("maxItems", Json::Int(most)),
    ])
}

fn most(held: u64) -> i64 {
    i64::try_from(held).unwrap_or(i64::MAX)
}

fn posed_shown() -> Vec<(&'static str, Json)> {
    vec![
        (
            "world",
            whole(
                "a world of a reversible law: the start is its cells at its frame; give this or law",
                1,
                i64::from(u32::MAX),
            ),
        ),
        (
            "law",
            property(
                "string",
                "the JSON text of a reversible law, with size and cells; give this or world",
            ),
        ),
        (
            "size",
            map(vec![
                ("type", text("array")),
                (
                    "items",
                    whole("a side, a power of two", 2, i64::from(SIDE_MOST)),
                ),
                (
                    "description",
                    text("with law: the start's [wide, deep, tall]"),
                ),
                ("minItems", Json::Int(3)),
                ("maxItems", Json::Int(3)),
            ]),
        ),
        (
            "cells",
            words_shown(
                "with law: every cell of the start, plane by plane, x fastest, then y, then z",
                most(READ_MOST),
            ),
        ),
        (
            "first",
            whole(
                "with law: the frame the start is at; 0 if left out",
                0,
                i64::from(u32::MAX),
            ),
        ),
        (
            "params",
            words_shown(
                "the law's params; the world's if left out",
                most(crate::law::limits::PARAMS as u64),
            ),
        ),
        (
            "frames",
            whole(
                "how many frames run from the start before the objective",
                1,
                i64::from(FRAMES_MOST),
            ),
        ),
        (
            "aim",
            object(
                vec![
                    ("from", corner_shown("the box's least corner, [x, y, z]")),
                    ("to", corner_shown("the box's greatest corner, [x, y, z]")),
                    (
                        "target",
                        words_shown(
                            "the box's wanted cells, x fastest, then y, then z; give this or world",
                            most(READ_MOST),
                        ),
                    ),
                    (
                        "world",
                        whole(
                            "take the target from this world's box, on plane 0; give this or target",
                            1,
                            i64::from(u32::MAX),
                        ),
                    ),
                ],
                &["from", "to"],
            ),
        ),
    ]
}

fn gradient_shown() -> Json {
    tool(
        "grit_law_gradient",
        "Exact gradient of a reversible law's params: the objective |cells - target| over a box after n frames, and d objective / d param times 2^64 per param.",
        posed_shown(),
        &["frames", "aim"],
    )
}

fn descend_shown() -> Json {
    let mut properties = posed_shown();
    properties.push((
        "steps",
        whole(
            "the most trial points walked after the first walk",
            1,
            i64::from(STEPS_MOST),
        ),
    ));
    tool(
        "grit_law_descend",
        "Descend a reversible law's params by exact gradients toward the aim, keeping only lower points: never worse than the start, a local optimum at best. Returns the best params, objective, trail and stop.",
        properties,
        &["frames", "aim", "steps"],
    )
}

#[derive(Debug)]
pub struct Told(pub Refusal, pub Vec<(&'static str, Json)>);

impl std::fmt::Display for Told {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(out, "{}", self.0.message)
    }
}

impl std::error::Error for Told {}

struct Posed {
    reversible: Reversible,
    grid: Grid3,
    start: Vec<u32>,
    first: u32,
    frames: u32,
    params: Vec<u32>,
    region: Region,
    target: Vec<u32>,
    units: u64,
}

impl Posed {
    fn aim(&self) -> Aim<'_> {
        Aim {
            region: self.region,
            target: &self.target,
        }
    }
}

fn not_reversible(field: &str, why: &crate::law::Refusal) -> Failed {
    Box::new(Told(
        Refusal::new(
            Code::NotReversible,
            field,
            format!("the law has no exact gradient: {why}; nothing ran"),
        ),
        vec![("law", why.json())],
    ))
}

struct Start {
    law: Law,
    grid: Grid3,
    planes: u32,
    first: u32,
    params: Option<Vec<u32>>,
    world: Option<u32>,
    field: &'static str,
}

fn started(server: &mut Server<'_>, held: &Json) -> Result<Start, Failed> {
    match (held.get("world"), held.get("law")) {
        (Some(_), None) => {
            for key in ["size", "cells", "first"] {
                if held.get(key).is_some() {
                    return Err(refused(
                        Code::Start,
                        &format!("$.{key}"),
                        format!("{key} goes with law; a world's start is its own cells"),
                    ));
                }
            }
            let number = int(held, "world")?;
            let live = server.world(held)?;
            let Some(law) = live.world.law().cloned() else {
                return Err(refused(
                    Code::NotReversible,
                    "$.world",
                    format!(
                        "the world runs the rule {}, not a law declaring \"reversible\": true; nothing ran",
                        live.world.rule()
                    ),
                ));
            };
            let size = live.world.size();
            let grid = Grid3::new(size[0], size[1], size[2]).ok_or("the world has no grid")?;
            Ok(Start {
                law,
                grid,
                planes: live.world.planes(),
                first: live.world.frame(),
                params: Some(live.world.params().to_vec()),
                world: Some(number),
                field: "$.world",
            })
        }
        (None, Some(_)) => {
            let law = Law::compile(words_of(held, "law")?).map_err(|why| {
                if why.code == crate::law::Code::Lifting {
                    not_reversible("$.law", &why)
                } else {
                    Box::new(super::law::Refused(why, "$.law")) as Failed
                }
            })?;
            let size = held.get("size").ok_or_else(|| {
                refused(
                    Code::Start,
                    "$.size",
                    "a start given inline needs size and cells".to_owned(),
                )
            })?;
            let size = ints(size, "size")?;
            let [wide, deep, tall] =
                <[u32; 3]>::try_from(size.as_slice()).map_err(|_| unread("size"))?;
            let grid = Grid3::new(wide, deep, tall)
                .filter(|grid| [grid.wide, grid.deep, grid.tall] == [wide, deep, tall])
                .ok_or_else(|| {
                    refused(
                        Code::Start,
                        "$.size",
                        format!(
                            "size {:?} has a side that is not a power of two from 2",
                            [wide, deep, tall]
                        ),
                    )
                })?;
            let planes = crate::rule3::Declared::planes(&law);
            Ok(Start {
                law,
                grid,
                planes: u32::try_from(planes).unwrap_or(u32::MAX),
                first: match held.get("first") {
                    None => 0,
                    Some(_) => int(held, "first")?,
                },
                params: None,
                world: None,
                field: "$.law",
            })
        }
        (given, _) => Err(refused(
            Code::Start,
            "$",
            if given.is_some() {
                "give a world or a law, and not both".to_owned()
            } else {
                "give a world, whose cells are the start, or a law with size and cells".to_owned()
            },
        )),
    }
}

fn reversible_of(start: &Start) -> Result<Reversible, Failed> {
    let reversible = Reversible::of(&start.law).map_err(|why| not_reversible(start.field, &why))?;
    if let Some((node, op)) = cut(reversible.forward()) {
        let path = start
            .law
            .places()
            .path(node)
            .unwrap_or_else(|| String::from("$.step"));
        return Err(Box::new(Told(
            Refusal::new(
                Code::NoGradient,
                start.field,
                format!(
                    "the law {} takes {op} of a value that moves with its params, at {path}, on the way to a cell; the adjoint carries no gradient through {op}, so the gradient would be wrong, and nothing ran",
                    start.law.name()
                ),
            ),
            vec![(
                "law",
                map(vec![
                    ("code", text("no_gradient")),
                    ("path", text(&path)),
                    ("found", text(op)),
                ]),
            )],
        )));
    }
    Ok(reversible)
}

enum Edges {
    Carried(Vec<u32>),
    Flat,
    Cut(&'static str, Vec<u32>),
}

fn edges(node: Node) -> Edges {
    match node {
        Node::Input(_) | Node::Const(_) => Edges::Carried(Vec::new()),
        Node::Add(one, other)
        | Node::Sub(one, other)
        | Node::Mul(one, other)
        | Node::Div(one, other) => Edges::Carried(vec![one, other]),
        Node::Shl(one, _) | Node::Shr(one, _) => Edges::Carried(vec![one]),
        Node::Select(_, then, otherwise) => Edges::Carried(vec![then, otherwise]),
        Node::Eq(..) | Node::Ne(..) | Node::Lt(..) | Node::Le(..) | Node::Gt(..) | Node::Ge(..) => {
            Edges::Flat
        }
        Node::Not(one) => Edges::Cut("not", vec![one]),
        Node::And(one, other) => Edges::Cut("and", vec![one, other]),
        Node::Or(one, other) => Edges::Cut("or", vec![one, other]),
        Node::Xor(one, other) => Edges::Cut("xor", vec![one, other]),
        Node::DotS(one, other) => Edges::Cut("dot_s", vec![one, other]),
        Node::DotU(one, other) => Edges::Cut("dot_u", vec![one, other]),
    }
}

fn outputs(dag: &crate::law::dag::Dag) -> Vec<u32> {
    let mut outs = dag.out().to_vec();
    for plane in dag.planes() {
        outs.extend(plane.iter().copied());
    }
    outs
}

fn word_input(dag: &crate::law::dag::Dag, which: u32) -> Option<usize> {
    if which < 8 {
        return usize::try_from(which).ok();
    }
    let above = usize::try_from(which.checked_sub(dag.first_plane())?).ok()?;
    (above < 8 * dag.planes().len()).then_some(8 + above)
}

fn param_input(dag: &crate::law::dag::Dag, which: u32) -> bool {
    (FIRST_PARAM..dag.first_plane()).contains(&which)
}

fn moved(dag: &crate::law::dag::Dag, words: &[bool]) -> Vec<bool> {
    let held = |id: u32, made: &[bool]| made.get(id as usize).copied().unwrap_or(false);
    let mut made: Vec<bool> = Vec::with_capacity(dag.len());
    for &node in dag.nodes() {
        let one = match (node, edges(node)) {
            (Node::Input(which), _) => {
                param_input(dag, which)
                    || word_input(dag, which)
                        .and_then(|word| words.get(word).copied())
                        .unwrap_or(false)
            }
            (_, Edges::Carried(from) | Edges::Cut(_, from)) => {
                from.iter().any(|&id| held(id, &made))
            }
            (_, Edges::Flat) => false,
        };
        made.push(one);
    }
    made
}

#[must_use]
pub fn cut(dag: &crate::law::dag::Dag) -> Option<(u32, &'static str)> {
    let outs = outputs(dag);
    let mut words = vec![false; outs.len()];
    let made = loop {
        let made = moved(dag, &words);
        let mut next = words.clone();
        for (word, &out) in outs.iter().enumerate() {
            if made.get(out as usize).copied().unwrap_or(false) {
                let plane = word / 8 * 8;
                let corner = word % 8;
                for one in [plane + corner, plane + 7 - corner] {
                    if let Some(slot) = next.get_mut(one) {
                        *slot = true;
                    }
                }
            }
        }
        if next == words {
            break made;
        }
        words = next;
    };
    let mut live = vec![false; dag.len()];
    for &out in &outs {
        if let Some(slot) = live.get_mut(out as usize) {
            *slot = true;
        }
    }
    let mut found = None;
    for (index, &node) in dag.nodes().iter().enumerate().rev() {
        if !live.get(index).copied().unwrap_or(false) {
            continue;
        }
        match edges(node) {
            Edges::Carried(from) => {
                for id in from {
                    if let Some(slot) = live.get_mut(id as usize) {
                        *slot = true;
                    }
                }
            }
            Edges::Flat => {}
            Edges::Cut(op, from) => {
                if from
                    .iter()
                    .any(|&id| made.get(id as usize).copied().unwrap_or(false))
                {
                    found = Some((u32::try_from(index).unwrap_or(u32::MAX), op));
                }
            }
        }
    }
    found
}

fn corner_of(held: &Json, key: &str) -> Result<[u32; 3], Failed> {
    let got = ints(held.get(key).ok_or_else(|| unread(key))?, key)?;
    <[u32; 3]>::try_from(got.as_slice()).map_err(|_| unread(key))
}

pub(super) fn region_of(aim: &Json, size: [u32; 3]) -> Result<Region, Failed> {
    let from = corner_of(aim, "from")?;
    let to = corner_of(aim, "to")?;
    for (corner, field) in [(from, "$.aim.from"), (to, "$.aim.to")] {
        if corner.iter().zip(size).any(|(one, side)| *one >= side) {
            return Err(limited(
                Code::Outside,
                field,
                format!(
                    "{corner:?} is outside the world of {} by {} by {} cells",
                    size[0], size[1], size[2]
                ),
                super::tools::sized(size.map(|side| side.saturating_sub(1))),
            ));
        }
    }
    if from.iter().zip(to).any(|(one, other)| *one > other) {
        return Err(refused(
            Code::Reversed,
            "$.aim.to",
            format!("the box from {from:?} to {to:?} has its first corner past its last"),
        ));
    }
    Ok(Region::new(from, to.map(|one| one + 1)))
}

pub(super) fn points_of(region: Region) -> Vec<[u32; 3]> {
    let mut points = Vec::new();
    for z in region.from[2]..region.to[2] {
        for y in region.from[1]..region.to[1] {
            for x in region.from[0]..region.to[0] {
                points.push([x, y, z]);
            }
        }
    }
    points
}

fn looked(reversible: &Reversible) -> u64 {
    u64::try_from(reversible.forward().halo().looking()).unwrap_or(u64::MAX)
}

fn units_of(reversible: &Reversible, grid: Grid3, frames: u32) -> u64 {
    let nodes = u64::try_from(reversible.forward().len() + reversible.inverse().len())
        .unwrap_or(u64::MAX)
        .saturating_add(OCT_UNITS)
        .saturating_add(LOOK_UNITS.saturating_mul(looked(reversible)));
    u64::from(grid.octs())
        .saturating_mul(u64::from(frames))
        .saturating_mul(nodes)
}

fn posed(server: &mut Server<'_>, held: &Json) -> Result<Posed, Failed> {
    let start = started(server, held)?;
    let reversible = reversible_of(&start)?;
    let grid = start.grid;
    let size = [grid.wide, grid.deep, grid.tall];
    let cells = u64::try_from(grid.cells())
        .unwrap_or(u64::MAX)
        .saturating_mul(u64::from(start.planes));
    if cells > CELLS_MOST {
        return Err(limited(
            Code::WalkLimit,
            start.field,
            format!(
                "the start holds {cells} words ({} cells on {} planes), more than the {CELLS_MOST} a walk on the CPU takes",
                grid.cells(),
                start.planes
            ),
            Json::Int(most(CELLS_MOST)),
        ));
    }
    let params = match (held.get("params"), start.params) {
        (Some(given), _) => ints(given, "params")?,
        (None, Some(own)) => own,
        (None, None) => Vec::new(),
    };
    let wanted = crate::rule3::Declared::params(reversible.forward());
    if params.len() != wanted {
        return Err(limited(
            Code::Params,
            "$.params",
            format!(
                "law {} reads {wanted} parameters, and {} were given",
                start.law.name(),
                params.len()
            ),
            counted(u32::try_from(wanted).unwrap_or(u32::MAX)),
        ));
    }
    let given = match held.get("cells") {
        Some(listed) => Some(ints(listed, "cells")?),
        None => None,
    };
    if start.world.is_none() {
        let length = given.as_ref().map_or(0, Vec::len) as u64;
        if length != cells {
            return Err(limited(
                Code::Start,
                "$.cells",
                format!(
                    "a start of size {size:?} on {} planes holds {cells} words, and {length} were given",
                    start.planes
                ),
                Json::Int(most(cells)),
            ));
        }
    }
    let frames = int(held, "frames")?;
    let aim = held.get("aim").ok_or_else(|| unread("aim"))?;
    let region = region_of(aim, size)?;
    let boxed = region.cells();
    let target_world = match (aim.get("target"), aim.get("world")) {
        (Some(listed), None) => {
            let target = ints(listed, "target")?;
            if target.len() as u128 != boxed {
                return Err(limited(
                    Code::TargetSize,
                    "$.aim.target",
                    format!(
                        "the box holds {boxed} cells, and the target {}",
                        target.len()
                    ),
                    Json::Int(i64::try_from(boxed).unwrap_or(i64::MAX)),
                ));
            }
            Err(target)
        }
        (None, Some(_)) => {
            let number = int(aim, "world")?;
            let other = server.world_at(number, "$.aim.world")?;
            let theirs = other.world.size();
            if region.to.iter().zip(theirs).any(|(to, side)| *to > side) {
                return Err(limited(
                    Code::Outside,
                    "$.aim.world",
                    format!(
                        "the box ends at {:?}, outside world {number} of {} by {} by {} cells",
                        region.to.map(|one| one - 1),
                        theirs[0],
                        theirs[1],
                        theirs[2]
                    ),
                    super::tools::sized(theirs.map(|side| side.saturating_sub(1))),
                ));
            }
            Ok(number)
        }
        (given, _) => {
            return Err(refused(
                Code::Aim,
                "$.aim",
                if given.is_some() {
                    "give the aim a target or a world, and not both".to_owned()
                } else {
                    "give the aim a target, its wanted cells, or a world to take them from"
                        .to_owned()
                },
            ));
        }
    };
    let units = units_of(&reversible, grid, frames);
    if units > WORK_MOST {
        let each = units / u64::from(frames.max(1));
        let fits = u32::try_from(WORK_MOST / each.max(1)).unwrap_or(u32::MAX);
        let planes = looked(&reversible);
        let looks = if planes == 0 {
            String::new()
        } else {
            format!(" plus {LOOK_UNITS} for each of the {planes} planes it looks at",)
        };
        return Err(limited(
            Code::WalkLimit,
            if fits == 0 { start.field } else { "$.frames" },
            format!(
                "a walk of {frames} frames over {} octs of a law of {} nodes with an inverse of {} is {units} units of work (octs times frames times the nodes of both plus {OCT_UNITS}{looks}), more than the {WORK_MOST} one walk may take; at most {fits} frames fit",
                grid.octs(),
                reversible.forward().len(),
                reversible.inverse().len()
            ),
            counted(fits),
        ));
    }
    server.fits(Work::Walk, units, "one walk out and back")?;
    let start_cells = match (start.world, given) {
        (Some(number), _) => world_cells(server, number, grid, start.planes)?,
        (None, Some(given)) => given,
        (None, None) => Vec::new(),
    };
    let target = match target_world {
        Err(target) => target,
        Ok(number) => server
            .world_at(number, "$.aim.world")?
            .world
            .cells_at_plane(&points_of(region), 0)?,
    };
    Ok(Posed {
        reversible,
        grid,
        start: start_cells,
        first: start.first,
        frames,
        params,
        region,
        target,
        units,
    })
}

pub(super) fn world_cells(
    server: &mut Server<'_>,
    number: u32,
    grid: Grid3,
    planes: u32,
) -> Result<Vec<u32>, Failed> {
    let points = points_of(Region::whole([grid.wide, grid.deep, grid.tall]));
    let live = server.world_at(number, "$.world")?;
    let mut cells = Vec::with_capacity(points.len() * planes as usize);
    for plane in 0..planes {
        cells.extend(live.world.cells_at_plane(&points, plane)?);
    }
    Ok(cells)
}

fn strayed_at(grid: Grid3, at: usize) -> ([u32; 3], u32) {
    let stride = grid.cells().max(1);
    let plane = u32::try_from(at / stride).unwrap_or(u32::MAX);
    let index = u32::try_from(at % stride).unwrap_or(0);
    let wide = grid.wide.max(1);
    let deep = grid.deep.max(1);
    (
        [index % wide, (index / wide) % deep, index / wide / deep],
        plane,
    )
}

fn fault(grid: Grid3, why: Fault) -> Failed {
    match why {
        Fault::Strayed { at, was, now } => {
            let (cell, plane) = strayed_at(grid, at);
            let three = Json::List(cell.iter().map(|one| counted(*one)).collect());
            Box::new(Told(
                Refusal::new(
                    Code::Strayed,
                    "$",
                    format!(
                        "the walk back did not come home: cell {cell:?} of plane {plane} was {was} and came back {now}; no world was touched"
                    ),
                ),
                vec![(
                    "strayed",
                    map(vec![
                        ("cell", three),
                        ("plane", counted(plane)),
                        ("was", counted(was)),
                        ("now", counted(now)),
                    ]),
                )],
            ))
        }
        Fault::Overflow => refused(
            Code::Overflow,
            "$.frames",
            format!("{why}; no world was touched"),
        ),
        other => refused(Code::Malformed, "$", other.to_string()),
    }
}

#[cfg(feature = "faults")]
fn walker(server: &Server<'_>, posed: &Posed) -> Reversible {
    let armed = server
        .book
        .faults
        .lock()
        .map(|mut faults| std::mem::take(&mut faults.stray_walk))
        .unwrap_or(false);
    if !armed {
        return posed.reversible.clone();
    }
    let mut steps = posed.reversible.steps().to_vec();
    if let Some(first) = steps.first_mut() {
        first.how = match first.how {
            adjoint::Lifting::Add => adjoint::Lifting::Sub,
            adjoint::Lifting::Sub | adjoint::Lifting::Xor => adjoint::Lifting::Add,
        };
    }
    Reversible::from_steps(posed.reversible.forward(), steps)
}

#[cfg(not(feature = "faults"))]
fn walker(_: &Server<'_>, posed: &Posed) -> Reversible {
    posed.reversible.clone()
}

fn walk(
    server: &mut Server<'_>,
    reversible: &Reversible,
    posed: &Posed,
    params: &[u32],
) -> Result<adjoint::Walked, Failed> {
    let meter = Meter::start();
    super::tools::faults::stall(1);
    let walked = reversible.walk(
        posed.grid,
        &posed.start,
        params,
        posed.first,
        posed.frames,
        &posed.aim(),
    );
    server
        .book
        .rates
        .measured(Work::Walk, posed.units, meter.spent());
    if faults::walked()
        && let Some(flag) = server.cancel.as_ref()
    {
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    walked.map_err(|why| fault(posed.grid, why))
}

fn listed(held: &[u32]) -> Json {
    Json::List(held.iter().map(|one| counted(*one)).collect())
}

fn told(value: i128) -> Json {
    text(&value.to_string())
}

fn graded(server: &mut Server<'_>, held: &Json) -> Result<Json, Failed> {
    let posed = posed(server, held)?;
    let reversible = walker(server, &posed);
    let walked = walk(server, &reversible, &posed, &posed.params)?;
    let mut answer = Vec::new();
    if let Some(number) = held.get("world").and_then(Json::int) {
        answer.push(("world", Json::Int(number)));
    }
    answer.extend([
        ("params", listed(&posed.params)),
        ("first", counted(posed.first)),
        ("frames", counted(posed.frames)),
        ("objective", exactly(u128::from(walked.objective))),
        (
            "gradient",
            Json::List(walked.told.iter().map(|one| told(*one)).collect()),
        ),
        ("fraction", counted(adjoint::FRACTION)),
        ("home", Json::Bool(true)),
    ]);
    Ok(map(answer))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Why {
    Settled(Settled),
    Stopped(Stop),
}

impl Why {
    const fn name(self) -> &'static str {
        match self {
            Self::Settled(Settled::Converged) => "converged",
            Self::Settled(Settled::Local) => "local",
            Self::Settled(Settled::Steps) => "steps",
            Self::Stopped(stop) => stop.name(),
        }
    }
}

fn halted(server: &Server<'_>, units: u64, began: bool) -> Option<Stop> {
    if let Some(stop) = server.stop() {
        return Some(stop);
    }
    if !began {
        return None;
    }
    let (least, _) = server.book.rates.least_of(Work::Walk, units);
    (least > server.clock.left()).then_some(Stop::Budget)
}

fn descended(server: &mut Server<'_>, held: &Json) -> Result<Json, Failed> {
    let most = int(held, "steps")?;
    let posed = posed(server, held)?;
    let reversible = walker(server, &posed);
    if let Some(stop) = halted(server, posed.units, false) {
        return Err(refused(
            stop.code(),
            if stop == Stop::Budget {
                "$.budget_ms"
            } else {
                "$"
            },
            format!(
                "the descent stopped on {} before its first walk; nothing ran",
                stop.name()
            ),
        ));
    }
    let first = walk(server, &reversible, &posed, &posed.params)?;
    let mut descender = Descender::new(&posed.params, &first, most);
    let why = loop {
        let params = match descender.next() {
            Next::Settled(settled) => break Why::Settled(settled),
            Next::Walk(params) => params,
        };
        if let Some(stop) = halted(server, posed.units, true) {
            break Why::Stopped(stop);
        }
        let walked = walk(server, &reversible, &posed, &params)?;
        descender.walked(&walked);
    };
    let descent = descender.descent();
    let mut answer = Vec::new();
    if let Some(number) = held.get("world").and_then(Json::int) {
        answer.push(("world", Json::Int(number)));
    }
    answer.extend([
        ("params", listed(&descent.params)),
        ("objective", exactly(u128::from(descent.objective))),
        ("stopped", text(why.name())),
        ("steps", counted(descent.steps)),
        (
            "trail",
            Json::List(
                descent
                    .trail
                    .iter()
                    .map(|tried| {
                        map(vec![
                            ("params", listed(&tried.params)),
                            ("objective", exactly(u128::from(tried.objective))),
                            ("kept", Json::Bool(tried.kept)),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("first", counted(posed.first)),
        ("frames", counted(posed.frames)),
    ]);
    if let Why::Stopped(stop) = why {
        answer.push(("hint", text(stop.code().hint())));
    }
    Ok(map(answer))
}
