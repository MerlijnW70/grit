use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::branch::format::{Image, Sha256, State, UNIFORM, brick_cells};
use crate::branch::{self, World};
use crate::host::Host;

use super::cost::{Cost, Priced};
use super::errors::{Code, Refusal, shown};

pub const NAME_MOST: usize = 64;

pub const NUMBER_MOST: u32 = 1 << 31;

static PENDING: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub const SUFFIX: &str = ".grit";

const KEPT: &str = "kept";

const NEXT: &str = "next";

const JOURNAL_LEAST: usize = 64 << 10;

const REPLAY_LEAST: Duration = Duration::from_secs(2);

pub const HARD: u32 = 2;

pub const WRITING_REPLAY_MOST: Duration = Duration::from_secs(120);

pub const WRITING_BYTES_FACTOR: usize = 8;

pub const WRITE_CHUNK: usize = 32;

const SNAPSHOT_BYTES_PER_SECOND: f64 = 2.5e8;

const DEVICES: [&str; 4] = ["con", "prn", "aux", "nul"];

const NUMBERED: [&str; 2] = ["com", "lpt"];

const CREATED: u32 = 1;

const SNAPSHOT: u32 = 2;

const WRITE: u32 = 3;

const STEP: u32 = 4;

const LAW: u32 = 5;

const PARAMS: u32 = 6;

const BACK: u32 = 7;

const FORMAT: u32 = 8;

const WRITE_PLANE: u32 = 9;

pub const FORMAT_VERSION: u32 = 2;

const PRICED: u32 = 1 << 8;

const PRICE_WORDS: usize = 3;

const RECORD_WORDS_MOST: usize = 1 << 24;

const BACK_BATCH: u32 = 64;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Made {
    pub rule: String,
    pub params: Vec<u32>,
    pub size: [u32; 3],
    pub sparse: bool,
    pub slots: Option<u32>,
}

impl Made {
    #[must_use]
    pub fn branch(&self) -> branch::Made<'_> {
        branch::Made {
            rule: &self.rule,
            params: &self.params,
            size: self.size,
            sparse: self.sparse,
            slots: self.slots,
        }
    }

    #[must_use]
    pub fn of_image(image: &Image) -> Self {
        let (sparse, slots) = match &image.state {
            State::Dense(_) => (false, None),
            State::Sparse { capacity, .. } => (true, Some(*capacity)),
        };
        Self {
            rule: image.law.name().to_owned(),
            params: image.params.clone(),
            size: image.size,
            sparse,
            slots,
        }
    }
}

#[derive(Debug)]
pub struct Snap {
    pub sum: [u8; 32],
    pub frame: u32,
    pub made: Made,
    pub image: Option<Image>,
}

