#!/bin/sh
# The Raze base OS has no D-Bus. Buildroot's post-build script for raze-base (os/raze.fragment, BR2_ROOTFS_POST_BUILD_SCRIPT).
# Buildroot runs it as `post-build.sh TARGET_DIR` after the packages are installed, before the image is made.
#
#   post-build.sh TARGET_DIR
#       Removes the D-Bus paths listed below from TARGET_DIR, then checks what is left. Any failure exits non-zero.
#   post-build.sh --self-test SCRATCH ROOT
#       The proof, run by hand (PR B41): ROOT is an extracted raze-base root. In SCRATCH the script runs on COPIES
#       of it: the copy must pass; the same copy with a listed path deleted, with a dbus-linked ELF kept, or with an
#       enabled Type=dbus unit added, must each fail.
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

die() { echo "raze-base post-build: $*" >&2; exit 1; }

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

# Optional: removed if present. dbus-named activation units of hostnamed and timedated (systemd's files), and the
# runtime socket directory tmpfiles makes.
DBUS_OPTIONAL='
run/dbus
usr/lib/systemd/system/dbus-org.freedesktop.hostname1.service
usr/lib/systemd/system/dbus-org.freedesktop.timedate1.service
'

is_elf() { [ "$(od -An -tx1 -N4 "$1" 2>/dev/null | tr -d ' \n')" = 7f454c46 ]; }

trim() {
  T=$1
  [ -d "$T" ] || die "no target directory: $T"

  # 1. Every required path must be there, before anything is removed. All missing ones are named.
  missing=""
  for p in $DBUS_REQUIRED; do
    if [ ! -e "$T/$p" ] && [ ! -L "$T/$p" ]; then missing="$missing $p"; fi
  done
  [ -z "$missing" ] || die "listed dbus path(s) missing from the target:$missing (the fixed list in os/post-build.sh is out of date with Buildroot, or the trim would be a silent no-op)"

  # 2. Remove them.
  for p in $DBUS_REQUIRED $DBUS_OPTIONAL; do
    rm -rf "$T/$p"
  done

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

  # 5. Enabled units: a unit linked from a *.wants or *.requires dir under etc/systemd or usr/lib/systemd. A Type=dbus
  #   unit needs the bus to start: that one fails the build. A BusName=-only unit (networkd is one) starts without the
  #   bus: it is listed as a note.
  enabled=$(find "$T/etc/systemd" "$T/usr/lib/systemd" -mindepth 1 -type d \( -name '*.wants' -o -name '*.requires' \) -print 2>/dev/null \
    | while IFS= read -r d; do ls -1A "$d"; done | sort -u || true)
  bus_units=$(printf '%s\n' "$enabled" | while IFS= read -r n; do
    [ -n "$n" ] || continue
    for dir in etc/systemd/system usr/lib/systemd/system usr/lib/systemd/user; do
      u=$T/$dir/$n
      if [ -f "$u" ]; then
        if grep -q '^Type=dbus' "$u"; then echo "TYPE $dir/$n"
        elif grep -q '^BusName=' "$u"; then echo "NOTE $dir/$n"
        fi
        break
      fi
    done
  done || true)
  types=$(printf '%s\n' "$bus_units" | grep '^TYPE ' || true)
  [ -z "$types" ] || die "enabled unit(s) with Type=dbus (they need the bus to start, and there is none): $types"
  notes=$(printf '%s\n' "$bus_units" | grep '^NOTE ' | sed 's/^NOTE //' | tr '\n' ' ' || true)
  [ -z "$notes" ] || echo "raze-base post-build: NOTE enabled unit(s) with BusName= that start without the bus: $notes"

  echo "raze-base post-build: dbus removed from $T (required $(echo $DBUS_REQUIRED | wc -w) paths, optional $(echo $DBUS_OPTIONAL | wc -w)); checks passed"
}

self_test() {
  scratch=$1 root=$2
  self=$(cd "$(dirname "$0")" && pwd)/$(basename "$0")
  [ -d "$root" ] || die "--self-test: no root directory: $root"
  mkdir -p "$scratch"
  copy() { rm -rf "$scratch/$1"; cp -a "$root" "$scratch/$1"; }
  run() { sh "$self" "$scratch/$1" >"$scratch/$1.log" 2>&1; }

  copy pass
  run pass || { cat "$scratch/pass.log" >&2; die "self-test: the extracted root did not pass"; }
  echo "self-test: pass: the extracted root passes ($(tail -1 "$scratch/pass.log"))"

  copy missing
  rm -f "$scratch/missing/usr/bin/dbus-send"
  if run missing; then die "self-test: a root with usr/bin/dbus-send deleted was not refused"; fi
  grep -q 'usr/bin/dbus-send' "$scratch/missing.log" || die "self-test: the missing-path refusal does not name the path"
  echo "self-test: fail: a listed path deleted is refused, naming it: $(grep 'missing' "$scratch/missing.log")"

  copy linked
  cp "$scratch/linked/usr/bin/dbus-send" "$scratch/linked/usr/bin/keep-send"
  if run linked; then die "self-test: a dbus-linked ELF left in the tree was not refused"; fi
  grep -q 'keep-send' "$scratch/linked.log" || die "self-test: the ELF refusal does not name the file"
  echo "self-test: fail: a libdbus-1 ELF left in the tree is refused, naming it"

  copy typed
  printf '[Unit]\nDescription=fake\n[Service]\nType=dbus\nBusName=org.example.Fake\nExecStart=/bin/false\n' \
    >"$scratch/typed/usr/lib/systemd/system/fake-bus.service"
  ln -s ../fake-bus.service "$scratch/typed/usr/lib/systemd/system/multi-user.target.wants/fake-bus.service"
  if run typed; then die "self-test: an enabled Type=dbus unit was not refused"; fi
  grep -q 'fake-bus.service' "$scratch/typed.log" || die "self-test: the unit refusal does not name the unit"
  echo "self-test: fail: an enabled Type=dbus unit is refused, naming it"

  echo "self-test: passed (the extracted root passes; a deleted listed path, a libdbus-1 ELF and an enabled Type=dbus unit are each refused)"
}

case ${1:-} in
  --self-test)
    [ $# -eq 3 ] || die "usage: post-build.sh --self-test SCRATCH ROOT"
    self_test "$2" "$3"
    ;;
  -h|--help)
    sed -n '2,12p' "$0"
    ;;
  "")
    die "usage: post-build.sh TARGET_DIR"
    ;;
  *)
    [ $# -eq 1 ] || die "usage: post-build.sh TARGET_DIR"
    trim "$1"
    ;;
esac
