#!/usr/bin/env bash
# The inner loop: only the checks a change can affect, each one a gate command (scripts/gate.sh) narrowed to the
# changed crates or boards. It is a worker's check; the full gate (scripts/gate.sh) is the lander's, on the merged
# result (AGENTS.md, "Workers and the landing gate").
#
# Usage:
#   scripts/check-scoped.sh            the changes against origin/next (committed, staged, unstaged and untracked)
#   scripts/check-scoped.sh BASE       the changes against BASE instead
#   scripts/check-scoped.sh --dry-run  print what would run, run nothing
#
# What a change selects (everything else is left to the full gate):
#   a crate's files          the gate's clippy and doctests (workspace-wide: cargo rebuilds only what changed and its
#                            dependents, with the gate's own features), and the tests of that crate and of every
#                            workspace crate that depends on it, directly or not (a nextest package filter; where nextest
#                            is absent, cargo test runs them all).
#   Cargo.toml, Cargo.lock,  every crate (a workspace-wide input).
#   rust-toolchain.toml
#   any .rs file             cargo fmt --all --check (the gate's step; it takes about a second).
#   devices/                 check-devices.sh (the whole tree: its checks are cross-file) and, when a product, board,
#                            family or component changed, check-config-only.sh.
#   devices/boards/B/        check-kernel-fragments.sh --only B and check-uboot-fragments.sh --only B (the real
#                            checks for that board; no self-tests).
#   devices/families/F/      check-kernel-fragments.sh and check-uboot-fragments.sh for every board (a family is
#                            shared: a family fragment changes every product of it, RK-FASTBOOT).
#   scripts/                 nothing scoped: the script says to run the full gate (a check itself changed).
# Left to the full gate: the device, kernel and U-Boot self-tests, the portable classification and the grep guards.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

dry=0 base=origin/next
for a in "$@"; do
  case $a in
    --dry-run) dry=1 ;;
    -h|--help) sed -n '2,27p' "$0"; exit 0 ;;
    *) base=$a ;;
  esac
done

# The default scratch root is beside the work tree, not in it: a test that needs a directory outside any repository
# (atlas-next bench_key) takes TMPDIR as that directory.
scratch_root=${ATLAS_SCRATCH_ROOT:-$(git rev-parse --show-toplevel).scratch}
mkdir -p "$scratch_root"
export TMPDIR
TMPDIR=$(mktemp -d "$scratch_root/scoped-XXXXXX")
trap 'rm -rf "$TMPDIR"' EXIT

mb=$(git merge-base "$base" HEAD)
changed=$( { git diff --name-only "$mb"; git diff --name-only --cached; git ls-files --others --exclude-standard; } | sort -u)
if [[ -z $changed ]]; then
  echo "check-scoped: no changes against $base"
  exit 0
fi

if grep -q '^scripts/' <<<"$changed"; then
  echo "check-scoped: scripts/ changed (a check itself): run the full gate, scripts/gate.sh" >&2
  exit 2
fi

