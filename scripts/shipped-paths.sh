#!/usr/bin/env bash
# SHIPPED-PATHS (user approval 2026-11-27): a program Atlas ships carries no path of the CARGO_HOME it was built
# with. A dependency's panic locations name its source file, and cargo unpacks the sources under CARGO_HOME, so
# a release atlas-app named the builder's ~/.cargo, the user name in it. The release and installer builds
# (scripts/deb-fast.sh, scripts/bundle.sh) remap that prefix to /cargo, as Gaia's rust builder does for the board's
# programs (its build-rust node: --remap-path-prefix=<build>/cargo-home=/cargo).
#
#   source scripts/shipped-paths.sh; shipped_paths_env
#                         exports CARGO_ENCODED_RUSTFLAGS: the flags cargo would have used (CARGO_ENCODED_RUSTFLAGS,
#                         else RUSTFLAGS, else the host's CARGO_TARGET_<TRIPLE>_RUSTFLAGS) plus the remap, so a mold
#                         link flag or another flag the caller set is kept; and unsets RUSTFLAGS. Rustflags in a cargo
#                         config file are not merged: the encoded flags take their place.
#   scripts/shipped-paths.sh check
#                         the two-home check: a small program whose dependency names its source file is built with
#                         the remap from the CARGO_HOME in use and again from a second home (a link to the first at
#                         another path); the two programs must be the same bytes and hold neither home's path. It is
#                         also built once without the remap, which must hold the home's path: the probe still embeds
#                         a dependency path, so the check can fail. The gate runs it with the static checks.
set -uo pipefail

shipped_paths_home() {  # the CARGO_HOME cargo uses, as cargo gives it to rustc
  local home=${CARGO_HOME:-$HOME/.cargo}
  case $home in /*) ;; *) home=$PWD/$home ;; esac
  # Git Bash on Windows: rustc sees the Windows form of the path.
  if command -v cygpath >/dev/null 2>&1; then home=$(cygpath -w "$home"); fi
  printf '%s\n' "$home"
}

shipped_paths_env() {
  local remap flags host var
  remap="--remap-path-prefix=$(shipped_paths_home)=/cargo"
  if [ -n "${CARGO_ENCODED_RUSTFLAGS:-}" ]; then
    flags=$CARGO_ENCODED_RUSTFLAGS$'\x1f'$remap
  else
    host=$(rustc -vV | sed -n 's/^host: //p')
    var=CARGO_TARGET_$(printf '%s' "$host" | tr 'a-z.-' 'A-Z__')_RUSTFLAGS
    # The space-separated form cargo reads from RUSTFLAGS, one flag per field here.
    local fields=()
    read -r -a fields <<< "${RUSTFLAGS:-${!var:-}}"
    fields+=("$remap")
    flags=$(IFS=$'\x1f'; printf '%s' "${fields[*]}")
  fi
  unset RUSTFLAGS
  export CARGO_ENCODED_RUSTFLAGS=$flags
}

shipped_paths_check() {
  local root target home dir rc=0 a b plain
  root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
  target=${CARGO_TARGET_DIR:-$root/target}
  case $target in /*) ;; *) target=$root/$target ;; esac
  dir=$target/shipped-paths
  mkdir -p "$dir/probe/src"
  # The dependency is the one Cargo.lock holds, so the gate's offline home has it.
  local version
  version=$(awk '$0 == "name = \"smallvec\"" { getline; gsub(/version = |"/, ""); print; exit }' "$root/Cargo.lock")
  [ -n "$version" ] || { echo "shipped-paths: Cargo.lock holds no smallvec, the probe's dependency" >&2; return 1; }
  # Written only when different, so a build that is current stays current.
  # A new manifest (another smallvec in Cargo.lock) takes a new lock file.
  write() { [ "$(cat "$1" 2>/dev/null)" = "$2" ] || { printf '%s\n' "$2" > "$1"; rm -f -- "$dir/probe/Cargo.lock"; }; }
  write "$dir/probe/Cargo.toml" "[package]
name = \"shipped-paths-probe\"
version = \"0.0.0\"
edition = \"2024\"
publish = false

[dependencies]
smallvec = \"=$version\"

[profile.release]
strip = true

[workspace]"
  # SmallVec::insert asserts with its own file's location: a generic, so the location is compiled into this program.
  write "$dir/probe/src/main.rs" 'fn main() {
    let mut list: smallvec::SmallVec<[u8; 4]> = smallvec::SmallVec::new();
    list.insert(std::env::args().count(), 1);
    println!("{}", list.len());
}'
  home=$(shipped_paths_home)
  ln -sfn "$home" "$dir/home-b"
  build() {  # <name> <CARGO_HOME> <remap: 1|0>: builds the probe into its own target, prints the program's path
    local flags=()
    [ "$3" = 1 ] && flags=("--remap-path-prefix=$2=/cargo")
    ( cd "$dir/probe" || exit 1
      [ -f Cargo.lock ] || CARGO_HOME=$2 cargo generate-lockfile -q ) >&2 \
      && ( cd "$dir/probe" && unset RUSTFLAGS && CARGO_HOME=$2 CARGO_TARGET_DIR=$dir/target-$1 \
             CARGO_ENCODED_RUSTFLAGS=$(IFS=$'\x1f'; printf '%s' "${flags[*]}") \
             cargo build -q --release --locked ) >&2 \
      && printf '%s\n' "$dir/target-$1/release/shipped-paths-probe"
  }
  a=$(build a "$home" 1) && b=$(build b "$dir/home-b" 1) && plain=$(build plain "$home" 0) \
    || { echo "shipped-paths: the probe did not build" >&2; return 1; }
  if ! grep -q -a -F "$home" "$plain"; then
    echo "shipped-paths: the probe built without the remap holds no path of $home: it proves nothing" >&2; rc=1
  fi
  if ! cmp -s "$a" "$b"; then
    echo "shipped-paths: the probe built from $home and from $dir/home-b differs: a CARGO_HOME path is in it" >&2; rc=1
  fi
  # The flags the builds get: the caller's kept, the remap added.
  local got want
  got=$(RUSTFLAGS='-C link-arg=-fuse-ld=mold' CARGO_ENCODED_RUSTFLAGS='' bash -c '. "$1"; unset CARGO_ENCODED_RUSTFLAGS; shipped_paths_env; printf %s "$CARGO_ENCODED_RUSTFLAGS|${RUSTFLAGS-unset}"' shipped-paths-env "${BASH_SOURCE[0]}")
  want="-C"$'\x1f'"link-arg=-fuse-ld=mold"$'\x1f'"--remap-path-prefix=$home=/cargo|unset"
  [ "$got" = "$want" ] || { echo "shipped-paths: shipped_paths_env gave '${got//$'\x1f'/ }', not '${want//$'\x1f'/ }'" >&2; rc=1; }
  local path
  for path in "$home" "$dir/home-b"; do
    if grep -q -a -F "$path" "$a" || grep -q -a -F "$path" "$b"; then
      echo "shipped-paths: the remapped probe still holds $path" >&2; rc=1
    fi
  done
  grep -q -a -F '/cargo/registry/' "$a" || { echo "shipped-paths: the remapped probe holds no /cargo/registry/ path" >&2; rc=1; }
  [ "$rc" = 0 ] && echo "shipped-paths: two homes, one program ($(wc -c < "$a") bytes); the dependency's path reads /cargo"
  return "$rc"
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
  case ${1:-} in
    check) shipped_paths_check ;;
    *) sed -n '2,18p' "$0" >&2; exit 2 ;;
  esac
fi
