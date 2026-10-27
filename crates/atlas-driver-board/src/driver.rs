use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use atlas_devices::{DeviceCatalog, DevicePackage};
use atlas_driver::{
    ActionsCapability, Candidate, Capabilities, DeviceKey, Driver, DriverError, DriverManifest,
    Family, HealthCheck, Identity, Link, LinkId, LinkKind, LinkSource, LogsCapability,
    SelfTestCapability, SlotsCapability, StatusCapability, TelemetryCapability, UpdateCapability,
};

use crate::browse::Browser;
use crate::contract::BoardIdentity;
use crate::gadget::GadgetProbe;
use crate::live::{BoardActions, BoardLogs, BoardTelemetry, resolve};
use crate::ssh::{AB_METHOD, SshAccess, SshUpdate};
use crate::ssh_actions::{
    BoardDeviceActions, BoardSelfTest, SELFTEST_DIAGNOSTIC, SshOffer, USB_BOOT_METHOD,
};
use crate::status::{BoardStatus, StatusTelemetry};

const LINK_ID: &str = "mdns";
/// The link a board's children are found on: the robot network behind its
/// USB route.
pub const ROUTED_LINK_ID: &str = "robot-net";
const FETCH_TIMEOUT: Duration = Duration::from_secs(2);

/// The network link: every `_pd-device._tcp` advertisement on any interface,
/// USB gadget networks included.
pub struct NetworkLinks;

#[async_trait]
impl LinkSource for NetworkLinks {
    async fn links(&self) -> Vec<Link> {
        // Starting the browser here means discovery has results by the
        // second scan without a separate start-up step.
        let _ = Browser::shared();
        vec![Link {
            id: LinkId(LINK_ID.into()),
            kind: LinkKind::Ethernet,
            label: "Network (mDNS)".into(),
        }]
    }

    fn name(&self) -> &str {
        "Network"
    }

    fn watch(&self, notify: atlas_driver::ChangeNotifier) -> bool {
        Browser::shared().subscribe(notify)
    }

    async fn health(&self) -> Vec<HealthCheck> {
        let browser = Browser::shared();
        vec![match browser.advertisements() {
            Ok(found) => HealthCheck::ok(
                "network.mdns",
                "Network discovery",
                format!(
                    "Listening for boards with mDNS; {} advertised right now.",
                    found.len()
                ),
            ),
            Err(error) => HealthCheck::error(
                "network.mdns",
                "Network discovery",
                format!("mDNS could not start: {error}"),
                "Allow Atlas through the firewall for UDP port 5353 on private networks.",
            ),
        }]
    }
}

/// One hardware model from the device catalog, found over the network.
pub struct BoardDriver {
    manifest: DriverManifest,
    package: Arc<DevicePackage>,
    http: reqwest::Client,
    /// What each device last reported, by the identity URL it answered on
    /// (a board on USB and Ethernet reports on both), for the optional live
    /// endpoints: a device's capabilities use the report from the link its
    /// record chose, never whichever answered last.
    reported: Mutex<HashMap<String, (DeviceKey, BoardIdentity)>>,
    /// Boards found without mDNS on this computer's own networks (sweep.rs).
    sweeper: crate::sweep::Sweeper,
    /// Boards found behind a board's USB route, by robot network.
    routed: Mutex<HashMap<crate::sweep::Net, crate::sweep::Sweeper>>,
    /// The last discovery's USB gadget probes, by candidate address.
    probes: Mutex<HashMap<String, GadgetProbe>>,
    ssh: SshAccess,
}

impl BoardDriver {
    #[cfg(test)]
    pub fn new(package: DevicePackage) -> Self {
        Self::with_ssh(package, SshAccess::default())
    }

