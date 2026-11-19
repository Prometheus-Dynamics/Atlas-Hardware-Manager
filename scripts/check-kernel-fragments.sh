#!/usr/bin/env bash
# Every line of every kernel fragment of every board must take (PR B25).
#
# For each board with a kernel: the pinned kernel tree is obtained (a shallow fetch of the one pinned commit
# or tag, cached by the pin), then the arm64 defconfig (or the family's), the fragments in the order a board
# applies them (the board's list replaces the family's), and `olddefconfig` run in a scratch O= directory with
# host tools (make, flex, bison, git, python3) and the arm64 cross compiler (ATLAS_KERNEL_CROSS, default
# aarch64-linux-gnu-). The resolve uses that toolchain, as the kernel build does: toolchain-probed symbols
# (ARM64_MTE, AS_HAS_*, CC_HAS_*) resolve differently with a host compiler (PR B29). No host fallback.
# Each requested line is compared with the resolved .config. A line that does not take FAILS, naming the
# board, the fragment file, the line number, the requested value and the resolved value.
#
# Usage:
#   scripts/check-kernel-fragments.sh                 check every board's kernel fragments
#   scripts/check-kernel-fragments.sh --self-test     the check above, then a scratch copy of the Raze's trim
#                                                     WITHOUT its selector lines (the state that failed in
#                                                     PR B25) must FAIL, naming the line
#   scripts/check-kernel-fragments.sh --devices DIR   check another tree laid out like devices/
#   scripts/check-kernel-fragments.sh --only BOARD    resolve only that board (the self-test's negative cases use it:
#                                                     the case changes one board, so only that board is resolved)
#
# Environment:
#   ATLAS_KERNEL_CACHE     where pinned kernel trees are kept (default ${XDG_CACHE_HOME:-$HOME/.cache}/atlas/kernel).
#                          CI caches this directory keyed by the kernel pins (.github/workflows/next.yml).
#   ATLAS_KERNEL_SKIP=1    locally only: skip the whole check, printing that it was skipped. A source that cannot be
#                          fetched is a FAIL otherwise, in CI as well: a kernel configuration that cannot be
#                          resolved is not a pass.
#   ATLAS_KERNEL_RESULT_CACHE=0  resolve every board, even one whose result is kept for the same inputs (results/ in
#                          the cache dir, keyed by tree, defconfig, fragment contents, toolchain and this script).
#   KEEP_KERNEL_SCRATCH=1  keep the scratch O= directories (default: removed).
#   ATLAS_LLVM_TOOLS_CACHE where the pinned LLVM and Rust tools of a board with [kernel] toolchain = "llvm" are kept
#                          (default: the kernel cache; about 1 GB, fetched once by scripts/kernel-llvm-tools.sh).
#
# A board whose kernel needs a patch series (a source named "*series*" in its family) is listed as SKIPPED: this
# check does not apply patch series, and the base tree alone cannot resolve such a board's fragments.
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
    *) echo "check-kernel-fragments: unknown argument: $1" >&2; exit 2 ;;
  esac
done

die() { echo "check-kernel-fragments: $*" >&2; exit 1; }

if [[ ${ATLAS_KERNEL_SKIP:-0} == 1 ]]; then
  echo "check-kernel-fragments: SKIPPED (ATLAS_KERNEL_SKIP=1): kernel fragments were NOT checked. CI runs this check." >&2
  exit 0
fi

