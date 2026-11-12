#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use grit::serve3::Server;
use grit::wire::{Json, read, written};

const ANSWER_MOST: usize = 1 << 20;

const FLAT: [&str; 7] = [
    "grit_machine",
    "grit_synthesise",
    "grit_least",
    "grit_slope",
    "grit_settle",
    "grit_pour",
    "grit_check",
];

fn request(id: &str, method: &str, params: &str) -> String {
    format!("{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"{method}\"{params}}}")
}

fn calling(name: &str, arguments: &str) -> String {
    request(
        "7",
        "tools/call",
        &format!(",\"params\":{{\"name\":\"{name}\",\"arguments\":{arguments}}}"),
    )
}

fn call(server: &mut Server<'_>, name: &str, arguments: &str) -> Json {
    server.heard(&calling(name, arguments)).unwrap()
}

fn answer(got: &Json) -> Json {
    let result = got.get("result").unwrap();
    assert_eq!(
        result.get("isError"),
        Some(&Json::Bool(false)),
        "{}",
        written(got)
    );
    result.get("structuredContent").cloned().unwrap()
}

struct Told {
    code: String,
    field: String,
    limit: Option<Json>,
    message: String,
}

fn refused(got: &Json) -> Told {
    let result = got
        .get("result")
        .unwrap_or_else(|| panic!("{}", written(got)));
    assert_eq!(
        result.get("isError"),
        Some(&Json::Bool(true)),
        "{}",
        written(got)
    );
    let told = result
        .get("structuredContent")
        .unwrap_or_else(|| panic!("no structured refusal: {}", written(got)));
    let code = told
        .get("code")
        .and_then(Json::text)
        .unwrap_or_else(|| panic!("no code: {}", written(got)))
        .to_owned();
    assert_eq!(
        told.get("refused").and_then(Json::text),
        Some(code.as_str())
    );
    let field = told
        .get("field")
        .and_then(Json::text)
        .unwrap_or_else(|| panic!("no field: {}", written(got)))
        .to_owned();
    assert!(field.starts_with('$'), "{field}");
    let hint = told.get("hint").and_then(Json::text).unwrap();
    assert!(hint.ends_with('.') && !hint.is_empty(), "{hint}");
    Told {
        code,
        field,
        limit: told.get("limit").cloned(),
        message: told.get("message").and_then(Json::text).unwrap().to_owned(),
    }
}

fn failed(got: &Json) -> (i64, String, Json) {
    let error = got
        .get("error")
        .unwrap_or_else(|| panic!("no error: {}", written(got)));
    let code = error.get("code").and_then(Json::int).unwrap();
    let data = error
        .get("data")
        .unwrap_or_else(|| panic!("no data: {}", written(got)));
    let named = data.get("code").and_then(Json::text).unwrap().to_owned();
    assert!(
        data.get("field")
            .and_then(Json::text)
            .is_some_and(|one| one.starts_with('$'))
    );
    assert!(data.get("hint").and_then(Json::text).is_some());
    (code, named, got.get("id").cloned().unwrap())
}

fn described(server: &mut Server<'_>) -> Json {
    answer(&call(server, "grit_describe", "{}"))
}

fn listed(server: &mut Server<'_>) -> Vec<Json> {
    let got = server.heard(&request("1", "tools/list", "")).unwrap();
    got.get("result")
        .and_then(|one| one.get("tools"))
        .and_then(Json::list)
        .unwrap()
        .to_vec()
}

#[test]
fn a_misspelt_argument_is_refused_by_name() {
    let mut server = Server::new(None);
    let told = refused(&call(
        &mut server,
        "grit_world_step",
        "{\"world\":1,\"frame\":100}",
    ));
    assert_eq!(told.code, "unknown_key");
    assert_eq!(told.field, "$.frame");
    assert!(told.message.contains("frames"), "{}", told.message);
    for (tool, arguments, field) in [
        ("grit_world_rules", "{\"rule\":\"sand\"}", "$.rule"),
        (
            "grit_world_create",
            "{\"wide\":8,\"deep\":8,\"tall\":8,\"rule\":\"sand\",\"spares\":true}",
            "$.spares",
        ),
        (
            "grit_world_read",
            "{\"world\":1,\"from\":[0,0,0],\"to\":[1,1,1],\"as\":\"words\"}",
            "$.as",
        ),
        (
            "grit_world_close",
            "{\"world\":1,\"force\":true}",
            "$.force",
        ),
        ("grit_describe", "{\"verbose\":true}", "$.verbose"),
    ] {
        let told = refused(&call(&mut server, tool, arguments));
        assert_eq!(
            (told.code.as_str(), told.field.as_str()),
            ("unknown_key", field)
        );
    }
}

