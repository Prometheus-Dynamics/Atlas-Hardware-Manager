#!/bin/sh
# The files generated from manifest.json match it, and the lint catches a
# manifest edit that wasn't regenerated or breaks a rule.
# Needs python3. Run: sh devices/raze/tests/manifest-lint.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../.." && pwd)
T=$(mktemp -d "${TMPDIR:-/tmp}/pd-manifest-lint.XXXXXX")
trap 'rm -rf "$T"' EXIT

fail() {
	echo "FAIL: $*" >&2
	exit 1
}

echo "the committed files match the manifest"
python3 "$root/devices/tools/gen-raze.py" --check || fail "run devices/tools/gen-raze.py and commit the result"

# A scratch copy of what the tool reads and writes.
mkdir -p "$T/devices"
cp -R "$root/devices/raze" "$root/devices/tools" "$T/devices/"
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
grep -q 'HW_FAN_LEVELS=.128 212 245 255 255.' "$T/devices/raze/gaia/assets/rootfs/usr/lib/pd-device/hardware.env" ||
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
	'c["fan"]["verified"]["no.such.fact"] = "unverified"'; do
	cp "$T/devices/raze/manifest.json" "$T/good.json"
	edit "$change"
	if gen --check > /dev/null 2>&1; then fail "the lint should refuse: $change"; fi
	cp "$T/good.json" "$T/devices/raze/manifest.json"
done
gen --check > /dev/null || fail "the restored manifest should pass"

echo "ok"
