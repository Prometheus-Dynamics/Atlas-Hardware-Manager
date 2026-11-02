//! Disk operations the logged-in user may do without elevation, through the
//! desktop's own disk service: eject a disk safely after writing it (so the
//! desktop can't auto-mount and dirty the fresh filesystems), mount one
//! read-only to look inside, and open a folder in the file manager.
//!
//! - Linux: `udisksctl` (UDisks2) and `xdg-open`.
//! - macOS: `diskutil` and `open`.
//! - Windows: not yet; ejecting is skipped and mounting is left to Explorer.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{BlockError, Disk};

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn run(program: &str, args: &[&str]) -> Result<String, BlockError> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|error| BlockError::Desktop(format!("{program}: {error}")))?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if output.status.success() {
        Ok(stdout)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(BlockError::Desktop(format!(
            "{program} {}: {}",
            args.join(" "),
            stderr.trim()
        )))
    }
}

/// Partition nodes of a disk, `/dev/sdc1` and so on.
#[cfg(target_os = "linux")]
fn partitions(disk: &Disk) -> Vec<String> {
    let mut parts: Vec<String> = std::fs::read_dir(Path::new("/sys/block").join(&disk.name))
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(&disk.name) && name != &disk.name)
        .map(|name| format!("/dev/{name}"))
        .collect();
    // By partition number: sdX10 sorts after sdX9.
    parts.sort_by_key(|name| {
        let digits = name.len() - name.trim_end_matches(|c: char| c.is_ascii_digit()).len();
        (
            name[name.len() - digits..].parse::<u32>().unwrap_or(0),
            name.clone(),
        )
    });
    parts
}

