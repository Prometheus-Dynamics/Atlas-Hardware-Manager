#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root_dir"

echo "==> Checking formatting"
cargo fmt --all --check

echo "==> Checking file sizes"
"$root_dir/scripts/check-file-sizes.sh"

echo "==> Testing the device package scripts"
sh devices/raze/tests/update.sh
sh devices/raze/tests/root-device.sh
sh devices/raze/tests/usb-boot.sh
sh devices/raze/tests/ssh-keys.sh
sh devices/raze/tests/stage-files.sh
sh devices/raze/tests/gadget-address.sh

echo "==> Running clippy"
cargo clippy --workspace --all-targets --all-features -- -D warnings

echo "==> Running tests"
cargo test --workspace
