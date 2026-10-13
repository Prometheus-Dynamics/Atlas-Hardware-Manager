#!/bin/sh
# Off-device test: mDNS on a Raze is avahi's. The image builds avahi-daemon
# (and libavahi-client, with D-Bus), enables it, and turns systemd-resolved's
# and NetworkManager's mDNS off; the identity's _pd-device._tcp is an avahi
# service file bound into /etc/avahi/services, with the contract's TXT.
# Run: sh devices/raze/tests/avahi.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
gaia=$here/../gaia
rootfs=$gaia/assets/rootfs
lib=$rootfs/usr/lib/board
T=$(mktemp -d "${TMPDIR:-/tmp}/board-avahi-test.XXXXXX")
trap 'rm -rf "$T"' EXIT

fail() {
	echo "FAIL: $*" >&2
	exit 1
}

echo "the image builds avahi-daemon and enables it"
grep -q '\["BR2_PACKAGE_AVAHI", "y"\]' "$gaia/runtime.toml" || fail "BR2_PACKAGE_AVAHI"
grep -q '\["BR2_PACKAGE_AVAHI_DAEMON", "y"\]' "$gaia/runtime.toml" || fail "BR2_PACKAGE_AVAHI_DAEMON"
grep -qx 'enable avahi-daemon.service' "$rootfs/usr/lib/systemd/system-preset/70-board.preset" || fail "avahi-daemon isn't enabled"
for id in raze-etc-avahi-daemon-conf raze-systemd-system-etc-avahi-services-mount raze-systemd-system-avahi-daemon-service-d-50-board; do
	grep -q "id = \"$id\"" "$gaia/runtime.toml" || fail "$id isn't staged"
done

echo "resolved and NetworkManager leave mDNS to avahi"
grep -qx 'MulticastDNS=no' "$rootfs/usr/lib/systemd/resolved.conf.d/60-board-mdns.conf" || fail "resolved's mDNS is on"
! grep -q 'connection.mdns=[12]' "$rootfs/usr/lib/NetworkManager/conf.d/60-board.conf" || fail "NetworkManager asks for mDNS"
! grep -q 'resolvectl mdns' "$lib/usb-gadget-setup" || fail "the gadget turns resolved's mDNS on"
grep -q '^deny-interfaces=usb0,usb1$' "$rootfs/etc/avahi/avahi-daemon.conf" || fail "avahi answers on the bridge's members too"

echo "the bind mount takes the directory identity writes"
mount=$rootfs/usr/lib/systemd/system/etc-avahi-services.mount
grep -qx 'What=/run/board/avahi-services' "$mount" || fail "What="
grep -qx 'Where=/etc/avahi/services' "$mount" || fail "Where="
grep -q 'BOARD_AVAHI_DIR:-/run/board/avahi-services' "$lib/identity" || fail "identity writes elsewhere"

echo "identity --dnssd writes an avahi service with the contract's TXT"
mkdir -p "$T/dt" "$T/etc" "$T/run" "$T/data"
printf '10000000a317bcbe\000' > "$T/dt/serial-number"
printf 'ID=helios\nVERSION_ID=2026.3.0\n' > "$T/os-release"
export BOARD_LIB_DIR=$lib BOARD_DT_DIR=$T/dt BOARD_ETC_DIR=$T/etc BOARD_RUN_DIR=$T/run BOARD_DATA_DIR=$T/data
export BOARD_OS_RELEASE=$T/os-release BOARD_AVAHI_DIR=$T/avahi BOARD_MACHINE_ID=$T/machine-id
sh "$lib/identity" --dnssd --quiet
python3 - "$T/avahi/pd-device.service" <<'PY'
import sys
import xml.etree.ElementTree as ET
root = ET.parse(sys.argv[1]).getroot()
assert root.tag == "service-group", root.tag
name = root.find("name")
assert name is not None and name.text == "%h" and name.get("replace-wildcards") == "yes", "name"
service = root.find("service")
assert service.findtext("type") == "_pd-device._tcp", service.findtext("type")
assert service.findtext("port") == "5899", service.findtext("port")
txt = dict(t.text.split("=", 1) for t in service.findall("txt-record"))
for key in ("contract", "model", "rev", "serial", "os", "os_ver", "path"):
    assert key in txt, key
assert txt["contract"] == "1" and txt["serial"] == "10000000a317bcbe", txt
assert txt["os"] == "helios" and txt["os_ver"] == "2026.3.0" and txt["path"] == "/.well-known/pd-device", txt
PY
echo "BOARD_MDNS=0 takes it away (the directory stays, for the mount)"
printf 'BOARD_MDNS=0\n' > "$T/etc/identity.env"
sh "$lib/identity" --dnssd --quiet
[ -d "$T/avahi" ] && [ ! -e "$T/avahi/pd-device.service" ] || fail "BOARD_MDNS=0"

echo "avahi ok"
