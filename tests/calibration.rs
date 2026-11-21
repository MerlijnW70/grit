#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

#[cfg(feature = "faults")]
mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use grit::calibrate::{Device, FILE, Key, Moved, Rate, Sample, Table, load};

fn device_named(name: &str) -> Device {
    Device {
        name: name.to_owned(),
        driver: 7,
        width: 4,
    }
}

fn every_key() -> Vec<Key> {
    let mut keys = vec![
        Key::Fork,
        Key::Restore,
        Key::Snapshot { file: false },
        Key::Snapshot { file: true },
        Key::Save,
        Key::File,
        Key::Params,
        Key::law("{\"law\":\"one\"}", false),
        Key::law("{\"law\":\"two\"}", true),
    ];
    for sparse in [false, true] {
        keys.push(Key::Build { sparse });
        keys.push(Key::Write { sparse });
        keys.push(Key::rule("sand", sparse));
        keys.push(Key::rule("heat", sparse));
    }
    keys
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, most: u64) -> u64 {
        self.next() % most.max(1)
    }
}

fn scratch(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "grit-calibration-{label}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn random_table(rng: &mut Rng, device: Device) -> Table {
    let mut table = Table::new(device);
    for key in every_key() {
        if rng.below(4) == 0 {
            continue;
        }
        table.set(
            key,
            Rate {
                per_unit_fs: 1 + rng.below(1 << 40),
                per_submit_ns: rng.below(1 << 32),
            },
        );
    }
    table
}

#[test]
fn every_key_names_itself_back() {
    for key in every_key() {
        assert_eq!(Key::parse(&key.name()), Some(key.clone()), "{}", key.name());
    }
    for wrong in [
        "",
        "build",
        "build/thin",
        "frame/dense/rule/",
        "frame/dense/law/123",
        "snapshot/disk",
        "fork/dense",
    ] {
        assert_eq!(Key::parse(wrong), None, "{wrong}");
    }
}

#[test]
fn three_runs_solve_to_floor_terms() {
    let rate = Rate {
        per_unit_fs: 2_500,
        per_submit_ns: 40_000,
    };
    let at = |units: u64, submits: u32| Sample {
        units,
        submits,
        time: rate.predict(units, submits),
    };
    let small = at(262_144 * 8, 1);
    let large = at(2_097_152 * 8, 1);
    let short = at(262_144, 1);
    let solved = Rate::floor(small, large, short).unwrap();
    assert!(solved.per_unit_fs.abs_diff(2_500) <= 1, "{solved:?}");
    assert!(solved.per_submit_ns.abs_diff(40_000) <= 1, "{solved:?}");
    for point in [small, large, short] {
        assert!(solved.predict(point.units, point.submits) <= point.time);
    }
    let per_frame = Duration::from_micros(12);
    let framed = |units: u64, frames: u32| Sample {
        units: units * u64::from(frames),
        submits: 1,
        time: rate.predict(units * u64::from(frames), 1) + per_frame * (frames - 1),
    };
    let solved = Rate::floor(framed(262_144, 8), framed(2_097_152, 8), framed(262_144, 1)).unwrap();
    assert!(solved.per_unit_fs.abs_diff(2_500) <= 1, "{solved:?}");
    assert!(
        solved.per_submit_ns.abs_diff(40_000) <= 1,
        "the fixed term comes from the one-frame run: {solved:?}"
    );
    assert!(
        solved.predict(2_097_152 * 64, 1) < framed(2_097_152, 64).time,
        "a long step is under-predicted by its extra frame overheads"
    );
    let flat = Sample {
        units: 2_097_152,
        submits: 1,
        time: Duration::from_micros(10),
    };
    let wide = Sample {
        units: 262_144,
        submits: 1,
        time: Duration::from_micros(11),
    };
    let unresolved = Rate::floor(wide, flat, wide).unwrap();
    assert_eq!(
        unresolved.per_unit_fs, 0,
        "a per-unit term the sizes do not resolve is stored as zero"
    );
    assert!(unresolved.per_submit_ns > 0);
    let same = Rate::floor(flat, flat, flat).unwrap();
    assert_eq!(
        same.per_unit_fs, 0,
        "two sizes with the same units resolve no per-unit term"
    );
    assert!(same.predict(flat.units, flat.submits) <= flat.time);
    assert_eq!(
        Rate::floor(at(1, 1), at(512, 16), at(1, 1)),
        None,
        "the two sizes must have the same submits"
    );
}

