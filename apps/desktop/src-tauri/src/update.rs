//! Atlas updating itself from a signed release feed (`atlas-release`'s `app_update`).
//!
//! - **The source** is configuration: the feed's address is the settings file's `update_feed`,
//!   else the one the build was given (`ATLAS_UPDATE_FEED` at build time). With neither there is
//!   no source, the page says so, and nothing is ever asked of the network.
//! - **A check** reads the feed and looks for a release of the Settings channel that is newer
//!   than this build and has an installer for how Atlas was installed here. It runs in the
//!   background. A feed that cannot be reached is asked again, the wait doubling from one second
//!   to five and never more, until it answers or the person stops it (the failure rule). A feed
//!   that answers with something Atlas refuses (no trusted signature, a manifest that is not
//!   one) is not asked again on its own: the refusal is shown in the library's words.
//! - **Trust.** A release is offered only when its manifest is signed by a trusted, unexpired
//!   key, and its installer is used only when it is the size and SHA-256 that manifest says.
//!   The security switch does not loosen this: an unsigned Atlas is never offered. The `dev`
//!   channel accepts a key kept for dev builds; `beta` and `stable` need a production key.
//! - **The download** is a job like any other ([`KIND`]), resumed from its partial file.
//! - **Applying** is as far as Atlas can go without being root. A system package (`.deb`,
//!   `.rpm`, `.msi`, a setup `.exe`, a `.dmg`) is handed to the system's own installer, which
//!   asks for what it needs; Atlas does not run it as root itself. An AppImage is one file the
//!   person owns: Atlas replaces it and says to start Atlas again. A copy that was not
//!   installed from a package has nothing to replace, and the page shows where the file is.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use atlas_driver::DriverError;
use atlas_jobs::{Backoff, DeviceClaim, JobId, JobKind};
use atlas_release::{
    AppQuery, AppUpdate, Fetch, InstallKind, ReleaseError, VerifiedInstaller, check_app_update,
    download_app_update,
};
use serde::Serialize;
use serde_json::Value;
use tokio::sync::Notify;

use crate::call::{CallError, CallResult};
use crate::core::{Core, now_ms};

/// The topic the Settings page reads.
pub const TOPIC: &str = "update";
/// The job kind of an installer download.
pub const KIND: &str = "app-update";
const STEP: &str = "download";
/// The wait between tries of a feed that does not answer: doubled from the first to the cap.
pub const RETRY: Backoff = Backoff {
    first: Duration::from_secs(1),
    cap: Duration::from_secs(5),
};
/// The feed a build was given, if any.
const BUILT_IN_FEED: Option<&str> = option_env!("ATLAS_UPDATE_FEED");

/// Where the update is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Phase {
    /// Nothing asked yet, or the last check found nothing.
    #[default]
    Idle,
    Checking,
    /// The feed did not answer; it is asked again after a wait.
    Retrying,
    /// The feed answered with something Atlas refuses.
    Refused,
    Available,
    Downloading,
    /// The download failed its check, or cannot go on.
    Failed,
    /// The installer is on this computer and checked.
    Ready,
    /// Handed to the system's installer, or (an AppImage) replaced.
    Applied,
}

#[derive(Clone, Debug, Default)]
struct State {
    phase: Phase,
    /// Counts checks, so a stopped or replaced check's late answer is dropped.
    generation: u64,
    checked_ms: Option<u64>,
    /// The last check found nothing newer.
    up_to_date: bool,
    available: Option<AppUpdate>,
    error: Option<String>,
    retry_at_ms: Option<u64>,
    attempt: u32,
    downloaded: u64,
    job: Option<JobId>,
    installer: Option<VerifiedInstaller>,
    /// What applying did, in words.
    applied: Option<String>,
}

/// How this copy was installed, and what an AppImage's own file is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Install {
    pub kind: InstallKind,
    pub app_image: Option<PathBuf>,
}