/// The mount points of `parts` in a /proc/mounts text, as the kernel
/// escapes them (`\040` for a space) decoded.
#[cfg(target_os = "linux")]
fn mounts_of(mounts: &str, parts: &[String]) -> Vec<String> {
    mounts
        .lines()
        .filter_map(|line| {
            let mut fields = line.split(' ');
            let source = fields.next()?;
            let point = fields.next()?;
            parts.iter().any(|part| part == source).then(|| unescape(point))
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn unescape(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\'
            && i + 3 < bytes.len()
            && let Ok(byte) = u8::from_str_radix(&field[i + 1..i + 4], 8)
        {
            out.push(byte);
            i += 4;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Unmounts everything on the disk, keeping it attached (a Pi gadget disk
/// stays reachable for its console). Best effort: what the desktop mounted
/// is unmounted, what fails is left. Without UDisks (a controller's own
/// automounter, as on a SystemCore after it restarts), what is still
/// mounted is unmounted directly, which needs root.
pub fn unmount_all(disk: &Disk) -> Result<(), BlockError> {
    #[cfg(target_os = "linux")]
    {
        let parts = partitions(disk);
        for part in &parts {
            let _ = run(
                "udisksctl",
                &["unmount", "--no-user-interaction", "-b", part],
            );
        }
        let mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
        for point in mounts_of(&mounts, &parts) {
            let _ = nix::mount::umount2(point.as_str(), nix::mount::MntFlags::empty());
        }
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        let node = disk
            .path
            .to_string_lossy()
            .replace("/dev/rdisk", "/dev/disk");
        run("diskutil", &["unmountDisk", &node]).map(|_| ())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = disk;
        Ok(())
    }
}

/// Unmounts everything on the disk and powers it off, so nothing can mount
/// it again until it is replugged. Best effort per partition; fails only
/// when the disk could not be powered off.
pub fn eject(disk: &Disk) -> Result<(), BlockError> {
    #[cfg(target_os = "linux")]
    {
        // The desktop may still be mounting what the write just exposed.
        for attempt in 0..5 {
            for part in partitions(disk) {
                let _ = run(
                    "udisksctl",
                    &["unmount", "--no-user-interaction", "-b", &part],
                );
            }
            let node = disk.path.to_string_lossy();
            match run(
                "udisksctl",
                &["power-off", "--no-user-interaction", "-b", &node],
            ) {
                Ok(_) => return Ok(()),
                Err(error) if attempt == 4 => return Err(error),
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(600)),
            }
        }
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        let node = disk
            .path
            .to_string_lossy()
            .replace("/dev/rdisk", "/dev/disk");
        run("diskutil", &["eject", &node]).map(|_| ())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = disk;
        Ok(())
    }
}

/// Mounts every partition read-only and returns where they landed.
pub fn mount_read_only(disk: &Disk) -> Result<Vec<PathBuf>, BlockError> {
    #[cfg(target_os = "linux")]
    {
        let mut points = Vec::new();
        for part in partitions(disk) {
            // Already mounted (say, by the desktop): reuse that.
            let existing =
                run("findmnt", &["-n", "-o", "TARGET", "--source", &part]).unwrap_or_default();
            if let Some(point) = existing.lines().next().filter(|line| !line.is_empty()) {
                points.push(PathBuf::from(point.trim()));
                continue;
            }
            let output = run(
                "udisksctl",
                &["mount", "--no-user-interaction", "-b", &part, "-o", "ro"],
            )?;
            points.push(mounted_at(&output)?);
        }
        Ok(points)
    }
    #[cfg(target_os = "macos")]
    {
        let node = disk
            .path
            .to_string_lossy()
            .replace("/dev/rdisk", "/dev/disk");
        run("diskutil", &["mountDisk", "readOnly", &node])?;
        Ok(Vec::new())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = disk;
        Err(BlockError::Desktop(
            "mounting is left to Windows Explorer on this system".into(),
        ))
    }
}

/// Opens a folder in the desktop's file manager.
pub fn open_folder(path: &Path) -> Result<(), BlockError> {
    let target = path.to_string_lossy();
    #[cfg(target_os = "linux")]
    let program = "xdg-open";
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(target_os = "windows")]
    let program = "explorer";
    Command::new(program)
        .arg(target.as_ref())
        .spawn()
        .map(|_| ())
        .map_err(|error| BlockError::Desktop(format!("{program}: {error}")))
}

/// Writes `contents` to `relative` (for example `board/authorized_keys`)
/// on the disk's first partition, the boot partition of a Pi image. Mounts
/// it read-write for the write when nothing has it mounted, and unmounts it
/// again; reuses a read-write mount the desktop already made.
pub fn write_boot_file(disk: &Disk, relative: &str, contents: &str) -> Result<(), BlockError> {
    #[cfg(target_os = "linux")]
    {
        let part = partitions(disk)
            .into_iter()
            .next()
            .ok_or_else(|| BlockError::Desktop(format!("{} has no partitions", disk.name)))?;
        let options =
            run("findmnt", &["-n", "-o", "OPTIONS", "--source", &part]).unwrap_or_default();
        let mounted =
            run("findmnt", &["-n", "-o", "TARGET", "--source", &part]).unwrap_or_default();
        let existing = mounted
            .lines()
            .next()
            .map(str::trim)
            .filter(|point| !point.is_empty());
        let read_write = options.split(',').any(|option| option.trim() == "rw");
        let (point, ours) = match existing {
            Some(point) if read_write => (PathBuf::from(point), false),
            Some(_) => {
                // Mounted read-only (say, by Browse files): remount it ours.
                run(
                    "udisksctl",
                    &["unmount", "--no-user-interaction", "-b", &part],
                )?;
                (mount_rw(&part)?, true)
            }
            None => (mount_rw(&part)?, true),
        };
        let target = point.join(relative);
        let result = (|| {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|error| BlockError::io(parent, error))?;
            }
            std::fs::write(&target, contents).map_err(|error| BlockError::io(&target, error))?;
            let file =
                std::fs::File::open(&target).map_err(|error| BlockError::io(&target, error))?;
            file.sync_all()
                .map_err(|error| BlockError::io(&target, error))
        })();
        if ours {
            let _ = run(
                "udisksctl",
                &["unmount", "--no-user-interaction", "-b", &part],
            );
        }
        result
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (disk, relative, contents);
        Err(BlockError::Desktop(
            "writing to the boot partition is only supported on Linux so far".into(),
        ))
    }
}

/// Reads `relative` from the disk's boot partition, if it is there.
pub fn read_boot_file(disk: &Disk, relative: &str) -> Result<Option<String>, BlockError> {
    #[cfg(target_os = "linux")]
    {
        let part = partitions(disk)
            .into_iter()
            .next()
            .ok_or_else(|| BlockError::Desktop(format!("{} has no partitions", disk.name)))?;
        let mounted =
            run("findmnt", &["-n", "-o", "TARGET", "--source", &part]).unwrap_or_default();
        let (point, ours) = match mounted
            .lines()
            .next()
            .map(str::trim)
            .filter(|p| !p.is_empty())
        {
            Some(point) => (PathBuf::from(point), false),
            None => {
                let output = run(
                    "udisksctl",
                    &["mount", "--no-user-interaction", "-b", &part, "-o", "ro"],
                )?;
                (mounted_at(&output)?, true)
            }
        };
        let text = std::fs::read_to_string(point.join(relative)).ok();
        if ours {
            let _ = run(
                "udisksctl",
                &["unmount", "--no-user-interaction", "-b", &part],
            );
        }
        Ok(text)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (disk, relative);
        Ok(None)
    }
}

#[cfg(target_os = "linux")]
fn mount_rw(part: &str) -> Result<PathBuf, BlockError> {
    let output = run("udisksctl", &["mount", "--no-user-interaction", "-b", part])?;
    mounted_at(&output)
}

/// The mount point in udisksctl's "Mounted /dev/sdc1 at /run/media/u/BOOT".
#[cfg(target_os = "linux")]
fn mounted_at(output: &str) -> Result<PathBuf, BlockError> {
    output
        .trim()
        .rsplit_once(" at ")
        .map(|(_, point)| PathBuf::from(point.trim_end_matches('.')))
        .ok_or_else(|| BlockError::Desktop(format!("unexpected udisksctl output: {output}")))
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn a_disks_mounts_are_found_with_their_points_decoded() {
        let mounts = "/dev/mmcblk0p5 / ext4 rw 0 0\n\
                      /dev/sda1 /V vfat rw,relatime 0 0\n\
                      /dev/sda2 /media/my\\040disk ext4 rw 0 0\n\
                      /dev/sdb1 /W vfat rw 0 0\n";
        let parts = vec!["/dev/sda1".to_string(), "/dev/sda2".to_string()];
        assert_eq!(mounts_of(mounts, &parts), ["/V", "/media/my disk"]);
    }
}
