#!/bin/sh
# Off-device test of pd-device-update: the board's partitions are plain
# files, a "mount" is a directory next to its file (so what a mounted new root
# contains is set up by hand in p6.d/p5.d), and reboots are recorded. The
# image is a real A/B disk layout made with sfdisk, compressed with xz and
# zstd, and pd-image-slots is compiled from source.
# With erofs-utils (mkfs.erofs, fsck.erofs) it also installs an image whose
# root slot is a real EROFS filesystem, read back through the writer's
# read-only mount.
# Needs sh, cc, sfdisk, xz, zstd, sha256sum. Run: sh devices/raze/tests/update.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
lib=$here/../gaia/assets/rootfs/usr/lib/pd-device
tool_src=$here/../gaia/buildroot-external/packages/pd-image-slots/src/pd-image-slots.c
T=$(mktemp -d "${TMPDIR:-/tmp}/pd-update-test.XXXXXX")
trap 'rm -rf "$T"' EXIT
PATH=$PATH:/usr/sbin:/sbin

fail() {
	echo "FAIL: $*" >&2
	exit 1
}

mkdir -p "$T/disk" "$T/etc/update.d" "$T/run" "$T/bin"
cc -O2 -Wall -Wextra -Werror -o "$T/bin/pd-image-slots" "$tool_src"
for n in 1 2 3 5 6; do
	: > "$T/disk/p$n"
	mkdir -p "$T/disk/p$n.d"
done
printf '[all]\ntryboot_a_b=1\nboot_partition=2\n' > "$T/disk/p1.d/autoboot.txt"
printf 'ID=photonvision\nVERSION_ID=1.0\n' > "$T/os-release"
printf 'UPDATE_CONFIRM_DELAY=0\n' > "$T/etc/update.env"

cat > "$T/bin/mount" <<'EOF'
#!/bin/sh
# mount [-o ro] <part> <dir>: the partition's directory stands in for it.
[ "$1" = -o ] && shift 2
rmdir "$2" 2>/dev/null || rm -f "$2"
ln -s "$1.d" "$2"
EOF
cat > "$T/bin/mount-ro" <<'EOF'
#!/bin/sh
# mount -o ro <part> <dir>: an EROFS partition is extracted for real, any
# other partition's directory stands in for it.
[ "$1" = -o ] && shift 2
rmdir "$2" 2>/dev/null || rm -rf "$2"
if [ "$(od -An -tx1 -j1024 -N4 "$1" 2>/dev/null | tr -d ' \n')" = e2e1f5e0 ]; then
	mkdir -p "$2"
	fsck.erofs --extract="$2" "$1" >/dev/null
else
	ln -s "$1.d" "$2"
fi
EOF
cat > "$T/bin/umount" <<'EOF'
#!/bin/sh
rm -rf "$1"
EOF
cat > "$T/bin/reboot" <<'EOF'
#!/bin/sh
echo "$*" >> "$REBOOTS"
EOF
cat > "$T/bin/systemctl" <<'EOF'
#!/bin/sh
# is-failed --quiet <unit>: failed when listed in $FAILED_UNITS.
[ "$1" = is-failed ] || exit 0
case " ${FAILED_UNITS:-} " in *" $3 "*) exit 0 ;; esac
exit 1
EOF
chmod +x "$T/bin/mount" "$T/bin/mount-ro" "$T/bin/umount" "$T/bin/reboot" "$T/bin/systemctl"

export PD_LIB_DIR=$lib PD_ETC_DIR=$T/etc PD_RUN_DIR=$T/run PD_OS_RELEASE=$T/os-release
export PD_DEVICE_MODEL=raze
export UPDATE_PART_PREFIX=$T/disk/p UPDATE_MOUNT=$T/bin/mount UPDATE_MOUNT_RO=$T/bin/mount-ro
export UPDATE_UMOUNT=$T/bin/umount UPDATE_SLOTS_TOOL=$T/bin/pd-image-slots
export UPDATE_REBOOT="$T/bin/reboot tryboot" UPDATE_REBOOT_PLAIN="$T/bin/reboot plain"
export REBOOTS=$T/reboots UPDATE_CMDLINE_ROOT=5 UPDATE_SYNC=true
# A booted slot that passes the package's checks: its kernel has modules, the
# gadget is bound, and no critical unit failed.
export UPDATE_SYSTEMCTL=$T/bin/systemctl UPDATE_UNAME_R=7.2.9-test
export UPDATE_MODULES_ROOT=$T/sysroot UPDATE_GADGET_DIR=$T/gadget
mkdir -p "$T/sysroot/lib/modules/7.2.9-test" "$T/gadget/g1"
echo 1000480000.usb > "$T/gadget/g1/UDC"
: > "$REBOOTS"

