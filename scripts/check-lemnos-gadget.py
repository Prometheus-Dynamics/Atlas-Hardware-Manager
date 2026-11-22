#!/usr/bin/env python3
"""The Lemnos gadget drop-in pairing (Lemnos R1d).

The base lemnosd unit has no gadget settings. A board whose board file has a `usb-gadget` device
needs the drop-in `packaging/systemd/lemnosd.service.d/gadget.conf` at
/etc/systemd/system/lemnosd.service.d/gadget.conf, and a board without one must not get it. An image with
BusyBox init (`BR2_INIT_BUSYBOX=y` in its OS fragments: the flasher) has no systemd, so it takes no drop-in at all:
S50lemnosd starts the gadget's root phase itself (Lemnos packaging/init).

The board file is the one the image carries: a component installs it as /etc/lemnos/board.toml with
`from = { description = "<name>" }`, and the board names it under `[descriptions]` (PR B26): `lemnos` for the
board's running image, another name for another of its board files (the flasher's, D6). So the file is read
through the board of each image that names the component, never through a copy under a component. The check
fails both ways, for every image of every product.

Run as `scripts/check-lemnos-gadget.py` from the repository root; `--self-test` checks the rule on
four fixtures (a gadget board with and without the drop-in, and a board without a gadget with and
without it), and on a board that names no Lemnos board file.
"""

import glob
import os
import sys
import tempfile
import tomllib

BOARD_TO = "/etc/lemnos/board.toml"
DROP_IN_TO = "/etc/systemd/system/lemnosd.service.d/gadget.conf"
DESCRIPTION = "lemnos"


def load(path):
    with open(path, "rb") as handle:
        return tomllib.load(handle)


def board_file(board_dir, name=DESCRIPTION):
    """The board's Lemnos board file `name`, through its [descriptions] (the family it extends is the fallback)."""
    board = load(os.path.join(board_dir, "board.toml"))
    names = board.get("descriptions")
    if names is None and "extends" in board:
        family = os.path.normpath(os.path.join(board_dir, board["extends"]))
        names = load(family).get("descriptions")
    if not names or name not in names:
        return None
    return os.path.normpath(os.path.join(board_dir, names[name]))


def board_has_gadget(path):
    board = load(path)
    return any(device.get("driver") == "usb-gadget" for device in board.get("devices", []))


def check_image(product_dir, component_paths, board_dir, label, systemd=True):
    """Returns the problems for one image: its components' installs against its board's Lemnos file."""
    problems = []
    board_path = None
    for component in component_paths:
        component_dir = os.path.dirname(os.path.normpath(os.path.join(product_dir, component)))
        installed = load(os.path.join(component_dir, "component.toml")).get("install", [])
        boards = [i for i in installed if i.get("to") == BOARD_TO]
        drop_ins = [i for i in installed if i.get("to") == DROP_IN_TO]
        if len(boards) > 1:
            problems.append(f"{component_dir}: installs {BOARD_TO} more than once")
        if not boards:
            if drop_ins:
                problems.append(f"{component_dir}: installs the gadget drop-in but no board file")
            continue
        source = boards[0].get("from")
        if not isinstance(source, dict) or set(source) != {"description"}:
            problems.append(f"{component_dir}: installs {BOARD_TO} from {source}, not from one of the board's descriptions")
            continue
        if board_path is None:
            board_path = board_file(board_dir, source["description"])
            if board_path is None or not os.path.isfile(board_path):
                problems.append(f"{label}: {component_dir} installs {BOARD_TO} from description `{source['description']}`, and its board {board_dir} names no such Lemnos board file that exists")
                return problems
        gadget = board_has_gadget(board_path)
        if not systemd:
            if drop_ins:
                problems.append(f"{label}: {component_dir} installs {DROP_IN_TO}, but the image has BusyBox init and no systemd")
            continue
        if gadget and not drop_ins:
            problems.append(f"{board_path} has a usb-gadget device but {component_dir} does not install {DROP_IN_TO}")
        if not gadget and drop_ins:
            problems.append(f"{board_path} has no usb-gadget device but {component_dir} installs {DROP_IN_TO}")
        for drop in drop_ins:
            if drop.get("from", {}).get("path") != "packaging/systemd/lemnosd.service.d/gadget.conf":
                problems.append(f"{component_dir}: the gadget drop-in is not Lemnos's gadget.conf")
    return problems


def busybox_init(product_dir, image):
    """Whether the image's OS fragments choose BusyBox init."""
    for fragment in (image.get("os") or {}).get("fragments", []):
        path = os.path.join(product_dir, fragment)
        if os.path.isfile(path):
            with open(path) as handle:
                if any(line.strip() == "BR2_INIT_BUSYBOX=y" for line in handle):
                    return True
    return False


