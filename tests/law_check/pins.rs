use grit::serve3::Server;
use grit::wire::{Json, written};

use super::pinned::{LONG, life};
use super::served::{
    Rng, bytes_hash, cells_text, heat_field, heat_law, int_at, learned, ok, quoted, set_of,
};

pub const NEW_TOOLS: [&str; 1] = ["grit_law_check"];

pub const NEW_CODES: [&str; 2] = ["check_start", "check_target"];

fn joined(answers: &[Json]) -> String {
    let shown: Vec<String> = answers.iter().map(written).collect();
    bytes_hash(shown.join("\n").as_bytes())
}

pub fn life_session(server: &mut Server<'_>) -> String {
    let law = learned(server, &life(), false);
    let mut answers = Vec::new();
    let made = ok(
        server,
        "grit_world_create",
        &format!(
            "{{\"wide\":64,\"deep\":64,\"tall\":2,\"law\":{},\"params\":[],\"alternate\":[0,1],\"budget_ms\":{LONG}}}",
            quoted(&law)
        ),
    );
    let world = int_at(&made, "world");
    answers.push(made);
    let mut rng = Rng(5);
    let words: Vec<u32> = (0..4096)
        .map(|_| u32::from(rng.below(1000) < 300))
        .collect();
    let cells = set_of([64, 64, 1], &words);
    answers.push(ok(
        server,
        "grit_world_write",
        &format!(
            "{{\"world\":{world},\"current\":true,\"cells\":{},\"budget_ms\":{LONG}}}",
            cells_text(&cells)
        ),
    ));
    answers.push(ok(
        server,
        "grit_world_step",
        &format!("{{\"world\":{world},\"frames\":8,\"budget_ms\":{LONG}}}"),
    ));
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
    joined(&answers)
}

pub fn heat_session(server: &mut Server<'_>) -> String {
    let law = heat_law(server);
    let mut answers = Vec::new();
    let made = ok(
        server,
        "grit_world_create",
        &format!(
            "{{\"wide\":32,\"deep\":32,\"tall\":2,\"law\":{},\"params\":[],\"alternate\":[0,1],\"budget_ms\":{LONG}}}",
            quoted(&law)
        ),
    );
    let world = int_at(&made, "world");
    answers.push(made);
    let mut rng = Rng(9);
    let words = heat_field(&mut rng, [32, 32, 1], super::served::FRAC);
    let cells = set_of([32, 32, 1], &words);
    answers.push(ok(
        server,
        "grit_world_write",
        &format!(
            "{{\"world\":{world},\"current\":true,\"cells\":{},\"budget_ms\":{LONG}}}",
            cells_text(&cells)
        ),
    ));
    answers.push(ok(
        server,
        "grit_world_step",
        &format!("{{\"world\":{world},\"frames\":5,\"budget_ms\":{LONG}}}"),
    ));
    answers.push(ok(
        server,
        "grit_world_read",
        &format!(
            "{{\"world\":{world},\"current\":true,\"from\":[0,0,0],\"to\":[31,31,0],\"budget_ms\":{LONG}}}"
        ),
    ));
    joined(&answers)
}

fn named(one: &Json, key: &str) -> Option<String> {
    one.get(key).and_then(Json::text).map(str::to_owned)
}

pub fn listing(server: &mut Server<'_>) -> String {
    let got = server
        .heard("{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}")
        .unwrap();
    let tools = got
        .get("result")
        .and_then(|one| one.get("tools"))
        .and_then(Json::list)
        .unwrap();
    let kept: Vec<Json> = tools
        .iter()
        .filter(|one| named(one, "name").is_none_or(|name| !NEW_TOOLS.contains(&name.as_str())))
        .cloned()
        .collect();
    bytes_hash(written(&Json::List(kept)).as_bytes())
}

pub fn described(server: &mut Server<'_>) -> String {
    let got = ok(server, "grit_describe", "{}");
    let tools: Vec<Json> = got
        .get("tools")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .filter(|one| one.text().is_none_or(|name| !NEW_TOOLS.contains(&name)))
        .cloned()
        .collect();
    let codes: Vec<Json> = got
        .get("codes")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .filter(|one| named(one, "code").is_none_or(|code| !NEW_CODES.contains(&code.as_str())))
        .cloned()
        .collect();
    let kept = Json::Map(vec![
        ("tools".to_owned(), Json::List(tools)),
        ("codes".to_owned(), Json::List(codes)),
        ("limits".to_owned(), got.get("limits").cloned().unwrap()),
        ("server".to_owned(), got.get("server").cloned().unwrap()),
    ]);
    bytes_hash(written(&kept).as_bytes())
}

pub fn c1_case(server: &mut Server<'_>) -> String {
    let law = learned(server, &life(), false);
    let mut rng = Rng(11);
    let size = [32, 32, 2];
    let starts: Vec<Vec<u32>> = (0..5)
        .map(|_| crate::soup(&mut rng, size, 350, false))
        .collect();
    let made = crate::case("life 2d wrapped", &law, size, 6);
    let checked = super::long::compared(server, &made, &starts, 100);
    bytes_hash(written(&checked.answer).as_bytes())
}

pub fn cells_targets(server: &mut Server<'_>) -> String {
    let law = learned(server, &life(), false);
    let mut rng = Rng(12);
    let size = [32, 32, 2];
    let given: Vec<String> = (0..3)
        .map(|at| {
            let start = crate::soup(&mut rng, size, 350, false);
            let mut picked: Vec<String> = set_of(size, &start)
                .iter()
                .step_by(3 + at)
                .map(|(spot, word)| format!("[{},{},{},{word}]", spot[0], spot[1], spot[2]))
                .collect();
            picked.push("[31,31,1,5]".to_owned());
            picked.push("[3,4,0,1]".to_owned());
            picked.push("[3,4,0,0]".to_owned());
            picked.push("[0,0,0,9]".to_owned());
            picked.push("[0,0,0,1]".to_owned());
            format!(
                "{{\"cells\":{},\"targets\":[{{\"frame\":4,\"cells\":[{}]}},{{\"cells\":[]}},{{\"frame\":0,\"cells\":{}}},{{\"frame\":2,\"dense\":{}}}]}}",
                cells_text(&set_of(size, &start)),
                picked.join(","),
                cells_text(&set_of(size, &start)),
                super::served::listed(&start)
            )
        })
        .collect();
    let got = ok(
        server,
        "grit_law_check",
        &format!(
            "{{\"law\":{},\"alternate\":[0,1],\"size\":[32,32,2],\"frames\":6,\"starts\":[{}],\"return\":true,\"budget_ms\":{LONG}}}",
            quoted(&law),
            given.join(",")
        ),
    );
    bytes_hash(written(&got).as_bytes())
}
