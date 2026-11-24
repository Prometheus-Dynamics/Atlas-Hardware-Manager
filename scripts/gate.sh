#!/usr/bin/env bash
# The check every PR into next must pass. CI runs exactly this.
#
# Shape (the one-minute rule, 2026-11-24): the cheap static checks first, all at once, failing fast; then the lanes,
# all at once, each in its own target directory so none waits on another's cargo lock. A lane whose inputs did not
# change since it passed is not run again (scripts/gate-lib.sh, the result cache), and a gate whose code tree did not
# change at all since it passed is done at once. The first failing lane stops the others.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
# shellcheck source=scripts/gate-lib.sh
. scripts/gate-lib.sh
gate_build_env
gate_started=$SECONDS

# Every temp file of the gate and of the scripts it runs lands under one scratch root, never /tmp (PR G12). mktemp
# and ${TMPDIR:-/tmp} both follow TMPDIR, which is set here for every step; the directory is removed on exit.
# The default scratch root is beside the work tree, not in it: a test that needs a directory outside any repository
# (atlas-next bench_key) takes TMPDIR as that directory.
scratch_root=${ATLAS_SCRATCH_ROOT:-$(git rev-parse --show-toplevel).scratch}
mkdir -p "$scratch_root"
gate_tmp=$(mktemp -d "$scratch_root/gate-XXXXXX")
export TMPDIR=$gate_tmp

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

