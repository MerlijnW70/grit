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
fn a_tally_word_past_what_json_integers_hold_is_reported_as_its_decimal_string() {
    use grit::serve3::tools::exactly;
    for (value, shown) in [
        (0_u128, "0"),
        (u128::from(u32::MAX), "4294967295"),
        (1 << 32, "4294967296"),
        ((1 << 63) - 1, "9223372036854775807"),
        (1 << 63, "\"9223372036854775808\""),
        ((1 << 96) - 1, "\"79228162514264337593543950335\""),
    ] {
        let got = exactly(value);
        assert_eq!(written(&got), shown);
        let back = match &got {
            Json::Int(one) => u128::try_from(*one).unwrap(),
            Json::Text(one) => one.parse::<u128>().unwrap(),
            other => panic!("{}", written(other)),
        };
        assert_eq!(back, value);
    }
    let mut server = Server::new(None);
    let limits = described(&mut server).get("limits").cloned().unwrap();
    assert!(limits.get("tally_words").and_then(Json::text).is_some());
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
        keep.append(5, entry, Duration::ZERO).unwrap();
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
        let snaps = |dir: &Path| {
            entries(&dir.join("kept"))
                .into_iter()
                .filter(|one| one.starts_with("snap-"))
                .collect::<Vec<String>>()
        };
        let before = snaps(&scratch.dir());
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
        assert_eq!(snaps(&scratch.dir()), before, "{rule}");
    }
    let kept = entries(&scratch.dir().join("kept"));
    let snapshots = kept.iter().filter(|one| one.starts_with("snap-")).count();
    assert!(snapshots >= 2, "{kept:?}");
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
        &format!(
            "{{\"world\":{world},\"cells\":{},\"budget_ms\":600000}}",
            writing(&cells)
        ),
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
        written_whole(&mut run, world, &cells);
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
            let wrote = answer(&first.ask(
                "grit_world_write",
                &format!(
                    "{{\"world\":{one},\"cells\":{},\"budget_ms\":600000}}",
                    writing(&cells)
                ),
            ));
            assert_eq!(wrote.int_at("written"), many as i64, "{}", written(&wrote));
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

impl Run {
    fn within(&mut self, id: i64, limit: Duration) -> Option<Json> {
        if let Some(got) = self.early.remove(&id) {
            return Some(got);
        }
        let began = Instant::now();
        loop {
            let left = limit.checked_sub(began.elapsed())?;
            let got = self.answers.recv_timeout(left).ok()?;
            let came = got.get("id").and_then(Json::int);
            if came == Some(id) {
                return Some(got);
            }
            if let Some(came) = came {
                self.early.insert(came, got);
            }
        }
    }
}

fn kept_sand(dir: &Path, number: u32, steps: &[u32]) {
    let keep = Keep::at(dir).unwrap();
    let mut ledger = Ledger::created(Made {
        rule: "sand".to_owned(),
        params: Vec::new(),
        size: [8, 8, 8],
        sparse: false,
        slots: None,
    });
    for frames in steps {
        ledger.push(Entry::Step(*frames), Duration::ZERO);
    }
    keep.begin(number, &ledger).unwrap();
}

fn kinds_listed(listed: &Json) -> Vec<(String, i64)> {
    listed
        .get("items")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .map(|one| {
            (
                one.get("kind").and_then(Json::text).unwrap().to_owned(),
                one.get("world").and_then(Json::int).unwrap_or(0),
            )
        })
        .collect()
}

#[test]
fn an_open_of_a_kept_world_stops_on_its_budget_and_its_cancel_and_opens_nothing() {
    let Some(host) = device("open within its budget") else {
        return;
    };
    drop(host);
    let scratch = Scratch::new("open-budget");
    kept_sand(&scratch.dir(), 1, &[3, 4_000_000_000]);
    let mut run = Run::new(&Options {
        dir: Some(scratch.dir()),
        memory: None,
    });
    let built = Instant::now();
    let probe = answer(&run.ask(
        "grit_world_create",
        "{\"wide\":8,\"deep\":8,\"tall\":8,\"rule\":\"sand\"}",
    ))
    .int_at("world");
    let build = built.elapsed();
    answer(&run.ask("grit_world_close", &format!("{{\"world\":{probe}}}")));
    eprintln!("building the world itself takes {build:?}, one step no budget can cut");
    let budget = 300 + 2 * build.as_millis() as u64;
    let began = Instant::now();
    let id = run.send(
        "grit_world_open",
        &format!("{{\"name\":\"world-1\",\"budget_ms\":{budget}}}"),
    );
    let Some(got) = run.within(id, Duration::from_millis(budget + 1000) + build) else {
        std::mem::forget(run);
        panic!(
            "an open with budget_ms {budget} had not answered after its budget, 1 s and {build:?}"
        );
    };
    eprintln!("open stopped by its budget after {:?}", began.elapsed());
    let told = refused(&got);
    assert_eq!(
        (told.code.as_str(), told.field.as_str()),
        ("budget", "$.budget_ms"),
        "{}",
        told.message
    );
    assert!(
        told.message.contains("of its 2 entries"),
        "{}",
        told.message
    );
    assert!(
        told.message.contains("nothing was opened"),
        "{}",
        told.message
    );
    let id = run.send(
        "grit_world_open",
        "{\"name\":\"world-1\",\"budget_ms\":600000}",
    );
    std::thread::sleep(Duration::from_millis(400));
    let asked = Instant::now();
    run.exec
        .line("{\"jsonrpc\":\"2.0\",\"id\":\"p\",\"method\":\"ping\"}");
    let pong = run.answers.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(pong.get("id"), Some(&Json::Text("p".to_owned())));
    assert!(asked.elapsed() < Duration::from_millis(500));
    let cancelled = Instant::now();
    run.cancel(id);
    let Some(got) = run.within(id, Duration::from_millis(1000) + build) else {
        std::mem::forget(run);
        panic!("a cancelled open had not answered after 1 s and {build:?}");
    };
    eprintln!("open stopped by its cancel after {:?}", cancelled.elapsed());
    let told = refused(&got);
    assert_eq!(told.code, "cancelled", "{}", told.message);
    assert!(
        told.message.contains("nothing was opened"),
        "{}",
        told.message
    );
    let listed = answer(&run.ask("grit_world_list", "{}"));
    assert_eq!(kinds_listed(&listed), [("kept".to_owned(), 1)]);
    let memory = answer(&run.ask("grit_describe", "{}"));
    assert_eq!(memory.get("memory").unwrap().int_at("held_bytes"), 0);
    let told = refused(&run.ask("grit_world_step", "{\"world\":1}"));
    assert_eq!(told.code, "no_world");
    run.exec.finish();
}

#[test]
fn a_world_stepped_by_budget_stopped_calls_keeps_its_replay_bounded() {
    let Some(host) = device("bounded replay") else {
        return;
    };
    drop(host);
    let scratch = Scratch::new("bounded");
    let options = Options {
        dir: Some(scratch.dir()),
        memory: None,
    };
    let mut run = Run::new(&options);
    let world = answer(&run.ask(
        "grit_world_create",
        "{\"wide\":64,\"deep\":64,\"tall\":64,\"rule\":\"sand\"}",
    ))
    .int_at("world");
    let mut seed = 0xB0B0_u64;
    let cells = scattered(&mut seed, 64, 4096, &SAND_WORDS);
    answer(&run.ask(
        "grit_world_write",
        &format!("{{\"world\":{world},\"cells\":{}}}", writing(&cells)),
    ));
    let began = Instant::now();
    let mut frames = 0;
    for _ in 0..16 {
        let got = answer(&run.ask(
            "grit_world_step",
            &format!("{{\"world\":{world},\"frames\":65536,\"budget_ms\":300}}"),
        ));
        frames = got.int_at("frame");
    }
    let stepping = began.elapsed();
    eprintln!("16 budget-stopped calls reached frame {frames} in {stepping:?}");
    assert!(stepping > Duration::from_secs(4), "{stepping:?}");
    let kept = entries(&scratch.dir().join("kept"));
    assert!(
        kept.iter().any(|one| one.starts_with("snap-")),
        "no snapshot after {stepping:?} of stepping: {kept:?}"
    );
    run.exec.finish();
    let mut again = Run::new(&options);
    let began = Instant::now();
    let opened = answer(&again.ask(
        "grit_world_open",
        &format!("{{\"name\":\"world-{world}\"}}"),
    ));
    let took = began.elapsed();
    eprintln!(
        "reopened at frame {} replaying {} entries in {took:?}",
        opened.int_at("frame"),
        opened.int_at("replayed")
    );
    assert_eq!(opened.int_at("frame"), frames);
    assert!(took < Duration::from_millis(2000 + 250 + 1500), "{took:?}");
    again.exec.finish();
}

#[cfg(feature = "faults")]
#[test]
fn a_world_that_cannot_be_rebuilt_is_reported_once_and_leaves_the_others_alone() {
    let Some(host) = device("one broken world") else {
        return;
    };
    drop(host);
    let scratch = Scratch::new("broken");
    let mut run = Run::new(&Options {
        dir: Some(scratch.dir()),
        memory: None,
    });
    let first = answer(&run.ask(
        "grit_world_create",
        "{\"wide\":8,\"deep\":8,\"tall\":8,\"rule\":\"sand\"}",
    ))
    .int_at("world");
    answer(&run.ask(
        "grit_world_save",
        &format!("{{\"world\":{first},\"name\":\"s\"}}"),
    ));
    let second = answer(&run.ask("grit_world_open", "{\"name\":\"s\"}")).int_at("world");
    for one in entries(&scratch.dir().join("kept")) {
        if one.starts_with("snap-") {
            std::fs::remove_file(scratch.dir().join("kept").join(one)).unwrap();
        }
    }
    armed(&run, None, true);
    let told = refused(&run.ask(
        "grit_world_step",
        &format!("{{\"world\":{first},\"frames\":1}}"),
    ));
    assert_eq!(told.code, "device_lost");
    let restored = told
        .all
        .get("restored")
        .and_then(Json::list)
        .unwrap()
        .to_vec();
    let pending: Vec<i64> = restored
        .iter()
        .filter(|one| one.get("pending") == Some(&Json::Bool(true)))
        .map(|one| one.int_at("world"))
        .collect();
    assert_eq!(pending, [first, second]);
    let restorations = |run: &mut Run| {
        answer(&run.ask("grit_describe", "{}"))
            .get("worlds")
            .unwrap()
            .int_at("restorations")
    };
    let mut seen = vec![restorations(&mut run)];
    for _ in 0..4 {
        answer(&run.ask(
            "grit_world_read",
            &format!("{{\"world\":{first},\"from\":[0,0,0],\"to\":[0,0,0]}}"),
        ));
        seen.push(restorations(&mut run));
    }
    assert_eq!(seen, [1; 5]);
    let told = refused(&run.ask(
        "grit_world_read",
        &format!("{{\"world\":{second},\"from\":[0,0,0],\"to\":[0,0,0]}}"),
    ));
    assert_eq!(told.code, "device_lost");
    assert!(
        told.message.contains("could not be rebuilt"),
        "{}",
        told.message
    );
    let note = told.all.get("restored").unwrap();
    assert_eq!(note.int_at("world"), second);
    assert!(note.get("failed").is_some(), "{}", written(note));
    let again = refused(&run.ask(
        "grit_world_read",
        &format!("{{\"world\":{second},\"from\":[0,0,0],\"to\":[0,0,0]}}"),
    ));
    assert!(again.all.get("restored").is_none());
    assert_eq!(restorations(&mut run), 1);
    let listed = answer(&run.ask("grit_world_list", "{}"));
    let kinds = kinds_listed(&listed);
    assert!(kinds.contains(&("broken".to_owned(), second)), "{kinds:?}");
    assert!(kinds.contains(&("open".to_owned(), first)), "{kinds:?}");
    answer(&run.ask("grit_world_close", &format!("{{\"world\":{second}}}")));
    let stepped = answer(&run.ask(
        "grit_world_step",
        &format!("{{\"world\":{first},\"frames\":2}}"),
    ));
    assert_eq!(stepped.int_at("frame"), 2);
    assert_eq!(restorations(&mut run), 1);
    run.exec.finish();
}

