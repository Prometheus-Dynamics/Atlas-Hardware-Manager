//! Platforms where disks are not listed or written yet. Every call says so,
//! so nothing on these platforms can pass for a write.

use std::io;
use std::sync::Arc;

use crate::{Disk, DiskError, DiskSource, Target};

fn unsupported() -> DiskError {
    DiskError::Unsupported("disk discovery and writing are only available on Linux so far".into())
}

pub fn list_disks() -> Result<Vec<Disk>, DiskError> {
    Err(unsupported())
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SysfsDisks;

impl DiskSource for SysfsDisks {
    fn disks(&self) -> Result<Vec<Disk>, DiskError> {
        list_disks()
    }
}

#[derive(Debug)]
pub struct BlockDevice {
    _disk: Disk,
}

impl BlockDevice {
    pub fn open(_disk: Disk, _source: Arc<dyn DiskSource>) -> Result<Self, DiskError> {
        Err(unsupported())
    }
}

impl Target for BlockDevice {
    fn capacity(&self) -> u64 {
        0
    }

    fn identity(&self) -> String {
        String::new()
    }

    fn write_at(&mut self, _offset: u64, _data: &[u8]) -> io::Result<()> {
        Err(io::Error::other(unsupported().to_string()))
    }

    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::other(unsupported().to_string()))
    }

    fn read_at(&mut self, _offset: u64, _buf: &mut [u8]) -> io::Result<()> {
        Err(io::Error::other(unsupported().to_string()))
    }

    fn reattach(&mut self) -> Result<(), String> {
        Err(unsupported().to_string())
    }
}
