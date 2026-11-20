#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

#[cfg(feature = "faults")]
mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use grit::serve3::cost::{Cost, Priced};
use grit::serve3::journal::{Entry, Keep, Ledger, Made, priced, replay_cost};
use grit::serve3::{branch, law, snapshot_cost, tools};

const MIB: u64 = 1 << 20;

const fn cost(kind: Priced, units: u64, submits: u32) -> Cost {
    Cost {
        kind,
        units,
        submits,
    }
}

fn made(size: [u32; 3], sparse: bool) -> Made {
    Made {
        rule: "sand".to_owned(),
        params: Vec::new(),
        size,
        sparse,
        slots: None,
    }
}

fn scratch(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir =
        std::env::temp_dir().join(format!("grit-units-{label}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn table() -> Vec<(&'static str, Cost, Cost)> {
    let dense = made([16, 16, 16], false);
    let sparse = made([32, 32, 32], true);
    let mut ledger = Ledger::created(dense.clone());
    ledger.push(Entry::Write(vec![([0, 0, 0], 1); 40]), Duration::ZERO);
    ledger.push(Entry::Step(100), Duration::ZERO);
    ledger.push(Entry::Params(vec![1, 2]), Duration::ZERO);
    ledger.push(Entry::Back(3), Duration::ZERO);
    vec![
        (
            "build 1 MiB",
            tools::build_cost(MIB),
            cost(Priced::Build, MIB, 1),
        ),
        (
            "write 0 dense",
            tools::write_cost(0, false),
            cost(Priced::Write, 0, 0),
        ),
        (
            "write 1 dense",
            tools::write_cost(1, false),
            cost(Priced::Write, 1, 1),
        ),
        (
            "write 32 dense",
            tools::write_cost(32, false),
            cost(Priced::Write, 32, 1),
        ),
        (
            "write 33 dense",
            tools::write_cost(33, false),
            cost(Priced::Write, 33, 2),
        ),
        (
            "write 96 dense",
            tools::write_cost(96, false),
            cost(Priced::Write, 96, 3),
        ),
        (
            "write 0 sparse",
            tools::write_cost(0, true),
            cost(Priced::Write, 0, 0),
        ),
        (
            "write 5000 sparse",
            tools::write_cost(5000, true),
            cost(Priced::Write, 5000, 1),
        ),
        (
            "frame cells dense 16^3",
            cost(Priced::Frame, tools::frame_cells([16, 16, 16], None), 0),
            cost(Priced::Frame, 4096, 0),
        ),
        (
            "frame cells sparse 14 bricks of 512",
            cost(
                Priced::Frame,
                tools::frame_cells([32, 32, 32], Some((14, 512))),
                0,
            ),
            cost(Priced::Frame, 7168, 0),
        ),
        (
            "0 frames",
            tools::frames_cost(4096, 0),
            cost(Priced::Frame, 0, 0),
        ),
        (
            "64 frames",
            tools::frames_cost(4096, 64),
            cost(Priced::Frame, 262_144, 1),
        ),
        (
            "65 frames",
            tools::frames_cost(4096, 65),
            cost(Priced::Frame, 266_240, 2),
        ),
        (
            "100 frames",
            tools::frames_cost(4096, 100),
            cost(Priced::Frame, 409_600, 2),
        ),
        ("params 3", law::params_cost(3), cost(Priced::Params, 12, 0)),
        (
            "fork 1 MiB",
            branch::fork_cost(MIB),
            cost(Priced::Fork, MIB, 0),
        ),
        (
            "restore 1 MiB",
            branch::restore_cost(MIB),
            cost(Priced::Restore, MIB, 0),
        ),
        (
            "save 1 MiB",
            branch::save_cost(MIB),
            cost(Priced::Save, MIB, 0),
        ),
        (
            "file 2236 bytes",
            branch::file_cost(2236),
            cost(Priced::File, 2236, 0),
        ),
        (
            "snapshot 1 MiB",
            snapshot_cost(MIB),
            cost(Priced::Snapshot, MIB, 0),
        ),
        (
            "dense step entry",
            priced(&dense, &Entry::Step(7), None),
            cost(Priced::Frame, 28_672, 1),
        ),
        (
            "dense back entry",
            priced(&dense, &Entry::Back(70), None),
            cost(Priced::Frame, 286_720, 2),
        ),
        (
            "sparse step entry with 14 bricks of 512",
            priced(&sparse, &Entry::Step(9), Some((14, 512))),
            cost(Priced::Frame, 64_512, 1),
        ),
        (
            "sparse step entry with bricks unknown",
            priced(&sparse, &Entry::Step(9), None),
            cost(Priced::Frame, 294_912, 1),
        ),
        (
            "sparse write entry",
            priced(&sparse, &Entry::Write(vec![([0, 0, 0], 1); 40]), None),
            cost(Priced::Write, 40, 1),
        ),
        (
            "params entry",
            priced(&dense, &Entry::Params(vec![1, 2]), None),
            cost(Priced::Params, 8, 0),
        ),
        (
            "replay of write 40, step 100, params 2, back 3",
            replay_cost(&ledger),
            cost(Priced::Replay, 40 + 409_600 + 8 + 12_288, 2 + 2 + 1),
        ),
        (
            "ledger units",
            cost(Priced::Replay, ledger.units(), 0),
            cost(Priced::Replay, 421_936, 0),
        ),
        (
            "ledger units from entry 2",
            cost(Priced::Replay, ledger.units_from(2), 0),
            cost(Priced::Replay, 12_296, 0),
        ),
    ]
}

#[test]
fn u1_every_work_function_gives_its_hand_computed_value_on_every_run() {
    let first = table();
    for (name, got, wanted) in &first {
        assert_eq!(got, wanted, "{name}");
    }
    std::thread::sleep(Duration::from_millis(30));
    let second = table();
    assert_eq!(
        first.iter().map(|(_, got, _)| *got).collect::<Vec<Cost>>(),
        second.iter().map(|(_, got, _)| *got).collect::<Vec<Cost>>()
    );
}

#[cfg(feature = "faults")]
#[test]
fn u1_the_work_functions_move_no_fake_time_and_read_no_clock() {
    use grit::clock::Fake;
    let fake = Fake::new();
    let before = (fake.elapsed(), fake.reads(), fake.slept());
    for (name, got, wanted) in table() {
        assert_eq!(got, wanted, "{name}");
    }
    assert_eq!((fake.elapsed(), fake.reads(), fake.slept()), before);
    assert_eq!(fake.elapsed(), Duration::ZERO);
}

struct Draws(u64);

impl Draws {
    fn next(&mut self, most: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % most
    }

    fn pair(&mut self, most: u64) -> (u64, u64) {
        let one = self.next(most);
        let other = self.next(most);
        (one.min(other), one.max(other))
    }
}

fn no_lower(name: &str, small: Cost, large: Cost) {
    assert!(
        small.units <= large.units && small.submits <= large.submits,
        "{name}: {small:?} prices more than {large:?}"
    );
}

#[test]
fn u2_more_cells_bytes_frames_or_bricks_never_price_lower() {
    let mut draws = Draws(0x9E37_79B9_7F4A_7C15);
    for _ in 0..4000 {
        let (less, more) = draws.pair(1 << 40);
        no_lower("build", tools::build_cost(less), tools::build_cost(more));
        for copy in [
            branch::fork_cost,
            branch::restore_cost,
            branch::save_cost,
            branch::file_cost,
            snapshot_cost,
        ] {
            no_lower("copy", copy(less), copy(more));
        }
        let (fewer, many) = draws.pair(1 << 20);
        let (fewer, many) = (
            usize::try_from(fewer).unwrap(),
            usize::try_from(many).unwrap(),
        );
        for sparse in [false, true] {
            no_lower(
                "write",
                tools::write_cost(fewer, sparse),
                tools::write_cost(many, sparse),
            );
        }
        no_lower("params", law::params_cost(fewer), law::params_cost(many));
        let (few_frames, many_frames) = draws.pair(70_000);
        let (few_frames, many_frames) = (
            u32::try_from(few_frames).unwrap(),
            u32::try_from(many_frames).unwrap(),
        );
        let (few_cells, many_cells) = draws.pair(1 << 30);
        no_lower(
            "frames",
            tools::frames_cost(few_cells, few_frames),
            tools::frames_cost(many_cells, many_frames),
        );
        let (few_bricks, many_bricks) = draws.pair(1 << 16);
        let per = usize::try_from(1 + draws.next(4096)).unwrap();
        let few = tools::frame_cells(
            [64, 64, 64],
            Some((u32::try_from(few_bricks).unwrap(), per)),
        );
        let lots = tools::frame_cells(
            [64, 64, 64],
            Some((u32::try_from(many_bricks).unwrap(), per)),
        );
        assert!(few <= lots, "{few_bricks} bricks against {many_bricks}");
        let side = |draw: u64| u32::try_from(1 + draw).unwrap();
        let small = [
            side(draws.next(256)),
            side(draws.next(256)),
            side(draws.next(256)),
        ];
        let grown = [
            small[0] + side(draws.next(64)) - 1,
            small[1],
            small[2] + side(draws.next(64)) - 1,
        ];
        assert!(tools::frame_cells(small, None) <= tools::frame_cells(grown, None));
        let mut short = Ledger::created(made([16, 16, 16], false));
        let entries = 1 + draws.next(12);
        for at in 0..entries {
            let entry = match draws.next(3) {
                0 => Entry::Write(vec![([0, 0, 0], 1); 1 + (at % 40) as usize]),
                1 => Entry::Step(u32::try_from(draws.next(300)).unwrap()),
                _ => Entry::Params(vec![0; 1 + (at % 3) as usize]),
            };
            short.push(entry, Duration::ZERO);
        }
        let mut long = short.clone();
        long.push(
            Entry::Step(u32::try_from(1 + draws.next(300)).unwrap()),
            Duration::ZERO,
        );
        no_lower("replay", replay_cost(&short), replay_cost(&long));
        assert!(short.units() <= long.units());
        let from = usize::try_from(draws.next(entries + 1)).unwrap();
        assert!(long.units_from(from + 1) <= long.units_from(from));
    }
}

#[test]
fn the_journal_hash_does_not_depend_on_units() {
    let mut plain = Ledger::created(made([16, 16, 16], false));
    let mut priced_apart = plain.clone();
    plain.push(Entry::Step(10), Duration::from_millis(4));
    priced_apart.push_priced(
        Entry::Step(10),
        Duration::from_millis(4),
        cost(Priced::Frame, 1, 99),
    );
    assert_ne!(plain.costs(), priced_apart.costs());
    assert_eq!(plain.journal(), priced_apart.journal());
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("journal-v1")
}

fn copied_fixture(label: &str) -> PathBuf {
    let dir = scratch(label);
    let kept = dir.join("kept");
    std::fs::create_dir_all(&kept).unwrap();
    for entry in std::fs::read_dir(fixture()).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), kept.join(entry.file_name())).unwrap();
    }
    dir
}

