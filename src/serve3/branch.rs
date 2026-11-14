use std::sync::Arc;
use std::time::Instant;

use crate::branch::format;
use crate::wire::{Json, counted, map, text};

use super::budget::{Short, Work, counted64};
use super::errors::Code;
use super::journal::{self, Keep, Ledger, Made, Snap, hex, regular, tail_sum};
use super::schema::{property, whole, with};
use super::tools::{
    admitted, first_pace, int, limited, refused, reported, sized, tool, unread, words_of,
    world_number,
};
use super::{Failed, Live, PAGE_BYTES, Server, Tool, image_shaped, shaped};

pub const TOOLS: &[Tool] = &[
    Tool {
        name: "grit_world_save",
        shown: save_shown,
        run: saved,
    },
    Tool {
        name: "grit_world_open",
        shown: open_shown,
        run: opened,
    },
    Tool {
        name: "grit_world_list",
        shown: list_shown,
        run: listed,
    },
    Tool {
        name: "grit_world_fork",
        shown: fork_shown,
        run: forked,
    },
    Tool {
        name: "grit_world_restore",
        shown: restore_shown,
        run: restored,
    },
];

fn name_shown(what: &str) -> Json {
    property(
        "string",
        &format!(
            "{what}: 1 to {} of a-z, 0-9, _ and -, starting with a letter or digit; the file is the name with .grit in the server directory",
            journal::NAME_MOST
        ),
    )
}

fn save_shown() -> Json {
    tool(
        "grit_world_save",
        "Write a world to a file in the server directory, with its checksum and a hash of its state.",
        vec![
            ("world", world_number()),
            ("name", name_shown("the file's name")),
            (
                "replace",
                with(
                    property(
                        "boolean",
                        "write over a file of that name; false if left out",
                    ),
                    vec![("default", Json::Bool(false))],
                ),
            ),
        ],
        &["world", "name"],
    )
}

fn open_shown() -> Json {
    tool(
        "grit_world_open",
        "Open a world from a file grit_world_save wrote, or a world kept from an earlier run of the server, named world- and its number, as grit_world_list shows it.",
        vec![(
            "name",
            name_shown("a file or kept world grit_world_list shows"),
        )],
        &["name"],
    )
}

fn list_shown() -> Json {
    tool(
        "grit_world_list",
        "List the open worlds, the worlds kept from an earlier run, and the saved files of the server directory.",
        vec![(
            "cursor",
            with(
                whole(
                    "the item the answer starts at; 0 if left out. An answer that stops early names the item to ask for next as next",
                    0,
                    i64::from(u32::MAX),
                ),
                vec![("default", Json::Int(0))],
            ),
        )],
        &[],
    )
}

fn fork_shown() -> Json {
    tool(
        "grit_world_fork",
        "Copy a world into a new one at the same frame; stepping both gives the same cells. Returns the new world's number.",
        vec![("world", world_number())],
        &["world"],
    )
}

fn restore_shown() -> Json {
    tool(
        "grit_world_restore",
        "Give a world the cells, frame and params of another of the same rule, size and layout, such as a fork taken earlier.",
        vec![
            ("world", world_number()),
            (
                "from",
                whole("the world whose state it takes", 1, i64::from(u32::MAX)),
            ),
        ],
        &["world", "from"],
    )
}

fn keep_of(server: &Server<'_>, field: &str) -> Result<Keep, Failed> {
    server.book.keep.clone().ok_or_else(|| {
        refused(
            Code::Path,
            field,
            "this server was started without a directory, so it keeps no files".to_owned(),
        )
    })
}

fn file_refusal(why: &crate::branch::Refused, name: &str) -> Failed {
    let mut refusal = super::branch_refusal(why);
    refusal.field = "$.name".to_owned();
    refusal.message = format!("the file {name}{} {}", journal::SUFFIX, refusal.message);
    Box::new(refusal)
}

fn saved(server: &mut Server<'_>, held: &Json) -> Result<Json, Failed> {
    let number = int(held, "world")?;
    let name = words_of(held, "name")?.to_owned();
    journal::file_name(&name, "$.name")?;
    let keep = keep_of(server, "$.name")?;
    let path = keep.saved(&name, "$.name")?;
    if kept_number(&name).is_some() {
        return Err(refused(
            Code::Path,
            "$.name",
            format!("names {KEPT_PREFIX} and a number belong to kept worlds; choose another name"),
        ));
    }
    let replace = matches!(held.get("replace"), Some(Json::Bool(true)));
    if regular(&path, "$.name")?.is_some() && !replace {
        return Err(refused(
            Code::Exists,
            "$.name",
            format!("a file named {name} is already in the server directory"),
        ));
    }
    server.world(held)?;
    let size = server.book.held.get(&number).map_or(0, |held| held.bytes);
    server.fits(
        server.book.rates.estimate(Work::Save, size),
        &format!("writing world {number} to a file"),
    )?;
    let began = Instant::now();
    let live = server.world(held)?;
    let image = live.world.image()?;
    let bytes = format::store(&path, &image).map_err(|why| file_refusal(&why, &name))?;
    server.book.rates.measure(Work::Save, size, began.elapsed());
    let sum = tail_sum(&path).map_err(|why| super::io_refusal(&why))?;
    Ok(map(vec![
        ("world", counted(number)),
        ("name", text(&name)),
        ("frame", counted(image.frame)),
        ("bytes", counted64(bytes)),
        ("checksum", text(&hex(&sum))),
        ("state", text(&hex(&journal::state_of(&image)))),
    ]))
}

