//! Atlas updating itself from its GitHub releases (atlas-release's
//! `app_update`): look for a newer release on start and every few hours
//! when the setting is on, tell the window (`atlas://app-update`), and on
//! the user's word download the installer for how Atlas is installed here,
//! check it, install it and restart.
//!
//! Installing a system package needs root: rpm and deb go through pkexec
//! (the desktop's own password prompt). An AppImage replaces its own file.
//! On Windows the setup program runs and Atlas quits so it can replace it;
//! on macOS the disk image opens.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use atlas_release::{
    AppRelease, AppUpdateQuery, InstallKind, PublicKey, VerifiedInstaller, download_app_installer,
    latest_app_release,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::state::AppState;

/// The version this build reports: the release's (CI sets ATLAS_VERSION
/// from the tag), else `<release>-dev.<count>+<commit>` for a build from a
/// checkout (scripts/build-version.rs).
pub fn version() -> &'static str {
    env!("ATLAS_BUILD_VERSION")
}

/// Whether this is a release build rather than a development one.
pub fn is_release_build() -> bool {
    !version().contains("-dev")
}

/// The keys release manifests must be signed with, one `ed25519:<hex>` per
/// line. Empty until a release key exists: then installers are checked
/// against their published SHA-256 only, and the window says so.
const UPDATE_KEYS: &str = include_str!("../update-keys.txt");

fn keys() -> Vec<PublicKey> {
    UPDATE_KEYS
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| PublicKey::parse(line).ok())
        .collect()
}

/// What to look for and where; part of the app's settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppUpdateSettings {
    /// Look for a new Atlas on start and every few hours.
    pub auto_check: bool,
    /// Offer pre-releases too.
    pub prereleases: bool,
    /// The GitHub repository whose releases are Atlas's (`owner/name`).
    pub repo: String,
}

impl Default for AppUpdateSettings {
    fn default() -> Self {
        Self {
            auto_check: true,
            prereleases: false,
            repo: "Prometheus-Dynamics/Atlas-Hardware-Manager".into(),
        }
    }
}

const RECHECK: Duration = Duration::from_secs(6 * 60 * 60);
const FIRST_CHECK: Duration = Duration::from_secs(20);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Phase {
    #[default]
    Idle,
    Checking,
    Downloading,
    Installing,
    /// Installed; restarting (or, on Windows, the setup program took over).
    Restarting,
}

/// What the window shows (`atlas://app-update` carries it on each change).
#[derive(Clone, Debug, Default, Serialize)]
pub struct AppUpdateStatus {
    pub current: String,
    pub release_build: bool,
    pub kind: Option<InstallKind>,
    pub phase: Phase,
    /// 0..1 while downloading.
    pub progress: Option<f32>,
    pub last_checked_ms: Option<u64>,
    /// A newer release, if there is one.
    pub available: Option<AppRelease>,
    /// Release manifests are checked against a built-in key.
    pub signing: bool,
    pub error: Option<String>,
}

pub struct AppUpdater {
    status: Mutex<AppUpdateStatus>,
    http: reqwest::Client,
}

impl AppUpdater {
    pub fn new() -> Self {
        Self {
            status: Mutex::new(AppUpdateStatus {
                current: version().into(),
                release_build: is_release_build(),
                kind: Some(install_kind()),
                signing: !keys().is_empty(),
                ..AppUpdateStatus::default()
            }),
            http: reqwest::Client::builder()
                .user_agent(concat!("atlas/", env!("ATLAS_BUILD_VERSION")))
                .connect_timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
        }
    }

