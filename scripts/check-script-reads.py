#!/usr/bin/env python3
"""Fails when code names a script that the code lanes' key does not hold.

    check-script-reads.py [--root DIR] KEPT_SCRIPT...
    check-script-reads.py --self-test

A code lane's result is kept while its key is unchanged, and that key holds only the scripts the
code lanes run (scripts/gate-lib.sh, GATE_CODE_LANE_SCRIPTS). A build, a test or the app that read
any other script (include_str!, include_bytes!, a path in a string) would keep a pass after that
script changed. This reads every source under crates/ and apps/ (Rust, TypeScript, JavaScript,
Svelte, TOML), leaves out comments, and names each string literal that holds a path under
scripts/ which is not one of KEPT_SCRIPT. Comments may name any script.
"""

import os
import re
import subprocess
import sys
import tempfile

EXTENSIONS = (".rs", ".ts", ".js", ".svelte", ".toml")
SCRIPT = re.compile(r"(?<![A-Za-z0-9_.-])scripts/([A-Za-z0-9_.-]+)")


def literals(text, toml):
    """The string literals of TEXT with their line numbers, outside comments."""
    out, i, line, n = [], 0, 1, len(text)
    while i < n:
        c = text[i]
        if c == "\n":
            line += 1
            i += 1
        elif toml and c == "#":
            i = text.find("\n", i)
            i = n if i < 0 else i
        elif not toml and text.startswith("//", i):
            i = text.find("\n", i)
            i = n if i < 0 else i
        elif not toml and text.startswith("/*", i):
            end = text.find("*/", i + 2)
            end = n if end < 0 else end + 2
            line += text.count("\n", i, end)
            i = end
        elif c in "\"'`":
            # A Rust char literal or lifetime ('a) holds no path; a short scan finds its end.
            start, i, buf = line, i + 1, []
            while i < n and text[i] != c:
                if text[i] == "\\" and i + 1 < n:
                    buf.append(text[i + 1])
                    i += 2
                    continue
                if text[i] == "\n":
                    if c != "`" and c != '"':
                        break
                    line += 1
                buf.append(text[i])
                i += 1
            i += 1
            out.append((start, "".join(buf)))
        else:
            i += 1
    return out


def check(root, kept):
    files = subprocess.run(
        ["git", "-C", root, "ls-files", "-co", "--exclude-standard", "--", "crates", "apps"],
        capture_output=True, text=True, check=True,
    ).stdout.split("\n")
    bad = []
    for name in files:
        if not name.endswith(EXTENSIONS):
            continue
        try:
            text = open(os.path.join(root, name), encoding="utf-8", errors="replace").read()
        except OSError:
            continue
        for line, literal in literals(text, name.endswith(".toml")):
            for found in SCRIPT.finditer(literal):
                script = "scripts/" + found.group(1)
                if script not in kept:
                    bad.append(f"{name}:{line}: names {script}, which the code lanes' key does not hold")
    return bad


def self_test():
    kept = {"scripts/gate.sh"}
    cases = [
        ("comment only", "src/a.rs", "// reads scripts/area.sh\n/* and scripts/deb-fast.sh */\nfn f() {}\n", False),
        ("a kept script", "src/a.rs", 'const G: &str = include_str!("../../scripts/gate.sh");\n', False),
        ("include_str of another", "src/a.rs", 'const A: &str = include_str!("../../../scripts/area.sh");\n', True),
        ("a path in a string", "tests/t.rs", 'fn t() { std::fs::read("scripts/check-devices.sh").unwrap(); }\n', True),
        ("a TypeScript string", "e2e/x.ts", "const p = `${root}/scripts/bundle.sh`;\n", True),
        ("a TOML value", "Cargo.toml", 'build = "../scripts/x.sh" # scripts/y.sh\n', True),
        ("a TOML comment", "Cargo.toml", "# scripts/y.sh\n", False),
    ]
    failed = 0
    for label, rel, body, should_fail in cases:
        with tempfile.TemporaryDirectory() as root:
            subprocess.run(["git", "init", "-q", root], check=True)
            path = os.path.join(root, "crates", "c", rel)
            os.makedirs(os.path.dirname(path))
            with open(path, "w") as f:
                f.write(body)
            bad = check(root, kept)
        if bool(bad) != should_fail:
            print(f"check-script-reads: self-test case '{label}' gave {bad or 'nothing'}", file=sys.stderr)
            failed += 1
    if failed:
        return 1
    print(f"check-script-reads: self-test passed ({len(cases)} cases: comments pass, a kept script passes, "
          "include_str, a Rust, TypeScript or TOML string naming another script fails)")
    return 0


def main(argv):
    if argv[:1] == ["--self-test"]:
        return self_test()
    root = "."
    if argv[:1] == ["--root"]:
        root, argv = argv[1], argv[2:]
    if not argv:
        print("check-script-reads: name the scripts the code lanes' key holds", file=sys.stderr)
        return 2
    bad = check(root, set(argv))
    for b in bad:
        print(b, file=sys.stderr)
    if bad:
        print("check-script-reads: add the script to GATE_CODE_LANE_SCRIPTS (scripts/gate-lib.sh) or stop reading it",
              file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
