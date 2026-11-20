#!/usr/bin/env bash
# What Lemnos slot-init needs in a slot root (Lemnos next 8b10f491's answer to the D1 rehearsal, 2026-11-20):
#   /sbin/init        an executable: slot-init execs it after switch_root (service/slot-init/src/boot.rs:64);
#   /data             a directory: slot-init moves the data partition onto it before switch_root (boot.rs:1432-1438);
#                     a root without it does not start ("can't move the data partition into the slot", build 9);
#   /etc/machine-id   a regular file: slot-init bind-mounts the staged id over it; without it the boot goes on with
#                     "MACHINE-ID REFUSED machine-id-target-missing" and a transient journald id.
# The board file in the root (/etc/lemnos/board.toml) must name /data as its system-slots `data`: lemnosd looks for
# the partition there after the switch. Paths are resolved through links inside the root, as the slot sees them
# (a merged-usr /sbin, /sbin/init -> ../lib/systemd/systemd).
#
#   scripts/check-slot-mounts.sh EROFS     read with fsck.erofs --extract: no mount, no root
#   scripts/check-slot-mounts.sh --self-test
#   scripts/build-product.sh runs it after every full build that makes an EROFS root.
#
# The boot partition's mount point is not checked: on Raze's tryboot record form nobody mounts it after slot-init
# (records are raw sector writes found through sysfs; Lemnos 8b10f491); `boot` matters for the file and ubi forms.
set -euo pipefail

check() {  # <erofs>
  local erofs=$1 tmp rc=0
  for t in fsck.erofs python3; do command -v "$t" >/dev/null || { echo "check-slot-mounts: $t is needed" >&2; return 2; }; done
  [[ -f $erofs ]] || { echo "check-slot-mounts: no image: $erofs" >&2; return 2; }
  tmp=$(mktemp -d)
  fsck.erofs --extract="$tmp/root" --preserve-perms "$erofs" >"$tmp/fsck.log" 2>&1 \
    || { cat "$tmp/fsck.log" >&2; rm -rf "$tmp"; echo "check-slot-mounts: fsck.erofs could not read $erofs" >&2; return 1; }
  python3 -I - "$tmp/root" "$erofs" <<'PY' || rc=$?
import os, stat, sys, tomllib
root, erofs = sys.argv[1:]
def resolve(rel):
    """rel as the slot sees it: every link inside the root followed (absolute ones from the root); None if missing."""
    todo, cur, hops = [p for p in rel.strip("/").split("/") if p], [], 0
    while todo:
        p = todo.pop(0)
        if p in ("", "."): continue
        if p == "..": cur = cur[:-1]; continue
        full = os.path.join(root, *cur, p)
        if os.path.islink(full):
            hops += 1
            if hops > 40: return None
            t = os.readlink(full)
            if t.startswith("/"): cur = []
            todo = [x for x in t.split("/") if x] + todo
            continue
        if not os.path.lexists(full): return None
        cur.append(p)
    return os.path.join(root, *cur)
bad = []
def want(rel, kind, why):
    p = resolve(rel)
    st = os.lstat(p) if p else None
    ok = st is not None and {
        "dir": lambda: stat.S_ISDIR(st.st_mode),
        "file": lambda: stat.S_ISREG(st.st_mode),
        "exec": lambda: stat.S_ISREG(st.st_mode) and st.st_mode & 0o111,
    }[kind]()
    if not ok: bad.append(f"{rel} is not {dict(dir='a directory', file='a regular file', exec='an executable file')[kind]} of the slot root ({why})")
want("/sbin/init", "exec", "slot-init execs it after switch_root")
want("/data", "dir", "slot-init moves the data partition onto it")
want("/etc/machine-id", "file", "slot-init bind-mounts the machine id over it")
board = resolve("/etc/lemnos/board.toml")
if board and os.path.isfile(board):
    with open(board, "rb") as f: b = tomllib.load(f)
    for d in b.get("devices", []):
        if d.get("driver") == "system-slots" and d.get("config", {}).get("data", "/data") != "/data":
            bad.append(f"/etc/lemnos/board.toml: system-slots device {d.get('id', '?')} names a data path other than /data (slot-init moves the partition to /data)")
for x in bad: print(f"check-slot-mounts: BAD {erofs}: {x}")
if bad: print(f"check-slot-mounts: FAILED ({len(bad)} finding(s))"); sys.exit(1)
print("check-slot-mounts: ok: /sbin/init executable, /data a directory, /etc/machine-id a regular file")
PY
  rm -rf "$tmp"
  return $rc
}

