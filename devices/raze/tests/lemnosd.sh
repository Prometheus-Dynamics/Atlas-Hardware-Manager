#!/bin/sh
# Off-device test of the lemnosd backend: raze-leds and the self-test as
# lemnosd clients, against a fake lemnos-ctl that records its calls and acts
# like lemnosd (devices and their status, the fan's duty and writers, the fan
# hand-back), on the fake board of fake-board.inc.
# Run: sh devices/raze/tests/lemnosd.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
lib=$here/../gaia/assets/rootfs/usr/lib/board
T=$(mktemp -d "${TMPDIR:-/tmp}/board-lemnosd-test.XXXXXX")
trap 'rm -rf "$T"' EXIT
. "$here/fake-board.inc"

# lemnosd's devices (board.toml ids) and their status.
cat > "$T/lemnos-devices" <<'EOF'
status-ring light available
fan fan available
cpu-thermal temperature available
imu imu available
magnetometer magnetometer available
power power-monitor available
usb-a-power gpio missing
EOF
echo 0.831373 > "$T/fan-duty"
cat > "$T/bin/lemnos-ctl" <<'EOF'
#!/bin/sh
# A fake lemnos-ctl: logs "<client> <args>" to $CTL_LOG and answers like
# lemnosd. $T/lemnosd-down: the socket doesn't answer.
client=lemnos-ctl
args=''
while [ $# -gt 0 ]; do
	case "$1" in
	--client) client=$2; shift 2 ;;
	--priority) args="$args --priority $2"; shift 2 ;;
	*) args="$args $1"; shift ;;
	esac
done
echo "$client$args" >> "$CTL_LOG"
set -- $args
[ "$1" != --priority ] || shift 2
case "$1:${2:-}" in
fan:restore)
	[ ! -e "$FAKE/restore-fails" ] || { echo "lemnos-ctl: cur_state: Permission denied" >&2; exit 1; }
	echo restored > "$FAKE/fan-restored"
	echo "restored 1 fan(s)"
	exit 0
	;;
esac
if [ -e "$FAKE/lemnosd-down" ]; then
	echo "lemnos-ctl: connect /run/lemnos/lemnosd.sock: No such file or directory" >&2
	exit 1
fi
case "$1" in
list)
	while read -r id class status; do
		printf '%-16s %-14s %-12s %-10s channels: [] controls: []\n' "$id" "$class" "-" "$status"
	done < "$FAKE/lemnos-devices"
	;;
read)
	[ "$2" = fan ] || { echo "lemnos-ctl: unknown device" >&2; exit 1; }
	printf 'fan        12345us available speed=- duty=%s pwm_mode=1.000000\n' "$(cat "$FAKE/fan-duty")"
	;;
set)
	case " lemnosd.fan helios board-selftest " in
	*" $client "*) ;;
	*) echo "lemnos-ctl: refused: not-allowed" >&2; exit 1 ;;
	esac
	echo "$4" > "$FAKE/fan-duty"
	echo "$2 $3 = $4"
	;;
led) ;;
*) echo "lemnos-ctl: unknown command $1" >&2; exit 1 ;;
esac
EOF
chmod +x "$T/bin/lemnos-ctl"
export CTL_LOG=$T/ctl.log FAKE=$T
unset BOARD_HW_BACKEND

leds() { sh "$lib/raze-leds" "$@"; }
# expect <log line>...: the calls since the last expect, exactly.
expect() {
	got=$(cat "$CTL_LOG" 2>/dev/null || true)
	want=$(printf '%s\n' "$@")
	[ "$got" = "$want" ] || fail "lemnos-ctl calls:
$got
expected:
$want"
	: > "$CTL_LOG"
}

echo "with lemnos-ctl installed, raze-leds is a lemnosd client"
leds color 255 0 0
expect "raze-leds led color ff0000 --brightness 1.00 --device status-ring"
[ ! -s "$T/dev/leds0" ] || fail "nothing may be written to /dev/leds0 with lemnosd"
leds dim 50
expect "raze-leds led color ff0000 --brightness 0.50 --device status-ring"
leds status warn
expect "raze-leds led off --device status-ring" "raze-leds led status warn --brightness 0.50 --device status-ring"
leds status warn
expect "raze-leds led status warn --brightness 0.50 --device status-ring"
leds blink 4
expect "raze-leds led status warn --brightness 0.50 --blink --period 250 --device status-ring"
leds show | grep -qx 'on=1 color=255,160,0,0 brightness=50% blinking=yes' || fail "show: $(leds show)"
leds color 0 0 255
expect "raze-leds led off --device status-ring" "raze-leds led color 0000ff --brightness 0.50 --device status-ring"
leds show | grep -q 'blinking=no' || fail "a colour stops the blink: $(leds show)"
leds pixel 2 0 255 0
expect "raze-leds led frame 000000,000000,00ff00,000000,000000,000000,000000,000000,000000,000000,000000,000000,000000,000000,000000,000000 --fade 0 --device status-ring"
leds pixel all 1 2 3
expect "raze-leds led color 010203 --fade 0 --device status-ring"
leds show | grep -q 'color=0,0,255,0' || fail "pixel must not change the saved state"
leds refresh
expect "raze-leds led color 0000ff --brightness 0.50 --device status-ring"
leds locate 3
expect "raze-leds led locate --seconds 3 --device status-ring"
leds set 0
expect "raze-leds led off --device status-ring"
leds status off
expect "raze-leds led off --device status-ring"
RAZE_LEDS_CLIENT=photonvision leds on
expect "photonvision led color 0000ff --brightness 0.50 --device status-ring"

