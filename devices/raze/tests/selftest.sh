#!/bin/sh
# Off-device test of the self-test and the LED index offset with the sysfs
# backend: the fake board of fake-board.inc (sysfs, /dev, configfs, the
# device tree, fake i2cdetect/i2cget/i2cset/systemctl/ip/media-ctl) and a
# backend that acts like the pwm-fan driver (a cooling state sets the duty).
# lemnosd.sh tests the lemnosd backend.
# Run: sh devices/raze/tests/selftest.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
lib=$here/../gaia/assets/rootfs/usr/lib/board
T=$(mktemp -d "${TMPDIR:-/tmp}/board-selftest-test.XXXXXX")
trap 'rm -rf "$T"' EXIT
. "$here/fake-board.inc"

# The backend: the real hw.sh with the sysfs backend, plus what the pwm-fan
# driver does when a cooling state is set (the duty follows; FAKE_FAN_LEVELS
# can break it), and a hook that interrupts the self-test mid-way.
cat > "$T/backend.sh" <<'EOF'
BOARD_HW_BACKEND=sysfs
. "$BOARD_LIB_DIR/hw.sh"
hw_fan_set_state() {
	printf '%s\n' "$1" > "$(hw_fan_cdev)/cur_state"
	set -- "$1" ${FAKE_FAN_LEVELS:-$HW_FAN_LEVELS}
	shift $(($1 + 1))
	echo "$1" > "$BOARD_SYS_DIR/class/hwmon/hwmon2/pwm1"
	if [ "${FAKE_KILL_AT_STATE:-}" = "$(cat "$(hw_fan_cdev)/cur_state")" ]; then
		kill -TERM $$
	fi
}
EOF
export BOARD_HW_BACKEND=$T/backend.sh

leds_hex() { od -An -tx1 -v "$T/dev/leds0" | tr -d ' \n'; }
zeros() { i=0; while [ "$i" -lt "$1" ]; do printf 00; i=$((i + 1)); done; }

echo "a healthy board passes, as JSON"
out=$(selftest --json)
valid_json "$out"
printf '%s' "$out" | grep -q '"ok":true' || fail "should pass: $out"
printf '%s' "$out" | grep -q '"board_serial":"10000000a317bcbe"' || fail "board serial: $out"
for id in leds fan camera i2c watchdog gadget; do check_status "$out" "$id" ok; done
[ "$(cat "$T/run/selftest.json")" = "$out" ] || fail "the report should be written to /run/board/selftest.json"
printf '%s' "$out" | grep -q '"steps":\[{"state":0,"pwm":179,"expected":179' || fail "fan steps: $out"
[ ! -s "$T/dev/leds0" ] || fail "with no raze-leds state, no LED frame may be written"
printf '%s' "$out" | grep -q '"backend":"sysfs"' || fail "the report names the backend: $out"

echo "I2C: buses found by their selectors, chip ids read, the BMM150 woken and put back"
echo "    (the IMU's PIO bus only through lemnosd: here it can't be probed)"
i2c=$(check_data "$out" i2c)
printf '%s' "$i2c" | python3 -c '
import json, sys
d = {x["id"]: x for x in json.load(sys.stdin)["devices"]}
assert all(x["bus_found_by"] == "selector" for k, x in d.items() if not k.startswith("imu")), d
assert all(d[k]["bus_found_by"] == "pio" and d[k]["result"] == "unknown" for k in ("imu-accel", "imu-gyro")), d
assert d["magnetometer"]["chip_id"] == "ok 0x32", d
assert d["power-monitor"]["chip_id"] == "ok 0x4954", d
' || fail "i2c data: $i2c"
grep -q '^i2cset -y 1 0x10 0x4b 1 b$' "$T/i2c.log" || fail "the BMM150 power bit should be set for the read: $(cat "$T/i2c.log")"
grep -q 'pio-i2c\| 4 0x' "$T/i2c.log" && fail "no i2c-tools reads on the PIO bus: $(cat "$T/i2c.log")"
check_message "$out" i2c | grep -q "2 can't be probed (no i2cdetect, or a PIO bus without lemnosd)" ||
	fail "the message says why: $(check_message "$out" i2c)"
grep -q '^1 0x10 0x4b 0x00$' "$T/regs" || fail "the BMM150 power control should be put back: $(cat "$T/regs")"

echo "I2C: a renumbered bus is found by its selector, a wrong chip id fails"
mv "$S/bus/i2c/devices/i2c-1" "$S/bus/i2c/devices/i2c-3"
sed -i.bak 's/^1 /3 /' "$T/i2c" "$T/regs"
out=$(selftest --json)
check_status "$out" i2c ok
check_data "$out" i2c | grep -q '"id": "power-monitor", "part": "INA238", "bus": 3, "bus_hint": 1, "bus_found_by": "selector"' ||
	fail "the hardware bus should be found as i2c-3: $(check_data "$out" i2c)"
sed -i.bak 's/^3 0x40 0xfe 0x4954$/3 0x40 0xfe 0x4955/' "$T/regs"
out=$(selftest --json)
check_status "$out" i2c fail
printf '%s' "$out" | grep -q 'INA238 (power-monitor) at 3/0x40: chip id 0x4955' || fail "the wrong chip id should be named: $out"
mv "$S/bus/i2c/devices/i2c-3" "$S/bus/i2c/devices/i2c-1"
sed -i.bak 's/^3 /1 /; s/^1 0x40 0xfe 0x4955$/1 0x40 0xfe 0x4954/' "$T/i2c" "$T/regs"

