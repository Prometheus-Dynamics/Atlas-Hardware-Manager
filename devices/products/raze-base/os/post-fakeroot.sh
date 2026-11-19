#!/bin/sh
# The Raze base OS has no D-Bus. Buildroot's post-fakeroot script for raze-base (os/raze.fragment,
# BR2_ROOTFS_POST_FAKEROOT_SCRIPT). Buildroot runs it as `post-fakeroot.sh TARGET_DIR` inside the rootfs step's fakeroot
# script (fs/common.mk), AFTER chown, mkusers (the users table: dbus's user and group), makedevs (the permissions
# table: dbus-daemon-launch-helper 4750) and the pre-command hooks (`systemctl preset-all`), and before the filesystem
# image (rootfs.tar) is made from TARGET_DIR. A removal before makedevs fails the build (D1, 2026-11-18): so the trim is
# here, and os/post-build.sh (the wait-online mask, which must precede preset-all) removes nothing.
#
#   post-fakeroot.sh TARGET_DIR
#       Removes the D-Bus paths listed below, the links preset-all made to dbus's units, and dbus's user and group
#       from etc/passwd, etc/group, etc/shadow (and gshadow); then checks the tree, and checks every entry of
#       Buildroot's permissions table (TARGET_DIR/../../full_devices_table.txt) that this script does not remove is
#       present with its mode, setuid and setgid bits included. Writes the table it checked to
#       $BINARIES_DIR/raze-rootfs.expected, which scripts/check-rootfs.sh holds the produced rootfs.tar to.
#       Any failure exits non-zero.
#   post-fakeroot.sh --self-test SCRATCH ROOT
#       The proof, run by hand (PR B41): ROOT is an extracted raze-base root. In SCRATCH the script runs on COPIES
#       of it, each made as the build with HOSTNAMED off makes the tree (hostnamed's units and binaries removed).
#       The pass copy must pass; the others must each fail, and the raw extracted root must fail on hostnamed.socket.
#
# Why here and not in the fragment: Buildroot's package/systemd/Config.in selects dbus (`select BR2_PACKAGE_DBUS if
# !BR2_PACKAGE_DBUS_BROKER`), so no fragment line can remove it. The trim is done on the installed tree.
# Why it is safe: libdbus-1 is linked only by dbus's own tools (checked on the extracted root); no Lemnos or Orion
# binary uses the bus; the units with BusName= are networkd, resolved, timesyncd, hostnamed and timedated, and the
# fragment turns the last three and hostnamed off. networkd stays on (Type=notify-reload: it starts without the bus).
#
# THE FIXED LIST: derived from the extracted root of Atlas f8e7a2c6 with `find . -path '*dbus*'`, plus dbus's
# sysusers and tmpfiles files and the three enabled wants links. Systemd's own files are not on it: libsystemd and
# systemd-stdio-bridge stay. A directory in the list is removed with its whole tree.
#
# A listed path that is missing fails the build (a Buildroot change then shows here, not as a silent no-op).
# OPTIONAL paths are removed if present: they exist only when a daemon's option is on (the fragment turns those off).
set -eu
set -f

die() { echo "raze-base post-fakeroot: $*" >&2; exit 1; }

# Required: the dbus package's own paths. Each must exist in the target tree. A directory is removed with its tree.
DBUS_REQUIRED='
etc/dbus-1
usr/bin/dbus-cleanup-sockets
usr/bin/dbus-daemon
usr/bin/dbus-launch
usr/bin/dbus-monitor
usr/bin/dbus-run-session
usr/bin/dbus-send
usr/bin/dbus-test-tool
usr/bin/dbus-update-activation-environment
usr/bin/dbus-uuidgen
usr/libexec/dbus-daemon-launch-helper
usr/lib/libdbus-1.so
usr/lib/libdbus-1.so.3
usr/lib/libdbus-1.so.3.38.3
usr/lib/systemd/system/dbus.service
usr/lib/systemd/system/dbus.socket
usr/lib/systemd/system/multi-user.target.wants/dbus.service
usr/lib/systemd/system/sockets.target.wants/dbus.socket
usr/lib/systemd/user/dbus.service
usr/lib/systemd/user/dbus.socket
usr/lib/systemd/user/sockets.target.wants/dbus.socket
usr/lib/sysusers.d/dbus.conf
usr/lib/tmpfiles.d/dbus.conf
usr/share/dbus-1
usr/share/xml/dbus-1
var/lib/dbus
'

