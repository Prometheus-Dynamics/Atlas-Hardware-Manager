#!/bin/sh
# Off-device test of ssh-hostkeys: the board keeps one set of SSH host keys
# on /data across OS images (a slot switch keeps the identity), seeded from
# the first OS's own keys, made when there are none, and sshd always gets
# keys in /run, even when /data can't hold them.
# Run: sh devices/raze/tests/ssh-hostkeys.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
lib=$here/../gaia/assets/rootfs/usr/lib/board
command -v ssh-keygen >/dev/null 2>&1 || {
	echo "ssh-hostkeys: skipped (no ssh-keygen)"
	exit 0
}
T=$(mktemp -d "${TMPDIR:-/tmp}/board-ssh-hostkeys.XXXXXX")
trap 'chmod -R u+w "$T" 2>/dev/null; rm -rf "$T"' EXIT
fail() {
	echo "FAIL: $*" >&2
	exit 1
}
run() {
	BOARD_LIB_DIR=$lib BOARD_RUN_DIR=$T/run BOARD_DATA_DIR=$T/data/board BOARD_ETC_DIR=$T/etc/board \
		BOARD_DATA_ROOT=$T/data SSH_HOSTKEYS_ETC=$1 SSH_HOSTKEY_TYPES="ed25519 ecdsa" \
		sh "$lib/ssh-hostkeys" 2> "$T/err"
}
fp() { ssh-keygen -lf "$1" | awk '{print $2}'; }
mkdir -p "$T/data/board" "$T/etc/board" "$T/os1" "$T/os2" "$T/os3"
ssh-keygen -q -t ed25519 -N '' -C '' -f "$T/os1/ssh_host_ed25519_key"
ssh-keygen -q -t ed25519 -N '' -C '' -f "$T/os2/ssh_host_ed25519_key"

echo "the first OS's own key becomes the board's; a type it lacks is made"
run "$T/os1"
[ "$(fp "$T/data/ssh/ssh_host_ed25519_key.pub")" = "$(fp "$T/os1/ssh_host_ed25519_key.pub")" ] || fail "not seeded from the OS"
[ -s "$T/data/ssh/ssh_host_ecdsa_key" ] || fail "no ecdsa key made"
[ "$(stat -c %a "$T/data/ssh/ssh_host_ed25519_key")" = 600 ] || fail "store key mode $(stat -c %a "$T/data/ssh/ssh_host_ed25519_key")"
[ "$(stat -c %a "$T/data/ssh")" = 700 ] || fail "store dir mode"
cmp -s "$T/data/ssh/ssh_host_ed25519_key" "$T/run/ssh/hostkeys/ssh_host_ed25519_key" || fail "sshd's copy differs"
[ "$(stat -c %a "$T/run/ssh/hostkeys/ssh_host_ed25519_key")" = 600 ] || fail "run key mode"

echo "another OS image (a slot switch) gets the same keys, not its own"
first=$(fp "$T/data/ssh/ssh_host_ed25519_key.pub")
run "$T/os2"
[ "$(fp "$T/run/ssh/hostkeys/ssh_host_ed25519_key.pub")" = "$first" ] || fail "the identity changed with the OS"
run "$T/os3"
[ "$(fp "$T/run/ssh/hostkeys/ssh_host_ed25519_key.pub")" = "$first" ] || fail "an OS without keys changed the identity"

echo "/data unusable: sshd still gets keys (the OS's own, else for this boot)"
rm -rf "$T/run"
chmod 0500 "$T/data"
mv "$T/data/ssh" "$T/data-ssh-aside" 2>/dev/null || { chmod 0700 "$T/data"; mv "$T/data/ssh" "$T/data-ssh-aside"; chmod 0500 "$T/data"; }
run "$T/os2"
[ "$(fp "$T/run/ssh/hostkeys/ssh_host_ed25519_key.pub")" = "$(fp "$T/os2/ssh_host_ed25519_key.pub")" ] || fail "no fallback to the OS's key"
[ -s "$T/run/ssh/hostkeys/ssh_host_ecdsa_key" ] || fail "no key for this boot"
grep -q "can't keep host keys" "$T/err" || fail "the reason wasn't said: $(cat "$T/err")"
chmod 0700 "$T/data"

echo "ok"
