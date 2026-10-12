use std::path::{Path, PathBuf};

use atlas_usbboot::BootFiles;

const GADGET_DIR: &str = "mass-storage-gadget64";

/// Places the mass-storage-gadget boot files may live, most specific first:
/// `ATLAS_USBBOOT_DIR`, the app's bundled resources, Atlas's data folder,
/// then where the `rpiboot` package installs them.
pub(crate) fn candidates(extra: &[PathBuf]) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = std::env::var_os("ATLAS_USBBOOT_DIR") {
        dirs.push(PathBuf::from(dir));
    }
    for base in extra {
        dirs.push(base.join(GADGET_DIR));
        dirs.push(base.clone());
    }
    if let Some(data) = std::env::var_os("ATLAS_DATA_DIR") {
        dirs.push(PathBuf::from(data).join("usbboot").join(GADGET_DIR));
    }
    for system in [
        "/usr/share/rpiboot",
        "/usr/local/share/rpiboot",
        "/opt/homebrew/share/rpiboot",
    ] {
        dirs.push(Path::new(system).join(GADGET_DIR));
    }
    dirs
}

/// The first usable boot files directory, or `None` with the places tried.
pub fn find_boot_files(extra: &[PathBuf]) -> Result<BootFiles, Vec<PathBuf>> {
    let tried = candidates(extra);
    for dir in &tried {
        if dir.join("boot.img").is_file()
            && let Ok(files) = BootFiles::open(dir)
        {
            return Ok(files);
        }
    }
    Err(tried)
}

const FLASHER_DIR: &str = "raze-flasher";

/// Places the raze-flasher bundle (Gaia's `raze-flasher-<version>.tar`,
/// unpacked: `bootfiles/2712/`, `config.txt`, `boot.img`) may live:
/// `ATLAS_FLASHER_DIR`, then `raze-flasher/` next to the mass-storage files.
pub(crate) fn flasher_candidates(extra: &[PathBuf]) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = std::env::var_os("ATLAS_FLASHER_DIR") {
        dirs.push(PathBuf::from(dir));
    }
    for base in extra {
        dirs.push(base.join(FLASHER_DIR));
    }
    if let Some(data) = std::env::var_os("ATLAS_DATA_DIR") {
        dirs.push(PathBuf::from(data).join("usbboot").join(FLASHER_DIR));
    }
    dirs
}

/// The raze-flasher boot files, unless there are none or `ATLAS_FLASHER=off`
/// asks for the mass-storage path.
pub fn find_flasher_files(extra: &[PathBuf]) -> Option<BootFiles> {
    if std::env::var("ATLAS_FLASHER").is_ok_and(|value| value.eq_ignore_ascii_case("off")) {
        return None;
    }
    flasher_candidates(extra)
        .into_iter()
        .filter(|dir| dir.join("boot.img").is_file())
        .find_map(|dir| BootFiles::open(&dir).ok())
}
