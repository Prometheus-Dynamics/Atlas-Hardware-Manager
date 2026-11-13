#!/usr/bin/env bash
# Checks every device description under devices/ with Gaia `next`.
#
#   scripts/check-devices.sh               check devices/, then prove the checker still catches
#                                          a broken file (the fixtures; the gate runs this)
#   scripts/check-devices.sh --root DIR    check another tree laid out like devices/ (no self-check)
#   scripts/check-devices.sh --write-locks regenerate every product's committed gaia.lock, then check
#                                          (with --root DIR, the same for that tree)
#   scripts/check-devices.sh --self-test   the verbose form: the good fixtures must pass, broken must
#                                          fail
#
# The Gaia commit is read from devices/gaia.lock. The gaia binary is $GAIA_BIN when set (used as
# given: gaia has no version stamp, so its commit is not checked), else a build from that commit
# cached under ${ATLAS_GAIA_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/atlas/gaia}/<commit>/.
# The recipes come from the same Gaia checkout. With a gaia that has `gaia validate`, every family,
# board, component and product file is validated, and every product is also planned. With one that
# does not, every product is planned and nothing else is checked. Each product's gaia.lock, committed
# beside its product.toml, must match what `gaia lock` writes; `--write-locks` writes them.
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
self=$repo/scripts/check-devices.sh
root=$repo/devices
mode=check
root_given=0
write_locks=0

die() { echo "check-devices: $*" >&2; exit 1; }

while [[ $# -gt 0 ]]; do
  case $1 in
    --root) [[ $# -ge 2 ]] || die "--root needs a directory"; root=$2; root_given=1; shift 2 ;;
    --self-test) mode=self-test; shift ;;
    --write-locks) write_locks=1; shift ;;
    -h|--help) sed -n '2,15p' "$self"; exit 0 ;;
    *) die "unknown argument: $1" ;;
  esac
done

# The broken fixture must fail, given its run's exit status ($2) and output ($1). Under gaia
# validate the output must also name the broken key, so the check shows it caught the bad file.
broken_failed_as_it_must() {
  local out=$1 rc=$2
  [[ $rc -ne 0 ]] || return 1
  if grep -q 'command: validate' "$out"; then
    grep -q 'target\.kernal: unknown key' "$out" || return 1
  fi
  return 0
}

fixtures=$repo/devices/tests/fixtures

if [[ $mode == self-test ]]; then
  status=0
  for good in good good-f8 good-rust; do
    echo "== $good fixture: must pass"
    "$self" --root "$fixtures/$good" || { echo "check-devices: self-test: $good fixture did not pass" >&2; status=1; }
  done
  echo "== broken fixture: must fail"
  out=$(mktemp)
  rc=0
  "$self" --root "$fixtures/broken" >"$out" 2>&1 || rc=$?
  cat "$out"
  if broken_failed_as_it_must "$out" "$rc"; then
    echo "check-devices: self-test: broken fixture failed as it must"
  else
    echo "check-devices: self-test: broken fixture did not fail as it must (see above)" >&2; status=1
  fi
  rm -f "$out"
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
tool_args=(--tool "python=$python3_path")
# The slot recipe names veritysetup, and gaia plan wants every named tool
# present even though planning never runs it (Gaia has accepted dropping that).
# Where it isn't installed, planning gets a stand-in so the description can
# still be checked; a build needs the real one. CI installs the real one.
if ! command -v veritysetup >/dev/null 2>&1; then
  stand_in_dir=$(mktemp -d)
  printf '#!/bin/sh\necho "veritysetup stand-in: planning only" >&2\nexit 1\n' > "$stand_in_dir/veritysetup"
  chmod +x "$stand_in_dir/veritysetup"
  tool_args+=(--tool "veritysetup=$stand_in_dir/veritysetup")
  echo "check-devices: NOTE veritysetup is not installed; planning uses a stand-in (a real build needs cryptsetup)"
