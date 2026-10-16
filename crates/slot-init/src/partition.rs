//! Finding a partition by `PARTUUID=` or `LABEL=` without blkid: BusyBox's
//! findfs has no PARTUUID support, and on the board it didn't find the
//! label either. The kernel lists every block device under
//! `/sys/class/block`; a partition has a `partition` file (its number) and
//! sits in its disk's directory. A PARTUUID is the MBR disk signature and
//! the partition number (`5056525a-02`), or the GPT entry's GUID; a label is
//! the ext2/3/4 superblock's volume name.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// What to look for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Spec {
    PartUuid(String),
    Label(String),
}

impl Spec {
    pub fn parse(text: &str) -> Result<Self, String> {
        match text.split_once('=') {
            Some(("PARTUUID", v)) if !v.is_empty() => Ok(Self::PartUuid(v.to_ascii_lowercase())),
            Some(("LABEL", v)) if !v.is_empty() => Ok(Self::Label(v.to_owned())),
            _ => Err(format!("`{text}`: expected PARTUUID=<id> or LABEL=<name>")),
        }
    }
}

/// An MBR disk's PARTUUID for partition `number` (`<signature>-<NN>`),
/// from the disk's first sector. None for a GPT disk (its protective MBR)
/// or no MBR at all.
pub fn mbr_partuuid(sector0: &[u8], number: u32) -> Option<String> {
    if sector0.len() < 512 || sector0[510] != 0x55 || sector0[511] != 0xaa {
        return None;
    }
    if sector0[446 + 4] == 0xee {
        return None;
    }
    let sig = u32::from_le_bytes(sector0[440..444].try_into().ok()?);
    Some(format!("{sig:08x}-{number:02x}"))
}

/// A GUID as GPT stores it (the first three fields little-endian).
pub fn guid_string(b: &[u8]) -> Option<String> {
    let b: &[u8; 16] = b.get(..16)?.try_into().ok()?;
    Some(format!(
        "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
        u16::from_le_bytes([b[4], b[5]]),
        u16::from_le_bytes([b[6], b[7]]),
        b[8],
        b[9],
        b[10],
        b[11],
        b[12],
        b[13],
        b[14],
        b[15]
    ))
}

/// A GPT disk's PARTUUID for partition `number`: `read(offset, len)` reads
/// the disk; `sector` is its logical block size.
pub fn gpt_partuuid(
    read: &mut dyn FnMut(u64, usize) -> Option<Vec<u8>>,
    sector: u64,
    number: u32,
) -> Option<String> {
    let header = read(sector, 92)?;
    if header.get(..8)? != b"EFI PART" || number == 0 {
        return None;
    }
    let entries = u64::from_le_bytes(header.get(72..80)?.try_into().ok()?);
    let count = u32::from_le_bytes(header.get(80..84)?.try_into().ok()?);
    let size = u32::from_le_bytes(header.get(84..88)?.try_into().ok()?);
    if number > count || size < 128 {
        return None;
    }
    let entry = read(
        entries * sector + u64::from(number - 1) * u64::from(size),
        32,
    )?;
    // An unused entry has a zero type GUID.
    if entry.get(..16)?.iter().all(|&b| b == 0) {
        return None;
    }
    guid_string(entry.get(16..32)?)
}

/// The ext2/3/4 volume name from a partition's first 2 KiB.
pub fn ext_label(head: &[u8]) -> Option<String> {
    let sb = head.get(1024..1024 + 136)?;
    if u16::from_le_bytes([sb[56], sb[57]]) != 0xef53 {
        return None;
    }
    let name = &sb[120..136];
    let end = name.iter().position(|&b| b == 0).unwrap_or(name.len());
    let label = std::str::from_utf8(&name[..end]).ok()?;
    (!label.is_empty()).then(|| label.to_owned())
}

fn read_at(path: &Path, offset: u64, len: usize) -> Option<Vec<u8>> {
    let mut file = File::open(path).ok()?;
    file.seek(SeekFrom::Start(offset)).ok()?;
    let mut buf = vec![0u8; len];
    file.read_exact(&mut buf).ok()?;
    Some(buf)
}

