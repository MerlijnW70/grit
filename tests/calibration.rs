#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

#[cfg(feature = "faults")]
mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use grit::calibrate::take::{GAP, Pair, Pass, Passes, primed};
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
                per_unit_fs: rng.below(1 << 40),
                per_frame_ns: rng.below(1 << 32),
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

fn framed(frames: u32, time: Duration) -> Sample {
    Sample {
        units: 262_144 * u64::from(frames),
        frames,
        submits: 1,
        time,
    }
}

#[test]
fn two_runs_solve_to_floor_terms() {
    let first = Duration::from_nanos(6_700);
    let further = Duration::from_nanos(6_155);
    let at = |frames: u32| framed(frames, first + further * (frames - 1));
    let rate = Rate::framed(&[(at(1), at(32))]).unwrap();
    assert_eq!(
        rate,
        Rate {
            per_unit_fs: 0,
            per_frame_ns: 6_155,
            per_submit_ns: 6_700,
        }
    );
    for frames in [1_u32, 2, 8, 32, 64, 1000] {
        assert_eq!(
            rate.predict(262_144 * u64::from(frames), frames, 1),
            at(frames).time,
            "{frames} frames"
        );
    }
    assert_eq!(
        rate.predict(1 << 40, 1, 1),
        first,
        "no rate grows with cells"
    );
    assert_eq!(
        rate.predict(0, 64, 2),
        first * 2 + further * 62,
        "a step of F frames in S submits is S per-submit and F - S per-frame terms"
    );
    assert_eq!(
        rate.predict(0, 1, 3),
        first * 3,
        "fewer frames than submits"
    );
    let flat = Rate::framed(&[(at(1), framed(32, first / 2))]).unwrap();
    assert_eq!(flat.per_frame_ns, 0, "a per-frame term is never below 0");
    assert_eq!(flat.per_submit_ns, 6_700);
    for wrong in [
        (at(2), at(32)),
        (at(1), at(1)),
        (
            Sample {
                submits: 2,
                ..at(1)
            },
            at(32),
        ),
        (
            at(1),
            Sample {
                submits: 2,
                ..at(32)
            },
        ),
        (framed(1, Duration::ZERO), at(32)),
        (at(1), framed(32, Duration::ZERO)),
    ] {
        assert_eq!(Rate::framed(&[wrong]), None, "{wrong:?}");
    }
    let empty = Sample { units: 0, ..at(32) };
    assert!(
        empty.valid(),
        "a sample of a world with no brick held is valid"
    );
    assert!(Rate::framed(&[(Sample { units: 0, ..at(1) }, empty)]).is_some());
    assert_eq!(Rate::framed(&[]), None, "no size, no rate");
    assert_eq!(
        Rate::framed(&[(at(1), at(32)), (at(2), at(32))]),
        None,
        "one wrong size refuses the rate"
    );
}

fn pair(short: u64, long: u64) -> Pair {
    Pair {
        short: framed(1, Duration::from_nanos(short)),
        long: framed(32, Duration::from_nanos(long)),
    }
}

fn pass(short: u64, long: u64) -> Pass {
    Pass {
        sizes: vec![pair(short, long)],
    }
}

