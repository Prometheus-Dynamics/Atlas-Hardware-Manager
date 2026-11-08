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
	# What the lock looks like, for a failure seen only elsewhere (CI).
	if [ -e "${BOARD_REBOOT_LOCK:-}" ]; then
		ino=$(ls -i "$BOARD_REBOOT_LOCK" | awk '{print $1}')
		echo "lock inode $ino, expected holder ${holder:-none}; /proc/locks:" >&2
		grep -E ":$ino( |$)" /proc/locks >&2 || true
		for p in $(awk -v ino="$ino" '$2 != "->" && $6 ~ (":" ino "$") { print $5 }' /proc/locks); do
			echo "pid $p: $(tr '\0' ' ' < "/proc/$p/cmdline" 2>&1) [$(awk '/^(State|PPid|Tgid):/ { printf "%s ", $0 }' "/proc/$p/status" 2>&1)]" >&2
		done
		ps -eo pid,ppid,stat,args 2>/dev/null | awk -v t=$$ -v h="${holder:-x}" '$1 == t || $2 == t || $1 == h' | cut -c1-160 >&2 || true
	fi
	exit 1
}
export BOARD_REBOOT_LOCK="$T/raze-gpu.lock" BOARD_REBOOT_LOCK_HOLD=0 BOARD_REBOOT_WAIT_FILE="$T/run/reboot-wait"

echo "a free lock: no holder, and the restart runs at once"
[ -z "$(sh "$rl" --holder)" ] || fail "a holder of a free lock: $(sh "$rl" --holder)"
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
[ ! -e "$BOARD_REBOOT_WAIT_FILE" ] || fail "the wait file outlived a refused restart"
grep -q 'still held' "$T/err" || fail "the refusal: $(cat "$T/err")"
# The flock that took it has exited: the holder named is the process
# still holding the fd ($holder, now `sleep 30`), with its command.
grep -q "waiting for raze-gpu.lock held by pid $holder (sleep 30)" "$T/err" || fail "the wait doesn't name the holder: $(cat "$T/err")"
[ "$(sh "$rl" --holder)" = "pid $holder (sleep 30)" ] || fail "--holder: $(sh "$rl" --holder), want pid $holder (sleep 30)"

echo "a restart that recovers the board goes anyway past the limit, naming the holder"
BOARD_REBOOT_LOCK_WAIT=1 BOARD_REBOOT_LOCK_OVERRIDE=1 sh "$rl" sh -c "echo ran >> '$T/runs'" 2> "$T/err" || fail "the recovery restart failed"
grep -q 'overriding raze-gpu.lock.*by pid [0-9]*' "$T/err" || fail "the override: $(cat "$T/err")"
[ "$(wc -l < "$T/runs")" = 2 ] || fail "the recovery restart didn't run"
echo ran > "$T/runs"
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
# While it waits, the wait file names the holder (HeliOS's reboot_waiting).
until [ -s "$BOARD_REBOOT_WAIT_FILE" ]; do kill -0 "$waiter" 2>/dev/null || fail "the waiter ended early"; done
grep -q "waiting for raze-gpu.lock held by pid $holder (sleep 30)" "$BOARD_REBOOT_WAIT_FILE" || fail "the wait file: $(cat "$BOARD_REBOOT_WAIT_FILE")"
kill "$holder"
holder=''
wait "$waiter" || fail "the waiting restart failed"
[ ! -e "$BOARD_REBOOT_WAIT_FILE" ] || fail "the wait file outlived the wait"
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
echo "a holder that shells out (the lock on an inherited fd) restarts at once instead of waiting for itself"
n=$(wc -l < "$T/runs")
(
	exec 3>> "$BOARD_REBOOT_LOCK"
	flock 3
	BOARD_REBOOT_LOCK_WAIT=1 BOARD_REBOOT_LOCK_HOLD=0 sh "$rl" sh -c "echo ran >> '$T/runs'"
) 2> "$T/err" || fail "the holder's own restart was refused: $(cat "$T/err")"
[ "$(wc -l < "$T/runs")" = $((n + 1)) ] || fail "the holder's own restart didn't run"
grep -q 'inherited fd 3' "$T/err" || fail "the log doesn't say it's the holder's own lock: $(cat "$T/err")"
flock -n "$BOARD_REBOOT_LOCK" true || fail "the lock outlived its holder"
echo ok
