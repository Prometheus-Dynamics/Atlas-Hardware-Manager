#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root_dir"

echo "==> Checking formatting"
cargo fmt --all --check

echo "==> Checking file sizes"
"$root_dir/scripts/check-file-sizes.sh"

echo "==> Testing the device package (every devices/raze/tests/*.sh, in parallel)"
sh devices/raze/tests/all.sh

echo "==> Running clippy"
cargo clippy --workspace --all-targets --all-features -- -D warnings

echo "==> Running tests"
# cargo-nextest runs every test binary's tests at once (cargo test runs the
# binaries one after another): 8 s instead of 24 s here. Doc tests are
# cargo test's alone.
if cargo nextest --version >/dev/null 2>&1; then
  cargo nextest run --workspace
  cargo test --workspace --doc
else
  cargo test --workspace
fi
