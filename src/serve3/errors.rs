use simdr::lanes::LaneError;

use crate::wire::{Json, map, text};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Code {
    Parse,
    LineTooLong,
    InvalidRequest,
    UnknownMethod,
    UnknownTool,
    Fault,
    AnswerTooLarge,
    Malformed,
    UnknownKey,
    DuplicateKey,
    TooMany,
    Outside,
    UnknownRule,
    Params,
    Closed,
    NoWorld,
    Reversed,
    Axis,
    Frames,
    Unfit,
    TooBig,
    TooLarge,
    TooManyGroups,
    Lane,
    PoolFull,
    LostWrite,
    NoComputeDevice,
    Device,
    Error,
}

pub const CODES: [Code; 29] = [
    Code::Parse,
    Code::LineTooLong,
    Code::InvalidRequest,
    Code::UnknownMethod,
    Code::UnknownTool,
    Code::Fault,
    Code::AnswerTooLarge,
    Code::Malformed,
    Code::UnknownKey,
    Code::DuplicateKey,
    Code::TooMany,
    Code::Outside,
    Code::UnknownRule,
    Code::Params,
    Code::Closed,
    Code::NoWorld,
    Code::Reversed,
    Code::Axis,
    Code::Frames,
    Code::Unfit,
    Code::TooBig,
    Code::TooLarge,
    Code::TooManyGroups,
    Code::Lane,
    Code::PoolFull,
    Code::LostWrite,
    Code::NoComputeDevice,
    Code::Device,
    Code::Error,
];

impl Code {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Parse => "parse",
            Self::LineTooLong => "line_too_long",
            Self::InvalidRequest => "invalid_request",
            Self::UnknownMethod => "unknown_method",
            Self::UnknownTool => "unknown_tool",
            Self::Fault => "fault",
            Self::AnswerTooLarge => "answer_too_large",
            Self::Malformed => "malformed",
            Self::UnknownKey => "unknown_key",
            Self::DuplicateKey => "duplicate_key",
            Self::TooMany => "too_many",
            Self::Outside => "outside",
            Self::UnknownRule => "unknown_rule",
            Self::Params => "params",
            Self::Closed => "closed",
            Self::NoWorld => "no_world",
            Self::Reversed => "reversed",
            Self::Axis => "axis",
            Self::Frames => "frames",
            Self::Unfit => "unfit",
            Self::TooBig => "too_big",
            Self::TooLarge => "too_large",
            Self::TooManyGroups => "too_many_groups",
            Self::Lane => "lane",
            Self::PoolFull => "pool_full",
            Self::LostWrite => "lost_write",
            Self::NoComputeDevice => "no_compute_device",
            Self::Device => "device",
            Self::Error => "error",
        }
    }

    #[must_use]
    pub const fn rpc(self) -> Option<i64> {
        match self {
            Self::Parse => Some(-32700),
            Self::LineTooLong | Self::InvalidRequest => Some(-32600),
            Self::UnknownMethod => Some(-32601),
            Self::UnknownTool => Some(-32602),
            Self::Fault => Some(-32603),
            _ => None,
        }
    }

    #[must_use]
    pub const fn hint(self) -> &'static str {
        match self {
            Self::Parse => {
                "Send one JSON-RPC 2.0 object per line, with whole numbers only and nothing after it."
            }
            Self::LineTooLong => {
                "Send lines of at most the limit in bytes; split a large write over several calls."
            }
            Self::InvalidRequest => {
                "Send one JSON object per line with a string method and an id that is a whole number, null, or a string of at most 256 bytes."
            }
            Self::UnknownMethod => "Use initialize, ping, tools/list or tools/call.",
            Self::UnknownTool => "Call a tool by a name tools/list gives.",
            Self::Fault => {
                "The server failed on this request and is still serving; report the line that caused it."
            }
            Self::AnswerTooLarge => {
                "Ask for less in one call: a smaller box, fewer items, or the next page through cursor."
            }
            Self::Malformed => {
                "Give the field the type, range and length its schema in tools/list names."
            }
            Self::UnknownKey => {
                "Remove the field or correct its name to one the tool's schema in tools/list names."
            }
            Self::DuplicateKey => "Give each field once.",
            Self::TooMany => {
                "Ask for at most the limit in one call and spread the rest over more calls."
            }
            Self::Outside => {
                "Name a place inside the world; its size is in the answer to grit_world_create."
            }
            Self::UnknownRule => "Use a rule name grit_world_rules lists.",
            Self::Params => {
                "Give exactly as many params as the rule reads; grit_world_rules lists the counts."
            }
            Self::Closed => "Create a new world; a closed world's number is never used again.",
            Self::NoWorld => "Use a world number grit_world_create answered with.",
            Self::Reversed => "Give from as the least corner and to as the greatest on every axis.",
            Self::Axis => "Use axis x, y or z.",
            Self::Frames => "Step from 1 to the limit of frames in one call.",
            Self::Unfit => "Ask for smaller sides; each is rounded up to a power of two.",
            Self::TooBig => "Ask for a smaller world, or a sparse one.",
            Self::TooLarge => {
                "Ask for a smaller world, or a sparse one; one of its bindings passes what the device holds."
            }
            Self::TooManyGroups => {
                "Ask for a smaller world; one pass needs more workgroups than the device dispatches."
            }
            Self::Lane => {
                "Give every side at least 2 cells; the engine cannot build its kernels for this shape."
            }
            Self::PoolFull => {
                "Create the sparse world with more slots, or write fewer bricks that are not all one word."
            }
            Self::LostWrite => {
                "Create the sparse world with more slots; a frame dropped a write it had no slot for."
            }
            Self::NoComputeDevice => {
                "Run the server on a machine with a Vulkan compute device; tools that need none still answer."
            }
            Self::Device => "Close the world and create it again; if it repeats, report the call.",
            Self::Error => "Change the arguments the message names.",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Refusal {
    pub code: Code,
    pub field: String,
    pub limit: Option<Json>,
    pub message: String,
}

impl Refusal {
    #[must_use]
    pub fn new(code: Code, field: &str, message: String) -> Self {
        Self {
            code,
            field: field.to_owned(),
            limit: None,
            message,
        }
    }

    #[must_use]
    pub fn limited(mut self, limit: Json) -> Self {
        self.limit = Some(limit);
        self
    }

    #[must_use]
    pub fn told(&self) -> Vec<(&'static str, Json)> {
        let mut told = vec![
            ("code", text(self.code.name())),
            ("field", text(&self.field)),
        ];
        if let Some(limit) = &self.limit {
            told.push(("limit", limit.clone()));
        }
        told.push(("hint", text(self.code.hint())));
        told
    }

    #[must_use]
    pub fn data(&self) -> Json {
        map(self.told())
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(out, "{}", self.message)
    }
}

impl std::error::Error for Refusal {}

const SHOWN: usize = 64;

#[must_use]
pub fn shown(held: &str) -> String {
    if held.len() <= SHOWN {
        return held.to_owned();
    }
    let mut end = SHOWN;
    while !held.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}... ({} bytes)",
        held.get(..end).unwrap_or(""),
        held.len()
    )
}

