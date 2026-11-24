#!/usr/bin/env bash
# The app's own checks (F2). The gate runs this (scripts/gate.sh, "ui gate").
#
#   scripts/ui-gate.sh                 every check below
#   scripts/ui-gate.sh --screens DIR   the same, and a picture of every screen and of its mockup
#                                      into DIR, light and dark, for comparing side by side
#   scripts/ui-gate.sh -- ARGS         ARGS go to playwright (for one test: -- -g "name")
#
# What it checks:
#   1. Every Tauri command is async: a plain `#[tauri::command] fn` runs on the thread that draws
#      the window, so one slow call would freeze the app.
#   2. The app's stylesheet is the design language's: apps/desktop/src/styles/atlas.css is
#      docs/mockups/atlas.css, byte for byte. What the app adds is in app.css, and names no colour.
#   3. The UI builds and type-checks.
#   4. The app's Rust side passes clippy and its tests with the simulated devices (feature `sim`,
#      which the workspace steps leave off).
#   5. The click tests: the built UI in headless Chromium against the UI bridge, which is the real
#      core with the simulated devices. Real pointer clicks; no function is called from a test
#      except to change the simulated world (unplug a board).
#
# Needs: bun, and Playwright's headless Chromium (fetched once into PLAYWRIGHT_BROWSERS_PATH,
# default <target>/ms-playwright; `--no-fetch` fails instead of fetching).
set -euo pipefail
# Each stage says when it starts, in seconds since the gate did, so a slow stage is seen in the log.
stage() { echo "==> ui gate: $* (+$SECONDS s)"; }
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"

screens="" fetch=1 pw_args=()
while [ $# -gt 0 ]; do
  case $1 in
    --screens) screens=$(mkdir -p "$2" && cd "$2" && pwd); shift 2 ;;
    --no-fetch) fetch=0; shift ;;
    --) shift; pw_args=("$@"); break ;;
    -h|--help) sed -n '2,24p' "$0"; exit 0 ;;
    *) echo "ui-gate: unknown argument $1" >&2; exit 2 ;;
  esac
done

app=apps/desktop
tauri_src=$app/src-tauri/src
target=${CARGO_TARGET_DIR:-$root/target}
case $target in /*) ;; *) target=$root/$target ;; esac

stage "every Tauri command is async"
# The line after each #[tauri::command] must not be a plain `fn`. grep exits 1 for no match (the
# pass), 0 for a match (a failure) and 2 for an error (also a failure: the check did not run).
commands=$(grep -rn -A1 --include='*.rs' '#\[tauri::command' "$tauri_src") || { echo "ui-gate: no Tauri command found under $tauri_src: the check has nothing to read" >&2; exit 1; }
if bad=$(grep -E -- '-[0-9]+-\s*(pub(\([a-z]+\))? )?fn ' <<<"$commands"); then
  echo "ui-gate: Tauri commands on the main thread (make them async):" >&2
  echo "$bad" >&2
  exit 1
fi

stage "the app's stylesheet is the design language's"
cmp docs/mockups/atlas.css $app/src/styles/atlas.css
if grep -nE '#[0-9a-fA-F]{3,8}\b|rgba?\(' $app/src/styles/app.css; then
  echo "ui-gate: app.css names a colour; colours are tokens in atlas.css" >&2
  exit 1
fi

stage "the UI builds and type-checks"
# The build goes to a scratch directory first and only files whose bytes changed are copied into
# dist/, so an unchanged UI leaves dist/ untouched: the app crate embeds dist/, and a rewritten
# dist/ with new mtimes recompiled it in each of the three cargo steps below.
(cd $app && bun install --frozen-lockfile >/dev/null)
built=$target/ui-dist
(cd $app && bun run build --outDir "$built" --emptyOutDir >/dev/null)
[ -f "$built/index.html" ] || { echo "ui-gate: the UI build wrote no index.html into $built" >&2; exit 1; }
mkdir -p $app/dist
rsync -r --checksum --delete "$built/" "$app/dist/"
# Two things run beside the steps after them, each in its own process group: the type-check
# (UI sources only), and the app's Rust side with the simulated devices (clippy, then its tests).
# The click tests need only the bridge binary, so they start as soon as it is built, without
# waiting for the type-check. A failing step
# ends the gate; the groups still running go with it (by their recorded PIDs).
bg_pids=()
trap 'for p in "${bg_pids[@]}"; do kill -TERM -- "-$p" 2>/dev/null || true; done' EXIT
check_log=$target/ui-check.log
setsid bash -c 'cd "$1" && exec bun run check' ui-check "$app" >"$check_log" 2>&1 &
check_pid=$!
bg_pids+=("$check_pid")

stage "the UI bridge with the simulated devices"
cargo build --locked -p atlas-app --features sim --bin atlas-ui-bridge
# The click tests run a copy: `cargo test` below rebuilds the package's binaries in target/debug while they run.
mkdir -p "$target/ui-bridge"
cp -f "$target/debug/atlas-ui-bridge" "$target/ui-bridge/atlas-ui-bridge"

stage "the app's Rust side with the simulated devices (beside the click tests; log $target/ui-rust.log)"
rust_log=$target/ui-rust.log
setsid bash -c 'cargo clippy --locked -p atlas-app --features sim --all-targets -- -D warnings && cargo test --locked -p atlas-app --features sim' \
  ui-rust >"$rust_log" 2>&1 &
rust_pid=$!
bg_pids+=("$rust_pid")

stage "click tests"
export PLAYWRIGHT_BROWSERS_PATH=${PLAYWRIGHT_BROWSERS_PATH:-$target/ms-playwright}
if ! compgen -G "$PLAYWRIGHT_BROWSERS_PATH/chromium_headless_shell-*" >/dev/null; then
  [ "$fetch" -eq 1 ] || { echo "ui-gate: no headless Chromium in $PLAYWRIGHT_BROWSERS_PATH" >&2; exit 1; }
  (cd $app && node node_modules/@playwright/test/cli.js install --only-shell chromium)
fi
export ATLAS_UI_BRIDGE=$target/ui-bridge/atlas-ui-bridge
export ATLAS_E2E_OUT=$target/ui-gate
[ -z "$screens" ] || export ATLAS_SCREENS_DIR=$screens
(cd $app && node node_modules/@playwright/test/cli.js test -c e2e/playwright.config.ts "${pw_args[@]+"${pw_args[@]}"}")

# The type-check ran beside the bridge's build and the clicks: they need only the built UI, not
# its types (8 s of svelte-check no longer waits in front of them).
stage "waiting for the type-check"
if ! wait "$check_pid"; then
  cat "$check_log" >&2
  echo "ui-gate: the UI does not type-check (svelte-check, above)" >&2
  exit 1
fi
tail -n 3 "$check_log"

stage "waiting for the Rust side"
if ! wait "$rust_pid"; then
  cat "$rust_log" >&2
  echo "ui-gate: the app's Rust side with the simulated devices failed (clippy or its tests, above)" >&2
  exit 1
fi
grep -E '^test result:|Finished' "$rust_log" | tail -n 4
trap - EXIT
echo "ui gate passed (+$SECONDS s)"
