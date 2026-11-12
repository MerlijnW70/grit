#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use grit::Host;
use grit::rule3::heat;
use grit::sand3::Grid3;
use grit::serve3::exec::Exec;
use grit::serve3::journal::{self, Entry, Keep, Ledger, Made};
use grit::serve3::{Options, Server, rules};
use grit::wire::{Json, read, written};

const WAIT: Duration = Duration::from_secs(300);

struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("grit-m2-{label}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn dir(&self) -> PathBuf {
        self.0.join("worlds")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut mixed = *state;
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    mixed ^ (mixed >> 31)
}

fn line(id: i64, name: &str, arguments: &str) -> String {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"tools/call\",\"params\":{{\"name\":\"{name}\",\"arguments\":{arguments}}}}}"
    )
}

fn call(server: &mut Server<'_>, name: &str, arguments: &str) -> Json {
    server.heard(&line(7, name, arguments)).unwrap()
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

struct Told {
    code: String,
    field: String,
    message: String,
    all: Json,
}

fn refused(got: &Json) -> Told {
    let result = got.get("result").unwrap();
    assert_eq!(
        result.get("isError"),
        Some(&Json::Bool(true)),
        "{}",
        written(got)
    );
    let all = result.get("structuredContent").cloned().unwrap();
    Told {
        code: all.get("code").and_then(Json::text).unwrap().to_owned(),
        field: all.get("field").and_then(Json::text).unwrap().to_owned(),
        message: all.get("message").and_then(Json::text).unwrap().to_owned(),
        all,
    }
}

fn numbers(held: &Json, key: &str) -> Vec<u32> {
    held.get(key)
        .and_then(Json::list)
        .unwrap()
        .iter()
        .map(|one| u32::try_from(one.int().unwrap()).unwrap())
        .collect()
}

fn writing(cells: &[([u32; 3], u32)]) -> String {
    let held: Vec<String> = cells
        .iter()
        .map(|(at, word)| format!("[{},{},{},{word}]", at[0], at[1], at[2]))
        .collect();
    format!("[{}]", held.join(","))
}

fn described(server: &mut Server<'_>) -> Json {
    answer(&call(server, "grit_describe", "{}"))
}

struct Model {
    rule: &'static str,
    params: Vec<u32>,
    grid: Grid3,
    cells: Vec<u32>,
    frame: u32,
}

impl Model {
    fn new(rule: &'static str, params: &[u32], side: u32) -> Self {
        let grid = Grid3::new(side, side, side).unwrap();
        Self {
            rule,
            params: params.to_vec(),
            grid,
            cells: vec![0; grid.cells()],
            frame: 0,
        }
    }

    fn write(&mut self, cells: &[([u32; 3], u32)]) {
        for (at, word) in cells {
            self.cells[self.grid.index(at[0], at[1], at[2])] = *word;
        }
    }

    fn step(&mut self, frames: u32) {
        let listed = rules()
            .into_iter()
            .find(|one| one.name() == self.rule)
            .unwrap();
        for _ in 0..frames {
            self.cells = listed.reference(self.grid, self.frame, &self.cells, &self.params);
            self.frame += 1;
        }
    }

    fn create(&self, sparse: bool) -> String {
        format!(
            "{{\"wide\":{side},\"deep\":{side},\"tall\":{side},\"rule\":\"{}\",\"params\":{:?},\"sparse\":{sparse}}}",
            self.rule,
            self.params,
            side = self.grid.wide
        )
    }

    fn read(&self, world: impl std::fmt::Display) -> String {
        let last = self.grid.wide - 1;
        format!("{{\"world\":{world},\"from\":[0,0,0],\"to\":[{last},{last},{last}]}}")
    }
}

fn scattered(state: &mut u64, side: u32, many: usize, words: &[u32]) -> Vec<([u32; 3], u32)> {
    (0..many)
        .map(|_| {
            let at = next(state);
            let word = words[(at >> 40) as usize % words.len()];
            (
                [
                    (at % u64::from(side)) as u32,
                    ((at >> 10) % u64::from(side)) as u32,
                    ((at >> 20) % u64::from(side)) as u32,
                ],
                word,
            )
        })
        .collect()
}

fn heat_words(state: &mut u64, many: usize) -> Vec<u32> {
    (0..many)
        .map(|_| {
            let at = next(state);
            let kind =
                [heat::WALL, heat::WATER, heat::FUEL, heat::FIRE, heat::ICE][(at % 5) as usize];
            heat::cell(kind, ((at >> 8) % 1200) as u32)
        })
        .collect()
}

const SAND_WORDS: [u32; 3] = [grit::sand::SAND, grit::sand::WATER, grit::sand::WALL];

struct Run {
    exec: Exec,
    answers: Receiver<Json>,
    id: i64,
    early: BTreeMap<i64, Json>,
}

fn opener() -> grit::serve3::exec::Opener {
    let pattern = std::env::var("GRIT_DEVICE")
        .ok()
        .filter(|one| !one.is_empty());
    Box::new(move || Host::matching(pattern.as_deref()))
}

impl Run {
    fn new(options: &Options) -> Self {
        let (exec, answers) = Exec::spawn(opener(), options).unwrap();
        Self {
            exec,
            answers,
            id: 100,
            early: BTreeMap::new(),
        }
    }

    fn send(&mut self, name: &str, arguments: &str) -> i64 {
        self.id += 1;
        self.exec.line(&line(self.id, name, arguments));
        self.id
    }

    fn cancel(&self, id: i64) {
        self.exec.line(&format!(
            "{{\"jsonrpc\":\"2.0\",\"method\":\"notifications/cancelled\",\"params\":{{\"requestId\":{id},\"reason\":\"test\"}}}}"
        ));
    }

    fn wait(&mut self, id: i64) -> Json {
        if let Some(got) = self.early.remove(&id) {
            return got;
        }
        loop {
            let got = self.answers.recv_timeout(WAIT).unwrap();
            let Some(came) = got.get("id").and_then(Json::int) else {
                panic!("an answer without an id: {}", written(&got));
            };
            if came == id {
                return got;
            }
            self.early.insert(came, got);
        }
    }

    fn ask(&mut self, name: &str, arguments: &str) -> Json {
        let id = self.send(name, arguments);
        self.wait(id)
    }

    fn batches(&mut self) -> i64 {
        let got = answer(&self.ask("grit_describe", "{}"));
        got.get("submissions").unwrap().int_at("batches")
    }
}

trait At {
    fn int_at(&self, key: &str) -> i64;
}

impl At for Json {
    fn int_at(&self, key: &str) -> i64 {
        self.get(key)
            .and_then(Json::int)
            .unwrap_or_else(|| panic!("no {key} in {}", written(self)))
    }
}

fn bad_names() -> Vec<&'static str> {
    vec![
        "",
        "..",
        "../outside",
        "..\\outside",
        "/etc/passwd",
        "\\windows\\win.ini",
        "C:\\Windows\\win",
        "C:outside",
        "c:",
        "\\\\server\\share\\world",
        "//server/share/world",
        "\\\\?\\C:\\world",
        "a/b",
        "a\\b",
        "a.b",
        "world.grit",
        "CON",
        "con",
        "nul",
        "prn",
        "aux",
        "com1",
        "lpt9",
        "Upper",
        "-dash",
        "_under",
        "space name",
        "caf\u{e9}",
        "nul\u{0}",
        "world-3",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    ]
}

