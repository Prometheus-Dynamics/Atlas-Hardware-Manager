#!/bin/sh
# Every device package test (each *.sh next to this file), in parallel, each
# under the POSIX sh that runs this file. Prints each test's output in order
# and fails if any test does. CI and scripts/ci.sh run this, so a new test is
# picked up without being listed anywhere.
# Run: sh devices/raze/tests/all.sh
set -u

here=$(cd "$(dirname "$0")" && pwd)
self=$(basename "$0")
T=$(mktemp -d "${TMPDIR:-/tmp}/board-tests.XXXXXX")
trap 'rm -rf "$T"' EXIT
sh=${TEST_SH:-sh}

tests=
for t in "$here"/*.sh; do
	name=$(basename "$t" .sh)
	[ "$name.sh" = "$self" ] && continue
	tests="$tests $name"
	( "$sh" "$t" > "$T/$name.log" 2>&1; echo $? > "$T/$name.rc" ) &
done
wait

failed=
for name in $tests; do
	rc=$(cat "$T/$name.rc")
	if [ "$rc" = 0 ]; then
		echo "$name ok"
	else
		echo "==> $name FAILED (exit $rc)"
		cat "$T/$name.log"
		failed="$failed $name"
	fi
done
if [ -n "$failed" ]; then
	echo "failed:$failed" >&2
	exit 1
fi
