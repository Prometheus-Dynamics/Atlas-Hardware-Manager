#!/bin/sh
# Off-device test of the slots writer (gaia/assets/rootfs/usr/lib/board/slots)
# and `update` handing over to it: install a bundle, refusals that change
# nothing, boot once, confirm, a failed trial, set-default, rollback, remove,
# pin, the device lock, the space reserve and the boot FAT's free space. slot-init's JSON commands are
# the real slot-init (built with cargo, or SLOT_INIT_BIN), python3 stubs
# without one; its device commands are stubbed. Run: sh devices/common/board-os/tests/slots.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
command -v python3 >/dev/null 2>&1 || {
	echo "slots: skipped (no python3 for the slot-init stub)"
	exit 0
}
T=$(mktemp -d "${TMPDIR:-/tmp}/board-slots-test.XXXXXX")
trap 'rm -rf "$T"' EXIT
_devices=$here/../../..
. "$here/compose-lib.inc"
lib=$T/lib
compose_lib "$lib"
fail() {
	echo "FAIL: $*" >&2
	exit 1
}

# The real slot-init for what it reads and writes, as on the board (a stub
# once hid that it didn't take `merge-layers`).
REAL_SLOT_INIT=${SLOT_INIT_BIN:-}
if [ -z "$REAL_SLOT_INIT" ] && command -v cargo >/dev/null 2>&1; then
	repo=$here/../../../..
	if (cd "$repo" && cargo build -q -p slot-init); then
		tdir=$(cd "$repo" && cargo metadata --no-deps --format-version 1 | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
		REAL_SLOT_INIT=$tdir/debug/slot-init
	fi
fi
export REAL_SLOT_INIT

mkdir -p "$T/bin"
cat > "$T/bin/slot-init" <<'STUB'
#!/bin/sh
case "$1" in
field | fields | layers | merge-layers | set-layer | boot-files | unpack-disk)
	[ -z "$REAL_SLOT_INIT" ] || {
		[ -z "${HELPER_LOG:-}" ] || echo "$1" >> "$HELPER_LOG"
		exec "$REAL_SLOT_INIT" "$@"
	}
	;;
esac
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
for l in m["layers"]: print(l["name"], l["id"], l.get("size", l.get("erofs_size")), l.get("sha256", l.get("erofs_sha256")), l["verity"]["root_hash"])' "$2" ;;
merge-layers) exec python3 -c 'import json,sys
m=json.load(open(sys.argv[1])); g=json.load(open(sys.argv[2]))
if any(l.get("verity") is None for l in g["layers"]): sys.exit(1)
m.pop("image",None); m.pop("verity",None); m["layers"]=g["layers"]; m["schema"]=2
print(json.dumps(m))' "$2" "$3" ;;
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
export UPDATE_SLOTS_STATE=$T/data/board/slots/state.json SLOTS_PARTUUID=abcd1234-02 SLOTS_RESERVE_KB=0 SLOTS_TRYING_HOLD_SEC=0
export UPDATE_REBOOT="echo tryboot >> $T/reboots" UPDATE_REBOOT_PLAIN="echo plain >> $T/reboots"
export BOARD_DEVICE_LOCK=$T/device-lock UPDATE_BOOT_ID_FILE=$T/boot_id
# A trial's checks (update's slot_sanity): modules for the running kernel,
# a bound USB gadget, no failed critical unit; no wait for the services.
export UPDATE_UNAME_R=7.2.9-test UPDATE_MODULES_ROOT=$T/sysroot UPDATE_GADGET_DIR=$T/gadget
export UPDATE_SYSTEMCTL=false
mkdir -p "$T/etc" "$T/run" "$T/boot/slots/pv-a" "$T/data/slots" "$T/data/board/slots"
mkdir -p "$T/sysroot/lib/modules/7.2.9-test" "$T/gadget/g1"
echo 1000480000.usb > "$T/gadget/g1/UDC"
# Housekeeping off until its own test (it would remove the slots below).
printf 'UPDATE_CONFIRM_DELAY=0\nUPDATE_LINK_DELAY=0\nSLOTS_KEEP=0\n' > "$T/etc/update.env"
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
# bf <id> <path>: a slot's boot file as kept on /data (slot-boot).
bf() { echo "$T/data/slots/bootfiles/$(awk -v p="$2" '$2 == p { print $1 }' "$T/data/slots/$1.boot")"; }
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
[ "$(SLOTS_BOOT_FREE_KB=1024 rc check "$T/pv-b.slot.tar")" = 0 ] || fail "check: an install needs no room on the boot FAT: $(cat "$T/out")"
snap > "$T/after"
cmp -s "$T/before" "$T/after" || fail "failed checks changed files: $(diff "$T/before" "$T/after")"

echo "a bundle installs as a slot on /data: the image, and its boot files (by content); nothing on the FAT"
bundle pv-b 2026.4.0 "PhotonVision Lab"
up stage "$T/pv-b.slot.tar" --sha256 "$(sha256sum "$T/pv-b.slot.tar" | cut -d' ' -f1)" 2>/dev/null
[ -f "$T/data/slots/pv-b.erofs" ] && [ -f "$T/data/slots/pv-b.json" ] || fail "image or manifest missing"
[ ! -e "$T/run/update/progress" ] || fail "the ring's progress file outlived the install: $(cat "$T/run/update/progress")"
grep -q "atlas.slot=pv-b atlas.roothash=cdcd" "$(bf pv-b cmdline.txt)" || fail "cmdline: $(cat "$T/data/slots/pv-b.boot")"
grep -q "atlas.data=PARTUUID=abcd1234-02" "$(bf pv-b cmdline.txt)" || fail "PARTUUID"
[ -f "$(bf pv-b overlays/bcm2712d0.dtbo)" ] || fail "overlays not kept"
[ -f "$(bf pv-b overlays/README)" ] || fail "no overlays/README: the firmware would ignore the slot's overlays"
cmp -s "$(bf pv-b slot.json)" "$T/data/slots/pv-b.json" || fail "the boot dir's slot.json"
(cd "$T/data/slots/bootfiles" && for f in *; do [ "$(sha256sum "$f" | cut -d' ' -f1)" = "$f" ] || fail "bootfiles/$f isn't its sha256"; done)
[ ! -e "$T/boot/slots/pv-b" ] || fail "an install put boot files on the FAT"
status | grep -q '"slot_staged":"pv-b"' || fail "staged: $(status)"
status | grep -q '"id":"pv-b"[^}]*"display_name":"PhotonVision Lab"' || fail "pv-b display_name: $(status)"
status | grep -q '"id":"pv-a"[^}]*"display_name":null' || fail "pv-a without display_name isn't null: $(status)"
ls "$T/data/slots" | grep -q install && fail "the work dir was left behind"
status | grep -q '"id":"pv-a"[^}]*"size":2,' || fail "pv-a's size isn't its file's: $(status)"

