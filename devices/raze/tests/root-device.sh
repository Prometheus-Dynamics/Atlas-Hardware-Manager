#!/bin/sh
# board_root_device / board_sibling_partition (lib.sh), which ssh-keys and update
# use to find the eMMC. Run: sh devices/raze/tests/root-device.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
T=$(mktemp -d "${TMPDIR:-/tmp}/board-root-test.XXXXXX")
trap 'rm -rf "$T"' EXIT
# shellcheck source=../gaia/assets/rootfs/usr/lib/board/lib.sh
. "$here/../gaia/assets/rootfs/usr/lib/board/lib.sh"
export BOARD_PROC_CMDLINE=$T/cmdline

check() {
	# check <cmdline> <expected root> <expected partition 1>
	printf '%s\n' "$1" > "$BOARD_PROC_CMDLINE"
	got=$(board_root_device)
	[ "$got" = "$2" ] || { echo "FAIL: '$1' gave root '$got', want '$2'" >&2; exit 1; }
	[ -n "$2" ] || return 0
	p1=$(board_sibling_partition "$got" 1)
	[ "$p1" = "$3" ] || { echo "FAIL: '$1' gave p1 '$p1', want '$3'" >&2; exit 1; }
}

check "console=tty1 root=/dev/mmcblk0p5 rootwait" /dev/mmcblk0p5 /dev/mmcblk0p1
check "root=/dev/mmcblk0p2 overlayroot=tmpfs ro" /dev/mmcblk0p2 /dev/mmcblk0p1
check "root=/dev/sda6 quiet" /dev/sda6 /dev/sda1
check "root=/dev/nvme0n1p3" /dev/nvme0n1p3 /dev/nvme0n1p1

# A slots board: root is dm-verity; the disk is /data's (atlas.data=), even
# with a leftover root= on the command line. findfs is stubbed; without
# PARTUUID support it finds the label.
mkdir -p "$T/bin"
cat > "$T/bin/findfs" <<'STUB'
#!/bin/sh
case "$1" in
PARTUUID=5056525a-02) [ -n "${NO_PARTUUID:-}" ] && exit 1; echo /dev/mmcblk0p2 ;;
LABEL=data) echo /dev/mmcblk0p2 ;;
*) exit 1 ;;
esac
STUB
chmod +x "$T/bin/findfs"
PATH=$T/bin:$PATH
slots="root=/dev/mmcblk0p5 ro atlas.slot=pv-a atlas.roothash=ab atlas.data=PARTUUID=5056525a-02 panic=5"
check "$slots" /dev/mmcblk0p2 /dev/mmcblk0p1
NO_PARTUUID=1 check "$slots" /dev/mmcblk0p2 /dev/mmcblk0p1
# slot-init's find-partition comes first (it knows PARTUUIDs; BusyBox's findfs doesn't).
printf '#!/bin/sh\n[ "$1 $2" = "find-partition PARTUUID=5056525a-02" ] && echo /dev/mmcblk1p2\n' > "$T/bin/slot-init"
chmod +x "$T/bin/slot-init"
BOARD_SLOT_INIT=$T/bin/slot-init check "$slots" /dev/mmcblk1p2 /dev/mmcblk1p1
[ "$(board_active_slot)" = pv-a ] || { echo "FAIL: active slot '$(board_active_slot)', want pv-a" >&2; exit 1; }
echo ok