impl Install {
    /// How the running program was installed.
    #[must_use]
    pub fn of_this_program() -> Self {
        let app_image = std::env::var_os("APPIMAGE")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        let kind = std::env::current_exe()
            .map(|exe| install_kind(&exe, app_image.is_some()))
            .unwrap_or(InstallKind::Source);
        Self { kind, app_image }
    }
}

/// How the program at `exe` was installed.
///
/// - Run from an AppImage (the runtime sets `APPIMAGE`): an AppImage.
/// - Linux packages say what they are in a one-word file they install,
///   `<prefix>/lib/atlas-hardware-manager/install-kind` (`deb` or `rpm`; scripts/deb-fast.sh and
///   the bundle configuration write it). No package tool is run to find out.
/// - Windows and macOS are told by where the installers put the program (carried from `dev`,
///   because that is how those installers lay it out; not run on this branch).
/// - Anything else was built from source or copied by hand: nothing to replace.
#[must_use]
pub fn install_kind(exe: &Path, app_image: bool) -> InstallKind {
    if app_image {
        return InstallKind::AppImage;
    }
    let dir = exe.parent().unwrap_or(exe);
    if cfg!(target_os = "windows") {
        return if dir.join("uninstall.exe").exists() {
            InstallKind::Nsis
        } else if dir
            .components()
            .any(|part| part.as_os_str() == "Program Files")
        {
            InstallKind::Msi
        } else {
            InstallKind::Source
        };
    }
    if cfg!(target_os = "macos") {
        return if dir.ends_with("Contents/MacOS") {
            InstallKind::Dmg
        } else {
            InstallKind::Source
        };
    }
    let marker = dir
        .parent()
        .map(|prefix| prefix.join("lib/atlas-hardware-manager/install-kind"));
    match marker
        .and_then(|path| std::fs::read_to_string(path).ok())
        .as_deref()
        .map(str::trim)
    {
        Some("deb") => InstallKind::Deb,
        Some("rpm") => InstallKind::Rpm,
        _ => InstallKind::Source,
    }
}

struct Shared {
    state: Mutex<State>,
    dirty: Notify,
    fetch: Arc<dyn Fetch>,
    install: Mutex<Install>,
    /// A feed set while running (the simulated world's). It wins over the settings.
    feed: Mutex<Option<String>>,
    /// Where installers are kept.
    dir: PathBuf,
    /// The waits between tries of a feed that does not answer.
    retry: Backoff,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

/// The app's self-update state. Held by the core.
#[derive(Clone)]
pub struct Updates {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for Updates {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Updates").finish_non_exhaustive()
    }
}

/// A fetcher for a build that could not make its HTTP client: every request says why.
struct NoNetwork(String);

#[async_trait]
impl Fetch for NoNetwork {
    async fn get(&self, _url: &str, _offset: u64) -> Result<atlas_release::Reply, String> {
        Err(self.0.clone())
    }
}

struct DownloadKind(Arc<Shared>);

#[async_trait]
impl JobKind for DownloadKind {
    fn name(&self) -> &str {
        KIND
    }

    /// The offer is not saved: after a restart the person checks again, and the download
    /// goes on from its partial file.
    fn resumable(&self) -> bool {
        false
    }

