use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use atlas_devices::{DeviceCatalog, DeviceManifest};
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
/// Recognised and read by its device package's data: the catalog's when it
/// has the roboRIO's with `fingerprints`, else the built-in one.
pub struct ControllerDriver {
    manifest: DriverManifest,
    http: reqwest::Client,
    package: Option<DeviceManifest>,
}

impl ControllerDriver {
    pub fn new(catalog: &DeviceCatalog) -> Self {
        Self {
            manifest: manifest(CONTROLLER_FAMILY, "roboRIO"),
            http: http(),
            package: catalog
                .by_model(CONTROLLER_FAMILY)
                .map(|package| package.manifest.clone())
                .filter(|manifest| !manifest.fingerprints.is_empty()),
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
        let package = match &self.package {
            Some(package) => package,
            None => crate::roborio::built_in()?,
        };
        let report = crate::roborio::read(&self.http, package, ip, true).await?;
        if let Some(team) = report.team {
            RobotTeams::shared().add(team);
        }
        Ok(report.to_identity(candidate.link.clone(), candidate.address.clone()))
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
/// computer is on when its API answers there. With Atlas's USB agent on it
/// (`crate::usb_agent`), the boards plugged into its USB too.
pub struct SystemcoreDriver {
    manifest: DriverManifest,
    http: reqwest::Client,
    /// The agent's package Atlas carries, and its version.
    agent_package: Option<(PathBuf, String)>,
}

impl SystemcoreDriver {
    /// `agent_dirs`: where the USB agent's `.ipk` may be
    /// (`crate::usb_agent::package`).
    pub fn new(agent_dirs: &[PathBuf]) -> Self {
        let mut manifest = manifest(crate::systemcore::SYSTEMCORE_FAMILY, "SystemCore");
        manifest.link_kinds = vec![LinkKind::Ethernet, LinkKind::UsbNetwork];
        Self {
            manifest,
            http: http(),
            agent_package: crate::usb_agent::package(agent_dirs).and_then(|path| {
                let version = crate::usb_agent::package_version(&path)?;
                Some((path, version))
            }),
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
        let mut identity =
            crate::systemcore::read(&self.http, ip, candidate.link.clone(), serial).await?;
        identity.versions = crate::systemcore_versions::read(ip).await;
        let running = crate::usb_agent::info(&self.http, ip)
            .await
            .map(|agent| agent.version);
        if let Some(running) = &running {
            identity
                .attributes
                .insert(crate::usb_agent::ATTRIBUTE.into(), running.clone());
        }
        if let Some((_, offered)) = &self.agent_package
            && running.as_ref() != Some(offered)
        {
            identity
                .attributes
                .insert(crate::usb_agent::OFFERED.into(), offered.clone());
        }
        if let Some(team) = identity.attributes.get("team").and_then(|t| t.parse().ok()) {
            RobotTeams::shared().add(team);
        }
        Ok(identity)
    }

    /// Its identity and web page, installing packages (`.ipk`) through its
    /// package manager, by SSH key (`crate::package`), and installing or
    /// updating Atlas's USB agent (`crate::usb_agent`).
    fn capabilities(&self, device: &Identity) -> Capabilities {
        let ip = ip_of(&device.address).ok();
        Capabilities {
            install: ip
                .map(|ip| {
                    Arc::new(crate::package::PackageMethod { ip })
                        as Arc<dyn atlas_driver::InstallMethod>
                })
                .into_iter()
                .collect(),
            actions: ip.map(|ip| {
                Arc::new(crate::usb_agent::AgentActions {
                    ip,
                    package: self.agent_package.as_ref().map(|(path, _)| path.clone()),
                }) as Arc<dyn atlas_driver::ActionsCapability>
            }),
            ..Capabilities::default()
        }
    }

    /// The boards behind it: as its HeliOS lists them
    /// (`crate::controller_peers`), reached only through the routes HeliOS
    /// offers for them; and, with Atlas's USB agent on it, the Raspberry Pi
    /// boards in USB boot on its USB (`crate::usb_agent`).
    async fn children(&self, device: &Identity) -> Result<Vec<Candidate>, DriverError> {
        let ip = ip_of(&device.address)?;
        let mut found = crate::controller_peers::candidates(&self.http, ip).await?;
        if device.attributes.contains_key(crate::usb_agent::ATTRIBUTE) {
            found.extend(crate::usb_agent::boards(&self.http, ip, &device.link).await);
        }
        Ok(found)
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

/// The robot network's drivers, for registering, with the device packages
/// that describe them and where the USB agent's package may be
/// (`agent_dirs`, `crate::usb_agent::package`).
pub fn drivers(catalog: &DeviceCatalog, agent_dirs: &[PathBuf]) -> Vec<Arc<dyn Driver>> {
    vec![
        Arc::new(ControllerDriver::new(catalog)),
        Arc::new(RadioDriver::default()),
        Arc::new(SystemcoreDriver::new(agent_dirs)),
    ]
}
