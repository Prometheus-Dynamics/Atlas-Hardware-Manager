use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use atlas_driver::robot_net::{CONTROLLER_FAMILY, RADIO_FAMILY, RobotTeams};
use atlas_driver::{
    Candidate, Capabilities, Driver, DriverError, DriverManifest, Family, Identity, Link, LinkKind,
    LogsCapability, Metric, TelemetryCapability,
};

use crate::console::ConsoleLogs;
use crate::probe::{RobotProbe, own_robot_nets};

const FETCH_TIMEOUT: Duration = Duration::from_secs(2);

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap_or_default()
}

fn ip_of(address: &str) -> Result<Ipv4Addr, DriverError> {
    address
        .parse()
        .map_err(|_| DriverError::Other(format!("`{address}` is not an IPv4 address")))
}

fn manifest(family: &str, name: &str) -> DriverManifest {
    DriverManifest {
        family: Family::new(family),
        name: name.into(),
        version: env!("CARGO_PKG_VERSION").into(),
        // Directly on a robot network; behind a board's USB route they are
        // that board's children (a gateway link).
        link_kinds: vec![LinkKind::Ethernet],
        priority: 0,
    }
}

/// The robot networks this computer is on directly, as candidates of
/// `family` on `link`.
async fn direct_candidates(link: &Link, family: &str) -> Vec<Candidate> {
    let nets = own_robot_nets();
    if nets.is_empty() {
        return Vec::new();
    }
    RobotProbe::shared()
        .candidates(&nets, &link.id)
        .await
        .into_iter()
        .filter(|candidate| candidate.family.as_str() == family)
        .collect()
}

/// The robot controller (roboRIO), read-only: identity, web page, console.
pub struct ControllerDriver {
    manifest: DriverManifest,
    http: reqwest::Client,
}

impl Default for ControllerDriver {
    fn default() -> Self {
        Self {
            manifest: manifest(CONTROLLER_FAMILY, "roboRIO"),
            http: http(),
        }
    }
}

#[async_trait]
impl Driver for ControllerDriver {
    fn manifest(&self) -> &DriverManifest {
        &self.manifest
    }

    async fn discover(&self, link: &Link) -> Result<Vec<Candidate>, DriverError> {
        Ok(direct_candidates(link, CONTROLLER_FAMILY).await)
    }

    async fn identify(&self, candidate: &Candidate) -> Result<Identity, DriverError> {
        let ip = ip_of(&candidate.address)?;
        let report = crate::roborio::read(&self.http, ip, true).await?;
        if let Some(team) = report.team {
            RobotTeams::shared().add(team);
        }
        report.to_identity(candidate.link.clone(), candidate.address.clone())
    }

    /// The console as logs, while it accepts a connection.
    fn capabilities(&self, device: &Identity) -> Capabilities {
        let console = device
            .attributes
            .get("services")
            .is_some_and(|services| services.split(',').any(|name| name == "console"));
        Capabilities {
            logs: ip_of(&device.address)
                .ok()
                .filter(|_| console)
                .map(|ip| Arc::new(ConsoleLogs { ip }) as Arc<dyn LogsCapability>),
            ..Capabilities::default()
        }
    }
}

/// The robot radio (VH-109), read-only: identity, web page, link readings.
pub struct RadioDriver {
    manifest: DriverManifest,
    http: reqwest::Client,
}

impl Default for RadioDriver {
    fn default() -> Self {
        Self {
            manifest: manifest(RADIO_FAMILY, "FRC radio"),
            http: http(),
        }
    }
}

#[async_trait]
impl Driver for RadioDriver {
    fn manifest(&self) -> &DriverManifest {
        &self.manifest
    }

    async fn discover(&self, link: &Link) -> Result<Vec<Candidate>, DriverError> {
        Ok(direct_candidates(link, RADIO_FAMILY).await)
    }

    async fn identify(&self, candidate: &Candidate) -> Result<Identity, DriverError> {
        let ip = ip_of(&candidate.address)?;
        let status = crate::radio::read(&self.http, ip).await?;
        let identity = status.to_identity(candidate.link.clone(), candidate.address.clone())?;
        if let Ok(team) = identity.key.serial.0.parse() {
            RobotTeams::shared().add(team);
        }
        Ok(identity)
    }

