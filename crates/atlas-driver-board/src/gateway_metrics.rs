//! A board's readings through the controller's gateway: HeliOS's `metrics`
//! (`/v1/peers/<node>/metrics`) under the shared ids, so its Health, the
//! Monitor and robot summaries work as for any board.
//!
//! The answer's `at_ms` is the board's clock, which has no RTC and restarts
//! wrong (12 h and 19 h behind on the bench), so it isn't used: a reading is
//! as old as its arrival, which is when the live store stamps it.

use async_trait::async_trait;
use atlas_driver::util::round_to as round;
use atlas_driver::{DriverError, Identity, Metric, TelemetryCapability, metric_ids};
use serde_json::Value;

use crate::gateway::Gateway;

pub(crate) struct GatewayTelemetry(pub(crate) Gateway);

#[async_trait]
impl TelemetryCapability for GatewayTelemetry {
    async fn read(&self, _device: &Identity) -> Result<Vec<Metric>, DriverError> {
        Ok(readings(&self.0.get("metrics").await?))
    }
}

/// HeliOS's top-level numbers as Atlas's readings; what's missing is left out.
fn readings(answer: &Value) -> Vec<Metric> {
    let number = |key: &str| answer.get(key).and_then(Value::as_f64);
    let mut metrics = Vec::new();
    if let Some(celsius) = number("temperature_c") {
        metrics.push(
            Metric::new(
                metric_ids::TEMPERATURE,
                "Temperature",
                round(celsius, 1),
                Some("°C"),
            )
            .range(85.0, Some(75.0)),
        );
    }
    if let Some(share) = number("cpu") {
        let mut cpu = Metric::new(metric_ids::CPU, "CPU", round(share * 100.0, 0), Some("%"))
            .range(100.0, Some(90.0));
        if let Some(cores) = answer.get("cpu_cores").and_then(Value::as_array) {
            cpu = cpu.detail(format!("{} cores", cores.len()));
        }
        metrics.push(cpu);
    }
    if let (Some(available), Some(total)) = (
        number("memory_available_bytes"),
        number("memory_total_bytes"),
    ) && total > 0.0
    {
        const MIB: f64 = 1024.0 * 1024.0;
        let used = total - available;
        metrics.push(
            Metric::new(
                metric_ids::MEMORY,
                "Memory",
                round(used / total * 100.0, 0),
                Some("%"),
            )
            .range(100.0, Some(90.0))
            .detail(format!("{:.0} of {:.0} MiB", used / MIB, total / MIB)),
        );
    }
    if let Some(uptime) = number("uptime_s") {
        metrics.push(Metric::new(metric_ids::UPTIME, "Uptime", uptime, Some("s")));
    }
    metrics
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_boards_answer_through_the_controller_gives_the_shared_readings() {
        // As Raze #2's HeliOS answered through a SystemCore (2026-11-03).
        let answer = json!({
            "at_ms": 1793717200934_u64, "cpu": 0.023, "cpu_cores": [0.02, 0.025, 0.024, 0.023],
            "memory_total_bytes": 4161486848_u64, "memory_available_bytes": 3918942208_u64,
            "temperature_c": 39.15, "throttled": null, "uptime_s": 21756,
            "metrics": [{ "id": "temperature", "label": "SoC temperature", "value": 39.2 }]
        });
        let metrics = readings(&answer);
        let ids: Vec<&str> = metrics.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["temp", "cpu", "memory", "uptime"]);
        assert_eq!(metrics[1].value, 2.0);
        assert_eq!(metrics[1].detail.as_deref(), Some("4 cores"));
        assert_eq!(metrics[2].value, 6.0);
        assert_eq!(metrics[2].detail.as_deref(), Some("231 of 3969 MiB"));
    }

    #[test]
    fn what_a_board_leaves_out_is_left_out() {
        assert!(readings(&json!({ "at_ms": 1 })).is_empty());
        let no_total = json!({ "memory_available_bytes": 5, "memory_total_bytes": 0 });
        assert!(readings(&no_total).is_empty());
    }
}
