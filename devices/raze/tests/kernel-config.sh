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

echo "an option an OS package's kernel fixups turn on (systemd: EFIVAR_FS) is reported, not failed"
cat > "$T/fixups" <<'EOF'
CONFIG_BLK_DEV_MD
CONFIG_UNRELATED
EOF
cat > "$T/fixed-up" <<'EOF'
CONFIG_ARM64_4K_PAGES=y
CONFIG_EROFS_FS=y
CONFIG_LOCALVERSION="-raze"
CONFIG_BLK_DEV_MD=m
EOF
out=$(sh "$check" "$T/fragment" "$T/fixed-up" "$T/fixups" 2>&1) || fail "a package fixup should pass: $out"
printf '%s\n' "$out" | grep -q "OS's Buildroot packages turn on" || fail "it should say so: $out"
printf '%s\n' "$out" | grep -q "CONFIG_BLK_DEV_MD: wanted n, got m" || fail "and name it: $out"
if sh "$check" "$T/fragment" "$T/bad" "$T/fixups" >/dev/null 2>&1; then fail "options no fixup explains still fail"; fi

echo "a later fragment wins (qemu.config's VIRTIO_MENU=y after raze.config's not set)"
cat "$here/../gaia/buildroot-external/linux/raze.config" "$here/../gaia/buildroot-external/linux/qemu.config" > "$T/both"
grep -qx '# CONFIG_VIRTIO_MENU is not set' "$T/both" || fail "raze.config no longer turns VIRTIO_MENU off: update qemu.config's note"
printf 'CONFIG_VIRTIO_MENU=y\nCONFIG_VIRTIO_BLK=y\n' > "$T/virt"
grep -h '^CONFIG_VIRTIO_MENU\|^CONFIG_VIRTIO_BLK' "$T/both" | sort -u > "$T/wanted"
printf 'CONFIG_VIRTIO_BLK=y\nCONFIG_VIRTIO_MENU=y\n' | cmp -s - "$T/wanted" || fail "qemu.config should ask for them: $(cat "$T/wanted")"
out=$(printf '# CONFIG_VIRTIO_MENU is not set\nCONFIG_VIRTIO_MENU=y\n' > "$T/order" && sh "$check" "$T/order" "$T/virt" 2>&1) || fail "the later line should win: $out"

echo "the package's own fragments parse (each line is a decision or a comment)"
for f in raze.config raze-trim.config qemu.config; do
	awk '!/^#/ && !/^$/ && !/^CONFIG_[A-Za-z0-9_]+=/ { print FILENAME ":" FNR ": " $0; bad = 1 } END { exit bad }' \
		"$here/../gaia/buildroot-external/linux/$f" || fail "$f has lines the check can't read"
done

echo "ok"
