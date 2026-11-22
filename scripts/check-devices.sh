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
# The Gaia commit is read from devices/gaia.lock. The gaia binary is $GAIA_BIN when set, else a build
# from that commit cached under ${ATLAS_GAIA_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/atlas/gaia}/<commit>/.
# Either way it must say, in `gaia --version`, that it was built from that commit (`gaia <version>
# (<commit>)`); a binary that says another commit, says none, or says `-dirty` is refused.
# The recipes come from the same Gaia checkout. Every family, board, component and product file is
# validated (products against the checkout's recipes/, so a bogus recipe name fails), and every
# product is planned.
#
# Each product's gaia.lock, committed beside its product.toml, is checked OFFLINE: `gaia lock` runs on a
# copy of it with git and curl replaced by programs that fail, so a lock that needs a request is a
# failure, not a silent fetch, and the plan uses the committed lock. `--write-locks` is the only mode
# that asks the network (each source's ref, a download, an image's digest): it starts from the committed
# lock, so a source the lock already holds for what its description names is not asked again (gaia lock's
# rule). What is new or changed is resolved. A host that is unreachable or rate limited there is a FAILURE,
# not a note. To refresh a source that did not change in its description, run `gaia lock --update NAME`.
#
# Products that take a bench SSH public key (`{ input = "bench-key" }` in a component) are planned with
# a throwaway public key the check makes in a temp dir for each run; no key is committed.
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
# slot_id_for and the test version (CHECK_VERSION): scripts/version-rule.sh, the one rule's one place.
source "$repo/scripts/version-rule.sh"
# The watchdog owner rule (PR WDOG-OWNER): its functions only; run as a script it is its own check.
source "$repo/scripts/check-watchdog-owner.sh"
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

# ---- the kept result -------------------------------------------------------------------------------------------------
# The default run (devices/, the check and its self-check) is deterministic in its inputs, so a run that passed is kept,
# keyed by every one of them, and a later run with the same key prints what it printed and passes without running.
# The inputs were read off a traced run (every file opened, stat'ed or executed, the children included):
#   - this repository's devices/ (every file, by path and content), the four scripts the check is made of, the
#     Cargo.toml and Cargo.lock (the Lemnos pin rules) and the rust-toolchain files;
#   - the gaia binary (by content) and the recipes of its checkout (by path and content);
#   - every host program the check or gaia runs or looks for, each by its resolved path and content, or as absent
#     (veritysetup's absence changes a note), python's version, and the Rust toolchain gaia asks for from devices/;
#   - the environment variables the check reads.
# A change to any of them misses the key and runs the check. A run with GAIA_BIN, ATLAS_BUILT_TREES or
# ATLAS_BUILT_BOOTLOADERS set (another binary or built trees to read) is never kept or answered from a kept result, nor
# is --root, --self-test or --write-locks. ATLAS_DEVICES_RESULT_CACHE=0 runs the check anyway. A kept run prints its
# output as one stream (stdout); the exit status is the check's own.
devices_result_key() {
  local lock commit gaia_bin t path
  lock=$repo/devices/gaia.lock
  commit=$(sed -n 's/^commit = "\(.*\)"$/\1/p' "$lock" 2>/dev/null) || return 1
  gaia_bin=${ATLAS_GAIA_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/atlas/gaia}/$commit/bin/gaia
  [[ -x $gaia_bin && -d ${gaia_bin%/bin/gaia}/src/recipes ]] || return 1
  {
    echo "devices-result v1"
    (cd "$repo" && find devices -type f -print0 | LC_ALL=C sort -z | xargs -0 sha256sum)
    (cd "$repo" && sha256sum scripts/check-devices.sh scripts/check-watchdog-owner.sh scripts/version-rule.sh \
      scripts/pc-key-strip.sh Cargo.toml Cargo.lock)
    (cd "$repo" && for f in rust-toolchain rust-toolchain.toml; do [[ -f $f ]] && sha256sum "$f"; done; true)
    sha256sum "$gaia_bin"
    (cd "${gaia_bin%/bin/gaia}/src" && find recipes -type f -print0 | LC_ALL=C sort -z | xargs -0 sha256sum)
    for t in bash awk grep sed find sort realpath dirname basename head tail tr cut rm mktemp cat cp mkdir ls wc cmp \
      chmod mv xargs env ssh-keygen python3 git curl bwrap veritysetup mkfs.erofs mke2fs debugfs make patch taskset \
      tar xz zstd gzip cpio fakeroot mtools mcopy sgdisk gdisk mkfs.vfat dtc; do
      if path=$(type -P "$t"); then echo "$t $path $(sha256sum < "$path")"; else echo "$t absent"; fi
    done
    python3 -VV
    (cd "$repo/devices" && rustc -vV 2>&1 || echo "rustc absent")
    getconf GNU_LIBC_VERSION 2>/dev/null || true
    env | grep -E '^(ATLAS_[A-Z_]*|GAIA_[A-Z_]*|CHECK_[A-Z_]*|LC_[A-Z]*|LANG|RUSTUP_TOOLCHAIN|TZ)=' | LC_ALL=C sort
  } | sha256sum | cut -c1-32
}
if [[ $mode == check && $root_given -eq 0 && $write_locks -eq 0 && ${ATLAS_DEVICES_RESULT_CACHE:-1} != 0 \
      && -z ${ATLAS_DEVICES_INNER:-} && -z ${GAIA_BIN:-} && -z ${ATLAS_BUILT_TREES:-} && -z ${ATLAS_BUILT_BOOTLOADERS:-} ]]; then
  results=${XDG_CACHE_HOME:-$HOME/.cache}/atlas/devices-results
  if key=$(devices_result_key) && [[ -n $key ]]; then
    if [[ -f $results/$key ]]; then
      touch "$results/$key" 2>/dev/null || true
      cat "$results/$key"
      echo "check-devices: a kept result: this run's inputs (devices/, the check's scripts, gaia and its recipes, the host tools, the environment) are those of a run that passed"
      exit 0
    fi
    mkdir -p "$results"
    find "$results" -maxdepth 1 -type f -mtime +30 -delete 2>/dev/null || true
    tmp_result=$results/.new-$key.$$
    set +e
    ATLAS_DEVICES_INNER=1 "$self" 2>&1 | tee "$tmp_result"
    rc=${PIPESTATUS[0]}
    set -e
    # Kept only when it passed and its inputs did not move while it ran.
    if [[ $rc -eq 0 && $(devices_result_key) == "$key" ]]; then mv -f "$tmp_result" "$results/$key"; else rm -f "$tmp_result"; fi
    exit "$rc"
  fi
fi

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

