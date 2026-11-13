//! The first-flash manifest, made from the image, and the signature record.
//!
//! The build's only output is the first-flash `.img.xz`. When it is flashed, this
//! module reads it once and makes the `flash.json` transport the board's
//! `system.flash/1` takes: the board the image is for, the raw image's kind
//! (`disk`), the size and SHA-256 of the raw image, and, when the file carries one,
//! the signature record as hex.
//!
//! # The file
//!
//! One image stream, optionally followed by one signature record stream (the
//! `xzdec` crate reads and checks the layout). Stream 1 is the raw image; its size
//! and SHA-256 cover exactly its decoded bytes, and no reader decodes past it.
//!
//! # The signature record
//!
//! The record is 108 bytes: `LMNSSIG1` | algorithm (1 = Ed25519) | 3 zero bytes |
//! key id (32, SHA-256 of the public key) | signature (64). The board rebuilds the
//! signed message from its own board id, the kind, and the manifest's size and
//! SHA-256 ([`signed_message`]). Atlas does not verify the signature: it holds no
//! trust store. It reports signed or unsigned, and the key id, before anything is
//! pushed. The board's verdict comes back as a [`BoardOutcome`].
//!
//! # Hashes
//!
//! `image.sha256` is the SHA-256 of the raw image: every byte of stream 1's decoded
//! output (or every byte of a plain `.img`). The xz container's own checks protect
//! the compressed stream while it decodes. The transport's SHA-256 over the pushed
//! file is the sender's and is not part of this manifest.
//!
//! # Transport JSON
//!
//! [`FlashManifest::to_json`] writes the bytes in one fixed order with no spaces:
//! `{"schema":1,"board":..,"format":"disk","image":{"size":..,"sha256":..}}`, with
//! `"signature":"<216 hex>"` last when a record is present. The JSON is transport
//! only: the signed object is the binary message, never these bytes.
//!
//! # Memory
//!
//! The pass holds a 256 KiB read buffer, a 1 MiB spool buffer, the xz dictionary
//! (at most 8 MiB, the decoder's bound is dictionary + 256 KiB), the partition table
//! (at most 1 MiB), and the record (108 bytes). The data partition goes to the
//! scratch directory, not memory.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

use atlas_xzdec::Layout;
use thiserror::Error;

use crate::error::ImageError;
use crate::ext;
use crate::names;
use crate::route::{Mode, Output, Router};
use crate::scratch::Scratch;

/// The manifest's schema, as Lemnos takes it.
pub const SCHEMA: u64 = 1;
/// The signature record's length.
pub const RECORD_BYTES: usize = 108;
/// The record's magic.
pub const RECORD_MAGIC: &[u8; 8] = b"LMNSSIG1";
/// The domain tag at the start of the signed message (with its NUL).
// PROVISIONAL: Lemnos W1 contract 2026-11-13, test vectors pending. The record and the
// signed message follow draft 2 of W1-actions.md; Lemnos's implementation confirms them.
pub const SIGNED_TAG: &[u8; 19] = b"lemnos-first-flash\0";
/// The signed message's version.
pub const SIGNED_VERSION: u32 = 1;
/// The most slot manifests read from the image.
const MAX_SLOTS: usize = 256;
/// The largest slot manifest read.
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
/// The slot manifest schema this reader takes (D13).
const SLOT_SCHEMA: u64 = 3;

/// The format of the file as sent: `xz` or `raw`. Transport, never in the manifest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Xz,
    Raw,
}

impl Format {
    /// The name the sender uses for it.
    pub fn as_str(self) -> &'static str {
        match self {
            Format::Xz => "xz",
            Format::Raw => "raw",
        }
    }
}

/// The kind of the raw image as the target sees it. Compression is never a kind.
/// NAND kinds come with W1b; an unknown kind is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageKind {
    /// A whole-disk block image.
    Disk,
}

