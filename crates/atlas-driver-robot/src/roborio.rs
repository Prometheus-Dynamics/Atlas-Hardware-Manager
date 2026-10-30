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
//!
//! Recognising it and reading what it is are data, its device package
//! (`devices/roborio/manifest.json`, built in); this adds what data can't
//! say: Phoenix's answer as a whole, the CAN devices, and whether robot
//! code runs.

use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::sync::LazyLock;

use atlas_devices::{DeviceManifest, NotRecognized, Recognized, Seen};
use atlas_driver::robot_net::{CONTROLLER_FAMILY, team_of};
use atlas_driver::{DeviceKey, DeviceMode, DriverError, Identity, LinkId, attributes};
use serde::{Deserialize, Serialize};

use crate::http_probe::HttpProbe;

/// The CTRE Phoenix diagnostics server's port.
pub const PHOENIX_PORT: u16 = 1250;
const PHOENIX_VERSION: &str = "/?action=getversion";
/// The console (RioLog) stream's port.
pub const CONSOLE_PORT: u16 = 1741;

/// The roboRIO's device package as built in: for when the catalog has
/// none (a test, `atlas robot pull`), or one without `fingerprints`.
static BUILT_IN: LazyLock<Result<DeviceManifest, String>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../../devices/roborio/manifest.json"))
        .map_err(|error| format!("devices/roborio/manifest.json: {error}"))
});

pub(crate) fn built_in() -> Result<&'static DeviceManifest, DriverError> {
    BUILT_IN
        .as_ref()
        .map_err(|error| DriverError::Other(error.clone()))
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
    /// The Phoenix diagnostics server, when the robot program runs one.
    pub phoenix: Option<PhoenixVersion>,
    /// The services that accept a connection, by name (the package's
    /// `services`).
    pub services: Vec<String>,
    pub can_devices: Option<Vec<CanDevice>>,
    /// What the package's data read.
    #[serde(skip)]
    pub found: Recognized,
}

