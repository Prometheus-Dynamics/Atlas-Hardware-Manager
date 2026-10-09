use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use atlas_devices::{DeviceCatalog, DevicePackage};
use atlas_driver::{
    ActionsCapability, Candidate, Capabilities, DeviceKey, Driver, DriverError, DriverManifest,
    Family, HealthCheck, Identity, Link, LinkId, LinkKind, LinkSource, LogsCapability,
    SelfTestCapability, StatusCapability, TelemetryCapability, UpdateCapability,
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
    /// What each device last reported, for the optional live endpoints.
    reported: Mutex<HashMap<DeviceKey, (String, BoardIdentity)>>,
    /// The last discovery's USB gadget probes, by candidate address.
    probes: Mutex<HashMap<String, GadgetProbe>>,
    ssh: SshAccess,
}

impl BoardDriver {
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

    fn reported(&self, key: &DeviceKey) -> Option<(String, BoardIdentity)> {
        self.reported
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(key)
            .cloned()
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
        let advertisements = Browser::shared()
            .advertisements()
            .map_err(DriverError::Unreachable)?;
        Ok(advertisements
            .into_iter()
            .filter(|ad| {
                ad.model()
                    .is_some_and(|model| model.eq_ignore_ascii_case(self.manifest.family.as_str()))
            })
            .map(|ad| Candidate {
                link: link.id.clone(),
                family: self.manifest.family.clone(),
                address: ad.identity_url(),
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
        if let Some(tty) = crate::serial::serial_console(&reported.serial) {
            identity.attributes.insert("serial_console".into(), tty);
        }
        self.reported
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(identity.key.clone(), (address, reported));
        Ok(identity)
    }

    /// Boards listing `ab-tryboot` update over SSH; reflashing goes through
    /// USB boot recovery. Metrics, logs, actions, status and events are
    /// offered when the device lists them; without a metrics endpoint, the
    /// status's temperatures and fan are the telemetry.
    fn capabilities(&self, device: &Identity) -> Capabilities {
        let Some((identity_url, reported)) = self.reported(&device.key) else {
            return Capabilities::default();
        };
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
            })
        };
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
            Arc::new(BoardStatus {
                http: http.clone(),
                status_url,
                events_url: endpoint("events"),
            })
        });
        // A board with the device package's writer, self-test or status is
        // one Atlas reaches as root over SSH (the Flash tab's key).
        let package_board = has(AB_METHOD) || !reported.diagnostics.is_empty() || status.is_some();
        let offer = SshOffer {
            usb_boot: has(USB_BOOT_METHOD),
            set_clock: device
                .attributes
                .contains_key(atlas_driver::attributes::CLOCK_OFFSET_S),
            power: package_board,
            updates: has(AB_METHOD),
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
        }
    }
}

#[cfg(test)]
mod tests;
