//! The raze-flasher path (docs/flasher.md): USB boot the board into the
//! flasher, then stream the image to it as it is stored. The board writes,
//! verifies and reboots itself and reports how far it is, so there's no
//! host block device, no elevation and no eject.

use std::path::Path;
use std::time::Duration;

use atlas_driver::{
    Artifact, CancellationToken, DriverError, Identity, ProgressSink, ReleaseRef, UpdateOutcome,
    UpdateStep,
};
use atlas_flasher::{FlasherError, ImageFormat, Progress};
use atlas_usbboot::{BootEvent, BootFiles, BootOptions, boot_device};

use crate::RpiConfig;
use crate::recover::{RESTARTED, forget_host_keys};

/// How long the flasher may take to come up after USB boot (~3 s on a CM5).
const FLASHER_APPEAR_TIMEOUT: Duration = Duration::from_secs(30);

/// The model that has a flasher.
const FLASHER_MODEL: &str = "raze";

/// How the flasher would take this image: `.zst` and `.xz` as they are
/// (the board decompresses), raw `.img` as is. Others (gzip, zip) go the
/// old way.
pub(crate) fn image_format(path: &Path) -> Option<ImageFormat> {
    let name = path.file_name()?.to_string_lossy().to_ascii_lowercase();
    if name.ends_with(".zst") {
        Some(ImageFormat::Zst)
    } else if name.ends_with(".xz") {
        Some(ImageFormat::Xz)
    } else if name.ends_with(".img") || name.ends_with(".bin") {
        Some(ImageFormat::Raw)
    } else {
        None
    }
}

/// Whether this board in USB boot has a flasher.
pub(crate) fn has_flasher(device: &Identity) -> bool {
    device
        .attributes
        .get("model")
        .is_some_and(|model| model == FLASHER_MODEL)
}

fn failed(step: UpdateStep, message: impl Into<String>) -> DriverError {
    DriverError::StepFailed {
        step: step.label().into(),
        message: message.into(),
    }
}

fn tail8(serial: &str) -> String {
    let serial = serial.trim().to_ascii_lowercase();
    serial[serial.len().saturating_sub(8)..].to_string()
}

