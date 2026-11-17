#!/usr/bin/env bash
# A REAL build of one product under devices/products with Gaia at the commit devices/gaia.lock names: the OS, the
# kernel, every component and the image. This compiles (a kernel and a Buildroot system), so it is never part of the
# gate; the gate stops at the resolved configuration (scripts/check-config-only.sh). It is run by hand, here or
# by the manual CI job (`build-product` in .github/workflows/next.yml).
#
#   scripts/build-product.sh <product> [--plan] [--bench-key FILE.pub] [--until STEP] [--jobs N] [--tool NAME=PATH]... [--out DIR]
#
#   <product>     a directory name under devices/products (raze-base, rock-5c-bringup, ...)
#   --plan        plan only: print the version and slot, run `gaia plan` with them, build nothing
#   The version is the Atlas dev scheme, <release>-dev.<count>+<commit> (dev's scripts/version.sh, via
#   scripts/version-rule.sh atlas_version); a slot-boot product's slot is <product>-<version> (slot_id_for). Gaia takes
#   both as --setting (IMAGE.version, IMAGE.slot): the product's product.toml sets neither. Printed before the build.
#   --pc-key      the PC's public key for a product that names `{ input = "pc-key" }`: its comment is removed first
#                 (scripts/pc-key-strip.sh), so only the ssh-ed25519 part reaches the image. Required for such a product.
#   --bench-key   the public key a bring-up product installs for its bench login (`{ input = "bench-key" }`).
#                 A product that names the input refuses to build without it: there is no throwaway key here,
#                 because an image nobody can log into proves nothing on a board.
#   --until STEP  stop after this step of the plan (a node name from `gaia plan`), for a partial build
#   --jobs N      Gaia's job count (default: Gaia's own)
#   --tool N=PATH a tool given to Gaia by name (repeatable), as `gaia build --tool`
#   --out DIR     copy what each output holds into DIR (the image files), with their sizes and SHA-256
#
# GAIA_BIN is the binary, else the one scripts/check-devices.sh built into its cache. GAIA_STORE is the content
# store, else ${XDG_CACHE_HOME:-$HOME/.cache}/atlas/gaia-store: a full build adds several GiB to it.
# The slot recipes need veritysetup, mkfs.erofs and the disk tools; a Rust component needs bwrap. A missing tool is a
# failure here, not a stand-in: this is the build, not the plan.
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
die() { echo "build-product: $*" >&2; exit 1; }
# atlas_version, slot_id_for, slot_settings_args: scripts/version-rule.sh, the one rule's one place.
source "$repo/scripts/version-rule.sh"

pname="" key="" pckey="" until_step="" jobs="" out="" plan_only=0
tools=()
while [[ $# -gt 0 ]]; do
  case $1 in
    --plan) plan_only=1; shift ;;
    --bench-key) key=${2:?--bench-key needs a file}; shift 2 ;;
    --pc-key) pckey=${2:?--pc-key needs a file}; shift 2 ;;
    --until) until_step=${2:?--until needs a step}; shift 2 ;;
    --jobs) jobs=${2:?--jobs needs a count}; shift 2 ;;
    --tool) tools+=(--tool "${2:?--tool needs NAME=PATH}"); shift 2 ;;
    --out) out=${2:?--out needs a directory}; shift 2 ;;
    -*) die "unknown flag $1" ;;
    *) [[ -z $pname ]] || die "one product only (got $pname and $1)"; pname=$1; shift ;;
  esac
done
[[ -n $pname ]] || die "usage: scripts/build-product.sh <product> [--plan] [--bench-key FILE.pub] [--until STEP] [--jobs N] [--out DIR]"
product=$repo/devices/products/$pname/product.toml
[[ -f $product ]] || die "no product $pname (have: $(ls "$repo/devices/products" | tr '\n' ' '))"

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

args=("$product" --recipes "$recipes" --tool "python=$(command -v python3)")
if grep -rqs 'input = "bench-key"' "$(dirname "$product")"; then
  [[ -n $key ]] || die "$pname installs a bench key: give --bench-key FILE.pub (a public key; it goes into the image)"
  [[ -f $key ]] || die "no such key file: $key"
  if grep -q 'PRIVATE KEY' "$key"; then die "$key is a PRIVATE key: give the .pub file"; fi
  args+=(--input "bench-key=$(realpath "$key")")
