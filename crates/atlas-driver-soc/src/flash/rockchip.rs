//! The Rockchip side of a flash: maskrom, the usbplug, the writes.

use super::*;

impl Job {
    pub(super) fn rockchip(&self, handle: &Handle) -> Result<String, DriverError> {
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
            && !self
                .loader_chips
                .as_ref()
                .is_some_and(|tags| tags.contains(&number))
        {
            return Err(failed(
                UpdateStep::Preflight,
                format!(
                    "{} is a loader for chip {number}; this board is {soc}, whose family accepts {}",
                    self.loader.display(),
                    self.loader_chips
                        .as_ref()
                        .map_or_else(|| "no chip".to_string(), |tags| tags.join(", "))
                ),
            ));
        }

        self.progress.step_started(UpdateStep::Transfer);
        // Every transfer's request and reply in hex lands in the job's log.
        let progress = self.progress.clone();
        let sink: atlas_socflash::traced::Sink =
            std::sync::Arc::new(move |line: &str| progress.log(line.to_string()));
        let mut io = handle.block_on(async {
            let info = self.wait_for(None).await?;
            let maskrom = list(self.soc)
                .await?
                .into_iter()
                .any(|(found, _)| found.location == self.location && found.stage == "maskrom");
            if maskrom {
                let io = NusbIo::open(&info, "Rockchip maskrom device", BULK_TIMEOUT)
                    .await
                    .map_err(usb(UpdateStep::Transfer))?;
                let mut io = Traced::new(Box::new(io), "maskrom").with_sink(sink.clone());
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
            let io = NusbIo::open(&info, "Rockchip loader", BULK_TIMEOUT)
                .await
                .map_err(usb(UpdateStep::Transfer))?;
            Ok::<_, DriverError>(Traced::new(Box::new(io), "loader").with_sink(sink))
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
        // The board must be the chip its record is for.
        let chip = handle
            .block_on(device.read_chip_info())
            .map_err(|error| error.to_string());
        let said = chip_verdict(
            chip,
            self.chip.as_deref(),
            self.loader_chips.as_deref(),
            self.chip_required,
        )
        .map_err(|why| failed(UpdateStep::Transfer, why))?;
        self.progress.log(said);
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
}