    /// A driver that updates A/B boards over SSH with `ssh`.
    pub fn with_ssh(package: DevicePackage, ssh: SshAccess) -> Self {
        Self {
            manifest: DriverManifest {
                family: Family::new(package.manifest.model.to_ascii_lowercase()),
                name: package.manifest.display_name().to_string(),
                version: env!("CARGO_PKG_VERSION").into(),
                link_kinds: vec![LinkKind::Ethernet, LinkKind::UsbNetwork],
                priority: 0,
            },
            package: Arc::new(package),
            http: reqwest::Client::builder()
                .timeout(FETCH_TIMEOUT)
                .build()
                .unwrap_or_default(),
            reported: Mutex::new(HashMap::new()),
            probes: Mutex::new(HashMap::new()),
            sweeper: crate::sweep::Sweeper::default(),
            routed: Mutex::new(HashMap::new()),
            ssh,
        }
    }

    /// This model's boards plugged in over USB, each probed at the gadget
    /// address its USB serial gives (see `gadget`).
    async fn gadget_candidates(&self, link: &Link) -> Vec<Candidate> {
        let probes = crate::gadget::probes(&self.package.manifest, &crate::gadget::gadgets().await);
        let candidates = probes
            .iter()
            .map(|probe| Candidate {
                link: link.id.clone(),
                family: self.manifest.family.clone(),
                address: probe.url.clone(),
            })
            .collect();
        *self.probes.lock().unwrap_or_else(|p| p.into_inner()) = probes
            .into_iter()
            .map(|probe| (probe.url.clone(), probe))
            .collect();
        candidates
    }

    /// A USB gadget board's identity: the first of its URLs that answers as
    /// the board with its USB serial. Returns the URL that did.
    async fn fetch_gadget(
        &self,
        probe: &GadgetProbe,
    ) -> Result<(String, BoardIdentity), DriverError> {
        let mut errors = Vec::new();
        for url in probe.urls() {
            match self.fetch(url).await {
                Ok(reported) => {
                    let serial = atlas_driver::attributes::normalize_board_serial(&reported.serial);
                    match &probe.serial {
                        Some(expected) if serial.as_ref() != Some(expected) => {
                            errors.push(format!(
                                "{url} is board {}, not the USB board {expected}",
                                reported.serial
                            ))
                        }
                        _ => return Ok((url.to_string(), reported)),
                    }
                }
                Err(error) => errors.push(format!("{url}: {error}")),
            }
        }
        Err(DriverError::Unreachable(errors.join("; ")))
    }

    /// A candidate's identity, or what its mDNS TXT record says when the
    /// endpoint doesn't answer.
    async fn fetch_candidate(&self, candidate: &Candidate) -> Result<BoardIdentity, DriverError> {
        match self.fetch(&candidate.address).await {
            Ok(reported) => Ok(reported),
            Err(error) => Browser::shared()
                .advertisements()
                .unwrap_or_default()
                .into_iter()
                .find(|ad| ad.identity_url() == candidate.address)
                .and_then(|ad| BoardIdentity::from_txt(&ad.txt))
                .ok_or_else(|| DriverError::Unreachable(format!("{}: {error}", candidate.address))),
        }
    }

    /// The robot networks this computer reaches through `device`'s USB
    /// route: none unless it was reached at its USB gadget address and
    /// routes (`usb_route` active); a route wider than a /24 narrows to the
    /// robot networks Atlas can name in it (the board's own addresses on it,
    /// the teams it knows).
    fn routed_nets(&self, device: &Identity) -> Vec<crate::sweep::Net> {
        let Some((address, report)) = self.reported(device) else {
            return Vec::new();
        };
        let Some(route) = report.usb_route.as_ref().filter(|route| route.active) else {
            return Vec::new();
        };
        let host = reqwest::Url::parse(&address)
            .ok()
            .and_then(|url| url.host_str().map(str::to_string));
        let on_usb = match &report.gadget {
            Some(gadget) => host.as_deref() == Some(gadget.address.as_str()),
            None => device.link.0 == crate::gadget::LINK_ID,
        };
        if !on_usb {
            return Vec::new();
        }
        let own: Vec<std::net::Ipv4Addr> = route
            .addresses
            .iter()
            .filter_map(|cidr| cidr.split('/').next()?.trim().parse().ok())
            .collect();
        let teams = atlas_driver::robot_net::RobotTeams::shared().all();
        let mut nets: Vec<crate::sweep::Net> = route
            .subnets
            .iter()
            .filter_map(|net| crate::sweep::Net::parse(net))
            .flat_map(|net| atlas_driver::robot_net::robot_nets_in(net, &own, &teams))
            .collect();
        nets.sort_unstable();
        nets.dedup();
        nets
    }

