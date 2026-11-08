//! Device families: what the boards of one chip family share in their
//! download mode (`devices/families/<id>.json`). A board names its family
//! with `"family"` and omits what the family gives: the recovery method and
//! the download-mode steps. Its own facts (USB id, flash, storage) stay in
//! its record.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::manifest::{EepromInfo, UsbId};

/// The schema every family file carries (`devices/schema/family.schema.json`).
pub const FAMILY_SCHEMA: &str = "urn:board:family:1";

/// One chip family: the data its boards share.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Family {
    #[serde(default)]
    pub schema: Option<String>,
    /// The id a board's `family` names.
    pub family: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub vendor: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// The recovery method (`rockusb`, `fel`, `k230-usb`) a board gets when
    /// its record omits `recovery.method`.
    pub method: String,
    /// The USB vendor id the family's download mode shows.
    pub usb_vendor: UsbId,
    /// The download-mode steps a board gets when its record omits
    /// `recovery.instructions`.
    #[serde(default)]
    pub entry: Vec<String>,
    /// Family-wide flash rules: data only, shaped by the family (`stages`,
    /// `loader_check`, `sram`). A board's own flash facts stay in its record.
    #[serde(default)]
    pub flash: Option<Value>,
    /// How each host OS matches the family's devices (`linux`, `windows`).
    #[serde(default)]
    pub host: BTreeMap<String, String>,
    /// The chip a download-mode PID stands for, keyed by the PID as four
    /// hex digits (`350b`).
    #[serde(default)]
    pub pid_chips: BTreeMap<String, String>,
    /// The family's bootloader images, `path`s relative to [`Family::dir`].
    #[serde(default)]
    pub eeprom: Option<EepromInfo>,
    #[serde(default)]
    pub todo: Vec<String>,
    /// `devices/families/<id>/`, where the family's files are.
    #[serde(skip)]
    pub dir: PathBuf,
}

impl Family {
    /// The family's name for people: its display name, else its id.
    pub fn display_name(&self) -> &str {
        self.display_name.as_deref().unwrap_or(&self.family)
    }

    /// The chip a download-mode PID stands for, where the family names it.
    pub fn chip_for_pid(&self, product_id: u16) -> Option<&str> {
        self.pid_chips
            .get(&format!("{product_id:04x}"))
            .map(String::as_str)
    }

    /// The chip tags a loader or chip reply may carry for a record's SoC
    /// (`flash.loader_chips`). `None` when the family names none for it.
    pub fn loader_chips(&self, soc: &str) -> Option<Vec<String>> {
        let table = self.flash.as_ref()?.get("loader_chips")?.as_object()?;
        let tags = table.get(soc)?.as_array()?;
        Some(
            tags.iter()
                .filter_map(Value::as_str)
                .map(str::to_ascii_uppercase)
                .collect(),
        )
    }
}

/// The families one device folder holds (`<root>/families/*.json`), and
/// the warnings for the files that aren't families. A broken file never
/// stops Atlas, as a broken package doesn't.
pub(crate) fn load_families(root: &Path) -> (Vec<Family>, Vec<String>) {
    let Ok(entries) = std::fs::read_dir(root.join("families")) else {
        return (Vec::new(), Vec::new());
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().and_then(|ext| ext.to_str()) == Some("json") && path.is_file()
        })
        .collect();
    files.sort();
    let mut families = Vec::new();
    let mut warnings = Vec::new();
    for file in files {
        match load_family(&file) {
            Ok(family) => families.push(family),
            Err(warning) => warnings.push(warning),
        }
    }
    (families, warnings)
}

fn load_family(path: &Path) -> Result<Family, String> {
    let data = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut family: Family =
        serde_json::from_slice(&data).map_err(|error| format!("{}: {error}", path.display()))?;
    family.dir = path.with_extension("");
    if family.schema.as_deref() != Some(FAMILY_SCHEMA) {
        return Err(format!(
            "{}: not a family file (its schema must be {FAMILY_SCHEMA})",
            path.display()
        ));
    }
    Ok(family)
}
