//! [`Traced`] logs every transfer through another [`UsbIo`]: the request and
//! the reply in hex, one line each, so a board's first run shows exactly what
//! it was sent and what it answered. The lines go to `tracing` (target
//! [`LOG_TARGET`]) and, when given one, to a sink such as a flash job's log.
//! It changes no transfer.

use std::sync::Arc;

use async_trait::async_trait;

use crate::transport::{UsbError, UsbIo};

/// `tracing` target of every transfer [`Traced`] logs.
pub const LOG_TARGET: &str = "atlas_socflash::usb";

/// Receives each line (without a newline).
pub type Sink = Arc<dyn Fn(&str) + Send + Sync>;

/// The first 64 bytes in hex, with the total length.
pub fn preview(bytes: &[u8]) -> String {
    let head: String = bytes
        .iter()
        .take(64)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if bytes.len() > 64 {
        format!("{head}... ({} bytes)", bytes.len())
    } else {
        format!("{head} ({} bytes)", bytes.len())
    }
}

/// Logs each transfer of `inner` as `label` names the device.
pub struct Traced {
    inner: Box<dyn UsbIo>,
    label: String,
    sink: Option<Sink>,
}

impl Traced {
    pub fn new(inner: Box<dyn UsbIo>, label: &str) -> Self {
        Self {
            inner,
            label: label.to_string(),
            sink: None,
        }
    }

    /// Also passes each line to `sink`.
    pub fn with_sink(mut self, sink: Sink) -> Self {
        self.sink = Some(sink);
        self
    }

    fn say(&self, failed: bool, text: String) {
        let line = format!("{} {text}", self.label);
        if failed {
            tracing::warn!(target: LOG_TARGET, "{line}");
        } else {
            tracing::info!(target: LOG_TARGET, "{line}");
        }
        if let Some(sink) = &self.sink {
            sink(&line);
        }
    }

    fn outcome<T>(
        &self,
        what: &str,
        result: &Result<T, UsbError>,
        reply: impl FnOnce(&T) -> String,
    ) {
        match result {
            Ok(value) => self.say(false, format!("{what}: ok{}", reply(value))),
            Err(error) => self.say(true, format!("{what}: failed: {error}")),
        }
    }
}

#[async_trait]
impl UsbIo for Traced {
    fn max_packet_out(&self) -> usize {
        self.inner.max_packet_out()
    }

    async fn control_out(
        &mut self,
        request: u8,
        value: u16,
        index: u16,
        data: &[u8],
    ) -> Result<(), UsbError> {
        self.say(
            false,
            format!(
                "control out request {request:#04x} value {value:#06x} index {index:#06x}: {}",
                preview(data)
            ),
        );
        let result = self.inner.control_out(request, value, index, data).await;
        self.outcome("control out", &result, |()| String::new());
        result
    }

    async fn control_in(
        &mut self,
        request: u8,
        value: u16,
        index: u16,
        length: u16,
    ) -> Result<Vec<u8>, UsbError> {
        self.say(
            false,
            format!(
                "control in request {request:#04x} value {value:#06x} index {index:#06x} length {length}"
            ),
        );
        let result = self.inner.control_in(request, value, index, length).await;
        self.outcome("control in", &result, |reply| {
            format!(" {}", preview(reply))
        });
        result
    }

    async fn bulk_out(&mut self, data: &[u8]) -> Result<(), UsbError> {
        self.say(false, format!("bulk out: {}", preview(data)));
        let result = self.inner.bulk_out(data).await;
        self.outcome("bulk out", &result, |()| String::new());
        result
    }

    async fn bulk_in(&mut self, max: usize) -> Result<Vec<u8>, UsbError> {
        self.say(false, format!("bulk in, up to {max} bytes"));
        let result = self.inner.bulk_in(max).await;
        self.outcome("bulk in", &result, |reply| format!(" {}", preview(reply)));
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::{Script, Step};
    use std::sync::Mutex;

    #[test]
    fn previews_cut_long_payloads_and_keep_the_length() {
        assert_eq!(preview(&[0xab, 0x01]), "ab01 (2 bytes)");
        let long = vec![0x11u8; 100];
        let text = preview(&long);
        assert!(text.starts_with(&"11".repeat(64)));
        assert!(text.ends_with("... (100 bytes)"));
    }

    #[tokio::test]
    async fn wrapping_changes_no_transfer_and_reaches_the_sink() {
        let script = Script::new(vec![
            Step::BulkOut(vec![1, 2, 3]),
            Step::BulkIn {
                max: 4,
                reply: vec![9, 9],
            },
        ]);
        let lines = Arc::new(Mutex::new(Vec::<String>::new()));
        let kept = Arc::clone(&lines);
        let sink: Sink = Arc::new(move |line: &str| {
            if let Ok(mut lines) = kept.lock() {
                lines.push(line.to_string());
            }
        });
        let mut traced = Traced::new(Box::new(script), "test").with_sink(sink);
        traced.bulk_out(&[1, 2, 3]).await.unwrap();
        assert_eq!(traced.bulk_in(4).await.unwrap(), vec![9, 9]);
        let lines = lines.lock().unwrap();
        assert_eq!(lines.len(), 4);
        assert!(lines[0].starts_with("test bulk out: 010203"));
        assert!(lines[3].ends_with("0909 (2 bytes)"));
    }
}
