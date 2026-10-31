#!/usr/bin/env bash
# Prepares everything a release bundle carries besides the app itself:
# the USB boot files, the SystemCore's USB agent package (.ipk), and the
# atlas-helper sidecar for the target triple,
# plus the atlas CLI sidecar on Windows. Elsewhere the app is the CLI too
# (invoked as `atlas`, or `atlas-app --cli`; the Linux packages install
# bundle/linux/atlas, which runs that), so it isn't built twice. Installed
# next to the app, the CLI finds the helper and the bundled boot files and
# device packages the same way the app does.
#
# Usage: scripts/prepare-bundle.sh [target-triple]
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root_dir"
target="${1:-$(rustc -vV | sed -n 's/^host: //p')}"

"$root_dir/scripts/fetch-usbboot-files.sh"
# The SystemCore's USB agent package, carried with the boot files
# (usbboot/atlas-usb-agent/ in the app's resources).
rm -rf "$root_dir/tools/usbboot/atlas-usb-agent"
"$root_dir/scripts/build-usb-agent-ipk.sh" "$root_dir/tools/usbboot/atlas-usb-agent"

packages=(-p atlas-helper)
if [[ "$target" == *windows* ]]; then
  # The app is a GUI-subsystem program there and can't use the console.
  packages+=(-p atlas-cli)
fi
cargo build --release "${packages[@]}" --target "$target"
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
if [[ "$target" == *windows* ]]; then
  cp "$target_dir/$target/release/atlas$extension" "$bin_dir/atlas-$target$extension"
fi
echo "Bundle inputs ready for $target"
