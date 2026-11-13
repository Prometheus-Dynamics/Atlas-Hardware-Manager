//! The write pipeline, the same on every system. A platform says how to take a
//! disk's volumes off, open its raw device and finish it; this module decides
//! the order and applies the safety rules that do not depend on the system:
//!
//! 1. the proof gate (an unproven system refuses without the acknowledgement);
//! 2. the elevation check of the platform;
//! 3. a fresh listing: the chosen disk must still be there, unchanged, safe to
//!    write and within the size bound;
//! 4. the image is checked (hash, format, fit) before any volume is touched;
//! 5. prepare, open, write and verify, through drops, then finish.
//!
//! A failed write is not finished: its raw device and locks are only dropped,
//! and nothing else happens to the disk after the failure.

use std::io;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::platform::{Acknowledgement, Guard, Platform, RawDisk, gate};
use crate::writer::{
    ResumePolicy, WriteOptions, WriteProgress, WriteReport, check_image, require_segments,
    write_checked,
};
use crate::{Disk, DiskError, Target, check_size_choice, check_target, find_again, same_disk};

/// A write to one disk: the volumes are off and the raw device is open until
/// [`Device::finish`] or the drop. Dropping it releases the disk without
/// finishing it.
pub struct Device {
    platform: Arc<dyn Platform>,
    disk: Disk,
    // Declared before `guard`: the raw device closes before the volumes come back.
    raw: Option<Box<dyn RawDisk>>,
    guard: Option<Guard>,
}

impl std::fmt::Debug for Device {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Device")
            .field("platform", &self.platform.name())
            .field("disk", &self.disk)
            .finish_non_exhaustive()
    }
}

/// Takes the volumes off, then opens the raw device. On an error the guard is
/// dropped here, so the volumes come back.
fn attach(platform: &dyn Platform, disk: &Disk) -> Result<(Box<dyn RawDisk>, Guard), DiskError> {
    let guard = platform.prepare(disk)?;
    let raw = platform.open(disk)?;
    Ok((raw, guard))
}

impl Device {
    /// Opens a disk for writing. The caller has checked `disk` on a fresh
    /// listing; [`write_disk`] does that.
    pub fn open(
        platform: Arc<dyn Platform>,
        disk: Disk,
        ack: Acknowledgement,
    ) -> Result<Self, DiskError> {
        gate(platform.as_ref(), ack)?;
        platform.may_write()?;
        Self::attach_checked(platform, disk)
    }

    /// Attaches a disk that the caller has just checked (`write_disk` does
    /// the gate, elevation and listing checks before calling this).
    fn attach_checked(platform: Arc<dyn Platform>, disk: Disk) -> Result<Self, DiskError> {
        let (raw, guard) = attach(platform.as_ref(), &disk)?;
        Ok(Self {
            platform,
            disk,
            raw: Some(raw),
            guard: Some(guard),
        })
    }

    /// The disk as it was last attached.
    pub fn disk(&self) -> &Disk {
        &self.disk
    }

    /// Ends a verified write: the platform flushes, re-reads and ejects, then
    /// the volumes are released.
    pub fn finish(mut self) -> Result<(), DiskError> {
        let raw = self
            .raw
            .take()
            .ok_or_else(|| DiskError::Finish("the disk is not open".into()))?;
        let result = self.platform.finish(&self.disk, raw);
        drop(self.guard.take());
        result.map_err(|error| DiskError::Finish(error.to_string()))
    }

    fn raw(&mut self) -> io::Result<&mut (dyn RawDisk + 'static)> {
        self.raw
            .as_deref_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "the disk is gone"))
    }
}

impl Target for Device {
    fn capacity(&self) -> u64 {
        self.disk.size_bytes
    }

    fn identity(&self) -> String {
        self.disk
            .usb_port
            .clone()
            .unwrap_or_else(|| self.disk.path.display().to_string())
    }

    fn write_at(&mut self, offset: u64, data: &[u8]) -> io::Result<()> {
        self.raw()?.write_at(offset, data)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.raw()?.sync()
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        self.raw()?.read_at(offset, buf)
    }

    /// After a drop: finds the disk again, checks it as new and still the
    /// chosen one, and attaches it.
    fn reattach(&mut self) -> Result<(), String> {
        self.raw = None;
        self.guard = None;
        let disks = self.platform.disks().map_err(|error| error.to_string())?;
        let now = find_again(&disks, &self.disk)
            .ok_or_else(|| format!("{} is not there", self.disk.name))?;
        check_target(&now, Some(self.disk.size_bytes)).map_err(|error| error.to_string())?;
        same_disk(&now, &self.disk).map_err(|error| error.to_string())?;
        let (raw, guard) =
            attach(self.platform.as_ref(), &now).map_err(|error| error.to_string())?;
        self.disk = now;
        self.raw = Some(raw);
        self.guard = Some(guard);
        Ok(())
    }
}

/// What a write asks for, besides the disk and the platform.
#[derive(Debug)]
pub struct WriteRequest<'a> {
    pub image: &'a Path,
    pub options: &'a WriteOptions,
    pub policy: &'a ResumePolicy,
    pub journal: Option<&'a Path>,
    /// Writes a disk above [`crate::LARGE_DISK_BYTES`]; false refuses one.
    pub allow_large: bool,
}

/// Writes `request.image` to the disk that was chosen (`chosen`, as listed
/// when the person chose it), on `platform`, through the rules in the module
/// documentation. Returns the report once the disk is finished.
pub fn write_disk(
    platform: Arc<dyn Platform>,
    chosen: &Disk,
    ack: Acknowledgement,
    request: &WriteRequest<'_>,
    progress: &mut dyn FnMut(WriteProgress),
    cancel: &AtomicBool,
) -> Result<WriteReport, DiskError> {
    gate(platform.as_ref(), ack)?;
    platform.may_write()?;
    require_segments(request.policy)?;
    let started = std::time::Instant::now();

    let disks = platform.disks()?;
    let now = find_again(&disks, chosen)
        .ok_or_else(|| DiskError::NotFound(chosen.path.display().to_string()))?;
    check_target(&now, Some(chosen.size_bytes))?;
    same_disk(&now, chosen)?;
    check_size_choice(&now, request.allow_large)?;

    let checked = check_image(
        request.image,
        request.options,
        now.size_bytes,
        progress,
        cancel,
    )?;
    let mut device = Device::attach_checked(platform, now)?;
    let report = write_checked(
        request.image,
        &checked,
        &mut device,
        request.options,
        request.policy,
        request.journal,
        progress,
        cancel,
        started,
    )?;
    device.finish()?;
    Ok(report)
}
