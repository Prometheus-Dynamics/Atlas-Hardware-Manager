//! Making a slot bundle from a [`SlotSource`]: two passes over the source,
//! then the tar.
//!
//! TEST AND SIM ONLY. This writer exists for the tests and the simulated node. It
//! builds its manifest with Lemnos's `Manifest::to_json` and its record and message
//! with Lemnos's `signing` functions, so its bytes are Lemnos's; but real bundles
//! are made by Gaia's `lemnos-ctl bundle`, which owns the board rules, the command
//! line and the tree. Nothing installs a bundle this writer made on a real board.
//!
//! Pass one hashes the rootfs (its SHA-256 and size; the verity root hash of its
//! data, salted, against the source's `verity.root_hash`; the file's length
//! against the layout) and every boot file. That is what `slot.json` says, and
//! `slot.sig` signs those exact bytes. Pass two writes the tar, hashing again as
//! it goes: if a source gives different bytes the second time, the write fails
//! rather than leave a bundle whose manifest is wrong.
//!
//! The appended tree is measured (its length, and its bytes in the SHA-256 of
//! the file) but its bytes are not recomputed: the root hash covers the data, not
//! the tree, and a tree that does not match its root fails the board's verity
//! check at boot, not the install. See the crate docs.
//!
//! Tar order: `slot.json`, `slot.sig`, `rootfs.erofs`, the `boot/` directory, then
//! `boot/...` in path order. Nothing else. The framing is Lemnos's ([`TarOut`]). Memory is one 64 KiB buffer and the verity
//! hasher's levels, whatever the image's size.

use std::collections::BTreeMap;
use std::io::{Read, Write};

use lemnos_slot_store::tar::write as lemnos_tar;
use sha2::{Digest, Sha256};

use atlas_image::{Verity, VerityError, tree_bytes};

use crate::BundleError;
use crate::manifest::{
    CMDLINE, ImageEntry, SlotManifest, SlotMeta, validate_boot_path, validate_id,
};
use crate::sign::sign_slot;
use crate::source::SlotSource;
use crate::verity::VerityHasher;

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
/// Fails if the id or a boot path is not valid, if `cmdline.txt` is missing, if
/// the verity values do not describe the layout, if the file is not its data and
/// whole tree, if the data does not give the root hash, or if a source changes
/// between reads. On failure `out` holds a partial tar that must be discarded.
pub fn write_bundle<W: Write>(
    source: &dyn SlotSource,
    id: &str,
    meta: &SlotMeta,
    signing_key: &[u8; 32],
    out: W,
) -> Result<SlotManifest, BundleError> {
    validate_id(id)?;
    let verity = source.verity()?;
    verity.check()?;

    // Pass one: what slot.json says.
    let rootfs = scan_rootfs(source, &verity)?;
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
        &verity,
        boot.iter()
            .map(|(path, file)| (path.clone(), file.sha256.clone()))
            .collect(),
    );
    manifest.validate()?;
    manifest.validate_written()?;
    let json = manifest.to_json()?;
    let signature = sign_slot(&json, signing_key);

    // Pass two: the tar.
    let mut tar = TarOut::new(out);
    tar.bytes(SLOT_JSON, &json)?;
    tar.bytes(SLOT_SIG, &signature)?;
    let mut reader = source.rootfs()?;
    tar.stream(ROOTFS, rootfs.size, &rootfs.sha256, &mut *reader)?;
    tar.dir(BOOT_DIR)?;
    for (path, file) in &boot {
        let mut reader = source.boot_file(path)?;
        let name = format!("{BOOT_DIR}{path}");
        tar.stream(&name, file.size, &file.sha256, &mut *reader)?;
    }
    tar.finish()?;
    Ok(manifest)
}

/// What pass one learned about the rootfs.
struct Rootfs {
    size: u64,
    sha256: String,
}

/// What pass one learned about one boot file.
struct Hashed {
    size: u64,
    sha256: String,
}

