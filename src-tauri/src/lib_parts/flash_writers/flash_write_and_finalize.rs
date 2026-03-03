use super::*;

fn should_retry_without_direct_io(stderr: &[u8]) -> bool {
    let message = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    message.contains("invalid argument")
        || message.contains("direct i/o")
        || message.contains("direct io")
        || message.contains("o_direct")
}

fn emit_direct_io_fallback_notice(progress_context: Option<&FlashProgressContext>) {
    let Some(context) = progress_context else {
        return;
    };
    emit_updater_progress(
        &context.app,
        context.run_id.as_deref(),
        "flash",
        "flash",
        "info",
        "Direct I/O is not supported on this target path. Retrying with buffered writes.",
    );
}

fn read_target_range_bytes(
    device_path: &str,
    linux_strategy: Option<LinuxPrivilegeStrategy>,
    progress_context: Option<&FlashProgressContext>,
    byte_offset: u64,
    byte_count: u64,
) -> Result<Vec<u8>, String> {
    #[cfg(target_os = "linux")]
    if let Some(strategy) = linux_strategy {
        let mut reader =
            spawn_linux_verify_reader(device_path, strategy, byte_offset, byte_count, progress_context)?;
        let mut stdout = reader
            .stdout
            .take()
            .ok_or_else(|| "Unable to capture verification reader stdout for geometry check.".to_string())?;
        let mut bytes = Vec::with_capacity(byte_count as usize);
        std::io::Read::read_to_end(&mut stdout, &mut bytes)
            .map_err(|error| format!("Failed to read target geometry bytes: {error}"))?;
        let output = reader
            .wait_with_output()
            .map_err(|error| format!("Failed to finalize target geometry reader: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "Target geometry reader failed: {}",
                trim_output(&output.stderr)
            ));
        }
        if bytes.len() as u64 != byte_count {
            return Err(format!(
                "Target geometry read length mismatch: read {} bytes, expected {} bytes.",
                bytes.len(),
                byte_count
            ));
        }
        return Ok(bytes);
    }

    let mut file = fs::File::open(device_path)
        .map_err(|error| format!("Failed to open flashed target for geometry check: {error}"))?;
    std::io::Seek::seek(&mut file, std::io::SeekFrom::Start(byte_offset))
        .map_err(|error| format!("Failed to seek flashed target for geometry check: {error}"))?;
    let mut bytes = vec![0u8; byte_count as usize];
    std::io::Read::read_exact(&mut file, &mut bytes)
        .map_err(|error| format!("Failed to read flashed target for geometry check: {error}"))?;
    Ok(bytes)
}

fn verify_ext4_root_geometry(
    device_path: &str,
    linux_strategy: Option<LinuxPrivilegeStrategy>,
    progress_context: Option<&FlashProgressContext>,
    root_range: Option<(u64, u64)>,
) -> Result<(), String> {
    let Some((root_start, root_len)) = root_range else {
        return Ok(());
    };

    // Read the first 4 KiB of the root partition and inspect ext4 superblock fields.
    let probe_len = 4096u64;
    let bytes = read_target_range_bytes(
        device_path,
        linux_strategy,
        progress_context,
        root_start,
        probe_len,
    )?;
    if bytes.len() < 2048 {
        return Err(format!(
            "Root geometry probe was too short ({} bytes).",
            bytes.len()
        ));
    }

    let sb = &bytes[1024..2048];
    let magic = u16::from_le_bytes([sb[56], sb[57]]);
    if magic != 0xEF53 {
        return Err(format!(
            "Root partition does not contain a valid ext4 superblock (magic=0x{magic:04x})."
        ));
    }

    let blocks_lo = u32::from_le_bytes([sb[4], sb[5], sb[6], sb[7]]) as u64;
    let log_block_size = u32::from_le_bytes([sb[24], sb[25], sb[26], sb[27]]);
    let block_size = 1024u64
        .checked_shl(log_block_size)
        .ok_or_else(|| format!("Invalid ext4 block size shift value: {log_block_size}"))?;

    let feature_incompat = u32::from_le_bytes([sb[96], sb[97], sb[98], sb[99]]);
    let mut blocks = blocks_lo;
    // ext4 INCOMPAT_64BIT: include high block-count bits when present.
    if feature_incompat & 0x80 != 0 {
        let blocks_hi = u32::from_le_bytes([sb[0x150], sb[0x151], sb[0x152], sb[0x153]]) as u64;
        blocks |= blocks_hi << 32;
    }

    let fs_bytes = blocks
        .checked_mul(block_size)
        .ok_or_else(|| "Ext4 filesystem size overflow during geometry validation.".to_string())?;
    if fs_bytes > root_len {
        return Err(format!(
            "Root filesystem geometry is invalid: superblock advertises {fs_bytes} bytes, but the root partition is {root_len} bytes."
        ));
    }

    Ok(())
}

