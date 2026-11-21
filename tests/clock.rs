#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

#[cfg(feature = "faults")]
mod common;

#[cfg(feature = "faults")]
mod delayed;

#[cfg(feature = "faults")]
mod faked {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use std::sync::mpsc::Receiver;

    use grit::Host;
    use grit::clock::{Fake, Time, aside, point, serving};
    use grit::serve3::exec::Exec;
    use grit::serve3::journal::{Entry, Keep, Ledger, Made};
    use grit::serve3::tools::faults;
    use grit::serve3::{Options, Server};
    use grit::wire::{Json, written};

    use super::common::device;
    use super::delayed::Waiting;

    const SIDE: u32 = 32;

    const KEPT_FRAMES: u32 = 400;

    const WRITTEN: u32 = 96;

    const STEPPED: u32 = 2048;

    fn line(name: &str, arguments: &str) -> String {
        format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{{\"name\":\"{name}\",\"arguments\":{arguments}}}}}"
        )
    }

    fn call(server: &mut Server<'_>, name: &str, arguments: &str) -> Json {
        server.heard(&line(name, arguments)).unwrap()
    }

    fn told(got: &Json) -> Json {
        got.get("result")
            .and_then(|result| result.get("structuredContent"))
            .cloned()
            .unwrap_or_else(|| panic!("no structured answer in {}", written(got)))
    }

    fn failed(got: &Json) -> bool {
        got.get("result").and_then(|result| result.get("isError")) == Some(&Json::Bool(true))
    }

    fn int_at(held: &Json, key: &str) -> i64 {
        held.get(key)
            .and_then(Json::int)
            .unwrap_or_else(|| panic!("no {key} in {}", written(held)))
    }

    fn fine(got: &Json) -> Json {
        assert!(!failed(got), "{}", written(got));
        let held = told(got);
        assert!(
            held.get("stopped").is_none(),
            "the call stopped: {}",
            written(got)
        );
        held
    }

    fn refused_on_budget(got: &Json) -> String {
        assert!(failed(got), "{}", written(got));
        let held = told(got);
        assert_eq!(
            held.get("code").and_then(Json::text),
            Some("budget"),
            "{}",
            written(got)
        );
        assert_eq!(
            held.get("field").and_then(Json::text),
            Some("$.budget_ms"),
            "{}",
            written(got)
        );
        held.get("message").and_then(Json::text).unwrap().to_owned()
    }

    fn stopped_on_budget(got: &Json) -> Json {
        assert!(!failed(got), "{}", written(got));
        let held = told(got);
        assert_eq!(
            held.get("stopped").and_then(Json::text),
            Some("budget"),
            "{}",
            written(got)
        );
        held
    }

    fn root(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("grit-clock-{label}-{nanos}"))
    }

    fn kept(root: &Path) -> Options {
        let dir = root.join("worlds");
        let keep = Keep::at(&dir).unwrap();
        let mut ledger = Ledger::created(Made {
            rule: "sand".to_owned(),
            params: Vec::new(),
            size: [SIDE, SIDE, SIDE],
            sparse: false,
            slots: None,
        });
        let cells: Vec<([u32; 3], u32)> = (0..WRITTEN)
            .map(|at| ([at % SIDE, SIDE - 1, at / SIDE], grit::sand::SAND))
            .collect();
        ledger.push(Entry::Write(cells), Duration::from_millis(3));
        ledger.push(Entry::Step(KEPT_FRAMES), Duration::from_secs(1));
        keep.begin(1, &ledger).unwrap();
        keep.set_next(2).unwrap();
        Options {
            dir: Some(dir),
            memory: None,
        }
    }

    fn cube(budget: u64) -> String {
        format!(
            "{{\"wide\":{SIDE},\"deep\":{SIDE},\"tall\":{SIDE},\"rule\":\"sand\",\"budget_ms\":{budget}}}"
        )
    }

    fn cells(world: i64, budget: u64) -> String {
        let cells: Vec<String> = (0..WRITTEN)
            .map(|at| {
                format!(
                    "[{},{},{},{}]",
                    at % SIDE,
                    SIDE - 1,
                    at / SIDE,
                    grit::sand::SAND
                )
            })
            .collect();
        format!(
            "{{\"world\":{world},\"cells\":[{}],\"budget_ms\":{budget}}}",
            cells.join(",")
        )
    }

    fn frames(world: i64, frames: u32, budget: u64) -> String {
        format!("{{\"world\":{world},\"frames\":{frames},\"budget_ms\":{budget}}}")
    }

    fn one(world: i64, budget: u64) -> String {
        format!("{{\"world\":{world},\"budget_ms\":{budget}}}")
    }

    fn kept_world(budget: u64) -> String {
        format!("{{\"name\":\"world-1\",\"budget_ms\":{budget}}}")
    }

