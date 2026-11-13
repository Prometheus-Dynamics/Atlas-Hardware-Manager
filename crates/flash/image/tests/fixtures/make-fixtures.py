#!/usr/bin/env python3
"""Makes the two ext4 fixtures the reader tests read: data.ext4 (the slot file
and its verity table, the real form's data partition and the bring-up form's
data partition) and rootfs.ext4 (the bring-up form's /boot).

Run from this directory: python3 make-fixtures.py. It needs e2fsprogs'
mke2fs and no root: `mke2fs -d` copies a directory in without mounting it.

The contents are deterministic (see pattern()), so the Rust tests in
tests/common regenerate the same bytes. The verity root is computed here with
hashlib, independently of the bundle crate's own implementation; the test
checks that the two agree.
"""

import hashlib
import os
import subprocess
import tempfile

BLOCK = 4096
SALT = bytes([0x5A]) * 16
DATA_BLOCKS = 4
HERE = os.path.dirname(os.path.abspath(__file__))


def pattern(n, seed):
    """The bytes the tests expect: n bytes of a fixed sequence for `seed`."""
    return bytes((i * 7 + seed * 13) % 251 for i in range(n))


def verity_root(data, salt):
    """dm-verity format 1, SHA-256, 4 KiB blocks: a block's digest is
    sha256(salt || block); a hash block holds 128 digests, zero padded; the root
    is the digest of the single top hash block."""
    level = [hashlib.sha256(salt + data[i:i + BLOCK]).digest()
             for i in range(0, len(data), BLOCK)]
    while True:
        blocks = [b"".join(level[i:i + 128]).ljust(BLOCK, b"\0")
                  for i in range(0, len(level), 128)]
        if len(blocks) == 1:
            return hashlib.sha256(salt + blocks[0]).hexdigest()
        level = [hashlib.sha256(salt + block).digest() for block in blocks]


def verity_text(root, salt, blocks):
    return (f"hash_type 1\nalgorithm sha256\nroot_hash {root}\n"
            f"salt {salt.hex()}\ndata_blocks {blocks}\n"
            f"data_block_size {BLOCK}\nhash_block_size {BLOCK}\n"
            f"hash_offset {blocks * BLOCK}\n")


def write(path, data):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as handle:
        handle.write(data)


def mke2fs(stage, out, label, size):
    if os.path.exists(out):
        os.remove(out)
    subprocess.run(["mke2fs", "-q", "-t", "ext4", "-b", "1024", "-N", "32",
                    "-O", "^has_journal", "-L", label, "-d", stage, out, size],
                   check=True)


def main():
    slot_data = pattern(DATA_BLOCKS * BLOCK, 1)
    slot_tree = pattern(BLOCK, 2)
    root = verity_root(slot_data, SALT)
    table = verity_text(root, SALT, DATA_BLOCKS)
    extlinux = (b"default l\nlabel l\n  kernel /boot/Image\n"
                b"  fdt /boot/board.dtb\n  append root=PARTLABEL=rootfs ro\n")

    with tempfile.TemporaryDirectory() as tmp:
        data_stage = os.path.join(tmp, "data")
        write(os.path.join(data_stage, "slots", "alpha"), slot_data + slot_tree)
        write(os.path.join(data_stage, "slots", "alpha.verity"), table.encode())
        mke2fs(data_stage, os.path.join(HERE, "data.ext4"), "data", "256K")

        rootfs_stage = os.path.join(tmp, "rootfs")
        write(os.path.join(rootfs_stage, "boot", "Image"), pattern(40000, 3))
        write(os.path.join(rootfs_stage, "boot", "board.dtb"), pattern(1234, 4))
        write(os.path.join(rootfs_stage, "boot", "extlinux", "extlinux.conf"), extlinux)
        mke2fs(rootfs_stage, os.path.join(HERE, "rootfs.ext4"), "rootfs", "256K")
    print("root", root)


if __name__ == "__main__":
    main()
