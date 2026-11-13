#!/usr/bin/env python3
"""The board id rule (PR B24): one string per board.

A board's id is the directory under devices/boards/, and it must also be the Gaia board `name` (its
board.toml), the Lemnos `[board] id` of each Lemnos board file (lemnos-board.toml and lemnos-board.pending.toml
beside the board), and the one line of /etc/usb-gadget.family in a product whose image has a bring-up USB
network (the gadget's addresses are derived from that line). The Lemnos board file an image carries is the one
the board names under `[descriptions] lemnos` (PR B26): it is read through the board of each image, never
through a component's copy. A product-local board (a board inside a product directory) takes the id of the
boards/ directory it stands for, so its name must still be one of those directories.

An id is frozen once an image has shipped: changing it changes the board's USB addresses, its identity key
and the slots it accepts (devices/README.md).

PLACEHOLDER (Gaia's next PR, not written yet): when the board data carries `[identity] serial = "device-tree"
| "cpuinfo"` and `serial_path`, those must agree with the Lemnos board file's system-identity device serial
source for the same board. Not checked here until those keys exist at the pinned Gaia commit.

Run from the repository root. `--self-test` checks the rule on fixtures.
"""

import glob
import os
import sys
import tempfile
import tomllib

FAMILY_TO = "/etc/usb-gadget.family"
NETWORK_TO = "/etc/init.d/S41usb0"


def load(path):
    with open(path, "rb") as handle:
        return tomllib.load(handle)


def installs(component_dir):
    path = os.path.join(component_dir, "component.toml")
    return load(path).get("install", []) if os.path.isfile(path) else []


def lemnos_file(board_dir):
    """The board's Lemnos board file under [descriptions] (its family's, when it extends one), or None."""
    board = load(os.path.join(board_dir, "board.toml"))
    names = board.get("descriptions")
    if names is None and "extends" in board:
        names = load(os.path.normpath(os.path.join(board_dir, board["extends"]))).get("descriptions")
    if not names or "lemnos" not in names:
        return None
    return os.path.normpath(os.path.join(board_dir, names["lemnos"]))


def check(root):
    problems = []
    boards = os.path.join(root, "devices", "boards")
    board_names = set(
        d for d in os.listdir(boards)
        if os.path.isfile(os.path.join(boards, d, "board.toml"))
    ) if os.path.isdir(boards) else set()

    for name in sorted(board_names):
        directory = os.path.join(boards, name)
        gaia_name = load(os.path.join(directory, "board.toml")).get("name")
        if gaia_name != name:
            problems.append(f"devices/boards/{name}/board.toml: name is {gaia_name!r}, the directory is {name!r}")
        for lemnos in sorted(glob.glob(os.path.join(directory, "lemnos-board*.toml"))):
            table = load(lemnos).get("board")
            if table is None:
                continue  # a pending file with no [board] yet; its id is the directory's when written
            if table.get("id") != name:
                problems.append(f"{os.path.relpath(lemnos, root)}: [board] id is {table.get('id')!r}, the board is {name!r}")

    products = os.path.join(root, "devices", "products")
    for product_dir in sorted(glob.glob(os.path.join(products, "*"))):
        product_file = os.path.join(product_dir, "product.toml")
        if not os.path.isfile(product_file):
            continue
        product = load(product_file)
        rel = os.path.relpath(product_file, root)
        components = product.get("components", {})
        family_lines = {}
        network = False
        for key, path in components.items():
            component_dir = os.path.dirname(os.path.normpath(os.path.join(product_dir, path)))
            for install in installs(component_dir):
                if install.get("to") == NETWORK_TO:
                    network = True
                if install.get("to") == FAMILY_TO:
                    source = install["from"]["file"]
                    with open(os.path.join(component_dir, source), encoding="utf-8") as handle:
                        family_lines[key] = handle.readline().rstrip("\n")
        for image in product.get("images", []):
            board_file = os.path.normpath(os.path.join(product_dir, image["board"]))
            board_name = load(board_file).get("name")
            if board_name not in board_names:
                problems.append(f"{rel}: image {image.get('name')!r} board name {board_name!r} is no devices/boards/ directory")
            lemnos = lemnos_file(os.path.dirname(board_file))
            if lemnos is not None and os.path.isfile(lemnos):
                table = load(lemnos).get("board", {})
                if table.get("id") != board_name:
                    problems.append(f"{os.path.relpath(lemnos, root)}: [board] id is {table.get('id')!r}, the image's board is {board_name!r}")
            if network and not family_lines:
                problems.append(f"{rel}: the USB network component is named but no component installs {FAMILY_TO}")
            for key, line in family_lines.items():
                if line != board_name:
                    problems.append(f"{rel}: {FAMILY_TO} (component {key!r}) is {line!r}, the image's board is {board_name!r}")
    return problems


