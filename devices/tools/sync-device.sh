#!/bin/sh
# Vendor a device package from an Atlas commit into an OS repository, and check
# a vendored copy. Interim tool until Gaia imports are taken straight from the
# Atlas git source everywhere.
#
#   sync-device.sh [--atlas <repo>] [--commit <rev>] [--force] <model> <dest>
#       Copy devices/<model> at <rev> (default HEAD) of the Atlas repo into
#       <dest>, stamp the commit into the package, and write <dest>/.device-lock
#       with the commit and a content hash. <dest> is replaced.
#
#   sync-device.sh --check [--atlas <repo>] <dest>
#       Fail if <dest> no longer matches its .device-lock (a file was edited,
#       added or removed). When the Atlas repo has the locked commit, also fail
#       if <dest> differs from what that commit produces.
#
# --atlas defaults to the Atlas checkout this script is in. Exit status: 0 ok,
# 1 mismatch or failure, 2 usage error.
set -eu

LOCK_NAME=.device-lock
STAMP_FILE=gaia/assets/rootfs/usr/lib/pd-device/device-package.env

die() {
	printf 'sync-device: %s\n' "$*" >&2
	exit 1
}

usage() {
	sed -n '2,20s/^# \{0,1\}//p' "$0" >&2
	exit 2
}

sha256_stdin() {
	if command -v sha256sum >/dev/null 2>&1; then
		sha256sum | cut -d' ' -f1
	else
		shasum -a 256 | cut -d' ' -f1
	fi
}

# Hash of every file and symlink under $1 except the lock: path, executable
# bit and content (or link target), in a fixed order.
content_hash() (
	cd "$1"
	find . \( -type f -o -type l \) ! -path "./$LOCK_NAME" -print | LC_ALL=C sort |
		while IFS= read -r f; do
			if [ -L "$f" ]; then
				printf 'link %s %s\n' "$f" "$(readlink "$f")"
			else
				x=-
				[ -x "$f" ] && x=x
				printf 'file %s %s %s\n' "$f" "$x" "$(sha256_stdin < "$f")"
			fi
		done | sha256_stdin
)

lock_value() {
	sed -n "s/^$2=//p" "$1" | head -n 1
}

default_atlas() {
	_here=$(cd "$(dirname "$0")" && pwd)
	git -C "$_here" rev-parse --show-toplevel 2>/dev/null || true
}

# export_package <atlas> <commit> <model> <outdir>: the package as vendored
# from <commit>, with the commit stamped in, at <outdir>/devices/<model>.
export_package() {
	git -C "$1" cat-file -e "$2:devices/$3/manifest.json" 2>/dev/null ||
		die "devices/$3/manifest.json does not exist at $2 in $1"
	git -C "$1" archive --format=tar "$2" "devices/$3" | tar -x -C "$4"
	_stamp="$4/devices/$3/$STAMP_FILE"
	if [ -f "$_stamp" ]; then
		sed "s/^PD_DEVICE_PACKAGE_COMMIT=.*/PD_DEVICE_PACKAGE_COMMIT=$2/" "$_stamp" > "$_stamp.new"
		cat "$_stamp.new" > "$_stamp"
		rm -f "$_stamp.new"
	fi
}

check=0
force=0
atlas=''
rev=HEAD
while [ $# -gt 0 ]; do
	case "$1" in
	--check) check=1 ;;
	--force) force=1 ;;
	--atlas)
		[ $# -ge 2 ] || usage
		atlas=$2
		shift
		;;
	--commit)
		[ $# -ge 2 ] || usage
		rev=$2
		shift
		;;
	-h | --help) usage ;;
	--*) usage ;;
	*) break ;;
	esac
	shift
done
[ -n "$atlas" ] || atlas=$(default_atlas)

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM

