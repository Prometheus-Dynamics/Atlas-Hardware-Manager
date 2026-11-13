//! Linux: sysfs for disks, mountinfo for mounts, no external tools. A whole
//! disk is written through one exclusive handle, after its own mounts are
//! taken off. An exclusive open fails while anything else holds the disk.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::unix::fs::{FileExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use crate::disk::is_system_mount;
use crate::platform::{Guard, LINUX_PROOF, Platform, Proof, RawDisk};
use crate::{Disk, DiskError, DiskSource};

const SKIPPED_PREFIXES: [&str; 8] = ["loop", "ram", "zram", "dm-", "md", "sr", "fd", "nbd"];

fn read_trimmed(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

/// `/dev/sdb1` → mount points, from /proc/self/mountinfo.
fn mounts_by_source() -> HashMap<String, Vec<String>> {
    let mut mounts: HashMap<String, Vec<String>> = HashMap::new();
    let Ok(text) = fs::read_to_string("/proc/self/mountinfo") else {
        return mounts;
    };
    for line in text.lines() {
        let Some((before, after)) = line.split_once(" - ") else {
            continue;
        };
        let mount_point = before.split_whitespace().nth(4).unwrap_or_default();
        let source = after.split_whitespace().nth(1).unwrap_or_default();
        if source.starts_with("/dev/") {
            mounts
                .entry(source.to_string())
                .or_default()
                .push(unescape_mount(mount_point));
        }
    }
    mounts
}

/// mountinfo escapes spaces and a few other bytes as `\040` style octal.
fn unescape_mount(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\'
            && index + 3 < bytes.len()
            && let Ok(value) = u8::from_str_radix(&raw[index + 1..index + 4], 8)
        {
            out.push(value);
            index += 4;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn swap_devices() -> Vec<String> {
    fs::read_to_string("/proc/swaps")
        .unwrap_or_default()
        .lines()
        .skip(1)
        .filter_map(|line| line.split_whitespace().next().map(str::to_string))
        .collect()
}

/// Device nodes of the device-mapper devices stacked on a disk or partition,
/// followed recursively (crypt on LVM on a partition).
fn mapped_holders(holders: &Path) -> Vec<Vec<String>> {
    let Ok(entries) = fs::read_dir(holders) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let holder = entry.file_name().to_string_lossy().into_owned();
        let sys = Path::new("/sys/block").join(&holder);
        let mut nodes = vec![format!("/dev/{holder}")];
        if let Some(mapped) = read_trimmed(&sys.join("dm/name")) {
            nodes.push(format!("/dev/mapper/{mapped}"));
        }
        found.push(nodes);
        found.extend(mapped_holders(&sys.join("holders")));
    }
    found
}

fn partitions(sys: &Path, name: &str) -> Vec<String> {
    fs::read_dir(sys)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|child| child.starts_with(name) && child.len() > name.len())
                .collect()
        })
        .unwrap_or_default()
}

/// The deepest `bus-port[.port…]` component of a sysfs device path, such as
/// `1-4.2` in `/sys/devices/…/usb1/1-4/1-4.2/1-4.2:1.0/host6/…`.
fn usb_port(resolved: &Path) -> Option<String> {
    resolved
        .components()
        .rev()
        .filter_map(|part| part.as_os_str().to_str())
        .find(|text| {
            let Some((bus, ports)) = text.split_once('-') else {
                return false;
            };
            !bus.is_empty()
                && bus.bytes().all(|b| b.is_ascii_digit())
                && !ports.is_empty()
                && ports.bytes().all(|b| b.is_ascii_digit() || b == b'.')
        })
        .map(str::to_string)
}

