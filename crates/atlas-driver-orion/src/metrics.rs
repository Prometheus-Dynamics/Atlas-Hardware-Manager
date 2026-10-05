//! Live readings from the node's status lane (`node/<id>`, keys `host.*`).

use std::sync::Arc;

use async_trait::async_trait;
use atlas_driver::{DriverError, Identity, Metric, TelemetryCapability, metric_ids};
use orion_control_plane::{NodeRecord, StatusEntry, StatusQuery, StatusSubject, TypedConfigValue};
use orion_core::NodeId;

use crate::transport::OrionTransport;

pub(crate) fn number(value: &TypedConfigValue) -> Option<f64> {
    match value {
        TypedConfigValue::UInt(value) => Some(*value as f64),
        TypedConfigValue::Int(value) => Some(*value as f64),
        _ => None,
    }
}

fn round(value: f64, places: i32) -> f64 {
    let scale = 10f64.powi(places);
    (value * scale).round() / scale
}

/// Turns `host.*` status entries into Atlas metrics. Known keys get the
/// shared ids (temp, memory, uptime), so robot summaries and gauges work.
pub(crate) fn metrics_from(entries: &[StatusEntry], cpu_count: Option<u32>) -> Vec<Metric> {
    let get = |key: &str| {
        entries
            .iter()
            .find(|entry| entry.key == key)
            .and_then(|entry| number(&entry.value))
    };
    let mut metrics = Vec::new();

    // The hottest sensor is "Temperature"; the rest keep their names.
    let mut temps: Vec<(&str, f64)> = entries
        .iter()
        .filter_map(|entry| {
            let sensor = entry.key.strip_prefix("host.temperature.")?;
            Some((sensor, number(&entry.value)? / 1000.0))
        })
        .collect();
    temps.sort_by(|a, b| b.1.total_cmp(&a.1));
    if let Some((_, hottest)) = temps.first() {
        metrics.push(
            Metric::new(
                metric_ids::TEMPERATURE,
                "Temperature",
                round(*hottest, 1),
                Some("°C"),
            )
            .range(85.0, Some(75.0)),
        );
    }

    if let Some(load) = get("host.load1_milli") {
        let cores = f64::from(cpu_count.unwrap_or(4).max(1));
        // Load as a share of the cores, so it reads like CPU use.
        metrics.push(
            Metric::new(
                metric_ids::CPU,
                "CPU load",
                round(load / 10.0 / cores, 0),
                Some("%"),
            )
            .range(100.0, Some(90.0)),
        );
    }
    if let (Some(available), Some(total)) = (
        get("host.memory_available_bytes"),
        get("host.memory_total_bytes"),
    ) && total > 0.0
    {
        metrics.push(
            Metric::new(
                metric_ids::MEMORY,
                "Memory",
                round((1.0 - available / total) * 100.0, 0),
                Some("%"),
            )
            .range(100.0, Some(90.0)),
        );
    }
    if let Some(uptime) = get("host.uptime_seconds") {
        metrics.push(Metric::new(metric_ids::UPTIME, "Uptime", uptime, Some("s")));
    }
    for (sensor, celsius) in temps.iter().skip(1) {
        metrics.push(Metric::new(
            &format!("temp.{sensor}"),
            sensor,
            round(*celsius, 1),
            Some("°C"),
        ));
    }
    for entry in entries {
        if let Some(name) = entry.key.strip_prefix("host.extra.")
            && let Some(value) = number(&entry.value)
        {
            metrics.push(Metric::new(&format!("extra.{name}"), name, value, None));
        }
    }
    metrics
}

pub(crate) struct OrionTelemetry {
    transport: Arc<dyn OrionTransport>,
    node: NodeId,
    cpu_count: Option<u32>,
}

impl OrionTelemetry {
    pub(crate) fn new(
        transport: Arc<dyn OrionTransport>,
        node: NodeId,
        record: &NodeRecord,
    ) -> Self {
        Self {
            transport,
            node,
            cpu_count: record.host.as_ref().and_then(|host| host.cpu_count),
        }
    }
}

#[async_trait]
impl TelemetryCapability for OrionTelemetry {
    async fn read(&self, _device: &Identity) -> Result<Vec<Metric>, DriverError> {
        let entries = self
            .transport
            .status(StatusQuery {
                subject: Some(StatusSubject::Node(self.node.clone())),
                key_prefix: Some("host.".into()),
            })
            .await?;
        Ok(metrics_from(&entries, self.cpu_count))
    }
}
