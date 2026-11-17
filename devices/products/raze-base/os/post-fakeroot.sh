#!/bin/sh
# The raze-base post-fakeroot script: the whole D-Bus removal and every check of the trim. Named by
# BR2_ROOTFS_POST_FAKEROOT_SCRIPT (os/raze.fragment) in the commit that pins the Gaia rev whose rootfs is the fakeroot
# stage's result. Until then it is NOT named by any fragment: the image keeps D-Bus (Gaia's timing copy does too).
#
#   post-fakeroot.sh TARGET_DIR                 the trim, on the per-filesystem copy
#   post-fakeroot.sh --self-test SCRATCH ROOT   the proof, on copies of a fake or built root
#
# WHERE IT RUNS (Buildroot, d5180309; the order is the reason this script is here, not in post-build.sh):
#   fs/common.mk:179  mkusers       adds the package users (dbus, systemd-network, ...) to the copy's /etc
#   fs/common.mk:180  makedevs      applies the permission table: DBUS_PERMISSIONS names the helper (dbus.mk:32-34),
#                                   and makedevs fails if the file is missing (makedevs.c:578-582): the helper must
#                                   exist here, so it is removed AFTER makedevs, by this script
#   fs/common.mk:181  ROOTFS_PRE_CMD_HOOKS: `systemctl preset-all` (systemd.mk:834, :836) enables and disables units
#                                   on the copy BEFORE this script runs; a dbus unit a preset enabled is linked by then,
#                                   and this script removes it and asserts that no link is left dangling
#   fs/common.mk:183-185  this script (BR2_ROOTFS_POST_FAKEROOT_SCRIPT), run as `post-fakeroot.sh TARGET_DIR`
#   fs/common.mk:196  the copy is deleted: this script's changes reach the image only through the fakeroot-stage root
#                     (Gaia's fix, pinned with this script's wiring).
#
# THE FIXED LIST (derived from the extracted root of Atlas f8e7a2c6 with `find . -path '*dbus*'`, plus dbus's sysusers and
# tmpfiles files and the three enabled wants links; the helper is in it because makedevs has run). A listed path that is
# missing fails the trim. A directory is removed with its whole tree. OPTIONAL paths are removed if present.
# dbus 1.16.2 (dbus.mk:9); libdbus soname 3.38.3 (dbus meson.build:63-80).
#
# THE CHECKS, after the removal: no path with dbus in it; no ELF file left that needs libdbus-1 (readelf -d, DT_NEEDED);
# the enabled units (the wants links preset-all made) are not Type=dbus, do not start a daemon this image turns off, and
# their sockets name a unit that exists; no wants link is dangling; the wait-online mask post-build.sh placed is still there.
set -eu
set -f

die() { echo "raze-base post-fakeroot: $*" >&2; exit 1; }

# Required: the dbus package's own paths, the helper included. Each must exist in the tree when this script runs.
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

# Optional: removed if present (the dbus-named activation units of hostnamed and timedated, and the runtime socket directory).
DBUS_OPTIONAL='
run/dbus
usr/lib/systemd/system/dbus-org.freedesktop.hostname1.service
usr/lib/systemd/system/dbus-org.freedesktop.timedate1.service
'

# The services of the daemons this image turns off (os/raze.fragment). A socket that starts one of them and is enabled
# fails the trim: the option removed the daemon, so the socket would start nothing.
OFF_SERVICES='systemd-resolved.service systemd-timesyncd.service systemd-hostnamed.service systemd-timedated.service'

WAIT_ONLINE=etc/systemd/system/systemd-networkd-wait-online.service

is_elf() { [ "$(od -An -tx1 -N4 "$1" 2>/dev/null | tr -d ' \n')" = 7f454c46 ]; }

# The file of unit $2 in tree $1: the first of etc/systemd/system, usr/lib/systemd/system, usr/lib/systemd/user. Prints
# MASKED when that entry is a mask (a link to /dev/null), and nothing when the unit is not there.
unit_file() {
  for dir in etc/systemd/system usr/lib/systemd/system usr/lib/systemd/user; do
    if [ -L "$1/$dir/$2" ] && [ "$(readlink "$1/$dir/$2")" = /dev/null ]; then echo MASKED; return 0; fi
    if [ -f "$1/$dir/$2" ]; then echo "$1/$dir/$2"; return 0; fi
  done
  return 0
}