fn entries(path: &Path) -> Vec<String> {
    let mut found: Vec<String> = std::fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    found.sort();
    found
}

#[test]
fn a_name_that_would_leave_the_directory_is_refused_by_name() {
    let scratch = Scratch::new("names");
    let options = Options {
        dir: Some(scratch.dir()),
        memory: None,
    };
    let mut server = Server::with(None, &options).unwrap();
    let outside = entries(&scratch.0);
    let inside = entries(&scratch.dir());
    for name in bad_names() {
        let quoted = written(&Json::Text(name.to_owned()));
        for (tool, arguments) in [
            (
                "grit_world_save",
                format!("{{\"world\":1,\"name\":{quoted}}}"),
            ),
            ("grit_world_open", format!("{{\"name\":{quoted}}}")),
        ] {
            let told = refused(&call(&mut server, tool, &arguments));
            if tool == "grit_world_open" && name == "world-3" {
                assert_eq!(told.code, "no_file", "{tool} {name:?}: {}", told.message);
                continue;
            }
            assert_eq!(
                (told.code.as_str(), told.field.as_str()),
                ("path", "$.name"),
                "{tool} {name:?}: {}",
                told.message
            );
            assert!(told.message.len() < 400, "{}", told.message);
        }
    }
    let told = refused(&call(
        &mut server,
        "grit_world_save",
        "{\"world\":1,\"name\":\"fine-name_2\"}",
    ));
    assert_eq!(told.code, "no_world");
    let told = refused(&call(
        &mut server,
        "grit_world_open",
        "{\"name\":\"fine-name_2\"}",
    ));
    assert_eq!(told.code, "no_file");
    assert_eq!(entries(&scratch.0), outside);
    assert_eq!(entries(&scratch.dir()), inside);
}

#[cfg(windows)]
fn link_to(target: &Path, link: &Path) -> bool {
    std::fs::create_dir_all(target).unwrap();
    Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()
        .is_ok_and(|done| done.status.success())
}

#[cfg(unix)]
fn link_to(target: &Path, link: &Path) -> bool {
    std::fs::write(target, b"GRITWLD1").unwrap();
    std::os::unix::fs::symlink(target, link).is_ok()
}

#[test]
fn a_link_in_the_directory_is_refused_as_a_path() {
    let scratch = Scratch::new("links");
    let options = Options {
        dir: Some(scratch.dir()),
        memory: None,
    };
    let mut server = Server::with(None, &options).unwrap();
    let target = scratch.0.join("outside");
    assert!(
        link_to(&target, &scratch.dir().join("sneaky.grit")),
        "no link could be made here"
    );
    let told = refused(&call(
        &mut server,
        "grit_world_open",
        "{\"name\":\"sneaky\"}",
    ));
    assert_eq!(
        (told.code.as_str(), told.field.as_str()),
        ("path", "$.name")
    );
    assert!(told.message.contains("link"), "{}", told.message);
    assert!(told.all.get("hint").and_then(Json::text).is_some());
    let told = refused(&call(
        &mut server,
        "grit_world_save",
        "{\"world\":1,\"name\":\"sneaky\",\"replace\":true}",
    ));
    assert_eq!(
        (told.code.as_str(), told.field.as_str()),
        ("path", "$.name")
    );
    let listed = answer(&call(&mut server, "grit_world_list", "{}"));
    assert_eq!(listed.int_at("total"), 0, "{}", written(&listed));
}

#[test]
fn describe_names_the_budgets_the_memory_and_the_new_codes() {
    let mut server = Server::new(None);
    let told = described(&mut server);
    let codes: Vec<&str> = told
        .get("codes")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .map(|one| one.get("code").and_then(Json::text).unwrap())
        .collect();
    for code in [
        "memory",
        "budget",
        "cancelled",
        "device_lost",
        "path",
        "no_file",
        "exists",
        "bad_file",
        "unlike",
        "io",
    ] {
        assert!(codes.contains(&code), "{code}");
    }
    let limits = told.get("limits").unwrap();
    assert_eq!(limits.int_at("budget_ms"), 30_000);
    assert_eq!(limits.int_at("budget_most_ms"), 600_000);
    assert_eq!(limits.int_at("submission_ms"), 250);
    assert_eq!(limits.int_at("batch_frames"), 64);
    assert_eq!(limits.int_at("name_bytes"), 64);
    let memory = told.get("memory").unwrap();
    assert_eq!(memory.int_at("allowed_bytes"), 0);
    assert_eq!(memory.int_at("held_bytes"), 0);
    let tools = told.get("tools").and_then(Json::list).unwrap();
    for name in [
        "grit_world_save",
        "grit_world_open",
        "grit_world_list",
        "grit_world_fork",
        "grit_world_restore",
    ] {
        assert!(tools.contains(&Json::Text(name.to_owned())), "{name}");
    }
}

#[test]
fn a_budget_outside_its_range_is_refused_before_anything_runs() {
    let mut server = Server::new(None);
    let got = call(
        &mut server,
        "grit_world_step",
        "{\"world\":1,\"budget_ms\":1.5}",
    );
    assert_eq!(
        got.get("error")
            .and_then(|one| one.get("code"))
            .and_then(Json::int),
        Some(-32700)
    );
    for budget in ["0", "600001", "-5", "\"fast\"", "null"] {
        let told = refused(&call(
            &mut server,
            "grit_world_step",
            &format!("{{\"world\":1,\"budget_ms\":{budget}}}"),
        ));
        assert_eq!(told.field, "$.budget_ms", "{budget}: {}", told.message);
    }
    let told = refused(&call(
        &mut server,
        "grit_world_step",
        "{\"world\":1,\"budget_ms\":600000}",
    ));
    assert_eq!(told.code, "no_world");
}

