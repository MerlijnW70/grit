use crate::law::Law;
use crate::law::adjoint::Aim;
use crate::question::Region;
use crate::rule3::Declared;
use crate::sand3::{Grid3, reference};
use crate::wire::{Json, counted, map, text};

use super::budget::{Stop, Work};
use super::errors::Code;
use super::gradient::{points_of, region_of, world_cells};
use super::schema::{object, property, whole, word};
use super::tools::{exactly, int, ints, limited, refused, tool, unread, words_of};
use super::{Failed, READ_MOST, Server, Tool};

pub const TOOLS: &[Tool] = &[
    Tool {
        name: "grit_law_score",
        shown: score_shown,
        run: scored,
    },
    Tool {
        name: "grit_learn_params",
        shown: search_shown,
        run: searched,
    },
];

pub const CELLS_MOST: u64 = 1 << 15;

pub const FRAMES_MOST: u32 = 4096;

pub const WORK_MOST: u64 = 1 << 26;

pub const OCT_UNITS: u64 = 128;

pub const SIDE_MOST: u32 = 1024;

pub const POPULATION_MOST: u32 = 256;

pub const GENERATIONS_MOST: u32 = 1024;

pub const STALE: u32 = 8;

pub const SEARCH: &str = "grit_law_score and grit_learn_params run any law forward on the CPU with sand3::reference, the CPU twin every device kernel is tested equal to: reversible or not, with cut ops or looks. The start is a world's cells on every plane at its frame, or law, size and cells given inline from frame first (0 if left out). The objective is the sum over the aim's box, on plane 0, of |cell - target| after frames frames, as grit_law_gradient. A run costs its octs times frames times the law's nodes plus 128; it may cost at most run_work, on a start of at most run_cells words, and cannot stop part way, so an answer can come one run late. A search is a (1+population) evolution strategy over whole params inside bounds, drawn from seed: generation 0 runs params; each later generation runs population children of the parent, each param moved by the sum of three draws from 0 to 2 spread, less 3 spread, and clamped to its bounds (a child equal to its parent moves one param by 1); the earliest lowest child becomes the parent when it is no worse. Each param's spread starts at a quarter of its bounds' width, at least 1; it grows by half when more than a fifth of a generation's children were no worse than the parent and shrinks by a third when fewer were, from 1 to the width. After search_stale generations in a row without a lower parent, with every spread at 1, the parent restarts at a point drawn evenly inside bounds and every spread starts again. It stops found (objective 0), generations, or on budget or cancelled before a run, and answers the earliest lowest objective of every run with its params, the trail of each finished generation's earliest lowest, and scored, the runs made. The same arguments give the same answer on every device; a stopped search's trail is a prefix of the whole one's. Refused before any run: law_refused, start, params, bounds, reversed, outside, target_size, aim, run_limit, and budget when the fastest of the last three runs the server measured at the largest size not above this one does not fit the time left. Neither writes a world or its journal: set learned params with grit_world_params";

pub mod faults {
    #[cfg(feature = "faults")]
    use std::cell::Cell;

    #[cfg(feature = "faults")]
    thread_local! {
        static RUNS: Cell<u64> = const { Cell::new(0) };
        static CANCEL: Cell<Option<u64>> = const { Cell::new(None) };
    }

    #[cfg(feature = "faults")]
    #[must_use]
    pub fn runs() -> u64 {
        RUNS.get()
    }

    #[cfg(feature = "faults")]
    pub fn cancel_after(runs: u64) {
        CANCEL.set(Some(runs));
    }

    #[cfg(feature = "faults")]
    pub fn disarm() {
        CANCEL.set(None);
    }

