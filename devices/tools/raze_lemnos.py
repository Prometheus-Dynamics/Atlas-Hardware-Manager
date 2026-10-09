"""The Raze's Lemnos board definition (/etc/lemnos/board.toml), for gen-raze.py.

lemnosd, the board's hardware service, builds its devices from a board
definition in Lemnos's format (Lemnos docs/board-definition.md, schema
devices/schema/lemnos-board.schema.json, vendored from the pinned Lemnos
commit). This module renders it from manifest.json and checks it:

- against the JSON Schema, with the small validator below (the keywords the
  schema uses: $ref, type, const, enum, required, properties,
  additionalProperties, items, pattern, minimum, maximum, anyOf);
- against what lemnos-board's DriverRegistry accepts per driver (placement,
  config and match keys), the checks `lemnos-ctl validate` makes beyond the
  schema;
- by parsing the TOML it wrote back (tomllib) and comparing.

`lemnos-ctl validate` itself runs too when gen-raze.py finds one (--lemnos-ctl
or $LEMNOS_CTL). Python 3.11+ standard library only.
"""

from __future__ import annotations

import json
import re
import subprocess
import tomllib
from pathlib import Path

SELECTOR_KEYS = ("name", "compatible", "of", "node")

# What lemnos-board's DriverRegistry (Lemnos b2634fc, crates/lemnos-board/src
# registry.rs and light.rs) accepts: placement, config keys, match keys.
LIGHT_KEYS = (
    "count", "wire", "offset", "direction", "brightness", "gpio", "fade_ms", "easing",
    "status_effect", "error_effect", "breathe_period_ms", "breathe_depth", "blink_period_ms",
    "blink_duty", "ok", "warn", "error", "busy", "locate", "locate_effect",
    "spinner_period_ms", "spinner_tail", "idle", "progress", "progress_background",
    "updating", "verifying", "writing", "staged", "booting", "rebooting", "failed",
)
INA_KEYS = ("shunt_micro_ohms", "shunt_ohms", "max_current_micro_amps", "max_current_amps")
DRIVERS = {
    "bmi088": ("i2c", ("gyro_address", "accel_range", "accel_rate", "gyro_range", "gyro_rate"), ()),
    "bmm150": ("i2c", ("preset", "data_rate"), ()),
    "ina226": ("i2c", INA_KEYS, ()),
    "ina238": ("i2c", INA_KEYS, ()),
    "ina260": ("i2c", (), ()),
    "hwmon-fan": ("platform", ("restore_mode",), ("name",)),
    "thermal-zone": ("platform", (), ("type",)),
    "ws2812": ("platform", LIGHT_KEYS, ()),
    "gpio-output": ("platform", ("chip", "line", "active_low", "initial"), ()),
}
EASINGS = {"linear", "ease-in", "ease-out", "ease-in-out", "sine"}
EFFECTS = {"solid", "blink", "breathe"}


class BoardError(Exception):
    pass


# ---------------------------------------------------------------------------
# A JSON Schema validator for the keywords the Lemnos schema uses.


def _is_type(value: object, kind: str) -> bool:
    if kind == "integer":
        return isinstance(value, int) and not isinstance(value, bool)
    if kind == "number":
        return isinstance(value, (int, float)) and not isinstance(value, bool)
    return {
        "string": str, "boolean": bool, "object": dict, "array": list, "null": type(None),
    }[kind] is type(value) or (kind == "object" and isinstance(value, dict))


