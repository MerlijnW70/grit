#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

#[cfg(feature = "faults")]
mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use grit::calibrate::Rate;
use grit::serve3::cost::{Cost, Priced, Tally};
use grit::serve3::journal::{Base, Entry, Keep, Ledger, Made, Snap, priced, replay_cost};
use grit::serve3::tools;

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
    let dir = std::env::temp_dir().join(format!(
        "grit-frames-{label}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn v2_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("journal-v2")
}

struct Kept {
    number: u32,
    base: Base,
    law: Option<Arc<str>>,
    entries: Vec<(Entry, Duration, Cost)>,
    begun: usize,
}

fn micros(count: u64) -> Duration {
    Duration::from_micros(count)
}

fn every_kind() -> Vec<Kept> {
    let dense = made([16, 16, 16], false);
    let sparse = made([32, 32, 32], true);
    let large = made([256, 256, 256], false);
    let pure = |made: &Made, entry: Entry, spent: u64| {
        let cost = priced(made, &entry, None);
        (entry, micros(spent), cost)
    };
    vec![
        Kept {
            number: 1,
            base: Base::Created(dense.clone()),
            law: None,
            entries: vec![
                pure(&dense, Entry::Write(vec![([1, 2, 3], 4); 40]), 1_250),
                pure(
                    &dense,
                    Entry::WritePlane(1, vec![([5, 6, 7], 8), ([9, 10, 11], 12), ([0, 0, 1], 3)]),
                    310,
                ),
                pure(&dense, Entry::Step(100), 9_000),
                pure(&dense, Entry::Params(vec![1, 2]), 20),
                pure(&dense, Entry::Back(3), 700),
                pure(&dense, Entry::Step(1), 90),
                pure(&dense, Entry::Step(65), 4_100),
            ],
            begun: 4,
        },
        Kept {
            number: 2,
            base: Base::Created(sparse.clone()),
            law: Some(Arc::from("{\"law\":\"fixture\"}")),
            entries: vec![
                pure(&sparse, Entry::Write(vec![([3, 2, 1], 2); 20]), 800),
                (
                    Entry::Step(30),
                    micros(2_000),
                    priced(&sparse, &Entry::Step(30), Some((14, 512))),
                ),
                (
                    Entry::Step(9),
                    micros(600),
                    Cost {
                        units: 12_345,
                        submits: 1,
                        ..Cost::none(Priced::Frame)
                    },
                ),
                (
                    Entry::Back(4),
                    micros(300),
                    priced(&sparse, &Entry::Back(4), Some((15, 512))),
                ),
            ],
            begun: 1,
        },
        Kept {
            number: 3,
            base: Base::Created(large.clone()),
            law: None,
            entries: vec![
                pure(&large, Entry::Step(70_000), 4_000_000),
                pure(&large, Entry::Back(69_999), 3_900_000),
            ],
            begun: 2,
        },
        Kept {
            number: 4,
            base: Base::Snapshot(Arc::new(Snap {
                sum: [7; 32],
                frame: 50,
                made: dense.clone(),
                image: None,
            })),
            law: None,
            entries: vec![
                pure(&dense, Entry::Step(64), 1_000),
                pure(&dense, Entry::Write(vec![([4, 4, 4], 1); 33]), 500),
                pure(&dense, Entry::Params(vec![9]), 10),
                (Entry::Step(2), micros(40), tools::frames_cost(4_096, 2)),
            ],
            begun: 0,
        },
    ]
}

fn ledger_of(kept: &Kept, upto: usize) -> Ledger {
    let mut ledger = match &kept.base {
        Base::Created(made) => Ledger::created(made.clone()),
        Base::Snapshot(snap) => Ledger::snapped(Arc::clone(snap)),
    }
    .with_law(kept.law.clone());
    for (entry, spent, cost) in kept.entries.iter().take(upto) {
        ledger.push_priced(entry.clone(), *spent, *cost);
    }
    ledger
}

fn written(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let keep = Keep::at(dir).unwrap();
    every_kind()
        .iter()
        .map(|kept| {
            keep.begin(kept.number, &ledger_of(kept, kept.begun))
                .unwrap();
            for (entry, spent, cost) in kept.entries.iter().skip(kept.begun) {
                keep.append_priced(kept.number, entry, *spent, *cost)
                    .unwrap();
            }
            let name = format!("world-{}.journal", kept.number);
            let bytes = std::fs::read(dir.join("kept").join(&name)).unwrap();
            (name, bytes)
        })
        .collect()
}

#[test]
fn f2_a_ledger_of_every_entry_kind_writes_the_bytes_of_the_format_2_fixture() {
    let dir = scratch("f2");
    let files = written(&dir);
    assert_eq!(files.len(), 4);
    for (name, bytes) in &files {
        let fixture = std::fs::read(v2_fixture().join(name)).unwrap_or_else(|why| {
            panic!(
                "{name}: {why}; the bytes this run wrote are in {}",
                dir.display()
            )
        });
        assert!(
            *bytes == fixture,
            "{name} differs from the fixture; the bytes this run wrote are in {}",
            dir.display()
        );
    }
    let again = written(&scratch("f2-again"));
    assert_eq!(files, again);
}

fn v1_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("journal-v1")
}