    pub fn status(&self) -> AppUpdateStatus {
        self.status
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn update(&self, app: &AppHandle, change: impl FnOnce(&mut AppUpdateStatus)) {
        let status = {
            let mut status = self
                .status
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            change(&mut status);
            status.clone()
        };
        let _ = app.emit("atlas://app-update", &status);
    }

    /// Looks for a newer release now.
    pub async fn check(&self, app: &AppHandle, settings: &AppUpdateSettings) -> AppUpdateStatus {
        if matches!(
            self.status().phase,
            Phase::Downloading | Phase::Installing | Phase::Restarting
        ) {
            return self.status();
        }
        self.update(app, |s| {
            s.phase = Phase::Checking;
            s.error = None;
        });
        let kind = install_kind();
        let query = AppUpdateQuery {
            repo: &settings.repo,
            current: version(),
            prereleases: settings.prereleases,
            kind,
            arch: std::env::consts::ARCH,
        };
        let result = latest_app_release(&self.http, &query).await;
        let now = now_ms();
        self.update(app, |s| {
            s.phase = Phase::Idle;
            s.last_checked_ms = Some(now);
            match result {
                Ok(available) => s.available = available,
                Err(error) => s.error = Some(error.to_string()),
            }
        });
        self.status()
    }

    /// Downloads, checks and installs the available release, then restarts.
    pub async fn install(&self, app: &AppHandle, cache_dir: &Path) -> Result<(), String> {
        let status = self.status();
        let release = status
            .available
            .clone()
            .ok_or("there is no newer Atlas to install")?;
        let kind = install_kind();
        if kind == InstallKind::Source {
            return Err(format!(
                "this Atlas wasn't installed from a package, so it can't replace itself; get {} from {}",
                release.version, release.url
            ));
        }
        if release.installer.is_none() {
            return Err(format!(
                "{} has no installer for this computer; see {}",
                release.version, release.url
            ));
        }
        if matches!(
            status.phase,
            Phase::Downloading | Phase::Installing | Phase::Restarting
        ) {
            return Err("an update is already under way".into());
        }
        self.update(app, |s| {
            s.phase = Phase::Downloading;
            s.progress = Some(0.0);
            s.error = None;
        });
        let dir = cache_dir.join("atlas-updates");
        let progress_app = app.clone();
        let downloaded = download_app_installer(&self.http, &release, &keys(), &dir, move |p| {
            if let Some(total) = p.total.filter(|t| *t > 0) {
                let fraction = (p.downloaded as f32 / total as f32).min(1.0);
                let updater = progress_app.state::<AppState>();
                let current = updater.app_update.status().progress.unwrap_or(0.0);
                // A few events per percent is plenty.
                if fraction - current >= 0.01 || fraction >= 1.0 {
                    updater
                        .app_update
                        .update(&progress_app, |s| s.progress = Some(fraction));
                }
            }
        })
        .await;
        let installer = match downloaded {
            Ok(installer) => installer,
            Err(error) => {
                let message = error.to_string();
                self.update(app, |s| {
                    s.phase = Phase::Idle;
                    s.progress = None;
                    s.error = Some(message.clone());
                });
                return Err(message);
            }
        };
        crate::logfile::line(
            "app-update",
            &format!(
                "{} downloaded ({}, sha256 {}{})",
                release.version,
                installer.path.display(),
                installer.sha256,
                if installer.signed {
                    ", signed"
                } else {
                    ", not signed"
                }
            ),
        );
        self.update(app, |s| {
            s.phase = Phase::Installing;
            s.progress = None;
        });
        let installed = tokio::task::spawn_blocking(move || run_installer(kind, &installer))
            .await
            .map_err(|e| e.to_string())
            .and_then(|r| r);
        match installed {
            Ok(Next::Restart) => {
                self.update(app, |s| s.phase = Phase::Restarting);
                // Let the window show it before the process is replaced.
                tokio::time::sleep(Duration::from_millis(600)).await;
                app.restart();
            }
            Ok(Next::Quit) => {
                self.update(app, |s| s.phase = Phase::Restarting);
                tokio::time::sleep(Duration::from_millis(600)).await;
                app.exit(0);
                Ok(())
            }
            Ok(Next::Stay) => {
                self.update(app, |s| s.phase = Phase::Idle);
                Ok(())
            }
            Err(error) => {
                self.update(app, |s| {
                    s.phase = Phase::Idle;
                    s.error = Some(error.clone());
                });
                Err(error)
            }
        }
    }
}

/// Checks on start and every few hours while the setting is on.
pub fn start(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK).await;
        loop {
            let state = app.state::<AppState>();
            let settings = state.settings().app_updates;
            if settings.auto_check {
                state.app_update.check(&app, &settings).await;
            }
            tokio::time::sleep(RECHECK).await;
        }
    });
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// How this Atlas got here, which says which installer replaces it.
pub fn install_kind() -> InstallKind {
    let Ok(exe) = std::env::current_exe() else {
        return InstallKind::Source;
    };
    detect_kind(&exe)
}

