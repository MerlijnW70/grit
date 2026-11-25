use crate::border::Absorb;
use crate::branch::World;
use crate::host::Host;
use crate::law::Law;
use crate::wire::{Json, counted, map, text};

use super::errors::{Code, Refusal};
use super::journal::Made;
use super::schema::{choice, object, property, whole, with, word};
use super::tools::{
    absorb_of, absorb_shown, admitted, build_cost, chosen, frames_cost, int, ints, limited,
    refused, sized, tool, world_frame_cells, write_cost,
};
use super::{Failed, Listed, Server, Tool, shaped};

pub const TOOLS: &[Tool] = &[Tool {
    name: "grit_law_check",
    shown: check_shown,
    run: checked,
}];

pub const STARTS_MOST: usize = 256;

pub const CELLS_MOST: u64 = 1 << 22;

pub const STEPS_MOST: u32 = 4096;

pub const WORK_MOST: u64 = 1 << 32;

pub const TARGETS_MOST: usize = 1024;

pub const PAGE_CELLS: usize = 16384;

pub const SHARED_MOST: u64 = 1 << 22;

pub const SPREAD_MOST: u64 = 8;

pub const HOST_MOST: u64 = 1 << 30;

pub const PIECE_CELLS: usize = 1 << 15;

pub const WORD_BYTES: u64 = 4;

pub const ENTRY_BYTES: u64 = 12;

pub const POINT_BYTES: u64 = 512;

pub const WRITTEN_BYTES: u64 = 48;

pub const SHOWN_BYTES: u64 = 1024;

pub const ENGINE_FIXED: u64 = 256 << 20;

pub const ENGINE_LARGEST: u64 = 16;

pub const ENGINE_KEPT: u64 = 8;

pub const CHECK: &str = "grit_law_check runs a law or rule on many starts for frames frames on the device and scores each start against its targets, in one call. A start is a box of the world's shape (size, as grit_world_create answers it), written to the plane reads, or the current plane at frame 0; its end state and its targets are on the plane writes, or the current plane at each target's frame. Under border wrapped a start is a torus of its own size; under border zero every cell outside its box reads 0 at every frame, as a world twice its size on every axis above 2 with that half absorbed. Each start equals, bit for bit, a world of its own made by grit_world_create with the same law, params, size, sparse, alternate and absorb, written, stepped and read. Starts share one dense world when the border wraps, no absorb is given and the law reads no at: each start is a tile with a margin of its own torus copies, (1 + halo) times frames cells wide on each side of one axis, so no tile reaches another within frames frames; every other start runs in a world of its own. Scores per target: cells, right, wrong and the first wrong cell (x fastest, then y, then z); under values real also the max and rms error in words and decoded with frac. Priced as the worlds it builds (bytes), the cells it writes, margins included, and the frames it steps (world cells times frames, held bricks on sparse worlds); its reads are not priced, as no read is. It stops only on budget or cancel before a build or a step, and then answers nothing";

pub const FINGERPRINT: &str = "sha256 of the start's end state: the box's words on every plane, plane 0 first, x fastest, then y, then z, each word 4 bytes little-endian";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Current,
    Plane(u32),
}

struct Ask {
    rule: Box<dyn Listed>,
    params: Vec<u32>,
    size: [u32; 3],
    sparse: bool,
    absorb: Option<Absorb>,
    zero: bool,
    steps: u32,
    reads: Side,
    writes: Side,
    frac: Option<u32>,
    planes: u32,
}

impl Ask {
    fn law(&self) -> Option<&Law> {
        self.rule.lawful().map(super::law::Lawful::law)
    }

    fn plane(&self, side: Side, step: u32) -> u32 {
        match side {
            Side::Plane(plane) => plane,
            Side::Current => self
                .law()
                .and_then(|law| law.current(step))
                .unwrap_or_default(),
        }
    }
}

enum Boxed {
    Dense(Vec<u32>),
    Cells(Vec<(u32, u32)>),
}

impl Boxed {
    fn into_dense(self, count: usize) -> Vec<u32> {
        match self {
            Self::Dense(words) => words,
            Self::Cells(cells) => {
                let mut words = vec![0_u32; count];
                for (at, value) in cells {
                    if let Some(slot) = words.get_mut(at as usize) {
                        *slot = value;
                    }
                }
                words
            }
        }
    }

    fn word(&self, at: usize, next: &mut usize) -> Option<u32> {
        match self {
            Self::Dense(words) => words.get(at).copied(),
            Self::Cells(cells) => match cells.get(*next) {
                Some(&(held, value)) if held as usize == at => {
                    *next += 1;
                    Some(value)
                }
                _ => Some(0),
            },
        }
    }
}

struct Goal {
    when: u32,
    words: Boxed,
}

struct Start {
    words: Vec<u32>,
    goals: Vec<Goal>,
}

#[derive(Clone, Copy, Default)]
struct Score {
    when: u32,
    count: u64,
    wrong: u64,
    earliest: Option<(usize, u32, u32)>,
    squares: u128,
    worst: u64,
}

impl Score {
    fn joined(self, other: Self) -> Self {
        Self {
            when: 0,
            count: self.count.saturating_add(other.count),
            wrong: self.wrong.saturating_add(other.wrong),
            earliest: None,
            squares: self.squares.saturating_add(other.squares),
            worst: self.worst.max(other.worst),
        }
    }
}

#[derive(Default)]
struct Ended {
    fingerprint: String,
    scored: Vec<u32>,
    scores: Vec<(usize, Score)>,
}

struct Tiles {
    axis: usize,
    margin: u32,
    span: u32,
    per: usize,
}

struct Placed {
    start: usize,
    origin: [u32; 3],
}

type Group = ([u32; 3], Option<Absorb>, Vec<Placed>);

fn along(held: [u32; 3], axis: usize) -> u32 {
    held.get(axis).copied().unwrap_or_default()
}

