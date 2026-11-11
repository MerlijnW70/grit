#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use grit::Host;
use grit::rule3::code::{self, ADD, DATA, HALT, MOVE, PLUS_X, PLUS_Z};
use grit::rule3::heat;
use grit::rule3::nature;
use grit::sand3::Grid3;
use grit::serve::handle;
use grit::serve3::{READ_MOST, STEP_MOST, Server, WORLD_TOOLS, rules};
use grit::wire::{Json, read, written};

fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut mixed = *state;
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    mixed ^ (mixed >> 31)
}

fn call(server: &mut Server<'_>, name: &str, arguments: &str) -> Json {
    let line = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{{\"name\":\"{name}\",\"arguments\":{arguments}}}}}"
    );
    server.heard(&line).unwrap()
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

fn refusal(got: &Json) -> String {
    let result = got.get("result").unwrap();
    assert_eq!(
        result.get("isError"),
        Some(&Json::Bool(true)),
        "{}",
        written(got)
    );
    let told = result.get("structuredContent").unwrap();
    let message = told.get("message").and_then(Json::text).unwrap();
    let text = result
        .get("content")
        .and_then(Json::list)
        .and_then(|one| one.first())
        .and_then(|one| one.get("text"))
        .and_then(Json::text)
        .unwrap();
    assert_eq!(message, text);
    told.get("refused").and_then(Json::text).unwrap().to_owned()
}

fn numbers(held: &Json, key: &str) -> Vec<u32> {
    held.get(key)
        .and_then(Json::list)
        .unwrap()
        .iter()
        .map(|one| u32::try_from(one.int().unwrap()).unwrap())
        .collect()
}

fn listed_by(names: &str) -> Vec<String> {
    let request = read(&format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"{names}}}"
    ))
    .unwrap();
    let got = Server::new(None).handle(&request).unwrap();
    got.get("result")
        .and_then(|one| one.get("tools"))
        .and_then(Json::list)
        .unwrap()
        .iter()
        .map(written)
        .collect()
}

fn created(server: &mut Server<'_>, rule: &str, size: [u32; 3], params: &[u32]) -> u32 {
    let got = answer(&call(
        server,
        "grit_world_create",
        &format!(
            "{{\"wide\":{},\"deep\":{},\"tall\":{},\"rule\":\"{rule}\",\"params\":{params:?}}}",
            size[0], size[1], size[2]
        ),
    ));
    assert_eq!(numbers(&got, "size"), size);
    got.counted("world").unwrap()
}

fn writing(cells: &[(u32, u32, u32, u32)]) -> String {
    let held: Vec<String> = cells
        .iter()
        .map(|(x, y, z, word)| format!("[{x},{y},{z},{word}]"))
        .collect();
    format!("[{}]", held.join(","))
}

fn read_all(server: &mut Server<'_>, world: u32, size: [u32; 3]) -> Vec<u32> {
    let got = answer(&call(
        server,
        "grit_world_read",
        &format!(
            "{{\"world\":{world},\"from\":[0,0,0],\"to\":[{},{},{}]}}",
            size[0] - 1,
            size[1] - 1,
            size[2] - 1
        ),
    ));
    numbers(&got, "cells")
}

fn step(server: &mut Server<'_>, world: u32, frames: u32) -> Json {
    answer(&call(
        server,
        "grit_world_step",
        &format!("{{\"world\":{world},\"frames\":{frames}}}"),
    ))
}

/// Every heat kind, the inert ones too, at temperatures around the
/// thresholds, fires with any life, and random kept bits.
fn heat_cell(state: &mut u64) -> u32 {
    let roll = next(state);
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

/// The CPU reference from `start` over `frames` frames from `first` on.
fn stepped(
    rule: &str,
    grid: Grid3,
    start: &[u32],
    params: &[u32],
    first: u32,
    frames: u32,
) -> Vec<u32> {
    let listed = rules().into_iter().find(|one| one.name() == rule).unwrap();
    (first..first + frames).fold(start.to_vec(), |cells, frame| {
        listed.reference(grid, frame, &cells, params)
    })
}

#[test]
fn the_server_lists_every_2d_tool_unchanged_and_then_the_world_tools() {
    let request = read("{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}").unwrap();
    let flat: Vec<String> = handle(None, &request)
        .unwrap()
        .get("result")
        .and_then(|one| one.get("tools"))
        .and_then(Json::list)
        .unwrap()
        .iter()
        .map(written)
        .collect();
    let all = listed_by("");
    assert_eq!(flat.len(), 7);
    assert_eq!(all[..flat.len()], flat[..]);
    let names: Vec<String> = all[flat.len()..]
        .iter()
        .map(|one| {
            read(one)
                .unwrap()
                .get("name")
                .and_then(Json::text)
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(names, WORLD_TOOLS);
}

#[test]
fn every_world_tool_says_what_it_takes_and_which_of_it_is_required() {
    for one in listed_by("").iter().skip(7) {
        let tool = read(one).unwrap();
        let description = tool.get("description").and_then(Json::text).unwrap();
        assert!(description.len() <= 200, "{description}");
        let schema = tool.get("inputSchema").unwrap();
        let properties = match schema.get("properties").unwrap() {
            Json::Map(held) => held.clone(),
            other => panic!("{}", written(other)),
        };
        for (name, property) in &properties {
            assert!(
                property.get("type").and_then(Json::text).is_some(),
                "{name}"
            );
        }
        for required in schema.get("required").and_then(Json::list).unwrap() {
            let required = required.text().unwrap();
            assert!(
                properties.iter().any(|(name, _)| name == required),
                "{required}"
            );
        }
    }
}

#[test]
fn a_2d_tool_answers_through_the_server_as_it_does_alone() {
    let arguments = "{\"inputs\":2,\"outputs\":1,\"gates\":1,\"table\":[0,0,0,1],\"chip\":[{\"kind\":8,\"left\":0,\"right\":1}]}";
    let line = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{{\"name\":\"grit_check\",\"arguments\":{arguments}}}}}"
    );
    let alone = handle(None, &read(&line).unwrap()).unwrap();
    let served = Server::new(None).heard(&line).unwrap();
    assert_eq!(written(&served), written(&alone));
    let unknown = "{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"tools/dream\"}";
    assert_eq!(
        written(&Server::new(None).heard(unknown).unwrap()),
        written(&handle(None, &read(unknown).unwrap()).unwrap())
    );
    assert!(
        Server::new(None)
            .heard("{\"jsonrpc\":\"2.0\",\"method\":\"initialized\"}")
            .is_none()
    );
}

#[test]
fn the_rules_are_listed_with_their_parameter_counts() {
    let got = answer(&call(&mut Server::new(None), "grit_world_rules", "{}"));
    assert_eq!(
        written(got.get("rules").unwrap()),
        "[{\"name\":\"sand\",\"params\":0},{\"name\":\"heat\",\"params\":3},{\"name\":\"code\",\"params\":0},{\"name\":\"nature\",\"params\":0}]"
    );
}

#[test]
fn an_unknown_rule_is_refused_and_the_refusal_lists_the_rules() {
    let mut server = Server::new(None);
    for rule in ["\"lava\"", "\"Heat\"", "\"\"", "7"] {
        let got = call(
            &mut server,
            "grit_world_create",
            &format!("{{\"wide\":16,\"deep\":16,\"tall\":16,\"rule\":{rule}}}"),
        );
        assert_eq!(refusal(&got), "unknown_rule", "{rule}");
        let told = got
            .get("result")
            .and_then(|one| one.get("structuredContent"))
            .unwrap();
        assert_eq!(
            written(told.get("rules").unwrap()),
            "[{\"name\":\"sand\",\"params\":0},{\"name\":\"heat\",\"params\":3},{\"name\":\"code\",\"params\":0},{\"name\":\"nature\",\"params\":0}]"
        );
        let message = told.get("message").and_then(Json::text).unwrap();
        assert!(
            message.contains("sand (0 parameters), heat (3 parameters)"),
            "{message}"
        );
    }
    let got = call(
        &mut server,
        "grit_world_create",
        "{\"wide\":16,\"deep\":16,\"tall\":16}",
    );
    assert_eq!(refusal(&got), "unknown_rule");
}

#[test]
fn a_wrong_parameter_count_is_refused_before_any_device_is_asked() {
    let mut server = Server::new(None);
    for (rule, params) in [
        ("heat", ""),
        ("heat", ",\"params\":[]"),
        ("heat", ",\"params\":[600,4]"),
        ("heat", ",\"params\":[600,4,4096,1]"),
        ("sand", ",\"params\":[1]"),
    ] {
        let got = call(
            &mut server,
            "grit_world_create",
            &format!("{{\"wide\":16,\"deep\":16,\"tall\":16,\"rule\":\"{rule}\"{params}}}"),
        );
        assert_eq!(refusal(&got), "params", "{rule}{params}");
        let told = got
            .get("result")
            .and_then(|one| one.get("structuredContent"))
            .unwrap();
        assert!(told.get("rules").is_some());
    }
    let got = call(
        &mut server,
        "grit_world_create",
        "{\"wide\":16,\"deep\":16,\"tall\":16,\"rule\":\"heat\",\"params\":[600,-4,4096]}",
    );
    assert_eq!(refusal(&got), "malformed");
}

#[test]
fn without_a_device_a_world_is_refused_by_name_and_nothing_opens() {
    let mut server = Server::new(None);
    let got = call(
        &mut server,
        "grit_world_create",
        "{\"wide\":16,\"deep\":16,\"tall\":16,\"rule\":\"sand\"}",
    );
    assert_eq!(refusal(&got), "no_compute_device");
    assert_eq!(server.open(), 0);
    let got = call(&mut server, "grit_world_step", "{\"world\":1}");
    assert_eq!(refusal(&got), "no_world");
}

#[test]
fn sizes_the_device_cannot_hold_are_refused_with_the_engines_errors() {
    let Some(host) = device("world sizes") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    for (size, named) in [
        ("65536,65536,65536", "unfit"),
        ("4294967295,2,2", "unfit"),
        ("2048,1024,1024", "too_big"),
        ("16,16,1", "lane"),
        ("16,16,0", "lane"),
    ] {
        let [wide, deep, tall] = [0, 1, 2].map(|at| size.split(',').nth(at).unwrap());
        let got = call(
            &mut server,
            "grit_world_create",
            &format!("{{\"wide\":{wide},\"deep\":{deep},\"tall\":{tall},\"rule\":\"sand\"}}"),
        );
        assert_eq!(refusal(&got), named, "{size}");
    }
    assert_eq!(server.open(), 0);
    let world = created(&mut server, "sand", [2, 2, 2], &[]);
    assert_eq!(read_all(&mut server, world, [2, 2, 2]), [0; 8]);
}

#[test]
fn a_side_is_rounded_up_to_a_power_of_two() {
    let Some(host) = device("world rounding") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let got = answer(&call(
        &mut server,
        "grit_world_create",
        "{\"wide\":9,\"deep\":3,\"tall\":2,\"rule\":\"sand\"}",
    ));
    assert_eq!(numbers(&got, "size"), [16, 4, 2]);
    assert_eq!(got.counted("cells"), Some(128));
    assert_eq!(got.counted("listed"), Some(16));
}

#[test]
fn a_cell_outside_the_world_is_refused_and_writes_nothing() {
    let Some(host) = device("world outside") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let size = [16, 8, 4];
    let world = created(&mut server, "sand", size, &[]);
    for cell in ["[16,0,0,2]", "[0,8,0,2]", "[0,0,4,2]", "[4294967295,0,0,2]"] {
        let got = call(
            &mut server,
            "grit_world_write",
            &format!("{{\"world\":{world},\"cells\":[[1,1,1,2],{cell}]}}"),
        );
        assert_eq!(refusal(&got), "outside", "{cell}");
    }
    assert_eq!(read_all(&mut server, world, size), vec![0; 512]);
    let got = answer(&call(
        &mut server,
        "grit_world_write",
        &format!("{{\"world\":{world},\"cells\":[[15,7,3,3]]}}"),
    ));
    assert_eq!(got.counted("written"), Some(1));
    let all = read_all(&mut server, world, size);
    assert_eq!(all[Grid3::new(16, 8, 4).unwrap().index(15, 7, 3)], 3);
    assert_eq!(all.iter().filter(|one| **one != 0).count(), 1);
    answer(&call(
        &mut server,
        "grit_world_write",
        &format!("{{\"world\":{world},\"cells\":[[2,3,1,1],[2,3,1,2],[2,3,1,3]]}}"),
    ));
    let got = answer(&call(
        &mut server,
        "grit_world_read",
        &format!("{{\"world\":{world},\"from\":[2,3,1],\"to\":[2,3,1]}}"),
    ));
    assert_eq!(numbers(&got, "cells"), [3]);
    for (from, to) in [
        ("[0,0,0]", "[16,0,0]"),
        ("[0,8,0]", "[0,8,0]"),
        ("[0,0,0]", "[0,0,4]"),
    ] {
        let got = call(
            &mut server,
            "grit_world_read",
            &format!("{{\"world\":{world},\"from\":{from},\"to\":{to}}}"),
        );
        assert_eq!(refusal(&got), "outside", "{from} {to}");
    }
    for (axis, at) in [("x", 16), ("y", 8), ("z", 4)] {
        let got = call(
            &mut server,
            "grit_world_slice",
            &format!("{{\"world\":{world},\"axis\":\"{axis}\",\"at\":{at}}}"),
        );
        assert_eq!(refusal(&got), "outside", "{axis} {at}");
        let got = answer(&call(
            &mut server,
            "grit_world_slice",
            &format!(
                "{{\"world\":{world},\"axis\":\"{axis}\",\"at\":{}}}",
                at - 1
            ),
        ));
        assert!(got.get("rows").and_then(Json::list).is_some());
    }
    for cell in ["[1,1,1]", "[1,1,1,2,0]", "7", "[1,1,1,-2]"] {
        let got = call(
            &mut server,
            "grit_world_write",
            &format!("{{\"world\":{world},\"cells\":[{cell}]}}"),
        );
        assert_eq!(refusal(&got), "malformed", "{cell}");
    }
}

#[test]
fn a_read_box_past_the_cap_is_refused_and_one_at_the_cap_is_not() {
    let Some(host) = device("world read cap") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let world = created(&mut server, "sand", [32, 16, 16], &[]);
    let got = answer(&call(
        &mut server,
        "grit_world_read",
        &format!("{{\"world\":{world},\"from\":[16,0,0],\"to\":[31,15,15]}}"),
    ));
    assert_eq!(numbers(&got, "cells").len() as u64, READ_MOST);
    assert_eq!(
        written(got.get("kinds").unwrap()),
        format!("[{{\"kind\":0,\"cells\":{READ_MOST}}}]")
    );
    let got = call(
        &mut server,
        "grit_world_read",
        &format!("{{\"world\":{world},\"from\":[15,0,0],\"to\":[31,15,15]}}"),
    );
    assert_eq!(refusal(&got), "too_many");
    let got = call(
        &mut server,
        "grit_world_read",
        &format!("{{\"world\":{world},\"from\":[0,0,0],\"to\":[31,15,15]}}"),
    );
    assert_eq!(refusal(&got), "too_many");
    let got = call(
        &mut server,
        "grit_world_read",
        &format!("{{\"world\":{world},\"from\":[3,0,0],\"to\":[2,0,0]}}"),
    );
    assert_eq!(refusal(&got), "reversed");
    let many: Vec<(u32, u32, u32, u32)> = (0..=READ_MOST as u32)
        .map(|at| (at % 32, (at / 32) % 16, at / 512, 2))
        .collect();
    let got = call(
        &mut server,
        "grit_world_write",
        &format!("{{\"world\":{world},\"cells\":{}}}", writing(&many)),
    );
    assert_eq!(refusal(&got), "too_many");
    let got = answer(&call(
        &mut server,
        "grit_world_write",
        &format!(
            "{{\"world\":{world},\"cells\":{}}}",
            writing(&many[..READ_MOST as usize])
        ),
    ));
    assert_eq!(got.counted("written"), Some(READ_MOST as u32));
}

#[test]
fn a_step_on_a_closed_world_is_refused_and_another_world_runs_on() {
    let Some(host) = device("world closed") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let first = created(&mut server, "sand", [8, 8, 8], &[]);
    let second = created(&mut server, "heat", [8, 8, 8], &heat::TUNED);
    assert_eq!((first, second), (1, 2));
    assert_eq!(server.open(), 2);
    let got = answer(&call(
        &mut server,
        "grit_world_close",
        &format!("{{\"world\":{first}}}"),
    ));
    assert_eq!(got.get("closed"), Some(&Json::Bool(true)));
    assert_eq!(server.open(), 1);
    for (tool, rest) in [
        ("grit_world_step", ""),
        ("grit_world_step", ",\"frames\":5"),
        ("grit_world_write", ",\"cells\":[[0,0,0,2]]"),
        ("grit_world_read", ",\"from\":[0,0,0],\"to\":[0,0,0]"),
        ("grit_world_slice", ",\"at\":0"),
        ("grit_world_close", ""),
    ] {
        let got = call(&mut server, tool, &format!("{{\"world\":{first}{rest}}}"));
        assert_eq!(refusal(&got), "closed", "{tool}");
    }
    let got = call(&mut server, "grit_world_step", "{\"world\":3}");
    assert_eq!(refusal(&got), "no_world");
    let got = call(&mut server, "grit_world_step", "{}");
    assert_eq!(refusal(&got), "malformed");
    assert_eq!(step(&mut server, second, 3).counted("frame"), Some(3));
    assert_eq!(created(&mut server, "sand", [2, 2, 2], &[]), 3);
}

#[test]
fn a_step_of_no_frames_or_too_many_is_refused_before_it_runs() {
    let Some(host) = device("world frames") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let world = created(&mut server, "sand", [8, 8, 8], &[]);
    for frames in [
        "0".to_owned(),
        "-1".to_owned(),
        (STEP_MOST + 1).to_string(),
        i64::MAX.to_string(),
    ] {
        let got = call(
            &mut server,
            "grit_world_step",
            &format!("{{\"world\":{world},\"frames\":{frames}}}"),
        );
        assert_eq!(refusal(&got), "frames", "{frames}");
    }
    let got = call(
        &mut server,
        "grit_world_step",
        &format!("{{\"world\":{world},\"frames\":\"3\"}}"),
    );
    assert_eq!(refusal(&got), "malformed");
    assert_eq!(step(&mut server, world, 1).counted("frame"), Some(1));
    assert_eq!(
        answer(&call(
            &mut server,
            "grit_world_step",
            &format!("{{\"world\":{world}}}")
        ))
        .counted("frame"),
        Some(2)
    );
    let got = step(&mut server, world, 130);
    assert_eq!(got.counted("frame"), Some(132));
    assert_eq!(numbers(&got, "tally").len(), 7);
}

#[test]
fn a_slice_names_its_axes_and_refuses_an_axis_it_does_not_have() {
    let Some(host) = device("world slice") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let world = created(&mut server, "heat", [4, 8, 2], &heat::TUNED);
    let word = heat::cell(heat::FIRE, 900);
    answer(&call(
        &mut server,
        "grit_world_write",
        &format!(
            "{{\"world\":{world},\"cells\":[[3,5,1,{word}],[0,0,0,{}]]}}",
            heat::cell(heat::KINDS + 3, 0)
        ),
    ));
    let slice = |server: &mut Server<'_>, axis: &str, at: u32, shown: &str| {
        answer(&call(
            server,
            "grit_world_slice",
            &format!("{{\"world\":{world},\"axis\":\"{axis}\",\"at\":{at},\"as\":\"{shown}\"}}"),
        ))
    };
    let got = slice(&mut server, "z", 1, "kinds");
    assert_eq!(got.get("columns").and_then(Json::text), Some("x"));
    assert_eq!(got.get("rows_along").and_then(Json::text), Some("y"));
    assert_eq!(
        written(got.get("rows").unwrap()),
        "[\"0000\",\"0000\",\"0000\",\"0000\",\"0000\",\"0006\",\"0000\",\"0000\"]"
    );
    let got = slice(&mut server, "x", 3, "words");
    assert_eq!(got.get("columns").and_then(Json::text), Some("y"));
    assert_eq!(got.get("rows_along").and_then(Json::text), Some("z"));
    assert_eq!(
        written(got.get("rows").unwrap()),
        format!("[[0,0,0,0,0,0,0,0],[0,0,0,0,0,{word},0,0]]")
    );
    let got = slice(&mut server, "y", 0, "kinds");
    assert_eq!(got.get("columns").and_then(Json::text), Some("x"));
    assert_eq!(got.get("rows_along").and_then(Json::text), Some("z"));
    assert_eq!(written(got.get("rows").unwrap()), "[\"b000\",\"0000\"]");
    for axis in ["w", "X", ""] {
        let got = call(
            &mut server,
            "grit_world_slice",
            &format!("{{\"world\":{world},\"axis\":\"{axis}\",\"at\":0}}"),
        );
        assert_eq!(refusal(&got), "axis", "{axis}");
    }
    let got = call(
        &mut server,
        "grit_world_slice",
        &format!("{{\"world\":{world},\"at\":0,\"as\":\"pixels\"}}"),
    );
    assert_eq!(refusal(&got), "malformed");
}

/// The heat rule driven through the tools, create, write, step 64 and read
/// all, equals the CPU reference of the same world and writes.
#[test]
fn a_heat_world_run_through_the_tools_equals_the_cpu_reference() {
    let Some(host) = device("world heat reference") else {
        return;
    };
    let size = [16, 16, 16];
    let grid = Grid3::new(16, 16, 16).unwrap();
    for seed in [1_u64, 0x5EED] {
        let mut server = Server::new(Some(&host));
        let world = created(&mut server, "heat", size, &heat::TUNED);
        let mut state = seed;
        let mut cells = Vec::new();
        for z in 0..16 {
            for y in 0..16 {
                for x in 0..16 {
                    cells.push((x, y, z, heat_cell(&mut state)));
                }
            }
        }
        answer(&call(
            &mut server,
            "grit_world_write",
            &format!("{{\"world\":{world},\"cells\":{}}}", writing(&cells)),
        ));
        let start: Vec<u32> = cells.iter().map(|one| one.3).collect();
        let got = step(&mut server, world, 64);
        assert_eq!(got.counted("frame"), Some(64));
        let want = stepped("heat", grid, &start, &heat::TUNED, 0, 64);
        assert_ne!(want, start, "nothing moved");
        assert_eq!(read_all(&mut server, world, size), want, "seed {seed:#x}");
        let layer = answer(&call(
            &mut server,
            "grit_world_slice",
            &format!("{{\"world\":{world},\"axis\":\"z\",\"at\":9}}"),
        ));
        let rows: Vec<String> = (0..16)
            .map(|y| {
                (0..16)
                    .map(|x| {
                        let kind = heat::kind_of(want[grid.index(x, y, 9)]);
                        char::from(b"0123456789abcdef"[kind as usize])
                    })
                    .collect()
            })
            .collect();
        let held: Vec<&str> = layer
            .get("rows")
            .and_then(Json::list)
            .unwrap()
            .iter()
            .map(|one| one.text().unwrap())
            .collect();
        assert_eq!(held, rows);
    }
}

/// A small scene of the nature rule driven through the tools, create,
/// write, step and read: rock under earth, a tree whose crown burns, a
/// column of sand and one of snow in the air, a pool of water and a cloud
/// of steam. Every frame read equals the CPU reference, the kinds read by
/// slice equal the reference's, and the scene changes as the rule says:
/// the fire spreads and burns out to ash, smoke rises and fades, sand and
/// snow fall, and the steam condenses.
#[test]
fn a_nature_scene_run_through_the_tools_equals_the_cpu_reference() {
    let Some(host) = device("world nature scene") else {
        return;
    };
    let size = [16, 16, 16];
    let grid = Grid3::new(16, 16, 16).unwrap();
    let warmth = 300;
    let mut cells = Vec::new();
    for z in 0..16 {
        for x in 0..16 {
            cells.push((x, 15, z, nature::cell(nature::ROCK, warmth)));
            cells.push((x, 14, z, nature::cell(nature::EARTH, warmth)));
        }
    }
    for y in 10..14 {
        cells.push((4, y, 4, nature::cell(nature::WOOD, warmth)));
    }
    for z in 3..6 {
        for x in 3..6 {
            for y in 8..10 {
                cells.push((x, y, z, nature::cell(nature::LEAVES, warmth)));
            }
        }
    }
    cells.push((
        4,
        9,
        4,
        nature::alive(nature::FIRE, 900, nature::LEAVES_LIFE),
    ));
    for y in 2..6 {
        cells.push((10, y, 10, nature::cell(nature::SAND, warmth)));
        cells.push((12, y, 4, nature::cell(nature::SNOW, 250)));
    }
    for z in 9..12 {
        for x in 2..5 {
            cells.push((x, 13, z, nature::cell(nature::WATER, warmth)));
            cells.push((x + 8, 12, z, nature::cell(nature::STEAM, 340)));
        }
    }
    let mut server = Server::new(Some(&host));
    let world = created(&mut server, "nature", size, &[]);
    answer(&call(
        &mut server,
        "grit_world_write",
        &format!("{{\"world\":{world},\"cells\":{}}}", writing(&cells)),
    ));
    let mut want = vec![0_u32; grid.cells()];
    for &(x, y, z, word) in &cells {
        want[grid.index(x, y, z)] = word;
    }
    let start = want.clone();
    let count = |cells: &[u32], kind: u32| {
        cells
            .iter()
            .filter(|&&word| nature::kind_of(word) == kind)
            .count()
    };
    let mut smoked = 0;
    for batch in 0..8 {
        let got = step(&mut server, world, 16);
        assert_eq!(got.counted("frame"), Some(16 * (batch + 1)));
        want = stepped("nature", grid, &want, &[], 16 * batch, 16);
        assert_eq!(read_all(&mut server, world, size), want, "batch {batch}");
        smoked = smoked.max(count(&want, nature::SMOKE));
    }
    let layer = answer(&call(
        &mut server,
        "grit_world_slice",
        &format!("{{\"world\":{world},\"axis\":\"z\",\"at\":4}}"),
    ));
    let rows: Vec<String> = (0..16)
        .map(|y| {
            (0..16)
                .map(|x| {
                    let kind = nature::kind_of(want[grid.index(x, y, 4)]);
                    char::from(b"0123456789abcdef"[kind as usize])
                })
                .collect()
        })
        .collect();
    let held: Vec<&str> = layer
        .get("rows")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .map(|one| one.text().unwrap())
        .collect();
    assert_eq!(held, rows);
    let burnt = count(&start, nature::WOOD) + count(&start, nature::LEAVES)
        - count(&want, nature::WOOD)
        - count(&want, nature::LEAVES);
    println!(
        "after 128 frames: {burnt} fuel burned, {} ash, most smoke {smoked}, {} fire, {} smoke, {} steam",
        count(&want, nature::ASH),
        count(&want, nature::FIRE),
        count(&want, nature::SMOKE),
        count(&want, nature::STEAM)
    );
    assert!(burnt > 9, "the fire did not spread");
    assert_eq!(
        count(&want, nature::ASH),
        burnt + 1,
        "the fuel and the first fire"
    );
    assert!(smoked > 0);
    assert_eq!(count(&want, nature::STEAM), 0, "the steam condensed");
    assert_eq!(count(&want, nature::SAND), 4);
    assert_eq!(nature::kind_of(want[grid.index(10, 13, 10)]), nature::SAND);
    assert_ne!(want, start);
}

/// Words a rule is written into a quiet world with: any word, a box of
/// them; for the code rule, an instruction that acts or is acted on, with
/// any way, payload and spare bit.
fn wild(state: &mut u64, rule: &str) -> u32 {
    match rule {
        "heat" => heat_cell(state),
        "code" => {
            let roll = next(state);
            let does = [code::COPY, MOVE, ADD, DATA, code::TURN][(roll % 5) as usize];
            let way = ((roll >> 8) % 8) as u32;
            code::word(does, way, (roll >> 16) as u32) | (((roll >> 63) as u32) << 31)
        }
        _ => (next(state) & 0xFFFF_FFFF) as u32,
    }
}

/// A small code program run through the tools, create, write, step and
/// read, ends in the world worked out by hand:
///
/// - an ADD at (2, 2, 3) facing +x walks along the DATA cells 5, 7, 11 and
///   13, one a frame, and stops before the HALT at (7, 2, 3) holding 36;
/// - a MOVE at (12, 4, 1) facing +z waits one frame for an oct in which it
///   faces +z, moves three cells, and stops as DATA before the HALT at
///   (12, 4, 5).
#[test]
fn a_code_program_run_through_the_tools_ends_in_the_world_worked_out_by_hand() {
    let Some(host) = device("world code program") else {
        return;
    };
    let size = [16, 8, 8];
    let grid = Grid3::new(16, 8, 8).unwrap();
    let packet = 0x00AB_CDEF;
    let program = [
        (2, 2, 3, code::word(ADD, PLUS_X, 0)),
        (3, 2, 3, code::word(DATA, 0, 5)),
        (4, 2, 3, code::word(DATA, 0, 7)),
        (5, 2, 3, code::word(DATA, 0, 11)),
        (6, 2, 3, code::word(DATA, 0, 13)),
        (7, 2, 3, code::word(HALT, 0, 0)),
        (12, 4, 1, code::word(MOVE, PLUS_Z, packet)),
        (12, 4, 5, code::word(HALT, 0, 0)),
    ];
    let world_after = |cells: &[(u32, u32, u32, u32)]| {
        let mut world = vec![0_u32; grid.cells()];
        for &(x, y, z, word) in cells {
            world[grid.index(x, y, z)] = word;
        }
        world
    };

    let mut server = Server::new(Some(&host));
    let world = created(&mut server, "code", size, &[]);
    answer(&call(
        &mut server,
        "grit_world_write",
        &format!("{{\"world\":{world},\"cells\":{}}}", writing(&program)),
    ));

    let first = step(&mut server, world, 1);
    assert_eq!(numbers(&first, "tally"), [2, 1, 3, 0, 0, 0, 1]);
    assert_eq!(
        read_all(&mut server, world, size),
        world_after(&[
            (3, 2, 3, code::word(ADD, PLUS_X, 5)),
            (4, 2, 3, code::word(DATA, 0, 7)),
            (5, 2, 3, code::word(DATA, 0, 11)),
            (6, 2, 3, code::word(DATA, 0, 13)),
            (7, 2, 3, code::word(HALT, 0, 0)),
            (12, 4, 1, code::word(MOVE, PLUS_Z, packet)),
            (12, 4, 5, code::word(HALT, 0, 0)),
        ])
    );

    step(&mut server, world, 1);
    assert_eq!(
        read_all(&mut server, world, size),
        world_after(&[
            (4, 2, 3, code::word(ADD, PLUS_X, 12)),
            (5, 2, 3, code::word(DATA, 0, 11)),
            (6, 2, 3, code::word(DATA, 0, 13)),
            (7, 2, 3, code::word(HALT, 0, 0)),
            (12, 4, 2, code::word(MOVE, PLUS_Z, packet)),
            (12, 4, 5, code::word(HALT, 0, 0)),
        ])
    );

    let done = world_after(&[
        (6, 2, 3, code::word(ADD, PLUS_X, 36)),
        (7, 2, 3, code::word(HALT, 0, 0)),
        (12, 4, 4, code::word(DATA, PLUS_Z, packet)),
        (12, 4, 5, code::word(HALT, 0, 0)),
    ]);
    let got = step(&mut server, world, 3);
    assert_eq!(got.counted("frame"), Some(5));
    assert_eq!(read_all(&mut server, world, size), done);
    let later = step(&mut server, world, 30);
    assert_eq!(later.counted("frame"), Some(35));
    assert_eq!(later.counted("listed"), Some(0));
    assert_eq!(read_all(&mut server, world, size), done);
}

/// Every registered rule: a world that has fallen quiet, a write into it
/// through the tools, and the frames after it equal the CPU reference. A
/// write that does not wake what it touches is never stepped.
#[test]
fn a_write_into_a_quiet_world_is_stepped_under_every_rule() {
    let Some(host) = device("world quiet write") else {
        return;
    };
    let size = [16, 8, 8];
    let grid = Grid3::new(16, 8, 8).unwrap();
    for rule in rules() {
        let params: Vec<u32> = (0..rule.params())
            .map(|at| heat::TUNED.get(at).copied().unwrap_or(1))
            .collect();
        let mut server = Server::new(Some(&host));
        let world = created(&mut server, rule.name(), size, &params);
        let quiet = step(&mut server, world, 2);
        assert_eq!(quiet.counted("listed"), Some(0), "{}", rule.name());
        let mut state = 0x0DD_5EED;
        let mut cells = Vec::new();
        for (x, y, z) in [(5, 2, 3), (6, 2, 3), (5, 3, 4), (6, 3, 4), (11, 6, 7)] {
            cells.push((x, y, z, wild(&mut state, rule.name())));
        }
        let wrote = answer(&call(
            &mut server,
            "grit_world_write",
            &format!("{{\"world\":{world},\"cells\":{}}}", writing(&cells)),
        ));
        let mut start = vec![0_u32; grid.cells()];
        for (x, y, z, word) in &cells {
            start[grid.index(*x, *y, *z)] = *word;
        }
        assert_eq!(
            stepped(rule.name(), grid, &vec![0; grid.cells()], &params, 0, 2),
            vec![0; grid.cells()]
        );
        let got = step(&mut server, world, 32);
        assert_eq!(got.counted("frame"), Some(34));
        let want = stepped(rule.name(), grid, &start, &params, 2, 32);
        assert_ne!(want, start, "{}: nothing moved", rule.name());
        assert_eq!(read_all(&mut server, world, size), want, "{}", rule.name());
        assert!(wrote.counted("listed").unwrap() > 0, "{}", rule.name());
    }
}

fn fnv(held: &str, hash: u64) -> u64 {
    held.bytes().fold(hash, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01B3)
    })
}

/// The hash of every answer of [`script`]; the same on every card.
const SCRIPT_HASH: u64 = 0x2276_1AFB_7B1C_8BAB;

/// A fixed script of calls on a heat world and a sand world: the hash of
/// every answer, which holds the reads, the tallies and the list lengths.
fn script(host: &Host) -> u64 {
    let mut server = Server::new(Some(host));
    let mut hash = 0xCBF2_9CE4_8422_2325;
    let mut state = 0x5C41_9700_u64;
    let mut run = |server: &mut Server<'_>, name: &str, arguments: String| {
        let got = answer(&call(server, name, &arguments));
        hash = fnv(&written(&got), hash);
        got
    };
    let heat_world = run(
        &mut server,
        "grit_world_create",
        format!(
            "{{\"wide\":16,\"deep\":8,\"tall\":16,\"rule\":\"heat\",\"params\":{:?}}}",
            heat::TUNED
        ),
    )
    .counted("world")
    .unwrap();
    let sand_world = run(
        &mut server,
        "grit_world_create",
        "{\"wide\":8,\"deep\":16,\"tall\":8,\"rule\":\"sand\"}".to_owned(),
    )
    .counted("world")
    .unwrap();
    let heated: Vec<(u32, u32, u32, u32)> = (0..600)
        .map(|_| {
            let at = next(&mut state);
            (
                (at % 16) as u32,
                ((at >> 8) % 8) as u32,
                ((at >> 16) % 16) as u32,
                heat_cell(&mut state),
            )
        })
        .collect();
    let poured: Vec<(u32, u32, u32, u32)> = (0..200)
        .map(|_| {
            let at = next(&mut state);
            let word =
                [grit::sand::SAND, grit::sand::WATER, grit::sand::WALL][(at >> 32) as usize % 3];
            (
                (at % 8) as u32,
                ((at >> 8) % 8) as u32,
                ((at >> 16) % 8) as u32,
                word,
            )
        })
        .collect();
    for (world, cells) in [(heat_world, &heated), (sand_world, &poured)] {
        run(
            &mut server,
            "grit_world_write",
            format!("{{\"world\":{world},\"cells\":{}}}", writing(cells)),
        );
    }
    for frames in [1, 7, 50] {
        for world in [heat_world, sand_world] {
            run(
                &mut server,
                "grit_world_step",
                format!("{{\"world\":{world},\"frames\":{frames}}}"),
            );
            run(
                &mut server,
                "grit_world_slice",
                format!("{{\"world\":{world},\"axis\":\"z\",\"at\":3}}"),
            );
        }
        run(
            &mut server,
            "grit_world_read",
            format!("{{\"world\":{heat_world},\"from\":[0,0,0],\"to\":[15,7,15]}}"),
        );
        run(
            &mut server,
            "grit_world_read",
            format!("{{\"world\":{sand_world},\"from\":[0,0,0],\"to\":[7,15,7]}}"),
        );
        run(
            &mut server,
            "grit_world_slice",
            format!("{{\"world\":{sand_world},\"axis\":\"x\",\"at\":4,\"as\":\"words\"}}"),
        );
        run(
            &mut server,
            "grit_world_write",
            format!(
                "{{\"world\":{heat_world},\"cells\":[[{at},2,{at},{}]]}}",
                heat::cell(heat::FIRE, 900) | (heat::LIFE << 4),
                at = frames.min(15)
            ),
        );
    }
    run(
        &mut server,
        "grit_world_close",
        format!("{{\"world\":{heat_world}}}"),
    );
    run(
        &mut server,
        "grit_world_step",
        format!("{{\"world\":{sand_world},\"frames\":100}}"),
    );
    run(
        &mut server,
        "grit_world_read",
        format!("{{\"world\":{sand_world},\"from\":[0,0,0],\"to\":[7,15,7]}}"),
    );
    hash
}

#[test]
fn a_fixed_script_of_calls_reads_the_same_twice_and_on_every_card() {
    let Some(host) = device("world script") else {
        return;
    };
    let first = script(&host);
    let second = script(&host);
    println!(
        "world script {} width {}: hash {first:#018x}",
        host.name(),
        host.width()
    );
    assert_eq!(first, second);
    assert_eq!(first, SCRIPT_HASH, "{first:#018x}");
}
