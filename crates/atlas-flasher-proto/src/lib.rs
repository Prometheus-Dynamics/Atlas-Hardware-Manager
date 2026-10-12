//! The raze-flasher protocol (docs/flasher.md, section 3), version 1.
//!
//! The link is a byte stream each way over a vendor bulk interface. Every
//! frame is an 8-byte header, `kind` (`J` JSON, `D` data), three zero bytes
//! and a little-endian `u32` length, then that many bytes. JSON frames carry
//! one [`Request`] or [`Reply`]; data frames carry the image after a
//! `write`, and an empty data frame ends it.
//!
//! USB has no message boundaries of its own: a transfer ends at a short
//! packet. So the sender ends every message with one: Atlas with a short or
//! zero-length packet, and the daemon (whose FunctionFS writes cannot add a
//! ZLP) by padding its JSON with a space so no reply is a whole number of
//! packets ([`json_frame`]).

use serde::{Deserialize, Serialize};

pub const PROTOCOL: u32 = 1;

/// The flasher's USB identity: as the Raze's own gadget, told apart by its
/// product string. Its serial is the board serial the boot ROM reported.
pub const VENDOR_ID: u16 = 0x1d6b;
pub const PRODUCT_ID: u16 = 0x0104;
pub const PRODUCT: &str = "Raze flasher";
/// The vendor interface's string, and its class.
pub const INTERFACE: &str = "atlas-flasher";
pub const INTERFACE_CLASS: u8 = 0xff;

pub const HEADER_LEN: usize = 8;
/// The largest frame either side sends or accepts.
pub const MAX_FRAME: usize = 1 << 20;
/// Full-speed bulk packets; high-speed's 512 is a multiple of it.
const PACKET: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Json,
    Data,
}

impl Kind {
    fn byte(self) -> u8 {
        match self {
            Self::Json => b'J',
            Self::Data => b'D',
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub kind: Kind,
    pub payload: Vec<u8>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum FrameError {
    UnknownKind(u8),
    TooLong(usize),
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownKind(kind) => write!(f, "unknown frame kind 0x{kind:02x}"),
            Self::TooLong(len) => {
                write!(f, "a {len}-byte frame is over the {MAX_FRAME}-byte limit")
            }
        }
    }
}

impl std::error::Error for FrameError {}

pub fn header(kind: Kind, len: usize) -> [u8; HEADER_LEN] {
    let mut header = [kind.byte(), 0, 0, 0, 0, 0, 0, 0];
    header[4..].copy_from_slice(&(len as u32).to_le_bytes());
    header
}

/// A data frame (header and bytes). An empty one ends a stream.
pub fn data_frame(bytes: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(HEADER_LEN + bytes.len());
    frame.extend_from_slice(&header(Kind::Data, bytes.len()));
    frame.extend_from_slice(bytes);
    frame
}

/// A JSON frame, padded with a space when it would otherwise end on a
/// packet boundary (see the module docs).
pub fn json_frame<T: Serialize>(message: &T) -> Vec<u8> {
    let mut json = serde_json::to_vec(message).unwrap_or_else(|_| b"{}".to_vec());
    if (HEADER_LEN + json.len()).is_multiple_of(PACKET) {
        json.push(b' ');
    }
    let mut frame = Vec::with_capacity(HEADER_LEN + json.len());
    frame.extend_from_slice(&header(Kind::Json, json.len()));
    frame.extend_from_slice(&json);
    frame
}

/// Splits a byte stream back into frames, however it was chunked.
#[derive(Default)]
pub struct Decoder {
    buffer: Vec<u8>,
}

impl Decoder {
    pub fn push(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
    }

