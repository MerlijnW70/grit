use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use crate::branch::format::{UNIFORM, brick_cells, lattice_of, place};
use crate::branch::{State, World};
use crate::host::Host;
use crate::wire::{Json, counted, map, text};

use super::budget::{
    self, BATCH_MOST, BUDGET_MOST_MS, BUDGET_MS, Pace, SUBMIT_MOST_MS, Stop, world_cost,
};
use super::errors::{self, Code, Refusal};
use super::journal::{self, Entry, Ledger, Made};
use super::schema::{choice, object, property, whole, with, word};
use super::transport::{ANSWER_MOST, DEPTH_MOST, ID_MOST, LINE_MOST, PROTOCOL};
use super::{
    DIGITS, Failed, Live, PAGE_BYTES, POOL_LEAST_CELLS, READ_MOST, SLICE_MOST, STEP_MOST, Server,
    Status, Tool, rules, rules_on, shaped,
};

pub const TOOLS: &[Tool] = &[
    Tool {
        name: "grit_world_rules",
        shown: rules_shown,
        run: listed_rules,
    },
    Tool {
        name: "grit_world_create",
        shown: create_shown,
        run: created,
    },
    Tool {
        name: "grit_world_write",
        shown: write_shown,
        run: written,
    },
    Tool {
        name: "grit_world_step",
        shown: step_shown,
        run: stepped,
    },
    Tool {
        name: "grit_world_read",
        shown: read_shown,
        run: read,
    },
    Tool {
        name: "grit_world_slice",
        shown: slice_shown,
        run: sliced,
    },
    Tool {
        name: "grit_world_close",
        shown: close_shown,
        run: closed,
    },
    Tool {
        name: "grit_describe",
        shown: describe_shown,
        run: described,
    },
];

#[cfg(feature = "faults")]
pub mod faults {
    use std::cell::Cell;

    use super::{Code, Failed, Refusal};

    thread_local! {
        static BATCH: Cell<Option<u32>> = const { Cell::new(None) };
    }

    pub fn fail_batch(nth: u32) {
        BATCH.set(Some(nth));
    }

    pub fn disarm() {
        BATCH.set(None);
    }

    pub(super) fn batch() -> Result<(), Failed> {
        match BATCH.get() {
            Some(0) => {
                BATCH.set(None);
                Err(Box::new(Refusal::new(
                    Code::Device,
                    "$",
                    "an armed fault failed this batch of frames before it was submitted".to_owned(),
                )))
            }
            Some(nth) => {
                BATCH.set(Some(nth - 1));
                Ok(())
            }
            None => Ok(()),
        }
    }
}

#[cfg(not(feature = "faults"))]
mod faults {
    use super::Failed;

    pub(super) const fn batch() -> Result<(), Failed> {
        Ok(())
    }
}

pub(super) fn refused(code: Code, field: &str, message: String) -> Failed {
    Box::new(Refusal::new(code, field, message))
}

pub(super) fn limited(code: Code, field: &str, message: String, limit: Json) -> Failed {
    Box::new(Refusal::new(code, field, message).limited(limit))
}

pub(super) fn unread(key: &str) -> Failed {
    refused(
        Code::Malformed,
        &format!("$.{key}"),
        format!("{key} did not reach the tool as its schema names it"),
    )
}

fn whole_of(one: &Json) -> Option<u32> {
    one.int().and_then(|one| u32::try_from(one).ok())
}

pub fn int(held: &Json, key: &str) -> Result<u32, Failed> {
    held.get(key).and_then(whole_of).ok_or_else(|| unread(key))
}

fn ints(one: &Json, key: &str) -> Result<Vec<u32>, Failed> {
    one.list()
        .ok_or_else(|| unread(key))?
        .iter()
        .map(|one| whole_of(one).ok_or_else(|| unread(key)))
        .collect()
}

fn corner(held: &Json, key: &str) -> Result<[u32; 3], Failed> {
    let got = ints(held.get(key).ok_or_else(|| unread(key))?, key)?;
    <[u32; 3]>::try_from(got.as_slice()).map_err(|_| unread(key))
}

