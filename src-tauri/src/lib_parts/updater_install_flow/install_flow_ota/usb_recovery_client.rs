use super::*;

#[derive(Debug, Deserialize)]
pub(crate) struct UsbRecoveryResponseError {
    pub(crate) code: String,
    pub(crate) message: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UsbRecoveryResponse {
    pub(crate) id: Value,
    pub(crate) ok: bool,
    pub(crate) result: Option<Value>,
    pub(crate) error: Option<UsbRecoveryResponseError>,
}

pub(crate) struct UsbRecoveryClient {
    port: fs::File,
    port_path: PathBuf,
    timeout: Duration,
    next_id: u64,
}

pub(crate) enum UsbRecoveryOpenError {
    PermissionDenied(String),
    Other(String),
}

impl UsbRecoveryClient {
    pub(crate) fn open(path: &Path, timeout: Duration) -> Result<Self, UsbRecoveryOpenError> {
        use std::os::unix::fs::OpenOptionsExt;

        let port = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)
            .map_err(|error| {
                if error.kind() == io::ErrorKind::PermissionDenied {
                    UsbRecoveryOpenError::PermissionDenied(format!(
                        "Unable to open USB recovery serial endpoint {}: {error}. \
Linux denied access to the serial device. Add your user to the `dialout` group and sign out/in \
(`sudo usermod -aG dialout $USER`), or run Atlas with elevated privileges.",
                        path.display()
                    ))
                } else {
                    UsbRecoveryOpenError::Other(format!(
                        "Unable to open USB recovery serial endpoint {}: {error}",
                        path.display()
                    ))
                }
            })?;

        Ok(Self {
            port,
            port_path: path.to_path_buf(),
            timeout,
            next_id: 1,
        })
    }

    pub(crate) fn port_label(&self) -> String {
        self.port_path.display().to_string()
    }

    pub(crate) fn call(&mut self, op: &str, params: Value) -> Result<Value, String> {
        let request_id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        let request = serde_json::json!({
            "id": request_id,
            "op": op,
            "params": params,
        });
        let request_text = serde_json::to_string(&request)
            .map_err(|error| format!("Failed to encode USB OTA request `{op}`: {error}"))?;
        self.port
            .write_all(request_text.as_bytes())
            .map_err(|error| format!("Failed to write USB OTA request `{op}`: {error}"))?;
        self.port
            .write_all(b"\n")
            .map_err(|error| format!("Failed to finish USB OTA request `{op}`: {error}"))?;
        self.port
            .flush()
            .map_err(|error| format!("Failed to flush USB OTA request `{op}`: {error}"))?;

        let started = Instant::now();
        let mut read_buf = [0u8; 4096];
        let mut buffer = Vec::<u8>::new();
        while started.elapsed() < self.timeout {
            match self.port.read(&mut read_buf) {
                Ok(0) => {}
                Ok(bytes_read) => {
                    buffer.extend_from_slice(&read_buf[..bytes_read]);
                    if let Some(newline_idx) = buffer.iter().position(|byte| *byte == b'\n') {
                        let mut line = buffer.drain(..=newline_idx).collect::<Vec<u8>>();
                        while matches!(line.last(), Some(b'\n' | b'\r')) {
                            let _ = line.pop();
                        }
                        if line.is_empty() {
                            continue;
                        }

                        let response = serde_json::from_slice::<UsbRecoveryResponse>(&line)
                            .map_err(|error| {
                                format!("Failed parsing USB OTA response for `{op}` as JSON: {error}")
                            })?;
                        if !usb_recovery_response_id_matches(Some(&response.id), request_id) {
                            continue;
                        }

                        if response.ok {
                            return response.result.ok_or_else(|| {
                                format!("USB OTA response for `{op}` did not include a result.")
                            });
                        }
                        let error = response.error.ok_or_else(|| {
                            format!(
                                "USB OTA request `{op}` failed but did not include structured error details."
                            )
                        })?;
                        return Err(format!(
                            "USB OTA request `{op}` failed ({}): {}",
                            error.code, error.message
                        ));
                    }
                }
                Err(error)
                    if error.kind() == io::ErrorKind::WouldBlock
                        || error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => {
                    return Err(format!("Failed reading USB OTA response for `{op}`: {error}"));
                }
            }
            if buffer.len() > 4 * 1024 * 1024 {
                return Err("USB OTA response exceeded 4 MiB without newline termination.".to_string());
            }
            thread::sleep(Duration::from_millis(5));
        }
        Err(format!(
            "Timed out waiting for USB OTA response to `{op}` after {}s.",
            self.timeout.as_secs()
        ))
    }
}
