use grit::serve3::Server;
use grit::wire::{Json, written};

use super::data::{SIDE, SOURCE, example, request};
use super::pinned::{LONG, life};
use super::served::{
    FRAC, Rng, bytes_hash, heat_field, heat_law, int_at, learned, listed, ok, quoted, set_of,
};

pub const NEW_CODES: [&str; 3] = ["driven_frames", "driven_length", "driven_written"];

pub const RECORDED: [(&str, &str); 5] = [
    (
        "step-session",
        "49b7dccdfd6c30c16deaea487484c92c43a2ee265835d8eaca6e4099e819d4e2",
    ),
    (
        "check-session",
        "ab6fdb8eca2c50ea96b4aa18ac97c3114ec6c0f5ce25d415301e32054fa5476f",
    ),
    (
        "learn-session",
        "b3a95ae0fe883053387b242ad6d4ea9d42e84b075cb49413339cee019a65b07e",
    ),
    (
        "tools-list",
        "a81c43d7f0c519c2aad4da60ef7f87683d5284e25fc10ffc6921396621c12e5e",
    ),
    (
        "describe",
        "68686abafc0ca086f6e42c58dee2538ade50ecf94097f78f4092e5ea1571a3cf",
    ),
];

fn joined(answers: &[Json]) -> String {
    let shown: Vec<String> = answers.iter().map(written).collect();
    bytes_hash(shown.join("\n").as_bytes())
}

fn cells_of(cells: &[([u32; 3], u32)]) -> String {
    let shown: Vec<String> = cells
        .iter()
        .map(|(at, word)| format!("[{},{},{},{word}]", at[0], at[1], at[2]))
        .collect();
    format!("[{}]", shown.join(","))
}

pub fn step_session(server: &mut Server<'_>) -> String {
    let law = learned(server, &life(), false);
    let mut answers = Vec::new();
    for (tall, sparse, absorb) in [(2, false, ""), (64, true, ",\"absorb\":[2,2,0]")] {
        let made = ok(
            server,
            "grit_world_create",
            &format!(
                "{{\"wide\":64,\"deep\":64,\"tall\":{tall},\"law\":{},\"params\":[],\"alternate\":[0,1],\"sparse\":{sparse}{absorb},\"budget_ms\":{LONG}}}",
                quoted(&law)
            ),
        );
        let world = int_at(&made, "world");
        answers.push(made);
        let mut rng = Rng(21);
        let words: Vec<u32> = (0..4096)
            .map(|_| u32::from(rng.below(1000) < 300))
            .collect();
        answers.push(ok(
            server,
            "grit_world_write",
            &format!(
                "{{\"world\":{world},\"current\":true,\"cells\":{},\"budget_ms\":{LONG}}}",
                cells_of(&set_of([64, 64, 1], &words))
            ),
        ));
        for frames in [1, 7, 3] {
            answers.push(ok(
                server,
                "grit_world_step",
                &format!("{{\"world\":{world},\"frames\":{frames},\"budget_ms\":{LONG}}}"),
            ));
        }
        answers.push(ok(
            server,
            "grit_world_read",
            &format!(
                "{{\"world\":{world},\"current\":true,\"form\":\"set\",\"from\":[0,0,0],\"to\":[63,63,1],\"budget_ms\":{LONG}}}"
            ),
        ));
        answers.push(ok(
            server,
            "grit_world_close",
            &format!("{{\"world\":{world}}}"),
        ));
    }
    joined(&answers)
}

pub fn check_session(server: &mut Server<'_>) -> String {
    let heat = heat_law(server);
    let life_law = learned(server, &life(), false);
    let mut rng = Rng(33);
    let size = [16, 16, 2];
    let starts: Vec<String> = (0..3)
        .map(|_| {
            let start = heat_field(&mut rng, size, FRAC);
            let target = heat_field(&mut rng, size, FRAC);
            format!(
                "{{\"dense\":{},\"targets\":[{{\"frame\":2,\"dense\":{}}},{{\"dense\":{}}}]}}",
                listed(&start),
                listed(&target),
                listed(&target)
            )
        })
        .collect();
    let mut answers = vec![ok(
        server,
        "grit_law_check",
        &format!(
            "{{\"law\":{},\"alternate\":[0,1],\"size\":[16,16,2],\"border\":\"zero\",\"values\":\"real\",\"frac\":{FRAC},\"frames\":5,\"starts\":[{}],\"return\":true,\"budget_ms\":{LONG}}}",
            quoted(&heat),
            starts.join(",")
        ),
    )];
    let soups: Vec<String> = (0..4)
        .map(|_| {
            let soup: Vec<u32> = (0..2048)
                .map(|_| u32::from(rng.below(1000) < 350))
                .collect();
            format!(
                "{{\"dense\":{},\"targets\":[{{\"dense\":{}}}]}}",
                listed(&soup),
                listed(&soup)
            )
        })
        .collect();
    answers.push(ok(
        server,
        "grit_law_check",
        &format!(
            "{{\"law\":{},\"alternate\":[0,1],\"size\":[32,32,2],\"frames\":6,\"starts\":[{}],\"budget_ms\":{LONG}}}",
            quoted(&life_law),
            soups.join(",")
        ),
    ));
    joined(&answers)
}

