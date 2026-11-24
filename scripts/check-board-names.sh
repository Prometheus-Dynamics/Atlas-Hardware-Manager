#!/usr/bin/env bash
# Every board name and marker at the pinned revs, checked: the Lemnos and Orion markers, the board citations, and the
# board-facing constants. Stdlib python3 only (run with -I). It fails when a rule below is broken; it prints each one.
#
# Usage:
#   scripts/check-board-names.sh              the tracked tree
#   scripts/check-board-names.sh --self-test  the checker on fixtures: each failure is caught, and a clean one passes
#
# The pinned revs are read from the workspace Cargo.toml (the `rev =` of every git dependency on Lemnos.git or
# Orion.git) and from devices/components/*/component.toml (their `ref =`; a directory named orion* is Orion, one named
# lemnos* or flasher-lemnos* is Lemnos). A rev matches a pinned one by prefix, at least seven hex digits.
#
# Rules (every tracked text file is read, except docs/prs/, which is PR history, and this script, whose rule text and
# fixtures are made to fail and are checked by --self-test; both are excluded from every rule):
#   (a) A LOCK marker names the pin. A LOCK marker is `LOCK` followed by `:` or a space on a line that names Lemnos or
#       Orion, and its block: that line and the comment lines after it. The block must name at least one rev, as
#       `Lemnos <hex>`, `Orion <hex>` or `rev <hex>`, and every rev it names must be pinned for that repo (or, with
#       no repo word, pinned for either).
#   (b) A `board:` citation names a pinned rev: `// board: Lemnos 214c0ca7 path:line`. A citation with no rev, or a
#       rev that is not pinned for that repo, fails.
#   (c) A board-facing string constant is cited or imported. Board-facing: a Rust `const` or `static` of type &str
#       whose literal value starts with `lemnos.`, `lemnos/`, `system.`, `/data/`, `/etc/lemnos`, `/run/orion` or
#       `/etc/board`, or is a contract tag (`x.y/N`). An imported value is not a literal and so is not checked. A literal
#       needs a `board:` citation on one of the 3 lines above it. Test code is outside this rule: a file under tests/,
#       a *_tests.rs file, and a `#[cfg(test)]` module.
#   (d) ATLAS_BENCH_NO_RENEW appears only in crates/tools/next/src/hold.rs, in this script, and in docs (docs/ and
#       *.md files).
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
exec python3 -I - "$@" <<'PY'
import re
import subprocess
import sys

HEX = r"[0-9a-f]{7,40}"
ATTRIBUTED = re.compile(r"\b(Lemnos|Orion)\s+(?:rev\s+)?(" + HEX + r")\b")
REV_WORD = re.compile(r"\brev\s+(" + HEX + r")\b")
CITATION = re.compile(r"\bboard:\s*(Lemnos|Orion)?\s*(" + HEX + r")?")
LOCK = re.compile(r"\bLOCK(?=[: ])")
CONST = re.compile(
    r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:const|static)\s+[A-Z0-9_]+\s*:\s*&(?:'static\s+)?str\s*=\s*\"((?:[^\"\\]|\\.)*)\"\s*;"
)
BOARD_PREFIXES = ("lemnos.", "lemnos/", "system.", "/data/", "/etc/lemnos", "/run/orion", "/etc/board")
CONTRACT_TAG = re.compile(r"^[a-z][a-z0-9-]*(\.[a-z0-9-]+)+/\d+$")
COMMENT_START = ("//", "#", "*", "--")
NO_RENEW = "ATLAS_BENCH_NO_RENEW"
NO_RENEW_HOME = "crates/tools/next/src/hold.rs"


def pinned_revs(root_text, components):
    """(lemnos, orion) pinned rev sets, from the workspace Cargo.toml text and the component files."""
    lemnos, orion = set(), set()
    for line in root_text.splitlines():
        m = re.search(r'git = "https://github\.com/Prometheus-Dynamics/(Lemnos|Orion)\.git".*?rev = "([0-9a-f]{40})"', line)
        if m:
            (lemnos if m.group(1) == "Lemnos" else orion).add(m.group(2))
    for name, text in components.items():
        base = name.split("/")[-2] if name.count("/") >= 2 else ""
        for line in text.splitlines():
            m = re.match(r'\s*(?:ref|rev)\s*=\s*"([0-9a-f]{40})"', line)
            if not m:
                continue
            if "orion" in base:
                orion.add(m.group(1))
            elif base.startswith("lemnos") or base.startswith("flasher-lemnos"):
                lemnos.add(m.group(1))
    return lemnos, orion