# The Lemnos rule (AGENTS.md: every Lemnos component in a product is locked to ONE rev). Every source of
# the Lemnos repository in a product's gaia.lock is one [source.*] block with a url and a commit: the
# commits of those blocks must be one. Prints the distinct revs, one per line.
#
# One named exception (coordinator 2026-11-22, choice A): in products/flasher-bcm2712, the source
# flasher-lemnos-way-back.src alone may be at another Lemnos rev. The tool first ships in Lemnos 612aea88, after the
# flasher's own Lemnos (da86d0b3), and it runs before lemnosd exists, talking to the firmware alone. DEBT: this goes when
# the flasher moves to one newer Lemnos after its flasher faults are fixed (pr/stack-D6F); see
# devices/components/flasher-lemnos-way-back/component.toml. $2, when given, is the one source name exempted.
lemnos_one_rev_exempt() {
  case $1 in
    products/flasher-bcm2712/product.toml) echo "flasher-lemnos-way-back.src" ;;
  esac
}
lemnos_revs() {
  awk -v exempt="${2:-}" '
    /^\[/ { lem = 0; name = $0; sub(/^\[source\."/, "", name); sub(/"\]$/, "", name); skip = (exempt != "" && name == exempt); next }
    /^url = .*Prometheus-Dynamics\/Lemnos/ { lem = !skip; next }
    /^commit = / && lem { c = $0; sub(/^commit = "/, "", c); sub(/"$/, "", c); print c }
  ' "$1" | sort -u
}

# Prints "REV1 and REV2" (plus a count of any more) and returns 1 when the lock pins two or more Lemnos revs.
lemnos_two_revs() {
  local revs=() r
  while IFS= read -r r; do
    [[ -n $r ]] && revs+=("$r")
  done < <(lemnos_revs "$1" "${2:-}")
  [[ ${#revs[@]} -le 1 ]] && return 0
  local more=$(( ${#revs[@]} - 2 )) tail=""
  (( more > 0 )) && tail=" (and $more more)"
  echo "${revs[0]} and ${revs[1]}$tail"
  return 1
}

# Proves the rule catches two revs and passes one, on a lock written here (the fixture is not a product).
lemnos_rule_self_test() {
  local two one rc=0
  two=$(mktemp); one=$(mktemp)
  cat >"$two" <<'EOF'
[source."a.src"]
kind = "git"
url = "https://github.com/Prometheus-Dynamics/Lemnos"
ref = "1111111111111111111111111111111111111111"
commit = "1111111111111111111111111111111111111111"

[source."b.src"]
kind = "git"
url = "https://github.com/Prometheus-Dynamics/Lemnos"
ref = "2222222222222222222222222222222222222222"
commit = "2222222222222222222222222222222222222222"
EOF
  sed 's/2222222222222222222222222222222222222222/1111111111111111111111111111111111111111/g' "$two" >"$one"
  if lemnos_two_revs "$two" >/dev/null; then
    echo "check-devices: self-test: a lock with two Lemnos revs was not refused" >&2; rc=1
  elif [[ $(lemnos_two_revs "$two") != "1111111111111111111111111111111111111111 and 2222222222222222222222222222222222222222" ]]; then
    echo "check-devices: self-test: the two-rev refusal does not name both revs" >&2; rc=1
  fi
  lemnos_two_revs "$one" >/dev/null || { echo "check-devices: self-test: a lock with one Lemnos rev was refused" >&2; rc=1; }
  # The named exception exempts exactly the one source it names, and nothing else.
  lemnos_two_revs "$two" "b.src" >/dev/null || { echo "check-devices: self-test: the exempted source still counted" >&2; rc=1; }
  if lemnos_two_revs "$two" "c.src" >/dev/null; then
    echo "check-devices: self-test: an exemption naming another source let two revs through" >&2; rc=1
  fi
  [[ $(lemnos_one_rev_exempt products/flasher-bcm2712/product.toml) == flasher-lemnos-way-back.src ]] \
    || { echo "check-devices: self-test: the flasher's way-back exception is not the one named" >&2; rc=1; }
  [[ -z $(lemnos_one_rev_exempt products/raze-base/product.toml) ]] \
    || { echo "check-devices: self-test: a slot product has a Lemnos exception" >&2; rc=1; }
  rm -f "$two" "$one"
  [[ $rc -eq 0 ]] && echo "check-devices: self-test: the Lemnos one-rev rule refuses two revs, passes one, and exempts only the one named source"
  return $rc
}

# The Atlas side of the Lemnos pin (PR L3): the workspace's lemnos-slot-store rev must be the Lemnos rev the
# products lock, so Atlas reads and verifies slot bundles in the format of the Lemnos the image is built from.
# Prints the rev on the lemnos-slot-store line of a Cargo.toml (empty when that line names none).
cargo_lemnos_rev() {  # <Cargo.toml>
  awk '/lemnos-slot-store/ && /Prometheus-Dynamics\/Lemnos/ {
    if (match($0, /rev = "[0-9a-f]+"/)) { print substr($0, RSTART + 7, RLENGTH - 8); exit }
  }' "$1"
}

# Prints one line per slot product whose gaia.lock pins a Lemnos rev other than the Cargo.toml's, and returns 1 when
# any does, or when no slot product pins Lemnos at all (nothing to hold Cargo to). Returns 0 with nothing printed
# when every slot product's Lemnos rev is the Cargo rev.
# Only slot products (a recipe of SLOT_RECIPES): Atlas reads their slot bundles, so their Lemnos must be the one
# whose bundle format Atlas was built with. A product that makes no slot (the flasher's initramfs-boot) is held to its
# own one Lemnos rev by the one-rev rule, and may be on another Lemnos than the slot products (D6: the bcm2712 flasher
# on Lemnos da86d0b3 while raze-base stays on its own pin).
# A product whose recipe is one of these boots a slot (sd-image makes the slot file, slot is the slot recipe, slot-boot and slot-disk are
# the slot with its bundle and boot partition: L2d). The Cargo pin rule and the slot chain read it.
SLOT_RECIPES="sd-image|slot|slot-boot|slot-disk"
lemnos_cargo_pin_rule() {  # <Cargo.toml> <devices dir>
  local cargo_rev lock rev bad=0 pinned=0
  cargo_rev=$(cargo_lemnos_rev "$1")
  if [[ -z $cargo_rev ]]; then
    echo "$1 names no Lemnos rev on its lemnos-slot-store line"
    return 1
  fi
  for lock in "$2"/products/*/gaia.lock; do
    [[ -f $lock ]] || continue
    grep -qE "^recipe = \"($SLOT_RECIPES)\"" "$(dirname "$lock")/product.toml" 2>/dev/null || continue
    while IFS= read -r rev; do
      [[ -n $rev ]] || continue
      pinned=1
      if [[ $rev != "$cargo_rev" ]]; then
        echo "$(basename "$(dirname "$lock")") locks Lemnos $rev, but $1 pins lemnos-slot-store at $cargo_rev"
        bad=1
      fi
    done < <(lemnos_revs "$lock")
  done
  [[ $pinned -eq 1 ]] || { echo "no product's gaia.lock pins Lemnos"; return 1; }
  return $bad
}

# The self-test: a Cargo rev that is not the products' Lemnos rev is refused and both revs are named; the real
# tree's pair passes.
lemnos_cargo_pin_self_test() {  # <Cargo.toml> <devices dir>
  local real rc=0 out bad
  real=$(cargo_lemnos_rev "$1")
  bad=$(mktemp)
  sed 's/rev = "[0-9a-f]*"/rev = "1111111111111111111111111111111111111111"/' "$1" >"$bad"
  if out=$(lemnos_cargo_pin_rule "$bad" "$2"); then
    echo "check-devices: self-test: a lemnos-slot-store rev other than the products' was not refused" >&2; rc=1
  elif [[ $out != *"$real"* ]]; then
    echo "check-devices: self-test: the pin refusal does not name the products' rev: $out" >&2; rc=1
  fi
  rm -f "$bad"
  lemnos_cargo_pin_rule "$1" "$2" >/dev/null || { echo "check-devices: self-test: the real Cargo pin no longer matches the products' Lemnos rev" >&2; rc=1; }
  # A product that makes no slot may lock another Lemnos; a slot product may not.
  local fake
  fake=$(mktemp -d)
  mkdir -p "$fake/products/slot" "$fake/products/flasher"
  printf 'recipe = "slot-boot"\n' >"$fake/products/slot/product.toml"
  printf 'recipe = "initramfs-boot"\n' >"$fake/products/flasher/product.toml"
  printf '[source."a.src"]\nurl = "https://github.com/Prometheus-Dynamics/Lemnos"\ncommit = "%s"\n' "$real" >"$fake/products/slot/gaia.lock"
  printf '[source."a.src"]\nurl = "https://github.com/Prometheus-Dynamics/Lemnos"\ncommit = "2222222222222222222222222222222222222222"\n' >"$fake/products/flasher/gaia.lock"
  lemnos_cargo_pin_rule "$1" "$fake" >/dev/null || { echo "check-devices: self-test: a product that makes no slot was held to the Cargo pin" >&2; rc=1; }
  printf 'recipe = "slot"\n' >"$fake/products/flasher/product.toml"
  lemnos_cargo_pin_rule "$1" "$fake" >/dev/null && { echo "check-devices: self-test: a slot product on another Lemnos rev was not refused" >&2; rc=1; }
  rm -rf "$fake"
  [[ $rc -eq 0 ]] && echo "check-devices: self-test: the Cargo pin rule refuses a lemnos-slot-store rev other than the slot products' Lemnos rev, holds no product that makes no slot to it, and passes the real tree"
  return $rc
}

# The one-copy rule for Lemnos crates (PR L3): Cargo.lock holds each Lemnos-repository package at ONE rev. Two
# copies of one crate would be two types for one format in one binary. It is checked from Cargo.lock, which
# `--locked` already holds to the manifests, so no cargo run is needed. Prints "NAME: REV REV" for each package
# at two or more revs (Lemnos's git source, its rev after the #).
lemnos_duplicate_crates() {  # <Cargo.lock>
  awk '
    function flush(   key, rev) {
      if (name != "" && src ~ /Prometheus-Dynamics\/Lemnos/) {
        rev = src; sub(/.*#/, "", rev)
        key = name SUBSEP rev
        if (!(key in seen)) { seen[key] = 1; count[name]++; revs[name] = revs[name] " " rev }
      }
      name = ""; src = ""
    }
    /^\[\[package\]\]/ { flush(); next }
    /^name = / { name = $3; gsub(/"/, "", name) }
    /^source = / { src = $0 }
    END { flush(); for (n in count) if (count[n] > 1) print n ":" revs[n] }
  ' "$1"
}

lemnos_duplicate_self_test() {  # <Cargo.lock>
  local two one rc=0
  two=$(mktemp); one=$(mktemp)
  cat >"$two" <<'EOF'
[[package]]
name = "shm-ring"
version = "1.0.0"
source = "git+https://github.com/Prometheus-Dynamics/Lemnos?rev=1111111111111111111111111111111111111111#1111111111111111111111111111111111111111"

[[package]]
name = "shm-ring"
version = "1.0.0"
source = "git+https://github.com/Prometheus-Dynamics/Lemnos.git?rev=2222222222222222222222222222222222222222#2222222222222222222222222222222222222222"
EOF
  sed '/2222222222222222222222222222222222222222/,$d' "$two" >"$one"
  if [[ -z $(lemnos_duplicate_crates "$two") ]]; then
    echo "check-devices: self-test: two revs of one Lemnos crate were not refused" >&2; rc=1
  elif [[ $(lemnos_duplicate_crates "$two") != *"1111111111111111111111111111111111111111"*"2222222222222222222222222222222222222222"* ]]; then
    echo "check-devices: self-test: the duplicate refusal does not name both revs" >&2; rc=1
  fi
  [[ -z $(lemnos_duplicate_crates "$one") ]] || { echo "check-devices: self-test: one rev of a Lemnos crate was refused" >&2; rc=1; }
  rm -f "$two" "$one"
  [[ $rc -eq 0 ]] && echo "check-devices: self-test: the Lemnos one-copy rule refuses one crate at two revs and passes one rev"
  return $rc
}

# The Lemnos board file rule (PR B26). A product whose components include lemnosd runs a daemon that reads
# /etc/lemnos/board.toml, so the board it is built for must name its Lemnos board file under [descriptions]
# (`lemnos = "lemnos-board.toml"`, beside the board.toml), and that file must exist. Zero names is refused here,
# naming the product and the board. Gaia refuses, at plan time, an install that names a description the board
# does not state, and a description file that does not exist; a product that carries lemnosd and installs
# nothing from the description gets no such refusal, which is the case this rule covers. (A board names one
# file per description name, so one name is one file.)
# Prints the description's path, relative to the board's directory: the board's own [descriptions], else the
# family it extends. Prints nothing when neither names it.
lemnos_description() {
  local file=$1 found fam
  found=$(awk '
    /^[[:space:]]*\[/ { sec = $0; gsub(/[[:space:]]/, "", sec); next }
    sec == "[descriptions]" && /^[[:space:]]*lemnos[[:space:]]*=/ {
      v = $0; sub(/^[^=]*=[[:space:]]*"/, "", v); sub(/".*$/, "", v); print v; exit
    }
  ' "$file")
  if [[ -z $found ]]; then
    fam=$({ grep -ho '^extends = "[^"]*"' "$file" || true; } | sed 's/^extends = "//; s/"$//' | head -1)
    if [[ -n $fam && -f $(dirname "$file")/$fam ]]; then
      found=$(lemnos_description "$(realpath -m "$(dirname "$file")/$fam")")
    fi
  fi
  echo "$found"
}

# Prints a line naming the product and its board and returns 1 when the product carries lemnosd and its board
# names no Lemnos board file, or names one that does not exist. Passes (prints nothing) otherwise.
lemnos_board_rule() {  # <product.toml>
  local product=$1 pdir pname board_rel board_file bname desc
  pdir=$(dirname "$product"); pname=$(basename "$pdir")
  board_rel=$({ grep -ho '"[^"]*board\.toml"' "$product" || true; } | tr -d '"' | head -1)
  board_file=$(realpath -m "$pdir/$board_rel")
  bname=$(basename "$(dirname "$board_file")")
  desc=$(lemnos_description "$board_file")
  if [[ -z $desc ]]; then
    echo "product $pname (board $bname): carries lemnosd, which reads /etc/lemnos/board.toml, but its board names no Lemnos board file under [descriptions] (lemnos = \"lemnos-board.toml\")"
    return 1
  fi
  if [[ ! -f $(dirname "$board_file")/$desc ]]; then
    echo "product $pname (board $bname): carries lemnosd, and its board names the Lemnos board file $desc, which is not beside the board"
    return 1
  fi
  return 0
}

# Proves the rule refuses a product whose board names no Lemnos board file, and one whose named file is missing,
# and passes the real Raze product, on a scratch copy of devices/.
lemnos_board_rule_self_test() {
  local tmp rc=0 out prod
  tmp=$(mktemp -d)
  cp -a "$repo/devices" "$tmp/devices"
  prod=$repo/devices/products/raze-base/product.toml
  if ! out=$(lemnos_board_rule "$prod"); then
    echo "check-devices: self-test: the Raze product no longer passes the Lemnos board file rule: $out" >&2; rc=1
  fi
  sed -i '/^\[descriptions\]$/,$d' "$tmp/devices/boards/raze/board.toml"
  if out=$(lemnos_board_rule "$tmp/devices/products/raze-base/product.toml"); then
    echo "check-devices: self-test: a lemnosd product whose board names no Lemnos board file was not refused" >&2; rc=1
  elif [[ $out != *"product raze-base (board raze)"* || $out != *"no Lemnos board file"* ]]; then
    echo "check-devices: self-test: the refusal does not name the product and board: $out" >&2; rc=1
  fi
  cp -a "$repo/devices/boards/raze/board.toml" "$tmp/devices/boards/raze/board.toml"
  rm -f "$tmp/devices/boards/raze/lemnos-board.toml"
  if out=$(lemnos_board_rule "$tmp/devices/products/raze-base/product.toml"); then
    echo "check-devices: self-test: a lemnosd product whose named Lemnos board file is missing was not refused" >&2; rc=1
  elif [[ $out != *"lemnos-board.toml, which is not beside the board"* ]]; then
    echo "check-devices: self-test: the missing-file refusal does not name the file: $out" >&2; rc=1
  fi
  rm -rf "$tmp"
  [[ $rc -eq 0 ]] && echo "check-devices: self-test: the Lemnos board file rule refuses a lemnosd product with no board file name or a missing file, and passes Raze"
  return $rc
}

# The bring-up rule (PR B31). Every bring-up product (its name ends in -bringup) carries the watchdog and LED signs
# (components/bringup-alive), its own LED and link settings (components/bringup-alive-settings, which install
# /etc/board-alive.leds and /etc/board-alive.links), and a family whose kernel fragment builds in the watchdog driver
# and both LED triggers the signs use. A fifth bring-up product cannot miss them. Prints one line per problem and
# returns 1 when there is any. The watchdog symbol is named per family here: a new family is added by name.
bringup_rule() {  # <devices dir>
  local root=$1 p pdir pname sc board bdir fam frag sym dst s rc=0 out="" want_off dt dtd dtsf ext extf loads links l
  for p in "$root"/products/*-bringup/product.toml; do
    [[ -f $p ]] || continue
    pdir=$(dirname "$p"); pname=$(basename "$pdir")
    if ! grep -q '^bringup-alive = "[^"]*bringup-alive/component\.toml"$' "$p"; then
      out+="product $pname: no bringup-alive component in [components] (the watchdog and LED signs)"$'\n'; rc=1
    elif ! grep -q '^components = \[.*"bringup-alive"' "$p"; then
      out+="product $pname: bringup-alive is named in [components] but no image lists it"$'\n'; rc=1
    fi
    sc=$pdir/components/bringup-alive-settings/component.toml
    if ! grep -q '^bringup-alive-settings = ' "$p" || [[ ! -f $sc ]]; then
      out+="product $pname: no bringup-alive-settings component (the LED and link the signs use)"$'\n'; rc=1
    else
      for dst in /etc/board-alive.leds /etc/board-alive.links; do
        grep -q "^to = \"$dst\"" "$sc" || { out+="product $pname: bringup-alive-settings does not install $dst"$'\n'; rc=1; }
      done
    fi
    board=$(grep -o '^board = "[^"]*"' "$p" | head -1 | cut -d'"' -f2)
    bdir=$(dirname "$(realpath -m "$pdir/$board")")
    fam=$(grep -o '^extends = "[^"]*"' "$bdir/board.toml" | head -1 | cut -d'"' -f2)
    fam=$(dirname "$(realpath -m "$bdir/$fam")")
    sym=
    case $(basename "$fam") in
      rk35xx) sym=CONFIG_DW_WATCHDOG ;;
      sun60i) sym=CONFIG_SUNXI_WATCHDOG ;;
      *) out+="product $pname: family $(basename "$fam") has no watchdog symbol in the bring-up rule"$'\n'; rc=1 ;;
    esac
    frag=$fam/assets/linux.fragment
    for s in ${sym:+$sym} CONFIG_LEDS_TRIGGER_HEARTBEAT CONFIG_LEDS_TRIGGER_TIMER; do
      grep -qx "$s=y" "$frag" 2>/dev/null || { out+="product $pname: $(basename "$fam")'s fragment does not build in $s=y (the bring-up signs need it)"$'\n'; rc=1; }
    done
    # The Ethernet (B31): the bring-up's network is matched by type, Buildroot's name-based eth0.network is off, and
    # every link the LED signs name is one the image creates: `ether` (bringup-network) or usb0 (usb-gadget-network).
    if ! grep -q '^bringup-network = "[^"]*bringup-network/component\.toml"$' "$p"; then
      out+="product $pname: no bringup-network component in [components] (the Ethernet by type)"$'\n'; rc=1
    fi
    # BR2_SYSTEM_DHCP is a STRING option in Buildroot (system/Config.in, default ""): it has no "is not set" form.
    # The line that turns the base fragment's value off is the empty string; Gaia holds every fragment line as
    # written, and "# BR2_SYSTEM_DHCP is not set" becomes BR2_SYSTEM_DHCP="" and fails its build (PR B35).
    if grep -q '^# BR2_SYSTEM_DHCP is not set$' "$pdir/os.fragment" 2>/dev/null; then
      out+="product $pname: its os.fragment says '# BR2_SYSTEM_DHCP is not set'; it is a string option, write BR2_SYSTEM_DHCP=\"\""$'\n'; rc=1
    elif ! grep -q '^BR2_SYSTEM_DHCP=""$' "$pdir/os.fragment" 2>/dev/null || grep -q '^BR2_SYSTEM_DHCP="..*"$' "$pdir/os.fragment" 2>/dev/null; then
      out+="product $pname: its os.fragment does not unset BR2_SYSTEM_DHCP (Buildroot's eth0.network matches Name=eth0, which the GMAC does not have)"$'\n'; rc=1
    fi
    # The loader (B31): an RK35xx bring-up board's device tree carries the reboot-mode node at the offset that
    # places U-Boot's boot-mode register inside the syscon it sits in, with the loader value; its SPL returns to the
    # BootROM for that value (families/rk35xx/assets/u-boot.fragment); and the image's extlinux loads that dtb.
    # The A733 is excused by name (no equivalent is established: docs/prs/B31.md, point 2).
    case $(basename "$fam") in
      rk35xx)
        case $(basename "$bdir") in
          radxa-zero-3e) want_off=0x200 ;;
          radxa-rock-5c) want_off=0x80 ;;
          radxa-rock-4d) want_off=0x40 ;;
          *) want_off=; out+="product $pname: board $(basename "$bdir") has no loader offset in the bring-up rule (add it with its U-Boot register)"$'\n'; rc=1 ;;
        esac
        dt=$(grep -o '^device_tree = "[^"]*"' "$bdir/board.toml" | head -1 | cut -d'"' -f2)
        dtd=$(grep -o '^device_tree_dir = "[^"]*"' "$bdir/board.toml" | head -1 | cut -d'"' -f2)
        dtsf=$bdir/${dtd:-}/$dt.dts
        if [[ -n ${want_off:-} ]]; then
          if [[ ! -f $dtsf ]] || ! grep -q "^\s*reboot-mode {" "$dtsf" || ! grep -q "mode-loader = <0xef08a53c>;" "$dtsf" || ! grep -q "offset = <$want_off>;" "$dtsf"; then
            out+="product $pname: its board's dts ($dtsf) lacks the reboot-mode node with mode-loader = <0xef08a53c> and offset = <$want_off>"$'\n'; rc=1
          fi
        fi
        grep -qx 'CONFIG_SPL_ROCKCHIP_BACK_TO_BROM=y' "$fam/assets/u-boot.fragment" 2>/dev/null || { out+="product $pname: $(basename "$fam")'s U-Boot fragment does not build in CONFIG_SPL_ROCKCHIP_BACK_TO_BROM=y (the SPL must return to the BootROM for the loader value)"$'\n'; rc=1; }
        ext=$(grep -o '^radxa-[a-z0-9-]*-extlinux = "[^"]*"' "$p" | head -1 | cut -d'"' -f2)
        if [[ -n $ext ]]; then
          extf=$(dirname "$(realpath -m "$pdir/$ext")")/extlinux.conf
          loads=$(grep -o 'devicetree /boot/[^ ]*' "$extf" 2>/dev/null | head -1 | sed 's#devicetree /boot/##')
          if [[ $loads != "${dt##*/}.dtb" ]]; then
            out+="product $pname: its extlinux loads /boot/${loads:-nothing}, but its board builds ${dt##*/}.dtb"$'\n'; rc=1
          fi
        fi
        ;;
      sun60i)
        out+="NOTE product $pname: its loader point is EXCUSED by name: the A733 chain does not build: three patch series are named and not applied (families/sun60i/family.toml). No reboot-to-loader equivalent is established either (UNVERIFIED; docs/prs/B31.md, point 2)"$'\n'
        ;;
    esac
    links=$pdir/components/bringup-alive-settings/files/board-alive.links
    if [[ -f $links ]]; then
      while IFS= read -r l; do
        case $l in
          ether) grep -q '^bringup-network = ' "$p" || { out+="product $pname: its LED link is ether, but it has no bringup-network component"$'\n'; rc=1; } ;;
          usb0) grep -q '^usb-gadget-network = ' "$p" || { out+="product $pname: its LED link is usb0, but it has no usb-gadget-network component (the one that creates usb0)"$'\n'; rc=1; } ;;
          *) out+="product $pname: its LED link '$l' is an interface the image does not create (only ether or usb0)"$'\n'; rc=1 ;;
        esac
      done < <(grep -v '^[[:space:]]*\(#\|$\)' "$links")
    fi
  done
  net=$root/components/bringup-network/files/10-bringup-ethernet.network
  if compgen -G "$root/products/*-bringup/product.toml" >/dev/null && { ! grep -qx 'Type=ether' "$net" 2>/dev/null || grep -v '^#' "$net" 2>/dev/null | grep -q 'Name=eth0'; }; then
    out+="components/bringup-network: its file must match Type=ether, and never a legacy name such as Name=eth0"$'\n'; rc=1
  fi
  printf '%s' "$out"
  return $rc
}

# Proves the bring-up rule passes the real tree, and refuses (on a scratch copy of devices/) a bring-up product
# with no watchdog component, one whose settings do not install the LED file, and a family missing a trigger.
bringup_rule_self_test() {
  local tmp rc=0 out
  tmp=$(mktemp -d)
  cp -a "$repo/devices" "$tmp/devices"
  # The fixture stands in for a bring-up product (generic name, not a live product): devices/tests/fixtures/bringup.
  mkdir -p "$tmp/devices/products/example-bringup" && cp -a "$repo/devices/tests/fixtures/bringup/." "$tmp/devices/products/example-bringup/"
  if ! out=$(bringup_rule "$tmp/devices"); then
    echo "check-devices: self-test: the real tree no longer passes the bring-up rule: $out" >&2; rc=1
  fi
  sed -i '/^bringup-alive = /d' "$tmp/devices/products/example-bringup/product.toml"
  if out=$(bringup_rule "$tmp/devices"); then
    echo "check-devices: self-test: a bring-up product with no watchdog component was not refused" >&2; rc=1
  elif [[ $out != *"product example-bringup: no bringup-alive component"* ]]; then
    echo "check-devices: self-test: the refusal does not name the product: $out" >&2; rc=1
  fi
  cp -a "$repo/devices/tests/fixtures/bringup/product.toml" "$tmp/devices/products/example-bringup/product.toml"
  sed -i '/^to = "\/etc\/board-alive.leds"$/d' "$tmp/devices/products/rock-5c-bringup/components/bringup-alive-settings/component.toml"
  if out=$(bringup_rule "$tmp/devices"); then
    echo "check-devices: self-test: settings that do not install the LED file were not refused" >&2; rc=1
  elif [[ $out != *"product rock-5c-bringup: bringup-alive-settings does not install /etc/board-alive.leds"* ]]; then
    echo "check-devices: self-test: the settings refusal does not name the file: $out" >&2; rc=1
  fi
  cp -a "$repo/devices/products/rock-5c-bringup/components/bringup-alive-settings/component.toml" "$tmp/devices/products/rock-5c-bringup/components/bringup-alive-settings/component.toml"
  sed -i '/^CONFIG_LEDS_TRIGGER_TIMER=y$/d' "$tmp/devices/families/rk35xx/assets/linux.fragment"
  if out=$(bringup_rule "$tmp/devices"); then
    echo "check-devices: self-test: a family without the timer trigger was not refused" >&2; rc=1
  elif [[ $out != *"rk35xx's fragment does not build in CONFIG_LEDS_TRIGGER_TIMER=y"* ]]; then
    echo "check-devices: self-test: the trigger refusal does not name the symbol: $out" >&2; rc=1
  fi
  cp -a "$repo/devices/components/bringup-network/files/10-bringup-ethernet.network" "$tmp/devices/components/bringup-network/files/10-bringup-ethernet.network"
  echo eth0 > "$tmp/devices/products/rock-5c-bringup/components/bringup-alive-settings/files/board-alive.links"
  if out=$(bringup_rule "$tmp/devices"); then
    echo "check-devices: self-test: an LED link named eth0 (not created by the image) was not refused" >&2; rc=1
  elif [[ $out != *"product rock-5c-bringup: its LED link 'eth0' is an interface the image does not create"* ]]; then
    echo "check-devices: self-test: the link refusal does not name the product and link: $out" >&2; rc=1
  fi
  echo ether > "$tmp/devices/products/rock-5c-bringup/components/bringup-alive-settings/files/board-alive.links"
  sed -i '/^BR2_SYSTEM_DHCP=""$/d' "$tmp/devices/products/example-bringup/os.fragment"
  if out=$(bringup_rule "$tmp/devices"); then
    echo "check-devices: self-test: a bring-up product that leaves BR2_SYSTEM_DHCP on was not refused" >&2; rc=1
  elif [[ $out != *"product example-bringup: its os.fragment does not unset BR2_SYSTEM_DHCP"* ]]; then
    echo "check-devices: self-test: the DHCP refusal does not name the product: $out" >&2; rc=1
  fi
  cp -a "$repo/devices/tests/fixtures/bringup/os.fragment" "$tmp/devices/products/example-bringup/os.fragment"
  sed -i 's/^Type=ether$/Name=eth0/' "$tmp/devices/components/bringup-network/files/10-bringup-ethernet.network"
  if out=$(bringup_rule "$tmp/devices"); then
    echo "check-devices: self-test: a bring-up network file matching eth0 by name was not refused" >&2; rc=1
  elif [[ $out != *"bringup-network: its file must match Type=ether"* ]]; then
    echo "check-devices: self-test: the network-file refusal does not name the file: $out" >&2; rc=1
  fi
  sed -i 's/offset = <0x40>;/offset = <0x44>;/' "$tmp/devices/boards/radxa-rock-4d/assets/dts/rockchip/rk3576-rock-4d-loader.dts"
  if out=$(bringup_rule "$tmp/devices"); then
    echo "check-devices: self-test: a reboot-mode node at the wrong offset was not refused" >&2; rc=1
  elif [[ $out != *"product example-bringup: its board's dts"* ]]; then
    echo "check-devices: self-test: the reboot-mode refusal does not name the product: $out" >&2; rc=1
  fi
  cp -a "$repo/devices/boards/radxa-rock-4d/assets/dts/rockchip/rk3576-rock-4d-loader.dts" "$tmp/devices/boards/radxa-rock-4d/assets/dts/rockchip/rk3576-rock-4d-loader.dts"
  sed -i 's#devicetree /boot/rk3588s-rock-5c-loader.dtb#devicetree /boot/rk3588s-rock-5c.dtb#' "$tmp/devices/components/radxa-rock-5c-extlinux/extlinux.conf"
  if out=$(bringup_rule "$tmp/devices"); then
    echo "check-devices: self-test: an extlinux that loads the wrong dtb was not refused" >&2; rc=1
  elif [[ $out != *"product rock-5c-bringup: its extlinux loads /boot/rk3588s-rock-5c.dtb"* ]]; then
    echo "check-devices: self-test: the extlinux refusal does not name the dtb: $out" >&2; rc=1
  fi
  rm -rf "$tmp"
  [[ $rc -eq 0 ]] && echo "check-devices: self-test: the bring-up rule passes the real tree and refuses a bring-up product with no watchdog component, settings without the LED file, a family without a trigger, a LED link the image does not create, a product that leaves BR2_SYSTEM_DHCP on, or a network file matched by a legacy name, a reboot-mode node at the wrong offset, or an extlinux that loads the wrong dtb"
  return $rc
}

# ---- the boot firmware, the slot's erofs compression, and the unnamed-file note (PR B32) -----------------

# Prints the boot firmware a board states: its own [boot] firmware, else its family's (Gaia reads it for the kernel
# contract). Prints nothing when neither states one.
boot_firmware() {  # <board.toml>
  local file=$1 found fam
  found=$(awk '
    /^[[:space:]]*\[/ { sec = $0; gsub(/[[:space:]]/, "", sec); next }
    sec == "[boot]" && /^[[:space:]]*firmware[[:space:]]*=/ {
      v = $0; sub(/^[^=]*=[[:space:]]*"/, "", v); sub(/".*$/, "", v); print v; exit
    }
  ' "$file")
  if [[ -z $found ]]; then
    fam=$({ grep -ho '^extends = "[^"]*"' "$file" || true; } | sed 's/^extends = "//; s/"$//' | head -1)
    if [[ -n $fam && -f $(dirname "$file")/$fam ]]; then
      found=$(boot_firmware "$(realpath -m "$(dirname "$file")/$fam")")
    fi
  fi
  echo "$found"
}

# Prints the `record` of the system-slots device in a Lemnos board file: the config's record key, or "tryboot" when the
# key is absent (drivers/system-slots reads it with that default). Prints nothing when the file has no system-slots device.
lemnos_slot_record() {  # <lemnos board file>
  awk '
    function flush(   rec) {
      if (!done && drv == "system-slots") {
        rec = "tryboot"
        if (match(cfg, /[{ ,]record[[:space:]]*=[[:space:]]*"[^"]*"/)) {
          rec = substr(cfg, RSTART, RLENGTH)
          sub(/^[{ ,]record[[:space:]]*=[[:space:]]*"/, "", rec); sub(/"$/, "", rec)
        }
        print rec; done = 1
      }
      drv = ""; cfg = ""
    }
    /^\[/ { flush(); next }
    /^[[:space:]]*driver[[:space:]]*=/ { drv = $0; sub(/^[^=]*=[[:space:]]*"/, "", drv); sub(/".*$/, "", drv) }
    /^[[:space:]]*config[[:space:]]*=/ { cfg = $0 }
    END { flush() }
  ' "$1"
}

# Gaia's boot firmware name -> the Lemnos system-slots record that boots it. The mapping is written here, once: a
# firmware with no line is refused by firmware_rule, so a new one is added on purpose. (Lemnos's other records,
# "uboot" and "file", have no Gaia firmware name yet.)
firmware_record() {  # <gaia firmware name>
  case $1 in
    rpi-tryboot) echo tryboot ;;
    *) ;;
  esac
}

# The boot firmware rule (PR B32). A board that states its boot firmware (Gaia's [boot] firmware, own or from its family)
# and names a Lemnos board file must agree with that file: the system-slots `record` is how the boot loader is told which
# slot to start, so it must be the record that firmware boots by (firmware_record). Prints one line per problem and
# returns 1 when any. A board with a firmware and no Lemnos file is not checked here (nothing to agree with).
firmware_rule() {  # <devices dir>
  local root=$1 b bdir bname fw want rec lf out="" rc=0
  for b in "$root"/boards/*/board.toml; do
    [[ -f $b ]] || continue
    bdir=$(dirname "$b"); bname=$(basename "$bdir")
    fw=$(boot_firmware "$b")
    [[ -n $fw ]] || continue
    lf=$(lemnos_description "$b")
    [[ -n $lf ]] || continue
    if [[ ! -f $bdir/$lf ]]; then
      out+="board $bname: [boot] firmware \"$fw\" and its Lemnos board file $lf, which is not beside the board"$'\n'; rc=1; continue
    fi
    want=$(firmware_record "$fw")
    if [[ -z $want ]]; then
      out+="board $bname: [boot] firmware \"$fw\" has no Lemnos record in firmware_record (scripts/check-devices.sh): add its line there"$'\n'; rc=1; continue
    fi
    rec=$(lemnos_slot_record "$bdir/$lf")
    if [[ -z $rec ]]; then
      out+="board $bname: its Lemnos board file $lf has no system-slots device to read a record from"$'\n'; rc=1; continue
    fi
    if [[ $rec != "$want" ]]; then
      out+="board $bname: [boot] firmware \"$fw\" boots by record \"$want\", but its Lemnos board file $lf says record \"$rec\""$'\n'; rc=1
    fi
  done
  printf '%s' "$out"
  return $rc
}

# The slot id, product and version of a slot-boot image (Lemnos next bfbe88f4, from its code):
#   product: lowercase [a-z0-9-], 1..64 bytes.  version: 1..256 bytes, printable, no control character.
#   slot id: [a-z0-9._-], 1..64 bytes, not starting with a dot; uppercase and `+` are refused by the board.
# THE ONE RULE (scripts/version-rule.sh): slot = <product>-<version>, with every `+` read as `-`, lowercased.
# The version and slot are NOT in product.toml: Gaia takes them as --setting (slot_settings_args), so a product that
# sets either is refused here, and the id checked is the one the CHECK_VERSION test value gives (or the version given).
slot_setting() {  # <product.toml> <key> -> the [images.settings] value (string or number)
  awk -v k="$2" '
    /^\[/ { on = ($0 == "[images.settings]") ; next }
    on && $1 == k && $2 == "=" { v = $0; sub(/^[^=]*=[ \t]*/, "", v); gsub(/^"|"[ \t]*(#.*)?$/, "", v); print v; exit }
  ' "$1"
}
slot_sets() {  # <product.toml> <key> -> "yes" when [images.settings] sets the key at all
  awk -v k="$2" '
    /^\[/ { on = ($0 == "[images.settings]") ; next }
    on && $1 == k && $2 == "=" { print "yes"; exit }
  ' "$1"
}
slot_id_rule() {  # <devices dir> [version to check with; default CHECK_VERSION]
  local dev=$1 ver=${2:-$CHECK_VERSION} p pname rc=0 out="" product want k
  while IFS= read -r p; do
    grep -qE '^recipe = "slot-(boot|disk)"' "$p" || continue
    pname=$(basename "$(dirname "$p")")
    for k in version slot; do
      if [[ $(slot_sets "$p" "$k") == yes ]]; then
        out+="products/$pname: sets $k in product.toml; the slot-boot recipe takes it as a --setting (scripts/version-rule.sh), so the product leaves it unset"$'\n'; rc=1
      fi
    done
    product=$(slot_setting "$p" product)
    if [[ ! $product =~ ^[a-z0-9-]{1,64}$ ]]; then
      out+="products/$pname: setting product \"$product\" is not 1..64 of [a-z0-9-] (Lemnos refuses it)"$'\n'; rc=1
    fi
    if [[ -z $ver || ${#ver} -gt 256 || $ver =~ [[:cntrl:]] ]]; then
      out+="products/$pname: version \"$ver\" is not 1..256 bytes of printable text (Lemnos refuses it)"$'\n'; rc=1
    fi
    want=$(slot_id_for "$product" "$ver")
    if [[ ! $want =~ ^[a-z0-9_-][a-z0-9._-]{0,63}$ ]]; then
      out+="products/$pname: the slot id \"$want\" that version \"$ver\" gives is not 1..64 of [a-z0-9._-] without a leading dot (Lemnos refuses it); shorten the version's commit part"$'\n'; rc=1
    fi
  done < <(find "$dev/products" -mindepth 2 -maxdepth 2 -name product.toml 2>/dev/null | sort)
  printf '%s' "$out"
  return $rc
}

# The boot partition rule: a slot-boot image's `boot_size` is its board's boot partition (the [[partitions]] entry
# with role = "boot", whose size Gaia lays the disk out by). Two numbers for one partition must be one number.
board_boot_size() {  # <board.toml> -> the size of its role = "boot" partition, as written
  awk '
    /^\[\[partitions\]\]/ { if (on && role == "boot") { print size; done = 1; exit } on = 1; role = ""; size = ""; next }
    /^\[/ { if (on && role == "boot") { print size; done = 1; exit } on = 0; next }
    on && $1 == "role" { role = $3; gsub(/"/, "", role) }
    on && $1 == "size" { size = $3; gsub(/"/, "", size) }
    END { if (!done && on && role == "boot") print size }
  ' "$1"
}
boot_size_rule() {  # <devices dir>
  local dev=$1 p pname board want have rc=0 out=""
  while IFS= read -r p; do
    grep -qE '^recipe = "slot-(boot|disk)"' "$p" || continue
    pname=$(basename "$(dirname "$p")")
    board=$(awk -F'"' '/^board = /{print $2; exit}' "$p")
    want=$(slot_setting "$p" boot_size)
    have=$(board_boot_size "$(dirname "$p")/$board")
    if [[ -z $have ]]; then
      out+="products/$pname: its board ($board) has no [[partitions]] entry with role = \"boot\" (the disk Gaia lays out needs it)"$'\n'; rc=1
    elif [[ $have != "$want" ]]; then
      out+="products/$pname: setting boot_size $want is not its board's boot partition size $have ($board): one partition, one number"$'\n'; rc=1
    fi
  done < <(find "$dev/products" -mindepth 2 -maxdepth 2 -name product.toml 2>/dev/null | sort)
  printf '%s' "$out"
  return $rc
}
boot_size_rule_self_test() {
  local tmp rc=0 out
  tmp=$(mktemp -d)
  cp -a "$repo/devices" "$tmp/devices"
  if ! out=$(boot_size_rule "$tmp/devices"); then
    echo "check-devices: self-test: the real tree no longer passes the boot partition rule: $out" >&2; rc=1
  fi
  sed -i 's/^size = "82837504"/size = "82837505"/' "$tmp/devices/boards/raze/board.toml"
  if out=$(boot_size_rule "$tmp/devices"); then
    echo "check-devices: self-test: a boot_size that is not the board's boot partition was not refused" >&2; rc=1
  elif [[ $out != *'one partition, one number'* ]]; then
    echo "check-devices: self-test: the boot partition refusal does not say why: $out" >&2; rc=1
  fi
  rm -rf "$tmp"
  [[ $rc -eq 0 ]] && echo "check-devices: self-test: the boot partition rule passes the real tree and refuses a boot_size that is not the board's"
  return $rc
}

# Proves the slot id rule passes the real tree and refuses: a product that sets version or slot, an uppercase or
# `+` version that gives a refused id, an id over 64 bytes, and a product name outside [a-z0-9-].
slot_id_rule_self_test() {
  local tmp rc=0 out pf sargs
  tmp=$(mktemp -d)
  cp -a "$repo/devices" "$tmp/devices"
  pf=$tmp/devices/products/raze-base/product.toml
  if ! out=$(slot_id_rule "$tmp/devices"); then
    echo "check-devices: self-test: the real tree no longer passes the slot id rule: $out" >&2; rc=1
  fi
  [[ $(slot_id_for raze-base "2026.1.0-dev.993+1A2B3C4D") == "raze-base-2026.1.0-dev.993-1a2b3c4d" ]] \
    || { echo "check-devices: self-test: slot_id_for does not read + as - and lowercase" >&2; rc=1; }
  # A product that sets the slot itself is refused: the slot is the version's, given as a --setting.
  sed -i 's/^\[images.settings\]$/[images.settings]\nslot = "raze-base-hand-picked"/' "$pf"
  if out=$(slot_id_rule "$tmp/devices"); then
    echo "check-devices: self-test: a product that sets slot was not refused" >&2; rc=1
  elif [[ $out != *'sets slot in product.toml'* ]]; then
    echo "check-devices: self-test: the slot refusal does not say why: $out" >&2; rc=1
  fi
  # A product that still sets version is refused by name (Gaia would refuse it too: a --setting for a set name).
  cp -a "$repo/devices/products/raze-base/product.toml" "$pf"
  sed -i 's/^\[images.settings\]$/[images.settings]\nversion = "2026.1.0-dev.1+l2d"/' "$pf"
  if out=$(slot_id_rule "$tmp/devices"); then
    echo "check-devices: self-test: a product that sets version was not refused" >&2; rc=1
  elif [[ $out != *'sets version in product.toml'* ]]; then
    echo "check-devices: self-test: the version refusal does not say why: $out" >&2; rc=1
  fi
  # A version whose slot id is over 64 bytes is refused (the check uses the version it is given).
  cp -a "$repo/devices/products/raze-base/product.toml" "$pf"
  if out=$(slot_id_rule "$tmp/devices" "2026.1.0-dev.1+0123456789abcdef0123456789abcdef01234567"); then
    echo "check-devices: self-test: a slot id over 64 bytes was not refused" >&2; rc=1
  elif [[ $out != *'is not 1..64 of [a-z0-9._-]'* ]]; then
    echo "check-devices: self-test: the length refusal does not say why: $out" >&2; rc=1
  fi
  # The settings the rule hands Gaia: the image's version and its slot, from the one rule.
  sargs=$(slot_settings_args "$pf" "$CHECK_VERSION" | tr '\n' ' ')
  [[ $sargs == "--setting raze-base.version=$CHECK_VERSION --setting raze-base.slot=raze-base-2026.1.0-dev.0-check " ]] \
    || { echo "check-devices: self-test: slot_settings_args gives \"$sargs\"" >&2; rc=1; }
  cp -a "$repo/devices/products/raze-base/product.toml" "$pf"
  sed -i 's/^product = ".*"/product = "Raze_Base"/' "$pf"
  if out=$(slot_id_rule "$tmp/devices"); then
    echo "check-devices: self-test: a product name outside [a-z0-9-] was not refused" >&2; rc=1
  elif [[ $out != *'setting product "Raze_Base"'* ]]; then
    echo "check-devices: self-test: the product refusal does not name it: $out" >&2; rc=1
  fi
  rm -rf "$tmp"
  [[ $rc -eq 0 ]] && echo "check-devices: self-test: the slot id rule passes the real tree and refuses a product that sets version or slot, an id over 64 bytes and a bad product name, and hands Gaia the test value's settings"
  return $rc
}

# Proves the boot firmware rule passes the real tree, refuses (on a scratch copy) a Lemnos record that disagrees with the
# board's firmware, and refuses a firmware with no record mapping.
firmware_rule_self_test() {
  local tmp rc=0 out
  tmp=$(mktemp -d)
  cp -a "$repo/devices" "$tmp/devices"
  if ! out=$(firmware_rule "$tmp/devices"); then
    echo "check-devices: self-test: the real tree no longer passes the boot firmware rule: $out" >&2; rc=1
  fi
  sed -i 's/record = "tryboot", boot_args/record = "uboot", boot_args/' "$tmp/devices/boards/raze/lemnos-board.toml"
  if out=$(firmware_rule "$tmp/devices"); then
    echo "check-devices: self-test: a Raze whose Lemnos record is uboot, against rpi-tryboot, was not refused" >&2; rc=1
  elif [[ $out != *'board raze: [boot] firmware "rpi-tryboot" boots by record "tryboot", but its Lemnos board file lemnos-board.toml says record "uboot"'* ]]; then
    echo "check-devices: self-test: the record refusal does not name the board, firmware and both records: $out" >&2; rc=1
  fi
  cp -a "$repo/devices/boards/raze/lemnos-board.toml" "$tmp/devices/boards/raze/lemnos-board.toml"
  sed -i 's/^firmware = "rpi-tryboot"$/firmware = "rpi-nope"/' "$tmp/devices/boards/raze/board.toml"
  if out=$(firmware_rule "$tmp/devices"); then
    echo "check-devices: self-test: a firmware with no record mapping was not refused" >&2; rc=1
  elif [[ $out != *'board raze: [boot] firmware "rpi-nope" has no Lemnos record'* ]]; then
    echo "check-devices: self-test: the mapping refusal does not name the firmware: $out" >&2; rc=1
  fi
  rm -rf "$tmp"
  [[ $rc -eq 0 ]] && echo "check-devices: self-test: the boot firmware rule passes the real tree and refuses a Lemnos record that disagrees with the firmware, and a firmware with no record mapping"
  return $rc
}

# Prints the kernel fragments a family or board names, one path per line, relative to the directory of that file.
fragments_of() {  # <family.toml or board.toml>
  local file=$1 f
  while IFS= read -r f; do
    printf '%s\n' "$(dirname "$file")/$f"
  done < <(grep -E '^fragments = ' "$file" 2>/dev/null | grep -ho '"[^"]*\.\(fragment\|config\)"' | tr -d '"')
}

# Succeeds when one of the files sets CONFIG_<sym>=y.
has_y() {  # <symbol> <file>...
  local sym=$1 f
  shift
  for f in "$@"; do
    [[ -f $f ]] && grep -qE "^CONFIG_${sym}=y\$" "$f" && return 0
  done
  return 1
}

# The kernel lines an erofs image needs for its compression (PR B32, from the INPUTS table). Cumulative: each compressor's
# option is under EROFS_FS_ZIP in the kernel's Kconfig, so zstd needs all three lines. Prints them, one per line; returns 1
# for a value with no line set. Not checked against a kernel tree here: the table is the rule.
erofs_lines() {  # <compression>
  case $1 in
    none) printf '%s\n' EROFS_FS ;;
    lz4|lz4hc) printf '%s\n' EROFS_FS EROFS_FS_ZIP ;;
    lzma) printf '%s\n' EROFS_FS EROFS_FS_ZIP EROFS_FS_ZIP_LZMA ;;
    deflate) printf '%s\n' EROFS_FS EROFS_FS_ZIP EROFS_FS_ZIP_DEFLATE ;;
    zstd) printf '%s\n' EROFS_FS EROFS_FS_ZIP EROFS_FS_ZIP_ZSTD ;;
    *) return 1 ;;
  esac
}

# Prints "IMAGE COMPRESSION" for each erofs image in a plan (`gaia plan --json`): the constant `compression` of the node
# labelled <image>::erofs. A compression that is not a constant prints "-".
erofs_images() {  # <plan.json>
  python3 -I -c '
import json, sys
graph = json.load(open(sys.argv[1]))["graph"]["graph"]
for node in graph["nodes"]:
    label = node.get("label", "")
    if label.endswith("::erofs"):
        const = node.get("consts", {}).get("compression", {})
        value = const.get("value", "-") if const.get("type") == "str" else "-"
        print(label[: -len("::erofs")], value)
' "$1"
}

# Prints the board file a product names, as a path, or nothing.
product_board() {  # <product.toml>
  local bpath
  bpath=$(grep -ho '"[^"]*board\.toml"' "$1" | tr -d '"' | head -1)
  if [[ -n $bpath ]]; then realpath -m "$(dirname "$1")/$bpath"; fi
  return 0
}

# Prints the name of the family the product's board extends, or nothing.
product_family() {  # <product.toml>
  local b fam
  b=$(product_board "$1")
  [[ -n $b && -f $b ]] || return 0
  fam=$(grep -ho '"[^"]*family\.toml"' "$b" | tr -d '"' | head -1)
  [[ -n $fam ]] || return 0
  basename "$(dirname "$(realpath -m "$(dirname "$b")/$fam")")"
}

# The slot root rule (PR B32; replaces the assumed zstd). Each product's own plan says what its erofs images compress with
# (erofs_images), and the kernel it builds (its family's fragments and its board's) must set the lines for that value
# (erofs_lines). A family that no product names is held to zstd, and the output says so. Prints one NOTE or FAIL line per
# item; returns 1 when any FAIL.
erofs_rule() {  # <devices dir> <plans dir>: plans hold <product dir name>.json
  local dev=$1 plans=$2 p pname pj b fam img comp lines sym f d named=" " out="" rc=0
  local frags=()
  while IFS= read -r p; do
    pname=$(basename "$(dirname "$p")")
    fam=$(product_family "$p")
    named+="$fam "
    if [[ -z $fam ]]; then
      out+="FAIL products/$pname: its board names no family, so its kernel cannot be checked"$'\n'; rc=1; continue
    fi
    pj=$plans/$pname.json
    [[ -f $pj ]] || continue
    b=$(product_board "$p")
    frags=()
    while IFS= read -r f; do frags+=("$f"); done < <(fragments_of "$dev/families/$fam/family.toml"; fragments_of "$b")
    while read -r img comp; do
      [[ -n $img ]] || continue
      if ! lines=$(erofs_lines "$comp"); then
        out+="FAIL products/$pname: image $img's erofs compression is '$comp' in its plan: no kernel lines are known for that"$'\n'; rc=1; continue
      fi
      out+="NOTE products/$pname: image $img compresses its slot with $comp (its plan): the kernel needs $(echo $lines)"$'\n'
      while IFS= read -r sym; do
        if ! has_y "$sym" ${frags[@]+"${frags[@]}"}; then
          out+="FAIL products/$pname: image $img compresses with $comp, so CONFIG_$sym must be =y; neither families/$fam nor the product's board builds it in"$'\n'; rc=1
        fi
      done <<<"$lines"
    done < <(erofs_images "$pj")
  done < <(find "$dev/products" -mindepth 2 -maxdepth 2 -name product.toml 2>/dev/null | sort)
  for d in "$dev"/families/*/; do
    [[ -d $d ]] || continue
    fam=$(basename "$d")
    [[ $named == *" $fam "* ]] && continue
    frags=()
    while IFS= read -r f; do frags+=("$f"); done < <(fragments_of "$d/family.toml")
    out+="NOTE families/$fam has no product yet: its slot root is held to zstd (assumed; no plan states it)"$'\n'
    for sym in EROFS_FS EROFS_FS_ZIP EROFS_FS_ZIP_ZSTD; do
      if ! has_y "$sym" ${frags[@]+"${frags[@]}"}; then
        out+="FAIL families/$fam: CONFIG_$sym is not =y in its kernel fragments, and a family with no product is held to zstd"$'\n'; rc=1
      fi
    done
  done
  printf '%s' "$out"
  return $rc
}

# Proves the erofs rule reads the plan's value: on a scratch copy, the Raze's family builds EROFS_FS_ZIP and no ZSTD line
# (every EROFS_FS_ZIP line is taken out first). The product with compression = "lz4" passes; the same kernel with the
# default (lzma, from the recipe; L2d) is refused by name. Then a family with no product is held to zstd (a refusal).
# Uses the gaia binary and the tool arguments the run set up.
erofs_rule_self_test() {
  local tmp rc=0 pa pb pc out prod lockcopy
  tmp=$(mktemp -d); pa=$(mktemp -d); pb=$(mktemp -d); pc=$(mktemp -d)
  cp -a "$repo/devices" "$tmp/devices"
  # Every EROFS_FS_ZIP line of the scratch copy goes, whatever the tree holds now (B34 adds some to the families); the
  # test then builds the one kernel it needs: the Raze's family builds EROFS_FS_ZIP and not EROFS_FS_ZIP_ZSTD.
  grep -rl -E '^CONFIG_EROFS_FS_ZIP' "$tmp/devices/families" "$tmp/devices/boards" 2>/dev/null \
    | xargs -r sed -i -E '/^CONFIG_EROFS_FS_ZIP(_[A-Z]+)?=y$/d'
  # The families that no product names (sun60i, since PADS removed its only product) are held to zstd by the rule itself, so
  # their real fragments go back in the scratch copy: the test controls bcm2712 (the product it plans) and nothing else.
  for f in "$repo"/devices/families/*/assets/*.fragment; do
    [[ $f == */families/bcm2712/* ]] && continue
    cp -a "$f" "$tmp/devices/${f#"$repo/devices/"}"
  done
  echo "CONFIG_EROFS_FS_ZIP=y" >> "$tmp/devices/families/bcm2712/assets/linux.fragment"
  prod=$tmp/devices/products/raze-base/product.toml
  lockcopy=$tmp/devices/products/raze-base/gaia.lock
  mapfile -t sargs < <(slot_settings_args "$prod" "$CHECK_VERSION")
  # The product states compression = "lzma" (L2d); the scratch copy swaps that line for lz4, then removes it for the recipe default.
  sed -i 's/^compression = "lzma"$/compression = "lz4"/' "$prod"
  grep -q '^compression = "lz4"$' "$prod" || { echo "check-devices: self-test: the scratch product has no compression line to change" >&2; rc=1; }
  if ! "$gaia" plan "$prod" --lock "$lockcopy" --recipes "$recipes" "${tool_args[@]}" ${sargs[@]+"${sargs[@]}"} --json >"$pa/raze-base.json" 2>"$errs"; then
    cat "$errs" >&2; echo "check-devices: self-test: the lz4 scratch product did not plan" >&2; rc=1
  fi
  sed -i '/^compression = "lz4"$/d' "$prod"   # the recipe's default (lzma at the pinned Gaia)
  if ! "$gaia" plan "$prod" --lock "$lockcopy" --recipes "$recipes" "${tool_args[@]}" ${sargs[@]+"${sargs[@]}"} --json >"$pb/raze-base.json" 2>"$errs"; then
    cat "$errs" >&2; echo "check-devices: self-test: the default scratch product did not plan" >&2; rc=1
  fi
  if out=$(erofs_rule "$tmp/devices" "$pa") && [[ $out == *"image raze-base compresses its slot with lz4"* ]]; then
    :
  else
    echo "check-devices: self-test: the lz4 product was refused, or its plan's lz4 was not read: $out" >&2; rc=1
  fi
  if out=$(erofs_rule "$tmp/devices" "$pb"); then
    echo "check-devices: self-test: the recipe's default (lzma) was not refused on a kernel with no EROFS_FS_ZIP_LZMA" >&2; rc=1
  elif [[ $out != *"image raze-base compresses with lzma, so CONFIG_EROFS_FS_ZIP_LZMA must be =y"* ]]; then
    echo "check-devices: self-test: the lzma refusal does not name the plan's value and the symbol: $out" >&2; rc=1
  fi
  # Every product of bcm2712 goes (raze-base, and the flasher, whose initramfs-boot makes no slot), so the family has none.
  local fp
  for fp in "$tmp"/devices/products/*/product.toml; do
    [[ $(product_family "$fp") == bcm2712 ]] && rm -rf "$(dirname "$fp")"
  done
  if out=$(erofs_rule "$tmp/devices" "$pc") || [[ $out != *"FAIL families/bcm2712: CONFIG_EROFS_FS_ZIP_ZSTD is not =y"* ]]; then
    echo "check-devices: self-test: a family with no product was not held to zstd by name: $out" >&2; rc=1
  fi
  rm -rf "$tmp" "$pa" "$pb" "$pc"
  [[ $rc -eq 0 ]] && echo "check-devices: self-test: the erofs rule reads each product's plan: lz4 passes where the recipe default lzma is refused on the same kernel, and a family with no product is held to zstd"
  return $rc
}

# The pad-table provenance rule (PR PADS, 2026-11-16). A family states, in devices/families/<f>/pads.toml, whether its
# bootloader applies a pad table at boot (U-Boot or SPL writes the SoC's iomux table, including the bank-voltage mode):
#   applies_pad_table = true                  (every board of the family states its own pads.toml)
# or, for a family whose pads are set by something other than the bootloader:
#   applies_pad_table = false
#   pad_table_reason = "<why no table applies>"
# A family that states neither is refused: an omitted statement is the mistake this rule stops. The statement is a sidecar
# file, not a key in family.toml, because Gaia refuses an unknown key in a description (deny_unknown_fields).
# A board's table is devices/boards/<b>/pads.toml (a product-local board takes one beside its board.toml), with three
# fields, required where the family says true and checked wherever present:
#   source = "<where the table came from: a schematic or a vendor dts, with its path and sheet or lines>"
#   derived_from_board = ""            (empty: the table is the board's own; a board id: copied from that board, refused)
#   bank_voltages = { IO0_IO1 = "1V8", IO2_IO63 = "3V3" }   (every bank the table drives, stated, not inherited)
# The rule runs over each product's board. Limit: it trusts the family's statement; it does not read the dts.
effective_has_file() {  # <board.toml> <family.toml> <absolute patch path> -> 0 when the effective kernel/bootloader/TF list has it
  # Gaia's append rule (a6299c62 replaced; the pinned rev after this PR appends): the effective list is the family's entries,
  # then the board's, unless the board's [kernel|bootloader|bootloader.trusted_firmware] says inherit_patches = false.
  python3 -I - "$1" "$2" "$3" <<'PY'
import os, sys, tomllib
bp, fp, want = sys.argv[1], sys.argv[2], os.path.normpath(sys.argv[3])
b = tomllib.load(open(bp, "rb")); f = tomllib.load(open(fp, "rb")) if os.path.isfile(fp) else {}
def table(d, path):
    for k in path:
        d = d.get(k, {}) if isinstance(d, dict) else {}
    return d
found = False
for path in (("kernel",), ("bootloader",), ("bootloader", "trusted_firmware")):
    bt, ft = table(b, path), table(f, path)
    inherit = bt.get("inherit_patches", True)
    fam = [(os.path.dirname(fp), e) for e in (ft.get("patches") or [])]
    brd = [(os.path.dirname(bp), e) for e in (bt.get("patches") or [])]
    eff = brd if inherit is False else (fam + brd if "patches" in bt else fam)
    for base, e in eff:
        if isinstance(e, dict) and "file" in e and os.path.normpath(os.path.join(base, e["file"])) == want:
            found = True
sys.exit(0 if found else 1)
PY
}
pads_section() {  # <pads.toml> -> its statement (top-level keys)
  cat "$1" 2>/dev/null
}
board_family_file() {  # <board.toml> -> the family.toml it extends, or nothing
  local f
  f=$(grep -ho '"[^"]*family\.toml"' "$1" | tr -d '"' | head -1)
  [[ -n $f ]] && realpath -m "$(dirname "$1")/$f"
  return 0
}
family_pad_state() {  # <family.toml> -> "true", "false:<reason>", "false:" (no reason), "unstated" or "missing"
  # The statement lives in pads.toml beside family.toml, not in family.toml: Gaia refuses an unknown key in a description
  # (deny_unknown_fields: `bootloader.applies_pad_table: unknown key`), and pads.toml is not a description Gaia reads.
  local pf v reason
  [[ -n $1 && -f $1 ]] || { echo missing; return 0; }
  pf=$(dirname "$1")/pads.toml
  [[ -f $pf ]] || { echo unstated; return 0; }
  v=$(grep -E '^applies_pad_table *=' "$pf" | sed -E 's/^applies_pad_table *= *//' | tr -d ' \t\r' | head -1)
  case $v in
    true) echo true ;;
    false) reason=$(grep -E '^pad_table_reason *=' "$pf" | sed -E 's/^pad_table_reason *= *"(.*)" *$/\1/' | head -1)
           echo "false:$reason" ;;
    *) echo unstated ;;
  esac
}
family_bank_names() {  # <family.toml> <soc> -> the bank names the family lists for that SoC (space-separated), or nothing
  local pf
  pf=$(dirname "$1")/pads.toml
  [[ -f $pf ]] || return 0
  awk -v soc="$2" '
    /^\[banks\./ { s = ($0 == "[banks." soc "]"); next }
    /^\[/ { s = 0 }
    s && /^names *=/ { gsub(/^names *= *"|" *$/, ""); print; exit }' "$pf"
}
io_domain_nodes() {  # <assets dir>... -> "file:line" for each io-domains node definition not marked disabled within 8 lines
  local d f n
  for d in "$@"; do
    [[ -d $d ]] || continue
    while IFS= read -r f; do
      while IFS= read -r n; do
        if ! sed -n "${n},$((n+8))p" "$f" | grep -q 'status *= *"disabled"'; then
          echo "$f:$n"
        fi
      done < <(grep -n -E '(io-domains|io_domains|io-voltage-domain)[^=;]*\{' "$f" | cut -d: -f1)
    done < <(grep -rl -E '(io-domains|io_domains)' "$d" 2>/dev/null)
  done
}
pads_rail_state() {  # <patch> <rail> -> "<rail>=range", "<rail>=fixed:<uV>", "<rail>=open" or "<rail>=absent"
  # Reads the POST-image of a unified diff (the context and added lines of every hunk of each .dts/.dtsi) and finds the
  # regulator node named <rail>. A node whose min and max are equal is fixed; a node with a min and a different max is a
  # range; a node with neither is open (no constraint). A node the patch does not show is absent.
  python3 -I -c '
import re, sys
patch, rail = sys.argv[1], sys.argv[2]
post, cur = {}, None
for line in open(patch, encoding="utf-8"):
    line = line.rstrip("\n")
    if line.startswith("+++ b/"):
        cur = line[6:]; post.setdefault(cur, []); continue
    if cur is None or line.startswith(("--- ", "diff ", "index ", "@@")):
        continue
    if line[:1] in (" ", "+"):
        post[cur].append(line[1:])
head = re.compile(r"(^|\s)" + re.escape(rail) + r" \{")
for f, lines in post.items():
    if not (f.endswith(".dts") or f.endswith(".dtsi")):
        continue
    for i, l in enumerate(lines):
        if not head.search(l):
            continue
        mn = mx = None
        for m in lines[i + 1:]:
            a = re.search(r"regulator-min-microvolt = <(\d+)>", m)
            b = re.search(r"regulator-max-microvolt = <(\d+)>", m)
            if a: mn = int(a.group(1))
            if b: mx = int(b.group(1))
            if re.match(r"^\s*};", m):
                break
        if mn is None and mx is None:
            print(rail + "=open"); sys.exit(0)
        if mn is not None and mn == mx:
            print(rail + "=fixed:" + str(mn)); sys.exit(0)
        print(rail + "=range"); sys.exit(0)
print(rail + "=absent")
' "$1" "$2"
}
pads_bank_value() {  # <inline table line> <key> -> the quoted value of key in it (empty when absent)
  grep -oE "(^|[ {,])$2 *= *\"[^\"]+\"" <<<"$1" | sed -E 's/^[ {,]?[A-Za-z0-9_-]+ *= *"//; s/"$//' | head -1
}
pads_facts_check() {  # <facts file> <bank> <kind> -> a reason when the built tree disagrees with the statement, else nothing
  # kinds: unsupplied (no vcc-p supply in the dts and the driver does not write the bank's mode), no-pin (no pio reference
  # to the bank in the dts), zero-pins (the SoC's pin table gives the bank no pins). Facts are read from the file; a missing
  # fact is a reason, never a pass.
  local f=$1 k=$2 kind=$3 sup io ref pins
  [[ -f $f ]] || { echo "kernel/post-patch-facts.txt is missing"; return 0; }
  case $kind in
    unsupplied)
      sup=$(awk -v k="$k" '($1=="pio-supply"||$1=="rpio-supply") && $2==k {print $3; exit}' "$f")
      io=$(awk -v k="$k" '$1=="io-bias" && $2==k {print $3; exit}' "$f")
      [[ -n $sup ]] || { echo "the facts file has no supply for bank $k"; return 0; }
      [[ $sup == none ]] || echo "the built dts names $sup as bank $k's supply"
      [[ $io == no ]] || echo "the built driver writes bank $k's mode (io-bias ${io:-missing})"
      ;;
    no-pin)
      ref=$(awk -v k="$k" '$1=="pio-ref" && $2==k {print $3; exit}' "$f")
      [[ -n $ref ]] || { echo "the facts file has no pin count for bank $k"; return 0; }
      [[ $ref == 0 ]] || echo "the built dts references $ref pin(s) of bank $k"
      ;;
    zero-pins)
      pins=$(awk -v k="$k" '$1=="bank-pins" && $2==k {print $3; exit}' "$f")
      [[ -n $pins ]] || { echo "the facts file has no pin count for bank $k"; return 0; }
      [[ $pins == 0 ]] || echo "bank $k has $pins pins in the SoC's pin table"
      ;;
    *) echo "unknown mechanical check $kind for bank $k" ;;
  esac
}
pads_rule() {  # <devices dir>
  local dev=$1 p b sec src derived bank name bname ff fam state reason applies soc names key val out=""
  local n k v found bad
  while IFS= read -r p; do
    b=$(product_board "$p")
    [[ -n $b && -f $b ]] || continue
    name=$(basename "$(dirname "$p")")
    bname=$(basename "$(dirname "$b")")
    ff=$(board_family_file "$b")
    [[ -n $ff ]] || continue
    fam=$(basename "$(dirname "$ff")")
    state=$(family_pad_state "$ff")
    case $state in
      missing|unstated)
        out+="FAIL $name: family $fam states no pad-table fact: its pads.toml must say applies_pad_table = true, or applies_pad_table = false with a pad_table_reason"$'\n'
        continue ;;
      false:*)
        reason=${state#false:}
        if [[ -z $reason ]]; then
          out+="FAIL $name: family $fam says it applies no pad table but gives no pad_table_reason in its pads.toml: say why, or set applies_pad_table = true"$'\n'
        fi
        if [[ ! -f $(dirname "$b")/pads.toml ]]; then continue; fi ;;
      true)
        if [[ ! -f $(dirname "$b")/pads.toml ]]; then
          out+="FAIL $name: board $bname: its family's bootloader applies a pad table, and the board states none. Add its pads.toml with soc, source, derived_from_board and bank_voltages"$'\n'
          continue
        fi ;;
    esac
    sec=$(pads_section "$(dirname "$b")/pads.toml")
    # A board may state that no pad or io-domain table is applied, with a reason and its evidence (the way a family does).
    applies=$(grep -E '^applies_pad_table *=' <<<"$sec" | sed -E 's/^applies_pad_table *= *//' | tr -d ' \t\r' | head -1)
    if [[ $applies == false ]]; then
      reason=$(grep -E '^pad_table_reason *=' <<<"$sec" | sed -E 's/^pad_table_reason *= *"(.*)" *$/\1/' | head -1)
      if [[ -z $reason ]]; then
        out+="FAIL $name: board $bname's pads.toml says no pad table is applied but gives no pad_table_reason: state the evidence for both the bootloader and the kernel"$'\n'
      fi
      # The kernel side of the statement, checked against the NODES (the io-domain driver applies nothing without a node). The
      # check reads the dts sources this repository ships for the board and family (their assets/). The in-tree dts the image
      # builds from is not in this repository, so the BUILT dtb is checked by Gaia's built-image check, not here.
      nodes=$(io_domain_nodes "$(dirname "$b")/assets" "$(dirname "$ff")/assets")
      if [[ -n $nodes ]]; then
        out+="FAIL $name: board $bname states no pad table is applied, but a shipped dts enables an io-domains node: ${nodes//$'\n'/; }. The statement is false, so the bank voltages are required"$'\n'
      fi
      # Secondary: the fragments must not set the family's kernel_option (a driver with no node applies nothing; this only
      # shows the option is unset in the fragments, not what the built image does).
      kopt=$(grep -E '^kernel_option *=' "$(dirname "$ff")/pads.toml" 2>/dev/null | sed -E 's/^kernel_option *= *"(.*)" *$/\1/' | head -1)
      if [[ -z $kopt ]]; then
        out+="FAIL $name: family $fam's pads.toml names no kernel_option, so board $bname's \"no table applied\" statement cannot be checked against the kernel"$'\n'
      else
        kfrags=()
        while IFS= read -r f; do kfrags+=("$f"); done < <(fragments_of "$ff"; fragments_of "$b")
        for f in ${kfrags[@]+"${kfrags[@]}"}; do
          [[ -f $f ]] && grep -qE "^CONFIG_${kopt}=y\$" "$f" && out+="FAIL $name: board $bname states no pad table is applied, but ${f#"$dev/"} sets CONFIG_${kopt}=y: the statement is false, so the bank voltages are required"$'\n'
        done
      fi
      continue
    fi
    src=$(grep -E '^source *=' <<<"$sec" | sed -E 's/^source *= *"(.*)" *$/\1/' | head -1)
    derived=$(grep -E '^derived_from_board *=' <<<"$sec" | sed -E 's/^derived_from_board *= *"(.*)" *$/\1/' | head -1)
    soc=$(grep -E '^soc *=' <<<"$sec" | sed -E 's/^soc *= *"(.*)" *$/\1/' | head -1)
    bank=$(grep -E '^bank_voltages *=' <<<"$sec" | grep -E '\{ *[A-Za-z0-9_-]+ *=' || true)
    if [[ -n $derived ]]; then
      out+="FAIL $name: board $bname's pad table is derived from board $derived, not its own: the bank voltages and pad modes are that board's, and another board's table can damage the chip (\"Please pay attention to the bank voltage! will damage the chip\"). Take the table from this board's own schematic or vendor dts"$'\n'
    fi
    if [[ -z $src ]]; then
      out+="FAIL $name: board $bname's pad table has no provenance: its pads.toml source is empty or missing, so nothing says where the table came from. Add the schematic or vendor dts it was read from"$'\n'
    fi
    if [[ -z $soc ]]; then
      out+="FAIL $name: board $bname's pads.toml names no soc, so the rule cannot know the SoC's full bank list"$'\n'
      continue
    fi
    names=$(family_bank_names "$ff" "$soc")
    if [[ -z $names ]]; then
      out+="FAIL $name: family $fam's pads.toml lists no banks for soc $soc: the family must give the SoC's full bank list ([banks.$soc] names)"$'\n'
      continue
    fi
    if [[ -z $bank ]]; then
      out+="FAIL $name: board $bname's bank voltages are not stated: its pads.toml must name every bank of $soc ($names): a voltage with its source, or \"fixed by hardware, per sheet N\""$'\n'
      continue
    fi
    # Every bank of the SoC must be named, and nothing else: a missing bank and an unknown bank both refuse the image.
    keys=$(grep -oE '[A-Za-z0-9_-]+ *= *"' <<<"$bank" | sed -E 's/ *= *"$//')
    for n in $names; do
      grep -qx -- "$n" <<<"$keys" || out+="FAIL $name: board $bname does not state bank $n of $soc: every bank needs a voltage with its source, or \"fixed by hardware, per sheet N\". The image is refused"$'\n'
    done
    for k in $keys; do
      grep -qw -- "$k" <<<"$names" || out+="FAIL $name: board $bname names bank $k, which is not a bank of $soc ($names)"$'\n'
    done
    # The value forms: a voltage ("1.8V", "3.3V"), or fixed by hardware with the sheet cited (the first two). Three more
    # statements (PR A7RAILS) name a rail that the board's correcting kernel patch leaves unprogrammed (a range):
    #   "set by the bootloader <name@rev> to <V> per <file:line>; not re-programmed by the kernel"
    #   "left as the PMIC powers it up; not programmed by our bootloader or kernel; source: <file> line <N>"
    # Both need the bank's rail (bank_rails = { PE = "bldo1" }) and the correcting patch (correction = "<file>" beside
    # pads.toml); the patch must leave that rail a range, not a fixed voltage. And "no pads: ... 0 pins" for a bank the
    # SoC's own pin table gives no pins (the source must say so).
    corr=$(grep -E '^correction *=' <<<"$sec" | sed -E 's/^correction *= *"(.*)" *$/\1/' | head -1)
    # The correcting patch is a build input (PR A7RAILS, second ruling; PR PATCH-LISTS: the effective list): a pads.toml that
    # names one must find it in the effective list (family, then board), where Gaia applies it after the series. Without
    # the entry the statement cites a patch the build does not carry, so the image is refused.
    if [[ -n $corr ]]; then
      if [[ ! -f $(dirname "$b")/$corr ]]; then
        out+="FAIL $name: board $bname's pads.toml names correction $corr, which is not a file beside the board"$'\n'
      fi
      if ! effective_has_file "$b" "$ff" "$(dirname "$b")/$corr"; then
        out+="FAIL $name: board $bname's pads.toml names correction $corr, but its effective kernel patch list (the family's entries, then the board's) does not include it: the statement cites a patch the build does not include. List { file = \"$corr\" } on the board or the family"$'\n'
      fi
    fi
    rails=$(grep -E '^bank_rails *=' <<<"$sec" || true)
    checks=$(grep -E '^bank_checks *=' <<<"$sec" || true)
    while IFS= read -r kv; do
      k=${kv%%=*}; k=${k// /}; v=${kv#*=}; v=${v#*\"}; v=${v%\"*}
      if [[ $v =~ ^[0-9.]+V$ ]] || [[ $v =~ ^[0-9.]+V\ to\ [0-9.]+V ]] || [[ $v =~ ^fixed\ by\ hardware,\ per\ sheet\ [0-9]+ ]]; then :
      elif [[ $v == left\ as\ the\ PMIC\ powers\ it\ up\;*source:* ]] \
        && [[ -z $(pads_bank_value "$rails" "$k") ]]; then
        # Left as the PMIC powers it up, with no rail named: the rail is unknown or conflicting (PD, PI, PJ). It is accepted
        # only with a mechanical check (bank_checks) that the built tree agrees, read from kernel/post-patch-facts.txt.
        fk=$(pads_bank_value "$checks" "$k")
        if [[ -z $fk ]]; then
          out+="FAIL $name: board $bname's bank $k is left as the PMIC powers it up with no rail, but its pads.toml bank_checks names no mechanical check for $k"$'\n'
        else
          why=$(pads_facts_check "$(dirname "$b")/kernel/post-patch-facts.txt" "$k" "$fk")
          [[ -z $why ]] || out+="FAIL $name: board $bname's bank $k statement is false: $why. The image is refused"$'\n'
        fi
      elif [[ $v =~ ^set\ by\ the\ bootloader\ .+\ to\ [0-9.]+V\ per\ [^\;]+\;\ not\ re-programmed\ by\ the\ kernel$ ]] \
        || [[ $v == left\ as\ the\ PMIC\ powers\ it\ up\;*source:* ]]; then
        rail=$(pads_bank_value "$rails" "$k")
        if [[ -z $rail ]]; then
          out+="FAIL $name: board $bname's bank $k states a rail, but its pads.toml bank_rails names no rail for $k"$'\n'
        elif [[ -z $corr || ! -f $(dirname "$b")/$corr ]]; then
          out+="FAIL $name: board $bname's bank $k leaves rail $rail unprogrammed, but the board's correcting kernel patch (correction in pads.toml) is missing"$'\n'
        else
          st=$(pads_rail_state "$(dirname "$b")/$corr" "$rail")
          case $st in
            "$rail=range"|"$rail=open") ;;
            "$rail=fixed"*) out+="FAIL $name: board $bname's bank $k says rail $rail is left unprogrammed, but $corr still pins it to a fixed voltage (${st#*=fixed:} microvolts): the statement is false, the image is refused"$'\n' ;;
            *) out+="FAIL $name: board $bname's bank $k: rail $rail is not in $corr, so the patch cannot be checked against the statement"$'\n' ;;
          esac
        fi
      elif [[ $v =~ ^no\ pads:\ .* ]]; then
        # No pads: the bank has zero pins in the SoC's own pin table (bank-pins in kernel/post-patch-facts.txt).
        why=$(pads_facts_check "$(dirname "$b")/kernel/post-patch-facts.txt" "$k" zero-pins)
        [[ -z $why ]] || out+="FAIL $name: board $bname's bank $k says no pads: $why. The image is refused"$'\n'
      else
        out+="FAIL $name: board $bname's bank $k value \"$v\" is neither a voltage (for example 1.8V), \"fixed by hardware, per sheet N\", nor a stated rail or no-pads form"$'\n'
      fi
    done < <(grep -oE '[A-Za-z0-9_-]+ *= *"[^"]*"' <<<"$bank")
    # A range is accepted only for a bank that a named part switches, and only with both endpoints (1.8V and 3.3V).
    if grep -qE '"[0-9.]+V to [0-9.]+V' <<<"$bank"; then
      if ! grep -qE '^switched_by *= *"[^"]+"' <<<"$sec"; then
        out+="FAIL $name: board $bname's bank_voltages gives a range but pads.toml has no switched_by: a range is accepted only with the part that switches it (for example the SD voltage regulator)"$'\n'
      fi
      if ! grep -q '1\.8' <<<"$bank" || ! grep -q '3\.3' <<<"$bank"; then
        out+="FAIL $name: board $bname's bank range does not name both endpoints 1.8V and 3.3V"$'\n'
      fi
    fi
  done < <(find "$dev/products" -mindepth 2 -maxdepth 2 -name product.toml 2>/dev/null | sort)
  printf '%s' "$out"
  [[ -z $out ]]
}
# Proves the pad rule on scratch fixtures: a flagged family's board with its own sourced and stated table passes, a board of a
# family that says false with a reason and no table passes, and it refuses each of the other cases, naming the board and reason.
pads_self_test() {
  local tmp rc=0 out
  tmp=$(mktemp -d)
  # mk <dir> <family state: true|false-reason|false-noreason|unstated|true-nobanks> <board pads.toml body, empty for none>
  mk() {
    mkdir -p "$1/devices/families/fam" "$1/devices/boards/bd" "$1/devices/products/pd"
    rm -f "$1/devices/families/fam/pads.toml" "$1/devices/boards/bd/pads.toml"
    printf 'version = 1\nname = "fam"\n' > "$1/devices/families/fam/family.toml"
    case $2 in
      true) printf 'applies_pad_table = true\n\n[banks.tst]\nnames = "A B"\n' > "$1/devices/families/fam/pads.toml" ;;
      true-nobanks) printf 'applies_pad_table = true\n' > "$1/devices/families/fam/pads.toml" ;;
      false-reason) printf 'applies_pad_table = false\npad_table_reason = "pads are set by the test firmware; not applicable"\n' > "$1/devices/families/fam/pads.toml" ;;
      false-noreason) printf 'applies_pad_table = false\n' > "$1/devices/families/fam/pads.toml" ;;
      *) : ;;
    esac
    printf 'version = 1\nextends = "../../families/fam/family.toml"\nname = "bd"\n' > "$1/devices/boards/bd/board.toml"
    [[ -n $3 ]] && printf '%s\n' "$3" > "$1/devices/boards/bd/pads.toml"
    printf 'version = 1\nname = "pd"\n\n[[images]]\nname = "pd"\nboard = "../../boards/bd/board.toml"\nrecipe = "sd-image"\n' > "$1/devices/products/pd/product.toml"
  }
  mk "$tmp/own" true 'soc = "tst"
