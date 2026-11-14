#!/usr/bin/env bash
# The config-only build of EVERY product under devices/products (PR B32; Gaia #120). Gaia resolves each image's
# Buildroot configuration (every fragment line must take, as written) and stops there, with nothing compiled:
#   gaia build <product.toml> --recipes <recipes> --input bench-key=<public key> --config-only
# A cold store fetches each family's kernel source first (Gaia measured about 100 s and 2.3 GiB per family), so this
# needs the network once; a warm store runs offline. GAIA_STORE is the content store: $GAIA_STORE, else
# ${XDG_CACHE_HOME:-$HOME/.cache}/atlas/gaia-store (CI caches it).
#
# The Gaia binary is $GAIA_BIN, else the one scripts/check-devices.sh built into its cache, at the commit
# devices/gaia.lock names. Run check-devices.sh first when there is no binary: it also fetches Gaia's recipes.
#
#   scripts/check-config-only.sh     the gate runs this (scripts/gate.sh)
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
die() { echo "check-config-only: $*" >&2; exit 1; }

lock=$repo/devices/gaia.lock
commit=$(sed -n 's/^commit = "\(.*\)"$/\1/p' "$lock")
[[ $commit =~ ^[0-9a-f]{40}$ ]] || die "$lock has no full commit"
cache=${ATLAS_GAIA_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/atlas/gaia}
dir=$cache/$commit
gaia=${GAIA_BIN:-$dir/bin/gaia}
[[ -x $gaia ]] || die "no gaia binary at $gaia: run scripts/check-devices.sh first (it builds Gaia at the pin), or set GAIA_BIN"
gaia_line=$("$gaia" --version 2>&1 | head -1 || true)
[[ $gaia_line == *"($commit)"* ]] || die "$gaia prints '$gaia_line', not the commit devices/gaia.lock names ($commit)"
recipes=$dir/src/recipes
[[ -d $recipes ]] || die "no recipes at $recipes: run scripts/check-devices.sh first (it fetches Gaia's checkout)"

export GAIA_STORE=${GAIA_STORE:-${XDG_CACHE_HOME:-$HOME/.cache}/atlas/gaia-store}
mkdir -p "$GAIA_STORE"
if [[ -z $(ls -A "$GAIA_STORE" 2>/dev/null) ]]; then
  echo "check-config-only: NOTE the store $GAIA_STORE is cold: this run FETCHES each family's kernel source (about 100 s and 2.3 GiB per family), so it needs the network"
else
  echo "check-config-only: store $GAIA_STORE is warm: no fetch is expected"
fi

# A throwaway public key for `bench-key`: made here, its private half deleted at once, never committed.
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
command -v ssh-keygen >/dev/null || die "ssh-keygen is needed for the throwaway bench key"
ssh-keygen -q -t ed25519 -N '' -C atlas-config-only-throwaway -f "$scratch/key"
rm -f "$scratch/key"
pub=$scratch/key.pub

# The tools a recipe names must be on PATH or given (the same stand-ins check-devices.sh uses for planning).
tool_args=(--tool "python=$(command -v python3)")
if ! command -v veritysetup >/dev/null 2>&1; then
  printf '#!/bin/sh\necho "veritysetup stand-in: config only" >&2\nexit 1\n' > "$scratch/veritysetup"
  chmod +x "$scratch/veritysetup"; tool_args+=(--tool "veritysetup=$scratch/veritysetup")
fi
if ! command -v bwrap >/dev/null 2>&1; then
  printf '#!/bin/sh\necho "bwrap stand-in: config only" >&2\nexit 1\n' > "$scratch/bwrap"
  chmod +x "$scratch/bwrap"; tool_args+=(--tool "bwrap=$scratch/bwrap")
fi

fail=0
for product in $(find "$repo/devices/products" -mindepth 2 -maxdepth 2 -name product.toml | sort); do
  pdir=$(dirname "$product"); pname=$(basename "$pdir")
  input_args=()
  if grep -rqs 'input = "bench-key"' "$pdir"; then input_args=(--input "bench-key=$pub"); fi
  out=$scratch/$pname.out
  start=$SECONDS
  if "$gaia" build "$product" --recipes "$recipes" ${input_args[@]+"${input_args[@]}"} "${tool_args[@]}" --config-only >"$out" 2>&1; then
    echo "check-config-only: ok products/$pname: configuration resolved, nothing compiled ($((SECONDS - start)) s)"
  else
    fail=1
    echo "check-config-only: FAIL products/$pname: gaia build --config-only failed ($((SECONDS - start)) s):" >&2
    sed 's/^/  /' "$out" >&2
  fi
done
[[ $fail -eq 0 ]] || { echo "check-config-only: FAILED" >&2; exit 1; }
echo "check-config-only: passed (every product's configuration resolves; nothing compiled)"