#[test]
fn a_misspelt_argument_to_a_2d_tool_is_refused_by_name() {
    let mut server = Server::new(None);
    let told = refused(&call(
        &mut server,
        "grit_check",
        "{\"inputs\":2,\"outputs\":1,\"gates\":1,\"tabel\":[0,0,0,1],\"chip\":[{\"kind\":8,\"left\":0,\"right\":1}]}",
    ));
    assert_eq!(
        (told.code.as_str(), told.field.as_str()),
        ("unknown_key", "$.tabel")
    );
    assert!(told.message.contains("table"), "{}", told.message);
    let told = refused(&call(
        &mut server,
        "grit_machine",
        "{\"state\":1,\"inputs\":1,\"outputs\":1,\"gates\":2,\"traces\":[{\"feed\":[1],\"want\":[1]},{\"fed\":[0],\"want\":[0]}]}",
    ));
    assert_eq!(
        (told.code.as_str(), told.field.as_str()),
        ("unknown_key", "$.traces[1].fed")
    );
    let alone = grit::serve::heard(
        None,
        &calling("grit_pour", "{\"wide\":16,\"deep\":16,\"wals\":false}"),
    )
    .unwrap();
    let told = refused(&alone);
    assert_eq!(
        (told.code.as_str(), told.field.as_str()),
        ("unknown_key", "$.wals")
    );
    for (tool, arguments, field) in [
        (
            "grit_pour",
            "{\"cells\":[{\"at\":1,\"is\":\"sand\"},{\"at\":1,\"iss\":\"water\"}]}",
            "$.cells[1].iss",
        ),
        (
            "grit_pour",
            "{\"brush\":{\"at\":3,\"spn\":4}}",
            "$.brush.spn",
        ),
        (
            "grit_check",
            "{\"inputs\":2,\"outputs\":1,\"gates\":1,\"table\":[0,0,0,1],\"chip\":[{\"kind\":8,\"left\":0,\"rigth\":1}]}",
            "$.chip[0].rigth",
        ),
        (
            "grit_slope",
            "{\"cells\":[{\"at\":3,\"was\":1}]}",
            "$.cells[0].was",
        ),
        (
            "grit_settle",
            "{\"want\":[{\"at\":3,\"iz\":1}]}",
            "$.want[0].iz",
        ),
    ] {
        let told = refused(&call(&mut server, tool, arguments));
        assert_eq!(
            (told.code.as_str(), told.field.as_str()),
            ("unknown_key", field),
            "{tool}"
        );
    }
}

#[test]
fn a_key_given_twice_is_refused_rather_than_read_once() {
    let mut server = Server::new(None);
    for line in [
        calling(
            "grit_world_step",
            "{\"world\":1,\"frames\":1,\"frames\":100}",
        ),
        calling(
            "grit_pour",
            "{\"cells\":[{\"at\":1,\"is\":\"sand\",\"is\":\"water\"}]}",
        ),
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"id\":2,\"method\":\"ping\"}".to_owned(),
    ] {
        let got = server.heard(&line).unwrap();
        let (code, named, id) = failed(&got);
        assert_eq!(
            (code, named.as_str(), id),
            (-32700, "parse", Json::Null),
            "{line}"
        );
        let message = got
            .get("error")
            .and_then(|one| one.get("message"))
            .and_then(Json::text)
            .unwrap();
        let at = line
            .rfind("\"frames\"")
            .or_else(|| line.rfind("\"is\""))
            .or_else(|| line.rfind("\"id\""))
            .unwrap();
        assert!(message.ends_with(&format!("{at} '\"'")), "{message}");
    }
}

#[test]
fn the_reader_takes_one_value_once_keyed_with_json_numbers() {
    for (held, whole) in [
        ("{\"a\":1} junk", false),
        ("{\"a\":1}}", false),
        ("{\"a\":1} {\"a\":1}", false),
        ("[1] ,", false),
        ("{\"a\":1} \t\r\n", true),
        ("{\"a\":1,\"a\":2}", false),
        ("{\"a\":{\"b\":1,\"b\":1}}", false),
        ("[{\"a\":1},{\"a\":1}]", true),
        ("{\"a\":1,\"A\":2}", true),
        ("+5", false),
        ("007", false),
        ("-01", false),
        ("00", false),
        ("-", false),
        ("1.5", false),
        ("1e3", false),
        ("5-", false),
        ("0", true),
        ("-0", true),
        ("-7", true),
        ("9223372036854775807", true),
        ("-9223372036854775808", true),
        ("9223372036854775808", false),
    ] {
        assert_eq!(read(held).is_ok(), whole, "{held}");
    }
    assert_eq!(read("[0,-0,10,-7]").unwrap(), read("[0,0,10,-7]").unwrap());
}

#[test]
fn arguments_that_are_not_an_object_are_refused() {
    let mut server = Server::new(None);
    for arguments in ["[1]", "5", "\"rules\"", "null", "true"] {
        let told = refused(&call(&mut server, "grit_world_rules", arguments));
        assert_eq!(
            (told.code.as_str(), told.field.as_str()),
            ("malformed", "$"),
            "{arguments}"
        );
        let told = refused(&call(&mut server, "grit_check", arguments));
        assert_eq!(
            (told.code.as_str(), told.field.as_str()),
            ("malformed", "$"),
            "{arguments}"
        );
    }
    let got = server
        .heard(&request(
            "3",
            "tools/call",
            ",\"params\":{\"name\":\"grit_world_rules\"}",
        ))
        .unwrap();
    assert!(answer(&got).get("rules").is_some());
}

#[test]
fn an_unknown_tool_is_refused_by_name() {
    let mut server = Server::new(None);
    for name in ["grit_world_explode", "", "GRIT_CHECK"] {
        let (code, named, id) = failed(&call(&mut server, name, "{}"));
        assert_eq!(
            (code, named.as_str(), id),
            (-32602, "unknown_tool", Json::Int(7))
        );
    }
    for params in [
        "",
        ",\"params\":7",
        ",\"params\":{\"arguments\":{}}",
        ",\"params\":{\"name\":3}",
    ] {
        let (code, named, _) = failed(&server.heard(&request("4", "tools/call", params)).unwrap());
        assert_eq!((code, named.as_str()), (-32602, "unknown_tool"), "{params}");
    }
}

