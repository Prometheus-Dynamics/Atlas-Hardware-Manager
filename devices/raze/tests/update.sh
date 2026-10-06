#!/bin/sh
# Off-device test of pd-device-update: partitions are plain files, a FAT
# "mount" is a directory next to its file, and reboots are recorded.
# Needs sh, tar, zstd, sha256sum. Run: sh devices/raze/tests/update.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
lib=$here/../gaia/assets/rootfs/usr/lib/pd-device
T=$(mktemp -d "${TMPDIR:-/tmp}/pd-update-test.XXXXXX")
trap 'rm -rf "$T"' EXIT

fail() {
	echo "FAIL: $*" >&2
	exit 1
}

mkdir -p "$T/disk" "$T/etc" "$T/run" "$T/bin" "$T/bundle"
for n in 1 2 3 5 6; do
	: > "$T/disk/p$n"
	mkdir -p "$T/disk/p$n.d"
done
printf '[all]\ntryboot_a_b=1\nboot_partition=2\n' > "$T/disk/p1.d/autoboot.txt"
printf 'ID=photonvision\nVERSION_ID=1.0\n' > "$T/os-release"
printf 'UPDATE_CONFIRM_DELAY=0\n' > "$T/etc/update.env"

cat > "$T/bin/mount" <<'EOF'
#!/bin/sh
rmdir "$2" 2>/dev/null || rm -f "$2"
ln -s "$1.d" "$2"
EOF
cat > "$T/bin/umount" <<'EOF'
#!/bin/sh
rm -f "$1"
EOF
cat > "$T/bin/reboot" <<'EOF'
#!/bin/sh
echo "$*" >> "$REBOOTS"
EOF
chmod +x "$T/bin/"*

export PD_LIB_DIR=$lib PD_ETC_DIR=$T/etc PD_RUN_DIR=$T/run PD_OS_RELEASE=$T/os-release
export UPDATE_PART_PREFIX=$T/disk/p UPDATE_MOUNT=$T/bin/mount UPDATE_UMOUNT=$T/bin/umount
export UPDATE_REBOOT="$T/bin/reboot tryboot" UPDATE_REBOOT_PLAIN="$T/bin/reboot plain"
export REBOOTS=$T/reboots UPDATE_CMDLINE_ROOT=5
: > "$REBOOTS"

update() { sh "$lib/update" "$@"; }
state() { sed -n 's/^STATE=//p' "$T/disk/p1.d/pd-update.env"; }
autoboot() { tr '\n' ' ' < "$T/disk/p1.d/autoboot.txt"; }

# A bundle for version 2.0.
head -c 300000 /dev/urandom > "$T/rootfs.ext4"
head -c 50000 /dev/urandom > "$T/boot.vfat"
zstd -q "$T/rootfs.ext4" -o "$T/bundle/rootfs.ext4.zst"
zstd -q "$T/boot.vfat" -o "$T/bundle/boot.vfat.zst"
cat > "$T/bundle/manifest.env" <<EOF
MODEL=raze
VERSION=2.0
OS=photonvision
BOOT_SHA256=$(sha256sum "$T/bundle/boot.vfat.zst" | cut -d' ' -f1)
ROOTFS_SHA256=$(sha256sum "$T/bundle/rootfs.ext4.zst" | cut -d' ' -f1)
EOF
(cd "$T/bundle" && tar -cf "$T/good.pdupdate" manifest.env boot.vfat.zst rootfs.ext4.zst)
# The boot image names slot A's root; the writer must point slot B's at p6.
printf 'console=tty1 root=/dev/mmcblk0p5 rootwait\n' > "$T/disk/p3.d/cmdline.txt"

echo "status on an A/B layout"
update status | grep -q '"slot_active":"A"' || fail "status should report slot A"
grep -q '"version_active":"1.0"' "$T/run/update.json" || fail "status should report 1.0"

echo "a corrupt bundle writes nothing"
cp "$T/bundle/manifest.env" "$T/bad-manifest.env"
sed -i 's/^ROOTFS_SHA256=.*/ROOTFS_SHA256=0000/' "$T/bundle/manifest.env"
(cd "$T/bundle" && tar -cf "$T/bad.pdupdate" manifest.env boot.vfat.zst rootfs.ext4.zst)
cp "$T/bad-manifest.env" "$T/bundle/manifest.env"
if update stage "$T/bad.pdupdate" 2>/dev/null; then fail "a bad hash should fail"; fi
[ "$(state)" = error ] || fail "state should be error, is $(state)"
[ ! -s "$T/disk/p6" ] || fail "root B was written despite the bad hash"
update rollback

echo "a bundle for another model is refused"
printf 'PD_DEVICE_MODEL=orion-cam\n' > "$T/etc/device-package.env"
if update stage "$T/good.pdupdate" 2>/dev/null; then fail "a wrong model should fail"; fi
rm "$T/etc/device-package.env"
update rollback

echo "stage writes slot B"
update stage "$T/good.pdupdate"
[ "$(state)" = staged ] || fail "state should be staged, is $(state)"
cmp -s "$T/rootfs.ext4" "$T/disk/p6" || fail "root B differs from the bundle"
cmp -s "$T/boot.vfat" "$T/disk/p3" || fail "boot B differs from the bundle"
grep -q "root=$T/disk/p6 " "$T/disk/p3.d/cmdline.txt" || fail "cmdline B should name p6"
[ ! -s "$T/disk/p5" ] || fail "the running root was touched"

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

echo "a failed health check restarts into the old slot"
update stage "$T/good.pdupdate"
update apply
printf '#!/bin/sh\nexit 1\n' > "$T/etc/update-health"
chmod +x "$T/etc/update-health"
: > "$REBOOTS"
if UPDATE_CMDLINE_ROOT=6 update confirm 2>/dev/null; then fail "a failed health check should fail"; fi
[ "$(state)" = trying ] || fail "state should stay trying"
grep -q '^plain$' "$REBOOTS" || fail "a failed check should reboot plainly"
grep -q 'boot_partition=2' "$T/disk/p1.d/autoboot.txt" || fail "default should still be slot A"

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
	if update rollback 2>/dev/null; then fail "a second writer should be refused"; fi
	exec 8>&-
fi

echo "an interrupted stage can be redone"
sed -i 's/^STATE=.*/STATE=staging/' "$T/disk/p1.d/pd-update.env"
UPDATE_CMDLINE_ROOT=6 update stage "$T/good.pdupdate"
[ "$(state)" = staged ] || fail "state should be staged, is $(state)"
UPDATE_CMDLINE_ROOT=6 update rollback

echo "identity reports the A/B method and state"
. "$lib/lib.sh"
pd_update_methods | grep -qx ab-tryboot || fail "update_methods should include ab-tryboot"

echo "ok"
