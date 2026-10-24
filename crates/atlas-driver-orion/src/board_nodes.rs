//! Each board's own orion-node, as Atlas reached the board: Orion's slots,
//! updates and actions for a camera without an Orion set up in Atlas, and
//! without an SSH key.
//!
//! A board's node listens on `orion+tcp://<board>:9200` and, while the
//! board's security switch is off, enrolls an operator on its hello from the
//! USB cable's network and the robot network for updates and slots
//! (devices/raze `orion-env`). So for every board device whose node the
//! configured Orion doesn't already know, Atlas keeps a directory of that
//! one node: its first identify of the board starts it (and finds the node
//! a moment later), each scan refreshes it, and from then on the board gets
//! Orion's capabilities as from the configured Orion: Orion first, the
//! board's own ways (USB console, SSH) when Orion can't.
//!
//! The node's key is trusted on first use per session (no pin kept): a
//! reflashed board gets a new key, and nothing here is more than the board
//! grants anyone on that network with its switch off.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use atlas_driver::attributes::BOARD_SERIAL;
use atlas_driver::{BundleHost, Capabilities, CapabilitySource, Identity, Link, LinkSource};
use orion_client::remote::OperatorIdentity;

use crate::directory::OrionDirectory;
use crate::live::LiveStreams;
use crate::remote::RemoteTransport;

/// How long a scan waits for one board's node.
const REFRESH_WITHIN: std::time::Duration = std::time::Duration::from_secs(3);

/// Boards' own nodes, by `orion+tcp://` URL.
pub struct BoardNodes {
    identity: OperatorIdentity,
    bundles: Option<Arc<dyn BundleHost>>,
    live: Option<Arc<LiveStreams>>,
    /// The configured Orion: boards it knows are left to it.
    configured: Option<Arc<OrionDirectory>>,
    /// The nodes' `orion+tcp` port (9200; tests use another).
    port: u16,
    nodes: RwLock<HashMap<String, Arc<OrionDirectory>>>,
}

impl BoardNodes {
    pub fn new(
        identity: OperatorIdentity,
        bundles: Option<Arc<dyn BundleHost>>,
        live: Option<Arc<LiveStreams>>,
        configured: Option<Arc<OrionDirectory>>,
    ) -> Arc<Self> {
        Self::on_port(
            identity,
            bundles,
            live,
            configured,
            crate::live::DEFAULT_PORT,
        )
    }

    /// [`new`](Self::new), with the boards' nodes on `port`.
    pub fn on_port(
        identity: OperatorIdentity,
        bundles: Option<Arc<dyn BundleHost>>,
        live: Option<Arc<LiveStreams>>,
        configured: Option<Arc<OrionDirectory>>,
        port: u16,
    ) -> Arc<Self> {
        Arc::new(Self {
            identity,
            bundles,
            live,
            configured,
            port,
            nodes: RwLock::new(HashMap::new()),
        })
    }

    /// The node of the board at `address` (its identity URL).
    fn url_for(&self, address: &str) -> String {
        let url = LiveStreams::url_for(address);
        match url.rsplit_once(':') {
            Some((host, _)) => format!("{host}:{}", self.port),
            None => url,
        }
    }

    /// The directory of the node at `url`, made (and refreshed in the
    /// background) the first time.
    fn directory(&self, url: &str) -> Arc<OrionDirectory> {
        if let Some(found) = self
            .nodes
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(url)
        {
            return found.clone();
        }
        let transport = Arc::new(RemoteTransport::new(
            self.identity.clone(),
            Some(url.to_owned()),
            None,
        ));
        let directory = OrionDirectory::new(transport, self.bundles.clone());
        if let Some(live) = &self.live {
            directory.set_live_streams(live.clone());
        }
        let made = self
            .nodes
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entry(url.to_owned())
            .or_insert(directory)
            .clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let refreshing = made.clone();
            runtime.spawn(async move {
                let _ = refreshing.refresh().await;
            });
        }
        made
    }
}

/// Whether `device` is a board Atlas reached over HTTP (its identity
/// endpoint), the only kind with a node of its own at that address.
fn board_address(device: &Identity) -> Option<&str> {
    device.attributes.get(BOARD_SERIAL)?;
    let address = device.address.as_str();
    (address.starts_with("http://") || address.starts_with("https://")).then_some(address)
}

#[async_trait]
impl LinkSource for BoardNodes {
    /// Contributes no links: refreshes each node before a scan.
    async fn links(&self) -> Vec<Link> {
        let nodes: Vec<Arc<OrionDirectory>> = self
            .nodes
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .values()
            .cloned()
            .collect();
        // A board that is away doesn't hold the scan up.
        futures::future::join_all(
            nodes
                .iter()
                .map(|node| tokio::time::timeout(REFRESH_WITHIN, node.refresh())),
        )
        .await;
        Vec::new()
    }

    fn name(&self) -> &str {
        "Boards' Orion"
    }
}

impl CapabilitySource for BoardNodes {
    fn data_first(&self) -> bool {
        true
    }

    fn capabilities_for(&self, device: &Identity) -> Capabilities {
        let Some(address) = board_address(device) else {
            return Capabilities::default();
        };
        if self
            .configured
            .as_ref()
            .is_some_and(|configured| configured.node_for(device).is_some())
        {
            return Capabilities::default();
        }
        self.directory(&self.url_for(address))
            .capabilities_for(device)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_boards_reached_over_http_have_a_node_there() {
        let mut device = Identity {
            key: atlas_driver::DeviceKey::new("raze", "6acf6699775f28dd"),
            model: "Raze".into(),
            mode: atlas_driver::DeviceMode::Normal,
            versions: Default::default(),
            name: None,
            link: atlas_driver::LinkId("usb-gadget".into()),
            address: "http://172.31.97.209:5899/.well-known/pd-device".into(),
            attributes: Default::default(),
        };
        assert_eq!(board_address(&device), None, "no board serial: not a board");
        device
            .attributes
            .insert(BOARD_SERIAL.into(), "6acf6699775f28dd".into());
        assert!(board_address(&device).is_some());
        assert_eq!(
            LiveStreams::url_for(board_address(&device).unwrap()),
            "orion+tcp://172.31.97.209:9200"
        );
        device.address = "usb:1-4".into();
        assert_eq!(board_address(&device), None);
    }
}