fi
# The rust builder names bwrap, and gaia plan fails without it on PATH ("needs its input `bwrap`")
# even though planning never runs it. Where it isn't installed, planning gets a stand-in, as
# veritysetup does above; a build of a Rust component needs the real one (bubblewrap).
if ! command -v bwrap >/dev/null 2>&1; then
  stand_in_dir=${stand_in_dir:-$(mktemp -d)}
  printf '#!/bin/sh\necho "bwrap stand-in: planning only" >&2\nexit 1\n' > "$stand_in_dir/bwrap"
  chmod +x "$stand_in_dir/bwrap"
  tool_args+=(--tool "bwrap=$stand_in_dir/bwrap")
  echo "check-devices: NOTE bwrap is not installed; planning uses a stand-in (a build of a Rust component needs bubblewrap)"
fi

# ---- which commands: validate when gaia has it (and plan always), else plan ----

if "$gaia" --help 2>&1 | grep -Eq '^ +gaia validate '; then
  verb=validate
  checks="gaia validate, then gaia plan"
else
  verb=plan
  checks="gaia plan"
fi
echo "gaia: $("$gaia" --version 2>&1 | head -1), command: $verb, locked commit $gaia_commit" >&2

# ---- the tree ------------------------------------------------------------------

[[ -d $root ]] || die "no such directory: $root"
root=$(cd "$root" && pwd)
cd "$root"
regen="scripts/check-devices.sh --write-locks"
[[ $root_given -eq 0 ]] || regen="$regen --root $root"
fail=0
bad() { echo "check-devices: FAIL $*" >&2; fail=1; }