fn read_copy(fixture: &Path, label: &str) -> Vec<(u32, Ledger)> {
    let dir = scratch(label);
    let kept = dir.join("kept");
    std::fs::create_dir_all(&kept).unwrap();
    for entry in std::fs::read_dir(fixture).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), kept.join(entry.file_name())).unwrap();
    }
    Keep::at(&dir).unwrap().earlier()
}

fn counted(entry: &Entry) -> u32 {
    match entry {
        Entry::Step(count) | Entry::Back(count) => *count,
        Entry::Write(_) | Entry::WritePlane(..) | Entry::Params(_) | Entry::Clear { .. } => 0,
    }
}

fn stepped_or_walked(ledger: &Ledger) -> usize {
    ledger
        .entries
        .iter()
        .filter(|entry| matches!(entry, Entry::Step(_) | Entry::Back(_)))
        .count()
}

#[test]
fn f3_a_journal_of_either_format_reads_frames_from_its_own_entries() {
    let main = read_copy(&v1_fixture(), "f3-v1");
    assert_eq!(main.len(), 4);
    let mut moved = 0;
    for (number, ledger) in &main {
        assert!(ledger.unreadable.is_none(), "world {number}");
        assert_eq!(ledger.entries.len(), ledger.costs().len());
        for (entry, cost) in ledger.entries.iter().zip(ledger.costs()) {
            assert_eq!(cost.frames, counted(entry), "world {number}: {entry:?}");
            assert_eq!(
                *cost,
                priced(ledger.made(), entry, None),
                "world {number}: {entry:?}"
            );
        }
        moved += stepped_or_walked(ledger);
    }
    assert!(
        moved >= 4,
        "the format 1 fixture holds {moved} steps and walks"
    );

    let read = read_copy(&v2_fixture(), "f3-v2");
    let kept = every_kind();
    assert_eq!(read.len(), kept.len());
    let mut moved = 0;
    for ((number, ledger), wanted) in read.iter().zip(&kept) {
        assert_eq!(*number, wanted.number);
        assert!(ledger.unreadable.is_none(), "world {number}");
        let entries: Vec<Entry> = wanted
            .entries
            .iter()
            .map(|(entry, _, _)| entry.clone())
            .collect();
        assert_eq!(ledger.entries, entries, "world {number}");
        for ((entry, _, written), cost) in wanted.entries.iter().zip(ledger.costs()) {
            assert_eq!(cost.frames, counted(entry), "world {number}: {entry:?}");
            assert_eq!(
                (cost.kind, cost.units, cost.submits),
                (written.kind, written.units, written.submits),
                "world {number}: {entry:?}"
            );
        }
        moved += stepped_or_walked(ledger);
    }
    assert!(
        moved >= 8,
        "the format 2 fixture holds {moved} steps and walks"
    );
}

#[test]
fn f3_a_ledger_keeps_the_frames_of_its_entry_and_not_of_the_cost_it_was_given() {
    let given = |kind: Priced| Cost {
        units: 123,
        frames: 7,
        submits: 4,
        ..Cost::none(kind)
    };
    let cases = [
        (Entry::Step(10), Priced::Frame, 10),
        (Entry::Back(5), Priced::Frame, 5),
        (Entry::Step(0), Priced::Frame, 0),
        (Entry::Write(vec![([1, 1, 1], 1); 3]), Priced::Write, 0),
        (Entry::WritePlane(1, vec![([2, 2, 2], 1)]), Priced::Write, 0),
        (Entry::Params(vec![1, 2]), Priced::Params, 0),
    ];
    let mut ledger = Ledger::created(made([16, 16, 16], false));
    for (entry, kind, frames) in cases.iter().cloned() {
        ledger.push_priced(entry.clone(), Duration::from_micros(5), given(kind));
        let held = *ledger.costs().last().unwrap();
        assert_eq!(
            held,
            Cost {
                frames,
                ..given(kind)
            },
            "{entry:?}"
        );
    }
    let mut pure = Ledger::created(made([16, 16, 16], false));
    for (entry, _, frames) in cases {
        pure.push(entry.clone(), Duration::ZERO);
        assert_eq!(pure.costs().last().unwrap().frames, frames, "{entry:?}");
    }
}

