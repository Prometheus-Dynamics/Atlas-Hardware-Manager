//! The boot files a Pi asks for: the second-stage bootloader and the files the
//! file server sends. The caller passes them in, as a folder or as an
//! in-memory set; this crate ships none.
//!
//! For a chip and a name, the lookup tries, most specific first:
//! `<chip folder>/<name>`, then `bootfiles/<chip folder>/<name>` (the unpacked
//! layout of a flasher bundle), then `<name>` at the top.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::{Chip, RpiBootError};

/// The EEPROM tool. An EEPROM recovery folder may ship it under this name in
/// place of the second stage.
const EEPROM_TOOL: &str = "recovery.bin";

#[derive(Debug)]
enum Source {
    Folder(PathBuf),
    Memory(BTreeMap<String, Vec<u8>>),
}

/// The boot files for one flasher image or one EEPROM recovery.
#[derive(Debug)]
pub struct BootFiles {
    source: Source,
}

impl BootFiles {
    /// Boot files in a folder. The files are read when asked for.
    pub fn open(folder: impl Into<PathBuf>) -> Result<Self, RpiBootError> {
        let folder = folder.into();
        if !folder.is_dir() {
            return Err(RpiBootError::Files(format!(
                "{} is not a folder of boot files",
                folder.display()
            )));
        }
        Ok(Self {
            source: Source::Folder(folder),
        })
    }

    /// Boot files held in memory, keyed by path relative to the boot files
    /// root, for example `2712/bootcode5.bin` or `config.txt`.
    pub fn in_memory<K, I>(files: I) -> Self
    where
        K: Into<String>,
        I: IntoIterator<Item = (K, Vec<u8>)>,
    {
        Self {
            source: Source::Memory(
                files
                    .into_iter()
                    .map(|(path, data)| (path.into(), data))
                    .collect(),
            ),
        }
    }

    /// The contents of `name` for `chip`, or `None` when there is no such
    /// file. A name that tries to leave the boot files is `None` too.
    pub fn read(&self, chip: Chip, name: &str) -> Result<Option<Vec<u8>>, RpiBootError> {
        let name = name.trim_start_matches('/');
        if name.is_empty() || name.contains("..") || name.contains('\\') || name.contains(':') {
            return Ok(None);
        }
        let candidates = [
            format!("{}/{name}", chip.folder()),
            format!("bootfiles/{}/{name}", chip.folder()),
            name.to_string(),
        ];
        for path in &candidates {
            if let Some(data) = self.fetch(path)? {
                return Ok(Some(data));
            }
        }
        Ok(None)
    }

    /// The second-stage bootloader for `chip`: its `bootcode` file, or the
    /// EEPROM tool.
    pub fn second_stage(&self, chip: Chip) -> Result<Vec<u8>, RpiBootError> {
        if let Some(data) = self.read(chip, chip.second_stage_file())? {
            return Ok(data);
        }
        self.read(chip, EEPROM_TOOL)?
            .ok_or(RpiBootError::MissingFile {
                name: chip.second_stage_file().into(),
                chip: chip.label(),
            })
    }

    fn fetch(&self, path: &str) -> Result<Option<Vec<u8>>, RpiBootError> {
        match &self.source {
            Source::Folder(folder) => {
                let file = folder.join(path);
                if !file.is_file() {
                    return Ok(None);
                }
                std::fs::read(&file).map(Some).map_err(|error| {
                    RpiBootError::Files(format!("could not read {}: {error}", file.display()))
                })
            }
            Source::Memory(files) => Ok(files.get(path).cloned()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_chip_folder_wins_then_the_unpacked_tree_then_the_top() {
        let files = BootFiles::in_memory([
            ("config.txt", b"top".to_vec()),
            ("bootfiles/2711/config.txt", b"unpacked".to_vec()),
            ("2711/config.txt", b"chip".to_vec()),
        ]);
        assert_eq!(
            files.read(Chip::Bcm2711, "config.txt").unwrap().unwrap(),
            b"chip"
        );
        assert_eq!(
            files.read(Chip::Bcm2712, "config.txt").unwrap().unwrap(),
            b"top"
        );
        let unpacked = BootFiles::in_memory([
            ("config.txt", b"top".to_vec()),
            ("bootfiles/2711/config.txt", b"unpacked".to_vec()),
        ]);
        assert_eq!(
            unpacked.read(Chip::Bcm2711, "config.txt").unwrap().unwrap(),
            b"unpacked"
        );
    }

    #[test]
    fn second_stage_comes_from_the_chip_folder_or_the_eeprom_tool() {
        let files = BootFiles::in_memory([("2712/bootcode5.bin", b"cm5".to_vec())]);
        assert_eq!(files.second_stage(Chip::Bcm2712).unwrap(), b"cm5");
        let tool = BootFiles::in_memory([("recovery.bin", b"tool".to_vec())]);
        assert_eq!(tool.second_stage(Chip::Bcm2712).unwrap(), b"tool");
        let none = BootFiles::in_memory([("config.txt", b"x".to_vec())]);
        assert_eq!(
            none.second_stage(Chip::Bcm2711),
            Err(RpiBootError::MissingFile {
                name: "bootcode4.bin".into(),
                chip: "BCM2711",
            })
        );
    }

    #[test]
    fn names_that_leave_the_boot_files_are_missing() {
        let files = BootFiles::in_memory([("config.txt", b"x".to_vec())]);
        for name in ["../secret", "a\\b", "c:d", ""] {
            assert_eq!(files.read(Chip::Bcm2711, name).unwrap(), None, "{name}");
        }
    }

    #[test]
    fn a_folder_is_read_as_it_is_and_must_exist() {
        let dir = std::env::temp_dir().join(format!("atlas-rpiboot-files-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("2711")).unwrap();
        std::fs::write(dir.join("2711/bootcode4.bin"), b"cm4").unwrap();
        std::fs::write(dir.join("boot.img"), b"flasher").unwrap();

        let files = BootFiles::open(&dir).unwrap();
        assert_eq!(files.second_stage(Chip::Bcm2711).unwrap(), b"cm4");
        assert_eq!(
            files.read(Chip::Bcm2711, "boot.img").unwrap().unwrap(),
            b"flasher"
        );
        assert_eq!(files.read(Chip::Bcm2711, "absent.img").unwrap(), None);
        assert!(BootFiles::open(dir.join("missing")).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
