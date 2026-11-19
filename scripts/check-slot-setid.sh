#!/usr/bin/env bash
# What ships: the setuid and setgid bits of a slot's EROFS root, the filesystem the board boots, checked against
# Buildroot's permissions table. Gaia clears every setuid and setgid bit when it makes the root unless the image lists
# the path in `keep_setid` (product.toml); this check makes each clearing a decision written in Atlas, not a side
# effect.
#
#   scripts/check-slot-setid.sh EROFS EXPECTED PRODUCT.toml IMAGE STRIPPED
#       EROFS     the slot root (read with fsck.erofs --extract: no mount, no root)
#       EXPECTED  images/raze-rootfs.expected (os/post-fakeroot.sh: the permissions-table entries, "path type mode")
#       PRODUCT   the product.toml whose [[images]] entry IMAGE carries keep_setid
#       STRIPPED  the product's stripped list: "path  reason" per line (# comments); every setid entry Atlas lets Gaia
#                 clear, each with its reason
#   scripts/check-slot-setid.sh --self-test   on small EROFS images made here; the gate runs it (check-devices)
#
# Fails when:
#   - a setuid/setgid entry of the table is neither in keep_setid nor in STRIPPED;
#   - a keep_setid path is not setuid/setgid in the EROFS as the table says, or is not in the table;
#   - a STRIPPED path still carries a setuid/setgid bit in the EROFS, is not a setid entry of the table, or has no reason;
#   - any other file in the EROFS carries a setuid/setgid bit.
set -euo pipefail

check() {  # <erofs> <expected> <product.toml> <image> <stripped>
  local erofs=$1 expected=$2 product=$3 image=$4 stripped=$5 tmp rc=0
  for t in fsck.erofs python3; do command -v "$t" >/dev/null || { echo "check-slot-setid: $t is needed" >&2; return 2; }; done
  tmp=$(mktemp -d)
  fsck.erofs --extract="$tmp/root" --preserve-perms "$erofs" >"$tmp/fsck.log" 2>&1 \
    || { cat "$tmp/fsck.log" >&2; rm -rf "$tmp"; echo "check-slot-setid: fsck.erofs could not read $erofs" >&2; return 1; }
  python3 -I - "$tmp/root" "$expected" "$product" "$image" "$stripped" <<'PY' || rc=$?
import os, stat, sys, tomllib
root, expected, product, image, stripped = sys.argv[1:]
SETID = stat.S_ISUID | stat.S_ISGID
bad = []
table = {}
for line in open(expected):
    line = line.strip()
    if not line or line.startswith("#"): continue
    path, kind, mode = line.split()
    if int(mode, 8) & SETID: table[path] = int(mode, 8) & SETID
with open(product, "rb") as f: prod = tomllib.load(f)
imgs = [i for i in prod.get("images", []) if i.get("name") == image]
if len(imgs) != 1: print(f"check-slot-setid: BAD {product}: no single image named {image}"); sys.exit(1)
keep = {p.lstrip("/") for p in imgs[0].get("keep_setid", [])}
strip = {}
for n, line in enumerate(open(stripped), 1):
    s = line.strip()
    if not s or s.startswith("#"): continue
    parts = s.split(None, 1)
    path = parts[0].lstrip("/")
    if len(parts) < 2 or not parts[1].strip(): bad.append(f"{stripped}:{n}: {path} has no reason")
    strip[path] = parts[1] if len(parts) > 1 else ""
def bits(rel):
    p = os.path.join(root, rel)
    if not os.path.lexists(p): return None
    return os.lstat(p).st_mode & SETID
for path, want in sorted(table.items()):
    have = bits(path)
    if path in keep:
        if have is None: bad.append(f"{path}: in keep_setid but not in the EROFS")
        elif have != want: bad.append(f"{path}: in keep_setid, the EROFS has setid bits {have:o}, the table says {want:o}")
    elif path in strip:
        if have: bad.append(f"{path}: on the stripped list, but the EROFS still carries setid bits {have:o}")
    else:
        bad.append(f"{path}: setid {want:o} in the permissions table, neither in keep_setid nor on the stripped list ({stripped}): decide it")
for path in sorted(keep - set(table)):
    bad.append(f"{path}: in keep_setid but not a setid entry of the permissions table")
for path in sorted(set(strip) - set(table)):
    bad.append(f"{path}: on the stripped list but not a setid entry of the permissions table (a stale line)")
for top, dirs, files in os.walk(root):
    for name in dirs + files:
        full = os.path.join(top, name); rel = os.path.relpath(full, root)
        if os.path.islink(full): continue
        if os.lstat(full).st_mode & SETID and rel not in keep:
            bad.append(f"{rel}: carries setid bits in the EROFS but is not in keep_setid")
for b in bad: print(f"check-slot-setid: BAD {b}")
if bad: print(f"check-slot-setid: FAILED ({len(bad)} finding(s))"); sys.exit(1)
print(f"check-slot-setid: ok: {len(table)} setid entr{'y' if len(table) == 1 else 'ies'} of the table: {len(keep)} kept, {len(table) - len(keep)} stripped by decision")
PY
  rm -rf "$tmp"
  return $rc
}

