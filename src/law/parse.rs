use std::collections::{HashMap, HashSet};

use super::expand::Places;
use super::{Code, Refusal, limits};
use crate::rule3::Tally;
use crate::wire::{self, Json, Torn};

#[derive(Clone, Copy, Debug)]
pub(crate) enum Step<'a> {
    Root,
    Key(&'a str),
    Index(usize),
}

#[derive(Clone, Copy, Debug)]
pub struct Path<'a> {
    parent: Option<&'a Path<'a>>,
    step: Step<'a>,
}

impl<'a> Path<'a> {
    #[must_use]
    pub const fn root() -> Self {
        Self {
            parent: None,
            step: Step::Root,
        }
    }

    #[must_use]
    pub const fn key<'b>(&'b self, key: &'b str) -> Path<'b>
    where
        'a: 'b,
    {
        Path {
            parent: Some(self),
            step: Step::Key(key),
        }
    }

    #[must_use]
    pub const fn index<'b>(&'b self, index: usize) -> Path<'b>
    where
        'a: 'b,
    {
        Path {
            parent: Some(self),
            step: Step::Index(index),
        }
    }

    #[must_use]
    pub fn shown(&self) -> String {
        let mut steps = Vec::new();
        let mut here = Some(self);
        while let Some(path) = here {
            steps.push(path.step);
            here = path.parent;
        }
        steps.reverse();
        shown(&steps)
    }
}

pub(crate) fn shown(steps: &[Step<'_>]) -> String {
    let mut out = String::from("$");
    for step in steps {
        match *step {
            Step::Root => {}
            Step::Key(key) if plain(key) && key.len() <= limits::NAME => {
                out.push('.');
                out.push_str(key);
            }
            Step::Key(key) => {
                out.push('[');
                out.push_str(&wire::written(&Json::Text(clipped(key))));
                out.push(']');
            }
            Step::Index(index) => {
                out.push('[');
                out.push_str(&index.to_string());
                out.push(']');
            }
        }
    }
    out
}

fn plain(key: &str) -> bool {
    let mut chars = key.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|one| one.is_ascii_alphanumeric() || one == '_')
}

pub(crate) fn refused(
    code: Code,
    path: &Path<'_>,
    expected: impl Into<String>,
    found: impl Into<String>,
) -> Refusal {
    Refusal {
        code,
        path: path.shown(),
        expected: expected.into(),
        found: found.into(),
        limit: None,
    }
}

pub(crate) fn limited(
    code: Code,
    path: &Path<'_>,
    expected: impl Into<String>,
    found: impl Into<String>,
    limit: u64,
) -> Refusal {
    Refusal {
        limit: Some(limit),
        ..refused(code, path, expected, found)
    }
}

pub(crate) fn clipped(text: &str) -> String {
    let mut chars = text.chars();
    let mut out: String = chars.by_ref().take(limits::SHOWN).collect();
    if chars.next().is_some() {
        out.push_str("...");
    }
    out
}

pub(crate) fn described(json: &Json) -> String {
    match json {
        Json::Null => String::from("null"),
        Json::Bool(held) => held.to_string(),
        Json::Int(held) => format!("the number {held}"),
        Json::Text(held) => format!("the text {}", wire::written(&Json::Text(clipped(held)))),
        Json::List(held) => format!("a list of {}", held.len()),
        Json::Map(held) => format!("an object of {} keys", held.len()),
    }
}

const SPACE: [u8; 4] = *b" \t\n\r";

fn ended(bytes: &[u8]) -> Option<usize> {
    let start = bytes.iter().position(|byte| !SPACE.contains(byte))?;
    let mut depth = 0_usize;
    let mut quoted = false;
    let mut escaped = false;
    for (at, &byte) in bytes.iter().enumerate().skip(start) {
        if quoted {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
            }
            continue;
        }
        match byte {
            b'"' => quoted = true,
            b'{' | b'[' => depth += 1,
            b'}' | b']' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(at + 1);
                }
            }
            _ => {}
        }
    }
    None
}

