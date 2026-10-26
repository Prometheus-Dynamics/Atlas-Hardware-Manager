#!/usr/bin/env bash
# Prints the version an Atlas build carries: ATLAS_VERSION when set (CI sets
# it from a tag), else <release>-dev.<commit count>+<commit>, the release
# being worked toward from Cargo.toml's [workspace.metadata.atlas]. The same
# rule as scripts/build-version.rs (the binaries' --version and About).
set -euo pipefail
root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -n "${ATLAS_VERSION:-}" ]]; then
  printf '%s\n' "$ATLAS_VERSION"
  exit 0
fi
release=$(sed -n '/^\[workspace.metadata.atlas\]/,/^\[/s/^release *= *"\(.*\)"/\1/p' "$root_dir/Cargo.toml")
if count=$(git -C "$root_dir" rev-list --count HEAD 2>/dev/null) &&
  commit=$(git -C "$root_dir" rev-parse --short=8 HEAD 2>/dev/null); then
  printf '%s-dev.%s+%s\n' "$release" "$count" "$commit"
else
  printf '%s-dev\n' "$release"
fi
