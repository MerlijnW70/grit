#![cfg(feature = "faults")]
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use common::device;

use grit::Host;
use grit::serve3::budget::{Meter, RECENT, Spent, Work};
use grit::serve3::journal::{Entry, Keep, Ledger, Made};
use grit::serve3::{Options, Server};
use grit::wire::{Json, written};

const SIDE: u32 = 64;

const SPINNERS: usize = 15;

const SPIN: Duration = Duration::from_millis(19);

const GAP: Duration = Duration::from_millis(1);

const CELLS: u32 = 64;

const FRAMES: u32 = 40;

fn line(name: &str, arguments: &str) -> String {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{{\"name\":\"{name}\",\"arguments\":{arguments}}}}}"
    )
}

fn call(server: &mut Server<'_>, name: &str, arguments: &str) -> Json {
    server.heard(&line(name, arguments)).unwrap()
}

fn outcome(got: &Json, error: bool) -> Json {
    let result = got
        .get("result")
        .unwrap_or_else(|| panic!("{}", written(got)));
    assert_eq!(
        result.get("isError"),
        Some(&Json::Bool(error)),
        "{}",
        written(got)
    );
    result.get("structuredContent").cloned().unwrap()
}

fn answered(got: &Json) -> Json {
    outcome(got, false)
}

fn refused(got: &Json) -> Json {
    outcome(got, true)
}

fn int_at(held: &Json, key: &str) -> i64 {
    held.get(key)
        .and_then(Json::int)
        .unwrap_or_else(|| panic!("no {key} in {}", written(held)))
}

fn text_at(held: &Json, key: &str) -> String {
    held.get(key)
        .and_then(Json::text)
        .unwrap_or_else(|| panic!("no {key} in {}", written(held)))
        .to_owned()
}

fn budgeted(budget: Option<Duration>) -> String {
    budget.map_or(String::new(), |held| {
        format!(",\"budget_ms\":{}", held.as_millis().max(1))
    })
}

fn create(server: &mut Server<'_>, budget: Option<Duration>) -> Json {
    call(
        server,
        "grit_world_create",
        &format!(
            "{{\"wide\":{SIDE},\"deep\":{SIDE},\"tall\":{SIDE},\"rule\":\"sand\"{}}}",
            budgeted(budget)
        ),
    )
}

fn close(server: &mut Server<'_>, world: i64) {
    answered(&call(
        server,
        "grit_world_close",
        &format!("{{\"world\":{world}}}"),
    ));
}

fn fork(server: &mut Server<'_>, world: i64, budget: Option<Duration>) -> Json {
    call(
        server,
        "grit_world_fork",
        &format!("{{\"world\":{world}{}}}", budgeted(budget)),
    )
}

fn cells() -> String {
    let cells: Vec<String> = (0..CELLS)
        .map(|at| format!("[{},0,{},1]", at % SIDE, at / SIDE))
        .collect();
    format!("[{}]", cells.join(","))
}

fn write(server: &mut Server<'_>, world: i64, budget: Option<Duration>) -> Json {
    call(
        server,
        "grit_world_write",
        &format!(
            "{{\"world\":{world},\"cells\":{}{}}}",
            cells(),
            budgeted(budget)
        ),
    )
}

fn timed<T>(work: impl FnOnce() -> T) -> (T, Duration) {
    let began = Instant::now();
    let done = work();
    (done, began.elapsed())
}

fn short_of(fits: Duration) -> Duration {
    (fits * 3).max(Duration::from_millis(20))
}

fn needs_at_least(told: &Json) {
    assert_eq!(text_at(told, "code"), "budget", "{}", written(told));
    assert_eq!(text_at(told, "field"), "$.budget_ms", "{}", written(told));
    let message = text_at(told, "message");
    assert!(
        message.contains("needs at least") && message.contains("nothing ran"),
        "{message}"
    );
}

fn off_cpu(server: &Server<'_>, work: Work, rounds: usize, by: Duration) {
    let faults = server.faults();
    let mut armed = faults.lock().unwrap();
    armed.off_cpu = vec![work; rounds];
    armed.off_cpu_by = by;
}

fn device_slow(server: &Server<'_>, work: Work, rounds: usize, by: Duration) {
    let faults = server.faults();
    let mut armed = faults.lock().unwrap();
    armed.held_work = vec![work; rounds];
    armed.held_by = by;
}

fn unarmed(server: &Server<'_>) {
    let faults = server.faults();
    let armed = faults.lock().unwrap();
    assert!(
        armed.off_cpu.is_empty(),
        "setup: an off-cpu hold was not used"
    );
    assert!(
        armed.held_work.is_empty(),
        "setup: a device hold was not used"
    );
}

#[derive(Clone, Copy)]
enum Hold {
    Hook,
    Contention,
}

mod pin {
    #[cfg(target_os = "linux")]
    pub type Mask = [u64; 16];

    #[cfg(target_os = "linux")]
    unsafe extern "C" {
        fn sched_getaffinity(pid: i32, size: usize, mask: *mut u64) -> i32;
        fn sched_setaffinity(pid: i32, size: usize, mask: *const u64) -> i32;
    }

    #[cfg(target_os = "linux")]
    pub fn current() -> Mask {
        let mut mask = [0_u64; 16];
        let read = unsafe { sched_getaffinity(0, size_of::<Mask>(), mask.as_mut_ptr()) };
        assert_eq!(read, 0, "sched_getaffinity failed");
        mask
    }

    #[cfg(target_os = "linux")]
    pub fn set(mask: &Mask) {
        let done = unsafe { sched_setaffinity(0, size_of::<Mask>(), mask.as_ptr()) };
        assert_eq!(done, 0, "sched_setaffinity failed");
    }

    #[cfg(target_os = "linux")]
    pub fn core(nth: usize) -> Mask {
        let allowed = current();
        let cores: Vec<usize> = (0..allowed.len() * 64)
            .filter(|at| allowed[at / 64] >> (at % 64) & 1 == 1)
            .collect();
        let chosen = cores[nth % cores.len()];
        let mut only = [0_u64; 16];
        only[chosen / 64] = 1 << (chosen % 64);
        only
    }

    #[cfg(target_os = "linux")]
    pub const fn yielding(_: bool) {}

    #[cfg(windows)]
    pub fn yielding(on: bool) {
        const IDLE: i32 = -15;
        const NORMAL: i32 = 0;
        let done = unsafe { SetThreadPriority(GetCurrentThread(), if on { IDLE } else { NORMAL }) };
        assert_ne!(done, 0, "SetThreadPriority failed");
    }

    #[cfg(windows)]
    pub type Mask = usize;

    #[cfg(windows)]
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentThread() -> *mut std::ffi::c_void;
        fn GetCurrentProcess() -> *mut std::ffi::c_void;
        fn SetThreadAffinityMask(thread: *mut std::ffi::c_void, mask: usize) -> usize;
        fn SetThreadPriority(thread: *mut std::ffi::c_void, priority: i32) -> i32;
        fn GetProcessAffinityMask(
            process: *mut std::ffi::c_void,
            mask: *mut usize,
            system: *mut usize,
        ) -> i32;
    }

    #[cfg(windows)]
    fn allowed() -> usize {
        let (mut process, mut system) = (0_usize, 0_usize);
        let read = unsafe {
            GetProcessAffinityMask(GetCurrentProcess(), &raw mut process, &raw mut system)
        };
        assert_ne!(read, 0, "GetProcessAffinityMask failed");
        process
    }

    #[cfg(windows)]
    pub fn current() -> Mask {
        let old = unsafe { SetThreadAffinityMask(GetCurrentThread(), allowed()) };
        assert_ne!(old, 0, "SetThreadAffinityMask failed");
        set(&old);
        old
    }

    #[cfg(windows)]
    pub fn set(mask: &Mask) {
        let old = unsafe { SetThreadAffinityMask(GetCurrentThread(), *mask) };
        assert_ne!(old, 0, "SetThreadAffinityMask failed");
    }

    #[cfg(windows)]
    pub fn core(nth: usize) -> Mask {
        let allowed = allowed();
        let cores: Vec<usize> = (0..usize::BITS as usize)
            .filter(|at| allowed >> at & 1 == 1)
            .collect();
        1 << cores[nth % cores.len()]
    }
}