# One target directory, one worktree. A target another worktree built in is refused: cargo decides "fresh" by file
# times, so a crate built there from NEWER files is taken as up to date for this tree's OLDER (different) sources, and
# the gate then passes or fails on code that is not in this tree (2026-11-23: pr/move13 failed to compile
# atlas-board-slots against pr/p1's atlas-bundle, with CARGO_TARGET_DIR shared between the two worktrees).
target=${CARGO_TARGET_DIR:-target}
case $target in /*) ;; *) target=$PWD/$target ;; esac
export CARGO_TARGET_DIR=$target
if [ -f "$target/.atlas-worktree" ] && [ "$(cat "$target/.atlas-worktree")" != "$(pwd)" ]; then
  echo "gate: $target was built in by $(cat "$target/.atlas-worktree"), not by $(pwd): give this worktree its own target directory" >&2
  exit 1
fi
# A fresh worktree's empty target starts from a seed (scripts/cargo-seed.sh; only with ATLAS_CARGO_SEED_DIR set).
scripts/cargo-seed.sh restore
# The target records its worktree, so scripts/prune-targets.sh can tell whose cache it is.
mkdir -p "$target" && pwd > "$target/.atlas-worktree"

# The whole gate's key: nothing tested changed since a passing gate, so the gate is done (no ceremony runs).
gate_key=$(gate_tree_key gate "${GATE_CODE[@]}")
if gate_known gate "$gate_key"; then
  echo "gate: nothing tested changed since the gate passed ($(gate_known_since gate "$gate_key")): code tree $gate_key"
  echo "gate passed"
  exit 0
fi

# Every lane runs in the background in its own process group, with its log kept apart, and is waited for below. A lane
# that fails stops the gate: the other lanes' process groups are stopped by the PIDs recorded here (never by name)
# and the failing lane's log is printed.
declare -A lane_name=() lane_log=() lane_t0=()
lane_pids=()
stop_lanes() {
  local p
  for p in "${lane_pids[@]+"${lane_pids[@]}"}"; do kill -TERM -- "-$p" 2>/dev/null || true; done
  for p in "${lane_pids[@]+"${lane_pids[@]}"}"; do wait "$p" 2>/dev/null || true; done
}
trap 'stop_lanes; rm -rf "$gate_tmp"' EXIT
lane() {  # <name> <lane_cmd argument>: starts a lane
  local log="$gate_tmp/lane-$1.log" pid
  setsid bash -c '. scripts/gate-lib.sh; lane_cmd "$1"' lane "$1" >"$log" 2>&1 &
  pid=$!
  lane_pids+=("$pid"); lane_name[$pid]=$1; lane_log[$pid]=$log; lane_t0[$pid]=$SECONDS
}
join_lanes() {  # waits for every lane, reporting each as it ends; the first failure ends the gate
  local done_pid rc p rest
  while (( ${#lane_pids[@]} > 0 )); do
    rc=0
    wait -n -p done_pid "${lane_pids[@]}" || rc=$?
    rest=()
    for p in "${lane_pids[@]}"; do [ "$p" = "$done_pid" ] || rest+=("$p"); done
    lane_pids=("${rest[@]+"${rest[@]}"}")
    if [ "$rc" -ne 0 ]; then
      echo "==> ${lane_name[$done_pid]}: FAILED (exit $rc) after $((SECONDS - lane_t0[$done_pid])) s"
      cat "${lane_log[$done_pid]}"
      echo "gate: ${lane_name[$done_pid]} failed; the other lanes were stopped" >&2
      exit "$rc"
    fi
    echo "==> ${lane_name[$done_pid]}: passed in $((SECONDS - lane_t0[$done_pid])) s"
    cat "${lane_log[$done_pid]}"
  done
}

step() {
  local label="$1" start
  shift
  echo "==> $label"
  start=$SECONDS
  "$@"
  echo "    $label: $((SECONDS - start)) s"
}

# A lane that keeps its pass: run only when its inputs changed. <lane> <pathspec-array-name> <command...>
cached() {
  local name=$1 spec=$2 key
  shift 2
  local -n _cached_paths=$spec
  key=$(gate_tree_key "$name $*" "${_cached_paths[@]}")
  if gate_known "$name" "$key"; then
    echo "    $name: nothing it reads changed since it passed ($(gate_known_since "$name" "$key"))"
    return 0
  fi
  "$@"
  gate_mark "$name" "$key"
}

# --- The static checks: each reads a few files and takes about a second; all of them run at once, first. ---------

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
export -f no_match socket_room_ok socket_room_self_test no_match_self_test step
export SUN_PATH_BYTES SOCKET_SUFFIX_BYTES

static_checks() {
  local s=()
  have_crates() { compgen -G "crates/*/*/Cargo.toml" > /dev/null || compgen -G "apps/*/src-tauri/Cargo.toml" > /dev/null; }
  have_crates && s+=("format|cargo fmt --all --check")
  # The Lemnos gadget drop-in (R1d): installed exactly where the installed board file has a usb-gadget device.
  [ -f scripts/check-lemnos-gadget.py ] && s+=("lemnos gadget drop-in pairing|python3 scripts/check-lemnos-gadget.py")
  # The board-side USB network interface name (L2b): a networkd file that names the gadget's interface must name lemnos0.
  [ -f scripts/check-usb-ifname.py ] && s+=("usb gadget interface name|python3 scripts/check-usb-ifname.py")
  # The Raze's lemnos0 address (serial-hash-v1): the script's six vectors, Raze #1 (172.31.12.81/29), and its refusals.
  [ -f scripts/check-lemnos0-address.py ] && s+=("lemnos0 address (serial-hash-v1)|python3 scripts/check-lemnos0-address.py")
  if [ -f scripts/check-slots-files.py ]; then
    s+=("bridge files (every file-taking device is given its directory)|python3 -I scripts/check-slots-files.py")
    s+=("bridge files self-test|python3 -I scripts/check-slots-files.py --self-test")
  fi
  # The Raze's wired Ethernet: one network file by type, the USB gadget's link matched by none.
  if [ -f scripts/check-wired-ethernet.py ]; then
    s+=("wired ethernet (not the gadget's link)|python3 scripts/check-wired-ethernet.py")
    s+=("wired ethernet self-test|python3 scripts/check-wired-ethernet.py --self-test")
  fi
  # Every shell script an image ships calls only commands its product lists, and the list is held to the product's
  # BusyBox config where that is a whole file (board-journal called chown on an image without it, 2026-11-23).
  if [ -f scripts/check-script-commands.py ]; then
    s+=("shipped scripts' commands|python3 -I scripts/check-script-commands.py")
    s+=("shipped scripts' commands self-test|python3 -I scripts/check-script-commands.py --self-test")
  fi
  # The board's small boot scripts (the Orion name from the serial, the journal's bind), run against made-up inputs.
  [ -f scripts/check-boot-scripts.sh ] && s+=("boot scripts|scripts/check-boot-scripts.sh")
  # Every board name and marker at the pinned revs (docs/pin-move-checklist.md, step 15).
  if [ -f scripts/check-board-names.sh ]; then
    s+=("board names at the pin|scripts/check-board-names.sh")
    s+=("board names self-test|scripts/check-board-names.sh --self-test")
  fi
  # The Raze's console follows the board's security switch (LOCK: Lemnos 214c0ca7 drivers/system-security/src/flow.rs:499): the getty's ExecCondition reads the view.
  [ -f scripts/check-console-gate.py ] && s+=("console follows the security view|python3 scripts/check-console-gate.py")
  # The Raze's Orion USB link: one name in orion.conf and in the bridge's LEMNOS_ORION_LINKS, from a usb-gadget device.
  [ -f scripts/check-orion-link.py ] && s+=("orion usb link: one name on both sides|python3 scripts/check-orion-link.py")
  # The board id: one string per board (directory, Gaia name, Lemnos [board] id, bring-up family line). PR B24.
  [ -f scripts/check-board-ids.py ] && s+=("board ids agree|python3 scripts/check-board-ids.py")
  # What must build on Windows and macOS (PR G9): the Orion-bound list against the graph.
  [ -x scripts/check-portable.sh ] && s+=("portable crates classification|scripts/check-portable.sh --classify")
  if compgen -G "crates/*/*/src" > /dev/null; then
    s+=("the socket path room check refuses a TMPDIR too long for sun_path|socket_room_self_test")
    s+=("the grep guard refuses a match even with a missing directory|no_match_self_test")
    s+=("no board or product names in code|no_match -rn -i -E '\\b(raze|helios|photonvision|zero-?3e|rock-?5c|cubie|luckfox|k230d)\\b' -- 'crates/*/*/src' 'apps/*/src-tauri/src'")
    s+=("no shell-outs to ssh for board jobs|no_match -rn -E 'Command::new\\(\"(ssh|scp|sftp)\"\\)' -- 'crates/*/*/src' 'apps/*/src-tauri/src'")
  fi
  local pids=() names=() logs=() i item rc failed=0
  for i in "${!s[@]}"; do
    item=${s[$i]}
    names+=("${item%%|*}"); logs+=("$gate_tmp/static-$i.log")
    # Each check records its own seconds, so a slow one is named, not guessed.
    (t0=$EPOCHREALTIME; rc=0; bash -c "${item#*|}" || rc=$?; t1=$EPOCHREALTIME
     printf '%s\n' "$(( (${t1/./} - ${t0/./}) / 1000000 ))" >"$gate_tmp/static-$i.secs"; exit "$rc") \
      >"$gate_tmp/static-$i.log" 2>&1 &
    pids+=("$!")
  done
  for i in "${!pids[@]}"; do
    rc=0; wait "${pids[$i]}" || rc=$?
    if [ "$rc" -ne 0 ]; then
      echo "==> ${names[$i]}: FAILED (exit $rc)"; cat "${logs[$i]}"; failed=1
    else
      echo "    ${names[$i]}: ok ($(cat "$gate_tmp/static-$i.secs" 2>/dev/null || echo '?') s)"
    fi
  done
  return "$failed"
}

