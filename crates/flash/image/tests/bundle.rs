//! The connection to the bundle writer: a slot bundle made by `atlas-bundle`
//! from a slot read out of an image matches the image's parts, member by member.
//!
//! The adapter below is the whole connection. It implements the bundle's
//! `SlotSource` over this crate's `Slot`; nothing in the bundle crate changes.

mod common;

use std::io::Read;

use atlas_bundle::{BootFile, BundleError, SlotMeta, SlotSource, write_bundle};
use atlas_image::{ImageError, Options, Slot, open};
use common::*;
use sha2::{Digest, Sha256};

struct FromImage<'a>(Slot<'a>);

fn to_bundle(error: ImageError) -> BundleError {
    BundleError::Io(error.to_string())
}

impl SlotSource for FromImage<'_> {
    fn verity_table(&self) -> Result<String, BundleError> {
        self.0.verity_table().map_err(to_bundle)
    }

    fn rootfs(&self) -> Result<Box<dyn Read + '_>, BundleError> {
        self.0.rootfs().map_err(to_bundle)
    }

    fn boot_files(&self) -> Result<Vec<BootFile>, BundleError> {
        Ok(self
            .0
            .boot_files()
            .map_err(to_bundle)?
            .into_iter()
            .map(|file| BootFile {
                path: file.path,
                size: file.size,
            })
            .collect())
    }

    fn boot_file(&self, path: &str) -> Result<Box<dyn Read + '_>, BundleError> {
        self.0.boot_file(path).map_err(to_bundle)
    }
}

/// The bundle's members, by name, with their bytes.
fn members(bundle: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut archive = tar::Archive::new(bundle);
    let mut out = Vec::new();
    for entry in archive.entries().expect("tar") {
        let mut entry = entry.expect("entry");
        let name = entry.path().expect("path").to_string_lossy().into_owned();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).expect("read member");
        out.push((name, bytes));
    }
    out
}

fn check_bundle_matches_parts(image_path: &std::path::Path, scratch: &std::path::Path) {
    let image = open(image_path, scratch, &["alpha"], &Options::default()).expect("open");
    let source = FromImage(image.slot("alpha").expect("slot"));
    let mut bundle = Vec::new();
    let manifest = write_bundle(
        &source,
        "alpha",
        &SlotMeta::default(),
        &[7; 32],
        &mut bundle,
    )
    .expect("bundle");

    let slot = slot_file();
    assert_eq!(manifest.image.size, slot.len() as u64);
    let digest: String = Sha256::digest(&slot)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(manifest.image.sha256, digest);

    let found = members(&bundle);
    let mut expected: Vec<(String, Vec<u8>)> = vec![("rootfs.erofs".into(), slot)];
    for (path, bytes) in real_boot() {
        expected.push((format!("boot/{path}"), bytes));
    }
    for (name, bytes) in &expected {
        let (_, got) = found
            .iter()
            .find(|(member, _)| member == name)
            .unwrap_or_else(|| panic!("the bundle has no {name}"));
        assert_eq!(got, bytes, "{name} differs from the image");
    }
    let names: Vec<&str> = found.iter().map(|(name, _)| name.as_str()).collect();
    assert!(
        names.starts_with(&["slot.json", "slot.sig", "rootfs.erofs"]),
        "{names:?}"
    );
    assert!(!names.iter().any(|name| name.contains("beta")), "{names:?}");
    assert_eq!(names.len(), 3 + expected.len() - 1, "{names:?}");
}

#[test]
fn a_bundle_from_the_image_matches_its_parts_from_xz() {
    let dir = TestDir::new("bundle-xz");
    let scratch = TestDir::new("bundle-xz-scratch");
    let path = dir.write("first-flash.img.xz", &xz(&real_disk()));
    check_bundle_matches_parts(&path, scratch.path());
}

#[test]
fn a_bundle_from_a_plain_img_matches_its_parts() {
    let dir = TestDir::new("bundle-img");
    let scratch = TestDir::new("bundle-img-scratch");
    let path = dir.write("first-flash.img", &real_disk());
    check_bundle_matches_parts(&path, scratch.path());
}

#[test]
fn the_verity_table_the_bundle_checks_is_the_images() {
    // The writer refuses a tree whose root does not match the table. The fixture
    // table was computed by the Python script; this is the bundle crate's own
    // check of that table against the slot's data.
    let dir = TestDir::new("bundle-root");
    let scratch = TestDir::new("bundle-root-scratch");
    let path = dir.write("first-flash.img.xz", &xz(&real_disk()));
    let image = open(&path, scratch.path(), &["alpha"], &Options::default()).expect("open");
    let source = FromImage(image.slot("alpha").expect("slot"));
    let mut sink = Vec::new();
    let result = write_bundle(&source, "alpha", &SlotMeta::default(), &[7; 32], &mut sink);
    assert!(result.is_ok(), "{result:?}");
}
