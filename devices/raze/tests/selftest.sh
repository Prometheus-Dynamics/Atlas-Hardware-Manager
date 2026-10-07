#!/bin/sh
# Off-device test of the self-test and the LED index offset: a fake sysfs,
# /dev and configfs, fake i2cdetect/systemctl/ip/media-ctl, and a backend that
# acts like the pwm-fan driver (a cooling state sets the duty).
# Run: sh devices/raze/tests/selftest.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
lib=$here/../gaia/assets/rootfs/usr/lib/board
T=$(mktemp -d "${TMPDIR:-/tmp}/board-selftest-test.XXXXXX")
trap 'rm -rf "$T"' EXIT

fail() {
	echo "FAIL: $*" >&2
	exit 1
}

S=$T/sys
mkdir -p "$T/dev" "$T/run" "$T/etc" "$T/data" "$T/dt" "$T/bin" "$T/net/usbbr0" "$T/cfg/usb_gadget/g1"
printf '10000000a317bcbe\000' > "$T/dt/serial-number"
: > "$T/dev/leds0"
: > "$T/dev/watchdog0"
: > "$T/dev/media0"
echo 1000480000.usb > "$T/cfg/usb_gadget/g1/UDC"
echo 0x1003 > "$T/net/usbbr0/flags"
echo up > "$T/net/usbbr0/operstate"

# Thermal: the fan's cooling device in state 1, a CPU zone driving it.
mkdir -p "$S/class/thermal/cooling_device0" "$S/class/thermal/cooling_device1" \
	"$S/class/thermal/thermal_zone0" "$S/class/hwmon/hwmon1" "$S/class/hwmon/hwmon2"
echo cpufreq-cpu0 > "$S/class/thermal/cooling_device0/type"
echo pwm-fan > "$S/class/thermal/cooling_device1/type"
echo 4 > "$S/class/thermal/cooling_device1/max_state"
echo 1 > "$S/class/thermal/cooling_device1/cur_state"
echo enabled > "$S/class/thermal/thermal_zone0/mode"
ln -s ../cooling_device1 "$S/class/thermal/thermal_zone0/cdev0"
echo 0 > "$S/class/thermal/thermal_zone0/cdev0_trip_point"
echo cpu_thermal > "$S/class/hwmon/hwmon1/name"
echo pwmfan > "$S/class/hwmon/hwmon2/name"
echo 212 > "$S/class/hwmon/hwmon2/pwm1"
mkdir -p "$S/class/watchdog/watchdog0"
echo bcm2835-wdt > "$S/class/watchdog/watchdog0/identity"

# I2C: buses 1 and 4, the camera bound on bus 10.
mkdir -p "$S/bus/i2c/devices/i2c-1" "$S/bus/i2c/devices/i2c-4" "$S/bus/i2c/devices/i2c-10" \
	"$S/bus/i2c/drivers/ov9282" "$S/bus/i2c/devices/10-0060" "$S/class/video4linux/video0"
echo ov9782 > "$S/bus/i2c/devices/10-0060/name"
ln -s ../../drivers/ov9282 "$S/bus/i2c/devices/10-0060/driver"
echo rp1-cfe-csi2_ch0 > "$S/class/video4linux/video0/name"

# Tools. i2cdetect answers for the addresses listed in $T/i2c ("bus addr").
printf '4 0x18\n4 0x68\n1 0x10\n1 0x40\n' > "$T/i2c"
cat > "$T/bin/i2cdetect" <<'EOF'
#!/bin/sh
# i2cdetect -y -r <bus> <first> <last>
bus=$3 addr=$(($4))
cell=--
grep -qx "$bus $(printf '0x%02x' "$addr")" "$I2C_TABLE" && cell=$(printf '%02x' "$addr")
printf '     0  1  2  3  4  5  6  7  8  9  a  b  c  d  e  f\n'
printf '%02x: %s\n' $((addr & 0xf0)) "$cell"
EOF
cat > "$T/bin/systemctl" <<'EOF'
#!/bin/sh
echo "${FAKE_WATCHDOG:-15s}"
EOF
cat > "$T/bin/ip" <<'EOF'
#!/bin/sh
echo "5: usbbr0    inet 172.31.250.1/24 brd 172.31.250.255 scope global usbbr0"
EOF
cat > "$T/bin/media-ctl" <<'EOF'
#!/bin/sh
printf 'Media controller API version 7.2.9\n\nMedia device information\n------------------------\ndriver          rp1-cfe\nmodel           rp1-cfe\n\n'
printf -- '- entity 1: csi2 (4 pads, 8 links)\n'
printf -- '- entity 20: ov9282 10-0060 (1 pad, 1 link)\n'
EOF
chmod +x "$T/bin/"*

# The backend: the real hw.sh, plus what the pwm-fan driver does when a
# cooling state is set (the duty follows; FAKE_FAN_LEVELS can break it), and
# a hook that interrupts the self-test mid-way.
cat > "$T/backend.sh" <<'EOF'
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

export PATH="$T/bin:$PATH"
export BOARD_LIB_DIR=$lib BOARD_ETC_DIR=$T/etc BOARD_DATA_DIR=$T/data BOARD_RUN_DIR=$T/run BOARD_DT_DIR=$T/dt
export BOARD_SYS_DIR=$S BOARD_DEV_DIR=$T/dev BOARD_CONFIGFS=$T/cfg BOARD_NET_DIR=$T/net
export BOARD_HW_BACKEND=$T/backend.sh BOARD_SELFTEST_ALLOW_USER=1 I2C_TABLE=$T/i2c
export SELFTEST_FAN_SETTLE=0 SELFTEST_LED_STEP=0 SELFTEST_LED_HOLD=0 BOARD_HW_MISSING=cam

selftest() { sh "$lib/selftest" "$@"; }
# check_status <report> <id> <ok|skip|fail>
check_status() {
	printf '%s' "$1" | grep -q "{\"id\":\"$2\",\"status\":\"$3\"" ||
		fail "$2 should be $3: $(printf '%s' "$1" | tr '{' '\n' | grep "\"id\":\"$2\"")"
}
valid_json() {
	if command -v python3 >/dev/null 2>&1; then
		printf '%s' "$1" | python3 -c 'import json, sys; r = json.load(sys.stdin); assert r["version"] == 1 and isinstance(r["checks"], list)' ||
			fail "not a valid report: $1"
	fi
}
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
printf '4 0x18\n1 0x10\n1 0x40\n' > "$T/i2c"
: > "$T/cfg/usb_gadget/g1/UDC"
out=$(FAKE_WATCHDOG=0 FAKE_FAN_LEVELS='76 43 10 0 0' selftest --json)
valid_json "$out"
printf '%s' "$out" | grep -q '"ok":false' || fail "should fail: $out"
for id in fan camera i2c watchdog gadget; do check_status "$out" "$id" fail; done
printf '%s' "$out" | grep -q 'BMI088 (imu-gyro) at 4/0x68: absent' || fail "the missing gyro should be named: $out"
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
