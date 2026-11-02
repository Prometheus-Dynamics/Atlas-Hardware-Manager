//! A whole disk on this Linux host as a resumable write's [`Target`].

use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::{FileExt, OpenOptionsExt};
use std::time::Instant;

use super::Target;
use crate::{BlockError, Disk};

/// Finds the disk again after a drop, by a deadline. It may come back under
/// another name (`sda` as `sdb`); it must be the same size.
pub type Reopen<'a> = Box<dyn FnMut(Instant) -> Result<Disk, String> + Send + 'a>;

pub struct DiskTarget<'a> {
    disk: Disk,
    file: Option<File>,
    reopen: Reopen<'a>,
}

/// One exclusive handle for writes and read-back alike: a second O_EXCL open
/// of a block device fails while the first is held.
fn open(disk: &Disk) -> Result<File, BlockError> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(nix::libc::O_EXCL)
        .open(&disk.path)
        .map_err(|error| BlockError::io(&disk.path, error))
}

impl<'a> DiskTarget<'a> {
    /// The caller has checked `disk` with [`check_target`](crate::check_target)
    /// and unmounted it, and `reopen` does the same for what it returns.
    pub fn open(disk: Disk, reopen: Reopen<'a>) -> Result<Self, BlockError> {
        Ok(Self {
            file: Some(open(&disk)?),
            disk,
            reopen,
        })
    }

    /// The disk as it is now, after any reattach.
    pub fn disk(&self) -> &Disk {
        &self.disk
    }

    fn file(&self) -> io::Result<&File> {
        self.file
            .as_ref()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "the disk is gone"))
    }
}

impl Target for DiskTarget<'_> {
    fn capacity(&self) -> u64 {
        self.disk.size_bytes
    }

    fn write_at(&mut self, offset: u64, data: &[u8]) -> io::Result<()> {
        self.file()?.write_all_at(data, offset)
    }

    fn flush(&mut self) -> io::Result<()> {
        // On a block device this also flushes the device's own cache.
        self.file()?.sync_data()
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        let file = self.file()?;
        // The pages are clean after the flush: dropping them makes the read
        // come from the medium.
        let _ = nix::fcntl::posix_fadvise(
            file,
            offset as i64,
            buf.len() as i64,
            nix::fcntl::PosixFadviseAdvice::POSIX_FADV_DONTNEED,
        );
        file.read_exact_at(buf, offset)
    }

    fn reattach(&mut self, deadline: Instant) -> Result<(), String> {
        self.file = None;
        let disk = (self.reopen)(deadline)?;
        if disk.size_bytes != self.disk.size_bytes {
            return Err(format!(
                "{} came back as {} bytes, not {}",
                disk.name, disk.size_bytes, self.disk.size_bytes
            ));
        }
        self.file = Some(open(&disk).map_err(|error| error.to_string())?);
        self.disk = disk;
        Ok(())
    }
}