# Optional: removed if present. dbus-named activation units of hostnamed and timedated (systemd's files), the
# runtime socket directory tmpfiles makes, and the links preset-all makes for dbus's units (it runs before this).
DBUS_OPTIONAL='
run/dbus
etc/systemd/system/multi-user.target.wants/dbus.service
etc/systemd/system/sockets.target.wants/dbus.socket
etc/systemd/user/sockets.target.wants/dbus.socket
etc/systemd/system/dbus-org.freedesktop.hostname1.service
etc/systemd/system/dbus-org.freedesktop.timedate1.service
etc/systemd/system/dbus-org.freedesktop.network1.service
usr/share/factory/var/lib/dbus
usr/lib/systemd/system/dbus-org.freedesktop.hostname1.service
usr/lib/systemd/system/dbus-org.freedesktop.timedate1.service
'

# The services of the daemons this image turns off (os/raze.fragment). A socket that starts one of them and is enabled
# fails the build: the option removes the daemon, so such a socket would start nothing.
OFF_SERVICES='systemd-resolved.service systemd-timesyncd.service systemd-hostnamed.service systemd-timedated.service'

is_elf() { [ "$(od -An -tx1 -N4 "$1" 2>/dev/null | tr -d ' \n')" = 7f454c46 ]; }

# The file of unit $1 in the tree: the first of etc/systemd/system, usr/lib/systemd/system, usr/lib/systemd/user.
# Prints MASKED when that entry is a mask (a link to /dev/null), and nothing when the unit is not there.
unit_file() {
  for dir in etc/systemd/system usr/lib/systemd/system usr/lib/systemd/user; do
    if [ -L "$T/$dir/$1" ] && [ "$(readlink "$T/$dir/$1")" = /dev/null ]; then echo MASKED; return 0; fi
    if [ -f "$T/$dir/$1" ]; then echo "$T/$dir/$1"; return 0; fi
  done
  return 0
}

# The verb (enable or disable) of the first preset line that matches unit $1, reading the preset files in name order
# (the order preset-all reads them). Prints nothing when no line matches.
preset_says() {
  printf '%s\n' "$presets" | while read -r _b verb pat; do
    [ -n "${pat:-}" ] || continue
    case $1 in $pat) echo "$verb"; break ;; esac
  done
}

