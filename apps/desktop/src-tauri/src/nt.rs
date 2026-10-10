//! The NetworkTables page: viewers (NT4 clients of a robot's or a camera's
//! server) and one local NT4 server, so a PhotonVision or HeliOS camera can
//! be tested without a roboRIO. Both run on orion-nt4. A viewer subscribes
//! to everything and sends what arrives to the UI in batches; the server's
//! topics are the user's, created and edited from the page. While it runs,
//! it also answers PhotonVision's time-sync pings (timesync.rs), as a
//! robot program would.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use orion_nt4::{
    Client, ClientConfig, ClientEvent, Properties, Server, ServerConfig, ServerEvent, ServerHandle,
    SubscribeOptions, TopicEvent, TopicOwner, Value,
};
use serde::Serialize;
use tauri::ipc::Channel;
use tokio::task::JoinHandle;

use crate::timesync;

/// How often a viewer sends what arrived (about 20 times a second).
const BATCH: Duration = Duration::from_millis(50);
/// The name Atlas connects to NT servers with (`/nt/<name>`).
const CLIENT_NAME: &str = "atlas";

/// A topic as a viewer sees it.
#[derive(Clone, Debug, Serialize)]
pub struct NtTopic {
    pub name: String,
    #[serde(rename = "type")]
    pub type_name: String,
    pub properties: Properties,
}

/// One value: the server's clock (µs) and the value (`{type, value}`).
#[derive(Clone, Debug, Serialize)]
pub struct NtValue {
    pub name: String,
    pub t_us: i64,
    pub value: Value,
}

/// What a viewer sends the UI.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "kebab-case", tag = "type")]
pub enum NtFrame {
    /// Connected (again), or not: why, and where it is trying.
    Status {
        connected: bool,
        host: String,
        port: u16,
        reason: Option<String>,
    },
    /// Topics that appeared, and ones that are gone.
    Topics {
        announced: Vec<NtTopic>,
        gone: Vec<String>,
    },
    /// Every value since the last batch, oldest first.
    Values { values: Vec<NtValue> },
}

/// `5338` means team 5338's robot (10.53.38.2); anything else is a host.
pub fn resolve_target(target: &str) -> String {
    let target = target.trim();
    match target.parse::<u32>() {
        Ok(team) if (1..=25_599).contains(&team) && !target.contains('.') => {
            format!("10.{}.{}.2", team / 100, team % 100)
        }
        _ => target.to_string(),
    }
}

/// A local server's state for the page.
#[derive(Clone, Debug, Serialize)]
pub struct NtServerInfo {
    pub port: u16,
    /// This computer's addresses the server answers on.
    pub addresses: Vec<IpAddr>,
    pub topics: Vec<NtServerTopic>,
    pub clients: Vec<NtServerClient>,
    pub time_sync: TimeSyncInfo,
}

/// PhotonVision time sync: our responder, and what each camera publishes
/// about it (`/photonvision/.timesync/<host>/...`).
#[derive(Clone, Debug, Serialize)]
pub struct TimeSyncInfo {
    #[serde(flatten)]
    pub responder: timesync::Status,
    pub cameras: Vec<TimeSyncCamera>,
}

/// A camera's own view of its sync (all µs; `None` until it publishes).
#[derive(Clone, Debug, Default, Serialize)]
pub struct TimeSyncCamera {
    pub name: String,
    /// Server time minus the camera's, filtered.
    pub offset_us: Option<i64>,
    /// The last round trip.
    pub rtt2_us: Option<i64>,
    pub pings: Option<i64>,
    pub pongs: Option<i64>,
    /// When it last got a pong, on its own clock.
    pub last_pong_us: Option<i64>,
}

const TIMESYNC_PREFIX: &str = "/photonvision/.timesync/";

