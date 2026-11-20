use std::collections::BTreeMap;
use std::time::Duration;

use crate::branch::World;
use crate::host::Host;
use crate::wire::{Json, counted, map, text};

use super::budget::{
    self, BATCH_MOST, BUDGET_MOST_MS, BUDGET_MS, Pace, SUBMIT_MOST_MS, Stop, Work, world_cost,
};
use super::cost::{Cost, Priced};
use super::errors::{self, Code, Refusal};
use super::journal::{self, Entry, Ledger, Made};
use super::schema::{choice, object, property, whole, with, word};
use super::transport::{ANSWER_MOST, DEPTH_MOST, ID_MOST, LINE_MOST, PROTOCOL};
use super::{
    DIGITS, Failed, Listed, Live, PAGE_BYTES, POOL_LEAST_CELLS, READ_MOST, SLICE_MOST, STEP_MOST,
    Server, Status, Tool, rules, rules_on, shaped,
};
use crate::calibrate::Key;

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
    use std::time::Duration;

    use super::{Code, Failed, Refusal};
    use crate::clock::Time;

    thread_local! {
        static BATCH: Cell<Option<u32>> = const { Cell::new(None) };
        static STALL: Cell<Option<(u32, Duration, bool)>> = const { Cell::new(None) };
        static SUBMISSION: Cell<Duration> = const { Cell::new(Duration::ZERO) };
        static CELLS: Cell<Duration> = const { Cell::new(Duration::ZERO) };
    }

    pub fn fail_batch(nth: u32) {
        BATCH.set(Some(nth));
    }

    pub fn stall_batch(nth: u32, by: Duration) {
        STALL.set(Some((nth, by, false)));
    }

    pub fn stall_every(by: Duration) {
        STALL.set(Some((0, by, true)));
    }

    pub fn stall_submission(by: Duration) {
        SUBMISSION.set(by);
    }

    pub fn stall_cells(by: Duration) {
        CELLS.set(by);
    }

    pub(crate) fn cells(time: &dyn Time, count: usize) {
        let by = CELLS.get();
        if !by.is_zero() {
            time.sleep(by.saturating_mul(u32::try_from(count).unwrap_or(u32::MAX)));
        }
    }

    pub(crate) fn stall(time: &dyn Time, frames: u32) {
        let fixed = SUBMISSION.get();
        if !fixed.is_zero() {
            time.sleep(fixed);
        }
        match STALL.get() {
            Some((0, by, every)) => {
                if every {
                    time.sleep(by.saturating_mul(frames));
                } else {
                    STALL.set(None);
                    time.sleep(by);
                }
            }
            Some((nth, by, every)) => STALL.set(Some((nth - 1, by, every))),
            None => {}
        }
    }

    pub fn disarm() {
        BATCH.set(None);
        STALL.set(None);
        SUBMISSION.set(Duration::ZERO);
        CELLS.set(Duration::ZERO);
    }

    pub(in crate::serve3) fn batch() -> Result<(), Failed> {
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
pub(crate) mod faults {
    use super::Failed;

    pub(in crate::serve3) const fn batch() -> Result<(), Failed> {
        Ok(())
    }

    pub(crate) fn stall(_: &dyn crate::clock::Time, _: u32) {}

    pub(crate) fn cells(_: &dyn crate::clock::Time, _: usize) {}
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

pub(super) fn ints(one: &Json, key: &str) -> Result<Vec<u32>, Failed> {
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

fn names() -> Vec<String> {
    rules().iter().map(|one| one.name().to_owned()).collect()
}

fn create_shown() -> Json {
    let side = |what: &str| whole(what, 0, i64::from(u32::MAX));
    let names = names();
    let named: Vec<&str> = names.iter().map(String::as_str).collect();
    tool(
        "grit_world_create",
        "Open a solid world of wide by deep by tall cells, each side rounded up to a power of two, every cell 0, under a rule or a law. Returns its number.",
        vec![
            ("wide", side("cells along x")),
            ("deep", side("cells along y, which is down")),
            ("tall", side("cells along z; at least 2")),
            (
                "rule",
                choice("a name grit_world_rules lists; give this or law", &named),
            ),
            (
                "law",
                property(
                    "string",
                    "the JSON text of a law, as tests/law/*.law.json; give this or rule. The answer names its hash and quiet class: proved steps listed octs, whole steps every oct and only dense. A law that looks (halo 1) runs only dense",
                ),
            ),
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
                            "exactly as many words as the rule or law reads; heat reads ignition, chance out of 16, flare",
                        ),
                    ),
                    ("default", Json::List(Vec::new())),
                ]),
            ),
        ],
        &["wide", "deep", "tall"],
    )
}

