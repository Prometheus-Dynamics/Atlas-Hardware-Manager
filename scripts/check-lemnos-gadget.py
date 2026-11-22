#!/usr/bin/env python3
"""The Lemnos gadget drop-in pairing (Lemnos R1d).

The base lemnosd unit has no gadget settings. A board whose board file has a `usb-gadget` device
needs the drop-in `packaging/systemd/lemnosd.service.d/gadget.conf` at
/etc/systemd/system/lemnosd.service.d/gadget.conf, and a board without one must not get it. An image with
BusyBox init (`BR2_INIT_BUSYBOX=y` in its OS fragments: the flasher) has no systemd, so it takes no drop-in at all:
S50lemnosd starts the gadget's root phase itself (Lemnos packaging/init).

lemnosd needs configfs mounted at /sys/kernel/config and never mounts it itself (Lemnos docs/daemon.md: an unmounted
configfs is "not there yet", tried again at the next setup, forever). A systemd image gets the mount from systemd's
sys-kernel-config.mount. A BusyBox image gets it only from /etc/fstab (inittab's `mount -a`), so a BusyBox image whose
board file has a usb-gadget device must assemble /etc/fstab with a component's `{ file }` fragment holding a
`configfs /sys/kernel/config configfs ...` line. Without it the gadget never binds the UDC: the bcm2712 flasher's first
RAM boot on 2026-11-22 hung exactly so.

The board file is the one the image carries: a component installs it as /etc/lemnos/board.toml with
`from = { description = "<name>" }`, and the board names it under `[descriptions]` (PR B26): `lemnos` for the
board's running image, another name for another of its board files (the flasher's, D6). So the file is read
through the board of each image that names the component, never through a copy under a component. The check
fails both ways, for every image of every product.

Run as `scripts/check-lemnos-gadget.py` from the repository root; `--self-test` checks the rule on
fixtures (a gadget board with and without the drop-in, a board without a gadget with and without it, BusyBox
gadget images with and without the configfs fstab line), and on a board that names no Lemnos board file.
"""

import glob
import os
import sys
import tempfile
import tomllib

BOARD_TO = "/etc/lemnos/board.toml"
DROP_IN_TO = "/etc/systemd/system/lemnosd.service.d/gadget.conf"
DESCRIPTION = "lemnos"
FSTAB = "/etc/fstab"
CONFIGFS = "/sys/kernel/config"


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


def check_image(product_dir, component_paths, board_dir, label, systemd=True, gadgets=None):
    """Returns the problems for one image: its components' installs against its board's Lemnos file. When `gadgets`
    is a list, True is appended to it if the image's Lemnos board file has a usb-gadget device."""
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
        if gadget and gadgets is not None:
            gadgets.append(True)
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


def mounts_configfs(text):
    """Whether fstab text has a configfs line on /sys/kernel/config."""
    for line in text.splitlines():
        fields = line.split()
        if len(fields) >= 3 and not fields[0].startswith("#") and fields[1] == CONFIGFS and fields[2] == "configfs":
            return True
    return False


def configfs_problems(product_dir, product, image, label):
    """A BusyBox image with a gadget: some component's `{ file }` fragment of /etc/fstab mounts configfs, and the
    product assembles /etc/fstab naming that component."""
    components = product.get("components", {})
    givers = []
    for name in image.get("components", []):
        if name not in components:
            continue
        component_dir = os.path.dirname(os.path.normpath(os.path.join(product_dir, components[name])))
        for fragment in load(os.path.join(component_dir, "component.toml")).get("fragments", []):
            source = fragment.get("from")
            if fragment.get("to") != FSTAB or not isinstance(source, dict) or set(source) != {"file"}:
                continue
            path = os.path.join(component_dir, source["file"])
            if os.path.isfile(path):
                with open(path) as handle:
                    if mounts_configfs(handle.read()):
                        givers.append(name)
    if not givers:
        return [f"{label}: BusyBox init and a usb-gadget device, but no component gives an {FSTAB} `{{ file }}` fragment "
                f"mounting configfs at {CONFIGFS}: lemnosd's gadget would never bind the UDC"]
    assembled = [a for a in image.get("assembled", []) if a.get("path") == FSTAB]
    if not assembled or not any(name in assembled[0].get("order", []) for name in givers):
        return [f"{label}: {', '.join(givers)} mounts configfs in an {FSTAB} fragment, but the image's "
                f"[[images.assembled]] for {FSTAB} does not name it"]
    return []


