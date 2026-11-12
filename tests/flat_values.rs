#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use grit::Host;
use grit::serve::handle;
use grit::wire::{Json, read, written};

fn answered(host: Option<&Host>, name: &str, arguments: &str) -> Json {
    let asked = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{{\"name\":\"{name}\",\"arguments\":{arguments}}}}}"
    );
    handle(host, &read(&asked).unwrap()).unwrap()
}

fn structured(host: Option<&Host>, name: &str, arguments: &str) -> Json {
    let answer = answered(host, name, arguments);
    let result = answer.get("result").unwrap();
    assert_ne!(
        result.get("isError"),
        Some(&Json::Bool(true)),
        "{name} {arguments}: {}",
        written(&answer)
    );
    result.get("structuredContent").cloned().unwrap()
}

fn refused(name: &str, arguments: &str) -> (String, String) {
    let answer = answered(None, name, arguments);
    let told = answer
        .get("result")
        .and_then(|one| one.get("structuredContent"))
        .unwrap_or_else(|| panic!("{name} {arguments}: {}", written(&answer)));
    (
        told.get("code")
            .and_then(Json::text)
            .unwrap_or("")
            .to_owned(),
        told.get("field")
            .and_then(Json::text)
            .unwrap_or("")
            .to_owned(),
    )
}

const TABLE: &str = "\"inputs\":2,\"outputs\":1,\"gates\":1,\"table\":[0,1,1,0]";

#[test]
fn a_value_of_the_wrong_type_or_range_is_refused_at_its_field_before_any_device() {
    let want = "\"want\":[{\"at\":0,\"is\":1}]";
    let cases: Vec<(&str, String, &str)> = vec![
        ("grit_slope", "{\"wide\":1}".into(), "$.wide"),
        ("grit_slope", "{\"wide\":512}".into(), "$.wide"),
        ("grit_slope", "{\"deep\":\"32\"}".into(), "$.deep"),
        ("grit_slope", "{\"kind\":5}".into(), "$.kind"),
        ("grit_slope", "{\"kind\":\"treacle\"}".into(), "$.kind"),
        ("grit_slope", "{\"turning\":-1}".into(), "$.turning"),
        ("grit_slope", "{\"swing\":4294967296}".into(), "$.swing"),
        ("grit_slope", "{\"depth\":0}".into(), "$.depth"),
        ("grit_slope", "{\"depth\":4097}".into(), "$.depth"),
        ("grit_slope", "{\"most\":0}".into(), "$.most"),
        ("grit_slope", "{\"at\":-1}".into(), "$.at"),
        ("grit_slope", "{\"at\":1024}".into(), "$.at"),
        ("grit_slope", "{\"fill\":2147483648}".into(), "$.fill"),
        ("grit_slope", "{\"carry\":0}".into(), "$.carry"),
        ("grit_slope", "{\"cells\":{}}".into(), "$.cells"),
        ("grit_slope", "{\"cells\":[{\"is\":1}]}".into(), "$.cells[0].at"),
        ("grit_slope", "{\"cells\":[{\"at\":1024,\"is\":1}]}".into(), "$.cells[0].at"),
        ("grit_slope", "{\"cells\":[{\"at\":3,\"is\":4294967295}]}".into(), "$.cells[0].is"),
        ("grit_slope", "{\"cells\":[{\"at\":3,\"is\":\"1\"}]}".into(), "$.cells[0].is"),
        ("grit_settle", "{}".into(), "$.want"),
        ("grit_settle", format!("{{{want},\"rounds\":0}}"), "$.rounds"),
        ("grit_settle", format!("{{{want},\"rounds\":1025}}"), "$.rounds"),
        ("grit_settle", format!("{{{want},\"step\":0}}"), "$.step"),
        ("grit_settle", format!("{{{want},\"ceiling\":-1}}"), "$.ceiling"),
        ("grit_settle", format!("{{{want},\"ceiling\":10}}"), "$.turning"),
        ("grit_settle", format!("{{{want},\"ceiling\":20,\"swing\":21}}"), "$.swing"),
        ("grit_settle", "{\"want\":[{\"at\":0,\"is\":-2147483649}]}".into(), "$.want[0].is"),
        ("grit_settle", "{\"want\":[{\"at\":2048,\"is\":1}]}".into(), "$.want[0].at"),
        ("grit_settle", "{\"want\":7}".into(), "$.want"),
        ("grit_pour", "{\"wide\":8}".into(), "$.wide"),
        ("grit_pour", "{\"deep\":2048}".into(), "$.deep"),
        ("grit_pour", "{\"frames\":0}".into(), "$.frames"),
        ("grit_pour", "{\"batch\":257}".into(), "$.batch"),
        ("grit_pour", "{\"most\":0}".into(), "$.most"),
        ("grit_pour", "{\"walls\":\"no\"}".into(), "$.walls"),
        ("grit_pour", "{\"cells\":[{\"at\":1,\"is\":3}]}".into(), "$.cells[0].is"),
        ("grit_pour", "{\"cells\":[{\"at\":16384,\"is\":\"sand\"}]}".into(), "$.cells[0].at"),
        ("grit_pour", "{\"brush\":5}".into(), "$.brush"),
        ("grit_pour", "{\"brush\":{\"at\":16384}}".into(), "$.brush.at"),
        ("grit_pour", "{\"brush\":{\"at\":64,\"span\":-3}}".into(), "$.brush.span"),
        ("grit_pour", "{\"brush\":{\"at\":64,\"is\":\"lava\"}}".into(), "$.brush.is"),
        ("grit_pour", "{\"frames\":100,\"brush\":{\"at\":64,\"frames\":101}}".into(), "$.brush.frames"),
        ("grit_synthesise", format!("{{{TABLE},\"crowd\":63}}"), "$.crowd"),
        ("grit_synthesise", format!("{{{TABLE},\"crowd\":65537}}"), "$.crowd"),
        ("grit_synthesise", format!("{{{TABLE},\"rounds\":0}}"), "$.rounds"),
        ("grit_synthesise", format!("{{{TABLE},\"seed\":-1}}"), "$.seed"),
        ("grit_synthesise", format!("{{{TABLE},\"stall\":-5}}"), "$.stall"),
        ("grit_synthesise", format!("{{{TABLE},\"shake\":65}}"), "$.shake"),
        ("grit_synthesise", "{\"outputs\":1,\"gates\":1,\"table\":[0,1,1,0]}".into(), "$.inputs"),
        ("grit_synthesise", "{\"inputs\":2,\"outputs\":1,\"gates\":1}".into(), "$.table"),
        ("grit_least", format!("{{{TABLE},\"crowd\":0}}"), "$.crowd"),
        ("grit_check", "{\"inputs\":2,\"gates\":1,\"table\":[0,1,1,0],\"chip\":[]}".into(), "$.outputs"),
        ("grit_check", "{\"inputs\":2,\"outputs\":1,\"gates\":1,\"table\":[0,1,1,2],\"chip\":[{\"kind\":6,\"left\":0,\"right\":1}]}".into(), "$.table[3]"),
        ("grit_check", "{\"inputs\":2,\"outputs\":1,\"gates\":1,\"table\":[0,1,1],\"chip\":[{\"kind\":6,\"left\":0,\"right\":1}]}".into(), "$.table"),
        ("grit_synthesise", "{\"inputs\":2,\"outputs\":2,\"gates\":2,\"table\":[0,1,4,0]}".into(), "$.table[2]"),
        ("grit_machine", "{\"state\":0,\"inputs\":1,\"outputs\":1,\"gates\":1,\"traces\":[{\"feed\":[0,1],\"want\":[2,3]}],\"chip\":[{\"kind\":10,\"left\":0,\"right\":0}]}".into(), "$.traces[0].want[0]"),
        ("grit_machine", "{\"state\":0,\"inputs\":1,\"outputs\":1,\"gates\":1,\"traces\":[{\"feed\":[0,2],\"want\":[0,1]}],\"chip\":[{\"kind\":10,\"left\":0,\"right\":0}]}".into(), "$.traces[0].feed[1]"),
        ("grit_machine", "{\"state\":0,\"inputs\":1,\"outputs\":1,\"gates\":1,\"traces\":[{\"feed\":[],\"want\":[0,1]}],\"chip\":[{\"kind\":10,\"left\":0,\"right\":0}]}".into(), "$.traces[0].feed"),
        ("grit_machine", "{\"state\":0,\"inputs\":1,\"outputs\":1,\"gates\":1,\"traces\":[{\"feed\":[0,1,1],\"want\":[0,1]}],\"chip\":[{\"kind\":10,\"left\":0,\"right\":0}]}".into(), "$.traces[0].feed"),
        ("grit_machine", "{\"state\":0,\"inputs\":1,\"outputs\":1,\"gates\":1,\"steps\":3,\"traces\":[{\"feed\":[0,1],\"want\":[0,1]}],\"chip\":[{\"kind\":10,\"left\":0,\"right\":0}]}".into(), "$.traces[0].want"),
        ("grit_machine", "{\"inputs\":1,\"outputs\":1,\"gates\":4,\"traces\":[{\"feed\":[1],\"want\":[1]}]}".into(), "$.state"),
        ("grit_machine", "{\"state\":1,\"inputs\":1,\"outputs\":1,\"gates\":4}".into(), "$.traces"),
        ("grit_machine", "{\"state\":1,\"inputs\":1,\"outputs\":1,\"gates\":4,\"steps\":-1,\"traces\":[{\"feed\":[1],\"want\":[1]}]}".into(), "$.steps"),
        ("grit_machine", "{\"state\":1,\"inputs\":1,\"outputs\":1,\"gates\":4,\"crowd\":70000,\"traces\":[{\"feed\":[1],\"want\":[1]}]}".into(), "$.crowd"),
    ];
    for (name, arguments, field) in &cases {
        let (code, at) = refused(name, arguments);
        assert_eq!(
            (code.as_str(), at.as_str()),
            ("malformed", *field),
            "{name} {arguments}"
        );
    }
}

fn properties(name: &str) -> Vec<(String, Json)> {
    let answer = handle(
        None,
        &read("{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}").unwrap(),
    )
    .unwrap();
    let tool = answer
        .get("result")
        .and_then(|one| one.get("tools"))
        .and_then(Json::list)
        .unwrap()
        .iter()
        .find(|one| one.get("name").and_then(Json::text) == Some(name))
        .cloned()
        .unwrap();
    match tool
        .get("inputSchema")
        .and_then(|one| one.get("properties"))
    {
        Some(Json::Map(held)) => held.clone(),
        _ => panic!("{name} has no properties"),
    }
}

fn defaults(name: &str) -> Vec<(String, Json)> {
    properties(name)
        .into_iter()
        .filter_map(|(key, one)| one.get("default").cloned().map(|held| (key, held)))
        .collect()
}

#[test]
fn every_value_a_tool_fills_in_for_itself_is_published() {
    let named =
        |name: &str| -> Vec<String> { defaults(name).into_iter().map(|(key, _)| key).collect() };
    for (name, keys) in [
        (
            "grit_slope",
            &[
                "wide", "deep", "kind", "turning", "swing", "depth", "fill", "carry", "most",
            ][..],
        ),
        (
            "grit_settle",
            &[
                "wide", "deep", "kind", "turning", "swing", "depth", "rounds", "step", "ceiling",
                "fill",
            ][..],
        ),
        (
            "grit_pour",
            &["wide", "deep", "frames", "walls", "most", "batch"][..],
        ),
        (
            "grit_synthesise",
            &["crowd", "rounds", "seed", "stall", "shake"][..],
        ),
        (
            "grit_least",
            &["crowd", "rounds", "seed", "stall", "shake"][..],
        ),
        (
            "grit_machine",
            &["crowd", "rounds", "seed", "stall", "shake"][..],
        ),
    ] {
        let published = named(name);
        for key in keys {
            assert!(
                published.iter().any(|one| one == key),
                "{name} fills in {key} and does not say with what: {published:?}"
            );
        }
    }
    let slope = properties("grit_slope");
    let is = slope
        .iter()
        .find(|(key, _)| key == "cells")
        .and_then(|(_, one)| one.get("items"))
        .and_then(|one| one.get("properties"))
        .and_then(|one| one.get("is"))
        .unwrap();
    assert_eq!(
        is.get("minimum").and_then(Json::int),
        Some(i64::from(i32::MIN))
    );
    assert_eq!(
        is.get("maximum").and_then(Json::int),
        Some(i64::from(i32::MAX))
    );
}

fn spelt(held: &[(String, Json)]) -> String {
    let parts: Vec<String> = held
        .iter()
        .map(|(key, one)| format!("\"{key}\":{}", written(one)))
        .collect();
    parts.join(",")
}

#[test]
fn a_value_left_out_is_the_published_default() {
    let Some(host) = device("a_value_left_out_is_the_published_default") else {
        return;
    };
    let cells = "\"cells\":[{\"at\":300,\"is\":-4000},{\"at\":301,\"is\":5000}]";
    let slope = spelt(&defaults("grit_slope"));
    assert_eq!(
        written(&structured(
            Some(&host),
            "grit_slope",
            &format!("{{{cells}}}")
        )),
        written(&structured(
            Some(&host),
            "grit_slope",
            &format!("{{{cells},{slope}}}")
        )),
        "slope with its defaults spelt out: {slope}"
    );
    let want = "\"want\":[{\"at\":300,\"is\":-3}]";
    let settle = spelt(&defaults("grit_settle"));
    assert_eq!(
        written(&structured(
            Some(&host),
            "grit_settle",
            &format!("{{{cells},{want}}}")
        )),
        written(&structured(
            Some(&host),
            "grit_settle",
            &format!("{{{cells},{want},{settle}}}")
        )),
        "settle with its defaults spelt out: {settle}"
    );
    let pour = spelt(&defaults("grit_pour"));
    let placed = "\"frames\":40,\"cells\":[{\"at\":200}],\"brush\":{\"at\":70,\"frames\":20,\"span\":49,\"is\":\"sand\"}";
    let pour = pour.replace("\"frames\":200,", "");
    assert_eq!(
        written(&structured(
            Some(&host),
            "grit_pour",
            "{\"frames\":40,\"cells\":[{\"at\":200}],\"brush\":{\"at\":70}}"
        )),
        written(&structured(
            Some(&host),
            "grit_pour",
            &format!("{{{placed},{pour}}}")
        )),
        "pour with its defaults spelt out: {pour}"
    );
}

struct Roll(u32);

impl Roll {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }
}