def schema_errors(value: object, schema: dict, root: dict, where: str = "") -> list[str]:
    if "$ref" in schema:
        ref = schema["$ref"]
        if not ref.startswith("#/"):
            raise BoardError(f"schema: unsupported $ref {ref}")
        target = root
        for part in ref[2:].split("/"):
            target = target[part]
        return schema_errors(value, target, root, where)
    at = where or "(top)"
    errors: list[str] = []
    if "anyOf" in schema:
        if all(schema_errors(value, option, root, where) for option in schema["anyOf"]):
            errors.append(f"{at}: matches none of the allowed forms")
    if "type" in schema:
        kinds = schema["type"] if isinstance(schema["type"], list) else [schema["type"]]
        if not any(_is_type(value, kind) for kind in kinds):
            return errors + [f"{at}: must be {' or '.join(kinds)}"]
    if "const" in schema and value != schema["const"]:
        errors.append(f"{at}: must be {json.dumps(schema['const'])}")
    if "enum" in schema and value not in schema["enum"]:
        errors.append(f"{at}: must be one of {schema['enum']}")
    if isinstance(value, str) and "pattern" in schema and not re.search(schema["pattern"], value):
        errors.append(f"{at}: {value!r} doesn't match {schema['pattern']}")
    if _is_type(value, "number"):
        if "minimum" in schema and value < schema["minimum"]:
            errors.append(f"{at}: below {schema['minimum']}")
        if "maximum" in schema and value > schema["maximum"]:
            errors.append(f"{at}: above {schema['maximum']}")
    if isinstance(value, dict):
        for key in schema.get("required", []):
            if key not in value:
                errors.append(f"{at}: {key} is required")
        properties = schema.get("properties", {})
        extra = schema.get("additionalProperties", True)
        for key, item in value.items():
            path = f"{where}.{key}" if where else key
            if key in properties:
                errors += schema_errors(item, properties[key], root, path)
            elif extra is False:
                errors.append(f"{at}: unknown key {key}")
            elif isinstance(extra, dict):
                errors += schema_errors(item, extra, root, path)
    if isinstance(value, list) and "items" in schema:
        for index, item in enumerate(value):
            errors += schema_errors(item, schema["items"], root, f"{where}[{index}]")
    return errors


def driver_errors(board: dict) -> list[str]:
    """What DriverRegistry::validate checks beyond the schema."""
    errors = []
    ids = [device["id"] for device in board.get("devices", [])]
    for dup in sorted({i for i in ids if ids.count(i) > 1}):
        errors.append(f"device id {dup} is used twice")
    for device in board.get("devices", []):
        where = f"devices[{device['id']}]"
        entry = DRIVERS.get(device["driver"])
        if not entry:
            errors.append(f"{where}: unknown driver {device['driver']}")
            continue
        placement, config_keys, match_keys = entry
        if placement == "i2c" and "bus" not in device:
            errors.append(f"{where}: {device['driver']} needs a bus")
        if placement == "platform" and ("bus" in device or "address" in device):
            errors.append(f"{where}: {device['driver']} takes path or match, not a bus")
        for key in device.get("config", {}):
            if key not in config_keys:
                errors.append(f"{where}: unknown config key {key}")
        for key in device.get("match", {}):
            if key not in match_keys:
                errors.append(f"{where}: unknown match key {key}")
        config = device.get("config", {})
        if device["driver"] in ("ina226", "ina238"):
            if not {"shunt_micro_ohms", "shunt_ohms"} & set(config) or not {
                "max_current_micro_amps", "max_current_amps"} & set(config):
                errors.append(f"{where}: needs a shunt and a maximum current")
            elif device["driver"] == "ina238" and config["max_current_micro_amps"] * config["shunt_micro_ohms"] > 163_840_000_000:
                errors.append(f"{where}: maximum current x shunt exceeds the INA238's 163.84 mV range")
        if device["driver"] == "ws2812":
            if not isinstance(config.get("count"), int):
                errors.append(f"{where}: count is required")
            if config.get("wire") not in (None, "rgb", "rgbw"):
                errors.append(f"{where}: wire must be rgb or rgbw")
            if config.get("direction") not in (None, "cw", "ccw"):
                errors.append(f"{where}: direction must be cw or ccw")
            if not 0 <= config.get("offset", 0) < config.get("count", 1):
                errors.append(f"{where}: offset must be in 0..count-1")
            if config.get("easing", "linear") not in EASINGS:
                errors.append(f"{where}: unknown easing {config['easing']}")
            if config.get("status_effect", "solid") not in EFFECTS:
                errors.append(f"{where}: unknown status_effect {config['status_effect']}")
        if device["driver"] == "gpio-output" and not {"chip", "line"} <= set(config):
            errors.append(f"{where}: needs chip and line")
    return errors


# ---------------------------------------------------------------------------
# From the manifest.


