#!/usr/bin/env bash
# What ships: whether a slot's units' system call filters are enforced. systemd ignores SystemCallFilter= when it was
# built without seccomp: it logs a line and runs the unit unfiltered, and nothing fails (Lemnos UNIT-SECCOMP, via the
# coordinator 2026-11-21). So a slot whose units set SystemCallFilter= is checked here, at build time.
#
#   scripts/check-slot-seccomp.sh EROFS KERNEL
#       EROFS   the slot root (read with fsck.erofs --extract: no mount, no root)
#       KERNEL  the kernel the slot boots (Image or Image.gz), whose configuration it carries (CONFIG_IKCONFIG)
#   scripts/check-slot-seccomp.sh --self-test   on small EROFS images and kernels made here; the gate runs it
#                                               (check-devices)
#   scripts/build-product.sh runs the check after every build that makes an EROFS root.
#
# A slot with no unit or drop-in setting SystemCallFilter= passes with nothing to check. Otherwise it fails when:
#   - systemd's shared library (usr/lib/systemd/libsystemd-shared-*.so, where PID 1's seccomp code lives) does not
#     need libseccomp (systemd built without seccomp: Buildroot builds it with -Dseccomp=enabled only when
#     BR2_PACKAGE_LIBSECCOMP=y);
#   - the libseccomp it needs is not in the root (usr/lib or lib, links followed inside the root);
#   - the kernel carries no configuration (no IKCFG_ST block: CONFIG_IKCONFIG off), so the next point can't be read;
#   - the kernel's configuration lacks CONFIG_SECCOMP=y or CONFIG_SECCOMP_FILTER=y. lemnosd's gadget helper and root
#     supervisor install their own filters with seccomp(2) and end without SECCOMP_FILTER.
# Each failure names the units that set a filter. readelf is the build host's binutils (it reads aarch64 ELF).
set -euo pipefail

