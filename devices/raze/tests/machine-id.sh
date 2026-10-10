#!/bin/sh
# Off-device test of machine-id: a stable id bound over /etc/machine-id (a
# kept one, else one from the board serial), journald restarted only when it
# changed. mount and systemctl are stubs. Run: sh devices/raze/tests/machine-id.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
lib=$here/../gaia/assets/rootfs/usr/lib/board
T=$(mktemp -d "${TMPDIR:-/tmp}/board-machine-id-test.XXXXXX")
trap 'rm -rf "$T"' EXIT

fail() {
	echo "FAIL: $*" >&2
	exit 1
}

mkdir -p "$T/dt" "$T/run" "$T/data" "$T/bin"
# A "bind mount" copies the file over its target; restarts are recorded.
printf '#!/bin/sh\n[ "$1" = --bind ] && cp "$2" "$3"\n' > "$T/bin/mount"
printf '#!/bin/sh\necho "$*" >> "%s/restarts"\n' "$T" > "$T/bin/systemctl"
chmod +x "$T/bin/mount" "$T/bin/systemctl"
export BOARD_LIB_DIR=$lib BOARD_RUN_DIR=$T/run BOARD_DATA_DIR=$T/data BOARD_ETC_DIR=$T/etc \
	BOARD_DT_DIR=$T/dt BOARD_MACHINE_ID=$T/none BOARD_MACHINE_ID_FILE=$T/machine-id \
	BOARD_MOUNT=$T/bin/mount BOARD_SYSTEMCTL=$T/bin/systemctl
run() { sh "$lib/machine-id" 2> "$T/log" || fail "machine-id failed: $(cat "$T/log")"; }
restarts() { cat "$T/restarts" 2>/dev/null | wc -l | tr -d ' '; }

echo "a transient id is replaced by one from the board serial, and journald restarted"
printf '10000000a317bcbe\000' > "$T/dt/serial-number"
printf 'f00df00df00df00df00df00df00df00d\n' > "$T/machine-id"
run
derived=$(cat "$T/machine-id")
printf '%s' "$derived" | grep -q '^[0-9a-f]\{32\}$' || fail "not a machine id: $derived"
[ "$derived" != f00df00df00df00df00df00df00df00d ] || fail "the transient id stayed"
[ "$(restarts)" = 1 ] || fail "journald should restart once, did $(restarts)"
grep -q "restart systemd-journald.service" "$T/restarts" || fail "restart: $(cat "$T/restarts")"

echo "the next boot gets the same id"
printf '0123456789abcdef0123456789abcdef\n' > "$T/machine-id"
run
[ "$(cat "$T/machine-id")" = "$derived" ] || fail "the id changed between boots"

echo "already right: nothing bound, journald left alone"
: > "$T/restarts"
run
[ "$(restarts)" = 0 ] || fail "nothing to change, but journald restarted"

echo "a kept id wins over the derived one"
printf 'aaaabbbbccccddddeeeeffff00001111\n' > "$T/data/machine-id"
run
[ "$(cat "$T/machine-id")" = aaaabbbbccccddddeeeeffff00001111 ] || fail "kept id: $(cat "$T/machine-id")"
printf 'not an id\n' > "$T/data/machine-id"
run
[ "$(cat "$T/machine-id")" = "$derived" ] || fail "a bad kept id is ignored"

echo "no serial and nothing kept: left as it is"
rm "$T/dt/serial-number" "$T/data/machine-id"
printf 'f00df00df00df00df00df00df00df00d\n' > "$T/machine-id"
run
[ "$(cat "$T/machine-id")" = f00df00df00df00df00df00df00df00d ] || fail "should be left alone"

echo "ok"