fn scattered(trial: u32) -> (String, (u32, u32)) {
    let mut roll = Roll((0x9e37_79b9 ^ (trial + 1).wrapping_mul(0x85eb_ca6b)) | 1);
    for _ in 0..8 {
        roll.next();
    }
    let mut places: Vec<u32> = (0..256).collect();
    for at in (1..places.len()).rev() {
        let other = roll.next() as usize % (at + 1);
        places.swap(at, other);
    }
    let cells: Vec<String> = places[..60]
        .iter()
        .map(|at| format!("{{\"at\":{at},\"is\":{}}}", 1 + roll.next() % 4095))
        .collect();
    let secret = (roll.next() % 4000, roll.next() % 4000);
    (format!("[{}]", cells.join(",")), secret)
}

fn walked(host: &Host, cells: &str, (turning, swing): (u32, u32)) -> Json {
    structured(
        Some(host),
        "grit_slope",
        &format!(
            "{{\"kind\":\"smooth\",\"wide\":16,\"deep\":16,\"depth\":8,\"carry\":false,\"most\":256,\"cells\":{cells},\"turning\":{turning},\"swing\":{swing}}}"
        ),
    )
}

#[test]
fn what_slope_answers_settle_takes_back_and_lands_on_exactly() {
    let Some(host) = device("what_slope_answers_settle_takes_back_and_lands_on_exactly") else {
        return;
    };
    let (cells, secret) = scattered(0);
    let made = walked(&host, &cells, secret);
    let ended = made.get("ended").and_then(Json::list).unwrap();
    let negative = ended
        .iter()
        .filter(|one| one.get("is").and_then(Json::int).unwrap() < 0)
        .count();
    eprintln!(
        "{} cells answered, {negative} of them below zero",
        ended.len()
    );
    assert!(negative > 0, "the round trip has to carry a negative cell");
    let want = written(made.get("ended").unwrap());
    let held = structured(
        Some(&host),
        "grit_settle",
        &format!(
            "{{\"kind\":\"smooth\",\"wide\":16,\"deep\":16,\"depth\":8,\"cells\":{cells},\"want\":{want},\"turning\":{},\"swing\":{},\"rounds\":1,\"ceiling\":8192}}",
            secret.0, secret.1
        ),
    );
    assert_eq!(
        held.get("apart").and_then(Json::int),
        Some(0),
        "at the parameters that made it, the grid slope answered is the grid settle lands on: {}",
        written(&held)
    );
}

