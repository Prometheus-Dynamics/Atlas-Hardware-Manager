#!/usr/bin/env bash
# The slot root has the directory the data partition is moved onto. Lemnos slot-init (c215f575,
# service/slot-init/src/boot.rs:1433) moves the data partition to <slot root>/data before switch_root, whatever the
# board file says; a root without /data does not start ("can't move the data partition into the slot", the D1
# rehearsal on build 9, 2026-11-20). The board file in the root (/etc/lemnos/board.toml) must name the same place:
# its system-slots `data` key is where lemnosd looks for the partition after the switch.
#
#   scripts/check-slot-mounts.sh EROFS     read with dump.erofs: no mount, no root
#   scripts/check-slot-mounts.sh --self-test
#   scripts/build-product.sh runs it after every full build that makes an EROFS root.
#
# Fails when /data is not a directory of the root, or when the root's board file has a system-slots device whose
# `data` is not "/data". A root without /etc/lemnos/board.toml has no slots device: only /data is checked.
# The boot partition's mount point (`boot`) is not checked: on the Raze it is FLAGGED in the board file until Lemnos
# and Gaia settle where the image mounts it.
set -euo pipefail

check() {  # <erofs>
  local erofs=$1 info board
  command -v dump.erofs >/dev/null || { echo "check-slot-mounts: dump.erofs is needed" >&2; return 2; }
  [[ -f $erofs ]] || { echo "check-slot-mounts: no image: $erofs" >&2; return 2; }
  # dump.erofs exits 0 on a missing path: its words are read instead.
  info=$(dump.erofs --path=/data "$erofs" 2>&1 || true)
  grep -qE '^Size: .*directory$' <<<"$info" \
    || { echo "check-slot-mounts: BAD $erofs: /data is not a directory of the slot root (Lemnos slot-init moves the data partition there)"; return 1; }
  info=$(dump.erofs --path=/etc/lemnos/board.toml "$erofs" 2>&1 || true)
  if grep -qE '^Size: .*regular file$' <<<"$info"; then
    board=$(dump.erofs --path=/etc/lemnos/board.toml --cat "$erofs")
    python3 -I -c '
import sys, tomllib
b = tomllib.loads(sys.stdin.read())
bad = [d.get("id", "?") for d in b.get("devices", []) if d.get("driver") == "system-slots" and d.get("config", {}).get("data", "/data") != "/data"]
for i in bad: print(f"check-slot-mounts: BAD /etc/lemnos/board.toml: system-slots device {i} names a data path other than /data (slot-init moves the partition to /data)")
sys.exit(1 if bad else 0)' <<<"$board" || return 1
  fi
  echo "check-slot-mounts: ok: /data is a directory of the slot root"
}

self_test() {
  local tmp rc=0
  tmp=$(mktemp -d); trap 'rm -rf "$tmp"' RETURN
  command -v mkfs.erofs >/dev/null || { echo "check-slot-mounts: self-test needs mkfs.erofs" >&2; return 2; }
  mk() {  # <name> <data: dir|none> <board data path or "">
    mkdir -p "$tmp/$1/etc/lemnos"
    [[ $2 == dir ]] && mkdir -p "$tmp/$1/data"
    [[ -n $3 ]] && printf '[[devices]]\nid = "slots"\ndriver = "system-slots"\nconfig = { data = "%s" }\n' "$3" > "$tmp/$1/etc/lemnos/board.toml"
    mkfs.erofs "$tmp/$1.erofs" "$tmp/$1" >/dev/null 2>&1
  }
  mk good dir /data; mk nodata none /data; mk elsewhere dir /srv/data; mk noboard dir ""
  check "$tmp/good.erofs" >/dev/null || { echo "check-slot-mounts: self-test: the good root failed" >&2; rc=1; }
  check "$tmp/noboard.erofs" >/dev/null || { echo "check-slot-mounts: self-test: a root without a board file failed" >&2; rc=1; }
  # A failing check is expected below: its words are kept, then read (a pipe would take its status under pipefail).
  check "$tmp/nodata.erofs" > "$tmp/nodata.log" 2>&1 || true
  grep -q '/data is not a directory' "$tmp/nodata.log" || { echo "check-slot-mounts: self-test: a root without /data passed" >&2; rc=1; }
  check "$tmp/elsewhere.erofs" > "$tmp/elsewhere.log" 2>&1 || true
  grep -q 'other than /data' "$tmp/elsewhere.log" || { echo "check-slot-mounts: self-test: a board file naming another data path passed" >&2; rc=1; }
  [[ $rc -eq 0 ]] && echo "check-slot-mounts: self-test: passed (a root without /data and a board file naming another data path each fail)"
  return $rc
}

case ${1:-} in
  --self-test) self_test ;;
  -h|--help|"") sed -n '2,16p' "$0" ;;
  *) [[ $# -eq 1 ]] || { echo "usage: scripts/check-slot-mounts.sh EROFS" >&2; exit 2; }; check "$1" ;;
esac
