#!/bin/sh
# Off-device test: the package's systemd units, with the standard boot
# targets and the units they order against, have no ordering cycle. systemd
# breaks a cycle at boot by dropping a job from it, and which one it drops
# varies: on r22 it dropped dbus.socket (board-pstore after basic.target but
# before systemd-pstore, which is before sysinit.target), so NetworkManager
# never started.
# Run: sh devices/raze/tests/unit-order.sh
set -eu
here=$(cd "$(dirname "$0")" && pwd)
python3 - "$here/../gaia/assets/rootfs/usr/lib/systemd/system" <<'PY'
import os, re, sys

root = sys.argv[1]
after, before = {}, {}  # unit -> set of units it is ordered after / before

def add(kind, unit, others):
    (after if kind == "After" else before).setdefault(unit, set()).update(others)

# Standard units, as systemd (and the OS's dbus, journald, pstore) define them.
std = {
    "local-fs.target": ("After", ["local-fs-pre.target"]),
    "sysinit.target": ("After", ["local-fs.target", "swap.target"]),
    "sockets.target": ("After", ["sysinit.target"]),
    "basic.target": ("After", ["sysinit.target", "sockets.target", "paths.target", "timers.target", "slices.target"]),
    "paths.target": ("After", ["sysinit.target"]),
    "timers.target": ("After", ["sysinit.target"]),
    "multi-user.target": ("After", ["basic.target"]),
    "network-pre.target": ("After", []),
    "network.target": ("After", ["network-pre.target"]),
    "network-online.target": ("After", ["network.target"]),
}
for unit, (kind, others) in std.items():
    add(kind, unit, others)
# DefaultDependencies=no early units of the OS.
add("Before", "systemd-pstore.service", ["sysinit.target"])
add("After", "systemd-pstore.service", ["local-fs.target"])
add("Before", "systemd-journald.service", ["sysinit.target"])
add("Before", "systemd-journal-flush.service", ["sysinit.target"])
add("After", "systemd-journal-flush.service", ["systemd-journald.service", "local-fs.target"])
add("Before", "systemd-udevd.service", ["sysinit.target"])
add("Before", "systemd-udev-trigger.service", ["sysinit.target"])
add("Before", "systemd-modules-load.service", ["sysinit.target"])
add("Before", "sys-kernel-config.mount", ["sysinit.target"])
add("After", "dbus.socket", ["sysinit.target"])
add("Before", "dbus.socket", ["sockets.target"])
add("Before", "systemd-resolved.service", ["network.target"])
add("After", "systemd-resolved.service", ["sysinit.target", "basic.target"])
add("After", "time-set.target", [])

for dirpath, _, files in os.walk(root):
    for name in files:
        if not re.search(r"\.(service|socket|path|timer|target|mount)$", name) or name.endswith(".conf"):
            continue
        unit = name.replace("@.", "@x.")
        text = open(os.path.join(dirpath, name)).read()
        section, default = None, True
        for line in text.splitlines():
            line = line.strip()
            if line.startswith("["):
                section = line
                continue
            if section != "[Unit]" or "=" not in line:
                continue
            key, value = line.split("=", 1)
            if key in ("After", "Before"):
                add(key, unit, value.split())
            if key == "DefaultDependencies" and value.strip() in ("no", "false", "0"):
                default = False
        if default:
            if unit.endswith(".socket"):
                add("After", unit, ["sysinit.target"])
                add("Before", unit, ["sockets.target"])
            elif unit.endswith(".path"):
                add("After", unit, ["sysinit.target"])
                add("Before", unit, ["paths.target"])
            elif unit.endswith(".timer"):
                add("After", unit, ["sysinit.target"])
                add("Before", unit, ["timers.target"])
            elif not unit.endswith(".target"):
                add("After", unit, ["sysinit.target", "basic.target"])

# Edges a -> b mean "a starts before b".
edges = {}
for unit, others in after.items():
    for o in others:
        edges.setdefault(o, set()).add(unit)
for unit, others in before.items():
    edges.setdefault(unit, set()).update(others)

state, stack, cycles = {}, [], []
def visit(u):
    state[u] = 1
    stack.append(u)
    for v in sorted(edges.get(u, ())):
        if state.get(v) == 1:
            cycles.append(stack[stack.index(v):] + [v])
        elif v not in state:
            visit(v)
    stack.pop()
    state[u] = 2
for u in sorted(edges):
    if u not in state:
        visit(u)
if cycles:
    for c in cycles:
        print("FAIL: ordering cycle: " + " -> ".join(c))
    sys.exit(1)
print("no ordering cycles across %d units" % len(edges))
PY
echo "unit-order ok"
