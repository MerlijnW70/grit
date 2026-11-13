//! The world tools: a solid world of the 3D listed engine that an agent
//! creates, writes, steps, reads and closes through MCP calls, without a
//! screen. [`Server`] holds the open worlds between calls and answers every
//! 2D tool through [`crate::serve::handle`] as before.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use simdr::lanes::LaneError;

use crate::branch::World;
use crate::bricks::{Bricks, PoolUse};
use crate::compact::Plan;
use crate::host::Host;
use crate::layout::{BINDINGS, Dimension};
use crate::live::{self, Loop};
use crate::rule3::Rule;
use crate::rule3::code::{self, Code};
use crate::rule3::embed::{self, Embed};
use crate::rule3::evo::{self, Evo};
use crate::rule3::heat::{self, Heat};
use crate::rule3::nature::{self, Nature};
use crate::rule3::sand::{self, Sand};
use crate::sand::Grid;
use crate::sand3::Grid3;
use crate::sweep;
use crate::wire::{Json, counted, map, text};

pub mod ask;
pub mod branch;
pub mod budget;
pub mod errors;
pub mod exec;
pub mod journal;
pub mod law;
pub mod learn;
pub mod schema;
pub mod tools;
pub mod transport;

use budget::{Clock, Memory, Pace, Short, Stop};
use errors::{Refusal, refusal_of};
use journal::{Base, Entry, Keep, Ledger, Made};
use schema::Check;

/// The most cells one `grit_world_read` returns, and one `grit_world_write`
/// takes: a box of 16 by 16 by 16.
pub const READ_MOST: u64 = 4096;

/// The most cells one `grit_world_slice` returns: a layer of 256 by 256.
pub const SLICE_MOST: u64 = 65536;

/// The most frames one `grit_world_step` runs.
pub const STEP_MOST: u32 = 65536;

pub const POOL_LEAST_CELLS: u64 = 1 << 21;

pub const PAGE_BYTES: usize = (512 - 4) << 10;

pub const IGNORED_SHOWN: usize = 64;

/// The digits a slice of kinds is drawn with, one a kind.
const DIGITS: &[u8; 16] = b"0123456789abcdef";

/// A rule the world tools run by name. [`Named`] makes one of any
/// [`Rule`].
pub trait Listed {
    fn name(&self) -> &'static str;

    /// How many words of parameters the rule reads.
    fn params(&self) -> usize;

    /// The words of the tally a frame under the rule leaves, as
    /// [`crate::rule3::Tally::words`] gives them.
    fn tally_words(&self) -> usize;

    /// The listed frame under the rule, as [`live::wired3_by`] builds it.
    fn wired(&self, subgroup: u32, grid: Grid, plan: &Plan) -> Result<Loop, LaneError>;

    /// The bindings' sizes, as [`live::sizes3`] gives them.
    fn sizes(&self, plan: &Plan) -> [usize; BINDINGS as usize];

    fn bricked(&self, bricks: &Bricks, plan: &Plan) -> Result<Loop, LaneError>;

    fn bricked_sizes(&self, bricks: &Bricks, plan: &Plan) -> Vec<usize>;

    /// One frame on the CPU, as [`crate::sand3::reference`] steps it.
    fn reference(&self, grid: Grid3, frame: u32, cells: &[u32], params: &[u32]) -> Vec<u32>;

    /// The kind of a cell word, from 0 to 15; a slice shows a kind past 15
    /// as `+`.
    fn kind(&self, word: u32) -> u32;
}

/// A [`Rule`] under a name, with the kind of its words.
pub struct Named<R> {
    pub name: &'static str,
    pub rule: R,
    pub kind: fn(u32) -> u32,
}

impl<R: Rule> Listed for Named<R> {
    fn name(&self) -> &'static str {
        self.name
    }

    fn params(&self) -> usize {
        self.rule.params()
    }

    fn tally_words(&self) -> usize {
        self.rule.tally().words()
    }

    fn wired(&self, subgroup: u32, grid: Grid, plan: &Plan) -> Result<Loop, LaneError> {
        live::wired3_by(&self.rule, subgroup, grid, plan)
    }

    fn sizes(&self, plan: &Plan) -> [usize; BINDINGS as usize] {
        live::sizes3(&self.rule, plan)
    }

    fn bricked(&self, bricks: &Bricks, plan: &Plan) -> Result<Loop, LaneError> {
        crate::bricks::arranged(&self.rule, bricks, plan, true)
    }

    fn bricked_sizes(&self, bricks: &Bricks, plan: &Plan) -> Vec<usize> {
        bricks.sizes(&self.rule, plan)
    }

    fn reference(&self, grid: Grid3, frame: u32, cells: &[u32], params: &[u32]) -> Vec<u32> {
        crate::sand3::reference(&self.rule, grid, frame, cells, params)
    }

    fn kind(&self, word: u32) -> u32 {
        (self.kind)(word)
    }
}

/// `rule` under `name`, with the kind of its words, as [`rules`] lists it.
#[must_use]
pub fn named<R: Rule + 'static>(
    name: &'static str,
    rule: R,
    kind: fn(u32) -> u32,
) -> Box<dyn Listed> {
    Box::new(Named { name, rule, kind })
}

/// Every rule the world tools know, in the order they are listed. A new
/// rule is one more line here.
#[must_use]
pub fn rules() -> Vec<Box<dyn Listed>> {
    rules_on(None)
}

#[must_use]
pub fn rules_on(host: Option<&Host>) -> Vec<Box<dyn Listed>> {
    let embedding = host.map_or_else(Embed::default, Embed::on);
    vec![
        named("sand", Sand, sand::material_of),
        named("heat", Heat, heat::kind_of),
        named("code", Code, code::instruction_of),
        named("nature", Nature, nature::kind_of),
        named("evo", Evo, evo::kind_of),
        named("embed", embedding, embed::kind_of),
    ]
}

