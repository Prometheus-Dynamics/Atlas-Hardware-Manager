//! The one pass over the image. The decoder (xz, or the raw bytes) writes into a
//! [`Router`], which is a `Write`: it reads the partition table from the first
//! sectors, then takes the wanted partitions in disk order. A stream cannot
//! seek, so nothing is read twice here:
//!
//! - the boot partition (the rootfs of the bring-up form, the FAT boot partition
//!   of the real form) is spooled, its boot files are copied out of it, and its
//!   spool is deleted before the next partition starts;
//! - the data partition is spooled and kept, because the slot file is read from
//!   it with random access (`ext4-view` needs that), and the bundle writer reads
//!   the slot twice.
//!
//! So at most one partition is spooled at a time, and the spool of the data
//! partition is the only large file kept. Bytes outside the wanted partitions
//! are decoded and dropped: the decoder runs to the end of the stream, so the
//! xz block checks run over the whole file.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::budget::Budget;
use crate::error::ImageError;
use crate::gpt::{self, Entry, Header};
use crate::names;
use crate::{Form, Options, PartitionInfo, ext, fat};

/// One boot file as the reader keeps it: where its bytes are now, and their size.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Extracted {
    pub(crate) path: PathBuf,
    pub(crate) size: u64,
}

/// The boot files of one slot, by path under its `boot/` (`cmdline.txt`,
/// `overlays/x.dtbo`). Keyed by path, sorted.
pub(crate) type BootSet = BTreeMap<String, Extracted>;

/// The boot sets by key: the slot id in the real form, and the empty key for the
/// one set every slot shares in the bring-up form (its `/boot`).
pub(crate) type BootSets = BTreeMap<String, BootSet>;

/// What the pass hands back: the form, the partitions, the data spool and the
/// extracted boot files.
#[derive(Debug)]
pub(crate) struct Output {
    pub(crate) form: Form,
    pub(crate) partitions: Vec<PartitionInfo>,
    pub(crate) data: PathBuf,
    pub(crate) boot: BootSets,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    /// The boot files come from this partition, then it is deleted.
    Boot,
    /// The slot file comes from this partition, and it stays.
    Data,
}

#[derive(Clone, Debug)]
struct Want {
    role: Role,
    name: String,
    start: u64,
    end: u64,
}

pub(crate) struct Router<'a> {
    scratch: &'a Path,
    requested: Vec<String>,
    options: &'a Options,
    budget: Budget,
    /// The bytes before the partition table ends, until it is read.
    head: Vec<u8>,
    header: Option<Header>,
    /// The next byte's offset in the image, once the table is read.
    pos: u64,
    form: Option<Form>,
    partitions: Vec<PartitionInfo>,
    wants: Vec<Want>,
    /// The index of the next wanted partition not yet finished.
    cursor: usize,
    spool: Option<(BufWriter<File>, PathBuf)>,
    data: Option<PathBuf>,
    boot: BootSets,
    /// The first failure. The decoder only ever sees "stopped" for it.
    error: Option<ImageError>,
}

const STOPPED: &str = "the image read has stopped";

impl<'a> Router<'a> {
    pub(crate) fn new(scratch: &'a Path, requested: Vec<String>, options: &'a Options) -> Self {
        Self {
            scratch,
            requested,
            options,
            budget: Budget::new(options.max_boot_bytes, options.max_file_bytes),
            head: Vec::new(),
            header: None,
            pos: 0,
            form: None,
            partitions: Vec::new(),
            wants: Vec::new(),
            cursor: 0,
            spool: None,
            data: None,
            boot: BootSets::new(),
            error: None,
        }
    }

    /// Ends the pass. `stream` is the decoder's own result: its failure is
    /// reported unless a router failure, which is the real cause, came first.
    pub(crate) fn complete(mut self, stream: Result<(), String>) -> Result<Output, ImageError> {
        if let Some(error) = self.error.take() {
            return Err(error);
        }
        if let Err(message) = stream {
            return Err(ImageError::Decode(message));
        }
        let Some(form) = self.form else {
            return Err(ImageError::Truncated(
                "the image ends inside its partition table".into(),
            ));
        };
        if let Some(want) = self.wants.get(self.cursor) {
            return Err(ImageError::Truncated(format!(
                "the image ends at byte {}, before the {} partition ({} bytes at {})",
                self.pos,
                want.name,
                want.end - want.start,
                want.start
            )));
        }
        let data = self
            .data
            .take()
            .ok_or_else(|| ImageError::Layout("the data partition was not read".into()))?;
        Ok(Output {
            form,
            partitions: self.partitions,
            data,
            boot: self.boot,
        })
    }

