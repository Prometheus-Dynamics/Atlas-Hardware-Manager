// How devices and jobs are shown: icons, sublines, friendly stage names.

import { keyString, type DeviceJobState, type DeviceRecord, type JobRecord, type UpdatePlan, type UpdateStep } from "#lib/api/client.ts";
import { deviceName } from "#lib/format.ts";
import type { IconName } from "#lib/ui/icons.ts";

const VISION = /raze|helios|cam|vision|photon|ov\d/i;
const BOARD = /mcu|stm32|esp32|rp2040|board|teensy|arduino/i;

/** In USB boot or a bootloader: only a full image write brings it back. */
export function isRecovery(record: DeviceRecord): boolean {
  return record.identity.mode === "recovery" || record.link_kind.kind === "usb-boot";
}

export function deviceIcon(record: DeviceRecord): IconName {
  if (isRecovery(record)) return "usb";
  if (record.capabilities.includes("gateway")) return "router";
  const text = `${record.key.family} ${record.identity.model} ${record.identity.attributes?.model ?? ""}`;
  if (VISION.test(text)) return "camera";
  if (BOARD.test(text)) return "cpu";
  return "cpu";
}

/** The short model name: "Raze", "cm5". */
export function modelName(record: DeviceRecord): string {
  return record.identity.model || record.key.family;
}

/** "<model> · <os or family>". */
export function deviceSubline(record: DeviceRecord): string {
  const attrs = record.identity.attributes ?? {};
  if (isRecovery(record)) return `${modelName(record)} · USB boot`;
  const second = attrs.os ?? record.key.family;
  return second.toLowerCase() === modelName(record).toLowerCase() ? modelName(record) : `${modelName(record)} · ${second}`;
}

/** The gateway's name for devices reached through another device. */
export function viaName(record: DeviceRecord, nameOf: (key: string) => string): string | null {
  return record.link_kind.kind === "gateway" ? nameOf(keyString(record.link_kind.via)) : null;
}

/** Name for the device in sentences: its label, else its model. */
export function displayModel(record: DeviceRecord): string {
  return record.identity.model || deviceName(record);
}

export function isRecoveryPlan(plan: UpdatePlan): boolean {
  return plan.concurrency.kind === "exclusive" && plan.concurrency.resource === "usb-boot";
}

export function stepLabel(step: UpdateStep, recovery: boolean): string {
  switch (step) {
    case "preflight":
      return "Checks";
    case "transfer":
      return recovery ? "USB boot" : "Transfer";
    case "apply":
      return "Writing";
    case "reboot":
      return "Restart";
    case "confirm":
      return "Verify";
  }
}

export type StageState = "done" | "current" | "failed" | "upcoming";

export function stageStates(job: DeviceJobState): { step: UpdateStep; state: StageState }[] {
  const steps = job.plan.steps;
  const s = job.status.status;
  const index = job.step ? steps.indexOf(job.step) : -1;
  const bad = s === "failed" || s === "rolled-back" || s === "needs-recovery" || s === "cancelled";
  return steps.map((step, i) => {
    if (s === "verified") return { step, state: "done" };
    if (i < index) return { step, state: "done" };
    if (i === index) {
      if (bad) return { step, state: "failed" };
      if (s === "running") return { step, state: job.fraction >= 1 ? "done" : "current" };
    }
    return { step, state: "upcoming" };
  });
}

/** "Flashing Raze", "Updating cam-front", "Updating 3 devices". */
export function jobTitle(job: JobRecord): string {
  const running = job.state === "running";
  if (job.devices.length === 1) {
    const d = job.devices[0];
    const recovery = isRecoveryPlan(d.plan);
    const verb = recovery ? (running ? "Flashing" : "Flashed") : running ? "Updating" : "Updated";
    if (!running && d.status.status !== "verified") return `${recovery ? "Flash" : "Update"} of ${d.name}`;
    return `${verb} ${d.name}`;
  }
  const n = job.devices.length || "";
  if (running) return `Updating ${n} devices`;
  const s = job.summary;
  const clean = s && s.failed + s.rolled_back + s.needs_recovery + s.cancelled + s.skipped === 0;
  return clean ? `Updated ${n} devices` : `Update of ${n} devices`;
}

/** A version guess from a file name: the stem without image extensions. */
export function versionFromFileName(path: string): string {
  const name = path.split(/[\\/]/).pop() ?? path;
  let stem = name;
  for (;;) {
    const next = stem.replace(/\.(img|xz|zst|gz|bin|hex|zip|raw|iso|uf2|tar)$/i, "");
    if (next === stem) break;
    stem = next;
  }
  return stem || name;
}

/** Friendly status sentence for a finished device. */
export function outcomeText(job: DeviceJobState): string {
  switch (job.status.status) {
    case "verified":
      return `Verified on ${job.status.version}`;
    case "rolled-back":
      return "Rolled back";
    case "needs-recovery":
      return "Needs recovery";
    case "failed":
      return "Failed";
    case "skipped":
      return "Skipped";
    case "cancelled":
      return "Cancelled";
    case "queued":
      return "Waiting";
    case "running":
      return "Running";
  }
}

/** After this long without any report, a running device looks stuck. */
export const QUIET_MS = 45_000;

/**
 * How long a running device has been silent, when that is long enough to
 * worry about; null otherwise.
 */
export function quietFor(job: DeviceJobState, now: number): number | null {
  if (job.status.status !== "running") return null;
  const last = job.last_activity_ms ?? job.started_ms;
  if (last === null) return null;
  const quiet = now - last;
  return quiet >= QUIET_MS ? quiet : null;
}

/** "emmc" → "eMMC"; null when the driver did not say. */
export function storageName(storage: string | undefined): string | null {
  if (!storage) return null;
  const known: Record<string, string> = { emmc: "eMMC", sd: "SD card", sdcard: "SD card", nvme: "NVMe", usb: "USB drive" };
  return known[storage.toLowerCase()] ?? storage;
}

/** "A Raze" for an unnamed board, else its own name: for sentences. */
export function aDevice(record: DeviceRecord): string {
  const name = deviceName(record);
  const model = record.identity.model;
  if (model && (name === model || name === keyString(record.key))) return `A ${model}`;
  return name;
}
