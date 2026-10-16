#!/bin/sh
# Off-device test of the slot initramfs's /init: slot-init itself
# (crates/slot-init, boot.rs), run as `slot-init boot` with recording ops
# under a prefix. The partition search, the superblock check, the state and
# manifest reading and every decision are the real code; mounts, module
# loads, the verity device, restarts and switch_root are recorded instead
# (record.rs). A fake sysfs and disk stand in for the eMMC.
# Needs cargo (builds slot-init for this host). Run: sh devices/raze/tests/slot-init.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
repo=$here/../../..
T=$(mktemp -d "${TMPDIR:-/tmp}/board-slot-init-test.XXXXXX")
trap 'rm -rf "$T"' EXIT
fail() {
	echo "FAIL: $*" >&2
	exit 1
}

if [ -n "${SLOT_INIT_BIN:-}" ]; then
	B=$SLOT_INIT_BIN
elif command -v cargo >/dev/null 2>&1; then
	(cd "$repo" && cargo build -q -p slot-init) || fail "slot-init didn't build"
	tdir=$(cd "$repo" && cargo metadata --no-deps --format-version 1 | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
	B=$tdir/debug/slot-init
else
	echo "slot-init: skipped (no cargo to build slot-init)"
	exit 0
fi
[ -x "$B" ] || fail "no slot-init at $B"

hash=$(printf 'ab%.0s' $(seq 32))
# write_at <file> <offset> <hex bytes...>
write_at() {
	f=$1 off=$2
	shift 2
	# shellcheck disable=SC2059
	printf "$(for b in "$@"; do printf '\\%03o' "0x$b"; done)" |
		dd of="$f" bs=1 seek="$off" conv=notrunc status=none
}

# The eMMC: an MBR disk with signature 5056525a, p1 (FAT, unread) and p2
# (ext4 labelled <label>, data by default; s_state <bytes>, clean by default).
disk() {
	label=${1:-data} state=${2:-01 00}
	rm -rf "$T/sys" "$T/dev"
	d=$T/sys/devices/mmc/mmcblk0
	mkdir -p "$d/mmcblk0p1" "$d/mmcblk0p2" "$T/sys/class/block" "$T/dev"
	echo 1 > "$d/mmcblk0p1/partition"
	echo 2 > "$d/mmcblk0p2/partition"
	ln -s ../../devices/mmc/mmcblk0 "$T/sys/class/block/mmcblk0"
	ln -s ../../devices/mmc/mmcblk0/mmcblk0p1 "$T/sys/class/block/mmcblk0p1"
	ln -s ../../devices/mmc/mmcblk0/mmcblk0p2 "$T/sys/class/block/mmcblk0p2"
	dd if=/dev/zero of="$T/dev/mmcblk0" bs=512 count=1 status=none
	write_at "$T/dev/mmcblk0" 440 5a 52 56 50
	write_at "$T/dev/mmcblk0" 450 0c
	write_at "$T/dev/mmcblk0" 510 55 aa
	dd if=/dev/zero of="$T/dev/mmcblk0p1" bs=2048 count=1 status=none
	dd if=/dev/zero of="$T/dev/mmcblk0p2" bs=2048 count=1 status=none
	write_at "$T/dev/mmcblk0p2" 1080 53 ef
	# shellcheck disable=SC2086
	write_at "$T/dev/mmcblk0p2" 1082 $state
	printf '%s' "$label" | dd of="$T/dev/mmcblk0p2" bs=1 seek=1144 conv=notrunc status=none
}

setup() {
	R=$T/root
	rm -rf "$R"
	mkdir -p "$R/proc/device-tree/chosen/bootloader" "$R/proc/sys/kernel" "$R/data/slots" "$R/data/board/slots" "$R/dev"
	echo "console=ttyAMA10 atlas.slot=pv-a atlas.roothash=$hash atlas.data=PARTUUID=5056525a-02 panic=5" > "$R/proc/cmdline"
	echo 7.2.9-v8-16k > "$R/proc/sys/kernel/osrelease"
	printf '\000\000\000\000' > "$R/proc/device-tree/chosen/bootloader/tryboot"
	# One data block, the verity superblock and one hash block.
	dd if=/dev/zero of="$R/data/slots/pv-a.erofs" bs=4096 count=3 status=none
	echo '{"schema":1,"id":"pv-a","family":"photonvision","kernel":"7.2.9-v8-16k","data_dirs":["photonvision_config"],"image":{"size":12288},"verity":{"salt":"00","hash_offset":4096,"data_blocks":1}}' > "$R/data/slots/pv-a.json"
	echo '{"schema":1,"default":"pv-a","previous":"pv-b","trial":null}' > "$R/data/board/slots/state.json"
	: > "$R/dev/kmsg"
	: > "$T/calls"
	disk
}
run() {
	SLOT_ROOT=$R SLOT_CALLS=$T/calls SLOT_SYS=$T/sys SLOT_DEV=$T/dev SLOT_FAIL=${FAIL_AT:-} \
		"$B" boot 2> "$T/err" || true
}
called() { grep -qF -- "$1" "$T/calls"; }
said() { grep -qF -- "$1" "$R/dev/kmsg"; }

echo "a slot starts: data by PARTUUID, the kernel check, verity, erofs, its data dirs, switch_root"
setup
run
called "mount -t ext4 -o noatime,errors=remount-ro $T/dev/mmcblk0p2 $R/data" || fail "data mount: $(cat "$T/calls")"
called "modprobe dm-verity (7.2.9-v8-16k)" || fail "modules: $(cat "$T/calls")"
called "open $R/data/slots/pv-a.erofs pv-a $hash" || fail "verity: $(cat "$T/calls")"
called "mount -t erofs -o ro /dev/dm-0 $R/newroot" || fail "erofs: $(cat "$T/calls")"
called "mount --bind $R/data/os/photonvision/photonvision_config $R/data/photonvision_config" || fail "data dirs: $(cat "$T/calls")"
called "mount --move $R/data $R/newroot/data" || fail "move: $(cat "$T/calls")"
called "switch_root $R/newroot /sbin/init" || fail "switch_root: $(cat "$T/calls")"
[ "$(cat "$R/data/board/slots/booted")" = pv-a ] || fail "booted marker"
said "starting slot pv-a (photonvision, trial 0)" || fail "log: $(cat "$R/dev/kmsg")"

echo "a trial that starts leaves trial-started (so a failure later is the slot's own system's)"
setup
printf '\000\000\000\001' > "$R/proc/device-tree/chosen/bootloader/tryboot"
run
called "switch_root $R/newroot /sbin/init" || fail "didn't start: $(cat "$T/calls")"
[ "$(cat "$R/data/board/slots/trial-started")" = pv-a ] || fail "no trial-started"
[ "$(cat "$R/data/board/slots/trial-init")" = pv-a ] || fail "no trial-init"
said "starting slot pv-a (photonvision, trial 1)" || fail "log: $(cat "$R/dev/kmsg")"

echo "a cut-short image on a trial is refused with the reason, logged, and the board restarts into the default"
setup
printf '\000\000\000\001' > "$R/proc/device-tree/chosen/bootloader/tryboot"
truncate -s 8192 "$R/data/slots/pv-a.erofs"
run
called "open " && fail "opened a cut-short image"
said "the image is 8192 bytes, slot.json says 12288: damaged or cut short" || fail "the reason: $(cat "$R/dev/kmsg")"
called "reboot" || fail "no restart: $(cat "$T/calls")"
grep -q "kernel log" "$R/data/board/slots/last-failure.log" || fail "no last-failure.log after a trial failure"
grep -q "kernel log" "$R/boot/slot-init.log" || fail "no slot-init.log on p1 after a trial failure"
[ ! -e "$R/data/board/slots/trial-started" ] || fail "trial-started for a slot that didn't start"

echo "a missing image on a trial boot restarts plainly (into the default)"
setup
printf '\000\000\000\001' > "$R/proc/device-tree/chosen/bootloader/tryboot"
rm "$R/data/slots/pv-a.erofs"
run
called "reboot" || fail "no plain restart: $(cat "$T/calls")"
called switch_root && fail "it started anyway"
[ ! -e "$R/data/board/slots/fallbacks" ] || fail "a trial failure isn't a fallback"

echo "on the default slot it falls back to the previous one on trial, the log kept"
setup
rm "$R/data/slots/pv-a.erofs"
run
[ "$(cat "$R/boot/trial.txt")" = "os_prefix=slots/pv-b/" ] || fail "trial.txt: $(cat "$R/boot/trial.txt" 2>/dev/null)"
called "mount -t vfat -o rw $T/dev/mmcblk0p1 $R/boot" || fail "FAT mount: $(cat "$T/calls")"
called "reboot-tryboot" || fail "no tryboot restart"
[ "$(cat "$R/data/board/slots/fallbacks")" = 1 ] || fail "fallback count"
grep -q "kernel log" "$R/data/board/slots/last-failure.log" || fail "no last-failure.log"
grep -q "kernel log" "$R/boot/slot-init.log" || fail "no slot-init.log on p1"

echo "after three fallbacks in a row: no fourth, a plain restart"
setup
rm "$R/data/slots/pv-a.erofs"
echo 3 > "$R/data/board/slots/fallbacks"
run
called "reboot-tryboot" && fail "a fourth fallback"
called "reboot" || fail "no restart: $(cat "$T/calls")"
said "fell back 3 times in a row" || fail "the reason: $(cat "$R/dev/kmsg")"

echo "a first flash (no previous slot) restarts with the reason"
setup
echo '{"schema":1,"default":"pv-a","previous":null,"trial":null}' > "$R/data/board/slots/state.json"
FAIL_AT=verity run
said "dm-verity refused it: DM_TABLE_LOAD: Invalid argument; no previous slot" || fail "the reason: $(cat "$R/dev/kmsg")"
called "reboot" || fail "no restart"

echo "with nothing answering to the PARTUUID, /data is found by its label"
setup
sed -i 's/PARTUUID=5056525a-02/PARTUUID=00000000-02/' "$R/proc/cmdline"
run
said "using the one labelled data" || fail "no label note: $(cat "$R/dev/kmsg")"
called "switch_root $R/newroot /sbin/init" || fail "didn't start: $(cat "$T/calls")"

echo "no data partition at all: the partitions it saw are in the log, then a restart"
setup
disk other
sed -i 's/PARTUUID=5056525a-02/PARTUUID=00000000-02/' "$R/proc/cmdline"
run
said "mmcblk0p2 (partition 2 of mmcblk0, PARTUUID 5056525a-02, LABEL other)" || fail "candidates: $(cat "$R/dev/kmsg")"
said "no slot can start: no data partition PARTUUID=00000000-02" || fail "the reason: $(cat "$R/dev/kmsg")"
called "reboot" || fail "no restart"

echo "a filesystem marked as having errors is mounted read-only, with a note"
setup
disk data "03 00"
run
called "mount -t ext4 -o ro,noatime $T/dev/mmcblk0p2 $R/data" || fail "not read-only: $(cat "$T/calls")"
said "marked as having errors" || fail "no note"

echo "an image built for another kernel isn't started"
setup
echo 6.12.0 > "$R/proc/sys/kernel/osrelease"
run
called switch_root && fail "started a slot for another kernel"
said "built for kernel 7.2.9-v8-16k, this is 6.12.0" || fail "the reason: $(cat "$R/dev/kmsg")"

echo "no kernel release to read is its own failure, not a mismatch"
setup
rm "$R/proc/sys/kernel/osrelease"
run
called switch_root && fail "started without knowing the kernel"
said "couldn't read the running kernel's release" || fail "the reason: $(cat "$R/dev/kmsg")"

echo "no atlas.slot: nothing to start, a restart with the reason"
setup
echo "console=ttyAMA10" > "$R/proc/cmdline"
run
said "no slot can start: no atlas.slot on the kernel command line" || fail "the reason: $(cat "$R/dev/kmsg")"
called "reboot" || fail "no restart"

echo "ok"
