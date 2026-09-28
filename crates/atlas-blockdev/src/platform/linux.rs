//! Linux: sysfs for disks, mountinfo for mounts, no external tools.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use super::Restore;
use crate::{BlockError, Disk};

/// Mount points that mark a disk as holding the running system.
const SYSTEM_MOUNTS: [&str; 7] = ["/", "/boot", "/boot/efi", "/efi", "/usr", "/var", "/home"];
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

pub(super) fn list_disks() -> Result<Vec<Disk>, BlockError> {
    let entries =
        fs::read_dir("/sys/block").map_err(|error| BlockError::List(error.to_string()))?;
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
        let system = mount_points
            .iter()
            .any(|point| SYSTEM_MOUNTS.contains(&point.as_str()))
            || nodes.iter().any(|node| swaps.contains(node));

        disks.push(Disk {
            path: PathBuf::from(format!("/dev/{name}")),
            size_bytes: sectors * 512,
            vendor: read_trimmed(&sys.join("device/vendor")),
            model: read_trimmed(&sys.join("device/model")),
            serial: read_trimmed(&sys.join("device/serial")),
            removable: read_trimmed(&sys.join("removable")).as_deref() == Some("1"),
            usb,
            system,
            mount_points,
            name,
        });
    }
    disks.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(disks)
}

pub(super) fn prepare_for_write(disk: &Disk) -> Result<Restore, BlockError> {
    // Deepest mounts first so nested mounts come off before their parents.
    let mut points = disk.mount_points.clone();
    points.sort_by_key(|point| std::cmp::Reverse(point.len()));
    for point in points {
        nix::mount::umount(point.as_str())
            .map_err(|error| BlockError::Busy(disk.name.clone(), format!("{point}: {error}")))?;
    }
    Ok(Restore::none())
}

pub(super) fn open_for_write(disk: &Disk) -> Result<File, BlockError> {
    // O_EXCL on a block device fails if anything still has it mounted.
    OpenOptions::new()
        .write(true)
        .custom_flags(nix::libc::O_EXCL)
        .open(&disk.path)
        .map_err(|error| BlockError::io(&disk.path, error))
}

pub(super) fn open_for_verify(disk: &Disk) -> Result<File, BlockError> {
    let file = File::open(&disk.path).map_err(|error| BlockError::io(&disk.path, error))?;
    // Drop cached pages so the read-back comes from the device.
    let _ = nix::fcntl::posix_fadvise(
        &file,
        0,
        0,
        nix::fcntl::PosixFadviseAdvice::POSIX_FADV_DONTNEED,
    );
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mount_escapes_are_decoded() {
        assert_eq!(unescape_mount("/media/My\\040Card"), "/media/My Card");
        assert_eq!(unescape_mount("/plain"), "/plain");
    }

    #[test]
    fn listing_marks_the_root_disk_as_system() {
        let disks = list_disks().unwrap();
        let root_mounted: Vec<&Disk> = disks
            .iter()
            .filter(|disk| disk.mount_points.iter().any(|point| point == "/"))
            .collect();
        assert!(root_mounted.iter().all(|disk| disk.system));
    }
}