    fn timed<'host>(host: &'host Host, options: &Options, fake: &Arc<Fake>) -> Server<'host> {
        let time: Arc<dyn Time> = Arc::clone(fake) as Arc<dyn Time>;
        Server::timed(Some(host), options, time).unwrap()
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Path5 {
        Build,
        Write,
        Step,
        Fork,
        Replay,
    }

    const PATHS: [Path5; 5] = [
        Path5::Build,
        Path5::Write,
        Path5::Step,
        Path5::Fork,
        Path5::Replay,
    ];

    fn asked(path: Path5, world: i64, budget: u64) -> (&'static str, String) {
        match path {
            Path5::Build => ("grit_world_create", cube(budget)),
            Path5::Write => ("grit_world_write", cells(world, budget)),
            Path5::Step => ("grit_world_step", frames(world, STEPPED, budget)),
            Path5::Fork => ("grit_world_fork", one(world, budget)),
            Path5::Replay => ("grit_world_open", kept_world(budget)),
        }
    }

    fn made(server: &mut Server<'_>) -> i64 {
        int_at(
            &fine(&call(server, "grit_world_create", &cube(30_000))),
            "world",
        )
    }

    #[test]
    fn k1_a_frozen_clock_never_stops_or_refuses_any_path_on_budget() {
        let Some(host) = device("k1 frozen") else {
            return;
        };
        for budget in [1_u64, 2, 600_000] {
            let root = root("k1a");
            let options = kept(&root);
            let fake = Arc::new(Fake::new());
            let mut server = timed(&host, &options, &fake);
            let built = fine(&call(&mut server, "grit_world_create", &cube(budget)));
            let world = int_at(&built, "world");
            assert_eq!(int_at(&built, "frame"), 0);

            let wrote = fine(&call(
                &mut server,
                "grit_world_write",
                &cells(world, budget),
            ));
            assert_eq!(
                int_at(&wrote, "written"),
                i64::from(WRITTEN),
                "{}",
                written(&wrote)
            );

            let stepped = fine(&call(
                &mut server,
                "grit_world_step",
                &frames(world, STEPPED, budget),
            ));
            assert_eq!(
                int_at(&stepped, "frame"),
                i64::from(STEPPED),
                "{}",
                written(&stepped)
            );

            let forked = fine(&call(&mut server, "grit_world_fork", &one(world, budget)));
            assert_eq!(
                int_at(&forked, "frame"),
                i64::from(STEPPED),
                "{}",
                written(&forked)
            );

            let opened = fine(&call(&mut server, "grit_world_open", &kept_world(budget)));
            assert_eq!(
                int_at(&opened, "frame"),
                i64::from(KEPT_FRAMES),
                "{}",
                written(&opened)
            );
            assert_eq!(int_at(&opened, "replayed"), 2, "{}", written(&opened));

            assert_eq!(server.open(), 3);
            assert_eq!(fake.elapsed(), Duration::ZERO);
            assert!(fake.reads() > 0, "the server never read its clock");
            drop(server);
            let _ = std::fs::remove_dir_all(&root);
        }
    }

    fn masked(message: &str) -> String {
        let mut out = String::new();
        let mut digits = false;
        for one in message.chars() {
            if one.is_ascii_digit() {
                if !digits {
                    out.push('#');
                }
                digits = true;
            } else {
                digits = false;
                out.push(one);
            }
        }
        out
    }

    struct Ready<'host> {
        server: Server<'host>,
        world: i64,
        root: PathBuf,
    }

    fn ready<'host>(host: &'host Host, label: &str, fake: &Arc<Fake>) -> Ready<'host> {
        let root = root(label);
        let options = kept(&root);
        let mut server = timed(host, &options, fake);
        let world = made(&mut server);
        Ready {
            server,
            world,
            root,
        }
    }

    fn ready_real<'host>(host: &'host Host, label: &str) -> Ready<'host> {
        let root = root(label);
        let options = kept(&root);
        let mut server = Server::with(Some(host), &options).unwrap();
        let world = made(&mut server);
        Ready {
            server,
            world,
            root,
        }
    }

    fn listed(server: &mut Server<'_>) -> String {
        written(&told(&call(server, "grit_world_list", "{}")))
    }

    #[test]
    fn k1_b_a_clock_past_the_deadline_at_the_first_read_refuses_every_path_as_the_real_clock_does()
    {
        let Some(host) = device("k1 first read") else {
            return;
        };
        let budget = 20_u64;
        let jump = Duration::from_millis(budget + 1);
        for path in PATHS {
            let fake = Arc::new(Fake::new());
            let mut faked = ready(&host, "k1b", &fake);
            let before = listed(&mut faked.server);
            let (name, arguments) = asked(path, faked.world, budget);
            faked.server.arriving(Some(fake.now()), None);
            fake.script([jump]);
            let got = call(&mut faked.server, name, &arguments);
            assert_eq!(fake.scripted(), 0, "{path:?}: the jump was never read");
            let message = refused_on_budget(&got);
            assert!(
                message.contains(&format!("past its budget of {budget} ms"))
                    && message.contains("nothing ran"),
                "{path:?}: {message}"
            );
            assert_eq!(faked.server.open(), 1, "{path:?}");
            assert_eq!(listed(&mut faked.server), before, "{path:?}");

            let mut real = ready_real(&host, "k1b-real");
            let (name, arguments) = asked(path, real.world, budget);
            real.server
                .arriving(Some(Instant::now().checked_sub(jump).unwrap()), None);
            let truth = call(&mut real.server, name, &arguments);
            let real_message = refused_on_budget(&truth);
            assert_eq!(
                masked(&written(&truth)),
                masked(&written(&got)),
                "{path:?}: the fake clock and the real clock answered differently"
            );
            assert_eq!(masked(&real_message), masked(&message), "{path:?}");
            assert_eq!(real.server.open(), 1, "{path:?}");
            drop(faked.server);
            drop(real.server);
            let _ = std::fs::remove_dir_all(&faked.root);
            let _ = std::fs::remove_dir_all(&real.root);
        }
    }

    #[test]
    fn k1_c_a_clock_past_the_deadline_at_the_first_read_inside_a_path_stops_it_on_budget() {
        let Some(host) = device("k1 inside") else {
            return;
        };
        let budget = 20_u64;
        let jump = Duration::from_millis(budget + 1);
        for path in PATHS {
            let fake = Arc::new(Fake::new());
            let mut held = ready(&host, "k1c", &fake);
            let before = listed(&mut held.server);
            let (name, arguments) = asked(path, held.world, budget);
            held.server.arriving(Some(fake.now()), None);
            fake.at("call", [jump]);
            let got = call(&mut held.server, name, &arguments);
            assert_eq!(
                fake.waiting("call"),
                0,
                "{path:?}: the call point was never passed"
            );
            eprintln!("k1c {path:?}: {}", written(&told(&got)));
            match path {
                Path5::Build | Path5::Fork | Path5::Replay => {
                    let message = refused_on_budget(&got);
                    assert!(
                        message.contains("0 ms left") && message.contains("nothing ran"),
                        "{path:?}: {message}"
                    );
                    assert_eq!(held.server.open(), 1, "{path:?}");
                    assert_eq!(listed(&mut held.server), before, "{path:?}");
                }
                Path5::Write => {
                    let answer = stopped_on_budget(&got);
                    assert_eq!(int_at(&answer, "written"), 1, "{}", written(&got));
                    assert_eq!(int_at(&answer, "asked"), i64::from(WRITTEN));
                    assert_eq!(held.server.open(), 1);
                }
                Path5::Step => {
                    let answer = stopped_on_budget(&got);
                    assert_eq!(int_at(&answer, "frame"), 0, "{}", written(&got));
                    assert_eq!(held.server.open(), 1);
                }
            }
            drop(held.server);
            let _ = std::fs::remove_dir_all(&held.root);
        }
    }

    #[test]
    fn k1_d_a_clock_past_the_deadline_inside_the_replay_cuts_it_and_opens_nothing() {
        let Some(host) = device("k1 replay cut") else {
            return;
        };
        let budget = 20_u64;
        let jump = Duration::from_millis(budget + 1);
        let fake = Arc::new(Fake::new());
        let mut held = ready(&host, "k1d", &fake);
        let before = listed(&mut held.server);
        held.server.arriving(Some(fake.now()), None);
        fake.at("entry", [jump]);
        let got = call(&mut held.server, "grit_world_open", &kept_world(budget));
        assert_eq!(
            fake.waiting("entry"),
            0,
            "the first entry was never reached"
        );
        let message = refused_on_budget(&got);
        assert!(
            message.contains("before the call stopped on budget; nothing was opened")
                && message.contains("through 0 of its 2 entries"),
            "{message}"
        );
        assert_eq!(held.server.open(), 1);
        assert_eq!(listed(&mut held.server), before);
        drop(held.server);
        let _ = std::fs::remove_dir_all(&held.root);
    }

    fn halted_frame(message: &str) -> u32 {
        let after = message.split("to frame ").nth(1).unwrap_or_default();
        after
            .split(' ')
            .next()
            .and_then(|one| one.parse().ok())
            .unwrap_or_else(|| panic!("no frame in {message}"))
    }

    #[test]
    fn k1_e_a_clock_past_the_deadline_inside_a_paced_replay_stops_between_batches() {
        let Some(host) = device("k1 paced") else {
            return;
        };
        let budget = 20_u64;
        let probe = Arc::new(Fake::new());
        let mut counted = ready(&host, "k1e-probe", &probe);
        counted.server.arriving(Some(probe.now()), None);
        let from = probe.reads();
        fine(&call(
            &mut counted.server,
            "grit_world_open",
            &kept_world(budget),
        ));
        let reads = usize::try_from(probe.reads() - from).unwrap();
        drop(counted.server);
        let _ = std::fs::remove_dir_all(&counted.root);
        assert!(reads > 16, "the replay read its clock only {reads} times");

        let fake = Arc::new(Fake::new());
        let mut held = ready(&host, "k1e", &fake);
        let before = listed(&mut held.server);
        held.server.arriving(Some(fake.now()), None);
        let mut script = vec![Duration::ZERO; reads / 2];
        script.push(Duration::from_millis(budget + 1));
        fake.script(script);
        let got = call(&mut held.server, "grit_world_open", &kept_world(budget));
        assert_eq!(fake.scripted(), 0, "the jump was never read");
        let message = refused_on_budget(&got);
        assert!(
            message.contains("through 1 of its 2 entries")
                && message.contains("before the call stopped on budget; nothing was opened"),
            "{message}"
        );
        eprintln!("k1e jump after {} of {reads} reads: {message}", reads / 2);
        let frame = halted_frame(&message);
        assert!(
            frame > 0 && frame < KEPT_FRAMES && (frame - 1).is_multiple_of(64),
            "the paced replay was not cut between its batches: {message}"
        );
        assert_eq!(held.server.open(), 1);
        assert_eq!(listed(&mut held.server), before);
        drop(held.server);
        let _ = std::fs::remove_dir_all(&held.root);
    }

    fn script() -> Vec<Duration> {
        let mut state = 0x2545_f491_4f6c_dd1d_u64;
        (0..200_000)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                Duration::from_micros(200 + state % 4_800)
            })
            .collect()
    }

    fn session(host: &Host) -> (Vec<String>, Duration, u64) {
        let fake = Arc::new(Fake::new());
        fake.script(script());
        let mut server = timed(host, &Options::default(), &fake);
        let mut answers = Vec::new();
        let mut said = |server: &mut Server<'_>, name: &str, arguments: &str| {
            let got = call(server, name, arguments);
            answers.push(written(&got));
            got
        };
        let world = int_at(
            &told(&said(&mut server, "grit_world_create", &cube(30_000))),
            "world",
        );
        said(&mut server, "grit_world_write", &cells(world, 30_000));
        said(&mut server, "grit_world_write", &cells(world, 3));
        said(&mut server, "grit_world_step", &frames(world, 3000, 30_000));
        said(&mut server, "grit_world_step", &frames(world, 65_536, 60));
        let fork = int_at(
            &told(&said(&mut server, "grit_world_fork", &one(world, 30_000))),
            "world",
        );
        said(&mut server, "grit_world_step", &frames(fork, 20_000, 45));
        said(&mut server, "grit_world_create", &cube(1));
        said(&mut server, "grit_world_fork", &one(fork, 6));
        said(&mut server, "grit_world_create", &cube(6));
        said(&mut server, "grit_describe", "{}");
        drop(server);
        (answers, fake.elapsed(), fake.reads())
    }

    #[test]
    fn k2_the_same_script_gives_the_same_answers_byte_for_byte() {
        let Some(host) = device("k2") else {
            return;
        };
        let (first, first_spent, first_reads) = session(&host);
        let (second, second_spent, second_reads) = session(&host);
        for one in &first {
            eprintln!("k2: {one}");
        }
        eprintln!("k2: {first_spent:?} over {first_reads} reads");
        assert_eq!(first.len(), second.len());
        for (at, (one, other)) in first.iter().zip(&second).enumerate() {
            assert_eq!(
                one, other,
                "answer {at} differs between two runs of one script"
            );
        }
        assert_eq!(first_spent, second_spent);
        assert_eq!(first_reads, second_reads);
        assert!(
            first
                .iter()
                .any(|one| one.contains("\"stopped\":\"budget\"")),
            "no call of the session stopped on budget, so it does not exercise pacing: {first:?}"
        );
        assert!(
            first.iter().any(|one| one.contains("longest_ms")),
            "the session reported no time: {first:?}"
        );
    }

    const BUDGETS: [u64; 2] = [1, 600_000];

    const WAITED: Duration = Duration::from_millis(4);

    const FLOOR: Duration = Duration::from_millis(20);

    fn paths(host: &Host, time: Arc<dyn Time>, budget: u64) -> (Vec<String>, Vec<Duration>) {
        let root = root("k2wait");
        let options = kept(&root);
        let mut server = Server::timed(Some(host), &options, time).unwrap();
        let mut answers = Vec::new();
        let mut took = Vec::new();
        let mut asked = |server: &mut Server<'_>, name: &str, arguments: &str| {
            let began = Instant::now();
            let got = call(server, name, arguments);
            took.push(began.elapsed());
            answers.push(written(&fine(&got)));
            got
        };
        let world = int_at(
            &told(&asked(&mut server, "grit_world_create", &cube(budget))),
            "world",
        );
        asked(&mut server, "grit_world_write", &cells(world, budget));
        asked(
            &mut server,
            "grit_world_step",
            &frames(world, STEPPED, budget),
        );
        asked(&mut server, "grit_world_fork", &one(world, budget));
        asked(&mut server, "grit_world_open", &kept_world(budget));
        asked(&mut server, "grit_describe", "{}");
        drop(server);
        let _ = std::fs::remove_dir_all(&root);
        (answers, took)
    }

    #[test]
    fn k2_real_time_spent_at_every_clock_read_moves_no_answer_on_any_path() {
        let Some(host) = device("k2 waiting") else {
            return;
        };
        let named = ["build", "write", "step", "fork", "replay", "describe"];
        let mut plain = Vec::new();
        for budget in BUDGETS {
            let fake = Arc::new(Fake::new());
            let (answers, _) = paths(&host, Arc::clone(&fake) as Arc<dyn Time>, budget);
            assert_eq!(fake.elapsed(), Duration::ZERO);
            let waiting = Arc::new(Waiting::new(WAITED, WAITED));
            let (delayed, took) = paths(&host, Arc::clone(&waiting) as Arc<dyn Time>, budget);
            let (waits, waited) = waiting.waited();
            assert_eq!(waiting.fake().elapsed(), Duration::ZERO);
            eprintln!(
                "k2 waiting, budget {budget}: {waits} waits, {waited:?} in all, calls took {took:?}"
            );
            for (at, name) in named.iter().enumerate() {
                assert_eq!(
                    answers[at], delayed[at],
                    "{name} at budget {budget}: real time spent at the clock reads changed the answer"
                );
                assert!(
                    *name == "describe" || took[at] >= FLOOR,
                    "{name} at budget {budget} took {:?}, under {FLOOR:?}",
                    took[at]
                );
            }
            plain.push(answers);
        }
        for (at, name) in named.iter().enumerate() {
            assert_eq!(
                plain[0][at], plain[1][at],
                "{name}: a frozen clock answered differently at budgets of 1 ms and 600000 ms"
            );
        }
    }

    #[test]
    fn a_script_steps_only_on_a_thread_serving_a_call_and_every_thread_sees_its_time() {
        let fake = Fake::new();
        fake.script([Duration::from_millis(3), Duration::from_millis(5)]);
        let base = fake.now();
        assert_eq!(fake.steps(), 0);
        assert_eq!(fake.elapsed(), Duration::ZERO);
        std::thread::scope(|scope| {
            let read = scope.spawn(|| {
                let _serving_elsewhere = serving();
                fake.now()
            });
            assert_eq!(read.join().unwrap(), base + Duration::from_millis(3));
        });
        assert_eq!(fake.steps(), 1);
        let held = serving();
        assert_eq!(fake.now(), base + Duration::from_millis(8));
        std::thread::scope(|scope| {
            let aside = scope.spawn(|| (fake.now(), fake.now()));
            let (one, two) = aside.join().unwrap();
            assert_eq!((one, two), (base + Duration::from_millis(8), one));
        });
        drop(held);
        fake.script([Duration::from_millis(7)]);
        assert_eq!(fake.now(), base + Duration::from_millis(8));
        assert_eq!(fake.scripted(), 1);
        assert_eq!(fake.steps(), 2);
        assert_eq!(fake.reads(), 6);
        std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    let _writer = aside();
                    fake.sleep(Duration::from_millis(50));
                    fake.now()
                })
                .join()
                .unwrap();
        });
        assert_eq!(fake.slept(), Duration::from_millis(50));
        assert_eq!(fake.elapsed(), Duration::from_millis(8));
        fake.sleep(Duration::from_millis(2));
        assert_eq!(fake.elapsed(), Duration::from_millis(10));
        assert_eq!(fake.scripted(), 1);
    }

    #[test]
    fn a_script_steps_at_a_named_point_only_while_serving_and_only_on_its_own_clock() {
        let fake = Arc::new(Fake::new());
        let other = Arc::new(Fake::new());
        fake.at("entry", [Duration::ZERO, Duration::from_millis(9)]);
        other.at("entry", [Duration::from_millis(1)]);
        let clock: Arc<dyn Time> = Arc::clone(&fake) as Arc<dyn Time>;
        point(clock.as_ref(), "entry");
        assert_eq!(
            fake.waiting("entry"),
            2,
            "a pass while not serving took a step"
        );
        let held = serving();
        point(clock.as_ref(), "call");
        point(clock.as_ref(), "entry");
        assert_eq!(fake.elapsed(), Duration::ZERO);
        point(clock.as_ref(), "entry");
        assert_eq!(fake.elapsed(), Duration::from_millis(9));
        point(clock.as_ref(), "entry");
        assert_eq!(fake.elapsed(), Duration::from_millis(9));
        assert_eq!(fake.steps(), 2);
        assert_eq!(fake.reads(), 0, "a point reads no clock");
        assert_eq!(fake.passed(), ["call", "entry", "entry", "entry"]);
        assert_eq!(other.elapsed(), Duration::ZERO);
        assert_eq!(other.waiting("entry"), 1);
        fake.at("entry", [Duration::from_millis(4)]);
        std::thread::scope(|scope| {
            scope
                .spawn(|| point(clock.as_ref(), "entry"))
                .join()
                .unwrap();
        });
        assert_eq!(
            fake.waiting("entry"),
            1,
            "a pass on a thread not serving took a step"
        );
        point(clock.as_ref(), "entry");
        assert_eq!(fake.elapsed(), Duration::from_millis(13));
        drop(held);
        drop(clock);
        drop(fake);
        let held = serving();
        let fresh: Arc<dyn Time> = Arc::new(Fake::new());
        point(fresh.as_ref(), "entry");
        drop(held);
        assert_eq!(fresh.now(), fresh.now());
    }

    struct Ran {
        answers: Vec<String>,
        elapsed: Duration,
        steps: u64,
        writes: Vec<(Duration, Duration)>,
    }

    fn kept_session(host: &Host) -> Ran {
        let root = root("k3kept");
        let dir = root.join("worlds");
        let options = Options {
            dir: Some(dir),
            memory: None,
        };
        let fake = Arc::new(Fake::new());
        let steps: Vec<Duration> = script().into_iter().take(64).collect();
        fake.at("call", steps);
        let zero = fake.now();
        let mut server = timed(host, &options, &fake);
        let mut answers = Vec::new();
        let mut said = |server: &mut Server<'_>, name: &str, arguments: &str| {
            let got = call(server, name, arguments);
            answers.push(written(&got));
            got
        };
        let world = int_at(
            &told(&said(&mut server, "grit_world_create", &cube(30_000))),
            "world",
        );
        said(&mut server, "grit_world_write", &cells(world, 30_000));
        said(&mut server, "grit_world_step", &frames(world, 40, 30_000));
        faults::stall_every(Duration::from_secs(3));
        said(&mut server, "grit_world_step", &frames(world, 2, 600_000));
        faults::disarm();
        let writing = server.writing();
        let mut book = server.into_book();
        book.drain();
        let mut server = Server::resumed(Some(host), book).0;
        said(&mut server, "grit_world_step", &frames(world, 5, 30_000));
        said(&mut server, "grit_world_write", &cells(world, 30_000));
        let fork = int_at(
            &told(&said(&mut server, "grit_world_fork", &one(world, 30_000))),
            "world",
        );
        said(&mut server, "grit_world_step", &frames(fork, 6, 30_000));
        said(&mut server, "grit_describe", "{}");
        let writes: Vec<(Duration, Duration)> = server
            .faults()
            .lock()
            .unwrap()
            .writes
            .iter()
            .map(|(began, finished)| {
                (
                    began.saturating_duration_since(zero),
                    finished.saturating_duration_since(zero),
                )
            })
            .collect();
        drop(server);
        let _ = std::fs::remove_dir_all(&root);
        let named = root.file_name().unwrap().to_string_lossy().into_owned();
        let answers = answers
            .into_iter()
            .map(|one| one.replace(&named, "grit-clock-k3kept"))
            .collect();
        assert!(writing, "the stalled step started no snapshot writer");
        assert!(!writes.is_empty(), "no snapshot landed");
        Ran {
            answers,
            elapsed: fake.elapsed(),
            steps: fake.steps(),
            writes,
        }
    }

    fn same(first: &Ran, second: &Ran, label: &str) {
        for one in &first.answers {
            eprintln!("{label}: {one}");
        }
        eprintln!(
            "{label}: {:?} over {} steps, writes {:?}",
            first.elapsed, first.steps, first.writes
        );
        assert_eq!(first.answers.len(), second.answers.len());
        for (at, (one, other)) in first.answers.iter().zip(&second.answers).enumerate() {
            assert_eq!(one, other, "{label}: answer {at} differs between two runs");
        }
        assert_eq!(first.elapsed, second.elapsed, "{label}: elapsed fake time");
        assert_eq!(first.steps, second.steps, "{label}: steps taken");
        assert_eq!(first.writes, second.writes, "{label}: snapshot writes");
    }

    #[test]
    fn k3_a_scripted_session_with_a_snapshot_writer_gives_the_same_bytes_twice() {
        let Some(host) = device("k3 kept") else {
            return;
        };
        let first = kept_session(&host);
        let second = kept_session(&host);
        assert!(first.steps > 0);
        same(&first, &second, "k3 kept");
    }

    fn exec_asked(
        exec: &Exec,
        answers: &Receiver<Json>,
        id: i64,
        name: &str,
        arguments: &str,
    ) -> Json {
        exec.line(&line(name, arguments).replacen("\"id\":7", &format!("\"id\":{id}"), 1));
        let got = answers.recv_timeout(Duration::from_secs(300)).unwrap();
        assert_eq!(
            got.get("id").and_then(Json::int),
            Some(id),
            "{}",
            written(&got)
        );
        got
    }

    fn exec_session() -> Ran {
        let fake = Arc::new(Fake::new());
        fake.script(script());
        let pattern = std::env::var("GRIT_DEVICE")
            .ok()
            .filter(|one| !one.is_empty());
        let (exec, outbox) = Exec::spawn_timed(
            Box::new(move || Host::matching(pattern.as_deref())),
            &Options::default(),
            Arc::clone(&fake) as Arc<dyn Time>,
        )
        .unwrap();
        let mut answers = Vec::new();
        let mut id = 0;
        let mut said = |name: &str, arguments: &str| {
            id += 1;
            let got = exec_asked(&exec, &outbox, id, name, arguments);
            answers.push(written(&got));
            got
        };
        let world = int_at(&told(&said("grit_world_create", &cube(30_000))), "world");
        said("grit_world_write", &cells(world, 30_000));
        said("grit_world_write", &cells(world, 3));
        said("grit_world_step", &frames(world, 3000, 30_000));
        said("grit_world_step", &frames(world, 65_536, 60));
        let fork = int_at(
            &told(&said("grit_world_fork", &one(world, 30_000))),
            "world",
        );
        said("grit_world_step", &frames(fork, 20_000, 45));
        said("grit_world_create", &cube(1));
        said("grit_describe", "{}");
        exec.finish();
        Ran {
            answers,
            elapsed: fake.elapsed(),
            steps: fake.steps(),
            writes: Vec::new(),
        }
    }

    #[test]
    fn k3_a_scripted_session_through_exec_gives_the_same_bytes_twice_beside_its_watcher() {
        let Some(host) = device("k3 exec") else {
            return;
        };
        drop(host);
        let first = exec_session();
        let second = exec_session();
        assert!(
            first
                .answers
                .iter()
                .any(|one| one.contains("\"stopped\":\"budget\"")),
            "no call stopped on budget: {:?}",
            first.answers
        );
        same(&first, &second, "k3 exec");
    }
}

