#!/usr/bin/env bash
# The UI gate (apps/desktop/e2e/ui-gate.spec.ts) in the Playwright image CI
# runs it in, so screenshots compare against baselines made the same way.
#
#   scripts/ui-gate.sh             build the UI, run the gate
#   scripts/ui-gate.sh --update    the same, rewriting e2e/screens: after a
#                                  change to how a page looks that you meant
#
# The report lands in target/ui-gate-report (index.html), the diffs and
# traces of what failed in target/ui-gate.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
image=mcr.microsoft.com/playwright:v1.62.1-noble
engine=$(command -v podman || command -v docker)
update=""
[ "${1:-}" = --update ] && update=--update-snapshots=all

(cd "$root/apps/desktop" && bun install --frozen-lockfile >/dev/null && bun run build >/dev/null)
"$engine" run --rm --ipc=host -e CI=true -v "$root:/src" -w /src/apps/desktop "$image" \
  node node_modules/@playwright/test/cli.js test -c e2e/playwright.config.ts $update
