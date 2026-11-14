#!/usr/bin/env bash
# Fails if the USB agent's flash path differs from its base commit: the
# block-device crate, flash.rs and claim.rs. The boot-only job must leave
# them byte-for-byte as they were, so a boot-only change can never reach
# the disk path.
set -euo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
base=295c1bed
diff=$(git -C "$root" diff "$base" -- crates/atlas-blockdev crates/atlas-usb-agent/src/flash.rs crates/atlas-usb-agent/src/claim.rs)
if [ -n "$diff" ]; then
  printf '%s\n' "$diff"
  echo "the flash path changed since $base" >&2
  exit 1
fi
echo "flash path unchanged since $base"
