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
[ -z "${HELPER_LOG:-}" ] || echo "$1" >> "$HELPER_LOG"
case "$1" in
field) exec python3 -c 'import json,sys
v=json.load(open(sys.argv[1]))
for k in sys.argv[2].split("."):
    if not isinstance(v,dict) or k not in v: sys.exit(1)
    v=v[k]
if v is None or isinstance(v,(dict,list)): sys.exit(1)
print(str(v).lower() if isinstance(v,bool) else v)' "$2" "$3" ;;
fields) f=$2; shift 2; exec python3 -c 'import json,sys
v=json.load(open(sys.argv[1]))
out=[]
for p in sys.argv[2:]:
    x=v
    for k in p.split("."):
        x=x.get(k) if isinstance(x,dict) else None
    s="" if x is None or isinstance(x,(dict,list)) else str(x).lower() if isinstance(x,bool) else str(x)
    if "\n" in s or "\0" in s: sys.exit(1)
    out.append(s)
print("\n".join(out))' "$f" "$@" ;;
flush) echo "$2" >> "$FLUSH_LOG"; exit 0 ;;
check-initramfs) ! grep -q XZ "$2" || { echo "slot-init: $2: an xz initramfs isn't supported on the Raze" >&2; exit 1; }; echo zstd ;;
layers) exec python3 -c 'import json,sys
m=json.load(open(sys.argv[1]))
if not m.get("layers"): sys.exit(1)
for l in m["layers"]: print(l["name"], l["id"], l["size"], l["sha256"], l["verity"]["root_hash"])' "$2" ;;
set-layer) exec python3 -c 'import json,sys
m=json.load(open(sys.argv[1])); l=json.load(open(sys.argv[2]))
if not m.get("layers"): sys.exit(1)
names=[x["name"] for x in m["layers"]]
if l["name"] not in names: sys.exit(1)
m["layers"][names.index(l["name"])]=l
print(json.dumps(m))' "$2" "$3" ;;
open) echo "open $2 $4 $5" >> "$LIVE_CALLS"; echo /dev/dm-9; exit 0 ;;
boot-files) exec python3 -c 'import json,sys
for n,h in json.load(open(sys.argv[1]))["boot"]["files"].items(): print(h+"  "+n)' "$2" ;;
esac
exit 1
STUB
chmod +x "$T/bin/slot-init"

export BOARD_LIB_DIR=$lib BOARD_ETC_DIR=$T/etc BOARD_RUN_DIR=$T/run BOARD_DATA_DIR=$T/data/board BOARD_DT_DIR=$T/dt
export SLOTS_DATA=$T/data SLOTS_BOOT_MOUNT=$T/boot SLOT_INIT=$T/bin/slot-init UPDATE_SYNC=true
export SLOTS_BOOT_DEV=/dev/fake-p1 FLUSH_LOG=$T/flushes HELPER_LOG=$T/helper-calls
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

# bundle <id> <version> [display_name]: a slot bundle in $T/<id>.slot.tar.
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
	if [ -n "${3:-}" ]; then
		python3 -c 'import json,sys
p=sys.argv[1]; m=json.load(open(p)); m["display_name"]=sys.argv[2]; json.dump(m,open(p,"w"))' "$b/slot.json" "$3"
	fi
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

echo "status reads each slot's manifest in one helper run"
: > "$T/helper-calls"
status > /dev/null
[ "$(grep -c '^fields$' "$T/helper-calls")" = 1 ] || fail "helper runs for pv-a's fields: $(cat "$T/helper-calls")"

echo "status lists the installed slot"
status | grep -q '"layout":"slots"' || fail "status: $(status)"
status | grep -q '"id":"pv-a"' || fail "pv-a listed: $(status)"
(. "$lib/lib.sh" && board_update_methods) | grep -qx slots-tryboot || fail "the identity doesn't list slots-tryboot"