#[test]
fn terms_are_the_least_over_world_sizes() {
    let sizes = [
        (
            framed(1, Duration::from_nanos(4_600)),
            framed(32, Duration::from_nanos(4_600 + 31 * 4_500)),
        ),
        (
            framed(1, Duration::from_nanos(15_500)),
            framed(32, Duration::from_nanos(15_500 + 31 * 14_600)),
        ),
        (
            framed(1, Duration::from_nanos(11_200)),
            framed(32, Duration::from_nanos(11_200 + 31 * 9_100)),
        ),
    ];
    let rate = Rate::framed(&sizes).unwrap();
    assert_eq!(
        rate.per_submit_ns, 4_600,
        "per_submit is the least 1-frame time"
    );
    assert_eq!(
        rate.per_frame_ns, 4_500,
        "per_frame is the least per-frame step"
    );
    assert_eq!(rate.per_unit_fs, 0);
    let crossed = [
        (
            framed(1, Duration::from_nanos(5_000)),
            framed(32, Duration::from_nanos(5_000 + 31 * 20_000)),
        ),
        (
            framed(1, Duration::from_nanos(30_000)),
            framed(32, Duration::from_nanos(30_000 + 31 * 7_000)),
        ),
    ];
    let rate = Rate::framed(&crossed).unwrap();
    assert_eq!(
        (rate.per_submit_ns, rate.per_frame_ns),
        (5_000, 7_000),
        "each term is the least over the sizes on its own"
    );
    for (set, rate) in [
        (&sizes[..], Rate::framed(&sizes).unwrap()),
        (&crossed[..], rate),
    ] {
        for (short, long) in set {
            assert!(rate.predict(short.units, 1, 1) <= short.time);
            assert!(rate.predict(long.units, 32, 1) <= long.time);
        }
    }
    for one in &sizes {
        let alone = Rate::framed(&[*one]).unwrap();
        let least = Rate::framed(&sizes).unwrap();
        assert!(
            least.per_submit_ns <= alone.per_submit_ns && least.per_frame_ns <= alone.per_frame_ns
        );
    }
}

#[test]
fn a_timed_run_counts_only_primed_and_back_to_back() {
    let gap = 500;
    let filler = (1_000, 2_000);
    let primer = (2_000 + gap, 3_000);
    let timed = (3_000 + gap, 4_000);
    assert!(
        primed(filler, primer, timed, gap),
        "gaps of exactly GAP count"
    );
    assert!(
        primed(filler, (2_000, 3_000), (3_000, 4_000), gap),
        "no gap counts"
    );
    assert!(
        !primed(filler, (2_001 + gap, 3_000), (3_000, 4_000), gap),
        "GAP + 1 tick before the primer does not count"
    );
    assert!(
        !primed(filler, (2_000, 3_000), (3_001 + gap, 4_000), gap),
        "GAP + 1 tick before the timed run does not count"
    );
    assert!(
        !primed(filler, (1_999, 3_000), (3_000, 4_000), gap),
        "a primer that starts before the filler ends is not back to back"
    );
    assert!(
        !primed(filler, (2_000, 3_000), (2_999, 4_000), gap),
        "a timed run that starts before the primer ends is not back to back"
    );
    assert_eq!(GAP, Duration::from_micros(500));
}