#[test]
fn a_step_of_one_tries_the_neighbours_before_it_stops() {
    let Some(host) = device("a_step_of_one_tries_the_neighbours_before_it_stops") else {
        return;
    };
    let (cells, secret) = scattered(1);
    let want = written(walked(&host, &cells, secret).get("ended").unwrap());
    let held = structured(
        Some(&host),
        "grit_settle",
        &format!(
            "{{\"kind\":\"smooth\",\"wide\":16,\"deep\":16,\"depth\":8,\"cells\":{cells},\"want\":{want},\"turning\":2000,\"swing\":2000,\"step\":256,\"rounds\":64,\"ceiling\":8192}}"
        ),
    );
    eprintln!(
        "secret {secret:?}, found ({:?}, {:?}) apart {:?} after {:?} rounds, {:?} of them beside its best",
        held.counted("turning"),
        held.counted("swing"),
        held.get("apart"),
        held.counted("rounds"),
        held.counted("probed")
    );
    assert!(
        held.counted("probed").unwrap() > 0,
        "the walk ended beside the answer only by trying a neighbour: {}",
        written(&held)
    );
    assert_eq!(held.get("apart").and_then(Json::int), Some(0));
    assert_eq!(
        (
            held.counted("turning").unwrap(),
            held.counted("swing").unwrap()
        ),
        secret
    );
}

