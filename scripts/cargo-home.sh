#!/usr/bin/env bash
# The gate's own CARGO_HOME (coordinator ruling 2026-11-27, as Lemnos did in pr/PKG-HOME): the gate never waits on
# another program's lock on ~/.cargo's package cache, and never downloads while a lane runs.
#
#   scripts/cargo-home.sh fetch [LANE...]
#                                   fills $ATLAS_CARGO_HOME, online, from every committed lock file, prints what it
#                                   holds, and makes each LANE its own home, $ATLAS_CARGO_HOME.lanes/LANE; the gate
#                                   runs it before any lane when ATLAS_CARGO_HOME is set
#
# A home per lane: cargo takes the package cache's lock for writing while it resolves, even offline, so the gate's
# lanes in one home waited on each other (2026-11-27, one shared home: 18 "Blocking waiting for file lock on package
# cache" lines in one gate). Each lane home holds a copy of the fetched crates (reflinks on btrfs) and unpacks its own
# sources. Its path stays the same from fetch to fetch, and only its contents follow the fetch's key: a crate's source
# path is part of what cargo builds it with, so a home per key would move every dependency's path on each lock change
# and every lane would rebuild all of them (the move to this home rebuilt ~200-270 crates per lane, a 798 s gate).
#
# The home's config.toml sets cargo offline and turns its automatic cleaning of the package cache off (written here
# when missing or different). Each fetch writes fetch-<key>.log in the home: each lock file's sha256, the sources (the
# registry, each git source at its commit), the number of registry crate files and a sha256 over their sums. The key
# is the lock files' hashes, the config and the Gaia pin: a home already fetched for that key is not fetched again.
# The Gaia binary the devices lane runs is built here too, when the pin's build is missing (scripts/check-devices.sh
# --gaia-only), so its clone and its crates are fetched before the lanes, not in one.
# ~/.cargo itself is never written: only its programs (the cargo and rustup proxies) are run.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

home=${ATLAS_CARGO_HOME:?ATLAS_CARGO_HOME names the gate\'s own CARGO_HOME}
[ "$home" != "$HOME/.cargo" ] || { echo "cargo-home: ATLAS_CARGO_HOME is ~/.cargo; it must be the repo's own" >&2; exit 2; }
[ "${1:-}" = fetch ] || { sed -n '2,26p' "$0" >&2; exit 2; }
shift
lanes=("$@")

mkdir -p "$home"
config="# Written by scripts/cargo-home.sh: the gate's own CARGO_HOME, offline, never cleaned automatically.
[net]
offline = true
[cache]
auto-clean-frequency = \"never\""
[ "$(cat "$home/config.toml" 2>/dev/null)" = "$config" ] || printf '%s\n' "$config" > "$home/config.toml"

mapfile -t locks < <(git ls-files '*Cargo.lock')
gaia_commit=$(sed -n 's/^commit = "\(.*\)"$/\1/p' devices/gaia.lock 2>/dev/null || true)
key=$( { for lock in "${locks[@]}"; do sha256sum "$lock"; done; printf '%s\n' "$config" "gaia $gaia_commit"; } | sha256sum | cut -c1-16)
log=$home/fetch-$key.log

summary() { grep -E '^fetch: ' "$log" | grep -vE '^fetch: (CARGO_HOME|gaia:)' | cut -c1-240; }
# Each lane's home: the fetched crates copied in (reflinks where the disk has them) when its key is not this fetch's.
lane_homes() {
  local lane dst
  for lane in "${lanes[@]}"; do
    [[ $lane =~ ^[a-z][a-z0-9-]*$ ]] || { echo "cargo-home: a lane is lower-case letters, digits and '-': '$lane'" >&2; return 1; }
    dst=$home.lanes/$lane
    [ "$(cat "$dst/.fetch-key" 2>/dev/null)" = "$key" ] && continue
    mkdir -p "$dst/registry"
    rm -f -- "$dst/.fetch-key"
    cp -a --reflink=auto "$home/registry/cache" "$home/registry/index" "$dst/registry/" \
      && cp -a --reflink=auto "$home/config.toml" "$dst/" \
      && { [ ! -d "$home/git" ] || cp -a --reflink=auto "$home/git" "$dst/"; } \
      || { echo "cargo-home: the $lane lane's home could not be made ($dst)" >&2; return 1; }
    echo "$key" > "$dst/.fetch-key"
    echo "cargo home: the $lane lane's home now holds fetch $key ($dst)"
  done
}
if [ -f "$log" ] && grep -q '^fetch: done' "$log"; then
  echo "cargo home: $home, fetched for key $key"
  summary
  lane_homes
  exit
fi

fetch() {
  local lock rc=0
  echo "fetch: CARGO_HOME=$home key $key"
  for lock in "${locks[@]}"; do
    echo "fetch: $lock sha256 $(sha256sum "$lock" | cut -d' ' -f1)"
    CARGO_HOME=$home cargo fetch --locked --config net.offline=false --manifest-path "$(dirname "$lock")/Cargo.toml" \
      || { echo "fetch: $lock: cargo fetch failed"; rc=1; }
  done
  echo "fetch: sources: $(sed -n 's/^source = "\([a-z]*+[^?#"]*\)[^#"]*\(#[0-9a-f]*\)\{0,1\}".*/\1\2/p' "${locks[@]}" | sort -u | tr '\n' ' ')"
  local crates
  crates=$(cd "$home/registry/cache" 2>/dev/null && find . -name '*.crate' -print0 | LC_ALL=C sort -z | xargs -0r sha256sum)
  echo "fetch: $(printf '%s' "$crates" | grep -c .) registry crate files, sha256 over their sums $(printf '%s\n' "$crates" | sha256sum | cut -c1-64)"
  echo "fetch: $(find "$home/git/checkouts" -mindepth 2 -maxdepth 2 -type d 2>/dev/null | wc -l) git checkouts"
  # The pinned Gaia, built online here when its build is missing, so the devices lane finds it built.
  if [ -n "$gaia_commit" ] && [ -x scripts/check-devices.sh ]; then
    CARGO_HOME=$home CARGO_NET_OFFLINE=false scripts/check-devices.sh --gaia-only 2>&1 | sed 's/^/fetch: gaia: /' \
      || { echo "fetch: the pinned Gaia could not be built"; rc=1; }
  fi
  [ "$rc" = 0 ] && echo "fetch: done"
  return "$rc"
}
fetch > "$log.part" 2>&1 || { echo "cargo-home: the gate's CARGO_HOME could not be filled ($log.part):" >&2; tail -5 "$log.part" >&2; exit 1; }
mv -f "$log.part" "$log"
echo "cargo home: $home, fetched now for key $key"
summary
lane_homes