impl ImageKind {
    /// The string in the manifest and in the signed message.
    pub fn as_str(self) -> &'static str {
        match self {
            ImageKind::Disk => "disk",
        }
    }

    /// The kind a string names. Any other string is refused, never guessed.
    pub fn parse(text: &str) -> Result<ImageKind, FlashError> {
        match text {
            "disk" => Ok(ImageKind::Disk),
            other => Err(FlashError::Record(format!("unknown image kind {other:?}"))),
        }
    }
}

/// The board an image is for, and the slots that name it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BoardInfo {
    pub(crate) board: String,
    pub(crate) slots: Vec<String>,
}

/// What a probe found: the board, its slots, the format, and the bytes the pass
/// read from the file (the layout's footers and indexes, and the decoded prefix).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoardProbe {
    pub board: String,
    pub slots: Vec<String>,
    pub format: Format,
    pub read: u64,
}

/// The signature algorithm. Only Ed25519 is defined.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Algorithm {
    Ed25519,
}

impl Algorithm {
    fn code(self) -> u8 {
        match self {
            Algorithm::Ed25519 => 1,
        }
    }
}

/// The signature record: the 108 bytes the file's second stream decodes to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub algorithm: Algorithm,
    /// SHA-256 of the public key.
    pub key_id: [u8; 32],
    /// Ed25519 over the signed message.
    pub signature: [u8; 64],
}

impl Record {
    /// Parses 108 bytes. A length other than 108 is a second stream that is not one
    /// record; a bad magic, algorithm or padding is a malformed record.
    pub fn parse(bytes: &[u8]) -> Result<Record, FlashError> {
        if bytes.len() != RECORD_BYTES {
            return Err(FlashError::NotFirstFlash(format!(
                "the second stream decodes to {} bytes, not one {RECORD_BYTES}-byte signature record",
                bytes.len()
            )));
        }
        if bytes[..8] != RECORD_MAGIC[..] {
            return Err(FlashError::Record("the magic is not LMNSSIG1".into()));
        }
        let algorithm = match bytes[8] {
            1 => Algorithm::Ed25519,
            other => {
                return Err(FlashError::Record(format!(
                    "algorithm {other} is not one this reader knows (1 = Ed25519)"
                )));
            }
        };
        if bytes[9..12] != [0, 0, 0] {
            return Err(FlashError::Record(
                "the three bytes after the algorithm are not zero".into(),
            ));
        }
        let mut key_id = [0u8; 32];
        key_id.copy_from_slice(&bytes[12..44]);
        let mut signature = [0u8; 64];
        signature.copy_from_slice(&bytes[44..108]);
        Ok(Record {
            algorithm,
            key_id,
            signature,
        })
    }

    /// The 108 bytes of the record.
    pub fn to_bytes(&self) -> [u8; RECORD_BYTES] {
        let mut out = [0u8; RECORD_BYTES];
        out[..8].copy_from_slice(RECORD_MAGIC);
        out[8] = self.algorithm.code();
        out[12..44].copy_from_slice(&self.key_id);
        out[44..108].copy_from_slice(&self.signature);
        out
    }

    /// The key id as lowercase hex.
    pub fn key_id_hex(&self) -> String {
        hex(&self.key_id)
    }
}

/// What is known about an image's signature, before anything is pushed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignatureState {
    /// No record: the file is the image stream alone.
    Unsigned,
    /// A record is present. Atlas does not verify it; the board does.
    Signed { key_id: [u8; 32], record: Record },
}

impl SignatureState {
    /// The key id, as lowercase hex, when signed.
    pub fn key_id_hex(&self) -> Option<String> {
        match self {
            SignatureState::Unsigned => None,
            SignatureState::Signed { key_id, .. } => Some(hex(key_id)),
        }
    }
}

