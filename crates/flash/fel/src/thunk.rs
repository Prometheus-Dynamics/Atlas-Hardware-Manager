//! The FEL thunk: the small routine Atlas writes into SRAM and runs with
//! FEL_RUN. Three variants, one per [`ThunkIsa`], each with the same swap
//! table and parts at the offsets of its [`Shape`]:
//!
//! - `thunk.S` (ARMv7, [`ThunkIsa::Armv7`]): FEL in AArch32, calls the SPL,
//!   swaps back, returns into the boot ROM's FEL loop.
//! - `thunk_a64.S` ([`ThunkIsa::Aarch64`]): the same contract for FEL in
//!   native AArch64, with AAPCS64 callee-saved registers.
//! - `thunk_rmr.S` ([`ThunkIsa::Aarch32ToAarch64`]): FEL in AArch32, swaps,
//!   writes the AArch64 entry to the SoC's reset vector register and requests
//!   a warm reset into AArch64 through RMR. It never returns to FEL.
//!
//! The design and the reason for each are in the crate root, and the words
//! below are the assembled `.text` of each source (zeroed data included),
//! which the host fills from the layout. The tests reassemble each source
//! when its assembler is installed and compare.

use crate::layout::{Swap, ThunkIsa};

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

/// Bytes of the code the table follows (the `table` label) in the ARMv7 thunk.
pub const TABLE_OFFSET: usize = 0x8c;
/// Swap entries each thunk's table holds; a zero byte count ends the list.
pub const TABLE_ENTRIES: usize = 8;
/// The ARMv7 thunk's SPL entry word (the `entry_word` label).
pub const ENTRY_OFFSET: usize = 0xec;
/// The ARMv7 thunk's pass flag (the `flag` label), zero before the SPL call.
pub const FLAG_OFFSET: usize = 0xf0;
/// The ARMv7 thunk's register save area (the `save_area` label): r4-r11, sp, lr.
pub const SAVE_OFFSET: usize = 0xf4;
/// The ARMv7 blob's length in bytes.
pub const BLOB_LEN: usize = WORDS.len() * 4;

/// Bytes of the AArch64 thunk's code before its table (the `table` label).
pub const A64_TABLE_OFFSET: usize = 0xa8;
/// The AArch64 thunk's SPL entry word (the `entry_word` label).
pub const A64_ENTRY_OFFSET: usize = 0x108;
/// The AArch64 thunk's pass flag (the `flag` label).
pub const A64_FLAG_OFFSET: usize = 0x10c;
/// The AArch64 thunk's register save area (the `save_area` label): x19-x30, sp.
pub const A64_SAVE_OFFSET: usize = 0x110;

/// Bytes of the RMR thunk's code before its table (the `table` label).
pub const RMR_TABLE_OFFSET: usize = 0x64;
/// The RMR thunk's AArch64 entry word (the `entry_word` label).
pub const RMR_ENTRY_OFFSET: usize = 0xc4;
/// The RMR thunk's reset vector word (the `vector_word` label): the SoC's
/// writable RVBAR backing store, filled from the layout.
pub const RMR_VECTOR_OFFSET: usize = 0xc8;

/// Where one thunk's parts sit in its blob, in bytes from the blob's start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shape {
    /// The assembled blob as little-endian words.
    pub words: &'static [u32],
    /// The swap table: [`TABLE_ENTRIES`] triples of 32-bit words.
    pub table: usize,
    /// The SPL entry word, filled with the layout's load address.
    pub entry: usize,
    /// The pass flag, when the thunk returns to FEL.
    pub flag: Option<usize>,
    /// The register save area, when the thunk returns to FEL.
    pub save: Option<usize>,
    /// The reset vector word, when the thunk requests a warm reset.
    pub vector: Option<usize>,
}

impl Shape {
    /// The blob's length in bytes.
    pub fn len(&self) -> usize {
        self.words.len() * 4
    }

    /// Always false: a thunk is never empty. Present for the lint.
    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }
}