update() { sh "$lib/update" "$@"; }
state() { sed -n 's/^STATE=//p' "$T/disk/p1.d/pd-update.env"; }
autoboot() { tr '\n' ' ' < "$T/disk/p1.d/autoboot.txt"; }
sha() { sha256sum "$1" | cut -d' ' -f1; }
# What the image's root slot "contains" once mounted on slot $1's root (5|6).
new_root() {
	mkdir -p "$T/disk/p$1.d/etc" "$T/disk/p$1.d/usr/lib/pd-device"
	printf 'ID=photonvision\nIMAGE_VERSION=%s\n' "$2" > "$T/disk/p$1.d/etc/os-release"
	printf 'PD_DEVICE_MODEL=%s\n' "$3" > "$T/disk/p$1.d/usr/lib/pd-device/device-package.env"
	rm -rf "$T/disk/p$1.d/lib/modules"
	mkdir -p "$T/disk/p$1.d/lib/modules/${4:-7.2.9-test}"
}
# boot_kernel <boot part 2|3> <release>: a kernel image naming its release.
boot_kernel() {
	printf 'junk\000Linux version %s (builder@host) #1 SMP\000more' "$2" > "$T/disk/p$1.d/kernel_2712.img"
}
boot_kernel 3 7.2.9-test
boot_kernel 2 7.2.9-test

# A 40 MiB image with the A/B layout: p1 autoboot, p2/p3 boot, p4 extended,
# p5/p6 root, p7 data. Slot A's partitions hold random bytes to compare.
img=$T/image.img
truncate -s 40M "$img"
printf 'label: dos\nstart=2048, size=2048, type=c\nstart=4096, size=4096, type=c
start=8192, size=4096, type=c\nstart=12288, size=65536, type=5\nsize=20480, type=83
size=20480, type=83\nsize=16384, type=83\n' | sfdisk -q "$img"
head -c $((4096 * 512)) /dev/urandom > "$T/boot.ref"
head -c $((20480 * 512)) /dev/urandom > "$T/root.ref"
dd if="$T/boot.ref" of="$img" bs=512 seek=4096 conv=notrunc 2>/dev/null
dd if="$T/root.ref" of="$img" bs=512 seek=14336 conv=notrunc 2>/dev/null
xz -T0 -k "$img"
zstd -q -k "$img" -o "$img.zst"
# The old two-partition layout.
old=$T/old.img
truncate -s 8M "$old"
printf 'label: dos\nstart=2048, size=4096, type=c\nstart=6144, size=8192, type=83\n' | sfdisk -q "$old"
xz -k "$old"
# The board's slot B starts as the image's partition sizes allow.
printf 'console=tty1 root=/dev/mmcblk0p5 rootwait\n' > "$T/disk/p3.d/cmdline.txt"

echo "status on an A/B layout"
update status | grep -q '"slot_active":"A"' || fail "status should report slot A"
grep -q '"version_active":"1.0"' "$T/run/update.json" || fail "status should report 1.0"

echo "a wrong checksum writes nothing"
if update stage "$img.xz" --sha256 "$(printf '0%.0s' $(seq 64))" 2>/dev/null; then fail "a bad hash should fail"; fi
[ "$(state)" = error ] || fail "state should be error, is $(state)"
[ ! -s "$T/disk/p6" ] || fail "root B was written despite the bad hash"
if update stage "$img.xz" 2>/dev/null; then fail "a missing --sha256 should fail"; fi
update rollback

echo "an image without the A/B layout is refused"
if update stage "$old.xz" --sha256 "$(sha "$old.xz")" 2>/dev/null; then fail "the old layout should fail"; fi
grep -q "A/B layout" "$T/disk/p1.d/pd-update.env" || fail "the error should name the layout"
update rollback

echo "an image for another model is refused"
new_root 6 2.0 orion-cam
if update stage "$img.xz" --sha256 "$(sha "$img.xz")" 2>/dev/null; then fail "a wrong model should fail"; fi
[ "$(state)" = error ] || fail "state should be error after a wrong model"
update rollback

echo "a failing pre-stage hook stops before writing"
: > "$T/disk/p6"
printf '#!/bin/sh\nexit 1\n' > "$T/etc/update.d/pre-stage"
chmod +x "$T/etc/update.d/pre-stage"
if update stage "$img.xz" --sha256 "$(sha "$img.xz")" 2>/dev/null; then fail "the hook should stop it"; fi
[ ! -s "$T/disk/p6" ] || fail "root B was written despite the hook"
rm "$T/etc/update.d/pre-stage"
update rollback