#[test]
fn a_line_that_is_not_a_request_is_answered_once_with_a_null_id() {
    let mut server = Server::new(None);
    let ping = request("1", "ping", "");
    for line in [
        "5".to_owned(),
        "[]".to_owned(),
        "\"x\"".to_owned(),
        "null".to_owned(),
        format!("[{ping}]"),
        "{\"jsonrpc\":\"2.0\",\"id\":{},\"method\":\"ping\"}".to_owned(),
        "{\"jsonrpc\":\"2.0\",\"id\":[1],\"method\":\"ping\"}".to_owned(),
        "{\"jsonrpc\":\"2.0\",\"id\":true,\"method\":\"ping\"}".to_owned(),
        "{\"jsonrpc\":\"2.0\",\"method\":7}".to_owned(),
        "{}".to_owned(),
    ] {
        let got = server
            .heard(&line)
            .unwrap_or_else(|| panic!("no answer to {line}"));
        let (code, named, id) = failed(&got);
        assert_eq!(
            (code, named.as_str(), id),
            (-32600, "invalid_request", Json::Null),
            "{line}"
        );
    }
    let (code, named, id) = failed(
        &server
            .heard("{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":7}")
            .unwrap(),
    );
    assert_eq!(
        (code, named.as_str(), id),
        (-32600, "invalid_request", Json::Int(9))
    );
    assert!(server.heard("   ").is_none());
    assert!(
        server
            .heard("{\"jsonrpc\":\"2.0\",\"method\":\"notifications/cancelled\",\"params\":{\"requestId\":1}}")
            .is_none()
    );
}

#[test]
fn two_values_on_one_line_are_refused_as_one_line() {
    let mut server = Server::new(None);
    let one = request("1", "ping", "");
    let two = request("2", "ping", "");
    for line in [
        format!("{one}{two}"),
        format!("{one} {two}"),
        format!("{one} x"),
        format!("{one}]"),
        format!("{one},"),
    ] {
        let (code, named, id) = failed(&server.heard(&line).unwrap());
        assert_eq!(
            (code, named.as_str(), id),
            (-32700, "parse", Json::Null),
            "{line}"
        );
    }
    let got = server.heard(&format!("  {one} \t\r")).unwrap();
    assert_eq!(got.get("id"), Some(&Json::Int(1)));
    assert!(got.get("result").is_some(), "{}", written(&got));
}

#[test]
fn an_id_past_its_cap_is_refused_and_one_at_it_is_echoed() {
    let mut server = Server::new(None);
    let cap = described(&mut server)
        .get("limits")
        .and_then(|one| one.counted("id_bytes"))
        .unwrap() as usize;
    let at = "i".repeat(cap);
    let got = server
        .heard(&request(&format!("\"{at}\""), "ping", ""))
        .unwrap();
    assert_eq!(got.get("id").and_then(Json::text), Some(at.as_str()));
    let past = "i".repeat(cap + 1);
    let (code, named, id) = failed(
        &server
            .heard(&request(&format!("\"{past}\""), "ping", ""))
            .unwrap(),
    );
    assert_eq!(
        (code, named.as_str(), id),
        (-32600, "invalid_request", Json::Null)
    );
    for id in ["-9223372036854775808", "9223372036854775807", "0", "null"] {
        let got = server.heard(&request(id, "ping", "")).unwrap();
        assert_eq!(written(got.get("id").unwrap()), id);
    }
}

#[test]
fn an_echo_never_makes_an_answer_past_the_cap() {
    let mut server = Server::new(None);
    let long = "a".repeat(900_000);
    let lines = [
        request("1", &long, ""),
        calling(
            "grit_world_step",
            &format!("{{\"world\":1,\"frames\":\"{long}\"}}"),
        ),
        calling("grit_world_step", &format!("{{\"world\":1,\"{long}\":1}}")),
        calling(
            "grit_world_slice",
            &format!("{{\"world\":1,\"at\":0,\"axis\":\"{long}\"}}"),
        ),
        calling(
            "grit_world_create",
            &format!("{{\"wide\":8,\"deep\":8,\"tall\":8,\"rule\":\"{long}\"}}"),
        ),
        calling(&long, "{}"),
        calling("grit_slope", &format!("{{\"kind\":\"{long}\"}}")),
        calling(
            "grit_pour",
            &format!("{{\"cells\":[{{\"at\":1,\"is\":\"{long}\"}}]}}"),
        ),
    ];
    for line in &lines {
        let got = server.heard(line).unwrap();
        let size = written(&got).len();
        assert!(size <= ANSWER_MOST, "{size} bytes answer {}", &line[..80]);
        assert!(
            got.get("error").is_some()
                || got.get("result").and_then(|one| one.get("isError")) == Some(&Json::Bool(true)),
            "{}",
            &written(&got)[..200]
        );
    }
}

#[test]
fn every_refusal_carries_a_code_from_the_described_list() {
    let mut server = Server::new(None);
    let codes: Vec<String> = described(&mut server)
        .get("codes")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .map(|one| one.get("code").and_then(Json::text).unwrap().to_owned())
        .collect();
    let tool_calls = [
        ("grit_world_step", "{}"),
        ("grit_world_step", "{\"world\":1}"),
        ("grit_world_step", "{\"world\":1,\"frames\":0}"),
        (
            "grit_world_create",
            "{\"wide\":8,\"deep\":8,\"tall\":8,\"rule\":\"lava\"}",
        ),
        (
            "grit_world_create",
            "{\"wide\":8,\"deep\":8,\"tall\":8,\"rule\":\"heat\"}",
        ),
        (
            "grit_world_create",
            "{\"wide\":8,\"deep\":8,\"tall\":8,\"rule\":\"sand\"}",
        ),
        ("grit_world_slice", "{\"world\":1,\"at\":0,\"axis\":\"w\"}"),
        (
            "grit_world_read",
            "{\"world\":1,\"from\":[0,0],\"to\":[1,1,1]}",
        ),
        (
            "grit_check",
            "{\"inputs\":2,\"outputs\":1,\"gates\":0,\"table\":[0,1,1,0]}",
        ),
        ("grit_slope", "{}"),
        ("grit_machine", "{\"traces\":[]}"),
    ];
    for (tool, arguments) in tool_calls {
        let told = refused(&call(&mut server, tool, arguments));
        assert!(
            codes.contains(&told.code),
            "{tool} {arguments}: {}",
            told.code
        );
    }
    for line in [
        "{".to_owned(),
        request("1", "tools/dream", ""),
        calling("grit_nothing", "{}"),
        "7".to_owned(),
    ] {
        let (_, named, _) = failed(&server.heard(&line).unwrap());
        assert!(codes.contains(&named), "{line}: {named}");
    }
    let told = refused(&call(
        &mut server,
        "grit_world_write",
        &format!(
            "{{\"world\":1,\"cells\":[{}]}}",
            vec!["[0,0,0,1]"; 4097].join(",")
        ),
    ));
    assert_eq!(
        (told.code.as_str(), told.field.as_str(), told.limit),
        ("too_many", "$.cells", Some(Json::Int(4096)))
    );
    let told = refused(&call(
        &mut server,
        "grit_world_step",
        "{\"world\":1,\"frames\":65537}",
    ));
    assert_eq!(
        (told.code.as_str(), told.field.as_str(), told.limit),
        ("frames", "$.frames", Some(Json::Int(65536)))
    );
}

