#!/usr/bin/env python3
"""What must build on Windows and macOS (PR G9). Called by scripts/check-portable.sh.

  classify <list> <metadata.json>                        check the data file against the graph
  plan     <list> <metadata.json> <triple>               print the crates to cargo-check (stdout)
  verdict  <list> <metadata.json> <triple> <log> <rc>    read cargo's log, pass or fail

An Orion crate is Linux-only at the locked rev when its lib.rs has a crate-level
#![cfg(target_os = "linux")]. An Atlas crate is Orion-bound when a dependency path (normal,
build or dev) from it reaches such a crate. The Orion edges in the locked graph are all
unconditional, so the answer does not depend on the target triple.

Every other workspace crate is portable and must build for each target. Two excused sets, each
entry with a written reason, are still compiled and printed on every run, and never fail the check:
  [does-not-build <triple>]  CI found it does not compile there; the comment is the first error.
  [not-yet <triple>]         waiting on another PR; the comment names it.
An excused crate that starts to build fails the check until its entry is removed.
"""

import json
import re
import sys

ORION_SOURCE = "git+https://github.com/Prometheus-Dynamics/Orion"
LINUX_GATE = re.compile(r'^#!\[cfg\(target_os = "linux"\)\]', re.M)
COULD_NOT_COMPILE = re.compile(r"could not compile `([^`]+)`(?: \(([^)]*)\))?")
ERROR_START = re.compile(r"^error(\[[A-Z0-9]+\])?:")
ANSI = re.compile(r"\x1b\[[0-9;]*m")


def die(message):
    print(f"check-portable: {message}", file=sys.stderr)
    sys.exit(1)


def load_list(path):
    """Return (orion_bound, does_not_build, not_yet).

    orion_bound: {crate: comment}; the other two: {triple: {crate: comment}}.
    """
    bound, not_build, not_yet = {}, {}, {}
    section = None
    with open(path, encoding="utf-8") as handle:
        for lineno, raw in enumerate(handle, 1):
            body, _, comment = raw.partition("#")
            body = body.strip()
            if not body:
                continue
            where = f"{path}:{lineno}"
            if body.startswith("[") and body.endswith("]"):
                head = body[1:-1].split()
                if head == ["orion-bound"]:
                    section = ("bound", None)
                elif len(head) == 2 and head[0] == "does-not-build":
                    section = ("not-build", head[1])
                    not_build.setdefault(head[1], {})
                elif len(head) == 2 and head[0] == "not-yet":
                    section = ("not-yet", head[1])
                    not_yet.setdefault(head[1], {})
                else:
                    die(f"{where}: unknown section {body}")
                continue
            if section is None:
                die(f"{where}: an entry before any section")
            if len(body.split()) != 1:
                die(f"{where}: one crate name per line, found {body!r}")
            comment = comment.strip()
            if not comment:
                die(f"{where}: {body} needs a comment (the Orion crate that binds it, or the reason)")
            kind, triple = section
            table = bound if kind == "bound" else (not_build if kind == "not-build" else not_yet)[triple]
            if body in table:
                die(f"{where}: {body} is listed twice")
            table[body] = comment
    return bound, not_build, not_yet


def lib_is_linux_only(package):
    for target in package["targets"]:
        if "lib" in target["kind"]:
            with open(target["src_path"], encoding="utf-8") as handle:
                return bool(LINUX_GATE.search(handle.read()))
    return False