fn torn(why: Torn) -> String {
    match why {
        Torn::Ended { at } => format!("the text ending at byte {at}"),
        Torn::Stray { at, saw } => format!("{saw:?} at byte {at}"),
        Torn::Twice { at } => format!("a key again at byte {at}"),
        Torn::TooDeep { at } => format!("nesting deeper than 64 at byte {at}"),
        Torn::NotANumber { at } => {
            format!("a number at byte {at} that is not a whole number of at most 19 digits")
        }
    }
}

pub fn read(text: &str) -> Result<Json, Refusal> {
    let root = Path::root();
    if text.len() > limits::BODY {
        return Err(limited(
            Code::Size,
            &root,
            format!("a law of at most {} bytes", limits::BODY),
            format!("{} bytes", text.len()),
            limits::BODY as u64,
        ));
    }
    let json = wire::read(text).map_err(|why| match why {
        Torn::Twice { at } => twice(text, at),
        other => refused(Code::Json, &root, "one JSON object", torn(other)),
    })?;
    if !matches!(json, Json::Map(_)) {
        return Err(refused(
            Code::Shape,
            &root,
            "a JSON object",
            described(&json),
        ));
    }
    let bytes = text.as_bytes();
    let end = ended(bytes).unwrap_or(bytes.len());
    if let Some(offset) = bytes
        .get(end..)
        .and_then(|rest| rest.iter().position(|byte| !SPACE.contains(byte)))
    {
        return Err(refused(
            Code::Trailing,
            &root,
            "nothing after the law's object",
            format!("more text at byte {}", end + offset),
        ));
    }
    Ok(json)
}

enum Frame {
    List(usize),
    Map(Option<String>, bool),
}

fn quoted_end(bytes: &[u8], from: usize) -> usize {
    let mut at = from + 1;
    while let Some(&byte) = bytes.get(at) {
        match byte {
            b'\\' => at += 2,
            b'"' => return at + 1,
            _ => at += 1,
        }
    }
    bytes.len()
}

fn key_at(text: &str, from: usize) -> String {
    let end = quoted_end(text.as_bytes(), from);
    match text.get(from..end).map(wire::read) {
        Some(Ok(Json::Text(key))) => key,
        _ => String::new(),
    }
}

fn twice(text: &str, at: usize) -> Refusal {
    let bytes = text.as_bytes();
    let mut frames: Vec<Frame> = Vec::new();
    let mut here = 0;
    while here < at {
        let Some(&byte) = bytes.get(here) else {
            break;
        };
        match byte {
            b'"' => {
                let end = quoted_end(bytes, here);
                if let Some(Frame::Map(key, expecting)) = frames.last_mut()
                    && *expecting
                {
                    *key = Some(key_at(text, here));
                    *expecting = false;
                }
                here = end;
                continue;
            }
            b'{' => frames.push(Frame::Map(None, true)),
            b'[' => frames.push(Frame::List(0)),
            b'}' | b']' => {
                frames.pop();
            }
            b',' => match frames.last_mut() {
                Some(Frame::List(index)) => *index += 1,
                Some(Frame::Map(_, expecting)) => *expecting = true,
                None => {}
            },
            _ => {}
        }
        here += 1;
    }
    let again = key_at(text, at);
    let mut steps: Vec<Step<'_>> = Vec::with_capacity(frames.len());
    let last = frames.len().saturating_sub(1);
    for (depth, frame) in frames.iter().enumerate() {
        match frame {
            Frame::List(index) => steps.push(Step::Index(*index)),
            Frame::Map(_, _) if depth == last => steps.push(Step::Key(&again)),
            Frame::Map(Some(key), _) => steps.push(Step::Key(key)),
            Frame::Map(None, _) => {}
        }
    }
    Refusal {
        code: Code::DuplicateKey,
        path: shown(&steps),
        expected: String::from("each key once"),
        found: format!(
            "the key {} again",
            wire::written(&Json::Text(clipped(&again)))
        ),
        limit: None,
    }
}

