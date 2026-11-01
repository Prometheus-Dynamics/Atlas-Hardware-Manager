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
use atlas_socflash::{NusbIo, Phase, UsbError};
use tokio::runtime::Handle;

use crate::driver::{Soc, list};
use crate::files::{Section, find_loader, find_ubi_start};

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

    fn rockchip(&self, handle: &Handle) -> Result<String, DriverError> {
        let bytes = std::fs::read(&self.loader).map_err(|error| {
            failed(
                UpdateStep::Preflight,
                format!("{}: {error}", self.loader.display()),
            )
        })?;
        let loader = Loader::parse(&bytes)
            .map_err(|error| failed(UpdateStep::Preflight, error.to_string()))?;
        self.progress.log(format!(
            "loader: chip {}, {} DDR, {} usbplug, {} loader entries; trailer CRC polynomial {:#010x}",
            loader.chip_number().unwrap_or_else(|| format!("{:#010x}", loader.chip)),
            loader
                .entries(atlas_socflash::rockchip::loader::EntryKind::Ddr)
                .count(),
            loader
                .entries(atlas_socflash::rockchip::loader::EntryKind::Usbplug)
                .count(),
            loader
                .entries(atlas_socflash::rockchip::loader::EntryKind::Loader)
                .count(),
            loader.crc_polynomial
        ));

        // A loader for another chip would run the wrong DDR init.
        if let (Some(number), Some(soc)) = (loader.chip_number(), self.chip.as_deref())
            && !soc.to_ascii_uppercase().contains(&number)
        {
            return Err(failed(
                UpdateStep::Preflight,
                format!(
                    "{} is a loader for chip {number}; this board is {soc}",
                    self.loader.display()
                ),
            ));
        }

        self.progress.step_started(UpdateStep::Transfer);
        let mut io = handle.block_on(async {
            let info = self.wait_for(None).await?;
            let maskrom = list(self.soc)
                .await?
                .into_iter()
                .any(|(found, _)| found.location == self.location && found.stage == "maskrom");
            if maskrom {
                let mut io = NusbIo::open(&info, "Rockchip maskrom device", BULK_TIMEOUT)
                    .await
                    .map_err(usb(UpdateStep::Transfer))?;
                self.progress
                    .log("maskrom: sending the DDR init and the usbplug");
                maskrom::download_boot(&mut io, &loader, &bytes)
                    .await
                    .map_err(usb(UpdateStep::Transfer))?;
                drop(io);
                self.progress.step_progress(UpdateStep::Transfer, 0.6);
            } else {
                self.progress
                    .log("already in loader mode; no maskrom download");
            }
            let info = self.wait_for(Some("loader")).await?;
            NusbIo::open(&info, "Rockchip loader", BULK_TIMEOUT)
                .await
                .map_err(usb(UpdateStep::Transfer))
        })?;
        let mut device = Rockusb::new(&mut io);
        handle.block_on(async {
            let mut tries = 0;
            loop {
                match device.test_unit_ready().await {
                    Ok(()) => break Ok(()),
                    Err(error) if tries >= 10 => break Err(usb(UpdateStep::Transfer)(error)),
                    Err(_) => {
                        tries += 1;
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                }
            }
        })?;
        // The board must be the chip its record is for: a guessed PID
        // (pid_unconfirmed) could name the wrong one.
        match handle.block_on(device.read_chip_info()) {
            Ok(info) => match (chip_number(&info), self.chip.as_deref()) {
                (Some(number), Some(soc)) if !soc.to_ascii_uppercase().contains(&number) => {
                    return Err(failed(
                        UpdateStep::Transfer,
                        format!(
                            "the board reports chip {number}, but its record is for {soc}: nothing was written"
                        ),
                    ));
                }
                (Some(number), _) => self.progress.log(format!("chip: {number}")),
                (None, _) => self.progress.log(format!("chip info: {info:02x?}")),
            },
            Err(error) => self.progress.log(format!("no chip info ({error})")),
        }
        let storage = match self.storage() {
            Some("emmc") => Some(Storage::Emmc),
            Some("sd") => Some(Storage::Sd),
            Some("spi-nor") => Some(Storage::SpiNor),
            _ => None,
        };
        if let Some(storage) = storage {
            handle
                .block_on(device.change_storage(storage))
                .map_err(usb(UpdateStep::Transfer))?;
        }
        let info = handle
            .block_on(device.read_flash_info())
            .map_err(usb(UpdateStep::Transfer))?;
        self.progress
            .log(format!("storage: {} bytes", info.bytes()));
        if info.sectors > 0 && self.spec.image_offset + self.size > info.bytes() {
            return Err(failed(
                UpdateStep::Transfer,
                format!(
                    "the image ({} bytes at {}) doesn't fit the board's {}-byte storage",
                    self.size,
                    self.spec.image_offset,
                    info.bytes()
                ),
            ));
        }
        self.progress.step_progress(UpdateStep::Transfer, 1.0);

        self.progress.step_started(UpdateStep::Apply);
        let cancelled = || self.cancel.is_cancelled();
        let open = |start: u64, length: u64| {
            Section::open(&self.image, start, length)
                .map_err(|error| failed(UpdateStep::Apply, error.to_string()))
        };
        let base_lba = |offset: u64| -> Result<u32, DriverError> {
            if !offset.is_multiple_of(SECTOR as u64) {
                return Err(failed(
                    UpdateStep::Apply,
                    format!("offset {offset} isn't on a sector"),
                ));
            }
            u32::try_from(offset / SECTOR as u64)
                .map_err(|_| failed(UpdateStep::Apply, "offset past 2 TiB"))
        };
        let apply = |error: atlas_socflash::rockchip::write::WriteError| match error {
            atlas_socflash::rockchip::write::WriteError::Cancelled => DriverError::Cancelled,
            other => failed(UpdateStep::Apply, other.to_string()),
        };

        let sha256 = match (self.storage(), self.spec.nand) {
            (Some("spi-nand"), Some(nand)) => {
                let flash_bytes = nand.size.unwrap_or(info.bytes());
                if let Some(parts) = &self.parts {
                    let mut written = Vec::new();
                    for planned in parts {
                        let Some(path) = &planned.image else { continue };
                        let offset = self.spec.image_offset + planned.part.offset;
                        let region = planned
                            .part
                            .size
                            .unwrap_or_else(|| flash_bytes.saturating_sub(offset));
                        self.progress
                            .log(format!("writing partition {}", planned.part.name));
                        let ubi = find_ubi_start(path, nand.peb_size)
                            .map_err(|error| failed(UpdateStep::Apply, error.to_string()))?;
                        if ubi == Some(0) {
                            self.nand_ubi(
                                handle,
                                &mut device,
                                path,
                                0,
                                planned.bytes,
                                offset,
                                region,
                                nand,
                            )?;
                            written.push(format!("{} (UBI)", planned.part.name));
                        } else {
                            let sha = self.nand_raw(
                                handle,
                                &mut device,
                                path,
                                0,
                                planned.bytes,
                                offset,
                                nand,
                            )?;
                            written.push(format!("{} {}", planned.part.name, &sha[..12]));
                        }
                    }
                    written.join(", ")
                } else {
                    let ubi = find_ubi_start(&self.image, nand.peb_size)
                        .map_err(|error| failed(UpdateStep::Apply, error.to_string()))?;
                    let head = ubi.unwrap_or(self.size);
                    let sha = self.nand_raw(
                        handle,
                        &mut device,
                        &self.image,
                        0,
                        head,
                        self.spec.image_offset,
                        nand,
                    )?;
                    if let Some(start) = ubi {
                        let offset = self.spec.image_offset + start;
                        self.nand_ubi(
                            handle,
                            &mut device,
                            &self.image,
                            start,
                            self.size - start,
                            offset,
                            flash_bytes.saturating_sub(offset),
                            nand,
                        )?;
                    }
                    sha
                }
            }
            (Some("spi-nand"), None) => {
                return Err(failed(
                    UpdateStep::Preflight,
                    "the record's SPI-NAND has no geometry (recovery.flash.nand)",
                ));
            }
            _ => {
                let mut report = self.report();
                handle
                    .block_on(write_image(
                        &mut device,
                        &mut open(0, self.size)?,
                        self.size,
                        base_lba(self.spec.image_offset)?,
                        true,
                        &mut report,
                        &cancelled,
                    ))
                    .map_err(apply)?
                    .sha256
            }
        };

        self.progress.step_started(UpdateStep::Confirm);
        handle
            .block_on(device.reset(Reset::Normal))
            .map_err(usb(UpdateStep::Confirm))?;
        self.progress.log("the board restarts into the new image");
        Ok(sha256)
    }

    /// Erases whole eraseblocks over `bytes` from `offset`, then writes
    /// them from `path` (`start`..) and reads them back.
    #[allow(clippy::too_many_arguments)]
    fn nand_raw(
        &self,
        handle: &Handle,
        device: &mut Rockusb<'_>,
        path: &Path,
        start: u64,
        bytes: u64,
        offset: u64,
        nand: NandGeometry,
    ) -> Result<String, DriverError> {
        let peb = u64::from(nand.peb_size);
        if !offset.is_multiple_of(peb) {
            return Err(failed(
                UpdateStep::Apply,
                format!("offset {offset:#x} isn't on an eraseblock ({peb} bytes)"),
            ));
        }
        let lba = sector_lba(offset)?;
        let per_block = (peb / SECTOR as u64) as u16;
        for block in 0..bytes.div_ceil(peb) {
            handle
                .block_on(device.erase_lba(lba + (block * u64::from(per_block)) as u32, per_block))
                .map_err(usb(UpdateStep::Apply))?;
        }
        let mut section = Section::open(path, start, bytes)
            .map_err(|error| failed(UpdateStep::Apply, error.to_string()))?;
        let mut report = self.report();
        let cancelled = || self.cancel.is_cancelled();
        handle
            .block_on(write_image(
                device,
                &mut section,
                bytes,
                lba,
                true,
                &mut report,
                &cancelled,
            ))
            .map(|written| written.sha256)
            .map_err(|error| match error {
                atlas_socflash::rockchip::write::WriteError::Cancelled => DriverError::Cancelled,
                other => failed(UpdateStep::Apply, other.to_string()),
            })
    }

    /// Writes the UBI image at `path` (`start`.., `bytes` long) to the
    /// `region` bytes of flash from `offset`, as ubiformat does.
    #[allow(clippy::too_many_arguments)]
    fn nand_ubi(
        &self,
        handle: &Handle,
        device: &mut Rockusb<'_>,
        path: &Path,
        start: u64,
        bytes: u64,
        offset: u64,
        region: u64,
        nand: NandGeometry,
    ) -> Result<(), DriverError> {
        let mut section = Section::open(path, start, bytes)
            .map_err(|error| failed(UpdateStep::Apply, error.to_string()))?;
        let image = atlas_ubi::read_image(&mut section, nand.peb_size)
            .map_err(|error| failed(UpdateStep::Apply, format!("the UBI image: {error}")))?;
        let mut nand_io = LbaNand {
            device,
            runtime: handle.clone(),
            base_lba: sector_lba(offset)?,
            pebs: (region / u64::from(nand.peb_size)) as u32,
            peb_size: nand.peb_size,
            page_size: nand.page_size,
        };
        let progress = self.progress.clone();
        let formatted =
            atlas_ubi::format(&mut nand_io, &mut section, &image, true, &mut |done, of| {
                progress.step_progress(
                    UpdateStep::Apply,
                    0.5 + 0.5 * done as f32 / of.max(1) as f32,
                );
            })
            .map_err(|error| failed(UpdateStep::Apply, error.to_string()))?;
        self.progress.log(format!(
            "UBI: {} PEBs written, {} given empty headers, {} marked bad",
            formatted.placed.len(),
            formatted.empty_pebs,
            formatted.marked_bad.len()
        ));
        Ok(())
    }

    fn k230(&self, handle: &Handle) -> Result<String, DriverError> {
        let loader = std::fs::read(&self.loader).map_err(|error| {
            failed(
                UpdateStep::Preflight,
                format!("{}: {error}", self.loader.display()),
            )
        })?;
        let address = match self.spec.load_address.as_deref() {
            Some(text) => u32::from_str_radix(text.trim_start_matches("0x"), 16)
                .map_err(|_| failed(UpdateStep::Preflight, format!("load address {text}")))?,
            None => bootrom::DEFAULT_LOAD_ADDRESS,
        };
        let medium = self.storage().and_then(Medium::from_name).ok_or_else(|| {
            failed(
                UpdateStep::Preflight,
                "the record names no K230 medium (emmc, sd, spi-nand, spi-nor)",
            )
        })?;

        self.progress.step_started(UpdateStep::Transfer);
        let mut io = handle.block_on(async {
            let info = self.wait_for(None).await?;
            let mut io = NusbIo::open(&info, "K230 USB boot device", BULK_TIMEOUT)
                .await
                .map_err(usb(UpdateStep::Transfer))?;
            match bootrom::stage(&mut io).await.map_err(usb(UpdateStep::Transfer))? {
                Stage::Burner => {
                    self.progress.log("the burner already runs");
                    return Ok(io);
                }
                Stage::BootRom => {}
                Stage::Unknown(text) => {
                    return Err(failed(UpdateStep::Transfer, format!("the board answered {text:?}, not K230")));
                }
            }
            self.progress.log(format!("boot ROM: loading the burner at {address:#010x}"));
            bootrom::run_loader(&mut io, &loader, address)
                .await
                .map_err(usb(UpdateStep::Transfer))?;
            drop(io);
            self.progress.step_progress(UpdateStep::Transfer, 0.5);
            // Same IDs before and after: wait for the burner to answer.
            let deadline = tokio::time::Instant::now() + REENUMERATE;
            loop {
                tokio::time::sleep(Duration::from_millis(500)).await;
                if let Ok(info) = self.wait_for(None).await
                    && let Ok(mut io) = NusbIo::open(&info, "K230 burner", BULK_TIMEOUT).await
                    && matches!(bootrom::stage(&mut io).await, Ok(Stage::Burner))
                {
                    return Ok(io);
                }
                if tokio::time::Instant::now() >= deadline {
                    return Err(failed(
                        UpdateStep::Transfer,
                        format!(
                            "the burner didn't answer within {} s: check the load address in the record ({address:#010x})",
                            REENUMERATE.as_secs()
                        ),
                    ));
                }
            }
        })?;
        let mut burner = Burner::new(&mut io);
        handle
            .block_on(burner.probe(medium))
            .map_err(usb(UpdateStep::Transfer))?;
        let info = handle
            .block_on(burner.info())
            .map_err(usb(UpdateStep::Transfer))?;
        self.progress.log(format!(
            "medium: {} bytes, {}-byte blocks, {}-byte erase blocks",
            info.capacity, info.block_size, info.erase_size
        ));
        if info.write_protected {
            return Err(failed(
                UpdateStep::Transfer,
                "the medium is write-protected",
            ));
        }
        if info.capacity > 0 && self.spec.image_offset + self.size > info.capacity {
            return Err(failed(
                UpdateStep::Transfer,
                format!(
                    "the image ({} bytes) doesn't fit the {}-byte medium",
                    self.size, info.capacity
                ),
            ));
        }
        self.progress.step_progress(UpdateStep::Transfer, 1.0);

        self.progress.step_started(UpdateStep::Apply);
        let unit = info.erase_size.max(info.block_size).max(1);
        let region = self.size.div_ceil(unit) * unit;
        let mut image = File::open(&self.image)
            .map_err(|error| failed(UpdateStep::Apply, error.to_string()))?;
        let mut report = self.report();
        let cancelled = || self.cancel.is_cancelled();
        let sent = handle
            .block_on(burner.write(
                self.spec.image_offset,
                &mut image,
                self.size,
                region,
                &mut report,
                &cancelled,
            ))
            .map_err(usb(UpdateStep::Apply))?;
        match handle.block_on(burner.sha256(self.spec.image_offset, self.size)) {
            Ok(read) if read == sent => self.progress.log("the burner's checksum matches"),
            Ok(read) => {
                return Err(failed(
                    UpdateStep::Apply,
                    format!("the burner reads back {read}, not the {sent} written"),
                ));
            }
            Err(error) => self.progress.log(format!(
                "written, not read back: the burner can't checksum ({error})"
            )),
        }
        self.progress.step_started(UpdateStep::Confirm);
        handle
            .block_on(burner.reboot())
            .map_err(usb(UpdateStep::Confirm))?;
        self.progress.log("the board restarts into the new image");
        Ok(sent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Gaia's real outputs, when they're on this machine (skipped otherwise).
    const GAIA: &str = "/mnt/ssd-samsung/gaia/work/.gaia/examples";

    #[test]
    fn gaias_k230d_image_has_valid_spl_heads() {
        let image = Path::new(GAIA).join("k230d-zero/out/images/sdcard.img");
        if !image.is_file() {
            return;
        }
        let lines = k230_heads(&image).unwrap();
        assert_eq!(lines.len(), 2);
        assert!(
            lines.iter().all(|line| line.contains("SHA-256 matches")),
            "{lines:?}"
        );
    }

    #[test]
    fn gaias_luckfox_output_is_a_partition_set() {
        let dir = Path::new(GAIA).join("luckfox-pico-pro/out/images");
        if !dir.join("env.img").is_file() {
            return;
        }
        let parts = crate::layout::partition_set(&dir.join("ubi.img"))
            .unwrap()
            .unwrap();
        let named: Vec<(&str, u64, bool)> = parts
            .iter()
            .map(|planned| {
                (
                    planned.part.name.as_str(),
                    planned.part.offset,
                    planned.image.is_some(),
                )
            })
            .collect();
        assert_eq!(
            named,
            [
                ("env", 0, true),
                ("idblock", 256 << 10, true),
                ("uboot", 512 << 10, true),
                ("boot", 1 << 20, true),
                ("boot_b", 7 << 20, false),
                ("ubi", 13 << 20, true),
            ]
        );
        // The UBI partition's image is UBI from its first byte, and reads.
        let ubi = dir.join("ubi.img");
        assert_eq!(find_ubi_start(&ubi, 128 << 10).unwrap(), Some(0));
        let image = atlas_ubi::read_image(&mut File::open(&ubi).unwrap(), 128 << 10).unwrap();
        let names: Vec<&str> = image.volumes.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, ["rootfs_a", "rootfs_b", "data"]);
    }
}
