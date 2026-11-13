//! What a disk is, which disks may be written, and how a path to one is
//! accepted. Pure: nothing here touches a device, so the rules are tested
//! with made-up disks.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::DiskError;

/// The largest disk that can be a card. SDXC cards top out at 2 TiB.
pub const MAX_CARD_BYTES: u64 = 2 << 40;

/// Trees whose mount means the running system is on the disk.
const SYSTEM_TREES: [&str; 5] = ["/boot", "/efi", "/usr", "/var", "/home"];

/// A whole disk, never a partition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Disk {
    /// The node written to: `/dev/sdb`, `/dev/mmcblk0`.
    pub path: PathBuf,
    /// Short name for people: `sdb`, `mmcblk0`.
    pub name: String,
    pub size_bytes: u64,
    pub vendor: Option<String>,
    pub model: Option<String>,
    pub serial: Option<String>,
    /// The kernel's removable flag. Only removable disks are written.
    pub removable: bool,
    /// Attached over USB. Information only: a USB disk that is not removable
    /// is refused, because an external drive is not a card.
    pub usb: bool,
    /// The USB port path it hangs off, like `1-4.2`. Lets a later lookup find
    /// the same device after it comes back under another name.
    #[serde(default)]
    pub usb_port: Option<String>,
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

/// Whether a mount point belongs to the running system.
pub fn is_system_mount(point: &str) -> bool {
    point == "/"
        || SYSTEM_TREES
            .iter()
            .any(|tree| point == *tree || point.starts_with(&format!("{tree}/")))
}

/// Refuses a disk that must not be written. The helper calls this on a
/// freshly listed disk, so a stale choice from the app cannot slip through.
pub fn check_target(disk: &Disk, expected_size: Option<u64>) -> Result<(), DiskError> {
    if disk.system || disk.mount_points.iter().any(|point| is_system_mount(point)) {
        return Err(DiskError::Unsafe(format!(
            "{} holds the running system or a boot partition and will not be written",
            disk.name
        )));
    }
    if !disk.removable {
        return Err(DiskError::Unsafe(format!(
            "{} is not a removable disk; only cards and removable disks are written",
            disk.name
        )));
    }
    if disk.size_bytes == 0 {
        return Err(DiskError::Unsafe(format!(
            "{} reports no media; insert a card or reconnect the device",
            disk.name
        )));
    }
    if disk.size_bytes > MAX_CARD_BYTES {
        return Err(DiskError::Unsafe(format!(
            "{} is {} bytes, larger than any card; it will not be written",
            disk.name, disk.size_bytes
        )));
    }
    if let Some(expected) = expected_size
        && expected != disk.size_bytes
    {
        return Err(DiskError::Unsafe(format!(
            "{} changed size since it was chosen ({expected} to {} bytes); choose it again",
            disk.name, disk.size_bytes
        )));
    }
    Ok(())
}

/// Accepts a device path only when it names a whole disk under `/dev`:
/// `sdX`, `mmcblkN` or `nvmeNnM`. Partitions, other names and anything with
/// a `..` or a shell character fail the shape, so no path reaches the disk
/// layer unvalidated.
pub fn device_path(text: &str) -> Result<PathBuf, DiskError> {
    let refused = || {
        DiskError::Unsafe(format!(
            "{text} is not a whole disk path; expected /dev/sdX, /dev/mmcblkN or /dev/nvmeNnM"
        ))
    };
    let name = text.strip_prefix("/dev/").ok_or_else(refused)?;
    if whole_disk_name(name) {
        Ok(PathBuf::from(text))
    } else {
        Err(refused())
    }
}

fn digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
}

fn whole_disk_name(name: &str) -> bool {
    let sd = name
        .strip_prefix("sd")
        .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_lowercase()));
    let mmc = name.strip_prefix("mmcblk").is_some_and(digits);
    let nvme = name
        .strip_prefix("nvme")
        .and_then(|rest| rest.split_once('n'))
        .is_some_and(|(controller, namespace)| digits(controller) && digits(namespace));
    sd || mmc || nvme
}

/// Where the disks on this computer come from. Production reads the kernel;
/// tests pass a list they made up.
pub trait DiskSource: Send + Sync {
    /// Every whole disk on this computer, internal ones included.
    fn disks(&self) -> Result<Vec<Disk>, DiskError>;
}