/// The board's verdict on the signature, as its job reports it. The wire values are
/// the four in the Lemnos table (W1 and I5g share it); any other value is kept as
/// `Unrecognised` with its raw text, never mapped to one of the four.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BoardOutcome {
    /// `verified`: the signature verified with a key on the board's list.
    Verified,
    /// `not_checked_unsigned`: unsigned, accepted with the security switch off.
    NotCheckedUnsigned,
    /// `not_checked_unknown_key`: signed by a key the board does not know, accepted with the switch off.
    NotCheckedUnknownKey,
    /// `refused`: refused before any write (or the job failed); the reason is the board's.
    Refused,
    /// A value this reader does not know, kept as the board sent it.
    Unrecognised(String),
}

impl BoardOutcome {
    /// Parses a wire value. Anything but the four values is `Unrecognised`.
    pub fn parse(wire: &str) -> BoardOutcome {
        match wire {
            "verified" => BoardOutcome::Verified,
            "not_checked_unsigned" => BoardOutcome::NotCheckedUnsigned,
            "not_checked_unknown_key" => BoardOutcome::NotCheckedUnknownKey,
            "refused" => BoardOutcome::Refused,
            other => BoardOutcome::Unrecognised(other.to_string()),
        }
    }

    /// The wire value.
    pub fn as_wire(&self) -> &str {
        match self {
            BoardOutcome::Verified => "verified",
            BoardOutcome::NotCheckedUnsigned => "not_checked_unsigned",
            BoardOutcome::NotCheckedUnknownKey => "not_checked_unknown_key",
            BoardOutcome::Refused => "refused",
            BoardOutcome::Unrecognised(raw) => raw,
        }
    }

    /// The job result's code for the three accepted outcomes (0, 1, 2). `refused`
    /// has none (it is refused before any write), and an unrecognised value has none.
    pub fn code(&self) -> Option<u8> {
        match self {
            BoardOutcome::Verified => Some(0),
            BoardOutcome::NotCheckedUnsigned => Some(1),
            BoardOutcome::NotCheckedUnknownKey => Some(2),
            BoardOutcome::Refused | BoardOutcome::Unrecognised(_) => None,
        }
    }

    /// The fixed reason for the three accepted outcomes, in plain words. A refusal's
    /// reason comes from the board with the verdict, not from here.
    pub fn reason(&self) -> Option<&'static str> {
        match self {
            BoardOutcome::Verified => Some("signature verified with a key on this board's list"),
            BoardOutcome::NotCheckedUnsigned => Some("unsigned image; signature not checked"),
            BoardOutcome::NotCheckedUnknownKey => Some("signed by an unknown key, not checked"),
            BoardOutcome::Refused | BoardOutcome::Unrecognised(_) => None,
        }
    }
}

// PROVISIONAL: Lemnos W1 contract 2026-11-13, test vectors pending (the message layout).
/// The message the signature covers, rebuilt the way the board rebuilds it:
/// the domain tag, a u32 version, the board id, the kind, the raw size as a u64,
/// and the raw SHA-256, with lengths as u32 little-endian.
pub fn signed_message(
    board: &str,
    kind: ImageKind,
    raw_size: u64,
    raw_sha256: &[u8; 32],
) -> Vec<u8> {
    let mut out = Vec::with_capacity(96);
    out.extend_from_slice(SIGNED_TAG);
    out.extend_from_slice(&SIGNED_VERSION.to_le_bytes());
    push_field(&mut out, board.as_bytes());
    push_field(&mut out, kind.as_str().as_bytes());
    out.extend_from_slice(&raw_size.to_le_bytes());
    out.extend_from_slice(raw_sha256);
    out
}

fn push_field(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(bytes);
}

// PROVISIONAL: Lemnos W1 contract 2026-11-13, test vectors pending (the record location).
/// Every question about the signature goes through here. Reads the record stream
/// only (its footer and index, and its 108 bytes); the image stream is not decoded.
pub fn signature(image: &Path) -> Result<SignatureState, FlashError> {
    let mut file = Counted::open(image)?;
    let layout = file_layout(&mut file)?;
    match layout.record {
        None => Ok(SignatureState::Unsigned),
        Some(_) => {
            let record = read_record_of(&mut file, &layout)?;
            Ok(SignatureState::Signed {
                key_id: record.key_id,
                record,
            })
        }
    }
}

