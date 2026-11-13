//! The first-flash manifest, made from the image.
//!
//! The build's only output is the first-flash `.img.xz`. When it is flashed, this
//! module reads it once and makes the `flash.json` that Lemnos's `system.flash/1`
//! takes (its schema 1): the board the image is for, the format, and the size and
//! SHA-256 of the raw image. Nothing in the image is trusted: the board comes from
//! the slots' own `board` values, every length is checked, and the image is
//! refused before anything is pushed when it is for another board.
//!
//! Two entry points:
//!
//! - [`probe_board`] reads the board alone. It stops as soon as the data partition
//!   is read, so a wrong image is refused after the bytes that carry the slots, and
//!   the rest of the stream is never decoded.
//! - [`flash_manifest`] makes the manifest. It hashes the raw image, so it reads the
//!   whole stream; with a `target` it refuses a wrong board at the same early point
//!   as the probe.
//!
//! # Hashes and what they cover
//!
//! `image.sha256` is the SHA-256 of the **raw** image: every byte the xz stream
//! decodes to, or every byte of a plain `.img`. `image.size` is the number of those
//! bytes. The xz container's own check (CRC-64 in the builder's files) protects the
//! compressed stream while it decodes. The transport's SHA-256 over the pushed file
//! (the compressed bytes as sent) is the sender's and is not part of this manifest.
//!
//! # Serialisation
//!
//! [`FlashManifest::to_json`] writes the bytes exactly, in one fixed order with no
//! spaces: `{"schema":1,"board":..,"format":..,"image":{"size":..,"sha256":..}}`.
//! The same bytes are what a detached signature would cover, so the form is fixed.
//!
//! # Signature
//!
//! A detached signature file, `flash.sig`, beside the image, is carried along
//! untouched and reported by presence. Its length is checked (a detached Ed25519
//! signature is 64 bytes, as Lemnos takes it); its content is not verified here.
//! No key is read.
//!
//! # Memory
//!
//! The pass holds one 256 KiB read buffer, one 1 MiB spool buffer, the xz
//! dictionary (at most 8 MiB: [`crate::xzscan::MAX_DICT_BYTES`] is checked from the
//! stream's index before any decode), the partition table (at most 1 MiB), and one
//! LZMA2 chunk. The data partition goes to the scratch directory, not to memory.
//! Nothing is allocated per chunk of the stream.

use std::fs::{self, File};
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use thiserror::Error;

use crate::decode;
use crate::error::ImageError;
use crate::ext;
use crate::names;
use crate::route::{Mode, Output, Router};
use crate::scratch::Scratch;
use crate::xzscan::{self, MAX_DICT_BYTES};

/// The manifest's schema, as Lemnos takes it.
pub const SCHEMA: u64 = 1;
/// The detached signature's file name, beside the image.
pub const SIGNATURE_FILE: &str = "flash.sig";
/// A detached Ed25519 signature, as Lemnos takes it.
pub const SIGNATURE_BYTES: u64 = 64;
/// The most slot manifests read from the image.
const MAX_SLOTS: usize = 256;
/// The largest slot manifest read.
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
/// The read buffer on the image file.
const READ_BUFFER: usize = 256 * 1024;

/// The format of the stream as Lemnos takes it: `xz` or `raw`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Xz,
    Raw,
}

impl Format {
    /// The name in the manifest.
    pub fn as_str(self) -> &'static str {
        match self {
            Format::Xz => "xz",
            Format::Raw => "raw",
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
/// read from the image file (compressed bytes for xz, counting the index and
/// headers read by seeking).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoardProbe {
    pub board: String,
    pub slots: Vec<String>,
    pub format: Format,
    pub read: u64,
}

/// The `flash.json` fields, as Lemnos's schema 1 names them. Fields are private:
/// the board is checked where it is read, and the bytes are made from them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlashManifest {
    board: String,
    format: Format,
    image_size: u64,
    image_sha256: String,
}

impl FlashManifest {
    /// The board id the image is for.
    pub fn board(&self) -> &str {
        &self.board
    }

    /// The stream's format.
    pub fn format(&self) -> Format {
        self.format
    }

    /// The raw image's size in bytes.
    pub fn image_size(&self) -> u64 {
        self.image_size
    }

    /// The raw image's SHA-256, lowercase hex.
    pub fn image_sha256(&self) -> &str {
        &self.image_sha256
    }

    /// The `flash.json` bytes: compact, the keys in this order, no trailing newline.
    /// The board id is `[a-z0-9-]` and the digest is hex, so nothing is escaped.
    pub fn to_json(&self) -> Vec<u8> {
        format!(
            "{{\"schema\":{SCHEMA},\"board\":\"{}\",\"format\":\"{}\",\"image\":{{\"size\":{},\"sha256\":\"{}\"}}}}",
            self.board,
            self.format.as_str(),
            self.image_size,
            self.image_sha256
        )
        .into_bytes()
    }
}

