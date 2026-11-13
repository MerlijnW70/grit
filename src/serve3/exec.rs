use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::host::{self, Host};
use crate::wire::{Json, map, text};

use super::errors::{Code, Refusal};
use super::schema::Check;
use super::transport::{self, Line, Served};
use super::{Book, Options, Server, Status};

pub const QUEUE_MOST: usize = 256;

pub const RESTORED_SHOWN: usize = 256;

pub type Opener = Box<dyn FnMut() -> Result<Option<Host>, host::Error> + Send>;

const QUEUED: u8 = 0;

const RUNNING: u8 = 1;

const ANSWERED: u8 = 2;

const WATCH: Duration = Duration::from_millis(5);

const LANDING: Duration = Duration::from_millis(10);

struct Job {
    request: Json,
    id: Json,
    arrived: Instant,
    cancel: Arc<AtomicBool>,
    state: Arc<AtomicU8>,
}

struct Flight {
    id: Json,
    cancel: Arc<AtomicBool>,
    state: Arc<AtomicU8>,
    arrived: Instant,
    budget_ms: u64,
    excused: Duration,
}

impl Flight {
    fn charged(&self, now: Instant, idle: Option<Instant>) -> Duration {
        let behind = idle.map_or(Duration::ZERO, |began| {
            now.saturating_duration_since(began.max(self.arrived))
        });
        now.saturating_duration_since(self.arrived)
            .saturating_sub(self.excused)
            .saturating_sub(behind)
    }
}

#[derive(Default)]
struct Flights {
    list: Vec<Flight>,
    idle: Option<Instant>,
}

struct Shared {
    status: Mutex<Status>,
    flights: Mutex<Flights>,
    batches: Arc<AtomicU64>,
    stopping: AtomicBool,
}

fn budget_of(request: &Json) -> u64 {
    request
        .get("params")
        .and_then(|params| params.get("arguments"))
        .and_then(|arguments| arguments.get("budget_ms"))
        .and_then(Json::int)
        .and_then(|one| u64::try_from(one).ok())
        .filter(|one| (1..=super::budget::BUDGET_MOST_MS).contains(one))
        .unwrap_or(super::budget::BUDGET_MS)
}

fn waited(flight: &Flight, charged: Duration) -> Json {
    let refusal = Refusal::new(
        Code::Budget,
        "$.budget_ms",
        format!(
            "the call waited {} ms behind earlier calls, past its budget of {} ms, so nothing ran",
            charged.as_millis(),
            flight.budget_ms
        ),
    )
    .limited(super::budget::counted64(flight.budget_ms));
    transport::answered(
        flight.id.clone(),
        super::world_refusal(refusal, &Json::Map(Vec::new()), Vec::new()),
    )
}

fn watch(shared: &Shared, answers: &Sender<Json>) {
    while !shared.stopping.load(Ordering::SeqCst) {
        std::thread::sleep(WATCH);
        let mut flights = shared
            .flights
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let now = Instant::now();
        let idle = flights.idle;
        flights.list.retain(|flight| {
            let charged = flight.charged(now, idle);
            if charged < Duration::from_millis(flight.budget_ms) {
                return true;
            }
            if flight
                .state
                .compare_exchange(QUEUED, ANSWERED, Ordering::SeqCst, Ordering::SeqCst)
                .is_err()
            {
                return true;
            }
            let _ = answers.send(waited(flight, charged));
            false
        });
    }
}

impl Shared {
    fn publish(&self, server: &Server<'_>) {
        *self.status.lock().unwrap_or_else(PoisonError::into_inner) = server.status();
    }

    fn landed(&self, cancel: &Arc<AtomicBool>) {
        let mut flights = self.flights.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(at) = flights
            .list
            .iter()
            .position(|flight| Arc::ptr_eq(&flight.cancel, cancel))
        {
            flights.list.swap_remove(at);
        }
    }

    fn excused(&self, cancel: &Arc<AtomicBool>) -> Duration {
        self.flights
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .list
            .iter()
            .find(|flight| Arc::ptr_eq(&flight.cancel, cancel))
            .map_or(Duration::ZERO, |flight| flight.excused)
    }

