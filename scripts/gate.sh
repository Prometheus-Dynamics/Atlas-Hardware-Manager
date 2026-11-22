#!/usr/bin/env bash
# The check every PR into next must pass. CI runs exactly this.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
# Incremental compilation is left to cargo (on for dev and test builds): measured interleaved, 2026-11-18, an edit to
# atlas-types then clippy + the test build took 22/15/24 s incremental against 31/25/43 s without, for 5.3 GB of target
# against 3.1 GB. CI builds from scratch and sets CARGO_INCREMENTAL=0 itself (next.yml), where it only costs.

# Every temp file of the gate and of the scripts it runs lands under one scratch root, never /tmp (PR G12). mktemp
# and ${TMPDIR:-/tmp} both follow TMPDIR, which is set here for every step; the directory is removed on exit.
# The default scratch root is beside the work tree, not in it: a test that needs a directory outside any repository
# (atlas-next bench_key) takes TMPDIR as that directory.
scratch_root=${ATLAS_SCRATCH_ROOT:-$(git rev-parse --show-toplevel).scratch}
mkdir -p "$scratch_root"
gate_tmp=$(mktemp -d "$scratch_root/gate-XXXXXX")
export TMPDIR=$gate_tmp
trap 'rm -rf "$gate_tmp"' EXIT

# A Unix socket path must fit sun_path: 108 bytes with the NUL on Linux, so 107 bytes of path. A test node's socket
# lives under $XDG_RUNTIME_DIR, not under TMPDIR (atlas-types::socket::node_dir): the runtime dir is short by design,
# while TMPDIR is whatever the caller gave (on CI, a long build path). The base is XDG_RUNTIME_DIR when it is set and
# leaves room, else TMPDIR (the fallback, with its own room check). The fixed worst-case suffix past the base is
# NODE_SOCKET_SUFFIX_MAX = 52 bytes ("/atlas-t-" + 16-digit hash + "-" + 10-digit pid + "-" + 4-digit counter + "/orion.sock").
# A unit test (the_suffix_constant_is_the_gate_figure) keeps the Rust constant and this figure equal.
SUN_PATH_BYTES=108
SOCKET_SUFFIX_BYTES=52
socket_room_ok() {  # <dir>: 0 when a socket path under it fits sun_path, else prints why and returns 1
  local n
  n=$(printf '%s' "$1" | wc -c)
  if (( n + SOCKET_SUFFIX_BYTES >= SUN_PATH_BYTES )); then
    echo "gate: base $1 is $n bytes; a test socket path under it needs $((n + SOCKET_SUFFIX_BYTES)) bytes, the limit is $((SUN_PATH_BYTES - 1)). Set XDG_RUNTIME_DIR or ATLAS_SCRATCH_ROOT to a shorter directory." >&2
    return 1
  fi
}
# The runtime dir is the base when it is set and leaves room; the temp dir is checked only when it is not.
if [ -n "${XDG_RUNTIME_DIR:-}" ] && socket_room_ok "$XDG_RUNTIME_DIR" >/dev/null 2>&1; then
  :
else
  socket_room_ok "$gate_tmp" || exit 1
fi

step() {
  local label="$1" start
  shift
  echo "==> $label"
  start=$SECONDS
  "$@"
  echo "    $label: $((SECONDS - start)) s"
}

# The data checks (device descriptions, then the config-only build that uses the Gaia binary the first one builds;
# and the kernel fragments) read only devices/ and their own caches, never the cargo target: they run alongside the
# cargo steps, each with its log kept apart. Each is joined below, in this file's order, and its log printed whole.
# A failing one fails the gate as before. The gate waits for every background check before it exits, so none is left
# running against a removed TMPDIR.
bg_pids=() bg_labels=() bg_logs=()
bg() {  # <label> <function or command...>
  local label="$1" log="$gate_tmp/bg-${#bg_pids[@]}.log"
  shift
  ( "$@" ) >"$log" 2>&1 &
  bg_pids+=("$!"); bg_labels+=("$label"); bg_logs+=("$log")
}
bg_wait_all() { local p; for p in "${bg_pids[@]}"; do wait "$p" 2>/dev/null || true; done; }
trap 'bg_wait_all; rm -rf "$gate_tmp"' EXIT
join_bg() {  # <label>: waits for that background check, prints its log, fails the gate if it failed
  local i rc
  for i in "${!bg_labels[@]}"; do
    [ "${bg_labels[$i]}" = "$1" ] || continue
    rc=0; wait "${bg_pids[$i]}" || rc=$?
    echo "==> $1 (ran alongside the cargo steps)"
    cat "${bg_logs[$i]}"
    [ "$rc" -eq 0 ] || { echo "gate: $1 failed (exit $rc)" >&2; exit "$rc"; }
    return 0
  done
  echo "gate: no background check named $1" >&2; exit 1
}

