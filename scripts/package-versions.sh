#!/usr/bin/env bash
# A development build's version as the Linux package managers order it.
# Atlas's own version, <release>-dev.<count>+<commit>, is semver: it sorts
# below its release. dpkg reads the `-dev…` as a Debian revision and sorts it
# above the release, and rpm allows no `-` in a version at all. So a dev .deb
# carries <release>~dev.<count>+<commit> (`~` sorts below the release) and a
# dev .rpm is <release> with release 0.dev.<count>+<commit> (a tagged
# release's is 1). A release's version is used as it is.
#
#   scripts/package-versions.sh deb <version>             the .deb's version
#   scripts/package-versions.sh rpm <version>             "<version> <release>"
#   scripts/package-versions.sh fix-debs <dir> <version>  sets that version in
#       each .deb in <dir> that Tauri built with <version>, renaming it to match
#   scripts/package-versions.sh rpm-config <version>      the Tauri --config
#       for the .rpm (empty for a release)
set -euo pipefail

version=${2:?usage: package-versions.sh deb|rpm|rpm-config <version> | fix-debs <dir> <version>}
case "$1" in
  deb)
    printf '%s\n' "${version/-/\~}"
    ;;
  rpm)
    if [[ "$version" == *-* ]]; then
      printf '%s 0.%s\n' "${version%%-*}" "${version#*-}"
    else
      printf '%s 1\n' "$version"
    fi
    ;;
  rpm-config)
    [[ "$version" == *-* ]] || exit 0
    printf '{"version":"%s","bundle":{"linux":{"rpm":{"release":"0.%s"}}}}\n' \
      "${version%%-*}" "${version#*-}"
    ;;
  fix-debs)
    dir=$2 version=${3:?fix-debs <dir> <version>}
    [[ "$version" == *-* ]] || exit 0
    wanted=${version/-/\~}
    shopt -s nullglob
    for deb in "$dir"/*"_${version}_"*.deb; do
      tmp=$(mktemp -d)
      trap 'rm -rf "$tmp"' EXIT
      dpkg-deb -R "$deb" "$tmp/root"
      sed -i "s/^Version: .*/Version: $wanted/" "$tmp/root/DEBIAN/control"
      name=$(basename "$deb")
      # xz: what every dpkg reads (Ubuntu's default, zstd, older ones don't).
      dpkg-deb --root-owner-group -Zxz -b "$tmp/root" "$dir/${name/"_${version}_"/_${wanted}_}" >/dev/null
      rm -f "$deb"
      rm -rf "$tmp"
      trap - EXIT
    done
    ;;
  *)
    echo "package-versions.sh: unknown command $1" >&2
    exit 2
    ;;
esac
