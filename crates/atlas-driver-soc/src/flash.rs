//! Flashing a board from its download mode, as its board record says:
//!
//! - Preflight: the record, its loader file (next to the image, or in the
//!   record's folder; never downloaded), the image decoded to a raw file.
//! - Transfer: Rockchip maskrom runs the loader's DDR init and usbplug; the
//!   K230 boot ROM runs the burner. The board comes back in loader mode on
//!   the same port.
//! - Apply: the image written at the record's offset and read back; on
//!   SPI-NAND, its UBI region written as ubiformat does.
//! - Confirm: the board restarts into what was written.
//!
//! The USB work runs on a blocking thread through the runtime's handle, so
//! atlas-ubi's (synchronous) writer can drive the (async) USB commands.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use atlas_devices::{DeviceCatalog, DevicePackage, FlashSpec, NandGeometry};
use atlas_driver::{
    Artifact, CancellationToken, Concurrency, DriverError, Identity, ProgressSink, ReleaseRef,
    UpdateCapability, UpdateOutcome, UpdatePlan, UpdateStep,
};
use atlas_socflash::k230::bootrom::{self, Stage};
use atlas_socflash::k230::kburn::{Burner, Medium};
use atlas_socflash::rockchip::loader::Loader;
use atlas_socflash::rockchip::maskrom;
use atlas_socflash::rockchip::rockusb::{Reset, Rockusb, SECTOR, Storage, chip_number};
use atlas_socflash::rockchip::write::{LbaNand, write_image};
use atlas_socflash::{DeviceIo, Phase, UsbError};
use tokio::runtime::Handle;

use crate::driver::{Soc, list};
use crate::files::{Section, find_loader, find_ubi_start};

mod k230;
mod rockchip;

const REENUMERATE: Duration = Duration::from_secs(20);
const BULK_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) struct SocUpdate {
    pub soc: Soc,
    pub catalog: Arc<DeviceCatalog>,
}

fn failed(step: UpdateStep, message: impl Into<String>) -> DriverError {
    DriverError::StepFailed {
        step: step.label().into(),
        message: message.into(),
    }
}

fn usb(step: UpdateStep) -> impl Fn(UsbError) -> DriverError {
    move |error| failed(step, error.to_string())
}

fn artifact(release: &ReleaseRef) -> Result<&Artifact, DriverError> {
    release.artifact.as_ref().ok_or_else(|| {
        DriverError::Incompatible(
            "choose an image file to write; add one on the Releases page".into(),
        )
    })
}

impl SocUpdate {
    fn record<'a>(
        &'a self,
        device: &Identity,
    ) -> Result<(&'a DevicePackage, &'a FlashSpec), DriverError> {
        let package = device
            .attributes
            .get("model")
            .and_then(|model| self.catalog.by_model(model))
            .ok_or_else(|| {
                DriverError::Incompatible(format!(
                    "no board record names this {} ({}); Atlas needs one (devices/<model>/manifest.json) to know the loader and the storage",
                    device.attributes.get("chip").map_or("board", String::as_str),
                    device.address
                ))
            })?;
        let spec = package.manifest.recovery.flash.as_ref().ok_or_else(|| {
            DriverError::Incompatible(format!(
                "the {} record doesn't say how to flash it (recovery.flash)",
                package.manifest.model
            ))
        })?;
        Ok((package, spec))
    }
}

/// Where the record's loader file is: next to the image, then in the
/// record's folder.
fn loader_path(
    package: &DevicePackage,
    spec: &FlashSpec,
    image: &Path,
) -> Result<PathBuf, DriverError> {
    if spec.loader.is_empty() {
        return Err(DriverError::Incompatible(format!(
            "the {} record names no loader file",
            package.manifest.model
        )));
    }
    let pattern = spec.loader.join(" or ");
    let dirs: Vec<PathBuf> = image
        .parent()
        .map(Path::to_path_buf)
        .into_iter()
        .chain([package.dir.clone()])
        .collect();
    find_loader(&spec.loader, &dirs).ok_or_else(|| {
        failed(
            UpdateStep::Preflight,
            format!(
                "the vendor loader {pattern} isn't next to the image or in {}; Atlas doesn't download it: put the vendor's file there (Gaia's build of this board carries it)",
                package.dir.display()
            ),
        )
    })
}

