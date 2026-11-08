//! A board's live readings pushed by its own orion-node: board-agent's
//! `board.live` stream (Orion `docs/streams.md`), over a push session on the
//! board's `orion+tcp` listener. Streams stay on the node that keeps them,
//! so this connects to the board itself, not to the node Atlas is set up
//! with; reading needs no enrollment where the board opens it
//! (`ORION_NODE_OPERATOR_OPEN_READ`).
//!
//! The samples are turned into rows of Atlas metrics, a row per
//! [`ROW_MS`] (averaged), so the UI's charts take them as they take polled
//! readings: one backfill of the kept history (a full graph at once), then
//! the rows as they complete.

use std::collections::BTreeMap;
use std::ops::{Deref, DerefMut};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use atlas_driver::{DriverError, HardwareFrame, LiveChannel, LiveDevice, Metric};
use orion_client::remote::{
    NodeTrust, OperatorIdentity, RemoteError, RemoteOperator, RemoteSession,
};
use orion_control_plane::{
    Backfill, ClientEventKind, StatusSubject, StreamCursor, StreamRef, StreamSubscription,
};
use serde::Serialize;

use crate::clock::ClockServer;
use crate::pairing::{OtherKey, PairingLog, Refusals};

/// A board's subscription: its push session, and the clock offers that go
/// with it (they stop when the feed is dropped).
pub(crate) struct Feed {
    session: RemoteSession,
    _clock: Option<ClockServer>,
}

impl Deref for Feed {
    type Target = RemoteSession;

    fn deref(&self) -> &RemoteSession {
        &self.session
    }
}

impl DerefMut for Feed {
    fn deref_mut(&mut self) -> &mut RemoteSession {
        &mut self.session
    }
}

/// Orion's `orion+tcp` port on boards.
pub const DEFAULT_PORT: u16 = 9200;
/// board-agent's /proc and /sys stream.
pub const STREAM: &str = "board.live";
/// board-agent's lemnosd stream.
pub const HARDWARE_STREAM: &str = "board.hardware";
/// How much of a device's readings a hardware view starts with.
const HARDWARE_BACKFILL_MS: u64 = 10_000;
/// One row of readings per this many ms.
pub const ROW_MS: u64 = 500;
/// How much history the first subscription asks for.
const BACKFILL_MS: u64 = 300_000;
/// Reconnecting gives up after this long without the board.
const GIVE_UP: Duration = Duration::from_secs(15);

/// What the UI gets.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LiveFrame {
    /// Rows of readings, oldest first. `backfill`: the history so far,
    /// replacing what the UI had.
    Rows {
        backfill: bool,
        rows: Vec<LiveRow>,
        /// The newest row as metrics (labels, units, ranges).
        latest: Vec<Metric>,
    },
    /// The stream ended and won't come back on its own: why.
    Gone { reason: String },
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LiveRow {
    /// This computer's clock (ms since the Unix epoch).
    pub t: f64,
    pub values: BTreeMap<String, f64>,
}

mod rows;
use rows::{Rows, metrics_of};

/// Connects to boards' orion-nodes for their live streams.
pub struct LiveStreams {
    identity: OperatorIdentity,
    /// A node key per node id, trusted on first use (`<dir>/<node id>`).
    pins: Option<PathBuf>,
    /// The boards (by URL) that refused this computer's key, with the fix.
    refused: Refusals,
}

impl LiveStreams {
    pub fn new(identity: OperatorIdentity, pins: Option<PathBuf>) -> Self {
        Self {
            identity,
            pins,
            refused: Refusals::default(),
        }
    }

    /// Logs a board's refusal of this computer's key (once per board and key).
    pub fn with_pairing_log(mut self, log: Arc<PairingLog>) -> Self {
        self.refused = Refusals::new(Some(log));
        self
    }

    /// The fix for the board at `address`, when it refused this computer's
    /// key the last time Atlas tried it.
    pub fn refusal(&self, address: &str) -> Option<OtherKey> {
        self.refused.get(&Self::url_for(address))
    }

    /// The error for a connection or request to the board at `url` that
    /// failed with `error`; a refused key is recorded and logged.
    fn failed(&self, url: &str, error: &RemoteError) -> DriverError {
        self.refused
            .rejection(url, &self.identity, error)
            .unwrap_or_else(|| crate::protocol::unreachable(url, error))
    }

    /// `orion+tcp://<host>:9200` for an address Atlas reached the board at:
    /// a host, or a URL of the board's (its identity endpoint,
    /// `http://<host>:5899/…`). An `orion+tcp://` URL is taken as it is.
    pub fn url_for(address: &str) -> String {
        if address.starts_with("orion+tcp://") {
            return address.to_owned();
        }
        if let Some(url) = crate::peer_tunnel::orion_url(address) {
            return url;
        }
        let host = atlas_driver::util::address_host(address).unwrap_or(address);
        format!(
            "orion+tcp://{}",
            atlas_driver::util::host_port(host, DEFAULT_PORT)
        )
    }

