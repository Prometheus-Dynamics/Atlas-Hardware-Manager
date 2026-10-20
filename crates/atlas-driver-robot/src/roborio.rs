//! The roboRIO, read without credentials.
//!
//! What it serves to anyone (seen on team 6390's roboRIO, FRC 2026 image):
//!
//! - `:80` and `:443`: NI's web configuration page (`Server: Embedthis-http`,
//!   title `HTML Device Configuration`). Its system facts (serial, image,
//!   firmware) come from `POST /nisysapi/server`, a query the page sends on
//!   load; Atlas doesn't send it (reads here are GETs). WebDAV under
//!   `/files/` needs a login (HTTP digest, realm `NIAuth`).
//! - `:1250`: CTRE Phoenix's diagnostics server, inside the robot program
//!   when it uses Phoenix: `GET /?action=getversion` (the server and FRC
//!   season) and `GET /?action=getdevices` (every CTRE device on every CAN
//!   bus: model, name, ID, firmware). Answering means robot code is running.
//! - `:5810` NetworkTables 4 and `:1735` NetworkTables 3: the robot program.
//! - `:1741`: the robot program's console (RioLog), streamed; see `console`.
//! - `:22` SSH (OpenSSH 8.3), `:21` FTP (vsftpd), `:3580` NI service
//!   locator: logins, not used here.
//!
//! Without a serial number to key it by, the robot controller's key is its
//! team number (`roborio:6390`): one robot controller per robot network.

use std::collections::BTreeMap;
use std::net::Ipv4Addr;

use atlas_driver::robot_net::{CONTROLLER_FAMILY, team_of};
use atlas_driver::{DeviceKey, DeviceMode, DriverError, Identity, LinkId, attributes};
use serde::{Deserialize, Serialize};

/// The CTRE Phoenix diagnostics server's port.
pub const PHOENIX_PORT: u16 = 1250;
/// NetworkTables 4's port (the robot program's server).
pub const NT4_PORT: u16 = 5810;
/// NetworkTables 3's port.
pub const NT3_PORT: u16 = 1735;
/// The Driver Station's TCP port.
pub const DS_PORT: u16 = 1740;
/// The console (RioLog) stream's port.
pub const CONSOLE_PORT: u16 = 1741;
/// SSH.
pub const SSH_PORT: u16 = 22;
/// FTP.
pub const FTP_PORT: u16 = 21;

/// The services Atlas looks for on a robot controller, by name.
pub const SERVICES: [(&str, u16); 7] = [
    ("ssh", SSH_PORT),
    ("ftp", FTP_PORT),
    ("nt4", NT4_PORT),
    ("nt3", NT3_PORT),
    ("ds", DS_PORT),
    ("console", CONSOLE_PORT),
    ("phoenix", PHOENIX_PORT),
];

/// Whether the web root is NI's device configuration page, by its `Server`
/// header and title.
pub fn is_ni_web_ui(server: Option<&str>, body: &str) -> bool {
    server.is_some_and(|server| server.starts_with("Embedthis"))
        && body.contains("<title>HTML Device Configuration</title>")
}

/// `GET :1250/?action=getversion`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhoenixVersion {
    /// `roboRIO` (or another host the server runs on).
    #[serde(rename = "System", default)]
    pub system: String,
    /// The FRC season it was built for, `FRC2026`.
    #[serde(rename = "ReleaseInfo", default)]
    pub release: String,
    /// `2026.3.0 (May 22 2026, 21:26:11)`.
    #[serde(rename = "Version", default)]
    pub version: String,
}

impl PhoenixVersion {
    /// The version without its build date: `2026.3.0`.
    pub fn short_version(&self) -> &str {
        self.version
            .split_whitespace()
            .next()
            .unwrap_or(&self.version)
    }

    /// The season's year, `2026` from `FRC2026`.
    pub fn season(&self) -> Option<&str> {
        self.release
            .strip_prefix("FRC")
            .filter(|year| !year.is_empty())
    }
}

/// One CTRE device, as `getdevices` lists it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanDevice {
    #[serde(rename = "Model", default)]
    pub model: String,
    /// The name the team gave it in Phoenix Tuner.
    #[serde(rename = "Name", default)]
    pub name: String,
    #[serde(rename = "ID", default)]
    pub id: u32,
    /// The CANivore's name (`can`), or empty for the roboRIO's own bus.
    #[serde(rename = "CANivoreDevName", default)]
    pub canivore: String,
    #[serde(rename = "CurrentVers", default)]
    pub firmware: String,
    #[serde(rename = "HardwareRev", default)]
    pub hardware: String,
    #[serde(rename = "SerialNo", default)]
    pub serial: String,
    #[serde(rename = "Vendor", default)]
    pub vendor: String,
    #[serde(rename = "SoftStatus", default)]
    pub status: String,
}

