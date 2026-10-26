//! Orion for the desktop app: Atlas's operator identity, the connection,
//! the directory that adds Orion's capabilities to devices, and the image
//! server boards download updates from.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use atlas_driver_orion::{BoardNodes, BundleHost, LiveStreams, OrionDirectory, RemoteTransport};
use atlas_image_server::{ImageServer, ImageServerConfig};

use crate::settings::{AppPaths, AppSettings};

const PIN_FILE: &str = "orion-node.pin";
/// The keys of the boards' own nodes, for their live streams.
const LIVE_PINS: &str = "orion-live-pins";

/// The image server's settings: every IPv4 interface on the chosen port,
/// since boards reach Atlas over the robot network or the USB gadget.
pub fn image_config(settings: &AppSettings) -> ImageServerConfig {
    ImageServerConfig {
        bind: SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), settings.image_server_port),
        host: settings
            .image_host
            .as_deref()
            .map(str::trim)
            .filter(|host| !host.is_empty())
            .map(str::to_string),
        ..ImageServerConfig::default()
    }
}

pub struct Orion {
    pub transport: Arc<RemoteTransport>,
    pub directory: Arc<OrionDirectory>,
    /// Serves update images to boards; Orion carries only the URL.
    pub images: ImageServer,
    /// Boards' live readings, from each board's own node.
    pub live: Arc<LiveStreams>,
    /// Each board's own node: Orion for boards the configured Orion doesn't
    /// know, with no key while their security switch is off.
    pub board_nodes: Arc<BoardNodes>,
}

impl Orion {
    pub fn new(paths: &AppPaths, settings: &AppSettings) -> Result<Self, String> {
        let identity = atlas_driver_orion::operator_identity(&paths.data_dir)?;
        let live = Arc::new(LiveStreams::new(
            identity.clone(),
            Some(paths.data_dir.join(LIVE_PINS)),
        ));
        let transport = Arc::new(RemoteTransport::new(
            identity.clone(),
            settings.orion_url.clone(),
            Some(paths.data_dir.join(PIN_FILE)),
        ));
        let images = ImageServer::new(image_config(settings))
            .with_logger(Arc::new(|line| crate::logfile::line("IMAGES", line)));
        let bundles: Arc<dyn BundleHost> = Arc::new(images.clone());
        let directory = OrionDirectory::new(transport.clone(), Some(bundles.clone()));
        directory.set_live_streams(live.clone());
        let board_nodes = BoardNodes::new(
            identity,
            Some(bundles),
            Some(live.clone()),
            Some(directory.clone()),
        );
        Ok(Self {
            transport,
            directory,
            images,
            live,
            board_nodes,
        })
    }

    /// Starts the image server when Orion is set up; otherwise the first
    /// update through Orion starts it. Call from inside the async runtime.
    /// A failure shows on the health screen and in the Orion settings.
    pub fn start_images_if_configured(&self, settings: &AppSettings) {
        if settings.orion_url.is_some() {
            let _ = self.images.start();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_image_server_listens_everywhere_on_the_chosen_port() {
        let settings = AppSettings {
            image_server_port: 7801,
            image_host: Some("  ".into()),
            ..AppSettings::default()
        };
        let config = image_config(&settings);
        assert_eq!(config.bind.to_string(), "0.0.0.0:7801");
        assert_eq!(config.host, None);
        assert_eq!(AppSettings::default().image_server_port, 7700);
    }
}
