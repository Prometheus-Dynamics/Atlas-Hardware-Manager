#!/bin/sh
# Off-device test of usb-boot: a fake firmware mailbox and device tree, and
# the reboot recorded. Run: sh devices/raze/tests/usb-boot.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
lib=$here/../gaia/assets/rootfs/usr/lib/board
T=$(mktemp -d "${TMPDIR:-/tmp}/board-usb-boot-test.XXXXXX")
trap 'rm -rf "$T"' EXIT

fail() {
	echo "FAIL: $*" >&2
	exit 1
}

mkdir -p "$T/dt" "$T/run"
printf 'raspberrypi,5-compute-module\000brcm,bcm2712\000' > "$T/dt/compatible"
cat > "$T/vcmailbox" <<'MB'
#!/bin/sh
echo "$*" >> "$CALLS"
# Echo the request back the way the firmware answers it.
printf '0x0000001c %s 0x0003808b 0x00000004 0x80000004 0x%08x 0x00000000\n' "${MB_CODE:-0x80000000}" "$4"
MB
chmod +x "$T/vcmailbox"

export BOARD_LIB_DIR=$lib BOARD_DT_DIR=$T/dt BOARD_RUN_DIR=$T/run BOARD_VCMAILBOX=$T/vcmailbox
export CALLS=$T/calls USB_BOOT_REBOOT="echo reboot >> $T/reboots" USB_BOOT_SYNC=true
run() { sh "$lib/usb-boot" "$@"; }

echo "a CM5 with vcmailbox supports it"
run --check || fail "--check should pass"
. "$lib/lib.sh"
board_update_methods | grep -qx usb-boot-reboot || fail "update_methods should include usb-boot-reboot"

echo "it sets RPIBOOT for one boot, then reboots"
run 2>/dev/null
grep -qx '0x0003808b 4 4 0x3' "$T/calls" || fail "mailbox call: $(cat "$T/calls")"
grep -qx reboot "$T/reboots" || fail "it should reboot"

echo "a refused request doesn't reboot"
rm -f "$T/reboots"
if MB_CODE=0x80000001 run 2>/dev/null; then fail "a refused request should fail"; fi
[ ! -e "$T/reboots" ] || fail "it rebooted after a refusal"

echo "another SoC doesn't support it"
printf 'raspberrypi,4-model-b\000brcm,bcm2711\000' > "$T/dt/compatible"
if run --check; then fail "--check should fail on bcm2711"; fi
if run 2>/dev/null; then fail "it should refuse on bcm2711"; fi
[ ! -e "$T/reboots" ] || fail "it rebooted on bcm2711"
board_update_methods | grep -qx usb-boot-reboot && fail "no usb-boot-reboot on bcm2711"

echo "ok"