/// The name of any refusal a world tool gives: a [`Refusal`], or the
/// engine's own error for a world the device cannot hold.
#[must_use]
pub fn refusal_name(why: &(dyn std::error::Error + 'static)) -> &'static str {
    refusal_of(why).code.name()
}

pub type Failed = Box<dyn std::error::Error>;

fn pooled(used: PoolUse) -> Json {
    map(vec![
        ("held", counted(used.held)),
        ("free", counted(used.free)),
        ("capacity", counted(used.capacity)),
        ("overflow", Json::Bool(used.overflow)),
        ("lost", Json::Bool(used.lost)),
    ])
}

#[derive(Debug, Clone, Default)]
pub struct Options {
    pub dir: Option<PathBuf>,
    pub memory: Option<u64>,
}

pub struct Live<'host> {
    world: World<'host>,
    rule: Box<dyn Listed>,
}

#[derive(Debug, Clone)]
pub struct Held {
    ledger: Ledger,
    bytes: u64,
    pace: Pace,
    note: Option<Json>,
    broken: Option<String>,
    snap_failure: Option<SnapFailure>,
    epoch: u64,
}

#[derive(Debug)]
struct Writing {
    number: u32,
    epoch: u64,
    from: usize,
    frame: u32,
    made: Made,
    bytes: u64,
    began: Instant,
    done: std::sync::mpsc::Receiver<Result<[u8; 32], String>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

#[derive(Debug, Clone, Copy)]
enum Wait {
    Poll,
    For(Duration),
    Landed,
}

const WRITER_STOPPED: &str = "the thread writing the snapshot stopped before it finished";

#[derive(Debug, Clone)]
pub struct SnapFailure {
    message: String,
    failures: u32,
    next: Instant,
}

pub const SNAP_RETRY_LEAST: Duration = Duration::from_secs(1);

pub const SNAP_RETRY_MOST: Duration = Duration::from_secs(300);

#[derive(Debug)]
pub struct Book {
    next: u32,
    held: BTreeMap<u32, Held>,
    earlier: BTreeMap<u32, Ledger>,
    keep: Option<Keep>,
    memory: Memory,
    memory_fixed: bool,
    lost: bool,
    restorations: u64,
    rates: budget::Rates,
    broken_earlier: BTreeMap<u32, String>,
    stats: Stats,
    ignored: Vec<String>,
    epochs: u64,
    writing: Option<Writing>,
    #[cfg(feature = "faults")]
    faults: Arc<std::sync::Mutex<Faults>>,
}

#[derive(Debug, Clone, Default)]
pub struct Stats {
    pub batches: Arc<std::sync::atomic::AtomicU64>,
    pub longest: Duration,
    pub longest_frames: u32,
}

#[cfg(feature = "faults")]
#[derive(Debug, Clone, Default)]
pub struct Faults {
    pub wedge_after_batch: Option<u32>,
    pub wedge_before_call: bool,
    pub fail_snapshots: u32,
    pub wedge_in_snapshot: bool,
    pub slow_idle_snapshot: Duration,
    pub slowed: u32,
}

impl Book {
    pub fn new(options: &Options) -> std::io::Result<Self> {
        let keep = options.dir.as_deref().map(Keep::locked).transpose()?;
        let (earlier, mut ignored) = keep.as_ref().map(Keep::scan).unwrap_or_default();
        let (kept_next, crafted) = keep.as_ref().map_or((None, None), Keep::next_checked);
        ignored.extend(crafted);
        let sums: Vec<[u8; 32]> = earlier.iter().filter_map(|(_, one)| one.sum()).collect();
        let mut found = BTreeMap::new();
        for (number, ledger) in earlier {
            match kept_next {
                Some(next) if number >= next => ignored.push(format!(
                    "kept/world-{number}.journal: world {number} is past the next number {next} this directory has given, so it was ignored"
                )),
                _ => {
                    found.insert(number, ledger);
                }
            }
        }
        let earlier: BTreeMap<u32, Ledger> = found;
        let mut broken_earlier = BTreeMap::new();
        for (number, ledger) in &earlier {
            if let Some(why) = &ledger.unreadable {
                broken_earlier.insert(*number, why.clone());
            }
        }
        if let Some(keep) = keep.as_ref() {
            for (number, ledger) in &earlier {
                if let Some(sum) = ledger.sum()
                    && journal::tail_sum(&keep.snapshot(&sum)).ok() != Some(sum)
                {
                    broken_earlier.insert(
                        *number,
                        format!(
                            "its snapshot snap-{} is gone or changed",
                            journal::hex(&sum)
                        ),
                    );
                }
            }
        }
        if let Some(keep) = keep.as_ref() {
            keep.sweep(&sums);
        }
        let next = kept_next.unwrap_or_else(|| {
            earlier
                .keys()
                .max()
                .map_or(1, |most| most.saturating_add(1))
        });
        Ok(Self {
            next,
            held: BTreeMap::new(),
            earlier,
            keep,
            memory: Memory::new(options.memory.unwrap_or(0)),
            memory_fixed: options.memory.is_some(),
            lost: false,
            restorations: 0,
            rates: budget::Rates::default(),
            broken_earlier,
            stats: Stats::default(),
            ignored,
            epochs: 0,
            writing: None,
            #[cfg(feature = "faults")]
            faults: Arc::default(),
        })
    }

    #[must_use]
    pub fn empty() -> Self {
        Self {
            next: 1,
            held: BTreeMap::new(),
            earlier: BTreeMap::new(),
            keep: None,
            memory: Memory::new(0),
            memory_fixed: false,
            lost: false,
            restorations: 0,
            rates: budget::Rates::default(),
            broken_earlier: BTreeMap::new(),
            stats: Stats::default(),
            ignored: Vec::new(),
            epochs: 0,
            writing: None,
            #[cfg(feature = "faults")]
            faults: Arc::default(),
        }
    }

    #[cfg(feature = "faults")]
    #[must_use]
    pub fn faults(&self) -> Arc<std::sync::Mutex<Faults>> {
        Arc::clone(&self.faults)
    }

    fn referenced(&self, sum: &[u8; 32]) -> bool {
        self.held
            .values()
            .map(|held| &held.ledger)
            .chain(self.earlier.values())
            .any(|ledger| ledger.sum().as_ref() == Some(sum))
    }

    fn release(&self, sum: Option<[u8; 32]>) {
        if let (Some(sum), Some(keep)) = (sum, self.keep.as_ref())
            && !self.referenced(&sum)
        {
            keep.drop_snapshot(&sum);
        }
    }

    fn bump(&mut self) -> u64 {
        self.epochs = self.epochs.wrapping_add(1);
        self.epochs
    }

    fn snap_failed(&mut self, number: u32, message: String) {
        let now = Instant::now();
        if let Some(held) = self.held.get_mut(&number) {
            let failures = held
                .snap_failure
                .as_ref()
                .map_or(1, |old| old.failures.saturating_add(1));
            let wait = SNAP_RETRY_LEAST
                .saturating_mul(1_u32 << failures.saturating_sub(1).min(16))
                .min(SNAP_RETRY_MOST);
            held.snap_failure = Some(SnapFailure {
                message,
                failures,
                next: now + wait,
            });
        }
    }

    fn writing_for(&self, number: u32) -> bool {
        self.writing
            .as_ref()
            .is_some_and(|writing| writing.number == number)
    }

    pub fn drain(&mut self) {
        self.landed(Wait::Landed);
    }

    fn landed(&mut self, wait: Wait) -> bool {
        let Some(writing) = self.writing.as_ref() else {
            return false;
        };
        let got = match wait {
            Wait::Poll => match writing.done.try_recv() {
                Ok(got) => got,
                Err(std::sync::mpsc::TryRecvError::Empty) => return false,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Err(WRITER_STOPPED.to_owned()),
            },
            Wait::For(left) => match writing.done.recv_timeout(left) {
                Ok(got) => got,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => return false,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    Err(WRITER_STOPPED.to_owned())
                }
            },
            Wait::Landed => writing
                .done
                .recv()
                .unwrap_or_else(|_| Err(WRITER_STOPPED.to_owned())),
        };
        let Some(mut writing) = self.writing.take() else {
            return false;
        };
        if let Some(thread) = writing.thread.take() {
            let _ = thread.join();
        }
        let cost = writing.began.elapsed();
        let number = writing.number;
        let current = self
            .held
            .get(&number)
            .is_some_and(|held| held.epoch == writing.epoch);
        let sum = match got {
            Ok(sum) => sum,
            Err(message) => {
                if current {
                    self.snap_failed(number, message);
                }
                return true;
            }
        };
        self.rates.measure(budget::Work::Save, writing.bytes, cost);
        if !current {
            self.release(Some(sum));
            return true;
        }
        let epoch = self.bump();
        let snap = Arc::new(journal::Snap {
            sum,
            frame: writing.frame,
            made: writing.made,
            image: None,
        });
        let keep = self.keep.clone();
        let Some(held) = self.held.get_mut(&number) else {
            return true;
        };
        let old = held.ledger.sum();
        let mut ledger = held.ledger.clone();
        ledger.rebase_at(snap, cost, writing.from);
        match keep.as_ref().map(|keep| keep.begin(number, &ledger)) {
            Some(Err(why)) => {
                self.release(Some(sum));
                self.snap_failed(number, format!("the journal could not be written: {why}"));
            }
            _ => {
                held.ledger = ledger;
                held.epoch = epoch;
                held.snap_failure = None;
                self.release(old);
            }
        }
        true
    }

    #[must_use]
    pub const fn lost(&self) -> bool {
        self.lost
    }

    #[must_use]
    pub fn batches(&self) -> Arc<std::sync::atomic::AtomicU64> {
        Arc::clone(&self.stats.batches)
    }
}

#[derive(Debug, Clone)]
pub struct Status {
    pub device: Json,
    pub memory: Json,
    pub worlds: Json,
    pub submissions: Json,
    pub directory: Json,
}

pub struct Tool {
    pub name: &'static str,
    pub shown: fn() -> Json,
    pub run: fn(&mut Server<'_>, &Json) -> Result<Json, Failed>,
}

/// The MCP server with its open worlds. Every 2D tool answers as
/// [`crate::serve::handle`] does; `tools/list` names the 2D tools and then
/// the world tools.
pub struct Server<'host> {
    host: Option<&'host Host>,
    book: Book,
    live: BTreeMap<u32, Live<'host>>,
    clock: Clock,
    arrived: Option<Instant>,
    cancel: Option<Arc<AtomicBool>>,
}

/// The names of the world tools, in the order `tools/list` gives them.
pub const WORLD_TOOLS: [&str; 13] = [
    "grit_world_rules",
    "grit_world_create",
    "grit_world_write",
    "grit_world_step",
    "grit_world_read",
    "grit_world_slice",
    "grit_world_close",
    "grit_describe",
    "grit_world_save",
    "grit_world_open",
    "grit_world_list",
    "grit_world_fork",
    "grit_world_restore",
];

fn served() -> impl Iterator<Item = &'static Tool> {
    tools::TOOLS
        .iter()
        .chain(law::TOOLS)
        .chain(ask::TOOLS)
        .chain(branch::TOOLS)
        .chain(learn::TOOLS)
}