#[derive(Debug, Clone)]
pub enum Base {
    Created(Made),
    Snapshot(Arc<Snap>),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Entry {
    Write(Vec<([u32; 3], u32)>),
    WritePlane(u32, Vec<([u32; 3], u32)>),
    Step(u32),
    Params(Vec<u32>),
    Back(u32),
}

impl Entry {
    #[must_use]
    pub fn bytes(&self) -> usize {
        match self {
            Self::Write(cells) => 16 * cells.len() + 8,
            Self::WritePlane(_, cells) => 16 * cells.len() + 12,
            Self::Step(_) | Self::Back(_) => 8,
            Self::Params(params) => 4 * params.len() + 8,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Ledger {
    pub base: Base,
    pub law: Option<Arc<str>>,
    pub entries: Vec<Entry>,
    pub frame: u32,
    recorded: Vec<Duration>,
    costs: Vec<Cost>,
    pub unreadable: Option<String>,
    bytes: usize,
    replay: Duration,
    snapping: Duration,
}

impl Ledger {
    #[must_use]
    pub const fn created(made: Made) -> Self {
        Self {
            base: Base::Created(made),
            law: None,
            entries: Vec::new(),
            frame: 0,
            recorded: Vec::new(),
            costs: Vec::new(),
            unreadable: None,
            bytes: 0,
            replay: Duration::ZERO,
            snapping: Duration::ZERO,
        }
    }

    #[must_use]
    pub fn snapped(snap: Arc<Snap>) -> Self {
        Self {
            frame: snap.frame,
            base: Base::Snapshot(snap),
            law: None,
            entries: Vec::new(),
            recorded: Vec::new(),
            costs: Vec::new(),
            unreadable: None,
            bytes: 0,
            replay: Duration::ZERO,
            snapping: Duration::ZERO,
        }
    }

    #[must_use]
    pub fn with_law(mut self, law: Option<Arc<str>>) -> Self {
        self.law = law;
        self
    }

    #[must_use]
    pub fn journal(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        match &self.base {
            Base::Created(made) => (0_u8, made).hash(&mut hasher),
            Base::Snapshot(snap) => (1_u8, snap.sum, snap.frame, &snap.made).hash(&mut hasher),
        }
        let recorded: Vec<u32> = self.recorded.iter().copied().map(micros).collect();
        (&self.law, &self.entries, recorded, self.frame).hash(&mut hasher);
        hasher.finish()
    }

    #[must_use]
    pub fn made(&self) -> &Made {
        match &self.base {
            Base::Created(made) => made,
            Base::Snapshot(snap) => &snap.made,
        }
    }

    #[must_use]
    pub fn sum(&self) -> Option<[u8; 32]> {
        match &self.base {
            Base::Created(_) => None,
            Base::Snapshot(snap) => Some(snap.sum),
        }
    }

    pub fn push(&mut self, entry: Entry, spent: Duration) {
        let cost = priced(self.made(), &entry, None);
        self.push_priced(entry, spent, cost);
    }

    pub fn push_priced(&mut self, entry: Entry, spent: Duration, cost: Cost) {
        match entry {
            Entry::Step(frames) => self.frame = self.frame.wrapping_add(frames),
            Entry::Back(frames) => self.frame = self.frame.wrapping_sub(frames),
            Entry::Write(_) | Entry::WritePlane(..) | Entry::Params(_) => {}
        }
        self.bytes = self.bytes.saturating_add(entry.bytes());
        self.replay = self.replay.saturating_add(spent);
        self.recorded.push(spent);
        self.costs.push(cost);
        self.entries.push(entry);
    }

    #[must_use]
    pub fn costs(&self) -> &[Cost] {
        &self.costs
    }

    #[must_use]
    pub fn units(&self) -> u64 {
        self.units_from(0)
    }

    #[must_use]
    pub fn units_from(&self, from: usize) -> u64 {
        self.costs
            .iter()
            .skip(from)
            .fold(0_u64, |all, cost| all.saturating_add(cost.units))
    }

    #[must_use]
    pub fn due(&self, world_bytes: u64, ratio: f64) -> bool {
        self.over(Duration::ZERO, world_bytes, ratio)
    }

    #[must_use]
    pub fn over(&self, extra: Duration, world_bytes: u64, ratio: f64) -> bool {
        self.past(extra, world_bytes, ratio, 1)
    }

    #[must_use]
    pub fn far_over(&self, extra: Duration, world_bytes: u64, ratio: f64) -> bool {
        self.past(extra, world_bytes, ratio, HARD)
    }

    #[must_use]
    pub fn far_over_writing(
        &self,
        extra: Duration,
        world_bytes: u64,
        ratio: f64,
        writing: Option<(usize, Duration)>,
    ) -> bool {
        let Some((from, elapsed)) = writing else {
            return self.far_over(extra, world_bytes, ratio);
        };
        let from = from.min(self.entries.len());
        let (tail_bytes, tail_replay) = self.entries.iter().zip(&self.recorded).skip(from).fold(
            (0_usize, Duration::ZERO),
            |(bytes, replay), (entry, spent)| {
                (
                    bytes.saturating_add(entry.bytes()),
                    replay.saturating_add(*spent),
                )
            },
        );
        let bound = usize::try_from(world_bytes)
            .unwrap_or(usize::MAX)
            .max(JOURNAL_LEAST);
        let ratio = ratio.max(1.0);
        let hard = self.replay_most().saturating_mul(HARD);
        let tail_most = self
            .replay_most()
            .max(elapsed.saturating_mul(4))
            .saturating_mul(HARD);
        let tail = super::budget::scaled(tail_replay.saturating_add(extra), ratio);
        let whole = super::budget::scaled(self.replay.saturating_add(extra), ratio);
        tail_bytes > bound.saturating_mul(HARD as usize)
            || tail > tail_most
            || self.bytes > bound.saturating_mul(WRITING_BYTES_FACTOR)
            || whole > hard.max(WRITING_REPLAY_MOST)
    }

    fn past(&self, extra: Duration, world_bytes: u64, ratio: f64, times: u32) -> bool {
        let bytes = usize::try_from(world_bytes)
            .unwrap_or(usize::MAX)
            .max(JOURNAL_LEAST)
            .saturating_mul(times as usize);
        let replaying = super::budget::scaled(self.replay.saturating_add(extra), ratio.max(1.0));
        self.bytes > bytes || replaying > self.replay_most().saturating_mul(times)
    }

    #[must_use]
    pub fn replay_most(&self) -> Duration {
        REPLAY_LEAST.max(self.snapping.saturating_mul(4))
    }

    #[must_use]
    pub const fn replay(&self) -> Duration {
        self.replay
    }

    #[must_use]
    pub fn step_recorded(&self) -> Duration {
        self.entries
            .iter()
            .zip(&self.recorded)
            .filter(|(entry, _)| matches!(entry, Entry::Step(_)))
            .fold(Duration::ZERO, |all, (_, spent)| all.saturating_add(*spent))
    }

    #[must_use]
    pub fn write_bytes(&self) -> u64 {
        self.entries
            .iter()
            .filter(|entry| matches!(entry, Entry::Write(_) | Entry::WritePlane(..)))
            .fold(0_u64, |all, entry| all.saturating_add(entry.bytes() as u64))
    }

    pub fn rebase(&mut self, snap: Arc<Snap>, cost: Duration) {
        let snapping = cost;
        let law = self.law.take();
        *self = Self::snapped(snap).with_law(law);
        self.snapping = snapping;
    }

    pub fn rebase_at(&mut self, snap: Arc<Snap>, cost: Duration, from: usize) {
        let entries = self.entries.split_off(from.min(self.entries.len()));
        let recorded = self.recorded.split_off(from.min(self.recorded.len()));
        let costs = self.costs.split_off(from.min(self.costs.len()));
        let mut ledger = Self::snapped(snap).with_law(self.law.take());
        ledger.snapping = cost;
        for ((entry, spent), priced) in entries.into_iter().zip(recorded).zip(costs) {
            ledger.push_priced(entry, spent, priced);
        }
        *self = ledger;
    }

    #[must_use]
    pub const fn journal_bytes(&self) -> usize {
        self.bytes
    }

    #[must_use]
    pub fn snapping(&self, world_bytes: u64) -> Duration {
        if self.snapping.is_zero() {
            Duration::from_secs_f64(world_bytes as f64 / SNAPSHOT_BYTES_PER_SECOND)
        } else {
            self.snapping
        }
    }
}

#[must_use]
pub fn hex(sum: &[u8; 32]) -> String {
    sum.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn path_refusal(field: &str, message: String) -> Refusal {
    Refusal::new(Code::Path, field, message)
}

pub fn file_name(name: &str, field: &str) -> Result<String, Refusal> {
    let told = shown(&format!("{name:?}"));
    if name.is_empty() {
        return Err(path_refusal(field, "the name is empty".to_owned()));
    }
    if name.len() > NAME_MOST {
        return Err(path_refusal(
            field,
            format!(
                "the name {told} holds {} bytes, more than the {NAME_MOST} a name may hold",
                name.len()
            ),
        )
        .limited(crate::wire::Json::Int(NAME_MOST as i64)));
    }
    if let Some((at, wrong)) = name.char_indices().find(|(_, one)| {
        !(one.is_ascii_lowercase() || one.is_ascii_digit() || *one == '_' || *one == '-')
    }) {
        return Err(path_refusal(
            field,
            format!(
                "the name {told} holds {wrong:?} at byte {at}; a name is made of a-z, 0-9, _ and - only, so it cannot leave the server directory"
            ),
        ));
    }
    if name.starts_with(['_', '-']) {
        return Err(path_refusal(
            field,
            format!(
                "the name {told} starts with {:?}; start it with a letter or digit",
                name.chars().next().unwrap_or('_')
            ),
        ));
    }
    let device = DEVICES.contains(&name)
        || NUMBERED.iter().any(|stem| {
            name.strip_prefix(stem)
                .is_some_and(|rest| rest.len() == 1 && rest.bytes().all(|one| one.is_ascii_digit()))
        });
    if device {
        return Err(path_refusal(
            field,
            format!("the name {told} is a device on Windows, not a file"),
        ));
    }
    Ok(format!("{name}{SUFFIX}"))
}

pub fn regular(path: &Path, field: &str) -> Result<Option<u64>, Refusal> {
    match fs::symlink_metadata(path) {
        Err(why) if why.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(why) => Err(Refusal::new(
            Code::Io,
            field,
            format!("the server could not look at the file: {why}"),
        )),
        Ok(found) if found.file_type().is_symlink() => Err(path_refusal(
            field,
            "the name is a link in the server directory; the server reads and writes only its own files"
                .to_owned(),
        )),
        Ok(found) if !found.is_file() => Err(path_refusal(
            field,
            "the name is not a file in the server directory".to_owned(),
        )),
        Ok(found) => Ok(Some(found.len())),
    }
}

pub fn tail_sum(path: &Path) -> io::Result<[u8; 32]> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::End(-32))?;
    let mut sum = [0_u8; 32];
    file.read_exact(&mut sum)?;
    Ok(sum)
}

#[derive(Debug, Clone)]
pub struct Keep {
    root: PathBuf,
    kept: PathBuf,
    lock: Option<Arc<File>>,
}

pub const LOCK: &str = "lock";

#[cfg(unix)]
fn locked(file: &File) -> io::Result<bool> {
    use std::os::unix::io::AsRawFd;
    unsafe extern "C" {
        fn flock(fd: std::ffi::c_int, operation: std::ffi::c_int) -> std::ffi::c_int;
    }
    const EXCLUSIVE: std::ffi::c_int = 2;
    const WITHOUT_WAITING: std::ffi::c_int = 4;
    let done = unsafe { flock(file.as_raw_fd(), EXCLUSIVE | WITHOUT_WAITING) };
    if done == 0 {
        return Ok(true);
    }
    let why = io::Error::last_os_error();
    if why.kind() == io::ErrorKind::WouldBlock {
        Ok(false)
    } else {
        Err(why)
    }
}

#[cfg(unix)]
fn lock_file(path: &Path) -> io::Result<Option<File>> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    Ok(locked(&file)?.then_some(file))
}

#[cfg(windows)]
fn lock_file(path: &Path) -> io::Result<Option<File>> {
    use std::os::windows::fs::OpenOptionsExt;
    const SHARING_VIOLATION: i32 = 32;
    match OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .open(path)
    {
        Ok(file) => Ok(Some(file)),
        Err(why) if why.raw_os_error() == Some(SHARING_VIOLATION) => Ok(None),
        Err(why) => Err(why),
    }
}

impl Keep {
    pub fn at(dir: &Path) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        let root = fs::canonicalize(dir)?;
        let kept = root.join(KEPT);
        fs::create_dir_all(&kept)?;
        if fs::symlink_metadata(&kept)?.file_type().is_symlink() {
            return Err(io::Error::other("the kept directory is a link"));
        }
        Ok(Self {
            root,
            kept,
            lock: None,
        })
    }

