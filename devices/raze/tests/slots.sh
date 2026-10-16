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
check-initramfs) ! grep -q XZ "$2" || { echo "slot-init: $2: an xz initramfs isn't supported on the Raze" >&2; exit 1; }; echo zstd ;;
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

echo "a bundle whose initramfs the kernel can't unpack is refused before anything is written"
bundle pv-d 2026.4.2
echo XZ > "$T/build/pv-d/boot/initramfs"
python3 -c 'import json,hashlib,sys
p=sys.argv[1]; m=json.load(open(p+"/slot.json"))
m["boot"]["files"]["initramfs"]=hashlib.sha256(open(p+"/boot/initramfs","rb").read()).hexdigest()
json.dump(m,open(p+"/slot.json","w"))' "$T/build/pv-d"
tar -cf "$T/pv-d.slot.tar" -C "$T/build/pv-d" slot.json boot rootfs.erofs
[ "$(rc stage "$T/pv-d.slot.tar")" = 3 ] || fail "an unbootable initramfs was installed: $(cat "$T/err")"
grep -q "initramfs won't boot: .*xz initramfs isn't supported" "$T/err" || fail "the reason: $(cat "$T/err")"
[ ! -e "$T/data/slots/pv-d.erofs" ] && [ ! -d "$T/boot/slots/pv-d" ] || fail "pv-d left behind"

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
status | grep -q "didn't reach its initramfs" || fail "the reason for a trial that never got to /init: $(status)"

echo "a trial whose /init ran but stopped says so"
up apply 2>/dev/null || true
up stage "$T/pv-c.slot.tar" 2>/dev/null || true
up apply 2>/dev/null
echo pv-c > "$T/data/board/slots/trial-init"
up confirm 2>/dev/null # still on pv-b
status | grep -q "its initramfs stopped" || fail "the reason: $(status)"
[ ! -e "$T/data/board/slots/trial-init" ] || fail "trial-init kept"

echo "a trial whose initramfs started it but whose system didn't confirm says so"
up apply 2>/dev/null || true
up stage "$T/pv-c.slot.tar" 2>/dev/null || true
up apply 2>/dev/null
echo pv-c > "$T/data/board/slots/trial-started"
up confirm 2>/dev/null # still on pv-b
status | grep -q "started but its system didn't confirm" || fail "the reason: $(status)"
[ ! -e "$T/data/board/slots/trial-started" ] || fail "trial-started kept"

echo "an image that isn't the size it was built at is listed as not intact"
status | grep -q '"id":"pv-c"[^}]*"intact":true' || fail "pv-c intact: $(status)"
truncate -s 3 "$T/data/slots/pv-c.erofs"
status | grep -q '"id":"pv-c"[^}]*"intact":false' || fail "pv-c damaged: $(status)"

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
