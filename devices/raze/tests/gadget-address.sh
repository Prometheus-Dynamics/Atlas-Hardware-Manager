#!/bin/sh
# Off-device test of the per-board USB gadget address (scheme serial-hash-v1).
# The vectors are the same as in atlas-devices (src/gadget.rs), so the device
# and Atlas provably compute the same address.
# Run: sh devices/raze/tests/gadget-address.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
lib=$here/../gaia/assets/rootfs/usr/lib/board
T=$(mktemp -d "${TMPDIR:-/tmp}/board-gadget-address-test.XXXXXX")
trap 'rm -rf "$T"' EXIT

fail() {
	echo "FAIL: $*" >&2
	exit 1
}

mkdir -p "$T/dt" "$T/etc" "$T/data" "$T/run"
export BOARD_LIB_DIR=$lib BOARD_DT_DIR=$T/dt BOARD_ETC_DIR=$T/etc BOARD_DATA_DIR=$T/data BOARD_RUN_DIR=$T/run
export BOARD_MACHINE_ID=$T/machine-id
. "$lib/lib.sh"
board_load_env usb-gadget.env

echo "serials map to their subnets"
while read -r serial want; do
	got=$(board_gadget_subnet "$serial") || fail "$serial gave no subnet"
	[ "$got" = "$want" ] || fail "$serial: got $got, want $want"
done <<'EOF'
a317bcbee5226d57 172.31.209.217/29
10000000ABCDEF01 172.31.0.113/29
e5226d57 172.31.209.217/29
00000000 172.31.43.201/29
ffffffff 172.31.141.25/29
deadbeef 172.31.209.1/29
cfc0641d 172.31.254.9/29
EOF

echo "a serial that isn't 8+ hex digits has no subnet"
for serial in '' short abc1234 not-a-hex-serial 'e5226d57 x'; do
	if board_gadget_subnet "$serial" >/dev/null; then fail "'$serial' should have no subnet"; fi
done

echo "the legacy /24 is never handed out"
i=0
while [ "$i" -lt 64 ]; do
	got=$(board_gadget_subnet "$(printf '%08x' $((i * 40503 + 7)))")
	case "$got" in 172.31.250.*) fail "$got is in the legacy subnet" ;; esac
	i=$((i + 1))
done

echo "the DHCP range is the rest of the subnet, at most 19 addresses"
[ "$(board_gadget_dhcp_range 172.31.209.217/29)" = 172.31.209.218,172.31.209.222,255.255.255.248,1h ] ||
	fail "range: $(board_gadget_dhcp_range 172.31.209.217/29)"
[ "$(board_gadget_dhcp_range 172.31.250.1/24)" = 172.31.250.2,172.31.250.20,255.255.255.0,1h ] ||
	fail "legacy range: $(board_gadget_dhcp_range 172.31.250.1/24)"
if board_gadget_dhcp_range 172.31.0.6/29 >/dev/null; then fail "no room after the last host"; fi

echo "the board serial picks the address; a pin or a non-hex serial don't"
printf 'a317bcbee5226d57\000' > "$T/dt/serial-number"
board_gadget_address
[ "$BOARD_GADGET_CIDR $BOARD_GADGET_ADDRESSING" = "172.31.209.217/29 serial-hash-v1" ] ||
	fail "derived: $BOARD_GADGET_CIDR $BOARD_GADGET_ADDRESSING"
USB_GADGET_ADDRESS=172.31.250.1/24 board_gadget_address
[ "$BOARD_GADGET_CIDR $BOARD_GADGET_ADDRESSING" = "172.31.250.1/24 pinned" ] ||
	fail "pinned: $BOARD_GADGET_CIDR $BOARD_GADGET_ADDRESSING"
USB_GADGET_SERIAL=my-board board_gadget_address
[ "$BOARD_GADGET_CIDR $BOARD_GADGET_ADDRESSING" = "172.31.250.1/24 fallback" ] ||
	fail "fallback: $BOARD_GADGET_CIDR $BOARD_GADGET_ADDRESSING"
rm "$T/dt/serial-number"
printf '0123456789abcdef0123456789abcdef\n' > "$T/machine-id"
[ "$(board_gadget_serial)" = 0123456789abcdef ] || fail "machine id serial: $(board_gadget_serial)"
board_gadget_address
[ "$BOARD_GADGET_CIDR" = "$(board_gadget_subnet 0123456789abcdef)" ] || fail "machine id address: $BOARD_GADGET_CIDR"
printf 'a317bcbee5226d57\000' > "$T/dt/serial-number"

echo "the identity reports it"
mkdir -p "$T/net"
json=$(BOARD_NET_DIR=$T/net BOARD_OS_RELEASE=/dev/null sh "$lib/identity")
case "$json" in
*'"serial":"a317bcbee5226d57"'*'"gadget":{"address":"172.31.209.217","prefix":29,"addressing":"serial-hash-v1"}'*) ;;
*) fail "identity: $json" ;;
esac
printf 'USB_GADGET_NET=none\n' > "$T/etc/usb-gadget.env"
json=$(BOARD_NET_DIR=$T/net BOARD_OS_RELEASE=/dev/null sh "$lib/identity")
case "$json" in *'"gadget"'*) fail "no gadget field without the bridge: $json" ;; esac
rm "$T/etc/usb-gadget.env"

echo "usb-gadget-setup sets the USB serial, the address and the DHCP range"
mkdir -p "$T/bin" "$T/configfs/usb_gadget" "$T/udc/fe980000.usb"
for tool in modprobe mountpoint systemctl resolvectl; do
	printf '#!/bin/sh\nexit 0\n' > "$T/bin/$tool"
done
printf '#!/bin/sh\necho "$*" >> "%s"\n' "$T/ip.log" > "$T/bin/ip"
chmod +x "$T/bin/"*
# A plain directory has no os_desc; the settings files win over the environment.
printf 'USB_GADGET_OS_DESC=0\n' > "$T/etc/usb-gadget.env"
PATH="$T/bin:$PATH" BOARD_CONFIGFS=$T/configfs BOARD_UDC_CLASS=$T/udc \
	sh "$lib/usb-gadget-setup" 2>"$T/setup.err" || fail "setup: $(cat "$T/setup.err")"
[ "$(cat "$T/configfs/usb_gadget/g1/strings/0x409/serialnumber")" = a317bcbee5226d57 ] ||
	fail "USB serial: $(cat "$T/configfs/usb_gadget/g1/strings/0x409/serialnumber")"
grep -qx 'addr replace 172.31.209.217/29 dev usbbr0' "$T/ip.log" || fail "ip: $(cat "$T/ip.log")"
grep -qx 'dhcp-range=172.31.209.218,172.31.209.222,255.255.255.248,1h' "$T/run/usb-gadget-dnsmasq.conf" ||
	fail "dnsmasq: $(cat "$T/run/usb-gadget-dnsmasq.conf")"

echo "ok"