echo "a bundle on stdin installs the same, and its tar never stays on /data"
up stage - --sha256 "$(sha256sum "$T/pv-b.slot.tar" | cut -d' ' -f1)" < "$T/pv-b.slot.tar" 2>/dev/null || fail "stdin stage failed"
[ -z "$(ls "$T/data/board/update" 2>/dev/null)" ] || fail "the streamed tar was left: $(ls "$T/data/board/update")"
[ "$(rc stage - --sha256 0000 < "$T/pv-b.slot.tar")" = 3 ] || fail "stdin: wrong sha accepted"
[ -z "$(ls "$T/data/board/update" 2>/dev/null)" ] || fail "a refused streamed tar was left"

echo "an upload on the board (file:// in /data/updates) installs in place; nothing else is read"
mkdir -p "$T/data/updates/u1"
cp "$T/pv-b.slot.tar" "$T/data/updates/u1/pv-b.slot.tar"
sha=$(sha256sum "$T/pv-b.slot.tar" | cut -d' ' -f1)
size=$(wc -c < "$T/pv-b.slot.tar" | tr -d ' ')
up stage-url "file://$T/data/updates/u1/pv-b.slot.tar" --sha256 "$sha" --size "$size" 2>/dev/null || fail "the upload didn't install: $(cat "$T/err" 2>/dev/null)"
status | grep -q '"slot_staged":"pv-b"' || fail "upload staged: $(status)"
[ -f "$T/data/updates/u1/pv-b.slot.tar" ] || fail "the upload was removed (it is the uploader's)"
[ -z "$(ls "$T/data/board/update" 2>/dev/null)" ] || fail "an upload was copied to the download dir"
cp "$T/pv-b.slot.tar" "$T/elsewhere.slot.tar"
[ "$(rc stage-url "file://$T/elsewhere.slot.tar" --sha256 "$sha" --size "$size")" = 3 ] || fail "a file outside /data/updates was read"
grep -q "only an upload in" "$T/err" || fail "outside: $(cat "$T/err")"
[ "$(rc stage-url "file://$T/data/updates/../../elsewhere.slot.tar" --sha256 "$sha" --size "$size")" = 3 ] || fail "a .. path left /data/updates"
[ "$(rc stage-url "file://$T/data/updates/u1/pv-b.slot.tar" --sha256 "$(printf 0%.0s $(seq 64))" --size "$size")" = 3 ] || fail "an upload with the wrong sha256 installed"
[ "$(rc stage-url "file://$T/data/updates/u1/none.slot.tar" --sha256 "$sha" --size "$size")" = 3 ] || fail "a missing upload wasn't refused"

echo "a slots build's .img.xz installs as the slot it carries, the same as its bundle (stage-image)"
image_tools=yes
for tool in mkfs.vfat mcopy mke2fs debugfs sfdisk xz; do
	command -v "$tool" >/dev/null 2>&1 || image_tools=''
done
if [ -z "$REAL_SLOT_INIT" ] || [ -z "$image_tools" ]; then
	echo "  skipped (needs the real slot-init, mtools, e2fsprogs, sfdisk, xz)"
else
	# image <id> <out.img.xz>: the disk Gaia's slots.toml makes for the
	# bundle `bundle` makes: MBR, p1 FAT (default.txt, slots/<id>/ with the
	# slot's own cmdline), p2 ext4 (slots/<id>.erofs and .json).
	image() {
		bundle "$1" 2026.4.2
		b=$T/build/$1
		i=$T/img/$1
		rm -rf "$i"
		mkdir -p "$i/fat/slots/$1" "$i/data/slots"
		cp -r "$b/boot/." "$i/fat/slots/$1/"
		cp "$b/slot.json" "$i/fat/slots/$1/slot.json"
		printf '%s atlas.slot=%s atlas.roothash=cdcd atlas.data=PARTUUID=5056525a-02 panic=5\n' "$(cat "$b/boot/cmdline.txt")" "$1" > "$i/fat/slots/$1/cmdline.txt"
		echo "os_prefix=slots/$1/" > "$i/fat/default.txt"
		cp "$b/rootfs.erofs" "$i/data/slots/$1.erofs"
		cp "$b/slot.json" "$i/data/slots/$1.json"
		mkfs.vfat -C "$i/p1" 33792 > /dev/null
		mcopy -s -i "$i/p1" "$i/fat/"* ::/
		mke2fs -q -t ext4 -d "$i/data" "$i/p2" 4M
		truncate -s $(((1 + 33 + 4) * 1048576)) "$i/disk"
		printf 'start=2048, size=67584, type=c\nstart=69632, size=8192, type=83\n' | sfdisk -q "$i/disk"
		dd if="$i/p1" of="$i/disk" bs=1M seek=1 conv=notrunc status=none
		dd if="$i/p2" of="$i/disk" bs=1M seek=34 conv=notrunc status=none
		xz -T1 -c "$i/disk" > "$2"
	}
	# A loop mount without root: the files read out with mtools and debugfs.
	cat > "$T/bin/img-mount" <<'MOUNT'
#!/bin/sh
echo "$2 $(basename "$5")" >> "$IMG_MOUNTS"
case $2 in
vfat) mcopy -s -n -i "$5" '::/*' "$6/" ;;
ext4) debugfs -R "rdump /slots $6" "$5" > /dev/null 2>&1 ;;
esac
MOUNT
	printf '#!/bin/sh\necho "umount $1" >> "$IMG_MOUNTS"\n' > "$T/bin/img-umount"
	chmod +x "$T/bin/img-mount" "$T/bin/img-umount"
	export IMG_MOUNTS=$T/img-mounts
	stage_image() { SLOTS_MOUNT=$T/bin/img-mount SLOTS_UMOUNT=$T/bin/img-umount rc stage-image "$@"; }

	image pv-i "$T/pv-i.img.xz"
	sha=$(sha256sum "$T/pv-i.img.xz" | cut -d' ' -f1)
	[ "$(stage_image "$T/pv-i.img.xz" --sha256 "$sha")" = 0 ] || fail "stage-image: $(cat "$T/out" "$T/err")"
	cmp -s "$T/data/slots/pv-i.erofs" "$T/build/pv-i/rootfs.erofs" || fail "stage-image: the root isn't the image's"
	cmp -s "$T/data/slots/pv-i.json" "$T/build/pv-i/slot.json" || fail "stage-image: slot.json isn't the image's"
	grep -qx "console=ttyAMA10 atlas.slot=pv-i atlas.roothash=cdcd[cd]* atlas.data=PARTUUID=abcd1234-02 panic=5" "$(bf pv-i cmdline.txt)" ||
		fail "stage-image: the slot's cmdline is the image's, not this board's: $(cat "$(bf pv-i cmdline.txt)")"
	cmp -s "$(bf pv-i kernel8.img)" "$T/build/pv-i/boot/kernel8.img" || fail "stage-image: the kernel"
	[ -f "$(bf pv-i overlays/README)" ] || fail "stage-image: no overlays/README"
	status | grep -q '"slot_staged":"pv-i"' || fail "stage-image staged: $(status)"
	grep -qx "vfat p1.img" "$IMG_MOUNTS" && grep -qx "ext4 p2.img" "$IMG_MOUNTS" || fail "the partitions: $(cat "$IMG_MOUNTS")"
	[ "$(grep -c '^umount ' "$IMG_MOUNTS")" = 2 ] || fail "the partitions weren't let go: $(cat "$IMG_MOUNTS")"
	ls -a "$T/data/slots" | grep -q install && fail "stage-image left its work dir (the unpacked disk)"

	echo "  on stdin, hashed on the way through; a wrong --sha256 installs nothing"
	image pv-j "$T/pv-j.img.xz"
	sha=$(sha256sum "$T/pv-j.img.xz" | cut -d' ' -f1)
	[ "$(stage_image - --sha256 "$(printf 0%.0s $(seq 64))" < "$T/pv-j.img.xz")" = 3 ] || fail "stdin: a wrong sha256 installed"
	[ ! -e "$T/data/slots/pv-j.erofs" ] && [ ! -e "$T/boot/slots/pv-j" ] || fail "stdin: a refused image left a slot"
	ls -a "$T/data/slots" | grep -q install && fail "stdin: a refused image left its work dir"
	[ "$(stage_image - --sha256 "$sha" < "$T/pv-j.img.xz")" = 0 ] || fail "stage-image stdin: $(cat "$T/out" "$T/err")"
	cmp -s "$T/data/slots/pv-j.erofs" "$T/build/pv-j/rootfs.erofs" || fail "stdin: the root isn't the image's"

	echo "  an uploaded .img.xz (the controller gateway's one update image) installs through stage-url"
	image pv-u "$T/pv-u.img.xz"
	mkdir -p "$T/data/updates/u2"
	cp "$T/pv-u.img.xz" "$T/data/updates/u2/pv-u.img.xz"
	sha=$(sha256sum "$T/pv-u.img.xz" | cut -d' ' -f1)
	size=$(wc -c < "$T/pv-u.img.xz" | tr -d ' ')
	[ "$(SLOTS_MOUNT=$T/bin/img-mount SLOTS_UMOUNT=$T/bin/img-umount rc stage-url "file://$T/data/updates/u2/pv-u.img.xz" --sha256 "$sha" --size "$size")" = 0 ] ||
		fail "stage-url .img.xz: $(cat "$T/out" "$T/err")"
	cmp -s "$T/data/slots/pv-u.erofs" "$T/build/pv-u/rootfs.erofs" || fail "stage-url .img.xz: the root isn't the image's"
	[ -f "$T/data/updates/u2/pv-u.img.xz" ] || fail "stage-url .img.xz: the upload was removed (it is the uploader's)"

	echo "  a broken root, the running slot, or a disk that isn't a slots image: nothing installed"
	image pv-k "$T/pv-k.img.xz"
	echo tampered >> "$T/img/pv-k/data/slots/pv-k.erofs"
	rm -f "$T/img/pv-k/p2" && mke2fs -q -t ext4 -d "$T/img/pv-k/data" "$T/img/pv-k/p2" 4M
	dd if="$T/img/pv-k/p2" of="$T/img/pv-k/disk" bs=1M seek=34 conv=notrunc status=none
	xz -T1 -c "$T/img/pv-k/disk" > "$T/pv-k.img.xz"
	[ "$(stage_image "$T/pv-k.img.xz")" = 1 ] || fail "a tampered root installed"
	grep -q "rootfs.erofs isn't the size slot.json says\|doesn't match slot.json's sha256" "$T/err" || fail "tampered: $(cat "$T/err")"
	[ ! -e "$T/data/slots/pv-k.erofs" ] || fail "a tampered root was kept"
	image pv-a "$T/pv-a.img.xz"
	before=$(status | sed 's/"free_bytes":[0-9]*//; s/"updated":"[^"]*"//')
	[ "$(stage_image "$T/pv-a.img.xz")" = 3 ] || fail "the running slot's image installed"
	# Refused after the unpack: the state is as it was, not an interrupted install.
	status | grep -q "interrupted" && fail "a refusal left the install looking interrupted: $(status)"
	[ "$(status | sed 's/"free_bytes":[0-9]*//; s/"updated":"[^"]*"//')" = "$before" ] || fail "a refusal changed the state: $(status)"
	mkdir "$T/device-lock" && echo test > "$T/device-lock/owner"
	[ "$(stage_image "$T/pv-j.img.xz")" = 3 ] || fail "stage-image ran under the device lock"
	rm -rf "$T/device-lock"
	head -c 4194304 /dev/zero | xz -T1 -c > "$T/zero.img.xz"
	[ "$(stage_image "$T/zero.img.xz")" = 1 ] || fail "a disk without an MBR installed"
	grep -q "no MBR" "$T/err" || fail "no MBR: $(cat "$T/err")"
	ls -a "$T/data/slots" | grep -q install && fail "a failed stage-image left its work dir"
	up remove pv-i > /dev/null 2>&1 || true
	up remove pv-j > /dev/null 2>&1 || true
	# Back to what the tests below start from: pv-b staged.
	up stage "$T/pv-b.slot.tar" 2> /dev/null || fail "restaging pv-b"
fi

echo "a bundle without an overlay the board's config loads is refused (it would boot without that device)"
printf 'dtoverlay=bcm2712d0\ndtoverlay=ov9782,cam0,clk-continuous\n' > "$T/boot/raze-device.txt"
bundle pv-o 2026.4.9
[ "$(rc stage "$T/pv-o.slot.tar")" = 3 ] || fail "a bundle without ov9782.dtbo was installed"
grep -q "no overlay for ov9782" "$T/err" || fail "reason: $(cat "$T/err")"
[ "$(rc check "$T/pv-o.slot.tar")" = 3 ] && grep -q "no overlay for ov9782" "$T/out" || fail "check passed a bundle without ov9782: $(cat "$T/out")"
[ ! -e "$T/data/slots/pv-o.erofs" ] && [ ! -d "$T/boot/slots/pv-o" ] || fail "a refused bundle left files"
: > "$T/build/pv-o/boot/overlays/ov9782.dtbo"
tar -cf "$T/pv-o.slot.tar" -C "$T/build/pv-o" slot.json boot rootfs.erofs
[ "$(rc check "$T/pv-o.slot.tar")" = 0 ] || fail "a bundle with every overlay fails its check: $(cat "$T/out" "$T/err")"
rm "$T/boot/raze-device.txt"

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
echo "a full boot FAT doesn't stop an install, only a boot once (nothing changed)"
bundle pv-f 2026.4.3
[ "$(SLOTS_BOOT_FREE_KB=1024 rc stage "$T/pv-f.slot.tar")" = 0 ] || fail "an install needed room on the boot FAT: $(cat "$T/err")"
[ ! -e "$T/boot/slots/pv-f" ] || fail "an install put boot files on the FAT"
[ "$(SLOTS_BOOT_FREE_KB=1024 rc boot-once pv-f)" = 3 ] || fail "a full boot FAT wasn't refused"
grep -q 'no room on the boot partition for slot pv-f' "$T/err" || fail "the boot FAT's reason: $(cat "$T/err")"
[ ! -e "$T/boot/slots/pv-f" ] && [ ! -e "$T/boot/slots/.pv-f.new" ] || fail "a full boot FAT left pv-f's boot files"
[ "$(cat "$T/boot/trial.txt")" = "os_prefix=slots/pv-a/" ] || fail "a refused boot once wrote trial.txt"
up remove pv-f 2>/dev/null
[ ! -e "$T/data/slots/pv-f.boot" ] || fail "remove left pv-f's boot files on /data"
up stage "$T/pv-b.slot.tar" 2> /dev/null || fail "restaging pv-b"
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

echo "the OS's pre-reboot hook runs before boot once, and a failing one stops it with nothing changed"
mkdir -p "$T/etc/update.d"
printf '#!/bin/sh\necho "$BOARD_UPDATE_SLOT $BOARD_UPDATE_VERSION" >> "%s"\nexit 1\n' "$T/hook-calls" > "$T/etc/update.d/pre-reboot"
chmod +x "$T/etc/update.d/pre-reboot"
: > "$T/reboots"
[ "$(rc apply)" = 3 ] || fail "a failing pre-reboot hook didn't stop the restart"
grep -q "the pre-reboot hook stopped the restart" "$T/err" || fail "hook refusal: $(cat "$T/err")"
[ "$(cat "$T/hook-calls")" = "pv-b 2026.4.0" ] || fail "the hook's slot and version: $(cat "$T/hook-calls")"
[ ! -s "$T/reboots" ] || fail "restarted despite the hook"
[ "$(cat "$T/boot/trial.txt")" = "os_prefix=slots/pv-a/" ] || fail "trial.txt changed despite the hook"
status | grep -q '"slot_staged":"pv-b"' || fail "the hook's refusal unstaged pv-b: $(status)"
sed -i 's/^exit 1$/exit 0/' "$T/etc/update.d/pre-reboot"

echo "boot once puts the slot's boot files on the FAT (checked), writes trial.txt and restarts with tryboot"
: > "$T/flushes"
up apply 2>/dev/null
grep -q "atlas.slot=pv-b atlas.roothash=cdcd" "$T/boot/slots/pv-b/cmdline.txt" || fail "the trial's cmdline on the FAT"
[ -f "$T/boot/slots/pv-b/overlays/README" ] && [ -f "$T/boot/slots/pv-b/slot.json" ] || fail "the trial's boot dir: $(ls -R "$T/boot/slots/pv-b")"
grep -qx /dev/fake-p1 "$T/flushes" || fail "the FAT's device wasn't flushed after the slot's boot files"
[ "$(wc -l < "$T/hook-calls")" = 2 ] || fail "the pre-reboot hook didn't run before the restart"
[ "$(cat "$T/boot/trial.txt")" = "os_prefix=slots/pv-b/" ] || fail "trial.txt: $(cat "$T/boot/trial.txt")"
[ "$(tail -n 1 "$T/reboots")" = tryboot ] || fail "no tryboot restart"
[ ! -e "$T/boot/tryboot.req" ] || fail "tryboot.req on a board the firmware boots"
[ "$(cat "$T/boot/default.txt")" = "os_prefix=slots/pv-a/" ] || fail "the default changed before confirm"

echo "confirm on the trial makes it the default, with a root copy"
echo pv-b > "$T/data/board/slots/booted"
up confirm 2>/dev/null
[ "$(cat "$T/boot/default.txt")" = "os_prefix=slots/pv-b/" ] || fail "default.txt: $(cat "$T/boot/default.txt")"
grep -q "kernel of pv-b" "$T/boot/kernel8.img" || fail "root copy"
status | grep -q '"slot_default":"pv-b","slot_previous":"pv-a"' || fail "state: $(status)"
grep -q '"previous":"pv-a"' "$T/data/board/slots/state.json" || fail "state.json for the initramfs: $(cat "$T/data/board/slots/state.json")"
[ -r "$T/data/slots/pv-a.boot" ] || fail "pv-a's boot files (on the FAT only, from before) weren't kept on /data"
grep -q old-kernel "$(bf pv-a kernel8.img)" || fail "pv-a's kept kernel"
[ -d "$T/boot/slots/pv-a" ] || fail "the previous slot's boot files left the FAT (slot-init's fallback needs them)"

echo "a trial only (--keep-default) runs once it is healthy, but the default stays"
up boot-once pv-a --keep-default 2>/dev/null
[ "$(cat "$T/boot/trial.txt")" = "os_prefix=slots/pv-a/" ] || fail "trial.txt for the trial only: $(cat "$T/boot/trial.txt")"
echo pv-a > "$T/data/board/slots/booted"
up confirm 2>/dev/null
[ "$(cat "$T/boot/default.txt")" = "os_prefix=slots/pv-b/" ] || fail "a trial only changed default.txt: $(cat "$T/boot/default.txt")"
grep -q "kernel of pv-b" "$T/boot/kernel8.img" || fail "a trial only replaced the root copy"
status | grep -q '"state":"confirmed"' || fail "a healthy trial only: $(status)"
status | grep -q '"slot_default":"pv-b","slot_previous":"pv-a"' || fail "a trial only moved the default: $(status)"
[ ! -e "$T/data/board/slots/trial-keeps-default" ] || fail "the trial-only marker outlived its confirm"
# The next restart is the default again: an ordinary boot.
echo pv-b > "$T/data/board/slots/booted"
up confirm 2>/dev/null
status | grep -q '"slot_default":"pv-b"' || fail "back on the default: $(status)"

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
# Under a runner (QEMU: atlas.tryboot= on the command line) the trial
# restart is p1's tryboot.req as well.
echo "console=ttyAMA0 atlas.tryboot=0" > "$T/cmdline"
export BOARD_PROC_CMDLINE=$T/cmdline
up apply 2>/dev/null
unset BOARD_PROC_CMDLINE
[ -e "$T/boot/tryboot.req" ] || fail "a runner's boot once left no tryboot.req"
rm "$T/boot/tryboot.req"
printf '\000\000\000\001' > "$T/dt/chosen/bootloader/tryboot"
up confirm 2>/dev/null
status | grep -q "the firmware did the trial boot but started pv-b" || fail "a tryboot that started another slot: $(status)"
grep -q '^GOOD=no' "$T/data/board/slots/pv-c.meta" || fail "pv-c marked bad after a real trial"
rm -rf "$T/dt"
for flag in 0 1; do
	echo "console=ttyAMA0 atlas.tryboot=$flag" > "$T/cmdline"
	[ "$(. "$lib/lib.sh" && BOARD_PROC_CMDLINE=$T/cmdline board_tryboot)" = $flag ] || fail "a runner's atlas.tryboot=$flag isn't the tryboot flag"
done
[ -z "$(. "$lib/lib.sh" && BOARD_PROC_CMDLINE=$T/cmdline-none board_tryboot)" ] || fail "a tryboot flag with neither the firmware's nor a runner's"

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

echo "a trial that fails its health check restarts back, and the default's boot keeps that reason"
up apply 2>/dev/null || true
up stage "$T/pv-c.slot.tar" 2>/dev/null || true
up apply 2>/dev/null
booted=$(cat "$T/data/board/slots/booted")
mkdir -p "$T/etc"
printf '#!/bin/sh\necho unhealthy >&2\nexit 1\n' > "$T/etc/update-health"
chmod +x "$T/etc/update-health"
echo pv-c > "$T/data/board/slots/booted"
echo pv-c > "$T/data/board/slots/trial-started"
: > "$T/reboots"
# A failing pre-reboot hook doesn't keep a bad trial running.
sed -i 's/^exit 0$/exit 1/' "$T/etc/update.d/pre-reboot"
up confirm 2>/dev/null || true
grep -qx plain "$T/reboots" || fail "an unhealthy trial didn't restart back: $(cat "$T/reboots")"
rm -f "$T/etc/update.d/pre-reboot"
grep -q '^GOOD=no' "$T/data/board/slots/pv-c.meta" || fail "the unhealthy trial isn't marked bad"
status | grep -q "slot pv-c failed its health check" || fail "the trial's reason: $(status)"
rm -f "$T/etc/update-health"
echo "$booted" > "$T/data/board/slots/booted"
up confirm 2>/dev/null
status | grep -q '"state":"rolled-back"' || fail "not rolled back: $(status)"
status | grep -q "slot pv-c failed its health check; back on $booted" || fail "the default's boot lost the reason: $(status)"

echo "a trial without the USB gadget fails its checks, restarts back, and keeps that reason"
up apply 2>/dev/null || true
up stage "$T/pv-c.slot.tar" 2>/dev/null || true
up apply 2>/dev/null
booted=$(cat "$T/data/board/slots/booted")
echo pv-c > "$T/data/board/slots/booted"
echo pv-c > "$T/data/board/slots/trial-started"
: > "$T/gadget/g1/UDC"
: > "$T/reboots"
up confirm 2>/dev/null || true
echo 1000480000.usb > "$T/gadget/g1/UDC"
grep -qx plain "$T/reboots" || fail "a trial without its gadget didn't restart back: $(cat "$T/reboots")"
grep -q '^GOOD=no' "$T/data/board/slots/pv-c.meta" || fail "the trial without its gadget isn't marked bad"
status | grep -q "slot pv-c failed its checks: the USB gadget (the board's management link) isn't up" ||
	fail "the trial's reason: $(status)"
echo "$booted" > "$T/data/board/slots/booted"
up confirm 2>/dev/null
status | grep -q '"state":"rolled-back"' || fail "not rolled back: $(status)"
status | grep -q "failed its checks: .*; back on $booted" || fail "the default's boot lost the reason: $(status)"

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

echo "rollback makes the previous slot the default, and reports rolled-back once the board is on it"
up rollback --no-reboot 2>/dev/null
[ "$(cat "$T/boot/default.txt")" = "os_prefix=slots/pv-a/" ] || fail "rollback default.txt"
status | grep -q '"slot_default":"pv-a","slot_previous":"pv-b"' || fail "rollback state: $(status)"
status | grep -q '"state":"rebooting"' || fail "rollback before the restart: $(status)"
status | grep -q '"error":null' || fail "rollback kept an old error: $(status)"
echo pv-a > "$T/data/board/slots/booted"
up confirm 2>/dev/null
status | grep -q '"state":"rolled-back"' || fail "rollback after the restart: $(status)"
status | grep -q '"slot_active":"pv-a"' || fail "rollback's slot: $(status)"

echo "reap: only boot files on the FAT whose slot isn't on /data go; never a slot, and confirm reaps nothing"
mkdir -p "$T/boot/slots/ghost" "$T/boot/slots/ghost2"
echo k > "$T/boot/slots/ghost/kernel8.img"
before=$(snap)
dry=$(up reap --dry-run)
[ "$(printf '%s' "$dry" | python3 -c 'import json,sys; print(" ".join(sorted(json.load(sys.stdin)["remove"])))')" = "ghost ghost2" ] ||
	fail "reap would remove: $dry"
[ "$(snap)" = "$before" ] || fail "a dry run changed files"
slots_before=$(ls "$T/data/slots" | sort)
up reap 2>/dev/null
[ ! -d "$T/boot/slots/ghost" ] && [ ! -d "$T/boot/slots/ghost2" ] || fail "the stray boot dirs stayed"
[ "$(ls "$T/data/slots" | sort)" = "$slots_before" ] || fail "reap touched /data's slots"
for kept in pv-a pv-b; do [ -d "$T/boot/slots/$kept" ] || fail "reap removed $kept's boot files"; done
grep -rq 'slot.reaped' "$T/data/board" || fail "no slot.reaped event"
[ "$(up reap --dry-run)" = '{"remove":[]}' ] || fail "nothing left to reap: $(up reap --dry-run)"
! grep -q 'cmd_reap' "$(sed -n '/^cmd_confirm() {/,/^}/p' "$lib/slots" > "$T/confirm.sh"; echo "$T/confirm.sh")" || fail "confirm reaps"

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
grep -q "atlas.slot=hx-a atlas.roothash=$(rh base),$(rh platform),$(rh app) " "$(bf hx-a cmdline.txt)" || fail "cmdline: $(cat "$(bf hx-a cmdline.txt)")"
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

echo "a bundle as the build makes it (Gaia's layers/layers.json and layers/<name>.erofs) checks and installs the same"
g=$T/build/gx-a
rm -rf "$g"
mkdir -p "$g/boot" "$g/layers"
echo "kernel of gx-a" > "$g/boot/kernel8.img"
echo "console=ttyAMA10" > "$g/boot/cmdline.txt"
recs=''
for n in base platform app; do
	printf 'gaia %s\n' "$n" > "$g/layers/$n.erofs"
	gid=$(sha256sum "$g/layers/$n.erofs" | cut -d' ' -f1)
	gsz=$(wc -c < "$g/layers/$n.erofs" | tr -d ' ')
	recs="$recs${recs:+,}{\"name\":\"$n\",\"id\":\"$gid\",\"tar\":\"layers/$n.tar\",\"tar_sha256\":\"$gid\",\"tar_size\":1,\"erofs\":\"layers/$n.erofs\",\"erofs_sha256\":\"$gid\",\"erofs_size\":$gsz,\"inputs\":\"x\",\"verity\":{\"root_hash\":\"$(rh "g$n")\",\"salt\":\"00\",\"hash_offset\":4096,\"data_blocks\":1,\"data_block_size\":4096,\"hash_block_size\":4096}}"
done
echo "{\"order\":[\"base\",\"platform\",\"app\"],\"layers\":[$recs]}" > "$g/layers/layers.json"
printf '{"schema":2,"id":"gx-a","family":"helios","version":"2026.2.0","variant":"styx","kernel":"7.2.9","boot":{"files":{"kernel8.img":"%s","cmdline.txt":"%s"}}}\n' \
	"$(sha256sum "$g/boot/kernel8.img" | cut -d' ' -f1)" "$(sha256sum "$g/boot/cmdline.txt" | cut -d' ' -f1)" > "$g/slot.json"
tar -cf "$T/gx-a.slot.tar" -C "$g" slot.json boot layers
[ "$(rc check "$T/gx-a.slot.tar")" = 0 ] || fail "check a Gaia bundle: $(cat "$T/out" "$T/err")"
grep -q '"layers_reused":0' "$T/out" || fail "check: $(cat "$T/out")"
up stage "$T/gx-a.slot.tar" 2>/dev/null || fail "stage a Gaia bundle: $(cat "$T/err")"
for n in base platform app; do
	[ -f "$T/data/slots/layers/$(printf 'gaia %s\n' "$n" | sha256sum | cut -d' ' -f1).erofs" ] || fail "layer $n not stored by id: $(ls "$T/data/slots/layers")"
done
grep -q '"layers"' "$T/data/slots/gx-a.json" || fail "the installed manifest has no layers: $(cat "$T/data/slots/gx-a.json")"
grep -q "atlas.roothash=$(rh gbase),$(rh gplatform),$(rh gapp) " "$(bf gx-a cmdline.txt)" || fail "cmdline: $(cat "$(bf gx-a cmdline.txt)")"
up remove gx-a 2>/dev/null || fail "remove gx-a"
sed -i 's/"verity":{[^}]*}/"verity":null/' "$g/layers/layers.json"
tar -cf "$T/gx-b.slot.tar" -C "$g" slot.json boot layers
[ "$(rc stage "$T/gx-b.slot.tar")" = 1 ] || fail "a bundle without verity was installed"

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
# Running, so its boot files are on the FAT (a booted slot's always are).
while read -r h f; do
	mkdir -p "$T/boot/slots/hx-p/$(dirname "$f")"
	cp "$T/data/slots/bootfiles/$h" "$T/boot/slots/hx-p/$f"
done < "$T/data/slots/hx-p.boot"
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
grep -q "atlas.roothash=$(rh base),$(rh platform),$(rh push-v2) " "$(bf hx-p cmdline.txt)" || fail "the kept copy of the cmdline missed the push"
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

echo "boot files on demand: the FAT holds the resident slots' only, the rest is staged when a slot is tried or made the default"
# A board of its own: q-0 booted from the image (its boot files on the FAT only).
H=$T/h
mkdir -p "$H/boot/slots/q-0" "$H/data/slots" "$H/data/board/slots" "$H/run"
export SLOTS_DATA=$H/data SLOTS_BOOT_MOUNT=$H/boot BOARD_RUN_DIR=$H/run BOARD_DATA_DIR=$H/data/board UPDATE_SLOTS_STATE=$H/data/board/slots/state.json
echo q0-kernel > "$H/boot/slots/q-0/kernel8.img"
echo 'console=ttyAMA10 atlas.slot=q-0' > "$H/boot/slots/q-0/cmdline.txt"
echo '{"schema":1,"id":"q-0","family":"helios","version":"0","variant":"styx","kernel":"7.2.9","verity":{"root_hash":"aa"}}' > "$H/data/slots/q-0.json"
cp "$H/data/slots/q-0.json" "$H/boot/slots/q-0/slot.json"
echo q0 > "$H/data/slots/q-0.erofs"
echo 'os_prefix=slots/q-0/' > "$H/boot/default.txt"
echo '{"schema":1,"default":"q-0","previous":null,"trial":null}' > "$H/data/board/slots/state.json"
hb() { echo "$H/data/slots/bootfiles/$(awk -v p="$2" '$2 == p { print $1 }' "$H/data/slots/$1.boot")"; }
fat() { (cd "$H/boot/slots" && ls -A | tr '\n' ' ' | sed 's/ $//'); }
boot_as() { echo "$1" > "$H/data/board/slots/booted"; up confirm 2>/dev/null; }
update_to() { # update_to <id> [stage flags]: stage, try, boot it, confirm
	id=$1
	shift
	bundle "$id" "1.$id"
	up stage "$T/$id.slot.tar" "$@" 2>/dev/null || fail "stage $id: $(rc stage "$T/$id.slot.tar"; cat "$T/err")"
	up apply 2>/dev/null || fail "apply $id"
	boot_as "$id"
}
boot_as q-0
[ -r "$H/data/slots/q-0.boot" ] || fail "the image's slot wasn't copied to /data on its first confirm"
grep -qx q0-kernel "$(hb q-0 kernel8.img)" || fail "q-0's kept kernel"
update_to q-1
[ "$(fat)" = "q-0 q-1" ] || fail "FAT after q-1: $(fat)"
update_to q-2
[ "$(fat)" = "q-1 q-2" ] || fail "the old slot stayed on the FAT (only running/default/previous/trial): $(fat)"
[ -r "$H/data/slots/q-0.json" ] && [ -r "$H/data/slots/q-0.boot" ] || fail "q-0 went from /data with its boot files"
# One grep over the store (a loop's non-matching grep would end the
# command substitution under dash's set -e).
[ "$(grep -lx dtbo "$H"/data/slots/bootfiles/* | wc -l | tr -d ' ')" = 1 ] ||
	fail "the slots' shared overlay is kept more than once"

echo "  set-default to a slot that is only on /data puts it back on the FAT first; rollback the same"
up set-default q-0 2>/dev/null || fail "set-default q-0: $(rc set-default q-0; cat "$T/err")"
[ "$(cat "$H/boot/default.txt")" = "os_prefix=slots/q-0/" ] || fail "default.txt: $(cat "$H/boot/default.txt")"
grep -qx q0-kernel "$H/boot/slots/q-0/kernel8.img" || fail "q-0's boot files on the FAT"
grep -qx q0-kernel "$H/boot/kernel8.img" || fail "the root copy"
[ "$(fat)" = "q-0 q-2" ] || fail "FAT after set-default (q-1 neither default nor previous): $(fat)"
up rollback --no-reboot 2>/dev/null
[ "$(cat "$H/boot/default.txt")" = "os_prefix=slots/q-2/" ] || fail "rollback default.txt"
boot_as q-2
[ "$(fat)" = "q-0 q-2" ] || fail "FAT after rollback: $(fat)"

echo "  a boot file that doesn't read back as kept stops boot once; nothing on the FAT, trial.txt as it was"
cp "$(hb q-1 kernel8.img)" "$T/q1-kernel"
echo bad > "$(hb q-1 kernel8.img)"
trial_before=$(cat "$H/boot/trial.txt" 2>/dev/null || true)
[ "$(rc boot-once q-1)" = 1 ] || fail "a damaged boot file was booted: $(cat "$T/err")"
grep -q "didn't read back" "$T/err" || fail "the reason: $(cat "$T/err")"
[ ! -e "$H/boot/slots/q-1" ] && [ ! -e "$H/boot/slots/.q-1.new" ] || fail "a damaged slot's dir on the FAT: $(fat)"
[ "$(cat "$H/boot/trial.txt" 2>/dev/null || true)" = "$trial_before" ] || fail "trial.txt changed"
cp "$T/q1-kernel" "$(hb q-1 kernel8.img)"

echo "  a half-written dir (power lost while staging) goes on the next boot"
mkdir -p "$H/boot/slots/.q-9.new"
boot_as q-2
[ ! -e "$H/boot/slots/.q-9.new" ] || fail "the half-written dir stayed: $(fat)"

echo "housekeeping: keep the last n (pinned ones never go, nor count), --keep-old skips it once"
[ "$(up keep)" = '{"keep":0}' ] || fail "keep from update.env: $(up keep)"
[ "$(up keep 2)" = '{"keep":2}' ] || fail "keep 2"
[ "$(rc keep two)" = 3 ] || fail "keep took a non-number"
up pin q-1
update_to q-3
for gone in q-0; do
	[ ! -e "$H/data/slots/$gone.json" ] && [ ! -e "$H/data/slots/$gone.erofs" ] && [ ! -e "$H/data/slots/$gone.boot" ] && [ ! -e "$H/boot/slots/$gone" ] ||
		fail "housekeeping left $gone"
done
for kept in q-1 q-2 q-3; do [ -r "$H/data/slots/$kept.json" ] || fail "housekeeping removed $kept"; done
grep -q '"kind":"slot.housekept"' "$H/data/board/events.jsonl" || fail "no slot.housekept event"
update_to q-4 --keep-old
for kept in q-1 q-2 q-3 q-4; do [ -r "$H/data/slots/$kept.json" ] || fail "--keep-old: $kept removed"; done
[ ! -e "$H/data/board/slots/keep-old" ] || fail "--keep-old outlived its update"
update_to q-5
for gone in q-2 q-3; do [ ! -e "$H/data/slots/$gone.json" ] || fail "housekeeping kept $gone"; done
for kept in q-1 q-4 q-5; do [ -r "$H/data/slots/$kept.json" ] || fail "housekeeping removed $kept"; done
for f in "$H/data/slots/bootfiles"/*; do
	grep -q "^${f##*/} " "$H/data/slots"/*.boot || fail "a boot file no slot lists stayed: ${f##*/}"
done

echo "the previous slot can be removed: rollback then has none, and state.json says so"
up remove q-4 2>/dev/null || fail "remove the previous slot: $(rc remove q-4; cat "$T/err")"
grep -q '"previous":null' "$H/data/board/slots/state.json" || fail "state.json: $(cat "$H/data/board/slots/state.json")"
[ ! -e "$H/boot/slots/q-4" ] && [ ! -e "$H/data/slots/q-4.boot" ] || fail "q-4 left boot files"
[ "$(rc rollback --no-reboot)" = 3 ] || fail "rollback without a previous slot"
up keep 0 > /dev/null
update_to q-6
for kept in q-1 q-5 q-6; do [ -r "$H/data/slots/$kept.json" ] || fail "keep 0 removed $kept"; done

echo "the failure memory: a stalled boot is a failure (refused unless forced, cleared by a confirm); a power cut never is"
F=$T/f
mkdir -p "$F/boot/slots/f-0" "$F/data/slots" "$F/data/board/slots" "$F/run"
export SLOTS_DATA=$F/data SLOTS_BOOT_MOUNT=$F/boot BOARD_RUN_DIR=$F/run BOARD_DATA_DIR=$F/data/board UPDATE_SLOTS_STATE=$F/data/board/slots/state.json
echo f0-kernel > "$F/boot/slots/f-0/kernel8.img"
echo 'console=ttyAMA10 atlas.slot=f-0' > "$F/boot/slots/f-0/cmdline.txt"
echo '{"schema":1,"id":"f-0","family":"helios","version":"0","variant":"styx","kernel":"7.2.9","verity":{"root_hash":"aa"}}' > "$F/data/slots/f-0.json"
cp "$F/data/slots/f-0.json" "$F/boot/slots/f-0/slot.json"
echo f0 > "$F/data/slots/f-0.erofs"
echo 'os_prefix=slots/f-0/' > "$F/boot/default.txt"
echo '{"schema":1,"default":"f-0","previous":null,"trial":null}' > "$F/data/board/slots/state.json"
fmeta() { sed -n "s/^$2=//p" "$F/data/board/slots/$1.meta" 2>/dev/null | head -n 1; }
# fboot <boot id> <running slot>: a boot that runs <slot> and runs its confirm
fboot() { echo "$1" > "$T/boot_id"; echo "$2" > "$F/data/board/slots/booted"; up confirm 2>/dev/null; }
# stall <boot id> <running slot>: a boot that never confirmed, its deadline fired
stall() { echo "$1" > "$T/boot_id"; echo "$2" > "$F/data/board/slots/booted"; SLOTS_UPTIME_FILE=$T/uptime-late up deadline 2>/dev/null || true; }
echo 400.0 > "$T/uptime-late"
fboot b-f0 f-0
[ -r "$F/data/slots/f-0.boot" ] || fail "f-0's boot files weren't kept on /data"
up keep 3 > /dev/null
bundle f-1 1.1
up stage "$T/f-1.slot.tar" 2>/dev/null || fail "stage f-1: $(cat "$T/err")"
up apply 2>/dev/null || fail "apply f-1: $(cat "$T/err")"
fboot b-f1 f-1
status | grep -q '"slot_default":"f-1","slot_previous":"f-0"' || fail "f-1 is the default: $(status)"
status | grep -q '"id":"f-0"[^}]*"failed":false,"last_failure":null' || fail "a healthy slot's failure fields: $(status)"
status | grep -q '"id":"f-1"[^}]*"last_confirm":{"boot":"b-f1"' || fail "the last confirm: $(status)"

echo "  a deadline that fires on a boot that never confirmed records the failure (once per boot), with its reason"
: > "$T/reboots"
stall b-f0s f-0
[ "$(fmeta f-0 FAILED)" = yes ] || fail "a stalled boot isn't a failure: $(cat "$F/data/board/slots/f-0.meta")"
[ "$(fmeta f-0 FAILURES)" = 1 ] || fail "the count: $(fmeta f-0 FAILURES)"
[ "$(fmeta f-0 FAIL_BOOT)" = b-f0s ] || fail "the failure's boot: $(fmeta f-0 FAIL_BOOT)"
fmeta f-0 FAIL_REASON | grep -q "hasn't confirmed its boot within 300 s" || fail "the reason: $(fmeta f-0 FAIL_REASON)"
grep -q '"kind":"slot.failed"' "$F/data/board/events.jsonl" || fail "no slot.failed event"
status | grep -q '"id":"f-0"[^}]*"failed":true' || fail "status doesn't say f-0 failed: $(status)"
status | grep -q '"last_failure":{"reason":"slot f-0 hasn'"'"'t confirmed' || fail "status has no last failure: $(status)"
stall b-f0s f-0
[ "$(fmeta f-0 FAILURES)" = 1 ] || fail "the same boot counted twice"
[ ! -s "$T/reboots" ] || fail "a stalled default restarted the board: $(cat "$T/reboots")"
echo f-1 > "$F/data/board/slots/booted"

echo "  a failed slot is refused by rollback, boot once and set-default unless forced"
[ "$(rc rollback --no-reboot)" = 3 ] || fail "rollback to a failed previous slot"
grep -q "failed since its last confirmation" "$T/err" || fail "the refusal: $(cat "$T/err")"
grep -q -- "--force" "$T/err" || fail "the refusal doesn't say --force: $(cat "$T/err")"
[ "$(cat "$F/boot/default.txt")" = "os_prefix=slots/f-1/" ] || fail "a refused rollback changed the default"
[ "$(rc boot-once f-0)" = 3 ] || fail "boot once of a failed slot"
grep -q "failed since its last confirmation" "$T/err" || fail "boot-once refusal: $(cat "$T/err")"
[ "$(rc set-default f-0)" = 3 ] || fail "set-default of a failed slot"
grep -q "failed since its last confirmation" "$T/err" || fail "set-default refusal: $(cat "$T/err")"
[ "$(rc set-default f-0 --force)" = 0 ] || fail "a forced set-default refused: $(cat "$T/err")"
[ "$(cat "$F/boot/default.txt")" = "os_prefix=slots/f-0/" ] || fail "the forced default: $(cat "$F/boot/default.txt")"
[ "$(rc boot-once f-0 --force)" = 0 ] || fail "a forced boot once refused: $(cat "$T/err")"
[ "$(cat "$F/boot/trial.txt")" = "os_prefix=slots/f-0/" ] || fail "trial.txt for the forced boot"
# The trial comes up and confirms: its failure clears (its count stays).
fboot b-f0t f-0
[ "$(fmeta f-0 FAILED)" = no ] || fail "a confirm didn't clear the failure: $(cat "$F/data/board/slots/f-0.meta")"
[ "$(fmeta f-0 FAILURES)" = 1 ] || fail "a confirm erased the count"
[ "$(fmeta f-0 CONFIRM_BOOT)" = b-f0t ] || fail "the confirm's boot"
status | grep -q '"id":"f-0"[^}]*"failed":false' || fail "status after the confirm: $(status)"
[ "$(rc boot-once f-1)" = 0 ] || fail "f-1 is no longer eligible: $(cat "$T/err")"
fboot b-f1b f-1
status | grep -q '"slot_default":"f-1"' || fail "f-1 is the default again: $(status)"

echo "  a power cut is never a failure: a trial that never confirmed after power-on keeps its record and its eligibility"
bundle f-2 1.2
up stage "$T/f-2.slot.tar" 2>/dev/null || fail "stage f-2: $(cat "$T/err")"
up apply f-2 2>/dev/null || fail "apply f-2: $(cat "$T/err")"
echo f-2 > "$F/data/board/slots/trial-started"
mkdir -p "$T/dt/chosen/bootloader"
printf '\000\000\020\000' > "$T/dt/chosen/bootloader/rsts" # PM_RSTS 0x1000: a power-on
fboot b-f2cut f-1
status | grep -q "began at power-on (the board lost power)" || fail "the power-on wasn't said: $(status)"
grep -q '^FAILED=yes' "$F/data/board/slots/f-2.meta" && fail "a power cut failed the trial: $(cat "$F/data/board/slots/f-2.meta")"
[ "$(fmeta f-2 GOOD)" = untried ] || fail "a power cut changed the trial's GOOD: $(fmeta f-2 GOOD)"
status | grep -q '"id":"f-2"[^}]*"failed":false,"last_failure":null' || fail "a power cut's failure fields: $(status)"
[ "$(rc boot-once f-2)" = 0 ] || fail "a power cut made the slot ineligible: $(cat "$T/err")"
rm -rf "$T/dt"
# The same trial, cut short without a power cut: its initramfs started it and it never confirmed.
echo f-2 > "$F/data/board/slots/trial-started"
fboot b-f2s f-1
grep -q '^FAILED=yes' "$F/data/board/slots/f-2.meta" || fail "a trial that started and never confirmed isn't a failure: $(cat "$F/data/board/slots/f-2.meta")"
fmeta f-2 FAIL_REASON | grep -q "started but its system didn't confirm" || fail "the trial's failure reason: $(fmeta f-2 FAIL_REASON)"

echo "  housekeeping: a failed slot doesn't use a keep place, and a recent one isn't removed"
bundle f-3 1.3
up stage "$T/f-3.slot.tar" 2>/dev/null || fail "stage f-3: $(cat "$T/err")"
up apply f-3 2>/dev/null || fail "apply f-3: $(cat "$T/err")"
fboot b-f3 f-3
[ -r "$F/data/slots/f-2.json" ] || fail "housekeeping removed the recent failed f-2"
[ -r "$F/data/slots/f-1.json" ] && [ -r "$F/data/slots/f-0.json" ] || fail "housekeeping removed an eligible slot"
grep -q '"default":"f-3"' "$F/data/board/slots/state.json" || fail "state after housekeeping: $(cat "$F/data/board/slots/state.json")"

echo "  a trial whose deadline fires goes back to the default, and stays failed"
bundle f-4 1.4
up stage "$T/f-4.slot.tar" 2>/dev/null || fail "stage f-4: $(cat "$T/err")"
up apply f-4 2>/dev/null || fail "apply f-4: $(cat "$T/err")"
echo b-f4 > "$T/boot_id"
echo f-4 > "$F/data/board/slots/booted"
: > "$T/reboots"
SLOTS_UPTIME_FILE=$T/uptime-late up deadline 2>/dev/null || true
[ "$(tail -n 1 "$T/reboots")" = plain ] || fail "a missed deadline on a trial didn't restart back: $(cat "$T/reboots")"
[ "$(fmeta f-4 FAILED)" = yes ] || fail "the trial's failure: $(cat "$F/data/board/slots/f-4.meta")"
[ "$(fmeta f-4 GOOD)" = no ] || fail "the trial isn't GOOD=no"
status | grep -q 'missed its confirm deadline' || fail "the deadline's reason: $(status)"
fboot b-f4back f-3
status | grep -q '"state":"rolled-back"' || fail "a missed trial's rollback: $(status)"
status | grep -q '"slot_default":"f-3"' || fail "the default moved: $(status)"
[ "$(fmeta f-4 FAILURES)" = 1 ] || fail "the trial's failure counted twice: $(fmeta f-4 FAILURES)"
[ "$(rc boot-once f-4)" = 3 ] || fail "a failed trial is bootable without force"
[ "$(rc boot-once f-4 --force)" = 0 ] || fail "a failed trial isn't bootable by force: $(cat "$T/err")"

echo "  corrupt meta is no known failure; the writer's status says so per slot"
printf 'GARBAGE\n\000x=\n' > "$F/data/board/slots/f-1.meta"
status | grep -q '"id":"f-1"[^}]*"failures":0,"failed":false,"last_failure":null,"last_confirm":null' || fail "corrupt meta isn't no failure: $(status)"

echo "  a failed slot still goes once it is older than the last n"
for g in g-1 g-2 g-3; do
	bundle "$g" 2.0
	up stage "$T/$g.slot.tar" 2>/dev/null || fail "stage $g: $(cat "$T/err")"
	up apply "$g" 2>/dev/null || fail "apply $g: $(cat "$T/err")"
	fboot "b-$g" "$g"
done
[ ! -e "$F/data/slots/f-2.json" ] || fail "the old failed f-2 was kept for ever"

# Back to the board the sections below expect.
export SLOTS_DATA=$T/data SLOTS_BOOT_MOUNT=$T/boot BOARD_RUN_DIR=$T/run BOARD_DATA_DIR=$T/data/board UPDATE_SLOTS_STATE=$T/data/board/slots/state.json

echo "ok"
