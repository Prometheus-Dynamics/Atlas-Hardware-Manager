#!/bin/sh
# Off-device test: mDNS on a Raze is systemd-resolved's. resolved's mDNS is
# on, avahi isn't in the image or enabled, and the identity's _pd-device._tcp
# is a .dnssd service with the contract's TXT. avahi.toml (for an OS whose
# programs use libavahi-client) brings avahi back, enables it and turns
# resolved's and NetworkManager's mDNS off; identity writes avahi's service
# file too, for its bind mount.
# Run: sh devices/raze/tests/mdns.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
gaia=$here/../gaia
rootfs=$gaia/assets/rootfs
lib=$rootfs/usr/lib/board
T=$(mktemp -d "${TMPDIR:-/tmp}/board-mdns-test.XXXXXX")
trap 'rm -rf "$T"' EXIT

fail() {
	echo "FAIL: $*" >&2
	exit 1
}

echo "resolved answers mDNS; no avahi in the image by default"
grep -qx 'MulticastDNS=yes' "$rootfs/usr/lib/systemd/resolved.conf.d/60-board-mdns.conf" || fail "resolved's mDNS is off"
grep -qx 'connection.mdns=2' "$rootfs/usr/lib/NetworkManager/conf.d/60-board.conf" || fail "NetworkManager doesn't ask for mDNS"
! grep -q 'BR2_PACKAGE_AVAHI' "$gaia/runtime.toml" || fail "runtime.toml builds avahi"
! grep -q '"./avahi.toml"' "$gaia/device.toml" || fail "device.toml imports avahi.toml"
! grep -q avahi "$rootfs/usr/lib/systemd/system-preset/70-board.preset" || fail "70-board.preset enables avahi"
grep -q 'MulticastDNS=yes' "$lib/usb-gadget-dhcp" || fail "networkd's usbbr0 has mDNS off"

echo "avahi.toml: avahi built, enabled and staged; resolved and NetworkManager off 5353"
grep -q '\["BR2_PACKAGE_AVAHI", "y"\]' "$gaia/avahi.toml" || fail "BR2_PACKAGE_AVAHI"
grep -q '\["BR2_PACKAGE_AVAHI_DAEMON", "y"\]' "$gaia/avahi.toml" || fail "BR2_PACKAGE_AVAHI_DAEMON"
grep -qx 'enable avahi-daemon.service' "$rootfs/usr/lib/systemd/system-preset/70-board-avahi.preset" || fail "avahi-daemon isn't enabled"
for id in raze-etc-avahi-daemon-conf raze-systemd-system-etc-avahi-services-mount \
	raze-systemd-system-avahi-daemon-service-d-50-board raze-systemd-resolved-conf-d-61-board-avahi-conf \
	raze-NetworkManager-conf-d-61-board-avahi-conf raze-systemd-system-preset-70-board-avahi-preset; do
	grep -q "id = \"$id\"" "$gaia/avahi.toml" || fail "$id isn't staged"
	grep -q "^  \"$id\",\$" "$gaia/avahi.toml" || fail "$id isn't fed"
	src=$(sed -n "/id = \"$id\"/{n;s/^src = \"\${self}\/\(.*\)\"$/\1/p}" "$gaia/avahi.toml")
	[ -f "$gaia/$src" ] || fail "$id: no $src"
done
grep -qx 'MulticastDNS=no' "$rootfs/usr/lib/systemd/resolved.conf.d/61-board-avahi.conf" || fail "resolved's mDNS stays on"
grep -qx 'connection.mdns=0' "$rootfs/usr/lib/NetworkManager/conf.d/61-board-avahi.conf" || fail "NetworkManager asks for mDNS"
grep -q '^deny-interfaces=usb0,usb1$' "$rootfs/etc/avahi/avahi-daemon.conf" || fail "avahi answers on the bridge's members too"

echo "the bind mount takes the directory identity writes"
mount=$rootfs/usr/lib/systemd/system/etc-avahi-services.mount
grep -qx 'What=/run/board/avahi-services' "$mount" || fail "What="
grep -qx 'Where=/etc/avahi/services' "$mount" || fail "Where="
grep -q 'BOARD_AVAHI_DIR:-/run/board/avahi-services' "$lib/identity" || fail "identity writes elsewhere"
grep -q 'BOARD_DNSSD_DIR:-/run/systemd/dnssd' "$lib/identity" || fail "identity's .dnssd goes elsewhere"

