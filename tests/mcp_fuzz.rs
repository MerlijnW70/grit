#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::Once;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use common::device;

use grit::rule3::heat;
use grit::sand3::Grid3;
use grit::serve3::{Server, rules_on};
use grit::wire::{Json, read, written};

const LINE_MOST: usize = 1 << 20;

const ANSWER_MOST: usize = 1 << 20;

const ID_MOST: usize = 256;

const SEED: u64 = 0x6D31_F022;

struct Counting;

thread_local! {
    static HELD: Cell<i64> = const { Cell::new(0) };
    static PEAK: Cell<i64> = const { Cell::new(0) };
    static FUZZING: Cell<bool> = const { Cell::new(false) };
}

fn moved(bytes: i64) {
    let _ = HELD.try_with(|held| {
        let now = held.get() + bytes;
        held.set(now);
        let _ = PEAK.try_with(|peak| {
            if now > peak.get() {
                peak.set(now);
            }
        });
    });
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        moved(layout.size() as i64);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        moved(-(layout.size() as i64));
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        moved(layout.size() as i64);
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        moved(size as i64 - layout.size() as i64);
        unsafe { System.realloc(ptr, layout, size) }
    }
}

#[global_allocator]
static COUNTING: Counting = Counting;

static FAULTS: AtomicU64 = AtomicU64::new(0);

static HOOKED: Once = Once::new();

fn hooked() {
    HOOKED.call_once(|| {
        let before = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if FUZZING.with(Cell::get) {
                FAULTS.fetch_add(1, Ordering::SeqCst);
            }
            before(info);
        }));
    });
}

struct Rng(u64);

impl Rng {
    fn of(seed: u64, line: u64) -> Self {
        let mut rng = Self(seed ^ line.wrapping_mul(0xD1B5_4A32_D192_ED03));
        rng.next();
        rng
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut mixed = self.0;
        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        mixed ^ (mixed >> 31)
    }

    fn below(&mut self, most: u64) -> u64 {
        if most == 0 { 0 } else { self.next() % most }
    }

    fn chance(&mut self, out_of: u64) -> bool {
        self.below(out_of) == 0
    }

    fn pick<'held, T>(&mut self, from: &'held [T]) -> &'held T {
        &from[self.below(from.len() as u64) as usize]
    }
}

const TOOLS: [&str; 17] = [
    "grit_world_rules",
    "grit_world_create",
    "grit_world_write",
    "grit_world_step",
    "grit_world_read",
    "grit_world_slice",
    "grit_world_close",
    "grit_describe",
    "grit_check",
    "grit_machine",
    "grit_synthesise",
    "grit_least",
    "grit_slope",
    "grit_settle",
    "grit_pour",
    "grit_world_explode",
    "",
];

const EXTREMES: [&str; 14] = [
    "0",
    "-1",
    "1",
    "2",
    "4294967295",
    "4294967296",
    "65537",
    "9223372036854775807",
    "-9223372036854775808",
    "99999999999999999999",
    "1.5",
    "1e3",
    "-0",
    "00",
];

fn id(rng: &mut Rng) -> String {
    match rng.below(10) {
        0 => format!("\"{}\"", "r".repeat(rng.below(300) as usize)),
        1 => "null".to_owned(),
        2 => (*rng.pick(&["-1", "9223372036854775807", "{}", "[1]", "true", "1.5"])).to_owned(),
        _ => rng.below(5).to_string(),
    }
}

fn point(rng: &mut Rng, side: u32) -> String {
    let at = |rng: &mut Rng| {
        if rng.chance(30) {
            side + rng.below(3) as u32
        } else {
            rng.below(u64::from(side)) as u32
        }
    };
    format!("[{},{},{}]", at(rng), at(rng), at(rng))
}