def matches(rev, pins):
    return any(p.startswith(rev) or rev.startswith(p) for p in pins)


def is_comment(line):
    return line.lstrip().startswith(COMMENT_START)


def check(files, root_text, components):
    """files: {path: text}. Returns (violations, pins) where a violation is (path, line, rule, words)."""
    lemnos, orion = pinned_revs(root_text, components)
    both = lemnos | orion
    out = []
    for path in sorted(files):
        if path.startswith("docs/prs/"):
            continue
        text = files[path]
        lines = text.splitlines()
        out.extend(rule_a(path, lines, lemnos, orion, both))
        out.extend(rule_b(path, lines, lemnos, orion))
        if path.endswith(".rs") and not path.startswith("scripts/"):
            out.extend(rule_c(path, lines))
        if NO_RENEW in text and path != NO_RENEW_HOME and not path.startswith("docs/") and not path.endswith(".md") and not path.startswith("scripts/check-board-names"):
            out.append((path, first_line(lines, NO_RENEW), "d", "ATLAS_BENCH_NO_RENEW outside its home"))
    return out, (lemnos, orion)


def first_line(lines, word):
    for number, line in enumerate(lines, 1):
        if word in line:
            return number
    return 0


def rule_a(path, lines, lemnos, orion, both):
    out = []
    for number, line in enumerate(lines, 1):
        if not LOCK.search(line) or not re.search(r"\b(Lemnos|Orion)\b", line):
            continue
        block = [line]
        for follow in lines[number:]:
            if not follow.strip() or not is_comment(follow):
                break
            block.append(follow)
        text = " ".join(block)
        named = [(m.group(1), m.group(2)) for m in ATTRIBUTED.finditer(text)]
        named += [(None, m.group(1)) for m in REV_WORD.finditer(text)]
        if not named:
            out.append((path, number, "a", "a LOCK marker names no rev"))
            continue
        for repo, rev in named:
            pins = lemnos if repo == "Lemnos" else orion if repo == "Orion" else both
            if not matches(rev, pins):
                out.append((path, number, "a", f"LOCK names rev {rev}, not pinned for {repo or 'either repo'}"))
    return out


def rule_b(path, lines, lemnos, orion):
    # A citation is a comment line (code comments, shell and config comments). Prose in Markdown is not one.
    if path.endswith(".md"):
        return []
    out = []
    for number, line in enumerate(lines, 1):
        if not is_comment(line) or not re.search(r"\bboard:\s*(Lemnos|Orion)\b", line):
            continue
        cite = CITATION.search(line)
        repo, rev = cite.group(1), cite.group(2)
        if not rev:
            out.append((path, number, "b", "a board: citation names no rev"))
            continue
        pins = lemnos if repo == "Lemnos" else orion if repo == "Orion" else (lemnos | orion)
        if not matches(rev, pins):
            out.append((path, number, "b", f"board: citation names rev {rev}, not pinned for {repo}"))
    return out


def rule_c(path, lines):
    if "/tests/" in "/" + path or path.endswith("_tests.rs"):
        return []
    out = []
    skip_until_close = False
    for number, line in enumerate(lines, 1):
        if line.strip() == "#[cfg(test)]":
            skip_until_close = True
            continue
        if skip_until_close:
            if line.startswith("}"):
                skip_until_close = False
            continue
        m = CONST.match(line)
        if not m:
            continue
        value = m.group(1)
        if not (value.startswith(BOARD_PREFIXES) or CONTRACT_TAG.match(value)):
            continue
        above = lines[max(0, number - 4):number - 1]
        if not any("board:" in above_line for above_line in above):
            out.append((path, number, "c", f"board-facing constant {value!r} has no board: citation within 3 lines"))
    return out


def run_tree():
    root = subprocess.run(["git", "rev-parse", "--show-toplevel"], capture_output=True, text=True, check=True).stdout.strip()
    tracked = subprocess.run(["git", "-C", root, "ls-files"], capture_output=True, text=True, check=True).stdout.split()
    files = {}
    for path in tracked:
        # This script's own rule text and self-test fixtures are made to fail the rules: it is checked by --self-test.
        if path.startswith("docs/prs/") or path.startswith("target") or path == "scripts/check-board-names.sh":
            continue
        try:
            with open(f"{root}/{path}", encoding="utf-8") as handle:
                files[path] = handle.read()
        except (OSError, UnicodeDecodeError):
            continue
    root_text = files.get("Cargo.toml", "")
    components = {p: t for p, t in files.items() if re.match(r"devices/components/[^/]+/component\.toml$", p)}
    return files, root_text, components