#[test]
fn a_call_counts_its_budget_from_its_own_arrival() {
    let (exec, answers) = Exec::spawn(Box::new(|| Ok(None)), &Options::default()).unwrap();
    let flat = "{\"inputs\":2,\"outputs\":1,\"gates\":1,\"table\":[0,0,0,1],\"chip\":[{\"kind\":8,\"left\":0,\"right\":1}]}";
    exec.line(&line(1, "grit_check", flat));
    let got = answers.recv_timeout(WAIT).unwrap();
    assert_eq!(got.get("id").and_then(Json::int), Some(1));
    exec.line(&line(2, "grit_world_rules", "{}"));
    let got = answers.recv_timeout(WAIT).unwrap();
    assert_eq!(got.get("id").and_then(Json::int), Some(2));
    std::thread::sleep(Duration::from_millis(300));
    exec.line(&line(3, "grit_world_rules", "{\"budget_ms\":100}"));
    let got = answers.recv_timeout(WAIT).unwrap();
    assert_eq!(got.get("id").and_then(Json::int), Some(3));
    let told = answer(&got);
    assert!(told.get("rules").is_some(), "{}", written(&told));
    exec.line(
        "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/cancelled\",\"params\":{\"requestId\":4}}",
    );
    exec.line(&line(4, "grit_world_rules", "{}"));
    let got = answers.recv_timeout(WAIT).unwrap();
    assert_eq!(got.get("id").and_then(Json::int), Some(4));
    answer(&got);
    exec.finish();
}

#[test]
fn a_list_of_many_files_comes_in_pages_that_join_to_the_whole() {
    let scratch = Scratch::new("pages");
    let options = Options {
        dir: Some(scratch.dir()),
        memory: None,
    };
    let mut server = Server::with(None, &options).unwrap();
    let long = "f".repeat(59);
    let many = 12_000;
    for at in 0..many {
        std::fs::write(scratch.dir().join(format!("{long}{at:04}.grit")), b"x").unwrap();
    }
    std::fs::write(scratch.dir().join("Not-Listed.grit"), b"x").unwrap();
    std::fs::write(scratch.dir().join("other.txt"), b"x").unwrap();
    std::fs::create_dir(scratch.dir().join("folder.grit")).unwrap();
    let mut names = Vec::new();
    let mut cursor = 0;
    let mut pages = 0;
    loop {
        let began = Instant::now();
        let got = server
            .heard(&line(
                7,
                "grit_world_list",
                &format!("{{\"cursor\":{cursor}}}"),
            ))
            .unwrap();
        assert!(written(&got).len() <= 1 << 20);
        assert!(began.elapsed() < Duration::from_secs(5));
        let page = answer(&got);
        assert_eq!(page.int_at("total"), many);
        for item in page.get("items").and_then(Json::list).unwrap() {
            assert_eq!(item.get("kind").and_then(Json::text), Some("saved"));
            names.push(item.get("name").and_then(Json::text).unwrap().to_owned());
        }
        pages += 1;
        match page.get("next").and_then(Json::int) {
            Some(next) => cursor = next,
            None => break,
        }
    }
    assert!(pages > 1, "{pages}");
    let mut want: Vec<String> = (0..many).map(|at| format!("{long}{at:04}")).collect();
    want.sort();
    assert_eq!(names, want);
    let told = refused(&call(
        &mut server,
        "grit_world_list",
        &format!("{{\"cursor\":{}}}", many + 1),
    ));
    assert_eq!(
        (told.code.as_str(), told.field.as_str()),
        ("outside", "$.cursor")
    );
}

#[test]
fn a_journal_torn_at_its_end_keeps_every_whole_record_before_it() {
    let scratch = Scratch::new("torn");
    let keep = Keep::at(&scratch.dir()).unwrap();
    let made = Made {
        rule: "sand".to_owned(),
        params: Vec::new(),
        size: [16, 16, 16],
        sparse: true,
        slots: Some(9),
    };
    let mut ledger = Ledger::created(made.clone());
    keep.begin(5, &ledger).unwrap();
    let entries = [
        Entry::Write(vec![([1, 2, 3], 4), ([5, 6, 7], 8)]),
        Entry::Step(64),
        Entry::Write(vec![([0, 0, 0], 1)]),
        Entry::Step(3),
    ];
    for entry in &entries {
        ledger.push(entry.clone(), Duration::ZERO);
        keep.append(5, entry).unwrap();
    }
    let path = scratch.dir().join("kept").join("world-5.journal");
    let whole = std::fs::read(&path).unwrap();
    let found = keep.earlier();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].0, 5);
    assert_eq!(found[0].1.entries, entries);
    assert_eq!(found[0].1.frame, 67);
    assert_eq!(found[0].1.made(), &made);
    for cut in 1..12 {
        std::fs::write(&path, &whole[..whole.len() - cut]).unwrap();
        let found = keep.earlier();
        assert_eq!(found[0].1.entries, entries[..3], "cut {cut}");
        assert_eq!(found[0].1.frame, 64);
    }
    let mut flipped = whole.clone();
    let at = whole.len() - 30;
    flipped[at] ^= 0x10;
    std::fs::write(&path, &flipped).unwrap();
    let found = keep.earlier();
    assert!(found[0].1.entries.len() < entries.len());
    std::fs::write(&path, &whole[..10]).unwrap();
    assert!(keep.earlier().is_empty());
    std::fs::write(scratch.dir().join("kept").join("world-007.journal"), &whole).unwrap();
    std::fs::write(scratch.dir().join("kept").join("world-0.journal"), &whole).unwrap();
    assert!(keep.earlier().is_empty());
    std::fs::write(&path, &whole).unwrap();
    let mut server = Server::with(
        None,
        &Options {
            dir: Some(scratch.dir()),
            memory: None,
        },
    )
    .unwrap();
    let listed = answer(&call(&mut server, "grit_world_list", "{}"));
    let item = &listed.get("items").and_then(Json::list).unwrap()[0];
    assert_eq!(item.get("kind").and_then(Json::text), Some("kept"));
    assert_eq!(item.get("name").and_then(Json::text), Some("world-5"));
    assert_eq!(item.int_at("frame"), 67);
    let told = refused(&call(&mut server, "grit_world_step", "{\"world\":5}"));
    assert_eq!(told.code, "no_world");
    assert!(told.message.contains("grit_world_open"), "{}", told.message);
    let created = refused(&call(
        &mut server,
        "grit_world_create",
        "{\"wide\":2,\"deep\":2,\"tall\":2,\"rule\":\"sand\"}",
    ));
    assert_eq!(created.code, "no_compute_device");
    let told = refused(&call(
        &mut server,
        "grit_world_open",
        "{\"name\":\"world-5\"}",
    ));
    assert_eq!(told.code, "no_compute_device");
}

#[test]
fn the_state_hash_reads_a_uniform_brick_held_or_not_as_the_same_cells() {
    use grit::branch::{Image, Law, State};
    let law = Law::Rule {
        name: "sand".to_owned(),
    };
    let held = Image {
        law: law.clone(),
        params: Vec::new(),
        frame: 9,
        size: [2, 2, 2],
        state: State::Sparse {
            capacity: 2,
            sides: [2, 2, 2],
            table: vec![0, 0],
            pool: vec![7; 8],
        },
    };
    let uniform = Image {
        state: State::Sparse {
            capacity: 2,
            sides: [2, 2, 2],
            table: vec![u32::MAX, 7],
            pool: Vec::new(),
        },
        ..held.clone()
    };
    assert_eq!(journal::state_of(&held), journal::state_of(&uniform));
    let mut moved = held.clone();
    moved.frame = 10;
    assert_ne!(journal::state_of(&held), journal::state_of(&moved));
    let mut other = held.clone();
    if let State::Sparse { pool, .. } = &mut other.state {
        pool[3] = 6;
    }
    assert_ne!(journal::state_of(&held), journal::state_of(&other));
}

