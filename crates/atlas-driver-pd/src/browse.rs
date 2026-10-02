//! One background mDNS browser shared by every PD driver.

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, OnceLock};

use mdns_sd::{ServiceDaemon, ServiceEvent};

pub const SERVICE_TYPE: &str = "_pd-device._tcp.local.";

/// A resolved `_pd-device._tcp` advertisement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Advertisement {
    pub address: IpAddr,
    pub port: u16,
    pub txt: BTreeMap<String, String>,
}

impl Advertisement {
    pub fn model(&self) -> Option<&str> {
        self.txt.get("model").map(String::as_str)
    }

    /// The identity endpoint URL, from the SRV port and TXT `path`.
    pub fn identity_url(&self) -> String {
        let path = self
            .txt
            .get("path")
            .map(String::as_str)
            .filter(|path| path.starts_with('/'))
            .unwrap_or(crate::IDENTITY_PATH);
        let host = match self.address {
            IpAddr::V4(v4) => v4.to_string(),
            IpAddr::V6(v6) => format!("[{v6}]"),
        };
        format!("http://{host}:{}{path}", self.port)
    }
}

type Services = Arc<Mutex<BTreeMap<String, Advertisement>>>;

/// Browses `_pd-device._tcp` in the background and keeps the current set.
pub struct Browser {
    state: Result<Services, String>,
}

impl Browser {
    /// The process-wide browser, started on first use.
    pub fn shared() -> &'static Browser {
        static BROWSER: OnceLock<Browser> = OnceLock::new();
        BROWSER.get_or_init(Browser::start)
    }

    fn start() -> Self {
        let daemon = match ServiceDaemon::new() {
            Ok(daemon) => daemon,
            Err(error) => {
                return Self {
                    state: Err(error.to_string()),
                };
            }
        };
        let receiver = match daemon.browse(SERVICE_TYPE) {
            Ok(receiver) => receiver,
            Err(error) => {
                return Self {
                    state: Err(error.to_string()),
                };
            }
        };
        let services: Services = Arc::default();
        let sink = services.clone();
        let spawned = std::thread::Builder::new()
            .name("atlas-mdns".into())
            .spawn(move || {
                // The daemon must outlive the receiver loop.
                let _daemon = daemon;
                while let Ok(event) = receiver.recv() {
                    let mut map = sink.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                    match event {
                        ServiceEvent::ServiceResolved(service) => {
                            let addresses: Vec<IpAddr> = service
                                .addresses
                                .iter()
                                .map(|scoped| scoped.to_ip_addr())
                                .filter(|ip| !ip.is_loopback())
                                .collect();
                            // Prefer IPv4: link-local IPv6 needs a scope id.
                            let Some(address) = addresses
                                .iter()
                                .find(|ip| ip.is_ipv4())
                                .or(addresses.first())
                                .copied()
                            else {
                                continue;
                            };
                            let txt = service
                                .txt_properties
                                .clone()
                                .into_property_map_str()
                                .into_iter()
                                .map(|(key, value)| (key.to_ascii_lowercase(), value))
                                .collect();
                            map.insert(
                                service.fullname.clone(),
                                Advertisement {
                                    address,
                                    port: service.port,
                                    txt,
                                },
                            );
                        }
                        ServiceEvent::ServiceRemoved(_, fullname) => {
                            map.remove(&fullname);
                        }
                        _ => {}
                    }
                }
            });
        match spawned {
            Ok(_) => Self {
                state: Ok(services),
            },
            Err(error) => Self {
                state: Err(error.to_string()),
            },
        }
    }

    /// Every advertisement currently known.
    pub fn advertisements(&self) -> Result<Vec<Advertisement>, String> {
        match &self.state {
            Ok(services) => Ok(services
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .values()
                .cloned()
                .collect()),
            Err(error) => Err(error.clone()),
        }
    }

    pub fn error(&self) -> Option<&str> {
        self.state.as_ref().err().map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_urls_use_the_srv_port_and_txt_path() {
        let mut ad = Advertisement {
            address: "10.0.0.5".parse().unwrap(),
            port: 5899,
            txt: BTreeMap::new(),
        };
        assert_eq!(
            ad.identity_url(),
            "http://10.0.0.5:5899/.well-known/pd-device"
        );
        ad.txt.insert("path".into(), "/id".into());
        assert_eq!(ad.identity_url(), "http://10.0.0.5:5899/id");
        ad.address = "fe80::1".parse().unwrap();
        assert_eq!(ad.identity_url(), "http://[fe80::1]:5899/id");
    }
}