def report(violations):
    for path, number, rule, words in violations:
        print(f"check-board-names: ({rule}) {path}:{number}: {words}")
    print(f"check-board-names: {len(violations)} flagged")
    return 1 if violations else 0


def self_test():
    root_text = 'orion = { git = "https://github.com/Prometheus-Dynamics/Orion.git", rev = "' + "2" * 40 + '" }\n' \
        'lemnos-core = { git = "https://github.com/Prometheus-Dynamics/Lemnos.git", rev = "' + "b" * 40 + '" }\n'
    components = {"devices/components/flasher-lemnosd/component.toml": 'ref = "' + "d" * 40 + '"\n'}
    good = "b" * 8
    orion_good = "2" * 8
    fixtures = {
        "pass.rs": (
            "// LOCK: Lemnos " + good + " service/orion/src/config.rs:81 (the lease)\n"
            "// board: Lemnos " + good + " service/orion/src/hello.rs:19\n"
            "pub const MAX: usize = 1;\n"
            "// board: Lemnos " + good + " service/orion/src/hello.rs:23\n"
            "pub const OFFERS: &str = \"offers\";\n"
            "// board: Orion " + orion_good + " crates/comm/discovery/src/mdns.rs:1\n"
            "// LOCK: Orion " + orion_good + " discovery\n"
            "pub const TAG: &str = \"system.slots/1\";\n"
        ),
        "lock_no_rev.rs": "// LOCK: Lemnos I6g lease period\npub const X: u32 = 1;\n",
        "lock_wrong_rev.rs": "// LOCK: Lemnos " + "c" * 8 + " the old pin\npub const X: u32 = 1;\n",
        "cite_wrong_rev.rs": "// board: Lemnos " + "c" * 8 + " service/x.rs:1\npub const X: u32 = 1;\n",
        "cite_no_rev.rs": "// board: Lemnos\npub const X: u32 = 1;\n",
        "const_uncited.rs": "pub const LIST: &str = \"system.slots.list\";\n",
        "const_far_citation.rs": "// board: Lemnos " + good + " x.rs:1\n\n\n\npub const LIST: &str = \"lemnos.devices\";\n",
        "no_renew.rs": "let x = \"ATLAS_BENCH_NO_RENEW\";\n",
        "docs/note.md": "ATLAS_BENCH_NO_RENEW is in the docs\n",
        "crates/tools/next/src/hold.rs": "const N: &str = \"ATLAS_BENCH_NO_RENEW\";\n",
        "docs/prs/old.md": "// LOCK: Lemnos I6g old text\n",
    }
    # Each failure fixture alone must be flagged with its rule, and the clean one must pass.
    expect = {
        "pass.rs": None,
        "lock_no_rev.rs": "a",
        "lock_wrong_rev.rs": "a",
        "cite_wrong_rev.rs": "b",
        "cite_no_rev.rs": "b",
        "const_uncited.rs": "c",
        "const_far_citation.rs": "c",
        "no_renew.rs": "d",
    }
    failures = []
    for name, rule in expect.items():
        violations, _ = check({name: fixtures[name]}, root_text, components)
        rules = {v[2] for v in violations}
        if rule is None:
            if violations:
                failures.append(f"{name} should pass, flagged {violations}")
        elif rule not in rules:
            failures.append(f"{name} should fail rule {rule}, got {violations}")
    # The home file, the docs and the PR history are exempt from rule d.
    violations, _ = check({k: fixtures[k] for k in ("docs/note.md", "crates/tools/next/src/hold.rs", "docs/prs/old.md")},
                          root_text, components)
    if violations:
        failures.append(f"exempt files were flagged: {violations}")
    if failures:
        for failure in failures:
            print("check-board-names --self-test: FAIL " + failure, file=sys.stderr)
        return 1
    print(f"check-board-names --self-test: ok ({len(expect)} fixtures)")
    return 0


if "--self-test" in sys.argv[1:]:
    sys.exit(self_test())
files, root_text, components = run_tree()
violations, _ = check(files, root_text, components)
sys.exit(report(violations))
PY
