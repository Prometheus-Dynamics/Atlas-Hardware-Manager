#!/bin/sh
# Off-device test: every service a .path unit starts has no start limit.
# With systemd's default (5 starts in 10 s), a burst of changes (the
# identity's watched files at boot) or of requests (locate pressed again
# and again) makes the service refuse to start; the path unit then fails
# with unit-start-limit-hit and watches nothing until the next boot.
# Run: sh devices/raze/tests/path-units.sh
set -eu
here=$(cd "$(dirname "$0")" && pwd)
units=$here/../gaia/assets/rootfs/usr/lib/systemd/system
failed=0
checked=0
for path in "$units"/*.path; do
	service=$(sed -n 's/^Unit=//p' "$path")
	[ -n "$service" ] || service=$(basename "$path" .path).service
	checked=$((checked + 1))
	if [ ! -f "$units/$service" ]; then
		echo "FAIL: $(basename "$path") starts $service, which the package doesn't have" >&2
		failed=1
	elif ! sed -n '/^\[Unit\]/,/^\[/p' "$units/$service" | grep -qx 'StartLimitIntervalSec=0'; then
		echo "FAIL: $service (started by $(basename "$path")) needs StartLimitIntervalSec=0 in [Unit]" >&2
		failed=1
	fi
done
[ "$checked" -gt 0 ] || { echo "FAIL: no .path units found in $units" >&2; exit 1; }
[ "$failed" = 0 ] || exit 1
echo "path-units ok ($checked)"
