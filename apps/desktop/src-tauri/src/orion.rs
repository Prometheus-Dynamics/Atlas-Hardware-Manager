//! Orion for the desktop app: Atlas's operator identity, the connection,
//! the directory that adds Orion's capabilities to devices, and the image
//! server boards download updates from.

use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::Arc;

use atlas_driver_orion::{BundleHost, OperatorIdentity, OrionDirectory, RemoteTransport};
use atlas_image_server::{ImageServer, ImageServerConfig};

use crate::settings::{AppPaths, AppSettings};

const KEY_FILE: &str = "orion-operator.key";
const PIN_FILE: &str = "orion-node.pin";

/// `atlas-<hostname>`, lowercase, safe for an operator name.
fn operator_name() -> String {
    let host = std::fs::read_to_string("/etc/hostname")
        .or_else(|_| std::fs::read_to_string("/proc/sys/kernel/hostname"))
        .ok()
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_default();
    let host: String = host
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let host = host.trim_matches('-');
    if host.is_empty() {
        "atlas".into()
    } else {
        format!("atlas-{host}")
    }
}

/// Loads Atlas's operator key, creating it (0600) the first time.
fn identity(dir: &Path) -> Result<OperatorIdentity, String> {
    let file = dir.join(KEY_FILE);
    let name = operator_name();
    if let Ok(bytes) = std::fs::read(&file) {
        return OperatorIdentity::from_secret_key_bytes(&name, &bytes)
            .map_err(|error| format!("{}: {error}", file.display()));
    }
    let identity = OperatorIdentity::generate(&name).map_err(|error| error.to_string())?;
    std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    write_private(&file, &identity.secret_key_bytes())?;
    Ok(identity)
}

/// Creates `file` readable only by this user (0600 on Unix; on Windows the
/// per-user data directory's ACL already restricts it).
fn write_private(file: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut handle = options
        .open(file)
        .map_err(|error| format!("{}: {error}", file.display()))?;
    handle
        .write_all(bytes)
        .map_err(|error| format!("{}: {error}", file.display()))
}

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
}

impl Orion {
    pub fn new(paths: &AppPaths, settings: &AppSettings) -> Result<Self, String> {
        let identity = identity(&paths.data_dir)?;
        let transport = Arc::new(RemoteTransport::new(
            identity,
            settings.orion_url.clone(),
            Some(paths.data_dir.join(PIN_FILE)),
        ));
        let images = ImageServer::new(image_config(settings))
            .with_logger(Arc::new(|line| crate::logfile::line("IMAGES", line)));
        let bundles: Arc<dyn BundleHost> = Arc::new(images.clone());
        let directory = OrionDirectory::new(transport.clone(), Some(bundles));
        Ok(Self {
            transport,
            directory,
            images,
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
    fn the_operator_key_is_created_once_and_kept_private() {
        let dir = std::env::temp_dir().join(format!("atlas-orion-{}", std::process::id()));
        let first = identity(&dir).unwrap();
        let again = identity(&dir).unwrap();
        assert_eq!(first.fingerprint(), again.fingerprint());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join(KEY_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert!(operator_name().starts_with("atlas"));
        let _ = std::fs::remove_dir_all(dir);
    }

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
