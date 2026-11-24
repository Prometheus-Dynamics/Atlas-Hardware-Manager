#!/usr/bin/env bash
# Every board a product builds: its U-Boot configuration must resolve, with no empty value and every fragment line
# taken (RK-FASTBOOT).
#
# Why: Atlas 287e2195 put the fastboot gadget into the rk35xx family's U-Boot fragment. On RK3566, RK3576 and RK3588,
# U-Boot gives CONFIG_FASTBOOT_BUF_ADDR (a hex symbol) no default, so the resolved .config held `CONFIG_FASTBOOT_BUF_ADDR=`.
# U-Boot's build then runs syncconfig, which asks for the value again and again with no answer: Gaia's rock-5c build
# hung 85 min. The gate never resolved a U-Boot configuration (check-kernel-fragments.sh resolves kernels only, and no
# rk35xx product was built between 287e2195 and Gaia's proof pair), so nothing caught it.
#
# For each board a product's image names (devices/products/*/product.toml, [[images]] `board`) with a bootloader: the pinned U-Boot tree is
# obtained (a shallow fetch of the commit the products' gaia.lock files name, cached by the pin), then the board's
# defconfig, its fragments in the order Gaia applies them (the board's list replaces the family's), and `olddefconfig`,
# in a scratch O= directory, with the arm64 cross compiler (U-Boot's Kconfig asks $(CC); ATLAS_KERNEL_CROSS, default
# aarch64-linux-gnu-, the kernel check's). Then it FAILS, naming the board:
#   - for each hex or int symbol the resolved .config leaves empty (`CONFIG_X=`, no value): U-Boot's build asks for it
#     without end. String symbols are written quoted (`CONFIG_X=""`) and are not this case.
#   - for each fragment line that does not take: the file, the line number, the requested value and the resolved one
#     (a symbol the tree does not have is "absent": the line is inert).
# A board whose bootloader takes patches is SKIPPED by name (this check does not apply them); none does today.
#
# Usage:
#   scripts/check-uboot-fragments.sh                 check every product's board
#   scripts/check-uboot-fragments.sh --self-test     the check above, then two copies of the tree that must FAIL: the
#                                                    RK3588 buffer address removed (an empty hex symbol, the 287e2195
#                                                    state) and a family line for a symbol U-Boot does not have
#   scripts/check-uboot-fragments.sh --devices DIR   check another tree laid out like devices/
#   scripts/check-uboot-fragments.sh --only BOARD    resolve only that board (passes, saying so, when no product builds it)
#
# Environment:
#   ATLAS_UBOOT_CACHE      where pinned U-Boot trees are kept (default: u-boot/ in the kernel check's cache,
#                          ${ATLAS_KERNEL_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/atlas/kernel}/u-boot, so CI's kernel cache keeps
#                          them too: its key covers the family and board files, where the U-Boot pins are).
#   ATLAS_UBOOT_SKIP=1     locally only: skip the whole check, printing that it was skipped. A source that cannot be
#                          fetched is a FAIL otherwise: a configuration that cannot be resolved is not a pass.
#   KEEP_UBOOT_SCRATCH=1   keep the scratch O= directories (default: removed).
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
devices=$repo/devices
mode=check
only=
while [[ $# -gt 0 ]]; do
  case $1 in
    --self-test) mode=self-test; shift ;;
    --devices) devices=$(cd "$2" && pwd); shift 2 ;;
    --only) only=$2; shift 2 ;;
    -h|--help) sed -n '2,36p' "$0"; exit 0 ;;
    *) echo "check-uboot-fragments: unknown argument: $1" >&2; exit 2 ;;
  esac
done

die() { echo "check-uboot-fragments: $*" >&2; exit 1; }

if [[ ${ATLAS_UBOOT_SKIP:-0} == 1 ]]; then
  echo "check-uboot-fragments: SKIPPED (ATLAS_UBOOT_SKIP=1): U-Boot configurations were NOT resolved." >&2
  exit 0
fi

