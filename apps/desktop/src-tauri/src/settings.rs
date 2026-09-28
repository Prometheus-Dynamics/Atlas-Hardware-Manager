use std::path::{Path, PathBuf};

use atlas_core::StagedRollout;
use serde::{Deserialize, Serialize};

/// Simulated device sets for demos and UI work.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SimScenario {
    Demo,
    Flaky,
}

impl SimScenario {
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "demo" | "1" | "true" => Some(Self::Demo),
            "flaky" => Some(Self::Flaky),
            _ => None,
        }
    }
}

/// User preferences, saved as `settings.json` in the data directory.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    /// Use simulated devices instead of hardware. Applies after a restart.
    pub simulated: Option<SimScenario>,
    /// Scan in the background so devices appear without pressing Scan.
    pub auto_scan: bool,
    pub scan_interval_ms: u64,
    /// Default for the staged rollout switch in the update dialog.
    pub staged_default: StagedRollout,
    /// Allow installing local files that no trusted key signed.
    pub allow_unsigned_local: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            simulated: None,
            auto_scan: true,
            scan_interval_ms: 3000,
            staged_default: StagedRollout::Auto,
            allow_unsigned_local: true,
        }
    }
}

impl AppSettings {
    pub fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|data| serde_json::from_slice(&data).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let data = serde_json::to_vec_pretty(self).map_err(|error| error.to_string())?;
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, data).map_err(|error| error.to_string())?;
        std::fs::rename(&temp, path).map_err(|error| error.to_string())
    }
}

/// Where Atlas keeps its files.
#[derive(Clone, Debug, Serialize)]
pub struct AppPaths {
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub settings_file: PathBuf,
    pub inventory_file: PathBuf,
    pub releases_file: PathBuf,
    pub release_cache_dir: PathBuf,
}

impl AppPaths {
    /// Simulated runs keep their own inventory so they never mix with
    /// real devices.
    pub fn resolve(simulated: bool) -> Self {
        let data_dir = std::env::var_os("ATLAS_DATA_DIR")
            .map(PathBuf::from)
            .or_else(|| dirs::data_dir().map(|dir| dir.join("atlas")))
            .unwrap_or_else(|| std::env::temp_dir().join("atlas"));
        let cache_dir = dirs::cache_dir()
            .map(|dir| dir.join("atlas"))
            .unwrap_or_else(|| data_dir.join("cache"));
        let inventory = if simulated {
            "inventory-sim.json"
        } else {
            "inventory.json"
        };
        Self {
            settings_file: data_dir.join("settings.json"),
            inventory_file: data_dir.join(inventory),
            releases_file: data_dir.join("releases.json"),
            release_cache_dir: cache_dir.join("releases"),
            data_dir,
            cache_dir,
        }
    }
}