#[test]
fn c2_scripted_samples_never_raise_a_rate() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let keys = every_key();
    let mut table = Table::new(device_named("scripted"));
    for key in &keys {
        table.set(
            key.clone(),
            Rate {
                per_unit_fs: 5_000_000,
                per_submit_ns: 100_000,
            },
        );
    }
    let mut lowered = 0;
    for round in 0..4000 {
        let key = &keys[usize::try_from(rng.below(keys.len() as u64)).unwrap()];
        let sample = Sample {
            units: rng.below(1 << 30),
            submits: u32::try_from(rng.below(100)).unwrap(),
            time: Duration::from_nanos(rng.below(10_000_000_000)),
        };
        let sound = rng.below(2) == 0;
        let before = table.rate(key).unwrap();
        let moved = table.lower(key, sample, sound);
        let after = table.rate(key).unwrap();
        assert!(
            after.per_unit_fs <= before.per_unit_fs && after.per_submit_ns <= before.per_submit_ns,
            "round {round}: {before:?} rose to {after:?} on {sample:?}"
        );
        for units in [1_u64, 1 << 10, 1 << 20, 1 << 30, 1 << 40] {
            for submits in [0_u32, 1, 64] {
                assert!(after.predict(units, submits) <= before.predict(units, submits));
            }
        }
        match moved {
            Moved::Lowered => {
                lowered += 1;
                assert!(sample.valid(), "round {round}");
                assert!(
                    after.predict(sample.units, sample.submits) <= sample.time,
                    "round {round}: lowered to {after:?}, still above {sample:?}"
                );
            }
            Moved::Unchanged => assert_eq!(before, after, "round {round}"),
            Moved::First => panic!("round {round}: a known rate was set again"),
        }
    }
    assert!(
        lowered > 40,
        "only {lowered} of 4000 samples lowered a rate"
    );
    let mut empty = Table::new(device_named("unknown"));
    let key = Key::rule("sand", false);
    let sample = Sample {
        units: 1 << 20,
        submits: 1,
        time: Duration::from_millis(3),
    };
    assert_eq!(empty.lower(&key, sample, false), Moved::Unchanged);
    assert_eq!(empty.rate(&key), None, "wall time never sets a first rate");
    for invalid in [
        Sample { units: 0, ..sample },
        Sample {
            time: Duration::ZERO,
            ..sample
        },
    ] {
        assert_eq!(empty.lower(&key, invalid, true), Moved::Unchanged);
        assert_eq!(empty.rate(&key), None);
    }
    assert_eq!(empty.lower(&key, sample, true), Moved::First);
    let first = empty.rate(&key).unwrap();
    assert!(first.predict(sample.units, sample.submits) <= sample.time);
    let slower = Sample {
        time: Duration::from_millis(30),
        ..sample
    };
    assert_eq!(empty.lower(&key, slower, true), Moved::Unchanged);
    assert_eq!(empty.rate(&key), Some(first));
}

#[test]
fn c3_a_table_written_and_read_back_is_equal() {
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    let dir = scratch("roundtrip");
    for round in 0..200 {
        let device = Device {
            name: format!("device \"{round}\" \\ ü"),
            driver: u32::try_from(rng.below(u64::from(u32::MAX))).unwrap(),
            width: u32::try_from(rng.below(64)).unwrap(),
        };
        let table = random_table(&mut rng, device.clone());
        assert_eq!(Table::read(&table.written()), Ok(table.clone()));
        table.save(&dir).unwrap();
        assert_eq!(load(&dir, &device), Ok(Some(table)), "round {round}");
    }
    assert_eq!(load(&scratch("absent"), &device_named("absent")), Ok(None));
}

fn refused(dir: &Path, device: &Device, wanted: &str) {
    let why = load(dir, device).unwrap_err();
    assert!(why.starts_with(FILE), "{why}");
    assert!(why.contains(wanted), "{why} lacks {wanted:?}");
    assert!(why.contains("calibrated again"), "{why}");
}

