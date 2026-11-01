//! A raw-flash board's image as a set of partition images (Gaia's SPI-NAND
//! boards: `env.img`, `idblock.img`, `uboot.img`, `boot.img`, `ubi.img`),
//! laid out by the `mtdparts` in the U-Boot environment it ships (`env.img`),
//! the same table the board's U-Boot and kernel then use. A flash writes the
//! factory `env.img` (slot A, both copies); the `env-*.img` beside it are for
//! the board's slot installs (`atlas-ubi nand-slot`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The environment file a partition set is recognised by.
pub const ENV_FILE: &str = "env.img";

/// A U-Boot environment image's variables: a CRC-32 of the rest of its copy
/// (little-endian), then NUL-separated `name=value` strings. The image may be
/// one copy, or two of equal size (Gaia's NAND boards: its 128K copies, read
/// from the first); a redundant copy has a flag byte first.
pub fn uboot_env(bytes: &[u8]) -> Result<BTreeMap<String, String>, String> {
    if bytes.len() < 8 {
        return Err("the environment is too short".into());
    }
    let stored = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let (start, bytes) = [bytes, &bytes[..bytes.len() / 2]]
        .into_iter()
        .flat_map(|copy| [4usize, 5].map(|start| (start, copy)))
        .find(|(start, copy)| atlas_ubi::crc::crc32(&copy[*start..]) == stored)
        .ok_or("the environment's CRC doesn't match its contents")?;
    Ok(bytes[start..]
        .split(|byte| *byte == 0)
        .take_while(|entry| !entry.is_empty())
        .filter_map(|entry| {
            let text = String::from_utf8_lossy(entry);
            let (name, value) = text.split_once('=')?;
            Some((name.to_string(), value.to_string()))
        })
        .collect())
}

/// One partition of an `mtdparts` table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Part {
    pub name: String,
    pub offset: u64,
    /// `None`: the rest of the flash (`-`).
    pub size: Option<u64>,
}

fn size(text: &str) -> Result<u64, String> {
    let (number, unit) = match text
        .char_indices()
        .find(|(_, c)| !c.is_ascii_hexdigit() && *c != 'x')
    {
        Some((at, _)) => text.split_at(at),
        None => (text, ""),
    };
    let value = if let Some(hex) = number.strip_prefix("0x") {
        u64::from_str_radix(hex, 16)
    } else {
        number.parse()
    }
    .map_err(|_| format!("size {text:?}"))?;
    let scale = match unit {
        "" => 1,
        "k" | "K" => 1 << 10,
        "m" | "M" => 1 << 20,
        "g" | "G" => 1 << 30,
        _ => return Err(format!("size {text:?}")),
    };
    Ok(value * scale)
}

/// The partitions of `mtdparts=<device>:<size>[@<offset>](<name>),...`
/// (Linux's cmdlinepart syntax); one without an offset follows the last.
pub fn mtdparts(value: &str) -> Result<Vec<Part>, String> {
    let (_, list) = value
        .split_once(':')
        .ok_or_else(|| format!("mtdparts {value:?} names no device"))?;
    let mut parts = Vec::new();
    let mut next = 0u64;
    for spec in list.split(',') {
        let (geometry, name) = spec
            .split_once('(')
            .and_then(|(geometry, rest)| Some((geometry, rest.strip_suffix(')')?)))
            .ok_or_else(|| format!("partition {spec:?} has no (name)"))?;
        let (length, offset) = match geometry.split_once('@') {
            Some((length, offset)) => (length, Some(size(offset)?)),
            None => (geometry, None),
        };
        let offset = offset.unwrap_or(next);
        let length = (length != "-").then(|| size(length)).transpose()?;
        next = length.map_or(u64::MAX, |length| offset + length);
        parts.push(Part {
            name: name.to_string(),
            offset,
            size: length,
        });
    }
    Ok(parts)
}

/// A partition and the image Gaia built for it, if any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Planned {
    pub part: Part,
    pub image: Option<PathBuf>,
    pub bytes: u64,
}

