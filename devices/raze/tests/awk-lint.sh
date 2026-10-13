#!/bin/sh
# Off-device test: the package's awk programs use nothing BusyBox awk lacks
# without FEATURE_AWK_LIBM (the image's build): no ^ or ** and no math
# functions. GNU awk on a test host runs them fine, so the tests that run
# the scripts don't catch it; r24's USB-link check died on `2 ^ n`.
# Run: sh devices/raze/tests/awk-lint.sh
set -eu
here=$(cd "$(dirname "$0")" && pwd)
python3 - "$here/../gaia/assets" <<'PY'
import os, re, sys

root = sys.argv[1]
bad = re.compile(r"\^|\*\*|\b(sqrt|exp|log|sin|cos|atan2|rand|srand)\s*\(")
# An awk call and its program: the first single-quoted argument after it.
call = re.compile(r"\bawk\b[^'\n]*'((?:[^']|'\\'')*)'", re.S)
problems = []
seen = 0
for dirpath, _, files in os.walk(root):
    for name in files:
        path = os.path.join(dirpath, name)
        try:
            text = open(path, encoding="utf-8").read()
        except (UnicodeDecodeError, OSError):
            continue
        if "awk" not in text:
            continue
        for m in call.finditer(text):
            program = m.group(1)
            seen += 1
            # Regex literals (/^.../) and strings may hold ^ legitimately.
            stripped = re.sub(r'"(?:[^"\\]|\\.)*"', '""', program)
            stripped = re.sub(r"#[^\n]*", "", stripped)
            stripped = re.sub(r"(^|[\s(!~,;{])/(?:[^/\\\n]|\\.)+/", r"\1//", stripped)
            for hit in bad.finditer(stripped):
                line = text[: m.start(1) + hit.start()].count("\n") + 1
                problems.append("%s:%d: %s" % (os.path.relpath(path, root), line, hit.group(0)))
for p in problems:
    print("FAIL: BusyBox awk has no math: " + p)
if problems:
    sys.exit(1)
print("%d awk programs checked" % seen)
PY
echo "awk-lint ok"