#[test]
fn c3_a_damaged_or_foreign_file_is_refused_by_name() {
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    let device = device_named("refused");
    let mut table = random_table(&mut rng, device.clone());
    table.set(
        Key::rule("sand", false),
        Rate {
            per_unit_fs: 123_456,
            per_submit_ns: 7_890,
        },
    );
    let whole = table.written();
    let dir = scratch("refused");
    let path = dir.join(FILE);
    std::fs::write(&path, &whole[..whole.len() / 2]).unwrap();
    refused(&dir, &device, "is not JSON");
    std::fs::write(&path, b"\x00\xff garbage").unwrap();
    refused(&dir, &device, "is not JSON");
    std::fs::write(&path, whole.replace("123456", "123457")).unwrap();
    refused(&dir, &device, "checksum");
    std::fs::write(
        &path,
        whole.replace("\"calibration\":1", "\"calibration\":2"),
    )
    .unwrap();
    refused(&dir, &device, "version 2");
    std::fs::write(&path, whole.replace("\"calibration\":1,", "")).unwrap();
    refused(&dir, &device, "no calibration version");
    let other = random_table(&mut rng, device_named("another card"));
    other.save(&dir).unwrap();
    refused(&dir, &device, "another device (another card");
    table.save(&dir).unwrap();
    assert_eq!(load(&dir, &device), Ok(Some(table)));
}

#[cfg(feature = "faults")]
mod device {
    use std::sync::{Arc, Mutex, PoisonError};
    use std::time::{Duration, Instant};

    use grit::calibrate::{
        self, Device, FILE, Key, Rate, Table, faults, held, inject, load, read, shared, still,
        taken,
    };
    use grit::clock::{Fake, Time};
    use grit::serve3::{Book, Server};
    use grit::wire::{Json, written};

    use super::{common, scratch};

    static ONE: Mutex<()> = Mutex::new(());

    fn alone() -> std::sync::MutexGuard<'static, ()> {
        ONE.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn rate_of(server: &Server<'_>, key: &Key) -> Option<Rate> {
        server.calibration().and_then(|table| table.rate(key))
    }

