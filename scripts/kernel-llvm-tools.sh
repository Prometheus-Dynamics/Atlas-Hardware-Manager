#!/usr/bin/env bash
# The LLVM toolchain a kernel with Rust is configured with (coordinator's pins, 2026-11-18), fetched by content into
# the kernel cache and never from the system: rustc 1.99.0 with rust-src, bindgen-cli 0.73.2 and LLVM 23.1.5 (clang,
# ld.lld and libclang for bindgen). Each archive is named by its URL AND its sha256 here; a download whose sha256
# differs is refused and removed. Nothing is installed outside the cache, and no system toolchain is used in their place.
#
# Usage: scripts/kernel-llvm-tools.sh [CACHE]    prints the prefix to put first on PATH, and LIBCLANG_PATH, as
#                                                 two lines "PATH=<dir>" and "LIBCLANG_PATH=<dir>"
# CACHE defaults to ${ATLAS_KERNEL_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/atlas/kernel}. The tools live under
# CACHE/llvm-tools/<key>/, where <key> is the sha256 of the pins and LAYOUT below: a changed pin is a new directory, and a directory is
# used only once its "complete" marker is written (an interrupted fetch is redone, never half-used).
#
# bindgen-cli is a crate (source): its .crate is fetched by its crates.io checksum, then built with
# `cargo install --locked` from that source with the fetched rustc (its own Cargo.lock pins its dependencies, and
# cargo checks each one's checksum from the registry index). It is the one step that reaches crates.io beyond the pinned
# archive itself.
set -euo pipefail

RUST_URL=https://static.rust-lang.org/dist/2026-10-01/rust-1.99.0-x86_64-unknown-linux-gnu.tar.xz
RUST_SHA=891c6366d7100feda0bca4c03ce63f3c9ac827cbebbc283e7433061d42c6a376
RUST_SRC_URL=https://static.rust-lang.org/dist/2026-10-01/rust-src-1.99.0.tar.xz
RUST_SRC_SHA=3f1f9b7ed48f4596fc87889b7b3c61747336a55c9c22db1ab0c697e0aadb77aa
BINDGEN_URL=https://static.crates.io/crates/bindgen-cli/0.73.2/download
BINDGEN_SHA=5622e043625b6d1f32c84f53699690a44d1419e657879de26eb252699f5ebb7c
LLVM_URL=https://github.com/llvm/llvm-project/releases/download/llvmorg-23.1.5/LLVM-23.1.5-Linux-X64.tar.xz
LLVM_SHA=f72edb0b36580ece7f52e23858f00c18fad4e87395ea7ad05d7e55c39f8fc78a

die() { echo "kernel-llvm-tools: $*" >&2; exit 1; }
for t in curl tar xz sha256sum; do command -v "$t" >/dev/null || die "$t is needed"; done

cache=${1:-${ATLAS_KERNEL_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/atlas/kernel}}
# The key: the pins and LAYOUT (raise it when the steps below change what the directory holds), not the whole file, so
# a comment edit does not fetch 2 GB again.
LAYOUT=2
key=$(printf '%s\n' "$LAYOUT" "$RUST_URL" "$RUST_SHA" "$RUST_SRC_URL" "$RUST_SRC_SHA" "$BINDGEN_URL" "$BINDGEN_SHA" "$LLVM_URL" "$LLVM_SHA" | sha256sum | cut -c1-32)
root=$cache/llvm-tools/$key
prefix=$root/prefix

emit() {
  echo "PATH=$prefix/bin:$root/llvm/bin"
  echo "LIBCLANG_PATH=$root/llvm/lib"
}
if [[ -f $root/complete ]]; then emit; exit 0; fi

rm -rf "$root"
mkdir -p "$root/dl" "$prefix"
work=$(mktemp -d "${TMPDIR:-$root}/llvm-tools-XXXXXX")
trap 'rm -rf "$work"' EXIT