source = "test schematic, sheet 4"
derived_from_board = ""
bank_voltages = { A = "1.8V", B = "fixed by hardware, per sheet 4" }'
  if ! out=$(pads_rule "$tmp/own/devices"); then echo "check-devices: self-test: a board that names every bank with a voltage or a sheet was refused: $out" >&2; rc=1; fi
  mk "$tmp/derived" true 'soc = "tst"
source = "SDK BPI dts"
derived_from_board = "bpi-canmv-k230d-zero"
bank_voltages = { A = "1.8V", B = "1.8V" }'
  if out=$(pads_rule "$tmp/derived/devices"); then echo "check-devices: self-test: a pad table derived from another board was not refused" >&2; rc=1
  elif [[ $out != *"derived from board bpi-canmv-k230d-zero"* || $out != *"bd"* ]]; then echo "check-devices: self-test: the derived refusal does not name the board and its origin: $out" >&2; rc=1; fi
  mk "$tmp/noprov" true 'soc = "tst"
source = ""
derived_from_board = ""
bank_voltages = { A = "1.8V", B = "1.8V" }'
  if out=$(pads_rule "$tmp/noprov/devices"); then echo "check-devices: self-test: a pad table with no provenance was not refused" >&2; rc=1
  elif [[ $out != *"no provenance"* || $out != *"bd"* ]]; then echo "check-devices: self-test: the provenance refusal does not name the board and reason: $out" >&2; rc=1; fi
  mk "$tmp/missingbank" true 'soc = "tst"
