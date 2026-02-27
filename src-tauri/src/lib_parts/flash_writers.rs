fn is_supported_flash_target_path(path: &str) -> bool {
    if cfg!(target_os = "windows") {
        return path
            .to_ascii_lowercase()
            .starts_with("\\\\.\\physicaldrive");
    }

    if cfg!(target_os = "macos") {
        return path.starts_with("/dev/disk") || path.starts_with("/dev/rdisk");
    }

    path.starts_with("/dev/")
}

fn flash_with_rust_stream(
    image_path: &Path,
    device_path: &str,
    progress_context: Option<&FlashProgressContext>,
) -> Result<Output, String> {
    let input = fs::File::open(image_path)
        .map_err(|error| format!("Failed to open image file for flashing: {error}"))?;
    let mut reader: Box<dyn Read> = if image_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .ends_with(".xz")
    {
        Box::new(xz2::read::XzDecoder::new(input))
    } else {
        Box::new(input)
    };

    let mut output = fs::OpenOptions::new()
        .write(true)
        .open(device_path)
        .map_err(|error| format!("Failed to open target device for writing: {error}"))?;

    let total_bytes = total_flash_bytes_for_image(image_path);
    let mut bytes_written = 0u64;
    let mut last_reported_bytes = 0u64;
    let mut buffer = vec![0u8; 8 * 1024 * 1024];
    loop {
        if is_update_cancel_requested() {
            return Err("Update canceled by user.".to_string());
        }
        let read_bytes = reader
            .read(&mut buffer)
            .map_err(|error| format!("Failed while reading image data: {error}"))?;
        if read_bytes == 0 {
            break;
        }
        output
            .write_all(&buffer[..read_bytes])
            .map_err(|error| format!("Failed while writing image to target: {error}"))?;
        bytes_written += read_bytes as u64;
        if let Some(context) = progress_context {
            if bytes_written.saturating_sub(last_reported_bytes) >= 8 * 1024 * 1024 {
                last_reported_bytes = bytes_written;
                emit_flash_write_progress(context, bytes_written, total_bytes);
            }
        }
    }
    if let Some(context) = progress_context {
        emit_flash_write_progress(context, bytes_written, total_bytes);
    }
    output
        .sync_all()
        .map_err(|error| format!("Failed to flush written image to disk: {error}"))?;

    Ok(Output {
        status: success_exit_status(),
        stdout: format!("Wrote {bytes_written} bytes").into_bytes(),
        stderr: Vec::new(),
    })
}

#[cfg(target_os = "linux")]
fn collect_mounted_partitions_from_lsblk_entry(
    entry: &Value,
    mounted: &mut Vec<(String, Vec<String>)>,
) {
    let path = entry
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();

    let mut mountpoints = Vec::new();
    if let Some(points) = entry.get("mountpoints").and_then(Value::as_array) {
        for point in points {
            if let Some(text) = point.as_str() {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    mountpoints.push(trimmed.to_string());
                }
            }
        }
    }
    mountpoints.sort();
    mountpoints.dedup();

    if !path.is_empty() && !mountpoints.is_empty() {
        mounted.push((path, mountpoints));
    }

    if let Some(children) = entry.get("children").and_then(Value::as_array) {
        for child in children {
            collect_mounted_partitions_from_lsblk_entry(child, mounted);
        }
    }
}