    /// The sweeper for one routed robot network.
    fn routed_sweeper(&self, net: crate::sweep::Net) -> crate::sweep::Sweeper {
        self.routed
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entry(net)
            .or_default()
            .clone()
    }

    /// The report from `device`'s own address, or, if that one is gone,
    /// any report of the same device.
    fn reported(&self, device: &Identity) -> Option<(String, BoardIdentity)> {
        let reported = self
            .reported
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reported
            .get(&device.address)
            .filter(|(key, _)| *key == device.key)
            .map(|(_, report)| (device.address.clone(), report.clone()))
            .or_else(|| {
                reported
                    .iter()
                    .filter(|(_, (key, _))| *key == device.key)
                    .min_by(|a, b| a.0.cmp(b.0))
                    .map(|(address, (_, report))| (address.clone(), report.clone()))
            })
    }

    async fn fetch(&self, url: &str) -> Result<BoardIdentity, String> {
        self.http
            .get(url)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|error| error.to_string())?
            .json::<BoardIdentity>()
            .await
            .map_err(|error| error.to_string())
    }
}

/// A driver for every model in the catalog, updating A/B boards over SSH
/// with `ssh`.
pub fn drivers_for_catalog(catalog: &DeviceCatalog, ssh: &SshAccess) -> Vec<Arc<dyn Driver>> {
    catalog
        .packages()
        .iter()
        .map(|package| {
            Arc::new(BoardDriver::with_ssh(package.clone(), ssh.clone())) as Arc<dyn Driver>
        })
        .collect()
}

#[async_trait]
impl Driver for BoardDriver {
    fn manifest(&self) -> &DriverManifest {
        &self.manifest
    }

    async fn discover(&self, link: &Link) -> Result<Vec<Candidate>, DriverError> {
        if link.id.0 == crate::gadget::LINK_ID {
            return Ok(self.gadget_candidates(link).await);
        }
        if link.id.0 != LINK_ID {
            return Ok(Vec::new());
        }
        // mDNS when it works; the sweep of this computer's robot networks
        // whatever mDNS does.
        let advertisements = Browser::shared().advertisements().unwrap_or_default();
        let mut addresses: Vec<String> = advertisements
            .into_iter()
            .filter(|ad| {
                ad.model()
                    .is_some_and(|model| model.eq_ignore_ascii_case(self.manifest.family.as_str()))
            })
            .map(|ad| ad.identity_url())
            .collect();
        // Networks behind a board's USB route are that board's children.
        let nets = crate::sweep::own_robot_nets();
        for found in self.sweeper.found(&self.http, nets) {
            if !addresses.contains(&found) {
                addresses.push(found);
            }
        }
        Ok(addresses
            .into_iter()
            .map(|address| Candidate {
                link: link.id.clone(),
                family: self.manifest.family.clone(),
                address,
            })
            .collect())
    }

