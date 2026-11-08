#!/bin/sh
# Off-device test of /etc/init.d/S00nand-slot: trial-check, then the watchdog
# in the background with the deadline; a stale trial is said; nothing a stub
# does can stop the boot. Run: sh devices/families/rockchip-maskrom/tests/nand-slot-hook.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
hook=$here/../gaia/assets/rootfs/etc/init.d/S00nand-slot
T=$(mktemp -d "${TMPDIR:-/tmp}/nand-slot-hook.XXXXXX")
trap 'rm -rf "$T"' EXIT

fail() {
	echo "FAIL: $*" >&2
	exit 1
}

cat > "$T/atlas-ubi" <<STUB
#!/bin/sh
echo "\$*" >> "$T/calls"
case "\$2" in
trial-check) printf '%s\n' "\${CHECK_OUTPUT:-}"; exit "\${CHECK_RC:-0}" ;;
esac
STUB
chmod +x "$T/atlas-ubi"
export ATLAS_UBI=$T/atlas-ubi KMSG=$T/kmsg

run() {
	: > "$T/calls"
	: > "$T/kmsg"
	sh "$hook" "$@" || fail "the hook failed the boot"
	# The watchdog runs in the background.
	tries=0
	until grep -q watchdog "$T/calls" || [ $tries -ge 50 ]; do
		sleep 0.1
		tries=$((tries + 1))
	done
}

echo "start: the check, then the watchdog with the deadline"
CHECK_OUTPUT='{"trial":false,"erased_stale_trial":false}' run start
[ "$(sed -n 1p "$T/calls")" = "nand-slot trial-check" ] || fail "calls: $(cat "$T/calls")"
grep -q 'nand-slot watchdog --deadline 600' "$T/calls" || fail "calls: $(cat "$T/calls")"
[ ! -s "$T/kmsg" ] || fail "said something on a clean boot: $(cat "$T/kmsg")"

echo "a stale trial copy is said; a failing check says why and goes on"
CHECK_OUTPUT='{"trial":true,"erased_stale_trial":true}' run start
grep -q "didn't erase" "$T/kmsg" || fail "kmsg: $(cat "$T/kmsg")"
CHECK_OUTPUT='{"error":"boom"}' CHECK_RC=1 run
grep -q 'trial-check: {"error":"boom"}' "$T/kmsg" || fail "kmsg: $(cat "$T/kmsg")"
grep -q watchdog "$T/calls" || fail "no watchdog after a failed check"

echo "stop does nothing"
: > "$T/calls"
sh "$hook" stop || fail "stop failed"
[ ! -s "$T/calls" ] || fail "stop ran: $(cat "$T/calls")"
echo ok