fn arguments(
    rng: &mut Rng,
    tool: &str,
    world: u32,
    word: &mut dyn FnMut(&mut Rng) -> u32,
) -> String {
    match tool {
        "grit_world_create" => {
            let rule = *rng.pick(&["sand", "heat", "nature", "code", "embed", "lava"]);
            let params = match rule {
                "heat" => "[600,4,4096]",
                "nature" | "embed" => "[0]",
                _ => "[]",
            };
            let side = |rng: &mut Rng| 2 << rng.below(3);
            format!(
                "{{\"wide\":{},\"deep\":{},\"tall\":{},\"rule\":\"{rule}\",\"params\":{params}{}}}",
                side(rng),
                side(rng),
                side(rng),
                if rng.chance(3) {
                    ",\"sparse\":true"
                } else {
                    ""
                }
            )
        }
        "grit_world_write" => {
            let cells: Vec<String> = (0..=rng.below(8))
                .map(|_| {
                    let at = point(rng, 16);
                    format!("[{},{}]", &at[1..at.len() - 1], word(rng))
                })
                .collect();
            format!("{{\"world\":{world},\"cells\":[{}]}}", cells.join(","))
        }
        "grit_world_step" => {
            if rng.chance(4) {
                format!("{{\"world\":{world}}}")
            } else {
                format!("{{\"world\":{world},\"frames\":{}}}", 1 + rng.below(4))
            }
        }
        "grit_world_read" => {
            let from: Vec<u64> = (0..3).map(|_| rng.below(16)).collect();
            let to: Vec<u64> = from
                .iter()
                .map(|one| (one + rng.below(8)).min(15))
                .collect();
            format!(
                "{{\"world\":{world},\"from\":[{},{},{}],\"to\":[{},{},{}]}}",
                from[0], from[1], from[2], to[0], to[1], to[2]
            )
        }
        "grit_world_slice" => format!(
            "{{\"world\":{world},\"axis\":\"{}\",\"at\":{},\"as\":\"{}\"{}}}",
            rng.pick(&["x", "y", "z"]),
            rng.below(17),
            rng.pick(&["kinds", "words"]),
            if rng.chance(4) {
                format!(",\"cursor\":{}", rng.below(17))
            } else {
                String::new()
            }
        ),
        "grit_world_close" => format!("{{\"world\":{world}}}"),
        "grit_check" => format!(
            "{{\"inputs\":2,\"outputs\":1,\"gates\":1,\"table\":[0,0,0,{}],\"chip\":[{{\"kind\":{},\"left\":0,\"right\":1}}]}}",
            rng.below(2),
            rng.below(16)
        ),
        "grit_machine" => format!(
            "{{\"state\":1,\"inputs\":1,\"outputs\":1,\"gates\":2,\"traces\":[{{\"feed\":[1,0],\"want\":[1,{}]}}],\"chip\":[{{\"kind\":{},\"left\":0,\"right\":1}},{{\"kind\":3,\"left\":1,\"right\":2}}]}}",
            rng.below(2),
            rng.below(16)
        ),
        "grit_synthesise" | "grit_least" => {
            "{\"inputs\":2,\"outputs\":1,\"gates\":2,\"table\":[0,1,1,0],\"crowd\":64,\"rounds\":2}"
                .to_owned()
        }
        "grit_slope" | "grit_settle" => {
            "{\"wide\":8,\"deep\":8,\"depth\":2,\"want\":[{\"at\":3,\"is\":1}]}".to_owned()
        }
        "grit_pour" => "{\"wide\":16,\"deep\":16,\"frames\":4}".to_owned(),
        _ => "{}".to_owned(),
    }
}

fn valid(
    rng: &mut Rng,
    world: u32,
    word: &mut dyn FnMut(&mut Rng) -> u32,
    tools: &[&str],
) -> String {
    let id = id(rng);
    match rng.below(20) {
        0 => format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"initialize\",\"params\":{{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{{}},\"clientInfo\":{{\"name\":\"fuzz\",\"version\":\"0\"}}}}}}"
        ),
        1 => format!("{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"tools/list\"}}"),
        2 => format!("{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"ping\"}}"),
        3 => "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}".to_owned(),
        4 => format!(
            "{{\"jsonrpc\":\"2.0\",\"method\":\"notifications/cancelled\",\"params\":{{\"requestId\":{id}}}}}"
        ),
        5 => format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"{}\"}}",
            rng.pick(&["resources/list", "prompts/list", "tools/dream", ""])
        ),
        _ => {
            let tool = *rng.pick(tools);
            let given = arguments(rng, tool, world, word);
            let meta = if rng.chance(10) {
                ",\"_meta\":{\"progressToken\":1}"
            } else {
                ""
            };
            format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"tools/call\",\"params\":{{\"name\":\"{tool}\",\"arguments\":{given}{meta}}}}}"
            )
        }
    }
}

