fn run_command_with_timeout(
    program: &str,
    args: &[&str],
    timeout: Duration,
) -> Result<TimedCommandOutput, String> {
    run_command_with_timeout_and_env(program, args, &[], timeout)
}

fn run_command_with_timeout_and_env(
    program: &str,
    args: &[&str],
    envs: &[(&str, &str)],
    timeout: Duration,
) -> Result<TimedCommandOutput, String> {
    let program_path = resolve_required_tool(program)?;
    let start = Instant::now();
    let mut command = Command::new(&program_path);
    command.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
    for (key, value) in envs {
        command.env(key, value);
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("Failed to spawn {}: {error}", path_to_string(&program_path)))?;

    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                let output = child
                    .wait_with_output()
                    .map_err(|error| format!("Failed to capture {program} output: {error}"))?;
                return Ok(TimedCommandOutput {
                    output,
                    timed_out: false,
                    canceled: false,
                    duration_ms: start.elapsed().as_millis() as u64,
                });
            }
            Ok(None) => {
                if is_update_cancel_requested() {
                    let _ = child.kill();
                    let mut output = child.wait_with_output().map_err(|error| {
                        format!("Failed to collect canceled {program} output: {error}")
                    })?;
                    output
                        .stderr
                        .extend_from_slice(b"\nOperation canceled by user.");
                    return Ok(TimedCommandOutput {
                        output,
                        timed_out: false,
                        canceled: true,
                        duration_ms: start.elapsed().as_millis() as u64,
                    });
                }
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let output = child.wait_with_output().map_err(|error| {
                        format!("Failed to collect timed-out {program} output: {error}")
                    })?;
                    return Ok(TimedCommandOutput {
                        output,
                        timed_out: true,
                        canceled: false,
                        duration_ms: start.elapsed().as_millis() as u64,
                    });
                }
                thread::sleep(Duration::from_millis(200));
            }
            Err(error) => {
                return Err(format!("Failed while waiting on {program}: {error}"));
            }
        }
    }
}

