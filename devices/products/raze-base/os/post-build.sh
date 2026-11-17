#!/bin/sh
# The Raze base OS's post-build script (os/raze.fragment, BR2_ROOTFS_POST_BUILD_SCRIPT). Buildroot runs it as
# `post-build.sh TARGET_DIR` in target-finalize (Makefile:827-832), on output/target, BEFORE the rootfs step.
#
# WHAT IT DOES: places the mask of systemd-networkd-wait-online.service, and nothing else. The mask must exist before
# `systemctl preset-all` runs (fs/common.mk:181 ROOTFS_PRE_CMD_HOOKS, systemd.mk:834): preset-all honours a mask, and
# systemd-networkd.service's [Install] has Also=systemd-networkd-wait-online.service (systemd-258.7
# units/systemd-networkd.service.in:70), so the mask is what keeps the wait out of network-online.target. Whether
# a preset `disable *` alone would also do it: NEEDS-THE-BUILD (the Also= link path in install.c was not read).
#
# WHAT IT DOES NOT DO: it does not touch D-Bus. The helper, the permission table and the dbus paths are removed by
# os/post-fakeroot.sh, which runs after makedevs and preset-all (fs/common.mk:179-185). Removing dbus here would
# break makedevs (the helper is named in DBUS_PERMISSIONS, dbus.mk:32-34) and would leave the presets' view different.
#
#   post-build.sh TARGET_DIR
#   post-build.sh --self-test SCRATCH ROOT   the proof, on copies of a fake or built root: the mask is placed and the
#                                             dbus paths are still there (this script must not remove them)
set -eu

die() { echo "raze-base post-build: $*" >&2; exit 1; }

WAIT_ONLINE=etc/systemd/system/systemd-networkd-wait-online.service

place_mask() {
  T=$1
  [ -d "$T" ] || die "no target directory: $T"
  mkdir -p "$T/etc/systemd/system"
  ln -sfn /dev/null "$T/$WAIT_ONLINE"
  [ "$(readlink "$T/$WAIT_ONLINE")" = /dev/null ] || die "the mask did not take: $WAIT_ONLINE"
  echo "raze-base post-build: wait-online masked in $T (dbus is left for os/post-fakeroot.sh)"
}

self_test() {
  scratch=$1 root=$2
  self=$(cd "$(dirname "$0")" && pwd)/$(basename "$0")
  [ -d "$root" ] || die "--self-test: no root directory: $root"
  mkdir -p "$scratch"
  rm -rf "${scratch:?}/pass"
  cp -a "$root" "$scratch/pass"
  sh "$self" "$scratch/pass" >"$scratch/pass.log" 2>&1 || { cat "$scratch/pass.log" >&2; die "self-test: post-build did not run on the tree"; }
  [ "$(readlink "$scratch/pass/$WAIT_ONLINE")" = /dev/null ] || die "self-test: no wait-online mask after post-build"
  [ -e "$scratch/pass/usr/libexec/dbus-daemon-launch-helper" ] || die "self-test: post-build removed the helper (it must be left for post-fakeroot)"
  [ -e "$scratch/pass/usr/lib/systemd/system/dbus.service" ] || die "self-test: post-build removed a dbus unit (it must be left for post-fakeroot)"
  echo "self-test: passed (the mask is placed; the dbus paths are left for post-fakeroot)"
}

case ${1:-} in
  --self-test)
    [ $# -eq 3 ] || die "usage: post-build.sh --self-test SCRATCH ROOT"
    self_test "$2" "$3"
    ;;
  -h|--help)
    sed -n '2,20p' "$0"
    ;;
  "")
    die "usage: post-build.sh TARGET_DIR"
    ;;
  *)
    [ $# -eq 1 ] || die "usage: post-build.sh TARGET_DIR"
    place_mask "$1"
    ;;
esac
