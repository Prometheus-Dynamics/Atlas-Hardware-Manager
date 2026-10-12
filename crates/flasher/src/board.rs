//! What the board is: its serial, model and eMMC, from the kernel.

use std::fs;
use std::path::{Path, PathBuf};

use atlas_flasher_proto::Emmc;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Board {
    pub serial: String,
    pub model: String,
    pub revision: Option<String>,
}

fn read_trimmed(path: impl AsRef<Path>) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let text = text.trim_matches(|c: char| c.is_whitespace() || c == '\0');
    (!text.is_empty()).then(|| text.to_string())
}

fn cpuinfo(field: &str) -> Option<String> {
    let text = fs::read_to_string("/proc/cpuinfo").ok()?;
    text.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        (name.trim() == field)
            .then(|| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

impl Board {
    /// This board, from the device tree and /proc/cpuinfo.
    pub fn detect() -> Self {
        Self {
            serial: read_trimmed("/proc/device-tree/serial-number")
                .or_else(|| cpuinfo("Serial"))
                .unwrap_or_else(|| "unknown".into()),
            model: read_trimmed("/proc/device-tree/model").unwrap_or_else(|| "unknown".into()),
            revision: cpuinfo("Revision"),
        }
    }
}

/// The eMMC behind `disk` (`/dev/mmcblk0`), from /sys/block. Its size
/// comes from the device itself when sysfs has no entry (a test file).
pub fn emmc(disk: &Path) -> Emmc {
    let name = disk
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let block = PathBuf::from("/sys/block").join(&name);
    let device = block.join("device");
    let size = read_trimmed(block.join("size"))
        .and_then(|sectors| sectors.parse::<u64>().ok())
        .map(|sectors| sectors * 512)
        .or_else(|| fs::metadata(disk).ok().map(|meta| meta.len()))
        .unwrap_or(0);
    Emmc {
        size,
        name: read_trimmed(device.join("name")),
        manfid: read_trimmed(device.join("manfid")),
        life_time: read_trimmed(device.join("life_time")),
        pre_eol_info: read_trimmed(device.join("pre_eol_info")),
    }
}