/// The first-flash manifest and what travels with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlashImage {
    pub manifest: FlashManifest,
    /// The `flash.json` bytes, exactly as [`FlashManifest::to_json`] makes them.
    pub json: Vec<u8>,
    /// The detached signature's bytes, when `flash.sig` is beside the image.
    pub signature: Option<Vec<u8>>,
    /// The bytes read from the image file.
    pub read: u64,
}

impl FlashImage {
    /// Whether a detached signature file came with the image.
    pub fn signature_present(&self) -> bool {
        self.signature.is_some()
    }
}

/// Why an image is not flashed to the board it was given to. Each message says in
/// plain words what was found and what to do; nothing is pushed in any of these.
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
    /// The image is not one this reader takes: damaged, truncated, or over a limit.
    #[error(transparent)]
    Image(#[from] ImageError),
    /// The image is for another board, or names none.
    #[error(transparent)]
    Refused(#[from] Refusal),
    /// The detached signature file is not 64 bytes.
    #[error("flash.sig is {0} bytes; a detached signature is 64 bytes")]
    Signature(u64),
}

impl FlashError {
    /// The same failure as an [`ImageError`], for the slot reader (which never
    /// makes a board refusal, so the refusal arm is a plain layout message).
    pub(crate) fn into_image(self) -> ImageError {
        match self {
            FlashError::Image(error) => error,
            FlashError::Refused(refusal) => ImageError::Layout(refusal.to_string()),
            FlashError::Signature(len) => {
                ImageError::Malformed(format!("flash.sig is {len} bytes, not 64"))
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

/// Reads the board from the slots of a spooled data partition. Every slot manifest
/// in `slots/` names a board; they must all name the same one.
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
        let value: serde_json::Value = serde_json::from_str(&text).map_err(|error| {
            ImageError::Malformed(format!("slot {id:?}'s manifest is not JSON: {error}"))
        })?;
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

/// One pass over the image in `mode`: the format is sniffed, an xz index is checked
/// before any decode, and the stream is routed.
struct Pass {
    output: Output,
    format: Format,
    read: u64,
}

fn run(image: &Path, scratch_parent: &Path, mode: Mode) -> Result<Pass, FlashError> {
    let options = crate::Options::default();
    let scratch = Scratch::create(scratch_parent)?;
    let mut file = Counted::open(image)?;
    let xz = xzscan::prepare(&mut file, MAX_DICT_BYTES)?;
    let mut input = BufReader::with_capacity(READ_BUFFER, file);
    let format = if xz { Format::Xz } else { Format::Raw };
    let mut router = Router::new(scratch.path(), &options, mode);
    let stream = if xz {
        decode::decode(&mut input, &mut router).map_err(|error| error.to_string())
    } else {
        io::copy(&mut input, &mut router)
            .map(|_| ())
            .map_err(|error| error.to_string())
    };
    let output = router.complete(stream)?;
    let read = input.get_ref().read;
    Ok(Pass {
        output,
        format,
        read,
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

/// Makes the first-flash manifest of the image at `image`, and reads its
/// `flash.sig` beside it when there is one. With `target` set, an image for another
/// board is refused at the data partition's end, before the rest of the stream is
/// decoded.
pub fn flash_manifest(
    image: &Path,
    scratch_parent: &Path,
    target: Option<&str>,
) -> Result<FlashImage, FlashError> {
    if let Some(target) = target {
        if !valid_board_id(target) {
            return Err(Refusal::BadTarget {
                target: target.to_string(),
            }
            .into());
        }
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
        format: pass.format,
        image_size: raw.len,
        image_sha256: raw.sha256.iter().map(|b| format!("{b:02x}")).collect(),
    };
    let json = manifest.to_json();
    let signature = signature_beside(image)?;
    Ok(FlashImage {
        manifest,
        json,
        signature,
        read: pass.read,
    })
}

/// The bytes of `flash.sig` beside the image, when it is there. It must be 64 bytes.
fn signature_beside(image: &Path) -> Result<Option<Vec<u8>>, FlashError> {
    let path = image
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(SIGNATURE_FILE);
    match fs::metadata(&path) {
        Ok(meta) if meta.is_file() => {
            let len = meta.len();
            if len != SIGNATURE_BYTES {
                return Err(FlashError::Signature(len));
            }
            let bytes = fs::read(&path).map_err(|error| ImageError::io("the signature", &error))?;
            Ok(Some(bytes))
        }
        Ok(_) => {
            Err(ImageError::Malformed(format!("{SIGNATURE_FILE} is not a regular file")).into())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(ImageError::io("the signature", &error).into()),
    }
}
