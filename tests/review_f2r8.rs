#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

#[cfg(feature = "faults")]
mod common;

#[cfg(feature = "faults")]
mod served {
    use std::time::{Duration, Instant};

    use grit::serve3::Server;
    use grit::wire::{Json, written};

    use super::common::device;

    fn call(server: &mut Server<'_>, tool: &str, arguments: &str) -> (Json, Duration) {
        let line = format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{{\"name\":\"{tool}\",\"arguments\":{arguments}}}}}"
        );
        let began = Instant::now();
        let got = server.heard(&line).unwrap();
        (got.get("result").cloned().unwrap_or(got), began.elapsed())
    }

    fn fine(result: &Json) -> bool {
        result.get("isError") == Some(&Json::Bool(false))
    }

    fn world_of(result: &Json) -> i64 {
        result
            .get("structuredContent")
            .and_then(|held| held.get("world"))
            .and_then(Json::int)
            .unwrap_or_else(|| panic!("no world in {}", written(result)))
    }

    const SIDE: u32 = 512;

    fn create(server: &mut Server<'_>, slots: Option<u32>) -> (i64, Duration) {
        let slots = slots.map_or(String::new(), |held| format!(",\"slots\":{held}"));
        let (got, took) = call(
            server,
            "grit_world_create",
            &format!(
                "{{\"wide\":{SIDE},\"deep\":{SIDE},\"tall\":{SIDE},\"rule\":\"sand\",\"sparse\":true{slots},\"budget_ms\":120000}}"
            ),
        );
        assert!(fine(&got), "setup: {}", written(&got));
        (world_of(&got), took)
    }

    fn snapped_at_frame_one(server: &mut Server<'_>, world: i64) {
        grit::serve3::tools::faults::stall_batch(0, Duration::from_secs(3));
        let (got, _) = call(
            server,
            "grit_world_step",
            &format!("{{\"world\":{world},\"frames\":1,\"budget_ms\":300000}}"),
        );
        assert!(fine(&got), "setup: {}", written(&got));
        grit::serve3::tools::faults::disarm();
    }

    fn read(server: &mut Server<'_>, world: i64, budget: Duration) -> (Json, Duration) {
        call(
            server,
            "grit_world_read",
            &format!(
                "{{\"world\":{world},\"from\":[0,0,0],\"to\":[0,0,0],\"budget_ms\":{}}}",
                budget.as_millis()
            ),
        )
    }

    #[test]
    fn r8a_a_small_sparse_world_is_refused_by_the_rebuild_of_a_larger_world_whose_snapshot_has_the_same_cells()
     {
        let Some(host) = device("r8a") else {
            return;
        };
        let mut server = Server::new(Some(&host));
        let (probe, _) = call(
            &mut server,
            "grit_world_create",
            &format!(
                "{{\"wide\":{SIDE},\"deep\":{SIDE},\"tall\":{SIDE},\"rule\":\"sand\",\"sparse\":true,\"slots\":{},\"budget_ms\":120000}}",
                u32::MAX
            ),
        );
        let code = probe
            .get("structuredContent")
            .and_then(|held| held.get("code"))
            .and_then(Json::text);
        if matches!(code, Some("too_big" | "too_large")) {
            eprintln!(
                "r8a: this device cannot hold a {SIDE}^3 sparse world: {}",
                written(&probe)
            );
            return;
        }
        drop(server);
        let mut server = Server::new(Some(&host));
        let (small, small_built) = create(&mut server, Some(1));
        let (large, large_built) = create(&mut server, Some(u32::MAX));
        eprintln!("created small {small} in {small_built:?}, large {large} in {large_built:?}");
        snapped_at_frame_one(&mut server, small);
        snapped_at_frame_one(&mut server, large);
        server.faults().lock().unwrap().wedge_before_call = true;
        let _ = call(&mut server, "grit_describe", "{}");
        assert!(server.lost(), "setup: the device was not lost");
        let book = server.into_book();
        drop(host);
        let host = device("r8a again").unwrap();
        let (mut server, _) = Server::resumed(Some(&host), book);
        let (rebuilt, large_took) = read(&mut server, large, Duration::from_secs(120));
        assert!(fine(&rebuilt), "setup: {}", written(&rebuilt));
        eprintln!("large rebuilt and read in {large_took:?}");
        let given = large_took / 3;
        let mut refused = 0;
        let mut ran = None;
        let mut first = None;
        for _ in 0..12 {
            let (got, took) = read(&mut server, small, given);
            if fine(&got) {
                ran = Some(took);
                break;
            }
            refused += 1;
            first.get_or_insert_with(|| written(&got));
        }
        eprintln!(
            "small: refused {refused} times with {given:?}, then ran in {ran:?}; first: {first:?}"
        );
        assert_eq!(
            refused,
            0,
            "world {small} (1 slot) and world {large} (the most slots) were both snapshotted in memory at frame 1 with the same cells, so their journals share one key; the larger world's rebuild finished in {large_took:?}, and the small world's rebuild given {given:?} was refused {refused} times by that sample (it then ran in {ran:?}): {}",
            first.unwrap_or_default()
        );
    }
}
