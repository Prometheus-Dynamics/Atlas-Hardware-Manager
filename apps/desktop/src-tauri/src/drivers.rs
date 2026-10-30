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
    #[test]
    fn packaged_udev_rule_matches_the_helper() {
        assert_eq!(
            include_str!("../bundle/linux/60-atlas-usbboot.rules"),
            atlas_usbboot::LINUX_UDEV_RULES_FILE
        );
    }
}
