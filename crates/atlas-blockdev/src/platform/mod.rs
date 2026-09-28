//! Per-OS disk listing and write preparation.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
use linux as os;
#[cfg(target_os = "macos")]
use macos as os;
#[cfg(target_os = "windows")]
use windows as os;

use std::fs::File;

use crate::{BlockError, Disk};

/// Every whole disk on this computer, internal ones included (marked).
pub fn list_disks() -> Result<Vec<Disk>, BlockError> {
    os::list_disks()
}

/// Undoes what [`prepare_for_write`] changed, such as taking a disk offline.
pub(crate) struct Restore(Option<Box<dyn FnOnce()>>);

impl Restore {
    #[cfg_attr(target_os = "windows", allow(dead_code))]
    pub(crate) fn none() -> Self {
        Self(None)
    }

    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub(crate) fn with(undo: impl FnOnce() + 'static) -> Self {
        Self(Some(Box::new(undo)))
    }
}

impl Drop for Restore {
    fn drop(&mut self) {
        if let Some(undo) = self.0.take() {
            undo();
        }
    }
}

/// Unmounts or takes offline everything on the disk so it can be written.
pub(crate) fn prepare_for_write(disk: &Disk) -> Result<Restore, BlockError> {
    os::prepare_for_write(disk)
}

/// Opens the disk for raw writing, refusing if the OS says it is in use.
pub(crate) fn open_for_write(disk: &Disk) -> Result<File, BlockError> {
    os::open_for_write(disk)
}

/// Opens the disk for a verification read that bypasses cached data.
pub(crate) fn open_for_verify(disk: &Disk) -> Result<File, BlockError> {
    os::open_for_verify(disk)
}

/// Raw disk nodes on Windows and macOS need writes in whole sectors.
pub(crate) const SECTOR: usize = 4096;
