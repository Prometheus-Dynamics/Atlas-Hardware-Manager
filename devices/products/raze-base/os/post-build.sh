#!/bin/sh
# The Raze base OS has no D-Bus and no network-online wait. Buildroot's post-build script for raze-base
# (os/raze.fragment, BR2_ROOTFS_POST_BUILD_SCRIPT). Buildroot runs it as `post-build.sh TARGET_DIR` after the packages
# are installed (Makefile, the target-finalize step), BEFORE the rootfs step that runs `systemctl preset-all`
# (fs/common.mk: ROOTFS_PRE_CMD_HOOKS in the fakeroot script). So a file placed here is in place when presets run.
#
#   post-build.sh TARGET_DIR
#       Removes the D-Bus paths listed below, masks systemd-networkd-wait-online.service, then checks the tree.
#       Any failure exits non-zero.
#   post-build.sh --self-test SCRATCH ROOT
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
# NETWORK-ONLINE: systemd-networkd.service carries Also=systemd-networkd-wait-online.service in its [Install] section,
# so preset-all links the wait-online unit whatever a preset says: a `disable` line in a lower-numbered preset file does
# not stop it (tested with systemd 259 --root preset-all), and a drop-in cannot change [Install] (also tested). A mask is
# honoured by preset-all (the unit is ignored and not linked), so it is made here. Nothing on the image orders on
# network-online.target except that unit's own WantedBy= (checked on the extracted root and devices/components).
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
  for p in $DBUS_REQUIRED; do
    if [ ! -e "$T/$p" ] && [ ! -L "$T/$p" ]; then missing="$missing $p"; fi
  done
  [ -z "$missing" ] || die "listed dbus path(s) missing from the target:$missing (the fixed list in os/post-build.sh is out of date with Buildroot, or the trim would be a silent no-op)"

  # 2. Remove them.
  for p in $DBUS_REQUIRED $DBUS_OPTIONAL; do
    rm -rf "$T/$p"
  done

  # 2b. Mask the network-online wait. The mask is the file preset-all honours (see the header); the unit stays in the
  #   tree, so nothing that names it breaks.
  mkdir -p "$T/etc/systemd/system"
  ln -sfn /dev/null "$T/etc/systemd/system/systemd-networkd-wait-online.service"

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
  [ -z "$notes" ] || echo "raze-base post-build: NOTE enabled units:
$notes"

  echo "raze-base post-build: dbus removed from $T (required $(echo $DBUS_REQUIRED | wc -w) paths, optional $(echo $DBUS_OPTIONAL | wc -w)); wait-online masked; checks passed"
}

self_test() {
  scratch=$1 root=$2
  self=$(cd "$(dirname "$0")" && pwd)/$(basename "$0")
  [ -d "$root" ] || die "--self-test: no root directory: $root"
  mkdir -p "$scratch"
  copy() { rm -rf "$scratch/$1"; cp -a "$root" "$scratch/$1"; }
  run() { sh "$self" "$scratch/$1" >"$scratch/$1.log" 2>&1; }
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

  echo "self-test: passed (the raw root is refused on hostnamed.socket; the HOSTNAMED-off tree passes with the mask; a deleted listed path, a libdbus-1 ELF, an enabled Type=dbus unit and a dangling socket are each refused)"
}

case ${1:-} in
  --self-test)
    [ $# -eq 3 ] || die "usage: post-build.sh --self-test SCRATCH ROOT"
    self_test "$2" "$3"
    ;;
  -h|--help)
    sed -n '2,15p' "$0"
    ;;
  "")
    die "usage: post-build.sh TARGET_DIR"
    ;;
  *)
    [ $# -eq 1 ] || die "usage: post-build.sh TARGET_DIR"
    trim "$1"
    ;;
esac
