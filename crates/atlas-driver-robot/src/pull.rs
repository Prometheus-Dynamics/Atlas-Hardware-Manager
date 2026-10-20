//! What Atlas can pull off the robot's devices without credentials, into a
//! folder: the robot controller's report, its CAN devices and a capture of
//! its console; the radio's status. The roboRIO's log files (`.wpilog` data
//! logs, the console's own log, NI's logs) are behind a login (SSH or
//! WebDAV), which this doesn't use; see docs/robot-network.md.

use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::json;

use crate::console;
use crate::radio::RadioStatus;
use crate::roborio::ControllerReport;

/// What a pull wrote.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Pulled {
    pub files: Vec<PathBuf>,
    /// What couldn't be read, one line each.
    pub missing: Vec<String>,
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap_or_default()
}

async fn write(path: PathBuf, text: String, pulled: &mut Pulled) -> Result<(), String> {
    tokio::fs::write(&path, text)
        .await
        .map_err(|error| format!("{}: {error}", path.display()))?;
    pulled.files.push(path);
    Ok(())
}

/// A fresh report from the robot controller at `ip`, with its CAN devices.
pub async fn controller(ip: Ipv4Addr) -> Result<ControllerReport, String> {
    crate::roborio::read(&client(), ip, true)
        .await
        .map_err(|error| error.to_string())
}

/// The radio's status at `ip` (its WPA key hashes left out).
pub async fn radio(ip: Ipv4Addr) -> Result<RadioStatus, String> {
    crate::radio::read(&client(), ip)
        .await
        .map_err(|error| error.to_string())
}

/// Pulls the robot controller's report (`roborio.json`), CAN devices
/// (`can-devices.json`) and `console_for` of its console (`console.txt`)
/// into `dir`.
pub async fn controller_into(
    ip: Ipv4Addr,
    dir: &Path,
    console_for: Duration,
) -> Result<Pulled, String> {
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|error| format!("{}: {error}", dir.display()))?;
    let mut pulled = Pulled::default();
    let report = controller(ip).await?;
    let pretty =
        |value: &serde_json::Value| serde_json::to_string_pretty(value).unwrap_or_default();
    write(
        dir.join("roborio.json"),
        pretty(&json!({
            "ip": report.ip,
            "team": report.team,
            "hostname": report.hostname(),
            "robot_code": report.robot_code(),
            "services": report.services,
            "phoenix": report.phoenix,
        })),
        &mut pulled,
    )
    .await?;
    match &report.can_devices {
        Some(devices) => {
            write(
                dir.join("can-devices.json"),
                pretty(&json!(devices)),
                &mut pulled,
            )
            .await?;
        }
        None => pulled
            .missing
            .push("CAN devices: Phoenix's diagnostics server (port 1250) didn't answer".into()),
    }
    if console_for > Duration::ZERO {
        let mut text = String::new();
        match console::capture(ip, console_for, |message| {
            text.push_str(&message.to_text());
            text.push('\n');
        })
        .await
        {
            Ok(count) => {
                if count == 0 {
                    pulled.missing.push(format!(
                        "console: nothing printed in {} s",
                        console_for.as_secs()
                    ));
                }
                write(dir.join("console.txt"), text, &mut pulled).await?;
            }
            Err(error) => pulled.missing.push(format!("console: {error}")),
        }
    }
    pulled.missing.push(
        "log files (/home/lvuser, /var/local/natinst/log): need SSH or WebDAV credentials".into(),
    );
    Ok(pulled)
}

/// Pulls the radio's status (`radio-status.json`) into `dir`.
pub async fn radio_into(ip: Ipv4Addr, dir: &Path) -> Result<Pulled, String> {
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|error| format!("{}: {error}", dir.display()))?;
    let mut pulled = Pulled::default();
    let status = radio(ip).await?;
    write(
        dir.join("radio-status.json"),
        serde_json::to_string_pretty(&status).unwrap_or_default(),
        &mut pulled,
    )
    .await?;
    Ok(pulled)
}