echo "check says what stage would do and changes nothing (no state, no files, no event, no lock)"
bundle pv-b 2026.4.0 "PhotonVision Lab"
snap() { (cd "$T" && find data boot run -type f -exec sha256sum {} + 2>/dev/null | sort); }
snap > "$T/before"
[ "$(rc check "$T/pv-b.slot.tar")" = 0 ] || fail "check refused a good bundle: $(cat "$T/out" "$T/err")"
grep -q '"ok":true,"id":"pv-b","family":"photonvision","version":"2026.4.0"' "$T/out" || fail "check: $(cat "$T/out")"
grep -q '"replaces":false' "$T/out" || fail "check: pv-b isn't installed yet: $(cat "$T/out")"
snap > "$T/after"
cmp -s "$T/before" "$T/after" || fail "check changed files: $(diff "$T/before" "$T/after")"
ls "${TMPDIR:-/tmp}" | grep -q '^slots-check' && fail "check left its temporary dir"
mkdir "$T/device-lock" && echo test > "$T/device-lock/owner"
[ "$(rc check "$T/pv-b.slot.tar")" = 0 ] || fail "check refused under the device lock: $(cat "$T/out")"
rm -rf "$T/device-lock"
bundle pv-a 2026.3.1
[ "$(rc check "$T/pv-a.slot.tar")" = 3 ] || fail "check: the running slot"
grep -q '"ok":false,"refused":true,"error":"pv-a is the running slot"' "$T/out" || fail "check refusal: $(cat "$T/out")"
bundle pv-c 2026.4.1
echo tampered >> "$T/build/pv-c/rootfs.erofs"
tar -cf "$T/pv-c.slot.tar" -C "$T/build/pv-c" slot.json boot rootfs.erofs
[ "$(rc check "$T/pv-c.slot.tar")" = 1 ] || fail "check: a broken image passed"
grep -q '"refused":false,"error":"rootfs.erofs isn' "$T/out" || fail "check: broken: $(cat "$T/out")"
(cd "$T/build/pv-b" && tar -cf "$T/pv-b-dot.slot.tar" ./slot.json ./boot ./rootfs.erofs)
[ "$(rc check "$T/pv-b-dot.slot.tar")" = 0 ] || fail "check: a tar with ./ names: $(cat "$T/out" "$T/err")"
[ "$(SLOTS_RESERVE_KB=999999999 rc check "$T/pv-b.slot.tar")" = 3 ] || fail "check: no room"
grep -q 'no room for this slot' "$T/out" || fail "check: no room: $(cat "$T/out")"
snap > "$T/after"
cmp -s "$T/before" "$T/after" || fail "failed checks changed files: $(diff "$T/before" "$T/after")"

echo "a bundle installs as a slot: the boot dir on the FAT, the image on /data"
bundle pv-b 2026.4.0 "PhotonVision Lab"
up stage "$T/pv-b.slot.tar" --sha256 "$(sha256sum "$T/pv-b.slot.tar" | cut -d' ' -f1)" 2>/dev/null
[ -f "$T/data/slots/pv-b.erofs" ] && [ -f "$T/data/slots/pv-b.json" ] || fail "image or manifest missing"
grep -q "atlas.slot=pv-b atlas.roothash=cdcd" "$T/boot/slots/pv-b/cmdline.txt" || fail "cmdline: $(cat "$T/boot/slots/pv-b/cmdline.txt")"
grep -q "atlas.data=PARTUUID=abcd1234-02" "$T/boot/slots/pv-b/cmdline.txt" || fail "PARTUUID"
[ -f "$T/boot/slots/pv-b/overlays/bcm2712d0.dtbo" ] || fail "overlays not copied"
[ -f "$T/boot/slots/pv-b/overlays/README" ] || fail "no overlays/README: the firmware would ignore the slot's overlays"
status | grep -q '"slot_staged":"pv-b"' || fail "staged: $(status)"
status | grep -q '"id":"pv-b"[^}]*"display_name":"PhotonVision Lab"' || fail "pv-b display_name: $(status)"
status | grep -q '"id":"pv-a"[^}]*"display_name":null' || fail "pv-a without display_name isn't null: $(status)"
ls "$T/data/slots" | grep -q install && fail "the work dir was left behind"
grep -qx /dev/fake-p1 "$T/flushes" || fail "the FAT's device wasn't flushed after the slot's boot files"
status | grep -q '"id":"pv-a"[^}]*"size":2,' || fail "pv-a's size isn't its file's: $(status)"

