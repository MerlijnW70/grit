#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

#[cfg(feature = "faults")]
mod common;

#[cfg(feature = "faults")]
mod device {
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, PoisonError};
    use std::time::{Duration, Instant};

    use grit::Host;
    use grit::branch::{Made, World};
    use grit::calibrate::take::{FILLER, FRAMES, Measured, Solved, WORLD, measured};
    use grit::calibrate::{self, Key, Sample, faults, times, up_front};

    use super::common;

    static ONE: Mutex<()> = Mutex::new(());

    static FLOORS: Mutex<Option<(String, Vec<Point>)>> = Mutex::new(None);

    static SPINNING: AtomicUsize = AtomicUsize::new(0);

    const REFERENCE_RUNS: usize = 200;

    const POINT_MOST: Duration = Duration::from_secs(120);

    const LONG: u32 = 64;

    const K: f64 = 1.5;

    const SMOKE: f64 = 4.0;

    const LOAD_MOST: Duration = Duration::from_secs(60);

    const START_MOST: Duration = Duration::from_secs(10);

    const START_CAP: Duration = Duration::from_secs(5);

    fn alone() -> std::sync::MutexGuard<'static, ()> {
        let held = ONE.lock().unwrap_or_else(PoisonError::into_inner);
        assert_eq!(
            SPINNING.load(Ordering::SeqCst),
            0,
            "an earlier test left load threads spinning"
        );
        held
    }

    struct Load {
        stop: Arc<AtomicBool>,
        threads: Vec<std::thread::JoinHandle<()>>,
        fired: Arc<AtomicBool>,
    }

    impl Load {
        fn start(count: usize, most: Duration) -> Self {
            let stop = Arc::new(AtomicBool::new(false));
            let fired = Arc::new(AtomicBool::new(false));
            let began = Instant::now();
            let mut threads: Vec<std::thread::JoinHandle<()>> = (0..count)
                .map(|_| {
                    let stop = Arc::clone(&stop);
                    SPINNING.fetch_add(1, Ordering::SeqCst);
                    std::thread::spawn(move || {
                        let mut turns = 0_u32;
                        while !stop.load(Ordering::Relaxed) {
                            std::hint::spin_loop();
                            turns = turns.wrapping_add(1);
                            if turns.is_multiple_of(4096) && began.elapsed() > most {
                                break;
                            }
                        }
                        SPINNING.fetch_sub(1, Ordering::SeqCst);
                    })
                })
                .collect();
            let watched = Arc::clone(&stop);
            let told = Arc::clone(&fired);
            threads.push(std::thread::spawn(move || {
                while !watched.load(Ordering::Relaxed) {
                    if began.elapsed() > most {
                        told.store(true, Ordering::SeqCst);
                        watched.store(true, Ordering::SeqCst);
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            }));
            Self {
                stop,
                threads,
                fired,
            }
        }

        fn fired(&self) -> bool {
            self.fired.load(Ordering::SeqCst)
        }
    }

    impl Drop for Load {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            for thread in self.threads.drain(..) {
                let _ = thread.join();
            }
        }
    }

    fn gpu(label: &str) -> Option<Host> {
        let host = common::device(label)?;
        if host.on_cpu() {
            eprintln!(
                "{label}: {} is a CPU device; it takes no timing",
                host.name()
            );
            return None;
        }
        Some(host)
    }

    #[derive(Debug, Clone)]
    struct Point {
        key: Key,
        name: &'static str,
        units: u64,
        frames: u32,
        floor: Duration,
        wall: Duration,
    }

    impl Point {
        fn bound(&self) -> bool {
            matches!(self.name, "short" | "long")
        }
    }

    fn warmed(filler: &mut World<'_>) {
        let began = Instant::now();
        while began.elapsed() < Duration::from_millis(1000) {
            filler.step(FRAMES).unwrap();
        }
    }

    fn units_of(world: &mut World<'_>, frames: u32) -> u64 {
        let cells = match world.pool() {
            Some(pool) => u64::from(world.pool_use().unwrap().unwrap().held) * pool.cells() as u64,
            None => world.size().iter().map(|side| u64::from(*side)).product(),
        };
        cells * u64::from(frames)
    }

    fn spread(world: &World<'_>, count: u32) -> Vec<([u32; 3], u32)> {
        let size = world.size();
        let sides = world.pool().unwrap().sides();
        let lattice = [0, 1, 2].map(|axis| size[axis].div_ceil(sides[axis]));
        (0..lattice[0] * lattice[1] * lattice[2])
            .take(count as usize)
            .map(|at| {
                let (x, y, z) = (
                    at % lattice[0],
                    at / lattice[0] % lattice[1],
                    at / (lattice[0] * lattice[1]),
                );
                ([x * sides[0], y * sides[1], z * sides[2]], 1)
            })
            .collect()
    }

    const fn made<'made>(
        rule: &'made str,
        params: &'made [u32],
        size: [u32; 3],
        sparse: bool,
    ) -> Made<'made> {
        Made {
            rule,
            params,
            size,
            sparse,
            slots: None,
        }
    }

    fn largest<'host>(
        host: &'host Host,
        rule: &str,
        params: &[u32],
        side: u32,
        sparse: bool,
    ) -> Option<World<'host>> {
        [side, side * 3 / 4, side / 2]
            .into_iter()
            .find_map(|side| World::create(host, &made(rule, params, [side; 3], sparse)).ok())
    }

    fn filled(mut world: World<'_>) -> World<'_> {
        let room = world.pool().unwrap().capacity();
        let all = spread(&world, room);
        world.write(&all).unwrap();
        world
    }

    struct Taker<'filler, 'host> {
        filler: &'filler mut World<'host>,
        points: Vec<Point>,
    }

    impl Taker<'_, '_> {
        fn measure(&mut self, key: &Key, name: &'static str, world: &mut World<'_>, frames: u32) {
            world.step(1).unwrap();
            let began = Instant::now();
            warmed(self.filler);
            let mut units = 0;
            let mut floor = Duration::MAX;
            let mut wall = Duration::MAX;
            for done in 0..REFERENCE_RUNS {
                assert!(
                    began.elapsed() <= POINT_MOST,
                    "a reference floor took more than {POINT_MOST:?} after {done} runs"
                );
                units = units.max(units_of(world, frames));
                self.filler.step(FRAMES).unwrap();
                world.step(frames).unwrap();
                let started = Instant::now();
                world.step(frames).unwrap();
                wall = wall.min(started.elapsed());
                floor = floor.min(world.on_device().unwrap() * frames);
            }
            self.points.push(Point {
                key: key.clone(),
                name,
                units,
                frames,
                floor,
                wall,
            });
        }
    }

    fn frame_points(host: &Host) -> Vec<Point> {
        let mut filler = World::create(host, &made("sand", &[], FILLER, false)).unwrap();
        let mut taker = Taker {
            filler: &mut filler,
            points: Vec::new(),
        };
        for rule in grit::serve3::rules_on(Some(host)) {
            let params = vec![0_u32; rule.params()];
            let name = rule.name();
            let dense = Key::rule(name, false);
            let mut world = World::create(host, &made(name, &params, WORLD, false)).unwrap();
            taker.measure(&dense, "short", &mut world, 1);
            taker.measure(&dense, "long", &mut world, LONG);
            drop(world);
            let mut tiny = World::create(host, &made(name, &params, [8; 3], false)).unwrap();
            taker.measure(&dense, "tiny", &mut tiny, LONG);
            drop(tiny);
            let mut far = largest(host, name, &params, 256, false).unwrap();
            taker.measure(&dense, "far", &mut far, LONG);
            drop(far);
            let mut mid = World::create(host, &made(name, &params, [32; 3], false)).unwrap();
            taker.measure(&dense, "mid", &mut mid, LONG);
            drop(mid);
            let mut slab = World::create(host, &made(name, &params, [128, 128, 8], false)).unwrap();
            taker.measure(&dense, "slab", &mut slab, LONG);
            drop(slab);
            if !host.sparse() {
                continue;
            }
            let sparse = Key::rule(name, true);
            let mut world = World::create(host, &made(name, &params, WORLD, true)).unwrap();
            taker.measure(&sparse, "short", &mut world, 1);
            taker.measure(&sparse, "long", &mut world, LONG);
            let mut world = filled(world);
            taker.measure(&sparse, "held", &mut world, LONG);
            drop(world);
            let mut far = filled(largest(host, name, &params, 128, true).unwrap());
            taker.measure(&sparse, "far", &mut far, LONG);
        }
        taker.points
    }

    fn floors(host: &Host) -> Vec<Point> {
        let mut held = FLOORS.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((name, points)) = held.as_ref()
            && name == host.name()
        {
            return points.clone();
        }
        let began = Instant::now();
        let points = frame_points(host);
        for one in &points {
            eprintln!(
                "FLOOR {} {} {}: {} units, {} frames, floor {:?}, fastest wall {:?}",
                host.name(),
                one.key.name(),
                one.name,
                one.units,
                one.frames,
                one.floor,
                one.wall
            );
        }
        eprintln!("FLOOR {} measured in {:?}", host.name(), began.elapsed());
        *held = Some((host.name().to_owned(), points.clone()));
        points
    }

    fn predicted(found: &BTreeMap<Key, Solved>, one: &Point) -> Option<Duration> {
        found
            .get(&one.key)
            .map(|(rate, _)| rate.predict(one.units, one.frames, 1))
    }

    fn report(label: &str, host: &Host, got: &Measured, took: Duration) {
        eprintln!(
            "CAL {label} {}: {} keys of {}, ended {:?} after {} passes in {took:?}; timed runs {} tried, {} counted, {} did not count ({:.1}%); device clock: warm {:?}, worlds {:?}, passes {:?}",
            host.name(),
            got.rates.len(),
            up_front(host).len(),
            got.ended,
            got.passes,
            got.tried,
            got.counted,
            got.tried - got.counted,
            100.0 * f64::from(got.tried - got.counted) / f64::from(got.tried.max(1)),
            got.warm,
            got.made,
            got.timed
        );
        for (key, (rate, _)) in &got.rates {
            eprintln!(
                "CAL {label} {} {}: per_submit {} ns, per_frame {} ns, per_unit {} fs",
                host.name(),
                key.name(),
                rate.per_submit_ns,
                rate.per_frame_ns,
                rate.per_unit_fs
            );
        }
    }

    fn one_sided(
        label: &str,
        host: &Host,
        points: &[Point],
        found: &BTreeMap<Key, Solved>,
        bound: f64,
    ) -> Vec<String> {
        let keys = up_front(host);
        let unknown: Vec<String> = keys
            .iter()
            .filter(|key| !found.contains_key(*key))
            .map(Key::name)
            .collect();
        eprintln!(
            "{label} {}: {} up-front keys, {} unknown {unknown:?}",
            host.name(),
            keys.len(),
            unknown.len()
        );
        let mut failures = Vec::new();
        let mut worst = 0.0_f64;
        for one in points {
            let Some(guess) = predicted(found, one) else {
                continue;
            };
            let ratio = guess.as_secs_f64() / one.floor.as_secs_f64().max(1e-12);
            worst = worst.max(ratio);
            eprintln!(
                "{label} {} {} {}: predicted {guess:?}, floor {:?}, P/R {ratio:.3}",
                host.name(),
                one.key.name(),
                one.name,
                one.floor
            );
            if ratio > bound {
                failures.push(format!(
                    "{} {}: P/R {ratio:.3} above {bound}",
                    one.key.name(),
                    one.name
                ));
            }
        }
        eprintln!("WORST {label} {}: P/R {worst:.3}", host.name());
        failures
    }

    fn timed(label: &str, host: &Host) -> Measured {
        let began = Instant::now();
        let got = measured(host).unwrap();
        report(label, host, &got, began.elapsed());
        got
    }

    fn deterministic(
        label: &str,
        host: &Host,
        prepare: impl FnOnce(),
        calibrate: impl FnOnce() -> Measured,
    ) {
        let points = floors(host);
        prepare();
        let found = calibrate();
        let failures = one_sided(label, host, &points, &found.rates, K);
        assert!(failures.is_empty(), "{label}: {failures:#?}");
    }

    #[test]
    fn c7_a_quiet_calibration_knows_every_up_front_key() {
        let Some(host) = common::device("c7") else {
            return;
        };
        let _alone = alone();
        if host.on_cpu() {
            let before = faults::timed_runs();
            assert!(!times(&host), "a CPU device has no time source");
            assert!(up_front(&host).is_empty());
            assert!(measured(&host).unwrap().rates.is_empty());
            let device = calibrate::device(&host);
            calibrate::forget(&device);
            let got = calibrate::taken(&host, None).unwrap();
            assert!(calibrate::read(&got.table).is_empty());
            calibrate::forget(&device);
            assert_eq!(
                faults::timed_runs(),
                before,
                "no calibration work ran on a CPU device"
            );
            eprintln!(
                "C7 {}: a CPU device, every key unknown and no calibration work ran",
                host.name()
            );
            return;
        }
        let wanted: Vec<Key> = grit::serve3::rules_on(Some(&host))
            .iter()
            .flat_map(|rule| {
                [false, true]
                    .into_iter()
                    .filter(|sparse| !sparse || host.sparse())
                    .map(|sparse| Key::rule(rule.name(), sparse))
            })
            .collect();
        assert_eq!(up_front(&host), wanted);
        let found = timed("c7", &host).rates;
        let missing: Vec<String> = wanted
            .iter()
            .filter(|key| !found.contains_key(*key))
            .map(Key::name)
            .collect();
        eprintln!(
            "C7 {}: up-front keys {:?}",
            host.name(),
            wanted.iter().map(Key::name).collect::<Vec<_>>()
        );
        assert!(
            missing.is_empty(),
            "{}: unknown after a quiet calibration: {missing:?}",
            host.name()
        );
    }

    #[test]
    fn sparse_keys_are_calibrated_on_worlds_as_built() {
        let Some(host) = gpu("sparse as built") else {
            return;
        };
        let _alone = alone();
        let found = timed("as built", &host).rates;
        let sparse: Vec<_> = found
            .iter()
            .filter(|(key, _)| matches!(key, Key::Frame { sparse: true, .. }))
            .collect();
        assert_eq!(sparse.is_empty(), !host.sparse());
        for (key, (_, samples)) in sparse {
            for sample in samples {
                assert_eq!(
                    sample.units,
                    0,
                    "{}: a calibration run held bricks: {sample:?}",
                    key.name()
                );
            }
        }
    }

    #[test]
    fn c9_start_up_stays_under_the_cap() {
        let Some(host) = gpu("c9") else {
            return;
        };
        let _alone = alone();
        let device = calibrate::device(&host);
        calibrate::forget(&device);
        let began = Instant::now();
        let got = timed("c9", &host);
        let took = began.elapsed();
        eprintln!(
            "STARTUP {} driver {} width {}: calibration took {} ms, {} passes",
            device.name,
            device.driver,
            device.width,
            took.as_millis(),
            got.passes
        );
        if took > START_CAP {
            eprintln!("ABOVE-CAP {}: {took:?} above {START_CAP:?}", device.name);
        }
        assert!(
            took <= START_MOST,
            "start-up took {took:?}, above {START_MOST:?}"
        );
        assert_eq!(got.rates.len(), up_front(&host).len());
    }

    #[test]
    fn c1_a_quiet_calibration_is_at_most_k_times_the_floor() {
        let Some(host) = gpu("c1 quiet") else {
            return;
        };
        let _alone = alone();
        deterministic("c1 quiet", &host, || {}, || timed("c1 quiet", &host));
    }

    #[test]
    fn c1_the_calibrating_thread_held_off_its_cpu_raises_no_rate() {
        let Some(host) = gpu("c1 hold") else {
            return;
        };
        let _alone = alone();
        deterministic(
            "c1 hold",
            &host,
            || {},
            || {
                faults::hold(Some(Arc::new(|| {
                    std::thread::sleep(Duration::from_millis(15));
                })));
                let found = std::panic::catch_unwind(|| timed("c1 hold", &host));
                faults::hold(None);
                found.unwrap()
            },
        );
    }

    #[test]
    fn c1_waits_injected_on_the_device_raise_no_rate() {
        let Some(host) = gpu("c1 device waits") else {
            return;
        };
        let _alone = alone();
        deterministic(
            "c1 device waits",
            &host,
            || {},
            || {
                faults::device_wait(Some(Arc::new(|| {
                    std::thread::sleep(Duration::from_millis(15));
                })));
                let found = std::panic::catch_unwind(|| timed("c1 device waits", &host));
                faults::device_wait(None);
                found.unwrap()
            },
        );
    }

    #[test]
    fn c1_another_world_stepping_on_the_same_host_raises_no_rate() {
        let Some(host) = gpu("c1 other world") else {
            return;
        };
        let _alone = alone();
        let stop = AtomicBool::new(false);
        let steps = AtomicU64::new(0);
        deterministic(
            "c1 other world",
            &host,
            || {},
            || {
                std::thread::scope(|scope| {
                    let busy = scope.spawn(|| {
                        let mut world =
                            World::create(&host, &made("sand", &[], FILLER, false)).unwrap();
                        while !stop.load(Ordering::Relaxed) {
                            world.step(64).unwrap();
                            steps.fetch_add(1, Ordering::Relaxed);
                        }
                    });
                    let began = Instant::now();
                    while steps.load(Ordering::Relaxed) < 2
                        && !busy.is_finished()
                        && began.elapsed() < POINT_MOST
                    {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    let found = std::panic::catch_unwind(|| timed("c1 other world", &host));
                    stop.store(true, Ordering::Relaxed);
                    busy.join().unwrap();
                    assert!(
                        steps.load(Ordering::Relaxed) >= 2,
                        "the other world never stepped"
                    );
                    found.unwrap()
                })
            },
        );
    }

    #[test]
    fn c1_a_calibration_after_device_idle_raises_no_rate() {
        let Some(host) = gpu("c1 idle") else {
            return;
        };
        let _alone = alone();
        deterministic(
            "c1 idle",
            &host,
            || std::thread::sleep(Duration::from_secs(5)),
            || timed("c1 idle", &host),
        );
    }

    fn loaded(
        host: &Host,
        points: &[Point],
        level: &str,
        spinners: usize,
        cores: usize,
    ) -> Measured {
        let load = Load::start(spinners, LOAD_MOST);
        let began = Instant::now();
        let found = std::panic::catch_unwind(|| measured(host));
        let took = began.elapsed();
        let fired = load.fired();
        drop(load);
        assert_eq!(
            SPINNING.load(Ordering::SeqCst),
            0,
            "the load threads did not stop"
        );
        assert!(
            !fired,
            "level {level}: the calibration did not return within {LOAD_MOST:?} under {spinners} spinners on {cores} cores ({took:?})"
        );
        let found = found.unwrap().unwrap();
        report(
            &format!("c1 load {level} ({spinners} spinners on {cores} cores)"),
            host,
            &found,
            took,
        );
        let above: Vec<String> = points
            .iter()
            .filter_map(|one| {
                let ratio = predicted(&found.rates, one)?.as_secs_f64() / one.floor.as_secs_f64();
                (ratio > 3.0).then(|| format!("{} {}: P/R {ratio:.3}", one.key.name(), one.name))
            })
            .collect();
        eprintln!("ABOVE-3 c1 load {level} {}: {above:?}", host.name());
        found
    }

    #[test]
    fn c1_real_cpu_load_stays_under_the_margin() {
        let Some(host) = gpu("c1 load") else {
            return;
        };
        let _alone = alone();
        let points = floors(&host);
        let cores = std::thread::available_parallelism().map_or(4, std::num::NonZero::get);
        let half = loaded(&host, &points, "half", (cores / 2).max(1), cores);
        let mut failures = one_sided("c1 load half", &host, &points, &half.rates, SMOKE);
        let missing: Vec<String> = up_front(&host)
            .iter()
            .filter(|key| !half.rates.contains_key(*key))
            .map(Key::name)
            .collect();
        if !missing.is_empty() {
            failures.push(format!("half load left {missing:?} unknown"));
        }
        let over = loaded(&host, &points, "over", 2 * cores, cores);
        failures.extend(one_sided(
            "c1 load over",
            &host,
            &points,
            &over.rates,
            SMOKE,
        ));
        assert!(failures.is_empty(), "c1 load: {failures:#?}");
    }

    #[test]
    fn c4_each_rate_is_plausible_against_plain_runs() {
        let Some(host) = gpu("c4") else {
            return;
        };
        let _alone = alone();
        let device = calibrate::device(&host);
        calibrate::forget(&device);
        let got = calibrate::taken(&host, None).unwrap();
        calibrate::forget(&device);
        let points = floors(&host);
        let table = calibrate::read(&got.table);
        let mut failures = Vec::new();
        for one in &points {
            let Some(rate) = table.rate(&one.key) else {
                failures.push(format!("{} unknown", one.key.name()));
                continue;
            };
            let guess = rate.predict(one.units, one.frames, 1);
            let high = guess.as_secs_f64() / one.wall.as_secs_f64();
            let low = one.floor.as_secs_f64() / guess.as_secs_f64().max(1e-12);
            eprintln!(
                "C4 {} {} {}: predicted {guess:?}, fastest wall {:?} (P/wall {high:.3}), floor {:?} (R/P {low:.3})",
                host.name(),
                one.key.name(),
                one.name,
                one.wall,
                one.floor
            );
            if high > 3.0 {
                failures.push(format!(
                    "{} {}: P/wall {high:.3} above 3",
                    one.key.name(),
                    one.name
                ));
            }
            if one.bound() && low > 10.0 {
                failures.push(format!(
                    "{} {}: R/P {low:.3} above 10 (low side)",
                    one.key.name(),
                    one.name
                ));
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
    }

    fn agree(label: &str, points: &[Point], idle: &Measured, busy: &Measured) -> Vec<String> {
        let mut failures = Vec::new();
        for point in points {
            let (Some(left), Some(right)) =
                (predicted(&idle.rates, point), predicted(&busy.rates, point))
            else {
                failures.push(format!(
                    "{label} {} {}: unknown in one of the two",
                    point.key.name(),
                    point.name
                ));
                continue;
            };
            let ratio = left.as_secs_f64().max(right.as_secs_f64())
                / left.as_secs_f64().min(right.as_secs_f64()).max(1e-12);
            eprintln!(
                "{label} {} {}: idle {left:?} busy {right:?} ratio {ratio:.3}",
                point.key.name(),
                point.name
            );
            if ratio > 1.25 {
                failures.push(format!(
                    "{label} {} {}: {ratio:.3}",
                    point.key.name(),
                    point.name
                ));
            }
        }
        failures
    }

    fn first_over(got: &Measured) -> f64 {
        got.first
            .iter()
            .filter_map(|(key, pass)| {
                let (_, samples) = got.rates.get(key)?;
                let ratio = |sample: Sample, last: &Sample| {
                    sample.time.as_secs_f64() / last.time.as_secs_f64().max(1e-12)
                };
                pass.sizes
                    .iter()
                    .flat_map(|pair| [pair.short, pair.long])
                    .zip(samples)
                    .map(|(first, last)| ratio(first, last))
                    .reduce(f64::max)
            })
            .fold(0.0_f64, f64::max)
    }

    #[test]
    fn c6_idle_and_busy_calibrations_agree() {
        let Some(host) = gpu("c6") else {
            return;
        };
        let _alone = alone();
        let points = floors(&host);
        let mut filler = World::create(&host, &made("sand", &[], FILLER, false)).unwrap();
        let mut failures = Vec::new();
        for round in 0..10 {
            std::thread::sleep(Duration::from_secs(5));
            let idle = timed(&format!("c6 round {round} idle"), &host);
            warmed(&mut filler);
            let busy = timed(&format!("c6 round {round} busy"), &host);
            eprintln!(
                "C6 {} round {round}: idle {} passes, first pass up to {:.3}x its final; busy {} passes, first pass up to {:.3}x its final",
                host.name(),
                idle.passes,
                first_over(&idle),
                busy.passes,
                first_over(&busy)
            );
            failures.extend(agree(&format!("c6 round {round}"), &points, &idle, &busy));
        }
        assert!(failures.is_empty(), "{failures:#?}");
    }
}