pub(super) fn words_of<'held>(held: &'held Json, key: &str) -> Result<&'held str, Failed> {
    held.get(key)
        .and_then(Json::text)
        .ok_or_else(|| unread(key))
}

pub(super) fn sized(held: [u32; 3]) -> Json {
    Json::List(held.iter().map(|one| counted(*one)).collect())
}

fn largest(size: [u32; 3]) -> Json {
    sized(size.map(|side| side.saturating_sub(1)))
}

fn inside(size: [u32; 3], at: [u32; 3], what: &str, field: &str) -> Result<(), Failed> {
    if at.iter().zip(size).all(|(one, side)| *one < side) {
        return Ok(());
    }
    Err(limited(
        Code::Outside,
        field,
        format!(
            "{what} {at:?} is outside the world of {} by {} by {} cells",
            size[0], size[1], size[2]
        ),
        largest(size),
    ))
}

pub(super) fn world_number() -> Json {
    whole(
        "the number grit_world_create answered with",
        1,
        i64::from(u32::MAX),
    )
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

fn budget_shown() -> Json {
    with(
        whole(
            &format!("milliseconds this call may take; {BUDGET_MS} if left out"),
            1,
            i64::try_from(BUDGET_MOST_MS).unwrap_or(i64::MAX),
        ),
        vec![(
            "default",
            Json::Int(i64::try_from(BUDGET_MS).unwrap_or(i64::MAX)),
        )],
    )
}

pub(super) fn tool(
    name: &str,
    what: &str,
    mut properties: Vec<(&str, Json)>,
    required: &[&str],
) -> Json {
    properties.push(("budget_ms", budget_shown()));
    map(vec![
        ("name", text(name)),
        ("description", text(what)),
        ("inputSchema", object(properties, required)),
    ])
}

fn rules_shown() -> Json {
    tool(
        "grit_world_rules",
        "Name the rules a world can follow and how many parameters each reads.",
        Vec::new(),
        &[],
    )
}

fn names() -> Vec<&'static str> {
    rules().iter().map(|one| one.name()).collect()
}

fn create_shown() -> Json {
    let side = |what: &str| whole(what, 0, i64::from(u32::MAX));
    tool(
        "grit_world_create",
        "Open a solid world of wide by deep by tall cells, each side rounded up to a power of two, every cell 0, under a rule. Returns its number. Refused when the device cannot hold it.",
        vec![
            ("wide", side("cells along x")),
            ("deep", side("cells along y, which is down")),
            ("tall", side("cells along z; at least 2")),
            ("rule", choice("a name grit_world_rules lists", &names())),
            (
                "sparse",
                with(
                    property(
                        "boolean",
                        "hold only bricks that are not all one word; false if left out",
                    ),
                    vec![("default", Json::Bool(false))],
                ),
            ),
            (
                "slots",
                whole(
                    "for a sparse world, how many bricks it may hold; if left out, a quarter of them, but never fewer than hold 2097152 cells",
                    1,
                    i64::from(u32::MAX),
                ),
            ),
            (
                "params",
                map(vec![
                    ("type", text("array")),
                    ("items", word()),
                    (
                        "description",
                        text(
                            "exactly as many words as the rule reads; heat reads ignition, chance out of 16, flare",
                        ),
                    ),
                    ("default", Json::List(Vec::new())),
                ]),
            ),
        ],
        &["wide", "deep", "tall", "rule"],
    )
}

fn write_shown() -> Json {
    tool(
        "grit_world_write",
        &format!(
            "Set cells between frames and wake them, so the next frame steps them. At most {READ_MOST} cells a call; the last write to a cell wins."
        ),
        vec![
            ("world", world_number()),
            (
                "cells",
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
                    ("description", text("each [x, y, z, word]")),
                    (
                        "maxItems",
                        Json::Int(i64::try_from(READ_MOST).unwrap_or(i64::MAX)),
                    ),
                ]),
            ),
        ],
        &["world", "cells"],
    )
}

fn step_shown() -> Json {
    tool(
        "grit_world_step",
        &format!(
            "Run frames, 1 to {STEP_MOST}. Returns the frame count so far, the rule's tally words of the last frame and how many octs it lists for the next."
        ),
        vec![
            ("world", world_number()),
            (
                "frames",
                with(
                    whole("how many frames; 1 if left out", 1, i64::from(STEP_MOST)),
                    vec![("default", Json::Int(1))],
                ),
            ),
        ],
        &["world"],
    )
}

