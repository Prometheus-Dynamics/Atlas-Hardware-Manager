#!/bin/sh
# Off-device test of ssh-keys: key files as editors and Atlas write them
# (no trailing newline, CRLF, comments, duplicates), read straight from a file
# instead of the boot partition. Run: sh devices/raze/tests/ssh-keys.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
lib=$here/../gaia/assets/rootfs/usr/lib/board
T=$(mktemp -d "${TMPDIR:-/tmp}/board-ssh-keys-test.XXXXXX")
trap 'rm -rf "$T"' EXIT

fail() {
	echo "FAIL: $*" >&2
	exit 1
}

export BOARD_LIB_DIR=$lib BOARD_ETC_DIR=$T/etc BOARD_DATA_DIR=$T/data BOARD_RUN_DIR=$T/run
mkdir -p "$T/run" "$T/home" "$T/etc"
# ssh-keys.env sets the home directory; override it the supported way.
printf 'SSH_KEYS_HOME=%s\n' "$T/home" > "$T/etc/ssh-keys.env"
run() { SSH_KEYS_SOURCE_FILE=$1 sh "$lib/ssh-keys" 2> "$T/log"; }
installed() { cat "$T/run/ssh/authorized_keys"; }
A='ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIA one@host'
B='ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAA two@host'

echo "a single key without a trailing newline (as the Flash tab wrote it)"
printf '%s' "$A" > "$T/keys"
run "$T/keys"
[ "$(installed)" = "$A" ] || fail "the key was not installed: '$(installed)'"
grep -q "installed 1 SSH key" "$T/log" || fail "it should log the count: $(cat "$T/log")"
grep -qxF "$A" "$T/home/.ssh/authorized_keys" || fail "the home copy should have the key"

echo "CRLF, a comment, junk, a duplicate, and no newline at the end"
printf '# my keys\r\n%s\r\nnot a key\r\n%s\r\n%s' "$A" "$B" "$A" > "$T/keys"
run "$T/keys"
[ "$(installed | wc -l)" -eq 2 ] || fail "expected 2 keys, got: $(installed)"
installed | grep -qxF "$B" || fail "the CRLF key was mangled: $(installed | od -c | head -3)"
grep -q "installed 2 SSH key" "$T/log" || fail "count: $(cat "$T/log")"
[ "$(grep -c . "$T/home/.ssh/authorized_keys")" -eq 2 ] || fail "the home copy should not repeat keys"

echo "a file with no usable key warns"
printf '# nothing here\n' > "$T/keys"
run "$T/keys"
[ ! -s "$T/run/ssh/authorized_keys" ] || fail "no keys should be installed"
grep -q "warning:.*no usable public key" "$T/log" || fail "it should warn: $(cat "$T/log")"
[ "$(grep -c . "$T/home/.ssh/authorized_keys")" -eq 2 ] || fail "keys already in the home copy are kept"

echo "ok"