trim() {
  T=$1
  [ -d "$T" ] || die "no target directory: $T"

  # 1. Every listed path must be there, before anything is removed. All missing ones are named.
  missing=""
  for p in $DBUS_REQUIRED; do
    # The wants links may be gone already: preset-all runs before this script and removes the link of a unit it disables.
    case $p in *.wants/*) continue ;; esac
    if [ ! -e "$T/$p" ] && [ ! -L "$T/$p" ]; then missing="$missing $p"; fi
  done
  [ -z "$missing" ] || die "listed dbus path(s) missing from the tree:$missing (the fixed list is out of date with Buildroot's dbus package, or the trim would be a silent no-op)"

  # 2. The wait-online mask (post-build.sh) must be there: preset-all ran before this script, and the mask is what it honours.
  [ "$(readlink "$T/$WAIT_ONLINE" 2>/dev/null || true)" = /dev/null ] \
    || die "the wait-online mask ($WAIT_ONLINE) is not in the tree: post-build.sh did not run, or preset-all ran without it"

  # 3. Remove them: the helper too (makedevs has applied the permission table). ${T:?} and ${p:?} stop the loop if either
  #    is empty, so no removal can resolve to the root.
  for p in $DBUS_REQUIRED $DBUS_OPTIONAL; do
    rm -rf "${T:?}/${p:?}"
  done

  # 4. Nothing with dbus in its path may be left. Host find, run in the tree.
  left=$(cd "$T" && find . -path '*dbus*' | sed 's|^\./||' || true)
  [ -z "$left" ] || die "dbus paths left after the removal (add them to the list or find their owner): $(echo $left)"

  # 5. No ELF file left under the tree may need libdbus-1. readelf is the host's GNU binutils: it reads arm64 ELF without
  #    the cross toolchain. Buildroot's TARGET_READELF is a make variable, not exported to this script.
  readelf_bin=${TARGET_READELF:-readelf}
  command -v "$readelf_bin" >/dev/null 2>&1 || die "no $readelf_bin on the build host (binutils); set TARGET_READELF"
  needing=$(find "$T" -type f | while IFS= read -r f; do
    if is_elf "$f" && "$readelf_bin" -d "$f" 2>/dev/null | grep -q 'Shared library: \[libdbus-1\.so'; then
      echo "${f#"$T"/}"
    fi
  done)
  [ -z "$needing" ] || die "ELF file(s) still need libdbus-1 after the removal: $(echo $needing)"

  # 6. The enabled units, as preset-all left them: the links in the wants/requires directories. Each must resolve; a
  #    dangling link is a unit this script removed that a preset had enabled.
  links=$(find "$T/etc/systemd/system" "$T/usr/lib/systemd/system" -mindepth 2 \( -path '*.wants/*' -o -path '*.requires/*' \) 2>/dev/null || true)
  dangling=""
  for l in $links; do
    [ -e "$l" ] || dangling="$dangling ${l#"$T"/}"
  done
  [ -z "$dangling" ] || die "dangling wants link(s) after the removal (a unit this script removed was enabled by a preset): $dangling"

  enabled=$(for l in $links; do echo "${l##*/}"; done | sort -u)
  report=$(for u in $enabled; do
    f=$(unit_file "$T" "$u")
    [ "$f" != MASKED ] || continue
    [ -n "$f" ] || continue
    case $u in
      *.service)
        if grep -q '^Type=dbus' "$f"; then echo "TYPE $u"; fi ;;
      *.socket)
        explicit=$(sed -n 's/^Service=//p' "$f" | tail -1)
        svc=${explicit:-${u%.socket}.service}
        case " $OFF_SERVICES " in
          *" $svc "*) echo "OFF $u $svc"; continue ;;
        esac
        sf=$(unit_file "$T" "$svc")
        if [ -z "$sf" ] || [ "$sf" = MASKED ]; then
          if [ -n "$explicit" ]; then echo "DANGLING $u $svc"; fi
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

  echo "raze-base post-fakeroot: dbus removed from $T (required $(echo $DBUS_REQUIRED | wc -w) paths, optional $(echo $DBUS_OPTIONAL | wc -w)); no dbus path, no libdbus ELF, no dangling wants link; checks passed"
}