fn sources(dir: &std::path::Path, found: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources(&path, found);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            found.push(path);
        }
    }
}

#[derive(Debug, Clone)]
struct Token {
    text: String,
    line: usize,
}

const fn ident_start(one: char) -> bool {
    one.is_ascii_alphabetic() || one == '_'
}

const fn ident_part(one: char) -> bool {
    one.is_ascii_alphanumeric() || one == '_'
}

fn quoted(chars: &[char], mut at: usize, line: &mut usize) -> usize {
    let mut hashes = 0;
    let raw = chars.get(at) == Some(&'r');
    if raw {
        at += 1;
        while chars.get(at) == Some(&'#') {
            hashes += 1;
            at += 1;
        }
    }
    at += 1;
    while let Some(&one) = chars.get(at) {
        if one == '\n' {
            *line += 1;
        }
        if !raw && one == '\\' {
            if chars.get(at + 1) == Some(&'\n') {
                *line += 1;
            }
            at += 2;
            continue;
        }
        if one == '"' && (0..hashes).all(|more| chars.get(at + 1 + more) == Some(&'#')) {
            return at + 1 + hashes;
        }
        at += 1;
    }
    at
}

fn token(text: &str, line: usize) -> Token {
    Token {
        text: text.to_owned(),
        line,
    }
}