    fn idling(&self, server: &mut Server<'_>) -> bool {
        self.aside(|| server.idle())
    }

    fn aside<T>(&self, work: impl FnOnce() -> T) -> T {
        self.flights
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .idle = Some(Instant::now());
        let did = work();
        let mut flights = self.flights.lock().unwrap_or_else(PoisonError::into_inner);
        let now = Instant::now();
        if let Some(began) = flights.idle.take() {
            for flight in &mut flights.list {
                let behind = now.saturating_duration_since(began.max(flight.arrived));
                flight.excused = flight.excused.saturating_add(behind);
            }
        }
        did
    }
}

pub struct Exec {
    jobs: Option<Sender<Job>>,
    answers: Sender<Json>,
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
    watcher: Option<JoinHandle<()>>,
    #[cfg(feature = "faults")]
    faults: Arc<Mutex<super::Faults>>,
}

struct Control<'shared> {
    shared: &'shared Shared,
}

impl Served for Control<'_> {
    fn listed(&self) -> Vec<Json> {
        super::listing_all()
    }

    fn called(&mut self, name: &str, arguments: &Json) -> Option<Json> {
        let tool = super::served().find(|tool| tool.name == name && name == "grit_describe")?;
        if let Err(refusal) = super::schema::checked(
            tool.name,
            &super::input(&(tool.shown)()),
            arguments,
            Check::Whole,
        ) {
            return Some(super::world_refusal(refusal, arguments, Vec::new()));
        }
        let mut status = self
            .shared
            .status
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Json::Map(fields) = &mut status.submissions {
            for (name, held) in fields.iter_mut() {
                if name == "batches" {
                    *held = super::budget::counted64(self.shared.batches.load(Ordering::SeqCst));
                }
            }
        }
        Some(super::delivery(super::tools::describe(&status)))
    }
}

fn usable(id: &Json) -> bool {
    match id {
        Json::Null | Json::Int(_) => true,
        Json::Text(held) => held.len() <= transport::ID_MOST,
        _ => false,
    }
}

fn routed(request: &Json) -> Option<Json> {
    if request.get("method").and_then(Json::text) != Some("tools/call") {
        return None;
    }
    let id = request.get("id").filter(|id| usable(id))?;
    let name = request
        .get("params")
        .filter(|params| matches!(params, Json::Map(_)))
        .and_then(|params| params.get("name"))
        .and_then(Json::text)?;
    (name != "grit_describe").then(|| id.clone())
}

fn patched(answer: Json, restored: &[Json]) -> Json {
    let Json::Map(envelope) = answer else {
        return answer;
    };
    Json::Map(
        envelope
            .into_iter()
            .map(|(key, one)| {
                if key != "result" {
                    return (key, one);
                }
                let Json::Map(result) = one else {
                    return (key, one);
                };
                let failed = result
                    .iter()
                    .any(|(name, held)| name == "isError" && *held == Json::Bool(true));
                let mut told = result
                    .iter()
                    .find(|(name, _)| name == "structuredContent")
                    .map(|(_, held)| held.clone())
                    .unwrap_or_else(|| map(vec![]));
                if let Json::Map(fields) = &mut told {
                    fields.retain(|(name, _)| name != "restored" && name != "restored_total");
                    fields.push((
                        "restored".to_owned(),
                        Json::List(restored.iter().take(RESTORED_SHOWN).cloned().collect()),
                    ));
                    fields.push((
                        "restored_total".to_owned(),
                        Json::Int(i64::try_from(restored.len()).unwrap_or(i64::MAX)),
                    ));
                }
                let said = if failed {
                    format!(
                        "{} The server opened the device again and rebuilt {} worlds; restored names each.",
                        told.get("message").and_then(Json::text).unwrap_or_default(),
                        restored.len()
                    )
                } else {
                    crate::wire::written(&told)
                };
                let rebuilt = map(vec![
                    (
                        "content",
                        Json::List(vec![map(vec![("type", text("text")), ("text", text(&said))])]),
                    ),
                    ("isError", Json::Bool(failed)),
                    ("structuredContent", told),
                ]);
                (key, rebuilt)
            })
            .collect(),
    )
}

