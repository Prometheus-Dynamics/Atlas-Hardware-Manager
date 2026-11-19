#!/usr/bin/env bash
# The produced rootfs, checked after a build (not the build's target directory): Buildroot's rootfs.tar, the
# filesystem a slot is made from, against what the image's post-fakeroot step left (images/raze-rootfs.expected:
# Buildroot's permissions-table entries after the trim, "path type mode"; os/post-fakeroot.sh writes it).
#
#   scripts/check-rootfs.sh ROOTFS.tar EXPECTED   checks, prints one line per finding, fails on any
#   scripts/check-rootfs.sh --self-test           the check on small tars made here: one passes, each fault fails;
#                                                 the gate runs this (scripts/check-devices.sh self-checks)
#
# It holds:
#   - no usr/libexec/dbus-daemon-launch-helper (the setuid helper of the bus the image does not have);
#   - no dbus user or group, and dbus in no group's members, in etc/passwd, etc/shadow, etc/group, etc/gshadow;
#   - every EXPECTED entry present, of its type (f or d), with its mode: setuid, setgid and sticky bits included.
# The slot's EROFS root is NOT this check's subject: Gaia strips set-uid bits when it makes a slot root unless the
# product lists the file (os/raze.fragment), so a setuid bit there is Gaia's rule, reported by its size report.
set -euo pipefail

check() {  # <rootfs.tar> <expected>
  python3 -I - "$1" "$2" <<'PY'
import sys, tarfile
tar_path, expected_path = sys.argv[1], sys.argv[2]
bad = []
with tarfile.open(tar_path) as t:
    members = {m.name.lstrip("./").rstrip("/"): m for m in t.getmembers()}
    def text(path):
        m = members.get(path)
        if m is None or not m.isfile():
            return None
        return t.extractfile(m).read().decode(errors="replace")
    if "usr/libexec/dbus-daemon-launch-helper" in members:
        bad.append("usr/libexec/dbus-daemon-launch-helper is in the rootfs")
    for f in ("etc/passwd", "etc/shadow", "etc/group", "etc/gshadow"):
        body = text(f)
        if body is None:
            continue
        for n, line in enumerate(body.splitlines(), 1):
            parts = line.split(":")
            if parts[0] == "dbus":
                bad.append(f"{f}:{n}: a dbus entry")
            elif f.endswith("group") and len(parts) >= 4 and "dbus" in parts[3].split(","):
                bad.append(f"{f}:{n}: dbus is a member of {parts[0]}")
    count = 0
    for n, line in enumerate(open(expected_path), 1):
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        path, kind, mode = line.split()
        count += 1
        m = members.get(path)
        if m is None:
            bad.append(f"{path}: missing (the permissions table names it)")
            continue
        if (kind == "f" and not m.isfile()) or (kind == "d" and not m.isdir()):
            bad.append(f"{path}: not a {'file' if kind == 'f' else 'directory'}")
        want = int(mode, 8) & 0o7777
        if m.mode & 0o7777 != want:
            bad.append(f"{path}: mode {m.mode & 0o7777:o}, the permissions table says {want:o}")
for b in bad:
    print(f"check-rootfs: BAD {b}")
if bad:
    print(f"check-rootfs: FAILED ({len(bad)} finding(s))")
    sys.exit(1)
print(f"check-rootfs: ok {tar_path}: no dbus launch helper, no dbus user or group, {count} permissions-table entries hold")
PY
}

self_test() {
  local tmp rc=0
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' RETURN
  mk() {  # <name> <setuid mode of bin/busybox> [extra: helper|user|member]
    local r=$tmp/$1
    mkdir -p "$r/bin" "$r/etc" "$r/usr/libexec"
    printf 'x' > "$r/bin/busybox"; chmod "$2" "$r/bin/busybox"
    printf 'root:x:0:0:root:/root:/bin/sh\n' > "$r/etc/passwd"
    printf 'root:x:0:\nwheel:x:10:root\n' > "$r/etc/group"
    case ${3:-} in
      helper) printf 'x' > "$r/usr/libexec/dbus-daemon-launch-helper" ;;
      user) printf 'dbus:x:81:81:dbus:/:/bin/false\n' >> "$r/etc/passwd" ;;
      member) printf 'audio:x:29:root,dbus\n' >> "$r/etc/group" ;;
    esac
    tar -C "$r" -cf "$tmp/$1.tar" .
  }
  printf '# test\nbin/busybox f 4755\n' > "$tmp/expected"
  mk pass 4755
  check "$tmp/pass.tar" "$tmp/expected" > "$tmp/pass.log" || { cat "$tmp/pass.log" >&2; echo "check-rootfs: self-test: the good rootfs failed" >&2; return 1; }
  local c
  for c in "nosuid 0755 - mode 755" "helper 4755 helper dbus-daemon-launch-helper" "user 4755 user etc/passwd:2" "member 4755 member dbus is a member of audio"; do
    set -- $c
    mk "$1" "$2" "${3#-}"
    if check "$tmp/$1.tar" "$tmp/expected" > "$tmp/$1.log"; then echo "check-rootfs: self-test: $1 passed" >&2; rc=1
    elif ! grep -q -- "${*:4}" "$tmp/$1.log"; then echo "check-rootfs: self-test: $1 failed without naming '${*:4}'" >&2; rc=1; fi
  done
  [[ $rc -eq 0 ]] && echo "check-rootfs: self-test: passed (a good rootfs passes; a lost setuid bit, the dbus helper, a dbus user and dbus as a group member each fail)"
  return $rc
}

case ${1:-} in
  --self-test) self_test ;;
  -h|--help|"") sed -n '2,16p' "$0" ;;
  *) [[ $# -eq 2 ]] || { echo "usage: scripts/check-rootfs.sh ROOTFS.tar EXPECTED" >&2; exit 2; }; check "$1" "$2" ;;
esac
