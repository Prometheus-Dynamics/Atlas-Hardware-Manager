use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::{Chip, UsbBootError};

const BLOCK: usize = 512;

/// The files a Pi asks for during USB boot, such as the
/// `mass-storage-gadget64` directory from `raspberrypi/usbboot`.
///
/// Lookup follows `rpiboot`: a loose file in `<dir>/<chip folder>/` wins,
/// then the entry in `bootfiles.bin` (or the same tree unpacked, as
/// `<dir>/bootfiles/<chip folder>/`, the raze-flasher bundle's layout), then
/// `<dir>/<name>`.
pub struct BootFiles {
    dir: PathBuf,
    /// `bootfiles.bin`, a tar archive with one folder per chip.
    archive: Option<Vec<u8>>,
    /// Lowercase entry name to (offset, length) in `archive`.
    entries: HashMap<String, (usize, usize)>,
}

/// The EEPROM flashing tool's own name. EEPROM recovery folders may ship it
/// under this name instead of `bootcode*.bin`.
const RECOVERY_TOOL: &str = "recovery.bin";

impl BootFiles {
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, UsbBootError> {
        let dir = dir.into();
        let archive_path = dir.join("bootfiles.bin");
        let archive = match std::fs::read(&archive_path) {
            Ok(data) => Some(data),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(UsbBootError::Io {
                    path: archive_path,
                    message: error.to_string(),
                });
            }
        };
        let entries = match &archive {
            Some(data) => index_tar(data)?,
            None => HashMap::new(),
        };
        let files = Self {
            dir,
            archive,
            entries,
        };
        let has_bootcode = [Chip::Bcm2837, Chip::Bcm2711, Chip::Bcm2712]
            .iter()
            .any(|chip| {
                files.contains(*chip, chip.second_stage_file())
                    || files.contains(*chip, RECOVERY_TOOL)
            });
        if !has_bootcode {
            return Err(UsbBootError::BootFilesMissing(files.dir));
        }
        Ok(files)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The loose files that can hold `name`, in order: the chip override,
    /// the unpacked `bootfiles/` tree, the top level.
    fn loose_candidates(&self, chip: Chip, name: &str) -> [PathBuf; 3] {
        [
            self.dir.join(chip.folder()).join(name),
            self.dir.join("bootfiles").join(chip.folder()).join(name),
            self.dir.join(name),
        ]
    }

    fn contains(&self, chip: Chip, name: &str) -> bool {
        self.archive_entry(chip, name).is_some()
            || self
                .loose_candidates(chip, name)
                .iter()
                .any(|path| path.is_file())
    }

    fn archive_entry(&self, chip: Chip, name: &str) -> Option<&[u8]> {
        let key = format!("{}/{}", chip.folder(), name).to_ascii_lowercase();
        let (offset, length) = *self.entries.get(&key)?;
        self.archive.as_ref()?.get(offset..offset + length)
    }

    /// The contents of `name` for `chip`, or `None` if there is no such file.
    /// Names that try to leave the boot directory are refused.
    pub fn read(&self, chip: Chip, name: &str) -> Result<Option<Vec<u8>>, UsbBootError> {
        let name = name.trim_start_matches('/');
        if name.is_empty() || name.contains("..") || name.contains('\\') || name.contains(':') {
            return Ok(None);
        }
        let [chip_override, unpacked, top_level] = self.loose_candidates(chip, name);
        let read = |path: &Path| {
            std::fs::read(path).map_err(|error| UsbBootError::Io {
                path: path.to_path_buf(),
                message: error.to_string(),
            })
        };
        if chip_override.is_file() {
            return read(&chip_override).map(Some);
        }
        if unpacked.is_file() {
            return read(&unpacked).map(Some);
        }
        if let Some(data) = self.archive_entry(chip, name) {
            return Ok(Some(data.to_vec()));
        }
        if top_level.is_file() {
            return read(&top_level).map(Some);
        }
        Ok(None)
    }

    /// The second-stage bootloader for `chip`: `bootcode*.bin`, or the
    /// EEPROM tool `recovery.bin` in an EEPROM recovery folder.
    pub fn second_stage(&self, chip: Chip) -> Result<Vec<u8>, UsbBootError> {
        if let Some(data) = self.read(chip, chip.second_stage_file())? {
            return Ok(data);
        }
        self.read(chip, RECOVERY_TOOL)?
            .ok_or(UsbBootError::MissingFile {
                name: chip.second_stage_file().into(),
                chip: chip.label(),
            })
    }
}

fn field(header: &[u8], start: usize, len: usize) -> &[u8] {
    let raw = &header[start..start + len];
    let end = raw.iter().position(|byte| *byte == 0).unwrap_or(len);
    &raw[..end]
}

fn parse_octal(raw: &[u8]) -> Option<usize> {
    let text = std::str::from_utf8(raw).ok()?.trim();
    if text.is_empty() {
        return Some(0);
    }
    usize::from_str_radix(text, 8).ok()
}