class Graph:
    def __init__(self, meta):
        packages = {p["id"]: p for p in meta["packages"]}
        self.name = {i: p["name"] for i, p in packages.items()}
        resolve = {n["id"]: n for n in meta["resolve"]["nodes"]}
        # Each edge: (dependency id, dev_only).
        self.edges = {
            i: [(d["pkg"], all(k["kind"] == "dev" for k in d["dep_kinds"])) for d in resolve[i]["deps"]]
            for i in resolve
        }
        self.deps = {i: [dep for dep, _ in edges] for i, edges in self.edges.items()}
        self.members = {self.name[i]: i for i in meta["workspace_members"]}
        orion = [i for i, p in packages.items() if (p.get("source") or "").startswith(ORION_SOURCE)]
        self.orion_names = sorted(self.name[i] for i in orion)
        self.linux_only = {i for i in orion if lib_is_linux_only(packages[i])}
        self.linux_names = sorted(self.name[i] for i in self.linux_only)
        self._memo = {}

    def reaches_linux_only(self, pid, stack=()):
        """The Linux-only Orion package that a dependency path from pid reaches, or None."""
        if pid in self.linux_only:
            return pid
        if pid in self._memo:
            return self._memo[pid]
        if pid in stack:
            return None
        found = None
        for dep in self.deps.get(pid, []):
            found = self.reaches_linux_only(dep, stack + (pid,))
            if found:
                break
        self._memo[pid] = found
        return found

    def workspace_deps(self, name):
        return sorted({self.name[d] for d in self.deps[self.members[name]] if d in self.members.values()})

    def binding(self, name):
        """The Linux-only Orion crate a crate binds on, by its best edge: normal before dev, direct before indirect."""
        best = None
        for dep, dev_only in self.edges[self.members[name]]:
            hit = self.reaches_linux_only(dep)
            if hit:
                key = (dev_only, dep not in self.linux_only, self.name[dep])
                if best is None or key < best[0]:
                    best = (key, dep, hit)
        if best is None:
            return "binds on a Linux-only Orion crate"
        _, dep, hit = best
        via = "" if dep == hit else f" (through {self.name[dep]})"
        dev = " (dev only)" if best[0][0] else ""
        return f"binds on {self.name[hit]}{via}{dev}"

    def analyse(self):
        """Return (bound: {crate: binding}, portable: [crate]) over the workspace crates."""
        bound = {n: self.binding(n) for n, pid in self.members.items() if self.reaches_linux_only(pid)}
        portable = sorted(n for n in self.members if n not in bound)
        return bound, portable


def classify(graph, bound_list, not_build, not_yet):
    """Check the data file against the graph. Return (report lines, errors, bound, portable)."""
    bound, portable = graph.analyse()
    excused = {n for table in list(not_build.values()) + list(not_yet.values()) for n in table}
    errors, lines = [], []
    for name in sorted(set(bound_list) | excused):
        if name not in graph.members:
            errors.append(f"{name} is in scripts/orion-bound.txt but is not a workspace crate")
    for name in sorted(set(bound_list) & excused):
        errors.append(f"{name} is Orion-bound and also excused; remove one entry")
    for name in sorted(graph.members):
        if name in bound:
            lines.append(f"orion-bound  {name:22} {bound[name]}")
            if name not in bound_list:
                direct = [d for d in graph.workspace_deps(name) if d in bound_list]
                if direct:
                    errors.append(
                        f"(c) {name} gained a dependency on Orion-bound {direct[0]}; "
                        f"list {name} under [orion-bound] or drop the dependency"
                    )
                else:
                    errors.append(
                        f"(c) {name} {bound[name]} but is not in [orion-bound]; "
                        f"list it there with its binding or drop the Orion dependency"
                    )
        else:
            lines.append(f"portable     {name}")
            if name in bound_list:
                errors.append(
                    f"(b) {name} is listed in [orion-bound] but no longer depends on a Linux-only "
                    f"Orion crate; remove it from scripts/orion-bound.txt"
                )
    return lines, errors, bound, portable


def load_graph(meta_path):
    with open(meta_path, encoding="utf-8") as handle:
        return Graph(json.load(handle))


def checked(list_path, meta_path):
    bound_list, not_build, not_yet = load_list(list_path)
    graph = load_graph(meta_path)
    lines, errors, bound, portable = classify(graph, bound_list, not_build, not_yet)
    return bound_list, not_build, not_yet, graph, lines, errors, bound, portable


def fail_with(errors, list_path):
    for error in errors:
        print(f"FAIL {error}", file=sys.stderr)
    die(f"{len(errors)} problem(s) with {list_path}")


def cmd_classify(list_path, meta_path):
    _, _, _, graph, lines, errors, bound, portable = checked(list_path, meta_path)
    print("Orion crates Linux-only at the locked rev: " + " ".join(graph.linux_names))
    for line in lines:
        print("  " + line)
    print(f"  {len(bound)} Orion-bound, {len(portable)} portable")
    if errors:
        fail_with(errors, list_path)
    print("check-portable: the list matches the graph")


