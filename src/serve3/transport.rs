use std::io::BufRead;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::wire::{Json, map, text};

use super::errors::{Code, Refusal, shown};

pub const LINE_MOST: usize = 1 << 20;

pub const ANSWER_MOST: usize = 1 << 20;

pub const ID_MOST: usize = 256;

pub const DEPTH_MOST: usize = 64;

pub const PROTOCOL: &str = "2024-11-05";

static FAULTS: AtomicU64 = AtomicU64::new(0);

#[must_use]
pub fn faults() -> u64 {
    FAULTS.load(Ordering::SeqCst)
}

pub trait Served {
    fn listed(&self) -> Vec<Json>;

    fn called(&mut self, name: &str, arguments: &Json) -> Option<Json>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    Held(Vec<u8>),
    Long { bytes: usize },
}

pub struct Lines<R> {
    reader: R,
}

impl<R: BufRead> Lines<R> {
    pub const fn new(reader: R) -> Self {
        Self { reader }
    }

    pub fn next_line(&mut self) -> std::io::Result<Option<Line>> {
        let mut held = Vec::new();
        let mut bytes = 0_usize;
        let mut ended = true;
        loop {
            let (used, done) = {
                let chunk = self.reader.fill_buf()?;
                if chunk.is_empty() {
                    break;
                }
                ended = false;
                let (part, done) = match chunk.iter().position(|one| *one == b'\n') {
                    Some(at) => (chunk.get(..at).unwrap_or(&[]), Some(at + 1)),
                    None => (chunk, None),
                };
                bytes = bytes.saturating_add(part.len());
                if bytes <= LINE_MOST {
                    held.extend_from_slice(part);
                } else {
                    held = Vec::new();
                }
                (done.unwrap_or(chunk.len()), done.is_some())
            };
            self.reader.consume(used);
            if done {
                break;
            }
        }
        if ended {
            return Ok(None);
        }
        if bytes > LINE_MOST {
            return Ok(Some(Line::Long { bytes }));
        }
        Ok(Some(Line::Held(held)))
    }
}

#[must_use]
pub fn failed(id: Json, refusal: &Refusal) -> Json {
    map(vec![
        ("jsonrpc", text("2.0")),
        ("id", id),
        (
            "error",
            map(vec![
                ("code", Json::Int(refusal.code.rpc().unwrap_or(-32603))),
                ("message", text(&refusal.message)),
                ("data", refusal.data()),
            ]),
        ),
    ])
}

#[must_use]
pub fn answered(id: Json, result: Json) -> Json {
    map(vec![
        ("jsonrpc", text("2.0")),
        ("id", id),
        ("result", result),
    ])
}

#[must_use]
pub fn too_long(bytes: usize) -> Json {
    failed(
        Json::Null,
        &Refusal::new(
            Code::LineTooLong,
            "$",
            format!("the line holds {bytes} bytes, more than the {LINE_MOST} a line may hold"),
        )
        .limited(Json::Int(i64::try_from(LINE_MOST).unwrap_or(i64::MAX))),
    )
}

fn blank(line: &str) -> bool {
    line.bytes()
        .all(|one| matches!(one, b' ' | b'\t' | b'\n' | b'\r'))
}

fn parse_failed(message: String) -> Json {
    failed(Json::Null, &Refusal::new(Code::Parse, "$", message))
}

fn torn(line: &str, why: crate::wire::Torn) -> String {
    if let crate::wire::Torn::Stray { at, saw: 'u' } = why
        && let Some(escape) = line.get(at..at + 6)
        && escape.starts_with("\\u")
        && u32::from_str_radix(escape.get(2..).unwrap_or(""), 16)
            .is_ok_and(|unit| (0xD800..=0xDFFF).contains(&unit))
    {
        return format!(
            "the line is not JSON this server reads: {escape} at byte {at} is half of a UTF-16 surrogate pair with no other half, which is no character"
        );
    }
    format!("the line is not JSON this server reads: {why}")
}

#[must_use]
pub fn not_utf8(bytes: usize, at: usize) -> Json {
    parse_failed(format!(
        "the line of {bytes} bytes is not UTF-8 from byte {at}"
    ))
}

pub fn heard(served: &mut dyn Served, line: &str) -> Option<Json> {
    if line.len() > LINE_MOST {
        return Some(too_long(line.len()));
    }
    if blank(line) {
        return None;
    }
    let request = match crate::wire::read(line) {
        Ok(request) => request,
        Err(why) => return Some(parse_failed(torn(line, why))),
    };
    handle(served, &request)
}

fn invalid(id: Json, field: &str, message: String) -> Option<Json> {
    Some(failed(
        id,
        &Refusal::new(Code::InvalidRequest, field, message),
    ))
}