/// Indexes regular files in a tar archive by lowercase path.
fn index_tar(data: &[u8]) -> Result<HashMap<String, (usize, usize)>, UsbBootError> {
    let mut entries = HashMap::new();
    let mut offset = 0;
    while offset + BLOCK <= data.len() {
        let header = &data[offset..offset + BLOCK];
        if header.iter().all(|byte| *byte == 0) {
            break;
        }
        let name = String::from_utf8_lossy(field(header, 0, 100)).into_owned();
        let prefix = if &header[257..262] == b"ustar" {
            String::from_utf8_lossy(field(header, 345, 155)).into_owned()
        } else {
            String::new()
        };
        let full_name = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let size = parse_octal(field(header, 124, 12))
            .ok_or_else(|| UsbBootError::BadArchive(format!("bad size for {full_name}")))?;
        let data_start = offset + BLOCK;
        data_start
            .checked_add(size)
            .filter(|end| *end <= data.len())
            .ok_or_else(|| UsbBootError::BadArchive(format!("{full_name} runs past the end")))?;
        let type_flag = header[156];
        if type_flag == b'0' || type_flag == 0 {
            entries.insert(
                full_name.trim_start_matches("./").to_ascii_lowercase(),
                (data_start, size),
            );
        }
        offset = data_start + size.div_ceil(BLOCK) * BLOCK;
    }
    Ok(entries)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Builds a minimal ustar archive for tests.
    pub(crate) fn tar(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        for (name, contents) in files {
            let mut header = [0u8; BLOCK];
            header[..name.len()].copy_from_slice(name.as_bytes());
            let size = format!("{:011o}\0", contents.len());
            header[124..136].copy_from_slice(size.as_bytes());
            header[156] = b'0';
            header[257..263].copy_from_slice(b"ustar\0");
            out.extend_from_slice(&header);
            out.extend_from_slice(contents);
            out.resize(out.len().div_ceil(BLOCK) * BLOCK, 0);
        }
        out.extend_from_slice(&[0u8; BLOCK * 2]);
        out
    }

    pub(crate) fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "atlas-usbboot-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_nanos())
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn files_come_from_the_archive_by_chip() {
        let dir = temp_dir("archive");
        std::fs::write(
            dir.join("bootfiles.bin"),
            tar(&[
                ("2711/bootcode4.bin", b"cm4-stage2"),
                ("2712/bootcode5.bin", b"cm5-stage2"),
                ("2711/config.txt", b"from-archive"),
            ]),
        )
        .unwrap();
        std::fs::write(dir.join("boot.img"), b"ramdisk").unwrap();

        let files = BootFiles::open(&dir).unwrap();

        assert_eq!(files.second_stage(Chip::Bcm2711).unwrap(), b"cm4-stage2");
        assert_eq!(files.second_stage(Chip::Bcm2712).unwrap(), b"cm5-stage2");
        assert_eq!(
            files.read(Chip::Bcm2711, "boot.img").unwrap().unwrap(),
            b"ramdisk"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// raze-flasher's bundle: bootfiles/2712/bootcode5.bin unpacked, the
    /// outer config.txt and boot.img at the top.
    #[test]
    fn an_unpacked_bootfiles_tree_works_like_the_archive() {
        let dir = temp_dir("unpacked");
        std::fs::create_dir_all(dir.join("bootfiles/2712")).unwrap();
        std::fs::write(dir.join("bootfiles/2712/bootcode5.bin"), b"cm5-stage2").unwrap();
        std::fs::write(dir.join("config.txt"), b"boot_ramdisk=1").unwrap();
        std::fs::write(dir.join("boot.img"), b"flasher").unwrap();

        let files = BootFiles::open(&dir).unwrap();

        assert_eq!(files.second_stage(Chip::Bcm2712).unwrap(), b"cm5-stage2");
        assert_eq!(
            files.read(Chip::Bcm2712, "config.txt").unwrap().unwrap(),
            b"boot_ramdisk=1"
        );
        assert_eq!(
            files.read(Chip::Bcm2712, "boot.img").unwrap().unwrap(),
            b"flasher"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_loose_chip_file_overrides_the_archive() {
        let dir = temp_dir("override");
        std::fs::write(
            dir.join("bootfiles.bin"),
            tar(&[
                ("2711/bootcode4.bin", b"stage2"),
                ("2711/config.txt", b"archive"),
            ]),
        )
        .unwrap();
        std::fs::create_dir_all(dir.join("2711")).unwrap();
        std::fs::write(dir.join("2711").join("config.txt"), b"override").unwrap();

        let files = BootFiles::open(&dir).unwrap();

        assert_eq!(
            files.read(Chip::Bcm2711, "config.txt").unwrap().unwrap(),
            b"override"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_eeprom_folder_may_name_its_tool_recovery_bin() {
        let dir = temp_dir("eeprom");
        std::fs::write(dir.join("recovery.bin"), b"tool").unwrap();
        std::fs::write(dir.join("pieeprom.bin"), b"image").unwrap();

        let files = BootFiles::open(&dir).unwrap();

        assert_eq!(files.second_stage(Chip::Bcm2712).unwrap(), b"tool");
        assert_eq!(
            files.read(Chip::Bcm2712, "pieeprom.bin").unwrap().unwrap(),
            b"image"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn path_traversal_is_refused() {
        let dir = temp_dir("traversal");
        std::fs::write(
            dir.join("bootfiles.bin"),
            tar(&[("2711/bootcode4.bin", b"x")]),
        )
        .unwrap();
        let files = BootFiles::open(&dir).unwrap();

        assert_eq!(files.read(Chip::Bcm2711, "../secret").unwrap(), None);
        assert_eq!(files.read(Chip::Bcm2711, "a\\b").unwrap(), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_directory_without_bootcode_is_rejected() {
        let dir = temp_dir("empty");
        assert!(matches!(
            BootFiles::open(&dir),
            Err(UsbBootError::BootFilesMissing(_))
        ));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_installed_usbboot_files_parse_when_present() {
        let dir = Path::new("/usr/share/rpiboot/mass-storage-gadget64");
        if !dir.join("bootfiles.bin").is_file() {
            return;
        }
        let files = BootFiles::open(dir).unwrap();
        assert!(files.second_stage(Chip::Bcm2711).unwrap().len() > 10_000);
        assert!(files.second_stage(Chip::Bcm2712).unwrap().len() > 10_000);
        assert!(files.read(Chip::Bcm2711, "boot.img").unwrap().is_some());
    }
}