struct Draws(u64);

impl Draws {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, most: u64) -> u64 {
        self.next() % most.max(1)
    }

    fn many(&mut self, most: u64) -> usize {
        usize::try_from(1 + self.below(most)).unwrap()
    }

    fn count(&mut self) -> u32 {
        match self.below(4) {
            0 => u32::MAX - u32::try_from(self.below(3)).unwrap(),
            1 => u32::try_from(self.below(1 << 31)).unwrap(),
            _ => u32::try_from(self.below(300)).unwrap(),
        }
    }

    fn wide(&mut self) -> u64 {
        match self.below(4) {
            0 => u64::MAX - self.below(3),
            1 => 0,
            _ => self.next(),
        }
    }
}

fn kind_at(at: u64) -> Priced {
    Priced::ALL[usize::try_from(at).unwrap() % Priced::ALL.len()]
}

#[test]
fn f4_the_replay_cost_and_the_tally_carry_the_saturating_sum_of_frames() {
    let mut draws = Draws(0x51_7CC1_B727_220A);
    let mut saturated = 0;
    for round in 0..2000 {
        let mut ledger = Ledger::created(made([16, 16, 16], false));
        let mut sum = 0_u32;
        for _ in 0..draws.below(12) {
            let entry = match draws.below(5) {
                0 => Entry::Step(draws.count()),
                1 => Entry::Back(draws.count()),
                2 => Entry::Write(vec![([0, 0, 0], 1); draws.many(40)]),
                3 => Entry::WritePlane(1, vec![([0, 0, 0], 1); draws.many(4)]),
                _ => Entry::Params(vec![0; draws.many(3)]),
            };
            sum = sum.saturating_add(counted(&entry));
            if draws.below(2) == 0 {
                ledger.push(entry, Duration::ZERO);
            } else {
                let cost = Cost {
                    units: draws.wide(),
                    frames: draws.count(),
                    submits: draws.count(),
                    ..Cost::none(kind_at(draws.below(10)))
                };
                ledger.push_priced(entry, Duration::ZERO, cost);
            }
        }
        if sum == u32::MAX {
            saturated += 1;
        }
        let replayed = replay_cost(&ledger);
        assert_eq!(replayed.kind, Priced::Replay, "round {round}");
        assert_eq!(replayed.frames, sum, "round {round}: {:?}", ledger.entries);

        let mut tally = Tally::default();
        let mut sums = [0_u32; Priced::ALL.len()];
        for _ in 0..draws.below(16) {
            let at = draws.below(10);
            let cost = Cost {
                units: draws.wide(),
                frames: draws.count(),
                submits: draws.count(),
                ..Cost::none(kind_at(at))
            };
            let index = usize::try_from(at).unwrap() % Priced::ALL.len();
            sums[index] = sums[index].saturating_add(cost.frames);
            tally.add(cost);
        }
        for (kind, sum) in Priced::ALL.iter().zip(sums) {
            assert_eq!(tally.of(*kind).frames, sum, "round {round}: {kind:?}");
            assert_eq!(tally.of(*kind).kind, *kind);
        }
    }
    assert!(saturated > 20, "only {saturated} ledgers reached u32::MAX");
}

fn floor_formula(rate: &Rate, units: u64, frames: u32, submits: u32) -> u128 {
    let unit_part = u128::from(units) * u128::from(rate.per_unit_fs) / 1_000_000;
    let submit_part = u128::from(submits) * u128::from(rate.per_submit_ns);
    let frame_part = if frames < submits {
        0
    } else {
        u128::from(frames - submits) * u128::from(rate.per_frame_ns)
    };
    unit_part
        .saturating_add(submit_part)
        .saturating_add(frame_part)
}

