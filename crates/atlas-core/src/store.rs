use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::DeviceRecord;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("inventory file {path}: {message}")]
    Io { path: PathBuf, message: String },
    #[error("inventory file {path} is not valid: {message}")]
    Format { path: PathBuf, message: String },
}

/// Where the remembered inventory lives between sessions.
pub trait InventoryStore: Send + Sync {
    fn load(&self) -> Result<Vec<DeviceRecord>, StoreError>;
    fn save(&self, records: &[DeviceRecord]) -> Result<(), StoreError>;
}

/// Keeps the inventory for the life of the process only.
#[derive(Default)]
pub struct MemoryStore {
    records: Mutex<Vec<DeviceRecord>>,
}

impl InventoryStore for MemoryStore {
    fn load(&self) -> Result<Vec<DeviceRecord>, StoreError> {
        Ok(self
            .records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone())
    }

    fn save(&self, records: &[DeviceRecord]) -> Result<(), StoreError> {
        *self
            .records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = records.to_vec();
        Ok(())
    }
}

const FORMAT_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct InventoryFile {
    version: u32,
    devices: Vec<DeviceRecord>,
}

/// A JSON file written atomically: temp file, sync, rename.
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

    fn io_error(&self, error: std::io::Error) -> StoreError {
        StoreError::Io {
            path: self.path.clone(),
            message: error.to_string(),
        }
    }
}

impl InventoryStore for JsonFileStore {
    fn load(&self) -> Result<Vec<DeviceRecord>, StoreError> {
        let data = match fs::read(&self.path) {
            Ok(data) => data,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(self.io_error(error)),
        };
        let file: InventoryFile =
            serde_json::from_slice(&data).map_err(|error| StoreError::Format {
                path: self.path.clone(),
                message: error.to_string(),
            })?;
        if file.version != FORMAT_VERSION {
            return Err(StoreError::Format {
                path: self.path.clone(),
                message: format!(
                    "format version {} is not supported (expected {FORMAT_VERSION})",
                    file.version
                ),
            });
        }
        Ok(file.devices)
    }

    fn save(&self, records: &[DeviceRecord]) -> Result<(), StoreError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|error| self.io_error(error))?;
        }
        let file = InventoryFile {
            version: FORMAT_VERSION,
            devices: records.to_vec(),
        };
        let data = serde_json::to_vec_pretty(&file).map_err(|error| StoreError::Format {
            path: self.path.clone(),
            message: error.to_string(),
        })?;
        let temp_path = self.path.with_extension("json.tmp");
        let mut temp = fs::File::create(&temp_path).map_err(|error| self.io_error(error))?;
        temp.write_all(&data)
            .map_err(|error| self.io_error(error))?;
        temp.sync_all().map_err(|error| self.io_error(error))?;
        drop(temp);
        // `fs::rename` replaces an existing file on every supported OS.
        fs::rename(&temp_path, &self.path).map_err(|error| self.io_error(error))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use atlas_driver::{CapabilityKind, DeviceKey, DeviceMode, Identity, LinkId, LinkKind};

    use super::*;
    use crate::Presence;

    fn record() -> DeviceRecord {
        DeviceRecord {
            key: DeviceKey::new("helios", "a"),
            identity: Identity {
                key: DeviceKey::new("helios", "a"),
                model: "cm5".into(),
                mode: DeviceMode::Normal,
                versions: BTreeMap::from([("os".to_string(), "1.0.0".to_string())]),
                name: Some("cam-front".into()),
                link: LinkId("usbnet:0".into()),
                address: "10.0.0.2".into(),
            },
            link_kind: LinkKind::UsbNetwork,
            capabilities: vec![CapabilityKind::Info],
            presence: Presence::Online,
            first_seen_ms: 1,
            last_seen_ms: 2,
            label: Some("front camera".into()),
            robot: Some("comp".into()),
        }
    }

    fn temp_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "atlas-core-store-{label}-{}-{}.json",
            std::process::id(),
            crate::time::now_ms()
        ))
    }

    #[test]
    fn json_store_round_trips_records() {
        let path = temp_path("roundtrip");
        let store = JsonFileStore::new(&path);

        store.save(&[record()]).unwrap();
        store.save(&[record()]).unwrap();
        assert_eq!(store.load().unwrap(), vec![record()]);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn missing_file_loads_empty() {
        let store = JsonFileStore::new(temp_path("missing"));
        assert!(store.load().unwrap().is_empty());
    }

    #[test]
    fn unknown_format_version_is_rejected() {
        let path = temp_path("version");
        fs::write(&path, br#"{"version":99,"devices":[]}"#).unwrap();

        let error = JsonFileStore::new(&path).load().unwrap_err();
        assert!(matches!(error, StoreError::Format { .. }));

        let _ = fs::remove_file(path);
    }
}