trim() {
  T=$1
  [ -d "$T" ] || die "no target directory: $T"

  # 1. Every required path must be there, before anything is removed. All missing ones are named.
  missing=""
  # A var/ path may be in usr/share/factory/var instead: Buildroot's BR2_INIT_SYSTEMD_VAR_FACTORY pre-command hook
  #   (skeleton-init-systemd.mk) moves var/ there before this script runs, and tmpfiles copies it back at boot.
  for p in $DBUS_REQUIRED; do
    if [ ! -e "$T/$p" ] && [ ! -L "$T/$p" ] && [ ! -e "$T/usr/share/factory/$p" ]; then missing="$missing $p"; fi
  done
  [ -z "$missing" ] || die "listed dbus path(s) missing from the target:$missing (the fixed list in os/post-fakeroot.sh is out of date with Buildroot, or the trim would be a silent no-op)"

  # 2. Remove them.
  for p in $DBUS_REQUIRED $DBUS_OPTIONAL; do
    rm -rf "$T/$p"
  done

  # 2a. The factory hook's tmpfiles lines (usr/lib/tmpfiles.d/00-buildroot-var.conf, `C! /var/lib/dbus`) copy the
  #   factory copy back to /var at boot: a line for a removed dbus path goes with it.
  vf=$T/usr/lib/tmpfiles.d/00-buildroot-var.conf
  if [ -f "$vf" ]; then
    grep -v '[[:space:]]/var/lib/dbus\([[:space:]]\|$\)' "$vf" > "$vf.raze-new" || true
    chmod "$(stat -c %a "$vf")" "$vf.raze-new"; mv -f "$vf.raze-new" "$vf"
  fi
  tmpl=$(grep -l 'dbus' "$T"/usr/lib/tmpfiles.d/*.conf "$T"/etc/tmpfiles.d/*.conf 2>/dev/null || true)
  [ -z "$tmpl" ] || die "tmpfiles lines still name dbus in: $(echo $tmpl)"

  # 2b. dbus's user and group, which mkusers wrote from dbus's users table: the line named dbus in passwd, shadow,
  #   group and gshadow, and dbus as a member of any other group.
  for f in etc/passwd etc/shadow etc/group etc/gshadow; do
    [ -f "$T/$f" ] || continue
    awk -F: -v OFS=: '$1 == "dbus" { next }
      FILENAME ~ /group$/ && NF >= 4 { n = split($4, m, ","); out = ""
        for (i = 1; i <= n; i++) if (m[i] != "dbus") out = out (out == "" ? "" : ",") m[i]; $4 = out }
      { print }' "$T/$f" > "$T/$f.raze-new"
    chmod "$(stat -c %a "$T/$f")" "$T/$f.raze-new"
    mv -f "$T/$f.raze-new" "$T/$f"
  done
  users=$(for f in etc/passwd etc/shadow etc/group etc/gshadow; do
    [ -f "$T/$f" ] && awk -F: -v f="$f" '$1 == "dbus" || ("," $4 ",") ~ /,dbus,/ { print f }' "$T/$f"; done || true)
  [ -z "$users" ] || die "dbus is still a user or group in: $(echo $users)"

  # 3. Nothing with dbus in its path may be left. Host find, run in the target tree.
  left=$(cd "$T" && find . -path '*dbus*' | sed 's|^\./||' || true)
  [ -z "$left" ] || die "dbus paths left after the removal (add them to the list or find their owner): $(echo $left)"

  # 4. No ELF file left under the target may need libdbus-1. readelf is the host's GNU binutils: its readelf reads
  #   arm64 ELF without the cross toolchain. Buildroot's TARGET_READELF is a make variable, not exported to this script.
  readelf_bin=${TARGET_READELF:-readelf}
  command -v "$readelf_bin" >/dev/null 2>&1 || die "no $readelf_bin on the build host (binutils); set TARGET_READELF"
  needing=$(find "$T" -type f | while IFS= read -r f; do
    if is_elf "$f" && "$readelf_bin" -d "$f" 2>/dev/null | grep -q 'Shared library: \[libdbus-1\.so'; then
      echo "${f#"$T"/}"
    fi
  done)
  [ -z "$needing" ] || die "ELF file(s) still need libdbus-1 after the removal: $(echo $needing)"

  # 5. Enabled units, as preset-all will leave them. A unit is enabled when a wants or requires link names it, or when the
  #   first preset line matching it says enable. The presets are applied after this script, so this is the set the
  #   presets will produce, read from the files (preset-all also enables a unit no line names: a limit, see the doc).
  wantnames=$(find "$T/etc/systemd" "$T/usr/lib/systemd" -mindepth 1 -type d \( -name '*.wants' -o -name '*.requires' \) -print 2>/dev/null \
    | while IFS= read -r d; do ls -1A "$d"; done | tr '\n' ' ' || true)
  presets=$(find "$T/etc/systemd/system-preset" "$T/usr/lib/systemd/system-preset" -maxdepth 1 -name '*.preset' -print 2>/dev/null \
    | while IFS= read -r f; do b=${f##*/}; grep -E '^(enable|disable)[[:space:]]' "$f" | sed "s|^|$b |"; done | sort -s -k1,1 || true)
  units=$(find "$T/etc/systemd/system" "$T/usr/lib/systemd/system" "$T/usr/lib/systemd/user" -maxdepth 1 \( -name '*.service' -o -name '*.socket' \) -print 2>/dev/null \
    | while IFS= read -r f; do echo "${f##*/}"; done | sort -u || true)
  enabled=""
  for u in $units; do
    case " $wantnames " in
      *" $u "*) enabled="$enabled $u"; continue ;;
    esac
    if [ "$(preset_says "$u")" = enable ]; then enabled="$enabled $u"; fi
  done

  #   - a Type=dbus service fails: it needs the bus to start;
  #   - a socket whose Service= names a missing or masked unit fails (a dangling socket);
  #   - a socket that starts a daemon this image turns off fails (OFF_SERVICES);
  #   - a BusName=-only service, and a socket with no Service= and no same-named service, are notes.
  report=$(for u in $enabled; do
    f=$(unit_file "$u")
    [ "$f" != MASKED ] || continue
    [ -n "$f" ] || continue
    case $u in
      *.service)
        if grep -q '^Type=dbus' "$f"; then echo "TYPE $u"
        elif grep -q '^BusName=' "$f"; then echo "NOTE $u"
        fi ;;
      *.socket)
        explicit=$(sed -n 's/^Service=//p' "$f" | tail -1)
        svc=${explicit:-${u%.socket}.service}
        case " $OFF_SERVICES " in
          *" $svc "*) echo "OFF $u $svc"; continue ;;
        esac
        sf=$(unit_file "$svc")
        if [ -z "$sf" ] || [ "$sf" = MASKED ]; then
          if [ -n "$explicit" ]; then echo "DANGLING $u $svc"; else echo "NOSVC $u"; fi
        fi ;;
    esac
  done || true)
  msgs=$(printf '%s\n' "$report" | while read -r kind a b; do
    case $kind in
      TYPE) echo "unit $a is Type=dbus and enabled: it needs the bus to start, and this image has none" ;;
      DANGLING) echo "socket $a is enabled and names Service=$b, which is missing or masked" ;;
      OFF) echo "socket $a is enabled and starts $b, a daemon this image turns off (os/raze.fragment)" ;;
    esac
  done || true)
  [ -z "$msgs" ] || die "enabled units that fail this image:
$msgs"
  notes=$(printf '%s\n' "$report" | while read -r kind a _b; do
    case $kind in
      NOTE) echo "  $a: BusName= only; it starts without the bus" ;;
      NOSVC) echo "  $a: enabled, with no Service= and no same-named service (systemd serves it, or it is a template)" ;;
    esac
  done || true)
  [ -z "$notes" ] || echo "raze-base post-fakeroot: NOTE enabled units:
$notes"

  # 6. Buildroot's permissions table, after this trim: every f/d entry this script does not remove must be present
  #   with its mode (setuid/setgid bits included; makedevs set them, fakeroot reports them). The table is the one
  #   fs/common.mk made for this filesystem (RAZE_DEVICES_TABLE overrides it, for the self-test).
  table=${RAZE_DEVICES_TABLE:-$T/../../full_devices_table.txt}
  [ -f "$table" ] || die "no permissions table at $table (fs/common.mk's full_devices_table.txt)"
  expected=$(awk '!/^#/ && NF >= 3 && ($2 == "f" || $2 == "d") { print $1, $2, $3 }' "$table" | while read -r path type mode; do
    rel=${path#/}; skip=0
    for p in $DBUS_REQUIRED $DBUS_OPTIONAL; do
      case $rel in "$p" | "$p"/*) skip=1; break ;; esac
    done
    [ "$skip" = 1 ] || echo "$rel $type $mode"
  done)
  bad=$(printf '%s
' "$expected" | while read -r rel type mode; do
    [ -n "$rel" ] || continue
    if [ ! -e "$T/$rel" ]; then echo "$rel: missing"; continue; fi
    have=$(stat -c %a "$T/$rel"); want=$(printf '%o' "0$mode")
    [ "$have" = "$want" ] || echo "$rel: mode $have, the table says $want"
  done)
  [ -z "$bad" ] || die "Buildroot's permissions table is not honoured after the trim: $(echo $bad)"
  if [ -n "${BINARIES_DIR:-}" ]; then
    printf '# raze-base: permissions-table entries left after the trim (path type mode), for scripts/check-rootfs.sh\n%s\n' \
      "$expected" > "$BINARIES_DIR/raze-rootfs.expected"
  fi

  echo "raze-base post-fakeroot: dbus removed from $T (required $(echo $DBUS_REQUIRED | wc -w) paths, optional $(echo $DBUS_OPTIONAL | wc -w)), dbus user and group gone; $(printf '%s\n' "$expected" | grep -c .) permissions-table entries hold; checks passed"
}

self_test() {
  scratch=$1 root=$2
  self=$(cd "$(dirname "$0")" && pwd)/$(basename "$0")
  [ -d "$root" ] || die "--self-test: no root directory: $root"
  mkdir -p "$scratch"
  copy() { rm -rf "$scratch/$1"; cp -a "$root" "$scratch/$1"; }
  # Each copy runs as the build runs it: post-build.sh (the mask), then this script, with a permissions table
  # holding a setuid file the trim keeps (bin/busybox 4755) and dbus's launch helper (which the trim removes).
  printf '/bin/busybox f 4755 0 0 - - - - -\n/usr/libexec/dbus-daemon-launch-helper f 4750 0 0 - - - - -\n' > "$scratch/table"
  run() { sh "$(dirname "$self")/post-build.sh" "$scratch/$1" >"$scratch/$1.log" 2>&1 &&
    RAZE_DEVICES_TABLE=$scratch/table sh "$self" "$scratch/$1" >>"$scratch/$1.log" 2>&1; }
  # The tree the build makes with HOSTNAMED off: hostnamed's units, its link and its binaries are not there.
  built() {
    copy "$1"
    d=$scratch/$1
    rm -f "$d/usr/lib/systemd/system/systemd-hostnamed.service" "$d/usr/lib/systemd/system/systemd-hostnamed.socket" \
      "$d/usr/lib/systemd/system/sockets.target.wants/systemd-hostnamed.socket" "$d/usr/lib/systemd/systemd-hostnamed" \
      "$d/usr/bin/hostnamectl"
  }
  refused_naming() {  # refused_naming CASE TEXT: the case must fail, and its log must name TEXT
    run "$1" && die "self-test: $1 was not refused"
    grep -q -- "$2" "$scratch/$1.log" || die "self-test: $1 refused, but the refusal does not name $2: $(tail -1 "$scratch/$1.log")"
    echo "self-test: fail: $1 is refused, naming $2: $(grep -m1 -- "$2" "$scratch/$1.log")"
  }

  copy asis
  refused_naming asis systemd-hostnamed.socket

  built pass
  run pass || { cat "$scratch/pass.log" >&2; die "self-test: the tree as the HOSTNAMED-off build makes it did not pass"; }
  [ "$(readlink "$scratch/pass/etc/systemd/system/systemd-networkd-wait-online.service")" = /dev/null ] \
    || die "self-test: the pass tree has no wait-online mask"
  echo "self-test: pass: the tree as the HOSTNAMED-off build makes it passes, with the wait-online mask: $(tail -1 "$scratch/pass.log")"
  grep '^  ' "$scratch/pass.log" | sed 's/^/self-test: note: /' || true

  built setuid
  chmod 0755 "$scratch/setuid/bin/busybox"
  refused_naming setuid "bin/busybox: mode 755"

  built withuser
  printf 'dbus:x:81:81:dbus:/:/bin/false\n' >> "$scratch/withuser/etc/passwd"
  run withuser || { cat "$scratch/withuser.log" >&2; die "self-test: a dbus passwd line was not removed"; }
  ! grep -q '^dbus:' "$scratch/withuser/etc/passwd" || die "self-test: the dbus passwd line is still there"
  echo "self-test: pass: a dbus line in etc/passwd is removed"

  built missing
  rm -f "$scratch/missing/usr/bin/dbus-send"
  refused_naming missing usr/bin/dbus-send

  built linked
  cp "$scratch/linked/usr/bin/dbus-send" "$scratch/linked/usr/bin/keep-send"
  refused_naming linked keep-send

  built typed
  printf '[Unit]\nDescription=fake\n[Service]\nType=dbus\nBusName=org.example.Fake\nExecStart=/bin/false\n' \
    >"$scratch/typed/usr/lib/systemd/system/fake-bus.service"
  ln -s ../fake-bus.service "$scratch/typed/usr/lib/systemd/system/multi-user.target.wants/fake-bus.service"
  refused_naming typed fake-bus.service

  built dangling
  printf '[Socket]\nListenStream=/run/fake-dangle\nService=fake-dangle.service\n' \
    >"$scratch/dangling/usr/lib/systemd/system/fake-dangle.socket"
  ln -s ../fake-dangle.socket "$scratch/dangling/usr/lib/systemd/system/sockets.target.wants/fake-dangle.socket"
  refused_naming dangling fake-dangle.socket

  echo "self-test: passed (the raw root is refused on hostnamed.socket; the HOSTNAMED-off tree passes with the mask; a setuid file whose table mode is not held, a deleted listed path, a libdbus-1 ELF, an enabled Type=dbus unit and a dangling socket are each refused)"
}

case ${1:-} in
  --self-test)
    [ $# -eq 3 ] || die "usage: post-fakeroot.sh --self-test SCRATCH ROOT"
    self_test "$2" "$3"
    ;;
  -h|--help)
    sed -n '2,15p' "$0"
    ;;
  "")
    die "usage: post-fakeroot.sh TARGET_DIR"
    ;;
  *)
    [ $# -eq 1 ] || die "usage: post-fakeroot.sh TARGET_DIR"
    trim "$1"
    ;;
esac
