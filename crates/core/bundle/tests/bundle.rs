//! The bundle against a synthetic slot: no Gaia image exists yet (X3), so the
//! source is a double that gives the bytes Gaia's `gaia.verity` would give
//! for its shape: an EROFS data region padded to 4 KiB blocks, a verity table
//! in Gaia's `verity.table` layout, and a tree after the data.
//!
//! The tree bytes are stand-ins: the writer checks the data's root hash
//! against the table and measures the rest; it never reads the tree's layout.

use std::cell::Cell;
use std::io::Read;

use atlas_bundle::{
    BLOCK, BootFile, BundleError, SlotMeta, SlotSource, inspect_bundle, root_hash, verify_slot,
    write_bundle,
};
use atlas_release::{PublicKey, public_key_for};
use sha2::{Digest, Sha256};

/// A throwaway secret, made when the test runs and never stored.
fn throwaway(label: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(std::process::id().to_le_bytes());
    hasher.update(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |t| t.as_nanos())
            .to_le_bytes(),
    );
    hasher.update(label.as_bytes());
    hasher.finalize().into()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Deterministic bytes.
fn pattern(len: usize, seed: usize) -> Vec<u8> {
    (0..len)
        .map(|i| ((i * 7 + seed * 13) % 251) as u8)
        .collect()
}

/// A reader that returns at most `chunk` bytes a call, to prove the writer
/// does not depend on how a source chunks its reads.
struct Trickle {
    bytes: Vec<u8>,
    pos: usize,
    chunk: usize,
}

impl Read for Trickle {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = buf.len().min(self.chunk).min(self.bytes.len() - self.pos);
        buf[..n].copy_from_slice(&self.bytes[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

/// Gaia's `verity.table` text for a data region (gaia.verity's `Table::text`).
fn gaia_table(root: &str, salt_hex: &str, data_blocks: u64) -> String {
    let hash_offset = data_blocks * BLOCK as u64;
    format!(
        "hash_type 1\nalgorithm sha256\nroot_hash {root}\nsalt {salt_hex}\n\
         uuid 01234567-89ab-4cde-8f01-23456789abcd\ndata_blocks {data_blocks}\n\
         data_block_size 4096\nhash_block_size 4096\nhash_offset {hash_offset}\n\
         hash_start_block {}\ntable 0 {} verity 1 /dev/x /dev/x 4096 4096 {data_blocks} {} sha256 {root} {salt_hex}\n",
        hash_offset / BLOCK as u64 + 1,
        data_blocks * 8,
        hash_offset / BLOCK as u64 + 1,
    )
}

struct Synthetic {
    data: Vec<u8>,
    tree: Vec<u8>,
    table: String,
    boot: Vec<(String, Vec<u8>)>,
    chunk: usize,
    /// Which call to `rootfs()` gives the data with its first byte flipped.
    tamper_on_call: Option<usize>,
    rootfs_calls: Cell<usize>,
}

impl Synthetic {
    /// A slot of `blocks` data blocks, its table computed from the data.
    fn new(blocks: usize) -> Self {
        let data = pattern(blocks * BLOCK, 1);
        let salt = Sha256::digest(&data);
        let root = root_hash(&salt, &data).expect("root");
        let table = gaia_table(&root, &hex(&salt), blocks as u64);
        Self {
            data,
            tree: pattern(3 * BLOCK, 2),
            table,
            boot: vec![
                (
                    "cmdline.txt".into(),
                    b"console=tty1 root=PARTUUID=abc panic=5\n".to_vec(),
                ),
                ("kernel8.img".into(), pattern(100_000, 3)),
                ("overlays/one.dtbo".into(), pattern(777, 4)),
            ],
            chunk: 64 * 1024,
            tamper_on_call: None,
            rootfs_calls: Cell::new(0),
        }
    }

    fn rootfs_bytes(&self) -> Vec<u8> {
        let mut bytes = self.data.clone();
        bytes.extend_from_slice(&self.tree);
        bytes
    }
}

impl SlotSource for Synthetic {
    fn verity_table(&self) -> Result<String, BundleError> {
        Ok(self.table.clone())
    }

    fn rootfs(&self) -> Result<Box<dyn Read + '_>, BundleError> {
        let call = self.rootfs_calls.get() + 1;
        self.rootfs_calls.set(call);
        let mut bytes = self.rootfs_bytes();
        if self.tamper_on_call == Some(call) {
            bytes[0] ^= 1;
        }
        Ok(Box::new(Trickle {
            bytes,
            pos: 0,
            chunk: self.chunk,
        }))
    }

    fn boot_files(&self) -> Result<Vec<BootFile>, BundleError> {
        Ok(self
            .boot
            .iter()
            .map(|(path, bytes)| BootFile {
                path: path.clone(),
                size: bytes.len() as u64,
            })
            .collect())
    }

    fn boot_file(&self, path: &str) -> Result<Box<dyn Read + '_>, BundleError> {
        let (_, bytes) = self
            .boot
            .iter()
            .find(|(p, _)| p == path)
            .ok_or_else(|| BundleError::Io(format!("no boot file {path}")))?;
        Ok(Box::new(Trickle {
            bytes: bytes.clone(),
            pos: 0,
            chunk: self.chunk,
        }))
    }
}

fn meta() -> SlotMeta {
    SlotMeta {
        family: Some("example".into()),
        version: Some("2026.1.0".into()),
        ..SlotMeta::default()
    }
}

fn keys_of(secret: &[u8; 32]) -> Vec<PublicKey> {
    vec![public_key_for(secret)]
}

fn write(source: &Synthetic, secret: &[u8; 32]) -> Result<Vec<u8>, BundleError> {
    let mut out = Vec::new();
    write_bundle(source, "slot-a.1", &meta(), secret, &mut out)?;
    Ok(out)
}

/// The members of a bundle: name and contents, in order.
fn members(bundle: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut archive = tar::Archive::new(bundle);
    archive
        .entries()
        .expect("entries")
        .map(|entry| {
            let mut entry = entry.expect("entry");
            let name = entry.path().expect("path").to_string_lossy().into_owned();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).expect("read");
            (name, bytes)
        })
        .collect()
}

