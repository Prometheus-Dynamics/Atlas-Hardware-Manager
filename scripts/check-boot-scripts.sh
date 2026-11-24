#!/usr/bin/env bash
# The board's small boot scripts, run off the board against made-up inputs (no root, no mount):
#   orion-node-name  (devices/components/orion-node-name)  the board's Orion name from its serial;
#   board-journal    (devices/components/raze-base-files)  the journal's bind onto /data, the paths that bind nothing.
# board-journal's bind itself needs root and is proven on the board.
# These run with the PC's tools, so they cannot say whether the image has a command a script calls: that is
# scripts/check-script-commands.py (every shipped script, held to its product's list and to the built root).
#
#   scripts/check-boot-scripts.sh            every case below; exit 1 at the first that fails
set -euo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
name_sh=$root/devices/components/orion-node-name/files/orion-node-name
journal_sh=$root/devices/components/raze-base-files/files/board-journal
t=$(mktemp -d)
trap 'rm -r "${t:?}"' EXIT
fails=0
expect() {  # <case> <want rc> <want text in output or file> <command...>
  local what=$1 rc_want=$2 text=$3 out rc=0
  shift 3
  out=$("$@" 2>&1) || rc=$?
  if [[ $rc -ne $rc_want || $out != *"$text"* ]]; then
    echo "check-boot-scripts: $what: rc $rc (want $rc_want), said: $out (want: $text)" >&2
    fails=$((fails + 1))
  fi
}

# ---- orion-node-name
name() {  # <conf text> <serial bytes>
  printf '%s' "$1" > "$t/name.conf"
  printf "$2" > "$t/serial"
  ORION_NODE_NAME_CONF=$t/name.conf ORION_NODE_NAME_SERIAL=$t/serial ORION_NODE_NAME_DIR=$t/run \
    ORION_NODE_NAME_HOSTNAME=${hostname_file:-$t/hostname} sh "$name_sh" && cat "$t/run/node-name.env" "$t/hostname"
}
expect "a 16-digit serial with a NUL" 0 "ORION_NODE_NAME=raze-26d57abc" name $'# x\nprefix=raze\n' '1000a317BCbe26d57ABC\000'
expect "an 8-digit serial with spaces" 0 "ORION_NODE_NAME=pv-deadbeef" name $'prefix=pv\n' ' DEADBEEF \n'
rm -f "$t/run/node-name.env"; mkdir -p "$t/run"; echo old > "$t/run/node-name.env"
expect "a serial that is not hex removes the old name" 1 "not hex" name $'prefix=raze\n' 'xyz12345'
[[ ! -e $t/run/node-name.env ]] || { echo "check-boot-scripts: the old name file was left" >&2; fails=$((fails + 1)); }
expect "a short serial" 1 "fewer than 8" name $'prefix=raze\n' 'abc1234'
expect "a prefix with a dot" 1 "no usable prefix" name $'prefix=ra.ze\n' '12345678'
expect "no prefix" 1 "no usable prefix" name $'other=1\n' '12345678'
expect "a 55-byte prefix" 1 "longer than 54" name "prefix=$(printf 'a%.0s' {1..55})" '12345678'
expect "a 54-byte prefix fits Orion's 63" 0 "-12345678" name "prefix=$(printf 'a%.0s' {1..54})" '12345678'
# The hostname is the node name too; a hostname that cannot be written is said, and the node name stays written.
expect "the hostname is the node name" 0 "raze-26d57abc" name $'prefix=raze\n' '1000a317bcbe26d57abc'
[[ $(cat "$t/hostname") == raze-26d57abc ]] || { echo "check-boot-scripts: the hostname file holds $(cat "$t/hostname")" >&2; fails=$((fails + 1)); }
hostname_file=$t/no-such-dir/hostname expect "a hostname that cannot be written" 1 "hostname could not be set" name $'prefix=raze\n' '12345678'
grep -q "ORION_NODE_NAME=raze-12345678" "$t/run/node-name.env" || { echo "check-boot-scripts: the node name was not kept when the hostname failed" >&2; fails=$((fails + 1)); }

# ---- board-journal (the paths that bind nothing)
journal() {  # <mountinfo lines>
  printf '%s' "$1" > "$t/mountinfo"
  BOARD_JOURNAL_DATA=$t/data/journal BOARD_JOURNAL_VAR=$t/var/log/journal BOARD_JOURNAL_ID=/etc/machine-id \
    BOARD_JOURNAL_MOUNTINFO=$t/mountinfo sh "$journal_sh"
}
bound='36 25 179:2 /machine-id /etc/machine-id rw - ext4 /dev/mmcblk0p2 rw
'
mkdir -p "$t/data"
expect "an unbound machine id binds nothing and says so" 0 "the journal stays in RAM" journal '1 0 0:1 / / ro - erofs x ro
'
[[ ! -e $t/data/journal ]] || { echo "check-boot-scripts: unbound, but /data/journal was made" >&2; fails=$((fails + 1)); }
ln -s /etc "$t/data/journal"
expect "a symlinked /data/journal is refused" 1 "is a symlink: refused" journal "$bound"
rm "$t/data/journal"
expect "a journal already mounted is left" 0 "already a mount" journal "$bound""40 25 0:5 / $t/var/log/journal rw - ext4 x rw
"

[[ $fails -eq 0 ]] || { echo "check-boot-scripts: $fails case(s) failed" >&2; exit 1; }
echo "check-boot-scripts: every case passed"
