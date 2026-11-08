#!/bin/sh
# check-boot-overlays.sh <boot FAT dir | disk image (.img, .img.xz)>
#
# Fails when the Raspberry Pi firmware would boot without an overlay the
# board's config asks for. For config.txt and tryboot.txt (the trial boot)
# it follows `include` lines (raze-device.txt, default.txt, trial.txt), takes
# each `dtoverlay=<name>` and the os_prefix the includes set, and looks for
# <name>.dtbo where the firmware reads overlays: ${os_prefix}overlays/ when
# that directory holds a README, else the FAT's root overlays/. A slot whose
# overlays/ lacks the README boots with the root's copies, which may not have
# the board's own (shipped PhotonVision 2026 images: no camera, ov9782.dtbo
# was only in the slot's overlays/); a slot overlays/ without its README
# fails too, even when the root happens to have the overlays. It also
# checks that the root overlays/,
# the firmware's fallback, has every overlay the default slot carries.
# An image is read with mtools (mcopy). Exit 0 when every overlay resolves.
set -eu

src=${1:?usage: check-boot-overlays.sh <boot FAT dir | disk image>}
tmp=''
cleanup() { [ -z "$tmp" ] || rm -rf "$tmp"; }
trap cleanup EXIT
if [ -d "$src" ]; then
	fat=$src
else
	tmp=$(mktemp -d "${TMPDIR:-/tmp}/boot-overlays.XXXXXX")
	img=$src
	case $src in *.xz) xz -dc "$src" > "$tmp/disk.img"; img=$tmp/disk.img ;; esac
	command -v mcopy > /dev/null || { echo "check-boot-overlays: needs mtools to read $src" >&2; exit 2; }
	start=$(sfdisk -d "$img" 2>/dev/null | awk -F'[=,]' '/start=/ { gsub(/ /, "", $2); print $2; exit }')
	[ -n "$start" ] || { echo "check-boot-overlays: no partition table in $src" >&2; exit 2; }
	mkdir "$tmp/fat"
	mcopy -s -n -i "$img@@$((start * 512))" ::/ "$tmp/fat/" 2>/dev/null
	fat=$tmp/fat
fi

# lc <file>: the file's lines, comments and CRs stripped.
lc() { tr -d '\r' < "$1" | sed 's/#.*//'; }
# A FAT is case-insensitive: find <dir>/<name> whatever its case.
has() { [ -d "$1" ] && ls -1 "$1" | tr 'A-Z' 'a-z' | grep -qx "$(printf '%s' "$2" | tr 'A-Z' 'a-z')"; }

fail=0
# resolve <config file>: prints "prefix <p>" and "overlay <name>" lines.
resolve() {
	[ -f "$fat/$1" ] || return 0
	lc "$fat/$1" | while IFS= read -r line; do
		line=$(printf '%s' "$line" | sed 's/^[[:space:]]*//; s/[[:space:]]*$//')
		case $line in
		include\ *) resolve "${line#include }" ;;
		os_prefix=*) echo "prefix ${line#os_prefix=}" ;;
		dtoverlay=*) n=${line#dtoverlay=}; n=${n%%,*}; [ -z "$n" ] || echo "overlay $n" ;;
		esac
	done
}

for cfg in config.txt tryboot.txt; do
	[ -f "$fat/$cfg" ] || continue
	out=$(resolve "$cfg")
	prefix=$(printf '%s\n' "$out" | sed -n 's/^prefix //p' | tail -n 1)
	dir=overlays
	if [ -n "$prefix" ] && has "$fat/${prefix}overlays" README; then
		dir=${prefix}overlays
	elif [ -n "$prefix" ] && [ -d "$fat/${prefix}overlays" ]; then
		# The slot's own overlays are ignored: whatever the root has boots,
		# by luck when it has them.
		echo "MISSING $cfg: ${prefix}overlays/ has no README, so the firmware ignores it and reads the root overlays/"
		fail=1
	fi
	for o in $(printf '%s\n' "$out" | sed -n 's/^overlay //p' | sort -u); do
		if has "$fat/$dir" "$o.dtbo"; then
			echo "ok    $cfg: $o ($dir/)"
		else
			echo "MISSING $cfg: $o.dtbo isn't in $dir/, the directory the firmware reads"
			fail=1
		fi
	done
	# The root copy is what the firmware falls back to: it has every overlay
	# the slot does.
	if [ "$cfg" = config.txt ] && [ "$dir" != overlays ] && [ -d "$fat/overlays" ]; then
		for f in "$fat/$dir"/*.dtbo; do
			[ -e "$f" ] || continue
			n=${f##*/}
			has "$fat/overlays" "$n" || { echo "MISSING root overlays/$n (the default slot has it; the fallback copy doesn't)"; fail=1; }
		done
	fi
done
exit "$fail"
