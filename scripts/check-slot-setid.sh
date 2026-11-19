#!/usr/bin/env bash
# What ships: the setuid and setgid bits of a slot's EROFS root, the filesystem the board boots, checked against the
# set-id bits of the tree it was made from (Buildroot's root and every component, merged). Gaia clears every setuid and
# setgid bit when it makes the root unless the image lists the path in `keep_setid` (product.toml); this check makes
# each clearing a decision written in Atlas, not a side effect.
#
#   scripts/check-slot-setid.sh EROFS SETID PRODUCT.toml IMAGE STRIPPED
#       EROFS     the slot root (read with fsck.erofs --extract: no mount, no root)
#       SETID     the EROFS step's `setid` output: one line per set-id bit of its input tree, `cleared /path MODE -> MODE`
#                 or `kept    /path MODE` (Gaia fs-erofs write_tar)
#       PRODUCT   the product.toml whose [[images]] entry IMAGE carries keep_setid
#       STRIPPED  the product's stripped list: "path  reason" per line (# comments); every setid entry Atlas lets Gaia
#                 clear, each with its reason
#   scripts/check-slot-setid.sh --self-test   on small EROFS images made here; the gate runs it (check-devices)
#   scripts/build-product.sh runs the check after a build for a product that has os/setid-stripped.
#
# Paths are compared after resolving directory links inside the root, so `bin/busybox` and `usr/bin/busybox` are one
# file on a merged-usr root.
#
# Fails when:
#   - a set-id bit of the input tree (SETID) is neither in keep_setid nor in STRIPPED;
#   - a keep_setid path is not setuid/setgid in the EROFS as SETID says, or is not a set-id file of the input;
#   - a STRIPPED path still carries a setuid/setgid bit in the EROFS, is not a set-id file of the input, or has no reason;
#   - any other file in the EROFS carries a setuid/setgid bit.
set -euo pipefail

check() {  # <erofs> <setid report> <product.toml> <image> <stripped>
  local erofs=$1 report=$2 product=$3 image=$4 stripped=$5 tmp rc=0
  for t in fsck.erofs python3; do command -v "$t" >/dev/null || { echo "check-slot-setid: $t is needed" >&2; return 2; }; done
  tmp=$(mktemp -d)
  fsck.erofs --extract="$tmp/root" --preserve-perms "$erofs" >"$tmp/fsck.log" 2>&1 \
    || { cat "$tmp/fsck.log" >&2; rm -rf "$tmp"; echo "check-slot-setid: fsck.erofs could not read $erofs" >&2; return 1; }
  python3 -I - "$tmp/root" "$report" "$product" "$image" "$stripped" <<'PY' || rc=$?
import os, re, stat, sys, tomllib
root, report, product, image, stripped = sys.argv[1:]
SETID = stat.S_ISUID | stat.S_ISGID
bad = []
def canon(rel):
    """rel with every directory link inside the root resolved (not the last part)."""
    parts = [p for p in rel.strip("/").split("/") if p not in ("", ".")]
    cur, hops = "", 0
    for p in parts[:-1]:
        nxt = os.path.normpath(os.path.join(cur, p))
        while os.path.islink(os.path.join(root, nxt)) and hops < 40:
            hops += 1
            t = os.readlink(os.path.join(root, nxt))
            nxt = os.path.normpath(t.lstrip("/") if t.startswith("/") else os.path.join(os.path.dirname(nxt), t))
        cur = nxt
    return os.path.normpath(os.path.join(cur, parts[-1])) if parts else "."
table = {}
for n, line in enumerate(open(report), 1):
    if not line.strip(): continue
    m = re.match(r"^(cleared|kept)\s+/(\S+) ([0-7]{4})(?: -> [0-7]{4})?$", line.rstrip("\n"))
    if not m: print(f"check-slot-setid: BAD {report}:{n}: not a line of Gaia's setid report: {line.strip()}"); sys.exit(1)
    table[canon(m.group(2))] = int(m.group(3), 8) & SETID
with open(product, "rb") as f: prod = tomllib.load(f)
imgs = [i for i in prod.get("images", []) if i.get("name") == image]
if len(imgs) != 1: print(f"check-slot-setid: BAD {product}: no single image named {image}"); sys.exit(1)
keep = {canon(p) for p in imgs[0].get("keep_setid", [])}
strip = {}
for n, line in enumerate(open(stripped), 1):
    s = line.strip()
    if not s or s.startswith("#"): continue
    parts = s.split(None, 1)
    path = canon(parts[0])
    if len(parts) < 2 or not parts[1].strip(): bad.append(f"{stripped}:{n}: {parts[0]} has no reason")
    strip[path] = parts[1] if len(parts) > 1 else ""
def bits(rel):
    p = os.path.join(root, rel)
    if not os.path.lexists(p): return None
    return os.lstat(p).st_mode & SETID
for path, want in sorted(table.items()):
    have = bits(path)
    if path in keep:
        if have is None: bad.append(f"{path}: in keep_setid but not in the EROFS")
        elif have != want: bad.append(f"{path}: in keep_setid, the EROFS has setid bits {have:o}, the input had {want:o}")
    elif path in strip:
        if have: bad.append(f"{path}: on the stripped list, but the EROFS still carries setid bits {have:o}")
    else:
        bad.append(f"{path}: setid {want:o} in the input tree, neither in keep_setid nor on the stripped list ({stripped}): decide it")
for path in sorted(keep - set(table)):
    bad.append(f"{path}: in keep_setid but not a set-id file of the input tree")
for path in sorted(set(strip) - set(table)):
    bad.append(f"{path}: on the stripped list but not a set-id file of the input tree (a stale line)")
for top, dirs, files in os.walk(root):
    for name in dirs + files:
        full = os.path.join(top, name); rel = os.path.relpath(full, root)
        if os.path.islink(full): continue
        if os.lstat(full).st_mode & SETID and rel not in keep:
            bad.append(f"{rel}: carries setid bits in the EROFS but is not in keep_setid")
for b in bad: print(f"check-slot-setid: BAD {b}")
if bad: print(f"check-slot-setid: FAILED ({len(bad)} finding(s))"); sys.exit(1)
print(f"check-slot-setid: ok: {len(table)} setid entr{'y' if len(table) == 1 else 'ies'} in the input tree: {len(keep)} kept, {len(table) - len(keep)} stripped by decision")
PY
  rm -rf "$tmp"
  return $rc
}