#[cfg(target_os = "linux")]
fn mounted_partitions_for_target_linux(
    device_path: &str,
) -> Result<Vec<(String, Vec<String>)>, String> {
    let Some(lsblk_path) = find_in_path("lsblk") else {
        return Err("`lsblk` is required to inspect mounted flash partitions.".to_string());
    };

    let output = Command::new(lsblk_path)
        .args(["-J", "-o", "PATH,TYPE,MOUNTPOINTS"])
        .arg(device_path)
        .output()
        .map_err(|error| format!("Failed to inspect target mount state with lsblk: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "lsblk failed while checking mounted partitions: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("Unable to parse lsblk JSON while checking mount state: {error}"))?;
    let mut mounted = Vec::new();
    if let Some(block_devices) = value.get("blockdevices").and_then(Value::as_array) {
        for device in block_devices {
            collect_mounted_partitions_from_lsblk_entry(device, &mut mounted);
        }
    }
    Ok(mounted)
}

#[cfg(target_os = "linux")]
fn ensure_target_partitions_unmounted_linux(device_path: &str) -> Result<(), String> {
    let mut mounted = mounted_partitions_for_target_linux(device_path)?;
    if mounted.is_empty() {
        return Ok(());
    }

    if let Some(udisksctl_path) = find_in_path("udisksctl") {
        for (partition_path, _) in &mounted {
            let _ = Command::new(&udisksctl_path)
                .args(["unmount", "-b", partition_path])
                .output();
        }
    }

    mounted = mounted_partitions_for_target_linux(device_path)?;
    if mounted.is_empty() {
        return Ok(());
    }

    let details = mounted
        .iter()
        .map(|(partition_path, mountpoints)| {
            format!("{partition_path} ({})", mountpoints.join(", "))
        })
        .collect::<Vec<_>>()
        .join("; ");

    Err(format!(
        "Refusing to flash mounted target {device_path}. Unmount these partitions first: {details}"
    ))
}

fn open_image_reader_for_flash(image_path: &Path) -> Result<Box<dyn Read>, String> {
    let input = fs::File::open(image_path)
        .map_err(|error| format!("Failed to open image file for verification: {error}"))?;
    let lower_name = image_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if lower_name.ends_with(".xz") {
        Ok(Box::new(xz2::read::XzDecoder::new(input)))
    } else {
        Ok(Box::new(input))
    }
}

fn emit_flash_verify_progress(
    context: &FlashProgressContext,
    bytes_verified: u64,
    bytes_total: Option<u64>,
) {
    let (progress_percent, message) = if let Some(total) = bytes_total {
        if total > 0 {
            let percent = ((bytes_verified as f64 / total as f64) * 100.0).clamp(0.0, 100.0);
            let progress_percent = if bytes_verified >= total {
                Some(99.5)
            } else {
                Some(percent)
            };
            (
                progress_percent,
                format!(
                    "Verifying {}: {:.1}% ({bytes_verified}/{total} bytes).",
                    context.target_path, percent
                ),
            )
        } else {
            (
                None,
                format!(
                    "Verifying {}: {bytes_verified} bytes checked.",
                    context.target_path
                ),
            )
        }
    } else {
        (
            None,
            format!(
                "Verifying {}: {bytes_verified} bytes checked.",
                context.target_path
            ),
        )
    };

    let payload = UpdaterProgressEvent {
        run_id: context.run_id.clone(),
        mode: "flash".to_string(),
        step: "verify".to_string(),
        status: "running".to_string(),
        message,
        timestamp_epoch_ms: epoch_ms(),
        stdout: None,
        stderr: None,
        exit_code: None,
        duration_ms: None,
        image_path: None,
        target_path: Some(context.target_path.clone()),
        progress_percent,
        bytes_written: Some(bytes_verified),
        bytes_total,
    };
    let _ = context.app.emit(UPDATER_PROGRESS_EVENT, payload);
}

fn discard_reader_bytes(reader: &mut dyn Read, mut bytes_to_skip: u64) -> Result<(), String> {
    if bytes_to_skip == 0 {
        return Ok(());
    }
    let mut buffer = vec![0u8; 4 * 1024 * 1024];
    while bytes_to_skip > 0 {
        let chunk_len = buffer.len().min(bytes_to_skip as usize);
        let read_bytes = reader
            .read(&mut buffer[..chunk_len])
            .map_err(|error| format!("Failed to skip bytes in source image stream: {error}"))?;
        if read_bytes == 0 {
            return Err(format!(
                "Reached end of source image while skipping to verification offset (remaining {bytes_to_skip} bytes)."
            ));
        }
        bytes_to_skip = bytes_to_skip.saturating_sub(read_bytes as u64);
    }
    Ok(())
}

fn verify_streams_match(
    source: &mut dyn Read,
    target: &mut dyn Read,
    bytes_total: Option<u64>,
    progress_context: Option<&FlashProgressContext>,
    offset_base: u64,
) -> Result<u64, String> {
    let mut source_buffer = vec![0u8; 4 * 1024 * 1024];
    let mut target_buffer = vec![0u8; 4 * 1024 * 1024];
    let mut bytes_verified = 0u64;
    let mut last_reported_bytes = 0u64;
    let mut remaining = bytes_total;

    loop {
        if let Some(left) = remaining {
            if left == 0 {
                break;
            }
        }

        if is_update_cancel_requested() {
            return Err("Update canceled by user.".to_string());
        }

        let read_len = match remaining {
            Some(left) => source_buffer.len().min(left as usize),
            None => source_buffer.len(),
        };
        let source_read = source
            .read(&mut source_buffer[..read_len])
            .map_err(|error| format!("Failed to read source image during verification: {error}"))?;
        if source_read == 0 {
            if remaining.is_some() {
                return Err(format!(
                    "Verification source stream ended early at byte offset {}.",
                    offset_base + bytes_verified
                ));
            }
            break;
        }

        target
            .read_exact(&mut target_buffer[..source_read])
            .map_err(|error| {
                format!(
                    "Verification failed while reading flashed target at byte offset {}: {error}",
                    offset_base + bytes_verified
                )
            })?;

        if source_buffer[..source_read] != target_buffer[..source_read] {
            let mismatch_index = source_buffer[..source_read]
                .iter()
                .zip(target_buffer[..source_read].iter())
                .position(|(left, right)| left != right)
                .unwrap_or(0);
            let mismatch_offset = offset_base + bytes_verified + mismatch_index as u64;
            return Err(format!(
                "Verification failed at byte offset {mismatch_offset}: flashed data does not match source image."
            ));
        }

        bytes_verified += source_read as u64;
        if let Some(left) = remaining.as_mut() {
            *left = left.saturating_sub(source_read as u64);
        }

        if let Some(context) = progress_context {
            if bytes_verified.saturating_sub(last_reported_bytes) >= 8 * 1024 * 1024 {
                last_reported_bytes = bytes_verified;
                emit_flash_verify_progress(context, bytes_verified, bytes_total);
            }
        }
    }

    if let Some(context) = progress_context {
        emit_flash_verify_progress(context, bytes_verified, bytes_total);
    }

    Ok(bytes_verified)
}

#[derive(Clone, Copy)]
struct PartitionSpan {
    part_type: u8,
    start_lba: u64,
    sectors: u64,
}

fn read_mbr_partition_spans(image_path: &Path) -> Result<Option<Vec<PartitionSpan>>, String> {
    let mut reader = open_image_reader_for_flash(image_path)?;
    let mut mbr = [0u8; 512];
    reader
        .read_exact(&mut mbr)
        .map_err(|error| format!("Failed to read image MBR for verification planning: {error}"))?;

    if mbr[510] != 0x55 || mbr[511] != 0xAA {
        return Ok(None);
    }

    let mut partitions = Vec::new();
    for index in 0..4 {
        let base = 446 + (index * 16);
        let part_type = mbr[base + 4];
        let start_lba = u32::from_le_bytes([
            mbr[base + 8],
            mbr[base + 9],
            mbr[base + 10],
            mbr[base + 11],
        ]) as u64;
        let sectors = u32::from_le_bytes([
            mbr[base + 12],
            mbr[base + 13],
            mbr[base + 14],
            mbr[base + 15],
        ]) as u64;
        if part_type == 0 || sectors == 0 {
            continue;
        }
        partitions.push(PartitionSpan {
            part_type,
            start_lba,
            sectors,
        });
    }

    if partitions.is_empty() {
        return Ok(None);
    }

    Ok(Some(partitions))
}

#[cfg(target_os = "linux")]
fn spawn_linux_verify_reader(
    device_path: &str,
    strategy: LinuxPrivilegeStrategy,
    byte_offset: u64,
    byte_count: u64,
    progress_context: Option<&FlashProgressContext>,
) -> Result<Child, String> {
    if byte_offset % 512 != 0 || byte_count % 512 != 0 {
        return Err(format!(
            "Unsupported verification range (offset={byte_offset}, count={byte_count}); expected 512-byte alignment."
        ));
    }
    let skip_blocks = byte_offset / 512;
    let count_blocks = byte_count / 512;

    let dd_path = resolve_required_tool("dd")?;
    let dd_path_string = path_to_string(&dd_path);
    let mut command = match strategy {
        LinuxPrivilegeStrategy::Direct => Command::new(dd_path),
        LinuxPrivilegeStrategy::SudoNoPrompt => {
            let sudo_path = resolve_required_tool("sudo")?;
            let mut command = Command::new(sudo_path);
            command.arg("-n").arg(&dd_path_string);
            command
        }
        LinuxPrivilegeStrategy::PkexecPrompt => {
            emit_flash_elevation_prompt(progress_context, "pkexec");
            let pkexec_path = resolve_required_tool("pkexec")?;
            let mut command = Command::new(pkexec_path);
            command.arg(&dd_path_string);
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

    command
        .arg(format!("if={device_path}"))
        .arg("bs=512")
        .arg("iflag=fullblock")
        .arg(format!("skip={skip_blocks}"))
        .arg(format!("count={count_blocks}"))
        .arg("status=none")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    command
        .spawn()
        .map_err(|error| format!("Failed to start flash verification reader: {error}"))
}

fn verify_flashed_target_range(
    image_path: &Path,
    device_path: &str,
    linux_strategy: Option<LinuxPrivilegeStrategy>,
    progress_context: Option<&FlashProgressContext>,
    byte_offset: u64,
    byte_count: u64,
) -> Result<(), String> {
    let mut image_reader = open_image_reader_for_flash(image_path)?;
    discard_reader_bytes(&mut *image_reader, byte_offset)?;
    let bytes_total = Some(byte_count);

    #[cfg(target_os = "linux")]
    if let Some(strategy) = linux_strategy {
        let mut target_reader_child =
            spawn_linux_verify_reader(device_path, strategy, byte_offset, byte_count, progress_context)?;
        let mut target_stdout = target_reader_child.stdout.take().ok_or_else(|| {
            "Unable to capture verification reader stdout from target device.".to_string()
        })?;

        let bytes_verified = match verify_streams_match(
            &mut *image_reader,
            &mut target_stdout,
            bytes_total,
            progress_context,
            byte_offset,
        ) {
            Ok(bytes_verified) => bytes_verified,
            Err(err) => {
                let _ = target_reader_child.kill();
                let target_output = target_reader_child.wait_with_output().ok();
                let stderr = target_output
                    .as_ref()
                    .map(|output| trim_output(&output.stderr))
                    .unwrap_or_default();
                if stderr.trim().is_empty() {
                    return Err(err);
                }
                return Err(format!("{err}. Verification reader stderr: {stderr}"));
            }
        };

        let target_output = target_reader_child
            .wait_with_output()
            .map_err(|error| format!("Failed to finalize flash verification reader: {error}"))?;
        if !target_output.status.success() {
            return Err(format!(
                "Flash verification reader failed: {}",
                trim_output(&target_output.stderr)
            ));
        }
        if bytes_verified != byte_count {
            return Err(format!(
                "Flash verification length mismatch: verified {bytes_verified} bytes, expected {byte_count} bytes."
            ));
        }
        return Ok(());
    }

    let mut target_file = fs::File::open(device_path)
        .map_err(|error| format!("Failed to open flashed target for verification: {error}"))?;
    use std::io::{Seek, SeekFrom};
    target_file
        .seek(SeekFrom::Start(byte_offset))
        .map_err(|error| format!("Failed to seek flashed target for verification: {error}"))?;

    let bytes_verified = verify_streams_match(
        &mut *image_reader,
        &mut target_file,
        bytes_total,
        progress_context,
        byte_offset,
    )?;
    if bytes_verified != byte_count {
        return Err(format!(
            "Flash verification length mismatch: verified {bytes_verified} bytes, expected {byte_count} bytes."
        ));
    }
    Ok(())
}

fn verify_flashed_target_matches_image(
    image_path: &Path,
    device_path: &str,
    linux_strategy: Option<LinuxPrivilegeStrategy>,
    progress_context: Option<&FlashProgressContext>,
) -> Result<(), String> {
    let root_range = read_mbr_partition_spans(image_path)?
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

    let mut ranges = vec![(0u64, 512u64)];
    if let Some(root) = root_range {
        ranges.push(root);
    }

    for (byte_offset, byte_count) in ranges {
        verify_flashed_target_range(
            image_path,
            device_path,
            linux_strategy,
            progress_context,
            byte_offset,
            byte_count,
        )?;
    }

    Ok(())
}

#[cfg(unix)]
fn success_exit_status() -> std::process::ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    std::process::ExitStatus::from_raw(0)
}

#[cfg(windows)]
fn success_exit_status() -> std::process::ExitStatus {
    use std::os::windows::process::ExitStatusExt;
    std::process::ExitStatus::from_raw(0)
}

fn flash_raw_image(
    image_path: &Path,
    device_path: &str,
    strategy: LinuxPrivilegeStrategy,
    progress_context: Option<&FlashProgressContext>,
) -> Result<Output, String> {
    let dd_path = resolve_required_tool("dd")?;
    let dd_path_string = path_to_string(&dd_path);
    let mut args = vec![
        format!("if={}", image_path.to_string_lossy()),
        format!("of={device_path}"),
        "bs=4M".to_string(),
        "conv=fsync".to_string(),
        "status=progress".to_string(),
    ];

    let mut command = match strategy {
        LinuxPrivilegeStrategy::Direct => Command::new(dd_path),
        LinuxPrivilegeStrategy::SudoNoPrompt => {
            let sudo_path = resolve_required_tool("sudo")?;
            let mut command = Command::new(sudo_path);
            command.arg("-n").arg(&dd_path_string);
            command
        }
        LinuxPrivilegeStrategy::PkexecPrompt => {
            emit_flash_elevation_prompt(progress_context, "pkexec");
            let pkexec_path = resolve_required_tool("pkexec")?;
            let mut command = Command::new(pkexec_path);
            command.arg(&dd_path_string);
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

    for arg in args.drain(..) {
        command.arg(arg);
    }
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = command.spawn().map_err(|error| match strategy {
        LinuxPrivilegeStrategy::Direct => format!("Failed to run dd: {error}"),
        LinuxPrivilegeStrategy::SudoNoPrompt => format!("Failed to run privileged dd: {error}"),
        LinuxPrivilegeStrategy::PkexecPrompt => {
            format!("Failed to run privileged dd via pkexec: {error}")
        }
        LinuxPrivilegeStrategy::SudoPrompt => {
            format!("Failed to run privileged dd via sudo prompt: {error}")
        }
    })?;
    run_dd_with_progress(child, total_flash_bytes_for_image(image_path), progress_context)
}

fn flash_compressed_xz(
    image_path: &Path,
    device_path: &str,
    strategy: LinuxPrivilegeStrategy,
    progress_context: Option<&FlashProgressContext>,
) -> Result<Output, String> {
    let xz_path = resolve_required_tool("xz")?;
    let dd_path = resolve_required_tool("dd")?;
    let dd_path_string = path_to_string(&dd_path);

    if strategy != LinuxPrivilegeStrategy::Direct {
        let xz_path_string = path_to_string(&xz_path);
        let image_path_string = path_to_string(image_path);
        let sh_path = resolve_required_tool("sh")?;
        let sh_path_string = path_to_string(&sh_path);
        let script = format!(
            "{} -dc -- {} | {} of={} bs=4M conv=fsync status=progress",
            shell_single_quote(&xz_path_string),
            shell_single_quote(&image_path_string),
            shell_single_quote(&dd_path_string),
            shell_single_quote(device_path),
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
            let mut command = Command::new(pkexec_path);
            command.arg(dd_path_string);
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
        .arg("status=progress")
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

fn finalize_flashed_target(device_path: &str) -> Result<String, String> {
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

fn trim_output(buffer: &[u8]) -> String {
    const MAX_CHARS: usize = 24_000;
    let text = String::from_utf8_lossy(buffer);
    if text.chars().count() <= MAX_CHARS {
        return text.to_string();
    }

    let truncated: String = text.chars().take(MAX_CHARS).collect();
    format!("{truncated}\n... output truncated ...")
}