/// Reads the rootfs once: the data through the verity hasher, the tree after it
/// through SHA-256 only. Checks the file's length against the layout, then the
/// data's root hash against the source's `verity.root_hash`.
fn scan_rootfs(source: &dyn SlotSource, verity: &Verity) -> Result<Rootfs, BundleError> {
    let data_len = verity.data_len();
    let mut reader = source.rootfs()?;
    let mut hasher = VerityHasher::new(&verity.salt);
    let mut sha = Sha256::new();
    let mut size = 0u64;
    let mut buf = vec![0u8; CHUNK];

    while size < data_len {
        let want = usize::try_from((data_len - size).min(CHUNK as u64)).unwrap_or(CHUNK);
        let read = read_chunk(&mut *reader, &mut buf[..want], ROOTFS)?;
        if read == 0 {
            // The file ends inside the data: the block count or the file is wrong.
            return Err(VerityError::Size {
                data_blocks: verity.data_blocks,
                size,
                expected: data_len.saturating_add(tree_bytes(verity.data_blocks)),
            }
            .into());
        }
        hasher.update(&buf[..read]);
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
    verity.check_size(size)?;
    let root_hash = hasher.finish()?;
    if root_hash != verity.root_hash {
        return Err(BundleError::RootHashMismatch {
            expected: verity.root_hash.clone(),
            actual: root_hash,
        });
    }
    Ok(Rootfs {
        size,
        sha256: to_hex(&sha.finalize()),
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

/// A tar framed as Lemnos frames one: each header is `lemnos_slot_store::tar::write`'s, the code
/// `lemnos-ctl bundle` writes with at the image's Lemnos pin (ustar, owner 0, time 0, mode 0644 for
/// a file and 0755 for a directory, a GNU long-name member over 99 bytes), with its padding and its
/// two zero blocks at the end. The same members in the same order are the same bytes as a bundle
/// `lemnos-ctl` made, so a slot read back out of an image can be held to the build's file whole.
pub(crate) struct TarOut<W: Write> {
    out: W,
}

impl<W: Write> TarOut<W> {
    pub(crate) fn new(out: W) -> Self {
        Self { out }
    }

    fn put(&mut self, name: &str, bytes: &[u8]) -> Result<(), BundleError> {
        self.out
            .write_all(bytes)
            .map_err(|error| BundleError::Io(format!("writing {name}: {error}")))
    }

    fn pad(&mut self, name: &str, size: u64) -> Result<(), BundleError> {
        self.put(name, &vec![0u8; lemnos_tar::padding(size)])
    }

    /// A small in-memory member.
    pub(crate) fn bytes(&mut self, name: &str, bytes: &[u8]) -> Result<(), BundleError> {
        self.put(name, &lemnos_tar::header(name, b'0', bytes.len() as u64))?;
        self.put(name, bytes)?;
        self.pad(name, bytes.len() as u64)
    }

    /// A directory entry (`boot/`), where Lemnos writes one.
    pub(crate) fn dir(&mut self, name: &str) -> Result<(), BundleError> {
        self.put(name, &lemnos_tar::header(name, b'5', 0))
    }

    /// A streamed member of `size` bytes. Fails if the stream's length or digest is not what was
    /// measured before; the tar is then broken and must be discarded.
    pub(crate) fn stream(
        &mut self,
        name: &str,
        size: u64,
        sha256: &str,
        reader: &mut dyn Read,
    ) -> Result<(), BundleError> {
        self.put(name, &lemnos_tar::header(name, b'0', size))?;
        let mut hashing = Hashing {
            inner: reader.take(size),
            sha: Sha256::new(),
            count: 0,
        };
        std::io::copy(&mut hashing, &mut self.out)
            .map_err(|error| BundleError::Io(format!("writing {name}: {error}")))?;
        let actual = to_hex(&hashing.sha.finalize());
        if hashing.count != size || actual != sha256 {
            return Err(BundleError::Changed {
                member: name.to_string(),
                expected: format!("{size} bytes, sha256 {sha256}"),
                actual: format!("{} bytes, sha256 {actual}", hashing.count),
            });
        }
        self.pad(name, size)
    }

    /// The end of the archive, flushed; gives the writer back.
    pub(crate) fn finish(mut self) -> Result<W, BundleError> {
        self.put("the end of the slot's file", &lemnos_tar::END)?;
        self.out
            .flush()
            .map_err(|error| BundleError::Io(format!("writing the slot's file: {error}")))?;
        Ok(self.out)
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The writer's bytes are Lemnos's: the same members through `TarOut` and through Lemnos's own
    /// `archive` are one file, a long boot path (a GNU long-name member) and a member that is not a
    /// whole block included.
    #[test]
    fn the_same_members_are_the_same_bytes_as_lemnos_writes() {
        let long = format!("boot/overlays/{}.dtbo", "x".repeat(120));
        let rootfs = vec![7u8; 5000];
        let files: [(&str, &[u8]); 5] = [
            (SLOT_JSON, b"{\"id\":\"slot-a\"}"),
            (ROOTFS, &rootfs),
            (BOOT_DIR, b""),
            ("boot/cmdline.txt", b"root=/dev/dm-0\n"),
            (&long, b"overlay"),
        ];
        let mut tar = TarOut::new(Vec::new());
        tar.bytes(files[0].0, files[0].1).expect("slot.json");
        tar.stream(
            ROOTFS,
            5000,
            &to_hex(&Sha256::digest(&rootfs)),
            &mut &rootfs[..],
        )
        .expect("rootfs");
        tar.dir(BOOT_DIR).expect("boot/");
        for (name, bytes) in &files[3..] {
            tar.stream(
                name,
                bytes.len() as u64,
                &to_hex(&Sha256::digest(bytes)),
                &mut &bytes[..],
            )
            .expect("a boot file");
        }
        assert_eq!(tar.finish().expect("the end"), lemnos_tar::archive(&files));
    }

    /// A stream shorter than its header said is refused, by name.
    #[test]
    fn a_short_stream_is_refused() {
        let mut tar = TarOut::new(Vec::new());
        let error = tar
            .stream(ROOTFS, 10, "00", &mut &[1u8, 2, 3][..])
            .expect_err("three bytes are not ten");
        assert!(matches!(error, BundleError::Changed { ref member, .. } if member == ROOTFS));
    }
}