enum Waiting {
    Unrun(Job),
}

fn work(
    mut opener: Opener,
    mut book: Book,
    jobs: &Receiver<Job>,
    answers: &Sender<Json>,
    shared: &Shared,
    ready: &Sender<()>,
) {
    let mut waiting: Option<Waiting> = None;
    loop {
        let host = opener().ok().flatten();
        let (mut server, _) = Server::resumed(host.as_ref(), book);
        shared.publish(&server);
        let _ = ready.send(());
        let mut retried = false;
        if let Some(Waiting::Unrun(job)) = waiting.take() {
            retried = true;
            waiting = Some(Waiting::Unrun(job));
        }
        let mut reopen = false;
        let mut answered = false;
        loop {
            if std::mem::take(&mut answered) && !server.lost() && shared.idling(&mut server) {
                shared.publish(&server);
                if server.lost() {
                    reopen = true;
                    break;
                }
            }
            let job = match waiting.take() {
                Some(Waiting::Unrun(job)) => job,
                None => match jobs.try_recv() {
                    Ok(job) => job,
                    Err(mpsc::TryRecvError::Empty) => {
                        if !server.lost() && shared.idling(&mut server) {
                            shared.publish(&server);
                            if server.lost() {
                                reopen = true;
                                break;
                            }
                            continue;
                        }
                        let waited = if server.writing() {
                            shared.aside(|| jobs.recv_timeout(LANDING))
                        } else {
                            shared.aside(|| {
                                let got = jobs
                                    .recv()
                                    .map_err(|_| mpsc::RecvTimeoutError::Disconnected);
                                #[cfg(feature = "faults")]
                                std::thread::sleep(
                                    server
                                        .faults()
                                        .lock()
                                        .map(|faults| faults.slow_wake)
                                        .unwrap_or_default(),
                                );
                                got
                            })
                        };
                        match waited {
                            Ok(job) => job,
                            Err(mpsc::RecvTimeoutError::Timeout) => continue,
                            Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        }
                    }
                    Err(mpsc::TryRecvError::Disconnected) => break,
                },
            };
            if server.lost() && !retried {
                waiting = Some(Waiting::Unrun(job));
                reopen = true;
                break;
            }
            retried = false;
            if job
                .state
                .compare_exchange(QUEUED, RUNNING, Ordering::SeqCst, Ordering::SeqCst)
                .is_err()
            {
                continue;
            }
            let arrived = job.arrived + shared.excused(&job.cancel);
            server.arriving(Some(arrived), Some(Arc::clone(&job.cancel)));
            let answer = server.handle(&job.request).unwrap_or(Json::Null);
            server.arriving(None, None);
            shared.publish(&server);
            if server.lost() {
                let _ = answers.send(patched(answer, &server.pending()));
                shared.landed(&job.cancel);
                reopen = true;
                break;
            }
            let _ = answers.send(answer);
            shared.landed(&job.cancel);
            #[cfg(feature = "faults")]
            std::thread::sleep(
                server
                    .faults()
                    .lock()
                    .map(|faults| faults.pause_after_answer)
                    .unwrap_or_default(),
            );
            answered = true;
        }
        book = server.into_book();
        drop(host);
        if !reopen {
            book.drain();
            return;
        }
    }
}