/// The cameras' time-sync topics, by host.
fn time_sync_cameras(topics: &[NtServerTopic]) -> Vec<TimeSyncCamera> {
    let mut cameras: Vec<TimeSyncCamera> = Vec::new();
    for topic in topics {
        let Some((host, field)) = topic
            .name
            .strip_prefix(TIMESYNC_PREFIX)
            .and_then(|rest| rest.split_once('/'))
        else {
            continue;
        };
        let number = match &topic.value {
            Some(Value::Int(v)) => Some(*v),
            Some(Value::Double(v)) => Some(*v as i64),
            _ => None,
        };
        let camera = match cameras.iter().position(|c| c.name == host) {
            Some(i) => &mut cameras[i],
            None => {
                cameras.push(TimeSyncCamera {
                    name: host.to_string(),
                    ..TimeSyncCamera::default()
                });
                cameras.last_mut().expect("pushed")
            }
        };
        // PhotonVision builds publish µs (`_us`) or ns (`_ns`); kept in µs.
        let ns = number.map(|v| v / 1000);
        match field {
            "offset_us" => camera.offset_us = number,
            "offset_ns" => camera.offset_us = ns,
            "rtt2_us" => camera.rtt2_us = number,
            "rtt2_ns" => camera.rtt2_us = ns,
            "ping_tx_count" => camera.pings = number,
            "pong_rx_count" => camera.pongs = number,
            "pong_rx_time_us" => camera.last_pong_us = number,
            "pong_rx_time_ns" => camera.last_pong_us = ns,
            _ => {}
        }
    }
    cameras
}

#[derive(Clone, Debug, Serialize)]
pub struct NtServerTopic {
    pub name: String,
    #[serde(rename = "type")]
    pub type_name: String,
    pub value: Option<Value>,
    pub t_us: Option<i64>,
    /// `local` (made here), `client` (published by `publisher`), or
    /// `unpublished` (kept with no publisher).
    pub owner: &'static str,
    pub publisher: Option<String>,
    pub persistent: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct NtServerClient {
    pub id: u64,
    pub name: String,
    pub subscriptions: usize,
    pub publications: usize,
}

/// What the local server tells the page.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "kebab-case", tag = "type")]
pub enum NtServerFrame {
    /// Topics or clients changed: re-read the server's state.
    Changed,
    /// A client wrote a value.
    Wrote {
        client: String,
        name: String,
        value: Value,
    },
    Warning {
        message: String,
    },
}

struct LocalServer {
    _server: Server,
    handle: ServerHandle,
    port: u16,
    bind: SocketAddr,
    watchers: Vec<JoinHandle<()>>,
    timesync: timesync::Responder,
}

#[derive(Default)]
pub struct Nt {
    viewers: Mutex<HashMap<u64, JoinHandle<()>>>,
    next: AtomicU64,
    server: tokio::sync::Mutex<Option<LocalServer>>,
}

impl Nt {
    /// Connects a viewer to `target` (a team number or a host) and sends
    /// what it sees to `frames` until [`disconnect`](Self::disconnect).
    pub fn connect(&self, target: &str, port: Option<u16>, frames: Channel<NtFrame>) -> u64 {
        let host = resolve_target(target);
        let port = port.unwrap_or(orion_nt4::DEFAULT_PORT);
        let mut config = ClientConfig::new(host.clone(), CLIENT_NAME);
        config.port = port;
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let task = tokio::spawn(async move {
            let mut client = Client::start(config);
            let handle = client.handle();
            let options = SubscribeOptions {
                prefix: true,
                ..SubscribeOptions::default()
            };
            let Ok(mut topics) = handle.subscribe(&["/"], options) else {
                return;
            };
            let _ = frames.send(NtFrame::Status {
                connected: false,
                host: host.clone(),
                port,
                reason: Some("connecting".into()),
            });
            let mut announced = Vec::new();
            let mut gone = Vec::new();
            let mut values = Vec::new();
            let mut tick = tokio::time::interval(BATCH);
            loop {
                tokio::select! {
                    Some(event) = client.next_event() => {
                        let status = match event {
                            ClientEvent::Connected => Some((true, None)),
                            ClientEvent::Disconnected { reason } => Some((false, Some(reason))),
                            _ => None,
                        };
                        if let Some((connected, reason)) = status {
                            let _ = frames.send(NtFrame::Status { connected, host: host.clone(), port, reason });
                        }
                    }
                    Some(event) = topics.next() => match event {
                        TopicEvent::Announced(info) => announced.push(NtTopic {
                            name: info.name,
                            type_name: info.type_name,
                            properties: info.properties,
                        }),
                        TopicEvent::Unannounced { name, .. } => gone.push(name),
                        TopicEvent::PropertiesChanged { .. } => {}
                        TopicEvent::Value { name, timestamp_us, value, .. } => {
                            values.push(NtValue { name, t_us: timestamp_us, value });
                        }
                    },
                    _ = tick.tick() => {
                        if !announced.is_empty() || !gone.is_empty() {
                            let frame = NtFrame::Topics {
                                announced: std::mem::take(&mut announced),
                                gone: std::mem::take(&mut gone),
                            };
                            if frames.send(frame).is_err() {
                                return;
                            }
                        }
                        if !values.is_empty() {
                            let frame = NtFrame::Values { values: std::mem::take(&mut values) };
                            if frames.send(frame).is_err() {
                                return;
                            }
                        }
                    }
                }
            }
        });
        self.viewers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(id, task);
        id
    }