echo "==> static checks (all at once, before any build)"
static_start=$SECONDS
static_checks || { echo "gate: a static check failed (after $((SECONDS - static_start)) s); nothing was built" >&2; exit 1; }
echo "    static checks: $((SECONDS - static_start)) s"

# --- The lanes: all at once, each cached on what it reads. --------------------------------------------------------

# The data checks read devices/ and the scripts (and their own caches), never the cargo target.
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
export -f devices_then_config_only cached gate_tree_key gate_known gate_mark gate_known_since gate_pass_dir

# Each cargo lane builds in its own target directory under this worktree's target, so the lanes run at once instead
# of one after another behind a single cargo lock. sccache shares their compiled dependencies.
export GATE_TARGET=$target
lane_target() { printf '%s/lane-%s' "$GATE_TARGET" "$1"; }

run_cached() {  # <lane> <kind: data|code> <command...> (in a lane's subshell)
  local name=$1 kind=$2
  shift 2
  local paths
  if [ "$kind" = data ]; then paths=(devices scripts ':(exclude)*.md'); else paths=("${GATE_CODE[@]}"); fi
  cached "$name" paths "$@"
}
export -f run_cached lane_target

lane_cmd() {  # the body of a lane, by name
  case $1 in
    devices) run_cached devices data devices_then_config_only ;;
    kernel) if [ -x scripts/check-kernel-fragments.sh ]; then run_cached kernel data step "kernel fragments (the check, then its self-test)" scripts/check-kernel-fragments.sh --self-test; fi ;;
    uboot) if [ -x scripts/check-uboot-fragments.sh ]; then run_cached uboot data step "u-boot fragments (the check, then its self-test)" scripts/check-uboot-fragments.sh --self-test; fi ;;
    clippy)
      # Cargo.lock is committed. --locked fails the gate when it is stale: run `cargo update -w` and commit it.
      CARGO_TARGET_DIR=$(lane_target clippy) run_cached clippy code step clippy cargo clippy --locked --workspace --all-targets -- -D warnings ;;
    alone)
      # The two programs, built alone. A workspace build turns on every feature some member asks for, a test-only one
      # included (atlas-board-slots' bundle-source, which its own tests enable): alone, the programs get none of them,
      # as a release build does. So a program that reaches for a test-only source fails here.
      CARGO_TARGET_DIR=$(lane_target alone) run_cached alone code step "programs alone (no test-only feature)" cargo check --locked -p atlas-next -p atlas-app ;;
    tests)
      # cargo-nextest runs every test as its own process, all binaries at once; doctests run after it. Tests that share
      # something across processes take a lock for it (atlas-sim-node::ports reserves ports by file lock).
      if cargo nextest --version >/dev/null 2>&1; then
        CARGO_TARGET_DIR=$(lane_target tests) run_cached tests code bash -c 'step tests cargo nextest run --locked --workspace --no-fail-fast && step doctests cargo test --locked --workspace --doc'
      else
        CARGO_TARGET_DIR=$(lane_target tests) run_cached tests code step tests cargo test --locked --workspace
      fi ;;
    ui)
      # The app's own checks: its UI build, its Rust side with the simulated devices, and the click tests. The browser
      # stays in the worktree's target (fetched once).
      if [ -x scripts/ui-gate.sh ]; then
        PLAYWRIGHT_BROWSERS_PATH=${PLAYWRIGHT_BROWSERS_PATH:-$GATE_TARGET/ms-playwright} \
          CARGO_TARGET_DIR=$(lane_target ui) run_cached ui code step "ui gate" scripts/ui-gate.sh
      fi ;;
  esac
}
export -f lane_cmd

have_crates() { compgen -G "crates/*/*/Cargo.toml" > /dev/null || compgen -G "apps/*/src-tauri/Cargo.toml" > /dev/null; }
lane devices lane_cmd devices
lane kernel lane_cmd kernel
lane uboot lane_cmd uboot
if have_crates; then
  lane clippy lane_cmd clippy
  lane alone lane_cmd alone
  lane tests lane_cmd tests
else
  echo "no crates yet"
fi
lane ui lane_cmd ui
join_lanes

gate_mark gate "$gate_key"
# A green gate's target becomes the seed for its key when there is none yet (only with ATLAS_CARGO_SEED_DIR set).
scripts/cargo-seed.sh save
echo "gate passed in $((SECONDS - gate_started)) s"