fn shut(tool: Json) -> Json {
    match tool {
        Json::Map(held) => Json::Map(
            held.into_iter()
                .map(|(key, one)| {
                    if key == "inputSchema" {
                        (key, schema::closed(one))
                    } else {
                        (key, one)
                    }
                })
                .collect(),
        ),
        other => other,
    }
}

fn flat() -> Vec<Json> {
    match crate::serve::tools() {
        Json::List(held) => held.into_iter().map(shut).collect(),
        other => vec![shut(other)],
    }
}

#[must_use]
pub fn listing_all() -> Vec<Json> {
    let mut listed = flat();
    listed.extend(served().map(|tool| (tool.shown)()));
    listed
}

fn input(shown: &Json) -> Json {
    shown
        .get("inputSchema")
        .cloned()
        .unwrap_or_else(|| schema::object(Vec::new(), &[]))
}

fn delivery(result: Json) -> Json {
    map(vec![
        (
            "content",
            Json::List(vec![map(vec![
                ("type", text("text")),
                ("text", text(&crate::wire::written(&result))),
            ])]),
        ),
        ("isError", Json::Bool(false)),
        ("structuredContent", result),
    ])
}

fn known_rules() -> Json {
    Json::List(
        rules()
            .iter()
            .map(|one| {
                map(vec![
                    ("name", text(one.name())),
                    (
                        "params",
                        counted(u32::try_from(one.params()).unwrap_or(u32::MAX)),
                    ),
                ])
            })
            .collect(),
    )
}