    async fn run_step(&self, _claim: &DeviceClaim, _step: &str) -> Result<(), DriverError> {
        let shared = &self.0;
        let update = lock(&shared.state).available.clone().ok_or_else(|| {
            DriverError::Incompatible("there is no update to download any more".into())
        })?;
        let progress = |progress: atlas_release::DownloadProgress| {
            lock(&shared.state).downloaded = progress.downloaded;
            shared.dirty.notify_one();
        };
        let result =
            download_app_update(shared.fetch.as_ref(), &update, &shared.dir, &progress).await;
        let mut state = lock(&shared.state);
        // Cancelled, or another check replaced the offer, while the last bytes arrived.
        if state.phase != Phase::Downloading || state.available.as_ref() != Some(&update) {
            return Ok(());
        }
        let outcome = match result {
            Ok(installer) => {
                state.phase = Phase::Ready;
                state.installer = Some(installer);
                Ok(())
            }
            // A disk that is full or a directory that is gone can change: tried again.
            Err(error @ ReleaseError::Io { .. }) => {
                state.error = Some(error.to_string());
                Err(DriverError::Other(error.to_string()))
            }
            // Everything else is the file or the server saying no: not tried again.
            Err(error) => {
                state.phase = Phase::Failed;
                state.error = Some(error.to_string());
                state.job = None;
                Err(DriverError::Incompatible(error.to_string()))
            }
        };
        drop(state);
        shared.dirty.notify_one();
        outcome
    }
}

/// One line of words, and the facts the page chooses its controls by.
#[derive(Serialize)]
struct View {
    /// Whether a feed is configured at all.
    source: bool,
    kind: InstallKind,
    phase: Phase,
    /// What to say on the Atlas line.
    words: String,
    /// `faint`, `ok`, `acc` or `err`: how the words are said.
    tone: &'static str,
    checked_ms: Option<u64>,
    version: Option<String>,
    size: Option<u64>,
    downloaded: u64,
    retry_at_ms: Option<u64>,
    /// What the apply button does, when there is one.
    apply: Option<&'static str>,
    installer: Option<String>,
}

impl Updates {
    /// The update state. `fetch` is the network (the simulated world gives its own); `data_dir`
    /// is where installers are kept; a feed that does not answer is asked again after `retry`
    /// ([`RETRY`] in the app).
    #[must_use]
    pub fn new(fetch: Option<Arc<dyn Fetch>>, data_dir: Option<&Path>, retry: Backoff) -> Self {
        let fetch = fetch.unwrap_or_else(|| match atlas_release::HttpFetcher::new() {
            Ok(fetcher) => Arc::new(fetcher),
            Err(error) => Arc::new(NoNetwork(error)),
        });
        let dir = data_dir.map_or_else(
            || std::env::temp_dir().join(format!("atlas-updates-{}", std::process::id())),
            |dir| dir.join("updates"),
        );
        Self {
            shared: Arc::new(Shared {
                state: Mutex::new(State::default()),
                dirty: Notify::new(),
                fetch,
                install: Mutex::new(Install::of_this_program()),
                feed: Mutex::new(None),
                dir,
                retry,
            }),
        }
    }

    /// The job kind the core registers.
    #[must_use]
    pub fn kind(&self) -> Arc<dyn JobKind> {
        Arc::new(DownloadKind(Arc::clone(&self.shared)))
    }

    /// Sets the feed for this run. `None` goes back to the settings.
    pub fn set_feed(&self, feed: Option<String>) {
        *lock(&self.shared.feed) = feed;
        self.shared.dirty.notify_one();
    }

    /// Says how this copy was installed, where the program cannot tell (a test).
    pub fn set_install(&self, install: Install) {
        *lock(&self.shared.install) = install;
        self.shared.dirty.notify_one();
    }

    fn feed(&self, core: &Core) -> Option<String> {
        lock(&self.shared.feed)
            .clone()
            .or_else(|| Some(core.settings().update_feed).filter(|feed| !feed.is_empty()))
            .or_else(|| BUILT_IN_FEED.map(str::to_owned))
            .filter(|feed| !feed.is_empty())
    }

