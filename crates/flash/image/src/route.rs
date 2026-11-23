//! The one pass over the image. The decoder (xz, or the raw bytes) writes into a
//! [`Router`], a `Write`: it reads the partition table (GPT or MBR) from the first sectors,
//! finds the data partition by the label of the filesystem in it (what the image itself says:
//! an MBR names no partition, and no partition is ever taken by its position), then spools that
//! partition to a scratch file. Bytes outside the data partition are decoded and dropped.
//!
//! How far the pass goes. A first-flash pass ([`Mode::Flash`], not a probe) runs to the end of
//! the stream: every byte is decoded, hashed and covered by the xz block checks, because the
//! whole disk is what is written. A slots pass ([`Mode::Slots`], what an install reads) ends as
//! soon as the data partition is spooled and every other partition has been looked at: the tail
//! of the disk is not decoded. What protects an install does not depend on the tail: the slot's
//! file and boot files are checked against the manifest's digests here, the manifest is signed,
//! and the board verifies it. A product image is a whole disk (15.6 GB for one board's, nearly all
//! of it the unwritten rest of the data partition), so decoding the tail took about a minute.
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
use crate::fsprobe::{self, Found, PROBE_BYTES};
use crate::gpt::{self, Header};
use crate::mbr;
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

/// Which partition the pass keeps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Select {
    /// The one partition whose filesystem carries this label: what the image itself says, on a
    /// GPT and an MBR disk alike. The data partition is found this way.
    Label(String),
    /// The one partition a GPT names so. An MBR names none, so an MBR image is refused.
    GptName(String),
}

/// One partition as the pass sees it.
#[derive(Debug)]
struct Part {
    /// Its number in the table, from 1.
    number: u32,
    /// How the table describes it: `type 0x0c` (MBR) or `named "data"` (GPT).
    kind: String,
    /// Its GPT name; empty on an MBR disk.
    name: String,
    start: u64,
    end: u64,
    /// Its first bytes, until they are probed.
    head: Vec<u8>,
    /// What its first bytes said, once read.
    found: Option<Found>,
}

impl Part {
    /// The partition in words, for a refusal: its number, the table's word for it, and the
    /// filesystem and label read from it.
    fn words(&self) -> String {
        let read = match &self.found {
            Some(found) => match &found.label {
                Some(label) => format!("{}, labelled {label:?}", found.filesystem),
                None => format!("{}, no label", found.filesystem),
            },
            None => "not read".to_owned(),
        };
        format!("partition {} ({}, {read})", self.number, self.kind)
    }
}

