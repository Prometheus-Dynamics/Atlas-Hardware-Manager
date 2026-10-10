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

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use atlas_driver::{
    CancellationToken, Concurrency, DeviceKey, DriverError, Identity, ProgressSink, ReleaseRef,
    UpdateCapability, UpdateOutcome, UpdatePlan, UpdateStep,
};
use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

/// The update method a board lists when it has the A/B layout and writer.
pub const AB_METHOD: &str = "ab-tryboot";

const WRITER: &str = "/usr/lib/board/update";
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

/// How Atlas reaches boards over SSH. Cheap to clone; shared with the app so
/// a settings change applies to the next update.
#[derive(Clone, Debug, Default)]
pub struct SshAccess(Arc<RwLock<SshConfig>>);

#[derive(Clone, Debug, Default)]
pub struct SshConfig {
    /// The private key; `None` lets ssh use its defaults and agent.
    pub identity_file: Option<PathBuf>,
    /// Atlas's own known_hosts; `None` uses the user's.
    pub known_hosts: Option<PathBuf>,
}

impl SshAccess {
    pub fn new(config: SshConfig) -> Self {
        Self(Arc::new(RwLock::new(config)))
    }

    pub fn set(&self, config: SshConfig) {
        *self
            .0
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = config;
    }

    pub fn get(&self) -> SshConfig {
        self.0
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

/// The private key next to a public key path (`id_ed25519.pub` →
/// `id_ed25519`), when it exists.
pub fn private_key_for(public: &Path) -> Option<PathBuf> {
    let name = public.file_name()?.to_str()?;
    let private = public.with_file_name(name.strip_suffix(".pub")?);
    private.is_file().then_some(private)
}

/// What `update status` prints.
#[derive(Clone, Debug, Default, Deserialize)]
struct Status {
    #[serde(default)]
    state: String,
    #[serde(default)]
    version_active: String,
    #[serde(default)]
    version_staged: String,
    #[serde(default)]
    progress: u32,
    #[serde(default)]
    error: String,
}

pub(crate) struct SshUpdate {
    pub(crate) access: SshAccess,
    pub(crate) host: String,
    pub(crate) key: DeviceKey,
}

impl SshUpdate {
    fn command(&self) -> Command {
        let config = self.access.get();
        let mut command = Command::new("ssh");
        command
            .args(["-o", "BatchMode=yes"])
            .args(["-o", "ConnectTimeout=8"])
            .args(["-o", "ServerAliveInterval=5", "-o", "ServerAliveCountMax=3"])
            .args(["-o", "StrictHostKeyChecking=accept-new"])
            .arg("-o")
            .arg(format!("HostKeyAlias=board-{}", self.key).replace(':', "-"));
        if let Some(known_hosts) = &config.known_hosts {
            if let Some(dir) = known_hosts.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
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

    /// Runs `script` on the board and returns its stdout. The board's event
    /// log records what it does as Atlas's.
    pub(crate) async fn run(&self, script: &str) -> Result<String, DriverError> {
        let output = self
            .command()
            .arg(as_atlas(script))
            .output()
            .await
            .map_err(|error| DriverError::Unreachable(format!("could not run ssh: {error}")))?;
        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
        }
        Err(ssh_error(
            output.status.code(),
            &String::from_utf8_lossy(&output.stderr),
        ))
    }

    /// Like [`run`](Self::run), handing each line the command writes to
    /// stderr to `on_line` as it comes (stdout is returned at the end).
    pub(crate) async fn run_watching(
        &self,
        script: &str,
        on_line: &(dyn Fn(&str) + Send + Sync),
    ) -> Result<String, DriverError> {
        use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};

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
        let mut file = tokio::fs::File::open(path)
            .await
            .map_err(|error| DriverError::Other(format!("{}: {error}", path.display())))?;
        let mut child = self
            .command()
            .arg(format!(
                "mkdir -p {REMOTE_DIR} && cat > {REMOTE_IMAGE}.part && mv {REMOTE_IMAGE}.part {REMOTE_IMAGE}"
            ))
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
                let _ = self.run(&format!("rm -f {REMOTE_IMAGE}.part")).await;
                return Err(DriverError::Cancelled);
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
                    DriverError::Unreachable(format!("the copy stopped: {error}"))
                } else {
                    ssh_error(output.and_then(|output| output.status.code()), &stderr)
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
            return Err(ssh_error(
                output.status.code(),
                &String::from_utf8_lossy(&output.stderr),
            ));
        }
        Ok(())
    }
}

/// `script` with `BOARD_EVENT_SOURCE=atlas` exported first: everything it
/// runs (the writer, `event`, the self-test) logs Atlas as the source.
pub(crate) fn as_atlas(script: &str) -> String {
    format!("export BOARD_EVENT_SOURCE=atlas; {script}")
}

/// A readable reason for a failed ssh run.
fn ssh_error(code: Option<i32>, stderr: &str) -> DriverError {
    let stderr = stderr.trim();
    if stderr.contains("Permission denied") {
        return DriverError::Incompatible(
            "the board doesn't accept your SSH key: reflash it with \"Add my SSH key\" ticked on the Flash tab".into(),
        );
    }
    if stderr.contains("REMOTE HOST IDENTIFICATION HAS CHANGED") {
        return DriverError::Incompatible(
            "the board's SSH host key changed (was it reflashed?): remove its board-… entry from Atlas's known_hosts and try again".into(),
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
        let boot_before = self.boot_id().await?;
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
        if let Err(error @ DriverError::Other(_)) = self.run(&format!("{WRITER} apply")).await {
            return Err(error);
        }
        let deadline = tokio::time::Instant::now() + REBOOT_TIMEOUT;
        loop {
            tokio::time::sleep(POLL).await;
            if let Ok(boot) = self.boot_id().await
                && !boot.is_empty()
                && boot != boot_before
            {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                return Ok(UpdateOutcome::NeedsRecovery {
                    reason: "the board didn't come back after restarting into the update".into(),
                });
            }
        }

        progress.step_started(UpdateStep::Confirm);
        let deadline = tokio::time::Instant::now() + CONFIRM_TIMEOUT;
        loop {
            // A failed health check restarts the board; keep polling through it.
            if let Ok(status) = self.status().await {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_parses_the_writer_output() {
        let status: Status = serde_json::from_str(
            r#"{"state":"staged","slot_active":"A","slot_staged":"B","version_active":"1.0","version_staged":"2.0","progress":1000,"error":""}"#,
        )
        .unwrap();
        assert_eq!(status.state, "staged");
        assert_eq!(status.version_staged, "2.0");
        assert_eq!(status.progress, 1000);
    }

    #[test]
    fn commands_run_as_atlas() {
        assert_eq!(
            as_atlas("/usr/lib/board/update status"),
            "export BOARD_EVENT_SOURCE=atlas; /usr/lib/board/update status"
        );
    }

    #[test]
    fn ssh_failures_read_as_reasons() {
        assert!(matches!(
            ssh_error(Some(255), "root@x: Permission denied (publickey)."),
            DriverError::Incompatible(_)
        ));
        assert!(matches!(
            ssh_error(
                Some(255),
                "ssh: connect to host x port 22: No route to host"
            ),
            DriverError::Unreachable(_)
        ));
        assert!(matches!(
            ssh_error(Some(1), "update: nothing is staged"),
            DriverError::Other(reason) if reason == "update: nothing is staged"
        ));
    }

    #[test]
    fn the_private_key_sits_next_to_the_public_one() {
        let dir = std::env::temp_dir().join(format!("atlas-board-key-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("id_test"), "private").unwrap();
        std::fs::write(dir.join("id_test.pub"), "ssh-ed25519 AAAA").unwrap();
        assert_eq!(
            private_key_for(&dir.join("id_test.pub")),
            Some(dir.join("id_test"))
        );
        assert_eq!(private_key_for(&dir.join("missing.pub")), None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
