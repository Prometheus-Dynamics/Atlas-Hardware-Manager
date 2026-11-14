#!/usr/bin/env bash
# Fails if the USB agent's flash path differs from its base commit (295c1bed):
#   - the block-device crate, flash.rs and claim.rs, byte for byte;
#   - serve.rs's `async fn start(` block (POST /v1/flash), line for line;
#   - the route lines for POST /v1/flash and PUT /v1/boot and /v1/images.
# A boot-only change must never reach the disk path, so these stay as they were.
set -euo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
base=295c1bed
serve=crates/atlas-usb-agent/src/serve.rs
status=0

diff=$(git -C "$root" diff "$base" -- crates/atlas-blockdev crates/atlas-usb-agent/src/flash.rs crates/atlas-usb-agent/src/claim.rs)
if [ -n "$diff" ]; then
  printf '%s\n' "$diff"
  echo "the flash path changed since $base" >&2
  status=1
fi

# The start block: from `async fn start(` to the first line that is exactly `}`.
start_block() {
  awk '/^async fn start\(/{f=1} f{print} f && /^}$/{exit}' 
}
base_start=$(git -C "$root" show "$base:$serve" | start_block)
head_start=$(start_block < "$root/$serve")
if [ -z "$base_start" ]; then
  echo "no async fn start( block at $base in $serve" >&2
  exit 1
fi
if [ "$base_start" != "$head_start" ]; then
  diff <(printf '%s\n' "$base_start") <(printf '%s\n' "$head_start") || true
  echo "serve.rs's async fn start( (POST /v1/flash) changed since $base" >&2
  status=1
fi

# The route lines, present unchanged.
routes=(
  '        (Method::POST, ["v1", "flash"]) => start(agent, request).await,'
  '        (Method::PUT, ["v1", "boot", name]) if BOOT_FILES.contains(name) => {'
  '        (Method::PUT, ["v1", "images", name]) if valid_name(name) => {'
)
for route in "${routes[@]}"; do
  if ! git -C "$root" show "$base:$serve" | grep -Fxq -- "$route"; then
    echo "the base has no route line: $route" >&2
    status=1
  elif ! grep -Fxq -- "$route" "$root/$serve"; then
    echo "route line changed or missing: $route" >&2
    status=1
  fi
done

if [ "$status" -ne 0 ]; then
  exit 1
fi
echo "flash path unchanged since $base (flash files, serve.rs start block, 3 route lines)"