fn create(server: &mut Server<'_>, arguments: &str) -> u32 {
    let got = answer(&call(server, "grit_world_create", arguments));
    u32::try_from(got.int_at("world")).unwrap()
}

fn state(server: &mut Server<'_>, world: u32, name: &str) -> (String, i64) {
    let got = answer(&call(
        server,
        "grit_world_save",
        &format!("{{\"world\":{world},\"name\":\"{name}\",\"replace\":true}}"),
    ));
    (
        got.get("state").and_then(Json::text).unwrap().to_owned(),
        got.int_at("frame"),
    )
}

#[test]
fn a_saved_world_opens_as_itself_and_a_changed_file_is_refused() {
    let Some(host) = device("save and open") else {
        return;
    };
    let scratch = Scratch::new("save");
    let options = Options {
        dir: Some(scratch.dir()),
        memory: None,
    };
    let mut server = Server::with(Some(&host), &options).unwrap();
    let mut seed = 0x5A7E_u64;
    for (rule, params, sparse) in [
        ("sand", Vec::new(), false),
        ("heat", heat::TUNED.to_vec(), true),
    ] {
        let mut model = Model::new(rule, &params, 16);
        let world = create(&mut server, &model.create(sparse));
        let cells = if rule == "sand" {
            scattered(&mut seed, 16, 300, &SAND_WORDS)
        } else {
            let words = heat_words(&mut seed, 300);
            scattered(&mut seed, 16, 300, &[0])
                .into_iter()
                .zip(words)
                .map(|((at, _), word)| (at, word))
                .collect()
        };
        answer(&call(
            &mut server,
            "grit_world_write",
            &format!("{{\"world\":{world},\"cells\":{}}}", writing(&cells)),
        ));
        model.write(&cells);
        answer(&call(
            &mut server,
            "grit_world_step",
            &format!("{{\"world\":{world},\"frames\":21}}"),
        ));
        model.step(21);
        let name = format!("{rule}-at-21");
        let saved = answer(&call(
            &mut server,
            "grit_world_save",
            &format!("{{\"world\":{world},\"name\":\"{name}\"}}"),
        ));
        assert_eq!(saved.int_at("frame"), 21);
        let told = refused(&call(
            &mut server,
            "grit_world_save",
            &format!("{{\"world\":{world},\"name\":\"{name}\"}}"),
        ));
        assert_eq!(
            (told.code.as_str(), told.field.as_str()),
            ("exists", "$.name")
        );
        let opened = answer(&call(
            &mut server,
            "grit_world_open",
            &format!("{{\"name\":\"{name}\"}}"),
        ));
        let copy = u32::try_from(opened.int_at("world")).unwrap();
        assert_eq!(opened.int_at("frame"), 21);
        assert_eq!(opened.get("name").and_then(Json::text), Some(name.as_str()));
        for one in [world, copy] {
            answer(&call(
                &mut server,
                "grit_world_step",
                &format!("{{\"world\":{one},\"frames\":9}}"),
            ));
        }
        model.step(9);
        let (first, _) = state(&mut server, world, "first");
        let (second, frame) = state(&mut server, copy, "second");
        assert_eq!(first, second, "{rule}");
        assert_eq!(frame, 30);
        for one in [world, copy] {
            let got = answer(&call(&mut server, "grit_world_read", &model.read(one)));
            assert_eq!(numbers(&got, "cells"), model.cells, "{rule} world {one}");
        }
        let path = scratch.dir().join(format!("{name}.grit"));
        let bytes = std::fs::read(&path).unwrap();
        for at in [0, 9, bytes.len() / 2, bytes.len() - 1] {
            let mut changed = bytes.clone();
            changed[at] ^= 0x41;
            std::fs::write(&path, &changed).unwrap();
            let told = refused(&call(
                &mut server,
                "grit_world_open",
                &format!("{{\"name\":\"{name}\"}}"),
            ));
            assert_eq!(
                (told.code.as_str(), told.field.as_str()),
                ("bad_file", "$.name"),
                "byte {at}: {}",
                told.message
            );
        }
        std::fs::write(&path, &bytes[..bytes.len() / 3]).unwrap();
        let told = refused(&call(
            &mut server,
            "grit_world_open",
            &format!("{{\"name\":\"{name}\"}}"),
        ));
        assert_eq!(told.code, "bad_file");
    }
    let kept = entries(&scratch.dir().join("kept"));
    let snapshots = kept.iter().filter(|one| one.starts_with("snap-")).count();
    assert_eq!(snapshots, 2, "{kept:?}");
    let listed = answer(&call(&mut server, "grit_world_list", "{}"));
    let open = listed
        .get("items")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .filter(|one| one.get("kind").and_then(Json::text) == Some("open"))
        .count();
    assert_eq!(open, 4);
}