#[test]
fn a_machine_that_passes_its_input_through_is_marked_on_whole_words() {
    let held = structured(
        None,
        "grit_machine",
        "{\"state\":0,\"inputs\":1,\"outputs\":1,\"gates\":1,\"traces\":[{\"feed\":[0,1],\"want\":[0,1]}],\"chip\":[{\"kind\":10,\"left\":0,\"right\":0}]}",
    );
    assert_eq!(
        held.get("solved"),
        Some(&Json::Bool(true)),
        "{}",
        written(&held)
    );
    assert_eq!(
        held.get("wrong").and_then(Json::list).map(<[Json]>::len),
        Some(0)
    );
}

#[test]
fn settle_takes_the_widest_cells_without_wrapping() {
    let Some(host) = device("settle_takes_the_widest_cells_without_wrapping") else {
        return;
    };
    let made = structured(
        Some(&host),
        "grit_slope",
        "{\"wide\":2,\"deep\":2,\"depth\":1,\"kind\":\"sum\",\"fill\":2147483647,\"carry\":false,\"most\":4}",
    );
    let ended: Vec<i64> = (0..4_u32)
        .map(|at| {
            made.get("ended")
                .and_then(Json::list)
                .unwrap()
                .iter()
                .find(|one| one.counted("at") == Some(at))
                .map_or(2_147_483_647, |one| {
                    one.get("is").and_then(Json::int).unwrap()
                })
        })
        .collect();
    let wanted: i64 = ended.iter().map(|one| (one + 2_147_483_648).abs()).sum();
    let want: Vec<String> = (0..4)
        .map(|at| format!("{{\"at\":{at},\"is\":-2147483648}}"))
        .collect();
    let held = structured(
        Some(&host),
        "grit_settle",
        &format!(
            "{{\"wide\":2,\"deep\":2,\"depth\":1,\"rounds\":1,\"kind\":\"sum\",\"fill\":2147483647,\"want\":[{}]}}",
            want.join(",")
        ),
    );
    eprintln!("ended {ended:?}, apart {:?}", held.get("apart"));
    assert_eq!(
        held.get("apart").and_then(Json::int),
        Some(wanted),
        "the distance is counted on whole numbers: {}",
        written(&held)
    );
}

