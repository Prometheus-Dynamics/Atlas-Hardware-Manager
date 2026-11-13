#!/usr/bin/env bash
# Checks every device description under devices/ with Gaia `next`.
#
#   scripts/check-devices.sh               check devices/, then prove the checker still catches
#                                          a broken file (the fixtures; the gate runs this)
#   scripts/check-devices.sh --root DIR    check another tree laid out like devices/ (no self-check)
#   scripts/check-devices.sh --self-test   the verbose form: good fixture must pass, broken must
#                                          fail, and the f8 fixture is printed
#
# The Gaia commit is read from devices/gaia.lock. The gaia binary is $GAIA_BIN when set (used as
# given: gaia has no version stamp, so its commit is not checked), else a build from that commit
# cached under ${ATLAS_GAIA_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/atlas/gaia}/<commit>/.
# The recipes come from the same Gaia checkout. Each product is checked with `gaia validate` when
# the binary has that command, else `gaia plan`.
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
self=$repo/scripts/check-devices.sh
root=$repo/devices
mode=check
root_given=0

die() { echo "check-devices: $*" >&2; exit 1; }

while [[ $# -gt 0 ]]; do
  case $1 in
    --root) [[ $# -ge 2 ]] || die "--root needs a directory"; root=$2; root_given=1; shift 2 ;;
    --self-test) mode=self-test; shift ;;
    -h|--help) sed -n '2,12p' "$self"; exit 0 ;;
    *) die "unknown argument: $1" ;;
  esac
done

if [[ $mode == self-test ]]; then
  fixtures=$repo/devices/tests/fixtures
  status=0
  echo "== good fixture: must pass"
  "$self" --root "$fixtures/good" || { echo "check-devices: self-test: good fixture did not pass" >&2; status=1; }
  echo "== broken fixture: must fail"
  if "$self" --root "$fixtures/broken"; then
    echo "check-devices: self-test: broken fixture passed, but it must fail" >&2; status=1
  else
    echo "check-devices: self-test: broken fixture failed as it must"
  fi
  echo "== f8 fixture: reported only (Gaia F8 is not on next yet)"
  "$self" --root "$fixtures/f8" || true
  exit $status
fi

# ---- the Gaia pin -------------------------------------------------------------

lock=$repo/devices/gaia.lock
[[ -f $lock ]] || die "missing $lock"
gaia_url=$(sed -n 's/^url = "\(.*\)"$/\1/p' "$lock")
gaia_commit=$(sed -n 's/^commit = "\(.*\)"$/\1/p' "$lock")
[[ -n $gaia_url ]] || die "$lock has no url"
[[ $gaia_commit =~ ^[0-9a-f]{40}$ ]] || die "$lock: commit must be a full 40-digit hex commit"

cache=${ATLAS_GAIA_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/atlas/gaia}
dir=$cache/$gaia_commit

# Clones the locked commit into $dir/src unless that checkout is already there.
ensure_src() {
  if [[ -f $dir/src/.built-from && $(cat "$dir/src/.built-from") == "$gaia_commit" ]]; then
    return 0
  fi
  command -v git >/dev/null || die "git is needed to fetch Gaia"
  local work=$dir.partial
  rm -rf "$work"; mkdir -p "$work"
  echo "gaia: cloning $gaia_url" >&2
  git clone -q --no-checkout "$gaia_url" "$work/src" \
    || die "cannot clone $gaia_url (no network, or the repository is private and no token is set)"
  git -C "$work/src" checkout -q --detach "$gaia_commit" \
    || die "commit $gaia_commit is not in $gaia_url"
  echo "$gaia_commit" > "$work/src/.built-from"
  mkdir -p "$dir"
  rm -rf "$dir/src"; mv "$work/src" "$dir/src"; rm -rf "$work"
}

# Builds gaia from $dir/src into $dir/bin/gaia unless that build is already there.
ensure_bin() {
  if [[ -x $dir/bin/gaia && -f $dir/bin/.built-from && $(cat "$dir/bin/.built-from") == "$gaia_commit" ]]; then
    echo "gaia: cached build $dir/bin/gaia" >&2
    return 0
  fi
  command -v cargo >/dev/null || die "cargo is needed to build Gaia (install Rust with rustup)"
  echo "gaia: building gaia at $gaia_commit (slow the first time)" >&2
  (cd "$dir/src" && CARGO_TARGET_DIR="$dir/target" cargo build --release -p gaia --bin gaia) >&2 \
    || die "building gaia at $gaia_commit failed"
  mkdir -p "$dir/bin"
  cp "$dir/target/release/gaia" "$dir/bin/gaia"
  rm -rf "$dir/target"
  echo "$gaia_commit" > "$dir/bin/.built-from"
}

ensure_src
if [[ -n ${GAIA_BIN:-} ]]; then
  gaia=$GAIA_BIN
  [[ -x $gaia ]] || die "GAIA_BIN=$gaia is not an executable"
  echo "gaia: using GAIA_BIN=$gaia; its commit is not checked (gaia has no version stamp)" >&2