    /// The next whole frame, if one has arrived.
    pub fn next_frame(&mut self) -> Option<Result<Frame, FrameError>> {
        if self.buffer.len() < HEADER_LEN {
            return None;
        }
        let kind = match self.buffer[0] {
            b'J' => Kind::Json,
            b'D' => Kind::Data,
            other => return Some(Err(FrameError::UnknownKind(other))),
        };
        let len = u32::from_le_bytes([
            self.buffer[4],
            self.buffer[5],
            self.buffer[6],
            self.buffer[7],
        ]) as usize;
        if len > MAX_FRAME {
            return Some(Err(FrameError::TooLong(len)));
        }
        if self.buffer.len() < HEADER_LEN + len {
            return None;
        }
        let payload = self.buffer[HEADER_LEN..HEADER_LEN + len].to_vec();
        self.buffer.drain(..HEADER_LEN + len);
        Some(Ok(Frame { kind, payload }))
    }

    /// Forgets a partial frame, when the link starts over.
    pub fn reset(&mut self) {
        self.buffer.clear();
    }
}

/// How the image is sent: as it is stored (`zst`, `xz`: the board
/// decompresses) or raw bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Raw,
    Zst,
    Xz,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RebootInto {
    Emmc,
    UsbBoot,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum Request {
    Hello {
        protocol: u32,
        atlas: String,
    },
    Info,
    /// Then data frames, `size` bytes in all, and an empty one.
    Write {
        /// Bytes as sent (the compressed size for `zst`).
        size: u64,
        /// SHA-256 of the bytes as sent, hex, checked as they arrive.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sha256: Option<String>,
        format: Format,
        #[serde(default)]
        keep_data: bool,
    },
    /// Reads back what the last write wrote.
    Verify,
    Reboot {
        into: RebootInto,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Writing,
    Verifying,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Emmc {
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manfid: Option<String>,
    /// JEDEC wear estimates (`life_time`: type A and B, `pre_eol_info`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub life_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_eol_info: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Reply {
    Hello {
        protocol: u32,
        daemon: String,
        serial: String,
        model: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        revision: Option<String>,
    },
    Info {
        serial: String,
        emmc: Emmc,
    },
    /// The write is accepted: send the data.
    Ready,
    /// `received` of `of` bytes as sent; `written` image bytes on the eMMC
    /// (verifying: read back).
    Progress {
        phase: Phase,
        received: u64,
        of: u64,
        written: u64,
    },
    Written {
        bytes: u64,
        sha256: String,
    },
    Verified {
        ok: bool,
        sha256: String,
    },
    Ok,
    Error {
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_survive_any_chunking() {
        let mut stream = json_frame(&Request::Info);
        stream.extend(data_frame(b"hello"));
        stream.extend(data_frame(b""));
        for chunk in 1..stream.len() {
            let mut decoder = Decoder::default();
            let mut frames = Vec::new();
            for piece in stream.chunks(chunk) {
                decoder.push(piece);
                while let Some(frame) = decoder.next_frame() {
                    frames.push(frame.unwrap());
                }
            }
            assert_eq!(frames.len(), 3, "chunk {chunk}");
            assert_eq!(frames[1].payload, b"hello");
            assert!(frames[2].payload.is_empty());
            let request: Request = serde_json::from_slice(&frames[0].payload).unwrap();
            assert_eq!(request, Request::Info);
        }
    }

    #[test]
    fn replies_never_fill_whole_packets() {
        for len in 0..300 {
            let reply = Reply::Error {
                message: "x".repeat(len),
            };
            assert!(!json_frame(&reply).len().is_multiple_of(PACKET), "{len}");
        }
    }

    #[test]
    fn messages_read_as_the_design_writes_them() {
        let write: Request =
            serde_json::from_str(r#"{"op":"write","size":10,"format":"zst"}"#).unwrap();
        assert_eq!(
            write,
            Request::Write {
                size: 10,
                sha256: None,
                format: Format::Zst,
                keep_data: false
            }
        );
        let reboot = serde_json::to_string(&Request::Reboot {
            into: RebootInto::Emmc,
        })
        .unwrap();
        assert_eq!(reboot, r#"{"op":"reboot","into":"emmc"}"#);
    }
}