    fn view(&self, core: &Core) -> View {
        let state = lock(&self.shared.state).clone();
        let kind = lock(&self.shared.install).kind;
        let source = self.feed(core).is_some();
        let version = state
            .available
            .as_ref()
            .map(|update| update.version.clone());
        let named = version.clone().unwrap_or_default();
        let (words, tone) = match state.phase {
            _ if !source => ("no update source configured".to_owned(), "faint"),
            Phase::Idle if state.up_to_date => ("up to date".to_owned(), "ok"),
            Phase::Idle => (String::new(), "faint"),
            Phase::Checking => ("checking…".to_owned(), "faint"),
            Phase::Retrying => (
                format!(
                    "the update feed did not answer (try {}): {}",
                    state.attempt,
                    state.error.clone().unwrap_or_default()
                ),
                "err",
            ),
            Phase::Refused | Phase::Failed => (state.error.clone().unwrap_or_default(), "err"),
            Phase::Available => (format!("{named} is available"), "acc"),
            Phase::Downloading => (format!("downloading {named}"), "acc"),
            Phase::Ready => (format!("{named} is downloaded and checked"), "ok"),
            Phase::Applied => (state.applied.clone().unwrap_or_default(), "ok"),
        };
        let apply = (state.phase == Phase::Ready).then_some(match kind {
            InstallKind::AppImage => "Replace this AppImage",
            InstallKind::Source => "Show the file",
            _ => "Open installer",
        });
        View {
            source,
            kind,
            phase: state.phase,
            words,
            tone,
            checked_ms: state.checked_ms,
            version,
            size: state.available.as_ref().map(|update| update.size),
            downloaded: state.downloaded,
            retry_at_ms: state.retry_at_ms,
            apply,
            installer: state
                .installer
                .as_ref()
                .map(|installer| installer.path.display().to_string()),
        }
    }

    /// Publishes the topic whenever the state changed. With a source configured, it checks once
    /// when the app opens.
    pub async fn follow(self, core: Core) {
        let publish = |updates: &Self| {
            if let Ok(data) = serde_json::to_value(updates.view(&core)) {
                core.publish(TOPIC, data);
            }
        };
        publish(&self);
        if self.feed(&core).is_some() {
            let _ = self.check(&core);
        }
        loop {
            self.shared.dirty.notified().await;
            publish(&self);
        }
    }

    /// Starts a check in the background. Refused while an installer is being downloaded.
    pub fn check(&self, core: &Core) -> Result<(), CallError> {
        let feed = self
            .feed(core)
            .ok_or_else(|| CallError::from("No update source is configured."))?;
        let generation = {
            let mut state = lock(&self.shared.state);
            if state.phase == Phase::Downloading {
                return Err("An update is being downloaded.".into());
            }
            *state = State {
                generation: state.generation + 1,
                checked_ms: state.checked_ms,
                phase: Phase::Checking,
                ..State::default()
            };
            state.generation
        };
        self.shared.dirty.notify_one();
        let shared = Arc::clone(&self.shared);
        let channel = core.settings().channel;
        let keys = core.trust().keyring().manifest_keys(channel != "dev");
        let kind = lock(&shared.install).kind;
        let log = core.log().clone();
        tokio::spawn(async move {
            let mut wait = shared.retry.first;
            loop {
                let query = AppQuery {
                    feed_url: &feed,
                    channel: &channel,
                    current: crate::VERSION,
                    kind,
                    arch: std::env::consts::ARCH,
                };
                let answer = check_app_update(shared.fetch.as_ref(), &keys, query).await;
                {
                    let mut state = lock(&shared.state);
                    if state.generation != generation {
                        return;
                    }
                    let again = match answer {
                        Ok(found) => {
                            state.checked_ms = Some(now_ms());
                            state.error = None;
                            state.retry_at_ms = None;
                            state.up_to_date = found.is_none();
                            state.phase = if found.is_some() {
                                Phase::Available
                            } else {
                                Phase::Idle
                            };
                            state.available = found;
                            false
                        }
                        // The feed could not be read: it may answer later.
                        Err(error @ ReleaseError::Download { .. }) => {
                            state.phase = Phase::Retrying;
                            state.attempt += 1;
                            // The line is short: the reason without the address. The log has both.
                            state.error = Some(match &error {
                                ReleaseError::Download { message, .. } => message.clone(),
                                other => other.to_string(),
                            });
                            state.retry_at_ms = Some(
                                now_ms() + u64::try_from(wait.as_millis()).unwrap_or(u64::MAX),
                            );
                            if state.attempt == 1 {
                                log.failed_call("update.check", &feed, &error.to_string());
                            }
                            true
                        }
                        // The feed answered, and Atlas refuses what it said.
                        Err(error) => {
                            state.checked_ms = Some(now_ms());
                            state.phase = Phase::Refused;
                            state.error = Some(error.to_string());
                            log.failed_call("update.check", &feed, &error.to_string());
                            false
                        }
                    };
                    drop(state);
                    shared.dirty.notify_one();
                    if !again {
                        return;
                    }
                }
                tokio::time::sleep(wait).await;
                wait = (wait * 2).min(shared.retry.cap);
                if lock(&shared.state).generation != generation {
                    return;
                }
            }
        });
        Ok(())
    }