#[test]
fn describe_names_every_code_limit_tool_device_and_feature() {
    let mut server = Server::new(None);
    let told = described(&mut server);
    let codes = told.get("codes").and_then(Json::list).unwrap();
    let mut names: Vec<&str> = codes
        .iter()
        .map(|one| one.get("code").and_then(Json::text).unwrap())
        .collect();
    for one in codes {
        let means = one.get("hint").and_then(Json::text).unwrap();
        assert!(means.ends_with('.'), "{means}");
    }
    let all = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), all);
    for code in [
        "parse",
        "line_too_long",
        "invalid_request",
        "unknown_method",
        "unknown_tool",
        "fault",
        "answer_too_large",
        "malformed",
        "unknown_key",
        "duplicate_key",
        "too_many",
        "outside",
        "no_compute_device",
    ] {
        assert!(names.contains(&code), "{code}");
    }
    let limits = told.get("limits").unwrap();
    assert_eq!(limits.counted("line_bytes"), Some(1 << 20));
    assert_eq!(limits.counted("answer_bytes"), Some(1 << 20));
    assert_eq!(limits.counted("read_cells"), Some(4096));
    assert_eq!(limits.counted("slice_cells"), Some(65536));
    assert_eq!(limits.counted("step_frames"), Some(65536));
    assert_eq!(limits.counted("depth"), Some(64));
    let tools: Vec<String> = told
        .get("tools")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .map(|one| one.text().unwrap().to_owned())
        .collect();
    let published: Vec<String> = listed(&mut server)
        .iter()
        .map(|one| one.get("name").and_then(Json::text).unwrap().to_owned())
        .collect();
    assert_eq!(tools, published);
    assert!(tools.contains(&"grit_describe".to_owned()));
    assert_eq!(told.get("device"), Some(&Json::Null));
    let features = told.get("features").unwrap();
    for (name, on) in [
        ("window", cfg!(feature = "window")),
        ("deep", cfg!(feature = "deep")),
        ("faults", cfg!(feature = "faults")),
    ] {
        assert_eq!(features.get(name), Some(&Json::Bool(on)), "{name}");
    }
}

#[test]
fn the_depth_limit_described_is_the_one_the_reader_keeps() {
    let mut server = Server::new(None);
    let depth = described(&mut server)
        .get("limits")
        .and_then(|one| one.counted("depth"))
        .unwrap() as usize;
    let nested = |deep: usize| {
        calling(
            "grit_world_rules",
            &format!("{}{}", "[".repeat(deep - 1), "]".repeat(deep - 1)),
        )
    };
    let (code, named, _) = failed(&server.heard(&nested(depth + 1)).unwrap());
    assert_eq!((code, named.as_str()), (-32700, "parse"));
    let told = refused(&server.heard(&nested(depth)).unwrap());
    assert_eq!(
        (told.code.as_str(), told.field.as_str()),
        ("malformed", "$")
    );
}

#[test]
fn ping_is_answered_with_an_empty_result() {
    let mut server = Server::new(None);
    let got = server.heard(&request("\"p\"", "ping", "")).unwrap();
    assert_eq!(
        written(&got),
        "{\"jsonrpc\":\"2.0\",\"id\":\"p\",\"result\":{}}"
    );
}

#[test]
fn the_2d_tools_alone_share_the_transport() {
    let line = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}{}";
    let (code, named, _) = failed(&grit::serve::heard(None, line).unwrap());
    assert_eq!((code, named.as_str()), (-32700, "parse"));
    let long = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\",\"x\":\"{}\"}}",
        "a".repeat(1 << 20)
    );
    let (code, named, _) = failed(&grit::serve::heard(None, &long).unwrap());
    assert_eq!((code, named.as_str()), (-32600, "line_too_long"));
    let got =
        grit::serve::handle(None, &read(&calling("grit_world_rules", "{}")).unwrap()).unwrap();
    let (code, named, _) = failed(&got);
    assert_eq!((code, named.as_str()), (-32602, "unknown_tool"));
}

struct Rng(u64);