if [[ $mode == self-test ]]; then
  # 1. the real check must pass
  "$0" --devices "$devices" || die "self-test: the real check did not pass (see above)"
  # 2. a tree whose Raze trim lacks the selector lines (PR B25's cause) must fail, naming the line
  tmp=$(mktemp -d "${TMPDIR:-/tmp}/atlas-kernel-selftest-XXXXXX")
  trap 'rm -rf "$tmp"' EXIT
  cp -a "$devices" "$tmp/devices"
  trim=$tmp/devices/boards/raze/kernel/raze-trim.config
  [[ -f $trim ]] || die "self-test: no $trim"
  grep -v '^# CONFIG_LATENCYTOP is not set$' "$trim" > "$trim.new" && mv "$trim.new" "$trim"
  out=$tmp/out.txt
  rc=0
  "$0" --devices "$tmp/devices" --only raze >"$out" 2>&1 || rc=$?
  if [[ $rc -eq 0 ]]; then
    cat "$out" >&2; die "self-test: a trim whose KALLSYMS_ALL line cannot take (LATENCYTOP removed) PASSED: the check does not catch it"
  fi
  if ! grep -q 'raze-trim.config' "$out" || ! grep -q 'CONFIG_KALLSYMS_ALL requested n, resolved y' "$out"; then
    cat "$out" >&2; die "self-test: the check failed but did not name raze-trim.config and CONFIG_KALLSYMS_ALL"
  fi
  # 3. a fragment with a bare "#" line must fail, naming the file and the line, before any kernel is resolved
  tmp3=$(mktemp -d "${TMPDIR:-/tmp}/atlas-kernel-selftest-bare-XXXXXX")
  trap 'rm -rf "$tmp" "$tmp3"' EXIT
  cp -a "$devices" "$tmp3/devices"
  printf '#\n' >> "$tmp3/devices/families/bcm2712/assets/linux.fragment"
  rc=0
  "$0" --devices "$tmp3/devices" >"$tmp3/out.txt" 2>&1 || rc=$?
  if [[ $rc -eq 0 ]]; then
    cat "$tmp3/out.txt" >&2; die "self-test: a fragment with a bare '#' line PASSED: the check does not catch it"
  fi
  if ! grep -q 'linux.fragment:[0-9]*: a bare comment marker' "$tmp3/out.txt"; then
    cat "$tmp3/out.txt" >&2; die "self-test: the check failed but did not name the file and line of the bare '#'"
  fi
  # 4. the toolchain is the build's: CONFIG_RUST=y takes only when the Raze is configured with LLVM (the pinned tools),
  #    and fails, naming the line, when the same fragment is resolved with gcc
  tmp4=$(mktemp -d "${TMPDIR:-/tmp}/atlas-kernel-selftest-llvm-XXXXXX")
  trap 'rm -rf "$tmp" "$tmp3" "$tmp4"' EXIT
  cp -a "$devices" "$tmp4/devices"
  printf '\n# self-test: Rust\nCONFIG_RUST=y\n' >> "$tmp4/devices/boards/raze/kernel/raze.config"
  rc=0
  "$0" --devices "$tmp4/devices" --only raze >"$tmp4/gcc.txt" 2>&1 || rc=$?
  if [[ $rc -eq 0 ]] || ! grep -q 'CONFIG_RUST requested y' "$tmp4/gcc.txt"; then
    cat "$tmp4/gcc.txt" >&2; die "self-test: CONFIG_RUST=y resolved with gcc did not fail naming CONFIG_RUST"
  fi
  python3 -I - "$tmp4/devices/boards/raze/board.toml" <<'PY' || die "self-test: could not set the Raze's toolchain"
import re, sys
p = sys.argv[1]; s = open(p).read()
s2 = re.sub(r'(?m)^\[kernel\]\n', '[kernel]\ntoolchain = "llvm"\n', s, count=1)
assert s2 != s; open(p, "w").write(s2)
PY
  rc=0
  "$0" --devices "$tmp4/devices" --only raze >"$tmp4/llvm.txt" 2>&1 || rc=$?
  if [[ $rc -ne 0 ]] || ! grep -q 'ok raze' "$tmp4/llvm.txt"; then
    cat "$tmp4/llvm.txt" >&2; die "self-test: CONFIG_RUST=y did not take with toolchain = \"llvm\" and the pinned tools"
  fi
  echo "check-kernel-fragments: self-test passed (the real check passes; a line that cannot take fails, naming it; a bare '#' line fails, naming its file and line; CONFIG_RUST=y takes with the pinned LLVM tools and fails with gcc)"
  exit 0
fi

for tool in make git python3 flex bison; do
  command -v "$tool" >/dev/null || die "$tool is needed (flex and bison are the kernel's host tools for configuration)"
