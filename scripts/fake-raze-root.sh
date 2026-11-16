#!/usr/bin/env bash
# A FAKE stand-in for an extracted raze-base root, for dry runs of os/post-build.sh and the device checks. It is NOT a
# built root: every file is a placeholder, and only the paths the post-build script names exist, plus the hostnamed
# units its self-test needs. The paths come from os/post-build.sh itself (DBUS_REQUIRED, DBUS_OPTIONAL), so the
# fixture cannot drift from the list it is meant to exercise.
#
#   scripts/fake-raze-root.sh DIR    makes DIR (must not exist) and fills it
set -euo pipefail
die() { echo "fake-raze-root: $*" >&2; exit 1; }
[ $# -eq 1 ] || die "usage: fake-raze-root.sh DIR"
dir=$1
[ ! -e "$dir" ] || die "$dir exists; give a new directory"
repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
pb=$repo/devices/products/raze-base/os/post-build.sh
block() { sed -n "/^$1='/,/^'/p" "$pb" | sed '1s/.*='"'"'//; $d' | sed '/^$/d'; }
mkdir -p "$dir"
echo "FAKE raze-base root (scripts/fake-raze-root.sh): placeholders, not a built root" > "$dir/FAKE-ROOT.txt"
for p in $(block DBUS_REQUIRED) $(block DBUS_OPTIONAL); do
  case $p in
    etc/dbus-1|usr/share/dbus-1|usr/share/xml/dbus-1|var/lib/dbus|run/dbus) mkdir -p "$dir/$p"; touch "$dir/$p/fake" ;;
    */multi-user.target.wants/*|*/sockets.target.wants/*) mkdir -p "$(dirname "$dir/$p")"
         ln -sfn "../$(basename "$p")" "$dir/$p" ;;
    *) mkdir -p "$(dirname "$dir/$p")"; echo "FAKE placeholder" > "$dir/$p" ;;
  esac
done
# The helper is not in DBUS_REQUIRED (post-build leaves it for post-fakeroot); the fixture still has it, as a built root would.
mkdir -p "$dir/usr/libexec" && echo "FAKE placeholder" > "$dir/usr/libexec/dbus-daemon-launch-helper"
# The hostnamed units the self-test's raw-root case needs (enabled socket, starts a daemon the image turns off).
u=$dir/usr/lib/systemd/system
mkdir -p "$u/sockets.target.wants" "$dir/usr/lib/systemd" "$dir/usr/bin"
printf '[Service]\nExecStart=/bin/false\n' > "$u/systemd-hostnamed.service"
printf '[Socket]\nListenStream=/run/fake-hostnamed\nService=systemd-hostnamed.service\n' > "$u/systemd-hostnamed.socket"
ln -sfn ../systemd-hostnamed.socket "$u/sockets.target.wants/systemd-hostnamed.socket"
echo "FAKE placeholder" > "$dir/usr/lib/systemd/systemd-hostnamed"
echo "FAKE placeholder" > "$dir/usr/bin/hostnamectl"
echo "fake-raze-root: made $dir (fake; every file is a placeholder)"