#[must_use]
pub fn shown_value(held: &Json) -> String {
    shown(&crate::wire::written(held))
}

fn int(held: impl TryInto<i64>) -> Json {
    held.try_into().map_or(Json::Null, Json::Int)
}

#[must_use]
pub fn refusal_of(why: &(dyn std::error::Error + 'static)) -> Refusal {
    if let Some(held) = why.downcast_ref::<Refusal>() {
        return held.clone();
    }
    let message = why.to_string();
    let plain = |code| Refusal::new(code, "$", message.clone());
    if why.downcast_ref::<crate::sand::Unfit>().is_some() {
        return plain(Code::Unfit);
    }
    if let Some(held) = why.downcast_ref::<crate::compact::TooBig>() {
        let limit = match held {
            crate::compact::TooBig::Blocks { limit, .. } => int(*limit),
            crate::compact::TooBig::Cells { limit, .. } => int(*limit),
        };
        return plain(Code::TooBig).limited(limit);
    }
    if why.downcast_ref::<LaneError>().is_some() {
        return plain(Code::Lane);
    }
    match why.downcast_ref::<crate::host::Error>() {
        Some(crate::host::Error::PoolFull { slots }) => plain(Code::PoolFull).limited(int(*slots)),
        Some(crate::host::Error::LostWrite) => plain(Code::LostWrite),
        Some(crate::host::Error::TooLarge { allowed, .. }) => {
            plain(Code::TooLarge).limited(int(*allowed))
        }
        Some(crate::host::Error::TooManyGroups { allowed, .. }) => {
            plain(Code::TooManyGroups).limited(int(*allowed))
        }
        Some(crate::host::Error::NoComputeDevice) => plain(Code::NoComputeDevice),
        Some(crate::host::Error::Lane(_)) => plain(Code::Lane),
        Some(_) => plain(Code::Device),
        None => plain(Code::Error),
    }
}

#[must_use]
pub fn described() -> Json {
    Json::List(
        CODES
            .iter()
            .map(|code| {
                map(vec![
                    ("code", text(code.name())),
                    ("rpc", code.rpc().map_or(Json::Null, Json::Int)),
                    ("hint", text(code.hint())),
                ])
            })
            .collect(),
    )
}
