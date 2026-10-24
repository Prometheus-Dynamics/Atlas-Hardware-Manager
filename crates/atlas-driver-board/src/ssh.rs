//! A/B updates over SSH (docs/ota.md) for boards whose identity lists
//! `ab-tryboot`: Atlas streams the same disk image that is flashed over USB
//! (`.img`, `.img.xz`, `.img.zst`, `.img.gz`) to `/data` with its SHA-256,
//! and the device package's writer (`/usr/lib/board/update`) copies the
//! image's slot A into the board's spare slot,
//! trial-boots it, and keeps it once it's healthy. The outcome is read from
//! the writer's durable state after the reboot, never from a command that
//! the reboot cut off.
//!
//! Atlas runs the system's OpenSSH client as root on the board, with the
//! key the Flash tab provisions. Host keys are trusted on first use and
//! pinned per board (`HostKeyAlias`), since every board answers on the same
//! USB gadget address.

use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use atlas_driver::{
    CancellationToken, Concurrency, DeviceKey, DriverError, Identity, ProgressSink, ReleaseRef,
    UpdateCapability, UpdateOutcome, UpdatePlan, UpdateStep,
};
use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

use atlas_driver::{TrialStop, trial_stopped};

use crate::console::{Console, ConsoleError, SharedOnLine};

mod access;
pub mod host_keys;
pub use access::{SshAccess, SshConfig, private_key_for};

/// The update method a board lists when it has the A/B layout and writer.
pub const AB_METHOD: &str = "ab-tryboot";

pub(crate) const WRITER: &str = "/usr/lib/board/update";
const REMOTE_DIR: &str = "/data/board/update";
/// The writer detects the compression from the file's first bytes.
const REMOTE_IMAGE: &str = "/data/board/update/image";
const IMAGE_SUFFIXES: [&str; 4] = [".img", ".img.xz", ".img.zst", ".img.gz"];
const POLL: Duration = Duration::from_secs(3);
/// Verify and write both slot images on a slow eMMC.
const STAGE_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// From `apply` to a new boot id over SSH.
const REBOOT_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// From the new boot to `confirmed` or `rolled-back`; the device waits up
/// to a few minutes for its application before judging it.
const CONFIRM_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// What `update status` prints. Every field may be missing or `null` (a
/// freshly flashed board has no staged version, no error), which reads as
/// empty or 0; fields Atlas doesn't use here (`version_previous`,
/// `started_by`, `phase`, ...) are ignored whatever they hold.
#[derive(Clone, Debug, Default, Deserialize)]
struct Status {
    #[serde(default, deserialize_with = "null_as_default")]
    state: String,
    #[serde(default, deserialize_with = "null_as_default")]
    version_active: String,
    #[serde(default, deserialize_with = "null_as_default")]
    version_staged: String,
    #[serde(default, deserialize_with = "null_as_default")]
    progress: u32,
    #[serde(default, deserialize_with = "null_as_default")]
    error: String,
}

/// `null` as the type's default, for the update writer's optional fields.
fn null_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

pub(crate) struct SshUpdate {
    pub(crate) access: SshAccess,
    pub(crate) host: String,
    pub(crate) key: DeviceKey,
    /// The board's USB serial: when it is plugged into this computer, its
    /// console runs commands first (console.rs), so no SSH key is needed.
    pub(crate) usb_serial: Option<String>,
}

/// The longest a command may take on the console (the self-test's own limit
/// is shorter).
const CONSOLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);

impl SshUpdate {
    fn command(&self) -> Command {
        let config = self.access.get();
        let mut command = Command::new("ssh");
        command
            .args(["-o", "BatchMode=yes"])
            .args(["-o", "ConnectTimeout=8"])
            .args(["-o", "ServerAliveInterval=5", "-o", "ServerAliveCountMax=3"])
            .args(["-o", "StrictHostKeyChecking=accept-new"])
            // Plain entries, so a board's can be found and dropped (Debian
            // hashes them by default).
            .args(["-o", "HashKnownHosts=no"])
            .arg("-o")
            .arg(format!("HostKeyAlias={}", self.alias()));
        if let Some(known_hosts) = &config.known_hosts {
            if let Some(dir) = known_hosts.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            host_keys::drop_hashed_once(known_hosts);
            command
                .arg("-o")
                .arg(format!("UserKnownHostsFile={}", known_hosts.display()));
        }
        if let Some(identity) = &config.identity_file {
            command
                .arg("-i")
                .arg(identity)
                .args(["-o", "IdentitiesOnly=yes"]);
        }
        command
            .arg(format!("root@{}", self.host))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        command
    }

