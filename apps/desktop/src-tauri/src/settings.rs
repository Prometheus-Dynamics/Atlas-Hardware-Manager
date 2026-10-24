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
    /// Watch for devices: scan on USB hotplug and network announcements,
    /// so devices appear and disappear without pressing Scan.
    pub auto_scan: bool,
    /// The safety-net rescan while watching, for devices that leave without
    /// a notice. Not a polling rate: changes are picked up immediately.
    pub scan_interval_ms: u64,
    /// Default for the staged rollout switch in the update dialog.
    pub staged_default: StagedRollout,
    /// Which boards get this computer's time when their clock is off (a
    /// Raze has no RTC battery), while watching.
    pub clock_sync: atlas_core::ClockSync,
    /// A public key file (like `~/.ssh/id_ed25519.pub`) to put on boards
    /// Atlas flashes, for SSH as root. Off when unset.
    #[serde(default)]
    pub ssh_key_file: Option<String>,
    /// The Orion node Atlas connects to as an operator
    /// (`orion+tcp://host:port`). Off when unset.
    #[serde(default)]
    pub orion_url: Option<String>,
    /// Board readings pushed by the board's own Orion node (its live
    /// streams) where it offers them; otherwise, and when off, polled.
    pub live_streams: bool,
    /// The TCP port boards download update images from (Orion updates).
    pub image_server_port: u16,
    /// The host put in image URLs when Atlas can't tell which of its
    /// addresses routes to a board. Automatic when unset.
    #[serde(default)]
    pub image_host: Option<String>,
    /// Atlas's own updates from GitHub.
    pub app_updates: crate::app_update::AppUpdateSettings,
    /// FRC team numbers whose robot networks (10.TE.AM.0/24) Atlas looks on
    /// behind a board's USB route, when the route is wider than one robot
    /// network. Teams devices were seen on are looked on anyway.
    #[serde(default)]
    pub teams: Vec<u16>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            simulated: None,
            auto_scan: true,
            scan_interval_ms: DEFAULT_FALLBACK_MS,
            staged_default: StagedRollout::Auto,
            clock_sync: atlas_core::ClockSync::default(),
            ssh_key_file: None,
            orion_url: None,
            live_streams: true,
            image_server_port: atlas_image_server::DEFAULT_PORT,
            image_host: None,
            app_updates: crate::app_update::AppUpdateSettings::default(),
            teams: Vec::new(),
        }
    }
}

pub const DEFAULT_FALLBACK_MS: u64 = 20_000;
pub const FALLBACK_RANGE_MS: (u64, u64) = (5_000, 300_000);

impl AppSettings {
    pub fn load(path: &Path) -> Self {
        let mut settings: Self = std::fs::read(path)
            .ok()
            .and_then(|data| serde_json::from_slice(&data).ok())
            .unwrap_or_default();
        // Older versions polled every few seconds; that value is not a
        // sensible safety-net interval.
        if settings.scan_interval_ms < FALLBACK_RANGE_MS.0 {
            settings.scan_interval_ms = DEFAULT_FALLBACK_MS;
        }
        if settings.image_server_port == 0 {
            settings.image_server_port = atlas_image_server::DEFAULT_PORT;
        }
        settings.note_teams();
        settings
    }

    /// Tells the drivers about the user's teams (they are only ever added
    /// to: a team removed here is forgotten at the next start).
    pub fn note_teams(&self) {
        let teams = atlas_driver::robot_net::RobotTeams::shared();
        for team in &self.teams {
            teams.add(*team);
        }
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
    /// What Atlas did and what went wrong, kept across restarts.
    pub log_file: PathBuf,
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
            log_file: data_dir.join("atlas.log"),
            data_dir,
            cache_dir,
        }
    }
}

/// Reads the public key file named in the settings: `Ok(None)` when unset,
/// an error when it is set but missing or not an OpenSSH public key.
/// How Atlas SSHes into boards for updates: the private key next to the
/// chosen public key (else ssh's defaults) and Atlas's own known_hosts.
pub fn ssh_config(paths: &AppPaths, file: Option<&str>) -> atlas_driver_board::SshConfig {
    let identity_file = file
        .map(str::trim)
        .filter(|file| !file.is_empty())
        .map(|file| match file.strip_prefix("~/") {
            Some(rest) => dirs::home_dir().unwrap_or_default().join(rest),
            None => std::path::PathBuf::from(file),
        })
        .and_then(|public| atlas_driver_board::private_key_for(&public))
        .or_else(|| own_key_unless_default(paths));
    atlas_driver_board::SshConfig {
        identity_file,
        known_hosts: Some(paths.data_dir.join("known_hosts")),
        // The key next to the private one (`ssh-key.add`).
        public_key: None,
    }
}

/// With no key chosen and none of ssh's defaults (`~/.ssh/id_ed25519`,
/// `id_ecdsa`, `id_rsa`), Atlas's own key (`<data>/atlas_ed25519`, made once
/// with ssh-keygen), so a board plugged in here can always be given a key
/// over its USB console and then reached over SSH. `None` leaves ssh its
/// defaults.
fn own_key_unless_default(paths: &AppPaths) -> Option<std::path::PathBuf> {
    let ssh = dirs::home_dir().map(|home| home.join(".ssh"));
    let has_default = ssh.is_some_and(|ssh| {
        ["id_ed25519", "id_ecdsa", "id_rsa"]
            .iter()
            .any(|name| ssh.join(name).is_file() && ssh.join(format!("{name}.pub")).is_file())
    });
    if has_default {
        return None;
    }
    let key = paths.data_dir.join("atlas_ed25519");
    if !key.is_file() {
        let _ = std::fs::create_dir_all(&paths.data_dir);
        let made = std::process::Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-C", "atlas", "-f"])
            .arg(&key)
            .stdin(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        if !made {
            return None;
        }
    }
    Some(key)
}

pub fn read_ssh_keys(file: Option<&str>) -> Result<Option<String>, String> {
    let Some(file) = file.map(str::trim).filter(|file| !file.is_empty()) else {
        return Ok(None);
    };
    let path = match file.strip_prefix("~/") {
        Some(rest) => dirs::home_dir()
            .ok_or("no home directory to expand ~ in")?
            .join(rest),
        None => std::path::PathBuf::from(file),
    };
    atlas_driver_rpi::SshKeys::read_public(&path).map(Some)
}

#[cfg(test)]
mod ssh_key_tests {
    use super::read_ssh_keys;

    #[test]
    fn only_public_keys_are_accepted() {
        let dir = std::env::temp_dir().join(format!("atlas-keys-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let public = dir.join("id.pub");
        std::fs::write(&public, "ssh-ed25519 AAAA me@host\n").unwrap();
        let private = dir.join("id");
        std::fs::write(&private, "-----BEGIN OPENSSH PRIVATE KEY-----\n").unwrap();

        assert_eq!(read_ssh_keys(None).unwrap(), None);
        assert_eq!(read_ssh_keys(Some(" ")).unwrap(), None);
        assert_eq!(
            read_ssh_keys(public.to_str()).unwrap().as_deref(),
            Some("ssh-ed25519 AAAA me@host")
        );
        assert!(read_ssh_keys(private.to_str()).is_err());
        assert!(read_ssh_keys(Some("/nonexistent/key.pub")).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
