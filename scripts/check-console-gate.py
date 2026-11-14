#!/usr/bin/env python3
"""The Raze's console follows the board's security switch (LOCK: Lemnos S2).

Runs the component's shipped script, devices/components/raze-base-files/files/console-allowed, against a view file
(LEMNOS_SECURITY_VIEW). The script is the ExecCondition= of the getty drop-ins: exit 0 starts the getty, exit 1 skips it.
Cases (the four the switch has, and the malformed forms, which count as ON):
  absent                      -> 0 (off: the console is open; nothing renders the view at our lock)
  off, generation N           -> 0
  on, generation N            -> 1 (no login)
  malformed (any other)       -> 1 (corrupt is ON): empty file, one line, three lines, no generation line, a bad
                                 generation number, an unknown state word, no newline at the end
Checks that the drop-ins install the script in the three template units (serial-getty, getty, autovt).
Run from the repository root. `--self-test` is the same run; the gate runs it.
"""

import os
import subprocess
import sys
import tempfile

COMPONENT = os.path.join("devices", "components", "raze-base-files")
SCRIPT = os.path.join(COMPONENT, "files", "console-allowed")
DROP_IN = os.path.join(COMPONENT, "files", "30-security-console.conf")
UNITS = ["serial-getty@.service.d", "getty@.service.d", "autovt@.service.d"]

CASES = [
    ("the view is absent", None, 0),
    ("off, generation 3", "off\ngeneration 3\n", 0),
    ("on, generation 3", "on\ngeneration 3\n", 1),
    ("empty file", "", 1),
    ("one line only", "on\n", 1),
    ("three lines", "off\ngeneration 3\nextra\n", 1),
    ("no generation line", "off\nfoo 3\n", 1),
    ("a bad generation number", "off\ngeneration x\n", 1),
    ("an unknown state word", "maybe\ngeneration 3\n", 1),
    ("no newline at the end", "off\ngeneration 3", 1),
]


def run(view_text, directory):
    env = dict(os.environ)
    path = os.path.join(directory, "security")
    if view_text is None:
        path = os.path.join(directory, "no-such-view")
    else:
        with open(path, "w", encoding="utf-8") as handle:
            handle.write(view_text)
    env["LEMNOS_SECURITY_VIEW"] = path
    return subprocess.run(["sh", SCRIPT], env=env, capture_output=True, text=True).returncode


def check():
    problems = []
    for label, view, expected in CASES:
        with tempfile.TemporaryDirectory() as directory:
            code = run(view, directory)
        if code != expected:
            problems.append(f"{label}: exit {code}, the rule needs {expected}")
    with open(DROP_IN, encoding="utf-8") as handle:
        text = handle.read()
    if "ExecCondition=/usr/lib/lemnos/console-allowed" not in text:
        problems.append(f"{DROP_IN}: no ExecCondition=/usr/lib/lemnos/console-allowed")
    with open(os.path.join(COMPONENT, "component.toml"), encoding="utf-8") as handle:
        component = handle.read()
    for unit in UNITS:
        target = f"/etc/systemd/system/{unit}/30-security-console.conf"
        if target not in component:
            problems.append(f"{COMPONENT}/component.toml does not install the drop-in to {target}")
    if "/usr/lib/lemnos/console-allowed" not in component:
        problems.append(f"{COMPONENT}/component.toml does not install the condition script")
    if not os.path.isfile(SCRIPT) or not os.access(SCRIPT, os.X_OK):
        problems.append(f"{SCRIPT} is missing or not executable")
    return problems


def main():
    problems = check()
    for problem in problems:
        print(f"check-console-gate: FAIL {problem}", file=sys.stderr)
    if problems:
        return 1
    print(f"check-console-gate: passed ({len(CASES)} view cases: absent and off open the console, on and malformed refuse it)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
