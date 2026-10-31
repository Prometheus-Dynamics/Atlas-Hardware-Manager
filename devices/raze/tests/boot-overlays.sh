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
echo "the FAT slots.toml assembles (its own entries, over fixture build outputs) passes the check"
# Gaia's file entries for the slots-fat tree, in a small model: src, src_glob
# (with exclude) and content, dest a dir when it ends in /. Fixture build
# outputs stand in for Buildroot's and the boot tree; only the config files
# and the overlays are assembled.
B=$T/build
mkdir -p "$B/images/rpi-firmware/overlays" "$B/images/raze-overlays" "$B/boot/overlays"
for o in bcm2712d0 dwc2 i2c1-pi5 ws2812-pio vc4-kms-v3d-pi5 ramoops-pi5 disable-bt-pi5; do : > "$B/images/rpi-firmware/overlays/$o.dtbo"; done
: > "$B/images/rpi-firmware/overlays/overlay_map.dtb"
for o in dwc2 i2c1-pi5 ws2812-pio vc4-kms-v3d-pi5 ramoops-pi5; do : > "$B/images/raze-overlays/$o.dtbo"; done
for o in ov9782 raze-fan raze-usb-power; do : > "$B/boot/overlays/$o.dtbo"; done
cp "$gaia/assets/boot/raze-device.txt" "$B/boot/raze-device.txt"
python3 - "$gaia" "$B" "$T/fat" <<'PY2' || fail "assembling the FAT: see above"
import fnmatch, glob, os, shutil, sys, tomllib
g, b, out = sys.argv[1:]
inputs = {"slot_family": "pv", "slot_version": "1", "slot_variant": "v", "slot_commit": "c0ffee00"}
def expand(v):
    for k, x in inputs.items():
        v = v.replace("${input." + k + "}", x)
    return (v.replace("${provider.buildroot_output}", b).replace("${assembly.tree.boot}", f"{b}/boot")
             .replace("${self}", g))
for e in tomllib.load(open(f"{g}/slots.toml", "rb"))["image"]["assembly"]["files"]:
    if e.get("tree") != "slots-fat":
        continue
    dest = expand(e["dest"])
    if not ("overlays" in dest or dest.endswith(".txt")) or dest.startswith("slots/") and dest.endswith("cmdline.txt") or dest == "cmdline.txt":
        continue
    target = os.path.join(out, dest)
    if "content" in e:
        os.makedirs(os.path.dirname(target), exist_ok=True)
        open(target, "w").write(expand(e["content"]))
        continue
    if "src_glob" in e:
        srcs = [f for f in glob.glob(expand(e["src_glob"])) if not any(fnmatch.fnmatch(os.path.basename(f), x) for x in e.get("exclude", []))]
    else:
        srcs = [expand(e["src"])]
    for src in srcs:
        if not os.path.exists(src):
            sys.exit(f"{e}: {src} doesn't exist (an entry the build would fail on)")
        t = os.path.join(target, os.path.basename(src)) if dest.endswith("/") else target
        os.makedirs(os.path.dirname(t), exist_ok=True)
        shutil.copy(src, t)
PY2
run "$T/fat" || fail "the FAT slots.toml assembles fails the check: $(cat "$T/out")"
[ -f "$T/fat/slots/pv-1-v-c0ffee00/overlays/README" ] || fail "no README in the FAT's slot overlays/"
echo "ok"