pub const SPECIAL: [&str; 14] = [
    "let", "if", "vec", "range", "at", "put", "map", "fold", "filter", "rev", "concat", "param",
    "noise", "look",
];

pub const BINARY: [&str; 19] = [
    "add", "sub", "mul", "div", "and", "or", "xor", "dot_s", "dot_u", "shl", "shr", "eq", "ne",
    "lt", "le", "gt", "ge", "both", "either",
];

pub const UNARY: [&str; 4] = ["not", "denied", "word", "count"];

pub const TERNARY: [&str; 1] = ["sel"];

pub const INPUTS: [&str; 6] = ["X", "Y", "Z", "cells", "frame", "at"];

const KEYS: [&str; 9] = [
    "law", "name", "params", "channels", "halo", "tally", "consts", "defs", "step",
];

const OPTIONAL: [&str; 1] = ["reversible"];

const TALLY_KEYS: [&str; 2] = ["counts", "weighted"];

fn is_op(name: &str) -> bool {
    SPECIAL.contains(&name)
        || BINARY.contains(&name)
        || UNARY.contains(&name)
        || TERNARY.contains(&name)
}

#[derive(Clone, Debug)]
pub struct Def<'p> {
    pub name: &'p str,
    pub args: Vec<&'p str>,
    pub body: &'p Json,
}

#[derive(Clone, Debug)]
pub struct Program<'p> {
    pub name: &'p str,
    pub params: usize,
    pub channels: u32,
    pub halo: u32,
    pub tally: Tally,
    pub reversible: bool,
    pub consts: Vec<(&'p str, u32)>,
    pub defs: Vec<Def<'p>>,
    pub named: HashMap<&'p str, usize>,
    pub step: &'p Json,
}

fn walked(json: &Json, path: &Path<'_>, depth: usize) -> Result<(), Refusal> {
    if depth > limits::DEPTH {
        return Err(limited(
            Code::Depth,
            path,
            format!("nesting of at most {}", limits::DEPTH),
            "deeper nesting",
            limits::DEPTH as u64,
        ));
    }
    match json {
        Json::List(items) => {
            for (at, item) in items.iter().enumerate() {
                walked(item, &path.index(at), depth + 1)?;
            }
        }
        Json::Map(fields) => {
            let mut seen = HashSet::with_capacity(fields.len());
            for (name, item) in fields {
                let here = path.key(name);
                if !seen.insert(name.as_str()) {
                    return Err(refused(
                        Code::DuplicateKey,
                        &here,
                        "each key once",
                        format!(
                            "the key {} again",
                            wire::written(&Json::Text(clipped(name)))
                        ),
                    ));
                }
                walked(item, &here, depth + 1)?;
            }
        }
        Json::Null | Json::Bool(_) | Json::Int(_) | Json::Text(_) => {}
    }
    Ok(())
}

fn fields<'p>(
    json: &'p Json,
    path: &Path<'_>,
    keys: &[&str],
    optional: &[&str],
) -> Result<&'p [(String, Json)], Refusal> {
    let Json::Map(fields) = json else {
        return Err(refused(Code::Shape, path, "an object", described(json)));
    };
    if let Some((name, _)) = fields
        .iter()
        .find(|(name, _)| !keys.contains(&name.as_str()) && !optional.contains(&name.as_str()))
    {
        let known: Vec<&str> = keys.iter().chain(optional).copied().collect();
        return Err(refused(
            Code::UnknownKey,
            &path.key(name),
            format!("only the keys {}", known.join(", ")),
            format!("the key {}", wire::written(&Json::Text(clipped(name)))),
        ));
    }
    for key in keys {
        if !fields.iter().any(|(name, _)| name == key) {
            return Err(refused(
                Code::Missing,
                &path.key(key),
                format!("the key {key}"),
                "no such key",
            ));
        }
    }
    Ok(fields)
}

