#!/usr/bin/env bash
# Fetches the Raspberry Pi mass-storage-gadget boot files that Atlas serves
# during USB boot, pinned to one raspberrypi/usbboot commit and checked
# against known SHA-256 digests. Output: tools/usbboot/mass-storage-gadget64/
#
# Usage: scripts/fetch-usbboot-files.sh [output-dir]
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out_dir="${1:-$root_dir/tools/usbboot/mass-storage-gadget64}"
commit="f905f2f5a92e086defa9607f8fd67633d96d5bd6"
base="https://raw.githubusercontent.com/raspberrypi/usbboot/$commit"

# path-in-repo  file-name  sha256
files=(
  "firmware/bootfiles.bin bootfiles.bin 344c6a2c6a9109e0d041f58bd1a65f839c4f30d8bbe6a73eabd8e894772805ad"
  "mass-storage-gadget64/boot.img boot.img ba98c962eb41270379681f78c97907890a791f281ff84bd0b98ea0fcf7fc441e"
  "mass-storage-gadget64/config.txt config.txt f74a9db07cd32f418e25754134840b138486e39cda2a4e536042b86c5c67b687"
)

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

mkdir -p "$out_dir"
for entry in "${files[@]}"; do
  read -r repo_path name expected <<<"$entry"
  target="$out_dir/$name"
  if [[ -f "$target" && "$(sha256 "$target")" == "$expected" ]]; then
    echo "ok       $name"
    continue
  fi
  echo "fetching $name"
  curl --fail --location --silent --show-error -o "$target.part" "$base/$repo_path"
  actual="$(sha256 "$target.part")"
  if [[ "$actual" != "$expected" ]]; then
    rm -f "$target.part"
    echo "error: $name has SHA-256 $actual, expected $expected" >&2
    exit 1
  fi
  mv "$target.part" "$target"
done
echo "USB boot files ready in $out_dir"
