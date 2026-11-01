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
use atlas_devices::{DeviceCatalog, DevicePackage, FlashSpec};
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
        let size =
            tokio::task::spawn_blocking(move || atlas_blockdev::decode_to_file(&source, &target))
                .await
                .map_err(|error| failed(UpdateStep::Preflight, error.to_string()))?
                .map_err(|error| failed(UpdateStep::Preflight, error.to_string()))?;
        progress.log(format!("image: {size} bytes"));

        let job = Job {
            soc: self.soc,
            location: device.address.clone(),
            loader,
            image: raw.clone(),
            size,
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
        let _ = std::fs::remove_file(&raw);
        let sha256 = outcome??;
        progress.log(format!("written and read back: sha256 {sha256}"));
        Ok(UpdateOutcome::Verified {
            version: release.version.clone(),
        })
    }
}

struct Job {
    soc: Soc,
    location: String,
    loader: PathBuf,
    image: PathBuf,
    size: u64,
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
                let ubi = find_ubi_start(&self.image, nand.peb_size)
                    .map_err(|error| failed(UpdateStep::Apply, error.to_string()))?;
                let head = ubi.unwrap_or(self.size);
                let mut report = self.report();
                let written = handle
                    .block_on(write_image(
                        &mut device,
                        &mut open(0, head)?,
                        head,
                        base_lba(self.spec.image_offset)?,
                        true,
                        &mut report,
                        &cancelled,
                    ))
                    .map_err(apply)?;
                if let Some(start) = ubi {
                    let mut section = open(start, self.size - start)?;
                    let image =
                        atlas_ubi::read_image(&mut section, nand.peb_size).map_err(|error| {
                            failed(
                                UpdateStep::Apply,
                                format!("the image's UBI region: {error}"),
                            )
                        })?;
                    let flash_bytes = nand.size.unwrap_or(info.bytes());
                    let region = flash_bytes.saturating_sub(self.spec.image_offset + start);
                    let mut nand_io = LbaNand {
                        device: &mut device,
                        runtime: handle.clone(),
                        base_lba: base_lba(self.spec.image_offset + start)?,
                        pebs: (region / u64::from(nand.peb_size)) as u32,
                        peb_size: nand.peb_size,
                        page_size: nand.page_size,
                    };
                    let progress = self.progress.clone();
                    let formatted = atlas_ubi::format(
                        &mut nand_io,
                        &mut section,
                        &image,
                        true,
                        &mut |done, of| {
                            progress.step_progress(
                                UpdateStep::Apply,
                                0.5 + 0.5 * done as f32 / of.max(1) as f32,
                            );
                        },
                    )
                    .map_err(|error| failed(UpdateStep::Apply, error.to_string()))?;
                    self.progress.log(format!(
                        "UBI: {} PEBs written, {} given empty headers, {} marked bad",
                        formatted.placed.len(),
                        formatted.empty_pebs,
                        formatted.marked_bad.len()
                    ));
                }
                written.sha256
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