elif [[ -n $key ]]; then
  die "$pname names no bench key, so --bench-key has nothing to go into"
fi
if grep -rqs 'input = "pc-key"' "$(dirname "$product")"; then
  [[ -n $pckey ]] || die "$pname installs the PC's key: give --pc-key FILE.pub"
  pcdir=$(mktemp -d)
  trap 'rm -rf "$pcdir"' EXIT
  "$repo/scripts/pc-key-strip.sh" "$pckey" "$pcdir/pc-key.pub" || die "--pc-key $pckey refused (see above)"
  args+=(--input "pc-key=$pcdir/pc-key.pub")
elif [[ -n $pckey ]]; then
  die "$pname names no pc-key input, so --pc-key has nothing to go into"
fi
args+=(${tools[@]+"${tools[@]}"})
[[ -z $jobs ]] || args+=(--jobs "$jobs")
[[ -z $until_step ]] || args+=(--until "$until_step")

# The version (the Atlas dev scheme of this checkout's HEAD; a tracked change is refused) and, for a slot-boot
# product, the slot: both as Gaia --settings, printed before anything runs.
version=$(atlas_version "$repo") || die "no version for the build (see above)"
setting=$(slot_settings_args "$product" "$version") || die "$pname's slot settings could not be read (see above)"
setargs=()
while IFS= read -r token; do
  [[ -n $token ]] && setargs+=("$token")
done <<<"$setting"
echo "build-product: version $version (the dev scheme of $(git -C "$repo" rev-parse --short=8 HEAD) at $repo)"
if [[ ${#setargs[@]} -gt 0 ]]; then
  for ((i = 0; i < ${#setargs[@]}; i += 2)); do
    echo "build-product: ${setargs[i]} ${setargs[i + 1]}"
  done
  args+=("${setargs[@]}")
else
  echo "build-product: $pname's recipe takes no version or slot: none given"
fi

if [[ $plan_only -eq 1 ]]; then
  echo "build-product: plan of $pname with $gaia_line (--plan: nothing built)"
  "$gaia" plan "${args[@]}" || die "gaia plan failed for $pname (Gaia's words are above)"
  exit 0
fi

echo "build-product: $pname with $gaia_line, store $GAIA_STORE${until_step:+, until $until_step}"
start=$SECONDS
"$gaia" build "${args[@]}" || die "FAILED products/$pname after $((SECONDS - start)) s (Gaia's words are above)"
echo "build-product: built products/$pname ($((SECONDS - start)) s)"

# What was made: each output of the plan, as Gaia shows it. A partial build has no outputs to show.
[[ -z $until_step ]] || { echo "build-product: partial build (--until $until_step): no outputs are listed"; exit 0; }
outputs=$("$gaia" plan "${args[@]}" 2>/dev/null | sed -n '/^outputs:$/,$p' | sed -n 's/^  \([^ ]*\)  .*/\1/p')
[[ -n $outputs ]] || die "the plan of $pname names no outputs"
[[ -z $out ]] || mkdir -p "$out"
for o in $outputs; do
  echo "build-product: output $o"
  shown=$("$gaia" show "$o") || die "gaia show $o failed"
  sed 's/^/  /' <<<"$shown"
  [[ -n $out ]] || continue
  # the store paths the output holds: copied out by name, with size and digest
  # A store object is named by its digest: the copy is named by the output (`raze-base.slot`), with a number
  # only when one output holds several objects.
  n=0
  while IFS= read -r path; do
    [[ -e $path ]] || continue
    n=$((n + 1))
    if [[ $n -eq 1 ]]; then dest=$out/$o; else dest=$out/$o.$n; fi
    cp -a --reflink=auto "$path" "$dest"
  done < <(grep -oE "$GAIA_STORE/[^ ]+" <<<"$shown" | sort -u)
  [[ $n -gt 0 ]] || die "gaia show $o names no store object under $GAIA_STORE: nothing to copy"
done
if [[ -n $out ]]; then
  echo "build-product: files in $out"
  (cd "$out" && find . -type f | sort | while IFS= read -r f; do
    printf '  %12d  %s  %s\n' "$(stat -c %s "$f")" "$(sha256sum "$f" | cut -d' ' -f1)" "${f#./}"
  done)
fi
