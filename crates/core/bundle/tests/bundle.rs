//! The bundle against a synthetic slot: the writer and the reader checked against
//! a double (the image path is in tests/image.rs). The double is a data region of
//! whole 4 KiB blocks, the four verity values for it, and a tree after the data of
//! the length the layout gives.
//!
//! The tree bytes are stand-ins: the writer checks the data's root hash against
//! the values and measures the tree's length; it does not recompute the tree (see
//! tests/veritysetup.rs for the layout checked against the real tool).

use std::cell::Cell;
use std::io::Read;

use atlas_bundle::{
    BLOCK, BOOT_FILES, BootFile, BootRule, BundleError, RECORD_LEN, Signature, SlotMeta,
    SlotSource, inspect_bundle, root_hash, slot_message, verify_slot, write_bundle,
};
use atlas_image::{Verity, VerityError, tree_bytes};
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

struct Synthetic {
    data: Vec<u8>,
    tree: Vec<u8>,
    verity: Verity,
    boot: Vec<(String, Vec<u8>)>,
    chunk: usize,
    /// Which call to `rootfs()` gives the data with its first byte flipped.
    tamper_on_call: Option<usize>,
    rootfs_calls: Cell<usize>,
}

impl Synthetic {
    /// A slot of `blocks` data blocks, its verity values computed from the data
    /// and a tree of the length the layout gives for that many blocks.
    fn new(blocks: usize) -> Self {
        let data = pattern(blocks * BLOCK, 1);
        let salt = Sha256::digest(&data);
        let root = root_hash(&salt, &data).expect("root");
        let verity = Verity::from_manifest(
            &root,
            Some(&hex(&salt)),
            blocks as u64,
            (blocks * BLOCK) as u64,
        )
        .expect("the values fit the layout");
        Self {
            data,
            tree: pattern(tree_bytes(blocks as u64) as usize, 2),
            verity,
            boot: vec![
                ("kernel8.img".into(), pattern(100_000, 3)),
                ("initramfs.cpio".into(), pattern(5_000, 5)),
                ("board.dtb".into(), pattern(900, 6)),
                ("cmdline.txt".into(), CMDLINE.to_vec()),
                ("overlays/lemnos-board.dtbo".into(), pattern(777, 4)),
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
    fn verity(&self) -> Result<Verity, BundleError> {
        Ok(self.verity.clone())
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

/// The complete command line the default board rule takes: its one boot-source word is on it.
const CMDLINE: &[u8] =
    b"console=tty1 root=PARTUUID=abc panic=5 os.src.rpi-boot-mode-1=/soc@107c000000/mmc@fff000\n";

fn meta() -> SlotMeta {
    SlotMeta {
        product: "example".into(),
        version: "2026.1.0".into(),
        board: "example-board".into(),
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
            "boot/board.dtb",
            "boot/cmdline.txt",
            "boot/initramfs.cpio",
            "boot/kernel8.img",
            "boot/overlays/lemnos-board.dtbo",
        ]
    );

    let all = members(&bundle);
    let json = &all[0].1;
    let sig = &all[1].1;
    assert_eq!(sig.len(), RECORD_LEN, "slot.sig is the 108-byte record");
    assert_eq!(
        verify_slot(json, sig, &keys_of(&secret)).expect("verifies"),
        Signature::Verified { signer: 0 }
    );

    let inspected =
        inspect_bundle(&bundle[..], &keys_of(&secret), &BootRule::default()).expect("inspects");
    assert_eq!(inspected.signature, Signature::Verified { signer: 0 });
    assert_eq!(inspected.manifest.id, "slot-a.1");
    assert_eq!(
        inspected.manifest.image.size,
        (5 * BLOCK) as u64 + tree_bytes(5)
    );
    assert_eq!(
        inspected.manifest.image.sha256,
        hex(&Sha256::digest(source.rootfs_bytes()))
    );
    assert_eq!(inspected.manifest.boot.files.len(), BOOT_FILES.len());
    assert_eq!(inspected.manifest.product, "example");
    assert_eq!(inspected.manifest.board, "example-board");
    assert_eq!(inspected.board, "example-board");
    assert_eq!(inspected.product, "example");
    assert_eq!(inspected.version, "2026.1.0");
}

#[test]
fn slot_json_is_schema_3_with_the_fields_lemnos_reads() {
    let secret = throwaway("schema");
    let bundle = write(&Synthetic::new(2), &secret).expect("writes");
    let json: serde_json::Value = serde_json::from_slice(&members(&bundle)[0].1).expect("json");
    assert_eq!(json["schema"], 3);
    assert_eq!(json["product"], "example");
    assert_eq!(json["version"], "2026.1.0");
    assert_eq!(json["board"], "example-board");
    assert!(json.get("family").is_none(), "no family key in schema 3");
    assert!(json.get("variant").is_none(), "absent when not given");
    assert_eq!(json["id"], "slot-a.1");
    assert_eq!(json["image"]["size"], 2 * BLOCK as u64 + tree_bytes(2));
    assert_eq!(json["image"]["sha256"].as_str().map(str::len), Some(64));
    assert_eq!(json["verity"]["root_hash"].as_str().map(str::len), Some(64));
    assert_eq!(json["verity"]["salt"].as_str().map(str::len), Some(64));
    assert_eq!(json["verity"]["data_blocks"], 2);
    assert_eq!(json["verity"]["hash_offset"], 2 * BLOCK as u64);
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
        inspect_bundle(&tampered[..], &keys_of(&secret), &BootRule::default()),
        Err(BundleError::BadSignature)
    ));
}

#[test]
fn a_key_the_reader_does_not_hold_is_reported_not_refused() {
    let signer = throwaway("signer");
    let other = throwaway("other");
    let bundle = write(&Synthetic::new(2), &signer).expect("writes");
    let inspected = inspect_bundle(&bundle[..], &keys_of(&other), &BootRule::default())
        .expect("an unknown key is not refused");
    let Signature::UnknownKey { key_id } = inspected.signature else {
        panic!("expected an unknown key, got {:?}", inspected.signature);
    };
    assert_eq!(
        key_id,
        hex(&atlas_bundle::key_id_of(&public_key_for(&signer)))
    );
    // With the right key among several, it is found and named by its index.
    let mut keys = keys_of(&other);
    keys.extend(keys_of(&signer));
    assert_eq!(
        inspect_bundle(&bundle[..], &keys, &BootRule::default())
            .expect("inspects")
            .signature,
        Signature::Verified { signer: 1 }
    );
}

#[test]
fn the_rootfs_must_give_the_root_hash() {
    let secret = throwaway("root");
    let mut source = Synthetic::new(4);
    // The root hash is for the data as it was; the data on the source is not.
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

    // Each of the five boot files is required: a source without it is refused by name.
    for name in BOOT_FILES {
        let mut missing = Synthetic::new(2);
        missing.boot.retain(|(p, _)| p != name);
        assert!(
            matches!(write(&missing, &secret), Err(BundleError::MissingBootFile(ref n)) if n == name),
            "{name}"
        );
    }
    // A sixth boot file is refused by name (Lemnos I5i9 will refuse it at install).
    let mut sixth = Synthetic::new(2);
    sixth.boot.push(("overlays/extra.dtbo".into(), vec![1]));
    assert!(matches!(
        write(&sixth, &secret),
        Err(BundleError::ExtraBootFile(ref n)) if n == "overlays/extra.dtbo"
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
fn a_wrong_salt_is_refused_by_the_root_hash() {
    let secret = throwaway("salt");
    let mut source = Synthetic::new(4);
    // The root was made with the real salt; the source now says another.
    source.verity.salt = [0x11; 32];
    assert!(matches!(
        write(&source, &secret),
        Err(BundleError::RootHashMismatch { .. })
    ));
}

#[test]
fn a_wrong_block_count_is_refused_by_the_file_size_rule() {
    let secret = throwaway("blocks");
    let blocks = |source: &mut Synthetic, n: u64| {
        source.verity = Verity::from_manifest(
            &source.verity.root_hash,
            Some(&source.verity.salt_hex()),
            n,
            n * BLOCK as u64,
        )
        .expect("the offset is made to fit, so only the count is wrong");
    };
    // One block more and one block fewer than the data, each with its offset made to fit.
    for wrong in [5, 3] {
        let mut source = Synthetic::new(4);
        blocks(&mut source, wrong);
        assert!(
            matches!(
                write(&source, &secret),
                Err(BundleError::Verity(VerityError::Size { .. }))
            ),
            "{wrong} blocks"
        );
    }
}

#[test]
fn a_wrong_offset_is_refused_before_anything_is_read() {
    let secret = throwaway("offset");
    let mut source = Synthetic::new(4);
    source.verity.hash_offset = 3 * BLOCK as u64;
    assert!(matches!(
        write(&source, &secret),
        Err(BundleError::Verity(VerityError::Offset { .. }))
    ));
}

#[test]
fn a_truncated_tree_is_refused_by_the_size_rule() {
    let secret = throwaway("truncated");
    let mut source = Synthetic::new(4);
    source.tree.pop();
    assert!(matches!(
        write(&source, &secret),
        Err(BundleError::Verity(VerityError::Size { .. }))
    ));
}

#[test]
fn a_file_shorter_than_its_data_is_refused_by_the_size_rule() {
    let secret = throwaway("short");
    let mut source = Synthetic::new(4);
    source.data.truncate(3 * BLOCK);
    source.tree.clear();
    assert!(matches!(
        write(&source, &secret),
        Err(BundleError::Verity(VerityError::Size { .. }))
    ));
}

/// A bundle's `slot.json` with the given verity values and a valid rest, signed.
/// A valid salt for the manifest tests: 64 hex digits.
const SALT_HEX: &str = "5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a";

/// A bundle's `slot.json` with the given verity values, the rest valid. `salt` is
/// the JSON text of the salt value (quoted), or `None` to leave the key out.
fn manifest_json_with(
    salt: Option<&str>,
    data_blocks: u64,
    hash_offset: u64,
    size: u64,
) -> Vec<u8> {
    let zeros = "0".repeat(64);
    let salt = salt.map_or(String::new(), |s| format!(r#""salt":"{s}","#));
    format!(
        r#"{{"schema":3,"id":"slot-a.1","product":"example","version":"1","board":"example-board","image":{{"size":{size},"sha256":"{zeros}"}},"verity":{{"root_hash":"{zeros}",{salt}"data_blocks":{data_blocks},"hash_offset":{hash_offset}}},"boot":{{"files":{{"kernel8.img":"{zeros}","initramfs.cpio":"{zeros}","board.dtb":"{zeros}","cmdline.txt":"{zeros}","overlays/lemnos-board.dtbo":"{zeros}"}}}}}}"#
    )
    .into_bytes()
}

fn manifest_json(data_blocks: u64, hash_offset: u64, size: u64) -> Vec<u8> {
    manifest_json_with(Some(SALT_HEX), data_blocks, hash_offset, size)
}

#[test]
fn a_manifest_whose_salt_is_missing_empty_or_the_wrong_length_is_refused_by_name() {
    let secret = throwaway("saltjson");
    let cases = [
        (None, VerityError::SaltMissing),
        (Some(""), VerityError::SaltEmpty),
        (Some("zz"), VerityError::SaltLength { digits: 2 }),
        (Some(&SALT_HEX[2..]), VerityError::SaltLength { digits: 62 }),
    ];
    for (salt, expected) in cases {
        let json = manifest_json_with(salt, 1, 4096, 4096);
        let bundle = bundle_with_manifest(&json, &secret);
        let error = inspect_bundle(&bundle[..], &keys_of(&secret), &BootRule::default())
            .expect_err("refused");
        assert!(
            matches!(&error, BundleError::Verity(e) if *e == expected),
            "{salt:?}: {error:?}"
        );
    }
}

#[test]
fn a_manifest_whose_offset_or_size_do_not_fit_is_refused_by_name() {
    let secret = throwaway("layoutjson");
    let offset = bundle_with_manifest(&manifest_json(1, 8192, 8192), &secret);
    assert!(matches!(
        inspect_bundle(&offset[..], &keys_of(&secret), &BootRule::default()),
        Err(BundleError::Verity(VerityError::Offset { .. }))
    ));
    // One data block has no tree, so 4096 is its whole length; 8192 is too long.
    let size = bundle_with_manifest(&manifest_json(1, 4096, 8192), &secret);
    assert!(matches!(
        inspect_bundle(&size[..], &keys_of(&secret), &BootRule::default()),
        Err(BundleError::Verity(VerityError::Size { .. }))
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
        inspect_bundle(&out[..], &keys_of(&secret), &BootRule::default()),
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
    let inspected =
        inspect_bundle(&bundle[..], &keys_of(&secret), &BootRule::default()).expect("inspects");
    assert_eq!(inspected.manifest.verity.root_hash, {
        let data = &source.data;
        root_hash(&Sha256::digest(data), data).expect("root")
    });
}

/// A bundle whose `slot.json` is `json` (signed by `secret` as given), with
/// the rest of a slot's members.
fn bundle_with_manifest(json: &[u8], secret: &[u8; 32]) -> Vec<u8> {
    let sig = atlas_bundle::sign_slot(json, secret)
        .expect("signs")
        .to_bytes();
    let mut out = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut out);
        let mut add = |name: &str, bytes: &[u8]| {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_entry_type(tar::EntryType::Regular);
            builder
                .append_data(&mut header, name, bytes)
                .expect("append");
        };
        add("slot.json", json);
        add("slot.sig", &sig);
        add("rootfs.erofs", &[0u8; 8]);
        builder.finish().expect("finish");
    }
    out
}

#[test]
fn schema_2_input_is_refused_by_name() {
    let secret = throwaway("schema2");
    let json = br#"{"schema":2,"id":"slot-a.1","family":"example","image":{"size":8,"sha256":"00"},"verity":{"root_hash":"00"},"boot":{"files":{}}}"#;
    let bundle = bundle_with_manifest(json, &secret);
    let error =
        inspect_bundle(&bundle[..], &keys_of(&secret), &BootRule::default()).expect_err("refused");
    let text = error.to_string();
    assert!(text.contains("schema 2"), "names the schema found: {text}");
    assert!(matches!(error, BundleError::Malformed(_)));
}

#[test]
fn a_family_key_is_refused_by_name() {
    let secret = throwaway("family");
    let json = br#"{"schema":3,"id":"slot-a.1","family":"example","product":"example","version":"1","board":"example-board","image":{"size":8,"sha256":"00"},"verity":{"root_hash":"00"},"boot":{"files":{}}}"#;
    let bundle = bundle_with_manifest(json, &secret);
    let error =
        inspect_bundle(&bundle[..], &keys_of(&secret), &BootRule::default()).expect_err("refused");
    assert!(error.to_string().contains("family"), "{error}");
}

#[test]
fn a_bundle_without_a_board_is_refused() {
    let secret = throwaway("noboard");
    let json = br#"{"schema":3,"id":"slot-a.1","product":"example","version":"1","image":{"size":8,"sha256":"00"},"verity":{"root_hash":"00","salt":"5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a","data_blocks":1,"hash_offset":4096},"boot":{"files":{}}}"#;
    let bundle = bundle_with_manifest(json, &secret);
    let error =
        inspect_bundle(&bundle[..], &keys_of(&secret), &BootRule::default()).expect_err("refused");
    assert!(error.to_string().contains("board"), "{error}");
}

#[test]
fn a_slot_for_another_board_is_refused_naming_both() {
    let secret = throwaway("board");
    let bundle = write(&Synthetic::new(2), &secret).expect("writes");
    let inspected =
        inspect_bundle(&bundle[..], &keys_of(&secret), &BootRule::default()).expect("inspects");
    assert!(atlas_bundle::check_board(&inspected.board, "example-board").is_ok());
    let error = atlas_bundle::check_board(&inspected.board, "other-board").expect_err("refused");
    let text = error.to_string();
    assert!(
        text.contains("example-board") && text.contains("other-board"),
        "{text}"
    );
}

#[test]
fn a_board_or_product_name_outside_the_rules_is_refused() {
    let mut source = Synthetic::new(2);
    let secret = throwaway("names");
    let mut bad = meta();
    bad.board = "Example_Board".into();
    let mut out = Vec::new();
    assert!(matches!(
        write_bundle(&source, "slot-a.1", &bad, &secret, &mut out),
        Err(BundleError::InvalidName { field: "board", .. })
    ));
    bad = meta();
    bad.product = String::new();
    assert!(matches!(
        write_bundle(&source, "slot-a.1", &bad, &secret, &mut out),
        Err(BundleError::InvalidName {
            field: "product",
            ..
        })
    ));
    source.boot.clear();
}

// ---- slot.sig: the record, the message, the order, the policy (Lemnos I5g2, I5i4) ----

/// The fixed vector of Lemnos I5g2: Ed25519 seed 0x07 x32 signs the message of `{"id":"vector"}`.
const VECTOR_SEED: [u8; 32] = [0x07; 32];
const VECTOR_JSON: &[u8] = br#"{"id":"vector"}"#;
const VECTOR_MESSAGE: &str =
    "6c656d6e6f732d736c6f742d62756e646c6500010000000f0000007b226964223a22766563746f72227d";
const VECTOR_RECORD: &str = "4c4d4e535349473101000000\
fe812c12f3ab4ce6ac5db69ac352f906cb1b11ef43fb33e252ef7ff552263889\
56be579db020703eb207da579036a89f0321fcd8ce2bac78a9aa939a58217e265ad27a7e1fc05829b0db05d37ac4d8dba35263c7a1b2644322b45ada4a101402";

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).expect("hex"))
        .collect()
}

/// The first byte at which two byte strings differ, for the message.
fn first_difference(a: &[u8], b: &[u8]) -> Option<usize> {
    a.iter()
        .zip(b)
        .position(|(x, y)| x != y)
        .or_else(|| (a.len() != b.len()).then(|| a.len().min(b.len())))
}

#[test]
fn the_fixed_vector_is_reproduced_byte_for_byte() {
    let message = slot_message(VECTOR_JSON).expect("the message");
    let expected = unhex(VECTOR_MESSAGE);
    if let Some(at) = first_difference(&message, &expected) {
        panic!(
            "the message differs at byte {at}: ours {:02x?}, the vector {:02x?}",
            message.get(at),
            expected.get(at)
        );
    }
    let record = atlas_bundle::sign_slot(VECTOR_JSON, &VECTOR_SEED)
        .expect("signs")
        .to_bytes();
    let expected = unhex(VECTOR_RECORD);
    assert_eq!(expected.len(), RECORD_LEN, "the vector is 108 bytes");
    if let Some(at) = first_difference(&record, &expected) {
        panic!(
            "the record differs at byte {at}: ours {:02x?}, the vector {:02x?}",
            record.get(at),
            expected.get(at)
        );
    }
    // The vector's key is the seed's public key, and the record verifies under it.
    let key = public_key_for(&VECTOR_SEED);
    assert_eq!(
        verify_slot(VECTOR_JSON, &expected, std::slice::from_ref(&key)).expect("verifies"),
        Signature::Verified { signer: 0 }
    );
}

/// The members of a bundle, re-tarred in the order given.
fn retar(parts: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut out);
        for (name, bytes) in parts {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_entry_type(tar::EntryType::Regular);
            builder
                .append_data(&mut header, name, &bytes[..])
                .expect("append");
        }
        builder.finish().expect("finish");
    }
    out
}

/// A written bundle with one member replaced by `bytes`, or removed when `None`.
fn with_member(bundle: &[u8], name: &str, bytes: Option<Vec<u8>>) -> Vec<u8> {
    let mut parts = members(bundle);
    match (parts.iter().position(|(n, _)| n == name), bytes) {
        (Some(at), Some(bytes)) => parts[at].1 = bytes,
        (Some(at), None) => {
            parts.remove(at);
        }
        (None, Some(bytes)) => parts.push((name.to_string(), bytes)),
        (None, None) => {}
    }
    retar(&parts)
}

#[test]
fn a_signature_over_the_bare_manifest_bytes_by_a_held_key_is_refused() {
    let secret = throwaway("bare");
    let bundle = write(&Synthetic::new(2), &secret).expect("writes");
    let json = members(&bundle).swap_remove(0).1;
    // The same key, the right key id, but the signature is over the bare bytes: the old form.
    let bare = atlas_release::sign_bytes(&json, &secret);
    let mut record = members(&bundle).swap_remove(1).1;
    record[44..108].copy_from_slice(&bare);
    assert!(matches!(
        verify_slot(&json, &record, &keys_of(&secret)),
        Err(BundleError::BadSignature)
    ));
    let tampered = with_member(&bundle, "slot.sig", Some(record));
    assert!(matches!(
        inspect_bundle(&tampered[..], &keys_of(&secret), &BootRule::default()),
        Err(BundleError::BadSignature)
    ));
}

#[test]
fn a_malformed_signature_record_is_refused_whatever_the_key_list() {
    let secret = throwaway("malformed");
    let bundle = write(&Synthetic::new(2), &secret).expect("writes");
    let good = members(&bundle).swap_remove(1).1;
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("64 bytes, the old bare form", good[..64].to_vec()),
        ("107 bytes", good[..107].to_vec()),
        ("a bad magic", {
            let mut bytes = good.clone();
            bytes[0] ^= 1;
            bytes
        }),
        ("algorithm 2", {
            let mut bytes = good.clone();
            bytes[8] = 2;
            bytes
        }),
        ("padding that is not zero", {
            let mut bytes = good.clone();
            bytes[10] = 1;
            bytes
        }),
    ];
    let json = members(&bundle).swap_remove(0).1;
    for (what, record) in cases {
        let refused = verify_slot(&json, &record, &keys_of(&secret));
        assert!(
            matches!(refused, Err(BundleError::Malformed(_))),
            "{what}: {refused:?}"
        );
        // Refused with no key list too: the record is checked first.
        assert!(matches!(
            verify_slot(&json, &record, &[]),
            Err(BundleError::Malformed(_))
        ));
        let tampered = with_member(&bundle, "slot.sig", Some(record));
        assert!(
            matches!(
                inspect_bundle(&tampered[..], &[], &BootRule::default()),
                Err(BundleError::Malformed(_))
            ),
            "{what} in a bundle"
        );
    }
}

#[test]
fn an_unsigned_bundle_is_legal_and_says_so() {
    let secret = throwaway("unsigned");
    let bundle = write(&Synthetic::new(2), &secret).expect("writes");
    let unsigned = with_member(&bundle, "slot.sig", None);
    let names: Vec<String> = members(&unsigned).into_iter().map(|(n, _)| n).collect();
    assert_eq!(names[0], "slot.json");
    assert_eq!(names[1], "rootfs.erofs");
    let inspected = inspect_bundle(&unsigned[..], &keys_of(&secret), &BootRule::default())
        .expect("an unsigned bundle inspects");
    assert_eq!(inspected.signature, Signature::Unsigned);
    assert_eq!(inspected.board, "example-board");
}

#[test]
fn slot_sig_must_be_second_and_slot_json_first() {
    let secret = throwaway("order");
    let bundle = write(&Synthetic::new(2), &secret).expect("writes");
    let parts = members(&bundle);
    let (json, sig, rest) = (&parts[0], &parts[1], &parts[2..]);

    // slot.sig after the rootfs.
    let mut late = vec![json.clone(), rest[0].clone(), sig.clone()];
    late.extend_from_slice(&rest[1..]);
    let error = inspect_bundle(&retar(&late)[..], &keys_of(&secret), &BootRule::default())
        .expect_err("refused");
    assert!(
        matches!(&error, BundleError::Malformed(m) if m.contains("slot.sig is out of order")),
        "{error:?}"
    );

    // slot.json not first.
    let swapped = vec![sig.clone(), json.clone()];
    let error = inspect_bundle(
        &retar(&swapped)[..],
        &keys_of(&secret),
        &BootRule::default(),
    )
    .expect_err("refused");
    assert!(
        matches!(&error, BundleError::Malformed(m) if m.contains("slot.json must be the first")),
        "{error:?}"
    );
}

#[test]
fn a_payload_member_before_slot_json_is_refused_by_the_order_rule() {
    let secret = throwaway("payload-first");
    let bundle = write(&Synthetic::new(2), &secret).expect("writes");
    let parts = members(&bundle);
    let mut first = vec![parts[2].clone()];
    first.extend_from_slice(&parts[..2]);
    let error = inspect_bundle(&retar(&first)[..], &keys_of(&secret), &BootRule::default())
        .expect_err("refused");
    assert!(
        matches!(&error, BundleError::Malformed(m) if m.contains("slot.json must be the first")),
        "{error:?}"
    );
}

#[test]
fn a_member_outside_the_set_is_refused_by_name() {
    let secret = throwaway("unlisted");
    let bundle = write(&Synthetic::new(2), &secret).expect("writes");
    // The old form's slot.key member is not a slot member any more.
    let with_key = with_member(&bundle, "slot.key", Some(vec![b'0'; 64]));
    let error = inspect_bundle(&with_key[..], &keys_of(&secret), &BootRule::default())
        .expect_err("refused");
    assert!(
        matches!(&error, BundleError::Malformed(m) if m.contains("slot.key is not part of a slot bundle")),
        "{error:?}"
    );
}

#[test]
fn a_listed_member_that_is_missing_is_refused_by_name() {
    let secret = throwaway("missing");
    let bundle = write(&Synthetic::new(2), &secret).expect("writes");
    let without = with_member(&bundle, "boot/overlays/lemnos-board.dtbo", None);
    let error =
        inspect_bundle(&without[..], &keys_of(&secret), &BootRule::default()).expect_err("refused");
    assert!(
        matches!(&error, BundleError::Malformed(m) if m == "the bundle has no boot file overlays/lemnos-board.dtbo"),
        "{error:?}"
    );
}

/// A written bundle whose `slot.json` is edited by `edit` and re-signed by `secret`: the rest of its
/// members as they were.
fn resigned(
    bundle: &[u8],
    secret: &[u8; 32],
    edit: impl FnOnce(&mut serde_json::Value),
) -> Vec<u8> {
    let mut parts = members(bundle);
    let mut json: serde_json::Value = serde_json::from_slice(&parts[0].1).expect("json");
    edit(&mut json);
    let bytes = serde_json::to_vec(&json).expect("json bytes");
    let sig = atlas_bundle::sign_slot(&bytes, secret)
        .expect("signs")
        .to_bytes();
    parts[0].1 = bytes;
    parts[1].1 = sig.to_vec();
    retar(&parts)
}

#[test]
fn each_boot_file_the_manifest_does_not_list_is_refused_by_the_reader() {
    // The reader's check: a slot.json that lists four of the five is refused, naming the one it
    // leaves out, before any member is believed.
    let secret = throwaway("unlisted-role");
    let bundle = write(&Synthetic::new(2), &secret).expect("writes");
    for name in BOOT_FILES {
        let edited = resigned(&bundle, &secret, |json| {
            json["boot"]["files"]
                .as_object_mut()
                .expect("files")
                .remove(name);
        });
        let error = inspect_bundle(&edited[..], &keys_of(&secret), &BootRule::default())
            .expect_err("refused");
        assert!(
            matches!(&error, BundleError::MissingBootFile(n) if n == name),
            "{name}: {error:?}"
        );
    }
    // A sixth entry, listed and carried, is refused by name.
    let sixth = resigned(&bundle, &secret, |json| {
        json["boot"]["files"]["overlays/extra.dtbo"] = serde_json::json!("00".repeat(32));
    });
    let error =
        inspect_bundle(&sixth[..], &keys_of(&secret), &BootRule::default()).expect_err("refused");
    assert!(
        matches!(&error, BundleError::ExtraBootFile(n) if n == "overlays/extra.dtbo"),
        "{error:?}"
    );
}

#[test]
fn a_member_that_does_not_match_its_manifest_digest_is_refused() {
    let secret = throwaway("digest");
    let bundle = write(&Synthetic::new(2), &secret).expect("writes");
    let mut kernel = members(&bundle)
        .into_iter()
        .find(|(n, _)| n == "boot/kernel8.img")
        .expect("kernel")
        .1;
    kernel[0] ^= 1;
    let changed = with_member(&bundle, "boot/kernel8.img", Some(kernel));
    let error =
        inspect_bundle(&changed[..], &keys_of(&secret), &BootRule::default()).expect_err("refused");
    assert!(
        matches!(&error, BundleError::Malformed(m) if m.contains("boot/kernel8.img doesn't match")),
        "{error:?}"
    );
}

#[test]
fn the_cmdline_must_carry_the_word_for_each_boot_source() {
    let secret = throwaway("cmdline");
    let bundle = write(&Synthetic::new(2), &secret).expect("writes");
    // The default rule: the Synthetic's line carries its word, so it inspects.
    assert!(inspect_bundle(&bundle[..], &keys_of(&secret), &BootRule::default()).is_ok());
    // A second source the line does not carry is refused, naming the word.
    let rule = BootRule {
        boot_args: "os".into(),
        sources: vec![
            (
                "rpi-boot-mode-1".into(),
                "/soc@107c000000/mmc@fff000".into(),
            ),
            ("mmc1".into(), "/soc/mmc@fe2e0000".into()),
        ],
    };
    let error = inspect_bundle(&bundle[..], &keys_of(&secret), &rule).expect_err("refused");
    assert!(
        matches!(&error, BundleError::MissingBootSource { boot_source, word }
            if boot_source == "mmc1" && word == "os.src.mmc1=/soc/mmc@fe2e0000"),
        "{error:?}"
    );
    // A prefix other than the one on the line is refused as well.
    let other_prefix = BootRule {
        boot_args: "lemnos".into(),
        ..BootRule::default()
    };
    assert!(matches!(
        inspect_bundle(&bundle[..], &keys_of(&secret), &other_prefix),
        Err(BundleError::MissingBootSource { .. })
    ));
}
