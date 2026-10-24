#!/bin/sh
# Off-device test of gaia/scripts/check-boot-overlays.sh on boot FAT trees
# laid out as slots.toml lays them out, and of slots.toml itself: its slot
# payload carries overlays/README and every overlay raze-device.txt loads.
# Run: sh devices/raze/tests/boot-overlays.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
gaia=$here/../gaia
check=$gaia/scripts/check-boot-overlays.sh
T=$(mktemp -d "${TMPDIR:-/tmp}/raze-boot-overlays.XXXXXX")
trap 'rm -rf "$T"' EXIT
fail() {
	echo "FAIL: $*" >&2
	exit 1
}
overlays=$(sed -n 's/^dtoverlay=\([^,]*\).*/\1/p' "$gaia/assets/boot/raze-device.txt" | sort -u)

# fat <dir> <slot overlays...>: a FAT as slots.toml makes it, slot "s1".
fat() {
	f=$1
	shift
	rm -rf "$f"
	mkdir -p "$f/overlays" "$f/slots/s1/overlays"
	cp "$gaia/assets/slots/config.txt" "$gaia/assets/slots/tryboot.txt" "$gaia/assets/boot/raze-device.txt" "$f/"
	echo "os_prefix=slots/s1/" > "$f/default.txt"
	echo "os_prefix=slots/s1/" > "$f/trial.txt"
	echo readme > "$f/slots/s1/overlays/README"
	for o in "$@"; do
		: > "$f/slots/s1/overlays/$o.dtbo"
		: > "$f/overlays/$o.dtbo"
	done
}
# shellcheck disable=SC2086
run() { sh "$check" "$1" > "$T/out" 2>&1; }

echo "every overlay in the slot's overlays/ with its README: passes"
# shellcheck disable=SC2086
fat "$T/good" $overlays
run "$T/good" || fail "a good FAT failed: $(cat "$T/out")"

echo "the shipped PV 2026 layout (no README, the root without the board's own overlays): fails, naming them"
# shellcheck disable=SC2086
fat "$T/shipped" $overlays
rm "$T/shipped/slots/s1/overlays/README" "$T/shipped/overlays/ov9782.dtbo" "$T/shipped/overlays/raze-fan.dtbo"
run "$T/shipped" && fail "the shipped layout passed"
grep -q "MISSING config.txt: ov9782.dtbo isn't in overlays/" "$T/out" || fail "ov9782: $(cat "$T/out")"
grep -q "MISSING tryboot.txt: raze-fan.dtbo" "$T/out" || fail "raze-fan on the trial boot: $(cat "$T/out")"

echo "no README, even with the root carrying everything: fails (the slot's overlays are ignored)"
# shellcheck disable=SC2086
fat "$T/noreadme" $overlays
rm "$T/noreadme/slots/s1/overlays/README"
run "$T/noreadme" && fail "a slot overlays/ without its README passed"
grep -q "has no README" "$T/out" || fail "reason: $(cat "$T/out")"

echo "an overlay missing from the slot, or from the root fallback copy: fails"
# shellcheck disable=SC2086
fat "$T/noslot" $overlays
rm "$T/noslot/slots/s1/overlays/ov9782.dtbo"
run "$T/noslot" && fail "a slot without ov9782 passed"
# shellcheck disable=SC2086
fat "$T/noroot" $overlays
rm "$T/noroot/overlays/raze-fan.dtbo"
run "$T/noroot" && fail "a root copy without raze-fan passed"
grep -q "MISSING root overlays/raze-fan.dtbo" "$T/out" || fail "reason: $(cat "$T/out")"

echo "the trial's prefix is checked on its own"
# shellcheck disable=SC2086
fat "$T/trial" $overlays
mkdir -p "$T/trial/slots/s2/overlays"
echo "os_prefix=slots/s2/" > "$T/trial/trial.txt"
echo readme > "$T/trial/slots/s2/overlays/README"
run "$T/trial" && fail "a trial slot without overlays passed"
grep -q "MISSING tryboot.txt: ov9782.dtbo isn't in slots/s2/overlays/" "$T/out" || fail "trial: $(cat "$T/out")"

echo "slots.toml's slot payload has overlays/README and each overlay raze-device.txt loads"
python3 - "$gaia" $overlays <<'PY' || fail "slots.toml: see above"
import sys, tomllib, os, fnmatch
g, want = sys.argv[1], sys.argv[2:]
a = tomllib.load(open(f"{g}/slots.toml", "rb"))["image"]["assembly"]
files = [f for f in a["files"] if f.get("tree") == "slots-payload"]
dests = {f["dest"] for f in files}
bad = []
if "overlays/README" not in dests:
    bad.append("no overlays/README in the slot payload")
named = {os.path.basename(f["src"]).removesuffix(".dtbo") for f in files if f.get("src", "").endswith(".dtbo") and f["dest"].startswith("overlays/")}
globbed = [f for f in files if f.get("src_glob", "").endswith("*.dtbo") and f["dest"].startswith("overlays")]
for o in want:
    if o in named:
        continue
    # The stock glob covers it unless it's excluded (the Raze builds its own).
    if any(f"{o}.dtbo" not in f.get("exclude", []) for f in globbed) and o not in ("ov9782", "raze-fan", "raze-usb-power"):
        continue
    bad.append(f"{o}.dtbo isn't copied into the slot payload's overlays/")
if bad:
    print("\n".join(bad), file=sys.stderr); sys.exit(1)
PY
echo "ok"
