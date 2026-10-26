//! Installing an opkg package (`.ipk`) on a SystemCore through its own
//! package manager (devices-as-data's `package` method), over SSH as
//! `systemcore` with this computer's key.
//!
//! Only a key: `BatchMode=yes` means ssh never asks for or uses a password,
//! and `sudo -n` never prompts (the unit's `systemcore` user has
//! passwordless sudo). A unit that doesn't take the key gets the one-line
//! `ssh-copy-id` step to run once.
//!
//! 1. Preflight: a login by key works.
//! 2. Transfer: the file streams into `/tmp/atlas-install.ipk` (whole, or
//!    not at all: it lands under another name first).
//! 3. Apply: `sudo -n opkg install --force-reinstall` (an upgrade keeps the
//!    package's conffiles), and the file goes.
//! 4. Confirm: every systemd service the package installs is active.
//!
//! Nothing on the unit changes before Apply ([`ChangeTracker`]).

use std::net::Ipv4Addr;
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use atlas_driver::{
    ArtifactKind, CancellationToken, ChangeTracker, DriverError, Identity, InstallFailure,
    InstallMethod, InstallMethodId, InstallPlan, InstallRequest, ProgressSink, UpdateOutcome,
    UpdateStep,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const USER: &str = "systemcore";
const REMOTE: &str = "/tmp/atlas-install.ipk";
/// How long the package's services have to come up.
const SETTLE: Duration = Duration::from_secs(30);

/// The package's name from an opkg file name (`helios_1.2.3_aarch64.ipk`
/// is `helios`), when it is a plain one.
pub fn package_name(file_name: &str) -> Option<String> {
    let name = file_name.strip_suffix(".ipk")?.split('_').next()?;
    let plain = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '+' | '-'));
    plain.then(|| name.to_string())
}

pub struct PackageMethod {
    pub ip: Ipv4Addr,
}

impl PackageMethod {
    fn ssh(&self) -> tokio::process::Command {
        let mut command = tokio::process::Command::new("ssh");
        command
            .args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=5"])
            .args(["-o", "StrictHostKeyChecking=accept-new"])
            .arg(format!("{USER}@{}", self.ip))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }

    fn key_missing(&self) -> DriverError {
        DriverError::Incompatible(format!(
            "this computer's SSH key isn't on the SystemCore yet. Add it once with: ssh-copy-id {USER}@{}",
            self.ip
        ))
    }

    /// Runs `script` as `systemcore`; its stdout, or why it failed.
    pub(crate) async fn run(&self, script: &str) -> Result<String, DriverError> {
        let output = self
            .ssh()
            .arg(script)
            .output()
            .await
            .map_err(|error| DriverError::Unreachable(format!("could not run ssh: {error}")))?;
        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("Permission denied") {
            return Err(self.key_missing());
        }
        let last = stderr
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .trim();
        Err(match output.status.code() {
            Some(255) | None => DriverError::Unreachable(if last.is_empty() {
                format!("SSH to the SystemCore at {} failed", self.ip)
            } else {
                last.to_string()
            }),
            Some(code) => DriverError::Other(if last.is_empty() {
                format!("the command failed (exit {code})")
            } else {
                last.to_string()
            }),
        })
    }

