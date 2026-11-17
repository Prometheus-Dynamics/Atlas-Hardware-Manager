#!/usr/bin/env bash
# Writes the PC's SSH public key for a named input `pc-key` with its comment removed: only `ssh-ed25519 <base64>`
# goes to OUT. The comment is the PC owner's (often an email address), and no file the image or the repo holds may carry
# it, so the comment is dropped here, on the Atlas side, before Gaia reads the input (Gaia copies a named input as
# written).
#
#   scripts/pc-key-strip.sh IN.pub OUT
#
# Refused (exit 1, nothing written): a private key, anything that is not exactly one line, and any line that is not an
# ssh-ed25519 public key. Only ssh-ed25519 is accepted: the bench's login is that one type.
set -euo pipefail
die() { echo "pc-key-strip: $*" >&2; exit 1; }
[[ $# -eq 2 ]] || die "usage: scripts/pc-key-strip.sh IN.pub OUT"
in=$1 out=$2
[[ -f $in ]] || die "no such key file: $in"
grep -q 'PRIVATE KEY' "$in" && die "$in is a PRIVATE key: give the .pub file"
lines=$(grep -c . "$in" || true)
[[ $lines -eq 1 ]] || die "$in has $lines non-empty lines: give the one public key line"
tmp=$(mktemp "$out.XXXXXX")
trap 'rm -f "$tmp"' EXIT
awk '{print $1" "$2}' "$in" > "$tmp"
grep -Eq '^ssh-ed25519 [A-Za-z0-9+/]+=*$' "$tmp" || die "$in is not an ssh-ed25519 public key line"
mv "$tmp" "$out"
trap - EXIT