def check_product(product_file):
    product_dir = os.path.dirname(product_file)
    product = load(product_file)
    problems = []
    for image in product.get("images", []):
        board_dir = os.path.dirname(os.path.normpath(os.path.join(product_dir, image["board"])))
        paths = [product.get("components", {})[name] for name in image.get("components", []) if name in product.get("components", {})]
        label = f"{product_file} image {image.get('name')!r}"
        systemd = not busybox_init(product_dir, image)
        gadgets = []
        problems.extend(check_image(product_dir, paths, board_dir, label, systemd=systemd, gadgets=gadgets))
        if gadgets and not systemd:
            problems.extend(configfs_problems(product_dir, product, image, label))
    return problems


def write(root, path, text):
    full = os.path.join(root, path)
    os.makedirs(os.path.dirname(full), exist_ok=True)
    with open(full, "w") as handle:
        handle.write(text)


def fixture(root, name, gadget, drop_in, description=True, busybox=False, listed=DESCRIPTION, installed=DESCRIPTION,
            configfs=None):
    """`configfs`: None for no fstab fragment, "assembled" for a configfs fragment the product assembles, "unassembled"
    for one it does not, "other" for an assembled fragment without the configfs line."""
    """A board with a Lemnos board file, a component that installs it by description, and a product."""
    board = f'version = 1\nname = "{name}"\n'
    if description:
        board += f'\n[descriptions]\n{listed} = "lemnos-board.toml"\n'
    write(root, f"devices/boards/{name}/board.toml", board)
    driver = "usb-gadget" if gadget else "system-reboot"
    write(root, f"devices/boards/{name}/lemnos-board.toml", f'[[devices]]\nid = "x"\ndriver = "{driver}"\n')
    text = f'version = 1\nname = "lemnos-fixture"\n[[install]]\nfrom = {{ description = "{installed}" }}\nto = "{BOARD_TO}"\n'
    if configfs:
        line = "configfs /sys/kernel/config configfs defaults 0 0\n" if configfs != "other" else "tmpfs /x tmpfs defaults 0 0\n"
        write(root, "devices/components/lemnos-fixture/fstab.fragment", line)
        text += f'[[fragments]]\nfrom = {{ file = "fstab.fragment" }}\nto = "{FSTAB}"\n'
    if drop_in:
        text += (
            '[[install]]\nfrom = { source = "lemnos", path = "packaging/systemd/lemnosd.service.d/gadget.conf" }\n'
            f'to = "{DROP_IN_TO}"\n'
        )
    write(root, "devices/components/lemnos-fixture/component.toml", text)
    write(root, f"devices/products/{name}-p/product.toml",
          f'version = 1\nname = "{name}-p"\n[components]\nlemnos-fixture = "../../components/lemnos-fixture/component.toml"\n'
          f'[[images]]\nname = "{name}-p"\nboard = "../../boards/{name}/board.toml"\ncomponents = ["lemnos-fixture"]\n'
          + ('os = { fragments = ["os/init.fragment"] }\n' if busybox else '')
          + (f'[[images.assembled]]\npath = "{FSTAB}"\norder = ["os", "lemnos-fixture"]\nrule_key = 2\n'
             if configfs in ("assembled", "other") else ''))
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
        "busybox-gadget-without-drop-in": (True, False, True, True, {"busybox": True, "configfs": "assembled"}),
        "busybox-gadget-with-drop-in": (True, True, True, False, {"busybox": True, "configfs": "assembled"}),
        "busybox-gadget-without-configfs": (True, False, True, False, {"busybox": True}),
        "busybox-gadget-configfs-not-assembled": (True, False, True, False, {"busybox": True, "configfs": "unassembled"}),
        "busybox-gadget-fstab-without-configfs": (True, False, True, False, {"busybox": True, "configfs": "other"}),
        "busybox-no-gadget-without-configfs": (False, False, True, True, {"busybox": True}),
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
    print("check-lemnos-gadget: self-test: the drop-in pairing refuses both ways, passes both ways, takes no drop-in on BusyBox init, needs configfs in a BusyBox gadget image's fstab, reads any board file the board names, and refuses a board that names no such file")
    return 0


def main():
    if "--self-test" in sys.argv:
        return self_test()
    problems = check_tree(".")
    for problem in problems:
        print(f"check-lemnos-gadget: FAIL {problem}", file=sys.stderr)
    if problems:
        return 1
    print("check-lemnos-gadget: passed (the gadget drop-in is installed exactly where the board has a usb-gadget device; "
          "every BusyBox gadget image mounts configfs)")
    return self_test()


if __name__ == "__main__":
    sys.exit(main())