const SCATTER: &str = "[{\"at\":9,\"is\":300},{\"at\":10,\"is\":-200},{\"at\":18,\"is\":500},{\"at\":27,\"is\":100},{\"at\":36,\"is\":-400},{\"at\":45,\"is\":250}]";

#[test]
fn a_walk_pinned_at_the_floor_tries_its_neighbours_before_it_stops() {
    let Some(host) = device("a_walk_pinned_at_the_floor_tries_its_neighbours_before_it_stops")
    else {
        return;
    };
    let want = "[{\"at\":54,\"is\":500},{\"at\":0,\"is\":-400},{\"at\":45,\"is\":300},{\"at\":9,\"is\":250},{\"at\":46,\"is\":-200},{\"at\":63,\"is\":100},{\"at\":49,\"is\":-2},{\"at\":51,\"is\":-2},{\"at\":53,\"is\":-2},{\"at\":55,\"is\":-2}]";
    let held = structured(
        Some(&host),
        "grit_settle",
        &format!(
            "{{\"wide\":8,\"deep\":8,\"kind\":\"bound\",\"turning\":0,\"swing\":0,\"depth\":4,\"rounds\":40,\"step\":1,\"cells\":{SCATTER},\"want\":{want}}}"
        ),
    );
    eprintln!(
        "bound from (0,0): ({:?},{:?}) apart {:?}, probed {:?}, stalled {:?}",
        held.counted("turning"),
        held.counted("swing"),
        held.get("apart"),
        held.counted("probed"),
        held.get("stalled")
    );
    assert_eq!(
        held.get("apart").and_then(Json::int),
        Some(0),
        "{}",
        written(&held)
    );
    assert!(held.counted("probed").unwrap() > 0);
}