/// The shape of the thunk `isa` names.
pub fn shape(isa: ThunkIsa) -> Shape {
    match isa {
        ThunkIsa::Armv7 => Shape {
            words: &WORDS,
            table: TABLE_OFFSET,
            entry: ENTRY_OFFSET,
            flag: Some(FLAG_OFFSET),
            save: Some(SAVE_OFFSET),
            vector: None,
        },
        ThunkIsa::Aarch64 => Shape {
            words: &A64_WORDS,
            table: A64_TABLE_OFFSET,
            entry: A64_ENTRY_OFFSET,
            flag: Some(A64_FLAG_OFFSET),
            save: Some(A64_SAVE_OFFSET),
            vector: None,
        },
        ThunkIsa::Aarch32ToAarch64 { .. } => Shape {
            words: &RMR_WORDS,
            table: RMR_TABLE_OFFSET,
            entry: RMR_ENTRY_OFFSET,
            flag: None,
            save: None,
            vector: Some(RMR_VECTOR_OFFSET),
        },
    }
}

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
/// swap table filled from `swaps` (buf1, buf2, bytes; zero-terminated), the
/// SPL entry address, and, for [`ThunkIsa::Aarch32ToAarch64`], the reset
/// vector word the layout names.
pub fn blob(isa: ThunkIsa, swaps: &[Swap], entry: u32) -> Result<Vec<u8>, TableError> {
    if swaps.len() > TABLE_ENTRIES {
        return Err(TableError::TooManyEntries(swaps.len()));
    }
    let shape = shape(isa);
    let mut bytes: Vec<u8> = shape
        .words
        .iter()
        .flat_map(|word| word.to_le_bytes())
        .collect();
    let mut table = Vec::with_capacity(TABLE_ENTRIES * 12);
    for swap in swaps {
        table.extend_from_slice(&swap.buf1.to_le_bytes());
        table.extend_from_slice(&swap.buf2.to_le_bytes());
        table.extend_from_slice(&swap.size.to_le_bytes());
    }
    table.resize(TABLE_ENTRIES * 12, 0);
    bytes[shape.table..shape.table + table.len()].copy_from_slice(&table);
    bytes[shape.entry..shape.entry + 4].copy_from_slice(&entry.to_le_bytes());
    if let (ThunkIsa::Aarch32ToAarch64 { reset_vector }, Some(at)) = (isa, shape.vector) {
        bytes[at..at + 4].copy_from_slice(&reset_vector.to_le_bytes());
    }
    Ok(bytes)
}

/// The RMR thunk (`thunk_rmr.S`), as little-endian 32-bit words: code, the swap
/// table, the AArch64 entry word and the reset vector word.
pub const RMR_WORDS: [u32; 51] = [
    0xe28f405c, 0xe4940004, 0xe4941004, 0xe4942004, 0xe3520000, 0x0a000006, 0xe5903000, 0xe591c000,
    0xe480c004, 0xe4813004, 0xe2522004, 0x1afffff9, 0xeafffff3, 0xe28f008c, 0xe5900000, 0xe28f1080,
    0xe5911000, 0xe5801000, 0xf57ff04f, 0xe3a02003, 0xee0c2f50, 0xf57ff04f, 0xf57ff06f, 0xe320f003,
    0xeafffffd, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000,
    0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000,
    0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000,
    0x00000000, 0x00000000, 0x00000000,
];

