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

echo "a power reading after the shutdown event, then an update's tryboot: still clean"
boot c1 clean
BOARD_BOOT_ID=c1 sh "$lib/event" power "board supply at shutdown start" >/dev/null 2>&1 || true
printf "[   1.0] booting\n[2542.5] reboot: Restarting system with command '0 tryboot'\n" > "$T/pstore/console-ramoops-0"
run c2
[ "$(pstore_events)" = 0 ] || fail "no event after a clean tryboot restart: $(events)"

echo "the kernel's own restart line counts as clean without a shutdown event"
boot d1
printf '[   1.0] booting\n[  90.0] reboot: Power down\n' > "$T/pstore/console-ramoops-0"
run d2
[ "$(pstore_events)" = 0 ] || fail "no event when the console shows the end of a shutdown: $(events)"

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

echo "--show prints a record as text, the panic record first; refuses odd ids"
out=$(sh "$lib/pstore" --show b4) || fail "--show b4"
first=$(printf '%s\n' "$out" | head -n 1)
[ "$first" = "== dmesg-ramoops-0" ] || fail "the panic record first: $out"
printf '%s\n' "$out" | grep -q 'Kernel panic - not syncing: Hard LOCKUP' || fail "its text: $out"
printf '%s\n' "$out" | grep -qx '== console-ramoops-0' || fail "then the console: $out"
[ "$(printf '%s\n' "$out" | grep -c '^== ')" = 2 ] || fail "each file once: $out"
sh "$lib/pstore" --show b3 | grep -q 'dwc2 1000480000.usb: something' || fail "the console's NULs dropped"
if sh "$lib/pstore" --show nope 2>/dev/null; then fail "--show without a record must fail"; fi
set +e
sh "$lib/pstore" --show ../b4 >/dev/null 2>&1
rc=$?
set -e
[ "$rc" = 2 ] || fail "--show ../b4: exit 2, got $rc"

echo "only the newest few are kept, newest by arrival, not by date (no RTC)"
# The dirs so far came before .order: they go first, oldest by date.
for n in 5 6 7; do
	printf 'x\n' > "$T/pstore/console-ramoops-0"
	BOARD_PSTORE_KEEP=3 run "b$n"
done
[ "$(ls -1 "$T/data/pstore" | wc -l | tr -d ' ')" = 3 ] || fail "three kept: $(ls "$T/data/pstore")"
[ -d "$T/data/pstore/b5" ] && [ -d "$T/data/pstore/b7" ] || fail "the newest three: $(ls "$T/data/pstore")"
# Every boot starts at the same clock: a new dir dated before the kept ones
# is still the newest, and stays.
touch -d '2026-01-01 00:00' "$T/data/pstore/b5" "$T/data/pstore/b6" "$T/data/pstore/b7"
printf 'x\n' > "$T/pstore/console-ramoops-0"
BOARD_PSTORE_KEEP=3 run b8
touch -d '2025-01-01 00:00' "$T/data/pstore/b8"
printf 'x\n' > "$T/pstore/console-ramoops-0"
BOARD_PSTORE_KEEP=3 run b9
[ -f "$T/data/pstore/b9/console-ramoops-0" ] || fail "the dir just written was pruned: $(ls "$T/data/pstore")"
[ -d "$T/data/pstore/b8" ] && [ ! -d "$T/data/pstore/b6" ] || fail "kept by arrival: $(ls "$T/data/pstore")"
[ "$(ls -1 "$T/data/pstore" | wc -l | tr -d ' ')" = 3 ] || fail "three kept: $(ls "$T/data/pstore")"

echo "ok"