fn run_dd_with_progress(
    mut child: Child,
    total_bytes: Option<u64>,
    progress_context: Option<&FlashProgressContext>,
) -> Result<Output, String> {
    const FLASH_PROCESS_TIMEOUT: Duration = Duration::from_secs(45 * 60);
    let mut stdout_handle = child
        .stdout
        .take()
        .ok_or_else(|| "Unable to capture dd stdout.".to_string())?;
    let mut stderr_handle = child
        .stderr
        .take()
        .ok_or_else(|| "Unable to capture dd stderr.".to_string())?;

    let stdout_bytes = std::sync::Arc::new(Mutex::new(Vec::<u8>::new()));
    let stderr_bytes = std::sync::Arc::new(Mutex::new(Vec::<u8>::new()));
    let stdout_done = std::sync::Arc::new(AtomicBool::new(false));
    let stderr_done = std::sync::Arc::new(AtomicBool::new(false));
    let latest_written_bytes = std::sync::Arc::new(AtomicU64::new(0));

    let stdout_bytes_thread = std::sync::Arc::clone(&stdout_bytes);
    let stdout_done_thread = std::sync::Arc::clone(&stdout_done);
    thread::spawn(move || {
        let mut buffer = [0u8; 4096];
        loop {
            match stdout_handle.read(&mut buffer) {
                Ok(0) => break,
                Ok(read_bytes) => {
                    if let Ok(mut target) = stdout_bytes_thread.lock() {
                        target.extend_from_slice(&buffer[..read_bytes]);
                    }
                }
                Err(_) => break,
            }
        }
        stdout_done_thread.store(true, Ordering::Relaxed);
    });

    let stderr_bytes_thread = std::sync::Arc::clone(&stderr_bytes);
    let stderr_done_thread = std::sync::Arc::clone(&stderr_done);
    let latest_written_bytes_thread = std::sync::Arc::clone(&latest_written_bytes);
    let progress_context_for_thread = progress_context.cloned();
    thread::spawn(move || {
        let mut buffer = [0u8; 4096];
        let mut tail = String::new();
        let mut last_reported_bytes = 0u64;
        let mut last_emit_instant = Instant::now() - Duration::from_secs(2);

        loop {
            match stderr_handle.read(&mut buffer) {
                Ok(0) => break,
                Ok(read_bytes) => {
                    if let Ok(mut target) = stderr_bytes_thread.lock() {
                        target.extend_from_slice(&buffer[..read_bytes]);
                    }

                    let chunk = String::from_utf8_lossy(&buffer[..read_bytes]);
                    tail.push_str(&chunk);
                    if tail.len() > 16_384 {
                        tail = tail.split_off(tail.len() - 8_192);
                    }

                    if let Some(bytes_written) = extract_latest_dd_written_bytes(&tail) {
                        let should_emit = bytes_written > last_reported_bytes
                            && (bytes_written.saturating_sub(last_reported_bytes) >= 8 * 1024 * 1024
                                || last_emit_instant.elapsed() >= Duration::from_millis(600));
                        if should_emit {
                            last_reported_bytes = bytes_written;
                            latest_written_bytes_thread.store(bytes_written, Ordering::Relaxed);
                            last_emit_instant = Instant::now();
                            if let Some(context) = progress_context_for_thread.as_ref() {
                                emit_flash_write_progress(context, bytes_written, total_bytes);
                            }
                        }
                    }
                }
                Err(_) => break,
            }
        }

        if let Some(bytes_written) = extract_latest_dd_written_bytes(&tail) {
            if bytes_written > last_reported_bytes {
                if let Some(context) = progress_context_for_thread.as_ref() {
                    emit_flash_write_progress(context, bytes_written, total_bytes);
                }
            }
        }
        stderr_done_thread.store(true, Ordering::Relaxed);
    });

    let started_wait = Instant::now();
    let mut timed_out = false;
    let mut last_heartbeat = Instant::now() - Duration::from_secs(30);
    let mut copy_complete_logged = false;
    let mut copy_complete_at = None;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if is_update_cancel_requested() {
                    let _ = child.kill();
                }
                if started_wait.elapsed() >= FLASH_PROCESS_TIMEOUT {
                    timed_out = true;
                    let _ = child.kill();
                } else if let Some(context) = progress_context {
                    if last_heartbeat.elapsed() >= Duration::from_secs(8) {
                        let written = latest_written_bytes.load(Ordering::Relaxed);
                        if let Some(total) = total_bytes {
                            if total > 0 && written >= total {
                                if !copy_complete_logged {
                                    copy_complete_logged = true;
                                    copy_complete_at = Some(Instant::now());
                                    emit_updater_progress(
                                        &context.app,
                                        context.run_id.as_deref(),
                                        "flash",
                                        "flash",
                                        "info",
                                        "Image payload copied. Waiting for device flush/process exit (this can take a while on slower media).",
                                    );
                                } else if let Some(copy_complete_instant) = copy_complete_at {
                                    let elapsed_after_copy = copy_complete_instant.elapsed().as_secs();
                                    emit_updater_progress(
                                        &context.app,
                                        context.run_id.as_deref(),
                                        "flash",
                                        "flash",
                                        "info",
                                        format!(
                                            "Still finalizing device flush ({elapsed_after_copy}s since payload copy completed)."
                                        ),
                                    );
                                }
                            }
                        } else {
                            emit_updater_progress(
                                &context.app,
                                context.run_id.as_deref(),
                                "flash",
                                "flash",
                                "info",
                                format!(
                                    "Flash process still running ({:.0}s elapsed).",
                                    started_wait.elapsed().as_secs_f64()
                                ),
                            );
                        }
                        last_heartbeat = Instant::now();
                    }
                }
                thread::sleep(Duration::from_millis(200));
            }
            Err(error) => return Err(format!("Failed while waiting on dd: {error}")),
        }
    };

    if let Some(context) = progress_context {
        if copy_complete_logged {
            let finalize_elapsed = copy_complete_at
                .map(|instant| instant.elapsed().as_secs())
                .unwrap_or(0);
            emit_updater_progress(
                &context.app,
                context.run_id.as_deref(),
                "flash",
                "flash",
                if status.success() { "info" } else { "warn" },
                if status.success() {
                    format!(
                        "Device flush completed; flash process exited ({finalize_elapsed}s after payload copy)."
                    )
                } else {
                    format!(
                        "Flash process exited during finalization after {finalize_elapsed}s."
                    )
                },
            );
        }
    }

    let readers_wait_started = Instant::now();
    while readers_wait_started.elapsed() < Duration::from_secs(3) {
        if stdout_done.load(Ordering::Relaxed) && stderr_done.load(Ordering::Relaxed) {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }

    let stdout = stdout_bytes.lock().map(|bytes| bytes.clone()).unwrap_or_default();
    let mut stderr = stderr_bytes.lock().map(|bytes| bytes.clone()).unwrap_or_default();

    if is_update_cancel_requested() {
        stderr.extend_from_slice(b"\nOperation canceled by user.");
    } else if timed_out {
        stderr.extend_from_slice(
            b"\nFlash command timed out while waiting for final process exit after writing data.",
        );
    }

    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

fn extract_latest_dd_written_bytes(text: &str) -> Option<u64> {
    let mut offset = 0usize;
    let mut latest = None;
    while let Some(relative_index) = text[offset..].find(" bytes") {
        let index = offset + relative_index;
        if let Some(value) = parse_number_before_index(text, index) {
            latest = Some(value);
        }
        offset = index + 6;
    }
    latest
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn parse_number_before_index(text: &str, end_index: usize) -> Option<u64> {
    let bytes = text.as_bytes();
    if end_index == 0 || end_index > bytes.len() {
        return None;
    }

    let mut end = end_index;
    while end > 0 && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }

    let mut start = end;
    while start > 0 {
        let ch = bytes[start - 1];
        if ch.is_ascii_digit() || ch == b',' {
            start -= 1;
        } else {
            break;
        }
    }
    if start == end {
        return None;
    }

    let numeric = text[start..end]
        .chars()
        .filter(|ch| ch.is_ascii_digit())
        .collect::<String>();
    numeric.parse::<u64>().ok()
}

fn total_flash_bytes_for_image(image_path: &Path) -> Option<u64> {
    let file_name = image_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if file_name.ends_with(".xz") {
        return xz_uncompressed_size(image_path);
    }

    fs::metadata(image_path).ok().map(|metadata| metadata.len())
}

fn xz_uncompressed_size(image_path: &Path) -> Option<u64> {
    let xz_path = resolve_required_tool("xz").ok()?;
    let output = Command::new(xz_path)
        .args(["--robot", "--list"])
        .arg(image_path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut fallback = None;
    for line in stdout.lines() {
        let fields = line.split('\t').collect::<Vec<_>>();
        if fields.len() < 5 {
            continue;
        }
        if fields[0] == "totals" {
            return fields[4].parse::<u64>().ok();
        }
        if fields[0] == "file" {
            fallback = fields[4].parse::<u64>().ok();
        }
    }
    fallback
}
