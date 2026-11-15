#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

#[cfg(feature = "faults")]
mod common;

#[cfg(feature = "faults")]
mod served {
    use std::time::{Duration, Instant};

    use grit::serve3::journal::{Entry, Keep, Ledger, Made};
    use grit::serve3::{Options, Server};
    use grit::wire::{Json, written};

    use super::common::device;

    fn call(server: &mut Server<'_>, tool: &str, arguments: &str) -> Json {
        let line = format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{{\"name\":\"{tool}\",\"arguments\":{arguments}}}}}"
        );
        let got = server.heard(&line).unwrap();
        got.get("result").cloned().unwrap_or(got)
    }

    fn open(server: &mut Server<'_>, number: u32, budget: Duration) -> (Json, Duration) {
        let began = Instant::now();
        let got = call(
            server,
            "grit_world_open",
            &format!(
                "{{\"name\":\"world-{number}\",\"budget_ms\":{}}}",
                budget.as_millis()
            ),
        );
        (got, began.elapsed())
    }

    fn ratios(server: &mut Server<'_>) -> i64 {
        let told = call(server, "grit_describe", "{}");
        let text = written(&told);
        let at = text.find("replay_observations").unwrap();
        text[at..]
            .chars()
            .skip_while(|one| !one.is_ascii_digit())
            .take_while(char::is_ascii_digit)
            .collect::<String>()
            .parse()
            .unwrap()
    }

    fn kept(label: &str, numbers: &[u32], size: [u32; 3], frames: u32) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("grit-{label}-{}-{nanos}", std::process::id()));
        let keep = Keep::at(&root.join("worlds")).unwrap();
        for &number in numbers {
            let mut ledger = Ledger::created(Made {
                rule: "sand".to_owned(),
                params: Vec::new(),
                size,
                sparse: false,
                slots: None,
            });
            if number < 10 {
                ledger.push(Entry::Step(frames), Duration::from_secs(1));
            }
            keep.begin(number, &ledger).unwrap();
        }
        keep.set_next(numbers.iter().max().unwrap() + 1).unwrap();
        root
    }

    fn needs(result: &Json) -> Duration {
        let text = written(result);
        let at = text
            .find("needs at least ")
            .unwrap_or_else(|| panic!("setup: no least in {text}"));
        let millis: u64 = text[at + 15..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>()
            .parse()
            .unwrap();
        Duration::from_millis(millis)
    }

    fn fine(result: &Json) -> bool {
        result.get("isError") == Some(&Json::Bool(false))
    }

    fn too_big_here(result: &Json) -> bool {
        let code = result
            .get("structuredContent")
            .and_then(|told| told.get("code"))
            .and_then(Json::text);
        matches!(code, Some("too_big" | "too_large"))
    }

    const FRAMES: u32 = 40;

    const STALL: Duration = Duration::from_millis(4);

    #[test]
    fn r6a_a_step_cut_charges_the_build_to_the_steps_and_refuses_an_open_that_fits() {
        let Some(host) = device("r6a") else {
            return;
        };
        let root = kept(
            "r6a",
            &[1, 2, 3, 4, 5, 6, 7, 10, 11],
            [1024, 512, 512],
            FRAMES,
        );
        let options = Options {
            dir: Some(root.join("worlds")),
            memory: None,
        };
        let mut server = Server::with(Some(&host), &options).unwrap();
        grit::serve3::tools::faults::stall_every(STALL);
        let (warm, _) = open(&mut server, 1, Duration::from_secs(60));
        if too_big_here(&warm) {
            eprintln!("this device cannot hold the world: {}", written(&warm));
            grit::serve3::tools::faults::disarm();
            drop(server);
            let _ = std::fs::remove_dir_all(&root);
            return;
        }
        assert!(fine(&warm), "setup: {}", written(&warm));
        let (bare, build) = open(&mut server, 10, Duration::from_secs(60));
        assert!(fine(&bare), "setup: {}", written(&bare));
        let (bare, again) = open(&mut server, 11, Duration::from_secs(60));
        assert!(fine(&bare), "setup: {}", written(&bare));
        let build = build.min(again);
        let (measured, took) = open(&mut server, 2, Duration::from_secs(60));
        assert!(fine(&measured), "setup: {}", written(&measured));
        let steps = took.saturating_sub(build);
        let (probe, _) = open(&mut server, 3, Duration::from_millis(1));
        let least = needs(&probe);
        let cut = least + Duration::from_millis(10);
        let enough = cut + build * 3 / 4;
        eprintln!(
            "open of world-2: {took:?}; build about {build:?}, steps about {steps:?}; least before any cut {least:?}"
        );
        let (fits, fit_took) = open(&mut server, 3, enough);
        eprintln!("open of world-3 with {enough:?}: {fit_took:?}");
        assert!(
            fine(&fits),
            "setup: an open with {enough:?} did not fit: {}",
            written(&fits)
        );
        let mut worst = cut;
        for number in 4..=6 {
            let before = ratios(&mut server);
            for attempt in 0..4 {
                let (probe, _) = open(&mut server, 7, Duration::from_millis(1));
                let cut = needs(&probe) + Duration::from_millis(10);
                worst = worst.max(cut);
                {
                    let faults = server.faults();
                    let mut armed = faults.lock().unwrap();
                    armed.held_replays = 1;
                    armed.held_by = cut.saturating_sub(build / 2);
                }
                let (stopped, stop_took) = open(&mut server, number, cut);
                eprintln!(
                    "open of world-{number} with {cut:?} (attempt {attempt}): {stop_took:?}: {}",
                    written(&stopped).chars().take(200).collect::<String>()
                );
                assert!(
                    written(&stopped).contains("stopped on budget"),
                    "setup: not cut by budget: {}",
                    written(&stopped)
                );
                if ratios(&mut server) == before + 1 {
                    break;
                }
            }
            server.faults().lock().unwrap().held_replays = 0;
            assert_eq!(
                ratios(&mut server),
                before + 1,
                "setup: no cut landed in the step in four attempts"
            );
        }
        let enough = enough.max(worst + build * 3 / 4);
        let (later, later_took) = open(&mut server, 7, enough);
        grit::serve3::tools::faults::disarm();
        drop(server);
        let _ = std::fs::remove_dir_all(&root);
        eprintln!(
            "open of world-7 with {enough:?} after three cuts at {cut:?}: {later_took:?}: {}",
            written(&later)
        );
        assert!(
            fine(&later),
            "world-3 opened in {fit_took:?} with {enough:?}; three identical replays held up in their steps and cut by budget with {cut:?} each, after a build of about {build:?}, prove only that the work needs more than {cut:?}; an identical open with {enough:?}, more time than every failing attempt, was refused because each cut charged its build time to the steps: {}",
            written(&later)
        );
    }

    #[test]
    fn r6b_the_fastest_tested_device_floor_refuses_forever_a_dense_build_this_device_just_ran() {
        let Some(host) = device("r6b") else {
            return;
        };
        let root = kept("r6b", &[10, 11, 12, 13], [512, 512, 512], 1);
        let options = Options {
            dir: Some(root.join("worlds")),
            memory: None,
        };
        let mut server = Server::with(Some(&host), &options).unwrap();
        let mut fastest = Duration::MAX;
        for number in 10..=12 {
            let (opened, took) = open(&mut server, number, Duration::from_secs(60));
            if too_big_here(&opened) {
                eprintln!("this device cannot hold the world: {}", written(&opened));
                drop(server);
                let _ = std::fs::remove_dir_all(&root);
                return;
            }
            assert!(fine(&opened), "setup: {}", written(&opened));
            eprintln!("open of bare 512^3 world-{number}: {took:?}");
            fastest = fastest.min(took);
        }
        let given = fastest + Duration::from_millis(10);
        let mut refused = 0;
        let mut first = None;
        for _ in 0..10 {
            let (later, _) = open(&mut server, 13, given);
            if fine(&later) {
                break;
            }
            refused += 1;
            first.get_or_insert(later);
        }
        drop(server);
        let _ = std::fs::remove_dir_all(&root);
        if let Some(first) = first {
            panic!(
                "an identical open of a bare 512^3 world finished whole in {fastest:?}; an open given {given:?} was refused {refused} times in a row, saying: {}",
                written(&first)
            );
        }
    }
}