fn fields_of<'p>(fields: &'p [(String, Json)], key: &str) -> Option<&'p Json> {
    fields
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, one)| one)
}

fn field<'p>(fields: &'p [(String, Json)], key: &str) -> &'p Json {
    fields_of(fields, key).unwrap_or(&Json::Null)
}

fn whole(
    json: &Json,
    path: &Path<'_>,
    code: Code,
    least: u64,
    most: u64,
    what: &str,
) -> Result<u64, Refusal> {
    let Json::Int(held) = json else {
        return Err(refused(Code::Shape, path, what, described(json)));
    };
    match u64::try_from(*held) {
        Ok(value) if (least..=most).contains(&value) => Ok(value),
        _ => Err(limited(
            code,
            path,
            if least == most {
                format!("{what} {least}")
            } else {
                format!("{what} from {least} to {most}")
            },
            held.to_string(),
            most,
        )),
    }
}

fn literal(json: &Json, path: &Path<'_>) -> Result<u32, Refusal> {
    match json {
        Json::Int(held) => u32::try_from(*held).map_err(|_| {
            refused(
                Code::Literal,
                path,
                "a whole number from 0 to 4294967295",
                held.to_string(),
            )
        }),
        other => Err(refused(
            Code::Shape,
            path,
            "a whole number from 0 to 4294967295",
            described(other),
        )),
    }
}

fn short(name: &str, path: &Path<'_>) -> Result<(), Refusal> {
    if name.len() > limits::NAME {
        return Err(limited(
            Code::Name,
            path,
            format!("a name of at most {} bytes", limits::NAME),
            format!("{} bytes", name.len()),
            limits::NAME as u64,
        ));
    }
    Ok(())
}

fn named<'p>(json: &'p Json, path: &Path<'_>, what: &str) -> Result<&'p str, Refusal> {
    match json {
        Json::Text(held) => short(held, path).map(|()| held.as_str()),
        other => Err(refused(Code::Shape, path, what, described(other))),
    }
}