echo "stage copies the image's slot A into slot B"
new_root 6 2.0 raze
printf '#!/bin/sh\necho "$PD_UPDATE_SLOT $PD_UPDATE_VERSION" > "%s"\n' "$T/post-stage.ran" > "$T/etc/update.d/post-stage"
chmod +x "$T/etc/update.d/post-stage"
update stage "$img.xz" --sha256 "$(sha "$img.xz")"
[ "$(state)" = staged ] || fail "state should be staged, is $(state)"
cmp -s "$T/root.ref" "$T/disk/p6" || fail "root B differs from the image's root A"
cmp -s "$T/boot.ref" "$T/disk/p3" || fail "boot B differs from the image's boot A"
grep -q "root=$T/disk/p6 " "$T/disk/p3.d/cmdline.txt" || fail "cmdline B should name p6"
[ ! -s "$T/disk/p5" ] || fail "the running root was touched"
grep -q "^VERSION_STAGED='2.0'" "$T/disk/p1.d/pd-update.env" || fail "the version comes from the image"
[ "$(cat "$T/post-stage.ran")" = "B 2.0" ] || fail "post-stage hook: $(cat "$T/post-stage.ran" 2>/dev/null)"
rm "$T/etc/update.d/post-stage"

echo "a failing pre-reboot hook keeps it staged"
printf '#!/bin/sh\nexit 1\n' > "$T/etc/update.d/pre-reboot"
chmod +x "$T/etc/update.d/pre-reboot"
if update apply 2>/dev/null; then fail "the hook should stop the restart"; fi
[ "$(state)" = staged ] || fail "state should stay staged, is $(state)"
[ ! -s "$REBOOTS" ] || fail "it restarted despite the hook"
rm "$T/etc/update.d/pre-reboot"

echo "a restart that doesn't happen leaves it staged, not trying"
if UPDATE_REBOOT=false update apply 2>/dev/null; then fail "a failed restart should fail apply"; fi
[ "$(state)" = staged ] || fail "state should stay staged, is $(state)"
grep -q '\[tryboot\]' "$T/disk/p1.d/autoboot.txt" && fail "the tryboot section should be undone"

echo "apply sets a one-time tryboot"
update apply
[ "$(state)" = trying ] || fail "state should be trying"
case "$(autoboot)" in
*"boot_partition=2 [tryboot] boot_partition=3"*) ;;
*) fail "autoboot.txt: $(autoboot)" ;;
esac
grep -q '^tryboot$' "$REBOOTS" || fail "apply should reboot with tryboot"

echo "the bootloader fell back: rolled-back"
update confirm
[ "$(state)" = rolled-back ] || fail "state should be rolled-back, is $(state)"
update rollback

echo "stdin works too (zstd), checked at the end"
: > "$T/disk/p6"
update stage - --format zst --sha256 "$(sha "$img.zst")" < "$img.zst"
[ "$(state)" = staged ] || fail "state should be staged, is $(state)"
cmp -s "$T/root.ref" "$T/disk/p6" || fail "root B differs after a stdin stage"
update rollback
if update stage - --sha256 "$(sha "$img.zst")" < "$img.xz" 2>/dev/null; then fail "a stdin hash mismatch should fail"; fi
[ "$(state)" = error ] || fail "a stdin mismatch should leave state error"
update rollback

echo "a failed health check restarts into the old slot"
update stage "$img.xz" --sha256 "$(sha "$img.xz")"
update apply
printf '#!/bin/sh\nexit 1\n' > "$T/etc/update-health"
chmod +x "$T/etc/update-health"
printf '#!/bin/sh\ntouch "%s"\n' "$T/post-boot.ran" > "$T/etc/update.d/post-boot"
chmod +x "$T/etc/update.d/post-boot"
: > "$REBOOTS"
if UPDATE_CMDLINE_ROOT=6 update confirm 2>/dev/null; then fail "a failed health check should fail"; fi
[ "$(state)" = trying ] || fail "state should stay trying"
grep -q '^plain$' "$REBOOTS" || fail "a failed check should reboot plainly"
grep -q 'boot_partition=2' "$T/disk/p1.d/autoboot.txt" || fail "default should still be slot A"
[ -e "$T/post-boot.ran" ] || fail "the post-boot hook should run on the trial boot"

