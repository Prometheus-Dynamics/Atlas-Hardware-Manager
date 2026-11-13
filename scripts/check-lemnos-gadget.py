#!/usr/bin/env python3
"""The Lemnos gadget drop-in pairing (Lemnos R1d).

The base lemnosd unit has no gadget settings. A board whose board file has a `usb-gadget` device
needs the drop-in `packaging/systemd/lemnosd.service.d/gadget.conf` at
/etc/systemd/system/lemnosd.service.d/gadget.conf, and a board without one must not get it. This
reads both facts from the data: the board file each component installs as /etc/lemnos/board.toml
(its `[[devices]]` drivers) and the component's own `[[install]]` lines. It fails both ways.

Run as `scripts/check-lemnos-gadget.py` from the repository root; `--self-test` checks the rule on
four fixtures (a gadget board with and without the drop-in, and a board without a gadget with and
without it).
"""

import glob
import os
import sys
import tempfile
import tomllib

BOARD_TO = "/etc/lemnos/board.toml"
DROP_IN_TO = "/etc/systemd/system/lemnosd.service.d/gadget.conf"


def installs(component_dir):
    with open(os.path.join(component_dir, "component.toml"), "rb") as handle:
        return tomllib.load(handle).get("install", [])


def board_has_gadget(board_path):
    with open(board_path, "rb") as handle:
        board = tomllib.load(handle)
    return any(device.get("driver") == "usb-gadget" for device in board.get("devices", []))


def check_component(component_dir):
    """Returns a list of problems for one component (empty when it pairs)."""
    installed = installs(component_dir)
    boards = [i for i in installed if i.get("to") == BOARD_TO]
    drop_ins = [i for i in installed if i.get("to") == DROP_IN_TO]
    problems = []
    if len(boards) > 1:
        problems.append(f"{component_dir}: installs {BOARD_TO} more than once")
    if not boards:
        if drop_ins:
            problems.append(f"{component_dir}: installs the gadget drop-in but no board file")
        return problems
    board_from = boards[0].get("from", {})
    board_path = os.path.join(component_dir, board_from.get("file", ""))
    gadget = board_has_gadget(board_path)
    if gadget and not drop_ins:
        problems.append(f"{board_path} has a usb-gadget device but {component_dir} does not install {DROP_IN_TO}")
    if not gadget and drop_ins:
        problems.append(f"{board_path} has no usb-gadget device but {component_dir} installs {DROP_IN_TO}")
    for drop in drop_ins:
        if drop.get("from", {}).get("path") != "packaging/systemd/lemnosd.service.d/gadget.conf":
            problems.append(f"{component_dir}: the gadget drop-in is not Lemnos's gadget.conf")
    return problems


def write_fixture(root, name, gadget, drop_in):
    directory = os.path.join(root, name)
    os.makedirs(os.path.join(directory, "files"))
    driver = "usb-gadget" if gadget else "system-reboot"
    with open(os.path.join(directory, "files", "board.toml"), "w") as handle:
        handle.write(f'[[devices]]\nid = "x"\ndriver = "{driver}"\n')
    text = f'name = "{name}"\n[[install]]\nfrom = {{ file = "files/board.toml" }}\nto = "{BOARD_TO}"\n'
    if drop_in:
        text += (
            '[[install]]\nfrom = { source = "lemnos", path = "packaging/systemd/lemnosd.service.d/gadget.conf" }\n'
            f'to = "{DROP_IN_TO}"\n'
        )
    with open(os.path.join(directory, "component.toml"), "w") as handle:
        handle.write(text)
    return directory


def self_test():
    cases = {
        "gadget-with-drop-in": (True, True, True),
        "gadget-without-drop-in": (True, False, False),
        "no-gadget-without-drop-in": (False, False, True),
        "no-gadget-with-drop-in": (False, True, False),
    }
    with tempfile.TemporaryDirectory() as root:
        for name, (gadget, drop_in, passes) in cases.items():
            problems = check_component(write_fixture(root, name, gadget, drop_in))
            if (not problems) != passes:
                print(f"check-lemnos-gadget: self-test: {name} gave {problems}", file=sys.stderr)
                return 1
    print("check-lemnos-gadget: self-test: the drop-in pairing refuses both ways and passes both ways")
    return 0


def main():
    if "--self-test" in sys.argv:
        return self_test()
    problems = []
    for component_dir in sorted(glob.glob("devices/components/*")):
        if os.path.isfile(os.path.join(component_dir, "component.toml")):
            problems.extend(check_component(component_dir))
    for problem in problems:
        print(f"check-lemnos-gadget: FAIL {problem}", file=sys.stderr)
    if problems:
        return 1
    print("check-lemnos-gadget: passed (the gadget drop-in is installed exactly where the board has a usb-gadget device)")
    return self_test()


if __name__ == "__main__":
    sys.exit(main())
