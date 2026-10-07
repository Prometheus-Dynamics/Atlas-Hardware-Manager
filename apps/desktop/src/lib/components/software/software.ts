// What a device's Software tab can offer: which images fit it, how it can
// get into USB boot, and the words for its update state.

import type { DeviceRecord, ReleaseEntry } from "#lib/api/client.ts";
import { compareVersionsDesc } from "#lib/format.ts";

/** The `usb-boot` device action: restarts a running board into USB boot. */
export const USB_BOOT_ACTION = "usb-boot";

/** The ids in the `update_methods` attribute, such as `ab-tryboot`. */
export function updateMethods(record: DeviceRecord): string[] {
  return (record.identity.attributes?.update_methods ?? "")
    .split(",")
    .map((m) => m.trim())
    .filter(Boolean);
}

export function boardSerial(record: DeviceRecord): string | null {
  return record.identity.attributes?.board_serial?.toLowerCase() ?? null;
}

/** The board name images list in `boards`: "raze". */
function boardName(record: DeviceRecord): string {
  return (record.identity.attributes?.model ?? record.identity.model.split(" ")[0] ?? record.key.family).toLowerCase();
}

/**
 * Images for this device: its family's catalog entries plus entries made
 * for the same board under another family (a running Raze and the same
 * Raze in USB boot are different families).
 */
export function imagesFor(record: DeviceRecord, entries: ReleaseEntry[]): ReleaseEntry[] {
  const board = boardName(record);
  return entries
    .filter((e) => e.family === record.key.family || e.boards.some((b) => b.toLowerCase() === board))
    .sort(newestFirst);
}

/** The version inside a name like "helios-raze-2026.3.1". */
export const versionPart = (v: string) => v.match(/\d+(?:\.\d+)+.*$/)?.[0] ?? v;

/** Newest first by the version number, whatever the name in front of it. */
export function newestFirst(a: ReleaseEntry, b: ReleaseEntry): number {
  return compareVersionsDesc(versionPart(a.version), versionPart(b.version)) || a.version.localeCompare(b.version);
}

/** The image to start with: a signed stable one of the OS the board runs now, if any. */
export function preferredImage(record: DeviceRecord, entries: ReleaseEntry[]): ReleaseEntry | undefined {
  const os = record.identity.attributes?.os?.toLowerCase();
  const good = entries.filter((e) => e.signed && e.channel === "stable");
  return (os ? good.find((e) => e.artifact_name.toLowerCase().includes(os)) : undefined) ?? good[0] ?? entries[0];
}

/** The manual steps into USB boot: the board's own, else the general ones. */
export function usbBootSteps(record: DeviceRecord, all: DeviceRecord[]): string[] {
  const own = record.identity.attributes?.recovery_steps;
  const model = record.identity.model.split(" ")[0];
  const sibling = all.find((r) => r.identity.attributes?.recovery_steps && r.identity.model.split(" ")[0] === model);
  const text =
    own ??
    sibling?.identity.attributes?.recovery_steps ??
    [
      `Power the ${model} off.`,
      "Hold its boot button.",
      "While holding it, connect its flashing USB port to this computer, then let go once it shows up.",
    ].join("\n");
  return text
    .split("\n")
    .map((s) => s.trim())
    .filter(Boolean);
}

/** The A/B updater's state, when the board reports it. Only what exists. */
export function slotFacts(record: DeviceRecord): { label: string; value: string }[] {
  const a = record.identity.attributes ?? {};
  const pick = (...keys: string[]) => keys.map((k) => a[k]).find((v) => v && v.trim());
  const facts: { label: string; value: string }[] = [];
  const slot = pick("slot_active", "update.slot_active", "update_slot_active");
  if (slot) facts.push({ label: "Active slot", value: slot });
  const state = pick("update_state", "update.state");
  const error = pick("update_error", "update.error");
  if (error) facts.push({ label: "Last update", value: error });
  else if (state) facts.push({ label: "Last update", value: stateText(state) });
  return facts;
}

function stateText(state: string): string {
  switch (state.toLowerCase()) {
    case "idle":
    case "committed":
    case "confirmed":
    case "good":
      return "Kept";
    case "rolled-back":
    case "rollback":
    case "rolledback":
      return "Rolled back";
    case "staged":
      return "Waiting for a restart";
    case "trial":
    case "trying":
      return "On trial";
    default:
      return state;
  }
}