devices_then_config_only() {
  # Board files are data. The config-only build of every product (PR B32, Gaia #120): each image's Buildroot
  # configuration resolves, every fragment line takes as written, nothing is compiled. A cold GAIA_STORE fetches the
  # kernel sources (network); a warm one is offline. It needs the gaia binary check-devices.sh builds, so it runs after.
  if [ -x scripts/check-devices.sh ]; then
    step "device descriptions" scripts/check-devices.sh
  fi
  if [ -x scripts/check-config-only.sh ]; then
    step "config-only build (every product)" scripts/check-config-only.sh
  fi
}
# A fresh worktree's empty target starts from a seed (scripts/cargo-seed.sh; only with ATLAS_CARGO_SEED_DIR set).
scripts/cargo-seed.sh restore
# The target records its worktree, so scripts/prune-targets.sh can tell whose cache it is.
mkdir -p "${CARGO_TARGET_DIR:-target}" && pwd > "${CARGO_TARGET_DIR:-target}/.atlas-worktree"

bg "device descriptions, then config-only" devices_then_config_only
# Every line of every board's kernel fragments must take (PR B25). Needs the pinned kernel trees (fetched once,
# cached by the pin) and flex and bison. A source that cannot be fetched is a failure, here as in CI.
# The self-test runs the real check first (every board, failing on any line that does not take) and then its negative
# cases, so one step covers both; running the real check again as its own step only repeated it.
if [ -x scripts/check-kernel-fragments.sh ]; then
  bg "kernel fragments" step "kernel fragments (the check, then its self-test)" scripts/check-kernel-fragments.sh --self-test
fi

# The workspace globs match nothing until the first crate lands.
if compgen -G "crates/*/*/Cargo.toml" > /dev/null || compgen -G "apps/*/src-tauri/Cargo.toml" > /dev/null; then
  step format cargo fmt --all --check
  # Cargo.lock is committed. --locked fails the gate when it is stale: run
  # `cargo update -w` (or just build once without --locked) and commit it.
  step clippy cargo clippy --locked --workspace --all-targets -- -D warnings
  # cargo-nextest runs every test as its own process, all binaries at once (cargo test runs one binary at a time):
  # the same cases (it skips the #[ignore]d ones as cargo test does), but not doctests, which run after it. Tests that
  # share something across processes take a lock for it (atlas-sim-node::ports reserves ports by file lock). CI installs
  # nextest (next.yml); where it is not installed, cargo test runs the same cases.
  if cargo nextest --version >/dev/null 2>&1; then
    step tests cargo nextest run --locked --workspace --no-fail-fast
    step doctests cargo test --locked --workspace --doc
  else
    step tests cargo test --locked --workspace
  fi
else
  echo "no crates yet"
fi

# The app's own checks, once it exists (F2 adds the script).
if [ -x scripts/ui-gate.sh ]; then
  step "ui gate" scripts/ui-gate.sh
fi

join_bg "device descriptions, then config-only"
if [ -x scripts/check-kernel-fragments.sh ]; then
  join_bg "kernel fragments"
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
socket_room_self_test() {
  local long short
  long=$(printf '%0.s/x' $(seq 1 40))   # 80 bytes: must be refused
  short=/mnt/ssd-samsung/atlas-next-scratch-G12/gate-abcdef   # 52 bytes: must be accepted
  socket_room_ok "$long" >/dev/null 2>&1 && { echo "gate self-test: a TMPDIR that leaves no room for a socket was accepted" >&2; return 1; }
  socket_room_ok "$short" || { echo "gate self-test: a short TMPDIR was refused" >&2; return 1; }
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
  step "the socket path room check refuses a TMPDIR too long for sun_path" socket_room_self_test
  step "the grep guard refuses a match even with a missing directory" no_match_self_test
  step "no board or product names in code" no_match -rn -i -E "\b(raze|helios|photonvision|zero-?3e|rock-?5c|cubie|luckfox|k230d)\b" -- "crates/*/*/src" "apps/*/src-tauri/src"
  step "no shell-outs to ssh for board jobs" no_match -rn -E "Command::new\(\"(ssh|scp|sftp)\"\)" -- "crates/*/*/src" "apps/*/src-tauri/src"
fi

# A green gate's target becomes the seed for its key when there is none yet (only with ATLAS_CARGO_SEED_DIR set).
scripts/cargo-seed.sh save
echo "gate passed"