done
# The cross compiler: every board is arm64, and the resolve must see the toolchain the kernel build sees.
CROSS_PREFIX=${ATLAS_KERNEL_CROSS:-aarch64-linux-gnu-}
command -v "${CROSS_PREFIX}gcc" >/dev/null || die "${CROSS_PREFIX}gcc is needed: the configuration is resolved with the arm64 cross toolchain, as the kernel build resolves it (Debian and Ubuntu package: gcc-aarch64-linux-gnu). The host compiler is not used in its place."
echo "check-kernel-fragments: resolving with ${CROSS_PREFIX}gcc $("${CROSS_PREFIX}gcc" -dumpfullversion 2>/dev/null) (ARCH=arm64 CROSS_COMPILE=$CROSS_PREFIX)" >&2
export ARCH=arm64 CROSS_COMPILE=$CROSS_PREFIX
cache=${ATLAS_KERNEL_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/atlas/kernel}
# Everything outside the source tree and the board's files that a resolve's result can depend on: this script (the
# resolve and the line check are in it), the cross and host toolchains (Kconfig asks the compiler what it supports),
# the host tools that build and run Kconfig, and the environment variables Kbuild reads. Part of every result's key.
tools_key=$( { sha256sum "$0" | cut -d' ' -f1
  "${CROSS_PREFIX}gcc" -v 2>&1; "${CROSS_PREFIX}ld" -v 2>&1 || true
  "${HOSTCC:-gcc}" -v 2>&1 || true; make --version | head -n 1; flex --version; bison --version | head -n 1
  python3 -I -c 'import sys; print(sys.version)'
  env | grep -E '^(ARCH|CROSS_COMPILE|LLVM|LLVM_IAS|CC|LD|HOSTCC|HOSTCXX|KCONFIG_[A-Z_]*|KBUILD_[A-Z_]*|SRCARCH)=' | LC_ALL=C sort
} | sha256sum | cut -c1-32)
scratch=$(mktemp -d "${TMPDIR:-/tmp}/atlas-kernel-XXXXXX")
trap '[[ ${KEEP_KERNEL_SCRATCH:-0} == 1 ]] || rm -rf "$scratch"' EXIT
mkdir -p "$cache"

# One line per board with a kernel, read from the TOML files (python3 tomllib):
#   name|family|kernel-url|kernel-ref|defconfig|fragment-paths|series-url|series-ref   or   name|SKIP|reason
manifest() {
  python3 - "$devices" <<'PY'
import glob, os, sys, tomllib
dev = sys.argv[1]
def load(p):
    with open(p, "rb") as f:
        return tomllib.load(f)
# The commit each source is locked to, from the products' committed gaia.lock files. A build takes the
# commit from the lock, never from `ref`, so this check fetches that same commit: a tag in a description is
# a name, and a name is asked of no host here. A source no product locks yet is fetched by its `ref`.
locked = {}
for lpath in sorted(glob.glob(os.path.join(dev, "products", "*", "gaia.lock"))):
    for key, entry in load(lpath).get("source", {}).items():
        if entry.get("kind") != "git":
            continue
        pin = (entry["url"], entry["ref"])
        seen = locked.setdefault(key, {}).setdefault(pin, (entry["commit"], lpath))
        if seen[0] != entry["commit"]:
            sys.exit(f"check-kernel-fragments: {key} at {pin[1]} is locked to {seen[0]} in {seen[1]} and to {entry['commit']} in {lpath}")
def commit_of(board_name, src_name, src):
    found = locked.get(f"{board_name}.{src_name}", {}).get((src["git"], src["ref"]))
    return found[0] if found else src["ref"]
for bpath in sorted(glob.glob(os.path.join(dev, "boards", "*", "board.toml"))):
    bdir = os.path.dirname(bpath)
    board = load(bpath)
    if "extends" not in board:
        continue
    fpath = os.path.normpath(os.path.join(bdir, board["extends"]))
    fam = load(fpath)
    fdir = os.path.dirname(fpath)
    bk = board.get("kernel", {})
    fk = fam.get("kernel", {})
    if not bk and not fk:
        continue
    name = board.get("name", os.path.basename(bdir))
    src_name = bk.get("source", fk.get("source"))
    if src_name is None:
        continue
    sources = dict(fam.get("sources", {}))
    sources.update(board.get("sources", {}))
    src = sources.get(src_name)
    if src is None or "git" not in src:
        print(f"{name}|SKIP|kernel source {src_name!r} is not a git source")
        continue
    series = sorted(k for k in sources if "series" in k)
    series_pin = ""
    if len(series) > 1:
        print(f"{name}|SKIP|the family names {len(series)} patch series ({', '.join(series)}); this check applies one")
        continue
    if series:
        ss = sources[series[0]]
        if "git" not in ss:
            print(f"{name}|SKIP|patch series {series[0]} is not a git source; this check cannot fetch it")
            continue
        series_pin = f"{ss['git']}|{commit_of(name, series[0], ss)}"
    # No default: a board whose kernel names no starting configuration cannot build (Buildroot stops), so it
    # fails here by name instead of being checked against a configuration nobody stated.
    defconfig = bk.get("defconfig", fk.get("defconfig"))
    if defconfig is None:
        print(f"{name}|SKIP|NO-DEFCONFIG")
        continue
    if "fragments" in bk:
        frags = [os.path.normpath(os.path.join(bdir, f)) for f in bk["fragments"]]
    else:
        frags = [os.path.normpath(os.path.join(fdir, f)) for f in fk.get("fragments", [])]
    fam_name = fam.get("name", os.path.basename(fdir))
    # The effective kernel patch list (PR PATCH-LISTS, Gaia's append rule: a board's list is appended to its family's, so
    # the family's entries come first; `inherit_patches = false` in [kernel] takes the board's list alone). Each `file`
    # entry is a patch beside the board or the family; applied after the series, in this order. Gaia a6299c62 replaced
    # the family's list instead: the family's entries are still listed here by the same rule.
    inherit = bk.get("inherit_patches", True)
    fam_list = [(fdir, e) for e in (fk.get("patches") or [])]
    board_list = [(bdir, e) for e in (bk.get("patches") or [])]
    eff = board_list if inherit is False else (fam_list + board_list if "patches" in bk else fam_list)
    extra = [os.path.normpath(os.path.join(base, e["file"])) for base, e in eff if isinstance(e, dict) and "file" in e]
    # The series pin is "url commit" (a space: the row's own separator is `|`, which the pin used to carry).
    series_pin = series_pin.replace("|", " ")
    # The compiler the kernel is configured and built with: [kernel] toolchain, the board's over the family's, "gcc"
    # when neither says. "llvm" resolves with LLVM=1 and the pinned LLVM/Rust tools (scripts/kernel-llvm-tools.sh), as a
    # kernel with Rust is built; Kconfig asks the compiler, so the check must use the build's.
    toolchain = bk.get("toolchain", fk.get("toolchain", "gcc"))
    if toolchain not in ("gcc", "llvm"):
        sys.exit(f"check-kernel-fragments: {name}: [kernel] toolchain = {toolchain!r}: it is \"gcc\" or \"llvm\"")
    print("|".join([name, fam_name, src["git"], commit_of(name, src_name, src), defconfig, ",".join(frags), series_pin, ",".join(extra), bdir, toolchain]))
PY
}

