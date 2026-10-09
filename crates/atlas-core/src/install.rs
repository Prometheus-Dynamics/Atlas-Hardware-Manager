//! Where installers put the files the app and the `atlas` CLI carry: the
//! Pi USB boot files (`usbboot/`) and the device packages (`devices/`).

use std::path::PathBuf;

/// Folders that may hold the bundled USB boot files, relative to the running
/// executable: next to it (Windows, dev), `../Resources` (macOS),
/// `../lib/<app>` (Linux packages, which install the app and the CLI in
/// `/usr/bin`).
pub fn bundled_usbboot_dirs() -> Vec<PathBuf> {
    let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(PathBuf::from))
    else {
        return Vec::new();
    };
    let mut dirs = vec![
        exe_dir.join("usbboot"),
        exe_dir.join("resources").join("usbboot"),
        exe_dir.join("../Resources/usbboot"),
    ];
    for name in [
        "atlas-hardware-manager",
        "Atlas Hardware Manager",
        "atlas-app",
    ] {
        dirs.push(exe_dir.join("../lib").join(name).join("usbboot"));
    }
    dirs
}

/// Folders that may hold bundled device packages: `devices/` next to each
/// bundled `usbboot/`.
pub fn bundled_device_dirs() -> Vec<PathBuf> {
    bundled_usbboot_dirs()
        .into_iter()
        .filter_map(|dir| dir.parent().map(|parent| parent.join("devices")))
        .collect()
}
