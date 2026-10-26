use std::path::{Path, PathBuf};

use crate::{CONTRACT, CompatList, DeviceManifest};

/// One `devices/<model>/` folder.
#[derive(Clone, Debug, PartialEq)]
pub struct DevicePackage {
    pub manifest: DeviceManifest,
    pub compat: CompatList,
    pub dir: PathBuf,
}

impl DevicePackage {
    /// The folder holding EEPROM files (`recovery.bin`, `pieeprom.bin`, ...),
    /// when the package ships one.
    pub fn eeprom_dir(&self) -> Option<PathBuf> {
        let dir = self.dir.join("eeprom");
        dir.is_dir().then_some(dir)
    }
}

/// Every device package Atlas found.
#[derive(Clone, Debug, Default)]
pub struct DeviceCatalog {
    packages: Vec<DevicePackage>,
    warnings: Vec<String>,
}

/// Where device packages may live: `ATLAS_DEVICES_DIR`, the given extra
/// dirs (app resources, data folder), then, for development builds, a
/// checkout's `devices/` found above the running program or the working
/// directory, and the one this crate was built from when it still exists.
/// (A build in a shared target dir can carry a worktree's path that has
/// since gone: it found no packages and said nothing.)
pub fn default_search_dirs(extra: &[PathBuf]) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = std::env::var_os("ATLAS_DEVICES_DIR") {
        dirs.push(PathBuf::from(dir));
    }
    dirs.extend(extra.iter().cloned());
    let starts = [
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf)),
        std::env::current_dir().ok(),
    ];
    for start in starts.into_iter().flatten() {
        if let Some(found) = checkout_devices_above(&start) {
            dirs.push(found);
        }
    }
    let built = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../devices");
    if built.is_dir() {
        dirs.push(built);
    }
    dirs.dedup();
    dirs
}

/// The nearest `devices/` above `start` that holds a device package (a
/// `<model>/manifest.json`), as in this repository.
fn checkout_devices_above(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .map(|dir| dir.join("devices"))
        .find(|devices| {
            std::fs::read_dir(devices).is_ok_and(|entries| {
                entries
                    .flatten()
                    .any(|entry| entry.path().join("manifest.json").is_file())
            })
        })
}

impl DeviceCatalog {
    /// A line for a health check: how many device packages loaded, or that
    /// none did and where Atlas looked (no packages: no boards are found).
    pub fn summary(&self, searched: &[PathBuf]) -> Result<String, String> {
        let models: Vec<&str> = self
            .packages
            .iter()
            .map(|p| p.manifest.model.as_str())
            .collect();
        if models.is_empty() {
            let looked: Vec<String> = searched.iter().map(|d| d.display().to_string()).collect();
            return Err(format!(
                "No device packages found, so no boards can be found. Looked in: {}",
                looked.join(", ")
            ));
        }
        Ok(format!("{} loaded: {}", models.len(), models.join(", ")))
    }
}

impl DeviceCatalog {
    /// Loads every `<dir>/<model>/manifest.json`. The first directory that
    /// has a model wins for that model. Problems become warnings, never
    /// errors: a broken package must not stop Atlas.
    pub fn load(dirs: &[PathBuf]) -> Self {
        let mut catalog = Self::default();
        for dir in dirs {
            let Ok(entries) = std::fs::read_dir(dir) else {
                continue;
            };
            let mut folders: Vec<PathBuf> = entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.join("manifest.json").is_file())
                .collect();
            folders.sort();
            for folder in folders {
                match load_package(&folder) {
                    Ok(package) => {
                        if catalog.by_model(&package.manifest.model).is_none() {
                            catalog.packages.push(package);
                        }
                    }
                    Err(warning) => catalog.warnings.push(warning),
                }
            }
        }
        catalog
    }

    /// A catalog of packages already in memory, for tests and embedders.
    pub fn from_packages(packages: Vec<DevicePackage>) -> Self {
        Self {
            packages,
            ..Self::default()
        }
    }

    pub fn packages(&self) -> &[DevicePackage] {
        &self.packages
    }

    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    pub fn by_model(&self, model: &str) -> Option<&DevicePackage> {
        self.packages
            .iter()
            .find(|package| package.manifest.model.eq_ignore_ascii_case(model))
    }

    /// Models whose recovery mode uses this USB boot id. Several models can
    /// share one SoC, so this may return more than one.
    pub fn for_usb_boot(&self, vid: u16, pid: u16) -> Vec<&DevicePackage> {
        self.packages
            .iter()
            .filter(|package| {
                package
                    .manifest
                    .recovery
                    .usb_boot
                    .is_some_and(|id| id.vid.0 == vid && id.pid.0 == pid)
            })
            .collect()
    }

    /// The model whose USB gadget strings match, if any.
    pub fn for_gadget(
        &self,
        manufacturer: Option<&str>,
        product: Option<&str>,
    ) -> Option<&DevicePackage> {
        self.packages
            .iter()
            .find(|package| package.manifest.matches_gadget(manufacturer, product))
    }
}

