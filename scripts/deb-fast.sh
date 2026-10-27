#!/usr/bin/env bash
# A Linux .deb of the desktop app fast: about a minute after a small change
# once the build output is warm. For fix builds and dev packages; tagged
# releases use scripts/bundle.sh (full LTO, smaller binaries).
#
#   scripts/deb-fast.sh                 Debian trixie (the default)
#   DEB_BASE=ubuntu:22.04 scripts/deb-fast.sh
#
# It builds in a container of the target distro (podman or docker), so the
# package links against that distro's WebKitGTK and glibc. What makes it
# quick:
# - the release profile without LTO (16 codegen units, incremental), set
#   through CARGO_PROFILE_RELEASE_* for this build only;
# - a build output dir and cargo registry that persist between runs
#   (DEB_CACHE, default ~/.cache/atlas-deb/<base>), so a one-line change
#   rebuilds the changed crates and the link;
# - the UI build skipped when apps/desktop's sources haven't changed since
#   the last run;
# - the container image built once (scripts/deb/Containerfile).
# The .deb lands in DEB_OUT (default dist/deb/), named after the commit.
# JOBS sets cargo's parallelism (default: all cores).
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
base=${DEB_BASE:-debian:trixie}
tag=atlas-deb:$(printf '%s' "$base" | tr ':/' '--')
cache=${DEB_CACHE:-$HOME/.cache/atlas-deb/$(printf '%s' "$base" | tr ':/' '--')}
out=${DEB_OUT:-$root_dir/dist/deb}
jobs=${JOBS:-$(nproc)}
engine=${CONTAINER_ENGINE:-$(command -v podman || command -v docker || true)}
[[ -n "$engine" ]] || { echo "deb-fast: needs podman or docker" >&2; exit 1; }
mkdir -p "$cache/target" "$cache/registry" "$cache/git" "$out"

start=$(date +%s)
# The image, once per base (and when the Containerfile changes).
want=$(sha256sum "$root_dir/scripts/deb/Containerfile" | cut -c1-12)
if [[ "$("$engine" image inspect --format '{{ index .Config.Labels "atlas.containerfile" }}' "$tag" 2>/dev/null)" != "$want" ]]; then
  echo "deb-fast: building the $base build image (once)"
  "$engine" build -q --build-arg BASE="$base" --label atlas.containerfile="$want" \
    -t "$tag" -f "$root_dir/scripts/deb/Containerfile" "$root_dir/scripts/deb" >/dev/null
fi

# The UI: rebuilt only when its sources changed.
ui_hash=$(cd "$root_dir/apps/desktop" && git ls-files -s -- . ':!src-tauri' | sha256sum | cut -c1-16)
ui_dirty=$(cd "$root_dir/apps/desktop" && git status --porcelain -- . ':!src-tauri' | sha256sum | cut -c1-16)
ui_key="$ui_hash-$ui_dirty"
skip_ui=0
[[ -f "$cache/ui.key" && "$(cat "$cache/ui.key")" == "$ui_key" && -f "$root_dir/apps/desktop/build/index.html" ]] && skip_ui=1

commit=$(git -C "$root_dir" rev-parse --short HEAD)
[[ -z "$(git -C "$root_dir" status --porcelain -- crates apps Cargo.toml Cargo.lock)" ]] || commit="$commit-dirty"
# The container has no git: the version (scripts/version.sh) goes in. The
# .deb carries it as dpkg orders it (scripts/package-versions.sh).
version=$("$root_dir/scripts/version.sh")

"$engine" run --rm --security-opt label=disable \
  -v "$root_dir:/src" -v "$cache/target:/target" \
  -v "$cache/registry:/opt/cargo/registry" -v "$cache/git:/opt/cargo/git" \
  -e CARGO_TARGET_DIR=/target -e CARGO_BUILD_JOBS="$jobs" \
  -e CARGO_PROFILE_RELEASE_LTO=false -e CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 \
  -e CARGO_PROFILE_RELEASE_INCREMENTAL=true \
  -e ATLAS_VERSION="$version" -e SKIP_UI="$skip_ui" -e HOST_UID="$(id -u)" -e HOST_GID="$(id -g)" \
  -w /src "$tag" bash -euo pipefail -c '
    target=x86_64-unknown-linux-gnu
    scripts/prepare-bundle.sh "$target" >/dev/null
    cd apps/desktop
    bun install --frozen-lockfile >/dev/null
    configs=(--config src-tauri/tauri.bundle.conf.json --config "{\"version\":\"$ATLAS_VERSION\"}")
    if [ "$SKIP_UI" = 1 ]; then
      configs+=(--config "{\"build\":{\"beforeBuildCommand\":\"\"}}")
    fi
    bun run tauri build --target "$target" --bundles deb "${configs[@]}"
    ../../scripts/package-versions.sh fix-debs "/target/$target/release/bundle/deb" "$ATLAS_VERSION"
    chown -R "$HOST_UID:$HOST_GID" /target build src-tauri/binaries 2>/dev/null || true
  '
printf '%s\n' "$ui_key" > "$cache/ui.key"
deb=$(ls -t "$cache/target/x86_64-unknown-linux-gnu/release/bundle/deb/"*.deb | head -n 1)
name=atlas-$commit-$(printf '%s' "$base" | tr ':/' '--').deb
cp "$deb" "$out/$name"
echo "deb-fast: $out/$name ($(( $(date +%s) - start )) s)"
