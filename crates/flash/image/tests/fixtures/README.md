# Fixtures for atlas-image

Two ext4 images, 256 KiB each, made on a workstation with e2fsprogs
(`mke2fs 1.47.3`) and no root:

- `data.ext4` (label `data`): `slots/alpha` (the slot file: four 4 KiB data
  blocks, then one tree block, 20480 bytes) and `slots/alpha.verity` (Gaia's
  verity table text, root hash computed by make-fixtures.py).
- `rootfs.ext4` (label `rootfs`): `boot/Image`, `boot/board.dtb`,
  `boot/extlinux/extlinux.conf` (the bring-up form's /boot).

They are made by `python3 make-fixtures.py` in this directory. The script writes
a staging tree, then runs `mke2fs -q -t ext4 -b 1024 -N 32 -O ^has_journal -L
<label> -d <stage> <image> 256K`. `-d` copies the tree in without mounting, so no
root is needed. The contents come from `pattern()` in the script, which the Rust
tests in `tests/common/mod.rs` repeat exactly.

The verity root is computed in the script with hashlib, not by the bundle crate.
The bundle tests check the two agree (the bundle writer refuses a mismatch).

The ext4 files carry the workstation's uid and the time they were made; the
images are not reproducible byte for byte, but their contents are.
