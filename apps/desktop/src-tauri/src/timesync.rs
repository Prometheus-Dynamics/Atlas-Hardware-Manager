//! PhotonVision's time sync, server side, for the local NT server: what a
//! robot program runs (photonlib's `TimeSyncServer`), so a camera pointed at
//! Atlas learns the offset to the server's clock and its frame timestamps
//! line up. It runs only while the local server does.
//!
//! The protocol (photon-targeting `net/TimeSyncServer.cpp`, `TimeSyncStructs.h`):
//! UDP on port 5810 (fixed, whatever the NT port). A ping is 10 bytes,
//! little-endian: version (1), message id (1), the client's time (u64, µs).
//! The pong is 18 bytes: version 1, message id 2, the same client time, and
//! the server's time (u64, µs) on the NT server's clock (`nt::Now()` there,
//! `orion_nt4::now_micros()` here, the clock its value timestamps use).
//! Anything else is dropped. The camera pings once a second and publishes
//! what it learned under `/photonvision/.timesync/<host>/`.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tokio::net::UdpSocket;
use tokio::task::JoinHandle;

/// The port PhotonVision pings (hard-coded in its `TimeSyncManager`).
pub const PORT: u16 = 5810;
const PING_LEN: usize = 10;
const PONG_LEN: usize = 18;
/// At most this many peers are kept (the oldest is dropped).
const MAX_PEERS: usize = 32;

/// The pong for a ping, or `None` for anything that isn't a version-1 ping.
pub fn pong(ping: &[u8], server_time_us: u64) -> Option<[u8; PONG_LEN]> {
    if ping.len() < PING_LEN || ping[0] != 1 || ping[1] != 1 {
        return None;
    }
    let mut out = [0u8; PONG_LEN];
    out[0] = 1;
    out[1] = 2;
    out[2..10].copy_from_slice(&ping[2..10]);
    out[10..18].copy_from_slice(&server_time_us.to_le_bytes());
    Some(out)
}

/// One address that pinged us.
#[derive(Clone, Debug, Serialize)]
pub struct Peer {
    pub address: IpAddr,
    /// Pongs sent to it.
    pub pongs: u64,
    /// When the last ping came, on this computer's clock (ms since the epoch).
    pub last_ms: u64,
}

/// The responder's state for the page.
#[derive(Clone, Debug, Serialize)]
pub struct Status {
    pub port: u16,
    /// Answering pings. False when the port couldn't be bound (`error`).
    pub listening: bool,
    pub error: Option<String>,
    pub peers: Vec<Peer>,
}

type Peers = Arc<Mutex<HashMap<IpAddr, Peer>>>;

/// The running responder; dropping it stops it.
pub struct Responder {
    port: u16,
    task: Option<JoinHandle<()>>,
    error: Option<String>,
    peers: Peers,
}

impl Drop for Responder {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

impl Responder {
    /// Answers pings on `bind` (every interface, port 5810, in the app).
    /// A port that can't be bound (a PhotonVision on this computer, another
    /// tool) is reported in the status, not an error: the NT server runs
    /// anyway.
    pub async fn start(bind: SocketAddr) -> Self {
        let peers: Peers = Arc::default();
        let socket = match UdpSocket::bind(bind).await {
            Ok(socket) => socket,
            Err(error) => {
                return Self {
                    port: bind.port(),
                    task: None,
                    error: Some(format!("couldn't listen on UDP {bind}: {error}")),
                    peers,
                };
            }
        };
        let port = socket.local_addr().map_or(bind.port(), |addr| addr.port());
        let seen = peers.clone();
        let task = tokio::spawn(async move {
            let mut buf = [0u8; 64];
            loop {
                let Ok((n, from)) = socket.recv_from(&mut buf).await else {
                    continue;
                };
                let now = u64::try_from(orion_nt4::now_micros()).unwrap_or(0);
                let Some(reply) = pong(&buf[..n], now) else {
                    continue;
                };
                if socket.send_to(&reply, from).await.is_err() {
                    continue;
                }
                let mut peers = seen.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                if !peers.contains_key(&from.ip())
                    && peers.len() >= MAX_PEERS
                    && let Some(oldest) = peers
                        .values()
                        .min_by_key(|peer| peer.last_ms)
                        .map(|peer| peer.address)
                {
                    peers.remove(&oldest);
                }
                let peer = peers.entry(from.ip()).or_insert(Peer {
                    address: from.ip(),
                    pongs: 0,
                    last_ms: 0,
                });
                peer.pongs += 1;
                peer.last_ms = now_ms();
            }
        });
        Self {
            port,
            task: Some(task),
            error: None,
            peers,
        }
    }