echo "BOARD_HW_BACKEND=sysfs writes frames itself, even with lemnos-ctl"
BOARD_HW_BACKEND=sysfs leds color 255 0 0
[ -s "$T/dev/leds0" ] || fail "the sysfs backend should write a frame"
expect
: > "$T/dev/leds0"
printf 'BOARD_HW_BACKEND=sysfs\n' > "$T/etc/hw.env"
leds off
expect
BOARD_HW_BACKEND=lemnosd leds off
expect "raze-leds led off --device status-ring"
rm "$T/etc/hw.env"
: > "$T/dev/leds0"

echo "the self-test goes through lemnosd: ring, fan duties and hand-back, sensors"
rm -f "$T/run/leds.state"
out=$(selftest --json)
valid_json "$out"
printf '%s' "$out" | grep -q '"backend":"lemnosd"' || fail "the report names the backend: $out"
for id in leds fan camera i2c watchdog gadget; do check_status "$out" "$id" ok; done
printf '%s' "$out" | grep -q 'lemnosd has the ring (status-ring on /dev/leds0) available' || fail "leds: $out"
printf '%s' "$out" | grep -q '"steps":\[{"state":0,"pwm":179,"expected":179,"rpm":null},{"state":1,"pwm":212' || fail "fan steps: $out"
printf '%s' "$out" | grep -q '"handed_back":true' || fail "the fan should be handed back: $out"
grep -q '^board-selftest set fan duty 0.7020$' "$CTL_LOG" || fail "duties go through lemnosd: $(cat "$CTL_LOG")"
grep -q '^board-selftest set fan duty 1.0000$' "$CTL_LOG" || fail "the top state is full duty: $(cat "$CTL_LOG")"
grep -e ' set fan ' -e ' fan restore$' "$CTL_LOG" | tail -n 1 | grep -q '^board-selftest fan restore$' ||
	fail "the hand-back ends the fan check: $(cat "$CTL_LOG")"
[ -e "$T/fan-restored" ] || fail "lemnos-ctl fan restore should have run"
[ "$(cat "$S/class/thermal/thermal_zone0/mode")" = enabled ] || fail "nothing is paused with lemnosd"
[ ! -s "$T/dev/leds0" ] || fail "no frame may be written to /dev/leds0"
check_data "$out" i2c | grep -q '"result": "service", "chip_id": "service", "lemnosd": "available"' ||
	fail "lemnosd's devices count as checked: $(check_data "$out" i2c)"
[ ! -s "$T/i2c.log" ] || fail "no I2C reads while lemnosd has the sensors: $(cat "$T/i2c.log")"
: > "$CTL_LOG"

echo "a sensor lemnosd lacks is probed, and the BMM150 is not woken behind lemnosd's back"
sed -i.bak 's/^magnetometer magnetometer available$/magnetometer magnetometer missing/' "$T/lemnos-devices"
out=$(selftest --json)
check_status "$out" i2c ok
check_data "$out" i2c | grep -q '"result": "present", "chip_id": "suspended", "lemnosd": "missing"' ||
	fail "magnetometer: $(check_data "$out" i2c)"
! grep -q i2cset "$T/i2c.log" || fail "no i2cset with lemnosd: $(cat "$T/i2c.log")"
mv "$T/lemnos-devices.bak" "$T/lemnos-devices"
: > "$CTL_LOG"

echo "interactive: test frames as board-selftest above other clients, then dropped"
printf 'y\ny\ny\ny\ny\ny\n' > "$T/answers"
out=$(SELFTEST_TTY_IN=$T/answers SELFTEST_TTY_OUT=$T/prompts selftest --interactive --json)
check_status "$out" leds ok
grep -q '^board-selftest --priority 100 led color ff0000 --fade 0 --device status-ring$' "$CTL_LOG" || fail "red: $(cat "$CTL_LOG")"
grep -q '^board-selftest --priority 100 led color 000000 --fade 0 --device status-ring$' "$CTL_LOG" || fail "the W step is dark: $(cat "$CTL_LOG")"
grep -q '^board-selftest --priority 100 led frame 00ff00,000000,' "$CTL_LOG" || fail "LED 0 green: $(cat "$CTL_LOG")"
grep '^board-selftest --priority 100 led ' "$CTL_LOG" | tail -n 1 | grep -q ' led off --device status-ring$' ||
	fail "the self-test's intents should be dropped at the end: $(cat "$CTL_LOG")"
: > "$CTL_LOG"

echo "lemnosd down: the ring and the fan fail and say why; raze-leds fails"
touch "$T/lemnosd-down"
out=$(selftest --json)
check_status "$out" leds fail
check_status "$out" fan fail
printf '%s' "$out" | grep -q "lemnosd doesn't answer" || fail "the reason: $out"
if leds on 2> "$T/err"; then fail "raze-leds should fail without lemnosd"; fi
grep -q 'is lemnosd.service running' "$T/err" || fail "raze-leds' message: $(cat "$T/err")"
rm "$T/lemnosd-down"

echo "a failed hand-back fails the fan check"
touch "$T/restore-fails"
out=$(selftest --json)
check_status "$out" fan fail
printf '%s' "$out" | grep -q 'the hand-back to the thermal governor failed' || fail "fan: $out"
rm "$T/restore-fails"

echo "ok"
