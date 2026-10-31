//! `atlas-usb-agent serve`: the agent as a service on the host the boards
//! are plugged into (installed with its package and run by systemd, with a
//! memory cap), serving the API in the crate's docs.
//!
//! Uploads stream to disk under the state directory, hashed on the way, so
//! memory stays flat whatever the image's size. A flash is a job that runs
//! in the service, not in the request: Atlas's link can drop and come back,
//! and it reads the job's events again from where it left off. Events are
//! also kept in `jobs/<id>.jsonl`, so they outlive a restart of the agent.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use atlas_usb_agent::{
    AgentInfo, BOOT_FILES, Event, FlashRequest, JobEvents, JobStarted, SHA256_HEADER, Stored,
    valid_name, valid_port,
};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::{Method, Request, Response, StatusCode};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::flash::{Flash, Sink};

/// The largest upload taken.
const MAX_UPLOAD: u64 = 16 << 30;
/// Images kept besides the newest; job logs kept.
const KEEP_IMAGES: usize = 2;
const KEEP_JOBS: usize = 10;

pub(crate) struct Options {
    pub listen: SocketAddr,
    pub state: PathBuf,
}

struct Job {
    id: String,
    events: Mutex<Vec<Event>>,
    finished: AtomicBool,
    cancel: Arc<AtomicBool>,
}

struct Agent {
    state: PathBuf,
    /// Newest last.
    jobs: Mutex<VecDeque<Arc<Job>>>,
}

impl Agent {
    fn dir(&self, name: &str) -> PathBuf {
        self.state.join(name)
    }

    fn running(&self) -> Option<Arc<Job>> {
        lock(&self.jobs)
            .iter()
            .find(|job| !job.finished.load(Ordering::Relaxed))
            .cloned()
    }

    fn has_boot_files(&self) -> bool {
        BOOT_FILES
            .iter()
            .all(|name| self.dir("boot").join(name).is_file())
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

type Reply = Response<Full<Bytes>>;

fn json(status: StatusCode, value: &impl serde::Serialize) -> Reply {
    let mut reply = Response::new(Full::new(Bytes::from(
        serde_json::to_vec(value).unwrap_or_default(),
    )));
    *reply.status_mut() = status;
    reply.headers_mut().insert(
        hyper::header::CONTENT_TYPE,
        hyper::header::HeaderValue::from_static("application/json"),
    );
    reply
}

fn error(status: StatusCode, message: impl Into<String>) -> Reply {
    json(status, &serde_json::json!({ "error": message.into() }))
}

pub(crate) async fn serve(options: Options) -> Result<(), String> {
    for dir in ["images", "boot", "jobs"] {
        std::fs::create_dir_all(options.state.join(dir))
            .map_err(|error| format!("{}: {error}", options.state.join(dir).display()))?;
    }
    let listener = tokio::net::TcpListener::bind(options.listen)
        .await
        .map_err(|error| format!("{}: {error}", options.listen))?;
    let agent = Arc::new(Agent {
        state: options.state,
        jobs: Mutex::new(VecDeque::new()),
    });
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            continue;
        };
        let agent = agent.clone();
        tokio::spawn(async move {
            let service = hyper::service::service_fn(move |request| {
                let agent = agent.clone();
                async move { Ok::<_, Infallible>(handle(agent, request).await) }
            });
            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(hyper_util::rt::TokioIo::new(stream), service)
                .await;
        });
    }
}