/// The record of a file with a layout that has one.
fn read_record_of(file: &mut Counted, layout: &Layout) -> Result<Record, FlashError> {
    let bytes = atlas_xzdec::read_record(file, layout)?;
    Record::parse(&bytes)
}

/// The layout of the file: xz is scanned, a plain file is the image itself.
fn file_layout(file: &mut Counted) -> Result<Layout, FlashError> {
    if atlas_xzdec::is_xz(file).map_err(|e| ImageError::Io(e.to_string()))? {
        Ok(atlas_xzdec::scan(file)?)
    } else {
        let len = file
            .seek(SeekFrom::End(0))
            .map_err(|e| ImageError::Io(e.to_string()))?;
        file.seek(SeekFrom::Start(0))
            .map_err(|e| ImageError::Io(e.to_string()))?;
        Ok(Layout::plain(len))
    }
}

/// The first-flash manifest: what `flash.json` carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlashManifest {
    board: String,
    image_size: u64,
    image_sha256: String,
}

impl FlashManifest {
    /// The board id the image is for.
    pub fn board(&self) -> &str {
        &self.board
    }

    /// The raw image's kind. Every block image is `disk`.
    pub fn kind(&self) -> ImageKind {
        ImageKind::Disk
    }

    /// The raw image's size in bytes.
    pub fn image_size(&self) -> u64 {
        self.image_size
    }

    /// The raw image's SHA-256, lowercase hex.
    pub fn image_sha256(&self) -> &str {
        &self.image_sha256
    }

    /// The transport JSON: fixed key order, no spaces, the record last when present.
    /// The board id is `[a-z0-9-]` and the digest is hex, so nothing is escaped.
    pub fn to_json(&self, record: Option<&Record>) -> Vec<u8> {
        let mut json = format!(
            "{{\"schema\":{SCHEMA},\"board\":\"{}\",\"format\":\"{}\",\"image\":{{\"size\":{},\"sha256\":\"{}\"}}",
            self.board,
            self.kind().as_str(),
            self.image_size,
            self.image_sha256
        );
        if let Some(record) = record {
            json.push_str(&format!(",\"signature\":\"{}\"", hex(&record.to_bytes())));
        }
        json.push('}');
        json.into_bytes()
    }
}

/// The first-flash manifest and what travels with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlashImage {
    pub manifest: FlashManifest,
    /// The transport JSON, exactly as [`FlashManifest::to_json`] makes it.
    pub json: Vec<u8>,
    /// What the file says about its signature.
    pub signature: SignatureState,
    /// The bytes read from the file.
    pub read: u64,
}

/// Why an image is not flashed to the board it was given to. Each message says in
/// plain words what was found; nothing is pushed in any of these.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum Refusal {
    #[error("this image has no slot, so it names no board. Nothing was pushed.")]
    NoSlot,
    #[error(
        "slot {slot:?} in this image has no board, so the image names none. Nothing was pushed."
    )]
    NoBoard { slot: String },
    #[error(
        "slot {slot:?} in this image names no valid board id (lowercase letters, digits and '-', at most 64). Nothing was pushed."
    )]
    BadBoard { slot: String },
    #[error(
        "slot {slot:?}'s manifest is schema {found}; this reader takes schema 3 only. Nothing was pushed."
    )]
    SlotSchema { slot: String, found: String },
    #[error(
        "slot {slot:?}'s manifest does not parse as JSON, so the image cannot be checked. Nothing was pushed."
    )]
    BadSlot { slot: String },
    #[error(
        "the slots in this image name two boards, {first:?} and {second:?}. An image is built for one board. Nothing was pushed."
    )]
    ConflictingBoards { first: String, second: String },
    #[error(
        "this image is for the board {image:?}, and the connected board is {target:?}. Nothing was pushed. Flash the image built for {target:?}, or connect {image:?}."
    )]
    WrongBoard { image: String, target: String },
    #[error(
        "the connected board's id {target:?} is not a board id (lowercase letters, digits and '-', at most 64)"
    )]
    BadTarget { target: String },
}

