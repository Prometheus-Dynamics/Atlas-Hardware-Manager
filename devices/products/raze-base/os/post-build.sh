#!/bin/sh
# The Raze base OS has no network-online wait. Buildroot's post-build script for raze-base (os/raze.fragment,
# BR2_ROOTFS_POST_BUILD_SCRIPT). Buildroot runs it as `post-build.sh TARGET_DIR` at target-finalize, BEFORE the rootfs
# step's fakeroot script (fs/common.mk: chown, mkusers, makedevs, the pre-command hooks with `systemctl preset-all`,
# then BR2_ROOTFS_POST_FAKEROOT_SCRIPT). The mask must be in place before preset-all, so it is made here.
#
# THIS SCRIPT REMOVES NOTHING (scripts/check-devices.sh refuses a post-build script that does): makedevs runs after
# it and fails on any file a package's permissions table names that is gone (the D1 build, 2026-11-18:
# dbus-daemon-launch-helper). Removals are in os/post-fakeroot.sh, after makedevs and preset-all.
#
# NETWORK-ONLINE: systemd-networkd.service carries Also=systemd-networkd-wait-online.service in its [Install] section,
# so preset-all links the wait-online unit whatever a preset says: a `disable` line in a lower-numbered preset file does
# not stop it (tested with systemd 259 --root preset-all), and a drop-in cannot change [Install] (also tested). A mask is
# honoured by preset-all (the unit is ignored and not linked). Nothing on the image orders on network-online.target
# except that unit's own WantedBy= (checked on the extracted root and devices/components).
set -eu

die() { echo "raze-base post-build: $*" >&2; exit 1; }
[ $# -eq 1 ] || die "usage: post-build.sh TARGET_DIR"
T=$1
[ -d "$T" ] || die "no target directory: $T"
mkdir -p "$T/etc/systemd/system"
ln -sfn /dev/null "$T/etc/systemd/system/systemd-networkd-wait-online.service"
echo "raze-base post-build: wait-online masked in $T"
