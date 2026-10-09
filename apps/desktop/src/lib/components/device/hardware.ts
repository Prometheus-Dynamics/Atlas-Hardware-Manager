// Helpers for the Hardware tab: readings named <prefix>_x, _y and _z (or
// <prefix>.x as Lemnos names them over Orion) become
// one row, number formatting, status pills, and the trend samples.

import type { HardwareReading } from "#lib/api/client.ts";
import type { Tone } from "#lib/format.ts";

/** Trend points kept per reading while the tab is open. */
export const SAMPLES = 60;

const AXIS = /^(.+)[._]([xyz])$/;

export type ReadingEntry =
  | { kind: "single"; key: string; reading: HardwareReading }
  | { kind: "axes"; key: string; label: string; axes: { axis: string; reading: HardwareReading }[] };

/** A reading name as words: accel_x and accel.x read "accel x". */
export const label = (name: string) => name.replaceAll(/[._]/g, " ");

/** Groups every <prefix>_x, _y, _z trio into one entry; the rest stay single. */
export function groupReadings(readings: HardwareReading[]): ReadingEntry[] {
  const groups = new Map<string, Map<string, HardwareReading>>();
  for (const reading of readings) {
    const match = AXIS.exec(reading.name);
    if (!match) continue;
    const axes = groups.get(match[1]) ?? new Map<string, HardwareReading>();
    axes.set(match[2], reading);
    groups.set(match[1], axes);
  }
  const trios = new Set([...groups].filter(([, axes]) => axes.size === 3).map(([prefix]) => prefix));

  const entries: ReadingEntry[] = [];
  const placed = new Set<string>();
  for (const reading of readings) {
    const match = AXIS.exec(reading.name);
    if (match && trios.has(match[1])) {
      if (placed.has(match[1])) continue;
      placed.add(match[1]);
      const axes = groups.get(match[1]) ?? new Map<string, HardwareReading>();
      entries.push({
        kind: "axes",
        key: `axes:${match[1]}`,
        label: label(match[1]),
        axes: ["x", "y", "z"].map((axis) => ({ axis, reading: axes.get(axis) as HardwareReading })),
      });
    } else {
      entries.push({ kind: "single", key: reading.name, reading });
    }
  }
  return entries;
}

/** Three significant figures; "-" when unknown. */
export function formatValue(value: number | null): string {
  if (value === null || !Number.isFinite(value)) return "-";
  if (value === 0) return "0";
  const decimals = Math.min(20, Math.max(0, 2 - Math.floor(Math.log10(Math.abs(value)))));
  return value.toFixed(decimals);
}

const PILLS: Record<string, { tone: Tone; label: string } | undefined> = {
  available: { tone: "success", label: "available" },
  degraded: { tone: "warning", label: "degraded" },
  faulted: { tone: "error", label: "faulted" },
  missing: { tone: "neutral", label: "not found" },
};

export function pillFor(status: string): { tone: Tone; label: string } {
  return PILLS[status] ?? { tone: "neutral", label: status };
}

/** The key a reading's trend is kept under. */
export const sampleKey = (device: string, reading: string) => `${device}/${reading}`;

/** A new ring of the last SAMPLES points, with `value` appended. */
export function appendSample(series: number[] | undefined, value: number): number[] {
  return [...(series ?? []), value].slice(-SAMPLES);
}
