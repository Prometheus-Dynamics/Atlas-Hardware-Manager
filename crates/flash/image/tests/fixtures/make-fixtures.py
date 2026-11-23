#!/usr/bin/env python3
"""Makes data.ext4, the fixture the reader tests read: the real form's data
partition in Lemnos's layout (slots/<id>.erofs, slots/<id>.json, slots/<id>.boot,
slots/bootfiles/<sha256>), for two slots: alpha (the test subject) and beta (a
decoy the reader must not mix in). alpha was installed from a signed bundle, so
it also keeps slots/alpha.sig, the bundle's slot.sig record (Lemnos docs/slots.md
"The kept signature"); beta is unsigned and keeps none.

Run from this directory: python3 make-fixtures.py. It needs e2fsprogs'
mke2fs and no root: `mke2fs -d` copies a directory in without mounting it.

The contents are deterministic (pattern()), so the Rust tests in
tests/common/mod.rs regenerate the same bytes. The verity root is computed here
with hashlib; the reader does not check it, and the tests compare it with the
bundle crate's hasher.
"""

import hashlib
import json
import os
import subprocess
import tempfile

BLOCK = 4096
SALT = bytes([0x5A]) * 32  # 32 bytes: the manifest carries 64 hex digits
# The throwaway Ed25519 seed alpha's slot.sig is made with. A test key, in the open on purpose: the bundle
# tests check the kept record verifies under its public key. Never a board's or a build's key.
SIGNING_SEED = bytes([0x11]) * 32
HERE = os.path.dirname(os.path.abspath(__file__))


def pattern(n, seed):
    """The bytes the tests expect: n bytes of a fixed sequence for `seed`."""
    return bytes((i * 7 + seed * 13) % 251 for i in range(n))


def verity_root(data, salt):
    """dm-verity format 1, SHA-256, 4 KiB blocks: a block's digest is
    sha256(salt || block); a hash block holds 128 digests, zero padded; the root
    is the digest of the single top hash block. One data block has no tree: its
    root is the salted digest of the block itself (veritysetup: "Hash blocks: 0")."""
    if len(data) == BLOCK:
        return hashlib.sha256(salt + data).hexdigest()
    level = [hashlib.sha256(salt + data[i:i + BLOCK]).digest()
             for i in range(0, len(data), BLOCK)]
    while True:
        blocks = [b"".join(level[i:i + 128]).ljust(BLOCK, b"\0")
                  for i in range(0, len(level), 128)]
        if len(blocks) == 1:
            return hashlib.sha256(salt + blocks[0]).hexdigest()
        level = [hashlib.sha256(salt + block).digest() for block in blocks]


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def write(path, data):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as handle:
        handle.write(data)


def slot_sig(manifest_bytes):
    """Lemnos's slot.sig over the manifest's exact bytes (lemnos_slot_store::signing): the 108-byte record
    LMNSSIG1 | algorithm 1 | 3 zero bytes | key id (SHA-256 of the public key) | Ed25519 signature over
    "lemnos-slot-bundle\\0" | u32 LE 1 | u32 LE length | the bytes. Signed with openssl (Ed25519 needs no
    module beyond the standard library this way)."""
    message = (b"lemnos-slot-bundle\0" + (1).to_bytes(4, "little")
               + len(manifest_bytes).to_bytes(4, "little") + manifest_bytes)
    der = bytes.fromhex("302e020100300506032b657004220420") + SIGNING_SEED
    with tempfile.TemporaryDirectory() as tmp:
        key, msg = os.path.join(tmp, "key.der"), os.path.join(tmp, "msg")
        write(key, der)
        write(msg, message)
        signature = subprocess.run(["openssl", "pkeyutl", "-sign", "-rawin", "-keyform", "DER", "-inkey", key,
                                    "-in", msg], check=True, capture_output=True).stdout
        public = subprocess.run(["openssl", "pkey", "-inform", "DER", "-in", key, "-pubout", "-outform", "DER"],
                                check=True, capture_output=True).stdout[-32:]
    assert len(signature) == 64 and len(public) == 32
    record = b"LMNSSIG1" + bytes([1, 0, 0, 0]) + hashlib.sha256(public).digest() + signature
    assert len(record) == 108
    return record


def tree_bytes(data_blocks):
    """The tree's size: none for one data block, else every level down to one block."""
    if data_blocks <= 1:
        return 0
    blocks, level = 0, data_blocks
    while True:
        level = -(-level // 128)
        blocks += level
        if level <= 1:
            return blocks * BLOCK


def make_slot(stage, slot_id, erofs, data_blocks, boot_files, signed=False):
    """erofs is the data (data_blocks blocks) then its tree. boot_files:
    {path: bytes}. Writes the slot's four data-side pieces."""
    slots = os.path.join(stage, "slots")
    data = erofs[:data_blocks * BLOCK]
    assert len(erofs) == data_blocks * BLOCK + tree_bytes(data_blocks), "the tree's size"
    root = verity_root(data, SALT)
    manifest = {
        "schema": 3,
        "id": slot_id,
        "product": "example",
        "version": "1",
        "board": "example-board",
        "image": {"size": len(erofs), "sha256": sha256(erofs)},
        "verity": {"root_hash": root, "salt": SALT.hex(),
                   "data_blocks": data_blocks, "hash_offset": data_blocks * BLOCK},
        "boot": {"files": {path: sha256(data) for path, data in boot_files.items()}},
    }
    manifest_bytes = (json.dumps(manifest, indent=2, sort_keys=True) + "\n").encode()
    everything = dict(boot_files)
    everything["slot.json"] = manifest_bytes
    lines = "".join(f"{sha256(everything[path])}  {path}\n" for path in sorted(everything))
    write(os.path.join(slots, f"{slot_id}.erofs"), erofs)
    write(os.path.join(slots, f"{slot_id}.json"), manifest_bytes)
    write(os.path.join(slots, f"{slot_id}.boot"), lines.encode())
    if signed:
        write(os.path.join(slots, f"{slot_id}.sig"), slot_sig(manifest_bytes))
    for data in everything.values():
        write(os.path.join(slots, "bootfiles", sha256(data)), data)
    return root


def main():
    alpha_erofs = pattern(4 * BLOCK, 1) + pattern(BLOCK, 2)
    alpha_boot = {
        "Image": pattern(40_000, 3),
        "board.dtb": pattern(1234, 4),
        "initramfs.cpio.gz": pattern(9000, 5),
        "cmdline.txt": b"console=tty1 root=PARTUUID=abc panic=5\n",
        "overlays/one.dtbo": pattern(777, 6),
    }
    beta_erofs = pattern(BLOCK, 7)  # one data block, no tree
    beta_boot = {"cmdline.txt": b"console=tty2 panic=5\n"}

    with tempfile.TemporaryDirectory() as tmp:
        stage = os.path.join(tmp, "data")
        alpha_root = make_slot(stage, "alpha", alpha_erofs, 4, alpha_boot, signed=True)
        make_slot(stage, "beta", beta_erofs, 1, beta_boot)
        out = os.path.join(HERE, "data.ext4")
        if os.path.exists(out):
            os.remove(out)
        subprocess.run(["mke2fs", "-q", "-t", "ext4", "-b", "1024", "-N", "64",
                        "-O", "^has_journal", "-L", "lemnos-data", "-d", stage, out, "512K"],
                       check=True)
    print("alpha root", alpha_root)


if __name__ == "__main__":
    main()
