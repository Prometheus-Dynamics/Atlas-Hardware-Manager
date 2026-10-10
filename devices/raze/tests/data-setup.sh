#!/bin/sh
# Off-device test of data-setup, on a real MBR layout made with sfdisk (p1-p3
# primary, p4 extended with logical p5-p7, free space after p7): the data
# partition grows to the end of the disk, a partition with no signature gets
# ext4, an existing filesystem is kept, a small ext4 is grown, a flash id
# resets a /data from another flash and keeps its own. Files stand in
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

echo "an EBR chain at the extended start, the image cut after p5, stale EBRs left over"
# Gaia can place every EBR before p5 so the image ends early: the table must
# come from that chain (not from what an old flash left), and growing p7
# rewrites its EBR there.
rm -f "$T/etc/data.env"
python3 "$here/data/ebr-chain.py" "$T/disk" cut
table() { sfdisk -d "$T/disk" | sed -n 's/.*[^0-9]\([0-9]\) *: *start= *\([0-9]*\), *size= *\([0-9]*\),.*/\1 \2 \3/p' | tr '\n' ' '; }
[ "$(table)" = "1 2048 16384 2 18432 32768 3 51200 32768 4 83968 88064 5 86016 32768 6 120832 32768 7 155648 16384 " ] ||
	fail "the chain's table: $(table)"
if command -v partx >/dev/null 2>&1; then
	[ "$(partx -g -o START,SECTORS "$T/disk" | tr -s ' \n' ' ')" = " 2048 16384 18432 32768 51200 32768 83968 88064 86016 32768 120832 32768 155648 16384 " ] ||
		fail "libblkid's view: $(partx -g -o START,SECTORS "$T/disk" | tr -s ' \n' ' ')"
fi
head -c 8M /dev/urandom > "$T/p7"
run
case "$(table)" in
"1 2048 16384 2 18432 32768 3 51200 32768 4 83968 178176 5 86016 32768 6 120832 32768 7 155648 106496 ") ;;
*) fail "after growing p7: $(table)" ;;
esac
[ "$(blkid -p -o value -s TYPE "$T/p7")" = ext4 ] || fail "p7 should be ext4"

echo "a flash id: the filesystem it makes carries it, the next boot keeps it"
disk
rm -f "$T/run/events.jsonl" "$T/run/nodata/events.jsonl"
head -c 8M /dev/urandom > "$T/p7"
printf 'build-1\n' > "$T/flash-id"
export BOARD_DATA_FLASH_ID_FILE=$T/flash-id
run
uuid1=$(blkid -p -o value -s UUID "$T/p7")
want=$(printf build-1 | sha256sum | cut -c1-32 | sed 's/^\(.\{8\}\)\(.\{4\}\)\(.\{4\}\)\(.\{4\}\)/\1-\2-\3-\4-/')
[ "$uuid1" = "$want" ] || fail "the UUID should come from the flash id: $uuid1, not $want"
cp "$T/p7" "$T/p7.ref"
run
cmp -s "$T/p7" "$T/p7.ref" || fail "the same flash must keep its /data"
events | grep -q '"kind":"data.reset"' && fail "no reset on the same flash: $(events)"

echo "a new flash over a used /data makes it new"
printf 'build-2\n' > "$T/flash-id"
run
[ "$(blkid -p -o value -s UUID "$T/p7")" != "$uuid1" ] || fail "a new flash should make a new filesystem"
[ "$(blkid -p -o value -s LABEL "$T/p7")" = data ] || fail "the label"
events | grep -q '"kind":"data.reset"' || fail "a data.reset event: $(events)"

echo "a flash id over an old /data from an image without one, or another filesystem"
truncate -s 8M "$T/p7"
mkfs.ext4 -q -F -L old "$T/p7"
run
[ "$(blkid -p -o value -s LABEL "$T/p7")" = data ] || fail "an old /data should be made new"
head -c 8M /dev/zero > "$T/p7"
mkfs.vfat "$T/p7" >/dev/null 2>&1 || mkswap "$T/p7" >/dev/null 2>&1 || true
if blkid -p "$T/p7" >/dev/null 2>&1; then
	run
	[ "$(blkid -p -o value -s TYPE "$T/p7")" = ext4 ] || fail "another filesystem should be made ext4"
fi

echo "an empty flash id file is no flash id"
: > "$T/flash-id"
truncate -s 8M "$T/p7"
mkfs.ext4 -q -F -L old "$T/p7"
run
[ "$(blkid -p -o value -s LABEL "$T/p7")" = old ] || fail "without an id, /data is kept"
unset BOARD_DATA_FLASH_ID_FILE

echo "ok"
