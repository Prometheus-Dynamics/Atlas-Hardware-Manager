//! The pins: one record per address, the board's key and the fingerprint it was pinned with.
//!
//! The file is JSON, written whole to a temporary file in the same directory and renamed over
//! the old one. A file that cannot be read is never overwritten: the command refuses, and says
//! where the file is.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use atlas_trial::{hex, key_from_hex};
use serde_json::{Value, json};

/// The pins file, in the state directory.
pub const PINS_FILE: &str = "pins.json";

/// The format written now.
const VERSION: u64 = 1;

/// The key a board was pinned with, and its fingerprint text (as the board's own session says it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pin {
    /// The address text, as Orion writes it (`Address`'s display form).
    pub address: String,
    pub key: [u8; 32],
    pub fingerprint: String,
}

/// The pins of one state directory.
#[derive(Debug)]
pub struct Pins {
    path: PathBuf,
    records: Vec<Pin>,
}

impl Pins {
    /// The pins in `state`. A missing file is no pins; an unreadable one is an error, with its
    /// place, and is left as it is.
    ///
    /// # Errors
    ///
    /// When the file exists but is not a pins file this build reads.
    pub fn load(state: &Path) -> Result<Self, String> {
        let path = state.join(PINS_FILE);
        let data = match fs::read(&path) {
            Ok(data) => data,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(Self {
                    path,
                    records: Vec::new(),
                });
            }
            Err(error) => {
                return Err(format!(
                    "the pins file {} cannot be read: {error}",
                    path.display()
                ));
            }
        };
        let unreadable = |why: &str| {
            format!(
                "the pins file {} cannot be read ({why}); it is left as it is: move it aside to start over",
                path.display()
            )
        };
        let document: Value =
            serde_json::from_slice(&data).map_err(|error| unreadable(&error.to_string()))?;
        if document.get("version").and_then(Value::as_u64) != Some(VERSION) {
            return Err(unreadable("not version 1"));
        }
        let entries = document
            .get("pins")
            .and_then(Value::as_array)
            .ok_or_else(|| unreadable("no pins list"))?;
        let mut records = Vec::with_capacity(entries.len());
        for entry in entries {
            let text = |field: &str| entry.get(field).and_then(Value::as_str).map(str::to_owned);
            let (Some(address), Some(key), Some(fingerprint)) =
                (text("address"), text("key"), text("fingerprint"))
            else {
                return Err(unreadable("a record without address, key or fingerprint"));
            };
            let key =
                key_from_hex(&key).ok_or_else(|| unreadable("a key that is not 64 hex digits"))?;
            records.push(Pin {
                address,
                key,
                fingerprint,
            });
        }
        Ok(Self { path, records })
    }

    /// The pin for `address`, if one is recorded.
    #[must_use]
    pub fn get(&self, address: &str) -> Option<&Pin> {
        self.records.iter().find(|pin| pin.address == address)
    }

    /// Records `pin` for its address, replacing any pin there.
    pub fn set(&mut self, pin: Pin) {
        self.records.retain(|record| record.address != pin.address);
        self.records.push(pin);
    }

    /// Writes the pins: a temporary file in the same directory, synced, renamed over the file.
    ///
    /// # Errors
    ///
    /// When the directory or the file cannot be written.
    pub fn save(&self) -> Result<(), String> {
        let document = json!({
            "version": VERSION,
            "pins": self.records.iter().map(|pin| json!({
                "address": pin.address,
                "key": hex(&pin.key),
                "fingerprint": pin.fingerprint,
            })).collect::<Vec<_>>(),
        });
        let data = serde_json::to_vec_pretty(&document).map_err(|error| error.to_string())?;
        self.write(&data).map_err(|error| {
            format!(
                "the pins file {} cannot be written: {error}",
                self.path.display()
            )
        })
    }

    fn write(&self, data: &[u8]) -> io::Result<()> {
        let parent = self
            .path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;
        let temp = self.path.with_extension("json.tmp");
        {
            let mut file = fs::File::create(&temp)?;
            file.write_all(data)?;
            file.sync_all()?;
        }
        fs::rename(&temp, &self.path)?;
        fs::File::open(parent)?.sync_all()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_round_trip_and_replace_per_address() {
        let dir = std::env::temp_dir().join(format!("atlas-next-pins-unit-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut pins = Pins::load(&dir).expect("a missing file is no pins");
        assert!(pins.get("a").is_none());
        pins.set(Pin {
            address: "a".into(),
            key: [1; 32],
            fingerprint: "sha256:01".into(),
        });
        pins.set(Pin {
            address: "a".into(),
            key: [2; 32],
            fingerprint: "sha256:02".into(),
        });
        pins.save().expect("the pins are written");
        let back = Pins::load(&dir).expect("the pins are read");
        assert_eq!(back.get("a").map(|pin| pin.key), Some([2; 32]));
        assert_eq!(back.records.len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unreadable_pins_file_is_refused_and_not_overwritten() {
        let dir = std::env::temp_dir().join(format!("atlas-next-pins-bad-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        fs::write(dir.join(PINS_FILE), b"not json").expect("the file is written");
        let error = Pins::load(&dir).expect_err("a bad file is refused");
        assert!(error.contains("left as it is"), "{error}");
        assert_eq!(
            fs::read(dir.join(PINS_FILE)).expect("still there"),
            b"not json"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