    /// A USB gadget candidate counts only when the board that answers has
    /// the USB serial its address came from.
    async fn identify(&self, candidate: &Candidate) -> Result<Identity, DriverError> {
        let probe = self
            .probes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&candidate.address)
            .filter(|_| candidate.link.0 == crate::gadget::LINK_ID)
            .cloned();
        let (address, reported) = match probe {
            Some(probe) => self.fetch_gadget(&probe).await?,
            None => (
                candidate.address.clone(),
                self.fetch_candidate(candidate).await?,
            ),
        };
        if !reported
            .model
            .eq_ignore_ascii_case(self.manifest.family.as_str())
        {
            return Err(DriverError::Other(format!(
                "{address} reports model {}, but advertised {}",
                reported.model, self.manifest.family
            )));
        }
        let mut identity =
            reported.to_identity(Some(&self.package), candidate.link.clone(), address.clone());
        if let Some(ad) = Browser::shared()
            .advertisements()
            .unwrap_or_default()
            .into_iter()
            .find(|ad| ad.identity_url() == address)
            .filter(|ad| !ad.addresses.is_empty())
        {
            let ips: Vec<String> = ad.addresses.iter().map(ToString::to_string).collect();
            identity
                .attributes
                .insert(atlas_driver::attributes::ADDRESSES.into(), ips.join(","));
        }
        if let Some(tty) = crate::serial::serial_console(&reported.serial) {
            identity.attributes.insert("serial_console".into(), tty);
        }
        self.reported
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(address, (identity.key.clone(), reported));
        Ok(identity)
    }

    /// A board routing its USB link onto the robot network (`usb_route`)
    /// is the way to everything on it: the boards that answer a sweep of
    /// each robot network it routes to, and the robot's controller and
    /// radio (`atlas-driver-robot`), all on its gateway link; each board to
    /// its own model's driver. Only from the
    /// board this computer reaches over USB; the board itself (on the robot
    /// network too) isn't its own child.
    async fn children(&self, device: &Identity) -> Result<Vec<Candidate>, DriverError> {
        let nets = self.routed_nets(device);
        if nets.is_empty() {
            return Ok(Vec::new());
        }
        let link = LinkId(ROUTED_LINK_ID.into());
        let own_serial = &device.key.serial.0;
        let mut candidates = Vec::new();
        for net in &nets {
            let found = self
                .routed_sweeper(*net)
                .found_now(&self.http, vec![*net])
                .await;
            candidates.extend(
                found
                    .into_iter()
                    .filter(|(_, swept)| swept.serial != *own_serial)
                    .filter(|(_, swept)| !swept.model.is_empty())
                    .map(|(address, swept)| Candidate {
                        link: link.clone(),
                        family: Family::new(swept.model),
                        address,
                    }),
            );
        }
        candidates.extend(
            atlas_driver_robot::RobotProbe::shared()
                .candidates(&nets, &link)
                .await,
        );
        Ok(candidates)
    }

    /// Boards listing `ab-tryboot` update over SSH; reflashing goes through
    /// USB boot recovery. Metrics, logs, actions, status and events are
    /// offered when the device lists them; without a metrics endpoint, the
    /// status's temperatures and fan are the telemetry.
    fn capabilities(&self, device: &Identity) -> Capabilities {
        let Some((identity_url, reported)) = self.reported(device) else {
            return Capabilities::default();
        };
        // Behind a robot controller only its identity and its orion-node
        // (atlas-driver-orion's tunnel) are offered: its endpoints and SSH
        // would resolve to the controller itself.
        if atlas_driver::controller_peer::ControllerPeer::from_address(&identity_url).is_some() {
            return Capabilities::default();
        }
        let endpoint = |name: &str| {
            reported
                .endpoints
                .get(name)
                .and_then(|path| resolve(&identity_url, path))
        };
        let http = self.http.clone();
        let has = |method: &str| reported.update_methods.iter().any(|m| m == method);
        // Root work (A/B updates, restarting into USB boot) goes over SSH to
        // the host the identity was read from.
        let ssh = || {
            let url = reqwest::Url::parse(&identity_url).ok()?;
            let host = url.host_str()?.trim_matches(['[', ']']).to_string();
            Some(SshUpdate {
                access: self.ssh.clone(),
                host,
                key: device.key.clone(),
                usb_serial: Some(reported.serial.clone()).filter(|serial| !serial.is_empty()),
            })
        };
        let slots = has(crate::slots::SLOTS_METHOD)
            .then(ssh)
            .flatten()
            .map(|ssh| Arc::new(crate::slots::BoardSlots { ssh }) as Arc<dyn SlotsCapability>);
        let update = has(AB_METHOD)
            .then(ssh)
            .flatten()
            .map(|ssh| Arc::new(ssh) as Arc<dyn UpdateCapability>);
        let http_actions = endpoint("actions")
            .filter(|_| !reported.actions.is_empty())
            .map(|url| BoardActions {
                http: http.clone(),
                url,
                actions: reported.actions.iter().map(|a| a.to_action()).collect(),
            });
        let status = endpoint("status").map(|status_url| {
            Arc::new(
                BoardStatus::new(
                    http.clone(),
                    status_url,
                    endpoint("events"),
                    endpoint("stream"),
                )
                .with_ssh(ssh()),
            )
        });
        // A board with the device package's writer, self-test or status is
        // one Atlas reaches as root over SSH (the Flash tab's key).
        let package_board = has(AB_METHOD)
            || slots.is_some()
            || !reported.diagnostics.is_empty()
            || status.is_some();
        let offer = SshOffer {
            usb_boot: has(USB_BOOT_METHOD),
            set_clock: device
                .attributes
                .contains_key(atlas_driver::attributes::CLOCK_OFFSET_S),
            power: package_board,
            updates: has(AB_METHOD),
            ssh_key: self.ssh.public_key(),
        };
        let ssh_actions = (offer.usb_boot || offer.set_clock || offer.power)
            .then(ssh)
            .flatten();
        let selftest = reported
            .diagnostics
            .iter()
            .any(|name| name == SELFTEST_DIAGNOSTIC)
            .then(ssh)
            .flatten()
            .map(|ssh| Arc::new(BoardSelfTest { ssh }) as Arc<dyn SelfTestCapability>);
        let actions = (http_actions.is_some() || ssh_actions.is_some()).then(|| {
            Arc::new(BoardDeviceActions {
                http: http_actions,
                ssh: ssh_actions,
                offer,
            }) as Arc<dyn ActionsCapability>
        });
        Capabilities {
            update,
            telemetry: endpoint("metrics")
                .map(|url| {
                    Arc::new(BoardTelemetry {
                        http: http.clone(),
                        url,
                    }) as Arc<dyn TelemetryCapability>
                })
                .or_else(|| {
                    status.clone().map(|status| {
                        Arc::new(StatusTelemetry(status)) as Arc<dyn TelemetryCapability>
                    })
                }),
            logs: endpoint("logs").map(|url| {
                Arc::new(BoardLogs {
                    http: http.clone(),
                    url,
                }) as Arc<dyn LogsCapability>
            }),
            actions,
            selftest,
            status: status.map(|status| status as Arc<dyn StatusCapability>),
            hardware: None,
            slots,
            install: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests;

/// Says, on the health screen and in `atlas doctor`, whether any device
/// package loaded: without one there is no board driver, and a scan finds
/// no boards in a few milliseconds without a word.
pub struct CatalogHealth {
    summary: Result<String, String>,
}

impl CatalogHealth {
    /// `searched`: every place Atlas looked (`atlas_devices::search_report`).
    pub fn new(catalog: &DeviceCatalog, searched: &[String]) -> Self {
        Self {
            summary: catalog.summary(searched),
        }
    }
}

#[async_trait]
impl LinkSource for CatalogHealth {
    async fn links(&self) -> Vec<Link> {
        Vec::new()
    }

    fn name(&self) -> &str {
        "Device packages"
    }

    async fn health(&self) -> Vec<HealthCheck> {
        vec![match &self.summary {
            Ok(detail) => HealthCheck::ok("device-packages", "Device packages", detail.clone()),
            Err(detail) => HealthCheck::error(
                "device-packages",
                "Device packages",
                detail.clone(),
                "Reinstall Atlas, or set ATLAS_DEVICES_DIR to a devices/ folder",
            ),
        }]
    }
}
