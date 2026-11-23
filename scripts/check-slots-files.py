#!/usr/bin/env python3
"""Every device that takes files through the Orion bridge is given its directory, and no other name is.

The bridge registers a file sink only for the devices LEMNOS_ORION_FILES names (board: Lemnos bbb3c420
service/orion/src/files.rs:3-5, config.rs:78); unset, no device takes files, and a slot bundle pushed to
lemnos/<device>/<name> never reaches the device's inbox, so every live install is refused NOT_FOUND (bar run 10,
2026-11-23). Under ProtectSystem=strict the inbox must also be writable through a drop-in
(packaging/systemd/lemnos-orion.service:2-4).

For each product image whose components install lemnos-orion.service, and whose board's Lemnos file has a
`system-slots` device: some installed lemnos-orion drop-in sets LEMNOS_ORION_FILES with `<device id>=<data>/slots/.incoming`
(`data` from the device's config, default /data) and lists that directory in ReadWritePaths.

Run from the repository root. `--self-test` checks the rule on fixtures.
"""

import os
import re
import sys
import tomllib

DROPIN = re.compile(r"^/etc/systemd/system/lemnos-orion\.service\.d/[^/]+\.conf$")
ENVFILE = "/etc/lemnos/lemnos-orion.env"
UNIT = "/usr/lib/systemd/system/lemnos-orion.service"
BOARD = "/etc/lemnos/board.toml"
# The drivers that take files through the bridge: system-slots (`install_file`, from <data>/slots/.incoming:
# board: Lemnos bbb3c420 packaging/systemd/lemnos-orion.env.example:23) and system-flash (the first-flash stream,
# docs/slots.md:3309). None other has an accept or a file method at the pin.
TAKES_FILES = ("system-slots", "system-flash")


def file_devices(lemnos_board):
    """{id: inbox or None} for each device whose driver takes files; None: any directory."""
    found = {}
    for dev in lemnos_board.get("devices", []):
        if dev.get("driver") == "system-slots":
            data = dev.get("config", {}).get("data", "/data").rstrip("/")
            found[dev["id"]] = f"{data}/slots/.incoming"
        elif dev.get("driver") in TAKES_FILES:
            found[dev["id"]] = None
    return found


def gives(text):
    """({device: dir} of LEMNOS_ORION_FILES, {stream devices}, {ReadWritePaths}) of a drop-in's or env file's text."""
    files, streams, writable = {}, set(), set()
    for raw in text.splitlines():
        line = raw.strip()
        if line.startswith("Environment="):
            line = line[len("Environment="):].strip('"')
        m = re.match(r"^LEMNOS_ORION_FILES=(.*)$", line)
        if m:
            for entry in m.group(1).split(","):
                dev, _, path = entry.strip().partition("=")
                if dev:
                    files[dev] = path.rstrip("/")
        m = re.match(r"^LEMNOS_ORION_STREAMS=(.*)$", line)
        if m:
            streams.update(e.partition("=")[0].strip() for e in m.group(1).split(",") if e.strip())
        m = re.match(r"^ReadWritePaths=(.*)$", line)
        if m:
            writable.update(p.lstrip("-").rstrip("/") for p in m.group(1).split())
    return files, streams, writable


def check(image_name, devices, has_bridge, systemd_unit, texts):
    """Errors for one image. devices: file_devices() of its board file; texts: its drop-ins and env file."""
    if not has_bridge:
        return []
    files, streams, writable = {}, set(), set()
    for text in texts:
        f, st, w = gives(text)
        files.update(f)
        streams |= st
        writable |= w
    errors = []
    for dev, inbox in sorted(devices.items()):
        if dev not in files:
            errors.append(f"{image_name}: the {dev!r} device takes files, and LEMNOS_ORION_FILES names it nowhere "
                          f"(the bridge gives it {files or 'nothing'}): every push to lemnos/{dev}/ is lost")
        elif inbox is not None and files[dev] != inbox:
            errors.append(f"{image_name}: LEMNOS_ORION_FILES gives {dev}={files[dev]}, and the slots device reads {inbox}")
        elif systemd_unit and files[dev] not in writable:
            errors.append(f"{image_name}: {files[dev]} is not in the lemnos-orion drop-ins' ReadWritePaths (ProtectSystem=strict)")
    for dev in sorted(set(files) | streams):
        if dev not in devices:
            errors.append(f"{image_name}: the bridge is given files or streams for {dev!r}, which is no file-taking device in the board file")
    return errors


