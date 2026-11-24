//! The UI bridge: the app's calls and updates, served to a browser on this computer.
//!
//! It exists for the click tests (`scripts/ui-gate.sh`) and for working on the UI in a browser:
//! the page is the real UI, the calls reach the real core, and the devices are the simulated
//! ones, always. It is not an HTTP gateway to a board, and no bundle contains it (it needs the
//! `sim` feature).
//!
//! - `GET /...` serves the built UI from `--dist`.
//! - `POST /call` with `{"name": .., "args": ..}` answers `{"ok": ..}` or `{"error": ".."}`.
//! - `GET /updates?after=N` answers the updates after number N, waiting up to twenty seconds for
//!   one. `{"resync": true}` means N is older than what is kept: read `snapshot` again.
//!
//! With `--request-log PATH` it writes one line when a request is read and one when it has been
//! answered (wall time in ms, the connection, the method and path, and for a call its name), so a
//! request that never ends is seen on the server's side too.
//!
//! Each connection is its own task on the runtime (hyper), and each answer is worked out on a
//! blocking thread: a call blocks on the core, and `/updates` waits up to twenty seconds. There is
//! no fixed pool of connection threads. tiny_http's pool (0.12) could leave a new connection queued
//! with no thread to read it while the browser kept its other connections open, so a page's load
//! never finished (docs/flakes.md, flash-goto-load).
//!
//! It listens on 127.0.0.1 only, and answers a call only to a page it served itself (the
//! `Origin` header), so another site open in the same browser cannot drive it.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use atlas_app::paths::Dirs;
use atlas_app::view::Update;
use atlas_app::{Core, Mode, Options};
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::{Bytes, Incoming};
use hyper::header::{CACHE_CONTROL, CONTENT_TYPE, ORIGIN};
use hyper::{Method, Request, Response, StatusCode};
use serde_json::{Value, json};

/// Updates kept for a page that polls.
const KEPT: usize = 512;
const POLL_WAIT: Duration = Duration::from_secs(20);
/// The largest call body read.
const MAX_CALL: usize = 4 << 20;

#[derive(Default)]
struct Feed {
    state: Mutex<(u64, VecDeque<(u64, Update)>)>,
    changed: Condvar,
}

impl Feed {
    fn push(&self, update: Update) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.0 += 1;
        let number = state.0;
        state.1.push_back((number, update));
        while state.1.len() > KEPT {
            state.1.pop_front();
        }
        self.changed.notify_all();
    }

    fn after(&self, after: u64) -> Value {
        let deadline = Instant::now() + POLL_WAIT;
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            let (last, kept) = &*state;
            let oldest = kept.front().map_or(*last + 1, |(number, _)| *number);
            if after > *last || after + 1 < oldest {
                return json!({ "resync": true, "last": last });
            }
            if after < *last {
                let updates: Vec<&Update> = kept
                    .iter()
                    .filter(|(number, _)| *number > after)
                    .map(|(_, update)| update)
                    .collect();
                return json!({ "last": last, "updates": updates });
            }
            let now = Instant::now();
            if now >= deadline {
                return json!({ "last": last, "updates": [] });
            }
            state = self
                .changed
                .wait_timeout(state, deadline - now)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }

    fn last(&self) -> u64 {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).0
    }
}

struct Args {
    dist: PathBuf,
    port: u16,
    data: Option<PathBuf>,
    scene: String,
    request_log: Option<PathBuf>,
}

/// The request log (`--request-log`): each line is written and flushed as it happens, so a run
/// that is killed keeps every line before it.
struct RequestLog(Mutex<File>);

impl RequestLog {
    fn line(&self, text: &str) {
        let ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_millis());
        let mut file = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let _ = writeln!(file, "{ms} {text}");
        let _ = file.flush();
    }
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        dist: PathBuf::from("dist"),
        port: 0,
        data: None,
        scene: "demo".into(),
        request_log: None,
    };
    let mut words = std::env::args().skip(1);
    while let Some(word) = words.next() {
        let mut value = || words.next().ok_or_else(|| format!("{word} needs a value"));
        match word.as_str() {
            "--dist" => args.dist = PathBuf::from(value()?),
            "--port" => {
                args.port = value()?
                    .parse()
                    .map_err(|_| "--port needs a number".to_string())?
            }
            "--data" => args.data = Some(PathBuf::from(value()?)),
            "--scene" => args.scene = value()?,
            "--request-log" => args.request_log = Some(PathBuf::from(value()?)),
            _ => return Err(format!("unknown argument {word}")),
        }
    }
    Ok(args)
}