/// The partition set next to `image` (in its directory): `None` when there's
/// no `env.img` with an `mtdparts` table there.
pub fn partition_set(image: &Path) -> Option<Result<Vec<Planned>, String>> {
    let dir = image.parent()?;
    let env = std::fs::read(dir.join(ENV_FILE)).ok()?;
    let table = match uboot_env(&env) {
        Ok(variables) => variables.get("mtdparts")?.clone(),
        Err(error) => return Some(Err(format!("{ENV_FILE}: {error}"))),
    };
    Some(mtdparts(&table).and_then(|parts| {
        parts
            .into_iter()
            .map(|part| {
                let path = dir.join(format!("{}.img", part.name));
                let bytes = std::fs::metadata(&path).map(|meta| meta.len()).ok();
                if let (Some(bytes), Some(size)) = (bytes, part.size)
                    && bytes > size
                {
                    return Err(format!(
                        "{}.img is {bytes} bytes; its partition is {size}",
                        part.name
                    ));
                }
                Ok(Planned {
                    image: bytes.map(|_| path),
                    bytes: bytes.unwrap_or(0),
                    part,
                })
            })
            .collect()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str =
        "rk-nand:256K(env),256K@256K(idblock),512K(uboot),6M(boot),6M(boot_b),-(ubi)";

    #[test]
    fn gaias_luckfox_table() {
        let parts = mtdparts(TABLE).unwrap();
        let at: Vec<(&str, u64, Option<u64>)> = parts
            .iter()
            .map(|p| (p.name.as_str(), p.offset, p.size))
            .collect();
        assert_eq!(
            at,
            [
                ("env", 0, Some(256 << 10)),
                ("idblock", 256 << 10, Some(256 << 10)),
                ("uboot", 512 << 10, Some(512 << 10)),
                ("boot", 1 << 20, Some(6 << 20)),
                ("boot_b", 7 << 20, Some(6 << 20)),
                ("ubi", 13 << 20, None),
            ]
        );
        assert!(mtdparts("no-colon").is_err());
        assert!(mtdparts("d:1M").is_err());
        assert_eq!(
            mtdparts("d:0x100000@0x200000(a)").unwrap()[0].offset,
            0x20_0000
        );
    }

    fn env_image(text: &str) -> Vec<u8> {
        let mut env = vec![0u8; 4096];
        let body: Vec<u8> = text.replace('\n', "\0").into_bytes();
        env[4..4 + body.len()].copy_from_slice(&body);
        let crc = atlas_ubi::crc::crc32(&env[4..]);
        env[..4].copy_from_slice(&crc.to_le_bytes());
        env
    }

    #[test]
    fn the_environment_and_the_set_beside_an_image() {
        let env = env_image(&format!("mtdparts={TABLE}\nsys_bootargs=ubi.mtd=5\n"));
        let variables = uboot_env(&env).unwrap();
        assert_eq!(variables["sys_bootargs"], "ubi.mtd=5");
        // Two copies, as Gaia's NAND factory image is, read from the first.
        let mut twice = env.clone();
        twice.extend_from_slice(&env);
        assert_eq!(uboot_env(&twice).unwrap()["sys_bootargs"], "ubi.mtd=5");
        let mut damaged = env.clone();
        damaged[10] ^= 1;
        assert!(uboot_env(&damaged).unwrap_err().contains("CRC"));

        let dir = std::env::temp_dir().join(format!("atlas-soc-set-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("env.img"), &env).unwrap();
        std::fs::write(dir.join("boot.img"), vec![1u8; 1000]).unwrap();
        std::fs::write(dir.join("ubi.img"), vec![2u8; 2000]).unwrap();
        let set = partition_set(&dir.join("ubi.img")).unwrap().unwrap();
        let files: Vec<(&str, bool, u64)> = set
            .iter()
            .map(|p| (p.part.name.as_str(), p.image.is_some(), p.bytes))
            .collect();
        assert_eq!(
            files,
            [
                ("env", true, 4096),
                ("idblock", false, 0),
                ("uboot", false, 0),
                ("boot", true, 1000),
                ("boot_b", false, 0),
                ("ubi", true, 2000),
            ]
        );
        std::fs::write(dir.join("uboot.img"), vec![0u8; 600 << 10]).unwrap();
        assert!(
            partition_set(&dir.join("ubi.img"))
                .unwrap()
                .unwrap_err()
                .contains("uboot.img")
        );
        assert!(partition_set(Path::new("/nonexistent/x.img")).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}
