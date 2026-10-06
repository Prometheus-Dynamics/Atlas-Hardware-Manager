#!/bin/sh
# pd_root_device / pd_sibling_partition (lib.sh), which ssh-keys and update
# use to find the eMMC. Run: sh devices/raze/tests/root-device.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
T=$(mktemp -d "${TMPDIR:-/tmp}/pd-root-test.XXXXXX")
trap 'rm -rf "$T"' EXIT
# shellcheck source=../gaia/assets/rootfs/usr/lib/pd-device/lib.sh
. "$here/../gaia/assets/rootfs/usr/lib/pd-device/lib.sh"
export PD_PROC_CMDLINE=$T/cmdline

check() {
	# check <cmdline> <expected root> <expected partition 1>
	printf '%s\n' "$1" > "$PD_PROC_CMDLINE"
	got=$(pd_root_device)
	[ "$got" = "$2" ] || { echo "FAIL: '$1' gave root '$got', want '$2'" >&2; exit 1; }
	[ -n "$2" ] || return 0
	p1=$(pd_sibling_partition "$got" 1)
	[ "$p1" = "$3" ] || { echo "FAIL: '$1' gave p1 '$p1', want '$3'" >&2; exit 1; }
}

check "console=tty1 root=/dev/mmcblk0p5 rootwait" /dev/mmcblk0p5 /dev/mmcblk0p1
check "root=/dev/mmcblk0p2 overlayroot=tmpfs ro" /dev/mmcblk0p2 /dev/mmcblk0p1
check "root=/dev/sda6 quiet" /dev/sda6 /dev/sda1
check "root=/dev/nvme0n1p3" /dev/nvme0n1p3 /dev/nvme0n1p1
echo ok
