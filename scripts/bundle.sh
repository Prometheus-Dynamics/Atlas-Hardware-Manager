#!/usr/bin/env bash
# Builds the desktop installers for this computer's OS: .deb/.rpm/AppImage
# on Linux, an NSIS installer on Windows, .dmg on macOS. Everything a user
# needs is inside: the app, atlas-helper, the Pi boot files, device
# packages, and the USB access setup (udev rule or WinUSB driver).
#
# Usage: scripts/bundle.sh [target-triple]
# ATLAS_VERSION (CI sets it from the tag, v2026.1.0 -> 2026.1.0) is the
# version the app reports and its installers carry; without it they carry
# <release>-dev.<commit count>+<commit> (scripts/version.sh), which the app
# treats as a development build when it looks for updates.
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
target="${1:-$(rustc -vV | sed -n 's/^host: //p')}"

"$root_dir/scripts/prepare-bundle.sh" "$target"

ATLAS_VERSION=$("$root_dir/scripts/version.sh")
export ATLAS_VERSION
configs=(--config src-tauri/tauri.bundle.conf.json --config "{\"version\":\"$ATLAS_VERSION\"}")
if [[ "$target" == *windows* ]]; then
  "$root_dir/scripts/fetch-windows-driver-tool.sh"
  configs+=(--config src-tauri/tauri.windows-bundle.conf.json)
fi

cd "$root_dir/apps/desktop"
bun run tauri build --target "$target" "${configs[@]}"
echo "Installers are in ${CARGO_TARGET_DIR:-$root_dir/target}/$target/release/bundle/"