pub(crate) fn verify_flashed_target_matches_image(
    image_path: &Path,
    device_path: &str,
    linux_strategy: Option<LinuxPrivilegeStrategy>,
    progress_context: Option<&FlashProgressContext>,
) -> Result<(), String> {
    let partition_spans = read_mbr_partition_spans(image_path)?;
    let root_range = partition_spans
        .as_ref()
        .and_then(|partitions| {
            partitions
                .iter()
                .find(|partition| partition.part_type == 0x83)
                .or_else(|| partitions.get(1))
                .map(|partition| {
                    (
                        partition.start_lba.saturating_mul(512),
                        partition.sectors.saturating_mul(512),
                    )
                })
        })
        .filter(|(_, length)| *length > 0);
    let non_root_range = partition_spans
        .as_ref()
        .and_then(|partitions| {
            partitions
                .iter()
                .map(|partition| {
                    (
                        partition.start_lba.saturating_mul(512),
                        partition.sectors.saturating_mul(512),
                    )
                })
                .find(|(start, len)| {
                    *len > 0
                        && Some((*start, *len)) != root_range
                })
        });

    #[cfg(target_os = "linux")]
    let non_strict_linux = !strict_flash_verify_enabled();
    #[cfg(not(target_os = "linux"))]
    let non_strict_linux = false;

    let mut ranges = vec![(0u64, 512u64)];
    if non_strict_linux {
        if let Some(non_root) = non_root_range {
            ranges.push(non_root);
        }
        if let Some(context) = progress_context {
            emit_updater_progress(
                &context.app,
                context.run_id.as_deref(),
                "flash",
                "verify",
                "info",
                "Linux non-strict verification mode: validating immutable regions and skipping full root-partition byte compare. Set ATLAS_STRICT_FLASH_VERIFY=1 for full strict verification.",
            );
        }
    } else if let Some(root) = root_range {
        ranges.push(root);
    }
    let root_partition_start = root_range.map(|(start, _)| start);
    let root_mutable_metadata_ranges = root_partition_start
        .map(|start| vec![(start.saturating_add(1024), 3 * 1024)])
        .unwrap_or_default();

    for (byte_offset, byte_count) in ranges {
        let ignored_mismatch_ranges = if Some(byte_offset) == root_partition_start {
            Some(root_mutable_metadata_ranges.as_slice())
        } else {
            None
        };
        verify_flashed_target_range(
            image_path,
            device_path,
            linux_strategy,
            progress_context,
            byte_offset,
            byte_count,
            ignored_mismatch_ranges,
        )?;
    }

    // Always validate root ext4 geometry explicitly. Non-strict verification mode skips
    // full root byte comparison, so this catches impossible superblock/partition mismatches.
    verify_ext4_root_geometry(device_path, linux_strategy, progress_context, root_range)?;

    Ok(())
}

#[cfg(unix)]
pub(crate) fn success_exit_status() -> std::process::ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    std::process::ExitStatus::from_raw(0)
}

#[cfg(windows)]
pub(crate) fn success_exit_status() -> std::process::ExitStatus {
    use std::os::windows::process::ExitStatusExt;
    std::process::ExitStatus::from_raw(0)
}