#[cfg(target_os = "linux")]
fn detect_kind(exe: &Path) -> InstallKind {
    if std::env::var_os("APPIMAGE").is_some() {
        return InstallKind::AppImage;
    }
    let owned_by = |program: &str, args: &[&str]| {
        Command::new(program)
            .args(args)
            .arg(exe)
            .output()
            .is_ok_and(|out| out.status.success())
    };
    if exe.starts_with("/usr") {
        if owned_by("rpm", &["-qf", "--quiet"]) {
            return InstallKind::Rpm;
        }
        if owned_by("dpkg", &["-S"]) {
            return InstallKind::Deb;
        }
    }
    InstallKind::Source
}

#[cfg(target_os = "windows")]
fn detect_kind(exe: &Path) -> InstallKind {
    let dir = exe.parent().unwrap_or(exe);
    if dir.join("uninstall.exe").exists() {
        InstallKind::Nsis
    } else if dir.components().any(|c| c.as_os_str() == "Program Files") {
        InstallKind::Msi
    } else {
        InstallKind::Source
    }
}

#[cfg(target_os = "macos")]
fn detect_kind(exe: &Path) -> InstallKind {
    if exe.to_string_lossy().contains(".app/Contents/MacOS/") {
        InstallKind::Dmg
    } else {
        InstallKind::Source
    }
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
fn detect_kind(_exe: &Path) -> InstallKind {
    InstallKind::Source
}

enum Next {
    Restart,
    Quit,
    /// Nothing more Atlas can do (a disk image the user drags).
    Stay,
}

fn run(program: &str, args: &[&str], file: &Path) -> Result<(), String> {
    let output = Command::new(program)
        .args(args)
        .arg(file)
        .output()
        .map_err(|e| format!("could not run {program}: {e}"))?;
    if output.status.success() {
        return Ok(());
    }
    // pkexec: 126 when the prompt was dismissed, 127 when not authorized.
    if program == "pkexec" && matches!(output.status.code(), Some(126 | 127)) {
        return Err("the update needs your password to install; it wasn't given".into());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let last = stderr
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("");
    Err(format!(
        "installing failed ({}){}",
        output.status,
        if last.is_empty() {
            String::new()
        } else {
            format!(": {last}")
        }
    ))
}

fn have(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
}

fn run_installer(kind: InstallKind, installer: &VerifiedInstaller) -> Result<Next, String> {
    let file = installer.path.as_path();
    match kind {
        InstallKind::Rpm => {
            if have("dnf") {
                run("pkexec", &["dnf", "install", "-y"], file)?;
            } else {
                run("pkexec", &["rpm", "-U", "--replacepkgs"], file)?;
            }
            Ok(Next::Restart)
        }
        InstallKind::Deb => {
            run("pkexec", &["apt-get", "install", "-y"], file)?;
            Ok(Next::Restart)
        }
        InstallKind::AppImage => {
            let target = PathBuf::from(std::env::var_os("APPIMAGE").ok_or("APPIMAGE isn't set")?);
            replace_appimage(file, &target)?;
            Ok(Next::Restart)
        }
        InstallKind::Nsis => {
            Command::new(file)
                .spawn()
                .map_err(|e| format!("could not start the installer: {e}"))?;
            Ok(Next::Quit)
        }
        InstallKind::Msi => {
            Command::new("msiexec")
                .arg("/i")
                .arg(file)
                .arg("/passive")
                .spawn()
                .map_err(|e| format!("could not start the installer: {e}"))?;
            Ok(Next::Quit)
        }
        InstallKind::Dmg => {
            Command::new("open")
                .arg(file)
                .spawn()
                .map_err(|e| format!("could not open the disk image: {e}"))?;
            Ok(Next::Stay)
        }
        InstallKind::Source => Err("this Atlas wasn't installed from a package".into()),
    }
}

/// Puts the new AppImage in place of the running one, by rename on the same
/// filesystem, so a failure leaves the old one.
fn replace_appimage(new: &Path, target: &Path) -> Result<(), String> {
    let staged = target.with_extension("new");
    std::fs::copy(new, &staged).map_err(|e| format!("{}: {e}", staged.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("{}: {e}", staged.display()))?;
    }
    std::fs::rename(&staged, target).map_err(|e| format!("{}: {e}", target.display()))
}