source = "test schematic, sheet 4"
derived_from_board = ""
bank_voltages = { A = "1.8V" }'
  if out=$(pads_rule "$tmp/missingbank/devices"); then echo "check-devices: self-test: a board that leaves a bank of its SoC unstated was not refused" >&2; rc=1
  elif [[ $out != *"does not state bank B"* || $out != *"board bd"* ]]; then echo "check-devices: self-test: the missing-bank refusal does not name the board and bank: $out" >&2; rc=1; fi
  mk "$tmp/unknownbank" true 'soc = "tst"
source = "test schematic, sheet 4"
derived_from_board = ""
bank_voltages = { A = "1.8V", B = "3.3V", C = "1.8V" }'
  if out=$(pads_rule "$tmp/unknownbank/devices"); then echo "check-devices: self-test: a board that names a bank its SoC does not have was not refused" >&2; rc=1
  elif [[ $out != *"names bank C"* || $out != *"board bd"* ]]; then echo "check-devices: self-test: the unknown-bank refusal does not name the board and bank: $out" >&2; rc=1; fi
  mk "$tmp/badvalue" true 'soc = "tst"
source = "test schematic, sheet 4"
derived_from_board = ""
bank_voltages = { A = "1.8V", B = "maybe" }'
  if out=$(pads_rule "$tmp/badvalue/devices"); then echo "check-devices: self-test: a bank value that is neither a voltage nor fixed by hardware was not refused" >&2; rc=1
  elif [[ $out != *"neither a voltage"* || $out != *"board bd"* ]]; then echo "check-devices: self-test: the value refusal does not name the board and reason: $out" >&2; rc=1; fi
  mk "$tmp/missing" true ''
  if out=$(pads_rule "$tmp/missing/devices"); then echo "check-devices: self-test: a flagged family's board with no pads.toml was not refused" >&2; rc=1
  elif [[ $out != *"states none"* || $out != *"board bd"* ]]; then echo "check-devices: self-test: the missing-table refusal does not name the board and reason: $out" >&2; rc=1; fi
  mk "$tmp/nobanks" true-nobanks 'soc = "tst"
