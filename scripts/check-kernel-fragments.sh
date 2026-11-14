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
#
# Environment:
#   ATLAS_KERNEL_CACHE     where pinned kernel trees are kept (default ${XDG_CACHE_HOME:-$HOME/.cache}/atlas/kernel).
#                          CI caches this directory keyed by the kernel pins (.github/workflows/next.yml).
#   ATLAS_KERNEL_SKIP=1    locally only: skip the whole check, printing that it was skipped. A source that cannot be
#                          fetched is a FAIL otherwise, in CI as well: a kernel configuration that cannot be
#                          resolved is not a pass.
#   KEEP_KERNEL_SCRATCH=1  keep the scratch O= directories (default: removed).
#
# A board whose kernel needs a patch series (a source named "*series*" in its family) is listed as SKIPPED: this
# check does not apply patch series, and the base tree alone cannot resolve such a board's fragments.
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
devices=$repo/devices
mode=check
while [[ $# -gt 0 ]]; do
  case $1 in
    --self-test) mode=self-test; shift ;;
    --devices) devices=$(cd "$2" && pwd); shift 2 ;;
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
  "$0" --devices "$tmp/devices" >"$out" 2>&1 || rc=$?
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
  echo "check-kernel-fragments: self-test passed (the real check passes; a line that cannot take fails, naming it; a bare '#' line fails, naming its file and line)"
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
    print("|".join([name, fam_name, src["git"], commit_of(name, src_name, src), defconfig, ",".join(frags), series_pin]))
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
  local bad=0 row name fam url ref defconfig frags series_pin f num
  local -a fa
  while IFS= read -r row; do
    IFS='|' read -r name fam url ref defconfig frags series_pin <<<"$row"
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

fails=0
checked=0
skipped=0
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
  IFS='|' read -r name fam url ref defconfig frags series_pin <<<"$row"
  if [[ $fam == SKIP && $url == NO-DEFCONFIG ]]; then
    die "$name: its kernel states no \`defconfig\` (board.toml or the family's [kernel]). Buildroot cannot build a kernel without a starting configuration; state it."
  fi
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
  if [[ -n ${series_pin:-} ]]; then
    surl=${series_pin%%|*}; sref=${series_pin#*|}
    fam_key=$(printf '%s %s %s' "$url" "$ref" "$surl $sref" | sha256sum | cut -c1-16)
    if [[ -z ${patched[$fam_key]:-} ]]; then
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
      pdir=$scratch/patched-$fam_key
      echo "check-kernel-fragments: applying $name's patch series (the series' own method: patch -p1 --fuzz=0, in its order) to a copy of $url at $ref" >&2
      rm -rf "$pdir"; cp -a "$src" "$pdir"
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
      patched[$fam_key]=$pdir
    fi
    res=${patched[$fam_key]}
  fi
  o=$scratch/$name
  mkdir -p "$o"
  target=$defconfig
  [[ $defconfig == defconfig ]] || target=${defconfig}_defconfig
  (cd "$res" && make -s ARCH=arm64 O="$o" "$target") >"$scratch/$name.log" 2>&1 \
    || die "$name: the $target step failed (log: $scratch/$name.log)"
  IFS=',' read -ra farr <<<"$frags"
  (cd "$res" && scripts/kconfig/merge_config.sh -m -O "$o" "$o/.config" "${farr[@]}") >>"$scratch/$name.log" 2>&1 \
    || die "$name: merge_config failed (log: $scratch/$name.log)"
  (cd "$res" && make -s ARCH=arm64 O="$o" olddefconfig) >>"$scratch/$name.log" 2>&1 \
    || die "$name: olddefconfig failed (log: $scratch/$name.log)"
  board_fail=0
  for f in "${farr[@]}"; do
    out=$(check_fragment "$o/.config" "$f" "$name" "$devices")
    if [[ -n $out ]]; then
      echo "$out" >&2
      board_fail=$((board_fail + $(printf '%s\n' "$out" | wc -l)))
    fi
  done
  if [[ $board_fail -gt 0 ]]; then
    fails=$((fails + board_fail))
  else
    echo "check-kernel-fragments: ok $name ($fam, ${#farr[@]} fragment file(s), every line takes)"
  fi
  checked=$((checked + 1))
done < <(manifest)

if [[ $fails -ne 0 ]]; then
  echo "check-kernel-fragments: FAILED: $fails kernel fragment line(s) do not take" >&2
  exit 1
fi
echo "check-kernel-fragments: passed ($checked kernel(s) checked, $skipped skipped)"