#[async_trait]
impl UpdateCapability for SocUpdate {
    fn plan(&self, device: &Identity, release: &ReleaseRef) -> Result<UpdatePlan, DriverError> {
        let artifact = artifact(release)?;
        let (package, spec) = self.record(device)?;
        let storage = spec
            .storage
            .clone()
            .or_else(|| package.manifest.recovery.storage_target.clone())
            .unwrap_or_else(|| "storage".into());
        let how = match self.soc {
            Soc::Rockchip => "Maskrom: run the loader",
            Soc::K230 => "USB boot: run the burner",
        };
        let untested = if package.manifest.recovery.untested() {
            " Written from the vendor's protocol and not yet tested on this board."
        } else {
            ""
        };
        Ok(UpdatePlan {
            steps: vec![
                UpdateStep::Preflight,
                UpdateStep::Transfer,
                UpdateStep::Apply,
                UpdateStep::Confirm,
            ],
            concurrency: Concurrency::Exclusive("usb-boot".into()),
            summary: format!(
                "{how}, write {} to the {storage}, read it back, restart.{untested}",
                artifact.name
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
        let (package, spec) = self.record(device)?;
        if package.manifest.recovery.untested() {
            progress.log(format!(
                "{}: flashing is written from the vendor's protocol and not yet tested on this board",
                package.manifest.display_name()
            ));
        }
        let loader = loader_path(package, spec, &artifact.path)?;
        progress.log(format!("loader: {}", loader.display()));

        // A SPI-NAND board's image may be a partition set (Gaia: env.img
        // with mtdparts, then <partition>.img beside it); else one image.
        let nand_board =
            spec.storage
                .as_deref()
                .or(package.manifest.recovery.storage_target.as_deref())
                == Some("spi-nand");
        let parts = if nand_board {
            crate::layout::partition_set(&artifact.path)
                .transpose()
                .map_err(|error| failed(UpdateStep::Preflight, error))?
        } else {
            None
        };
        let mut temporary = None;
        let (image, size) = if let Some(parts) = &parts {
            for planned in parts {
                let size = planned
                    .part
                    .size
                    .map_or_else(|| "the rest".to_string(), |size| format!("{size} bytes"));
                match &planned.image {
                    Some(path) => progress.log(format!(
                        "partition {} at {:#x} ({size}): {} ({} bytes)",
                        planned.part.name,
                        planned.part.offset,
                        path.display(),
                        planned.bytes
                    )),
                    None => progress.log(format!(
                        "partition {} at {:#x} ({size}): no image, left as is",
                        planned.part.name, planned.part.offset
                    )),
                }
            }
            let extent = parts
                .iter()
                .filter(|planned| planned.image.is_some())
                .map(|planned| planned.part.offset + planned.bytes)
                .max()
                .unwrap_or(0);
            (artifact.path.clone(), extent)
        } else {
            // A raw copy of the image: its size up front, read more than once.
            // In the user's cache, not /tmp, which is often RAM.
            let cache = dirs::cache_dir()
                .unwrap_or_else(std::env::temp_dir)
                .join("atlas")
                .join("soc-images");
            std::fs::create_dir_all(&cache).map_err(|error| {
                failed(
                    UpdateStep::Preflight,
                    format!("{}: {error}", cache.display()),
                )
            })?;
            let raw = cache.join(format!(
                "{}-{}.img",
                std::process::id(),
                artifact.sha256.get(..12).unwrap_or("image")
            ));
            let source = artifact.path.clone();
            let target = raw.clone();
            let size = tokio::task::spawn_blocking(move || {
                atlas_blockdev::decode_to_file(&source, &target)
            })
            .await
            .map_err(|error| failed(UpdateStep::Preflight, error.to_string()))?
            .map_err(|error| failed(UpdateStep::Preflight, error.to_string()))?;
            progress.log(format!("image: {size} bytes"));
            temporary = Some(raw.clone());
            (raw, size)
        };

        // The K230 boot ROM checks the SPL's firmware head on SD and eMMC: a
        // damaged one would never boot, so it stops the flash here.
        if self.soc == Soc::K230 && !nand_board {
            for line in k230_heads(&image).map_err(|error| failed(UpdateStep::Preflight, error))? {
                progress.log(line);
            }
        }

        let job = Job {
            soc: self.soc,
            location: device.address.clone(),
            loader,
            image,
            size,
            parts,
            spec: spec.clone(),
            storage_target: package.manifest.recovery.storage_target.clone(),
            chip: package
                .manifest
                .platform
                .as_ref()
                .and_then(|platform| platform.soc.clone()),
            chip_required: device.attributes.contains_key("pid_unconfirmed"),
            progress: progress.clone(),
            cancel: cancel.clone(),
        };
        let handle = Handle::current();
        let outcome = tokio::task::spawn_blocking(move || job.run(&handle))
            .await
            .map_err(|error| failed(UpdateStep::Apply, error.to_string()));
        if let Some(raw) = temporary {
            let _ = std::fs::remove_file(raw);
        }
        let sha256 = outcome??;
        progress.log(format!("written and read back: sha256 {sha256}"));
        Ok(UpdateOutcome::Verified {
            version: release.version.clone(),
        })
    }
}

/// The SPL firmware heads of a K230 SD/eMMC image, checked: one line each,
/// or why the image can't boot.
fn k230_heads(image: &Path) -> Result<Vec<String>, String> {
    use atlas_socflash::k230::firmware::{BODY_OFFSET, Head, SPL_OFFSETS, check};
    use std::io::{Read, Seek, SeekFrom};
    let mut file = File::open(image).map_err(|error| format!("{}: {error}", image.display()))?;
    let mut lines = Vec::new();
    for offset in SPL_OFFSETS {
        let mut head = vec![0u8; BODY_OFFSET];
        let read = file
            .seek(SeekFrom::Start(offset))
            .and_then(|_| file.read_exact(&mut head));
        if read.is_err() || &head[..4] != atlas_socflash::k230::firmware::MAGIC {
            lines.push(format!(
                "no K230 firmware head at {offset:#x}: the boot ROM may not boot this image"
            ));
            continue;
        }
        let length = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
        let mut body = vec![0u8; length.min(16 << 20)];
        file.read_exact(&mut body)
            .map_err(|_| format!("the K230 firmware head at {offset:#x} runs past the image"))?;
        head.extend(body);
        match check(&head) {
            Head::Valid { length, encryption } => lines.push(format!(
                "K230 SPL head at {offset:#x}: {length} bytes, encryption {encryption}, SHA-256 matches"
            )),
            Head::Damaged(why) => {
                return Err(format!("the K230 firmware head at {offset:#x} is damaged: {why}"));
            }
            Head::Missing => {}
        }
    }
    Ok(lines)
}

/// What the board's chip info says against its record's SoC: a line to log,
/// or why nothing may be written. A different chip always stops the flash.
/// With `required` (the record's PID is a guess), so does a chip that can't
/// be read: a destructive write needs the board itself to say who it is.
fn chip_verdict(
    info: Result<Vec<u8>, String>,
    soc: Option<&str>,
    required: bool,
) -> Result<String, String> {
    let guess = "its USB id is a guess, so the board must confirm its chip";
    let info = match info {
        Ok(info) => info,
        Err(error) if required => {
            return Err(format!(
                "no chip info ({error}); {guess}: nothing was written"
            ));
        }
        Err(error) => return Ok(format!("no chip info ({error})")),
    };
    match (chip_number(&info), soc) {
        (Some(number), Some(soc)) if soc.to_ascii_uppercase().contains(&number) => {
            Ok(format!("chip: {number}"))
        }
        (Some(number), Some(soc)) => Err(format!(
            "the board reports chip {number}, but its record is for {soc}: nothing was written"
        )),
        _ if required => Err(format!(
            "unreadable chip info {info:02x?}; {guess}: nothing was written"
        )),
        (Some(number), None) => Ok(format!("chip: {number}")),
        (None, _) => Ok(format!("chip info: {info:02x?}")),
    }
}

fn sector_lba(offset: u64) -> Result<u32, DriverError> {
    if !offset.is_multiple_of(SECTOR as u64) {
        return Err(failed(
            UpdateStep::Apply,
            format!("offset {offset} isn't on a sector"),
        ));
    }
    u32::try_from(offset / SECTOR as u64)
        .map_err(|_| failed(UpdateStep::Apply, "offset past 2 TiB"))
}

struct Job {
    soc: Soc,
    location: String,
    loader: PathBuf,
    image: PathBuf,
    /// Bytes from the image offset to the end of what's written.
    size: u64,
    /// A partition set (SPI-NAND) instead of one image.
    parts: Option<Vec<crate::layout::Planned>>,
    spec: FlashSpec,
    storage_target: Option<String>,
    /// The record's SoC (`RK3576`), checked against what the board reports.
    chip: Option<String>,
    /// The record's PID is a guess: the board must confirm the chip.
    chip_required: bool,
    progress: ProgressSink,
    cancel: CancellationToken,
}

impl Job {
    fn storage(&self) -> Option<&str> {
        self.spec
            .storage
            .as_deref()
            .or(self.storage_target.as_deref())
    }

    /// The download-mode device at this job's port once it's at `stage`,
    /// waiting out the re-enumeration after a loader starts.
    async fn wait_for(
        &self,
        stage: Option<&str>,
    ) -> Result<atlas_socflash::transport::DeviceInfo, DriverError> {
        let deadline = tokio::time::Instant::now() + REENUMERATE;
        loop {
            if self.cancel.is_cancelled() {
                return Err(DriverError::Cancelled);
            }
            if let Some((_, info)) = list(self.soc).await?.into_iter().find(|(found, _)| {
                found.location == self.location && stage.is_none_or(|stage| found.stage == stage)
            }) {
                return Ok(info);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(failed(
                    UpdateStep::Transfer,
                    format!(
                        "the board didn't come back on USB {} within {} s{}",
                        self.location,
                        REENUMERATE.as_secs(),
                        stage
                            .map(|stage| format!(" in {stage} mode"))
                            .unwrap_or_default()
                    ),
                ));
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    fn report(&self) -> impl FnMut(Phase, u64, u64) + Send + '_ {
        move |phase, done, of| {
            let fraction = if of == 0 {
                1.0
            } else {
                done as f32 / of as f32
            };
            // Writing is the first 70 % of Apply, reading back the rest.
            let (start, span) = match phase {
                Phase::Writing => (0.0, 0.7),
                Phase::Verifying => (0.7, 0.3),
            };
            self.progress
                .step_progress(UpdateStep::Apply, start + span * fraction);
        }
    }

    fn run(self, handle: &Handle) -> Result<String, DriverError> {
        match self.soc {
            Soc::Rockchip => self.rockchip(handle),
            Soc::K230 => self.k230(handle),
        }
    }
}

#[cfg(test)]
mod tests;
