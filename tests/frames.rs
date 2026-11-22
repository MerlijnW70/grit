#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use grit::serve3::cost::{Cost, Priced};
use grit::serve3::journal::{Base, Entry, Keep, Ledger, Made, Snap, priced};
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