pub const KEPT_PREFIX: &str = "world-";

#[must_use]
pub fn kept_number(name: &str) -> Option<u32> {
    let digits = name.strip_prefix(KEPT_PREFIX)?;
    let number = digits.parse::<u32>().ok()?;
    (number > 0 && number.to_string() == digits).then_some(number)
}

#[must_use]
pub fn kept_name(number: u32) -> String {
    format!("{KEPT_PREFIX}{number}")
}

fn opened(server: &mut Server<'_>, held: &Json) -> Result<Json, Failed> {
    let name = words_of(held, "name")?.to_owned();
    match kept_number(&name) {
        Some(number) => opened_kept(server, number),
        None => opened_file(server, &name),
    }
}

fn opened_file(server: &mut Server<'_>, name: &str) -> Result<Json, Failed> {
    journal::file_name(name, "$.name")?;
    let keep = keep_of(server, "$.name")?;
    let path = keep.saved(name, "$.name")?;
    let Some(length) = regular(&path, "$.name")? else {
        return Err(refused(
            Code::NoFile,
            "$.name",
            format!("no file named {name} is in the server directory"),
        ));
    };
    let host = server.host.ok_or(crate::host::Error::NoComputeDevice)?;
    let allowed = server.book.memory.allowed();
    if length > allowed {
        return Err(Box::new(Short {
            asked: length,
            held: server.book.memory.held(),
            allowed,
            what: format!("the file {name}{}", journal::SUFFIX),
        }));
    }
    server.fits(
        server.book.rates.estimate(Work::File, length),
        &format!("reading the file {name}{}", journal::SUFFIX),
    )?;
    let began = Instant::now();
    let pending = keep
        .copied(&path, server.book.next)
        .map_err(|why| super::io_refusal(&why))?;
    let image = match format::fetch(&pending) {
        Ok(image) => image,
        Err(why) => {
            let _ = std::fs::remove_file(&pending);
            return Err(file_refusal(&why, name));
        }
    };
    server
        .book
        .rates
        .measure(Work::File, length, began.elapsed());
    let (sum, _) = keep
        .settle(&pending)
        .map_err(|why| super::io_refusal(&why))?;
    let law = super::law::image_text(&image);
    let opened = (|| {
        let planned = super::law::planned(host, image.law.name(), law.as_deref(), &image.params)?;
        let sizes = image_shaped(host, planned.as_ref(), &image)?;
        let bytes = admitted(server, host, &sizes, "the world")?;
        server.fits(
            server.book.rates.estimate(Work::Build, bytes),
            "building the world from the file",
        )?;
        let began = Instant::now();
        let world = super::law::opened(host, &image)?;
        server
            .book
            .rates
            .measure(Work::Build, bytes, began.elapsed());
        let rule = super::law::listed(host, &world)?;
        if let Some(lawful) = rule.lawful()
            && world.pool().is_some()
            && !lawful.class().proved()
        {
            return Err(super::law::quiet_refused(
                lawful.law(),
                lawful.class(),
                "$.name",
            ));
        }
        Ok::<_, Failed>(Live { world, rule })
    })();
    let mut live = match opened {
        Ok(opened) => opened,
        Err(why) => {
            server.book.release(Some(sum));
            return Err(why);
        }
    };
    let id = server.number()?;
    let bytes = super::world_bytes(host, &live);
    let pace = first_pace(&live.world);
    let listed = live.world.listed()?;
    let mut answer = vec![
        ("world", counted(id)),
        ("name", text(name)),
        ("rule", text(live.rule.name())),
        (
            "params",
            Json::List(image.params.iter().map(|one| counted(*one)).collect()),
        ),
        ("size", sized(live.world.size())),
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
    let ledger = Ledger::snapped(Arc::new(Snap {
        sum,
        frame: image.frame,
        made: Made::of_image(&image),
        image: None,
    }))
    .with_law(law);
    if let Some(note) = server.adopt(id, live, ledger, bytes, pace) {
        answer.push(("journal", note));
    }
    Ok(map(answer))
}

fn opened_kept(server: &mut Server<'_>, number: u32) -> Result<Json, Failed> {
    if let Some(why) = server.book.broken_earlier.get(&number) {
        return Err(refused(
            Code::BadFile,
            "$.name",
            format!(
                "world {number} cannot be rebuilt ({}); close it to remove its files",
                super::errors::shown(why)
            ),
        ));
    }
    let Some(ledger) = server.book.earlier.get(&number).cloned() else {
        if server.live.contains_key(&number) {
            return Err(refused(
                Code::Exists,
                "$.name",
                format!("world {number} is open already"),
            ));
        }
        return Err(refused(
            Code::NoFile,
            "$.name",
            format!("no world {number} is kept from an earlier run of the server"),
        ));
    };
    let host = server.host.ok_or(crate::host::Error::NoComputeDevice)?;
    let planned = super::law::planned(
        host,
        &ledger.made().rule,
        ledger.law.as_deref(),
        &ledger.made().params,
    )?;
    let (_, sizes) = shaped(host, planned.as_ref(), ledger.made())?;
    let bytes = admitted(server, host, &sizes, "the world")?;
    server.fits(
        server.replay_estimate(&ledger, bytes),
        &format!("rebuilding world {number} from its journal"),
    )?;
    let mut pace = super::budget::Pace::first(0);
    let world = match server.replayed_world(host, &ledger, &mut pace, bytes) {
        Ok(world) => world,
        Err(why) => {
            let stopped = why.downcast_ref::<super::budget::Halted>().is_some();
            if !stopped && !super::lost_device(why.as_ref()) && !server.wedged() {
                let said = super::refused_with(why.as_ref()).0.message;
                server.book.broken_earlier.insert(number, said);
            }
            return Err(why);
        }
    };
    let rule = super::law::listed(host, &world)?;
    let mut live = Live { world, rule };
    let bytes = super::world_bytes(host, &live);
    let replayed = u32::try_from(ledger.entries.len()).unwrap_or(u32::MAX);
    let listed = live.world.listed()?;
    let mut answer = vec![
        ("world", counted(number)),
        ("name", text(&kept_name(number))),
        ("rule", text(live.rule.name())),
        ("size", sized(live.world.size())),
        ("frame", counted(live.world.frame())),
        ("listed", counted(listed)),
        ("replayed", counted(replayed)),
    ];
    if let Some(lawful) = live.rule.lawful() {
        answer.extend(lawful.told());
    }
    if let Some(used) = live.world.pool_use()? {
        answer.push(("sparse", Json::Bool(true)));
        answer.push(("pool_use", super::pooled(used)));
    }
    server.book.earlier.remove(&number);
    if let Some(note) = server.adopt(number, live, ledger, bytes, pace) {
        answer.push(("journal", note));
    }
    Ok(map(answer))
}

fn shown_made(ledger: &Ledger) -> Vec<(&'static str, Json)> {
    let made = ledger.made();
    let mut shown = vec![("rule", text(&made.rule))];
    if let Some(law) = &ledger.law {
        shown.push(("law", text(&super::law::hash_of(law))));
    }
    shown.extend([
        ("size", sized(made.size)),
        ("sparse", Json::Bool(made.sparse)),
        ("frame", counted(ledger.frame)),
    ]);
    shown
}

fn listed(server: &mut Server<'_>, held: &Json) -> Result<Json, Failed> {
    let cursor = int(held, "cursor")? as usize;
    let mut items = Vec::new();
    for (number, one) in &server.book.held {
        let state = if server.live.contains_key(number) {
            "open"
        } else if one.broken.is_some() {
            "broken"
        } else {
            "lost"
        };
        let mut item = vec![("kind", text(state)), ("world", counted(*number))];
        item.extend(shown_made(&one.ledger));
        if let Some(lawful) = server.live.get(number).and_then(|live| live.rule.lawful()) {
            item.extend(lawful.told().into_iter().filter(|(key, _)| *key != "law"));
        }
        item.push(("bytes", counted64(one.bytes)));
        if let Some(failure) = &one.snap_failure {
            item.push((
                "snapshot_failed",
                map(vec![
                    ("message", text(&failure.message)),
                    ("failures", counted(failure.failures)),
                ]),
            ));
        }
        items.push(item);
    }
    for (number, ledger) in &server.book.earlier {
        let kind = if server.book.broken_earlier.contains_key(number) {
            "broken"
        } else {
            "kept"
        };
        let mut item = vec![
            ("kind", text(kind)),
            ("world", counted(*number)),
            ("name", text(&kept_name(*number))),
        ];
        item.extend(shown_made(ledger));
        items.push(item);
    }
    if let Some(keep) = server.book.keep.as_ref() {
        for (name, bytes) in keep.listed().map_err(|why| super::io_refusal(&why))? {
            items.push(vec![
                ("kind", text("saved")),
                ("name", text(&name)),
                ("bytes", counted64(bytes)),
            ]);
        }
    }
    let total = items.len();
    if cursor > total {
        return Err(limited(
            Code::Outside,
            "$.cursor",
            format!("item {cursor} is past the {total} items listed"),
            counted(u32::try_from(total).unwrap_or(u32::MAX)),
        ));
    }
    let mut page = Vec::new();
    let mut bytes = 0_usize;
    let mut next = None;
    for (at, item) in items.into_iter().enumerate().skip(cursor) {
        let one = map(item);
        let raw = crate::wire::written(&one);
        bytes += raw.len() + crate::wire::written(&text(&raw)).len();
        if bytes > PAGE_BYTES * 2 && !page.is_empty() {
            next = Some(at);
            break;
        }
        page.push(one);
    }
    let mut answer = vec![
        ("items", Json::List(page)),
        ("total", counted(u32::try_from(total).unwrap_or(u32::MAX))),
    ];
    if let Some(next) = next {
        answer.push(("next", counted(u32::try_from(next).unwrap_or(u32::MAX))));
    }
    Ok(map(answer))
}

fn forked(server: &mut Server<'_>, held: &Json) -> Result<Json, Failed> {
    let number = int(held, "world")?;
    server.world(held)?;
    let host = server.host.ok_or(crate::host::Error::NoComputeDevice)?;
    let source = server
        .book
        .held
        .get(&number)
        .cloned()
        .ok_or_else(|| unread("world"))?;
    server.book.memory.admits(source.bytes, "the fork")?;
    server.fits(
        server.book.rates.estimate(Work::Fork, source.bytes),
        &format!("copying world {number}"),
    )?;
    let began = Instant::now();
    let live = server.world(held)?;
    let fork = live.world.fork()?;
    server
        .book
        .rates
        .measure(Work::Fork, source.bytes, began.elapsed());
    let rule = super::law::listed(host, &fork)?;
    let id = server.number()?;
    let mut live = Live { world: fork, rule };
    let listed = live.world.listed()?;
    let mut answer = vec![
        ("world", counted(id)),
        ("from", counted(number)),
        ("rule", text(live.rule.name())),
        ("size", sized(live.world.size())),
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
    if let Some(note) = server.adopt(id, live, source.ledger, source.bytes, source.pace) {
        answer.push(("journal", note));
    }
    Ok(map(answer))
}

fn restored(server: &mut Server<'_>, held: &Json) -> Result<Json, Failed> {
    let number = int(held, "world")?;
    let from = int(held, "from")?;
    server.world(held)?;
    server.world_at(from, "$.from")?;
    if number == from {
        let live = server.world(held)?;
        let listed = live.world.listed()?;
        let frame = live.world.frame();
        return reported(
            live,
            vec![
                ("world", counted(number)),
                ("from", counted(from)),
                ("frame", counted(frame)),
                ("listed", counted(listed)),
            ],
        );
    }
    let size = server.book.held.get(&from).map_or(0, |held| held.bytes);
    server.fits(
        server.book.rates.estimate(Work::Fork, size),
        &format!("copying world {from} into world {number}"),
    )?;
    let began = Instant::now();
    let mut source = server.live.remove(&from).ok_or_else(|| unread("from"))?;
    let done = server
        .live
        .get_mut(&number)
        .ok_or_else(|| unread("world"))
        .and_then(|target| target.world.restore(&mut source.world));
    server.live.insert(from, source);
    done?;
    server.book.rates.measure(Work::Fork, size, began.elapsed());
    let taken = server
        .book
        .held
        .get(&from)
        .map(|held| (held.ledger.clone(), held.pace))
        .ok_or_else(|| unread("from"))?;
    let old = server
        .book
        .held
        .get(&number)
        .and_then(|held| held.ledger.sum());
    let failed = server
        .book
        .keep
        .as_ref()
        .and_then(|keep| keep.begin(number, &taken.0).err());
    let epoch = server.book.bump();
    if let Some(held) = server.book.held.get_mut(&number) {
        held.ledger = taken.0;
        held.pace = taken.1;
        held.epoch = epoch;
    }
    server.book.release(old);
    let host = server.host.ok_or(crate::host::Error::NoComputeDevice)?;
    let live = server.world(held)?;
    live.rule = super::law::listed(host, &live.world)?;
    let listed = live.world.listed()?;
    let mut answer = vec![
        ("world", counted(number)),
        ("from", counted(from)),
        ("frame", counted(live.world.frame())),
        ("listed", counted(listed)),
    ];
    if let Some(lawful) = live.rule.lawful() {
        answer.extend(lawful.told());
    }
    if failed.is_some() {
        answer.push((
            "journal",
            text("the journal could not be written; the world is kept in memory only"),
        ));
    }
    reported(live, answer)
}
