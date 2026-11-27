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
# The result cache and the data key (scripts/gate-lib.sh): the click run and the Rust side's tests are skipped when
# what they run is what passed (scripts/bin-key.py).
# shellcheck source=scripts/gate-lib.sh
. scripts/gate-lib.sh

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
# The build and the type-check read only the UI's own sources, so each is skipped when those are
# what its last pass had: a change to the Rust side alone rebuilt the UI on every run, on the
# clicks' path (2026-11-27: the build stage 36 s at load 152-186, 1.2 s at load 60).
ui_key() {  # <path...> under apps/desktop: the key of those files and of bun
  (cd $app && { bun --version; git ls-files -co --exclude-standard -z -- "$@" | sort -z | xargs -0 -r sha256sum; } | sha256sum | cut -c1-32)
}
# Everything vite reads: the lockfile (bun install is frozen to it), the page, the package, the
# sources, the configs, and the files vite would pick up if they were there (public/, .env*,
# a PostCSS config). The UI reads no environment variable (no import.meta.env, no define).
ui_sources=(bun.lock index.html package.json src svelte.config.js tsconfig.json vite.config.ts public '.env*'
  'postcss.config.*')
build_key=$(ui_key "${ui_sources[@]}")
check_key=$(ui_key "${ui_sources[@]}" e2e)
built=$target/ui-dist
if [ -f "$built/index.html" ] && [ "$(cat "$target/ui-dist.key" 2>/dev/null)" = "$build_key" ]; then
  echo "    the UI's sources are those of its last build: kept ($build_key)"
else
  rm -f "${target:?}/ui-dist.key"
  (cd $app && bun run build --outDir "$built" --emptyOutDir >/dev/null)
  [ -f "$built/index.html" ] || { echo "ui-gate: the UI build wrote no index.html into $built" >&2; exit 1; }
  echo "$build_key" >"$target/ui-dist.key"
fi
mkdir -p $app/dist
rsync -r --checksum --delete "$built/" "$app/dist/"
# Two things run beside the steps after them, each in its own process group: the type-check
# (UI sources only), and the app's Rust side with the simulated devices (clippy, then its tests).
# The click tests need only the built UI and the bridge binary, so they do not wait for the
# type-check. A failing step ends the gate; the groups still running go with it (by their recorded PIDs).
bg_pids=()
trap 'for p in "${bg_pids[@]}"; do kill -TERM -- "-$p" 2>/dev/null || true; done' EXIT
check_log=$target/ui-check.log
check_pid=
if [ "$(cat "$target/ui-check.pass" 2>/dev/null)" = "$check_key" ]; then
  echo "    the type-check's sources are those of its last pass: kept ($check_key)"
else
  rm -f "${target:?}/ui-check.pass"
  setsid bash -c 'cd "$1" && exec bun run check' ui-check "$app" >"$check_log" 2>&1 &
  check_pid=$!
  bg_pids+=("$check_pid")
fi

# The click tests start before the bridge is built: Playwright's start, its workers and Chromium
# (7-8 s at load ~110) overlap the bridge's build and link, and each test's bridge waits for the
# file $ATLAS_UI_BRIDGE_READY, which is written only once this run's bridge is in place. The file
# is removed first, so no test ever runs a bridge from an earlier run.
stage "click tests (they start now and wait for the bridge)"
export PLAYWRIGHT_BROWSERS_PATH=${PLAYWRIGHT_BROWSERS_PATH:-$target/ms-playwright}
# The headless shell of the revision this Playwright runs, not any revision: a directory left by an
# older Playwright (CI restores one in target/) passed a name-only check, nothing was fetched, and
# every click test failed to launch it (2026-11-27). Playwright names the directory it wants and
# writes INSTALLATION_COMPLETE in it last.
plan=$(cd $app && node node_modules/@playwright/test/cli.js install --dry-run --only-shell chromium)
shell_dir=$(sed -n 's/^ *Install location: *\(.*\/chromium_headless_shell-[0-9][0-9]*\)$/\1/p' <<<"$plan" | head -n 1)
[ -n "$shell_dir" ] || { echo "ui-gate: Playwright named no headless Chromium directory (install --dry-run)" >&2; exit 1; }
# Where the browser came from is kept beside it when it is fetched (the source URL, the revision and the
# SHA-256 of the executable), and said on every run.
source_record=$shell_dir.atlas-source
if [ ! -f "$shell_dir/INSTALLATION_COMPLETE" ]; then
  [ "$fetch" -eq 1 ] || { echo "ui-gate: no headless Chromium at $shell_dir" >&2; exit 1; }
  (cd $app && node node_modules/@playwright/test/cli.js install --only-shell chromium)
  url=$(awk -v dir="$shell_dir" '$1 == "Install" && $3 == dir { want = 1; next } want && $1 == "Download" { print $3; exit }' <<<"$plan")
  {
    echo "revision ${shell_dir##*-}"
    echo "url ${url:-unknown}"
    echo "sha256 $(sha256sum "$shell_dir/chrome-headless-shell-linux64/chrome-headless-shell" | cut -d' ' -f1)"
    echo "fetched $(date -u +%FT%TZ)"
  } >"$source_record"
fi
if [ -f "$source_record" ]; then
  echo "    headless Chromium: $(tr '\n' ' ' <"$source_record")"
else
  echo "    headless Chromium: $shell_dir (fetched before its source was recorded)"