fn lexed(source: &str) -> Vec<Token> {
    let chars: Vec<char> = source.chars().collect();
    let mut found = Vec::new();
    let mut line = 1;
    let mut at = 0;
    while let Some(&one) = chars.get(at) {
        let next = chars.get(at + 1).copied();
        if one == '\n' {
            line += 1;
            at += 1;
        } else if one.is_whitespace() {
            at += 1;
        } else if one == '/' && next == Some('/') {
            while chars.get(at).is_some_and(|held| *held != '\n') {
                at += 1;
            }
        } else if one == '/' && next == Some('*') {
            let mut depth = 0_usize;
            while let Some(&held) = chars.get(at) {
                let after = chars.get(at + 1).copied();
                if held == '/' && after == Some('*') {
                    depth += 1;
                    at += 2;
                } else if held == '*' && after == Some('/') {
                    depth -= 1;
                    at += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    if held == '\n' {
                        line += 1;
                    }
                    at += 1;
                }
            }
        } else if one == '"' {
            let from = line;
            at = quoted(&chars, at, &mut line);
            found.push(token("\"\"", from));
        } else if one == '\'' {
            let close = if next == Some('\\') {
                (at + 2..chars.len()).find(|end| chars.get(*end) == Some(&'\''))
            } else if chars.get(at + 2) == Some(&'\'') {
                Some(at + 2)
            } else {
                None
            };
            if let Some(end) = close {
                at = end + 1;
                found.push(token("''", line));
            } else {
                found.push(token("'", line));
                at += 1;
            }
        } else if ident_start(one) {
            let begin = at;
            while chars.get(at).copied().is_some_and(ident_part) {
                at += 1;
            }
            let word: String = chars.get(begin..at).unwrap().iter().collect();
            let after = chars.get(at).copied();
            let prefix = matches!(word.as_str(), "r" | "b" | "br" | "c" | "cr");
            let raw = word.ends_with('r');
            let raw_string = raw
                && after == Some('#')
                && (at..chars.len())
                    .find(|end| chars.get(*end) != Some(&'#'))
                    .is_some_and(|end| chars.get(end) == Some(&'"'));
            if prefix && (after == Some('"') || raw_string) {
                let from = line;
                at = quoted(&chars, if raw { at - 1 } else { at }, &mut line);
                found.push(token("\"\"", from));
            } else if word == "b" && after == Some('\'') {
                let end = (at + 2..chars.len())
                    .find(|end| chars.get(*end) == Some(&'\'') && chars.get(end - 1) != Some(&'\\'))
                    .unwrap_or(chars.len());
                at = end + 1;
                found.push(token("''", line));
            } else if word == "r" && after == Some('#') {
                at += 1;
                let begin = at;
                while chars.get(at).copied().is_some_and(ident_part) {
                    at += 1;
                }
                let name: String = chars.get(begin..at).unwrap().iter().collect();
                found.push(token(&name, line));
            } else {
                found.push(Token { text: word, line });
            }
        } else if one.is_ascii_digit() {
            while chars.get(at).copied().is_some_and(|held| {
                ident_part(held)
                    || (held == '.' && chars.get(at + 1).is_some_and(char::is_ascii_digit))
            }) {
                at += 1;
            }
            found.push(token("0", line));
        } else {
            let pair: String = [one, next.unwrap_or(' ')].iter().collect();
            let joined = matches!(pair.as_str(), "::" | "->" | "=>");
            if joined {
                found.push(token(&pair, line));
                at += 2;
            } else {
                found.push(token(&one.to_string(), line));
                at += 1;
            }
        }
    }
    found
}