check() {
  local erofs=$1 kernel=$2 tmp rc=0
  for t in fsck.erofs python3 readelf; do command -v "$t" >/dev/null || { echo "check-slot-seccomp: $t is needed" >&2; return 2; }; done
  [[ -f $kernel ]] || { echo "check-slot-seccomp: no kernel file at $kernel" >&2; return 1; }
  tmp=$(mktemp -d)
  fsck.erofs --extract="$tmp/root" "$erofs" >"$tmp/fsck.log" 2>&1 \
    || { cat "$tmp/fsck.log" >&2; rm -rf "$tmp"; echo "check-slot-seccomp: fsck.erofs could not read $erofs" >&2; return 1; }
  python3 -I - "$tmp/root" "$kernel" <<'PY' || rc=$?
import glob, gzip, os, re, subprocess, sys, zlib
root, kernel = sys.argv[1:]
units, seen = [], set()
for top in ("usr/lib/systemd/system", "etc/systemd/system", "lib/systemd/system"):
    base = os.path.join(root, top)
    real = os.path.realpath(base)
    if real in seen or not os.path.isdir(base): continue
    seen.add(real)
    for d, _, files in os.walk(base):
        for n in sorted(files):
            f = os.path.join(d, n)
            if os.path.islink(f) or not (n.endswith((".service", ".conf", ".socket", ".mount", ".swap"))): continue
            for line in open(f, errors="replace"):
                m = re.match(r"\s*SystemCallFilter\s*=\s*(\S.*)$", line)
                if m: units.append(os.path.relpath(f, root)); break
units = sorted(set(units))
if not units:
    print("check-slot-seccomp: ok: no unit sets SystemCallFilter=, nothing to enforce"); sys.exit(0)
named = f"{len(units)} file(s) set SystemCallFilter=: {', '.join(units)}"
bad = []

def inside(rel):
    """rel resolved inside the root (links followed, absolute ones taken from the root)."""
    p, hops = rel.lstrip("/"), 0
    while hops < 40:
        full = os.path.join(root, p)
        if not os.path.islink(full): return p if os.path.exists(full) else None
        t = os.readlink(full); hops += 1
        p = os.path.normpath(t.lstrip("/") if t.startswith("/") else os.path.join(os.path.dirname(p), t))
    return None

shared = sorted(glob.glob(os.path.join(root, "usr/lib/systemd/libsystemd-shared-*.so")))
if len(shared) != 1:
    bad.append(f"no single usr/lib/systemd/libsystemd-shared-*.so in the root ({len(shared)} found): systemd's seccomp can't be read")
else:
    out = subprocess.run(["readelf", "-d", shared[0]], capture_output=True, text=True).stdout
    needed = re.findall(r"\(NEEDED\)\s+Shared library: \[([^\]]+)\]", out)
    sec = [n for n in needed if n.startswith("libseccomp.so")]
    rel = os.path.relpath(shared[0], root)
    if not sec:
        bad.append(f"{rel} does not need libseccomp: systemd was built without seccomp, so these filters are ignored "
                   f"(BR2_PACKAGE_LIBSECCOMP=y builds it with -Dseccomp=enabled)")
    elif not any(inside(f"{d}/{sec[0]}") for d in ("usr/lib", "lib", "usr/lib64", "lib64")):
        bad.append(f"{rel} needs {sec[0]}, which is not in the root")
    else:
        print(f"check-slot-seccomp: {rel} needs {sec[0]}, present")

data = open(kernel, "rb").read()
if data[:2] == b"\x1f\x8b":
    data = zlib.decompressobj(31).decompress(data)
i = data.find(b"IKCFG_ST")
if i < 0:
    bad.append(f"{kernel} carries no configuration (no IKCFG_ST: CONFIG_IKCONFIG off), so its seccomp options can't be read")
else:
    try:
        config = zlib.decompressobj(31).decompress(data[i + 8:]).decode(errors="replace").splitlines()
    except zlib.error as e:
        config = None; bad.append(f"{kernel}: its configuration block does not unpack ({e})")
    if config is not None:
        for k in ("CONFIG_SECCOMP", "CONFIG_SECCOMP_FILTER"):
            if f"{k}=y" not in config:
                bad.append(f"{kernel}: {k}=y is not in the kernel's configuration")
        if not any(f"{k}=y" not in config for k in ("CONFIG_SECCOMP", "CONFIG_SECCOMP_FILTER")):
            print("check-slot-seccomp: the kernel has CONFIG_SECCOMP=y and CONFIG_SECCOMP_FILTER=y")
for b in bad: print(f"check-slot-seccomp: BAD {b}")
if bad: print(f"check-slot-seccomp: FAILED ({len(bad)} finding(s)); {named}"); sys.exit(1)
print(f"check-slot-seccomp: ok: {named}")
PY
  rm -rf "$tmp"
  return $rc
}