source = "test schematic, sheet 4"
derived_from_board = ""
bank_voltages = { A = "1.8V" }'
  if out=$(pads_rule "$tmp/nobanks/devices"); then echo "check-devices: self-test: a family that lists no banks for the board's SoC was not refused" >&2; rc=1
  elif [[ $out != *"lists no banks"* ]]; then echo "check-devices: self-test: the no-banks refusal does not name the reason: $out" >&2; rc=1; fi
  mk "$tmp/bdfalse" true 'applies_pad_table = false
pad_table_reason = "no pad or io-domain table is applied (test evidence)"'
  printf 'kernel_option = "FAKE_IODOMAIN"\n' >> "$tmp/bdfalse/devices/families/fam/pads.toml"
  if ! out=$(pads_rule "$tmp/bdfalse/devices"); then echo "check-devices: self-test: a board that states no pad table is applied, with a reason, was refused: $out" >&2; rc=1; fi
  mk "$tmp/bdnoreason" true 'applies_pad_table = false'
  if out=$(pads_rule "$tmp/bdnoreason/devices"); then echo "check-devices: self-test: a board that states no pad table is applied with no reason was not refused" >&2; rc=1
  elif [[ $out != *"no pad_table_reason"* || $out != *"board bd"* ]]; then echo "check-devices: self-test: the board no-reason refusal does not name the board and reason: $out" >&2; rc=1; fi
  mk "$tmp/rangeno" true 'soc = "tst"
source = "test schematic, sheet 4"
derived_from_board = ""
bank_voltages = { A = "1.8V to 3.3V (sd regulator, run-time)", B = "3.3V" }'
  if out=$(pads_rule "$tmp/rangeno/devices"); then echo "check-devices: self-test: a bank range with no switched_by was not refused" >&2; rc=1
  elif [[ $out != *"no switched_by"* || $out != *"board bd"* ]]; then echo "check-devices: self-test: the range refusal does not name the board and reason: $out" >&2; rc=1; fi
  mk "$tmp/rangeok" true 'soc = "tst"
source = "test schematic, sheet 4"
derived_from_board = ""
bank_voltages = { A = "1.8V to 3.3V (sd regulator, run-time)", B = "3.3V" }
switched_by = "A: the SD card voltage regulator, 1.8V and 3.3V"'
  if ! out=$(pads_rule "$tmp/rangeok/devices"); then echo "check-devices: self-test: a bank range with switched_by and both endpoints was refused: $out" >&2; rc=1; fi
  # The rail statements (PR A7RAILS): a rail the correcting patch leaves a range passes; the same statement on a rail the
  # patch still pins to a fixed voltage is refused; a rail statement with no correcting patch is refused; "no pads" needs 0 pins.
  railpatch='--- a/x.dts
+++ b/x.dts
@@ -1,4 +1,4 @@
 reg_x: bldo2 {
-regulator-min-microvolt = <1800000>;
+regulator-min-microvolt = <500000>;
 regulator-max-microvolt = <3400000>;
 };'
  railfixed='--- a/x.dts
+++ b/x.dts
@@ -1,4 +1,4 @@
 reg_x: bldo2 {
-regulator-min-microvolt = <3300000>;
+regulator-min-microvolt = <1800000>;
 regulator-max-microvolt = <1800000>;
 };'
  railstate='set by the bootloader u-boot@9da4b3e8 to 1.8V per sun60i-a733-cubie-a7s.dts:178-183; not re-programmed by the kernel'
  mk "$tmp/railok" true 'soc = "tst"
source = "test schematic, sheet 4"
derived_from_board = ""
bank_voltages = { A = "'"$railstate"'", B = "fixed by hardware, per sheet 4" }
bank_rails = { A = "bldo2" }
correction = "corr.patch"'
  printf '%s\n' "$railpatch" > "$tmp/railok/devices/boards/bd/corr.patch"
  printf '[kernel]\npatches = [{ file = "corr.patch" }]\n' >> "$tmp/railok/devices/boards/bd/board.toml"
  if ! out=$(pads_rule "$tmp/railok/devices"); then echo "check-devices: self-test: a rail statement on a rail the patch leaves a range was refused: $out" >&2; rc=1; fi
  mk "$tmp/railfixed" true 'soc = "tst"
source = "test schematic, sheet 4"
derived_from_board = ""
bank_voltages = { A = "'"$railstate"'", B = "fixed by hardware, per sheet 4" }
bank_rails = { A = "bldo2" }
correction = "corr.patch"'
  printf '%s\n' "$railfixed" > "$tmp/railfixed/devices/boards/bd/corr.patch"
  printf '[kernel]\npatches = [{ file = "corr.patch" }]\n' >> "$tmp/railfixed/devices/boards/bd/board.toml"
  if out=$(pads_rule "$tmp/railfixed/devices"); then echo "check-devices: self-test: a rail statement on a rail the patch still pins to a fixed voltage was not refused" >&2; rc=1
  elif [[ $out != *"still pins it to a fixed voltage"* ]]; then echo "check-devices: self-test: the fixed-rail refusal does not say so: $out" >&2; rc=1; fi
  # The gap (PR A7RAILS, second ruling): a correction the board does not wire into its kernel patches refuses the image.
  mk "$tmp/railunwired" true 'soc = "tst"
source = "test schematic, sheet 4"
derived_from_board = ""
bank_voltages = { A = "'"$railstate"'", B = "fixed by hardware, per sheet 4" }
bank_rails = { A = "bldo2" }
correction = "corr.patch"'
  printf '%s\n' "$railpatch" > "$tmp/railunwired/devices/boards/bd/corr.patch"
  if out=$(pads_rule "$tmp/railunwired/devices"); then echo "check-devices: self-test: a correction that the board does not wire into its kernel patches was not refused" >&2; rc=1
  elif [[ $out != *"does not include it"* ]]; then echo "check-devices: self-test: the unwired-correction refusal does not say so: $out" >&2; rc=1; fi
  # The effective list (PR PATCH-LISTS): a correction listed on the family counts; a board that opts out with
  # inherit_patches = false does not inherit it, so the same statement is refused.
  mk "$tmp/railfam" true 'soc = "tst"
source = "test schematic, sheet 4"
derived_from_board = ""
bank_voltages = { A = "'"$railstate"'", B = "fixed by hardware, per sheet 4" }
bank_rails = { A = "bldo2" }
correction = "corr.patch"'
  printf '%s\n' "$railpatch" > "$tmp/railfam/devices/boards/bd/corr.patch"
  printf 'version = 1\nname = "fam"\n\n[kernel]\npatches = [{ file = "../../boards/bd/corr.patch" }]\n' > "$tmp/railfam/devices/families/fam/family.toml"
  printf 'version = 1\nextends = "../../families/fam/family.toml"\nname = "bd"\n' > "$tmp/railfam/devices/boards/bd/board.toml"
  out=$(pads_rule "$tmp/railfam/devices") || true
  if [[ -n $out ]]; then echo "check-devices: self-test: a correction listed on the family (in the effective list) was refused: $out" >&2; rc=1; fi
  mk "$tmp/railoptout" true 'soc = "tst"
source = "test schematic, sheet 4"
derived_from_board = ""
bank_voltages = { A = "'"$railstate"'", B = "fixed by hardware, per sheet 4" }
bank_rails = { A = "bldo2" }
correction = "corr.patch"'
  printf '%s\n' "$railpatch" > "$tmp/railoptout/devices/boards/bd/corr.patch"
  printf 'version = 1\nname = "fam"\n\n[kernel]\npatches = [{ file = "../../boards/bd/corr.patch" }]\n' > "$tmp/railoptout/devices/families/fam/family.toml"
  printf 'version = 1\nextends = "../../families/fam/family.toml"\nname = "bd"\n\n[kernel]\ninherit_patches = false\npatches = []\n' > "$tmp/railoptout/devices/boards/bd/board.toml"
  out=$(pads_rule "$tmp/railoptout/devices") || true
  if [[ -z $out || $out != *"does not include it"* ]]; then echo "check-devices: self-test: a board that opts out of the family's list did not refuse its correction: $out" >&2; rc=1; fi
  mk "$tmp/railnopatch" true 'soc = "tst"
source = "test schematic, sheet 4"
derived_from_board = ""
bank_voltages = { A = "left as the PMIC powers it up; not programmed by our bootloader or kernel; source: radxa board.dts line 1369", B = "fixed by hardware, per sheet 4" }
bank_rails = { A = "aldo1" }'
  if out=$(pads_rule "$tmp/railnopatch/devices"); then echo "check-devices: self-test: a rail statement with no correcting patch was not refused" >&2; rc=1; fi
  # The mechanical checks (PR A7RAILS): a bank statement with no rail is checked against kernel/post-patch-facts.txt.
  # facts <dir> <lines>: writes the facts file of the fixture board (the same name the real board uses).
  facts() { mkdir -p "$1/devices/boards/bd/kernel"; printf '%s\n' "$2" > "$1/devices/boards/bd/kernel/post-patch-facts.txt"; }
  nopadstate='no pads: pinctrl-sun60i-a733.c a_nr_bank_pins gives A 0 pins; source: kernel/post-patch-facts.txt'
  leftstate='left as the PMIC powers it up; supply BLDO2 or SWOUT1, the sources conflict (both cited); neither programmed by our kernel; source: kernel/post-patch-facts.txt'
  mk "$tmp/nopads" true 'soc = "tst"
source = "A733 driver bank table"
derived_from_board = ""
bank_voltages = { A = "'"$nopadstate"'", B = "fixed by hardware, per sheet 4" }
bank_checks = { A = "zero-pins" }'
  facts "$tmp/nopads" 'bank-pins A 0'
  if ! out=$(pads_rule "$tmp/nopads/devices"); then echo "check-devices: self-test: a no-pads bank with 0 pins was refused: $out" >&2; rc=1; fi
  mk "$tmp/nopadsbad" true 'soc = "tst"
source = "A733 driver bank table"
derived_from_board = ""
bank_voltages = { A = "'"$nopadstate"'", B = "fixed by hardware, per sheet 4" }
bank_checks = { A = "zero-pins" }'
  facts "$tmp/nopadsbad" 'bank-pins A 3'
  if out=$(pads_rule "$tmp/nopadsbad/devices"); then echo "check-devices: self-test: a no-pads bank that has pins was not refused" >&2; rc=1; fi
  mk "$tmp/leftok" true 'soc = "tst"
source = "test schematic, sheet 4"
derived_from_board = ""
bank_voltages = { A = "'"$leftstate"'", B = "fixed by hardware, per sheet 4" }
bank_checks = { A = "unsupplied" }'
  facts "$tmp/leftok" 'pio-supply A none
io-bias A no'
  if ! out=$(pads_rule "$tmp/leftok/devices"); then echo "check-devices: self-test: a left-as-powered bank with no supply and no mode write was refused: $out" >&2; rc=1; fi
  mk "$tmp/leftsup" true 'soc = "tst"
source = "test schematic, sheet 4"
derived_from_board = ""
bank_voltages = { A = "'"$leftstate"'", B = "fixed by hardware, per sheet 4" }
bank_checks = { A = "unsupplied" }'
  facts "$tmp/leftsup" 'pio-supply A swout1
io-bias A no'
  if out=$(pads_rule "$tmp/leftsup/devices"); then echo "check-devices: self-test: a left-as-powered bank whose built dts names a supply was not refused" >&2; rc=1
  elif [[ $out != *"names swout1 as bank A's supply"* ]]; then echo "check-devices: self-test: the supply refusal does not say so: $out" >&2; rc=1; fi
  mk "$tmp/leftio" true 'soc = "tst"
source = "test schematic, sheet 4"
derived_from_board = ""
bank_voltages = { A = "'"$leftstate"'", B = "fixed by hardware, per sheet 4" }
bank_checks = { A = "unsupplied" }'
  facts "$tmp/leftio" 'pio-supply A none
io-bias A yes'
  if out=$(pads_rule "$tmp/leftio/devices"); then echo "check-devices: self-test: a left-as-powered bank whose driver writes its mode was not refused" >&2; rc=1; fi
  mk "$tmp/nopinbad" true 'soc = "tst"
source = "test schematic, sheet 4"
derived_from_board = ""
bank_voltages = { A = "left as the PMIC powers it up; no supply in any source; no pin of bank A is referenced; source: kernel/post-patch-facts.txt", B = "fixed by hardware, per sheet 4" }
bank_checks = { A = "no-pin" }'
  facts "$tmp/nopinbad" 'pio-ref A 2'
  if out=$(pads_rule "$tmp/nopinbad/devices"); then echo "check-devices: self-test: a bank the dts references was accepted as no-pin" >&2; rc=1; fi
  mk "$tmp/nochk" true 'soc = "tst"
source = "test schematic, sheet 4"
derived_from_board = ""
bank_voltages = { A = "'"$leftstate"'", B = "fixed by hardware, per sheet 4" }'
  facts "$tmp/nochk" 'pio-supply A none
io-bias A no'
  if out=$(pads_rule "$tmp/nochk/devices"); then echo "check-devices: self-test: a left-as-powered bank with no mechanical check was not refused" >&2; rc=1; fi
  mk "$tmp/nofacts" true 'soc = "tst"
source = "test schematic, sheet 4"
derived_from_board = ""
bank_voltages = { A = "'"$leftstate"'", B = "fixed by hardware, per sheet 4" }
bank_checks = { A = "unsupplied" }'
  if out=$(pads_rule "$tmp/nofacts/devices"); then echo "check-devices: self-test: a left-as-powered bank with no facts file was not refused" >&2; rc=1; fi
  mk "$tmp/ffalse" false-reason ''
  if ! out=$(pads_rule "$tmp/ffalse/devices"); then echo "check-devices: self-test: a family that says no pad table applies, with a reason, was refused: $out" >&2; rc=1; fi
  # The kernel side of a "no table applied" board statement: its fragments must not set the family's kernel_option.
  mk "$tmp/kclean" false-reason ''
  printf 'kernel_option = "FAKE_IODOMAIN"\n' >> "$tmp/kclean/devices/families/fam/pads.toml"
  printf 'applies_pad_table = false\npad_table_reason = "no table (test)"\n' > "$tmp/kclean/devices/boards/bd/pads.toml"
  printf 'version = 1\nname = "fam"\nfragments = ["assets/k.fragment"]\n' > "$tmp/kclean/devices/families/fam/family.toml"
  mkdir -p "$tmp/kclean/devices/families/fam/assets"; printf '# CONFIG_FAKE_IODOMAIN is not set\n' > "$tmp/kclean/devices/families/fam/assets/k.fragment"
  if ! out=$(pads_rule "$tmp/kclean/devices"); then echo "check-devices: self-test: a board stating no table applies, with the kernel option unset, was refused: $out" >&2; rc=1; fi
  mk "$tmp/kset" false-reason ''
  printf 'kernel_option = "FAKE_IODOMAIN"\n' >> "$tmp/kset/devices/families/fam/pads.toml"
  printf 'applies_pad_table = false\npad_table_reason = "no table (test)"\n' > "$tmp/kset/devices/boards/bd/pads.toml"
  printf 'version = 1\nname = "fam"\nfragments = ["assets/k.fragment"]\n' > "$tmp/kset/devices/families/fam/family.toml"
  mkdir -p "$tmp/kset/devices/families/fam/assets"; printf 'CONFIG_FAKE_IODOMAIN=y\n' > "$tmp/kset/devices/families/fam/assets/k.fragment"
  if out=$(pads_rule "$tmp/kset/devices"); then echo "check-devices: self-test: a no-table statement with the kernel option set was not refused" >&2; rc=1
  elif [[ $out != *"sets CONFIG_FAKE_IODOMAIN=y"* || $out != *"board bd"* ]]; then echo "check-devices: self-test: the kernel-option refusal does not name the board and fragment: $out" >&2; rc=1; fi
  # The node side: a shipped dts that enables an io-domains node makes the "no table applied" statement false; a disabled one is fine.
  mk "$tmp/nodeon" true 'applies_pad_table = false
pad_table_reason = "no table (test)"'
  printf 'kernel_option = "FAKE_IODOMAIN"\n' >> "$tmp/nodeon/devices/families/fam/pads.toml"
  mkdir -p "$tmp/nodeon/devices/boards/bd/assets"; printf '&pinctrl {\n\tpmu_io_domains: io-domains {\n\t\tstatus = "okay";\n\t};\n};\n' > "$tmp/nodeon/devices/boards/bd/assets/board.dtsi"
  if out=$(pads_rule "$tmp/nodeon/devices"); then echo "check-devices: self-test: a no-table statement with an enabled io-domains node was not refused" >&2; rc=1
  elif [[ $out != *"enables an io-domains node"* || $out != *"board bd"* ]]; then echo "check-devices: self-test: the node refusal does not name the board and node: $out" >&2; rc=1; fi
  mk "$tmp/nodeoff" true 'applies_pad_table = false
pad_table_reason = "no table (test)"'
  printf 'kernel_option = "FAKE_IODOMAIN"\n' >> "$tmp/nodeoff/devices/families/fam/pads.toml"
  mkdir -p "$tmp/nodeoff/devices/boards/bd/assets"; printf '&pinctrl {\n\tpmu_io_domains: io-domains {\n\t\tstatus = "disabled";\n\t};\n};\n' > "$tmp/nodeoff/devices/boards/bd/assets/board.dtsi"
  if ! out=$(pads_rule "$tmp/nodeoff/devices"); then echo "check-devices: self-test: a no-table statement with a disabled io-domains node was refused: $out" >&2; rc=1; fi
  mk "$tmp/fnoreason" false-noreason ''
  if out=$(pads_rule "$tmp/fnoreason/devices"); then echo "check-devices: self-test: a family that says no pad table applies with no reason was not refused" >&2; rc=1
  elif [[ $out != *"no pad_table_reason"* || $out != *"fam"* ]]; then echo "check-devices: self-test: the no-reason refusal does not name the family and reason: $out" >&2; rc=1; fi
  mk "$tmp/unstated" unstated ''
  if out=$(pads_rule "$tmp/unstated/devices"); then echo "check-devices: self-test: a family that states no pad-table fact was not refused" >&2; rc=1
  elif [[ $out != *"states no pad-table fact"* || $out != *"fam"* ]]; then echo "check-devices: self-test: the unstated refusal does not name the family and reason: $out" >&2; rc=1; fi
  rm -rf "$tmp"
  [[ $rc -eq 0 ]] && echo "check-devices: self-test: the pad rule passes a board that names every bank of its SoC with a voltage or a sheet, and refuses a derived, unsourced, missing, partial or unknown bank, a bad value, a family with no banks, an unreasoned or unstated family, and a range without switched_by"
  return $rc
}

