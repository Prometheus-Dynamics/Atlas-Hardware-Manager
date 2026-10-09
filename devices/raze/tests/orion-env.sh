#!/bin/sh
# Off-device test of orion-env: the board's Orion node id, from the serial
# or the OS's own orion-node.env. Run: sh devices/raze/tests/orion-env.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
lib=$here/../gaia/assets/rootfs/usr/lib/board
T=$(mktemp -d "${TMPDIR:-/tmp}/board-orion-env-test.XXXXXX")
trap 'rm -rf "$T"' EXIT

fail() {
	echo "FAIL: $*" >&2
	exit 1
}

export BOARD_LIB_DIR=$lib BOARD_ETC_DIR=$T/etc BOARD_DATA_DIR=$T/data BOARD_RUN_DIR=$T/run \
	BOARD_DT_DIR=$T/dt BOARD_MACHINE_ID=$T/machine-id ORION_NODE_ENV=$T/orion-node.env
mkdir -p "$T/dt" "$T/etc"
printf '10000000a317bcbe\000' > "$T/dt/serial-number"
run() { sh "$lib/orion-env" 2> "$T/log" || fail "orion-env failed: $(cat "$T/log")"; }
env_is() { [ "$(cat "$T/run/orion.env")" = "$1" ] || fail "orion.env: $(cat "$T/run/orion.env"), want $1"; }

# No OS setting: the board's name, as Atlas shows it.
run
env_is 'ORION_NODE_ID=raze-a317bcbe'

# The OS's own id wins (the last assignment, quotes and export dropped).
printf '# ORION_NODE_ID=commented\nORION_NODE_ID=first\nexport ORION_NODE_ID="robot-cam-1"\n' > "$T/orion-node.env"
run
env_is 'ORION_NODE_ID=robot-cam-1'

# A commented-out example sets nothing.
printf '#ORION_NODE_ID=node.example\n' > "$T/orion-node.env"
run
env_is 'ORION_NODE_ID=raze-a317bcbe'

# No serial and no machine id: nothing written, orion-node keeps its own.
rm -f "$T/dt/serial-number" "$T/run/orion.env"
run
[ ! -e "$T/run/orion.env" ] || fail "wrote an id without a serial"

echo "orion-env: ok"