if [[ $mode == self-test ]]; then
  # The real check and the two broken copies resolve at the same time (each in its own scratch directories); the
  # results are then read in order. 1: the real check must pass. 2: the RK3588 buffer address removed (the 287e2195
  # state) must fail, naming the empty symbol. 3: a family line for a symbol this U-Boot does not have must fail,
  # naming the file and line.
  tmp=$(mktemp -d "${TMPDIR:-/tmp}/atlas-uboot-selftest-XXXXXX")
  trap 'rm -rf "$tmp"' EXIT
  cp -a "$devices" "$tmp/a"
  soc=$tmp/a/families/rk35xx/assets/u-boot-rk3588.fragment
  [[ -f $soc ]] || die "self-test: no $soc"
  grep -v '^CONFIG_FASTBOOT_BUF_ADDR=' "$soc" > "$soc.new" && mv "$soc.new" "$soc"
  cp -a "$devices" "$tmp/b"
  fam=$tmp/b/families/rk35xx/assets/u-boot.fragment
  printf 'CONFIG_ATLAS_SELF_TEST_NO_SUCH_SYMBOL=y\n' >> "$fam"
  line=$(wc -l < "$fam")
  "$0" --devices "$devices" >"$tmp/real.txt" 2>&1 &
  real_pid=$!
  "$0" --devices "$tmp/a" --only radxa-rock-5c >"$tmp/a.txt" 2>&1 &
  a_pid=$!
  "$0" --devices "$tmp/b" --only radxa-zero-3e >"$tmp/b.txt" 2>&1 &
  b_pid=$!
  rc=0
  wait "$real_pid" || rc=$?
  cat "$tmp/real.txt"
  [[ $rc -eq 0 ]] || die "self-test: the real check did not pass (see above)"
  rc=0
  wait "$a_pid" || rc=$?
  if [[ $rc -eq 0 ]]; then
    cat "$tmp/a.txt" >&2; die "self-test: the ROCK 5C without its fastboot buffer address PASSED: the check does not catch an empty hex symbol"
  fi
  grep -q 'FAIL radxa-rock-5c: CONFIG_FASTBOOT_BUF_ADDR resolved empty' "$tmp/a.txt" \
    || { cat "$tmp/a.txt" >&2; die "self-test: the check failed but did not name CONFIG_FASTBOOT_BUF_ADDR as empty"; }
  rc=0
  wait "$b_pid" || rc=$?
  if [[ $rc -eq 0 ]] || ! grep -q "families/rk35xx/assets/u-boot.fragment:$line: CONFIG_ATLAS_SELF_TEST_NO_SUCH_SYMBOL requested y, resolved absent" "$tmp/b.txt"; then
    cat "$tmp/b.txt" >&2; die "self-test: a family line for a symbol U-Boot does not have did not fail naming its file and line"
  fi
  echo "check-uboot-fragments: self-test passed (the real check passes; a board left with an empty hex symbol fails, naming it; a fragment line that does not take fails, naming its file and line)"
  exit 0
fi

for tool in make git python3 flex bison; do
  command -v "$tool" >/dev/null || die "$tool is needed (flex and bison build U-Boot's Kconfig)"
done
CROSS_PREFIX=${ATLAS_KERNEL_CROSS:-aarch64-linux-gnu-}
command -v "${CROSS_PREFIX}gcc" >/dev/null || die "${CROSS_PREFIX}gcc is needed: U-Boot's Kconfig asks the compiler (scripts/Kconfig.include), so the configuration is resolved with the arm64 cross toolchain the build uses. The host compiler is not used in its place."
cache=${ATLAS_UBOOT_CACHE:-${ATLAS_KERNEL_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/atlas/kernel}/u-boot}
scratch=$(mktemp -d "${TMPDIR:-/tmp}/atlas-uboot-XXXXXX")
trap '[[ ${KEEP_UBOOT_SCRATCH:-0} == 1 ]] || rm -rf "$scratch"' EXIT
mkdir -p "$cache"

