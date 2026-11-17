#!/usr/bin/env bash
# The exec bit of the scripts a fragment names. Gaia stores a fragment-named `@here@/<file>` with mode 0644 at the pinned
# rev 9efa797f (so a script loses its exec bit and Buildroot cannot run it); the fix is in a later Gaia rev. This check
# asks Gaia for the stored trees of each product and fails when an executable script of ours is stored without it.
#
#   scripts/check-fragment-modes.sh --self-test   a stored 0644 copy is refused; a 0755 one passes; no copy is refused
#                                                 (no Gaia needed: the gate runs this)
#   scripts/check-fragment-modes.sh --real        Gaia's stored trees for every product (needs the Gaia binary at the
#                                                 pin; this is NOT in the gate until the commit that pins the fixed rev)
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
die() { echo "check-fragment-modes: $*" >&2; exit 1; }

# check_modes <store> <file>...: each file must be stored, byte for byte, with the exec bit when it is executable here.
check_modes() {
  local store=$1 f base copies copy ok=0 bad=0
  shift
  for f in "$@"; do
    base=$(basename "$f")
    [ -x "$f" ] || { echo "check-fragment-modes: ok $f: not executable here, nothing to check"; continue; }
    copies=0
    while IFS= read -r copy; do
      cmp -s "$copy" "$f" || continue
      copies=$((copies + 1))
      if [ -x "$copy" ]; then
        echo "check-fragment-modes: ok $f: stored executable ($copy)"
      else
        echo "check-fragment-modes: FAIL $f: stored $(stat -c %a "$copy") at $copy (executable in the tree): Gaia drops the exec bit of a fragment-named script" >&2
        bad=$((bad + 1))
      fi
    done < <(find "$store/objects/tree" -mindepth 3 -maxdepth 3 -type f -name "$base" 2>/dev/null)
    if [ "$copies" -eq 0 ]; then
      echo "check-fragment-modes: FAIL $f: no stored copy with the same bytes under $store/objects/tree" >&2
      bad=$((bad + 1))
    fi
  done
  [ "$bad" -eq 0 ]
}

if [ "${1:-}" = --self-test ]; then
  t=$(mktemp -d "${TMPDIR:-/tmp}/fm-XXXXXX"); trap 'rm -rf "$t"' EXIT
  mkdir -p "$t/ours" "$t/store/objects/tree/ab/cdef"
  printf '#!/bin/sh\nexit 0\n' > "$t/ours/post-build.sh"; chmod 0755 "$t/ours/post-build.sh"
  cp "$t/ours/post-build.sh" "$t/store/objects/tree/ab/cdef/post-build.sh"; chmod 0644 "$t/store/objects/tree/ab/cdef/post-build.sh"
  if check_modes "$t/store" "$t/ours/post-build.sh" >/dev/null 2>&1; then die "self-test: a stored 0644 copy was accepted"; fi
  chmod 0755 "$t/store/objects/tree/ab/cdef/post-build.sh"
  check_modes "$t/store" "$t/ours/post-build.sh" >/dev/null || die "self-test: a stored 0755 copy was refused"
  rm -f "$t/store/objects/tree/ab/cdef/post-build.sh"
  if check_modes "$t/store" "$t/ours/post-build.sh" >/dev/null 2>&1; then die "self-test: a file with no stored copy was accepted"; fi
  echo "check-fragment-modes: self-test passed (a stored 0644 copy is refused; a 0755 copy passes; no stored copy is refused)"
  exit 0
fi

[ "${1:-}" = --real ] || die "usage: check-fragment-modes.sh --self-test | --real"
source "$repo/scripts/version-rule.sh"
commit=$(sed -n 's/^commit = "\(.*\)"$/\1/p' "$repo/devices/gaia.lock")
cache=${ATLAS_GAIA_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/atlas/gaia}
gaia=${GAIA_BIN:-$cache/$commit/bin/gaia}
[ -x "$gaia" ] || die "no gaia binary at $gaia (set GAIA_BIN)"
recipes=$cache/$commit/src/recipes
[ -d "$recipes" ] || die "no recipes at $recipes"
export GAIA_STORE=${GAIA_STORE:-${XDG_CACHE_HOME:-$HOME/.cache}/atlas/gaia-store}
mkdir -p "$GAIA_STORE"
scratch=$(mktemp -d "${TMPDIR:-/tmp}/fm-real-XXXXXX"); trap 'rm -rf "$scratch"' EXIT
ssh-keygen -q -t ed25519 -N '' -C atlas-fragment-modes-throwaway -f "$scratch/key" && rm -f "$scratch/key"
fail=0
while IFS= read -r product; do
  pdir=$(dirname "$product"); pname=$(basename "$pdir")
  args=(--until gaia.source.input)
  grep -rqs 'input = "bench-key"' "$pdir" && args+=(--input "bench-key=$scratch/key.pub")
  mapfile -t sa < <(slot_settings_args "$product" "$CHECK_VERSION")
  [ ${#sa[@]} -eq 0 ] || args+=("${sa[@]}")
  if ! "$gaia" build "$product" --recipes "$recipes" "${args[@]}" --tool "python=$(command -v python3)" >"$scratch/$pname.log" 2>&1; then
    echo "check-fragment-modes: FAIL products/$pname: gaia build --until gaia.source.input failed: $(tail -1 "$scratch/$pname.log")" >&2
    fail=1; continue
  fi
  mapfile -t named < <(find "$pdir" -name '*.fragment' -print0 | xargs -0 -r sed -n 's/^BR2_[A-Z_]*SCRIPT="@here@\/\([^"]*\)"/\1/p' | sort -u)
  files=()
  for n in "${named[@]}"; do
    f=$(find "$pdir" -path "*/$n" | head -1)
    [ -n "$f" ] && files+=("$f")
  done
  if ! check_modes "$GAIA_STORE" "${files[@]}"; then fail=1; fi
done < <(find "$repo/devices/products" -mindepth 2 -maxdepth 2 -name product.toml | sort)
[ "$fail" -eq 0 ] || die "FAILED: the stored trees do not keep the exec bit (Gaia $commit)"
echo "check-fragment-modes: passed (every fragment-named executable is stored executable)"