    pub fn status(&self) -> Status {
        let mut peers: Vec<Peer> = self
            .peers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .values()
            .cloned()
            .collect();
        peers.sort_by_key(|peer| peer.address);
        Status {
            port: self.port,
            listening: self.task.is_some(),
            error: self.error.clone(),
            peers,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ping(client_time: u64) -> Vec<u8> {
        let mut p = vec![1u8, 1];
        p.extend_from_slice(&client_time.to_le_bytes());
        p
    }

    #[test]
    fn a_pong_echoes_the_client_time_and_adds_ours() {
        let reply = pong(&ping(0x0102_0304_0506_0708), 42).unwrap();
        assert_eq!(reply[..2], [1, 2]);
        assert_eq!(
            u64::from_le_bytes(reply[2..10].try_into().unwrap()),
            0x0102_0304_0506_0708
        );
        assert_eq!(u64::from_le_bytes(reply[10..18].try_into().unwrap()), 42);
    }

    #[test]
    fn only_version_1_pings_are_answered() {
        assert!(pong(&ping(5)[..9], 1).is_none(), "short");
        let mut wrong_version = ping(5);
        wrong_version[0] = 2;
        assert!(pong(&wrong_version, 1).is_none());
        let mut a_pong = ping(5);
        a_pong[1] = 2;
        assert!(pong(&a_pong, 1).is_none(), "a pong is not a ping");
    }

    #[tokio::test]
    async fn a_camera_gets_a_pong_on_the_servers_clock() {
        let responder = Responder::start(SocketAddr::from(([127, 0, 0, 1], 0))).await;
        let status = responder.status();
        assert!(status.listening, "{status:?}");
        let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let before = u64::try_from(orion_nt4::now_micros()).unwrap();
        client
            .send_to(&ping(7), ("127.0.0.1", status.port))
            .await
            .unwrap();
        let mut buf = [0u8; 64];
        let (n, _) = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            client.recv_from(&mut buf),
        )
        .await
        .expect("a pong")
        .unwrap();
        assert_eq!(n, PONG_LEN);
        assert_eq!(u64::from_le_bytes(buf[2..10].try_into().unwrap()), 7);
        let server_time = u64::from_le_bytes(buf[10..18].try_into().unwrap());
        let after = u64::try_from(orion_nt4::now_micros()).unwrap();
        assert!(
            (before..=after).contains(&server_time),
            "{before} {server_time} {after}"
        );
        let peers = responder.status().peers;
        assert_eq!((peers.len(), peers[0].pongs), (1, 1), "{peers:?}");
    }

    /// The server's time is read for each pong: two pongs 100 ms apart are
    /// about 100 ms apart on the server's clock too.
    #[tokio::test]
    async fn each_pong_reads_the_clock_anew() {
        let responder = Responder::start(SocketAddr::from(([127, 0, 0, 1], 0))).await;
        let port = responder.status().port;
        let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let server_time = || {
            let client = &client;
            async move {
                client.send_to(&ping(1), ("127.0.0.1", port)).await.unwrap();
                let mut buf = [0u8; 64];
                tokio::time::timeout(
                    std::time::Duration::from_secs(2),
                    client.recv_from(&mut buf),
                )
                .await
                .expect("a pong")
                .unwrap();
                u64::from_le_bytes(buf[10..18].try_into().unwrap())
            }
        };
        let first = server_time().await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let second = server_time().await;
        let apart = second.saturating_sub(first);
        assert!(
            (90_000..=200_000).contains(&apart),
            "pongs 100 ms apart are {apart} µs apart on the server's clock"
        );
    }

    #[tokio::test]
    async fn a_port_in_use_is_reported_not_fatal() {
        let taken = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let responder = Responder::start(taken.local_addr().unwrap()).await;
        let status = responder.status();
        assert!(!status.listening);
        assert!(status.error.unwrap().contains("couldn't listen"));
    }
}