echo "a bundle on stdin installs the same, and its tar never stays on /data"
up stage - --sha256 "$(sha256sum "$T/pv-b.slot.tar" | cut -d' ' -f1)" < "$T/pv-b.slot.tar" 2>/dev/null || fail "stdin stage failed"
[ -z "$(ls "$T/data/board/update" 2>/dev/null)" ] || fail "the streamed tar was left: $(ls "$T/data/board/update")"
[ "$(rc stage - --sha256 0000 < "$T/pv-b.slot.tar")" = 3 ] || fail "stdin: wrong sha accepted"
[ -z "$(ls "$T/data/board/update" 2>/dev/null)" ] || fail "a refused streamed tar was left"

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
up confirm 2>/dev/null # still running pv-b: pv-c never started (tried once more)
up confirm 2>/dev/null # and again: rolled back
status | grep -q '"state":"rolled-back"' || fail "rolled-back: $(status)"
grep -q '^GOOD=no' "$T/data/board/slots/pv-c.meta" || fail "pv-c marked bad"
[ "$(rc set-default pv-c)" = 3 ] || fail "set-default on a slot that never booted"
status | grep -q "didn't reach its initramfs" || fail "the reason for a trial that never got to /init: $(status)"
status | grep -q "power-on" && fail "a power-on blamed without a reset register: $(status)"

echo "a trial that didn't reach /init is tried once more, then rolled back; tryboot 1 says the firmware started another slot"
: > "$T/flushes"
up apply 2>/dev/null || true
up stage "$T/pv-c.slot.tar" 2>/dev/null || true
up apply 2>/dev/null
grep -qx /dev/fake-p1 "$T/flushes" || fail "trial.txt wasn't flushed before the restart"
mkdir -p "$T/dt/chosen/bootloader"
printf '\000\000\000\000' > "$T/dt/chosen/bootloader/tryboot"
: > "$T/reboots"
: > "$T/flushes"
up confirm 2>/dev/null
[ "$(cat "$T/reboots")" = tryboot ] || fail "a lost tryboot flag isn't tried once more: $(cat "$T/reboots")"
status | grep -q '"slot_trial":"pv-c"' || fail "the retry's trial: $(status)"
grep -q 'update.retry' "$T/data/board/events.jsonl" || fail "no update.retry event"
grep -q "os_prefix=slots/pv-c/" "$T/boot/trial.txt" || fail "trial.txt for the retry"
grep -qx /dev/fake-p1 "$T/flushes" || fail "the retry's trial.txt wasn't flushed"
up confirm 2>/dev/null # lost again: no third try
[ "$(cat "$T/reboots")" = tryboot ] || fail "tried a third time: $(cat "$T/reboots")"
status | grep -q "didn't reach its initramfs's /init (its kernel stopped before it, or the restart lost the tryboot flag)" || fail "a trial that didn't reach /init: $(status)"
grep -q '^GOOD=no' "$T/data/board/slots/pv-c.meta" || fail "a slot that didn't start isn't marked bad: $(cat "$T/data/board/slots/pv-c.meta")"
up apply 2>/dev/null
printf '\000\000\000\001' > "$T/dt/chosen/bootloader/tryboot"
up confirm 2>/dev/null
status | grep -q "the firmware did the trial boot but started pv-b" || fail "a tryboot that started another slot: $(status)"
grep -q '^GOOD=no' "$T/data/board/slots/pv-c.meta" || fail "pv-c marked bad after a real trial"
rm -rf "$T/dt"

echo "a trial noticed on a boot that began at power-on says the board lost power"
up apply 2>/dev/null || true
up stage "$T/pv-c.slot.tar" 2>/dev/null || true
up apply 2>/dev/null
mkdir -p "$T/dt/chosen/bootloader"
printf '\000\000\020\000' > "$T/dt/chosen/bootloader/rsts" # PM_RSTS 0x1000
up confirm 2>/dev/null # tried once more
up confirm 2>/dev/null
status | grep -q "began at power-on (the board lost power)" || fail "the power-on wasn't said: $(status)"
printf '\000\000\000\040' > "$T/dt/chosen/bootloader/rsts" # 0x20, a reset
(. "$lib/lib.sh" && [ "$(board_reset_cause)" = reset ]) || fail "0x20 isn't a reset"
rm -rf "$T/dt"

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

