#!/usr/bin/env python3
"""Every device that takes files through the Orion bridge is given its directory, and no other name is.

The bridge registers a file sink only for the devices LEMNOS_ORION_FILES names (board: Lemnos 6c8568d0
service/orion/src/files.rs:3-5, config.rs:78); unset, no device takes files, and a slot bundle pushed to
lemnos/<device>/<name> never reaches the device's inbox, so every live install is refused NOT_FOUND (bar run 10,
2026-11-23). Under ProtectSystem=strict the inbox must also be writable through a drop-in
(packaging/systemd/lemnos-orion.service:2-4).

For each product image whose components install lemnos-orion.service, and whose board's Lemnos file has a
`system-slots` device: some installed lemnos-orion drop-in sets LEMNOS_ORION_FILES with `<device id>=<data>/slots/.incoming`
(`data` from the device's config, default /data) and lists that directory in ReadWritePaths.

Two ways to satisfy it. An image whose board component has the generator write both (Lemnos 6c8568d0,
`lemnos-ctl tree --bridge-env FILE --bridge-dropin FILE`, from the same board file lemnosd reads) and places them (the
env as an install or a fragment of /etc/lemnos/lemnos-orion.env, the drop-in under lemnos-orion.service.d/) names every
file-taking device by construction: the declarations are checked, and no hand-written file may also set
LEMNOS_ORION_FILES. An image with no such step (the flasher, on an older Lemnos) is checked by its files' text.

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
# board: Lemnos 6c8568d0 packaging/systemd/lemnos-orion.env.example:23) and system-flash (the first-flash stream,
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


def generated(component, board_key):
    """Problems of a component's generated bridge files, or None when it generates none.

    The step must describe the board file the image installs (`board_key`), pass both flags, and place both outputs.
    """
    step = component.get("generate") or {}
    args = step.get("args", [])
    if "--bridge-env" not in args and "--bridge-dropin" not in args:
        return None
    problems = []
    def out_of(flag):
        if flag not in args or args.index(flag) + 1 >= len(args):
            return None
        return args[args.index(flag) + 1].removeprefix("@out@/")
    env, dropin = out_of("--bridge-env"), out_of("--bridge-dropin")
    if env is None or dropin is None:
        return ["the generator is given only one of --bridge-env and --bridge-dropin: the settings and the unit's grant must come from the same run"]
    if step.get("description") != board_key:
        problems.append(f"the generator describes {step.get('description')!r}, and the image installs the board file {board_key!r}")
    for name in (env, dropin):
        if name not in step.get("outputs", []):
            problems.append(f"{name} is written by the generator but is not a declared output")
    placed_env = [i.get("to") for i in component.get("install", []) + component.get("fragments", [])
                  if i.get("from", {}).get("generated") == env]
    placed_dropin = [i.get("to") for i in component.get("install", []) if i.get("from", {}).get("generated") == dropin]
    if placed_env != [ENVFILE]:
        problems.append(f"the generated settings ({env}) must be placed at {ENVFILE} and nowhere else, not {placed_env}")
    if len(placed_dropin) != 1 or not DROPIN.match(placed_dropin[0] or ""):
        problems.append(f"the generated drop-in ({dropin}) must be installed once under lemnos-orion.service.d/, not {placed_dropin}")
    return problems


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
            board_key, steps = None, []
            for comp in image.get("components", []):
                cpath = os.path.normpath(os.path.join(base, product["components"][comp]))
                component = load(cpath)
                steps.append((comp, component))
                for item in component.get("install", []):
                    to, src = item.get("to", ""), item.get("from", {})
                    has_bridge |= "lemnos-orion" in to
                    systemd_unit |= to == UNIT
                    if to == BOARD:
                        key = src.get("description")
                        board_key = key
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
            name = f"{product_dir}/{image.get('name', '?')}"
            made = [(comp, generated(component, board_key)) for comp, component in steps]
            made = [(comp, problems) for comp, problems in made if problems is not None]
            if has_bridge and made:
                # The generator names the devices: its declarations are checked, and no hand-written file may also name any.
                for comp, problems in made:
                    errors += [f"{name}: {comp}: {problem}" for problem in problems]
                if len(made) > 1:
                    errors.append(f"{name}: {', '.join(comp for comp, _ in made)} each generate the bridge's settings: one board file, one generator run")
                for text in texts:
                    files, streams, _ = gives(text)
                    if files or streams:
                        errors.append(f"{name}: a hand-written file also sets LEMNOS_ORION_FILES or _STREAMS ({sorted(set(files) | streams)}): the generated settings are the only ones")
                continue
            errors += check(name, devices, has_bridge, systemd_unit, texts)
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
    # The generated form: both flags, both outputs declared and placed, for the board file the image installs.
    gen = {"generate": {"description": "lemnos", "args": ["tree", "@description@", "--bridge-env", "@out@/e.env", "--bridge-dropin", "@out@/d.conf"],
                        "outputs": ["e.env", "d.conf"]},
           "fragments": [{"from": {"generated": "e.env"}, "to": ENVFILE}],
           "install": [{"from": {"generated": "d.conf"}, "to": "/etc/systemd/system/lemnos-orion.service.d/files.conf"}]}
    assert generated(gen, "lemnos") == [] and generated({"generate": {"args": ["tree"]}}, "lemnos") is None
    assert any("describes" in e for e in generated(gen, "flasher-lemnos"))
    one = {**gen, "generate": {**gen["generate"], "args": ["tree", "--bridge-env", "@out@/e.env"]}}
    assert any("only one of" in e for e in generated(one, "lemnos"))
    assert any("must be installed once" in e for e in generated({**gen, "install": []}, "lemnos"))
    assert any("must be placed at" in e for e in generated({**gen, "fragments": []}, "lemnos"))
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