pub(crate) fn flash_raw_image(
    image_path: &Path,
    device_path: &str,
    strategy: LinuxPrivilegeStrategy,
    progress_context: Option<&FlashProgressContext>,
) -> Result<Output, String> {
    let dd_path = resolve_required_tool("dd")?;
    let dd_path_string = path_to_string(&dd_path);
    let run_once = |use_direct_io: bool| -> Result<Output, String> {
        let mut args = vec![
            format!("if={}", image_path.to_string_lossy()),
            format!("of={device_path}"),
            "bs=4M".to_string(),
            "conv=fsync".to_string(),
            "status=progress".to_string(),
        ];
        if use_direct_io {
            args.push("oflag=direct".to_string());
        }

        let mut command = match strategy {
            LinuxPrivilegeStrategy::Direct => Command::new(&dd_path),
            LinuxPrivilegeStrategy::SudoNoPrompt => {
                let sudo_path = resolve_required_tool("sudo")?;
                let mut command = Command::new(sudo_path);
                command.arg("-n").arg(&dd_path_string);
                command
            }
            LinuxPrivilegeStrategy::PkexecPrompt => {
                emit_flash_elevation_prompt(progress_context, "pkexec");
                let pkexec_path = resolve_required_tool("pkexec")?;
                let sh_path = resolve_required_tool("sh")?;
                let sh_path_string = path_to_string(&sh_path);
                let mut command = Command::new(pkexec_path);
                command
                    .arg(sh_path_string)
                    .arg("-c")
                    .arg("exec \"$@\"")
                    .arg("atlas-pkexec")
                    .arg(&dd_path_string);
                command
            }
            LinuxPrivilegeStrategy::SudoPrompt => {
                emit_flash_elevation_prompt(progress_context, "sudo");
                let sudo_path = resolve_required_tool("sudo")?;
                let askpass_path = resolve_sudo_askpass_helper();
                let mut command = Command::new(sudo_path);
                if let Some(path) = askpass_path.as_ref() {
                    command.arg("-A").env("SUDO_ASKPASS", path_to_string(path));
                }
                command.arg(&dd_path_string);
                command
            }
        };

        for arg in args {
            command.arg(arg);
        }
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let child = command.spawn().map_err(|error| match strategy {
            LinuxPrivilegeStrategy::Direct => format!("Failed to run dd: {error}"),
            LinuxPrivilegeStrategy::SudoNoPrompt => {
                format!("Failed to run privileged dd: {error}")
            }
            LinuxPrivilegeStrategy::PkexecPrompt => {
                format!("Failed to run privileged dd via pkexec: {error}")
            }
            LinuxPrivilegeStrategy::SudoPrompt => {
                format!("Failed to run privileged dd via sudo prompt: {error}")
            }
        })?;
        run_dd_with_progress(child, total_flash_bytes_for_image(image_path), progress_context)
    };

    let direct_output = run_once(true)?;
    if direct_output.status.success() || !should_retry_without_direct_io(&direct_output.stderr) {
        return Ok(direct_output);
    }

    emit_direct_io_fallback_notice(progress_context);
    run_once(false)
}

pub(crate) fn flash_compressed_xz(
    image_path: &Path,
    device_path: &str,
    strategy: LinuxPrivilegeStrategy,
    progress_context: Option<&FlashProgressContext>,
) -> Result<Output, String> {
    let direct_output =
        flash_compressed_xz_with_direct_io(image_path, device_path, strategy, progress_context, true)?;
    if direct_output.status.success() || !should_retry_without_direct_io(&direct_output.stderr) {
        return Ok(direct_output);
    }

    emit_direct_io_fallback_notice(progress_context);
    flash_compressed_xz_with_direct_io(image_path, device_path, strategy, progress_context, false)
}