async fn handle(agent: Arc<Agent>, request: Request<Incoming>) -> Reply {
    let path = request.uri().path().to_string();
    let parts: Vec<&str> = path.trim_matches('/').split('/').collect();
    match (request.method().clone(), parts.as_slice()) {
        (Method::GET, ["v1", "agent"]) => json(
            StatusCode::OK,
            &AgentInfo {
                version: env!("ATLAS_BUILD_VERSION").into(),
                boot_files: agent.has_boot_files(),
                running: agent.running().map(|job| job.id.clone()),
            },
        ),
        (Method::GET, ["v1", "usb"]) => json(StatusCode::OK, &crate::flash::usb_devices()),
        (Method::PUT, ["v1", "boot", name]) if BOOT_FILES.contains(name) => {
            let target = agent.dir("boot").join(name);
            upload(request, &target, None).await
        }
        (Method::PUT, ["v1", "images", name]) if valid_name(name) => {
            let Some(sha256) = request
                .headers()
                .get(SHA256_HEADER)
                .and_then(|value| value.to_str().ok())
                .map(str::to_ascii_lowercase)
            else {
                return error(
                    StatusCode::BAD_REQUEST,
                    format!("{SHA256_HEADER} is required"),
                );
            };
            let target = agent.dir("images").join(name);
            let reply = upload(request, &target, Some(&sha256)).await;
            if reply.status() == StatusCode::OK {
                prune(&agent.dir("images"), KEEP_IMAGES, Some(&target));
            }
            reply
        }
        (Method::POST, ["v1", "flash"]) => start(agent, request).await,
        (Method::GET, ["v1", "jobs", id]) if valid_name(id) => {
            let from = request
                .uri()
                .query()
                .and_then(|query| query.split('&').find_map(|pair| pair.strip_prefix("from=")))
                .and_then(|from| from.parse().ok())
                .unwrap_or(0);
            match job_events(&agent, id, from) {
                Some(events) => json(StatusCode::OK, &events),
                None => error(StatusCode::NOT_FOUND, format!("no job {id}")),
            }
        }
        (Method::DELETE, ["v1", "jobs", id]) => {
            match lock(&agent.jobs).iter().find(|job| job.id == *id) {
                Some(job) => {
                    job.cancel.store(true, Ordering::Relaxed);
                    json(StatusCode::OK, &serde_json::json!({ "cancelling": id }))
                }
                None => error(StatusCode::NOT_FOUND, format!("no job {id}")),
            }
        }
        _ => error(
            StatusCode::NOT_FOUND,
            format!("no {} {path}", request.method()),
        ),
    }
}

/// Streams the request body to `target` (through a `.part` file), checking
/// its SHA-256 when one is given.
async fn upload(request: Request<Incoming>, target: &Path, sha256: Option<&str>) -> Reply {
    match receive(request.into_body(), target, sha256).await {
        Ok(stored) => json(StatusCode::OK, &stored),
        Err(message) => error(StatusCode::BAD_REQUEST, message),
    }
}

