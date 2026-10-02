//! Host readiness for USB boot and image writes.

use atlas_blockdev::HelperClient;
use atlas_driver::{DriverError, HealthCheck};
use atlas_usbboot::list_boot_devices;

/// Fix action: install the udev rule (Linux) or WinUSB driver (Windows).
pub(crate) const INSTALL_USB_ACCESS: &str = "usbboot.install-access";

/// Runs [`INSTALL_USB_ACCESS`] through the elevated helper.
pub(crate) async fn fix(action: &str) -> Result<String, DriverError> {
    if action != INSTALL_USB_ACCESS {
        return Err(DriverError::Unsupported(format!("no fix named `{action}`")));
    }
    tokio::task::spawn_blocking(|| {
        HelperClient::locate().and_then(|helper| helper.install_usb_access())
    })
    .await
    .map_err(|error| DriverError::Other(error.to_string()))?
    .map_err(|error| match error {
        atlas_blockdev::BlockError::Elevation { message, fix } => {
            DriverError::Other(format!("{message} {fix}"))
        }
        other => DriverError::Other(other.to_string()),
    })
}

use crate::RpiConfig;
use crate::boot_files::find_boot_files;

pub(crate) async fn usb_access() -> HealthCheck {
    match list_boot_devices().await {
        Ok(devices) if devices.iter().any(|device| device.needs_driver) => HealthCheck::error(
            "usb.driver",
            "USB boot driver",
            "A Pi is in USB boot mode, but Windows has no WinUSB driver bound to it.",
            "Press Fix to install the driver (one administrator prompt), then replug the board.",
        )
        .with_fix_action(INSTALL_USB_ACCESS),
        Ok(devices) => HealthCheck::ok(
            "usb.access",
            "USB access",
            match devices.len() {
                0 => "USB devices can be listed. No Pi is in USB boot mode right now.".to_string(),
                count => format!("USB devices can be listed. {count} Pi in USB boot mode."),
            },
        ),
        Err(error) => HealthCheck::error(
            "usb.access",
            "USB access",
            error.to_string(),
            error
                .fix()
                .unwrap_or("Reconnect the device and check the USB cable.")
                .to_string(),
        ),
    }
}

#[cfg(target_os = "linux")]
fn udev_check() -> Option<HealthCheck> {
    let dirs = [
        "/etc/udev/rules.d",
        "/usr/lib/udev/rules.d",
        "/lib/udev/rules.d",
    ];
    let found = dirs.iter().any(|dir| {
        std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .any(|entry| {
                std::fs::read_to_string(entry.path())
                    .is_ok_and(|text| text.contains("0a5c") && text.contains("2711"))
            })
    });
    Some(if found {
        HealthCheck::ok(
            "usbboot.udev",
            "USB boot permissions",
            "A udev rule gives this user access to Pi boot devices.",
        )
    } else {
        HealthCheck::warning(
            "usbboot.udev",
            "USB boot permissions",
            "No udev rule for Pi boot devices was found, so USB boot would need root.",
            "Press Fix to install it (one password prompt), then replug the board.",
        )
        .with_fix_action(INSTALL_USB_ACCESS)
    })
}

#[cfg(not(target_os = "linux"))]
fn udev_check() -> Option<HealthCheck> {
    None
}

pub(crate) async fn driver_checks(config: &RpiConfig) -> Vec<HealthCheck> {
    let mut checks = Vec::new();
    checks.push(match find_boot_files(&config.boot_file_dirs) {
        Ok(files) => HealthCheck::ok(
            "usbboot.files",
            "USB boot files",
            format!(
                "Mass-storage-gadget files found in {}.",
                files.dir().display()
            ),
        ),
        Err(tried) => HealthCheck::error(
            "usbboot.files",
            "USB boot files",
            format!(
                "No mass-storage-gadget boot files found. Looked in: {}.",
                tried
                    .iter()
                    .map(|dir| dir.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            "Reinstall Atlas, or run scripts/fetch-usbboot-files.sh and set ATLAS_USBBOOT_DIR.",
        ),
    });
    checks.push(match HelperClient::locate() {
        Ok(helper) => HealthCheck::ok(
            "blockdev.helper",
            "Disk writer",
            format!("atlas-helper found at {}.", helper.path().display()),
        ),
        Err(error) => HealthCheck::error(
            "blockdev.helper",
            "Disk writer",
            error.to_string(),
            "Reinstall Atlas, or set ATLAS_HELPER to the atlas-helper binary.",
        ),
    });
    checks.extend(udev_check());
    checks
}
