#!/bin/sh
# Off-device test of the slots writer (gaia/assets/rootfs/usr/lib/board/slots)
# and `update` handing over to it: install a bundle, refusals that change
# nothing, boot once, confirm, a failed trial, set-default, rollback, remove,
# pin, the device lock and the space reserve. slot-init's JSON reading is
# stubbed with python3. Run: sh devices/raze/tests/slots.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
lib=$here/../gaia/assets/rootfs/usr/lib/board
command -v python3 >/dev/null 2>&1 || {
	echo "slots: skipped (no python3 for the slot-init stub)"
	exit 0
}
T=$(mktemp -d "${TMPDIR:-/tmp}/board-slots-test.XXXXXX")
trap 'rm -rf "$T"' EXIT
fail() {
	echo "FAIL: $*" >&2
	exit 1
}

mkdir -p "$T/bin"
cat > "$T/bin/slot-init" <<'STUB'
#!/bin/sh
case "$1" in
field) exec python3 -c 'import json,sys
v=json.load(open(sys.argv[1]))
for k in sys.argv[2].split("."):
    if not isinstance(v,dict) or k not in v: sys.exit(1)
    v=v[k]
print(str(v).lower() if isinstance(v,bool) else v)' "$2" "$3" ;;
boot-files) exec python3 -c 'import json,sys
for n,h in json.load(open(sys.argv[1]))["boot"]["files"].items(): print(h+"  "+n)' "$2" ;;
esac
exit 1
STUB
chmod +x "$T/bin/slot-init"

export BOARD_LIB_DIR=$lib BOARD_ETC_DIR=$T/etc BOARD_RUN_DIR=$T/run BOARD_DATA_DIR=$T/data/board
export SLOTS_DATA=$T/data SLOTS_BOOT_MOUNT=$T/boot SLOT_INIT=$T/bin/slot-init UPDATE_SYNC=true
export UPDATE_SLOTS_STATE=$T/data/board/slots/state.json SLOTS_PARTUUID=abcd1234-02 SLOTS_RESERVE_KB=0
export UPDATE_REBOOT="echo tryboot >> $T/reboots" UPDATE_REBOOT_PLAIN="echo plain >> $T/reboots"
export BOARD_DEVICE_LOCK=$T/device-lock UPDATE_BOOT_ID_FILE=$T/boot_id
mkdir -p "$T/etc" "$T/run" "$T/boot/slots/pv-a" "$T/data/slots" "$T/data/board/slots"
echo boot-1 > "$T/boot_id"
echo 'os_prefix=slots/pv-a/' > "$T/boot/default.txt"
echo 'os_prefix=slots/pv-a/' > "$T/boot/trial.txt"
echo old-kernel > "$T/boot/slots/pv-a/kernel8.img"
echo a > "$T/data/slots/pv-a.erofs"
echo '{"schema":1,"id":"pv-a","family":"photonvision","version":"2026.3.0","variant":"styx","kernel":"7.2.9","verity":{"root_hash":"aa"}}' > "$T/data/slots/pv-a.json"
echo '{"schema":1,"default":"pv-a","previous":null,"trial":null}' > "$T/data/board/slots/state.json"
echo pv-a > "$T/data/board/slots/booted"

# bundle <id> <version>: a slot bundle in $T/<id>.slot.tar.
bundle() {
	b=$T/build/$1
	rm -rf "$b"
	mkdir -p "$b/boot/overlays"
	echo "kernel of $1" > "$b/boot/kernel8.img"
	echo "dtbo" > "$b/boot/overlays/bcm2712d0.dtbo"
	echo "console=ttyAMA10" > "$b/boot/cmdline.txt"
	printf 'erofs image of %s\n' "$1" > "$b/rootfs.erofs"
	size=$(wc -c < "$b/rootfs.erofs" | tr -d ' ')
	sha=$(sha256sum "$b/rootfs.erofs" | cut -d' ' -f1)
	k=$(sha256sum "$b/boot/kernel8.img" | cut -d' ' -f1)
	d=$(sha256sum "$b/boot/overlays/bcm2712d0.dtbo" | cut -d' ' -f1)
	c=$(sha256sum "$b/boot/cmdline.txt" | cut -d' ' -f1)
	printf '{"schema":1,"id":"%s","family":"photonvision","version":"%s","variant":"styx","kernel":"7.2.9","image":{"size":%s,"sha256":"%s"},"verity":{"root_hash":"%s","salt":"00","hash_offset":4096,"data_blocks":1},"boot":{"files":{"kernel8.img":"%s","overlays/bcm2712d0.dtbo":"%s","cmdline.txt":"%s"}}}\n' \
		"$1" "$2" "$size" "$sha" "$(printf 'cd%.0s' $(seq 32))" "$k" "$d" "$c" > "$b/slot.json"
	tar -cf "$T/$1.slot.tar" -C "$b" slot.json boot rootfs.erofs
}
up() { sh "$lib/update" "$@"; }
status() { up status; }
rc() {
	set +e
	up "$@" > "$T/out" 2> "$T/err"
	r=$?
	set -e
	echo "$r"
}