# Checks the requested lines of one fragment against the resolved .config. Prints FAIL lines and returns 0.
check_fragment() {  # <resolved .config> <fragment file> <board> <devices dir>
  python3 - "$1" "$2" "$3" "$4" <<'PY'
import os, re, sys
cfg, frag, board, dev = sys.argv[1:5]
resolved = {}
for line in open(cfg):
    m = re.match(r"^(CONFIG_\w+)=(.*)$", line.strip())
    if m:
        resolved[m.group(1)] = m.group(2)
    m = re.match(r"^# (CONFIG_\w+) is not set$", line.strip())
    if m:
        resolved[m.group(1)] = "n"
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
    if got == "absent":
        # The symbol does not exist in this tree: the line is inert (a "not set" that asks for nothing). It must
        # be removed or replaced by a line for a symbol that exists (PR B25: no line may remain that does not take).
        print(f"FAIL {board}: {rel}:{num}: {sym} requested {want}, resolved absent (no such symbol in this kernel tree: the line is inert)")
        continue
    if want == "n" and got == "n":
        continue
    if got != want:
        print(f"FAIL {board}: {rel}:{num}: {sym} requested {want}, resolved {got}")
PY
}

# A kernel fragment line that is only "#" is refused by file and line, before any kernel is fetched: Gaia's build
# chokes on it (PR B29). Replace the line with an empty one. Checked on every fragment a board reads.
bare_marker_check() {
  local bad=0 row name fam url ref defconfig frags series_pin extra bdir f num
  local -a fa
  while IFS= read -r row; do
    IFS='|' read -r name fam url ref defconfig frags series_pin extra bdir toolchain <<<"$row"
    [[ $fam == SKIP ]] && continue
    IFS=',' read -ra fa <<<"$frags"
    for f in "${fa[@]}"; do
      while IFS= read -r num; do
        echo "check-kernel-fragments: FAIL $name: ${f#$devices/}:$num: a bare comment marker (a line of only '#'). Replace it with an empty line." >&2
        bad=$((bad + 1))
      done < <(grep -nE '^#[[:space:]]*$' "$f" 2>/dev/null | cut -d: -f1)
    done
  done < <(manifest)
  [[ $bad -eq 0 ]] || die "$bad bare comment marker line(s) in kernel fragments (see above)"
}
bare_marker_check