#[derive(Debug, Clone, Default)]
struct Place {
    owner: String,
    function: String,
    importing: bool,
    opens: bool,
}

impl Place {
    fn named(&self) -> String {
        match (self.owner.is_empty(), self.function.is_empty()) {
            (_, true) => String::new(),
            (true, false) => self.function.clone(),
            (false, false) => format!("{}::{}", self.owner, self.function),
        }
    }
}

fn word_at(tokens: &[Token], at: usize) -> &str {
    tokens.get(at).map_or("", |one| one.text.as_str())
}

fn identifier(word: &str) -> bool {
    word.chars().next().is_some_and(ident_start)
}

fn implemented(tokens: &[Token], from: usize) -> String {
    let mut at = from + 1;
    if word_at(tokens, at) == "<" {
        at = closing(tokens, at) + 1;
    }
    let mut name = String::new();
    let mut depth = 0_i32;
    while at < tokens.len() {
        match word_at(tokens, at) {
            "{" | "where" | ";" if depth == 0 => break,
            "for" if depth == 0 => name.clear(),
            "<" => depth += 1,
            ">" => depth -= 1,
            word if depth == 0 && identifier(word) && !matches!(word, "dyn" | "unsafe") => {
                word.clone_into(&mut name);
            }
            _ => {}
        }
        at += 1;
    }
    name
}

fn placed(tokens: &[Token]) -> Vec<Place> {
    let mut places = Vec::with_capacity(tokens.len());
    let mut frames: Vec<(bool, String, usize)> = Vec::new();
    let mut pending: Option<(bool, String)> = None;
    let mut depth = 0_usize;
    let mut importing = false;
    let mut nest = 0_usize;
    for (at, token) in tokens.iter().enumerate() {
        let mut opens = false;
        match token.text.as_str() {
            "impl"
                if matches!(
                    at.checked_sub(1).map_or("", |one| word_at(tokens, one)),
                    "" | "}" | ";" | "{" | "]" | "unsafe"
                ) =>
            {
                pending = Some((true, implemented(tokens, at)));
            }
            "fn" => pending = Some((false, word_at(tokens, at + 1).to_owned())),
            "use" => importing = true,
            "(" | "[" => nest += 1,
            ")" | "]" => nest = nest.saturating_sub(1),
            ";" if nest == 0 => {
                importing = false;
                if pending.as_ref().is_some_and(|(owner, _)| !owner) {
                    pending = None;
                }
            }
            "{" => {
                if let Some((owner, name)) = pending.take() {
                    frames.push((owner, name, depth));
                    opens = true;
                }
                depth += 1;
            }
            "}" => {
                depth = depth.saturating_sub(1);
                while frames.last().is_some_and(|(_, _, open)| *open >= depth) {
                    frames.pop();
                }
            }
            _ => {}
        }
        let innermost = |owner: bool| {
            frames
                .iter()
                .rev()
                .find(|frame| frame.0 == owner)
                .map(|frame| frame.1.clone())
                .unwrap_or_default()
        };
        places.push(Place {
            owner: innermost(true),
            function: innermost(false),
            importing,
            opens,
        });
    }
    places
}

const WAITS: [&str; 10] = [
    "sleep",
    "sleep_ms",
    "sleep_until",
    "recv_timeout",
    "recv_deadline",
    "wait_timeout",
    "wait_timeout_ms",
    "wait_timeout_while",
    "park_timeout",
    "park_timeout_ms",
];

const SOURCES: [&str; 16] = [
    "SystemTime",
    "UNIX_EPOCH",
    "cmd_write_timestamp",
    "get_query_pool_results",
    "get_calibrated_timestamps",
    "clock_gettime",
    "CLOCK_THREAD_CPUTIME_ID",
    "CLOCK_PROCESS_CPUTIME_ID",
    "GetThreadTimes",
    "GetProcessTimes",
    "QueryThreadCycleTime",
    "QueryProcessCycleTime",
    "QueryPerformanceCounter",
    "getrusage",
    "_rdtsc",
    "__rdtsc",
];

const MADE: [(&str, &str); 6] = [
    ("Guard", "default"),
    ("Book", "new"),
    ("Book", "empty"),
    ("Server", "new"),
    ("Server", "with"),
    ("Exec", "spawn"),
];

fn led(tokens: &[Token], at: usize) -> &str {
    let mut from = at;
    while from >= 2 && word_at(tokens, from - 1) == "::" && identifier(word_at(tokens, from - 2)) {
        from -= 2;
    }
    if from >= 1 && word_at(tokens, from - 1) == "::" {
        from -= 1;
    }
    from.checked_sub(1).map_or("", |one| word_at(tokens, one))
}

type Defaulted = (usize, String, Vec<String>);

fn defaulted(tokens: &[Token]) -> Vec<Defaulted> {
    let mut found = Vec::new();
    for (at, one) in tokens.iter().enumerate() {
        if one.text != "derive" || word_at(tokens, at + 1) != "(" {
            continue;
        }
        let shut = closing(tokens, at + 1);
        let derives = (at + 2..shut).any(|walk| word_at(tokens, walk) == "Default");
        let Some(kind) = (shut..tokens.len())
            .find(|walk| matches!(word_at(tokens, *walk), "struct" | "enum" | "union"))
        else {
            continue;
        };
        if !derives {
            continue;
        }
        let open = opened_after(tokens, kind);
        let end = match word_at(tokens, open) {
            "{" | "(" => closing(tokens, open),
            _ => open,
        };
        let words = (open..end)
            .map(|walk| word_at(tokens, walk).to_owned())
            .filter(|word| identifier(word))
            .collect();
        found.push((at, word_at(tokens, kind + 1).to_owned(), words));
    }
    found
}

