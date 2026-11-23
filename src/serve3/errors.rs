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
    Memory,
    Budget,
    Cancelled,
    DeviceLost,
    Path,
    NoFile,
    Exists,
    BadFile,
    Unlike,
    Io,
    RuleOrLaw,
    LawRefused,
    LawNotBuilt,
    KernelNotBuilt,
    QuietRefused,
    HaloSparse,
    ChipRefused,
    Unbound,
    Overbound,
    PlanePast,
    BitPast,
    OffsetPast,
    BitShared,
    HaloWritten,
    ChipTooBig,
    NotReversible,
    PastStart,
    Strayed,
    NoGradient,
    TargetSize,
    Overflow,
    Start,
    Aim,
    WalkLimit,
    Bounds,
    RunLimit,
    Example,
    Conflict,
    Asymmetric,
    Unsolved,
    LearnLimit,
    CountReads,
    NotCounting,
    StartShape,
    StartRule,
    NoiseRule,
    Places,
    PlacesRule,
    SupportNoise,
    SupportLimit,
    NotIterating,
    PlaneOrCurrent,
    ReadForm,
    MarkUnknown,
    MarkUnlike,
    StepsLimit,
    StepsRule,
    StepsNoise,
    StepsWrites,
    StepsSpace,
    NoRuleK,
    ParallelBit,
}