#[test]
fn a_fork_steps_as_its_source_and_a_restore_takes_the_fork_back() {
    let Some(host) = device("fork and restore") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let mut seed = 0xF0F0_u64;
    for sparse in [false, true] {
        let mut model = Model::new("sand", &[], 16);
        let world = create(&mut server, &model.create(sparse));
        let cells = scattered(&mut seed, 16, 400, &SAND_WORDS);
        answer(&call(
            &mut server,
            "grit_world_write",
            &format!("{{\"world\":{world},\"cells\":{}}}", writing(&cells)),
        ));
        model.write(&cells);
        answer(&call(
            &mut server,
            "grit_world_step",
            &format!("{{\"world\":{world},\"frames\":5}}"),
        ));
        model.step(5);
        let at_fork = model.cells.clone();
        let mut forks = Vec::new();
        for _ in 0..2 {
            let forked = answer(&call(
                &mut server,
                "grit_world_fork",
                &format!("{{\"world\":{world}}}"),
            ));
            assert_eq!(forked.int_at("frame"), 5);
            assert_eq!(forked.int_at("from"), i64::from(world));
            forks.push(u32::try_from(forked.int_at("world")).unwrap());
        }
        let (fork, undo) = (forks[0], forks[1]);
        for one in [world, fork] {
            answer(&call(
                &mut server,
                "grit_world_step",
                &format!("{{\"world\":{one},\"frames\":40}}"),
            ));
        }
        model.step(40);
        for one in [world, fork] {
            let got = answer(&call(&mut server, "grit_world_read", &model.read(one)));
            assert_eq!(numbers(&got, "cells"), model.cells, "sparse {sparse}");
            assert_eq!(got.int_at("frame"), 45);
        }
        let back = answer(&call(
            &mut server,
            "grit_world_restore",
            &format!("{{\"world\":{world},\"from\":{undo}}}"),
        ));
        assert_eq!(back.int_at("frame"), 5);
        let got = answer(&call(&mut server, "grit_world_read", &model.read(world)));
        assert_eq!(numbers(&got, "cells"), at_fork, "sparse {sparse}");
        answer(&call(
            &mut server,
            "grit_world_step",
            &format!("{{\"world\":{world},\"frames\":40}}"),
        ));
        let got = answer(&call(&mut server, "grit_world_read", &model.read(world)));
        assert_eq!(numbers(&got, "cells"), model.cells, "sparse {sparse}");
        assert_eq!(got.int_at("frame"), 45);
        let undone = answer(&call(&mut server, "grit_world_read", &model.read(undo)));
        assert_eq!(numbers(&undone, "cells"), at_fork);
        let other = create(
            &mut server,
            "{\"wide\":8,\"deep\":8,\"tall\":8,\"rule\":\"sand\"}",
        );
        let told = refused(&call(
            &mut server,
            "grit_world_restore",
            &format!("{{\"world\":{world},\"from\":{other}}}"),
        ));
        assert_eq!(told.code, "unlike", "{}", told.message);
        answer(&call(
            &mut server,
            "grit_world_close",
            &format!("{{\"world\":{undo}}}"),
        ));
        let told = refused(&call(
            &mut server,
            "grit_world_restore",
            &format!("{{\"world\":{world},\"from\":{undo}}}"),
        ));
        assert_eq!(
            (told.code.as_str(), told.field.as_str()),
            ("closed", "$.from")
        );
        let told = refused(&call(
            &mut server,
            "grit_world_fork",
            &format!("{{\"world\":{undo}}}"),
        ));
        assert_eq!(told.code, "closed");
        let same = answer(&call(
            &mut server,
            "grit_world_restore",
            &format!("{{\"world\":{world},\"from\":{world}}}"),
        ));
        assert_eq!(same.int_at("frame"), 45);
        for one in [world, fork, other] {
            answer(&call(
                &mut server,
                "grit_world_close",
                &format!("{{\"world\":{one}}}"),
            ));
        }
    }
}

#[cfg(feature = "faults")]
#[test]
fn a_world_past_the_memory_budget_is_refused_before_anything_is_made() {
    use grit::resident::faults::{Step, count};
    let Some(host) = device("memory budget") else {
        return;
    };
    let roomy = Server::new(Some(&host));
    let heap = host.heap();
    let mut probe = roomy;
    assert_eq!(
        described(&mut probe)
            .get("memory")
            .unwrap()
            .int_at("allowed_bytes"),
        i64::try_from(grit::serve3::budget::Memory::share_of(heap)).unwrap()
    );
    assert_eq!(grit::serve3::budget::Memory::share_of(4_000), 3_000);
    let made = "{\"wide\":32,\"deep\":32,\"tall\":32,\"rule\":\"sand\"}";
    create(&mut probe, made);
    let one = u64::try_from(
        described(&mut probe)
            .get("memory")
            .unwrap()
            .int_at("held_bytes"),
    )
    .unwrap();
    assert!(one > 32 * 32 * 32 * 4, "{one}");
    drop(probe);
    let options = Options {
        dir: None,
        memory: Some(one + one / 2),
    };
    let mut server = Server::with(Some(&host), &options).unwrap();
    let first = create(&mut server, made);
    let buffers = count(Step::Buffer).made;
    let memories = count(Step::Memory).made;
    for (tool, arguments) in [
        ("grit_world_create", made.to_owned()),
        ("grit_world_fork", format!("{{\"world\":{first}}}")),
        (
            "grit_world_create",
            "{\"wide\":64,\"deep\":64,\"tall\":64,\"rule\":\"sand\",\"sparse\":true}".to_owned(),
        ),
    ] {
        let told = refused(&call(&mut server, tool, &arguments));
        assert_eq!(told.code, "memory", "{tool}: {}", told.message);
        assert_eq!(told.all.int_at("held"), i64::try_from(one).unwrap());
        assert_eq!(
            told.all.int_at("allowed"),
            i64::try_from(one + one / 2).unwrap()
        );
        assert_eq!(
            told.all.int_at("limit"),
            i64::try_from(one + one / 2).unwrap()
        );
        assert!(told.all.int_at("asked") > i64::try_from(one / 2).unwrap());
        assert!(told.message.contains(&format!("{one}")), "{}", told.message);
    }
    assert_eq!(count(Step::Buffer).made, buffers);
    assert_eq!(count(Step::Memory).made, memories);
    let stepped = answer(&call(
        &mut server,
        "grit_world_step",
        &format!("{{\"world\":{first},\"frames\":3}}"),
    ));
    assert_eq!(stepped.int_at("frame"), 3);
    answer(&call(
        &mut server,
        "grit_world_close",
        &format!("{{\"world\":{first}}}"),
    ));
    let second = create(&mut server, made);
    assert!(second > first);
    assert_eq!(
        described(&mut server)
            .get("memory")
            .unwrap()
            .int_at("held_bytes"),
        i64::try_from(one).unwrap()
    );
}

fn budgeted_side(host: &Host) -> u32 {
    let sand = rules()
        .into_iter()
        .find(|one| one.name() == "sand")
        .unwrap();
    let mut side = 512;
    while side > 32 {
        let made = Made {
            rule: "sand".to_owned(),
            params: Vec::new(),
            size: [side, side, side],
            sparse: true,
            slots: None,
        };
        if let Ok((_, sizes)) = grit::serve3::shaped(host, sand.as_ref(), &made)
            && host.limits().admits(&sizes, None).is_ok()
        {
            return side;
        }
        side /= 2;
    }
    side
}

