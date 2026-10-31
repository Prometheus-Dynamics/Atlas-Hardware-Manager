//! A board's attached UBI volumes, from sysfs, and writing a slot image into
//! one: the update ioctl, the bytes streamed with their SHA-256, then read
//! back from the volume and compared.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// One volume of an attached UBI device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttachedVolume {
    /// `ubi0_1`
    pub node: String,
    pub name: String,
    /// The bytes it can hold: reserved LEBs times the usable LEB size.
    pub capacity: u64,
}

fn read_number(path: &Path) -> Option<u64> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// The volumes under `sysfs` (`/sys/class/ubi` on a board).
pub fn attached_volumes(sysfs: &Path) -> Vec<AttachedVolume> {
    let mut volumes: Vec<AttachedVolume> = std::fs::read_dir(sysfs)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let node = entry.file_name().to_string_lossy().into_owned();
            // ubi0_1 is a volume; ubi0 the device, ubi_ctrl the control node.
            let (device, index) = node.strip_prefix("ubi")?.split_once('_')?;
            if device.parse::<u32>().is_err() || index.parse::<u32>().is_err() {
                return None;
            }
            let dir = entry.path();
            let name = std::fs::read_to_string(dir.join("name"))
                .ok()?
                .trim()
                .to_string();
            let lebs = read_number(&dir.join("reserved_ebs"))?;
            let leb = read_number(&dir.join("usable_eb_size"))?;
            Some(AttachedVolume {
                node,
                name,
                capacity: lebs * leb,
            })
        })
        .collect();
    volumes.sort_by(|a, b| a.node.cmp(&b.node));
    volumes
}

/// What a slot write did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Written {
    pub volume: String,
    pub bytes: u64,
    pub sha256: String,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Copies `bytes` from `source` into `sink`, hashing them.
pub fn copy_hashed(
    source: &mut dyn Read,
    sink: &mut dyn Write,
    bytes: u64,
    progress: &mut dyn FnMut(u64),
) -> Result<String, String> {
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    let mut left = bytes;
    while left > 0 {
        let take = left.min(buffer.len() as u64) as usize;
        source
            .read_exact(&mut buffer[..take])
            .map_err(|error| format!("reading the image: {error}"))?;
        hasher.update(&buffer[..take]);
        sink.write_all(&buffer[..take])
            .map_err(|error| format!("writing the volume: {error}"))?;
        left -= take as u64;
        progress(bytes - left);
    }
    Ok(hex(&hasher.finalize()))
}

/// The SHA-256 of the first `bytes` of `source`.
pub fn hash_prefix(source: &mut dyn Read, bytes: u64) -> Result<String, String> {
    copy_hashed(source, &mut std::io::sink(), bytes, &mut |_| {})
}

/// Writes `bytes` of `image` into the volume named `name` (under `sysfs`,
/// its node under `dev`), refusing one too small, then reads it back. With
/// `expected`, the image's SHA-256 must match it too.
#[cfg(target_os = "linux")]
pub fn write_volume(
    sysfs: &Path,
    dev: &Path,
    name: &str,
    image: &mut dyn Read,
    bytes: u64,
    expected: Option<&str>,
    progress: &mut dyn FnMut(u64),
) -> Result<Written, String> {
    let volumes = attached_volumes(sysfs);
    let volume = volumes
        .iter()
        .find(|volume| volume.name == name)
        .ok_or_else(|| format!("no attached UBI volume named {name}"))?;
    if bytes > volume.capacity {
        return Err(format!(
            "the image is {bytes} bytes; volume {name} holds {}",
            volume.capacity
        ));
    }
    let path: PathBuf = dev.join(&volume.node);
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    crate::sys::start_update(&file, bytes)?;
    let sha256 = copy_hashed(image, &mut file, bytes, progress)?;
    file.sync_all()
        .map_err(|error| format!("{}: {error}", path.display()))?;
    drop(file);
    if let Some(expected) = expected
        && !expected.eq_ignore_ascii_case(&sha256)
    {
        return Err(format!(
            "the image's SHA-256 is {sha256}, not {expected}; volume {name} now holds it and must not be booted"
        ));
    }
    let mut readback =
        std::fs::File::open(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    let back = hash_prefix(&mut readback, bytes)?;
    if back != sha256 {
        return Err(format!(
            "volume {name} reads back {back}, not the {sha256} written"
        ));
    }
    Ok(Written {
        volume: name.to_string(),
        bytes,
        sha256,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volumes_come_from_sysfs() {
        let root = std::env::temp_dir().join(format!("atlas-ubi-sysfs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (node, name, lebs) in [("ubi0_1", "rootfs_b", "40"), ("ubi0_0", "rootfs_a", "40")] {
            let dir = root.join(node);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("name"), format!("{name}\n")).unwrap();
            std::fs::write(dir.join("reserved_ebs"), lebs).unwrap();
            std::fs::write(dir.join("usable_eb_size"), "126976\n").unwrap();
        }
        std::fs::create_dir_all(root.join("ubi0")).unwrap();
        std::fs::create_dir_all(root.join("ubi_ctrl")).unwrap();
        let volumes = attached_volumes(&root);
        assert_eq!(
            volumes,
            [
                AttachedVolume {
                    node: "ubi0_0".into(),
                    name: "rootfs_a".into(),
                    capacity: 40 * 126_976
                },
                AttachedVolume {
                    node: "ubi0_1".into(),
                    name: "rootfs_b".into(),
                    capacity: 40 * 126_976
                },
            ]
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_copy_hashes_what_it_writes() {
        let data = vec![7u8; 3_000_000];
        let mut sink = Vec::new();
        let mut last = 0;
        let sha = copy_hashed(&mut &data[..], &mut sink, data.len() as u64, &mut |done| {
            last = done
        })
        .unwrap();
        assert_eq!(sink, data);
        assert_eq!(last, 3_000_000);
        assert_eq!(sha, hash_prefix(&mut &data[..], 3_000_000).unwrap());
        assert!(copy_hashed(&mut &data[..10], &mut sink, 11, &mut |_| {}).is_err());
    }
}
