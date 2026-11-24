#!/usr/bin/env bash
# The working area: where worktrees, build output, caches and logs live on a developer machine, built from nothing by
# this script. Nothing about the area is kept only in someone's scratch: the layout, the environment each worktree
# runs with and the way long runs are started are all here.
#
#   ATLAS_AREA (default /mnt/nvme-wd/atlas)
#     wt/<name>        one git worktree per piece of work (branch pr/<name>)
#     target/<name>    its own CARGO_TARGET_DIR (never shared: cargo's mtime freshness mixes code between worktrees);
#                      on btrfs its own subvolume, as are tmp/<name> and cache/gaia-store (see make_dir)
#     tmp/<name>       its TMPDIR and gate scratch root (short, so test socket paths fit)
#     logs/<name>      the logs of its runs, each with the load at start and end
#     cache/           shared caches: gaia-store (GAIA_STORE), downloads (sccache stays in its own default, shared)
#
# Usage:
#   scripts/area.sh setup                         make the layout and say which tools are missing
#   scripts/area.sh worktree <name> [<base>]      add wt/<name> on branch pr/<name>: the existing one, else a new
#                                                 one from <base> (default next)
#   scripts/area.sh env <name>                    print the exports a run in wt/<name> uses (eval "$(... env x)")
#   scripts/area.sh run <name> <unit> -- <cmd...> run <cmd> in wt/<name> as a systemd user unit at low priority on
#                                                 the build CPUs, logging to logs/<name>/<unit>.log; .load
#                                                 holds the load at start and end, wall and CPU seconds
#
# Build runs use nice 19 on CPUs ATLAS_CPUS (default 6-31) with ATLAS_JOBS (default 13) jobs, as the shared host's
# rules ask. The machine's bench key is passed by path only (ATLAS_BENCH_KEY_DIR); its files are never read here.
set -euo pipefail

area=${ATLAS_AREA:-/mnt/nvme-wd/atlas}
cpus=${ATLAS_CPUS:-6-31}
jobs=${ATLAS_JOBS:-13}
key_dir=${ATLAS_BENCH_KEY_DIR:-/mnt/ssd-samsung/atlas-bench-key}
repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && git rev-parse --show-toplevel)

die() { echo "area: $*" >&2; exit 1; }

# On btrfs, each heavy directory is its own subvolume: in the disk's shared top-level subvolume every metadata read
# (a cargo freshness check, Gaia's lstat walk of a reused tree) queued behind other sessions' writers in
# btrfs_tree_read_lock (Gaia measured a 0.15-0.22 s walk at 0.4-4.6 s, and 94 s in a gate). Elsewhere, a directory.
make_dir() {  # <path>
  [[ -e $1 ]] && return 0
  mkdir -p "$(dirname "$1")"
  if [[ $(stat -f -c %T "$(dirname "$1")") == btrfs ]] && command -v btrfs >/dev/null 2>&1; then
    btrfs subvolume create "$1" >/dev/null
  else
    mkdir -p "$1"
  fi
}

name_ok() { [[ $1 =~ ^[A-Za-z0-9][A-Za-z0-9._-]*$ ]] || die "a name is letters, digits, '.', '_' or '-': '$1'"; }

setup() {
  mkdir -p "$area"/{wt,target,tmp,logs} "$area"/cache/downloads
  make_dir "$area/cache/gaia-store"
  local t missing=0
  for t in cargo rustc cargo-nextest sccache mold bun python3 git systemd-run taskset; do
    if ! command -v "$t" >/dev/null 2>&1; then
      echo "area: MISSING $t" >&2
      missing=1
    fi
  done
  echo "area: $area is laid out (wt target tmp logs cache)"
  [[ $missing = 0 ]] || echo "area: the gate falls back without sccache or mold, but is slower; the rest are needed" >&2
}

worktree() {  # <name> [<base>]
  local name=$1 base=${2:-next}
  name_ok "$name"
  setup >/dev/null
  [[ -e $area/wt/$name ]] && die "$area/wt/$name exists"
  if git -C "$repo" show-ref --verify --quiet "refs/heads/pr/$name"; then
    git -C "$repo" worktree add "$area/wt/$name" "pr/$name"
    base="its own head (pr/$name existed)"
  else
    git -C "$repo" worktree add -b "pr/$name" "$area/wt/$name" "$base"
  fi
  make_dir "$area/target/$name"
  make_dir "$area/tmp/$name"
  mkdir -p "$area/logs/$name"
  echo "area: wt/$name on pr/$name from $base; target, tmp and logs made"
}

env_of() {  # <name>
  local name=$1
  name_ok "$name"
  [[ -d $area/wt/$name ]] || die "no worktree $area/wt/$name (scripts/area.sh worktree $name)"
  cat <<EOF
export CARGO_TARGET_DIR=$area/target/$name
export TMPDIR=$area/tmp/$name
export ATLAS_SCRATCH_ROOT=$area/tmp/$name/scratch
export GAIA_STORE=$area/cache/gaia-store
export CARGO_BUILD_JOBS=$jobs
export ATLAS_BENCH_SEED_FILE=$key_dir/bench.seed
export ATLAS_BENCH_PUBKEY_FILE=$key_dir/bench.pub
EOF
}

run() {  # <name> <unit> -- <cmd...>
  local name=$1 unit=$2
  shift 2
  [[ ${1:-} = -- ]] || die "usage: area.sh run <name> <unit> -- <cmd...>"
  shift
  [[ $# -gt 0 ]] || die "no command"
  name_ok "$unit"
  local envs log
  envs=$(env_of "$name")
  log=$area/logs/$name/$unit.log
  mkdir -p "$area/logs/$name" "$area/tmp/$name"
  local setenv=() line
  while IFS= read -r line; do
    line=${line#export }
    setenv+=("--setenv=$line")
  done <<<"$envs"
  systemd-run --user --unit="atlas-$name-$unit" --collect -p LimitNOFILE=524288 \
    -p WorkingDirectory="$area/wt/$name" "${setenv[@]}" --setenv=PATH="$PATH" --setenv=HOME="$HOME" --setenv=CPUS="$cpus" \
    -- bash -c 'log=$1 load=$1.load; shift
      { date +%T; cat /proc/loadavg; } >"$load"
      rc=0
      /usr/bin/time -o "$load" -a -f "wall %e s, CPU %U s user + %S s system" \
        nice -n 19 taskset -c "$CPUS" "$@" >"$log" 2>&1 || rc=$?
      { date +%T; cat /proc/loadavg; echo "rc=$rc"; } >>"$load"
      echo "rc=$rc" >>"$log"
      exit "$rc"' area-run "$log" "$@" >/dev/null
  echo "area: started atlas-$name-$unit; log $log"
}

case ${1:-} in
  setup) setup ;;
  worktree) shift; [[ $# -ge 1 ]] || die "usage: area.sh worktree <name> [<base>]"; worktree "$@" ;;
  env) shift; [[ $# -eq 1 ]] || die "usage: area.sh env <name>"; env_of "$1" ;;
  run) shift; [[ $# -ge 3 ]] || die "usage: area.sh run <name> <unit> -- <cmd...>"; run "$@" ;;
  *) sed -n '2,/^set -euo/p' "${BASH_SOURCE[0]}" | sed '$d; s/^# \{0,1\}//'; exit 2 ;;
esac
