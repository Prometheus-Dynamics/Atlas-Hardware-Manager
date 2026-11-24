#!/usr/bin/env python3
# Prints the workspace crates a change touches: each changed file belongs to the crate whose directory holds it, and
# the selection is those crates plus every workspace crate that depends on one of them (reverse dependencies,
# transitively). A change to Cargo.toml, Cargo.lock or rust-toolchain.toml selects every crate. Reads
# `cargo metadata --no-deps` on stdin; the changed files are argument 1, one per line. Used by scripts/check-scoped.sh.
#
# It fails when its input says nothing (no metadata, no workspace member, no changed file), so a broken pipe never
# reads as "no crate changed". A change that touches no crate prints nothing and passes.
import json
import os
import sys

if len(sys.argv) != 2:
    sys.exit("usage: cargo metadata --format-version 1 --no-deps | scoped-crates.py <changed files>")
try:
    meta = json.load(sys.stdin)
    root = meta["workspace_root"]
    pkgs = {p["name"]: p for p in meta["packages"]}
except (ValueError, KeyError, TypeError) as e:
    sys.exit(f"check-scoped: cargo metadata on stdin is not readable ({type(e).__name__}: {e})")
if not pkgs:
    sys.exit("check-scoped: cargo metadata lists no workspace member")
changed = [line for line in sys.argv[1].splitlines() if line]
if not changed:
    sys.exit("check-scoped: no changed file given")
if any(f in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml") for f in changed):
    print("\n".join(sorted(pkgs)))
    sys.exit()
dirs = {name: os.path.relpath(os.path.dirname(p["manifest_path"]), root) for name, p in pkgs.items()}
hit = {n for n, d in dirs.items() for f in changed if f == d or f.startswith(d + "/")}
rdeps = {n: set() for n in pkgs}
for n, p in pkgs.items():
    for d in p["dependencies"]:
        if d.get("path") and d["name"] in pkgs:
            rdeps[d["name"]].add(n)
todo, sel = list(hit), set(hit)
while todo:
    for r in rdeps[todo.pop()]:
        if r not in sel:
            sel.add(r)
            todo.append(r)
print("\n".join(sorted(sel)))
