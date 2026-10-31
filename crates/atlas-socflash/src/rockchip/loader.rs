//! Rockchip's loader file (`rk3588_spl_loader_v1.xx.bin`, boot_merger's
//! output; rkdeveloptool RKBoot.h `STRUCT_RKBOOT_HEAD`, packed,
//! little-endian): the 471 entries (DDR init) and 472 entries (usbplug)
//! maskrom runs, and the loader entries (FlashHead, FlashData, FlashBoot)
//! that make the idblock. A CRC-32 of everything before it closes the file.

use crate::rockchip::crc::{LOADER_POLYNOMIALS, rk_crc32};

const MAGIC_BOOT: [u8; 4] = *b"BOOT";
const MAGIC_LDR: [u8; 4] = *b"LDR ";
/// The header's fields end at 45 (57 reserved bytes follow, 102 in all).
const HEADER_FIELDS: usize = 45;
/// Bytes of an entry the parser reads: size, type, 20 UTF-16 name
/// characters, data offset, data size, delay.
const ENTRY_FIELDS: usize = 57;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    /// Code maskrom runs first (DDR init), sent with wIndex 0x471.
    Ddr,
    /// Code maskrom runs next (the usbplug), sent with wIndex 0x472.
    Usbplug,
    /// Code the idblock carries (FlashHead, FlashData, FlashBoot).
    Loader,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub kind: EntryKind,
    pub name: String,
    pub offset: u32,
    pub size: u32,
    /// Milliseconds to wait after sending it (471/472 entries).
    pub delay_ms: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Loader {
    pub magic: [u8; 4],
    pub version: u32,
    /// `emSupportChip`: newer chips' values aren't in rkdeveloptool's enum.
    pub chip: u32,
    pub signed: bool,
    /// Header flag `ucRc4Flag`: nonzero means the entries are not RC4'd.
    pub rc4_disabled: bool,
    pub entries: Vec<Entry>,
    /// The polynomial the trailer matched (see [`LOADER_POLYNOMIALS`]).
    pub crc_polynomial: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoaderError(pub String);

impl std::fmt::Display for LoaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "not a usable Rockchip loader: {}", self.0)
    }
}

impl std::error::Error for LoaderError {}

fn le32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

impl Loader {
    pub fn parse(file: &[u8]) -> Result<Self, LoaderError> {
        let fail = |message: String| Err(LoaderError(message));
        if file.len() < HEADER_FIELDS + 4 {
            return fail(format!("{} bytes is too short", file.len()));
        }
        let magic: [u8; 4] = file[0..4].try_into().unwrap_or_default();
        if magic != MAGIC_BOOT && magic != MAGIC_LDR {
            return fail(format!("magic {:02x?} isn't BOOT or LDR", magic));
        }
        let body = &file[..file.len() - 4];
        let trailer = le32(file, file.len() - 4);
        let Some(crc_polynomial) = LOADER_POLYNOMIALS
            .into_iter()
            .find(|polynomial| rk_crc32(body, *polynomial) == trailer)
        else {
            return fail(
                "its trailing CRC-32 doesn't match its contents (damaged or cut short)".into(),
            );
        };

        let tables = [
            (EntryKind::Ddr, file[25], le32(file, 26), file[30]),
            (EntryKind::Usbplug, file[31], le32(file, 32), file[36]),
            (EntryKind::Loader, file[37], le32(file, 38), file[42]),
        ];
        let mut entries = Vec::new();
        for (kind, count, offset, size) in tables {
            if count > 0 && usize::from(size) < ENTRY_FIELDS {
                return fail(format!(
                    "{kind:?} entries are {size} bytes, under {ENTRY_FIELDS}"
                ));
            }
            for index in 0..usize::from(count) {
                let at = offset as usize + index * usize::from(size);
                let Some(raw) = body.get(at..at + ENTRY_FIELDS) else {
                    return fail(format!("{kind:?} entry {index} lies past the end"));
                };
                // The name: 20 UTF-16LE units, up to the first NUL.
                let name: String = raw[5..45]
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|unit| u16::from_le_bytes([unit[0], unit[1]]))
                    .take_while(|unit| *unit != 0)
                    .map(|unit| char::from_u32(u32::from(unit)).unwrap_or('?'))
                    .collect();
                let entry = Entry {
                    kind,
                    name,
                    offset: le32(raw, 45),
                    size: le32(raw, 49),
                    delay_ms: le32(raw, 53),
                };
                let end = u64::from(entry.offset) + u64::from(entry.size);
                if entry.size == 0 || end > body.len() as u64 {
                    return fail(format!(
                        "entry {} ({} bytes at {}) lies past the end",
                        entry.name, entry.size, entry.offset
                    ));
                }
                entries.push(entry);
            }
        }
        if !entries.iter().any(|entry| entry.kind == EntryKind::Ddr)
            || !entries.iter().any(|entry| entry.kind == EntryKind::Usbplug)
        {
            return fail("it has no 471 (DDR) or no 472 (usbplug) entry for maskrom".into());
        }
        Ok(Self {
            magic,
            version: le32(file, 6),
            chip: le32(file, 21),
            signed: file[43] == b'S',
            rc4_disabled: file[44] != 0,
            entries,
            crc_polynomial,
        })
    }

    pub fn entries(&self, kind: EntryKind) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(move |entry| entry.kind == kind)
    }

    pub fn loader_entry(&self, name: &str) -> Option<&Entry> {
        self.entries(EntryKind::Loader)
            .find(|entry| entry.name == name)
    }
}

