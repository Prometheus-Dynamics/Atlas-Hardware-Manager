#!/bin/sh
# Off-device test of data-owners: a service's state on /data that an
# earlier image left under another uid goes to the user's id in this image;
# what is already right, unknown users, missing paths and paths outside
# /data are left alone. chown is stubbed (no root needed).
# Run: sh devices/raze/tests/data-owners.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
lib=$here/../gaia/assets/rootfs/usr/lib/board
T=$(mktemp -d "${TMPDIR:-/tmp}/board-data-owners.XXXXXX")
trap 'rm -rf "$T"' EXIT
fail() {
	echo "FAIL: $*" >&2
	exit 1
}

# The package's lib with this test's conf beside it.
mkdir -p "$T/lib/data-owners.d" "$T/etc/data-owners.d" "$T/data/orion/db" "$T/data/mine" "$T/outside" "$T/run" "$T/board"
cp "$lib/lib.sh" "$lib/data-owners" "$T/lib/"
me=$(id -un)
cat > "$T/lib/data-owners.d/os.conf" <<CONF
# root's in this "image"; the files are this test's user's, as if an
# earlier image had numbered it differently.
root $T/data/orion
$me $T/data/mine
no-such-user-here $T/data/mine
root $T/data/missing
CONF
echo "root $T/outside" > "$T/etc/data-owners.d/local.conf"
cat > "$T/chown" <<'STUB'
#!/bin/sh
echo "chown $*" >> "$CALLS"
STUB
chmod +x "$T/chown"
: > "$T/calls"

CALLS=$T/calls BOARD_LIB_DIR=$T/lib BOARD_ETC_DIR=$T/etc BOARD_RUN_DIR=$T/run BOARD_DATA_DIR=$T/board \
	BOARD_DATA_ROOT=$T/data BOARD_DATA_OWNERS_CHOWN=$T/chown sh "$T/lib/data-owners" 2> "$T/err"

echo "state left under another uid goes to this image's user"
grep -qx "chown -R -P 0:0 $T/data/orion" "$T/calls" || fail "orion: $(cat "$T/calls")"
grep -q '"data.owners"' "$T/board/events.jsonl" 2>/dev/null || grep -rq data.owners "$T/board" "$T/run" || fail "no data.owners event"

echo "right owners, unknown users, missing paths and paths outside /data are left alone"
[ "$(wc -l < "$T/calls" | tr -d ' ')" = 1 ] || fail "extra chowns: $(cat "$T/calls")"

echo "ok"