    fn feed(&mut self, buf: &[u8]) -> Result<(), ImageError> {
        if self.form.is_some() {
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

    /// Chooses the form from the partition names and the wanted partitions.
    fn plan(&mut self, entries: Vec<Entry>) -> Result<(), ImageError> {
        let names_cfg = &self.options.names;
        let named = |name: &str| -> Result<Option<Entry>, ImageError> {
            let mut hits = entries.iter().filter(|entry| entry.name == name);
            let first = hits.next().cloned();
            if first.is_some() && hits.next().is_some() {
                return Err(ImageError::Layout(format!(
                    "two partitions are named {name:?}"
                )));
            }
            Ok(first)
        };
        let has_rootfs = named(&names_cfg.rootfs)?.is_some();
        let has_boot = named(&names_cfg.boot)?.is_some();
        let (form, boot_name) = match (has_rootfs, has_boot) {
            (true, false) => (Form::BringUp, names_cfg.rootfs.clone()),
            (false, true) => (Form::Real, names_cfg.boot.clone()),
            (true, true) => {
                return Err(ImageError::Layout(
                    "the image has both a rootfs and a boot partition: neither form".into(),
                ));
            }
            (false, false) => {
                return Err(ImageError::Layout(
                    "the image has neither a rootfs nor a boot partition".into(),
                ));
            }
        };
        let boot = named(&boot_name)?
            .ok_or_else(|| ImageError::Layout(format!("no {boot_name:?} partition")))?;
        let data = named(&names_cfg.data)?
            .ok_or_else(|| ImageError::Layout(format!("no {:?} partition", names_cfg.data)))?;
        if boot.start >= data.start {
            return Err(ImageError::Layout(format!(
                "the {:?} partition lies before the {boot_name:?} partition: the image is read once, in disk order",
                names_cfg.data
            )));
        }
        for (what, entry) in [(&boot_name, &boot), (&names_cfg.data, &data)] {
            if entry.len > self.options.max_spool_bytes {
                return Err(ImageError::Limit(format!(
                    "the {what:?} partition is {} bytes, more than the {} the reader spools",
                    entry.len, self.options.max_spool_bytes
                )));
            }
        }
        self.wants = vec![
            Want {
                role: Role::Boot,
                name: boot.name.clone(),
                start: boot.start,
                end: boot.start + boot.len,
            },
            Want {
                role: Role::Data,
                name: data.name.clone(),
                start: data.start,
                end: data.start + data.len,
            },
        ];
        self.partitions = entries
            .into_iter()
            .map(|entry| PartitionInfo {
                name: entry.name,
                start: entry.start,
                len: entry.len,
            })
            .collect();
        self.form = Some(form);
        Ok(())
    }

    /// Routes image bytes from `self.pos` on: skips the gaps, spools the wanted
    /// partitions, discards the rest.
    fn feed_data(&mut self, mut buf: &[u8]) -> Result<(), ImageError> {
        while !buf.is_empty() {
            let Some(want) = self.wants.get(self.cursor).cloned() else {
                self.pos += buf.len() as u64;
                return Ok(());
            };
            if self.pos < want.start {
                let skip = (want.start - self.pos).min(buf.len() as u64) as usize;
                self.pos += skip as u64;
                buf = &buf[skip..];
                continue;
            }
            if self.spool.is_none() {
                self.open_spool()?;
            }
            let take = (want.end - self.pos).min(buf.len() as u64) as usize;
            let (writer, _) = self
                .spool
                .as_mut()
                .ok_or_else(|| ImageError::Io("the spool is not open".into()))?;
            writer
                .write_all(&buf[..take])
                .map_err(|error| ImageError::io("the spool", &error))?;
            self.pos += take as u64;
            buf = &buf[take..];
            if self.pos == want.end {
                self.finish_want(&want)?;
            }
        }
        Ok(())
    }

    fn open_spool(&mut self) -> Result<(), ImageError> {
        let path = self.scratch.join(format!("partition-{}.img", self.cursor));
        let file = File::create(&path).map_err(|error| ImageError::io("the spool", &error))?;
        self.spool = Some((BufWriter::with_capacity(1 << 20, file), path));
        Ok(())
    }

    fn finish_want(&mut self, want: &Want) -> Result<(), ImageError> {
        let (mut writer, path) = self
            .spool
            .take()
            .ok_or_else(|| ImageError::Io("the spool is not open".into()))?;
        writer
            .flush()
            .map_err(|error| ImageError::io("the spool", &error))?;
        drop(writer);
        self.cursor += 1;
        match want.role {
            Role::Data => self.data = Some(path),
            Role::Boot => {
                self.extract_boot(&path)?;
                fs::remove_file(&path).map_err(|error| ImageError::io("the spool", &error))?;
            }
        }
        Ok(())
    }

    fn extract_boot(&mut self, spool: &Path) -> Result<(), ImageError> {
        let form = self
            .form
            .ok_or_else(|| ImageError::Layout("the boot partition came before the table".into()))?;
        match form {
            Form::BringUp => {
                let fs = ext::load(spool)?;
                let dir = format!("/{}", self.options.names.bringup_boot_dir);
                let dest = self.scratch.join("boot").join("shared");
                let set = ext::extract(&fs, &dir, &dest, &mut self.budget)?;
                self.boot.insert(String::new(), set);
            }
            Form::Real => {
                for id in self.requested.clone() {
                    let dir = names::expand(&self.options.names.boot_dir, &id);
                    names::split_rel(&dir)?;
                    let dest = self.scratch.join("boot").join(&id);
                    let set = fat::extract(spool, &dir, &dest, &mut self.budget)?;
                    self.boot.insert(id, set);
                }
            }
        }
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
