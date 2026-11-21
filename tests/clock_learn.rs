#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

#[cfg(feature = "faults")]
mod common;

#[cfg(feature = "faults")]
mod delayed;

#[cfg(feature = "faults")]
mod faked {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use grit::Host;
    use grit::clock::{Fake, Time};
    use grit::serve3::law_learn::faults::{disarm, rounds_each_submission};
    use grit::serve3::{Options, Server};
    use grit::wire::{Json, read, written};

    use super::common::device;
    use super::delayed::Waiting;

    const TASK: &str = include_str!("law_learn/x0b_task.json");

    const BUDGETS: [u64; 2] = [1, 600_000];

    const WAITED: Duration = Duration::from_millis(4);

    const FLOOR: Duration = Duration::from_millis(20);

    fn grids(held: &Json) -> String {
        let words: Vec<String> = held
            .list()
            .unwrap()
            .iter()
            .flat_map(|row| row.list().unwrap().iter().map(written))
            .collect();
        format!("[{}]", words.join(","))
    }

    fn arguments(budget: u64) -> String {
        let task = read(TASK).unwrap();
        let examples: Vec<String> = task
            .get("train")
            .and_then(Json::list)
            .unwrap()
            .iter()
            .take(8)
            .map(|pair| {
                format!(
                    "{{\"size\":[32,32,1],\"input\":[{}],\"output\":[{}]}}",
                    grids(pair.get("input").unwrap()),
                    grids(pair.get("output").unwrap())
                )
            })
            .collect();
        format!(
            "{{\"stencil\":\"3x3\",\"reads\":[{{\"plane\":0,\"bit\":0}}],\"writes\":[{{\"plane\":1,\"bit\":0}}],\"border\":\"zero\",\"rule\":\"minimal\",\"examples\":[{}],\"budget_ms\":{budget}}}",
            examples.join(",")
        )
    }

    fn learned(host: &Host, time: Arc<dyn Time>, budget: u64) -> (String, Duration) {
        let mut server = Server::timed(Some(host), &Options::default(), time).unwrap();
        let line = format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{{\"name\":\"grit_law_learn\",\"arguments\":{}}}}}",
            arguments(budget)
        );
        disarm();
        rounds_each_submission(1);
        let began = Instant::now();
        let got = server.heard(&line).unwrap();
        let took = began.elapsed();
        disarm();
        let told = got
            .get("result")
            .and_then(|result| result.get("structuredContent"))
            .cloned()
            .unwrap_or_else(|| panic!("{}", written(&got)));
        assert_eq!(
            got.get("result").and_then(|result| result.get("isError")),
            Some(&Json::Bool(false)),
            "{}",
            written(&got)
        );
        assert_eq!(
            told.get("stopped").and_then(Json::text),
            Some("ladder"),
            "budget {budget}: {}",
            written(&told)
        );
        (written(&told), took)
    }

    #[test]
    fn k2_real_time_spent_at_every_clock_read_moves_no_learned_answer() {
        let Some(host) = device("k2 learn waiting") else {
            return;
        };
        let mut plain = Vec::new();
        for budget in BUDGETS {
            let fake = Arc::new(Fake::new());
            let (answer, _) = learned(&host, Arc::clone(&fake) as Arc<dyn Time>, budget);
            assert_eq!(fake.elapsed(), Duration::ZERO);
            let waiting = Arc::new(Waiting::new(WAITED, WAITED));
            let (delayed, took) = learned(&host, Arc::clone(&waiting) as Arc<dyn Time>, budget);
            let (waits, waited) = waiting.waited();
            eprintln!(
                "k2 learn, budget {budget}: {waits} waits, {waited:?} in all, the call took {took:?}"
            );
            assert_eq!(waiting.fake().elapsed(), Duration::ZERO);
            assert_eq!(
                answer, delayed,
                "budget {budget}: real time spent at the clock reads changed the learned answer"
            );
            assert!(took >= FLOOR, "budget {budget}: the call took {took:?}");
            plain.push(answer);
        }
        assert_eq!(
            plain[0], plain[1],
            "a frozen clock learned differently at budgets of 1 ms and 600000 ms"
        );
    }
}
