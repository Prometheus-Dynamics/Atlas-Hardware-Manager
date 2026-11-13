# Fixture for atlas-image

`data.ext4`: one ext4 image, 512 KiB, label `data`, made on a workstation with
e2fsprogs (`mke2fs 1.47.3`) and no root. It is the real form's data partition in
Lemnos's layout, for two slots:

- `slots/alpha.erofs`: 20480 bytes (four 4 KiB data blocks, then one tree block).
- `slots/alpha.json`: the manifest (`schema` 3, `image.size`, `image.sha256`,
  `verity.root_hash`, `verity.salt`, `verity.data_blocks`, `verity.hash_offset`,
  `boot.files`).
- `slots/alpha.boot`: the boot list, `sha256sum -c` form, including `slot.json`.
- `slots/beta.*`: a second slot of one data block and no tree (4096 bytes), so a read of
  one slot is checked not to mix in the other.
- `slots/bootfiles/<sha256>`: the boot files by content: `Image`, `board.dtb`,
  `initramfs.cpio.gz`, `cmdline.txt`, `overlays/one.dtbo`, and the manifest bytes
  (`slot.json`).

Made by `python3 make-fixtures.py` in this directory. The script writes a staging
tree, then runs `mke2fs -q -t ext4 -b 1024 -N 64 -O ^has_journal -L data -d
<stage> data.ext4 512K`. `-d` copies the tree in without mounting, so no root is
needed. The contents come from `pattern()` in the script; the Rust tests in
`tests/common/mod.rs` repeat it exactly.

The verity roots are computed in the script with hashlib, not by the bundle crate.
The tests check the manifests' roots against the bundle crate's hasher.

The ext4 files carry the workstation's uid and the time they were made. The images
are not reproducible byte for byte; their contents are.
