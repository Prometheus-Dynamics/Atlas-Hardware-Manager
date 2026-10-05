use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use atlas_devices::{DeviceCatalog, DevicePackage};
use atlas_driver::{
    ActionsCapability, Candidate, Capabilities, DeviceKey, Driver, DriverError, DriverManifest,
    Family, HealthCheck, Identity, Link, LinkId, LinkKind, LinkSource, LogsCapability,
    TelemetryCapability,
};

use crate::browse::Browser;
use crate::contract::PdIdentity;
use crate::live::{PdActions, PdLogs, PdTelemetry, resolve};

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
                    "Listening for PD devices with mDNS; {} advertised right now.",
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
pub struct PdDriver {
    manifest: DriverManifest,
    package: Arc<DevicePackage>,
    http: reqwest::Client,
    /// What each device last reported, for the optional live endpoints.
    reported: Mutex<HashMap<DeviceKey, (String, PdIdentity)>>,
}

impl PdDriver {
    pub fn new(package: DevicePackage) -> Self {
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
        }
    }

    fn reported(&self, key: &DeviceKey) -> Option<(String, PdIdentity)> {
        self.reported
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(key)
            .cloned()
    }

    async fn fetch(&self, url: &str) -> Result<PdIdentity, String> {
        self.http
            .get(url)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|error| error.to_string())?
            .json::<PdIdentity>()
            .await
            .map_err(|error| error.to_string())
    }
}

/// A driver for every model in the catalog.
pub fn drivers_for_catalog(catalog: &DeviceCatalog) -> Vec<Arc<dyn Driver>> {
    catalog
        .packages()
        .iter()
        .map(|package| Arc::new(PdDriver::new(package.clone())) as Arc<dyn Driver>)
        .collect()
}

#[async_trait]
impl Driver for PdDriver {
    fn manifest(&self) -> &DriverManifest {
        &self.manifest
    }

    async fn discover(&self, link: &Link) -> Result<Vec<Candidate>, DriverError> {
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

    async fn identify(&self, candidate: &Candidate) -> Result<Identity, DriverError> {
        let reported = match self.fetch(&candidate.address).await {
            Ok(reported) => reported,
            Err(error) => Browser::shared()
                .advertisements()
                .unwrap_or_default()
                .into_iter()
                .find(|ad| ad.identity_url() == candidate.address)
                .and_then(|ad| PdIdentity::from_txt(&ad.txt))
                .ok_or_else(|| {
                    DriverError::Unreachable(format!("{}: {error}", candidate.address))
                })?,
        };
        if !reported
            .model
            .eq_ignore_ascii_case(self.manifest.family.as_str())
        {
            return Err(DriverError::Other(format!(
                "{} reports model {}, but advertised {}",
                candidate.address, reported.model, self.manifest.family
            )));
        }
        let mut identity = reported.to_identity(
            Some(&self.package),
            candidate.link.clone(),
            candidate.address.clone(),
        );
        if let Some(tty) = crate::serial::serial_console(&reported.serial) {
            identity.attributes.insert("serial_console".into(), tty);
        }
        self.reported
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(identity.key.clone(), (candidate.address.clone(), reported));
        Ok(identity)
    }

    /// Updates for running devices come from OS adapters (HeliOS OTA and
    /// others) as they land; reflashing goes through USB boot recovery.
    /// Metrics, logs, and actions are offered when the device lists them.
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
        Capabilities {
            telemetry: endpoint("metrics").map(|url| {
                Arc::new(PdTelemetry {
                    http: http.clone(),
                    url,
                }) as Arc<dyn TelemetryCapability>
            }),
            logs: endpoint("logs").map(|url| {
                Arc::new(PdLogs {
                    http: http.clone(),
                    url,
                }) as Arc<dyn LogsCapability>
            }),
            actions: endpoint("actions")
                .filter(|_| !reported.actions.is_empty())
                .map(|url| {
                    Arc::new(PdActions {
                        http: http.clone(),
                        url,
                        actions: reported.actions.iter().map(|a| a.to_action()).collect(),
                    }) as Arc<dyn ActionsCapability>
                }),
            ..Capabilities::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    fn package() -> DevicePackage {
        DevicePackage {
            manifest: serde_json::from_str(
                r#"{ "contract": 1, "model": "raze", "display_name": "Raze" }"#,
            )
            .unwrap(),
            compat: Default::default(),
            dir: std::path::PathBuf::from("/nonexistent"),
        }
    }

    /// Serves one HTTP response with `body`, then closes.
    async fn serve_once(body: &'static str) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 1024];
            let _ = socket.read(&mut request).await;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes()).await;
        });
        format!("http://{address}/.well-known/pd-device")
    }

    #[tokio::test]
    async fn identify_reads_the_identity_endpoint() {
        let url = serve_once(
            r#"{ "contract": 1, "model": "Raze", "serial": "ABC", "os": { "name": "helios", "version": "2026.3.1" } }"#,
        )
        .await;
        let driver = PdDriver::new(package());

        let identity = driver
            .identify(&Candidate {
                link: LinkId("mdns".into()),
                family: Family::new("raze"),
                address: url,
            })
            .await
            .unwrap();

        assert_eq!(identity.key.to_string(), "raze:abc");
        assert_eq!(identity.model, "Raze");
        assert_eq!(identity.attributes["os"], "helios");
    }

    #[tokio::test]
    async fn listed_endpoints_become_capabilities() {
        let url = serve_once(
            r#"{ "model": "raze", "serial": "1",
                 "endpoints": { "metrics": "/api/metrics", "actions": "/api/actions" },
                 "actions": [ { "id": "locate", "label": "Find it" } ],
                 "camera_stream": "/stream.mjpg" }"#,
        )
        .await;
        let driver = PdDriver::new(package());
        let identity = driver
            .identify(&Candidate {
                link: LinkId("mdns".into()),
                family: Family::new("raze"),
                address: url.clone(),
            })
            .await
            .unwrap();

        let capabilities = driver.capabilities(&identity);
        assert!(capabilities.telemetry.is_some());
        assert!(capabilities.logs.is_none());
        let actions = capabilities.actions.unwrap().actions(&identity);
        assert_eq!(actions[0].label, "Find it");
        assert!(identity.attributes["camera_stream"].ends_with("/stream.mjpg"));
    }

    #[tokio::test]
    async fn a_plain_device_offers_no_extras() {
        let url = serve_once(r#"{ "model": "raze", "serial": "2" }"#).await;
        let driver = PdDriver::new(package());
        let identity = driver
            .identify(&Candidate {
                link: LinkId("mdns".into()),
                family: Family::new("raze"),
                address: url,
            })
            .await
            .unwrap();
        let capabilities = driver.capabilities(&identity);
        assert_eq!(
            capabilities.kinds(),
            vec![atlas_driver::CapabilityKind::Info]
        );
    }

    #[tokio::test]
    async fn a_different_model_is_rejected() {
        let url = serve_once(r#"{ "model": "other", "serial": "1" }"#).await;
        let driver = PdDriver::new(package());

        let result = driver
            .identify(&Candidate {
                link: LinkId("mdns".into()),
                family: Family::new("raze"),
                address: url,
            })
            .await;

        assert!(matches!(result, Err(DriverError::Other(_))));
    }
}
