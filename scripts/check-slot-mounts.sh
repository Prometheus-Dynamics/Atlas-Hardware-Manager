#!/usr/bin/env bash
# What Lemnos slot-init needs in a slot root (Lemnos next 8b10f491's answer to the D1 rehearsal, 2026-11-20):
#   /sbin/init        an executable: slot-init execs it after switch_root (service/slot-init/src/boot.rs:64);
#   /data             a directory: slot-init moves the data partition onto it before switch_root (boot.rs:1432-1438);
#                     a root without it does not start ("can't move the data partition into the slot", build 9);
#   /etc/machine-id   a regular file: slot-init bind-mounts the staged id over it; without it the boot goes on with
#                     "MACHINE-ID REFUSED machine-id-target-missing" and a transient journald id.
# And what Lemnos 7d91d645's slot-init refuses a slot without (SLOT-ROOT-PRECHECK, docs/slots.md "What a slot's root
# must contain"; F3-1): lemnosd feeds the watchdog after slot-init, so a root that can't start it ends in a silent reset:
#   /data             also EMPTY (the data partition is moved onto it);
#   /usr/bin/lemnosd  an executable file;
#   lemnosd.service   enabled: a *.wants/lemnosd.service link in /etc/systemd/system or /usr/lib/systemd/system to the
#                     unit file, and the unit not masked (/etc/systemd/system/lemnosd.service -> /dev/null); or, under
#                     BusyBox init, an executable /etc/init.d/S50lemnosd;
#   user lemnos in /etc/passwd and group lemnos in /etc/group (lemnosd drops to them).
# The board file in the root (/etc/lemnos/board.toml) must name /data as its system-slots `data`: lemnosd looks for
# the partition there after the switch. Paths are resolved through links inside the root, as the slot sees them
# (a merged-usr /sbin, /sbin/init -> ../lib/systemd/systemd).
#
#   scripts/check-slot-mounts.sh EROFS     read with fsck.erofs --extract: no mount, no root
#   scripts/check-slot-mounts.sh --self-test
#   scripts/build-product.sh runs it after every full build that makes an EROFS root.
#
# Every board file in the root (/etc/lemnos/*.toml) whose [board] names a `soc` needs that SoC's description, a regular file at
# /usr/share/lemnos/socs/<soc>.toml: lemnosd checks the live device tree against it (the image contract since
# 2026-11-20; without it lemnosd says "live tree not checked").
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
d = resolve("/data")
if d and os.path.isdir(d) and os.listdir(d):
    bad.append(f"/data is not empty in the slot root ({', '.join(sorted(os.listdir(d))[:5])}): slot-init moves the data partition onto it")
want("/usr/bin/lemnosd", "exec", "the unit starts it; Lemnos slot-init refuses a root without it")
def enabled():
    unit = None
    for base in ("/etc/systemd/system", "/usr/lib/systemd/system"):
        p = resolve(base)
        if not p or not os.path.isdir(p): continue
        for w in sorted(os.listdir(p)):
            if not w.endswith(".wants"): continue
            link = os.path.join(p, w, "lemnosd.service")
            if os.path.islink(link):
                t = os.readlink(link)
                r = resolve(t if t.startswith("/") else os.path.relpath(os.path.join(os.path.dirname(link), t), root))
                if r and os.path.isfile(r): unit = f"{base}/{w}/lemnosd.service"
    masked = os.path.islink(os.path.join(root, "etc/systemd/system/lemnosd.service")) and \
        os.readlink(os.path.join(root, "etc/systemd/system/lemnosd.service")) == "/dev/null"
    if masked: return "masked (/etc/systemd/system/lemnosd.service -> /dev/null)"
    if unit: return None
    s50 = resolve("/etc/init.d/S50lemnosd")
    if s50 and os.path.isfile(s50) and os.stat(s50).st_mode & 0o111: return None
    return "not enabled (no *.wants/lemnosd.service link to the unit file, no executable /etc/init.d/S50lemnosd)"
e = enabled()
if e: bad.append(f"lemnosd.service is {e}: Lemnos slot-init refuses the slot")
for table, name in (("/etc/passwd", "user"), ("/etc/group", "group")):
    p = resolve(table)
    names = set()
    if p and os.path.isfile(p):
        with open(p, encoding="utf-8", errors="replace") as fh: names = {l.split(":", 1)[0] for l in fh if ":" in l}
    if "lemnos" not in names: bad.append(f"no {name} lemnos in {table}: lemnosd drops to it; Lemnos slot-init refuses the slot")
board = resolve("/etc/lemnos/board.toml")
if board and os.path.isfile(board):
    with open(board, "rb") as f: b = tomllib.load(f)
    for d in b.get("devices", []):
        if d.get("driver") == "system-slots" and d.get("config", {}).get("data", "/data") != "/data":
            bad.append(f"/etc/lemnos/board.toml: system-slots device {d.get('id', '?')} names a data path other than /data (slot-init moves the partition to /data)")
socs = set()
etc_lemnos = resolve("/etc/lemnos")
if etc_lemnos and os.path.isdir(etc_lemnos):
    for c in sorted(os.listdir(etc_lemnos)):
        f = os.path.join(etc_lemnos, c)
        if not c.endswith(".toml") or not os.path.isfile(f): continue
        try:
            with open(f, "rb") as fh: doc = tomllib.load(fh)
        except (OSError, tomllib.TOMLDecodeError): continue
        soc = doc.get("board", {}).get("soc") if isinstance(doc.get("board"), dict) else None
        if isinstance(soc, str) and soc and soc not in socs:
            socs.add(soc)
            want(f"/usr/share/lemnos/socs/{soc}.toml", "file", f"/etc/lemnos/{c} names soc {soc}: lemnosd checks the live tree against it")