echo "identity --dnssd writes a .dnssd and an avahi service with the contract's TXT"
mkdir -p "$T/dt" "$T/etc" "$T/run" "$T/data" "$T/bin"
# systemctl: resolved is running while $T/resolved exists; calls logged.
cat > "$T/bin/systemctl" <<EOF
#!/bin/sh
echo "systemctl \$*" >> "$T/systemctl.log"
case "\$*" in
*is-active*) [ -e "$T/resolved" ] ;;
esac
EOF
chmod +x "$T/bin/systemctl"
export PATH="$T/bin:$PATH" BOARD_DNSSD_DIR=$T/dnssd
printf '10000000a317bcbe\000' > "$T/dt/serial-number"
printf 'ID=helios\nVERSION_ID=2026.3.0\n' > "$T/os-release"
export BOARD_LIB_DIR=$lib BOARD_DT_DIR=$T/dt BOARD_ETC_DIR=$T/etc BOARD_RUN_DIR=$T/run BOARD_DATA_DIR=$T/data
export BOARD_OS_RELEASE=$T/os-release BOARD_AVAHI_DIR=$T/avahi BOARD_MACHINE_ID=$T/machine-id
export BOARD_SLOT_JSON=$T/no-slot.json
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
dnssd=$T/dnssd/pd-device.dnssd
python3 - "$dnssd" <<'PY'
import configparser, sys
c = configparser.ConfigParser(interpolation=None)
c.optionxform = str
c.read(sys.argv[1])
s = c["Service"]
assert s["Name"] == "%H" and s["Type"] == "_pd-device._tcp" and s["Port"] == "5899", dict(s)
txt = dict(kv.split("=", 1) for kv in s["TxtText"].split())
for key in ("contract", "model", "rev", "serial", "os", "os_ver", "path"):
    assert key in txt, key
assert txt["contract"] == "1" and txt["serial"] == "10000000a317bcbe", txt
assert txt["os"] == "helios" and txt["os_ver"] == "2026.3.0" and txt["path"] == "/.well-known/pd-device", txt
PY
! grep -q reload "$T/systemctl.log" 2>/dev/null || fail "resolved isn't running, nothing to reload"
echo "a change while resolved runs reloads it; no change, no reload"
touch "$T/resolved"
printf 'ID=helios\nVERSION_ID=2026.3.1\n' > "$T/os-release"
: > "$T/systemctl.log"
sh "$lib/identity" --dnssd --quiet
grep -qx 'systemctl reload systemd-resolved.service' "$T/systemctl.log" || fail "no reload: $(cat "$T/systemctl.log")"
: > "$T/systemctl.log"
sh "$lib/identity" --dnssd --quiet
! grep -q reload "$T/systemctl.log" || fail "reloaded for nothing"
rm "$T/resolved"
echo "on a slots board the version is the running slot's"
printf 'ID=helios\nVERSION_ID="v2026.2.0"\nIMAGE_VERSION="v2026.2.0"\n' > "$T/os-release"
printf '{"schema": 1, "id": "helios-2026.2.0-dev.727-styx-756b5cda", "family": "helios",\n "version": "2026.2.0-dev.727", "variant": "styx"}\n' > "$T/slot.json"
BOARD_SLOT_JSON=$T/slot.json sh "$lib/identity" --dnssd --quiet
grep -q '<txt-record>os_ver=2026.2.0-dev.727</txt-record>' "$T/avahi/pd-device.service" || fail "os_ver: $(grep os_ver "$T/avahi/pd-device.service")"
. "$lib/lib.sh"
[ "$(BOARD_SLOT_JSON=$T/slot.json board_os_version)" = 2026.2.0-dev.727 ] || fail "board_os_version"
[ "$(board_os_version)" = v2026.2.0 ] || fail "without a slot: $(board_os_version)"

echo "BOARD_MDNS=0 takes it away (the directory stays, for the mount)"
printf 'BOARD_MDNS=0\n' > "$T/etc/identity.env"
sh "$lib/identity" --dnssd --quiet
[ -d "$T/avahi" ] && [ ! -e "$T/avahi/pd-device.service" ] || fail "BOARD_MDNS=0"
[ ! -e "$dnssd" ] || fail "BOARD_MDNS=0 leaves the .dnssd"

echo "mdns ok"
