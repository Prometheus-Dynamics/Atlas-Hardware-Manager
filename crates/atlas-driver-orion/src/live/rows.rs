//! The samples of a live stream as rows of Atlas metrics: one row per
//! [`ROW_MS`], averaged, and the metrics a row carries as the UI names them.

use std::collections::BTreeMap;

use atlas_driver::util::round_to as round;
use atlas_driver::{Metric, metric_ids};
use orion_control_plane::SeriesDecl;

use super::{LiveRow, ROW_MS};

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

/// A row's values as Atlas metrics, as `metrics_from` names them.
pub(super) fn metrics_of(values: &BTreeMap<String, f64>) -> Vec<Metric> {
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
pub(super) struct Rows {
    /// Series index to metric id, from the catalog.
    metrics: Vec<Option<String>>,
    /// Row start (ms on the board's clock) to sums.
    open: BTreeMap<u64, BTreeMap<String, (f64, u32)>>,
    /// The board's clock minus this computer's, ms.
    offset_ms: Option<f64>,
}

impl Rows {
    pub(super) fn add(
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

    pub(super) fn catalog(&mut self, series: &[SeriesDecl]) {
        self.metrics = series.iter().map(|s| metric_for(&s.name)).collect();
        self.open.clear();
    }

    /// The rows complete so far (the newest stays open).
    pub(super) fn take(&mut self) -> Vec<LiveRow> {
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