for x in bad: print(f"check-slot-mounts: BAD {erofs}: {x}")
if bad: print(f"check-slot-mounts: FAILED ({len(bad)} finding(s))"); sys.exit(1)
print("check-slot-mounts: ok: /sbin/init executable, /data an empty directory, /etc/machine-id a regular file, /usr/bin/lemnosd executable, lemnosd.service enabled, user and group lemnos" + (f", SoC description(s) for {', '.join(sorted(socs))}" if socs else ""))
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
    mkdir -p "$r/usr/bin" "$r/usr/lib/systemd/system" "$r/etc/systemd/system/multi-user.target.wants"
    printf x > "$r/usr/bin/lemnosd"; chmod 755 "$r/usr/bin/lemnosd"; printf '[Service]\n' > "$r/usr/lib/systemd/system/lemnosd.service"
    ln -s /usr/lib/systemd/system/lemnosd.service "$r/etc/systemd/system/multi-user.target.wants/lemnosd.service"
    printf 'root:x:0:0::/root:/bin/sh\nlemnos:x:900:900::/:/bin/false\n' > "$r/etc/passwd"
    printf 'root:x:0:\nlemnos:x:900:\n' > "$r/etc/group"
    for a in "$@"; do case $a in
      no-data) rmdir "$r/data" ;;
      no-init) rm "$r/usr/sbin/init" ;;
      noexec-init) chmod 644 "$r/usr/lib/systemd/systemd" ;;
      no-mid) rm "$r/etc/machine-id" ;;
      dir-mid) rm "$r/etc/machine-id"; mkdir "$r/etc/machine-id" ;;
      data=*) data=${a#data=} ;;
      full-data) : > "$r/data/stray" ;;
      no-lemnosd) rm "$r/usr/bin/lemnosd" ;;
      not-enabled) rm "$r/etc/systemd/system/multi-user.target.wants/lemnosd.service" ;;
      masked) ln -s /dev/null "$r/etc/systemd/system/lemnosd.service" ;;
      s50) rm "$r/etc/systemd/system/multi-user.target.wants/lemnosd.service"; mkdir -p "$r/etc/init.d"; printf x > "$r/etc/init.d/S50lemnosd"; chmod 755 "$r/etc/init.d/S50lemnosd" ;;
      no-user) printf 'root:x:0:0::/root:/bin/sh\n' > "$r/etc/passwd" ;;
      no-group) printf 'root:x:0:\n' > "$r/etc/group" ;;
    esac; done
    printf '[board]\nsoc = "x-soc"\n[[devices]]\nid = "slots"\ndriver = "system-slots"\nconfig = { data = "%s" }\n' "$data" > "$r/etc/lemnos/board.toml"
    mkdir -p "$r/usr/share/lemnos/socs"; printf 'name = "x-soc"\n' > "$r/usr/share/lemnos/socs/x-soc.toml"
    for a in "$@"; do case $a in
      no-soc) rm "$r/usr/share/lemnos/socs/x-soc.toml" ;;
      bench-soc) printf '[board]\nsoc = "y-soc"\n' > "$r/etc/lemnos/board.bench.toml" ;;
    esac; done
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
  mk nosoc no-soc;            expect_fail nosoc "/usr/share/lemnos/socs/x-soc.toml is not a regular file"
  mk benchsoc bench-soc;      expect_fail benchsoc "/etc/lemnos/board.bench.toml names soc y-soc"
  mk fulldata full-data;      expect_fail fulldata "/data is not empty"
  mk nolemnosd no-lemnosd;    expect_fail nolemnosd "/usr/bin/lemnosd is not an executable file"
  mk notenabled not-enabled;  expect_fail notenabled "lemnosd.service is not enabled"
  mk masked masked;           expect_fail masked "lemnosd.service is masked"
  mk nouser no-user;          expect_fail nouser "no user lemnos in /etc/passwd"
  mk nogroup no-group;        expect_fail nogroup "no group lemnos in /etc/group"
  mk s50 s50
  check "$tmp/s50.erofs" > "$tmp/s50.log" 2>&1 || { cat "$tmp/s50.log" >&2; echo "check-slot-mounts: self-test: a BusyBox root with S50lemnosd failed" >&2; rc=1; }
  [[ $rc -eq 0 ]] && echo "check-slot-mounts: self-test: passed (no /data, no or non-executable /sbin/init, a missing or non-file /etc/machine-id, and a board file naming another data path, a board file (any /etc/lemnos/*.toml) whose soc has no /usr/share/lemnos/socs/<soc>.toml, a non-empty /data, no /usr/bin/lemnosd, lemnosd.service not enabled or masked, and no user or group lemnos each fail; a BusyBox S50lemnosd passes; links resolve as the slot sees them)"
  return $rc
}

case ${1:-} in
  --self-test) self_test ;;
  -h|--help|"") sed -n '2,28p' "$0" ;;
  *) [[ $# -eq 1 ]] || { echo "usage: scripts/check-slot-mounts.sh EROFS" >&2; exit 2; }; check "$1" ;;
esac