#[cfg(feature = "faults")]
const MAIN_STATES: [(u32, i64, Option<&str>); 4] = [
    (
        1,
        107,
        Some("ce4c0ac4667246c78b02556869b14d77e178924fbdf639ce9fe12f1a9ac1bc7c"),
    ),
    (2, 39, None),
    (
        3,
        13,
        Some("cb2e901ee3eccd2bc2213fc0188b543815cd064df03a0879c1bbc18be9a94dcd"),
    ),
    (
        4,
        7,
        Some("016c9b7c7bca3115e3f2df91fdc301ec1009d7ca34f875ee7299d7347a84cfa8"),
    ),
];

fn pure_costs(ledger: &Ledger) -> Vec<Cost> {
    ledger
        .entries
        .iter()
        .map(|entry| priced(ledger.made(), entry, None))
        .collect()
}

#[test]
fn u4_a_journal_written_by_main_reads_with_units_from_the_pure_functions() {
    let dir = copied_fixture("v1-read");
    let found = Keep::at(&dir).unwrap().earlier();
    assert_eq!(
        found.iter().map(|(number, _)| *number).collect::<Vec<_>>(),
        [1, 2, 3, 4]
    );
    for (number, ledger) in &found {
        assert!(ledger.unreadable.is_none(), "world {number}");
        assert!(!ledger.entries.is_empty(), "world {number}");
        assert_eq!(ledger.costs(), pure_costs(ledger), "world {number}");
    }
    let sparse = &found[1].1;
    assert!(sparse.made().sparse);
    let frames: Vec<u32> = sparse
        .entries
        .iter()
        .filter_map(|entry| match entry {
            Entry::Step(frames) => Some(*frames),
            _ => None,
        })
        .collect();
    assert_eq!(frames, [30, 9]);
    let stepped: Vec<Cost> = sparse
        .entries
        .iter()
        .zip(sparse.costs())
        .filter(|(entry, _)| matches!(entry, Entry::Step(_)))
        .map(|(_, cost)| *cost)
        .collect();
    assert_eq!(
        stepped,
        [
            cost(Priced::Frame, 32 * 32 * 32 * 30, 1),
            cost(Priced::Frame, 32 * 32 * 32 * 9, 1),
        ]
    );
    let snapped = &found[2].1;
    assert!(snapped.sum().is_some(), "world 3 starts from a snapshot");
    let lawful = &found[3].1;
    assert!(lawful.law.is_some());
    assert!(lawful.entries.contains(&Entry::Params(vec![2, 1])));
    assert!(lawful.entries.contains(&Entry::Back(3)));
    assert_eq!(
        replay_cost(lawful),
        lawful
            .costs()
            .iter()
            .fold(Cost::none(Priced::Replay), |all, one| all.plus(*one))
    );
}