# Resolves one board's configuration in its own O= directory and checks every fragment line against it. Prints the
# FAIL lines, then a last line "RESULT <n failed lines> <fragment files>"; a step that cannot run ends with exit 1.
# A resolve is deterministic: the same source tree, defconfig, fragments, toolchain and script give the same result.
# So each result is kept, keyed by all of them: the tree by its pin (or its patched tree's key), each fragment by its
# name AND content, in order, and tools_key above. Any change to any of them misses the key and resolves again; a hit
# prints exactly what the resolve printed. Results are stored only when the resolve ran to its RESULT line.
# ATLAS_KERNEL_RESULT_CACHE=0 resolves every board anyway.
resolve_board() {  # <name> <family> <source tree> <defconfig> <fragment list> <toolchain>
  local name=$1 res=$3 defconfig=$4 frags=$5 key kept tmp f tkey=$tools_key
  local -a farr
  if [[ ${ATLAS_KERNEL_RESULT_CACHE:-1} == 0 ]]; then resolve_board_now "$@"; return; fi
  IFS=',' read -ra farr <<<"$frags"
  [[ $6 == llvm ]] && tkey="$tools_key llvm $llvm_key"
  key=$( { echo "$name|$defconfig"; cat "$res/.pin" "$res/.patched" 2>/dev/null; echo "$tkey"
    for f in "${farr[@]}"; do echo "== ${f#$devices/}"; sha256sum < "$f"; done; } | sha256sum | cut -c1-32)
  kept=$cache/results/$key
  if [[ -f $kept ]]; then
    touch "$kept" 2>/dev/null || true
    echo "check-kernel-fragments: $name: kept result (the same tree, defconfig, fragments, toolchain and script)"
    cat "$kept"; return 0
  fi
  mkdir -p "$cache/results"
  # A result unused for 30 days goes (each is a few lines; a hit refreshes its time).
  find "$cache/results" -maxdepth 1 -type f -mtime +30 -delete 2>/dev/null || true
  tmp=$cache/results/.new-$key.$BASHPID
  resolve_board_now "$@" >"$tmp" || { cat "$tmp"; rm -f "$tmp"; return 1; }
  cat "$tmp"
  if [[ $(tail -n 1 "$tmp") == RESULT\ * ]]; then mv -f "$tmp" "$kept"; else rm -f "$tmp"; fi
}

resolve_board_now() {  # <name> <family> <source tree> <defconfig> <fragment list> <toolchain>
  local name=$1 fam=$2 res=$3 defconfig=$4 frags=$5 o target out board_fail=0 f
  local -a farr
  # "llvm": the kernel's own LLVM=1 build, with the pinned tools first on PATH and no cross prefix (clang targets arm64
  # from ARCH). Exported in this board's subshell only.
  # RUSTC and BINDGEN are named on make's command line either way (the kernel Makefile sets them with a plain `=`):
  # a gcc board's resolve must not find a Rust toolchain the build machine has and Gaia's gcc build has not (a host
  # rustc and bindgen on PATH made CONFIG_RUST=y take under gcc, found by the self-test, 2026-11-18).
  local -a mk=(RUSTC=atlas-gcc-board-has-no-rustc BINDGEN=atlas-gcc-board-has-no-bindgen)
  if [[ $6 == llvm ]]; then
    export PATH="$llvm_path:$PATH" LIBCLANG_PATH=$llvm_libclang LLVM=1
    unset CROSS_COMPILE
    mk=(RUSTC="${llvm_path%%:*}/rustc" BINDGEN="${llvm_path%%:*}/bindgen")
  fi
  o=$scratch/$name
  mkdir -p "$o"
  target=$defconfig
  [[ $defconfig == defconfig ]] || target=${defconfig}_defconfig
  (cd "$res" && make -s ARCH=arm64 "${mk[@]}" O="$o" "$target") >"$scratch/$name.log" 2>&1 \
    || { echo "check-kernel-fragments: $name: the $target step failed (log: $scratch/$name.log)"; return 1; }
  IFS=',' read -ra farr <<<"$frags"
  (cd "$res" && scripts/kconfig/merge_config.sh -m -O "$o" "$o/.config" "${farr[@]}") >>"$scratch/$name.log" 2>&1 \
    || { echo "check-kernel-fragments: $name: merge_config failed (log: $scratch/$name.log)"; return 1; }
  (cd "$res" && make -s ARCH=arm64 "${mk[@]}" O="$o" olddefconfig) >>"$scratch/$name.log" 2>&1 \
    || { echo "check-kernel-fragments: $name: olddefconfig failed (log: $scratch/$name.log)"; return 1; }
  for f in "${farr[@]}"; do
    out=$(check_fragment "$o/.config" "$f" "$name" "$devices")
    if [[ -n $out ]]; then
      echo "$out"
      board_fail=$((board_fail + $(printf '%s\n' "$out" | wc -l)))
    fi
  done
  echo "RESULT $board_fail ${#farr[@]}"
}

