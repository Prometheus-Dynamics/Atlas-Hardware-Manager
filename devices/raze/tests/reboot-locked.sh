#!/bin/sh
# Off-device test of reboot-locked: a restart waits for the bench's lock,
# refuses past BOARD_REBOOT_LOCK_WAIT, runs at once when the lock is free,
# and holds the lock after the command. Run: sh devices/raze/tests/reboot-locked.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
rl=$here/../gaia/assets/rootfs/usr/lib/board/reboot-locked
command -v flock >/dev/null 2>&1 || {
	echo "reboot-locked: skipped (no flock)"
	exit 0
}
T=$(mktemp -d "${TMPDIR:-/tmp}/reboot-locked-test.XXXXXX")
holder=''
trap '[ -z "$holder" ] || kill "$holder" 2>/dev/null; rm -rf "$T"' EXIT
fail() {
	echo "FAIL: $*" >&2
	exit 1
}
export BOARD_REBOOT_LOCK="$T/raze-gpu.lock" BOARD_REBOOT_LOCK_HOLD=0

echo "a free lock: the restart runs at once"
sh "$rl" sh -c "echo ran >> '$T/runs'"
[ "$(cat "$T/runs")" = ran ] || fail "the command didn't run"

echo "a held lock: the restart waits, and past the limit is refused"
# One process holding it (sleep inherits the fd), so killing it frees it.
sh -c 'exec 9>> "$BOARD_REBOOT_LOCK"; flock 9; exec sleep 30' &
holder=$!
until ! flock -n "$BOARD_REBOOT_LOCK" true; do :; done
set +e
BOARD_REBOOT_LOCK_WAIT=1 sh "$rl" sh -c "echo ran >> '$T/runs'" 2> "$T/err"
r=$?
set -e
[ "$r" = 3 ] || fail "exit $r, want 3"
grep -q 'still held' "$T/err" || fail "the refusal: $(cat "$T/err")"
[ "$(wc -l < "$T/runs")" = 1 ] || fail "restarted under someone else's lock"

echo "the holder's own restart (<lock>.reboot-ok) goes ahead under its lock"
touch "$BOARD_REBOOT_LOCK.reboot-ok"
BOARD_REBOOT_LOCK_WAIT=1 sh "$rl" sh -c "echo ran >> '$T/runs'" || fail "the holder's restart was refused"
[ "$(wc -l < "$T/runs")" = 2 ] || fail "the holder's restart didn't run"
[ ! -e "$BOARD_REBOOT_LOCK.reboot-ok" ] || fail "the marker outlived its restart"
: > "$T/runs"; echo ran > "$T/runs"

echo "the restart goes once the lock is released"
BOARD_REBOOT_LOCK_WAIT=20 sh "$rl" sh -c "echo ran >> '$T/runs'" &
waiter=$!
kill "$holder"
holder=''
wait "$waiter" || fail "the waiting restart failed"
[ "$(wc -l < "$T/runs")" = 2 ] || fail "the waiting restart didn't run"

echo "the lock is held after the command, until the hold ends"
BOARD_REBOOT_LOCK_HOLD=3 sh "$rl" true &
pid=$!
until ! flock -n "$BOARD_REBOOT_LOCK" true; do kill -0 "$pid" 2>/dev/null || fail "ended early"; done
wait "$pid"
flock -n "$BOARD_REBOOT_LOCK" true || fail "the lock outlived the hold"

echo "a failed command is not held"
set +e
sh "$rl" false
r=$?
set -e
[ "$r" != 0 ] || fail "a failed restart reported success"
flock -n "$BOARD_REBOOT_LOCK" true || fail "a failed restart kept the lock"
echo ok
