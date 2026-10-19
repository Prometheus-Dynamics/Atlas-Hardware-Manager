#!/bin/sh
# Static check of the USB gadget consoles' getty drop-ins: systemd must not
# reset, vhangup or deallocate ttyGS0/ttyGS1. PID 1 writes reset and probe
# sequences to the tty then, and on a gadget port the computer doesn't read,
# those writes block, PID 1 stops feeding the watchdog, and the board resets
# mid-shutdown (bench board 2026-10-19). Run: sh devices/raze/tests/gadget-getty.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
units=$here/../gaia/assets/rootfs/usr/lib/systemd/system
fail() {
	echo "FAIL: $*" >&2
	exit 1
}
for t in ttyGS0 ttyGS1; do
	f=$units/serial-getty@$t.service.d/50-board.conf
	[ -r "$f" ] || fail "$f is missing"
	for k in TTYReset TTYVHangup TTYVTDisallocate; do
		grep -qx "$k=no" "$f" || fail "$t: $k=no is missing from $f"
	done
done
echo "ok"