/// Why an image could not be read for flashing.
#[derive(Debug, Error)]
pub enum FlashError {
    /// The file is not one this reader takes: damaged, truncated, or over a limit.
    #[error(transparent)]
    Image(#[from] ImageError),
    /// The file is for another board, or names none.
    #[error(transparent)]
    Refused(#[from] Refusal),
    /// The second stream is not one signature record, or there is a third stream.
    #[error("not a first-flash image: {0}")]
    NotFirstFlash(String),
    /// The signature record is malformed.
    #[error("malformed signature record: {0}")]
    Record(String),
}

impl From<atlas_xzdec::XzError> for FlashError {
    fn from(error: atlas_xzdec::XzError) -> Self {
        FlashError::Image(ImageError::from(error))
    }
}

impl FlashError {
    /// The same failure as an [`ImageError`], for the slot reader.
    pub(crate) fn into_image(self) -> ImageError {
        match self {
            FlashError::Image(error) => error,
            FlashError::Refused(refusal) => ImageError::Layout(refusal.to_string()),
            FlashError::NotFirstFlash(text) => {
                ImageError::Layout(format!("not a first-flash image: {text}"))
            }
            FlashError::Record(text) => {
                ImageError::Malformed(format!("malformed signature record: {text}"))
            }
        }
    }
}

/// A board id: 1 to 64 of `[a-z0-9-]`.
pub fn valid_board_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Checks the image's board against the connected one. A board id that is not
/// valid is refused as itself, so a typo is named as one.
pub fn check_target(manifest: &FlashManifest, target: &str) -> Result<(), Refusal> {
    if !valid_board_id(target) {
        return Err(Refusal::BadTarget {
            target: target.to_string(),
        });
    }
    if manifest.board != target {
        return Err(Refusal::WrongBoard {
            image: manifest.board.clone(),
            target: target.to_string(),
        });
    }
    Ok(())
}

/// A file opened for reading that counts the bytes read from it.
pub(crate) struct Counted {
    file: File,
    read: u64,
}

impl Counted {
    pub(crate) fn open(path: &Path) -> Result<Self, ImageError> {
        let file = File::open(path).map_err(|error| ImageError::io("the image", &error))?;
        Ok(Self { file, read: 0 })
    }
}

impl Read for Counted {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let count = self.file.read(buf)?;
        self.read += count as u64;
        Ok(count)
    }
}

impl Seek for Counted {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.file.seek(pos)
    }
}

/// Reads the board from the slots of a spooled data partition. Every `slots/*.json`
/// is a slot manifest: it must parse, be schema 3, and name a valid board. They must
/// all name the same board. Nothing is skipped.
pub(crate) fn read_board(spool: &Path) -> Result<BoardInfo, FlashError> {
    let fs = ext::load(spool)?;
    let Some(files) = ext::list_files(&fs, "/slots", MAX_SLOTS * 4)? else {
        return Err(Refusal::NoSlot.into());
    };
    let mut slots = Vec::new();
    let mut board: Option<String> = None;
    for name in files {
        let Some(id) = name.strip_suffix(".json") else {
            continue;
        };
        names::check_id(id)?;
        let text = ext::read_text(&fs, &format!("/slots/{name}"), MAX_MANIFEST_BYTES)?;
        let value: serde_json::Value =
            serde_json::from_str(&text).map_err(|_| Refusal::BadSlot {
                slot: id.to_string(),
            })?;
        match value.get("schema") {
            Some(serde_json::Value::Number(number)) if number.as_u64() == Some(SLOT_SCHEMA) => {}
            other => {
                let found = match other {
                    None => "missing".to_string(),
                    Some(serde_json::Value::Number(number)) => number.to_string(),
                    Some(_) => "not a number".to_string(),
                };
                return Err(Refusal::SlotSchema {
                    slot: id.to_string(),
                    found,
                }
                .into());
            }
        }
        let this = match value.get("board") {
            None => return Err(Refusal::NoBoard { slot: id.into() }.into()),
            Some(serde_json::Value::String(text)) if valid_board_id(text) => text.clone(),
            Some(_) => return Err(Refusal::BadBoard { slot: id.into() }.into()),
        };
        match &board {
            None => board = Some(this),
            Some(first) if *first == this => {}
            Some(first) => {
                return Err(Refusal::ConflictingBoards {
                    first: first.clone(),
                    second: this,
                }
                .into());
            }
        }
        slots.push(id.to_string());
    }
    let Some(board) = board else {
        return Err(Refusal::NoSlot.into());
    };
    Ok(BoardInfo { board, slots })
}

