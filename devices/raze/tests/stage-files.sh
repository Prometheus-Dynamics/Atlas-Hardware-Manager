#!/bin/sh
# Every file the Raze Gaia modules stage (`src = "@self/..."`) must be
# tracked in git, so a clean checkout of a commit builds. Also every id in a
# feed list must name a staged file. Run: sh devices/raze/tests/stage-files.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
gaia=$here/../gaia
repo=$(git -C "$here" rev-parse --show-toplevel)
prefix=${gaia#"$repo"/}
bad=0

for toml in "$gaia"/*.toml; do
	sed -n 's/^src = "@self\/\(.*\)"$/\1/p' "$toml" | while IFS= read -r src; do
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

[ "$bad" = 0 ] || exit 1
echo "ok"
