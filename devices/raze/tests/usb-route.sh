#!/bin/sh
# Off-device test of usb-route: the USB link onto the board's other networks
# (NAT, forwarding, DHCP routes), with ip, nft and systemctl faked.
# Run: sh devices/raze/tests/usb-route.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
lib=$here/../gaia/assets/rootfs/usr/lib/board
T=$(mktemp -d "${TMPDIR:-/tmp}/board-usb-route-test.XXXXXX")
trap 'rm -rf "$T"' EXIT

fail() {
	echo "FAIL: $*" >&2
	exit 1
}

mkdir -p "$T/dt" "$T/etc" "$T/data" "$T/run" "$T/bin"
export BOARD_LIB_DIR=$lib BOARD_DT_DIR=$T/dt BOARD_ETC_DIR=$T/etc BOARD_DATA_DIR=$T/data BOARD_RUN_DIR=$T/run
export BOARD_MACHINE_ID=$T/machine-id BOARD_IP_FORWARD=$T/ip_forward BOARD_NFT=$T/bin/nft
echo 0 > "$T/ip_forward"

# ip: the bridge exists while $T/bridge does; addresses are $T/addrs.
cat > "$T/bin/ip" <<EOF
#!/bin/sh
case "\$*" in
"link show dev usbbr0") [ -e "$T/bridge" ] ;;
"-4 -o addr show scope global") cat "$T/addrs" 2>/dev/null ;;
*) exit 0 ;;
esac
EOF
# nft: a rules file loaded with -f, listed, deleted; every call logged.
cat > "$T/bin/nft" <<EOF
#!/bin/sh
echo "nft \$*" >> "$T/nft.log"
case "\$1" in
-f) cat > "$T/nft.rules" ;;
list) [ -e "$T/nft.rules" ] ;;
delete) rm -f "$T/nft.rules" ;;
esac
EOF
cat > "$T/bin/systemctl" <<EOF
#!/bin/sh
echo "systemctl \$*" >> "$T/systemctl.log"
EOF
chmod +x "$T/bin/ip" "$T/bin/nft" "$T/bin/systemctl"
export PATH="$T/bin:$PATH"

route() { sh "$lib/usb-route" "$@"; }
is() {
	# is <expected> <python expression over d (the status JSON)> <what>
	got=$(route status | python3 -c "import json,sys; d=json.load(sys.stdin); print($2)")
	[ "$got" = "$1" ] || fail "$3: got '$got', want '$1'"
}