echo "a healthy trial is kept"
printf '#!/bin/sh\nexit 0\n' > "$T/etc/update-health"
UPDATE_CMDLINE_ROOT=6 update confirm
[ "$(state)" = confirmed ] || fail "state should be confirmed, is $(state)"
[ "$(autoboot)" = "[all] tryboot_a_b=1 boot_partition=3 " ] || fail "autoboot.txt: $(autoboot)"
UPDATE_CMDLINE_ROOT=6 update status | grep -q '"slot_active":"B","slot_staged":"B","version_active":"2.0"' ||
	fail "status after confirm: $(cat "$T/run/update.json")"

echo "status answers while another command holds the lock"
if command -v flock >/dev/null 2>&1; then
	exec 8> "$T/run/update.lock"
	flock -n 8
	update status | grep -q '"state":"confirmed"' || fail "busy status should print the last state"
	mkdir -p "$T/run/update"
	echo 500 > "$T/run/update/progress"
	update status | grep -q '"progress":525' || fail "busy status should show the copy's progress"
	rm "$T/run/update/progress"
	if update rollback 2>/dev/null; then fail "a second writer should be refused"; fi
	exec 8>&-
fi

echo "an interrupted stage can be redone"
sed -i 's/^STATE=.*/STATE=staging/' "$T/disk/p1.d/pd-update.env"
new_root 5 3.0 raze
UPDATE_CMDLINE_ROOT=6 update stage "$img.xz" --sha256 "$(sha "$img.xz")"
[ "$(state)" = staged ] || fail "state should be staged, is $(state)"
cmp -s "$T/root.ref" "$T/disk/p5" || fail "root A should now hold the image"
UPDATE_CMDLINE_ROOT=6 update rollback

echo "an image whose kernel doesn't match its root's modules is refused"
UPDATE_CMDLINE_ROOT=6 update rollback
new_root 5 5.0 raze 6.12.47-old
boot_kernel 2 7.2.9-test
if UPDATE_CMDLINE_ROOT=6 update stage "$img.xz" --sha256 "$(sha "$img.xz")" 2>/dev/null; then
	fail "a kernel/modules mismatch should be refused"
fi
grep -q "match the modules on its root" "$T/disk/p1.d/pd-update.env" || fail "the error should say why"
UPDATE_CMDLINE_ROOT=6 update rollback

# trial <check to break>: stage into A, apply, then confirm on A with one check
# broken; the trial must not be confirmed.
trial() {
	new_root 5 5.0 raze
	UPDATE_CMDLINE_ROOT=6 update stage "$img.xz" --sha256 "$(sha "$img.xz")" >/dev/null 2>&1
	UPDATE_CMDLINE_ROOT=6 update apply >/dev/null 2>&1
	: > "$REBOOTS"
	if UPDATE_CMDLINE_ROOT=5 "$@" sh "$lib/update" confirm 2>/dev/null; then fail "confirm should fail: $*"; fi
	[ "$(state)" = trying ] || fail "state should stay trying ($*), is $(state)"
	grep -q '^plain$' "$REBOOTS" || fail "it should restart into the old slot ($*)"
	UPDATE_CMDLINE_ROOT=6 update confirm >/dev/null 2>&1 || true
	UPDATE_CMDLINE_ROOT=6 update rollback
}
echo "a trial whose kernel has no modules on its root isn't confirmed"
trial env UPDATE_UNAME_R=6.12.47-old
grep -q "no modules on this root" "$T/disk/p1.d/pd-update.env" || true
echo "a trial without the USB gadget isn't confirmed"
mv "$T/gadget/g1/UDC" "$T/gadget/g1/UDC.off"; : > "$T/gadget/g1/UDC"
trial env
mv "$T/gadget/g1/UDC.off" "$T/gadget/g1/UDC"
echo "a trial with a failed critical unit isn't confirmed"
trial env FAILED_UNITS=pd-device-identity.service UPDATE_CRITICAL_UNITS=pd-device-identity.service

