#!/bin/sh
# linux/check-config.sh, the build's check that every option raze.config sets
# or turns off took effect in the final kernel config.
# Run: sh devices/raze/tests/kernel-config.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
check=$here/../gaia/buildroot-external/linux/check-config.sh
T=$(mktemp -d "${TMPDIR:-/tmp}/board-kernel-config.XXXXXX")
trap 'rm -rf "$T"' EXIT

fail() {
	echo "FAIL: $*" >&2
	exit 1
}

cat > "$T/fragment" <<'EOF'
# A comment, and a blank line, are ignored.

CONFIG_ARM64_4K_PAGES=y
CONFIG_EROFS_FS=y
CONFIG_LOCALVERSION="-raze"
# CONFIG_MODULE_COMPRESS is not set
# CONFIG_BLK_DEV_MD is not set
EOF

echo "every option took effect: ok, silently"
cat > "$T/good" <<'EOF'
CONFIG_ARM64_4K_PAGES=y
CONFIG_EROFS_FS=y
CONFIG_LOCALVERSION="-raze"
# CONFIG_MODULE_COMPRESS is not set
CONFIG_OTHER=m
EOF
out=$(sh "$check" "$T/fragment" "$T/good" 2>&1) || fail "a matching config should pass: $out"
[ -z "$out" ] || fail "nothing to say when all is well: $out"

echo "a 'not set' that came back as m or y, and a value that changed, are named"
cat > "$T/bad" <<'EOF'
CONFIG_ARM64_16K_PAGES=y
CONFIG_EROFS_FS=m
CONFIG_LOCALVERSION="-raze"
CONFIG_BLK_DEV_MD=m
EOF
if out=$(sh "$check" "$T/fragment" "$T/bad" 2>&1); then fail "a drifted config should fail"; fi
for want in "CONFIG_ARM64_4K_PAGES: wanted y, got n" "CONFIG_EROFS_FS: wanted y, got m" "CONFIG_BLK_DEV_MD: wanted n, got m"; do
	printf '%s\n' "$out" | grep -qF "$want" || fail "should name '$want': $out"
done
printf '%s\n' "$out" | grep -q "MODULE_COMPRESS\|LOCALVERSION" && fail "options that took effect aren't named: $out"

echo "the package's own raze.config parses (each line is a decision or a comment)"
awk '!/^#/ && !/^$/ && !/^CONFIG_[A-Za-z0-9_]+=/ { print FILENAME ":" FNR ": " $0; bad = 1 } END { exit bad }' \
	"$here/../gaia/buildroot-external/linux/raze.config" || fail "raze.config has lines the check can't read"

echo "ok"