#[test]
fn a_second_server_on_one_directory_is_refused_by_name() {
    let scratch = Scratch::new("lock");
    let options = Options {
        dir: Some(scratch.dir()),
        memory: None,
    };
    let first = Server::with(None, &options).unwrap();
    let Err(why) = Server::with(None, &options) else {
        panic!("a second server opened the same directory");
    };
    let said = why.to_string();
    assert!(
        said.contains("another grit server holds the directory"),
        "{said}"
    );
    let Err(why) = Exec::spawn(Box::new(|| Ok(None)), &options) else {
        panic!("a second executor opened the same directory");
    };
    assert!(why.to_string().contains("another grit server"), "{why}");
    let output = Command::new(env!("CARGO_BIN_EXE_grit-mcp"))
        .arg("--dir")
        .arg(scratch.dir())
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!output.status.success());
    let said = String::from_utf8_lossy(&output.stderr);
    assert!(
        said.contains("another grit server holds the directory"),
        "{said}"
    );
    drop(first);
    let again = Server::with(None, &options).unwrap();
    drop(again);
    let nested = Options {
        dir: Some(scratch.dir().join(".").join("kept").join("..")),
        memory: None,
    };
    let held = Server::with(None, &options).unwrap();
    assert!(Server::with(None, &nested).is_err());
    drop(held);
}

#[test]
fn a_kept_number_out_of_range_or_past_next_is_ignored_by_name() {
    let Some(host) = device("kept numbers") else {
        return;
    };
    let scratch = Scratch::new("numbers");
    kept_sand(&scratch.dir(), u32::MAX, &[]);
    kept_sand(&scratch.dir(), (1 << 31) + 1, &[]);
    kept_sand(&scratch.dir(), 3, &[2]);
    std::fs::write(scratch.dir().join("kept").join("next"), "4294967295").unwrap();
    let options = Options {
        dir: Some(scratch.dir()),
        memory: None,
    };
    {
        let mut server = Server::with(Some(&host), &options).unwrap();
        let worlds = described(&mut server).get("worlds").cloned().unwrap();
        let ignored: Vec<String> = worlds
            .get("ignored")
            .and_then(Json::list)
            .unwrap()
            .iter()
            .map(|one| one.text().unwrap().to_owned())
            .collect();
        assert_eq!(worlds.int_at("ignored_total"), 3, "{ignored:?}");
        for named in [
            "kept/world-4294967295.journal",
            "kept/world-2147483649.journal",
            "kept/next",
        ] {
            assert!(
                ignored.iter().any(|one| one.starts_with(named)),
                "{named}: {ignored:?}"
            );
        }
        let listed = answer(&call(&mut server, "grit_world_list", "{}"));
        assert_eq!(kinds_listed(&listed), [("kept".to_owned(), 3)]);
        let made = create(
            &mut server,
            "{\"wide\":4,\"deep\":4,\"tall\":4,\"rule\":\"sand\"}",
        );
        assert_eq!(made, 4);
    }
    kept_sand(&scratch.dir(), 12, &[]);
    {
        let mut server = Server::with(Some(&host), &options).unwrap();
        let worlds = described(&mut server).get("worlds").cloned().unwrap();
        let ignored = written(worlds.get("ignored").unwrap());
        assert!(
            ignored.contains("world-12.journal: world 12 is past the next number 5"),
            "{ignored}"
        );
        let made = create(
            &mut server,
            "{\"wide\":4,\"deep\":4,\"tall\":4,\"rule\":\"sand\"}",
        );
        assert_eq!(made, 5);
        let opened = answer(&call(
            &mut server,
            "grit_world_open",
            "{\"name\":\"world-3\"}",
        ));
        assert_eq!(opened.int_at("frame"), 2);
    }
}

fn read_meter(server: &mut Server<'_>) -> (i64, i64) {
    let told = described(server).get("submissions").cloned().unwrap();
    (told.int_at("read_waits"), told.int_at("words_read"))
}

#[test]
fn reading_one_cell_costs_the_same_device_waits_and_words_whatever_the_world_size() {
    let Some(host) = device("one-cell read cost") else {
        return;
    };
    let large = budgeted_side(&host);
    let mut server = Server::new(Some(&host));
    let mut costs = Vec::new();
    for (sparse, side) in [(false, 16), (false, 128), (true, 32), (true, large)] {
        let world = create(
            &mut server,
            &format!(
                "{{\"wide\":{side},\"deep\":{side},\"tall\":{side},\"rule\":\"sand\",\"sparse\":{sparse}}}"
            ),
        );
        let last = side - 1;
        answer(&call(
            &mut server,
            "grit_world_write",
            &format!("{{\"world\":{world},\"cells\":[[1,1,1,7],[{last},{last},{last},9]]}}"),
        ));
        for (at, want) in [([1, 1, 1], 7), ([last, last, last], 9), ([2, 3, 4], 0)] {
            let before = read_meter(&mut server);
            let got = answer(&call(
                &mut server,
                "grit_world_read",
                &format!(
                    "{{\"world\":{world},\"from\":[{},{},{}],\"to\":[{},{},{}]}}",
                    at[0], at[1], at[2], at[0], at[1], at[2]
                ),
            ));
            let after = read_meter(&mut server);
            assert_eq!(numbers(&got, "cells"), [want]);
            let cost = (after.0 - before.0, after.1 - before.1);
            eprintln!(
                "sparse {sparse} side {side} cell {at:?}: {} waits, {} words",
                cost.0, cost.1
            );
            assert!(cost.0 >= 1 && cost.0 <= 2, "{cost:?}");
            assert!(
                cost.1 >= 1,
                "the read did not go through the targeted reader: {cost:?}"
            );
            costs.push((sparse, side, cost));
        }
        if side <= 256 {
            let before = read_meter(&mut server);
            answer(&call(
                &mut server,
                "grit_world_slice",
                &format!(
                    "{{\"world\":{world},\"axis\":\"z\",\"at\":{last},\"cursor\":{}}}",
                    last.saturating_sub(1)
                ),
            ));
            let after = read_meter(&mut server);
            assert!(
                after.0 - before.0 <= 2,
                "slice: {} waits",
                after.0 - before.0
            );
        }
        answer(&call(
            &mut server,
            "grit_world_close",
            &format!("{{\"world\":{world}}}"),
        ));
    }
    for sparse in [false, true] {
        let mut sides: Vec<u32> = costs
            .iter()
            .filter(|(kind, _, _)| *kind == sparse)
            .map(|(_, side, _)| *side)
            .collect();
        sides.dedup();
        let most = |side: u32| {
            costs
                .iter()
                .filter(|(kind, at, _)| *kind == sparse && *at == side)
                .map(|(_, _, cost)| cost.1)
                .max()
                .unwrap()
        };
        let (small, large) = (sides[0], sides[sides.len() - 1]);
        assert!(
            most(large) <= most(small) && most(small) <= 4096,
            "sparse {sparse}: one cell read {} words at side {small} and {} at side {large}",
            most(small),
            most(large)
        );
    }
}

#[test]
fn a_save_an_open_a_fork_and_a_create_too_slow_for_their_budget_are_refused_before_they_start() {
    let Some(host) = device("work estimated before it starts") else {
        return;
    };
    let scratch = Scratch::new("estimates");
    let options = Options {
        dir: Some(scratch.dir()),
        memory: None,
    };
    let mut server = Server::with(Some(&host), &options).unwrap();
    let made = "{\"wide\":128,\"deep\":128,\"tall\":128,\"rule\":\"sand\"";
    let world = create(&mut server, &format!("{made}}}"));
    answer(&call(
        &mut server,
        "grit_world_save",
        &format!("{{\"world\":{world},\"name\":\"first\"}}"),
    ));
    let before = entries(&scratch.dir());
    let listed = answer(&call(&mut server, "grit_world_list", "{}")).int_at("total");
    for (tool, arguments, what) in [
        (
            "grit_world_save",
            format!("{{\"world\":{world},\"name\":\"second\",\"budget_ms\":1}}"),
            "writing world",
        ),
        (
            "grit_world_open",
            "{\"name\":\"first\",\"budget_ms\":1}".to_owned(),
            "",
        ),
        (
            "grit_world_fork",
            format!("{{\"world\":{world},\"budget_ms\":1}}"),
            "copying world",
        ),
        (
            "grit_world_create",
            format!("{made},\"budget_ms\":1}}"),
            "building the world",
        ),
    ] {
        let began = Instant::now();
        let told = refused(&call(&mut server, tool, &arguments));
        let took = began.elapsed();
        assert_eq!(
            (told.code.as_str(), told.field.as_str()),
            ("budget", "$.budget_ms"),
            "{tool}: {}",
            told.message
        );
        assert!(told.message.contains(what), "{tool}: {}", told.message);
        assert!(told.message.contains("nothing ran"), "{}", told.message);
        assert!(
            told.all.int_at("limit") > 1,
            "{tool}: {}",
            written(&told.all)
        );
        assert!(took < Duration::from_millis(50), "{tool}: {took:?}");
    }
    assert_eq!(entries(&scratch.dir()), before);
    let after = answer(&call(&mut server, "grit_world_list", "{}")).int_at("total");
    assert_eq!(after, listed);
    let rates = described(&mut server)
        .get("submissions")
        .and_then(|one| one.get("rates"))
        .cloned()
        .unwrap();
    for work in ["build", "save"] {
        assert!(
            rates.get(work).unwrap().int_at("observations") >= 1,
            "{work}: {}",
            written(&rates)
        );
    }
    let next = create(&mut server, &format!("{made}}}"));
    assert_eq!(next, world + 1);
}

fn dense_side(host: &Host) -> u32 {
    let sand = rules()
        .into_iter()
        .find(|one| one.name() == "sand")
        .unwrap();
    for side in [256, 128, 64] {
        let made = Made {
            rule: "sand".to_owned(),
            params: Vec::new(),
            size: [side, side, side],
            sparse: false,
            slots: None,
        };
        if let Ok((_, sizes)) = grit::serve3::shaped(host, sand.as_ref(), &made)
            && host.limits().admits(&sizes, None).is_ok()
        {
            return side;
        }
    }
    32
}