fn read_shown() -> Json {
    tool(
        "grit_world_read",
        &format!(
            "Read the cells of a box, corners included, x fastest, then y, then z, and count them by kind. At most {READ_MOST} cells."
        ),
        vec![
            ("world", world_number()),
            ("from", corner_shown("the least corner, [x, y, z]")),
            ("to", corner_shown("the greatest corner, [x, y, z]")),
        ],
        &["world", "from", "to"],
    )
}

fn slice_shown() -> Json {
    tool(
        "grit_world_slice",
        &format!(
            "Read one layer across an axis as rows. Kinds are one hex digit a cell, + past 15; words are numbers. At most {SLICE_MOST} cells."
        ),
        vec![
            ("world", world_number()),
            (
                "axis",
                with(
                    choice("x, y or z; z if left out", &["x", "y", "z"]),
                    vec![("default", text("z"))],
                ),
            ),
            (
                "at",
                whole("which layer along the axis", 0, i64::from(u32::MAX)),
            ),
            (
                "as",
                with(
                    choice("kinds or words; kinds if left out", &["kinds", "words"]),
                    vec![("default", text("kinds"))],
                ),
            ),
            (
                "cursor",
                with(
                    whole(
                        "the row the answer starts at; 0 if left out. An answer that stops before the last row names the row to ask for next as next",
                        0,
                        i64::from(u32::MAX),
                    ),
                    vec![("default", Json::Int(0))],
                ),
            ),
        ],
        &["world", "at"],
    )
}

fn close_shown() -> Json {
    tool(
        "grit_world_close",
        "Close a world and free its memory on the device. Its number is never used again.",
        vec![("world", world_number())],
        &["world"],
    )
}

fn describe_shown() -> Json {
    tool(
        "grit_describe",
        "Name every refusal code with what to do about it, every limit, every tool, the device and the build features of this server.",
        Vec::new(),
        &[],
    )
}

fn listed_rules(_: &mut Server<'_>, _: &Json) -> Result<Json, Failed> {
    Ok(map(vec![("rules", super::known_rules())]))
}

