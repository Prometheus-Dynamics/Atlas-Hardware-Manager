#!/usr/bin/env python3
"""The board-side USB network interface name (L2b).

The default composition names the gadget's NCM interface with `ifname=lemnos%d` (Lemnos
drivers/usb-gadget DEFAULT_INTERFACES; a board file with no `interfaces` gets it). The kernel gives
the first free number, so the board-side name is `lemnos0`. A systemd-networkd file under devices/
that matches the gadget's name must name exactly that interface: a wildcard such as `usb*` or
`lemnos*` would also catch a host-role USB adapter, so the match here is exact.

LOCK: the name is read from Lemnos contracts/usb-link.toml `[network] board_interface` (version 3, Lemnos
R1i) when LEMNOS_USB_LINK names that file; otherwise the tip's DEFAULT_INTERFACES `lemnos%d` -> `lemnos0` is used.

Run from the repository root. `--self-test` checks the rule on fixtures.
"""

import glob
import os
import re
import sys
import tempfile
import tomllib

# The contracts file of the Lemnos source the lock names. The gate has no Lemnos checkout, so the path comes from
# LEMNOS_USB_LINK (a Gaia-fetched or locally exported contracts/usb-link.toml). Unset, or a file with no [network]
# table (Lemnos's version 2, which is what the tip and the lock carry today), the fallback below is used.
CONTRACT = os.environ.get("LEMNOS_USB_LINK", "")
# LOCK: Lemnos R1i (contracts/usb-link.toml version 3, [network] board_interface_template / board_interface).
# Fallback until the lock has it: the tip's DEFAULT_INTERFACES `ifname=lemnos%d`, first free number lemnos0.
FALLBACK_TEMPLATE = "lemnos%d"
FALLBACK_FIRST = "lemnos0"


def contract_names():
    """(template, first name, where it came from) from the contracts file's [network] table, else the fallback."""
    if CONTRACT and os.path.isfile(CONTRACT):
        with open(CONTRACT, "rb") as handle:
            network = tomllib.load(handle).get("network")
        if network and "board_interface" in network:
            return network.get("board_interface_template", FALLBACK_TEMPLATE), network["board_interface"], CONTRACT
    return FALLBACK_TEMPLATE, FALLBACK_FIRST, "the fallback (LOCK: Lemnos R1i)"


TEMPLATE, FIRST, SOURCE = contract_names()
# Names the gadget's network interface can take (kernel usb<N> without ifname, or the template's lemnos<N>).
GADGET_NAME = re.compile(r"^(usb|lemnos)[0-9]+$")


def network_names(path):
    """The `Name=` values under [Match] of a systemd-networkd file, in order."""
    names = []
    in_match = False
    with open(path, encoding="utf-8") as handle:
        for line in handle:
            text = line.strip()
            if text.startswith("["):
                in_match = text == "[Match]"
            elif in_match and text.startswith("Name="):
                names.extend(part for part in text[len("Name="):].split() if part)
    return names


def check_tree(root):
    problems = []
    for path in sorted(glob.glob(os.path.join(root, "devices", "**", "*.network"), recursive=True)):
        rel = os.path.relpath(path, root)
        for name in network_names(path):
            if GADGET_NAME.match(name) and name != FIRST:
                problems.append(f"{rel}: [Match] Name={name}: the gadget's network interface is {FIRST} (default composition ifname={TEMPLATE})")
    return problems


def self_test():
    cases = {
        "gadget name lemnos0 passes": ("[Match]\nName=lemnos0\n", True),
        "usb0 for the gadget is refused": ("[Match]\nName=usb0\n", False),
        "a host adapter name is not matched": ("[Match]\nName=enx001122334455\n", True),
        "a wildcard names no interface and is not matched": ("[Match]\nName=lemnos*\n", True),
    }
    failed = False
    for label, (text, passes) in cases.items():
        with tempfile.TemporaryDirectory() as root:
            directory = os.path.join(root, "devices", "products", "fixture")
            os.makedirs(directory)
            with open(os.path.join(directory, "x.network"), "w", encoding="utf-8") as handle:
                handle.write(text)
            if (not check_tree(root)) != passes:
                print(f"check-usb-ifname: self-test: {label} gave {check_tree(root)}", file=sys.stderr)
                failed = True
    if failed:
        return 1
    print("check-usb-ifname: self-test: the gadget interface name is refused unless it is lemnos0; a host adapter name is not matched")
    return 0


def main():
    if "--self-test" in sys.argv:
        return self_test()
    problems = check_tree(".")
    for problem in problems:
        print(f"check-usb-ifname: FAIL {problem}", file=sys.stderr)
    if problems:
        return 1
    found = sum(len(network_names(p)) for p in glob.glob("devices/**/*.network", recursive=True))
    print(f"check-usb-ifname: passed ({found} networkd name(s) under devices/; the gadget's interface is {FIRST}, from {SOURCE})")
    return self_test()


if __name__ == "__main__":
    sys.exit(main())
