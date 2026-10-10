#!/bin/sh
# Off-device test of data-setup, on a real MBR layout made with sfdisk (p1-p3
# primary, p4 extended with logical p5-p7, free space after p7): the data
# partition grows to the end of the disk, a partition with no signature gets
# ext4, an existing filesystem is kept, a small ext4 is grown. Files stand in
# for the disk and the partition. Needs sfdisk, mkfs.ext4, e2fsck,
# resize2fs, dumpe2fs, blkid. Run: sh devices/raze/tests/data-setup.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
lib=$here/../gaia/assets/rootfs/usr/lib/board
T=$(mktemp -d "${TMPDIR:-/tmp}/board-data-setup-test.XXXXXX")
trap 'rm -rf "$T"' EXIT
PATH=$PATH:/usr/sbin:/sbin

fail() {
	echo "FAIL: $*" >&2
	exit 1
}
for tool in sfdisk mkfs.ext4 e2fsck resize2fs dumpe2fs blkid; do
	command -v "$tool" >/dev/null 2>&1 || {
		echo "  (no $tool here: skipped)"
		exit 0
	}
done

mkdir -p "$T/run" "$T/data" "$T/etc"
export BOARD_LIB_DIR=$lib BOARD_RUN_DIR=$T/run BOARD_DATA_DIR=$T/run/nodata BOARD_ETC_DIR=$T/etc \
	BOARD_DATA_DISK=$T/disk BOARD_DATA_DEV=$T/p7
disk() {
	truncate -s 128M "$T/disk"
	# 1 MiB aligned: p1 8M, p2 16M, p3 16M, p4 extended to 96M, p5 16M,
	# p6 16M, p7 8M; the last 32 MiB of the disk are free.
	sfdisk -q "$T/disk" >/dev/null <<'LAYOUT'
label: dos
unit: sectors
start=2048, size=16384, type=c
start=18432, size=32768, type=c
start=51200, size=32768, type=c
start=83968, size=112640, type=5
start=86016, size=32768, type=83
start=120832, size=32768, type=83
start=155648, size=16384, type=83
LAYOUT
}
p7_size() { sfdisk -d "$T/disk" | sed -n 's/.*[^0-9]7 *: *start=.*size= *\([0-9]*\),.*/\1/p'; }
p4_size() { sfdisk -d "$T/disk" | sed -n 's/.*[^0-9]4 *: *start=.*size= *\([0-9]*\),.*/\1/p'; }
run() { sh "$lib/data-setup" 2> "$T/log" || fail "data-setup failed: $(cat "$T/log")"; }
events() { cat "$T/run/events.jsonl" "$T/run/nodata/events.jsonl" 2>/dev/null; }

echo "a fresh flash: p7 grows to the end of the disk and gets ext4"
disk
head -c 8M /dev/urandom > "$T/p7"
before=$(p7_size)
run
[ "$(p7_size)" -gt "$before" ] || fail "p7 should grow: $before -> $(p7_size)"
[ "$(p4_size)" -gt 112640 ] || fail "the extended partition should grow: $(p4_size)"
end=$(sfdisk -d "$T/disk" | sed -n 's/.*[^0-9]7 *: *start= *\([0-9]*\), *size= *\([0-9]*\),.*/\1 \2/p')
set -- $end
[ $((($1 + $2) * 512)) -gt $((126 * 1024 * 1024)) ] || fail "p7 should end near the disk's end: $end"
[ "$(blkid -p -o value -s TYPE "$T/p7")" = ext4 ] || fail "p7 should be ext4"
[ "$(blkid -p -o value -s LABEL "$T/p7")" = data ] || fail "the label"
events | grep -q '"kind":"data.grown"' || fail "a data.grown event: $(events)"
events | grep -q '"kind":"data.created"' || fail "a data.created event: $(events)"

echo "the next boot changes nothing"
cp "$T/p7" "$T/p7.ref"
grown=$(p7_size)
run
cmp -s "$T/p7" "$T/p7.ref" || fail "an existing filesystem must be left alone"
[ "$(p7_size)" = "$grown" ] || fail "the table shouldn't change again"
[ "$(events | grep -c '"kind":"data.created"')" = 1 ] || fail "created only once"

echo "a filesystem it doesn't make is kept, whatever it is"
disk
truncate -s 8M "$T/p7"
mkfs.ext4 -q -F -L old "$T/p7"
printf 'kept\n' > "$T/marker"
run
[ "$(blkid -p -o value -s LABEL "$T/p7")" = old ] || fail "an old /data must be kept"

echo "a small ext4 grows to fill its grown partition"
rm -f "$T/run/events.jsonl" "$T/run/nodata/events.jsonl"
disk
truncate -s 24M "$T/p7"
mkfs.ext4 -q -F "$T/p7" 8M
blocks() { dumpe2fs -h "$T/p7" 2>/dev/null | sed -n 's/^Block count: *//p'; }
small=$(blocks)
run
[ "$(blocks)" -gt "$small" ] || fail "the filesystem should grow: $small -> $(blocks)"
events | grep -q '"kind":"data.resized"' || fail "a data.resized event: $(events)"

echo "BOARD_DATA_GROW=0 leaves the table alone"
disk
head -c 8M /dev/urandom > "$T/p7"
printf 'BOARD_DATA_GROW=0\n' > "$T/etc/data.env"
run
[ "$(p7_size)" = 16384 ] || fail "the table changed: $(p7_size)"
[ "$(blkid -p -o value -s TYPE "$T/p7")" = ext4 ] || fail "it still gets a filesystem"

echo "ok"