fn listing() -> String {
    rules()
        .iter()
        .map(|one| format!("{} ({} parameters)", one.name(), one.params()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn world_refusal(
    mut refusal: Refusal,
    arguments: &Json,
    mut more: Vec<(&'static str, Json)>,
) -> Json {
    let code = refusal.code;
    if code == errors::Code::UnknownRule {
        let asked = match arguments.get("rule") {
            None => "\"\"".to_owned(),
            Some(Json::Text(asked)) => errors::shown(&format!("{asked:?}")),
            Some(other) => errors::shown_value(other),
        };
        refusal.message = format!("rule {asked} is not one of {}", listing());
    }
    if matches!(code, errors::Code::UnknownRule | errors::Code::Params) {
        more.push(("rules", known_rules()));
    }
    transport::refused(&refusal, more)
}

pub struct Flat<'host> {
    pub host: Option<&'host Host>,
}

fn flat_called(host: Option<&Host>, listed: &[Json], name: &str, arguments: &Json) -> Option<Json> {
    let shown = listed
        .iter()
        .find(|one| one.get("name").and_then(Json::text) == Some(name))?;
    if let Err(refusal) = schema::checked(name, &input(shown), arguments, Check::Keys) {
        return Some(transport::refused(&refusal, Vec::new()));
    }
    Some(match crate::serve::ran(host, name, arguments) {
        Ok(result) => delivery(result),
        Err(why) => transport::refused(&refusal_of(why.as_ref()), Vec::new()),
    })
}

impl transport::Served for Flat<'_> {
    fn listed(&self) -> Vec<Json> {
        flat()
    }

    fn called(&mut self, name: &str, arguments: &Json) -> Option<Json> {
        flat_called(self.host, &flat(), name, arguments)
    }
}

fn lost_device(why: &(dyn std::error::Error + 'static)) -> bool {
    why.downcast_ref::<tools::Lost>().is_some()
        || matches!(
            why.downcast_ref::<crate::host::Error>(),
            Some(
                crate::host::Error::Wedged { .. }
                    | crate::host::Error::Vulkan(ash::vk::Result::ERROR_DEVICE_LOST)
            )
        )
}

fn branch_refusal(why: &crate::branch::Refused) -> Refusal {
    use crate::branch::Refused;
    let code = match why {
        Refused::Unfit { .. } => errors::Code::Unfit,
        Refused::Rule { .. } => errors::Code::UnknownRule,
        Refused::Params { .. } => errors::Code::Params,
        Refused::Unlike { .. } => errors::Code::Unlike,
        Refused::Outside { .. } => errors::Code::Outside,
        Refused::Io { .. } => errors::Code::Io,
        Refused::Magic
        | Refused::Version { .. }
        | Refused::Checksum
        | Refused::Truncated { .. }
        | Refused::Sizes { .. }
        | Refused::LawHash
        | Refused::Section { .. }
        | Refused::Trailing { .. }
        | Refused::Malformed { .. } => errors::Code::BadFile,
    };
    Refusal::new(code, "$", format!("{why} ({})", why.name()))
}

fn refused_with(why: &(dyn std::error::Error + 'static)) -> (Refusal, Vec<(&'static str, Json)>) {
    if let Some(short) = why.downcast_ref::<Short>() {
        return (short.refusal(), short.told());
    }
    if let Some(halted) = why.downcast_ref::<budget::Halted>() {
        return (halted.refusal(), Vec::new());
    }
    if let Some(tools::Lost(refusal)) = why.downcast_ref::<tools::Lost>() {
        return (refusal.clone(), Vec::new());
    }
    if let Some(held) = why.downcast_ref::<crate::branch::Refused>() {
        return (branch_refusal(held), Vec::new());
    }
    if lost_device(why) {
        return (
            Refusal::new(
                errors::Code::DeviceLost,
                "$",
                format!(
                    "the device was lost during this call ({why}); every open world is kept by its journal"
                ),
            ),
            Vec::new(),
        );
    }
    (refusal_of(why), Vec::new())
}

fn number_of(held: &Json, key: &str) -> Option<u32> {
    held.get(key)
        .and_then(Json::int)
        .and_then(|one| u32::try_from(one).ok())
}

impl<'host> Server<'host> {
    #[must_use]
    pub fn new(host: Option<&'host Host>) -> Self {
        Self::resumed(host, Book::empty()).0
    }

    pub fn with(host: Option<&'host Host>, options: &Options) -> std::io::Result<Self> {
        Ok(Self::resumed(host, Book::new(options)?).0)
    }

    #[must_use]
    pub fn resumed(host: Option<&'host Host>, mut book: Book) -> (Self, Vec<Json>) {
        if !book.memory_fixed {
            book.memory
                .set_allowed(host.map_or(0, |host| Memory::share_of(host.heap())));
        }
        let was_lost = book.lost;
        book.lost = false;
        let mut server = Self {
            host,
            book,
            live: BTreeMap::new(),
            clock: Clock::of(Instant::now(), budget::BUDGET_MS),
            arrived: None,
            cancel: None,
        };
        let mut restored = Vec::new();
        if !was_lost {
            return (server, restored);
        }
        restored = server.pending();
        if server.wedged() {
            server.lose();
        }
        (server, restored)
    }

    fn replay_estimate(&self, ledger: &Ledger, bytes: u64) -> Duration {
        let rates = &self.book.rates;
        let snapshot = match (&ledger.base, self.book.keep.as_ref()) {
            (Base::Snapshot(snap), Some(keep)) if snap.image.is_none() => {
                std::fs::metadata(keep.snapshot(&snap.sum)).map_or(Duration::ZERO, |found| {
                    rates.estimate(budget::Work::File, found.len())
                })
            }
            _ => Duration::ZERO,
        };
        rates
            .estimate(budget::Work::Build, bytes)
            .saturating_add(snapshot)
            .saturating_add(rates.estimate(budget::Work::Write, ledger.write_bytes()))
            .saturating_add(rates.replaying(ledger.step_recorded()))
    }

    fn fits(&self, needs: Duration, what: &str) -> Result<(), Failed> {
        let left = self.clock.left();
        if needs > left {
            return Err(Box::new(budget::too_slow(needs, left, what)));
        }
        Ok(())
    }

    pub(super) fn replayed_world(
        &mut self,
        host: &'host Host,
        ledger: &Ledger,
        pace: &mut Pace,
        bytes: u64,
    ) -> Result<World<'host>, Failed> {
        let guard = self.guard();
        let mut timings = journal::Timings::default();
        let built = journal::rebuilt(
            host,
            ledger,
            self.book.keep.as_ref(),
            &guard,
            &mut timings,
            |world, frames| paced(world, frames, pace, &guard),
        );
        let rates = &mut self.book.rates;
        if !timings.build.is_zero() {
            rates.measure(budget::Work::Build, bytes, timings.build);
        }
        if timings.file_bytes > 0 {
            rates.measure(budget::Work::File, timings.file_bytes, timings.file);
        }
        if timings.write_bytes > 0 {
            rates.measure(budget::Work::Write, timings.write_bytes, timings.writes);
        }
        if built.is_ok() {
            rates.measure_replay(timings.recorded, timings.steps);
        }
        built
    }

    fn rebuild(&mut self, number: u32) -> Result<(), Failed> {
        let Some(held) = self.book.held.get(&number) else {
            return Ok(());
        };
        let ledger = held.ledger.clone();
        let bytes = held.bytes;
        let mut pace = held.pace;
        let from_frame = match &ledger.base {
            Base::Created(_) => 0,
            Base::Snapshot(snap) => snap.frame,
        };
        let replayed = u32::try_from(ledger.entries.len()).unwrap_or(u32::MAX);
        let host = self.host.ok_or(crate::host::Error::NoComputeDevice)?;
        self.fits(
            self.replay_estimate(&ledger, bytes),
            &format!("rebuilding world {number} after the lost device"),
        )?;
        let built = self
            .replayed_world(host, &ledger, &mut pace, bytes)
            .and_then(|world| {
                let rule = rule_of(host, world.rule())?;
                Ok(Live { world, rule })
            });
        let mut note = vec![
            ("world", counted(number)),
            ("from_frame", counted(from_frame)),
            ("replayed", counted(replayed)),
            ("frame", counted(ledger.frame)),
        ];
        let mut broken = None;
        match built {
            Ok(live) => {
                self.live.insert(number, live);
            }
            Err(why) => {
                if why.downcast_ref::<budget::Halted>().is_some() {
                    return Err(why);
                }
                let said = refused_with(why.as_ref()).0.message;
                if self.wedged() || lost_device(why.as_ref()) {
                    self.lose();
                    return Err(why);
                }
                broken = Some(said.clone());
                note.push(("failed", text(&said)));
            }
        }
        if let Some(held) = self.book.held.get_mut(&number) {
            held.broken = broken;
            held.pace = pace;
            held.note = Some(map(note));
        }
        Ok(())
    }

    #[must_use]
    pub fn into_book(self) -> Book {
        self.book
    }

    #[must_use]
    pub const fn lost(&self) -> bool {
        self.book.lost
    }

    #[cfg(feature = "faults")]
    #[must_use]
    pub fn faults(&self) -> Arc<std::sync::Mutex<Faults>> {
        self.book.faults()
    }

    /// How many worlds are open.
    #[must_use]
    pub fn open(&self) -> usize {
        self.live.len()
    }

    pub fn arriving(&mut self, arrived: Option<Instant>, cancel: Option<Arc<AtomicBool>>) {
        self.arrived = arrived;
        self.cancel = cancel;
    }

    fn cancelled(&self) -> bool {
        self.cancel
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::SeqCst))
    }

    fn guard(&self) -> budget::Guard {
        budget::Guard {
            deadline: Some(self.clock.deadline),
            cancel: self.cancel.clone(),
        }
    }

    fn stop(&self) -> Option<Stop> {
        if self.cancelled() {
            return Some(Stop::Cancelled);
        }
        self.clock.left().is_zero().then_some(Stop::Budget)
    }

    fn lose(&mut self) {
        if !self.book.lost {
            self.book.restorations = self.book.restorations.saturating_add(1);
        }
        self.live.clear();
        self.book.lost = true;
    }

    #[must_use]
    pub fn pending(&self) -> Vec<Json> {
        self.book
            .held
            .iter()
            .filter(|(number, held)| held.broken.is_none() && !self.live.contains_key(number))
            .map(|(number, held)| {
                map(vec![
                    ("world", counted(*number)),
                    ("frame", counted(held.ledger.frame)),
                    ("pending", Json::Bool(true)),
                ])
            })
            .collect()
    }

    fn wedged(&self) -> bool {
        self.host.is_some_and(Host::wedged)
    }

    fn world(&mut self, held: &Json) -> Result<&mut Live<'host>, Failed> {
        let number = tools::int(held, "world")?;
        self.world_at(number, "$.world")
    }

    fn world_at(&mut self, number: u32, field: &str) -> Result<&mut Live<'host>, Failed> {
        let pending = !self.live.contains_key(&number)
            && self
                .book
                .held
                .get(&number)
                .is_some_and(|held| held.broken.is_none());
        if pending && !self.book.lost && !self.wedged() {
            self.rebuild(number)?;
        }
        if self.live.contains_key(&number) {
            return self
                .live
                .get_mut(&number)
                .ok_or_else(|| "the world went away".into());
        }
        let broken = self
            .book
            .held
            .get(&number)
            .and_then(|held| held.broken.clone());
        let refusal = if let Some(why) = broken {
            Refusal::new(
                errors::Code::DeviceLost,
                field,
                format!(
                    "world {number} could not be rebuilt after the device was lost ({}); close it",
                    errors::shown(&why)
                ),
            )
        } else if self.book.held.contains_key(&number) {
            Refusal::new(
                errors::Code::DeviceLost,
                field,
                format!(
                    "world {number} was on a device that was lost, and this server has not rebuilt it"
                ),
            )
        } else if self.book.earlier.contains_key(&number) {
            Refusal::new(
                errors::Code::NoWorld,
                field,
                format!(
                    "world {number} is kept from an earlier run of the server; open it with grit_world_open first"
                ),
            )
        } else if number < self.book.next {
            Refusal::new(
                errors::Code::Closed,
                field,
                format!("world {number} was closed"),
            )
        } else {
            Refusal::new(
                errors::Code::NoWorld,
                field,
                format!("no world {number} was ever opened"),
            )
        };
        Err(Box::new(refusal))
    }

    fn number(&mut self) -> Result<u32, Failed> {
        let id = self.book.next;
        let next = id
            .checked_add(1)
            .filter(|_| id < journal::NUMBER_MOST)
            .ok_or_else(|| {
                Box::new(
                    Refusal::new(
                        errors::Code::TooMany,
                        "$",
                        format!(
                            "every world number up to {} has been used",
                            journal::NUMBER_MOST - 1
                        ),
                    )
                    .limited(counted(journal::NUMBER_MOST - 1)),
                ) as Failed
            })?;
        if let Some(keep) = self.book.keep.as_ref() {
            keep.set_next(next).map_err(|why| io_refusal(&why))?;
        }
        self.book.next = next;
        Ok(id)
    }

    fn adopt(
        &mut self,
        number: u32,
        live: Live<'host>,
        ledger: Ledger,
        bytes: u64,
        pace: Pace,
    ) -> Option<Json> {
        let failed = self
            .book
            .keep
            .as_ref()
            .and_then(|keep| keep.begin(number, &ledger).err());
        self.book.memory.hold(number, bytes);
        let epoch = self.book.bump();
        self.book.held.insert(
            number,
            Held {
                ledger,
                bytes,
                pace,
                note: None,
                broken: None,
                snap_failure: None,
                epoch,
            },
        );
        self.live.insert(number, live);
        failed.map(|why| text(&format!("the journal could not be written: {why}")))
    }

    fn journaled(&mut self, number: u32, entry: Entry, spent: Duration) -> Option<Json> {
        let keep = self.book.keep.clone();
        let failed = keep
            .as_ref()
            .and_then(|keep| keep.append(number, &entry, spent).err());
        let held = self.book.held.get_mut(&number)?;
        held.ledger.push(entry, spent);
        failed.map(|why| text(&format!("the journal could not be written: {why}")))
    }

    fn settle(&mut self, number: u32) -> Option<Json> {
        let held = self.book.held.get(&number)?;
        if !held.ledger.due(held.bytes, self.book.rates.replay())
            || self.clock.left() < self.snap_estimate(number)
        {
            return None;
        }
        match self.snap_now(number) {
            Ok(note) => note,
            Err(why) => Some(text(&format!(
                "the snapshot could not be taken: {}",
                refused_with(why.as_ref()).0.message
            ))),
        }
    }

    fn owed(&mut self, number: u32) -> Result<Option<Json>, Failed> {
        let Some(held) = self.book.held.get(&number) else {
            return Ok(None);
        };
        let ratio = self.book.rates.replay();
        if !held.ledger.due(held.bytes, ratio) {
            return Ok(None);
        }
        let far = held.ledger.far_over(Duration::ZERO, held.bytes, ratio);
        if self.book.writing_for(number) {
            if !far {
                return Ok(None);
            }
            self.book.landed(Wait::For(self.clock.left()));
            if self.book.writing_for(number) {
                return Err(Box::new(Refusal::new(
                    errors::Code::Budget,
                    "$.budget_ms",
                    format!(
                        "world {number} owes a snapshot of its journal before it changes again, and the one being written did not land within this call's budget; nothing ran"
                    ),
                )));
            }
            return self.owed(number);
        }
        let Some(held) = self.book.held.get(&number) else {
            return Ok(None);
        };
        if let Some(failure) = held.snap_failure.as_ref()
            && failure.next > Instant::now()
        {
            if !far {
                return Ok(None);
            }
            return Err(Box::new(Refusal::new(
                errors::Code::Io,
                "$.world",
                format!(
                    "world {number} owes a snapshot of its journal before it changes again, and its last {} attempts failed ({}); the next is due in {} ms; nothing ran",
                    failure.failures,
                    errors::shown(&failure.message),
                    failure
                        .next
                        .saturating_duration_since(Instant::now())
                        .as_millis()
                ),
            )));
        }
        let needs = self.snap_estimate(number);
        if self.clock.left() < needs && !far {
            return Ok(None);
        }
        if self.clock.left() < needs {
            return Err(Box::new(
                Refusal::new(
                    errors::Code::Budget,
                    "$.budget_ms",
                    format!(
                        "world {number} owes a snapshot of its journal before it changes again, which needs about {} ms, and this call has {} ms left; nothing ran",
                        needs.as_millis(),
                        self.clock.left().as_millis()
                    ),
                )
                .limited(budget::counted64(
                    u64::try_from(needs.as_millis().saturating_add(1)).unwrap_or(u64::MAX),
                )),
            ));
        }
        self.snap_now(number)
    }

    fn snap_now(&mut self, number: u32) -> Result<Option<Json>, Failed> {
        if self.book.writing_for(number) {
            self.book.landed(Wait::Poll);
            if self.book.writing_for(number) || !self.due(number) {
                return Ok(None);
            }
        }
        let taken = self.snap_attempt(number);
        match &taken {
            Ok(_) => {
                if let Some(held) = self.book.held.get_mut(&number) {
                    held.snap_failure = None;
                }
            }
            Err(why) => {
                let message = refused_with(why.as_ref()).0.message;
                self.book.snap_failed(number, message);
            }
        }
        taken
    }

    fn due(&self, number: u32) -> bool {
        self.book
            .held
            .get(&number)
            .is_some_and(|held| held.ledger.due(held.bytes, self.book.rates.replay()))
    }

    fn snap_image(&mut self, number: u32) -> Result<(crate::branch::Image, Made, Instant), Failed> {
        #[cfg(feature = "faults")]
        {
            let (fail, wedge) = self
                .book
                .faults
                .lock()
                .map(|mut faults| {
                    let fail = faults.fail_snapshots > 0;
                    if fail {
                        faults.fail_snapshots -= 1;
                    }
                    (fail, std::mem::take(&mut faults.wedge_in_snapshot))
                })
                .unwrap_or((false, false));
            if wedge && let Some(host) = self.host {
                host.wedge();
            }
            if fail {
                return Err(Box::new(Refusal::new(
                    errors::Code::Io,
                    "$",
                    "an armed fault failed this snapshot".to_owned(),
                )));
            }
        }
        let made = self
            .book
            .held
            .get(&number)
            .ok_or("the world has no journal")?
            .ledger
            .made()
            .clone();
        let live = self.live.get_mut(&number).ok_or("the world is not open")?;
        let began = Instant::now();
        let image = live.world.image()?;
        let made = Made {
            slots: match &image.state {
                crate::branch::State::Sparse { capacity, .. } => Some(*capacity),
                crate::branch::State::Dense(_) => made.slots,
            },
            ..made
        };
        Ok((image, made, began))
    }

    fn snap_attempt(&mut self, number: u32) -> Result<Option<Json>, Failed> {
        let (image, made, began) = self.snap_image(number)?;
        let frame = image.frame;
        let keep = self.book.keep.clone();
        let snap = match keep.as_ref() {
            Some(keep) => journal::Snap {
                sum: stored(keep, number, &image)?,
                frame,
                made,
                image: None,
            },
            None => journal::Snap {
                sum: journal::state_of(&image),
                frame,
                made,
                image: Some(image),
            },
        };
        let sum = snap.sum;
        let cost = began.elapsed();
        let size = self.book.held.get(&number).map_or(0, |held| held.bytes);
        self.book.rates.measure(budget::Work::Save, size, cost);
        let epoch = self.book.bump();
        let held = self
            .book
            .held
            .get_mut(&number)
            .ok_or("the world has no journal")?;
        let old = held.ledger.sum();
        let mut ledger = held.ledger.clone();
        ledger.rebase(Arc::new(snap), cost);
        if let Some(Err(why)) = keep.as_ref().map(|keep| keep.begin(number, &ledger)) {
            self.book.release(Some(sum));
            return Ok(Some(text(&format!(
                "the journal could not be written: {why}"
            ))));
        }
        held.ledger = ledger;
        held.epoch = epoch;
        self.book.release(old);
        Ok(None)
    }

    fn snap_behind(&mut self, number: u32, keep: Keep) -> Result<(), Failed> {
        let started = self.snap_image(number).and_then(|(image, made, began)| {
            let held = self
                .book
                .held
                .get(&number)
                .ok_or("the world has no journal")?;
            let (epoch, from, bytes) = (held.epoch, held.ledger.entries.len(), held.bytes);
            let frame = image.frame;
            let slow = self.slow_idle();
            let (sender, done) = std::sync::mpsc::channel();
            let thread = std::thread::Builder::new()
                .name("grit-snapshot".to_owned())
                .spawn(move || {
                    std::thread::sleep(slow);
                    let got = stored(&keep, number, &image)
                        .map_err(|why| refused_with(why.as_ref()).0.message);
                    let _ = sender.send(got);
                })
                .map_err(|why| io_refusal(&why))?;
            self.book.writing = Some(Writing {
                number,
                epoch,
                from,
                frame,
                made,
                bytes,
                began,
                done,
                thread: Some(thread),
            });
            Ok(())
        });
        if let Err(why) = &started {
            let message = refused_with(why.as_ref()).0.message;
            self.book.snap_failed(number, message);
        }
        started
    }

    fn overdue(&self, number: u32, extra: Duration) -> bool {
        self.book.held.get(&number).is_some_and(|held| {
            held.ledger
                .over(extra, held.bytes, self.book.rates.replay())
        })
    }

    fn snapping(&self, number: u32) -> Duration {
        self.snap_estimate(number)
    }

    fn snap_estimate(&self, number: u32) -> Duration {
        self.book.held.get(&number).map_or(Duration::ZERO, |held| {
            self.book.rates.estimate(budget::Work::Save, held.bytes)
        })
    }

    fn far_overdue(&self, number: u32, extra: Duration) -> bool {
        self.book.held.get(&number).is_some_and(|held| {
            held.ledger
                .far_over(extra, held.bytes, self.book.rates.replay())
        })
    }

    pub fn idle(&mut self) -> bool {
        if self.book.writing.is_some() {
            return self.book.landed(Wait::Poll);
        }
        let ratio = self.book.rates.replay();
        let now = Instant::now();
        let behind = self.book.keep.is_some();
        let owed = self
            .book
            .held
            .iter()
            .find(|(number, held)| {
                let quick = behind
                    || self.book.rates.estimate(budget::Work::Save, held.bytes)
                        <= Duration::from_millis(budget::SUBMIT_MOST_MS);
                self.live.contains_key(number)
                    && held.ledger.due(held.bytes, ratio)
                    && (quick || held.ledger.far_over(Duration::ZERO, held.bytes, ratio))
                    && held
                        .snap_failure
                        .as_ref()
                        .is_none_or(|failure| failure.next <= now)
            })
            .map(|(number, _)| *number);
        let Some(number) = owed else {
            return false;
        };
        let wedged_before = self.wedged();
        self.clock = Clock::of(Instant::now(), budget::BUDGET_MOST_MS);
        let taken = match self.book.keep.clone() {
            Some(keep) => self.snap_behind(number, keep),
            None => self.snap_now(number).map(|_| ()),
        };
        let lost = (!wedged_before && self.wedged())
            || taken
                .as_ref()
                .err()
                .is_some_and(|why| lost_device(why.as_ref()));
        if lost {
            self.lose();
        }
        true
    }

    #[cfg(feature = "faults")]
    fn slow_idle(&self) -> Duration {
        self.book
            .faults
            .lock()
            .map(|mut faults| {
                if !faults.slow_idle_snapshot.is_zero() {
                    faults.slowed = faults.slowed.saturating_add(1);
                }
                faults.slow_idle_snapshot
            })
            .unwrap_or_default()
    }

    #[cfg(not(feature = "faults"))]
    const fn slow_idle(&self) -> Duration {
        Duration::ZERO
    }

    #[must_use]
    pub const fn writing(&self) -> bool {
        self.book.writing.is_some()
    }

    fn wait_write(&mut self, number: u32) {
        if self.book.writing_for(number) {
            self.book.landed(Wait::For(self.clock.left()));
        }
    }

    fn snap_waiting(&self, number: u32) -> bool {
        self.book.writing_for(number)
            || self
                .book
                .held
                .get(&number)
                .and_then(|held| held.snap_failure.as_ref())
                .is_some_and(|failure| failure.next > Instant::now())
    }

    #[must_use]
    pub fn snapshot_failures(&self) -> usize {
        self.book
            .held
            .values()
            .filter(|held| held.snap_failure.is_some())
            .count()
    }

    fn forget(&mut self, number: u32) {
        self.live.remove(&number);
        let old = self
            .book
            .held
            .remove(&number)
            .and_then(|held| held.ledger.sum());
        self.book.memory.free(number);
        if let Some(keep) = self.book.keep.as_ref() {
            keep.forget(number);
        }
        self.book.release(old);
    }

    #[must_use]
    pub fn status(&self) -> Status {
        let device = self.host.map_or(Json::Null, |host| {
            map(vec![
                ("name", text(host.name())),
                ("subgroup", counted(host.width())),
                ("heap_bytes", budget::counted64(host.heap())),
            ])
        });
        let many = |count: usize| counted(u32::try_from(count).unwrap_or(u32::MAX));
        Status {
            device,
            memory: self.book.memory.shown(),
            worlds: map(vec![
                ("open", many(self.live.len())),
                (
                    "lost",
                    many(self.book.held.len().saturating_sub(self.live.len())),
                ),
                ("kept_earlier", many(self.book.earlier.len())),
                ("snapshot_failures", many(self.snapshot_failures())),
                ("restorations", budget::counted64(self.book.restorations)),
                (
                    "ignored",
                    Json::List(
                        self.book
                            .ignored
                            .iter()
                            .take(IGNORED_SHOWN)
                            .map(|one| text(one))
                            .collect(),
                    ),
                ),
                ("ignored_total", many(self.book.ignored.len())),
            ]),
            submissions: map(vec![
                (
                    "batches",
                    budget::counted64(self.book.stats.batches.load(Ordering::SeqCst)),
                ),
                (
                    "longest_ms",
                    budget::counted64(
                        u64::try_from(self.book.stats.longest.as_millis()).unwrap_or(u64::MAX),
                    ),
                ),
                ("longest_frames", counted(self.book.stats.longest_frames)),
                ("rates", self.book.rates.shown()),
                (
                    "read_waits",
                    budget::counted64(
                        self.live
                            .values()
                            .fold(0_u64, |all, one| all.saturating_add(one.world.read_waits())),
                    ),
                ),
                (
                    "words_read",
                    budget::counted64(
                        self.live
                            .values()
                            .fold(0_u64, |all, one| all.saturating_add(one.world.words_read())),
                    ),
                ),
            ]),
            directory: self
                .book
                .keep
                .as_ref()
                .map_or(Json::Null, |keep| text(&keep.root().to_string_lossy())),
        }
    }

    /// Answers one request, as [`crate::serve::handle`] does for the 2D
    /// tools.
    pub fn handle(&mut self, request: &Json) -> Option<Json> {
        transport::handle(self, request)
    }

    /// Reads one line and answers it, as [`crate::serve::heard`] does.
    pub fn heard(&mut self, line: &str) -> Option<Json> {
        transport::heard(self, line)
    }

    fn noted(&mut self, number: Option<u32>) -> Option<Json> {
        number
            .and_then(|number| self.book.held.get_mut(&number))
            .and_then(|held| held.note.take())
    }

    fn run_tool(&mut self, tool: &Tool, arguments: &Json) -> Json {
        let given =
            match schema::checked(tool.name, &input(&(tool.shown)()), arguments, Check::Whole) {
                Ok(given) => given,
                Err(refusal) => return world_refusal(refusal, arguments, Vec::new()),
            };
        let budget_ms = given
            .get("budget_ms")
            .and_then(Json::int)
            .and_then(|one| u64::try_from(one).ok())
            .unwrap_or(budget::BUDGET_MS);
        let arrived = self.arrived.take().unwrap_or_else(Instant::now);
        self.clock = Clock::of(arrived, budget_ms);
        if self.cancelled() {
            return world_refusal(
                Refusal::new(
                    errors::Code::Cancelled,
                    "$",
                    "the call was cancelled before it began; nothing ran".to_owned(),
                ),
                arguments,
                Vec::new(),
            );
        }
        if self.clock.left().is_zero() {
            return world_refusal(
                Refusal::new(
                    errors::Code::Budget,
                    "$.budget_ms",
                    format!(
                        "the call waited {} ms behind earlier calls, past its budget of {budget_ms} ms, so nothing ran",
                        self.clock.spent().as_millis()
                    ),
                )
                .limited(budget::counted64(budget_ms)),
                arguments,
                Vec::new(),
            );
        }
        let wedged_before = self.wedged();
        #[cfg(feature = "faults")]
        {
            let armed = self
                .book
                .faults
                .lock()
                .map(|mut faults| std::mem::take(&mut faults.wedge_before_call))
                .unwrap_or(false);
            if armed && let Some(host) = self.host {
                host.wedge();
            }
        }
        let number = number_of(&given, "world");
        let ran = (tool.run)(self, &given);
        let lost = (!wedged_before && self.wedged())
            || ran
                .as_ref()
                .err()
                .is_some_and(|why| lost_device(why.as_ref()));
        if lost {
            self.lose();
        }
        let note = self.noted(number);
        match ran {
            Ok(Json::Map(mut result)) => {
                if let Some(note) = note {
                    result.push(("restored".to_owned(), note));
                }
                delivery(Json::Map(result))
            }
            Ok(result) => delivery(result),
            Err(why) => {
                let (mut refusal, mut more) = refused_with(why.as_ref());
                if lost && why.downcast_ref::<crate::host::Error>().is_some() {
                    refusal.code = errors::Code::DeviceLost;
                }
                if let Some(note) = note {
                    more.push(("restored", note));
                }
                world_refusal(refusal, arguments, more)
            }
        }
    }
}