/// One partition the kernel lists: its name, number and disk.
struct Part {
    name: String,
    number: u32,
    disk: String,
}

fn partitions(sys: &Path) -> Vec<Part> {
    let class = sys.join("class").join("block");
    let mut found: Vec<Part> = std::fs::read_dir(&class)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let real = std::fs::canonicalize(entry.path()).ok()?;
            let number = std::fs::read_to_string(real.join("partition"))
                .ok()?
                .trim()
                .parse()
                .ok()?;
            let disk = real.parent()?.file_name()?.to_string_lossy().into_owned();
            Some(Part { name, number, disk })
        })
        .collect();
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

/// What one partition is called, for the search and for the report.
fn describe(sys: &Path, dev: &Path, part: &Part) -> (Option<String>, Option<String>) {
    let disk = dev.join(&part.disk);
    let partuuid = read_at(&disk, 0, 512)
        .and_then(|sector0| mbr_partuuid(&sector0, part.number))
        .or_else(|| {
            let sector = std::fs::read_to_string(
                sys.join("block")
                    .join(&part.disk)
                    .join("queue")
                    .join("logical_block_size"),
            )
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(512);
            gpt_partuuid(
                &mut |offset, len| read_at(&disk, offset, len),
                sector,
                part.number,
            )
        });
    let label = read_at(&dev.join(&part.name), 0, 2048).and_then(|head| ext_label(&head));
    (partuuid, label)
}

