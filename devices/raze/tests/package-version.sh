#!/bin/sh
# Off-device test of board_package_version: a dev base gets the Atlas count and
# short commit an image was told, a release stands as it is, and the tree's base
# is what the manifest says. Run: sh devices/raze/tests/package-version.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
lib=$here/../../common/board-os/gaia/assets/rootfs/usr/lib/board
T=$(mktemp -d "${TMPDIR:-/tmp}/board-package-version.XXXXXX")
trap 'rm -rf "$T"' EXIT
export BOARD_LIB_DIR=$lib BOARD_RUN_DIR=$T/run BOARD_DATA_DIR=$T/data BOARD_ETC_DIR=$T/etc

fail() {
	echo "FAIL: $*" >&2
	exit 1
}

# shellcheck disable=SC1091
. "$lib/lib.sh"

check() {
	BOARD_PACKAGE_VERSION=$1 BOARD_PACKAGE_COUNT=$2 BOARD_PACKAGE_COMMIT=$3
	got=$(board_package_version)
	[ "$got" = "$4" ] || fail "$1 count '$2' commit '$3': got '$got', want '$4'"
}

commit=fad5bcc0123456789abcdef0123456789abcdef0
check 2026.1.0-dev 412 "$commit" 2026.1.0-dev.412+fad5bcc0
check 2026.1.0-dev '' "$commit" 2026.1.0-dev+fad5bcc0
check 2026.1.0-dev '' '' 2026.1.0-dev
check 2026.1.0 412 "$commit" 2026.1.0
check '' '' '' ''

echo "the tree's base is the manifest's, in the -dev form"
base=$(sed -n 's/^BOARD_PACKAGE_VERSION=//p' "$here/../gaia/assets/rootfs/usr/lib/board/board-package.env")
grep -q "\"package_version\": \"$base\"" "$here/../manifest.json" || fail "manifest isn't $base"
case $base in
*-dev) ;;
*) fail "$base isn't a dev base" ;;
esac
echo ok