# The RK35xx U-Boot environment rule (Z2, the coordinator's HARD RULE of 2026-11-14): no RK slot image ships with an
# as-built U-Boot environment. A product on a slot recipe whose board extends an rk35xx family must have, in the U-Boot
# fragments that board builds with (the board's own [bootloader] fragments when it names some, else the family's), the lines
# CONFIG_ENV_MMC_USE_DT=y (no raw-offset fallback over the partition table) and CONFIG_ENV_MMC_DEVICE_INDEX=1 (the SD card is
# U-Boot's mmc 1), and the family's table must carry the uboot-env partition (16M, 128K) the environment lives in.
# Prints one FAIL line per problem and returns 1 when any. Reads the descriptions only (no Gaia run).
RK_ENV_RECIPES="slot|slot-boot|slot-disk"
bootloader_fragments() {  # <family.toml or board.toml> -> the [bootloader] fragments, as paths
  local f=$1
  awk '/^\[bootloader\]/{s=1;next} /^\[/{s=0} s && /^fragments = /' "$f" | grep -ho '"[^"]*"' | tr -d '"' \
    | while IFS= read -r x; do realpath -m "$(dirname "$f")/$x"; done
}
rk_env_rule() {  # <devices dir>
  local dev=$1 p b fam famfile frags=() text out="" f
  while IFS= read -r p; do
    grep -qE "^recipe = \"($RK_ENV_RECIPES)\"" "$p" || continue
    b=$(product_board "$p")
    [[ -n $b && -f $b ]] || continue
    fam=$(product_family "$p")
    [[ $fam == rk35* ]] || continue
    famfile=$dev/families/$fam/family.toml
    local name; name=$(basename "$(dirname "$p")")
    mapfile -t frags < <(bootloader_fragments "$b")
    [[ ${#frags[@]} -gt 0 ]] || mapfile -t frags < <(bootloader_fragments "$famfile")
    text=""
    for f in ${frags[@]+"${frags[@]}"}; do [[ -f $f ]] && text+=$(grep -v '^[[:space:]]*#' "$f")$'\n'; done
    grep -qx 'CONFIG_ENV_MMC_USE_DT=y' <<<"$text" \
      || out+="FAIL $name: its U-Boot config lacks CONFIG_ENV_MMC_USE_DT=y: without it the environment falls back to the raw offsets over the partition table, so a slot record is lost: add it to the U-Boot fragment (rk35xx family assets/u-boot.fragment)"$'\n'
    grep -qx 'CONFIG_ENV_MMC_DEVICE_INDEX=1' <<<"$text" \
      || out+="FAIL $name: its U-Boot config lacks CONFIG_ENV_MMC_DEVICE_INDEX=1: the SD card is U-Boot's mmc 1 on this family, so the environment would be looked for on the disabled eMMC (mmc 0): add it to the U-Boot fragment"$'\n'
    awk '/name = "uboot-env"/{s=1} s&&/^\[\[/{s=0} s' "$famfile" | grep -q 'offset = "16M"' \
      && awk '/name = "uboot-env"/{s=1} s&&/^\[\[/{s=0} s' "$famfile" | grep -q 'size = "128K"' \
      || out+="FAIL $name: its family $fam has no uboot-env partition at 16M of 128K: the image GPT must carry the environment partition"$'\n'
  done < <(find "$dev/products" -mindepth 2 -maxdepth 2 -name product.toml 2>/dev/null | sort)
  printf '%s' "$out"
  [[ -z $out ]]
}
# Proves the RK env rule passes the real tree and refuses a scratch copy whose rk35xx U-Boot fragment lacks each line.
rk_env_self_test() {
  local tmp rc=0 out
  tmp=$(mktemp -d)
  cp -a "$repo/devices" "$tmp/devices"
  if ! out=$(rk_env_rule "$repo/devices"); then echo "check-devices: self-test: the RK env rule refuses the real tree: $out" >&2; rc=1; fi
  sed -i '/^CONFIG_ENV_MMC_USE_DT=y$/d' "$tmp/devices/families/rk35xx/assets/u-boot.fragment"
  if out=$(rk_env_rule "$tmp/devices"); then echo "check-devices: self-test: a slot image without CONFIG_ENV_MMC_USE_DT=y was not refused" >&2; rc=1
  elif [[ $out != *"CONFIG_ENV_MMC_USE_DT=y"* ]]; then echo "check-devices: self-test: the USE_DT refusal does not name the line: $out" >&2; rc=1; fi
  cp -a "$repo/devices/families/rk35xx/assets/u-boot.fragment" "$tmp/devices/families/rk35xx/assets/u-boot.fragment"
  sed -i '/^CONFIG_ENV_MMC_DEVICE_INDEX=1$/d' "$tmp/devices/families/rk35xx/assets/u-boot.fragment"
  if out=$(rk_env_rule "$tmp/devices"); then echo "check-devices: self-test: a slot image without CONFIG_ENV_MMC_DEVICE_INDEX=1 was not refused" >&2; rc=1
  elif [[ $out != *"CONFIG_ENV_MMC_DEVICE_INDEX=1"* ]]; then echo "check-devices: self-test: the DEVICE_INDEX refusal does not name the line: $out" >&2; rc=1; fi
  rm -rf "$tmp"
  [[ $rc -eq 0 ]] && echo "check-devices: self-test: the RK env rule passes the tree and refuses a slot image missing USE_DT or DEVICE_INDEX"
  return $rc
}

# Prints each description file (family.toml, board.toml, component.toml) under <devices dir> that no product or board
# names, relative to that dir. A product or board names a file by a quoted path ending .toml. Only description files count:
# a board's other .toml files (lemnos-board.toml, lemnos-board.pending.toml) are not descriptions. (PR B32: the note used to
# list every .toml, so the three boards that two products each name were reported as in no product.)
unnamed_descriptions() {  # <devices dir>
  local dev=$1 used f d ref
  used=$(mktemp)
  while IFS= read -r f; do
    d=$(dirname "$dev/$f")
    { grep -ho '"[^"]*\.toml"' "$dev/$f" || true; } | tr -d '"' | while IFS= read -r ref; do
      realpath -m "$d/$ref"
    done
  done < <(cd "$dev" && find boards products -mindepth 2 -maxdepth 2 \( -name board.toml -o -name product.toml \) | sort) >>"$used"
  while IFS= read -r f; do
    grep -qxF "$(realpath -m "$dev/$f")" "$used" || printf '%s\n' "$f"
  done < <(cd "$dev" && find families boards components -mindepth 2 -maxdepth 2 \( -name family.toml -o -name board.toml -o -name component.toml \) | sort)
  rm -f "$used"
}

# Proves the unnamed note: no board that a product names is listed (the bug), and a board that nothing names is.
unnamed_self_test() {
  local tmp rc=0 out p b rel
  out=$(unnamed_descriptions "$repo/devices")
  for p in "$repo"/devices/products/*/product.toml; do
    b=$(product_board "$p"); rel=${b#"$repo/devices/"}
    if grep -qxF "$rel" <<<"$out"; then
      echo "check-devices: self-test: $rel is named by a product but listed as in no product: $out" >&2; rc=1
    fi
  done
  tmp=$(mktemp -d)
  cp -a "$repo/devices" "$tmp/devices"
  mkdir -p "$tmp/devices/boards/zz-unnamed"
  printf 'version = 1\n' > "$tmp/devices/boards/zz-unnamed/board.toml"
  out=$(unnamed_descriptions "$tmp/devices")
  if [[ $out != *"boards/zz-unnamed/board.toml"* ]]; then
    echo "check-devices: self-test: a board that nothing names is not listed: $out" >&2; rc=1
  fi
  rm -rf "$tmp"
  [[ $rc -eq 0 ]] && echo "check-devices: self-test: the in-no-product note skips a board's other .toml files and lists a board nothing names"
  return $rc
}

fixtures=$repo/devices/tests/fixtures

if [[ $mode == self-test ]]; then
  status=0
  for good in good good-f8 good-rust good-sd-image; do
    echo "== $good fixture: must pass"
    "$self" --root "$fixtures/$good" || { echo "check-devices: self-test: $good fixture did not pass" >&2; status=1; }
  done
  lemnos_rule_self_test || status=1
  lemnos_board_rule_self_test || status=1
  bringup_rule_self_test || status=1
  firmware_rule_self_test || status=1
  unnamed_self_test || status=1
  pads_self_test || status=1
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
else
  ensure_bin
  gaia=$dir/bin/gaia
fi
# The binary must be a build of the pinned commit: it prints `gaia <version> (<commit>)`, and that
# commit must be devices/gaia.lock's. A binary that prints no commit (an old Gaia) is refused too.
gaia_line=$("$gaia" --version 2>&1 | head -1 || true)
if [[ ! $gaia_line =~ ^gaia\ [^\ ]+\ \(([^\)]*)\)$ ]]; then
  die "$gaia does not print its commit ('$gaia_line'), so it cannot be checked against $gaia_commit (devices/gaia.lock). Build that commit, or unset GAIA_BIN to let the check build it."
fi
gaia_built=${BASH_REMATCH[1]}
if [[ $gaia_built != "$gaia_commit" ]]; then
  die "$gaia was built from '$gaia_built', but devices/gaia.lock pins $gaia_commit: a stale binary. Rebuild Gaia at $gaia_commit, or unset GAIA_BIN."
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

# ---- the slot root chain (PR B16) ------------------------------------------------
# A slot's root is opened by lemnos-slot-init before any module can be loaded from a root filesystem
# (Lemnos docs/slots.md: the initramfs loads dm modules only from /lib/modules, and Lemnos is removing
# that loader). So on every slot-booting family, the chain below is built in (=y) in the family's own
# kernel fragments, and no fragment of that family's boards may set one of them to m or "not set".
# The list lives here, once. "*" applies to every slot family; "FAMILY:" applies to that family only.
#   sha256: CRYPTO_SHA256 is dm-verity's hash (the verity table names sha256); the storage host is the
#   controller the root and data partitions sit on (the Raze: the CM5 eMMC host; the RK35xx: the DW MMC).
#   MMC_BLOCK: the block layer over MMC. Without it a host driver finds the card and no block device exists.
#   "*" is right while both slot families boot from MMC; an NVMe-booting family would name BLK_DEV_NVME instead.
#   The RK35xx boards have two storage hosts: the SD slot (DesignWare MMC) and the eMMC socket (DWCMSHC, the
#   rk3588 compatible; the RK3576's rk3576-dwcmshc binds the same driver through its fallback). Both are built in.
SLOT_CHAIN=(
  "*:BLK_DEV_DM" "*:DM_BUFIO" "*:DM_VERITY" "*:BLK_DEV_LOOP" "*:EROFS_FS" "*:EXT4_FS" "*:CRYPTO_SHA256"
  # The slot root is zstd-compressed EROFS (Gaia's slot recipes): without the decompressor built in it does not mount.
  "*:EROFS_FS_ZIP" "*:EROFS_FS_ZIP_ZSTD"
  "*:MMC_BLOCK"
  "sun60i:MMC_SUNXI" "bcm2712:MMC_SDHCI_BRCMSTB"
  # the Pi firmware mailbox slot-init uses for RPIBOOT (kernel contract version 2, PR B29)
  "bcm2712:BRCM_CHAR_DRIVERS" "bcm2712:MAILBOX" "bcm2712:BCM2835_MBOX" "bcm2712:BCM_VCIO" "bcm2712:RASPBERRYPI_FIRMWARE"
  "rk35xx:MMC_DW_ROCKCHIP" "rk35xx:MMC_SDHCI_OF_DWCMSHC"
)
# SLOT_RECIPES (the recipes that boot a slot) is defined with the Cargo pin rule, near the top.
# The LZMA decompression stream count a slot family's kernel states (see the rule in slot_chain_check).
LZMA_STREAMS=4

# Textual check: this reads the fragment files, not a resolved kernel config (the script has no kernel
# tree). The resolved-config proof is the table in docs/prs/B16.md, from a merge_config run per board.
# Usage: slot_chain_check <devices dir>. Prints one FAIL line per problem and returns 1 if any.
slot_chain_check() {
  local dev=$1 rc=0 fam sym want val f b p recipe fragfile
  local families=() boards=()
  # slot families: the family of every board that a slot-recipe product names
  while IFS= read -r p; do
    grep -qE "^recipe = \"($SLOT_RECIPES)\"" "$p" || continue
    local bpath
    bpath=$(grep -ho '"[^"]*board\.toml"' "$p" | tr -d '"' | head -1)
    [[ -n $bpath ]] || continue
    b=$(realpath -m "$(dirname "$p")/$bpath")
    fam=$(grep -ho '"[^"]*family\.toml"' "$b" | tr -d '"' | head -1)
    [[ -n $fam ]] || continue
    fam=$(basename "$(dirname "$(realpath -m "$(dirname "$b")/$fam")")")
    [[ " ${families[*]-} " == *" $fam "* ]] || families+=("$fam")
  done < <(find "$dev/products" -mindepth 2 -maxdepth 2 -name product.toml 2>/dev/null | sort)
  for fam in ${families[@]+"${families[@]}"}; do
    local famfile=$dev/families/$fam/family.toml
    # the family's own kernel fragments, then every board's kernel fragments that extends it
    local frags=()
    while IFS= read -r f; do frags+=("$dev/families/$fam/$f"); done < <(
      grep -E '^fragments = ' "$famfile" | grep -ho '"[^"]*\.fragment"' | tr -d '"')
    local bfiles=()
    while IFS= read -r b; do
      [[ $(grep -c "families/$fam/family.toml" "$b") -gt 0 ]] || continue
      local bdir; bdir=$(dirname "$b")
      while IFS= read -r f; do bfiles+=("$bdir/$f"); done < <(
        grep -E '^fragments = ' "$b" | grep -ho '"[^"]*\.\(fragment\|config\)"' | tr -d '"')
    done < <(find "$dev/boards" -mindepth 2 -maxdepth 2 -name board.toml 2>/dev/null | sort)
    # Each fragment is read once into a table of its lines; the checks below look a line up instead of running a grep per
    # file and symbol (thousands of processes per run, slow under load). A lookup is the old pattern's exact-line match.
    local -A lines_of=()
    local ln
    for f in ${frags[@]+"${frags[@]}"} ${bfiles[@]+"${bfiles[@]}"}; do
      [[ -f $f && -z ${lines_of[$f]+x} ]] || continue
      lines_of[$f]=$'\n'
      while IFS= read -r ln || [[ -n $ln ]]; do lines_of[$f]+="$ln"$'\n'; done <"$f"
    done
    has_line() { [[ ${lines_of[$1]-} == *$'\n'"$2"$'\n'* ]]; }  # <file> <exact line>
    # required: each chain symbol that applies to this family must be =y in one of the family's fragments
    for entry in "${SLOT_CHAIN[@]}"; do
      [[ ${entry%%:*} == "*" || ${entry%%:*} == "$fam" ]] || continue
      sym=${entry#*:}
      local found=""
      for f in ${frags[@]+"${frags[@]}"}; do
        if [[ -f $f ]] && has_line "$f" "CONFIG_${sym}=y"; then found=$f; fi
      done
      if [[ -z $found ]]; then
        echo "check-devices: FAIL families/$fam: CONFIG_$sym is not =y in the family's kernel fragments (found: none). The slot's root is opened before any module can load, so a slot of this family does not boot." >&2
        rc=1
      fi
    done
    # forbidden: no fragment of this family or of its boards may take a chain symbol to m or off
    for f in ${frags[@]+"${frags[@]}"} ${bfiles[@]+"${bfiles[@]}"}; do
      [[ -f $f ]] || continue
      for entry in "${SLOT_CHAIN[@]}"; do
        [[ ${entry%%:*} == "*" || ${entry%%:*} == "$fam" ]] || continue
        sym=${entry#*:}
        if has_line "$f" "CONFIG_${sym}=m" || has_line "$f" "# CONFIG_${sym} is not set"; then
          val=$(grep -oE "^CONFIG_${sym}=m|^# CONFIG_${sym} is not set" "$f" | head -1)
          echo "check-devices: FAIL ${f#$dev/}: $val, but CONFIG_$sym must be =y on a slot family ($fam): the slot's root is opened before any module can load." >&2
          rc=1
        fi
      done
    done
    # stated: a fragment that builds the LZMA decompressor in also states its stream count, as $LZMA_STREAMS.
    # Each stream keeps a dictionary and working memory for the life of the mount, and the kernel's default is
    # one per possible CPU up to 16, so an unstated count follows NR_CPUS. The 64 MB whole-system goal fixes it.
    for f in ${frags[@]+"${frags[@]}"} ${bfiles[@]+"${bfiles[@]}"}; do
      [[ -f $f ]] && has_line "$f" "CONFIG_EROFS_FS_ZIP_LZMA=y" || continue
      val=$(grep -E '^CONFIG_EROFS_FS_ZIP_LZMA_DEFAULT_MAX_STREAMS=' "$f" | head -1)
      if [[ $val != "CONFIG_EROFS_FS_ZIP_LZMA_DEFAULT_MAX_STREAMS=$LZMA_STREAMS" ]]; then
        echo "check-devices: FAIL ${f#$dev/}: CONFIG_EROFS_FS_ZIP_LZMA=y, but the stream count is ${val:-not stated}; a slot family ($fam) states CONFIG_EROFS_FS_ZIP_LZMA_DEFAULT_MAX_STREAMS=$LZMA_STREAMS (each stream holds memory for the life of the mount; the kernel default follows NR_CPUS; the system's goal is 64 MB of RAM)." >&2
        rc=1
      fi
    done
  done
  return $rc
}

# Proves the slot root chain check catches (1) a board fragment that sets a chain symbol to m, and (2) a family
# fragment that lacks a required =y, both on a scratch copy of devices/; and passes on the real tree.
slot_chain_self_test() {
  local tmp rc=0
  tmp=$(mktemp -d)
  cp -a "$repo/devices" "$tmp/devices"
  echo "CONFIG_DM_VERITY=m" >> "$tmp/devices/boards/raze/kernel/raze.config"
  slot_chain_check "$tmp/devices" >/dev/null 2>&1 && rc=1
  cp -a "$repo/devices/." "$tmp/devices/"
  sed -i '/^CONFIG_EXT4_FS=y$/d' "$tmp/devices/families/rk35xx/assets/linux.fragment"
  slot_chain_check "$tmp/devices" >/dev/null 2>&1 && rc=1
  # (3) the LZMA stream count removed, and (4) raised, in a fragment that builds LZMA in
  cp -a "$repo/devices/." "$tmp/devices/"
  sed -i '/^CONFIG_EROFS_FS_ZIP_LZMA_DEFAULT_MAX_STREAMS=/d' "$tmp/devices/boards/raze/kernel/raze.config"
  slot_chain_check "$tmp/devices" >/dev/null 2>&1 && rc=1
  cp -a "$repo/devices/." "$tmp/devices/"
  sed -i 's/^CONFIG_EROFS_FS_ZIP_LZMA_DEFAULT_MAX_STREAMS=.*/CONFIG_EROFS_FS_ZIP_LZMA_DEFAULT_MAX_STREAMS=16/' "$tmp/devices/boards/raze/kernel/raze.config"
  slot_chain_check "$tmp/devices" >/dev/null 2>&1 && rc=1
  rm -rf "$tmp"
  [[ $rc -eq 0 ]] && slot_chain_check "$repo/devices" >/dev/null 2>&1 || rc=1
  return $rc
}

# ---- the tree ------------------------------------------------------------------

[[ -d $root ]] || die "no such directory: $root"
root=$(cd "$root" && pwd)
cd "$root"
regen="scripts/check-devices.sh --write-locks"
[[ $root_given -eq 0 ]] || regen="$regen --root $root"
fail=0
bad() { echo "check-devices: FAIL $*" >&2; fail=1; }
# A rule prints its findings and returns 1 with them. A rule that returns non-zero with NOTHING printed did not run
# (an awk or grep error): that is a failure too, never a pass (G10; `$(rule) || true` passed it).
rule_ran() {  # <rule name> <status> <output>
  local rest=${3//[[:space:]]/}
  if [[ $2 -ne 0 && -z $rest ]]; then bad "$1 stopped with status $2 and gave no reason: the rule did not run"; fi
}

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
# component that nothing names would never be checked. Every one must be named (unnamed_descriptions).
errs=$(mktemp)
outs=$(mktemp)
unnamed=$(unnamed_descriptions "$root")
unbuilt=""
if [ -n "$unnamed" ]; then
  if [ "$verb" = validate ]; then
    while IFS= read -r f; do unbuilt+=" ${f%/*}"; done <<<"$unnamed"
  else
    while IFS= read -r f; do bad "$f: no product or board names it, so gaia never reads it"; done <<<"$unnamed"
  fi
fi
# The slot root chain is enforced on every run (see slot_chain_check above).
slot_chain_check "$root" || fail=1
# The boot firmware rule (PR B32): a board's Gaia [boot] firmware agrees with its Lemnos board file's record.
firmware_status=0
firmware_msg=$(firmware_rule "$root") || firmware_status=$?
rule_ran firmware_rule "$firmware_status" "$firmware_msg"
while IFS= read -r line; do
  case $line in
    "") ;;
    *) bad "$line" ;;
  esac
done <<<"$firmware_msg"
# The slot id rule: a slot-boot image's id is derived from its version by the one rule (slot_id_for).
slotid_status=0
slotid_msg=$(slot_id_rule "$root") || slotid_status=$?
rule_ran slot_id_rule "$slotid_status" "$slotid_msg"
while IFS= read -r line; do
  case $line in
    "") ;;
    *) bad "$line" ;;
  esac
done <<<"$slotid_msg"
# The boot partition rule: a slot-boot image's boot_size is its board's boot partition.
bootsize_status=0
bootsize_msg=$(boot_size_rule "$root") || bootsize_status=$?
rule_ran boot_size_rule "$bootsize_status" "$bootsize_msg"
while IFS= read -r line; do
  case $line in
    "") ;;
    *) bad "$line" ;;
  esac
done <<<"$bootsize_msg"
# The bring-up rule (PR B31): every bring-up product carries the watchdog and LED signs (tree-wide).
bringup_status=0
bringup_msg=$(bringup_rule "$root") || bringup_status=$?
rule_ran bringup_rule "$bringup_status" "$bringup_msg"
while IFS= read -r line; do
  case $line in
    NOTE\ *) echo "check-devices: $line" ;;
    "") ;;
    *) bad "$line" ;;
  esac
done <<<"$bringup_msg"

# The watchdog owner rule (PR WDOG-OWNER): every slot product has exactly one watchdog owner (a Lemnos hw-watchdog keepalive
# or systemd's RuntimeWatchdogSec, never both or neither). The built trees come from ATLAS_BUILT_TREES ("NAME=DIR ...").
watchdog_status=0
watchdog_msg=$(watchdog_owner_rule "$root" ${ATLAS_BUILT_TREES:-} -- ${ATLAS_BUILT_BOOTLOADERS:-}) || watchdog_status=$?
rule_ran watchdog_owner_rule "$watchdog_status" "$watchdog_msg"
while IFS= read -r line; do
  case $line in
    NOTE\ *) echo "check-devices: $line" ;;
    FINDING\ *) bad "${line#FINDING }" ;;
    "") ;;
    *) bad "$line" ;;
  esac
done <<<"$watchdog_msg"

if [ -n "$unbuilt" ]; then
  echo "check-devices: NOTE in no product yet, validated but never planned:$unbuilt"
fi

