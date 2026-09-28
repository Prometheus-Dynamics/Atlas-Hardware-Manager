use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::BlockError;

/// A whole disk, never a partition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Disk {
    /// The node written to: `/dev/sdb`, `/dev/rdisk4`, `\\.\PhysicalDrive2`.
    pub path: PathBuf,
    /// Short name for people: `sdb`, `disk4`, `PhysicalDrive2`.
    pub name: String,
    pub size_bytes: u64,
    pub vendor: Option<String>,
    pub model: Option<String>,
    pub serial: Option<String>,
    pub removable: bool,
    /// Attached over USB, which includes a Pi in mass-storage-gadget mode.
    pub usb: bool,
    /// Holds the running system, a boot partition, or swap.
    pub system: bool,
    pub mount_points: Vec<String>,
}

impl Disk {
    /// `vendor model`, or the name when neither is known.
    pub fn description(&self) -> String {
        let text = [self.vendor.as_deref(), self.model.as_deref()]
            .into_iter()
            .flatten()
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        if text.is_empty() {
            self.name.clone()
        } else {
            text
        }
    }
}

/// Refuses disks that must never be overwritten. The helper calls this on a
/// freshly listed disk, so a stale choice from the app cannot slip through.
pub fn check_target(disk: &Disk, expected_size: Option<u64>) -> Result<(), BlockError> {
    if disk.system {
        return Err(BlockError::Unsafe(format!(
            "{} holds the running system or a boot partition and will not be written",
            disk.name
        )));
    }
    if !disk.usb && !disk.removable {
        return Err(BlockError::Unsafe(format!(
            "{} is an internal fixed disk; only USB or removable disks can be written",
            disk.name
        )));
    }
    if disk.size_bytes == 0 {
        return Err(BlockError::Unsafe(format!(
            "{} reports no media; insert a card or reconnect the device",
            disk.name
        )));
    }
    if let Some(expected) = expected_size
        && expected != disk.size_bytes
    {
        return Err(BlockError::Unsafe(format!(
            "{} changed size since it was chosen ({expected} to {} bytes); choose it again",
            disk.name, disk.size_bytes
        )));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn disk() -> Disk {
        Disk {
            path: PathBuf::from("/dev/sdz"),
            name: "sdz".into(),
            size_bytes: 32_000_000_000,
            vendor: Some("RPi-MSD-".into()),
            model: Some("0001".into()),
            serial: None,
            removable: true,
            usb: true,
            system: false,
            mount_points: Vec::new(),
        }
    }

    #[test]
    fn usb_removable_disks_pass() {
        assert!(check_target(&disk(), Some(32_000_000_000)).is_ok());
    }

    #[test]
    fn system_internal_empty_and_resized_disks_are_refused() {
        let mut system = disk();
        system.system = true;
        let mut internal = disk();
        internal.usb = false;
        internal.removable = false;
        let mut empty = disk();
        empty.size_bytes = 0;

        for refused in [system, internal, empty] {
            assert!(matches!(
                check_target(&refused, None),
                Err(BlockError::Unsafe(_))
            ));
        }
        assert!(check_target(&disk(), Some(1)).is_err());
    }

    #[test]
    fn description_prefers_vendor_and_model() {
        assert_eq!(disk().description(), "RPi-MSD- 0001");
        let mut bare = disk();
        bare.vendor = None;
        bare.model = Some("  ".into());
        assert_eq!(bare.description(), "sdz");
    }
}
