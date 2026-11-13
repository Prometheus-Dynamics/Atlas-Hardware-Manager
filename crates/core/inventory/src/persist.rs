use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use atlas_types::CapabilityKind;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::history::HistoryEntry;
use crate::inventory::{AliasEntry, Inventory, PowerCycle};
use crate::record::{DeviceRecord, LinkSighting};
use crate::robots::Robot;

/// The format written now. A file without a version, or with an older one,
/// loads with defaults for missing fields. A newer one loads as far as this
/// build understands it: unknown values are dropped (see [`LoadOutcome`]).
pub const FORMAT_VERSION: u32 = 1;

/// What loading found. Loading never fails: the app always gets an
/// inventory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadOutcome {
    /// No file yet. The inventory starts empty.
    Fresh,
    /// The saved inventory was read. `skipped_records` whole records were
    /// dropped because they could not be read at all (for example a device
    /// state this build does not know). `dropped_values` unknown values
    /// inside kept records were dropped: capability kinds, link kinds and
    /// history kinds. Everything else was kept.
    Restored {
        skipped_records: usize,
        dropped_values: usize,
    },
    /// The file could not be used at all. The inventory starts empty. The
    /// file was moved to `aside` when it could be, so nothing it held is lost.
    Reset {
        reason: String,
        aside: Option<PathBuf>,
    },
}

#[derive(Serialize)]
struct Saved {
    version: u32,
    devices: Vec<DeviceRecord>,
    aliases: Vec<AliasEntry>,
    robots: Vec<Robot>,
    power_cycles: BTreeMap<String, Vec<PowerCycle>>,
}

/// The file's top level. Each record, alias and robot is read on its own, so
/// one value this build does not know costs that value and nothing else.
#[derive(Deserialize)]
struct Loaded {
    #[serde(default)]
    devices: Vec<Value>,
    #[serde(default)]
    aliases: Vec<Value>,
    #[serde(default)]
    robots: Vec<Value>,
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
            Ok(loaded) => loaded,
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
        let (devices, aliases, robots, power_cycles) = inventory.to_parts();
        let data = serde_json::to_vec(&Saved {
            version: FORMAT_VERSION,
            devices,
            aliases,
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

fn parse(data: &[u8]) -> Result<(Inventory, LoadOutcome), String> {
    let loaded: Loaded = serde_json::from_slice(data).map_err(|error| error.to_string())?;
    let mut dropped_values = 0;
    let mut skipped_records = 0;

    let mut devices = Vec::new();
    for value in loaded.devices {
        match read_record(value, &mut dropped_values) {
            Some(record) => devices.push(record),
            None => skipped_records += 1,
        }
    }
    let aliases = read_each::<AliasEntry>(loaded.aliases, &mut dropped_values);
    let robots = read_each::<Robot>(loaded.robots, &mut dropped_values);

    let inventory = Inventory::from_parts(devices, aliases, robots, loaded.power_cycles);
    Ok((
        inventory,
        LoadOutcome::Restored {
            skipped_records,
            dropped_values,
        },
    ))
}

/// Reads one record. Unknown capability kinds, link kinds and history kinds
/// are dropped from it and counted. Returns `None` when the record still
/// cannot be read (an unknown device state, for example).
fn read_record(mut value: Value, dropped: &mut usize) -> Option<DeviceRecord> {
    if let Some(kinds) = value.pointer_mut("/capabilities/kinds") {
        retain_known::<CapabilityKind>(kinds, dropped);
    }
    if let Some(links) = value.get_mut("links") {
        retain_known::<LinkSighting>(links, dropped);
    }
    if let Some(history) = value.get_mut("history") {
        retain_known::<HistoryEntry>(history, dropped);
    }
    serde_json::from_value(value).ok()
}

/// Reads each value of a list, dropping and counting the ones that do not
/// read.
fn read_each<T: DeserializeOwned>(values: Vec<Value>, dropped: &mut usize) -> Vec<T> {
    values
        .into_iter()
        .filter_map(|value| match serde_json::from_value(value) {
            Ok(item) => Some(item),
            Err(_) => {
                *dropped += 1;
                None
            }
        })
        .collect()
}

/// Keeps the items of a JSON array that read as `T`; counts the rest.
fn retain_known<T: DeserializeOwned>(list: &mut Value, dropped: &mut usize) {
    let Some(items) = list.as_array_mut() else {
        return;
    };
    let before = items.len();
    items.retain(|item| serde_json::from_value::<T>(item.clone()).is_ok());
    *dropped += before - items.len();
}

#[cfg(unix)]
fn sync_dir(dir: &Path) -> io::Result<()> {
    fs::File::open(dir)?.sync_all()
}

#[cfg(not(unix))]
fn sync_dir(_dir: &Path) -> io::Result<()> {
    Ok(())
}
