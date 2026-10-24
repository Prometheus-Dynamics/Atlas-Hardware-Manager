//! Where devices pull release files from.

use std::net::IpAddr;

use crate::{Artifact, DriverError, HealthCheck};

/// Makes a release file reachable by URL for devices to pull. Update bytes
/// never travel over Orion; the device downloads the image from Atlas.
pub trait BundleHost: Send + Sync {
    /// A URL the device at `peer` (when its address is known) can fetch
    /// `artifact` from: the host must be an address of this computer that
    /// routes to the device.
    fn url_for(&self, artifact: &Artifact, peer: Option<IpAddr>) -> Result<String, DriverError>;

    /// Whether devices can reach the host, for the health screen.
    fn health(&self) -> Option<HealthCheck> {
        None
    }
}

/// A device's reason for a pull that failed before anything was installed
/// (the slots writer's "downloading <url> failed: …", or "the download
/// failed" from device packages before the reason was added): another way
/// in may still work.
pub fn download_failed(message: &str) -> bool {
    let message = message.trim_start_matches("slots: ");
    message.starts_with("the download failed")
        || (message.starts_with("downloading ") && message.contains(" failed"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_download_is_told_from_a_failed_install() {
        assert!(download_failed("the download failed"));
        assert!(download_failed("slots: the download failed"));
        assert!(download_failed(
            "downloading http://10.0.0.2:8099/i/ab.slot.tar failed: Failed to connect to 10.0.0.2 port 8099"
        ));
        assert!(!download_failed("the download doesn't match --sha256"));
        assert!(!download_failed(
            "rootfs.erofs doesn't match slot.json's sha256"
        ));
    }
}
