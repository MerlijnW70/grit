use std::collections::BTreeMap;

use crate::bricks::{Bricks, Content};
use crate::compact::Plan;
use crate::layout::{Dimension, PARAMS, TALLY};
use crate::live;
use crate::resident::Groups;
use crate::rule3::FIRST_PARAM;
use crate::sand::Grid;
use crate::sand3::Grid3;
use crate::sweep;
use crate::wire::{Json, counted, map, text};

use super::errors::{self, Code, Refusal};
use super::schema::{choice, object, property, whole, with, word};
use super::transport::{ANSWER_MOST, DEPTH_MOST, ID_MOST, LINE_MOST, PROTOCOL};
use super::{
    BATCH, DIGITS, Failed, PAGE_BYTES, POOL_LEAST_CELLS, READ_MOST, SLICE_MOST, STEP_MOST, Server,
    Slot, Tool, World, rules, rules_on,
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

fn refused(code: Code, field: &str, message: String) -> Failed {
    Box::new(Refusal::new(code, field, message))
}

fn limited(code: Code, field: &str, message: String, limit: Json) -> Failed {
    Box::new(Refusal::new(code, field, message).limited(limit))
}

fn unread(key: &str) -> Failed {
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

fn words_of<'held>(held: &'held Json, key: &str) -> Result<&'held str, Failed> {
    held.get(key)
        .and_then(Json::text)
        .ok_or_else(|| unread(key))
}

fn sized(held: [u32; 3]) -> Json {
    Json::List(held.iter().map(|one| counted(*one)).collect())
}

fn largest(size: [u32; 3]) -> Json {
    sized(size.map(|side| side.saturating_sub(1)))
}