# Fetches URL to $root/dl/NAME and refuses it unless its sha256 is SHA. Retries a host that does not answer.
fetch() {  # url sha name
  local url=$1 sha=$2 name=$3 got
  curl -fsSL --retry 3 --retry-delay 10 --connect-timeout 30 --speed-time 60 --speed-limit 1024 -o "$root/dl/$name.part" "$url" \
    || die "fetching $url failed"
  got=$(sha256sum "$root/dl/$name.part" | cut -d' ' -f1)
  if [[ $got != "$sha" ]]; then
    rm -f "$root/dl/$name.part"
    die "$url: sha256 $got, pinned $sha: refused"
  fi
  mv "$root/dl/$name.part" "$root/dl/$name"
  echo "kernel-llvm-tools: $name ok (sha256 $sha)" >&2
}

fetch "$RUST_URL" "$RUST_SHA" rust.tar.xz
fetch "$RUST_SRC_URL" "$RUST_SRC_SHA" rust-src.tar.xz
fetch "$BINDGEN_URL" "$BINDGEN_SHA" bindgen-cli.crate
fetch "$LLVM_URL" "$LLVM_SHA" llvm.tar.xz

# rustc, cargo and the host std from the rust archive, and rust-src, into one prefix by their own installers (local
# copies only: install.sh reads the archive's files and writes under --prefix).
tar -xJf "$root/dl/rust.tar.xz" -C "$work"
tar -xJf "$root/dl/rust-src.tar.xz" -C "$work"
bash "$work"/rust-1.99.0-x86_64-unknown-linux-gnu/install.sh --prefix="$prefix" --disable-ldconfig \
  --components=rustc,cargo,rust-std-x86_64-unknown-linux-gnu >/dev/null
bash "$work"/rust-src-1.99.0/install.sh --prefix="$prefix" --disable-ldconfig >/dev/null

# Only what an LLVM=1 kernel configure and build calls (the kernel Makefile's CC, LD, AR, NM, OBJCOPY, OBJDUMP, READELF,
# STRIP and HOSTCC/HOSTCXX), clang's own headers, and libclang for bindgen: about 1 GB of the archive's 12 GB.
# The prebuilt's binaries are statically linked against LLVM (no libLLVM to carry).
mkdir -p "$root/llvm"
tar -xJf "$root/dl/llvm.tar.xz" -C "$root/llvm" --strip-components=1 --wildcards \
  '*/bin/clang' '*/bin/clang-[0-9]*' '*/bin/clang++' '*/bin/lld' '*/bin/ld.lld' '*/bin/llvm-ar' '*/bin/llvm-nm' \
  '*/bin/llvm-objcopy' '*/bin/llvm-objdump' '*/bin/llvm-readobj' '*/bin/llvm-readelf' '*/bin/llvm-strip' \
  '*/lib/clang/*' '*/lib/libclang.so*'

mkdir -p "$work/bindgen"
tar -xzf "$root/dl/bindgen-cli.crate" -C "$work/bindgen" --strip-components=1
CARGO_HOME=$root/cargo-home RUSTUP_TOOLCHAIN= PATH="$prefix/bin:$PATH" CARGO_TARGET_DIR=$work/bindgen-target \
  "$prefix/bin/cargo" install --locked --path "$work/bindgen" --root "$prefix" --quiet \
  || die "building bindgen-cli 0.73.2 failed"
rm -rf "$root/cargo-home/registry/src"

"$prefix/bin/rustc" --version | grep -q '^rustc 1\.99\.0 ' || die "rustc is not 1.99.0"
LIBCLANG_PATH=$root/llvm/lib "$prefix/bin/bindgen" --version | grep -q '^bindgen 0\.73\.2' || die "bindgen is not 0.73.2"
"$root/llvm/bin/clang" --version | grep -q 'clang version 23\.1\.5' || die "clang is not 23.1.5"
# The archives are checked and unpacked: they go (the lean rule), the key keeps their hashes.
rm -rf "$root/dl"
touch "$root/complete"
emit