#[test]
fn f5_one_rate_function_prices_a_cost_by_the_floor_formula_for_every_kind() {
    let mut draws = Draws(0x2545_F491_4F6C_DD1D);
    let edges = [0_u64, 1, 999_999, 1_000_000, u64::MAX - 1, u64::MAX];
    let counts = [0_u32, 1, 2, 63, 64, 65, u32::MAX - 1, u32::MAX];
    let mut cases = Vec::new();
    for one in edges {
        for frames in counts {
            for submits in counts {
                cases.push((
                    Rate {
                        per_unit_fs: one,
                        per_frame_ns: one,
                        per_submit_ns: one,
                    },
                    one,
                    frames,
                    submits,
                ));
            }
        }
    }
    for _ in 0..20_000 {
        cases.push((
            Rate {
                per_unit_fs: draws.wide(),
                per_frame_ns: draws.wide(),
                per_submit_ns: draws.wide(),
            },
            draws.wide(),
            draws.count(),
            draws.count(),
        ));
    }
    let mut capped = 0;
    let mut fewer = 0;
    for (rate, units, frames, submits) in cases {
        let wanted = floor_formula(&rate, units, frames, submits);
        if wanted > u128::from(u64::MAX) {
            capped += 1;
        }
        if frames < submits {
            fewer += 1;
        }
        for kind in Priced::ALL {
            let cost = Cost {
                kind,
                units,
                frames,
                submits,
            };
            assert_eq!(rate.nanos(&cost), wanted, "{rate:?} {cost:?}");
            assert_eq!(
                rate.predict(&cost),
                Duration::from_nanos(u64::try_from(wanted).unwrap_or(u64::MAX)),
                "{rate:?} {cost:?}"
            );
        }
    }
    assert!(capped > 100, "only {capped} predictions above u64::MAX ns");
    assert!(
        fewer > 100,
        "only {fewer} cases with fewer frames than submits"
    );
    let still = Rate {
        per_unit_fs: 0,
        per_frame_ns: 6_155,
        per_submit_ns: 6_700,
    };
    let step = |frames: u32, submits: u32| Cost {
        units: 1 << 40,
        frames,
        submits,
        ..Cost::none(Priced::Frame)
    };
    assert_eq!(still.nanos(&step(1, 1)), 6_700);
    assert_eq!(still.nanos(&step(64, 2)), 2 * 6_700 + 62 * 6_155);
    assert_eq!(still.nanos(&step(1, 3)), 3 * 6_700);
}

#[cfg(feature = "faults")]
mod served {
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
    use std::time::Duration;

    use grit::Host;
    use grit::calibrate::{self, Key, Rate, Sample, Table, held};
    use grit::clock::{Fake, Time};
    use grit::serve3::budget::BATCH_MOST;
    use grit::serve3::cost::{Cost, Priced};
    use grit::serve3::journal::Entry;
    use grit::serve3::tools::faults;
    use grit::serve3::{Book, Options, Server};
    use grit::wire::{Json, written};

    use super::{common, scratch};

    const MIX: &str = include_str!("law/mix.law.json");

    static ALONE: Mutex<()> = Mutex::new(());