#[test]
fn a_kept_world_too_large_to_build_in_the_budget_is_refused_before_it_is_built() {
    let Some(host) = device("kept open estimated") else {
        return;
    };
    let side = dense_side(&host);
    drop(host);
    let scratch = Scratch::new("kept-estimate");
    let options = Options {
        dir: Some(scratch.dir()),
        memory: None,
    };
    let mut run = Run::new(&options);
    let world = answer(&run.ask(
        "grit_world_create",
        &format!("{{\"wide\":{side},\"deep\":{side},\"tall\":{side},\"rule\":\"sand\"}}"),
    ))
    .int_at("world");
    answer(&run.ask(
        "grit_world_step",
        &format!("{{\"world\":{world},\"frames\":3}}"),
    ));
    run.exec.finish();
    let mut again = Run::new(&options);
    let began = Instant::now();
    let told = refused(&again.ask(
        "grit_world_open",
        &format!("{{\"name\":\"world-{world}\",\"budget_ms\":20}}"),
    ));
    let took = began.elapsed();
    eprintln!(
        "a {side}^3 kept world refused in {took:?}: {}",
        told.message
    );
    assert_eq!(told.code, "budget", "{}", told.message);
    assert!(
        told.message.contains("rebuilding world"),
        "{}",
        told.message
    );
    assert!(took < Duration::from_millis(20 + 100), "{took:?}");
    let opened = answer(&again.ask(
        "grit_world_open",
        &format!("{{\"name\":\"world-{world}\"}}"),
    ));
    assert_eq!(opened.int_at("frame"), 3);
    again.exec.finish();
}

#[cfg(feature = "faults")]
#[test]
fn after_a_lost_device_only_the_worlds_a_call_touches_are_rebuilt_inside_its_budget() {
    let Some(host) = device("lazy rebuild") else {
        return;
    };
    drop(host);
    let scratch = Scratch::new("lazy");
    let mut run = Run::new(&Options {
        dir: Some(scratch.dir()),
        memory: None,
    });
    let mut seed = 0x1A2B_u64;
    let mut models = Vec::new();
    for _ in 0..3 {
        let mut model = Model::new("sand", &[], 16);
        let world = answer(&run.ask("grit_world_create", &model.create(false))).int_at("world");
        let cells = scattered(&mut seed, 16, 250, &SAND_WORDS);
        written_whole(&mut run, world, &cells);
        model.write(&cells);
        answer(&run.ask(
            "grit_world_step",
            &format!("{{\"world\":{world},\"frames\":20}}"),
        ));
        model.step(20);
        models.push((world, model));
    }
    let worlds = |run: &mut Run| {
        answer(&run.ask("grit_describe", "{}"))
            .get("worlds")
            .cloned()
            .unwrap()
    };
    armed(&run, None, true);
    let first = models[0].0;
    let began = Instant::now();
    let told = refused(&run.ask(
        "grit_world_read",
        &format!("{{\"world\":{first},\"from\":[0,0,0],\"to\":[0,0,0],\"budget_ms\":1000}}"),
    ));
    let lost_took = began.elapsed();
    assert!(lost_took < Duration::from_millis(1000), "{lost_took:?}");
    assert_eq!(told.code, "device_lost", "{}", told.message);
    let restored = told
        .all
        .get("restored")
        .and_then(Json::list)
        .unwrap()
        .to_vec();
    assert_eq!(restored.len(), 3);
    for note in &restored {
        assert_eq!(
            note.get("pending"),
            Some(&Json::Bool(true)),
            "{}",
            written(note)
        );
    }
    let now = worlds(&mut run);
    assert_eq!((now.int_at("open"), now.int_at("lost")), (0, 3));
    let (second, model) = &models[1];
    let got = answer(&run.ask("grit_world_read", &model.read(second)));
    assert_eq!(numbers(&got, "cells"), model.cells);
    let note = got.get("restored").unwrap();
    assert_eq!(note.int_at("world"), *second);
    assert_eq!(note.int_at("frame"), 20);
    let now = worlds(&mut run);
    assert_eq!((now.int_at("open"), now.int_at("lost")), (1, 2));
    let third = models[2].0;
    let told = refused(&run.ask(
        "grit_world_read",
        &format!("{{\"world\":{third},\"from\":[0,0,0],\"to\":[0,0,0],\"budget_ms\":1}}"),
    ));
    assert_eq!(told.code, "budget", "{}", told.message);
    assert!(
        told.message.contains("rebuilding world"),
        "{}",
        told.message
    );
    let now = worlds(&mut run);
    assert_eq!((now.int_at("open"), now.int_at("lost")), (1, 2));
    for (world, model) in &models {
        let got = answer(&run.ask("grit_world_read", &model.read(world)));
        assert_eq!(numbers(&got, "cells"), model.cells, "world {world}");
    }
    let now = worlds(&mut run);
    assert_eq!(
        (
            now.int_at("open"),
            now.int_at("lost"),
            now.int_at("restorations")
        ),
        (3, 0, 1)
    );
    run.exec.finish();
}

#[test]
fn the_journal_bound_counts_replay_at_the_rate_replay_runs() {
    let scratch = Scratch::new("ratio");
    let keep = Keep::at(&scratch.dir()).unwrap();
    let mut ledger = Ledger::created(Made {
        rule: "sand".to_owned(),
        params: Vec::new(),
        size: [8, 8, 8],
        sparse: false,
        slots: None,
    });
    keep.begin(1, &ledger).unwrap();
    for spent in [400, 500, 600] {
        let entry = Entry::Step(10);
        let spent = Duration::from_millis(spent);
        ledger.push(entry.clone(), spent);
        keep.append(1, &entry, spent).unwrap();
    }
    assert_eq!(ledger.replay(), Duration::from_millis(1500));
    assert!(!ledger.over(Duration::ZERO, 0, 1.0));
    assert!(!ledger.over(Duration::ZERO, 0, 1.3));
    assert!(ledger.over(Duration::ZERO, 0, 1.4));
    assert!(ledger.over(Duration::ZERO, 0, 2.0));
    assert!(ledger.over(Duration::from_millis(600), 0, 1.0));
    let found = keep.earlier();
    assert_eq!(found[0].1.replay(), Duration::from_millis(1500));
    assert!(found[0].1.over(Duration::ZERO, 0, 2.0));
}

#[test]
fn the_last_world_number_is_refused_and_never_handed_out_twice() {
    let Some(host) = device("last world number") else {
        return;
    };
    let scratch = Scratch::new("last-number");
    let most = journal::NUMBER_MOST;
    Keep::at(&scratch.dir())
        .unwrap()
        .set_next(most - 1)
        .unwrap();
    let options = Options {
        dir: Some(scratch.dir()),
        memory: None,
    };
    let made = "{\"wide\":4,\"deep\":4,\"tall\":4,\"rule\":\"sand\"}";
    {
        let mut server = Server::with(Some(&host), &options).unwrap();
        let world = create(&mut server, made);
        assert_eq!(world, most - 1);
        answer(&call(
            &mut server,
            "grit_world_close",
            &format!("{{\"world\":{world}}}"),
        ));
        let told = refused(&call(&mut server, "grit_world_create", made));
        assert_eq!(told.code, "too_many", "{}", told.message);
        assert_eq!(told.all.int_at("limit"), i64::from(most - 1));
        assert!(
            told.message.contains(&format!("up to {} ", most - 1)),
            "{}",
            told.message
        );
    }
    let next = std::fs::read_to_string(scratch.dir().join("kept").join("next")).unwrap();
    assert_eq!(next.trim(), most.to_string());
    let mut server = Server::with(Some(&host), &options).unwrap();
    let worlds = described(&mut server).get("worlds").cloned().unwrap();
    assert_eq!(worlds.int_at("ignored_total"), 0, "{}", written(&worlds));
    let told = refused(&call(&mut server, "grit_world_create", made));
    assert_eq!(told.code, "too_many", "{}", told.message);
    let told = refused(&call(
        &mut server,
        "grit_world_step",
        &format!("{{\"world\":{}}}", most - 1),
    ));
    assert_eq!(told.code, "closed");
}

#[test]
fn a_kept_world_that_cannot_be_rebuilt_is_listed_broken_and_closes_with_its_files() {
    let scratch = Scratch::new("broken-kept");
    let keep = Keep::at(&scratch.dir()).unwrap();
    let made = Made {
        rule: "sand".to_owned(),
        params: Vec::new(),
        size: [8, 8, 8],
        sparse: false,
        slots: None,
    };
    let gone = std::sync::Arc::new(journal::Snap {
        sum: [7; 32],
        frame: 4,
        made: made.clone(),
        image: None,
    });
    keep.begin(2, &Ledger::snapped(gone)).unwrap();
    keep.begin(3, &Ledger::created(made)).unwrap();
    keep.set_next(4).unwrap();
    drop(keep);
    let options = Options {
        dir: Some(scratch.dir()),
        memory: None,
    };
    for _ in 0..2 {
        let mut server = Server::with(None, &options).unwrap();
        let listed = answer(&call(&mut server, "grit_world_list", "{}"));
        assert_eq!(
            kinds_listed(&listed),
            [("broken".to_owned(), 2), ("kept".to_owned(), 3)]
        );
        let told = refused(&call(
            &mut server,
            "grit_world_open",
            "{\"name\":\"world-2\"}",
        ));
        assert_eq!(told.code, "bad_file", "{}", told.message);
        assert!(told.message.contains("close it"), "{}", told.message);
    }
    let mut server = Server::with(None, &options).unwrap();
    let closed = answer(&call(&mut server, "grit_world_close", "{\"world\":2}"));
    assert_eq!(closed.get("closed"), Some(&Json::Bool(true)));
    assert!(!entries(&scratch.dir().join("kept")).contains(&"world-2.journal".to_owned()));
    let listed = answer(&call(&mut server, "grit_world_list", "{}"));
    assert_eq!(kinds_listed(&listed), [("kept".to_owned(), 3)]);
    let told = refused(&call(&mut server, "grit_world_step", "{\"world\":2}"));
    assert_eq!(told.code, "closed");
    let closed = answer(&call(&mut server, "grit_world_close", "{\"world\":3}"));
    assert_eq!(closed.get("closed"), Some(&Json::Bool(true)));
    drop(server);
    let mut server = Server::with(None, &options).unwrap();
    let listed = answer(&call(&mut server, "grit_world_list", "{}"));
    assert_eq!(listed.int_at("total"), 0);
}

