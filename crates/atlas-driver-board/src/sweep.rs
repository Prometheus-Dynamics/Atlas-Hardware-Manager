//! Finding boards without mDNS: an identity request to every address of the
//! robot networks this computer can reach. mDNS doesn't cross a router (a
//! camera's USB route onto the robot network is one) and many laptops don't
//! resolve or browse it at all, so the board driver also sweeps:
//!
//! - this computer's own robot-style networks: an IPv4 `10.x.y.0/24` (or
//!   narrower) it has an address on, as FRC's 10.TE.AM.x robot network is
//!   (the network link's discovery);
//! - the robot networks a board routes its USB link onto (`usb_route`), so
//!   a laptop on one camera's USB finds every device behind it. Those are
//!   the board's children (its gateway link), one sweeper per network: see
//!   `BoardDriver::children`. A route wider than a /24 is narrowed to the
//!   robot networks Atlas can name in it (`atlas_driver::robot_net`).
//!
//! A sweep runs at most once a minute, in the background: discovery hands
//! out what the last one found. Only a network's first sweep is waited for
//! (children, `found_now`). Wider networks than a /24 are not swept (too
//! many addresses to ask).

use std::collections::{BTreeMap, BTreeSet};
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How often a sweep may run.
const EVERY: Duration = Duration::from_secs(60);
/// How long one address may take to answer.
const TIMEOUT: Duration = Duration::from_millis(600);
/// Addresses asked at once.
const AT_ONCE: usize = 48;
/// The port every board serves its identity on (board-http.socket), as the
/// USB gadget's links assume too.
pub(crate) const PORT: u16 = 5899;

/// What a board answering a sweep said it is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Swept {
    pub(crate) serial: String,
    /// The model, lowercase: the family of its driver.
    pub(crate) model: String,
}

/// An IPv4 network to sweep.
pub(crate) use atlas_driver::robot_net::Net;

/// This computer's robot-style networks: its IPv4 addresses in 10.0.0.0/8
/// on a /24 or narrower.
pub(crate) fn own_robot_nets() -> Vec<Net> {
    let Ok(interfaces) = if_addrs::get_if_addrs() else {
        return Vec::new();
    };
    interfaces
        .into_iter()
        .filter_map(|interface| match interface.addr {
            if_addrs::IfAddr::V4(v4) if v4.ip.octets()[0] == 10 && !interface.is_loopback() => {
                let prefix = u8::try_from(u32::from(v4.netmask).count_ones()).ok()?;
                (prefix >= 24).then(|| Net::of(v4.ip, prefix)).flatten()
            }
            _ => None,
        })
        .collect()
}

/// The identity URL of the board at `ip`, as its mDNS advertisement gives it.
#[cfg(test)]
fn identity_url(ip: Ipv4Addr) -> String {
    identity_url_at(ip, PORT)
}

fn identity_url_at(ip: Ipv4Addr, port: u16) -> String {
    format!("http://{ip}:{port}{}", crate::IDENTITY_PATH)
}

#[derive(Default)]
struct State {
    /// The identity URLs the last sweep found answering, with what each
    /// reported.
    found: BTreeMap<String, Swept>,
    last: Option<Instant>,
    running: bool,
}

/// What the sweeps found, and the next one when it's due.
#[derive(Clone)]
pub(crate) struct Sweeper {
    state: Arc<Mutex<State>>,
    port: u16,
}

impl Default for Sweeper {
    fn default() -> Self {
        Self {
            state: Arc::default(),
            port: PORT,
        }
    }
}

