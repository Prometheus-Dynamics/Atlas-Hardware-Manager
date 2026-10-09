//! Serves update images over HTTP for devices to pull.
//!
//! Orion carries only the intent of an update; the board downloads the
//! image from Atlas and checks its size and SHA-256 itself (docs/ota.md).
//! This server offers registered files only, each under its own
//! unguessable token: `GET /images/<token>/<name>`, where the token is 32
//! random bytes in hex and the name must equal the file's name. Unknown
//! tokens get 404 and nothing is listed. Ranges (for resuming), `HEAD`,
//! `Content-Length`, and a strong ETag (the SHA-256) are supported; files
//! are streamed from disk, never read into memory. A registration expires
//! a day after its last request and can be revoked; each allows a few
//! downloads at once.
//!
//! [`ImageServer`] is the Orion driver's [`BundleHost`]: it starts on
//! first use and names the address of this computer that routes to the
//! board.

mod http;
mod registry;
mod route;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use atlas_driver::{Artifact, DriverError, HealthCheck};
use atlas_driver_orion::BundleHost;
use hyper::service::service_fn;
use hyper_util::rt::{TokioIo, TokioTimer};
use serde::Serialize;
use tokio::task::JoinHandle;

pub use registry::Registered;
pub use route::{listening_addresses, local_address_for};

use crate::registry::{Found, Registry};

pub const DEFAULT_PORT: u16 = 7700;

/// Writes one line to the app's log.
pub type Logger = Arc<dyn Fn(&str) + Send + Sync>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageServerConfig {
    /// Where to listen. Boards reach Atlas over the robot network or the
    /// USB gadget, so the default is every IPv4 interface.
    pub bind: SocketAddr,
    /// The host put in URLs when the route to a board can't be told (a
    /// name or address of this computer the boards can reach).
    pub host: Option<String>,
    /// A registration expires this long after its last request.
    pub idle_expiry: Duration,
    /// Downloads of one registration that may run at once.
    pub max_downloads_per_token: usize,
}

impl Default for ImageServerConfig {
    fn default() -> Self {
        Self {
            bind: SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), DEFAULT_PORT),
            host: None,
            idle_expiry: Duration::from_secs(24 * 60 * 60),
            max_downloads_per_token: 4,
        }
    }
}

/// What the server shares with its connections.
pub(crate) struct Shared {
    registry: Mutex<Registry>,
    config: RwLock<ImageServerConfig>,
    logger: RwLock<Option<Logger>>,
}

impl Shared {
    pub(crate) fn log(&self, line: &str) {
        let logger = self
            .logger
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if let Some(logger) = logger {
            logger(line);
        }
    }

    fn config(&self) -> ImageServerConfig {
        self.config
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn registry(&self) -> std::sync::MutexGuard<'_, Registry> {
        self.registry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn lookup(&self, token: &str) -> Option<Found> {
        let expiry = self.config().idle_expiry;
        self.registry().lookup(token, expiry)
    }

    pub(crate) fn max_downloads(&self) -> usize {
        self.config().max_downloads_per_token.max(1)
    }
}

#[derive(Default)]
struct Listener {
    running: Option<(SocketAddr, JoinHandle<()>)>,
    error: Option<String>,
}

struct Inner {
    shared: Arc<Shared>,
    listener: Mutex<Listener>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        let listener = self
            .listener
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((_, task)) = listener.running.take() {
            task.abort();
        }
    }
}

/// How the server is doing, for the settings screen.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ImageServerStatus {
    pub listening: bool,
    /// The configured listen address, like `0.0.0.0:7700`.
    pub bind: String,
    pub port: u16,
    /// Addresses boards can use, when listening.
    pub addresses: Vec<String>,
    /// The configured fallback host.
    pub host: Option<String>,
    /// Why it couldn't listen, when it couldn't.
    pub error: Option<String>,
    /// Images on offer right now.
    pub registrations: usize,
}

/// The image server. Cheap to clone; the listener stops when the last
/// clone is dropped (downloads in progress finish).
#[derive(Clone)]
pub struct ImageServer {
    inner: Arc<Inner>,
}