    fn capabilities(&self, device: &Identity) -> Capabilities {
        Capabilities {
            telemetry: ip_of(&device.address).ok().map(|ip| {
                Arc::new(RadioTelemetry {
                    http: self.http.clone(),
                    ip,
                }) as Arc<dyn TelemetryCapability>
            }),
            ..Capabilities::default()
        }
    }
}

struct RadioTelemetry {
    http: reqwest::Client,
    ip: Ipv4Addr,
}

#[async_trait]
impl TelemetryCapability for RadioTelemetry {
    async fn read(&self, _device: &Identity) -> Result<Vec<Metric>, DriverError> {
        Ok(crate::radio::read(&self.http, self.ip).await?.metrics())
    }
}

/// A WPILib SystemCore, read-only (`crate::systemcore`): over its USB link
/// when one is plugged in here, and at `.2` on a robot network this
/// computer is on when its API answers there.
pub struct SystemcoreDriver {
    manifest: DriverManifest,
    http: reqwest::Client,
}

impl Default for SystemcoreDriver {
    fn default() -> Self {
        let mut manifest = manifest(crate::systemcore::SYSTEMCORE_FAMILY, "SystemCore");
        manifest.link_kinds = vec![LinkKind::Ethernet, LinkKind::UsbNetwork];
        Self {
            manifest,
            http: http(),
        }
    }
}

#[async_trait]
impl Driver for SystemcoreDriver {
    fn manifest(&self) -> &DriverManifest {
        &self.manifest
    }

    async fn discover(&self, link: &Link) -> Result<Vec<Candidate>, DriverError> {
        use crate::systemcore::{SYSTEMCORE_FAMILY, answers, plugged_in, usb_address};
        let candidate = |address: String| Candidate {
            link: link.id.clone(),
            family: Family::new(SYSTEMCORE_FAMILY),
            address,
        };
        match link.kind {
            LinkKind::UsbNetwork => {
                if plugged_in().await.is_empty() || !answers(usb_address()).await {
                    return Ok(Vec::new());
                }
                Ok(vec![candidate(usb_address().to_string())])
            }
            _ => {
                let mut found = Vec::new();
                for controller in direct_candidates(link, CONTROLLER_FAMILY).await {
                    if let Ok(ip) = ip_of(&controller.address)
                        && answers(ip).await
                    {
                        found.push(candidate(controller.address));
                    }
                }
                Ok(found)
            }
        }
    }

    async fn identify(&self, candidate: &Candidate) -> Result<Identity, DriverError> {
        let ip = ip_of(&candidate.address)?;
        let usb = ip == crate::systemcore::usb_address();
        let serial = if usb {
            crate::systemcore::plugged_in().await.into_iter().next()
        } else {
            None
        };
        let identity =
            crate::systemcore::read(&self.http, ip, candidate.link.clone(), serial).await?;
        if let Some(team) = identity.attributes.get("team").and_then(|t| t.parse().ok()) {
            RobotTeams::shared().add(team);
        }
        Ok(identity)
    }

    /// Read-only: its identity and web page, nothing else yet.
    fn capabilities(&self, _device: &Identity) -> Capabilities {
        Capabilities::default()
    }

    /// While one is plugged in here: whether its USB link has an address,
    /// and that nothing else on this computer holds the unit's.
    async fn health(&self) -> Vec<atlas_driver::HealthCheck> {
        use crate::systemcore::{
            host_addresses, plugged_in, usb_address, usb_link_checks, usb_links,
        };
        if plugged_in().await.is_empty() {
            return Vec::new();
        }
        usb_link_checks(usb_address(), &usb_links(), &host_addresses())
    }
}

/// The robot network's drivers, for registering.
pub fn drivers() -> Vec<Arc<dyn Driver>> {
    vec![
        Arc::new(ControllerDriver::default()),
        Arc::new(RadioDriver::default()),
        Arc::new(SystemcoreDriver::default()),
    ]
}