self_test() {
  local tmp rc=0
  tmp=$(mktemp -d); trap 'rm -rf "$tmp"' RETURN
  command -v mkfs.erofs >/dev/null || { echo "check-slot-mounts: self-test needs mkfs.erofs" >&2; return 2; }
  mk() {  # <name> [no-data] [no-init] [noexec-init] [no-mid] [dir-mid] [data=PATH]
    local n=$1 a data=/data; shift
    local r="$tmp/$n"
    # merged usr and a linked init, as on the Raze root
    mkdir -p "$r/usr/sbin" "$r/usr/lib/systemd" "$r/etc/lemnos"; ln -s usr/sbin "$r/sbin"
    printf x > "$r/usr/lib/systemd/systemd"; chmod 755 "$r/usr/lib/systemd/systemd"
    ln -s ../lib/systemd/systemd "$r/usr/sbin/init"
    mkdir "$r/data"; : > "$r/etc/machine-id"
    for a in "$@"; do case $a in
      no-data) rmdir "$r/data" ;;
      no-init) rm "$r/usr/sbin/init" ;;
      noexec-init) chmod 644 "$r/usr/lib/systemd/systemd" ;;
      no-mid) rm "$r/etc/machine-id" ;;
      dir-mid) rm "$r/etc/machine-id"; mkdir "$r/etc/machine-id" ;;
      data=*) data=${a#data=} ;;
    esac; done
    printf '[[devices]]\nid = "slots"\ndriver = "system-slots"\nconfig = { data = "%s" }\n' "$data" > "$r/etc/lemnos/board.toml"
    mkfs.erofs "$tmp/$n.erofs" "$r" >/dev/null 2>&1
  }
  expect_fail() {  # <name> <text>   (a failing check is expected: its words are kept, then read)
    check "$tmp/$1.erofs" > "$tmp/$1.log" 2>&1 && { echo "check-slot-mounts: self-test: $1 passed" >&2; rc=1; return; }
    grep -qF -- "$2" "$tmp/$1.log" || { echo "check-slot-mounts: self-test: $1 failed without naming '$2'" >&2; cat "$tmp/$1.log" >&2; rc=1; }
  }
  mk good
  check "$tmp/good.erofs" > "$tmp/good.log" 2>&1 || { cat "$tmp/good.log" >&2; echo "check-slot-mounts: self-test: the good root failed" >&2; rc=1; }
  mk nodata no-data;          expect_fail nodata "/data is not a directory"
  mk noinit no-init;          expect_fail noinit "/sbin/init is not an executable file"
  mk noexec noexec-init;      expect_fail noexec "/sbin/init is not an executable file"
  mk nomid no-mid;            expect_fail nomid "/etc/machine-id is not a regular file"
  mk dirmid dir-mid;          expect_fail dirmid "/etc/machine-id is not a regular file"
  mk elsewhere data=/srv/data; expect_fail elsewhere "other than /data"
  [[ $rc -eq 0 ]] && echo "check-slot-mounts: self-test: passed (no /data, no or non-executable /sbin/init, a missing or non-file /etc/machine-id, and a board file naming another data path each fail; links resolve as the slot sees them)"
  return $rc
}

case ${1:-} in
  --self-test) self_test ;;
  -h|--help|"") sed -n '2,19p' "$0" ;;
  *) [[ $# -eq 1 ]] || { echo "usage: scripts/check-slot-mounts.sh EROFS" >&2; exit 2; }; check "$1" ;;
esac
