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
        threads: 8,
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
fn two_sizes_solve_to_the_terms_that_made_them() {
    let rate = Rate {
        per_unit_fs: 2_500,
        per_submit_ns: 40_000,
    };
    let at = |units: u64, submits: u32| Sample {
        units,
        submits,
        time: rate.predict(units, submits),
    };
    let solved = Rate::solved(at(262_144, 1), at(2_097_152, 1)).unwrap();
    assert!(solved.per_unit_fs.abs_diff(2_500) <= 1, "{solved:?}");
    assert!(solved.per_submit_ns.abs_diff(40_000) <= 1, "{solved:?}");
    let chunks = Rate::solved(at(1, 1), at(512, 16)).unwrap();
    assert!(chunks.per_submit_ns.abs_diff(40_000) <= 1, "{chunks:?}");
    let flat = Sample {
        units: 2_097_152,
        submits: 1,
        time: Duration::from_micros(10),
    };
    assert_eq!(
        Rate::solved(
            Sample {
                units: 262_144,
                submits: 1,
                time: Duration::from_micros(11),
            },
            flat
        ),
        None,
        "a larger run that took no longer gives no per-unit term: unknown"
    );
    assert_eq!(Rate::solved(flat, flat), None);
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
            threads: u32::try_from(rng.below(64)).unwrap(),
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
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, PoisonError};
    use std::time::{Duration, Instant};

    use grit::Host;
    use grit::branch::{Made, World};
    use grit::calibrate::take::{COPY_LARGE, Plan, Solved, measured, plan};
    use grit::calibrate::{
        self, FILE, Key, Meter, Rate, Sample, Table, faults, held, inject, load, read, shared,
        taken,
    };
    use grit::clock::{Fake, Time};
    use grit::serve3::budget::world_cost;
    use grit::serve3::{Book, Server};
    use grit::wire::{Json, written};

    use super::{common, scratch};

    static ONE: Mutex<()> = Mutex::new(());

    fn alone() -> std::sync::MutexGuard<'static, ()> {
        ONE.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn shown(label: &str, found: &BTreeMap<Key, Solved>) {
        for (key, (rate, [small, large])) in found {
            eprintln!(
                "{label} {} per_unit_fs={} per_submit_ns={} small=({} units, {} submits, {:?}) large=({} units, {} submits, {:?})",
                key.name(),
                rate.per_unit_fs,
                rate.per_submit_ns,
                small.units,
                small.submits,
                small.time,
                large.units,
                large.submits,
                large.time
            );
        }
    }

    fn at_most_half_again(
        label: &str,
        base: &BTreeMap<Key, Solved>,
        other: &BTreeMap<Key, Solved>,
    ) {
        shown(&format!("{label} base"), base);
        shown(&format!("{label} contended"), other);
        let mut compared = 0;
        let mut unknown = 0;
        let mut failures = Vec::new();
        for (key, (rate, points)) in base {
            let Some((contended, _)) = other.get(key) else {
                eprintln!("{label} {} unknown under contention", key.name());
                unknown += 1;
                continue;
            };
            for point in points {
                let calm = rate.nanos(point.units, point.submits);
                let pressed = contended.nanos(point.units, point.submits);
                compared += 1;
                eprintln!(
                    "{label} {} at {} units: contended/uncontended = {:.3}",
                    key.name(),
                    point.units,
                    pressed as f64 / calm.max(1) as f64
                );
                if pressed.saturating_mul(2) > calm.saturating_mul(3) {
                    failures.push(format!(
                        "{} at {} units, {} submits: {pressed} ns under contention against {calm} ns",
                        key.name(),
                        point.units,
                        point.submits
                    ));
                }
            }
        }
        eprintln!(
            "{label}: {} keys calibrated uncontended, {unknown} unknown under contention, {compared} points compared",
            base.len()
        );
        assert!(failures.is_empty(), "{label}: {failures:#?}");
        for key in other.keys().filter(|key| !base.contains_key(*key)) {
            eprintln!(
                "{label} {} known only under contention: its quiet calibration found no per-unit term",
                key.name()
            );
        }
    }

    fn calibrated(host: &Host) -> Option<BTreeMap<Key, Solved>> {
        if calibrate::meter(host).is_none() {
            eprintln!(
                "{}: no sound time source on this device, so every rate stays unknown",
                host.name()
            );
            assert!(measured(host).unwrap().is_empty());
            return None;
        }
        let found = measured(host).unwrap();
        assert!(
            found.keys().any(|key| matches!(key, Key::Frame { .. })),
            "{}: no frame rate calibrated",
            host.name()
        );
        Some(found)
    }

    #[test]
    fn c1_the_calibrating_thread_held_off_its_cpu_raises_no_rate() {
        let Some(host) = common::device("c1 hold") else {
            return;
        };
        let _alone = alone();
        let Some(base) = calibrated(&host) else {
            return;
        };
        faults::hold(Some(Arc::new(|| {
            std::thread::sleep(Duration::from_millis(15));
        })));
        let held = measured(&host);
        faults::hold(None);
        at_most_half_again("c1 hold", &base, &held.unwrap());
    }

    #[test]
    fn c1_real_cpu_load_raises_no_rate() {
        let Some(host) = common::device("c1 load") else {
            return;
        };
        let _alone = alone();
        let Some(base) = calibrated(&host) else {
            return;
        };
        let stop = Arc::new(AtomicBool::new(false));
        let cores = std::thread::available_parallelism().map_or(4, std::num::NonZero::get);
        let load: Vec<_> = (0..2 * cores)
            .map(|seed| {
                let stop = Arc::clone(&stop);
                std::thread::spawn(move || {
                    let mut spin = seed as u64 | 1;
                    while !stop.load(Ordering::Relaxed) {
                        for _ in 0..10_000 {
                            spin ^= spin << 13;
                            spin ^= spin >> 7;
                            spin ^= spin << 17;
                        }
                    }
                    spin
                })
            })
            .collect();
        let loaded = measured(&host);
        stop.store(true, Ordering::Relaxed);
        for one in load {
            let _ = one.join();
        }
        eprintln!("c1 load: {} spinning threads on {cores} cores", 2 * cores);
        at_most_half_again("c1 load", &base, &loaded.unwrap());
    }

    #[test]
    fn c1_another_world_stepping_on_the_same_host_raises_no_rate() {
        let Some(host) = common::device("c1 other world") else {
            return;
        };
        let _alone = alone();
        let Some(base) = calibrated(&host) else {
            return;
        };
        let stop = AtomicBool::new(false);
        let steps = std::sync::atomic::AtomicU64::new(0);
        let loaded = std::thread::scope(|scope| {
            let busy = scope.spawn(|| {
                let made = Made {
                    rule: "sand",
                    params: &[],
                    size: plan(calibrate::meter(&host).unwrap()).large,
                    sparse: false,
                    slots: None,
                };
                let mut world = World::create(&host, &made).unwrap();
                while !stop.load(Ordering::Relaxed) {
                    world.step(64).unwrap();
                    steps.fetch_add(1, Ordering::Relaxed);
                }
            });
            while steps.load(Ordering::Relaxed) < 2 {
                std::thread::yield_now();
            }
            let loaded = measured(&host);
            stop.store(true, Ordering::Relaxed);
            busy.join().unwrap();
            loaded
        });
        eprintln!(
            "c1 other world: the other world ran {} steps of 64 frames",
            steps.load(Ordering::Relaxed)
        );
        at_most_half_again("c1 other world", &base, &loaded.unwrap());
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

    fn third(size: [u32; 3]) -> [u32; 3] {
        size.map(|side| side * 3 / 4)
    }

    fn middle(mut times: Vec<Duration>) -> Duration {
        times.sort_unstable();
        times[times.len() / 2]
    }

    fn plain_frames(host: &Host, plan: Plan, rule: &str, sparse: bool) -> Option<Sample> {
        let rules = grit::serve3::rules_on(Some(host));
        let params = vec![0_u32; rules.iter().find(|one| one.name() == rule)?.params()];
        let size = if sparse {
            plan.sparse
        } else {
            third(plan.large)
        };
        let made = Made {
            rule,
            params: &params,
            size,
            sparse,
            slots: None,
        };
        let mut world = World::create(host, &made).unwrap();
        if let Some(pool) = world.pool() {
            let sides = pool.sides();
            let lattice = [0, 1, 2].map(|axis| size[axis] / sides[axis]);
            let count = pool.capacity() / 2;
            let cells: Vec<([u32; 3], u32)> = (0..lattice[0] * lattice[1] * lattice[2])
                .take(count as usize)
                .map(|at| {
                    let (x, y, z) = (
                        at % lattice[0],
                        at / lattice[0] % lattice[1],
                        at / (lattice[0] * lattice[1]),
                    );
                    ([x * sides[0], y * sides[1], z * sides[2]], 1)
                })
                .collect();
            world.write(&cells).unwrap();
        }
        world.step(1).unwrap();
        let mut times = Vec::new();
        let mut units = 0;
        for _ in 0..3 {
            let cells = match world.pool() {
                Some(pool) => {
                    u64::from(world.pool_use().unwrap().unwrap().held) * pool.cells() as u64
                }
                None => size.iter().map(|side| u64::from(*side)).product(),
            };
            units = cells * u64::from(plan.frames);
            let began = Instant::now();
            world.step(plan.frames).unwrap();
            times.push(began.elapsed());
        }
        Some(Sample {
            units,
            submits: 1,
            time: middle(times),
        })
    }

    fn plain(host: &Host, plan: Plan, key: &Key) -> Option<Sample> {
        let size = third(COPY_LARGE);
        let sand = |sparse| Made {
            rule: "sand",
            params: &[],
            size,
            sparse,
            slots: None,
        };
        let mut times = Vec::new();
        let (units, submits) = match key {
            Key::Frame {
                sparse,
                by: calibrate::By::Rule(rule),
            } => return plain_frames(host, plan, rule, *sparse),
            Key::Build { sparse } => {
                let mut units = 0;
                for _ in 0..3 {
                    let began = Instant::now();
                    let world = World::create(host, &sand(*sparse)).unwrap();
                    times.push(began.elapsed());
                    units = world_cost(world.sizes());
                }
                (units, 1)
            }
            Key::Write { sparse } => {
                let mut world = World::create(host, &sand(*sparse)).unwrap();
                let (count, calls) = if *sparse { (512, 1) } else { (256, 8) };
                let cells: Vec<([u32; 3], u32)> = (0..count)
                    .map(|at: u32| ([at % 16, at / 16 % 16, at / 256], 1))
                    .collect();
                for _ in 0..3 {
                    let began = Instant::now();
                    for piece in cells.chunks(cells.len() / calls) {
                        world.write(piece).unwrap();
                    }
                    times.push(began.elapsed());
                }
                (u64::from(count), u32::try_from(calls).unwrap())
            }
            Key::Fork | Key::Restore | Key::Snapshot { file: false } => {
                let mut source = World::create(host, &sand(false)).unwrap();
                let mut target = World::create(host, &sand(false)).unwrap();
                for _ in 0..3 {
                    let began = Instant::now();
                    match key {
                        Key::Fork => drop(source.fork().unwrap()),
                        Key::Restore => target.restore(&mut source).unwrap(),
                        _ => drop(source.image().unwrap()),
                    }
                    times.push(began.elapsed());
                }
                (world_cost(source.sizes()), 1)
            }
            _ => return None,
        };
        Some(Sample {
            units,
            submits,
            time: middle(times),
        })
    }

    #[test]
    fn c4_each_calibrated_rate_predicts_a_plain_run_within_ten_times() {
        let Some(host) = common::device("c4 plausible") else {
            return;
        };
        let _alone = alone();
        let device = calibrate::device(&host);
        calibrate::forget(&device);
        let began = Instant::now();
        let got = taken(&host, None).unwrap();
        let took = began.elapsed();
        calibrate::forget(&device);
        assert!(got.calibrated);
        eprintln!(
            "STARTUP {} driver {} width {} threads {}: calibration took {} ms",
            device.name,
            device.driver,
            device.width,
            device.threads,
            took.as_millis()
        );
        let table = read(&got.table);
        let Some(meter) = calibrate::meter(&host) else {
            assert!(table.is_empty());
            return;
        };
        let plan = plan(meter);
        if let Meter::Stamps = meter {
            assert!(
                table
                    .rates()
                    .all(|(key, _)| matches!(key, Key::Frame { sparse: false, .. })),
                "on a GPU only dense frames have a sound source"
            );
        }
        let mut failures = Vec::new();
        let mut checked = 0;
        for (key, rate) in table.rates() {
            let Some(run) = plain(&host, plan, key) else {
                failures.push(format!("{}: no plain run", key.name()));
                continue;
            };
            let predicted = rate.predict(run.units, run.submits);
            let ratio = run.time.as_secs_f64() / predicted.as_secs_f64().max(1e-12);
            eprintln!(
                "C4 {} {}: predicted {:?} for {} units and {} submits, the plain run took {:?}, ratio {:.2}",
                device.name,
                key.name(),
                predicted,
                run.units,
                run.submits,
                run.time,
                ratio
            );
            checked += 1;
            if !(0.1..=10.0).contains(&ratio) {
                failures.push(format!("{}: ratio {ratio:.2}", key.name()));
            }
        }
        assert!(checked > 0);
        assert!(failures.is_empty(), "{failures:#?}");
    }
}