fn main() {
    atlas_app::started();
    let args = match parse_args() {
        Ok(args) => args,
        Err(error) => {
            eprintln!(
                "atlas-ui-bridge: {error}\nusage: atlas-ui-bridge [--dist DIR] [--port N] [--data DIR] [--scene demo|empty]"
            );
            std::process::exit(2);
        }
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("atlas-ui-bridge: the runtime cannot start: {error}");
            std::process::exit(1);
        }
    };
    let data = args.data.clone().unwrap_or_else(|| {
        std::env::temp_dir().join(format!("atlas-ui-bridge-{}", std::process::id()))
    });
    let mut options = Options::new(Some(Dirs::under(data)), Mode::Sim);
    options.scene = args.scene.clone();
    let core = runtime.block_on(Core::open(options));

    let feed = Arc::new(Feed::default());
    {
        let feed = Arc::clone(&feed);
        let core = core.clone();
        let mut updates = core.subscribe();
        runtime.spawn(async move {
            loop {
                match updates.recv().await {
                    Ok(update) => feed.push(update),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        for (topic, data) in core.topics() {
                            feed.push(Update { topic, data });
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                }
            }
        });
    }

    let listener = match std::net::TcpListener::bind(("127.0.0.1", args.port)) {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!(
                "atlas-ui-bridge: cannot listen on 127.0.0.1:{}: {error}",
                args.port
            );
            std::process::exit(1);
        }
    };
    let port = listener.local_addr().map_or(args.port, |addr| addr.port());
    let origin = format!("http://127.0.0.1:{port}");
    println!("atlas-ui-bridge: listening on {origin}");

    let log = match &args.request_log {
        Some(path) => match File::create(path) {
            Ok(file) => Some(Arc::new(RequestLog(Mutex::new(file)))),
            Err(error) => {
                eprintln!(
                    "atlas-ui-bridge: cannot write the request log {}: {error}",
                    path.display()
                );
                std::process::exit(1);
            }
        },
        None => None,
    };
    let shared = Arc::new(Shared {
        core,
        handle: runtime.handle().clone(),
        feed,
        dist: args.dist.clone(),
        origin,
        log,
    });
    runtime.block_on(accept(listener, shared));
}

/// What every connection's requests are answered from.
struct Shared {
    core: Core,
    handle: tokio::runtime::Handle,
    feed: Arc<Feed>,
    dist: PathBuf,
    origin: String,
    log: Option<Arc<RequestLog>>,
}

/// Takes connections for as long as the bridge runs: each one is its own task.
async fn accept(listener: std::net::TcpListener, shared: Arc<Shared>) {
    let listener = listener
        .set_nonblocking(true)
        .and_then(|()| tokio::net::TcpListener::from_std(listener));
    let listener = match listener {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("atlas-ui-bridge: cannot take connections: {error}");
            std::process::exit(1);
        }
    };
    let next = AtomicU64::new(0);
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            // A connection that failed before it was taken (reset, out of files): the next one is
            // taken as usual.
            Err(error) => {
                eprintln!("atlas-ui-bridge: a connection could not be taken: {error}");
                tokio::time::sleep(Duration::from_millis(10)).await;
                continue;
            }
        };
        let connection = next.fetch_add(1, Ordering::Relaxed);
        let shared = Arc::clone(&shared);
        tokio::spawn(async move {
            let service = hyper::service::service_fn(move |request| {
                let shared = Arc::clone(&shared);
                async move { Ok::<_, Infallible>(answer(request, connection, shared).await) }
            });
            // A connection the browser drops mid-request ends here; nothing else depends on it.
            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(hyper_util::rt::TokioIo::new(stream), service)
                .await;
        });
    }
}