# lbundle <id> <out.tar> <name:content>...: a layered slot bundle; each
# layer's file is its content, its id the sha256 of it (as Gaia names them),
# its root hash a fixed one per name. Layers named with a leading "-" are
# listed in slot.json but left out of the tar (the board should have them).
lbundle() {
	id=$1 out=$2
	shift 2
	b=$T/build/$id
	rm -rf "$b"
	mkdir -p "$b/boot/overlays" "$b/layers"
	echo "kernel of $id" > "$b/boot/kernel8.img"
	echo "console=ttyAMA10" > "$b/boot/cmdline.txt"
	k=$(sha256sum "$b/boot/kernel8.img" | cut -d' ' -f1)
	c=$(sha256sum "$b/boot/cmdline.txt" | cut -d' ' -f1)
	layers='' members='slot.json boot'
	for spec in "$@"; do
		skip=''
		case $spec in -*) skip=1 spec=${spec#-} ;; esac
		name=${spec%%:*}
		printf '%s\n' "${spec#*:}" > "$b/l"
		lid=$(sha256sum "$b/l" | cut -d' ' -f1)
		mv "$b/l" "$b/layers/$lid.erofs"
		size=$(wc -c < "$b/layers/$lid.erofs" | tr -d ' ')
		root=$(printf '%s' "$name" | sha256sum | cut -d' ' -f1)
		layers="$layers${layers:+,}{\"name\":\"$name\",\"id\":\"$lid\",\"size\":$size,\"sha256\":\"$lid\",\"verity\":{\"root_hash\":\"$root\",\"salt\":\"00\",\"hash_offset\":4096,\"data_blocks\":1}}"
		if [ -n "$skip" ]; then rm "$b/layers/$lid.erofs"; else members="$members layers/$lid.erofs"; fi
	done
	printf '{"schema":2,"id":"%s","family":"helios","version":"2026.2.0","variant":"styx","kernel":"7.2.9","layers":[%s],"boot":{"files":{"kernel8.img":"%s","cmdline.txt":"%s"}}}\n' "$id" "$layers" "$k" "$c" > "$b/slot.json"
	# shellcheck disable=SC2086
	tar -cf "$out" -C "$b" $members
}
lid() { printf '%s\n' "$1" | sha256sum | cut -d' ' -f1; }
rh() { printf '%s' "$1" | sha256sum | cut -d' ' -f1; }

echo "a layered bundle installs its layers into the shared layer store, and the cmdline lists their root hashes"
lbundle hx-a "$T/hx-a.slot.tar" base:base-1 platform:raze-1 app:app-1
up stage "$T/hx-a.slot.tar" 2>/dev/null || fail "layered stage: $(rc stage "$T/hx-a.slot.tar"; cat "$T/err")"
for c in base-1 raze-1 app-1; do
	[ -f "$T/data/slots/layers/$(lid $c).erofs" ] || fail "layer $c not in the store: $(ls "$T/data/slots/layers")"
done
[ ! -e "$T/data/slots/hx-a.erofs" ] || fail "a layered slot got a one-image file"
grep -q "atlas.slot=hx-a atlas.roothash=$(rh base),$(rh platform),$(rh app) " "$T/boot/slots/hx-a/cmdline.txt" || fail "cmdline: $(cat "$T/boot/slots/hx-a/cmdline.txt")"
status | grep -q '"id":"hx-a"[^]]*"intact":true' || fail "hx-a intact: $(status)"
status | grep -q '"id":"hx-a",[^{]*"display_name":null,"layers":\[{"name":"base"' || fail "hx-a layers listed: $(status)"
status | grep -q '"id":"pv-a"[^}]*"layers":null' || fail "a one-image slot's layers aren't null: $(status)"