    #[cfg(feature = "faults")]
    pub(super) fn ran() -> bool {
        RUNS.set(RUNS.get().saturating_add(1));
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
    pub(super) const fn ran() -> bool {
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

fn posed_shown(params: &str) -> Vec<(&'static str, Json)> {
    vec![
        (
            "world",
            whole(
                "a world of a law: the start is its cells at its frame; give this or law",
                1,
                i64::from(u32::MAX),
            ),
        ),
        (
            "law",
            property(
                "string",
                "the JSON text of a law, with size and cells; give this or world",
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
            words_shown(params, most(crate::law::limits::PARAMS as u64)),
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

fn score_shown() -> Json {
    tool(
        "grit_law_score",
        "Score a law's params: run it forward from a start on the CPU and answer |cells - target| summed over a box after n frames. Any law.",
        posed_shown("the law's params; the world's if left out"),
        &["frames", "aim"],
    )
}

fn search_shown() -> Json {
    let mut properties =
        posed_shown("the search's first parent, inside bounds; the world's if left out");
    properties.extend([
        (
            "bounds",
            map(vec![
                ("type", text("array")),
                (
                    "items",
                    map(vec![
                        ("type", text("array")),
                        ("items", word()),
                        (
                            "description",
                            text("[least, greatest] of one param, both included"),
                        ),
                        ("minItems", Json::Int(2)),
                        ("maxItems", Json::Int(2)),
                    ]),
                ),
                (
                    "description",
                    text("one [least, greatest] for each param of the law"),
                ),
                (
                    "maxItems",
                    Json::Int(most(crate::law::limits::PARAMS as u64)),
                ),
            ]),
        ),
        (
            "seed",
            whole(
                "the seed of the search's random draws",
                0,
                i64::from(u32::MAX),
            ),
        ),
        (
            "population",
            whole(
                "children run in each generation",
                1,
                i64::from(POPULATION_MOST),
            ),
        ),
        (
            "generations",
            whole(
                "the most generations after the first parent",
                1,
                i64::from(GENERATIONS_MOST),
            ),
        ),
    ]);
    tool(
        "grit_learn_params",
        "Search a law's params inside bounds toward the aim by a seeded evolution strategy. Any law. Returns the best params, their objective, the trail and why it stopped.",
        properties,
        &[
            "frames",
            "aim",
            "bounds",
            "seed",
            "population",
            "generations",
        ],
    )
}

struct Posed {
    law: Law,
    grid: Grid3,
    start: Vec<u32>,
    first: u32,
    frames: u32,
    params: Vec<u32>,
    region: Region,
    target: Vec<u32>,
    units: u64,
    bounds: Vec<(u32, u32)>,
}

impl Posed {
    const fn aim(&self) -> Aim<'_> {
        Aim {
            region: self.region,
            target: self.target.as_slice(),
        }
    }
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
                    Code::Start,
                    "$.world",
                    format!(
                        "world {number} runs the rule {}, not a law; give a law inline with size and cells. Nothing ran",
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
            let law = Law::compile(words_of(held, "law")?)
                .map_err(|why| Box::new(super::law::Refused(why, "$.law")) as Failed)?;
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
            let planes = Declared::planes(&law);
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

fn units_of(law: &Law, grid: Grid3, frames: u32) -> u64 {
    let nodes = u64::try_from(law.nodes())
        .unwrap_or(u64::MAX)
        .saturating_add(OCT_UNITS);
    u64::from(grid.octs())
        .saturating_mul(u64::from(frames))
        .saturating_mul(nodes)
}

fn bounds_of(held: &Json, params: &[u32]) -> Result<Vec<(u32, u32)>, Failed> {
    let listed = held
        .get("bounds")
        .and_then(Json::list)
        .ok_or_else(|| unread("bounds"))?;
    if listed.len() != params.len() {
        return Err(limited(
            Code::Params,
            "$.bounds",
            format!(
                "the law reads {} params, and {} bounds were given",
                params.len(),
                listed.len()
            ),
            counted(u32::try_from(params.len()).unwrap_or(u32::MAX)),
        ));
    }
    let mut bounds = Vec::with_capacity(listed.len());
    for (at, one) in listed.iter().enumerate() {
        let pair = ints(one, "bounds")?;
        let [least, greatest] =
            <[u32; 2]>::try_from(pair.as_slice()).map_err(|_| unread("bounds"))?;
        if least > greatest {
            return Err(refused(
                Code::Reversed,
                &format!("$.bounds[{at}]"),
                format!(
                    "the bounds [{least}, {greatest}] of param {at} have their least past their greatest"
                ),
            ));
        }
        bounds.push((least, greatest));
    }
    for (at, (&param, &(least, greatest))) in params.iter().zip(&bounds).enumerate() {
        if param < least || param > greatest {
            return Err(limited(
                Code::Bounds,
                &format!("$.params[{at}]"),
                format!("param {at} is {param}, outside its bounds [{least}, {greatest}]"),
                Json::List(vec![counted(least), counted(greatest)]),
            ));
        }
    }
    if bounds.iter().all(|(least, greatest)| least == greatest) {
        return Err(refused(
            Code::Bounds,
            "$.bounds",
            "every param's bounds hold one value, so no child can differ from its parent; score the params with grit_law_score".to_owned(),
        ));
    }
    Ok(bounds)
}

fn posed(server: &mut Server<'_>, held: &Json, search: bool) -> Result<Posed, Failed> {
    let start = started(server, held)?;
    let grid = start.grid;
    let size = [grid.wide, grid.deep, grid.tall];
    let cells = u64::try_from(grid.cells())
        .unwrap_or(u64::MAX)
        .saturating_mul(u64::from(start.planes));
    if cells > CELLS_MOST {
        return Err(limited(
            Code::RunLimit,
            start.field,
            format!(
                "the start holds {cells} words ({} cells on {} planes), more than the {CELLS_MOST} a run on the CPU takes",
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
    let wanted = Declared::params(&start.law);
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
    let bounds = if search {
        bounds_of(held, &params)?
    } else {
        Vec::new()
    };
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
    let units = units_of(&start.law, grid, frames);
    if units > WORK_MOST {
        let each = units / u64::from(frames.max(1));
        let fits = u32::try_from(WORK_MOST / each.max(1)).unwrap_or(u32::MAX);
        return Err(limited(
            Code::RunLimit,
            if fits == 0 { start.field } else { "$.frames" },
            format!(
                "a run of {frames} frames over {} octs of a law of {} nodes is {units} units of work (octs times frames times the nodes plus {OCT_UNITS}), more than the {WORK_MOST} one run may take; at most {fits} frames fit",
                grid.octs(),
                start.law.nodes()
            ),
            counted(fits),
        ));
    }
    server.fits(Work::Run, units, "one run of the law")?;
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
        law: start.law,
        grid,
        start: start_cells,
        first: start.first,
        frames,
        params,
        region,
        target,
        units,
        bounds,
    })
}

#[cfg(feature = "faults")]
fn wedging(server: &Server<'_>) {
    let due = server
        .book
        .faults
        .lock()
        .map(|mut faults| match faults.wedge_after_runs {
            Some(0 | 1) => {
                faults.wedge_after_runs = None;
                true
            }
            Some(left) => {
                faults.wedge_after_runs = Some(left - 1);
                false
            }
            None => false,
        })
        .unwrap_or(false);
    if due && let Some(host) = server.host {
        host.wedge();
    }
}

#[cfg(not(feature = "faults"))]
const fn wedging(_: &Server<'_>) {}

fn objective_of(posed: &Posed, params: &[u32]) -> u64 {
    let mut cells = posed.start.clone();
    for frame in (0..posed.frames).map(|done| posed.first.wrapping_add(done)) {
        cells = reference(&posed.law, posed.grid, frame, &cells, params);
    }
    posed.aim().measured(posed.grid, &cells)
}

fn run(server: &mut Server<'_>, posed: &Posed, params: &[u32]) -> u64 {
    let began = server.now();
    super::tools::faults::stall(server.time(), 1);
    let objective = objective_of(posed, params);
    server
        .book
        .rates
        .measure(Work::Run, posed.units, server.since(began));
    if faults::ran()
        && let Some(flag) = server.cancel.as_ref()
    {
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    wedging(server);
    objective
}

fn listed(held: &[u32]) -> Json {
    Json::List(held.iter().map(|one| counted(*one)).collect())
}

fn scored(server: &mut Server<'_>, held: &Json) -> Result<Json, Failed> {
    let posed = posed(server, held, false)?;
    let objective = run(server, &posed, &posed.params);
    let mut answer = Vec::new();
    if let Some(number) = held.get("world").and_then(Json::int) {
        answer.push(("world", Json::Int(number)));
    }
    answer.extend([
        ("params", listed(&posed.params)),
        ("first", counted(posed.first)),
        ("frames", counted(posed.frames)),
        ("objective", exactly(u128::from(objective))),
    ]);
    Ok(map(answer))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Why {
    Found,
    Generations,
    Stopped(Stop),
}

impl Why {
    const fn name(self) -> &'static str {
        match self {
            Self::Found => "found",
            Self::Generations => "generations",
            Self::Stopped(stop) => stop.name(),
        }
    }
}

fn halted(server: &Server<'_>, units: u64) -> Option<Stop> {
    if let Some(stop) = server.stop() {
        return Some(stop);
    }
    let (least, _) = server.book.rates.least_of(Work::Run, units);
    (least > server.clock.left()).then_some(Stop::Budget)
}

struct Draws(u64);

impl Draws {
    const fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut mixed = self.0;
        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        mixed ^ (mixed >> 31)
    }

    fn below(&mut self, most: u64) -> u64 {
        u64::try_from((u128::from(self.next()) * u128::from(most)) >> 64).unwrap_or(0)
    }
}

const fn width(bound: (u32, u32)) -> u64 {
    (bound.1 - bound.0) as u64
}

fn child_of(parent: &[u32], spread: &[u64], bounds: &[(u32, u32)], draws: &mut Draws) -> Vec<u32> {
    let mut child: Vec<u32> = parent
        .iter()
        .zip(spread)
        .zip(bounds)
        .map(|((&param, &spread), &(least, greatest))| {
            if least == greatest {
                return param;
            }
            let span = spread.saturating_mul(2).saturating_add(1);
            let drawn: u64 = (0..3).map(|_| draws.below(span)).sum();
            let moved = i128::from(param) + i128::from(drawn) - 3 * i128::from(spread);
            u32::try_from(moved.clamp(i128::from(least), i128::from(greatest))).unwrap_or(least)
        })
        .collect();
    if child != parent {
        return child;
    }
    let free: Vec<usize> = bounds
        .iter()
        .enumerate()
        .filter(|(_, bound)| bound.0 < bound.1)
        .map(|(at, _)| at)
        .collect();
    let at = free
        .get(usize::try_from(draws.below(free.len() as u64)).unwrap_or(0))
        .copied()
        .unwrap_or(0);
    let up = draws.below(2) == 0;
    if let (Some(slot), Some(&(least, greatest))) = (child.get_mut(at), bounds.get(at)) {
        *slot = if (up && *slot < greatest) || *slot == least {
            slot.saturating_add(1)
        } else {
            slot.saturating_sub(1)
        };
    }
    child
}

fn opened(bound: (u32, u32)) -> u64 {
    (width(bound) / 4).max(1)
}

fn restarted(bounds: &[(u32, u32)], draws: &mut Draws) -> Vec<u32> {
    bounds
        .iter()
        .map(|&bound| {
            let moved = u32::try_from(draws.below(width(bound) + 1)).unwrap_or(0);
            bound.0.saturating_add(moved)
        })
        .collect()
}

fn adapted(spread: &mut [u64], bounds: &[(u32, u32)], kept: u32, population: u32) {
    let rate = u64::from(kept) * 5;
    let whole = u64::from(population);
    for (one, &bound) in spread.iter_mut().zip(bounds) {
        let most = width(bound).max(1);
        *one = if rate > whole {
            one.saturating_add(one.div_ceil(2)).min(most)
        } else if rate < whole {
            (*one * 2 / 3).max(1)
        } else {
            *one
        };
    }
}

type Scored = (Vec<u32>, u64);

struct Searched {
    best: Scored,
    trail: Vec<Scored>,
    scored: u64,
    why: Why,
}

fn lower(held: &mut Option<Scored>, params: &[u32], objective: u64) {
    if held.as_ref().is_none_or(|(_, least)| objective < *least) {
        *held = Some((params.to_vec(), objective));
    }
}

fn search(
    server: &mut Server<'_>,
    posed: &Posed,
    seed: u32,
    population: u32,
    generations: u32,
) -> Result<Searched, Failed> {
    if let Some(stop) = server.stop() {
        return Err(refused(
            stop.code(),
            if stop == Stop::Budget {
                "$.budget_ms"
            } else {
                "$"
            },
            format!(
                "the search stopped on {} before its first run; nothing ran",
                stop.name()
            ),
        ));
    }
    let mut draws = Draws(u64::from(seed));
    let mut parent = posed.params.clone();
    let mut held = run(server, posed, &parent);
    let mut best = Some((parent.clone(), held));
    let mut trail = vec![(parent.clone(), held)];
    let mut scored = 1_u64;
    let mut spread: Vec<u64> = posed.bounds.iter().map(|&bound| opened(bound)).collect();
    let why = 'generations: {
        if held == 0 {
            break 'generations Why::Found;
        }
        let mut stale = 0_u32;
        for _ in 0..generations {
            let mut lowest: Option<Scored> = None;
            let mut kept = 0_u32;
            for _ in 0..population {
                if let Some(stop) = halted(server, posed.units) {
                    break 'generations Why::Stopped(stop);
                }
                let child = child_of(&parent, &spread, &posed.bounds, &mut draws);
                let objective = run(server, posed, &child);
                scored += 1;
                lower(&mut best, &child, objective);
                lower(&mut lowest, &child, objective);
                if objective <= held {
                    kept += 1;
                }
                if objective == 0 {
                    trail.extend(lowest);
                    break 'generations Why::Found;
                }
            }
            let Some((params, objective)) = lowest else {
                break 'generations Why::Generations;
            };
            trail.push((params.clone(), objective));
            if objective < held {
                stale = 0;
            } else {
                stale += 1;
            }
            if objective <= held {
                parent = params;
                held = objective;
            }
            adapted(&mut spread, &posed.bounds, kept, population);
            if stale >= STALE && spread.iter().all(|&one| one == 1) {
                stale = 0;
                for (one, &bound) in spread.iter_mut().zip(&posed.bounds) {
                    *one = opened(bound);
                }
                parent = restarted(&posed.bounds, &mut draws);
                held = u64::MAX;
            }
        }
        Why::Generations
    };
    Ok(Searched {
        best: best.ok_or("the search kept no best")?,
        trail,
        scored,
        why,
    })
}

fn searched(server: &mut Server<'_>, held: &Json) -> Result<Json, Failed> {
    let seed = int(held, "seed")?;
    let population = int(held, "population")?;
    let generations = int(held, "generations")?;
    let posed = posed(server, held, true)?;
    let Searched {
        best,
        trail,
        scored,
        why,
    } = search(server, &posed, seed, population, generations)?;
    let mut answer = Vec::new();
    if let Some(number) = held.get("world").and_then(Json::int) {
        answer.push(("world", Json::Int(number)));
    }
    answer.extend([
        ("params", listed(&best.0)),
        ("objective", exactly(u128::from(best.1))),
        ("stopped", text(why.name())),
        (
            "generations",
            counted(u32::try_from(trail.len().saturating_sub(1)).unwrap_or(u32::MAX)),
        ),
        ("scored", exactly(u128::from(scored))),
        (
            "trail",
            Json::List(
                trail
                    .iter()
                    .map(|(params, objective)| {
                        map(vec![
                            ("params", listed(params)),
                            ("objective", exactly(u128::from(*objective))),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("seed", counted(seed)),
        ("first", counted(posed.first)),
        ("frames", counted(posed.frames)),
    ]);
    if let Why::Stopped(stop) = why {
        answer.push(("hint", text(stop.code().hint())));
    }
    Ok(map(answer))
}
