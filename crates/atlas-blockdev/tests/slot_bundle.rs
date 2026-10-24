//! A slot bundle from a real slots image: `ATLAS_SLOT_IMAGE=<id>.img.xz`
//! (and `ATLAS_SLOT_OUT=<dir>`, default the temp dir) with `--ignored`.
//! With `ATLAS_SLOT_BUNDLE=<id>.slot.tar`, the build's own bundle, every
//! member must match it byte for byte.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::PathBuf;

fn members(path: &std::path::Path) -> BTreeMap<String, Vec<u8>> {
    let mut archive = tar::Archive::new(std::fs::File::open(path).unwrap());
    let mut out = BTreeMap::new();
    for entry in archive.entries().unwrap() {
        let mut entry = entry.unwrap();
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let name = entry
            .path()
            .unwrap()
            .to_string_lossy()
            .trim_start_matches("./")
            .to_string();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        out.insert(name, bytes);
    }
    out
}

#[test]
#[ignore = "needs ATLAS_SLOT_IMAGE"]
fn a_real_slots_image_gives_the_builds_bundle() {
    let image = PathBuf::from(std::env::var("ATLAS_SLOT_IMAGE").unwrap());
    let out = std::env::var("ATLAS_SLOT_OUT").map_or_else(|_| std::env::temp_dir(), PathBuf::from);
    let made = atlas_blockdev::slot_bundle_from_image(&image, &out).unwrap();
    let ours = members(&made.path);
    assert!(ours.contains_key("slot.json") && ours.contains_key("rootfs.erofs"));
    if let Ok(theirs) = std::env::var("ATLAS_SLOT_BUNDLE") {
        let theirs = members(std::path::Path::new(&theirs));
        assert_eq!(
            ours.keys().collect::<Vec<_>>(),
            theirs.keys().collect::<Vec<_>>()
        );
        for (name, bytes) in &theirs {
            assert!(ours[name] == *bytes, "{name} differs");
        }
    }
}