impl ControllerReport {
    /// What the package's data read, made into a report.
    pub fn from_found(ip: Ipv4Addr, found: Recognized) -> Self {
        let phoenix = found
            .answer(PHOENIX_PORT, PHOENIX_VERSION)
            .filter(|answer| (200..300).contains(&answer.status))
            .and_then(|answer| serde_json::from_str::<PhoenixVersion>(&answer.body).ok())
            .filter(|version| !version.version.is_empty());
        let services = found
            .attributes
            .get("services")
            .map(|list| {
                list.split(',')
                    .filter(|name| !name.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        Self {
            ip: Some(ip),
            team: found.key.parse().ok(),
            phoenix,
            services,
            can_devices: None,
            found,
        }
    }

    /// Whether the robot program is running: its NetworkTables server or
    /// Phoenix's diagnostics server answers.
    pub fn robot_code(&self) -> bool {
        self.phoenix.is_some() || self.services.iter().any(|name| name == "nt4")
    }

    /// The hostname the roboRIO image gives itself: `roboRIO-6390-FRC`.
    pub fn hostname(&self) -> Option<String> {
        self.found.attributes.get(attributes::HOSTNAME).cloned()
    }

    pub fn to_identity(&self, link: LinkId, address: String) -> Identity {
        let mut attrs = self.found.attributes.clone();
        attrs.insert(
            "robot_code".into(),
            if self.robot_code() {
                "running"
            } else {
                "not running"
            }
            .into(),
        );
        if let Some(devices) = &self.can_devices {
            attrs.insert("can.devices".into(), devices.len().to_string());
            let mut buses: Vec<&str> = devices.iter().map(CanDevice::bus).collect();
            buses.sort_unstable();
            buses.dedup();
            attrs.insert("can.buses".into(), buses.join(","));
        }
        Identity {
            key: DeviceKey::new(CONTROLLER_FAMILY, self.found.key.clone()),
            model: self.found.model.clone().unwrap_or_else(|| "roboRIO".into()),
            mode: DeviceMode::Normal,
            versions: self.found.versions.clone(),
            name: self.found.name.clone(),
            link,
            address,
            attributes: attrs,
        }
    }
}

/// Reads a robot controller at `ip` by `package`'s data, and (with `can`)
/// its CAN devices. Fails when it isn't a roboRIO, or isn't on a robot
/// network to key it by team.
pub async fn read(
    http: &reqwest::Client,
    package: &DeviceManifest,
    ip: Ipv4Addr,
    can: bool,
) -> Result<ControllerReport, DriverError> {
    let seen = Seen {
        address: ip.to_string(),
        facts: team_of(ip)
            .map(|team| ("team".to_string(), team.to_string()))
            .into_iter()
            .collect::<BTreeMap<_, _>>(),
    };
    let probe = HttpProbe { http, ip };
    let found = atlas_devices::recognize(package, &probe, &seen)
        .await
        .map_err(|refused| match refused {
            NotRecognized::Unreachable(why) => DriverError::Unreachable(why),
            other => DriverError::Other(other.to_string()),
        })?;
    // Only once it is a roboRIO: nothing else is asked for its CAN bus.
    let can_devices = if can {
        can_devices(http, ip).await.ok()
    } else {
        None
    };
    Ok(ControllerReport {
        can_devices,
        ..ControllerReport::from_found(ip, found)
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = include_str!("../tests/fixtures/roborio-root.html");
    const VERSION: &str = include_str!("../tests/fixtures/phoenix-getversion.json");
    const DEVICES: &str = include_str!("../tests/fixtures/phoenix-getdevices.json");

    /// The fixtures' answers; `open` ports accept a connection.
    struct Fixture {
        server: &'static str,
        root: &'static str,
        phoenix: bool,
        open: Vec<u16>,
    }

    impl atlas_devices::Probe for Fixture {
        async fn get(&self, port: u16, path: &str) -> Option<atlas_devices::HttpAnswer> {
            let body = match (port, path) {
                (80, "/") => self.root,
                (PHOENIX_PORT, PHOENIX_VERSION) if self.phoenix => VERSION,
                _ => return None,
            };
            Some(atlas_devices::HttpAnswer {
                status: 200,
                headers: [("server".to_string(), self.server.to_string())].into(),
                body: body.into(),
            })
        }

        async fn open(&self, ports: Vec<u16>) -> Vec<bool> {
            ports.iter().map(|port| self.open.contains(port)).collect()
        }
    }

    fn running() -> Fixture {
        Fixture {
            server: "Embedthis-http",
            root: ROOT,
            phoenix: true,
            open: vec![22, 5810, PHOENIX_PORT],
        }
    }

    async fn read_fixture(
        fixture: &Fixture,
        address: &str,
    ) -> Result<ControllerReport, NotRecognized> {
        let ip: Ipv4Addr = address.parse().unwrap();
        let seen = Seen {
            address: address.into(),
            facts: team_of(ip)
                .map(|team| ("team".to_string(), team.to_string()))
                .into_iter()
                .collect(),
        };
        let found = atlas_devices::recognize(built_in().unwrap(), fixture, &seen).await?;
        Ok(ControllerReport::from_found(ip, found))
    }

    #[tokio::test]
    async fn its_package_recognises_ni_s_web_page() {
        let found = read_fixture(&running(), "10.63.90.2").await.unwrap();
        assert_eq!(found.found.fingerprint, "ni-web-ui");
        for (server, root) in [("nginx", ROOT), ("Embedthis-http", "<html></html>")] {
            let other = Fixture {
                server,
                root,
                ..running()
            };
            assert!(matches!(
                read_fixture(&other, "10.63.90.2").await,
                Err(NotRecognized::NotThisModel(_))
            ));
        }
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

    #[tokio::test]
    async fn a_controller_is_keyed_by_its_team() {
        let report = ControllerReport {
            can_devices: parse_devices(DEVICES).ok(),
            ..read_fixture(&running(), "10.63.90.2").await.unwrap()
        };
        assert_eq!(report.team, Some(6390));
        assert_eq!(report.services, ["ssh", "nt4", "phoenix"]);
        assert_eq!(report.phoenix.as_ref().unwrap().short_version(), "2026.3.0");
        let identity = report.to_identity(LinkId("robot-net".into()), "10.63.90.2".into());
        assert_eq!(identity.key.to_string(), "roborio:6390");
        assert_eq!(identity.model, "roboRIO");
        assert_eq!(identity.name.as_deref(), Some("roboRIO-6390-FRC"));
        assert_eq!(identity.versions["phoenix"], "2026.3.0");
        assert_eq!(identity.versions["season"], "2026");
        let attrs = &identity.attributes;
        assert_eq!(attrs["manage_url"], "http://10.63.90.2/");
        assert_eq!(attrs["hostname"], "roboRIO-6390-FRC");
        assert_eq!(attrs["robot_code"], "running");
        assert_eq!(attrs["services"], "ssh,nt4,phoenix");
        assert_eq!(attrs["can.devices"], "4");
        assert_eq!(attrs["can.buses"], "can,rio");
        assert_eq!(attrs["team"], "6390");

        let idle = Fixture {
            phoenix: false,
            open: vec![22],
            ..running()
        };
        let idle = read_fixture(&idle, "10.63.90.2").await.unwrap();
        let identity = idle.to_identity(LinkId("x".into()), "10.63.90.2".into());
        assert_eq!(identity.attributes["robot_code"], "not running");
        assert_eq!(identity.model, "roboRIO");
        assert!(identity.versions.is_empty());
        assert!(matches!(
            read_fixture(&running(), "192.168.1.2").await,
            Err(NotRecognized::NoKey(_))
        ));
    }
}
