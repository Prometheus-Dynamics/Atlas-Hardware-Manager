//! Windows: `Get-Disk` for listing, offline/online around raw writes to
//! `\\.\PhysicalDriveN`. No unsafe Win32 calls are needed.

use std::fs::{File, OpenOptions};
use std::path::PathBuf;
use std::process::Command;

use serde::Deserialize;
use serde_json::Value;

use super::Restore;
use crate::{BlockError, Disk};

/// `MSFT_Disk.BusType` value for USB.
const BUS_TYPE_USB: u64 = 7;

fn powershell(script: &str) -> Result<String, BlockError> {
    let output = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()
        .map_err(|error| BlockError::List(format!("powershell: {error}")))?;
    if !output.status.success() {
        return Err(BlockError::List(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// PowerShell emits one object bare and several as an array.
fn as_list(value: Value) -> Vec<Value> {
    match value {
        Value::Array(items) => items,
        Value::Null => Vec::new(),
        other => vec![other],
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct GetDisk {
    number: u32,
    friendly_name: Option<String>,
    serial_number: Option<String>,
    size: Option<u64>,
    bus_type: Option<Value>,
    is_boot: Option<bool>,
    is_system: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct GetPartition {
    disk_number: u32,
    drive_letter: Option<Value>,
}

fn drive_letter(value: &Value) -> Option<String> {
    let letter = match value {
        Value::String(text) => text.trim().chars().next(),
        Value::Number(code) => code.as_u64().and_then(|code| char::from_u32(code as u32)),
        _ => None,
    }?;
    letter
        .is_ascii_alphabetic()
        .then(|| format!("{}:\\", letter.to_ascii_uppercase()))
}

pub(super) fn list_disks() -> Result<Vec<Disk>, BlockError> {
    let disks_json = powershell(
        "Get-Disk | Select-Object Number,FriendlyName,SerialNumber,Size,BusType,IsBoot,IsSystem \
         | ConvertTo-Json -Compress",
    )?;
    let partitions_json = powershell(
        "Get-Partition | Select-Object DiskNumber,DriveLetter | ConvertTo-Json -Compress",
    )
    .unwrap_or_default();

    let disks: Vec<GetDisk> = as_list(serde_json::from_str(&disks_json).unwrap_or(Value::Null))
        .into_iter()
        .filter_map(|value| serde_json::from_value(value).ok())
        .collect();
    let partitions: Vec<GetPartition> =
        as_list(serde_json::from_str(&partitions_json).unwrap_or(Value::Null))
            .into_iter()
            .filter_map(|value| serde_json::from_value(value).ok())
            .collect();

    Ok(disks
        .into_iter()
        .map(|disk| {
            let usb = match &disk.bus_type {
                Some(Value::Number(code)) => code.as_u64() == Some(BUS_TYPE_USB),
                Some(Value::String(text)) => text.eq_ignore_ascii_case("usb"),
                _ => false,
            };
            let mount_points = partitions
                .iter()
                .filter(|part| part.disk_number == disk.number)
                .filter_map(|part| part.drive_letter.as_ref().and_then(drive_letter))
                .collect();
            let name = format!("PhysicalDrive{}", disk.number);
            Disk {
                path: PathBuf::from(format!(r"\\.\{name}")),
                name,
                size_bytes: disk.size.unwrap_or(0),
                vendor: None,
                model: disk.friendly_name,
                serial: disk.serial_number.map(|serial| serial.trim().to_string()),
                removable: usb,
                usb,
                system: disk.is_boot.unwrap_or(false) || disk.is_system.unwrap_or(false),
                mount_points,
            }
        })
        .collect())
}

fn disk_number(disk: &Disk) -> Result<u32, BlockError> {
    disk.name
        .strip_prefix("PhysicalDrive")
        .and_then(|number| number.parse().ok())
        .ok_or_else(|| BlockError::NotFound(disk.name.clone()))
}

pub(super) fn prepare_for_write(disk: &Disk) -> Result<Restore, BlockError> {
    let number = disk_number(disk)?;
    // Offline disks have no mounted volumes, so raw writes are not blocked.
    powershell(&format!("Set-Disk -Number {number} -IsOffline $true"))
        .map_err(|error| BlockError::Busy(disk.name.clone(), error.to_string()))?;
    Ok(Restore::with(move || {
        let _ = powershell(&format!(
            "Set-Disk -Number {number} -IsOffline $false; Update-HostStorageCache"
        ));
    }))
}

pub(super) fn open_for_write(disk: &Disk) -> Result<File, BlockError> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(&disk.path)
        .map_err(|error| BlockError::io(&disk.path, error))
}

pub(super) fn open_for_verify(disk: &Disk) -> Result<File, BlockError> {
    File::open(&disk.path).map_err(|error| BlockError::io(&disk.path, error))
}