else
  ensure_bin
  gaia=$dir/bin/gaia
fi
recipes=$dir/src/recipes

# python3 is the tool the python component builder names; gaia plan needs it given.
python3_path=$(command -v python3) || die "python3 is needed (the python builder's tool)"

# ---- which command: validate when gaia has it, else plan ----------------------

if "$gaia" --help 2>&1 | grep -Eq '^ +gaia validate '; then
  verb=validate
else
  verb=plan
fi
echo "gaia: $("$gaia" --version 2>&1 | head -1), command: $verb, locked commit $gaia_commit" >&2

# ---- the tree ------------------------------------------------------------------

[[ -d $root ]] || die "no such directory: $root"
cd "$root"
fail=0
bad() { echo "check-devices: FAIL $*" >&2; fail=1; }

# Each kind is one directory per item, holding its description file.
for kind in families:family.toml boards:board.toml components:component.toml products:product.toml; do
  dirname_=${kind%%:*}; desc=${kind#*:}
  [[ -d $dirname_ ]] || continue
  while IFS= read -r entry; do
    [[ $entry == .gitkeep ]] && continue
    if [[ ! -d $dirname_/$entry ]]; then
      bad "$dirname_/$entry: a file here; each $dirname_ item is its own directory (see devices/README.md)"
    elif [[ ! -f $dirname_/$entry/$desc ]]; then
      bad "$dirname_/$entry: no $desc in it"
    fi
  done < <(ls -A "$dirname_")
done

count() { [[ -d $1 ]] || { echo 0; return; }; find "$1" -mindepth 2 -maxdepth 2 -name "$2" | wc -l; }
nf=$(count families family.toml); nb=$(count boards board.toml)
nc=$(count components component.toml); np=$(count products product.toml)
echo "check-devices: checking $nf families, $nb boards, $nc components, $np products under $root with gaia $verb"

# Gaia reads a file only through a product or a board's extends, so a family, board or
# component that nothing names would never be checked. Every one must be named.
used=$(mktemp)
errs=$(mktemp)
outs=$(mktemp)
for f in $(find boards products -mindepth 2 -maxdepth 2 \( -name board.toml -o -name product.toml \) 2>/dev/null | sort); do
  d=$(dirname "$f")
  { grep -ho '"[^"]*\.toml"' "$f" || true; } | tr -d '"' | while IFS= read -r ref; do
    realpath -m "$d/$ref"
  done
done >> "$used"
for f in $(find families boards components -mindepth 2 -maxdepth 2 -name '*.toml' 2>/dev/null | sort); do
  if ! grep -qxF "$(realpath -m "$f")" "$used"; then
    bad "$f: no product or board names it, so gaia never reads it"
  fi
done

# Every product is locked in a temp file, then validated or planned with that lock. Gaia writes a
# tree path relative to the product's directory only when it lies under it; components live in
# devices/components/, so a committed product lock would hold this checkout's absolute paths and
# fail anywhere else. So a product lock is never committed here; if one exists it must still match.
locks=$(mktemp -d)
trap 'rm -rf "$locks" "$used" "$errs" "$outs"' EXIT
for product in $(find products -mindepth 2 -maxdepth 2 -name product.toml 2>/dev/null | sort); do
  pdir=$(dirname "$product")
  if [[ -f $pdir/gaia.lock ]] && ! "$gaia" lock "$product" --check >"$errs" 2>&1; then
    bad "$product: committed gaia.lock does not match (delete it: the check locks products itself):"
    sed 's/^/  /' "$errs" >&2
    continue
  fi
  lockfile=$locks/$(echo "$product" | tr '/' '_').lock
  if ! "$gaia" lock "$product" --lock "$lockfile" >"$errs" 2>&1; then
    bad "$product: gaia lock failed:"
    sed 's/^/  /' "$errs" >&2
    continue
  fi
  if "$gaia" "$verb" "$product" --lock "$lockfile" --recipes "$recipes" --tool "python=$python3_path" >"$outs" 2>&1; then
    echo "check-devices: ok $product (gaia $verb)"
  else
    bad "$product: gaia $verb failed:"
    sed 's/^/  /' "$outs" >&2
  fi
done

if [[ $fail -ne 0 ]]; then
  echo "check-devices: FAILED" >&2
  exit 1
fi
echo "check-devices: passed ($nf families, $nb boards, $nc components, $np products)"

# The default run also proves the checker still catches a broken file: the good fixture must
# pass and the broken one must fail. Not run for --root, which names a tree to check alone.
if [[ $root_given -eq 0 ]]; then
  fixtures=$repo/devices/tests/fixtures
  out=$(mktemp)
  if ! "$self" --root "$fixtures/good" >"$out" 2>&1; then
    cat "$out" >&2; die "self-check: the good fixture no longer passes"
  fi
  if "$self" --root "$fixtures/broken" >"$out" 2>&1; then
    die "self-check: the broken fixture passed, so the checker no longer catches a broken file"
  fi
  rm -f "$out"
  echo "check-devices: self-check passed (good fixture passes, broken fixture fails)"
fi