fn put(held: &mut [u32; 3], axis: usize, value: u32) {
    if let Some(slot) = held.get_mut(axis) {
        *slot = value;
    }
}

fn plus(one: [u32; 3], other: [u32; 3]) -> [u32; 3] {
    let [a, b, c] = one;
    let [x, y, z] = other;
    [a + x, b + y, c + z]
}

fn count_of(size: [u32; 3]) -> u64 {
    size.iter().map(|side| u64::from(*side)).product()
}

fn index_of(size: [u32; 3], at: [u32; 3]) -> usize {
    let wide = size[0] as usize;
    let deep = size[1] as usize;
    (at[2] as usize * deep + at[1] as usize) * wide + at[0] as usize
}

fn point_of(size: [u32; 3], at: usize) -> [u32; 3] {
    let wide = (size[0] as usize).max(1);
    let deep = (size[1] as usize).max(1);
    [
        u32::try_from(at % wide).unwrap_or_default(),
        u32::try_from(at / wide % deep).unwrap_or_default(),
        u32::try_from(at / (wide * deep)).unwrap_or_default(),
    ]
}

fn word_shown(what: &str) -> Json {
    map(vec![
        ("type", text("array")),
        ("items", word()),
        ("description", text(what)),
    ])
}

fn triple(what: &str) -> Json {
    map(vec![
        ("type", text("array")),
        ("items", word()),
        ("description", text(what)),
        ("minItems", Json::Int(3)),
        ("maxItems", Json::Int(3)),
    ])
}

fn cells_shown(what: &str) -> Json {
    map(vec![
        ("type", text("array")),
        (
            "items",
            map(vec![
                ("type", text("array")),
                ("items", word()),
                ("minItems", Json::Int(4)),
                ("maxItems", Json::Int(4)),
            ]),
        ),
        ("description", text(what)),
    ])
}

