#!/bin/sh
# The raze-base post-fakeroot script (BR2_ROOTFS_POST_FAKEROOT_SCRIPT, os/raze.fragment). Buildroot runs it as
# `post-fakeroot.sh TARGET_DIR` inside the fakeroot step, AFTER makedevs has applied the package permission tables
# (fs/common.mk:180 makedevs, :183-185 this script). It is the only place the dbus helper is removed:
#   - Buildroot's dbus package names usr/libexec/dbus-daemon-launch-helper in DBUS_PERMISSIONS (dbus.mk:32-34), and
#     makedevs fails when the file is missing (package/makedevs/makedevs.c:578-582), so post-build.sh leaves it.
#   - the helper is setuid (4750 root:dbus) and the image has no D-Bus, so this script removes it here, after makedevs.
#
# THE SHIPPED ROOT: this script is valid only when the rootfs is the fakeroot stage's result (Gaia's fix, pinned in the
# commit that wires this script into os/raze.fragment). Under the current pin the fakeroot-stage copy is discarded
# (fs/common.mk:196), so a removal here would not reach the shipped image.
#
#   post-fakeroot.sh TARGET_DIR   removes the helper, then the whole "nothing with dbus in its path" check
set -eu
set -f

die() { echo "raze-base post-fakeroot: $*" >&2; exit 1; }

HELPER='usr/libexec/dbus-daemon-launch-helper'

[ $# -eq 1 ] || die "usage: post-fakeroot.sh TARGET_DIR"
T=$1
[ -d "$T" ] || die "no target directory: $T"

# 1. The helper must be there to be removed: an absent helper means Buildroot's table changed (shows here, not as a
#   silent no-op).
[ -e "$T/$HELPER" ] || [ -L "$T/$HELPER" ] || die "$HELPER is absent before the removal: Buildroot's DBUS_PERMISSIONS (package/dbus/dbus.mk) may have changed"

# 2. Remove it.
rm -f "$T/$HELPER"

# 3. Nothing with dbus in its path may be left, the helper included. post-build.sh removed the rest of the dbus list.
left=$(cd "$T" && find . -path '*dbus*' | sed 's|^\./||' || true)
[ -z "$left" ] || die "dbus paths left after the removal (add them to the list or find their owner): $(echo $left)"

echo "raze-base post-fakeroot: $HELPER removed from $T; no dbus path left"