/// The device of the partition `spec` names (`/dev/mmcblk0p2`), with sysfs
/// at `sys` and the device nodes in `dev`. Fails with every partition it
/// looked at, and what each is called.
pub fn find(spec: &Spec, sys: &Path, dev: &Path) -> Result<PathBuf, String> {
    let parts = partitions(sys);
    let mut seen = Vec::new();
    for part in &parts {
        let (partuuid, label) = describe(sys, dev, part);
        let hit = match spec {
            Spec::PartUuid(want) => partuuid.as_deref() == Some(want.as_str()),
            Spec::Label(want) => label.as_deref() == Some(want.as_str()),
        };
        if hit {
            return Ok(dev.join(&part.name));
        }
        seen.push(format!(
            "{} (partition {} of {}, PARTUUID {}, LABEL {})",
            part.name,
            part.number,
            part.disk,
            partuuid.as_deref().unwrap_or("?"),
            label.as_deref().unwrap_or("-")
        ));
    }
    Err(if seen.is_empty() {
        format!("no partitions under {}", sys.join("class/block").display())
    } else {
        format!("no partition matches; saw {}", seen.join("; "))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mbr(sig: u32) -> Vec<u8> {
        let mut s = vec![0u8; 512];
        s[440..444].copy_from_slice(&sig.to_le_bytes());
        s[446 + 4] = 0x0c;
        s[510] = 0x55;
        s[511] = 0xaa;
        s
    }

    fn ext(label: &str) -> Vec<u8> {
        let mut h = vec![0u8; 2048];
        h[1024 + 56..1024 + 58].copy_from_slice(&0xef53u16.to_le_bytes());
        h[1024 + 120..1024 + 120 + label.len()].copy_from_slice(label.as_bytes());
        h
    }

    #[test]
    fn mbr_partuuids_are_signature_and_number() {
        assert_eq!(
            mbr_partuuid(&mbr(0x5056525a), 2).as_deref(),
            Some("5056525a-02")
        );
        let mut gpt = mbr(1);
        gpt[446 + 4] = 0xee;
        assert_eq!(mbr_partuuid(&gpt, 1), None);
        assert_eq!(mbr_partuuid(&[0u8; 512], 1), None);
    }

    #[test]
    fn gpt_partuuids_come_from_the_entry() {
        let mut disk = vec![0u8; 512 * 4];
        disk[512..520].copy_from_slice(b"EFI PART");
        disk[512 + 72..512 + 80].copy_from_slice(&2u64.to_le_bytes());
        disk[512 + 80..512 + 84].copy_from_slice(&4u32.to_le_bytes());
        disk[512 + 84..512 + 88].copy_from_slice(&128u32.to_le_bytes());
        // Entry 2: a type, and the GUID 01020304-0506-0708-090a-0b0c0d0e0f10.
        let e = 1024 + 128;
        disk[e] = 1;
        let guid = [4, 3, 2, 1, 6, 5, 8, 7, 9, 10, 11, 12, 13, 14, 15, 16];
        disk[e + 16..e + 32].copy_from_slice(&guid);
        let mut read = |o: u64, l: usize| disk.get(o as usize..o as usize + l).map(<[u8]>::to_vec);
        assert_eq!(
            gpt_partuuid(&mut read, 512, 2).as_deref(),
            Some("01020304-0506-0708-090a-0b0c0d0e0f10")
        );
        assert_eq!(gpt_partuuid(&mut read, 512, 1), None, "an unused entry");
        assert_eq!(gpt_partuuid(&mut read, 512, 9), None, "past the table");
    }

    #[test]
    fn ext_labels() {
        assert_eq!(ext_label(&ext("data")).as_deref(), Some("data"));
        assert_eq!(ext_label(&vec![0u8; 2048]), None);
        assert_eq!(ext_label(&ext("")), None);
    }

    #[test]
    fn specs() {
        assert_eq!(
            Spec::parse("PARTUUID=5056525A-02").unwrap(),
            Spec::PartUuid("5056525a-02".into())
        );
        assert_eq!(
            Spec::parse("LABEL=data").unwrap(),
            Spec::Label("data".into())
        );
        assert!(Spec::parse("UUID=x").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn finds_the_partition_in_a_fake_sysfs() {
        use std::os::unix::fs::symlink;
        let root = std::env::temp_dir().join(format!("slot-init-part-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (sys, dev) = (root.join("sys"), root.join("dev"));
        let disk = sys.join("devices/mmc/mmcblk0");
        for (part, n) in [("mmcblk0p1", 1), ("mmcblk0p2", 2)] {
            std::fs::create_dir_all(disk.join(part)).unwrap();
            std::fs::write(disk.join(part).join("partition"), format!("{n}\n")).unwrap();
        }
        // A boot area is a disk of its own: no partition file.
        std::fs::create_dir_all(sys.join("devices/mmc/mmcblk0boot0")).unwrap();
        std::fs::create_dir_all(sys.join("class/block")).unwrap();
        for (name, target) in [
            ("mmcblk0", "../../devices/mmc/mmcblk0"),
            ("mmcblk0p1", "../../devices/mmc/mmcblk0/mmcblk0p1"),
            ("mmcblk0p2", "../../devices/mmc/mmcblk0/mmcblk0p2"),
            ("mmcblk0boot0", "../../devices/mmc/mmcblk0boot0"),
        ] {
            symlink(target, sys.join("class/block").join(name)).unwrap();
        }
        std::fs::create_dir_all(&dev).unwrap();
        std::fs::write(dev.join("mmcblk0"), mbr(0x5056525a)).unwrap();
        std::fs::write(dev.join("mmcblk0p1"), vec![0u8; 2048]).unwrap();
        std::fs::write(dev.join("mmcblk0p2"), ext("data")).unwrap();

        let by_id = find(&Spec::parse("PARTUUID=5056525a-02").unwrap(), &sys, &dev).unwrap();
        assert_eq!(by_id, dev.join("mmcblk0p2"));
        let by_label = find(&Spec::Label("data".into()), &sys, &dev).unwrap();
        assert_eq!(by_label, dev.join("mmcblk0p2"));
        let missing = find(&Spec::PartUuid("5056525a-07".into()), &sys, &dev).unwrap_err();
        assert!(
            missing
                .contains("mmcblk0p2 (partition 2 of mmcblk0, PARTUUID 5056525a-02, LABEL data)"),
            "{missing}"
        );
        assert!(!missing.contains("boot0"), "{missing}");
        let _ = std::fs::remove_dir_all(&root);
    }
}