fn real_by_default(files: &[(String, String)]) -> std::collections::BTreeSet<String> {
    let mut real: std::collections::BTreeSet<String> = ["Guard".to_owned()].into();
    let all: Vec<Defaulted> = files
        .iter()
        .flat_map(|(_, text)| defaulted(&lexed(text)))
        .collect();
    loop {
        let before = real.len();
        for (_, name, words) in &all {
            if words.iter().any(|word| real.contains(word)) {
                real.insert(name.clone());
            }
        }
        if real.len() == before {
            return real;
        }
    }
}

fn time_imports(tokens: &[Token], places: &[Place]) -> Vec<usize> {
    let mut found = Vec::new();
    let mut at = 0;
    while at < tokens.len() {
        let inside = places.get(at).is_some_and(|place| place.importing);
        if !(inside && word_at(tokens, at) == "time" && word_at(tokens, at + 1) == "::") {
            at += 1;
            continue;
        }
        let mut walk = at + 2;
        let mut depth = 0_i32;
        let mut alias = false;
        while walk < tokens.len() {
            let word = word_at(tokens, walk);
            match word {
                "{" => depth += 1,
                "}" if depth == 0 => break,
                "}" => {
                    depth -= 1;
                    if depth == 0 {
                        walk += 1;
                        break;
                    }
                }
                "," | ";" if depth == 0 => break,
                "as" => alias = true,
                "*" => found.push(walk),
                _ if identifier(word) => {
                    if !alias && word != "Duration" && word != "self" {
                        found.push(walk);
                    }
                    alias = false;
                }
                _ => {}
            }
            walk += 1;
        }
        at = walk;
    }
    found
}

fn kinds_at(
    tokens: &[Token],
    places: &[Place],
    at: usize,
    carriers: &[String],
    server: bool,
) -> Vec<String> {
    let place = places.get(at).cloned().unwrap_or_default();
    let raw = word_at(tokens, at);
    let word = if raw == "Self" && !place.owner.is_empty() {
        place.owner.as_str()
    } else {
        raw
    };
    let before = at.checked_sub(1).map_or("", |one| word_at(tokens, one));
    let lead = led(tokens, at);
    let after = word_at(tokens, at + 1);
    let then = word_at(tokens, at + 2);
    let defined = before == "fn";
    let mut found = Vec::new();
    if word == "Real" {
        found.push("Real".to_owned());
    }
    if word == "real" && before != "." && !defined {
        found.push("real".to_owned());
    }
    for (kind, made) in MADE {
        if word == kind && after == "::" && then == made {
            found.push(format!("{kind}::{made}"));
        }
    }
    if matches!(word, "Guard" | "Book" | "Server" | "Exec") {
        let defining = matches!(before, "struct" | "impl" | "for" | "enum" | "union");
        let body = places.get(at + 1).is_some_and(|one| one.opens);
        if word == "Guard" && after == "{" && !defining && !body {
            found.push("Guard {".to_owned());
        }
        if lead == "<" && (after == "as" || (after == ">" && then == "::")) {
            found.push(format!("<{word}>"));
        }
        if (place.importing && after == "as") || (lead == "=" && after == ";") {
            found.push(format!("{word} renamed"));
        }
    }
    if word == "Instant"
        && !place.importing
        && (after == "::"
            || after == "as"
            || after == ";"
            || lead == "="
            || (after == ">" && then == "::"))
    {
        found.push("Instant".to_owned());
    }
    if matches!(word, "elapsed" | "duration_since")
        && !defined
        && (matches!(before, "." | "::") || after == "(")
    {
        found.push(word.to_owned());
    }
    if WAITS.contains(&word) && !defined {
        found.push(word.to_owned());
    }
    if SOURCES.contains(&word) {
        found.push(word.to_owned());
    }
    if server
        && carriers.iter().any(|one| one == word)
        && !defined
        && (matches!(before, "." | "::") || after == "(")
    {
        found.push(format!("carrier {word}"));
    }
    found
}

fn carrier_file(relative: &str) -> bool {
    matches!(
        relative,
        "src/resident.rs" | "src/host.rs" | "src/live.rs" | "src/calibrate.rs"
    ) || relative.starts_with("src/calibrate/")
}

fn server_file(relative: &str) -> bool {
    relative == "src/serve.rs" || relative.starts_with("src/serve3/")
}

