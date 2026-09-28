//! Recovery: USB boot, find the new disk, write, verify.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use atlas_blockdev::{Disk, HelperClient, HelperRequest, WriteProgress, list_disks};
use atlas_driver::{
    Artifact, CancellationToken, Concurrency, DriverError, Identity, ProgressSink, ReleaseRef,
    UpdateCapability, UpdateOutcome, UpdatePlan, UpdateStep,
};
use atlas_usbboot::{BootEvent, BootOptions, boot_device};

use crate::RpiConfig;
use crate::boot_files::find_boot_files;

/// How long the eMMC may take to appear as a disk after USB boot.
const DISK_APPEAR_TIMEOUT: Duration = Duration::from_secs(90);
const DISK_POLL: Duration = Duration::from_secs(1);

pub(crate) struct RpiRecovery {
    config: Arc<RpiConfig>,
}

impl RpiRecovery {
    pub(crate) fn new(config: Arc<RpiConfig>) -> Self {
        Self { config }
    }
}

fn failed(step: UpdateStep, message: impl Into<String>) -> DriverError {
    DriverError::StepFailed {
        step: step.label().into(),
        message: message.into(),
    }
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

fn artifact(release: &ReleaseRef) -> Result<&Artifact, DriverError> {
    release.artifact.as_ref().ok_or_else(|| {
        DriverError::Incompatible(
            "choose an image file to write; add one on the Releases page".into(),
        )
    })
}

async fn disks() -> Result<Vec<Disk>, DriverError> {
    tokio::task::spawn_blocking(list_disks)
        .await
        .map_err(|error| DriverError::Other(error.to_string()))?
        .map_err(|error| DriverError::Other(error.to_string()))
}

/// Waits for exactly one new USB disk that was not there before booting.
async fn wait_for_new_disk(
    before: &BTreeSet<PathBuf>,
    cancel: &CancellationToken,
    progress: &ProgressSink,
) -> Result<Disk, DriverError> {
    let deadline = tokio::time::Instant::now() + DISK_APPEAR_TIMEOUT;
    loop {
        if cancel.is_cancelled() {
            return Err(DriverError::Cancelled);
        }
        let fresh: Vec<Disk> = disks()
            .await?
            .into_iter()
            .filter(|disk| disk.usb && !disk.system && !before.contains(&disk.path))
            .filter(|disk| disk.size_bytes > 0)
            .collect();
        match fresh.as_slice() {
            [disk] => {
                progress.log(format!(
                    "eMMC appeared as {} ({}, {})",
                    disk.name,
                    disk.description(),
                    human_size(disk.size_bytes)
                ));
                return Ok(disk.clone());
            }
            [] => {}
            several => {
                return Err(failed(
                    UpdateStep::Apply,
                    format!(
                        "{} new USB disks appeared ({}); unplug other drives and try again",
                        several.len(),
                        several
                            .iter()
                            .map(|disk| disk.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                ));
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(failed(
                UpdateStep::Apply,
                "the board booted but its eMMC never appeared as a disk; check the USB cable \
                 and that the board is a compute module with eMMC",
            ));
        }
        tokio::time::sleep(DISK_POLL).await;
    }
}

#[async_trait]
impl UpdateCapability for RpiRecovery {
    fn plan(&self, _device: &Identity, release: &ReleaseRef) -> Result<UpdatePlan, DriverError> {
        let artifact = artifact(release)?;
        Ok(UpdatePlan {
            steps: vec![
                UpdateStep::Preflight,
                UpdateStep::Transfer,
                UpdateStep::Apply,
                UpdateStep::Confirm,
            ],
            // One USB boot at a time keeps "which new disk is it" unambiguous.
            concurrency: Concurrency::Exclusive("usb-boot".into()),
            summary: format!(
                "USB boot, write {} ({}) to the eMMC, then verify",
                artifact.name,
                human_size(artifact.size_bytes)
            ),
        })
    }

    async fn run(
        &self,
        device: &Identity,
        release: &ReleaseRef,
        progress: &ProgressSink,
        cancel: &CancellationToken,
    ) -> Result<UpdateOutcome, DriverError> {
        let artifact = artifact(release)?.clone();

        progress.step_started(UpdateStep::Preflight);
        if !artifact.path.is_file() {
            return Err(failed(
                UpdateStep::Preflight,
                format!("the image {} is missing", artifact.path.display()),
            ));
        }
        let files = find_boot_files(&self.config.boot_file_dirs).map_err(|tried| {
            failed(
                UpdateStep::Preflight,
                format!(
                    "USB boot files not found (looked in {}); see Settings > Host health",
                    tried
                        .iter()
                        .map(|dir| dir.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            )
        })?;
        let helper = HelperClient::locate()
            .map_err(|error| failed(UpdateStep::Preflight, error.to_string()))?;
        let before: BTreeSet<PathBuf> = disks().await?.into_iter().map(|disk| disk.path).collect();
        progress.log(format!("boot files: {}", files.dir().display()));

        progress.step_started(UpdateStep::Transfer);
        let events = progress.clone();
        let token = cancel.clone();
        let options = BootOptions {
            location: Some(device.address.clone()),
            ..BootOptions::default()
        };
        boot_device(
            &files,
            &options,
            &move |event| match event {
                BootEvent::SecondStage { chip, bytes } => {
                    events.log(format!(
                        "sent second stage for {} ({bytes} bytes)",
                        chip.label()
                    ));
                    events.step_progress(UpdateStep::Transfer, 0.3);
                }
                BootEvent::FileSent { name, bytes } => {
                    events.log(format!("sent {name} ({bytes} bytes)"));
                    if name == "boot.img" {
                        events.step_progress(UpdateStep::Transfer, 0.9);
                    }
                }
                BootEvent::FileServerDone => events.step_progress(UpdateStep::Transfer, 1.0),
                _ => {}
            },
            &move || token.is_cancelled(),
        )
        .await
        .map_err(|error| match error {
            atlas_usbboot::UsbBootError::Cancelled => DriverError::Cancelled,
            other => {
                let fix = other.fix().map(|fix| format!(" {fix}")).unwrap_or_default();
                failed(UpdateStep::Transfer, format!("{other}.{fix}"))
            }
        })?;

        progress.step_started(UpdateStep::Apply);
        progress.log("waiting for the eMMC to appear as a USB disk");
        let disk = wait_for_new_disk(&before, cancel, progress).await?;

        // From here the write runs in the elevated helper. Cancelling asks
        // it to stop, which leaves the eMMC partly written: the device
        // stays in recovery and can simply be recovered again.
        let stop = Arc::new(AtomicBool::new(false));
        let watcher = {
            let stop = stop.clone();
            let cancel = cancel.clone();
            tokio::spawn(async move {
                cancel.cancelled().await;
                stop.store(true, Ordering::Relaxed);
            })
        };
        let request = HelperRequest {
            device: disk.path.clone(),
            image: artifact.path.clone(),
            image_sha256: Some(artifact.sha256.clone()),
            expected_size: disk.size_bytes,
            verify: true,
        };
        let sink = progress.clone();
        let written = tokio::task::spawn_blocking(move || {
            let mut confirming = false;
            helper.write(
                &request,
                &mut |update| match update {
                    WriteProgress::Writing {
                        input_done,
                        input_total,
                        ..
                    } if input_total > 0 => {
                        sink.step_progress(
                            UpdateStep::Apply,
                            input_done as f32 / input_total as f32,
                        );
                    }
                    WriteProgress::Verifying { done, total } if total > 0 => {
                        if !confirming {
                            confirming = true;
                            sink.step_started(UpdateStep::Confirm);
                        }
                        sink.step_progress(UpdateStep::Confirm, done as f32 / total as f32);
                    }
                    _ => {}
                },
                &stop,
            )
        })
        .await
        .map_err(|error| DriverError::Other(error.to_string()))?;
        watcher.abort();

        let report = written.map_err(|error| match error {
            atlas_blockdev::BlockError::Cancelled => DriverError::Cancelled,
            atlas_blockdev::BlockError::Elevation { message, fix } => {
                failed(UpdateStep::Apply, format!("{message} {fix}"))
            }
            other => failed(UpdateStep::Apply, other.to_string()),
        })?;
        progress.step_started(UpdateStep::Confirm);
        progress.step_progress(UpdateStep::Confirm, 1.0);
        progress.log(format!(
            "wrote and verified {} in {} s (sha256 {})",
            human_size(report.bytes_written),
            report.seconds,
            &report.sha256[..12.min(report.sha256.len())]
        ));
        progress.log("remove the nRPIBOOT jumper and power-cycle the board to boot the new image");
        Ok(UpdateOutcome::Verified {
            version: release.version.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use atlas_driver::{DeviceKey, DeviceMode, Family, LinkId};

    use super::*;

    fn identity() -> Identity {
        Identity {
            key: DeviceKey::new("rpi", "port-1-2"),
            model: "BCM2711".into(),
            mode: DeviceMode::Recovery,
            versions: BTreeMap::new(),
            name: None,
            link: LinkId("usb-boot".into()),
            address: "1-2".into(),
        }
    }

    fn release(artifact: Option<Artifact>) -> ReleaseRef {
        ReleaseRef {
            family: Family::new("rpi"),
            version: "2026.3.1".into(),
            artifact,
        }
    }

    #[test]
    fn plan_needs_an_image_and_runs_one_usb_boot_at_a_time() {
        let recovery = RpiRecovery::new(Arc::new(RpiConfig::default()));
        assert!(matches!(
            recovery.plan(&identity(), &release(None)),
            Err(DriverError::Incompatible(_))
        ));

        let plan = recovery
            .plan(
                &identity(),
                &release(Some(Artifact {
                    name: "helios.img.xz".into(),
                    path: PathBuf::from("/tmp/helios.img.xz"),
                    sha256: "00".repeat(32),
                    size_bytes: 1_500_000_000,
                })),
            )
            .unwrap();
        assert_eq!(plan.concurrency, Concurrency::Exclusive("usb-boot".into()));
        assert!(plan.summary.contains("1.5 GB"));
        assert_eq!(plan.steps.len(), 4);
    }

    #[test]
    fn sizes_read_naturally() {
        assert_eq!(human_size(512), "512.0 B");
        assert_eq!(human_size(31_914_983_424), "31.9 GB");
    }
}