# gaia validate stops at the first bad file and names its file and key, so each file gets its own
# run and every bad file is reported. The kind comes from the directory the file sits in.
validate_file() {  # <kind> <file>
  # A product's recipe names are checked against the locked Gaia's recipes/ (validate --recipes).
  local recipes_arg=()
  if [[ $1 == product && -d $recipes ]]; then recipes_arg=(--recipes "$recipes"); fi
  if "$gaia" validate "$2" --kind "$1" ${recipes_arg[@]+"${recipes_arg[@]}"} >"$outs" 2>&1; then
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

# Nothing unique per unit is in an image. Orion's identity, its trust file and its SSH host key, node
# logic's state, and a bring-up product's dropbear host key are made on the board at first boot, on the
# data partition. An image that carried one would give every board flashed from it the same identity,
# and Orion cannot see that from inside. So no description installs a file there: under the data
# partition (the one exception is the empty mount-point marker), under Orion's default state directory,
# or anything named like a host key or a node identity. This covers pending/ too.
state_status=0
state_hits=$(grep -rnE '^[[:space:]]*to[[:space:]]*=[[:space:]]*"(/data/|/var/lib/orion|/etc/dropbear/|[^"]*(host_key|node-identity|\.ed25519)[^"]*")' \
           --include='*.toml' .) || state_status=$?
# grep: 0 = hits, 1 = none, 2 = an error (an unreadable file): an error fails, never passes quietly (G10).
[[ $state_status -le 1 ]] || bad "the per-unit state search failed (grep status $state_status): it did not run"
while IFS= read -r hit; do
  [[ -n $hit ]] || continue
  bad "an image must not carry per-unit state or keys (made on the board at first boot): $hit"
done < <(grep -vE 'to[[:space:]]*=[[:space:]]*"/data/\.mountpoint"' <<<"$state_hits" || true)

# Every product is validated (when gaia can), then its lock is checked, then it is planned.
#
# The plain check (no --write-locks) never asks the network. The committed gaia.lock is copied, and
# `gaia lock --check` must find it matching the descriptions; then `gaia lock` runs on a second copy
# with git and curl set to /bin/false, so any request it would make is an error. The regenerated copy
# must equal the committed lock byte for byte (this also re-hashes every directory source, which
# --check does not). Only then is the product planned, with the committed lock.
#
# --write-locks starts each product's lock from its committed lock and lets gaia lock resolve what that
# lock lacks (the network). A source whose host is unreachable or rate limited is retried a few times,
# then it is a FAILURE: the committed lock is left as it was.
offline_tool=$(command -v false)
locks=$(mktemp -d)
plans=$(mktemp -d)
bench_dir=""
lock_tries="1 2 3"
trap 'rm -rf "$locks" "$plans" "$errs" "$outs" ${stand_in_dir:+"$stand_in_dir"} ${bench_dir:+"$bench_dir"}' EXIT

# Writes the lock $2 for product $1 (the network for what it lacks). Retries a host that is down or
# limiting requests; any other error ends it at once. Returns 0 when the lock was written.
write_lock() {
  local product=$1 lockfile=$2 attempt rc network
  for attempt in $lock_tries; do
    rc=0
    timeout "${LOCK_TRY_SECONDS:-60}" "$gaia" lock "$product" --lock "$lockfile" >"$errs" 2>&1 || rc=$?
    [[ $rc -eq 0 ]] && return 0
    if [[ $rc -eq 124 ]]; then
      echo "gaia lock: timed out after ${LOCK_TRY_SECONDS:-60} s" >>"$errs"
    fi
    network=0
    if [[ $rc -eq 124 ]] || grep -qiE 'unable to access|could not resolve|failed to connect|could not connect|timed out|connection reset|temporary failure|HTTP 429|HTTP 50[0234]|too many requests' "$errs"; then
      network=1
    fi
    [[ $network -eq 1 && $attempt != "${lock_tries##* }" ]] || break
    sleep $((attempt * 5))
  done
  return 1
}

# A throwaway bench public key for the products that take one (see the header). Made once per run,
# in a temp dir; the private half is deleted at once, so only the public key exists.
bench_key_pub() {
  if [[ -z $bench_dir ]]; then
    command -v ssh-keygen >/dev/null || die "ssh-keygen is needed to make the throwaway bench key that bring-up products are planned with"
    bench_dir=$(mktemp -d)
    ssh-keygen -q -t ed25519 -N '' -C atlas-check-throwaway -f "$bench_dir/key" || die "cannot make the throwaway bench key"
    rm -f "$bench_dir/key"
  fi
  echo "$bench_dir/key.pub"
}
for product in $(find products -mindepth 2 -maxdepth 2 -name product.toml 2>/dev/null | sort); do
  pdir=$(dirname "$product")
  committed=$pdir/gaia.lock
  if [[ $verb == validate ]]; then
    validate_file product "$product"
  fi
  lockfile=$locks/$(echo "$product" | tr '/' '_').lock
  if [[ $write_locks -eq 1 ]]; then
    if [[ -f $committed ]]; then cp "$committed" "$lockfile"; else rm -f "$lockfile"; fi
    if ! write_lock "$product" "$lockfile"; then
      bad "$product: gaia lock could not resolve its sources (a host unreachable or rate limited, or a bad description); the committed lock was left as it was:"
      sed 's/^/  /' "$errs" >&2
      continue
    fi
    cp "$lockfile" "$committed"
    echo "check-devices: wrote $root/$committed"
  else
    if [[ ! -f $committed ]]; then
      bad "$product: no gaia.lock beside it; write it with: $regen"
      continue
    fi
    cp "$committed" "$lockfile"
    if ! "$gaia" lock "$product" --lock "$lockfile" --check >"$errs" 2>&1; then
      bad "$product: gaia.lock beside it does not match the descriptions; regenerate with: $regen"
      sed 's/^/  /' "$errs" >&2
      continue
    fi
    cp "$committed" "$lockfile.offline"
    if ! "$gaia" lock "$product" --lock "$lockfile.offline" --tool "git=$offline_tool" --tool "curl=$offline_tool" >"$errs" 2>&1; then
      bad "$product: gaia lock needs the network for this lock (a source is not locked for what its description says, or a directory changed to a source it did not hash); regenerate with: $regen"
      sed 's/^/  /' "$errs" >&2
      continue
    fi
    if ! cmp -s "$lockfile.offline" "$committed"; then
      bad "$product: gaia.lock beside it differs from what gaia lock writes; regenerate with: $regen"
      { diff "$committed" "$lockfile.offline" || true; } | sed 's/^/  /' >&2
      continue
    fi
  fi
  # The Lemnos rule: every Lemnos source in this product's lock is one rev.
  if ! lemnos_msg=$(lemnos_two_revs "$committed" "$(lemnos_one_rev_exempt "$product")"); then
    bad "$product: Lemnos is locked to more than one rev in this product, $lemnos_msg: every Lemnos component of a product must be locked to one rev"
  fi
  # The Lemnos board file rule: a product that carries lemnosd names its board's Lemnos board file (PR B26).
  if grep -q '/lemnosd/component\.toml"' "$product"; then
    if ! lemnos_board_msg=$(lemnos_board_rule "$product"); then
      bad "$product: $lemnos_board_msg"
    fi
  fi
  # A product whose components take a named input (`{ input = NAME }`) is planned with the input given;
  # gaia refuses an input that nothing uses, so only those products get it.
  plan_args=()
  if grep -rqs 'input = "bench-key"' "$pdir"; then
    plan_args=(--input "bench-key=$(bench_key_pub)")
  fi
  # pc-key (zero-3e-bench): the throwaway key with its comment removed by pc-key-strip.sh, as build-product does.
  if grep -rqs 'input = "pc-key"' "$pdir"; then
    pckey_dir=$(mktemp -d)
    ssh-keygen -q -t ed25519 -N '' -C atlas-check-throwaway -f "$pckey_dir/key" && rm -f "$pckey_dir/key"
    "$repo/scripts/pc-key-strip.sh" "$pckey_dir/key.pub" "$pckey_dir/pc-key.pub"
    plan_args+=(--input "pc-key=$pckey_dir/pc-key.pub")
  fi
  # A slot-boot image's version and slot are Gaia --settings, from the rule's TEST value (CHECK_VERSION, not a release).
  mapfile -t slot_args < <(slot_settings_args "$product" "$CHECK_VERSION")
  if [[ ${#slot_args[@]} -gt 0 ]]; then plan_args+=("${slot_args[@]}"); fi
  pjson=$plans/$(basename "$pdir").json
  if "$gaia" plan "$product" --lock "$committed" --recipes "$recipes" "${tool_args[@]}" ${plan_args[@]+"${plan_args[@]}"} --json >"$pjson" 2>"$outs"; then
    echo "check-devices: ok $product: planned, not built (gaia plan; this repo holds no build record)"
  else
    rm -f "$pjson"
    bad "$product: gaia plan failed:"
    sed 's/^/  /' "$outs" >&2
  fi
done

# The slot root rule (PR B32): each product's erofs compression, from its own plan, against the kernel it builds.
erofs_status=0
erofs_msg=$(erofs_rule "$root" "$plans") || erofs_status=$?
rule_ran erofs_rule "$erofs_status" "$erofs_msg"
while IFS= read -r line; do
  case $line in
    NOTE\ *) echo "check-devices: $line" ;;
    FAIL\ *) bad "${line#FAIL }" ;;
    "") ;;
  esac
done <<<"$erofs_msg"

# The pad-table provenance rule (PR PADS): a family that applies a pad table is built only from its boards' own sourced, stated tables.
if ! pads_out=$(pads_rule "$root"); then
  while IFS= read -r line; do [[ -n $line ]] && bad "${line#FAIL }"; done <<<"$pads_out"
fi

# The slot-disk data size and layout rule (Z2; the set the bench boards copy, coordinator 2026-11-16). On a GPT board a
# slot-disk product must (a) hold keep slots' bay sets, the reserve and the journal allowance in its data partition with a
# 10 % margin: data >= 1.10 x (keep x bay + reserve_mb + journal); and (b) fit the whole layout on the smallest card:
# layout end (the first partition's offset + the table's sizes + the backup GPT header) <= the card's usable bytes.
# keep and reserve_mb must be stated in the board's system-slots device (no inherited Lemnos default: 3 and 2048 MiB).
# bay = the sum of the board's enforced bay sizes ([boot] bay_*: refused by name beyond their size, Lemnos install.rs:698).
# The smallest bench card is 8 GB (board.toml's own statement); it is taken as 7.4 GiB usable (7,945,689,497 bytes): a safety
# margin for cards that are smaller than nominal. Journal allowance: 64 MiB, the journal cap when it lands (not yet built).
DATA_CARD_USABLE_BYTES=7945689497
DATA_JOURNAL_BYTES=67108864
DATA_MARGIN_PCT=10
DATA_BACKUP_GPT_BYTES=1048576
to_bytes() {  # <K|M|G suffixed or plain number> -> bytes
  case $1 in
    *K) echo $(( ${1%K} * 1024 )) ;;
    *M) echo $(( ${1%M} * 1048576 )) ;;
    *G) echo $(( ${1%G} * 1073741824 )) ;;
    *) echo "$1" ;;
  esac
}
data_size_rule() {  # <devices dir>
  local dev=$1 p b name size keep reserve bay need typ lem out="" k v sizes_sum=0 off_max=0 total x
  while IFS= read -r p; do
    grep -qE '^recipe = "slot-disk"' "$p" || continue
    b=$(product_board "$p"); [[ -n $b && -f $b ]] || continue
    name=$(basename "$(dirname "$p")")
    typ=$(awk '/^\[\[partitions\]\]/{f=0} /^role = "data"/{f=1} f && /^type = /{print $3; exit}' "$b" | tr -d '"')
    [[ $typ =~ ^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$ ]] || continue
    size=$(awk '/^\[\[partitions\]\]/{f=0} /^role = "data"/{f=1} f && /^size = /{gsub(/"/,"",$3); print $3; exit}' "$b")
    [[ -n $size ]] || { out+="FAIL $name: its board declares no size for the data partition"$'\n'; continue; }
    size=$(to_bytes "$size")
    lem=$(dirname "$b")/lemnos-board.toml
    keep=$(grep -v '^[[:space:]]*#' "$lem" 2>/dev/null | grep -o 'keep = [0-9]*' | head -1 | grep -o '[0-9]*$')
    reserve=$(grep -v '^[[:space:]]*#' "$lem" 2>/dev/null | grep -o 'reserve_mb = [0-9]*' | head -1 | grep -o '[0-9]*$')
    if [[ -z $keep || -z $reserve ]]; then
      out+="FAIL $name: its board's system-slots device does not state keep and reserve_mb explicitly (Lemnos's defaults are 3 and 2048 MiB: a GPT bench states its own)"$'\n'; continue
    fi
    bay=0
    for k in bay_kernel_bytes bay_initramfs_bytes bay_device_tree_bytes bay_cmdline_bytes bay_overlay_bytes; do
      v=$(grep -v '^[[:space:]]*#' "$lem" 2>/dev/null | grep -o "$k = [0-9]*" | head -1 | grep -o '[0-9]*$')
      if [[ -z $v ]]; then out+="FAIL $name: its board file states no $k (the enforced bay sizes the rule reads)"$'\n'; bay=-1; break; fi
      bay=$(( bay + v ))
    done
    (( bay >= 0 )) || continue
    need=$(( (keep * bay + reserve * 1048576 + DATA_JOURNAL_BYTES) * (100 + DATA_MARGIN_PCT) / 100 ))
    if (( size < need )); then
      out+="FAIL $name: data partition $size bytes is below 1.10 x (keep $keep x bays $bay + reserve ${reserve} MiB + journal 64 MiB) = $need bytes: the first install would be refused NO_SPACE"$'\n'
    fi
    # (b) the layout on the smallest card: the largest explicit offset, plus every sized partition, plus the backup GPT header.
    sizes_sum=0; off_max=0
    while IFS= read -r x; do [[ -n $x ]] && sizes_sum=$(( sizes_sum + $(to_bytes "$x") )); done < <(grep -E '^size = ' "$b" | tr -d '"' | awk '{print $3}')
    while IFS= read -r x; do [[ -n $x ]] && { x=$(to_bytes "$x"); (( x > off_max )) && off_max=$x; }; done < <(grep -E '^offset = ' "$b" | tr -d '"' | awk '{print $3}')
    total=$(( off_max + sizes_sum + DATA_BACKUP_GPT_BYTES ))
    if (( total > DATA_CARD_USABLE_BYTES )); then
      out+="FAIL $name: the layout is $total bytes (offset $off_max + partitions $sizes_sum + backup GPT $DATA_BACKUP_GPT_BYTES), above the smallest card's $DATA_CARD_USABLE_BYTES usable bytes (7.4 GiB): it does not fit"$'\n'
    fi
  done < <(find "$dev/products" -mindepth 2 -maxdepth 2 -name product.toml 2>/dev/null | sort)
  printf '%s' "$out"
  [[ -z $out ]]
}

# Self-test: the data size and layout rule passes the tree; refuses a data partition below the minimum, a layout larger than
# the card, and a missing keep.
data_size_self_test() {
  local tmp rc=0 out
  tmp=$(mktemp -d)
  cp -a "$repo/devices" "$tmp/devices"
  if ! out=$(data_size_rule "$tmp/devices"); then echo "check-devices: self-test: the data size rule refuses the real tree: $out" >&2; rc=1; fi
  sed -i 's/^size = "1073741824"$/size = "104857600"/' "$tmp/devices/boards/radxa-zero-3e/board.toml"
  if out=$(data_size_rule "$tmp/devices"); then echo "check-devices: self-test: a 100 MiB data partition was not refused" >&2; rc=1
  elif [[ $out != *"data partition 104857600 bytes is below"* ]]; then echo "check-devices: self-test: the refusal does not name the size: $out" >&2; rc=1; fi
  cp -a "$repo/devices/boards/radxa-zero-3e/board.toml" "$tmp/devices/boards/radxa-zero-3e/board.toml"
  sed -i 's/^size = "1073741824"$/size = "8589934592"/' "$tmp/devices/boards/radxa-zero-3e/board.toml"
  if out=$(data_size_rule "$tmp/devices"); then echo "check-devices: self-test: an 8 GiB data partition (a layout larger than the card) was not refused" >&2; rc=1
  elif [[ $out != *"it does not fit"* ]]; then echo "check-devices: self-test: the layout refusal does not say so: $out" >&2; rc=1; fi
  cp -a "$repo/devices/boards/radxa-zero-3e/board.toml" "$tmp/devices/boards/radxa-zero-3e/board.toml"
  sed -i 's/ keep = 2, reserve_mb = 128,/ reserve_mb = 128,/' "$tmp/devices/boards/radxa-zero-3e/lemnos-board.toml"
  if out=$(data_size_rule "$tmp/devices"); then echo "check-devices: self-test: a slot-disk board without keep was not refused" >&2; rc=1
  elif [[ $out != *"does not state keep and reserve_mb explicitly"* ]]; then echo "check-devices: self-test: the keep refusal does not say so: $out" >&2; rc=1; fi
  rm -rf "$tmp"
  [[ $rc -eq 0 ]] && echo "check-devices: self-test: the data size and layout rule passes the tree and refuses a data partition below the minimum, a layout larger than the card, and a missing keep"
  return $rc
}

# The open-timeout condition (Z2): every rk35xx family fragment builds the watchdog driver in (=y, not =m) and sets the
# open timeout to 30 s, the Raze's value. The timeout only covers the driver probe to slot-init's open, and it runs only
# when the driver is built in (watchdog_dev.c:1088; the coordinator's ruling 2026-11-16). Prints one FAIL line per problem.
open_timeout_rule() {  # <devices dir>
  local dev=$1 f out=""
  for f in "$dev"/families/rk35*/assets/linux.fragment; do
    [[ -f $f ]] || continue
    grep -qx 'CONFIG_DW_WATCHDOG=y' "$f" || out+="FAIL $(basename "$(dirname "$(dirname "$f")")"): its kernel fragment does not build the watchdog driver in (CONFIG_DW_WATCHDOG=y): the open timeout would not run"$'\n'
    grep -qx 'CONFIG_WATCHDOG_OPEN_TIMEOUT=30' "$f" || out+="FAIL $(basename "$(dirname "$(dirname "$f")")"): its kernel fragment does not set CONFIG_WATCHDOG_OPEN_TIMEOUT=30 (the Raze's value)"$'\n'
  done
  printf '%s' "$out"
  [[ -z $out ]]
}

# The fan owner rule (coordinator 2026-11-20, D1 build 15): a board must have one working fan owner. At the pinned
# Lemnos rev, lemnosd's pwm-channel-fan never binds under the real unit (ProtectKernelTunables=yes: the sysfs pwm
# export gets EROFS), so a Lemnos board file that declares one, in a board or a component, fails unless the board also
# carries the kernel fallback: CONFIG_SENSORS_PWM_FAN=y or =m in one of its kernel configs (boards/<id>/kernel/*.config,
# its family's assets/linux.fragment) AND a [boot] config_lines entry that enables a pwm-fan node (`cooling_fan` or a
# dtoverlay whose name has "fan"). Without a PWM fan the fan line's [[lines]] boot = "high" is the owner: the firmware
# drives it high and `lemnos-ctl tree` holds it with a kernel gpio-hog (full speed, by 993's raze-fan overlay: normal
# polarity). A component's board file is checked against the board its [board] id names.
fan_owner_rule() {  # <devices dir>
  python3 -I - "$1" <<'PY'
import glob, os, re, sys, tomllib
dev = sys.argv[1]; rc = 0
files = sorted(glob.glob(os.path.join(dev, "boards", "*", "lemnos-board.toml")) +
               glob.glob(os.path.join(dev, "components", "*", "files", "lemnos-board*.toml")))
for f in files:
    d = tomllib.load(open(f, "rb"))
    fans = [x.get("id", "?") for x in d.get("devices", []) if x.get("driver") == "pwm-channel-fan"]
    if not fans:
        continue
    bid = d.get("board", {}).get("id", "")
    bdir = os.path.join(dev, "boards", bid)
    configs = glob.glob(os.path.join(bdir, "kernel", "*.config"))
    btoml = os.path.join(bdir, "board.toml")
    if os.path.isfile(btoml):
        m = re.search(r'^extends = "([^"]+)"', open(btoml).read(), re.M)
        if m:
            configs.append(os.path.join(os.path.dirname(os.path.normpath(os.path.join(bdir, m.group(1)))), "assets", "linux.fragment"))
    kernel = any(re.search(r"^CONFIG_SENSORS_PWM_FAN=[ym]$", open(c).read(), re.M) for c in configs if os.path.isfile(c))
    lines = d.get("boot", {}).get("config_lines", [])
    node = any("cooling_fan" in l or re.match(r"^dtoverlay=[^,]*fan", l) for l in lines)
    if not (kernel and node):
        miss = " and ".join(w for w, ok in (("no CONFIG_SENSORS_PWM_FAN=y|m in its kernel configs", kernel),
                                            ("no config_lines entry enabling a pwm-fan node", node)) if not ok)
        print(f"FAIL {os.path.relpath(f, dev)}: declares pwm-channel-fan {', '.join(fans)} but board {bid!r} has no kernel fan fallback ({miss}): lemnosd's PWM fan does not bind under the real unit at the pinned Lemnos rev, so the board would have no working fan owner")
        rc = 1
sys.exit(rc)
PY
}
fan_owner_self_test() {
  local tmp rc=0 f
  tmp=$(mktemp -d); cp -a "$root" "$tmp/devices"
  fan_owner_rule "$tmp/devices" >/dev/null || { rm -rf "$tmp"; return 1; }
  f="$tmp/devices/boards/raze/lemnos-board.toml"
  printf '\n[[devices]]\nid = "fan"\ndriver = "pwm-channel-fan"\nuses = { pwm = "pwm:/axi/pcie@1000120000/rp1/pwm@9c000:3" }\n' >>"$f"
  local out
  if out=$(fan_owner_rule "$tmp/devices" 2>&1); then rc=1
  elif [[ $out != *"declares pwm-channel-fan fan but board 'raze' has no kernel fan fallback"* ]]; then rc=1; fi
  printf 'CONFIG_SENSORS_PWM_FAN=m\n' >>"$tmp/devices/boards/raze/kernel/raze.config"
  sed -i 's/^config_lines = \["dtoverlay=vc4-kms-v3d-pi5", /config_lines = ["dtparam=cooling_fan=on", "dtoverlay=vc4-kms-v3d-pi5", /' "$f"
  fan_owner_rule "$tmp/devices" >/dev/null || rc=1
  rm -rf "$tmp"; return $rc
}
if ! fan_out=$(fan_owner_rule "$root"); then
  while IFS= read -r line; do [[ -n $line ]] && bad "${line#FAIL }"; done <<<"$fan_out"
fi

