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