fn spot(rng: &mut Rng, line: &str) -> usize {
    let mut at = rng.below(line.len() as u64 + 1) as usize;
    while !line.is_char_boundary(at) {
        at -= 1;
    }
    at
}

fn mutated(rng: &mut Rng, mut line: String) -> String {
    for _ in 0..=rng.below(3) {
        match rng.below(12) {
            0 => {
                let from = spot(rng, &line);
                let mut to = (from + 1 + rng.below(10) as usize).min(line.len());
                while !line.is_char_boundary(to) {
                    to += 1;
                }
                line.replace_range(from..to, "");
            }
            1 => {
                let at = spot(rng, &line);
                let one = *rng.pick(&[
                    "{",
                    "}",
                    "[",
                    "]",
                    "\"",
                    ",",
                    ":",
                    "0",
                    "-",
                    "a",
                    "\\",
                    " ",
                    "\u{0}",
                    "\u{ff}",
                    "\u{1F600}",
                    "\t",
                ]);
                line.insert_str(at, one);
            }
            2 => {
                let from = spot(rng, &line);
                let to = spot(rng, &line).max(from);
                let copy = line[from..to].to_owned();
                let at = spot(rng, &line);
                line.insert_str(at, &copy);
            }
            3 => {
                let digits: Vec<usize> = line
                    .char_indices()
                    .filter(|(_, one)| one.is_ascii_digit())
                    .map(|(at, _)| at)
                    .collect();
                if !digits.is_empty() {
                    let from = *rng.pick(&digits);
                    let to = line[from..]
                        .find(|one: char| !one.is_ascii_digit())
                        .map_or(line.len(), |end| from + end);
                    let extreme: &&str = rng.pick(&EXTREMES);
                    line.replace_range(from..to, extreme);
                }
            }
            4 => {
                let keys: Vec<usize> = line.match_indices("\":").map(|(at, _)| at).collect();
                if !keys.is_empty() {
                    let at = *rng.pick(&keys);
                    if rng.chance(2) && at > 1 && line.is_char_boundary(at - 1) {
                        line.remove(at - 1);
                    } else {
                        line.insert(at, 's');
                    }
                }
            }
            5 => {
                if let Some(at) = line.find("\"arguments\":") {
                    let deep = 1 + rng.below(80) as usize;
                    let start = at + "\"arguments\":".len();
                    line.insert_str(start, &"[".repeat(deep));
                    let end = line.rfind('}').unwrap_or(line.len());
                    let end = line[..end].rfind('}').map_or(end, |one| one + 1);
                    line.insert_str(end.min(line.len()), &"]".repeat(deep));
                }
            }
            6 => {
                if let Some(at) = line.find("\"arguments\":{") {
                    let key = *rng.pick(&["world", "frames", "frame", "cells", "rule", "at", "zz"]);
                    line.insert_str(
                        at + "\"arguments\":{".len(),
                        &format!("\"{key}\":{},", rng.pick(&EXTREMES)),
                    );
                }
            }
            7 => {
                let long = if rng.chance(300) {
                    LINE_MOST + rng.below(4096) as usize - 2048
                } else {
                    rng.below(20_000) as usize
                };
                let fill = if rng.chance(2) {
                    format!("\"{}\"", "x".repeat(long))
                } else {
                    format!("[{}0]", "0,".repeat(long / 2))
                };
                match line.find("\"arguments\":{") {
                    Some(at) => {
                        line.insert_str(at + "\"arguments\":{".len(), &format!("\"long\":{fill},"))
                    }
                    None => line = format!("{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":{fill}}}"),
                }
            }
            8 => {
                if let Some(at) = line.find("\"id\":") {
                    line.insert_str(at, &format!("\"id\":{},", id(rng)));
                } else {
                    line = line.replacen('{', &format!("{{\"id\":{},", id(rng)), 1);
                }
            }
            9 => line = format!("[{line},{line}]"),
            10 => {
                let tail: &&str = rng.pick(&["", " ", "x", "{}", "\r", "]"]);
                line.push_str(tail);
            }
            _ => {
                if let Some(at) = line.find("\"id\":") {
                    let end = line[at..]
                        .find([',', '}'])
                        .map_or(line.len(), |one| at + one);
                    line.replace_range(at..end, "\"idx\":0");
                }
            }
        }
    }
    line
}