impl Entry {
    pub fn data<'a>(&self, file: &'a [u8]) -> &'a [u8] {
        &file[self.offset as usize..(self.offset + self.size) as usize]
    }
}

/// Builds a loader file as boot_merger lays one out, for tests.
pub fn build_for_test(entries: &[(EntryKind, &str, &[u8], u32)], polynomial: u32) -> Vec<u8> {
    const HEADER: usize = 102;
    const ENTRY: usize = 57;
    let mut file = vec![0u8; HEADER];
    file[0..4].copy_from_slice(&MAGIC_BOOT);
    file[4..6].copy_from_slice(&(HEADER as u16).to_le_bytes());
    file[6..10].copy_from_slice(&0x0101u32.to_le_bytes());
    file[44] = 1; // RC4 off
    let mut tables = Vec::new();
    let mut data_at = HEADER + entries.len() * ENTRY;
    for kind in [EntryKind::Ddr, EntryKind::Usbplug, EntryKind::Loader] {
        let start = file.len() + tables.len();
        let mine: Vec<_> = entries.iter().filter(|e| e.0 == kind).collect();
        let (count_at, offset_at, size_at) = match kind {
            EntryKind::Ddr => (25, 26, 30),
            EntryKind::Usbplug => (31, 32, 36),
            EntryKind::Loader => (37, 38, 42),
        };
        file[count_at] = mine.len() as u8;
        file[offset_at..offset_at + 4].copy_from_slice(&(start as u32).to_le_bytes());
        file[size_at] = ENTRY as u8;
        for (kind, name, data, delay) in mine {
            let mut entry = vec![0u8; ENTRY];
            entry[0] = ENTRY as u8;
            let code: u32 = match kind {
                EntryKind::Ddr => 1,
                EntryKind::Usbplug => 2,
                EntryKind::Loader => 4,
            };
            entry[1..5].copy_from_slice(&code.to_le_bytes());
            for (index, unit) in name.encode_utf16().take(20).enumerate() {
                entry[5 + index * 2..7 + index * 2].copy_from_slice(&unit.to_le_bytes());
            }
            entry[45..49].copy_from_slice(&(data_at as u32).to_le_bytes());
            entry[49..53].copy_from_slice(&(data.len() as u32).to_le_bytes());
            entry[53..57].copy_from_slice(&delay.to_le_bytes());
            data_at += data.len();
            tables.extend(entry);
        }
    }
    file.extend(tables);
    for (_, _, data, _) in entries.iter().filter(|e| e.0 == EntryKind::Ddr) {
        file.extend_from_slice(data);
    }
    for (_, _, data, _) in entries.iter().filter(|e| e.0 == EntryKind::Usbplug) {
        file.extend_from_slice(data);
    }
    for (_, _, data, _) in entries.iter().filter(|e| e.0 == EntryKind::Loader) {
        file.extend_from_slice(data);
    }
    let crc = rk_crc32(&file, polynomial);
    file.extend_from_slice(&crc.to_le_bytes());
    file
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(polynomial: u32) -> Vec<u8> {
        build_for_test(
            &[
                (EntryKind::Ddr, "rk3588_ddr", &[1; 100], 1),
                (EntryKind::Usbplug, "rk3588_usbplug", &[2; 50], 0),
                (EntryKind::Loader, "FlashHead", &[3; 10], 0),
                (EntryKind::Loader, "FlashData", &[4; 20], 0),
                (EntryKind::Loader, "FlashBoot", &[5; 30], 0),
            ],
            polynomial,
        )
    }

    #[test]
    fn entries_and_their_data() {
        for polynomial in LOADER_POLYNOMIALS {
            let file = sample(polynomial);
            let loader = Loader::parse(&file).unwrap();
            assert_eq!(loader.crc_polynomial, polynomial);
            assert!(loader.rc4_disabled);
            let ddr: Vec<_> = loader.entries(EntryKind::Ddr).collect();
            assert_eq!((ddr[0].name.as_str(), ddr[0].delay_ms), ("rk3588_ddr", 1));
            assert_eq!(ddr[0].data(&file), &[1; 100]);
            let boot = loader.loader_entry("FlashBoot").unwrap();
            assert_eq!(boot.data(&file), &[5; 30]);
        }
    }

    #[test]
    fn damage_and_missing_stages_are_refused() {
        let mut file = sample(LOADER_POLYNOMIALS[0]);
        file[200] ^= 1;
        assert!(Loader::parse(&file).unwrap_err().0.contains("CRC"));
        assert!(Loader::parse(b"BOOT").is_err());
        let only_ddr = build_for_test(
            &[(EntryKind::Ddr, "ddr", &[1; 4], 0)],
            LOADER_POLYNOMIALS[0],
        );
        assert!(Loader::parse(&only_ddr).unwrap_err().0.contains("472"));
        let mut other = sample(LOADER_POLYNOMIALS[0]);
        other[0] = b'X';
        assert!(Loader::parse(&other).unwrap_err().0.contains("magic"));
    }
}