echo "the fan is handed back: state and governor as before"
[ "$(cat "$S/class/thermal/cooling_device1/cur_state")" = 1 ] || fail "cur_state not restored"
[ "$(cat "$S/class/thermal/thermal_zone0/mode")" = enabled ] || fail "thermal zone left disabled"
[ ! -d "$T/run/selftest.lock" ] || fail "the lock was left behind"

echo "with raze-leds state, the ring's frame is redrawn as it was"
printf 'ON=1 R=255 G=0 B=0 W=0 BRIGHT=100\n' > "$T/run/leds.state"
out=$(selftest --json)
check_status "$out" leds ok
want=$(i=0; while [ "$i" -lt 16 ]; do printf ff000000; i=$((i + 1)); done)
[ "$(leds_hex)" = "$want" ] || fail "LED frame: $(leds_hex)"

echo "the LED index offset follows the manifest (5) and its settings"
sh "$lib/raze-leds" pixel 0 255 0 0
[ "$(leds_hex)" = "$(zeros 20)ff000000$(zeros 40)" ] || fail "LED 0 should be slot 5: $(leds_hex)"
sh "$lib/raze-leds" pixel 15 0 0 255
[ "$(leds_hex)" = "$(zeros 16)0000ff00$(zeros 44)" ] || fail "LED 15 should be slot 4: $(leds_hex)"
RAZE_LEDS_OFFSET=0 sh "$lib/raze-leds" pixel 0 255 0 0
[ "$(leds_hex)" = "ff000000$(zeros 60)" ] || fail "offset 0: $(leds_hex)"
printf 'RAZE_LEDS_DIRECTION=-1\n' > "$T/etc/raze-leds.env"
sh "$lib/raze-leds" pixel 1 0 255 0
[ "$(leds_hex)" = "$(zeros 16)00ff0000$(zeros 44)" ] || fail "direction -1: LED 1 should be slot 4: $(leds_hex)"
rm "$T/etc/raze-leds.env"
sh "$lib/raze-leds" show | grep -q 'color=255,0,0,0' || fail "pixel must not change the saved state"
sh "$lib/raze-leds" refresh
[ "$(leds_hex)" = "$want" ] || fail "refresh should redraw the state: $(leds_hex)"

echo "interactive: the operator's answers are recorded and the ring is put back"
printf 'y\ny\ny\ny\ny\ny\n' > "$T/answers"
out=$(SELFTEST_TTY_IN=$T/answers SELFTEST_TTY_OUT=$T/prompts selftest --interactive --json)
valid_json "$out"
check_status "$out" leds ok
check_status "$out" fan ok
printf '%s' "$out" | grep -q '"answers":{"red":"yes","green":"yes","blue":"yes","w":"yes","walk":"yes"}' || fail "answers: $out"
grep -q 'Is the whole ring dark' "$T/prompts" || fail "the W step should expect a dark ring"
[ "$(leds_hex)" = "$want" ] || fail "the ring should be back to its state: $(leds_hex)"
printf 'y\nn\ny\ny\ny\ny\n' > "$T/answers"
out=$(SELFTEST_TTY_IN=$T/answers SELFTEST_TTY_OUT=$T/prompts selftest --interactive --json)
check_status "$out" leds fail

echo "broken hardware fails, and the text report exits 1"
rm "$S/bus/i2c/devices/10-0060/driver"
printf '1 0x10\n' > "$T/i2c"
: > "$T/cfg/usb_gadget/g1/UDC"
out=$(FAKE_WATCHDOG=0 FAKE_FAN_LEVELS='76 43 10 0 0' selftest --json)
valid_json "$out"
printf '%s' "$out" | grep -q '"ok":false' || fail "should fail: $out"
for id in fan camera i2c watchdog gadget; do check_status "$out" "$id" fail; done
printf '%s' "$out" | grep -q 'INA238 (power-monitor) at 1/0x40: absent' || fail "the missing power monitor should be named: $out"
if FAKE_WATCHDOG=0 selftest > "$T/text"; then fail "a failed check should exit 1"; fi
grep -q '^fail  camera' "$T/text" || fail "text report: $(cat "$T/text")"
[ "$(cat "$S/class/thermal/cooling_device1/cur_state")" = 1 ] || fail "cur_state not restored after failures"

echo "without i2cdetect the scan is skipped"
out=$(BOARD_HW_MISSING='cam i2cdetect' selftest --json)
check_status "$out" i2c skip

echo "an interrupted self-test still hands the fan back"
if FAKE_KILL_AT_STATE=3 selftest --json > /dev/null 2>&1; then fail "the interrupted run should not succeed"; fi
[ "$(cat "$S/class/thermal/cooling_device1/cur_state")" = 1 ] || fail "cur_state not restored after an interrupt"
[ "$(cat "$S/class/thermal/thermal_zone0/mode")" = enabled ] || fail "thermal zone left disabled after an interrupt"
[ ! -d "$T/run/selftest.lock" ] || fail "the lock was left behind after an interrupt"

echo "one at a time, and root only"
mkdir "$T/run/selftest.lock"
if selftest --json > /dev/null 2>&1; then fail "a second self-test should be refused"; fi
rmdir "$T/run/selftest.lock"
if [ "$(id -u)" != 0 ]; then
	if BOARD_SELFTEST_ALLOW_USER=0 selftest --json > /dev/null 2>&1; then fail "it should refuse to run as a user"; fi
fi

echo "the identity lists the self-test"
. "$lib/lib.sh"
board_identity_json | grep -q '"diagnostics":\["selftest"\]' || fail "identity: $(board_identity_json)"

echo "ok"
