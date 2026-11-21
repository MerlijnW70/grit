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
    use grit::calibrate::take::{FRAMES, LARGE, SMALL, SPARSE, Solved, measured, seeded};
    use grit::calibrate::{self, Key, faults, times};

    use super::common;

    static ONE: Mutex<()> = Mutex::new(());

    static FLOORS: Mutex<Option<(String, Vec<Point>)>> = Mutex::new(None);

    static SPINNING: AtomicUsize = AtomicUsize::new(0);

    const REFERENCE_RUNS: usize = 200;

    const POINT_MOST: Duration = Duration::from_secs(120);

    const LONG: u32 = 64;

    const SPARSE_FEW: u32 = 2;

    const K: f64 = 1.5;

    const SMOKE: f64 = 4.0;

    const LOAD_MOST: Duration = Duration::from_secs(180);

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
            let mut threads: Vec<std::thread::JoinHandle<()>> = (0..count)
                .map(|_| {
                    let stop = Arc::clone(&stop);
                    SPINNING.fetch_add(1, Ordering::SeqCst);
                    std::thread::spawn(move || {
                        while !stop.load(Ordering::Relaxed) {
                            std::hint::spin_loop();
                        }
                        SPINNING.fetch_sub(1, Ordering::SeqCst);
                    })
                })
                .collect();
            let watched = Arc::clone(&stop);
            let told = Arc::clone(&fired);
            threads.push(std::thread::spawn(move || {
                let began = Instant::now();
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
        floor: Duration,
        wall: Duration,
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

    fn reference(
        filler: &mut World<'_>,
        world: &mut World<'_>,
        frames: u32,
    ) -> (u64, Duration, Duration) {
        let began = Instant::now();
        warmed(filler);
        let mut units = 0;
        let mut floor = Duration::MAX;
        let mut wall = Duration::MAX;
        for done in 0..REFERENCE_RUNS {
            assert!(
                began.elapsed() <= POINT_MOST,
                "a reference floor took more than {POINT_MOST:?} after {done} runs"
            );
            filler.step(FRAMES).unwrap();
            units = units.max(units_of(world, frames));
            let started = Instant::now();
            world.step(frames).unwrap();
            wall = wall.min(started.elapsed());
            floor = floor.min(world.on_device().unwrap() * frames);
        }
        (units, floor, wall)
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

    fn far_of<'host>(
        host: &'host Host,
        rule: &str,
        params: &[u32],
        large: [u32; 3],
        sparse: bool,
    ) -> Option<World<'host>> {
        [
            large.map(|side| side * 2),
            large.map(|side| side * 3 / 2),
            large,
        ]
        .into_iter()
        .find_map(|size| {
            World::create(
                host,
                &Made {
                    rule,
                    params,
                    size,
                    sparse,
                    slots: None,
                },
            )
            .ok()
        })
    }

    fn measure(
        points: &mut Vec<Point>,
        filler: &mut World<'_>,
        key: &Key,
        name: &'static str,
        world: &mut World<'_>,
        frames: u32,
    ) {
        world.step(1).unwrap();
        let (units, floor, wall) = reference(filler, world, frames);
        points.push(Point {
            key: key.clone(),
            name,
            units,
            floor,
            wall,
        });
    }

    fn frame_points(host: &Host) -> Vec<Point> {
        let mut filler = World::create(host, &sand(LARGE)).unwrap();
        filler.write(&seeded(LARGE)).unwrap();
        let mut points = Vec::new();
        for rule in grit::serve3::rules_on(Some(host)) {
            let params = vec![0_u32; rule.params()];
            let name = rule.name();
            let made = |size, sparse| Made {
                rule: name,
                params: &params,
                size,
                sparse,
                slots: None,
            };
            if let (Ok(mut little), Ok(mut big)) = (
                World::create(host, &made(SMALL, false)),
                World::create(host, &made(LARGE, false)),
            ) {
                let key = Key::rule(name, false);
                measure(&mut points, &mut filler, &key, "short", &mut little, 1);
                measure(&mut points, &mut filler, &key, "large", &mut big, LONG);
                drop((little, big));
                if let Some(mut far) = far_of(host, name, &params, LARGE, false) {
                    measure(&mut points, &mut filler, &key, "far", &mut far, LONG);
                }
            }
            if let Ok(mut world) = World::create(host, &made(SPARSE, true)) {
                let key = Key::rule(name, true);
                let room = world.pool().unwrap().capacity();
                let few = spread(&world, SPARSE_FEW.min(room));
                world.write(&few).unwrap();
                measure(&mut points, &mut filler, &key, "short", &mut world, 1);
                let many = spread(&world, room);
                world.write(&many).unwrap();
                measure(&mut points, &mut filler, &key, "large", &mut world, LONG);
                drop(world);
                if let Some(mut far) = far_of(host, name, &params, SPARSE, true) {
                    let room = far.pool().unwrap().capacity();
                    let many = spread(&far, room);
                    far.write(&many).unwrap();
                    measure(&mut points, &mut filler, &key, "far", &mut far, LONG);
                }
            }
        }
        points
    }

    const fn sand(size: [u32; 3]) -> Made<'static> {
        Made {
            rule: "sand",
            params: &[],
            size,
            sparse: false,
            slots: None,
        }
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
                "FLOOR {} {} {}: {} units, floor {:?}, fastest wall {:?}",
                host.name(),
                one.key.name(),
                one.name,
                one.units,
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
            .map(|(rate, _)| rate.predict(one.units, 1))
    }

    fn up_front(points: &[Point]) -> Vec<Key> {
        let mut keys: Vec<Key> = points.iter().map(|one| one.key.clone()).collect();
        keys.dedup();
        keys
    }

    fn one_sided(
        label: &str,
        points: &[Point],
        found: &BTreeMap<Key, Solved>,
        bound: f64,
    ) -> Vec<String> {
        let unknown: Vec<String> = up_front(points)
            .iter()
            .filter(|key| !found.contains_key(*key))
            .map(Key::name)
            .collect();
        eprintln!(
            "{label}: {} up-front keys, {} unknown {unknown:?}",
            up_front(points).len(),
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
                "{label} {} {}: predicted {guess:?}, floor {:?}, P/R {ratio:.3}",
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
        eprintln!("WORST {label}: P/R {worst:.3}");
        failures
    }

    fn deterministic(
        label: &str,
        host: &Host,
        prepare: impl FnOnce(),
        calibrate: impl FnOnce() -> BTreeMap<Key, Solved>,
    ) {
        let points = floors(host);
        prepare();
        let found = calibrate();
        let failures = one_sided(label, &points, &found, K);
        assert!(failures.is_empty(), "{label}: {failures:#?}");
    }

    fn wanted(host: &Host) -> Vec<Key> {
        let mut wanted = Vec::new();
        for rule in grit::serve3::rules_on(Some(host)) {
            let params = vec![0_u32; rule.params()];
            let made = |size, sparse| Made {
                rule: rule.name(),
                params: &params,
                size,
                sparse,
                slots: None,
            };
            if World::create(host, &made(SMALL, false)).is_ok()
                && World::create(host, &made(LARGE, false)).is_ok()
            {
                wanted.push(Key::rule(rule.name(), false));
            }
            if World::create(host, &made(SPARSE, true)).is_ok() {
                wanted.push(Key::rule(rule.name(), true));
            }
        }
        wanted
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
            assert!(measured(&host).unwrap().is_empty());
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
        let found = measured(&host).unwrap();
        let wanted = wanted(&host);
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
    fn c1_a_quiet_calibration_is_at_most_k_times_the_floor() {
        let Some(host) = gpu("c1 quiet") else {
            return;
        };
        let _alone = alone();
        deterministic("c1 quiet", &host, || {}, || measured(&host).unwrap());
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
                let found = measured(&host);
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
                let found = measured(&host);
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
                        let mut world = World::create(&host, &sand(LARGE)).unwrap();
                        while !stop.load(Ordering::Relaxed) {
                            world.step(64).unwrap();
                            steps.fetch_add(1, Ordering::Relaxed);
                        }
                    });
                    while steps.load(Ordering::Relaxed) < 2 {
                        std::thread::yield_now();
                    }
                    let found = measured(&host);
                    stop.store(true, Ordering::Relaxed);
                    busy.join().unwrap();
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
            || measured(&host).unwrap(),
        );
    }

    #[test]
    fn c1_real_cpu_load_stays_under_the_margin() {
        let Some(host) = gpu("c1 load") else {
            return;
        };
        let _alone = alone();
        let points = floors(&host);
        let cores = std::thread::available_parallelism().map_or(4, std::num::NonZero::get);
        let load = Load::start(2 * cores, LOAD_MOST);
        let began = Instant::now();
        let found = measured(&host);
        let took = began.elapsed();
        let fired = load.fired();
        drop(load);
        assert!(
            !fired,
            "the calibration did not finish within {LOAD_MOST:?} under {} spinners on {cores} cores; the load was stopped after {took:?}",
            2 * cores
        );
        assert_eq!(
            SPINNING.load(Ordering::SeqCst),
            0,
            "the load threads did not stop"
        );
        let found = found.unwrap();
        eprintln!(
            "c1 load: {} spinners on {cores} cores, calibration took {took:?}",
            2 * cores
        );
        let above: Vec<String> = points
            .iter()
            .filter_map(|one| {
                let ratio = predicted(&found, one)?.as_secs_f64() / one.floor.as_secs_f64();
                (ratio > 3.0).then(|| format!("{} {}: P/R {ratio:.3}", one.key.name(), one.name))
            })
            .collect();
        eprintln!("ABOVE-3 c1 load {}: {above:?}", host.name());
        let failures = one_sided("c1 load", &points, &found, SMOKE);
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
        let began = Instant::now();
        let got = calibrate::taken(&host, None).unwrap();
        let took = began.elapsed();
        calibrate::forget(&device);
        eprintln!(
            "STARTUP {} driver {} width {}: calibration took {} ms",
            device.name,
            device.driver,
            device.width,
            took.as_millis()
        );
        let points = floors(&host);
        let table = calibrate::read(&got.table);
        let mut failures = Vec::new();
        for one in &points {
            let Some(rate) = table.rate(&one.key) else {
                failures.push(format!("{} unknown", one.key.name()));
                continue;
            };
            let guess = rate.predict(one.units, 1);
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
            if one.name != "far" && low > 10.0 {
                failures.push(format!(
                    "{} {}: R/P {low:.3} above 10 (low side)",
                    one.key.name(),
                    one.name
                ));
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
    }

    fn agree(
        label: &str,
        points: &[Point],
        one: &BTreeMap<Key, Solved>,
        other: &BTreeMap<Key, Solved>,
    ) -> Vec<String> {
        let mut failures = Vec::new();
        for point in points {
            let (Some(left), Some(right)) = (predicted(one, point), predicted(other, point)) else {
                failures.push(format!(
                    "{} {}: unknown in one of the two",
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
                failures.push(format!("{} {}: {ratio:.3}", point.key.name(), point.name));
            }
        }
        failures
    }

    #[test]
    fn c6_idle_and_busy_calibrations_agree() {
        let Some(host) = gpu("c6") else {
            return;
        };
        let _alone = alone();
        let points = floors(&host);
        let mut filler = World::create(&host, &sand(LARGE)).unwrap();
        filler.write(&seeded(LARGE)).unwrap();
        let mut failures = Vec::new();
        for round in 0..10 {
            std::thread::sleep(Duration::from_secs(5));
            let idle = measured(&host).unwrap();
            warmed(&mut filler);
            let busy = measured(&host).unwrap();
            failures.extend(agree(&format!("c6 round {round}"), &points, &idle, &busy));
        }
        assert!(failures.is_empty(), "{failures:#?}");
    }
}
