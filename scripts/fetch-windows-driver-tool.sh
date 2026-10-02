#!/usr/bin/env bash
# Fetches wdi-simple.exe (libwdi, LGPL-3.0) as shipped in Raspberry Pi's
# rpiboot installer, pinned to the same raspberrypi/usbboot commit as the
# boot files and checked against its SHA-256. The Windows installer runs it
# to bind WinUSB to the Pi boot ROM ids. Output: tools/windows/wdi-simple.exe
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out_dir="${1:-$root_dir/tools/windows}"
commit="f905f2f5a92e086defa9607f8fd67633d96d5bd6"
url="https://raw.githubusercontent.com/raspberrypi/usbboot/$commit/win32/redist/wdi-simple.exe"
expected="9a986bdf3dbaf1580d1e0e07e46b17d0b08e6cbd1edc1da33d5f6ac5b72780e5"

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

mkdir -p "$out_dir"
target="$out_dir/wdi-simple.exe"
if [[ -f "$target" && "$(sha256 "$target")" == "$expected" ]]; then
  echo "ok       wdi-simple.exe"
  exit 0
fi
echo "fetching wdi-simple.exe"
curl --fail --location --silent --show-error -o "$target.part" "$url"
actual="$(sha256 "$target.part")"
if [[ "$actual" != "$expected" ]]; then
  rm -f "$target.part"
  echo "error: wdi-simple.exe has SHA-256 $actual, expected $expected" >&2
  exit 1
fi
mv "$target.part" "$target"
echo "wdi-simple.exe ready in $out_dir"
