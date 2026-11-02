//! Elsewhere than Linux a disk is not a resumable write's target: the agent
//! that needs one runs on a Linux controller. Same API, refused on open.

use std::io;
use std::time::Instant;

use super::Target;
use crate::{BlockError, Disk};

pub type Reopen<'a> = Box<dyn FnMut(Instant) -> Result<Disk, String> + Send + 'a>;

pub struct DiskTarget<'a> {
    disk: Disk,
    _reopen: Reopen<'a>,
}

fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "resumable disk writes run on Linux only",
    )
}

impl<'a> DiskTarget<'a> {
    pub fn open(disk: Disk, _reopen: Reopen<'a>) -> Result<Self, BlockError> {
        Err(BlockError::io(&disk.path, unsupported()))
    }

    pub fn disk(&self) -> &Disk {
        &self.disk
    }
}

impl Target for DiskTarget<'_> {
    fn capacity(&self) -> u64 {
        self.disk.size_bytes
    }

    fn write_at(&mut self, _offset: u64, _data: &[u8]) -> io::Result<()> {
        Err(unsupported())
    }

    fn flush(&mut self) -> io::Result<()> {
        Err(unsupported())
    }

    fn read_at(&mut self, _offset: u64, _buf: &mut [u8]) -> io::Result<()> {
        Err(unsupported())
    }

    fn reattach(&mut self, _deadline: Instant) -> Result<(), String> {
        Err(unsupported().to_string())
    }
}