    pub fn locked(dir: &Path) -> io::Result<Self> {
        let mut keep = Self::at(dir)?;
        let path = keep.kept.join(LOCK);
        match lock_file(&path)? {
            Some(file) => {
                keep.lock = Some(Arc::new(file));
                Ok(keep)
            }
            None => Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                format!(
                    "another grit server holds the directory {}: its lock {} is taken; give this server a directory of its own",
                    keep.root.display(),
                    path.display()
                ),
            )),
        }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn saved(&self, name: &str, field: &str) -> Result<PathBuf, Refusal> {
        Ok(self.root.join(file_name(name, field)?))
    }

    pub fn listed(&self) -> io::Result<Vec<(String, u64)>> {
        let mut found = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if !kind.is_file() {
                continue;
            }
            let file = entry.file_name();
            let Some(file) = file.to_str() else {
                continue;
            };
            let Some(name) = file.strip_suffix(SUFFIX) else {
                continue;
            };
            if file_name(name, "$").is_err() {
                continue;
            }
            let bytes = entry.metadata().map(|held| held.len()).unwrap_or(0);
            found.push((name.to_owned(), bytes));
        }
        found.sort();
        Ok(found)
    }

    #[must_use]
    pub fn snapshot(&self, sum: &[u8; 32]) -> PathBuf {
        self.kept.join(format!("snap-{}{SUFFIX}", hex(sum)))
    }

    #[must_use]
    pub fn pending(&self, world: u32) -> PathBuf {
        let count = PENDING.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.kept.join(format!(
            "pending-{world}-{}-{count}{SUFFIX}",
            std::process::id()
        ))
    }

    fn journal(&self, world: u32) -> PathBuf {
        self.kept.join(format!("world-{world}.journal"))
    }

    pub fn copied(&self, from: &Path, world: u32) -> io::Result<PathBuf> {
        let pending = self.pending(world);
        let _ = fs::remove_file(&pending);
        fs::copy(from, &pending)?;
        Ok(pending)
    }

    pub fn settle(&self, pending: &Path) -> io::Result<([u8; 32], PathBuf)> {
        let sum = tail_sum(pending)?;
        let path = self.snapshot(&sum);
        if path.exists() {
            fs::remove_file(pending)?;
        } else {
            fs::rename(pending, &path)?;
        }
        Ok((sum, path))
    }

    pub fn begin(&self, world: u32, ledger: &Ledger) -> io::Result<()> {
        let mut words = Vec::new();
        record(&mut words, base_record(&ledger.base));
        record(&mut words, (FORMAT, vec![FORMAT_VERSION]));
        if let Some(law) = &ledger.law {
            record(&mut words, (LAW, text_words(law)));
        }
        for (at, entry) in ledger.entries.iter().enumerate() {
            let spent = ledger.recorded.get(at).copied().unwrap_or_default();
            let cost = ledger
                .costs
                .get(at)
                .copied()
                .unwrap_or_else(|| priced(ledger.made(), entry, None));
            record(&mut words, priced_record(entry, spent, cost));
        }
        let path = self.journal(world);
        let temporary = self.kept.join(format!(".world-{world}.journal.tmp"));
        {
            let mut file = File::create(&temporary)?;
            file.write_all(&bytes_of(&words))?;
            file.sync_all()?;
        }
        fs::rename(&temporary, &path)
    }

    pub fn append(&self, world: u32, entry: &Entry, spent: Duration) -> io::Result<()> {
        self.appended(world, entry_record(entry, spent))
    }

    pub fn append_priced(
        &self,
        world: u32,
        entry: &Entry,
        spent: Duration,
        cost: Cost,
    ) -> io::Result<()> {
        self.appended(world, priced_record(entry, spent, cost))
    }

    fn appended(&self, world: u32, held: (u32, Vec<u32>)) -> io::Result<()> {
        let mut words = Vec::new();
        record(&mut words, held);
        let mut file = OpenOptions::new().append(true).open(self.journal(world))?;
        file.write_all(&bytes_of(&words))
    }

    pub fn sweep(&self, kept: &[[u8; 32]]) {
        let Ok(entries) = fs::read_dir(&self.kept) else {
            return;
        };
        let wanted: Vec<String> = kept
            .iter()
            .map(|sum| format!("snap-{}{SUFFIX}", hex(sum)))
            .collect();
        for entry in entries.flatten() {
            let file = entry.file_name();
            let Some(file) = file.to_str() else {
                continue;
            };
            let stale = (file.starts_with("snap-") && !wanted.iter().any(|one| one == file))
                || file.starts_with("pending-")
                || file.ends_with(".tmp");
            if stale && entry.file_type().is_ok_and(|kind| kind.is_file()) {
                let _ = fs::remove_file(entry.path());
            }
        }
    }

    pub fn forget(&self, world: u32) {
        let _ = fs::remove_file(self.journal(world));
    }

    pub fn drop_snapshot(&self, sum: &[u8; 32]) {
        let _ = fs::remove_file(self.snapshot(sum));
    }

    #[must_use]
    pub fn next(&self) -> u32 {
        self.next_checked().0.unwrap_or(1)
    }

    #[must_use]
    pub fn next_checked(&self) -> (Option<u32>, Option<String>) {
        let Ok(held) = fs::read_to_string(self.kept.join(NEXT)) else {
            return (None, None);
        };
        match held.trim().parse::<u32>() {
            Ok(next) if (1..=NUMBER_MOST).contains(&next) => (Some(next), None),
            _ => (
                None,
                Some(format!(
                    "kept/{NEXT}: {} is not a world number from 1 to {NUMBER_MOST}, so it was ignored",
                    shown(&format!("{:?}", held.trim()))
                )),
            ),
        }
    }

    pub fn set_next(&self, next: u32) -> io::Result<()> {
        let temporary = self.kept.join(".next.tmp");
        fs::write(&temporary, next.to_string())?;
        fs::rename(&temporary, self.kept.join(NEXT))
    }

    #[must_use]
    pub fn earlier(&self) -> Vec<(u32, Ledger)> {
        self.scan().0
    }

    #[must_use]
    pub fn scan(&self) -> (Vec<(u32, Ledger)>, Vec<String>) {
        let mut ignored = Vec::new();
        let Ok(entries) = fs::read_dir(&self.kept) else {
            return (Vec::new(), ignored);
        };
        let mut found = Vec::new();
        for entry in entries.flatten() {
            let file = entry.file_name();
            let Some(stem) = file
                .to_str()
                .and_then(|file| file.strip_prefix("world-"))
                .and_then(|file| file.strip_suffix(".journal"))
            else {
                continue;
            };
            let named = format!("kept/{}", shown(&file.to_string_lossy()));
            let number = stem
                .parse::<u32>()
                .ok()
                .filter(|number| (1..=NUMBER_MOST).contains(number) && number.to_string() == stem);
            let Some(number) = number else {
                ignored.push(format!(
                    "{named}: not a world number from 1 to {NUMBER_MOST}, so it was ignored"
                ));
                continue;
            };
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if !kind.is_file() {
                ignored.push(format!("{named}: not a file, so it was ignored"));
                continue;
            }
            match self.read_journal(number) {
                Some(ledger) => found.push((number, ledger)),
                None => ignored.push(format!(
                    "{named}: its first record is not a whole world, so it was ignored"
                )),
            }
        }
        found.sort_by_key(|(number, _)| *number);
        ignored.sort();
        (found, ignored)
    }

    fn read_journal(&self, world: u32) -> Option<Ledger> {
        let bytes = fs::read(self.journal(world)).ok()?;
        let words: Vec<u32> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|chunk| u32::from_le_bytes(*chunk))
            .collect();
        let mut records = Records {
            words: &words,
            at: 0,
        };
        let (tag, payload) = records.next()?;
        let base = base_of(tag, payload)?;
        let mut ledger = match base {
            Base::Created(made) => Ledger::created(made),
            Base::Snapshot(snap) => Ledger::snapped(snap),
        };
        let mut index = 0_usize;
        let mut ahead = records.at;
        let mut version = 1_u32;
        if let Some((FORMAT, payload)) = records.next() {
            if payload != [FORMAT_VERSION] {
                ledger.unreadable = Some(format!(
                    "its journal is in format {}, and this server reads formats 1 and {FORMAT_VERSION}",
                    payload.first().copied().unwrap_or_default()
                ));
                return Some(ledger);
            }
            version = FORMAT_VERSION;
            ahead = records.at;
        }
        records.at = ahead;
        if let Some((LAW, payload)) = records.next() {
            ledger.law = Some(Arc::from(text_of(payload)?));
            index += 1;
            ahead = records.at;
        }
        records.at = ahead;
        while let Some((tag, payload)) = records.next() {
            index += 1;
            let read = if tag & PRICED == 0 {
                entry_of(tag, payload).map(|(entry, spent)| {
                    let cost = priced(ledger.made(), &entry, None);
                    (entry, spent, cost)
                })
            } else if version < FORMAT_VERSION {
                None
            } else {
                priced_of(tag & !PRICED, payload)
            };
            let Some((entry, spent, cost)) = read else {
                ledger.unreadable = Some(format!(
                    "record {index} of its journal has a good checksum and is not a record this server reads (tag {tag}, {} words)",
                    payload.len()
                ));
                break;
            };
            ledger.push_priced(entry, spent, cost);
        }
        Some(ledger)
    }
}

