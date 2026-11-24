#!/usr/bin/env python3
# Removes every unit of a workspace member from a copied cargo target (a seed): its fingerprint directory and every file
# in deps/ and build/ carrying the same hash. Reads `cargo metadata --no-deps` on stdin. Used by scripts/cargo-seed.sh.
#
# It fails when a member unit would remain, and also when its input says nothing: no metadata, no workspace member, or
# a seed with no profile to strip. A seed that was never stripped must never be kept as if it were.
import json
import os
import re
import shutil
import sys

if len(sys.argv) != 2:
    sys.exit("usage: cargo metadata --format-version 1 --no-deps | seed-strip-members.py <seed dir>")
seed = sys.argv[1]
try:
    members = {p["name"] for p in json.load(sys.stdin)["packages"]}
except (ValueError, KeyError, TypeError) as e:
    sys.exit(f"cargo-seed: cargo metadata on stdin is not readable ({type(e).__name__}: {e})")
if not members:
    sys.exit("cargo-seed: cargo metadata lists no workspace member")
UNIT = re.compile(r"(.+)-([0-9a-f]{16})")
HASH = re.compile(r"-([0-9a-f]{16})(\.|$)")


def remove(p):
    if os.path.isdir(p) and not os.path.islink(p):
        shutil.rmtree(p)
    else:
        os.remove(p)


left = profiles = 0
for prof in os.listdir(seed):
    fp = os.path.join(seed, prof, ".fingerprint")
    if not os.path.isdir(fp):
        continue
    profiles += 1
    hashes = set()
    for d in os.listdir(fp):
        m = UNIT.fullmatch(d)
        if m and m.group(1) in members:
            hashes.add(m.group(2))
            shutil.rmtree(os.path.join(fp, d))
    for sub in ("deps", "build"):
        base = os.path.join(seed, prof, sub)
        for f in os.listdir(base) if os.path.isdir(base) else []:
            m = HASH.search(f)
            if m and m.group(1) in hashes:
                remove(os.path.join(base, f))
    # What stays must belong to a unit that is not a member: every hash in deps/ and build/ has a fingerprint left.
    kept = set()
    for d in os.listdir(fp):
        m = UNIT.fullmatch(d)
        if m:
            kept.add(m.group(2))
            if m.group(1) in members:
                left += 1
    for sub in ("deps", "build"):
        base = os.path.join(seed, prof, sub)
        for f in os.listdir(base) if os.path.isdir(base) else []:
            m = HASH.search(f)
            if m and m.group(1) not in kept:
                remove(os.path.join(base, f))
if not profiles:
    sys.exit(f"cargo-seed: {seed} holds no profile with a .fingerprint directory: nothing to keep")
if left:
    sys.exit(f"cargo-seed: {left} workspace member unit(s) left in the seed")