#[test]
fn a_first_rate_needs_two_passes_that_agree() {
    let key = Key::rule("sand", false);
    let other = Key::rule("sand", true);
    let keys = [key.clone(), other.clone()];
    let slow = pass(13_000, 400_000);
    let fast = pass(6_700, 197_000);
    let near = pass(7_000, 210_000);
    let mut passes = Passes::default();
    passes.add(&key, slow.clone());
    assert_eq!(passes.agreed(&key), None, "one pass is not a rate");
    passes.add(&key, fast.clone());
    assert_eq!(passes.agreed(&key), None, "slow then fast do not agree");
    assert!(
        passes.solved().is_empty(),
        "slow, fast and then out of budget stores nothing"
    );
    passes.add(&key, near.clone());
    let (rate, samples) = passes.agreed(&key).unwrap();
    assert_eq!(rate.per_submit_ns, 6_700, "the least over the two passes");
    assert_eq!(rate.per_frame_ns, (197_000 - 6_700) / 31);
    assert_eq!(
        samples,
        vec![
            framed(1, Duration::from_nanos(6_700)),
            framed(32, Duration::from_nanos(197_000))
        ]
    );
    assert!(!passes.settled(&keys), "a key with no pass is not settled");
    passes.add(&other, fast.clone());
    passes.add(&other, pass(6_700, 247_000));
    assert!(!passes.settled(&keys), "1.254x apart at the long point");
    passes.add(&other, pass(6_700, 246_000));
    assert!(passes.settled(&keys));
    assert_eq!(passes.solved().len(), 2);
    let three = Key::rule("heat", false);
    let sized = |middle: u64| Pass {
        sizes: vec![
            pair(4_600, 144_000),
            pair(middle, 467_000),
            pair(11_200, 293_000),
        ],
    };
    passes.add(&three, sized(15_500));
    passes.add(&three, sized(19_500));
    assert_eq!(
        passes.agreed(&three),
        None,
        "two passes that differ by 1.258x at one point of one size do not agree"
    );
    passes.add(&three, sized(19_000));
    let (rate, samples) = passes.agreed(&three).unwrap();
    assert_eq!(samples.len(), 6, "every point of every size");
    assert_eq!(rate.per_submit_ns, 4_600);
    assert_eq!(rate.per_frame_ns, (144_000 - 4_600) / 31);
    let mut fewer = sized(19_000);
    fewer.sizes.pop();
    passes.add(&three, fewer);
    assert_eq!(
        passes.agreed(&three),
        None,
        "passes over different sizes do not agree"
    );
    passes.add(&key, slow.clone());
    assert_eq!(passes.agreed(&key), None, "only the last two passes count");
    assert!(!passes.settled(&keys));
    let agree = grit::calibrate::take::agree;
    assert!(agree(Duration::from_nanos(4), Duration::from_nanos(5)));
    assert!(!agree(
        Duration::from_nanos(4_000),
        Duration::from_nanos(5_001)
    ));
    assert!(!agree(Duration::ZERO, Duration::ZERO));
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
                per_frame_ns: 60_000,
                per_submit_ns: 100_000,
            },
        );
    }
    let mut lowered = 0;
    for round in 0..4000 {
        let key = &keys[usize::try_from(rng.below(keys.len() as u64)).unwrap()];
        let sample = Sample {
            units: rng.below(1 << 30),
            frames: u32::try_from(rng.below(200)).unwrap(),
            submits: u32::try_from(rng.below(100)).unwrap(),
            time: Duration::from_nanos(rng.below(10_000_000_000)),
        };
        let before = table.rate(key).unwrap();
        let moved = table.lower(key, sample);
        let after = table.rate(key).unwrap();
        assert!(
            after.per_unit_fs <= before.per_unit_fs
                && after.per_frame_ns <= before.per_frame_ns
                && after.per_submit_ns <= before.per_submit_ns,
            "round {round}: {before:?} rose to {after:?} on {sample:?}"
        );
        for units in [0_u64, 1, 1 << 10, 1 << 20, 1 << 30, 1 << 40] {
            for (frames, submits) in [(0_u32, 0_u32), (1, 1), (64, 1), (64, 2), (1000, 16)] {
                assert!(
                    after.predict(units, frames, submits) <= before.predict(units, frames, submits)
                );
            }
        }
        match moved {
            Moved::Lowered => {
                lowered += 1;
                assert!(sample.valid(), "round {round}");
                assert!(
                    after.predict(sample.units, sample.frames, sample.submits) <= sample.time,
                    "round {round}: lowered to {after:?}, still above {sample:?}"
                );
            }
            Moved::Unchanged => assert_eq!(before, after, "round {round}"),
        }
    }
    assert!(
        lowered > 40,
        "only {lowered} of 4000 samples lowered a rate"
    );
    let mut empty = Table::new(device_named("unknown"));
    let key = Key::rule("sand", false);
    for sample in [
        framed(8, Duration::from_millis(3)),
        framed(8, Duration::from_nanos(1)),
        framed(8, Duration::ZERO),
        Sample {
            units: 0,
            ..framed(8, Duration::from_millis(3))
        },
    ] {
        assert_eq!(empty.lower(&key, sample), Moved::Unchanged);
        assert_eq!(empty.rate(&key), None, "a sample never sets a first rate");
    }
    let known = Rate {
        per_unit_fs: 0,
        per_frame_ns: 6_000,
        per_submit_ns: 7_000,
    };
    empty.set(key.clone(), known);
    assert_eq!(
        empty.lower(&key, framed(8, Duration::from_millis(3))),
        Moved::Unchanged,
        "a slower sample leaves the rate"
    );
    assert_eq!(
        empty.lower(&key, framed(8, Duration::ZERO)),
        Moved::Unchanged
    );
    assert_eq!(empty.rate(&key), Some(known));
    assert_eq!(
        empty.lower(&key, framed(8, Duration::from_nanos(24_500))),
        Moved::Lowered
    );
    assert_eq!(
        empty.rate(&key),
        Some(Rate {
            per_unit_fs: 0,
            per_frame_ns: 3_000,
            per_submit_ns: 3_500,
        }),
        "a sample of half the predicted time halves every term"
    );
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
            per_unit_fs: 0,
            per_frame_ns: 123_456,
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
        whole.replace("\"calibration\":2", "\"calibration\":3"),
    )
    .unwrap();
    refused(&dir, &device, "version 3");
    let older = whole
        .replace("\"calibration\":2", "\"calibration\":1")
        .replace(",\"per_frame_ns\":123456", "");
    assert_ne!(older, whole);
    std::fs::write(&path, older).unwrap();
    refused(
        &dir,
        &device,
        "is calibration version 1, and this server reads version 2",
    );
    std::fs::write(&path, whole.replace("\"calibration\":2,", "")).unwrap();
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
        self, Device, FILE, Key, Rate, Table, faults, held, inject, load, missing, read, shared,
        still, taken, up_front,
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
            per_unit_fs: 0,
            per_frame_ns: 1_000_000_000,
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
                now.per_unit_fs <= last.per_unit_fs
                    && now.per_frame_ns <= last.per_frame_ns
                    && now.per_submit_ns <= last.per_submit_ns,
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
            rate_of(&server, &sparse).unwrap().per_frame_ns < before.per_frame_ns,
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
            per_unit_fs: 0,
            per_frame_ns: 7,
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
                per_unit_fs: 0,
                per_frame_ns: 9,
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
            per_unit_fs: 0,
            per_frame_ns: 1_000_000_000,
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
                per_unit_fs: 0,
                per_frame_ns: 1,
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
                per_unit_fs: 0,
                per_frame_ns: 3,
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

    #[test]
    fn an_incomplete_calibration_is_never_stored() {
        let Some(host) = common::device("incomplete") else {
            return;
        };
        let _alone = alone();
        if host.on_cpu() {
            assert!(up_front(&host).is_empty());
            return;
        }
        let device = calibrate::device(&host);
        calibrate::forget(&device);
        let dir = scratch("incomplete");
        faults::device_wait(Some(Arc::new(|| {
            std::thread::sleep(Duration::from_millis(2500));
        })));
        let began = Instant::now();
        let gave_up = std::panic::catch_unwind(|| taken(&host, Some(&dir)));
        faults::device_wait(None);
        let gave_up = gave_up.unwrap().unwrap();
        eprintln!("a calibration that gave up took {:?}", began.elapsed());
        assert!(gave_up.calibrated);
        let partial = read(&gave_up.table);
        assert!(
            !missing(&partial, &host).is_empty(),
            "the device-wait hook did not make the calibration give up: {partial:?}"
        );
        assert!(
            !dir.join(FILE).exists(),
            "a table that lacks an up-front key was written"
        );
        let again = taken(&host, Some(&dir)).unwrap();
        assert!(
            again.calibrated,
            "an incomplete table in memory is calibrated again"
        );
        let whole = read(&again.table);
        assert_eq!(
            missing(&whole, &host),
            Vec::<Key>::new(),
            "a quiet calibration knows every up-front key"
        );
        assert_eq!(load(&dir, &device), Ok(Some(whole.clone())));
        calibrate::forget(&device);
        let first = up_front(&host)[0].clone();
        let mut lacking = Table::new(device.clone());
        for (key, rate) in whole.rates() {
            if *key != first {
                lacking.set(key.clone(), *rate);
            }
        }
        lacking.save(&dir).unwrap();
        let reloaded = taken(&host, Some(&dir)).unwrap();
        let why = reloaded.refused.clone().unwrap();
        assert!(
            why.starts_with(FILE)
                && why.contains(&first.name())
                && why.contains("calibrated again"),
            "{why}"
        );
        assert!(reloaded.calibrated);
        assert!(missing(&read(&reloaded.table), &host).is_empty());
        calibrate::forget(&device);
    }
}