    pub fn disconnect(&self, id: u64) {
        if let Some(task) = self
            .viewers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&id)
        {
            task.abort();
        }
    }

    /// Starts the local server on `port` (5810 by default), on every
    /// interface, so cameras on any of this computer's networks reach it.
    /// Topics kept across restarts (`persistent`) live in `persist`.
    pub async fn start_server(
        &self,
        port: Option<u16>,
        persist: Option<std::path::PathBuf>,
    ) -> Result<NtServerInfo, String> {
        let mut slot = self.server.lock().await;
        if slot.is_none() {
            let bind = SocketAddr::from(([0, 0, 0, 0], port.unwrap_or(orion_nt4::DEFAULT_PORT)));
            let server = Server::start(ServerConfig {
                bind,
                persist_path: persist,
            })
            .await
            .map_err(|error| format!("couldn't start a NetworkTables server on {bind}: {error}"))?;
            let handle = server.handle();
            let port = server.local_addr().port();
            // PhotonVision pings 5810 whatever the NT port; a test server on
            // port 0 gets a free one too.
            let sync_port = if bind.port() == 0 { 0 } else { timesync::PORT };
            let timesync =
                timesync::Responder::start(SocketAddr::from(([0, 0, 0, 0], sync_port))).await;
            *slot = Some(LocalServer {
                _server: server,
                handle,
                port,
                bind,
                watchers: Vec::new(),
                timesync,
            });
        }
        Ok(info(slot.as_ref().expect("started")))
    }

    pub async fn stop_server(&self) {
        if let Some(server) = self.server.lock().await.take() {
            for watcher in server.watchers {
                watcher.abort();
            }
        }
    }

    pub async fn server_info(&self) -> Option<NtServerInfo> {
        self.server.lock().await.as_ref().map(info)
    }

    /// Runs `work` on the running server's local API.
    pub async fn with_server<T>(
        &self,
        work: impl FnOnce(&ServerHandle) -> orion_nt4::Result<T>,
    ) -> Result<T, String> {
        let slot = self.server.lock().await;
        let server = slot
            .as_ref()
            .ok_or_else(|| "the local NetworkTables server isn't running".to_string())?;
        work(&server.handle).map_err(|error| error.to_string())
    }

    /// Sends the server's changes to `frames` until it stops.
    pub async fn watch_server(&self, frames: Channel<NtServerFrame>) -> Result<(), String> {
        let mut slot = self.server.lock().await;
        let server = slot
            .as_mut()
            .ok_or_else(|| "the local NetworkTables server isn't running".to_string())?;
        let mut events = server.handle.events();
        let handle = server.handle.clone();
        server.watchers.push(tokio::spawn(async move {
            while let Ok(event) = events.recv().await {
                let frame = match event {
                    ServerEvent::ValueChanged {
                        name,
                        value,
                        client: Some(client),
                        ..
                    } => NtServerFrame::Wrote {
                        client: client_name(&handle, client),
                        name,
                        value,
                    },
                    ServerEvent::ValueChanged { client: None, .. } => continue,
                    ServerEvent::ProtocolWarning { message, .. }
                    | ServerEvent::PersistFailed { message } => NtServerFrame::Warning { message },
                    _ => NtServerFrame::Changed,
                };
                if frames.send(frame).is_err() {
                    return;
                }
            }
        }));
        Ok(())
    }
}