struct Records<'words> {
    words: &'words [u32],
    at: usize,
}

impl<'words> Records<'words> {
    fn next(&mut self) -> Option<(u32, &'words [u32])> {
        let tag = *self.words.get(self.at)?;
        let length = *self.words.get(self.at + 1)? as usize;
        if length > RECORD_WORDS_MOST {
            return None;
        }
        let payload = self.words.get(self.at + 2..self.at + 2 + length)?;
        let check = *self.words.get(self.at + 2 + length)?;
        if check != checked(tag, payload) {
            return None;
        }
        self.at += 3 + length;
        Some((tag, payload))
    }
}

fn checked(tag: u32, payload: &[u32]) -> u32 {
    let mut hash = 0x811C_9DC5_u32;
    for word in std::iter::once(tag)
        .chain(std::iter::once(
            u32::try_from(payload.len()).unwrap_or(u32::MAX),
        ))
        .chain(payload.iter().copied())
    {
        for byte in word.to_le_bytes() {
            hash ^= u32::from(byte);
            hash = hash.wrapping_mul(0x0100_0193);
        }
    }
    hash
}

fn record(words: &mut Vec<u32>, (tag, payload): (u32, Vec<u32>)) {
    let check = checked(tag, &payload);
    words.push(tag);
    words.push(u32::try_from(payload.len()).unwrap_or(u32::MAX));
    words.extend_from_slice(&payload);
    words.push(check);
}