impl Sweeper {
    /// The identity URLs found so far; starts a sweep of `nets` in the
    /// background when one is due.
    pub(crate) fn found(&self, http: &reqwest::Client, nets: Vec<Net>) -> Vec<String> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let due = state.last.is_none_or(|last| last.elapsed() >= EVERY);
        if due && !state.running && !nets.is_empty() {
            state.running = true;
            let sweeper = self.clone();
            let http = http.clone();
            tokio::spawn(async move { sweeper.sweep(&http, nets).await });
        }
        state.found.keys().cloned().collect()
    }

    /// Like [`found`](Self::found), with each URL's serial, but the first
    /// sweep is waited for (a few seconds at most: [`TIMEOUT`] per round of
    /// [`AT_ONCE`]) so a one-off scan (`atlas ls`) sees what's there.
    pub(crate) async fn found_now(
        &self,
        http: &reqwest::Client,
        nets: Vec<Net>,
    ) -> BTreeMap<String, Swept> {
        let first = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let first = state.last.is_none() && !state.running && !nets.is_empty();
            if first {
                state.running = true;
            }
            first
        };
        if first {
            self.sweep(http, nets).await;
        } else {
            self.found(http, nets);
        }
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .found
            .clone()
    }

    async fn sweep(&self, http: &reqwest::Client, nets: Vec<Net>) {
        let hosts: Vec<Ipv4Addr> = nets
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .flat_map(Net::hosts)
            .collect();
        let mut found = BTreeMap::new();
        for chunk in hosts.chunks(AT_ONCE) {
            let asks = chunk.iter().map(|ip| {
                let url = identity_url_at(*ip, self.port);
                let http = http.clone();
                async move {
                    let answer = http.get(&url).timeout(TIMEOUT).send().await;
                    let identity = match answer {
                        Ok(response) if response.status().is_success() => {
                            response.json::<serde_json::Value>().await.ok()
                        }
                        _ => None,
                    };
                    // A board's identity: a contract and a model.
                    identity
                        .filter(|value| {
                            value.get("model").is_some() && value.get("contract").is_some()
                        })
                        .map(|value| {
                            let text = |name: &str| {
                                value
                                    .get(name)
                                    .and_then(serde_json::Value::as_str)
                                    .unwrap_or_default()
                                    .to_string()
                            };
                            let swept = Swept {
                                serial: crate::contract::key_serial(&text("serial")),
                                model: text("model").to_ascii_lowercase(),
                            };
                            (url, swept)
                        })
                }
            });
            found.extend(futures_join(asks).await.into_iter().flatten());
        }
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.found = found;
        state.last = Some(Instant::now());
        state.running = false;
    }
}

/// Runs the futures concurrently and collects their results, in order.
async fn futures_join<F, T>(futures: impl IntoIterator<Item = F>) -> Vec<T>
where
    F: std::future::Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let handles: Vec<_> = futures.into_iter().map(tokio::spawn).collect();
    let mut out = Vec::with_capacity(handles.len());
    for handle in handles {
        if let Ok(value) = handle.await {
            out.push(value);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn networks_parse_to_their_base_and_small_ones_list_their_hosts() {
        let net = Net::parse("10.53.38.11/24").unwrap();
        assert_eq!(
            net,
            Net {
                base: Ipv4Addr::new(10, 53, 38, 0),
                prefix: 24
            }
        );
        let hosts = net.hosts();
        assert_eq!(
            (hosts.len(), hosts[0], hosts[253]),
            (
                254,
                Ipv4Addr::new(10, 53, 38, 1),
                Ipv4Addr::new(10, 53, 38, 254)
            )
        );
        assert_eq!(Net::parse("172.31.209.216/29").unwrap().hosts().len(), 6);
        assert!(
            Net::parse("192.168.4.0/22").unwrap().hosts().is_empty(),
            "too wide to sweep"
        );
        assert!(Net::parse("nonsense").is_none());
        assert!(Net::parse("10.0.0.1/33").is_none());
    }

    #[test]
    fn a_swept_url_is_the_same_as_the_advertised_one() {
        assert_eq!(
            identity_url(Ipv4Addr::new(10, 53, 38, 11)),
            "http://10.53.38.11:5899/.well-known/pd-device"
        );
    }

    /// A board answering on the loopback is found; the address next to it,
    /// with nothing listening, isn't.
    #[tokio::test]
    async fn a_sweep_finds_what_answers_with_an_identity() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = [0u8; 1024];
                let _ = socket.read(&mut buf).await;
                let body = r#"{"contract":1,"model":"raze","serial":"x"}"#;
                let reply = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(reply.as_bytes()).await;
            }
        });
        let sweeper = Sweeper {
            state: Arc::default(),
            port,
        };
        let http = reqwest::Client::new();
        sweeper
            .sweep(&http, vec![Net::parse("127.0.0.0/30").unwrap()])
            .await;
        assert_eq!(
            sweeper.found(&http, Vec::new()),
            vec![format!("http://127.0.0.1:{port}{}", crate::IDENTITY_PATH)]
        );
    }
}