#[test]
fn a_long_step_on_a_large_world_returns_within_its_budget_with_whole_frames() {
    let Some(host) = device("budgeted step") else {
        return;
    };
    let side = budgeted_side(&host);
    eprintln!("budgeted step on a sparse world of side {side}");
    let mut server = Server::new(Some(&host));
    let world = create(
        &mut server,
        &format!(
            "{{\"wide\":{side},\"deep\":{side},\"tall\":{side},\"rule\":\"sand\",\"sparse\":true}}"
        ),
    );
    let mut seed = 0xB0D6_u64;
    let cells: Vec<([u32; 3], u32)> = scattered(&mut seed, 64, 4096, &SAND_WORDS)
        .into_iter()
        .map(|(at, word)| ([at[0] + side / 2 - 32, at[1], at[2] + side / 2 - 32], word))
        .collect();
    answer(&call(
        &mut server,
        "grit_world_write",
        &format!("{{\"world\":{world},\"cells\":{}}}", writing(&cells)),
    ));
    let fork = u32::try_from(
        answer(&call(
            &mut server,
            "grit_world_fork",
            &format!("{{\"world\":{world}}}"),
        ))
        .int_at("world"),
    )
    .unwrap();
    let budget = 500;
    let began = Instant::now();
    let got = answer(&call(
        &mut server,
        "grit_world_step",
        &format!("{{\"world\":{world},\"frames\":65536,\"budget_ms\":{budget}}}"),
    ));
    let took = began.elapsed();
    eprintln!("{} of 65536 frames in {took:?}", got.int_at("ran"));
    assert!(
        took < Duration::from_millis(budget + 1000),
        "{took:?}: {}",
        written(&got)
    );
    assert_eq!(got.get("stopped").and_then(Json::text), Some("budget"));
    let ran = got.int_at("ran");
    assert!(ran > 0 && ran < 65536, "{ran}");
    assert_eq!(got.int_at("frame"), ran);
    assert_eq!(got.int_at("frames"), 65536);
    let mut left = ran;
    while left > 0 {
        let step = left.min(65536);
        answer(&call(
            &mut server,
            "grit_world_step",
            &format!("{{\"world\":{fork},\"frames\":{step},\"budget_ms\":600000}}"),
        ));
        left -= step;
    }
    let one = answer(&call(
        &mut server,
        "grit_world_read",
        &format!(
            "{{\"world\":{world},\"from\":[{},{},{}],\"to\":[{},{},{}]}}",
            side / 2 - 8,
            side - 16,
            side / 2 - 8,
            side / 2 + 7,
            side - 1,
            side / 2 + 7
        ),
    ));
    let other = answer(&call(
        &mut server,
        "grit_world_read",
        &format!(
            "{{\"world\":{fork},\"from\":[{},{},{}],\"to\":[{},{},{}]}}",
            side / 2 - 8,
            side - 16,
            side / 2 - 8,
            side / 2 + 7,
            side - 1,
            side / 2 + 7
        ),
    ));
    assert_eq!(numbers(&one, "cells"), numbers(&other, "cells"));
    assert_eq!(one.int_at("frame"), other.int_at("frame"));
    let submissions = described(&mut server).get("submissions").cloned().unwrap();
    let longest = submissions.int_at("longest_ms");
    assert!(
        longest <= 250 || submissions.int_at("longest_frames") == 1,
        "{}",
        written(&submissions)
    );
}

#[test]
fn ping_and_describe_answer_at_once_while_a_step_runs_and_cancel_stops_it_between_batches() {
    let Some(host) = device("cancel") else {
        return;
    };
    let side = budgeted_side(&host).min(256);
    drop(host);
    let mut run = Run::new(&Options::default());
    let created = answer(&run.ask(
        "grit_world_create",
        &format!(
            "{{\"wide\":{side},\"deep\":{side},\"tall\":{side},\"rule\":\"sand\",\"sparse\":true}}"
        ),
    ));
    let world = created.int_at("world");
    let mut seed = 0xCA9C_u64;
    let cells: Vec<([u32; 3], u32)> = scattered(&mut seed, 32, 4096, &SAND_WORDS)
        .into_iter()
        .map(|(at, word)| ([at[0], at[1], at[2]], word))
        .collect();
    answer(&run.ask(
        "grit_world_write",
        &format!("{{\"world\":{world},\"cells\":{}}}", writing(&cells)),
    ));
    let fork =
        answer(&run.ask("grit_world_fork", &format!("{{\"world\":{world}}}"))).int_at("world");
    let other = answer(&run.ask(
        "grit_world_create",
        "{\"wide\":8,\"deep\":8,\"tall\":8,\"rule\":\"sand\"}",
    ))
    .int_at("world");
    let before = run.batches();
    let long = run.send(
        "grit_world_step",
        &format!("{{\"world\":{world},\"frames\":65536,\"budget_ms\":600000}}"),
    );
    let queued = run.send(
        "grit_world_step",
        &format!("{{\"world\":{other},\"frames\":5}}"),
    );
    let starved = run.send(
        "grit_world_step",
        &format!("{{\"world\":{other},\"frames\":5,\"budget_ms\":1}}"),
    );
    run.cancel(queued);
    let started = Instant::now();
    while run.batches() == before {
        assert!(started.elapsed() < WAIT, "the step never began");
    }
    let asked = Instant::now();
    run.exec
        .line("{\"jsonrpc\":\"2.0\",\"id\":\"p\",\"method\":\"ping\"}");
    let pong = loop {
        let got = run.answers.recv_timeout(WAIT).unwrap();
        if got.get("id") == Some(&Json::Text("p".to_owned())) {
            break got;
        }
        let id = got.get("id").and_then(Json::int).unwrap();
        assert_ne!(id, long, "the step answered before the ping");
        run.early.insert(id, got);
    };
    let ping_took = asked.elapsed();
    assert_eq!(
        written(&pong),
        "{\"jsonrpc\":\"2.0\",\"id\":\"p\",\"result\":{}}"
    );
    assert!(ping_took < Duration::from_millis(500), "{ping_took:?}");
    let described = answer(&run.ask("grit_describe", "{}"));
    assert!(described.get("device").unwrap().get("name").is_some());
    assert!(!run.early.contains_key(&long));
    let cancelled_at = Instant::now();
    run.cancel(long);
    let stopped = answer(&run.wait(long));
    let late = cancelled_at.elapsed();
    eprintln!("cancel answered in {late:?}, ping in {ping_took:?}");
    assert!(late < Duration::from_millis(1000), "{late:?}");
    assert_eq!(
        stopped.get("stopped").and_then(Json::text),
        Some("cancelled")
    );
    let ran = stopped.int_at("ran");
    assert!(ran > 0 && ran < 65536, "{ran}");
    assert_eq!(stopped.int_at("frame"), ran);
    let told = refused(&run.wait(queued));
    assert_eq!(told.code, "cancelled");
    let told = refused(&run.wait(starved));
    assert_eq!(
        (told.code.as_str(), told.field.as_str()),
        ("budget", "$.budget_ms")
    );
    let after = answer(&run.ask(
        "grit_world_read",
        &format!("{{\"world\":{other},\"from\":[0,0,0],\"to\":[0,0,0]}}"),
    ));
    assert_eq!(after.int_at("frame"), 0);
    let mut left = ran;
    while left > 0 {
        let step = left.min(65536);
        answer(&run.ask(
            "grit_world_step",
            &format!("{{\"world\":{fork},\"frames\":{step},\"budget_ms\":600000}}"),
        ));
        left -= step;
    }
    let box_of = |world: i64| {
        format!(
            "{{\"world\":{world},\"from\":[0,{},0],\"to\":[15,{},15]}}",
            side - 16,
            side - 1
        )
    };
    let first = answer(&run.ask("grit_world_read", &box_of(world)));
    let second = answer(&run.ask("grit_world_read", &box_of(fork)));
    assert_eq!(numbers(&first, "cells"), numbers(&second, "cells"));
    assert_eq!(first.int_at("frame"), second.int_at("frame"));
    run.exec.finish();
}

