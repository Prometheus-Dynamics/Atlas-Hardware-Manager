#!/usr/bin/env bash
# scripts/package-versions.sh: a dev build's .deb and .rpm sort below the
# release they lead to and above older dev builds, as dpkg and rpm order them.
set -euo pipefail
pv="$(dirname "${BASH_SOURCE[0]}")/../package-versions.sh"
fail() { echo "package-versions: $*" >&2; exit 1; }

[[ "$("$pv" deb 2026.1.0-dev.40+0123abcd)" == "2026.1.0~dev.40+0123abcd" ]] || fail "deb version"
[[ "$("$pv" deb 2026.1.0)" == "2026.1.0" ]] || fail "a release's deb version changed"
[[ "$("$pv" rpm 2026.1.0-dev.40+0123abcd)" == "2026.1.0 0.dev.40+0123abcd" ]] || fail "rpm version"
[[ "$("$pv" rpm 2026.1.0)" == "2026.1.0 1" ]] || fail "a release's rpm version changed"
[[ -z "$("$pv" rpm-config 2026.1.0)" ]] || fail "a release got an rpm config"

if command -v dpkg >/dev/null; then
  old=$("$pv" deb 2026.1.0-dev.9+ffffffff) new=$("$pv" deb 2026.1.0-dev.40+0123abcd)
  dpkg --compare-versions "$new" lt 2026.1.0 || fail "dev .deb $new not below the release"
  dpkg --compare-versions "$old" lt "$new" || fail "dev .deb $old not below $new"
  dpkg --compare-versions 2025.9.0 lt "$old" || fail "dev .deb $old not above the last release"
fi
# rpmdev-vercmp exits 12 when the first is older.
rpm_lt() { local rc=0; rpmdev-vercmp "$1" "$2" >/dev/null || rc=$?; [[ $rc == 12 ]]; }
if command -v rpmdev-vercmp >/dev/null; then
  read -r v r < <("$pv" rpm 2026.1.0-dev.40+0123abcd)
  read -r ov or < <("$pv" rpm 2026.1.0-dev.9+ffffffff)
  rpm_lt "$v-$r" 2026.1.0-1 || fail "dev .rpm $v-$r not below the release"
  rpm_lt "$ov-$or" "$v-$r" || fail "dev .rpm $ov-$or not below $v-$r"
fi
echo "package-versions: ok"
