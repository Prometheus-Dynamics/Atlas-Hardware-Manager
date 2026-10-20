//! `atlas robot`: the robot network's own devices, the robot controller
//! (roboRIO) and the radio, read without credentials.

use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use atlas_core::Atlas;
use atlas_driver::robot_net::{CONTROLLER_FAMILY, RADIO_FAMILY};
use atlas_driver_robot::{console, pull};
use clap::Subcommand;
use serde_json::json;

use crate::commands::{CommandResult, resolve};
use crate::output;

#[derive(Subcommand)]
pub(crate) enum RobotCommand {
    /// What the device reports: versions, team, services, web page; the
    /// radio's mode, channel and links.
    Info { device: String },
    /// The CTRE devices on the robot's CAN buses (Phoenix's diagnostics
    /// server, while the robot program runs).
    Can { device: String },
    /// Streams the robot program's console (what RioLog shows). Ctrl-C stops.
    Console {
        device: String,
        /// Stop after this many seconds (0: until Ctrl-C).
        #[arg(long, default_value_t = 0)]
        seconds: u64,
    },
    /// Saves what can be read without a login into a folder: the report,
    /// the CAN devices and a console capture (robot controller), or the
    /// status (radio).
    Logs {
        device: String,
        /// The folder (default: `robot-<device>-<time>` here).
        #[arg(long)]
        out: Option<PathBuf>,
        /// Seconds of console to capture.
        #[arg(long, default_value_t = 10)]
        seconds: u64,
    },
    /// Prints the device's web page address.
    Open { device: String },
}

/// Which robot device a selector means: a device Atlas knows (name,
/// `family:serial`, serial) or an IPv4 address.
enum Target {
    Controller(Ipv4Addr),
    Radio(Ipv4Addr),
}

async fn target(atlas: &Atlas, selector: &str) -> Result<(Target, String), String> {
    if let Ok(ip) = selector.parse::<Ipv4Addr>() {
        if pull::controller(ip).await.is_ok() {
            return Ok((Target::Controller(ip), selector.to_string()));
        }
        if pull::radio(ip).await.is_ok() {
            return Ok((Target::Radio(ip), selector.to_string()));
        }
        return Err(format!("{ip} is neither a roboRIO nor an FRC radio"));
    }
    atlas.scan().await;
    let key = resolve(atlas, selector)?;
    let record = atlas
        .device(&key)
        .ok_or_else(|| format!("{key} is not in the inventory"))?;
    let ip: Ipv4Addr = record
        .identity
        .address
        .parse()
        .map_err(|_| format!("{key} has no IPv4 address ({})", record.identity.address))?;
    match key.family.as_str() {
        CONTROLLER_FAMILY => Ok((Target::Controller(ip), key.to_string())),
        RADIO_FAMILY => Ok((Target::Radio(ip), key.to_string())),
        _ => Err(format!(
            "{key} is not a robot controller or radio; `atlas ls` lists the {CONTROLLER_FAMILY}:… and {RADIO_FAMILY}:… devices"
        )),
    }
}