#[test]
fn a_fast_observation_moves_no_estimate_and_a_slow_one_raises_it_at_once() {
    use grit::serve3::budget::{Rates, Work};
    let unrelated = 64 << 20;
    for work in Work::ALL {
        let mut fresh = Rates::default();
        let prior = fresh.estimate(work, unrelated);
        let mut first = Rates::default();
        first.measure(work, unrelated, prior.mul_f64(0.001));
        assert_eq!(
            first.estimate(work, unrelated),
            prior,
            "{work:?} fast first"
        );
        let mut first = Rates::default();
        first.measure(work, unrelated, prior.mul_f64(3.0));
        assert!(
            first.estimate(work, unrelated) >= prior.mul_f64(2.99),
            "{work:?} a slow first observation must raise the estimate at once"
        );
        for _ in 0..2 {
            fresh.measure(work, unrelated, prior.mul_f64(0.5));
        }
        let settled = fresh.estimate(work, unrelated);
        assert!(settled < prior, "{work:?} never learns a faster device");
        let mut poisoned = fresh.clone();
        poisoned.measure(work, unrelated, settled.mul_f64(0.001));
        assert_eq!(
            poisoned.estimate(work, unrelated),
            settled,
            "{work:?} fast poison"
        );
        let mut slow = fresh.clone();
        slow.measure(work, unrelated, settled.mul_f64(1000.0));
        assert!(
            slow.estimate(work, unrelated) >= settled.mul_f64(999.0),
            "{work:?} a slow observation must raise the estimate at once"
        );
        for _ in 0..3 {
            slow.measure(work, unrelated, settled);
        }
        let back = slow.estimate(work, unrelated);
        assert!(
            back <= settled.mul_f64(1.01) && back >= settled.mul_f64(0.99),
            "{work:?}: {back:?} against {settled:?}"
        );
    }
    let mut rates = Rates::default();
    let prior = rates.replay();
    rates.measure_replay(Duration::from_millis(40), Duration::from_secs(20));
    assert!((rates.replay() - prior).abs() < 1e-9);
    for _ in 0..2 {
        rates.measure_replay(Duration::from_secs(1), Duration::from_millis(1500));
    }
    let settled = rates.replay();
    rates.measure_replay(Duration::from_secs(1), Duration::from_millis(10));
    assert!(
        (rates.replay() - settled).abs() < 1e-9,
        "{settled} {}",
        rates.replay()
    );
    rates.measure_replay(Duration::from_secs(1), Duration::from_secs(6));
    assert!(rates.replay() >= 6.0 - 1e-9, "{}", rates.replay());
}

#[test]
fn a_write_replayed_on_open_does_not_slow_the_next_create() {
    let Some(host) = device("write replay and build rate") else {
        return;
    };
    let side = dense_side(&host);
    drop(host);
    let scratch = Scratch::new("write-build");
    let options = Options {
        dir: Some(scratch.dir()),
        memory: None,
    };
    let keep = Keep::at(&scratch.dir()).unwrap();
    let mut ledger = Ledger::created(Made {
        rule: "sand".to_owned(),
        params: Vec::new(),
        size: [64, 64, 64],
        sparse: false,
        slots: None,
    });
    let mut seed = 0x7719_u64;
    let cells = scattered(&mut seed, 64, 200, &SAND_WORDS);
    ledger.push(Entry::Write(cells), Duration::from_millis(50));
    keep.begin(1, &ledger).unwrap();
    keep.set_next(2).unwrap();
    drop(keep);
    let mut again = Run::new(&options);
    let opened = answer(&again.ask("grit_world_open", "{\"name\":\"world-1\"}"));
    eprintln!("opened: {}", written(&opened));
    assert_eq!(opened.int_at("replayed"), 1);
    let rates = answer(&again.ask("grit_describe", "{}"))
        .get("submissions")
        .and_then(|one| one.get("rates"))
        .cloned()
        .unwrap();
    eprintln!("rates after the open: {}", written(&rates));
    assert!(rates.get("write").unwrap().int_at("observations") >= 1);
    for _ in 0..2 {
        let made = answer(&again.ask(
            "grit_world_create",
            &format!("{{\"wide\":{side},\"deep\":{side},\"tall\":{side},\"rule\":\"sand\"}}"),
        ));
        answer(&again.ask(
            "grit_world_close",
            &format!("{{\"world\":{}}}", made.int_at("world")),
        ));
    }
    again.exec.finish();
}

#[test]
fn one_skewed_replay_raises_the_replay_estimate_at_once_and_no_further_than_its_clamp() {
    let Some(host) = device("skewed replay") else {
        return;
    };
    drop(host);
    let scratch = Scratch::new("skewed");
    let keep = Keep::at(&scratch.dir()).unwrap();
    let mut ledger = Ledger::created(Made {
        rule: "sand".to_owned(),
        params: Vec::new(),
        size: [32, 32, 32],
        sparse: false,
        slots: None,
    });
    ledger.push(Entry::Step(40000), Duration::from_millis(50));
    keep.begin(1, &ledger).unwrap();
    keep.set_next(2).unwrap();
    drop(keep);
    let mut run = Run::new(&Options {
        dir: Some(scratch.dir()),
        memory: None,
    });
    let opened = answer(&run.ask(
        "grit_world_open",
        "{\"name\":\"world-1\",\"budget_ms\":600000}",
    ));
    assert_eq!(opened.int_at("frame"), 40000);
    let rates = answer(&run.ask("grit_describe", "{}"))
        .get("submissions")
        .and_then(|one| one.get("rates"))
        .cloned()
        .unwrap();
    eprintln!("rates after the skewed replay: {}", written(&rates));
    assert_eq!(rates.int_at("replay_observations"), 1);
    let ratio = rates.int_at("replay_per_recorded_percent");
    assert!(ratio > 200 && ratio <= 800, "{ratio}");
    run.exec.finish();
}

