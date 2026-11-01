//! Real vendor loaders through the parser and the idblock builder, when
//! `ATLAS_TEST_LOADERS` names some (paths separated by `:`); skipped
//! otherwise, since the files aren't in the repository.
//!
//! ```sh
//! ATLAS_TEST_LOADERS=.../rk3588_spl_loader_v1.18.113.bin \
//!   cargo test -p atlas-socflash --test real_loaders -- --nocapture
//! ```

use std::io::Write;

use atlas_socflash::rockchip::loader::{EntryKind, Loader};
use atlas_socflash::rockchip::maskrom::stage_bytes;
use atlas_socflash::rockchip::write::idblock;

#[test]
fn real_loaders_parse() {
    let Ok(paths) = std::env::var("ATLAS_TEST_LOADERS") else {
        return;
    };
    let mut out = std::io::stdout().lock();
    for path in paths.split(':').filter(|path| !path.is_empty()) {
        let file = std::fs::read(path).unwrap_or_else(|error| panic!("{path}: {error}"));
        let loader = Loader::parse(&file).unwrap_or_else(|error| panic!("{path}: {error}"));
        let _ = writeln!(
            out,
            "{path}: magic {:?}, version {:#x}, chip {:?}, signed {}, rc4 disabled {}, trailer polynomial {:#010x}",
            String::from_utf8_lossy(&loader.magic),
            loader.version,
            loader.chip_number(),
            loader.signed,
            loader.rc4_disabled,
            loader.crc_polynomial
        );
        for entry in &loader.entries {
            let _ = writeln!(
                out,
                "  {:?} {:?}: {} bytes at {}, delay {} ms",
                entry.kind, entry.name, entry.size, entry.offset, entry.delay_ms
            );
        }
        assert!(loader.entries(EntryKind::Ddr).count() >= 1);
        assert!(loader.entries(EntryKind::Usbplug).count() >= 1);
        for entry in loader
            .entries(EntryKind::Ddr)
            .chain(loader.entries(EntryKind::Usbplug))
        {
            let (wire, _) = stage_bytes(entry.data(&file));
            assert!(wire.len() >= entry.size as usize + 2);
        }
        assert!(loader.chip_number().is_some(), "{path}: chip");
        // RK3568 on carry FlashHead (RKNS); the idblock must agree with it.
        let block = idblock(&loader, &file).unwrap_or_else(|error| panic!("{path}: {error}"));
        let _ = writeln!(
            out,
            "  idblock: {} bytes, agrees with FlashHead",
            block.len()
        );
    }
}