    /// The board's node at `url`, its key checked against the pin.
    pub(crate) async fn connect(&self, url: &str) -> Result<RemoteOperator, DriverError> {
        let operator = RemoteOperator::connect(url, self.identity.clone(), NodeTrust::FirstUse)
            .await
            .map_err(|error| self.failed(url, &error))?;
        self.refused.accept(url);
        if let Some(dir) = &self.pins {
            let pin = dir.join(operator.node_id().as_str());
            let key = operator.node_public_key();
            match std::fs::read(&pin) {
                Ok(pinned) if pinned != key => {
                    return Err(DriverError::Unreachable(format!(
                        "Orion at {url}: node {} has a new key ({}); not trusted",
                        operator.node_id(),
                        operator.node_fingerprint()
                    )));
                }
                Ok(_) => {}
                Err(_) => {
                    let _ = std::fs::create_dir_all(dir);
                    let _ = std::fs::write(&pin, key);
                }
            }
        }
        Ok(operator)
    }

    /// Follows the board at `address` into `sink` until `sink` returns
    /// false, or the board is gone for [`GIVE_UP`] (then a `Gone` frame).
    /// `Err` when the first connection or subscription fails: the board
    /// has no stream, and the caller polls instead.
    pub async fn follow(
        &self,
        address: &str,
        sink: impl Fn(LiveFrame) -> bool + Send,
    ) -> Result<(), DriverError> {
        let url = Self::url_for(address);
        let mut session = self.subscribe(&url, Backfill::LastMs(BACKFILL_MS)).await?;
        let mut rows = Rows::default();
        let mut kinds = Vec::new();
        let mut cursor: Option<StreamCursor> = None;
        // The next backfill is the whole history (the first, or after a
        // reset), not what a resume missed.
        let mut replace = true;
        loop {
            let Some(event) = session.next_event().await else {
                // The connection ended: pick up where it was.
                let since = cursor.map_or(Backfill::LastMs(BACKFILL_MS), Backfill::Since);
                replace = cursor.is_none();
                let started = tokio::time::Instant::now();
                session = loop {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    match self.subscribe(&url, since).await {
                        Ok(session) => break session,
                        Err(error) if started.elapsed() > GIVE_UP => {
                            sink(LiveFrame::Gone {
                                reason: error.to_string(),
                            });
                            return Ok(());
                        }
                        Err(_) => {}
                    }
                };
                continue;
            };
            let frame = match event.event {
                ClientEventKind::StreamCatalog(catalog) => {
                    rows.catalog(&catalog.series);
                    kinds = catalog.series.iter().map(|s| s.kind).collect();
                    None
                }
                ClientEventKind::StreamData(data) => {
                    if !data.backfill
                        && let Some(seen) = cursor
                        && seen.epoch == data.cursor.epoch
                        && data.prev_seq != seen.seq
                    {
                        // Something was lost: ask again from what was seen.
                        replace = false;
                        let _ = session
                            .subscribe_stream(
                                StreamSubscription::all(data.stream.clone())
                                    .with_backfill(Backfill::Since(seen)),
                            )
                            .await;
                        continue;
                    }
                    cursor = Some(data.cursor);
                    let backfill = data.backfill && replace;
                    if data.backfill {
                        replace = false;
                    }
                    rows.add(&data, &kinds);
                    let out = rows.take();
                    (backfill || !out.is_empty()).then(|| {
                        let latest = out
                            .last()
                            .map(|row| metrics_of(&row.values))
                            .unwrap_or_default();
                        LiveFrame::Rows {
                            backfill,
                            rows: out,
                            latest,
                        }
                    })
                }
                ClientEventKind::StreamReset(_) => {
                    replace = true;
                    None
                }
                _ => None,
            };
            if let Some(frame) = frame
                && !sink(frame)
            {
                return Ok(());
            }
        }
    }

    async fn subscribe(&self, url: &str, backfill: Backfill) -> Result<Feed, DriverError> {
        self.subscribe_to(url, STREAM, Vec::new(), backfill).await
    }

    async fn subscribe_to(
        &self,
        url: &str,
        stream: &str,
        series: Vec<String>,
        backfill: Backfill,
    ) -> Result<Feed, DriverError> {
        let operator = self.connect(url).await?;
        let unreachable = |error: RemoteError| self.failed(url, &error);
        let clock = ClockServer::start(&operator);
        let session = operator.open_session("atlas").await.map_err(unreachable)?;
        let stream = StreamRef {
            owner: StatusSubject::Node(operator.node_id().clone()),
            id: stream.into(),
        };
        let mut subscription = StreamSubscription::all(stream).with_backfill(backfill);
        if !series.is_empty() {
            subscription = subscription.with_series(series);
        }
        session
            .subscribe_stream(subscription)
            .await
            .map_err(unreachable)?;
        Ok(Feed {
            session,
            _clock: clock,
        })
    }

