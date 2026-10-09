#!/bin/sh
# The files generated from manifest.json match it, and the lint catches a
# manifest edit that wasn't regenerated or breaks a rule.
# Needs python3. Run: sh devices/raze/tests/manifest-lint.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../.." && pwd)
T=$(mktemp -d "${TMPDIR:-/tmp}/board-manifest-lint.XXXXXX")
trap 'rm -rf "$T"' EXIT

fail() {
	echo "FAIL: $*" >&2
	exit 1
}

echo "the committed files match the manifest"
python3 "$root/devices/tools/gen-raze.py" --check || fail "run devices/tools/gen-raze.py and commit the result"

# A scratch copy of what the tool reads and writes.
mkdir -p "$T/devices"
cp -R "$root/devices/raze" "$root/devices/tools" "$root/devices/schema" "$root/devices/README.md" "$T/devices/"
gen() { python3 "$T/devices/tools/gen-raze.py" "$@"; }
edit() {
	python3 - "$T/devices/raze/manifest.json" "$1" <<'EOF'
import json, sys
path, change = sys.argv[1], sys.argv[2]
m = json.load(open(path))
exec(change, {"m": m, "c": m["capabilities"]})
json.dump(m, open(path, "w"), indent=2)
EOF
}

echo "a changed fact is caught, with a diff, and regenerating fixes it"
edit 'c["fan"]["cooling_levels"] = [128, 212, 245, 255, 255]; c["fan"]["min_level"] = 128'
if gen --check > "$T/diff" 2>/dev/null; then fail "--check should fail after a manifest change"; fi
grep -q '^+.*cooling-levels = <128 212 245 255 255>;' "$T/diff" || fail "the diff should show the new levels: $(cat "$T/diff")"
gen > /dev/null
gen --check > /dev/null || fail "--check should pass after regenerating"
grep -q 'HW_FAN_LEVELS=.128 212 245 255 255.' "$T/devices/raze/gaia/assets/rootfs/usr/lib/board/hardware.env" ||
	fail "hardware.env should follow the manifest"

echo "a hand edit of a generated region is caught"
sed -i.bak 's/num_leds=16/num_leds=12/' "$T/devices/raze/gaia/assets/boot/raze-device.txt"
if gen --check > /dev/null 2>&1; then fail "--check should catch a hand edit"; fi
gen > /dev/null

echo "the LED overlay passes rgbw only for 32-bit rings"
edit 'c["leds"]["wire_format"] = "grbw32"; c["leds"]["rgbw"] = True'
gen > /dev/null
grep -q '^dtoverlay=ws2812-pio,gpio=13,num_leds=16,brightness=255,rgbw,dev_name=leds%d$' \
	"$T/devices/raze/gaia/assets/boot/raze-device.txt" || fail "a 32-bit ring needs rgbw"
edit 'c["leds"]["wire_format"] = "grb24"; c["leds"]["rgbw"] = False'
gen > /dev/null

echo "board.toml follows the manifest: ring, fan, sensors on their bus selectors"
board=$T/devices/raze/gaia/assets/rootfs/etc/lemnos/board.toml
python3 - "$board" <<'EOF' || fail "board.toml: see above"
import sys, tomllib
b = tomllib.load(open(sys.argv[1], "rb"))
d = {x["id"]: x for x in b["devices"]}
assert b["format"] == "lemnos.board" and b["schema_version"] == 1 and b["board"]["id"] == "raze"
ring = d["status-ring"]["config"]
assert (ring["count"], ring["wire"], ring["offset"], ring["direction"]) == (16, "rgb", 5, "cw"), ring
assert (ring["fade_ms"], ring["easing"], ring["status_effect"]) == (250, "ease-in-out", "breathe"), ring
assert d["fan"]["match"] == {"name": "pwmfan"} and "config" not in d["fan"], d["fan"]
assert d["cpu-thermal"]["match"] == {"type": "cpu-thermal"}
assert (d["imu"]["bus"], d["imu"]["address"], d["imu"]["config"]["gyro_address"]) == ("i2c:compatible=i2c-gpio", 0x18, 0x68)
for s, a in (("magnetometer", 0x10), ("power", 0x40)):
    assert (d[s]["bus"], d[s]["address"]) == ("i2c:of=/axi/pcie@1000120000/rp1/i2c@74000", a), d[s]
assert d["power"]["config"] == {"shunt_micro_ohms": 10000, "max_current_micro_amps": 16384000}
assert d["usb-a-power"]["config"] == {"chip": "pinctrl-rp1", "line": 20, "initial": True}
EOF
edit 'c["leds"]["index"]["direction"] = -1'
gen > /dev/null
grep -q 'direction = "ccw"' "$board" || fail "direction -1 should be ccw"
edit 'c["leds"]["index"]["direction"] = 1; c["i2c"]["buses"][1]["select"] = {"name": "i2c-gpio-1"}'
gen > /dev/null
grep -q 'bus = "i2c:name=i2c-gpio-1"' "$board" || fail "the bus selector should follow the manifest"
edit 'del c["i2c"]["buses"][1]["select"]; del c["i2c"]["verified"]["buses.1.select"]'
gen > /dev/null
grep -q 'bus = "i2c-4"' "$board" || fail "without a selector the bus is i2c-<n>"
cp "$root/devices/raze/manifest.json" "$T/devices/raze/manifest.json"
gen > /dev/null