struct Starved {
    saved: pin::Mask,
    stop: Arc<AtomicBool>,
    spinners: Vec<std::thread::JoinHandle<()>>,
}

impl Starved {
    fn on(core: pin::Mask, count: usize) -> Self {
        let saved = pin::current();
        let stop = Arc::new(AtomicBool::new(false));
        let spinners = (0..count)
            .map(|_| {
                let stop = Arc::clone(&stop);
                std::thread::spawn(move || {
                    pin::set(&core);
                    while !stop.load(Ordering::Relaxed) {
                        let began = Instant::now();
                        while began.elapsed() < SPIN {
                            std::hint::spin_loop();
                        }
                        std::thread::sleep(GAP);
                    }
                })
            })
            .collect();
        pin::set(&core);
        pin::yielding(true);
        Self {
            saved,
            stop,
            spinners,
        }
    }
}

impl Drop for Starved {
    fn drop(&mut self) {
        pin::yielding(false);
        pin::set(&self.saved);
        self.stop.store(true, Ordering::Relaxed);
        for spinner in self.spinners.drain(..) {
            let _ = spinner.join();
        }
    }
}

fn opened_on(hold: Hold, core: pin::Mask, label: &str) -> Option<Host> {
    if matches!(hold, Hold::Hook) {
        return device(label);
    }
    let saved = pin::current();
    pin::set(&core);
    let host = device(label);
    pin::set(&saved);
    host
}

