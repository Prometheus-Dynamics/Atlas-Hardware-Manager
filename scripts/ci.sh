#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root_dir"

echo "==> Checking formatting"
cargo fmt --all --check

echo "==> Checking file sizes"
"$root_dir/scripts/check-file-sizes.sh"

echo "==> Testing the device package (update writer, self-test, manifest lint)"
sh devices/raze/tests/update.sh
sh devices/raze/tests/root-device.sh
sh devices/raze/tests/usb-boot.sh
sh devices/raze/tests/selftest.sh
sh devices/raze/tests/manifest-lint.sh

echo "==> Running clippy"
cargo clippy --workspace --all-targets --all-features -- -D warnings

echo "==> Running tests"
cargo test --workspace