self_test() {
  local tmp rc=0
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' RETURN
  for t in mkfs.erofs cc python3; do command -v "$t" >/dev/null || { echo "check-slot-seccomp: self-test needs $t" >&2; return 2; }; done
  # Host ELF files stand in for the aarch64 ones: readelf reads NEEDED the same way.
  printf 'int seccomp_init(void){return 0;}\n' > "$tmp/s.c"
  cc -shared -fPIC -Wl,-soname,libseccomp.so.2 -o "$tmp/libseccomp.so.2" "$tmp/s.c"
  printf 'int seccomp_init(void); int sd(void){return seccomp_init();}\n' > "$tmp/w.c"
  cc -shared -fPIC -o "$tmp/with.so" "$tmp/w.c" -L"$tmp" -l:libseccomp.so.2
  printf 'int sd(void){return 0;}\n' > "$tmp/n.c"
  cc -shared -fPIC -o "$tmp/without.so" "$tmp/n.c"
  kern() {  # <file> <config lines> [raw]: a kernel holding IKCFG_ST + gzip(config) + IKCFG_ED, gzipped unless raw
    python3 -I - "$@" <<'PY'
import gzip, sys
out, cfg = sys.argv[1], sys.argv[2].replace("\\n", "\n")
body = b"\0" * 64 + (b"IKCFG_ST" + gzip.compress(cfg.encode()) + b"IKCFG_ED" if cfg else b"no config here") + b"\0" * 64
open(out, "wb").write(body if len(sys.argv) > 3 else gzip.compress(body))
PY
  }
  kern "$tmp/k-good" 'CONFIG_SECCOMP=y\nCONFIG_SECCOMP_FILTER=y\nCONFIG_IKCONFIG=y\n'
  kern "$tmp/k-raw" 'CONFIG_SECCOMP=y\nCONFIG_SECCOMP_FILTER=y\n' raw
  kern "$tmp/k-nofilter" 'CONFIG_SECCOMP=y\n# CONFIG_SECCOMP_FILTER is not set\n'
  kern "$tmp/k-noconfig" ''
  mk() {  # <name> <with|without> <lib: yes|no> <filter unit: yes|no>
    local r="$tmp/$1"
    mkdir -p "$r/usr/lib/systemd/system/x.service.d" "$r/etc/systemd/system"; ln -s usr/lib "$r/lib"
    cp "$tmp/$2.so" "$r/usr/lib/systemd/libsystemd-shared-258.so"
    [[ $3 == yes ]] && cp "$tmp/libseccomp.so.2" "$r/usr/lib/libseccomp.so.2.6.0" && ln -s libseccomp.so.2.6.0 "$r/usr/lib/libseccomp.so.2"
    printf '[Service]\nExecStart=/bin/true\n' > "$r/usr/lib/systemd/system/x.service"
    [[ $4 == yes ]] && printf '[Service]\nSystemCallFilter=@system-service\n' > "$r/usr/lib/systemd/system/x.service.d/f.conf"
    mkfs.erofs "$tmp/$1.erofs" "$r" >/dev/null 2>&1
  }
  pass() {  # <name> <kernel> <text>
    if ! check "$tmp/$1.erofs" "$2" > "$tmp/$1.log" 2>&1; then echo "check-slot-seccomp: self-test: $1 failed" >&2; cat "$tmp/$1.log" >&2; rc=1
    elif ! grep -q -- "$3" "$tmp/$1.log"; then echo "check-slot-seccomp: self-test: $1 passed without '$3'" >&2; cat "$tmp/$1.log" >&2; rc=1; fi
  }
  expect_fail() {  # <name> <kernel> <text>
    if check "$tmp/$1.erofs" "$2" > "$tmp/$1.log" 2>&1; then echo "check-slot-seccomp: self-test: $1 passed" >&2; rc=1
    elif ! grep -q -- "$3" "$tmp/$1.log"; then echo "check-slot-seccomp: self-test: $1 failed without naming '$3'" >&2; cat "$tmp/$1.log" >&2; rc=1; fi
  }
  mk good with yes yes;          pass good "$tmp/k-good" "usr/lib/systemd/system/x.service.d/f.conf"
  pass good "$tmp/k-raw" "CONFIG_SECCOMP_FILTER=y"
  mk nofilter without no no;     pass nofilter "$tmp/k-noconfig" "no unit sets SystemCallFilter="
  mk nolib without no yes;       expect_fail nolib "$tmp/k-good" "does not need libseccomp: systemd was built without seccomp"
  mk missing with no yes;        expect_fail missing "$tmp/k-good" "needs libseccomp.so.2, which is not in the root"
  expect_fail good "$tmp/k-nofilter" "CONFIG_SECCOMP_FILTER=y is not in the kernel's configuration"
  expect_fail good "$tmp/k-noconfig" "carries no configuration"
  [[ $rc -eq 0 ]] && echo "check-slot-seccomp: self-test: passed (a filter unit with libseccomp linked and both kernel options passes, gzipped or raw; a root with no filter unit passes; systemd without libseccomp, a missing libseccomp, a kernel without SECCOMP_FILTER and a kernel with no configuration each fail by name)"
  return $rc
}

case ${1:-} in
  --self-test) self_test ;;
  -h|--help) sed -n '2,24p' "$0" ;;
  *) [[ $# -eq 2 ]] || { echo "usage: check-slot-seccomp.sh EROFS KERNEL | --self-test" >&2; exit 2; }; check "$1" "$2" ;;
esac
