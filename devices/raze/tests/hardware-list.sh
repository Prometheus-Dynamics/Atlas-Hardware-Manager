#!/bin/sh
# Static check of gaia/hardware.toml, the hardware list Gaia's image check
# reads: every overlay raze-device.txt loads is in an entry (or listed here as
# not a device), every overlay an entry names is built by boot.toml, every
# file an entry names is staged by the package, and each entry has the keys
# Gaia expects. Run: sh devices/raze/tests/hardware-list.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
command -v python3 >/dev/null 2>&1 || {
	echo "hardware-list: skipped (no python3)"
	exit 0
}
python3 - "$here/../gaia" <<'PY'
import glob, os, re, sys, tomllib
g = sys.argv[1]
bad = []
hw = tomllib.load(open(f"{g}/hardware.toml", "rb"))["image"]["hardware"]
keys = {"id", "kind", "modules", "compatible", "overlays", "files", "firmware"}
ids = set()
for e in hw:
    if set(e) != keys:
        bad.append(f"{e.get('id')}: keys {sorted(set(e) ^ keys)}")
    if e.get("id") in ids:
        bad.append(f"{e['id']}: listed twice")
    ids.add(e.get("id"))
    if not e.get("compatible"):
        bad.append(f"{e.get('id')}: no compatible")
named = {o for e in hw for o in e["overlays"]}
# Overlays raze-device.txt loads (dtoverlay=<name>[,args]).
txt = open(f"{g}/assets/boot/raze-device.txt").read()
loaded = set(re.findall(r"^dtoverlay=([A-Za-z0-9_-]+)", txt, re.M))
for o in sorted(loaded - named):
    bad.append(f"raze-device.txt loads overlay {o}, which no hardware entry names")
# Overlays boot.toml puts in the boot partition.
boot = tomllib.load(open(f"{g}/boot.toml", "rb"))["image"]["assembly"]
built = {os.path.basename(x["dest"]).removesuffix(".dtbo") for x in boot.get("transforms", []) + boot.get("files", []) if x["dest"].endswith(".dtbo")}
for o in sorted(named - built):
    bad.append(f"hardware entry overlay {o} isn't built by boot.toml")
# Files the package stages.
staged = set()
for f in glob.glob(f"{g}/*.toml"):
    for item in tomllib.load(open(f, "rb")).get("stage", {}).get("files", []):
        staged.add(item.get("dest"))
for e in hw:
    for f in e["files"]:
        if f not in staged:
            bad.append(f"{e['id']}: {f} isn't staged by the package")
if bad:
    print("\n".join(bad), file=sys.stderr)
    sys.exit(1)
PY
echo "ok"
