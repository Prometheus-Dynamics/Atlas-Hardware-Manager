//! Hardware drivers compiled into the app.

use atlas_core::AtlasBuilder;
use atlas_driver_board::SshAccess;
use atlas_driver_rpi::SshKeys;

use crate::settings::AppPaths;

/// Registers every hardware driver and link source this build includes,
/// as the CLI does, with the app's data folder.
pub fn register_hardware(
    builder: AtlasBuilder,
    paths: &AppPaths,
    ssh_keys: &SshKeys,
    ssh_access: &SshAccess,
    warnings: &mut Vec<String>,
) -> AtlasBuilder {
    atlas_cli::register_hardware(
        builder,
        Some(&paths.data_dir),
        ssh_keys,
        ssh_access,
        warnings,
    )
}

#[cfg(test)]
mod tests {
    /// The rule the Linux packages install must be the one the helper's
    /// one-click fix writes, so both paths give the same access.
    /// The Windows installer binds WinUSB to what the helper's fix binds.
    #[test]
    fn the_windows_installer_binds_every_boot_id() {
        let hooks = include_str!("../bundle/windows/hooks.nsh");
        let pi = atlas_usbboot::BOOT_PRODUCT_IDS.map(|pid| ("Raspberry Pi USB boot", 0x0a5c, pid));
        for (name, vid, pid) in pi.into_iter().chain(atlas_usbboot::SOC_BOOT_IDS) {
            let line = format!("ATLAS_BIND_WINUSB \"{name}\" 0x{vid:04x} 0x{pid:04x}");
            assert!(hooks.contains(&line), "{line}");
        }
    }

    #[test]
    fn packaged_udev_rule_matches_the_helper() {
        assert_eq!(
            include_str!("../bundle/linux/60-atlas-usbboot.rules"),
            atlas_usbboot::LINUX_UDEV_RULES_FILE
        );
    }
}
