#!/bin/sh
# Every file the Raze Gaia modules stage (`src = "${self}/..."`) must be
# tracked in git, so a clean checkout of a commit builds. Also every id in a
# feed list must name a staged file, and every staged file must be fed. Run: sh devices/raze/tests/stage-files.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
repo=$(git -C "$here" rev-parse --show-toplevel)
bad=0

# check_layers <gaia dir>
check_layers() {
	gaia=$1
	prefix=${gaia#"$repo"/}

	for toml in "$gaia"/*.toml; do
		sed -n 's/^src = "${self}\/\(.*\)"$/\1/p' "$toml" | while IFS= read -r src; do
			if ! git -C "$repo" ls-files --error-unmatch "$prefix/$src" >/dev/null 2>&1; then
				echo "FAIL: ${toml##*/} stages $src, which isn't tracked in git" >&2
				echo x
			fi
		done
	done > "${TMPDIR:-/tmp}/stage-files.$$"
	[ ! -s "${TMPDIR:-/tmp}/stage-files.$$" ] || bad=1
	rm -f "${TMPDIR:-/tmp}/stage-files.$$"

	ids=$(sed -n 's/^id = "\(.*\)"$/\1/p' "$gaia"/*.toml)
	for toml in "$gaia"/*.toml; do
		sed -n '/^stage_files = \[/,/^\]/p' "$toml" | sed -n 's/^ *"\(.*\)",*$/\1/p' | while IFS= read -r id; do
			printf '%s\n' "$ids" | grep -qx "$id" || echo "FAIL: ${toml##*/} feeds $id, which no stage entry defines" >&2
		done
	done 2> "${TMPDIR:-/tmp}/stage-ids.$$"
	if [ -s "${TMPDIR:-/tmp}/stage-ids.$$" ]; then
		cat "${TMPDIR:-/tmp}/stage-ids.$$" >&2
		bad=1
	fi
	rm -f "${TMPDIR:-/tmp}/stage-ids.$$"

	# The other way: with a feed list, Gaia stages only the ids it names, so a
	# stage entry left out of every list never reaches the image. lemnosd-env
	# replaces an item that Lemnos's own layer feeds.
	fed=$(printf 'lemnosd-env\n'; for toml in "$gaia"/*.toml; do
		sed -n '/^stage_files = \[/,/^\]/p' "$toml" | sed -n 's/^ *"\(.*\)",*$/\1/p'
		sed -n 's/^stage_files = \[\(.*\)\]$/\1/p' "$toml" | tr ',' '\n' | sed -n 's/^ *"\(.*\)" *$/\1/p'
	done)
	for toml in "$gaia"/*.toml; do
		awk '/^\[\[stage\.files\]\]/ { f = 1; next } /^\[/ { f = 0 } f && /^id = "/ { sub(/^id = "/, ""); sub(/"$/, ""); print }' "$toml" |
			while IFS= read -r id; do
				printf '%s\n' "$fed" | grep -qx "$id" || echo "FAIL: ${toml##*/} stages $id, which no feed list names (so the image won't contain it)" >&2
			done
	done 2> "${TMPDIR:-/tmp}/stage-fed.$$"
	if [ -s "${TMPDIR:-/tmp}/stage-fed.$$" ]; then
		cat "${TMPDIR:-/tmp}/stage-fed.$$" >&2
		bad=1
	fi
	rm -f "${TMPDIR:-/tmp}/stage-fed.$$"
}

# The package, and the layer every board shares.
check_layers "$here/../gaia"
check_layers "$here/../../common/board-os/gaia"

[ "$bad" = 0 ] || exit 1
echo "ok"