echo "a second slot sharing base and platform ships only its app layer; check says what it reuses"
lbundle hx-b "$T/hx-b.slot.tar" -base:base-1 -platform:raze-1 app:app-2
[ "$(rc check "$T/hx-b.slot.tar")" = 0 ] || fail "check: $(cat "$T/out" "$T/err")"
grep -q '"layers_reused":2' "$T/out" || fail "check reuse: $(cat "$T/out")"
up stage "$T/hx-b.slot.tar" 2>/dev/null || fail "app-only stage: $(cat "$T/err")"
[ "$(ls "$T/data/slots/layers" | wc -l | tr -d ' ')" = 4 ] || fail "layers: $(ls "$T/data/slots/layers")"

echo "a layer that's neither in the bundle nor on the board, or doesn't match, is refused; nothing changes"
lbundle hx-c "$T/hx-c.slot.tar" -base:base-9 platform:raze-1 app:app-3
[ "$(rc stage "$T/hx-c.slot.tar")" = 1 ] || fail "a missing layer was accepted"
grep -q "layer base ($(lid base-9)) is neither in the bundle nor on the board" "$T/err" || fail "reason: $(cat "$T/err")"
[ "$(rc check "$T/hx-c.slot.tar")" = 1 ] || fail "check: a missing layer passed"
lbundle hx-c "$T/hx-c.slot.tar" base:base-1 platform:raze-1 app:app-3
echo tampered >> "$T/build/hx-c/layers/$(lid app-3).erofs"
tar -cf "$T/hx-c.slot.tar" -C "$T/build/hx-c" slot.json boot "layers/$(lid app-3).erofs"
[ "$(rc stage "$T/hx-c.slot.tar")" = 1 ] || fail "a tampered layer was accepted"
[ ! -e "$T/data/slots/hx-c.json" ] && [ ! -e "$T/data/slots/layers/$(lid app-3).erofs" ] || fail "a refused layer was kept"
[ "$(ls "$T/data/slots/layers" | wc -l | tr -d ' ')" = 4 ] || fail "layers after refusals: $(ls "$T/data/slots/layers")"

echo "removing a layered slot keeps the layers another slot has, drops its own"
up remove hx-a 2>/dev/null || fail "remove hx-a: $(cat "$T/err")"
[ -f "$T/data/slots/layers/$(lid base-1).erofs" ] && [ -f "$T/data/slots/layers/$(lid raze-1).erofs" ] || fail "shared layers went"
[ ! -e "$T/data/slots/layers/$(lid app-1).erofs" ] || fail "hx-a's own app layer stayed"
[ -f "$T/data/slots/layers/$(lid app-2).erofs" ] || fail "hx-b's app layer went"
truncate -s 1 "$T/data/slots/layers/$(lid app-2).erofs"
status | grep -q '"id":"hx-b"[^]]*"intact":false' || fail "a short layer isn't listed as damaged: $(status)"
up remove hx-b 2>/dev/null
[ -z "$(ls "$T/data/slots/layers")" ] || fail "layers left with no slot: $(ls "$T/data/slots/layers")"

