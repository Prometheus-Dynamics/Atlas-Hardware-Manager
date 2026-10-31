//! Atlas's USB agent on a SystemCore (`atlas-usb-agent`, from its `.ipk`):
//! how Atlas reaches the boards plugged into the controller's USB, as
//! Phoenix Tuner reaches CAN devices through the server it installs on a
//! roboRIO.
//!
//! - Whether it runs, and which version: `GET :5809/v1/agent`, as the
//!   SystemCore's `usb_agent` attribute.
//! - Installing or upgrading it is one action, `usb-agent.install`: the
//!   `.ipk` Atlas carries goes through the SystemCore's package manager
//!   (`crate::package`, by SSH key).
//! - The Raspberry Pi boards in USB boot behind it (`GET :5809/v1/usb`)
//!   are the SystemCore's children, for the rpi driver to flash through it.

use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use atlas_driver::{
    ActionsCapability, Artifact, ArtifactKind, CancellationToken, Candidate, DeviceAction,
    DriverError, Family, Identity, InstallMethod, InstallRequest, LinkId, ProgressSink,
};
use atlas_usb_agent::{AgentInfo, PORT, UsbDevice, UsbKind};

/// The rpi driver's family: boards in USB boot behind the agent are its.
const RPI_FAMILY: &str = "rpi";
/// The SystemCore attribute with the running agent's version.
pub const ATTRIBUTE: &str = "usb_agent";
/// The SystemCore attribute with the version of the agent Atlas carries,
/// when it differs from the running one (or none runs).
pub const OFFERED: &str = "usb_agent.offered";
pub const INSTALL: &str = "usb-agent.install";
const ASK_TIMEOUT: Duration = Duration::from_millis(800);

/// Where the agent's `.ipk` may be: `ATLAS_USB_AGENT_IPK`, else the newest
/// `atlas-usb-agent_*_aarch64.ipk` in any of `dirs` (the app's `usbboot/`
/// resources, its data folder's) or their `atlas-usb-agent/`, where the
/// installers carry it.
pub fn package(dirs: &[PathBuf]) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("ATLAS_USB_AGENT_IPK").map(PathBuf::from)
        && path.is_file()
    {
        return Some(path);
    }
    dirs.iter()
        .flat_map(|dir| [dir.clone(), dir.join("atlas-usb-agent")])
        .flat_map(|dir| std::fs::read_dir(dir).into_iter().flatten().flatten())
        .map(|entry| entry.path())
        .filter(|path| package_version(path).is_some())
        .max_by_key(|path| std::fs::metadata(path).and_then(|m| m.modified()).ok())
}

/// The agent version a package carries, as the agent reports it:
/// `atlas-usb-agent_2026.1.0~dev.408+7c31465b_aarch64.ipk` is
/// `2026.1.0-dev.408+7c31465b`.
pub fn package_version(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    let version = name
        .strip_prefix("atlas-usb-agent_")?
        .strip_suffix("_aarch64.ipk")?;
    (!version.is_empty()).then(|| version.replacen('~', "-", 1))
}

async fn get<T: serde::de::DeserializeOwned>(
    http: &reqwest::Client,
    ip: Ipv4Addr,
    path: &str,
) -> Option<T> {
    http.get(format!("http://{ip}:{PORT}{path}"))
        .timeout(ASK_TIMEOUT)
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json()
        .await
        .ok()
}

/// The running agent, if one answers.
pub async fn info(http: &reqwest::Client, ip: Ipv4Addr) -> Option<AgentInfo> {
    get(http, ip, "/v1/agent").await
}

/// The Raspberry Pi boards behind the agent at `ip`: in USB boot, or
/// showing their eMMC.
pub async fn boards(http: &reqwest::Client, ip: Ipv4Addr, link: &LinkId) -> Vec<Candidate> {
    let devices: Vec<UsbDevice> = get(http, ip, "/v1/usb").await.unwrap_or_default();
    devices
        .into_iter()
        .filter(|device| matches!(device.kind, UsbKind::UsbBoot | UsbKind::Gadget))
        .map(|device| Candidate {
            link: link.clone(),
            family: Family::new(RPI_FAMILY),
            address: atlas_usb_agent::board_address(&format!("{ip}:{PORT}"), &device.port),
        })
        .collect()
}

/// Installs or upgrades the agent from the package Atlas carries.
pub struct AgentActions {
    pub ip: Ipv4Addr,
    pub package: Option<PathBuf>,
}

#[async_trait]
impl ActionsCapability for AgentActions {
    fn actions(&self, device: &Identity) -> Vec<DeviceAction> {
        let Some(offered) = device.attributes.get(OFFERED) else {
            return Vec::new();
        };
        let label = match device.attributes.get(ATTRIBUTE) {
            Some(running) => format!("Update Atlas's USB agent ({running} to {offered})"),
            None => format!("Install Atlas's USB agent ({offered})"),
        };
        vec![DeviceAction {
            id: INSTALL.into(),
            label,
            destructive: false,
        }]
    }

    async fn run_action(&self, device: &Identity, action_id: &str) -> Result<(), DriverError> {
        if action_id != INSTALL {
            return Err(DriverError::Unsupported(action_id.into()));
        }
        let path = self.package.clone().ok_or_else(|| {
            DriverError::Incompatible(
                "this Atlas doesn't carry the USB agent's package (atlas-usb-agent_*_aarch64.ipk)"
                    .into(),
            )
        })?;
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let size_bytes = std::fs::metadata(&path)
            .map(|meta| meta.len())
            .map_err(|error| DriverError::Other(format!("{}: {error}", path.display())))?;
        let request = InstallRequest {
            artifact: Artifact {
                name,
                path: path.clone(),
                sha256: String::new(),
                size_bytes,
            },
            kind: ArtifactKind::Ipk,
            version: package_version(&path).unwrap_or_default(),
            options: Default::default(),
        };
        let method = crate::package::PackageMethod { ip: self.ip };
        InstallMethod::run(
            &method,
            device,
            &request,
            &ProgressSink::new(|_| {}),
            &CancellationToken::new(),
        )
        .await
        .map(|_| ())
        .map_err(|failure| failure.error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_package_names_its_version_as_the_agent_reports_it() {
        let path = Path::new("/x/atlas-usb-agent_2026.1.0~dev.408+7c31465b_aarch64.ipk");
        assert_eq!(
            package_version(path).as_deref(),
            Some("2026.1.0-dev.408+7c31465b")
        );
        assert_eq!(
            package_version(Path::new("/x/atlas-usb-agent_2026.1.0_aarch64.ipk")).as_deref(),
            Some("2026.1.0")
        );
        assert_eq!(package_version(Path::new("/x/helios_1_aarch64.ipk")), None);
    }

    #[test]
    fn the_action_offers_install_or_update() {
        let actions = AgentActions {
            ip: Ipv4Addr::LOCALHOST,
            package: None,
        };
        let mut device = crate::systemcore::identity(
            Some(6390),
            Ipv4Addr::LOCALHOST,
            LinkId("usb".into()),
            None,
        );
        assert!(actions.actions(&device).is_empty());
        device.attributes.insert(OFFERED.into(), "2".into());
        assert_eq!(
            actions.actions(&device)[0].label,
            "Install Atlas's USB agent (2)"
        );
        device.attributes.insert(ATTRIBUTE.into(), "1".into());
        assert_eq!(
            actions.actions(&device)[0].label,
            "Update Atlas's USB agent (1 to 2)"
        );
    }
}
