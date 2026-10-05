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
    parts.sort();
    parts
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
            // "Mounted /dev/sdc1 at /run/media/user/BOOT"
            if let Some((_, point)) = output.trim().rsplit_once(" at ") {
                points.push(PathBuf::from(point.trim_end_matches('.')));
            }
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