#[test]
fn a_walk_stalls_only_where_no_neighbour_is_nearer() {
    let Some(host) = device("a_walk_stalls_only_where_no_neighbour_is_nearer") else {
        return;
    };
    let common = |kind: &str| {
        format!("\"wide\":8,\"deep\":8,\"kind\":\"{kind}\",\"depth\":4,\"cells\":{SCATTER}")
    };
    let mut stops = 0;
    for kind in [
        "product", "sum", "turn", "bound", "smooth", "living", "twinned",
    ] {
        for (t, s) in [(12, 0), (12, 3), (20, 5), (3, 9)] {
            let made = structured(
                Some(&host),
                "grit_slope",
                &format!(
                    "{{{},\"turning\":{t},\"swing\":{s},\"carry\":false,\"most\":64}}",
                    common(kind)
                ),
            );
            let want = written(made.get("ended").unwrap());
            for (from_t, from_s) in [(0, 0), (t + 6, s + 6), (t, s + 1), (t + 1, s)] {
                let held = structured(
                    Some(&host),
                    "grit_settle",
                    &format!(
                        "{{{},\"turning\":{from_t},\"swing\":{from_s},\"rounds\":64,\"step\":1,\"want\":{want}}}",
                        common(kind)
                    ),
                );
                if held.get("stalled") != Some(&Json::Bool(true)) {
                    continue;
                }
                stops += 1;
                let apart = held.get("apart").and_then(Json::int).unwrap();
                let bt = i64::from(held.counted("turning").unwrap());
                let bs = i64::from(held.counted("swing").unwrap());
                for (dt, ds) in [
                    (1, 1),
                    (-1, -1),
                    (1, -1),
                    (-1, 1),
                    (0, 1),
                    (0, -1),
                    (1, 0),
                    (-1, 0),
                ] {
                    let (nt, ns) = (bt + dt, bs + ds);
                    if nt < 0 || ns < 0 {
                        continue;
                    }
                    let there = structured(
                        Some(&host),
                        "grit_settle",
                        &format!(
                            "{{{},\"turning\":{nt},\"swing\":{ns},\"rounds\":1,\"want\":{want}}}",
                            common(kind)
                        ),
                    );
                    let gap = there.get("apart").and_then(Json::int).unwrap();
                    assert!(
                        gap >= apart,
                        "{kind} target ({t},{s}) from ({from_t},{from_s}) stalled at ({bt},{bs}) apart {apart}, and ({nt},{ns}) is apart {gap}"
                    );
                }
            }
        }
    }
    eprintln!("{stops} walks stalled, each where no neighbour is nearer");
    assert!(stops > 0);
}

