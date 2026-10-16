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
use std::path::PathBuf;
use std::time::Duration;

use atlas_driver::{DriverError, HardwareFrame, LiveChannel, LiveDevice, Metric, metric_ids};
use orion_client::remote::{NodeTrust, OperatorIdentity, RemoteError, RemoteOperator};
use orion_control_plane::{
    Backfill, ClientEventKind, SeriesDecl, StatusSubject, StreamCursor, StreamRef,
    StreamSubscription,
};
use serde::Serialize;

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

fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

/// The Atlas metric a series feeds, if any.
fn metric_for(series: &str) -> Option<String> {
    match series {
        "cpu" => Some(metric_ids::CPU.into()),
        "memory" | "memory.available" | "uptime" | "fan.rpm" => Some(series.into()),
        "fan.pwm" => Some(metric_ids::FAN.into()),
        _ => {
            if let Some(core) = series.strip_prefix("cpu.core") {
                return Some(format!("{}{core}", metric_ids::CPU_CORE_PREFIX));
            }
            series
                .strip_suffix(".temp")
                .map(|zone| format!("temp.{zone}"))
        }
    }
}

fn round(value: f64, places: i32) -> f64 {
    let scale = 10f64.powi(places);
    (value * scale).round() / scale
}

/// A row's values as Atlas metrics, as `metrics_from` names them.
fn metrics_of(values: &BTreeMap<String, f64>) -> Vec<Metric> {
    let mut metrics = Vec::new();
    if let Some(&temp) = values.get(metric_ids::TEMPERATURE) {
        metrics.push(
            Metric::new(
                metric_ids::TEMPERATURE,
                "Temperature",
                round(temp, 1),
                Some("°C"),
            )
            .range(85.0, Some(75.0)),
        );
    }
    let cores = values
        .keys()
        .filter(|id| id.starts_with(metric_ids::CPU_CORE_PREFIX))
        .count();
    if let Some(&cpu) = values.get(metric_ids::CPU) {
        let mut metric =
            Metric::new(metric_ids::CPU, "CPU", round(cpu, 0), Some("%")).range(100.0, Some(90.0));
        if cores > 0 {
            metric = metric.detail(format!("{cores} cores"));
        }
        metrics.push(metric);
    }
    for (id, &value) in values {
        if let Some(core) = id.strip_prefix(metric_ids::CPU_CORE_PREFIX) {
            metrics.push(
                Metric::new(id, &format!("Core {core}"), round(value, 0), Some("%"))
                    .range(100.0, Some(90.0)),
            );
        }
    }
    if let Some(&memory) = values.get(metric_ids::MEMORY) {
        let mut metric = Metric::new(metric_ids::MEMORY, "Memory", round(memory, 0), Some("%"))
            .range(100.0, Some(90.0));
        if let Some(available) = values.get("memory.available") {
            metric = metric.detail(format!("{available:.0} MiB free"));
        }
        metrics.push(metric);
    }
    if let Some(&fan) = values.get(metric_ids::FAN) {
        let mut metric =
            Metric::new(metric_ids::FAN, "Fan", round(fan, 0), Some("%")).range(100.0, None);
        if let Some(rpm) = values.get("fan.rpm") {
            metric = metric.detail(format!("{rpm:.0} rpm"));
        }
        metrics.push(metric);
    }
    if let Some(&uptime) = values.get(metric_ids::UPTIME) {
        metrics.push(Metric::new(
            metric_ids::UPTIME,
            "Uptime",
            uptime.floor(),
            Some("s"),
        ));
    }
    let hottest = values.get(metric_ids::TEMPERATURE).copied();
    for (id, &value) in values {
        if let Some(zone) = id.strip_prefix("temp.")
            && Some(value) != hottest
        {
            metrics.push(Metric::new(id, zone, round(value, 1), Some("°C")));
        }
    }
    metrics
}

/// Samples into rows: sums per row and metric, a row complete once a
/// later one has begun.
#[derive(Default)]
struct Rows {
    /// Series index to metric id, from the catalog.
    metrics: Vec<Option<String>>,
    /// Row start (ms on the board's clock) to sums.
    open: BTreeMap<u64, BTreeMap<String, (f64, u32)>>,
    /// The board's clock minus this computer's, ms.
    offset_ms: Option<f64>,
}