/// The disk that is `was` again, after it came back: the same USB port when
/// it had one, else the same path. `None` when it is not there.
pub fn find_again(disks: &[Disk], was: &Disk) -> Option<Disk> {
    disks
        .iter()
        .find(|disk| match (&was.usb_port, &disk.usb_port) {
            (Some(before), Some(now)) => before == now,
            _ => disk.path == was.path,
        })
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn card() -> Disk {
        Disk {
            path: PathBuf::from("/dev/sdz"),
            name: "sdz".into(),
            size_bytes: 32_000_000_000,
            vendor: Some("Card".into()),
            model: Some("Reader".into()),
            serial: None,
            removable: true,
            usb: true,
            usb_port: Some("1-4.2".into()),
            system: false,
            mount_points: vec!["/run/media/user/CARD".into()],
        }
    }

    fn refused(disk: &Disk, expected: Option<u64>) -> bool {
        matches!(check_target(disk, expected), Err(DiskError::Unsafe(_)))
    }

    #[test]
    fn a_removable_card_passes() {
        assert!(check_target(&card(), Some(32_000_000_000)).is_ok());
        let mut unmounted = card();
        unmounted.mount_points.clear();
        assert!(check_target(&unmounted, None).is_ok());
    }

    #[test]
    fn a_system_disk_is_refused_by_flag_or_by_mount() {
        let mut flagged = card();
        flagged.system = true;
        assert!(refused(&flagged, None));

        for point in ["/", "/boot", "/boot/efi", "/usr/lib", "/var", "/home/sozo"] {
            let mut mounted = card();
            mounted.mount_points = vec![point.into()];
            assert!(refused(&mounted, None), "{point} should refuse");
        }
    }

    #[test]
    fn a_removable_mount_outside_the_system_is_not_a_system_disk() {
        let mut media = card();
        media.mount_points = vec!["/run/media/user/CARD".into(), "/mnt/cards".into()];
        assert!(check_target(&media, None).is_ok());
        assert!(!is_system_mount("/bootable"));
        assert!(!is_system_mount("/variety"));
    }

    #[test]
    fn a_disk_that_is_not_removable_is_refused_even_over_usb() {
        let mut internal = card();
        internal.removable = false;
        internal.usb = false;
        assert!(refused(&internal, None));

        let mut external = card();
        external.removable = false;
        assert!(refused(&external, None), "a USB drive is not a card");
    }

    #[test]
    fn empty_oversized_and_resized_disks_are_refused() {
        let mut empty = card();
        empty.size_bytes = 0;
        assert!(refused(&empty, None));

        let mut huge = card();
        huge.size_bytes = MAX_CARD_BYTES + 1;
        assert!(refused(&huge, None));
        huge.size_bytes = MAX_CARD_BYTES;
        assert!(check_target(&huge, None).is_ok());

        assert!(refused(&card(), Some(1)));
    }

    #[test]
    fn description_prefers_vendor_and_model() {
        assert_eq!(card().description(), "Card Reader");
        let mut bare = card();
        bare.vendor = None;
        bare.model = Some("  ".into());
        assert_eq!(bare.description(), "sdz");
    }

    #[test]
    fn whole_disks_are_accepted_and_everything_else_is_not() {
        for good in ["/dev/sdb", "/dev/sdaa", "/dev/mmcblk0", "/dev/nvme0n1"] {
            assert_eq!(device_path(good).unwrap(), PathBuf::from(good), "{good}");
        }
        for bad in [
            "/dev/sdb1",
            "/dev/mmcblk0p1",
            "/dev/nvme0n1p2",
            "/dev/sdB",
            "/dev/",
            "/dev/sda;reboot",
            "/dev/../sda",
            "/dev/sda 2",
            "sdb",
            "/tmp/sdb",
            "/dev/sd",
        ] {
            assert!(device_path(bad).is_err(), "{bad} should be refused");
        }
    }

    #[test]
    fn a_disk_is_found_again_by_its_usb_port_or_its_path() {
        let mut moved = card();
        moved.path = PathBuf::from("/dev/sdq");
        assert_eq!(find_again(&[moved.clone()], &card()), Some(moved));

        let mut no_port = card();
        no_port.usb_port = None;
        let mut again = card();
        again.usb_port = None;
        assert_eq!(find_again(&[again.clone()], &no_port), Some(again));
        let mut elsewhere = card();
        elsewhere.path = PathBuf::from("/dev/sdq");
        assert_eq!(find_again(&[elsewhere], &no_port), None);
    }
}
