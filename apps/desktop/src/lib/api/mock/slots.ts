// A slots board's images for the browser build: what the slots writer
// would report, and its refusals.
import type { DeviceKey, ProgressUpdate, Slot, SlotCommand, SlotSet } from "../types";
import { keyString } from "../types";
import { simDevice } from "./data";
import { sleep } from "./bus";

const GiB = 1024 ** 3;

const slot = (id: string, version: string, variant: string, flags: string[], extra: Partial<Slot> = {}): Slot => ({
  id,
  family: id.startsWith("photonvision") ? "photonvision" : "helios",
  version,
  variant,
  kernel: "7.2.9",
  size: Math.round((variant === "styx" ? 0.62 : 0.48) * GiB),
  verity: true,
  flags,
  pinned: false,
  good: flags.includes("running") ? "yes" : "",
  installed: "2026-10-12T18:04:00Z",
  last_boot: flags.includes("running") ? "2026-10-15T07:31:00Z" : "",
  ...extra,
});

const sets = new Map<string, SlotSet>();

function seed(): SlotSet {
  return {
    slot_active: "photonvision-2026.3.0-styx-1a2b3c4d",
    slot_default: "photonvision-2026.3.0-styx-1a2b3c4d",
    slot_previous: "photonvision-2026.2.1-styx-9e8f7a6b",
    slot_trial: "",
    free_bytes: Math.round(21.4 * GiB),
    reserve_bytes: 2 * GiB,
    slots: [
      slot("photonvision-2026.3.0-styx-1a2b3c4d", "2026.3.0", "styx", ["running", "default"]),
      slot("photonvision-2026.2.1-styx-9e8f7a6b", "2026.2.1", "styx", ["previous"], { good: "yes", pinned: true, last_boot: "2026-10-11T22:10:00Z" }),
      slot("helios-2026.4.0-base-55aa01ff", "2026.4.0", "base", [], { good: "no", last_boot: "2026-10-14T16:45:00Z" }),
    ],
  };
}

function setOf(key: DeviceKey): SlotSet | null {
  if (!simDevice(key)?.caps?.includes("slots")) return null;
  const k = keyString(key);
  if (!sets.has(k)) sets.set(k, seed());
  return sets.get(k)!;
}

function flagsFor(set: SlotSet) {
  for (const s of set.slots) {
    s.flags = [
      ...(s.id === set.slot_active ? ["running"] : []),
      ...(s.id === set.slot_default ? ["default"] : []),
      ...(s.id === set.slot_previous ? ["previous"] : []),
      ...(s.id === set.slot_trial ? ["trial"] : []),
    ];
  }
}

export function deviceSlots(key: DeviceKey): SlotSet | null {
  const set = setOf(key);
  return set ? structuredClone(set) : null;
}

export function slotCommand(key: DeviceKey, command: SlotCommand) {
  const set = setOf(key);
  if (!set) throw `${keyString(key)} has no slots`;
  const s = set.slots.find((x) => x.id === command.id);
  if (!s) throw `no slot ${command.id}`;
  switch (command.command) {
    case "boot-once":
      if (s.id === set.slot_active) throw `${s.id} is already running`;
      // The mock comes back on it and it confirms.
      set.slot_previous = set.slot_default;
      set.slot_default = s.id;
      set.slot_active = s.id;
      s.good = "yes";
      s.last_boot = new Date().toISOString();
      break;
    case "set-default":
      if (s.good !== "yes") throw `${s.id} hasn't booted well yet; boot it once first`;
      set.slot_previous = set.slot_default;
      set.slot_default = s.id;
      break;
    case "remove":
      if (s.id === set.slot_active) throw `${s.id} is the running slot`;
      if (s.id === set.slot_default) throw `${s.id} is the default slot`;
      if (s.pinned) throw `${s.id} is pinned`;
      set.slots = set.slots.filter((x) => x !== s);
      set.free_bytes += s.size;
      if (set.slot_previous === s.id) set.slot_previous = "";
      break;
    case "pin":
      s.pinned = command.pinned;
      break;
  }
  flagsFor(set);
}

export async function installSlot(key: DeviceKey, path: string, pin: boolean, boot: boolean, onProgress: (u: ProgressUpdate) => void) {
  const set = setOf(key);
  if (!set) throw `${keyString(key)} has no slots`;
  const name = path.split(/[\\/]/).pop() ?? path;
  if (!name.endsWith(".slot.tar")) throw "that isn't a slot bundle (.slot.tar)";
  const id = name.replace(/\.slot\.tar$/, "").replace(/[^a-z0-9._-]/g, "-").slice(0, 64);
  const size = Math.round(0.55 * GiB);
  if (size > set.free_bytes - set.reserve_bytes) throw "not enough room on /data; remove a slot first";
  onProgress({ type: "step-started", step: "transfer" });
  for (let i = 1; i <= 10; i++) {
    await sleep(180);
    onProgress({ type: "step-progress", step: "transfer", fraction: i / 10 });
  }
  const parts = id.split("-");
  set.slots = set.slots.filter((x) => x.id !== id);
  set.slots.push(slot(id, parts[1] ?? "", parts[2] ?? "", [], { pinned: pin, installed: new Date().toISOString() }));
  set.free_bytes -= size;
  flagsFor(set);
  onProgress({ type: "log", message: "installed" });
  if (boot) slotCommand(key, { command: "boot-once", id });
}