def bus_ref(caps: dict, number: int) -> str:
    """A Lemnos bus string: the bus's selector from the manifest, else i2c-<n>."""
    for bus in caps["i2c"]["buses"]:
        if bus["bus"] == number and bus.get("select"):
            keys = [k for k in SELECTOR_KEYS if k in bus["select"]]
            if keys:
                return "i2c:" + ";".join(f"{k}={bus['select'][k]}" for k in keys)
    return f"i2c-{number}"


def unverified(section: dict, fact: str) -> bool:
    return section.get("verified", {}).get(fact, "unverified") == "unverified"


def board_definition(manifest: dict, version: str) -> tuple[dict, dict[str, list[str]]]:
    """The board definition and a comment block per device id."""
    caps = manifest["capabilities"]
    leds, fan, i2c, usb = caps["leds"], caps["fan"], caps["i2c"], caps["usb-power"]
    devices: list[dict] = []
    notes: dict[str, list[str]] = {}

    ring = leds["lemnosd"]
    look = ring.get("look", {})
    config = {
        "count": leds["count"],
        "wire": "rgbw" if leds["wire_format"].endswith("32") else "rgb",
        "offset": leds["index"]["offset"],
        "direction": "cw" if leds["index"]["direction"] == 1 else "ccw",
        "gpio": leds["gpio"],
        **look,
    }
    devices.append({"id": ring["device"], "driver": "ws2812", "path": leds["device"], "config": config})
    notes[ring["device"]] = [
        f"{leds['count']} {leds['part']} LEDs on GPIO{leds['gpio']} (ws2812-pio), {leds['wire_format']} on the wire;",
        f"logical LED 0 is physical LED {leds['index']['offset']}.",
    ]
    for fact, label in (("count", "count"), ("index.direction", "direction")):
        if unverified(leds, fact):
            notes[ring["device"]].append(f"{label}: from the manifest, unverified on hardware.")

    lf = fan["lemnosd"]
    devices.append({
        "id": lf["device"], "driver": "hwmon-fan", "match": {"name": fan["hwmon_name"]},
        "poll_ms": lf["poll_ms"], "writers": lf["writers"],
    })
    notes[lf["device"]] = [
        f"The {fan['cooling_device_type']} fan (hwmon name {fan['hwmon_name']}). pwm-fan has no automatic mode,",
        "so no restore_mode: when lemnosd stops, the fan gets back the governor's",
        f"cooling state and {fan['thermal_zone']} re-evaluates. The duty floor "
        f"({fan['min_level']}/{fan['level_max']}) is the kernel's.",
    ]
    devices.append({
        "id": lf["thermal_device"], "driver": "thermal-zone", "match": {"type": fan["thermal_zone"]},
        "poll_ms": lf["poll_ms"],
    })

    # Sensors: one board device per lemnosd.device; a part with two
    # addresses (the BMI088) names the second in config.
    sensors: dict[str, dict] = {}
    for index, part in enumerate(i2c["devices"]):
        ld = part.get("lemnosd")
        if not ld:
            continue
        if "driver" in ld:
            device = {"id": ld["device"], "driver": ld["driver"]}
            if "label" in ld:
                device["label"] = ld["label"]
            device["bus"] = bus_ref(caps, part["bus"])
            device["address"] = part["address"]
            device["poll_ms"] = ld["poll_ms"]
            if "raw" in ld:
                # Brokered raw reads for these clients (the self-test's chip-id check).
                device["raw"] = ld["raw"]
            device["config"] = {}
            if "shunt_ohm" in part:
                device["config"]["shunt_micro_ohms"] = round(part["shunt_ohm"] * 1e6)
                device["config"]["max_current_micro_amps"] = round(part["max_current_a"] * 1e6)
            sensors[ld["device"]] = device
            note = notes.setdefault(ld["device"], [])
            bus = next(b for b in i2c["buses"] if b["bus"] == part["bus"])
            note.append(f"{part['part']} {part['function']} at 0x{part['address']:02x} on the {bus['kind']} bus"
                        f" (i2c-{part['bus']} on the 7.2.9 image).")
            if not unverified(i2c, "devices"):
                note.append("Address and chip id verified on a board by chip-id register reads.")
            if unverified(i2c, f"buses.{i2c['buses'].index(bus)}.select"):
                note.append("Bus selector from the device tree, unverified on hardware.")
            if "shunt_ohm" in part and unverified(i2c, f"devices.{index}.shunt_ohm"):
                note.append(f"Shunt {part['shunt_ohm']} ohm: from the manifest, unverified against the schematic.")
            if "raw" in ld:
                note.append(f"Raw reads (chip id) through lemnosd for: {', '.join(ld['raw'])}.")
            if "max_current_a" in part:
                note.append(f"Maximum current {part['max_current_a']} A: the INA238's 163.84 mV full scale over the")
                note.append("shunt (a 0.5 mA LSB), derived, not a board rating.")
    for part in i2c["devices"]:
        ld = part.get("lemnosd") or {}
        if "config" in ld:
            sensors[ld["device"]]["config"][ld["config"]] = part["address"]
            notes[ld["device"]].append(f"{part['part']} {part['function']} at 0x{part['address']:02x}.")
    for device in sensors.values():
        if not device["config"]:
            del device["config"]
        devices.append(device)

    port = usb["lemnosd"]
    line = next(p for p in usb["ports"] if p["name"] == port["port"])
    gpio = {"chip": usb["gpio_chip"], "line": line["gpio"]}
    if not line.get("active_high", True):
        gpio["active_low"] = True
    gpio["initial"] = True
    devices.append({"id": port["device"], "driver": "gpio-output", "config": gpio})
    notes[port["device"]] = [
        f"The {port['port']} port's power enable. The raze-usb-power overlay hogs this line by",
        "default, so lemnosd reports it missing until the OS loads the overlay with hog=off.",
        "Its safe state is on: lemnosd drives it high at start (initial), and a client's",
        "write is undone when its connection ends (`lemnos-ctl set` writes persist until",
        "`lemnos-ctl restore usb-a-power`). A device's line is never handed out raw, so",
        "it needs no [[lines]] entry.",
    ]
    if unverified(usb, "gpio_chip"):
        notes[port["device"]].append("gpio chip label: unverified on hardware.")

    board = {
        "format": "lemnos.board",
        "schema_version": 1,
        "board": {
            "id": manifest["model"],
            "name": manifest["display_name"],
            "generated_by": f"atlas devices/tools/gen-raze.py, {manifest['model']} package {version}",
        },
        "devices": devices,
    }
    return board, notes


