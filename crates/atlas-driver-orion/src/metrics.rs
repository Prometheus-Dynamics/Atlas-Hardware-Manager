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

/// `512 MiB`, `1.2 GiB`.
fn bytes(value: f64) -> String {
    const MIB: f64 = 1024.0 * 1024.0;
    if value >= 1024.0 * MIB {
        format!("{:.1} GiB", value / (1024.0 * MIB))
    } else {
        format!("{:.0} MiB", value / MIB)
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

    let loads: Vec<f64> = ["host.load1_milli", "host.load5_milli", "host.load15_milli"]
        .into_iter()
        .filter_map(|key| get(key).map(|milli| milli / 1000.0))
        .collect();
    let load_detail = (!loads.is_empty()).then(|| {
        let loads: Vec<String> = loads.iter().map(|load| format!("{load:.2}")).collect();
        format!("load {}", loads.join(" · "))
    });
    let mut cores: Vec<(u32, f64)> = entries
        .iter()
        .filter_map(|entry| {
            let index = entry
                .key
                .strip_prefix("host.cpu")?
                .strip_suffix("_busy_milli")?
                .parse()
                .ok()?;
            Some((index, number(&entry.value)? / 10.0))
        })
        .collect();
    cores.sort_by_key(|(index, _)| *index);
    // What the node measured over its last window (/proc/stat); before its
    // first window, the load as a share of the cores.
    let cpu = get("host.cpu_busy_milli")
        .map(|busy| ("CPU", busy / 10.0))
        .or_else(|| {
            get("host.load1_milli").map(|load| {
                let cores = f64::from(cpu_count.unwrap_or(4).max(1));
                ("CPU load", load / 10.0 / cores)
            })
        });
    if let Some((label, percent)) = cpu {
        let mut metric = Metric::new(metric_ids::CPU, label, round(percent, 0), Some("%"))
            .range(100.0, Some(90.0));
        let count = (!cores.is_empty())
            .then_some(cores.len())
            .or(cpu_count.map(|count| count as usize));
        let detail: Vec<String> = count
            .map(|count| format!("{count} cores"))
            .into_iter()
            .chain(load_detail)
            .collect();
        if !detail.is_empty() {
            metric = metric.detail(detail.join(" · "));
        }
        metrics.push(metric);
    }
    for (index, percent) in &cores {
        metrics.push(
            Metric::new(
                &format!("{}{index}", metric_ids::CPU_CORE_PREFIX),
                &format!("Core {index}"),
                round(*percent, 0),
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
        let used = total - available;
        metrics.push(
            Metric::new(
                metric_ids::MEMORY,
                "Memory",
                round(used / total * 100.0, 0),
                Some("%"),
            )
            .range(100.0, Some(90.0))
            .detail(format!("{} of {}", bytes(used), bytes(total))),
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

#[cfg(test)]
mod tests {
    use orion_control_plane::StatusEntry;

    use super::*;

    fn entries(values: &[(&str, u64)]) -> Vec<StatusEntry> {
        values
            .iter()
            .map(|(key, value)| {
                StatusEntry::new(
                    StatusSubject::Node(NodeId::new("n")),
                    *key,
                    TypedConfigValue::UInt(*value),
                )
            })
            .collect()
    }

    fn find<'a>(metrics: &'a [Metric], id: &str) -> &'a Metric {
        metrics.iter().find(|metric| metric.id == id).unwrap()
    }

    #[test]
    fn cpu_is_what_the_node_measured_with_each_core_and_the_load() {
        let metrics = metrics_from(
            &entries(&[
                ("host.cpu_busy_milli", 375),
                ("host.cpu1_busy_milli", 900),
                ("host.cpu0_busy_milli", 120),
                ("host.load1_milli", 420),
                ("host.load5_milli", 380),
                ("host.load15_milli", 300),
                ("host.memory_total_bytes", 4 * 1024 * 1024 * 1024),
                ("host.memory_available_bytes", 3 * 1024 * 1024 * 1024),
            ]),
            Some(4),
        );
        let cpu = find(&metrics, "cpu");
        assert_eq!((cpu.label.as_str(), cpu.value), ("CPU", 38.0));
        assert_eq!(
            cpu.detail.as_deref(),
            Some("2 cores · load 0.42 · 0.38 · 0.30")
        );
        assert_eq!(find(&metrics, "cpu.core.0").value, 12.0);
        assert_eq!(find(&metrics, "cpu.core.1").label, "Core 1");
        let memory = find(&metrics, "memory");
        assert_eq!(memory.value, 25.0);
        assert_eq!(memory.detail.as_deref(), Some("1.0 GiB of 4.0 GiB"));
    }

    #[test]
    fn before_its_first_window_cpu_is_the_load_per_core() {
        let metrics = metrics_from(&entries(&[("host.load1_milli", 2000)]), Some(4));
        let cpu = find(&metrics, "cpu");
        assert_eq!((cpu.label.as_str(), cpu.value), ("CPU load", 50.0));
        assert_eq!(cpu.detail.as_deref(), Some("4 cores · load 2.00"));
        assert!(!metrics.iter().any(|m| m.id.starts_with("cpu.core.")));
    }
}