. "$lib/lib.sh"
board_load_env usb-gadget.env
board_gadget_address
via=${BOARD_GADGET_CIDR%/*}

touch "$T/bridge"
cat > "$T/addrs" <<'EOF'
2: eth0    inet 10.53.38.11/24 brd 10.53.38.255 scope global eth0\       valid_lft forever preferred_lft forever
5: usbbr0    inet 172.31.209.217/29 brd 172.31.209.223 scope global usbbr0\       valid_lft forever preferred_lft forever
6: usb0    inet 172.31.0.9/29 scope global usb0\       valid_lft forever preferred_lft forever
EOF

echo "on by default: NAT out of the other interfaces, forwarding on, a route to eth0's network"
route apply
grep -q 'iifname "usbbr0" oifname != "usbbr0" masquerade' "$T/nft.rules" || fail "no masquerade: $(cat "$T/nft.rules")"
grep -q 'oifname "usbbr0" drop' "$T/nft.rules" || fail "the robot network must not reach in"
[ "$(cat "$T/ip_forward")" = 1 ] || fail "forwarding should be on"
grep -qx "option:classless-static-route,10.53.38.0/24,$via" "$T/run/usb-gadget-dhcp-routes" ||
	fail "routes: $(cat "$T/run/usb-gadget-dhcp-routes")"
grep -qx "249,10.53.38.0/24,$via" "$T/run/usb-gadget-dhcp-routes" || fail "no option 249 for Windows"
grep -q 'kill -s HUP board-usb-gadget-dhcp.service' "$T/systemctl.log" || fail "dnsmasq should reread its routes"
is "True True ['10.53.38.0/24']" 'd["enabled"], d["active"], d["subnets"]' "status"
is "['10.53.38.11/24']" 'd["addresses"]' "the board's own address on it"

echo "applying again with nothing changed rewrites nothing"
before=$(grep -c -- '-f' "$T/nft.log")
: > "$T/systemctl.log"
route apply
[ "$(grep -c -- '-f' "$T/nft.log")" = "$before" ] || fail "the NAT table was rewritten"
[ ! -s "$T/systemctl.log" ] || fail "dnsmasq was told for nothing"

echo "a second network: routes to both; link-local, loopback and the USB link never"
cat >> "$T/addrs" <<'EOF'
3: wlan0    inet 192.168.4.7/22 brd 192.168.7.255 scope global wlan0\       valid_lft forever preferred_lft forever
4: eth0    inet 169.254.3.4/16 brd 169.254.255.255 scope global eth0\       valid_lft forever preferred_lft forever
EOF
route apply
grep -qx "option:classless-static-route,10.53.38.0/24,$via,192.168.4.0/22,$via" "$T/run/usb-gadget-dhcp-routes" ||
	fail "routes: $(cat "$T/run/usb-gadget-dhcp-routes")"
is "['10.53.38.0/24', '192.168.4.0/22']" 'd["subnets"]' "subnets"
is "['10.53.38.11/24', '192.168.4.7/22']" 'd["addresses"]' "addresses"

echo "a static address on a wide network: the route is wide, the address names the robot network"
cp "$T/addrs" "$T/addrs.keep"
cat > "$T/addrs" <<'EOF'
2: eth0    inet 10.63.90.12/8 brd 10.255.255.255 scope global eth0\       valid_lft forever preferred_lft forever
EOF
route apply
is "['10.0.0.0/8'] ['10.63.90.12/8']" 'd["subnets"], d["addresses"]' "wide route"
mv "$T/addrs.keep" "$T/addrs"
route apply

echo "set off: no NAT, forwarding back off, no routes, and the choice is kept"
route set off
[ ! -e "$T/nft.rules" ] || fail "the NAT table should be gone"
[ "$(cat "$T/ip_forward")" = 0 ] || fail "forwarding should be back off"
[ ! -s "$T/run/usb-gadget-dhcp-routes" ] || fail "routes left: $(cat "$T/run/usb-gadget-dhcp-routes")"
[ "$(cat "$T/data/usb-route")" = off ] || fail "the choice isn't kept"
is "False False" 'd["enabled"], d["active"]' "status off"
route apply
[ ! -e "$T/nft.rules" ] || fail "apply must keep the choice"

echo "forwarding something else turned on stays on"
route set on
echo 1 > "$T/ip_forward"
rm -f "$T/run/usb-route.state.forward"
route set off
[ "$(cat "$T/ip_forward")" = 1 ] || fail "forwarding this didn't turn on was turned off"
echo 0 > "$T/ip_forward"

echo "an OS default of off, until chosen on"
rm -f "$T/data/usb-route"
echo 'USB_GADGET_ROUTE=0' > "$T/etc/usb-gadget.env"
route apply
[ ! -e "$T/nft.rules" ] || fail "off by the OS default"
route set on
[ -e "$T/nft.rules" ] || fail "chosen on"
rm -f "$T/etc/usb-gadget.env"

echo "no USB link: nothing to route, not active"
rm -f "$T/bridge"
route apply
[ ! -s "$T/run/usb-gadget-dhcp-routes" ] || fail "routes without a USB link"
is "True False" 'd["enabled"], d["active"]' "no bridge"

echo "the identity carries it"
json=$(BOARD_NET_DIR=$T/net BOARD_OS_RELEASE=/dev/null sh "$lib/identity")
printf '%s' "$json" | python3 -c 'import json,sys; d=json.load(sys.stdin); assert d["usb_route"] == {"enabled": True, "active": False, "subnets": [], "addresses": []}, d.get("usb_route")' ||
	fail "identity: $json"

echo "ok"