impl Rng {
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
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Wrong {
    Type,
    Missing,
    Unknown,
    Duplicate,
    Range,
    Choice,
    Long,
    Short,
    Count,
}

fn keyword(schema: &Json, key: &str) -> Option<i64> {
    schema.get(key).and_then(Json::int)
}

fn path_of(path: &str, key: &str) -> String {
    let mut letters = key.chars();
    let plain = letters
        .next()
        .is_some_and(|one| one.is_ascii_alphabetic() || one == '_')
        && letters.all(|one| one.is_ascii_alphanumeric() || one == '_');
    if plain {
        format!("{path}.{key}")
    } else {
        format!("{path}[{}]", written(&Json::Text(key.to_owned())))
    }
}

fn faults(schema: &Json, value: &Json, path: &str, whole: bool, out: &mut Vec<(String, Wrong)>) {
    let kind = schema.get("type").and_then(Json::text).unwrap_or("");
    match kind {
        "object" => {
            let Json::Map(held) = value else {
                if whole || path == "$" {
                    out.push((path.to_owned(), Wrong::Type));
                }
                return;
            };
            let Some(Json::Map(properties)) = schema.get("properties") else {
                return;
            };
            let closed = schema.get("additionalProperties") == Some(&Json::Bool(false));
            let mut seen: Vec<&str> = Vec::new();
            for (name, one) in held {
                let here = path_of(path, name);
                if seen.contains(&name.as_str()) {
                    out.push((here, Wrong::Duplicate));
                    continue;
                }
                seen.push(name);
                match properties.iter().find(|(key, _)| key == name) {
                    Some((_, inner)) => faults(inner, one, &here, whole, out),
                    None if closed => out.push((here, Wrong::Unknown)),
                    None => {}
                }
            }
            if whole {
                for required in schema.get("required").and_then(Json::list).unwrap_or(&[]) {
                    let required = required.text().unwrap();
                    if !held.iter().any(|(name, _)| name == required) {
                        out.push((path_of(path, required), Wrong::Missing));
                    }
                }
            }
        }
        "array" => {
            let Json::List(held) = value else {
                if whole {
                    out.push((path.to_owned(), Wrong::Type));
                }
                return;
            };
            if whole {
                let least = keyword(schema, "minItems").unwrap_or(0);
                let most = keyword(schema, "maxItems").unwrap_or(i64::MAX);
                let many = held.len() as i64;
                if least == most && many != least {
                    out.push((path.to_owned(), Wrong::Count));
                } else if many > most {
                    out.push((path.to_owned(), Wrong::Long));
                } else if many < least {
                    out.push((path.to_owned(), Wrong::Short));
                }
            }
            if let Some(items) = schema.get("items") {
                for (at, one) in held.iter().enumerate() {
                    faults(items, one, &format!("{path}[{at}]"), whole, out);
                }
            }
        }
        _ if !whole => {}
        "integer" => match value.int() {
            None => out.push((path.to_owned(), Wrong::Type)),
            Some(one) => {
                let least = keyword(schema, "minimum").unwrap_or(i64::MIN);
                let most = keyword(schema, "maximum").unwrap_or(i64::MAX);
                if one < least || one > most {
                    out.push((path.to_owned(), Wrong::Range));
                }
            }
        },
        "string" => match value.text() {
            None => out.push((path.to_owned(), Wrong::Type)),
            Some(one) => {
                if let Some(allowed) = schema.get("enum").and_then(Json::list)
                    && !allowed.iter().any(|choice| choice.text() == Some(one))
                {
                    out.push((path.to_owned(), Wrong::Choice));
                }
            }
        },
        "boolean" => {
            if !matches!(value, Json::Bool(_)) {
                out.push((path.to_owned(), Wrong::Type));
            }
        }
        other => panic!("a schema type the checker does not know: {other}"),
    }
}

fn code_of(tool: &str, path: &str, wrong: Wrong) -> &'static str {
    match (tool, path, wrong) {
        ("grit_world_create", "$.rule", Wrong::Missing | Wrong::Type | Wrong::Choice) => {
            "unknown_rule"
        }
        ("grit_world_step", "$.frames", Wrong::Range) => "frames",
        ("grit_world_slice", "$.axis", Wrong::Type | Wrong::Choice) => "axis",
        (_, _, Wrong::Unknown) => "unknown_key",
        (_, _, Wrong::Duplicate) => "duplicate_key",
        (_, _, Wrong::Long) => "too_many",
        _ => "malformed",
    }
}

fn sample(schema: &Json, rng: &mut Rng, flat: bool) -> Json {
    match schema.get("type").and_then(Json::text).unwrap_or("") {
        "object" => {
            let mut held = Vec::new();
            let required: Vec<&str> = schema
                .get("required")
                .and_then(Json::list)
                .unwrap_or(&[])
                .iter()
                .map(|one| one.text().unwrap())
                .collect();
            if let Some(Json::Map(properties)) = schema.get("properties") {
                for (name, inner) in properties {
                    if required.contains(&name.as_str()) || rng.chance(2) {
                        held.push((name.clone(), sample(inner, rng, flat)));
                    }
                }
            }
            Json::Map(held)
        }
        "array" => {
            let least = keyword(schema, "minItems").unwrap_or(0).max(0) as u64;
            let most = keyword(schema, "maxItems").unwrap_or(4).max(0) as u64;
            let many = if rng.chance(200) {
                most
            } else {
                least + rng.below(most.min(least + 5) - least + 1)
            };
            let items = schema
                .get("items")
                .cloned()
                .unwrap_or(Json::Map(Vec::new()));
            Json::List((0..many).map(|_| sample(&items, rng, flat)).collect())
        }
        "integer" => {
            if flat {
                return Json::Int(rng.below(6) as i64);
            }
            let least = keyword(schema, "minimum").unwrap_or(-1000);
            let most = keyword(schema, "maximum").unwrap_or(1000);
            let span = (most as i128 - least as i128) as u128;
            Json::Int(match rng.below(5) {
                0 => least,
                1 => most,
                2 => least.saturating_add(1).min(most),
                _ => (least as i128 + (u128::from(rng.next()) % (span + 1)) as i128) as i64,
            })
        }
        "string" => match schema.get("enum").and_then(Json::list) {
            Some(allowed) => allowed[rng.below(allowed.len() as u64) as usize].clone(),
            None => Json::Text(["sand", "x", "", "bound"][rng.below(4) as usize].to_owned()),
        },
        "boolean" => Json::Bool(rng.chance(2)),
        _ => Json::Null,
    }
}

fn places(value: &Json, path: &str, out: &mut Vec<String>) {
    out.push(path.to_owned());
    match value {
        Json::Map(held) => {
            for (name, one) in held {
                places(one, &path_of(path, name), out);
            }
        }
        Json::List(held) => {
            for (at, one) in held.iter().enumerate() {
                places(one, &format!("{path}[{at}]"), out);
            }
        }
        _ => {}
    }
}

