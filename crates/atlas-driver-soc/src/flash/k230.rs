//! The K230 side of a flash: the boot ROM, then Canaan's burner.

use super::*;

impl Job {
    pub(super) fn k230(&self, handle: &Handle) -> Result<String, DriverError> {
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
            let mut io = DeviceIo::open(&info, "K230 USB boot device", BULK_TIMEOUT)
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
                    && let Ok(mut io) = DeviceIo::open(&info, "K230 burner", BULK_TIMEOUT).await
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
