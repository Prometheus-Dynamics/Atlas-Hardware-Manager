//! Host commands: readiness checks and disks.

use std::process::ExitCode;

use atlas_core::Atlas;
use atlas_driver::HealthStatus;
use serde_json::json;

use crate::output;

type CommandResult = Result<ExitCode, String>;

pub(crate) async fn doctor(atlas: &Atlas, as_json: bool) -> CommandResult {
    let checks = atlas.health_checks().await;
    let failing = checks
        .iter()
        .any(|check| check.status == HealthStatus::Error);
    if as_json {
        output::json(&json!({ "checks": checks }));
    } else {
        for check in &checks {
            let mark = match check.status {
                HealthStatus::Ok => "ok  ",
                HealthStatus::Warning => "warn",
                HealthStatus::Error => "FAIL",
            };
            output::line(&format!("[{mark}] {}: {}", check.label, check.detail));
            if let Some(fix) = &check.fix {
                output::line(&format!("       fix: {fix}"));
            }
            if let Some(action) = &check.fix_action {
                output::line(&format!("       run: atlas fix {action}"));
            }
        }
    }
    Ok(if failing {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

pub(crate) async fn fix(atlas: &Atlas, action: &str) -> CommandResult {
    let message = atlas
        .fix_health(action)
        .await
        .map_err(|error| error.to_string())?;
    output::line(&message);
    Ok(ExitCode::SUCCESS)
}

pub(crate) async fn disks(as_json: bool) -> CommandResult {
    let disks = tokio::task::spawn_blocking(atlas_blockdev::list_disks)
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;
    if as_json {
        output::json(&json!({ "disks": disks }));
        return Ok(ExitCode::SUCCESS);
    }
    let rows: Vec<Vec<String>> = disks
        .iter()
        .map(|disk| {
            let writable = match atlas_blockdev::check_target(disk, None) {
                Ok(()) => "yes".to_string(),
                Err(reason) => format!("no: {reason}"),
            };
            vec![
                disk.path.display().to_string(),
                disk.description(),
                format!("{:.1} GB", disk.size_bytes as f64 / 1e9),
                if disk.usb { "usb" } else { "internal" }.to_string(),
                disk.mount_points.join(", "),
                writable,
            ]
        })
        .collect();
    output::table(
        &["DISK", "DESCRIPTION", "SIZE", "BUS", "MOUNTED", "WRITABLE"],
        &rows,
    );
    Ok(ExitCode::SUCCESS)
}