fn flash_compressed_xz_with_direct_io(
    image_path: &Path,
    device_path: &str,
    strategy: LinuxPrivilegeStrategy,
    progress_context: Option<&FlashProgressContext>,
    use_direct_io: bool,
) -> Result<Output, String> {
    let xz_path = resolve_required_tool("xz")?;
    let dd_path = resolve_required_tool("dd")?;
    let dd_path_string = path_to_string(&dd_path);

    if strategy != LinuxPrivilegeStrategy::Direct {
        let xz_path_string = path_to_string(&xz_path);
        let image_path_string = path_to_string(image_path);
        let sh_path = resolve_required_tool("sh")?;
        let sh_path_string = path_to_string(&sh_path);
        let direct_dd_args = if use_direct_io {
            " iflag=fullblock oflag=direct"
        } else {
            ""
        };
        let script = format!(
            "{} -dc -- {} | {} of={} bs=4M conv=fsync status=progress{}",
            shell_single_quote(&xz_path_string),
            shell_single_quote(&image_path_string),
            shell_single_quote(&dd_path_string),
            shell_single_quote(device_path),
            direct_dd_args,
        );

        let mut command = match strategy {
            LinuxPrivilegeStrategy::SudoNoPrompt => {
                let sudo_path = resolve_required_tool("sudo")?;
                let mut command = Command::new(sudo_path);
                command.arg("-n");
                command
            }
            LinuxPrivilegeStrategy::PkexecPrompt => {
                emit_flash_elevation_prompt(progress_context, "pkexec");
                let pkexec_path = resolve_required_tool("pkexec")?;
                Command::new(pkexec_path)
            }
            LinuxPrivilegeStrategy::SudoPrompt => {
                emit_flash_elevation_prompt(progress_context, "sudo");
                let sudo_path = resolve_required_tool("sudo")?;
                let askpass_path = resolve_sudo_askpass_helper();
                let mut command = Command::new(sudo_path);
                if let Some(path) = askpass_path.as_ref() {
                    command.arg("-A").env("SUDO_ASKPASS", path_to_string(path));
                }
                command
            }
            LinuxPrivilegeStrategy::Direct => unreachable!("direct branch handled above"),
        };

        command
            .arg(sh_path_string)
            .arg("-c")
            .arg(script)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let child = command.spawn().map_err(|error| match strategy {
            LinuxPrivilegeStrategy::SudoNoPrompt => {
                format!("Failed to run xz/dd flash via sudo -n: {error}")
            }
            LinuxPrivilegeStrategy::PkexecPrompt => {
                format!("Failed to run xz/dd flash via pkexec: {error}")
            }
            LinuxPrivilegeStrategy::SudoPrompt => {
                format!("Failed to run xz/dd flash via sudo prompt: {error}")
            }
            LinuxPrivilegeStrategy::Direct => format!("Failed to run xz/dd flash: {error}"),
        })?;

        return run_dd_with_progress(child, total_flash_bytes_for_image(image_path), progress_context);
    }

    let mut xz = Command::new(xz_path)
        .args(["-dc"])
        .arg(image_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Failed to start xz decompression: {error}"))?;

    let xz_stdout = xz
        .stdout
        .take()
        .ok_or_else(|| "Unable to capture xz stdout for flashing pipeline.".to_string())?;

    let mut dd_command = match strategy {
        LinuxPrivilegeStrategy::Direct => Command::new(dd_path),
        LinuxPrivilegeStrategy::SudoNoPrompt => {
            let sudo_path = resolve_required_tool("sudo")?;
            let mut command = Command::new(sudo_path);
            command.arg("-n").arg(dd_path_string);
            command
        }
        LinuxPrivilegeStrategy::PkexecPrompt => {
            emit_flash_elevation_prompt(progress_context, "pkexec");
            let pkexec_path = resolve_required_tool("pkexec")?;
            let sh_path = resolve_required_tool("sh")?;
            let sh_path_string = path_to_string(&sh_path);
            let mut command = Command::new(pkexec_path);
            command
                .arg(sh_path_string)
                .arg("-c")
                .arg("exec \"$@\"")
                .arg("atlas-pkexec")
                .arg(dd_path_string);
            command
        }
        LinuxPrivilegeStrategy::SudoPrompt => {
            emit_flash_elevation_prompt(progress_context, "sudo");
            let sudo_path = resolve_required_tool("sudo")?;
            let askpass_path = resolve_sudo_askpass_helper();
            let mut command = Command::new(sudo_path);
            if let Some(path) = askpass_path.as_ref() {
                command.arg("-A").env("SUDO_ASKPASS", path_to_string(path));
            }
            command.arg(dd_path_string);
            command
        }
    };

    dd_command
        .arg(format!("of={device_path}"))
        .arg("bs=4M")
        .arg("conv=fsync")
        .arg("status=progress");
    if use_direct_io {
        dd_command.arg("iflag=fullblock").arg("oflag=direct");
    }
    dd_command
        .stdin(Stdio::from(xz_stdout))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let dd = dd_command
        .spawn()
        .map_err(|error| format!("Failed to run dd for xz image: {error}"))?;
    let dd_output = run_dd_with_progress(dd, total_flash_bytes_for_image(image_path), progress_context)?;

    let xz_output = xz
        .wait_with_output()
        .map_err(|error| format!("Failed to finalize xz process: {error}"))?;

    if !xz_output.status.success() {
        return Ok(Output {
            status: xz_output.status,
            stdout: dd_output.stdout,
            stderr: [dd_output.stderr, xz_output.stderr].concat(),
        });
    }

    Ok(dd_output)
}

pub(crate) fn finalize_flashed_target(device_path: &str) -> Result<String, String> {
    if cfg!(target_os = "linux") {
        let sync_path = resolve_required_tool("sync")
            .map_err(|_| "Unable to find `sync`; cannot safely finalize flash target.".to_string())?;
        let sync_status = Command::new(sync_path)
            .status()
            .map_err(|error| format!("Failed to execute `sync` before finalize: {error}"))?;
        if !sync_status.success() {
            return Err("`sync` failed while finalizing flashed target.".to_string());
        }

        if let Ok(udisksctl_path) = resolve_required_tool("udisksctl") {
            if let Ok(output) = Command::new(udisksctl_path)
                .args(["power-off", "-b", device_path])
                .output()
            {
                if output.status.success() {
                    return Ok(format!(
                        "Flash target {device_path} powered off. Device should reboot; if not, unplug/replug USB."
                    ));
                }
            }
        }

        if let Ok(eject_path) = resolve_required_tool("eject") {
            if let Ok(output) = Command::new(eject_path).arg(device_path).output() {
                if output.status.success() {
                    return Ok(format!(
                        "Flash target {device_path} ejected. Power cycle the device if it does not reboot automatically."
                    ));
                }
            }
        }

        return Ok(format!(
            "Flash target {device_path} was synced, but automatic eject/power-cycle was unavailable. Manually power cycle the device now."
        ));
    }

    if cfg!(target_os = "macos") {
        let sync_path = resolve_required_tool("sync")
            .map_err(|_| "Unable to find `sync`; cannot safely finalize flash target.".to_string())?;
        let sync_status = Command::new(sync_path)
            .status()
            .map_err(|error| format!("Failed to execute `sync` before finalize: {error}"))?;
        if !sync_status.success() {
            return Err("`sync` failed while finalizing flashed target.".to_string());
        }

        if let Ok(diskutil_path) = resolve_required_tool("diskutil") {
            if let Ok(output) = Command::new(diskutil_path)
                .args(["eject", device_path])
                .output()
            {
                if output.status.success() {
                    return Ok(format!(
                        "Flash target {device_path} ejected. Power cycle the device if it does not reboot automatically."
                    ));
                }
            }
        }

        return Ok(format!(
            "Flash target {device_path} was synced, but automatic eject/power-cycle was unavailable. Manually power cycle the device now."
        ));
    }

    Err(format!(
        "Automatic power-cycle is not supported on this platform for {device_path}. Manually power cycle the device now."
    ))
}

pub(crate) fn trim_output(buffer: &[u8]) -> String {
    const MAX_CHARS: usize = 24_000;
    let text = String::from_utf8_lossy(buffer);
    if text.chars().count() <= MAX_CHARS {
        return text.to_string();
    }

    let truncated: String = text.chars().take(MAX_CHARS).collect();
    format!("{truncated}\n... output truncated ...")
}
