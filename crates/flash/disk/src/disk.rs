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

/// The disk that was chosen is the disk that is there: the same size, and the
/// same serial, model and USB port wherever both records have them.
pub fn same_disk(now: &Disk, chosen: &Disk) -> Result<(), DiskError> {
    let differs =
        |a: &Option<String>, b: &Option<String>| matches!((a, b), (Some(x), Some(y)) if x != y);
    if now.size_bytes != chosen.size_bytes
        || differs(&now.serial, &chosen.serial)
        || differs(&now.model, &chosen.model)
        || differs(&now.usb_port, &chosen.usb_port)
    {
        return Err(DiskError::Unsafe(format!(
            "{} is not the disk that was chosen (its size, model, serial or port differs); choose it again",
            chosen.name
        )));
    }
    Ok(())
}

/// The largest disk written on the first choice. A card for a first flash is
/// smaller than this; a larger removable disk is more often one that holds
/// data, so it is written only after a second, explicit choice.
pub const LARGE_DISK_BYTES: u64 = 256_000_000_000;

/// Refuses a disk above [`LARGE_DISK_BYTES`] unless the caller chose it.
pub fn check_size_choice(disk: &Disk, allow_large: bool) -> Result<(), DiskError> {
    if !allow_large && disk.size_bytes > LARGE_DISK_BYTES {
        return Err(DiskError::Unsafe(format!(
            "{} is {} bytes, more than {LARGE_DISK_BYTES} (256 GB); it is written only after a second, explicit choice that it is the card to write",
            disk.name, disk.size_bytes
        )));
    }
    Ok(())
}

/// Accepts a device path only when it names a whole disk of this system:
/// `/dev/sdX`, `/dev/mmcblkN` or `/dev/nvmeNnM` on Linux, `/dev/diskN` on
/// macOS, `\\.\PhysicalDriveN` on Windows. Partitions, other names and
/// anything with a `..` or a shell character fail the shape, so no path reaches
/// the disk layer unvalidated.
pub fn device_path(text: &str) -> Result<PathBuf, DiskError> {
    if whole_disk_path(text) {
        Ok(PathBuf::from(text))
    } else {
        Err(DiskError::Unsafe(format!(
            "{text} is not a whole disk path; {WHOLE_DISK_SHAPE}"
        )))
    }
}

#[cfg(target_os = "linux")]
const WHOLE_DISK_SHAPE: &str = "expected /dev/sdX, /dev/mmcblkN or /dev/nvmeNnM";
#[cfg(target_os = "macos")]
const WHOLE_DISK_SHAPE: &str = "expected /dev/diskN";
#[cfg(windows)]
const WHOLE_DISK_SHAPE: &str = r"expected \\.\PhysicalDriveN";
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
const WHOLE_DISK_SHAPE: &str = "no whole disk names are known on this system";

#[cfg(target_os = "linux")]
fn whole_disk_path(text: &str) -> bool {
    text.strip_prefix("/dev/").is_some_and(whole_disk_name)
}

#[cfg(target_os = "macos")]
fn whole_disk_path(text: &str) -> bool {
    text.strip_prefix("/dev/disk").is_some_and(digits)
}

#[cfg(windows)]
fn whole_disk_path(text: &str) -> bool {
    text.strip_prefix(r"\\.\PhysicalDrive").is_some_and(digits)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn whole_disk_path(_text: &str) -> bool {
    false
}

fn digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(target_os = "linux")]
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
    #[cfg(target_os = "linux")]
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
    #[cfg(target_os = "macos")]
    fn macos_whole_disks_are_accepted_and_slices_are_not() {
        assert!(device_path("/dev/disk2").is_ok());
        for bad in ["/dev/disk2s1", "/dev/rdisk2", "/dev/disk", "/dev/sda"] {
            assert!(device_path(bad).is_err(), "{bad} should be refused");
        }
    }

    #[test]
    #[cfg(windows)]
    fn windows_whole_disks_are_accepted_and_partitions_are_not() {
        assert!(device_path(r"\\.\PhysicalDrive2").is_ok());
        for bad in [
            r"\\.\PhysicalDrive",
            r"\\.\C:",
            "/dev/sdb",
            r"\\.\PhysicalDrive2 x",
        ] {
            assert!(device_path(bad).is_err(), "{bad} should be refused");
        }
    }

    #[test]
    fn a_chosen_disk_must_still_be_the_disk_that_is_there() {
        let chosen = card();
        assert!(same_disk(&card(), &chosen).is_ok());
        let mut other_serial = card();
        other_serial.serial = Some("ABC".into());
        let mut chosen_serial = card();
        chosen_serial.serial = Some("XYZ".into());
        assert!(same_disk(&other_serial, &chosen_serial).is_err());
        let mut resized = card();
        resized.size_bytes += 512;
        assert!(same_disk(&resized, &chosen).is_err());
        let mut moved = card();
        moved.usb_port = Some("2-1".into());
        assert!(same_disk(&moved, &chosen).is_err());
    }

    #[test]
    fn a_disk_above_the_large_bound_needs_a_second_choice() {
        let mut big = card();
        big.size_bytes = LARGE_DISK_BYTES + 1;
        assert!(check_size_choice(&big, false).is_err());
        assert!(check_size_choice(&big, true).is_ok());
        assert!(check_size_choice(&card(), false).is_ok());
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