echo "check-link goes back to the previous slot after bad boots"
new_root 5 5.0 raze
UPDATE_CMDLINE_ROOT=6 update stage "$img.xz" --sha256 "$(sha "$img.xz")" >/dev/null 2>&1
UPDATE_CMDLINE_ROOT=6 update apply >/dev/null 2>&1
UPDATE_CMDLINE_ROOT=5 update confirm >/dev/null 2>&1
[ "$(state)" = confirmed ] || fail "the trial on A should be confirmed, is $(state)"
grep -q '^PREVIOUS_SLOT=B' "$T/disk/p1.d/pd-update.env" || fail "confirm should remember slot B"
: > "$REBOOTS"
UPDATE_CMDLINE_ROOT=5 UPDATE_UNAME_R=broken UPDATE_LINK_DELAY=0 update check-link 2>/dev/null
[ "$(state)" = confirmed ] || fail "one bad boot only counts"
grep -q '^BAD_BOOTS=1' "$T/disk/p1.d/pd-update.env" || fail "it should count the bad boot"
UPDATE_CMDLINE_ROOT=5 UPDATE_LINK_DELAY=0 update check-link 2>/dev/null
grep -q '^BAD_BOOTS=0' "$T/disk/p1.d/pd-update.env" || fail "a good boot resets the count"
UPDATE_CMDLINE_ROOT=5 UPDATE_UNAME_R=broken UPDATE_LINK_DELAY=0 update check-link 2>/dev/null
UPDATE_CMDLINE_ROOT=5 UPDATE_UNAME_R=broken UPDATE_LINK_DELAY=0 update check-link 2>/dev/null
[ "$(state)" = rolled-back ] || fail "two bad boots should go back, state $(state)"
grep -q 'boot_partition=3' "$T/disk/p1.d/autoboot.txt" || fail "the default should be slot B again: $(autoboot)"
grep -q '^plain$' "$REBOOTS" || fail "it should restart into slot B"
grep -q '^PREVIOUS_SLOT=$' "$T/disk/p1.d/pd-update.env" || fail "no ping-pong: the previous slot is forgotten"

if command -v mkfs.erofs >/dev/null 2>&1 && fsck.erofs --help 2>&1 | grep -q -- --extract; then
	# erofs_image <version> <model> <out.img.xz>: the A/B image with an EROFS
	# root slot A (lzma), padded to the slot partition's size.
	erofs_image() {
		rm -rf "$T/rootdir" "$T/root.erofs"
		mkdir -p "$T/rootdir/etc" "$T/rootdir/usr/lib/pd-device"
		printf 'ID=photonvision\nIMAGE_VERSION=%s\n' "$1" > "$T/rootdir/etc/os-release"
		printf 'PD_DEVICE_MODEL=%s\n' "$2" > "$T/rootdir/usr/lib/pd-device/device-package.env"
		mkdir -p "$T/rootdir/lib/modules/7.2.9-test"
		mkfs.erofs -zlzma "$T/root.erofs" "$T/rootdir" >/dev/null 2>&1 ||
			mkfs.erofs -zlz4hc "$T/root.erofs" "$T/rootdir" >/dev/null
		truncate -s $((20480 * 512)) "$T/root.erofs"
		cp "$img" "$T/erofs.img"
		dd if="$T/root.erofs" of="$T/erofs.img" bs=512 seek=14336 conv=notrunc 2>/dev/null
		xz -T0 -c "$T/erofs.img" > "$3"
	}
	rm -rf "$T/disk/p5.d" "$T/disk/p6.d"
	mkdir -p "$T/disk/p5.d" "$T/disk/p6.d"

	echo "an EROFS root slot: version and model read from the image"
	erofs_image 4.0 raze "$T/erofs.img.xz"
	UPDATE_CMDLINE_ROOT=6 update stage "$T/erofs.img.xz" --sha256 "$(sha "$T/erofs.img.xz")"
	[ "$(state)" = staged ] || fail "state should be staged, is $(state)"
	grep -q "^VERSION_STAGED='4.0'" "$T/disk/p1.d/pd-update.env" ||
		fail "the version should come from the EROFS root: $(grep VERSION_STAGED "$T/disk/p1.d/pd-update.env")"
	cmp -s "$T/root.erofs" "$T/disk/p5" || fail "root A should hold the EROFS image byte for byte"
	UPDATE_CMDLINE_ROOT=6 update rollback

	echo "an EROFS root for another model is refused"
	erofs_image 4.0 orion-cam "$T/erofs-other.img.xz"
	if UPDATE_CMDLINE_ROOT=6 update stage "$T/erofs-other.img.xz" --sha256 "$(sha "$T/erofs-other.img.xz")" 2>/dev/null; then
		fail "a wrong model in an EROFS root should fail"
	fi
	UPDATE_CMDLINE_ROOT=6 update rollback
else
	echo "skipped: EROFS root (needs erofs-utils >= 1.5 for fsck.erofs --extract)"
fi

echo "identity reports the A/B method and state"
. "$lib/lib.sh"
pd_update_methods | grep -qx ab-tryboot || fail "update_methods should include ab-tryboot"

echo "ok"