fi
mkdir -p "$target/ui-bridge"
export ATLAS_UI_BRIDGE=$target/ui-bridge/atlas-ui-bridge
export ATLAS_UI_BRIDGE_READY=$target/ui-bridge/ready
rm -f "${ATLAS_UI_BRIDGE_READY:?}"
export ATLAS_E2E_OUT=$target/ui-gate
[ -z "$screens" ] || export ATLAS_SCREENS_DIR=$screens
setsid bash -c 'cd "$1" && shift && exec node node_modules/@playwright/test/cli.js test -c e2e/playwright.config.ts "$@"' \
  ui-clicks "$app" "${pw_args[@]+"${pw_args[@]}"}" &
pw_pid=$!
bg_pids+=("$pw_pid")

stage "the UI bridge with the simulated devices"
cargo build --locked -p atlas-app --features sim --bin atlas-ui-bridge
# The click tests run a copy: `cargo test` below rebuilds the package's binaries in target/debug
# while they run. It is moved into place whole, then said to be ready.
cp -f "$target/debug/atlas-ui-bridge" "$ATLAS_UI_BRIDGE.new"
mv -f "$ATLAS_UI_BRIDGE.new" "$ATLAS_UI_BRIDGE"
echo "$SECONDS" >"$ATLAS_UI_BRIDGE_READY"

# The click run is keyed on what it runs: the bridge's loaded sections, the built UI's bytes, the browser's revision,
# and the data key (the specs and every other file but the Rust sources). A run that already passed with this key is
# stopped (it started before the bridge was built) and not run again. A run with arguments or pictures is never kept.
clicks_key=""
if [ ${#pw_args[@]} -eq 0 ] && [ -z "$screens" ]; then
  if data_key=$(gate_data_key) \
    && clicks_key=$(python3 -I scripts/bin-key.py key "clicks ${shell_dir##*/}" "$data_key" --dir "$built" -- "$ATLAS_UI_BRIDGE"); then
    if gate_known clicks "$clicks_key"; then
      kill -TERM -- "-$pw_pid" 2>/dev/null || true
      wait "$pw_pid" 2>/dev/null || true
      pw_pid=""
      echo "    the click run's bridge, UI and specs are those of a passing run ($clicks_key): stopped, not run"
    fi
  else
    clicks_key=""
  fi
fi

stage "the app's Rust side with the simulated devices (beside the click tests; log $target/ui-rust.log)"
rust_log=$target/ui-rust.log
# Its tests are keyed as the tests lane keys its programs: what the test programs load, what the package's programs
# load (the bridge test runs atlas-ui-bridge), the build scripts' output and the data key. Clippy always runs.
sim_tests() {
  local built programs=() outs=() args=() data key d
  built=$(cargo test --locked -p atlas-app --features sim --no-run --message-format json) || return
  mapfile -t programs < <(python3 -I -c 'import json, sys
for line in sys.stdin:
    m = json.loads(line)
    if m.get("reason") == "compiler-artifact" and m.get("executable"):
        print(m["executable"])' <<<"$built" | sort -u)
  mapfile -t outs < <(python3 -I -c 'import json, sys
for line in sys.stdin:
    m = json.loads(line)
    if m.get("reason") == "build-script-executed" and m.get("out_dir"):
        print(m["out_dir"])' <<<"$built" | sort -u)
  for d in "${outs[@]}"; do args+=(--dir "$d"); done
  data=$(gate_data_key) || return
  if [ ${#programs[@]} -gt 0 ] && key=$(python3 -I scripts/bin-key.py key "sim tests" "$data" "${args[@]}" -- "${programs[@]}"); then
    if gate_known sim-tests "$key"; then
      echo "    the sim tests' ${#programs[@]} programs load what they loaded when they passed ($key): not run"
      return 0
    fi
  else
    key=""
  fi
  cargo test --locked -p atlas-app --features sim || return
  [ -z "$key" ] || gate_mark sim-tests "$key"
}
export -f sim_tests
setsid bash -c '. scripts/gate-lib.sh && cargo clippy --locked -p atlas-app --features sim --all-targets -- -D warnings && sim_tests' \
  ui-rust >"$rust_log" 2>&1 &
rust_pid=$!
bg_pids+=("$rust_pid")

if [ -n "$pw_pid" ]; then
  stage "waiting for the click tests"
  if ! wait "$pw_pid"; then
    echo "ui-gate: the click tests failed (above)" >&2
    exit 1
  fi
  [ -z "$clicks_key" ] || gate_mark clicks "$clicks_key"
fi

# The type-check ran beside the bridge's build and the clicks: they need only the built UI, not
# its types (8 s of svelte-check no longer waits in front of them).
if [ -n "$check_pid" ]; then
  stage "waiting for the type-check"
  if ! wait "$check_pid"; then
    cat "$check_log" >&2
    echo "ui-gate: the UI does not type-check (svelte-check, above)" >&2
    exit 1
  fi
  tail -n 3 "$check_log"
  echo "$check_key" >"$target/ui-check.pass"
fi

stage "waiting for the Rust side"
if ! wait "$rust_pid"; then
  cat "$rust_log" >&2
  echo "ui-gate: the app's Rust side with the simulated devices failed (clippy or its tests, above)" >&2
  exit 1
fi
grep -E '^test result:|Finished' "$rust_log" | tail -n 4
trap - EXIT
echo "ui gate passed (+$SECONDS s)"
