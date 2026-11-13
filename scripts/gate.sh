#!/usr/bin/env bash
# The check every PR into next must pass. CI runs exactly this.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}"

step() {
  local label="$1" start
  shift
  echo "==> $label"
  start=$SECONDS
  "$@"
  echo "    $label: $((SECONDS - start)) s"
}

# The workspace globs match nothing until the first crate lands.
if compgen -G "crates/*/*/Cargo.toml" > /dev/null || compgen -G "apps/*/src-tauri/Cargo.toml" > /dev/null; then
  step format cargo fmt --all --check
  # Cargo.lock is committed. --locked fails the gate when it is stale: run
  # `cargo update -w` (or just build once without --locked) and commit it.
  step clippy cargo clippy --locked --workspace --all-targets -- -D warnings
  step tests cargo test --locked --workspace
else
  echo "no crates yet"
fi

# The app's own checks, once it exists (F2 adds the script).
if [ -x scripts/ui-gate.sh ]; then
  step "ui gate" scripts/ui-gate.sh
fi

# Board files are data. They are checked by their own script once B1 adds it.
if [ -x scripts/check-devices.sh ]; then
  step "device descriptions" scripts/check-devices.sh
fi

# Nothing that runs on a board lives here except the flasher image description
# and the SystemCore package description, which are data under devices/.
if compgen -G "crates/*/*/src" > /dev/null; then
  step "no board or product names in code" bash -c '! grep -rn -i -E "\b(raze|helios|photonvision|zero-?3e|rock-?5c|cubie|luckfox|k230d)\b" crates/*/*/src apps/*/src-tauri/src 2>/dev/null'
  step "no shell-outs to ssh for board jobs" bash -c '! grep -rn -E "Command::new\(\"(ssh|scp|sftp)\"\)" crates/*/*/src apps/*/src-tauri/src 2>/dev/null'
fi

echo "gate passed"