async fn receive(
    mut body: Incoming,
    target: &Path,
    sha256: Option<&str>,
) -> Result<Stored, String> {
    let part = target.with_extension("part");
    let mut file = tokio::fs::File::create(&part)
        .await
        .map_err(|error| format!("{}: {error}", part.display()))?;
    let mut hasher = Sha256::new();
    let mut size = 0u64;
    let result = async {
        while let Some(frame) = body.frame().await {
            let frame = frame.map_err(|error| format!("the upload broke off: {error}"))?;
            let Ok(data) = frame.into_data() else {
                continue;
            };
            size += data.len() as u64;
            if size > MAX_UPLOAD {
                return Err(format!("larger than {} GiB", MAX_UPLOAD >> 30));
            }
            hasher.update(&data);
            file.write_all(&data)
                .await
                .map_err(|error| format!("{}: {error}", part.display()))?;
        }
        file.sync_all()
            .await
            .map_err(|error| format!("{}: {error}", part.display()))?;
        let actual = hex(&hasher.finalize());
        if let Some(expected) = sha256
            && actual != expected
        {
            return Err(format!("its SHA-256 is {actual}, not {expected}"));
        }
        Ok(actual)
    }
    .await;
    match result {
        Ok(actual) => {
            tokio::fs::rename(&part, target)
                .await
                .map_err(|error| format!("{}: {error}", target.display()))?;
            Ok(Stored {
                name: target
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                size,
                sha256: actual,
            })
        }
        Err(message) => {
            let _ = tokio::fs::remove_file(&part).await;
            Err(message)
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Keeps the newest `keep` files in `dir` (and `also`), removing the rest.
fn prune(dir: &Path, keep: usize, also: Option<&Path>) {
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((modified, entry.path()))
        })
        .filter(|(_, path)| Some(path.as_path()) != also)
        .collect();
    files.sort();
    let extra = files.len().saturating_sub(keep);
    for (_, path) in files.into_iter().take(extra) {
        let _ = std::fs::remove_file(path);
    }
}

async fn start(agent: Arc<Agent>, request: Request<Incoming>) -> Reply {
    let body = match request.into_body().collect().await {
        Ok(body) => body.to_bytes(),
        Err(_) => return error(StatusCode::BAD_REQUEST, "the request broke off"),
    };
    if body.len() > 1 << 20 {
        return error(StatusCode::BAD_REQUEST, "the request is too large");
    }
    let flash: FlashRequest = match serde_json::from_slice(&body) {
        Ok(flash) => flash,
        Err(problem) => return error(StatusCode::BAD_REQUEST, problem.to_string()),
    };
    if !valid_port(&flash.port) || !valid_name(&flash.image) {
        return error(StatusCode::BAD_REQUEST, "a bad port or image name");
    }
    let image = agent.dir("images").join(&flash.image);
    if !image.is_file() {
        return error(StatusCode::NOT_FOUND, format!("no image {}", flash.image));
    }
    if !agent.has_boot_files() {
        return error(StatusCode::CONFLICT, "the boot files aren't here yet");
    }
    let job = {
        let mut jobs = lock(&agent.jobs);
        if let Some(running) = jobs
            .iter()
            .find(|job| !job.finished.load(Ordering::Relaxed))
        {
            return error(
                StatusCode::CONFLICT,
                format!("job {} is still running", running.id),
            );
        }
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs());
        let job = Arc::new(Job {
            id: format!("{stamp}-{}", flash.port),
            events: Mutex::new(Vec::new()),
            finished: AtomicBool::new(false),
            cancel: Arc::new(AtomicBool::new(false)),
        });
        jobs.push_back(job.clone());
        while jobs.len() > KEEP_JOBS {
            jobs.pop_front();
        }
        job
    };
    prune(&agent.dir("jobs"), KEEP_JOBS, None);
    let log_path = agent.dir("jobs").join(format!("{}.jsonl", job.id));
    let sink: Sink = {
        let job = job.clone();
        Arc::new(move |event: Event| {
            if let Ok(line) = serde_json::to_string(&event)
                && let Ok(mut file) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&log_path)
            {
                let _ = writeln!(file, "{line}");
            }
            lock(&job.events).push(event);
        })
    };
    let run = Flash {
        port: flash.port,
        boot_files: agent.dir("boot"),
        image,
        sha256: flash.sha256,
        boot_writes: flash.boot_writes,
    };
    let running = job.clone();
    tokio::spawn(async move {
        let last = match crate::flash::flash(run, sink.clone(), running.cancel.clone()).await {
            Ok(done) => done,
            Err(message) => Event::Error { message },
        };
        sink(last);
        running.finished.store(true, Ordering::Relaxed);
    });
    json(StatusCode::OK, &JobStarted { id: job.id.clone() })
}

/// A job's events from `from` on: from memory, or its log after a restart
/// of the agent (a job the log doesn't end is one the restart cut short).
fn job_events(agent: &Agent, id: &str, from: usize) -> Option<JobEvents> {
    if let Some(job) = lock(&agent.jobs).iter().find(|job| job.id == id) {
        let events = lock(&job.events);
        return Some(JobEvents {
            id: id.into(),
            events: events.iter().skip(from).cloned().collect(),
            next: events.len().max(from),
            finished: job.finished.load(Ordering::Relaxed),
        });
    }
    let text = std::fs::read_to_string(agent.dir("jobs").join(format!("{id}.jsonl"))).ok()?;
    let mut events: Vec<Event> = text
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    if !events.last().is_some_and(Event::is_last) {
        events.push(Event::Error {
            message: "the agent restarted during this flash; flash the board again".into(),
        });
    }
    Some(JobEvents {
        id: id.into(),
        next: events.len().max(from),
        events: events.into_iter().skip(from).collect(),
        finished: true,
    })
}