fn bytes_of(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}

fn text_words(held: &str) -> Vec<u32> {
    let bytes = held.as_bytes();
    let mut words = Vec::with_capacity(1 + bytes.len().div_ceil(4));
    words.push(u32::try_from(bytes.len()).unwrap_or(u32::MAX));
    for chunk in bytes.chunks(4) {
        let mut word = [0_u8; 4];
        if let Some(into) = word.get_mut(..chunk.len()) {
            into.copy_from_slice(chunk);
        }
        words.push(u32::from_le_bytes(word));
    }
    words
}

fn text_of(words: &[u32]) -> Option<String> {
    let (&length, rest) = words.split_first()?;
    let length = length as usize;
    if rest.len() != length.div_ceil(4) {
        return None;
    }
    let bytes: Vec<u8> = rest
        .iter()
        .flat_map(|word| word.to_le_bytes())
        .take(length)
        .collect();
    String::from_utf8(bytes).ok()
}

fn made_words(made: &Made, words: &mut Vec<u32>) {
    let name = made.rule.as_bytes();
    words.push(u32::try_from(name.len()).unwrap_or(u32::MAX));
    for chunk in name.chunks(4) {
        let mut held = [0_u8; 4];
        if let Some(into) = held.get_mut(..chunk.len()) {
            into.copy_from_slice(chunk);
        }
        words.push(u32::from_le_bytes(held));
    }
    words.push(u32::try_from(made.params.len()).unwrap_or(u32::MAX));
    words.extend_from_slice(&made.params);
    words.extend_from_slice(&made.size);
    words.push(u32::from(made.sparse));
    words.push(u32::from(made.slots.is_some()));
    words.push(made.slots.unwrap_or(0));
}