fn at_path<'held>(value: &'held mut Json, path: &str) -> &'held mut Json {
    let mut here = value;
    let mut rest = path.strip_prefix('$').unwrap();
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("[\"") {
            let end = after.find("\"]").unwrap();
            let key = &after[..end];
            rest = &after[end + 2..];
            here = match here {
                Json::Map(held) => &mut held.iter_mut().find(|(name, _)| name == key).unwrap().1,
                _ => unreachable!(),
            };
        } else if let Some(after) = rest.strip_prefix('[') {
            let end = after.find(']').unwrap();
            let at: usize = after[..end].parse().unwrap();
            rest = &after[end + 1..];
            here = match here {
                Json::List(held) => &mut held[at],
                _ => unreachable!(),
            };
        } else {
            let after = rest.strip_prefix('.').unwrap();
            let end = after.find(['.', '[']).unwrap_or(after.len());
            let key = &after[..end];
            rest = &after[end..];
            here = match here {
                Json::Map(held) => &mut held.iter_mut().find(|(name, _)| name == key).unwrap().1,
                _ => unreachable!(),
            };
        }
    }
    here
}

fn mutate(value: &mut Json, rng: &mut Rng) {
    let mut all = Vec::new();
    places(value, "$", &mut all);
    let path = all[rng.below(all.len() as u64) as usize].clone();
    let here = at_path(value, &path);
    let odd = [
        Json::Null,
        Json::Bool(true),
        Json::Int(-1),
        Json::Int(i64::MAX),
        Json::Int(i64::MIN),
        Json::Int(4_294_967_296),
        Json::Int(0),
        Json::Text("z".to_owned()),
        Json::List(vec![Json::Int(1)]),
        Json::Map(Vec::new()),
    ];
    match rng.below(6) {
        0 | 1 => *here = odd[rng.below(odd.len() as u64) as usize].clone(),
        2 => {
            if let Json::Map(held) = here {
                let key = ["frame", "zz", "Frames", "world ", "_meta"][rng.below(5) as usize];
                held.push((key.to_owned(), Json::Int(1)));
            } else {
                *here = Json::Text("wrong".to_owned());
            }
        }
        3 => {
            if let Json::Map(held) = here
                && !held.is_empty()
            {
                let at = rng.below(held.len() as u64) as usize;
                if rng.chance(2) {
                    held.remove(at);
                } else {
                    let copy = held[at].clone();
                    held.push((copy.0, Json::Int(2)));
                }
            } else {
                *here = Json::Bool(false);
            }
        }
        4 => {
            if let Json::List(held) = here {
                if rng.chance(2) || held.is_empty() {
                    held.push(held.first().cloned().unwrap_or(Json::Int(0)));
                } else {
                    held.pop();
                }
            } else {
                *here = Json::List(Vec::new());
            }
        }
        _ => {
            if let Json::Int(one) = here {
                *one = one.saturating_add([1, -1, 65536, -65536][rng.below(4) as usize]);
            } else {
                *here = Json::Int(3);
            }
        }
    }
}

#[test]
fn every_published_schema_and_its_validator_agree_on_a_thousand_argument_sets() {
    let mut server = Server::new(None);
    let tools = listed(&mut server);
    let known = [
        "type",
        "description",
        "properties",
        "required",
        "additionalProperties",
        "items",
        "minimum",
        "maximum",
        "minItems",
        "maxItems",
        "enum",
        "default",
    ];
    let mut walk = vec![];
    for tool in &tools {
        walk.push(tool.get("inputSchema").unwrap().clone());
    }
    while let Some(schema) = walk.pop() {
        let Json::Map(held) = &schema else { continue };
        for (key, one) in held {
            assert!(
                known.contains(&key.as_str()),
                "a keyword no validator reads: {key}"
            );
            match key.as_str() {
                "items" => walk.push(one.clone()),
                "properties" => {
                    if let Json::Map(inner) = one {
                        walk.extend(inner.iter().map(|(_, one)| one.clone()));
                    }
                }
                _ => {}
            }
        }
    }
    let mut refused_sets = 0;
    let mut passed_sets = 0;
    for tool in &tools {
        let name = tool.get("name").and_then(Json::text).unwrap();
        let schema = tool.get("inputSchema").unwrap();
        assert_eq!(
            schema.get("additionalProperties"),
            Some(&Json::Bool(false)),
            "{name}"
        );
        let flat = FLAT.contains(&name);
        let mut rng = Rng(0xA6EE ^ name.len() as u64);
        for set in 0..1000 {
            let mut arguments = sample(schema, &mut rng, flat);
            if rng.chance(2) {
                for _ in 0..=rng.below(2) {
                    mutate(&mut arguments, &mut rng);
                }
            }
            let mut wrong = Vec::new();
            faults(schema, &arguments, "$", !flat, &mut wrong);
            let line = calling(name, &written(&arguments));
            let got = server.heard(&line).unwrap();
            let result = got.get("result");
            let told = result.and_then(|one| one.get("structuredContent"));
            let code = told.and_then(|one| one.get("code")).and_then(Json::text);
            let field = told.and_then(|one| one.get("field")).and_then(Json::text);
            if wrong.iter().any(|(_, kind)| *kind == Wrong::Duplicate) {
                refused_sets += 1;
                assert_eq!(
                    got.get("error")
                        .and_then(|one| one.get("code"))
                        .and_then(Json::int),
                    Some(-32700),
                    "{name} set {set}: a key given twice reads as no JSON: {}",
                    written(&got)
                );
                continue;
            }
            if wrong.is_empty() {
                passed_sets += 1;
                let schema_codes: &[&str] = if flat {
                    &["unknown_key", "duplicate_key", "malformed"]
                } else {
                    &[
                        "unknown_key",
                        "duplicate_key",
                        "malformed",
                        "too_many",
                        "unknown_rule",
                        "axis",
                        "frames",
                    ]
                };
                assert!(
                    code.is_none_or(|one| !schema_codes.contains(&one)),
                    "{name} set {set}: valid by the schema and refused as {code:?} at {field:?}: {}\n{}",
                    written(&arguments),
                    written(&got)
                );
            } else {
                refused_sets += 1;
                let expected: Vec<(String, &str)> = wrong
                    .iter()
                    .map(|(path, kind)| (path.clone(), code_of(name, path, *kind)))
                    .collect();
                let got_pair = (field.unwrap_or("").to_owned(), code.unwrap_or(""));
                assert!(
                    expected
                        .iter()
                        .any(|(path, code)| (path.as_str(), *code)
                            == (got_pair.0.as_str(), got_pair.1)),
                    "{name} set {set}: the schema finds {expected:?} and the server answered {got_pair:?}: {}\n{}",
                    written(&arguments),
                    written(&got)
                );
            }
        }
    }
    println!("{passed_sets} sets valid by their schema, {refused_sets} refused by it");
    assert!(refused_sets > 1000 && passed_sets > 1000);
}