# The crates: each changed file belongs to the workspace crate whose directory holds it; the selection is those crates
# and every workspace crate that depends on one of them (reverse dependencies, transitively), from cargo metadata.
crates=$(cargo metadata --format-version 1 --locked --offline --no-deps | python3 -I -c '
import json, os, sys
meta = json.load(sys.stdin)
root = meta["workspace_root"]
changed = [l for l in sys.argv[1].splitlines() if l]
pkgs = {p["name"]: p for p in meta["packages"]}
dirs = {name: os.path.relpath(os.path.dirname(p["manifest_path"]), root) for name, p in pkgs.items()}
if any(f in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml") for f in changed):
    print("\n".join(sorted(pkgs))); sys.exit()
hit = {n for n, d in dirs.items() for f in changed if f == d or f.startswith(d + "/")}
rdeps = {n: set() for n in pkgs}
for n, p in pkgs.items():
    for d in p["dependencies"]:
        if d.get("path") and d["name"] in pkgs:
            rdeps[d["name"]].add(n)
todo, sel = list(hit), set(hit)
while todo:
    for r in rdeps[todo.pop()]:
        if r not in sel:
            sel.add(r); todo.append(r)
print("\n".join(sorted(sel)))
' "$changed")

boards=() all_kernels=0 devices=0 config_only=0
while IFS= read -r f; do
  case $f in
    devices/boards/*/*) b=${f#devices/boards/}; boards+=("${b%%/*}"); devices=1; config_only=1 ;;
    devices/families/*) all_kernels=1; devices=1; config_only=1 ;;
    devices/products/*|devices/components/*) devices=1; config_only=1 ;;
    devices/*) devices=1 ;;
  esac
done <<<"$changed"
mapfile -t boards < <(printf '%s\n' "${boards[@]+"${boards[@]}"}" | sed '/^$/d' | sort -u)

# Three lanes, as in the gate: cargo; the device descriptions (then the config-only build, which uses the gaia binary
# the device check builds); the kernel. They run side by side, each logged apart, printed in this order.
cargo_lane=() devices_lane=() kernel_lane=()
if grep -q '\.rs$' <<<"$changed"; then cargo_lane+=("cargo fmt --all --check"); fi
if [[ -n $crates ]]; then
  # The build stays workspace-wide, as the gate's: selecting crates with -p changes feature unification and rebuilds
  # dependencies the gate already built (measured: 55 crates compiled for a one-line change). cargo rebuilds only the
  # changed crates and their dependents either way; what is narrowed is which tests run.
  cargo_lane+=("cargo clippy --locked --workspace --all-targets -- -D warnings")
  if cargo nextest --version >/dev/null 2>&1; then
    filter=$(sed 's/.*/package(&)/' <<<"$crates" | paste -sd'|' -)
    cargo_lane+=("cargo nextest run --locked --workspace --no-fail-fast --no-tests=pass -E $filter")
  else
    cargo_lane+=("cargo test --locked --workspace")
  fi
  cargo_lane+=("cargo test --locked --workspace --doc")
fi
if [[ $devices -eq 1 ]]; then
  devices_lane+=("scripts/check-devices.sh")
  # The gate's small data checks over devices/ (each about a second).
  for py in check-lemnos-gadget check-usb-ifname check-lemnos0-address check-wired-ethernet check-console-gate check-orion-link check-board-ids; do
    [[ -f scripts/$py.py ]] && devices_lane+=("python3 scripts/$py.py")
  done
fi
[[ $config_only -eq 1 ]] && devices_lane+=("scripts/check-config-only.sh")
if [[ $all_kernels -eq 1 ]]; then
  kernel_lane+=("scripts/check-kernel-fragments.sh")
  [[ -x scripts/check-uboot-fragments.sh ]] && kernel_lane+=("scripts/check-uboot-fragments.sh")
else
  for b in "${boards[@]+"${boards[@]}"}"; do
    # The kernel check resolves a board whose board.toml extends a family (the app-only board directories do not).
    if grep -qs '^extends' "devices/boards/$b/board.toml"; then
      kernel_lane+=("scripts/check-kernel-fragments.sh --only $b")
      [[ -x scripts/check-uboot-fragments.sh ]] && kernel_lane+=("scripts/check-uboot-fragments.sh --only $b")
    fi
  done
fi

echo "check-scoped: $(wc -l <<<"$changed") changed file(s) against $base; the full gate still runs before landing"
if [[ $(( ${#cargo_lane[@]} + ${#devices_lane[@]} + ${#kernel_lane[@]} )) -eq 0 ]]; then
  echo "check-scoped: nothing scoped to run for these files (docs or other files no check reads)"
  exit 0
fi
[[ -n $crates ]] && echo "  tests of: $(paste -sd' ' - <<<"$crates")"
for c in "${cargo_lane[@]+"${cargo_lane[@]}"}" "${devices_lane[@]+"${devices_lane[@]}"}" "${kernel_lane[@]+"${kernel_lane[@]}"}"; do
  echo "  $c"
done
[[ $dry -eq 1 ]] && exit 0

run_lane() {  # <command>...: each in order, stopping at the first failure
  local c start
  for c in "$@"; do
    echo "==> $c"
    start=$SECONDS
    # shellcheck disable=SC2086  # each command line is its own words, built above
    $c || return $?
    echo "    $((SECONDS - start)) s"
  done
}
pids=() logs=()
for lane in cargo devices kernel; do
  declare -n cmds=${lane}_lane
  [[ ${#cmds[@]} -gt 0 ]] || continue
  logs+=("$TMPDIR/$lane.log")
  ( run_lane "${cmds[@]}" ) >"$TMPDIR/$lane.log" 2>&1 &
  pids+=("$!")
done
rc=0 rcs=()
for i in "${!pids[@]}"; do r=0; wait "${pids[$i]}" || r=$?; rcs+=("$r"); done
for i in "${!pids[@]}"; do
  cat "${logs[$i]}"
  [[ ${rcs[$i]} -eq 0 ]] || { echo "check-scoped: FAILED in $(basename "${logs[$i]}" .log) (exit ${rcs[$i]})" >&2; rc=1; }
done
[[ $rc -eq 0 ]] && echo "check-scoped: passed (scoped; run scripts/gate.sh before landing)"
exit $rc