pub(crate) async fn robot(atlas: &Atlas, command: RobotCommand, as_json: bool) -> CommandResult {
    match command {
        RobotCommand::Info { device } => info(atlas, &device, as_json).await,
        RobotCommand::Can { device } => can(atlas, &device, as_json).await,
        RobotCommand::Console { device, seconds } => {
            let (Target::Controller(ip), _) = target(atlas, &device).await? else {
                return Err("only the robot controller has a console".into());
            };
            let duration = if seconds == 0 {
                Duration::from_secs(365 * 24 * 3600)
            } else {
                Duration::from_secs(seconds)
            };
            let capture = console::capture(ip, duration, |message| {
                if as_json {
                    output::json(&json!(message.to_log_line()));
                } else {
                    output::line(&message.to_text());
                }
            });
            tokio::select! {
                result = capture => { result?; }
                _ = tokio::signal::ctrl_c() => {}
            }
            Ok(ExitCode::SUCCESS)
        }
        RobotCommand::Logs {
            device,
            out,
            seconds,
        } => {
            let (target, name) = target(atlas, &device).await?;
            let dir = out.unwrap_or_else(|| {
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or_default();
                PathBuf::from(format!("robot-{}-{stamp}", name.replace([':', '/'], "-")))
            });
            if !as_json && matches!(target, Target::Controller(_)) && seconds > 0 {
                output::line(&format!("Capturing {seconds} s of the console…"));
            }
            let pulled = match target {
                Target::Controller(ip) => {
                    pull::controller_into(ip, &dir, Duration::from_secs(seconds)).await?
                }
                Target::Radio(ip) => pull::radio_into(ip, &dir).await?,
            };
            if as_json {
                output::json(&json!({ "dir": dir, "pulled": pulled }));
            } else {
                for file in &pulled.files {
                    output::line(&format!("wrote {}", file.display()));
                }
                for missing in &pulled.missing {
                    output::line(&format!("not pulled: {missing}"));
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        RobotCommand::Open { device } => {
            let (target, _) = target(atlas, &device).await?;
            let url = match target {
                Target::Controller(ip) | Target::Radio(ip) => format!("http://{ip}/"),
            };
            if as_json {
                output::json(&json!({ "url": url }));
            } else {
                output::line(&url);
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

async fn info(atlas: &Atlas, device: &str, as_json: bool) -> CommandResult {
    match target(atlas, device).await?.0 {
        Target::Controller(ip) => {
            let report = pull::controller(ip).await?;
            if as_json {
                output::json(&json!({
                    "kind": "robot-controller",
                    "ip": report.ip,
                    "team": report.team,
                    "hostname": report.hostname(),
                    "robot_code": report.robot_code(),
                    "services": report.services,
                    "phoenix": report.phoenix,
                    "can_devices": report.can_devices.as_ref().map(Vec::len),
                    "web_ui": format!("http://{ip}/"),
                }));
                return Ok(ExitCode::SUCCESS);
            }
            let phoenix = report.phoenix.as_ref();
            let rows = [
                (
                    "Model",
                    phoenix.map_or("roboRIO".into(), |p| p.system.clone()),
                ),
                ("Address", ip.to_string()),
                (
                    "Team",
                    report.team.map(|t| t.to_string()).unwrap_or_default(),
                ),
                ("Hostname", report.hostname().unwrap_or_default()),
                (
                    "Robot code",
                    if report.robot_code() {
                        "running"
                    } else {
                        "not running"
                    }
                    .into(),
                ),
                ("Services", report.services.join(", ")),
                (
                    "Phoenix",
                    phoenix.map_or("-".into(), |p| format!("{} ({})", p.version, p.release)),
                ),
                (
                    "CAN devices",
                    report
                        .can_devices
                        .as_ref()
                        .map_or("-".into(), |d| d.len().to_string()),
                ),
                ("Web UI", format!("http://{ip}/")),
            ];
            for (label, value) in rows {
                output::line(&format!("{label:<12} {value}"));
            }
        }
        Target::Radio(ip) => {
            let status = pull::radio(ip).await?;
            if as_json {
                output::json(&json!({ "kind": "radio", "ip": ip, "status": status }));
                return Ok(ExitCode::SUCCESS);
            }
            let band = |band: &Option<atlas_driver_robot::radio::Band>| {
                band.as_ref().map_or("-".to_string(), |b| {
                    if !b.is_enabled {
                        format!("{} (off)", b.ssid)
                    } else if b.is_linked {
                        format!(
                            "{}: linked, {} dBm, SNR {} dB, rx {} / tx {} Mbps, {}",
                            b.ssid,
                            b.signal_dbm,
                            b.signal_noise_ratio,
                            b.rx_rate_mbps,
                            b.tx_rate_mbps,
                            b.connection_quality
                        )
                    } else {
                        format!("{}: not linked", b.ssid)
                    }
                })
            };
            let rows = [
                ("Model", status.model().to_string()),
                ("Firmware", status.firmware().unwrap_or("-").to_string()),
                ("Address", ip.to_string()),
                ("Team", status.team_number.to_string()),
                ("Mode", status.mode_label()),
                ("Status", status.status.to_ascii_lowercase()),
                ("Channel", status.channel.clone()),
                ("2.4 GHz", band(&status.band_24)),
                ("6 GHz", band(&status.band_6)),
                ("6 GHz off", status.six_ghz_bypassed.to_string()),
                ("Web UI", format!("http://{ip}/")),
            ];
            for (label, value) in rows {
                output::line(&format!("{label:<12} {value}"));
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

async fn can(atlas: &Atlas, device: &str, as_json: bool) -> CommandResult {
    let (Target::Controller(ip), _) = target(atlas, device).await? else {
        return Err("CAN devices are listed by the robot controller".into());
    };
    let report = pull::controller(ip).await?;
    let devices = report.can_devices.ok_or_else(|| {
        format!("{ip}: Phoenix's diagnostics server (port 1250) didn't answer; is the robot program running?")
    })?;
    if as_json {
        output::json(&json!({ "devices": devices }));
        return Ok(ExitCode::SUCCESS);
    }
    let rows: Vec<Vec<String>> = devices
        .iter()
        .map(|d| {
            vec![
                d.bus().to_string(),
                d.id.to_string(),
                d.model.clone(),
                d.name.trim().to_string(),
                d.firmware.clone(),
                d.status.trim_end_matches('.').to_string(),
            ]
        })
        .collect();
    output::table(&["BUS", "ID", "MODEL", "NAME", "FIRMWARE", "STATUS"], &rows);
    Ok(ExitCode::SUCCESS)
}
