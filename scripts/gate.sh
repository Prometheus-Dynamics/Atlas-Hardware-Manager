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

# The config-only build of every product (PR B32, Gaia #120): each image's Buildroot configuration resolves, every
# fragment line takes as written, nothing is compiled. A cold GAIA_STORE fetches the kernel sources (network); a warm one is offline.
if [ -x scripts/check-config-only.sh ]; then
  step "config-only build (every product)" scripts/check-config-only.sh
fi

# Every line of every board's kernel fragments must take (PR B25). Needs the pinned kernel trees (fetched once,
# cached by the pin) and flex and bison. A source that cannot be fetched is a failure, here as in CI.
if [ -x scripts/check-kernel-fragments.sh ]; then
  step "kernel fragments" scripts/check-kernel-fragments.sh
  step "kernel fragments self-test" scripts/check-kernel-fragments.sh --self-test
fi

# The Lemnos gadget drop-in (R1d): installed exactly where the installed board file has a usb-gadget device.
if [ -f scripts/check-lemnos-gadget.py ]; then
  step "lemnos gadget drop-in pairing" python3 scripts/check-lemnos-gadget.py
fi

# The board-side USB network interface name (L2b): a networkd file that names the gadget's interface must name lemnos0.
if [ -f scripts/check-usb-ifname.py ]; then
  step "usb gadget interface name" python3 scripts/check-usb-ifname.py
fi

# The Raze's lemnos0 address (serial-hash-v1): the script's six vectors, Raze #1 (172.31.12.81/29), and its refusals.
if [ -f scripts/check-lemnos0-address.py ]; then
  step "lemnos0 address (serial-hash-v1)" python3 scripts/check-lemnos0-address.py
fi

# The Raze's console follows the board's security switch (LOCK: Lemnos S2): the getty's ExecCondition reads the view.
if [ -f scripts/check-console-gate.py ]; then
  step "console follows the security view" python3 scripts/check-console-gate.py
fi

# The Raze's Orion USB link: one name in orion.conf and in the bridge's LEMNOS_ORION_LINKS, from a usb-gadget device.
if [ -f scripts/check-orion-link.py ]; then
  step "orion usb link: one name on both sides" python3 scripts/check-orion-link.py
fi

# The board id: one string per board (directory, Gaia name, Lemnos [board] id, bring-up family line). PR B24.
if [ -f scripts/check-board-ids.py ]; then
  step "board ids agree" python3 scripts/check-board-ids.py
fi

# What must build on Windows and macOS (PR G9): the Orion-bound list against the graph. The
# cross checks need those targets, so CI runs them natively (the portable job in next.yml).
if [ -x scripts/check-portable.sh ]; then
  step "portable crates classification" scripts/check-portable.sh --classify
fi

# Nothing that runs on a board lives here except the flasher image description
# and the SystemCore package description, which are data under devices/.
#
# no_match passes only when grep finds nothing (status 1). A match (0) fails, and so does a grep error (2): with
# `! grep` an error counted as a pass, and grep reports 2 even when it also matched, so a glob that named a missing
# directory (apps/*/src-tauri/src before the app lands) turned both checks into no-ops (G10). Only directories that
# exist are searched.
no_match() {  # <grep args...> -- <dir globs...>
  local args=() dirs=() glob d status
  while [ "$1" != "--" ]; do args+=("$1"); shift; done
  shift
  for glob in "$@"; do
    for d in $glob; do [ -d "$d" ] && dirs+=("$d"); done
  done
  [ ${#dirs[@]} -gt 0 ] || return 0
  status=0
  grep "${args[@]}" "${dirs[@]}" || status=$?
  case $status in
    1) return 0 ;;
    0) echo "    found above: not allowed here" >&2; return 1 ;;
    *) echo "    grep failed (status $status): the check did not run" >&2; return 1 ;;
  esac
}
no_match_self_test() {
  local d
  d=$(mktemp -d "${TMPDIR:-/mnt/ssd-samsung/atlas-next-scratch-root}/gate-no-match.XXXXXX" 2>/dev/null) || d=$(mktemp -d)
  mkdir -p "$d/a" && echo "raze" > "$d/a/f"
  no_match -rn -i raze -- "$d/a" "$d/missing/*" >/dev/null 2>&1 && { rm -r "$d"; echo "no_match passed a match" >&2; return 1; }
  no_match -rn -i helios -- "$d/a" "$d/missing/*" || { rm -r "$d"; echo "no_match failed a clean tree" >&2; return 1; }
  rm -r "$d"
}
if compgen -G "crates/*/*/src" > /dev/null; then
  step "the grep guard refuses a match even with a missing directory" no_match_self_test
  step "no board or product names in code" no_match -rn -i -E "\b(raze|helios|photonvision|zero-?3e|rock-?5c|cubie|luckfox|k230d)\b" -- "crates/*/*/src" "apps/*/src-tauri/src"
  step "no shell-outs to ssh for board jobs" no_match -rn -E "Command::new\(\"(ssh|scp|sftp)\"\)" -- "crates/*/*/src" "apps/*/src-tauri/src"
fi

echo "gate passed"
