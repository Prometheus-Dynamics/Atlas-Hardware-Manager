#!/usr/bin/env bash
# What must build on Windows and macOS (PR G9). The data is scripts/orion-bound.txt; the logic is
# scripts/check-portable.py. Bash only runs cargo.
#
#   scripts/check-portable.sh --classify       check the data file against cargo metadata (any host;
#                                              the gate runs this)
#   scripts/check-portable.sh <target-triple>  classify, then `cargo check --locked` every portable
#                                              crate for <target-triple>, natively: lib, bins and
#                                              tests (compiled, not run). CI runs this on
#                                              windows-latest and macos-latest.
#
# Every run prints each crate it skips and why. Exit status is 0 only when every check passes.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

if [ $# -ne 1 ]; then
  echo "usage: $0 --classify | <target-triple>" >&2
  exit 2
fi
mode=$1

# Windows runners may have only `python` on PATH.
PY=python3
if ! python3 -c 'pass' > /dev/null 2>&1; then
  PY=python
fi

LIST=scripts/orion-bound.txt
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

cargo metadata --locked --format-version 1 > "$tmp/metadata.json"

if [ "$mode" = "--classify" ]; then
  "$PY" -I scripts/check-portable.py classify "$LIST" "$tmp/metadata.json"
  exit 0
fi

triple=$mode
args=()
while IFS= read -r crate; do
  args+=(-p "$crate")
done < <("$PY" -I scripts/check-portable.py plan "$LIST" "$tmp/metadata.json" "$triple")
if [ "${#args[@]}" -eq 0 ]; then
  echo "check-portable: no portable crates to check for $triple" >&2
  exit 1
fi

echo "==> cargo check --locked --keep-going --target $triple --lib --bins --tests (${#args[@]} crates, $(( ${#args[@]} / 2 )) -p)"
set +e
cargo check --locked --keep-going --target "$triple" --lib --bins --tests "${args[@]}" > "$tmp/cargo.log" 2>&1
rc=$?
set -e
cat "$tmp/cargo.log"
echo "==> cargo exit status $rc"

"$PY" -I scripts/check-portable.py verdict "$LIST" "$tmp/metadata.json" "$triple" "$tmp/cargo.log" "$rc"