fn client_name(handle: &ServerHandle, id: u64) -> String {
    handle
        .clients()
        .into_iter()
        .find(|client| client.id == id)
        .map_or_else(|| format!("client {id}"), |client| client.name)
}

fn info(server: &LocalServer) -> NtServerInfo {
    let clients = server.handle.clients();
    let name_of = |id: u64| {
        clients
            .iter()
            .find(|client| client.id == id)
            .map(|client| client.name.clone())
    };
    let mut topics: Vec<NtServerTopic> = server
        .handle
        .topics()
        .into_iter()
        .map(|topic| {
            let (owner, publisher) = match topic.owner {
                TopicOwner::Local => ("local", None),
                TopicOwner::Client(id) => ("client", name_of(id)),
                TopicOwner::Unpublished => ("unpublished", None),
            };
            NtServerTopic {
                persistent: topic
                    .properties
                    .get("persistent")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
                name: topic.name,
                type_name: topic.type_name,
                value: topic.value,
                t_us: topic.timestamp_us,
                owner,
                publisher,
            }
        })
        .collect();
    topics.sort_by(|a, b| a.name.cmp(&b.name));
    let time_sync = TimeSyncInfo {
        responder: server.timesync.status(),
        cameras: time_sync_cameras(&topics),
    };
    NtServerInfo {
        time_sync,
        port: server.port,
        addresses: atlas_image_server::listening_addresses(server.bind),
        topics,
        clients: clients
            .into_iter()
            .map(|client| NtServerClient {
                id: client.id,
                name: client.name,
                subscriptions: client.subscriptions,
                publications: client.publications,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tauri::ipc::InvokeResponseBody;

    use super::*;

    /// A channel that keeps what is sent to it, as JSON text.
    fn collecting<T: tauri::ipc::IpcResponse>() -> (Channel<T>, Arc<Mutex<Vec<String>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let into = seen.clone();
        let channel = Channel::new(move |body| {
            if let InvokeResponseBody::Json(json) = body {
                into.lock().unwrap().push(json);
            }
            Ok(())
        });
        (channel, seen)
    }

    async fn eventually(what: &str, done: impl Fn() -> bool) {
        for _ in 0..250 {
            if done() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("{what} didn't happen");
    }

    #[tokio::test]
    async fn a_viewer_sees_the_local_servers_topics_and_values() {
        let nt = Nt::default();
        let started = nt.start_server(Some(0), None).await.unwrap();
        assert!(started.port > 0);
        nt.with_server(|server| server.set_value("/Camera/exposure", Value::Double(20.0)))
            .await
            .unwrap();
        let (server_frames, wrote) = collecting::<NtServerFrame>();
        nt.watch_server(server_frames).await.unwrap();

        let (frames, seen) = collecting::<NtFrame>();
        let id = nt.connect("127.0.0.1", Some(started.port), frames);
        eventually("the viewer connects and sees the topic", || {
            let seen = seen.lock().unwrap();
            seen.iter().any(|f| f.contains(r#""connected":true"#))
                && seen
                    .iter()
                    .any(|f| f.contains(r#""name":"/Camera/exposure","type":"double""#))
                && seen
                    .iter()
                    .any(|f| f.contains(r#""value":{"type":"double","value":20.0}"#))
        })
        .await;
        nt.with_server(|server| server.set_value("/Camera/exposure", Value::Double(33.5)))
            .await
            .unwrap();
        eventually("a new value arrives", || {
            seen.lock().unwrap().iter().any(|f| f.contains("33.5"))
        })
        .await;
        let info = nt.server_info().await.unwrap();
        assert_eq!(
            info.clients.len(),
            1,
            "the viewer is a client: {:?}",
            info.clients
        );
        assert_eq!(info.clients[0].name, CLIENT_NAME);
        assert_eq!(info.topics[0].owner, "local");
        assert!(
            wrote
                .lock()
                .unwrap()
                .iter()
                .any(|f| f.contains(r#""type":"changed""#)),
            "the page hears of the client"
        );
        nt.disconnect(id);
        nt.stop_server().await;
        assert!(nt.server_info().await.is_none());
    }

    #[test]
    fn time_sync_topics_in_nanoseconds_are_kept_in_microseconds() {
        let topic = |field: &str, value: i64| NtServerTopic {
            name: format!("/photonvision/.timesync/pv/{field}"),
            type_name: "int".into(),
            value: Some(Value::Int(value)),
            t_us: None,
            owner: "client",
            publisher: None,
            persistent: false,
        };
        let cameras = time_sync_cameras(&[
            topic("offset_ns", -7_157_847_513_440),
            topic("rtt2_ns", 381_000),
            topic("pong_rx_time_ns", 7_171_590_000_000),
        ]);
        let camera = &cameras[0];
        assert_eq!(
            (camera.offset_us, camera.rtt2_us, camera.last_pong_us),
            (Some(-7_157_847_513), Some(381), Some(7_171_590_000))
        );
    }

    #[tokio::test]
    async fn the_server_answers_time_sync_and_shows_what_cameras_publish() {
        let nt = Nt::default();
        let started = nt.start_server(Some(0), None).await.unwrap();
        assert!(
            started.time_sync.responder.listening,
            "{:?}",
            started.time_sync
        );
        assert!(started.time_sync.cameras.is_empty());
        // What PhotonVision's TimeSyncManager publishes, by host.
        nt.with_server(|server| {
            for (field, value) in [
                ("offset_us", 1234),
                ("rtt2_us", 800),
                ("ping_tx_count", 5),
                ("pong_rx_count", 4),
            ] {
                let name = format!("/photonvision/.timesync/photonvision/{field}");
                server.publish(&name, "int", Properties::new())?;
                server.set_value(&name, Value::Int(value))?;
            }
            Ok(())
        })
        .await
        .unwrap();
        let info = nt.server_info().await.unwrap();
        let camera = &info.time_sync.cameras[0];
        assert_eq!(
            (
                camera.name.as_str(),
                camera.offset_us,
                camera.rtt2_us,
                camera.pings,
                camera.pongs,
                camera.last_pong_us
            ),
            (
                "photonvision",
                Some(1234),
                Some(800),
                Some(5),
                Some(4),
                None
            )
        );
        // A ping to the responder gets a pong.
        let client = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut ping = vec![1u8, 1];
        ping.extend_from_slice(&9u64.to_le_bytes());
        client
            .send_to(&ping, ("127.0.0.1", info.time_sync.responder.port))
            .await
            .unwrap();
        let mut buf = [0u8; 32];
        let (n, _) = tokio::time::timeout(Duration::from_secs(2), client.recv_from(&mut buf))
            .await
            .expect("a pong")
            .unwrap();
        assert_eq!((n, buf[1]), (18, 2));
        nt.stop_server().await;
        // Stopped with the server: no more pongs.
        client
            .send_to(&ping, ("127.0.0.1", info.time_sync.responder.port))
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(300), client.recv_from(&mut buf))
                .await
                .is_err(),
            "no responder without the server"
        );
    }

    #[test]
    fn a_team_number_is_its_robot() {
        assert_eq!(resolve_target("5338"), "10.53.38.2");
        assert_eq!(resolve_target(" 254 "), "10.2.54.2");
        assert_eq!(resolve_target("1"), "10.0.1.2");
        assert_eq!(resolve_target("10.0.0.5"), "10.0.0.5");
        assert_eq!(resolve_target("photonvision.local"), "photonvision.local");
        assert_eq!(resolve_target("99999"), "99999", "not a team number");
    }
}