fn box_shown(what: &str, more: Vec<(&'static str, Json)>) -> Json {
    let mut properties = vec![
        (
            "cells",
            cells_shown(
                "each [x, y, z, word] in the box; every other cell is 0; the last of a cell wins",
            ),
        ),
        (
            "dense",
            word_shown("every word of the box, x fastest, then y, then z"),
        ),
        (
            "world",
            whole(
                "take the box from this open world, from its corner from",
                1,
                i64::from(u32::MAX),
            ),
        ),
        (
            "from",
            triple("with world: the box's least corner in that world"),
        ),
        (
            "plane",
            whole(
                "with world: its plane to take; its current plane if it iterates, else 0, if left out",
                0,
                i64::from(crate::law::limits::CHANNELS - 1),
            ),
        ),
    ];
    properties.extend(more);
    with(object(properties, &[]), vec![("description", text(what))])
}

fn check_shown() -> Json {
    tool(
        "grit_law_check",
        "Run a law on many starts for frames frames on the device and score each start against its targets, in one call; the starts field says how they run.",
        vec![
            (
                "rule",
                property("string", "a rule grit_world_rules lists; give this or law"),
            ),
            (
                "law",
                property("string", "the JSON text of a law; give this or rule"),
            ),
            (
                "alternate",
                map(vec![
                    ("type", text("array")),
                    ("items", word()),
                    (
                        "description",
                        text("with law: [a, b], as grit_world_create takes it"),
                    ),
                    ("minItems", Json::Int(2)),
                    ("maxItems", Json::Int(2)),
                ]),
            ),
            (
                "params",
                word_shown("the law's or rule's params; none if left out"),
            ),
            (
                "size",
                triple(
                    "every start's [wide, deep, tall]: a world's shape as grit_world_create answers it",
                ),
            ),
            (
                "sparse",
                property("boolean", "run each start in a sparse world, one a start"),
            ),
            (
                "absorb",
                triple(
                    "[x, y, z] cells of each start's box zeroed after every frame, as grit_world_create takes it",
                ),
            ),
            (
                "border",
                choice(
                    "wrapped: a start is a torus of its own size; zero: every cell outside its box reads 0 at every frame. wrapped if left out",
                    &["wrapped", "zero"],
                ),
            ),
            (
                "frames",
                whole("frames every start runs", 1, i64::from(u32::MAX)),
            ),
            (
                "reads",
                whole(
                    "the plane a start is written to; 0 if left out, or the current plane when the law iterates",
                    0,
                    i64::from(u32::MAX),
                ),
            ),
            (
                "writes",
                whole(
                    "the plane scored and answered; 0 if left out, or the current plane when the law iterates",
                    0,
                    i64::from(u32::MAX),
                ),
            ),
            (
                "current",
                property(
                    "boolean",
                    "true: starts are written to the current plane at frame 0 and scored on the current plane; the default for a law that iterates",
                ),
            ),
            (
                "values",
                choice(
                    "binary: score words equal or not; real: also the error of signed fixed-point words with frac fractional bits. binary if left out",
                    &["binary", "real"],
                ),
            ),
            (
                "frac",
                whole(
                    "with values real: the fractional bits of every word; 16 if left out",
                    0,
                    i64::from(super::law_real::FRAC_MOST),
                ),
            ),
            (
                "starts",
                map(vec![
                    ("type", text("array")),
                    (
                        "items",
                        box_shown(
                            "a start: exactly one of cells, dense and world, and its targets",
                            vec![(
                                "targets",
                                map(vec![
                                    ("type", text("array")),
                                    (
                                        "items",
                                        box_shown(
                                            "a state the start should reach: exactly one of cells, dense and world, at frame",
                                            vec![(
                                                "frame",
                                                whole(
                                                    "the frame it is compared at, 0 to frames; frames if left out",
                                                    0,
                                                    i64::from(u32::MAX),
                                                ),
                                            )],
                                        ),
                                    ),
                                    ("description", text("the start's targets; none if left out")),
                                ]),
                            )],
                        ),
                    ),
                    ("description", text(CHECK)),
                ]),
            ),
            (
                "return",
                property(
                    "boolean",
                    "true: also answer the end states on the scored plane, as [start, x, y, z, word] of each cell not 0, a page at a time",
                ),
            ),
            (
                "page",
                whole(
                    "with return: the page of end-state cells to answer; 0 if left out",
                    0,
                    i64::from(u32::MAX),
                ),
            ),
        ],
        &["size", "frames", "starts"],
    )
}

fn checked(server: &mut Server<'_>, held: &Json) -> Result<Json, Failed> {
    let host = server.host.ok_or(crate::host::Error::NoComputeDevice)?;
    let ask = asked(server, host, held)?;
    let listed_starts = starts_of(&ask, held)?;
    let give = matches!(held.get("return"), Some(Json::Bool(true)));
    let tiles = tiled(host, &ask, listed_starts.len());
    let groups = grouped(&ask, tiles.as_ref(), listed_starts.len());
    bounded(&ask, listed_starts, (&groups, tiles.is_some()), give)?;
    let starts = started(server, &ask, listed_starts)?;
    let page = match held.get("page") {
        Some(_) => usize::try_from(int(held, "page")?).unwrap_or(usize::MAX),
        None => 0,
    };
    let mut ended: Vec<Ended> = (0..starts.len()).map(|_| Ended::default()).collect();
    let all = groups.len();
    for (done, (shape, absorb, placed)) in groups.iter().enumerate() {
        if let Some(stop) = server.stop() {
            return Err(Box::new(Refusal::new(
                stop.code(),
                "$",
                format!(
                    "the check stopped before world {} of {all}, so nothing is answered",
                    done + 1
                ),
            )));
        }
        ran(
            server,
            host,
            (&ask, &starts, give),
            (*shape, *absorb, placed, tiles.as_ref()),
            &mut ended,
        )?;
    }
    answered(&ask, &starts, &ended, (tiles.as_ref(), all), (give, page))
}

fn side_of(held: &Json, key: &str) -> Result<Option<u32>, Failed> {
    match held.get(key) {
        None => Ok(None),
        Some(_) => int(held, key).map(Some),
    }
}

fn asked(server: &Server<'_>, host: &Host, held: &Json) -> Result<Ask, Failed> {
    let rule = chosen(server, held)?;
    let params = match held.get("params") {
        Some(given) => ints(given, "params")?,
        None => Vec::new(),
    };
    if params.len() != rule.params() {
        return Err(limited(
            Code::Params,
            "$.params",
            format!(
                "{} reads {} params, and {} were given",
                rule.name(),
                rule.params(),
                params.len()
            ),
            counted(u32::try_from(rule.params()).unwrap_or(u32::MAX)),
        ));
    }
    let sparse = matches!(held.get("sparse"), Some(Json::Bool(true)));
    let rule: Box<dyn Listed> = match rule.lawful() {
        None => rule,
        Some(lawful) => {
            let law = lawful.law().clone();
            let class = super::law::background_class(&law, &params);
            if sparse && !class.proved() {
                return Err(super::law::quiet_refused(&law, &class, "$.sparse"));
            }
            Box::new(super::law::Lawful::new(law, class))
        }
    };
    let given = ints(held.get("size").ok_or("size")?, "size")?;
    let size = <[u32; 3]>::try_from(given.as_slice()).map_err(|_| "size")?;
    let shape = Made {
        rule: rule.name().to_owned(),
        params: params.clone(),
        size,
        sparse,
        slots: None,
    };
    let (grid, _) = shaped(host, rule.as_ref(), &shape)?;
    let world = [grid.wide, grid.deep, grid.tall];
    if world != size {
        return Err(limited(
            Code::CheckStart,
            "$.size",
            format!(
                "a start of {size:?} cells is not a world's shape: grit_world_create makes it {world:?}"
            ),
            sized(world),
        ));
    }
    let absorb = absorb_of(held, size)?;
    let zero = held.get("border").and_then(Json::text) == Some("zero");
    let steps = int(held, "frames")?;
    if steps > STEPS_MOST {
        return Err(limited(
            Code::Frames,
            "$.frames",
            format!("{steps} frames is past the {STEPS_MOST} one check runs"),
            counted(STEPS_MOST),
        ));
    }
    let planes = rule
        .lawful()
        .map_or(1, |lawful| lawful.law().channels().max(1));
    let iterates = rule
        .lawful()
        .is_some_and(|lawful| lawful.law().alternate().is_some());
    let reads = side_of(held, "reads")?;
    let writes = side_of(held, "writes")?;
    let current = match held.get("current") {
        Some(Json::Bool(asked)) => *asked,
        _ => iterates && reads.is_none() && writes.is_none(),
    };
    let (reads, writes) = if current {
        if !iterates {
            return Err(refused(
                Code::NotIterating,
                "$.current",
                "the law was not given alternate, so it has no current plane".to_owned(),
            ));
        }
        if let Some(key) = [("reads", reads), ("writes", writes)]
            .iter()
            .find_map(|(key, given)| given.map(|_| *key))
        {
            return Err(refused(
                Code::PlaneOrCurrent,
                &format!("$.{key}"),
                format!("give {key} or current, not both"),
            ));
        }
        (Side::Current, Side::Current)
    } else {
        for (key, given) in [("reads", reads), ("writes", writes)] {
            if let Some(plane) = given.filter(|plane| *plane >= planes) {
                return Err(limited(
                    Code::Outside,
                    &format!("$.{key}"),
                    format!("plane {plane} is past the law's {planes} planes"),
                    counted(planes.saturating_sub(1)),
                ));
            }
        }
        (
            Side::Plane(reads.unwrap_or_default()),
            Side::Plane(writes.unwrap_or_default()),
        )
    };
    let numeric = held.get("values").and_then(Json::text) == Some("real");
    if held.get("page").is_some() && !matches!(held.get("return"), Some(Json::Bool(true))) {
        return Err(refused(
            Code::Malformed,
            "$.page",
            "page is read only with return true".to_owned(),
        ));
    }
    let frac = match (numeric, held.get("frac")) {
        (true, None) => Some(16),
        (true, Some(_)) => Some(int(held, "frac")?),
        (false, None) => None,
        (false, Some(_)) => {
            return Err(refused(
                Code::Malformed,
                "$.frac",
                "frac is read only with values real".to_owned(),
            ));
        }
    };
    Ok(Ask {
        rule,
        params,
        size,
        sparse,
        absorb,
        zero,
        steps,
        reads,
        writes,
        frac,
        planes,
    })
}

const FORMS: [&str; 3] = ["cells", "dense", "world"];

fn boxed(
    server: &mut Server<'_>,
    held: &Json,
    field: &str,
    size: [u32; 3],
    code: Code,
) -> Result<Boxed, Failed> {
    let given: Vec<&str> = FORMS
        .into_iter()
        .filter(|key| held.get(key).is_some())
        .collect();
    let count = usize::try_from(count_of(size))?;
    let largest = sized(size.map(|side| side.saturating_sub(1)));
    match given.as_slice() {
        ["cells"] => {
            let listed_cells = held.get("cells").and_then(Json::list).ok_or("cells")?;
            let mut cells = Vec::with_capacity(listed_cells.len());
            for (at, one) in listed_cells.iter().enumerate() {
                let got = ints(one, "cells")?;
                let [x, y, z, value] = <[u32; 4]>::try_from(got.as_slice()).map_err(|_| "cells")?;
                if x >= size[0] || y >= size[1] || z >= size[2] {
                    return Err(limited(
                        code,
                        &format!("{field}.cells[{at}]"),
                        format!(
                            "cell {at} at {:?} is outside the box of {size:?}",
                            [x, y, z]
                        ),
                        largest,
                    ));
                }
                cells.push((u32::try_from(index_of(size, [x, y, z]))?, value));
            }
            cells.reverse();
            cells.sort_by_key(|(at, _)| *at);
            cells.dedup_by_key(|(at, _)| *at);
            cells.retain(|(_, value)| *value != 0);
            Ok(Boxed::Cells(cells))
        }
        ["dense"] => {
            let words = ints(held.get("dense").ok_or("dense")?, "dense")?;
            if words.len() != count {
                return Err(limited(
                    code,
                    &format!("{field}.dense"),
                    format!(
                        "{} words were given, and the box of {size:?} holds {count}",
                        words.len()
                    ),
                    counted(u32::try_from(count).unwrap_or(u32::MAX)),
                ));
            }
            Ok(Boxed::Dense(words))
        }
        ["world"] => {
            let number = int(held, "world")?;
            let Some(from) = held.get("from") else {
                return Err(refused(
                    code,
                    &format!("{field}.from"),
                    "a box from a world needs from, its least corner".to_owned(),
                ));
            };
            let from = <[u32; 3]>::try_from(ints(from, "from")?.as_slice()).map_err(|_| "from")?;
            let plane = match held.get("plane") {
                Some(_) => Some(int(held, "plane")?),
                None => None,
            };
            let live = server.world_at(number, &format!("{field}.world"))?;
            let source = live.world.size();
            if (0..3).any(|axis| {
                u64::from(along(from, axis)) + u64::from(along(size, axis))
                    > u64::from(along(source, axis))
            }) {
                return Err(limited(
                    Code::Outside,
                    &format!("{field}.from"),
                    format!("a box of {size:?} from {from:?} passes world {number} of {source:?}"),
                    sized(
                        [0, 1, 2].map(|axis| along(source, axis).saturating_sub(along(size, axis))),
                    ),
                ));
            }
            let planes = live.world.planes();
            let plane = match plane {
                Some(plane) if plane >= planes => {
                    return Err(limited(
                        Code::Outside,
                        &format!("{field}.plane"),
                        format!("plane {plane} is past world {number}'s {planes} planes"),
                        counted(planes.saturating_sub(1)),
                    ));
                }
                Some(plane) => plane,
                None => live.world.current().unwrap_or_default(),
            };
            let mut words = Vec::with_capacity(count);
            let mut low = 0;
            while low < count {
                let high = low.saturating_add(PIECE_CELLS).min(count);
                let points: Vec<[u32; 3]> = (low..high)
                    .map(|at| plus(from, point_of(size, at)))
                    .collect();
                words.extend(live.world.cells_at_plane(&points, plane)?);
                low = high;
            }
            Ok(Boxed::Dense(words))
        }
        _ => Err(refused(
            code,
            field,
            "give exactly one of cells, dense and world".to_owned(),
        )),
    }
}

fn starts_of<'held>(ask: &Ask, held: &'held Json) -> Result<&'held [Json], Failed> {
    let listed_starts = held.get("starts").and_then(Json::list).ok_or("starts")?;
    if listed_starts.is_empty() {
        return Err(refused(
            Code::CheckStart,
            "$.starts",
            "give at least one start".to_owned(),
        ));
    }
    if listed_starts.len() > STARTS_MOST {
        return Err(limited(
            Code::TooMany,
            "$.starts",
            format!(
                "{} starts were given, past the {STARTS_MOST} one check runs; nothing ran",
                listed_starts.len()
            ),
            counted(u32::try_from(STARTS_MOST).unwrap_or(u32::MAX)),
        ));
    }
    let per = count_of(ask.size);
    let many = u64::try_from(listed_starts.len())?;
    let cells = many.saturating_mul(per);
    if cells > CELLS_MOST {
        return Err(limited(
            Code::TooMany,
            "$.starts",
            format!(
                "{many} starts of {per} cells are {cells} cells, past the {CELLS_MOST} one check holds; nothing ran"
            ),
            Json::Int(i64::try_from(CELLS_MOST / per.max(1))?),
        ));
    }
    let work = cells.saturating_mul(u64::from(ask.steps));
    if work > WORK_MOST {
        return Err(limited(
            Code::Frames,
            "$.frames",
            format!(
                "{cells} cells for {} frames are {work} cell frames, past the {WORK_MOST} one check runs; nothing ran",
                ask.steps
            ),
            Json::Int(i64::try_from(WORK_MOST / cells.max(1))?),
        ));
    }
    Ok(listed_starts)
}