const LONG: Duration = Duration::from_secs(600);

const ESCALATIONS: usize = 2;

fn plain_rounds(mut work: impl FnMut() -> Duration) -> Duration {
    work();
    (0..RECENT).map(|_| work()).max().unwrap()
}

fn held_rounds(
    hold: Hold,
    core: pin::Mask,
    server: &mut Server<'_>,
    budget: Duration,
    arm: impl Fn(&Server<'_>),
    mut work: impl FnMut(&mut Server<'_>),
    label: &str,
) {
    let mut spinners = SPINNERS;
    for escalation in 0..=ESCALATIONS {
        let mut least = Duration::MAX;
        for round in 0..RECENT {
            let took = match hold {
                Hold::Hook => {
                    arm(server);
                    timed(|| work(server)).1
                }
                Hold::Contention => {
                    let starved = Starved::on(core, spinners);
                    let took = timed(|| work(server)).1;
                    drop(starved);
                    took
                }
            };
            eprintln!("{label}: held round {round} beside {spinners} spinners took {took:?}");
            least = least.min(took);
        }
        if least > budget {
            return;
        }
        assert!(
            matches!(hold, Hold::Contention) && escalation < ESCALATIONS,
            "setup: {label} held at least {least:?}, budget {budget:?}"
        );
        spinners *= 4;
    }
}

fn e1_build(hold: Hold, label: &str) {
    let core = pin::core(0);
    let Some(host) = opened_on(hold, core, label) else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let fits = plain_rounds(|| {
        let (made, took) = timed(|| answered(&create(&mut server, None)));
        close(&mut server, int_at(&made, "world"));
        took
    });
    let budget = short_of(fits);
    held_rounds(
        hold,
        core,
        &mut server,
        budget,
        |server| off_cpu(server, Work::Build, 1, budget * 2),
        |server| {
            let made = answered(&create(server, Some(LONG)));
            close(server, int_at(&made, "world"));
        },
        label,
    );
    unarmed(&server);
    let got = create(&mut server, Some(budget));
    eprintln!(
        "{label}: build with {budget:?}, plain at most {fits:?}: {}",
        written(&got)
    );
    let made = answered(&got);
    close(&mut server, int_at(&made, "world"));
}

fn e1_fork(hold: Hold, label: &str) {
    let core = pin::core(1);
    let Some(host) = opened_on(hold, core, label) else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let world = int_at(&answered(&create(&mut server, None)), "world");
    let fits = plain_rounds(|| {
        let (forked, took) = timed(|| answered(&fork(&mut server, world, None)));
        close(&mut server, int_at(&forked, "world"));
        took
    });
    let budget = short_of(fits);
    held_rounds(
        hold,
        core,
        &mut server,
        budget,
        |server| off_cpu(server, Work::Fork, 1, budget * 2),
        |server| {
            let forked = answered(&fork(server, world, Some(LONG)));
            close(server, int_at(&forked, "world"));
        },
        label,
    );
    unarmed(&server);
    let got = fork(&mut server, world, Some(budget));
    eprintln!(
        "{label}: fork with {budget:?}, plain at most {fits:?}: {}",
        written(&got)
    );
    let forked = answered(&got);
    close(&mut server, int_at(&forked, "world"));
}

fn whole_write(got: &Json) {
    let wrote = answered(got);
    assert!(wrote.get("stopped").is_none(), "{}", written(got));
    assert_eq!(
        int_at(&wrote, "written"),
        i64::from(CELLS),
        "{}",
        written(got)
    );
}

fn unrefused_write(got: &Json) {
    let wrote = answered(got);
    assert!(int_at(&wrote, "written") > 0, "{}", written(got));
    if wrote.get("stopped").is_none() {
        assert_eq!(
            int_at(&wrote, "written"),
            i64::from(CELLS),
            "{}",
            written(got)
        );
    } else {
        assert_eq!(text_at(&wrote, "stopped"), "budget", "{}", written(got));
    }
}

fn e1_write(hold: Hold, label: &str) {
    let core = pin::core(2);
    let Some(host) = opened_on(hold, core, label) else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let world = int_at(&answered(&create(&mut server, None)), "world");
    let fits = plain_rounds(|| {
        let (got, took) = timed(|| write(&mut server, world, None));
        whole_write(&got);
        took
    });
    let budget = short_of(fits);
    held_rounds(
        hold,
        core,
        &mut server,
        budget,
        |server| off_cpu(server, Work::Write, 1, budget * 2),
        |server| whole_write(&write(server, world, Some(LONG))),
        label,
    );
    unarmed(&server);
    let got = write(&mut server, world, Some(budget));
    eprintln!(
        "{label}: write with {budget:?}, plain at most {fits:?}: {}",
        written(&got)
    );
    unrefused_write(&got);
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "grit-floors-{label}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn kept(&self, worlds: u32) -> Options {
        let dir = self.0.join("worlds");
        let keep = Keep::at(&dir).unwrap();
        for number in 1..=worlds {
            let mut ledger = Ledger::created(Made {
                rule: "sand".to_owned(),
                params: Vec::new(),
                size: [32, 32, 32],
                sparse: false,
                slots: None,
            });
            ledger.push(Entry::Step(FRAMES), Duration::from_millis(100));
            keep.begin(number, &ledger).unwrap();
        }
        keep.set_next(worlds + 1).unwrap();
        Options {
            dir: Some(dir),
            memory: None,
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn open(server: &mut Server<'_>, number: u32, budget: Option<Duration>) -> Json {
    call(
        server,
        "grit_world_open",
        &format!("{{\"name\":\"world-{number}\"{}}}", budgeted(budget)),
    )
}

fn replays_held(server: &Server<'_>, by: Duration) {
    let faults = server.faults();
    let mut armed = faults.lock().unwrap();
    armed.held_replays = 1;
    armed.held_by = by;
}

const KEPT: u32 = 96;

const LONG_FRAMES: u32 = 4000;

static STARVING: std::sync::RwLock<()> = std::sync::RwLock::new(());

fn beside() -> std::sync::RwLockReadGuard<'static, ()> {
    STARVING
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn starving() -> std::sync::RwLockWriteGuard<'static, ()> {
    STARVING
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn e1_replay(hold: Hold, label: &str) {
    let core = pin::core(3);
    let Some(host) = opened_on(hold, core, label) else {
        return;
    };
    let scratch = Scratch::new(label);
    let mut server = Server::with(Some(&host), &scratch.kept(KEPT)).unwrap();
    let mut number = 0;
    let fits = plain_rounds(|| {
        number += 1;
        let (got, took) = timed(|| open(&mut server, number, None));
        answered(&got);
        took
    });
    let budget = short_of(fits);
    let number = std::cell::Cell::new(number);
    held_rounds(
        hold,
        core,
        &mut server,
        budget,
        |server| replays_held(server, budget * 2),
        |server| {
            number.set(number.get() + 1);
            answered(&open(server, number.get(), Some(LONG)));
        },
        label,
    );
    assert_eq!(server.faults().lock().unwrap().held_replays, 0);
    let got = open(&mut server, number.get() + 1, Some(budget));
    eprintln!(
        "{label}: open with {budget:?}, plain at most {fits:?}: {}",
        written(&got)
    );
    assert_eq!(int_at(&answered(&got), "frame"), i64::from(FRAMES));
}

#[test]
fn e1_a_build_held_off_its_cpu_by_a_hook_never_refuses_the_same_build() {
    let _beside = beside();
    e1_build(Hold::Hook, "e1 build hook");
}

#[test]
fn e1_a_build_starved_of_its_cpu_never_refuses_the_same_build() {
    let _starving = starving();
    e1_build(Hold::Contention, "e1 build contention");
}

#[test]
fn e1_a_fork_held_off_its_cpu_by_a_hook_never_refuses_the_same_fork() {
    let _beside = beside();
    e1_fork(Hold::Hook, "e1 fork hook");
}

#[test]
fn e1_a_fork_starved_of_its_cpu_never_refuses_the_same_fork() {
    let _starving = starving();
    e1_fork(Hold::Contention, "e1 fork contention");
}

#[test]
fn e1_a_write_held_off_its_cpu_by_a_hook_is_never_refused() {
    let _beside = beside();
    e1_write(Hold::Hook, "e1 write hook");
}

#[test]
fn e1_a_write_starved_of_its_cpu_is_never_refused() {
    let _starving = starving();
    e1_write(Hold::Contention, "e1 write contention");
}

#[test]
fn e1_a_replay_held_off_its_cpu_by_a_hook_never_refuses_the_same_replay() {
    let _beside = beside();
    e1_replay(Hold::Hook, "e1 replay hook");
}

#[test]
fn e1_a_replay_starved_of_its_cpu_never_refuses_the_same_replay() {
    let _starving = starving();
    e1_replay(Hold::Contention, "e1 replay contention");
}

#[test]
fn e1_cut_replays_whose_builds_were_held_off_their_cpu_never_refuse_the_next_open() {
    let _beside = beside();
    let Some(host) = device("e1 cut replays") else {
        return;
    };
    let scratch = Scratch::new("e1-cut");
    let dir = scratch.0.join("worlds");
    let keep = Keep::at(&dir).unwrap();
    let mut ledger = Ledger::created(Made {
        rule: "sand".to_owned(),
        params: Vec::new(),
        size: [32, 32, 32],
        sparse: false,
        slots: None,
    });
    ledger.push(Entry::Step(LONG_FRAMES), Duration::from_millis(400));
    keep.begin(1, &ledger).unwrap();
    keep.set_next(2).unwrap();
    drop(keep);
    let options = Options {
        dir: Some(dir),
        memory: None,
    };
    let mut server = Server::with(Some(&host), &options).unwrap();
    let cube = "{\"wide\":32,\"deep\":32,\"tall\":32,\"rule\":\"sand\"}";
    let fits = plain_rounds(|| {
        let (made, took) = timed(|| answered(&call(&mut server, "grit_world_create", cube)));
        close(&mut server, int_at(&made, "world"));
        took
    });
    let budget = short_of(fits).max(Duration::from_millis(600));
    grit::serve3::tools::faults::stall_every(budget / 64);
    off_cpu(&server, Work::Build, RECENT, budget * 2);
    for round in 0..RECENT {
        let told = refused(&open(&mut server, 1, Some(budget)));
        let message = text_at(&told, "message");
        eprintln!("held build {round} with {budget:?}: {message}");
        assert!(message.contains("nothing was opened"), "{message}");
    }
    unarmed(&server);
    let told = refused(&open(&mut server, 1, Some(budget)));
    grit::serve3::tools::faults::disarm();
    let message = text_at(&told, "message");
    eprintln!("open with {budget:?} after {RECENT} held builds: {message}");
    assert!(
        message.contains("nothing was opened") && !message.contains("needs at least"),
        "{message}"
    );
}

#[test]
fn e2_a_build_the_device_itself_makes_slow_is_refused_up_front_after_holds_off_its_cpu() {
    let _beside = beside();
    let Some(host) = device("e2 build") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let fits = plain_rounds(|| {
        let (made, took) = timed(|| answered(&create(&mut server, None)));
        close(&mut server, int_at(&made, "world"));
        took
    });
    let budget = short_of(fits);
    off_cpu(&server, Work::Build, RECENT, budget * 2);
    for _ in 0..RECENT {
        let made = answered(&create(&mut server, None));
        close(&mut server, int_at(&made, "world"));
    }
    let got = create(&mut server, Some(budget));
    let made = answered(&got);
    close(&mut server, int_at(&made, "world"));
    device_slow(&server, Work::Build, RECENT, budget * 2);
    for _ in 0..RECENT {
        let made = answered(&create(&mut server, None));
        close(&mut server, int_at(&made, "world"));
    }
    unarmed(&server);
    let got = create(&mut server, Some(budget));
    eprintln!("e2 build with {budget:?}: {}", written(&got));
    let told = refused(&got);
    needs_at_least(&told);
    assert!(
        u128::try_from(int_at(&told, "limit")).unwrap() > (budget * 2).as_millis(),
        "{}",
        written(&told)
    );
    let busy = server.faults();
    busy.lock().unwrap().held_work = vec![Work::Build];
    refused(&create(&mut server, Some(budget)));
    assert_eq!(
        busy.lock().unwrap().held_work.len(),
        1,
        "the refused build ran"
    );
}

#[test]
fn e2_a_fork_the_device_itself_makes_slow_is_refused_up_front() {
    let _beside = beside();
    let Some(host) = device("e2 fork") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let world = int_at(&answered(&create(&mut server, None)), "world");
    let fits = plain_rounds(|| {
        let (forked, took) = timed(|| answered(&fork(&mut server, world, None)));
        close(&mut server, int_at(&forked, "world"));
        took
    });
    let budget = short_of(fits);
    device_slow(&server, Work::Fork, RECENT, budget * 2);
    for _ in 0..RECENT {
        let forked = answered(&fork(&mut server, world, None));
        close(&mut server, int_at(&forked, "world"));
    }
    unarmed(&server);
    let got = fork(&mut server, world, Some(budget));
    eprintln!("e2 fork with {budget:?}: {}", written(&got));
    needs_at_least(&refused(&got));
}

#[test]
fn e2_a_replay_the_device_itself_makes_slow_is_refused_up_front() {
    let _beside = beside();
    let Some(host) = device("e2 replay") else {
        return;
    };
    let scratch = Scratch::new("e2-replay");
    let worlds = 2 * u32::try_from(RECENT).unwrap() + 2;
    let mut server = Server::with(Some(&host), &scratch.kept(worlds)).unwrap();
    let mut number = 0;
    let fits = plain_rounds(|| {
        number += 1;
        let (got, took) = timed(|| open(&mut server, number, None));
        answered(&got);
        took
    });
    let budget = short_of(fits);
    let frame = (budget * 2) / FRAMES;
    grit::serve3::tools::faults::stall_every(frame);
    for _ in 0..RECENT {
        number += 1;
        answered(&open(&mut server, number, None));
    }
    grit::serve3::tools::faults::disarm();
    number += 1;
    let got = open(&mut server, number, Some(budget));
    eprintln!("e2 replay with {budget:?}: {}", written(&got));
    let told = refused(&got);
    needs_at_least(&told);
    assert!(
        text_at(&told, "message").contains("rebuilding world"),
        "{}",
        written(&told)
    );
}

fn sequence(server: &mut Server<'_>) -> Vec<String> {
    let mut told = Vec::new();
    let made = answered(&create(server, None));
    told.push(written(&made));
    let world = int_at(&made, "world");
    told.push(written(&answered(&write(server, world, None))));
    told.push(written(&answered(&call(
        server,
        "grit_world_step",
        &format!("{{\"world\":{world},\"frames\":{FRAMES}}}"),
    ))));
    let forked = answered(&fork(server, world, None));
    told.push(written(&forked));
    told.push(written(&answered(&call(
        server,
        "grit_world_read",
        &format!(
            "{{\"world\":{},\"from\":[0,0,0],\"to\":[{},0,3]}}",
            int_at(&forked, "world"),
            SIDE - 1
        ),
    ))));
    close(server, int_at(&forked, "world"));
    close(server, world);
    told
}

#[test]
fn e4_holds_off_the_cpu_never_change_the_bytes_of_an_answer() {
    let _beside = beside();
    let Some(host) = device("e4") else {
        return;
    };
    let mut plain = Server::new(Some(&host));
    let first = sequence(&mut plain);
    let mut held = Server::new(Some(&host));
    {
        let faults = held.faults();
        let mut armed = faults.lock().unwrap();
        armed.off_cpu = vec![Work::Build, Work::Write, Work::Fork];
        armed.off_cpu_by = Duration::from_millis(300);
    }
    let second = sequence(&mut held);
    unarmed(&held);
    assert_eq!(first, second);
}

const HELD_TRANSFER_MOST: Duration = Duration::from_millis(200);

struct Transfers;

impl Transfers {
    fn slowed(by: Duration) -> Self {
        grit::resident::faults::slow_transfers(by);
        Self
    }
}

impl Drop for Transfers {
    fn drop(&mut self) {
        grit::resident::faults::slow_transfers(Duration::ZERO);
    }
}

fn held_on_the_device<T>(budget: Duration, mut work: impl FnMut() -> T, label: &str) {
    let each = (budget * 2).min(HELD_TRANSFER_MOST);
    eprintln!("{label}: each transfer held {each:?}, budget {budget:?}");
    let slowed = Transfers::slowed(each);
    for round in 0..RECENT {
        let (_, took) = timed(&mut work);
        eprintln!("{label}: device held round {round} took {took:?}");
        assert!(
            took > budget,
            "setup: {label} took {took:?}, budget {budget:?}"
        );
    }
    drop(slowed);
}

fn refused_without_running(got: &Json, took: Duration, budget: Duration, label: &str) {
    eprintln!(
        "{label} with {budget:?} answered in {took:?}: {}",
        written(got)
    );
    needs_at_least(&refused(got));
    assert!(took < budget, "{label}: the refused call ran for {took:?}");
}

#[test]
fn e2_a_build_whose_transfers_the_device_itself_holds_is_refused_up_front() {
    let _beside = beside();
    let Some(host) = device("e2 build transfers") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let fits = plain_rounds(|| {
        let (made, took) = timed(|| answered(&create(&mut server, None)));
        close(&mut server, int_at(&made, "world"));
        took
    });
    let budget = short_of(fits);
    held_on_the_device(
        budget,
        || {
            let made = answered(&create(&mut server, None));
            close(&mut server, int_at(&made, "world"));
        },
        "e2 build transfers",
    );
    let (got, took) = timed(|| create(&mut server, Some(budget)));
    refused_without_running(&got, took, budget, "e2 build transfers");
}

#[test]
fn e2_a_fork_whose_copies_the_device_itself_holds_is_refused_up_front() {
    let _beside = beside();
    let Some(host) = device("e2 fork copies") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let world = int_at(&answered(&create(&mut server, None)), "world");
    let fits = plain_rounds(|| {
        let (forked, took) = timed(|| answered(&fork(&mut server, world, None)));
        close(&mut server, int_at(&forked, "world"));
        took
    });
    let budget = short_of(fits);
    held_on_the_device(
        budget,
        || {
            let forked = answered(&fork(&mut server, world, None));
            close(&mut server, int_at(&forked, "world"));
        },
        "e2 fork copies",
    );
    let (got, took) = timed(|| fork(&mut server, world, Some(budget)));
    refused_without_running(&got, took, budget, "e2 fork copies");
}

fn shown_as_waiting(spent: Spent, budget: Duration, label: &str) {
    assert!(
        spent.wall > budget,
        "setup: {label} held {spent:?}, budget {budget:?}"
    );
    assert!(
        spent.own > budget,
        "setup: {label} held {spent:?}, budget {budget:?}: no core it may run on was idle while it waited, so nothing shows the wait was not for a cpu"
    );
}

struct Disk;

impl Disk {
    fn slowed(by: Duration) -> Self {
        grit::host::disk::slow_sync(by);
        Self
    }
}

impl Drop for Disk {
    fn drop(&mut self) {
        grit::host::disk::slow_sync(Duration::ZERO);
    }
}

fn save(server: &mut Server<'_>, world: i64, name: u32, budget: Option<Duration>) -> Json {
    call(
        server,
        "grit_world_save",
        &format!(
            "{{\"world\":{world},\"name\":\"saved-{name}\"{}}}",
            budgeted(budget)
        ),
    )
}

#[test]
fn e2_a_save_whose_sync_the_disk_itself_holds_is_refused_up_front() {
    let _beside = beside();
    let Some(host) = device("e2 save sync") else {
        return;
    };
    let scratch = Scratch::new("e2-save");
    let mut server = Server::with(Some(&host), &scratch.kept(0)).unwrap();
    let world = int_at(&answered(&create(&mut server, None)), "world");
    let mut name = 0;
    let fits = plain_rounds(|| {
        name += 1;
        let (got, took) = timed(|| save(&mut server, world, name, None));
        answered(&got);
        took
    });
    let budget = short_of(fits);
    let slowed = Disk::slowed(budget * 2);
    for round in 0..RECENT {
        name += 1;
        let meter = Meter::start();
        answered(&save(&mut server, world, name, None));
        let spent = meter.spent();
        eprintln!("e2 save sync: disk held round {round}: {spent:?}");
        shown_as_waiting(spent, budget, "e2 save sync");
    }
    drop(slowed);
    name += 1;
    let (got, took) = timed(|| save(&mut server, world, name, Some(budget)));
    refused_without_running(&got, took, budget, "e2 save sync");
}

struct Allocations;

impl Allocations {
    fn slowed(by: Duration) -> Self {
        grit::host::driver::slow_allocations(by);
        Self
    }
}

impl Drop for Allocations {
    fn drop(&mut self) {
        grit::host::driver::slow_allocations(Duration::ZERO);
    }
}

#[test]
fn e2_a_build_whose_allocations_the_driver_holds_is_refused_up_front() {
    let _starving = starving();
    let Some(host) = device("e2 build allocations") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let fits = plain_rounds(|| {
        let (made, took) = timed(|| answered(&create(&mut server, None)));
        close(&mut server, int_at(&made, "world"));
        took
    });
    let budget = short_of(fits);
    let slowed = Allocations::slowed(budget * 2);
    for round in 0..RECENT {
        let meter = Meter::start();
        let made = answered(&create(&mut server, None));
        let spent = meter.spent();
        close(&mut server, int_at(&made, "world"));
        eprintln!("e2 build allocations: driver held round {round}: {spent:?}");
        shown_as_waiting(spent, budget, "e2 build allocations");
    }
    drop(slowed);
    let (got, took) = timed(|| create(&mut server, Some(budget)));
    refused_without_running(&got, took, budget, "e2 build allocations");
}