echo "status lists the installed slot"
status | grep -q '"layout":"slots"' || fail "status: $(status)"
status | grep -q '"id":"pv-a"' || fail "pv-a listed: $(status)"
(. "$lib/lib.sh" && board_update_methods) | grep -qx slots-tryboot || fail "the identity doesn't list slots-tryboot"

echo "a bundle installs as a slot: the boot dir on the FAT, the image on /data"
bundle pv-b 2026.4.0
up stage "$T/pv-b.slot.tar" --sha256 "$(sha256sum "$T/pv-b.slot.tar" | cut -d' ' -f1)" 2>/dev/null
[ -f "$T/data/slots/pv-b.erofs" ] && [ -f "$T/data/slots/pv-b.json" ] || fail "image or manifest missing"
grep -q "atlas.slot=pv-b atlas.roothash=cdcd" "$T/boot/slots/pv-b/cmdline.txt" || fail "cmdline: $(cat "$T/boot/slots/pv-b/cmdline.txt")"
grep -q "atlas.data=PARTUUID=abcd1234-02" "$T/boot/slots/pv-b/cmdline.txt" || fail "PARTUUID"
[ -f "$T/boot/slots/pv-b/overlays/bcm2712d0.dtbo" ] || fail "overlays not copied"
status | grep -q '"slot_staged":"pv-b"' || fail "staged: $(status)"
ls "$T/data/slots" | grep -q install && fail "the work dir was left behind"

echo "refusals change nothing: wrong --sha256, the running slot, a broken file, no room, the device lock"
[ "$(rc stage "$T/pv-b.slot.tar" --sha256 0000)" = 3 ] || fail "wrong sha accepted"
bundle pv-a 2026.3.1
[ "$(rc stage "$T/pv-a.slot.tar")" = 3 ] || fail "the running slot was overwritten"
grep -q old-kernel "$T/boot/slots/pv-a/kernel8.img" || fail "the running slot's boot files changed"
bundle pv-c 2026.4.1
echo tampered >> "$T/build/pv-c/rootfs.erofs"
tar -cf "$T/pv-c.slot.tar" -C "$T/build/pv-c" slot.json boot rootfs.erofs
[ "$(rc stage "$T/pv-c.slot.tar")" = 1 ] || fail "a tampered image was installed"
[ ! -e "$T/data/slots/pv-c.erofs" ] || fail "tampered image left on /data"
[ "$(SLOTS_RESERVE_KB=999999999 rc stage "$T/pv-b.slot.tar")" = 3 ] || fail "no room wasn't refused"
mkdir "$T/device-lock"
[ "$(rc stage "$T/pv-b.slot.tar")" = 3 ] || fail "the device lock didn't hold"
rmdir "$T/device-lock"

echo "boot once writes trial.txt and restarts with tryboot"
up apply 2>/dev/null
[ "$(cat "$T/boot/trial.txt")" = "os_prefix=slots/pv-b/" ] || fail "trial.txt: $(cat "$T/boot/trial.txt")"
[ "$(tail -n 1 "$T/reboots")" = tryboot ] || fail "no tryboot restart"
[ "$(cat "$T/boot/default.txt")" = "os_prefix=slots/pv-a/" ] || fail "the default changed before confirm"

echo "confirm on the trial makes it the default, with a root copy"
echo pv-b > "$T/data/board/slots/booted"
up confirm 2>/dev/null
[ "$(cat "$T/boot/default.txt")" = "os_prefix=slots/pv-b/" ] || fail "default.txt: $(cat "$T/boot/default.txt")"
grep -q "kernel of pv-b" "$T/boot/kernel8.img" || fail "root copy"
status | grep -q '"slot_default":"pv-b","slot_previous":"pv-a"' || fail "state: $(status)"
grep -q '"previous":"pv-a"' "$T/data/board/slots/state.json" || fail "state.json for the initramfs: $(cat "$T/data/board/slots/state.json")"

echo "a trial that didn't come up is noted on the next confirm"
up stage "$T/pv-c.slot.tar" 2>/dev/null || true
bundle pv-c 2026.4.1
up stage "$T/pv-c.slot.tar" 2>/dev/null
up apply 2>/dev/null
up confirm 2>/dev/null # still running pv-b: pv-c never started
status | grep -q '"state":"rolled-back"' || fail "rolled-back: $(status)"
grep -q '^GOOD=no' "$T/data/board/slots/pv-c.meta" || fail "pv-c marked bad"
[ "$(rc set-default pv-c)" = 3 ] || fail "set-default on a slot that never booted"

echo "remove: not the running or default slot, not a pinned one"
[ "$(rc remove pv-b)" = 3 ] || fail "removed the running slot"
up pin pv-a
[ "$(rc remove pv-a)" = 3 ] || fail "removed a pinned slot"
up unpin pv-a
up remove pv-c 2>/dev/null
[ ! -e "$T/data/slots/pv-c.erofs" ] && [ ! -d "$T/boot/slots/pv-c" ] || fail "pv-c not removed"

echo "rollback makes the previous slot the default"
up rollback --no-reboot 2>/dev/null
[ "$(cat "$T/boot/default.txt")" = "os_prefix=slots/pv-a/" ] || fail "rollback default.txt"
status | grep -q '"slot_default":"pv-a","slot_previous":"pv-b"' || fail "rollback state: $(status)"

echo "ok"