/// The AArch64 thunk (`thunk_a64.S`), as little-endian 32-bit words: code, the
/// swap table, the SPL entry word, the pass flag and the register save area.
pub const A64_WORDS: [u32; 94] = [
    0x10000880, 0xa8815013, 0xa8815815, 0xa8816017, 0xa8816819, 0xa881701b, 0xa881781d, 0x910003e1,
    0xf9000001, 0x10000742, 0xb900005f, 0x100003e4, 0xb8404480, 0xb8404481, 0xb8404482, 0x34000102,
    0xb9400003, 0xb9400025, 0xb8004405, 0xb8004423, 0x71001042, 0x54ffff61, 0x17fffff6, 0x10000582,
    0xb9400041, 0x350000e1, 0x52800021, 0xb9000041, 0x100004c0, 0xb9400000, 0xd63f0000, 0x17ffffec,
    0x10000480, 0xa8c15013, 0xa8c15815, 0xa8c16017, 0xa8c16819, 0xa8c1701b, 0xa8c1781d, 0xf9400001,
    0x9100003f, 0xd65f03c0, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000,
    0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000,
    0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000,
    0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000,
    0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000,
    0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000,
    0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000,
];

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

    /// The assembled source: its .text bytes, and each label's offset from
    /// the object's symbol table (read with `llvm-nm`).
    struct Assembled {
        text: Vec<u8>,
        symbols: Vec<(String, usize)>,
    }

    fn finish(object: &Path, out: &Path, objcopy: &Path) -> Option<Assembled> {
        let copied = Command::new(objcopy)
            .args(["-O", "binary", "-j", ".text"])
            .arg(object)
            .arg(out)
            .status()
            .ok()?;
        if !copied.success() {
            return None;
        }
        let nm = tool("llvm-nm")?;
        let listing = Command::new(nm).arg("-n").arg(object).output().ok()?;
        if !listing.status.success() {
            return None;
        }
        let symbols = String::from_utf8_lossy(&listing.stdout)
            .lines()
            .filter_map(|line| {
                let mut parts = line.split_whitespace();
                let address = usize::from_str_radix(parts.next()?, 16).ok()?;
                let _kind = parts.next()?;
                Some((parts.next()?.to_string(), address))
            })
            .collect();
        let text = std::fs::read(out).ok()?;
        Some(Assembled { text, symbols })
    }

    /// Assembles `source` for ARMv7 with clang (as thunk.S is).
    fn assemble_armv7(source: &Path, dir: &Path) -> Option<Assembled> {
        let clang = tool("clang")?;
        let objcopy = tool("llvm-objcopy")?;
        let object = dir.join("armv7.o");
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
        finish(&object, &dir.join("armv7.bin"), &objcopy)
    }

    /// Assembles `source` for AArch64 with the GNU cross assembler.
    fn assemble_aarch64(source: &Path, dir: &Path) -> Option<Assembled> {
        let assembler = tool("aarch64-linux-gnu-as")?;
        let objcopy = tool("aarch64-linux-gnu-objcopy")?;
        let object = dir.join("aarch64.o");
        let assembled = Command::new(assembler)
            .arg("-o")
            .arg(&object)
            .arg(source)
            .status()
            .ok()?;
        if !assembled.success() {
            return None;
        }
        finish(&object, &dir.join("aarch64.bin"), &objcopy)
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("atlas-fel-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a scratch directory for the assembler");
        dir
    }

    fn source(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(name)
    }

    fn word_bytes(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|word| word.to_le_bytes()).collect()
    }

    #[allow(
        clippy::print_stderr,
        reason = "the skip notice is the test's only output"
    )]
    fn skip(what: &str) {
        eprintln!("skipped: {what}; the thunk source is not compared");
    }

    /// The label offsets the constants name, checked against the object.
    fn check_labels(assembled: &Assembled, expected: &[(&str, usize)]) {
        for (label, offset) in expected {
            let found = assembled
                .symbols
                .iter()
                .find(|(name, _)| name == label)
                .map(|(_, address)| *address);
            assert_eq!(found, Some(*offset), "label {label} is not at {offset:#x}");
        }
    }

    #[test]
    fn the_armv7_words_are_the_assembled_source() {
        let dir = scratch("armv7");
        let Some(assembled) = assemble_armv7(&source("thunk.S"), &dir) else {
            skip("clang or llvm-objcopy is not installed (or failed)");
            return;
        };
        assert_eq!(
            assembled.text,
            word_bytes(&WORDS),
            "thunk.rs WORDS are stale: reassemble src/thunk.S"
        );
        check_labels(
            &assembled,
            &[
                ("table", TABLE_OFFSET),
                ("entry_word", ENTRY_OFFSET),
                ("flag", FLAG_OFFSET),
                ("save_area", SAVE_OFFSET),
            ],
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_rmr_words_are_the_assembled_source() {
        let dir = scratch("rmr");
        let Some(assembled) = assemble_armv7(&source("thunk_rmr.S"), &dir) else {
            skip("clang or llvm-objcopy is not installed (or failed)");
            return;
        };
        assert_eq!(
            assembled.text,
            word_bytes(&RMR_WORDS),
            "thunk.rs RMR_WORDS are stale: reassemble src/thunk_rmr.S"
        );
        check_labels(
            &assembled,
            &[
                ("table", RMR_TABLE_OFFSET),
                ("entry_word", RMR_ENTRY_OFFSET),
                ("vector_word", RMR_VECTOR_OFFSET),
            ],
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_aarch64_words_are_the_assembled_source() {
        let dir = scratch("a64");
        let Some(assembled) = assemble_aarch64(&source("thunk_a64.S"), &dir) else {
            skip("aarch64-linux-gnu-as or aarch64-linux-gnu-objcopy is not installed (or failed)");
            return;
        };
        assert_eq!(
            assembled.text,
            word_bytes(&A64_WORDS),
            "thunk.rs A64_WORDS are stale: reassemble src/thunk_a64.S"
        );
        check_labels(
            &assembled,
            &[
                ("table", A64_TABLE_OFFSET),
                ("entry_word", A64_ENTRY_OFFSET),
                ("flag", A64_FLAG_OFFSET),
                ("save_area", A64_SAVE_OFFSET),
            ],
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_shapes_fill_their_blobs_exactly() {
        let armv7 = shape(ThunkIsa::Armv7);
        assert_eq!(armv7.len(), 284);
        assert_eq!(armv7.table + TABLE_ENTRIES * 12, armv7.entry);
        assert_eq!(armv7.save.unwrap() + 10 * 4, armv7.len());
        let a64 = shape(ThunkIsa::Aarch64);
        assert_eq!(a64.len(), 376);
        assert_eq!(a64.table + TABLE_ENTRIES * 12, a64.entry);
        assert_eq!(a64.save.unwrap() + 13 * 8, a64.len());
        let rmr = shape(ThunkIsa::Aarch32ToAarch64 { reset_vector: 0 });
        assert_eq!(rmr.len(), 204);
        assert_eq!(rmr.table + TABLE_ENTRIES * 12, rmr.entry);
        assert_eq!(rmr.vector.unwrap(), rmr.entry + 4);
        assert_eq!(rmr.vector.unwrap() + 4, rmr.len());
        assert!(rmr.flag.is_none() && rmr.save.is_none());
    }

    #[test]
    fn the_blob_carries_the_table_and_the_entry() {
        let swaps = [Swap {
            buf1: 0x1800,
            buf2: 0x44000,
            size: 0x800,
        }];
        let bytes = blob(ThunkIsa::Armv7, &swaps, 0x0002_0000).unwrap();
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
        assert!(blob(ThunkIsa::Armv7, &too_many, 0).is_err());
    }

    #[test]
    fn the_aarch64_blob_carries_its_own_offsets() {
        let swaps = [Swap {
            buf1: 0x1800,
            buf2: 0x44000,
            size: 0x800,
        }];
        let bytes = blob(ThunkIsa::Aarch64, &swaps, 0x0004_7000).unwrap();
        assert_eq!(bytes.len(), 376);
        assert_eq!(
            &bytes[A64_TABLE_OFFSET..A64_TABLE_OFFSET + 4],
            &0x1800u32.to_le_bytes()
        );
        assert_eq!(
            &bytes[A64_ENTRY_OFFSET..A64_ENTRY_OFFSET + 4],
            &0x0004_7000u32.to_le_bytes()
        );
    }

    #[test]
    fn the_rmr_blob_carries_the_reset_vector_the_layout_names() {
        let isa = ThunkIsa::Aarch32ToAarch64 {
            reset_vector: 0x0800_1004,
        };
        let bytes = blob(isa, &[], 0x0004_7000).unwrap();
        assert_eq!(bytes.len(), 204);
        assert_eq!(
            &bytes[RMR_ENTRY_OFFSET..RMR_ENTRY_OFFSET + 4],
            &0x0004_7000u32.to_le_bytes()
        );
        assert_eq!(
            &bytes[RMR_VECTOR_OFFSET..RMR_VECTOR_OFFSET + 4],
            &0x0800_1004u32.to_le_bytes()
        );
        // With no swaps the table is all zero: the thunk swaps nothing.
        assert!(
            bytes[RMR_TABLE_OFFSET..RMR_TABLE_OFFSET + TABLE_ENTRIES * 12]
                .iter()
                .all(|byte| *byte == 0)
        );
    }
}
