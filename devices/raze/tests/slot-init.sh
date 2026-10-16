#!/bin/sh
# Off-device test of the slot initramfs's /init (gaia/assets/initramfs/init)
# with stub commands: a slot starts; a missing image on a trial restarts into
# the default; on the default it falls back to the previous slot on trial,
# at most three times; a kernel mismatch counts as a failure.
# Run: sh devices/raze/tests/slot-init.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
init=$here/../gaia/assets/initramfs/init
T=$(mktemp -d "${TMPDIR:-/tmp}/board-slot-init-test.XXXXXX")
trap 'rm -rf "$T"' EXIT
fail() {
	echo "FAIL: $*" >&2
	exit 1
}

mkdir -p "$T/bin"
cat > "$T/bin/slot-init" <<'STUB'
#!/bin/sh
echo "slot-init $*" >> "$CALLS"
case "$1" in
arg) sed -n "s/.*[[:space:]]$2=\([^[:space:]]*\).*/\1/p" "$SLOT_ROOT/proc/cmdline" | grep . ;;
kernel) sed -n 's/.*"kernel":"\([^"]*\)".*/\1/p' "$2" ;;
family) sed -n 's/.*"family":"\([^"]*\)".*/\1/p' "$2" ;;
data-dirs) sed -n 's/.*"data_dirs":\["\([^"]*\)"\].*/\1/p' "$2" ;;
previous) v=$(sed -n 's/.*"previous":"\([^"]*\)".*/\1/p' "$2"); [ -n "$v" ] && echo "$v" ;;
open) echo /dev/dm-0 ;;
reboot-tryboot) exit 0 ;;
esac
STUB
cat > "$T/bin/stub" <<'STUB'
#!/bin/sh
echo "$(basename "$0") $*" >> "$CALLS"
STUB
chmod +x "$T/bin/slot-init" "$T/bin/stub"
for c in mount e2fsck modprobe switch_root reboot shell; do ln -s stub "$T/bin/$c"; done
cat > "$T/bin/findfs" <<'STUB'
#!/bin/sh
echo "findfs $*" >> "$CALLS"
# NO_PARTUUID: a findfs that only knows labels.
case "$1" in PARTUUID=*) [ -z "${NO_PARTUUID:-}" ] || exit 1 ;; esac
echo /dev/mmcblk0p2
STUB
chmod +x "$T/bin/findfs"

setup() {
	R=$T/root
	rm -rf "$R"
	mkdir -p "$R/proc/device-tree/chosen/bootloader" "$R/data/slots" "$R/data/board/slots" "$R/newroot/data" "$R/dev"
	echo "console=ttyAMA10 atlas.slot=pv-a atlas.roothash=$(printf 'ab%.0s' $(seq 32)) atlas.data=PARTUUID=1234-02 panic=5" > "$R/proc/cmdline"
	printf '\000\000\000\000' > "$R/proc/device-tree/chosen/bootloader/tryboot"
	echo 'x' > "$R/data/slots/pv-a.erofs"
	echo '{"schema":1,"id":"pv-a","family":"photonvision","kernel":"7.2.9-v8-16k","data_dirs":["photonvision_config"],"verity":{}}' > "$R/data/slots/pv-a.json"
	echo '{"schema":1,"default":"pv-a","previous":"pv-b","trial":null}' > "$R/data/board/slots/state.json"
	: > "$T/calls"
}
run() {
	SLOT_ROOT=$R CALLS=$T/calls SLOT_INIT=$T/bin/slot-init SLOT_MOUNT=$T/bin/mount SLOT_E2FSCK=$T/bin/e2fsck \
		SLOT_MODPROBE=$T/bin/modprobe SLOT_SWITCH_ROOT=$T/bin/switch_root SLOT_REBOOT="$T/bin/reboot -f" \
		SLOT_SHELL=$T/bin/shell SLOT_FINDFS=$T/bin/findfs SLOT_UNAME="echo ${UNAME:-7.2.9-v8-16k}" SLOT_WAIT=1 \
		sh "$init" 2> "$T/log" || true
}
called() { grep -q -- "$1" "$T/calls"; }

echo "a slot starts: data, verity, erofs, its data dirs, switch_root"
setup
run
called "e2fsck -p /dev/mmcblk0p2" || fail "e2fsck: $(cat "$T/calls")"
called "mount -t ext4 -o noatime,errors=remount-ro /dev/mmcblk0p2 $R/data" || fail "data mount: $(cat "$T/calls")"
called "slot-init open $R/data/slots/pv-a.erofs $R/data/slots/pv-a.json abab" || fail "verity: $(cat "$T/calls")"
called "mount -t erofs -o ro /dev/dm-0 $R/newroot" || fail "erofs: $(cat "$T/calls")"
called "mount --bind $R/data/os/photonvision/photonvision_config $R/data/photonvision_config" || fail "data dirs: $(cat "$T/calls")"
called "switch_root $R/newroot /sbin/init" || fail "switch_root: $(cat "$T/calls")"
[ "$(cat "$R/data/board/slots/booted")" = pv-a ] || fail "booted marker"

echo "a missing image on a trial boot restarts plainly (into the default)"
setup
printf '\000\000\000\001' > "$R/proc/device-tree/chosen/bootloader/tryboot"
rm "$R/data/slots/pv-a.erofs"
run
called "reboot -f" || fail "no plain restart: $(cat "$T/calls")"
called switch_root && fail "it started anyway"
[ ! -e "$R/data/board/slots/fallbacks" ] || fail "a trial failure isn't a fallback"

echo "on the default slot it falls back to the previous one on trial"
setup
rm "$R/data/slots/pv-a.erofs"
run
[ "$(cat "$R/boot/trial.txt")" = "os_prefix=slots/pv-b/" ] || fail "trial.txt: $(cat "$R/boot/trial.txt" 2>/dev/null)"
called "mount -t vfat /dev/mmcblk0p1 $R/boot" || fail "FAT mount: $(cat "$T/calls")"
called "slot-init reboot-tryboot" || fail "no tryboot restart"
[ "$(cat "$R/data/board/slots/fallbacks")" = 1 ] || fail "fallback count"

echo "after three fallbacks in a row: a shell, not a fourth"
setup
rm "$R/data/slots/pv-a.erofs"
echo 3 > "$R/data/board/slots/fallbacks"
run
called "shell" || fail "no rescue shell: $(cat "$T/calls")"
called "reboot-tryboot" && fail "a fourth fallback"

echo "a findfs without PARTUUID finds /data by its label"
setup
NO_PARTUUID=1 run
called "findfs LABEL=data" || fail "no label lookup: $(cat "$T/calls")"
called "switch_root $R/newroot /sbin/init" || fail "didn't start: $(cat "$T/calls")"

echo "an image built for another kernel isn't started"
setup
UNAME=6.12.0 run
called switch_root && fail "started a slot for another kernel"
grep -q "built for kernel 7.2.9-v8-16k, this is 6.12.0" "$R/dev/kmsg" || fail "the reason: $(cat "$R/dev/kmsg")"

echo "ok"