fn garbage(rng: &mut Rng) -> String {
    match rng.below(4) {
        0 => {
            let bytes: Vec<u8> = (0..rng.below(120))
                .map(|_| {
                    let one = rng.below(256) as u8;
                    if one == b'\n' { b' ' } else { one }
                })
                .collect();
            String::from_utf8_lossy(&bytes).into_owned()
        }
        1 => {
            let deep = 1 + rng.below(200) as usize;
            format!("{}{}", "[".repeat(deep), "]".repeat(deep))
        }
        2 => {
            let deep = 1 + rng.below(100) as usize;
            format!("{}1{}", "{\"a\":".repeat(deep), "}".repeat(deep))
        }
        _ => (0..rng.below(60))
            .map(|_| {
                *rng.pick(&[
                    "{",
                    "}",
                    "[",
                    "]",
                    "\"",
                    ",",
                    ":",
                    "1",
                    "true",
                    "null",
                    "\"id\"",
                    "\"method\"",
                    " ",
                ])
            })
            .collect(),
    }
}

fn free_line(seed: u64, index: u64) -> String {
    let mut rng = Rng::of(seed, index);
    let mut word = |rng: &mut Rng| rng.next() as u32;
    match rng.below(20) {
        0..2 => garbage(&mut rng),
        2..10 => {
            let world = 1 + rng.below(3) as u32;
            valid(&mut rng, world, &mut word, &TOOLS)
        }
        _ => {
            let line = valid(&mut rng, 1, &mut word, &TOOLS);
            mutated(&mut rng, line)
        }
    }
}

fn blank(line: &str) -> bool {
    line.bytes()
        .all(|one| matches!(one, b' ' | b'\t' | b'\n' | b'\r'))
}

fn end_of_value(held: &[u8]) -> usize {
    let space = |one: &u8| matches!(one, b' ' | b'\t' | b'\n' | b'\r');
    let mut at = 0;
    while held.get(at).is_some_and(space) {
        at += 1;
    }
    match held.get(at) {
        Some(b'{' | b'[') => {
            let mut deep = 0_usize;
            let mut quoted = false;
            while let Some(&one) = held.get(at) {
                at += 1;
                if quoted {
                    match one {
                        b'\\' => at += 1,
                        b'"' => quoted = false,
                        _ => {}
                    }
                    continue;
                }
                match one {
                    b'"' => quoted = true,
                    b'[' | b'{' => deep += 1,
                    b']' | b'}' => {
                        deep = deep.saturating_sub(1);
                        if deep == 0 {
                            return at;
                        }
                    }
                    _ => {}
                }
            }
            at
        }
        Some(b'"') => {
            at += 1;
            while let Some(&one) = held.get(at) {
                at += 1;
                match one {
                    b'\\' => at += 1,
                    b'"' => return at,
                    _ => {}
                }
            }
            at
        }
        _ => {
            while held.get(at).is_some_and(|one| {
                !space(one) && !matches!(one, b'[' | b']' | b'{' | b'}' | b'"' | b',' | b':')
            }) {
                at += 1;
            }
            at
        }
    }
}

#[derive(Debug, PartialEq)]
enum Expect {
    Nothing,
    Failed(i64),
    Answered(Json),
}

fn expected(line: &str) -> (Expect, Json) {
    if line.len() > LINE_MOST {
        return (Expect::Failed(-32600), Json::Null);
    }
    if blank(line) {
        return (Expect::Nothing, Json::Null);
    }
    let Ok(request) = read(line) else {
        return (Expect::Failed(-32700), Json::Null);
    };
    if !blank(&line[end_of_value(line.as_bytes())..]) {
        return (Expect::Failed(-32700), Json::Null);
    }
    let Json::Map(held) = &request else {
        return (Expect::Failed(-32600), Json::Null);
    };
    for key in ["jsonrpc", "id", "method", "params"] {
        if held.iter().filter(|(name, _)| name == key).count() > 1 {
            return (Expect::Failed(-32600), Json::Null);
        }
    }
    let id = request.get("id");
    if let Some(id) = id
        && !match id {
            Json::Int(_) | Json::Null => true,
            Json::Text(one) => one.len() <= ID_MOST,
            _ => false,
        }
    {
        return (Expect::Failed(-32600), Json::Null);
    }
    if request.get("method").and_then(Json::text).is_none() {
        return (Expect::Failed(-32600), id.cloned().unwrap_or(Json::Null));
    }
    match id {
        None => (Expect::Nothing, Json::Null),
        Some(id) => (Expect::Answered(id.clone()), id.clone()),
    }
}