    /// Streams the file into REMOTE.
    async fn send(
        &self,
        request: &InstallRequest,
        progress: &ProgressSink,
    ) -> Result<(), DriverError> {
        let path = &request.artifact.path;
        let mut file = tokio::fs::File::open(path)
            .await
            .map_err(|error| DriverError::Other(format!("{}: {error}", path.display())))?;
        let size = tokio::fs::metadata(path)
            .await
            .map(|m| m.len())
            .unwrap_or(1)
            .max(1);
        let mut child = self
            .ssh()
            .arg(format!(
                "cat > {REMOTE}.part && mv -f {REMOTE}.part {REMOTE}"
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
            let read = file
                .read(&mut buffer)
                .await
                .map_err(|error| DriverError::Other(format!("{}: {error}", path.display())))?;
            if read == 0 {
                break;
            }
            if stdin.write_all(&buffer[..read]).await.is_err() {
                break;
            }
            sent += read as u64;
            progress.step_progress(UpdateStep::Transfer, sent as f32 / size as f32);
        }
        drop(stdin);
        let output = child
            .wait_with_output()
            .await
            .map_err(|error| DriverError::Unreachable(error.to_string()))?;
        if output.status.success() && sent == size {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("Permission denied") {
            return Err(self.key_missing());
        }
        Err(DriverError::Unreachable(format!(
            "the copy to the SystemCore stopped after {} of {} bytes: {}",
            sent,
            size,
            stderr.trim()
        )))
    }
}

#[async_trait]
impl InstallMethod for PackageMethod {
    fn id(&self) -> InstallMethodId {
        InstallMethodId::Package
    }

    fn kinds(&self) -> &'static [ArtifactKind] {
        &[ArtifactKind::Ipk]
    }

    fn plan(
        &self,
        _device: &Identity,
        request: &InstallRequest,
    ) -> Result<InstallPlan, DriverError> {
        package_name(&request.artifact.name).ok_or_else(|| {
            DriverError::Incompatible(format!(
                "{} isn't named like an opkg package (name_version_arch.ipk)",
                request.artifact.name
            ))
        })?;
        Ok(InstallPlan {
            method: InstallMethodId::Package,
            steps: vec![
                UpdateStep::Preflight,
                UpdateStep::Transfer,
                UpdateStep::Apply,
                UpdateStep::Confirm,
            ],
            erases: false,
            restarts: false,
            summary: format!(
                "Copies {} to the SystemCore, installs it with its package manager (opkg; its settings files stay) and checks its services run",
                request.artifact.name
            ),
        })
    }

    async fn run(
        &self,
        _device: &Identity,
        request: &InstallRequest,
        progress: &ProgressSink,
        cancel: &CancellationToken,
    ) -> Result<UpdateOutcome, InstallFailure> {
        let (tracker, sink) = ChangeTracker::watch(progress);
        let package = package_name(&request.artifact.name).ok_or_else(|| {
            tracker.failure(DriverError::Incompatible(format!(
                "{} isn't an opkg package name",
                request.artifact.name
            )))
        })?;
        sink.step_started(UpdateStep::Preflight);
        self.run("true").await.map_err(|e| tracker.failure(e))?;
        if cancel.is_cancelled() {
            return Err(tracker.failure(DriverError::Cancelled));
        }
        sink.step_started(UpdateStep::Transfer);
        self.send(request, &sink)
            .await
            .map_err(|e| tracker.failure(e))?;
        sink.step_progress(UpdateStep::Transfer, 1.0);

        sink.step_started(UpdateStep::Apply);
        let installed = self
            .run(&format!(
                "sudo -n opkg install --force-reinstall {REMOTE}; rc=$?; rm -f {REMOTE}; exit $rc"
            ))
            .await;
        installed.map_err(|error| {
            tracker.failure(DriverError::StepFailed {
                step: "Installing the package".into(),
                message: error.to_string(),
            })
        })?;

        sink.step_started(UpdateStep::Confirm);
        let units: Vec<String> = self
            .run(&format!(
                "opkg files {package} | sed -n 's#.*/systemd/system/\\([^/]*\\.service\\)$#\\1#p'"
            ))
            .await
            .map_err(|e| tracker.failure(e))?
            .lines()
            .map(str::trim)
            .filter(|unit| {
                !unit.is_empty()
                    && unit
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "@._-".contains(c))
            })
            .map(str::to_string)
            .collect();
        let deadline = tokio::time::Instant::now() + SETTLE;
        loop {
            let mut down = Vec::new();
            for unit in &units {
                if self
                    .run(&format!("systemctl is-active --quiet {unit}"))
                    .await
                    .is_err()
                {
                    down.push(unit.clone());
                }
            }
            if down.is_empty() {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(tracker.failure(DriverError::StepFailed {
                    step: "Checking its services".into(),
                    message: format!(
                        "installed, but not running after {} s: {}",
                        SETTLE.as_secs(),
                        down.join(", ")
                    ),
                }));
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        crate::systemcore_versions::forget(self.ip);
        sink.log(if units.is_empty() {
            format!("{package} installed (it has no services)")
        } else {
            format!("{package} installed; running: {}", units.join(", "))
        });
        Ok(UpdateOutcome::Verified {
            version: request.version.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_names_come_from_opkg_file_names() {
        assert_eq!(
            package_name("helios_2026.3.0_aarch64.ipk").as_deref(),
            Some("helios")
        );
        assert_eq!(
            package_name("elastic-dashboard_1.0_all.ipk").as_deref(),
            Some("elastic-dashboard")
        );
        assert_eq!(
            package_name("x;rm -rf_1_all.ipk"),
            None,
            "never onto a command line"
        );
        assert_eq!(package_name("helios.img.xz"), None);
    }
}