fn flipped_at(path: &Path, at: usize) {
    let mut bytes = std::fs::read(path).unwrap();
    bytes[at] ^= 0x10;
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn a_damaged_units_word_is_refused_like_any_damaged_record() {
    let entries = [
        Entry::Write(vec![([1, 2, 3], 4), ([5, 6, 7], 8)]),
        Entry::Step(64),
        Entry::Step(3),
    ];
    let mut ledger = Ledger::created(made([16, 16, 16], false));
    for entry in &entries {
        ledger.push(entry.clone(), Duration::from_micros(700));
    }
    let mut outcomes = Vec::new();
    for (label, from_end) in [("units", 16), ("submits", 8), ("micros", 20)] {
        let dir = scratch(&format!("damaged-{label}"));
        let keep = Keep::at(&dir).unwrap();
        keep.begin(1, &ledger).unwrap();
        let path = dir.join("kept").join("world-1.journal");
        let whole = std::fs::read(&path).unwrap();
        let read = keep.earlier();
        assert_eq!(read[0].1.entries, entries);
        assert_eq!(read[0].1.costs(), ledger.costs());
        flipped_at(&path, whole.len() - from_end);
        let read = keep.earlier();
        assert_eq!(read.len(), 1, "{label}");
        assert!(read[0].1.unreadable.is_none(), "{label}");
        outcomes.push((
            read[0].1.entries.clone(),
            read[0].1.costs().to_vec(),
            read[0].1.frame,
        ));
    }
    assert_eq!(outcomes[0].0, entries[..2]);
    assert_eq!(outcomes[0].1, ledger.costs()[..2]);
    assert_eq!(outcomes[0].2, 64);
    assert_eq!(outcomes[0], outcomes[1]);
    assert_eq!(outcomes[0], outcomes[2]);
}

fn checked(tag: u32, payload: &[u32]) -> u32 {
    let mut check = 0x811C_9DC5_u32;
    let length = u32::try_from(payload.len()).unwrap();
    for word in [tag, length].iter().chain(payload) {
        for byte in word.to_le_bytes() {
            check ^= u32::from(byte);
            check = check.wrapping_mul(0x0100_0193);
        }
    }
    check
}

fn appended_record(path: &Path, tag: u32, payload: &[u32]) {
    let mut bytes = std::fs::read(path).unwrap();
    let length = u32::try_from(payload.len()).unwrap();
    for word in [tag, length]
        .iter()
        .chain(payload)
        .chain(std::iter::once(&checked(tag, payload)))
    {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn a_record_without_its_units_words_or_in_an_unknown_format_is_refused_by_name() {
    let mut ledger = Ledger::created(made([8, 8, 8], false));
    ledger.push(Entry::Step(5), Duration::from_millis(1));
    let dir = scratch("short-units");
    let keep = Keep::at(&dir).unwrap();
    keep.begin(1, &ledger).unwrap();
    let path = dir.join("kept").join("world-1.journal");
    appended_record(&path, 4 | 256, &[5, 1000, 20]);
    let read = keep.earlier();
    let why = read[0].1.unreadable.clone().unwrap();
    assert!(why.contains("record 2"), "{why}");
    assert_eq!(read[0].1.entries, [Entry::Step(5)]);

    let fixture_dir = copied_fixture("v1-priced");
    let path = fixture_dir.join("kept").join("world-1.journal");
    appended_record(&path, 4 | 256, &[5, 1000, 20, 0, 1]);
    let read = Keep::at(&fixture_dir).unwrap().earlier();
    let why = read[0].1.unreadable.clone().unwrap();
    assert!(why.contains("tag 260"), "{why}");

    let dir = scratch("format-3");
    let keep = Keep::at(&dir).unwrap();
    keep.begin(1, &ledger).unwrap();
    let path = dir.join("kept").join("world-1.journal");
    let bytes = std::fs::read(&path).unwrap();
    let base_words = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    let at = 4 * (3 + base_words);
    assert_eq!(
        u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()),
        8,
        "the format record follows the base"
    );
    let mut changed = bytes[..at].to_vec();
    for word in [8_u32, 1, 3, checked(8, &[3])] {
        changed.extend_from_slice(&word.to_le_bytes());
    }
    changed.extend_from_slice(&bytes[at + 16..]);
    std::fs::write(&path, changed).unwrap();
    let read = keep.earlier();
    let why = read[0].1.unreadable.clone().unwrap();
    assert!(why.contains("format 3"), "{why}");
}

#[cfg(feature = "faults")]
mod served {
    use std::sync::Arc;
    use std::time::Duration;

    use grit::Host;
    use grit::clock::{Fake, Time};
    use grit::serve3::cost::{Cost, Priced};
    use grit::serve3::journal::{Entry, Keep, Ledger, Made, priced, replay_cost};
    use grit::serve3::tools::faults;
    use grit::serve3::{Options, Server};
    use grit::wire::{Json, written};

    use super::{MAIN_STATES, MIB, copied_fixture, cost, pure_costs, scratch};

    use super::common;

    const MIX: &str = include_str!("law/mix.law.json");

    fn call(server: &mut Server<'_>, name: &str, arguments: &str) -> Json {
        let line = format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{{\"name\":\"{name}\",\"arguments\":{arguments}}}}}"
        );
        let got = server.heard(&line).unwrap();
        assert!(
            got.get("result").and_then(|result| result.get("isError")) != Some(&Json::Bool(true)),
            "{name}: {}",
            written(&got)
        );
        let told = got
            .get("result")
            .and_then(|result| result.get("structuredContent"))
            .cloned()
            .unwrap();
        assert!(told.get("stopped").is_none(), "{name}: {}", written(&told));
        told
    }

    fn int_at(held: &Json, key: &str) -> i64 {
        held.get(key)
            .and_then(Json::int)
            .unwrap_or_else(|| panic!("no {key} in {}", written(held)))
    }

    fn held_bricks(held: &Json) -> u32 {
        let used = held
            .get("pool_use")
            .unwrap_or_else(|| panic!("no pool_use in {}", written(held)));
        u32::try_from(int_at(used, "held")).unwrap()
    }

    fn cells(world: i64, count: u32, side: u32, seed: u32) -> String {
        let listed: Vec<String> = (0..count)
            .map(|at| {
                let mixed = at.wrapping_mul(2_654_435_761).wrapping_add(seed);
                format!(
                    "[{},{},{},{}]",
                    mixed % side,
                    (mixed / side) % side,
                    (mixed / (side * side)) % side,
                    1 + (mixed >> 20) % 3
                )
            })
            .collect();
        format!(
            "{{\"world\":{world},\"cells\":[{}],\"budget_ms\":600000}}",
            listed.join(",")
        )
    }

    fn create(server: &mut Server<'_>, arguments: &str) -> Json {
        call(server, "grit_world_create", arguments)
    }

    fn dense(server: &mut Server<'_>, side: u32) -> i64 {
        int_at(
            &create(
                server,
                &format!(
                    "{{\"wide\":{side},\"deep\":{side},\"tall\":{side},\"rule\":\"sand\",\"params\":[],\"budget_ms\":600000}}"
                ),
            ),
            "world",
        )
    }

    fn sparse(server: &mut Server<'_>) -> (i64, u32) {
        let told = create(
            server,
            "{\"wide\":32,\"deep\":32,\"tall\":32,\"rule\":\"sand\",\"params\":[],\"sparse\":true,\"budget_ms\":600000}",
        );
        (int_at(&told, "world"), held_bricks(&told))
    }

    fn lawful(server: &mut Server<'_>) -> i64 {
        int_at(
            &create(
                server,
                &format!(
                    "{{\"wide\":8,\"deep\":8,\"tall\":8,\"law\":{},\"params\":[1,2],\"budget_ms\":600000}}",
                    written(&Json::Text(MIX.to_owned()))
                ),
            ),
            "world",
        )
    }

    fn step(server: &mut Server<'_>, world: i64, frames: u32) -> Json {
        call(
            server,
            "grit_world_step",
            &format!("{{\"world\":{world},\"frames\":{frames},\"budget_ms\":600000}}"),
        )
    }

    fn back(server: &mut Server<'_>, world: i64, frames: u32) -> Json {
        call(
            server,
            "grit_world_back",
            &format!("{{\"world\":{world},\"frames\":{frames},\"budget_ms\":600000}}"),
        )
    }

    fn params(server: &mut Server<'_>, world: i64) -> Json {
        call(
            server,
            "grit_world_params",
            &format!("{{\"world\":{world},\"params\":[2,1],\"budget_ms\":600000}}"),
        )
    }

    fn brick_cells(host: &Host) -> usize {
        let made = Made {
            rule: "sand".to_owned(),
            params: Vec::new(),
            size: [32, 32, 32],
            sparse: true,
            slots: None,
        };
        grit::branch::World::create(host, &made.branch())
            .unwrap()
            .pool()
            .unwrap()
            .cells()
    }

    fn timed<'host>(host: &'host Host, options: &Options, fake: &Arc<Fake>) -> Server<'host> {
        let time: Arc<dyn Time> = Arc::clone(fake) as Arc<dyn Time>;
        Server::timed(Some(host), options, time).unwrap()
    }

    fn kept(dir: &std::path::Path) -> Options {
        Options {
            dir: Some(dir.to_path_buf()),
            memory: None,
        }
    }

    fn tallies(server: &Server<'_>) -> Vec<Cost> {
        Priced::ALL.iter().map(|kind| server.tally(*kind)).collect()
    }

    struct Session {
        tally: Vec<Cost>,
        ledgers: Vec<Vec<Cost>>,
        bricks: (u32, u32),
    }

    fn session(host: &Host, fake: &Arc<Fake>) -> Session {
        let dir = scratch("session");
        let mut server = timed(host, &kept(&dir), fake);
        let per = u64::try_from(brick_cells(host)).unwrap();
        let world = dense(&mut server, 16);
        assert_eq!(server.tally(Priced::Build), cost(Priced::Build, MIB, 1));
        call(&mut server, "grit_world_write", &cells(world, 40, 16, 1));
        assert_eq!(server.tally(Priced::Write), cost(Priced::Write, 40, 2));
        step(&mut server, world, 100);
        assert_eq!(server.tally(Priced::Frame), cost(Priced::Frame, 409_600, 2));
        let fork = int_at(
            &call(
                &mut server,
                "grit_world_fork",
                &format!("{{\"world\":{world},\"budget_ms\":600000}}"),
            ),
            "world",
        );
        assert_eq!(server.tally(Priced::Fork), cost(Priced::Fork, MIB, 0));
        call(
            &mut server,
            "grit_world_restore",
            &format!("{{\"world\":{world},\"from\":{fork},\"budget_ms\":600000}}"),
        );
        assert_eq!(server.tally(Priced::Restore), cost(Priced::Restore, MIB, 0));
        let saved = call(
            &mut server,
            "grit_world_save",
            &format!("{{\"world\":{world},\"name\":\"units\",\"budget_ms\":600000}}"),
        );
        assert_eq!(server.tally(Priced::Save), cost(Priced::Save, MIB, 0));
        let file = u64::try_from(int_at(&saved, "bytes")).unwrap();
        call(
            &mut server,
            "grit_world_open",
            "{\"name\":\"units\",\"budget_ms\":600000}",
        );
        assert_eq!(server.tally(Priced::File), cost(Priced::File, file, 0));
        assert_eq!(server.tally(Priced::Build), cost(Priced::Build, 2 * MIB, 2));
        let (bricked, created_bricks) = sparse(&mut server);
        assert_eq!(created_bricks, 0);
        assert_eq!(server.tally(Priced::Build), cost(Priced::Build, 3 * MIB, 3));
        let written_to = call(&mut server, "grit_world_write", &cells(bricked, 20, 32, 3));
        assert_eq!(server.tally(Priced::Write), cost(Priced::Write, 60, 3));
        let first = held_bricks(&written_to);
        assert!(first > 0);
        let after = step(&mut server, bricked, 30);
        let second = held_bricks(&after);
        step(&mut server, bricked, 9);
        let bricked_units = (u64::from(first) * 30 + u64::from(second) * 9) * per;
        assert_eq!(
            server.tally(Priced::Frame),
            cost(Priced::Frame, 409_600 + bricked_units, 4)
        );
        let law_world = lawful(&mut server);
        params(&mut server, law_world);
        assert_eq!(server.tally(Priced::Params), cost(Priced::Params, 8, 0));
        step(&mut server, law_world, 6);
        back(&mut server, law_world, 3);
        assert_eq!(
            server.tally(Priced::Frame),
            cost(Priced::Frame, 409_600 + bricked_units + 512 * 9, 6)
        );
        assert_eq!(server.tally(Priced::Snapshot), Cost::none(Priced::Snapshot));
        assert_eq!(server.tally(Priced::Replay), Cost::none(Priced::Replay));
        let ledgers = [world, fork, bricked, law_world]
            .iter()
            .map(|number| {
                server
                    .ledger(u32::try_from(*number).unwrap())
                    .unwrap()
                    .costs()
                    .to_vec()
            })
            .collect();
        Session {
            tally: tallies(&server),
            ledgers,
            bricks: (first, second),
        }
    }

    fn script() -> Vec<Duration> {
        let mut state = 0x2545_f491_4f6c_dd1d_u64;
        (0..100_000)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                Duration::from_micros(50 + state % 950)
            })
            .collect()
    }

    #[test]
    fn u1_each_call_is_priced_exactly_and_alike_on_a_frozen_and_a_moving_clock() {
        let Some(host) = common::device("u1 priced calls") else {
            return;
        };
        let frozen = Arc::new(Fake::new());
        let first = session(&host, &frozen);
        let moving = Arc::new(Fake::new());
        moving.script(script());
        let second = session(&host, &moving);
        assert!(moving.elapsed() > Duration::ZERO);
        assert_eq!(frozen.elapsed(), Duration::ZERO);
        assert_eq!(first.bricks, second.bricks);
        assert_eq!(first.tally, second.tally);
        assert_eq!(first.ledgers, second.ledgers);
    }

    fn stalled_step(server: &mut Server<'_>, world: i64, frames: u32) {
        faults::stall_every(Duration::from_secs(3));
        step(server, world, frames);
        faults::disarm();
    }

    fn agree(live: &Ledger, read: &Ledger, label: &str) {
        assert_eq!(live.entries, read.entries, "{label}");
        assert_eq!(live.frame, read.frame, "{label}");
        assert_eq!(live.sum(), read.sum(), "{label}");
        assert_eq!(live.costs(), read.costs(), "{label}");
        assert_eq!(live.units(), read.units(), "{label}");
    }

    #[test]
    fn u3_the_ledger_and_the_journal_agree_on_units_through_a_rebase_and_a_reopen() {
        let Some(host) = common::device("u3 ledger and journal") else {
            return;
        };
        let dir = scratch("u3");
        let fake = Arc::new(Fake::new());
        let mut server = timed(&host, &kept(&dir), &fake);
        let per = brick_cells(&host);
        let law_world = lawful(&mut server);
        call(&mut server, "grit_world_write", &cells(law_world, 30, 8, 6));
        step(&mut server, law_world, 6);
        params(&mut server, law_world);
        step(&mut server, law_world, 4);
        back(&mut server, law_world, 3);
        let fork = int_at(
            &call(
                &mut server,
                "grit_world_fork",
                &format!("{{\"world\":{law_world},\"budget_ms\":600000}}"),
            ),
            "world",
        );
        step(&mut server, fork, 2);
        let (bricked, _) = sparse(&mut server);
        let written_to = call(&mut server, "grit_world_write", &cells(bricked, 20, 32, 3));
        let first = held_bricks(&written_to);
        let after = step(&mut server, bricked, 30);
        let second = held_bricks(&after);
        step(&mut server, bricked, 9);
        let snapped = dense(&mut server, 8);
        call(&mut server, "grit_world_write", &cells(snapped, 30, 8, 4));
        stalled_step(&mut server, snapped, 2);
        let mut book = server.into_book();
        book.drain();
        let mut server = Server::resumed(Some(&host), book).0;
        step(&mut server, snapped, 5);
        call(&mut server, "grit_world_write", &cells(snapped, 2, 8, 5));
        step(&mut server, snapped, 6);
        let numbers = [law_world, fork, bricked, snapped].map(|one| u32::try_from(one).unwrap());
        let live: Vec<Ledger> = numbers
            .iter()
            .map(|number| server.ledger(*number).unwrap().clone())
            .collect();
        assert!(
            live[3].sum().is_some(),
            "the stalled step rebased world {snapped}"
        );
        assert!(
            live[3].entries.len() < 6,
            "the rebase dropped the entries before the snapshot: {:?}",
            live[3].entries
        );
        let read = Keep::at(&dir).unwrap().earlier();
        let mut wanted = numbers.to_vec();
        wanted.sort_unstable();
        assert_eq!(
            read.iter().map(|(number, _)| *number).collect::<Vec<_>>(),
            wanted
        );
        for (number, ledger) in numbers.iter().zip(&live) {
            let found = &read.iter().find(|(one, _)| one == number).unwrap().1;
            agree(ledger, found, &format!("world {number}"));
            if *number != numbers[2] {
                assert_eq!(ledger.costs(), pure_costs(ledger), "world {number}");
            }
        }
        let sparse_costs = live[2].costs().to_vec();
        let sparse_entries = &live[2].entries;
        assert_eq!(
            sparse_entries[1..],
            [Entry::Step(30), Entry::Step(9)],
            "{sparse_entries:?}"
        );
        assert_eq!(
            sparse_costs[0],
            priced(live[2].made(), &sparse_entries[0], None)
        );
        assert_eq!(
            sparse_costs[1],
            priced(live[2].made(), &sparse_entries[1], Some((first, per)))
        );
        assert_eq!(
            sparse_costs[2],
            priced(live[2].made(), &sparse_entries[2], Some((second, per)))
        );
        assert!(sparse_costs[1].units < priced(live[2].made(), &sparse_entries[1], None).units);
        let mut book = server.into_book();
        book.drain();
        drop(book);
        let mut again = timed(&host, &kept(&dir), &fake);
        let before = again.tally(Priced::Replay);
        assert_eq!(before, Cost::none(Priced::Replay));
        let mut replayed = Cost::none(Priced::Replay);
        for (number, ledger) in numbers.iter().zip(&live) {
            call(
                &mut again,
                "grit_world_open",
                &format!("{{\"name\":\"world-{number}\",\"budget_ms\":600000}}"),
            );
            replayed = replayed.plus(replay_cost(ledger));
            agree(
                ledger,
                again.ledger(*number).unwrap(),
                &format!("reopened {number}"),
            );
        }
        assert_eq!(again.tally(Priced::Replay), replayed);
        let reread = Keep::at(&dir).unwrap().earlier();
        for (number, ledger) in numbers.iter().zip(&live) {
            let found = &reread.iter().find(|(one, _)| one == number).unwrap().1;
            agree(ledger, found, &format!("rewritten {number}"));
        }
    }

    const MAIN_SPARSE_TALLY: [i64; 7] = [30, 10, 5, 12, 14, 14, 15];

    fn sparse_on_this_device(server: &mut Server<'_>) -> String {
        let (world, _) = sparse(server);
        call(server, "grit_world_write", &cells(world, 20, 32, 3));
        step(server, world, 30);
        let last = step(server, world, 9);
        let tally: Vec<i64> = last
            .get("tally")
            .and_then(Json::list)
            .unwrap()
            .iter()
            .map(|one| one.int().unwrap())
            .collect();
        assert_eq!(tally, MAIN_SPARSE_TALLY);
        let saved = call(
            server,
            "grit_world_save",
            &format!("{{\"world\":{world},\"name\":\"live-sparse\",\"budget_ms\":600000}}"),
        );
        assert_eq!(int_at(&saved, "frame"), 39);
        saved.get("state").and_then(Json::text).unwrap().to_owned()
    }

    #[test]
    fn u4_a_journal_written_by_main_reopens_bit_for_bit() {
        let Some(host) = common::device("u4 main journal") else {
            return;
        };
        let dir = copied_fixture("v1-open");
        let read = Keep::at(&dir).unwrap().earlier();
        let per = u64::try_from(brick_cells(&host)).unwrap();
        let mut server = Server::with(Some(&host), &kept(&dir)).unwrap();
        for ((number, frame, state), (found, ledger)) in MAIN_STATES.iter().zip(&read) {
            assert_eq!(number, found);
            let opened = call(
                &mut server,
                "grit_world_open",
                &format!("{{\"name\":\"world-{number}\",\"budget_ms\":600000}}"),
            );
            assert_eq!(int_at(&opened, "frame"), *frame, "world {number}");
            assert_eq!(server.ledger(*number).unwrap().costs(), ledger.costs());
            if *number == 2 {
                let held = u64::from(held_bricks(&opened));
                let dense_bound = ledger
                    .costs()
                    .iter()
                    .zip(&ledger.entries)
                    .filter(|(_, entry)| matches!(entry, Entry::Step(_)))
                    .map(|(one, _)| one.units)
                    .sum::<u64>();
                assert_eq!(dense_bound, 32 * 32 * 32 * 39);
                assert!(held * per * 39 <= dense_bound, "{held} bricks of {per}");
            }
            let saved = call(
                &mut server,
                "grit_world_save",
                &format!("{{\"world\":{number},\"name\":\"main-{number}\",\"budget_ms\":600000}}"),
            );
            let state = state.map_or_else(|| sparse_on_this_device(&mut server), str::to_owned);
            assert_eq!(
                saved.get("state").and_then(Json::text),
                Some(state.as_str()),
                "world {number} differs from the run on main"
            );
            assert_eq!(int_at(&saved, "frame"), *frame);
        }
        let mut book = server.into_book();
        book.drain();
        drop(book);
        let rewritten = Keep::at(&dir).unwrap().earlier();
        for ((number, ledger), (_, again)) in read.iter().zip(&rewritten) {
            assert_eq!(ledger.entries, again.entries, "world {number}");
            assert_eq!(ledger.costs(), again.costs(), "world {number}");
            assert_eq!(again.costs(), pure_costs(again), "world {number}");
        }
    }

    #[test]
    fn review_r1_a_sparse_step_right_after_a_rebuild() {
        let Some(host) = common::device("review r1") else {
            return;
        };
        let per = brick_cells(&host);
        let mut units = Vec::new();
        for rebuilt in [false, true] {
            let dir = scratch(if rebuilt { "review-r1b" } else { "review-r1a" });
            let fake = Arc::new(Fake::new());
            let mut server = timed(&host, &kept(&dir), &fake);
            let (world, _) = sparse(&mut server);
            let after = call(&mut server, "grit_world_write", &cells(world, 20, 32, 3));
            let held = held_bricks(&after);
            let number = u32::try_from(world).unwrap();
            if rebuilt {
                let book = server.into_book();
                server = Server::resumed(Some(&host), book).0;
            }
            let told = step(&mut server, world, 30);
            let ledger = server.ledger(number).unwrap().clone();
            let last = *ledger.costs().last().unwrap();
            eprintln!(
                "R1 rebuilt={rebuilt} per={per} held_before={held} last={last:?} restored={:?} tally={:?}",
                told.get("restored").map(written),
                told.get("tally").map(written)
            );
            let read = Keep::at(&dir).unwrap().earlier();
            eprintln!("R1 journal costs={:?}", read[0].1.costs());
            units.push(last.units);
        }
        assert_eq!(
            units[0], units[1],
            "the same step of the same world state has one price"
        );
    }
}
