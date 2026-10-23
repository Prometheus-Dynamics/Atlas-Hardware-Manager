#!/bin/sh
# Off-device test of buildroot-external/scripts/drop-unneeded-libs.sh with
# this computer's own ELF files: a library nothing needs goes (with its
# symlink), one something needs stays, and without a readelf nothing goes.
# Run: sh devices/raze/tests/drop-unneeded-libs.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
script=$here/../gaia/buildroot-external/scripts/drop-unneeded-libs.sh
re=$(command -v readelf || true)
bin=$(for b in /usr/bin/true /bin/true /usr/bin/env; do [ -x "$b" ] && { echo "$b"; break; }; done)
[ -n "$re" ] && [ -n "$bin" ] || {
	echo "drop-unneeded-libs: skipped (no readelf)"
	exit 0
}
T=$(mktemp -d "${TMPDIR:-/tmp}/raze-drop-libs.XXXXXX")
trap 'rm -rf "$T"' EXIT
fail() {
	echo "FAIL: $*" >&2
	exit 1
}
mkdir -p "$T/usr/bin" "$T/usr/lib"
ln -s usr/lib "$T/lib"
cp "$bin" "$T/usr/bin/true"
libc=$("$re" -d "$bin" | sed -n 's/.*(NEEDED).*\[\(libc\.so[^]]*\)\].*/\1/p' | head -n 1)
[ -n "$libc" ] || fail "this computer's $bin needs no libc"
cp "$bin" "$T/usr/lib/libgfortran.so.5.0.0"
ln -s libgfortran.so.5.0.0 "$T/usr/lib/libgfortran.so.5"
cp "$bin" "$T/usr/lib/$libc"

echo "without a readelf nothing goes"
sh "$script" "$T" /nonexistent/readelf libgfortran.so.5 > /dev/null 2>&1 && fail "ran without a readelf"
[ -e "$T/usr/lib/libgfortran.so.5.0.0" ] || fail "dropped without a readelf"

echo "a library nothing needs goes, with its link; one something needs stays"
sh "$script" "$T" "$re" libgfortran.so.5 "$libc" > "$T/out"
[ ! -e "$T/usr/lib/libgfortran.so.5" ] && [ ! -L "$T/usr/lib/libgfortran.so.5" ] && [ ! -e "$T/usr/lib/libgfortran.so.5.0.0" ] || fail "libgfortran stayed: $(ls "$T/usr/lib")"
[ -e "$T/usr/lib/$libc" ] || fail "$libc went"
grep -q "keeping $libc" "$T/out" || fail "output: $(cat "$T/out")"
echo "ok"