#[cfg(feature = "faults")]
fn armed(run: &Run, after_batch: Option<u32>, before_call: bool) {
    let faults = run.exec.faults();
    let mut held = faults.lock().unwrap();
    held.wedge_after_batch = after_batch;
    held.wedge_before_call = before_call;
}

#[cfg(feature = "faults")]
#[test]
fn after_a_lost_device_every_world_is_back_at_its_frame() {
    let Some(host) = device("lost device") else {
        return;
    };
    drop(host);
    let scratch = Scratch::new("lost");
    let mut run = Run::new(&Options {
        dir: Some(scratch.dir()),
        memory: None,
    });
    let mut seed = 0x1057_u64;
    let mut models = Vec::new();
    for (rule, params, sparse, frames) in [
        ("sand", Vec::new(), false, 70),
        ("heat", heat::TUNED.to_vec(), true, 50),
        ("sand", Vec::new(), true, 30),
    ] {
        let mut model = Model::new(rule, &params, 16);
        let world = answer(&run.ask("grit_world_create", &model.create(sparse))).int_at("world");
        let cells = if rule == "heat" {
            let words = heat_words(&mut seed, 500);
            scattered(&mut seed, 16, 500, &[0])
                .into_iter()
                .zip(words)
                .map(|((at, _), word)| (at, word))
                .collect()
        } else {
            scattered(&mut seed, 16, 500, &SAND_WORDS)
        };
        answer(&run.ask(
            "grit_world_write",
            &format!("{{\"world\":{world},\"cells\":{}}}", writing(&cells)),
        ));
        model.write(&cells);
        answer(&run.ask(
            "grit_world_step",
            &format!("{{\"world\":{world},\"frames\":{frames}}}"),
        ));
        model.step(frames);
        models.push((world, model));
    }
    let first = models[0].0;
    let mut writes = 0;
    while !entries(&scratch.dir().join("kept"))
        .iter()
        .any(|one| one.starts_with("snap-"))
    {
        assert!(writes < 64, "no snapshot was taken after {writes} writes");
        let cells = scattered(&mut seed, 16, 4096, &SAND_WORDS);
        answer(&run.ask(
            "grit_world_write",
            &format!("{{\"world\":{first},\"cells\":{}}}", writing(&cells)),
        ));
        models[0].1.write(&cells);
        writes += 1;
    }
    eprintln!("a snapshot after {writes} writes of 4096 cells");
    answer(&run.ask(
        "grit_world_step",
        &format!("{{\"world\":{first},\"frames\":4}}"),
    ));
    models[0].1.step(4);
    let forked = answer(&run.ask("grit_world_fork", &format!("{{\"world\":{}}}", models[0].0)));
    let fork = forked.int_at("world");
    let mut fork_model = Model::new("sand", &[], 16);
    fork_model.cells = models[0].1.cells.clone();
    fork_model.frame = models[0].1.frame;
    answer(&run.ask(
        "grit_world_step",
        &format!("{{\"world\":{fork},\"frames\":11}}"),
    ));
    fork_model.step(11);
    models.push((fork, fork_model));
    armed(&run, Some(1), false);
    let lost = run.ask(
        "grit_world_step",
        &format!("{{\"world\":{first},\"frames\":130}}"),
    );
    let told = refused(&lost);
    assert_eq!(told.code, "device_lost", "{}", told.message);
    let ran: u32 = told
        .message
        .split(" of the 130 frames ran")
        .next()
        .and_then(|head| head.rsplit(' ').next())
        .and_then(|count| count.parse().ok())
        .unwrap_or_else(|| panic!("{}", told.message));
    assert!(ran > 0 && ran < 130, "{}", told.message);
    models[0].1.step(ran);
    assert!(
        told.message
            .contains(&format!("frame {}", models[0].1.frame)),
        "{}",
        told.message
    );
    let restored = told
        .all
        .get("restored")
        .and_then(Json::list)
        .unwrap()
        .to_vec();
    assert_eq!(restored.len(), models.len(), "{}", written(&told.all));
    for note in &restored {
        let world = note.int_at("world");
        let (_, model) = models.iter().find(|(one, _)| *one == world).unwrap();
        assert_eq!(note.int_at("frame"), i64::from(model.frame));
        assert!(note.get("failed").is_none(), "{}", written(note));
    }
    for (world, model) in &models {
        let got = answer(&run.ask("grit_world_read", &model.read(*world)));
        assert_eq!(got.int_at("frame"), i64::from(model.frame), "world {world}");
        assert_eq!(numbers(&got, "cells"), model.cells, "world {world}");
        assert!(
            got.get("restored").is_some(),
            "world {world}: {}",
            written(&got)
        );
        let again = answer(&run.ask("grit_world_read", &model.read(*world)));
        assert!(again.get("restored").is_none());
    }
    armed(&run, None, true);
    let cells = scattered(&mut seed, 16, 50, &SAND_WORDS);
    let lost = run.ask(
        "grit_world_write",
        &format!("{{\"world\":{first},\"cells\":{}}}", writing(&cells)),
    );
    let told = refused(&lost);
    assert_eq!(told.code, "device_lost", "{}", told.message);
    assert_eq!(
        told.all.get("restored").and_then(Json::list).unwrap().len(),
        models.len()
    );
    for (world, model) in &mut models {
        let frames = 17;
        let got = answer(&run.ask(
            "grit_world_step",
            &format!("{{\"world\":{world},\"frames\":{frames}}}"),
        ));
        model.step(frames);
        assert_eq!(got.int_at("frame"), i64::from(model.frame));
        let got = answer(&run.ask("grit_world_read", &model.read(*world)));
        assert_eq!(numbers(&got, "cells"), model.cells, "world {world}");
    }
    let described = answer(&run.ask("grit_describe", "{}"));
    assert_eq!(described.get("worlds").unwrap().int_at("restorations"), 2);
    run.exec.finish();
}