fn form_bytes(held: &Json, box_bytes: u64) -> (u64, bool) {
    let given: Vec<&str> = FORMS
        .into_iter()
        .filter(|key| held.get(key).is_some())
        .collect();
    match given.as_slice() {
        ["cells"] => {
            let entries = held
                .get("cells")
                .and_then(Json::list)
                .map_or(0, <[Json]>::len);
            (
                u64::try_from(entries)
                    .unwrap_or(u64::MAX)
                    .saturating_mul(ENTRY_BYTES),
                false,
            )
        }
        ["dense"] => (box_bytes, false),
        ["world"] => (box_bytes, true),
        _ => (0, false),
    }
}

fn bounded(
    ask: &Ask,
    listed_starts: &[Json],
    (groups, tiling): (&[Group], bool),
    giving: bool,
) -> Result<(), Failed> {
    let per = count_of(ask.size);
    let box_bytes = per.saturating_mul(WORD_BYTES);
    let many = u64::try_from(listed_starts.len()).unwrap_or(u64::MAX);
    let starts = many.saturating_mul(box_bytes);
    let mut targets = 0_u64;
    let mut scores = 0_u64;
    let mut heaviest = (0_usize, 0_u64);
    let mut from_worlds = listed_starts.iter().any(|one| one.get("world").is_some());
    for (at, one) in listed_starts.iter().enumerate() {
        let mut own = 0_u64;
        for target in one.get("targets").and_then(Json::list).unwrap_or_default() {
            let (bytes, world) = form_bytes(target, box_bytes);
            own = own.saturating_add(bytes);
            scores = scores.saturating_add(1);
            from_worlds |= world;
        }
        targets = targets.saturating_add(own);
        if own > heaviest.1 {
            heaviest = (at, own);
        }
    }
    let ends = if giving { starts } else { 0 };
    let reads = if from_worlds {
        per.min(u64::try_from(PIECE_CELLS).unwrap_or(u64::MAX))
            .saturating_mul(POINT_BYTES)
    } else {
        0
    };
    let runs = groups
        .iter()
        .map(|(size, _, _)| {
            let cells = count_of(*size);
            let written = if tiling { cells } else { per };
            cells
                .saturating_mul(u64::from(ask.planes))
                .saturating_mul(WORD_BYTES)
                .saturating_add(box_bytes.saturating_mul(2))
                .saturating_add(written.saturating_mul(WRITTEN_BYTES))
        })
        .max()
        .unwrap_or_default();
    let planes_of = |size: &[u32; 3]| count_of(*size).saturating_mul(u64::from(ask.planes));
    let largest = groups
        .iter()
        .map(|(size, _, _)| planes_of(size))
        .max()
        .unwrap_or_default();
    let every = groups
        .iter()
        .map(|(size, _, _)| planes_of(size))
        .fold(0_u64, u64::saturating_add);
    let engine = ENGINE_FIXED
        .saturating_add(largest.saturating_mul(ENGINE_LARGEST))
        .saturating_add(every.saturating_sub(largest).saturating_mul(ENGINE_KEPT));
    let shown = many
        .saturating_add(scores)
        .saturating_mul(SHOWN_BYTES)
        .saturating_add(if giving {
            u64::try_from(PAGE_CELLS * 6 * std::mem::size_of::<Json>()).unwrap_or(u64::MAX)
        } else {
            0
        });
    let once = reads.max(runs).max(shown);
    let total = starts
        .saturating_add(targets)
        .saturating_add(ends)
        .saturating_add(engine)
        .saturating_add(once);
    if total > HOST_MOST {
        let field = if targets > starts.max(ends).max(engine).max(once) {
            format!("$.starts[{}].targets", heaviest.0)
        } else {
            "$.starts".to_owned()
        };
        return Err(limited(
            Code::TooMany,
            &field,
            format!(
                "the check would hold {total} bytes of host memory, past the {HOST_MOST} one check holds: starts {starts}, targets {targets}, end states {ends}, the worlds it builds {engine}, and at most {once} at once to read boxes of worlds, run a world or answer; nothing ran"
            ),
            Json::Int(i64::try_from(HOST_MOST)?),
        ));
    }
    Ok(())
}

