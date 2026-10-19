#!/bin/sh
# Off-device test of usb-gadget-watch: a USB link that keeps sending is left
# alone; one whose member stops sending for three probes in a row gets its
# gadget rebound (once per gap, a few times a boot) with an event; a member
# without carrier isn't probed. Fake sysfs and configfs directories stand in.
# Run: sh devices/raze/tests/usb-gadget-watch.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
lib=$here/../gaia/assets/rootfs/usr/lib/board
T=$(mktemp -d "${TMPDIR:-/tmp}/board-gadget-watch.XXXXXX")
trap 'rm -rf "$T"' EXIT
fail() {
	echo "FAIL: $*" >&2
	exit 1
}

mkdir -p "$T/run" "$T/data" "$T/etc" "$T/net/usbbr0/brif/usb0" "$T/net/usbbr0/brif/usb1" \
	"$T/net/usb0/statistics" "$T/net/usb1/statistics" "$T/cfg/usb_gadget/g1"
echo 1 > "$T/net/usb0/carrier"
echo 0 > "$T/net/usb1/carrier"
echo 84 > "$T/net/usb0/statistics/tx_packets"
echo 5 > "$T/net/usb1/statistics/tx_packets"
echo 1000480000.usb > "$T/cfg/usb_gadget/g1/UDC"
mkdir -p "$T/debug/usb/1000480000.usb"
mkdir -p "$T/proc"
echo " 32:  390045  0  0  0  GICv2  105 Level  1000480000.usb" > "$T/proc/interrupts"
echo "GOTGCTL = 0x000d0000" > "$T/debug/usb/1000480000.usb/regdump"
echo "Endpoint index 1, named ep1in" > "$T/debug/usb/1000480000.usb/ep1in"
# The board's settings file, as an OS or a user would pin the address.
echo 'USB_GADGET_ADDRESS=172.31.12.81/29' > "$T/etc/usb-gadget.env"
export BOARD_LIB_DIR=$lib BOARD_RUN_DIR=$T/run BOARD_DATA_DIR=$T/data BOARD_ETC_DIR=$T/etc \
	BOARD_NET_DIR=$T/net BOARD_CONFIGFS=$T/cfg BOARD_DEBUGFS=$T/debug BOARD_PROC_DIR=$T/proc USB_GADGET_WATCH_IRQ_GAP=0 \
	USB_GADGET_WATCH_SETTLE=0
# The probe: what a healthy link does (a frame leaves usb0), and where it was sent.
healthy="echo \"\$1\" >> $T/probes; echo \$((\$(cat $T/net/usb0/statistics/tx_packets) + 1)) > $T/net/usb0/statistics/tx_packets"
stalled="echo \"\$1\" >> $T/probes"
check() { USB_GADGET_WATCH_PROBE=$1 USB_GADGET_WATCH_NOW=$2 sh "$lib/usb-gadget-watch" --check 2> "$T/err" || fail "check failed: $(cat "$T/err")"; }
rebounds() { cat "$T/data/events.jsonl" 2>/dev/null | grep -c '"kind":"usb-gadget.rebound"' || true; }
# A rebind writes "" then the UDC back: the file ends with the UDC either way,
# so count them by the events.

echo "a link that sends is left alone"
for t in 10 30 50 70; do check "$healthy" $t; done
[ "$(rebounds)" = 0 ] || fail "a healthy link was rebound"
[ "$(sort -u "$T/probes")" = 172.31.12.82 ] || fail "the probe's target: $(sort -u "$T/probes")"
[ "$(wc -l < "$T/probes" | tr -d ' ')" = 4 ] || fail "usb1 has no carrier and shouldn't be probed: $(wc -l < "$T/probes")"

echo "three failed probes in a row rebind the gadget, with an event"
check "$stalled" 90
check "$healthy" 110 # one that sends resets the count
check "$stalled" 130
check "$stalled" 150
[ "$(rebounds)" = 0 ] || fail "rebound after two"
check "$stalled" 170
[ "$(rebounds)" = 1 ] || fail "not rebound after three: $(cat "$T/data/events.jsonl" 2>/dev/null)"
[ "$(cat "$T/cfg/usb_gadget/g1/UDC")" = 1000480000.usb ] || fail "the UDC wasn't written back"
grep -q 'stopped sending (usb0' "$T/data/events.jsonl" || fail "the event says which member"
grep -q '"tx_packets":"[0-9]' "$T/data/events.jsonl" || fail "the event carries the counters"
d=$(ls -d "$T/data/usb-gadget-watch"/0* | head -1)
grep -q "usb0 tx_packets" "$d/counters" || fail "the counters weren't kept: $(cat "$d/counters" 2>/dev/null)"
[ "$(cat "$d/regdump")" = "GOTGCTL = 0x000d0000" ] || fail "the controller's regdump wasn't kept"
[ -f "$d/ep1in" ] || fail "the IN endpoints weren't kept"
grep -q 1000480000.usb "$d/interrupts.0" && [ -f "$d/interrupts.1" ] || fail "the interrupt snapshots weren't kept"

echo "not again within the gap, and at most three times a boot"
for t in 190 210 230 250; do check "$stalled" $t; done
[ "$(rebounds)" = 1 ] || fail "rebound again within 300 s: $(rebounds)"
for t in 500 520 540 900 920 940 1300 1320 1340 1700 1720 1740; do check "$stalled" $t; done
[ "$(rebounds)" = 3 ] || fail "three a boot at most: $(rebounds)"
[ "$(ls "$T/data/usb-gadget-watch" | wc -l | tr -d ' ')" = 3 ] || fail "one evidence dir a rebind: $(ls "$T/data/usb-gadget-watch")"

echo "not bound: nothing to rebind"
rm -f "$T/run/usb-gadget-watch.state" "$T/data/events.jsonl"
: > "$T/cfg/usb_gadget/g1/UDC"
for t in 10 30 50; do check "$stalled" $t; done
[ "$(rebounds)" = 0 ] || fail "rebound an unbound gadget"

echo "ok"
