//! The one pass over the image. The decoder (xz, or the raw bytes) writes into a
//! [`Router`], a `Write`: it reads the partition table from the first sectors,
//! then spools the data partition to a scratch file. Bytes outside the data
//! partition are decoded and dropped, so the decoder runs to the end of the
//! stream and the xz block checks cover the whole file.
//!
//! The data partition is kept: the slot files are read from it with random access
//! (`ext4-view` needs that), and a bundle writer reads a slot's file twice. Its
//! length is the scratch disk the read needs.
//!
//! In the first-flash mode ([`Mode::Flash`]) every decoded byte is also hashed
//! (the raw image's SHA-256), and the board is read from the slots as soon as the
//! data partition is spooled. A board the caller did not ask for stops the pass
//! there, and so does a probe for the board alone: the bytes after the data
//! partition are then never decoded.

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::error::ImageError;
use crate::flash::{self, BoardInfo, FlashError, Refusal};
use crate::gpt::{self, Entry, Header};
use crate::{Options, PartitionInfo};

/// What the pass does with the board.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    /// `open`: the slots are read later; nothing is hashed and no board is read.
    Slots,
    /// The first-flash pass. `target`, when set, is the board the caller is
    /// connected to: a different board refuses at the data partition's end.
    /// `stop_after_board` ends the pass once the board is known.
    Flash {
        target: Option<String>,
        stop_after_board: bool,
    },
}

/// The raw image as the pass saw it: its length and SHA-256.
#[derive(Debug)]
pub(crate) struct Raw {
    pub(crate) len: u64,
    pub(crate) sha256: [u8; 32],
}

/// What the pass hands back: the partitions, the data spool, and in the first-flash
/// mode the board and (when the pass ran to the end) the raw image's digest.
#[derive(Debug)]
pub(crate) struct Output {
    pub(crate) partitions: Vec<PartitionInfo>,
    pub(crate) data: PathBuf,
    pub(crate) board: Option<BoardInfo>,
    pub(crate) raw: Option<Raw>,
}

pub(crate) struct Router<'a> {
    scratch: &'a Path,
    options: &'a Options,
    mode: Mode,
    /// The bytes before the partition table ends, until it is read.
    head: Vec<u8>,
    header: Option<Header>,
    /// The next byte's offset in the image, once the table is read.
    pos: u64,
    /// The data partition, once the table names it.
    want: Option<(u64, u64)>,
    partitions: Vec<PartitionInfo>,
    spool: Option<(BufWriter<File>, PathBuf)>,
    data: Option<PathBuf>,
    board: Option<BoardInfo>,
    /// The raw image's digest and length, in the first-flash mode.
    hash: Option<Sha256>,
    raw_len: u64,
    /// Set when the pass ends by its own choice (a probe has its board).
    stopped: bool,
    /// The board check refused the image. Reported before any decode error.
    refused: Option<Refusal>,
    /// The first failure. The decoder only ever sees "stopped" for it.
    error: Option<ImageError>,
}

const STOPPED: &str = "the image read has stopped";

impl<'a> Router<'a> {
    pub(crate) fn new(scratch: &'a Path, options: &'a Options, mode: Mode) -> Self {
        let hash = matches!(mode, Mode::Flash { .. }).then(Sha256::new);
        Self {
            scratch,
            options,
            mode,
            head: Vec::new(),
            header: None,
            pos: 0,
            want: None,
            partitions: Vec::new(),
            spool: None,
            data: None,
            board: None,
            hash,
            raw_len: 0,
            stopped: false,
            refused: None,
            error: None,
        }
    }

    /// Ends the pass. `stream` is the decoder's own result; a refusal or a router
    /// failure is the real cause and is reported first.
    pub(crate) fn complete(mut self, stream: Result<(), String>) -> Result<Output, FlashError> {
        if let Some(refusal) = self.refused.take() {
            return Err(FlashError::Refused(refusal));
        }
        if let Some(error) = self.error.take() {
            return Err(FlashError::Image(error));
        }
        if self.stopped {
            let data = self
                .data
                .take()
                .ok_or_else(|| ImageError::Layout("the data partition was not read".into()))?;
            return Ok(Output {
                partitions: self.partitions,
                data,
                board: self.board,
                raw: None,
            });
        }
        if let Err(message) = stream {
            return Err(FlashError::Image(ImageError::Decode(message)));
        }
        let Some((start, end)) = self.want else {
            return Err(FlashError::Image(ImageError::Truncated(
                "the image ends inside its partition table".into(),
            )));
        };
        if self.pos < end {
            return Err(FlashError::Image(ImageError::Truncated(format!(
                "the image ends at byte {}, inside the {:?} partition ({} bytes at {start})",
                self.pos,
                self.options.names.data,
                end - start
            ))));
        }
        let data = self.data.take().ok_or_else(|| {
            FlashError::Image(ImageError::Layout("the data partition was not read".into()))
        })?;
        let raw_len = self.raw_len;
        let raw = self.hash.take().map(|hash| Raw {
            len: raw_len,
            sha256: hash.finalize().into(),
        });
        Ok(Output {
            partitions: self.partitions,
            data,
            board: self.board,
            raw,
        })
    }