/// Every whole disk on this computer, read from sysfs and mountinfo. Marks
/// the disks that hold the running system. Reads only; nothing is opened for
/// writing.
pub fn list_disks() -> Result<Vec<Disk>, DiskError> {
    let entries = fs::read_dir("/sys/block").map_err(|error| DiskError::List(error.to_string()))?;
    let mounts = mounts_by_source();
    let swaps = swap_devices();
    let mut disks = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if SKIPPED_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix))
        {
            continue;
        }
        let sys = entry.path();
        let sectors: u64 = read_trimmed(&sys.join("size"))
            .and_then(|text| text.parse().ok())
            .unwrap_or(0);
        let resolved = fs::canonicalize(&sys).unwrap_or_else(|_| sys.clone());
        let usb = resolved.components().any(|part| {
            part.as_os_str()
                .to_str()
                .is_some_and(|text| text.starts_with("usb"))
        });
        let usb_port = usb.then(|| usb_port(&resolved)).flatten();

        let parts = partitions(&sys, &name);
        let mut nodes = vec![format!("/dev/{name}")];
        nodes.extend(parts.iter().map(|part| format!("/dev/{part}")));
        // LVM and dm-crypt: the mount source is the mapped device above us.
        let mut holder_dirs = vec![sys.join("holders")];
        holder_dirs.extend(parts.iter().map(|part| sys.join(part).join("holders")));
        for holder in holder_dirs.iter().flat_map(|dir| mapped_holders(dir)) {
            nodes.extend(holder);
        }
        let mut mount_points: Vec<String> = nodes
            .iter()
            .filter_map(|node| mounts.get(node))
            .flatten()
            .cloned()
            .collect();
        // btrfs subvolumes and bind mounts repeat the same point.
        mount_points.sort();
        mount_points.dedup();
        let system = mount_points.iter().any(|point| is_system_mount(point))
            || nodes.iter().any(|node| swaps.contains(node));

        disks.push(Disk {
            path: PathBuf::from(format!("/dev/{name}")),
            size_bytes: sectors * 512,
            vendor: read_trimmed(&sys.join("device/vendor")),
            model: read_trimmed(&sys.join("device/model")),
            serial: read_trimmed(&sys.join("device/serial")),
            removable: read_trimmed(&sys.join("removable")).as_deref() == Some("1"),
            usb,
            usb_port,
            system,
            mount_points,
            name,
        });
    }
    disks.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(disks)
}

/// Takes the disk's own mounts off, deepest first, so nested mounts come off
/// before their parents. A system mount never gets here: `check_target`
/// refuses those disks first.
fn unmount_all(disk: &Disk) -> Result<(), DiskError> {
    let mut points = disk.mount_points.clone();
    points.sort_by_key(|point| std::cmp::Reverse(point.len()));
    for point in points {
        nix::mount::umount(point.as_str())
            .map_err(|error| DiskError::Busy(disk.name.clone(), format!("{point}: {error}")))?;
    }
    Ok(())
}

/// One exclusive handle for writes and read-back alike. A second exclusive
/// open of a block device fails while the first is held.
fn open_exclusive(disk: &Disk) -> Result<File, DiskError> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(nix::libc::O_EXCL)
        .open(&disk.path)
        .map_err(|error| DiskError::io(&disk.path, error))
}

impl RawDisk for File {
    fn write_at(&mut self, offset: u64, data: &[u8]) -> io::Result<()> {
        self.write_all_at(data, offset)
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        // The pages are clean after the flush: dropping them makes the read
        // come from the medium.
        let _ = nix::fcntl::posix_fadvise(
            &*self,
            offset as i64,
            buf.len() as i64,
            nix::fcntl::PosixFadviseAdvice::POSIX_FADV_DONTNEED,
        );
        self.read_exact_at(buf, offset)
    }

    fn sync(&mut self) -> io::Result<()> {
        // On a block device this also flushes the device's own cache.
        self.sync_data()
    }
}

/// Linux: sysfs lists the disks, mountinfo says what is mounted, unmounting
/// takes the disk's own mounts off, and one exclusive open holds the device.
#[derive(Clone, Copy, Debug, Default)]
pub struct LinuxPlatform;

impl DiskSource for LinuxPlatform {
    fn disks(&self) -> Result<Vec<Disk>, DiskError> {
        list_disks()
    }
}

impl Platform for LinuxPlatform {
    fn name(&self) -> &'static str {
        "Linux"
    }

    fn proof(&self) -> Proof {
        LINUX_PROOF
    }

    fn prepare(&self, disk: &Disk) -> Result<Guard, DiskError> {
        unmount_all(disk)?;
        Ok(Box::new(()))
    }

    fn open(&self, disk: &Disk) -> Result<Box<dyn RawDisk>, DiskError> {
        Ok(Box::new(open_exclusive(disk)?))
    }

    fn finish(&self, disk: &Disk, mut raw: Box<dyn RawDisk>) -> Result<(), DiskError> {
        raw.sync().map_err(|error| DiskError::io(&disk.path, error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_usb_port_is_the_deepest_port_component() {
        let path = Path::new(
            "/sys/devices/pci0000:00/0000:02:00.0/usb1/1-4/1-4.2/1-4.2:1.0/host6/target6:0:0/6:0:0:0/block/sdc",
        );
        assert_eq!(usb_port(path).as_deref(), Some("1-4.2"));
        assert_eq!(
            usb_port(Path::new("/sys/devices/virtual/block/zram0")),
            None
        );
    }

    #[test]
    fn mount_escapes_are_decoded() {
        assert_eq!(unescape_mount("/media/My\\040Card"), "/media/My Card");
        assert_eq!(unescape_mount("/plain"), "/plain");
    }

    #[test]
    fn listing_marks_the_root_disk_as_system() {
        let disks = list_disks().unwrap();
        for disk in disks
            .iter()
            .filter(|disk| disk.mount_points.iter().any(|point| point == "/"))
        {
            assert!(disk.system);
        }
    }
}