fn io_refusal(why: &std::io::Error) -> Failed {
    Box::new(Refusal::new(
        errors::Code::Io,
        "$",
        format!("the server directory could not be written: {why}"),
    ))
}

fn rule_of(host: &Host, name: &str) -> Result<Box<dyn Listed>, Failed> {
    let mut known = rules_on(Some(host));
    let at = known
        .iter()
        .position(|one| one.name() == name)
        .ok_or_else(|| {
            Box::new(crate::branch::Refused::Rule {
                name: name.to_owned(),
            }) as Failed
        })?;
    Ok(known.swap_remove(at))
}

fn paced(
    world: &mut World<'_>,
    frames: u32,
    pace: &mut Pace,
    guard: &budget::Guard,
) -> Result<(), Failed> {
    let mut done = 0_u32;
    while done < frames {
        if let Some(stop) = guard.stop() {
            return Err(Box::new(budget::Cut(stop)));
        }
        let step = pace.batch(frames - done, guard.left());
        if step == 0 {
            return Err(Box::new(budget::Cut(Stop::Budget)));
        }
        let began = Instant::now();
        world.step(step)?;
        pace.ran(step, began.elapsed());
        done += step;
    }
    Ok(())
}

fn stored(keep: &Keep, number: u32, image: &crate::branch::Image) -> Result<[u8; 32], Failed> {
    let pending = keep.pending(number);
    crate::branch::format::store(&pending, image)?;
    let (sum, _) = keep.settle(&pending).map_err(|why| {
        let _ = std::fs::remove_file(&pending);
        io_refusal(&why)
    })?;
    Ok(sum)
}