pub(super) fn reported(
    live: &mut Live<'_>,
    mut answer: Vec<(&'static str, Json)>,
) -> Result<Json, Failed> {
    if let Some(used) = live.world.pool_use()? {
        answer.push(("sparse", Json::Bool(true)));
        answer.push(("pool_use", super::pooled(used)));
    }
    Ok(map(answer))
}

pub(super) fn admitted(
    server: &Server<'_>,
    host: &Host,
    sizes: &[usize],
    what: &str,
) -> Result<u64, Failed> {
    host.limits().admits(sizes, None)?;
    let bytes = world_cost(sizes);
    server.book.memory.admits(bytes, what)?;
    host.limits()
        .admits_on(sizes, None, host.heap())
        .map_err(|why| match why {
            crate::host::Error::TooMuchMemory { bytes, heap } => Box::new(budget::Short {
                asked: bytes,
                held: server.book.memory.held(),
                allowed: heap,
                what: format!("{what}, on a device of {heap} bytes,"),
            }) as Failed,
            other => Box::new(other) as Failed,
        })?;
    Ok(bytes)
}

pub(super) fn first_pace(world: &World<'_>) -> Pace {
    let cells = world.pool().map_or_else(
        || world.size().iter().map(|&side| u64::from(side)).product(),
        |bricks| u64::from(bricks.capacity()) * bricks.cells() as u64,
    );
    Pace::first(cells)
}

fn created(server: &mut Server<'_>, held: &Json) -> Result<Json, Failed> {
    let mut known = rules_on(server.host);
    let asked = words_of(held, "rule")?;
    let at = known
        .iter()
        .position(|one| one.name() == asked)
        .ok_or_else(|| unread("rule"))?;
    let rule = known.swap_remove(at);
    let params = ints(
        held.get("params").ok_or_else(|| unread("params"))?,
        "params",
    )?;
    if params.len() != rule.params() {
        return Err(limited(
            Code::Params,
            "$.params",
            format!(
                "rule {} reads {} parameters, and {} were given",
                rule.name(),
                rule.params(),
                params.len()
            ),
            counted(u32::try_from(rule.params()).unwrap_or(u32::MAX)),
        ));
    }
    let wide = int(held, "wide")?;
    let deep = int(held, "deep")?;
    let tall = int(held, "tall")?;
    let host = server.host.ok_or(crate::host::Error::NoComputeDevice)?;
    let sparse = matches!(held.get("sparse"), Some(Json::Bool(true)));
    let slots = match held.get("slots") {
        None => None,
        Some(_) => Some(int(held, "slots")?),
    };
    let mut made = Made {
        rule: rule.name().to_owned(),
        params: params.clone(),
        size: [wide, deep, tall],
        sparse,
        slots,
    };
    let (grid, sizes) = shaped(host, rule.as_ref(), &made)?;
    made.size = [grid.wide, grid.deep, grid.tall];
    let bytes = admitted(server, host, &sizes, "the world")?;
    let world = World::create(host, &made.branch())?;
    let id = server.number()?;
    let pace = first_pace(&world);
    let mut live = super::Live { world, rule };
    let listed = live.world.listed()?;
    let cells = u64::from(grid.wide) * u64::from(grid.deep) * u64::from(grid.tall);
    let mut answer = vec![
        ("world", counted(id)),
        ("rule", text(live.rule.name())),
        (
            "params",
            Json::List(params.iter().map(|one| counted(*one)).collect()),
        ),
        ("size", sized(live.world.size())),
        ("cells", Json::Int(i64::try_from(cells)?)),
        ("frame", counted(live.world.frame())),
        ("listed", counted(listed)),
    ];
    if let Some(used) = live.world.pool_use()? {
        answer.push(("sparse", Json::Bool(true)));
        answer.push(("pool_use", super::pooled(used)));
    }
    if let Some(note) = server.adopt(id, live, Ledger::created(made), bytes, pace) {
        answer.push(("journal", note));
    }
    Ok(map(answer))
}

fn written(server: &mut Server<'_>, held: &Json) -> Result<Json, Failed> {
    let number = int(held, "world")?;
    let live = server.world(held)?;
    let size = live.world.size();
    let list = held
        .get("cells")
        .and_then(Json::list)
        .ok_or_else(|| unread("cells"))?;
    let mut points = Vec::with_capacity(list.len());
    for (at, one) in list.iter().enumerate() {
        let got = ints(one, "cells")?;
        let [x, y, z, word] = <[u32; 4]>::try_from(got.as_slice()).map_err(|_| unread("cells"))?;
        inside(
            size,
            [x, y, z],
            &format!("cell {at} at"),
            &format!("$.cells[{at}]"),
        )?;
        points.push(([x, y, z], word));
    }
    let began = Instant::now();
    live.world.write(&points)?;
    let spent = began.elapsed();
    let written = u32::try_from(points.len())?;
    let journal = server.journaled(number, Entry::Write(points), spent);
    let snapshot = server.settle(number);
    let live = server.world(held)?;
    let listed = live.world.listed()?;
    let frame = live.world.frame();
    let mut answer = vec![
        ("world", counted(number)),
        ("written", counted(written)),
        ("frame", counted(frame)),
        ("listed", counted(listed)),
    ];
    if let Some(used) = live.world.pool_use()? {
        answer.push(("sparse", Json::Bool(true)));
        answer.push(("pool_use", super::pooled(used)));
    }
    if let Some(note) = journal.or(snapshot) {
        answer.push(("journal", note));
    }
    Ok(map(answer))
}

#[cfg(feature = "faults")]
fn wedged_after(server: &Server<'_>, batch: u32) -> bool {
    let armed = server
        .book
        .faults
        .lock()
        .map(|mut faults| {
            if faults.wedge_after_batch == Some(batch) {
                faults.wedge_after_batch = None;
                true
            } else {
                false
            }
        })
        .unwrap_or(false);
    if armed && let Some(host) = server.host {
        host.wedge();
    }
    armed
}

#[cfg(not(feature = "faults"))]
const fn wedged_after(_: &Server<'_>, _: u32) -> bool {
    false
}

fn stepped(server: &mut Server<'_>, held: &Json) -> Result<Json, Failed> {
    let number = int(held, "world")?;
    let frames = int(held, "frames")?;
    server.world(held)?;
    let mut pace = server
        .book
        .held
        .get(&number)
        .map(|held| held.pace)
        .ok_or("the world has no journal")?;
    let mut done = 0_u32;
    let mut spent = Duration::ZERO;
    let mut stopped = None;
    let mut failure: Option<Failed> = None;
    let mut batch = 0_u32;
    while done < frames {
        if let Some(stop) = server.stop() {
            stopped = Some(stop);
            break;
        }
        let step = pace.batch(frames - done, server.clock.left());
        if step == 0 {
            stopped = Some(Stop::Budget);
            break;
        }
        if let Err(why) = faults::batch() {
            failure = Some(why);
            break;
        }
        let began = Instant::now();
        let ran = server.world(held)?.world.step(step);
        let took = began.elapsed();
        if let Err(why) = ran {
            failure = Some(why);
            break;
        }
        if wedged_after(server, batch) {
            failure = Some(Box::new(crate::host::Error::Wedged { after: took }));
            break;
        }
        pace.ran(step, took);
        let stats = &mut server.book.stats;
        stats
            .batches
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if took > stats.longest {
            stats.longest = took;
            stats.longest_frames = step;
        }
        spent = spent.saturating_add(took);
        done += step;
        batch += 1;
    }
    if let Some(held) = server.book.held.get_mut(&number) {
        held.pace = pace;
    }
    let journal = if done > 0 {
        server.journaled(number, Entry::Step(done), spent)
    } else {
        None
    };
    let frame = server
        .book
        .held
        .get(&number)
        .map_or(0, |held| held.ledger.frame);
    if let Some(why) = failure {
        let (mut refusal, _) = super::refused_with(why.as_ref());
        refusal.message = format!(
            "{}; {done} of the {frames} frames ran before it, and the world is at frame {frame}",
            refusal.message
        );
        if super::lost_device(why.as_ref()) {
            refusal.code = Code::DeviceLost;
            return Err(Box::new(Lost(refusal)));
        }
        return Err(Box::new(refusal));
    }
    let snapshot = server.settle(number);
    let live = server.world(held)?;
    let tally = live.world.tally()?;
    let listed = live.world.listed()?;
    let mut answer = vec![
        ("world", counted(number)),
        ("frames", counted(frames)),
        ("frame", counted(live.world.frame())),
        (
            "tally",
            Json::List(tally.iter().map(|one| counted(*one)).collect()),
        ),
        ("listed", counted(listed)),
    ];
    if let Some(used) = live.world.pool_use()? {
        answer.push(("sparse", Json::Bool(true)));
        answer.push(("pool_use", super::pooled(used)));
    }
    if let Some(stop) = stopped {
        answer.push(("stopped", text(stop.name())));
        answer.push(("ran", counted(done)));
        answer.push(("hint", text(stop.code().hint())));
    }
    if let Some(note) = journal.or(snapshot) {
        answer.push(("journal", note));
    }
    Ok(map(answer))
}

#[derive(Debug)]
pub struct Lost(pub Refusal);

impl std::fmt::Display for Lost {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(out, "{}", self.0.message)
    }
}