fn started(
    server: &mut Server<'_>,
    ask: &Ask,
    listed_starts: &[Json],
) -> Result<Vec<Start>, Failed> {
    let mut targets = 0_usize;
    let mut starts = Vec::with_capacity(listed_starts.len());
    for (at, one) in listed_starts.iter().enumerate() {
        let field = format!("$.starts[{at}]");
        let words = boxed(server, one, &field, ask.size, Code::CheckStart)?
            .into_dense(usize::try_from(count_of(ask.size))?);
        let mut goals = Vec::new();
        if let Some(given) = one.get("targets").and_then(Json::list) {
            targets = targets.saturating_add(given.len());
            if targets > TARGETS_MOST {
                return Err(limited(
                    Code::TooMany,
                    &format!("{field}.targets"),
                    format!(
                        "the starts so far name {targets} targets, past the {TARGETS_MOST} one check scores; nothing ran"
                    ),
                    counted(u32::try_from(TARGETS_MOST).unwrap_or(u32::MAX)),
                ));
            }
            for (place, target) in given.iter().enumerate() {
                let path = format!("{field}.targets[{place}]");
                let when = match target.get("frame") {
                    Some(_) => int(target, "frame")?,
                    None => ask.steps,
                };
                if when > ask.steps {
                    return Err(limited(
                        Code::CheckTarget,
                        &format!("{path}.frame"),
                        format!(
                            "a target at frame {when} is past the {} frames the check runs",
                            ask.steps
                        ),
                        counted(ask.steps),
                    ));
                }
                let words = boxed(server, target, &path, ask.size, Code::CheckTarget)?;
                goals.push(Goal { when, words });
            }
        }
        starts.push(Start { words, goals });
    }
    Ok(starts)
}

