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
//! It listens on 127.0.0.1 only, and answers a call only to a page it served itself (the
//! `Origin` header), so another site open in the same browser cannot drive it.

use std::collections::VecDeque;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use atlas_app::paths::Dirs;
use atlas_app::view::Update;
use atlas_app::{Core, Mode, Options};
use serde_json::{Value, json};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

/// Updates kept for a page that polls.
const KEPT: usize = 512;
const POLL_WAIT: Duration = Duration::from_secs(20);
const WORKERS: usize = 8;

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
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        dist: PathBuf::from("dist"),
        port: 0,
        data: None,
        scene: "demo".into(),
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

    let server = match Server::http(("127.0.0.1", args.port)) {
        Ok(server) => Arc::new(server),
        Err(error) => {
            eprintln!(
                "atlas-ui-bridge: cannot listen on 127.0.0.1:{}: {error}",
                args.port
            );
            std::process::exit(1);
        }
    };
    let port = server
        .server_addr()
        .to_ip()
        .map_or(args.port, |addr| addr.port());
    let origin = format!("http://127.0.0.1:{port}");
    println!("atlas-ui-bridge: listening on {origin}");

    let workers: Vec<_> = (0..WORKERS)
        .map(|_| {
            let server = Arc::clone(&server);
            let feed = Arc::clone(&feed);
            let core = core.clone();
            let handle = runtime.handle().clone();
            let dist = args.dist.clone();
            let origin = origin.clone();
            std::thread::spawn(move || {
                while let Ok(request) = server.recv() {
                    serve(request, &core, &handle, &feed, &dist, &origin);
                }
            })
        })
        .collect();
    for worker in workers {
        let _ = worker.join();
    }
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("a static header is valid")
}

fn json_reply(request: Request, status: u16, body: &Value) {
    let _ = request.respond(
        Response::from_string(body.to_string())
            .with_status_code(StatusCode(status))
            .with_header(header("Content-Type", "application/json"))
            .with_header(header("Cache-Control", "no-store")),
    );
}

fn serve(
    mut request: Request,
    core: &Core,
    runtime: &tokio::runtime::Handle,
    feed: &Feed,
    dist: &Path,
    origin: &str,
) {
    let url = request.url().to_owned();
    let (path, query) = url.split_once('?').unwrap_or((&url, ""));
    match (request.method(), path) {
        (Method::Post, "/call") => {
            let from_here = request
                .headers()
                .iter()
                .find(|h| h.field.equiv("Origin"))
                .is_some_and(|h| h.value.as_str() == origin);
            if !from_here {
                return json_reply(
                    request,
                    403,
                    &json!({ "error": "calls are answered only to the page this bridge serves" }),
                );
            }
            let mut body = String::new();
            if request
                .as_reader()
                .take(4 << 20)
                .read_to_string(&mut body)
                .is_err()
            {
                return json_reply(
                    request,
                    400,
                    &json!({ "error": "the call could not be read" }),
                );
            }
            let Ok(asked) = serde_json::from_str::<Value>(&body) else {
                return json_reply(request, 400, &json!({ "error": "the call is not JSON" }));
            };
            let name = asked
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let arguments = asked.get("args").cloned().unwrap_or(Value::Null);
            let mut reply = match runtime.block_on(atlas_app::call::call(core, &name, arguments)) {
                Ok(value) => json!({ "ok": value }),
                Err(error) => json!({ "error": error.0 }),
            };
            if name == "snapshot" {
                // The page follows updates from the number its snapshot was taken at.
                reply["last"] = json!(feed.last());
            }
            json_reply(request, 200, &reply);
        }
        (Method::Get, "/updates") => {
            let after = query
                .split('&')
                .find_map(|pair| pair.strip_prefix("after="))
                .and_then(|value| value.parse().ok())
                .unwrap_or(0);
            json_reply(request, 200, &feed.after(after));
        }
        (Method::Get, _) => serve_file(request, dist, path),
        _ => json_reply(
            request,
            405,
            &json!({ "error": "not a call this bridge answers" }),
        ),
    }
}

fn serve_file(request: Request, dist: &Path, path: &str) {
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
        let _ =
            request.respond(Response::from_string("not found").with_status_code(StatusCode(404)));
        return;
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
    let _ = request.respond(
        Response::from_data(bytes)
            .with_header(header("Content-Type", kind))
            .with_header(header("Cache-Control", "no-store")),
    );
}
