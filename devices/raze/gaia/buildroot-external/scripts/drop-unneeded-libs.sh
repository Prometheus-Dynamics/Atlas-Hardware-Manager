#!/bin/sh
# drop-unneeded-libs.sh <target dir> <readelf> <soname>...: removes each
# shared library (/lib, /usr/lib) whose soname no ELF in the target needs
# (DT_NEEDED), with its symlinks. For libraries a package brings without
# anything using them (the toolchain's libgfortran). A library opened by
# dlopen isn't seen; name only ones nothing loads that way.
set -eu
target=$1 readelf=$2
shift 2
# Fail closed: without a working readelf every soname would look unneeded.
command -v "$readelf" > /dev/null 2>&1 || { echo "raze: no readelf at $readelf; dropping nothing" >&2; exit 1; }
needed=$(find "$target" -type f \( -perm -u+x -o -name '*.so*' \) 2>/dev/null | while IFS= read -r f; do
	[ "$(head -c 4 "$f" 2>/dev/null | od -An -c | tr -d ' ')" = '177ELF' ] || continue
	"$readelf" -d "$f" 2>/dev/null | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p'
done | sort -u)
printf '%s\n' "$needed" | grep -q '^libc\.so' || { echo "raze: found nothing that needs libc; dropping nothing" >&2; exit 1; }
for so in "$@"; do
	if printf '%s\n' "$needed" | grep -qx "$so"; then
		echo "raze: keeping $so (needed)"
		continue
	fi
	for dir in "$target/lib" "$target/usr/lib"; do
		# /lib as a link to usr/lib: one place, done once.
		[ ! -L "$dir" ] || continue
		for f in "$dir/$so" "$dir/$so".*; do
			[ -e "$f" ] || [ -L "$f" ] || continue
			real=$(readlink -f "$f" 2>/dev/null || true)
			rm -f "$f"
			case $real in "$dir"/*) rm -f "$real" ;; esac
			echo "raze: dropped ${f#"$target"} (nothing needs $so)"
		done
	done
done