pub const CODES: [Code; 91] = [
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
    Code::Memory,
    Code::Budget,
    Code::Cancelled,
    Code::DeviceLost,
    Code::Path,
    Code::NoFile,
    Code::Exists,
    Code::BadFile,
    Code::Unlike,
    Code::Io,
    Code::RuleOrLaw,
    Code::LawRefused,
    Code::LawNotBuilt,
    Code::KernelNotBuilt,
    Code::QuietRefused,
    Code::HaloSparse,
    Code::ChipRefused,
    Code::Unbound,
    Code::Overbound,
    Code::PlanePast,
    Code::BitPast,
    Code::OffsetPast,
    Code::BitShared,
    Code::HaloWritten,
    Code::ChipTooBig,
    Code::NotReversible,
    Code::PastStart,
    Code::Strayed,
    Code::NoGradient,
    Code::TargetSize,
    Code::Overflow,
    Code::Start,
    Code::Aim,
    Code::WalkLimit,
    Code::Bounds,
    Code::RunLimit,
    Code::Example,
    Code::Conflict,
    Code::Asymmetric,
    Code::Unsolved,
    Code::LearnLimit,
    Code::CountReads,
    Code::NotCounting,
    Code::StartShape,
    Code::StartRule,
    Code::NoiseRule,
    Code::Places,
    Code::PlacesRule,
    Code::SupportNoise,
    Code::SupportLimit,
    Code::NotIterating,
    Code::PlaneOrCurrent,
    Code::ReadForm,
    Code::MarkUnknown,
    Code::MarkUnlike,
    Code::StepsLimit,
    Code::StepsRule,
    Code::StepsNoise,
    Code::StepsWrites,
    Code::StepsSpace,
    Code::NoRuleK,
    Code::ParallelBit,
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
            Self::Memory => "memory",
            Self::Budget => "budget",
            Self::Cancelled => "cancelled",
            Self::DeviceLost => "device_lost",
            Self::Path => "path",
            Self::NoFile => "no_file",
            Self::Exists => "exists",
            Self::BadFile => "bad_file",
            Self::Unlike => "unlike",
            Self::Io => "io",
            Self::RuleOrLaw => "rule_or_law",
            Self::LawRefused => "law_refused",
            Self::LawNotBuilt => "law_not_built",
            Self::KernelNotBuilt => "kernel_not_built",
            Self::QuietRefused => "quiet_refused",
            Self::HaloSparse => "halo_sparse",
            Self::ChipRefused => "chip_refused",
            Self::Unbound => "unbound",
            Self::Overbound => "overbound",
            Self::PlanePast => "plane_past",
            Self::BitPast => "bit_past",
            Self::OffsetPast => "offset_past",
            Self::BitShared => "bit_shared",
            Self::HaloWritten => "halo_written",
            Self::ChipTooBig => "chip_too_big",
            Self::NotReversible => "not_reversible",
            Self::PastStart => "past_start",
            Self::Strayed => "strayed",
            Self::NoGradient => "no_gradient",
            Self::TargetSize => "target_size",
            Self::Overflow => "overflow",
            Self::Start => "start",
            Self::Aim => "aim",
            Self::WalkLimit => "walk_limit",
            Self::Bounds => "bounds",
            Self::RunLimit => "run_limit",
            Self::Example => "example",
            Self::Conflict => "conflict",
            Self::Asymmetric => "asymmetric",
            Self::Unsolved => "unsolved",
            Self::LearnLimit => "learn_limit",
            Self::CountReads => "count_reads",
            Self::NotCounting => "not_counting",
            Self::StartShape => "start_shape",
            Self::StartRule => "start_rule",
            Self::NoiseRule => "noise_rule",
            Self::Places => "places",
            Self::PlacesRule => "places_rule",
            Self::SupportNoise => "support_noise",
            Self::SupportLimit => "support_limit",
            Self::NotIterating => "not_iterating",
            Self::PlaneOrCurrent => "plane_or_current",
            Self::ReadForm => "read_form",
            Self::MarkUnknown => "mark_unknown",
            Self::MarkUnlike => "mark_unlike",
            Self::StepsLimit => "steps_limit",
            Self::StepsRule => "steps_rule",
            Self::StepsNoise => "steps_noise",
            Self::StepsWrites => "steps_writes",
            Self::StepsSpace => "steps_space",
            Self::NoRuleK => "no_rule_k",
            Self::ParallelBit => "parallel_bit",
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
            Self::Memory => {
                "Close or save a world first, or ask for a smaller one; the answer names the bytes asked, held and allowed."
            }
            Self::Budget => {
                "Give the call a larger budget_ms, up to the limit grit_describe names, or ask for less in one call."
            }
            Self::Cancelled => "Nothing past the frames reported ran; call again for the rest.",
            Self::DeviceLost => {
                "Call again; the server rebuilt every world it could at the frame it reports, as restored says."
            }
            Self::Path => {
                "Name a file with 1 to 64 of a-z, 0-9, _ and -, starting with a letter or digit, and not a device name such as con or nul."
            }
            Self::NoFile => "Name a file grit_world_list shows, or a world it lists as kept.",
            Self::Exists => "Choose another name, or pass replace true to write over the file.",
            Self::BadFile => {
                "Open another file; this one is not a world this server wrote, or it was changed since."
            }
            Self::Unlike => {
                "Restore a world only from one of the same rule, size and layout, such as a fork of it."
            }
            Self::Io => "Free disk space or fix the server directory, then call again.",
            Self::RuleOrLaw => {
                "Give exactly one of rule, a name grit_world_rules lists, and law, the JSON text of a law."
            }
            Self::LawRefused => {
                "Fix the law at the path its refusal names; law.code says what is wrong there."
            }
            Self::KernelNotBuilt => {
                "This device's driver could not compile one of the engine's kernels for this call, so retrying on this device fails the same way. No world was made or changed, and a kept world stays kept: it opens on a device that compiles it. Use another device, and report the call with the device the message names."
            }
            Self::LawNotBuilt => {
                "This device's driver could not compile a kernel made from this law, though the law is valid. No world was made or changed, and a kept world stays kept: it opens on a device that compiles it. Write the failing part another way or use another device, and report the law with the device the message names."
            }
            Self::QuietRefused => {
                "Make the world dense, which steps every oct, or keep params under which the law is proved."
            }
            Self::HaloSparse => {
                "Make the world dense: a law that looks (halo 1) runs only on dense worlds."
            }
            Self::ChipRefused => {
                "Give a chip as grit_machine takes it: inputs plus state at most 31, at least state plus outputs gates, each gate reading an input or a gate before it, kinds 0 to 15."
            }
            Self::Unbound => {
                "Give one binding for every chip input in bind_inputs, every output in bind_outputs and every state bit in bind_state, in the chip's order."
            }
            Self::Overbound => {
                "Give exactly one binding for each chip input, output and state bit; remove the bindings past the chip's count."
            }
            Self::PlanePast => "Name a plane below channels.",
            Self::BitPast => "Name a bit from 0 to 31.",
            Self::OffsetPast => "Give dx, dy and dz each as -1, 0 or 1.",
            Self::BitShared => "Give every output and state bit its own bit of its plane.",
            Self::ChipTooBig => {
                "Search again with fewer gates; a chip of at most limits.law_from_chip_gates gates always fits one law."
            }
            Self::HaloWritten => {
                "Read a plane an output or state bit is on only at offset 0; put the inputs that read neighbours on a plane the chip does not write, such as plane 0 with the outputs on plane 1."
            }
            Self::NotReversible => {
                "Walk back or take gradients only through a law that declares \"reversible\": true and is lifting steps; for any other, fork the world before stepping and restore the fork, or search its params."
            }
            Self::PastStart => "Walk back at most as many frames as the world's frame.",
            Self::Strayed => {
                "Report the law and the call: an accepted reversible law must come home; the world was left as it was."
            }
            Self::NoGradient => {
                "Take gradients only through a law whose params reach its cells only through add, sub, mul, div, shifts, sel arms and compares; search the params of any other law."
            }
            Self::TargetSize => {
                "Give the target exactly as many words as the aim's box holds, x fastest, then y, then z."
            }
            Self::Overflow => {
                "Take the gradient over fewer frames or a smaller box; it passed the 128-bit fixed point it is carried in."
            }
            Self::Start => {
                "Give a world, or a law with size, each side a power of two from 2, and cells, size times channels words plane by plane; not both."
            }
            Self::Aim => {
                "Give the aim exactly one of target, the box's wanted cells, and world, a world to take them from."
            }
            Self::WalkLimit => {
                "Take gradients on a smaller start or over fewer frames; grit_describe names walk_cells and walk_work."
            }
            Self::Bounds => {
                "Give each param inside its [least, greatest] bounds, and bounds that let at least one param move."
            }
            Self::RunLimit => {
                "Score or search on a smaller start or over fewer frames; grit_describe names run_cells and run_work."
            }
            Self::Example => {
                "Give each example either size, input and output inline, or world with an optional output_world, from and to; not both."
            }
            Self::Conflict => {
                "Fix or drop the examples that give one neighbourhood two outputs; rows names each with where it was seen."
            }
            Self::Asymmetric => {
                "Use rule minimal, or fix the examples whose neighbourhoods are rotations or reflections of each other with different outputs; orbits names them."
            }
            Self::Unsolved => {
                "Search with more gates, rounds or crowd; no circuit at or below gates was found within them."
            }
            Self::LearnLimit => {
                "Give fewer reads or example cells, fewer rows to search, or at least as many gates as writes; grit_describe names learn_inputs, learn_cells and learn_examples."
            }
            Self::CountReads => {
                "Give rule totalistic or outer exactly one read, the bit whose places it counts, or use rule minimal or symmetric."
            }
            Self::NotCounting => {
                "Use rule minimal or symmetric, or fix the examples that give one count two outputs; counts names each with a row for each output."
            }
            Self::StartShape => {
                "Give start as many inputs as the stencil's places times the reads, one output for each write and no state, as the chip of an earlier grit_law_learn answer with the same stencil, reads and writes has."
            }
            Self::StartRule => {
                "Drop start, or use rule minimal or symmetric; totalistic and outer read their tables in one pass with no search to start."
            }
            Self::NoiseRule => {
                "Drop noise or give it refuse, or use rule totalistic or outer; minimal and symmetric take no vote."
            }
            Self::Places => {
                "Give places a preset the stencil has (hex is 3x3 only), or place_list distinct place indices of the stencil with its centre, not both."
            }
            Self::PlacesRule => {
                "Drop places and place_list, or use rule totalistic or outer; minimal and symmetric read every place."
            }
            Self::SupportNoise => {
                "Drop min_support, or give noise vote; only a vote has counts decided on few cells."
            }
            Self::SupportLimit => {
                "Give min_support from 1 to the stencil's places times 64: 576 for 3x3, 1728 for 3x3x3."
            }
            Self::NotIterating => {
                "Name the plane by number: only a world made with alternate has a current plane."
            }
            Self::PlaneOrCurrent => {
                "Give plane or current true, not both: current names the plane itself."
            }
            Self::ReadForm => {
                "Give since only with form changed, always with it, and mark only with form set or changed."
            }
            Self::MarkUnknown => {
                "Read with form set and mark true to take a new mark; marks are held in this server's memory only, so a save, an open, a restart and a rebuild after a lost device start without them."
            }
            Self::MarkUnlike => {
                "Read with the from, to and plane or current of the read that took the mark, or take a new mark."
            }
            Self::StepsLimit => "Give steps from 1 to 8; grit_describe names law_steps_most.",
            Self::StepsRule => {
                "Drop steps or give it 1, or use rule totalistic or outer; only a counting law is searched over its tables."
            }
            Self::StepsNoise => {
                "Drop noise or give it refuse, or give steps 1; a vote over examples k steps apart is not offered."
            }
            Self::StepsWrites => {
                "Give one write on the read's bit: a law applied k times reads back what it wrote."
            }
            Self::StepsSpace => {
                "Count fewer places with places or place_list, so the rule has at most 18 classes; grit_describe names law_steps_classes."
            }
            Self::NoRuleK => {
                "No counting table applied k times gives every example; check steps, border and the examples, or use another rule; closest names the nearest table and its wrong cells."
            }
            Self::ParallelBit => {
                "Give every read and write bit 0 with parallel true: each is the whole word of its plane, bit j of it copy j; or drop parallel."
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Refusal {
    pub code: Code,
    pub field: String,
    pub limit: Option<Json>,
    pub message: String,
    pub hint: Option<&'static str>,
}

impl Refusal {
    #[must_use]
    pub fn new(code: Code, field: &str, message: String) -> Self {
        Self {
            code,
            field: field.to_owned(),
            limit: None,
            message,
            hint: None,
        }
    }

    #[must_use]
    pub const fn hinted(mut self, hint: &'static str) -> Self {
        self.hint = Some(hint);
        self
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
        told.push(("hint", text(self.hint.unwrap_or_else(|| self.code.hint()))));
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

pub const FULL_BOX: &str = "Split the box into smaller boxes, or read a box this full with form dense: it answers every cell's word, up to its own limit of cells a call, and costs less than a list once a box is about a quarter set.";

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
    if why.downcast_ref::<crate::branch::LawUnbuilt>().is_some() {
        return Refusal::new(Code::LawNotBuilt, "$.law", message);
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
        Some(crate::host::Error::Unbuilt { .. }) => plain(Code::KernelNotBuilt),
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