def write(root, path, text):
    full = os.path.join(root, path)
    os.makedirs(os.path.dirname(full), exist_ok=True)
    with open(full, "w", encoding="utf-8") as handle:
        handle.write(text)


def fixture(root, name="alpha", name_in_file=None, lemnos_id=None, family="alpha", network=True, named_id=None):
    write(root, f"devices/boards/{name}/board.toml", f'version = 1\nname = "{name_in_file or name}"\n')
    if lemnos_id is not None:
        write(root, f"devices/boards/{name}/lemnos-board.pending.toml", f'[board]\nid = "{lemnos_id}"\n')
    if named_id is not None:
        write(root, f"devices/boards/{name}/board.toml",
              f'version = 1\nname = "{name}"\n\n[descriptions]\nlemnos = "lemnos-board.toml"\n')
        write(root, f"devices/boards/{name}/lemnos-board.toml", f'[board]\nid = "{named_id}"\n')
    write(root, "devices/components/fam/component.toml",
          'version = 1\nname = "fam"\n[[install]]\nfrom = { file = "files/usb-gadget.family" }\n'
          'to = "/etc/usb-gadget.family"\nmode = "0644"\n')
    write(root, "devices/components/fam/files/usb-gadget.family", f"{family}\n")
    net = ""
    if network:
        net = ('[[install]]\nfrom = { file = "files/S41usb0" }\nto = "/etc/init.d/S41usb0"\nmode = "0755"\n')
        write(root, "devices/components/net/component.toml", 'version = 1\nname = "net"\n' + net)
    comps = '[components]\nfam = "../../components/fam/component.toml"\n'
    if network:
        comps += 'net = "../../components/net/component.toml"\n'
    write(root, "devices/products/p1/product.toml",
          'version = 1\nname = "p1"\n' + comps +
          '[[images]]\nname = "p1"\nboard = "../../boards/alpha/board.toml"\n'
          'components = ["fam"]\n')


def self_test():
    cases = [
        ("a matching id passes", {}, []),
        ("a Gaia name that is not the directory is refused", {"name_in_file": "beta"}, ["name is 'beta'"]),
        ("a Lemnos id that is not the directory is refused", {"lemnos_id": "beta"}, ["[board] id is 'beta'"]),
        ("a Lemnos file named by the board with another id is refused", {"named_id": "beta"}, ["[board] id is 'beta', the image's board is 'alpha'"]),
        ("a Lemnos file named by the board with its own id passes", {"named_id": "alpha"}, []),
        ("a family line that is not the board is refused", {"family": "beta"}, ["is 'beta', the image's board is 'alpha'"]),
        ("a network with no family line is refused", {"family": None, "network": True}, ["no component installs"]),
    ]
    failed = []
    for label, kwargs, expect in cases:
        with tempfile.TemporaryDirectory() as root:
            if kwargs.get("family", "alpha") is None:
                kwargs = dict(kwargs, family="alpha")
                os.makedirs(os.path.join(root, "devices", "components"), exist_ok=True)
                fixture(root, **kwargs)
                os.remove(os.path.join(root, "devices", "components", "fam", "files", "usb-gadget.family"))
                os.rmdir(os.path.join(root, "devices", "components", "fam", "files"))
                write(root, "devices/components/fam/component.toml", 'version = 1\nname = "fam"\n')
            else:
                fixture(root, **kwargs)
            found = check(root)
            ok = (not expect and not found) or (expect and found and all(any(e in p for p in found) for e in expect))
            if not ok:
                failed.append(f"{label}: {found}")
    for line in failed:
        print(f"check-board-ids: self-test FAIL {line}", file=sys.stderr)
    if failed:
        return 1
    print("check-board-ids: self-test: the id rule refuses a directory, Gaia name, Lemnos id or family line that disagree, and passes a match")
    return 0


def main():
    if "--self-test" in sys.argv:
        return self_test()
    problems = check(".")
    for problem in problems:
        print(f"check-board-ids: FAIL {problem}", file=sys.stderr)
    if problems:
        return 1
    print("check-board-ids: passed (every board's directory, Gaia name, Lemnos id and bring-up family line are one string)")
    return self_test()


if __name__ == "__main__":
    sys.exit(main())