pub fn program(json: &Json) -> Result<Program<'_>, Refusal> {
    let root = Path::root();
    walked(json, &root, 0)?;
    let top = fields(json, &root, &KEYS, &OPTIONAL)?;
    whole(
        field(top, "law"),
        &root.key("law"),
        Code::Version,
        u64::from(limits::VERSION),
        u64::from(limits::VERSION),
        "the language version",
    )?;
    let name = named(
        field(top, "name"),
        &root.key("name"),
        "the law's name as text",
    )?;
    let params = whole(
        field(top, "params"),
        &root.key("params"),
        Code::Params,
        0,
        limits::PARAMS as u64,
        "a count of params",
    )?;
    let channels = whole(
        field(top, "channels"),
        &root.key("channels"),
        Code::Channels,
        1,
        u64::from(limits::CHANNELS),
        "a count of channels",
    )?;
    let halo = whole(
        field(top, "halo"),
        &root.key("halo"),
        Code::Halo,
        0,
        u64::from(limits::HALO),
        "a halo radius",
    )?;
    let tally_path = root.key("tally");
    let tallied = fields(field(top, "tally"), &tally_path, &TALLY_KEYS, &[])?;
    let counts = whole(
        field(tallied, "counts"),
        &tally_path.key("counts"),
        Code::Counts,
        1,
        limits::COUNTS as u64,
        "a count of tally counts",
    )?;
    let weighted = match field(tallied, "weighted") {
        Json::Bool(held) => *held,
        other => {
            return Err(refused(
                Code::Shape,
                &tally_path.key("weighted"),
                "true or false",
                described(other),
            ));
        }
    };
    let reversible = match fields_of(top, "reversible") {
        None => false,
        Some(Json::Bool(true)) => true,
        Some(other) => {
            return Err(refused(
                Code::Shape,
                &root.key("reversible"),
                "true, or no key for a law that is not reversible",
                described(other),
            ));
        }
    };
    let consts_path = root.key("consts");
    let Json::Map(const_fields) = field(top, "consts") else {
        return Err(refused(
            Code::Shape,
            &consts_path,
            "an object of names to numbers",
            described(field(top, "consts")),
        ));
    };
    let mut consts = Vec::with_capacity(const_fields.len());
    for (key, value) in const_fields {
        let here = consts_path.key(key);
        short(key, &here)?;
        if INPUTS.contains(&key.as_str()) {
            return Err(refused(
                Code::Reserved,
                &here,
                format!("a name other than {}", INPUTS.join(", ")),
                format!("the name {key}"),
            ));
        }
        consts.push((key.as_str(), literal(value, &here)?));
    }
    let defs_path = root.key("defs");
    let Json::Map(def_fields) = field(top, "defs") else {
        return Err(refused(
            Code::Shape,
            &defs_path,
            "an object of names to [[args], body]",
            described(field(top, "defs")),
        ));
    };
    let mut defs = Vec::with_capacity(def_fields.len());
    let mut by_name = HashMap::with_capacity(def_fields.len());
    for (key, value) in def_fields {
        let here = defs_path.key(key);
        short(key, &here)?;
        if is_op(key) {
            return Err(refused(
                Code::Reserved,
                &here,
                "a name that is not an operation of the language",
                format!("the operation {key}"),
            ));
        }
        let (args, body) = match value {
            Json::List(pair) => match pair.as_slice() {
                [Json::List(args), body] => (args, body),
                [other, _] => {
                    return Err(refused(
                        Code::Shape,
                        &here.index(0),
                        "a list of argument names",
                        described(other),
                    ));
                }
                _ => {
                    return Err(refused(
                        Code::Shape,
                        &here,
                        "[[args], body]",
                        described(value),
                    ));
                }
            },
            other => {
                return Err(refused(
                    Code::Shape,
                    &here,
                    "[[args], body]",
                    described(other),
                ));
            }
        };
        let args_path = here.index(0);
        let mut names = Vec::with_capacity(args.len());
        for (at, arg) in args.iter().enumerate() {
            names.push(named(arg, &args_path.index(at), "an argument name")?);
        }
        by_name.insert(key.as_str(), defs.len());
        defs.push(Def {
            name: key,
            args: names,
            body,
        });
    }
    let program = Program {
        name,
        params: usize::try_from(params).unwrap_or(usize::MAX),
        channels: u32::try_from(channels).unwrap_or(u32::MAX),
        halo: u32::try_from(halo).unwrap_or(u32::MAX),
        tally: Tally {
            counts: usize::try_from(counts).unwrap_or(usize::MAX),
            weighted,
        },
        reversible,
        consts,
        defs,
        named: by_name,
        step: field(top, "step"),
    };
    checked(&program, json)?;
    Ok(program)
}

struct Checking<'c, 'p> {
    program: &'c Program<'p>,
    globals: HashSet<&'p str>,
    locals: Vec<&'p str>,
    bound: HashMap<&'p str, usize>,
    calls: Vec<(usize, usize)>,
    site: usize,
}

fn arity(name: &str) -> Option<(usize, Option<usize>)> {
    if BINARY.contains(&name) {
        return Some((2, Some(2)));
    }
    if UNARY.contains(&name) {
        return Some((1, Some(1)));
    }
    if TERNARY.contains(&name) {
        return Some((3, Some(3)));
    }
    match name {
        "let" | "at" => Some((2, Some(2))),
        "if" | "put" | "map" | "filter" => Some((3, Some(3))),
        "fold" => Some((5, Some(5))),
        "range" | "rev" | "param" => Some((1, Some(1))),
        "noise" => Some((0, Some(0))),
        "vec" | "concat" => Some((0, None)),
        _ => None,
    }
}

