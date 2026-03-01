use super::*;

#[derive(Debug, Deserialize)]
pub(crate) struct OtaUploadResponse {
    pub(crate) image_url: String,
    pub(crate) filename: Option<String>,
    pub(crate) size_bytes: Option<u64>,
    pub(crate) sha256: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OtaApplyResponse {
    pub(crate) update_id: Option<String>,
    pub(crate) message: Option<String>,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct OtaRuntimeState {
    pub(crate) update_id: Option<String>,
    pub(crate) stage: Option<String>,
    pub(crate) progress_percent: Option<f64>,
    pub(crate) last_error: Option<String>,
}

pub(crate) struct OtaUploadProgressReader {
    inner: fs::File,
    app: tauri::AppHandle,
    run_id: Option<String>,
    endpoint_label: String,
    bytes_total: Option<u64>,
    bytes_read: u64,
    last_percent_bucket: i32,
    last_emitted_bytes: u64,
}

impl OtaUploadProgressReader {
    pub(crate) fn new(
        inner: fs::File,
        app: tauri::AppHandle,
        run_id: Option<String>,
        endpoint_label: String,
        bytes_total: Option<u64>,
    ) -> Self {
        Self {
            inner,
            app,
            run_id,
            endpoint_label,
            bytes_total,
            bytes_read: 0,
            last_percent_bucket: -1,
            last_emitted_bytes: 0,
        }
    }

    fn emit_progress(&mut self, force: bool) {
        if let Some(total) = self.bytes_total.filter(|value| *value > 0) {
            let percent = ((self.bytes_read as f64 / total as f64) * 100.0).clamp(0.0, 100.0);
            let bucket = (percent / 2.0).floor() as i32;
            if !force && bucket <= self.last_percent_bucket {
                return;
            }
            self.last_percent_bucket = bucket;
            emit_ota_step_progress_with_bytes(
                &self.app,
                self.run_id.as_deref(),
                "ota-upload",
                "running",
                format!(
                    "Uploading OTA image to {}: {:.1}% ({}/{} bytes).",
                    self.endpoint_label, percent, self.bytes_read, total
                ),
                Some(percent),
                Some(self.bytes_read),
                Some(total),
            );
            return;
        }

        if !force && self.bytes_read.saturating_sub(self.last_emitted_bytes) < 8 * 1024 * 1024 {
            return;
        }
        self.last_emitted_bytes = self.bytes_read;
        emit_ota_step_progress_with_bytes(
            &self.app,
            self.run_id.as_deref(),
            "ota-upload",
            "running",
            format!(
                "Uploading OTA image to {}: {} bytes sent.",
                self.endpoint_label, self.bytes_read
            ),
            None,
            Some(self.bytes_read),
            None,
        );
    }
}

impl Read for OtaUploadProgressReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let bytes = self.inner.read(buf)?;
        if bytes > 0 {
            self.bytes_read = self.bytes_read.saturating_add(bytes as u64);
            self.emit_progress(false);
        } else {
            self.emit_progress(true);
        }
        Ok(bytes)
    }
}