self_test() {
  scratch=$1 root=$2
  self=$(cd "$(dirname "$0")" && pwd)/$(basename "$0")
  [ -d "$root" ] || die "--self-test: no root directory: $root"
  mkdir -p "$scratch"
  copy() { rm -rf "${scratch:?}/${1:?}"; cp -a "$root" "$scratch/$1"; }
  run() { sh "$self" "$scratch/$1" >"$scratch/$1.log" 2>&1; }
  # The tree the build makes with HOSTNAMED off, and with the mask post-build.sh places.
  built() {
    copy "$1"
    d=$scratch/$1
    rm -f "$d/usr/lib/systemd/system/systemd-hostnamed.service" "$d/usr/lib/systemd/system/systemd-hostnamed.socket" \
      "$d/usr/lib/systemd/system/sockets.target.wants/systemd-hostnamed.socket" "$d/usr/lib/systemd/systemd-hostnamed" \
      "$d/usr/bin/hostnamectl"
    mkdir -p "$d/etc/systemd/system"
    ln -sfn /dev/null "$d/etc/systemd/system/systemd-networkd-wait-online.service"
  }
  refused_naming() {  # refused_naming CASE TEXT: the case must fail, and its log must name TEXT
    run "$1" && die "self-test: $1 was not refused"
    grep -q -- "$2" "$scratch/$1.log" || die "self-test: $1 refused, but the refusal does not name $2: $(tail -1 "$scratch/$1.log")"
    echo "self-test: fail: $1 is refused, naming $2: $(grep -m1 -- "$2" "$scratch/$1.log")"
  }

  copy asis
  mkdir -p "$scratch/asis/etc/systemd/system"
  ln -sfn /dev/null "$scratch/asis/etc/systemd/system/systemd-networkd-wait-online.service"
  refused_naming asis systemd-hostnamed.socket

  built pass
  run pass || { cat "$scratch/pass.log" >&2; die "self-test: the tree as the HOSTNAMED-off build makes it did not pass"; }
  echo "self-test: pass: the HOSTNAMED-off tree passes: $(tail -1 "$scratch/pass.log")"

  built nomask
  rm -f "$scratch/nomask/etc/systemd/system/systemd-networkd-wait-online.service"
  refused_naming nomask systemd-networkd-wait-online.service

  built missing
  rm -f "$scratch/missing/usr/bin/dbus-send"
  refused_naming missing usr/bin/dbus-send

  built nohelper
  rm -f "$scratch/nohelper/usr/libexec/dbus-daemon-launch-helper"
  refused_naming nohelper usr/libexec/dbus-daemon-launch-helper

  built linked
  # The ELF that needs libdbus-1: the real dbus-send on a built root; on the fake tree, a test binary linked here from the
  # host's libdbus-1.so.3 (same soname), into the copy only.
  if is_elf "$scratch/linked/usr/bin/dbus-send"; then
    cp "$scratch/linked/usr/bin/dbus-send" "$scratch/linked/usr/bin/keep-send"
  else
    command -v gcc >/dev/null 2>&1 || die "self-test: no gcc on the host: the linked case needs an ELF that needs libdbus-1"
    lib=$(ldconfig -p 2>/dev/null | sed -n 's/^[[:space:]]*libdbus-1\.so\.3 .*=> //p' | head -1)
    [ -n "$lib" ] || die "self-test: no host libdbus-1.so.3 to link the linked case against"
    echo 'int main(void){return 0;}' | gcc -Wl,--no-as-needed -o "$scratch/linked/usr/bin/keep-send" -x c - -x none "$lib" \
      || die "self-test: gcc could not link the linked case"
  fi
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

  # A preset enabled a dbus unit that is NOT on the fixed list: the removal leaves its link dangling. Refused.
  built danglink
  printf '[Unit]\nDescription=fake dbus unit not on the list\n[Service]\nExecStart=/bin/false\n' \
    >"$scratch/danglink/usr/lib/systemd/system/dbus-fake-extra.service"
  ln -s ../dbus-fake-extra.service "$scratch/danglink/usr/lib/systemd/system/multi-user.target.wants/dbus-fake-extra.service"
  refused_naming danglink dbus-fake-extra.service

  # A wants link whose target a preset-enabled unit left missing, with no dbus in its name: the dangling check catches it.
  built gonelink
  ln -s ../gone-unit.service "$scratch/gonelink/usr/lib/systemd/system/multi-user.target.wants/gone-unit.service"
  refused_naming gonelink gone-unit.service

  echo "self-test: passed (the raw root is refused on hostnamed.socket; the HOSTNAMED-off tree passes; a missing mask, a deleted listed path, the helper absent, a libdbus-1 ELF, an enabled Type=dbus unit, a dangling socket and a dangling wants link to a dbus unit are each refused)"
}

case ${1:-} in
  --self-test)
    [ $# -eq 3 ] || die "usage: post-fakeroot.sh --self-test SCRATCH ROOT"
    self_test "$2" "$3"
    ;;
  -h|--help)
    sed -n '2,20p' "$0"
    ;;
  "")
    die "usage: post-fakeroot.sh TARGET_DIR"
    ;;
  *)
    [ $# -eq 1 ] || die "usage: post-fakeroot.sh TARGET_DIR"
    trim "$1"
    ;;
esac
