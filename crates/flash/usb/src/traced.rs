//! [`Traced`] logs and records every transfer of another [`UsbIo`]. It
//! changes no transfer: the device sees exactly what the loader sent, and the
//! loader gets exactly what the device answered.

use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;

use crate::record::{Record, Transfer, preview};
use crate::{UsbError, UsbIo};

/// `tracing` target of every transfer line.
pub const LOG_TARGET: &str = "atlas_usb::transfer";

/// Receives each transfer line, without a newline.
pub type Sink = Arc<dyn Fn(&str) + Send + Sync>;

/// Collects the [`Record`]s of one or more traced devices.
#[derive(Clone, Debug, Default)]
pub struct Recorder {
    records: Arc<Mutex<Vec<Record>>>,
}

impl Recorder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Every record so far, in order.
    pub fn records(&self) -> Vec<Record> {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn push(&self, record: Record) {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(record);
    }
}

/// One device's transfers, traced under `label`.
pub struct Traced {
    inner: Box<dyn UsbIo>,
    label: String,
    sink: Option<Sink>,
    recorder: Option<Recorder>,
}

impl fmt::Debug for Traced {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Traced")
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

impl Traced {
    pub fn new(inner: Box<dyn UsbIo>, label: impl Into<String>) -> Self {
        Self {
            inner,
            label: label.into(),
            sink: None,
            recorder: None,
        }
    }

    /// Also passes each line to `sink`, such as a flash job's log.
    pub fn with_sink(mut self, sink: Sink) -> Self {
        self.sink = Some(sink);
        self
    }

    /// Also keeps each transfer and its reply in `recorder`.
    pub fn recording(mut self, recorder: Recorder) -> Self {
        self.recorder = Some(recorder);
        self
    }

    fn finish(&self, transfer: Transfer, reply: Result<Vec<u8>, UsbError>) {
        let outcome = match &reply {
            Ok(bytes) if bytes.is_empty() => "ok".to_string(),
            Ok(bytes) => format!("ok {}", preview(bytes)),
            Err(error) => format!("failed: {error}"),
        };
        let line = format!("{} {} -> {outcome}", self.label, transfer.describe());
        if reply.is_ok() {
            tracing::info!(target: LOG_TARGET, "{line}");
        } else {
            tracing::warn!(target: LOG_TARGET, "{line}");
        }
        if let Some(sink) = &self.sink {
            sink(&line);
        }
        if let Some(recorder) = &self.recorder {
            recorder.push(Record { transfer, reply });
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
        let transfer = Transfer::ControlOut {
            request,
            value,
            index,
            data: data.to_vec(),
        };
        let result = self.inner.control_out(request, value, index, data).await;
        self.finish(transfer, result.clone().map(|()| Vec::new()));
        result
    }

    async fn control_in(
        &mut self,
        request: u8,
        value: u16,
        index: u16,
        length: u16,
    ) -> Result<Vec<u8>, UsbError> {
        let transfer = Transfer::ControlIn {
            request,
            value,
            index,
            length,
        };
        let result = self.inner.control_in(request, value, index, length).await;
        self.finish(transfer, result.clone());
        result
    }

    async fn bulk_out(&mut self, data: &[u8]) -> Result<(), UsbError> {
        let result = self.inner.bulk_out(data).await;
        self.finish(
            Transfer::BulkOut(data.to_vec()),
            result.clone().map(|()| Vec::new()),
        );
        result
    }

    async fn bulk_in(&mut self, max: usize) -> Result<Vec<u8>, UsbError> {
        let result = self.inner.bulk_in(max).await;
        self.finish(Transfer::BulkIn { max }, result.clone());
        result
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::sim::{Answer, SimBus};
    use crate::{DeviceSummary, OpenOptions, UsbBus};

    #[tokio::test]
    async fn tracing_changes_no_transfer_and_reaches_the_sink() {
        let bus = SimBus::new();
        bus.attach(DeviceSummary::new("1-1", 1, 2));
        bus.script(
            "1-1",
            [Answer::Ok, Answer::Data(vec![9, 9, 9]), Answer::Stall],
        );
        let device = bus.open("1-1", &OpenOptions::default()).await.unwrap();

        let lines = Arc::new(Mutex::new(Vec::<String>::new()));
        let kept = Arc::clone(&lines);
        let sink: Sink = Arc::new(move |line: &str| {
            kept.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(line.to_string());
        });
        let recorder = Recorder::new();
        let mut traced = Traced::new(device, "loader")
            .with_sink(sink)
            .recording(recorder.clone());

        traced.bulk_out(&[1, 2, 3]).await.unwrap();
        assert_eq!(traced.bulk_in(2).await.unwrap(), vec![9, 9]);
        assert_eq!(traced.control_in(0x10, 0, 0, 4).await, Err(UsbError::Stall));

        let records = recorder.records();
        assert_eq!(records.len(), 3);
        assert_eq!(records[0].transfer, Transfer::BulkOut(vec![1, 2, 3]));
        assert_eq!(records[1].reply, Ok(vec![9, 9]));
        assert_eq!(records[2].reply, Err(UsbError::Stall));

        let lines = lines.lock().unwrap_or_else(PoisonError::into_inner);
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("loader bulk out: 010203"));
        assert!(lines[1].ends_with("-> ok 0909 (2 bytes)"));
        assert!(lines[2].ends_with("-> failed: the device stalled the transfer"));
    }
}