impl ImageServer {
    pub fn new(config: ImageServerConfig) -> Self {
        Self {
            inner: Arc::new(Inner {
                shared: Arc::new(Shared {
                    registry: Mutex::new(Registry::default()),
                    config: RwLock::new(config),
                    logger: RwLock::new(None),
                }),
                listener: Mutex::new(Listener::default()),
            }),
        }
    }

    /// Logs every request (token cut short) and finished download.
    pub fn with_logger(self, logger: Logger) -> Self {
        *self
            .inner
            .shared
            .logger
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(logger);
        self
    }

    pub fn config(&self) -> ImageServerConfig {
        self.inner.shared.config()
    }

    fn listener(&self) -> std::sync::MutexGuard<'_, Listener> {
        self.inner
            .listener
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Applies new settings. A running server moves to a changed address;
    /// registrations are kept.
    pub fn reconfigure(&self, config: ImageServerConfig) -> std::io::Result<()> {
        let moved = config.bind != self.config().bind;
        *self
            .inner
            .shared
            .config
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = config;
        let running = self.listener().running.is_some();
        if moved && running {
            self.stop();
            self.start()?;
        }
        Ok(())
    }

    /// Starts listening, if not already, and returns the bound address.
    /// Must be called from inside a Tokio runtime.
    pub fn start(&self) -> std::io::Result<SocketAddr> {
        let mut listener = self.listener();
        if let Some((local, _)) = &listener.running {
            return Ok(*local);
        }
        let bind = self.config().bind;
        let result = (|| {
            let runtime = tokio::runtime::Handle::try_current()
                .map_err(|_| std::io::Error::other("no async runtime to serve from"))?;
            let socket = std::net::TcpListener::bind(bind)?;
            socket.set_nonblocking(true)?;
            let local = socket.local_addr()?;
            let _entered = runtime.enter();
            let socket = tokio::net::TcpListener::from_std(socket)?;
            let task = runtime.spawn(serve(self.inner.shared.clone(), socket));
            Ok((local, task))
        })();
        match result {
            Ok((local, task)) => {
                listener.running = Some((local, task));
                listener.error = None;
                drop(listener);
                self.inner.shared.log(&format!("listening on {local}"));
                Ok(local)
            }
            Err(error) => {
                let message = format!("can't listen on {bind}: {error}");
                listener.error = Some(message.clone());
                drop(listener);
                self.inner.shared.log(&message);
                Err(error)
            }
        }
    }