fn made_of(words: &[u32]) -> Option<(Made, &[u32])> {
    let (&length, rest) = words.split_first()?;
    let length = length as usize;
    if length > NAME_MOST {
        return None;
    }
    let packed = length.div_ceil(4);
    let name: Vec<u8> = rest
        .get(..packed)?
        .iter()
        .flat_map(|word| word.to_le_bytes())
        .take(length)
        .collect();
    let rule = String::from_utf8(name).ok()?;
    let rest = rest.get(packed..)?;
    let (&count, rest) = rest.split_first()?;
    let count = count as usize;
    let params = rest.get(..count)?.to_vec();
    let rest = rest.get(count..)?;
    let [wide, deep, tall, sparse, slotted, slots] = <[u32; 6]>::try_from(rest.get(..6)?).ok()?;
    let made = Made {
        rule,
        params,
        size: [wide, deep, tall],
        sparse: sparse == 1,
        slots: (slotted == 1).then_some(slots),
    };
    Some((made, rest.get(6..)?))
}

fn base_record(base: &Base) -> (u32, Vec<u32>) {
    let mut words = Vec::new();
    match base {
        Base::Created(made) => {
            made_words(made, &mut words);
            (CREATED, words)
        }
        Base::Snapshot(snap) => {
            words.extend(
                snap.sum
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|chunk| u32::from_le_bytes(*chunk)),
            );
            words.push(snap.frame);
            made_words(&snap.made, &mut words);
            (SNAPSHOT, words)
        }
    }
}

fn base_of(tag: u32, payload: &[u32]) -> Option<Base> {
    match tag {
        CREATED => {
            let (made, rest) = made_of(payload)?;
            rest.is_empty().then_some(Base::Created(made))
        }
        SNAPSHOT => {
            let sum_words = payload.get(..8)?;
            let mut sum = [0_u8; 32];
            for (into, word) in sum.as_chunks_mut::<4>().0.iter_mut().zip(sum_words) {
                *into = word.to_le_bytes();
            }
            let frame = *payload.get(8)?;
            let (made, rest) = made_of(payload.get(9..)?)?;
            rest.is_empty().then(|| {
                Base::Snapshot(Arc::new(Snap {
                    sum,
                    frame,
                    made,
                    image: None,
                }))
            })
        }
        _ => None,
    }
}

fn micros(spent: Duration) -> u32 {
    u32::try_from(spent.as_micros()).unwrap_or(u32::MAX)
}

fn entry_record(entry: &Entry, spent: Duration) -> (u32, Vec<u32>) {
    let micros = micros(spent);
    match entry {
        Entry::Write(cells) => {
            let mut words = Vec::with_capacity(cells.len() * 4);
            for (at, word) in cells {
                words.extend_from_slice(at);
                words.push(*word);
            }
            words.push(micros);
            (WRITE, words)
        }
        Entry::WritePlane(plane, cells) => {
            let mut words = Vec::with_capacity(cells.len() * 4 + 2);
            words.push(*plane);
            for (at, word) in cells {
                words.extend_from_slice(at);
                words.push(*word);
            }
            words.push(micros);
            (WRITE_PLANE, words)
        }
        Entry::Step(frames) => (STEP, vec![*frames, micros]),
        Entry::Back(frames) => (BACK, vec![*frames, micros]),
        Entry::Params(params) => {
            let mut words = params.clone();
            words.push(micros);
            (PARAMS, words)
        }
    }
}

