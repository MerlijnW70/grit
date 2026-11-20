#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

#[cfg(feature = "faults")]
mod common;

#[cfg(feature = "faults")]
mod faked {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use grit::Host;
    use grit::clock::{Fake, Time};
    use grit::serve3::journal::{Entry, Keep, Ledger, Made};
    use grit::serve3::{Options, Server};
    use grit::wire::{Json, written};

    use super::common::device;

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
            fake.script([Duration::ZERO, jump]);
            let got = call(&mut held.server, name, &arguments);
            assert_eq!(fake.scripted(), 0, "{path:?}: the jump was never read");
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
        fake.script([Duration::ZERO, Duration::ZERO, jump]);
        let got = call(&mut held.server, "grit_world_open", &kept_world(budget));
        assert_eq!(fake.scripted(), 0, "the jump was never read");
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

const ALLOWED: &[(&str, &str, &str, usize)] = &[
    ("src/credit.rs", "use Instant", "", 1),
    ("src/credit.rs", "Instant::now", "Marker::submitted", 1),
    ("src/credit.rs", "::now(", "Marker::submitted", 1),
    ("src/credit.rs", ".elapsed()", "Marker::submitted", 1),
    ("src/live.rs", "Instant::now", "cheaper", 2),
    ("src/live.rs", "::now(", "cheaper", 2),
    ("src/live.rs", ".elapsed()", "cheaper", 2),
    ("src/resident.rs", "use Instant", "", 1),
    ("src/resident.rs", "Instant::now", "Resident::write", 1),
    ("src/resident.rs", "::now(", "Resident::write", 1),
    ("src/resident.rs", ".elapsed()", "Resident::write", 1),
    ("src/resident.rs", "Instant::now", "Resident::fetched", 1),
    ("src/resident.rs", "::now(", "Resident::fetched", 1),
    ("src/resident.rs", ".elapsed()", "Resident::fetched", 1),
    ("src/resident.rs", "Instant::now", "Resident::start", 1),
    ("src/resident.rs", "::now(", "Resident::start", 1),
    ("src/resident.rs", ".elapsed()", "Resident::land", 1),
    ("src/resident.rs", "Instant::now", "Resident::run_times", 1),
    ("src/resident.rs", "::now(", "Resident::run_times", 1),
    ("src/resident.rs", ".elapsed()", "Resident::run_times", 1),
    ("src/serve.rs", "Instant::now", "gauged", 1),
    ("src/serve.rs", "::now(", "gauged", 1),
    ("src/serve.rs", ".elapsed()", "gauged", 1),
    ("src/serve.rs", "Instant::now", "walked", 1),
    ("src/serve.rs", "::now(", "walked", 1),
    ("src/serve.rs", ".elapsed()", "walked", 1),
    ("src/serve3/budget.rs", "use Instant", "", 1),
    ("src/serve3/budget.rs", "real(", "Guard::default", 1),
    ("src/serve3/exec.rs", "use Instant", "", 1),
    ("src/serve3/exec.rs", "sleep(", "watch", 1),
    ("src/serve3/exec.rs", "sleep(", "held_for_queued", 1),
    ("src/serve3/exec.rs", "recv_timeout(LANDING)", "work", 1),
    ("src/serve3/mod.rs", "use Instant", "", 1),
    ("src/serve3/mod.rs", "real(", "Book::new", 1),
    ("src/serve3/mod.rs", "real(", "Book::empty", 1),
    ("src/serve3/mod.rs", "recv_timeout(left)", "Book::landed", 1),
];

const SPELLED: [&str; 7] = [
    "Instant::now",
    "::now(",
    ".elapsed()",
    "SystemTime",
    "park_timeout",
    "Instant as",
    "UNIX_EPOCH",
];

fn ident(byte: Option<&u8>) -> bool {
    byte.is_some_and(|one| one.is_ascii_alphanumeric() || *one == b'_')
}

fn tokens(line: &str, token: &str) -> usize {
    let bytes = line.as_bytes();
    line.match_indices(token)
        .filter(|(at, _)| {
            let before = at.checked_sub(1).and_then(|one| bytes.get(one));
            let open = token.ends_with('(');
            !ident(before) && (open || !ident(bytes.get(at + token.len())))
        })
        .count()
}

fn sleeps(line: &str) -> usize {
    let bytes = line.as_bytes();
    line.match_indices("sleep(")
        .filter(|(at, _)| {
            let before = at.checked_sub(1).and_then(|one| bytes.get(one));
            before != Some(&b'.') && !ident(before)
        })
        .count()
}

fn kinds(line: &str, server: bool) -> Vec<(String, usize)> {
    let mut found: Vec<(String, usize)> = SPELLED
        .iter()
        .map(|spelled| ((*spelled).to_owned(), line.matches(spelled).count()))
        .collect();
    found.push(("sleep(".to_owned(), sleeps(line)));
    let waits = line.matches("recv_timeout").count();
    let named = ["recv_timeout(LANDING)", "recv_timeout(left)"]
        .into_iter()
        .find(|wait| line.contains(wait))
        .filter(|_| waits == 1)
        .unwrap_or("recv_timeout");
    found.push((named.to_owned(), waits));
    let trimmed = line.trim_start();
    let import = (trimmed.starts_with("use ") || trimmed.starts_with("pub use "))
        && tokens(line, "Instant") > 0;
    found.push(("use Instant".to_owned(), usize::from(import)));
    if server {
        found.push(("Real".to_owned(), tokens(line, "Real")));
        found.push(("real(".to_owned(), tokens(line, "real(")));
    } else {
        found.push(("clock::".to_owned(), line.matches("clock::").count()));
    }
    found.retain(|(_, count)| *count > 0);
    found
}

fn named(line: &str) -> Option<String> {
    let at = line.find("fn ")?;
    let before = at.checked_sub(1).and_then(|one| line.as_bytes().get(one));
    if ident(before) {
        return None;
    }
    let rest = line.get(at + 3..)?;
    let name: String = rest
        .chars()
        .take_while(|one| one.is_ascii_alphanumeric() || *one == '_')
        .collect();
    (!name.is_empty()).then_some(name)
}

fn implemented(line: &str) -> Option<String> {
    let rest = line.strip_prefix("impl")?;
    let rest = rest.strip_prefix(char::is_whitespace).or_else(|| {
        rest.strip_prefix('<')
            .and_then(|generic| generic.split_once('>').map(|(_, after)| after))
    })?;
    let rest = rest.split_once(" for ").map_or(rest, |(_, of)| of);
    let name: String = rest
        .trim_start()
        .chars()
        .take_while(|one| one.is_ascii_alphanumeric() || *one == '_')
        .collect();
    (!name.is_empty()).then_some(name)
}

#[test]
fn the_server_reads_time_only_through_its_clock() {
    let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = crate_root.join("src");
    let mut found = Vec::new();
    sources(&root, &mut found);
    assert!(found.len() > 40, "{found:?}");
    let mut seen: std::collections::BTreeMap<(String, String, String), Vec<usize>> =
        std::collections::BTreeMap::new();
    let mut scanned = 0_usize;
    for path in found {
        let relative = path
            .strip_prefix(crate_root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if relative.starts_with("src/bin/") || relative == "src/clock.rs" {
            continue;
        }
        scanned += 1;
        let server = relative == "src/serve.rs" || relative.starts_with("src/serve3/");
        let text = std::fs::read_to_string(&path).unwrap();
        let mut function = String::new();
        let mut owner = String::new();
        for (at, line) in text.lines().enumerate() {
            if line == "}" {
                owner.clear();
                function.clear();
                continue;
            }
            if let Some(name) = implemented(line) {
                owner = name;
                function.clear();
            }
            if let Some(name) = named(line) {
                function = if owner.is_empty() {
                    name
                } else {
                    format!("{owner}::{name}")
                };
            }
            let trimmed = line.trim_start();
            let import = trimmed.starts_with("use ") || trimmed.starts_with("pub use ");
            for (kind, count) in kinds(line, server) {
                let place = if import && kind == "use Instant" {
                    String::new()
                } else {
                    function.clone()
                };
                seen.entry((relative.clone(), kind, place))
                    .or_default()
                    .extend(std::iter::repeat_n(at + 1, count));
            }
        }
    }
    assert!(scanned > 30, "only {scanned} sources scanned");
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