    /// The manual off: stops asking the feed. Nothing is asked again until the person checks.
    pub fn stop(&self) {
        let mut state = lock(&self.shared.state);
        if matches!(state.phase, Phase::Checking | Phase::Retrying) {
            *state = State {
                generation: state.generation + 1,
                checked_ms: state.checked_ms,
                ..State::default()
            };
        }
        drop(state);
        self.shared.dirty.notify_one();
    }

    /// Starts downloading the offered installer, as a job.
    pub async fn download(&self, core: &Core) -> Result<(), CallError> {
        {
            let mut state = lock(&self.shared.state);
            if !matches!(state.phase, Phase::Available | Phase::Failed) || state.available.is_none()
            {
                return Err("There is no update to download.".into());
            }
            state.phase = Phase::Downloading;
            state.error = None;
            state.downloaded = 0;
        }
        self.shared.dirty.notify_one();
        match core
            .jobs()
            .submit(KIND, DeviceClaim::none(), vec![STEP.to_owned()])
            .await
        {
            Ok(job) => {
                lock(&self.shared.state).job = Some(job);
                Ok(())
            }
            Err(error) => {
                let mut state = lock(&self.shared.state);
                state.phase = Phase::Available;
                drop(state);
                self.shared.dirty.notify_one();
                Err(CallError(error.to_string()))
            }
        }
    }

    /// Stops the download. What arrived stays in its partial file and is used next time.
    pub fn cancel(&self, core: &Core) {
        let job = {
            let mut state = lock(&self.shared.state);
            if state.phase != Phase::Downloading {
                return;
            }
            state.phase = Phase::Available;
            state.job.take()
        };
        if let Some(job) = job {
            core.jobs().cancel(job);
        }
        self.shared.dirty.notify_one();
    }

