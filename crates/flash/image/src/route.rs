//! The one pass over the image. The decoder (xz, or the raw bytes) writes into a
//! [`Router`], a `Write`: it reads the partition table from the first sectors,
//! then spools the data partition to a scratch file. Bytes outside the data
//! partition are decoded and dropped, so the decoder runs to the end of the
//! stream and the xz block checks cover the whole file.
//!
//! The data partition is kept: the slot files are read from it with random access
//! (`ext4-view` needs that), and a bundle writer reads a slot's file twice. Its
//! length is the scratch disk the read needs.

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::error::ImageError;
use crate::gpt::{self, Entry, Header};
use crate::{Options, PartitionInfo};

/// What the pass hands back: the partitions and the data spool.
#[derive(Debug)]
pub(crate) struct Output {
    pub(crate) partitions: Vec<PartitionInfo>,
    pub(crate) data: PathBuf,
}

pub(crate) struct Router<'a> {
    scratch: &'a Path,
    options: &'a Options,
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
    /// The first failure. The decoder only ever sees "stopped" for it.
    error: Option<ImageError>,
}

const STOPPED: &str = "the image read has stopped";

impl<'a> Router<'a> {
    pub(crate) fn new(scratch: &'a Path, options: &'a Options) -> Self {
        Self {
            scratch,
            options,
            head: Vec::new(),
            header: None,
            pos: 0,
            want: None,
            partitions: Vec::new(),
            spool: None,
            data: None,
            error: None,
        }
    }

    /// Ends the pass. `stream` is the decoder's own result; a router failure is
    /// the real cause and is reported first.
    pub(crate) fn complete(mut self, stream: Result<(), String>) -> Result<Output, ImageError> {
        if let Some(error) = self.error.take() {
            return Err(error);
        }
        if let Err(message) = stream {
            return Err(ImageError::Decode(message));
        }
        let Some((start, end)) = self.want else {
            return Err(ImageError::Truncated(
                "the image ends inside its partition table".into(),
            ));
        };
        if self.pos < end {
            return Err(ImageError::Truncated(format!(
                "the image ends at byte {}, inside the {:?} partition ({} bytes at {start})",
                self.pos,
                self.options.names.data,
                end - start
            )));
        }
        let data = self
            .data
            .take()
            .ok_or_else(|| ImageError::Layout("the data partition was not read".into()))?;
        Ok(Output {
            partitions: self.partitions,
            data,
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
            }
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<(), ImageError> {
        let (mut writer, path) = self
            .spool
            .take()
            .ok_or_else(|| ImageError::Io("the spool is not open".into()))?;
        writer
            .flush()
            .map_err(|error| ImageError::io("the spool", &error))?;
        drop(writer);
        self.data = Some(path);
        Ok(())
    }
}

impl Write for Router<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.error.is_some() {
            return Err(io::Error::other(STOPPED));
        }
        match self.feed(buf) {
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
