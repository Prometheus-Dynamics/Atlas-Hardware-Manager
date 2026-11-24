#!/usr/bin/env python3
# Prints the workspace crates whose library has a doc example, one per line: the only crates `cargo test --doc` has
# anything to run for. Reads `cargo metadata --no-deps` on stdin. Used by scripts/gate.sh (the tests lane).
#
# A doc example is a fenced block in a doc comment (/// or //!) of a .rs file under the library's source directory,
# or any `#[doc = include_str!(...)]` / `#![doc = include_str!(...)]` (its file is not scanned, so the crate is
# always kept). Running rustdoc for the ~40 crates with none cost ~40 s of the gate and ran no test. A crate that
# gains an example is selected by the same scan, so nothing is skipped by a stale list.
#
# It fails when its input says nothing (no metadata, no library at all).
import json
import os
import re
import sys

FENCE = re.compile(r"^\s*//[/!].*```")
INCLUDE = re.compile(r"#!?\[doc\s*=\s*include_str!")
try:
    meta = json.load(sys.stdin)
    packages = meta["packages"]
except (ValueError, KeyError, TypeError) as e:
    sys.exit(f"doc-crates: cargo metadata on stdin is not readable ({type(e).__name__}: {e})")
libs = 0
for p in sorted(packages, key=lambda p: p["name"]):
    lib = next((t for t in p["targets"] if "lib" in t["kind"] or "proc-macro" in t["kind"]), None)
    if lib is None or not lib.get("doctest", True):
        continue
    libs += 1
    src = os.path.dirname(lib["src_path"])
    found = False
    for root, _dirs, files in os.walk(src):
        for f in files:
            if not f.endswith(".rs"):
                continue
            with open(os.path.join(root, f), encoding="utf-8") as fh:
                for line in fh:
                    if FENCE.match(line) or INCLUDE.search(line):
                        found = True
                        break
            if found:
                break
        if found:
            break
    if found:
        print(p["name"])
if libs == 0:
    sys.exit("doc-crates: cargo metadata lists no library: nothing was scanned")