pub(crate) struct Router<'a> {
    scratch: &'a Path,
    options: &'a Options,
    mode: Mode,
    select: Select,
    /// The bytes before the partition table ends, until it is read.
    head: Vec<u8>,
    header: Option<Header>,
    /// Whether the table is read (then `parts` is the disk's partitions, by start).
    planned: bool,
    /// The next byte's offset in the image, once the table is read.
    pos: u64,
    parts: Vec<Part>,
    /// The partition being kept, as an index into `parts`, once it is known.
    chosen: Option<usize>,
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
    /// A pass that keeps the data partition: the one whose filesystem carries the label the
    /// options state.
    pub(crate) fn new(scratch: &'a Path, options: &'a Options, mode: Mode) -> Self {
        let hash = matches!(mode, Mode::Flash { .. }).then(Sha256::new);
        Self {
            scratch,
            options,
            mode,
            select: Select::Label(options.names.data_label.clone()),
            head: Vec::new(),
            header: None,
            planned: false,
            pos: 0,
            parts: Vec::new(),
            chosen: None,
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

    /// The same pass, keeping the partition `select` picks instead.
    pub(crate) fn keeping(mut self, select: Select) -> Self {
        self.select = select;
        self
    }

    /// What is being looked for, in words.
    fn wanted(&self) -> String {
        match &self.select {
            Select::Label(label) => format!("a partition whose filesystem is labelled {label:?}"),
            Select::GptName(name) => format!("a partition named {name:?}"),
        }
    }

    /// Every partition in words, for a refusal that says what the image holds.
    fn listed(&self) -> String {
        self.parts
            .iter()
            .map(Part::words)
            .collect::<Vec<_>>()
            .join(", ")
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
        if !self.planned {
            return Err(FlashError::Image(ImageError::Truncated(
                "the image ends inside its partition table".into(),
            )));
        }
        let Some(chosen) = self.chosen else {
            // The image ended before a partition's first bytes: it is cut short, not unlabelled.
            if let Some(cut) = self
                .parts
                .iter()
                .find(|part| part.found.is_none() && self.pos < part.end)
                .filter(|_| matches!(self.select, Select::Label(_)))
            {
                return Err(FlashError::Image(ImageError::Truncated(format!(
                    "the image ends at byte {}, before partition {} ({}) could be read",
                    self.pos, cut.number, cut.kind
                ))));
            }
            return Err(FlashError::Image(ImageError::Layout(format!(
                "the image has no {}: it holds {}",
                self.wanted().trim_start_matches("a "),
                self.listed()
            ))));
        };
        let part = &self.parts[chosen];
        if self.pos < part.end {
            return Err(FlashError::Image(ImageError::Truncated(format!(
                "the image ends at byte {}, inside partition {} ({}; {} bytes at {})",
                self.pos,
                part.number,
                part.kind,
                part.end - part.start,
                part.start
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
        if self.planned {
            return self.feed_data(buf);
        }
        self.head.extend_from_slice(buf);
        if self.head.len() < 1024 {
            return Ok(());
        }
        if self.header.is_none() {
            // A GPT disk has its header at LBA 1 (behind a protective MBR). A disk with no GPT
            // header and a signed first sector is an MBR disk.
            if self.head[512..].starts_with(gpt::SIGNATURE) {
                self.header = Some(gpt::header(&self.head)?);
            } else if mbr::signed(&self.head) && !mbr::protective(&self.head) {
                let entries = mbr::entries(&self.head[..512])?;
                let rest = self.head.split_off(512);
                self.head = Vec::new();
                self.pos = 512;
                self.plan(
                    entries
                        .into_iter()
                        .map(|entry| Part {
                            number: u32::from(entry.number),
                            kind: format!("type {:#04x}", entry.kind),
                            name: String::new(),
                            start: entry.start,
                            end: entry.start + entry.len,
                            head: Vec::new(),
                            found: None,
                        })
                        .collect(),
                    "mbr",
                )?;
                return self.feed_data(&rest);
            } else {
                return Err(ImageError::Table(
                    "the partition table is not valid: there is no GPT header at LBA 1 and no MBR partition table in the first sector".into(),
                ));
            }
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
        self.plan(
            entries
                .into_iter()
                .enumerate()
                .map(|(index, entry)| Part {
                    number: index as u32 + 1,
                    kind: format!("named {:?}", entry.name),
                    name: entry.name,
                    start: entry.start,
                    end: entry.start + entry.len,
                    head: Vec::new(),
                    found: None,
                })
                .collect(),
            "gpt",
        )?;
        self.feed_data(&rest)
    }

    /// Takes the table's partitions, in the order their bytes come. A partition picked by its GPT
    /// name is known here; one picked by its filesystem's label is known when its first bytes pass.
    fn plan(&mut self, mut parts: Vec<Part>, scheme: &str) -> Result<(), ImageError> {
        parts.sort_by_key(|part| part.start);
        if let Some(pair) = parts.windows(2).find(|pair| pair[1].start < pair[0].end) {
            return Err(ImageError::Table(format!(
                "partitions {} and {} overlap",
                pair[0].number, pair[1].number
            )));
        }
        if let Some(early) = parts.iter().find(|part| part.start < self.pos) {
            return Err(ImageError::Table(format!(
                "partition {} starts inside the partition table",
                early.number
            )));
        }
        self.partitions = parts
            .iter()
            .map(|part| PartitionInfo {
                name: part.name.clone(),
                start: part.start,
                len: part.end - part.start,
            })
            .collect();
        self.parts = parts;
        self.planned = true;
        if let Select::GptName(name) = &self.select {
            if scheme != "gpt" {
                return Err(ImageError::Layout(format!(
                    "the image's partition table is an MBR, which names no partition, so there is no partition named {name:?}: it holds {} (not read: a partition is found by name only on a GPT disk)",
                    self.listed()
                )));
            }
            let mut hits = self
                .parts
                .iter()
                .enumerate()
                .filter(|(_, part)| &part.name == name);
            let first = hits
                .next()
                .map(|(index, _)| index)
                .ok_or_else(|| ImageError::Layout(format!("no {name:?} partition")))?;
            if hits.next().is_some() {
                return Err(ImageError::Layout(format!(
                    "two partitions are named {name:?}"
                )));
            }
            self.choose(first)?;
        }
        Ok(())
    }

    /// Keeps partition `index`: checks it against the spool cap and opens the spool.
    fn choose(&mut self, index: usize) -> Result<(), ImageError> {
        let part = &self.parts[index];
        let len = part.end - part.start;
        if len > self.options.max_spool_bytes {
            return Err(ImageError::Limit(format!(
                "partition {} ({}) is {len} bytes, more than the {} the reader spools",
                part.number, part.kind, self.options.max_spool_bytes
            )));
        }
        let path = self.scratch.join("data.img");
        let file = File::create(&path).map_err(|error| ImageError::io("the spool", &error))?;
        self.spool = Some((BufWriter::with_capacity(1 << 20, file), path));
        self.chosen = Some(index);
        Ok(())
    }

    fn spool_write(&mut self, bytes: &[u8]) -> Result<(), ImageError> {
        let (writer, _) = self
            .spool
            .as_mut()
            .ok_or_else(|| ImageError::Io("the spool is not open".into()))?;
        writer
            .write_all(bytes)
            .map_err(|error| ImageError::io("the spool", &error))
    }

    /// Routes image bytes from `self.pos` on. Between partitions the bytes are dropped. Inside
    /// the kept partition they are spooled. Inside any other, when the partition is found by its
    /// label, its first bytes are read for the filesystem and label, and the rest is dropped.
    fn feed_data(&mut self, mut buf: &[u8]) -> Result<(), ImageError> {
        if !self.planned {
            return Err(ImageError::Layout("the table was not read".into()));
        }
        while !buf.is_empty() {
            // The partition these bytes are in, or the next one after them.
            let Some(index) = self.parts.iter().position(|part| self.pos < part.end) else {
                self.pos += buf.len() as u64;
                return Ok(());
            };
            let (start, end) = (self.parts[index].start, self.parts[index].end);
            if self.pos < start {
                let skip = (start - self.pos).min(buf.len() as u64) as usize;
                self.pos += skip as u64;
                buf = &buf[skip..];
                continue;
            }
            if self.chosen == Some(index) && self.spool.is_some() {
                let take = (end - self.pos).min(buf.len() as u64) as usize;
                self.spool_write(&buf[..take])?;
                self.pos += take as u64;
                buf = &buf[take..];
                if self.pos == end {
                    self.finish()?;
                    if self.stopped {
                        return Ok(());
                    }
                }
                continue;
            }
            let by_label = match &self.select {
                Select::Label(label) => Some(label.clone()),
                Select::GptName(_) => None,
            };
            if let Some(label) = by_label.filter(|_| self.parts[index].found.is_none()) {
                let need = (PROBE_BYTES as u64).min(end - start) as usize;
                let have = self.parts[index].head.len();
                let take = (need - have).min(buf.len());
                self.parts[index].head.extend_from_slice(&buf[..take]);
                self.pos += take as u64;
                buf = &buf[take..];
                if self.parts[index].head.len() < need {
                    continue;
                }
                let head = std::mem::take(&mut self.parts[index].head);
                let found = fsprobe::probe(&head);
                let matched = found.label.as_deref() == Some(label.as_str());
                self.parts[index].found = Some(found);
                if !matched {
                    self.stop_when_known();
                    if self.stopped {
                        return Ok(());
                    }
                    continue;
                }
                if let Some(first) = self.chosen {
                    return Err(ImageError::Layout(format!(
                        "two partitions' filesystems are labelled {label:?}: partition {} and partition {}; the image holds {}",
                        self.parts[first].number,
                        self.parts[index].number,
                        self.listed()
                    )));
                }
                self.choose(index)?;
                self.spool_write(&head)?;
                if self.pos == end {
                    self.finish()?;
                    if self.stopped {
                        return Ok(());
                    }
                }
                continue;
            }
            // Not kept: the rest of this partition is dropped.
            let skip = (end - self.pos).min(buf.len() as u64) as usize;
            self.pos += skip as u64;
            buf = &buf[skip..];
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
                Err(other) => return Err(ImageError::Layout(other.to_string())),
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
        self.stop_when_known();
        Ok(())
    }

    /// Ends a slots pass as soon as nothing after this point can change its answer: the kept
    /// partition is spooled, and every other partition has been looked at (so a second partition
    /// with the same label would have been refused). The bytes after that are not decoded.
    ///
    /// So a read for an install checks the image up to there: the xz checks of the blocks it
    /// decoded, and the slot's own digests and signature. A whole-disk flash ([`Mode::Flash`]
    /// run to the end) decodes and checks all of it.
    fn stop_when_known(&mut self) {
        if !matches!(self.mode, Mode::Slots) || self.data.is_none() {
            return;
        }
        let all_seen = match &self.select {
            Select::GptName(_) => true,
            Select::Label(_) => self.parts.iter().all(|part| part.found.is_some()),
        };
        if all_seen {
            self.stopped = true;
        }
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