/// What to do when the flasher booted but then failed: the board is no
/// longer in USB boot, so it has to be put back.
const AGAIN: &str = "Power-cycle the board with its boot button held to try again \
    (set ATLAS_FLASHER=off to use the mass-storage path).";

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run(
    config: &RpiConfig,
    device: &Identity,
    release: &ReleaseRef,
    artifact: &Artifact,
    files: BootFiles,
    format: ImageFormat,
    progress: &ProgressSink,
    cancel: &CancellationToken,
) -> Result<UpdateOutcome, DriverError> {
    progress.log(format!("raze-flasher: {}", files.dir().display()));

    progress.step_started(UpdateStep::Transfer);
    let events = progress.clone();
    let token = cancel.clone();
    let options = BootOptions {
        location: Some(device.address.clone()),
        ..BootOptions::default()
    };
    let booted = boot_device(
        &files,
        &options,
        &move |event| match event {
            BootEvent::SecondStage { chip, bytes } => {
                events.log(format!(
                    "sent second stage for {} ({bytes} bytes)",
                    chip.label()
                ));
                events.step_progress(UpdateStep::Transfer, 0.2);
            }
            BootEvent::FileSent { name, bytes } => {
                events.log(format!("sent {name} ({bytes} bytes)"));
                if name == "boot.img" {
                    events.step_progress(UpdateStep::Transfer, 0.5);
                }
            }
            BootEvent::FileServerDone => events.step_progress(UpdateStep::Transfer, 0.6),
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
    let serial = booted.serial.as_deref().map(tail8);
    if let Some(serial) = &serial {
        progress.log(format!("board serial {serial}"));
    }

    progress.log("waiting for the flasher to start");
    let info = tokio::select! {
        found = atlas_flasher::wait_for(serial.as_deref(), FLASHER_APPEAR_TIMEOUT) => found,
        () = cancel.cancelled() => return Err(DriverError::Cancelled),
    }
    .map_err(|error| failed(UpdateStep::Transfer, error.to_string()))?
    .ok_or_else(|| {
        failed(
            UpdateStep::Transfer,
            format!(
                "the flasher didn't show up on USB within {} s. {AGAIN}",
                FLASHER_APPEAR_TIMEOUT.as_secs()
            ),
        )
    })?;
    let mut flasher = atlas_flasher::open(&info)
        .await
        .map_err(|error| failed(UpdateStep::Transfer, error.to_string()))?;
    let hello = flasher
        .hello(
            concat!("atlas ", env!("CARGO_PKG_VERSION")),
            serial.as_deref(),
        )
        .await
        .map_err(|error| failed(UpdateStep::Transfer, error.to_string()))?;
    progress.log(format!(
        "flasher {} on {} (serial {})",
        hello.daemon, hello.model, hello.serial
    ));
    match flasher.info().await {
        Ok(emmc) => progress.log(format!(
            "eMMC: {} GB{}{}",
            emmc.size / 1_000_000_000,
            emmc.name
                .map(|name| format!(", {name}"))
                .unwrap_or_default(),
            emmc.life_time
                .map(|wear| format!(", wear {wear}"))
                .unwrap_or_default(),
        )),
        Err(error) => progress.log(format!("could not read the eMMC's details: {error}")),
    }
    progress.step_progress(UpdateStep::Transfer, 1.0);

    // The image goes as it is stored; the board writes as it arrives.
    progress.step_started(UpdateStep::Apply);
    let file = tokio::fs::File::open(&artifact.path)
        .await
        .map_err(|error| {
            failed(
                UpdateStep::Apply,
                format!("{}: {error}", artifact.path.display()),
            )
        })?;
    let size = file
        .metadata()
        .await
        .map_err(|error| failed(UpdateStep::Apply, error.to_string()))?
        .len();
    let started = std::time::Instant::now();
    let sink = progress.clone();
    let mut on_progress =
        move |update: Progress| sink.step_progress(UpdateStep::Apply, update.fraction() as f32);
    let written = tokio::select! {
        written = flasher.write(
            tokio::io::BufReader::with_capacity(1 << 20, file),
            size,
            Some(artifact.sha256.clone()),
            format,
            &mut on_progress,
        ) => written,
        () = cancel.cancelled() => {
            progress.log("stopped; the eMMC is partly written, so the board needs flashing again");
            return Err(DriverError::Cancelled);
        }
    };
    let (bytes, sha256) =
        written.map_err(|error| failed(UpdateStep::Apply, format!("{error}. {AGAIN}")))?;
    progress.step_progress(UpdateStep::Apply, 1.0);

    progress.step_started(UpdateStep::Confirm);
    let sink = progress.clone();
    let mut on_verify =
        move |update: Progress| sink.step_progress(UpdateStep::Confirm, update.fraction() as f32);
    flasher
        .verify(&mut on_verify)
        .await
        .map_err(|error| match error {
            FlasherError::VerifyMismatch { .. } => failed(
                UpdateStep::Confirm,
                format!("the eMMC read back different from what was written. {AGAIN}"),
            ),
            other => failed(UpdateStep::Confirm, other.to_string()),
        })?;
    progress.step_progress(UpdateStep::Confirm, 1.0);
    progress.log(format!(
        "wrote and verified {:.1} GB in {} s (sha256 {})",
        bytes as f64 / 1e9,
        started.elapsed().as_secs(),
        &sha256[..12.min(sha256.len())]
    ));
    if config.ssh_keys.get().is_some() {
        progress.log("your SSH key is added once the new image runs, over the board's USB console");
    }
    if let (Some(path), Some(serial)) = (&config.known_hosts, &serial) {
        match forget_host_keys(path, serial) {
            Ok(0) => {}
            Ok(_) => {
                progress.log("forgot the board's old SSH host key (the new image makes its own)")
            }
            Err(error) => progress.log(format!("could not update {}: {error}", path.display())),
        }
    }

    match flasher.reboot().await {
        Ok(()) => {
            progress.log("the board is restarting into the new image");
            progress.log(RESTARTED);
        }
        Err(error) => progress.log(format!(
            "the image is written and verified, but the board didn't take the restart ({error}); power-cycle it"
        )),
    }
    Ok(UpdateOutcome::Verified {
        version: release.version.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flasher_takes_zstd_and_raw_images() {
        assert_eq!(
            image_format(Path::new("/x/raze-2026.4.img.zst")),
            Some(ImageFormat::Zst)
        );
        assert_eq!(image_format(Path::new("raze.IMG")), Some(ImageFormat::Raw));
        assert_eq!(
            image_format(Path::new("raze.img.xz")),
            Some(ImageFormat::Xz)
        );
        assert_eq!(image_format(Path::new("raze.img.gz")), None);
        assert_eq!(tail8("10000000A317BCBE"), "a317bcbe");
    }
}