#[test]
fn a_bundle_has_the_members_in_order_and_its_signature_verifies() {
    let secret = throwaway("round-trip");
    let source = Synthetic::new(5);
    let bundle = write(&source, &secret).expect("writes");

    let names: Vec<String> = members(&bundle).into_iter().map(|(n, _)| n).collect();
    assert_eq!(
        names,
        [
            "slot.json",
            "slot.sig",
            "rootfs.erofs",
            "boot/cmdline.txt",
            "boot/kernel8.img",
            "boot/overlays/one.dtbo",
        ]
    );

    let all = members(&bundle);
    let json = &all[0].1;
    let sig = &all[1].1;
    assert_eq!(sig.len(), 64, "slot.sig is the raw 64-byte signature");
    assert_eq!(
        verify_slot(json, sig, &keys_of(&secret)).expect("verifies"),
        0
    );

    let inspected = inspect_bundle(&bundle[..], &keys_of(&secret)).expect("inspects");
    assert_eq!(inspected.signer, 0);
    assert_eq!(inspected.manifest.id, "slot-a.1");
    assert_eq!(
        inspected.manifest.image.size,
        (5 * BLOCK + 3 * BLOCK) as u64
    );
    assert_eq!(
        inspected.manifest.image.sha256,
        hex(&Sha256::digest(source.rootfs_bytes()))
    );
    assert_eq!(inspected.manifest.boot.files.len(), 3);
    assert_eq!(inspected.manifest.family.as_deref(), Some("example"));
}

#[test]
fn slot_json_is_schema_2_with_the_fields_lemnos_reads() {
    let secret = throwaway("schema");
    let bundle = write(&Synthetic::new(2), &secret).expect("writes");
    let json: serde_json::Value = serde_json::from_slice(&members(&bundle)[0].1).expect("json");
    assert_eq!(json["schema"], 2);
    assert_eq!(json["id"], "slot-a.1");
    assert_eq!(json["image"]["size"], 5 * BLOCK as u64);
    assert_eq!(json["image"]["sha256"].as_str().map(str::len), Some(64));
    assert_eq!(json["verity"]["root_hash"].as_str().map(str::len), Some(64));
    assert!(json["boot"]["files"]["cmdline.txt"].is_string());
    assert!(json.get("layers").is_none(), "no layered slots");
}

#[test]
fn changing_one_byte_of_slot_json_breaks_the_signature() {
    let secret = throwaway("tamper");
    let bundle = write(&Synthetic::new(3), &secret).expect("writes");
    let (_, json) = members(&bundle).swap_remove(0);
    let sig = members(&bundle).swap_remove(1).1;

    // The same bytes verify; one byte changed does not.
    assert!(verify_slot(&json, &sig, &keys_of(&secret)).is_ok());
    let mut changed = json.clone();
    let at = changed
        .windows(4)
        .position(|w| w == b"slot")
        .expect("the id is in slot.json");
    changed[at + 4] ^= 1;
    assert_ne!(changed, json);
    assert!(matches!(
        verify_slot(&changed, &sig, &keys_of(&secret)),
        Err(BundleError::BadSignature)
    ));

    // The same change inside the tar is refused by the inspector.
    let mut tampered = bundle.clone();
    let offset = 512 + at + 4; // slot.json's data starts after its 512-byte header
    assert_eq!(tampered[offset], json[at + 4]);
    tampered[offset] ^= 1;
    assert!(matches!(
        inspect_bundle(&tampered[..], &keys_of(&secret)),
        Err(BundleError::BadSignature)
    ));
}