impl CanDevice {
    /// The bus it is on: the CANivore's name, or `rio`.
    pub fn bus(&self) -> &str {
        if self.canivore.is_empty() {
            "rio"
        } else {
            &self.canivore
        }
    }
}

#[derive(Deserialize)]
struct DeviceList {
    #[serde(rename = "DeviceArray", default)]
    devices: Vec<CanDevice>,
}

/// `GET :1250/?action=getdevices`: every bus's devices, by bus then ID.
pub fn parse_devices(json: &str) -> Result<Vec<CanDevice>, String> {
    let mut devices = serde_json::from_str::<DeviceList>(json)
        .map_err(|error| error.to_string())?
        .devices;
    devices.sort_by(|a, b| (a.bus(), a.id).cmp(&(b.bus(), b.id)));
    Ok(devices)
}

/// What Atlas read from a robot controller.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ControllerReport {
    pub ip: Option<Ipv4Addr>,
    pub team: Option<u16>,
    /// NI's web configuration page answered.
    pub web_ui: bool,
    /// The Phoenix diagnostics server, when the robot program runs one.
    pub phoenix: Option<PhoenixVersion>,
    /// The services that accept a connection, by name (see [`SERVICES`]).
    pub services: Vec<String>,
    pub can_devices: Option<Vec<CanDevice>>,
}

impl ControllerReport {
    /// Whether the robot program is running: its NetworkTables server or
    /// Phoenix's diagnostics server answers.
    pub fn robot_code(&self) -> bool {
        self.phoenix.is_some() || self.services.iter().any(|name| name == "nt4")
    }

    /// The hostname the roboRIO image gives itself: `roboRIO-6390-FRC`.
    pub fn hostname(&self) -> Option<String> {
        self.team.map(|team| format!("roboRIO-{team}-FRC"))
    }

    pub fn to_identity(&self, link: LinkId, address: String) -> Result<Identity, DriverError> {
        let team = self.team.ok_or_else(|| {
            DriverError::Other(format!(
                "{address} is not on a 10.TE.AM.x robot network, so it has no team to key it by"
            ))
        })?;
        let mut versions = BTreeMap::new();
        let mut attrs = BTreeMap::new();
        attrs.insert("team".into(), team.to_string());
        if let Some(hostname) = self.hostname() {
            attrs.insert(attributes::HOSTNAME.into(), hostname);
        }
        if self.web_ui {
            attrs.insert(attributes::WEB_UI.into(), format!("http://{address}/"));
        }
        attrs.insert(
            "robot_code".into(),
            if self.robot_code() {
                "running"
            } else {
                "not running"
            }
            .into(),
        );
        attrs.insert("services".into(), self.services.join(","));
        if let Some(phoenix) = &self.phoenix {
            versions.insert("phoenix".into(), phoenix.short_version().to_string());
            if let Some(season) = phoenix.season() {
                versions.insert("season".into(), season.to_string());
            }
        }
        if let Some(devices) = &self.can_devices {
            attrs.insert("can.devices".into(), devices.len().to_string());
            let mut buses: Vec<&str> = devices.iter().map(CanDevice::bus).collect();
            buses.sort_unstable();
            buses.dedup();
            attrs.insert("can.buses".into(), buses.join(","));
        }
        let model = self
            .phoenix
            .as_ref()
            .map(|phoenix| phoenix.system.clone())
            .filter(|system| !system.is_empty())
            .unwrap_or_else(|| "roboRIO".into());
        Ok(Identity {
            key: DeviceKey::new(CONTROLLER_FAMILY, team.to_string()),
            model,
            mode: DeviceMode::Normal,
            versions,
            name: self.hostname(),
            link,
            address,
            attributes: attrs,
        })
    }
}