impl Exec {
    pub fn spawn(opener: Opener, options: &Options) -> std::io::Result<(Self, Receiver<Json>)> {
        let book = Book::new(options)?;
        let batches = book.batches();
        #[cfg(feature = "faults")]
        let faults = book.faults();
        let (jobs, inbox) = mpsc::channel::<Job>();
        let (answers, outbox) = mpsc::channel::<Json>();
        let (ready, started) = mpsc::channel::<()>();
        let shared = Arc::new(Shared {
            status: Mutex::new(Server::new(None).status()),
            flights: Mutex::new(Flights::default()),
            batches,
            stopping: AtomicBool::new(false),
        });
        let worker = {
            let shared = Arc::clone(&shared);
            let answers = answers.clone();
            std::thread::Builder::new()
                .name("grit-worker".to_owned())
                .spawn(move || work(opener, book, &inbox, &answers, &shared, &ready))?
        };
        let watcher = {
            let shared = Arc::clone(&shared);
            let answers = answers.clone();
            std::thread::Builder::new()
                .name("grit-watch".to_owned())
                .spawn(move || watch(&shared, &answers))?
        };
        let _ = started.recv();
        Ok((
            Self {
                jobs: Some(jobs),
                answers,
                shared,
                worker: Some(worker),
                watcher: Some(watcher),
                #[cfg(feature = "faults")]
                faults,
            },
            outbox,
        ))
    }

    #[cfg(feature = "faults")]
    #[must_use]
    pub fn faults(&self) -> Arc<Mutex<super::Faults>> {
        Arc::clone(&self.faults)
    }

    pub fn heard(&self, line: Line) {
        let answer = match line {
            Line::Long { bytes } => Some(transport::too_long(bytes)),
            Line::Held(bytes) => {
                let held = bytes.len();
                match String::from_utf8(bytes) {
                    Ok(line) => self.read(&line),
                    Err(why) => Some(transport::not_utf8(held, why.utf8_error().valid_up_to())),
                }
            }
        };
        if let Some(answer) = answer {
            let _ = self.answers.send(answer);
        }
    }

    pub fn line(&self, line: &str) {
        if let Some(answer) = self.read(line) {
            let _ = self.answers.send(answer);
        }
    }

    fn read(&self, line: &str) -> Option<Json> {
        let mut control = Control {
            shared: &self.shared,
        };
        let Ok(request) = crate::wire::read(line) else {
            return transport::heard(&mut control, line);
        };
        if request.get("method").and_then(Json::text) == Some("notifications/cancelled")
            && request.get("id").is_none()
        {
            self.cancel(&request);
            return None;
        }
        let Some(id) = routed(&request) else {
            return transport::handle(&mut control, &request);
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let state = Arc::new(AtomicU8::new(QUEUED));
        let arrived = Instant::now();
        {
            let mut flights = self
                .shared
                .flights
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if flights.list.len() >= QUEUE_MOST {
                return Some(transport::failed(
                    id,
                    &Refusal::new(
                        Code::TooMany,
                        "$",
                        format!(
                            "the server holds {QUEUE_MOST} calls that have not been answered; wait for their answers"
                        ),
                    )
                    .limited(Json::Int(i64::try_from(QUEUE_MOST).unwrap_or(i64::MAX))),
                ));
            }
            flights.list.push(Flight {
                id: id.clone(),
                cancel: Arc::clone(&cancel),
                state: Arc::clone(&state),
                arrived,
                budget_ms: budget_of(&request),
                excused: Duration::ZERO,
            });
        }
        let job = Job {
            request,
            id,
            arrived,
            cancel,
            state,
        };
        let refused = self.jobs.as_ref().and_then(|jobs| jobs.send(job).err());
        refused.map(|job| {
            let job = job.0;
            self.shared.landed(&job.cancel);
            transport::failed(
                job.id,
                &Refusal::new(
                    Code::Fault,
                    "$",
                    "the server's worker has stopped and cannot run this call".to_owned(),
                ),
            )
        })
    }

    fn cancel(&self, request: &Json) {
        let Some(asked) = request
            .get("params")
            .and_then(|params| params.get("requestId"))
        else {
            return;
        };
        let flights = self
            .shared
            .flights
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for flight in &flights.list {
            if &flight.id == asked {
                flight.cancel.store(true, Ordering::SeqCst);
            }
        }
    }

    pub fn finish(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        self.jobs = None;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        self.shared.stopping.store(true, Ordering::SeqCst);
        if let Some(watcher) = self.watcher.take() {
            let _ = watcher.join();
        }
    }
}

impl Drop for Exec {
    fn drop(&mut self) {
        self.stop();
    }
}
