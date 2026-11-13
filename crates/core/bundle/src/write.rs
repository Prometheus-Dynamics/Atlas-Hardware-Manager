//! Making a slot bundle from a [`SlotSource`]: two passes over the source,
//! then the tar.
//!
//! Pass one hashes the rootfs (its SHA-256 and size, and the verity root hash
//! of its data against the table) and every boot file. That is what
//! `slot.json` says, and `slot.sig` signs those exact bytes. Pass two writes
//! the tar, hashing again as it goes: if a source gives different bytes the
//! second time, the write fails rather than leave a bundle whose manifest is
//! wrong.
//!
//! Tar order: `slot.json`, `slot.sig`, `rootfs.erofs`, then `boot/...` in
//! path order. Nothing else. Memory is one 64 KiB buffer and the verity
//! hasher's levels, whatever the image's size.

use std::collections::BTreeMap;
use std::io::{Read, Write};

use sha2::{Digest, Sha256};

use crate::BundleError;
use crate::manifest::{
    CMDLINE, ImageEntry, SlotManifest, SlotMeta, validate_boot_path, validate_id,
};
use crate::sign::sign_slot;
use crate::source::SlotSource;
use crate::verity::{VerityHasher, VerityTable};

/// The read buffer for every stream.
pub(crate) const CHUNK: usize = 64 * 1024;
/// The member names.
pub const SLOT_JSON: &str = "slot.json";
pub const SLOT_SIG: &str = "slot.sig";
pub const ROOTFS: &str = "rootfs.erofs";
pub const BOOT_DIR: &str = "boot/";

/// Writes the bundle of `source` as `id` to `out` and returns the manifest
/// it carries. `signing_key` is the ed25519 secret that signs `slot.json`.
///
/// Fails if the id or a boot path is not valid, if `cmdline.txt` is missing,
/// if the table is not one this writer reads, if the rootfs's verity tree
/// does not give the table's root hash, or if a source changes between reads.
/// On failure `out` holds a partial tar that must be discarded.
pub fn write_bundle<W: Write>(
    source: &dyn SlotSource,
    id: &str,
    meta: &SlotMeta,
    signing_key: &[u8; 32],
    out: W,
) -> Result<SlotManifest, BundleError> {
    validate_id(id)?;
    let table = VerityTable::parse(&source.verity_table()?)?;

    // Pass one: what slot.json says.
    let rootfs = scan_rootfs(source, &table)?;
    let boot = scan_boot(source)?;
    if !boot.contains_key(CMDLINE) {
        return Err(BundleError::MissingCmdline);
    }
    let manifest = SlotManifest::new(
        id,
        meta,
        ImageEntry {
            size: rootfs.size,
            sha256: rootfs.sha256.clone(),
        },
        rootfs.root_hash,
        boot.iter()
            .map(|(path, file)| (path.clone(), file.sha256.clone()))
            .collect(),
    );
    manifest.validate()?;
    let json = manifest.to_json()?;
    let signature = sign_slot(&json, signing_key);

    // Pass two: the tar.
    let mut tar = tar::Builder::new(out);
    append_bytes(&mut tar, SLOT_JSON, &json)?;
    append_bytes(&mut tar, SLOT_SIG, &signature)?;
    let mut reader = source.rootfs()?;
    append_stream(&mut tar, ROOTFS, rootfs.size, &rootfs.sha256, &mut *reader)?;
    for (path, file) in &boot {
        let mut reader = source.boot_file(path)?;
        let name = format!("{BOOT_DIR}{path}");
        append_stream(&mut tar, &name, file.size, &file.sha256, &mut *reader)?;
    }
    let mut out = tar
        .into_inner()
        .map_err(|error| BundleError::Io(format!("writing the bundle: {error}")))?;
    out.flush()
        .map_err(|error| BundleError::Io(format!("writing the bundle: {error}")))?;
    Ok(manifest)
}

/// What pass one learned about the rootfs.
struct Rootfs {
    size: u64,
    sha256: String,
    root_hash: String,
}

/// What pass one learned about one boot file.
struct Hashed {
    size: u64,
    sha256: String,
}

