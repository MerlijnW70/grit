use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Instant;

use crate::host::{self, Host};
use crate::wire::{Json, map, text};

use super::errors::{Code, Refusal};
use super::schema::Check;
use super::transport::{self, Line, Served};
use super::{Book, Options, Server, Status};

pub const QUEUE_MOST: usize = 256;

pub type Opener = Box<dyn FnMut() -> Result<Option<Host>, host::Error> + Send>;

struct Job {
    request: Json,
    id: Json,
    arrived: Instant,
    cancel: Arc<AtomicBool>,
}

struct Shared {
    status: Mutex<Status>,
    flights: Mutex<Vec<(Json, Arc<AtomicBool>)>>,
    batches: Arc<AtomicU64>,
}

impl Shared {
    fn publish(&self, server: &Server<'_>) {
        *self.status.lock().unwrap_or_else(PoisonError::into_inner) = server.status();
    }

    fn landed(&self, cancel: &Arc<AtomicBool>) {
        let mut flights = self.flights.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(at) = flights
            .iter()
            .position(|(_, flag)| Arc::ptr_eq(flag, cancel))
        {
            flights.swap_remove(at);
        }
    }
}

pub struct Exec {
    jobs: Option<Sender<Job>>,
    answers: Sender<Json>,
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
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
                    fields.retain(|(name, _)| name != "restored");
                    fields.push(("restored".to_owned(), Json::List(restored.to_vec())));
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
    Answered(Job, Json),
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
        let (mut server, restored) = Server::resumed(host.as_ref(), book);
        shared.publish(&server);
        let _ = ready.send(());
        let mut retried = false;
        match waiting.take() {
            Some(Waiting::Answered(job, answer)) => {
                let _ = answers.send(patched(answer, &restored));
                shared.landed(&job.cancel);
            }
            Some(Waiting::Unrun(job)) => {
                retried = true;
                waiting = Some(Waiting::Unrun(job));
            }
            None => {}
        }
        let mut reopen = false;
        loop {
            let job = match waiting.take() {
                Some(Waiting::Unrun(job)) => job,
                Some(answered) => {
                    waiting = Some(answered);
                    break;
                }
                None => match jobs.recv() {
                    Ok(job) => job,
                    Err(_) => break,
                },
            };
            if server.lost() && !retried {
                waiting = Some(Waiting::Unrun(job));
                reopen = true;
                break;
            }
            retried = false;
            server.arriving(job.arrived, Some(Arc::clone(&job.cancel)));
            let answer = server.handle(&job.request).unwrap_or(Json::Null);
            shared.publish(&server);
            if server.lost() {
                waiting = Some(Waiting::Answered(job, answer));
                reopen = true;
                break;
            }
            let _ = answers.send(answer);
            shared.landed(&job.cancel);
        }
        book = server.into_book();
        drop(host);
        if !reopen {
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
            flights: Mutex::new(Vec::new()),
            batches,
        });
        let worker = {
            let shared = Arc::clone(&shared);
            let answers = answers.clone();
            std::thread::Builder::new()
                .name("grit-worker".to_owned())
                .spawn(move || work(opener, book, &inbox, &answers, &shared, &ready))?
        };
        let _ = started.recv();
        Ok((
            Self {
                jobs: Some(jobs),
                answers,
                shared,
                worker: Some(worker),
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
        {
            let mut flights = self
                .shared
                .flights
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if flights.len() >= QUEUE_MOST {
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
            flights.push((id.clone(), Arc::clone(&cancel)));
        }
        let job = Job {
            request,
            id,
            arrived: Instant::now(),
            cancel,
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
        for (id, flag) in flights.iter() {
            if id == asked {
                flag.store(true, Ordering::SeqCst);
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
    }
}

impl Drop for Exec {
    fn drop(&mut self) {
        self.stop();
    }
}