if [ "$check" = 1 ]; then
	[ $# -eq 1 ] || usage
	dest=$1
	lock="$dest/$LOCK_NAME"
	[ -f "$lock" ] || die "$lock not found: $dest was not written by sync-device.sh"
	model=$(lock_value "$lock" model)
	commit=$(lock_value "$lock" commit)
	want=$(lock_value "$lock" content_sha256)
	[ -n "$model" ] && [ -n "$commit" ] && [ -n "$want" ] || die "$lock is incomplete"

	got=$(content_hash "$dest")
	status=0
	if [ "$got" != "$want" ]; then
		printf 'sync-device: %s does not match %s: files were edited, added or removed.\n' "$dest" "$LOCK_NAME" >&2
		printf '  locked  %s\n  actual  %s\n' "$want" "$got" >&2
		printf '  Change the package in Atlas and re-sync instead of editing the copy.\n' >&2
		status=1
	fi

	if [ -n "$atlas" ] && git -C "$atlas" cat-file -e "$commit^{commit}" 2>/dev/null; then
		export_package "$atlas" "$commit" "$model" "$tmp"
		expected=$(content_hash "$tmp/devices/$model")
		if [ "$expected" != "$want" ]; then
			printf 'sync-device: %s says %s, but Atlas %s produces %s.\n' "$LOCK_NAME" "$want" "$commit" "$expected" >&2
			status=1
		fi
		if [ "$status" != 0 ]; then
			diff -r -x "$LOCK_NAME" "$tmp/devices/$model" "$dest" >&2 || true
		fi
	else
		printf 'sync-device: Atlas commit %s not available; checked the content hash only.\n' "$commit" >&2
	fi

	if [ "$status" = 0 ]; then
		printf 'sync-device: %s matches %s (model %s, Atlas %s)\n' "$dest" "$LOCK_NAME" "$model" "$commit"
	fi
	exit "$status"
fi

[ $# -eq 2 ] || usage
model=$1
dest=$2
case "$model" in
'' | */* | .*) die "invalid model name: $model" ;;
esac
case "$dest" in
'' | / | . | .. | */. | */..) die "refusing to replace $dest" ;;
esac
[ -n "$atlas" ] || die "no Atlas repository; pass --atlas <repo>"
commit=$(git -C "$atlas" rev-parse --verify "$rev^{commit}" 2>/dev/null) ||
	die "cannot resolve $rev in $atlas"

if [ -d "$dest" ] && [ -d "$atlas/devices/$model" ] &&
	[ "$(cd "$dest" && pwd -P)" = "$(cd "$atlas/devices/$model" && pwd -P)" ]; then
	die "$dest is the package source in Atlas itself"
fi
if [ -e "$dest" ] && [ "$force" = 0 ]; then
	if [ ! -f "$dest/$LOCK_NAME" ] && [ -n "$(ls -A "$dest" 2>/dev/null)" ]; then
		die "$dest exists and was not written by sync-device.sh; use --force to replace it"
	fi
fi

export_package "$atlas" "$commit" "$model" "$tmp"
pkg="$tmp/devices/$model"
hash=$(content_hash "$pkg")
version=$(sed -n 's/^[[:space:]]*"package_version":[[:space:]]*"\([^"]*\)".*/\1/p' "$pkg/manifest.json" | head -n 1)
origin=$(git -C "$atlas" remote get-url origin 2>/dev/null || printf '%s' "$atlas")
{
	printf '# Vendored device package, written by Atlas devices/tools/sync-device.sh.\n'
	printf '# Do not edit files in this directory: change the package in Atlas and\n'
	printf '# re-sync. Check with: sync-device.sh --check <this directory>\n'
	printf 'format=1\n'
	printf 'model=%s\n' "$model"
	printf 'package_version=%s\n' "$version"
	printf 'commit=%s\n' "$commit"
	printf 'content_sha256=%s\n' "$hash"
	printf 'source=%s\n' "$origin"
} > "$pkg/$LOCK_NAME"

rm -rf "$dest"
mkdir -p "$(dirname "$dest")"
mv "$pkg" "$dest"
printf 'sync-device: %s <- Atlas %s devices/%s (package %s)\n' "$dest" "$commit" "$model" "$version"