def load(path):
    with open(path, "rb") as handle:
        return tomllib.load(handle)


def run_tree():
    errors, checked = [], 0
    for product_dir in sorted(os.listdir("devices/products")):
        product_path = os.path.join("devices/products", product_dir, "product.toml")
        if not os.path.exists(product_path):
            continue
        product = load(product_path)
        base = os.path.dirname(product_path)
        for image in product.get("images", []):
            board_path = os.path.normpath(os.path.join(base, image.get("board", "")))
            board = load(board_path) if os.path.isfile(board_path) else {}
            devices, has_bridge, systemd_unit, texts = {}, False, False, []
            for comp in image.get("components", []):
                cpath = os.path.normpath(os.path.join(base, product["components"][comp]))
                for item in load(cpath).get("install", []):
                    to, src = item.get("to", ""), item.get("from", {})
                    has_bridge |= "lemnos-orion" in to
                    systemd_unit |= to == UNIT
                    if to == BOARD:
                        key = src.get("description")
                        path = board.get("descriptions", {}).get(key) if key else None
                        if path:
                            devices = file_devices(load(os.path.join(os.path.dirname(board_path), path)))
                        elif "file" in src:
                            devices = file_devices(load(os.path.join(os.path.dirname(cpath), src["file"])))
                    if (DROPIN.match(to) or to == ENVFILE) and "file" in src:
                        with open(os.path.join(os.path.dirname(cpath), src["file"])) as f:
                            texts.append(f.read())
            if has_bridge:
                checked += 1
            errors += check(f"{product_dir}/{image.get('name', '?')}", devices, has_bridge, systemd_unit, texts)
    return errors, checked


def self_test():
    slots = {"slots": "/data/slots/.incoming"}
    good = "[Service]\nEnvironment=LEMNOS_ORION_FILES=slots=/data/slots/.incoming\nReadWritePaths=-/data/slots/.incoming\n"
    assert check("x", slots, True, True, [good]) == []
    # The fault bar run 10 found: only the links drop-in.
    links = "[Service]\nEnvironment=LEMNOS_ORION_LINKS=gadget=orion-link\n"
    assert any("names it nowhere" in e for e in check("x", slots, True, True, [links]))
    # Named, but not writable under systemd; fine under BusyBox init (no unit).
    named = "[Service]\nEnvironment=LEMNOS_ORION_FILES=slots=/data/slots/.incoming\n"
    assert any("ReadWritePaths" in e for e in check("x", slots, True, True, [named]))
    assert check("x", slots, True, False, [named]) == []
    # The wrong directory for a slots device.
    wrong = "[Service]\nEnvironment=LEMNOS_ORION_FILES=slots=/data/slots\nReadWritePaths=/data/slots\n"
    assert any("reads /data/slots/.incoming" in e for e in check("x", slots, True, True, [wrong]))
    # A flash device takes files from any directory; the env-file form (the flasher's), with streams.
    flash = {"flash": None}
    env = "LEMNOS_ORION_STREAMS=flash=image.img.xz+image.img\nLEMNOS_ORION_FILES=flash=/run/lemnos-flash\n"
    assert check("x", flash, True, False, [env]) == []
    # A name the board file does not have, in files or in streams.
    assert any("'slotz'" in e for e in check("x", slots, True, True, [good, "LEMNOS_ORION_FILES=slotz=/x\n"]))
    assert any("'flash'" in e for e in check("x", slots, True, True, [good, "LEMNOS_ORION_STREAMS=flash=a+b\n"]))
    # No bridge on the image: nothing to check. A board with no file-taking device and no names: fine.
    assert check("x", slots, False, False, []) == [] and check("x", {}, True, True, [links]) == []
    # The data directory comes from the device's config.
    assert file_devices({"devices": [{"id": "s", "driver": "system-slots", "config": {"data": "/var/d/"}},
                                     {"id": "f", "driver": "system-flash"}, {"id": "g", "driver": "usb-gadget"}]}) \
        == {"s": "/var/d/slots/.incoming", "f": None}
    print("check-slots-files self-test: ok")


def main():
    if sys.argv[1:] == ["--self-test"]:
        self_test()
        return 0
    errors, checked = run_tree()
    for e in errors:
        print(f"FAIL: {e}", file=sys.stderr)
    if errors:
        return 1
    print(f"bridge files: {checked} image(s) with the bridge; every file-taking device is given its directory, and no other name is")
    return 0


if __name__ == "__main__":
    sys.exit(main())