fn reads_at(law: &Law) -> bool {
    crate::law::quiet::reads(law.dag()).at
}

fn tiled(host: &Host, ask: &Ask, many: usize) -> Option<Tiles> {
    let law = ask.law()?;
    if many < 2 || ask.zero || ask.absorb.is_some() || ask.sparse || reads_at(law) {
        return None;
    }
    let reach = u64::from(law.halo()).saturating_add(1);
    let margin = reach
        .saturating_mul(u64::from(ask.steps))
        .next_multiple_of(2);
    let alone = u64::try_from(many).ok()?.saturating_mul(count_of(ask.size));
    let mut best: Option<(u64, Tiles)> = None;
    for axis in 0..3 {
        let side = u64::from(along(ask.size, axis));
        let span = side.saturating_add(margin.saturating_mul(2));
        let others = count_of(ask.size) / side.max(1);
        let fits = |held: u64| {
            held.saturating_mul(span)
                .checked_next_power_of_two()
                .is_some_and(|whole| whole.saturating_mul(others) <= SHARED_MOST)
        };
        let mut per = 1_u64;
        while per < u64::try_from(many).ok()? && fits(per + 1) {
            per += 1;
        }
        if per < 2 {
            continue;
        }
        let worlds = u64::try_from(many).ok()?.div_ceil(per);
        let whole = per
            .saturating_mul(span)
            .checked_next_power_of_two()?
            .saturating_mul(others)
            .saturating_mul(worlds);
        if whole > alone.saturating_mul(SPREAD_MOST) {
            continue;
        }
        let mut size = ask.size;
        let length = per.saturating_mul(span).checked_next_power_of_two()?;
        put(&mut size, axis, u32::try_from(length).ok()?);
        let shape = Made {
            rule: ask.rule.name().to_owned(),
            params: ask.params.clone(),
            size,
            sparse: false,
            slots: None,
        };
        let fitted = shaped(host, ask.rule.as_ref(), &shape)
            .is_ok_and(|(grid, _)| [grid.wide, grid.deep, grid.tall] == size);
        if !fitted {
            continue;
        }
        if best.as_ref().is_none_or(|(held, _)| whole < *held) {
            best = Some((
                whole,
                Tiles {
                    axis,
                    margin: u32::try_from(margin).ok()?,
                    span: u32::try_from(span).ok()?,
                    per: usize::try_from(per).ok()?,
                },
            ));
        }
    }
    best.map(|(_, tiles)| tiles)
}

fn grouped(ask: &Ask, tiles: Option<&Tiles>, many: usize) -> Vec<Group> {
    if let Some(tiles) = tiles {
        let mut out = Vec::new();
        let mut at = 0;
        while at < many {
            let held = tiles.per.min(many - at);
            let mut size = ask.size;
            let length = u64::try_from(held)
                .unwrap_or(u64::MAX)
                .saturating_mul(u64::from(tiles.span))
                .checked_next_power_of_two()
                .unwrap_or(u64::MAX);
            put(
                &mut size,
                tiles.axis,
                u32::try_from(length).unwrap_or(u32::MAX),
            );
            let placed = (0..held)
                .map(|tile| {
                    let mut origin = [0_u32; 3];
                    put(
                        &mut origin,
                        tiles.axis,
                        u32::try_from(tile)
                            .unwrap_or(u32::MAX)
                            .saturating_mul(tiles.span)
                            .saturating_add(tiles.margin),
                    );
                    Placed {
                        start: at + tile,
                        origin,
                    }
                })
                .collect();
            out.push((size, None, placed));
            at += held;
        }
        return out;
    }
    let mut size = ask.size;
    let mut origin = [0_u32; 3];
    let mut shell = ask.absorb.map_or([0; 3], |absorb| absorb.cells());
    if ask.zero {
        for axis in 0..3 {
            let side = along(ask.size, axis);
            if side > 2 {
                put(&mut size, axis, side.saturating_mul(2));
                put(&mut origin, axis, side / 2);
                let grown = along(shell, axis).saturating_add(side / 2);
                put(&mut shell, axis, grown);
            }
        }
    }
    let absorb = Absorb::new(shell);
    (0..many)
        .map(|start| (size, absorb, vec![Placed { start, origin }]))
        .collect()
}

fn built<'host>(
    server: &mut Server<'host>,
    host: &'host Host,
    ask: &Ask,
    size: [u32; 3],
    absorb: Option<Absorb>,
) -> Result<World<'host>, Failed> {
    let shape = Made {
        rule: ask.rule.name().to_owned(),
        params: ask.params.clone(),
        size,
        sparse: ask.sparse,
        slots: None,
    };
    let (_, bindings) = shaped(host, ask.rule.as_ref(), &shape)?;
    let bytes = admitted(server, host, &bindings, "a world of the check")?;
    let world = match ask.rule.lawful() {
        None => World::create_absorbing(host, &shape.branch(), absorb)?,
        Some(lawful) => World::create_law_absorbing(
            host,
            lawful.law(),
            &shape.branch(),
            super::law::stepping_of(lawful.class(), ask.sparse),
            absorb,
        )?,
    };
    server.priced(build_cost(bytes));
    Ok(world)
}