fn load_package(folder: &Path) -> Result<DevicePackage, String> {
    let manifest_path = folder.join("manifest.json");
    let data = std::fs::read(&manifest_path)
        .map_err(|error| format!("{}: {error}", manifest_path.display()))?;
    let manifest: DeviceManifest = serde_json::from_slice(&data)
        .map_err(|error| format!("{}: {error}", manifest_path.display()))?;
    if manifest.contract > CONTRACT {
        return Err(format!(
            "{}: device contract {} is newer than this Atlas understands ({CONTRACT}); update Atlas",
            manifest_path.display(),
            manifest.contract
        ));
    }
    let compat_path = folder.join("compat.json");
    let compat = match std::fs::read(&compat_path) {
        Ok(data) => serde_json::from_slice(&data)
            .map_err(|error| format!("{}: {error}", compat_path.display()))?,
        Err(_) => CompatList::default(),
    };
    Ok(DevicePackage {
        manifest,
        compat,
        dir: folder.to_path_buf(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::tests::RAZE;

    fn temp_root(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "atlas-devices-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_nanos())
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_checkouts_devices_is_found_above_the_program_and_none_is_said() {
        let root = temp_root("above");
        std::fs::create_dir_all(root.join("devices/raze")).unwrap();
        std::fs::write(root.join("devices/raze/manifest.json"), RAZE).unwrap();
        let deep = root.join("target/debug/deps");
        std::fs::create_dir_all(&deep).unwrap();
        assert_eq!(checkout_devices_above(&deep), Some(root.join("devices")));

        let catalog = DeviceCatalog::load(&[root.join("devices")]);
        assert_eq!(catalog.summary(&[]).unwrap(), "1 loaded: raze");
        let empty = DeviceCatalog::load(&[root.join("nothing")]);
        let why = empty.summary(&[root.join("nothing")]).unwrap_err();
        assert!(why.starts_with("No device packages found"), "{why}");
        assert!(why.contains("nothing"), "{why}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn packages_load_and_bad_ones_become_warnings() {
        let root = temp_root("load");
        std::fs::create_dir_all(root.join("raze/eeprom")).unwrap();
        std::fs::write(root.join("raze/manifest.json"), RAZE).unwrap();
        std::fs::write(
            root.join("raze/compat.json"),
            r#"[{ "os": "helios", "versions": "*", "status": "known-good" }]"#,
        )
        .unwrap();
        std::fs::create_dir_all(root.join("broken")).unwrap();
        std::fs::write(root.join("broken/manifest.json"), b"{ not json").unwrap();
        std::fs::create_dir_all(root.join("future")).unwrap();
        std::fs::write(
            root.join("future/manifest.json"),
            r#"{ "contract": 99, "model": "future" }"#,
        )
        .unwrap();

        let catalog = DeviceCatalog::load(std::slice::from_ref(&root));

        assert_eq!(catalog.packages().len(), 1);
        assert_eq!(catalog.warnings().len(), 2);
        let raze = catalog.by_model("RAZE").unwrap();
        assert_eq!(raze.compat.entries.len(), 1);
        assert!(raze.eeprom_dir().is_some());
        assert_eq!(catalog.for_usb_boot(0x0a5c, 0x2712).len(), 1);
        assert!(catalog.for_usb_boot(0x0a5c, 0x2711).is_empty());
        assert!(
            catalog
                .for_gadget(Some("Prometheus Dynamics"), Some("Raze"))
                .is_some()
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn missing_dirs_give_an_empty_catalog() {
        let catalog = DeviceCatalog::load(&[PathBuf::from("/definitely/not/here")]);
        assert!(catalog.packages().is_empty());
        assert!(catalog.warnings().is_empty());
    }
}
