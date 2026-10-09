# shellcheck shell=sh
# A fake Raze for the off-device tests (sourced by selftest.sh and
# lemnosd.sh; not a test itself): sysfs, /dev, configfs and the device tree
# under $T, and fake i2cdetect, i2cget, i2cset, systemctl, ip and media-ctl on
# PATH. Needs $T (a scratch directory) and $lib (the package's
# usr/lib/board). Sets $S (the fake sysfs).

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

# I2C: the hardware bus i2c-1 (RP1 i2c@74000), the i2c-gpio bus i2c-4 and the
# camera bound on i2c-10. Adapters point at their device-tree nodes, as the
# kernel's of_node links do, so the buses' selectors find them.
base=$S/firmware/devicetree/base
mkdir -p "$base/axi/pcie@1000120000/rp1/i2c@74000" "$base/axi/pcie@1000120000/rp1/i2c@88000" "$base/i2c@0"
printf 'snps,designware-i2c\000' > "$base/axi/pcie@1000120000/rp1/i2c@74000/compatible"
printf 'snps,designware-i2c\000' > "$base/axi/pcie@1000120000/rp1/i2c@88000/compatible"
printf 'i2c-gpio\000' > "$base/i2c@0/compatible"
# fake_adapter <bus> <device-tree node under base>
fake_adapter() {
	mkdir -p "$S/bus/i2c/devices/i2c-$1"
	ln -sfn "$base/$2" "$S/bus/i2c/devices/i2c-$1/of_node"
}
fake_adapter 1 axi/pcie@1000120000/rp1/i2c@74000
fake_adapter 4 i2c@0
fake_adapter 10 axi/pcie@1000120000/rp1/i2c@88000
mkdir -p "$S/bus/i2c/drivers/ov9282" "$S/bus/i2c/devices/10-0060" "$S/class/video4linux/video0"
echo ov9782 > "$S/bus/i2c/devices/10-0060/name"
ln -s ../../drivers/ov9282 "$S/bus/i2c/devices/10-0060/driver"
echo rp1-cfe-csi2_ch0 > "$S/class/video4linux/video0/name"

# Tools. i2cdetect answers for the addresses listed in $T/i2c ("bus addr");
# i2cget and i2cset read and write registers in $T/regs ("bus addr reg
# value", words as an SMBus word read returns them) and log to $T/i2c.log.
printf '4 0x18\n4 0x68\n1 0x10\n1 0x40\n' > "$T/i2c"
printf '4 0x18 0x00 0x1e\n4 0x68 0x00 0x0f\n1 0x10 0x40 0x32\n1 0x10 0x4b 0x00\n1 0x40 0xfe 0x4954\n' > "$T/regs"
cat > "$T/bin/i2cdetect" <<'EOF'
#!/bin/sh
# i2cdetect -y -r <bus> <first> <last>
bus=$3 addr=$(($4))
cell=--
grep -qx "$bus $(printf '0x%02x' "$addr")" "$I2C_TABLE" && cell=$(printf '%02x' "$addr")
printf '     0  1  2  3  4  5  6  7  8  9  a  b  c  d  e  f\n'
printf '%02x: %s\n' $((addr & 0xf0)) "$cell"
EOF
cat > "$T/bin/i2cget" <<'EOF'
#!/bin/sh
# i2cget -y <bus> <addr> <reg> [b|w]. A BMM150 (0x10) reads 0 for its chip id
# while its power control bit (0x4b bit 0) is clear.
echo "i2cget $*" >> "$I2C_LOG"
bus=$2 addr=$(printf '0x%02x' $(($3))) reg=$(printf '0x%02x' $(($4)))
grep -qx "$bus $addr" "$I2C_TABLE" || { echo "Error: Read failed" >&2; exit 1; }
v=$(awk -v k="$bus $addr $reg" '$1 " " $2 " " $3 == k { print $4 }' "$I2C_REGS")
if [ "$addr" = 0x10 ] && [ "$reg" = 0x40 ]; then
	p=$(awk -v k="$bus $addr 0x4b" '$1 " " $2 " " $3 == k { print $4 }' "$I2C_REGS")
	[ $((p & 1)) = 1 ] || v=0x00
fi
echo "${v:-0x00}"
EOF
cat > "$T/bin/i2cset" <<'EOF'
#!/bin/sh
# i2cset -y <bus> <addr> <reg> <value> [b|w]
echo "i2cset $*" >> "$I2C_LOG"
bus=$2 addr=$(printf '0x%02x' $(($3))) reg=$(printf '0x%02x' $(($4))) v=$(printf '0x%02x' $(($5)))
awk -v k="$bus $addr $reg" -v v="$v" '$1 " " $2 " " $3 == k { $4 = v; hit = 1 } { print } END { if (!hit) print k " " v }' \
	"$I2C_REGS" > "$I2C_REGS.tmp" && mv "$I2C_REGS.tmp" "$I2C_REGS"
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

export PATH="$T/bin:$PATH"
export BOARD_LIB_DIR=$lib BOARD_ETC_DIR=$T/etc BOARD_DATA_DIR=$T/data BOARD_RUN_DIR=$T/run BOARD_DT_DIR=$T/dt
export BOARD_SYS_DIR=$S BOARD_DEV_DIR=$T/dev BOARD_CONFIGFS=$T/cfg BOARD_NET_DIR=$T/net
export BOARD_SELFTEST_ALLOW_USER=1 I2C_TABLE=$T/i2c I2C_REGS=$T/regs I2C_LOG=$T/i2c.log
export SELFTEST_FAN_SETTLE=0 SELFTEST_LED_STEP=0 SELFTEST_LED_HOLD=0 BOARD_HW_MISSING=cam

fail() {
	echo "FAIL: $*" >&2
	exit 1
}
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
# The data of check <id> in a report, as JSON.
check_data() {
	printf '%s' "$1" | python3 -c 'import json, sys; print(json.dumps(next(c["data"] for c in json.load(sys.stdin)["checks"] if c["id"] == sys.argv[1])))' "$2"
}