#[test]
fn a_key_that_did_not_sign_does_not_verify() {
    let signer = throwaway("signer");
    let other = throwaway("other");
    let bundle = write(&Synthetic::new(2), &signer).expect("writes");
    assert!(matches!(
        inspect_bundle(&bundle[..], &keys_of(&other)),
        Err(BundleError::BadSignature)
    ));
    // With the right key among several, it is found and named.
    let mut keys = keys_of(&other);
    keys.extend(keys_of(&signer));
    assert_eq!(
        inspect_bundle(&bundle[..], &keys).expect("inspects").signer,
        1
    );
}

#[test]
fn the_rootfs_must_give_the_tables_root_hash() {
    let secret = throwaway("root");
    let mut source = Synthetic::new(4);
    // The table is for the data as it was; the data on the source is not.
    source.data[10] ^= 1;
    assert!(matches!(
        write(&source, &secret),
        Err(BundleError::RootHashMismatch { .. })
    ));
}

#[test]
fn a_source_that_changes_between_reads_is_refused() {
    let secret = throwaway("changed");
    let mut source = Synthetic::new(3);
    // The first read (pass one) is right; the second (the tar) is not.
    source.tamper_on_call = Some(2);
    assert!(matches!(
        write(&source, &secret),
        Err(BundleError::Changed { .. })
    ));
}

#[test]
fn the_output_does_not_depend_on_how_the_source_chunks_its_reads() {
    let secret = throwaway("chunks");
    let whole = write(&Synthetic::new(7), &secret).expect("whole");
    let mut trickle = Synthetic::new(7);
    trickle.chunk = 1;
    let bytes = write(&trickle, &secret).expect("one byte a read");
    assert_eq!(whole, bytes);
    assert_eq!(whole, write(&Synthetic::new(7), &secret).expect("again"));
}

#[test]
fn invalid_inputs_are_refused_before_anything_is_signed() {
    let secret = throwaway("refuse");
    let source = Synthetic::new(2);
    let mut out = Vec::new();
    let long = "x".repeat(65);
    for id in ["", "..", "a/b", "sp ace", long.as_str()] {
        assert!(
            write_bundle(&source, id, &meta(), &secret, &mut out).is_err(),
            "id {id:?}"
        );
    }

    let mut no_cmdline = Synthetic::new(2);
    no_cmdline.boot.retain(|(p, _)| p != "cmdline.txt");
    assert!(matches!(
        write(&no_cmdline, &secret),
        Err(BundleError::MissingCmdline)
    ));

    for bad in ["../evil", "/abs", "a//b", "a/./b", "x\\y"] {
        let mut source = Synthetic::new(2);
        source.boot.push((bad.into(), vec![1]));
        assert!(
            matches!(
                write(&source, &secret),
                Err(BundleError::InvalidBootPath(_))
            ),
            "boot path {bad:?}"
        );
    }

    let mut twice = Synthetic::new(2);
    twice.boot.push(("kernel8.img".into(), vec![1]));
    assert!(matches!(
        write(&twice, &secret),
        Err(BundleError::DuplicateBootFile(_))
    ));
}

#[test]
fn a_table_that_is_not_gaias_format_is_refused() {
    let secret = throwaway("table");
    let mut source = Synthetic::new(2);
    source.table = source.table.replace("hash_type 1", "hash_type 2");
    assert!(matches!(
        write(&source, &secret),
        Err(BundleError::Table(_))
    ));
}

#[test]
fn a_bundle_with_an_extra_member_is_refused() {
    let secret = throwaway("extra");
    let bundle = write(&Synthetic::new(2), &secret).expect("writes");
    let mut out = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut out);
        for (name, bytes) in members(&bundle) {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_entry_type(tar::EntryType::Regular);
            builder
                .append_data(&mut header, &name, &bytes[..])
                .expect("append");
        }
        let extra = b"not part of a slot";
        let mut header = tar::Header::new_gnu();
        header.set_size(extra.len() as u64);
        header.set_mode(0o644);
        header.set_entry_type(tar::EntryType::Regular);
        builder
            .append_data(&mut header, "boot/extra.txt", &extra[..])
            .expect("append");
        builder.finish().expect("finish");
    }
    assert!(matches!(
        inspect_bundle(&out[..], &keys_of(&secret)),
        Err(BundleError::Malformed(_))
    ));
}

#[test]
fn a_larger_slot_is_streamed_with_the_same_checks() {
    // 4 MiB of data: 1024 blocks, a two-level tree, read through 64 KiB
    // chunks. The writer keeps to fixed buffers; this only checks the result.
    let secret = throwaway("large");
    let source = Synthetic::new(1024);
    let bundle = write(&source, &secret).expect("writes");
    let inspected = inspect_bundle(&bundle[..], &keys_of(&secret)).expect("inspects");
    assert_eq!(inspected.manifest.verity.root_hash, {
        let data = &source.data;
        root_hash(&Sha256::digest(data), data).expect("root")
    });
}
