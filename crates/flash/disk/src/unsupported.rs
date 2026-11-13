//! Systems where disks are not listed or written. Every call says so, so
//! nothing on these systems can pass for a write.

use crate::platform::{Guard, Platform, Proof, RawDisk};
use crate::{Disk, DiskError, DiskSource};

/// The platform of a system Atlas does not write disks on.
#[derive(Clone, Copy, Debug, Default)]
pub struct UnsupportedPlatform;

fn unsupported() -> DiskError {
    DiskError::Unsupported("disk discovery and writing are not available on this system".into())
}

impl DiskSource for UnsupportedPlatform {
    fn disks(&self) -> Result<Vec<Disk>, DiskError> {
        Err(unsupported())
    }
}

impl Platform for UnsupportedPlatform {
    fn name(&self) -> &'static str {
        "this system"
    }

    fn proof(&self) -> Proof {
        Proof::Unproven
    }

    fn prepare(&self, _disk: &Disk) -> Result<Guard, DiskError> {
        Err(unsupported())
    }

    fn open(&self, _disk: &Disk) -> Result<Box<dyn RawDisk>, DiskError> {
        Err(unsupported())
    }

    fn finish(&self, _disk: &Disk, _raw: Box<dyn RawDisk>) -> Result<(), DiskError> {
        Err(unsupported())
    }
}