    /// This board's `HostKeyAlias` in Atlas's known_hosts.
    pub(crate) fn alias(&self) -> String {
        host_keys::alias(&self.key.to_string())
    }

    /// Atlas just changed what the board runs: a new host key from it is
    /// expected for a while, and taken without asking.
    pub(crate) fn expect_new_host_key(&self) {
        host_keys::expect_new_key(&self.alias());
    }

    /// Drops the board's pinned host key from Atlas's known_hosts (the next
    /// connection pins the one it has now).
    pub(crate) fn forget_host_key(&self) -> Result<(), DriverError> {
        let config = self.access.get();
        let Some(known_hosts) = &config.known_hosts else {
            return Err(DriverError::Incompatible(
                "Atlas uses your own known_hosts for boards; remove the board's entry there".into(),
            ));
        };
        if host_keys::forget(known_hosts, &self.alias()) {
            Ok(())
        } else {
            Err(DriverError::Other(format!(
                "couldn't update {} (is ssh-keygen installed?)",
                known_hosts.display()
            )))
        }
    }

    /// Whether the board is plugged into this computer (it has a console).
    pub(crate) fn has_console(&self) -> bool {
        self.console().is_some()
    }

    /// The board's console, when it is plugged into this computer.
    fn console(&self) -> Option<Console> {
        self.usb_serial.as_deref().and_then(Console::find)
    }

    /// Runs `script` on the console when there is one that answers. `None`:
    /// nothing ran there, so SSH may.
    pub(crate) async fn on_console(
        &self,
        script: &str,
        on_line: Option<SharedOnLine>,
    ) -> Option<Result<Vec<u8>, DriverError>> {
        let console = self.console()?;
        match console
            .run(&as_atlas(script), CONSOLE_TIMEOUT, on_line)
            .await
        {
            Ok(out) => Some(Ok(out)),
            Err(ConsoleError::Failed(error)) => Some(Err(error)),
            Err(ConsoleError::NotThere(_)) => None,
        }
    }

    /// Runs `script` on the board as root and returns its stdout: on its
    /// USB console when it is plugged in here, else over SSH. The board's
    /// event log records what it does as Atlas's.
    pub(crate) async fn run(&self, script: &str) -> Result<String, DriverError> {
        self.run_bytes(script)
            .await
            .map(|out| String::from_utf8_lossy(&out).into_owned())
    }

    /// [`run`](Self::run), with stdout as bytes (a tarball).
    pub(crate) async fn run_bytes(&self, script: &str) -> Result<Vec<u8>, DriverError> {
        if let Some(result) = self.on_console(script, None).await {
            return result;
        }
        let mut retried = false;
        loop {
            let output = self
                .command()
                .arg(as_atlas(script))
                .output()
                .await
                .map_err(|error| DriverError::Unreachable(format!("could not run ssh: {error}")))?;
            if output.status.success() {
                return Ok(output.stdout);
            }
            let stderr = String::from_utf8_lossy(&output.stderr);
            // A new OS on the board (Atlas just put it there): its new host
            // key replaces the old pin, once.
            if !retried
                && host_key_changed(&stderr)
                && host_keys::new_key_expected(&self.alias())
                && self.forget_host_key().is_ok()
            {
                retried = true;
                continue;
            }
            return Err(ssh_error(output.status.code(), &stderr));
        }
    }

    /// Like [`run`](Self::run), handing each line the command writes to
    /// stderr to `on_line` as it comes (stdout is returned at the end).
    pub(crate) async fn run_watching(
        &self,
        script: &str,
        on_line: &(dyn Fn(&str) + Send + Sync),
    ) -> Result<String, DriverError> {
        use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};