    /// Applies the downloaded installer as far as Atlas can without being root (see the module
    /// text). The file is hashed again first: it is used only if it is still what was checked.
    pub async fn apply(&self, core: &Core) -> Result<(), CallError> {
        let (installer, version) = {
            let state = lock(&self.shared.state);
            match (&state.installer, &state.available, state.phase) {
                (Some(installer), Some(update), Phase::Ready) => {
                    (installer.clone(), update.version.clone())
                }
                _ => return Err("There is no downloaded update.".into()),
            }
        };
        let again = atlas_release::sha256_file(&installer.path)
            .await
            .map_err(|error| CallError(error.to_string()))?;
        if again != installer.sha256 {
            let mut state = lock(&self.shared.state);
            state.phase = Phase::Failed;
            state.installer = None;
            state.error = Some("the downloaded file changed after it was checked".into());
            drop(state);
            self.shared.dirty.notify_one();
            return Err(
                "The downloaded file changed after it was checked. Download it again.".into(),
            );
        }
        let install = lock(&self.shared.install).clone();
        let said = match install.kind {
            InstallKind::AppImage => {
                let target = install
                    .app_image
                    .ok_or_else(|| CallError::from("This AppImage's own file is not known."))?;
                let from = installer.path.clone();
                let to = target.clone();
                tokio::task::spawn_blocking(move || replace_file(&from, &to))
                    .await
                    .map_err(|error| CallError(error.to_string()))?
                    .map_err(|error| {
                        CallError(format!(
                            "{} could not be replaced: {error}",
                            target.display()
                        ))
                    })?;
                format!("replaced with {version}: start Atlas again to run it")
            }
            InstallKind::Source => {
                let folder = installer.path.parent().unwrap_or(&installer.path);
                core.opener()
                    .open(&folder.display().to_string())
                    .map_err(CallError)?;
                // Nothing was applied: the file is shown, and the offer stays.
                return Ok(());
            }
            _ => {
                core.opener()
                    .open(&installer.path.display().to_string())
                    .map_err(CallError)?;
                format!("{version}: handed to the system's installer")
            }
        };
        core.log().breadcrumb(&format!("update applied: {said}"));
        let mut state = lock(&self.shared.state);
        state.phase = Phase::Applied;
        state.applied = Some(said);
        drop(state);
        self.shared.dirty.notify_one();
        Ok(())
    }
}

/// Puts `from` in place of `to`: copied beside it, made runnable, then renamed over it, so `to`
/// is always a whole file. A program that is running keeps its old file until it exits.
fn replace_file(from: &Path, to: &Path) -> std::io::Result<()> {
    let name = to.file_name().unwrap_or_default().to_string_lossy();
    let temp = to.with_file_name(format!(".{name}.new"));
    std::fs::copy(from, &temp)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o755))?;
    }
    std::fs::rename(&temp, to).inspect_err(|_| {
        let _ = std::fs::remove_file(&temp);
    })
}

/// The `update.*` calls.
pub async fn call(core: &Core, name: &str, _arguments: Value) -> CallResult {
    let updates = core.app_update();
    match name {
        "update.check" => {
            core.log().breadcrumb("update: check");
            updates.check(core)?;
        }
        "update.stop" => {
            core.log().breadcrumb("update: stopped asking");
            updates.stop();
        }
        "update.download" => {
            core.log().breadcrumb("update: download");
            updates.download(core).await?;
        }
        "update.cancel" => {
            core.log().breadcrumb("update: download cancelled");
            updates.cancel(core);
        }
        "update.apply" => updates.apply(core).await?,
        _ => return Err(CallError(format!("no call named `{name}` in this build"))),
    }
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_linux_package_says_what_it_is_in_its_marker_and_anything_else_is_source() {
        let root = tempfile::tempdir().unwrap();
        let exe = root.path().join("usr/bin/atlas-app");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        assert_eq!(install_kind(&exe, true), InstallKind::AppImage);
        if cfg!(target_os = "linux") {
            assert_eq!(install_kind(&exe, false), InstallKind::Source);
            let marker = root
                .path()
                .join("usr/lib/atlas-hardware-manager/install-kind");
            std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
            std::fs::write(&marker, "deb\n").unwrap();
            assert_eq!(install_kind(&exe, false), InstallKind::Deb);
            std::fs::write(&marker, "rpm").unwrap();
            assert_eq!(install_kind(&exe, false), InstallKind::Rpm);
            std::fs::write(&marker, "something else").unwrap();
            assert_eq!(install_kind(&exe, false), InstallKind::Source);
        }
    }

    #[test]
    fn replacing_a_file_leaves_the_new_bytes_runnable_and_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let (from, to) = (dir.path().join("new"), dir.path().join("Atlas.AppImage"));
        std::fs::write(&from, b"new program").unwrap();
        std::fs::write(&to, b"old program").unwrap();
        replace_file(&from, &to).unwrap();
        assert_eq!(std::fs::read(&to).unwrap(), b"new program");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(to.metadata().unwrap().permissions().mode() & 0o777, 0o755);
        }
        // A target whose directory is gone fails and leaves nothing behind.
        assert!(replace_file(&from, &dir.path().join("gone/Atlas.AppImage")).is_err());
    }
}
