//! The FEL thunk: the small ARM routine Atlas writes into SRAM and runs
//! with FEL_RUN. It swaps the boot ROM's stack buffers out, calls the SPL,
//! swaps them back and returns into the ROM's FEL loop. Design and the
//! reason it exists are in fel/mod.rs; the source is fel/thunk.S, and the
//! words below are its assembled .text (plus the zeroed data the host
//! fills). The test reassembles thunk.S with clang when clang is installed
//! and compares.

use super::layout::Swap;

/// The assembled blob, as little-endian 32-bit words: code, then the swap
/// table, the SPL entry word, the pass flag and the register save area.
pub const WORDS: [u32; 71] = [
    0xe28f00ec, 0xe8a00ff0, 0xe480d004, 0xe480e004, 0xe3a01000, 0xe28f20d4, 0xe5821000, 0xe28f4068,
    0xe4940004, 0xe4941004, 0xe4942004, 0xe3520000, 0x0a000006, 0xe5903000, 0xe591c000, 0xe480c004,
    0xe4813004, 0xe2522004, 0x1afffff9, 0xeafffff3, 0xe28f2098, 0xe5921000, 0xe3510000, 0x1a000005,
    0xe3a01001, 0xe5821000, 0xe28f007c, 0xe5900000, 0xe12fff30, 0xeaffffe8, 0xe28f0074, 0xe8b00ff0,
    0xe490d004, 0xe490e004, 0xe12fff1e, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000,
    0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000,
    0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000,
    0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000,
    0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000,
];

/// Bytes of the code the table follows (the `table` label).
pub const TABLE_OFFSET: usize = 0x8c;
/// Swap entries the table holds; a zero byte count ends the list.
pub const TABLE_ENTRIES: usize = 8;
/// The SPL entry word (the `entry_word` label).
pub const ENTRY_OFFSET: usize = 0xec;
/// The pass flag (the `flag` label), zero before the SPL call.
pub const FLAG_OFFSET: usize = 0xf0;
/// The register save area (the `save_area` label): r4-r11, sp, lr.
pub const SAVE_OFFSET: usize = 0xf4;
/// The whole blob's length in bytes.
pub const BLOB_LEN: usize = WORDS.len() * 4;

/// Why a swap table can't be written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TableError {
    TooManyEntries(usize),
}

impl std::fmt::Display for TableError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooManyEntries(count) => write!(
                f,
                "{count} swap buffers; the thunk's table holds {TABLE_ENTRIES}"
            ),
        }
    }
}

impl std::error::Error for TableError {}

/// The thunk blob as the host writes it at the thunk address: the code, the
/// swap table filled from `swaps` (buf1, buf2, bytes; zero-terminated), and
/// the SPL entry address.
pub fn blob(swaps: &[Swap], entry: u32) -> Result<Vec<u8>, TableError> {
    if swaps.len() > TABLE_ENTRIES {
        return Err(TableError::TooManyEntries(swaps.len()));
    }
    let mut bytes: Vec<u8> = WORDS.iter().flat_map(|word| word.to_le_bytes()).collect();
    let mut table = Vec::with_capacity(TABLE_ENTRIES * 12);
    for swap in swaps {
        table.extend_from_slice(&swap.buf1.to_le_bytes());
        table.extend_from_slice(&swap.buf2.to_le_bytes());
        table.extend_from_slice(&swap.size.to_le_bytes());
    }
    table.resize(TABLE_ENTRIES * 12, 0);
    bytes[TABLE_OFFSET..TABLE_OFFSET + table.len()].copy_from_slice(&table);
    bytes[ENTRY_OFFSET..ENTRY_OFFSET + 4].copy_from_slice(&entry.to_le_bytes());
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    fn tool(name: &str) -> Option<PathBuf> {
        let path = std::env::var_os("PATH")?;
        std::env::split_paths(&path)
            .map(|dir| dir.join(name))
            .find(|candidate| candidate.is_file())
    }

    /// Assembles fel/thunk.S with clang and returns its .text bytes.
    fn assemble(source: &Path, out: &Path) -> Option<Vec<u8>> {
        let clang = tool("clang")?;
        let objcopy = tool("llvm-objcopy")?;
        let object = out.with_extension("o");
        let assembled = Command::new(clang)
            .args(["--target=armv7a-none-eabi", "-c"])
            .arg(source)
            .arg("-o")
            .arg(&object)
            .status()
            .ok()?;
        if !assembled.success() {
            return None;
        }
        let copied = Command::new(objcopy)
            .args(["-O", "binary", "-j", ".text"])
            .arg(&object)
            .arg(out)
            .status()
            .ok()?;
        if !copied.success() {
            return None;
        }
        std::fs::read(out).ok()
    }

    #[test]
    #[allow(
        clippy::print_stderr,
        reason = "the skip notice is the test's only output"
    )]
    fn the_words_are_the_assembled_source() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fel/thunk.S");
        let out = std::env::temp_dir().join(format!("atlas-fel-thunk-{}.bin", std::process::id()));
        let Some(bytes) = assemble(&source, &out) else {
            eprintln!(
                "skipped: clang or llvm-objcopy is not installed (or failed); thunk.S not compared"
            );
            return;
        };
        let _ = std::fs::remove_file(&out);
        let _ = std::fs::remove_file(out.with_extension("o"));
        let words: Vec<u8> = WORDS.iter().flat_map(|word| word.to_le_bytes()).collect();
        assert_eq!(
            bytes, words,
            "thunk.rs WORDS are stale: reassemble fel/thunk.S"
        );
        assert_eq!(bytes.len(), BLOB_LEN);
    }

    #[test]
    fn the_offsets_are_the_source_labels() {
        // Measured from `llvm-nm` on the assembled object: table at 0x8c,
        // entry_word 0xec, flag 0xf0, save_area 0xf4, blob 284 bytes.
        assert_eq!(TABLE_OFFSET, 0x8c);
        assert_eq!(ENTRY_OFFSET, 0xec);
        assert_eq!(FLAG_OFFSET, 0xf0);
        assert_eq!(SAVE_OFFSET, 0xf4);
        assert_eq!(BLOB_LEN, 284);
        // The table, then entry, flag and save area fill the blob exactly.
        assert_eq!(TABLE_OFFSET + TABLE_ENTRIES * 12, ENTRY_OFFSET);
        assert_eq!(SAVE_OFFSET + 10 * 4, BLOB_LEN);
    }

    #[test]
    fn the_blob_carries_the_table_and_the_entry() {
        let swaps = [Swap {
            buf1: 0x1800,
            buf2: 0x44000,
            size: 0x800,
        }];
        let bytes = blob(&swaps, 0x0002_0000).unwrap();
        assert_eq!(bytes.len(), BLOB_LEN);
        assert_eq!(
            &bytes[TABLE_OFFSET..TABLE_OFFSET + 4],
            &0x1800u32.to_le_bytes()
        );
        assert_eq!(
            &bytes[TABLE_OFFSET + 8..TABLE_OFFSET + 12],
            &0x800u32.to_le_bytes()
        );
        assert_eq!(&bytes[TABLE_OFFSET + 12..TABLE_OFFSET + 16], &[0; 4]);
        assert_eq!(
            &bytes[ENTRY_OFFSET..ENTRY_OFFSET + 4],
            &0x0002_0000u32.to_le_bytes()
        );
        let too_many = vec![swaps[0]; TABLE_ENTRIES + 1];
        assert!(blob(&too_many, 0).is_err());
    }
}