fn inside(world: &World<'_>, at: [u32; 3], what: &str, field: &str) -> Result<(), Failed> {
    if at.iter().zip(world.size()).all(|(one, side)| *one < side) {
        return Ok(());
    }
    let size = world.size();
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

fn world_number() -> Json {
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

fn tool(name: &str, what: &str, properties: Vec<(&str, Json)>, required: &[&str]) -> Json {
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
    let grid = Grid::solid(wide, deep, tall)?;
    let width = host.width();
    let blocks = grid.blocks().div_ceil(sweep::cells(width));
    let plan = if sparse {
        Plan::bricked(width, blocks)?
    } else {
        Plan::over_in(width, blocks, Dimension::Three)?
    };
    let pool = if sparse {
        let most = u32::try_from(((1_u64 << 32) - 1) / (u64::from(plan.per_block) * 8))
            .unwrap_or(u32::MAX)
            .min(plan.blocks);
        let brick_cells = u64::from(plan.per_block) * 8;
        let least = u32::try_from(POOL_LEAST_CELLS.div_ceil(brick_cells)).unwrap_or(u32::MAX);
        let capacity = slots.unwrap_or((plan.blocks / 4).max(least)).clamp(1, most);
        Some(Bricks::new(grid, &plan, capacity)?)
    } else {
        None
    };
    let ((passes, stages), sizes) = match pool.as_ref() {
        Some(bricks) => (
            rule.bricked(bricks, &plan)?,
            rule.bricked_sizes(bricks, &plan),
        ),
        None => (rule.wired(width, grid, &plan)?, rule.sizes(&plan).to_vec()),
    };
    let solid = Grid3::new(grid.wide, grid.deep, grid.tall).ok_or_else(|| {
        refused(
            Code::Unfit,
            "$",
            format!("a world of {wide} by {deep} by {tall} cells is not solid"),
        )
    })?;
    if pool.is_some() {
        host.limits().admits_on(&sizes, None, host.heap())?;
    }
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let mut live = host.resident(&sizes, &modules)?;
    for (binding, room) in sizes.iter().enumerate() {
        live.write(binding, &vec![0_u32; *room])?;
    }
    if !params.is_empty() {
        live.write_at(PARAMS as usize, FIRST_PARAM, &params)?;
    }
    match pool.as_ref() {
        Some(bricks) => {
            live.write(PARAMS as usize, &[u32::MAX])?;
            let packed = bricks.compose(|_| Content::Uniform(0))?;
            bricks.write_packed(&mut live, &plan, &packed)?;
            bricks.seed(&mut live, &plan)?;
        }
        None => {
            live.write(PARAMS as usize, &[u32::MAX, plan.dump()])?;
            live::seed(&mut live, grid, &plan)?;
        }
    }

    let mut world = World {
        rule,
        live,
        stages,
        grid,
        solid,
        plan,
        frame: 0,
        pool,
    };
    let listed = world.listed()?;
    let id = server.next;
    server.next = id.checked_add(1).ok_or_else(|| {
        limited(
            Code::TooMany,
            "$",
            "every world number has been used".to_owned(),
            counted(u32::MAX),
        )
    })?;
    let answer = world.reported(vec![
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
    ])?;
    server.worlds.insert(id, Slot::Open(Box::new(world)));
    Ok(answer)
}

fn written(server: &mut Server<'_>, held: &Json) -> Result<Json, Failed> {
    let number = int(held, "world")?;
    let world = server.world(held)?;
    let list = held
        .get("cells")
        .and_then(Json::list)
        .ok_or_else(|| unread("cells"))?;
    let mut cells = Vec::with_capacity(list.len());
    let mut points = Vec::with_capacity(list.len());
    for (at, one) in list.iter().enumerate() {
        let got = ints(one, "cells")?;
        let [x, y, z, word] = <[u32; 4]>::try_from(got.as_slice()).map_err(|_| unread("cells"))?;
        inside(
            world,
            [x, y, z],
            &format!("cell {at} at"),
            &format!("$.cells[{at}]"),
        )?;
        points.push(([x, y, z], word));
        if world.pool.is_none() {
            cells.push((u32::try_from(world.index([x, y, z]))?, word));
        }
    }
    match world.pool.as_ref() {
        Some(bricks) => bricks.put_at(&mut world.live, &points)?,
        None => live::put3(&mut world.live, world.grid, &world.plan, &cells)?,
    }
    let listed = world.listed()?;
    world.reported(vec![
        ("world", counted(number)),
        ("written", counted(u32::try_from(points.len())?)),
        ("frame", counted(world.frame)),
        ("listed", counted(listed)),
    ])
}

fn stepped(server: &mut Server<'_>, held: &Json) -> Result<Json, Failed> {
    let number = int(held, "world")?;
    let world = server.world(held)?;
    let frames = int(held, "frames")?;
    let groups: Vec<&[Groups]> = world.stages.iter().map(Vec::as_slice).collect();
    let mut done = 0_u32;
    while done < frames {
        let step = BATCH.min(frames - done);
        world.live.staged_frames(&groups, step)?;
        done += step;
    }
    world.frame = world.frame.wrapping_add(frames);
    let tally = world.live.read(TALLY as usize, world.rule.tally_words())?;
    let listed = world.listed()?;
    world.reported(vec![
        ("world", counted(number)),
        ("frames", counted(frames)),
        ("frame", counted(world.frame)),
        (
            "tally",
            Json::List(tally.iter().map(|one| counted(*one)).collect()),
        ),
        ("listed", counted(listed)),
    ])
}

fn read(server: &mut Server<'_>, held: &Json) -> Result<Json, Failed> {
    let number = int(held, "world")?;
    let world = server.world(held)?;
    let from = corner(held, "from")?;
    let to = corner(held, "to")?;
    inside(world, from, "from", "$.from")?;
    inside(world, to, "to", "$.to")?;
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
    for word in world.cells_at(&points)? {
        *kinds.entry(world.rule.kind(word)).or_insert(0_u32) += 1;
        words.push(counted(word));
    }
    Ok(map(vec![
        ("world", counted(number)),
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

fn sliced(server: &mut Server<'_>, held: &Json) -> Result<Json, Failed> {
    let number = int(held, "world")?;
    let world = server.world(held)?;
    let axis = words_of(held, "axis")?;
    let (across, down, through) = match axis {
        "x" => (1, 2, 0),
        "y" => (0, 2, 1),
        _ => (0, 1, 2),
    };
    let at = int(held, "at")?;
    let size = world.size();
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
    let state = world.cells_at(&points)?;
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
        ("frame", counted(world.frame)),
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
    server.world(held)?;
    server.worlds.insert(world, Slot::Closed);
    Ok(map(vec![
        ("world", counted(world)),
        ("closed", Json::Bool(true)),
    ]))
}

fn number(held: impl TryInto<i64>) -> Json {
    held.try_into().map_or(Json::Null, Json::Int)
}

fn described(server: &mut Server<'_>, _: &Json) -> Result<Json, Failed> {
    let tools: Vec<Json> = super::transport::Served::listed(&*server)
        .iter()
        .filter_map(|one| one.get("name").cloned())
        .collect();
    let device = server.host.map_or(Json::Null, |host| {
        map(vec![
            ("name", text(host.name())),
            ("subgroup", counted(host.width())),
            ("heap_bytes", number(host.heap())),
        ])
    });
    Ok(map(vec![
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
            ]),
        ),
        ("device", device),
        ("faults_caught", number(super::transport::faults())),
        (
            "features",
            map(vec![
                ("window", Json::Bool(cfg!(feature = "window"))),
                ("deep", Json::Bool(cfg!(feature = "deep"))),
                ("faults", Json::Bool(cfg!(feature = "faults"))),
            ]),
        ),
    ]))
}