    /// Stops accepting connections. Downloads in progress finish.
    pub fn stop(&self) {
        if let Some((local, task)) = self.listener().running.take() {
            task.abort();
            self.inner
                .shared
                .log(&format!("stopped listening on {local}"));
        }
    }

    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.listener().running.as_ref().map(|(local, _)| *local)
    }

    /// Offers the file at `path` under a new token. It must be a regular
    /// file of `size` bytes; `sha256` (lowercase hex) is its ETag.
    pub fn register(
        &self,
        path: &Path,
        sha256: &str,
        size: u64,
    ) -> Result<Registered, DriverError> {
        let expiry = self.config().idle_expiry;
        let mut registry = self.inner.shared.registry();
        registry.prune(expiry);
        let registered = registry.register(path, sha256, size)?;
        drop(registry);
        self.inner.shared.log(&format!(
            "offering {} as {}",
            path.display(),
            registered.logged_path()
        ));
        Ok(registered)
    }

    /// Withdraws a registration; its URL answers 404 from now on.
    pub fn revoke(&self, token: &str) -> bool {
        self.inner.shared.registry().revoke(token)
    }

    /// The host for URLs a board at `peer` fetches: the bound address when
    /// bound to one, else the local address of the route to `peer`, else
    /// the configured host.
    pub fn host_for(&self, peer: Option<IpAddr>, local: SocketAddr) -> Result<String, DriverError> {
        if !local.ip().is_unspecified() {
            return Ok(route::url_host(local.ip()));
        }
        if let Some(ip) = peer
            .and_then(local_address_for)
            .filter(|ip| route::serves(local, *ip))
        {
            return Ok(route::url_host(ip));
        }
        self.config()
            .host
            .map(|host| host.trim().to_string())
            .filter(|host| !host.is_empty())
            .ok_or_else(|| {
                DriverError::Other(
                    "It isn't clear which of this computer's addresses the board reaches; set the image host in Settings › Orion".into(),
                )
            })
    }

    /// Starts the server if needed, offers the artifact, and returns the URL
    /// a board at `peer` downloads it from.
    pub fn url_for_peer(
        &self,
        artifact: &Artifact,
        peer: Option<IpAddr>,
    ) -> Result<String, DriverError> {
        let local = self.start().map_err(|error| {
            DriverError::Other(format!(
                "the image server can't listen on {}: {error}",
                self.config().bind
            ))
        })?;
        let host = self.host_for(peer, local)?;
        let registered = self.register(&artifact.path, &artifact.sha256, artifact.size_bytes)?;
        Ok(format!(
            "http://{host}:{}{}",
            local.port(),
            registered.path()
        ))
    }

    pub fn status(&self) -> ImageServerStatus {
        let config = self.config();
        let (local, error) = {
            let listener = self.listener();
            (
                listener.running.as_ref().map(|(local, _)| *local),
                listener.error.clone(),
            )
        };
        let registrations = {
            let mut registry = self.inner.shared.registry();
            registry.prune(config.idle_expiry);
            registry.len()
        };
        ImageServerStatus {
            listening: local.is_some(),
            bind: config.bind.to_string(),
            port: local.map_or(config.bind.port(), |local| local.port()),
            addresses: local
                .map(listening_addresses)
                .unwrap_or_default()
                .into_iter()
                .map(|ip| ip.to_string())
                .collect(),
            host: config.host,
            error,
            registrations,
        }
    }

    /// Whether boards can download from Atlas, for the health screen.
    pub fn health(&self) -> HealthCheck {
        const ID: &str = "orion.image-server";
        const LABEL: &str = "Image server";
        let status = self.status();
        if let Some(error) = &status.error {
            return HealthCheck::error(
                ID,
                LABEL,
                format!("Boards can't download updates: the server {error}."),
                "Choose another image server port in Settings › Orion, or stop what uses this one.",
            );
        }
        if !status.listening {
            return HealthCheck::ok(ID, LABEL, "Starts when an update through Orion needs it.");
        }
        if status.addresses.is_empty() {
            return HealthCheck::warning(
                ID,
                LABEL,
                format!(
                    "Listening on port {}, but this computer has no network address boards can reach.",
                    status.port
                ),
                "Connect to the robot network or plug in a board over USB.",
            );
        }
        HealthCheck::ok(
            ID,
            LABEL,
            format!(
                "Serving update images on TCP port {} at {}. If boards can't download, allow that port through this computer's firewall.",
                status.port,
                status.addresses.join(", ")
            ),
        )
    }
}

impl BundleHost for ImageServer {
    fn url_for(&self, artifact: &Artifact, peer: Option<IpAddr>) -> Result<String, DriverError> {
        self.url_for_peer(artifact, peer)
    }

    fn health(&self) -> Option<HealthCheck> {
        Some(ImageServer::health(self))
    }
}

/// Accepts connections until aborted.
async fn serve(shared: Arc<Shared>, listener: tokio::net::TcpListener) {
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(error) => {
                // Out of file descriptors and the like: back off briefly.
                shared.log(&format!("accept failed: {error}"));
                tokio::time::sleep(Duration::from_millis(200)).await;
                continue;
            }
        };
        let shared = shared.clone();
        tokio::spawn(async move {
            let service = service_fn(move |request| http::handle(shared.clone(), peer, request));
            let mut builder = hyper::server::conn::http1::Builder::new();
            builder
                .timer(TokioTimer::new())
                .header_read_timeout(Duration::from_secs(30));
            let _ = builder
                .serve_connection(TokioIo::new(stream), service)
                .await;
        });
    }
}