pub fn learn_session(server: &mut Server<'_>) -> String {
    let mut rng = Rng(44);
    let size = [SIDE, SIDE, 1];
    let examples: Vec<String> = (0..2)
        .map(|at| {
            let one = example(&mut rng, size, at, SOURCE, 12, 2);
            let shots: Vec<String> = one
                .shots
                .iter()
                .map(|shot| format!("[{}]", listed(shot)))
                .collect();
            format!(
                "{{\"size\":[{SIDE},{SIDE},1],\"sequence\":[{}]}}",
                shots.join(",")
            )
        })
        .collect();
    let plain = ok(server, "grit_law_learn", &request(&examples, ""));
    let hidden = ok(
        server,
        "grit_law_learn",
        &request(&examples, ",\"hidden\":1,\"poly\":2,\"law_frac\":12"),
    );
    joined(&[plain, hidden])
}

fn without_driven(held: &Json) -> Json {
    match held {
        Json::Map(pairs) => Json::Map(
            pairs
                .iter()
                .filter(|(key, _)| key != "driven")
                .map(|(key, one)| (key.clone(), without_driven(one)))
                .collect(),
        ),
        Json::List(items) => Json::List(items.iter().map(without_driven).collect()),
        other => other.clone(),
    }
}

fn tools(server: &mut Server<'_>) -> Json {
    let got = server
        .heard("{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}")
        .unwrap();
    got.get("result")
        .and_then(|one| one.get("tools"))
        .cloned()
        .unwrap()
}

pub fn listing(server: &mut Server<'_>) -> String {
    bytes_hash(written(&without_driven(&tools(server))).as_bytes())
}

fn codes(server: &mut Server<'_>) -> (Json, Vec<String>) {
    let got = ok(server, "grit_describe", "{}");
    let names = got
        .get("codes")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .filter_map(|one| one.get("code").and_then(Json::text).map(str::to_owned))
        .collect();
    (got, names)
}

pub fn described(server: &mut Server<'_>) -> String {
    let (got, _) = codes(server);
    let kept: Vec<Json> = got
        .get("codes")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .filter(|one| {
            one.get("code")
                .and_then(Json::text)
                .is_none_or(|code| !NEW_CODES.contains(&code))
        })
        .cloned()
        .collect();
    let shown = Json::Map(vec![
        ("tools".to_owned(), got.get("tools").cloned().unwrap()),
        ("codes".to_owned(), Json::List(kept)),
        ("limits".to_owned(), got.get("limits").cloned().unwrap()),
        ("server".to_owned(), got.get("server").cloned().unwrap()),
    ]);
    bytes_hash(written(&shown).as_bytes())
}

fn has_driven(held: &Json) -> bool {
    match held {
        Json::Map(pairs) => pairs
            .iter()
            .any(|(key, one)| key == "driven" || has_driven(one)),
        Json::List(items) => items.iter().any(has_driven),
        _ => false,
    }
}

pub fn new_forms_described(server: &mut Server<'_>) {
    let (_, names) = codes(server);
    for code in NEW_CODES {
        assert!(names.iter().any(|one| one == code), "{code} not described");
    }
    let listed_tools = tools(server);
    let mut driving: Vec<String> = listed_tools
        .list()
        .unwrap()
        .iter()
        .filter(|one| has_driven(one))
        .map(|one| one.get("name").and_then(Json::text).unwrap().to_owned())
        .collect();
    driving.sort();
    assert_eq!(
        driving,
        ["grit_law_check", "grit_law_learn", "grit_world_step"],
        "driven is described on exactly the three tools"
    );
}
