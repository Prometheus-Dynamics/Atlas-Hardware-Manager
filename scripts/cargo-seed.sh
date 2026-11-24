#!/usr/bin/env bash
# A fresh worktree's first build starts from a copy of the registry and git dependencies an earlier worktree built, so
# only Atlas's own crates compile. Measured interleaved on the gate PC, 2026-11-18, clippy + the test build of a fresh
# worktree: 152/145 s cold (271 crates) against 6+92/10+77 s seeded (40 crates).
#
# A seed holds NOTHING of the workspace's own crates. Their artifacts carry the building checkout's absolute paths
# (env!("CARGO_MANIFEST_DIR"), file!(), include_str!), while cargo fingerprints a path package relative to the
# workspace root and by mtime: a copied member artifact newer than this checkout's sources would be taken as fresh, and
# its tests would read the other worktree's files. So save keeps only deps/, build/ and .fingerprint/ of the profile
# directories, minus every unit of a workspace member (found by its fingerprint directory, <package>-<hash>, and removed
# with every file carrying that hash), and marks the seed .members-stripped; restore takes no seed without that mark.
#
# The dependencies are correct by cargo's own rules: a registry or git crate's fingerprint holds the rustc version, the
# profile, the features and its source (the same registry and git checkouts for every worktree), and a build script's
# recorded out dir is rewritten by cargo when the target moves; whatever does not match is rebuilt, never reused.
#
# Usage (the gate calls both when ATLAS_CARGO_SEED_DIR is set; nothing happens without it, so CI is untouched):
#   scripts/cargo-seed.sh restore   when the target dir is absent or empty: copy in the seed for this key (or the newest
#                                   seed when there is none for it: cargo rebuilds what differs)
#   scripts/cargo-seed.sh save      after a green build: keep this target as the seed for its key, unless one exists
#
# The key: Cargo.lock, Cargo.toml's [profile.*] sections, rust-toolchain.toml and `rustc -vV`. Seeds live in
# $ATLAS_CARGO_SEED_DIR (put it on the targets' filesystem: the copy is then a reflink, near free, on btrfs or XFS).
# The two newest seeds are kept. Every write is under a lock, and a seed is renamed into place only when complete.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

die() { echo "cargo-seed: $*" >&2; exit 1; }
seeds=${ATLAS_CARGO_SEED_DIR:-}
[[ -n $seeds ]] || exit 0
target=${CARGO_TARGET_DIR:-$(pwd)/target}
mkdir -p "$seeds"

key=$( { cat Cargo.lock; awk '/^\[profile/{p=1} /^\[/{if ($0 !~ /^\[profile/) p=0} p' Cargo.toml
         cat rust-toolchain.toml 2>/dev/null || true; rustc -vV; } | sha256sum | cut -c1-16)

# Removes every unit of a workspace member from a copied target: its fingerprint directory and every file in deps/ and
# build/ carrying the same hash. Fails when a member unit would remain.
strip_members() {  # <seed dir>
  local seed=$1
  cargo metadata --format-version 1 --locked --offline --no-deps | python3 -I scripts/seed-strip-members.py "$seed"
}

case ${1:-} in
  restore)
    if [[ -d $target ]] && [[ -n $(ls -A "$target" 2>/dev/null) ]]; then exit 0; fi
    (
      flock -s -w 600 9 || die "the seed lock was not free in 600 s"
      src=$seeds/$key
      [[ -f $src/.seed-complete && -f $src/.members-stripped ]] \
        || src=$(ls -1dt "$seeds"/*/ 2>/dev/null | while read -r d; do [[ -f $d/.seed-complete && -f $d/.members-stripped ]] && { echo "${d%/}"; break; }; done)
      if [[ -z $src ]]; then echo "cargo-seed: no seed yet; a cold build" >&2; exit 0; fi
      mkdir -p "$(dirname "$target")"
      rm -rf "$target"
      cp -a --reflink=auto "$src" "$target"
      rm -f "$target/.seed-complete" "$target/.members-stripped"
      echo "cargo-seed: $target starts from the seed $(basename "$src")$([[ $(basename "$src") == "$key" ]] || echo ' (another key: cargo rebuilds what differs)')" >&2
    ) 9>"$seeds/.lock"
    ;;
  save)
    [[ -d $target ]] || exit 0
    (
      flock -w 600 9 || die "the seed lock was not free in 600 s"
      [[ -f $seeds/$key/.seed-complete && -f $seeds/$key/.members-stripped ]] && exit 0
      tmp=$seeds/.new-$key.$$
      rm -rf "$tmp"
      mkdir -p "$tmp"
      for prof in "$target"/*/; do
        prof=${prof%/}
        [[ -d $prof/.fingerprint ]] || continue
        mkdir -p "$tmp/${prof##*/}"
        for d in .fingerprint deps build; do
          [[ -d $prof/$d ]] && cp -a --reflink=auto "$prof/$d" "$tmp/${prof##*/}/$d"
        done
      done
      strip_members "$tmp"
      touch "$tmp/.members-stripped" "$tmp/.seed-complete"
      rm -rf "${seeds:?}/$key"
      mv "$tmp" "$seeds/$key"
      # The two newest complete seeds stay; older ones go.
      ls -1dt "$seeds"/*/ | tail -n +3 | while read -r d; do rm -rf "${d%/}"; done
      echo "cargo-seed: kept $target as the seed $key" >&2
    ) 9>"$seeds/.lock"
    ;;
  *) die "usage: cargo-seed.sh restore|save" ;;
esac