/// Reads a robot controller at `ip`: its web page, Phoenix, which services
/// answer, and (with `can`) its CAN devices. Fails when it isn't a roboRIO.
pub async fn read(
    http: &reqwest::Client,
    ip: Ipv4Addr,
    can: bool,
) -> Result<ControllerReport, DriverError> {
    let root = http
        .get(format!("http://{ip}/"))
        .send()
        .await
        .map_err(|error| DriverError::Unreachable(format!("{ip}: {error}")))?;
    let server = root
        .headers()
        .get(reqwest::header::SERVER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let body = root.text().await.unwrap_or_default();
    if !is_ni_web_ui(server.as_deref(), &body) {
        return Err(DriverError::Other(format!(
            "{ip} is not a roboRIO (web server {})",
            server.as_deref().unwrap_or("unnamed")
        )));
    }
    let (phoenix, services, can_devices) =
        tokio::join!(phoenix_version(http, ip), open_services(ip), async {
            if can {
                can_devices(http, ip).await.ok()
            } else {
                None
            }
        });
    Ok(ControllerReport {
        ip: Some(ip),
        team: team_of(ip),
        web_ui: true,
        phoenix,
        services,
        can_devices,
    })
}

async fn phoenix_version(http: &reqwest::Client, ip: Ipv4Addr) -> Option<PhoenixVersion> {
    http.get(format!("http://{ip}:{PHOENIX_PORT}/?action=getversion"))
        .send()
        .await
        .ok()?
        .json::<PhoenixVersion>()
        .await
        .ok()
        .filter(|version| !version.version.is_empty())
}

/// The CTRE devices on every CAN bus, from Phoenix's diagnostics server
/// (within `http`'s timeout: a busy bus can take seconds).
pub async fn can_devices(http: &reqwest::Client, ip: Ipv4Addr) -> Result<Vec<CanDevice>, String> {
    let body = http
        .get(format!("http://{ip}:{PHOENIX_PORT}/?action=getdevices"))
        .send()
        .await
        .map_err(|error| error.to_string())?
        .text()
        .await
        .map_err(|error| error.to_string())?;
    parse_devices(&body)
}

/// The [`SERVICES`] that accept a connection at `ip`.
pub async fn open_services(ip: Ipv4Addr) -> Vec<String> {
    let checks = SERVICES.iter().map(|(name, port)| async move {
        let connect = tokio::net::TcpStream::connect((ip, *port));
        matches!(
            tokio::time::timeout(crate::probe::TIMEOUT, connect).await,
            Ok(Ok(_))
        )
        .then(|| (*name).to_string())
    });
    futures::future::join_all(checks)
        .await
        .into_iter()
        .flatten()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = include_str!("../tests/fixtures/roborio-root.html");
    const VERSION: &str = include_str!("../tests/fixtures/phoenix-getversion.json");
    const DEVICES: &str = include_str!("../tests/fixtures/phoenix-getdevices.json");

    #[test]
    fn the_ni_web_page_is_a_roborio() {
        assert!(is_ni_web_ui(Some("Embedthis-http"), ROOT));
        assert!(!is_ni_web_ui(Some("nginx"), ROOT));
        assert!(!is_ni_web_ui(Some("Embedthis-http"), "<html></html>"));
        assert!(!is_ni_web_ui(None, ROOT));
    }

    #[test]
    fn phoenix_reports_its_version_and_season() {
        let version: PhoenixVersion = serde_json::from_str(VERSION).unwrap();
        assert_eq!(version.system, "roboRIO");
        assert_eq!(version.short_version(), "2026.3.0");
        assert_eq!(version.season(), Some("2026"));
    }

    #[test]
    fn can_devices_list_by_bus_then_id() {
        let devices = parse_devices(DEVICES).unwrap();
        let listed: Vec<(&str, u32, &str, &str)> = devices
            .iter()
            .map(|d| (d.bus(), d.id, d.model.as_str(), d.name.as_str()))
            .collect();
        assert_eq!(
            listed,
            [
                ("can", 0, "Pigeon 2", "Pigeon "),
                ("can", 1, "Talon FX vers. C", "FLDrive"),
                ("can", 9, "CANCoder", "FLEncoder"),
                ("rio", 33, "Talon FX", "leftShooter1"),
            ]
        );
        assert_eq!(devices[1].firmware, "26.3.0.0 (Phoenix 6)");
        assert!(parse_devices("not json").is_err());
    }

    #[test]
    fn a_controller_is_keyed_by_its_team() {
        let report = ControllerReport {
            ip: Some(Ipv4Addr::new(10, 63, 90, 2)),
            team: Some(6390),
            web_ui: true,
            phoenix: serde_json::from_str(VERSION).ok(),
            services: vec!["ssh".into(), "nt4".into()],
            can_devices: parse_devices(DEVICES).ok(),
        };
        let identity = report
            .to_identity(LinkId("robot-net".into()), "10.63.90.2".into())
            .unwrap();
        assert_eq!(identity.key.to_string(), "roborio:6390");
        assert_eq!(identity.model, "roboRIO");
        assert_eq!(identity.name.as_deref(), Some("roboRIO-6390-FRC"));
        assert_eq!(identity.versions["phoenix"], "2026.3.0");
        assert_eq!(identity.versions["season"], "2026");
        let attrs = &identity.attributes;
        assert_eq!(attrs["manage_url"], "http://10.63.90.2/");
        assert_eq!(attrs["robot_code"], "running");
        assert_eq!(attrs["can.devices"], "4");
        assert_eq!(attrs["can.buses"], "can,rio");
        assert_eq!(attrs["team"], "6390");

        let idle = ControllerReport {
            services: vec!["ssh".into()],
            phoenix: None,
            ..report.clone()
        };
        let identity = idle.to_identity(LinkId("x".into()), "10.63.90.2".into());
        assert_eq!(identity.unwrap().attributes["robot_code"], "not running");
        let off_robot_net = ControllerReport {
            team: None,
            ..report
        };
        assert!(
            off_robot_net
                .to_identity(LinkId("x".into()), "192.168.1.2".into())
                .is_err()
        );
    }
}