# A layered slot to push to: hx-p, "running".
lbundle hx-p "$T/hx-p.slot.tar" base:base-p platform:raze-p app:app-p
up stage "$T/hx-p.slot.tar" 2>/dev/null || fail "stage hx-p: $(cat "$T/err")"
# The live side's hooks: mounts are recorded, and an erofs "mount" unpacks
# the layer (a tar of its tree here) where it would show.
mkdir -p "$T/live/usr" "$T/live/opt" "$T/live/etc"
cat > "$T/bin/mount" <<'HOOK'
#!/bin/sh
echo "mount $*" >> "$LIVE_CALLS"
[ "$1 $2" = "-t erofs" ] || exit 0
eval "dir=\${$#}"
tar -xf "$PUSH_TREE" -C "$dir"
HOOK
cat > "$T/bin/umount" <<'HOOK'
#!/bin/sh
echo "umount $*" >> "$LIVE_CALLS"
HOOK
cat > "$T/bin/systemctl" <<'HOOK'
#!/bin/sh
echo "systemctl $*" >> "$LIVE_CALLS"
[ "$1 $2 $3" != "is-active -q helios-idle.service" ]
HOOK
chmod +x "$T/bin/mount" "$T/bin/umount" "$T/bin/systemctl"
: > "$T/mountinfo"
# ptree <dir> <tar> <name> <tree...>: a layer push: layer.json and <id>.erofs.
ptree() {
	d=$T/build/push-$1
	rm -rf "$d"
	mkdir -p "$d/tree/usr/lib/systemd/system" "$d/tree/usr/bin" "$d/tree/etc" "$d/out"
	echo "helios $1" > "$d/tree/usr/bin/helios"
	echo "[Service]" > "$d/tree/usr/lib/systemd/system/helios-api.service"
	echo "[Service]" > "$d/tree/usr/lib/systemd/system/helios-idle.service"
	echo "[Service]" > "$d/tree/usr/lib/systemd/system/helios-diag@.service"
	echo "conf $1" > "$d/tree/etc/helios.conf"
	tar -cf "$d/layer.img" -C "$d/tree" usr etc
	plid=$(sha256sum "$d/layer.img" | cut -d' ' -f1)
	cp "$d/layer.img" "$d/out/$plid.erofs"
	psize=$(wc -c < "$d/layer.img" | tr -d ' ')
	printf '{"name":"app","id":"%s","size":%s,"sha256":"%s","verity":{"root_hash":"%s","salt":"00","hash_offset":4096,"data_blocks":1}}\n' \
		"$plid" "$psize" "$plid" "$(rh "push-$1")" > "$d/out/layer.json"
	tar -cf "$2" -C "$d/out" layer.json "$plid.erofs"
	PUSH_TREE=$d/layer.img
}
push() {
	set +e
	SLOTS_RUNNING=hx-p SLOTS_MOUNT=$T/bin/mount SLOTS_UMOUNT=$T/bin/umount UPDATE_SYSTEMCTL=$T/bin/systemctl \
		SLOTS_MOUNTINFO=$T/mountinfo SLOTS_LIVE_ROOT=$T/live LIVE_CALLS=$T/live-calls PUSH_TREE=$PUSH_TREE \
		sh "$lib/update" push "$@" > "$T/out" 2> "$T/err"
	r=$?
	set -e
	echo "$r"
}

echo "push: an app layer replaces the running slot's, kept for the next boot and laid over /usr now"
ptree v2 "$T/push-v2.tar"
v2=$plid
: > "$T/live-calls"
[ "$(push "$T/push-v2.tar")" = 0 ] || fail "push: $(cat "$T/out" "$T/err")"
[ -f "$T/data/slots/layers/$v2.erofs" ] || fail "the pushed layer isn't in the store"
grep -q "\"id\": *\"$v2\"" "$T/data/slots/hx-p.json" || fail "the manifest doesn't list it: $(cat "$T/data/slots/hx-p.json")"
grep -q "atlas.roothash=$(rh base),$(rh platform),$(rh push-v2) " "$T/boot/slots/hx-p/cmdline.txt" || fail "cmdline: $(cat "$T/boot/slots/hx-p/cmdline.txt")"
[ ! -e "$T/data/slots/layers/$(lid app-p).erofs" ] || [ -e "$T/data/board/slots/hx-p.prev.json" ] || fail "the replaced layer went without a revert record"
[ -f "$T/data/slots/layers/$(lid app-p).erofs" ] || fail "the replaced layer went (push --revert needs it): $(ls "$T/data/slots/layers" "$T/data/board/slots"; cat "$T/data/board/slots/hx-p.prev.json")"
grep -qx "open $T/data/slots/layers/$v2.erofs $(rh push-v2) push-app-$(printf '%s' "$v2" | cut -c1-12)" "$T/live-calls" || fail "verity open: $(cat "$T/live-calls")"
grep -q "^mount --bind $T/live/ $T/run/layers/root$" "$T/live-calls" || fail "booted root bind: $(cat "$T/live-calls")"
grep -q "^mount -t overlay overlay -o ro,lowerdir=$T/run/layers/app-[0-9a-f]*/usr:$T/run/layers/root/usr $T/live/usr$" "$T/live-calls" || fail "usr overlay: $(cat "$T/live-calls")"
grep -q "overlay.*$T/live/etc" "$T/live-calls" && fail "/etc was laid over live"
grep -q "^systemctl restart helios-api.service$" "$T/live-calls" || fail "the running unit wasn't restarted: $(cat "$T/live-calls")"
grep -q "restart helios-idle.service" "$T/live-calls" && fail "an inactive unit was started"
grep -q "helios-diag@" "$T/live-calls" && fail "a template unit was touched"
grep -q '"live":\["/usr"\],"next_boot":\["/etc"\],"restarted":\["helios-api.service"\]' "$T/out" || fail "result: $(cat "$T/out")"
grep -q '"kind":"slot.layer-pushed"' "$T/data/board/events.jsonl" || fail "no event"