        // The console hands lines over from its own thread: pass them on
        // through a channel, polled while it runs.
        if self.console().is_some() {
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
            let send: SharedOnLine = Arc::new(move |line: &str| {
                let _ = tx.send(line.to_owned());
            });
            let run = self.on_console(script, Some(send));
            tokio::pin!(run);
            let result = loop {
                tokio::select! {
                    result = &mut run => break result,
                    Some(line) = rx.recv() => on_line(&line),
                }
            };
            while let Ok(line) = rx.try_recv() {
                on_line(&line);
            }
            if let Some(result) = result {
                return result.map(|out| String::from_utf8_lossy(&out).into_owned());
            }
        }
        let mut child = self
            .command()
            .arg(as_atlas(script))
            .spawn()
            .map_err(|error| DriverError::Unreachable(format!("could not run ssh: {error}")))?;
        let mut stdout = child.stdout.take().expect("piped");
        let stderr = child.stderr.take().expect("piped");
        let read_out = async {
            let mut text = String::new();
            let _ = stdout.read_to_string(&mut text).await;
            text
        };
        let read_err = async {
            let mut kept = String::new();
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                on_line(&line);
                kept.push_str(&line);
                kept.push('\n');
            }
            kept
        };
        let (out, err) = tokio::join!(read_out, read_err);
        let status = child
            .wait()
            .await
            .map_err(|error| DriverError::Unreachable(format!("ssh: {error}")))?;
        if status.success() {
            Ok(out)
        } else {
            Err(ssh_error(status.code(), &err))
        }
    }

    async fn status(&self) -> Result<Status, DriverError> {
        let text = self.run(&format!("{WRITER} status")).await?;
        serde_json::from_str(text.trim())
            .map_err(|error| DriverError::Other(format!("unreadable update status: {error}")))
    }

    async fn boot_id(&self) -> Result<String, DriverError> {
        Ok(self
            .run("cat /proc/sys/kernel/random/boot_id")
            .await?
            .trim()
            .to_string())
    }

    /// Streams `path` to the board, reporting the share sent.
    async fn upload(
        &self,
        path: &Path,
        size: u64,
        progress: &ProgressSink,
        cancel: &CancellationToken,
    ) -> Result<(), DriverError> {
        let script = format!(
            "mkdir -p {REMOTE_DIR} && cat > {REMOTE_IMAGE}.part && mv {REMOTE_IMAGE}.part {REMOTE_IMAGE}"
        );
        let cleanup = format!("rm -f {REMOTE_IMAGE}.part");
        self.upload_to(path, size, &script, &cleanup, progress, cancel)
            .await
    }

    /// Streams `path` into `script`'s stdin over SSH, reporting the share
    /// sent; on a cancel, runs `cleanup`.
    pub(crate) async fn upload_to(
        &self,
        path: &Path,
        size: u64,
        script: &str,
        cleanup: &str,
        progress: &ProgressSink,
        cancel: &CancellationToken,
    ) -> Result<(), DriverError> {
        match self
            .upload_once(path, size, script, cleanup, progress, cancel)
            .await
        {
            // The board is plugged in here but doesn't have this computer's
            // key: the console (no key needed) adds it, as "Add my SSH key"
            // does, and the copy runs again over the USB network.
            Err(Refused::Key(_)) if self.has_console() => {
                progress.log(
                    "the board doesn't have this computer's SSH key; adding it over the USB console"
                        .to_string(),
                );
                if let Err(why) = self.add_key_over_console().await {
                    return Err(DriverError::Other(format!(
                        "the board doesn't accept this computer's SSH key, and adding it over the USB console didn't work: {why}"
                    )));
                }
                self.upload_once(path, size, script, cleanup, progress, cancel)
                    .await
                    .map_err(|refused| match refused {
                        Refused::Key(_) => DriverError::Other(
                            "the board still doesn't accept this computer's SSH key after it was added over the USB console".into(),
                        ),
                        Refused::Other(error) => error,
                    })
            }
            Err(Refused::Key(error) | Refused::Other(error)) => Err(error),
            Ok(()) => Ok(()),
        }
    }

    /// Puts this computer's public key on the board over its USB console.
    async fn add_key_over_console(&self) -> Result<(), String> {
        let key = self
            .access
            .public_key()
            .ok_or("this computer has no SSH public key (choose one in Settings)")?;
        let script = crate::ssh_actions::add_key_script(&key).map_err(|error| error.to_string())?;
        match self.on_console(&script, None).await {
            Some(Ok(_)) => Ok(()),
            Some(Err(error)) => Err(error.to_string()),
            None => Err("the console didn't answer".into()),
        }
    }

    async fn upload_once(
        &self,
        path: &Path,
        size: u64,
        script: &str,
        cleanup: &str,
        progress: &ProgressSink,
        cancel: &CancellationToken,
    ) -> Result<(), Refused> {
        let mut file = tokio::fs::File::open(path)
            .await
            .map_err(|error| DriverError::Other(format!("{}: {error}", path.display())))?;
        let mut child = self
            .command()
            .arg(script)
            .stdin(Stdio::piped())
            .spawn()
            .map_err(|error| DriverError::Unreachable(format!("could not run ssh: {error}")))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| DriverError::Other("ssh has no stdin".into()))?;
        let mut buffer = vec![0u8; 256 * 1024];
        let mut sent = 0u64;
        loop {
            if cancel.is_cancelled() {
                let _ = child.kill().await;
                let _ = self.run(cleanup).await;
                return Err(DriverError::Cancelled.into());
            }
            let read = file
                .read(&mut buffer)
                .await
                .map_err(|error| DriverError::Other(format!("{}: {error}", path.display())))?;
            if read == 0 {
                break;
            }
            if let Err(error) = stdin.write_all(&buffer[..read]).await {
                // ssh exited; its stderr says why.
                drop(stdin);
                let output = child.wait_with_output().await.ok();
                let stderr = output
                    .as_ref()
                    .map(|output| String::from_utf8_lossy(&output.stderr).into_owned())
                    .unwrap_or_default();
                return Err(if stderr.trim().is_empty() {
                    DriverError::Unreachable(format!("the copy stopped: {error}")).into()
                } else {
                    Refused::from_ssh(output.and_then(|output| output.status.code()), &stderr)
                });
            }
            sent += read as u64;
            progress.step_progress(UpdateStep::Transfer, sent as f32 / size.max(1) as f32);
        }
        drop(stdin);
        let output = child
            .wait_with_output()
            .await
            .map_err(|error| DriverError::Unreachable(error.to_string()))?;
        if !output.status.success() {
            return Err(Refused::from_ssh(
                output.status.code(),
                &String::from_utf8_lossy(&output.stderr),
            ));
        }
        Ok(())
    }
}

