//! The platform seam: everything that differs between systems when a disk is
//! written. The write itself (`device`, `writer`, `sector`) is the same code on
//! every system; a platform only says how to list disks, take their volumes off,
//! open the raw device, finish it, and who may do it.

use std::io;
use std::sync::Arc;

use crate::{Disk, DiskError, DiskSource};

/// Whether a system's disk write has been shown to work on real hardware.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Proof {
    Proven,
    Unproven,
}

/// Whether the caller accepts a write on a system whose proof is withheld.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Acknowledgement {
    /// Not accepted: a write to a real disk is refused on an unproven system.
    Withheld,
    /// Accepted on purpose, after the host test on that system passed.
    Given,
}

/// THE ONE PLACE where a system's disk write is marked proven on hardware.
///
/// Flip a value here only after the host test in
/// `docs/host-tests/disk-writer.md` has passed on that system, and say so in
/// the PR. Until then a write to a real disk on that system is refused by
/// default. Listing disks stays available: it writes nothing.
pub const LINUX_PROOF: Proof = Proof::Proven;
/// UNPROVEN: needs the Windows host test (docs/host-tests/disk-writer.md).
pub const WINDOWS_PROOF: Proof = Proof::Unproven;
/// UNPROVEN: needs the macOS host test (docs/host-tests/disk-writer.md).
pub const MACOS_PROOF: Proof = Proof::Unproven;

/// The proof of the system this build runs on.
pub const fn host_proof() -> Proof {
    if cfg!(target_os = "linux") {
        LINUX_PROOF
    } else if cfg!(windows) {
        WINDOWS_PROOF
    } else if cfg!(target_os = "macos") {
        MACOS_PROOF
    } else {
        Proof::Unproven
    }
}

/// The acknowledgement a helper passes for this build: given exactly when the
/// host system is proven.
pub fn host_acknowledgement() -> Acknowledgement {
    match host_proof() {
        Proof::Proven => Acknowledgement::Given,
        Proof::Unproven => Acknowledgement::Withheld,
    }
}

/// Holds a disk's volumes unmounted or locked for as long as it lives. Dropping
/// it releases them.
pub type Guard = Box<dyn Send>;

/// An open raw disk: positioned writes and reads, and a flush to the medium.
/// Reads come from the medium, not from a cache.
pub trait RawDisk: Send {
    fn write_at(&mut self, offset: u64, data: &[u8]) -> io::Result<()>;
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<()>;
    /// Makes what was written durable on the medium.
    fn sync(&mut self) -> io::Result<()>;
}

/// What a system provides for writing a disk.
pub trait Platform: DiskSource {
    /// Names the system in messages: `Linux`, `Windows`, `macOS`.
    fn name(&self) -> &'static str;

    fn proof(&self) -> Proof;

    /// Refuses when this process may not write a disk, with a message that
    /// says how to get the right (elevation, an authorisation prompt).
    fn may_write(&self) -> Result<(), DiskError> {
        Ok(())
    }

    /// Takes the disk's volumes off (unmounted, or locked and dismounted), so
    /// no file system writes into the sectors being written.
    fn prepare(&self, disk: &Disk) -> Result<Guard, DiskError>;

    /// Opens the raw device of a disk that [`Platform::prepare`] has taken off.
    fn open(&self, disk: &Disk) -> Result<Box<dyn RawDisk>, DiskError>;

    /// Ends a write that was verified: flushes, makes the system read the new
    /// contents, and ejects where that is the norm. Takes the raw device.
    fn finish(&self, disk: &Disk, raw: Box<dyn RawDisk>) -> Result<(), DiskError>;
}

/// The refusal every write path goes through first. Unproven systems refuse
/// without the acknowledgement, before anything is listed or touched.
pub(crate) fn gate(platform: &dyn Platform, ack: Acknowledgement) -> Result<(), DiskError> {
    if platform.proof() == Proof::Unproven && ack == Acknowledgement::Withheld {
        return Err(DiskError::Unsupported(format!(
            "writing a disk on {} has not been proven on this system yet; run the host test in docs/host-tests/disk-writer.md and then set its proof in crates/flash/disk/src/platform.rs",
            platform.name()
        )));
    }
    Ok(())
}

/// The platform of the system this build runs on.
pub fn host_platform() -> Arc<dyn Platform> {
    #[cfg(target_os = "linux")]
    {
        Arc::new(crate::linux::LinuxPlatform)
    }
    #[cfg(windows)]
    {
        Arc::new(crate::windows::WindowsPlatform)
    }
    #[cfg(target_os = "macos")]
    {
        Arc::new(crate::macos::MacPlatform)
    }
    #[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
    {
        Arc::new(crate::unsupported::UnsupportedPlatform)
    }
}

/// Every whole disk on this computer, from the host platform. Read only.
pub fn list_disks() -> Result<Vec<Disk>, DiskError> {
    host_platform().disks()
}

/// The host's disks, as a [`DiskSource`]. Production reads the system; tests
/// pass a list they made up instead.
#[derive(Clone, Copy, Debug, Default)]
pub struct SysfsDisks;

impl DiskSource for SysfsDisks {
    fn disks(&self) -> Result<Vec<Disk>, DiskError> {
        list_disks()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unproven_system_refuses_without_the_acknowledgement() {
        struct Unproven;
        impl DiskSource for Unproven {
            fn disks(&self) -> Result<Vec<Disk>, DiskError> {
                Ok(Vec::new())
            }
        }
        impl Platform for Unproven {
            fn name(&self) -> &'static str {
                "Testland"
            }
            fn proof(&self) -> Proof {
                Proof::Unproven
            }
            fn prepare(&self, _: &Disk) -> Result<Guard, DiskError> {
                unreachable!("the gate refuses before anything is prepared")
            }
            fn open(&self, _: &Disk) -> Result<Box<dyn RawDisk>, DiskError> {
                unreachable!("the gate refuses before anything is opened")
            }
            fn finish(&self, _: &Disk, _: Box<dyn RawDisk>) -> Result<(), DiskError> {
                unreachable!("the gate refuses before anything is finished")
            }
        }
        let refused = gate(&Unproven, Acknowledgement::Withheld);
        assert!(
            matches!(&refused, Err(DiskError::Unsupported(text)) if text.contains("not been proven"))
        );
        assert!(gate(&Unproven, Acknowledgement::Given).is_ok());
    }

    #[test]
    fn the_host_acknowledgement_follows_the_host_proof() {
        let expected = match host_proof() {
            Proof::Proven => Acknowledgement::Given,
            Proof::Unproven => Acknowledgement::Withheld,
        };
        assert_eq!(host_acknowledgement(), expected);
    }
}