def cmd_plan(list_path, meta_path, triple):
    bound_list, not_build, not_yet, _, _, errors, bound, portable = checked(list_path, meta_path)
    if errors:
        fail_with(errors, list_path)
    for name in sorted(bound):
        print(f"skip  {name:22} Orion-bound: {bound[name]}", file=sys.stderr)
    for name, reason in sorted(not_build.get(triple, {}).items()):
        print(f"excused {name:20} does not build on {triple} (listed): {reason}", file=sys.stderr)
    for name, reason in sorted(not_yet.get(triple, {}).items()):
        print(f"excused {name:20} not yet on {triple}: {reason}", file=sys.stderr)
    # Excused crates are still compiled: a failure is printed, a success fails the check.
    for name in portable:
        print(name)


def cmd_verdict(list_path, meta_path, triple, log_path, rc):
    _, not_build, not_yet, graph, _, errors, _, portable = checked(list_path, meta_path)
    if errors:
        fail_with(errors, list_path)
    excused = {}
    for name, reason in not_build.get(triple, {}).items():
        excused[name] = ("does not build (listed)", reason)
    for name, reason in not_yet.get(triple, {}).items():
        excused[name] = ("not yet", reason)
    # cargo --keep-going names each unit it could not compile: "error: could not compile `x` (lib)".
    # The first error block before that line is the unit's first error.
    first_error, block_first, lib_failing = {}, None, set()
    with open(log_path, encoding="utf-8", errors="replace") as handle:
        for line in handle:
            text = ANSI.sub("", line).rstrip("\r\n")
            match = COULD_NOT_COMPILE.search(text)
            if match:
                name, unit = match.group(1), match.group(2) or ""
                first_error.setdefault(name, block_first or text.strip())
                if unit == "lib":
                    lib_failing.add(name)
                block_first = None
            elif block_first is None and ERROR_START.match(text):
                block_first = text.strip()
    failing = set(first_error)
    if int(rc) != 0 and not failing:
        die(f"cargo check failed for {triple} without a compile error; see the log above")
    # Only a library that does not build blocks the crates that depend on it. A failing test
    # or bin target blocks nothing else.
    blocked = {}
    changed = True
    while changed:
        changed = False
        for name in portable:
            if name in lib_failing or name in blocked:
                continue
            for dep in graph.workspace_deps(name):
                if dep in lib_failing or dep in blocked:
                    blocked[name] = dep
                    changed = True
                    break
    failed = built = excused_failing = 0
    for name in portable:
        broken = name in failing or name in blocked
        why = first_error.get(name) or f"blocked by {blocked.get(name)}"
        if name in excused:
            label, reason = excused[name]
            if broken:
                excused_failing += 1
                print(f"{label} {name}: {why}")
                print(f"     recorded: {reason}")
            else:
                failed += 1
                print(f"FAIL {name} builds on {triple} now but is {label} in scripts/orion-bound.txt; "
                      f"remove the entry", file=sys.stderr)
        elif broken:
            failed += 1
            print(f"FAIL {name} does not build on {triple}: {why}", file=sys.stderr)
        else:
            built += 1
            print(f"ok   {name} builds on {triple}")
    print(f"check-portable: {built} built, {excused_failing} excused and still failing, "
          f"{failed} failing on {triple}")
    if failed:
        die(f"{failed} portable crate(s) fail on {triple}")
    print("check-portable: every portable crate builds, or is excused with a reason")


def main(argv):
    if len(argv) < 2:
        die("usage: check-portable.py classify|plan|verdict ...")
    command, args = argv[1], argv[2:]
    if command == "classify" and len(args) == 2:
        cmd_classify(*args)
    elif command == "plan" and len(args) == 3:
        cmd_plan(*args)
    elif command == "verdict" and len(args) == 5:
        cmd_verdict(*args)
    else:
        die(f"bad arguments to {command}")


if __name__ == "__main__":
    main(sys.argv)