def check_product(product_file):
    product_dir = os.path.dirname(product_file)
    product = load(product_file)
    problems = []
    for image in product.get("images", []):
        board_dir = os.path.dirname(os.path.normpath(os.path.join(product_dir, image["board"])))
        paths = [product.get("components", {})[name] for name in image.get("components", []) if name in product.get("components", {})]
        problems.extend(check_image(product_dir, paths, board_dir, f"{product_file} image {image.get('name')!r}",
                                    systemd=not busybox_init(product_dir, image)))
    return problems


def write(root, path, text):
    full = os.path.join(root, path)
    os.makedirs(os.path.dirname(full), exist_ok=True)
    with open(full, "w") as handle:
        handle.write(text)


def fixture(root, name, gadget, drop_in, description=True, busybox=False, listed=DESCRIPTION, installed=DESCRIPTION):
    """A board with a Lemnos board file, a component that installs it by description, and a product."""
    board = f'version = 1\nname = "{name}"\n'
    if description:
        board += f'\n[descriptions]\n{listed} = "lemnos-board.toml"\n'
    write(root, f"devices/boards/{name}/board.toml", board)
    driver = "usb-gadget" if gadget else "system-reboot"
    write(root, f"devices/boards/{name}/lemnos-board.toml", f'[[devices]]\nid = "x"\ndriver = "{driver}"\n')
    text = f'version = 1\nname = "lemnos-fixture"\n[[install]]\nfrom = {{ description = "{installed}" }}\nto = "{BOARD_TO}"\n'
    if drop_in:
        text += (
            '[[install]]\nfrom = { source = "lemnos", path = "packaging/systemd/lemnosd.service.d/gadget.conf" }\n'
            f'to = "{DROP_IN_TO}"\n'
        )
    write(root, "devices/components/lemnos-fixture/component.toml", text)
    write(root, f"devices/products/{name}-p/product.toml",
          f'version = 1\nname = "{name}-p"\n[components]\nlemnos-fixture = "../../components/lemnos-fixture/component.toml"\n'
          f'[[images]]\nname = "{name}-p"\nboard = "../../boards/{name}/board.toml"\ncomponents = ["lemnos-fixture"]\n'
          + ('os = { fragments = ["os/init.fragment"] }\n' if busybox else ''))
    if busybox:
        write(root, f"devices/products/{name}-p/os/init.fragment", "BR2_INIT_BUSYBOX=y\n")


def check_tree(root):
    problems = []
    for product_file in sorted(glob.glob(os.path.join(root, "devices", "products", "*", "product.toml"))):
        problems.extend(check_product(product_file))
    return problems


def self_test():
    # (gadget, drop-in installed, whether the board names its Lemnos file, expected to pass, extra fixture settings)
    cases = {
        "gadget-with-drop-in": (True, True, True, True, {}),
        "gadget-without-drop-in": (True, False, True, False, {}),
        "no-gadget-without-drop-in": (False, False, True, True, {}),
        "no-gadget-with-drop-in": (False, True, True, False, {}),
        "board-names-no-lemnos-file": (True, True, False, False, {}),
        "busybox-gadget-without-drop-in": (True, False, True, True, {"busybox": True}),
        "busybox-gadget-with-drop-in": (True, True, True, False, {"busybox": True}),
        "another-description-the-board-names": (True, True, True, True, {"listed": "flasher-lemnos", "installed": "flasher-lemnos"}),
        "a-description-the-board-does-not-name": (True, True, True, False, {"installed": "flasher-lemnos"}),
    }
    failed = False
    for name, (gadget, drop_in, described, passes, extra) in cases.items():
        with tempfile.TemporaryDirectory() as root:
            fixture(root, name, gadget, drop_in, description=described, **extra)
            problems = check_tree(root)
            if (not problems) != passes:
                print(f"check-lemnos-gadget: self-test: {name} gave {problems}", file=sys.stderr)
                failed = True
    if failed:
        return 1
    print("check-lemnos-gadget: self-test: the drop-in pairing refuses both ways, passes both ways, takes no drop-in on BusyBox init, reads any board file the board names, and refuses a board that names no such file")
    return 0


def main():
    if "--self-test" in sys.argv:
        return self_test()
    problems = check_tree(".")
    for problem in problems:
        print(f"check-lemnos-gadget: FAIL {problem}", file=sys.stderr)
    if problems:
        return 1
    print("check-lemnos-gadget: passed (the gadget drop-in is installed exactly where the board has a usb-gadget device)")
    return self_test()


if __name__ == "__main__":
    sys.exit(main())