# One line per board a product builds:  name|url|commit|defconfig|fragment-paths   or   name|SKIP|reason
manifest() {
  python3 -I - "$devices" <<'PY'
import glob, os, sys, tomllib
dev = sys.argv[1]
def load(p):
    with open(p, "rb") as f:
        return tomllib.load(f)
locked = {}
for lpath in sorted(glob.glob(os.path.join(dev, "products", "*", "gaia.lock"))):
    for key, entry in load(lpath).get("source", {}).items():
        if entry.get("kind") != "git":
            continue
        pin = (entry["url"], entry["ref"])
        seen = locked.setdefault(key, {}).setdefault(pin, (entry["commit"], lpath))
        if seen[0] != entry["commit"]:
            sys.exit(f"check-uboot-fragments: {key} at {pin[1]} is locked to {seen[0]} in {seen[1]} and to {entry['commit']} in {lpath}")
boards = set()
for ppath in sorted(glob.glob(os.path.join(dev, "products", "*", "product.toml"))):
    for image in load(ppath).get("images", []):
        if image.get("board"):
            boards.add(os.path.normpath(os.path.join(os.path.dirname(ppath), image["board"])))
for bpath in sorted(boards):
    bdir = os.path.dirname(bpath)
    board = load(bpath)
    name = board.get("name", os.path.basename(bdir))
    fam, fdir = {}, bdir
    if "extends" in board:
        fpath = os.path.normpath(os.path.join(bdir, board["extends"]))
        fam, fdir = load(fpath), os.path.dirname(fpath)
    bb, fb = board.get("bootloader", {}), fam.get("bootloader", {})
    if not bb and not fb:
        continue
    src_name = bb.get("source", fb.get("source"))
    sources = dict(fam.get("sources", {}))
    sources.update(board.get("sources", {}))
    src = sources.get(src_name) if src_name else None
    if src is None or "git" not in src:
        print(f"{name}|SKIP|bootloader source {src_name!r} is not a git source")
        continue
    if bb.get("patches") or (fb.get("patches") and bb.get("inherit_patches", True) is not False):
        print(f"{name}|SKIP|its bootloader takes patches, which this check does not apply")
        continue
    defconfig = bb.get("defconfig", fb.get("defconfig"))
    if not defconfig:
        print(f"{name}|SKIP|NO-DEFCONFIG")
        continue
    if "fragments" in bb:
        frags = [os.path.normpath(os.path.join(bdir, f)) for f in bb["fragments"]]
    else:
        frags = [os.path.normpath(os.path.join(fdir, f)) for f in fb.get("fragments", [])]
    found = locked.get(f"{name}.{src_name}", {}).get((src["git"], src["ref"]))
    print("|".join([name, src["git"], found[0] if found else src["ref"], defconfig, ",".join(frags)]))
PY
}

# Checks one resolved .config: every empty hex/int symbol, then every line of every fragment. Prints FAIL lines.
check_config() {  # <resolved .config> <board> <devices dir> <fragment>...
  python3 -I - "$@" <<'PY'
import os, re, sys
cfg, board, dev, frags = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4:]
resolved = {}
for line in open(cfg):
    s = line.strip()
    m = re.match(r"^(CONFIG_\w+)=(.*)$", s)
    if m:
        resolved[m.group(1)] = m.group(2)
        continue
    m = re.match(r"^# (CONFIG_\w+) is not set$", s)
    if m:
        resolved[m.group(1)] = "n"
for sym, val in resolved.items():
    if val == "":
        print(f"FAIL {board}: {sym} resolved empty: a hex or int symbol with no value and no default for this board. "
              f"U-Boot's build (syncconfig) asks for it without end. State it in a fragment the board lists.")
for frag in frags:
    rel = os.path.relpath(frag, dev)
    for num, line in enumerate(open(frag), 1):
        s = line.strip()
        m = re.match(r"^(CONFIG_\w+)=(.*)$", s)
        if m:
            sym, want = m.group(1), m.group(2)
        else:
            m = re.match(r"^# (CONFIG_\w+) is not set$", s)
            if not m:
                continue
            sym, want = m.group(1), "n"
        got = resolved.get(sym, "absent")
        if got == "absent" and want == "n":
            continue
        if got == "absent":
            print(f"FAIL {board}: {rel}:{num}: {sym} requested {want}, resolved absent (no such symbol in this U-Boot tree: the line is inert)")
        elif got != want:
            print(f"FAIL {board}: {rel}:{num}: {sym} requested {want}, resolved {got}")
PY
}

fetch_pin() {  # <empty dir> <url> <commit or tag>: 3 tries, 10 s apart, each bounded
  local dir=$1 url=$2 ref=$3 try
  for try in 1 2 3; do
    rm -rf "$dir"; mkdir -p "$dir"
    if git -C "$dir" init -q \
      && git -C "$dir" -c http.connectTimeout=30 -c http.lowSpeedLimit=1000 -c http.lowSpeedTime=60 fetch -q --depth 1 "$url" "$ref" \
      && git -C "$dir" checkout -q FETCH_HEAD; then
      if [[ $ref =~ ^[0-9a-f]{40}$ && $(git -C "$dir" rev-parse HEAD) != "$ref" ]]; then
        echo "check-uboot-fragments: $url answered $ref with another commit; not used" >&2
        return 1
      fi
      return 0
    fi
    echo "check-uboot-fragments: fetch of $ref from $url failed (try $try of 3)" >&2
    [[ $try == 3 ]] || sleep 10
  done
  return 1
}

