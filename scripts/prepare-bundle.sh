#!/usr/bin/env bash
# Prepares everything a release bundle carries besides the app itself:
# the USB boot files and the atlas-helper and atlas (the CLI) sidecars for
# the target triple. Installed next to the app, the CLI finds the helper and
# the bundled boot files and device packages the same way the app does.
#
# Usage: scripts/prepare-bundle.sh [target-triple]
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root_dir"
target="${1:-$(rustc -vV | sed -n 's/^host: //p')}"

"$root_dir/scripts/fetch-usbboot-files.sh"

cargo build --release -p atlas-helper -p atlas-cli --target "$target"
# Cargo's output directory, as cargo resolves it: CARGO_TARGET_DIR, or a
# build.target-dir in a .cargo/config.toml (for example on a faster disk
# than the checkout's), else target/ here. Guessing missed the config file
# and bundled a stale CLI from an old target/.
target_dir="$(cargo metadata --format-version 1 --no-deps \
  | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p' | sed 's/\\\\/\\/g')"
[[ -n "$target_dir" ]] || { echo "can't find cargo's target directory" >&2; exit 1; }
extension=""
if [[ "$target" == *windows* ]]; then
  extension=".exe"
fi
bin_dir="$root_dir/apps/desktop/src-tauri/binaries"
mkdir -p "$bin_dir"
cp "$target_dir/$target/release/atlas-helper$extension" "$bin_dir/atlas-helper-$target$extension"
cp "$target_dir/$target/release/atlas$extension" "$bin_dir/atlas-$target$extension"
echo "Bundle inputs ready for $target"