#[test]
fn a_misspelt_argument_leaves_the_world_where_it_was() {
    let Some(host) = device("misspelt argument") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let world = answer(&call(
        &mut server,
        "grit_world_create",
        "{\"wide\":8,\"deep\":8,\"tall\":8,\"rule\":\"sand\"}",
    ))
    .counted("world")
    .unwrap();
    let told = refused(&call(
        &mut server,
        "grit_world_step",
        &format!("{{\"world\":{world},\"frame\":100}}"),
    ));
    assert_eq!(
        (told.code.as_str(), told.field.as_str()),
        ("unknown_key", "$.frame")
    );
    let got = answer(&call(
        &mut server,
        "grit_world_step",
        &format!("{{\"world\":{world}}}"),
    ));
    assert_eq!(got.counted("frame"), Some(1));
    let named = described(&mut server);
    let device = named.get("device").unwrap();
    assert_eq!(device.get("name").and_then(Json::text), Some(host.name()));
    assert_eq!(device.counted("subgroup"), Some(host.width()));
}

#[test]
fn a_slice_past_one_answer_comes_in_pages_that_join_to_the_whole() {
    let Some(host) = device("slice pages") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let world = answer(&call(
        &mut server,
        "grit_world_create",
        "{\"wide\":256,\"deep\":256,\"tall\":2,\"rule\":\"sand\"}",
    ))
    .counted("world")
    .unwrap();
    let word = |x: u32, y: u32| u32::MAX - x - 256 * y;
    for rows in 0..16 {
        let cells: Vec<String> = (0..4096)
            .map(|at| {
                let (x, y) = (at % 256, rows * 16 + at / 256);
                format!("[{x},{y},1,{}]", word(x, y))
            })
            .collect();
        answer(&call(
            &mut server,
            "grit_world_write",
            &format!("{{\"world\":{world},\"cells\":[{}]}}", cells.join(",")),
        ));
    }
    let mut cursor = 0;
    let mut rows = Vec::new();
    let mut pages = 0;
    loop {
        let got = call(
            &mut server,
            "grit_world_slice",
            &format!("{{\"world\":{world},\"at\":1,\"as\":\"words\",\"cursor\":{cursor}}}"),
        );
        let size = written(&got).len();
        assert!(size <= ANSWER_MOST, "{size}");
        let page = answer(&got);
        pages += 1;
        rows.extend(page.get("rows").and_then(Json::list).unwrap().to_vec());
        match page.get("next") {
            None => break,
            Some(next) => {
                let next = next.int().unwrap();
                assert_eq!(next as usize, rows.len());
                cursor = next;
            }
        }
    }
    assert!(pages > 1, "{pages}");
    assert_eq!(rows.len(), 256);
    for (y, row) in rows.iter().enumerate() {
        let words: Vec<u32> = row
            .list()
            .unwrap()
            .iter()
            .map(|one| one.int().unwrap() as u32)
            .collect();
        let want: Vec<u32> = (0..256).map(|x| word(x, y as u32)).collect();
        assert_eq!(words, want, "row {y}");
    }
    let kinds = answer(&call(
        &mut server,
        "grit_world_slice",
        &format!("{{\"world\":{world},\"at\":1}}"),
    ));
    assert_eq!(kinds.get("rows").and_then(Json::list).unwrap().len(), 256);
    assert!(kinds.get("next").is_none());
    let told = refused(&call(
        &mut server,
        "grit_world_slice",
        &format!("{{\"world\":{world},\"at\":1,\"cursor\":256}}"),
    ));
    assert_eq!(
        (told.code.as_str(), told.field.as_str(), told.limit),
        ("outside", "$.cursor", Some(Json::Int(255)))
    );
}

#[test]
fn the_largest_read_answer_fits_one_answer() {
    let Some(host) = device("largest read") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let world = answer(&call(
        &mut server,
        "grit_world_create",
        "{\"wide\":16,\"deep\":16,\"tall\":16,\"rule\":\"heat\",\"params\":[600,4,4096]}",
    ))
    .counted("world")
    .unwrap();
    let cells: Vec<String> = (0..4096)
        .map(|at| format!("[{},{},{},4294967295]", at % 16, (at / 16) % 16, at / 256))
        .collect();
    answer(&call(
        &mut server,
        "grit_world_write",
        &format!("{{\"world\":{world},\"cells\":[{}]}}", cells.join(",")),
    ));
    let got = call(
        &mut server,
        "grit_world_read",
        &format!("{{\"world\":{world},\"from\":[0,0,0],\"to\":[15,15,15]}}"),
    );
    let size = written(&got).len();
    println!("the largest read answer is {size} bytes");
    assert!(size * 8 < ANSWER_MOST, "{size}");
    assert_eq!(
        answer(&got)
            .get("cells")
            .and_then(Json::list)
            .unwrap()
            .len(),
        4096
    );
}