pub fn shaped(host: &Host, _: &dyn Listed, made: &Made) -> Result<(Grid, Vec<usize>), Failed> {
    let [wide, deep, tall] = made.size;
    let grid = Grid::solid(wide, deep, tall)?;
    if grid.tall < 2 {
        return Err(Box::new(Refusal::new(
            errors::Code::Lane,
            "$.tall",
            format!(
                "a world of {wide} by {deep} by {tall} cells is not solid: tall needs at least 2 cells"
            ),
        )));
    }
    if !made.sparse && Grid3::new(grid.wide, grid.deep, grid.tall).is_none() {
        let width = host.width();
        let blocks = grid.blocks().div_ceil(sweep::cells(width));
        Plan::over_in(width, blocks, Dimension::Three)?;
        return Err(Box::new(Refusal::new(
            errors::Code::Unfit,
            "$",
            format!(
                "a dense world of {wide} by {deep} by {tall} cells is not solid; ask for a sparse one"
            ),
        )));
    }
    let sizes = crate::branch::sizes_of(host, &made.branch())?;
    Ok((grid, sizes))
}

pub fn image_shaped(
    host: &Host,
    rule: &dyn Listed,
    image: &crate::branch::Image,
) -> Result<Vec<usize>, Failed> {
    let mut made = Made::of_image(image);
    if let crate::branch::State::Sparse {
        capacity,
        sides,
        pool,
        ..
    } = &image.state
    {
        let [wide, deep, tall] = image.size;
        let grid = Grid::solid(wide, deep, tall)?;
        let width = host.width();
        let blocks = grid.blocks().div_ceil(sweep::cells(width));
        let plan = Plan::bricked(width, blocks)?;
        let probe = Bricks::new(grid, &plan, 1)?;
        let per = crate::branch::format::brick_cells(*sides).max(1);
        let held = u32::try_from(pool.len() / per).unwrap_or(u32::MAX);
        made.slots = Some(if probe.sides() == *sides {
            (*capacity).max(held)
        } else {
            plan.blocks
        });
    }
    Ok(shaped(host, rule, &made)?.1)
}

fn world_bytes(_: &Host, live: &Live<'_>) -> u64 {
    budget::world_cost(live.world.sizes())
}

impl transport::Served for Server<'_> {
    fn listed(&self) -> Vec<Json> {
        listing_all()
    }

    fn called(&mut self, name: &str, arguments: &Json) -> Option<Json> {
        let Some(tool) = served().find(|tool| tool.name == name) else {
            let answer = flat_called(self.host, &flat(), name, arguments);
            if self.wedged() {
                self.lose();
            }
            return answer;
        };
        Some(self.run_tool(tool, arguments))
    }
}