# Each kind is one directory per item, holding its description file. A board directory that holds
# only app/ is recognition-only (a radio, a roboRIO) or a board whose description is not merged
# yet: it is skipped, not counted, and reported in one line. Every other bad entry is reported and
# the rest of the tree is still checked.
app_only=()
for kind in families:family.toml boards:board.toml components:component.toml products:product.toml; do
  dirname_=${kind%%:*}; desc=${kind#*:}
  [[ -d $dirname_ ]] || continue
  while IFS= read -r entry; do
    [[ $entry == .gitkeep ]] && continue
    if [[ ! -d $dirname_/$entry ]]; then
      bad "$dirname_/$entry: a file here; each $dirname_ item is its own directory (see devices/README.md)"
    elif [[ -f $dirname_/$entry/$desc ]]; then
      continue
    elif [[ $dirname_ == boards && $(ls -A "$dirname_/$entry") == app && -d $dirname_/$entry/app ]]; then
      app_only+=("$entry")
    else
      bad "$dirname_/$entry: no $desc in it (a board directory may hold only app/ when it has no $desc)"
    fi
  done < <(ls -A "$dirname_")
done
echo "check-devices: skipped ${#app_only[@]} app-only board director$([[ ${#app_only[@]} == 1 ]] && echo y || echo ies), not counted as boards${app_only[*]+: ${app_only[*]}}"

count() { [[ -d $1 ]] || { echo 0; return; }; find "$1" -mindepth 2 -maxdepth 2 -name "$2" | wc -l; }
nf=$(count families family.toml); nb=$(count boards board.toml)
nc=$(count components component.toml); np=$(count products product.toml)
echo "check-devices: checking $nf families, $nb boards, $nc components, $np products under $root with $checks"

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
unbuilt=""
for f in $(find families boards components -mindepth 2 -maxdepth 2 -name '*.toml' 2>/dev/null | sort); do
  if ! grep -qxF "$(realpath -m "$f")" "$used"; then
    # gaia validate reads every file on its own, so a board no product names
    # yet (a record of a board whose sources aren't chosen) is still checked.
    # Without validate, gaia reads a file only through a product.
    if [ "$verb" = validate ]; then
      unbuilt="$unbuilt ${f%/*.toml}"
    else
      bad "$f: no product or board names it, so gaia never reads it"
    fi
  fi
done

if [ -n "$unbuilt" ]; then
  echo "check-devices: NOTE in no product yet, validated but never planned:$unbuilt"
fi

# gaia validate stops at the first bad file and names its file and key, so each file gets its own
# run and every bad file is reported. The kind comes from the directory the file sits in.
validate_file() {  # <kind> <file>
  if "$gaia" validate "$2" --kind "$1" >"$outs" 2>&1; then
    cat "$outs"
  else
    bad "$2: gaia validate failed:"
    sed 's/^/  /' "$outs" >&2
  fi
}
if [[ $verb == validate ]]; then
  for f in $(find families -mindepth 2 -maxdepth 2 -name family.toml 2>/dev/null | sort); do
    validate_file family "$f"
  done
  for f in $(find boards -mindepth 2 -maxdepth 2 -name board.toml 2>/dev/null | sort); do
    validate_file board "$f"
  done
  for f in $(find components -mindepth 2 -maxdepth 2 -name component.toml 2>/dev/null | sort); do
    validate_file component "$f"
  done
fi

# Every product is validated (when gaia can), then locked into a temp file, and that lock must
# match the gaia.lock committed beside its product.toml (--write-locks writes it). Then the product is
# planned with the committed lock. A tree path in a lock is relative to the product's directory, so
# the same lock holds in every checkout.
locks=$(mktemp -d)
trap 'rm -rf "$locks" "$used" "$errs" "$outs" ${stand_in_dir:+"$stand_in_dir"}' EXIT
for product in $(find products -mindepth 2 -maxdepth 2 -name product.toml 2>/dev/null | sort); do
  pdir=$(dirname "$product")
  committed=$pdir/gaia.lock
  if [[ $verb == validate ]]; then
    validate_file product "$product"
  fi
  lockfile=$locks/$(echo "$product" | tr '/' '_').lock
  # gaia lock asks the network where each source's ref points. A host being
  # unreachable says nothing about the description, so it is retried, and if
  # it stays unreachable the committed lock is planned as it is and the
  # freshness comparison is skipped, out loud.
  lock_ok=0
  offline=0
  for attempt in 1 2 3; do
    if "$gaia" lock "$product" --lock "$lockfile" >"$errs" 2>&1; then
      lock_ok=1
      break
    fi
    if grep -qiE 'unable to access|could not resolve|failed to connect|could not connect|timed out|connection reset|temporary failure' "$errs"; then
      offline=1
      sleep $((attempt * 5))
    else
      offline=0
      break
    fi
  done
  if [[ $lock_ok -eq 0 && $offline -eq 1 && $write_locks -eq 0 && -f $committed ]]; then
    echo "check-devices: NOTE $product: a source host was unreachable, so its gaia.lock was not compared with a fresh one:"
    sed 's/^/  /' "$errs"
    cp "$committed" "$lockfile"
  elif [[ $lock_ok -eq 0 ]]; then
    bad "$product: gaia lock failed:"
    sed 's/^/  /' "$errs" >&2
    continue
  fi
  if [[ $write_locks -eq 1 ]]; then
    cp "$lockfile" "$committed"
    echo "check-devices: wrote $root/$committed"
  elif [[ ! -f $committed ]]; then
    bad "$product: no gaia.lock beside it; write it with: $regen"
    continue
  elif ! cmp -s "$lockfile" "$committed"; then
    bad "$product: gaia.lock beside it differs from what gaia lock writes; regenerate with: $regen"
    { diff "$committed" "$lockfile" || true; } | sed 's/^/  /' >&2
    continue
  fi
  if "$gaia" plan "$product" --lock "$committed" --recipes "$recipes" "${tool_args[@]}" >"$outs" 2>&1; then
    echo "check-devices: ok $product (gaia plan)"
  else
    bad "$product: gaia plan failed:"
    sed 's/^/  /' "$outs" >&2
  fi
done

if [[ $fail -ne 0 ]]; then
  echo "check-devices: FAILED" >&2
  exit 1
fi
echo "check-devices: passed ($nf families, $nb boards, $nc components, $np products)"

# The default run also proves the checker still catches a broken file: the good fixtures must
# pass and the broken one must fail. Not run for --root, which names a tree to check alone.
if [[ $root_given -eq 0 ]]; then
  out=$(mktemp)
  for good in good good-f8 good-rust; do
    if ! "$self" --root "$fixtures/$good" >"$out" 2>&1; then
      cat "$out" >&2; die "self-check: the $good fixture no longer passes"
    fi
  done
  rc=0
  "$self" --root "$fixtures/broken" >"$out" 2>&1 || rc=$?
  if ! broken_failed_as_it_must "$out" "$rc"; then
    cat "$out" >&2
    die "self-check: the broken fixture passed, or failed without naming its bad key, so the checker no longer catches a broken file"
  fi
  rm -f "$out"
  echo "check-devices: self-check passed (good fixtures pass, broken fixture fails)"
fi