echo "the board.toml checks catch what Lemnos would refuse"
python3 - "$T/devices/tools" "$T/devices/schema/lemnos-board.schema.json" <<'EOF' || fail "the validator: see above"
import json, sys
sys.path.insert(0, sys.argv[1])
import raze_lemnos as rl
schema = json.load(open(sys.argv[2]))
good = {"format": "lemnos.board", "schema_version": 1, "board": {"id": "raze"}, "devices": [
    {"id": "imu", "driver": "bmi088", "bus": "i2c:compatible=i2c-gpio", "address": 24},
    {"id": "fan", "driver": "hwmon-fan", "match": {"name": "pwmfan"}}]}
assert not rl.schema_errors(good, schema, schema) and not rl.driver_errors(good)
for bad in (
    {"format": "lemnos.board", "schema_version": 2, "board": {"id": "raze"}},
    {"format": "lemnos.board", "schema_version": 1, "board": {"id": "Raze"}},
    {"format": "lemnos.board", "schema_version": 1, "board": {"id": "raze"}, "extra": 1},
    {"format": "lemnos.board", "schema_version": 1, "board": {"id": "raze"}, "devices": [{"id": "x", "driver": "bmi088", "bus": "i2c:color=red"}]},
    {"format": "lemnos.board", "schema_version": 1, "board": {"id": "raze"}, "devices": [{"id": "x", "driver": "bmi088", "bus": "i2c-1", "address": 200}]},
    {"format": "lemnos.board", "schema_version": 1, "board": {"id": "raze"}, "devices": [{"id": "x", "driver": "bmi088", "bus": "i2c-1", "config": {"x": {}}}]},
):
    assert rl.schema_errors(bad, schema, schema), bad
for bad in (
    [{"id": "x", "driver": "bmi088", "bus": "i2c-1", "config": {"gyro": 1}}],
    [{"id": "x", "driver": "hwmon-fan", "bus": "i2c-1"}],
    [{"id": "x", "driver": "hwmon-fan", "match": {"type": "x"}}],
    [{"id": "x", "driver": "ina238", "bus": "i2c-1", "config": {"shunt_micro_ohms": 10000}}],
    [{"id": "x", "driver": "ina238", "bus": "i2c-1", "config": {"shunt_micro_ohms": 10000, "max_current_micro_amps": 20000000}}],
    [{"id": "x", "driver": "ws2812", "config": {"count": 16, "wire": "grb"}}],
    [{"id": "x", "driver": "nope"}],
    [{"id": "x", "driver": "thermal-zone"}, {"id": "x", "driver": "thermal-zone"}],
):
    assert rl.driver_errors({"devices": bad}), bad
EOF

echo "rule breaks are refused"
for change in \
	'm["kernel"]["defconfig"] = "bcm2711"' \
	'm["kernel"]["commit"] = "0" * 40' \
	'm["kernel"]["builtin"] = ["btrfs"]' \
	'm["kernel"]["page_size_kib"] = 8' \
	'del m["kernel"]["verified"]["page_size_kib"]' \
	'c["leds"]["rgbw"] = True' \
	'c["leds"]["index"]["offset"] = 16' \
	'c["leds"]["index"]["direction"] = 2' \
	'c["leds"]["userspace"]["layout"] = "rgb"' \
	'c["fan"]["pwm"]["polarity"] = "inverted"' \
	'c["fan"]["cooling_levels"] = [255, 212, 245, 255, 255]' \
	'c["fan"]["min_level"] = 100' \
	'c["fan"]["trips_c"] = [50, 60, 75]' \
	'c["camera"]["i2c_address"] = 97' \
	'c["i2c"]["devices"][0]["address"] = "0x18"' \
	'c["i2c"]["devices"][0]["bus"] = 7' \
	'c["i2c"]["devices"][1]["id"] = c["i2c"]["devices"][0]["id"]' \
	'del c["leds"]["verified"]["index.offset"]' \
	'c["fan"]["verified"]["pwm.polarity"] = {"by": "x", "date": "yesterday", "method": "y"}' \
	'c["fan"]["verified"]["no.such.fact"] = "unverified"' \
	'c["i2c"]["buses"][0]["select"] = {"of": "/a b"}' \
	'c["i2c"]["buses"][0]["select"] = {"path": "/axi"}' \
	'c["i2c"]["devices"][3]["chip_id"]["smbus_word"] = 0x5449' \
	'c["i2c"]["devices"][1]["lemnosd"]["driver"] = "bmi088"' \
	'del c["i2c"]["devices"][3]["max_current_a"]' \
	'c["i2c"]["devices"][3]["max_current_a"] = 20.0' \
	'c["fan"]["lemnosd"]["writers"] = ["helios"]' \
	'del c["leds"]["lemnosd"]' \
	'c["leds"]["lemnosd"]["look"]["easing"] = "bouncy"' \
	'c["hardware-service"]["gaia_import"]["rev"] = "cd72ad0"' \
	'c["hardware-service"]["gaia_import"]["rev"] = "0" * 40' \
	'c["hardware-service"]["update_status"] = "/run/pd-device/update.json"'; do
	cp "$T/devices/raze/manifest.json" "$T/good.json"
	edit "$change"
	if gen --check > /dev/null 2>&1; then fail "the lint should refuse: $change"; fi
	cp "$T/good.json" "$T/devices/raze/manifest.json"
done
gen --check > /dev/null || fail "the restored manifest should pass"

echo "ok"
