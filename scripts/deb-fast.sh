#!/usr/bin/env bash
# A Linux .deb of the desktop app, fast: for fix builds and dev packages. Tagged releases use
# scripts/bundle.sh (the release profile as the root Cargo.toml sets it: thin LTO, one codegen
# unit), which calls this script with --release for its .deb, so both lay the package out alike.
#
#   scripts/deb-fast.sh              build, package, check, print the package and its size
#   scripts/deb-fast.sh --release    the same with the full release profile (slow, smaller)
#   scripts/deb-fast.sh --no-build   package what the target directory already holds
#
# What makes it quick: the release profile without LTO, with 16 codegen units and incremental
# builds, set through CARGO_PROFILE_RELEASE_* for this run only; the target directory is the
# workspace's own (CARGO_TARGET_DIR), so a small change rebuilds the changed crates and the link;
# and the package is put together with dpkg-deb directly, with no bundler step.
#
# The package links against THIS computer's WebKitGTK and glibc. It installs on the same
# distribution family and release as it was built on (or newer); a package for Debian or Ubuntu is
# built on Debian or Ubuntu. On a computer without dpkg it can be made and inspected, not installed.
#
# In the package: /usr/bin/atlas-app, /usr/bin/atlas-helper (the disk writer runs it through
# pkexec), the menu entry, the icons, and /usr/lib/atlas-hardware-manager/install-kind ("deb",
# which the app's own update reads). Not in it, and checked: the UI bridge and the simulated
# devices (feature `sim`).
#
# The version is the workspace rule's (scripts/version-rule.sh): ATLAS_VERSION when set (a tagged
# release), else <release>-dev.<commit count>+<commit>, which needs a committed tree. The package
# carries it as dpkg orders it: the first `-` becomes `~`, so a dev build sorts below its release.
#
# Output: DEB_OUT (default <target>/deb). Needs: cargo, bun, dpkg-deb.
set -euo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"

profile=fast build=1
for a in "$@"; do
  case $a in
    --release) profile=release ;;
    --no-build) build=0 ;;
    -h|--help) sed -n '2,29p' "$0"; exit 0 ;;
    *) echo "deb-fast: unknown argument $a" >&2; exit 2 ;;
  esac
done
command -v dpkg-deb >/dev/null || { echo "deb-fast: needs dpkg-deb (the dpkg package)" >&2; exit 1; }

# shellcheck source=scripts/version-rule.sh
source scripts/version-rule.sh
version=${ATLAS_VERSION:-$(atlas_version "$root")} || exit 1
deb_version=${version/-/\~}
case $(uname -m) in
  x86_64) arch=amd64 ;;
  aarch64) arch=arm64 ;;
  *) echo "deb-fast: no Debian architecture name for $(uname -m)" >&2; exit 1 ;;
esac

target=${CARGO_TARGET_DIR:-$root/target}
case $target in /*) ;; *) target=$root/$target ;; esac
out=${DEB_OUT:-$target/deb}
app=apps/desktop
start=$SECONDS

if [ "$build" -eq 1 ]; then
  if [ "$profile" = fast ]; then
    export CARGO_PROFILE_RELEASE_LTO=false CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 CARGO_PROFILE_RELEASE_INCREMENTAL=true
  fi
  echo "==> deb-fast: the UI"
  (cd $app && bun install --frozen-lockfile >/dev/null && bun run build >/dev/null)
  echo "==> deb-fast: atlas-app and atlas-helper ($profile profile)"
  # custom-protocol: the UI is inside the program. No `sim`: a package has no simulated devices.
  ATLAS_VERSION=$version cargo build --locked --release -p atlas-app --bin atlas-app -p atlas-helper --features atlas-app/custom-protocol
fi
for bin in atlas-app atlas-helper; do
  [ -x "$target/release/$bin" ] || { echo "deb-fast: $target/release/$bin is not built" >&2; exit 1; }
done
# The simulated devices' calls are named in a `sim` build and nowhere else.
if grep -q -a 'sim\.set_online' "$target/release/atlas-app"; then
  echo "deb-fast: $target/release/atlas-app was built with the simulated devices (feature sim): not packaged" >&2
  exit 1
fi

stage=$(mktemp -d "$target/deb-stage.XXXXXX")
trap 'rm -rf "$stage"' EXIT
pkg=$stage/atlas-hardware-manager
install -D -m 0755 "$target/release/atlas-app" "$pkg/usr/bin/atlas-app"
install -D -m 0755 "$target/release/atlas-helper" "$pkg/usr/bin/atlas-helper"
install -D -m 0644 $app/src-tauri/bundle/linux/install-kind-deb "$pkg/usr/lib/atlas-hardware-manager/install-kind"
install -D -m 0644 $app/src-tauri/bundle/linux/atlas-hardware-manager.desktop "$pkg/usr/share/applications/atlas-hardware-manager.desktop"
install -D -m 0644 $app/src-tauri/icons/32x32.png "$pkg/usr/share/icons/hicolor/32x32/apps/atlas-app.png"
install -D -m 0644 $app/src-tauri/icons/128x128.png "$pkg/usr/share/icons/hicolor/128x128/apps/atlas-app.png"
install -D -m 0644 "$app/src-tauri/icons/128x128@2x.png" "$pkg/usr/share/icons/hicolor/256x256@2/apps/atlas-app.png"
installed_kb=$(du -sk "$pkg/usr" | cut -f1)
mkdir -p "$pkg/DEBIAN"
cat > "$pkg/DEBIAN/control" <<CONTROL
Package: atlas-hardware-manager
Version: $deb_version
Architecture: $arch
Maintainer: Prometheus Dynamics <https://github.com/Prometheus-Dynamics/Atlas-Hardware-Manager>
Installed-Size: $installed_kb
Depends: libwebkit2gtk-4.1-0, libgtk-3-0, pkexec | policykit-1
Section: utils
Priority: optional
Homepage: https://github.com/Prometheus-Dynamics/Atlas-Hardware-Manager
Description: Find, flash, update and watch your boards
 Atlas Hardware Manager finds every board on your computer and your robot,
 flashes it, updates it and manages its slots, and gets you in when it is broken.
CONTROL

mkdir -p "$out"
deb=$out/atlas-hardware-manager_${deb_version}_${arch}.deb
dpkg-deb --root-owner-group -Zxz --build "$pkg" "$deb" >/dev/null

# The package as built, read back: what a person installing it gets.
listing=$(dpkg-deb --contents "$deb")
for want in ./usr/bin/atlas-app ./usr/bin/atlas-helper ./usr/lib/atlas-hardware-manager/install-kind ./usr/share/applications/atlas-hardware-manager.desktop; do
  grep -q " $want\$" <<<"$listing" || { echo "deb-fast: $want is missing from $deb" >&2; exit 1; }
done
if grep -q 'ui-bridge' <<<"$listing"; then
  echo "deb-fast: the UI bridge is in $deb" >&2; exit 1
fi
size() { stat -c %s "$1" | awk '{ printf "%.1f MB", $1 / 1e6 }'; }
echo "deb-fast: $deb"
echo "deb-fast: version $deb_version, $(size "$deb") packed, $(awk -v k="$installed_kb" 'BEGIN { printf "%.1f MB", k / 1024 }') installed (atlas-app $(size "$target/release/atlas-app"), atlas-helper $(size "$target/release/atlas-helper")), $((SECONDS - start)) s"
echo "deb-fast: install with: sudo apt install $deb"