/// Why a copy over SSH failed: the board refused this computer's key (which
/// the USB console can fix), or anything else.
enum Refused {
    Key(DriverError),
    Other(DriverError),
}

impl Refused {
    fn from_ssh(code: Option<i32>, stderr: &str) -> Self {
        let error = ssh_error(code, stderr);
        if key_refused(stderr) {
            Self::Key(error)
        } else {
            Self::Other(error)
        }
    }
}

impl From<DriverError> for Refused {
    fn from(error: DriverError) -> Self {
        Self::Other(error)
    }
}

/// ssh got no further than the board refusing every key offered.
fn key_refused(stderr: &str) -> bool {
    stderr.contains("Permission denied")
}

/// `script` with `BOARD_EVENT_SOURCE=atlas` exported first: everything it
/// runs (the writer, `event`, the self-test) logs Atlas as the source.
pub(crate) fn as_atlas(script: &str) -> String {
    format!("export BOARD_EVENT_SOURCE=atlas; {script}")
}

/// ssh refused the board's host key: not the one Atlas pinned.
fn host_key_changed(stderr: &str) -> bool {
    stderr.contains("REMOTE HOST IDENTIFICATION HAS CHANGED")
        || stderr.contains("Host key verification failed")
}

/// A readable reason for a failed ssh run.
fn ssh_error(code: Option<i32>, stderr: &str) -> DriverError {
    let stderr = stderr.trim();
    if key_refused(stderr) {
        return DriverError::Incompatible(
            "Atlas needs SSH access to the board for this, and the board doesn't have this computer's key. Plug it into this computer's USB once and choose \"Add my SSH key\" in its menu (over USB no key is needed); after that it works over the network too".into(),
        );
    }
    if host_key_changed(stderr) {
        return DriverError::Incompatible(
            "the board's SSH host key changed (a new OS on it, or a reflash): if you put it there, choose \"Trust its new SSH key\" in the board's More menu, then try again".into(),
        );
    }
    let last = stderr.lines().last().unwrap_or("").to_string();
    match code {
        // 255 is ssh's own failure: the board isn't reachable.
        Some(255) | None => DriverError::Unreachable(if last.is_empty() {
            "SSH to the board failed".into()
        } else {
            last
        }),
        Some(code) => DriverError::Other(if last.is_empty() {
            format!("the board's command failed (exit {code})")
        } else {
            last
        }),
    }
}

