#!/bin/sh
# Off-device test: the failed units are kept on events, hardware.json isn't
# polled. Any service failing starts board-health.service (service.d), which
# itself doesn't (it would start itself again), has no start limit (a burst
# of failures at boot) and only refreshes the failed units: board-agent keeps
# hardware.json from one lemnosd subscription. Its timer is the 60 s
# backstop for units that recovered.
# Run: sh devices/raze/tests/board-health.sh
set -eu
here=$(cd "$(dirname "$0")" && pwd)
units=$here/../gaia/assets/rootfs/usr/lib/systemd/system
fail() { echo "FAIL: $*" >&2; exit 1; }
grep -qx 'OnFailure=board-health.service' "$units/service.d/50-board-failed.conf" || fail "service.d doesn't start board-health.service on a failure"
grep -qx 'OnFailure=' "$units/board-health.service.d/50-no-on-failure.conf" || fail "board-health.service would start itself on its own failure"
sed -n '/^\[Unit\]/,/^\[/p' "$units/board-health.service" | grep -qx 'StartLimitIntervalSec=0' || fail "board-health.service needs StartLimitIntervalSec=0"
exec_start=$(sed -n 's/^ExecStart=//p' "$units/board-health.service")
[ "$exec_start" = "/usr/lib/board/status --refresh-failed --quiet" ] || fail "board-health.service runs: $exec_start"
grep -qx 'OnUnitActiveSec=60s' "$units/board-health.timer" || fail "board-health.timer isn't the 60 s backstop"
for unit in "$units"/*.service "$units"/*.timer; do
	! grep -q -- '--refresh-hardware' "$unit" || fail "$(basename "$unit") polls lemnosd (board-agent keeps hardware.json)"
done
echo "board-health ok"