/// One pass over the file: its layout and record are read first (footers, indexes
/// and 108 bytes), then the image stream is decoded through the router.
struct Pass {
    output: Output,
    format: Format,
    read: u64,
    record: Option<Record>,
}

fn run(image: &Path, scratch_parent: &Path, mode: Mode) -> Result<Pass, FlashError> {
    let options = crate::Options::default();
    let scratch = Scratch::create(scratch_parent)?;
    let mut file = Counted::open(image)?;
    let format = if atlas_xzdec::is_xz(&mut file).map_err(|e| ImageError::Io(e.to_string()))? {
        Format::Xz
    } else {
        Format::Raw
    };
    let layout = file_layout(&mut file)?;
    let record = match layout.record {
        Some(_) => Some(read_record_of(&mut file, &layout)?),
        None => None,
    };
    let mut router = Router::new(scratch.path(), &options, mode);
    let stream = atlas_xzdec::decode_image(&mut file, &layout, &mut router)
        .map_err(|error| error.to_string());
    let output = router.complete(stream)?;
    Ok(Pass {
        output,
        format,
        read: file.read,
        record,
    })
}

/// Reads the board of the image at `image` and stops. The pass ends as soon as the
/// data partition is read, so a probe of a large image costs only the stream up to
/// the data partition (when the data partition is last, that is the whole stream).
pub fn probe_board(image: &Path, scratch_parent: &Path) -> Result<BoardProbe, FlashError> {
    let pass = run(
        image,
        scratch_parent,
        Mode::Flash {
            target: None,
            stop_after_board: true,
        },
    )?;
    let info = pass
        .output
        .board
        .ok_or_else(|| ImageError::Layout("the board was not read".into()))?;
    Ok(BoardProbe {
        board: info.board,
        slots: info.slots,
        format: pass.format,
        read: pass.read,
    })
}

/// Makes the first-flash manifest of the image at `image`. With `target` set, an
/// image for another board is refused at the data partition's end, before the rest
/// of the stream is decoded.
pub fn flash_manifest(
    image: &Path,
    scratch_parent: &Path,
    target: Option<&str>,
) -> Result<FlashImage, FlashError> {
    if let Some(target) = target.filter(|t| !valid_board_id(t)) {
        return Err(Refusal::BadTarget {
            target: target.to_string(),
        }
        .into());
    }
    let pass = run(
        image,
        scratch_parent,
        Mode::Flash {
            target: target.map(str::to_string),
            stop_after_board: false,
        },
    )?;
    let (Some(info), Some(raw)) = (pass.output.board, pass.output.raw) else {
        return Err(
            ImageError::Layout("the pass did not reach the end of the image".into()).into(),
        );
    };
    let manifest = FlashManifest {
        board: info.board,
        image_size: raw.len,
        image_sha256: hex(&raw.sha256),
    };
    let json = manifest.to_json(pass.record.as_ref());
    let signature = match pass.record {
        Some(record) => SignatureState::Signed {
            key_id: record.key_id,
            record,
        },
        None => SignatureState::Unsigned,
    };
    Ok(FlashImage {
        manifest,
        json,
        signature,
        read: pass.read,
    })
}

/// Lowercase hex.
pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