fn arguments(count: usize) -> String {
    match count {
        1 => String::from("1 argument"),
        _ => format!("{count} arguments"),
    }
}

impl<'p> Checking<'_, 'p> {
    fn bind(&mut self, name: &'p str) {
        *self.bound.entry(name).or_insert(0) += 1;
        self.locals.push(name);
    }

    fn unbind(&mut self, mark: usize) {
        while self.locals.len() > mark {
            if let Some(name) = self.locals.pop()
                && let Some(many) = self.bound.get_mut(name)
            {
                *many = many.saturating_sub(1);
            }
        }
    }

    fn binder(&self, json: &'p Json, path: &Path<'_>) -> Result<&'p str, Refusal> {
        named(json, path, "a name to bind")
    }

    fn expression(&mut self, json: &'p Json, path: &Path<'_>) -> Result<(), Refusal> {
        match json {
            Json::Int(_) => literal(json, path).map(|_| ()),
            Json::Text(name) => {
                short(name, path)?;
                if self.bound.get(name.as_str()).is_some_and(|&many| many > 0)
                    || self.globals.contains(name.as_str())
                {
                    Ok(())
                } else {
                    Err(refused(
                        Code::UnknownName,
                        path,
                        "a const, an input or a bound name",
                        format!("the name {}", wire::written(&Json::Text(clipped(name)))),
                    ))
                }
            }
            Json::List(items) => {
                self.site = std::ptr::from_ref(json).addr();
                self.operation(items, path)
            }
            other => Err(refused(
                Code::Shape,
                path,
                "a number, a name or an operation list",
                described(other),
            )),
        }
    }

    fn operation(&mut self, items: &'p [Json], path: &Path<'_>) -> Result<(), Refusal> {
        let Some((head, args)) = items.split_first() else {
            return Err(refused(
                Code::Shape,
                path,
                "an operation list [op, args...]",
                "an empty list",
            ));
        };
        let Json::Text(op) = head else {
            return Err(refused(
                Code::Shape,
                &path.index(0),
                "an operation name",
                described(head),
            ));
        };
        short(op, &path.index(0))?;
        if op == "look" {
            return Err(refused(
                Code::Unsupported,
                &path.index(0),
                "an operation other than look: the halo is not built yet",
                "look",
            ));
        }
        let (least, most) = match arity(op) {
            Some(range) => range,
            None => match self.program.named.get(op.as_str()) {
                Some(&index) => {
                    let wanted = self.program.defs.get(index).map_or(0, |def| def.args.len());
                    self.calls.push((index, self.site));
                    (wanted, Some(wanted))
                }
                None => {
                    return Err(refused(
                        Code::UnknownOp,
                        &path.index(0),
                        "an operation of the language or a def",
                        format!("the name {}", wire::written(&Json::Text(clipped(op)))),
                    ));
                }
            },
        };
        if args.len() < least || most.is_some_and(|most| args.len() > most) {
            let expected = match most {
                Some(most) if most == least => arguments(least),
                _ => format!("at least {}", arguments(least)),
            };
            return Err(refused(
                Code::Arity,
                path,
                format!("{op} with {expected}"),
                arguments(args.len()),
            ));
        }
        let arg = |at: usize| args.get(at).unwrap_or(&Json::Null);
        let at = |at: usize| path.index(at + 1);
        match op.as_str() {
            "let" => {
                let Json::List(bindings) = arg(0) else {
                    return Err(refused(
                        Code::Shape,
                        &at(0),
                        "a list of [name, value] bindings",
                        described(arg(0)),
                    ));
                };
                let bindings_path = at(0);
                let mark = self.locals.len();
                for (index, binding) in bindings.iter().enumerate() {
                    let here = bindings_path.index(index);
                    let Json::List(pair) = binding else {
                        return Err(refused(
                            Code::Shape,
                            &here,
                            "a binding [name, value]",
                            described(binding),
                        ));
                    };
                    let [name, value] = pair.as_slice() else {
                        return Err(refused(
                            Code::Shape,
                            &here,
                            "a binding [name, value]",
                            described(binding),
                        ));
                    };
                    let name = self.binder(name, &here.index(0))?;
                    self.expression(value, &here.index(1))?;
                    self.bind(name);
                }
                self.expression(arg(1), &at(1))?;
                self.unbind(mark);
            }
            "map" | "filter" => {
                let name = self.binder(arg(0), &at(0))?;
                self.expression(arg(1), &at(1))?;
                let mark = self.locals.len();
                self.bind(name);
                self.expression(arg(2), &at(2))?;
                self.unbind(mark);
            }
            "fold" => {
                let acc = self.binder(arg(0), &at(0))?;
                self.expression(arg(1), &at(1))?;
                let name = self.binder(arg(2), &at(2))?;
                self.expression(arg(3), &at(3))?;
                let mark = self.locals.len();
                self.bind(acc);
                self.bind(name);
                self.expression(arg(4), &at(4))?;
                self.unbind(mark);
            }
            _ => {
                for (index, one) in args.iter().enumerate() {
                    self.expression(one, &at(index))?;
                }
            }
        }
        Ok(())
    }
}

