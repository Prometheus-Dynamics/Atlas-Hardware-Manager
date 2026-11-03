#!/usr/bin/env bash
# Builds sparse-fixture.img.xz, the image the sparse write tests use: an MBR
# disk of 16 MiB with a FAT16 boot partition (1 MiB, 6 MiB) and an ext4 data
# partition with metadata checksums (8 MiB, 8 MiB), each holding a file of
# zeros (allocated: it must be written) and one of data. Needs sfdisk,
# mkfs.fat, mtools, mke2fs and xz; run it from anywhere.
set -euo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
head -c 65536 /dev/zero > "$work/zeros.bin"
# Fixed bytes, not random, so the fixture rebuilds the same.
seq 1 9000 | tr '\n' ' ' > "$work/data.bin"
truncate -s 40960 "$work/data.bin"
mkdir "$work/root" && cp "$work/zeros.bin" "$work/data.bin" "$work/root/"
truncate -s 16M "$work/disk.img"
printf 'label: dos\nlabel-id: 0x5056525a\nstart=2048, size=12288, type=e, bootable\nstart=16384, size=16384, type=83\n' |
  sfdisk -q "$work/disk.img"
truncate -s 6M "$work/fat.img"
mkfs.fat -F 16 -s 2 -i 4709fdef -n BOOT "$work/fat.img" >/dev/null
MTOOLS_SKIP_CHECK=1 mcopy -i "$work/fat.img" "$work/zeros.bin" "$work/data.bin" ::/
E2FSPROGS_FAKE_TIME=1700000000 mke2fs -q -t ext4 -b 4096 -L data \
  -U d54724c4-3db9-4a11-8ea0-25de67f96148 -E hash_seed=d54724c4-3db9-4a11-8ea0-25de67f96148 \
  -O metadata_csum,64bit,flex_bg -d "$work/root" "$work/ext4.img" 8M
dd if="$work/fat.img" of="$work/disk.img" bs=512 seek=2048 conv=notrunc status=none
dd if="$work/ext4.img" of="$work/disk.img" bs=512 seek=16384 conv=notrunc status=none
xz -9 -c "$work/disk.img" > "$here/sparse-fixture.img.xz"