fn placed_cells(
    ask: &Ask,
    starts: &[Start],
    size: [u32; 3],
    placed: &[Placed],
    tiles: Option<&Tiles>,
) -> Vec<([u32; 3], u32)> {
    let mut cells = Vec::new();
    for one in placed {
        let Some(start) = starts.get(one.start) else {
            continue;
        };
        match tiles {
            Some(tiles) => {
                let axis = tiles.axis;
                let side = along(ask.size, axis);
                let base = along(one.origin, axis).saturating_sub(tiles.margin);
                let reach = tiles.margin.div_ceil(side.max(1));
                for (at, value) in start.words.iter().enumerate() {
                    if *value == 0 {
                        continue;
                    }
                    let spot = point_of(ask.size, at);
                    let source = along(spot, axis);
                    for copy in 0..=reach.saturating_mul(2).saturating_add(1) {
                        let shifted = i64::from(source)
                            + i64::from(side) * (i64::from(copy) - i64::from(reach));
                        let inside = shifted + i64::from(tiles.margin);
                        if inside < 0 || inside >= i64::from(tiles.span) {
                            continue;
                        }
                        let mut place = spot;
                        put(
                            &mut place,
                            axis,
                            base + u32::try_from(inside).unwrap_or_default(),
                        );
                        if place.iter().zip(size).all(|(one, side)| *one < side) {
                            cells.push((place, *value));
                        }
                    }
                }
            }
            None => {
                for (at, value) in start.words.iter().enumerate() {
                    if *value == 0 {
                        continue;
                    }
                    cells.push((plus(one.origin, point_of(ask.size, at)), *value));
                }
            }
        }
    }
    cells
}

fn box_words(
    all: &[u32],
    size: [u32; 3],
    plane: u32,
    origin: [u32; 3],
    sides: [u32; 3],
) -> Vec<u32> {
    let whole = usize::try_from(count_of(size)).unwrap_or(usize::MAX);
    let base = whole.saturating_mul(plane as usize);
    let count = usize::try_from(count_of(sides)).unwrap_or_default();
    (0..count)
        .map(|at| {
            all.get(base + index_of(size, plus(origin, point_of(sides, at))))
                .copied()
                .unwrap_or_default()
        })
        .collect()
}

fn scored(when: u32, got: &[u32], want: &Boxed, decoding: bool) -> Score {
    let mut score = Score {
        when,
        count: u64::try_from(got.len()).unwrap_or(u64::MAX),
        ..Score::default()
    };
    let mut next = 0_usize;
    for (at, one) in got.iter().enumerate() {
        let Some(other) = want.word(at, &mut next) else {
            break;
        };
        if *one == other {
            continue;
        }
        score.wrong += 1;
        if score.earliest.is_none() {
            score.earliest = Some((at, other, *one));
        }
        if decoding {
            let gap = i128::from(one.cast_signed()) - i128::from(other.cast_signed());
            score.squares = score.squares.saturating_add(gap.unsigned_abs().pow(2));
            score.worst = score
                .worst
                .max(u64::try_from(gap.unsigned_abs()).unwrap_or(u64::MAX));
        }
    }
    score
}

fn fingerprint(
    all: &[u32],
    size: [u32; 3],
    planes: u32,
    origin: [u32; 3],
    sides: [u32; 3],
) -> String {
    let mut hash = crate::branch::Sha256::new();
    for plane in 0..planes {
        for word in box_words(all, size, plane, origin, sides) {
            hash.update(&word.to_le_bytes());
        }
    }
    crate::law::hex(&hash.finish())
}

fn ran<'host>(
    server: &mut Server<'host>,
    host: &'host Host,
    (ask, starts, giving): (&Ask, &[Start], bool),
    (size, absorb, placed, tiles): ([u32; 3], Option<Absorb>, &[Placed], Option<&Tiles>),
    ended: &mut [Ended],
) -> Result<(), Failed> {
    let mut world = built(server, host, ask, size, absorb)?;
    let cells = placed_cells(ask, starts, size, placed, tiles);
    if !cells.is_empty() {
        world.write_plane(ask.plane(ask.reads, 0), &cells)?;
        server.priced(write_cost(cells.len(), ask.sparse));
    }
    server.priced(frames_cost(world_frame_cells(&world), ask.steps));
    let mut stops: Vec<u32> = placed
        .iter()
        .filter_map(|one| starts.get(one.start))
        .flat_map(|start| start.goals.iter().map(|goal| goal.when))
        .chain([ask.steps])
        .collect();
    stops.sort_unstable();
    stops.dedup();
    let decoding = ask.frac.is_some();
    let mut now = 0_u32;
    for stop in stops {
        while now < stop {
            if let Some(halt) = server.stop() {
                return Err(Box::new(Refusal::new(
                    halt.code(),
                    "$",
                    format!(
                        "the check stopped at frame {now} of {}, so nothing is answered",
                        ask.steps
                    ),
                )));
            }
            let piece = (stop - now).min(super::budget::BATCH_MOST);
            world.step(piece)?;
            now += piece;
        }
        let all = world.cells()?;
        let plane = ask.plane(ask.writes, stop);
        for one in placed {
            let (Some(start), Some(end)) = (starts.get(one.start), ended.get_mut(one.start)) else {
                continue;
            };
            let got = box_words(&all, size, plane, one.origin, ask.size);
            for (place, goal) in start.goals.iter().enumerate() {
                if goal.when == stop {
                    end.scores
                        .push((place, scored(stop, &got, &goal.words, decoding)));
                }
            }
            if stop == ask.steps {
                end.fingerprint = fingerprint(&all, size, ask.planes, one.origin, ask.size);
                if giving {
                    end.scored = got;
                }
            }
        }
    }
    Ok(())
}

fn scientific(value: f64) -> Json {
    text(&format!("{value:.9e}"))
}