fn checked(program: &Program<'_>, json: &Json) -> Result<(), Refusal> {
    let root = Path::root();
    let mut globals: HashSet<&str> = INPUTS.iter().copied().collect();
    globals.extend(program.consts.iter().map(|(name, _)| *name));
    let mut checking = Checking {
        program,
        globals,
        locals: Vec::new(),
        bound: HashMap::new(),
        calls: Vec::new(),
        site: 0,
    };
    let defs_path = root.key("defs");
    let mut calls = Vec::with_capacity(program.defs.len());
    for def in &program.defs {
        checking.unbind(0);
        for &arg in &def.args {
            checking.bind(arg);
        }
        checking.calls.clear();
        let body_path = defs_path.key(def.name);
        checking.expression(def.body, &body_path.index(1))?;
        calls.push(std::mem::take(&mut checking.calls));
    }
    checking.unbind(0);
    checking.expression(program.step, &root.key("step"))?;
    unlooped(program, &calls, json)
}

fn unlooped(
    program: &Program<'_>,
    calls: &[Vec<(usize, usize)>],
    json: &Json,
) -> Result<(), Refusal> {
    let mut state = vec![0_u8; calls.len()];
    for start in 0..calls.len() {
        if state.get(start).copied() != Some(0) {
            continue;
        }
        let mut stack: Vec<(usize, usize)> = vec![(start, 0)];
        if let Some(slot) = state.get_mut(start) {
            *slot = 1;
        }
        while let Some(top) = stack.last_mut() {
            let (def, next) = *top;
            match calls.get(def).and_then(|out| out.get(next)) {
                Some((callee, site)) => {
                    top.1 += 1;
                    match state.get(*callee).copied() {
                        Some(0) => {
                            if let Some(slot) = state.get_mut(*callee) {
                                *slot = 1;
                            }
                            stack.push((*callee, 0));
                        }
                        Some(1) => {
                            let cycle: Vec<&str> = stack
                                .iter()
                                .skip_while(|(held, _)| held != callee)
                                .filter_map(|(held, _)| program.defs.get(*held).map(|one| one.name))
                                .chain(program.defs.get(*callee).map(|one| one.name))
                                .collect();
                            return Err(Refusal {
                                code: Code::Recursion,
                                path: Places::of(json, &[*site])
                                    .path(0)
                                    .unwrap_or_else(|| String::from("$.defs")),
                                expected: String::from("defs that do not call themselves"),
                                found: clipped(&cycle.join(" -> ")),
                                limit: None,
                            });
                        }
                        _ => {}
                    }
                }
                None => {
                    if let Some(slot) = state.get_mut(def) {
                        *slot = 2;
                    }
                    stack.pop();
                }
            }
        }
    }
    Ok(())
}
