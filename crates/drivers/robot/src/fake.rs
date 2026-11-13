//! A scripted robot network for the tests: what answers, what connects, what
//! names resolve, and what never answers at all. Every request is recorded,
//! so a test can say exactly what this crate asked.

use std::collections::{BTreeMap, BTreeSet};
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;

use crate::reach::{Answer, Reach};

#[derive(Debug, Default)]
struct Script {
    pages: BTreeMap<(Ipv4Addr, u16, String), Answer>,
    open: BTreeSet<(Ipv4Addr, u16)>,
    names: BTreeMap<String, Vec<Ipv4Addr>>,
    hang_all: bool,
    hang: BTreeSet<Ipv4Addr>,
    requests: Vec<String>,
}

/// A fake robot network. Builder methods take and return the fake; clones
/// share one script and one request log.
#[derive(Clone, Debug, Default)]
pub struct Fake {
    script: Arc<Mutex<Script>>,
}

impl Fake {
    pub fn new() -> Self {
        Self::default()
    }

    fn script(&self) -> MutexGuard<'_, Script> {
        self.script.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// `ip:port` answers a GET of `path` with `status`, `server` and `body`.
    pub fn page(
        self,
        ip: Ipv4Addr,
        port: u16,
        path: &str,
        status: u16,
        server: Option<&str>,
        body: &str,
    ) -> Self {
        self.script().pages.insert(
            (ip, port, path.to_string()),
            Answer {
                status,
                server: server.map(str::to_string),
                body: body.to_string(),
            },
        );
        self
    }

    /// `ip:port` accepts a connection.
    pub fn open(self, ip: Ipv4Addr, port: u16) -> Self {
        self.script().open.insert((ip, port));
        self
    }

    /// `name` resolves to `ips`.
    pub fn name(self, name: &str, ips: &[Ipv4Addr]) -> Self {
        self.script().names.insert(name.to_string(), ips.to_vec());
        self
    }

    /// Nothing ever answers, on any address: the network is down.
    pub fn hang(self) -> Self {
        self.script().hang_all = true;
        self
    }

    /// Nothing answers on this one address; the others behave as scripted.
    pub fn hang_for(self, ip: Ipv4Addr) -> Self {
        self.script().hang.insert(ip);
        self
    }

    /// The fake as the network every driver is given.
    pub fn reach(&self) -> Arc<dyn Reach> {
        Arc::new(self.clone())
    }

    /// Every request made, in order: `GET ip:port path`, `CONNECT ip:port`, `RESOLVE name`.
    pub fn requests(&self) -> Vec<String> {
        self.script().requests.clone()
    }

    /// How many connects were made.
    pub fn connects(&self) -> usize {
        self.requests()
            .iter()
            .filter(|request| request.starts_with("CONNECT"))
            .count()
    }

    fn hangs(&self, ip: Option<Ipv4Addr>) -> bool {
        let script = self.script();
        script.hang_all || ip.is_some_and(|ip| script.hang.contains(&ip))
    }
}

#[async_trait]
impl Reach for Fake {
    async fn get(&self, ip: Ipv4Addr, port: u16, path: &str) -> Option<Answer> {
        self.script()
            .requests
            .push(format!("GET {ip}:{port} {path}"));
        if self.hangs(Some(ip)) {
            return std::future::pending().await;
        }
        self.script()
            .pages
            .get(&(ip, port, path.to_string()))
            .cloned()
    }

    async fn open(&self, ip: Ipv4Addr, port: u16) -> bool {
        self.script().requests.push(format!("CONNECT {ip}:{port}"));
        if self.hangs(Some(ip)) {
            return std::future::pending().await;
        }
        self.script().open.contains(&(ip, port))
    }

    async fn resolve(&self, name: &str) -> Vec<Ipv4Addr> {
        self.script().requests.push(format!("RESOLVE {name}"));
        if self.hangs(None) {
            return std::future::pending().await;
        }
        self.script().names.get(name).cloned().unwrap_or_default()
    }
}