self_test() {
  local tmp rc=0
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' RETURN
  command -v mkfs.erofs >/dev/null || { echo "check-slot-setid: self-test needs mkfs.erofs" >&2; return 2; }
  # A merged-usr root: bin -> usr/bin. Gaia's report names usr/bin; the lists name bin/ (resolved to one file).
  printf 'kept    /usr/bin/su 4755\ncleared /usr/bin/busybox 4755 -> 0755\n' > "$tmp/setid.txt"
  printf 'version = 1\n[[images]]\nname = "img"\nkeep_setid = ["/bin/su"]\n' > "$tmp/product.toml"
  printf '# test\nbin/busybox  login runs as root from agetty; no applet needs set-uid\n' > "$tmp/stripped"
  mk() {  # <name> <su mode> <busybox mode>
    mkdir -p "$tmp/$1/usr/bin" "$tmp/$1/tmp"; ln -s usr/bin "$tmp/$1/bin"
    printf x > "$tmp/$1/usr/bin/su"; printf x > "$tmp/$1/usr/bin/busybox"; printf x > "$tmp/$1/usr/bin/ls"
    chmod "$2" "$tmp/$1/usr/bin/su"; chmod "$3" "$tmp/$1/usr/bin/busybox"; chmod 755 "$tmp/$1/usr/bin/ls"
    mkfs.erofs "$tmp/$1.erofs" "$tmp/$1" >/dev/null 2>&1
  }
  expect_fail() {  # <name> <text> [stripped file]
    if check "$tmp/$1.erofs" "$tmp/setid.txt" "$tmp/product.toml" img "${3:-$tmp/stripped}" > "$tmp/$1.log" 2>&1; then
      echo "check-slot-setid: self-test: $1 passed" >&2; rc=1
    elif ! grep -q -- "$2" "$tmp/$1.log"; then echo "check-slot-setid: self-test: $1 failed without naming '$2'" >&2; cat "$tmp/$1.log" >&2; rc=1; fi
  }
  mk pass 4755 755
  check "$tmp/pass.erofs" "$tmp/setid.txt" "$tmp/product.toml" img "$tmp/stripped" > "$tmp/pass.log" 2>&1 \
    || { cat "$tmp/pass.log" >&2; echo "check-slot-setid: self-test: the good image failed" >&2; return 1; }
  mk lost 755 755;        expect_fail lost "usr/bin/su: in keep_setid, the EROFS has setid bits 0"
  mk leaked 4755 4755;    expect_fail leaked "usr/bin/busybox: on the stripped list, but the EROFS still carries"
  printf '# none\n' > "$tmp/none"
  expect_fail pass "usr/bin/busybox: setid 4000 in the input tree, neither in keep_setid nor on the stripped list" "$tmp/none"
  printf 'bin/busybox\n' > "$tmp/noreason"
  expect_fail pass "has no reason" "$tmp/noreason"
  printf 'bin/busybox  a reason\nbin/ls  stale\n' > "$tmp/stale"
  expect_fail pass "usr/bin/ls: on the stripped list but not a set-id file" "$tmp/stale"
  printf 'garbage line\n' > "$tmp/bad.txt"
  if check "$tmp/pass.erofs" "$tmp/bad.txt" "$tmp/product.toml" img "$tmp/stripped" > "$tmp/bad.log" 2>&1 || ! grep -q "not a line of Gaia's setid report" "$tmp/bad.log"; then
    echo "check-slot-setid: self-test: an unreadable report was not refused" >&2; rc=1
  fi
  [[ $rc -eq 0 ]] && echo "check-slot-setid: self-test: passed (a kept bit that goes missing, a stripped bit that leaks, an undecided setid entry, a reasonless line, a stale line and an unreadable report each fail; bin/ resolves to usr/bin/)"
  return $rc
}

case ${1:-} in
  --self-test) self_test ;;
  -h|--help|"") sed -n '2,27p' "$0" ;;
  *) [[ $# -eq 5 ]] || { echo "usage: scripts/check-slot-setid.sh EROFS SETID PRODUCT.toml IMAGE STRIPPED" >&2; exit 2; }; check "$@" ;;
esac