fn write_shown() -> Json {
    tool(
        "grit_world_write",
        &format!(
            "Set cells of one plane between frames and wake them, so the next frame steps them. At most {READ_MOST} cells a call; the last write to a cell wins."
        ),
        vec![
            ("world", world_number()),
            ("plane", plane_shown()),
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
            ("plane", plane_shown()),
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
            ("plane", plane_shown()),
        ],
        &["world", "at"],
    )
}

fn plane_shown() -> Json {
    with(
        whole(
            "which plane of a law world's words, below its channels; 0 if left out",
            0,
            i64::from(crate::law::limits::CHANNELS - 1),
        ),
        vec![("default", Json::Int(0))],
    )
}

fn plane_of(held: &Json, world: &World<'_>) -> Result<u32, Failed> {
    let plane = int(held, "plane")?;
    let planes = world.planes();
    if plane >= planes {
        return Err(limited(
            Code::Outside,
            "$.plane",
            format!("plane {plane} is past the world's {planes} planes"),
            counted(planes.saturating_sub(1)),
        ));
    }
    Ok(plane)
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

fn chosen(server: &Server<'_>, held: &Json) -> Result<Box<dyn Listed>, Failed> {
    match (held.get("rule"), held.get("law")) {
        (Some(_), None) => {
            let mut known = rules_on(server.host);
            let asked = words_of(held, "rule")?;
            let at = known
                .iter()
                .position(|one| one.name() == asked)
                .ok_or_else(|| unread("rule"))?;
            Ok(known.swap_remove(at))
        }
        (None, Some(_)) => {
            let law = super::law::compiled(words_of(held, "law")?)?;
            Ok(Box::new(super::law::Lawful::new(
                law,
                crate::law::quiet::Class::Proved,
            )))
        }
        (given, _) => Err(refused(
            Code::RuleOrLaw,
            "$",
            if given.is_some() {
                "give a rule or a law, and not both".to_owned()
            } else {
                "give a rule, a name grit_world_rules lists, or a law, the JSON text of one"
                    .to_owned()
            },
        )),
    }
}

fn created(server: &mut Server<'_>, held: &Json) -> Result<Json, Failed> {
    let rule = chosen(server, held)?;
    let params = ints(
        held.get("params").ok_or_else(|| unread("params"))?,
        "params",
    )?;
    if params.len() != rule.params() {
        return Err(limited(
            Code::Params,
            "$.params",
            format!(
                "{} {} reads {} parameters, and {} were given",
                if rule.lawful().is_some() {
                    "law"
                } else {
                    "rule"
                },
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
    let sparse = matches!(held.get("sparse"), Some(Json::Bool(true)));
    let rule: Box<dyn Listed> = match rule.lawful() {
        None => rule,
        Some(lawful) => {
            let law = lawful.law().clone();
            super::law::halo_refused(&law, sparse, "$.sparse")?;
            let class = super::law::background_class(&law, &params);
            if sparse && !class.proved() {
                return Err(super::law::quiet_refused(&law, &class, "$.sparse"));
            }
            Box::new(super::law::Lawful::new(law, class))
        }
    };
    let host = server.host.ok_or(crate::host::Error::NoComputeDevice)?;
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
    server.fits(Work::build(sparse), bytes, "building the world")?;
    let began = server.now();
    server.slowly(Work::build(sparse));
    let world = match rule.lawful() {
        None => World::create(host, &made.branch())?,
        Some(lawful) => World::create_law_stepped(
            host,
            lawful.law(),
            &made.branch(),
            super::law::stepping_of(lawful.class(), sparse),
        )?,
    };
    let took = server.since(began);
    server.book.rates.measure(Work::build(sparse), bytes, took);
    server.sampled(&Key::Build { sparse }, (bytes, 1), took, None);
    server.priced(build_cost(bytes));
    let id = server.number()?;
    let pace = first_pace(&world);
    let law = rule
        .lawful()
        .map(|lawful| std::sync::Arc::from(lawful.law().canonical()));
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
    if let Some(lawful) = live.rule.lawful() {
        answer.extend(lawful.told());
    }
    if let Some(used) = live.world.pool_use()? {
        answer.push(("sparse", Json::Bool(true)));
        answer.push(("pool_use", super::pooled(used)));
    }
    let ledger = Ledger::created(made).with_law(law);
    if let Some(note) = server.adopt(id, live, ledger, bytes, pace) {
        answer.push(("journal", note));
    }
    Ok(map(answer))
}

fn written(server: &mut Server<'_>, held: &Json) -> Result<Json, Failed> {
    let number = int(held, "world")?;
    let live = server.world(held)?;
    let size = live.world.size();
    let plane = plane_of(held, &live.world)?;
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
    let owed = server.owed(number)?;
    let sparse = server.world(held)?.world.pool().is_some();
    let total = points.len();
    let mut done = 0_usize;
    let mut spent = Duration::ZERO;
    let mut stopped = None;
    let mut failure: Option<Failed> = None;
    while done < total {
        if done > 0
            && let Some(stop) = server.stop()
        {
            stopped = Some(stop);
            break;
        }
        let per_cell = server
            .book
            .rates
            .estimate(Work::Write, 16)
            .max(Duration::from_nanos(1));
        let room = server.clock.left().min(AIM_WRITE);
        let fits = usize::try_from(room.as_nanos() / per_cell.as_nanos()).unwrap_or(usize::MAX);
        let probing = server.book.rates.observed(Work::Write) < budget::PADDED;
        let chunk = if sparse {
            total
        } else if probing {
            1
        } else {
            fits.clamp(1, total - done)
        };
        let piece = points.get(done..done + chunk).unwrap_or_default();
        let began = server.now();
        faults::cells(server.time(), piece.len());
        let ran = server.world(held)?.world.write_plane(plane, piece);
        let took = server.since(began);
        if let Err(why) = ran {
            failure = Some(why);
            break;
        }
        server
            .book
            .rates
            .measure(Work::write(sparse), 16 * chunk as u64, took);
        server.sampled(
            &Key::Write { sparse },
            (u64::try_from(chunk).unwrap_or(u64::MAX), 1),
            took,
            None,
        );
        spent = spent.saturating_add(took);
        done += chunk;
    }
    let written = u32::try_from(done)?;
    let journal = if done > 0 {
        points.truncate(done);
        let entry = if plane == 0 {
            Entry::Write(points)
        } else {
            Entry::WritePlane(plane, points)
        };
        server
            .journaled(number, entry, spent, write_cost(done, sparse))
            .or(owed)
    } else {
        owed
    };
    if let Some(why) = failure {
        let (mut refusal, _) = super::refused_with(why.as_ref());
        refusal.message = format!(
            "{}; {done} of the {total} cells were written before it",
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
    let listed = live.world.listed()?;
    let frame = live.world.frame();
    let mut answer = vec![
        ("world", counted(number)),
        ("written", counted(written)),
        ("frame", counted(frame)),
        ("listed", counted(listed)),
    ];
    if plane > 0 {
        answer.insert(1, ("plane", counted(plane)));
    }
    if let Some(used) = live.world.pool_use()? {
        answer.push(("sparse", Json::Bool(true)));
        answer.push(("pool_use", super::pooled(used)));
    }
    if let Some(stop) = stopped {
        answer.push(("stopped", text(stop.name())));
        answer.push(("asked", counted(u32::try_from(total)?)));
        answer.push(("hint", text(stop.code().hint())));
    }
    if let Some(note) = journal.or(snapshot) {
        answer.push(("journal", note));
    }
    Ok(map(answer))
}

const AIM_WRITE: Duration = Duration::from_millis(budget::SUBMIT_MOST_MS / 2);

#[must_use]
pub const fn build_cost(bytes: u64) -> Cost {
    Cost {
        kind: Priced::Build,
        units: bytes,
        submits: 1,
    }
}

#[must_use]
pub fn write_cost(cells: usize, sparse: bool) -> Cost {
    let chunks = if sparse {
        cells.min(1)
    } else {
        cells.div_ceil(journal::WRITE_CHUNK)
    };
    Cost {
        kind: Priced::Write,
        units: u64::try_from(cells).unwrap_or(u64::MAX),
        submits: u32::try_from(chunks).unwrap_or(u32::MAX),
    }
}

#[must_use]
pub fn frame_cells(size: [u32; 3], bricks: Option<(u32, usize)>) -> u64 {
    match bricks {
        Some((held, per)) => u64::from(held).saturating_mul(u64::try_from(per).unwrap_or(u64::MAX)),
        None => size
            .iter()
            .fold(1_u64, |all, side| all.saturating_mul(u64::from(*side))),
    }
}

#[must_use]
pub fn world_frame_cells(world: &World<'_>) -> u64 {
    let bricks = world
        .pool()
        .zip(world.bricks_seen())
        .map(|(pool, held)| (held, pool.cells()));
    frame_cells(world.size(), bricks)
}

#[must_use]
pub fn by_fallback(world: &World<'_>) -> bool {
    world.pool().is_some() && world.bricks_seen().is_none()
}

pub(super) fn frame_key(live: &super::Live<'_>) -> Key {
    let sparse = live.world.pool().is_some();
    match live.rule.lawful() {
        Some(lawful) => Key::law(lawful.law().canonical(), sparse),
        None => Key::rule(live.rule.name(), sparse),
    }
}

struct Sound<'host> {
    cpu: Option<&'host crate::calibrate::cpu::Driver>,
    needed: bool,
    total: Option<Duration>,
    mark: Option<crate::calibrate::cpu::Mark>,
    spans: Vec<Option<(u64, u64)>>,
}

impl<'host> Sound<'host> {
    fn of(host: Option<&'host Host>, needed: bool) -> Self {
        Self {
            cpu: host
                .filter(|host| host.on_cpu())
                .and_then(Host::driver_threads),
            needed,
            total: Some(Duration::ZERO),
            mark: None,
            spans: Vec::new(),
        }
    }

    fn before(&mut self) {
        self.mark = match self.cpu {
            Some(driver) if self.needed => driver.mark(),
            _ => None,
        };
    }

    fn after(&mut self, world: &World<'_>, frames: u32) {
        if self.cpu.is_none() {
            self.spans.push(world.on_device_ticks());
        }
        let measured = crate::calibrate::scripted_sound().unwrap_or_else(|| match self.cpu {
            Some(driver) => self
                .mark
                .take()
                .zip(driver.closing())
                .map(|(from, to)| driver.between(from, to)),
            None => world.on_device().map(|frame| frame * frames),
        });
        self.total = self.total.zip(measured).map(|(all, one)| all + one);
    }

    fn taken(&self) -> Option<(Duration, bool)> {
        let first = self.cpu.is_some()
            || (self.spans.len() >= 2 && crate::calibrate::take::kept_busy(&self.spans));
        self.total.map(|total| (total, first))
    }
}

#[must_use]
pub const fn frames_cost(cells: u64, frames: u32) -> Cost {
    Cost {
        kind: Priced::Frame,
        units: cells.saturating_mul(frames as u64),
        submits: frames.div_ceil(BATCH_MOST),
    }
}

#[cfg(feature = "faults")]
pub(super) fn wedged_after(server: &Server<'_>, batch: u32) -> bool {
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
pub(super) const fn wedged_after(_: &Server<'_>, _: u32) -> bool {
    false
}

#[cfg(feature = "faults")]
fn forgotten(server: &mut Server<'_>, held: &Json) -> Result<(), Failed> {
    let armed = server
        .book
        .faults
        .lock()
        .map(|mut faults| std::mem::take(&mut faults.forget_bricks))
        .unwrap_or(false);
    if armed {
        server.world(held)?.world.forget_bricks();
    }
    Ok(())
}

#[cfg(not(feature = "faults"))]
const fn forgotten(_: &mut Server<'_>, _: &Json) -> Result<(), Failed> {
    Ok(())
}

fn stepped(server: &mut Server<'_>, held: &Json) -> Result<Json, Failed> {
    let number = int(held, "world")?;
    let frames = int(held, "frames")?;
    forgotten(server, held)?;
    let cells = world_frame_cells(&server.world(held)?.world);
    let (key, fallback) = {
        let live = server.world(held)?;
        (frame_key(live), by_fallback(&live.world))
    };
    let mut sound = Sound::of(server.host, !server.knows(&key));
    let mut ran = Duration::ZERO;
    let owed = server.owed(number)?;
    let mut pace = server
        .book
        .held
        .get(&number)
        .map(|held| held.pace)
        .ok_or("the world has no journal")?;
    let mut done = 0_u32;
    let mut pending = 0_u32;
    let mut spent = Duration::ZERO;
    let mut stopped = None;
    let mut failure: Option<Failed> = None;
    let mut journal = owed;
    let mut batch = 0_u32;
    while done < frames {
        if let Some(stop) = server.stop() {
            stopped = Some(stop);
            break;
        }
        let step = if batch == 0 {
            pace.opening(frames - done, server.clock.left())
        } else {
            pace.batch(frames - done, server.clock.left())
        };
        if step == 0 {
            stopped = Some(Stop::Budget);
            break;
        }
        if let Err(why) = faults::batch() {
            failure = Some(why);
            break;
        }
        let began = server.now();
        faults::stall(server.time(), step);
        sound.before();
        let stepped = server.world(held)?.world.step(step);
        let took = server.since(began);
        if let Err(why) = stepped {
            failure = Some(why);
            break;
        }
        sound.after(&server.world(held)?.world, step);
        ran = ran.saturating_add(took);
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
        pending += step;
        batch += 1;
        if done < frames && server.overdue(number, spent) {
            if server.far_overdue(number, spent) {
                server.wait_write();
            }
            let fits =
                server.clock.left() >= server.snapping(number) && !server.snap_waiting(number);
            if !fits && !server.far_overdue(number, spent) {
                continue;
            }
            journal = server
                .journaled(
                    number,
                    Entry::Step(pending),
                    spent,
                    frames_cost(cells, pending),
                )
                .or(journal);
            pending = 0;
            spent = Duration::ZERO;
            if !fits {
                stopped = Some(Stop::Budget);
                break;
            }
            match server.snap_now(number) {
                Ok(note) => journal = note.or(journal),
                Err(why) => {
                    if super::lost_device(why.as_ref()) {
                        failure = Some(why);
                        break;
                    }
                    journal = Some(text(&format!(
                        "the snapshot could not be taken: {}",
                        super::refused_with(why.as_ref()).0.message
                    )));
                }
            }
        }
    }
    if let Some(held) = server.book.held.get_mut(&number) {
        held.pace = pace;
    }
    if done > 0 && failure.is_none() && !fallback {
        server.sampled(
            &key,
            (cells.saturating_mul(u64::from(done)), batch),
            ran,
            sound.taken(),
        );
    }
    if pending > 0 {
        journal = server
            .journaled(
                number,
                Entry::Step(pending),
                spent,
                frames_cost(cells, pending),
            )
            .or(journal);
    }
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
    let tally = live.world.exact_tally()?;
    let listed = live.world.listed()?;
    let mut answer = vec![
        ("world", counted(number)),
        ("frames", counted(frames)),
        ("frame", counted(live.world.frame())),
        (
            "tally",
            Json::List(tally.iter().map(|one| exactly(*one)).collect()),
        ),
        ("listed", counted(listed)),
    ];
    if let Some(lawful) = live.rule.lawful() {
        answer.push(("tallied", text(lawful.tallied())));
    }
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

#[must_use]
pub fn exactly(value: u128) -> Json {
    i64::try_from(value).map_or_else(|_| text(&value.to_string()), Json::Int)
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
    let plane = plane_of(held, &live.world)?;
    let mut words = Vec::new();
    let mut kinds = BTreeMap::new();
    for word in live.world.cells_at_plane(&points, plane)? {
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
    let plane = plane_of(held, &live.world)?;
    let state = live.world.cells_at_plane(&points, plane)?;
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
    if let Some(ledger) = server.book.earlier.remove(&world) {
        server.book.broken_earlier.remove(&world);
        if let Some(keep) = server.book.keep.as_ref() {
            keep.forget(world);
        }
        server.book.release(ledger.sum());
        return Ok(map(vec![
            ("world", counted(world)),
            ("closed", Json::Bool(true)),
        ]));
    }
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
                ("world_number_most", number(journal::NUMBER_MOST - 1)),
                (
                    "tally_words",
                    text(
                        "a whole number up to 9223372036854775807, and past it the same number as a decimal string",
                    ),
                ),
                ("queue_calls", number(super::exec::QUEUE_MOST)),
                ("law_tally", text(super::law::TALLIED)),
                ("law_look", text(super::law::LOOK)),
                ("law_reversible", text(super::back::REVERSIBLE)),
                ("law_gradient", text(super::gradient::GRADIENT)),
                ("walk_cells", number(super::gradient::CELLS_MOST)),
                ("walk_work", number(super::gradient::WORK_MOST)),
                ("walk_frames", number(super::gradient::FRAMES_MOST)),
                ("descend_steps", number(super::gradient::STEPS_MOST)),
                ("law_search", text(super::learn::SEARCH)),
                ("run_cells", number(super::learn::CELLS_MOST)),
                ("run_work", number(super::learn::WORK_MOST)),
                ("run_frames", number(super::learn::FRAMES_MOST)),
                ("search_population", number(super::learn::POPULATION_MOST)),
                ("search_generations", number(super::learn::GENERATIONS_MOST)),
                ("search_stale", number(super::learn::STALE)),
                ("law_from_chip", text(super::law::FROM_CHIP)),
                ("law_from_chip_gates", number(crate::law::chip::GATES_SURE)),
                ("law_learn", text(super::law_learn::LEARN)),
                ("law_learn_rows", text(crate::stencil::ORDER)),
                ("learn_cells", number(super::law_learn::CELLS_MOST)),
                ("learn_examples", number(super::law_learn::EXAMPLES_MOST)),
                ("learn_inputs", number(super::law_learn::INPUTS_MOST)),
                (
                    "law_from_chip_nodes",
                    text(&format!(
                        "a law holds {} nodes; a chip costs up to {} a gate (3 in each of the 8 cells of an oct) plus up to 1600 for its bindings, so {} gates always fit, and more fit when gates are copies or constants; the answer names the nodes used",
                        crate::law::limits::NODES,
                        crate::law::chip::NODES_PER_GATE,
                        crate::law::chip::GATES_SURE
                    )),
                ),
                (
                    "budget_late",
                    text(
                        "an answer can come after its budget by up to one batch (submission_ms), and by up to the time the server takes to snapshot its largest world between calls (reading it back, and without a directory also hashing it); a call is not charged for that wait. Steps run in batches. The server times each batch, by any client, and splits its time into a cost per batch and a cost per frame from pairs of batches at least 1.5 times apart in frames (the cost per batch is the third highest of the last five splits, or the lowest while there are fewer than three). A batch grows past the last one only as far as the last batch's time per frame shows it fits both the time left and the aim; a smaller batch is sized by the split costs, or before any split by the last batch's whole time. A world's first batch is one frame. A step is refused at once only when the world has run at least two batches, both took longer than its budget, and either their time per frame alone, or their split costs together with the world's last two one-frame batches (the newer run in the previous call) say one frame does not fit; after two calls in a row were refused since the last batch the next one runs one frame to measure again. After such a refusal or doubt the world's next call starts with a one-frame batch. Otherwise a step whose first batch does not fit runs one frame to measure; an open or rebuild runs one frame at the start of each journal entry its replay would otherwise stop at. Any of these answers can come after its budget by up to one frame of that world. Work that cannot stop part way (building, copying, saving or reading a world, a snapshot a world owes, and one sparse journal write) is refused at once when it cannot fit the time left by the larger of two figures: the fastest of the last three times the server measured that kind of work (dense and sparse worlds, snapshots to disk and in memory, and files opened and replayed, each apart) at the largest size it measured that is not above this one, and the fastest any tested device has shown per byte. Otherwise it runs to its end, so its answer can come after its budget by as much as it then runs slower than that, as when the device is busy, a world of a new shape compiles its kernels on its first build, or a world is larger than any the server has measured. After 8 such refusals in a row for one kind and size by the server's own measurements, the next call runs to measure again, so work that truly cannot fit runs late at most once in every 9 such calls. Replayed steps and writes are refused up front only by the server's own measurements of them: steps by the fastest of the last three replay speeds, writes like work that cannot stop part way but without the per-byte figure. Without any, the replay runs and stops on its budget, and every replay stopped so adds a measurement",
                    ),
                ),
                (
                    "memory_share",
                    text(&format!(
                        "{}/{} of the device heap unless the server was started with its own",
                        budget::SHARE_NUMERATOR,
                        budget::SHARE_DENOMINATOR
                    )),
                ),
                ("credit_depth", number(crate::credit::DEPTH_MOST)),
                ("credit_pairs", number(crate::credit::PAIRS)),
                ("credit_pairs_most", number(crate::credit::PAIRS_MOST)),
                ("credit_moves", number(crate::credit::MOVES)),
                ("credit_moves_most", number(crate::credit::MOVES_MOST)),
                ("credit_listed", number(crate::credit::LISTED)),
                ("credit_pairs_listed", number(crate::credit::PAIRS_LISTED)),
                ("credit_work", number(crate::credit::WORK)),
                ("credit_work_most", number(crate::credit::WORK_MOST)),
                ("escape_rounds", number(crate::credit::ESCAPE_ROUNDS)),
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