#[test]
fn a_journal_record_with_a_good_checksum_that_does_not_read_is_refused_by_name() {
    let scratch = Scratch::new("unreadable");
    let keep = Keep::at(&scratch.dir()).unwrap();
    let mut ledger = Ledger::created(Made {
        rule: "sand".to_owned(),
        params: Vec::new(),
        size: [8, 8, 8],
        sparse: false,
        slots: None,
    });
    ledger.push(Entry::Step(5), Duration::from_millis(1));
    keep.begin(1, &ledger).unwrap();
    keep.set_next(2).unwrap();
    drop(keep);
    let path = scratch.dir().join("kept").join("world-1.journal");
    let mut bytes = std::fs::read(&path).unwrap();
    let mut check = 0x811C_9DC5_u32;
    let payload = [500_u32];
    for word in [4_u32, 1].iter().chain(&payload) {
        for byte in word.to_le_bytes() {
            check ^= u32::from(byte);
            check = check.wrapping_mul(0x0100_0193);
        }
    }
    for word in [4_u32, 1, 500, check] {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    std::fs::write(&path, &bytes).unwrap();
    let options = Options {
        dir: Some(scratch.dir()),
        memory: None,
    };
    let mut server = Server::with(None, &options).unwrap();
    let listed = answer(&call(&mut server, "grit_world_list", "{}"));
    assert_eq!(kinds_listed(&listed), [("broken".to_owned(), 1)]);
    let told = refused(&call(
        &mut server,
        "grit_world_open",
        "{\"name\":\"world-1\"}",
    ));
    assert_eq!(told.code, "bad_file", "{}", told.message);
    assert!(told.message.contains("record 2"), "{}", told.message);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    answer(&call(&mut server, "grit_world_close", "{\"world\":1}"));
    assert!(!path.exists());
}

#[test]
fn a_world_stepped_with_small_budgets_keeps_making_progress() {
    let Some(host) = device("small budgets progress") else {
        return;
    };
    drop(host);
    let scratch = Scratch::new("progress");
    let mut run = Run::new(&Options {
        dir: Some(scratch.dir()),
        memory: None,
    });
    let world = answer(&run.ask(
        "grit_world_create",
        "{\"wide\":128,\"deep\":128,\"tall\":128,\"rule\":\"sand\"}",
    ))
    .int_at("world");
    let mut seed = 0x5113_u64;
    let cells = scattered(&mut seed, 128, 4096, &SAND_WORDS);
    answer(&run.ask(
        "grit_world_write",
        &format!("{{\"world\":{world},\"cells\":{}}}", writing(&cells)),
    ));
    let mut frame = 0;
    for call in 0..16 {
        let got = answer(&run.ask(
            "grit_world_step",
            &format!("{{\"world\":{world},\"frames\":65536,\"budget_ms\":300}}"),
        ));
        let now = got.int_at("frame");
        assert!(
            now > frame,
            "call {call} made no progress: {}",
            written(&got)
        );
        frame = now;
    }
    let rates = answer(&run.ask("grit_describe", "{}"))
        .get("submissions")
        .and_then(|one| one.get("rates"))
        .cloned()
        .unwrap();
    eprintln!(
        "16 small-budget steps reached frame {frame}; rates {}",
        written(&rates)
    );
    assert!(rates.get("save").unwrap().int_at("observations") >= 1);
    assert!(
        entries(&scratch.dir().join("kept"))
            .iter()
            .any(|one| one.starts_with("snap-"))
    );
    run.exec.finish();
}

#[test]
fn a_write_past_its_budget_stops_between_cells_and_says_how_many_it_wrote() {
    let Some(host) = device("write budget") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let world = create(
        &mut server,
        "{\"wide\":64,\"deep\":64,\"tall\":64,\"rule\":\"sand\"}",
    );
    let mut seed = 0x3311_u64;
    let mut cells = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for (at, word) in scattered(&mut seed, 64, 8000, &SAND_WORDS) {
        if seen.insert(at) && cells.len() < 4096 {
            cells.push((at, word));
        }
    }
    let began = Instant::now();
    let got = answer(&call(
        &mut server,
        "grit_world_write",
        &format!(
            "{{\"world\":{world},\"cells\":{},\"budget_ms\":100}}",
            writing(&cells)
        ),
    ));
    let took = began.elapsed();
    let wrote = usize::try_from(got.int_at("written")).unwrap();
    eprintln!(
        "a write of {} cells with budget_ms 100 wrote {wrote} in {took:?}",
        cells.len()
    );
    assert!(took < Duration::from_millis(100 + 250 + 100), "{took:?}");
    assert_eq!(got.get("stopped").and_then(Json::text), Some("budget"));
    assert_eq!(got.int_at("asked"), i64::try_from(cells.len()).unwrap());
    assert!(wrote >= 1 && wrote < cells.len(), "{wrote}");
    for (index, (at, word)) in [
        (0, cells[0]),
        (wrote - 1, cells[wrote - 1]),
        (wrote, cells[wrote]),
    ] {
        let got = answer(&call(
            &mut server,
            "grit_world_read",
            &format!(
                "{{\"world\":{world},\"from\":[{0},{1},{2}],\"to\":[{0},{1},{2}]}}",
                at[0], at[1], at[2]
            ),
        ));
        let want = if index < wrote { word } else { 0 };
        assert_eq!(numbers(&got, "cells"), [want], "cell {index}");
    }
}

#[test]
fn an_evo_world_made_and_stepped_through_the_tools_equals_the_cpu_reference() {
    let Some(host) = device("evo world") else {
        return;
    };
    let side = 16;
    let grid = Grid3::new(side, side, side).unwrap();
    let mut server = Server::new(Some(&host));
    let mut params = grit::rule3::evo::DEFAULTS.to_vec();
    params[grit::rule3::evo::SEED] = 0x5EED;
    let made = answer(&call(
        &mut server,
        "grit_world_create",
        &format!(
            "{{\"wide\":{side},\"deep\":{side},\"tall\":{side},\"rule\":\"evo\",\"params\":{params:?}}}"
        ),
    ));
    assert_eq!(made.get("rule").and_then(Json::text), Some("evo"));
    let world = made.int_at("world");
    let mut seed = 0xE70_u64;
    let mut want = vec![0_u32; grid.cells()];
    let mut cells = Vec::new();
    for _ in 0..256 {
        let roll = next(&mut seed);
        let at = [
            (roll % u64::from(side)) as u32,
            ((roll >> 8) % u64::from(side)) as u32,
            ((roll >> 16) % u64::from(side)) as u32,
        ];
        let genome = ((roll >> 24) & u64::from(grit::rule3::evo::GENOME_MASK)) as u32;
        let word = grit::rule3::evo::cell(genome, 128 + ((roll >> 48) % 128) as u32, 0);
        want[grid.index(at[0], at[1], at[2])] = word;
        cells.push((at, word));
    }
    answer(&call(
        &mut server,
        "grit_world_write",
        &format!("{{\"world\":{world},\"cells\":{}}}", writing(&cells)),
    ));
    let listed = rules().into_iter().find(|one| one.name() == "evo").unwrap();
    let start = want.clone();
    let mut frame = 0;
    for batch in 0..4 {
        let got = answer(&call(
            &mut server,
            "grit_world_step",
            &format!("{{\"world\":{world},\"frames\":16}}"),
        ));
        assert_eq!(got.int_at("frame"), i64::from(frame + 16));
        for _ in 0..16 {
            want = listed.reference(grid, frame, &want, &params);
            frame += 1;
        }
        let read = answer(&call(
            &mut server,
            "grit_world_read",
            &format!(
                "{{\"world\":{world},\"from\":[0,0,0],\"to\":[{0},{0},{0}]}}",
                side - 1
            ),
        ));
        assert_eq!(numbers(&read, "cells"), want, "batch {batch}");
    }
    let alive = want
        .iter()
        .filter(|&&word| grit::rule3::evo::alive_of(word))
        .count();
    eprintln!("an evo world of {side}^3 after {frame} frames holds {alive} living cells");
    assert_ne!(want, start);
}

#[test]
fn the_highest_world_number_described_is_the_highest_handed_out() {
    let mut server = Server::new(None);
    let limits = described(&mut server).get("limits").cloned().unwrap();
    assert_eq!(
        limits.int_at("world_number_most"),
        i64::from(journal::NUMBER_MOST) - 1
    );
}

#[cfg(feature = "faults")]
fn off_worker_most(run: &mut Run, budget: u64, unit: Duration) -> Duration {
    let told = answer(&run.ask("grit_describe", "{}"))
        .get("submissions")
        .cloned()
        .unwrap();
    let batch = u64::try_from(told.int_at("longest_ms"))
        .unwrap()
        .max(grit::serve3::budget::SUBMIT_MOST_MS);
    Duration::from_millis(budget + batch + 300) + unit
}

#[cfg(feature = "faults")]
fn idle_longest(run: &mut Run) -> Duration {
    let told = answer(&run.ask("grit_describe", "{}"))
        .get("submissions")
        .cloned()
        .unwrap();
    Duration::from_millis(u64::try_from(told.int_at("idle_longest_ms")).unwrap())
}

fn late_most(run: &mut Run, budget: u64) -> Duration {
    let told = answer(&run.ask("grit_describe", "{}"))
        .get("submissions")
        .cloned()
        .unwrap();
    let batch = u64::try_from(told.int_at("longest_ms"))
        .unwrap()
        .max(grit::serve3::budget::SUBMIT_MOST_MS);
    let idle = u64::try_from(told.int_at("idle_longest_ms")).unwrap();
    Duration::from_millis(budget + batch + idle + 150)
}

#[cfg(feature = "faults")]
fn written_whole(run: &mut Run, world: i64, cells: &[([u32; 3], u32)]) {
    let wrote = answer(&run.ask(
        "grit_world_write",
        &format!(
            "{{\"world\":{world},\"cells\":{},\"budget_ms\":250000}}",
            writing(cells)
        ),
    ));
    assert_eq!(
        wrote.int_at("written"),
        i64::try_from(cells.len()).unwrap(),
        "{}",
        written(&wrote)
    );
}

fn kept_files(scratch: &Scratch) -> Vec<String> {
    entries(&scratch.dir().join("kept"))
}

fn far_over(run: &mut Run, scratch: &Scratch, world: i64) {
    for _ in 0..12 {
        answer(&run.ask(
            "grit_world_step",
            &format!("{{\"world\":{world},\"frames\":65536,\"budget_ms\":2500}}"),
        ));
        answer(&run.ask("grit_world_rules", "{}"));
        let item = listed_world(run, world);
        if item.get("snapshot_failed").is_some()
            || kept_files(scratch)
                .iter()
                .any(|one| one.starts_with("snap-"))
        {
            return;
        }
    }
    panic!("the world never owed a snapshot");
}

fn listed_world(run: &mut Run, world: i64) -> Json {
    let listed = answer(&run.ask("grit_world_list", "{}"));
    listed
        .get("items")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .find(|one| one.get("world").and_then(Json::int) == Some(world))
        .cloned()
        .unwrap()
}

fn sand_world(run: &mut Run, side: u32) -> i64 {
    let world = answer(&run.ask(
        "grit_world_create",
        &format!("{{\"wide\":{side},\"deep\":{side},\"tall\":{side},\"rule\":\"sand\"}}"),
    ))
    .int_at("world");
    let cells: Vec<([u32; 3], u32)> = (0..side.min(32))
        .map(|at| ([at, 0, at], grit::sand::SAND))
        .collect();
    answer(&run.ask(
        "grit_world_write",
        &format!("{{\"world\":{world},\"cells\":{}}}", writing(&cells)),
    ));
    world
}

#[cfg(feature = "faults")]
#[test]
fn a_failed_snapshot_between_calls_backs_off_is_named_and_leaves_no_files() {
    let Some(host) = device("failed idle snapshot") else {
        return;
    };
    drop(host);
    let scratch = Scratch::new("snap-fails");
    let mut run = Run::new(&Options {
        dir: Some(scratch.dir()),
        memory: None,
    });
    let world = sand_world(&mut run, 32);
    run.exec.faults().lock().unwrap().fail_snapshots = 1000;
    far_over(&mut run, &scratch, world);
    let began = Instant::now();
    for _ in 0..40 {
        answer(&run.ask("grit_world_rules", "{}"));
        std::thread::sleep(Duration::from_millis(25));
    }
    let spent = began.elapsed();
    let item = listed_world(&mut run, world);
    let failed = item
        .get("snapshot_failed")
        .cloned()
        .unwrap_or_else(|| panic!("no snapshot failure named: {}", written(&item)));
    let failures = failed.int_at("failures");
    eprintln!(
        "{failures} snapshot attempts in {spent:?}: {}",
        written(&failed)
    );
    assert!(
        (1..=3).contains(&failures),
        "{failures} attempts in {spent:?}"
    );
    assert!(
        failed
            .get("message")
            .and_then(Json::text)
            .unwrap()
            .contains("armed fault")
    );
    let worlds = answer(&run.ask("grit_describe", "{}"))
        .get("worlds")
        .cloned()
        .unwrap();
    assert_eq!(worlds.int_at("snapshot_failures"), 1);
    let left: Vec<String> = kept_files(&scratch)
        .into_iter()
        .filter(|one| one.starts_with("pending-") || one.ends_with(".tmp") || one.starts_with('.'))
        .collect();
    assert!(left.is_empty(), "{left:?}");
    let got = run.ask(
        "grit_world_step",
        &format!("{{\"world\":{world},\"frames\":3}}"),
    );
    let said = written(&got);
    let result = got.get("result").unwrap();
    let told = result.get("structuredContent").unwrap();
    if result.get("isError") == Some(&Json::Bool(true)) {
        assert_eq!(told.get("code").and_then(Json::text), Some("io"), "{said}");
    }
    assert!(
        said.contains("armed fault"),
        "the failure is not named: {said}"
    );
    run.exec.faults().lock().unwrap().fail_snapshots = 0;
    let began = Instant::now();
    loop {
        let got = run.ask(
            "grit_world_step",
            &format!("{{\"world\":{world},\"frames\":1}}"),
        );
        let result = got.get("result").unwrap();
        if result.get("isError") == Some(&Json::Bool(true)) {
            let told = result.get("structuredContent").unwrap();
            assert_eq!(
                told.get("code").and_then(Json::text),
                Some("io"),
                "{}",
                written(&got)
            );
            assert!(written(&got).contains("the next is"), "{}", written(&got));
        }
        let item = listed_world(&mut run, world);
        if item.get("snapshot_failed").is_none() {
            break;
        }
        assert!(
            began.elapsed() < Duration::from_secs(20),
            "{}",
            written(&item)
        );
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(
        kept_files(&scratch)
            .iter()
            .any(|one| one.starts_with("snap-"))
    );
    run.exec.finish();
}

#[cfg(feature = "faults")]
#[test]
fn a_slow_snapshot_between_calls_never_costs_a_sequential_client_its_budget() {
    let Some(host) = device("slow idle snapshot") else {
        return;
    };
    drop(host);
    let scratch = Scratch::new("slow-idle");
    let mut run = Run::new(&Options {
        dir: Some(scratch.dir()),
        memory: None,
    });
    let world = sand_world(&mut run, 32);
    let faults = run.exec.faults();
    let slow = Duration::from_millis(3000);
    faults.lock().unwrap().slow_write = slow;
    let budget = 40;
    let due = Ledger::created(Made {
        rule: "sand".to_owned(),
        params: Vec::new(),
        size: [32, 32, 32],
        sparse: false,
        slots: None,
    })
    .replay_most();
    let mut worst = Duration::ZERO;
    let mut after = None;
    let mut calls = 0;
    let mut stepping = Duration::ZERO;
    while after.is_none_or(|first: Instant| first.elapsed() < slow * 2) {
        assert!(
            after.is_some() || stepping <= due * 2,
            "no snapshot was taken between {calls} calls that spent {stepping:?}, twice the replay of {due:?} that makes one due"
        );
        let began = Instant::now();
        let got = run.ask(
            "grit_world_step",
            &format!("{{\"world\":{world},\"frames\":65536,\"budget_ms\":{budget}}}"),
        );
        let took = began.elapsed();
        let result = got.get("result").unwrap();
        assert_eq!(
            result.get("isError"),
            Some(&Json::Bool(false)),
            "call {calls} after {took:?}: {}",
            written(&got)
        );
        assert!(
            got_frames(&got) > 0,
            "call {calls} ran no frame: {}",
            written(&got)
        );
        let most = off_worker_most(&mut run, budget, slow / 2);
        assert!(
            took <= most,
            "call {calls} took {took:?}, more than {most:?}"
        );
        worst = worst.max(took);
        calls += 1;
        if after.is_none() {
            stepping += took;
        }
        if after.is_none() && faults.lock().unwrap().slowed > 0 {
            after = Some(Instant::now());
        }
    }
    eprintln!(
        "{calls} steps of {budget} ms beside a snapshot written for {slow:?} between calls; the slowest took {worst:?}"
    );
    let idle = idle_longest(&mut run);
    assert!(
        idle < slow / 2,
        "the longest unit of work between calls took {idle:?}, near the injected write of {slow:?}"
    );
    run.exec.finish();
    let left: Vec<String> = kept_files(&scratch)
        .into_iter()
        .filter(|one| one.starts_with("pending-") || one.ends_with(".tmp"))
        .collect();
    assert!(left.is_empty(), "{left:?}");
    assert!(
        kept_files(&scratch)
            .iter()
            .any(|one| one.starts_with("snap-"))
    );
}

fn warm_pace() -> grit::serve3::budget::Pace {
    use grit::serve3::budget::{BATCH_MOST, Pace};
    let mut pace = Pace::first(32 * 32 * 32);
    for _ in 0..4 {
        pace.ran(BATCH_MOST, PACE_FRAME * BATCH_MOST);
    }
    pace
}

const PACE_FRAME: Duration = Duration::from_micros(20);

const PACE_BUDGET: Duration = Duration::from_millis(40);

fn grown_back(pace: &mut grit::serve3::budget::Pace) -> u32 {
    let mut step = 0;
    for _ in 0..8 {
        step = pace.batch(65536, PACE_BUDGET);
        assert!(step > 0, "{pace:?}");
        pace.ran(step, PACE_FRAME * step);
    }
    step
}

#[test]
fn one_held_up_batch_never_locks_a_short_budget_out_of_a_world() {
    use grit::serve3::budget::{BATCH_MOST, SUBMIT_MOST_MS};
    let pace = warm_pace();
    for frames in [1, 2, 4] {
        let mut held = pace;
        held.ran(frames, Duration::from_millis(SUBMIT_MOST_MS));
        let step = held.opening(65536, PACE_BUDGET);
        assert!(
            step > 0,
            "{frames} frames held up for {SUBMIT_MOST_MS} ms refused a {PACE_BUDGET:?} budget: {held:?}"
        );
        held.ran(step, PACE_FRAME * step);
        assert_eq!(grown_back(&mut held), BATCH_MOST, "{held:?}");
    }
    let mut late = pace;
    late.ran(1, Duration::from_millis(SUBMIT_MOST_MS));
    assert_eq!(late.batch(65536, PACE_FRAME / 2), 0, "{late:?}");
}

#[test]
fn a_held_up_full_batch_never_locks_a_short_budget_out_of_a_world() {
    use grit::serve3::budget::BATCH_MOST;
    let mut pace = warm_pace();
    pace.ran(BATCH_MOST, Duration::from_secs(3));
    assert_eq!(pace.batch(65536, PACE_BUDGET), 0, "{pace:?}");
    assert_eq!(pace.opening(65536, PACE_BUDGET), 1, "{pace:?}");
    pace.ran(1, PACE_FRAME);
    assert_eq!(grown_back(&mut pace), BATCH_MOST, "{pace:?}");
}

#[test]
fn a_held_up_first_batch_never_locks_a_short_budget_out_of_a_world() {
    use grit::serve3::budget::{BATCH_MOST, Pace};
    let mut pace = Pace::first(32 * 32 * 32);
    let first = pace.opening(65536, PACE_BUDGET);
    assert!(first > 0, "{pace:?}");
    pace.ran(first, Duration::from_secs(3));
    assert_eq!(pace.opening(65536, PACE_BUDGET), 1, "{pace:?}");
    pace.ran(1, PACE_FRAME);
    assert_eq!(grown_back(&mut pace), BATCH_MOST, "{pace:?}");
}

#[test]
fn a_world_whose_frames_do_not_fit_is_refused_up_front_after_one_frame_confirms_it() {
    use grit::serve3::budget::BATCH_MOST;
    let slow = PACE_BUDGET * 2;
    let mut pace = warm_pace();
    pace.ran(BATCH_MOST, slow * BATCH_MOST);
    let mut probed = Vec::new();
    for call in 0..64 {
        let step = pace.opening(65536, PACE_BUDGET);
        if step > 0 {
            assert_eq!(step, 1, "call {call}: {pace:?}");
            probed.push(call);
            pace.ran(1, slow);
        }
    }
    assert_eq!(probed, vec![0, 2, 5, 10, 19, 36], "{pace:?}");
    let next = (0..64)
        .find(|_| pace.opening(65536, PACE_BUDGET) > 0)
        .unwrap();
    pace.ran(1, PACE_FRAME);
    assert_eq!(
        grown_back(&mut pace),
        BATCH_MOST,
        "after {next} more refusals: {pace:?}"
    );
}

#[test]
fn any_run_of_held_up_probes_ends_at_the_first_probe_that_is_not() {
    use grit::serve3::budget::{BATCH_MOST, SUBMIT_MOST_MS};
    for held in 0..8_u32 {
        let mut pace = warm_pace();
        pace.ran(BATCH_MOST, Duration::from_secs(3));
        let mut probes = 0;
        let mut calls = 0_u32;
        loop {
            assert!(
                calls < 1 << (held + 2),
                "{held} held-up probes: still refused after {calls} calls: {pace:?}"
            );
            calls += 1;
            if pace.opening(65536, PACE_BUDGET) == 0 {
                continue;
            }
            if probes < held {
                pace.ran(1, Duration::from_millis(SUBMIT_MOST_MS));
                probes += 1;
                continue;
            }
            pace.ran(1, PACE_FRAME);
            break;
        }
        assert_eq!(grown_back(&mut pace), BATCH_MOST, "{held}: {pace:?}");
    }
}

#[cfg(feature = "faults")]
fn sand_server_world(server: &mut Server<'_>) -> i64 {
    let world = answer(&call(
        server,
        "grit_world_create",
        "{\"wide\":32,\"deep\":32,\"tall\":32,\"rule\":\"sand\"}",
    ))
    .int_at("world");
    let cells: Vec<([u32; 3], u32)> = (0..32).map(|at| ([at, 0, at], grit::sand::SAND)).collect();
    answer(&call(
        server,
        "grit_world_write",
        &format!("{{\"world\":{world},\"cells\":{}}}", writing(&cells)),
    ));
    world
}

#[cfg(feature = "faults")]
fn short_steps_all_run(server: &mut Server<'_>, world: i64, what: &str) {
    let short = format!("{{\"world\":{world},\"frames\":65536,\"budget_ms\":40}}");
    for call_at in 0..8 {
        let got = call(server, "grit_world_step", &short);
        assert!(
            got_frames(&got) > 0,
            "call {call_at} after {what} ran no frame: {}",
            written(&got)
        );
    }
}

#[cfg(feature = "faults")]
#[test]
fn a_single_frame_held_up_on_the_device_never_locks_short_budgets_out_of_a_world() {
    let Some(host) = device("held up batch") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let world = sand_server_world(&mut server);
    let short = format!("{{\"world\":{world},\"frames\":65536,\"budget_ms\":40}}");
    for _ in 0..4 {
        assert!(got_frames(&call(&mut server, "grit_world_step", &short)) > 0);
    }
    let held = Duration::from_millis(grit::serve3::budget::SUBMIT_MOST_MS);
    grit::serve3::tools::faults::stall_batch(0, held);
    answer(&call(
        &mut server,
        "grit_world_step",
        &format!("{{\"world\":{world},\"frames\":1}}"),
    ));
    short_steps_all_run(
        &mut server,
        world,
        &format!("one frame held up for {held:?}"),
    );
}

#[cfg(feature = "faults")]
#[test]
fn a_full_batch_held_up_on_the_device_never_locks_short_budgets_out_of_a_world() {
    let Some(host) = device("held up full batch") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let world = sand_server_world(&mut server);
    let whole = format!(
        "{{\"world\":{world},\"frames\":{}}}",
        grit::serve3::budget::BATCH_MOST
    );
    for _ in 0..2 {
        answer(&call(&mut server, "grit_world_step", &whole));
    }
    let held = Duration::from_secs(3);
    grit::serve3::tools::faults::stall_batch(0, held);
    answer(&call(&mut server, "grit_world_step", &whole));
    let submissions = described(&mut server).get("submissions").cloned().unwrap();
    assert_eq!(
        submissions.int_at("longest_frames"),
        i64::from(grit::serve3::budget::BATCH_MOST),
        "{}",
        written(&submissions)
    );
    short_steps_all_run(
        &mut server,
        world,
        &format!("a full batch held up for {held:?}"),
    );
}

#[cfg(feature = "faults")]
#[test]
fn a_first_batch_held_up_on_the_device_never_locks_short_budgets_out_of_a_world() {
    let Some(host) = device("held up first batch") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let world = sand_server_world(&mut server);
    let held = Duration::from_secs(3);
    grit::serve3::tools::faults::stall_batch(0, held);
    let first = answer(&call(
        &mut server,
        "grit_world_step",
        &format!("{{\"world\":{world},\"frames\":9}}"),
    ));
    assert_eq!(first.int_at("frame"), 9);
    short_steps_all_run(
        &mut server,
        world,
        &format!("a first batch held up for {held:?}"),
    );
}

#[cfg(feature = "faults")]
#[test]
fn a_world_whose_frames_outlast_a_short_budget_is_refused_at_once_between_rare_probes() {
    let Some(host) = device("frames over budget") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let world = sand_server_world(&mut server);
    answer(&call(
        &mut server,
        "grit_world_step",
        &format!("{{\"world\":{world},\"frames\":64}}"),
    ));
    let budget = Duration::from_millis(40);
    let frame = budget * 2;
    let late = Duration::from_millis(grit::serve3::budget::SUBMIT_MOST_MS);
    let short = format!(
        "{{\"world\":{world},\"frames\":65536,\"budget_ms\":{}}}",
        budget.as_millis()
    );
    grit::serve3::tools::faults::stall_every(frame);
    answer(&call(
        &mut server,
        "grit_world_step",
        &format!("{{\"world\":{world},\"frames\":1}}"),
    ));
    let mut probed = Vec::new();
    for call_at in 0..12 {
        let began = Instant::now();
        let got = call(&mut server, "grit_world_step", &short);
        let took = began.elapsed();
        let ran = got_frames(&got);
        if ran > 0 {
            assert_eq!(ran, 1, "call {call_at}: {}", written(&got));
            probed.push(call_at);
        } else {
            assert!(took < budget, "refused call {call_at} took {took:?}");
        }
        assert!(
            took <= budget + frame + late,
            "call {call_at} took {took:?}, more than its budget, one frame of {frame:?} and one batch"
        );
    }
    grit::serve3::tools::faults::disarm();
    assert_eq!(probed, vec![0, 2, 5, 10]);
    let ran = got_frames(&call(
        &mut server,
        "grit_world_step",
        &format!("{{\"world\":{world},\"frames\":65536,\"budget_ms\":1000}}"),
    ));
    assert!(ran > 0);
    short_steps_all_run(&mut server, world, "frames that fit again");
}

#[cfg(feature = "faults")]
#[test]
fn a_frame_held_up_while_a_world_is_rebuilt_never_refuses_a_journal_that_fits() {
    let Some(host) = device("held up replay") else {
        return;
    };
    let scratch = Scratch::new("held-replay");
    let options = Options {
        dir: Some(scratch.dir()),
        memory: None,
    };
    let mut first = Server::with(Some(&host), &options).unwrap();
    let world = sand_server_world(&mut first);
    for frames in [1, 64] {
        answer(&call(
            &mut first,
            "grit_world_step",
            &format!("{{\"world\":{world},\"frames\":{frames}}}"),
        ));
    }
    drop(first);
    let mut again = Server::with(Some(&host), &options).unwrap();
    let held = Duration::from_millis(1500);
    grit::serve3::tools::faults::stall_batch(0, held);
    let got = call(
        &mut again,
        "grit_world_open",
        &format!("{{\"name\":\"world-{world}\",\"budget_ms\":2500}}"),
    );
    grit::serve3::tools::faults::disarm();
    let opened = answer(&got);
    assert_eq!(opened.int_at("frame"), 65, "{}", written(&got));
}

#[cfg(feature = "faults")]
fn behind_idle_work(dir: bool) {
    let Some(host) = device("slow idle work") else {
        return;
    };
    drop(host);
    let scratch = Scratch::new("idle-work");
    let mut run = Run::new(&Options {
        dir: dir.then(|| scratch.dir()),
        memory: None,
    });
    let world = sand_world(&mut run, 32);
    let faults = run.exec.faults();
    let slow = Duration::from_millis(1500);
    faults.lock().unwrap().slow_idle_work = slow;
    faults.lock().unwrap().pause_after_answer = Duration::from_millis(5);
    let budget = 100;
    let want = if dir { 2 } else { 1 };
    let mut behind = Duration::ZERO;
    let mut calls = 0;
    while faults.lock().unwrap().idle_works < want + 1 {
        assert!(calls < 200, "the server did no idle work");
        let began = Instant::now();
        let got = run.ask(
            "grit_world_step",
            &format!("{{\"world\":{world},\"frames\":65536,\"budget_ms\":{budget}}}"),
        );
        let took = began.elapsed();
        let result = got.get("result").unwrap();
        assert_eq!(
            result.get("isError"),
            Some(&Json::Bool(false)),
            "call {calls} after {took:?}: {}",
            written(&got)
        );
        let most = off_worker_most(&mut run, budget, slow + slow / 3);
        assert!(
            took <= most,
            "call {calls} took {took:?}, more than {most:?}"
        );
        assert!(
            got_frames(&got) > 0,
            "call {calls} ran no frame: {}",
            written(&got)
        );
        if took > slow / 2 {
            behind = behind.max(took);
        }
        calls += 1;
    }
    let units = faults.lock().unwrap().idle_works;
    eprintln!(
        "{calls} steps of {budget} ms beside {units} units of idle work of {slow:?} each (directory {dir}); the longest call behind one took {behind:?}"
    );
    assert!(behind > slow / 2, "no call waited behind idle work");
    run.exec.finish();
}

#[cfg(feature = "faults")]
fn got_frames(got: &Json) -> i64 {
    got.get("result")
        .and_then(|one| one.get("structuredContent"))
        .and_then(|one| one.get("ran"))
        .and_then(Json::int)
        .unwrap_or(0)
}

#[cfg(feature = "faults")]
#[test]
fn a_call_queued_behind_slow_idle_work_keeps_its_whole_budget_and_no_more() {
    behind_idle_work(true);
}

#[cfg(feature = "faults")]
#[test]
fn a_call_queued_behind_a_slow_snapshot_kept_in_memory_keeps_its_whole_budget() {
    behind_idle_work(false);
}

#[cfg(feature = "faults")]
#[test]
fn a_call_to_an_idle_server_is_not_charged_for_the_worker_waking() {
    let Some(host) = device("slow wake") else {
        return;
    };
    drop(host);
    let mut run = Run::new(&Options::default());
    let world = sand_world(&mut run, 16);
    let wake = Duration::from_millis(60);
    run.exec.faults().lock().unwrap().slow_wake = wake;
    std::thread::sleep(Duration::from_millis(100));
    let budget = 20;
    let began = Instant::now();
    let got = run.ask(
        "grit_world_step",
        &format!("{{\"world\":{world},\"frames\":65536,\"budget_ms\":{budget}}}"),
    );
    let took = began.elapsed();
    let result = got.get("result").unwrap();
    assert_eq!(
        result.get("isError"),
        Some(&Json::Bool(false)),
        "after {took:?}: {}",
        written(&got)
    );
    let told = result.get("structuredContent").unwrap();
    assert!(told.int_at("ran") > 0, "{}", written(&got));
    eprintln!("a call of {budget} ms to a worker that woke {wake:?} late answered in {took:?}");
    run.exec.finish();
}

#[cfg(feature = "faults")]
#[test]
fn a_slow_background_write_never_refuses_a_sequential_client() {
    let Some(host) = device("slow background write") else {
        return;
    };
    drop(host);
    let scratch = Scratch::new("slow-write");
    let options = Options {
        dir: Some(scratch.dir()),
        memory: None,
    };
    let mut run = Run::new(&options);
    let world = sand_world(&mut run, 32);
    let faults = run.exec.faults();
    let slow = Duration::from_millis(3000);
    faults.lock().unwrap().slow_write = slow;
    let budget = 200;
    let began = Instant::now();
    let mut calls = 0;
    let mut frame = 0;
    let mut worst = Duration::ZERO;
    while began.elapsed() < slow * 4 || faults.lock().unwrap().slowed < 3 {
        assert!(calls < 400, "no write was started between calls");
        let asked = Instant::now();
        let got = run.ask(
            "grit_world_step",
            &format!("{{\"world\":{world},\"frames\":65536,\"budget_ms\":{budget}}}"),
        );
        let took = asked.elapsed();
        let result = got.get("result").unwrap();
        assert_eq!(
            result.get("isError"),
            Some(&Json::Bool(false)),
            "call {calls} after {took:?}: {}",
            written(&got)
        );
        let told = result.get("structuredContent").unwrap();
        assert!(
            told.int_at("frame") > frame,
            "call {calls}: {}",
            written(&got)
        );
        frame = told.int_at("frame");
        let most = off_worker_most(&mut run, budget, slow / 2);
        assert!(
            took <= most,
            "call {calls} took {took:?}, more than {most:?}"
        );
        worst = worst.max(took);
        calls += 1;
    }
    let slowed = faults.lock().unwrap().slowed;
    eprintln!(
        "{calls} steps of {budget} ms beside {slowed} background writes of {slow:?} each reached frame {frame}; the slowest took {worst:?}"
    );
    let idle = idle_longest(&mut run);
    assert!(
        idle < slow / 2,
        "the longest unit of work between calls took {idle:?}, near the injected write of {slow:?}"
    );
    run.exec.finish();
    let mut again = Run::new(&options);
    let opened = answer(&again.ask(
        "grit_world_open",
        &format!("{{\"name\":\"world-{world}\",\"budget_ms\":600000}}"),
    ));
    assert_eq!(opened.int_at("frame"), frame);
    again.exec.finish();
}

#[test]
fn a_write_in_flight_bounds_the_journal_after_it_and_caps_the_whole() {
    let made = Made {
        rule: "sand".to_owned(),
        params: Vec::new(),
        size: [16, 16, 16],
        sparse: false,
        slots: None,
    };
    let bytes = 1 << 20;
    let mut ledger = Ledger::created(made.clone());
    ledger.push(Entry::Step(1), Duration::from_secs(30));
    let from = Some((ledger.entries.len(), Duration::ZERO));
    assert!(ledger.far_over(Duration::ZERO, bytes, 2.0));
    assert!(ledger.far_over_writing(Duration::ZERO, bytes, 2.0, None));
    assert!(!ledger.far_over_writing(Duration::ZERO, bytes, 2.0, from));
    ledger.push(Entry::Step(1), Duration::from_secs(2));
    assert!(!ledger.far_over_writing(Duration::ZERO, bytes, 2.0, from));
    assert!(ledger.far_over_writing(Duration::from_millis(1), bytes, 2.0, from));
    let started = from.map(|(at, _)| (at, Duration::from_secs(2)));
    ledger.push(Entry::Step(1), Duration::from_secs(4));
    assert!(ledger.far_over_writing(Duration::ZERO, bytes, 2.0, from));
    assert!(!ledger.far_over_writing(Duration::ZERO, bytes, 2.0, started));
    assert!(ledger.far_over_writing(Duration::from_millis(2001), bytes, 2.0, started));
    let mut whole = Ledger::created(made);
    whole.push(Entry::Step(1), Duration::from_secs(120));
    let from = Some((whole.entries.len(), Duration::ZERO));
    assert!(!whole.far_over_writing(Duration::ZERO, bytes, 1.0, from));
    assert!(whole.far_over_writing(Duration::from_millis(1), bytes, 1.0, from));
    assert_eq!(journal::WRITING_REPLAY_MOST, Duration::from_secs(120));
}

#[test]
fn a_call_queued_behind_another_is_charged_that_wait_once() {
    let Some(host) = device("charged once") else {
        return;
    };
    drop(host);
    let mut run = Run::new(&Options::default());
    let world = sand_world(&mut run, 128);
    let first = 600;
    let second = 1500;
    let sent = Instant::now();
    let ahead = run.send(
        "grit_world_step",
        &format!("{{\"world\":{world},\"frames\":65536,\"budget_ms\":{first}}}"),
    );
    let behind = run.send(
        "grit_world_step",
        &format!("{{\"world\":{world},\"frames\":65536,\"budget_ms\":{second}}}"),
    );
    let went = answer(&run.wait(ahead));
    assert!(went.int_at("ran") > 0, "{}", written(&went));
    let got = run.wait(behind);
    let took = sent.elapsed();
    let told = answer(&got);
    assert!(told.int_at("ran") > 0, "{}", written(&got));
    let most = late_most(&mut run, second);
    let least = Duration::from_millis(second - grit::serve3::budget::SUBMIT_MOST_MS);
    eprintln!(
        "a call of {second} ms behind one of {first} ms answered {took:?} after both were sent (between {least:?} and {most:?})"
    );
    assert!(took <= most, "{took:?}, more than {most:?}");
    assert!(took >= least, "{took:?}, less than {least:?}");
    run.exec.finish();
}

#[cfg(feature = "faults")]
#[test]
fn a_slow_disk_never_stalls_a_step_that_snapshots_inside_its_budget() {
    let Some(host) = device("slow disk in a step") else {
        return;
    };
    drop(host);
    let scratch = Scratch::new("slow-disk");
    let mut run = Run::new(&Options {
        dir: Some(scratch.dir()),
        memory: None,
    });
    let world = sand_world(&mut run, 64);
    let faults = run.exec.faults();
    let slow = Duration::from_millis(3000);
    faults.lock().unwrap().slow_write = slow;
    let budget = 1000;
    let due = Ledger::created(Made {
        rule: "sand".to_owned(),
        params: Vec::new(),
        size: [64, 64, 64],
        sparse: false,
        slots: None,
    })
    .replay_most();
    let snaps = |scratch: &Scratch| -> Vec<String> {
        kept_files(scratch)
            .into_iter()
            .filter(|one| one.starts_with("snap-"))
            .collect()
    };
    let mut worst = Duration::ZERO;
    let mut frame = 0;
    let mut began = Vec::new();
    let mut owed = Some(due);
    let mut stepping = Duration::ZERO;
    let mut writing: Option<(Instant, Vec<String>)> = None;
    let mut call = 0;
    while call < 10 || began.len() < 2 {
        if let Some((from, before)) = writing.as_ref()
            && snaps(&scratch) != *before
        {
            owed = Some(due.max(from.elapsed() * 4));
            writing = None;
        }
        match (owed, writing.as_ref()) {
            (Some(owed), _) => assert!(
                stepping <= owed * 2,
                "snapshot {} did not begin after {stepping:?} of steps, twice the replay of {owed:?} that makes it due",
                began.len() + 1
            ),
            (None, Some((from, _))) => assert!(
                from.elapsed() <= slow * 4,
                "the write of {slow:?} begun by call {:?} had not landed after {:?}",
                began.last(),
                from.elapsed()
            ),
            (None, None) => {}
        }
        let asked = Instant::now();
        let got = run.ask(
            "grit_world_step",
            &format!("{{\"world\":{world},\"frames\":65536,\"budget_ms\":{budget}}}"),
        );
        let took = asked.elapsed();
        let told = answer(&got);
        assert!(
            told.int_at("frame") > frame,
            "call {call}: {}",
            written(&got)
        );
        frame = told.int_at("frame");
        let most = off_worker_most(&mut run, budget, slow / 2);
        assert!(
            took <= most,
            "call {call} took {took:?}, more than {most:?}"
        );
        worst = worst.max(took);
        stepping += took;
        let slowed = faults.lock().unwrap().slowed;
        if usize::try_from(slowed).unwrap() > began.len() {
            began.push(call);
            writing = Some((asked, snaps(&scratch)));
            owed = None;
            stepping = Duration::ZERO;
        }
        call += 1;
    }
    let slowed = faults.lock().unwrap().slowed;
    eprintln!(
        "{call} steps of {budget} ms beside {slowed} snapshot writes of {slow:?}, begun by calls {began:?}; the slowest took {worst:?}"
    );
    assert!(slowed >= 2, "{slowed}");
    let idle = idle_longest(&mut run);
    assert!(
        idle < slow / 2,
        "the longest unit of work between calls took {idle:?}, near the injected write of {slow:?}"
    );
    run.exec.finish();
}

#[cfg(feature = "faults")]
#[test]
fn a_world_restored_while_its_snapshot_is_written_is_held_to_its_new_journal() {
    let Some(host) = device("restore during a write") else {
        return;
    };
    drop(host);
    let scratch = Scratch::new("restore-writing");
    let keep = Keep::at(&scratch.dir()).unwrap();
    let mut ledger = Ledger::created(Made {
        rule: "sand".to_owned(),
        params: Vec::new(),
        size: [32, 32, 32],
        sparse: false,
        slots: None,
    });
    ledger.push(Entry::Step(4), Duration::from_secs(30));
    keep.begin(1, &ledger).unwrap();
    keep.set_next(2).unwrap();
    drop(keep);
    let mut run = Run::new(&Options {
        dir: Some(scratch.dir()),
        memory: None,
    });
    let faults = run.exec.faults();
    faults.lock().unwrap().fail_snapshots = 1;
    answer(&run.ask(
        "grit_world_open",
        "{\"name\":\"world-1\",\"budget_ms\":600000}",
    ));
    let target = answer(&run.ask(
        "grit_world_create",
        "{\"wide\":32,\"deep\":32,\"tall\":32,\"rule\":\"sand\"}",
    ))
    .int_at("world");
    faults.lock().unwrap().slow_write = Duration::from_secs(6);
    let began = Instant::now();
    while faults.lock().unwrap().slowed == 0 {
        assert!(began.elapsed() < Duration::from_secs(30), "no write began");
        answer(&run.ask(
            "grit_world_step",
            &format!("{{\"world\":{target},\"frames\":65536,\"budget_ms\":1500}}"),
        ));
    }
    answer(&run.ask(
        "grit_world_restore",
        &format!("{{\"world\":{target},\"from\":1,\"budget_ms\":600000}}"),
    ));
    let told = refused(&run.ask(
        "grit_world_step",
        &format!("{{\"world\":{target},\"frames\":1,\"budget_ms\":300}}"),
    ));
    assert_eq!(told.code, "budget", "{}", told.message);
    assert!(told.message.contains("being written"), "{}", told.message);
    run.exec.finish();
}

#[cfg(feature = "faults")]
#[test]
fn a_world_far_over_while_another_worlds_snapshot_is_written_is_refused_once() {
    let Some(host) = device("another world's write") else {
        return;
    };
    drop(host);
    let scratch = Scratch::new("other-writing");
    let keep = Keep::at(&scratch.dir()).unwrap();
    for world in [1, 2] {
        let mut ledger = Ledger::created(Made {
            rule: "sand".to_owned(),
            params: Vec::new(),
            size: [32, 32, 32],
            sparse: false,
            slots: None,
        });
        ledger.push(Entry::Step(4), Duration::from_secs(30));
        keep.begin(world, &ledger).unwrap();
    }
    keep.set_next(3).unwrap();
    drop(keep);
    let mut run = Run::new(&Options {
        dir: Some(scratch.dir()),
        memory: None,
    });
    let faults = run.exec.faults();
    faults.lock().unwrap().slow_write = Duration::from_secs(6);
    answer(&run.ask(
        "grit_world_open",
        "{\"name\":\"world-1\",\"budget_ms\":600000}",
    ));
    answer(&run.ask(
        "grit_world_open",
        "{\"name\":\"world-2\",\"budget_ms\":600000}",
    ));
    assert_eq!(faults.lock().unwrap().slowed, 1);
    let told = refused(&run.ask(
        "grit_world_step",
        "{\"world\":2,\"frames\":1,\"budget_ms\":300}",
    ));
    assert_eq!(told.code, "budget", "{}", told.message);
    assert!(told.message.contains("being written"), "{}", told.message);
    run.exec.finish();
}

#[test]
fn a_directory_planted_where_a_snapshot_is_written_does_not_stop_snapshots() {
    let Some(host) = device("planted pending directory") else {
        return;
    };
    drop(host);
    let scratch = Scratch::new("planted");
    let options = Options {
        dir: Some(scratch.dir()),
        memory: None,
    };
    std::fs::create_dir_all(scratch.dir().join("kept")).unwrap();
    for name in ["pending-1.grit", "pending-1-0.grit"] {
        std::fs::create_dir_all(scratch.dir().join("kept").join(name)).unwrap();
    }
    let mut run = Run::new(&options);
    let world = sand_world(&mut run, 32);
    far_over(&mut run, &scratch, world);
    let began = Instant::now();
    while !kept_files(&scratch)
        .iter()
        .any(|one| one.starts_with("snap-"))
    {
        answer(&run.ask("grit_world_rules", "{}"));
        assert!(
            began.elapsed() < Duration::from_secs(20),
            "{:?}",
            kept_files(&scratch)
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let item = listed_world(&mut run, world);
    assert!(item.get("snapshot_failed").is_none(), "{}", written(&item));
    let files = kept_files(&scratch);
    assert!(files.contains(&"pending-1.grit".to_owned()), "{files:?}");
    let strays: Vec<&String> = files
        .iter()
        .filter(|one| {
            one.starts_with("pending-") && *one != "pending-1.grit" && *one != "pending-1-0.grit"
        })
        .collect();
    assert!(strays.is_empty(), "{files:?}");
    run.exec.finish();
}

#[cfg(feature = "faults")]
#[test]
fn a_device_lost_during_a_snapshot_between_calls_is_handled_as_a_loss() {
    let Some(host) = device("loss in idle snapshot") else {
        return;
    };
    drop(host);
    let scratch = Scratch::new("idle-loss");
    let mut run = Run::new(&Options {
        dir: Some(scratch.dir()),
        memory: None,
    });
    let world = sand_world(&mut run, 32);
    run.exec.faults().lock().unwrap().wedge_in_snapshot = true;
    let mut frame = 0;
    let mut saw = false;
    let began = Instant::now();
    loop {
        let got = run.ask(
            "grit_world_step",
            &format!("{{\"world\":{world},\"frames\":65536,\"budget_ms\":200}}"),
        );
        let result = got.get("result").unwrap();
        let told = result.get("structuredContent").unwrap();
        if result.get("isError") == Some(&Json::Bool(true)) {
            assert_eq!(
                told.get("code").and_then(Json::text),
                Some("budget"),
                "{}",
                written(told)
            );
        } else {
            frame = frame.max(told.int_at("frame"));
            saw |= told.get("restored").is_some();
        }
        let worlds = answer(&run.ask("grit_describe", "{}"))
            .get("worlds")
            .cloned()
            .unwrap();
        if worlds.int_at("restorations") == 1 {
            break;
        }
        assert!(
            began.elapsed() < Duration::from_secs(30),
            "{}",
            written(&worlds)
        );
    }
    let got = answer(&run.ask(
        "grit_world_read",
        &format!("{{\"world\":{world},\"from\":[0,31,0],\"to\":[15,31,15]}}"),
    ));
    saw |= got.get("restored").is_some();
    assert!(saw, "no answer said the world was restored");
    assert_eq!(got.int_at("frame"), frame);
    let worlds = answer(&run.ask("grit_describe", "{}"))
        .get("worlds")
        .cloned()
        .unwrap();
    assert_eq!(worlds.int_at("restorations"), 1);
    run.exec.finish();
}

fn step_within(run: &mut Run, world: i64, budget: u64) -> (Duration, i64) {
    let began = Instant::now();
    let got = run.ask(
        "grit_world_step",
        &format!("{{\"world\":{world},\"frames\":65536,\"budget_ms\":{budget}}}"),
    );
    let took = began.elapsed();
    let told = got
        .get("result")
        .and_then(|one| one.get("structuredContent"))
        .cloned()
        .unwrap();
    (took, told.get("frame").and_then(Json::int).unwrap_or(-1))
}

#[test]
fn a_cold_server_stepping_with_small_budgets_never_overruns_by_more_than_a_batch() {
    let Some(host) = device("cold small budgets") else {
        return;
    };
    drop(host);
    let scratch = Scratch::new("cold");
    let options = Options {
        dir: Some(scratch.dir()),
        memory: None,
    };
    let budget = 300;
    let mut worst = Duration::ZERO;
    let mut moved = 0;
    let mut run = Run::new(&options);
    let world = sand_world(&mut run, 128);
    for _ in 0..12 {
        let (took, frame) = step_within(&mut run, world, budget);
        worst = worst.max(took);
        moved += i64::from(frame > 0);
        let most = late_most(&mut run, budget);
        assert!(took <= most, "first run: {took:?}, more than {most:?}");
    }
    run.exec.finish();
    let mut again = Run::new(&options);
    answer(&again.ask(
        "grit_world_open",
        &format!("{{\"name\":\"world-{world}\"}}"),
    ));
    for _ in 0..8 {
        let (took, _) = step_within(&mut again, world, budget);
        worst = worst.max(took);
        let most = late_most(&mut again, budget);
        assert!(
            took <= most,
            "after a restart: {took:?}, more than {most:?}"
        );
    }
    let small = sand_world(&mut again, 64);
    for _ in 0..8 {
        let (took, frame) = step_within(&mut again, small, budget);
        worst = worst.max(took);
        moved += i64::from(frame > 0);
        let most = late_most(&mut again, budget);
        assert!(took <= most, "a new size: {took:?}, more than {most:?}");
    }
    eprintln!("the slowest of 28 small-budget steps took {worst:?}; {moved} advanced");
    assert!(moved >= 10, "{moved}");
    again.exec.finish();
}