    fn alone() -> MutexGuard<'static, ()> {
        ALONE.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn line(name: &str, arguments: &str) -> String {
        format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{{\"name\":\"{name}\",\"arguments\":{arguments}}}}}"
        )
    }

    fn call(server: &mut Server<'_>, name: &str, arguments: &str) -> Json {
        server.heard(&line(name, arguments)).unwrap()
    }

    fn failed(got: &Json) -> bool {
        got.get("result").and_then(|result| result.get("isError")) == Some(&Json::Bool(true))
    }

    fn told(got: &Json) -> Json {
        got.get("result")
            .and_then(|result| result.get("structuredContent"))
            .cloned()
            .unwrap_or_else(|| panic!("no structured answer in {}", written(got)))
    }

    fn fine(server: &mut Server<'_>, name: &str, arguments: &str) -> Json {
        let got = call(server, name, arguments);
        assert!(!failed(&got), "{name}: {}", written(&got));
        told(&got)
    }

    fn int_at(held: &Json, key: &str) -> i64 {
        held.get(key)
            .and_then(Json::int)
            .unwrap_or_else(|| panic!("no {key} in {}", written(held)))
    }

    fn frame_of(held: &Json) -> u32 {
        u32::try_from(int_at(held, "frame")).unwrap()
    }

    fn timed<'host>(host: &'host Host, options: &Options, fake: &Arc<Fake>) -> Server<'host> {
        let time: Arc<dyn Time> = Arc::clone(fake) as Arc<dyn Time>;
        Server::timed(Some(host), options, time).unwrap()
    }

    const fn plain() -> Options {
        Options {
            dir: None,
            memory: None,
        }
    }

    fn kept(dir: &std::path::Path) -> Options {
        Options {
            dir: Some(dir.to_path_buf()),
            memory: None,
        }
    }

    fn sand(server: &mut Server<'_>, side: u32) -> i64 {
        int_at(
            &fine(
                server,
                "grit_world_create",
                &format!(
                    "{{\"wide\":{side},\"deep\":{side},\"tall\":{side},\"rule\":\"sand\",\"params\":[],\"budget_ms\":600000}}"
                ),
            ),
            "world",
        )
    }

    fn lawful(server: &mut Server<'_>) -> i64 {
        int_at(
            &fine(
                server,
                "grit_world_create",
                &format!(
                    "{{\"wide\":8,\"deep\":8,\"tall\":8,\"law\":{},\"params\":[1,2],\"budget_ms\":600000}}",
                    written(&Json::Text(MIX.to_owned()))
                ),
            ),
            "world",
        )
    }

    fn cells(world: i64, count: u32, side: u32) -> String {
        let listed: Vec<String> = (0..count)
            .map(|at| {
                let mixed = at.wrapping_mul(2_654_435_761).wrapping_add(17);
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

    fn moved(world: i64, frames: u32, budget: u64) -> String {
        format!("{{\"world\":{world},\"frames\":{frames},\"budget_ms\":{budget}}}")
    }

    fn tallied(server: &Server<'_>) -> Vec<Cost> {
        Priced::ALL.iter().map(|kind| server.tally(*kind)).collect()
    }

    struct Journaled {
        ran: u32,
        entries: usize,
        stopped: Option<String>,
        answer: Json,
    }

    #[derive(Clone, Copy)]
    enum Way {
        Step,
        Back,
    }

    fn ran_in_refusal(message: &str) -> u32 {
        let (before, _) = message
            .split_once(" of the ")
            .unwrap_or_else(|| panic!("no count of frames run in {message:?}"));
        before
            .rsplit(|one: char| !one.is_ascii_digit())
            .next()
            .unwrap()
            .parse()
            .unwrap()
    }

    fn journaled(
        server: &mut Server<'_>,
        world: i64,
        way: Way,
        frames: u32,
        budget: u64,
    ) -> Journaled {
        let number = u32::try_from(world).unwrap();
        let ledger = server.ledger(number).unwrap().clone();
        let before = ledger.entries.len();
        let frame = ledger.frame;
        let tally = tallied(server);
        let name = match way {
            Way::Step => "grit_world_step",
            Way::Back => "grit_world_back",
        };
        let got = call(server, name, &moved(world, frames, budget));
        let answer = told(&got);
        let ran = if failed(&got) {
            ran_in_refusal(answer.get("message").and_then(Json::text).unwrap())
        } else {
            let now = frame_of(&answer);
            match way {
                Way::Step => now - frame,
                Way::Back => frame - now,
            }
        };
        let after = server.ledger(number).unwrap().clone();
        assert_eq!(
            after.entries[..before],
            ledger.entries[..],
            "the call kept the entries before it"
        );
        let mut sum = 0_u32;
        for (entry, cost) in after.entries[before..].iter().zip(&after.costs()[before..]) {
            match (way, entry) {
                (Way::Step, Entry::Step(count)) | (Way::Back, Entry::Back(count)) => {
                    assert_eq!(cost.frames, *count, "{entry:?} {cost:?}");
                    assert_eq!(cost.kind, Priced::Frame);
                }
                _ => panic!("{name} journaled {entry:?}"),
            }
            sum += cost.frames;
        }
        assert_eq!(
            sum, ran,
            "{name}: the journaled frames are the frames that ran"
        );
        let now = tallied(server);
        for ((kind, was), is) in Priced::ALL.iter().zip(&tally).zip(&now) {
            let grew = if *kind == Priced::Frame { ran } else { 0 };
            assert_eq!(is.frames - was.frames, grew, "{name}: the {kind:?} tally");
        }
        Journaled {
            ran,
            entries: after.entries.len() - before,
            stopped: answer
                .get("stopped")
                .and_then(Json::text)
                .map(str::to_owned),
            answer,
        }
    }

    #[test]
    fn f1_every_step_and_walk_journals_and_tallies_the_frames_that_ran() {
        let Some(host) = common::device("f1 frames that ran") else {
            return;
        };
        let _alone = alone();
        let fake = Arc::new(Fake::new());
        let mut server = timed(&host, &plain(), &fake);
        let world = sand(&mut server, 16);

        let one = journaled(&mut server, world, Way::Step, 1, 600_000);
        assert_eq!((one.ran, one.entries), (1, 1));

        let long = journaled(&mut server, world, Way::Step, 3 * BATCH_MOST + 5, 600_000);
        assert_eq!((long.ran, long.entries), (3 * BATCH_MOST + 5, 1));

        faults::stall_every(Duration::from_millis(10));
        let cut = journaled(&mut server, world, Way::Step, 640, 1_000);
        faults::disarm();
        assert_eq!(
            cut.stopped.as_deref(),
            Some("budget"),
            "{}",
            written(&cut.answer)
        );
        assert!(cut.ran > 0 && cut.ran < 640, "the brake cut at {}", cut.ran);

        faults::fail_batch(2);
        let failing = journaled(&mut server, world, Way::Step, 400, 600_000);
        faults::disarm();
        assert!(
            failing.ran > 0 && failing.ran < 400,
            "the failed batch stopped at {}",
            failing.ran
        );

        let split = sand(&mut server, 8);
        server.faults().lock().unwrap().fail_snapshots = 1000;
        faults::stall_every(Duration::from_secs(1));
        let owed = journaled(&mut server, split, Way::Step, 12, 600_000);
        faults::disarm();
        server.faults().lock().unwrap().fail_snapshots = 0;
        assert!(
            owed.entries >= 2,
            "a step past its snapshot point journals more than once: {} entries, {}",
            owed.entries,
            written(&owed.answer)
        );

        let law = lawful(&mut server);
        journaled(&mut server, law, Way::Step, 300, 600_000);
        let walked = journaled(&mut server, law, Way::Back, 70, 600_000);
        assert_eq!((walked.ran, walked.entries), (70, 1));

        faults::stall_every(Duration::from_millis(10));
        let braked = journaled(&mut server, law, Way::Back, 200, 500);
        faults::disarm();
        assert_eq!(
            braked.stopped.as_deref(),
            Some("budget"),
            "{}",
            written(&braked.answer)
        );
        assert!(
            braked.ran > 0 && braked.ran < 200,
            "the brake cut the walk at {}",
            braked.ran
        );

        let number = u32::try_from(law).unwrap();
        let ledger = server.ledger(number).unwrap().clone();
        let tally = tallied(&server);
        faults::fail_batch(1);
        let got = call(&mut server, "grit_world_back", &moved(law, 20, 600_000));
        faults::disarm();
        assert!(failed(&got), "{}", written(&got));
        assert_eq!(server.ledger(number).unwrap().entries, ledger.entries);
        assert_eq!(server.ledger(number).unwrap().costs(), ledger.costs());
        assert_eq!(
            server.tally(Priced::Frame),
            tally[Priced::ALL
                .iter()
                .position(|kind| *kind == Priced::Frame)
                .unwrap()],
            "a failed walk tallies nothing"
        );
    }

    #[test]
    fn f1_calls_that_run_no_frame_carry_no_frames() {
        let Some(host) = common::device("f1 no frames") else {
            return;
        };
        let _alone = alone();
        let fake = Arc::new(Fake::new());
        let dir = scratch("f1-kept");
        let mut server = timed(&host, &kept(&dir), &fake);
        let world = sand(&mut server, 16);
        fine(&mut server, "grit_world_write", &cells(world, 40, 16));
        let fork = int_at(
            &fine(
                &mut server,
                "grit_world_fork",
                &format!("{{\"world\":{world},\"budget_ms\":600000}}"),
            ),
            "world",
        );
        fine(
            &mut server,
            "grit_world_restore",
            &format!("{{\"world\":{world},\"from\":{fork},\"budget_ms\":600000}}"),
        );
        fine(
            &mut server,
            "grit_world_save",
            &format!("{{\"world\":{world},\"name\":\"f1\",\"budget_ms\":600000}}"),
        );
        fine(
            &mut server,
            "grit_world_open",
            "{\"name\":\"f1\",\"budget_ms\":600000}",
        );
        let law = lawful(&mut server);
        fine(
            &mut server,
            "grit_world_params",
            &format!("{{\"world\":{law},\"params\":[2,1],\"budget_ms\":600000}}"),
        );
        for kind in Priced::ALL {
            assert_eq!(server.tally(kind).frames, 0, "{kind:?}");
        }
        for kind in [
            Priced::Build,
            Priced::Write,
            Priced::Fork,
            Priced::Restore,
            Priced::Save,
            Priced::File,
            Priced::Params,
        ] {
            assert!(server.tally(kind).units > 0, "{kind:?} ran");
        }

        let mut snapping = timed(&host, &plain(), &fake);
        let small = sand(&mut snapping, 8);
        faults::stall_every(Duration::from_secs(1));
        let stepped = fine(&mut snapping, "grit_world_step", &moved(small, 12, 600_000));
        faults::disarm();
        let ran = frame_of(&stepped);
        assert!(ran > 0, "{}", written(&stepped));
        assert!(
            snapping.tally(Priced::Snapshot).units > 0,
            "the step took a snapshot: {}",
            written(&stepped)
        );
        for kind in Priced::ALL {
            let wanted = if kind == Priced::Frame { ran } else { 0 };
            assert_eq!(snapping.tally(kind).frames, wanted, "{kind:?}");
        }
    }

    fn batches(server: &mut Server<'_>) -> u32 {
        let told = fine(server, "grit_describe", "{}");
        let submissions = told
            .get("submissions")
            .unwrap_or_else(|| panic!("no submissions in {}", written(&told)));
        u32::try_from(int_at(submissions, "batches")).unwrap()
    }

    #[test]
    fn f6_a_step_lowers_its_rate_by_the_sample_of_the_frames_and_batches_that_ran() {
        let Some(host) = common::device("f6 sample") else {
            return;
        };
        let _alone = alone();
        let key = Key::rule("sand", false);
        let first = Rate {
            per_unit_fs: 7_000_000,
            per_frame_ns: 1_000_000_000,
            per_submit_ns: 2_000_000_000,
        };
        let mut table = Table::new(calibrate::device(&host));
        table.set(key.clone(), first);
        let fake = Arc::new(Fake::new());
        let time: Arc<dyn Time> = Arc::clone(&fake) as Arc<dyn Time>;
        let book = Book::empty_timed(time).calibrated(held(table, None));
        let mut server = Server::resumed(Some(&host), book).0;
        let world = sand(&mut server, 32);
        let each = Duration::from_millis(5);
        let sound = calibrate::times(&host);
        let mut rate = first;
        let mut more = 0;
        for frames in [64_u32, 200, 1] {
            let before_frame = server.ledger(u32::try_from(world).unwrap()).unwrap().frame;
            let before = batches(&mut server);
            faults::stall_submission(each);
            calibrate::faults::script(std::iter::repeat_n(each, 4096));
            let told = fine(
                &mut server,
                "grit_world_step",
                &moved(world, frames, 600_000),
            );
            faults::disarm();
            calibrate::faults::script([]);
            let ran = frame_of(&told) - before_frame;
            assert_eq!(ran, frames);
            let run = batches(&mut server) - before;
            if run > ran.div_ceil(BATCH_MOST) {
                more += 1;
            }
            let sample = Sample {
                cost: Cost {
                    kind: Priced::Frame,
                    units: 32 * 32 * 32 * u64::from(ran),
                    frames: ran,
                    submits: run,
                },
                time: each * run,
            };
            for _ in 0..if sound { 2 } else { 1 } {
                rate = rate.lowered(sample).unwrap_or(rate);
            }
            assert_eq!(
                server.calibration().unwrap().rate(&key),
                Some(rate),
                "{frames} frames in {run} batches"
            );
        }
        assert_ne!(rate, first, "the steps lowered the rate");
        assert!(
            more >= 1,
            "a first step of 64 frames runs more batches than ceil(64 / {BATCH_MOST})"
        );
    }

    fn mix_key() -> Key {
        let law = grit::law::Law::compile(MIX).unwrap();
        Key::law(law.canonical(), false)
    }

    fn table_of(host: &Host, term: u64) -> Table {
        let mut table = Table::new(calibrate::device(host));
        for key in [
            Key::Build { sparse: false },
            Key::Write { sparse: false },
            Key::Fork,
            Key::Restore,
            Key::Snapshot { file: false },
            Key::Snapshot { file: true },
            Key::Save,
            Key::File,
            Key::Params,
            Key::rule("sand", false),
            mix_key(),
        ] {
            table.set(
                key,
                Rate {
                    per_unit_fs: term,
                    per_frame_ns: term,
                    per_submit_ns: term,
                },
            );
        }
        table
    }

    fn booked(options: &Options, fake: &Arc<Fake>, table: Option<&calibrate::Shared>) -> Book {
        let time: Arc<dyn Time> = Arc::clone(fake) as Arc<dyn Time>;
        let book = Book::timed(options, time).unwrap();
        match table {
            Some(shared) => book.calibrated(Arc::clone(shared)),
            None => book,
        }
    }

    fn session(host: &Host, label: &str, budget: u64, table: Option<Table>) -> Vec<String> {
        let dir: PathBuf = scratch(label);
        let unique = dir.file_name().unwrap().to_string_lossy().into_owned();
        let shared = table.map(|table| held(table, None));
        let fake = Arc::new(Fake::new());
        let options = kept(&dir);
        let mut server = Server::resumed(Some(host), booked(&options, &fake, shared.as_ref())).0;
        let mut said = Vec::new();
        let mut say = |server: &mut Server<'_>, name: &str, arguments: String| {
            let got = call(server, name, &arguments);
            said.push(written(&got).replace(&unique, "DIR"));
            told(&got)
        };
        let created = say(
            &mut server,
            "grit_world_create",
            format!(
                "{{\"wide\":8,\"deep\":8,\"tall\":8,\"rule\":\"sand\",\"params\":[],\"budget_ms\":{budget}}}"
            ),
        );
        let world = int_at(&created, "world");
        say(
            &mut server,
            "grit_world_write",
            cells(world, 40, 8).replace("600000", &budget.to_string()),
        );
        say(&mut server, "grit_world_step", moved(world, 100, budget));
        let fork = int_at(
            &say(
                &mut server,
                "grit_world_fork",
                format!("{{\"world\":{world},\"budget_ms\":{budget}}}"),
            ),
            "world",
        );
        say(&mut server, "grit_world_step", moved(fork, 7, budget));
        say(
            &mut server,
            "grit_world_restore",
            format!("{{\"world\":{world},\"from\":{fork},\"budget_ms\":{budget}}}"),
        );
        let law = int_at(
            &say(
                &mut server,
                "grit_world_create",
                format!(
                    "{{\"wide\":8,\"deep\":8,\"tall\":8,\"law\":{},\"params\":[1,2],\"budget_ms\":{budget}}}",
                    written(&Json::Text(MIX.to_owned()))
                ),
            ),
            "world",
        );
        say(&mut server, "grit_world_step", moved(law, 10, budget));
        say(&mut server, "grit_world_back", moved(law, 4, budget));
        say(
            &mut server,
            "grit_world_save",
            format!("{{\"world\":{world},\"name\":\"n1\",\"budget_ms\":{budget}}}"),
        );
        say(&mut server, "grit_describe", "{}".to_owned());
        let mut book = server.into_book();
        book.drain();
        drop(book);
        let mut again = Server::resumed(Some(host), booked(&options, &fake, shared.as_ref())).0;
        let opened = say(
            &mut again,
            "grit_world_open",
            format!("{{\"name\":\"world-{world}\",\"budget_ms\":{budget}}}"),
        );
        assert!(
            int_at(&opened, "replayed") > 0,
            "the reopen replays: {}",
            written(&opened)
        );
        say(
            &mut again,
            "grit_world_open",
            format!("{{\"name\":\"n1\",\"budget_ms\":{budget}}}"),
        );
        say(&mut again, "grit_describe", "{}".to_owned());
        assert_eq!(fake.elapsed(), Duration::ZERO);
        said
    }

    #[test]
    fn n1_no_rate_in_a_table_moves_an_answer() {
        let Some(host) = common::device("n1 no rate moves an answer") else {
            return;
        };
        let _alone = alone();
        for budget in [1_u64, 600_000] {
            let mut runs = Vec::new();
            for (name, table) in [
                ("none", None),
                ("ones", Some(table_of(&host, 1))),
                ("most", Some(table_of(&host, u64::MAX))),
            ] {
                let said = session(&host, &format!("n1-{name}-{budget}"), budget, table);
                let refused: Vec<&String> = said
                    .iter()
                    .filter(|answer| answer.contains("\"isError\":true"))
                    .collect();
                if budget == 600_000 {
                    assert!(refused.is_empty(), "{name}: {refused:?}");
                }
                eprintln!(
                    "N1 {name} at {budget} ms: {} answers, {} refused",
                    said.len(),
                    refused.len()
                );
                runs.push((name, said));
            }
            let (first_name, first) = &runs[0];
            for (name, said) in &runs[1..] {
                assert_eq!(said.len(), first.len());
                for (one, other) in first.iter().zip(said) {
                    assert_eq!(
                        one, other,
                        "at {budget} ms the {name} table differs from {first_name}"
                    );
                }
            }
        }
    }
}
