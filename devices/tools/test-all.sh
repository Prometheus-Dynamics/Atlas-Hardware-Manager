#!/bin/sh
# Every device test (each *.sh in a tests/ directory under devices/), in
# parallel, each under the POSIX sh that runs this file. Prints each test's output in order
# and fails if any test does. CI and scripts/ci.sh run this, so a new test is
# picked up without being listed anywhere.
# Run: sh devices/tools/test-all.sh
set -u

here=$(cd "$(dirname "$0")" && pwd)
T=$(mktemp -d "${TMPDIR:-/tmp}/board-tests.XXXXXX")
trap 'rm -rf "$T"' EXIT
# TEST_SH may be a command with arguments, e.g. TEST_SH='busybox sh'.
sh=${TEST_SH:-sh}

tests=
for t in "$here"/../*/tests/*.sh "$here"/../*/*/tests/*.sh; do
	[ -f "$t" ] || continue
	name=$(basename "$t" .sh)
	tests="$tests $name"
	# shellcheck disable=SC2086
	( $sh "$t" > "$T/$name.log" 2>&1; echo $? > "$T/$name.rc" ) &
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