    /// Follows board-agent's `board.hardware` stream (lemnosd's devices) for
    /// the devices `wanted` (id, period ms) into `sink`, as the board's own
    /// hardware stream frames them: `Devices` from the catalog, then
    /// `Samples` per device (Unix µs). Subscribing is what makes the board
    /// read a device it reads only while watched. `Err` when the first
    /// connection fails (the caller falls back to the board's stream).
    pub async fn follow_hardware(
        &self,
        address: &str,
        wanted: &[(String, u32)],
        sink: impl Fn(HardwareFrame) -> bool + Send,
    ) -> Result<(), DriverError> {
        let url = Self::url_for(address);
        let patterns: Vec<String> = wanted.iter().map(|(id, _)| format!("{id}.*")).collect();
        let mut session = self
            .subscribe_to(
                &url,
                HARDWARE_STREAM,
                patterns.clone(),
                Backfill::LastMs(HARDWARE_BACKFILL_MS),
            )
            .await?;
        // Per series index: (device index, channel index); per device, its channel count.
        let mut layout: Vec<Option<(usize, usize)>> = Vec::new();
        let mut kinds = Vec::new();
        let mut widths: Vec<usize> = Vec::new();
        let mut cursor: Option<StreamCursor> = None;
        loop {
            let Some(event) = session.next_event().await else {
                let since = cursor.map_or(Backfill::LastMs(HARDWARE_BACKFILL_MS), Backfill::Since);
                let started = tokio::time::Instant::now();
                session = loop {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    match self
                        .subscribe_to(&url, HARDWARE_STREAM, patterns.clone(), since)
                        .await
                    {
                        Ok(session) => break session,
                        Err(error) if started.elapsed() > GIVE_UP => {
                            sink(HardwareFrame::Gone {
                                reason: error.to_string(),
                            });
                            return Ok(());
                        }
                        Err(_) => {}
                    }
                };
                continue;
            };
            match event.event {
                ClientEventKind::StreamCatalog(catalog) => {
                    kinds = catalog.series.iter().map(|s| s.kind).collect();
                    let mut devices = Vec::new();
                    layout = vec![None; catalog.series.len()];
                    widths.clear();
                    for (d, (id, period)) in wanted.iter().enumerate() {
                        let prefix = format!("{id}.");
                        let mut channels = Vec::new();
                        for (i, series) in catalog.series.iter().enumerate() {
                            if let Some(name) = series.name.strip_prefix(&prefix) {
                                layout[i] = Some((d, channels.len()));
                                channels.push(LiveChannel {
                                    name: name.to_owned(),
                                    unit: series.unit.clone().unwrap_or_default(),
                                });
                            }
                        }
                        widths.push(channels.len());
                        devices.push(LiveDevice {
                            id: id.clone(),
                            status: if channels.is_empty() {
                                String::new()
                            } else {
                                "available".into()
                            },
                            period_ms: *period,
                            missing: channels.is_empty(),
                            channels,
                            ..LiveDevice::default()
                        });
                    }
                    if !sink(HardwareFrame::Devices { devices }) {
                        return Ok(());
                    }
                }
                ClientEventKind::StreamData(data) => {
                    if !data.backfill
                        && let Some(seen) = cursor
                        && seen.epoch == data.cursor.epoch
                        && data.prev_seq != seen.seq
                    {
                        let _ = session
                            .subscribe_stream(
                                StreamSubscription::all(data.stream.clone())
                                    .with_series(patterns.clone())
                                    .with_backfill(Backfill::Since(seen)),
                            )
                            .await;
                        continue;
                    }
                    cursor = Some(data.cursor);
                    // Per device: time -> a value per channel.
                    let mut rows: Vec<BTreeMap<u64, Vec<Option<f64>>>> =
                        vec![BTreeMap::new(); widths.len()];
                    for chunk in &data.chunks {
                        let index = usize::from(chunk.series);
                        let (Some(Some((device, channel))), Some(kind)) =
                            (layout.get(index), kinds.get(index))
                        else {
                            continue;
                        };
                        let Ok(samples) = chunk.samples(*kind) else {
                            continue;
                        };
                        for (t_us, value) in samples {
                            let row = rows[*device]
                                .entry(t_us)
                                .or_insert_with(|| vec![None; widths[*device]]);
                            row[*channel] = Some(value);
                        }
                    }
                    for (device, samples) in rows.into_iter().enumerate() {
                        if samples.is_empty() {
                            continue;
                        }
                        if !sink(HardwareFrame::Samples {
                            device: wanted[device].0.clone(),
                            samples: samples.into_iter().collect(),
                        }) {
                            return Ok(());
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests;