self_test() {
  local tmp rc=0
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' RETURN
  command -v mkfs.erofs >/dev/null || { echo "check-slot-setid: self-test needs mkfs.erofs" >&2; return 2; }
  printf 'bin/su f 4755\nbin/busybox f 4755\nbin/ls f 755\n' > "$tmp/expected"
  printf 'version = 1\n[[images]]\nname = "img"\nkeep_setid = ["/bin/su"]\n' > "$tmp/product.toml"
  printf '# test\nbin/busybox  login runs as root from agetty; no applet needs set-uid\n' > "$tmp/stripped"
  mk() {  # <name> <su mode> <busybox mode>
    mkdir -p "$tmp/$1/bin"; printf x > "$tmp/$1/bin/su"; printf x > "$tmp/$1/bin/busybox"; printf x > "$tmp/$1/bin/ls"
    chmod "$2" "$tmp/$1/bin/su"; chmod "$3" "$tmp/$1/bin/busybox"; chmod 755 "$tmp/$1/bin/ls"
    mkfs.erofs "$tmp/$1.erofs" "$tmp/$1" >/dev/null 2>&1
  }
  expect_fail() {  # <name> <text> [stripped file]
    if check "$tmp/$1.erofs" "$tmp/expected" "$tmp/product.toml" img "${3:-$tmp/stripped}" > "$tmp/$1.log" 2>&1; then
      echo "check-slot-setid: self-test: $1 passed" >&2; rc=1
    elif ! grep -q -- "$2" "$tmp/$1.log"; then echo "check-slot-setid: self-test: $1 failed without naming '$2'" >&2; cat "$tmp/$1.log" >&2; rc=1; fi
  }
  mk pass 4755 755
  check "$tmp/pass.erofs" "$tmp/expected" "$tmp/product.toml" img "$tmp/stripped" > "$tmp/pass.log" 2>&1 \
    || { cat "$tmp/pass.log" >&2; echo "check-slot-setid: self-test: the good image failed" >&2; return 1; }
  mk lost 755 755;        expect_fail lost "bin/su: in keep_setid, the EROFS has setid bits 0"
  mk leaked 4755 4755;    expect_fail leaked "bin/busybox: on the stripped list, but the EROFS still carries"
  printf '# none\n' > "$tmp/none"
  expect_fail pass "bin/busybox: setid 4000 in the permissions table, neither in keep_setid nor on the stripped list" "$tmp/none"
  printf 'bin/busybox\n' > "$tmp/noreason"
  expect_fail pass "has no reason" "$tmp/noreason"
  [[ $rc -eq 0 ]] && echo "check-slot-setid: self-test: passed (a kept bit that goes missing, a stripped bit that leaks, an undecided setid entry and a reasonless line each fail)"
  return $rc
}

case ${1:-} in
  --self-test) self_test ;;
  -h|--help|"") sed -n '2,22p' "$0" ;;
  *) [[ $# -eq 5 ]] || { echo "usage: scripts/check-slot-setid.sh EROFS EXPECTED PRODUCT.toml IMAGE STRIPPED" >&2; exit 2; }; check "$@" ;;
esac