fn checked(line: &str, got: Option<&Json>, codes: &[String]) -> Result<(), String> {
    let (want, id) = expected(line);
    let Some(got) = got else {
        return if want == Expect::Nothing {
            Ok(())
        } else {
            Err(format!("no answer where {want:?} was due"))
        };
    };
    if want == Expect::Nothing {
        return Err(format!(
            "an answer where none was due: {}",
            short(&written(got))
        ));
    }
    let text = written(got);
    if text.len() > ANSWER_MOST {
        return Err(format!("an answer of {} bytes", text.len()));
    }
    if read(&text).as_ref() != Ok(got) {
        return Err("the answer does not read back as itself".to_owned());
    }
    if got.get("jsonrpc").and_then(Json::text) != Some("2.0") {
        return Err(format!("no jsonrpc 2.0: {}", short(&text)));
    }
    if got.get("id") != Some(&id) {
        return Err(format!(
            "the answer carries id {:?} where {} was due",
            got.get("id").map(written),
            written(&id)
        ));
    }
    let named = |data: Option<&Json>| -> Result<(), String> {
        let data = data.ok_or_else(|| format!("no structured refusal: {}", short(&text)))?;
        let code = data.get("code").and_then(Json::text).unwrap_or("");
        if !codes.is_empty() && !codes.iter().any(|one| one == code) {
            return Err(format!("code {code:?} is not in the described list"));
        }
        if !data
            .get("field")
            .and_then(Json::text)
            .is_some_and(|one| one.starts_with('$'))
        {
            return Err(format!("no field: {}", short(&text)));
        }
        if data.get("hint").and_then(Json::text).is_none() {
            return Err(format!("no hint: {}", short(&text)));
        }
        Ok(())
    };
    match (got.get("result"), got.get("error")) {
        (Some(_), Some(_)) | (None, None) => {
            Err(format!("not one of result and error: {}", short(&text)))
        }
        (None, Some(error)) => {
            if let Expect::Failed(code) = want
                && error.get("code").and_then(Json::int) != Some(code)
            {
                return Err(format!(
                    "error {:?} where {code} was due",
                    error.get("code")
                ));
            }
            named(error.get("data"))
        }
        (Some(result), None) => {
            if let Expect::Failed(code) = want {
                return Err(format!(
                    "a result where error {code} was due: {}",
                    short(&text)
                ));
            }
            if result.get("isError") == Some(&Json::Bool(true)) {
                named(result.get("structuredContent"))?;
            }
            Ok(())
        }
    }
}

fn short(text: &str) -> String {
    text.chars().take(300).collect()
}

fn seed() -> u64 {
    std::env::var("GRIT_FUZZ_SEED").ok().map_or(SEED, |held| {
        let held = held.trim();
        held.strip_prefix("0x").map_or_else(
            || held.parse().unwrap(),
            |hex| u64::from_str_radix(hex, 16).unwrap(),
        )
    })
}