impl Rows {
    fn add(
        &mut self,
        data: &orion_control_plane::StreamData,
        kinds: &[orion_control_plane::SeriesKind],
    ) {
        let mut newest = 0u64;
        for chunk in &data.chunks {
            let index = usize::from(chunk.series);
            let (Some(Some(metric)), Some(kind)) = (self.metrics.get(index), kinds.get(index))
            else {
                continue;
            };
            let Ok(samples) = chunk.samples(*kind) else {
                continue;
            };
            for (t_us, value) in samples {
                let row = t_us / 1000 / ROW_MS * ROW_MS;
                newest = newest.max(t_us / 1000);
                let sum = self
                    .open
                    .entry(row)
                    .or_default()
                    .entry(metric.clone())
                    .or_insert((0.0, 0));
                sum.0 += value;
                sum.1 += 1;
            }
        }
        if newest > 0 {
            // A sample arrives after it was taken, so the clocks' difference
            // is at least what is seen: keep the largest, unless the board's
            // clock was set (a jump).
            let seen = newest as f64 - now_ms();
            self.offset_ms = Some(match self.offset_ms {
                Some(offset) if (seen - offset).abs() < 5_000.0 => offset.max(seen),
                _ => seen,
            });
        }
    }

    fn catalog(&mut self, series: &[SeriesDecl]) {
        self.metrics = series.iter().map(|s| metric_for(&s.name)).collect();
        self.open.clear();
    }

    /// The rows complete so far (the newest stays open).
    fn take(&mut self) -> Vec<LiveRow> {
        let Some(&last) = self.open.keys().next_back() else {
            return Vec::new();
        };
        let keep = self.open.split_off(&last);
        let offset = self.offset_ms.unwrap_or(0.0);
        let done = std::mem::replace(&mut self.open, keep);
        done.into_iter()
            .map(|(row, sums)| {
                let mut values: BTreeMap<String, f64> = sums
                    .into_iter()
                    .map(|(id, (sum, n))| (id, sum / f64::from(n.max(1))))
                    .collect();
                // The hottest zone is "the" temperature.
                let hottest = values
                    .iter()
                    .filter(|(id, _)| id.starts_with("temp."))
                    .map(|(_, v)| *v)
                    .reduce(f64::max);
                if let Some(hottest) = hottest {
                    values.insert(metric_ids::TEMPERATURE.into(), hottest);
                }
                LiveRow {
                    t: (row + ROW_MS) as f64 - offset,
                    values,
                }
            })
            .collect()
    }
}

/// Connects to boards' orion-nodes for their live streams.
pub struct LiveStreams {
    identity: OperatorIdentity,
    /// A node key per node id, trusted on first use (`<dir>/<node id>`).
    pins: Option<PathBuf>,
}

impl LiveStreams {
    pub fn new(identity: OperatorIdentity, pins: Option<PathBuf>) -> Self {
        Self { identity, pins }
    }

    /// `orion+tcp://<address>:9200` for an address Atlas reached the board
    /// at (a URL is taken as it is).
    pub fn url_for(address: &str) -> String {
        if address.contains("://") {
            return address.to_owned();
        }
        if address.contains(':') && !address.starts_with('[') {
            return format!("orion+tcp://[{address}]:{DEFAULT_PORT}");
        }
        format!("orion+tcp://{address}:{DEFAULT_PORT}")
    }

    async fn connect(&self, url: &str) -> Result<RemoteOperator, DriverError> {
        let operator = RemoteOperator::connect(url, self.identity.clone(), NodeTrust::FirstUse)
            .await
            .map_err(|error| DriverError::Unreachable(format!("Orion at {url}: {error}")))?;
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

    async fn subscribe(
        &self,
        url: &str,
        backfill: Backfill,
    ) -> Result<orion_client::remote::RemoteSession, DriverError> {
        self.subscribe_to(url, STREAM, Vec::new(), backfill).await
    }

    async fn subscribe_to(
        &self,
        url: &str,
        stream: &str,
        series: Vec<String>,
        backfill: Backfill,
    ) -> Result<orion_client::remote::RemoteSession, DriverError> {
        let operator = self.connect(url).await?;
        let unreachable =
            |error: RemoteError| DriverError::Unreachable(format!("Orion at {url}: {error}"));
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
        Ok(session)
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