#[test]
fn what_slope_answers_settle_takes_back_whatever_the_fill() {
    let Some(host) = device("what_slope_answers_settle_takes_back_whatever_the_fill") else {
        return;
    };
    for kind in [
        "product", "sum", "turn", "bound", "smooth", "living", "twinned",
    ] {
        for fill in [0_i64, 7, -3] {
            let common = format!(
                "\"wide\":4,\"deep\":4,\"kind\":\"{kind}\",\"turning\":12,\"swing\":0,\"depth\":3,\"fill\":{fill},\"cells\":[{{\"at\":5,\"is\":40}},{{\"at\":10,\"is\":0}}]"
            );
            let made = structured(
                Some(&host),
                "grit_slope",
                &format!("{{{common},\"carry\":false,\"most\":4096}}"),
            );
            assert_eq!(made.get("fill").and_then(Json::int), Some(fill));
            let want = written(made.get("ended").unwrap());
            let held = structured(
                Some(&host),
                "grit_settle",
                &format!("{{{common},\"rounds\":1,\"want\":{want}}}"),
            );
            assert_eq!(
                held.get("apart").and_then(Json::int),
                Some(0),
                "{kind} fill {fill}: {}",
                written(&held)
            );
        }
    }
}

#[test]
fn a_walk_never_walks_a_point_twice() {
    let Some(host) = device("a_walk_never_walks_a_point_twice") else {
        return;
    };
    let mut runs = 0;
    let mut probes = 0;
    for kind in [
        "product", "sum", "turn", "bound", "smooth", "living", "twinned",
    ] {
        for (t, s) in [(12_u32, 0_u32), (12, 3), (3, 9)] {
            let made = structured(
                Some(&host),
                "grit_slope",
                &format!(
                    "{{\"wide\":8,\"deep\":8,\"kind\":\"{kind}\",\"turning\":{t},\"swing\":{s},\"depth\":4,\"cells\":{SCATTER},\"carry\":false,\"most\":64}}"
                ),
            );
            let want = written(made.get("ended").unwrap());
            for (from_t, from_s, step, ceiling) in [
                (0, 0, 1, 16384),
                (0, 0, 8, 16384),
                (t + 6, s + 6, 4, 16384),
                (1, 1, 8, 16384),
                (t, s, 8, t.max(s) + 1),
                (t + 1, s, 16, t.max(s) + 1),
                (0, 0, 4, 0),
            ] {
                if from_t > ceiling || from_s > ceiling {
                    continue;
                }
                runs += 1;
                let args = format!(
                    "{{\"wide\":8,\"deep\":8,\"kind\":\"{kind}\",\"turning\":{from_t},\"swing\":{from_s},\"depth\":4,\"rounds\":60,\"step\":{step},\"ceiling\":{ceiling},\"fill\":0,\"cells\":{SCATTER},\"want\":{want}}}"
                );
                let held = structured(Some(&host), "grit_settle", &args);
                let trail = held.get("trail").and_then(Json::list).unwrap();
                let mut seen = std::collections::BTreeSet::new();
                for (at, one) in trail.iter().enumerate() {
                    let point = (
                        one.counted("turning").unwrap(),
                        one.counted("swing").unwrap(),
                    );
                    assert!(
                        seen.insert(point),
                        "{args}: round {at} walks {point:?} again"
                    );
                }
                let probed = held.counted("probed").unwrap() as usize;
                assert!(
                    probed < trail.len(),
                    "{args}: {probed} probes in {} rounds",
                    trail.len()
                );
                probes += probed;
            }
        }
    }
    eprintln!("{runs} walks, {probes} probes, no point walked twice");
    assert!(probes > 0);
}

#[test]
fn probed_counts_the_neighbours_walked_and_no_more() {
    let Some(host) = device("probed_counts_the_neighbours_walked_and_no_more") else {
        return;
    };
    let start = "[{\"at\":528,\"is\":600},{\"at\":529,\"is\":500}]";
    for rounds in 1..=8_u32 {
        let held = structured(
            Some(&host),
            "grit_settle",
            &format!(
                "{{\"wide\":32,\"deep\":32,\"depth\":6,\"turning\":20,\"rounds\":{rounds},\"step\":4,\"fill\":0,\"cells\":{start},\"want\":[{{\"at\":528,\"is\":1}}]}}"
            ),
        );
        let walked = held.get("trail").and_then(Json::list).unwrap().len() as u32;
        assert_eq!(walked, rounds.min(6), "{}", written(&held));
        assert_eq!(
            held.counted("probed"),
            Some(walked - 1),
            "after the start every round walked a neighbour, and {walked} rounds were walked: {}",
            written(&held)
        );
        assert_eq!(
            held.get("stalled"),
            Some(&Json::Bool(rounds >= 6)),
            "{}",
            written(&held)
        );
    }
}
