#!/bin/sh
# Off-device test of pstore: the kernel logs a boot left in pstore are moved
# to /data/board/pstore/<boot id>/ and freed, a kernel.pstore event is logged
# for a panic record or after an unclean shutdown (not for the console log
# of a clean restart), and only the newest few are kept. A directory stands
# in for /sys/fs/pstore. Run: sh devices/raze/tests/pstore.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
lib=$here/../gaia/assets/rootfs/usr/lib/board
T=$(mktemp -d "${TMPDIR:-/tmp}/board-pstore-test.XXXXXX")
trap 'rm -rf "$T"' EXIT

fail() {
	echo "FAIL: $*" >&2
	exit 1
}

mkdir -p "$T/pstore" "$T/run" "$T/data"
export BOARD_LIB_DIR=$lib BOARD_RUN_DIR=$T/run BOARD_DATA_DIR=$T/data BOARD_ETC_DIR=$T/etc \
	BOARD_PSTORE_DIR=$T/pstore
run() { BOARD_BOOT_ID=$1 sh "$lib/pstore" 2> "$T/log" || fail "pstore failed: $(cat "$T/log")"; }
events() { cat "$T/data/events.jsonl" 2>/dev/null; }
pstore_events() { events | grep -c '"kind":"kernel.pstore"' || true; }
# boot <id> [clean]: an earlier boot's events, ending in a shutdown if clean.
boot() {
	BOARD_BOOT_ID=$1 sh "$lib/event" boot "boot" >/dev/null 2>&1 || true
	[ "${2:-}" != clean ] || BOARD_BOOT_ID=$1 sh "$lib/event" shutdown "shutting down cleanly" >/dev/null 2>&1 || true
}

echo "nothing in pstore: nothing happens"
run b1
[ ! -d "$T/data/pstore" ] || fail "no directory without logs"

echo "the console log after a clean restart is kept quietly"
boot b1 clean
events | grep -q '"kind":"shutdown"' || fail "the test's shutdown event: $(events)"
printf '[   1.0] booting\n[  60.0] reboot: Restarting system\n' > "$T/pstore/console-ramoops-0"
run b2
[ -f "$T/data/pstore/b2/console-ramoops-0" ] || fail "the console log should be copied"
[ ! -e "$T/pstore/console-ramoops-0" ] || fail "and freed from pstore"
[ "$(pstore_events)" = 0 ] || fail "no event for a clean restart: $(events)"

echo "after an unclean end: an event, with the console's last line"
boot b2
printf '[   1.0] booting\n[  74.5] dwc2 1000480000.usb: something\000\000\000' > "$T/pstore/console-ramoops-0"
run b3
[ "$(pstore_events)" = 1 ] || fail "one event: $(events)"
events | grep '"kind":"kernel.pstore"' | grep -q '"previous_clean":"false"' || fail "previous_clean: $(events)"
events | grep '"kind":"kernel.pstore"' | grep -q '"last_line":"\[  74.5\] dwc2 1000480000.usb: something"' ||
	fail "the last line: $(events)"
events | grep -q "without a clean shutdown; its kernel log is in $T/data/pstore/b3" || fail "the message: $(events)"

echo "a panic record: an event even after a clean shutdown"
boot b3 clean
printf 'Panic#1 Part1\nKernel panic - not syncing: Hard LOCKUP\n' > "$T/pstore/dmesg-ramoops-0"
: > "$T/pstore/console-ramoops-0"
run b4
[ "$(pstore_events)" = 2 ] || fail "two events: $(events)"
events | grep '"kind":"kernel.pstore"' | tail -n 1 | grep -q '"panics":"1"' || fail "the panic count: $(events)"
events | grep -q 'ended in a kernel panic or oops' || fail "the panic message: $(events)"
[ -z "$(ls -A "$T/pstore")" ] || fail "pstore should be empty: $(ls -A "$T/pstore")"

echo "only the newest five are kept"
for n in 5 6 7 8 9; do
	sleep 1
	printf 'x\n' > "$T/pstore/console-ramoops-0"
	run "b$n"
done
[ "$(ls -1 "$T/data/pstore" | wc -l | tr -d ' ')" = 5 ] || fail "five kept: $(ls "$T/data/pstore")"
[ ! -d "$T/data/pstore/b2" ] && [ -d "$T/data/pstore/b9" ] || fail "the oldest go: $(ls "$T/data/pstore")"

echo "ok"
