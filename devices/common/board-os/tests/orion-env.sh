#!/bin/sh
# Off-device test of orion-env: the board's Orion node id, from the serial
# or the OS's own orion-node.env, and orion-node's access from the security
# switch. Run: sh devices/common/board-os/tests/orion-env.sh
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
env_is 'ORION_NODE_ID=raze-bcbe'

# The OS's own id wins (the last assignment, quotes and export dropped).
printf '# ORION_NODE_ID=commented\nORION_NODE_ID=first\nexport ORION_NODE_ID="robot-cam-1"\n' > "$T/orion-node.env"
run
env_is 'ORION_NODE_ID=robot-cam-1'

# Orion's placeholder isn't an id: every board would share it.
printf 'ORION_NODE_ID=node-local\n' > "$T/orion-node.env"
run
env_is 'ORION_NODE_ID=raze-bcbe'

# A commented-out example sets nothing.
printf '#ORION_NODE_ID=node.example\n' > "$T/orion-node.env"
run
env_is 'ORION_NODE_ID=raze-bcbe'

# No serial and no machine id: nothing written, orion-node keeps its own.
rm -f "$T/dt/serial-number" "$T/run/orion.env"
run
[ ! -e "$T/run/orion.env" ] || fail "wrote an id without a serial"

# orion-node's access, switch off (the default): listening, open to read,
# the USB cable's network enrolled for what Atlas does, never usb-boot.
printf '10000000a317bcbe\000' > "$T/dt/serial-number"
: > "$T/orion-node.env"
access() { grep -v '^#' "$T/run/orion-node.env"; }
run
access | grep -qx 'ORION_NODE_PEER_ADDR=0.0.0.0:9200' || fail "no listener: $(access)"
access | grep -qx 'ORION_NODE_PEER_AUTH=optional' || fail "auth: $(access)"
access | grep -qx 'ORION_NODE_OPERATOR_OPEN_READ=status,streams' || fail "open read: $(access)"
access | grep -Eqx 'ORION_NODE_OPERATOR_AUTO_ENROLL=[0-9.]+/[0-9]+=update\*,slot\.\*,reboot,locate,clock\.set,set,restore,release,power\.\*,calibration\.\*,looks\.\*,light\.\*,usb-route\.set;10\.0\.0\.0/8=update\*,slot\.\*,reboot,locate,clock\.set,set,restore,release,power\.\*,calibration\.\*,looks\.\*,light\.\*,usb-route\.set' ||
	fail "auto-enroll: $(access)"

# The network, not the board's own address in it.
echo 'USB_GADGET_ADDRESS=172.31.12.81/29' > "$T/etc/usb-gadget.env"
run
access | grep -Fqx 'ORION_NODE_OPERATOR_AUTO_ENROLL=172.31.12.80/29=update*,slot.*,reboot,locate,clock.set,set,restore,release,power.*,calibration.*,looks.*,light.*,usb-route.set;10.0.0.0/8=update*,slot.*,reboot,locate,clock.set,set,restore,release,power.*,calibration.*,looks.*,light.*,usb-route.set' ||
	fail "auto-enroll network: $(access)"
! access | grep -q 'usb-boot' || fail "usb-boot by network: $(access)"
rm -f "$T/etc/usb-gadget.env"

# The robot networks are a setting: none, or others (a bad one is left out).
ORION_NODE_ROBOT_NETS='' sh "$lib/orion-env" 2> "$T/log" || fail "orion-env: $(cat "$T/log")"
access | grep -Eqx 'ORION_NODE_OPERATOR_AUTO_ENROLL=[0-9.]+/[0-9]+=update\*,slot\.\*,reboot,locate,clock\.set,set,restore,release,power\.\*,calibration\.\*,looks\.\*,light\.\*,usb-route\.set' ||
	fail "no robot networks: $(access)"
ORION_NODE_ROBOT_NETS='192.168.1.0/24 nonsense 10.12.34.0' sh "$lib/orion-env" 2> "$T/log" || fail "orion-env: $(cat "$T/log")"
access | grep -Eq ';192\.168\.1\.0/24=update\*,slot\.\*,reboot,locate,clock\.set,set,restore,release,power\.\*,calibration\.\*,looks\.\*,light\.\*,usb-route\.set$' || fail "robot networks: $(access)"
! access | grep -q 'nonsense\|10\.12\.34' || fail "a bad robot network went in: $(access)"

# The OS's own listener address is kept.
printf 'ORION_NODE_PEER_ADDR=10.0.0.2:9300\n' > "$T/orion-node.env"
run
! access | grep -q PEER_ADDR || fail "overrode the OS's address: $(access)"
: > "$T/orion-node.env"

# Switch on: auth required, nothing open, nobody enrolled by network.
mkdir -p "$T/data" "$T/bin"
echo BOARD_SECURITY=1 > "$T/data/security.env"
printf '#!/bin/sh\necho "$*" >> %s/systemctl.calls\n' "$T" > "$T/bin/systemctl"
chmod +x "$T/bin/systemctl"
PATH=$T/bin:$PATH sh "$lib/orion-env" --restart 2> "$T/log" || fail "orion-env --restart: $(cat "$T/log")"
access | grep -qx 'ORION_NODE_PEER_AUTH=required' || fail "auth on: $(access)"
access | grep -qx 'ORION_NODE_OPERATOR_OPEN_READ=' || fail "open read on: $(access)"
access | grep -qx 'ORION_NODE_OPERATOR_AUTO_ENROLL=' || fail "auto-enroll on: $(access)"
[ "$(cat "$T/systemctl.calls")" = 'try-restart orion-node.service' ] || fail "no restart on a change"

# Nothing changed: no restart.
PATH=$T/bin:$PATH sh "$lib/orion-env" --restart 2> "$T/log" || fail "orion-env --restart: $(cat "$T/log")"
[ "$(wc -l < "$T/systemctl.calls")" -eq 1 ] || fail "restarted without a change"

# The host keys' public halves and the clock flag, by default and when the OS sets its own.
: > "$T/orion-node.env"
run
grep -qx "ORION_NODE_SSH_HOST_KEYS=$T/run/ssh-public" "$T/run/orion-node.env" || fail "no host-key directory"
grep -qx "ORION_NODE_TIME_SYNCED_FILE=$T/run/orion-time/synced" "$T/run/orion-node.env" || fail "no clock flag"
[ "$(stat -c %a "$T/run/orion-time")" = 755 ] || fail "the clock flag's directory isn't readable"
printf 'ORION_NODE_SSH_HOST_KEYS=/elsewhere\nORION_NODE_TIME_SYNCED_FILE=/elsewhere/flag\n' > "$T/orion-node.env"
run
if grep -q 'ORION_NODE_SSH_HOST_KEYS\|ORION_NODE_TIME_SYNCED_FILE' "$T/run/orion-node.env"; then fail "the OS's own settings overridden"; fi

echo "orion-env: ok"