/// Reads the rootfs once: the data through the verity hasher, the tree after
/// it through SHA-256 only, and checks the root hash against the table.
fn scan_rootfs(source: &dyn SlotSource, table: &VerityTable) -> Result<Rootfs, BundleError> {
    let data_len = table
        .data_len()
        .ok_or_else(|| BundleError::Table("the data is too long".into()))?;
    let mut reader = source.rootfs()?;
    let mut verity = VerityHasher::new(&table.salt);
    let mut sha = Sha256::new();
    let mut size = 0u64;
    let mut buf = vec![0u8; CHUNK];

    while size < data_len {
        let want = usize::try_from((data_len - size).min(CHUNK as u64)).unwrap_or(CHUNK);
        let read = read_chunk(&mut *reader, &mut buf[..want], ROOTFS)?;
        if read == 0 {
            return Err(BundleError::Malformed(format!(
                "{ROOTFS} is {size} bytes, shorter than the table's {data_len} bytes of data"
            )));
        }
        verity.update(&buf[..read]);
        sha.update(&buf[..read]);
        size += read as u64;
    }
    loop {
        let read = read_chunk(&mut *reader, &mut buf, ROOTFS)?;
        if read == 0 {
            break;
        }
        sha.update(&buf[..read]);
        size += read as u64;
    }
    if size == data_len {
        return Err(BundleError::Malformed(format!(
            "{ROOTFS} has no verity tree after its data"
        )));
    }
    let root_hash = verity.finish()?;
    if root_hash != table.root_hash {
        return Err(BundleError::RootHashMismatch {
            expected: table.root_hash.clone(),
            actual: root_hash,
        });
    }
    Ok(Rootfs {
        size,
        sha256: to_hex(&sha.finalize()),
        root_hash,
    })
}

/// Hashes every boot file of the source. Invalid or duplicate paths are
/// refused before the file is read.
fn scan_boot(source: &dyn SlotSource) -> Result<BTreeMap<String, Hashed>, BundleError> {
    let mut boot = BTreeMap::new();
    let mut buf = vec![0u8; CHUNK];
    for file in source.boot_files()? {
        validate_boot_path(&file.path)?;
        if boot.contains_key(&file.path) {
            return Err(BundleError::DuplicateBootFile(file.path));
        }
        let mut reader = source.boot_file(&file.path)?;
        let mut sha = Sha256::new();
        let mut size = 0u64;
        loop {
            let read = read_chunk(&mut *reader, &mut buf, &file.path)?;
            if read == 0 {
                break;
            }
            sha.update(&buf[..read]);
            size += read as u64;
        }
        let sha256 = to_hex(&sha.finalize());
        boot.insert(file.path, Hashed { size, sha256 });
    }
    Ok(boot)
}

/// Appends a small in-memory member.
fn append_bytes<W: Write>(
    tar: &mut tar::Builder<W>,
    name: &str,
    bytes: &[u8],
) -> Result<(), BundleError> {
    let mut header = header(bytes.len() as u64);
    tar.append_data(&mut header, name, bytes)
        .map_err(|error| BundleError::Io(format!("writing {name}: {error}")))
}

/// Appends a streamed member of `size` bytes. Fails if the stream's length or
/// digest is not what pass one measured.
fn append_stream<W: Write>(
    tar: &mut tar::Builder<W>,
    name: &str,
    size: u64,
    sha256: &str,
    reader: &mut dyn Read,
) -> Result<(), BundleError> {
    let mut hashing = Hashing {
        inner: reader.take(size),
        sha: Sha256::new(),
        count: 0,
    };
    let mut header = header(size);
    tar.append_data(&mut header, name, &mut hashing)
        .map_err(|error| BundleError::Io(format!("writing {name}: {error}")))?;
    let actual = to_hex(&hashing.sha.finalize());
    if hashing.count != size || actual != sha256 {
        return Err(BundleError::Changed {
            member: name.to_string(),
            expected: format!("{size} bytes, sha256 {sha256}"),
            actual: format!("{} bytes, sha256 {actual}", hashing.count),
        });
    }
    Ok(())
}

/// A regular file's header with the reproducible fields: mode 0644, owner 0,
/// mtime 0.
fn header(size: u64) -> tar::Header {
    let mut header = tar::Header::new_gnu();
    header.set_size(size);
    header.set_mode(0o644);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    header.set_entry_type(tar::EntryType::Regular);
    header
}

/// Counts and hashes what a member's stream gives, up to the member's size.
struct Hashing<'a> {
    inner: std::io::Take<&'a mut dyn Read>,
    sha: Sha256,
    count: u64,
}

impl Read for Hashing<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buf)?;
        self.sha.update(&buf[..read]);
        self.count += read as u64;
        Ok(read)
    }
}

/// Reads up to `buf.len()` bytes, retrying interruptions.
pub(crate) fn read_chunk(
    reader: &mut dyn Read,
    buf: &mut [u8],
    what: &str,
) -> Result<usize, BundleError> {
    loop {
        match reader.read(buf) {
            Ok(read) => return Ok(read),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(BundleError::io(what, error)),
        }
    }
}

pub(crate) fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