    fn feed(&mut self, buf: &[u8]) -> Result<(), ImageError> {
        if self.want.is_some() {
            return self.feed_data(buf);
        }
        self.head.extend_from_slice(buf);
        if self.header.is_none() {
            if self.head.len() < 1024 {
                return Ok(());
            }
            self.header = Some(gpt::header(&self.head)?);
        }
        let Some(header) = self.header.clone() else {
            return Ok(());
        };
        let end = header.end();
        let end_len = usize::try_from(end).map_err(|_| ImageError::Table("too far".into()))?;
        if self.head.len() < end_len {
            return Ok(());
        }
        let entries = gpt::entries(&self.head[..end_len], &header)?;
        let rest = self.head.split_off(end_len);
        self.head = Vec::new();
        self.pos = end;
        self.plan(entries)?;
        self.feed_data(&rest)
    }

    /// Finds the data partition by name and checks it against the spool cap.
    fn plan(&mut self, entries: Vec<Entry>) -> Result<(), ImageError> {
        let name = &self.options.names.data;
        let mut hits = entries.iter().filter(|entry| &entry.name == name);
        let data = hits
            .next()
            .cloned()
            .ok_or_else(|| ImageError::Layout(format!("no {name:?} partition")))?;
        if hits.next().is_some() {
            return Err(ImageError::Layout(format!(
                "two partitions are named {name:?}"
            )));
        }
        if data.len > self.options.max_spool_bytes {
            return Err(ImageError::Limit(format!(
                "the {name:?} partition is {} bytes, more than the {} the reader spools",
                data.len, self.options.max_spool_bytes
            )));
        }
        self.want = Some((data.start, data.start + data.len));
        self.partitions = entries
            .into_iter()
            .map(|entry| PartitionInfo {
                name: entry.name,
                start: entry.start,
                len: entry.len,
            })
            .collect();
        Ok(())
    }

    /// Routes image bytes from `self.pos` on: skips the gap before the data
    /// partition, spools the partition, discards what follows it.
    fn feed_data(&mut self, mut buf: &[u8]) -> Result<(), ImageError> {
        let Some((start, end)) = self.want else {
            return Err(ImageError::Layout("the table was not read".into()));
        };
        while !buf.is_empty() {
            if self.pos < start {
                let skip = (start - self.pos).min(buf.len() as u64) as usize;
                self.pos += skip as u64;
                buf = &buf[skip..];
                continue;
            }
            if self.pos >= end {
                self.pos += buf.len() as u64;
                return Ok(());
            }
            if self.spool.is_none() {
                let path = self.scratch.join("data.img");
                let file =
                    File::create(&path).map_err(|error| ImageError::io("the spool", &error))?;
                self.spool = Some((BufWriter::with_capacity(1 << 20, file), path));
            }
            let take = (end - self.pos).min(buf.len() as u64) as usize;
            let (writer, _) = self
                .spool
                .as_mut()
                .ok_or_else(|| ImageError::Io("the spool is not open".into()))?;
            writer
                .write_all(&buf[..take])
                .map_err(|error| ImageError::io("the spool", &error))?;
            self.pos += take as u64;
            buf = &buf[take..];
            if self.pos == end {
                self.finish()?;
                if self.stopped {
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    /// The data partition is spooled. In the first-flash mode its slots name the
    /// board; a refusal is kept for the caller, and a probe stops here.
    fn finish(&mut self) -> Result<(), ImageError> {
        let (mut writer, path) = self
            .spool
            .take()
            .ok_or_else(|| ImageError::Io("the spool is not open".into()))?;
        writer
            .flush()
            .map_err(|error| ImageError::io("the spool", &error))?;
        drop(writer);

        let flash = match &self.mode {
            Mode::Slots => None,
            Mode::Flash {
                target,
                stop_after_board,
            } => Some((target.clone(), *stop_after_board)),
        };
        if let Some((target, stop_after_board)) = flash {
            let info = match flash::read_board(&path) {
                Ok(info) => info,
                Err(FlashError::Refused(refusal)) => {
                    self.refused = Some(refusal);
                    return Err(ImageError::Layout("the image was refused".into()));
                }
                Err(FlashError::Image(error)) => return Err(error),
                Err(FlashError::Signature(_)) => {
                    return Err(ImageError::Layout("unexpected signature error".into()));
                }
            };
            if let Some(target) = target.filter(|t| info.board != *t) {
                self.refused = Some(Refusal::WrongBoard {
                    image: info.board.clone(),
                    target,
                });
                return Err(ImageError::Layout("the image was refused".into()));
            }
            self.board = Some(info);
            if stop_after_board {
                self.stopped = true;
            }
        }
        self.data = Some(path);
        Ok(())
    }
}

impl Write for Router<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.error.is_some() || self.refused.is_some() || self.stopped {
            return Err(io::Error::other(STOPPED));
        }
        if let Some(hash) = self.hash.as_mut() {
            hash.update(buf);
            self.raw_len += buf.len() as u64;
        }
        match self.feed(buf) {
            Ok(()) if self.stopped => Err(io::Error::other(STOPPED)),
            Ok(()) => Ok(buf.len()),
            Err(error) => {
                self.error = Some(error);
                Err(io::Error::other(STOPPED))
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