#[cfg(feature = "faults")]
#[test]
fn a_wait_that_fails_after_its_submit_reports_the_frame_the_world_is_rebuilt_at() {
    let Some(host) = device("wait fails") else {
        return;
    };
    {
        let mut server = Server::new(Some(&host));
        let mut model = Model::new("sand", &[], 8);
        let world = create(&mut server, &model.create(false));
        let mut seed = 0x3A17_u64;
        let cells = scattered(&mut seed, 8, 60, &SAND_WORDS);
        answer(&call(
            &mut server,
            "grit_world_write",
            &format!("{{\"world\":{world},\"cells\":{}}}", writing(&cells)),
        ));
        model.write(&cells);
        server.faults().lock().unwrap().wedge_after_batch = Some(0);
        let told = refused(&call(
            &mut server,
            "grit_world_step",
            &format!("{{\"world\":{world},\"frames\":130}}"),
        ));
        assert_eq!(told.code, "device_lost");
        assert!(
            told.message
                .contains("0 of the 130 frames ran before it, and the world is at frame 0"),
            "{}",
            told.message
        );
        let told = refused(&call(&mut server, "grit_world_read", &model.read(world)));
        assert_eq!(told.code, "device_lost");
        assert!(server.lost());
        let listed = answer(&call(&mut server, "grit_world_list", "{}"));
        let item = &listed.get("items").and_then(Json::list).unwrap()[0];
        assert_eq!(item.get("kind").and_then(Json::text), Some("lost"));
        assert_eq!(item.int_at("frame"), 0);
        answer(&call(
            &mut server,
            "grit_world_close",
            &format!("{{\"world\":{world}}}"),
        ));
        assert_eq!(
            described(&mut server)
                .get("memory")
                .unwrap()
                .int_at("held_bytes"),
            0
        );
        let told = refused(&call(&mut server, "grit_world_read", &model.read(world)));
        assert_eq!(told.code, "closed");
    }
    drop(host);
    let mut run = Run::new(&Options::default());
    let mut model = Model::new("sand", &[], 8);
    let world = answer(&run.ask("grit_world_create", &model.create(false))).int_at("world");
    let mut seed = 0x3A17_u64;
    let cells = scattered(&mut seed, 8, 60, &SAND_WORDS);
    answer(&run.ask(
        "grit_world_write",
        &format!("{{\"world\":{world},\"cells\":{}}}", writing(&cells)),
    ));
    model.write(&cells);
    armed(&run, Some(0), false);
    let told = refused(&run.ask(
        "grit_world_step",
        &format!("{{\"world\":{world},\"frames\":130}}"),
    ));
    assert_eq!(told.code, "device_lost");
    let got = answer(&run.ask("grit_world_read", &model.read(world)));
    assert_eq!(got.int_at("frame"), 0);
    assert_eq!(numbers(&got, "cells"), model.cells);
    let got = answer(&run.ask(
        "grit_world_step",
        &format!("{{\"world\":{world},\"frames\":130}}"),
    ));
    model.step(130);
    assert_eq!(got.int_at("frame"), 130);
    let got = answer(&run.ask("grit_world_read", &model.read(world)));
    assert_eq!(numbers(&got, "cells"), model.cells);
    run.exec.finish();
}

struct Process {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    id: i64,
}

impl Process {
    fn start(dir: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_grit-mcp"))
            .arg("--dir")
            .arg(dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            input,
            output,
            id: 0,
        }
    }

    fn ask(&mut self, name: &str, arguments: &str) -> Json {
        self.id += 1;
        writeln!(self.input, "{}", line(self.id, name, arguments)).unwrap();
        self.input.flush().unwrap();
        loop {
            let mut held = String::new();
            assert!(
                self.output.read_line(&mut held).unwrap() > 0,
                "the server ended"
            );
            let got = read(held.trim_end()).unwrap();
            if got.get("id").and_then(Json::int) == Some(self.id) {
                return got;
            }
        }
    }

    fn kill(mut self) {
        self.child.kill().unwrap();
        self.child.wait().unwrap();
    }
}

#[test]
fn a_world_survives_the_end_of_its_server_and_opens_at_its_frame() {
    let Some(host) = device("restart") else {
        return;
    };
    drop(host);
    let scratch = Scratch::new("restart");
    let mut seed = 0x8E57_u64;
    let mut model = Model::new("sand", &[], 16);
    let mut sparse_model = Model::new("sand", &[], 16);
    let mut first = Process::start(&scratch.dir());
    let world = answer(&first.ask("grit_world_create", &model.create(false))).int_at("world");
    let sparse =
        answer(&first.ask("grit_world_create", &sparse_model.create(true))).int_at("world");
    let closed = answer(&first.ask(
        "grit_world_create",
        "{\"wide\":4,\"deep\":4,\"tall\":4,\"rule\":\"sand\"}",
    ))
    .int_at("world");
    answer(&first.ask("grit_world_close", &format!("{{\"world\":{closed}}}")));
    for round in 0..3 {
        let many = if round == 1 { 4096 } else { 300 };
        let cells = scattered(&mut seed, 16, many, &SAND_WORDS);
        for (one, held) in [(world, &mut model), (sparse, &mut sparse_model)] {
            answer(&first.ask(
                "grit_world_write",
                &format!("{{\"world\":{one},\"cells\":{}}}", writing(&cells)),
            ));
            held.write(&cells);
            answer(&first.ask(
                "grit_world_step",
                &format!("{{\"world\":{one},\"frames\":23}}"),
            ));
            held.step(23);
        }
    }
    first.kill();
    let mut second = Process::start(&scratch.dir());
    let listed = answer(&second.ask("grit_world_list", "{}"));
    let items = listed.get("items").and_then(Json::list).unwrap();
    let kept: Vec<(i64, i64)> = items
        .iter()
        .filter(|one| one.get("kind").and_then(Json::text) == Some("kept"))
        .map(|one| (one.int_at("world"), one.int_at("frame")))
        .collect();
    assert_eq!(kept, [(world, 69), (sparse, 69)], "{}", written(&listed));
    let told = refused(&second.ask("grit_world_step", &format!("{{\"world\":{world}}}")));
    assert_eq!(told.code, "no_world");
    let told = refused(&second.ask("grit_world_step", &format!("{{\"world\":{closed}}}")));
    assert_eq!(told.code, "closed");
    for (one, held) in [(world, &mut model), (sparse, &mut sparse_model)] {
        let opened =
            answer(&second.ask("grit_world_open", &format!("{{\"name\":\"world-{one}\"}}")));
        assert_eq!(opened.int_at("world"), one);
        assert_eq!(opened.int_at("frame"), 69);
        let got = answer(&second.ask("grit_world_read", &held.read(one)));
        assert_eq!(numbers(&got, "cells"), held.cells, "world {one}");
        answer(&second.ask(
            "grit_world_step",
            &format!("{{\"world\":{one},\"frames\":5}}"),
        ));
        held.step(5);
        let got = answer(&second.ask("grit_world_read", &held.read(one)));
        assert_eq!(numbers(&got, "cells"), held.cells, "world {one}");
        let told =
            refused(&second.ask("grit_world_open", &format!("{{\"name\":\"world-{one}\"}}")));
        assert_eq!(told.code, "exists");
    }
    let fresh = answer(&second.ask(
        "grit_world_create",
        "{\"wide\":4,\"deep\":4,\"tall\":4,\"rule\":\"sand\"}",
    ))
    .int_at("world");
    assert!(fresh > closed, "{fresh}");
    second.kill();
    let mut third = Process::start(&scratch.dir());
    let opened = answer(&third.ask(
        "grit_world_open",
        &format!("{{\"name\":\"world-{world}\"}}"),
    ));
    assert_eq!(opened.int_at("frame"), 74);
    let got = answer(&third.ask("grit_world_read", &model.read(world)));
    assert_eq!(numbers(&got, "cells"), model.cells);
    third.kill();
}
