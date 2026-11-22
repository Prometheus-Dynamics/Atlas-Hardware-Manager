// What a person is told about a device: its name, a line about what it is, one state word and
// one tone, and how it is reached. The sidebar and the device list both read these, so they
// always agree. The tones are the design language's (docs/design-language.md, "Chip"): ok is
// running, warn needs you later, err failed, info is seen or removable, acc needs you now, off
// is offline.

import type { Device, Link, LinkSighting } from "../api/types";
import { size } from "../ui/format";

export type Tone = "ok" | "warn" | "err" | "info" | "acc" | "off";
export type StateWord = { tone: Tone; word: string; /** the dot beside the word */ dot: Tone | "ring" };

export function stateOf(device: Device): StateWord {
  if (device.presence === "offline") return { tone: "off", word: "Offline", dot: "off" };
  const { state, lemnos } = device.identity;
  switch (state) {
    case "running":
      if (lemnos.state === "not-answering") return { tone: "err", word: "Lemnos not answering", dot: "err" };
      if (lemnos.state === "revision-mismatch") return { tone: "warn", word: "Needs an update", dot: "warn" };
      return { tone: "ok", word: "Running", dot: "ok" };
    case "recovery":
      return { tone: "warn", word: "Recovery", dot: "warn" };
    case "boot-rom":
      return { tone: "acc", word: "Boot ROM, waiting", dot: "acc" };
    case "removable-disk":
      return { tone: "info", word: "Removable disk", dot: "info" };
    case "robot-device":
      // A controller is listed by the robot drivers only while it does not run Orion.
      if (device.role === "controller") return { tone: "warn", word: "Waiting for Orion", dot: "warn" };
      return { tone: "ok", word: "Reporting", dot: "ok" };
  }
}

/** Where a device's row or tree entry leads. A robot's own device has no screen of its own: it
 *  is a row on its robot's screen. */
export function pageOf(device: Device): "board" | "flash" | "disk" | "systemcore" | "robot" {
  if (device.role === "controller") return "systemcore";
  if (device.identity.state === "robot-device" && device.robot !== null) return "robot";
  if (device.presence === "online" && device.identity.state === "boot-rom") return "flash";
  if (device.identity.state === "removable-disk") return "disk";
  return "board";
}

/** What the route of [`pageOf`] is opened with: the robot's name for a robot's screen, else
 *  the device's id. */
export function argOf(device: Device): string {
  return pageOf(device) === "robot" ? (device.robot ?? "") : device.id;
}

/** Whether the row offers Flash now: a board waiting in its boot ROM that the driver can write. */
export function canFlash(device: Device): boolean {
  return device.presence === "online" && device.identity.state === "boot-rom" && device.shown.includes("flash");
}

const diskSize = (device: Device) => {
  const bytes = Number(device.identity.attributes["size-bytes"]);
  return Number.isFinite(bytes) && bytes > 0 ? size(bytes) : "";
};

/** A boot ROM's model without the USB id it was found by ("Rockchip boot ROM (2207:350b)"). The
 *  id is said where the board is flashed, not beside its name. */
const romModel = (model: string) => model.replace(/\s*\([0-9a-f]{4}:[0-9a-f]{4}\)$/i, "");

/** The chip family of a board in its boot ROM, in the short words the mockup uses ("Rockchip
 *  family"), read from the model its boot ROM driver reports ("Rockchip boot ROM"). A model
 *  that is not written that way is shown as it is. */
const romFamily = (model: string) => {
  const found = /^(.+?)\s+boot ROM$/i.exec(romModel(model));
  return found ? `${found[1]} family` : romModel(model);
};

/** The device's name: the person's label or the device's own name; a device that has neither is
 *  called by what it is (its model, and for a disk its size), not by its key. */
export function nameOf(device: Device): string {
  if (device.label !== null || device.identity.name !== null) return device.name;
  const model = device.model_name ?? device.identity.model;
  if (model === null) return device.name;
  if (device.identity.state === "boot-rom") return romModel(model);
  return device.identity.state === "removable-disk" ? `${model} ${diskSize(device)}`.trim() : model;
}

/** The faint words beside the name: what it runs, or what it is. Empty when there is nothing
 *  to add to the name. */
export function kindOf(device: Device): string {
  const { state, versions, attributes, model } = device.identity;
  const named = device.label !== null || device.identity.name !== null;
  const what = device.model_name ?? model ?? "";
  if (state === "removable-disk") return (attributes["device"] ?? "").replace(/^.*\//, "");
  if (state === "running" || state === "recovery") {
    const version = versions["os"] ?? versions["firmware"] ?? versions["bootloader"];
    // What it runs and at which version; a device that names no product is called by its model.
    return [versions["product"] ?? what, version].filter(Boolean).join(" ");
  }
  if (state === "boot-rom") return named && what ? romFamily(what) : "";
  return named ? what : "";
}

/** A sighting counts as a way the device is reached now when it answered about as recently as
 *  the record did (a link is asked again within a minute). */
const LIVE_MS = 150_000;

/** The host of an Orion address (`orion+tcp://10.0.0.2:9200`), or the address as it is. */
function host(address: string): string {
  const found = /^[a-z+]+:\/\/([^:/]+)/.exec(address);
  return found?.[1] ?? address;
}

function linkWord(sighting: LinkSighting, links: Link[], alone: boolean, names: (id: string) => string | undefined): string {
  const label = links.find((link) => link.id === sighting.link)?.label ?? sighting.link;
  switch (sighting.kind.kind) {
    case "usb-network":
    case "usb-serial":
      return label;
    case "ethernet":
      return alone ? `network ${host(sighting.address)}` : host(sighting.address);
    case "usb-boot":
      return `${label} ${sighting.address} · boot ROM`;
    case "local-disk":
      return label;
    case "gateway": {
      const via = sighting.kind.via as { family?: string; serial?: string; node?: string };
      const id = via.family !== undefined ? `${via.family}:${via.serial}` : (via.node ?? "");
      return `USB via ${names(id) ?? id}`;
    }
    case "simulated":
      return "simulated";
  }
}

/** How the device is reached: one short word per link it answers on now, or, for a device that
 *  is offline, every link it was last seen on. One record per board; its links are these. */
export function reachedBy(device: Device, links: Link[], names: (id: string) => string | undefined): string[] {
  const live =
    device.presence === "online"
      ? device.links.filter((sighting) => sighting.last_seen_ms >= device.last_seen_ms - LIVE_MS)
      : device.links;
  // USB first, then the network, whichever answered first: the chips never swap places.
  const ORDER = ["usb-network", "usb-serial", "usb-boot", "local-disk", "ethernet", "gateway", "simulated"];
  const shown = (live.length > 0 ? live : device.links.slice(-1))
    .slice()
    .sort((a, b) => ORDER.indexOf(a.kind.kind) - ORDER.indexOf(b.kind.kind) || a.link.localeCompare(b.link));
  return shown.map((sighting) => linkWord(sighting, links, shown.length === 1, names));
}
