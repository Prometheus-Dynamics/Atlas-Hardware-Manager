use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::inventory::{Inventory, PowerCycle};
use crate::record::DeviceRecord;
use crate::robots::Robot;

/// The format written now. Files without a version, or with an older one,
/// load: missing fields take their defaults.
pub const FORMAT_VERSION: u32 = 1;

/// What loading found. Loading never fails: the app always gets an
/// inventory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadOutcome {
    /// No file yet. The inventory starts empty.
    Fresh,
    /// The saved inventory was read.
    Restored,
    /// The file could not be used. The inventory starts empty. The file was
    /// moved to `aside` when it could be, so nothing it held is lost.
    Reset {
        reason: String,
        aside: Option<PathBuf>,
    },
}

#[derive(Serialize)]
struct Saved {
    version: u32,
    devices: Vec<DeviceRecord>,
    robots: Vec<Robot>,
    power_cycles: BTreeMap<String, Vec<PowerCycle>>,
}

#[derive(Deserialize)]
struct Loaded {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    devices: Vec<DeviceRecord>,
    #[serde(default)]
    robots: Vec<Robot>,
    #[serde(default)]
    power_cycles: BTreeMap<String, Vec<PowerCycle>>,
}

/// The inventory in one JSON file.
#[derive(Clone, Debug)]
pub struct JsonFileStore {
    path: PathBuf,
}

impl JsonFileStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Reads the saved inventory. A missing file is a fresh start. A file
    /// that is corrupt, or from a newer Atlas, is moved aside and the
    /// inventory starts empty. A file that cannot be read at all is left in
    /// place.
    pub fn load(&self) -> (Inventory, LoadOutcome) {
        let data = match fs::read(&self.path) {
            Ok(data) => data,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return (Inventory::new(), LoadOutcome::Fresh);
            }
            Err(error) => {
                return (
                    Inventory::new(),
                    LoadOutcome::Reset {
                        reason: format!("cannot read it: {error}"),
                        aside: None,
                    },
                );
            }
        };
        match parse(&data) {
            Ok(inventory) => (inventory, LoadOutcome::Restored),
            Err(reason) => {
                let aside = self.set_aside();
                (Inventory::new(), LoadOutcome::Reset { reason, aside })
            }
        }
    }

    /// Writes the inventory crash-safely: a temp file is written and synced,
    /// then renamed over the saved file, then the directory is synced. A
    /// crash leaves either the old file or the new one, never a partial one.
    pub fn save(&self, inventory: &Inventory) -> io::Result<()> {
        let parent = self
            .path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty());
        if let Some(parent) = parent {
            fs::create_dir_all(parent)?;
        }
        let (devices, robots, power_cycles) = inventory.to_parts();
        let data = serde_json::to_vec(&Saved {
            version: FORMAT_VERSION,
            devices,
            robots,
            power_cycles,
        })
        .map_err(io::Error::other)?;

        let temp = self.temp_path();
        {
            let mut file = fs::File::create(&temp)?;
            file.write_all(&data)?;
            file.sync_all()?;
        }
        fs::rename(&temp, &self.path)?;
        sync_dir(parent.unwrap_or_else(|| Path::new(".")))
    }

    fn temp_path(&self) -> PathBuf {
        let mut name = self
            .path
            .file_name()
            .map(|name| name.to_os_string())
            .unwrap_or_default();
        name.push(".tmp");
        self.path.with_file_name(name)
    }

    /// Moves the unusable file to `<name>.bad-<unix ms>`. Returns where it
    /// went, or `None` when it could not be moved.
    fn set_aside(&self) -> Option<PathBuf> {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_millis());
        let mut name = self.path.file_name()?.to_os_string();
        name.push(format!(".bad-{stamp}"));
        let aside = self.path.with_file_name(name);
        fs::rename(&self.path, &aside).ok().map(|()| aside)
    }
}

fn parse(data: &[u8]) -> Result<Inventory, String> {
    let loaded: Loaded = serde_json::from_slice(data).map_err(|error| error.to_string())?;
    if loaded.version > FORMAT_VERSION {
        return Err(format!(
            "format version {} is newer than this Atlas reads ({FORMAT_VERSION})",
            loaded.version
        ));
    }
    Ok(Inventory::from_parts(
        loaded.devices,
        loaded.robots,
        loaded.power_cycles,
    ))
}

#[cfg(unix)]
fn sync_dir(dir: &Path) -> io::Result<()> {
    fs::File::open(dir)?.sync_all()
}

#[cfg(not(unix))]
fn sync_dir(_dir: &Path) -> io::Result<()> {
    Ok(())
}