fn score_shown(
    score: &Score,
    size: [u32; 3],
    frac: Option<u32>,
    when: bool,
) -> Vec<(&'static str, Json)> {
    let mut shown = Vec::new();
    if when {
        shown.push(("frame", counted(score.when)));
    }
    shown.extend([
        ("cells", super::tools::exactly(u128::from(score.count))),
        (
            "right",
            super::tools::exactly(u128::from(score.count - score.wrong)),
        ),
        ("wrong", super::tools::exactly(u128::from(score.wrong))),
    ]);
    if let Some((at, want, got)) = score.earliest {
        shown.push((
            "first_wrong",
            map(vec![
                ("at", sized(point_of(size, at))),
                ("want", counted(want)),
                ("got", counted(got)),
            ]),
        ));
    }
    if let Some(frac) = frac {
        let unit = f64::from(1_u32 << frac);
        let rms = if score.count == 0 {
            0.0
        } else {
            (score.squares as f64 / score.count as f64).sqrt()
        };
        shown.extend([
            ("max", super::tools::exactly(u128::from(score.worst))),
            ("sum_squares", text(&score.squares.to_string())),
            ("rms", scientific(rms)),
            ("max_real", scientific(score.worst as f64 / unit)),
            ("rms_real", scientific(rms / unit)),
        ]);
    }
    shown
}

fn answered(
    ask: &Ask,
    starts: &[Start],
    ended: &[Ended],
    (tiles, worlds): (Option<&Tiles>, usize),
    (giving, page): (bool, usize),
) -> Result<Json, Failed> {
    let mut answer = Vec::new();
    match ask.law() {
        Some(law) => answer.push(("law", text(&law.hex()))),
        None => answer.push(("rule", text(ask.rule.name()))),
    }
    answer.extend([
        ("size", sized(ask.size)),
        ("frames", counted(ask.steps)),
        ("border", text(if ask.zero { "zero" } else { "wrapped" })),
    ]);
    if let Some(absorb) = ask.absorb {
        answer.push(("absorb", absorb_shown(absorb)));
    }
    match (ask.reads, ask.writes) {
        (Side::Plane(reads), Side::Plane(writes)) => {
            answer.push(("reads", counted(reads)));
            answer.push(("writes", counted(writes)));
        }
        _ => answer.push(("current", Json::Bool(true))),
    }
    match ask.frac {
        Some(frac) => {
            answer.push(("values", text("real")));
            answer.push(("frac", counted(frac)));
            answer.push(("format", text(super::law_real::FORMAT)));
        }
        None => answer.push(("values", text("binary"))),
    }
    let mut run = vec![("worlds", counted(u32::try_from(worlds).unwrap_or(u32::MAX)))];
    match tiles {
        Some(tiles) => run.extend([
            ("layout", text("tiled")),
            (
                "axis",
                counted(u32::try_from(tiles.axis).unwrap_or_default()),
            ),
            ("margin", counted(tiles.margin)),
            (
                "starts_per_world",
                counted(u32::try_from(tiles.per).unwrap_or(u32::MAX)),
            ),
        ]),
        None => run.push(("layout", text("apart"))),
    }
    answer.push(("run", map(run)));
    answer.push(("fingerprint", text(FINGERPRINT)));
    let mut total = Score::default();
    let mut targets = 0_u64;
    let mut shown_starts = Vec::with_capacity(starts.len());
    for (at, end) in ended.iter().enumerate() {
        let mut shown_scores: Vec<Json> = vec![Json::Null; end.scores.len()];
        for (place, score) in &end.scores {
            total = total.joined(*score);
            targets += 1;
            if let Some(slot) = shown_scores.get_mut(*place) {
                *slot = map(score_shown(score, ask.size, ask.frac, true));
            }
        }
        shown_starts.push(map(vec![
            ("start", counted(u32::try_from(at).unwrap_or(u32::MAX))),
            ("fingerprint", text(&end.fingerprint)),
            ("targets", Json::List(shown_scores)),
        ]));
    }
    answer.push(("starts", Json::List(shown_starts)));
    let mut totals = vec![
        (
            "starts",
            counted(u32::try_from(starts.len()).unwrap_or(u32::MAX)),
        ),
        ("targets", super::tools::exactly(u128::from(targets))),
    ];
    totals.extend(score_shown(&total, ask.size, ask.frac, false));
    answer.push(("totals", map(totals)));
    if giving {
        answer.push(("return", returned(ask.size, ended, page)?));
    }
    Ok(map(answer))
}

fn returned(size: [u32; 3], ended: &[Ended], page: usize) -> Result<Json, Failed> {
    let held: usize = ended
        .iter()
        .map(|end| end.scored.iter().filter(|word| **word != 0).count())
        .sum();
    let pages = held.div_ceil(PAGE_CELLS).max(1);
    if page >= pages {
        return Err(limited(
            Code::Outside,
            "$.page",
            format!("page {page} is past the {pages} pages of {held} end-state cells"),
            counted(u32::try_from(pages - 1).unwrap_or(u32::MAX)),
        ));
    }
    let skip = page * PAGE_CELLS;
    let mut cells = Vec::new();
    let mut seen = 0_usize;
    'pages: for (at, end) in ended.iter().enumerate() {
        for (place, word) in end.scored.iter().enumerate() {
            if *word == 0 {
                continue;
            }
            if seen >= skip {
                if cells.len() == PAGE_CELLS {
                    break 'pages;
                }
                let spot = point_of(size, place);
                cells.push(Json::List(vec![
                    counted(u32::try_from(at).unwrap_or(u32::MAX)),
                    counted(spot[0]),
                    counted(spot[1]),
                    counted(spot[2]),
                    counted(*word),
                ]));
            }
            seen += 1;
        }
    }
    Ok(map(vec![
        ("page", counted(u32::try_from(page).unwrap_or(u32::MAX))),
        ("pages", counted(u32::try_from(pages).unwrap_or(u32::MAX))),
        ("count", counted(u32::try_from(held).unwrap_or(u32::MAX))),
        ("cells", Json::List(cells)),
    ]))
}
