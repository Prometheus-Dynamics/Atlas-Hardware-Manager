#!/usr/bin/env bash
# Frees the cargo target dirs of worktrees whose work has landed. A target dir is a cache: removing one costs a rebuild,
# never work. What must never happen is removing one in use, so a target is removed only when ALL of these hold:
#   - its worktree's HEAD is already in origin/next (merged), or the worktree its .atlas-worktree names is gone;
#   - cargo's own build lock in it (debug/.cargo-lock, release/.cargo-lock) can be taken right now (no cargo is running
#     in it), and is held while the directory is renamed away;
#   - no process has its working directory inside the target or its worktree (read from /proc, no signals sent).
# A removed target is first renamed (atomic), then deleted; each removal is logged.
#
# Usage:
#   scripts/prune-targets.sh [--apply] [--log FILE] TARGET...   print what would go (default), or remove it (--apply)
#
# Which worktree a target belongs to: the file .atlas-worktree in it (the gate writes it), else the scratch layout this
# repository's workers use: <dir>/<name>-scratch-<ID>/target belongs to the worktree <dir>/<name>-<ID>.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

apply=0 log=""
targets=()
while [[ $# -gt 0 ]]; do
  case $1 in
    --apply) apply=1 ;;
    --log) log=$2; shift ;;
    -h|--help) sed -n '2,15p' "$0"; exit 0 ;;
    *) targets+=("$1") ;;
  esac
  shift
done
[[ ${#targets[@]} -gt 0 ]] || { echo "prune-targets: name the target dirs to consider" >&2; exit 2; }

git fetch -q origin next 2>/dev/null || echo "prune-targets: could not fetch origin/next; using the local ref" >&2
mapfile -t worktrees < <(git worktree list --porcelain | sed -n 's/^worktree //p')

owner_of() {  # <target> -> the worktree path it belongs to
  local t=$1 p
  if [[ -f $t/.atlas-worktree ]]; then cat "$t/.atlas-worktree"; return; fi
  p=$(dirname "$t")
  if [[ $(basename "$p") == *-scratch-* ]]; then
    local b=${p##*/}
    echo "$(dirname "$p")/${b%%-scratch-*}-${b##*-scratch-}"
  fi
}

busy_cwd() {  # <dir>...: 0 when some process has its cwd inside one of them
  local c d
  for c in /proc/[0-9]*/cwd; do
    c=$(readlink "$c" 2>/dev/null) || continue
    for d in "$@"; do
      [[ -n $d && ( $c == "$d" || $c == "$d"/* ) ]] && return 0
    done
  done
  return 1
}

freed=0
for t in "${targets[@]}"; do
  t=$(realpath -m "$t")
  [[ -d $t ]] || continue
  w=$(owner_of "$t")
  why=""
  if [[ -z $w ]]; then
    echo "keep  $t: its worktree is not known (no .atlas-worktree, not the scratch layout)"; continue
  fi
  listed=0
  for x in "${worktrees[@]}"; do [[ $x == "$w" ]] && listed=1; done
  if [[ $listed -eq 1 ]]; then
    head=$(git -C "$w" rev-parse HEAD 2>/dev/null) || { echo "keep  $t: cannot read $w's HEAD"; continue; }
    if git merge-base --is-ancestor "$head" origin/next; then
      why="its worktree's HEAD ${head:0:8} is in origin/next"
    else
      echo "keep  $t: $w has work not in origin/next"; continue
    fi
  elif [[ -f $t/.atlas-worktree ]]; then
    why="no worktree of this repository is at $w any more (named by its .atlas-worktree)"
  else
    echo "keep  $t: no worktree at $w, and only the directory name points there (no .atlas-worktree)"; continue
  fi
  if busy_cwd "$t" "$w"; then
    echo "keep  $t: a process is working inside it or its worktree"; continue
  fi
  size=$(du -sh "$t" 2>/dev/null | cut -f1)
  if [[ $apply -eq 0 ]]; then
    echo "would remove $t ($size): $why"; continue
  fi
  # Hold cargo's own build locks while the directory is renamed away: a cargo starting now waits, then finds no target.
  locks=()
  for l in "$t"/debug/.cargo-lock "$t"/release/.cargo-lock; do [[ -e $l ]] && locks+=("$l"); done
  gone=$t.pruning.$$
  if ! (
    for i in "${!locks[@]}"; do
      exec {fd}<>"${locks[$i]}"
      flock -n "$fd" || exit 1
    done
    busy_cwd "$t" "$w" && exit 1
    mv "$t" "$gone"
  ); then
    echo "keep  $t: cargo is building in it (its lock is held) or a process started inside it"; continue
  fi
  rm -rf "$gone"
  freed=$((freed + 1))
  line="$(date -Is) removed $t ($size): $why; cargo lock free, no process inside"
  echo "$line"
  [[ -n $log ]] && echo "$line" >> "$log"
done
[[ $apply -eq 1 ]] && echo "prune-targets: removed $freed target dir(s)"
exit 0