fn codes_of(server: &mut Server<'_>) -> Vec<String> {
    let got = server
        .heard("{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"grit_describe\",\"arguments\":{}}}")
        .unwrap();
    got.get("result")
        .and_then(|one| one.get("structuredContent"))
        .and_then(|one| one.get("codes"))
        .and_then(Json::list)
        .map(|held| {
            held.iter()
                .filter_map(|one| one.get("code").and_then(Json::text).map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn caught(server: &mut Server<'_>) -> Option<i64> {
    server
        .heard("{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"grit_describe\",\"arguments\":{}}}")
        .and_then(|got| {
            got.get("result")
                .and_then(|one| one.get("structuredContent"))
                .and_then(|one| one.get("faults_caught"))
                .and_then(Json::int)
        })
}

fn heard(server: &mut Server<'_>, line: &str) -> Option<Json> {
    FUZZING.with(|one| one.set(true));
    let got = server.heard(line);
    FUZZING.with(|one| one.set(false));
    got
}

#[test]
fn a_line_past_the_cap_is_refused_and_the_next_line_answers() {
    let mut server = Server::new(None);
    let shaped = |bytes: usize| {
        let head = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\",\"x\":\"";
        format!("{head}{}\"}}", "a".repeat(bytes - head.len() - 2))
    };
    let at = shaped(LINE_MOST);
    assert_eq!(at.len(), LINE_MOST);
    let got = server.heard(&at).unwrap();
    assert!(got.get("result").is_some(), "{}", short(&written(&got)));
    let past = shaped(LINE_MOST + 1);
    let got = server.heard(&past).unwrap();
    assert_eq!(
        got.get("id"),
        Some(&Json::Null),
        "{}",
        short(&written(&got))
    );
    assert_eq!(
        got.get("error")
            .and_then(|one| one.get("code"))
            .and_then(Json::int),
        Some(-32600)
    );
    let mut child = Command::new(env!("CARGO_BIN_EXE_grit-mcp"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let feeding = std::thread::spawn(move || {
        input.write_all(shaped(3 * LINE_MOST).as_bytes()).unwrap();
        input.write_all(b"\n").unwrap();
        input
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}\n")
            .unwrap();
        input
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"ping\",\"x\":\"\xff\xfe\"}\n\n")
            .unwrap();
        input.write_all(shaped(LINE_MOST + 1).as_bytes()).unwrap();
    });
    let answers: Vec<Json> = BufReader::new(child.stdout.take().unwrap())
        .lines()
        .map(|one| read(&one.unwrap()).unwrap())
        .collect();
    feeding.join().unwrap();
    assert!(child.wait().unwrap().success());
    let shown: Vec<String> = answers.iter().map(|one| short(&written(one))).collect();
    assert_eq!(answers.len(), 4, "{shown:?}");
    for at in [0, 3] {
        assert_eq!(answers[at].get("id"), Some(&Json::Null), "{shown:?}");
        assert_eq!(
            answers[at]
                .get("error")
                .and_then(|one| one.get("code"))
                .and_then(Json::int),
            Some(-32600),
            "{shown:?}"
        );
    }
    assert_eq!(
        written(&answers[1]),
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{}}"
    );
    assert_eq!(
        written(&answers[2]),
        "{\"jsonrpc\":\"2.0\",\"id\":3,\"result\":{}}"
    );
}

#[test]
fn the_device_free_fuzz_answers_every_line_once_and_holds_its_memory() {
    hooked();
    let seed = seed();
    let mut server = Server::new(None);
    let codes = codes_of(&mut server);
    let lines: u64 = std::env::var("GRIT_FUZZ_LINES")
        .ok()
        .map_or(100_000, |held| held.parse().unwrap());
    let only: Option<u64> = std::env::var("GRIT_FUZZ_LINE")
        .ok()
        .map(|held| held.parse().unwrap());
    let started = Instant::now();
    let before = HELD.with(Cell::get);
    PEAK.with(|peak| peak.set(before));
    let mut settled = before;
    let mut bytes = 0_usize;
    let mut answered = 0_u64;
    for index in only.map_or(0..lines, |one| one..one + 1) {
        let line = free_line(seed, index);
        bytes += line.len();
        let got = heard(&mut server, &line);
        answered += u64::from(got.is_some());
        if let Err(why) = checked(&line, got.as_ref(), &codes) {
            panic!(
                "seed {seed:#x} line {index}: {why}\nline: {}\nreplay alone with GRIT_FUZZ_SEED={seed:#x} GRIT_FUZZ_LINE={index}",
                short(&line)
            );
        }
        drop(got);
        drop(line);
        if index == 1000 {
            settled = HELD.with(Cell::get);
        }
        assert_eq!(
            FAULTS.load(Ordering::SeqCst),
            0,
            "seed {seed:#x} line {index}: the server panicked; replay alone with GRIT_FUZZ_SEED={seed:#x} GRIT_FUZZ_LINE={index}"
        );
    }
    let peak = PEAK.with(Cell::get) - before;
    let after = HELD.with(Cell::get);
    let spent = started.elapsed();
    println!(
        "fuzz seed {seed:#x}: {lines} lines, {bytes} bytes, {answered} answered in {:.1} s; peak {peak} bytes above the start, {} bytes held after line 1000 and {} at the end",
        spent.as_secs_f64(),
        settled - before,
        after - before
    );
    assert!(peak <= 40 * LINE_MOST as i64, "{peak}");
    assert_eq!(caught(&mut server), Some(0));
    if only.is_none() {
        assert!(
            after - settled <= 64 << 10,
            "{} bytes more held",
            after - settled
        );
    }
}

fn heat_word(rng: &mut Rng) -> u32 {
    let roll = rng.next();
    let kind = match roll % 32 {
        0..10 => heat::AIR,
        10..12 => heat::WALL,
        12..15 => heat::ICE,
        15..19 => heat::WATER,
        19..22 => heat::STEAM,
        22..26 => heat::FUEL,
        26..28 => heat::FIRE,
        28..31 => heat::ASH,
        _ => heat::KINDS + ((roll >> 40) % 8) as u32,
    };
    let temperature = ((roll >> 8) % 1200) as u32;
    let life = ((roll >> 20) % 16) as u32;
    let kept = ((roll >> 24) & 0xFF) as u32;
    heat::cell(kind, temperature) | (life << 4) | (kept << 8)
}

struct Kept {
    rule: String,
    params: Vec<u32>,
    grid: Grid3,
    cells: Vec<u32>,
    frame: u32,
    open: bool,
}

fn whole(held: &Json) -> Vec<u32> {
    held.list()
        .unwrap()
        .iter()
        .map(|one| u32::try_from(one.int().unwrap()).unwrap())
        .collect()
}

#[test]
fn the_device_fuzz_keeps_every_world_equal_to_its_replay() {
    let Some(host) = device("device fuzz") else {
        return;
    };
    hooked();
    let seed = seed();
    let rules = rules_on(Some(&host));
    let mut server = Server::new(Some(&host));
    let codes = codes_of(&mut server);
    let mut kept: Vec<(u32, Kept)> = Vec::new();
    for (rule, params, sparse) in [
        ("sand", "[]", false),
        ("heat", "[600,4,4096]", true),
        ("nature", "[0]", false),
    ] {
        let line = format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{{\"name\":\"grit_world_create\",\"arguments\":{{\"wide\":16,\"deep\":16,\"tall\":16,\"rule\":\"{rule}\",\"params\":{params},\"sparse\":{sparse}}}}}}}"
        );
        let got = server.heard(&line).unwrap();
        let told = got
            .get("result")
            .and_then(|one| one.get("structuredContent"))
            .unwrap();
        kept.push((
            told.counted("world").unwrap(),
            Kept {
                rule: rule.to_owned(),
                params: whole(&read(params).unwrap()),
                grid: Grid3::new(16, 16, 16).unwrap(),
                cells: vec![0; 4096],
                frame: 0,
                open: true,
            },
        ));
    }
    let lines: u64 = std::env::var("GRIT_FUZZ_DEVICE_LINES")
        .ok()
        .map_or(2000, |held| held.parse().unwrap());
    let tools = [
        "grit_world_write",
        "grit_world_write",
        "grit_world_write",
        "grit_world_step",
        "grit_world_step",
        "grit_world_step",
        "grit_world_read",
        "grit_world_slice",
    ];
    let rare = [
        "grit_world_rules",
        "grit_describe",
        "grit_world_create",
        "grit_world_close",
    ];
    let started = Instant::now();
    let mut changed = 0;
    for index in 0..lines {
        let mut rng = Rng::of(seed ^ 0xDE71CE, index);
        let world = if rng.chance(20) {
            4 + rng.below(3) as u32
        } else {
            1 + rng.below(3) as u32
        };
        let heat_world = kept
            .iter()
            .any(|(id, one)| *id == world && one.rule == "heat");
        let mut word = |rng: &mut Rng| {
            if heat_world {
                heat_word(rng)
            } else {
                rng.next() as u32
            }
        };
        let line = match rng.below(20) {
            0 => garbage(&mut rng),
            1 if rng.chance(10) => {
                let tool = *rng.pick(&rare);
                if tool == "grit_world_close" && !rng.chance(8) {
                    valid(&mut rng, world, &mut word, &["grit_world_rules"])
                } else {
                    valid(&mut rng, world, &mut word, &[tool])
                }
            }
            1..6 => {
                let line = valid(&mut rng, world, &mut word, &tools);
                mutated(&mut rng, line)
            }
            _ => valid(&mut rng, world, &mut word, &tools),
        };
        let got = heard(&mut server, &line);
        if let Err(why) = checked(&line, got.as_ref(), &codes) {
            panic!(
                "seed {seed:#x} device line {index}: {why}\nline: {}\nreplay with GRIT_FUZZ_SEED={seed:#x}",
                short(&line)
            );
        }
        assert_eq!(
            FAULTS.load(Ordering::SeqCst),
            0,
            "seed {seed:#x} device line {index}: the server panicked"
        );
        let Some(result) = got.as_ref().and_then(|one| one.get("result")) else {
            continue;
        };
        if result.get("isError") != Some(&Json::Bool(false)) {
            continue;
        }
        let Some(told) = result.get("structuredContent") else {
            continue;
        };
        let request = read(&line).unwrap();
        let params = request.get("params").unwrap();
        let name = params.get("name").and_then(Json::text).unwrap();
        let empty = Json::Map(Vec::new());
        let arguments = params.get("arguments").unwrap_or(&empty);
        let mine = |kept: &mut Vec<(u32, Kept)>| {
            let world = arguments.counted("world").unwrap();
            kept.iter_mut().position(|(id, _)| *id == world).unwrap()
        };
        match name {
            "grit_world_create" => {
                let size = whole(told.get("size").unwrap());
                let grid = Grid3::new(size[0], size[1], size[2]).unwrap();
                kept.push((
                    told.counted("world").unwrap(),
                    Kept {
                        rule: told.get("rule").and_then(Json::text).unwrap().to_owned(),
                        params: whole(told.get("params").unwrap()),
                        grid,
                        cells: vec![0; grid.cells()],
                        frame: 0,
                        open: true,
                    },
                ));
            }
            "grit_world_write" => {
                let at = mine(&mut kept);
                let one = &mut kept[at].1;
                for cell in arguments.get("cells").and_then(Json::list).unwrap() {
                    let cell = whole(cell);
                    let place = one.grid.index(cell[0], cell[1], cell[2]);
                    one.cells[place] = cell[3];
                }
                changed += 1;
            }
            "grit_world_step" => {
                let at = mine(&mut kept);
                let one = &mut kept[at].1;
                let rule = rules.iter().find(|rule| rule.name() == one.rule).unwrap();
                let frames = arguments.counted("frames").unwrap_or(1);
                for _ in 0..frames {
                    one.cells = rule.reference(one.grid, one.frame, &one.cells, &one.params);
                    one.frame += 1;
                }
                assert_eq!(
                    told.counted("frame"),
                    Some(one.frame),
                    "device line {index}"
                );
                changed += 1;
            }
            "grit_world_close" => {
                let at = mine(&mut kept);
                kept[at].1.open = false;
            }
            _ => {}
        }
    }
    assert_eq!(caught(&mut server), Some(0));
    let mut compared = 0;
    for (world, one) in kept.iter().filter(|(_, one)| one.open) {
        let mut cells = Vec::new();
        let layers = (4096 / (one.grid.wide * one.grid.deep)).max(1);
        for from in (0..one.grid.tall).step_by(layers as usize) {
            let to = (from + layers).min(one.grid.tall) - 1;
            let got = server
                .heard(&format!(
                    "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{{\"name\":\"grit_world_read\",\"arguments\":{{\"world\":{world},\"from\":[0,0,{from}],\"to\":[{},{},{to}]}}}}}}",
                    one.grid.wide - 1,
                    one.grid.deep - 1
                ))
                .unwrap();
            let told = got
                .get("result")
                .and_then(|one| one.get("structuredContent"))
                .unwrap();
            assert_eq!(told.counted("frame"), Some(one.frame), "world {world}");
            cells.extend(whole(told.get("cells").unwrap()));
        }
        assert_eq!(
            cells, one.cells,
            "seed {seed:#x}: world {world} under {} left its CPU replay",
            one.rule
        );
        compared += 1;
    }
    println!(
        "device fuzz seed {seed:#x} on {} width {}: {lines} lines, {changed} writes and steps, {compared} worlds equal to their replay, {:.1} s",
        host.name(),
        host.width(),
        started.elapsed().as_secs_f64()
    );
    assert!(
        compared >= 1 && changed > lines / 10,
        "{compared} {changed}"
    );
}