fn priced_record(entry: &Entry, spent: Duration, cost: Cost) -> (u32, Vec<u32>) {
    let (tag, mut words) = entry_record(entry, spent);
    let [low, high] = [cost.units, cost.units >> 32].map(|half| (half & 0xFFFF_FFFF) as u32);
    words.extend_from_slice(&[low, high, cost.submits]);
    (tag | PRICED, words)
}

fn priced_of(tag: u32, payload: &[u32]) -> Option<(Entry, Duration, Cost)> {
    let (payload, price) = payload.split_at_checked(payload.len().checked_sub(PRICE_WORDS)?)?;
    let [low, high, submits] = <[u32; PRICE_WORDS]>::try_from(price).ok()?;
    let (entry, spent) = entry_of(tag, payload)?;
    let kind = match entry {
        Entry::Write(_) | Entry::WritePlane(..) => Priced::Write,
        Entry::Step(_) | Entry::Back(_) => Priced::Frame,
        Entry::Params(_) => Priced::Params,
    };
    let cost = Cost {
        kind,
        units: u64::from(low) | (u64::from(high) << 32),
        submits,
    };
    Some((entry, spent, cost))
}

#[must_use]
pub fn priced(made: &Made, entry: &Entry, bricks: Option<(u32, usize)>) -> Cost {
    match entry {
        Entry::Write(cells) | Entry::WritePlane(_, cells) => {
            super::tools::write_cost(cells.len(), made.sparse)
        }
        Entry::Step(frames) | Entry::Back(frames) => {
            super::tools::frames_cost(super::tools::frame_cells(made.size, bricks), *frames)
        }
        Entry::Params(params) => super::law::params_cost(params.len()),
    }
}

#[must_use]
pub fn replay_cost(ledger: &Ledger) -> Cost {
    ledger
        .costs
        .iter()
        .fold(Cost::none(Priced::Replay), |all, cost| all.plus(*cost))
}

fn entry_of(tag: u32, payload: &[u32]) -> Option<(Entry, Duration)> {
    let (&micros, payload) = payload.split_last()?;
    let spent = Duration::from_micros(u64::from(micros));
    match tag {
        WRITE => {
            let (cells, rest) = payload.as_chunks::<4>();
            rest.is_empty().then(|| {
                (
                    Entry::Write(
                        cells
                            .iter()
                            .map(|[x, y, z, word]| ([*x, *y, *z], *word))
                            .collect(),
                    ),
                    spent,
                )
            })
        }
        WRITE_PLANE => {
            let (&plane, payload) = payload.split_first()?;
            let (cells, rest) = payload.as_chunks::<4>();
            rest.is_empty().then(|| {
                (
                    Entry::WritePlane(
                        plane,
                        cells
                            .iter()
                            .map(|[x, y, z, word]| ([*x, *y, *z], *word))
                            .collect(),
                    ),
                    spent,
                )
            })
        }
        STEP => match payload {
            [frames] => Some((Entry::Step(*frames), spent)),
            _ => None,
        },
        PARAMS => Some((Entry::Params(payload.to_vec()), spent)),
        BACK => match payload {
            [frames] => Some((Entry::Back(*frames), spent)),
            _ => None,
        },
        _ => None,
    }
}

#[must_use]
pub fn state_of(image: &Image) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(image.law.name().as_bytes());
    hash.update(&[0]);
    let words = |hash: &mut Sha256, held: &[u32]| {
        for chunk in held.chunks(1 << 14) {
            let bytes: Vec<u8> = chunk.iter().flat_map(|word| word.to_le_bytes()).collect();
            hash.update(&bytes);
        }
    };
    words(
        &mut hash,
        &[u32::try_from(image.params.len()).unwrap_or(u32::MAX)],
    );
    words(&mut hash, &image.params);
    words(&mut hash, &[image.frame]);
    words(&mut hash, &image.size);
    match &image.state {
        State::Dense(cells) => {
            words(&mut hash, &[0]);
            words(&mut hash, cells);
        }
        State::Sparse {
            sides, table, pool, ..
        } => {
            words(&mut hash, &[1]);
            words(&mut hash, sides);
            let per = brick_cells(*sides);
            let planes = image.channels() as usize;
            if planes > 1 {
                words(&mut hash, &[u32::try_from(planes).unwrap_or(u32::MAX)]);
            }
            let count = table.len() / planes.saturating_add(1);
            let (entries, upper) = table.split_at(count.saturating_mul(2).min(table.len()));
            let held_count = pool.len() / per.saturating_mul(planes).max(1);
            for (brick, pair) in entries.as_chunks::<2>().0.iter().enumerate() {
                let [slot, value] = *pair;
                for plane in 0..planes {
                    if slot == UNIFORM {
                        let value = match plane.checked_sub(1) {
                            None => value,
                            Some(upper_plane) => upper
                                .get(upper_plane.saturating_mul(count).saturating_add(brick))
                                .copied()
                                .unwrap_or_default(),
                        };
                        words(&mut hash, &[0, value]);
                        continue;
                    }
                    let from = plane
                        .saturating_mul(held_count)
                        .saturating_add(slot as usize)
                        .saturating_mul(per);
                    let held = pool.get(from..from.saturating_add(per)).unwrap_or_default();
                    match held.split_first() {
                        Some((first, rest)) if rest.iter().all(|one| one == first) => {
                            words(&mut hash, &[0, *first]);
                        }
                        _ => {
                            words(&mut hash, &[1]);
                            words(&mut hash, held);
                        }
                    }
                }
            }
        }
    }
    hash.finish()
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Timings {
    pub build: Duration,
    pub file_bytes: u64,
    pub file: Duration,
    pub write_bytes: u64,
    pub writes: Duration,
    pub recorded: Duration,
    pub steps: Duration,
}