/// One request: read, logged, and answered on a blocking thread.
async fn answer(
    request: Request<Incoming>,
    connection: u64,
    shared: Arc<Shared>,
) -> Response<Full<Bytes>> {
    let what = format!("c{connection} {} {}", request.method(), request.uri());
    if let Some(log) = &shared.log {
        log.line(&format!("begin {what}"));
    }
    let started = Instant::now();
    let method = request.method().clone();
    let url = request.uri().path_and_query().map_or_else(
        || request.uri().path().to_owned(),
        |pq| pq.as_str().to_owned(),
    );
    let from = request
        .headers()
        .get(ORIGIN)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let body = if method == Method::POST {
        match Limited::new(request.into_body(), MAX_CALL).collect().await {
            Ok(collected) => Some(collected.to_bytes()),
            Err(_) => None,
        }
    } else {
        Some(Bytes::new())
    };
    let worked = Arc::clone(&shared);
    let (reply, call) =
        tokio::task::spawn_blocking(move || serve(&method, &url, from.as_deref(), body, &worked))
            .await
            .unwrap_or_else(|_| {
                (
                    json_reply(
                        500,
                        &json!({ "error": "the bridge failed while answering" }),
                    ),
                    None,
                )
            });
    if let Some(log) = &shared.log {
        let took = started.elapsed().as_millis();
        let call = call.map(|name| format!(" call {name}")).unwrap_or_default();
        log.line(&format!("end {what} {took} ms{call}"));
    }
    reply
}

fn reply(status: u16, kind: &str, body: impl Into<Bytes>) -> Response<Full<Bytes>> {
    let mut response = Response::new(Full::new(body.into()));
    *response.status_mut() =
        StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    if let Ok(value) = kind.parse() {
        response.headers_mut().insert(CONTENT_TYPE, value);
    }
    response.headers_mut().insert(
        CACHE_CONTROL,
        hyper::header::HeaderValue::from_static("no-store"),
    );
    response
}

fn json_reply(status: u16, body: &Value) -> Response<Full<Bytes>> {
    reply(status, "application/json", body.to_string())
}

/// Answers one request. Gives the call's name for a `/call` whose body was read, for the log.
/// `body` is `None` for a call whose body could not be read (or was over [`MAX_CALL`]).
fn serve(
    method: &Method,
    url: &str,
    from: Option<&str>,
    body: Option<Bytes>,
    shared: &Shared,
) -> (Response<Full<Bytes>>, Option<String>) {
    let (path, query) = url.split_once('?').unwrap_or((url, ""));
    match (method, path) {
        (&Method::POST, "/call") => {
            if from != Some(shared.origin.as_str()) {
                return (
                    json_reply(
                        403,
                        &json!({ "error": "calls are answered only to the page this bridge serves" }),
                    ),
                    None,
                );
            }
            let Some(body) = body else {
                return (
                    json_reply(400, &json!({ "error": "the call could not be read" })),
                    None,
                );
            };
            let Ok(asked) = serde_json::from_slice::<Value>(&body) else {
                return (
                    json_reply(400, &json!({ "error": "the call is not JSON" })),
                    None,
                );
            };
            let name = asked
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let arguments = asked.get("args").cloned().unwrap_or(Value::Null);
            let mut answer =
                match shared
                    .handle
                    .block_on(atlas_app::call::call(&shared.core, &name, arguments))
                {
                    Ok(value) => json!({ "ok": value }),
                    Err(error) => json!({ "error": error.0 }),
                };
            if name == "snapshot" {
                // The page follows updates from the number its snapshot was taken at.
                answer["last"] = json!(shared.feed.last());
            }
            (json_reply(200, &answer), Some(name))
        }
        (&Method::GET, "/updates") => {
            let after = query
                .split('&')
                .find_map(|pair| pair.strip_prefix("after="))
                .and_then(|value| value.parse().ok())
                .unwrap_or(0);
            (json_reply(200, &shared.feed.after(after)), None)
        }
        (&Method::GET, _) => (serve_file(&shared.dist, path), None),
        _ => (
            json_reply(405, &json!({ "error": "not a call this bridge answers" })),
            None,
        ),
    }
}

fn serve_file(dist: &Path, path: &str) -> Response<Full<Bytes>> {
    let relative = path.trim_start_matches('/');
    let relative = if relative.is_empty() {
        "index.html"
    } else {
        relative
    };
    let inside = Path::new(relative)
        .components()
        .all(|part| matches!(part, std::path::Component::Normal(_)));
    let file = dist.join(relative);
    let bytes = if inside {
        std::fs::read(&file).ok()
    } else {
        None
    };
    let Some(bytes) = bytes else {
        return reply(404, "text/plain; charset=utf-8", "not found");
    };
    let kind = match file.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("woff2") => "font/woff2",
        Some("json") => "application/json",
        _ => "application/octet-stream",
    };
    reply(200, kind, bytes)
}