#[async_trait]
impl UpdateCapability for SshUpdate {
    fn plan(&self, _device: &Identity, release: &ReleaseRef) -> Result<UpdatePlan, DriverError> {
        let artifact = release
            .artifact
            .as_ref()
            .ok_or_else(|| DriverError::Incompatible("choose an image to install".into()))?;
        let name = artifact.name.to_ascii_lowercase();
        if !IMAGE_SUFFIXES.iter().any(|suffix| name.ends_with(suffix)) {
            return Err(DriverError::Incompatible(format!(
                "{} isn't a disk image (.img, .img.xz, .img.zst or .img.gz)",
                artifact.name
            )));
        }
        Ok(UpdatePlan {
            steps: vec![
                UpdateStep::Preflight,
                UpdateStep::Transfer,
                UpdateStep::Apply,
                UpdateStep::Reboot,
                UpdateStep::Confirm,
            ],
            concurrency: Concurrency::Parallel,
            summary: format!(
                "Update to {} over SSH from {}: the board copies the new version into its spare slot, restarts into it, and keeps it once it's healthy; settings and data stay",
                release.version, artifact.name
            ),
        })
    }

    async fn run(
        &self,
        _device: &Identity,
        release: &ReleaseRef,
        progress: &ProgressSink,
        cancel: &CancellationToken,
    ) -> Result<UpdateOutcome, DriverError> {
        let artifact = release
            .artifact
            .as_ref()
            .ok_or_else(|| DriverError::Incompatible("no image chosen".into()))?;
        // It goes into the board's command line: hex only.
        if artifact.sha256.len() != 64 || !artifact.sha256.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(DriverError::Other(format!(
                "{} has no usable SHA-256, which the board needs to check its copy",
                artifact.name
            )));
        }

        progress.step_started(UpdateStep::Preflight);
        let status = self.status().await?;
        // `rebooting` from an earlier boot reads as `trying`; a stage left
        // over from a power loss reads as `error`.
        if matches!(status.state.as_str(), "trying" | "rebooting" | "staging") {
            return Err(DriverError::Incompatible(format!(
                "the board is still busy with its last update ({}); wait for it to finish",
                status.state
            )));
        }
        let free_kib: u64 = self
            .run(&format!(
                "df -Pk {} | awk 'NR == 2 {{ print $4 }}'",
                "/data"
            ))
            .await?
            .trim()
            .parse()
            .unwrap_or(u64::MAX);
        if free_kib.saturating_mul(1024) < artifact.size_bytes + 16 * 1024 * 1024 {
            return Err(DriverError::Incompatible(format!(
                "/data has {} MiB free; the image needs {} MiB",
                free_kib / 1024,
                artifact.size_bytes / (1024 * 1024) + 16
            )));
        }
        let mut boot_before = self.boot_id().await?;
        progress.log(format!("running {}", status.version_active));

        progress.step_started(UpdateStep::Transfer);
        self.upload(&artifact.path, artifact.size_bytes, progress, cancel)
            .await?;
        if cancel.is_cancelled() {
            let _ = self.run(&format!("rm -f {REMOTE_IMAGE}")).await;
            return Err(DriverError::Cancelled);
        }

