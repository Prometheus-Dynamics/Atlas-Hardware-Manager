#!/usr/bin/env bash
# The desktop installers for this computer's OS, with the release profile.
#
#   scripts/bundle.sh                 every installer this OS makes
#   scripts/bundle.sh rpm appimage    only these (Linux: deb, rpm, appimage)
#
#   Linux    .deb       scripts/deb-fast.sh --release (one layout for the fast and the release
#                       package, and a version dpkg orders correctly)
#            .rpm       Tauri's bundler
#            .AppImage  Tauri's bundler; it fetches the AppImage tools from GitHub on first use
#                       (tauri-apps/binary-releases) into ~/.cache/tauri
#   Windows  setup .exe (NSIS, installed for the machine: the disk writer needs elevation)
#                       NOT RUN on this branch: written from Tauri's configuration, untested
#   macOS    .dmg       NOT RUN on this branch: written from Tauri's configuration, untested
#
# Every package holds atlas-app and, beside it, atlas-helper (the disk writer). None holds the UI
# bridge or the simulated devices: they need the `sim` feature, which is not built here, and the
# script checks the program for it. The bundle settings are apps/desktop/src-tauri/
# tauri.bundle.conf.json (and tauri.windows-bundle.conf.json), kept apart from tauri.conf.json so a
# plain `cargo build` and `tauri dev` need no helper binary in place.
#
# ATLAS_VERSION (set from the tag for a release) is the version the app reports and its installers
# carry; without it they carry <release>-dev.<commit count>+<commit> (scripts/version-rule.sh). The
# .deb and the .rpm carry a dev version as <release>~dev.<count>+<commit>, below its release.
#
# Each installer's size is printed (lean rule). Nothing is uploaded or published by this script.
set -euo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"
case ${1:-} in -h|--help) sed -n '2,25p' "$0"; exit 0 ;; esac

# shellcheck source=scripts/version-rule.sh
source scripts/version-rule.sh
ATLAS_VERSION=${ATLAS_VERSION:-$(atlas_version "$root")} || exit 1
export ATLAS_VERSION
triple=$(rustc -vV | sed -n 's/^host: //p')
target=${CARGO_TARGET_DIR:-$root/target}
case $target in /*) ;; *) target=$root/$target ;; esac
app=apps/desktop

case $triple in
  *linux*) all=(deb rpm appimage) exe="" ;;
  *windows*) all=(nsis) exe=.exe ;;
  *apple*) all=(dmg) exe="" ;;
  *) echo "bundle: no installers for $triple" >&2; exit 1 ;;
esac
wanted=("$@"); [ ${#wanted[@]} -gt 0 ] || wanted=("${all[@]}")
for kind in "${wanted[@]}"; do
  [[ " ${all[*]} " == *" $kind "* ]] || { echo "bundle: $kind is not an installer for $triple (${all[*]})" >&2; exit 2; }
done

size() { wc -c < "$1" | awk '{ printf "%.1f MB", $1 / 1e6 }'; }
made=()

# The .deb: the same script as the fast one, with the release profile. It builds both programs.
tauri_kinds=()
for kind in "${wanted[@]}"; do
  if [ "$kind" = deb ]; then
    scripts/deb-fast.sh --release
    made+=("$(ls -t "${DEB_OUT:-$target/deb}"/*.deb | head -n 1)")
  else
    tauri_kinds+=("$kind")
  fi
done

if [ ${#tauri_kinds[@]} -gt 0 ]; then
  echo "==> bundle: atlas-helper, beside the app in every package"
  cargo build --locked --release -p atlas-helper
  mkdir -p $app/src-tauri/binaries
  # Tauri's externalBin wants the program named with the target triple; the package holds it
  # without it, beside atlas-app.
  cp "$target/release/atlas-helper$exe" "$app/src-tauri/binaries/atlas-helper-$triple$exe"

  configs=(--config src-tauri/tauri.bundle.conf.json)
  [[ $triple == *windows* ]] && configs+=(--config src-tauri/tauri.windows-bundle.conf.json)
  # The version the packages carry. Tauri's rpm bundler writes a dev version's `-` as `~` itself.
  configs+=(--config "{\"version\":\"$ATLAS_VERSION\"}")
  echo "==> bundle: ${tauri_kinds[*]} (Tauri's bundler)"
  bundles=$(IFS=,; echo "${tauri_kinds[*]}")
  (cd $app && bun install --frozen-lockfile >/dev/null && bun run tauri build --bundles "$bundles" "${configs[@]}")
  if grep -q -a 'sim\.set_online' "$target/release/atlas-app$exe"; then
    echo "bundle: atlas-app was built with the simulated devices (feature sim): do not ship these" >&2
    exit 1
  fi
  while IFS= read -r file; do made+=("$file"); done < <(find "$target/release/bundle" -newer "$app/src-tauri/binaries/atlas-helper-$triple$exe" -type f \( -name '*.rpm' -o -name '*.AppImage' -o -name '*-setup.exe' -o -name '*.dmg' \))
fi

[ ${#made[@]} -gt 0 ] || { echo "bundle: no installer was made" >&2; exit 1; }
echo "bundle: version $ATLAS_VERSION"
for file in "${made[@]}"; do echo "bundle: $(size "$file")  $file"; done