# ---------------------------------------------------------------------------
# TOML.

HEX_KEYS = {"address", "gyro_address"}


def toml_value(key: str, value: object) -> str:
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, int):
        return f"0x{value:02x}" if key in HEX_KEYS else str(value)
    if isinstance(value, float):
        return repr(value)
    if isinstance(value, str):
        return json.dumps(value, ensure_ascii=False)
    if isinstance(value, list):
        return "[" + ", ".join(toml_value("", v) for v in value) + "]"
    if isinstance(value, dict):
        return "{ " + ", ".join(f"{k} = {toml_value(k, v)}" for k, v in value.items()) + " }"
    raise BoardError(f"can't write {value!r} as TOML")


def to_toml(board: dict, notes: dict[str, list[str]], header: str) -> str:
    out = [header, "#\n# lemnosd's board definition (Lemnos docs/board-definition.md). The Raze package\n",
           "# installs it as /etc/lemnos/board.toml; an OS replaces it with its own file there.\n"]
    for key in ("format", "schema_version"):
        out.append(f"{key} = {toml_value(key, board[key])}\n")
    out.append("\n[board]\n")
    out += [f"{k} = {toml_value(k, v)}\n" for k, v in board["board"].items()]
    for device in board["devices"]:
        out.append("\n")
        out += [f"# {line}\n" for line in notes.get(device["id"], [])]
        out.append("[[devices]]\n")
        out += [f"{k} = {toml_value(k, v)}\n" for k, v in device.items()]
    return "".join(out)


def check(board: dict, text: str, schema_path: Path) -> None:
    schema = json.loads(schema_path.read_text())
    errors = schema_errors(board, schema, schema) + driver_errors(board)
    if tomllib.loads(text) != board:
        errors.append("the written TOML doesn't parse back to the same definition")
    if errors:
        raise BoardError("board.toml: " + "; ".join(errors))


def lemnos_ctl_validate(ctl: str, path: Path) -> tuple[bool, str]:
    run = subprocess.run([ctl, "validate", str(path)], capture_output=True, text=True)
    return run.returncode == 0, (run.stdout + run.stderr).strip()
