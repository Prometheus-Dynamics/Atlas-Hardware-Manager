//! macOS: `diskutil` plists for disks; raw `/dev/rdiskN` nodes for I/O.

use std::fs::{File, OpenOptions};
use std::path::PathBuf;
use std::process::Command;

use plist::Value;

use super::Restore;
use crate::{BlockError, Disk};

fn diskutil_plist(args: &[&str]) -> Result<Value, BlockError> {
    let output = Command::new("diskutil")
        .args(args)
        .output()
        .map_err(|error| BlockError::List(format!("diskutil: {error}")))?;
    if !output.status.success() {
        return Err(BlockError::List(format!(
            "diskutil {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Value::from_reader(std::io::Cursor::new(output.stdout))
        .map_err(|error| BlockError::List(format!("diskutil output: {error}")))
}

fn text(dict: &plist::Dictionary, key: &str) -> Option<String> {
    dict.get(key)
        .and_then(Value::as_string)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn flag(dict: &plist::Dictionary, key: &str) -> bool {
    dict.get(key).and_then(Value::as_boolean).unwrap_or(false)
}

pub(super) fn list_disks() -> Result<Vec<Disk>, BlockError> {
    let listing = diskutil_plist(&["list", "-plist", "physical"])?;
    let whole = listing
        .as_dictionary()
        .and_then(|dict| dict.get("WholeDisks"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let all = listing
        .as_dictionary()
        .and_then(|dict| dict.get("AllDisksAndPartitions"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut disks = Vec::new();
    for name in whole.iter().filter_map(Value::as_string) {
        let info = diskutil_plist(&["info", "-plist", name])?;
        let Some(info) = info.as_dictionary() else {
            continue;
        };
        let mut mount_points = Vec::new();
        if let Some(entry) = all
            .iter()
            .filter_map(Value::as_dictionary)
            .find(|entry| entry.get("DeviceIdentifier").and_then(Value::as_string) == Some(name))
        {
            if let Some(point) = text(entry, "MountPoint") {
                mount_points.push(point);
            }
            for partition in entry
                .get("Partitions")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_dictionary)
            {
                if let Some(point) = text(partition, "MountPoint") {
                    mount_points.push(point);
                }
            }
        }
        let internal = flag(info, "Internal");
        let usb = text(info, "BusProtocol").as_deref() == Some("USB");
        let system = mount_points
            .iter()
            .any(|point| point == "/" || point.starts_with("/System"));
        disks.push(Disk {
            path: PathBuf::from(format!("/dev/r{name}")),
            name: name.to_string(),
            size_bytes: info
                .get("TotalSize")
                .or_else(|| info.get("Size"))
                .and_then(Value::as_unsigned_integer)
                .unwrap_or(0),
            vendor: None,
            model: text(info, "MediaName"),
            serial: None,
            removable: flag(info, "Removable")
                || flag(info, "RemovableMedia")
                || flag(info, "Ejectable"),
            usb,
            usb_port: None,
            system: system || (internal && !usb),
            mount_points,
        });
    }
    Ok(disks)
}

pub(super) fn prepare_for_write(disk: &Disk) -> Result<Restore, BlockError> {
    let output = Command::new("diskutil")
        .args(["unmountDisk", "force", &format!("/dev/{}", disk.name)])
        .output()
        .map_err(|error| BlockError::Busy(disk.name.clone(), error.to_string()))?;
    if !output.status.success() {
        return Err(BlockError::Busy(
            disk.name.clone(),
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    Ok(Restore::none())
}

pub(super) fn open_for_write(disk: &Disk) -> Result<File, BlockError> {
    OpenOptions::new()
        .write(true)
        .open(&disk.path)
        .map_err(|error| BlockError::io(&disk.path, error))
}

pub(super) fn open_for_verify(disk: &Disk) -> Result<File, BlockError> {
    // The raw node is uncached, so this reads from the device.
    File::open(&disk.path).map_err(|error| BlockError::io(&disk.path, error))
}
