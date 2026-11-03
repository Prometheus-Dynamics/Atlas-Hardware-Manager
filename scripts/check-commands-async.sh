#!/usr/bin/env bash
# Every Tauri command runs on the async runtime, never on the main thread:
# a plain `#[tauri::command] fn` runs on the thread that draws the window,
# so one slow lock or disk read freezes Atlas. Use `async fn` or
# `#[tauri::command(async)]`.
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bad=$(grep -rn -A1 --include='*.rs' '#\[tauri::command\]' "$root_dir/apps/desktop/src-tauri/src" \
    | grep -E -- '-[0-9]+-\s*pub(\(crate\))? fn ' || true)
if [[ -n "$bad" ]]; then
    echo "Tauri commands on the main thread (make them async or #[tauri::command(async)]):"
    echo "$bad" | sed "s#$root_dir/##"
    exit 1
fi