        // Staging only writes the spare slot, after the writer checks the
        // copy's SHA-256; it reports 0..1000.
        progress.step_started(UpdateStep::Apply);
        let script = format!(
            "{WRITER} stage {REMOTE_IMAGE} --sha256 {}; rc=$?; rm -f {REMOTE_IMAGE}; exit $rc",
            artifact.sha256.to_ascii_lowercase()
        );
        let stage = self.run(&script);
        tokio::pin!(stage);
        let deadline = tokio::time::Instant::now() + STAGE_TIMEOUT;
        let staged = loop {
            tokio::select! {
                result = &mut stage => break result,
                () = tokio::time::sleep(POLL) => {
                    if let Ok(status) = self.status().await {
                        progress.step_progress(UpdateStep::Apply, status.progress as f32 / 1000.0);
                    }
                    if tokio::time::Instant::now() >= deadline {
                        return Err(DriverError::Other("staging the update took too long".into()));
                    }
                }
            }
        };
        if let Err(error) = staged {
            let reason = self
                .status()
                .await
                .ok()
                .map(|status| status.error)
                .filter(|error| !error.is_empty());
            return Err(match reason {
                Some(reason) => DriverError::Other(reason),
                None => error,
            });
        }
        let staged = self.status().await?;
        progress.step_progress(UpdateStep::Apply, 1.0);
        progress.log(format!(
            "staged {}; the board is restarting into it",
            staged.version_staged
        ));

        // Past this point the board is restarting: no cancelling. The
        // connection may drop with the reboot; only a refusal by the writer
        // itself counts as a failure.
        progress.step_started(UpdateStep::Reboot);
        self.expect_new_host_key();
        if let Err(error @ DriverError::Other(_)) = self.run(&format!("{WRITER} apply")).await {
            return Err(error);
        }
        // Once per restart into the trial: the board restarts again by itself
        // when the firmware ignored the first (see trial_stopped).
        'restart: loop {
            let deadline = tokio::time::Instant::now() + REBOOT_TIMEOUT;
            let boot_after = loop {
                tokio::time::sleep(POLL).await;
                if let Ok(boot) = self.boot_id().await
                    && !boot.is_empty()
                    && boot != boot_before
                {
                    break boot;
                }
                if tokio::time::Instant::now() >= deadline {
                    return Ok(UpdateOutcome::NeedsRecovery {
                        reason: "the board didn't come back after restarting into the update"
                            .into(),
                    });
                }
            };

            progress.step_started(UpdateStep::Confirm);
            let deadline = tokio::time::Instant::now() + CONFIRM_TIMEOUT;
            loop {
                // A failed health check restarts the board; keep polling through it.
                if let Ok(status) = self.status().await {
                    match trial_stopped(&status.state, &status.error) {
                        Some(TrialStop::Retrying) => {
                            progress.log(status.error.clone());
                            boot_before = boot_after;
                            continue 'restart;
                        }
                        Some(TrialStop::Stopped) => {
                            return Ok(UpdateOutcome::RolledBack {
                                reason: status.error,
                            });
                        }
                        None => {}
                    }
                    match status.state.as_str() {
                        "confirmed" => {
                            return Ok(if status.version_active == staged.version_staged {
                                UpdateOutcome::Verified {
                                    version: status.version_active,
                                }
                            } else {
                                UpdateOutcome::RolledBack {
                                    reason: format!(
                                        "the board runs {} instead of {}",
                                        status.version_active, staged.version_staged
                                    ),
                                }
                            });
                        }
                        "rolled-back" => {
                            return Ok(UpdateOutcome::RolledBack {
                                reason: if status.error.is_empty() {
                                    "the new version didn't confirm itself".into()
                                } else {
                                    status.error
                                },
                            });
                        }
                        _ => {}
                    }
                }
                if tokio::time::Instant::now() >= deadline {
                    return Ok(UpdateOutcome::RolledBack {
                    reason: "the board never confirmed the new version; it returns to the old one on its next restart".into(),
                });
                }
                tokio::time::sleep(POLL).await;
            }
        }
    }
}

#[cfg(test)]
mod tests;