    fn call(server: &mut Server<'_>, name: &str, arguments: &str) -> Json {
        let line = format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{{\"name\":\"{name}\",\"arguments\":{arguments}}}}}"
        );
        let got = server.heard(&line).unwrap();
        let told = got
            .get("result")
            .and_then(|result| result.get("structuredContent"))
            .cloned()
            .unwrap();
        assert!(told.get("stopped").is_none(), "{name}: {}", written(&told));
        told
    }

    fn world_of(told: &Json) -> i64 {
        told.get("world").and_then(Json::int).unwrap()
    }

    #[test]
    fn c2_a_server_never_raises_a_rate_and_skips_fallback_prices() {
        let Some(host) = common::device("c2 server") else {
            return;
        };
        let _alone = alone();
        let dense = Key::rule("sand", false);
        let sparse = Key::rule("sand", true);
        let high = Rate {
            per_unit_fs: 1_000_000_000_000,
            per_submit_ns: 1_000_000_000,
        };
        let mut table = Table::new(calibrate::device(&host));
        table.set(dense.clone(), high);
        table.set(sparse.clone(), high);
        let fake = Arc::new(Fake::new());
        let time: Arc<dyn Time> = Arc::clone(&fake) as Arc<dyn Time>;
        let book = Book::empty_timed(time).calibrated(held(table, None));
        let mut server = Server::resumed(Some(&host), book).0;
        let world = world_of(&call(
            &mut server,
            "grit_world_create",
            "{\"wide\":32,\"deep\":32,\"tall\":32,\"rule\":\"sand\",\"params\":[],\"budget_ms\":600000}",
        ));
        let step = format!("{{\"world\":{world},\"frames\":64,\"budget_ms\":600000}}");
        let mut last = rate_of(&server, &dense).unwrap();
        let mut lowered = 0;
        let script = [
            (Duration::from_secs(9), Duration::from_secs(9)),
            (Duration::from_millis(400), Duration::from_secs(5)),
            (Duration::from_secs(20), Duration::from_secs(30)),
            (Duration::from_secs(7), Duration::from_millis(90)),
            (Duration::from_secs(50), Duration::from_secs(50)),
            (Duration::from_millis(30), Duration::from_millis(2)),
            (Duration::from_secs(3), Duration::ZERO),
        ];
        for (round, (wall, sound)) in script.into_iter().enumerate() {
            fake.script(std::iter::repeat_n(wall / 8, 64));
            faults::script(std::iter::repeat_n(sound / 8, 64));
            call(&mut server, "grit_world_step", &step);
            let now = rate_of(&server, &dense).unwrap();
            assert!(
                now.per_unit_fs <= last.per_unit_fs && now.per_submit_ns <= last.per_submit_ns,
                "round {round}: {last:?} rose to {now:?}"
            );
            if now != last {
                lowered += 1;
            }
            last = now;
        }
        faults::script([]);
        assert!(lowered >= 3, "only {lowered} steps lowered the rate");
        let told = call(
            &mut server,
            "grit_world_create",
            "{\"wide\":32,\"deep\":32,\"tall\":32,\"rule\":\"sand\",\"params\":[],\"sparse\":true,\"budget_ms\":600000}",
        );
        let other = world_of(&told);
        call(
            &mut server,
            "grit_world_write",
            &format!(
                "{{\"world\":{other},\"cells\":[[1,1,1,1],[20,20,20,1]],\"budget_ms\":600000}}"
            ),
        );
        let before = rate_of(&server, &sparse).unwrap();
        server.faults().lock().unwrap().forget_bricks = true;
        fake.script(std::iter::repeat_n(Duration::from_micros(1), 64));
        faults::script(std::iter::repeat_n(Duration::from_nanos(1), 64));
        call(
            &mut server,
            "grit_world_step",
            &format!("{{\"world\":{other},\"frames\":4,\"budget_ms\":600000}}"),
        );
        faults::script([]);
        assert_eq!(
            rate_of(&server, &sparse),
            Some(before),
            "a step priced at the dense count because its bricks were unknown moved the rate"
        );
        fake.script(std::iter::repeat_n(Duration::from_micros(1), 64));
        faults::script(std::iter::repeat_n(Duration::from_nanos(1), 64));
        call(
            &mut server,
            "grit_world_step",
            &format!("{{\"world\":{other},\"frames\":4,\"budget_ms\":600000}}"),
        );
        faults::script([]);
        assert!(
            rate_of(&server, &sparse).unwrap().per_unit_fs < before.per_unit_fs,
            "the same step with its bricks known is a sample and lowers the rate"
        );
        let heat = Key::rule("heat", false);
        let warm = world_of(&call(
            &mut server,
            "grit_world_create",
            "{\"wide\":16,\"deep\":16,\"tall\":16,\"rule\":\"heat\",\"params\":[0,0,0],\"budget_ms\":600000}",
        ));
        fake.script(std::iter::repeat_n(Duration::from_secs(1), 64));
        faults::script(std::iter::repeat_n(Duration::from_secs(5), 64));
        call(
            &mut server,
            "grit_world_step",
            &format!("{{\"world\":{warm},\"frames\":4,\"budget_ms\":600000}}"),
        );
        faults::script([]);
        assert_eq!(
            rate_of(&server, &heat),
            None,
            "a finished step never sets a first rate, however its time was measured"
        );
    }

    #[test]
    fn c3_a_table_of_another_device_is_never_used_on_resume() {
        let Some(host) = common::device("c3 other device") else {
            return;
        };
        let _alone = alone();
        let mine = calibrate::device(&host);
        calibrate::forget(&mine);
        let other = Device {
            name: format!("{} elsewhere", mine.name),
            ..mine.clone()
        };
        let old = Rate {
            per_unit_fs: 7,
            per_submit_ns: 7,
        };
        let mut foreign = Table::new(other);
        foreign.set(Key::rule("sand", false), old);
        let book = Book::empty().calibrated(held(foreign.clone(), None));
        let server = Server::resumed(Some(&host), book).0;
        assert_eq!(
            server.calibration(),
            None,
            "a table taken on another device key is not used"
        );
        let mut own = Table::new(mine.clone());
        own.set(
            Key::rule("sand", false),
            Rate {
                per_unit_fs: 9,
                per_submit_ns: 9,
            },
        );
        let _ = inject(own.clone());
        let book = Book::empty().calibrated(held(foreign, None));
        let server = Server::resumed(Some(&host), book).0;
        assert_eq!(
            server.calibration(),
            Some(own),
            "the host's own table replaces the other device's"
        );
        let book = server.into_book();
        assert_ne!(
            book.calibration()
                .and_then(|table| table.rate(&Key::rule("sand", false))),
            Some(old)
        );
        calibrate::forget(&mine);
    }

    #[test]
    fn a_still_table_keeps_its_rates_against_real_samples() {
        let Some(host) = common::device("still table") else {
            return;
        };
        let _alone = alone();
        let key = Key::rule("sand", false);
        let high = Rate {
            per_unit_fs: 1_000_000_000_000,
            per_submit_ns: 1_000_000_000,
        };
        let mut table = Table::new(calibrate::device(&host));
        table.set(key.clone(), high);
        let fake = Arc::new(Fake::new());
        let time: Arc<dyn Time> = Arc::clone(&fake) as Arc<dyn Time>;
        let book = Book::empty_timed(time).calibrated(still(table.clone()));
        let mut server = Server::resumed(Some(&host), book).0;
        let world = world_of(&call(
            &mut server,
            "grit_world_create",
            "{\"wide\":32,\"deep\":32,\"tall\":32,\"rule\":\"sand\",\"params\":[],\"budget_ms\":600000}",
        ));
        fake.script(std::iter::repeat_n(Duration::from_micros(1), 64));
        faults::script(std::iter::repeat_n(Duration::from_nanos(1), 64));
        call(
            &mut server,
            "grit_world_step",
            &format!("{{\"world\":{world},\"frames\":8,\"budget_ms\":600000}}"),
        );
        call(
            &mut server,
            "grit_world_step",
            &format!("{{\"world\":{world},\"frames\":8,\"budget_ms\":600000}}"),
        );
        faults::script([]);
        assert_eq!(
            server.calibration(),
            Some(table),
            "a still table is never lowered"
        );
    }

    #[test]
    fn c3_an_injected_table_is_used_as_given_and_nothing_is_calibrated() {
        let Some(host) = common::device("c3 injected") else {
            return;
        };
        let _alone = alone();
        let device = calibrate::device(&host);
        let mut table = Table::new(device.clone());
        table.set(
            Key::rule("sand", false),
            Rate {
                per_unit_fs: 1,
                per_submit_ns: 2,
            },
        );
        let _ = inject(table.clone());
        let server = Server::new(Some(&host));
        assert_eq!(server.calibration(), Some(table.clone()));
        let began = Instant::now();
        let got = taken(&host, None).unwrap();
        assert!(!got.calibrated, "an injected table is not calibrated again");
        assert_eq!(read(&got.table), table);
        eprintln!("taking an injected table took {:?}", began.elapsed());
        let mut other = Table::new(device.clone());
        other.set(
            Key::Fork,
            Rate {
                per_unit_fs: 3,
                per_submit_ns: 4,
            },
        );
        let book = Book::empty().calibrated(held(other.clone(), None));
        let server = Server::resumed(Some(&host), book).0;
        assert_eq!(server.calibration(), Some(other));
        calibrate::forget(&device);
        assert_eq!(Server::new(Some(&host)).calibration(), None);
        assert!(shared(&device).is_none());
    }

    #[test]
    fn c3_a_damaged_file_is_refused_by_name_and_the_device_calibrated_again() {
        let Some(host) = common::device("c3 damaged") else {
            return;
        };
        let _alone = alone();
        let device = calibrate::device(&host);
        calibrate::forget(&device);
        let dir = scratch("damaged");
        std::fs::write(dir.join(FILE), "{\"calibration\":1,\"body\":{}").unwrap();
        let got = taken(&host, Some(&dir)).unwrap();
        let why = got.refused.clone().unwrap();
        assert!(
            why.starts_with(FILE) && why.contains("is not JSON"),
            "{why}"
        );
        assert!(got.calibrated);
        let fresh = read(&got.table);
        assert_eq!(load(&dir, &device), Ok(Some(fresh.clone())));
        calibrate::forget(&device);
        let again = taken(&host, Some(&dir)).unwrap();
        assert!(!again.calibrated && again.refused.is_none());
        assert_eq!(read(&again.table), fresh);
        calibrate::forget(&device);
    }
}