#[test]
fn the_line_reader_holds_a_line_at_the_cap_and_skips_one_past_it() {
    use grit::serve3::transport::{LINE_MOST, Line, Lines};
    let at = vec![b'a'; LINE_MOST];
    let past = vec![b'b'; LINE_MOST + 1];
    let mut input = Vec::new();
    let parts: [&[u8]; 10] = [
        &at, b"\n", &past, b"\n", b"\r\n", b"\n", b"{}\r\n", &past, b"\n", b"tail",
    ];
    for part in parts {
        input.extend_from_slice(part);
    }
    for capacity in [1, 7, 4096, 1 << 22] {
        let mut lines = Lines::new(std::io::BufReader::with_capacity(
            capacity,
            std::io::Cursor::new(input.clone()),
        ));
        let mut got = Vec::new();
        while let Some(line) = lines.next_line().unwrap() {
            got.push(line);
        }
        assert_eq!(
            got,
            vec![
                Line::Held(at.clone()),
                Line::Long {
                    bytes: LINE_MOST + 1
                },
                Line::Held(b"\r".to_vec()),
                Line::Held(Vec::new()),
                Line::Held(b"{}\r".to_vec()),
                Line::Long {
                    bytes: LINE_MOST + 1
                },
                Line::Held(b"tail".to_vec()),
            ],
            "{capacity}"
        );
    }
    let mut lines = Lines::new(std::io::Cursor::new(vec![b'c'; 3 * LINE_MOST]));
    assert_eq!(
        lines.next_line().unwrap(),
        Some(Line::Long {
            bytes: 3 * LINE_MOST
        })
    );
    assert_eq!(lines.next_line().unwrap(), None);
}

#[test]
fn a_step_at_the_frame_cap_passes_the_schema_and_one_past_it_does_not() {
    let mut server = Server::new(None);
    let told = refused(&call(
        &mut server,
        "grit_world_step",
        "{\"world\":1,\"frames\":65536}",
    ));
    assert_eq!(told.code, "no_world");
    let told = refused(&call(
        &mut server,
        "grit_world_step",
        "{\"world\":1,\"frames\":0}",
    ));
    assert_eq!(
        (told.code.as_str(), told.limit),
        ("frames", Some(Json::Int(1)))
    );
    let told = refused(&call(&mut server, "grit_world_step", "{\"world\":0}"));
    assert_eq!(
        (told.code.as_str(), told.field.as_str()),
        ("malformed", "$.world")
    );
    let cells = |many: usize| {
        format!(
            "{{\"world\":1,\"cells\":[{}]}}",
            vec!["[0,0,0,1]"; many].join(",")
        )
    };
    assert_eq!(
        refused(&call(&mut server, "grit_world_write", &cells(4096))).code,
        "no_world"
    );
    assert_eq!(
        refused(&call(&mut server, "grit_world_write", &cells(0))).code,
        "no_world"
    );
    let told = refused(&call(
        &mut server,
        "grit_world_write",
        "{\"world\":1,\"cells\":[[0,0,0,1],[0,0,0,1,1]]}",
    ));
    assert_eq!(
        (told.code.as_str(), told.field.as_str(), told.limit),
        ("malformed", "$.cells[1]", Some(Json::Int(4)))
    );
    let told = refused(&call(
        &mut server,
        "grit_world_create",
        "{\"wide\":8,\"deep\":8,\"tall\":8,\"rule\":\"sand\",\"sparse\":true,\"slots\":0}",
    ));
    assert_eq!(
        (told.code.as_str(), told.field.as_str()),
        ("malformed", "$.slots")
    );
}

#[test]
fn a_sparse_world_past_four_billion_cells_opens_and_keeps_a_far_cell() {
    let Some(host) = device("sparse past u32 cells") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let asked =
        "{\"wide\":4096,\"deep\":256,\"tall\":4096,\"rule\":\"sand\",\"sparse\":true,\"slots\":64}";
    let grid = grit::sand::Grid::solid(4096, 256, 4096).unwrap();
    let blocks = grid.blocks().div_ceil(grit::sweep::cells(host.width()));
    if grit::compact::Plan::bricked(host.width(), blocks).is_err() {
        let told = refused(&call(&mut server, "grit_world_create", asked));
        println!(
            "width {}: the engine plans no 2^32-cell sparse world, refused as {}",
            host.width(),
            told.code
        );
        assert_eq!(told.code, "too_big");
        return;
    }
    let made = answer(&call(&mut server, "grit_world_create", asked));
    assert_eq!(made.get("cells"), Some(&Json::Int(1 << 32)));
    let world = made.counted("world").unwrap();
    answer(&call(
        &mut server,
        "grit_world_write",
        &format!("{{\"world\":{world},\"cells\":[[4095,255,4095,3],[0,0,0,2]]}}"),
    ));
    let got = answer(&call(
        &mut server,
        "grit_world_read",
        &format!("{{\"world\":{world},\"from\":[4094,254,4094],\"to\":[4095,255,4095]}}"),
    ));
    let cells: Vec<i64> = got
        .get("cells")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .map(|one| one.int().unwrap())
        .collect();
    assert_eq!(cells, [0, 0, 0, 0, 0, 0, 0, 3]);
    let told = refused(&call(
        &mut server,
        "grit_world_create",
        "{\"wide\":4096,\"deep\":256,\"tall\":4096,\"rule\":\"sand\"}",
    ));
    assert!(
        ["unfit", "too_big", "too_large"].contains(&told.code.as_str()),
        "{}",
        told.code
    );
}
