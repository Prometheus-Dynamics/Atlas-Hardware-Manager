#!/usr/bin/env python3
"""Makes data.ext4, the fixture the reader tests read: the real form's data
partition in Lemnos's layout (slots/<id>.erofs, slots/<id>.json, slots/<id>.boot,
slots/bootfiles/<sha256>), for two slots: alpha (the test subject) and beta (a
decoy the reader must not mix in).

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


def make_slot(stage, slot_id, erofs, data_blocks, boot_files):
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
        alpha_root = make_slot(stage, "alpha", alpha_erofs, 4, alpha_boot)
        make_slot(stage, "beta", beta_erofs, 1, beta_boot)
        out = os.path.join(HERE, "data.ext4")
        if os.path.exists(out):
            os.remove(out)
        subprocess.run(["mke2fs", "-q", "-t", "ext4", "-b", "1024", "-N", "64",
                        "-O", "^has_journal", "-L", "data", "-d", stage, out, "512K"],
                       check=True)
    print("alpha root", alpha_root)


if __name__ == "__main__":
    main()