pub fn rebuilt<'host>(
    host: &'host Host,
    ledger: &Ledger,
    keep: Option<&Keep>,
    guard: &super::budget::Guard,
    timings: &mut Timings,
    mut stepping: impl FnMut(&mut World<'host>, u32) -> Result<(), super::Failed>,
) -> Result<World<'host>, super::Failed> {
    if let Some(stop) = guard.stop() {
        return Err(Box::new(super::budget::Halted {
            stop,
            entries: 0,
            of: ledger.entries.len(),
            frame: 0,
            goal: ledger.frame,
        }));
    }
    let began = guard.time.now();
    let mut world = match &ledger.base {
        Base::Created(made) => match &ledger.law {
            None => World::create(host, &made.branch())?,
            Some(law) => super::law::made_world(host, law, made)?,
        },
        Base::Snapshot(snap) => match (&snap.image, keep) {
            (Some(image), _) => super::law::opened(host, image)?,
            (None, Some(keep)) => {
                let path = keep.snapshot(&snap.sum);
                if tail_sum(&path).ok() != Some(snap.sum) {
                    return Err(Box::new(Refusal::new(
                        Code::BadFile,
                        "$",
                        format!(
                            "the snapshot {} is gone or changed, so the world cannot be rebuilt",
                            hex(&snap.sum)
                        ),
                    )));
                }
                let reading = guard.time.now();
                let image = crate::branch::format::fetch(&path)?;
                timings.file_bytes = fs::metadata(&path).map_or(0, |found| found.len());
                timings.file = guard.time.since(reading);
                super::law::opened(host, &image)?
            }
            (None, None) => {
                return Err(Box::new(Refusal::new(
                    Code::Path,
                    "$",
                    "the world's snapshot lies in a directory this server does not have".to_owned(),
                )));
            }
        },
    };
    timings.build = guard.time.since(began).saturating_sub(timings.file);
    let of = ledger.entries.len();
    for (at, entry) in ledger.entries.iter().enumerate() {
        let halted = |stop, frame| {
            Box::new(super::budget::Halted {
                stop,
                entries: at,
                of,
                frame,
                goal: ledger.frame,
            }) as super::Failed
        };
        if let Some(stop) = guard.stop() {
            return Err(halted(stop, world.frame()));
        }
        let written = match entry {
            Entry::Write(cells) => Some((0, cells)),
            Entry::WritePlane(plane, cells) => Some((*plane, cells)),
            _ => None,
        };
        match entry {
            Entry::Write(_) | Entry::WritePlane(..) => {
                let (plane, cells) = written.ok_or("a write entry holds no cells")?;
                let writing = guard.time.now();
                let chunk = if world.pool().is_some() {
                    cells.len().max(1)
                } else {
                    WRITE_CHUNK
                };
                for piece in cells.chunks(chunk) {
                    if let Some(stop) = guard.stop() {
                        return Err(halted(stop, world.frame()));
                    }
                    super::tools::faults::cells(guard.time.as_ref(), piece.len());
                    world.write_plane(plane, piece)?;
                }
                timings.writes = timings.writes.saturating_add(guard.time.since(writing));
                timings.write_bytes = timings.write_bytes.saturating_add(entry.bytes() as u64);
            }
            Entry::Params(params) => super::law::reparam(&mut world, params)?,
            Entry::Back(frames) => {
                let mut done = 0_u32;
                while done < *frames {
                    if let Some(stop) = guard.stop() {
                        return Err(halted(stop, world.frame()));
                    }
                    let step = BACK_BATCH.min(*frames - done);
                    world.back(step)?;
                    done += step;
                }
            }
            Entry::Step(frames) => {
                let stepped = guard.time.now();
                if let Err(why) = stepping(&mut world, *frames) {
                    return Err(match why.downcast_ref::<super::budget::Cut>() {
                        Some(cut) => halted(cut.0, world.frame()),
                        None => why,
                    });
                }
                timings.steps = timings.steps.saturating_add(guard.time.since(stepped));
                let recorded = ledger.recorded.get(at).copied().unwrap_or_default();
                timings.recorded = timings.recorded.saturating_add(recorded);
            }
        }
    }
    if world.frame() != ledger.frame {
        return Err(Box::new(Refusal::new(
            Code::DeviceLost,
            "$",
            format!(
                "the rebuilt world reached frame {} and its journal says {}",
                world.frame(),
                ledger.frame
            ),
        )));
    }
    Ok(world)
}
