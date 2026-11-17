#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

#[cfg(feature = "faults")]
mod pipeline {
    use super::common::device;

    use ash::vk;
    use grit::resident::faults::{self, Step};
    use grit::serve3::{Options, Server};
    use grit::wire::{Json, written};

    const SIDE: u32 = 8;
    const DIVIDER: &str = "[\"w7v1\",[\"sub\",\"w7v0\",[\"sel\",[\"lt\",[\"div\",4294967295,\"w3v0\"],\"frame\"],1,0]]]";

    fn lifting_text() -> String {
        let mut all: Vec<String> = (0..8)
            .map(|k| format!("[\"w{k}v0\",[\"at\",\"cells\",{k}]]"))
            .collect();
        all.push(DIVIDER.to_owned());
        let out: Vec<String> = (0..8)
            .map(|k| if k == 7 { "\"w7v1\"".to_owned() } else { format!("\"w{k}v0\"") })
            .collect();
        format!(
            "{{\"law\":1,\"name\":\"divider\",\"params\":1,\"channels\":1,\"halo\":0,\"reversible\":true,\"tally\":{{\"counts\":1,\"weighted\":false}},\"consts\":{{}},\"defs\":{{}},\"step\":[\"let\",[{}],[\"vec\",[\"vec\",{}],[\"vec\",0]]]}}",
            all.join(","),
            out.join(",")
        )
    }

    fn call(server: &mut Server<'_>, name: &str, arguments: &str) -> (bool, Json) {
        let line = format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{{\"name\":\"{name}\",\"arguments\":{arguments}}}}}"
        );
        let got = server.heard(&line).unwrap();
        let held = got.get("result").unwrap();
        (
            held.get("isError") == Some(&Json::Bool(true)),
            held.get("structuredContent").cloned().unwrap(),
        )
    }

    #[test]
    fn a_refused_open_of_a_kept_law_world_changes_nothing() {
        let Some(host) = device("a_refused_open_of_a_kept_law_world_changes_nothing") else {
            return;
        };
        let dir = std::env::temp_dir().join(format!("grit-review-n1r2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let options = Options {
            dir: Some(dir.join("worlds")),
            memory: None,
        };
        let world = {
            let mut server = Server::with(Some(&host), &options).unwrap();
            let (error, made) = call(
                &mut server,
                "grit_world_create",
                &format!(
                    "{{\"wide\":{SIDE},\"deep\":{SIDE},\"tall\":{SIDE},\"law\":{},\"params\":[0]}}",
                    written(&Json::Text(lifting_text()))
                ),
            );
            assert!(!error, "{}", written(&made));
            let world = made.get("world").and_then(Json::int).unwrap();
            let (error, stepped) = call(
                &mut server,
                "grit_world_step",
                &format!("{{\"world\":{world},\"frames\":2}}"),
            );
            assert!(!error, "{}", written(&stepped));
            world
        };
        let mut server = Server::with(Some(&host), &options).unwrap();
        let open = format!("{{\"name\":\"world-{world}\"}}");
        faults::reset();
        faults::arm_with(Step::Pipeline, 0, vk::Result::ERROR_UNKNOWN);
        let (error, told) = call(&mut server, "grit_world_open", &open);
        faults::disarm();
        assert!(faults::fired() && error, "{}", written(&told));
        println!("refused: {}", written(&told));
        assert_eq!(told.get("code"), Some(&Json::Text("law_not_built".into())));
        let (error, again) = call(&mut server, "grit_world_open", &open);
        println!("again: {}", written(&again));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            !error,
            "a law_not_built refusal whose hint says nothing changed left the kept world unopenable: {}",
            written(&again)
        );
    }
}