# The pinned LLVM/Rust tools, fetched only when a board's toolchain is "llvm"; their key is the fetch script's hash
# (its pins are in it), part of the result key of every llvm board.
llvm_path= llvm_libclang=
llvm_key=$(sha256sum "$repo/scripts/kernel-llvm-tools.sh" | cut -c1-32)
fails=0
checked=0
skipped=0
pids=()
names=()
fams=()
declare -A patched=()
# Fetches one pinned commit or tag of `url` into `dir` (an empty directory) and checks it out.
#
# A host that does not answer is tried again: 3 tries, 10 s apart, each bounded (connect and stall), so an
# unreachable host costs minutes, not the job's whole time. git.kernel.org has been unreachable from CI
# runners more than once.
#
# When the pin is a full commit id, the same commit is then asked of a second host that carries the same
# history (MIRRORS). That is not a weaker check: a commit id names its content, and the fetched commit is
# compared with the pin before it is used. A tag pin is never taken from a second host, because a tag is
# only a name there.
declare -A MIRRORS=(
  [https://git.kernel.org/pub/scm/linux/kernel/git/stable/linux.git]=https://github.com/gregkh/linux.git
)
fetch_pin() {
  local dir=$1 url=$2 ref=$3 from try
  local -a hosts=("$url")
  if [[ $ref =~ ^[0-9a-f]{40}$ && -n ${MIRRORS[$url]:-} ]]; then hosts+=("${MIRRORS[$url]}"); fi
  for from in "${hosts[@]}"; do
    for try in 1 2 3; do
      rm -rf "$dir"; mkdir -p "$dir"
      if git -C "$dir" init -q \
        && git -C "$dir" -c http.connectTimeout=30 -c http.lowSpeedLimit=1000 -c http.lowSpeedTime=60 \
             fetch -q --depth 1 "$from" "$ref" \
        && git -C "$dir" checkout -q FETCH_HEAD; then
        if [[ $ref =~ ^[0-9a-f]{40}$ && $(git -C "$dir" rev-parse HEAD) != "$ref" ]]; then
          echo "check-kernel-fragments: $from answered $ref with another commit; not used" >&2
          break
        fi
        [[ $from == "$url" ]] || echo "check-kernel-fragments: $url did not answer; commit $ref taken from $from and checked against the pin" >&2
        return 0
      fi
      echo "check-kernel-fragments: fetch of $ref from $from failed (try $try of 3)" >&2
      [[ $try == 3 ]] || sleep 10
    done
  done
  return 1
}

while IFS= read -r row; do
  IFS='|' read -r name fam url ref defconfig frags series_pin extra bdir toolchain <<<"$row"
  if [[ $fam == SKIP && $url == NO-DEFCONFIG ]]; then
    die "$name: its kernel states no \`defconfig\` (board.toml or the family's [kernel]). Buildroot cannot build a kernel without a starting configuration; state it."
  fi
  if [[ -n $only && $name != "$only" ]]; then continue; fi
  if [[ $fam == SKIP ]]; then
    echo "check-kernel-fragments: SKIPPED $name: $url" >&2
    skipped=$((skipped + 1))
    continue
  fi
  key=$(printf '%s %s' "$url" "$ref" | sha256sum | cut -c1-16)
  src=$cache/$key/src
  if [[ ! -f $src/.pin ]]; then
    echo "check-kernel-fragments: fetching $name's kernel: $url at $ref" >&2
    rm -rf "$cache/$key"; mkdir -p "$src"
    if ! fetch_pin "$src" "$url" "$ref" 2>"$scratch/fetch.err"; then
      cat "$scratch/fetch.err" >&2
      rm -rf "$cache/$key"
      die "$name: CANNOT FETCH $url at $ref. This FAILS in CI too: a kernel configuration that cannot be resolved is not a pass. Locally only, ATLAS_KERNEL_SKIP=1 skips this check, visibly."
    fi
    echo "$url $ref" > "$src/.pin"
  fi
  res=$src
  if [[ -n ${series_pin:-} || -n ${extra:-} ]]; then
    surl=""; sref=""
    if [[ -n ${series_pin:-} ]]; then surl=${series_pin% *}; sref=${series_pin##* }; fi
    # The patched tree is kept in the cache, keyed by everything that makes it: the kernel pin, the series pin, and the
    # board's own patch files BY CONTENT (they live in this repository and change without a pin moving). A kept tree is
    # never written after it is made (the resolve uses O=), so reusing it is the same input as patching again. It is
    # made in a temporary directory and renamed into place only when every patch applied.
    xsum=""
    if [[ -n ${extra:-} ]]; then
      IFS=',' read -ra xs <<<"$extra"
      for x in "${xs[@]}"; do [[ -f $x ]] || die "$name: its kernel patch ${x#$devices/} is missing"; done
      xsum=$(cat "${xs[@]}" | sha256sum | cut -c1-16)
    fi
    # The patch files are named relative to the devices tree, not by absolute path: every worktree (and the self-test's
    # copy of the tree) with the same patches shares one kept tree.
    xnames=${extra:+${extra//$devices\//}}
    fam_key=$(printf '%s %s %s %s %s' "$url" "$ref" "$surl $sref" "$xnames" "$xsum" | sha256sum | cut -c1-16)
    pkept=$cache/patched-$fam_key
    if [[ -z ${patched[$fam_key]:-} && -f $pkept/.patched ]]; then
      echo "check-kernel-fragments: $name's patched tree is kept ($pkept: the same pins and patch contents)" >&2
      patched[$fam_key]=$pkept
    fi
    if [[ -z ${patched[$fam_key]:-} ]]; then
      pdir=$cache/.patched-$fam_key.$$
      echo "check-kernel-fragments: applying $name's kernel patches (the series, then the board's own) to a copy of $url at $ref" >&2
      rm -rf "$pdir"; cp -a --reflink=auto "$src" "$pdir"
      rm -f "$pdir/.pin"
      if [[ -n $surl ]]; then
        skey=$(printf '%s %s' "$surl" "$sref" | sha256sum | cut -c1-16)
        ssrc=$cache/series-$skey/src
        if [[ ! -f $ssrc/.pin ]]; then
          echo "check-kernel-fragments: fetching $name's patch series: $surl at $sref" >&2
          rm -rf "$cache/series-$skey"; mkdir -p "$ssrc"
          if ! fetch_pin "$ssrc" "$surl" "$sref" 2>"$scratch/series-fetch.err"; then
            cat "$scratch/series-fetch.err" >&2
            rm -rf "$cache/series-$skey"
            die "$name: CANNOT FETCH the patch series $surl at $sref. This FAILS in CI too."
          fi
          echo "$surl $sref" > "$ssrc/.pin"
        fi
        n=0
        while IFS= read -r p; do
          [[ -z $p ]] && continue
          if ! { patch -d "$pdir" -p1 --fuzz=0 --batch --forward --dry-run -s < "$ssrc/patches/$p" && patch -d "$pdir" -p1 --fuzz=0 --batch --forward --no-backup-if-mismatch -s < "$ssrc/patches/$p"; } >"$scratch/patch.out" 2>&1; then
            echo "check-kernel-fragments: FAIL $name: patch $p (number $((n + 1)) of the series) does not apply to $ref:" >&2
            sed 's/^/    /' "$scratch/patch.out" >&2
            die "$name: the patch series does not apply to $ref (see above)"
          fi
          n=$((n + 1))
        done < <(awk '{ sub(/[[:space:]]*#.*/, "") } /[^[:space:]]/' "$ssrc/patches/series")
        echo "check-kernel-fragments: $name's patch series applied: $n patches, 0 failed" >&2
      fi
      # The board's own patches, after the series, in the order its kernel.patches lists them (PR A7RAILS).
      if [[ -n ${extra:-} ]]; then
        IFS=',' read -ra xs <<<"$extra"
        for x in "${xs[@]}"; do
          [[ -f $x ]] || die "$name: its kernel patch ${x#$devices/} is missing"
          if ! { patch -d "$pdir" -p1 --fuzz=0 --batch --forward --dry-run -s < "$x" && patch -d "$pdir" -p1 --fuzz=0 --batch --forward --no-backup-if-mismatch -s < "$x"; } >"$scratch/patch.out" 2>&1; then
            sed 's/^/    /' "$scratch/patch.out" >&2
            die "$name: the board's kernel patch ${x#$devices/} does not apply after the series (see above)"
          fi
          echo "check-kernel-fragments: $name: applied ${x#$devices/} after the series, 0 failed" >&2
        done
      fi
      echo "$url $ref | $surl $sref | $xnames | $xsum" > "$pdir/.patched"
      # Two gates (two worktrees) can build the same tree at once: the swap is under a lock, and a tree another run
      # finished first is kept as it is (it may be in use), this run's copy dropped.
      (
        flock -w 600 9 || die "the patched-tree lock $cache/.patched-$fam_key.lock was not free in 600 s"
        if [[ -f $pkept/.patched ]]; then rm -rf "$pdir"; else rm -rf "$pkept"; mv "$pdir" "$pkept"; fi
      ) 9>"$cache/.patched-$fam_key.lock" || exit 1
      patched[$fam_key]=$pkept
    fi
    res=${patched[$fam_key]}
    # Facts verified against the patched tree (PR A7RAILS): the recorded facts must be what the built kernel says.
    # Every run, kept tree or not (the facts file is in this repository and is not part of the key).
    if [[ -n ${extra:-} && -n $bdir && -f $bdir/kernel/post-patch-facts.txt ]]; then
      python3 -I "$repo/scripts/check-board-facts.py" "$res" "$bdir" >&2 \
        || die "$name: its recorded facts do not match the kernel built with its patches (see above)"
    fi
  fi
  # The resolve of each board is independent (its own O= directory, read-only source tree): it runs in the background
  # and its result is read below in board order, so the output and the verdict are the same as a serial run.
  if [[ $toolchain == llvm && -z $llvm_path ]]; then
    echo "check-kernel-fragments: $name is configured with LLVM: the pinned LLVM and Rust tools (scripts/kernel-llvm-tools.sh)" >&2
    tools_out=$("$repo/scripts/kernel-llvm-tools.sh" "${ATLAS_LLVM_TOOLS_CACHE:-$cache}") || die "$name: the pinned LLVM and Rust tools could not be fetched (see above). This FAILS in CI too."
    llvm_path=$(sed -n 's/^PATH=//p' <<<"$tools_out"); llvm_libclang=$(sed -n 's/^LIBCLANG_PATH=//p' <<<"$tools_out")
    [[ -n $llvm_path && -n $llvm_libclang ]] || die "$name: kernel-llvm-tools.sh printed no PATH or LIBCLANG_PATH"
  fi
  resolve_board "$name" "$fam" "$res" "$defconfig" "$frags" "$toolchain" > "$scratch/$name.result" 2>&1 &
  pids+=("$!"); names+=("$name"); fams+=("$fam")
done < <(manifest)

for i in "${!pids[@]}"; do
  name=${names[$i]}
  rc=0
  wait "${pids[$i]}" || rc=$?
  result=$(tail -n 1 "$scratch/$name.result")
  if [[ $rc -ne 0 || $result != RESULT\ * ]]; then
    cat "$scratch/$name.result" >&2
    die "$name: its configuration could not be resolved (see above)"
  fi
  read -r _ board_fail nfrag <<<"$result"
  if [[ $board_fail -gt 0 ]]; then
    head -n -1 "$scratch/$name.result" >&2
    fails=$((fails + board_fail))
  else
    fam=${fams[$i]}
    kept_note=""
    grep -q "^check-kernel-fragments: $name: kept result" "$scratch/$name.result" && kept_note=", a kept result"
    echo "check-kernel-fragments: ok $name ($fam, $nfrag fragment file(s), every line takes$kept_note)"
  fi
  checked=$((checked + 1))
done

if [[ -n $only && $checked -eq 0 ]]; then
  die "--only $only: no such board with a kernel was checked"
fi
if [[ $fails -ne 0 ]]; then
  echo "check-kernel-fragments: FAILED: $fails kernel fragment line(s) do not take" >&2
  exit 1
fi
echo "check-kernel-fragments: passed ($checked kernel(s) checked, $skipped skipped)"