echo "a second push swaps the live overlay (not stacked on the first) and drops the layer nothing needs"
ptree v3 "$T/push-v3.tar"
v3=$plid
: > "$T/live-calls"
[ "$(push "$T/push-v3.tar")" = 0 ] || fail "push v3: $(cat "$T/out" "$T/err")"
grep -q "^umount -l $T/live/usr$" "$T/live-calls" || fail "the old overlay stayed: $(cat "$T/live-calls")"
grep -q -- "--bind" "$T/live-calls" && fail "the booted root was bound again"
grep -q "lowerdir=$T/run/layers/app-[0-9a-f]*/usr:$T/run/layers/root/usr" "$T/live-calls" || fail "v3 overlay: $(cat "$T/live-calls")"
[ ! -e "$T/data/slots/layers/$(lid app-p).erofs" ] || fail "the layer two pushes back stayed"
[ -f "$T/data/slots/layers/$v2.erofs" ] || fail "the layer a revert needs went"

echo "pushing what's already there changes nothing; --revert puts back the previous layer"
[ "$(push "$T/push-v3.tar")" = 0 ] && grep -q '"changed":false' "$T/out" || fail "same layer: $(cat "$T/out")"
[ "$(push --revert app --no-restart)" = 0 ] || fail "revert: $(cat "$T/out" "$T/err")"
grep -q "atlas.roothash=$(rh base),$(rh platform),$(rh push-v2) " "$T/boot/slots/hx-p/cmdline.txt" || fail "revert cmdline: $(cat "$T/boot/slots/hx-p/cmdline.txt")"

echo "a push is refused on a one-image slot, under the device lock, and when it doesn't match"
set +e
SLOTS_RUNNING=pv-a sh "$lib/update" push "$T/push-v2.tar" > /dev/null 2>&1
r=$?
set -e
[ "$r" = 3 ] || fail "pushed to a one-image slot ($r)"
mkdir "$T/device-lock" && echo test > "$T/device-lock/owner"
[ "$(push "$T/push-v3.tar")" = 3 ] || fail "pushed under the device lock"
rm -rf "$T/device-lock"
ptree v4 "$T/push-v4.tar"
echo tampered >> "$T/build/push-v4/out/$plid.erofs"
tar -cf "$T/push-v4.tar" -C "$T/build/push-v4/out" layer.json "$plid.erofs"
[ "$(push "$T/push-v4.tar")" = 1 ] || fail "a tampered layer was pushed"
[ ! -e "$T/data/slots/layers/$plid.erofs" ] || fail "a refused layer was kept"
grep -q "atlas.roothash=$(rh base),$(rh platform),$(rh push-v2) " "$T/boot/slots/hx-p/cmdline.txt" || fail "a refused push changed the cmdline"

echo "a dir with something mounted under it is left to the next boot"
ptree v5 "$T/push-v5.tar"
echo "36 25 0:31 / $T/live/usr/local rw - tmpfs tmpfs rw" > "$T/mountinfo"
: > "$T/live-calls"
[ "$(push "$T/push-v5.tar")" = 0 ] || fail "push v5: $(cat "$T/out" "$T/err")"
grep -q "overlay.*$T/live/usr$" "$T/live-calls" && fail "/usr was laid over a submount"
grep -q '"next_boot":\["/usr","/etc"\]' "$T/out" || fail "result: $(cat "$T/out")"

echo "ok"