fn scanned() -> Vec<(String, String)> {
    let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut found = Vec::new();
    sources(&crate_root.join("src"), &mut found);
    let mut held: Vec<(String, String)> = found
        .into_iter()
        .map(|path| {
            let relative = path
                .strip_prefix(crate_root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            (relative, std::fs::read_to_string(&path).unwrap())
        })
        .filter(|(relative, _)| !relative.starts_with("src/bin/") && relative != "src/clock.rs")
        .collect();
    held.sort();
    held
}

fn holds_time(kind: &[Token], timed: &std::collections::BTreeSet<String>) -> bool {
    kind.iter()
        .any(|one| matches!(one.text.as_str(), "Duration" | "Instant") || timed.contains(&one.text))
}

fn visible(tokens: &[Token], at: usize) -> bool {
    let mut back = at;
    while back > 0 {
        back -= 1;
        match word_at(tokens, back) {
            "pub" => return true,
            "const" | "unsafe" | "async" | "extern" | "\"\"" => {}
            ")" => {
                while back > 0 && word_at(tokens, back) != "(" {
                    back -= 1;
                }
                return back > 0 && word_at(tokens, back - 1) == "pub";
            }
            _ => return false,
        }
    }
    false
}

fn closing(tokens: &[Token], open: usize) -> usize {
    let (left, right) = match word_at(tokens, open) {
        "(" => ("(", ")"),
        "[" => ("[", "]"),
        "<" => ("<", ">"),
        _ => ("{", "}"),
    };
    let mut depth = 0_i32;
    for (at, token) in tokens.iter().enumerate().skip(open) {
        if token.text == left {
            depth += 1;
        } else if token.text == right {
            depth -= 1;
            if depth == 0 {
                return at;
            }
        }
    }
    tokens.len()
}

type Field = (String, bool, Vec<Token>);

fn fields(tokens: &[Token], open: usize, public: bool) -> Vec<Field> {
    let end = closing(tokens, open);
    let mut found = Vec::new();
    let mut at = open + 1;
    while at < end {
        if word_at(tokens, at) == "#" {
            at = closing(tokens, at + 1) + 1;
            continue;
        }
        let mut shown = public;
        if word_at(tokens, at) == "pub" {
            shown = true;
            at += 1;
            if word_at(tokens, at) == "(" {
                at = closing(tokens, at) + 1;
            }
        }
        let name = word_at(tokens, at).to_owned();
        if word_at(tokens, at + 1) != ":" {
            at += 1;
            continue;
        }
        let mut walk = at + 2;
        let mut depth = 0_i32;
        let mut kind = Vec::new();
        while walk < end {
            match word_at(tokens, walk) {
                "<" | "(" | "[" => depth += 1,
                ">" | ")" | "]" => depth -= 1,
                "," if depth == 0 => break,
                _ => {}
            }
            if let Some(one) = tokens.get(walk) {
                kind.push(one.clone());
            }
            walk += 1;
        }
        found.push((name, shown, kind));
        at = walk + 1;
    }
    found
}

fn opened_after(tokens: &[Token], at: usize) -> usize {
    let open = at + 2;
    if word_at(tokens, open) == "<" {
        closing(tokens, open) + 1
    } else {
        open
    }
}

fn returned(tokens: &[Token], from: usize, owner: &str) -> Vec<Token> {
    let mut kind = Vec::new();
    let mut walk = from;
    let mut depth = 0_i32;
    while let Some(one) = tokens.get(walk) {
        match one.text.as_str() {
            "{" | ";" | "where" if depth == 0 => break,
            "<" | "(" | "[" => depth += 1,
            ">" | ")" | "]" => depth -= 1,
            _ => {}
        }
        let mut one = one.clone();
        if one.text == "Self" {
            owner.clone_into(&mut one.text);
        }
        kind.push(one);
        walk += 1;
    }
    kind
}

fn carried(text: &str) -> Vec<String> {
    let tokens = lexed(text);
    let places = placed(&tokens);
    let mut structs: Vec<(String, Vec<Field>)> = Vec::new();
    let mut held: Vec<(String, Vec<Token>)> = Vec::new();
    for (at, one) in tokens.iter().enumerate() {
        let open = opened_after(&tokens, at);
        match one.text.as_str() {
            "struct" if word_at(&tokens, open) == "{" => {
                structs.push((
                    word_at(&tokens, at + 1).to_owned(),
                    fields(&tokens, open, false),
                ));
            }
            "enum" if word_at(&tokens, open) == "{" && visible(&tokens, at) => {
                let end = closing(&tokens, open);
                let mut walk = open + 1;
                while walk < end {
                    match word_at(&tokens, walk) {
                        "{" => {
                            for (name, _, kind) in fields(&tokens, walk, true) {
                                held.push((name, kind));
                            }
                            walk = closing(&tokens, walk);
                        }
                        "(" => walk = closing(&tokens, walk),
                        _ => {}
                    }
                    walk += 1;
                }
            }
            "fn" if visible(&tokens, at) => {
                let shut = closing(&tokens, open);
                if word_at(&tokens, shut + 1) == "->" {
                    let owner = places
                        .get(at)
                        .map(|place| place.owner.as_str())
                        .unwrap_or("");
                    held.push((
                        word_at(&tokens, at + 1).to_owned(),
                        returned(&tokens, shut + 2, owner),
                    ));
                }
            }
            _ => {}
        }
    }
    let mut timed = std::collections::BTreeSet::new();
    loop {
        let before = timed.len();
        for (name, inside) in &structs {
            if inside
                .iter()
                .any(|(_, shown, kind)| *shown && holds_time(kind, &timed))
            {
                timed.insert(name.clone());
            }
        }
        if timed.len() == before {
            break;
        }
    }
    let mut found: Vec<String> = structs
        .iter()
        .flat_map(|(_, inside)| inside.iter())
        .filter(|(_, shown, kind)| *shown && holds_time(kind, &timed))
        .map(|(name, _, _)| name.clone())
        .chain(
            held.iter()
                .filter(|(_, kind)| holds_time(kind, &timed))
                .map(|(name, _)| name.clone()),
        )
        .collect();
    found.sort();
    found.dedup();
    found
}

const CARRIERS: &[(&str, &str)] = &[
    ("src/host.rs", "after"),
    ("src/live.rs", "cheaper"),
    ("src/live.rs", "every"),
    ("src/live.rs", "listed"),
    ("src/resident.rs", "copying"),
    ("src/resident.rs", "frame"),
    ("src/resident.rs", "frame_reading"),
    ("src/resident.rs", "frames"),
    ("src/resident.rs", "on_device"),
    ("src/resident.rs", "patience"),
    ("src/resident.rs", "patience_of"),
    ("src/resident.rs", "pushing"),
    ("src/resident.rs", "recording"),
    ("src/resident.rs", "stage_times"),
    ("src/resident.rs", "staged"),
    ("src/resident.rs", "staged_frames"),
    ("src/resident.rs", "staged_reading"),
    ("src/resident.rs", "staged_reading_both"),
    ("src/resident.rs", "wait"),
    ("src/resident.rs", "wait_reading"),
    ("src/resident.rs", "wait_reading_both"),
    ("src/resident.rs", "warmed"),
];

const ALLOWED: &[(&str, &str, &str, usize)] = &[
    ("src/credit.rs", "Instant", "Marker::submitted", 1),
    ("src/credit.rs", "elapsed", "Marker::submitted", 1),
    ("src/credit.rs", "use time item", "", 1),
    ("src/live.rs", "Instant", "cheaper", 2),
    ("src/live.rs", "elapsed", "cheaper", 2),
    ("src/resident.rs", "Instant", "Host::buffers_in_with", 1),
    ("src/resident.rs", "Instant", "Resident::fetched", 1),
    ("src/resident.rs", "Instant", "Resident::run_times", 1),
    ("src/resident.rs", "Instant", "Resident::start", 1),
    ("src/resident.rs", "Instant", "Resident::write", 1),
    (
        "src/resident.rs",
        "cmd_write_timestamp",
        "Resident::run_times",
        4,
    ),
    ("src/resident.rs", "elapsed", "Host::buffers_in_with", 1),
    ("src/resident.rs", "elapsed", "Resident::fetched", 1),
    ("src/resident.rs", "elapsed", "Resident::land", 1),
    ("src/resident.rs", "elapsed", "Resident::run_times", 1),
    ("src/resident.rs", "elapsed", "Resident::write", 1),
    (
        "src/resident.rs",
        "get_query_pool_results",
        "Resident::split",
        1,
    ),
    (
        "src/resident.rs",
        "get_query_pool_results",
        "Resident::timed",
        1,
    ),
    ("src/resident.rs", "use time item", "", 1),
    ("src/serve.rs", "Instant", "gauged", 1),
    ("src/serve.rs", "Instant", "walked", 1),
    ("src/serve.rs", "carrier listed", "credit_answer", 2),
    ("src/serve.rs", "carrier listed", "poured", 1),
    ("src/serve.rs", "carrier listed", "sown", 1),
    ("src/serve.rs", "carrier on_device", "Staged::on_device", 1),
    ("src/serve.rs", "carrier on_device", "gauged", 1),
    ("src/serve.rs", "carrier on_device", "spent_on", 1),
    (
        "src/serve.rs",
        "carrier staged_frames",
        "Staged::submitted",
        1,
    ),
    ("src/serve.rs", "carrier staged_frames", "gauged", 1),
    ("src/serve.rs", "carrier staged_frames", "poured", 1),
    ("src/serve.rs", "elapsed", "gauged", 1),
    ("src/serve.rs", "elapsed", "walked", 1),
    ("src/serve3/back.rs", "carrier frame", "Strayed::refusal", 1),
    ("src/serve3/back.rs", "carrier frame", "Strayed::told", 1),
    ("src/serve3/back.rs", "carrier frame", "probed", 1),
    ("src/serve3/back.rs", "carrier frame", "walked", 3),
    (
        "src/serve3/back.rs",
        "carrier frames",
        "Strayed::refusal",
        1,
    ),
    ("src/serve3/back.rs", "carrier listed", "walked", 1),
    ("src/serve3/branch.rs", "carrier frame", "forked", 1),
    ("src/serve3/branch.rs", "carrier frame", "opened_file", 2),
    ("src/serve3/branch.rs", "carrier frame", "opened_kept", 1),
    ("src/serve3/branch.rs", "carrier frame", "restored", 2),
    ("src/serve3/branch.rs", "carrier frame", "saved", 1),
    ("src/serve3/branch.rs", "carrier frame", "shown_made", 1),
    ("src/serve3/branch.rs", "carrier listed", "forked", 2),
    ("src/serve3/branch.rs", "carrier listed", "listed", 1),
    ("src/serve3/branch.rs", "carrier listed", "opened_file", 2),
    ("src/serve3/branch.rs", "carrier listed", "opened_kept", 2),
    ("src/serve3/branch.rs", "carrier listed", "restored", 3),
    ("src/serve3/budget.rs", "Guard {", "Guard::default", 1),
    (
        "src/serve3/budget.rs",
        "carrier frame",
        "Halted::refusal",
        1,
    ),
    ("src/serve3/budget.rs", "carrier frames", "Pace::batch", 2),
    ("src/serve3/budget.rs", "carrier frames", "Pace::cost", 1),
    ("src/serve3/budget.rs", "carrier frames", "apart", 4),
    ("src/serve3/budget.rs", "carrier frames", "predicted", 2),
    ("src/serve3/budget.rs", "carrier frames", "split", 4),
    ("src/serve3/budget.rs", "carrier frames", "within", 1),
    ("src/serve3/budget.rs", "real", "Guard::default", 1),
    ("src/serve3/budget.rs", "use time item", "", 1),
    ("src/serve3/exec.rs", "Book::new", "Exec::spawn", 1),
    ("src/serve3/exec.rs", "Server::new", "Exec::spawned", 1),
    ("src/serve3/exec.rs", "recv_timeout", "work", 1),
    ("src/serve3/exec.rs", "sleep", "held_for_queued", 2),
    ("src/serve3/exec.rs", "sleep", "watch", 1),
    ("src/serve3/exec.rs", "sleep", "work", 2),
    ("src/serve3/exec.rs", "use time item", "", 1),
    ("src/serve3/gradient.rs", "carrier frame", "started", 1),
    ("src/serve3/gradient.rs", "carrier frames", "descended", 1),
    ("src/serve3/gradient.rs", "carrier frames", "graded", 1),
    ("src/serve3/gradient.rs", "carrier frames", "walk", 1),
    ("src/serve3/gradient.rs", "carrier listed", "descended", 2),
    ("src/serve3/gradient.rs", "carrier listed", "graded", 1),
    (
        "src/serve3/journal.rs",
        "carrier frame",
        "Ledger::journal",
        2,
    ),
    (
        "src/serve3/journal.rs",
        "carrier frame",
        "Ledger::push_priced",
        4,
    ),
    (
        "src/serve3/journal.rs",
        "carrier frame",
        "Ledger::snapped",
        1,
    ),
    ("src/serve3/journal.rs", "carrier frame", "base_record", 1),
    ("src/serve3/journal.rs", "carrier frame", "rebuilt", 10),
    ("src/serve3/journal.rs", "carrier frame", "state_of", 1),
    ("src/serve3/law.rs", "carrier frame", "params_written", 1),
    ("src/serve3/law.rs", "carrier listed", "params_written", 1),
    (
        "src/serve3/law_learn.rs",
        "carrier patience",
        "calibrated",
        1,
    ),
    ("src/serve3/law_learn.rs", "carrier warmed", "calibrated", 1),
    ("src/serve3/learn.rs", "carrier frame", "started", 1),
    ("src/serve3/learn.rs", "carrier frames", "objective_of", 1),
    ("src/serve3/learn.rs", "carrier frames", "scored", 1),
    ("src/serve3/learn.rs", "carrier frames", "searched", 1),
    ("src/serve3/learn.rs", "carrier listed", "scored", 1),
    ("src/serve3/learn.rs", "carrier listed", "searched", 2),
    ("src/serve3/mod.rs", "Book::empty", "Server::new", 1),
    ("src/serve3/mod.rs", "Book::new", "Server::with", 1),
    ("src/serve3/mod.rs", "Guard {", "Server::guard", 1),
    ("src/serve3/mod.rs", "carrier frame", "Book::landed", 1),
    ("src/serve3/mod.rs", "carrier frame", "Server::pending", 1),
    ("src/serve3/mod.rs", "carrier frame", "Server::rebuild", 2),
    (
        "src/serve3/mod.rs",
        "carrier frame",
        "Server::snap_attempt",
        1,
    ),
    (
        "src/serve3/mod.rs",
        "carrier frame",
        "Server::snap_behind",
        1,
    ),
    ("src/serve3/mod.rs", "carrier listed", "Server::rebuild", 1),
    ("src/serve3/mod.rs", "real", "Book::empty", 1),
    ("src/serve3/mod.rs", "real", "Book::new", 1),
    ("src/serve3/mod.rs", "recv_timeout", "Book::landed", 1),
    ("src/serve3/mod.rs", "sleep", "Server::idle_slowly", 1),
    ("src/serve3/mod.rs", "sleep", "Server::replayed_world", 1),
    ("src/serve3/mod.rs", "sleep", "Server::slowly", 1),
    ("src/serve3/mod.rs", "sleep", "Server::snap_attempt", 1),
    ("src/serve3/mod.rs", "sleep", "Server::snap_behind", 1),
    ("src/serve3/mod.rs", "use time item", "", 1),
    ("src/serve3/tools.rs", "carrier frame", "created", 1),
    ("src/serve3/tools.rs", "carrier frame", "read", 1),
    ("src/serve3/tools.rs", "carrier frame", "sliced", 1),
    ("src/serve3/tools.rs", "carrier frame", "stepped", 2),
    ("src/serve3/tools.rs", "carrier frame", "written", 1),
    ("src/serve3/tools.rs", "carrier listed", "created", 1),
    ("src/serve3/tools.rs", "carrier listed", "stepped", 1),
    ("src/serve3/tools.rs", "carrier listed", "written", 1),
    ("src/serve3/tools.rs", "sleep", "cells", 1),
    ("src/serve3/tools.rs", "sleep", "stall", 3),
    ("src/serve3/transport.rs", "carrier listed", "respond", 1),
];

fn carriers_named() -> Vec<String> {
    let mut names: Vec<String> = CARRIERS
        .iter()
        .map(|(_, name)| (*name).to_owned())
        .collect();
    names.sort();
    names.dedup();
    names
}

#[test]
fn every_carrier_of_a_measured_duration_is_listed() {
    let mut derived = Vec::new();
    let mut files = 0_usize;
    for (relative, text) in scanned() {
        if !carrier_file(&relative) {
            continue;
        }
        files += 1;
        for name in carried(&text) {
            derived.push((relative.clone(), name));
        }
    }
    assert!(files >= 3, "only {files} carrier files scanned");
    let listed: Vec<(String, String)> = CARRIERS
        .iter()
        .map(|(file, name)| ((*file).to_owned(), (*name).to_owned()))
        .collect();
    let missing: Vec<String> = derived
        .iter()
        .filter(|one| !listed.contains(one))
        .map(|(file, name)| format!("{file}: {name}"))
        .collect();
    let gone: Vec<String> = listed
        .iter()
        .filter(|one| !derived.contains(one))
        .map(|(file, name)| format!("{file}: {name}"))
        .collect();
    assert!(
        missing.is_empty() && gone.is_empty(),
        "carriers of a measured duration not listed:\n{}\nlisted and not found:\n{}",
        missing.join("\n"),
        gone.join("\n")
    );
}

#[test]
fn the_server_reads_time_only_through_its_clock() {
    let carriers = carriers_named();
    assert!(!carriers.is_empty(), "no carrier is listed");
    let mut seen: std::collections::BTreeMap<(String, String, String), Vec<usize>> =
        std::collections::BTreeMap::new();
    let mut files = 0_usize;
    let sources = scanned();
    let real = real_by_default(&sources);
    for (relative, text) in sources {
        files += 1;
        let server = server_file(&relative);
        let tokens = lexed(&text);
        let places = placed(&tokens);
        let mut kinds: Vec<(usize, String)> = Vec::new();
        for (at, _, words) in defaulted(&tokens) {
            if let Some(held) = words.iter().find(|word| real.contains(*word)) {
                kinds.push((at, format!("Default holding {held}")));
            }
        }
        for at in 0..tokens.len() {
            for kind in kinds_at(&tokens, &places, at, &carriers, server) {
                kinds.push((at, kind));
            }
        }
        for at in time_imports(&tokens, &places) {
            kinds.push((at, "use time item".to_owned()));
        }
        for (at, kind) in kinds {
            let place = places.get(at).map(Place::named).unwrap_or_default();
            let line = tokens.get(at).map_or(0, |one| one.line);
            seen.entry((relative.clone(), kind, place))
                .or_default()
                .push(line);
        }
    }
    assert!(files > 30, "only {files} sources scanned");
    let allowed: std::collections::BTreeMap<(String, String, String), usize> = ALLOWED
        .iter()
        .map(|(file, kind, place, count)| {
            (
                ((*file).to_owned(), (*kind).to_owned(), (*place).to_owned()),
                *count,
            )
        })
        .collect();
    let mut wrong = Vec::new();
    for (key, lines) in &seen {
        let wanted = allowed.get(key).copied().unwrap_or(0);
        if lines.len() != wanted {
            wrong.push(format!(
                "{} in {:?}: {} of {:?} at lines {lines:?}, and {wanted} are allowed",
                key.0,
                key.2,
                lines.len(),
                key.1
            ));
        }
    }
    for (key, wanted) in &allowed {
        if !seen.contains_key(key) {
            wrong.push(format!(
                "{} in {:?}: no {:?}, and the allowlist names {wanted}",
                key.0, key.2, key.1
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "time read outside the server's clock:\n{}",
        wrong.join("\n")
    );
}