impl std::error::Error for Lost {}

pub(super) fn cells_at(world: &mut World<'_>, points: &[[u32; 3]]) -> Result<Vec<u32>, Failed> {
    let image = world.image()?;
    let size = image.size;
    match &image.state {
        State::Dense(cells) => points
            .iter()
            .map(|at| {
                let index = (u64::from(at[2]) * u64::from(size[1]) + u64::from(at[1]))
                    * u64::from(size[0])
                    + u64::from(at[0]);
                usize::try_from(index)
                    .ok()
                    .and_then(|index| cells.get(index).copied())
                    .ok_or_else(|| "the world read back short".into())
            })
            .collect(),
        State::Sparse {
            sides, table, pool, ..
        } => {
            let lattice = lattice_of(size, *sides);
            let per = brick_cells(*sides);
            Ok(points
                .iter()
                .map(|&at| {
                    let (brick, offset) = place(at, *sides, lattice);
                    let slot = table.get(brick * 2).copied().unwrap_or(UNIFORM);
                    let value = table.get(brick * 2 + 1).copied().unwrap_or(0);
                    if slot == UNIFORM {
                        value
                    } else {
                        pool.get(slot as usize * per + offset).copied().unwrap_or(0)
                    }
                })
                .collect())
        }
    }
}

fn read(server: &mut Server<'_>, held: &Json) -> Result<Json, Failed> {
    let number = int(held, "world")?;
    let live = server.world(held)?;
    let size = live.world.size();
    let from = corner(held, "from")?;
    let to = corner(held, "to")?;
    inside(size, from, "from", "$.from")?;
    inside(size, to, "to", "$.to")?;
    if from.iter().zip(to).any(|(one, other)| *one > other) {
        return Err(refused(
            Code::Reversed,
            "$.to",
            format!("the box from {from:?} to {to:?} has its first corner past its last"),
        ));
    }
    let cells: u64 = from
        .iter()
        .zip(to)
        .map(|(one, other)| u64::from(other - one) + 1)
        .product();
    if cells > READ_MOST {
        return Err(limited(
            Code::TooMany,
            "$.to",
            format!("the box names {cells} cells, more than the {READ_MOST} one call carries"),
            Json::Int(i64::try_from(READ_MOST)?),
        ));
    }
    let mut points = Vec::new();
    for z in from[2]..=to[2] {
        for y in from[1]..=to[1] {
            for x in from[0]..=to[0] {
                points.push([x, y, z]);
            }
        }
    }
    let mut words = Vec::new();
    let mut kinds = BTreeMap::new();
    for word in cells_at(&mut live.world, &points)? {
        *kinds.entry(live.rule.kind(word)).or_insert(0_u32) += 1;
        words.push(counted(word));
    }
    Ok(map(vec![
        ("world", counted(number)),
        ("frame", counted(live.world.frame())),
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

fn sliced(server: &mut Server<'_>, held: &Json) -> Result<Json, Failed> {
    let number = int(held, "world")?;
    let live = server.world(held)?;
    let axis = words_of(held, "axis")?;
    let (across, down, through) = match axis {
        "x" => (1, 2, 0),
        "y" => (0, 2, 1),
        _ => (0, 1, 2),
    };
    let at = int(held, "at")?;
    let size = live.world.size();
    let side = |axis: usize| size.get(axis).copied().unwrap_or(0);
    if at >= side(through) {
        return Err(limited(
            Code::Outside,
            "$.at",
            format!(
                "layer {axis} {at} is outside the world of {} by {} by {} cells",
                size[0], size[1], size[2]
            ),
            counted(side(through).saturating_sub(1)),
        ));
    }
    let shown = words_of(held, "as")?;
    let cells = u64::from(side(across)) * u64::from(side(down));
    if cells > SLICE_MOST {
        return Err(limited(
            Code::TooMany,
            "$.axis",
            format!("the slice names {cells} cells, more than the {SLICE_MOST} one call carries"),
            Json::Int(i64::try_from(SLICE_MOST)?),
        ));
    }
    let cursor = int(held, "cursor")?;
    if cursor >= side(down) {
        return Err(limited(
            Code::Outside,
            "$.cursor",
            format!("row {cursor} is past the {} rows of this slice", side(down)),
            counted(side(down).saturating_sub(1)),
        ));
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
    let mut points = Vec::new();
    for row in cursor..side(down) {
        for column in 0..side(across) {
            points.push(place(row, column));
        }
    }
    let state = cells_at(&mut live.world, &points)?;
    let mut rows = Vec::new();
    let mut read = state.into_iter();
    let mut bytes = 0_usize;
    let mut next = None;
    for row in cursor..side(down) {
        let mut kinds = String::new();
        let mut words = Vec::new();
        for _ in 0..side(across) {
            let word = read.next().ok_or("the world read back short")?;
            if shown == "kinds" {
                let kind = live.rule.kind(word);
                kinds.push(
                    DIGITS
                        .get(kind as usize)
                        .map_or('+', |digit| char::from(*digit)),
                );
            } else {
                words.push(counted(word));
            }
        }
        let one = if shown == "kinds" {
            text(&kinds)
        } else {
            Json::List(words)
        };
        bytes += crate::wire::written(&one).len() + 1;
        if bytes > PAGE_BYTES && !rows.is_empty() {
            next = Some(row);
            break;
        }
        rows.push(one);
    }
    let named = ["x", "y", "z"];
    let name = |axis: usize| text(named.get(axis).copied().unwrap_or("x"));
    let mut answer = vec![
        ("world", counted(number)),
        ("frame", counted(live.world.frame())),
        ("axis", text(axis)),
        ("at", counted(at)),
        ("columns", name(across)),
        ("rows_along", name(down)),
        ("as", text(shown)),
        ("rows", Json::List(rows)),
    ];
    if let Some(next) = next {
        answer.push(("next", counted(next)));
    }
    Ok(map(answer))
}

fn closed(server: &mut Server<'_>, held: &Json) -> Result<Json, Failed> {
    let world = int(held, "world")?;
    if !server.book.held.contains_key(&world) {
        server.world(held)?;
    }
    server.forget(world);
    Ok(map(vec![
        ("world", counted(world)),
        ("closed", Json::Bool(true)),
    ]))
}

fn number(held: impl TryInto<i64>) -> Json {
    held.try_into().map_or(Json::Null, Json::Int)
}

fn described(server: &mut Server<'_>, _: &Json) -> Result<Json, Failed> {
    Ok(describe(&server.status()))
}

#[must_use]
pub fn describe(status: &Status) -> Json {
    let tools: Vec<Json> = super::listing_all()
        .iter()
        .filter_map(|one| one.get("name").cloned())
        .collect();
    map(vec![
        (
            "server",
            map(vec![
                ("name", text("grit")),
                ("version", text("0.0.0")),
                ("protocol", text(PROTOCOL)),
            ]),
        ),
        ("tools", Json::List(tools)),
        ("codes", errors::described()),
        (
            "limits",
            map(vec![
                ("line_bytes", number(LINE_MOST)),
                ("answer_bytes", number(ANSWER_MOST)),
                ("page_bytes", number(PAGE_BYTES)),
                ("id_bytes", number(ID_MOST)),
                ("depth", number(DEPTH_MOST)),
                ("read_cells", number(READ_MOST)),
                ("write_cells", number(READ_MOST)),
                ("slice_cells", number(SLICE_MOST)),
                ("step_frames", number(STEP_MOST)),
                ("pool_least_cells", number(POOL_LEAST_CELLS)),
                ("crowd", number(crate::serve::CROWD)),
                ("rounds", number(crate::serve::ROUNDS)),
                ("work", number(crate::serve::WORK)),
                ("rungs", number(crate::serve::RUNGS)),
                ("budget_ms", number(BUDGET_MS)),
                ("budget_most_ms", number(BUDGET_MOST_MS)),
                ("submission_ms", number(SUBMIT_MOST_MS)),
                ("batch_frames", number(BATCH_MOST)),
                ("name_bytes", number(journal::NAME_MOST)),
                ("world_least_bytes", number(budget::WORLD_LEAST)),
                ("queue_calls", number(super::exec::QUEUE_MOST)),
                (
                    "memory_share",
                    text(&format!(
                        "{}/{} of the device heap unless the server was started with its own",
                        budget::SHARE_NUMERATOR,
                        budget::SHARE_DENOMINATOR
                    )),
                ),
            ]),
        ),
        ("device", status.device.clone()),
        ("memory", status.memory.clone()),
        ("worlds", status.worlds.clone()),
        ("submissions", status.submissions.clone()),
        ("directory", status.directory.clone()),
        ("faults_caught", number(super::transport::faults())),
        (
            "features",
            map(vec![
                ("window", Json::Bool(cfg!(feature = "window"))),
                ("deep", Json::Bool(cfg!(feature = "deep"))),
                ("faults", Json::Bool(cfg!(feature = "faults"))),
            ]),
        ),
    ])
}