resolve_board() {  # <name> <source tree> <defconfig> <fragment list>: prints FAIL lines, then "RESULT <fails> <files>"
  local name=$1 res=$2 defconfig=$3 o target out
  local -a farr
  IFS=',' read -ra farr <<<"$4"
  o=$scratch/$name
  mkdir -p "$o"
  # Gaia takes the name with or without the suffix (the rk35xx boards state it without, the sun60i ones with).
  target=${defconfig%_defconfig}_defconfig
  (cd "$res" && make -s ARCH=arm CROSS_COMPILE="$CROSS_PREFIX" O="$o" "$target") >"$scratch/$name.log" 2>&1 </dev/null \
    || { echo "check-uboot-fragments: $name: the $target step failed (log: $scratch/$name.log)"; return 1; }
  (cd "$res" && scripts/kconfig/merge_config.sh -m -O "$o" "$o/.config" "${farr[@]}") >>"$scratch/$name.log" 2>&1 </dev/null \
    || { echo "check-uboot-fragments: $name: merge_config failed (log: $scratch/$name.log)"; return 1; }
  # olddefconfig never asks (it takes defaults and leaves a symbol with none empty); the bound is only a guard.
  (cd "$res" && timeout 600 make -s ARCH=arm CROSS_COMPILE="$CROSS_PREFIX" O="$o" olddefconfig) >>"$scratch/$name.log" 2>&1 </dev/null \
    || { echo "check-uboot-fragments: $name: olddefconfig failed (log: $scratch/$name.log)"; return 1; }
  out=$(check_config "$o/.config" "$name" "$devices" "${farr[@]}")
  [[ -n $out ]] && echo "$out"
  echo "RESULT $( [[ -n $out ]] && printf '%s\n' "$out" | wc -l || echo 0) ${#farr[@]}"
}

checked=0 skipped=0 fails=0 matched=0
pids=() names=()
rows=$(manifest) || die "the board descriptions could not be read (see above)"
while IFS= read -r row; do
  [[ -z $row ]] && continue
  IFS='|' read -r name url ref defconfig frags <<<"$row"
  if [[ -n $only && $name != "$only" ]]; then continue; fi
  matched=1
  if [[ $url == SKIP ]]; then
    [[ $ref == NO-DEFCONFIG ]] && die "$name: its bootloader states no \`defconfig\` (board.toml or the family's [bootloader]); state it."
    echo "check-uboot-fragments: SKIPPED $name: $ref" >&2
    skipped=$((skipped + 1)); continue
  fi
  key=$(printf '%s %s' "$url" "$ref" | sha256sum | cut -c1-16)
  src=$cache/$key/src
  # Two gates (two worktrees) can want the same tree at once: the fetch is under a lock, and a tree another run
  # finished is used as it is.
  (
    flock -w 900 9 || die "the tree lock $cache/$key.lock was not free in 900 s"
    if [[ ! -f $src/.pin ]]; then
      echo "check-uboot-fragments: fetching $name's U-Boot: $url at $ref" >&2
      rm -rf "$cache/$key"; mkdir -p "$src"
      if ! fetch_pin "$src" "$url" "$ref"; then
        rm -rf "$cache/$key"
        die "$name: CANNOT FETCH $url at $ref. A configuration that cannot be resolved is not a pass. Locally only, ATLAS_UBOOT_SKIP=1 skips this check, visibly."
      fi
      echo "$url $ref" > "$src/.pin"
    fi
  ) 9>"$cache/$key.lock" || exit 1
  resolve_board "$name" "$src" "$defconfig" "$frags" > "$scratch/$name.result" 2>&1 &
  pids+=("$!"); names+=("$name")
done <<<"$rows"

if [[ -n $only && $matched -eq 0 ]]; then
  echo "check-uboot-fragments: $only: no product builds this board with a bootloader; nothing to resolve"
  exit 0
fi

for i in "${!pids[@]}"; do
  name=${names[$i]}
  rc=0
  wait "${pids[$i]}" || rc=$?
  result=$(tail -n 1 "$scratch/$name.result")
  if [[ $rc -ne 0 || $result != RESULT\ * ]]; then
    cat "$scratch/$name.result" >&2
    die "$name: its U-Boot configuration could not be resolved (see above)"
  fi
  read -r _ board_fail nfrag <<<"$result"
  if [[ $board_fail -gt 0 ]]; then
    head -n -1 "$scratch/$name.result" >&2
    fails=$((fails + board_fail))
  else
    echo "check-uboot-fragments: ok $name ($nfrag fragment file(s): every line takes, no empty value)"
  fi
  checked=$((checked + 1))
done

if [[ $fails -ne 0 ]]; then
  echo "check-uboot-fragments: FAILED: $fails finding(s) (see above)" >&2
  exit 1
fi
echo "check-uboot-fragments: passed ($checked U-Boot configuration(s) resolved, $skipped skipped)"