fn usable(id: &Json) -> bool {
    match id {
        Json::Null | Json::Int(_) => true,
        Json::Text(held) => held.len() <= ID_MOST,
        _ => false,
    }
}

pub fn handle(served: &mut dyn Served, request: &Json) -> Option<Json> {
    let Json::Map(held) = request else {
        return invalid(
            Json::Null,
            "$",
            "the line is JSON but not an object; send one request object per line".to_owned(),
        );
    };
    for key in ["jsonrpc", "id", "method", "params"] {
        if held.iter().filter(|(name, _)| name == key).count() > 1 {
            return invalid(
                Json::Null,
                &format!("$.{key}"),
                format!("the request names {key} more than once"),
            );
        }
    }
    let id = request.get("id");
    if let Some(id) = id
        && !usable(id)
    {
        return invalid(
            Json::Null,
            "$.id",
            format!(
                "the id {} is not a whole number, null, or a string of at most {ID_MOST} bytes",
                shown(&crate::wire::written(id))
            ),
        );
    }
    let Some(way) = request.get("method").and_then(Json::text) else {
        return invalid(
            id.cloned().unwrap_or(Json::Null),
            "$.method",
            "the request has no method that is a string".to_owned(),
        );
    };
    let id = id.cloned()?;
    let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        respond(served, way, request.get("params"), id.clone())
    }));
    let answer = answer.unwrap_or_else(|_| {
        FAULTS.fetch_add(1, Ordering::SeqCst);
        failed(
            id.clone(),
            &Refusal::new(
                Code::Fault,
                "$",
                "the server hit a fault while answering this request and has no answer for it"
                    .to_owned(),
            ),
        )
    });
    Some(bounded(id, way, answer))
}

fn bounded(id: Json, way: &str, answer: Json) -> Json {
    let bytes = crate::wire::written(&answer).len();
    if bytes <= ANSWER_MOST {
        return answer;
    }
    let refusal = Refusal::new(
        Code::AnswerTooLarge,
        "$",
        format!(
            "the answer would hold {bytes} bytes, more than the {ANSWER_MOST} one answer may hold"
        ),
    )
    .limited(Json::Int(i64::try_from(ANSWER_MOST).unwrap_or(i64::MAX)));
    if way == "tools/call" {
        return answered(id, refused(&refusal, Vec::new()));
    }
    failed(id, &refusal)
}

#[must_use]
pub fn refused(refusal: &Refusal, more: Vec<(&'static str, Json)>) -> Json {
    let mut told = vec![
        ("refused", text(refusal.code.name())),
        ("message", text(&refusal.message)),
    ];
    told.extend(refusal.told());
    told.extend(more);
    map(vec![
        (
            "content",
            Json::List(vec![map(vec![
                ("type", text("text")),
                ("text", text(&refusal.message)),
            ])]),
        ),
        ("isError", Json::Bool(true)),
        ("structuredContent", map(told)),
    ])
}

fn initialized() -> Json {
    map(vec![
        ("protocolVersion", text(PROTOCOL)),
        ("capabilities", map(vec![("tools", map(vec![]))])),
        (
            "serverInfo",
            map(vec![("name", text("grit")), ("version", text("0.0.0"))]),
        ),
    ])
}

fn unknown_tool(id: Json, field: &str, message: String) -> Json {
    failed(id, &Refusal::new(Code::UnknownTool, field, message))
}

fn respond(served: &mut dyn Served, way: &str, params: Option<&Json>, id: Json) -> Json {
    match way {
        "initialize" => answered(id, initialized()),
        "ping" => answered(id, map(vec![])),
        "tools/list" => answered(id, map(vec![("tools", Json::List(served.listed()))])),
        "tools/call" => {
            let Some(Json::Map(_)) = params else {
                return unknown_tool(
                    id,
                    "$.params",
                    "tools/call needs params, an object that names the tool".to_owned(),
                );
            };
            let Some(name) = params.and_then(|one| one.get("name")).and_then(Json::text) else {
                return unknown_tool(
                    id,
                    "$.params.name",
                    "tools/call needs params.name, the name of a tool".to_owned(),
                );
            };
            let arguments = params
                .and_then(|one| one.get("arguments"))
                .cloned()
                .unwrap_or(Json::Map(Vec::new()));
            match served.called(name, &arguments) {
                Some(result) => answered(id, result),
                None => unknown_tool(
                    id,
                    "$.params.name",
                    format!(
                        "no tool is named {}; tools/list names them",
                        shown(&crate::wire::written(&text(name)))
                    ),
                ),
            }
        }
        other => failed(
            id,
            &Refusal::new(Code::UnknownMethod, "$.method", shown(other)),
        ),
    }
}