# The named-toolchain rule (2026-11-18, raze-base's libgpiod v2): every Buildroot image's fragments name its toolchain
# variant. A fragment that names none resolves against whatever the default is (Buildroot's internal toolchain with
# 7.1 headers here; a Bootlin STABLE default elsewhere has 5.4), so the config-only build could pass on a default the
# product does not own and the image would drop packages that need newer headers. Accepted: the internal toolchain
# (BR2_TOOLCHAIN_BUILDROOT=y), a Bootlin variant (BR2_TOOLCHAIN_EXTERNAL_BOOTLIN_<VARIANT>=y) or a custom external one.
named_toolchain_rule() {  # <devices dir>
  python3 -I - "$1" <<'PY'
import os, re, sys, tomllib
dev = sys.argv[1]; pdir = os.path.join(dev, "products"); rc = 0
want = re.compile(r"^BR2_TOOLCHAIN_(BUILDROOT|EXTERNAL_BOOTLIN_[A-Z0-9_]+|EXTERNAL_CUSTOM)=y$", re.M)
for name in sorted(os.listdir(pdir)):
    f = os.path.join(pdir, name, "product.toml")
    if not os.path.isfile(f): continue
    with open(f, "rb") as h: prod = tomllib.load(h)
    for spec in prod.get("images") or []:
        img = spec.get("name", "?") if isinstance(spec, dict) else "?"
        os_ = spec.get("os") if isinstance(spec, dict) else None
        if not isinstance(os_, dict) or os_.get("provider") != "buildroot": continue
        text = ""
        for frag in os_.get("fragments", []):
            p = os.path.join(pdir, name, frag)
            if os.path.isfile(p): text += open(p).read() + "\n"
        if not want.search(text):
            print(f"FAIL products/{name}: image {img}'s Buildroot fragments name no toolchain variant (BR2_TOOLCHAIN_BUILDROOT=y, BR2_TOOLCHAIN_EXTERNAL_BOOTLIN_<VARIANT>=y or BR2_TOOLCHAIN_EXTERNAL_CUSTOM=y): the config would resolve against a default"); rc = 1
sys.exit(rc)
PY
}
named_toolchain_self_test() {
  local tmp rc=0
  tmp=$(mktemp -d); cp -a "$root" "$tmp/devices"
  named_toolchain_rule "$tmp/devices" >/dev/null || { rm -rf "$tmp"; return 1; }
  sed -i '/^BR2_TOOLCHAIN_EXTERNAL_BOOTLIN_AARCH64_GLIBC_BLEEDING_EDGE=y$/d' "$tmp/devices/products/raze-base/os/raze.fragment"
  local out
  out=$(named_toolchain_rule "$tmp/devices") && rc=1
  [[ $out == *'products/raze-base: image raze-base'* ]] || rc=1
  rm -rf "$tmp"; return $rc
}
if ! nt_out=$(named_toolchain_rule "$root"); then
  while IFS= read -r line; do [[ -n $line ]] && bad "${line#FAIL }"; done <<<"$nt_out"
fi

# The post-build-removes-nothing rule (2026-11-18, the D1 build's dbus failure): a script a Buildroot fragment names as
# BR2_ROOTFS_POST_BUILD_SCRIPT runs before the rootfs step's makedevs (fs/common.mk), and makedevs fails on any file a
# package's permissions table names that is gone. So such a script removes nothing (no rm, rmdir, unlink or
# find -delete outside comments).
# A post-fakeroot script is allowed again since the Gaia 171ca25f pin (FAKEROOT-OUTPUT: its changes reach the slot);
# the 2026-11-19 refusal of BR2_ROOTFS_POST_FAKEROOT_SCRIPT went with that pin, as it said it would.
post_build_rule() {  # <devices dir>
  python3 -I - "$1" <<'PY'
import os, re, sys
dev = sys.argv[1]; rc = 0
line_re = re.compile(r'^BR2_ROOTFS_POST_BUILD_SCRIPT="([^"]*)"', re.M)
any_re = re.compile(r'^BR2_ROOTFS_POST_(BUILD|FAKEROOT|IMAGE)_SCRIPT="([^"]*)"', re.M)
rm_re = re.compile(r'(^|[\s;&|(`])(rm|rmdir|unlink)(\s|$)|\s-delete(\s|$)')
for top, _, files in os.walk(os.path.join(dev, "products")):
    for name in files:
        if not name.endswith(".fragment"): continue
        frag = os.path.join(top, name)
        for m in any_re.finditer(open(frag).read()):
            for word in m.group(2).split():
                if not word.startswith("@here@/"): continue
                script = os.path.normpath(os.path.join(top, word[len("@here@/"):]))
                if not os.path.isfile(script):
                    print(f"FAIL {os.path.relpath(frag, dev)}: BR2_ROOTFS_POST_{m.group(1)}_SCRIPT names {word}, which is not a file"); rc = 1
                elif not os.access(script, os.X_OK):
                    print(f"FAIL {os.path.relpath(script, dev)}: not executable (BR2_ROOTFS_POST_{m.group(1)}_SCRIPT runs it directly): chmod 755 and commit the mode"); rc = 1
        for m in line_re.finditer(open(frag).read()):
            for word in m.group(1).split():
                if not word.startswith("@here@/"): continue
                script = os.path.normpath(os.path.join(top, word[len("@here@/"):]))
                if not os.path.isfile(script): continue
                for n, text in enumerate(open(script), 1):
                    code = text.split("#", 1)[0] if not text.lstrip().startswith("#") else ""
                    if rm_re.search(code):
                        print(f"FAIL {os.path.relpath(script, dev)}:{n}: a post-build script removes files ({text.strip()}): makedevs runs after it and fails on a removed file a permissions table names"); rc = 1
sys.exit(rc)
PY
}
post_build_self_test() {
  local tmp rc=0 out
  tmp=$(mktemp -d); cp -a "$root" "$tmp/devices"
  post_build_rule "$tmp/devices" >/dev/null || { rm -rf "$tmp"; return 1; }
  printf 'rm -rf "$T/usr/libexec/dbus-daemon-launch-helper"\n' >> "$tmp/devices/products/raze-base/os/post-build.sh"
  out=$(post_build_rule "$tmp/devices") && rc=1
  [[ $out == *'products/raze-base/os/post-build.sh'* ]] || rc=1
  cp -a "$root/products/raze-base/os/post-build.sh" "$tmp/devices/products/raze-base/os/post-build.sh"
  chmod 644 "$tmp/devices/products/raze-base/os/post-build.sh"
  out=$(post_build_rule "$tmp/devices") && rc=1
  [[ $out == *'post-build.sh: not executable'* ]] || rc=1
  chmod 755 "$tmp/devices/products/raze-base/os/post-build.sh"
  # A post-fakeroot script that is not a file is refused like any other (the name check covers all three hooks).
  printf 'BR2_ROOTFS_POST_FAKEROOT_SCRIPT="@here@/no-such-script.sh"\n' >> "$tmp/devices/products/raze-base/os/raze.fragment"
  out=$(post_build_rule "$tmp/devices") && rc=1
  [[ $out == *'BR2_ROOTFS_POST_FAKEROOT_SCRIPT names @here@/no-such-script.sh, which is not a file'* ]] || rc=1
  rm -rf "$tmp"; return $rc
}
if ! pb_out=$(post_build_rule "$root"); then
  while IFS= read -r line; do [[ -n $line ]] && bad "${line#FAIL }"; done <<<"$pb_out"
fi

# The fixed-ids rule (coordinator's ruling 2026-11-20: any user that can own files on /data has a fixed id, so an image
# update never orphans its files). Every component's [[users]] states its uid and every [[groups]] its gid; no two
# users (or groups) declared in one product share an id; and a product with os/data-owners (USER UID GROUP GID reason)
# lists exactly the users its image's components declare, at the declared ids. scripts/check-slot-users.sh holds the
# built root to the same table.
fixed_ids_rule() {  # <devices dir>
  python3 -I - "$1" <<'PY'
import glob, os, sys, tomllib
dev = sys.argv[1]; rc = 0
def load(p):
    with open(p, "rb") as f: return tomllib.load(f)
for p in sorted(glob.glob(os.path.join(dev, "**", "component.toml"), recursive=True)):
    if os.sep + "tests" + os.sep in p: continue
    c = load(p); rel = os.path.relpath(p, dev)
    for u in c.get("users", []):
        if not isinstance(u.get("uid"), int): print(f"FAIL {rel}: user {u.get('name')} has no fixed uid (any user that can own files on /data has one)"); rc = 1
    for g in c.get("groups", []):
        if not isinstance(g.get("gid"), int): print(f"FAIL {rel}: group {g.get('name')} has no fixed gid"); rc = 1
for p in sorted(glob.glob(os.path.join(dev, "products", "*", "product.toml"))):
    prod = load(p); pdir = os.path.dirname(p); rel = os.path.relpath(p, dev)
    users, groups = {}, {}
    for img in prod.get("images", []):
        for name in img.get("components", []):
            cp = prod.get("components", {}).get(name)
            if not cp: continue
            c = load(os.path.normpath(os.path.join(pdir, cp)))
            for u in c.get("users", []): users[u["name"]] = (u.get("uid"), u.get("group"))
            for g in c.get("groups", []): groups[g["name"]] = g.get("gid")
    for kind, table in (("uid", {n: v[0] for n, v in users.items()}), ("gid", groups)):
        seen = {}
        for n, i in table.items():
            if i is not None and i in seen: print(f"FAIL {rel}: {kind} {i} is declared for both {seen[i]} and {n}"); rc = 1
            seen.setdefault(i, n)
    owners = os.path.join(pdir, "os", "data-owners")
    if not os.path.isfile(owners): continue
    rows = {}
    for n, line in enumerate(open(owners), 1):
        t = line.strip()
        if not t or t.startswith("#"): continue
        f = t.split(None, 4)
        if len(f) < 5 or not f[1].isdigit() or not f[3].isdigit():
            print(f"FAIL {os.path.relpath(owners, dev)}:{n}: not USER UID GROUP GID reason"); rc = 1; continue
        rows[f[0]] = (int(f[1]), f[2], int(f[3]))
    for name, (uid, grp) in sorted(users.items()):
        if name not in rows: print(f"FAIL {os.path.relpath(owners, dev)}: user {name} (declared, uid {uid}) is not in the table"); rc = 1; continue
        r = rows[name]
        if (r[0], r[1], r[2]) != (uid, grp, groups.get(grp)):
            print(f"FAIL {os.path.relpath(owners, dev)}: {name} is {r[0]} {r[1]} {r[2]} in the table, {uid} {grp} {groups.get(grp)} in the declarations"); rc = 1
    for name in sorted(set(rows) - set(users)):
        print(f"FAIL {os.path.relpath(owners, dev)}: {name} is in the table but no component of the product declares it"); rc = 1
sys.exit(rc)
PY
}
fixed_ids_self_test() {
  local tmp rc=0 out
  tmp=$(mktemp -d); cp -a "$root" "$tmp/devices"
  fixed_ids_rule "$tmp/devices" >/dev/null || { rm -rf "$tmp"; return 1; }
  sed -i 's/^uid = 900$/uid = 905/' "$tmp/devices/components/lemnos/component.toml"
  out=$(fixed_ids_rule "$tmp/devices") && rc=1
  [[ $out == *'lemnos is 900 lemnos 900 in the table, 905 lemnos 900 in the declarations'* ]] || rc=1
  sed -i '/^uid = 905$/d' "$tmp/devices/components/lemnos/component.toml"
  out=$(fixed_ids_rule "$tmp/devices") && rc=1
  [[ $out == *'user lemnos has no fixed uid'* ]] || rc=1
  rm -rf "$tmp"; return $rc
}
if ! fi_out=$(fixed_ids_rule "$root"); then
  while IFS= read -r line; do [[ -n $line ]] && bad "${line#FAIL }"; done <<<"$fi_out"
fi

# The verify-reads-as-it-wrote rule (2026-11-19, the D1 build 6 failure): a [generate.<step>] verify re-reads the step's
# output with the same tool, so it must name every description input the run names (--layout-of, --socs) with the same
# value. A verify without --socs resolved the board's SoC file against the working directory and failed only after the
# full image build (socs/bcm2712-rp1.toml: No such file).
generate_verify_rule() {  # <devices dir>
  python3 -I - "$1" <<'PY'
import os, sys, tomllib
dev = sys.argv[1]; rc = 0
def flag(argv, name):
    return argv[argv.index(name) + 1] if name in argv and argv.index(name) + 1 < len(argv) else None
for top, _, files in os.walk(dev):
    if "component.toml" not in files: continue
    f = os.path.join(top, "component.toml")
    with open(f, "rb") as h: comp = tomllib.load(h)
    for step, spec in sorted((comp.get("generate") or {}).items()):
        if not isinstance(spec, dict) or "verify" not in spec: continue
        args, verify = spec.get("args") or [], spec["verify"]
        for name in ("--layout-of", "--socs"):
            want = flag(args, name)
            if want is not None and flag(verify, name) != want:
                print(f"FAIL {os.path.relpath(f, dev)}: [generate.{step}] verify does not name {name} {want} as its args do: the verify reads the description differently from the run"); rc = 1
sys.exit(rc)
PY
}
generate_verify_self_test() {
  local tmp rc=0 out f
  tmp=$(mktemp -d); cp -a "$root" "$tmp/devices"
  generate_verify_rule "$tmp/devices" >/dev/null || { rm -rf "$tmp"; return 1; }
  f=$tmp/devices/components/raze-lemnos-board/component.toml
  sed -i '/^verify = \["data-partition"/s/"--socs", "@source@\/socs", //' "$f"
  out=$(generate_verify_rule "$tmp/devices") && rc=1
  [[ $out == *'raze-lemnos-board/component.toml: [generate.data] verify does not name --socs'* ]] || rc=1
  rm -rf "$tmp"; return $rc
}
if ! gv_out=$(generate_verify_rule "$root"); then
  while IFS= read -r line; do [[ -n $line ]] && bad "${line#FAIL }"; done <<<"$gv_out"
fi

# The slot-disk data size rule (Z2): a GPT slot product's data partition holds keep slots and the reserve.
if ! ds_out=$(data_size_rule "$root"); then
  while IFS= read -r line; do [[ -n $line ]] && bad "${line#FAIL }"; done <<<"$ds_out"
fi
# The open-timeout condition (Z2): the rk35xx kernel fragments build the watchdog in and set the open timeout.
if ! ot_out=$(open_timeout_rule "$root"); then
  while IFS= read -r line; do [[ -n $line ]] && bad "${line#FAIL }"; done <<<"$ot_out"
fi
# The RK35xx U-Boot environment rule (Z2): every rk35xx slot image's U-Boot config is the hard-rule one.
if ! rk_env_out=$(rk_env_rule "$root"); then
  while IFS= read -r line; do [[ -n $line ]] && bad "${line#FAIL }"; done <<<"$rk_env_out"
fi

# The Atlas side of the Lemnos pin and the one-copy rule (PR L3). Both read this repository's own Cargo files,
# so they run for the default tree only. They are here, in the device checker, because that is where the Lemnos
# rev of the image is held (the products' gaia.lock) and this checker already runs in the gate.
if [[ $root_given -eq 0 ]]; then
  if ! cargo_msg=$(lemnos_cargo_pin_rule "$repo/Cargo.toml" "$root"); then
    bad "Lemnos pin: ${cargo_msg//$'\n'/; }"
  fi
  dup_msg=$(lemnos_duplicate_crates "$repo/Cargo.lock")
  if [[ -n $dup_msg ]]; then
    bad "Lemnos crates at two revs in Cargo.lock: ${dup_msg//$'\n'/; }: one rev per Lemnos package"
  fi
fi

if [[ $fail -ne 0 ]]; then
  echo "check-devices: FAILED" >&2
  exit 1
fi
echo "check-devices: passed ($nf families, $nb boards, $nc components, $np products)"

# The default run also proves the checker still catches a broken file: the good fixtures must
# pass and the broken one must fail. Not run for --root, which names a tree to check alone.
if [[ $root_given -eq 0 ]]; then
  # Every self-check below is independent: each works on its own scratch copies (mktemp), never on the real tree, and
  # the fixture runs are separate processes. They run side by side, each with its own output files and its own copy of
  # the shared scratch files errs and outs (the only globals a self-test writes). Each function is still called as
  # `fn || ...` (set -e behaves inside it as it did when the list ran in order). All are waited for, then their output
  # is printed in the list's order and the first failure, in that order, stops the run with its message.
  st_dir=$(mktemp -d)
  st_names=() st_pids=() st_msgs=()
  st() {  # <failure message> <command...>
    local n=${#st_names[@]} msg=$1
    shift
    ( errs=$st_dir/$n.errs; outs=$st_dir/$n.outs; : >"$errs"; : >"$outs"; "$@" || exit $? ) >"$st_dir/$n.out" 2>"$st_dir/$n.err" &
    st_names+=("$*"); st_pids+=("$!"); st_msgs+=("$msg")
  }
  fixture_passes() {  # <fixture>
    local o
    o=$(mktemp)
    "$self" --root "$fixtures/$1" >"$o" 2>&1 || { cat "$o" >&2; rm -f "$o"; return 1; }
    rm -f "$o"
  }
  broken_fixture_fails() {
    local o rc=0
    o=$(mktemp)
    "$self" --root "$fixtures/broken" >"$o" 2>&1 || rc=$?
    broken_failed_as_it_must "$o" "$rc" || { cat "$o" >&2; rm -f "$o"; return 1; }
    rm -f "$o"
  }
  wd_self_test_quiet() { wd_self_test "$repo" >/dev/null; }
  # The version gate: a binary that prints another commit, or none, is refused, and the refusal names the pin.
  version_gate_self_test() {
    local fake vout other fb rc
    fake=$(mktemp -d)
    vout=$(mktemp)
    other=1111111111111111111111111111111111111111
    printf '#!/bin/sh\necho "gaia 2.0.6-dev (%s)"\n' "$other" > "$fake/gaia-other"
    printf '#!/bin/sh\necho "gaia 2.0.6-dev"\n' > "$fake/gaia-none"
    chmod +x "$fake/gaia-other" "$fake/gaia-none"
    for fb in gaia-other gaia-none; do
      rc=0
      GAIA_BIN="$fake/$fb" "$self" --root "$fixtures/good" >"$vout" 2>&1 || rc=$?
      if [[ $rc -eq 0 ]] || ! grep -q "devices/gaia.lock" "$vout" || ! grep -q "$gaia_commit" "$vout"; then
        cat "$vout" >&2; rm -rf "$fake" "$vout"; echo "check-devices: self-check: a $fb binary was not refused naming the pinned commit" >&2; return 1
      fi
    done
    rm -rf "$fake" "$vout"
  }
  for good in good good-f8 good-rust good-sd-image; do
    st "self-check: the $good fixture no longer passes" fixture_passes "$good"
  done
  st "self-check: the broken fixture passed, or failed without naming its bad key, so the checker no longer catches a broken file" broken_fixture_fails
  st "self-check: the Lemnos one-rev rule no longer refuses two revs in one product" lemnos_rule_self_test
  st "self-check: the Cargo pin rule no longer refuses a lemnos-slot-store rev other than the products' Lemnos rev" lemnos_cargo_pin_self_test "$repo/Cargo.toml" "$root"
  st "self-check: the one-copy rule no longer refuses a Lemnos crate at two revs" lemnos_duplicate_self_test "$repo/Cargo.lock"
  st "self-check: the Lemnos board file rule no longer refuses a lemnosd product without a board file" lemnos_board_rule_self_test
  st "self-check: the slot root chain check no longer refuses a module or a missing =y" slot_chain_self_test
  st "self-check: the bring-up rule no longer refuses a bring-up product without its signs" bringup_rule_self_test
  st "self-check: the boot firmware rule no longer refuses a Lemnos record that disagrees with its firmware" firmware_rule_self_test
  st "self-check: the in-no-product note lists a product's board, or misses a board nothing names" unnamed_self_test
  st "self-check: the pad rule no longer refuses a derived, unsourced, unstated or missing table, or an unstated family" pads_self_test
  st "self-check: the erofs rule no longer reads each product's plan" erofs_rule_self_test
  st "self-check: the data size rule no longer refuses a too-small data partition or a missing keep" data_size_self_test
  st "self-check: the RK env rule no longer refuses a slot image without USE_DT or DEVICE_INDEX" rk_env_self_test
  st "self-check: the slot id rule no longer refuses an id that is not the version's" slot_id_rule_self_test
  st "self-check: the boot partition rule no longer refuses a boot_size that is not the board's" boot_size_rule_self_test
  st "self-check: the fan owner rule no longer refuses a PWM fan without a kernel fallback, or refuses one with it" fan_owner_self_test
  st "self-check: the named-toolchain rule no longer refuses a Buildroot image whose fragments name no toolchain" named_toolchain_self_test
  st "self-check: the post-build rule no longer refuses a post-build script that removes a file, a non-executable script or a hook script that is not a file" post_build_self_test
  st "self-check: the fixed-ids rule no longer refuses a user without a fixed uid or a declaration that differs from os/data-owners" fixed_ids_self_test
  st "self-check: the generate verify rule no longer refuses a verify that names --socs differently from its run" generate_verify_self_test
  st "self-check: check-slot-setid.sh no longer refuses a slot whose setid bits differ from the decisions" "$repo/scripts/check-slot-setid.sh" --self-test
  st "self-check: check-slot-mounts.sh no longer refuses a slot root without /data" "$repo/scripts/check-slot-mounts.sh" --self-test
  st "self-check: check-slot-seccomp.sh no longer refuses a slot whose filters systemd or the kernel would not enforce" "$repo/scripts/check-slot-seccomp.sh" --self-test
  st "self-check: check-slot-users.sh no longer refuses a slot root missing a unit's user or an enable link" "$repo/scripts/check-slot-users.sh" --self-test
  st "self-check: the watchdog owner rule no longer refuses a product with two owners, none, or a build that disagrees with its inputs" wd_self_test_quiet
  st "self-check: a binary from another commit, or one that prints none, was not refused naming the pinned commit" version_gate_self_test
  st_rcs=()
  for i in "${!st_pids[@]}"; do
    rc=0; wait "${st_pids[$i]}" || rc=$?
    st_rcs+=("$rc")
  done
  for i in "${!st_pids[@]}"; do
    cat "$st_dir/$i.out"
    cat "$st_dir/$i.err" >&2
    [[ ${st_rcs[$i]} -eq 0 ]] || die "${st_msgs[$i]}"
  done
  rm -rf "$st_dir"
  echo "check-devices: self-check passed (good fixtures pass, broken fixture fails, a wrong or unknown gaia commit is refused)"
fi
