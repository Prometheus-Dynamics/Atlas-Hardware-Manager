use crate::UsbError;

/// One transfer as a loader asked for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Transfer {
    ControlOut {
        request: u8,
        value: u16,
        index: u16,
        data: Vec<u8>,
    },
    ControlIn {
        request: u8,
        value: u16,
        index: u16,
        length: u16,
    },
    BulkOut(Vec<u8>),
    BulkIn {
        max: usize,
    },
}

impl Transfer {
    /// The request, with the first 64 bytes of any payload in hex.
    pub fn describe(&self) -> String {
        match self {
            Self::ControlOut {
                request,
                value,
                index,
                data,
            } => format!(
                "control out request {request:#04x} value {value:#06x} index {index:#06x}: {}",
                preview(data)
            ),
            Self::ControlIn {
                request,
                value,
                index,
                length,
            } => format!(
                "control in request {request:#04x} value {value:#06x} index {index:#06x} length {length}"
            ),
            Self::BulkOut(data) => format!("bulk out: {}", preview(data)),
            Self::BulkIn { max } => format!("bulk in, up to {max} bytes"),
        }
    }

    /// What this transfer is, for a timeout message.
    pub fn what(&self) -> &'static str {
        match self {
            Self::ControlOut { .. } | Self::ControlIn { .. } => "a control transfer",
            Self::BulkOut(_) => "a bulk write",
            Self::BulkIn { .. } => "a bulk read",
        }
    }
}

/// A transfer and what came back: the bytes read, or the error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub transfer: Transfer,
    pub reply: Result<Vec<u8>, UsbError>,
}

/// The first 64 bytes in hex, with the total length.
pub(crate) fn preview(bytes: &[u8]) -> String {
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

#[cfg(test)]
mod tests {
    use super::preview;

    #[test]
    fn previews_cut_long_payloads_and_keep_the_length() {
        assert_eq!(preview(&[0xab, 0x01]), "ab01 (2 bytes)");
        let long = vec![0x11u8; 100];
        let text = preview(&long);
        assert!(text.starts_with(&"11".repeat(64)));
        assert!(text.ends_with("... (100 bytes)"));
    }
}
